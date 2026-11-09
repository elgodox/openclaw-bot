//! Everything drawn on screen.

use crate::app::{reconnect_note, time_label, App, InfoTab, ProfileForm};
use crate::gateway::ConnState;
use crate::look::{default_look, draw_bot, Palette, COLORS, SHAPES};
use crate::model::*;
use crate::settings::{Profile, ProfileKind, SecretKind, Theme, BotLook};
use eframe::egui::{
    self, Align, Button, Color32, CornerRadius, Frame, Key, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Vec2,
};

const SIDEBAR_WIDTH: f32 = 272.0;
const INFO_WIDTH: f32 = 340.0;

fn r(n: u8) -> CornerRadius {
    CornerRadius::same(n)
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let p = app.palette();
    crate::look::apply_theme(ui.ctx(), &p, app.settings.theme);
    let ctx = ui.ctx().clone();

    update_banner(app, ui, &p);

    egui::Panel::left("rail")
        .exact_size(crate::manager_ui::RAIL_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(4, 8)))
        .show(ui, |ui| crate::manager_ui::rail(app, ui, &p));

    if app.mgr.section != crate::manager::Section::Chats {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| crate::manager_ui::central(app, ui, &p));
        dialogs(app, &ctx, &p);
        crate::manager_ui::dialogs(app, &ctx, &p);
        toast(app, &ctx, &p);
        return;
    }

    egui::Panel::left("sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::same(12)))
        .show(ui, |ui| sidebar(app, ui, &p));

    let info_open = app.settings.info_open && app.is_connected() && app.selected_agent.is_some();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            // The conversation and the info column split this space by hand,
            // so wide content can't push one over the other.
            let full = ui.max_rect();
            let split = if info_open { full.right() - INFO_WIDTH } else { full.right() };
            let main = egui::Rect::from_min_max(full.min, egui::pos2(split, full.bottom()));
            let mut main_ui = ui.new_child(egui::UiBuilder::new().max_rect(main));
            main_ui.set_clip_rect(main);
            if app.is_connected() {
                chat_view(app, &mut main_ui, &p);
            } else {
                connection_view(app, &mut main_ui, &p);
            }
            if info_open {
                let side = egui::Rect::from_min_max(egui::pos2(split, full.top()), full.max);
                ui.painter().rect_filled(side, 0.0, p.sidebar);
                let mut info_ui = ui.new_child(egui::UiBuilder::new().max_rect(side.shrink(14.0)));
                info_ui.set_clip_rect(side);
                info_panel(app, &mut info_ui, &p);
            }
        });

    dialogs(app, &ctx, &p);
    crate::manager_ui::dialogs(app, &ctx, &p);
    toast(app, &ctx, &p);
    handle_dropped_files(app, &ctx);
}

fn handle_dropped_files(app: &mut App, ctx: &egui::Context) {
    let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
    for f in dropped {
        let path = f.path().to_path_buf();
        {
            app.add_file(&path);
        }
    }
}

fn toast(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some((text, ok, at)) = app.toast.clone() else { return };
    if at.elapsed().as_secs() >= 5 {
        app.toast = None;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    egui::Area::new(egui::Id::new("toast"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -96.0])
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.card)
                .stroke(Stroke::new(1.0, if ok { p.border } else { p.red }))
                .corner_radius(r(9))
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.label(RichText::new(text).color(if ok { p.text } else { p.red }));
                });
        });
}

// ---- sidebar -------------------------------------------------------------

fn bot_look(app: &App, id: &str) -> (usize, usize) {
    app.settings
        .looks
        .get(id)
        .map(|l| (l.shape, l.color))
        .unwrap_or_else(|| default_look(id))
}

fn sidebar(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        draw_bot(ui.painter(), rect, 0, 0, true);
        ui.add_space(2.0);
        ui.label(RichText::new("OpenClaw Bot").size(16.0).strong());
    });
    ui.add_space(4.0);
    connection_picker(app, ui, p);
    ui.add_space(8.0);

    let can_chat = app.is_connected() && app.selected_agent.is_some();
    ui.add_enabled_ui(can_chat, |ui| {
        let new_chat = primary_button(ui, p, "+  Nuevo chat", true);
        if new_chat.clicked() {
            if let Some(a) = app.selected_agent.clone() {
                app.create_chat(&a, None);
            }
        }
    });
    ui.add_space(6.0);
    ui.add(TextEdit::singleline(&mut app.search).hint_text("Buscar Bots").desired_width(f32::INFINITY));
    ui.add_space(8.0);

    let needle = app.search.to_lowercase();
    let filtered: Vec<Agent> = app
        .agents
        .iter()
        .filter(|a| needle.is_empty() || a.name.to_lowercase().contains(&needle))
        .cloned()
        .collect();

    ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 44.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if !app.is_connected() {
            ui.add_space(6.0);
            ui.label(RichText::new("Conéctate para ver tus Bots").size(12.5).color(p.text3));
            return;
        }
        if app.agents.is_empty() {
            ui.label(RichText::new("Cargando…").color(p.text3));
        }
        let pinned: Vec<Agent> = filtered.iter().filter(|a| app.settings.pinned.contains(&a.id)).cloned().collect();
        if !pinned.is_empty() {
            section_label(ui, p, "Bots fijados");
            for a in &pinned {
                bot_row(app, ui, p, a);
            }
            ui.add_space(6.0);
        }
        section_label(ui, p, "Bots");
        let visible: Vec<Agent> = filtered
            .iter()
            .filter(|a| !app.settings.pinned.contains(&a.id) && !app.settings.hidden.contains(&a.id))
            .cloned()
            .collect();
        for a in &visible {
            bot_row(app, ui, p, a);
        }
        let hidden: Vec<Agent> = filtered
            .iter()
            .filter(|a| app.settings.hidden.contains(&a.id) && !app.settings.pinned.contains(&a.id))
            .cloned()
            .collect();
        if !hidden.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new(format!("Bots ocultos ({})", hidden.len())).size(12.0).color(p.text2))
                .id_salt("hidden")
                .show(ui, |ui| {
                    for a in &hidden {
                        bot_row(app, ui, p, a);
                    }
                });
        }
    });

    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.horizontal(|ui| {
            if ui.add(flat_button(p, "⚙  Ajustes")).clicked() {
                app.show_settings = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let can_create = app.is_connected() && app.is_admin();
                if ui
                    .add_enabled(can_create, flat_button(p, "+ Nuevo Bot"))
                    .on_disabled_hover_text("Requiere una conexión con permiso de administrador")
                    .clicked()
                {
                    app.new_bot.open = true;
                    app.new_bot.error = None;
                }
            });
        });
        ui.add_space(4.0);
    });
}

