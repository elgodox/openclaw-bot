//! What the app shows, read out of the gateway's JSON.

use serde_json::Value;

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn first<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str).filter(|t| !t.trim().is_empty()))
}

#[derive(Clone, Debug, Default)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub emoji: String,
    pub model: String,
    pub workspace: String,
}

impl Agent {
    pub fn from_json(v: &Value) -> Option<Self> {
        if v.get("kind").and_then(Value::as_str) == Some("system") {
            return None;
        }
        let id = s(v, "id")?;
        let identity = v.get("identity").cloned().unwrap_or(Value::Null);
        let name = first(&identity, &["name"])
            .or(first(v, &["name"]))
            .unwrap_or(&id)
            .to_string();
        let emoji = first(&identity, &["emoji"]).unwrap_or("").to_string();
        let model = match v.get("model") {
            Some(Value::String(m)) => m.clone(),
            Some(Value::Object(o)) => {
                let primary = o.get("primary").and_then(Value::as_str).unwrap_or("");
                if primary.is_empty() {
                    o.get("id").and_then(Value::as_str).unwrap_or("").to_string()
                } else {
                    primary.to_string()
                }
            }
            _ => String::new(),
        };
        Some(Self {
            id,
            name,
            emoji,
            model,
            workspace: s(v, "workspace").unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SessionRow {
    pub key: String,
    pub agent_id: String,
    pub title: String,
    pub is_main: bool,
    pub active: bool,
    pub archived: bool,
    pub unread: bool,
    pub updated_ms: i64,
    pub session_id: String,
    pub model: String,
}

impl SessionRow {
    pub fn from_json(v: &Value) -> Option<Self> {
        let key = s(v, "key")?;
        // Background work and sub-agents don't belong in the chat list.
        if v.get("isBackground").and_then(Value::as_bool) == Some(true)
            || v.get("isDock").and_then(Value::as_bool) == Some(true)
            || v.get("spawnDepth").and_then(Value::as_i64).unwrap_or(0) > 0
        {
            return None;
        }
        let agent_id = s(v, "agentId")
            .or_else(|| key.strip_prefix("agent:")?.split(':').next().map(str::to_string))
            .unwrap_or_default();
        let title = first(v, &["displayName", "label", "derivedTitle", "lastMessagePreview"])
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        Some(Self {
            active: v.get("hasActiveRun").and_then(Value::as_bool).unwrap_or(false),
            archived: v.get("archived").and_then(Value::as_bool).unwrap_or(false),
            unread: v.get("unread").and_then(Value::as_bool).unwrap_or(false),
            is_main: v.get("isMain").and_then(Value::as_bool).unwrap_or(false),
            updated_ms: v
                .get("lastInteractionAt")
                .and_then(Value::as_i64)
                .or(v.get("updatedAt").and_then(Value::as_i64))
                .unwrap_or(0),
            session_id: s(v, "sessionId").unwrap_or_default(),
            model: s(v, "model").unwrap_or_default(),
            key,
            agent_id,
            title,
        })
    }

    pub fn shown_title(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        if self.is_main {
            "Conversación principal".into()
        } else {
            "Chat sin título".into()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
    Notice,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    pub attachments: Vec<String>,
    pub at_ms: i64,
    /// Sent but not yet confirmed.
    pub pending: bool,
    pub failed: bool,
}

/// All the visible text of a transcript message.
pub fn message_text(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for p in parts {
                if p.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(t) = p.get("text").and_then(Value::as_str) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
            }
            out
        }
        _ => msg
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn message_attachments(msg: &Value) -> Vec<String> {
    let mut out = vec![];
    if let Some(parts) = msg.get("content").and_then(Value::as_array) {
        for p in parts {
            match p.get("type").and_then(Value::as_str) {
                Some("image") => out.push(
                    first(p, &["alt", "fileName", "name"]).unwrap_or("imagen").to_string(),
                ),
                Some("file") | Some("attachment") | Some("audio") | Some("video") => out.push(
                    first(p, &["fileName", "name", "alt"]).unwrap_or("archivo").to_string(),
                ),
                _ => {}
            }
        }
    }
    out
}

/// Turns a chat.history answer into rows for the screen.
pub fn parse_history(v: &Value) -> Vec<ChatMessage> {
    let mut out = vec![];
    for m in v.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let role = match m.get("role").and_then(Value::as_str) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        let text = message_text(m);
        let attachments = message_attachments(m);
        let kind = m.pointer("/__openclaw/kind").and_then(Value::as_str);
        let (role, text) = match kind {
            Some("compaction") => (Role::Notice, "La conversación se resumió para ahorrar espacio.".to_string()),
            Some("reset") => (Role::Notice, "La conversación se reinició aquí.".to_string()),
            _ => (role, text),
        };
        if text.trim().is_empty() && attachments.is_empty() {
            continue;
        }
        out.push(ChatMessage {
            role,
            text,
            attachments,
            at_ms: m.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
            pending: false,
            failed: false,
        });
    }
    out
}

#[derive(Clone, Debug)]
pub struct ToolStep {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub running: bool,
    pub failed: bool,
}

/// One line about a tool, from a live "agent" event.
pub fn tool_step(data: &Value) -> Option<(String, String, String, String)> {
    let phase = first(data, &["phase"])?.to_string();
    let id = first(data, &["toolCallId", "id", "callId"]).unwrap_or("").to_string();
    let name = first(data, &["name", "toolName", "tool"]).unwrap_or("herramienta").to_string();
    let detail = tool_summary(data.get("args").or(data.get("input")));
    Some((phase, id, name, detail))
}

fn tool_summary(args: Option<&Value>) -> String {
    let Some(args) = args else { return String::new() };
    if let Some(t) = args.as_str() {
        return shorten(t, 120);
    }
    for key in ["command", "path", "file", "url", "query", "pattern", "description"] {
        if let Some(v) = args.get(key).and_then(Value::as_str) {
            return shorten(v, 120);
        }
    }
    String::new()
}

pub fn shorten(t: &str, max: usize) -> String {
    let flat: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        flat.chars().take(max).collect::<String>() + "…"
    }
}

#[derive(Clone, Debug)]
pub struct Approval {
    pub id: String,
    pub plugin: bool,
    pub title: String,
    pub detail: String,
    pub cwd: String,
    pub agent_id: String,
    pub expires_ms: i64,
}

impl Approval {
    pub fn from_event(v: &Value, plugin: bool) -> Option<Self> {
        let id = s(v, "id")?;
        let req = v.get("request").cloned().unwrap_or(Value::Null);
        let (title, detail) = if plugin {
            (
                first(&req, &["title"]).unwrap_or("Un complemento pide permiso").to_string(),
                first(&req, &["description"]).unwrap_or("").to_string(),
            )
        } else {
            (
                "Quiere ejecutar un comando".to_string(),
                first(&req, &["commandPreview", "command", "rawCommand"])
                    .unwrap_or("")
                    .to_string(),
            )
        };
        Some(Self {
            id,
            plugin,
            title,
            detail,
            cwd: first(&req, &["cwd"]).unwrap_or("").to_string(),
            agent_id: first(&req, &["agentId"]).unwrap_or("").to_string(),
            expires_ms: v.get("expiresAtMs").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub agent_id: String,
    pub enabled: bool,
    pub schedule: String,
    pub last: String,
    pub revision: String,
}

impl Routine {
    pub fn from_json(v: &Value) -> Option<Self> {
        let id = s(v, "id")?;
        let name = first(v, &["displayName", "name"]).unwrap_or(&id).to_string();
        let sch = v.get("schedule").cloned().unwrap_or(Value::Null);
        let schedule = match first(&sch, &["kind"]).unwrap_or("") {
            "cron" => first(&sch, &["expr"]).unwrap_or("").to_string(),
            "every" => {
                let ms = sch.get("everyMs").and_then(Value::as_i64).unwrap_or(0);
                format!("cada {}", friendly_duration(ms))
            }
            "at" => format!("una vez: {}", first(&sch, &["at"]).unwrap_or("")),
            _ => String::new(),
        };
        let last = match first(v, &["lastRunStatus"]).or(first(
            v.get("state").unwrap_or(&Value::Null),
            &["lastStatus", "lastRunStatus"],
        )) {
            Some("ok") => "Última vez: bien".to_string(),
            Some("error") => "Última vez: falló".to_string(),
            Some("skipped") => "Última vez: omitida".to_string(),
            _ => "Aún no se ejecutó".to_string(),
        };
        Some(Self {
            agent_id: first(v, &["effectiveAgentId", "agentId"]).unwrap_or("").to_string(),
            enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            revision: s(v, "configRevision").unwrap_or_default(),
            id,
            name,
            schedule,
            last,
        })
    }
}

pub fn friendly_duration(ms: i64) -> String {
    let m = ms / 60_000;
    if m < 1 {
        format!("{} s", ms / 1000)
    } else if m < 60 {
        format!("{m} min")
    } else if m < 60 * 24 {
        format!("{} h", m / 60)
    } else {
        format!("{} días", m / (60 * 24))
    }
}

#[derive(Clone, Debug)]
pub struct Skill {
    pub key: String,
    pub name: String,
    pub emoji: String,
    pub description: String,
    pub enabled: bool,
    pub eligible: bool,
    pub missing: String,
}

impl Skill {
    pub fn from_json(v: &Value) -> Option<Self> {
        let key = s(v, "skillKey").or_else(|| s(v, "name"))?;
        let mut missing = vec![];
        if let Some(m) = v.get("missing").and_then(Value::as_object) {
            for (what, items) in m {
                if let Some(a) = items.as_array() {
                    if !a.is_empty() {
                        missing.push(format!(
                            "{what}: {}",
                            a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                        ));
                    }
                }
            }
        }
        Some(Self {
            name: s(v, "name").unwrap_or_else(|| key.clone()),
            emoji: s(v, "emoji").unwrap_or_default(),
            description: s(v, "description").unwrap_or_default(),
            enabled: !v.get("disabled").and_then(Value::as_bool).unwrap_or(false),
            eligible: v.get("eligible").and_then(Value::as_bool).unwrap_or(false),
            missing: missing.join(" · "),
            key,
        })
    }
}

#[derive(Clone, Debug)]
pub struct BotFile {
    pub name: String,
    pub missing: bool,
    pub size: i64,
}

impl BotFile {
    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            name: s(v, "name")?,
            missing: v.get("missing").and_then(Value::as_bool).unwrap_or(false),
            size: v.get("size").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_text_parts() {
        let m = json!({"role":"assistant","content":[{"type":"text","text":"hola"},{"type":"toolCall"},{"type":"text","text":"mundo"}]});
        assert_eq!(message_text(&m), "hola\nmundo");
    }

    #[test]
    fn hides_background_sessions() {
        assert!(SessionRow::from_json(&json!({"key":"agent:a:cron:1","isBackground":true})).is_none());
        let r = SessionRow::from_json(&json!({"key":"agent:a:chat-1","displayName":"Nota"})).unwrap();
        assert_eq!(r.agent_id, "a");
        assert_eq!(r.shown_title(), "Nota");
    }

    #[test]
    fn history_keeps_user_and_assistant_only() {
        let h = json!({"messages":[
            {"role":"user","content":"hi","timestamp":1},
            {"role":"toolResult","content":"x"},
            {"role":"assistant","content":[{"type":"text","text":"ok"}],"timestamp":2}]});
        let rows = parse_history(&h);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text, "ok");
    }

    #[test]
    fn reads_tool_events() {
        let (phase, id, name, detail) =
            tool_step(&json!({"phase":"start","toolCallId":"t1","name":"read","args":{"path":"a.txt"}})).unwrap();
        assert_eq!((phase.as_str(), id.as_str(), name.as_str(), detail.as_str()), ("start", "t1", "read", "a.txt"));
    }
}
