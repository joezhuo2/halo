use crate::command::Command;
pub use crate::notifications::Severity;
use crate::theme::Theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Response, Sense, Ui, Vec2};

pub use egui_phosphor::regular as icons;
pub use eframe::egui::accesskit::Role;

pub const ANIM: f32 = 0.14;
pub const FIELD: egui::Margin = egui::Margin::symmetric(8, 5);

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
    notify_warning(text);
}

pub fn recall(ui: &Ui, id: Id, history: &[String], at: &mut Option<usize>, text: &mut String) {
    if history.is_empty() || !ui.memory(|m| m.has_focus(id)) {
        return;
    }
    let (up, down) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)));
    let last = history.len() - 1;
    if up {
        let newest = if history[last] == *text { last.saturating_sub(1) } else { last };
        *at = Some(at.map_or(newest, |at| at.saturating_sub(1)));
    }
    if down {
        *at = at.and_then(|at| (at < last).then_some(at + 1));
    }
    if up || down {
        *text = at.map(|at| history[at].clone()).unwrap_or_default();
    }
}

#[allow(dead_code)]
pub fn notify_info(text: impl Into<String>) {
    notify_with(Severity::Info, text, vec![]);
}

pub fn notify_warning(text: impl Into<String>) {
    notify_with(Severity::Warning, text, vec![]);
}

#[allow(dead_code)]
pub fn notify_error(text: impl Into<String>) {
    notify_with(Severity::Error, text, vec![]);
}

pub fn notify_with(severity: Severity, text: impl Into<String>, actions: Vec<(String, Command)>) {
    crate::notifications::push(severity, text.into(), actions);
}

pub fn progress(title: impl Into<String>) -> crate::notifications::Progress {
    crate::notifications::progress(title.into())
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

pub fn anim(ui: &Ui, id: Id, on: bool) -> f32 {
    animate(ui, id, on, ANIM)
}

fn animate(ui: &Ui, id: Id, on: bool, time: f32) -> f32 {
    if crate::a11y::reduced() {
        return f32::from(u8::from(on));
    }
    ui.ctx().animate_bool_with_time(id, on, time)
}

pub enum Enter {
    Primary,
    Multiline,
    Widget,
}

fn modal_key(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Key { key: egui::Key::Enter | egui::Key::Escape, .. })
}

#[derive(Default)]
struct ModalInput;

impl egui::Plugin for ModalInput {
    fn debug_name(&self) -> &'static str { "Halo modal keyboard" }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let Some(layer) = ui.memory(|m| m.top_modal_layer()) else { return };
        let keys = ui.input_mut(|i| {
            let keys: Vec<_> = i.events.iter().enumerate().filter(|(_, e)| modal_key(e)).map(|(at, e)| (at, e.clone())).collect();
            i.events.retain(|e| !modal_key(e));
            keys
        });
        let id = layer.id.with(ui.ctx().viewport_id());
        ui.data_mut(|d| d.insert_temp(id, keys));
    }
}

pub fn modal<R>(ctx: &egui::Context, id: impl egui::AsId, mode: Enter, show: impl FnOnce(&mut Ui, DialogKeys) -> R) -> egui::ModalResponse<R> {
    ctx.plugin_or_default::<ModalInput>();
    let modal = egui::Modal::new(Id::new(id)).frame(egui::Frame::popup(&ctx.global_style()).inner_margin(18.0));
    let layer = modal.area.layer();
    let top = ctx.memory(|m| m.top_modal_layer() == Some(layer));
    let popup = egui::Popup::is_any_open(ctx);
    let mut enter = false;
    let mut escape = false;
    if top {
        let id = layer.id.with(ctx.viewport_id());
        let keys = ctx.data_mut(|d| d.remove_temp::<Vec<(usize, egui::Event)>>(id)).unwrap_or_default();
        ctx.input_mut(|i| {
            for (at, event) in keys {
                let egui::Event::Key { key, pressed, repeat, modifiers, .. } = &event else { continue };
                if *key == egui::Key::Escape && modifiers.is_none() {
                    escape |= *pressed && !*repeat;
                } else if !popup && *key == egui::Key::Enter && !matches!(mode, Enter::Widget)
                    && (!matches!(mode, Enter::Multiline) || modifiers.ctrl || modifiers.command)
                {
                    enter |= *pressed && !*repeat && !modifiers.alt && !modifiers.shift;
                } else {
                    i.events.insert(at.min(i.events.len()), event);
                }
            }
        });
    }
    let response = modal.show(ctx, |ui| {
        if escape {
            ui.close();
        }
        show(ui, DialogKeys { enter: enter && !escape })
    });
    if top {
        ctx.input_mut(|i| i.events.retain(|e| !modal_key(e)));
    }
    response
}

