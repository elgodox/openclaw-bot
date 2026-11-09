//! The screens of the administration side.

use crate::app::{time_label, App};
use crate::cli::JobState;
use crate::config_edit::View;
use crate::look::Palette;
use crate::manager::{Confirm, Section};
use crate::model::shorten;
use eframe::egui::{self, Align, Button, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, TextEdit, Vec2};
use serde_json::{json, Value};
use std::rc::Rc;

fn r(n: u8) -> CornerRadius {
    CornerRadius::same(n)
}

fn data(app: &App, key: &str) -> Rc<Value> {
    app.mgr.data.get(key).cloned().unwrap_or_else(|| Rc::new(Value::Null))
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn card<R>(ui: &mut egui::Ui, p: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(r(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(14.5).strong());
    ui.add_space(2.0);
}

fn muted(ui: &mut egui::Ui, p: &Palette, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(12.0).color(p.text2));
}

fn kv(ui: &mut egui::Ui, p: &Palette, k: &str, v: impl Into<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(k).size(12.5).color(p.text2));
        ui.label(RichText::new(v.into()).size(12.5));
    });
}

fn primary(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(Button::new(RichText::new(text).color(Color32::WHITE)).fill(p.bubble).corner_radius(r(8)).min_size(Vec2::new(0.0, 30.0)))
}

fn danger(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(Button::new(RichText::new(text).color(p.red)))
}

fn dot(ui: &mut egui::Ui, c: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, c);
}

fn empty(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(12.5).color(p.text3));
}

fn error_note(app: &App, ui: &mut egui::Ui, p: &Palette, key: &str) {
    if let Some(e) = app.mgr.errors.get(key) {
        ui.label(RichText::new(e).size(12.0).color(p.amber));
    }
}

fn gib(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else {
        format!("{:.0} MB", bytes / 1024.0 / 1024.0)
    }
}

fn human_duration(ms: u64) -> String {
    let m = ms / 60_000;
    if m < 60 {
        format!("{m} min")
    } else if m < 60 * 48 {
        format!("{} h {} min", m / 60, m % 60)
    } else {
        format!("{} días", m / 60 / 24)
    }
}

fn confirm(app: &mut App, title: &str, body: &str, button: &str, run: impl FnOnce(&mut App, &egui::Context) + 'static) {
    app.mgr.confirm = Some(Confirm {
        title: title.to_string(),
        body: body.to_string(),
        button: button.to_string(),
        run: Box::new(run),
    });
}

// ---- navigation -------------------------------------------------------------

pub const RAIL_WIDTH: f32 = 78.0;

pub fn rail(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let pending_devices = data(app, "pairing")["pending"].as_array().map(|a| a.len()).unwrap_or(0);
    let pending_dm = data(app, "pairing_requests")["requests"].as_array().map(|a| a.len()).unwrap_or(0);
    let ctx = ui.ctx().clone();
    ScrollArea::vertical().auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for section in Section::ALL {
            let selected = app.mgr.section == section;
            let badge = match section {
                Section::Devices => pending_devices,
                Section::Channels => pending_dm,
                Section::Approvals => app.approvals.len(),
                _ => 0,
            };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(RAIL_WIDTH - 8.0, 44.0), Sense::click());
            let fill = if selected {
                p.selected
            } else if resp.hovered() {
                p.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, r(8), fill);
            let color = if selected { p.text } else { p.text2 };
            ui.painter().text(
                rect.center_top() + Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                section.glyph(),
                egui::FontId::proportional(17.0),
                color,
            );
            ui.painter().text(
                rect.center_bottom() - Vec2::new(0.0, 10.0),
                egui::Align2::CENTER_CENTER,
                section.title(),
                egui::FontId::proportional(10.5),
                color,
            );
            if badge > 0 {
                let at = rect.right_top() + Vec2::new(-14.0, 11.0);
                ui.painter().circle_filled(at, 8.0, p.red);
                ui.painter().text(at, egui::Align2::CENTER_CENTER, badge.to_string(), egui::FontId::proportional(10.0), Color32::WHITE);
            }
            if resp.clicked() && !selected {
                app.open_section(section, &ctx);
            }
        }
    });
}

fn page_header(app: &mut App, ui: &mut egui::Ui, p: &Palette, text: &str, sub: &str) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(text).size(21.0).strong());
            if !sub.is_empty() {
                ui.label(RichText::new(sub).size(12.5).color(p.text2));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if app.is_connected() && ui.button("↻  Actualizar").clicked() {
                app.load_section(&ctx);
            }
        });
    });
    ui.add_space(10.0);
}

/// Shown instead of a section that needs a live OpenClaw.
fn offline(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    let reason = app.disconnected_reason().unwrap_or_default();
    card(ui, p, |ui| {
        title(ui, "OpenClaw no está conectado");
        muted(ui, p, reason);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if app.cli_host().is_some() && primary(ui, p, "Iniciar OpenClaw").clicked() {
                app.run_cli("Iniciar OpenClaw", &["gateway", "start"], &ctx);
                app.mgr.section = Section::Maintenance;
            }
            if ui.button("Volver a intentar").clicked() {
                app.connect(&ctx);
            }
            if ui.button("Conexiones…").clicked() {
                app.mgr.section = Section::Config;
                app.mgr.forms.config_tab = 2;
            }
        });
    });
}

pub fn central(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let section = app.mgr.section;
    let works_offline = matches!(section, Section::Maintenance | Section::Console | Section::Config | Section::Fleet);
    Frame::new().fill(p.bg).inner_margin(Margin::symmetric(26, 20)).show(ui, |ui| {
        ui.set_min_height(ui.available_height());
        if !app.is_connected() && !works_offline {
            page_header(app, ui, p, section.title(), "");
            offline(app, ui, p);
            return;
        }
        match section {
            Section::Chats => {}
            Section::Overview => overview(app, ui, p),
            Section::Bots => bots(app, ui, p),
            Section::Models => models(app, ui, p),
            Section::Channels => channels(app, ui, p),
            Section::Plugins => plugins(app, ui, p),
            Section::Skills => skills(app, ui, p),
            Section::Routines => routines(app, ui, p),
            Section::Devices => devices(app, ui, p),
            Section::Fleet => fleet(app, ui, p),
            Section::Approvals => approvals(app, ui, p),
            Section::Secrets => secrets(app, ui, p),
            Section::Usage => usage(app, ui, p),
            Section::Logs => logs(app, ui, p),
            Section::Maintenance => maintenance(app, ui, p),
            Section::Config => config(app, ui, p),
            Section::Console => console(app, ui, p),
        }
    });
}

// ---- overview ---------------------------------------------------------------

fn stat(ui: &mut egui::Ui, p: &Palette, width: f32, label: &str, value: &str, note: &str) {
    Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.border)).corner_radius(r(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(width);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(label).size(11.5).color(p.text2));
            ui.label(RichText::new(value).size(21.0).strong());
            ui.label(RichText::new(note).size(11.0).color(p.text3));
        });
    });
}

fn overview(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    page_header(app, ui, p, "Resumen", "Cómo está tu OpenClaw ahora mismo");
    let (health, status, system, update, channels_v, nodes, pairing, cost) = (
        data(app, "health"),
        data(app, "status"),
        data(app, "system"),
        data(app, "update"),
        data(app, "channels"),
        data(app, "nodes"),
        data(app, "pairing"),
        data(app, "usage_cost"),
    );
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let ok = health["ok"].as_bool().unwrap_or(false);
        let accounts: usize = channels_v["channelAccounts"].as_object().map(|o| o.values().filter_map(Value::as_array).map(|a| a.len()).sum()).unwrap_or(0);
        let running: usize = channels_v["channelAccounts"].as_object().map(|o| {
            o.values().filter_map(Value::as_array).flatten().filter(|a| a["running"].as_bool() == Some(true) || a["connected"].as_bool() == Some(true)).count()
        }).unwrap_or(0);
        let online_nodes = nodes["nodes"].as_array().map(|a| a.iter().filter(|n| n["connected"].as_bool() == Some(true)).count()).unwrap_or(0);
        let total_nodes = nodes["nodes"].as_array().map(|a| a.len()).unwrap_or(0);
        let sessions = status["sessions"]["count"].as_u64().unwrap_or(0);
        let width = ((ui.available_width() - 3.0 * 12.0) / 4.0 - 26.0).max(120.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(12.0, 12.0);
            stat(ui, p, width, "Estado", if ok { "En línea" } else { "Revisar" }, app.hello.as_ref().map(|h| h.version.as_str()).unwrap_or(""));
            stat(ui, p, width, "Bots", &app.agents.len().to_string(), &format!("{sessions} conversaciones"));
            stat(ui, p, width, "Canales", &format!("{running}/{accounts}"), "activos");
            stat(ui, p, width, "Equipos", &format!("{online_nodes}/{total_nodes}"), "en línea");
        });
        ui.add_space(12.0);

        let pending_d = pairing["pending"].as_array().map(|a| a.len()).unwrap_or(0);
        let pending_a = app.approvals.len();
        if pending_d + pending_a > 0 {
            card(ui, p, |ui| {
                title(ui, "Necesita tu atención");
                ui.horizontal(|ui| {
                    if pending_d > 0 && ui.button(format!("{pending_d} equipos por aprobar")).clicked() {
                        app.open_section(Section::Devices, &ctx);
                    }
                    if pending_a > 0 && ui.button(format!("{pending_a} permisos por responder")).clicked() {
                        app.open_section(Section::Approvals, &ctx);
                    }
                });
            });
            ui.add_space(12.0);
        }

        ui.columns(2, |cols| {
            card(&mut cols[0], p, |ui| {
                title(ui, "Este OpenClaw");
                kv(ui, p, "Equipo", s(&system, "machineName").to_string());
                kv(ui, p, "Sistema", s(&system, "osLabel").to_string());
                if let Some(u) = system["uptimeMs"].as_u64() {
                    kv(ui, p, "Encendido hace", human_duration(u));
                }
                if let (Some(t), Some(f)) = (system["memoryTotalBytes"].as_f64(), system["memoryFreeBytes"].as_f64()) {
                    kv(ui, p, "Memoria libre", format!("{} de {}", gib(f), gib(t)));
                }
                if let (Some(t), Some(a)) = (system["diskTotalBytes"].as_f64(), system["diskAvailableBytes"].as_f64()) {
                    kv(ui, p, "Disco libre", format!("{} de {}", gib(a), gib(t)));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Reiniciar OpenClaw").clicked() {
                        confirm(app, "Reiniciar OpenClaw", "Se cerrará un momento y volverá solo. Las respuestas en curso pueden interrumpirse.", "Reiniciar", |a, _| a.restart_gateway());
                    }
                    if app.cli_host().is_some() && ui.button("Abrir panel web").clicked() {
                        app.run_cli("Abrir el panel web", &["dashboard"], &ctx);
                    }
                });
            });
            card(&mut cols[1], p, |ui| {
                title(ui, "Actualizaciones y gastos");
                match update["updateAvailable"].as_object() {
                    Some(u) => {
                        let v = u.get("latestVersion").and_then(Value::as_str).unwrap_or("nueva");
                        kv(ui, p, "Disponible", v.to_string());
                        if primary(ui, p, "Actualizar ahora").clicked() {
                            app.open_section(Section::Maintenance, &ctx);
                        }
                    }
                    None => kv(ui, p, "Versión", "Al día"),
                }
                kv(ui, p, "Canal", s(&update, "effectiveChannel").to_string());
                ui.add_space(4.0);
                let total = cost["totals"]["totalCost"].as_f64().unwrap_or(0.0);
                let tokens = cost["totals"]["totalTokens"].as_u64().unwrap_or(0);
                kv(ui, p, "Últimos 7 días", format!("{} tokens · US$ {total:.2}", group_digits(tokens)));
            });
        });
    });
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

