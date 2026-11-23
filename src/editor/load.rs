use super::buffer::{Document, HUGE, Sel};
use super::highlight::Highlighter;
use super::image_view::{self, ImageView};
use super::intel::from_lsp;
use super::{Doc, Editors, Tab};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align, CornerRadius, Layout, Rect, RichText, Ui, UiBuilder, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

pub enum Place {
    Goto(usize, usize),
    Lsp((usize, usize)),
}

pub enum Content {
    Text,
    Deferred,
    Loading { rx: Receiver<Loaded>, at: Option<Place> },
    Binary,
    Huge(u64),
    Image(Box<ImageView>),
}

impl Content {
    pub fn is_text(&self) -> bool {
        matches!(self, Content::Text)
    }
}

pub enum Loaded {
    Doc(Box<Document>),
    Binary,
    Huge(u64),
    Failed(String),
}

pub fn spawn(path: PathBuf, force: bool) -> Receiver<Loaded> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(read(&path, force));
    });
    rx
}

fn read(path: &Path, force: bool) -> Loaded {
    let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) => return Loaded::Failed(format!("Cannot open {name}: {e}")),
    };
    if size > HUGE && !force {
        return Loaded::Huge(size);
    }
    match std::fs::read(path) {
        Ok(bytes) => Document::from_bytes(path, &bytes, None, force).map_or(Loaded::Binary, |doc| Loaded::Doc(Box::new(doc))),
        Err(e) => Loaded::Failed(format!("Cannot open {name}: {e}")),
    }
}

fn pending(path: &Path) -> Content {
    match image_view::is_image(path) {
        true => Content::Image(Box::new(ImageView::open(path.to_path_buf()))),
        false => Content::Loading { rx: spawn(path.to_path_buf(), false), at: None },
    }
}

fn notice_page(ui: &mut Ui, theme: &Theme, rect: Rect, text: &str, button: &str) -> bool {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
    let mut clicked = false;
    let area = Rect::from_center_size(rect.center(), vec2(rect.width().min(520.0), 140.0));
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(text).color(theme.text_dim));
            ui.add_space(12.0);
            clicked = ui.button(button).clicked();
        });
    });
    clicked
}

impl Editors {
    fn placeholder(&mut self, path: &Path, content: Content) -> u64 {
        let title = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
        let mut buf = Document::from_text("", title);
        buf.path = Some(path.to_path_buf());
        buf.read_only = true;
        let id = self.insert_doc(buf);
        if let Some(doc) = self.docs.get_mut(&id) {
            doc.content = content;
        }
        id
    }

    pub(super) fn insert_pending(&mut self, path: &Path) -> u64 {
        self.placeholder(path, pending(path))
    }

    pub(super) fn insert_deferred(&mut self, path: &Path) -> u64 {
        self.placeholder(path, Content::Deferred)
    }

    fn wake(&mut self, id: u64) {
        if let Some(doc) = self.docs.get_mut(&id).filter(|d| matches!(d.content, Content::Deferred))
            && let Some(path) = doc.buf.path.as_deref()
        {
            doc.content = pending(path);
        }
    }

