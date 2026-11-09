//! An editor for the whole OpenClaw configuration, drawn from the schema
//! OpenClaw publishes about itself, so every setting (including those added by
//! plugins) appears without being written out one by one here.

use eframe::egui::{self, Color32, RichText, ScrollArea, TextEdit};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

/// What OpenClaw puts in place of a secret when it reads the config back.
pub const REDACTED: &str = "__OPENCLAW_REDACTED__";

pub type Segs = Vec<String>;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Int,
    Num,
    Str,
    Enum(Vec<Value>),
    /// A secret: plain text or a reference to a secret store.
    Secret,
    Object,
    Map(Value),
    Array(Value),
    Any,
}

fn branches(s: &Value) -> Option<&Vec<Value>> {
    s.get("anyOf").or_else(|| s.get("oneOf")).and_then(Value::as_array)
}

fn is_null_branch(s: &Value) -> bool {
    s.get("type").and_then(Value::as_str) == Some("null")
}

/// Works out how a schema node should be edited.
pub fn classify(s: &Value) -> (Kind, Value) {
    if let Some(all) = s.get("allOf").and_then(Value::as_array) {
        if let Some(first) = all.iter().find(|b| b.get("properties").is_some()).or(all.first()) {
            return classify(first);
        }
    }
    if let Some(list) = branches(s) {
        let rest: Vec<&Value> = list.iter().filter(|b| !is_null_branch(b)).collect();
        if rest.len() == 1 {
            return classify(rest[0]);
        }
        let constants: Option<Vec<Value>> = rest
            .iter()
            .map(|b| match b.get("const") {
                Some(c) => Some(vec![c.clone()]),
                None => b.get("enum").and_then(Value::as_array).cloned(),
            })
            .try_fold(vec![], |mut acc, x| {
                acc.extend(x?);
                Some(acc)
            });
        if let Some(values) = constants {
            return (Kind::Enum(values), s.clone());
        }
        let has_string = rest.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("string") && b.get("const").is_none());
        let has_object = rest.iter().any(|b| b.get("oneOf").is_some() || b.get("properties").is_some());
        if has_string && has_object && rest.len() == 2 {
            return (Kind::Secret, s.clone());
        }
        return (Kind::Any, s.clone());
    }
    if let Some(v) = s.get("const") {
        return (Kind::Enum(vec![v.clone()]), s.clone());
    }
    if let Some(v) = s.get("enum").and_then(Value::as_array) {
        return (Kind::Enum(v.clone()), s.clone());
    }
    let ty = match s.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or(""),
        _ => "",
    };
    match ty {
        "boolean" => (Kind::Bool, s.clone()),
        "integer" => (Kind::Int, s.clone()),
        "number" => (Kind::Num, s.clone()),
        "string" => (Kind::Str, s.clone()),
        "array" => (Kind::Array(s.get("items").cloned().unwrap_or(Value::Null)), s.clone()),
        "object" => {
            let has_props = s.get("properties").and_then(Value::as_object).is_some_and(|p| !p.is_empty());
            if has_props {
                (Kind::Object, s.clone())
            } else if let Some(extra) = s.get("additionalProperties").filter(|e| e.is_object()) {
                (Kind::Map(extra.clone()), s.clone())
            } else {
                (Kind::Any, s.clone())
            }
        }
        _ => (Kind::Any, s.clone()),
    }
}

pub fn value_at<'a>(root: &'a Value, segs: &[String]) -> Option<&'a Value> {
    let mut v = root;
    for s in segs {
        v = v.get(s.as_str())?;
    }
    Some(v)
}

