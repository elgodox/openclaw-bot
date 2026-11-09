//! `--probe`: a command-line check of the connection. Prints only the shape of
//! what the gateway returns (field names and counts), never the values.

use crate::gateway::{Client, ConnState, NetEvent, Secret, SshSpec, Target};
use crate::identity::{DeviceIdentity, SecretStore};
use crate::settings::{detect_local, AppSettings, Profile, ProfileKind, SecretKind};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Builds the connection target for a profile.
pub fn target_for(profile: &Profile, store: &SecretStore) -> Target {
    let scopes: Vec<String> = {
        let mut s = vec!["operator.read", "operator.write", "operator.approvals"];
        if profile.admin {
            // OpenClaw's admin scope already covers device pairing.
            s.insert(0, "operator.admin");
        } else {
            s.push("operator.pairing");
        }
        s.into_iter().map(String::from).collect()
    };
    let stored = store.get(&profile.secret_key());
    let (url, secret, route, ssh) = match profile.kind {
        ProfileKind::Local => {
            let local = detect_local();
            let secret = match profile.secret_kind {
                SecretKind::None => None,
                _ => stored_secret(profile, stored)
                    .or_else(|| local.token.clone().map(Secret::Token))
                    .or_else(|| local.password.clone().map(Secret::Password)),
            };
            let route = format!("local:{}", local.port);
            (local.url(), secret, route, None)
        }
        ProfileKind::Direct => (
            profile.url.trim().to_string(),
            stored_secret(profile, stored),
            format!("url:{}#{}", profile.url.trim(), profile.tls_fingerprint.trim()),
            None,
        ),
        ProfileKind::Ssh => (
            String::new(),
            stored_secret(profile, stored),
            format!("ssh:{}:{}", profile.ssh_target.trim(), profile.remote_port),
            Some(SshSpec {
                target: profile.ssh_target.clone(),
                identity: profile.ssh_identity.clone(),
                remote_port: profile.remote_port,
            }),
        ),
    };
    let tls_pin = Some(profile.tls_fingerprint.trim().to_string()).filter(|s| !s.is_empty());
    let device_token = store.get(&device_token_key(&route));
    Target {
        url,
        secret,
        device_token,
        route,
        tls_pin,
        ssh,
        scopes,
    }
}

pub fn device_token_key(route: &str) -> String {
    format!("device-token:{route}")
}

fn stored_secret(profile: &Profile, stored: Option<String>) -> Option<Secret> {
    match profile.secret_kind {
        SecretKind::None => None,
        SecretKind::Password => stored.map(Secret::Password),
        SecretKind::Token => stored.map(Secret::Token),
    }
}