// ---- bots ---------------------------------------------------------------------

fn bots(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Bots", "Crea, cambia y borra tus Bots y sus conversaciones");
    ui.horizontal(|ui| {
        if primary(ui, p, "+  Nuevo Bot").clicked() {
            app.new_bot.open = true;
            app.new_bot.error = None;
        }
    });
    ui.add_space(8.0);
    let agents = app.agents.clone();
    let sessions = app.sessions.clone();
    let models = app.models.clone();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for a in &agents {
            card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&a.name).size(14.5).strong());
                        muted(ui, p, format!("{} · {}", a.id, shorten(&a.workspace, 60)));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if danger(ui, p, "Eliminar").clicked() {
                            app.delete_bot(&a.id);
                        }
                        if ui.button("Renombrar").clicked() {
                            app.rename_bot = Some((a.id.clone(), a.name.clone()));
                        }
                        if ui.button("Abrir chat").clicked() {
                            app.select_agent(&a.id);
                            app.mgr.section = Section::Chats;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Modelo").size(12.5).color(p.text2));
                    let shown = if a.model.is_empty() { "El predeterminado".to_string() } else { a.model.clone() };
                    egui::ComboBox::from_id_salt(("bot-model", &a.id)).selected_text(shorten(&shown, 42)).width(300.0).show_ui(ui, |ui| {
                        if ui.selectable_label(a.model.is_empty(), "El predeterminado").clicked() {
                            app.update_bot_model(&a.id, None);
                        }
                        for m in &models {
                            if ui.selectable_label(&a.model == m, m).clicked() {
                                app.update_bot_model(&a.id, Some(m));
                            }
                        }
                    });
                });
                let own: Vec<_> = sessions.iter().filter(|s| s.agent_id == a.id).collect();
                egui::CollapsingHeader::new(RichText::new(format!("{} conversaciones", own.len())).size(12.5))
                    .id_salt(("bot-chats", &a.id))
                    .show(ui, |ui| {
                        for row in own.iter().take(40) {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(shorten(&row.title, 56)).size(12.5));
                                ui.label(RichText::new(time_label(row.updated_ms)).size(11.0).color(p.text3));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let key = row.key.clone();
                                    if ui.small_button("Archivar").clicked() {
                                        app.archive_chat(&key);
                                    }
                                    let k2 = key.clone();
                                    if ui.small_button("Compactar").on_hover_text("Resume lo antiguo para gastar menos").clicked() {
                                        app.act("sessions.compact", json!({"key": k2}), "Conversación compactada");
                                    }
                                    let k3 = key.clone();
                                    if ui.small_button("Vaciar").clicked() {
                                        confirm(app, "Vaciar conversación", "Empieza de nuevo y deja atrás lo hablado.", "Vaciar", move |a, _| {
                                            a.act("sessions.reset", json!({"key": k3}), "Conversación vaciada");
                                        });
                                    }
                                });
                            });
                        }
                    });
            });
            ui.add_space(8.0);
        }
    });
}

// ---- models -------------------------------------------------------------------

fn models(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Modelos", "Qué inteligencia usan tus Bots y con qué cuenta");
    let (auth, catalog) = (data(app, "auth"), data(app, "catalog"));
    let current = app.default_model();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        card(ui, p, |ui| {
            title(ui, "Modelo principal");
            muted(ui, p, "El que usan todos los Bots que no tengan uno propio.");
            ui.add_space(4.0);
            ui.label(RichText::new(if current.is_empty() { "Sin elegir".to_string() } else { current.clone() }).size(14.0).strong());
            ui.add_space(4.0);
            ui.add(TextEdit::singleline(&mut app.mgr.forms.model_search).hint_text("Buscar en el catálogo").desired_width(320.0));
            error_note(app, ui, p, "catalog");
            let needle = app.mgr.forms.model_search.to_lowercase();
            let rows: Vec<&Value> = catalog["models"].as_array().map(|a| {
                let mut v: Vec<&Value> = a.iter().filter(|m| {
                    needle.is_empty() || s(m, "id").to_lowercase().contains(&needle) || s(m, "name").to_lowercase().contains(&needle) || s(m, "provider").to_lowercase().contains(&needle)
                }).collect();
                v.sort_by_key(|m| (m["recommended"].as_bool() != Some(true), m["available"].as_bool() == Some(false)));
                v
            }).unwrap_or_default();
            ui.add_space(4.0);
            ScrollArea::vertical().id_salt("catalog").max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                for m in rows.iter().take(150) {
                    let id = s(m, "id");
                    let provider = s(m, "provider");
                    let full = if id.starts_with(&format!("{provider}/")) || provider.is_empty() { id.to_string() } else { format!("{provider}/{id}") };
                    ui.horizontal(|ui| {
                        let usable = m["available"].as_bool() != Some(false);
                        let name = if s(m, "name").is_empty() { id } else { s(m, "name") };
                        ui.label(RichText::new(shorten(name, 40)).size(12.5).color(if usable { p.text } else { p.text3 }));
                        ui.label(RichText::new(shorten(&full, 44)).size(11.0).color(p.text3));
                        if m["recommended"].as_bool() == Some(true) {
                            ui.label(RichText::new("recomendado").size(10.5).color(p.accent));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if full == current {
                                ui.label(RichText::new("en uso").size(11.0).color(p.green));
                            } else if ui.small_button("Usar").clicked() {
                                app.set_default_model(&full);
                            }
                        });
                    });
                }
                if rows.is_empty() {
                    empty(ui, p, "No hay modelos que mostrar. Agrega una cuenta abajo y actualiza.");
                }
            });
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            title(ui, "Cuentas y claves");
            error_note(app, ui, p, "auth");
            let providers = auth["providers"].as_array().cloned().unwrap_or_default();
            if providers.is_empty() {
                empty(ui, p, "Todavía no hay ninguna cuenta guardada.");
            }
            for row in &providers {
                let id = s(row, "provider").to_string();
                ui.horizontal(|ui| {
                    dot(ui, if s(row, "status") == "ok" || row["ok"].as_bool() == Some(true) { p.green } else { p.amber });
                    ui.label(RichText::new(&id).strong());
                    let profiles = row["profiles"].as_array().map(|a| a.len()).unwrap_or(0);
                    muted(ui, p, format!("{profiles} cuenta(s) {}", s(row, "status")));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let id2 = id.clone();
                        if danger(ui, p, "Desconectar").clicked() {
                            confirm(app, "Desconectar cuenta", "Se borrarán las claves guardadas de este proveedor.", "Desconectar", move |a, _| a.logout_provider(&id2));
                        }
                        if ui.button("Probar").clicked() {
                            app.probe_provider(&id);
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Agregar una clave").size(13.0));
            let caps = auth["providerCapabilities"].as_array().cloned().unwrap_or_default();
            ui.horizontal(|ui| {
                let label = if app.mgr.forms.provider.is_empty() { "Proveedor".to_string() } else { app.mgr.forms.provider.clone() };
                egui::ComboBox::from_id_salt("provider").selected_text(label).width(180.0).show_ui(ui, |ui| {
                    for c in caps.iter().filter(|c| c["apiKeySupported"].as_bool() == Some(true)) {
                        let name = s(c, "provider").to_string();
                        ui.selectable_value(&mut app.mgr.forms.provider, name.clone(), name);
                    }
                });
                ui.add(TextEdit::singleline(&mut app.mgr.forms.api_key).password(true).hint_text("Pega la clave").desired_width(300.0));
                if primary(ui, p, "Guardar").clicked() {
                    app.save_api_key();
                }
            });
            ui.add_space(8.0);
            muted(ui, p, "Para entrar con tu cuenta (sin clave), usa el asistente de OpenClaw.");
            if ui.button("Abrir asistente de OpenClaw").clicked() {
                app.start_wizard("Asistente de OpenClaw", json!({"flow": "setup"}));
            }
        });
    });
}

// ---- channels -----------------------------------------------------------------

const CHANNELS: &[&str] = &[
    "telegram", "whatsapp", "discord", "slack", "signal", "imessage", "googlechat", "msteams", "matrix", "mattermost",
    "irc", "line", "feishu", "nostr", "nextcloud-talk", "synology-chat", "twitch", "x", "zalo", "zalouser", "sms", "tlon",
    "buzz", "raft", "a2a", "clickclack", "reef",
];