pub struct DialogKeys {
    enter: bool,
}

impl DialogKeys {
    pub fn primary(&self, ui: &mut Ui, label: impl Into<egui::WidgetText>, enabled: bool) -> bool {
        let button = egui::Button::new(label.into()).stroke(ui.visuals().selection.stroke);
        let response = ui.add_enabled(enabled, button);
        response.clicked() || (response.enabled() && self.enter)
    }

    pub fn choice(&self, ui: &mut Ui, label: &str, primary: bool) -> bool {
        if primary { self.primary(ui, label, true) } else { ui.button(label).clicked() }
    }
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
    state_border(ui.painter(), theme, rect, hovered, selected);
    focus_ring(ui, theme, rect, id);
}

pub fn focus_ring(ui: &Ui, theme: &Theme, rect: Rect, id: Id) {
    crate::a11y::mark(ui, id);
    if ui.memory(|m| m.has_focus(id)) {
        ui.painter().rect_stroke(rect.shrink(0.5), CornerRadius::same(2), egui::Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
    }
}

pub fn access(ui: &Ui, id: Id, role: Role, name: &str, state: impl FnOnce(&mut egui::accesskit::Node)) {
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_role(role);
        node.set_label(name);
        state(node);
    });
}

pub fn item(ui: &Ui, resp: &Response, role: Role, name: &str, selected: bool) {
    access(ui, resp.id, role, name, |n| n.set_selected(selected));
}

pub fn plain(text: &str) -> String {
    text.chars().filter(|c| !('\u{e000}'..='\u{f8ff}').contains(c)).collect::<String>().trim().to_owned()
}

pub fn list_item(ui: &Ui, resp: &Response, name: &str, selected: bool, (index, count): (usize, usize)) {
    access(ui, resp.id, Role::ListItem, name, |n| {
        n.set_selected(selected);
        n.set_position_in_set(index + 1);
        n.set_size_of_set(count);
    });
}

pub fn tree_item(ui: &Ui, resp: &Response, name: &str, selected: bool, depth: usize, expanded: Option<bool>) {
    access(ui, resp.id, Role::TreeItem, name, |n| {
        n.set_selected(selected);
        n.set_level(depth + 1);
        if let Some(open) = expanded {
            n.set_expanded(open);
        }
    });
}

pub fn state_border(painter: &egui::Painter, theme: &Theme, rect: Rect, hovered: bool, selected: bool) {
    if hovered || selected {
        contrast_border(painter, theme, rect, if selected { theme.accent } else { theme.border });
    }
}