fn outline(v: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                let t = match x {
                    Value::Object(m) => format!("object({})", m.len()),
                    Value::Array(a) => format!("array[{}]", a.len()),
                    Value::String(_) => "string".into(),
                    Value::Number(_) => "number".into(),
                    Value::Bool(_) => "bool".into(),
                    Value::Null => "null".into(),
                };
                out.push_str(&format!("{pad}{k}: {t}\n"));
                if depth < 3 && (x.is_object() || x.is_array()) {
                    outline(x, depth + 1, out);
                }
            }
        }
        Value::Array(a) => {
            if let Some(x) = a.first() {
                out.push_str(&format!("{pad}[0]:\n"));
                outline(x, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// `--probe-admin`: reads everything the administration screens use, and prints
/// only whether each worked and the shape of the answer. Changes nothing: the
/// guided-setup check starts a session and cancels it at once.
fn run_admin() -> i32 {
    let mut store = SecretStore::open();
    let identity = DeviceIdentity::load_or_create(&mut store);
    let settings = AppSettings::load();
    let profile = settings.active_profile().clone();
    let target = target_for(&profile, &store);
    let (mut client, rx) = Client::start(target, identity, || {});
    let reads: Vec<(&str, Value)> = vec![
        ("health", json!({})),
        ("status", json!({})),
        ("system.info", json!({})),
        ("update.status", json!({})),
        ("update.runs.list", json!({})),
        ("backup.status", json!({})),
        ("channels.status", json!({"probe": false})),
        ("channels.pairing.list", json!({})),
        ("node.list", json!({})),
        ("node.pair.list", json!({})),
        ("device.pair.list", json!({})),
        ("usage.status", json!({})),
        ("usage.cost", json!({"days": 7})),
        ("models.authStatus", json!({"refresh": false})),
        ("models.list", json!({"view": "all"})),
        ("config.get", json!({})),
        ("config.schema", json!({})),
        ("plugins.list", json!({})),
        ("hooks.status", json!({})),
        ("skills.status", json!({})),
        ("cron.list", json!({"includeDisabled": true})),
        ("cron.status", json!({})),
        ("exec.approvals.get", json!({})),
        ("approval.history", json!({"limit": 5})),
        ("secrets.store.list", json!({})),
        ("logs.tail", json!({"limit": 5, "maxBytes": 20000})),
        ("catalog.browse", json!({"kind": "plugin", "pageSize": 1})),
    ];
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut pending: std::collections::HashMap<u64, String> = Default::default();
    let mut started = false;
    let mut failed = 0;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(NetEvent::Hello(h)) => {
                println!("conectado a OpenClaw {}  permisos: {:?}  funciones: {}", h.version, h.scopes, h.methods.len());
                for (m, p) in &reads {
                    pending.insert(client.request(m, p.clone()), m.to_string());
                }
                pending.insert(client.request("wizard.start", json!({"flow": "channels", "channel": "telegram"})), "wizard.start".into());
                started = true;
            }
            Ok(NetEvent::State(ConnState::AuthFailed(m))) => {
                println!("no entró: {m}");
                return 1;
            }
            Ok(NetEvent::Response { id, result }) => {
                let Some(m) = pending.remove(&id) else { continue };
                match result {
                    Ok(v) => {
                        if m == "config.schema" {
                            let mut ed = crate::config_edit::ConfigEditor::default();
                            ed.set_schema(&v);
                            println!("--- config.schema: {} grupos; ejemplo: {:?}", ed.groups.len(), ed.groups.iter().take(3).map(|g| g.1.clone()).collect::<Vec<_>>());
                            ed.search = "telegram bot token".into();
                            println!("    búsqueda 'telegram bot token': {} resultados", ed.search_results(50).len());
                            let (node, hint) = ed.node_for(&["gateway".into(), "bind".into()]).unwrap_or_default();
                            println!("    gateway.bind se edita como {:?} (ayuda en {hint})", crate::config_edit::classify(&node).0);
                            let (node, _) = ed.node_for(&["gateway".into(), "auth".into(), "token".into()]).unwrap_or_default();
                            println!("    gateway.auth.token se edita como {:?}", crate::config_edit::classify(&node).0);
                        } else if m == "wizard.start" {
                            let step = &v["step"];
                            println!("--- wizard.start: sesión {} primer paso tipo {:?} opciones {}", v["sessionId"].is_string(), step["type"].as_str(), step["options"].as_array().map(|a| a.len()).unwrap_or(0));
                            if let Some(s) = v["sessionId"].as_str() {
                                pending.insert(client.request("wizard.cancel", json!({"sessionId": s})), "wizard.cancel".into());
                            }
                        } else {
                            let mut o = String::new();
                            outline(&v, 1, &mut o);
                            let short: Vec<&str> = o.lines().take(14).collect();
                            println!("--- {m}\n{}", short.join("\n"));
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        println!("--- {m}: ERROR {} {}", e.code, e.message);
                    }
                }
                if started && pending.is_empty() {
                    break;
                }
            }
            _ => {}
        }
    }
    client.stop();
    println!("errores: {failed}  sin respuesta: {}", pending.len());
    if failed == 0 && pending.is_empty() { 0 } else { 1 }
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--probe-admin") {
        return run_admin();
    }
    let mut store = SecretStore::open();
    let identity = DeviceIdentity::load_or_create(&mut store);
    let settings = AppSettings::load();
    let profile = settings.active_profile().clone();
    let target = target_for(&profile, &store);
    println!("perfil: {} ({:?})  ruta: {}", profile.name, profile.kind, target.route);
    println!("equipo: {}", &identity.device_id[..16]);
    println!(
        "clave: {}  token guardado: {}",
        if target.secret.is_some() { "sí" } else { "no" },
        if target.device_token.is_some() { "sí" } else { "no" }
    );
    let (mut client, rx) = Client::start(target, identity, || {});

    let session_key = arg_value(args, "--history");
    let methods: Vec<(&str, Value)> = vec![
        ("agents.list", json!({})),
        (
            "sessions.list",
            json!({"limit": 3, "includeDerivedTitles": true, "includeLastMessage": true}),
        ),
        ("cron.list", json!({"includeDisabled": true})),
        ("skills.status", json!({})),
        ("exec.approval.list", json!({})),
        ("models.list", json!({})),
    ];

    // `--send "texto"` runs one real message through a new chat, then archives it.
    let send_text = arg_value(args, "--send");
    let send_agent = arg_value(args, "--agent");
    let archive_key = arg_value(args, "--archive");
    let mut send_session: Option<String> = None;
    let mut send_session_id: Option<String> = None;
    let deadline = Instant::now() + Duration::from_secs(if send_text.is_some() { 180 } else { 30 });
    let mut pending: std::collections::HashMap<u64, String> = Default::default();
    let mut connected = false;
    let mut exit = 1;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(NetEvent::State(s)) => {
                println!("estado: {s:?}");
                if let ConnState::Connected = s {
                    connected = true;
                }
                if matches!(s, ConnState::AuthFailed(_) | ConnState::Pairing { .. }) {
                    break;
                }
            }
            Ok(NetEvent::Hello(h)) => {
                println!(
                    "conectado a OpenClaw {}  permisos: {:?}  métodos: {}",
                    h.version,
                    h.scopes,
                    h.methods.len()
                );
                if archive_key.is_some() {
                    let id = client.request("sessions.list", json!({"limit": 200}));
                    pending.insert(id, "archive-list".into());
                    continue;
                }
                if send_text.is_some() {
                    let params = match &send_agent {
                        Some(a) => json!({"agentId": a, "titleSource": "Prueba de OpenClaw Bots"}),
                        None => json!({"titleSource": "Prueba de OpenClaw Bots"}),
                    };
                    pending.insert(client.request("sessions.create", params), "sessions.create".into());
                    continue;
                }
                for (m, p) in &methods {
                    pending.insert(client.request(m, p.clone()), m.to_string());
                }
                if let Some(k) = &session_key {
                    let id = client.request("chat.history", json!({"sessionKey": k, "limit": 5}));
                    pending.insert(id, "chat.history".into());
                }
            }
            Ok(NetEvent::DeviceToken { route, token }) => {
                store.set(&device_token_key(&route), &token);
                println!("token de dispositivo guardado para {route}");
            }
            Ok(NetEvent::ForgetDeviceToken { route }) => {
                store.delete(&device_token_key(&route));
            }
            Ok(NetEvent::Response { id, result }) => {
                let Some(m) = pending.remove(&id) else { continue };
                if let Some(key) = &archive_key {
                    match (m.as_str(), result) {
                        ("archive-list", Ok(v)) => {
                            let row = v["sessions"].as_array().and_then(|a| a.iter().find(|s| s["key"] == key.as_str()));
                            let Some(row) = row else {
                                println!("no se encontró ese chat");
                                break;
                            };
                            let params = json!({"key": key, "archived": true, "expectedSessionId": row["sessionId"]});
                            pending.insert(client.request("sessions.patch", params), "archive-patch".into());
                        }
                        ("archive-patch", Ok(_)) => {
                            println!("chat archivado");
                            exit = 0;
                            break;
                        }
                        (other, Err(e)) => {
                            println!("{other}: ERROR {} {}", e.code, e.message);
                            break;
                        }
                        _ => {}
                    }
                    continue;
                }
                if send_text.is_some() {
                    match (m.as_str(), result) {
                        ("sessions.create", Ok(v)) => {
                            let key = v["key"].as_str().unwrap_or("").to_string();
                            println!("chat de prueba creado: {key}");
                            send_session_id = v["sessionId"].as_str().map(str::to_string);
                            let params = json!({
                                "sessionKey": key,
                                "message": send_text.clone().unwrap(),
                                "deliver": false,
                                "idempotencyKey": uuid::Uuid::new_v4().to_string(),
                            });
                            send_session = Some(key);
                            pending.insert(client.request("chat.send", params), "chat.send".into());
                        }
                        ("chat.send", Ok(v)) => println!("enviado; acuse: {}", v["status"]),
                        ("sessions.patch", Ok(_)) => {
                            println!("chat de prueba archivado");
                            exit = 0;
                            break;
                        }
                        (other, Err(e)) => {
                            println!("{other}: ERROR {} {}", e.code, e.message);
                            break;
                        }
                        _ => {}
                    }
                    continue;
                }
                match result {
                    Ok(v) => {
                        if m == "sessions.list" {
                            if let Some(k) = v["sessions"][0]["key"].as_str() {
                                let id = client.request("chat.history", json!({"sessionKey": k, "limit": 4}));
                                pending.insert(id, "chat.history".into());
                            }
                        }
                        if m == "agents.list" {
                            if let Some(a) = v["agents"][0]["id"].as_str() {
                                let id = client.request("agents.files.list", json!({"agentId": a}));
                                pending.insert(id, "agents.files.list".into());
                            }
                        }
                        let mut o = String::new();
                        outline(&v, 1, &mut o);
                        println!("--- {m}\n{o}");
                    }
                    Err(e) => println!("--- {m}: ERROR {} {}", e.code, e.message),
                }
                if pending.is_empty() && connected {
                    exit = 0;
                    break;
                }
            }
            Ok(NetEvent::Event { event, payload }) => {
                if send_text.is_none() {
                    println!("(evento {event})");
                    continue;
                }
                if payload["sessionKey"].as_str() != send_session.as_deref() {
                    continue;
                }
                if event == "agent" {
                    println!("  agente/{} {}", payload["stream"].as_str().unwrap_or(""), payload["data"]["phase"]);
                }
                if event == "chat" {
                    let state = payload["state"].as_str().unwrap_or("");
                    println!("  chat {state} (+{} letras)", payload["deltaText"].as_str().map_or(0, |t| t.len()));
                    if matches!(state, "final" | "aborted" | "error") {
                        let text = crate::model::message_text(&payload["message"]);
                        println!("respuesta: {text}");
                        if let Some(e) = payload["errorMessage"].as_str() {
                            println!("error: {e}");
                        }
                        let key = send_session.clone().unwrap_or_default();
                        let mut params = json!({"key": key, "archived": true});
                        if let Some(sid) = &send_session_id {
                            params["expectedSessionId"] = json!(sid);
                        }
                        let id = client.request("sessions.patch", params);
                        pending.insert(id, "sessions.patch".into());
                    }
                }
            }
            Err(_) => {}
        }
    }
    client.stop();
    exit
}
