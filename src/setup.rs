//! The installer and first-run assistant.
//!
//! The same program file is the app and its installer: named like an installer
//! (or started with `--setup`) it shows these steps instead of the app. What it
//! does with OpenClaw follows OpenClaw's own guide for scripted setups, and it
//! never changes an existing OpenClaw unless the person asks for that part.

use crate::cli::{self, Host};
use crate::config_edit::{build_patch, ConfigEditor};
use crate::gateway::{Client, ConnState, NetEvent};
use crate::identity::{DeviceIdentity, SecretStore};
use crate::look::{self, Palette};
use crate::probe::{device_token_key, target_for};
use crate::settings::{detect_local, AppSettings, Profile, ProfileKind, SecretKind, Theme};
use eframe::egui::{self, Button, Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, Sense, Stroke, TextEdit, Vec2};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const APP_TITLE: &str = "OpenClaw Bot";
const EXE_NAME: &str = "OpenClaw Bot.exe";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenClawBot";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// True when this program file should act as the installer.
pub fn is_setup_mode(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--setup" || a == "--setup-sandbox") {
        return true;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n.contains("setup"))
}

/// An AI provider offered in the assistant.
pub struct Provider {
    pub label: &'static str,
    pub id: &'static str,
    pub auth_choice: &'static str,
    pub env: &'static str,
    pub needs_key: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider { label: "Anthropic (Claude)", id: "anthropic", auth_choice: "apiKey", env: "ANTHROPIC_API_KEY", needs_key: true },
    Provider { label: "OpenAI (ChatGPT)", id: "openai", auth_choice: "openai-api-key", env: "OPENAI_API_KEY", needs_key: true },
    Provider { label: "Google (Gemini)", id: "google", auth_choice: "gemini-api-key", env: "GEMINI_API_KEY", needs_key: true },
    Provider { label: "xAI (Grok)", id: "xai", auth_choice: "xai-api-key", env: "XAI_API_KEY", needs_key: true },
    Provider { label: "OpenRouter", id: "openrouter", auth_choice: "openrouter-api-key", env: "OPENROUTER_API_KEY", needs_key: true },
    Provider { label: "Together", id: "together", auth_choice: "together-api-key", env: "TOGETHER_API_KEY", needs_key: true },
    Provider { label: "Ollama (en este equipo)", id: "ollama", auth_choice: "ollama", env: "", needs_key: false },
];

#[derive(Clone)]
pub struct RemoteDraft {
    pub name: String,
    pub ssh: bool,
    pub url: String,
    pub ssh_target: String,
    pub ssh_identity: String,
    pub remote_port: String,
    pub password: bool,
    pub secret: String,
    pub fingerprint: String,
    pub admin: bool,
}