fn section_label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).size(11.5).color(p.text3));
}

fn flat_button<'a>(p: &Palette, text: &'a str) -> Button<'a> {
    Button::new(RichText::new(text).size(13.0).color(p.text2)).frame(false).min_size(Vec2::new(0.0, 28.0))
}

fn primary_button(ui: &mut egui::Ui, p: &Palette, text: &str, wide: bool) -> egui::Response {
    let b = Button::new(RichText::new(text).color(Color32::WHITE).size(13.5))
        .fill(p.bubble)
        .corner_radius(r(8))
        .min_size(Vec2::new(if wide { ui.available_width() } else { 0.0 }, 34.0));
    ui.add(b)
}

fn connection_picker(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let prof = app.active_profile();
    let (note, ok) = reconnect_note(&app.conn);
    let dot = match app.conn {
        ConnState::Connected => p.green,
        ConnState::Pairing { .. } => p.amber,
        ConnState::AuthFailed(_) => p.red,
        ConnState::Disconnected(_) => p.red,
        _ => p.amber,
    };
    let frame = Frame::new()
        .fill(p.card)
        .corner_radius(r(8))
        .inner_margin(Margin::symmetric(9, 6))
        .stroke(Stroke::new(1.0, p.border));
    let inner = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let (c, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(c.center(), 4.0, dot);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(&prof.name).size(13.0).strong());
                let small = if ok {
                    match prof.kind {
                        ProfileKind::Local => "Local · en este equipo".to_string(),
                        ProfileKind::Direct => "Remoto · por red".to_string(),
                        ProfileKind::Ssh => "Remoto · por túnel SSH".to_string(),
                    }
                } else {
                    shorten(&note, 34)
                };
                ui.label(RichText::new(small).size(11.0).color(p.text2));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("▾").color(p.text3));
            });
        });
    });
    let resp = ui.interact(inner.response.rect, ui.id().with("conn-pick"), Sense::click());
    let resp = if ok { resp.on_hover_text(note) } else { resp };
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(240.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new("Conectar a").size(11.5).color(p.text3));
        let profiles: Vec<Profile> = app.settings.profiles.clone();
        for prof in profiles {
            let mark = if prof.id == app.settings.active { "●  " } else { "    " };
            let kind = match prof.kind {
                ProfileKind::Local => "local",
                ProfileKind::Direct => "remoto",
                ProfileKind::Ssh => "SSH",
            };
            if ui.button(format!("{mark}{} ({kind})", prof.name)).clicked() {
                let ctx = ui.ctx().clone();
                app.switch_profile(prof.id.clone(), &ctx);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Añadir OpenClaw remoto…").clicked() {
            app.profile_form = Some(ProfileForm {
                profile: Profile::default(),
                secret: String::new(),
                is_new: true,
            });
            ui.close();
        }
        if ui.button("Administrar conexiones…").clicked() {
            app.show_settings = true;
            ui.close();
        }
    });
}

fn bot_row(app: &mut App, ui: &mut egui::Ui, p: &Palette, a: &Agent) {
    let selected = app.selected_agent.as_deref() == Some(&a.id);
    let (shape, color) = bot_look(app, &a.id);
    let busy = app.agent_busy(&a.id);
    let waiting = app.approvals.iter().any(|ap| ap.agent_id == a.id);

    let row_h = 36.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, r(8), p.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, r(8), p.hover);
    }
    let icon = egui::Rect::from_min_size(rect.min + Vec2::new(6.0, 7.0), Vec2::splat(22.0));
    draw_bot(ui.painter(), icon, shape, color, true);
    let name = if a.emoji.is_empty() { a.name.clone() } else { format!("{} {}", a.emoji, a.name) };
    ui.painter().with_clip_rect(rect.shrink(4.0)).text(
        rect.min + Vec2::new(36.0, row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        shorten(&name, 26),
        egui::FontId::proportional(14.0),
        p.text,
    );
    if waiting || busy {
        let color = if waiting { p.amber } else { p.green };
        ui.painter().circle_filled(rect.right_center() - Vec2::new(14.0, 0.0), 4.0, color);
    }
    if resp.clicked() {
        app.select_agent(&a.id);
    }
    resp.context_menu(|ui| bot_menu(app, ui, a));

    if selected {
        ui.indent(("chats", &a.id), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let chats = app.chats_of(&a.id);
            if chats.is_empty() {
                ui.label(RichText::new("Sin chats todavía").size(12.0).color(p.text3));
            }
            for s in chats.iter().take(12) {
                chat_row(app, ui, p, s);
            }
            if chats.len() > 12 {
                ui.label(RichText::new(format!("y {} más", chats.len() - 12)).size(11.5).color(p.text3));
            }
        });
        ui.add_space(4.0);
    }
}

fn bot_menu(app: &mut App, ui: &mut egui::Ui, a: &Agent) {
    if ui.button("Nuevo chat").clicked() {
        app.create_chat(&a.id, None);
        ui.close();
    }
    let pinned = app.settings.pinned.contains(&a.id);
    if ui.button(if pinned { "Quitar de fijados" } else { "Fijar" }).clicked() {
        if pinned {
            app.settings.pinned.retain(|x| x != &a.id);
        } else {
            app.settings.pinned.push(a.id.clone());
        }
        app.settings.save();
        ui.close();
    }
    let hidden = app.settings.hidden.contains(&a.id);
    if ui.button(if hidden { "Mostrar en la lista" } else { "Ocultar" }).clicked() {
        if hidden {
            app.settings.hidden.retain(|x| x != &a.id);
        } else {
            app.settings.hidden.push(a.id.clone());
        }
        app.settings.save();
        ui.close();
    }
    if ui.button("Cambiar aspecto…").clicked() {
        app.look_for = Some(a.id.clone());
        ui.close();
    }
    ui.separator();
    if app.is_admin() {
        if ui.button("Cambiar nombre…").clicked() {
            app.rename_bot = Some((a.id.clone(), a.name.clone()));
            ui.close();
        }
        if ui.button("Eliminar Bot…").clicked() {
            app.delete_bot(&a.id);
            ui.close();
        }
    } else {
        ui.add_enabled(false, Button::new("Cambiar nombre… (requiere administrador)"));
    }
}

