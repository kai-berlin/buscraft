//! Colours, visuals and a few small shared widgets.
//! Light, flat look with a green title bar, white panels and thin grey dividers.

use egui::{Color32, CornerRadius, FontId, Margin, RichText, Stroke, TextStyle, Ui};

pub const HEADER: Color32 = Color32::from_rgb(0x3C, 0x9B, 0x34);
pub const HEADER_DARK: Color32 = Color32::from_rgb(0x2F, 0x82, 0x28);
pub const HEADER_TEXT: Color32 = Color32::WHITE;
pub const HEADER_MUTED: Color32 = Color32::from_rgb(0xDD, 0xF0, 0xDA);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x67, 0xC0);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xCC, 0xE4, 0xF7);
pub const LINE: Color32 = Color32::from_rgb(0xE3, 0xE3, 0xE3);
pub const OK: Color32 = Color32::from_rgb(0x2E, 0x9D, 0x4F);
pub const WARN: Color32 = Color32::from_rgb(0xD9, 0x8A, 0x10);
pub const BAD: Color32 = Color32::from_rgb(0xD1, 0x34, 0x38);
pub const MUTED: Color32 = Color32::from_rgb(0x76, 0x76, 0x76);

pub fn apply(ctx: &egui::Context, dark: bool, zoom: f32) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.selection.bg_fill = if dark { Color32::from_rgb(0x2A, 0x5A, 0x8A) } else { ACCENT_SOFT };
    v.selection.stroke = Stroke::new(1.0, if dark { Color32::WHITE } else { Color32::from_rgb(0x00, 0x4E, 0x96) });
    v.hyperlink_color = ACCENT;
    if !dark {
        v.panel_fill = Color32::WHITE;
        v.window_fill = Color32::WHITE;
        v.faint_bg_color = Color32::from_rgb(0xF7, 0xF7, 0xF7);
        v.extreme_bg_color = Color32::WHITE;
        v.override_text_color = Some(Color32::from_rgb(0x1F, 0x1F, 0x1F));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xCC, 0xCC, 0xCC));
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xF3, 0xF3, 0xF3);
        v.widgets.inactive.bg_fill = Color32::from_rgb(0xF3, 0xF3, 0xF3);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xE5, 0xF1, 0xFB);
        v.widgets.hovered.bg_fill = Color32::from_rgb(0xE5, 0xF1, 0xFB);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xB9, 0xD7, 0xF0));
    }
    for w in [&mut v.widgets.hovered, &mut v.widgets.inactive, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(2);
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(7.0, 4.0);
        s.spacing.button_padding = egui::vec2(7.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(13.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.0));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(18.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.5));
    });
    ctx.set_zoom_factor(zoom);
}

/// A tab on the green title bar.
pub fn header_tab(ui: &mut Ui, selected: bool, text: &str) -> egui::Response {
    let rt = RichText::new(text).size(13.5).color(if selected { Color32::from_rgb(0x1F, 0x1F, 0x1F) } else { HEADER_TEXT });
    ui.add(
        egui::Button::new(rt)
            .fill(if selected { Color32::WHITE } else { HEADER_DARK })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 })
            .min_size(egui::vec2(120.0, 28.0)),
    )
}

/// A tab on a light surface (view switcher, property tabs).
pub fn tab(ui: &mut Ui, selected: bool, text: &str) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let fg = if selected { if dark { Color32::WHITE } else { ACCENT } } else { ui.visuals().text_color() };
    let rt = RichText::new(text).color(fg);
    let r = ui.add(egui::Button::new(rt).fill(if selected && !dark { Color32::from_rgb(0xF2, 0xF8, 0xFD) } else { Color32::TRANSPARENT }).stroke(Stroke::NONE));
    if selected {
        let rect = r.rect;
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, ACCENT));
    }
    r
}

/// A flat toolbar button (icon + text, no frame until hovered).
pub fn tool(ui: &mut Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).size(12.5)).frame_when_inactive(false))
}

pub fn status_mark(ui: &mut Ui, ok: bool, applicable: bool) {
    if !applicable {
        ui.label(RichText::new("–").color(MUTED));
    } else if ok {
        ui.label(RichText::new("✔").color(OK));
    } else {
        ui.label(RichText::new("✖").color(BAD));
    }
}

pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).strong().size(13.0));
    ui.add_space(2.0);
}

pub fn card_frame(ui: &Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark { Color32::from_gray(32) } else { Color32::WHITE })
        .stroke(Stroke::new(1.0, if dark { Color32::from_gray(60) } else { LINE }))
        .inner_margin(Margin::same(8))
}

pub fn surface_frame(ui: &Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark { Color32::from_gray(27) } else { Color32::WHITE })
        .inner_margin(Margin::same(6))
}

/// A thin full-width divider line.
pub fn divider(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().hline(r.left()..=r.right(), y, Stroke::new(1.0, LINE));
    ui.add_space(1.0);
}
