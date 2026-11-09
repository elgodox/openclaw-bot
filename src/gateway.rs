//! The connection to an OpenClaw gateway: WebSocket, handshake, device proof,
//! reconnects, and optionally an SSH tunnel for remote computers.
//!
//! Runs on its own thread; the window talks to it through channels.

use crate::identity::{device_auth_payload_v3, hex, DeviceIdentity};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

pub const PROTOCOL_VERSION: i64 = 4;
pub const CLIENT_ID: &str = "gateway-client";
pub const CLIENT_MODE: &str = "ui";
pub const CLIENT_PLATFORM: &str = "windows";
pub const CLIENT_FAMILY: &str = "desktop";
pub const APP_NAME: &str = "OpenClaw Bot";
// Limits a gateway applies when its greeting doesn't state them.
pub const DEFAULT_MAX_PAYLOAD: u64 = 26214400;
pub const DEFAULT_MAX_ATTACHMENT: u64 = 20971520;
pub const DEFAULT_MAX_IMAGE: u64 = 6291456;

#[derive(Clone, Debug)]
pub enum Secret {
    Token(String),
    Password(String),
}

#[derive(Clone, Debug)]
pub struct SshSpec {
    pub target: String,
    pub identity: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub url: String,
    pub secret: Option<Secret>,
    /// Device token saved earlier for this route, if any.
    pub device_token: Option<String>,
    pub route: String,
    pub tls_pin: Option<String>,
    pub ssh: Option<SshSpec>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnState {
    Idle,
    Tunnel,
    Connecting,
    Connected,
    /// The computer running OpenClaw has to approve this device.
    Pairing { request_id: String },
    /// The gateway refused the credentials; waiting for the person.
    AuthFailed(String),
    Disconnected(String),
}

#[derive(Clone, Debug, Default)]
pub struct HelloInfo {
    pub version: String,
    pub scopes: Vec<String>,
    pub methods: Vec<String>,
    pub max_payload: u64,
    pub max_attachment: u64,
    pub max_image: u64,
    pub default_agent: Option<String>,
    pub snapshot: Value,
}

#[derive(Clone, Debug)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl RpcError {
    fn local(message: impl Into<String>) -> Self {
        Self {
            code: "LOCAL".into(),
            message: message.into(),
            details: Value::Null,
        }
    }
}

pub enum NetEvent {
    State(ConnState),
    Hello(HelloInfo),
    /// A device token the gateway issued, to save for this route.
    DeviceToken { route: String, token: String },
    /// A saved device token stopped working and should be forgotten.
    ForgetDeviceToken { route: String },
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
    Event {
        event: String,
        payload: Value,
    },
}

enum Cmd {
    Request { id: u64, method: String, params: Value },
    Stop,
}

pub struct Client {
    tx: Sender<Cmd>,
    next_id: u64,
    pub target_route: String,
}

impl Client {
    pub fn start(
        target: Target,
        identity: DeviceIdentity,
        repaint: impl Fn() + Send + 'static,
    ) -> (Self, Receiver<NetEvent>) {
        let (tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let target_route = target.route.clone();
        thread::Builder::new()
            .name("gateway".into())
            .spawn(move || {
                let mut sup = Supervisor {
                    target,
                    identity,
                    events: ev_tx,
                    commands: cmd_rx,
                    repaint: Box::new(repaint),
                    tunnel: None,
                    used_device_token: false,
                };
                sup.run();
            })
            .expect("gateway thread");
        (
            Self {
                tx,
                next_id: 1,
                target_route,
            },
            ev_rx,
        )
    }

    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.tx.send(Cmd::Request {
            id,
            method: method.to_string(),
            params,
        });
        id
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

enum ConnectError {
    /// Worth trying again by itself.
    Retry(String),
    /// Retry after this long (the gateway is starting up).
    RetryAfter(Duration, String),
    Pairing { request_id: String },
    Auth(String),
    /// A saved device token was refused; try the shared secret.
    DeviceTokenRefused,
}

struct Supervisor {
    target: Target,
    identity: DeviceIdentity,
    events: Sender<NetEvent>,
    commands: Receiver<Cmd>,
    repaint: Box<dyn Fn() + Send>,
    tunnel: Option<(Child, u16)>,
    used_device_token: bool,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.close_tunnel();
    }
}

impl Supervisor {
    fn emit(&self, ev: NetEvent) {
        let _ = self.events.send(ev);
        (self.repaint)();
    }