    pub(super) fn poll_loads(&mut self, ctx: &egui::Context) {
        let waiting: Vec<u64> = self.docs.iter().filter(|(_, d)| matches!(d.content, Content::Loading { .. })).map(|(id, _)| *id).collect();
        for id in waiting {
            let Some(Content::Loading { rx, .. }) = self.docs.get(&id).map(|d| &d.content) else { continue };
            let loaded = match rx.try_recv() {
                Ok(loaded) => loaded,
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(30));
                    continue;
                }
                Err(TryRecvError::Disconnected) => Loaded::Failed("The file could not be read.".into()),
            };
            match loaded {
                Loaded::Doc(buf) => self.arrive(id, *buf),
                Loaded::Binary => self.docs.get_mut(&id).iter_mut().for_each(|d| d.content = Content::Binary),
                Loaded::Huge(size) => self.docs.get_mut(&id).iter_mut().for_each(|d| d.content = Content::Huge(size)),
                Loaded::Failed(error) => {
                    self.banner = Some((error, None));
                    let tabs: Vec<_> = (0..self.groups.len()).flat_map(|g| (0..self.groups[g].tabs.len()).rev().map(move |t| (g, t))).collect();
                    let tabs: Vec<_> = tabs.into_iter().filter(|(g, t)| self.groups[*g].tabs[*t].doc == id).collect();
                    for (g, t) in tabs {
                        self.close_tab(g, t);
                    }
                }
            }
        }
    }

    fn arrive(&mut self, id: u64, mut buf: Document) {
        buf.large = buf.rope.len_bytes() as u64 > self.large_bytes;
        if !self.config.of(buf.path.as_deref()).detect_indentation {
            buf.indent = None;
        }
        let note = buf.note.clone();
        let Some(doc) = self.docs.get_mut(&id) else { return };
        let at = match std::mem::replace(&mut doc.content, Content::Text) {
            Content::Loading { at, .. } => at,
            _ => None,
        };
        let caret = at.map(|place| match place {
            Place::Goto(line, column) => {
                let line = line.min(buf.lines() - 1);
                buf.line_start(line) + column.min(buf.line_len(line))
            }
            Place::Lsp(pos) => from_lsp(&buf, pos),
        });
        doc.hl = Highlighter::new(buf.path.as_deref().filter(|_| !buf.large), &buf.rope);
        doc.buf = buf;
        if let Some(note) = note {
            self.banner = Some((note, None));
        }
        if let Some(pos) = caret {
            for tab in self.groups.iter_mut().flat_map(|g| &mut g.tabs).filter(|t| t.doc == id && t.diff.is_none()) {
                tab.sels = vec![Sel::caret(pos)];
                tab.reveal = Some(pos);
            }
        }
    }

    pub(super) fn load_now(&mut self, id: u64) {
        self.wake(id);
        let path = self.docs.get(&id).filter(|d| matches!(d.content, Content::Loading { .. })).and_then(|d| d.buf.path.clone());
        if let Some(buf) = path.and_then(|path| Document::load(&path).ok()) {
            self.arrive(id, buf);
        }
    }

    pub(super) fn pend_place(&mut self, place: Place) -> bool {
        let Some((g, t)) = self.active_tab() else { return false };
        self.wake(self.groups[g].tabs[t].doc);
        match self.docs.get_mut(&self.groups[g].tabs[t].doc) {
            Some(Doc { content: Content::Loading { at, .. }, .. }) => {
                *at = Some(place);
                true
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn special(&mut self, ui: &mut Ui, g: usize, body: &mut Rect, theme: &Theme, settings: &Settings, doc: &mut Doc, tab: &mut Tab) -> bool {
        if let (Some(md), Content::Text) = (tab.md.as_mut(), &doc.content) {
            let out = md.ui(ui, theme, *body, &doc.buf, doc.top, settings.font_size + 1.0);
            self.previews.extend(out.into_iter().map(|out| (g, tab.doc, out)));
            return true;
        }
        let force = |doc: &mut Doc| {
            if let Some(path) = doc.buf.path.clone() {
                doc.content = Content::Loading { rx: spawn(path, true), at: None };
            }
        };
        match &mut doc.content {
            Content::Text => {
                if doc.buf.large {
                    let bar = Rect::from_min_size(body.min, vec2(body.width(), 26.0));
                    body.min.y = bar.bottom();
                    ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.panel);
                    ui.scope_builder(UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)), |ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new("Large file: highlighting, language features, minimap, git changes, blame, word wrap and local history are off.").size(12.0).color(theme.text_dim));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(6.0);
                            if ui.small_button("Enable Anyway").clicked() {
                                doc.buf.large = false;
                                doc.hl = Highlighter::new(doc.buf.path.as_deref(), &doc.buf.rope);
                            }
                        });
                    });
                }
                false
            }
            Content::Deferred | Content::Loading { .. } => {
                if matches!(doc.content, Content::Deferred)
                    && let Some(path) = doc.buf.path.as_deref()
                {
                    doc.content = pending(path);
                }
                ui.painter().rect_filled(*body, CornerRadius::ZERO, theme.bg);
                ui::spinner(ui, body.center(), 10.0, theme.text_dim);
                true
            }
            Content::Binary => {
                let text = "The file is not displayed in the text editor because it is either binary or uses an unsupported text encoding.";
                if notice_page(ui, theme, *body, text, "Open Anyway") {
                    force(doc);
                }
                true
            }
            Content::Huge(size) => {
                let text = format!("{} is {} MB. Opening a file this large can be slow and use a lot of memory.", doc.buf.title, *size >> 20);
                if notice_page(ui, theme, *body, &text, "Open Anyway") {
                    force(doc);
                }
                true
            }
            Content::Image(view) => {
                view.ui(ui, theme, *body, &format!("{}:{}", doc.buf.title, tab.doc));
                true
            }
        }
    }

    pub fn image_status(&self) -> Option<String> {
        let (g, t) = self.active_tab()?;
        match &self.docs.get(&self.groups[g].tabs[t].doc)?.content {
            Content::Image(view) => view.status(),
            _ => None,
        }
    }

    pub fn image_zoom(&mut self, by: Option<f32>) -> bool {
        let Some((g, t)) = self.active_tab() else { return false };
        match self.docs.get_mut(&self.groups[g].tabs[t].doc).map(|d| &mut d.content) {
            Some(Content::Image(view)) if view.ready() => {
                view.step(by);
                true
            }
            _ => false,
        }
    }

    pub(super) fn reopen_text(&mut self, id: u64) {
        if let Some(doc) = self.docs.get_mut(&id)
            && matches!(doc.content, Content::Image(_))
            && let Some(path) = doc.buf.path.clone()
        {
            doc.content = Content::Loading { rx: spawn(path, false), at: None };
        }
    }

    #[cfg(test)]
    pub(super) fn finish_loads(&mut self) {
        let ctx = egui::Context::default();
        for _ in 0..500 {
            self.poll_loads(&ctx);
            if !self.docs.values().any(|d| matches!(d.content, Content::Loading { .. })) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("loads did not finish");
    }
}