fn channels(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Canales", "Por dónde te hablan tus Bots: Telegram, WhatsApp, Discord y más");
    let status = data(app, "channels");
    let pairing = data(app, "pairing_requests");
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        card(ui, p, |ui| {
            title(ui, "Añadir un canal");
            muted(ui, p, "OpenClaw te guía paso a paso y pide lo necesario para cada canal.");
            ui.horizontal_wrapped(|ui| {
                for c in CHANNELS {
                    if ui.button(*c).clicked() {
                        app.start_wizard(&format!("Añadir {c}"), json!({"flow": "channels", "channel": c}));
                    }
                }
            });
        });
        ui.add_space(10.0);
        let requests = pairing["requests"].as_array().cloned().unwrap_or_default();
        if !requests.is_empty() {
            card(ui, p, |ui| {
                title(ui, "Personas pidiendo hablar con tus Bots");
                for q in &requests {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(s(q, "senderLabel")).strong());
                        muted(ui, p, format!("{} · {}", s(q, "channelLabel"), s(q, "accountId")));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let args = json!({"channel": q["channel"], "accountId": q["accountId"], "requestId": q["requestId"]});
                            let a2 = args.clone();
                            if ui.button("Rechazar").clicked() {
                                app.act("channels.pairing.dismiss", a2, "Solicitud rechazada");
                            }
                            if primary(ui, p, "Aprobar").clicked() {
                                app.act("channels.pairing.approve", args, "Persona aprobada");
                            }
                        });
                    });
                }
            });
            ui.add_space(10.0);
        }
        error_note(app, ui, p, "channels");
        let mut any = false;
        if let Some(map) = status["channelAccounts"].as_object() {
            let labels = &status["channelLabels"];
            for (channel, accounts) in map {
                let Some(list) = accounts.as_array() else { continue };
                if list.is_empty() {
                    continue;
                }
                any = true;
                card(ui, p, |ui| {
                    title(ui, labels[channel].as_str().unwrap_or(channel));
                    for a in list {
                        let id = s(a, "accountId").to_string();
                        let running = a["running"].as_bool() == Some(true) || a["connected"].as_bool() == Some(true);
                        ui.horizontal(|ui| {
                            dot(ui, if running { p.green } else if a["lastError"].is_string() { p.red } else { p.amber });
                            ui.label(RichText::new(if s(a, "name").is_empty() { &id } else { s(a, "name") }).strong());
                            let state = if running { "Conectado" } else if a["configured"].as_bool() == Some(false) { "Sin configurar" } else { "Detenido" };
                            muted(ui, p, state);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let target = json!({"channel": channel, "accountId": id});
                                let t2 = target.clone();
                                if danger(ui, p, "Cerrar sesión").clicked() {
                                    confirm(app, "Cerrar sesión del canal", "Tendrás que volver a vincularlo para usarlo.", "Cerrar sesión", move |a, _| {
                                        a.act("channels.logout", t2, "Sesión cerrada");
                                    });
                                }
                                if ui.button("Vincular").on_hover_text("Muestra un código QR, para canales como WhatsApp").clicked() {
                                    app.start_qr(channel, &id);
                                }
                                if running {
                                    if ui.button("Detener").clicked() {
                                        app.act("channels.stop", target, "Canal detenido");
                                    }
                                } else if ui.button("Iniciar").clicked() {
                                    app.act("channels.start", target, "Canal iniciado");
                                }
                            });
                        });
                        if let Some(e) = a["lastError"].as_str() {
                            ui.label(RichText::new(shorten(e, 140)).size(11.5).color(p.red));
                        }
                    }
                });
                ui.add_space(8.0);
            }
        }
        if !any {
            empty(ui, p, "Aún no tienes canales. Elige uno arriba para empezar.");
        }
        if let Some(issues) = status["statusIssues"].as_array() {
            for i in issues {
                ui.label(RichText::new(i["message"].as_str().unwrap_or("")).size(12.0).color(p.amber));
            }
        }
    });
}

// ---- plugins ------------------------------------------------------------------

fn tabs(ui: &mut egui::Ui, p: &Palette, current: &mut usize, labels: &[&str]) {
    ui.horizontal(|ui| {
        for (i, l) in labels.iter().enumerate() {
            let on = *current == i;
            let text = RichText::new(*l).size(13.5).color(if on { p.text } else { p.text2 });
            if ui.add(Button::new(text).frame(on)).clicked() {
                *current = i;
            }
        }
    });
    ui.add_space(8.0);
}

fn plugins(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Complementos", "Lo que amplía a OpenClaw: proveedores, canales y herramientas");
    let mut tab = app.mgr.forms.plugin_tab;
    tabs(ui, p, &mut tab, &["Instalados", "Buscar e instalar", "Ganchos"]);
    app.mgr.forms.plugin_tab = tab;
    let list = data(app, "plugins");
    match tab {
        0 => {
            ui.add(TextEdit::singleline(&mut app.mgr.forms.plugin_search).hint_text("Filtrar").desired_width(320.0));
            error_note(app, ui, p, "plugins");
            let needle = app.mgr.forms.plugin_search.to_lowercase();
            let rows: Vec<&Value> = list["plugins"].as_array().map(|a| {
                a.iter()
                    .filter(|x| x["installed"].as_bool() == Some(true) || x["enabled"].as_bool() == Some(true))
                    .filter(|x| needle.is_empty() || s(x, "name").to_lowercase().contains(&needle) || s(x, "id").to_lowercase().contains(&needle))
                    .collect()
            }).unwrap_or_default();
            ui.add_space(6.0);
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for x in rows.iter().take(250) {
                    let id = s(x, "id").to_string();
                    let on = x["enabled"].as_bool() == Some(true);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new(s(x, "name")).size(13.5).strong());
                            muted(ui, p, shorten(s(x, "description"), 90));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut v = on;
                            if ui.checkbox(&mut v, "").changed() {
                                app.plugin_enable(&id, v);
                            }
                            muted(ui, p, s(x, "version"));
                            if s(x, "origin") != "bundled" && s(x, "origin") != "core" {
                                let id2 = id.clone();
                                if danger(ui, p, "Quitar").clicked() {
                                    confirm(app, "Quitar complemento", "Se borrará este complemento y su configuración.", "Quitar", move |a, _| {
                                        a.act("plugins.uninstall", json!({"pluginId": id2}), "Complemento quitado");
                                    });
                                }
                            }
                        });
                    });
                    ui.add(egui::Separator::default().spacing(4.0));
                }
                if rows.is_empty() {
                    empty(ui, p, "Nada que mostrar con ese filtro.");
                }
            });
        }
        1 => plugin_browse(app, ui, p),
        _ => hooks(app, ui, p),
    }
}

fn plugin_browse(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let catalog = data(app, "catalog_plugin");
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(&mut app.mgr.forms.plugin_spec).hint_text("Buscar en ClawHub o escribe un paquete").desired_width(320.0));
        if primary(ui, p, "Buscar").clicked() {
            let q = app.mgr.forms.plugin_spec.clone();
            app.browse_catalog("plugin", &q);
        }
        let mut src = app.mgr.forms.plugin_source;
        egui::ComboBox::from_id_salt("plugin-src").selected_text(match src { 0 => "Instalar de npm", 1 => "Instalar de git", _ => "Instalar de una carpeta" }).show_ui(ui, |ui| {
            ui.selectable_value(&mut src, 0, "Instalar de npm");
            ui.selectable_value(&mut src, 1, "Instalar de git");
            ui.selectable_value(&mut src, 2, "Instalar de una carpeta");
        });
        app.mgr.forms.plugin_source = src;
        if ui.button("Instalar").clicked() {
            let spec = app.mgr.forms.plugin_spec.trim().to_string();
            if !spec.is_empty() {
                let params = match src {
                    0 => json!({"source": "npm", "spec": spec}),
                    1 => json!({"source": "git", "spec": spec}),
                    _ => json!({"source": "local", "path": spec}),
                };
                app.plugin_install(params);
            }
        }
    });
    error_note(app, ui, p, "catalog_plugin");
    if let Some(e) = catalog["remoteError"].as_str() {
        ui.label(RichText::new(format!("ClawHub no respondió: {e}")).size(12.0).color(p.amber));
    }
    ui.add_space(6.0);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let items = catalog["items"].as_array().cloned().unwrap_or_default();
        for it in &items {
            let c = &it["catalog"];
            let name = c["displayName"].as_str().or(c["name"].as_str()).or(c["packageName"].as_str()).unwrap_or("");
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(name).size(13.5).strong());
                    muted(ui, p, shorten(c["summary"].as_str().or(c["description"].as_str()).unwrap_or(""), 90));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if it["local"]["installed"].as_bool() == Some(true) {
                        muted(ui, p, "Instalado");
                    } else if let Some(pkg) = c["packageName"].as_str() {
                        if ui.button("Instalar").clicked() {
                            app.plugin_install(json!({"source": "clawhub", "packageName": pkg}));
                        }
                    }
                });
            });
            ui.add(egui::Separator::default().spacing(4.0));
        }
        if items.is_empty() {
            empty(ui, p, "Busca algo para ver resultados.");
        }
    });
}

fn hooks(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let v = data(app, "hooks");
    error_note(app, ui, p, "hooks");
    muted(ui, p, "Acciones automáticas que OpenClaw ejecuta cuando ocurre algo.");
    ui.add_space(6.0);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for h in v["hooks"].as_array().cloned().unwrap_or_default() {
            let key = s(&h, "hookKey").to_string();
            let on = h["enabledByConfig"].as_bool() == Some(true);
            ui.horizontal(|ui| {
                let mut value = on;
                let usable = h["requirementsSatisfied"].as_bool() != Some(false);
                if ui.add_enabled(usable, egui::Checkbox::new(&mut value, "")).changed() {
                    let mut entries = serde_json::Map::new();
                    entries.insert(key.clone(), json!({"enabled": value}));
                    app.patch_config(json!({"hooks": {"internal": {"entries": entries}}}));
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(s(&h, "name")).strong());
                    muted(ui, p, shorten(s(&h, "description"), 110));
                });
            });
        }
    });
}

// ---- skills -------------------------------------------------------------------