pub fn contrast_border(painter: &egui::Painter, theme: &Theme, rect: Rect, color: Color32) {
    if theme.contrast {
        painter.rect_stroke(rect.shrink(0.5), CornerRadius::ZERO, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
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
        state_border(ui.painter(), theme, rect.shrink(1.0), resp.hovered() || resp.has_focus(), active);
        focus_ring(ui, theme, rect.shrink(1.0), resp.id);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(size * 0.62),
            mix(mix(theme.text_dim, theme.text, h), theme.accent, a),
        );
    }
    access(ui, resp.id, Role::Button, tip, |_| {});
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

pub fn tabs(ui: &mut Ui, theme: &Theme, id: Id, labels: &[&str], active: &mut usize) -> Option<usize> {
    let (mut underline, mut dragged) = (None, None);
    for (index, label) in labels.iter().enumerate() {
        let galley = ui.painter().layout_no_wrap((*label).to_owned(), FontId::proportional(11.5), Color32::PLACEHOLDER);
        let (r, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 20.0, 32.0), Sense::click_and_drag());
        if resp.clicked() {
            *active = index;
        }
        if resp.drag_started() {
            dragged = Some(index);
        }
        let a = anim(ui, resp.id.with("active"), *active == index);
        let h = anim(ui, resp.id.with("hover"), resp.hovered());
        item(ui, &resp, Role::Tab, label, *active == index);
        focus_ring(ui, theme, r, resp.id);
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, mix(theme.text_dim, theme.text, a.max(h)));
        if *active == index {
            underline = Some(r);
        }
    }
    if let Some(r) = underline {
        let time = if crate::a11y::reduced() { 0.0 } else { ANIM };
        let left = ui.ctx().animate_value_with_time(id.with("left"), r.left() + 10.0, time);
        let right = ui.ctx().animate_value_with_time(id.with("right"), r.right() - 10.0, time);
        let bar = Rect::from_min_max(egui::pos2(left, r.bottom() - 4.0), egui::pos2(right, r.bottom() - 2.5));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.accent);
    }
    dragged
}

pub fn fade_in(ui: &Ui, id: Id) -> f32 {
    if crate::a11y::reduced() {
        return 1.0;
    }
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

pub fn spinner(ui: &Ui, center: egui::Pos2, radius: f32, color: Color32) {
    if !ui.is_rect_visible(Rect::from_center_size(center, Vec2::splat(radius * 2.0 + 2.0))) {
        return;
    }
    let start = if crate::a11y::reduced() {
        0.0
    } else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        (ui.input(|i| i.time) * 5.0 % std::f64::consts::TAU) as f32
    };
    let points = (0..=16).map(|i| center + radius * Vec2::angled(start + i as f32 / 16.0 * 1.5 * std::f32::consts::PI)).collect();
    ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

pub fn shimmer(ui: &Ui, theme: &Theme, rect: Rect) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, CornerRadius::same(3), mix(theme.panel, theme.text_dim, 0.18));
    if crate::a11y::reduced() {
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
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

pub fn shimmer_text(painter: &egui::Painter, theme: &Theme, pos: egui::Pos2, galley: std::sync::Arc<egui::Galley>) {
    let rect = Rect::from_min_size(pos, galley.size());
    if !painter.clip_rect().intersects(rect) {
        return;
    }
    if crate::a11y::reduced() {
        painter.galley(pos, galley, theme.text_dim);
        return;
    }
    painter.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    let phase = (painter.ctx().input(|i| i.time) % 1.2 / 1.2) as f32;
    let band = (rect.width() * 0.4).max(24.0);
    let x = rect.left() - band + phase * (rect.width() + band);
    let lit = Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + band, rect.bottom()));
    painter.galley(pos, galley.clone(), with_alpha(theme.text_dim, 0.55));
    painter.with_clip_rect(lit.intersect(painter.clip_rect())).galley(pos, galley, theme.text);
}

pub fn skeleton(ui: &mut Ui, theme: &Theme, rows: usize) {
    for row in 0..rows {
        let (_, r) = ui.allocate_space(egui::vec2(ui.available_width(), 22.0));
        let width = (r.width() * skeleton_fraction(row)).min(r.width() - 12.0).max(0.0);
        shimmer(ui, theme, Rect::from_min_size(egui::pos2(r.left() + 12.0, r.center().y - 4.0), egui::vec2(width, 8.0)));
    }
}