fn chat_row(app: &mut App, ui: &mut egui::Ui, p: &Palette, s: &SessionRow) {
    let selected = app.selected_session.as_deref() == Some(&s.key);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, r(6), p.hover);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, r(6), p.card);
    }
    let color = if selected { p.text } else { p.text2 };
    ui.painter().with_clip_rect(rect).text(
        rect.left_center() + Vec2::new(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        shorten(&s.shown_title(), 30),
        egui::FontId::proportional(12.5),
        color,
    );
    if s.active {
        ui.painter().circle_filled(rect.right_center() - Vec2::new(10.0, 0.0), 3.0, p.green);
    } else if s.unread {
        ui.painter().circle_filled(rect.right_center() - Vec2::new(10.0, 0.0), 3.0, p.accent);
    }
    if resp.clicked() {
        app.open_session(s.key.clone());
    }
    resp.context_menu(|ui| {
        if ui.button("Cambiar nombre…").clicked() {
            app.rename = Some((s.key.clone(), s.shown_title()));
            ui.close();
        }
        if !s.is_main && ui.button("Archivar").clicked() {
            app.archive_chat(&s.key);
            ui.close();
        }
    });
}

// ---- connection screens --------------------------------------------------

fn connection_view(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let ctx = ui.ctx().clone();
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(40.0));
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        draw_bot(ui.painter(), rect, 0, 0, true);
        ui.add_space(10.0);
        ui.label(RichText::new("Tus Bots, con OpenClaw").size(26.0).strong());
        ui.add_space(4.0);
        let prof = app.active_profile();
        ui.label(RichText::new(format!("Conexión: {}", prof.name)).size(14.0).color(p.text2));
        ui.add_space(18.0);

        let width = ui.available_width().min(460.0);
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Center), |ui| {
        ui.set_width(width);
        match app.conn.clone() {
            ConnState::Pairing { request_id } => pairing_card(ui, p, &request_id),
            ConnState::AuthFailed(msg) => {
                info_card(ui, p, "No se pudo entrar", &msg, true);
                ui.add_space(10.0);
                ui.label(RichText::new("Pega la clave de tu OpenClaw:").size(13.0).color(p.text2));
                ui.add(TextEdit::singleline(&mut app.secret_prompt).password(true).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, p, "Guardar y reintentar", false).clicked() {
                        app.save_secret_and_retry(&ctx);
                    }
                    if ui.button("Volver a pedir acceso").clicked() {
                        app.forget_device_access(&ctx);
                    }
                });
            }
            ConnState::Disconnected(msg) => {
                info_card(ui, p, "Sin conexión", &msg, true);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, p, "Reintentar ahora", false).clicked() {
                        app.connect(&ctx);
                    }
                    if ui.button("Ajustes de conexión").clicked() {
                        app.show_settings = true;
                    }
                });
                if prof.kind == ProfileKind::Local && !app.local.config_found {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("No encontré OpenClaw en este equipo. Instálalo o conéctate a uno remoto.")
                            .size(12.5)
                            .color(p.text2),
                    );
                }
            }
            _ => {
                ui.spinner();
                ui.add_space(6.0);
                ui.label(RichText::new(reconnect_note(&app.conn).0).color(p.text2));
            }
        }
        if let Some(note) = &app.local.note {
            if prof.kind == ProfileKind::Local {
                ui.add_space(8.0);
                ui.label(RichText::new(note).size(12.5).color(p.amber));
            }
        }
        });
    });
}

fn info_card(ui: &mut egui::Ui, p: &Palette, title: &str, body: &str, warn: bool) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, if warn { p.amber } else { p.border }))
        .corner_radius(r(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.label(RichText::new(body).size(13.0).color(p.text2));
        });
}

fn pairing_card(ui: &mut egui::Ui, p: &Palette, request_id: &str) {
    let id = if request_id.is_empty() { "<id>" } else { request_id };
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.amber))
        .corner_radius(r(10))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Aprueba este equipo").strong());
            ui.label(
                RichText::new("OpenClaw debe autorizar este equipo una vez. En el computador donde corre OpenClaw, ejecuta:")
                    .size(13.0)
                    .color(p.text2),
            );
            ui.add_space(6.0);
            let cmd = format!("openclaw devices approve {id}");
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&cmd).monospace().size(12.5));
                if ui.small_button("Copiar").clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("En cuanto lo apruebes, esta ventana se conecta sola.").size(12.0).color(p.text3));
        });
}

// ---- conversation --------------------------------------------------------

fn chat_view(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let Some(agent) = app.current_agent().cloned() else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Elige un Bot para empezar").color(p.text2));
        });
        return;
    };
    egui::Panel::top("chat-head")
        .exact_size(52.0)
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(18, 10)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                let (shape, color) = bot_look(app, &agent.id);
                draw_bot(ui.painter(), rect, shape, color, true);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(&agent.name).size(15.0).strong());
                    let title = app.current_session().map(|s| s.shown_title()).unwrap_or_else(|| "Chat nuevo".into());
                    ui.label(RichText::new(shorten(&title, 60)).size(11.5).color(p.text2));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(flat_button(p, "ⓘ")).on_hover_text("Detalles del Bot").clicked() {
                        app.settings.info_open = !app.settings.info_open;
                        app.settings.save();
                    }
                    let model = app.current_session().map(|s| s.model.clone()).filter(|m| !m.is_empty()).unwrap_or(agent.model.clone());
                    if !model.is_empty() {
                        ui.label(RichText::new(shorten(&model, 34)).size(11.5).color(p.text3));
                    }
                });
            });
        });

    egui::Panel::bottom("composer")
        .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 18, right: 18, top: 6, bottom: 14 }))
        .show(ui, |ui| composer(app, ui, p));

    egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
        let stick = app.stick_to_bottom;
        let area = ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true);
        let out = area.show(ui, |ui| {
            ui.add_space(10.0);
            let w = ui.available_width();
            let margin = ((w - 760.0) / 2.0).max(18.0);
            Frame::new().inner_margin(Margin::symmetric(margin as i8, 0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                messages(app, ui, p, &agent);
            });
            ui.add_space(6.0);
            if stick {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
        let _ = out;
        app.stick_to_bottom = false;
    });
}

