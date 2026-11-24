use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Align2, CornerRadius, FontId, Key, Layout, Rect, Sense, TextEdit, Ui, UiBuilder, pos2, vec2};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const STEP: usize = 1024;
const MAX_HITS: usize = 200_000;
const SHOWN: usize = 2000;

#[derive(Default)]
pub struct Index {
    marks: Vec<u64>,
    pub lines: usize,
    pub scanned: u64,
    pub done: bool,
    error: Option<String>,
}

impl Index {
    fn feed(&mut self, chunk: &[u8]) {
        if self.marks.is_empty() {
            (self.marks, self.lines) = (vec![0], 1);
        }
        for (i, b) in chunk.iter().enumerate() {
            if *b == b'\n' {
                if self.lines.is_multiple_of(STEP) {
                    self.marks.push(self.scanned + i as u64 + 1);
                }
                self.lines += 1;
            }
        }
        self.scanned += chunk.len() as u64;
    }
}

pub fn scan(index: &Mutex<Index>, path: &Path) -> Result<bool, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let from = index.lock().unwrap().scanned;
    if size < from {
        *index.lock().unwrap() = Index::default();
        return Ok(true);
    }
    if size == from {
        index.lock().unwrap().done = true;
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(from)).map_err(|e| e.to_string())?;
    let mut chunk = vec![0; 1 << 20];
    let mut left = size - from;
    while left > 0 {
        let n = file.read(&mut chunk[..(left as usize).min(1 << 20)]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let mut index = index.lock().unwrap();
        index.feed(&chunk[..n]);
        index.done = false;
        left -= n as u64;
    }
    index.lock().unwrap().done = true;
    Ok(true)
}

pub fn read_lines(index: &Mutex<Index>, path: &Path, top: usize, count: usize) -> Vec<String> {
    let (start, skip, total) = {
        let index = index.lock().unwrap();
        let Some(mark) = index.marks.get(top / STEP) else { return vec![] };
        (*mark, top % STEP, index.lines)
    };
    let Ok(mut file) = File::open(path) else { return vec![] };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut line = Vec::new();
    for _ in 0..skip {
        line.clear();
        if reader.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
            return vec![];
        }
    }
    let mut out = Vec::with_capacity(count);
    while out.len() < count.min(total.saturating_sub(top)) {
        line.clear();
        let read = reader.by_ref().take(SHOWN as u64 * 4).read_until(b'\n', &mut line).unwrap_or(0);
        if read == 0 {
            out.push(String::new());
            break;
        }
        if !line.ends_with(b"\n") {
            let mut rest = Vec::new();
            let _ = reader.read_until(b'\n', &mut rest);
        }
        let text = String::from_utf8_lossy(&line);
        out.push(text.trim_end_matches(['\n', '\r']).chars().take(SHOWN).map(|c| if c == '\t' { ' ' } else { c }).collect());
    }
    out
}

pub fn search(path: &Path, needle: &str, stop: &AtomicBool, hits: &Mutex<(Vec<usize>, bool)>) {
    let fold = !needle.chars().any(char::is_uppercase);
    let needle = if fold { needle.to_lowercase() } else { needle.to_owned() }.into_bytes();
    let Ok(file) = File::open(path) else { return hits.lock().unwrap().1 = true };
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let (mut line, mut number, mut found) = (Vec::new(), 0, Vec::new());
    while !stop.load(Ordering::Relaxed) {
        line.clear();
        if reader.read_until(b'\n', &mut line).unwrap_or(0) == 0 {
            break;
        }
        if fold {
            line.make_ascii_lowercase();
        }
        if line.windows(needle.len()).any(|w| w == needle.as_slice()) {
            found.push(number);
            if found.len() >= MAX_HITS {
                break;
            }
        }
        number += 1;
        if !found.is_empty() && number % 100_000 == 0 {
            hits.lock().unwrap().0.append(&mut found);
        }
    }
    let mut hits = hits.lock().unwrap();
    hits.0.append(&mut found);
    hits.1 = true;
}