pub fn list_end(ui: &mut Ui, theme: &Theme, loading: bool) -> bool {
    if loading {
        skeleton(ui, theme, 2);
    }
    !loading && ui.min_rect().bottom() <= ui.clip_rect().bottom() + 200.0
}

pub fn checkbox(ui: &mut Ui, theme: &Theme, value: &mut bool, text: impl Into<egui::WidgetText>) -> Response {
    let galley = text.into().into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button);
    let label = if galley.is_empty() { 0.0 } else { 8.0 + galley.size().x };
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(16.0 + label, galley.size().y.max(16.0)), Sense::click());
    if resp.clicked() {
        *value ^= true;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, galley.text()));
    let border = animate(ui, resp.id.with("border"), resp.hovered() || *value, 0.2);
    let fill = animate(ui, resp.id.with("fill"), *value, 0.35);
    let mark = animate(ui, resp.id.with("mark"), *value && fill >= 0.34, if *value { 0.25 } else { 0.15 });
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
    crate::a11y::mark(ui, resp.id);
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

pub fn caret(painter: &egui::Painter, center: egui::Pos2, open: f32, color: Color32) {
    let galley = painter.layout_no_wrap(icons::CARET_RIGHT.to_owned(), FontId::proportional(11.0), color);
    let pos = center - galley.size() / 2.0;
    painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle_and_anchor(open * std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER));
}

pub fn section(ui: &mut Ui, theme: &Theme, title: &str, open: &mut bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::click());
    if resp.clicked() {
        *open ^= true;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, ui.is_enabled(), title));
    access(ui, resp.id, Role::Button, title, |n| n.set_expanded(*open));
    let h = anim(ui, resp.id.with("hover"), ui.rect_contains_pointer(rect));
    let t = anim(ui, resp.id.with("open"), *open);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, mix(theme.panel, theme.bar, 0.5 + 0.5 * h));
    painter.hline(rect.x_range(), rect.top() + 0.5, egui::Stroke::new(1.0, theme.border));
    caret(&painter, egui::pos2(rect.left() + 12.0, rect.center().y), t, theme.text);
    painter.text(egui::pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, title, FontId::proportional(11.5), theme.text);
    focus_ring(ui, theme, rect, resp.id);
    resp
}

pub fn reveal(ui: &Ui, id: Id, open: bool) -> f32 {
    cubic_out(animate(ui, id, open, ANIM * 1.4))
}

pub const SECTION: f32 = 24.0;
const SECTION_MIN: f32 = 66.0;

pub fn splitter(ui: &mut Ui, theme: &Theme, id: Id, line: Rect, vertical: bool) -> Response {
    let resp = ui.interact(line.expand(3.0), id, Sense::click_and_drag());
    let hot = resp.hovered() || resp.dragged();
    if hot {
        ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
    }
    let t = anim(ui, id.with("hot"), hot);
    ui.painter().rect_filled(line.expand(t), CornerRadius::ZERO, mix(theme.border, theme.accent, t));
    resp
}