fn skills(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Habilidades", "Lo que tus Bots saben hacer");
    let mut tab = app.mgr.forms.skill_tab;
    tabs(ui, p, &mut tab, &["Instaladas", "Buscar e instalar"]);
    app.mgr.forms.skill_tab = tab;
    if tab == 1 {
        let catalog = data(app, "catalog_skill");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut app.mgr.forms.skill_search).hint_text("Buscar en ClawHub").desired_width(320.0));
            if primary(ui, p, "Buscar").clicked() {
                let q = app.mgr.forms.skill_search.clone();
                app.browse_catalog("skill", &q);
            }
        });
        error_note(app, ui, p, "catalog_skill");
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let items = catalog["items"].as_array().cloned().unwrap_or_default();
            for it in &items {
                let c = &it["catalog"];
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(c["displayName"].as_str().or(c["name"].as_str()).unwrap_or(s(it, "id"))).size(13.5).strong());
                        muted(ui, p, shorten(c["summary"].as_str().or(c["description"].as_str()).unwrap_or(""), 90));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if it["local"]["installed"].as_bool() == Some(true) {
                            muted(ui, p, "Instalada");
                        } else if ui.button("Instalar").clicked() {
                            app.act("skills.install", json!({"source": "clawhub", "slug": it["id"]}), "Habilidad instalada");
                        }
                    });
                });
                ui.add(egui::Separator::default().spacing(4.0));
            }
            if items.is_empty() {
                empty(ui, p, "Busca algo para ver resultados.");
            }
        });
        return;
    }
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(&mut app.mgr.forms.skill_search).hint_text("Filtrar").desired_width(320.0));
        if ui.button("Actualizar todas desde ClawHub").clicked() {
            app.act("skills.update", json!({"source": "clawhub", "all": true}), "Habilidades actualizadas");
        }
    });
    error_note(app, ui, p, "skills");
    let needle = app.mgr.forms.skill_search.to_lowercase();
    let v = data(app, "skills");
    ui.add_space(4.0);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for x in v["skills"].as_array().cloned().unwrap_or_default() {
            let name = s(&x, "name").to_string();
            if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
                continue;
            }
            let key = x["skillKey"].as_str().unwrap_or(&name).to_string();
            let on = x["disabled"].as_bool() != Some(true);
            let eligible = x["eligible"].as_bool() != Some(false);
            ui.horizontal(|ui| {
                let mut value = on;
                if ui.checkbox(&mut value, "").changed() {
                    app.act("skills.update", json!({"skillKey": key, "enabled": value}), "Listo");
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(format!("{} {}", s(&x, "emoji"), name)).size(13.5).strong());
                    muted(ui, p, shorten(s(&x, "description"), 110));
                    if !eligible {
                        let bins: Vec<String> = x["missing"]["bins"].as_array().map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        let envs: Vec<String> = x["missing"]["env"].as_array().map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        let mut need = bins;
                        need.extend(envs);
                        ui.label(RichText::new(format!("Falta: {}", need.join(", "))).size(11.5).color(p.amber));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(opts) = x["install"].as_array() {
                        for o in opts.iter().take(2) {
                            let label = o["label"].as_str().unwrap_or("Instalar");
                            if !eligible && ui.small_button(shorten(label, 28)).clicked() {
                                app.act("skills.install", json!({"name": name, "installId": o["id"]}), "Instalado");
                            }
                        }
                    }
                });
            });
            ui.add(egui::Separator::default().spacing(4.0));
        }
    });
}

// ---- routines -----------------------------------------------------------------

fn routines(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Rutinas", "Tareas que tus Bots hacen solos, en el horario que elijas");
    ui.horizontal(|ui| {
        if primary(ui, p, "+  Nueva rutina").clicked() {
            app.mgr.forms.routine.open = true;
            app.mgr.forms.routine.error = None;
        }
    });
    ui.add_space(8.0);
    error_note(app, ui, p, "cron");
    let list = app.routines.clone();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            empty(ui, p, "Aún no hay rutinas.");
        }
        for job in &list {
            card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    let mut on = job.enabled;
                    if ui.checkbox(&mut on, "").changed() {
                        app.toggle_routine(job);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&job.name).size(14.0).strong());
                        muted(ui, p, format!("{}  ·  {}", job.schedule, job.last));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let id = job.id.clone();
                        if danger(ui, p, "Borrar").clicked() {
                            confirm(app, "Borrar rutina", "Dejará de ejecutarse y no se puede deshacer.", "Borrar", move |a, _| {
                                a.act("cron.remove", json!({"id": id}), "Rutina borrada");
                                a.reload_routines();
                            });
                        }
                        if ui.button("Ejecutar ahora").clicked() {
                            app.run_routine(&job.id);
                        }
                    });
                });
            });
            ui.add_space(6.0);
        }
    });
}

// ---- devices ------------------------------------------------------------------

fn devices(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Equipos", "Los teléfonos, computadores y apps conectados a este OpenClaw");
    ui.horizontal(|ui| {
        if primary(ui, p, "Conectar un teléfono o equipo").clicked() {
            app.request_setup_code();
        }
    });
    ui.add_space(8.0);
    let pairing = data(app, "pairing");
    let nodes = data(app, "nodes");
    let node_pairing = data(app, "node_pairing");
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let pending = pairing["pending"].as_array().cloned().unwrap_or_default();
        let pending_nodes = node_pairing["pending"].as_array().cloned().unwrap_or_default();
        if !pending.is_empty() || !pending_nodes.is_empty() {
            card(ui, p, |ui| {
                title(ui, "Esperando tu aprobación");
                for d in &pending {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if s(d, "displayName").is_empty() { "Equipo sin nombre" } else { s(d, "displayName") }).strong());
                        muted(ui, p, format!("{} · {}", s(d, "platform"), s(d, "remoteIp")));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let id = s(d, "requestId").to_string();
                            let id2 = id.clone();
                            if ui.button("Rechazar").clicked() {
                                app.act("device.pair.reject", json!({"requestId": id2}), "Rechazado");
                            }
                            if primary(ui, p, "Aprobar").clicked() {
                                app.act("device.pair.approve", json!({"requestId": id}), "Equipo aprobado");
                            }
                        });
                    });
                }
                for d in &pending_nodes {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if s(d, "displayName").is_empty() { "Equipo sin nombre" } else { s(d, "displayName") }).strong());
                        muted(ui, p, format!("capacidades · {}", s(d, "platform")));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let id = s(d, "requestId").to_string();
                            let id2 = id.clone();
                            if ui.button("Rechazar").clicked() {
                                app.act("node.pair.reject", json!({"requestId": id2}), "Rechazado");
                            }
                            if primary(ui, p, "Aprobar").clicked() {
                                app.act("node.pair.approve", json!({"requestId": id}), "Equipo aprobado");
                            }
                        });
                    });
                }
            });
            ui.add_space(10.0);
        }
        error_note(app, ui, p, "pairing");
        card(ui, p, |ui| {
            title(ui, "Con acceso a OpenClaw");
            let paired = pairing["paired"].as_array().cloned().unwrap_or_default();
            if paired.is_empty() {
                empty(ui, p, "Ninguno todavía.");
            }
            for d in &paired {
                let id = s(d, "deviceId").to_string();
                let mine = id == app.identity.device_id;
                ui.horizontal(|ui| {
                    dot(ui, if d["connected"].as_bool() == Some(true) { p.green } else { p.text3 });
                    let name = d["label"].as_str().or(d["displayName"].as_str()).unwrap_or("Sin nombre");
                    ui.label(RichText::new(if mine { format!("{name} (este)") } else { name.to_string() }).strong());
                    muted(ui, p, format!("{} · {}", s(d, "platform"), time_label(d["lastSeenAtMs"].as_i64().unwrap_or(0))));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !mine {
                            let id2 = id.clone();
                            if danger(ui, p, "Quitar").clicked() {
                                confirm(app, "Quitar equipo", "Perderá el acceso a este OpenClaw hasta que lo apruebes de nuevo.", "Quitar", move |a, _| {
                                    a.act("device.pair.remove", json!({"deviceId": id2}), "Equipo quitado");
                                });
                            }
                        }
                        if ui.button("Renombrar").clicked() {
                            app.mgr.forms.rename_device = Some((id.clone(), name.to_string()));
                        }
                    });
                });
            }
        });
        ui.add_space(10.0);
        error_note(app, ui, p, "nodes");
        card(ui, p, |ui| {
            title(ui, "Equipos que hacen trabajo para los Bots");
            muted(ui, p, "Computadores o teléfonos donde los Bots pueden usar pantalla, cámara o comandos.");
            ui.add_space(4.0);
            let list = nodes["nodes"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                empty(ui, p, "Ninguno.");
            }
            for n in &list {
                let id = s(n, "nodeId").to_string();
                ui.horizontal(|ui| {
                    dot(ui, if n["connected"].as_bool() == Some(true) { p.green } else { p.text3 });
                    let name = if s(n, "displayName").is_empty() { &id } else { s(n, "displayName") };
                    ui.label(RichText::new(name).strong());
                    let commands = n["commands"].as_array().map(|a| a.len()).unwrap_or(0);
                    muted(ui, p, format!("{} · {commands} funciones", s(n, "platform")));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let id2 = id.clone();
                        if danger(ui, p, "Quitar").clicked() {
                            confirm(app, "Quitar equipo", "Ya no podrá hacer trabajo para los Bots.", "Quitar", move |a, _| {
                                a.act("node.pair.remove", json!({"nodeId": id2}), "Equipo quitado");
                            });
                        }
                        if ui.button("Renombrar").clicked() {
                            app.mgr.forms.rename_node = Some((id.clone(), name.to_string()));
                        }
                    });
                });
            }
        });
    });
}

// ---- permissions --------------------------------------------------------------

