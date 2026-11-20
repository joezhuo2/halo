use crate::theme::Theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Ui, Vec2};

pub use egui_phosphor::regular as icons;

pub const ANIM: f32 = 0.14;

static NOTICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn notify(text: impl Into<String>) {
    NOTICES.lock().unwrap().push(text.into());
}

pub fn take_notices() -> Vec<String> {
    std::mem::take(&mut *NOTICES.lock().unwrap())
}

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

pub fn tabs(ui: &mut Ui, theme: &Theme, id: Id, labels: &[&str], active: &mut usize) {
    let mut underline = None;
    for (index, label) in labels.iter().enumerate() {
        let galley = ui.painter().layout_no_wrap((*label).to_owned(), FontId::proportional(11.5), Color32::PLACEHOLDER);
        let (r, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 20.0, 32.0), Sense::click());
        if resp.clicked() {
            *active = index;
        }
        let a = anim(ui, resp.id.with("active"), *active == index);
        let h = anim(ui, resp.id.with("hover"), resp.hovered());
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, mix(theme.text_dim, theme.text, a.max(h)));
        if *active == index {
            underline = Some(r);
        }
    }
    if let Some(r) = underline {
        let left = ui.ctx().animate_value_with_time(id.with("left"), r.left() + 10.0, ANIM);
        let right = ui.ctx().animate_value_with_time(id.with("right"), r.right() - 10.0, ANIM);
        let bar = Rect::from_min_max(egui::pos2(left, r.bottom() - 4.0), egui::pos2(right, r.bottom() - 2.5));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.accent);
    }
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
