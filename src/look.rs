//! Colours, fonts and the little Bot faces.

use eframe::egui::{self, epaint, Color32, CornerRadius, Pos2, Rect, Stroke, Vec2};
use crate::settings::Theme;

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub field: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub bubble: Color32,
    pub on_bubble: Color32,
    pub green: Color32,
    pub red: Color32,
    pub amber: Color32,
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            bg: Color32::from_rgb(0x14, 0x14, 0x14),
            sidebar: Color32::from_rgb(0x18, 0x18, 0x18),
            card: Color32::from_rgb(0x1f, 0x1f, 0x1f),
            field: Color32::from_rgb(0x24, 0x24, 0x24),
            hover: Color32::from_rgb(0x28, 0x28, 0x28),
            selected: Color32::from_rgb(0x2e, 0x2e, 0x2e),
            border: Color32::from_rgb(0x33, 0x33, 0x33),
            text: Color32::from_rgb(0xf5, 0xf5, 0xf5),
            text2: Color32::from_rgb(0xa6, 0xa6, 0xa6),
            text3: Color32::from_rgb(0x78, 0x78, 0x78),
            accent: Color32::from_rgb(0x59, 0x9c, 0xe7),
            bubble: Color32::from_rgb(0x12, 0x6f, 0xd4),
            on_bubble: Color32::WHITE,
            green: Color32::from_rgb(0x3f, 0xa2, 0x66),
            red: Color32::from_rgb(0xe0, 0x56, 0x56),
            amber: Color32::from_rgb(0xd9, 0xa4, 0x41),
        },
        Theme::Light => Palette {
            bg: Color32::from_rgb(0xff, 0xff, 0xff),
            sidebar: Color32::from_rgb(0xf5, 0xf5, 0xf5),
            card: Color32::from_rgb(0xf7, 0xf7, 0xf7),
            field: Color32::from_rgb(0xf0, 0xf0, 0xf0),
            hover: Color32::from_rgb(0xea, 0xea, 0xea),
            selected: Color32::from_rgb(0xe2, 0xe2, 0xe2),
            border: Color32::from_rgb(0xdc, 0xdc, 0xdc),
            text: Color32::from_rgb(0x1c, 0x1c, 0x1c),
            text2: Color32::from_rgb(0x55, 0x55, 0x55),
            text3: Color32::from_rgb(0x88, 0x88, 0x88),
            accent: Color32::from_rgb(0x16, 0x80, 0xf3),
            bubble: Color32::from_rgb(0x12, 0x6f, 0xd4),
            on_bubble: Color32::WHITE,
            green: Color32::from_rgb(0x2e, 0x8b, 0x57),
            red: Color32::from_rgb(0xc9, 0x3c, 0x3c),
            amber: Color32::from_rgb(0xb8, 0x86, 0x1b),
        },
    }
}

pub fn apply_theme(ctx: &egui::Context, p: &Palette, theme: Theme) {
    let mut v = if theme == Theme::Dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.bubble;
    v.selection.stroke = Stroke::new(1.0, p.on_bubble);
    v.hyperlink_color = p.accent;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.weak_bg_fill = p.card;
    v.widgets.inactive.weak_bg_fill = p.field;
    v.widgets.inactive.bg_fill = p.field;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.active.weak_bg_fill = p.selected;
    v.widgets.active.bg_fill = p.selected;
    v.widgets.inactive.corner_radius = CornerRadius::same(7);
    v.widgets.hovered.corner_radius = CornerRadius::same(7);
    v.widgets.active.corner_radius = CornerRadius::same(7);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(7);
    v.window_corner_radius = CornerRadius::same(12);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 6.0);
    });
}