    fn close_tunnel(&mut self) {
        if let Some((mut child, _)) = self.tunnel.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn run(&mut self) {
        let mut delay = Duration::from_secs(1);
        loop {
            let url = match self.prepare_url() {
                Ok(u) => u,
                Err(e) => {
                    self.emit(NetEvent::State(ConnState::Disconnected(e)));
                    if self.wait_or_stop(Duration::from_secs(8)) {
                        return;
                    }
                    continue;
                }
            };
            self.emit(NetEvent::State(ConnState::Connecting));
            match self.connect_once(&url) {
                Ok(mut session) => {
                    delay = Duration::from_secs(1);
                    self.emit(NetEvent::State(ConnState::Connected));
                    self.emit(NetEvent::Hello(session.hello.clone()));
                    let reason = self.run_session(&mut session);
                    match reason {
                        None => return, // asked to stop
                        Some(why) => self.emit(NetEvent::State(ConnState::Disconnected(why))),
                    }
                }
                Err(ConnectError::Pairing { request_id }) => {
                    self.emit(NetEvent::State(ConnState::Pairing { request_id }));
                    if self.wait_or_stop(Duration::from_secs(5)) {
                        return;
                    }
                    continue;
                }
                Err(ConnectError::Auth(why)) => {
                    self.emit(NetEvent::State(ConnState::AuthFailed(why)));
                    // No loop of refused attempts: wait for new settings.
                    loop {
                        if self.wait_or_stop(Duration::from_secs(3600)) {
                            return;
                        }
                    }
                }
                Err(ConnectError::DeviceTokenRefused) => {
                    self.emit(NetEvent::ForgetDeviceToken {
                        route: self.target.route.clone(),
                    });
                    self.target.device_token = None;
                    self.used_device_token = false;
                    continue;
                }
                Err(ConnectError::RetryAfter(wait, why)) => {
                    self.emit(NetEvent::State(ConnState::Disconnected(why)));
                    if self.wait_or_stop(wait) {
                        return;
                    }
                    continue;
                }
                Err(ConnectError::Retry(why)) => {
                    self.emit(NetEvent::State(ConnState::Disconnected(why)));
                }
            }
            if self.wait_or_stop(delay) {
                return;
            }
            delay = (delay * 2).min(Duration::from_secs(15));
        }
    }

    /// Waits, dropping requests meanwhile. True when asked to stop.
    fn wait_or_stop(&self, how_long: Duration) -> bool {
        let until = Instant::now() + how_long;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self.commands.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(Cmd::Stop) => return true,
                Ok(Cmd::Request { id, .. }) => {
                    self.emit(NetEvent::Response {
                        id,
                        result: Err(RpcError::local("No conectado")),
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return true,
            }
        }
    }

    /// Opens the SSH tunnel when needed and returns the address to dial.
    fn prepare_url(&mut self) -> Result<String, String> {
        let Some(ssh) = self.target.ssh.clone() else {
            return Ok(self.target.url.clone());
        };
        if let Some((child, port)) = self.tunnel.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(format!("ws://127.0.0.1:{port}")),
                _ => self.close_tunnel(),
            }
        }
        self.emit(NetEvent::State(ConnState::Tunnel));
        let port = free_port().ok_or("No hay un puerto local libre")?;
        let (host, ssh_port) = split_ssh_target(&ssh.target);
        let mut cmd = Command::new("ssh");
        cmd.args(["-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes"])
            .args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"])
            .args(["-L", &format!("{port}:127.0.0.1:{}", ssh.remote_port)]);
        if let Some(p) = ssh_port {
            cmd.args(["-p", &p]);
        }
        if !ssh.identity.trim().is_empty() {
            cmd.args(["-i", ssh.identity.trim()]);
        }
        cmd.arg(host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        hide_console(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("No se pudo iniciar ssh: {e}"))?;
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let mut text = String::new();
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_string(&mut text);
                }
                let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                return Err(format!("El túnel SSH se cerró ({status}): {first}"));
            }
            if TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_millis(250),
            )
            .is_ok()
            {
                break;
            }
            if Instant::now() > until {
                let _ = child.kill();
                let _ = child.wait();
                return Err("El túnel SSH no respondió a tiempo".into());
            }
            thread::sleep(Duration::from_millis(200));
        }
        self.tunnel = Some((child, port));
        Ok(format!("ws://127.0.0.1:{port}"))
    }

    fn connect_once(&mut self, url: &str) -> Result<Session, ConnectError> {
        let mut ws = open_socket(url, self.target.tls_pin.as_deref())
            .map_err(ConnectError::Retry)?;
        set_timeout(&mut ws, Some(Duration::from_secs(15)));

        // 1. Wait for the challenge.
        let (nonce, ts) = wait_challenge(&mut ws).map_err(ConnectError::Retry)?;

        // 2. Choose credentials: saved device token first, else the shared secret.
        let scopes = self.target.scopes.clone();
        let mut auth = json!({});
        let mut signature_token = String::new();
        self.used_device_token = false;
        if let Some(t) = &self.target.device_token {
            auth["deviceToken"] = json!(t);
            signature_token = t.clone();
            self.used_device_token = true;
        } else {
            match &self.target.secret {
                Some(Secret::Token(t)) => {
                    auth["token"] = json!(t);
                    signature_token = t.clone();
                }
                Some(Secret::Password(p)) => auth["password"] = json!(p),
                None => {}
            }
        }

        // 3. Sign and send connect.
        let payload = device_auth_payload_v3(
            &self.identity.device_id,
            CLIENT_ID,
            CLIENT_MODE,
            "operator",
            &scopes,
            ts,
            &signature_token,
            &nonce,
            CLIENT_PLATFORM,
            CLIENT_FAMILY,
        );
        let req_id = "connect-1";
        let frame = json!({
            "type": "req",
            "id": req_id,
            "method": "connect",
            "params": {
                "minProtocol": PROTOCOL_VERSION,
                "maxProtocol": PROTOCOL_VERSION,
                "client": {
                    "id": CLIENT_ID,
                    "displayName": APP_NAME,
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": CLIENT_PLATFORM,
                    "deviceFamily": CLIENT_FAMILY,
                    "mode": CLIENT_MODE,
                },
                "caps": ["agent-kind", "approvals", "exec-approvals", "plugin-approvals",
                         "tool-events", "chat-only-assistant-text"],
                "role": "operator",
                "scopes": scopes,
                "auth": auth,
                "locale": "es",
                "userAgent": format!("openclaw-manager/{}", env!("CARGO_PKG_VERSION")),
                "device": {
                    "id": self.identity.device_id,
                    "publicKey": self.identity.public_key,
                    "signature": self.identity.sign(&payload),
                    "signedAt": ts,
                    "nonce": nonce,
                },
            }
        });
        ws.send(Message::text(frame.to_string()))
            .map_err(|e| ConnectError::Retry(format!("No se pudo enviar el saludo: {e}")))?;

        // 4. Read the answer.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > deadline {
                return Err(ConnectError::Retry("OpenClaw no respondió al saludo".into()));
            }
            let msg = ws
                .read()
                .map_err(|e| ConnectError::Retry(format!("Conexión cerrada al saludar: {e}")))?;
            let Message::Text(text) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            if v["type"] == "res" && v["id"] == req_id {
                if v["ok"].as_bool() != Some(true) {
                    return Err(self.classify_connect_error(&v["error"]));
                }
                let session = self.finish_hello(ws, &v["payload"]);
                return Ok(session);
            }
        }
    }

    fn classify_connect_error(&self, err: &Value) -> ConnectError {
        let code = err["details"]["code"].as_str().unwrap_or("");
        let gw_code = err["code"].as_str().unwrap_or("");
        let msg = err["message"].as_str().unwrap_or("error de conexión").to_string();
        if code == "PAIRING_REQUIRED" || gw_code == "NOT_PAIRED" {
            let request_id = err["details"]["requestId"].as_str().unwrap_or("").to_string();
            return ConnectError::Pairing { request_id };
        }
        if gw_code == "UNAVAILABLE" {
            let wait = err["details"]["retryAfterMs"].as_u64().unwrap_or(2000);
            return ConnectError::RetryAfter(
                Duration::from_millis(wait.clamp(250, 30_000)),
                "OpenClaw se está iniciando".into(),
            );
        }
        if self.used_device_token && code.starts_with("AUTH_") && code != "AUTH_SCOPE_MISMATCH" {
            return ConnectError::DeviceTokenRefused;
        }
        if code.starts_with("AUTH") || gw_code == "UNAUTHORIZED" || code.starts_with("DEVICE_AUTH") {
            let friendly = match code {
                "AUTH_TOKEN_MISMATCH" | "AUTH_TOKEN_MISSING" | "AUTH_PASSWORD_MISMATCH"
                | "AUTH_PASSWORD_MISSING" => {
                    "La clave de OpenClaw no coincide. Revísala en Ajustes.".to_string()
                }
                "AUTH_SCOPE_MISMATCH" => {
                    "Este equipo no tiene permiso para lo que pide. Aprueba otra vez el dispositivo."
                        .to_string()
                }
                _ => format!("OpenClaw rechazó el ingreso: {msg}"),
            };
            return ConnectError::Auth(friendly);
        }
        ConnectError::Retry(msg)
    }

    fn finish_hello(&mut self, ws: Ws, payload: &Value) -> Session {
        let auth = &payload["auth"];
        if let Some(t) = auth["deviceToken"].as_str() {
            let is_new = self.target.device_token.as_deref() != Some(t);
            if is_new {
                self.target.device_token = Some(t.to_string());
                self.emit(NetEvent::DeviceToken {
                    route: self.target.route.clone(),
                    token: t.to_string(),
                });
            }
        }
        let snapshot = payload["snapshot"].clone();
        let default_agent = snapshot["sessionDefaults"]["defaultAgentId"]
            .as_str()
            .or(snapshot["defaultAgentId"].as_str())
            .map(str::to_string);
        let hello = HelloInfo {
            version: payload["server"]["version"].as_str().unwrap_or("").into(),
            scopes: str_list(&auth["scopes"]),
            methods: str_list(&payload["features"]["methods"]),
            max_payload: payload["policy"]["maxPayload"].as_u64().unwrap_or(DEFAULT_MAX_PAYLOAD),
            max_attachment: payload["policy"]["attachments"]["maxBytes"]
                .as_u64()
                .unwrap_or(DEFAULT_MAX_ATTACHMENT),
            max_image: payload["policy"]["attachments"]["maxImageBytes"]
                .as_u64()
                .unwrap_or(DEFAULT_MAX_IMAGE),
            default_agent,
            snapshot,
        };
        let tick_ms = payload["policy"]["tickIntervalMs"].as_u64().unwrap_or(15_000);
        Session {
            ws,
            hello,
            tick: Duration::from_millis(tick_ms.max(1000)),
            last_seen: Instant::now(),
            last_seq: None,
            inflight: Default::default(),
        }
    }

    /// Runs until the connection ends. None means the app asked to stop.
    fn run_session(&mut self, s: &mut Session) -> Option<String> {
        set_nonblocking(&mut s.ws);
        loop {
            let mut busy = false;
            // Outgoing requests.
            loop {
                match self.commands.try_recv() {
                    Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                        let _ = s.ws.close(None);
                        let _ = s.ws.flush();
                        return None;
                    }
                    Err(TryRecvError::Empty) => break,
                    Ok(Cmd::Request { id, method, params }) => {
                        busy = true;
                        let wire_id = format!("r{id}");
                        let frame = json!({"type":"req","id":wire_id,"method":method,"params":params});
                        s.inflight.insert(wire_id, id);
                        if let Err(e) = write_frame(&mut s.ws, frame.to_string()) {
                            return Some(format!("Conexión perdida: {e}"));
                        }
                    }
                }
            }
            // Incoming frames.
            match s.ws.read() {
                Ok(Message::Text(text)) => {
                    busy = true;
                    s.last_seen = Instant::now();
                    if let Some(why) = self.handle_frame(s, text.as_str()) {
                        return Some(why);
                    }
                }
                Ok(Message::Close(frame)) => {
                    let why = frame
                        .map(|f| format!("{} ({})", f.reason, u16::from(f.code)))
                        .unwrap_or_default();
                    return Some(format!("OpenClaw cerró la conexión {why}"));
                }
                Ok(_) => {
                    busy = true;
                    s.last_seen = Instant::now();
                }
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Some(format!("Conexión perdida: {e}")),
            }
            let _ = s.ws.flush();
            // The gateway ticks regularly; silence means the link is dead.
            if s.last_seen.elapsed() > s.tick * 3 {
                return Some("OpenClaw dejó de responder".into());
            }
            if !busy {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn handle_frame(&self, s: &mut Session, text: &str) -> Option<String> {
        let v: Value = serde_json::from_str(text).ok()?;
        match v["type"].as_str()? {
            "res" => {
                let wire = v["id"].as_str()?;
                let id = s.inflight.remove(wire)?;
                let result = if v["ok"].as_bool() == Some(true) {
                    Ok(v["payload"].clone())
                } else {
                    Err(RpcError {
                        code: v["error"]["code"].as_str().unwrap_or("ERROR").into(),
                        message: v["error"]["message"].as_str().unwrap_or("").into(),
                        details: v["error"]["details"].clone(),
                    })
                };
                self.emit(NetEvent::Response { id, result });
            }
            "event" => {
                let event = v["event"].as_str()?.to_string();
                if let Some(seq) = v["seq"].as_u64() {
                    let gap = s.last_seq.is_some_and(|l| seq > l + 1);
                    s.last_seq = Some(seq);
                    if gap {
                        return Some("Se perdieron eventos; reconectando".into());
                    }
                }
                if event == "tick" {
                    return None;
                }
                self.emit(NetEvent::Event {
                    event,
                    payload: v["payload"].clone(),
                });
            }
            _ => {}
        }
        None
    }
}

struct Session {
    ws: Ws,
    hello: HelloInfo,
    tick: Duration,
    last_seen: Instant,
    last_seq: Option<u64>,
    inflight: std::collections::HashMap<String, u64>,
}

fn str_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn write_frame(ws: &mut Ws, frame: String) -> Result<(), String> {
    match ws.send(Message::text(frame)) {
        Ok(()) => Ok(()),
        Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn wait_challenge(ws: &mut Ws) -> Result<(String, i64), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() > deadline {
            return Err("OpenClaw no envió su desafío de conexión".into());
        }
        let msg = ws.read().map_err(|e| format!("Conexión cerrada: {e}"))?;
        let Message::Text(text) = msg else { continue };
        let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
            continue;
        };
        if v["type"] == "event" && v["event"] == "connect.challenge" {
            let nonce = v["payload"]["nonce"].as_str().unwrap_or("").to_string();
            let ts = v["payload"]["ts"].as_i64().unwrap_or(-1);
            if nonce.is_empty() || ts < 0 {
                return Err("El desafío de conexión llegó incompleto".into());
            }
            return Ok((nonce, ts));
        }
    }
}

fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

pub fn split_ssh_target(target: &str) -> (String, Option<String>) {
    let t = target.trim();
    match t.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            (h.to_string(), Some(p.to_string()))
        }
        _ => (t.to_string(), None),
    }
}

fn set_timeout(ws: &mut Ws, d: Option<Duration>) {
    if let Some(stream) = tcp(ws) {
        let _ = stream.set_read_timeout(d);
        let _ = stream.set_write_timeout(d);
    }
}

fn set_nonblocking(ws: &mut Ws) {
    if let Some(stream) = tcp(ws) {
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(None);
        let _ = stream.set_nonblocking(true);
    }
}

fn tcp(ws: &mut Ws) -> Option<&mut TcpStream> {
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_mut()),
        _ => None,
    }
}

/// Dials the URL, doing TLS and the WebSocket upgrade, all within time limits.
pub fn open_socket(url: &str, pin: Option<&str>) -> Result<Ws, String> {
    let parsed = url::Url::parse(url).map_err(|_| format!("Dirección no válida: {url}"))?;
    let tls = match parsed.scheme() {
        "ws" => false,
        "wss" => true,
        _ => return Err("La dirección debe empezar con ws:// o wss://".into()),
    };
    let host = parsed.host_str().ok_or("La dirección no tiene equipo")?.to_string();
    let port = parsed.port().unwrap_or(if tls { 443 } else { 80 });
    let addrs: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("No se encontró {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("No se encontró {host}"));
    }
    // A name can resolve to several addresses; any that answers will do.
    let mut stream = None;
    let mut last = None;
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, Duration::from_secs(10)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let stream = stream.ok_or_else(|| {
        format!(
            "No se pudo llegar a {host}:{port} ({})",
            last.map(|e| e.to_string()).unwrap_or_default()
        )
    })?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let connector = if tls {
        Some(Connector::Rustls(Arc::new(tls_config(pin)?)))
    } else {
        None
    };
    let (ws, _resp) = tungstenite::client_tls_with_config(url, stream, None, connector)
        .map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => friendly_ws_error(e, pin.is_some()),
            tungstenite::HandshakeError::Interrupted(_) => "Tiempo agotado al conectar".to_string(),
        })?;
    Ok(ws)
}