/// Turns the edits into the merge patch OpenClaw expects, plus the list of
/// arrays whose entries the person chose to remove or replace.
pub fn build_patch(edits: &BTreeMap<Segs, Value>, current: &Value) -> (Value, Vec<String>) {
    let mut patch = Value::Object(Map::new());
    let mut replace = vec![];
    for (segs, val) in edits {
        let mut cursor = &mut patch;
        for (i, s) in segs.iter().enumerate() {
            if i + 1 == segs.len() {
                if let Value::Object(m) = cursor {
                    m.insert(s.clone(), val.clone());
                }
            } else {
                if !cursor.get(s.as_str()).is_some_and(Value::is_object) {
                    if let Value::Object(m) = cursor {
                        m.insert(s.clone(), json!({}));
                    }
                }
                cursor = cursor.get_mut(s.as_str()).unwrap();
            }
        }
        let was = value_at(current, segs);
        match (val, was) {
            (Value::Array(_), Some(Value::Array(_))) => replace.push(segs.join(".")),
            (Value::Null, Some(w)) => collect_arrays(w, segs, &mut replace),
            _ => {}
        }
    }
    replace.sort();
    replace.dedup();
    (patch, replace)
}

fn collect_arrays(v: &Value, at: &[String], out: &mut Vec<String>) {
    match v {
        Value::Array(_) => out.push(at.join(".")),
        Value::Object(o) => {
            for (k, x) in o {
                let mut next = at.to_vec();
                next.push(k.clone());
                collect_arrays(x, &next, out);
            }
        }
        _ => {}
    }
}

#[derive(Clone)]
pub struct SearchRow {
    pub segs: Segs,
    pub label: String,
    pub help: String,
    pub haystack: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Form,
    File,
}

pub struct ConfigEditor {
    pub schema: Option<Value>,
    pub hints: Value,
    pub current: Value,
    pub base_hash: Option<String>,
    pub path: String,
    pub file_text: String,
    pub file_dirty: bool,
    pub edits: BTreeMap<Segs, Value>,
    buffers: HashMap<Segs, String>,
    new_key: HashMap<Segs, String>,
    pub search: String,
    pub group: Option<String>,
    pub focus: Option<Segs>,
    index: Vec<SearchRow>,
    pub groups: Vec<(String, String)>,
    pub message: Option<(String, bool)>,
    pub loading: bool,
    pub saving: bool,
    pub view: View,
    pub generation: u64,
}

impl Default for ConfigEditor {
    fn default() -> Self {
        Self {
            schema: None,
            hints: Value::Null,
            current: json!({}),
            base_hash: None,
            path: String::new(),
            file_text: String::new(),
            file_dirty: false,
            edits: BTreeMap::new(),
            buffers: HashMap::new(),
            new_key: HashMap::new(),
            search: String::new(),
            group: None,
            focus: None,
            index: vec![],
            groups: vec![],
            message: None,
            loading: false,
            saving: false,
            view: View::Form,
            generation: 0,
        }
    }
}

/// Spanish names for the main groups; the rest use OpenClaw's own label.
fn group_title(key: &str, fallback: &str) -> String {
    let es = match key {
        "gateway" => "Conexión (Gateway)",
        "agents" => "Bots",
        "models" => "Modelos",
        "auth" => "Cuentas de modelos",
        "channels" => "Canales",
        "tools" => "Herramientas",
        "skills" => "Habilidades",
        "plugins" => "Complementos",
        "session" => "Conversaciones",
        "messages" => "Mensajes",
        "memory" => "Memoria",
        "cron" => "Rutinas",
        "hooks" => "Ganchos",
        "approvals" => "Aprobaciones",
        "security" => "Seguridad",
        "secrets" => "Claves",
        "browser" => "Navegador",
        "ui" => "Interfaz",
        "update" => "Actualizaciones",
        "logging" => "Registros",
        "diagnostics" => "Diagnóstico",
        "telemetry" => "Telemetría",
        "talk" => "Voz",
        "tts" => "Lectura en voz alta",
        "attachments" => "Adjuntos",
        "storage" => "Almacenamiento",
        "database" => "Base de datos",
        "env" => "Variables de entorno",
        "wizard" => "Asistente",
        "commands" => "Comandos",
        "discovery" => "Descubrimiento",
        "nodeHost" => "Equipos conectados",
        "bindings" => "Rutas de canales",
        "transcripts" => "Transcripciones",
        "desktop" => "Escritorio",
        "proxy" => "Proxy",
        "mcp" => "Servidores MCP",
        "cloudWorkers" => "Trabajadores en la nube",
        _ => fallback,
    };
    es.to_string()
}

