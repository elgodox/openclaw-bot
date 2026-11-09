//! The app's state and everything that talks to the gateway.

use crate::gateway::{Client, ConnState, HelloInfo, NetEvent, Target};
use crate::identity::{DeviceIdentity, SecretStore};
use crate::model::*;
use crate::probe::{device_token_key, target_for};
use crate::settings::{detect_local, AppSettings, LocalGateway, Profile, ProfileKind, SecretKind};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use eframe::egui;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SESSION_LIST_PARAMS: &str =
    r#"{"limit": 200, "includeDerivedTitles": true, "includeLastMessage": true}"#;

pub enum Req {
    Agents,
    Sessions,
    Subscribed,
    History { session: String },
    Send { session: String },
    Abort,
    Approve { id: String },
    Files { agent: String },
    FileGet { agent: String, name: String },
    FileSet { agent: String, name: String },
    Skills,
    SkillToggle,
    Jobs,
    JobToggle,
    JobRun,
    CreateBot,
    UpdateBot,
    DeleteBot,
    CreateSession { agent: String, first: Option<Outgoing> },
    Rename,
    Archive { session: String },
    Models,
    Mgr(crate::manager::MgrReq),
}

/// A message on its way to a session that doesn't exist yet.
#[derive(Clone)]
pub struct Outgoing {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Clone)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

impl Attachment {
    fn wire(&self) -> Value {
        json!({
            "type": if self.mime.starts_with("image/") { "image" } else { "file" },
            "mimeType": self.mime,
            "fileName": self.name,
            "content": STANDARD.encode(&self.bytes),
            "origin": "file",
        })
    }
}

#[derive(Default)]
pub struct Chat {
    pub messages: Vec<ChatMessage>,
    pub loading: bool,
    pub run_id: Option<String>,
    pub streaming: bool,
    pub live_text: String,
    pub tools: Vec<ToolStep>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum InfoTab {
    Activity,
    Files,
    Routines,
    Skills,
    Details,
}

/// The form for adding or editing a connection.
pub struct ProfileForm {
    pub profile: Profile,
    pub secret: String,
    pub is_new: bool,
}

#[derive(Default)]
pub struct NewBotForm {
    pub open: bool,
    pub name: String,
    pub workspace: String,
    pub error: Option<String>,
}

pub struct FileEditor {
    pub agent: String,
    pub name: String,
    pub text: String,
    pub loading: bool,
    pub dirty: bool,
}

pub struct App {
    pub settings: AppSettings,
    pub store: SecretStore,
    pub identity: DeviceIdentity,
    pub local: LocalGateway,
    client: Option<Client>,
    events: Option<Receiver<NetEvent>>,
    pending: HashMap<u64, Req>,
    pub conn: ConnState,
    pub hello: Option<HelloInfo>,

    pub agents: Vec<Agent>,
    pub default_agent: String,
    pub sessions: Vec<SessionRow>,
    pub selected_agent: Option<String>,
    pub selected_session: Option<String>,
    pub chats: HashMap<String, Chat>,
    pub approvals: Vec<Approval>,
    pub routines: Vec<Routine>,
    pub skills: Vec<Skill>,
    pub files: Vec<BotFile>,
    pub models: Vec<String>,
    pub file_editor: Option<FileEditor>,
    pub mgr: crate::manager::Manager,

    pub draft: String,
    pub attachments: Vec<Attachment>,
    pub search: String,
    pub info_tab: InfoTab,
    pub info_agent: String,
    pub stick_to_bottom: bool,
    pub show_settings: bool,
    pub profile_form: Option<ProfileForm>,
    pub new_bot: NewBotForm,
    pub look_for: Option<String>,
    pub rename: Option<(String, String)>,
    pub rename_bot: Option<(String, String)>,
    pub confirm_delete_bot: Option<String>,
    pub secret_prompt: String,
    pub toast: Option<(String, bool, Instant)>,
    pub markdown: egui_commonmark::CommonMarkCache,
    last_sessions_request: Instant,
    sessions_dirty: bool,
    pub repaint: Option<egui::Context>,
    pub updater: crate::update::Updater,