fn fleet(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    page_header(app, ui, p, "Flota", "Tus máquinas: enrolar, inventario, comandos y escritorio remoto");
    let configured = app.mgr.fleet.is_configured();

    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Configuración del toolkit.
        card(ui, p, |ui| {
            title(ui, "Toolkit de flota");
            muted(ui, p, "Carpeta orchestrator del toolkit (OpenClaw-Bot-Fleet.zip, en cada versión de GitHub) y el Python a usar.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Carpeta:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.dir).desired_width(340.0).hint_text("…/OpenClaw-Bot-Fleet/orchestrator"));
                if ui.button("Elegir…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.mgr.fleet.dir = dir.display().to_string();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Python:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.python).desired_width(140.0).hint_text("python"));
                if ui.button("Guardar").clicked() {
                    app.save_fleet_settings();
                    app.toast("Configuración de flota guardada.", true);
                }
            });
            if !configured {
                ui.add_space(4.0);
                muted(ui, p, "Todavía no encuentro fleet.py en esa carpeta.");
            }
        });
        ui.add_space(10.0);

        if let Some(err) = app.mgr.fleet.error.clone() {
            card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(err).color(p.red));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Ocultar").clicked() {
                            app.mgr.fleet.error = None;
                        }
                        if app.mgr.fleet.missing_deps && ui.button("Instalar dependencias").clicked() {
                            match app.mgr.fleet.install_deps_command() {
                                Ok(cmd) => {
                                    let c = ctx.clone();
                                    let id = app.mgr.jobs.start("Instalar dependencias de la flota", "python -m pip install -r requirements.txt".into(), cmd, None, move || c.request_repaint());
                                    app.mgr.forms.job_open = Some(id);
                                    app.mgr.fleet.error = None;
                                    app.mgr.fleet.missing_deps = false;
                                }
                                Err(e) => app.mgr.fleet.error = Some(e),
                            }
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }

        // Passphrase del vault (sólo en memoria).
        card(ui, p, |ui| {
            title(ui, "Vault");
            muted(ui, p, "Passphrase maestra para abrir el vault de contraseñas admin. Sólo en esta sesión.");
            ui.horizontal(|ui| {
                ui.label("Passphrase:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.passphrase).password(true).desired_width(240.0));
            });
        });
        ui.add_space(10.0);

        // Inventario.
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                title(ui, "Inventario");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(configured && !app.mgr.fleet.loading, Button::new("Actualizar")).clicked() {
                        let c = ctx.clone();
                        app.mgr.fleet.refresh(move || c.request_repaint());
                    }
                    if app.mgr.fleet.loading {
                        ui.label(RichText::new("Cargando…").color(p.amber).size(12.0));
                    }
                });
            });
            ui.add_space(4.0);
            if app.mgr.fleet.machines.is_empty() {
                if app.mgr.fleet.loaded_once {
                    empty(ui, p, "No hay máquinas enroladas todavía. Usá «Enrolar una máquina» abajo.");
                } else {
                    muted(ui, p, "Tocá «Actualizar» para leer el inventario desde Headscale.");
                }
            } else {
                for m in app.mgr.fleet.machines.clone() {
                    Frame::new().fill(p.field).corner_radius(r(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            dot(ui, if m.online { p.green } else { p.text3 });
                            ui.label(RichText::new(&m.hostname).strong());
                            ui.label(RichText::new(format!("{}  ·  {}", m.mesh_ip, m.os)).color(p.text2).size(12.0));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Usar").on_hover_text("Elegir como destino de comandos").clicked() {
                                    app.mgr.fleet.target = m.hostname.clone();
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        });
        ui.add_space(10.0);

        // Ejecutar comando por SSH.
        card(ui, p, |ui| {
            title(ui, "Ejecutar en una máquina");
            ui.horizontal(|ui| {
                ui.label("Destino:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.target).desired_width(180.0).hint_text("hostname"));
                ui.checkbox(&mut app.mgr.fleet.use_sudo, "con admin/sudo");
            });
            ui.horizontal(|ui| {
                ui.label("Comando:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.command).desired_width(340.0).hint_text("uptime"));
                let ready = configured
                    && !app.mgr.fleet.target.trim().is_empty()
                    && !app.mgr.fleet.command.trim().is_empty();
                if ui.add_enabled(ready, Button::new("Ejecutar")).clicked() {
                    let host = app.mgr.fleet.target.trim().to_string();
                    let cmdline = app.mgr.fleet.command.trim().to_string();
                    let sudo = app.mgr.fleet.use_sudo;
                    // --sudo va antes del host: después, argparse lo tomaría como parte del comando.
                    let mut args: Vec<&str> = vec!["run"];
                    if sudo {
                        args.push("--sudo");
                    }
                    args.push(&host);
                    args.push(&cmdline); // el comando viaja como un solo argumento
                    app.run_fleet("Comando remoto", &args, None, &ctx);
                }
            });
            muted(ui, p, "La salida aparece abajo, en vivo.");
        });
        ui.add_space(10.0);

        // Vault: guardar contraseña de admin.
        card(ui, p, |ui| {
            title(ui, "Guardar contraseña de administrador");
            muted(ui, p, "Para máquinas que piden admin/sudo. Se guarda cifrada en el vault.");
            ui.horizontal(|ui| {
                ui.label("Host:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.vault_host).desired_width(150.0).hint_text("hostname"));
                ui.label("Contraseña:");
                ui.add(TextEdit::singleline(&mut app.mgr.fleet.vault_pw).password(true).desired_width(170.0));
                let ready = configured
                    && !app.mgr.fleet.vault_host.trim().is_empty()
                    && !app.mgr.fleet.vault_pw.is_empty()
                    && !app.mgr.fleet.passphrase.is_empty();
                if ui.add_enabled(ready, Button::new("Guardar")).clicked() {
                    let host = app.mgr.fleet.vault_host.trim().to_string();
                    let pw = std::mem::take(&mut app.mgr.fleet.vault_pw);
                    app.run_fleet_set_admin(&host, &pw, &ctx);
                }
            });
            if app.mgr.fleet.passphrase.is_empty() {
                muted(ui, p, "Primero cargá la passphrase maestra del vault, arriba.");
            }
        });
        ui.add_space(10.0);

        // Enrolar una máquina nueva.
        card(ui, p, |ui| {
            title(ui, "Enrolar una máquina");
            muted(ui, p, "Generá el comando de instalación y corrélo en la máquina nueva.");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.mgr.fleet.enroll_platform, 0, "Linux");
                ui.selectable_value(&mut app.mgr.fleet.enroll_platform, 1, "Windows");
                if ui.add_enabled(configured, Button::new("Generar comando")).clicked() {
                    let c = ctx.clone();
                    app.mgr.fleet.refresh_enroll(move || c.request_repaint());
                }
            });
            if !app.mgr.fleet.enroll_cmd.is_empty() {
                ui.add_space(4.0);
                Frame::new().fill(p.field).corner_radius(r(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.label(RichText::new(&app.mgr.fleet.enroll_cmd).monospace().size(11.5));
                });
                if ui.small_button("Copiar").clicked() {
                    ui.ctx().copy_text(app.mgr.fleet.enroll_cmd.clone());
                }
            }
        });
        ui.add_space(10.0);

        // Salida en vivo de los comandos (reusa el panel de trabajos).
        if app.mgr.forms.job_open.is_some() {
            job_view(app, ui, p);
        }
    });
}

fn approvals(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Permisos", "Qué pueden hacer tus Bots sin preguntar");
    let policy = data(app, "exec_policy");
    if !app.mgr.forms.approvals_loaded && !policy.is_null() {
        let d = &policy["resolvedDefaults"];
        let f = &mut app.mgr.forms;
        f.approvals_security = d["security"].as_str().unwrap_or("allowlist").to_string();
        f.approvals_ask = d["ask"].as_str().unwrap_or("on-miss").to_string();
        f.approvals_fallback = d["askFallback"].as_str().unwrap_or("deny").to_string();
        f.approvals_auto = d["autoAllowSkills"].as_bool().unwrap_or(false);
        f.approvals_loaded = true;
    }
    let history = data(app, "approval_history");
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let pending = app.approvals.clone();
        if !pending.is_empty() {
            card(ui, p, |ui| {
                title(ui, "Pidiendo permiso ahora");
                for a in &pending {
                    ui.label(RichText::new(&a.title).strong());
                    ui.label(RichText::new(&a.detail).monospace().size(12.0));
                    ui.horizontal(|ui| {
                        if primary(ui, p, "Permitir una vez").clicked() {
                            app.resolve_approval(&a.id, a.plugin, "allow-once");
                        }
                        if ui.button("Permitir siempre").clicked() {
                            app.resolve_approval(&a.id, a.plugin, "allow-always");
                        }
                        if ui.button("Denegar").clicked() {
                            app.resolve_approval(&a.id, a.plugin, "deny");
                        }
                    });
                    ui.add_space(6.0);
                }
            });
            ui.add_space(10.0);
        }
        error_note(app, ui, p, "exec_policy");
        card(ui, p, |ui| {
            title(ui, "Cuando un Bot quiere ejecutar comandos");
            let f = &mut app.mgr.forms;
            let combo = |ui: &mut egui::Ui, id: &str, v: &mut String, opts: &[(&str, &str)]| {
                let label = opts.iter().find(|(k, _)| k == v).map(|(_, l)| l.to_string()).unwrap_or_else(|| v.clone());
                egui::ComboBox::from_id_salt(id).selected_text(label).width(280.0).show_ui(ui, |ui| {
                    for (k, l) in opts {
                        ui.selectable_value(v, k.to_string(), *l);
                    }
                });
            };
            egui::Grid::new("policy").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Qué se permite");
                combo(ui, "sec", &mut f.approvals_security, &[("deny", "Nada"), ("allowlist", "Solo lo de la lista"), ("full", "Todo")]);
                ui.end_row();
                ui.label("Cuándo preguntar");
                combo(ui, "ask", &mut f.approvals_ask, &[("off", "Nunca"), ("on-miss", "Si no está en la lista"), ("always", "Siempre")]);
                ui.end_row();
                ui.label("Si nadie responde");
                combo(ui, "fb", &mut f.approvals_fallback, &[("deny", "Denegar"), ("allowlist", "Solo lo de la lista"), ("full", "Permitir")]);
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut f.approvals_auto, "Permitir lo que necesitan las habilidades");
                ui.end_row();
            });
            ui.add_space(6.0);
            if primary(ui, p, "Guardar permisos").clicked() {
                app.save_exec_policy();
            }
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            title(ui, "Historial");
            let items = history["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                empty(ui, p, "Sin decisiones en los últimos 30 días.");
            }
            for i in items.iter().take(50) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(s(i, "decision")).size(12.5).strong());
                    muted(ui, p, shorten(i["summary"].as_str().or(i["title"].as_str()).unwrap_or(s(i, "kind")), 90));
                });
            }
        });
    });
}

// ---- secrets ------------------------------------------------------------------

fn secrets(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Claves", "Datos secretos que OpenClaw guarda para tus Bots");
    let v = data(app, "secrets");
    card(ui, p, |ui| {
        title(ui, "Guardar una clave");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut app.mgr.forms.secret_name).hint_text("Nombre, como MI_CLAVE").desired_width(220.0));
            ui.add(TextEdit::singleline(&mut app.mgr.forms.secret_value).password(true).hint_text("Valor").desired_width(280.0));
            ui.checkbox(&mut app.mgr.forms.secret_is_env, "Disponible como variable");
            if primary(ui, p, "Guardar").clicked() {
                let name = app.mgr.forms.secret_name.trim().to_string();
                let value = app.mgr.forms.secret_value.clone();
                if !name.is_empty() && !value.is_empty() {
                    let kind = if app.mgr.forms.secret_is_env { "env" } else { "secret" };
                    app.mgr.forms.secret_value.clear();
                    app.mgr.forms.secret_name.clear();
                    app.act("secrets.store.set", json!({"name": name, "value": value, "kind": kind}), "Clave guardada");
                }
            }
        });
    });
    ui.add_space(10.0);
    error_note(app, ui, p, "secrets");
    ui.horizontal(|ui| {
        title(ui, "Guardadas");
        if ui.small_button("Volver a leer las claves").on_hover_text("OpenClaw vuelve a leer todas sus claves").clicked() {
            app.act("secrets.reload", json!({}), "Claves recargadas");
        }
    });
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let list = v["entries"].as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            empty(ui, p, "No hay claves guardadas.");
        }
        for e in &list {
            let name = s(e, "name").to_string();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&name).monospace());
                muted(ui, p, s(e, "kind"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if danger(ui, p, "Borrar").clicked() {
                        let n = name.clone();
                        confirm(app, "Borrar clave", "Lo que la use dejará de funcionar.", "Borrar", move |a, _| {
                            a.act("secrets.store.delete", json!({"name": n}), "Clave borrada");
                        });
                    }
                });
            });
        }
    });
}

