use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Color32, ColorImage, CornerRadius, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

pub const MAX_SIDE: u32 = 8192;
pub const EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"];

pub struct Decoded {
    pub image: ColorImage,
    pub size: [u32; 2],
    pub bytes: u64,
}

pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

pub fn capped(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_SIDE {
        return (width, height);
    }
    let scale = MAX_SIDE as f64 / longest as f64;
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

pub fn decode(path: &Path) -> Result<Decoded, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    decode_bytes(&bytes, path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")))
}

pub fn decode_bytes(bytes: &[u8], svg: bool) -> Result<Decoded, String> {
    let len = bytes.len() as u64;
    if svg {
        let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).map_err(|e| format!("Invalid SVG: {e}"))?;
        let (w, h) = ((tree.size().width().ceil() as u32).max(1), (tree.size().height().ceil() as u32).max(1));
        let (cw, ch) = capped(w, h);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(cw, ch).ok_or("The SVG has no size.")?;
        let scale = resvg::tiny_skia::Transform::from_scale(cw as f32 / tree.size().width(), ch as f32 / tree.size().height());
        resvg::render(&tree, scale, &mut pixmap.as_mut());
        let image = ColorImage::from_rgba_premultiplied([cw as usize, ch as usize], pixmap.data());
        return Ok(Decoded { image, size: [w, h], bytes: len });
    }
    let mut img = image::load_from_memory(bytes).map_err(|e| format!("Cannot decode the image: {e}"))?.into_rgba8();
    let size = [img.width(), img.height()];
    let (cw, ch) = capped(size[0], size[1]);
    if (cw, ch) != (size[0], size[1]) {
        img = image::imageops::resize(&img, cw, ch, image::imageops::FilterType::Triangle);
    }
    let image = ColorImage::from_rgba_unmultiplied([cw as usize, ch as usize], img.as_raw());
    Ok(Decoded { image, size, bytes: len })
}

pub struct ImageView {
    rx: Option<Receiver<Result<Decoded, String>>>,
    tex: Option<TextureHandle>,
    pub size: [u32; 2],
    pub bytes: u64,
    error: Option<String>,
    pub zoom: Option<f32>,
    pub shown: f32,
    pan: Vec2,
}

impl ImageView {
    pub fn open(path: PathBuf) -> Self {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(decode(&path));
        });
        Self { rx: Some(rx), tex: None, size: [0, 0], bytes: 0, error: None, zoom: None, shown: 1.0, pan: Vec2::ZERO }
    }

    pub fn ready(&self) -> bool {
        self.tex.is_some()
    }

    pub fn status(&self) -> Option<String> {
        self.ready().then(|| format!("{} x {}   {}   {}%", self.size[0], self.size[1], size_label(self.bytes), (self.shown * 100.0).round()))
    }

    pub fn step(&mut self, by: Option<f32>) {
        match by {
            Some(factor) => self.zoom = Some((self.shown * factor).clamp(0.02, 64.0)),
            None => (self.zoom, self.pan) = (None, Vec2::ZERO),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, name: &str) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(Ok(done)) => {
                    self.tex = Some(ui.ctx().load_texture(name, done.image, TextureOptions::LINEAR));
                    (self.size, self.bytes, self.rx) = (done.size, done.bytes, None);
                }
                Ok(Err(error)) => (self.error, self.rx) = (Some(error), None),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui::spinner(ui, rect.center(), 10.0, theme.text_dim);
                    return;
                }
                Err(_) => (self.error, self.rx) = (Some("The image could not be read.".into()), None),
            }
        }
        let Some(tex) = &self.tex else {
            let text = self.error.clone().unwrap_or_default();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), theme.text_dim);
            return;
        };
        let natural = vec2(self.size[0] as f32, self.size[1] as f32);
        let fit = (rect.width() / natural.x).min(rect.height() / natural.y).min(1.0);
        let resp = ui.interact(rect, egui::Id::new(("image_view", name)), Sense::click_and_drag());
        if resp.hovered() {
            let wheel = ui.input(|i| i.zoom_delta());
            if wheel != 1.0 {
                self.zoom = Some((self.zoom.unwrap_or(fit) * wheel).clamp(0.02, 64.0));
            }
        }
        if resp.dragged() {
            self.pan += resp.drag_delta();
        } else if resp.clicked() {
            self.zoom = if self.zoom == Some(1.0) { None } else { Some(1.0) };
            self.pan = Vec2::ZERO;
        }
        self.shown = self.zoom.unwrap_or(fit);
        if self.zoom.is_none() {
            self.pan = Vec2::ZERO;
        }
        let image = Rect::from_center_size(rect.center() + self.pan, natural * self.shown);
        let painter = ui.painter().with_clip_rect(rect);
        let visible = image.intersect(rect);
        let (light, dark) = (ui::mix(theme.bg, theme.text, 0.12), ui::mix(theme.bg, theme.text, 0.05));
        painter.rect_filled(visible, CornerRadius::ZERO, light);
        let cell = 12.0;
        let (first_x, first_y) = (((visible.left() - image.left()) / cell).floor() as i32, ((visible.top() - image.top()) / cell).floor() as i32);
        let (last_x, last_y) = (((visible.right() - image.left()) / cell).ceil() as i32, ((visible.bottom() - image.top()) / cell).ceil() as i32);
        for cy in first_y..last_y {
            for cx in (first_x..last_x).filter(|cx| (cx + cy) % 2 != 0) {
                let at = image.min + vec2(cx as f32, cy as f32) * cell;
                painter.rect_filled(Rect::from_min_size(at, Vec2::splat(cell)).intersect(visible), CornerRadius::ZERO, dark);
            }
        }
        painter.image(tex.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
    }
}

