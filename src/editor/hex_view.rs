use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align, Align2, CornerRadius, Event, FontId, Key, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

const WIDTH: usize = 16;

pub struct HexView {
    path: PathBuf,
    rx: Option<Receiver<Result<Vec<u8>, String>>>,
    pub bytes: Vec<u8>,
    error: Option<String>,
    pub dirty: bool,
    pub cursor: usize,
    low: bool,
    ascii: bool,
    top: usize,
    edited: BTreeSet<usize>,
    undo: Vec<(usize, u8, bool)>,
    saved: usize,
}

pub fn hex_digit(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

pub fn shown(b: u8) -> char {
    if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' }
}

impl HexView {
    pub fn open(path: PathBuf) -> Self {
        let (tx, rx) = channel();
        let from = path.clone();
        std::thread::spawn(move || {
            let read = match std::fs::metadata(&from) {
                Ok(meta) if meta.len() > super::buffer::HUGE => Err(format!("The file is {} MB; the hex editor opens files up to 256 MB.", meta.len() >> 20)),
                _ => std::fs::read(&from).map_err(|e| e.to_string()),
            };
            let _ = tx.send(read);
        });
        HexView { path, rx: Some(rx), bytes: vec![], error: None, dirty: false, cursor: 0, low: false, ascii: false, top: 0, edited: BTreeSet::new(), undo: vec![], saved: 0 }
    }

    pub fn save(&mut self) -> Result<(), String> {
        if self.rx.is_some() || self.error.is_some() {
            return Err("The file is not loaded.".into());
        }
        crate::wsl::write_file(&self.path, &self.bytes).map_err(|e| format!("Cannot save {}: {e}", self.path.display()))?;
        (self.dirty, self.edited, self.saved) = (false, BTreeSet::new(), self.undo.len());
        Ok(())
    }

    pub fn put(&mut self, c: char) -> bool {
        let Some(old) = self.bytes.get(self.cursor).copied() else { return false };
        let new = match self.ascii {
            true if c.is_ascii() && !c.is_ascii_control() => c as u8,
            true => return false,
            false => {
                let Some(d) = hex_digit(c) else { return false };
                if self.low { (old & 0xF0) | d } else { (old & 0x0F) | (d << 4) }
            }
        };
        if self.undo.len() < self.saved {
            self.saved = usize::MAX;
        }
        self.undo.push((self.cursor, old, self.low));
        self.bytes[self.cursor] = new;
        self.edited.insert(self.cursor);
        self.dirty = true;
        match (self.ascii, self.low) {
            (false, false) => self.low = true,
            _ => {
                self.low = false;
                self.cursor = (self.cursor + 1).min(self.bytes.len().saturating_sub(1));
            }
        }
        true
    }

    pub fn undo(&mut self) {
        if let Some((at, old, low)) = self.undo.pop() {
            self.bytes[at] = old;
            (self.cursor, self.low) = (at, low);
            self.dirty = self.undo.len() != self.saved;
            if !self.undo.iter().any(|u| u.0 == at) {
                self.edited.remove(&at);
            }
        }
    }

    fn go(&mut self, to: isize) {
        let last = self.bytes.len().saturating_sub(1) as isize;
        (self.cursor, self.low) = (to.clamp(0, last) as usize, false);
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, font_size: f32) -> bool {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(Ok(bytes)) => (self.bytes, self.rx) = (bytes, None),
                Ok(Err(e)) => (self.error, self.rx) = (Some(e), None),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui::spinner(ui, rect.center(), 10.0, theme.text_dim);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
                    return false;
                }
                Err(_) => (self.error, self.rx) = (Some("The file could not be read.".into()), None),
            }
        }
        let mut as_text = false;
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 28.0));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.panel);
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 3.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
            let value = self.bytes.get(self.cursor).map_or(String::new(), |b| format!("  ·  value 0x{b:02X} ({b})"));
            let text = format!("{}  ·  offset 0x{:08X} ({}){value}{}", super::image_view::size_label(self.bytes.len() as u64), self.cursor, self.cursor, if self.dirty { "  ·  modified" } else { "" });
            ui.label(RichText::new(text).size(12.0).color(theme.text_dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(!self.dirty, egui::Button::new("Open as Text").small()).on_hover_text("Reopen the file in the text editor").clicked() {
                    as_text = true;
                }
            });
        });
        if let Some(error) = &self.error {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, error, FontId::proportional(13.0), theme.text_dim);
            return as_text;
        }
        let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
        let font = FontId::monospace(font_size);
        let (cw, row_h) = ui.ctx().fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font) + 3.0));
        let rows_shown = ((body.height() / row_h) as usize).max(1);
        let total_rows = self.bytes.len().div_ceil(WIDTH).max(1);
        let id = egui::Id::new(("hex_view", &self.path));
        let resp = ui.interact(body, id, Sense::click());
        let (hex_x, ascii_x) = (body.left() + 12.0 + 10.0 * cw, body.left() + 12.0 + (10 + WIDTH * 3 + 2) as f32 * cw);
        if resp.clicked() {
            resp.request_focus();
            if let Some(p) = resp.interact_pointer_pos() {
                let row = self.top + ((p.y - body.top()) / row_h) as usize;
                let (col, ascii) = match p.x >= ascii_x - cw {
                    true => (((p.x - ascii_x) / cw).max(0.0) as usize, true),
                    false => {
                        let x = ((p.x - hex_x) / cw).max(0.0) as usize;
                        ((if x >= 25 { x - 1 } else { x }) / 3, false)
                    }
                };
                self.ascii = ascii;
                self.go((row * WIDTH + col.min(WIDTH - 1)) as isize);
            }
        }
        let focused = ui.memory(|m| m.has_focus(id));
        if focused {
            ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false }));
            let events = ui.input(|i| i.events.clone());
            let at = self.cursor as isize;
            for event in events {
                match event {
                    Event::Text(text) => {
                        for c in text.chars() {
                            self.put(c);
                        }
                    }
                    Event::Key { key, pressed: true, modifiers, .. } => match key {
                        Key::ArrowLeft => self.go(at - 1),
                        Key::ArrowRight => self.go(at + 1),
                        Key::ArrowUp => self.go(at - WIDTH as isize),
                        Key::ArrowDown => self.go(at + WIDTH as isize),
                        Key::PageUp => self.go(at - (rows_shown * WIDTH) as isize),
                        Key::PageDown => self.go(at + (rows_shown * WIDTH) as isize),
                        Key::Home if modifiers.command => self.go(0),
                        Key::End if modifiers.command => self.go(isize::MAX / 2),
                        Key::Home => self.go(at - at % WIDTH as isize),
                        Key::End => self.go(at - at % WIDTH as isize + WIDTH as isize - 1),
                        Key::Tab => self.ascii ^= true,
                        Key::Z if modifiers.command => self.undo(),
                        _ => {}
                    },
                    _ => {}
                }
            }
            let row = self.cursor / WIDTH;
            if row < self.top {
                self.top = row;
            } else if row >= self.top + rows_shown {
                self.top = row + 1 - rows_shown;
            }
        }
        if resp.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            self.top = (self.top as isize - (wheel / row_h).round() as isize).max(0) as usize;
        }
        self.top = self.top.min(total_rows.saturating_sub(rows_shown));
        let painter = ui.painter().with_clip_rect(body);
        let cursor_color = ui::with_alpha(theme.accent, if focused { 0.45 } else { 0.2 });
        for r in 0..rows_shown.min(total_rows - self.top) {
            let row = self.top + r;
            let y = body.top() + r as f32 * row_h + 1.5;
            painter.text(pos2(body.left() + 12.0, y), Align2::LEFT_TOP, format!("{:08X}", row * WIDTH), font.clone(), theme.text_dim);
            for col in 0..WIDTH {
                let at = row * WIDTH + col;
                let Some(&b) = self.bytes.get(at) else { break };
                let x = hex_x + (col * 3 + (col >= 8) as usize) as f32 * cw;
                let ax = ascii_x + col as f32 * cw;
                if at == self.cursor {
                    let (hx, aw) = if self.ascii { (x, 2.0 * cw) } else { (x + if self.low { cw } else { 0.0 }, cw) };
                    let (strong, weak) = if self.ascii { ((ax, cw), (x, 2.0 * cw)) } else { ((hx, aw), (ax, cw)) };
                    painter.rect_filled(Rect::from_min_size(pos2(strong.0, y - 1.5), vec2(strong.1, row_h)), CornerRadius::ZERO, cursor_color);
                    painter.rect_filled(Rect::from_min_size(pos2(weak.0, y - 1.5), vec2(weak.1, row_h)), CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.12));
                }
                let color = match (self.edited.contains(&at), b) {
                    (true, _) => theme.warning,
                    (_, 0) => ui::mix(theme.text_dim, theme.bg, 0.4),
                    _ => theme.text,
                };
                painter.text(pos2(x, y), Align2::LEFT_TOP, format!("{b:02X}"), font.clone(), color);
                painter.text(pos2(ax, y), Align2::LEFT_TOP, shown(b), font.clone(), if b.is_ascii_graphic() { color } else { theme.text_dim });
            }
        }
        as_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_nibbles_and_ascii_then_saves_and_undoes() {
        let dir = std::env::temp_dir().join(format!("halo-hex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, [0u8, 1, 2, 3]).unwrap();
        let mut hex = HexView::open(path.clone());
        let bytes = hex.rx.take().unwrap().recv().unwrap().unwrap();
        hex.bytes = bytes;
        assert!(hex.put('a') && hex.put('B') && !hex.put('z'));
        assert_eq!((hex.bytes[0], hex.cursor, hex.dirty), (0xAB, 1, true));
        hex.ascii = true;
        assert!(hex.put('Z'));
        assert_eq!((hex.bytes[1], hex.cursor), (b'Z', 2));
        hex.save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0xAB, b'Z', 2, 3]);
        assert!(!hex.dirty);
        hex.undo();
        assert_eq!((hex.bytes[1], hex.cursor), (1, 1));
        assert_eq!((shown(b'a'), shown(0)), ('a', '.'));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