pub struct StreamView {
    path: PathBuf,
    pub index: Arc<Mutex<Index>>,
    stop: Arc<AtomicBool>,
    top: usize,
    pub follow: bool,
    cache: (usize, usize, u64, Vec<String>),
    query: String,
    searched: String,
    hits: Arc<Mutex<(Vec<usize>, bool)>>,
    search_stop: Arc<AtomicBool>,
    current: Option<usize>,
    picked: Option<usize>,
}

impl Drop for StreamView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.search_stop.store(true, Ordering::Relaxed);
    }
}

impl StreamView {
    pub fn open(path: PathBuf, ctx: &egui::Context) -> Self {
        let (index, stop) = (Arc::new(Mutex::new(Index::default())), Arc::new(AtomicBool::new(false)));
        let (i, s, p, ctx) = (index.clone(), stop.clone(), path.clone(), ctx.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match scan(&i, &p) {
                    Ok(true) => ctx.request_repaint(),
                    Ok(false) => {}
                    Err(e) => {
                        i.lock().unwrap().error = Some(e);
                        ctx.request_repaint();
                    }
                }
                std::thread::sleep(Duration::from_millis(400));
            }
        });
        let done = Arc::new(Mutex::new((vec![], true)));
        StreamView { path, index, stop, top: 0, follow: false, cache: (usize::MAX, 0, 0, vec![]), query: String::new(), searched: String::new(), hits: done, search_stop: Arc::new(AtomicBool::new(false)), current: None, picked: None }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        self.search_stop.store(true, Ordering::Relaxed);
        (self.searched, self.current) = (self.query.clone(), None);
        self.hits = Arc::new(Mutex::new((vec![], self.query.is_empty())));
        self.search_stop = Arc::new(AtomicBool::new(false));
        if self.query.is_empty() {
            return;
        }
        let (path, needle, stop, hits, ctx) = (self.path.clone(), self.query.clone(), self.search_stop.clone(), self.hits.clone(), ctx.clone());
        std::thread::spawn(move || {
            search(&path, &needle, &stop, &hits);
            ctx.request_repaint();
        });
    }

    fn step(&mut self, forward: bool, rows: usize) {
        let hits = self.hits.lock().unwrap();
        let from = self.current.map_or(self.top, |c| hits.0.get(c).copied().unwrap_or(self.top));
        let at = match forward {
            true => hits.0.partition_point(|&h| h <= from).min(hits.0.len().saturating_sub(1)),
            false => hits.0.partition_point(|&h| h < from).saturating_sub(1),
        };
        let at = match (self.current, forward) {
            (None, true) => hits.0.partition_point(|&h| h < self.top),
            _ => at,
        };
        if let Some(&line) = hits.0.get(at) {
            self.current = Some(at);
            self.follow = false;
            self.top = line.saturating_sub(rows / 3);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, font_size: f32) -> bool {
        let mut open_editor = false;
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        let (lines, scanned, done, error) = {
            let index = self.index.lock().unwrap();
            (index.lines.max(1), index.scanned, index.done, index.error.clone())
        };
        let (hit_count, searching) = {
            let hits = self.hits.lock().unwrap();
            (hits.0.len(), !hits.1)
        };
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.panel);
        let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
        let font = FontId::monospace(font_size);
        let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font)) + 3.0;
        let rows = ((body.height() / row_h) as usize).max(1);
        let mut search_now = false;
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 3.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
            let status = format!("{}  ·  {} lines{}", super::image_view::size_label(scanned), lines, if done { "" } else { "  ·  indexing…" });
            ui.label(egui::RichText::new(status).size(12.0).color(theme.text_dim));
            ui.add_space(12.0);
            let edit = ui.add(TextEdit::singleline(&mut self.query).hint_text("Search").desired_width(220.0));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                match self.query == self.searched {
                    true => self.step(!ui.input(|i| i.modifiers.shift), rows),
                    false => search_now = true,
                }
                edit.request_focus();
            }
            if !self.searched.is_empty() {
                let label = match (self.current, searching) {
                    (Some(c), _) => format!("{} of {hit_count}{}", c + 1, if searching { "+" } else { "" }),
                    (None, true) => format!("{hit_count}+ found"),
                    (None, false) => format!("{hit_count} found"),
                };
                ui.label(egui::RichText::new(label).size(12.0).color(theme.text_dim));
            }
            if ui::icon_button(ui, theme, icons::ARROW_UP, 22.0, false, "Previous match (Shift+Enter)").clicked() {
                self.step(false, rows);
            }
            if ui::icon_button(ui, theme, icons::ARROW_DOWN, 22.0, false, "Next match (Enter)").clicked() {
                self.step(true, rows);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Open in Editor").on_hover_text("Load the whole file into the text editor").clicked() {
                    open_editor = true;
                }
                ui.add_space(8.0);
                ui.checkbox(&mut self.follow, "Follow").on_hover_text("Keep the end of the file in view as it grows");
            });
        });
        if search_now {
            self.start_search(ui.ctx());
        }
        if let Some(error) = error {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, format!("Cannot read the file: {error}"), FontId::proportional(13.0), theme.text_dim);
            return open_editor;
        }
        let resp = ui.interact(body, egui::Id::new(("stream_view", &self.path)), Sense::click_and_drag());
        let max_top = lines.saturating_sub(rows);
        if resp.hovered() {
            let (wheel, keys) = ui.input(|i| (i.smooth_scroll_delta.y, (i.key_pressed(Key::PageDown), i.key_pressed(Key::PageUp), i.modifiers.command && i.key_pressed(Key::End), i.modifiers.command && i.key_pressed(Key::Home))));
            let lines_moved = (-wheel / row_h).round() as isize;
            if lines_moved != 0 {
                self.top = (self.top as isize + lines_moved).max(0) as usize;
                self.follow &= lines_moved > 0;
            }
            match keys {
                (true, ..) => self.top += rows,
                (_, true, ..) => (self.top, self.follow) = (self.top.saturating_sub(rows), false),
                (_, _, true, _) => self.follow = true,
                (.., true) => (self.top, self.follow) = (0, false),
                _ => {}
            }
        }
        let track = Rect::from_min_max(pos2(body.right() - 10.0, body.top()), body.max);
        let thumb_h = (track.height() * rows as f32 / lines as f32).clamp(24.0, track.height());
        let bar_resp = ui.interact(track, egui::Id::new(("stream_bar", &self.path)), Sense::click_and_drag());
        if let Some(pos) = bar_resp.interact_pointer_pos().filter(|_| bar_resp.dragged() || bar_resp.clicked()) {
            let frac = ((pos.y - track.top() - thumb_h / 2.0) / (track.height() - thumb_h).max(1.0)).clamp(0.0, 1.0);
            self.top = (frac as f64 * max_top as f64).round() as usize;
            self.follow = self.top >= max_top;
        }
        if self.follow {
            self.top = max_top;
            ui.ctx().request_repaint_after(Duration::from_millis(400));
        }
        self.top = self.top.min(max_top);
        if (self.cache.0, self.cache.1, self.cache.2) != (self.top, rows, scanned) {
            self.cache = (self.top, rows, scanned, read_lines(&self.index, &self.path, self.top, rows));
        }
        let gutter = ((self.top + rows).to_string().len().max(4) as f32 + 2.0) * ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
        let painter = ui.painter().with_clip_rect(body);
        let current = self.current.and_then(|c| self.hits.lock().unwrap().0.get(c).copied());
        let needle = (!self.searched.is_empty()).then(|| self.searched.to_lowercase());
        let clicked = resp.clicked().then(|| resp.interact_pointer_pos()).flatten().map(|p| self.top + ((p.y - body.top()) / row_h) as usize);
        if clicked.is_some() {
            self.picked = clicked;
        }
        if resp.secondary_clicked() {
            self.picked = resp.interact_pointer_pos().map(|p| self.top + ((p.y - body.top()) / row_h) as usize);
        }
        for (i, text) in self.cache.3.iter().enumerate() {
            let line = self.top + i;
            let y = body.top() + i as f32 * row_h;
            let row = Rect::from_min_size(pos2(body.left(), y), vec2(body.width() - 10.0, row_h));
            if Some(line) == current || Some(line) == self.picked {
                painter.rect_filled(row, CornerRadius::ZERO, ui::with_alpha(theme.accent, if Some(line) == current { 0.18 } else { 0.1 }));
            }
            painter.text(pos2(body.left() + gutter, y + 1.5), Align2::RIGHT_TOP, (line + 1).to_string(), font.clone(), theme.text_dim);
            let x = body.left() + gutter + 14.0;
            if let Some(needle) = &needle {
                let lower = text.to_lowercase();
                let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'm'));
                for (at, _) in lower.match_indices(needle.as_str()).take(50) {
                    let col = lower[..at].chars().count() as f32;
                    let r = Rect::from_min_size(pos2(x + col * cw, y), vec2(needle.chars().count() as f32 * cw, row_h));
                    painter.rect_filled(r, CornerRadius::same(2), ui::with_alpha(theme.warning, 0.35));
                }
            }
            painter.text(pos2(x, y + 1.5), Align2::LEFT_TOP, text, font.clone(), theme.text);
        }
        let thumb_top = track.top() + (track.height() - thumb_h) * if max_top == 0 { 0.0 } else { self.top as f32 / max_top as f32 };
        let thumb = Rect::from_min_size(pos2(track.left() + 2.0, thumb_top), vec2(6.0, thumb_h));
        painter.rect_filled(thumb, CornerRadius::same(3), ui::with_alpha(theme.text_dim, if bar_resp.hovered() || bar_resp.dragged() { 0.5 } else { 0.25 }));
        let picked = self.picked.and_then(|p| p.checked_sub(self.top)).and_then(|i| self.cache.3.get(i)).cloned();
        resp.context_menu(|ui| {
            if ui.add_enabled(picked.is_some(), egui::Button::new("Copy Line")).clicked() {
                ui.ctx().copy_text(picked.clone().unwrap_or_default());
                ui.close();
            }
        });
        if resp.hovered() && ui.input(|i| i.modifiers.command && i.key_pressed(Key::C)) && let Some(text) = picked {
            ui.ctx().copy_text(text);
        }
        open_editor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_reads_searches_and_follows_a_growing_file() {
        let dir = std::env::temp_dir().join(format!("halo-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.log");
        let text: String = (0..5000).map(|n| format!("line {n}{}\n", if n % 1000 == 7 { " ERROR" } else { "" })).collect();
        std::fs::write(&path, &text).unwrap();
        let index = Mutex::new(Index::default());
        assert!(scan(&index, &path).unwrap());
        assert_eq!(index.lock().unwrap().lines, 5001);
        assert_eq!(read_lines(&index, &path, 0, 2), ["line 0", "line 1"]);
        assert_eq!(read_lines(&index, &path, 2049, 2), ["line 2049", "line 2050"]);
        assert_eq!(read_lines(&index, &path, 4999, 5), ["line 4999", ""]);
        let hits = Mutex::new((vec![], false));
        search(&path, "error", &AtomicBool::new(false), &hits);
        assert_eq!(hits.lock().unwrap().0, [7, 1007, 2007, 3007, 4007]);
        let hits = Mutex::new((vec![], false));
        search(&path, "Error", &AtomicBool::new(false), &hits);
        assert!(hits.lock().unwrap().0.is_empty());
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"tail\n").unwrap();
        assert!(scan(&index, &path).unwrap());
        assert!(!scan(&index, &path).unwrap());
        assert_eq!(read_lines(&index, &path, 5000, 1), ["tail"]);
        std::fs::write(&path, "short\n").unwrap();
        assert!(scan(&index, &path).unwrap());
        scan(&index, &path).unwrap();
        let lines = index.lock().unwrap().lines;
        assert_eq!((lines, read_lines(&index, &path, 0, 1)), (2, vec!["short".to_owned()]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    use std::io::Write;
}
