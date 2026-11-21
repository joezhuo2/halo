use crate::theme::Theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Ui, Vec2};

pub use egui_phosphor::regular as icons;

pub const ANIM: f32 = 0.14;
pub const FIELD: egui::Margin = egui::Margin::symmetric(8, 5);

static NOTICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn open_url(url: &str) {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    #[cfg(windows)]
    let launched = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let launched = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let launched = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = launched {
        notify(format!("Could not open {url}: {error}"));
    }
}

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

pub fn modal(ctx: &egui::Context, id: impl egui::AsId) -> egui::Modal {
    egui::Modal::new(Id::new(id)).frame(egui::Frame::popup(&ctx.global_style()).inner_margin(18.0))
}

pub fn outline(ui: &mut Ui) {
    let border = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.visuals_mut().widgets.inactive.bg_stroke = border;
}

pub fn menu_pad(ui: &mut Ui) {
    ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
}

fn logo_coverage(x: f32, y: f32) -> (bool, bool) {
    const APOTHEM: f32 = 0.667;
    const ROUND: f32 = 0.13;
    const PETAL: f32 = 0.47;
    const COS: f32 = 0.866_025_4;
    let side = APOTHEM / COS;
    let top = (x - x.min(side / 2.0)).hypot(y - APOTHEM);
    let along = ((side - x) * 0.5 + y * COS).clamp(0.0, side);
    let slant = (x - side + 0.5 * along).hypot(y - COS * along);
    let inside = y <= APOTHEM && COS * x + 0.5 * y <= APOTHEM;
    (inside || top.min(slant) <= ROUND, (x - PETAL).hypot(y) <= PETAL && x.hypot(y - PETAL) <= PETAL)
}

pub fn logo_image(size: usize, dark: bool) -> egui::ColorImage {
    const N: usize = 4;
    let star = if dark { 0 } else { 255 };
    let unit = |v: usize, sample: usize| (((v * N + sample) as f32 + 0.5) / (size * N) as f32 * 2.0 - 1.0).abs();
    let pixel = |index: usize| {
        let (mut hexagon, mut petals) = (0u32, 0u32);
        for sample in 0..N * N {
            let inside = logo_coverage(unit(index % size, sample % N), unit(index / size, sample / N));
            hexagon += inside.0 as u32;
            petals += (inside.0 && inside.1) as u32;
        }
        let total = (N * N) as u32;
        let channel = |fill: u32| ((fill * (hexagon - petals) + star * petals) / total) as u8;
        Color32::from_rgba_premultiplied(channel(191), channel(224), channel(240), (255 * hexagon / total) as u8)
    };
    egui::ColorImage::new([size, size], (0..size * size).map(pixel).collect())
}

pub fn logo(ui: &mut Ui, size: f32, dark: bool) -> Response {
    let pixels = (size * ui.ctx().pixels_per_point()).round() as usize;
    let id = Id::new(("logo", pixels, dark));
    let texture = ui.data_mut(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let texture = ui.ctx().load_texture("logo", logo_image(pixels, dark), egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    ui.add(egui::Image::new((texture.id(), Vec2::splat(size))))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_centered_logo() {
        let image = logo_image(64, true);
        let at = |x: usize, y: usize| image.pixels[y * 64 + x];
        assert_eq!(at(0, 0), Color32::TRANSPARENT);
        assert_eq!(at(42, 42), Color32::BLACK);
        assert_eq!(at(54, 32), Color32::from_rgb(191, 224, 240));
        assert_eq!(logo_image(64, false).pixels[42 * 64 + 42], Color32::WHITE);
        assert!((0..64 * 64).all(|i| at(i % 64, i / 64) == at(63 - i % 64, i / 64) && at(i % 64, i / 64) == at(i % 64, 63 - i / 64)));
    }
}