impl Default for RemoteDraft {
    fn default() -> Self {
        Self {
            name: "Remoto".into(),
            ssh: false,
            url: String::new(),
            ssh_target: String::new(),
            ssh_identity: String::new(),
            remote_port: crate::settings::DEFAULT_PORT.to_string(),
            password: false,
            secret: String::new(),
            fingerprint: String::new(),
            admin: true,
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub dir: String,
    pub start_menu: bool,
    pub desktop: bool,
    pub autostart: bool,
    pub open_after: bool,
    pub use_local: bool,
    pub use_remote: bool,
    pub install_openclaw: bool,
    pub tag: String,
    pub change_gateway: bool,
    pub port: String,
    pub bind: usize,
    pub auth: usize,
    pub own_secret: String,
    pub tailscale: usize,
    pub service: bool,
    pub change_ai: bool,
    pub provider: usize,
    pub api_key: String,
    pub model: String,
    pub ollama_url: String,
    pub change_bot: bool,
    pub bot_name: String,
    pub workspace: String,
    pub change_channels: bool,
    pub telegram: String,
    pub discord: String,
    pub slack_bot: String,
    pub slack_app: String,
    pub remotes: Vec<RemoteDraft>,
    pub theme: Theme,
    pub notifications: bool,
}

pub const BINDS: [(&str, &str); 4] = [
    ("loopback", "Solo este equipo (lo más seguro)"),
    ("tailnet", "Mi red Tailscale"),
    ("lan", "Mi red local"),
    ("auto", "Automático"),
];

impl Options {
    fn new() -> Self {
        let dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Programs")
            .join(APP_TITLE)
            .to_string_lossy()
            .into_owned();
        Self {
            dir,
            start_menu: true,
            desktop: true,
            autostart: false,
            open_after: true,
            use_local: true,
            use_remote: false,
            install_openclaw: true,
            tag: "stable".into(),
            change_gateway: true,
            port: crate::settings::DEFAULT_PORT.to_string(),
            bind: 0,
            auth: 0,
            own_secret: String::new(),
            tailscale: 0,
            service: true,
            change_ai: true,
            provider: 0,
            api_key: String::new(),
            model: String::new(),
            ollama_url: String::new(),
            change_bot: true,
            bot_name: "Principal".into(),
            workspace: String::new(),
            change_channels: false,
            telegram: String::new(),
            discord: String::new(),
            slack_bot: String::new(),
            slack_app: String::new(),
            remotes: vec![],
            theme: Theme::Dark,
            notifications: true,
        }
    }
}

/// What the assistant found on this computer.
#[derive(Clone, Default)]
pub struct Found {
    pub checked: bool,
    pub version: Option<String>,
    pub has_config: bool,
    pub gateway_up: bool,
    pub port: u16,
    /// `gateway.mode = "remote"`: the address of the other computer's
    /// gateway this one is a node of. Such a computer never gets a gateway.
    pub node_of: Option<String>,
    pub node_token: Option<String>,
    /// The app already has connections to other computers saved.
    pub has_remote_profiles: bool,
}

// ---- the plan ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    CopyApp,
    Shortcuts,
    Register,
    SaveApp,
    InstallOpenClaw,
    /// `openclaw <args>` with secrets passed through the environment.
    Cli { args: Vec<String>, env: Vec<(String, String)> },
    /// `openclaw config patch --stdin`.
    Patch { patch: Value, replace: Vec<String> },
    WaitGateway,
    SetKey { provider: String, key: String },
    SetModel { provider: String, model: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanStep {
    pub title: String,
    pub action: Action,
}

fn step(title: &str, action: Action) -> PlanStep {
    PlanStep { title: title.to_string(), action }
}

/// Works out what the assistant is going to do. Nothing here touches the system.
pub fn build_plan(o: &Options, found: &Found, advanced: Option<(Value, Vec<String>)>) -> Vec<PlanStep> {
    let mut plan = vec![
        step("Copiar OpenClaw Bot", Action::CopyApp),
        step("Crear accesos directos", Action::Shortcuts),
        step("Registrar en Windows", Action::Register),
        step("Guardar tus conexiones y preferencias", Action::SaveApp),
    ];
    if !o.use_local {
        return plan;
    }
    if found.version.is_none() && o.install_openclaw {
        plan.push(step("Instalar OpenClaw", Action::InstallOpenClaw));
    }
    let port: Option<u16> = o.port.trim().parse().ok();
    let provider = PROVIDERS.get(o.provider);
    let fresh = !found.has_config;

    if fresh {
        // A new install: OpenClaw's scripted setup creates everything at once.
        let mut args: Vec<String> = ["onboard", "--non-interactive", "--accept-risk", "--mode", "local", "--skip-daemon", "--skip-health", "--skip-channels", "--skip-skills", "--skip-search", "--skip-ui", "--skip-hooks", "--suppress-gateway-token-output"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut env = vec![];
        match provider.filter(|_| o.change_ai) {
            Some(p) if !p.needs_key || !o.api_key.trim().is_empty() => {
                args.extend(["--auth-choice".into(), p.auth_choice.into()]);
                if p.needs_key {
                    args.extend(["--secret-input-mode".into(), "plaintext".into()]);
                    env.push((p.env.to_string(), o.api_key.trim().to_string()));
                } else {
                    if !o.ollama_url.trim().is_empty() {
                        args.extend(["--custom-base-url".into(), o.ollama_url.trim().into()]);
                    }
                    if !o.model.trim().is_empty() {
                        args.extend(["--custom-model-id".into(), o.model.trim().into()]);
                    }
                }
            }
            _ => args.extend(["--auth-choice".into(), "skip".into()]),
        }
        if o.change_bot {
            if !o.bot_name.trim().is_empty() {
                args.extend(["--agent-name".into(), o.bot_name.trim().into()]);
            }
            if !o.workspace.trim().is_empty() {
                args.extend(["--workspace".into(), o.workspace.trim().into()]);
            }
        }
        if let Some(p) = port {
            args.extend(["--gateway-port".into(), p.to_string()]);
        }
        args.extend(["--gateway-bind".into(), BINDS[o.bind.min(3)].0.into()]);
        plan.push(step("Crear la configuración de OpenClaw", Action::Cli { args, env }));
    } else if o.change_bot && !o.bot_name.trim().is_empty() {
        // Adds a Bot when one with that name doesn't exist yet.
        let workspace = if o.workspace.trim().is_empty() {
            dirs::home_dir().unwrap_or_default().join(".openclaw").join(format!("workspace-{}", o.bot_name.trim().to_lowercase())).to_string_lossy().into_owned()
        } else {
            o.workspace.trim().to_string()
        };
        let args = vec!["agents".to_string(), "add".to_string(), o.bot_name.trim().to_string(), "--workspace".to_string(), workspace, "--non-interactive".to_string()];
        plan.push(step("Crear tu Bot", Action::Cli { args, env: vec![] }));
    }

    if let Some((patch, replace)) = settings_patch(o, fresh) {
        plan.push(step("Aplicar tus ajustes", Action::Patch { patch, replace }));
    }
    if let Some((patch, replace)) = advanced {
        if patch.as_object().is_some_and(|m| !m.is_empty()) {
            plan.push(step("Aplicar la configuración avanzada", Action::Patch { patch, replace }));
        }
    }
    let changed_service_settings = o.change_gateway || fresh;
    if found.node_of.is_some() && !o.change_gateway {
        // A node of another computer's gateway: there is nothing to turn on here.
    } else if o.service && changed_service_settings {
        plan.push(step("Instalar y encender el servicio", Action::Cli { args: strings(&["gateway", "install", "--force"]), env: vec![] }));
    } else if !found.gateway_up {
        plan.push(step("Encender OpenClaw", Action::Cli { args: strings(&["gateway", "start"]), env: vec![] }));
    } else if plan.iter().any(|s| matches!(s.action, Action::Patch { .. })) && !o.service {
        plan.push(step("Reiniciar OpenClaw", Action::Cli { args: strings(&["gateway", "restart"]), env: vec![] }));
    }
    let needs_gateway = !fresh && o.change_ai && found.node_of.is_none();
    if needs_gateway {
        plan.push(step("Esperar a OpenClaw", Action::WaitGateway));
        if let Some(p) = provider {
            if p.needs_key && !o.api_key.trim().is_empty() {
                plan.push(step("Guardar la clave del modelo", Action::SetKey { provider: p.id.to_string(), key: o.api_key.trim().to_string() }));
            }
            if !o.model.trim().is_empty() {
                plan.push(step("Elegir el modelo principal", Action::SetModel { provider: p.id.to_string(), model: o.model.trim().to_string() }));
            }
        }
    }
    plan
}

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Settings that go in as one patch, so secrets travel through stdin only.
fn settings_patch(o: &Options, fresh: bool) -> Option<(Value, Vec<String>)> {
    let mut patch = json!({});
    if o.change_gateway {
        if let Ok(p) = o.port.trim().parse::<u16>() {
            patch["gateway"]["port"] = json!(p);
        }
        patch["gateway"]["bind"] = json!(BINDS[o.bind.min(3)].0);
        let tail = ["off", "serve", "funnel"][o.tailscale.min(2)];
        patch["gateway"]["tailscale"] = json!({"mode": tail});
        let secret = o.own_secret.trim();
        match o.auth {
            0 => {
                // Keep or generate the token.
                if !fresh {
                    patch["gateway"]["auth"] = json!({"mode": "token"});
                }
            }
            1 if !secret.is_empty() => patch["gateway"]["auth"] = json!({"mode": "token", "token": secret}),
            2 if !secret.is_empty() => patch["gateway"]["auth"] = json!({"mode": "password", "password": secret}),
            3 => patch["gateway"]["auth"] = json!({"mode": "none"}),
            _ => {}
        }
    }
    if o.change_channels {
        if !o.telegram.trim().is_empty() {
            patch["channels"]["telegram"] = json!({"enabled": true, "botToken": o.telegram.trim()});
        }
        if !o.discord.trim().is_empty() {
            patch["channels"]["discord"] = json!({"enabled": true, "token": o.discord.trim()});
        }
        if !o.slack_bot.trim().is_empty() && !o.slack_app.trim().is_empty() {
            patch["channels"]["slack"] = json!({"enabled": true, "mode": "socket", "botToken": o.slack_bot.trim(), "appToken": o.slack_app.trim()});
        }
    }
    if patch.as_object().is_some_and(|m| m.is_empty()) {
        None
    } else {
        Some((patch, vec![]))
    }
}

// ---- running the plan ----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum StepState {
    Waiting,
    Running,
    Done,
    Skipped,
    Failed(String),
}

pub struct Progress {
    pub steps: Vec<(PlanStep, StepState)>,
    pub output: Vec<String>,
    pub running: bool,
    pub finished: bool,
}

#[derive(Clone)]
pub struct Runtime {
    pub options: Options,
    pub sandbox: Option<PathBuf>,
}

fn say(p: &Arc<Mutex<Progress>>, line: impl Into<String>) {
    if let Ok(mut g) = p.lock() {
        g.output.push(line.into());
        if g.output.len() > 2000 {
            g.output.drain(..200);
        }
    }
}

/// Runs a command and shows its output as it comes.
fn stream(mut cmd: Command, stdin: Option<String>, p: &Arc<Mutex<Progress>>) -> Result<(), String> {
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("No se pudo iniciar: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let mut pumps = vec![];
    let streams: Vec<Box<dyn std::io::Read + Send>> = vec![
        Box::new(child.stdout.take().ok_or("sin salida")?),
        Box::new(child.stderr.take().ok_or("sin salida")?),
    ];
    for s in streams {
        let p = p.clone();
        pumps.push(thread::spawn(move || {
            for line in BufReader::new(s).split(b'\n').map_while(Result::ok) {
                say(&p, cli::strip_ansi(&String::from_utf8_lossy(&line)));
            }
        }));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    for h in pumps {
        let _ = h.join();
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("Terminó con error ({})", status.code().unwrap_or(-1)))
    }
}

fn install_dir(rt: &Runtime) -> PathBuf {
    PathBuf::from(rt.options.dir.trim())
}

fn powershell(script: &str) -> Command {
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]);
    cli::hide_console(&mut c);
    c
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn reg_add(key: &str, name: &str, value: &str, dword: bool) -> Result<(), String> {
    let mut c = Command::new("reg");
    c.args(["add", key, "/v", name, "/t", if dword { "REG_DWORD" } else { "REG_SZ" }, "/d", value, "/f"]);
    cli::hide_console(&mut c);
    let out = c.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("No se pudo escribir en el registro: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn reg_delete(key: &str, value: Option<&str>) {
    let mut c = Command::new("reg");
    c.args(["delete", key]);
    if let Some(v) = value {
        c.args(["/v", v]);
    }
    c.arg("/f");
    cli::hide_console(&mut c);
    let _ = c.output();
}

/// After a self-update, shows the new version in the Windows list of programs.
pub fn refresh_registered_version() {
    let mut c = Command::new("reg");
    c.args(["query", UNINSTALL_KEY, "/v", "DisplayName"]);
    cli::hide_console(&mut c);
    if c.output().is_ok_and(|o| o.status.success()) {
        let _ = reg_add(UNINSTALL_KEY, "DisplayVersion", env!("CARGO_PKG_VERSION"), false);
    }
}

fn start_menu_link() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join(r"Microsoft\Windows\Start Menu\Programs").join("OpenClaw Bot.lnk")
}

fn desktop_link() -> Option<PathBuf> {
    dirs::desktop_dir().map(|d| d.join("OpenClaw Bot.lnk"))
}

fn make_shortcut(link: &Path, target: &Path) -> Result<(), String> {
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut({});$s.TargetPath={};$s.WorkingDirectory={};$s.Save()",
        ps_quote(&link.to_string_lossy()),
        ps_quote(&target.to_string_lossy()),
        ps_quote(&target.parent().unwrap_or(Path::new(".")).to_string_lossy()),
    );
    let out = powershell(&script).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("No se pudo crear el acceso directo: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Connects to the local OpenClaw with this app's identity.
struct Session {
    client: Client,
    rx: std::sync::mpsc::Receiver<NetEvent>,
    store: SecretStore,
}

impl Session {
    fn open(limit: Duration) -> Result<Self, String> {
        let mut store = SecretStore::open();
        let identity = DeviceIdentity::load_or_create(&mut store);
        let target = target_for(&Profile::local(), &store);
        let (client, rx) = Client::start(target, identity, || {});
        let mut s = Self { client, rx, store };
        let until = Instant::now() + limit;
        loop {
            match s.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(NetEvent::Hello(_)) => return Ok(s),
                Ok(NetEvent::State(ConnState::AuthFailed(m))) => return Err(m),
                Ok(NetEvent::DeviceToken { route, token }) => {
                    s.store.set(&device_token_key(&route), &token);
                }
                Ok(NetEvent::ForgetDeviceToken { route }) => s.store.delete(&device_token_key(&route)),
                _ => {}
            }
            if Instant::now() > until {
                return Err("OpenClaw no respondió a tiempo.".into());
            }
        }
    }

    fn call(&mut self, method: &str, params: Value, limit: Duration) -> Result<Value, String> {
        let id = self.client.request(method, params);
        let until = Instant::now() + limit;
        loop {
            match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(NetEvent::Response { id: got, result }) if got == id => {
                    return result.map_err(|e| if e.message.is_empty() { e.code } else { e.message });
                }
                Ok(NetEvent::DeviceToken { route, token }) => {
                    self.store.set(&device_token_key(&route), &token);
                }
                _ => {}
            }
            if Instant::now() > until {
                return Err("OpenClaw no respondió a tiempo.".into());
            }
        }
    }
}

fn run_action(action: &Action, rt: &Runtime, p: &Arc<Mutex<Progress>>) -> Result<(), String> {
    let o = &rt.options;
    let dir = install_dir(rt);
    let sandboxed = rt.sandbox.is_some();
    let sandbox_note = |what: &str| say(p, format!("(prueba: {what} omitido)"));
    match action {
        Action::CopyApp => {
            std::fs::create_dir_all(&dir).map_err(|e| format!("No se pudo crear la carpeta: {e}"))?;
            let here = std::env::current_exe().map_err(|e| e.to_string())?;
            let dest = dir.join(EXE_NAME);
            if here == dest {
                say(p, "Ya estaba instalado aquí.");
                return Ok(());
            }
            std::fs::copy(&here, &dest).map_err(|e| {
                if e.raw_os_error() == Some(32) {
                    "OpenClaw Bot está abierto. Ciérralo y reintenta.".to_string()
                } else {
                    format!("No se pudo copiar el programa: {e}")
                }
            })?;
            say(p, format!("Instalado en {}", dest.display()));
            Ok(())
        }
        Action::Shortcuts => {
            let target = dir.join(EXE_NAME);
            if sandboxed {
                sandbox_note("accesos directos");
                return Ok(());
            }
            if o.start_menu {
                make_shortcut(&start_menu_link(), &target)?;
            }
            if o.desktop {
                if let Some(l) = desktop_link() {
                    make_shortcut(&l, &target)?;
                }
            }
            Ok(())
        }
        Action::Register => {
            let target = dir.join(EXE_NAME);
            if sandboxed {
                sandbox_note("registro de Windows");
                return Ok(());
            }
            let t = target.to_string_lossy();
            reg_add(UNINSTALL_KEY, "DisplayName", APP_TITLE, false)?;
            reg_add(UNINSTALL_KEY, "DisplayVersion", env!("CARGO_PKG_VERSION"), false)?;
            reg_add(UNINSTALL_KEY, "Publisher", "elgodox", false)?;
            reg_add(UNINSTALL_KEY, "InstallLocation", &dir.to_string_lossy(), false)?;
            reg_add(UNINSTALL_KEY, "DisplayIcon", &t, false)?;
            reg_add(UNINSTALL_KEY, "UninstallString", &format!("\"{t}\" --uninstall"), false)?;
            reg_add(UNINSTALL_KEY, "NoModify", "1", true)?;
            reg_add(UNINSTALL_KEY, "NoRepair", "1", true)?;
            if o.autostart {
                reg_add(RUN_KEY, "OpenClawBot", &format!("\"{t}\""), false)?;
            } else {
                reg_delete(RUN_KEY, Some("OpenClawBot"));
            }
            Ok(())
        }
        Action::SaveApp => {
            if sandboxed {
                sandbox_note("guardado de conexiones");
                return Ok(());
            }
            let mut settings = AppSettings::load();
            settings.theme = o.theme;
            settings.notifications = o.notifications;
            let mut store = SecretStore::open();
            let mut first_remote = None;
            if o.use_remote {
                for d in &o.remotes {
                    let profile = Profile {
                        name: d.name.trim().to_string(),
                        kind: if d.ssh { ProfileKind::Ssh } else { ProfileKind::Direct },
                        url: d.url.trim().to_string(),
                        ssh_target: d.ssh_target.trim().to_string(),
                        ssh_identity: d.ssh_identity.trim().to_string(),
                        remote_port: d.remote_port.trim().parse().unwrap_or(crate::settings::DEFAULT_PORT),
                        secret_kind: if d.secret.trim().is_empty() { SecretKind::None } else if d.password { SecretKind::Password } else { SecretKind::Token },
                        tls_fingerprint: d.fingerprint.trim().to_string(),
                        admin: d.admin,
                        ..Default::default()
                    };
                    if !d.secret.trim().is_empty() {
                        store.set(&profile.secret_key(), d.secret.trim());
                    }
                    first_remote.get_or_insert(profile.id.clone());
                    settings.profiles.retain(|x| x.name != profile.name || x.kind == ProfileKind::Local);
                    settings.profiles.push(profile);
                }
            }
            if !o.use_local {
                if let Some(id) = first_remote {
                    settings.active = id;
                }
            }
            settings.setup_done = true;
            settings.save();
            Ok(())
        }
        Action::InstallOpenClaw => {
            if sandboxed {
                sandbox_note("instalación de OpenClaw");
                return Ok(());
            }
            stream(cli::official_installer(&o.tag), None, p)?;
            if cli::find_local().is_none() {
                return Err("La instalación terminó pero no encuentro la orden de OpenClaw.".into());
            }
            Ok(())
        }
        Action::Cli { args, env } => {
            if sandboxed {
                sandbox_note(&format!("openclaw {}", args.first().map(String::as_str).unwrap_or("")));
                return Ok(());
            }
            let mut c = cli::build(&Host::Local, args)?;
            for (k, v) in env {
                c.env(k, v);
            }
            say(p, format!("openclaw {}", args.join(" ")));
            stream(c, None, p)
        }
        Action::Patch { patch, replace } => {
            if sandboxed {
                sandbox_note("ajustes de OpenClaw");
                return Ok(());
            }
            let mut args = strings(&["config", "patch", "--stdin"]);
            for r in replace {
                args.push("--replace-path".into());
                args.push(r.clone());
            }
            let c = cli::build(&Host::Local, &args)?;
            stream(c, Some(patch.to_string()), p)
        }
        Action::WaitGateway => {
            if sandboxed {
                sandbox_note("espera de OpenClaw");
                return Ok(());
            }
            let until = Instant::now() + Duration::from_secs(90);
            loop {
                let port = detect_local().port;
                if std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(1)).is_ok() {
                    say(p, format!("OpenClaw responde en el puerto {port}."));
                    return Ok(());
                }
                if Instant::now() > until {
                    return Err("OpenClaw no encendió a tiempo. Revisa el paso anterior.".into());
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
        Action::SetKey { provider, key } => {
            if sandboxed {
                sandbox_note("guardado de la clave");
                return Ok(());
            }
            let mut s = Session::open(Duration::from_secs(60))?;
            s.call("models.authSetApiKey", json!({"provider": provider, "apiKey": key}), Duration::from_secs(60))?;
            Ok(())
        }
        Action::SetModel { provider, model } => {
            if sandboxed {
                sandbox_note("elección del modelo");
                return Ok(());
            }
            let mut s = Session::open(Duration::from_secs(60))?;
            let full = if model.contains('/') { model.clone() } else { format!("{provider}/{model}") };
            let got = s.call("config.get", json!({}), Duration::from_secs(30))?;
            let mut params = json!({"raw": json!({"agents": {"defaults": {"model": {"primary": full}}}}).to_string(), "note": "OpenClaw Bot"});
            if let Some(h) = got["hash"].as_str() {
                params["baseHash"] = json!(h);
            }
            s.call("config.patch", params, Duration::from_secs(60))?;
            Ok(())
        }
    }
}

fn spawn_runner(progress: Arc<Mutex<Progress>>, rt: Runtime, repaint: impl Fn() + Send + 'static) {
    thread::spawn(move || {
        loop {
            let next = {
                let mut g = progress.lock().unwrap();
                match g.steps.iter().position(|(_, st)| *st == StepState::Waiting) {
                    Some(i) => {
                        g.steps[i].1 = StepState::Running;
                        Some((i, g.steps[i].0.clone()))
                    }
                    None => None,
                }
            };
            let Some((i, s)) = next else { break };
            say(&progress, format!("— {}", s.title));
            repaint();
            let result = run_action(&s.action, &rt, &progress);
            let mut g = progress.lock().unwrap();
            match result {
                Ok(()) => g.steps[i].1 = StepState::Done,
                Err(e) => {
                    g.output.push(format!("Error: {e}"));
                    g.steps[i].1 = StepState::Failed(e);
                    g.running = false;
                    drop(g);
                    repaint();
                    return;
                }
            }
            drop(g);
            repaint();
        }
        let mut g = progress.lock().unwrap();
        g.running = false;
        g.finished = true;
        drop(g);
        repaint();
    });
}

// ---- the assistant window --------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    Welcome,
    Where,
    Local,
    Gateway,
    Ai,
    Bot,
    Channels,
    Remote,
    Advanced,
    Prefs,
    Review,
    Install,
}

const PAGES: [(Page, &str); 12] = [
    (Page::Welcome, "Inicio"),
    (Page::Where, "Dónde instalar"),
    (Page::Local, "OpenClaw"),
    (Page::Gateway, "Conexión"),
    (Page::Ai, "Inteligencia"),
    (Page::Bot, "Tu Bot"),
    (Page::Channels, "Canales"),
    (Page::Remote, "OpenClaw remoto"),
    (Page::Advanced, "Avanzado"),
    (Page::Prefs, "Preferencias"),
    (Page::Review, "Resumen"),
    (Page::Install, "Instalar"),
];

pub struct SetupApp {
    o: Options,
    found: Arc<Mutex<Found>>,
    page: Page,
    editor: ConfigEditor,
    editor_state: Arc<Mutex<EditorLoad>>,
    progress: Arc<Mutex<Progress>>,
    started: bool,
    sandbox: Option<PathBuf>,
    auto: bool,
    error: Option<String>,
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_sent: bool,
    shot_done: Arc<AtomicBool>,
    configured_defaults: bool,
    installed_exe: Option<PathBuf>,
}

#[derive(Default)]
struct EditorLoad {
    started: bool,
    loaded: Option<(Value, Value)>,
    error: Option<String>,
}

fn r(n: u8) -> CornerRadius {
    CornerRadius::same(n)
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

impl SetupApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: &[String], screenshot: Option<PathBuf>) -> Self {
        look::load_fonts(&cc.egui_ctx);
        let mut o = Options::new();
        let sandbox = arg_value(args, "--setup-sandbox").map(PathBuf::from);
        if let Some(d) = &sandbox {
            o.dir = d.to_string_lossy().into_owned();
        }
        let page = arg_value(args, "--view")
            .and_then(|v| PAGES.iter().find(|(p, _)| format!("{p:?}").eq_ignore_ascii_case(&v)).map(|(p, _)| *p))
            .unwrap_or(Page::Welcome);
        let found = Arc::new(Mutex::new(Found::default()));
        if screenshot.is_none() {
            let f = found.clone();
            let ctx = cc.egui_ctx.clone();
            thread::spawn(move || {
                let local = detect_local();
                let gateway_up = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], local.port).into(), Duration::from_millis(800)).is_ok();
                let version = cli::local_version();
                let has_remote_profiles = AppSettings::load().profiles.iter().any(|p| p.kind != ProfileKind::Local);
                let (node_of, node_token) = match local.remote {
                    Some(r) => (Some(r.url), r.token),
                    None => (None, None),
                };
                if let Ok(mut g) = f.lock() {
                    *g = Found { checked: true, version, has_config: local.config_found, gateway_up, port: local.port, node_of, node_token, has_remote_profiles };
                }
                ctx.request_repaint();
            });
        } else if let Ok(mut g) = found.lock() {
            // The picture shows a computer without OpenClaw, never this one's data.
            *g = Found { checked: true, version: None, has_config: false, gateway_up: false, port: 18789, ..Default::default() };
        }
        let auto = args.iter().any(|a| a == "--auto-install");
        Self {
            o,
            found,
            page,
            editor: ConfigEditor::default(),
            editor_state: Default::default(),
            progress: Arc::new(Mutex::new(Progress { steps: vec![], output: vec![], running: false, finished: false })),
            started: false,
            sandbox,
            auto,
            error: None,
            screenshot,
            frames: 0,
            shot_sent: false,
            shot_done: Arc::new(AtomicBool::new(false)),
            configured_defaults: false,
            installed_exe: None,
        }
    }

    fn found(&self) -> Found {
        self.found.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// An existing OpenClaw keeps its settings unless the person opts in.
    fn apply_detected_defaults(&mut self) {
        let f = self.found();
        if !f.checked || self.configured_defaults {
            return;
        }
        self.configured_defaults = true;
        if f.has_config {
            self.o.change_gateway = false;
            self.o.change_ai = false;
            self.o.change_bot = false;
            self.o.service = false;
        }
        if f.port != 0 {
            self.o.port = f.port.to_string();
        }
        if f.version.is_some() {
            self.o.install_openclaw = false;
        }
        if let Some(url) = &f.node_of {
            // This computer uses another computer's OpenClaw: manage that one.
            self.o.use_local = false;
            if f.has_remote_profiles {
                self.o.use_remote = false;
            } else {
                self.o.use_remote = true;
                if self.o.remotes.is_empty() && !url.is_empty() {
                    self.o.remotes.push(RemoteDraft {
                        name: "Gateway central".into(),
                        url: url.clone(),
                        secret: f.node_token.clone().unwrap_or_default(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    fn plan(&self) -> Vec<PlanStep> {
        let advanced = if self.editor.is_dirty() { Some(build_patch(&self.editor.edits, &self.editor.current)) } else { None };
        build_plan(&self.o, &self.found(), advanced)
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        let plan = self.plan();
        if let Ok(mut g) = self.progress.lock() {
            g.steps = plan.into_iter().map(|s| (s, StepState::Waiting)).collect();
            g.output.clear();
            g.running = true;
            g.finished = false;
        }
        self.installed_exe = Some(PathBuf::from(self.o.dir.trim()).join(EXE_NAME));
        let rt = Runtime { options: self.o.clone(), sandbox: self.sandbox.clone() };
        let c = ctx.clone();
        spawn_runner(self.progress.clone(), rt, move || c.request_repaint());
        self.started = true;
    }

    fn retry(&mut self, ctx: &egui::Context, skip_failed: bool) {
        if let Ok(mut g) = self.progress.lock() {
            for (_, st) in g.steps.iter_mut() {
                if matches!(st, StepState::Failed(_)) {
                    *st = if skip_failed { StepState::Skipped } else { StepState::Waiting };
                }
            }
            g.running = true;
            g.finished = false;
        }
        let rt = Runtime { options: self.o.clone(), sandbox: self.sandbox.clone() };
        let c = ctx.clone();
        spawn_runner(self.progress.clone(), rt, move || c.request_repaint());
    }

    fn visible_pages(&self) -> Vec<(Page, &'static str)> {
        PAGES
            .iter()
            .copied()
            .filter(|(p, _)| match p {
                Page::Local | Page::Gateway | Page::Ai | Page::Bot | Page::Channels | Page::Advanced => self.o.use_local,
                Page::Remote => self.o.use_remote,
                _ => true,
            })
            .collect()
    }

    fn validate(&self, page: Page) -> Option<String> {
        match page {
            Page::Where if self.o.dir.trim().is_empty() => Some("Elige una carpeta para instalar.".into()),
            Page::Welcome if !self.o.use_local && !self.o.use_remote && !self.found().has_remote_profiles => Some("Elige al menos una opción.".into()),
            Page::Gateway if self.o.use_local && self.o.change_gateway => {
                if self.o.port.trim().parse::<u16>().map_or(true, |p| p == 0) {
                    Some("El puerto debe ser un número entre 1 y 65535.".into())
                } else if matches!(self.o.auth, 1 | 2) && self.o.own_secret.trim().is_empty() {
                    Some("Escribe la clave que quieres usar.".into())
                } else if self.o.auth == 3 && self.o.bind != 0 {
                    Some("Sin clave, OpenClaw solo puede abrirse a este equipo.".into())
                } else {
                    None
                }
            }
            Page::Ai if self.o.use_local && self.o.change_ai => {
                let p = &PROVIDERS[self.o.provider];
                (p.needs_key && self.o.api_key.trim().is_empty()).then(|| "Pega la clave, o desmarca esta sección para hacerlo después.".to_string())
            }
            Page::Channels if self.o.use_local && self.o.change_channels => {
                let one = self.o.slack_bot.trim().is_empty() != self.o.slack_app.trim().is_empty();
                one.then(|| "Slack necesita las dos claves.".to_string())
            }
            Page::Remote if self.o.use_remote => {
                if self.o.remotes.is_empty() {
                    return Some("Añade al menos un OpenClaw remoto.".into());
                }
                for d in &self.o.remotes {
                    if !d.ssh && !(d.url.starts_with("ws://") || d.url.starts_with("wss://")) {
                        return Some(format!("La dirección de \"{}\" debe empezar con ws:// o wss://", d.name));
                    }
                    if d.ssh && d.ssh_target.trim().is_empty() {
                        return Some(format!("Escribe el equipo SSH de \"{}\"", d.name));
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn load_editor(&mut self, ctx: &egui::Context) {
        {
            let mut g = self.editor_state.lock().unwrap();
            if g.started {
                return;
            }
            g.started = true;
            g.error = None;
        }
        let state = self.editor_state.clone();
        let c = ctx.clone();
        thread::spawn(move || {
            let host = Host::Local;
            let result = (|| -> Result<(Value, Value), String> {
                let out = cli::capture(&host, &strings(&["config", "schema", "--json"]), None, Duration::from_secs(90))?;
                if out.code != 0 {
                    return Err(out.stderr);
                }
                let text = out.stdout;
                let start = text.find('{').ok_or("OpenClaw no devolvió sus ajustes.")?;
                let schema: Value = serde_json::from_str(&text[start..]).map_err(|e| e.to_string())?;
                let file = cli::capture(&host, &strings(&["config", "file", "--json"]), None, Duration::from_secs(30))?;
                let path = serde_json::from_str::<Value>(file.stdout.trim()).ok().and_then(|v| v["path"].as_str().map(PathBuf::from));
                let mut current = json!({});
                if let Some(p) = &path {
                    if let Ok(t) = std::fs::read_to_string(p) {
                        current = json5::from_str(&t).unwrap_or_else(|_| json!({}));
                    }
                }
                let wrapped = if schema.get("uiHints").is_some() { schema } else { json!({"schema": schema, "uiHints": {}}) };
                Ok((wrapped, json!({"sourceConfig": current})))
            })();
            let mut g = state.lock().unwrap();
            match result {
                Ok(v) => g.loaded = Some(v),
                Err(e) => {
                    g.error = Some(e);
                    g.started = false;
                }
            }
            drop(g);
            c.request_repaint();
        });
    }

    fn take_loaded_editor(&mut self) {
        let loaded = self.editor_state.lock().ok().and_then(|mut g| g.loaded.take());
        if let Some((schema, current)) = loaded {
            self.editor.set_schema(&schema);
            self.editor.set_current(&current);
        }
    }

    fn step_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        ctx.request_repaint();
        self.frames += 1;
        if self.shot_done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.frames >= 120 && !self.shot_sent {
            self.shot_sent = true;
            let done = self.shot_done.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img: Arc<egui::ColorImage>| {
                if let Some(d) = path.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let [w, h] = img.size;
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                    let _ = buf.save(&path);
                }
                done.store(true, Ordering::SeqCst);
            })));
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        match self.page {
            Page::Welcome => self.page_welcome(ui, p),
            Page::Where => self.page_where(ui),
            Page::Local => self.page_local(ui, p),
            Page::Gateway => self.page_gateway(ui, p),
            Page::Ai => self.page_ai(ui, p),
            Page::Bot => self.page_bot(ui, p),
            Page::Channels => self.page_channels(ui, p),
            Page::Remote => self.page_remote(ui, p),
            Page::Advanced => self.page_advanced(ui, p, ctx),
            Page::Prefs => self.page_prefs(ui),
            Page::Review => self.page_review(ui, p),
            Page::Install => self.page_install(ui, p, ctx),
        }
    }

    fn page_welcome(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Bienvenido a OpenClaw Bot");
        note(ui, p, "Todo OpenClaw en una sola app. Este asistente deja listo lo que elijas; lo demás se puede cambiar después en Configurar.");
        ui.add_space(14.0);
        ui.label(RichText::new("¿Qué OpenClaw quieres administrar?").strong());
        ui.add_space(4.0);
        ui.checkbox(&mut self.o.use_local, "El de este equipo");
        ui.checkbox(&mut self.o.use_remote, "Uno en otro equipo (por red, Tailscale o túnel SSH)");
        let f = self.found();
        ui.add_space(14.0);
        if !f.checked {
            ui.horizontal(|ui| {
                ui.spinner();
                note(ui, p, "Mirando qué hay en este equipo…");
            });
        } else if let Some(url) = &f.node_of {
            let shown = if url.is_empty() { "otro equipo".to_string() } else { url.clone() };
            let saved = if f.has_remote_profiles { " Tus conexiones guardadas se conservan." } else { "" };
            note(ui, p, format!("Este equipo usa el OpenClaw de {shown} (modo nodo). No instalaré ni encenderé un OpenClaw aquí.{saved}"));
        } else {
            match (&f.version, f.has_config) {
                (Some(v), true) => note(ui, p, format!("Encontré OpenClaw {v} ya configurado. No cambiaré nada de él a menos que lo marques.")),
                (Some(v), false) => note(ui, p, format!("Encontré OpenClaw {v}, todavía sin configurar.")),
                (None, _) => note(ui, p, "OpenClaw todavía no está en este equipo; lo instalaré si lo dejas marcado."),
            }
        }
    }

    fn page_where(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Dónde instalar la app");
        ui.add_space(8.0);
        ui.label("Carpeta");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut self.o.dir).desired_width(520.0));
            if ui.button("Elegir…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.o.dir = d.to_string_lossy().into_owned();
                }
            }
        });
        ui.add_space(10.0);
        ui.checkbox(&mut self.o.start_menu, "Acceso en el menú Inicio");
        ui.checkbox(&mut self.o.desktop, "Acceso en el escritorio");
        ui.checkbox(&mut self.o.autostart, "Abrir al iniciar sesión");
        ui.checkbox(&mut self.o.open_after, "Abrir al terminar");
    }

    fn page_local(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "OpenClaw en este equipo");
        let f = self.found();
        ui.add_space(6.0);
        match &f.version {
            Some(v) => {
                ui.label(RichText::new(format!("Ya está instalado (versión {v}).")).strong());
                note(ui, p, "Se conserva tal como está. Puedes actualizarlo después en Servicio.");
            }
            None => {
                ui.checkbox(&mut self.o.install_openclaw, "Descargar e instalar OpenClaw ahora");
                note(ui, p, "Se instala con el instalador oficial de OpenClaw (descarga Node si hace falta). Necesita Internet.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Versión");
                    for t in ["stable", "beta", "dev"] {
                        ui.selectable_value(&mut self.o.tag, t.to_string(), t);
                    }
                });
            }
        }
        ui.add_space(10.0);
        ui.checkbox(&mut self.o.service, "Mantener OpenClaw encendido en segundo plano");
        note(ui, p, "Lo instala como servicio de Windows para que arranque solo con el equipo.");
    }

    fn change_toggle(&mut self, ui: &mut egui::Ui, p: &Palette, which: usize, text: &str) -> bool {
        let f = self.found();
        let flag = match which {
            0 => &mut self.o.change_gateway,
            1 => &mut self.o.change_ai,
            2 => &mut self.o.change_bot,
            _ => &mut self.o.change_channels,
        };
        if f.has_config || which == 3 {
            ui.checkbox(flag, text);
            if !*flag {
                note(ui, p, "Se conserva lo que tienes ahora.");
            }
            ui.add_space(8.0);
        } else {
            *flag = true;
        }
        *flag
    }

    fn page_gateway(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Conexión de OpenClaw");
        note(ui, p, "La puerta por la que entran esta app y tus otros equipos.");
        ui.add_space(8.0);
        if !self.change_toggle(ui, p, 0, "Cambiar la conexión") {
            return;
        }
        egui::Grid::new("gw").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("Puerto");
            ui.add(TextEdit::singleline(&mut self.o.port).desired_width(100.0));
            ui.end_row();
            ui.label("Quién puede entrar");
            ui.vertical(|ui| {
                for (i, (_, l)) in BINDS.iter().enumerate() {
                    ui.radio_value(&mut self.o.bind, i, *l);
                }
            });
            ui.end_row();
            ui.label("Clave de acceso");
            ui.vertical(|ui| {
                ui.radio_value(&mut self.o.auth, 0, "Generar una automáticamente");
                ui.radio_value(&mut self.o.auth, 1, "Usar mi propio token");
                ui.radio_value(&mut self.o.auth, 2, "Usar una contraseña");
                ui.radio_value(&mut self.o.auth, 3, "Sin clave (solo este equipo)");
            });
            ui.end_row();
            if matches!(self.o.auth, 1 | 2) {
                ui.label("Clave");
                ui.add(TextEdit::singleline(&mut self.o.own_secret).password(true).desired_width(320.0));
                ui.end_row();
            }
            ui.label("Tailscale");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.o.tailscale, 0, "No usar");
                ui.radio_value(&mut self.o.tailscale, 1, "Solo mi red");
                ui.radio_value(&mut self.o.tailscale, 2, "Público");
            });
            ui.end_row();
        });
        if self.o.tailscale == 2 && self.o.auth != 2 {
            ui.add_space(6.0);
            note(ui, p, "Para exponerlo al público, OpenClaw exige usar contraseña.");
        }
    }

    fn page_ai(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "La inteligencia de tus Bots");
        note(ui, p, "Pega una clave ahora, o déjalo para después en la sección Modelos.");
        ui.add_space(8.0);
        let f = self.found();
        if f.has_config {
            ui.checkbox(&mut self.o.change_ai, "Agregar o cambiar un modelo");
            if !self.o.change_ai {
                note(ui, p, "Se conserva lo que tienes ahora.");
                return;
            }
            ui.add_space(8.0);
        } else {
            ui.checkbox(&mut self.o.change_ai, "Configurar el modelo ahora");
            if !self.o.change_ai {
                return;
            }
            ui.add_space(8.0);
        }
        ui.label("Proveedor");
        for (i, pr) in PROVIDERS.iter().enumerate() {
            ui.radio_value(&mut self.o.provider, i, pr.label);
        }
        ui.add_space(8.0);
        let provider = &PROVIDERS[self.o.provider];
        egui::Grid::new("ai").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            if provider.needs_key {
                ui.label("Clave");
                ui.add(TextEdit::singleline(&mut self.o.api_key).password(true).desired_width(360.0));
                ui.end_row();
            } else {
                ui.label("Dirección");
                ui.add(TextEdit::singleline(&mut self.o.ollama_url).hint_text("http://127.0.0.1:11434").desired_width(360.0));
                ui.end_row();
            }
            ui.label("Modelo");
            ui.add(TextEdit::singleline(&mut self.o.model).hint_text("Vacío para usar el recomendado").desired_width(360.0));
            ui.end_row();
        });
        ui.add_space(6.0);
        note(ui, p, "Para entrar con tu cuenta en vez de una clave, usa el asistente de OpenClaw desde Modelos, ya instalada la app.");
    }

    fn page_bot(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Tu primer Bot");
        ui.add_space(8.0);
        if !self.change_toggle(ui, p, 2, "Crear un Bot nuevo") {
            return;
        }
        egui::Grid::new("bot").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Nombre");
            ui.add(TextEdit::singleline(&mut self.o.bot_name).desired_width(320.0));
            ui.end_row();
            ui.label("Carpeta de trabajo");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut self.o.workspace).hint_text("Automática si la dejas vacía").desired_width(320.0));
                if ui.button("Elegir…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.o.workspace = d.to_string_lossy().into_owned();
                    }
                }
            });
            ui.end_row();
        });
    }

    fn page_channels(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Canales");
        note(ui, p, "Por dónde podrás hablarles a tus Bots. WhatsApp y los demás se vinculan desde la app con su código.");
        ui.add_space(8.0);
        ui.checkbox(&mut self.o.change_channels, "Conectar ahora Telegram, Discord o Slack");
        if !self.o.change_channels {
            return;
        }
        ui.add_space(8.0);
        egui::Grid::new("ch").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Telegram");
            ui.add(TextEdit::singleline(&mut self.o.telegram).password(true).hint_text("Token del bot (de BotFather)").desired_width(380.0));
            ui.end_row();
            ui.label("Discord");
            ui.add(TextEdit::singleline(&mut self.o.discord).password(true).hint_text("Token del bot").desired_width(380.0));
            ui.end_row();
            ui.label("Slack (bot)");
            ui.add(TextEdit::singleline(&mut self.o.slack_bot).password(true).hint_text("xoxb-…").desired_width(380.0));
            ui.end_row();
            ui.label("Slack (app)");
            ui.add(TextEdit::singleline(&mut self.o.slack_app).password(true).hint_text("xapp-…").desired_width(380.0));
            ui.end_row();
        });
    }

    fn page_remote(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "OpenClaw en otros equipos");
        note(ui, p, "La primera vez, ese equipo tendrá que aprobar a este. La app te avisa cuando haga falta.");
        ui.add_space(8.0);
        let mut remove = None;
        let count = self.o.remotes.len();
        ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 50.0).show(ui, |ui| {
            for (i, d) in self.o.remotes.iter_mut().enumerate() {
                Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(r(8)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    egui::Grid::new(("remote", i)).num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Nombre");
                        ui.add(TextEdit::singleline(&mut d.name).desired_width(360.0));
                        ui.end_row();
                        ui.label("Cómo llegar");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut d.ssh, false, "Por red o Tailscale");
                            ui.selectable_value(&mut d.ssh, true, "Túnel SSH");
                        });
                        ui.end_row();
                        if d.ssh {
                            ui.label("Equipo SSH");
                            ui.add(TextEdit::singleline(&mut d.ssh_target).hint_text("usuario@equipo").desired_width(360.0));
                            ui.end_row();
                            ui.label("Puerto remoto");
                            ui.add(TextEdit::singleline(&mut d.remote_port).desired_width(100.0));
                            ui.end_row();
                            ui.label("Clave SSH");
                            ui.add(TextEdit::singleline(&mut d.ssh_identity).hint_text("Opcional").desired_width(360.0));
                            ui.end_row();
                        } else {
                            ui.label("Dirección");
                            ui.add(TextEdit::singleline(&mut d.url).hint_text("wss://mi-equipo.ts.net o ws://192.168.1.20:18789").desired_width(360.0));
                            ui.end_row();
                            ui.label("Huella TLS");
                            ui.add(TextEdit::singleline(&mut d.fingerprint).hint_text("Solo con certificado propio").desired_width(360.0));
                            ui.end_row();
                        }
                        ui.label("Clave");
                        ui.horizontal(|ui| {
                            ui.add(TextEdit::singleline(&mut d.secret).password(true).hint_text("Token o contraseña").desired_width(240.0));
                            ui.checkbox(&mut d.password, "Es contraseña");
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut d.admin, "Permitir administrar");
                        ui.end_row();
                    });
                    if ui.small_button("Quitar").clicked() {
                        remove = Some(i);
                    }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(i) = remove {
            self.o.remotes.remove(i);
        }
        if ui.button("+ Añadir otro").clicked() || (count == 0 && self.o.use_remote) {
            self.o.remotes.push(RemoteDraft::default());
        }
    }

    fn page_advanced(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        heading(ui, "Configuración avanzada");
        note(ui, p, "Todos los ajustes de OpenClaw, por si quieres afinar algo desde ya. Es opcional: también están en la app.");
        ui.add_space(6.0);
        self.take_loaded_editor();
        if !self.editor.has_schema() {
            let f = self.found();
            if f.version.is_none() {
                note(ui, p, "Se podrá abrir cuando OpenClaw ya esté instalado. Sigue adelante; podrás usarla en la app.");
                return;
            }
            let (started, err) = {
                let g = self.editor_state.lock().unwrap();
                (g.started, g.error.clone())
            };
            if let Some(e) = err {
                ui.label(RichText::new(crate::model::shorten(&e, 300)).color(p.red));
            }
            if started {
                ui.horizontal(|ui| {
                    ui.spinner();
                    note(ui, p, "Leyendo los ajustes de OpenClaw…");
                });
            } else if ui.button("Abrir la configuración avanzada").clicked() {
                self.load_editor(ctx);
            }
            return;
        }
        let full = ui.available_rect_before_wrap();
        let left_w = 190.0;
        let left = egui::Rect::from_min_max(full.min, egui::pos2(full.left() + left_w, full.bottom()));
        let right = egui::Rect::from_min_max(egui::pos2(full.left() + left_w + 14.0, full.top()), full.max);
        let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(left));
        lui.add(TextEdit::singleline(&mut self.editor.search).hint_text("Buscar").desired_width(left_w));
        ScrollArea::vertical().id_salt("adv-groups").auto_shrink([false, false]).show(&mut lui, |ui| {
            if self.editor.search.trim().is_empty() {
                for (key, label) in self.editor.groups.clone() {
                    let on = self.editor.group.as_deref() == Some(&key) && self.editor.focus.is_none();
                    if ui.selectable_label(on, label).clicked() {
                        self.editor.group = Some(key);
                        self.editor.focus = None;
                    }
                }
            } else {
                for row in self.editor.search_results(100) {
                    let label = if row.label.is_empty() { row.segs.join(".") } else { row.label.clone() };
                    if ui.selectable_label(false, crate::model::shorten(&label, 28)).clicked() {
                        self.editor.focus = Some(row.segs);
                    }
                }
            }
        });
        let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right));
        rui.set_clip_rect(right);
        self.editor.form_ui(&mut rui, p.text2, p.accent, p.amber);
        ui.allocate_rect(full, Sense::hover());
    }

    fn page_prefs(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Preferencias de la app");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Tema");
            ui.selectable_value(&mut self.o.theme, Theme::Dark, "Oscuro");
            ui.selectable_value(&mut self.o.theme, Theme::Light, "Claro");
        });
        ui.add_space(6.0);
        ui.checkbox(&mut self.o.notifications, "Avisarme cuando un Bot necesite respuesta");
    }

    fn page_review(&mut self, ui: &mut egui::Ui, p: &Palette) {
        heading(ui, "Resumen");
        note(ui, p, "Esto es lo que haré. Puedes volver atrás para cambiar lo que quieras.");
        ui.add_space(8.0);
        ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            for (i, s) in self.plan().iter().enumerate() {
                ui.label(format!("{}.  {}", i + 1, s.title));
            }
        });
    }

    fn page_install(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        let (steps, output, running, finished) = {
            let g = self.progress.lock().unwrap();
            (g.steps.clone(), g.output.clone(), g.running, g.finished)
        };
        if !self.started {
            heading(ui, "Listo para instalar");
            note(ui, p, "Pulsa Instalar para empezar. Puede tardar unos minutos.");
            return;
        }
        heading(ui, if finished { "¡Todo listo!" } else if running { "Instalando…" } else { "Falta poco" });
        ui.add_space(6.0);
        for (s, st) in &steps {
            ui.horizontal(|ui| {
                let (c, mark) = match st {
                    StepState::Waiting => (p.text3, "○"),
                    StepState::Running => (p.amber, "…"),
                    StepState::Done => (p.green, "✔"),
                    StepState::Skipped => (p.text2, "⊘"),
                    StepState::Failed(_) => (p.red, "✖"),
                };
                ui.label(RichText::new(mark).color(c));
                ui.label(&s.title);
                if *st == StepState::Running {
                    ui.spinner();
                }
            });
            if let StepState::Failed(e) = st {
                ui.label(RichText::new(e).size(12.0).color(p.red));
            }
        }
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Ver detalle").default_open(!finished && !running).show(ui, |ui| {
            Frame::new().fill(p.field).corner_radius(r(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                ScrollArea::vertical().max_height(190.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                    ui.label(RichText::new(output.join("\n")).monospace().size(11.0));
                });
            });
        });
        if !running && !finished {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new("Reintentar").color(Color32::WHITE)).fill(p.bubble)).clicked() {
                    self.retry(ctx, false);
                }
                if ui.button("Omitir este paso y seguir").clicked() {
                    self.retry(ctx, true);
                }
            });
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        let pages = self.visible_pages();
        let idx = pages.iter().position(|(pg, _)| *pg == self.page).unwrap_or(0);
        let last = idx + 1 == pages.len();
        ui.horizontal(|ui| {
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).size(12.5).color(p.red));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let finished = self.progress.lock().map(|g| g.finished).unwrap_or(false);
                let running = self.progress.lock().map(|g| g.running).unwrap_or(false);
                if self.page == Page::Install {
                    if finished {
                        if ui.add(Button::new(RichText::new("Terminar").color(Color32::WHITE)).fill(p.bubble).min_size(Vec2::new(110.0, 34.0))).clicked() {
                            if self.o.open_after && self.sandbox.is_none() {
                                if let Some(exe) = &self.installed_exe {
                                    let _ = Command::new(exe).spawn();
                                }
                            }
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    } else if !self.started && ui.add(Button::new(RichText::new("Instalar").color(Color32::WHITE)).fill(p.bubble).min_size(Vec2::new(110.0, 34.0))).clicked() {
                        self.error = None;
                        self.start_install(ctx);
                    }
                    if !running && !finished && ui.add(Button::new("Atrás").min_size(Vec2::new(80.0, 34.0))).clicked() {
                        self.started = false;
                        self.page = pages[idx - 1].0;
                    }
                    return;
                }
                if ui.add(Button::new(RichText::new("Siguiente").color(Color32::WHITE)).fill(p.bubble).min_size(Vec2::new(110.0, 34.0))).clicked() {
                    match self.validate(self.page) {
                        Some(e) => self.error = Some(e),
                        None => {
                            self.error = None;
                            if !last {
                                self.page = pages[idx + 1].0;
                            }
                        }
                    }
                }
                if idx > 0 && ui.add(Button::new("Atrás").min_size(Vec2::new(80.0, 34.0))).clicked() {
                    self.error = None;
                    self.page = pages[idx - 1].0;
                }
            });
        });
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(22.0).strong());
}