pub fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / (1 << 10) as f64),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba() -> image::RgbaImage {
        image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 100, y as u8 * 100, 50, if x == 0 { 0 } else { 255 }]))
    }

    fn encode(format: image::ImageFormat) -> Vec<u8> {
        let mut out = std::io::Cursor::new(vec![]);
        let img = image::DynamicImage::ImageRgba8(rgba());
        match format {
            image::ImageFormat::Jpeg => img.to_rgb8().write_to(&mut out, format).unwrap(),
            image::ImageFormat::Bmp | image::ImageFormat::Gif | image::ImageFormat::Png | image::ImageFormat::Ico | image::ImageFormat::WebP => {
                img.write_to(&mut out, format).unwrap()
            }
            _ => unreachable!(),
        }
        out.into_inner()
    }

    #[test]
    fn decodes_every_raster_format() {
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg, image::ImageFormat::Gif, image::ImageFormat::Bmp, image::ImageFormat::Ico, image::ImageFormat::WebP] {
            let bytes = encode(format);
            let done = decode_bytes(&bytes, false).unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!((done.size, done.image.size, done.bytes), ([3, 2], [3, 2], bytes.len() as u64), "{format:?}");
        }
        let png = decode_bytes(&encode(image::ImageFormat::Png), false).unwrap();
        assert_eq!((png.image.pixels[0].a(), png.image.pixels[1].a()), (0, 255));
        assert!(decode_bytes(b"not an image", false).is_err());
    }

    #[test]
    fn decodes_svg_and_caps_large_sizes() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;
        let done = decode_bytes(svg, true).unwrap();
        assert_eq!((done.size, done.image.size), ([40, 20], [40, 20]));
        assert_eq!(done.image.pixels[0], Color32::from_rgb(255, 0, 0));
        let wide = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16384" height="4096"/>"#;
        let done = decode_bytes(wide, true).unwrap();
        assert_eq!((done.size, done.image.size), ([16384, 4096], [8192, 2048]));
        assert_eq!(capped(100, 200), (100, 200));
        assert_eq!(capped(8192, 9000), (7457, 8192));
        assert!(decode_bytes(b"<svg", true).is_err());
        assert!(is_image(Path::new("a/B.PNG")) && is_image(Path::new("x.svg")) && !is_image(Path::new("x.rs")));
    }

    #[test]
    fn zoom_steps_and_fit() {
        let mut view = ImageView { rx: None, tex: None, size: [10, 10], bytes: 5, error: None, zoom: None, shown: 0.5, pan: Vec2::ZERO };
        view.step(Some(2.0));
        assert_eq!(view.zoom, Some(1.0));
        view.step(None);
        assert_eq!(view.zoom, None);
        assert_eq!(size_label(1536), "1.5 KB");
    }
}
