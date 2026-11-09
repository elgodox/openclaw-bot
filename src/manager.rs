//! The administration side of the app: what each section loads from OpenClaw
//! and what its buttons do.

use crate::app::{App, Req};
use crate::cli::{Host, Jobs};
use crate::config_edit::{build_patch, ConfigEditor, View};
use crate::gateway::{ConnState, RpcError};
use crate::settings::ProfileKind;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use eframe::egui;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Chats,
    Overview,
    Bots,
    Models,
    Channels,
    Plugins,
    Skills,
    Routines,
    Devices,
    Fleet,
    Approvals,
    Secrets,
    Usage,
    Logs,
    Maintenance,
    Config,
    Console,
}

impl Section {
    pub const ALL: [Section; 17] = [
        Section::Chats,
        Section::Overview,
        Section::Bots,
        Section::Models,
        Section::Channels,
        Section::Plugins,
        Section::Skills,
        Section::Routines,
        Section::Devices,
        Section::Fleet,
        Section::Approvals,
        Section::Secrets,
        Section::Usage,
        Section::Logs,
        Section::Maintenance,
        Section::Config,
        Section::Console,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Chats => "Chats",
            Section::Overview => "Resumen",
            Section::Bots => "Bots",
            Section::Models => "Modelos",
            Section::Channels => "Canales",
            Section::Plugins => "Complementos",
            Section::Skills => "Habilidades",
            Section::Routines => "Rutinas",
            Section::Devices => "Equipos",
            Section::Fleet => "Flota",
            Section::Approvals => "Permisos",
            Section::Secrets => "Claves",
            Section::Usage => "Uso",
            Section::Logs => "Registros",
            Section::Maintenance => "Servicio",
            Section::Config => "Configurar",
            Section::Console => "Consola",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Section::Chats => "💬",
            Section::Overview => "⌂",
            Section::Bots => "☺",
            Section::Models => "✦",
            Section::Channels => "☎",
            Section::Plugins => "⚙",
            Section::Skills => "★",
            Section::Routines => "⏰",
            Section::Devices => "🖥",
            Section::Fleet => "🛰",
            Section::Approvals => "✔",
            Section::Secrets => "🔑",
            Section::Usage => "📊",
            Section::Logs => "📄",
            Section::Maintenance => "🔧",
            Section::Config => "☰",
            Section::Console => "⌨",
        }
    }

    pub fn from_name(name: &str) -> Option<Section> {
        let n = name.to_lowercase();
        Section::ALL.iter().copied().find(|s| s.title().to_lowercase() == n || format!("{s:?}").to_lowercase() == n)
    }
}

pub enum MgrReq {
    Data(String),
    Act { ok: Option<String>, retry: Option<(String, Value)> },
    ConfigSchema,
    ConfigGet,
    ConfigSave,
    Wizard,
    QrStart { channel: String, account: String },
    QrWait { channel: String, account: String, session: Option<String> },
    SetupCode,
    Logs,
    Rpc,
    PluginReview { method: String, params: Value },
}

pub struct WizardRun {
    pub title: String,
    pub session: Option<String>,
    pub step: Option<Value>,
    pub done: bool,
    pub error: Option<String>,
    pub busy: bool,
    pub text: String,
    pub picks: HashSet<usize>,
    pub summary: String,
}

pub struct QrLogin {
    pub channel: String,
    pub png: Option<(u64, Vec<u8>)>,
    pub message: String,
    pub done: bool,
    pub failed: bool,
    pub cancelled: bool,
}

pub struct SetupCode {
    pub code: String,
    pub gateway: String,
    pub png: Option<Vec<u8>>,
    pub expires_ms: Option<i64>,
}

/// A question to confirm before doing something that can't be undone.
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub button: String,
    pub run: Box<dyn FnOnce(&mut App, &egui::Context)>,
}