fn messages(app: &mut App, ui: &mut egui::Ui, p: &Palette, agent: &Agent) {
    let key = app.selected_session.clone();
    let Some(chat) = key.as_ref().and_then(|k| app.chats.get(k)) else {
        welcome(ui, p, agent, app);
        return;
    };
    if chat.loading && chat.messages.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.spinner();
        });
        return;
    }
    if chat.messages.is_empty() && chat.run_id.is_none() && !chat.streaming && chat.error.is_none() {
        welcome(ui, p, agent, app);
        return;
    }

    let rows: Vec<ChatMessage> = chat.messages.clone();
    let (live, streaming) = (chat.live_text.clone(), chat.streaming);
    let tools = chat.tools.clone();
    let error = chat.error.clone();
    let run_active = chat.run_id.is_some() || streaming;
    let agent_id = agent.id.clone();
    let approvals: Vec<Approval> = app
        .approvals
        .iter()
        .filter(|a| a.agent_id.is_empty() || a.agent_id == agent_id)
        .cloned()
        .collect();

    for (i, m) in rows.iter().enumerate() {
        ui.push_id(i, |ui| match m.role {
            Role::User => user_bubble(ui, p, m),
            Role::Assistant => assistant_block(app, ui, p, &m.text, &m.attachments),
            Role::Notice => notice(ui, p, &m.text),
        });
        ui.add_space(10.0);
    }

    if run_active {
        if !tools.is_empty() {
            tool_steps(ui, p, &tools);
        }
        if live.trim().is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Pensando…").size(13.0).color(p.text2));
            });
        } else {
            ui.push_id("live", |ui| assistant_block(app, ui, p, &live, &[]));
        }
        ui.add_space(8.0);
    }

    for a in &approvals {
        approval_card(app, ui, p, a);
    }

    if let Some(e) = error {
        Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.red))
            .corner_radius(r(8))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.label(RichText::new(e).size(13.0).color(p.red));
            });
    }
}

fn welcome(ui: &mut egui::Ui, p: &Palette, agent: &Agent, app: &App) {
    ui.vertical_centered(|ui| {
        ui.add_space(70.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
        let (shape, color) = bot_look(app, &agent.id);
        draw_bot(ui.painter(), rect, shape, color, true);
        ui.add_space(8.0);
        ui.label(RichText::new(&agent.name).size(22.0).strong());
        ui.label(RichText::new("Escribe un mensaje para empezar").size(13.5).color(p.text2));
    });
}

fn user_bubble(ui: &mut egui::Ui, p: &Palette, m: &ChatMessage) {
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width() * 0.82, 0.0),
            Layout::top_down(Align::Max),
            |ui| {
                Frame::new()
                    .fill(if m.failed { p.red } else { p.bubble })
                    .corner_radius(CornerRadius { nw: 14, ne: 4, sw: 14, se: 14 })
                    .inner_margin(Margin::symmetric(13, 9))
                    .show(ui, |ui| {
                        if !m.text.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&m.text).size(14.0).color(p.on_bubble)).wrap());
                        }
                        for a in &m.attachments {
                            ui.label(RichText::new(format!("📎 {a}")).size(12.0).color(p.on_bubble));
                        }
                    });
                let mut footer = time_label(m.at_ms);
                if m.pending {
                    footer = "Enviando…".into();
                }
                if m.failed {
                    footer = "No se envió".into();
                }
                if !footer.is_empty() {
                    ui.label(RichText::new(footer).size(10.5).color(p.text3));
                }
            },
        );
    });
}

fn assistant_block(app: &mut App, ui: &mut egui::Ui, p: &Palette, text: &str, files: &[String]) {
    ui.scope(|ui| {
        ui.style_mut().override_text_style = Some(egui::TextStyle::Body);
        ui.style_mut().text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.5));
        egui_commonmark::CommonMarkViewer::new().show(ui, &mut app.markdown, text);
        for f in files {
            ui.label(RichText::new(format!("📎 {f}")).size(12.5).color(p.text2));
        }
    });
}

fn notice(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).size(12.0).color(p.text3));
    });
}

fn tool_steps(ui: &mut egui::Ui, p: &Palette, tools: &[ToolStep]) {
    let shown = tools.len().saturating_sub(6);
    Frame::new()
        .fill(p.card)
        .corner_radius(r(8))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            if shown > 0 {
                ui.label(RichText::new(format!("{shown} pasos anteriores")).size(11.5).color(p.text3));
            }
            for t in &tools[shown..] {
                tool_line(ui, p, t);
            }
        });
    ui.add_space(6.0);
}

fn tool_line(ui: &mut egui::Ui, p: &Palette, t: &ToolStep) {
    ui.horizontal(|ui| {
        if t.running {
            ui.add(egui::Spinner::new().size(12.0));
        } else if t.failed {
            ui.label(RichText::new("✕").size(12.0).color(p.red));
        } else {
            ui.label(RichText::new("✓").size(12.0).color(p.green));
        }
        ui.label(RichText::new(&t.name).size(12.5).strong());
        if !t.detail.is_empty() {
            ui.label(RichText::new(&t.detail).size(11.5).monospace().color(p.text3));
        }
    });
}

