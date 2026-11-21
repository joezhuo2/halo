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

fn back_out(t: f32) -> f32 {
    let u = t.clamp(0.0, 1.0) - 1.0;
    1.0 + 2.701_58 * u * u * u + 1.701_58 * u * u
}

fn cubic_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn skeleton_fraction(row: usize) -> f32 {
    0.35 + 0.05 * ((row * 7 + 3) % 11) as f32
}

#[allow(dead_code)]
pub fn spinner(ui: &Ui, center: egui::Pos2, radius: f32, color: Color32) {
    if !ui.is_rect_visible(Rect::from_center_size(center, Vec2::splat(radius * 2.0 + 2.0))) {
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    let start = (ui.input(|i| i.time) * 5.0 % std::f64::consts::TAU) as f32;
    let points = (0..=16).map(|i| center + radius * Vec2::angled(start + i as f32 / 16.0 * 1.5 * std::f32::consts::PI)).collect();
    ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

#[allow(dead_code)]
pub fn shimmer(ui: &Ui, theme: &Theme, rect: Rect) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    ui.painter().rect_filled(rect, CornerRadius::same(3), mix(theme.panel, theme.text_dim, 0.18));
    let phase = (ui.input(|i| i.time) % 1.2 / 1.2) as f32;
    let band = (rect.width() * 0.4).max(24.0);
    let x = rect.left() - band / 2.0 + phase * (rect.width() + band);
    let mut mesh = egui::Mesh::default();
    for (dx, color) in [(-band / 2.0, Color32::TRANSPARENT), (0.0, with_alpha(theme.text, 0.1)), (band / 2.0, Color32::TRANSPARENT)] {
        mesh.colored_vertex(egui::pos2(x + dx, rect.top()), color);
        mesh.colored_vertex(egui::pos2(x + dx, rect.bottom()), color);
    }
    for i in [0, 2] {
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    ui.painter().with_clip_rect(rect).add(mesh);
}

#[allow(dead_code)]
pub fn skeleton(ui: &mut Ui, theme: &Theme, rows: usize) {
    for row in 0..rows {
        let (_, r) = ui.allocate_space(egui::vec2(ui.available_width(), 22.0));
        let width = (r.width() * skeleton_fraction(row)).min(r.width() - 12.0).max(0.0);
        shimmer(ui, theme, Rect::from_min_size(egui::pos2(r.left() + 12.0, r.center().y - 4.0), egui::vec2(width, 8.0)));
    }
}

#[allow(dead_code)]
pub fn checkbox(ui: &mut Ui, theme: &Theme, value: &mut bool, text: impl Into<egui::WidgetText>) -> Response {
    let galley = text.into().into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button);
    let label = if galley.is_empty() { 0.0 } else { 8.0 + galley.size().x };
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(16.0 + label, galley.size().y.max(16.0)), Sense::click());
    if resp.clicked() {
        *value ^= true;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, galley.text()));
    let ctx = ui.ctx();
    let border = ctx.animate_bool_with_time(resp.id.with("border"), resp.hovered() || *value, 0.2);
    let fill = ctx.animate_bool_with_time(resp.id.with("fill"), *value, 0.35);
    let mark = ctx.animate_bool_with_time(resp.id.with("mark"), *value && fill >= 0.34, if *value { 0.25 } else { 0.15 });
    let press = anim(ui, resp.id.with("press"), resp.is_pointer_button_down_on());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    let s = 16.0 * (1.0 - 0.15 * press);
    let center = egui::pos2(rect.left() + 8.0, rect.center().y);
    let bx = Rect::from_center_size(center, Vec2::splat(s));
    let scale = if *value { back_out(fill) } else { fill * fill };
    if scale > 0.0 {
        painter.rect_filled(Rect::from_center_size(center, Vec2::splat(s * scale)), s * scale / 4.0, theme.accent);
    }
    let edge = mix(mix(theme.border, theme.text_dim, 0.4), theme.accent, border);
    painter.rect_stroke(bx, s / 4.0, egui::Stroke::new(1.5, edge), egui::StrokeKind::Inside);
    if resp.has_focus() {
        painter.rect_stroke(bx.expand(2.0), s / 4.0 + 2.0, egui::Stroke::new(1.0, theme.accent), egui::StrokeKind::Outside);
    }
    if mark > 0.0 {
        let at = |x: f32, y: f32| bx.min + egui::vec2(x, y) * (s / 20.0);
        let points = [at(5.0, 10.5), at(8.2, 13.7), at(15.0, 6.8)];
        let width = 0.1 * s;
        let mut clip = bx.expand(width);
        clip.max.x = egui::lerp(points[0].x - width..=points[2].x + width, mark);
        let painter = painter.with_clip_rect(clip);
        painter.line(points.to_vec(), egui::Stroke::new(width, theme.panel));
        painter.circle_filled(points[0], width / 2.0, theme.panel);
        painter.circle_filled(points[2], width / 2.0, theme.panel);
    }
    painter.galley(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, theme.text);
    resp
}

#[allow(dead_code)]
pub fn caret(painter: &egui::Painter, center: egui::Pos2, open: f32, color: Color32) {
    let galley = painter.layout_no_wrap(icons::CARET_RIGHT.to_owned(), FontId::proportional(11.0), color);
    let pos = center - galley.size() / 2.0;
    painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle_and_anchor(open * std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER));
}

#[allow(dead_code)]
pub fn section(ui: &mut Ui, theme: &Theme, title: &str, open: &mut bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::click());
    if resp.clicked() {
        *open ^= true;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, ui.is_enabled(), title));
    let h = anim(ui, resp.id.with("hover"), ui.rect_contains_pointer(rect));
    let t = anim(ui, resp.id.with("open"), *open);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, mix(theme.panel, theme.bar, 0.5 + 0.5 * h));
    painter.hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0, theme.border));
    caret(&painter, egui::pos2(rect.left() + 12.0, rect.center().y), t, theme.text);
    painter.text(egui::pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, title, FontId::proportional(11.5), theme.text);
    resp
}

#[allow(dead_code)]
pub fn reveal(ui: &Ui, id: Id, open: bool) -> f32 {
    cubic_out(ui.ctx().animate_bool_with_time(id, open, ANIM * 1.4))
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

    #[test]
    fn eases_and_varies_skeleton_rows() {
        for ease in [back_out, cubic_out] {
            assert!(ease(0.0).abs() < 1e-5 && (ease(1.0) - 1.0).abs() < 1e-5);
            assert!(ease(0.1) > 0.0 && ease(-1.0) == ease(0.0) && ease(2.0) == ease(1.0));
        }
        assert!((0..20).any(|i| back_out(i as f32 / 20.0) > 1.05));
        assert!((0..20).all(|i| cubic_out(i as f32 / 20.0) <= cubic_out((i + 1) as f32 / 20.0)));
        assert!((0..50).all(|row| (0.35..=0.85).contains(&skeleton_fraction(row))));
        assert!((0..4).map(skeleton_fraction).any(|f| f != skeleton_fraction(0)));
    }
}