    // --screenshot
    pub screenshot_path: Option<std::path::PathBuf>,
    pub frames: u32,
    screenshot_sent: bool,
    screenshot_done: Arc<AtomicBool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<std::path::PathBuf>) -> Self {
        crate::look::load_fonts(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut store = SecretStore::open();
        let identity = DeviceIdentity::load_or_create(&mut store);
        let settings = AppSettings::load();
        let demo = screenshot.is_some() && !std::env::args().any(|a| a == "--live");
        let mut app = Self {
            local: detect_local(),
            settings,
            store,
            identity,
            client: None,
            events: None,
            pending: HashMap::new(),
            conn: ConnState::Idle,
            hello: None,
            agents: vec![],
            default_agent: String::new(),
            sessions: vec![],
            selected_agent: None,
            selected_session: None,
            chats: HashMap::new(),
            approvals: vec![],
            routines: vec![],
            skills: vec![],
            files: vec![],
            models: vec![],
            file_editor: None,
            mgr: Default::default(),
            draft: String::new(),
            attachments: vec![],
            search: String::new(),
            info_tab: InfoTab::Activity,
            info_agent: String::new(),
            stick_to_bottom: true,
            show_settings: false,
            profile_form: None,
            new_bot: NewBotForm::default(),
            look_for: None,
            rename: None,
            rename_bot: None,
            confirm_delete_bot: None,
            secret_prompt: String::new(),
            toast: None,
            markdown: Default::default(),
            last_sessions_request: Instant::now() - Duration::from_secs(10),
            sessions_dirty: false,
            repaint: Some(cc.egui_ctx.clone()),
            updater: crate::update::Updater::new(!demo),
            screenshot_path: screenshot,
            frames: 0,
            screenshot_sent: false,
            screenshot_done: Arc::new(AtomicBool::new(false)),
        };
        // Toolkit de flota: recordar carpeta e intérprete entre sesiones.
        app.mgr.fleet.dir = app.settings.fleet_dir.clone();
        if !app.settings.fleet_python.trim().is_empty() {
            app.mgr.fleet.python = app.settings.fleet_python.clone();
        }
        if demo {
            app.fill_demo();
        } else {
            app.connect(&cc.egui_ctx);
            // `--section <name>` opens one section, for checking a real connection.
            let args: Vec<String> = std::env::args().collect();
            if let Some(s) = args.iter().position(|a| a == "--section").and_then(|i| args.get(i + 1)).and_then(|n| crate::manager::Section::from_name(n)) {
                app.mgr.section = s;
            }
        }
        app
    }

    /// Sample content for the automatic screenshot, so a private conversation
    /// never ends up saved in a picture. `--live` shows the real connection.
    fn fill_demo(&mut self) {
        self.conn = ConnState::Connected;
        self.hello = Some(HelloInfo {
            version: "demo".into(),
            scopes: vec!["operator.admin".into()],
            ..Default::default()
        });
        self.agents = DEMO_BOTS
            .iter()
            .map(|(id, name, model)| Agent {
                id: id.to_string(),
                name: name.to_string(),
                model: model.to_string(),
                emoji: String::new(),
                workspace: format!("~/bots/{id}"),
            })
            .collect();
        let row = |key: &str, agent: &str, title: &str, main: bool, active: bool| SessionRow {
            key: key.into(),
            agent_id: agent.into(),
            title: title.into(),
            is_main: main,
            active,
            archived: false,
            unread: false,
            updated_ms: 0,
            session_id: String::new(),
            model: "claude-opus-5-5".into(),
        };
        self.sessions = vec![
            row("agent:ayudante:main", "ayudante", "Plan de la semana", true, false),
            row("agent:ayudante:c1", "ayudante", "Resumen del informe", false, false),
            row("agent:ayudante:c2", "ayudante", "Ideas para el viaje", false, false),
            row("agent:notas:main", "notas", "Notas de hoy", true, true),
        ];
        self.selected_agent = Some("ayudante".into());
        self.selected_session = Some("agent:ayudante:main".into());
        self.info_agent = "ayudante".into();
        let at = now_ms();
        let msg = |role, text: &str| ChatMessage {
            role,
            text: text.into(),
            attachments: vec![],
            at_ms: at,
            pending: false,
            failed: false,
        };
        let chat = Chat {
            messages: vec![
                msg(Role::User, "¿Qué tengo pendiente esta semana?"),
                msg(
                    Role::Assistant,
                    "Revisé tu lista. Esto es lo que queda:\n\n- **Lunes:** enviar el informe\n- **Miércoles:** reunión de equipo\n- **Viernes:** cerrar el presupuesto\n\n¿Quieres que te recuerde alguna de estas?",
                ),
                msg(Role::User, "Sí, recuérdame lo del presupuesto el jueves."),
            ],
            tools: vec![ToolStep {
                id: "t1".into(),
                name: "agenda".into(),
                detail: "buscando lo pendiente".into(),
                running: true,
                failed: false,
            }],
            run_id: Some("demo".into()),
            streaming: true,
            live_text: "Listo, crearé el recordatorio para el jueves por la mañana.".into(),
            ..Default::default()
        };
        self.chats.insert("agent:ayudante:main".into(), chat);
        self.approvals.push(Approval {
            id: "a1".into(),
            plugin: false,
            title: "Quiere ejecutar un comando".into(),
            detail: "crear-recordatorio --dia jueves".into(),
            cwd: "~/bots/ayudante".into(),
            agent_id: "ayudante".into(),
            expires_ms: 0,
        });
        self.routines = vec![Routine {
            id: "r1".into(),
            name: "Resumen de la mañana".into(),
            agent_id: "ayudante".into(),
            enabled: true,
            schedule: "0 9 * * *".into(),
            last: "Última vez: bien".into(),
            revision: String::new(),
        }];
        self.skills = vec![Skill {
            key: "agenda".into(),
            name: "Agenda".into(),
            emoji: String::new(),
            description: "Lee y crea recordatorios".into(),
            enabled: true,
            eligible: true,
            missing: String::new(),
        }];
        self.files = vec![BotFile { name: "AGENTS.md".into(), missing: false, size: 1200 }];
        // `--view` picks which screen a test picture shows.
        let args: Vec<String> = std::env::args().collect();
        let view = args.iter().position(|a| a == "--view").and_then(|i| args.get(i + 1)).cloned();
        self.fill_demo_manager(view.as_deref());
        match view.as_deref() {
            Some("settings") => self.show_settings = true,
            Some("remote") => {
                self.profile_form = Some(ProfileForm {
                    profile: Profile {
                        name: "Oficina".into(),
                        kind: ProfileKind::Ssh,
                        ssh_target: "usuario@servidor".into(),
                        ..Default::default()
                    },
                    secret: String::new(),
                    is_new: true,
                });
            }
            Some("pairing") => {
                self.conn = ConnState::Pairing { request_id: "abc123".into() };
            }
            Some("offline") => {
                self.conn = ConnState::Disconnected("No se pudo llegar a 127.0.0.1:18789".into());
            }
            _ => {}
        }
    }

    pub fn palette(&self) -> crate::look::Palette {
        crate::look::palette(self.settings.theme)
    }

    pub fn active_profile(&self) -> Profile {
        self.settings.active_profile().clone()
    }

    pub fn is_connected(&self) -> bool {
        self.conn == ConnState::Connected
    }

    pub fn is_admin(&self) -> bool {
        self.hello
            .as_ref()
            .is_some_and(|h| h.scopes.iter().any(|s| s == "operator.admin"))
    }

    /// (Re)starts the connection for the active profile.
    pub fn connect(&mut self, ctx: &egui::Context) {
        self.disconnect();
        self.local = detect_local();
        let profile = self.active_profile();
        let target: Target = target_for(&profile, &self.store);
        let ctx2 = ctx.clone();
        let (client, rx) = Client::start(target, self.identity.clone(), move || ctx2.request_repaint());
        self.client = Some(client);
        self.events = Some(rx);
    }

    pub fn disconnect(&mut self) {
        if let Some(c) = self.client.take() {
            c.stop();
        }
        self.events = None;
        self.pending.clear();
        self.conn = ConnState::Idle;
        self.hello = None;
        self.agents.clear();
        self.sessions.clear();
        self.chats.clear();
        self.approvals.clear();
        self.routines.clear();
        self.skills.clear();
        self.files.clear();
        self.models.clear();
        self.mgr.loading.clear();
        self.mgr.wizard = None;
        self.mgr.qr = None;
        self.selected_agent = None;
        self.selected_session = None;
        self.file_editor = None;
    }

    pub fn switch_profile(&mut self, id: String, ctx: &egui::Context) {
        if self.settings.active == id {
            return;
        }
        self.settings.active = id;
        self.settings.save();
        self.connect(ctx);
    }

    pub fn call(&mut self, method: &str, params: Value, req: Req) {
        if let Some(c) = self.client.as_mut() {
            let id = c.request(method, params);
            self.pending.insert(id, req);
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, ok: bool) {
        self.toast = Some((text.into(), ok, Instant::now()));
    }

    // ---- network ---------------------------------------------------------

    pub fn pump(&mut self, ctx: &egui::Context) {
        let mut got = vec![];
        if let Some(rx) = &self.events {
            while let Ok(ev) = rx.try_recv() {
                got.push(ev);
            }
        }
        for ev in got {
            self.handle(ev, ctx);
        }
        self.mgr_pump(ctx);
        if self.sessions_dirty && self.last_sessions_request.elapsed() > Duration::from_millis(800) {
            self.sessions_dirty = false;
            self.load_sessions();
        }
    }

    fn handle(&mut self, ev: NetEvent, ctx: &egui::Context) {
        match ev {
            NetEvent::State(s) => {
                let connected = s == ConnState::Connected;
                if !connected && self.conn == ConnState::Connected {
                    self.pending.clear();
                    for chat in self.chats.values_mut() {
                        if chat.run_id.is_some() {
                            chat.error = Some("Se perdió la conexión durante la respuesta.".into());
                        }
                        chat.run_id = None;
                        chat.streaming = false;
                        chat.live_text.clear();
                    }
                }
                self.conn = s;
            }
            NetEvent::Hello(h) => {
                self.default_agent = h.default_agent.clone().unwrap_or_default();
                self.hello = Some(h);
                self.bootstrap();
                self.mgr_bootstrap(ctx);
            }
            NetEvent::DeviceToken { route, token } => {
                if !self.store.set(&device_token_key(&route), &token) {
                    self.toast("No se pudo guardar el acceso en el almacén de Windows.", false);
                }
            }
            NetEvent::ForgetDeviceToken { route } => self.store.delete(&device_token_key(&route)),
            NetEvent::Response { id, result } => {
                if let Some(req) = self.pending.remove(&id) {
                    self.on_response(req, result, ctx);
                }
            }
            NetEvent::Event { event, payload } => self.on_event(&event, &payload, ctx),
        }
    }

    fn bootstrap(&mut self) {
        self.call("agents.list", json!({}), Req::Agents);
        let params: Value = serde_json::from_str(SESSION_LIST_PARAMS).unwrap();
        self.call("sessions.subscribe", params, Req::Subscribed);
        self.call("cron.list", json!({"includeDisabled": true}), Req::Jobs);
        self.call("exec.approval.list", json!({}), Req::Approve { id: "list".into() });
        self.call("models.list", json!({}), Req::Models);
        if let Some(key) = self.selected_session.clone() {
            self.load_history(&key);
        }
        if let Some(agent) = self.selected_agent.clone() {
            self.load_agent_panels(&agent);
        }
    }

    pub fn load_sessions(&mut self) {
        self.last_sessions_request = Instant::now();
        let params: Value = serde_json::from_str(SESSION_LIST_PARAMS).unwrap();
        self.call("sessions.list", params, Req::Sessions);
    }

    pub fn load_history(&mut self, key: &str) {
        self.chats.entry(key.to_string()).or_default().loading = true;
        self.call(
            "chat.history",
            json!({"sessionKey": key, "limit": 200}),
            Req::History { session: key.to_string() },
        );
    }

    pub fn load_agent_panels(&mut self, agent: &str) {
        self.info_agent = agent.to_string();
        self.call("agents.files.list", json!({"agentId": agent}), Req::Files { agent: agent.to_string() });
        self.call("skills.status", json!({"agentId": agent}), Req::Skills);
        self.call("cron.list", json!({"includeDisabled": true}), Req::Jobs);
    }

    fn on_response(&mut self, req: Req, result: Result<Value, crate::gateway::RpcError>, ctx: &egui::Context) {
        let req = match req {
            Req::Mgr(m) => {
                self.mgr_response(m, result, ctx);
                return;
            }
            other => other,
        };
        let v = match result {
            Ok(v) => v,
            Err(e) => {
                self.on_error(req, e);
                return;
            }
        };
        match req {
            Req::Agents => {
                self.agents = v["agents"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Agent::from_json).collect())
                    .unwrap_or_default();
                if let Some(id) = v["defaultId"].as_str() {
                    self.default_agent = id.to_string();
                }
                let keep = self
                    .selected_agent
                    .as_ref()
                    .is_some_and(|id| self.agents.iter().any(|a| &a.id == id));
                if !keep {
                    let start = self
                        .agents
                        .iter()
                        .find(|a| a.id == self.default_agent)
                        .or(self.agents.first())
                        .map(|a| a.id.clone());
                    if let Some(id) = start {
                        self.select_agent(&id);
                    }
                }
            }
            Req::Subscribed => {
                if v.get("list").is_some() {
                    self.apply_sessions(&v["list"]);
                }
            }
            Req::Sessions => self.apply_sessions(&v),
            Req::History { session } => {
                let chat = self.chats.entry(session.clone()).or_default();
                chat.messages = parse_history(&v);
                chat.loading = false;
                // A run that was in progress when we opened or reconnected.
                if let Some(run) = v.get("inFlightRun") {
                    chat.run_id = run["runId"].as_str().map(str::to_string);
                    chat.streaming = chat.run_id.is_some();
                    chat.live_text = run["text"].as_str().unwrap_or("").to_string();
                }
                if self.selected_session.as_deref() == Some(&session) {
                    self.stick_to_bottom = true;
                }
            }
            Req::Send { session } => {
                let chat = self.chats.entry(session).or_default();
                for m in chat.messages.iter_mut().rev() {
                    if m.pending {
                        m.pending = false;
                        break;
                    }
                }
                if let Some(run) = v["runId"].as_str() {
                    if chat.run_id.is_none() {
                        chat.run_id = Some(run.to_string());
                        chat.streaming = true;
                    }
                }
            }
            Req::Abort => {}
            Req::Approve { id } => {
                if id == "list" {
                    if let Some(a) = v.as_array() {
                        for x in a {
                            if let Some(ap) = Approval::from_event(x, false) {
                                self.add_approval(ap, ctx);
                            }
                        }
                    }
                } else {
                    self.approvals.retain(|a| a.id != id);
                }
            }
            Req::Files { agent } => {
                if self.info_agent == agent {
                    self.files = v["files"]
                        .as_array()
                        .map(|a| a.iter().filter_map(BotFile::from_json).collect())
                        .unwrap_or_default();
                }
            }
            Req::FileGet { agent, name } => {
                let text = v["file"]["content"].as_str().or(v["content"].as_str()).unwrap_or("").to_string();
                if let Some(ed) = &mut self.file_editor {
                    if ed.agent == agent && ed.name == name {
                        ed.text = text;
                        ed.loading = false;
                    }
                }
            }
            Req::FileSet { agent, name } => {
                if let Some(ed) = &mut self.file_editor {
                    if ed.agent == agent && ed.name == name {
                        ed.dirty = false;
                    }
                }
                self.toast("Archivo guardado", true);
                self.call("agents.files.list", json!({"agentId": agent}), Req::Files { agent });
            }
            Req::Skills => {
                self.skills = v["skills"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Skill::from_json).collect())
                    .unwrap_or_default();
                self.skills.sort_by(|a, b| b.eligible.cmp(&a.eligible).then(a.name.cmp(&b.name)));
            }
            Req::SkillToggle => {
                let agent = self.info_agent.clone();
                self.call("skills.status", json!({"agentId": agent}), Req::Skills);
            }
            Req::Jobs => {
                self.routines = v["jobs"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Routine::from_json).collect())
                    .unwrap_or_default();
            }
            Req::JobToggle => self.call("cron.list", json!({"includeDisabled": true}), Req::Jobs),
            Req::JobRun => self.toast("Rutina en marcha", true),
            Req::CreateBot => {
                let id = v["agentId"].as_str().unwrap_or("").to_string();
                self.new_bot = NewBotForm::default();
                self.call("agents.list", json!({}), Req::Agents);
                if !id.is_empty() {
                    self.toast("Bot creado", true);
                    self.select_agent(&id);
                }
            }
            Req::UpdateBot => {
                self.call("agents.list", json!({}), Req::Agents);
                self.toast("Bot actualizado", true);
            }
            Req::DeleteBot => {
                if let Some(id) = self.confirm_delete_bot.take() {
                    self.settings.pinned.retain(|x| x != &id);
                    self.settings.hidden.retain(|x| x != &id);
                    self.settings.save();
                    if self.selected_agent.as_deref() == Some(&id) {
                        self.selected_agent = None;
                        self.selected_session = None;
                    }
                }
                self.call("agents.list", json!({}), Req::Agents);
                self.load_sessions();
            }
            Req::CreateSession { agent, first } => {
                let key = v["key"].as_str().unwrap_or("").to_string();
                if key.is_empty() {
                    self.toast("OpenClaw no devolvió el nuevo chat", false);
                    return;
                }
                self.selected_agent = Some(agent);
                self.selected_session = Some(key.clone());
                self.chats.entry(key.clone()).or_default();
                self.load_sessions();
                if let Some(out) = first {
                    self.send_to(&key, out);
                }
            }
            Req::Rename | Req::Archive { .. } => self.load_sessions(),
            Req::Models => {
                self.models = v["models"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|m| m["available"].as_bool() != Some(false))
                            .filter_map(|m| {
                                let id = m["id"].as_str()?;
                                Some(match m["provider"].as_str() {
                                    Some(p) if !id.starts_with(&format!("{p}/")) => format!("{p}/{id}"),
                                    _ => id.to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Req::Mgr(_) => {}
        }
    }

    fn on_error(&mut self, req: Req, e: crate::gateway::RpcError) {
        let msg = if e.message.is_empty() { e.code.clone() } else { e.message.clone() };
        match req {
            Req::Send { session } => {
                let chat = self.chats.entry(session).or_default();
                chat.run_id = None;
                chat.streaming = false;
                for m in chat.messages.iter_mut().rev() {
                    if m.pending {
                        m.pending = false;
                        m.failed = true;
                        break;
                    }
                }
                chat.error = Some(format!("No se envió: {msg}"));
            }
            Req::History { session } => {
                let chat = self.chats.entry(session).or_default();
                chat.loading = false;
                chat.error = Some(format!("No se pudo cargar: {msg}"));
            }
            Req::CreateBot => self.new_bot.error = Some(msg),
            Req::CreateSession { first, .. } => {
                if let Some(out) = first {
                    self.draft = out.text;
                    self.attachments = out.attachments;
                }
                self.toast(format!("No se pudo crear el chat: {msg}"), false);
            }
            Req::Approve { id } if id == "list" => {}
            // Each of these only fills a panel; an error simply leaves it empty.
            Req::Skills | Req::Jobs | Req::Files { .. } | Req::Models | Req::Mgr(_) => {}
            _ => self.toast(msg, false),
        }
    }

    fn apply_sessions(&mut self, v: &Value) {
        let mut rows: Vec<SessionRow> = v["sessions"]
            .as_array()
            .map(|a| a.iter().filter_map(SessionRow::from_json).filter(|s| !s.archived).collect())
            .unwrap_or_default();
        rows.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms));
        self.sessions = rows;
        // The first load chooses which conversation to show for the selected Bot.
        if self.selected_session.is_none() {
            if let Some(agent) = self.selected_agent.clone() {
                self.open_default_session(&agent);
            }
        }
    }

    fn on_event(&mut self, event: &str, p: &Value, ctx: &egui::Context) {
        self.mgr_event(event, p, ctx);
        match event {
            "chat" => self.on_chat_event(p),
            "agent" => self.on_agent_event(p),
            "sessions.changed" => self.sessions_dirty = true,
            "exec.approval.requested" => {
                if let Some(a) = Approval::from_event(p, false) {
                    self.add_approval(a, ctx);
                }
            }
            "plugin.approval.requested" => {
                if let Some(a) = Approval::from_event(p, true) {
                    self.add_approval(a, ctx);
                }
            }
            "exec.approval.resolved" | "plugin.approval.resolved" => {
                if let Some(id) = p["id"].as_str() {
                    self.approvals.retain(|a| a.id != id);
                }
            }
            "shutdown" => {
                self.conn = ConnState::Disconnected("OpenClaw se está apagando".into());
            }
            _ => {}
        }
    }

    fn add_approval(&mut self, a: Approval, ctx: &egui::Context) {
        if self.approvals.iter().any(|x| x.id == a.id) {
            return;
        }
        if self.settings.notifications {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
        self.approvals.push(a);
    }

    fn on_chat_event(&mut self, p: &Value) {
        let Some(session) = p["sessionKey"].as_str().map(str::to_string) else { return };
        // Only the chats on screen (or in memory) need live text.
        if !self.chats.contains_key(&session) && self.selected_session.as_deref() != Some(&session) {
            return;
        }
        let selected = self.selected_session.as_deref() == Some(&session);
        let chat = self.chats.entry(session.clone()).or_default();
        let run = p["runId"].as_str().map(str::to_string);
        if run.is_some() {
            chat.run_id = run;
        }
        match p["state"].as_str().unwrap_or("") {
            "delta" => {
                chat.streaming = true;
                let snapshot = (!p["message"].is_null()).then(|| message_text(&p["message"]));
                if let Some(t) = snapshot.filter(|t| !t.is_empty() || p["replace"].as_bool() == Some(true)) {
                    chat.live_text = t;
                } else if p["replace"].as_bool() == Some(true) {
                    chat.live_text = p["deltaText"].as_str().unwrap_or("").to_string();
                } else {
                    chat.live_text.push_str(p["deltaText"].as_str().unwrap_or(""));
                }
                if selected {
                    self.stick_to_bottom = true;
                }
            }
            state @ ("final" | "aborted" | "error") => {
                let text = if p.get("message").is_some() && !p["message"].is_null() {
                    message_text(&p["message"])
                } else {
                    std::mem::take(&mut chat.live_text)
                };
                chat.live_text.clear();
                chat.run_id = None;
                chat.streaming = false;
                for t in chat.tools.iter_mut() {
                    t.running = false;
                }
                if !text.trim().is_empty() {
                    chat.messages.push(ChatMessage {
                        role: Role::Assistant,
                        text,
                        attachments: vec![],
                        at_ms: now_ms(),
                        pending: false,
                        failed: false,
                    });
                }
                if state == "error" {
                    chat.error = Some(
                        p["errorMessage"].as_str().unwrap_or("La respuesta falló.").to_string(),
                    );
                }
                if state == "aborted" {
                    chat.messages.push(ChatMessage {
                        role: Role::Notice,
                        text: "Detuviste la respuesta.".into(),
                        attachments: vec![],
                        at_ms: now_ms(),
                        pending: false,
                        failed: false,
                    });
                }
                if selected {
                    self.stick_to_bottom = true;
                }
                // The gateway's copy is the authoritative one.
                self.load_history(&session);
                self.sessions_dirty = true;
            }
            _ => {}
        }
    }

    fn on_agent_event(&mut self, p: &Value) {
        let Some(session) = p["sessionKey"].as_str() else { return };
        let Some(chat) = self.chats.get_mut(session) else { return };
        if p["stream"].as_str() != Some("tool") {
            return;
        }
        let data = &p["data"];
        let Some((phase, id, name, detail)) = tool_step(data) else { return };
        let is_error = data["isError"].as_bool().unwrap_or(false);
        match phase.as_str() {
            "start" => chat.tools.push(ToolStep {
                id,
                name,
                detail,
                running: true,
                failed: false,
            }),
            "result" | "end" => {
                if let Some(t) = chat.tools.iter_mut().rev().find(|t| t.id == id || (id.is_empty() && t.running)) {
                    t.running = false;
                    t.failed = is_error;
                } else {
                    chat.tools.push(ToolStep {
                        id,
                        name,
                        detail,
                        running: false,
                        failed: is_error,
                    });
                }
            }
            _ => {}
        }
        if chat.tools.len() > 200 {
            chat.tools.drain(..chat.tools.len() - 200);
        }
    }

    // ---- actions ---------------------------------------------------------

    pub fn select_agent(&mut self, id: &str) {
        self.selected_agent = Some(id.to_string());
        self.selected_session = None;
        self.open_default_session(id);
        if self.is_connected() {
            self.load_agent_panels(id);
        }
    }

    /// Opens the Bot's most recent chat (or its main one).
    fn open_default_session(&mut self, agent: &str) {
        let key = self
            .sessions
            .iter()
            .find(|s| s.agent_id == agent && s.is_main)
            .or_else(|| self.sessions.iter().find(|s| s.agent_id == agent))
            .map(|s| s.key.clone());
        if let Some(key) = key {
            self.open_session(key);
        }
    }

    pub fn open_session(&mut self, key: String) {
        self.selected_session = Some(key.clone());
        self.stick_to_bottom = true;
        if self.is_connected() {
            self.load_history(&key);
        }
    }

    pub fn current_chat(&self) -> Option<&Chat> {
        self.selected_session.as_ref().and_then(|k| self.chats.get(k))
    }

    pub fn current_agent(&self) -> Option<&Agent> {
        self.selected_agent
            .as_ref()
            .and_then(|id| self.agents.iter().find(|a| &a.id == id))
    }

    pub fn current_session(&self) -> Option<&SessionRow> {
        self.selected_session
            .as_ref()
            .and_then(|k| self.sessions.iter().find(|s| &s.key == k))
    }

    pub fn chats_of(&self, agent: &str) -> Vec<SessionRow> {
        self.sessions.iter().filter(|s| s.agent_id == agent).cloned().collect()
    }

    pub fn agent_busy(&self, agent: &str) -> bool {
        self.sessions.iter().any(|s| s.agent_id == agent && s.active)
    }

    pub fn send_draft(&mut self) {
        let text = self.draft.trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if !self.is_connected() {
            self.toast("Aún no hay conexión con OpenClaw", false);
            return;
        }
        let Some(agent) = self.selected_agent.clone() else {
            self.toast("Elige un Bot primero", false);
            return;
        };
        if self.attachments.iter().map(|a| a.bytes.len() as u64).sum::<u64>() * 4 / 3
            > self.hello.as_ref().map(|h| h.max_payload).unwrap_or(crate::gateway::DEFAULT_MAX_PAYLOAD)
        {
            self.toast("Los adjuntos pesan demasiado para enviarse juntos", false);
            return;
        }
        let out = Outgoing {
            text,
            attachments: std::mem::take(&mut self.attachments),
        };
        self.draft.clear();
        match self.selected_session.clone() {
            Some(key) => self.send_to(&key, out),
            None => self.create_chat(&agent, Some(out)),
        }
    }

    pub fn create_chat(&mut self, agent: &str, first: Option<Outgoing>) {
        let mut params = json!({"agentId": agent});
        if let Some(out) = &first {
            params["titleSource"] = json!(shorten(&out.text, 900));
        }
        self.call(
            "sessions.create",
            params,
            Req::CreateSession {
                agent: agent.to_string(),
                first,
            },
        );
    }

    fn send_to(&mut self, session: &str, out: Outgoing) {
        let names: Vec<String> = out.attachments.iter().map(|a| a.name.clone()).collect();
        let chat = self.chats.entry(session.to_string()).or_default();
        chat.error = None;
        chat.messages.push(ChatMessage {
            role: Role::User,
            text: out.text.clone(),
            attachments: names,
            at_ms: now_ms(),
            pending: true,
            failed: false,
        });
        chat.tools.clear();
        chat.streaming = true;
        chat.live_text.clear();
        self.stick_to_bottom = true;
        let mut params = json!({
            "sessionKey": session,
            "message": out.text,
            "deliver": false,
            "idempotencyKey": uuid::Uuid::new_v4().to_string(),
        });
        if !out.attachments.is_empty() {
            params["attachments"] = Value::Array(out.attachments.iter().map(Attachment::wire).collect());
        }
        self.call("chat.send", params, Req::Send { session: session.to_string() });
    }

    pub fn stop_run(&mut self) {
        let Some(key) = self.selected_session.clone() else { return };
        let mut params = json!({"sessionKey": key});
        if let Some(run) = self.chats.get(&key).and_then(|c| c.run_id.clone()) {
            params["runId"] = json!(run);
        }
        self.call("chat.abort", params, Req::Abort);
    }

    pub fn resolve_approval(&mut self, id: &str, plugin: bool, decision: &str) {
        let method = if plugin { "plugin.approval.resolve" } else { "exec.approval.resolve" };
        self.call(method, json!({"id": id, "decision": decision}), Req::Approve { id: id.to_string() });
    }

    pub fn create_bot(&mut self) {
        let name = self.new_bot.name.trim().to_string();
        if name.is_empty() {
            self.new_bot.error = Some("Ponle un nombre".into());
            return;
        }
        let mut params = json!({"name": name});
        let w = self.new_bot.workspace.trim();
        if !w.is_empty() {
            params["workspace"] = json!(w);
        }
        self.call("agents.create", params, Req::CreateBot);
    }

    pub fn update_bot_name(&mut self, id: &str, name: &str) {
        self.call("agents.update", json!({"agentId": id, "name": name}), Req::UpdateBot);
    }

    pub fn update_bot_model(&mut self, id: &str, model: Option<&str>) {
        self.call("agents.update", json!({"agentId": id, "model": model}), Req::UpdateBot);
    }

    pub fn delete_bot(&mut self, id: &str) {
        self.confirm_delete_bot = Some(id.to_string());
    }

    pub fn delete_bot_now(&mut self, id: &str) {
        self.call("agents.delete", json!({"agentId": id}), Req::DeleteBot);
    }

    pub fn rename_chat(&mut self, key: &str, label: &str) {
        self.call("sessions.patch", json!({"key": key, "label": label}), Req::Rename);
    }

    pub fn archive_chat(&mut self, key: &str) {
        let mut params = json!({"key": key, "archived": true});
        if let Some(row) = self.sessions.iter().find(|s| s.key == key) {
            if !row.session_id.is_empty() {
                params["expectedSessionId"] = json!(row.session_id);
            }
        }
        self.call("sessions.patch", params, Req::Archive { session: key.to_string() });
        if self.selected_session.as_deref() == Some(key) {
            self.selected_session = None;
            if let Some(a) = self.selected_agent.clone() {
                self.open_default_session(&a);
            }
        }
    }

    pub fn toggle_skill(&mut self, key: &str, enabled: bool) {
        self.call("skills.update", json!({"skillKey": key, "enabled": enabled}), Req::SkillToggle);
    }

    pub fn reload_routines(&mut self) {
        self.call("cron.list", json!({"includeDisabled": true}), Req::Jobs);
    }

    pub fn toggle_routine(&mut self, r: &Routine) {
        self.call(
            "cron.update",
            json!({"id": r.id, "expectedConfigRevision": r.revision, "patch": {"enabled": !r.enabled}}),
            Req::JobToggle,
        );
    }

    pub fn run_routine(&mut self, id: &str) {
        self.call("cron.run", json!({"id": id, "mode": "force"}), Req::JobRun);
    }

    pub fn open_file(&mut self, agent: &str, name: &str) {
        self.file_editor = Some(FileEditor {
            agent: agent.to_string(),
            name: name.to_string(),
            text: String::new(),
            loading: true,
            dirty: false,
        });
        self.call(
            "agents.files.get",
            json!({"agentId": agent, "name": name}),
            Req::FileGet { agent: agent.to_string(), name: name.to_string() },
        );
    }

    pub fn save_file(&mut self) {
        let Some(ed) = &self.file_editor else { return };
        let (agent, name, content) = (ed.agent.clone(), ed.name.clone(), ed.text.clone());
        self.call(
            "agents.files.set",
            json!({"agentId": agent, "name": name, "content": content}),
            Req::FileSet { agent, name },
        );
    }

    pub fn add_file(&mut self, path: &std::path::Path) {
        let limit = self.hello.as_ref().map(|h| h.max_attachment).unwrap_or(crate::gateway::DEFAULT_MAX_ATTACHMENT);
        let image_limit = self.hello.as_ref().map(|h| h.max_image).unwrap_or(crate::gateway::DEFAULT_MAX_IMAGE);
        let is_image = path
            .file_name()
            .map(|n| mime_for(&n.to_string_lossy()).starts_with("image/"))
            .unwrap_or(false);
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() as u64 > limit => {
                self.toast("Ese archivo es más grande de lo que OpenClaw acepta", false);
            }
            Ok(bytes) if is_image && bytes.len() as u64 > image_limit => {
                self.toast("Esa imagen es más grande de lo que OpenClaw acepta", false);
            }
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "archivo".into());
                let mime = mime_for(&name);
                if self.attachments.iter().filter(|a| a.mime.starts_with("image/")).count() >= 8 && mime.starts_with("image/") {
                    self.toast("Máximo 8 imágenes por mensaje", false);
                    return;
                }
                self.attachments.push(Attachment { name, mime, bytes });
            }
            Err(e) => self.toast(format!("No se pudo leer el archivo: {e}"), false),
        }
    }

