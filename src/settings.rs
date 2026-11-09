//! Connection profiles, app preferences and detection of the local OpenClaw.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 18789;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub enum ProfileKind {
    /// OpenClaw on this computer.
    #[default]
    Local,
    /// A remote address reachable directly (LAN, Tailscale, wss://).
    Direct,
    /// A remote computer reached through an SSH tunnel.
    Ssh,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub enum SecretKind {
    #[default]
    Token,
    Password,
    None,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: ProfileKind,
    /// ws:// or wss:// address for Direct profiles. Empty for Local and Ssh.
    pub url: String,
    /// user@host or user@host:port for Ssh profiles.
    pub ssh_target: String,
    pub ssh_identity: String,
    pub remote_port: u16,
    pub secret_kind: SecretKind,
    pub tls_fingerprint: String,
    /// Ask the gateway for operator.admin (create Bots, change settings).
    pub admin: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Remoto".into(),
            kind: ProfileKind::Direct,
            url: String::new(),
            ssh_target: String::new(),
            ssh_identity: String::new(),
            remote_port: DEFAULT_PORT,
            secret_kind: SecretKind::Token,
            tls_fingerprint: String::new(),
            admin: false,
        }
    }
}

impl Profile {
    pub fn local() -> Self {
        Self {
            id: "local".into(),
            name: "Este equipo".into(),
            kind: ProfileKind::Local,
            admin: true,
            ..Default::default()
        }
    }

    pub fn secret_key(&self) -> String {
        format!("profile-secret:{}", self.id)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BotLook {
    pub shape: usize,
    pub color: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AppSettings {
    pub profiles: Vec<Profile>,
    pub active: String,
    pub theme: Theme,
    pub pinned: Vec<String>,
    pub hidden: Vec<String>,
    pub looks: HashMap<String, BotLook>,
    pub info_open: bool,
    pub notifications: bool,
    pub setup_done: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            profiles: vec![Profile::local()],
            active: "local".into(),
            theme: Theme::Dark,
            pinned: vec![],
            hidden: vec![],
            looks: HashMap::new(),
            info_open: true,
            notifications: true,
            setup_done: false,
        }
    }
}

fn data_dir() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenClaw Bots");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

impl AppSettings {
    pub fn path() -> PathBuf {
        data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let mut s: AppSettings = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if !s.profiles.iter().any(|p| p.kind == ProfileKind::Local) {
            s.profiles.insert(0, Profile::local());
        }
        if !s.profiles.iter().any(|p| p.id == s.active) {
            s.active = s.profiles[0].id.clone();
        }
        s
    }

    pub fn save(&self) {
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), t);
        }
    }

    pub fn active_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|p| p.id == self.active)
            .unwrap_or(&self.profiles[0])
    }
}

/// What the local OpenClaw config says about how to reach it.
#[derive(Clone, Debug, Default)]
pub struct LocalGateway {
    pub port: u16,
    pub tls: bool,
    pub token: Option<String>,
    pub password: Option<String>,
    pub config_found: bool,
    pub note: Option<String>,
}

impl LocalGateway {
    pub fn url(&self) -> String {
        format!(
            "{}://127.0.0.1:{}",
            if self.tls { "wss" } else { "ws" },
            self.port
        )
    }
}

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var("OPENCLAW_CONFIG_PATH") {
        return PathBuf::from(p);
    }
    let state = std::env::var("OPENCLAW_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".openclaw"));
    state.join("openclaw.json")
}

/// A value in the config can be plain text or a reference to an env variable.
fn resolve_secret(v: Option<&Value>, note: &mut Option<String>) -> Option<String> {
    match v? {
        Value::String(s) => {
            // "${VAR}" substitution.
            let t = s.trim();
            if let Some(name) = t.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
                std::env::var(name).ok()
            } else if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Object(o) => {
            if o.get("source").and_then(Value::as_str) == Some("env") {
                o.get("id")
                    .and_then(Value::as_str)
                    .and_then(|n| std::env::var(n).ok())
            } else {
                *note = Some(
                    "La clave de OpenClaw está en un gestor de secretos; escríbela en Ajustes."
                        .to_string(),
                );
                None
            }
        }
        _ => None,
    }
}

/// Reads the OpenClaw config of this computer, without ever showing the secret.
pub fn detect_local() -> LocalGateway {
    let mut gw = LocalGateway {
        port: DEFAULT_PORT,
        ..Default::default()
    };
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(cfg) = json5::from_str::<Value>(&text) {
            gw.config_found = true;
            let g = cfg.get("gateway");
            if let Some(p) = g.and_then(|g| g.get("port")).and_then(Value::as_u64) {
                gw.port = p as u16;
            }
            gw.tls = g
                .and_then(|g| g.get("tls"))
                .and_then(|t| t.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let auth = g.and_then(|g| g.get("auth"));
            let mut note = None;
            gw.token = resolve_secret(auth.and_then(|a| a.get("token")), &mut note);
            gw.password = resolve_secret(auth.and_then(|a| a.get("password")), &mut note);
            gw.note = note;
        }
    }
    if let Ok(p) = std::env::var("OPENCLAW_GATEWAY_PORT") {
        if let Ok(p) = p.trim().parse() {
            gw.port = p;
        }
    }
    if let Ok(t) = std::env::var("OPENCLAW_GATEWAY_TOKEN") {
        if !t.trim().is_empty() {
            gw.token = Some(t);
        }
    }
    if let Ok(t) = std::env::var("OPENCLAW_GATEWAY_PASSWORD") {
        if !t.trim().is_empty() {
            gw.password = Some(t);
        }
    }
    gw
}
