use eframe::egui::{self, Color32, FontId, Stroke, TextStyle};
pub const BG: Color32 = Color32::from_rgb(246, 248, 252);
pub const CANVAS: Color32 = Color32::from_rgb(238, 242, 249);
pub const WHITE: Color32 = Color32::WHITE;
pub const INK: Color32 = Color32::from_rgb(28, 40, 61);
pub const MUTED: Color32 = Color32::from_rgb(107, 121, 143);
pub const BORDER: Color32 = Color32::from_rgb(223, 229, 239);
pub const BLUE: Color32 = Color32::from_rgb(63, 98, 221);
pub const BLUE_SOFT: Color32 = Color32::from_rgb(233, 239, 255);
pub const GREEN: Color32 = Color32::from_rgb(24, 156, 119);
pub const RED: Color32 = Color32::from_rgb(202, 68, 82);
pub const AMBER: Color32 = Color32::from_rgb(174, 119, 21);

pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.set_global_style(egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::proportional(24.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into(),
        visuals: egui::Visuals {
            override_text_color: Some(INK),
            panel_fill: BG,
            window_fill: WHITE,
            extreme_bg_color: BG,
            selection: egui::style::Selection {
                bg_fill: BLUE_SOFT,
                stroke: Stroke::new(1.0, BLUE),
            },
            ..egui::Visuals::light()
        },
        spacing: egui::Spacing {
            item_spacing: egui::vec2(10.0, 10.0),
            button_padding: egui::vec2(14.0, 10.0),
            ..Default::default()
        },
        ..Default::default()
    });
}

pub fn small_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(10.0).strong().color(MUTED));
    ui.add_space(4.0);
}
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0).color(MUTED));
}
pub fn monitor(p: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let r = egui::Rect::from_center_size(center, egui::vec2(size, size * 0.65));
    p.rect_stroke(r, 3.0, Stroke::new(1.7, color), egui::StrokeKind::Inside);
    p.line_segment(
        [
            egui::pos2(center.x, r.bottom()),
            egui::pos2(center.x, r.bottom() + size * 0.16),
        ],
        Stroke::new(1.7, color),
    );
    p.line_segment(
        [
            egui::pos2(center.x - size * 0.18, r.bottom() + size * 0.17),
            egui::pos2(center.x + size * 0.18, r.bottom() + size * 0.17),
        ],
        Stroke::new(1.7, color),
    );
}