fn approval_card(app: &mut App, ui: &mut egui::Ui, p: &Palette, a: &Approval) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.amber))
        .corner_radius(r(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&a.title).strong());
            if !a.detail.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(shorten(&a.detail, 400)).monospace().size(12.5))
                        .wrap(),
                );
            }
            if !a.cwd.is_empty() {
                ui.label(RichText::new(format!("en {}", a.cwd)).size(11.5).color(p.text3));
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if primary_button(ui, p, "Permitir una vez", false).clicked() {
                    app.resolve_approval(&a.id, a.plugin, "allow-once");
                }
                if !a.plugin && ui.button("Permitir siempre").clicked() {
                    app.resolve_approval(&a.id, a.plugin, "allow-always");
                }
                if ui.button(RichText::new("Rechazar").color(p.red)).clicked() {
                    app.resolve_approval(&a.id, a.plugin, "deny");
                }
            });
        });
    ui.add_space(8.0);
}

fn composer(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let w = ui.available_width();
    let margin = ((w - 760.0) / 2.0).max(0.0);
    let run_active = app.current_chat().is_some_and(|c| c.run_id.is_some() || c.streaming);
    ui.horizontal(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width((w - margin * 2.0).min(760.0));
            if !app.attachments.is_empty() {
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, a) in app.attachments.iter().enumerate() {
                        Frame::new().fill(p.card).corner_radius(r(6)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                            ui.label(RichText::new(shorten(&a.name, 26)).size(12.0));
                            if ui.small_button("✕").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
                if let Some(i) = remove {
                    app.attachments.remove(i);
                }
                ui.add_space(4.0);
            }
            Frame::new()
                .fill(p.field)
                .corner_radius(r(14))
                .stroke(Stroke::new(1.0, p.border))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.add(flat_button(p, "+")).on_hover_text("Adjuntar archivos").clicked() {
                            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                                for path in paths {
                                    app.add_file(&path);
                                }
                            }
                        }
                        let id = ui.id().with("draft");
                        let send_width = 72.0;
                        let resp = ui.add_sized(
                            [(ui.available_width() - send_width - 8.0).max(60.0), 28.0],
                            TextEdit::multiline(&mut app.draft)
                                .id(id)
                                .hint_text("Escribe un mensaje")
                                .desired_rows(1)
                                .lock_focus(true)
                                .frame(Frame::NONE),
                        );
                        let enter = resp.has_focus()
                            && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                        if enter {
                            // Remove the newline Enter just typed.
                            if app.draft.ends_with('\n') {
                                app.draft.pop();
                            }
                            app.send_draft();
                        }
                        if run_active {
                            if ui.add(Button::new("Detener").corner_radius(r(8)).min_size(Vec2::new(send_width, 30.0))).clicked() {
                                app.stop_run();
                            }
                        } else {
                            let can = !app.draft.trim().is_empty() || !app.attachments.is_empty();
                            let b = Button::new(RichText::new("Enviar").color(Color32::WHITE))
                                .fill(p.bubble)
                                .corner_radius(r(8))
                                .min_size(Vec2::new(send_width, 30.0));
                            if ui.add_enabled(can, b).clicked() {
                                app.send_draft();
                            }
                        }
                    });
                });
        });
    });
}

// ---- right panel ---------------------------------------------------------

fn info_panel(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding = Vec2::new(6.0, 3.0);
        for (tab, label) in [
            (InfoTab::Activity, "Actividad"),
            (InfoTab::Files, "Archivos"),
            (InfoTab::Routines, "Rutinas"),
            (InfoTab::Skills, "Habilidades"),
            (InfoTab::Details, "Detalles"),
        ] {
            let selected = app.info_tab == tab;
            let text = RichText::new(label).size(12.0).color(if selected { p.text } else { p.text2 });
            let b = Button::new(text).frame(selected).fill(p.selected).corner_radius(r(6)).min_size(Vec2::new(0.0, 24.0));
            if ui.add(b).clicked() {
                app.info_tab = tab;
            }
        }
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.info_tab {
        InfoTab::Activity => activity_tab(app, ui, p),
        InfoTab::Files => files_tab(app, ui, p),
        InfoTab::Routines => routines_tab(app, ui, p),
        InfoTab::Skills => skills_tab(app, ui, p),
        InfoTab::Details => details_tab(app, ui, p),
    });
}

fn activity_tab(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let approvals: Vec<Approval> = app.approvals.clone();
    if !approvals.is_empty() {
        ui.label(RichText::new("Necesita tu respuesta").size(12.5).strong());
        for a in &approvals {
            approval_card(app, ui, p, a);
        }
    }
    let tools = app.current_chat().map(|c| c.tools.clone()).unwrap_or_default();
    if tools.is_empty() {
        ui.label(RichText::new("Sin actividad en esta respuesta").size(12.5).color(p.text3));
    } else {
        ui.spacing_mut().item_spacing.y = 3.0;
        for t in tools.iter().rev() {
            tool_line(ui, p, t);
        }
    }
}

fn files_tab(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let agent = app.info_agent.clone();
    if app.files.is_empty() {
        ui.label(RichText::new("Este Bot aún no tiene archivos propios").size(12.5).color(p.text3));
    }
    let files = app.files.clone();
    for f in &files {
        let label = if f.missing { format!("{} (vacío)", f.name) } else { f.name.clone() };
        if ui.button(RichText::new(label).size(12.5)).clicked() {
            if f.missing {
                app.file_editor = Some(crate::app::FileEditor {
                    agent: agent.clone(),
                    name: f.name.clone(),
                    text: String::new(),
                    loading: false,
                    dirty: true,
                });
            } else {
                app.open_file(&agent, &f.name);
            }
        }
    }
}

fn routines_tab(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let agent = app.info_agent.clone();
    let list: Vec<Routine> = app.routines.iter().filter(|r| r.agent_id == agent || r.agent_id.is_empty()).cloned().collect();
    if list.is_empty() {
        ui.label(RichText::new("Este Bot no tiene rutinas").size(12.5).color(p.text3));
        return;
    }
    for r_ in &list {
        Frame::new().fill(p.card).corner_radius(r(8)).inner_margin(Margin::same(9)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&r_.name).strong().size(13.0));
            if !r_.schedule.is_empty() {
                ui.label(RichText::new(&r_.schedule).monospace().size(11.5).color(p.text2));
            }
            ui.label(RichText::new(&r_.last).size(11.5).color(p.text3));
            ui.horizontal(|ui| {
                let label = if r_.enabled { "Pausar" } else { "Activar" };
                if ui.small_button(label).clicked() {
                    app.toggle_routine(r_);
                }
                if ui.small_button("Ejecutar ahora").clicked() {
                    app.run_routine(&r_.id);
                }
            });
        });
        ui.add_space(6.0);
    }
}