#[derive(Default)]
pub struct RoutineForm {
    pub open: bool,
    pub name: String,
    pub agent: String,
    pub kind: usize,
    pub cron: String,
    pub tz: String,
    pub every_minutes: String,
    pub at: String,
    pub message: String,
    pub delete_after: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Forms {
    pub provider: String,
    pub api_key: String,
    pub model_search: String,
    pub plugin_search: String,
    pub plugin_tab: usize,
    pub plugin_spec: String,
    pub plugin_source: usize,
    pub skill_search: String,
    pub skill_tab: usize,
    pub secret_name: String,
    pub secret_value: String,
    pub secret_is_env: bool,
    pub routine: RoutineForm,
    pub rename_device: Option<(String, String)>,
    pub rename_node: Option<(String, String)>,
    pub log_filter: String,
    pub log_follow: bool,
    pub console_line: String,
    pub console_mode: usize,
    pub rpc_method: String,
    pub rpc_params: String,
    pub rpc_output: String,
    pub rpc_busy: bool,
    pub job_open: Option<u64>,
    pub approvals_security: String,
    pub approvals_ask: String,
    pub approvals_fallback: String,
    pub approvals_auto: bool,
    pub approvals_loaded: bool,
    pub backup_output: String,
    pub backup_verify: String,
    pub config_tab: usize,
    pub usage_days: usize,
}

pub struct Manager {
    pub section: Section,
    pub data: HashMap<String, Rc<Value>>,
    pub errors: HashMap<String, String>,
    pub loading: HashSet<String>,
    pub config: ConfigEditor,
    pub wizard: Option<WizardRun>,
    pub qr: Option<QrLogin>,
    pub setup_code: Option<SetupCode>,
    pub confirm: Option<Confirm>,
    pub jobs: Jobs,
    pub forms: Forms,
    pub fleet: crate::fleet::FleetState,
    pub log_lines: Vec<String>,
    pub log_cursor: Option<u64>,
    pub log_busy: bool,
    pub log_file: String,
    last_log: Instant,
    last_section_load: Instant,
    png_counter: u64,
}

impl Default for Manager {
    fn default() -> Self {
        let forms = Forms {
            log_follow: true,
            usage_days: 30,
            rpc_params: "{}".into(),
            routine: RoutineForm { tz: String::new(), every_minutes: "60".into(), cron: "0 9 * * *".into(), ..Default::default() },
            ..Default::default()
        };
        Self {
            section: Section::Chats,
            data: HashMap::new(),
            errors: HashMap::new(),
            loading: HashSet::new(),
            config: ConfigEditor::default(),
            wizard: None,
            qr: None,
            setup_code: None,
            confirm: None,
            jobs: Jobs::default(),
            forms,
            fleet: crate::fleet::FleetState::default(),
            log_lines: vec![],
            log_cursor: None,
            log_busy: false,
            log_file: String::new(),
            last_log: Instant::now() - Duration::from_secs(10),
            last_section_load: Instant::now() - Duration::from_secs(60),
            png_counter: 0,
        }
    }
}

fn data_url_png(url: &str) -> Option<Vec<u8>> {
    STANDARD.decode(url.strip_prefix("data:image/png;base64,")?).ok()
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(o) => o.get(key).or_else(|| o.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

impl App {
    fn req(&mut self, method: &str, params: Value, r: MgrReq) {
        self.call(method, params, Req::Mgr(r));
    }

    pub fn fetch(&mut self, key: &str, method: &str, params: Value) {
        self.mgr.loading.insert(key.to_string());
        self.req(method, params, MgrReq::Data(key.to_string()));
    }

    /// Calls a method for its effect, then reloads the current section.
    pub fn act(&mut self, method: &str, params: Value, ok: &str) {
        let ok = (!ok.is_empty()).then(|| ok.to_string());
        self.req(method, params, MgrReq::Act { ok, retry: None });
    }

    fn act_retryable(&mut self, method: &str, params: Value, ok: &str) {
        let retry = Some((method.to_string(), params.clone()));
        self.req(method, params, MgrReq::Act { ok: Some(ok.to_string()), retry });
    }

    pub fn open_section(&mut self, s: Section, ctx: &egui::Context) {
        self.mgr.section = s;
        self.load_section(ctx);
    }

    /// Loads what the visible section shows.
    pub fn load_section(&mut self, _ctx: &egui::Context) {
        self.mgr.last_section_load = Instant::now();
        if !self.is_connected() {
            return;
        }
        match self.mgr.section {
            Section::Chats | Section::Console | Section::Fleet => {}
            Section::Overview => {
                self.fetch("health", "health", json!({}));
                self.fetch("status", "status", json!({}));
                self.fetch("system", "system.info", json!({}));
                self.fetch("update", "update.status", json!({}));
                self.fetch("channels", "channels.status", json!({}));
                self.fetch("nodes", "node.list", json!({}));
                self.fetch("pairing", "device.pair.list", json!({}));
                self.fetch("usage_cost", "usage.cost", json!({"days": 7}));
            }
            Section::Bots => {
                self.fetch("status", "status", json!({}));
                self.load_sessions();
            }
            Section::Models => {
                self.load_config_values();
                self.fetch("auth", "models.authStatus", json!({"refresh": false}));
                self.fetch("catalog", "models.list", json!({"view": "all"}));
            }
            Section::Channels => {
                self.fetch("channels", "channels.status", json!({"probe": false}));
                self.fetch("pairing_requests", "channels.pairing.list", json!({}));
            }
            Section::Plugins => {
                self.fetch("plugins", "plugins.list", json!({}));
                self.fetch("hooks", "hooks.status", json!({}));
            }
            Section::Skills => self.fetch("skills", "skills.status", json!({})),
            Section::Routines => {
                self.fetch("cron", "cron.list", json!({"includeDisabled": true}));
                self.fetch("cron_status", "cron.status", json!({}));
            }
            Section::Devices => {
                self.fetch("pairing", "device.pair.list", json!({}));
                self.fetch("nodes", "node.list", json!({}));
                self.fetch("node_pairing", "node.pair.list", json!({}));
            }
            Section::Approvals => {
                self.fetch("exec_policy", "exec.approvals.get", json!({}));
                self.fetch("approval_history", "approval.history", json!({"limit": 50}));
            }
            Section::Secrets => self.fetch("secrets", "secrets.store.list", json!({})),
            Section::Usage => {
                let days = self.mgr.forms.usage_days;
                self.fetch("usage_status", "usage.status", json!({}));
                self.fetch("usage_cost", "usage.cost", json!({"days": days}));
            }
            Section::Logs => {
                self.mgr.log_cursor = None;
                self.mgr.log_lines.clear();
                self.poll_logs();
            }
            Section::Maintenance => {
                self.fetch("update", "update.status", json!({}));
                self.fetch("update_runs", "update.runs.list", json!({}));
                self.fetch("backup", "backup.status", json!({}));
                self.fetch("system", "system.info", json!({}));
            }
            Section::Config => {
                self.load_config_values();
                if !self.mgr.config.has_schema() {
                    self.mgr.config.loading = true;
                    self.req("config.schema", json!({}), MgrReq::ConfigSchema);
                }
            }
        }
    }

    pub fn load_config_values(&mut self) {
        self.req("config.get", json!({}), MgrReq::ConfigGet);
    }

    pub fn mgr_bootstrap(&mut self, ctx: &egui::Context) {
        self.mgr.config.schema = None;
        self.load_section(ctx);
        // Requests already waiting are worth knowing about from any section.
        self.fetch("pairing", "device.pair.list", json!({}));
        self.fetch("pairing_requests", "channels.pairing.list", json!({}));
    }

    pub fn mgr_pump(&mut self, ctx: &egui::Context) {
        self.mgr.jobs.pump();
        self.mgr.fleet.pump(); // la flota funciona aunque no haya gateway conectado
        if !self.is_connected() {
            return;
        }
        match self.mgr.section {
            Section::Logs if self.mgr.forms.log_follow => {
                if self.mgr.last_log.elapsed() > Duration::from_secs(2) && !self.mgr.log_busy {
                    self.poll_logs();
                }
                ctx.request_repaint_after(Duration::from_millis(500));
            }
            Section::Overview => {
                if self.mgr.last_section_load.elapsed() > Duration::from_secs(10) {
                    self.load_section(ctx);
                }
                ctx.request_repaint_after(Duration::from_secs(5));
            }
            _ => {}
        }
    }

    pub fn poll_logs(&mut self) {
        self.mgr.last_log = Instant::now();
        self.mgr.log_busy = true;
        let mut params = json!({"limit": 500, "maxBytes": 250000});
        if let Some(c) = self.mgr.log_cursor {
            params["cursor"] = json!(c);
        }
        self.req("logs.tail", params, MgrReq::Logs);
    }

    pub fn mgr_event(&mut self, event: &str, p: &Value, ctx: &egui::Context) {
        match event {
            "device.pair.requested" | "device.pair.resolved" | "node.pair.requested" | "node.pair.resolved" => {
                self.fetch("pairing", "device.pair.list", json!({}));
                if self.mgr.section == Section::Devices {
                    self.fetch("node_pairing", "node.pair.list", json!({}));
                }
                if event.ends_with("requested") && self.settings.notifications {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                }
            }
            "plugins.changed" if self.mgr.section == Section::Plugins => self.load_section(ctx),
            "cron" if self.mgr.section == Section::Routines => self.load_section(ctx),
            _ => {}
        }
        let _ = p;
    }

    pub fn mgr_response(&mut self, req: MgrReq, result: Result<Value, RpcError>, ctx: &egui::Context) {
        match req {
            MgrReq::Data(key) => {
                self.mgr.loading.remove(&key);
                match result {
                    Ok(v) => {
                        self.mgr.errors.remove(&key);
                        if key == "exec_policy" {
                            self.mgr.forms.approvals_loaded = false;
                        }
                        self.mgr.data.insert(key, Rc::new(v));
                    }
                    Err(e) => {
                        let text = friendly_error(&e);
                        self.mgr.errors.insert(key, text);
                    }
                }
            }
            MgrReq::Act { ok, retry } => match result {
                Ok(_) => {
                    if let Some(t) = ok {
                        self.toast(t, true);
                    }
                    self.load_section(ctx);
                }
                Err(e) => self.on_action_error(e, retry),
            },
            MgrReq::ConfigSchema => {
                self.mgr.config.loading = false;
                match result {
                    Ok(v) => self.mgr.config.set_schema(&v),
                    Err(e) => self.mgr.config.message = Some((friendly_error(&e), false)),
                }
            }
            MgrReq::ConfigGet => match result {
                Ok(v) => {
                    self.mgr.config.set_current(&v);
                    self.mgr.data.insert("config".into(), Rc::new(v));
                }
                Err(e) => self.mgr.config.message = Some((friendly_error(&e), false)),
            },
            MgrReq::ConfigSave => {
                self.mgr.config.saving = false;
                match result {
                    Ok(v) => {
                        self.mgr.config.discard();
                        self.mgr.config.file_dirty = false;
                        let changed = v["changedPaths"].as_array().map(|a| a.len()).unwrap_or(0);
                        let restart = v["restart"].is_object() || v["restartPending"].as_bool() == Some(true);
                        let msg = match (changed, restart) {
                            (_, true) => "Guardado. OpenClaw se reinicia para aplicarlo.".to_string(),
                            (0, _) => "Guardado. No hizo falta cambiar nada.".to_string(),
                            (n, _) => format!("Guardado y aplicado ({n} cambios)."),
                        };
                        self.mgr.config.message = Some((msg, true));
                        self.toast("Configuración guardada", true);
                        self.load_config_values();
                    }
                    Err(e) => {
                        let stale = e.message.to_lowercase().contains("hash");
                        let text = if stale {
                            "La configuración cambió mientras editabas. Recargué los valores; revisa y guarda otra vez.".to_string()
                        } else {
                            friendly_error(&e)
                        };
                        self.mgr.config.message = Some((text, false));
                        if stale {
                            self.load_config_values();
                        }
                    }
                }
            }
            MgrReq::Wizard => self.on_wizard(result, ctx),
            MgrReq::QrStart { channel, account } => match result {
                Ok(v) => {
                    let session = v["sessionKey"].as_str().map(str::to_string);
                    self.apply_qr(&v);
                    if v["connected"].as_bool() == Some(true) {
                        if let Some(q) = &mut self.mgr.qr {
                            q.done = true;
                        }
                    } else {
                        self.wait_qr(channel, account, session);
                    }
                }
                Err(e) => self.fail_qr(friendly_error(&e)),
            },
            MgrReq::QrWait { channel, account, session } => match result {
                Ok(v) => {
                    if self.mgr.qr.as_ref().is_none_or(|q| q.cancelled) {
                        return;
                    }
                    self.apply_qr(&v);
                    if v["connected"].as_bool() == Some(true) {
                        if let Some(q) = &mut self.mgr.qr {
                            q.done = true;
                        }
                        self.toast("Canal conectado", true);
                        self.load_section(ctx);
                    } else {
                        let session = v["sessionKey"].as_str().map(str::to_string).or(session);
                        self.wait_qr(channel, account, session);
                    }
                }
                Err(e) => self.fail_qr(friendly_error(&e)),
            },
            MgrReq::SetupCode => match result {
                Ok(v) => {
                    self.mgr.setup_code = Some(SetupCode {
                        code: v["setupCode"].as_str().unwrap_or("").to_string(),
                        gateway: v["gatewayUrl"].as_str().unwrap_or("").to_string(),
                        png: v["qrDataUrl"].as_str().and_then(data_url_png),
                        expires_ms: v["expiresAtMs"].as_i64(),
                    });
                }
                Err(e) => self.toast(friendly_error(&e), false),
            },
            MgrReq::Logs => {
                self.mgr.log_busy = false;
                if let Ok(v) = result {
                    if v["reset"].as_bool() == Some(true) {
                        self.mgr.log_lines.clear();
                    }
                    self.mgr.log_cursor = v["cursor"].as_u64();
                    self.mgr.log_file = v["file"].as_str().unwrap_or("").to_string();
                    if let Some(a) = v["lines"].as_array() {
                        self.mgr.log_lines.extend(a.iter().filter_map(Value::as_str).map(tidy_log_line));
                    }
                    let n = self.mgr.log_lines.len();
                    if n > 6000 {
                        self.mgr.log_lines.drain(..n - 6000);
                    }
                }
            }
            MgrReq::PluginReview { method, mut params } => match result {
                Ok(v) => {
                    if let Some(t) = find_key(&v, "reviewToken").and_then(Value::as_str) {
                        params["acknowledgeCapabilities"] = json!({"reviewToken": t});
                    }
                    self.act(&method, params, "Listo");
                }
                Err(e) => self.toast(friendly_error(&e), false),
            },
            MgrReq::Rpc => {
                self.mgr.forms.rpc_busy = false;
                self.mgr.forms.rpc_output = match result {
                    Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                    Err(e) => format!(
                        "Error {}: {}\n{}",
                        e.code,
                        e.message,
                        if e.details.is_null() { String::new() } else { e.details.to_string() }
                    ),
                };
            }
        }
    }

    fn on_action_error(&mut self, e: RpcError, retry: Option<(String, Value)>) {
        if let Some((method, params)) = retry {
            let warning = e.details.get("installPolicyCode").and_then(Value::as_str) == Some("install_policy_warning_acknowledgement_required");
            let token = find_key(&e.details, "reviewToken").and_then(Value::as_str).map(str::to_string);
            let plugin = params["pluginId"].as_str().map(str::to_string);
            let needs_review = token.is_some() || e.message.to_lowercase().contains("capabilit");
            if warning || needs_review {
                let reason = e.details.get("reason").and_then(Value::as_str).unwrap_or(&e.message).to_string();
                self.mgr.confirm = Some(Confirm {
                    title: if warning { "Este complemento tiene una advertencia".into() } else { "Este complemento pide permisos".into() },
                    body: format!("{reason}\n\nInstálalo solo si confías en quien lo publica."),
                    button: "Aceptar y continuar".into(),
                    run: Box::new(move |app, _| {
                        let mut next = params.clone();
                        if warning {
                            next["acknowledgeInstallPolicyWarning"] = json!(true);
                        }
                        if let Some(t) = token {
                            next["acknowledgeCapabilities"] = json!({"reviewToken": t});
                            app.act(&method, next, "Listo");
                        } else if let (false, Some(id)) = (warning, plugin) {
                            app.review_then_retry(id, method, next);
                        } else {
                            app.act(&method, next, "Listo");
                        }
                    }),
                });
                return;
            }
        }
        let text = friendly_error(&e);
        self.toast(text, false);
    }

    /// Some plugins need a review token, which only an inspection issues. The
    /// person has already agreed by the time this runs.
    fn review_then_retry(&mut self, plugin: String, method: String, params: Value) {
        self.req("plugins.inspect", json!({"pluginId": plugin}), MgrReq::PluginReview { method, params });
    }

    fn apply_qr(&mut self, v: &Value) {
        self.mgr.png_counter += 1;
        let n = self.mgr.png_counter;
        if let Some(q) = &mut self.mgr.qr {
            if let Some(png) = v["qrDataUrl"].as_str().and_then(data_url_png) {
                q.png = Some((n, png));
            }
            if let Some(m) = v["message"].as_str() {
                q.message = m.to_string();
            }
        }
    }

    fn fail_qr(&mut self, text: String) {
        if let Some(q) = &mut self.mgr.qr {
            q.failed = true;
            q.message = text;
        }
    }

    fn wait_qr(&mut self, channel: String, account: String, session: Option<String>) {
        let mut params = json!({"channel": channel, "timeoutMs": 120000});
        if !account.is_empty() {
            params["accountId"] = json!(account);
        }
        if let Some(s) = &session {
            params["sessionKey"] = json!(s);
        }
        self.req("web.login.wait", params, MgrReq::QrWait { channel, account, session });
    }

    pub fn start_qr(&mut self, channel: &str, account: &str) {
        self.mgr.qr = Some(QrLogin {
            channel: channel.to_string(),
            png: None,
            message: "Pidiendo el código…".into(),
            done: false,
            failed: false,
            cancelled: false,
        });
        let mut params = json!({"channel": channel, "force": false, "timeoutMs": 30000});
        if !account.is_empty() {
            params["accountId"] = json!(account);
        }
        self.req("web.login.start", params, MgrReq::QrStart { channel: channel.to_string(), account: account.to_string() });
    }

    pub fn request_setup_code(&mut self) {
        self.req("device.pair.setupCode", json!({}), MgrReq::SetupCode);
    }

    // ---- config ----------------------------------------------------------

    pub fn save_config_form(&mut self) {
        let (patch, replace) = build_patch(&self.mgr.config.edits, &self.mgr.config.current);
        self.send_patch(patch, replace, "OpenClaw Bot");
    }

    fn send_patch(&mut self, patch: Value, replace: Vec<String>, note: &str) {
        let mut params = json!({"raw": patch.to_string(), "note": note});
        if !replace.is_empty() {
            params["replacePaths"] = json!(replace);
        }
        if let Some(h) = &self.mgr.config.base_hash {
            params["baseHash"] = json!(h);
        }
        self.mgr.config.saving = true;
        self.mgr.config.message = None;
        self.req("config.patch", params, MgrReq::ConfigSave);
    }

    /// Writes a small change without going through the form.
    pub fn patch_config(&mut self, patch: Value) {
        self.send_patch(patch, vec![], "OpenClaw Bot");
    }

    pub fn save_config_file(&mut self) {
        let mut params = json!({"raw": self.mgr.config.file_text.clone(), "note": "OpenClaw Bot"});
        if let Some(h) = &self.mgr.config.base_hash {
            params["baseHash"] = json!(h);
        }
        self.mgr.config.saving = true;
        self.mgr.config.message = None;
        self.req("config.apply", params, MgrReq::ConfigSave);
    }

    pub fn default_model(&self) -> String {
        let m = crate::config_edit::value_at(&self.mgr.config.current, &["agents".into(), "defaults".into(), "model".into()]);
        match m {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v["primary"].as_str().unwrap_or("").to_string(),
            None => String::new(),
        }
    }

    pub fn set_default_model(&mut self, model: &str) {
        self.patch_config(json!({"agents": {"defaults": {"model": {"primary": model}}}}));
    }

    // ---- guided setup over the Gateway ------------------------------------

    pub fn start_wizard(&mut self, title: &str, params: Value) {
        self.mgr.wizard = Some(WizardRun {
            title: title.to_string(),
            session: None,
            step: None,
            done: false,
            error: None,
            busy: true,
            text: String::new(),
            picks: HashSet::new(),
            summary: String::new(),
        });
        self.req("wizard.start", params, MgrReq::Wizard);
    }

    fn on_wizard(&mut self, result: Result<Value, RpcError>, ctx: &egui::Context) {
        let mut advance: Option<String> = None;
        let mut finished = false;
        let Some(w) = &mut self.mgr.wizard else { return };
        w.busy = false;
        match result {
            Ok(v) => {
                if let Some(s) = v["sessionId"].as_str() {
                    w.session = Some(s.to_string());
                }
                w.done = v["done"].as_bool().unwrap_or(false);
                if let Some(e) = v["error"].as_str() {
                    w.error = Some(e.to_string());
                }
                w.step = v.get("step").filter(|s| !s.is_null()).cloned();
                w.text.clear();
                w.picks.clear();
                if let Some(step) = &w.step {
                    if let Some(init) = step.get("initialValue") {
                        if let Some(t) = init.as_str() {
                            w.text = t.to_string();
                        }
                        if let (Some(opts), Some(list)) = (step["options"].as_array(), init.as_array()) {
                            for (i, o) in opts.iter().enumerate() {
                                if list.contains(&o["value"]) {
                                    w.picks.insert(i);
                                }
                            }
                        }
                    }
                    if step["type"] == "progress" {
                        advance = w.session.clone();
                    }
                }
                if w.done {
                    finished = true;
                    let channels = v["channels"].as_array().map(|a| a.len()).unwrap_or(0);
                    if channels > 0 {
                        w.summary = "El canal quedó configurado.".into();
                    }
                }
            }
            Err(e) => {
                w.error = Some(friendly_error(&e));
                w.done = true;
            }
        }
        if let Some(session) = advance {
            std::thread::sleep(Duration::from_millis(120));
            self.wizard_next(session, None);
        }
        if finished {
            self.load_section(ctx);
        }
    }

    pub fn wizard_next(&mut self, session: String, answer: Option<Value>) {
        let mut params = json!({"sessionId": session});
        if let Some(a) = answer {
            params["answer"] = a;
        }
        if let Some(w) = &mut self.mgr.wizard {
            w.busy = true;
        }
        self.req("wizard.next", params, MgrReq::Wizard);
    }

    pub fn wizard_answer(&mut self, value: Value) {
        let Some(w) = &self.mgr.wizard else { return };
        let (Some(session), Some(step)) = (w.session.clone(), w.step.clone()) else { return };
        let answer = json!({"stepId": step["id"], "value": value});
        self.wizard_next(session, Some(answer));
    }

    pub fn wizard_cancel(&mut self) {
        if let Some(w) = self.mgr.wizard.take() {
            if let (Some(s), false) = (w.session, w.done) {
                self.call("wizard.cancel", json!({"sessionId": s}), Req::Abort);
            }
        }
    }

    // ---- commands ---------------------------------------------------------

    pub fn cli_host(&self) -> Option<Host> {
        let p = self.active_profile();
        match p.kind {
            ProfileKind::Local => Some(Host::Local),
            ProfileKind::Ssh => Some(Host::Ssh { target: p.ssh_target, identity: p.ssh_identity }),
            ProfileKind::Direct => None,
        }
    }

    pub fn run_cli(&mut self, title: &str, args: &[&str], ctx: &egui::Context) {
        let Some(host) = self.cli_host() else {
            self.toast("Esta conexión es solo por red: para esto usa un túnel SSH.", false);
            return;
        };
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let c = ctx.clone();
        let id = self.mgr.jobs.run(title, &host, args, None, move || c.request_repaint());
        self.mgr.forms.job_open = Some(id);
    }

    /// Corre `fleet.py <args>` como trabajo en vivo y abre su panel de salida.
    pub fn run_fleet(&mut self, title: &str, args: &[&str], stdin: Option<String>, ctx: &egui::Context) {
        let cmd = match self.mgr.fleet.command(args) {
            Ok(c) => c,
            Err(e) => {
                self.toast(e, false);
                return;
            }
        };
        let shown = format!("fleet.py {}", args.join(" "));
        let c = ctx.clone();
        let id = self.mgr.jobs.start(title, shown, cmd, stdin, move || c.request_repaint());
        self.mgr.forms.job_open = Some(id);
    }

    /// Guarda (cifrada) la contraseña admin de un host sin exponerla en la línea de comando.
    pub fn run_fleet_set_admin(&mut self, host: &str, admin_pw: &str, ctx: &egui::Context) {
        let mut cmd = match self.mgr.fleet.command(&["vault", "set-admin", host]) {
            Ok(c) => c,
            Err(e) => {
                self.toast(e, false);
                return;
            }
        };
        cmd.env("FLEET_ADMIN_PASSWORD", admin_pw);
        let shown = format!("fleet.py vault set-admin {host}");
        let c = ctx.clone();
        let id = self.mgr.jobs.start("Guardar contraseña admin", shown, cmd, None, move || c.request_repaint());
        self.mgr.forms.job_open = Some(id);
    }

    /// Persiste la configuración del toolkit de flota entre sesiones.
    pub fn save_fleet_settings(&mut self) {
        self.settings.fleet_dir = self.mgr.fleet.dir.trim().to_string();
        self.settings.fleet_python = self.mgr.fleet.python.trim().to_string();
        self.settings.save();
    }

    pub fn run_console_line(&mut self, ctx: &egui::Context) {
        let line = self.mgr.forms.console_line.trim().to_string();
        let mut args = crate::cli::split_args(&line);
        if args.first().map(String::as_str) == Some("openclaw") {
            args.remove(0);
        }
        if args.is_empty() {
            return;
        }
        let Some(host) = self.cli_host() else {
            self.toast("Esta conexión es solo por red: la consola necesita un túnel SSH.", false);
            return;
        };
        let c = ctx.clone();
        let id = self.mgr.jobs.run("Consola", &host, args, None, move || c.request_repaint());
        self.mgr.forms.job_open = Some(id);
    }

    pub fn send_rpc(&mut self) {
        let method = self.mgr.forms.rpc_method.trim().to_string();
        if method.is_empty() {
            return;
        }
        match json5::from_str::<Value>(self.mgr.forms.rpc_params.trim()) {
            Ok(params) => {
                self.mgr.forms.rpc_busy = true;
                self.mgr.forms.rpc_output.clear();
                self.req(&method, params, MgrReq::Rpc);
            }
            Err(_) => self.mgr.forms.rpc_output = "Los parámetros no son JSON válido.".into(),
        }
    }

    pub fn restart_gateway(&mut self) {
        self.act("gateway.restart.request", json!({"note": "OpenClaw Bot"}), "OpenClaw se está reiniciando");
    }

    // ---- models ----------------------------------------------------------

    pub fn save_api_key(&mut self) {
        let provider = self.mgr.forms.provider.trim().to_string();
        let key = self.mgr.forms.api_key.trim().to_string();
        if provider.is_empty() || key.is_empty() {
            self.toast("Elige el proveedor y pega la clave", false);
            return;
        }
        self.mgr.forms.api_key.clear();
        self.act("models.authSetApiKey", json!({"provider": provider, "apiKey": key}), "Clave guardada");
    }

    pub fn probe_provider(&mut self, provider: &str) {
        self.act("models.probe", json!({"provider": provider, "timeoutMs": 20000}), "Prueba terminada");
    }

    pub fn logout_provider(&mut self, provider: &str) {
        self.act("models.authLogout", json!({"provider": provider}), "Cuenta desconectada");
    }

    // ---- plugins ----------------------------------------------------------

    pub fn plugin_enable(&mut self, id: &str, enabled: bool) {
        self.act_retryable("plugins.setEnabled", json!({"pluginId": id, "enabled": enabled}), if enabled { "Complemento activado" } else { "Complemento desactivado" });
    }

    pub fn plugin_install(&mut self, params: Value) {
        self.act_retryable("plugins.install", params, "Complemento instalado");
    }

    pub fn browse_catalog(&mut self, kind: &str, query: &str) {
        let key = format!("catalog_{kind}");
        let mut params = json!({"kind": kind, "pageSize": 30});
        if !query.trim().is_empty() {
            params["query"] = json!(query.trim());
        }
        if kind == "skill" {
            let agent = if self.default_agent.is_empty() { self.agents.first().map(|a| a.id.clone()).unwrap_or_default() } else { self.default_agent.clone() };
            params["agentId"] = json!(agent);
        }
        self.fetch(&key, "catalog.browse", params);
    }

    // ---- routines ----------------------------------------------------------

    pub fn create_routine(&mut self) {
        let f = &self.mgr.forms.routine;
        let name = f.name.trim().to_string();
        let message = f.message.trim().to_string();
        if name.is_empty() || message.is_empty() {
            self.mgr.forms.routine.error = Some("Falta el nombre o lo que debe hacer".into());
            return;
        }
        let schedule = match f.kind {
            0 => {
                let mut s = json!({"kind": "cron", "expr": f.cron.trim()});
                if !f.tz.trim().is_empty() {
                    s["tz"] = json!(f.tz.trim());
                }
                s
            }
            1 => match f.every_minutes.trim().parse::<u64>() {
                Ok(m) if m > 0 => json!({"kind": "every", "everyMs": m * 60_000}),
                _ => {
                    self.mgr.forms.routine.error = Some("Escribe cada cuántos minutos".into());
                    return;
                }
            },
            _ => {
                if f.at.trim().is_empty() {
                    self.mgr.forms.routine.error = Some("Escribe cuándo debe ocurrir".into());
                    return;
                }
                json!({"kind": "at", "at": f.at.trim()})
            }
        };
        let mut params = json!({
            "name": name,
            "schedule": schedule,
            "sessionTarget": "isolated",
            "wakeMode": "now",
            "payload": {"kind": "agentTurn", "message": message},
            "enabled": true,
        });
        if !f.agent.is_empty() {
            params["agentId"] = json!(f.agent);
        }
        if f.delete_after {
            params["deleteAfterRun"] = json!(true);
        }
        self.mgr.forms.routine = RoutineForm { every_minutes: "60".into(), cron: "0 9 * * *".into(), ..Default::default() };
        self.act("cron.add", params, "Rutina creada");
    }

    // ---- permissions -------------------------------------------------------

    pub fn save_exec_policy(&mut self) {
        let Some(snapshot) = self.mgr.data.get("exec_policy").cloned() else { return };
        let mut file = snapshot["file"].clone();
        let f = &self.mgr.forms;
        if !file["defaults"].is_object() {
            file["defaults"] = json!({});
        }
        file["defaults"]["security"] = json!(f.approvals_security);
        file["defaults"]["ask"] = json!(f.approvals_ask);
        file["defaults"]["askFallback"] = json!(f.approvals_fallback);
        file["defaults"]["autoAllowSkills"] = json!(f.approvals_auto);
        let mut params = json!({"file": file});
        if let Some(h) = snapshot["hash"].as_str() {
            params["baseHash"] = json!(h);
        }
        self.act("exec.approvals.set", params, "Permisos guardados");
    }

    /// Sample content for automatic pictures, so nothing real ends up in one.
    pub fn fill_demo_manager(&mut self, view: Option<&str>) {
        let Some(section) = view.and_then(Section::from_name) else { return };
        self.mgr.section = section;
        let put = |app: &mut App, key: &str, v: Value| {
            app.mgr.data.insert(key.to_string(), Rc::new(v));
        };
        put(self, "health", json!({"ok": true}));
        put(self, "status", json!({"sessions": {"count": 42}}));
        put(self, "system", json!({"machineName": "MI-EQUIPO", "osLabel": "Windows 11", "uptimeMs": 7_200_000u64,
            "memoryTotalBytes": 34_359_738_368u64, "memoryFreeBytes": 12_884_901_888u64,
            "diskTotalBytes": 1_000_000_000_000u64, "diskAvailableBytes": 400_000_000_000u64}));
        put(self, "update", json!({"effectiveChannel": "stable"}));
        put(self, "channels", json!({
            "channelLabels": {"telegram": "Telegram", "whatsapp": "WhatsApp"},
            "channelAccounts": {
                "telegram": [{"accountId": "default", "name": "Mi bot", "configured": true, "running": true}],
                "whatsapp": [{"accountId": "default", "configured": true, "running": false}],
            },
            "statusIssues": [],
        }));
        put(self, "pairing_requests", json!({"requests": [
            {"requestId": "1", "channel": "telegram", "channelLabel": "Telegram", "accountId": "default", "senderLabel": "Ana"}
        ]}));
        put(self, "nodes", json!({"nodes": [
            {"nodeId": "n1", "displayName": "Portátil", "platform": "windows", "connected": true, "commands": ["a", "b", "c"]}
        ]}));
        put(self, "node_pairing", json!({"pending": []}));
        put(self, "pairing", json!({
            "pending": [{"requestId": "r1", "displayName": "Teléfono nuevo", "platform": "android", "remoteIp": "192.168.1.30"}],
            "paired": [{"deviceId": "d1", "displayName": "Panel web", "platform": "web", "connected": true, "lastSeenAtMs": 0}],
        }));
        put(self, "usage_cost", json!({
            "totals": {"totalCost": 12.5, "totalTokens": 1_250_000},
            "daily": (0..14).map(|i| json!({"totalTokens": 40_000 + i * 8_000})).collect::<Vec<_>>(),
        }));
        put(self, "usage_status", json!({"providers": []}));
        put(self, "auth", json!({
            "providers": [{"provider": "anthropic", "status": "ok", "profiles": [{"id": "anthropic:default"}]}],
            "providerCapabilities": [{"provider": "anthropic", "apiKeySupported": true}, {"provider": "openai", "apiKeySupported": true}],
        }));
        put(self, "catalog", json!({"models": [
            {"id": "claude-opus-5-5", "provider": "anthropic", "name": "Claude Opus 5.5", "recommended": true, "available": true},
            {"id": "claude-haiku-5-5", "provider": "anthropic", "name": "Claude Haiku 5.5", "available": true},
        ]}));
        put(self, "plugins", json!({"plugins": [
            {"id": "browser", "name": "Navegador", "description": "Deja que los Bots usen un navegador", "version": "1.2.0", "installed": true, "enabled": true, "origin": "bundled"},
            {"id": "extra", "name": "Calendario", "description": "Lee y crea eventos", "version": "0.4.1", "installed": true, "enabled": false, "origin": "clawhub"},
        ]}));
        put(self, "hooks", json!({"hooks": []}));
        put(self, "skills", json!({"skills": [
            {"name": "Agenda", "emoji": "", "description": "Lee y crea recordatorios", "eligible": true},
            {"name": "Correo", "emoji": "", "description": "Lee tu correo", "eligible": false, "missing": {"env": ["GMAIL_TOKEN"], "bins": []}},
        ]}));
        put(self, "exec_policy", json!({"resolvedDefaults": {"security": "allowlist", "ask": "on-miss", "askFallback": "deny", "autoAllowSkills": true}}));
        put(self, "approval_history", json!({"items": [{"decision": "allow-once", "summary": "crear-recordatorio --dia jueves", "kind": "exec"}]}));
        put(self, "secrets", json!({"entries": [{"name": "MI_CLAVE", "kind": "secret"}]}));
        self.mgr.log_lines = vec![
            "09:12:01 INFO  OpenClaw en marcha".into(),
            "09:12:04 INFO  canal telegram conectado".into(),
            "09:15:32 WARN  el modelo tardó más de lo normal".into(),
        ];
        self.mgr.config.set_schema(&json!({
            "schema": {"type": "object", "properties": {
                "gateway": {"type": "object", "properties": {
                    "port": {"type": "integer", "title": "Gateway Port", "description": "TCP port"},
                    "bind": {"anyOf": [{"type": "string", "const": "loopback"}, {"type": "string", "const": "lan"}], "title": "Gateway Bind Mode", "description": "Network bind profile."},
                    "auth": {"type": "object", "properties": {
                        "token": {"type": "string", "title": "Gateway Token", "description": "Shared secret."}
                    }, "title": "Gateway Auth"},
                }},
                "logging": {"type": "object", "properties": {"level": {"type": "string", "title": "Log Level"}}},
            }},
            "uiHints": {"gateway": {"label": "Gateway", "order": 30}, "gateway.auth.token": {"sensitive": true, "label": "Gateway Token"}},
        }));
        self.mgr.config.set_current(&json!({"sourceConfig": {"gateway": {"port": 18789, "bind": "loopback", "auth": {"token": crate::config_edit::REDACTED}}}, "hash": "demo", "path": "~/.openclaw/openclaw.json"}));
        self.mgr.forms.approvals_loaded = false;
        if section == Section::Console {
            self.mgr.forms.console_line = "status --all".into();
        }
    }

    pub fn is_local_profile(&self) -> bool {
        self.active_profile().kind == ProfileKind::Local
    }

    /// True for connections that can't reach the computer by itself.
    pub fn disconnected_reason(&self) -> Option<String> {
        match &self.conn {
            ConnState::Connected => None,
            ConnState::Disconnected(m) | ConnState::AuthFailed(m) => Some(m.clone()),
            ConnState::Pairing { .. } => Some("Falta aprobar este equipo en OpenClaw.".into()),
            _ => Some("Conectando…".into()),
        }
    }

    pub fn start_config_for_view(&mut self, view: View) {
        self.mgr.config.view = view;
    }
}

fn tidy_log_line(raw: &str) -> String {
    // Lines are JSON records; show the time, level and message.
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return raw.to_string();
    };
    let time = v["time"].as_str().or(v["_meta"]["date"].as_str()).unwrap_or("");
    let time = time.get(11..19).unwrap_or(time);
    let level = v["level"].as_str().or(v["_meta"]["logLevelName"].as_str()).unwrap_or("").to_uppercase();
    let msg = ["message", "msg", "1", "0"].iter().find_map(|k| v[*k].as_str()).unwrap_or("");
    if msg.is_empty() {
        raw.to_string()
    } else {
        format!("{time} {level:<5} {msg}")
    }
}

pub fn friendly_error(e: &RpcError) -> String {
    let m = if e.message.is_empty() { e.code.clone() } else { e.message.clone() };
    let lower = m.to_lowercase();
    if lower.contains("missing scope") || e.code == "FORBIDDEN" || lower.contains("not authorized") {
        return format!("Esta conexión no tiene permiso para esto ({m}). Activa \"Permitir administrar\" en la conexión y aprueba este equipo otra vez.");
    }
    if lower.contains("unknown method") {
        return "Tu versión de OpenClaw no tiene esta función. Actualízala en Servicio.".into();
    }
    if e.code == "LOCAL" {
        return "No hay conexión con OpenClaw.".into();
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_review_token_anywhere() {
        let v = json!({"a": [{"b": {"reviewToken": "t"}}]});
        assert_eq!(find_key(&v, "reviewToken").unwrap(), "t");
        assert!(find_key(&v, "nope").is_none());
    }

    #[test]
    fn tidies_json_log_lines() {
        let line = r#"{"time":"2026-11-08T12:30:45.000Z","level":"info","message":"hola"}"#;
        assert_eq!(tidy_log_line(line), "12:30:45 INFO  hola");
        assert_eq!(tidy_log_line("texto"), "texto");
    }

    #[test]
    fn every_section_has_a_name_and_can_be_found() {
        for s in Section::ALL {
            assert_eq!(Section::from_name(s.title()), Some(s));
        }
    }
}