impl ConfigEditor {
    pub fn has_schema(&self) -> bool {
        self.schema.is_some()
    }

    pub fn is_dirty(&self) -> bool {
        !self.edits.is_empty()
    }

    /// Takes in the schema payload (`config.schema`).
    pub fn set_schema(&mut self, payload: &Value) {
        let schema = payload.get("schema").cloned().unwrap_or_else(|| payload.clone());
        self.hints = payload.get("uiHints").cloned().unwrap_or(Value::Null);
        self.groups.clear();
        let mut rows: Vec<(i64, String, String)> = vec![];
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for key in props.keys() {
                if key == "meta" || key == "$schema" {
                    continue;
                }
                let hint = self.hints.get(key.as_str());
                let order = hint.and_then(|h| h.get("order")).and_then(Value::as_i64).unwrap_or(9000);
                let label = hint.and_then(|h| h.get("label")).and_then(Value::as_str).unwrap_or(key);
                rows.push((order, group_title(key, label), key.clone()));
            }
        }
        rows.sort();
        self.groups = rows.into_iter().map(|(_, t, k)| (k, t)).collect();
        if self.group.is_none() {
            self.group = self.groups.first().map(|(k, _)| k.clone());
        }
        let mut index = vec![];
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (k, v) in props {
                if k == "meta" {
                    continue;
                }
                self.index_node(v, vec![k.clone()], k.clone(), 0, &mut index);
            }
        }
        self.index = index;
        self.schema = Some(schema);
    }

    fn index_node(&self, s: &Value, segs: Segs, hint: String, depth: usize, out: &mut Vec<SearchRow>) {
        if out.len() > 40_000 || depth > 12 {
            return;
        }
        let (kind, node) = classify(s);
        let h = self.hints.get(hint.as_str());
        let label = h.and_then(|h| h.get("label")).and_then(Value::as_str)
            .or(node.get("title").and_then(Value::as_str)).unwrap_or("").to_string();
        let help = h.and_then(|h| h.get("help")).and_then(Value::as_str)
            .or(node.get("description").and_then(Value::as_str)).unwrap_or("").to_string();
        let dotted = segs.join(".");
        out.push(SearchRow {
            haystack: format!("{dotted} {label} {help}").to_lowercase(),
            segs: segs.clone(),
            label,
            help,
        });
        match kind {
            Kind::Object => {
                if let Some(props) = node.get("properties").and_then(Value::as_object) {
                    for (k, v) in props {
                        let mut next = segs.clone();
                        next.push(k.clone());
                        self.index_node(v, next, format!("{hint}.{k}"), depth + 1, out);
                    }
                }
            }
            Kind::Map(v) => {
                let mut next = segs.clone();
                next.push("*".into());
                self.index_node(&v, next, format!("{hint}.*"), depth + 1, out);
            }
            _ => {}
        }
    }

    /// Takes in the current config (`config.get`).
    pub fn set_current(&mut self, payload: &Value) {
        self.current = payload
            .get("sourceConfig")
            .or_else(|| payload.get("parsed"))
            .or_else(|| payload.get("config"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        self.base_hash = payload.get("hash").and_then(Value::as_str).map(str::to_string);
        self.path = payload.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        if !self.file_dirty {
            self.file_text = payload.get("raw").and_then(Value::as_str).unwrap_or("").to_string();
        }
        self.buffers.clear();
        self.generation += 1;
    }

    pub fn discard(&mut self) {
        self.edits.clear();
        self.buffers.clear();
        self.new_key.clear();
        self.generation += 1;
    }

    fn effective(&self, segs: &[String]) -> Option<Value> {
        if let Some(v) = self.edits.get(segs) {
            return Some(v.clone()).filter(|v| !v.is_null());
        }
        for cut in (1..segs.len()).rev() {
            if let Some(v) = self.edits.get(&segs[..cut]) {
                return value_at(v, &segs[cut..]).cloned();
            }
        }
        value_at(&self.current, segs).cloned()
    }

    fn set(&mut self, segs: &[String], value: Value) {
        let original = value_at(&self.current, segs);
        if original == Some(&value) {
            self.edits.remove(segs);
        } else {
            self.edits.insert(segs.to_vec(), value);
        }
    }

    fn clear_edit(&mut self, segs: &[String]) {
        self.edits.remove(segs);
        self.buffers.remove(segs);
    }

    /// Removes the setting so OpenClaw goes back to its default.
    fn unset(&mut self, segs: &[String]) {
        self.buffers.remove(segs);
        if value_at(&self.current, segs).is_some() {
            self.edits.insert(segs.to_vec(), Value::Null);
        } else {
            self.edits.remove(segs);
        }
    }

    pub fn search_results(&self, limit: usize) -> Vec<SearchRow> {
        let terms: Vec<String> = self.search.to_lowercase().split_whitespace().map(str::to_string).collect();
        if terms.is_empty() {
            return vec![];
        }
        self.index
            .iter()
            .filter(|r| terms.iter().all(|t| r.haystack.contains(t.as_str())))
            .take(limit)
            .cloned()
            .collect()
    }

    fn hint(&self, key: &str) -> Option<&Value> {
        self.hints.get(key)
    }

    /// The schema node for a path, along with its hint key.
    pub fn node_for(&self, segs: &[String]) -> Option<(Value, String)> {
        let mut node = self.schema.as_ref()?.clone();
        let mut hint = String::new();
        for (i, seg) in segs.iter().enumerate() {
            let (kind, resolved) = classify(&node);
            let (next, part) = match kind {
                Kind::Map(v) => (v, "*".to_string()),
                _ => (resolved.get("properties")?.get(seg.as_str())?.clone(), seg.clone()),
            };
            if i > 0 {
                hint.push('.');
            }
            hint.push_str(&part);
            node = next;
        }
        Some((node, hint))
    }

    /// Draws the form for the selected group or searched setting.
    pub fn form_ui(&mut self, ui: &mut egui::Ui, muted: Color32, accent: Color32, warn: Color32) {
        let (segs, title) = if let Some(f) = self.focus.clone() {
            (f.clone(), f.join(" › "))
        } else if let Some(g) = self.group.clone() {
            let t = self.groups.iter().find(|(k, _)| *k == g).map(|(_, t)| t.clone()).unwrap_or_else(|| g.clone());
            (vec![g], t)
        } else {
            return;
        };
        let cut = segs.iter().position(|s| s == "*").unwrap_or(segs.len());
        let segs: Segs = segs[..cut].to_vec();
        let Some((node, hint)) = self.node_for(&segs) else {
            ui.label("Este ajuste ya no existe en tu OpenClaw.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(18.0).strong());
            if self.focus.is_some() && ui.small_button("Volver").clicked() {
                self.focus = None;
            }
        });
        if let Some(help) = self.hint(&hint).and_then(|h| h.get("help")).and_then(Value::as_str) {
            ui.label(RichText::new(help).size(12.0).color(muted));
        }
        ui.add_space(6.0);
        let (kind, resolved) = classify(&node);
        let generation = self.generation;
        ScrollArea::vertical().id_salt(("config-form", generation)).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if kind == Kind::Object {
                self.object_body(ui, &segs, &hint, &resolved, 0, muted, accent, warn);
            } else {
                self.node_ui(ui, &segs, &hint, &node, 0, muted, accent, warn);
            }
            ui.add_space(30.0);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn object_body(&mut self, ui: &mut egui::Ui, segs: &[String], hint: &str, node: &Value, depth: usize, muted: Color32, accent: Color32, warn: Color32) {
        let Some(props) = node.get("properties").and_then(Value::as_object) else { return };
        let mut keys: Vec<(&String, &Value)> = props.iter().collect();
        // Simple settings first, then the groups that open up.
        keys.sort_by_key(|(k, v)| {
            let (kind, _) = classify(v);
            (matches!(kind, Kind::Object | Kind::Map(_)), (*k).clone())
        });
        for (k, v) in keys {
            let mut next = segs.to_vec();
            next.push(k.clone());
            self.node_ui(ui, &next, &format!("{hint}.{k}"), v, depth, muted, accent, warn);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn node_ui(&mut self, ui: &mut egui::Ui, segs: &[String], hint_key: &str, schema: &Value, depth: usize, muted: Color32, accent: Color32, warn: Color32) {
        if depth > 14 {
            return;
        }
        let (kind, node) = classify(schema);
        let hint = self.hint(hint_key).cloned().unwrap_or(Value::Null);
        let key = segs.last().map(String::as_str).unwrap_or("");
        let label = hint.get("label").and_then(Value::as_str)
            .or(node.get("title").and_then(Value::as_str))
            .unwrap_or(key)
            .to_string();
        let help = hint.get("help").and_then(Value::as_str)
            .or(node.get("description").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let sensitive = hint.get("sensitive").and_then(Value::as_bool).unwrap_or(false);
        let placeholder = hint.get("placeholder").and_then(Value::as_str).unwrap_or("").to_string();
        let default = node.get("default").or_else(|| schema.get("default")).cloned();
        let edited = self.edits.contains_key(segs);
        let current = self.effective(segs);
        let id = ui.scope_id().with(segs);

        match kind {
            Kind::Object | Kind::Map(_) => {
                let count = current.as_ref().and_then(Value::as_object).map(|o| o.len()).unwrap_or(0);
                let mark = if self.edits.keys().any(|k| k.starts_with(segs)) { " •" } else { "" };
                let title = if count > 0 {
                    format!("{label}  ({count}){mark}")
                } else {
                    format!("{label}{mark}")
                };
                let header = egui::CollapsingHeader::new(RichText::new(title).strong())
                    .id_salt(id)
                    .default_open(false)
                    .show(ui, |ui| {
                        if !help.is_empty() {
                            ui.label(RichText::new(&help).size(11.5).color(muted));
                        }
                        match &kind {
                            Kind::Object => self.object_body(ui, segs, hint_key, &node, depth + 1, muted, accent, warn),
                            Kind::Map(v) => self.map_body(ui, segs, hint_key, v, &current, depth, muted, accent, warn),
                            _ => {}
                        }
                    });
                let _ = header;
            }
            Kind::Array(ref items) => {
                let (item_kind, _) = classify(items);
                let simple = matches!(item_kind, Kind::Str | Kind::Int | Kind::Num | Kind::Enum(_));
                self.row_header(ui, segs, &label, &help, edited, current.is_some(), muted, accent);
                if simple {
                    let text = self.buffers.entry(segs.to_vec()).or_insert_with(|| {
                        current.as_ref().and_then(Value::as_array).map(|a| {
                            a.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join("\n")
                        }).unwrap_or_default()
                    });
                    let resp = ui.add(TextEdit::multiline(text).desired_rows(2).desired_width(f32::INFINITY).hint_text("Uno por línea"));
                    if resp.changed() {
                        let numeric = matches!(item_kind, Kind::Int | Kind::Num);
                        let items: Vec<Value> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| {
                            if numeric {
                                l.parse::<f64>().ok().and_then(|n| serde_json::Number::from_f64(n)).map(Value::Number).unwrap_or_else(|| json!(l))
                            } else {
                                json!(l)
                            }
                        }).collect();
                        let v = Value::Array(items);
                        self.set(segs, v);
                    }
                } else {
                    self.json_field(ui, segs, &current, muted, warn);
                }
            }
            Kind::Any => {
                self.row_header(ui, segs, &label, &help, edited, current.is_some(), muted, accent);
                self.json_field(ui, segs, &current, muted, warn);
            }
            Kind::Bool => {
                ui.horizontal_wrapped(|ui| {
                    let mut on = current.as_ref().and_then(Value::as_bool).or(default.as_ref().and_then(Value::as_bool)).unwrap_or(false);
                    if ui.checkbox(&mut on, RichText::new(&label).size(13.0)).on_hover_text(&help).changed() {
                        self.set(segs, json!(on));
                    }
                    self.row_tools(ui, segs, edited, current.is_some(), muted, accent);
                });
                if !help.is_empty() {
                    ui.label(RichText::new(short(&help)).size(11.0).color(muted)).on_hover_text(&help);
                }
            }
            Kind::Enum(ref values) => {
                self.row_header(ui, segs, &label, &help, edited, current.is_some(), muted, accent);
                let shown = current.as_ref().map(show_value).unwrap_or_else(|| match &default {
                    Some(d) => format!("Predeterminado ({})", show_value(d)),
                    None => "Predeterminado".to_string(),
                });
                egui::ComboBox::from_id_salt(id).selected_text(shown).width(320.0).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "Predeterminado").clicked() {
                        self.unset(segs);
                    }
                    for v in values {
                        if ui.selectable_label(current.as_ref() == Some(v), show_value(v)).clicked() {
                            self.set(segs, v.clone());
                        }
                    }
                });
            }
            Kind::Str | Kind::Int | Kind::Num | Kind::Secret => {
                self.row_header(ui, segs, &label, &help, edited, current.is_some(), muted, accent);
                let secret = sensitive || kind == Kind::Secret;
                let is_redacted = current.as_ref().is_some_and(|v| v.as_str() == Some(REDACTED));
                let is_reference = kind == Kind::Secret && current.as_ref().is_some_and(Value::is_object);
                if is_reference {
                    ui.label(RichText::new(format!("Usa una referencia: {}", current.as_ref().map(show_value).unwrap_or_default())).size(12.0).color(muted));
                }
                let text = self.buffers.entry(segs.to_vec()).or_insert_with(|| {
                    if is_redacted || is_reference {
                        String::new()
                    } else {
                        current.as_ref().map(plain_value).unwrap_or_default()
                    }
                });
                let hint_text = if is_redacted {
                    "Guardada. Escribe para cambiarla".to_string()
                } else if !placeholder.is_empty() {
                    placeholder
                } else if let Some(d) = &default {
                    show_value(d)
                } else {
                    String::new()
                };
                let resp = ui.add(TextEdit::singleline(text).password(secret).hint_text(hint_text).desired_width(420.0));
                if resp.changed() {
                    let t = text.trim().to_string();
                    match (&kind, t.is_empty()) {
                        (_, true) if is_redacted => {
                            self.edits.remove(segs);
                        }
                        (_, true) => self.unset_keep_buffer(segs),
                        (Kind::Int, _) => match t.parse::<i64>() {
                            Ok(n) => self.set(segs, json!(n)),
                            Err(_) => {
                                self.edits.remove(segs);
                            }
                        },
                        (Kind::Num, _) => match t.parse::<f64>() {
                            Ok(n) => self.set(segs, json!(n)),
                            Err(_) => {
                                self.edits.remove(segs);
                            }
                        },
                        _ => self.set(segs, json!(t)),
                    }
                }
                if matches!(kind, Kind::Int | Kind::Num) {
                    let bad = !self.buffers.get(segs).is_some_and(|t| t.trim().is_empty() || t.trim().parse::<f64>().is_ok());
                    if bad {
                        ui.label(RichText::new("Escribe un número").size(11.5).color(warn));
                    }
                }
            }
        }
    }

    fn unset_keep_buffer(&mut self, segs: &[String]) {
        if value_at(&self.current, segs).is_some() {
            self.edits.insert(segs.to_vec(), Value::Null);
        } else {
            self.edits.remove(segs);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn map_body(&mut self, ui: &mut egui::Ui, segs: &[String], hint_key: &str, item: &Value, current: &Option<Value>, depth: usize, muted: Color32, accent: Color32, warn: Color32) {
        let keys: Vec<String> = current.as_ref().and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default();
        if keys.is_empty() {
            ui.label(RichText::new("Sin entradas todavía.").size(12.0).color(muted));
        }
        for k in keys {
            let mut next = segs.to_vec();
            next.push(k.clone());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&k).monospace().size(12.5));
                if ui.small_button("Quitar").clicked() {
                    self.unset(&next);
                }
            });
            ui.indent(("map", &k), |ui| {
                self.node_ui(ui, &next, &format!("{hint_key}.*"), item, depth + 1, muted, accent, warn);
            });
        }
        ui.horizontal(|ui| {
            let buf = self.new_key.entry(segs.to_vec()).or_default();
            ui.add(TextEdit::singleline(buf).hint_text("Nombre de la nueva entrada").desired_width(220.0));
            let name = buf.trim().to_string();
            if ui.add_enabled(!name.is_empty(), egui::Button::new("Añadir")).clicked() {
                let mut next = segs.to_vec();
                next.push(name);
                let starter = match classify(item).0 {
                    Kind::Object | Kind::Map(_) | Kind::Any => json!({}),
                    Kind::Array(_) => json!([]),
                    Kind::Bool => json!(false),
                    Kind::Int | Kind::Num => json!(0),
                    _ => json!(""),
                };
                self.edits.insert(next, starter);
                self.new_key.remove(segs);
            }
        });
    }

    fn json_field(&mut self, ui: &mut egui::Ui, segs: &[String], current: &Option<Value>, muted: Color32, warn: Color32) {
        let text = self.buffers.entry(segs.to_vec()).or_insert_with(|| {
            current.as_ref().map(|v| serde_json::to_string_pretty(v).unwrap_or_default()).unwrap_or_default()
        });
        let resp = ui.add(TextEdit::multiline(text).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("Valor en formato JSON"));
        let trimmed = text.trim().to_string();
        let parsed = if trimmed.is_empty() { None } else { json5::from_str::<Value>(&trimmed).ok() };
        if resp.changed() {
            match (&parsed, trimmed.is_empty()) {
                (Some(v), _) => self.set(segs, v.clone()),
                (None, true) => self.unset_keep_buffer(segs),
                (None, false) => {
                    self.edits.remove(segs);
                }
            }
        }
        if !trimmed.is_empty() && parsed.is_none() {
            ui.label(RichText::new("Formato no válido; este valor no se guardará.").size(11.5).color(warn));
        } else {
            let _ = muted;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row_header(&mut self, ui: &mut egui::Ui, segs: &[String], label: &str, help: &str, edited: bool, has_value: bool, muted: Color32, accent: Color32) {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(label).size(13.0)).on_hover_text(help);
            self.row_tools(ui, segs, edited, has_value, muted, accent);
        });
        if !help.is_empty() {
            ui.label(RichText::new(short(help)).size(11.0).color(muted)).on_hover_text(help);
        }
    }

    fn row_tools(&mut self, ui: &mut egui::Ui, segs: &[String], edited: bool, has_value: bool, muted: Color32, accent: Color32) {
        if edited {
            ui.label(RichText::new("cambiado").size(10.5).color(accent));
            if ui.small_button("Deshacer").clicked() {
                self.clear_edit(segs);
            }
        } else if has_value && value_at(&self.current, segs).is_some() {
            let b = egui::Button::new(RichText::new("Quitar").size(10.5).color(muted)).frame(false);
            if ui.add(b).on_hover_text("Volver al valor predeterminado de OpenClaw").clicked() {
                self.unset(segs);
            }
        }
    }
}