fn skills_tab(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    if app.skills.is_empty() {
        ui.label(RichText::new("Sin habilidades").size(12.5).color(p.text3));
        return;
    }
    let skills = app.skills.clone();
    for s in &skills {
        ui.horizontal(|ui| {
            let mut on = s.enabled;
            let enabled = app.is_admin();
            if ui.add_enabled(enabled, egui::Checkbox::without_text(&mut on)).changed() {
                app.toggle_skill(&s.key, on);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let color = if s.eligible { p.text } else { p.text3 };
                ui.label(RichText::new(format!("{} {}", s.emoji, s.name).trim().to_string()).size(12.5).color(color));
                let d = if !s.eligible && !s.missing.is_empty() {
                    format!("Falta {}", s.missing)
                } else {
                    shorten(&s.description, 70)
                };
                ui.label(RichText::new(d).size(11.0).color(p.text3));
            });
        });
        ui.add_space(3.0);
    }
}

fn details_tab(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let Some(agent) = app.current_agent().cloned() else { return };
    let (shape, color) = bot_look(app, &agent.id);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
        draw_bot(ui.painter(), rect, shape, color, true);
        ui.vertical(|ui| {
            ui.label(RichText::new(&agent.name).size(15.0).strong());
            if ui.small_button("Cambiar aspecto").clicked() {
                app.look_for = Some(agent.id.clone());
            }
        });
    });
    ui.add_space(8.0);
    labeled(ui, p, "Identificador", &agent.id);
    if !agent.workspace.is_empty() {
        labeled(ui, p, "Carpeta de trabajo", &agent.workspace);
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Modelo").size(12.0).color(p.text3));
    if app.is_admin() && !app.models.is_empty() {
        let current = if agent.model.is_empty() { "Predeterminado".to_string() } else { agent.model.clone() };
        let mut choice = None;
        egui::ComboBox::from_id_salt("model")
            .selected_text(shorten(&current, 34))
            .width(260.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(agent.model.is_empty(), "Predeterminado").clicked() {
                    choice = Some(None);
                }
                for m in app.models.clone() {
                    if ui.selectable_label(m == agent.model, &m).clicked() {
                        choice = Some(Some(m));
                    }
                }
            });
        if let Some(c) = choice {
            app.update_bot_model(&agent.id, c.as_deref());
        }
    } else {
        ui.label(RichText::new(if agent.model.is_empty() { "Predeterminado" } else { &agent.model }).size(13.0));
    }
}

fn labeled(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.label(RichText::new(label).size(12.0).color(p.text3));
    ui.add(egui::Label::new(RichText::new(value).size(12.5)).wrap());
    ui.add_space(4.0);
}

// ---- dialogs -------------------------------------------------------------

fn dialogs(app: &mut App, ctx: &egui::Context, p: &Palette) {
    settings_window(app, ctx, p);
    profile_window(app, ctx, p);
    new_bot_window(app, ctx);
    look_window(app, ctx);
    rename_windows(app, ctx);
    file_editor_window(app, ctx, p);
    delete_bot_window(app, ctx, p);
}

fn settings_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    if !app.show_settings {
        return;
    }
    let mut open = true;
    egui::Window::new("Ajustes")
        .open(&mut open)
        .collapsible(false)
        .default_width(500.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| settings_body(app, ui, ctx, p));
    if !open {
        app.show_settings = false;
        app.settings.save();
    }
}