// ---- usage --------------------------------------------------------------------

fn usage(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    page_header(app, ui, p, "Uso", "Cuánto han consumido tus Bots");
    let status = data(app, "usage_status");
    let cost = data(app, "usage_cost");
    ui.horizontal(|ui| {
        for d in [7usize, 30, 90] {
            if ui.selectable_label(app.mgr.forms.usage_days == d, format!("{d} días")).clicked() {
                app.mgr.forms.usage_days = d;
                app.load_section(&ctx);
            }
        }
    });
    ui.add_space(8.0);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        error_note(app, ui, p, "usage_cost");
        let totals = &cost["totals"];
        ui.horizontal(|ui| {
            stat(ui, p, 190.0, "Gasto", &format!("US$ {:.2}", totals["totalCost"].as_f64().unwrap_or(0.0)), "estimado");
            stat(ui, p, 190.0, "Tokens", &group_digits(totals["totalTokens"].as_u64().unwrap_or(0)), "en total");
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            title(ui, "Por día");
            let days = cost["daily"].as_array().cloned().unwrap_or_default();
            let max = days.iter().map(|d| d["totalTokens"].as_f64().unwrap_or(0.0)).fold(1.0, f64::max);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 120.0), Sense::hover());
            let n = days.len().max(1);
            let w = rect.width() / n as f32;
            for (i, d) in days.iter().enumerate() {
                let f = (d["totalTokens"].as_f64().unwrap_or(0.0) / max) as f32;
                let x = rect.left() + i as f32 * w;
                let bar = egui::Rect::from_min_max(egui::pos2(x + 1.0, rect.bottom() - f * rect.height()), egui::pos2(x + (w - 1.0).max(1.0), rect.bottom()));
                ui.painter().rect_filled(bar, r(2), p.accent);
            }
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            title(ui, "Cupos de tus proveedores");
            let list = status["providers"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                empty(ui, p, "Tus proveedores no informan cupos.");
            }
            for x in &list {
                ui.label(RichText::new(x["displayName"].as_str().or(x["provider"].as_str()).unwrap_or("")).strong());
                for w in x["windows"].as_array().cloned().unwrap_or_default() {
                    let used = w["usedPercent"].as_f64().unwrap_or(0.0);
                    kv(ui, p, w["label"].as_str().unwrap_or(""), format!("{used:.0} % usado"));
                }
            }
        });
    });
}

// ---- logs ---------------------------------------------------------------------

fn logs(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Registros", "Lo que OpenClaw va anotando");
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(&mut app.mgr.forms.log_filter).hint_text("Filtrar").desired_width(320.0));
        ui.checkbox(&mut app.mgr.forms.log_follow, "Seguir en vivo");
        if ui.button("Copiar").clicked() {
            let text = app.mgr.log_lines.join("\n");
            ui.ctx().copy_text(text);
        }
        muted(ui, p, shorten(&app.mgr.log_file, 60));
    });
    ui.add_space(6.0);
    let needle = app.mgr.forms.log_filter.to_lowercase();
    Frame::new().fill(p.field).corner_radius(r(8)).inner_margin(Margin::same(10)).show(ui, |ui| {
        ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let lines: Vec<&String> = app.mgr.log_lines.iter().filter(|l| needle.is_empty() || l.to_lowercase().contains(&needle)).collect();
            let start = lines.len().saturating_sub(1500);
            for l in &lines[start..] {
                let color = if l.contains("ERROR") || l.contains("FATAL") {
                    p.red
                } else if l.contains("WARN") {
                    p.amber
                } else {
                    p.text2
                };
                ui.label(RichText::new(l.as_str()).monospace().size(11.5).color(color));
            }
        });
    });
}

// ---- service & maintenance ------------------------------------------------------

fn job_view(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let Some(id) = app.mgr.forms.job_open else { return };
    let Some(job) = app.mgr.jobs.get(id) else { return };
    let (state_text, color) = match &job.state {
        JobState::Running => ("En curso…".to_string(), p.amber),
        JobState::Finished(0) => ("Terminó bien".to_string(), p.green),
        JobState::Finished(c) => (format!("Terminó con error ({c})"), p.red),
        JobState::Failed(e) => (e.clone(), p.red),
        JobState::Stopped => ("Detenido".to_string(), p.text2),
    };
    let running = job.state == JobState::Running;
    let job_title = job.title.clone();
    let shown = job.shown.clone();
    let text = job.text();
    ui.add_space(10.0);
    card(ui, p, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(job_title).strong());
            ui.label(RichText::new(state_text).size(12.0).color(color));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Cerrar").clicked() {
                    app.mgr.forms.job_open = None;
                }
                if ui.small_button("Copiar").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                if running && ui.small_button("Detener").clicked() {
                    app.mgr.jobs.stop(id);
                }
            });
        });
        ui.label(RichText::new(shown).monospace().size(11.0).color(p.text3));
        ui.add_space(4.0);
        Frame::new().fill(p.field).corner_radius(r(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
            ScrollArea::both().id_salt("job").max_height(260.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                ui.label(RichText::new(text).monospace().size(11.5));
            });
        });
    });
}

fn maintenance(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    page_header(app, ui, p, "Servicio", "Encender, reparar, actualizar y respaldar OpenClaw");
    let can_cli = app.cli_host().is_some();
    let update = data(app, "update");
    let local = app.is_local_profile();
    let installed = !local || crate::cli::find_local().is_some();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if !installed {
            card(ui, p, |ui| {
                title(ui, "OpenClaw no está instalado en este equipo");
                if primary(ui, p, "Abrir el asistente de instalación").clicked() {
                    launch_setup(app);
                }
            });
            ui.add_space(10.0);
        }
        if !can_cli {
            card(ui, p, |ui| {
                title(ui, "Conexión por red");
                muted(ui, p, "Aquí se puede reiniciar y actualizar. Para apagar, reparar o respaldar, conéctate por túnel SSH.");
            });
            ui.add_space(10.0);
        }
        let connected = app.is_connected();
        ui.columns(2, |cols| {
            card(&mut cols[0], p, |ui| {
                title(ui, "Servicio de OpenClaw");
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(can_cli, Button::new("Ver estado")).clicked() {
                        app.run_cli("Estado del servicio", &["gateway", "status"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Encender")).clicked() {
                        app.run_cli("Encender OpenClaw", &["gateway", "start"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Apagar")).clicked() {
                        confirm(app, "Apagar OpenClaw", "Tus Bots dejarán de responder hasta que lo enciendas.", "Apagar", |a, c| {
                            a.run_cli("Apagar OpenClaw", &["gateway", "stop"], c);
                        });
                    }
                    if ui.add_enabled(can_cli || connected, Button::new("Reiniciar")).clicked() {
                        if can_cli {
                            app.run_cli("Reiniciar OpenClaw", &["gateway", "restart"], &ctx);
                        } else {
                            app.restart_gateway();
                        }
                    }
                    if ui.add_enabled(can_cli, Button::new("Instalar como servicio")).on_hover_text("Hace que OpenClaw arranque solo con el equipo").clicked() {
                        app.run_cli("Instalar el servicio", &["gateway", "install", "--force"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Quitar servicio")).clicked() {
                        confirm(app, "Quitar servicio", "OpenClaw dejará de arrancar solo. Tus datos no se borran.", "Quitar", |a, c| {
                            a.run_cli("Quitar el servicio", &["gateway", "uninstall"], c);
                        });
                    }
                });
            });
            card(&mut cols[1], p, |ui| {
                title(ui, "Revisar y reparar");
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(can_cli, Button::new("Revisar")).clicked() {
                        app.run_cli("Revisión", &["doctor", "--non-interactive"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Reparar")).clicked() {
                        app.run_cli("Reparación", &["doctor", "--repair", "--non-interactive"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Revisar seguridad")).clicked() {
                        app.run_cli("Seguridad", &["security", "audit"], &ctx);
                    }
                    if ui.add_enabled(can_cli, Button::new("Corregir seguridad")).clicked() {
                        app.run_cli("Seguridad", &["security", "audit", "--fix"], &ctx);
                    }
                });
            });
        });
        ui.add_space(10.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], p, |ui| {
                title(ui, "Actualizar");
                let channel = s(&update, "effectiveChannel").to_string();
                kv(ui, p, "Canal", channel.clone());
                if let Some(u) = update["updateAvailable"].as_object() {
                    kv(ui, p, "Nueva versión", u.get("latestVersion").and_then(Value::as_str).unwrap_or("").to_string());
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(connected, Button::new("Actualizar OpenClaw")).clicked() {
                        confirm(app, "Actualizar OpenClaw", "Se reiniciará al terminar.", "Actualizar", |a, _| {
                            a.act("update.run", json!({"note": "OpenClaw Bot"}), "Actualización iniciada");
                        });
                    }
                    if ui.add_enabled(can_cli, Button::new("Comprobar versión")).clicked() {
                        app.run_cli("Versión", &["update", "status"], &ctx);
                    }
                    for c in ["stable", "beta", "dev"] {
                        if ui.add_enabled(can_cli && channel != c, Button::new(format!("Usar {c}"))).clicked() {
                            let cmd = ["update", "--channel", c, "--yes"];
                            app.run_cli("Cambiar de canal", &cmd, &ctx);
                        }
                    }
                });
            });
            card(&mut cols[1], p, |ui| {
                title(ui, "Copias de seguridad");
                ui.horizontal_wrapped(|ui| {
                    ui.add(TextEdit::singleline(&mut app.mgr.forms.backup_output).hint_text("Carpeta destino (opcional)").desired_width(240.0));
                    if ui.add_enabled(can_cli, Button::new("Crear copia")).clicked() {
                        let dest = app.mgr.forms.backup_output.trim().to_string();
                        let mut args = vec!["backup", "create"];
                        if !dest.is_empty() {
                            args.push("--output");
                            args.push(&dest);
                        }
                        app.run_cli("Crear copia", &args, &ctx);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(TextEdit::singleline(&mut app.mgr.forms.backup_verify).hint_text("Archivo para comprobar").desired_width(240.0));
                    if ui.add_enabled(can_cli, Button::new("Comprobar copia")).clicked() {
                        let path = app.mgr.forms.backup_verify.trim().to_string();
                        if !path.is_empty() {
                            app.run_cli("Comprobar copia", &["backup", "verify", &path], &ctx);
                        }
                    }
                });
            });
        });
        ui.add_space(10.0);
        card(ui, p, |ui| {
            title(ui, "Volver a empezar");
            muted(ui, p, "Estas acciones borran datos. No se pueden deshacer.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Abrir el asistente de configuración").clicked() {
                    launch_setup(app);
                }
                if ui.add_enabled(can_cli, Button::new(RichText::new("Restablecer configuración").color(p.red))).clicked() {
                    confirm(app, "Restablecer configuración", "Se borra la configuración de OpenClaw. Tus Bots, sesiones y claves se conservan.", "Restablecer", |a, c| {
                        a.run_cli("Restablecer", &["reset", "--scope", "config", "--yes", "--non-interactive"], c);
                    });
                }
                if ui.add_enabled(can_cli, Button::new(RichText::new("Borrar todo").color(p.red))).clicked() {
                    confirm(app, "Borrar todo", "Se borra la configuración, las claves, las conversaciones y los archivos de tus Bots.", "Borrar todo", |a, c| {
                        a.run_cli("Borrar todo", &["reset", "--scope", "full", "--yes", "--non-interactive"], c);
                    });
                }
                if ui.add_enabled(can_cli, Button::new(RichText::new("Desinstalar OpenClaw").color(p.red))).clicked() {
                    confirm(app, "Desinstalar OpenClaw", "Quita el servicio y los datos de OpenClaw de ese equipo.", "Desinstalar", |a, c| {
                        a.run_cli("Desinstalar OpenClaw", &["uninstall", "--all", "--yes", "--non-interactive"], c);
                    });
                }
            });
        });
        job_view(app, ui, p);
        ui.add_space(20.0);
    });
}

