use crate::theme::Theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Ui, Vec2};

pub use egui_phosphor::regular as icons;

pub const ANIM: f32 = 0.14;

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

pub fn anim(ui: &Ui, id: Id, on: bool) -> f32 {
    ui.ctx().animate_bool_with_time(id, on, ANIM)
}

pub fn row_bg(ui: &Ui, theme: &Theme, rect: Rect, id: Id, hovered: bool, selected: bool) {
    let h = anim(ui, id.with("hover"), hovered);
    let s = anim(ui, id.with("selected"), selected);
    let color = mix(mix(Color32::TRANSPARENT, theme.hover, h), theme.selection, s);
    if color.a() > 0 {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, color);
    }
}

pub fn icon_button(ui: &mut Ui, theme: &Theme, icon: &str, size: f32, active: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let h = anim(ui, resp.id.with("hover"), resp.hovered());
    let a = anim(ui, resp.id.with("active"), active);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            CornerRadius::same(4),
            mix(Color32::TRANSPARENT, theme.hover, h),
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(size * 0.62),
            mix(mix(theme.text_dim, theme.text, h), theme.accent, a),
        );
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

pub fn fade_in(ui: &Ui, id: Id) -> f32 {
    let now = ui.input(|i| i.time);
    let start = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    let t = ((now - start) as f32 / (ANIM * 1.5)).clamp(0.0, 1.0);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    t
}

pub fn with_alpha(color: Color32, t: f32) -> Color32 {
    color.gamma_multiply(t.clamp(0.0, 1.0))
}

pub fn section_header(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).color(theme.text_dim));
    });
    ui.add_space(4.0);
}