    /// Saves a profile from the form. Returns true when the active connection changed.
    pub fn save_profile(&mut self, form: ProfileForm, ctx: &egui::Context) {
        let ProfileForm { mut profile, secret, .. } = form;
        if profile.name.trim().is_empty() {
            profile.name = "Remoto".into();
        }
        let key = profile.secret_key();
        if profile.secret_kind == SecretKind::None {
            self.store.delete(&key);
        } else if !secret.trim().is_empty() {
            if !self.store.set(&key, secret.trim()) {
                self.toast("Se guardará solo durante esta sesión", false);
            }
        }
        if let Some(existing) = self.settings.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            self.settings.profiles.push(profile.clone());
        }
        self.settings.active = profile.id.clone();
        self.settings.save();
        self.connect(ctx);
    }

    pub fn delete_profile(&mut self, id: &str, ctx: &egui::Context) {
        let Some(p) = self.settings.profiles.iter().find(|p| p.id == id).cloned() else { return };
        if p.kind == ProfileKind::Local {
            return;
        }
        self.store.delete(&p.secret_key());
        self.settings.profiles.retain(|p| p.id != id);
        if self.settings.active == id {
            self.settings.active = "local".into();
            self.settings.save();
            self.connect(ctx);
        } else {
            self.settings.save();
        }
    }

    pub fn profile_has_secret(&self, p: &Profile) -> bool {
        self.store.get(&p.secret_key()).is_some()
    }

    /// Saves the secret typed on the "needs a key" screen.
    pub fn save_secret_and_retry(&mut self, ctx: &egui::Context) {
        let profile = self.active_profile();
        let secret = self.secret_prompt.trim().to_string();
        if secret.is_empty() {
            return;
        }
        self.store.set(&profile.secret_key(), &secret);
        self.secret_prompt.clear();
        self.connect(ctx);
    }

    pub fn forget_device_access(&mut self, ctx: &egui::Context) {
        let target = target_for(&self.active_profile(), &self.store);
        self.store.delete(&device_token_key(&target.route));
        self.connect(ctx);
    }

    // ---- automatic screenshot --------------------------------------------

    pub fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot_path.clone() else { return };
        ctx.request_repaint();
        self.frames += 1;
        if self.screenshot_done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // `--frames` gives a live connection time to load before the picture.
        let wanted = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|w| w[0] == "--frames")
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(120);
        let ready = self.frames >= wanted;
        if ready && !self.screenshot_sent {
            self.screenshot_sent = true;
            let done = self.screenshot_done.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(
                move |img: Arc<egui::ColorImage>| {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let [w, h] = img.size;
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                    if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, bytes) {
                        let _ = buf.save(&path);
                    }
                    done.store(true, Ordering::SeqCst);
                },
            )));
        }
    }
}