fn note(ui: &mut egui::Ui, p: &Palette, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(12.5).color(p.text2));
}

impl eframe::App for SetupApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_detected_defaults();
        self.step_screenshot(ctx);
        let (running, finished) = self.progress.lock().map(|g| (g.running, g.finished)).unwrap_or((false, false));
        if running {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.auto {
            if !self.started {
                if self.found().checked {
                    self.page = Page::Install;
                    self.start_install(ctx);
                }
                return;
            }
            if finished || !running {
                let g = self.progress.lock().unwrap();
                for (s, st) in &g.steps {
                    println!("{st:?}  {}", s.title);
                }
                for line in &g.output {
                    println!("{line}");
                }
                let ok = g.steps.iter().all(|(_, st)| *st == StepState::Done);
                std::process::exit(if ok { 0 } else { 1 });
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let p = look::palette(self.o.theme);
        look::apply_theme(ui.ctx(), &p, self.o.theme);
        let ctx = ui.ctx().clone();
        egui::Panel::left("steps").exact_size(190.0).resizable(false).frame(Frame::new().fill(p.sidebar).inner_margin(Margin::same(14))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                look::draw_bot(ui.painter(), rect, 0, 0, true);
                ui.label(RichText::new("Instalador").size(15.0).strong());
            });
            ui.add_space(14.0);
            let current = self.page;
            let pages = self.visible_pages();
            let idx = pages.iter().position(|(pg, _)| *pg == current).unwrap_or(0);
            for (i, (pg, label)) in pages.iter().enumerate() {
                let (color, strong) = if *pg == current { (p.text, true) } else if i < idx { (p.text2, false) } else { (p.text3, false) };
                let mut t = RichText::new(*label).size(13.0).color(color);
                if strong {
                    t = t.strong();
                }
                ui.label(t);
                ui.add_space(3.0);
            }
        });
        egui::Panel::bottom("footer").frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 12))).show(ui, |ui| self.footer(ui, &p, &ctx));
        egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(28, 22))).show(ui, |ui| self.body(ui, &p, &ctx));
    }
}