/// Connections and app preferences; also shown inside Configurar.
pub fn settings_body(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, p: &Palette) {
    ui.label(RichText::new("Conexiones").strong());
    ui.add_space(4.0);
    let profiles = app.settings.profiles.clone();
    for prof in &profiles {
        Frame::new().fill(p.field).corner_radius(r(8)).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(&prof.name).strong().size(13.5));
                    ui.label(RichText::new(profile_summary(prof, app)).size(11.5).color(p.text2));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if prof.kind != ProfileKind::Local && ui.small_button("Quitar").clicked() {
                        app.delete_profile(&prof.id, ctx);
                    }
                    if ui.small_button("Editar").clicked() {
                        app.profile_form = Some(ProfileForm { profile: prof.clone(), secret: String::new(), is_new: false });
                    }
                    if prof.id != app.settings.active && ui.small_button("Usar").clicked() {
                        app.switch_profile(prof.id.clone(), ctx);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
    if ui.button("Añadir OpenClaw remoto…").clicked() {
        app.profile_form = Some(ProfileForm { profile: Profile::default(), secret: String::new(), is_new: true });
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Apariencia").strong());
    ui.horizontal(|ui| {
        if ui.selectable_value(&mut app.settings.theme, Theme::Dark, "Oscura").changed() {
            app.settings.save();
        }
        if ui.selectable_value(&mut app.settings.theme, Theme::Light, "Clara").changed() {
            app.settings.save();
        }
    });
    ui.add_space(6.0);
    if ui.checkbox(&mut app.settings.notifications, "Avisarme cuando un Bot necesite respuesta").changed() {
        app.settings.save();
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Este equipo").strong());
    ui.label(RichText::new(format!("Dispositivo: {}…", &app.identity.device_id[..16])).size(12.0).color(p.text2));
    if !app.store.keychain_ok {
        ui.label(RichText::new("No hay almacén seguro: las claves solo duran esta sesión.").size(12.0).color(p.amber));
    }
    if ui.small_button("Olvidar el acceso de esta conexión").clicked() {
        app.forget_device_access(ctx);
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Actualizaciones").strong());
    ui.label(RichText::new(format!("Versión instalada: {}", crate::update::CURRENT)).size(12.0).color(p.text2));
    let state = app.updater.state();
    let note = match &state {
        crate::update::State::Idle => None,
        crate::update::State::Checking => Some(("Buscando…".to_string(), p.text2)),
        crate::update::State::UpToDate => Some(("Tienes la última versión.".to_string(), p.green)),
        crate::update::State::Available(r) => Some((format!("Hay una versión nueva: {}", r.tag_name), p.amber)),
        crate::update::State::Downloading { version, .. } => Some((format!("Descargando {version}…"), p.text2)),
        crate::update::State::Restarting => Some(("Reiniciando…".to_string(), p.text2)),
        crate::update::State::Failed(e) => Some((e.clone(), p.amber)),
    };
    if let Some((text, color)) = note {
        ui.label(RichText::new(text).size(12.0).color(color));
    }
    ui.horizontal(|ui| {
        if let crate::update::State::Available(r) = &state {
            if ui.button("Actualizar ahora").clicked() {
                app.updater.install(r.clone(), ctx);
            }
        }
        if ui.small_button("Buscar actualizaciones").clicked() {
            app.updater.dismissed = None;
            app.updater.check_now(ctx);
        }
        ui.hyperlink_to("Ver versiones", format!("https://github.com/{}/releases", crate::update::REPO));
    });
}

/// A strip across the top when a newer version is out or being installed.
fn update_banner(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    use crate::update::State;
    let state = app.updater.state();
    let show = match &state {
        State::Available(r) => app.updater.dismissed.as_deref() != Some(r.tag_name.as_str()),
        State::Downloading { .. } | State::Restarting => true,
        _ => false,
    };
    if !show {
        return;
    }
    let ctx = ui.ctx().clone();
    egui::Panel::top("update-banner")
        .frame(Frame::new().fill(p.accent).inner_margin(Margin::symmetric(14, 7)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                match &state {
                    State::Available(r) => {
                        ui.label(RichText::new(format!("Hay una versión nueva de OpenClaw Bot ({}).", r.tag_name)).strong());
                        if ui.button("Actualizar ahora").clicked() {
                            app.updater.install(r.clone(), &ctx);
                        }
                        if !r.html_url.is_empty() {
                            ui.hyperlink_to(RichText::new("Novedades").color(Color32::WHITE), &r.html_url);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Más tarde").clicked() {
                                app.updater.dismissed = Some(r.tag_name.clone());
                            }
                        });
                    }
                    State::Downloading { version, done, total } => {
                        ui.label(RichText::new(format!("Descargando {version}…")).strong());
                        if *total > 0 {
                            let f = *done as f32 / *total as f32;
                            ui.add(egui::ProgressBar::new(f).desired_width(220.0).show_percentage());
                        } else {
                            ui.label(format!("{:.1} MB", *done as f64 / 1_048_576.0));
                        }
                    }
                    _ => {
                        ui.label(RichText::new("Instalando la versión nueva y reiniciando…").strong());
                    }
                }
            });
        });
}

fn profile_summary(prof: &Profile, app: &App) -> String {
    match prof.kind {
        ProfileKind::Local => format!("En este equipo · puerto {}", app.local.port),
        ProfileKind::Direct => if prof.url.trim().is_empty() { "Sin dirección".to_string() } else { prof.url.clone() },
        ProfileKind::Ssh => format!("Túnel SSH a {}", prof.ssh_target),
    }
}

fn profile_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(mut form) = app.profile_form.take() else { return };
    let mut open = true;
    let mut submit = false;
    let has_secret = app.profile_has_secret(&form.profile);
    egui::Window::new(if form.is_new { "Añadir OpenClaw remoto" } else { "Editar conexión" })
        .open(&mut open)
        .collapsible(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Grid::new("profile").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.add(TextEdit::singleline(&mut form.profile.name).desired_width(280.0));
                ui.end_row();
                if form.profile.kind != ProfileKind::Local {
                    ui.label("Cómo llegar");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut form.profile.kind, ProfileKind::Direct, "Por red o Tailscale");
                        ui.selectable_value(&mut form.profile.kind, ProfileKind::Ssh, "Túnel SSH");
                    });
                    ui.end_row();
                }
                match form.profile.kind {
                    ProfileKind::Local => {}
                    ProfileKind::Direct => {
                        ui.label("Dirección");
                        ui.add(TextEdit::singleline(&mut form.profile.url)
                            .hint_text("wss://mi-equipo.tailnet.ts.net o ws://192.168.1.20:18789")
                            .desired_width(280.0));
                        ui.end_row();
                        ui.label("Huella TLS");
                        ui.add(TextEdit::singleline(&mut form.profile.tls_fingerprint)
                            .hint_text("Solo con certificado propio (sha256)")
                            .desired_width(280.0));
                        ui.end_row();
                    }
                    ProfileKind::Ssh => {
                        ui.label("Equipo SSH");
                        ui.add(TextEdit::singleline(&mut form.profile.ssh_target).hint_text("usuario@equipo o usuario@equipo:2222").desired_width(280.0));
                        ui.end_row();
                        ui.label("Clave SSH");
                        ui.add(TextEdit::singleline(&mut form.profile.ssh_identity).hint_text("Opcional, ruta al archivo").desired_width(280.0));
                        ui.end_row();
                        ui.label("Puerto remoto");
                        let mut text = form.profile.remote_port.to_string();
                        if ui.add(TextEdit::singleline(&mut text).desired_width(80.0)).changed() {
                            if let Ok(v) = text.trim().parse() {
                                form.profile.remote_port = v;
                            }
                        }
                        ui.end_row();
                    }
                }
                ui.label("Acceso");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut form.profile.secret_kind, SecretKind::Token, "Token");
                    ui.selectable_value(&mut form.profile.secret_kind, SecretKind::Password, "Contraseña");
                    ui.selectable_value(&mut form.profile.secret_kind, SecretKind::None, "Ninguno");
                });
                ui.end_row();
                if form.profile.secret_kind != SecretKind::None {
                    ui.label("Clave");
                    let hint = if has_secret { "Guardada — escribe para cambiarla" } else if form.profile.kind == ProfileKind::Local { "Se usa la de este equipo" } else { "Token o contraseña de OpenClaw" };
                    ui.add(TextEdit::singleline(&mut form.secret).password(true).hint_text(hint).desired_width(280.0));
                    ui.end_row();
                }
                ui.label("");
                ui.checkbox(&mut form.profile.admin, "Permitir administrar (crear Bots, activar habilidades)");
                ui.end_row();
            });
            ui.add_space(6.0);
            if form.profile.kind != ProfileKind::Local {
                ui.label(RichText::new("La primera vez, OpenClaw te pedirá aprobar este equipo.").size(12.0).color(p.text3));
                if form.profile.kind == ProfileKind::Ssh {
                    ui.label(RichText::new("El túnel usa tu configuración de ssh; la llave debe estar lista sin pedir contraseña.").size(12.0).color(p.text3));
                }
            }
            ui.add_space(8.0);
            if primary_button(ui, p, "Guardar y conectar", false).clicked() {
                submit = true;
            }
        });
    if submit {
        let valid = match form.profile.kind {
            ProfileKind::Direct => !form.profile.url.trim().is_empty(),
            ProfileKind::Ssh => !form.profile.ssh_target.trim().is_empty(),
            ProfileKind::Local => true,
        };
        if valid {
            app.save_profile(form, ctx);
            return;
        }
        app.toast("Falta la dirección", false);
    }
    if open {
        app.profile_form = Some(form);
    }
}