fn launch_setup(app: &mut App) {
    match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--setup").spawn()) {
        Ok(_) => app.toast("Abriendo el asistente…", true),
        Err(e) => app.toast(format!("No se pudo abrir el asistente: {e}"), false),
    }
}

// ---- configuration -------------------------------------------------------------

fn config(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    page_header(app, ui, p, "Configurar", "Todos los ajustes de OpenClaw y de esta app");
    let mut tab = app.mgr.forms.config_tab;
    tabs(ui, p, &mut tab, &["Ajustes de OpenClaw", "Archivo completo", "Conexiones y app"]);
    app.mgr.forms.config_tab = tab;
    let ctx = ui.ctx().clone();
    if tab == 2 {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            crate::ui::settings_body(app, ui, &ctx, p);
        });
        return;
    }
    if !app.is_connected() {
        offline(app, ui, p);
        return;
    }
    if !app.mgr.config.has_schema() {
        if app.mgr.config.loading {
            ui.spinner();
            muted(ui, p, "Leyendo los ajustes…");
        } else if ui.button("Cargar ajustes").clicked() {
            app.load_section(&ctx);
        }
        if let Some((m, _)) = &app.mgr.config.message {
            ui.label(RichText::new(m).color(p.red));
        }
        return;
    }
    let dirty = app.mgr.config.is_dirty();
    let show_message = |app: &mut App, ui: &mut egui::Ui, p: &Palette| {
        if let Some((m, ok)) = app.mgr.config.message.clone() {
            ui.label(RichText::new(m).size(12.0).color(if ok { p.green } else { p.red }));
        }
    };
    if tab == 1 {
        app.start_config_for_view(View::File);
        muted(ui, p, format!("Archivo: {}", app.mgr.config.path));
        show_message(app, ui, p);
        ui.horizontal(|ui| {
            if primary(ui, p, "Guardar archivo").clicked() {
                confirm(app, "Guardar el archivo completo", "Reemplaza toda la configuración. OpenClaw la valida antes de aplicarla.", "Guardar", |a, _| a.save_config_file());
            }
            if ui.button("Descartar").clicked() {
                app.mgr.config.file_dirty = false;
                app.load_config_values();
            }
        });
        ui.add_space(6.0);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let resp = ui.add(TextEdit::multiline(&mut app.mgr.config.file_text).code_editor().desired_width(f32::INFINITY).desired_rows(36));
            if resp.changed() {
                app.mgr.config.file_dirty = true;
            }
        });
        return;
    }
    app.start_config_for_view(View::Form);
    show_message(app, ui, p);
    let full = ui.available_rect_before_wrap();
    let left_w = 216.0;
    let footer = 44.0;
    let left = egui::Rect::from_min_max(full.min, egui::pos2(full.left() + left_w, full.bottom() - footer));
    let right = egui::Rect::from_min_max(egui::pos2(full.left() + left_w + 18.0, full.top()), egui::pos2(full.right(), full.bottom() - footer));
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(left));
    lui.add(TextEdit::singleline(&mut app.mgr.config.search).hint_text("Buscar un ajuste").desired_width(left_w));
    lui.add_space(6.0);
    let search_on = !app.mgr.config.search.trim().is_empty();
    ScrollArea::vertical().id_salt("groups").auto_shrink([false, false]).show(&mut lui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if search_on {
            let hits = app.mgr.config.search_results(120);
            if hits.is_empty() {
                empty(ui, p, "Sin resultados");
            }
            for row in hits {
                let label = if row.label.is_empty() { row.segs.join(".") } else { row.label.clone() };
                if ui.selectable_label(false, shorten(&label, 30)).on_hover_text(format!("{}\n{}", row.segs.join("."), row.help)).clicked() {
                    app.mgr.config.focus = Some(row.segs);
                }
            }
        } else {
            let groups = app.mgr.config.groups.clone();
            for (key, label) in groups {
                let on = app.mgr.config.group.as_deref() == Some(&key) && app.mgr.config.focus.is_none();
                let marked = app.mgr.config.edits.keys().any(|k| k.first() == Some(&key));
                let text = if marked { format!("{label} •") } else { label };
                if ui.selectable_label(on, text).clicked() {
                    app.mgr.config.group = Some(key);
                    app.mgr.config.focus = None;
                }
            }
        }
    });
    let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right));
    rui.set_clip_rect(right);
    app.mgr.config.form_ui(&mut rui, p.text2, p.accent, p.amber);
    let foot = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - footer + 8.0), full.max);
    let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(foot));
    fui.horizontal(|ui| {
        let saving = app.mgr.config.saving;
        let n = app.mgr.config.edits.len();
        if ui.add_enabled(dirty && !saving, Button::new(RichText::new(if saving { "Guardando…" } else { "Guardar cambios" }).color(Color32::WHITE)).fill(p.bubble).min_size(Vec2::new(0.0, 30.0))).clicked() {
            app.save_config_form();
        }
        if ui.add_enabled(dirty, Button::new("Descartar")).clicked() {
            app.mgr.config.discard();
        }
        if dirty {
            muted(ui, p, format!("{n} cambios sin guardar"));
        }
        if ui.button("Asistente guiado").clicked() {
            app.start_wizard("Asistente de OpenClaw", json!({"flow": "setup"}));
        }
    });
    ui.allocate_rect(full, Sense::hover());
}

// ---- console -------------------------------------------------------------------

fn console(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    page_header(app, ui, p, "Consola", "Para lo que no tenga botón: cualquier orden de OpenClaw");
    let mut tab = app.mgr.forms.console_mode;
    tabs(ui, p, &mut tab, &["Órdenes", "Llamadas directas"]);
    app.mgr.forms.console_mode = tab;
    if tab == 0 {
        ui.horizontal(|ui| {
            ui.label(RichText::new("openclaw").monospace().color(p.text2));
            let resp = ui.add(TextEdit::singleline(&mut app.mgr.forms.console_line).font(egui::TextStyle::Monospace).hint_text("status --all").desired_width(ui.available_width() - 90.0));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if primary(ui, p, "Ejecutar").clicked() || enter {
                app.run_console_line(&ctx);
            }
        });
        muted(ui, p, "No uses aquí órdenes que hagan preguntas en pantalla; para esas están los asistentes.");
        let jobs: Vec<(u64, String, bool)> = app.mgr.jobs.list.iter().rev().take(10).map(|j| (j.id, j.title.clone(), j.state == JobState::Running)).collect();
        ui.horizontal_wrapped(|ui| {
            for (id, t, running) in jobs {
                let label = if running { format!("● {t}") } else { t };
                if ui.selectable_label(app.mgr.forms.job_open == Some(id), label).clicked() {
                    app.mgr.forms.job_open = Some(id);
                }
            }
        });
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| job_view(app, ui, p));
        return;
    }
    if !app.is_connected() {
        offline(app, ui, p);
        return;
    }
    let methods: Vec<String> = {
        let mut m = app.hello.as_ref().map(|h| h.methods.clone()).unwrap_or_default();
        m.sort();
        m
    };
    ui.horizontal(|ui| {
        let label = if app.mgr.forms.rpc_method.is_empty() { "Elige una función".to_string() } else { app.mgr.forms.rpc_method.clone() };
        egui::ComboBox::from_id_salt("rpc").selected_text(label).width(320.0).height(420.0).show_ui(ui, |ui| {
            for m in &methods {
                ui.selectable_value(&mut app.mgr.forms.rpc_method, m.clone(), m);
            }
        });
        ui.add(TextEdit::singleline(&mut app.mgr.forms.rpc_method).hint_text("o escribe su nombre").desired_width(240.0));
        let busy = app.mgr.forms.rpc_busy;
        if ui.add_enabled(!busy, Button::new(RichText::new(if busy { "Enviando…" } else { "Enviar" }).color(Color32::WHITE)).fill(p.bubble)).clicked() {
            app.send_rpc();
        }
    });
    ui.add_space(4.0);
    ui.add(TextEdit::multiline(&mut app.mgr.forms.rpc_params).code_editor().desired_rows(4).desired_width(f32::INFINITY));
    ui.add_space(6.0);
    ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let mut shown = app.mgr.forms.rpc_output.as_str();
        ui.add(TextEdit::multiline(&mut shown).code_editor().desired_width(f32::INFINITY));
    });
}

// ---- dialogs -------------------------------------------------------------------

pub fn dialogs(app: &mut App, ctx: &egui::Context, p: &Palette) {
    confirm_window(app, ctx, p);
    wizard_window(app, ctx, p);
    qr_window(app, ctx, p);
    setup_code_window(app, ctx, p);
    routine_window(app, ctx, p);
    rename_device_windows(app, ctx);
}