pub fn stack(ui: &mut Ui, theme: &Theme, id: Id, panes: &[(bool, Option<f32>)], sizes: &mut [Option<f32>], mut add: impl FnMut(&mut Ui, usize)) {
    let area = ui.available_rect_before_wrap();
    let open: Vec<f32> = panes.iter().enumerate().map(|(i, p)| reveal(ui, id.with(("reveal", i)), p.0)).collect();
    let wants: Vec<Option<f32>> = panes.iter().zip(sizes.iter()).map(|(p, size)| size.or(p.1)).collect();
    let avail = (area.height() - panes.len() as f32 * SECTION).max(0.0);
    let min = SECTION_MIN.min(avail / open.iter().sum::<f32>().max(1.0));
    let heights = stack_heights(avail, min, &open, &wants);
    let (mut tops, mut top) = (vec![], area.top());
    for (i, height) in heights.iter().enumerate() {
        let rect = Rect::from_min_size(egui::pos2(area.left(), top), egui::vec2(area.width(), SECTION + height));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt((id, i)), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            add(ui, i);
        });
        tops.push(top);
        top = rect.bottom();
    }
    for i in 1..panes.len() {
        let Some(prev) = panes[..i].iter().rposition(|p| p.0).filter(|_| panes[i].0) else { continue };
        let line = Rect::from_x_y_ranges(area.x_range(), tops[i] - 0.5..=tops[i] + 0.5);
        let resp = splitter(ui, theme, id.with(("split", i)), line, false);
        if resp.double_clicked() {
            (sizes[prev], sizes[i]) = (None, None);
        } else if resp.dragged() {
            stack_resize(sizes, &heights, &open, &wants, (prev, i), resp.drag_delta().y, min);
        }
    }
    ui.advance_cursor_after_rect(area);
}

pub fn stack_heights(avail: f32, min: f32, open: &[f32], wants: &[Option<f32>]) -> Vec<f32> {
    let total = avail * open.iter().sum::<f32>().min(1.0);
    let flex: f32 = open.iter().zip(wants).filter(|(_, w)| w.is_none()).map(|(t, _)| t).sum();
    let fixed: f32 = open.iter().zip(wants).filter_map(|(t, w)| w.map(|w| w * t)).sum();
    let mut heights: Vec<f32> = open.iter().zip(wants).map(|(&t, w)| match w {
        Some(w) => w * t,
        None => ((total - fixed).max(0.0) * t / flex).max(min * t),
    }).collect();
    let sum: f32 = heights.iter().sum();
    if sum < total {
        let count: f32 = open.iter().sum();
        for (h, t) in heights.iter_mut().zip(open) {
            *h += (total - sum) * if sum > 0.0 { *h / sum } else { t / count };
        }
    } else if sum > total {
        let room: Vec<f32> = heights.iter().zip(open).map(|(h, t)| (h - min * t).max(0.0)).collect();
        let spare: f32 = room.iter().sum();
        if spare > 0.0 {
            let take = (sum - total).min(spare);
            heights.iter_mut().zip(&room).for_each(|(h, r)| *h -= r * take / spare);
        }
        let left: f32 = heights.iter().sum();
        if left > total {
            heights.iter_mut().for_each(|h| *h *= total / left);
        }
    }
    heights
}

fn stack_resize(sizes: &mut [Option<f32>], heights: &[f32], open: &[f32], wants: &[Option<f32>], (prev, next): (usize, usize), dy: f32, min: f32) {
    let dy = dy.clamp((min - heights[prev]).min(0.0), (heights[next] - min).max(0.0));
    if dy == 0.0 {
        return;
    }
    for (((size, &h), &t), want) in sizes.iter_mut().zip(heights).zip(open).zip(wants) {
        if t >= 1.0 && want.is_some_and(|w| w != h) {
            *size = Some(h);
        }
    }
    (sizes[prev], sizes[next]) = (Some(heights[prev] + dy), Some(heights[next] - dy));
}