fn new_bot_window(app: &mut App, ctx: &egui::Context) {
    if !app.new_bot.open {
        return;
    }
    let mut open = true;
    let mut create = false;
    egui::Window::new("Nuevo Bot")
        .open(&mut open)
        .collapsible(false)
        .default_width(380.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Nombre");
            ui.add(TextEdit::singleline(&mut app.new_bot.name).desired_width(f32::INFINITY));
            ui.add_space(4.0);
            ui.label("Carpeta de trabajo (opcional)");
            ui.add(TextEdit::singleline(&mut app.new_bot.workspace).hint_text("En el equipo donde corre OpenClaw").desired_width(f32::INFINITY));
            if let Some(e) = &app.new_bot.error {
                ui.colored_label(egui::Color32::from_rgb(0xe0, 0x56, 0x56), e);
            }
            ui.add_space(8.0);
            if ui.button("Crear").clicked() {
                create = true;
            }
        });
    if create {
        app.create_bot();
    }
    if !open {
        app.new_bot.open = false;
    }
}

fn look_window(app: &mut App, ctx: &egui::Context) {
    let Some(id) = app.look_for.clone() else { return };
    let mut open = true;
    egui::Window::new("Aspecto del Bot")
        .open(&mut open)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let (mut shape, mut color) = bot_look(app, &id);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(320.0);
                for s in 0..SHAPES {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
                    if s == shape {
                        ui.painter().rect_filled(rect, r(8), ui.visuals().selection.bg_fill.gamma_multiply(0.35));
                    }
                    draw_bot(ui.painter(), rect.shrink(4.0), s, color, true);
                    if resp.clicked() {
                        shape = s;
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (i, c) in COLORS.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                    ui.painter().circle_filled(rect.center(), if i == color { 11.0 } else { 9.0 }, *c);
                    if resp.clicked() {
                        color = i;
                    }
                }
            });
            app.settings.looks.insert(id.clone(), BotLook { shape, color });
        });
    if !open {
        app.look_for = None;
        app.settings.save();
    }
}

fn rename_windows(app: &mut App, ctx: &egui::Context) {
    if let Some((key, mut text)) = app.rename.clone() {
        let mut open = true;
        let mut ok = false;
        egui::Window::new("Cambiar nombre del chat")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add(TextEdit::singleline(&mut text).desired_width(300.0));
                ok = ui.button("Guardar").clicked();
            });
        if ok && !text.trim().is_empty() {
            app.rename_chat(&key, text.trim());
            app.rename = None;
        } else if !open {
            app.rename = None;
        } else {
            app.rename = Some((key, text));
        }
    }
    if let Some((id, mut text)) = app.rename_bot.clone() {
        let mut open = true;
        let mut ok = false;
        egui::Window::new("Cambiar nombre del Bot")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add(TextEdit::singleline(&mut text).desired_width(300.0));
                ok = ui.button("Guardar").clicked();
            });
        if ok && !text.trim().is_empty() {
            app.update_bot_name(&id, text.trim());
            app.rename_bot = None;
        } else if !open {
            app.rename_bot = None;
        } else {
            app.rename_bot = Some((id, text));
        }
    }
}

fn file_editor_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    if app.file_editor.is_none() {
        return;
    }
    let mut open = true;
    let mut save = false;
    let title = app.file_editor.as_ref().map(|e| e.name.clone()).unwrap_or_default();
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .default_size([620.0, 480.0])
        .show(ctx, |ui| {
            let ed = app.file_editor.as_mut().unwrap();
            if ed.loading {
                ui.spinner();
                return;
            }
            ScrollArea::vertical().max_height(ui.available_height() - 40.0).show(ui, |ui| {
                let resp = ui.add(
                    TextEdit::multiline(&mut ed.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(20),
                );
                if resp.changed() {
                    ed.dirty = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let dirty = ed.dirty;
                if ui.add_enabled(dirty, Button::new(RichText::new("Guardar").color(Color32::WHITE)).fill(p.bubble)).clicked() {
                    save = true;
                }
                if !dirty {
                    ui.label(RichText::new("Sin cambios").size(12.0).color(p.text3));
                }
            });
        });
    if save {
        app.save_file();
    }
    if !open {
        app.file_editor = None;
    }
}

fn delete_bot_window(app: &mut App, ctx: &egui::Context, p: &Palette) {
    let Some(id) = app.confirm_delete_bot.clone() else { return };
    let name = app.agents.iter().find(|a| a.id == id).map(|a| a.name.clone()).unwrap_or(id.clone());
    let mut close = false;
    egui::Window::new("Eliminar Bot")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.label(format!("¿Eliminar a {name}? Sus chats dejan de estar disponibles."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Eliminar").color(p.red)).clicked() {
                    app.delete_bot_now(&id);
                    close = true;
                }
                if ui.button("Cancelar").clicked() {
                    app.confirm_delete_bot = None;
                }
            });
        });
    let _ = close;
}