fn confirm_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(c) = app.mgr.confirm.take() else { return };
    let mut choice = None;
    egui::Window::new(c.title.clone())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(&c.body);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new(&c.button).color(Color32::WHITE)).fill(p.red)).clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancelar").clicked() {
                    choice = Some(false);
                }
            });
        });
    match choice {
        Some(true) => (c.run)(app, ctx),
        Some(false) => {}
        None => app.mgr.confirm = Some(c),
    }
}

fn wizard_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(w) = &app.mgr.wizard else { return };
    let title_text = w.title.clone();
    let step = w.step.clone();
    let done = w.done;
    let error = w.error.clone();
    let busy = w.busy;
    let summary = w.summary.clone();
    let mut open = true;
    let mut answer: Option<Value> = None;
    let mut close = false;
    egui::Window::new(title_text)
        .open(&mut open)
        .collapsible(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            if let Some(e) = &error {
                ui.label(RichText::new(e).color(p.red));
            }
            if done {
                ui.label(RichText::new(if error.is_none() { "Listo." } else { "No se pudo terminar." }).strong());
                if !summary.is_empty() {
                    ui.label(&summary);
                }
                if primary(ui, p, "Cerrar").clicked() {
                    close = true;
                }
                return;
            }
            let Some(step) = step else {
                ui.spinner();
                return;
            };
            let w = app.mgr.wizard.as_mut().unwrap();
            if let Some(t) = step["title"].as_str() {
                ui.label(RichText::new(t).size(15.0).strong());
            }
            if let Some(m) = step["message"].as_str() {
                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    ui.label(m);
                });
            }
            ui.add_space(6.0);
            let kind = step["type"].as_str().unwrap_or("");
            let opts = step["options"].as_array().cloned().unwrap_or_default();
            match kind {
                "note" | "action" => {
                    if let Some(url) = step["externalUrl"].as_str() {
                        if ui.button("Abrir en el navegador").clicked() {
                            open_url(url);
                        }
                    }
                    if let Some(code) = step["deviceCode"]["code"].as_str() {
                        ui.label(RichText::new(code).monospace().size(20.0).strong());
                    }
                    if !busy && primary(ui, p, "Continuar").clicked() {
                        answer = Some(json!(true));
                    }
                }
                "progress" => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Trabajando…");
                    });
                }
                "text" => {
                    let sensitive = step["sensitive"].as_bool().unwrap_or(false);
                    ui.add(TextEdit::singleline(&mut w.text).password(sensitive).hint_text(step["placeholder"].as_str().unwrap_or("")).desired_width(460.0));
                    ui.add_space(6.0);
                    if !busy && primary(ui, p, "Continuar").clicked() {
                        answer = Some(json!(w.text));
                    }
                }
                "confirm" => {
                    ui.horizontal(|ui| {
                        if !busy && primary(ui, p, "Sí").clicked() {
                            answer = Some(json!(true));
                        }
                        if !busy && ui.button("No").clicked() {
                            answer = Some(json!(false));
                        }
                    });
                }
                "select" => {
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for o in &opts {
                            let label = o["label"].as_str().unwrap_or("");
                            let hint = o["hint"].as_str().unwrap_or("");
                            let text = if hint.is_empty() { label.to_string() } else { format!("{label}  —  {hint}") };
                            if !busy && ui.button(text).clicked() {
                                answer = Some(o["value"].clone());
                            }
                        }
                    });
                }
                "multiselect" => {
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (i, o) in opts.iter().enumerate() {
                            let mut on = w.picks.contains(&i);
                            if ui.checkbox(&mut on, o["label"].as_str().unwrap_or("")).changed() {
                                if on {
                                    w.picks.insert(i);
                                } else {
                                    w.picks.remove(&i);
                                }
                            }
                        }
                    });
                    ui.add_space(6.0);
                    if !busy && primary(ui, p, "Continuar").clicked() {
                        let values: Vec<Value> = opts.iter().enumerate().filter(|(i, _)| w.picks.contains(i)).map(|(_, o)| o["value"].clone()).collect();
                        answer = Some(json!(values));
                    }
                }
                _ => {
                    if !busy && primary(ui, p, "Continuar").clicked() {
                        answer = Some(json!(true));
                    }
                }
            }
            if busy {
                ui.spinner();
            }
        });
    if let Some(a) = answer {
        app.wizard_answer(a);
    }
    if close || !open {
        app.wizard_cancel();
    }
}

fn open_url(url: &str) {
    let mut c = std::process::Command::new("rundll32");
    c.args(["url.dll,FileProtocolHandler", url]);
    crate::cli::hide_console(&mut c);
    let _ = c.spawn();
}

fn png_image(ui: &mut egui::Ui, name: String, bytes: &[u8], size: f32) {
    ui.add(egui::Image::from_bytes(name, bytes.to_vec()).fit_to_exact_size(Vec2::splat(size)));
}

fn qr_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(q) = &app.mgr.qr else { return };
    let (message, done, failed, png) = (q.message.clone(), q.done, q.failed, q.png.clone());
    let channel = q.channel.clone();
    let mut open = true;
    let mut close = false;
    egui::Window::new(format!("Vincular {channel}"))
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let (Some((n, bytes)), false) = (&png, done) {
                    Frame::new().fill(Color32::WHITE).inner_margin(Margin::same(10)).corner_radius(r(8)).show(ui, |ui| {
                        png_image(ui, format!("bytes://qr-{n}.png"), bytes, 280.0);
                    });
                    ui.add_space(6.0);
                } else if !done && !failed {
                    ui.spinner();
                }
                let color = if failed { p.red } else if done { p.green } else { p.text };
                ui.label(RichText::new(if done { "Vinculado".to_string() } else { message }).color(color));
                ui.add_space(6.0);
                if ui.button(if done || failed { "Cerrar" } else { "Cancelar" }).clicked() {
                    close = true;
                }
            });
        });
    if close || !open {
        if let Some(q) = &mut app.mgr.qr {
            q.cancelled = true;
        }
        app.mgr.qr = None;
    }
}

fn setup_code_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(c) = &app.mgr.setup_code else { return };
    let (code, gateway, png, expires) = (c.code.clone(), c.gateway.clone(), c.png.clone(), c.expires_ms);
    let mut open = true;
    egui::Window::new("Conectar un teléfono o equipo")
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.vertical_centered(|ui| {
                if let Some(bytes) = &png {
                    Frame::new().fill(Color32::WHITE).inner_margin(Margin::same(10)).corner_radius(r(8)).show(ui, |ui| {
                        png_image(ui, format!("bytes://setup-{}.png", code.len()), bytes, 260.0);
                    });
                }
                ui.add_space(6.0);
                ui.label("Escanéalo con la app de OpenClaw, o escribe este código:");
                ui.label(RichText::new(&code).monospace().strong());
                if ui.button("Copiar código").clicked() {
                    ui.ctx().copy_text(code.clone());
                }
                ui.label(RichText::new(format!("Dirección: {gateway}")).size(11.5).color(p.text2));
                if let Some(ms) = expires {
                    ui.label(RichText::new(format!("Vence: {}", time_label(ms))).size(11.5).color(p.text2));
                }
            });
        });
    if !open {
        app.mgr.setup_code = None;
    }
}

fn routine_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    if !app.mgr.forms.routine.open {
        return;
    }
    let agents = app.agents.clone();
    let mut open = true;
    let mut create = false;
    egui::Window::new("Nueva rutina")
        .open(&mut open)
        .collapsible(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let f = &mut app.mgr.forms.routine;
            egui::Grid::new("routine").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.add(TextEdit::singleline(&mut f.name).desired_width(320.0));
                ui.end_row();
                ui.label("Bot");
                let shown = agents.iter().find(|a| a.id == f.agent).map(|a| a.name.clone()).unwrap_or_else(|| "El predeterminado".into());
                egui::ComboBox::from_id_salt("r-agent").selected_text(shown).width(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.agent, String::new(), "El predeterminado");
                    for a in &agents {
                        ui.selectable_value(&mut f.agent, a.id.clone(), &a.name);
                    }
                });
                ui.end_row();
                ui.label("Cuándo");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut f.kind, 0, "Horario");
                    ui.selectable_value(&mut f.kind, 1, "Cada tanto");
                    ui.selectable_value(&mut f.kind, 2, "Una vez");
                });
                ui.end_row();
                match f.kind {
                    0 => {
                        ui.label("Horario");
                        ui.add(TextEdit::singleline(&mut f.cron).hint_text("0 9 * * * (cada día a las 9)").desired_width(320.0));
                        ui.end_row();
                        ui.label("Zona horaria");
                        ui.add(TextEdit::singleline(&mut f.tz).hint_text("Opcional, como America/Bogota").desired_width(320.0));
                        ui.end_row();
                    }
                    1 => {
                        ui.label("Cada (minutos)");
                        ui.add(TextEdit::singleline(&mut f.every_minutes).desired_width(120.0));
                        ui.end_row();
                    }
                    _ => {
                        ui.label("Fecha y hora");
                        ui.add(TextEdit::singleline(&mut f.at).hint_text("2026-12-24T18:00:00-05:00").desired_width(320.0));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut f.delete_after, "Borrarla después de ejecutarse");
                        ui.end_row();
                    }
                }
                ui.label("Qué debe hacer");
                ui.add(TextEdit::multiline(&mut f.message).desired_rows(3).desired_width(320.0));
                ui.end_row();
            });
            if let Some(e) = &f.error {
                ui.label(RichText::new(e).color(p.red));
            }
            ui.add_space(6.0);
            if primary(ui, p, "Crear rutina").clicked() {
                create = true;
            }
        });
    if create {
        app.create_routine();
        if app.mgr.forms.routine.error.is_none() {
            app.reload_routines();
        }
    }
    if !open {
        app.mgr.forms.routine.open = false;
    }
}

fn rename_device_windows(app: &mut App, ctx: &egui::Context) {
    for node in [false, true] {
        let slot = if node { app.mgr.forms.rename_node.take() } else { app.mgr.forms.rename_device.take() };
        let Some((id, mut name)) = slot else { continue };
        let mut open = true;
        let mut save = false;
        egui::Window::new("Renombrar equipo")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.add(TextEdit::singleline(&mut name).desired_width(280.0));
                if ui.button("Guardar").clicked() {
                    save = true;
                }
            });
        if save {
            if node {
                app.act("node.rename", json!({"nodeId": id, "displayName": name.trim()}), "Equipo renombrado");
            } else {
                app.act("device.pair.rename", json!({"deviceId": id, "label": name.trim()}), "Equipo renombrado");
            }
        } else if open {
            if node {
                app.mgr.forms.rename_node = Some((id, name));
            } else {
                app.mgr.forms.rename_device = Some((id, name));
            }
        }
    }
}