fn short(text: &str) -> String {
    let first = text.split_inclusive(". ").next().unwrap_or(text).trim();
    let mut out: String = first.chars().take(180).collect();
    if first.chars().count() > 180 {
        out.push('…');
    }
    out
}

fn plain_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn show_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(s: &str) -> Segs {
        s.split('.').map(str::to_string).collect()
    }

    #[test]
    fn recognises_each_kind_of_setting() {
        let e = json!({"anyOf":[{"type":"string","const":"a"},{"type":"string","const":"b"}]});
        assert_eq!(classify(&e).0, Kind::Enum(vec![json!("a"), json!("b")]));
        assert_eq!(classify(&json!({"type":"boolean"})).0, Kind::Bool);
        let secret = json!({"anyOf":[{"type":"string"},{"oneOf":[{"type":"object","properties":{"source":{}}}]}]});
        assert_eq!(classify(&secret).0, Kind::Secret);
        let opt = json!({"anyOf":[{"type":"null"},{"type":"integer"}]});
        assert_eq!(classify(&opt).0, Kind::Int);
        let map = json!({"type":"object","additionalProperties":{"type":"string"}});
        assert_eq!(classify(&map).0, Kind::Map(json!({"type":"string"})));
    }

    #[test]
    fn patch_nests_values_and_deletes_with_null() {
        let mut edits = BTreeMap::new();
        edits.insert(segs("gateway.port"), json!(19000));
        edits.insert(segs("gateway.auth.mode"), json!("token"));
        edits.insert(segs("tools.profile"), Value::Null);
        let (patch, replace) = build_patch(&edits, &json!({"tools": {"profile": "full"}}));
        assert_eq!(patch, json!({"gateway": {"port": 19000, "auth": {"mode": "token"}}, "tools": {"profile": null}}));
        assert!(replace.is_empty());
    }

    #[test]
    fn replacing_or_deleting_an_array_is_declared() {
        let current = json!({"a": {"list": [1, 2]}, "b": {"inner": {"xs": [1]}, "y": 1}});
        let mut edits = BTreeMap::new();
        edits.insert(segs("a.list"), json!([1]));
        edits.insert(segs("b"), Value::Null);
        let (_, replace) = build_patch(&edits, &current);
        assert_eq!(replace, vec!["a.list".to_string(), "b.inner.xs".to_string()]);
    }

    #[test]
    fn editor_finds_nested_nodes_through_maps() {
        let mut ed = ConfigEditor::default();
        ed.set_schema(&json!({
            "schema": {"type":"object","properties":{
                "agents":{"type":"object","properties":{
                    "entries":{"type":"object","additionalProperties":{"type":"object","properties":{"name":{"type":"string"}}}}
                }}
            }},
            "uiHints": {"agents.entries.*.name": {"label": "Agent Name", "help": "The name"}}
        }));
        let (node, hint) = ed.node_for(&segs("agents.entries.main.name")).unwrap();
        assert_eq!(hint, "agents.entries.*.name");
        assert_eq!(node, json!({"type":"string"}));
        ed.search = "agent name".into();
        let hits = ed.search_results(10);
        assert!(hits.iter().any(|r| r.label == "Agent Name"));
    }

    #[test]
    fn untouched_values_do_not_become_edits() {
        let mut ed = ConfigEditor::default();
        ed.current = json!({"gateway": {"port": 18789}});
        ed.set(&segs("gateway.port"), json!(18789));
        assert!(!ed.is_dirty());
        ed.set(&segs("gateway.port"), json!(1));
        assert!(ed.is_dirty());
    }
}