/// Uses the Windows fonts when present, the built-in font otherwise.
pub fn load_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let mut add = |name: &str, file: &str, front: bool, family: egui::FontFamily| {
        if let Ok(bytes) = std::fs::read(format!(r"{dir}\Fonts\{file}")) {
            fonts
                .font_data
                .insert(name.to_string(), egui::FontData::from_owned(bytes).into());
            let list = fonts.families.entry(family).or_default();
            if front {
                list.insert(0, name.to_string());
            } else {
                list.push(name.to_string());
            }
        }
    };
    add("ui", "segoeui.ttf", true, egui::FontFamily::Proportional);
    add("symbols", "seguisym.ttf", false, egui::FontFamily::Proportional);
    add("emoji", "seguiemj.ttf", false, egui::FontFamily::Proportional);
    add("code", "consola.ttf", true, egui::FontFamily::Monospace);
    ctx.set_fonts(fonts);
}

pub const SHAPES: usize = 10;
pub const COLORS: [Color32; 8] = [
    Color32::from_rgb(0x59, 0x9c, 0xe7),
    Color32::from_rgb(0xe8, 0x8a, 0x5d),
    Color32::from_rgb(0x6f, 0xc2, 0x8c),
    Color32::from_rgb(0xc9, 0x7d, 0xd8),
    Color32::from_rgb(0xe6, 0xb9, 0x4f),
    Color32::from_rgb(0xe0, 0x6f, 0x8b),
    Color32::from_rgb(0x4f, 0xbf, 0xbf),
    Color32::from_rgb(0xb7, 0xb7, 0xb7),
];

/// Picks a look from the Bot's id, so each one stays the same.
pub fn default_look(id: &str) -> (usize, usize) {
    let h = id.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    ((h % SHAPES as u32) as usize, ((h >> 8) % COLORS.len() as u32) as usize)
}

fn outline(shape: usize, n: usize) -> Vec<Vec2> {
    use std::f32::consts::TAU;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let a = i as f32 / n as f32 * TAU;
        let (c, s) = (a.cos(), a.sin());
        let r = match shape {
            0 => 1.0,                                                  // circle
            1 => 1.0 / (c.abs().powi(4) + s.abs().powi(4)).powf(0.25), // squircle
            2 => {
                // hexagon
                let sector = (a / (TAU / 6.0)).floor();
                let mid = sector * TAU / 6.0 + TAU / 12.0;
                0.92 / (a - mid).cos() * (TAU / 12.0).cos()
            }
            3 => 1.0 / (c.abs() + s.abs()),                  // diamond
            4 => 0.74 + 0.26 * (a * 5.0 - TAU / 4.0).cos(),  // star
            5 => 0.78 + 0.22 * (a * 8.0).cos(),              // flower
            6 => 0.86 + 0.14 * (a - TAU * 0.25).sin(),       // egg
            7 => {
                // arch: round on top, flat at the bottom
                if s > 0.55 { 0.72 / s } else { 1.0 }
            }
            8 => 0.82 + 0.18 * (a * 3.0).cos().abs().powi(2), // clover
            _ => 0.88 + 0.12 * (a * 6.0).cos().powi(2),       // pebble
        };
        pts.push(Vec2::new(c * r, s * r));
    }
    pts
}

/// Draws a Bot: a soft shape with two eyes and a small smile.
pub fn draw_bot(painter: &egui::Painter, rect: Rect, shape: usize, color: usize, face: bool) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let fill = COLORS[color % COLORS.len()];
    let pts = outline(shape % SHAPES, 48);
    let mut mesh = epaint::Mesh::default();
    mesh.colored_vertex(c, fill);
    for p in &pts {
        mesh.colored_vertex(c + *p * r, fill);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
    if face && r >= 7.0 {
        let eye = Color32::from_rgb(0x1c, 0x1c, 0x1c);
        let ex = r * 0.34;
        let ey = -r * 0.14;
        let er = (r * 0.13).max(1.3);
        painter.circle_filled(c + Vec2::new(-ex, ey), er, eye);
        painter.circle_filled(c + Vec2::new(ex, ey), er, eye);
        let smile: Vec<Pos2> = (0..=8)
            .map(|i| {
                let t = i as f32 / 8.0;
                let a = std::f32::consts::PI * (0.2 + 0.6 * t);
                c + Vec2::new(a.cos() * r * 0.3, r * 0.14 + a.sin() * r * 0.2)
            })
            .collect();
        painter.add(egui::Shape::line(smile, Stroke::new((r * 0.09).max(1.0), eye)));
    }
}