pub const DEMO_BOTS: [(&str, &str, &str); 3] = [
    ("ayudante", "Ayudante", "claude-opus-5-5"),
    ("notas", "Notas", "claude-haiku-5-5"),
    ("viajes", "Viajes", "claude-haiku-5-5"),
];

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn mime_for(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" | "csv" => "text/plain",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub fn time_label(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if ms <= 0 {
        return String::new();
    }
    let Some(dt) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let now = Local::now();
    if dt.date_naive() == now.date_naive() {
        dt.format("%H:%M").to_string()
    } else {
        dt.format("%d/%m %H:%M").to_string()
    }
}

pub fn reconnect_note(conn: &ConnState) -> (String, bool) {
    match conn {
        ConnState::Idle => ("Sin conexión".into(), false),
        ConnState::Tunnel => ("Abriendo el túnel SSH…".into(), false),
        ConnState::Connecting => ("Conectando con OpenClaw…".into(), false),
        ConnState::Connected => ("Conectado".into(), true),
        ConnState::Pairing { .. } => ("Falta aprobar este equipo".into(), false),
        ConnState::AuthFailed(m) => (m.clone(), false),
        ConnState::Disconnected(m) => (m.clone(), false),
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump(ctx);
        self.updater.tick(ctx);
        self.screenshot_step(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui::draw(self, ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.disconnect();
        self.settings.save();
    }
}