fn friendly_ws_error(e: tungstenite::Error, pinned: bool) -> String {
    let text = e.to_string();
    if text.contains("InvalidCertificate") || text.to_lowercase().contains("certificate") {
        if pinned {
            return "El certificado del equipo no coincide con la huella guardada".into();
        }
        return "Certificado no confiable. Si tu OpenClaw usa un certificado propio, pega su huella en Ajustes".into();
    }
    format!("No se pudo abrir la conexión: {text}")
}

fn tls_config(pin: Option<&str>) -> Result<rustls::ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let config = match pin.map(normalize_fingerprint).filter(|p| !p.is_empty()) {
        Some(pin) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCert { pin, provider }))
            .with_no_client_auth(),
        None => {
            let roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    Ok(config)
}

pub fn normalize_fingerprint(s: &str) -> String {
    s.trim()
        .trim_start_matches("sha256:")
        .trim_start_matches("SHA256:")
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Trust exactly one certificate, as OpenClaw's own clients do with a pin.
#[derive(Debug)]
struct PinnedCert {
    pin: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if hex(&Sha256::digest(end_entity.as_ref())) == self.pin {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_ssh_targets() {
        assert_eq!(split_ssh_target("me@box"), ("me@box".into(), None));
        assert_eq!(split_ssh_target("me@box:2222"), ("me@box".into(), Some("2222".into())));
    }

    #[test]
    fn fingerprints_normalize() {
        assert_eq!(normalize_fingerprint("sha256:AB:cd:01"), "abcd01");
    }
}