#[cfg(test)]
mod tests {
    use super::super::Command;
    use super::*;
    use crate::git::Git;
    use crate::settings::Settings;

    fn frame(ctx: &egui::Context, editors: &mut Editors) {
        let (theme, git, settings) = (Theme::dark(), Git::new(), Settings::default());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 600.0))), focused: true, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut vec![]));
        out.textures_delta.clear();
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content(editors: &Editors) -> &Content {
        &editors.docs.values().next().unwrap().content
    }

    #[test]
    fn binary_files_show_a_notice_and_never_save_until_opened_anyway() {
        let dir = dir("binary");
        let path = dir.join("data.bin");
        std::fs::write(&path, [b'M', b'Z', 0, 1, 2, 3, 0, 0, b'x']).unwrap();
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        editors.open(&path);
        editors.finish_loads();
        assert!(matches!(content(&editors), Content::Binary));
        frame(&ctx, &mut editors);
        editors.exec(&Command::Save, &settings, &ctx);
        assert!(!editors.save_doc(*editors.docs.keys().next().unwrap(), true));
        assert_eq!(std::fs::read(&path).unwrap(), [b'M', b'Z', 0, 1, 2, 3, 0, 0, b'x']);
        let id = *editors.docs.keys().next().unwrap();
        editors.docs.get_mut(&id).unwrap().content = Content::Loading { rx: spawn(path.clone(), true), at: None };
        editors.finish_loads();
        assert!(content(&editors).is_text() && editors.docs[&id].buf.len() == 9);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_load_in_the_background_and_keep_a_pending_jump() {
        let dir = dir("pending");
        let path = dir.join("a.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        editors.open(&path);
        assert!(matches!(content(&editors), Content::Loading { .. }));
        frame(&ctx, &mut editors);
        editors.exec(&Command::GoTo(3, 2), &settings, &ctx);
        editors.finish_loads();
        assert_eq!(editors.groups[0].tabs[0].sels[0].head, 9);
        assert_eq!(editors.docs.values().next().unwrap().buf.rope.to_string(), "one\ntwo\nthree\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_file_closes_its_tab_with_an_error() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.open(&std::env::temp_dir().join("halo-missing-file.txt"));
        for _ in 0..200 {
            frame(&ctx, &mut editors);
            if editors.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(editors.is_empty() && editors.docs.is_empty() && editors.banner.as_ref().is_some_and(|b| b.0.contains("Cannot open")));
    }

    #[test]
    fn large_files_turn_the_heavy_features_off_until_enabled() {
        let dir = dir("large");
        let path = dir.join("big.rs");
        std::fs::write(&path, "fn main() {}\n".repeat(50)).unwrap();
        let (ctx, mut editors) = (egui::Context::default(), Editors::new());
        editors.large_bytes = 100;
        editors.open(&path);
        editors.finish_loads();
        let doc = editors.docs.values_mut().next().unwrap();
        assert!(doc.buf.large && !doc.buf.read_only && doc.hl.language == "Plain Text");
        frame(&ctx, &mut editors);
        assert!(editors.docs.values().next().unwrap().lang.synced == 0);
        editors.docs.values_mut().next().unwrap().buf.large = false;
        let small = dir.join("small.rs");
        std::fs::write(&small, "fn a() {}\n").unwrap();
        editors.open(&small);
        editors.finish_loads();
        let id = editors.find_doc(&small).unwrap();
        assert!(!editors.docs[&id].buf.large && editors.docs[&id].hl.language != "Plain Text");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn images_open_as_image_tabs_and_zoom() {
        let dir = dir("image");
        let path = dir.join("a.png");
        image::RgbaImage::from_pixel(20, 10, image::Rgba([1, 2, 3, 255])).save(&path).unwrap();
        let (ctx, mut editors) = (egui::Context::default(), Editors::new());
        editors.open(&path);
        for _ in 0..400 {
            frame(&ctx, &mut editors);
            if editors.image_status().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(editors.image_status().unwrap().starts_with("20 x 10"));
        assert!(editors.image_zoom(Some(2.0)));
        frame(&ctx, &mut editors);
        assert!(editors.image_status().unwrap().ends_with("200%"), "{:?}", editors.image_status());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restored_tabs_load_when_first_shown_or_edited() {
        let dir = dir("deferred");
        let paths: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| dir.join(n)).collect();
        for path in &paths {
            std::fs::write(path, "alpha beta\n").unwrap();
        }
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        paths.iter().for_each(|p| editors.open_restored(p));
        let deferred = |editors: &Editors, p: &Path| matches!(editors.docs[&editors.find_doc(p).unwrap()].content, Content::Deferred);
        assert!(paths.iter().all(|p| deferred(&editors, p)));
        frame(&ctx, &mut editors);
        editors.finish_loads();
        assert!(!deferred(&editors, &paths[2]) && deferred(&editors, &paths[0]) && deferred(&editors, &paths[1]));
        assert_eq!(editors.replace_in(&paths[0], &crate::editor::Pattern::new("beta", false, false, false).unwrap(), "gamma", false), Ok(1));
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "alpha gamma\n");
        editors.show_tab(0, 1);
        frame(&ctx, &mut editors);
        editors.finish_loads();
        assert_eq!(editors.docs[&editors.find_doc(&paths[1]).unwrap()].buf.rope.to_string(), "alpha beta\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