// ---- entry points --------------------------------------------------------------

pub fn run_setup(args: Vec<String>, screenshot: Option<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Instalador de OpenClaw Bot")
            .with_icon(crate::app_icon())
            .with_inner_size([940.0, 660.0])
            .with_min_inner_size([760.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native("OpenClaw Bot Setup", options, Box::new(move |cc| Ok(Box::new(SetupApp::new(cc, &args, screenshot.clone())))))
}

/// Removes the app. OpenClaw itself and its data stay unless asked for.
pub struct UninstallApp {
    remove_settings: bool,
    done: Option<String>,
}

fn uninstall(remove_settings: bool) -> String {
    let _ = std::fs::remove_file(start_menu_link());
    if let Some(l) = desktop_link() {
        let _ = std::fs::remove_file(l);
    }
    reg_delete(RUN_KEY, Some("OpenClawBot"));
    reg_delete(UNINSTALL_KEY, None);
    if remove_settings {
        let _ = std::fs::remove_file(AppSettings::path());
    }
    // The running program can't delete itself, so a short command does it once
    // this window has closed. `rmdir` without /s only removes an empty folder.
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_name().is_some_and(|n| n == EXE_NAME) {
            if let Some(dir) = exe.parent() {
                let line = format!("ping -n 3 127.0.0.1 >nul & del \"{}\" & rmdir \"{}\"", exe.display(), dir.display());
                let mut c = Command::new("cmd");
                c.args(["/D", "/C", &line]);
                cli::hide_console(&mut c);
                let _ = c.spawn();
            }
        }
    }
    "OpenClaw Bot se desinstaló.".to_string()
}

impl eframe::App for UninstallApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let p = look::palette(Theme::Dark);
        look::apply_theme(ui.ctx(), &p, Theme::Dark);
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(24))).show(ui, |ui| {
            heading(ui, "Desinstalar OpenClaw Bot");
            ui.add_space(8.0);
            if let Some(msg) = &self.done {
                ui.label(msg);
                if ui.button("Cerrar").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            note(ui, &p, "OpenClaw y tus Bots se quedan como están. Para quitarlos usa Servicio en la app, antes de desinstalar.");
            ui.add_space(8.0);
            ui.checkbox(&mut self.remove_settings, "Borrar también mis conexiones y preferencias");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new("Desinstalar").color(Color32::WHITE)).fill(p.red)).clicked() {
                    self.done = Some(uninstall(self.remove_settings));
                }
                if ui.button("Cancelar").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

pub fn run_uninstall() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Desinstalar OpenClaw Bot").with_icon(crate::app_icon()).with_inner_size([520.0, 280.0]),
        ..Default::default()
    };
    eframe::run_native("OpenClaw Bot Uninstall", options, Box::new(move |cc| {
        look::load_fonts(&cc.egui_ctx);
        Ok(Box::new(UninstallApp { remove_settings: false, done: None }))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_found() -> Found {
        Found { checked: true, version: None, has_config: false, gateway_up: false, port: 18789, ..Default::default() }
    }

    fn existing_found() -> Found {
        Found { checked: true, version: Some("2026.11.1".into()), has_config: true, gateway_up: true, port: 18789, ..Default::default() }
    }

    #[test]
    fn a_node_of_another_gateway_never_gets_one_started() {
        // gateway.mode = "remote": nothing listens here, and nothing should.
        let found = Found { gateway_up: false, node_of: Some("ws://100.105.45.37:18789".into()), ..existing_found() };
        let mut o = Options::new();
        o.change_gateway = false;
        o.change_ai = false;
        o.change_bot = false;
        o.service = false;
        let plan = build_plan(&o, &found, None);
        assert!(!plan.iter().any(|s| matches!(&s.action, Action::Cli { args, .. } if args[0] == "gateway")));
        assert!(!plan.iter().any(|s| s.action == Action::WaitGateway));
        o.service = true;
        let plan = build_plan(&o, &found, None);
        assert!(!plan.iter().any(|s| matches!(&s.action, Action::Cli { args, .. } if args[0] == "gateway")));
    }

    #[test]
    fn a_node_drops_this_computer_from_the_connections() {
        let remote = Profile { name: "Raspberry Pi".into(), url: "wss://pi:9443".into(), ..Default::default() };
        let mut s = AppSettings { profiles: vec![Profile::local(), remote.clone()], active: "local".into(), ..Default::default() };
        s.fit_local_profile(true);
        assert_eq!(s.profiles.len(), 1);
        assert_eq!(s.active, remote.id);
        // Without another connection, "Este equipo" stays so there is something to use.
        let mut lone = AppSettings::default();
        lone.fit_local_profile(true);
        assert_eq!(lone.profiles.len(), 1);
        // A computer with its own gateway keeps it.
        let mut own = AppSettings { profiles: vec![remote], ..Default::default() };
        own.fit_local_profile(false);
        assert!(own.profiles.iter().any(|p| p.kind == ProfileKind::Local));
    }

    #[test]
    fn a_fresh_computer_gets_everything_installed() {
        let mut o = Options::new();
        o.api_key = "sk-test-key".into();
        let plan = build_plan(&o, &fresh_found(), None);
        assert!(plan.iter().any(|s| s.action == Action::InstallOpenClaw));
        let onboard = plan.iter().find_map(|s| match &s.action {
            Action::Cli { args, env } if args[0] == "onboard" => Some((args, env)),
            _ => None,
        }).unwrap();
        assert!(onboard.0.contains(&"--non-interactive".to_string()));
        assert!(onboard.0.contains(&"--accept-risk".to_string()));
        // The secret travels through the environment, never the command line.
        assert!(!onboard.0.iter().any(|a| a.contains("sk-test-key")));
        assert_eq!(onboard.1[0], ("ANTHROPIC_API_KEY".to_string(), "sk-test-key".to_string()));
        assert!(plan.iter().any(|s| matches!(&s.action, Action::Cli { args, .. } if args[0] == "gateway" && args[1] == "install")));
    }

    #[test]
    fn an_existing_openclaw_is_left_alone_by_default() {
        let mut o = Options::new();
        o.change_gateway = false;
        o.change_ai = false;
        o.change_bot = false;
        o.service = false;
        let plan = build_plan(&o, &existing_found(), None);
        let touching: Vec<_> = plan.iter().filter(|s| matches!(s.action, Action::Cli { .. } | Action::Patch { .. } | Action::InstallOpenClaw | Action::SetKey { .. } | Action::SetModel { .. })).collect();
        assert!(touching.is_empty(), "no debería tocar OpenClaw: {touching:?}");
    }

    #[test]
    fn a_remote_only_setup_never_touches_openclaw_here() {
        let mut o = Options::new();
        o.use_local = false;
        o.use_remote = true;
        let plan = build_plan(&o, &fresh_found(), None);
        assert!(plan.iter().all(|s| matches!(s.action, Action::CopyApp | Action::Shortcuts | Action::Register | Action::SaveApp)));
    }

    #[test]
    fn gateway_secrets_are_only_in_the_patch() {
        let mut o = Options::new();
        o.auth = 2;
        o.own_secret = "mi-clave".into();
        o.bind = 1;
        let plan = build_plan(&o, &fresh_found(), None);
        let patch = plan.iter().find_map(|s| match &s.action { Action::Patch { patch, .. } => Some(patch.clone()), _ => None }).unwrap();
        assert_eq!(patch["gateway"]["auth"], json!({"mode": "password", "password": "mi-clave"}));
        assert_eq!(patch["gateway"]["bind"], "tailnet");
        for s in &plan {
            if let Action::Cli { args, .. } = &s.action {
                assert!(!args.iter().any(|a| a.contains("mi-clave")));
            }
        }
    }

    #[test]
    fn channels_land_where_openclaw_expects_them() {
        let mut o = Options::new();
        o.change_channels = true;
        o.telegram = "123:abc".into();
        let (p, _) = settings_patch(&o, true).unwrap();
        assert_eq!(p["channels"]["telegram"]["botToken"], "123:abc");
        assert_eq!(p["channels"]["telegram"]["enabled"], true);
    }

    #[test]
    fn changing_the_model_on_an_existing_install_goes_through_openclaw() {
        let mut o = Options::new();
        o.change_gateway = false;
        o.change_bot = false;
        o.service = false;
        o.change_ai = true;
        o.provider = 1;
        o.api_key = "k".into();
        o.model = "gpt-x".into();
        let plan = build_plan(&o, &existing_found(), None);
        assert!(plan.iter().any(|s| s.action == Action::SetKey { provider: "openai".into(), key: "k".into() }));
        assert!(plan.iter().any(|s| s.action == Action::SetModel { provider: "openai".into(), model: "gpt-x".into() }));
        assert!(!plan.iter().any(|s| matches!(&s.action, Action::Cli { args, .. } if args[0] == "onboard")));
    }

    #[test]
    fn advanced_edits_become_their_own_step() {
        let o = Options::new();
        let plan = build_plan(&o, &existing_found(), Some((json!({"logging": {"level": "debug"}}), vec![])));
        assert!(plan.iter().any(|s| s.title.contains("avanzada")));
    }

    #[test]
    fn powershell_quotes_are_doubled() {
        assert_eq!(ps_quote("O'Brien"), "'O''Brien'");
    }
}