pub fn direct_wheel(events: &mut Vec<egui::Event>, line: f32, smooth: bool) {
    use egui::{Event, MouseWheelUnit, TouchPhase};
    *events = std::mem::take(events)
        .into_iter()
        .flat_map(|event| match event {
            Event::MouseWheel { unit, delta, phase: TouchPhase::Move, modifiers }
                if (unit == MouseWheelUnit::Line && (!smooth || delta.x.fract() != 0.0 || delta.y.fract() != 0.0)) || (unit == MouseWheelUnit::Point && !smooth) =>
            {
                let delta = if unit == MouseWheelUnit::Line { delta * line } else { delta };
                let parts = (delta.length() / 7.0).ceil().clamp(1.0, 1000.0);
                vec![Event::MouseWheel { unit: MouseWheelUnit::Point, delta: delta / parts, phase: TouchPhase::Move, modifiers }; parts as usize]
            }
            other => vec![other],
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_touchpad_and_unsmoothed_wheel_input_into_direct_steps() {
        use egui::{Event, MouseWheelUnit as U, TouchPhase, vec2};
        let wheel = |unit, y| Event::MouseWheel { unit, delta: vec2(0.0, y), phase: TouchPhase::Move, modifiers: Default::default() };
        let sum = |events: &[Event]| events.iter().map(|e| if let Event::MouseWheel { unit: U::Point, delta, .. } = e { assert!(delta.length() < 8.0); delta.y } else { 0.0 }).sum::<f32>();
        let mut events = vec![wheel(U::Line, -0.35), wheel(U::Line, 1.0), wheel(U::Point, 30.0), Event::Copy];
        direct_wheel(&mut events, 40.0, true);
        assert_eq!((events.len(), sum(&events[..2])), (5, -14.0));
        assert!(matches!(events[2..], [Event::MouseWheel { unit: U::Line, .. }, Event::MouseWheel { unit: U::Point, .. }, Event::Copy]));
        let mut events = vec![wheel(U::Line, 1.0), wheel(U::Point, 30.0)];
        direct_wheel(&mut events, 40.0, false);
        assert_eq!((events.len(), sum(&events).round()), (11, 70.0));
    }

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

    fn shapes(theme: &Theme) -> usize {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(100.0, 20.0));
            row_bg(ui, theme, rect, Id::new("row"), true, true);
            icon_button(ui, theme, icons::X, 20.0, true, "");
        });
        out.textures_delta.clear();
        out.shapes.len()
    }

    #[test]
    fn high_contrast_themes_draw_borders() {
        assert_eq!(shapes(&Theme::hc_dark()), shapes(&Theme::dark()) + 2);
        assert_eq!(shapes(&Theme::hc_light()), shapes(&Theme::light()) + 2);
    }

    #[test]
    fn stack_heights_fill_the_space_and_respect_the_minimum() {
        let sum = |h: &[f32]| h.iter().sum::<f32>();
        assert_eq!(stack_heights(400.0, 66.0, &[1.0; 3], &[Some(100.0), None, Some(50.0)]), [100.0, 250.0, 50.0]);
        assert_eq!(stack_heights(300.0, 66.0, &[1.0; 3], &[None; 3]), [100.0; 3]);
        let collapsed = stack_heights(400.0, 66.0, &[1.0, 0.0, 1.0], &[Some(100.0), None, Some(50.0)]);
        assert!((sum(&collapsed) - 400.0).abs() < 1e-3 && collapsed[1] == 0.0 && collapsed[0] > 2.0 * collapsed[2] - 1e-3);
        let squeezed = stack_heights(400.0, 66.0, &[1.0; 3], &[Some(300.0), None, Some(200.0)]);
        assert!((sum(&squeezed) - 400.0).abs() < 1e-3 && squeezed[1] == 66.0 && squeezed.iter().all(|&h| h >= 66.0));
        let tiny = stack_heights(60.0, 20.0, &[1.0; 3], &[Some(300.0), None, Some(200.0)]);
        assert!((sum(&tiny) - 60.0).abs() < 1e-3);
        assert_eq!(stack_heights(400.0, 66.0, &[1.0, 1.0], &[Some(0.0), None]), [0.0, 400.0]);
        assert_eq!(stack_heights(400.0, 66.0, &[0.0; 2], &[Some(10.0), None]), [0.0; 2]);
    }

    #[test]
    fn stack_resize_moves_one_border_and_clamps() {
        let (open, wants) = ([1.0; 3], [Some(100.0), None, None]);
        let heights = stack_heights(400.0, 66.0, &open, &wants);
        let mut sizes = [None; 3];
        stack_resize(&mut sizes, &heights, &open, &wants, (1, 2), 500.0, 66.0);
        assert_eq!(sizes, [None, Some(234.0), Some(66.0)]);
        let wants = [Some(100.0), sizes[1], sizes[2]];
        assert_eq!(stack_heights(400.0, 66.0, &open, &wants), [100.0, 234.0, 66.0]);
        stack_resize(&mut sizes, &stack_heights(400.0, 66.0, &open, &wants), &open, &wants, (1, 2), 66.0, 66.0);
        assert_eq!(sizes, [None, Some(234.0), Some(66.0)]);
    }

    fn run_stack(ctx: &egui::Context, open: &mut [bool; 3], sizes: &mut [Option<f32>], events: Vec<egui::Event>) -> Vec<f32> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(200.0, 400.0))),
            events,
            ..Default::default()
        };
        let mut heights = vec![0.0; 3];
        let mut out = ctx.run_ui(input, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let panes = [(open[0], Some(60.0)), (open[1], None), (open[2], Some(100.0))];
            stack(ui, &Theme::dark(), Id::new("stack"), &panes, sizes, |ui, i| {
                section(ui, &Theme::dark(), "PANE", &mut open[i]);
                heights[i] = ui.max_rect().height();
            });
        });
        out.textures_delta.clear();
        heights
    }

    #[test]
    fn dragging_a_stack_border_resizes_its_neighbours() {
        let ctx = egui::Context::default();
        let (mut open, mut sizes) = ([true; 3], [None; 3]);
        let rest = |ctx: &egui::Context, open: &mut [bool; 3], sizes: &mut [Option<f32>]| (0..30).map(|_| run_stack(ctx, open, sizes, vec![])).last().unwrap();
        let start = rest(&ctx, &mut open, &mut sizes);
        assert_eq!(start, [84.0, 192.0, 124.0]);
        let total: f32 = start.iter().sum();
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |ctx: &egui::Context, open: &mut [bool; 3], sizes: &mut [Option<f32>], from: egui::Pos2, dy: f32| {
            let to = from + Vec2::new(0.0, dy);
            run_stack(ctx, open, sizes, vec![egui::Event::PointerMoved(from)]);
            run_stack(ctx, open, sizes, vec![button(from, true)]);
            run_stack(ctx, open, sizes, vec![egui::Event::PointerMoved(to)]);
            run_stack(ctx, open, sizes, vec![button(to, false)]);
        };
        let border = egui::pos2(100.0, start[0] + start[1]);
        drag(&ctx, &mut open, &mut sizes, border, -40.0);
        let moved = rest(&ctx, &mut open, &mut sizes);
        assert_eq!(moved, [84.0, 152.0, 164.0]);
        assert_eq!(moved.iter().sum::<f32>(), total);

        drag(&ctx, &mut open, &mut sizes, border - Vec2::new(0.0, 40.0), -500.0);
        let squeezed = rest(&ctx, &mut open, &mut sizes);
        assert_eq!(squeezed[1], SECTION + SECTION_MIN);
        assert_eq!(squeezed.iter().sum::<f32>(), total);

        open[2] = false;
        let collapsed = rest(&ctx, &mut open, &mut sizes);
        assert_eq!(collapsed[2], SECTION);
        assert!((collapsed.iter().sum::<f32>() - total).abs() < 0.01);
        open[2] = true;
        assert_eq!(rest(&ctx, &mut open, &mut sizes), squeezed);

        let line = egui::pos2(100.0, squeezed[0] + squeezed[1]);
        run_stack(&ctx, &mut open, &mut sizes, vec![egui::Event::PointerMoved(line)]);
        for _ in 0..2 {
            run_stack(&ctx, &mut open, &mut sizes, vec![button(line, true)]);
            run_stack(&ctx, &mut open, &mut sizes, vec![button(line, false)]);
        }
        assert_eq!(sizes, [None; 3]);
        assert_eq!(rest(&ctx, &mut open, &mut sizes), start);
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
