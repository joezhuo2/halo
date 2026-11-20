mod buffer;
mod highlight;

use crate::command::{self, Command, Scope};
use crate::git::{Git, LineChange};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use buffer::{After, Document, Kind, Motion, Sel, merge_sels};
use eframe::egui::text::{CCursor, LayoutJob, TextFormat};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Event, EventFilter, FontId, Id, Key, Layout, PointerButton,
    Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, ViewportCommand, pos2,
    scroll_area::ScrollBarVisibility, vec2,
};
use highlight::{Highlighter, Spans};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TAB_H: f32 = 34.0;
const MAX_COLS: usize = 5000;

pub struct EditorStatus {
    pub line: usize,
    pub column: usize,
    pub selected: usize,
    pub cursors: usize,
    pub language: String,
}

struct Doc {
    buf: Document,
    hl: Highlighter,
    max_cols: (u64, usize),
}

#[derive(Default)]
struct Tab {
    doc: u64,
    sels: Vec<Sel>,
    reveal: Option<usize>,
    set_scroll: Option<f32>,
    drag_anchor: Option<usize>,
}

#[derive(Default)]
struct Group {
    tabs: Vec<Tab>,
    active: usize,
}

#[derive(Default)]
struct Find {
    open: bool,
    replace: bool,
    query: String,
    with: String,
    case: bool,
    word: bool,
    focus: bool,
    matches: Vec<(usize, usize)>,
    key: (u64, u64, String, bool, bool),
}

enum TabAction {
    Activate(usize),
    Close(usize),
    CloseOthers(usize),
    CloseAll,
    CopyPath(usize),
}

pub struct Editors {
    docs: HashMap<u64, Doc>,
    next_id: u64,
    untitled: usize,
    groups: Vec<Group>,
    active: usize,
    focused: bool,
    want_focus: bool,
    find: Find,
    goto: Option<String>,
    goto_focus: bool,
    closing: Option<(usize, usize)>,
    banner: Option<String>,
    last_input: f64,
}

fn comment_tokens(path: Option<&Path>) -> (&'static str, Option<&'static str>) {
    let ext = path.and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "py" | "sh" | "bash" | "zsh" | "rb" | "toml" | "yaml" | "yml" | "ps1" | "r" | "pl" | "conf" | "ini" | "cfg"
        | "mk" | "env" | "gitignore" => ("#", None),
        "lua" | "sql" | "hs" => ("--", None),
        "html" | "htm" | "xml" | "svg" | "md" | "vue" => ("<!--", Some("-->")),
        "css" => ("/*", Some("*/")),
        "bat" | "cmd" => ("REM", None),
        "tex" => ("%", None),
        "lisp" | "clj" | "el" | "asm" => (";", None),
        _ => ("//", None),
    }
}

fn visible_text(buf: &Document, line: usize) -> String {
    let text = buf.line_text(line);
    if text.len() <= MAX_COLS {
        return text;
    }
    text.chars().take(MAX_COLS).collect()
}

fn job(text: &str, spans: Option<&Spans>, font: &FontId, color: Color32) -> LayoutJob {
    let Some(spans) = spans.filter(|s| !s.is_empty()) else {
        return LayoutJob::simple_singleline(text.to_owned(), font.clone(), color);
    };
    let mut job = LayoutJob::default();
    let mut at = 0;
    for (len, c) in spans.iter() {
        let end = (at + len).min(text.len());
        if at >= text.len() || !text.is_char_boundary(end) {
            break;
        }
        job.append(&text[at..end], 0.0, TextFormat::simple(font.clone(), *c));
        at = end;
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, TextFormat::simple(font.clone(), color));
    }
    job
}

fn handle_event(buf: &mut Document, tab: &mut Tab, event: &Event, settings: &Settings, page: usize, ctx: &egui::Context) -> bool {
    let unit = if settings.insert_spaces { " ".repeat(settings.tab_size) } else { "\t".to_owned() };
    let sels = &mut tab.sels;
    match event {
        Event::Text(text) => {
            buf.insert_text(sels, text);
        }
        Event::Paste(text) => {
            buf.insert_text(sels, &text.replace("\r\n", "\n").replace('\r', "\n"));
        }
        Event::Copy => {
            ctx.copy_text(buf.copy_text(sels));
            return false;
        }
        Event::Cut => {
            ctx.copy_text(buf.copy_text(sels));
            buf.cut(sels);
        }
        Event::Key { key, pressed: true, modifiers, .. } => {
            let (extend, word) = (modifiers.shift, modifiers.command);
            let motion = match key {
                Key::ArrowLeft => Some(if word { Motion::WordLeft } else { Motion::Left }),
                Key::ArrowRight => Some(if word { Motion::WordRight } else { Motion::Right }),
                Key::ArrowUp => Some(Motion::Up),
                Key::ArrowDown => Some(Motion::Down),
                Key::Home => Some(if word { Motion::DocStart } else { Motion::Home }),
                Key::End => Some(if word { Motion::DocEnd } else { Motion::End }),
                Key::PageUp => Some(Motion::PageUp(page)),
                Key::PageDown => Some(Motion::PageDown(page)),
                _ => None,
            };
            if let Some(motion) = motion {
                buf.move_sels(sels, motion, extend);
                return true;
            }
            match key {
                Key::Enter => buf.newline(sels, &unit),
                Key::Tab if modifiers.shift => buf.outdent(sels, settings.tab_size),
                Key::Tab => buf.tab(sels, settings.tab_size, settings.insert_spaces),
                Key::Backspace => buf.backspace(sels, word),
                Key::Delete => buf.delete(sels, word),
                Key::Escape => {
                    let first = sels[0];
                    *sels = vec![Sel::caret(first.head)];
                    true
                }
                _ => return false,
            };
        }
        _ => return false,
    }
    true
}

impl Editors {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            next_id: 1,
            untitled: 0,
            groups: vec![Group::default()],
            active: 0,
            focused: false,
            want_focus: false,
            find: Find::default(),
            goto: None,
            goto_focus: false,
            closing: None,
            banner: None,
            last_input: 0.0,
        }
    }

    fn active_tab(&self) -> Option<(usize, usize)> {
        let g = self.active.min(self.groups.len().checked_sub(1)?);
        let group = &self.groups[g];
        (!group.tabs.is_empty()).then(|| (g, group.active.min(group.tabs.len() - 1)))
    }

    fn with_active<R>(&mut self, f: impl FnOnce(&mut Document, &mut Tab) -> R) -> Option<R> {
        let (g, t) = self.active_tab()?;
        let tab = &mut self.groups[g].tabs[t];
        let doc = self.docs.get_mut(&tab.doc)?;
        doc.buf.clamp(&mut tab.sels);
        let result = f(&mut doc.buf, tab);
        tab.reveal = tab.sels.last().map(|s| s.head);
        Some(result)
    }

    fn insert_doc(&mut self, buf: Document) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(note) = &buf.note {
            self.banner = Some(note.clone());
        }
        let hl = Highlighter::new(buf.path.as_deref(), &buf.rope);
        self.docs.insert(id, Doc { buf, hl, max_cols: (0, 0) });
        id
    }

    fn add_tab(&mut self, doc: u64, sels: Vec<Sel>) {
        self.active = self.active.min(self.groups.len() - 1);
        let group = &mut self.groups[self.active];
        group.tabs.push(Tab { doc, sels, ..Default::default() });
        group.active = group.tabs.len() - 1;
        self.want_focus = true;
    }

    pub fn open(&mut self, path: &Path) {
        self.active = self.active.min(self.groups.len() - 1);
        let existing = self.docs.iter().find(|(_, d)| d.buf.path.as_deref() == Some(path)).map(|(id, _)| *id);
        if let Some(id) = existing {
            return self.show_doc(id);
        }
        match Document::load(path) {
            Ok(buf) => {
                let id = self.insert_doc(buf);
                self.add_tab(id, vec![Sel::caret(0)]);
            }
            Err(error) => self.banner = Some(error),
        }
    }

    fn show_doc(&mut self, id: u64) {
        let group = &mut self.groups[self.active];
        match group.tabs.iter().position(|t| t.doc == id) {
            Some(t) => {
                group.active = t;
                self.want_focus = true;
            }
            None => self.add_tab(id, vec![Sel::caret(0)]),
        }
    }

    pub fn open_text(&mut self, title: &str, text: &str) {
        self.active = self.active.min(self.groups.len() - 1);
        let text = if text.is_empty() { "No differences.\n".to_owned() } else { text.replace("\r\n", "\n") };
        let mut buf = Document::from_text(&text, title.to_owned());
        buf.read_only = true;
        let existing = self.docs.iter().find(|(_, d)| d.buf.read_only && d.buf.title == title).map(|(id, _)| *id);
        let id = existing.unwrap_or_else(|| self.insert_doc(Document::from_text("", String::new())));
        if let Some(doc) = self.docs.get_mut(&id) {
            doc.hl = Highlighter::new(Some(Path::new("changes.diff")), &buf.rope);
            doc.max_cols = (0, 0);
            doc.buf = buf;
        }
        self.show_doc(id);
    }

    fn references(&self, doc: u64) -> usize {
        self.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.doc == doc).count()
    }

    fn close_tab(&mut self, g: usize, t: usize) {
        let Some(group) = self.groups.get_mut(g) else { return };
        if t >= group.tabs.len() {
            return;
        }
        let doc = group.tabs.remove(t).doc;
        if group.active >= t && group.active > 0 {
            group.active -= 1;
        }
        if self.references(doc) == 0 {
            self.docs.remove(&doc);
        }
        self.want_focus = true;
    }

    fn request_close(&mut self, g: usize, t: usize) {
        let Some(doc) = self.groups.get(g).and_then(|group| group.tabs.get(t)).map(|tab| tab.doc) else { return };
        let dirty = self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty());
        if dirty && self.references(doc) == 1 {
            self.closing = Some((g, t));
        } else {
            self.close_tab(g, t);
        }
    }

    fn save_doc(&mut self, id: u64, ask: bool) -> bool {
        let Some(doc) = self.docs.get_mut(&id) else { return false };
        if doc.buf.path.is_none() || ask {
            let mut dialog = rfd::FileDialog::new().set_file_name(&doc.buf.title);
            if let Some(dir) = doc.buf.path.as_ref().and_then(|p| p.parent()) {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.save_file() else { return false };
            doc.hl = Highlighter::new(Some(&path), &doc.buf.rope);
            doc.buf.path = Some(path);
        }
        match doc.buf.save() {
            Ok(()) => true,
            Err(error) => {
                self.banner = Some(error);
                false
            }
        }
    }

    fn open_find(&mut self, replace: bool) {
        let selected = self
            .with_active(|buf, tab| {
                let s = tab.sels[0];
                (!s.is_empty() && buf.line_of(s.start()) == buf.line_of(s.end())).then(|| buf.text_of(s.start(), s.end()))
            })
            .flatten();
        if let Some(text) = selected {
            self.find.query = text;
        }
        self.find.replace = replace || (self.find.open && self.find.replace);
        self.find.open = true;
        self.find.focus = true;
    }

    pub fn exec(&mut self, cmd: &Command, settings: &Settings, ctx: &egui::Context) -> bool {
        use Command::*;
        ctx.request_repaint();
        match cmd {
            NewFile => {
                self.untitled += 1;
                let id = self.insert_doc(Document::from_text("", format!("Untitled-{}", self.untitled)));
                self.add_tab(id, vec![Sel::caret(0)]);
                return true;
            }
            SaveAll => {
                let ids: Vec<u64> = self.docs.iter().filter(|(_, d)| d.buf.is_dirty() && d.buf.path.is_some()).map(|(id, _)| *id).collect();
                for id in ids {
                    self.save_doc(id, false);
                }
                return true;
            }
            _ => {}
        }
        let Some((g, t)) = self.active_tab() else { return false };
        let doc_id = self.groups[g].tabs[t].doc;
        let unit = if settings.insert_spaces { " ".repeat(settings.tab_size) } else { "\t".to_owned() };
        let _ = unit;
        let mut refocus = true;
        match cmd {
            Save => {
                self.save_doc(doc_id, false);
            }
            SaveAs => {
                self.save_doc(doc_id, true);
            }
            CloseEditor => self.request_close(g, t),
            SplitEditor => {
                let sels = self.groups[g].tabs[t].sels.clone();
                self.groups.insert(g + 1, Group::default());
                self.active = g + 1;
                self.add_tab(doc_id, sels);
            }
            NextEditor | PrevEditor => {
                let group = &mut self.groups[g];
                let count = group.tabs.len();
                group.active = if *cmd == NextEditor { (t + 1) % count } else { (t + count - 1) % count };
            }
            Find | Replace => {
                self.open_find(*cmd == Replace);
                refocus = false;
            }
            GoToLine => {
                self.goto = Some(String::new());
                self.goto_focus = true;
                refocus = false;
            }
            Paste => ctx.send_viewport_cmd(ViewportCommand::RequestPaste),
            Undo | Redo | Cut | Copy | ToggleComment | SelectAll | AddCursorAbove | AddCursorBelow | SelectNextOccurrence => {
                let cmd = cmd.clone();
                let ctx = ctx.clone();
                self.with_active(|buf, tab| {
                    let sels = &mut tab.sels;
                    match cmd {
                        Undo => {
                            buf.undo(sels);
                        }
                        Redo => {
                            buf.redo(sels);
                        }
                        Copy => ctx.copy_text(buf.copy_text(sels)),
                        Cut => {
                            ctx.copy_text(buf.copy_text(sels));
                            buf.cut(sels);
                        }
                        ToggleComment => {
                            let (open, close) = comment_tokens(buf.path.as_deref());
                            buf.toggle_comment(sels, open, close);
                        }
                        SelectAll => *sels = vec![Sel::new(0, buf.len())],
                        AddCursorAbove => {
                            buf.add_cursor(sels, false);
                        }
                        AddCursorBelow => {
                            buf.add_cursor(sels, true);
                        }
                        _ => {
                            buf.select_next_occurrence(sels);
                        }
                    }
                });
                if matches!(cmd, SelectAll | AddCursorAbove)
                    && let Some((g, t)) = self.active_tab()
                {
                    let tab = &mut self.groups[g].tabs[t];
                    tab.reveal = if cmd == SelectAll { None } else { tab.sels.first().map(|s| s.head) };
                }
            }
            _ => return false,
        }
        self.last_input = ctx.input(|i| i.time);
        if refocus {
            self.want_focus = true;
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.tabs.is_empty())
    }

    pub fn open_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = vec![];
        for tab in self.groups.iter().flat_map(|g| &g.tabs) {
            if let Some(path) = self.docs.get(&tab.doc).and_then(|d| d.buf.path.clone())
                && !out.contains(&path)
            {
                out.push(path);
            }
        }
        out
    }

    pub fn active_path(&self) -> Option<PathBuf> {
        let (g, t) = self.active_tab()?;
        self.docs.get(&self.groups[g].tabs[t].doc)?.buf.path.clone()
    }

    pub fn status(&self) -> Option<EditorStatus> {
        let (g, t) = self.active_tab()?;
        let tab = &self.groups[g].tabs[t];
        let doc = self.docs.get(&tab.doc)?;
        let head = tab.sels.first()?.head.min(doc.buf.len());
        let line = doc.buf.line_of(head);
        Some(EditorStatus {
            line: line + 1,
            column: head - doc.buf.line_start(line) + 1,
            selected: tab.sels.iter().map(|s| s.end() - s.start()).sum(),
            cursors: tab.sels.len(),
            language: doc.hl.language.clone(),
        })
    }

    pub fn has_focus(&self) -> bool {
        self.focused
    }

    pub fn any_dirty(&self) -> bool {
        self.docs.values().any(|d| d.buf.is_dirty())
    }

    pub fn discard_all(&mut self) {
        for doc in self.docs.values_mut() {
            doc.buf.mark_clean();
        }
    }

    pub fn reload_clean(&mut self) {
        for doc in self.docs.values_mut().filter(|d| !d.buf.is_dirty()) {
            let Some(path) = doc.buf.path.clone() else { continue };
            if let Ok(buf) = Document::load(&path)
                && buf.rope != doc.buf.rope
            {
                doc.hl = Highlighter::new(Some(&path), &buf.rope);
                doc.max_cols = (0, 0);
                doc.buf = buf;
            }
        }
    }

    pub fn path_changed(&mut self, old: &Path, new: Option<&Path>) {
        let affected: Vec<u64> = self
            .docs
            .iter()
            .filter(|(_, d)| d.buf.path.as_ref().is_some_and(|p| p.starts_with(old)))
            .map(|(id, _)| *id)
            .collect();
        for id in affected {
            let Some(doc) = self.docs.get_mut(&id) else { continue };
            let Some(path) = doc.buf.path.clone() else { continue };
            match new {
                Some(new) => {
                    let rest = path.strip_prefix(old).unwrap_or(Path::new(""));
                    let target = if rest.as_os_str().is_empty() { new.to_path_buf() } else { new.join(rest) };
                    doc.buf.title = target.file_name().unwrap_or(target.as_os_str()).to_string_lossy().into_owned();
                    doc.hl = Highlighter::new(Some(&target), &doc.buf.rope);
                    doc.buf.path = Some(target);
                }
                None if doc.buf.is_dirty() => {
                    doc.buf.path = None;
                    doc.buf.mark_dirty();
                }
                None => {
                    for group in &mut self.groups {
                        group.tabs.retain(|t| t.doc != id);
                        group.active = group.active.min(group.tabs.len().saturating_sub(1));
                    }
                    self.docs.remove(&id);
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, settings: &Settings, git: &Git, _cmds: &mut Vec<Command>) {
        self.focused = false;
        self.active = self.active.min(self.groups.len().saturating_sub(1));
        let rect = ui.available_rect_before_wrap();
        let count = self.groups.len();
        let width = rect.width() / count as f32;
        let mut pending = vec![];
        for g in 0..count {
            let area = Rect::from_min_size(pos2(rect.left() + g as f32 * width, rect.top()), vec2(width, rect.height()));
            let fade = ui::fade_in(ui, Id::new(("editor_group_fade", g, count)));
            ui.scope_builder(UiBuilder::new().max_rect(area).id_salt(("editor_group", g)), |ui| {
                ui.set_clip_rect(area.intersect(ui.clip_rect()));
                if count > 1 && g + 1 == count {
                    ui.set_opacity(fade);
                }
                self.group_ui(ui, g, area, theme, settings, git, &mut pending);
            });
            if g > 0 {
                ui.painter().vline(area.left(), area.y_range(), Stroke::new(1.0, theme.border));
            }
        }
        ui.allocate_rect(rect, Sense::hover());
        for cmd in pending {
            self.exec(&cmd, settings, ui.ctx());
        }
        self.dialogs(ui.ctx());
        if self.groups.len() > 1 && self.closing.is_none() {
            let before = self.groups.len();
            let mut index = 0;
            let mut active = self.active;
            self.groups.retain(|group| {
                let keep = !group.tabs.is_empty();
                if !keep && index < active {
                    active -= 1;
                }
                index += 1;
                keep
            });
            if self.groups.is_empty() {
                self.groups.push(Group::default());
            }
            self.active = active.min(self.groups.len() - 1);
            if self.groups.len() != before {
                self.want_focus = true;
            }
        }
    }

    fn group_ui(&mut self, ui: &mut Ui, g: usize, rect: Rect, theme: &Theme, settings: &Settings, git: &Git, pending: &mut Vec<Command>) {
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), TAB_H));
        self.tab_strip(ui, g, strip, theme, pending);
        let body = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
        if self.groups[g].tabs.is_empty() {
            ui.painter().rect_filled(body, CornerRadius::ZERO, theme.bg);
            return;
        }
        let t = self.groups[g].active.min(self.groups[g].tabs.len() - 1);
        self.groups[g].active = t;
        let mut tab = std::mem::take(&mut self.groups[g].tabs[t]);
        let doc_id = tab.doc;
        if let Some(mut doc) = self.docs.remove(&doc_id) {
            self.view(ui, g, body, theme, settings, git, pending, &mut doc, &mut tab);
            self.docs.insert(doc_id, doc);
        }
        self.groups[g].tabs[t] = tab;
    }

    fn tab_strip(&mut self, ui: &mut Ui, g: usize, strip: Rect, theme: &Theme, pending: &mut Vec<Command>) {
        ui.painter().rect_filled(strip, CornerRadius::ZERO, theme.panel);
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, theme.border));
        let infos: Vec<(String, bool, Option<PathBuf>)> = self.groups[g]
            .tabs
            .iter()
            .filter_map(|t| self.docs.get(&t.doc))
            .map(|d| (d.buf.title.clone(), d.buf.is_dirty(), d.buf.path.clone()))
            .collect();
        let active = self.groups[g].active;
        let active_group = g == self.active;
        let mut action = None;
        let tabs_rect = Rect::from_min_max(strip.min, pos2(strip.right() - 34.0, strip.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(tabs_rect), |ui| {
            ScrollArea::horizontal().id_salt(("editor_tabs", g)).scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (t, (title, dirty, path)) in infos.iter().enumerate() {
                        let galley = ui.painter().layout_no_wrap(title.clone(), FontId::proportional(13.0), Color32::PLACEHOLDER);
                        let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 62.0, TAB_H), Sense::click());
                        let id = Id::new(("editor_tab", g, t));
                        let a = ui::anim(ui, id.with("active"), t == active);
                        let h = ui::anim(ui, id.with("hover"), resp.hovered());
                        let close = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
                        let over_close = resp.hovered() && ui.rect_contains_pointer(close);
                        let ch = ui::anim(ui, id.with("close"), over_close);
                        let p = ui.painter();
                        p.rect_filled(r, CornerRadius::ZERO, ui::mix(ui::mix(theme.panel, theme.hover, h * 0.5), theme.bg, a));
                        p.vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, theme.border));
                        if a > 0.0 {
                            let line = Rect::from_min_max(r.min, pos2(r.right() - 1.0, r.top() + 2.0));
                            let color = if active_group { theme.accent } else { theme.text_dim };
                            p.rect_filled(line, CornerRadius::ZERO, ui::with_alpha(color, a));
                        }
                        p.text(
                            pos2(r.left() + 12.0, r.center().y),
                            Align2::LEFT_CENTER,
                            icons::FILE_CODE,
                            FontId::proportional(14.0),
                            ui::mix(theme.text_dim, theme.accent, a * 0.8),
                        );
                        let text_pos = pos2(r.left() + 32.0, r.center().y - galley.size().y / 2.0);
                        p.galley(text_pos, galley, ui::mix(theme.text_dim, theme.text, a.max(h)));
                        if *dirty && !over_close {
                            p.circle_filled(close.center(), 4.0, ui::mix(theme.text_dim, theme.text, a));
                        } else if h > 0.0 || t == active {
                            p.rect_filled(close, CornerRadius::same(4), ui::mix(Color32::TRANSPARENT, theme.hover, ch));
                            let color = ui::with_alpha(ui::mix(theme.text_dim, theme.text, ch), h.max(a));
                            p.text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(12.0), color);
                        }
                        if resp.clicked() {
                            action = Some(if over_close { TabAction::Close(t) } else { TabAction::Activate(t) });
                        }
                        if resp.middle_clicked() {
                            action = Some(TabAction::Close(t));
                        }
                        let tip = path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| title.clone());
                        resp.on_hover_text(tip).context_menu(|ui| {
                            ui.set_min_width(160.0);
                            for (label, choice) in [
                                ("Close", TabAction::Close(t)),
                                ("Close Others", TabAction::CloseOthers(t)),
                                ("Close All", TabAction::CloseAll),
                                ("Copy Path", TabAction::CopyPath(t)),
                            ] {
                                if ui.button(label).clicked() {
                                    action = Some(choice);
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            });
        });
        let split = Rect::from_center_size(pos2(strip.right() - 17.0, strip.center().y), vec2(24.0, 24.0));
        ui.scope_builder(UiBuilder::new().max_rect(split), |ui| {
            if ui::icon_button(ui, theme, icons::SQUARE_SPLIT_HORIZONTAL, 24.0, false, "Split Editor Right (Ctrl+\\)").clicked() {
                self.active = g;
                pending.push(Command::SplitEditor);
            }
        });
        match action {
            Some(TabAction::Activate(t)) => {
                self.groups[g].active = t;
                self.active = g;
                self.want_focus = true;
            }
            Some(TabAction::Close(t)) => self.request_close(g, t),
            Some(TabAction::CloseOthers(keep)) => {
                for t in (0..self.groups[g].tabs.len()).rev().filter(|t| *t != keep) {
                    let doc = self.groups[g].tabs[t].doc;
                    if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                        self.close_tab(g, t);
                    }
                }
            }
            Some(TabAction::CloseAll) => {
                for t in (0..self.groups[g].tabs.len()).rev() {
                    let doc = self.groups[g].tabs[t].doc;
                    if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                        self.close_tab(g, t);
                    }
                }
            }
            Some(TabAction::CopyPath(t)) => {
                if let Some(path) = infos.get(t).and_then(|i| i.2.as_ref()) {
                    ui.ctx().copy_text(path.to_string_lossy().into_owned());
                }
            }
            None => {}
        }
    }

    fn refresh_matches(&mut self, doc_id: u64, buf: &Document, tab: &mut Tab) {
        let f = &mut self.find;
        let key = &f.key;
        if key.0 == doc_id && key.1 == buf.version && key.2 == f.query && key.3 == f.case && key.4 == f.word {
            return;
        }
        let typed = key.2 != f.query || key.3 != f.case || key.4 != f.word;
        f.matches = buf.find_all(&f.query, f.case, f.word);
        f.key = (doc_id, buf.version, f.query.clone(), f.case, f.word);
        if typed && let Some(first) = tab.sels.first() {
            let from = first.start();
            if let Some(m) = f.matches.iter().find(|m| m.0 >= from).or(f.matches.first()) {
                tab.sels = vec![Sel::new(m.0, m.1)];
                tab.reveal = Some(m.0);
            }
        }
    }

    fn step_match(&mut self, tab: &mut Tab, forward: bool) {
        let matches = &self.find.matches;
        let Some(current) = tab.sels.first().copied() else { return };
        let on_match = matches.iter().any(|m| m.0 == current.start() && m.1 == current.end());
        let found = if forward {
            matches.iter().find(|m| if on_match { m.0 > current.start() } else { m.0 >= current.start() }).or(matches.first())
        } else {
            matches.iter().rev().find(|m| m.0 < current.start()).or(matches.last())
        };
        if let Some(m) = found {
            tab.sels = vec![Sel::new(m.0, m.1)];
            tab.reveal = Some(m.0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn view(
        &mut self,
        ui: &mut Ui,
        g: usize,
        body: Rect,
        theme: &Theme,
        settings: &Settings,
        git: &Git,
        pending: &mut Vec<Command>,
        doc: &mut Doc,
        tab: &mut Tab,
    ) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let doc_id = tab.doc;
        let id = Id::new(("editor_view", g, doc_id));
        let is_active = g == self.active;
        let font = FontId::monospace(settings.font_size);
        let (char_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.row_height(&font)));
        let line_h = (row_h * 1.25).round();
        let Doc { buf, hl, max_cols } = doc;
        if let Some(line) = buf.dirty_from.take() {
            hl.invalidate(line);
        }
        buf.clamp(&mut tab.sels);

        ui.painter().rect_filled(body, CornerRadius::ZERO, theme.bg);
        let minimap_w = if settings.minimap && body.width() > 420.0 { 78.0 } else { 0.0 };
        let text_rect = Rect::from_min_max(body.min, pos2(body.right() - minimap_w, body.bottom()));

        if self.want_focus && is_active && self.goto.is_none() {
            ui.memory_mut(|m| m.request_focus(id));
            self.want_focus = false;
        }
        let has_focus = ui.memory(|m| m.has_focus(id));
        if has_focus {
            self.focused = true;
            self.active = g;
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
            });
            for binding in command::bindings().iter().filter(|b| b.scope == Scope::Editor) {
                if ui.input_mut(|i| i.consume_shortcut(&binding.shortcut)) {
                    pending.push(binding.command.clone());
                }
            }
            let page = ((text_rect.height() / line_h) as usize).saturating_sub(2).max(1);
            let events = ui.input(|i| i.events.clone());
            for event in &events {
                if handle_event(buf, tab, event, settings, page, &ctx) {
                    self.last_input = now;
                    tab.reveal = tab.sels.last().map(|s| s.head);
                }
            }
            if let Some(line) = buf.dirty_from.take() {
                hl.invalidate(line);
            }
        }
        let find_open = self.find.open && is_active;
        if find_open {
            self.refresh_matches(doc_id, buf, tab);
        }

        let total = buf.lines();
        if max_cols.0 != buf.version {
            *max_cols = (buf.version, (0..total).map(|l| buf.line_len(l)).max().unwrap_or(0).min(MAX_COLS));
        }
        let digits = total.to_string().len().max(3) as f32;
        let gutter_w = (digits + 3.0) * char_w + 6.0;
        let content = vec2(gutter_w + (max_cols.1 as f32 + 10.0) * char_w, total as f32 * line_h + (text_rect.height() * 0.5).max(line_h));
        let idle = now - self.last_input;
        let caret_alpha = if !has_focus {
            0.35
        } else if !(0.5..=30.0).contains(&idle) {
            1.0
        } else {
            0.5 + 0.5 * ((idle - 0.5) * std::f64::consts::TAU / 1.2).cos() as f32
        };
        if has_focus && idle <= 30.0 {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        let matches: &[(usize, usize)] = if find_open { &self.find.matches } else { &[] };
        let path = buf.path.clone();
        let mut activated = false;
        let mut touched = false;
        let mut area = ScrollArea::both().id_salt(id.with("scroll")).auto_shrink(false);
        if let Some(y) = tab.set_scroll.take() {
            area = area.vertical_scroll_offset(y.max(0.0));
        }
        let buf_ref: &Document = buf;
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                area.show_viewport(ui, |ui, viewport| {
                    let (content_rect, _) = ui.allocate_exact_size(vec2(content.x.max(viewport.width()), content.y), Sense::hover());
                    let origin = content_rect.min;
                    let clip = ui.clip_rect();
                    let resp = ui.interact(clip, id, Sense::click_and_drag());
                    let text_x = origin.x + gutter_w;
                    let hit = |ui: &Ui, pos: Pos2| -> usize {
                        let line = (((pos.y - origin.y) / line_h).floor().max(0.0) as usize).min(total - 1);
                        let text = visible_text(buf_ref, line);
                        let len = text.chars().count();
                        let galley = ui.painter().layout_job(LayoutJob::simple_singleline(text, font.clone(), theme.text));
                        let column = usize::from(galley.cursor_from_pos(vec2(pos.x - text_x, 0.0)).index);
                        buf_ref.line_start(line) + column.min(len)
                    };

                    let pointer = ui.input(|i| i.pointer.interact_pos());
                    let (pressed, mods) = ui.input(|i| (i.pointer.primary_pressed(), i.modifiers));
                    if let Some(pos) = pointer {
                        if pressed && resp.is_pointer_button_down_on() {
                            resp.request_focus();
                            activated = true;
                            touched = true;
                            let p = hit(ui, pos);
                            if mods.alt {
                                tab.sels.push(Sel::caret(p));
                                merge_sels(&mut tab.sels);
                                tab.drag_anchor = Some(p);
                            } else if mods.shift && let Some(s) = tab.sels.last_mut() {
                                s.head = p;
                                s.goal = None;
                                tab.drag_anchor = Some(s.anchor);
                            } else {
                                tab.sels = vec![Sel::caret(p)];
                                tab.drag_anchor = Some(p);
                            }
                        } else if resp.dragged_by(PointerButton::Primary) {
                            let p = hit(ui, pos);
                            if let Some(anchor) = tab.drag_anchor
                                && let Some(s) = tab.sels.iter_mut().find(|s| s.anchor == anchor)
                            {
                                s.head = p;
                                s.goal = None;
                            }
                            touched = true;
                            if !clip.contains(pos) {
                                ui.scroll_to_rect(Rect::from_center_size(pos, vec2(2.0, 2.0)), None);
                                ui.ctx().request_repaint();
                            }
                        }
                        if resp.double_clicked() {
                            let (a, b) = buf_ref.word_at(hit(ui, pos));
                            tab.sels = vec![Sel::new(a, b)];
                        }
                        if resp.triple_clicked() {
                            let line = buf_ref.line_of(hit(ui, pos));
                            tab.sels = vec![Sel::new(buf_ref.line_start(line), buf_ref.line_start(line + 1).min(buf_ref.len()))];
                        }
                        if resp.hovered() && pos.x > clip.left() + gutter_w {
                            ui.ctx().set_cursor_icon(CursorIcon::Text);
                        }
                    }
                    if resp.drag_stopped() {
                        merge_sels(&mut tab.sels);
                    }

                    let first = ((viewport.min.y / line_h).floor().max(0.0) as usize).min(total - 1);
                    let last = ((viewport.max.y / line_h).ceil() as usize + 1).min(total);
                    if !hl.ensure(&buf_ref.rope, first..last, theme, Duration::from_millis(8)) {
                        ui.ctx().request_repaint();
                    }
                    let painter = ui.painter().clone();
                    let single = tab.sels.len() == 1 && tab.sels[0].is_empty();
                    let current_line = single.then(|| buf_ref.line_of(tab.sels[0].head));
                    let bracket = if single { buf_ref.matching_bracket(tab.sels[0].head) } else { None };

                    for line in first..last {
                        let y = origin.y + line as f32 * line_h;
                        let start = buf_ref.line_start(line);
                        let text = visible_text(buf_ref, line);
                        let len = text.chars().count();
                        if current_line == Some(line) {
                            let row = Rect::from_min_max(pos2(clip.left(), y), pos2(clip.right(), y + line_h));
                            painter.rect_filled(row, CornerRadius::ZERO, ui::mix(theme.bg, theme.hover, 0.4));
                        }
                        let galley = painter.layout_job(job(&text, hl.get(line), &font, theme.text));
                        let x_at = |column: usize| text_x + galley.pos_from_cursor(CCursor::new(column.min(len))).min.x;
                        let from = matches.partition_point(|m| m.1 <= start);
                        for m in matches[from..].iter().take_while(|m| m.0 <= start + len) {
                            let r = Rect::from_min_max(pos2(x_at(m.0.saturating_sub(start)), y), pos2(x_at(m.1 - start), y + line_h));
                            painter.rect_filled(r, CornerRadius::same(2), ui::with_alpha(theme.accent, 0.25));
                        }
                        for s in tab.sels.iter().filter(|s| !s.is_empty() && s.end() >= start && s.start() <= start + len) {
                            let x0 = if s.start() <= start { text_x } else { x_at(s.start() - start) };
                            let x1 = if s.end() > start + len { x_at(len) + char_w * 0.6 } else { x_at(s.end() - start) };
                            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h)), CornerRadius::same(2), theme.selection);
                        }
                        if let Some((a, b)) = bracket {
                            for p in [a, b].into_iter().filter(|p| *p >= start && *p < start + len) {
                                let r = Rect::from_min_max(pos2(x_at(p - start), y + 1.0), pos2(x_at(p - start + 1), y + line_h - 1.0));
                                painter.rect(r, CornerRadius::same(2), ui::with_alpha(theme.accent, 0.12), Stroke::new(1.0, ui::with_alpha(theme.accent, 0.7)), StrokeKind::Inside);
                            }
                        }
                        painter.galley(pos2(text_x, y + (line_h - galley.size().y) / 2.0), galley.clone(), theme.text);
                        for s in tab.sels.iter().filter(|s| s.head >= start && s.head <= start + len) {
                            let x = x_at(s.head - start);
                            let caret = Rect::from_min_max(pos2(x - 0.75, y + 1.0), pos2(x + 0.75, y + line_h - 1.0));
                            painter.rect_filled(caret, CornerRadius::ZERO, ui::with_alpha(theme.accent, caret_alpha));
                        }
                    }

                    let gutter = Rect::from_min_max(clip.min, pos2(clip.left() + gutter_w - 4.0, clip.bottom()));
                    painter.rect_filled(gutter, CornerRadius::ZERO, theme.bg);
                    let number_font = FontId::monospace(settings.font_size - 1.0);
                    for line in first..last {
                        let y = origin.y + line as f32 * line_h + line_h / 2.0;
                        let color = if current_line == Some(line) { theme.text } else { ui::with_alpha(theme.text_dim, 0.75) };
                        painter.text(pos2(gutter.right() - 12.0, y), Align2::RIGHT_CENTER, line + 1, number_font.clone(), color);
                    }
                    if let Some(changes) = path.as_deref().and_then(|p| git.line_changes(p)) {
                        let from = changes.partition_point(|c| c.0 < first);
                        for (line, kind) in changes[from..].iter().take_while(|c| c.0 <= last) {
                            let y = origin.y + *line as f32 * line_h;
                            let x = gutter.right() - 6.0;
                            let (r, color) = match kind {
                                LineChange::Added => (Rect::from_min_max(pos2(x, y), pos2(x + 3.0, y + line_h)), theme.added),
                                LineChange::Modified => (Rect::from_min_max(pos2(x, y), pos2(x + 3.0, y + line_h)), theme.modified),
                                LineChange::Deleted => (Rect::from_min_max(pos2(x, y - 2.0), pos2(x + 6.0, y + 2.0)), theme.deleted),
                            };
                            painter.rect_filled(r, CornerRadius::ZERO, color);
                        }
                    }

                    if let Some(pos) = tab.reveal.take() {
                        let line = buf_ref.line_of(pos);
                        let x = text_x + (pos - buf_ref.line_start(line)) as f32 * char_w;
                        let top = origin.y + line as f32 * line_h;
                        let target = Rect::from_min_max(pos2(x - gutter_w - char_w * 3.0, top - line_h), pos2(x + char_w * 4.0, top + line_h * 2.0));
                        ui.scroll_to_rect(target, None);
                    }
                })
            })
            .inner;
        let scroll_y = out.state.offset.y;
        if activated {
            self.active = g;
        }
        if touched {
            self.last_input = now;
        }

        if minimap_w > 0.0 {
            let mm = Rect::from_min_max(pos2(text_rect.right(), body.top()), body.max);
            let resp = ui.interact(mm, id.with("minimap"), Sense::click_and_drag());
            let p = ui.painter().with_clip_rect(mm);
            p.vline(mm.left(), mm.y_range(), Stroke::new(1.0, ui::with_alpha(theme.border, 0.6)));
            let scale = (mm.height() / total as f32).min(2.0);
            let (count, step) = if scale >= 1.0 { (total, scale) } else { (mm.height() as usize, 1.0) };
            let color = ui::with_alpha(theme.text_dim, 0.4);
            for i in 0..count {
                let line = if scale >= 1.0 { i } else { (i as f32 / scale) as usize };
                if line >= total {
                    break;
                }
                let len = buf.line_len(line).min(90);
                let indent = buf.rope.line(line).chars().take(90).take_while(|c| c.is_whitespace()).count();
                if indent >= len {
                    continue;
                }
                let y = mm.top() + i as f32 * step;
                let bar = Rect::from_min_max(pos2(mm.left() + 6.0 + indent as f32 * 0.75, y), pos2(mm.left() + 6.0 + len as f32 * 0.75, y + (step - 0.5).max(1.0)));
                p.rect_filled(bar, CornerRadius::ZERO, color);
            }
            let visible = text_rect.height() / line_h;
            let top = mm.top() + scroll_y / line_h * scale;
            let slider = Rect::from_min_max(pos2(mm.left() + 1.0, top), pos2(mm.right(), top + (visible * scale).max(12.0)));
            let h = ui::anim(ui, id.with("minimap_hover"), resp.hovered() || resp.dragged());
            p.rect_filled(slider, CornerRadius::ZERO, ui::with_alpha(theme.text, 0.06 + 0.08 * h));
            if (resp.clicked() || resp.dragged())
                && let Some(pos) = resp.interact_pointer_pos()
            {
                tab.set_scroll = Some((((pos.y - mm.top()) / scale - visible / 2.0) * line_h).max(0.0));
            }
        }

        let find_t = ui::anim(ui, Id::new(("editor_find", g)), find_open);
        if find_t > 0.0 && is_active {
            self.find_ui(ui, text_rect, find_t, theme, buf, tab);
        }

        if is_active && let Some(mut text) = self.goto.take() {
            let rect = Rect::from_min_size(pos2(text_rect.center().x - 160.0, body.top() + 8.0), vec2(320.0, 34.0));
            ui.painter().rect(rect, CornerRadius::same(6), theme.raised, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
            let hint = format!("Go to line (1 - {total}), optional :column");
            let resp = ui.put(rect.shrink(6.0), TextEdit::singleline(&mut text).hint_text(hint).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
            if self.goto_focus {
                resp.request_focus();
                self.goto_focus = false;
            }
            if resp.has_focus() {
                self.focused = true;
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    let mut parts = text.split(':').map(|p| p.trim().parse::<usize>().ok());
                    if let Some(Some(line)) = parts.next() {
                        let line = line.clamp(1, total) - 1;
                        let column = parts.next().flatten().unwrap_or(1).max(1) - 1;
                        let pos = buf.line_start(line) + column.min(buf.line_len(line));
                        tab.sels = vec![Sel::caret(pos)];
                        tab.reveal = Some(pos);
                    }
                }
                self.want_focus = true;
            } else {
                self.goto = Some(text);
            }
        }

        if is_active && let Some(message) = self.banner.clone() {
            let rect = Rect::from_min_max(pos2(body.left(), body.bottom() - 30.0), pos2(text_rect.right(), body.bottom()));
            ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(theme.raised, theme.warning, 0.2));
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 3.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                if ui::icon_button(ui, theme, icons::X, 22.0, false, "Dismiss").clicked() {
                    self.banner = None;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&message).color(theme.text)).truncate());
                });
            });
        }
    }

    fn find_ui(&mut self, ui: &mut Ui, area: Rect, t: f32, theme: &Theme, buf: &mut Document, tab: &mut Tab) {
        let rows = if self.find.replace { 2.0 } else { 1.0 };
        let size = vec2(430.0f32.min(area.width() - 30.0), 10.0 + rows * 28.0);
        let rect = Rect::from_min_size(pos2(area.right() - size.x - 22.0, area.top() - (1.0 - t) * size.y), size);
        let corners = CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 };
        ui.painter().with_clip_rect(area).rect(rect, corners, theme.raised, Stroke::new(1.0, theme.border), StrokeKind::Inside);
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let shift = ui.input(|i| i.modifiers.shift);
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        let (mut step, mut close, mut replace_one, mut replace_all) = (0, false, false, false);
        let current = tab.sels.first().and_then(|s| self.find.matches.iter().position(|m| m.0 == s.start() && m.1 == s.end()));
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 5.0))), |ui| {
            ui.set_clip_rect(area);
            ui.set_opacity(t);
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            ui.horizontal(|ui| {
                let caret = if self.find.replace { icons::CARET_DOWN } else { icons::CARET_RIGHT };
                if ui::icon_button(ui, theme, caret, 22.0, false, "Toggle Replace").clicked() {
                    self.find.replace ^= true;
                }
                let field = ui.add(TextEdit::singleline(&mut self.find.query).desired_width(170.0).hint_text("Find"));
                if self.find.focus {
                    field.request_focus();
                    self.find.focus = false;
                }
                if field.has_focus() {
                    self.focused = true;
                }
                if field.lost_focus() {
                    if enter {
                        step = if shift { -1 } else { 1 };
                        field.request_focus();
                    } else if escape {
                        close = true;
                    }
                }
                ui.add_space(4.0);
                let (label, color) = match (self.find.query.is_empty(), self.find.matches.len(), current) {
                    (true, ..) => (String::new(), theme.text_dim),
                    (_, 0, _) => ("No results".to_owned(), theme.error),
                    (_, n, Some(i)) => (format!("{} of {n}", i + 1), theme.text_dim),
                    (_, n, None) => (format!("{n} found"), theme.text_dim),
                };
                ui.label(RichText::new(label).size(12.0).color(color));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui::icon_button(ui, theme, icons::X, 22.0, false, "Close (Escape)").clicked() {
                        close = true;
                    }
                    if ui::icon_button(ui, theme, icons::ARROW_DOWN, 22.0, false, "Next Match (Enter)").clicked() {
                        step = 1;
                    }
                    if ui::icon_button(ui, theme, icons::ARROW_UP, 22.0, false, "Previous Match (Shift+Enter)").clicked() {
                        step = -1;
                    }
                    if ui::icon_button(ui, theme, icons::TEXT_UNDERLINE, 22.0, self.find.word, "Match Whole Word").clicked() {
                        self.find.word ^= true;
                    }
                    if ui::icon_button(ui, theme, icons::TEXT_AA, 22.0, self.find.case, "Match Case").clicked() {
                        self.find.case ^= true;
                    }
                });
            });
            if self.find.replace {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    let field = ui.add(TextEdit::singleline(&mut self.find.with).desired_width(170.0).hint_text("Replace"));
                    if field.has_focus() {
                        self.focused = true;
                    }
                    if field.lost_focus() && enter {
                        replace_one = true;
                        field.request_focus();
                    } else if field.lost_focus() && escape {
                        close = true;
                    }
                    ui.add_space(4.0);
                    if ui::icon_button(ui, theme, icons::SWAP, 22.0, false, "Replace (Enter)").clicked() {
                        replace_one = true;
                    }
                    if ui::icon_button(ui, theme, icons::ARROWS_CLOCKWISE, 22.0, false, "Replace All").clicked() {
                        replace_all = true;
                    }
                });
            }
        });
        if replace_all && !self.find.matches.is_empty() {
            let edits = self.find.matches.iter().map(|m| (m.0, m.1, self.find.with.clone())).collect();
            buf.apply(&mut tab.sels, edits, Kind::Other, After::Carets);
            tab.sels.truncate(1);
        } else if replace_one {
            if let Some(i) = current {
                let m = self.find.matches[i];
                buf.apply(&mut tab.sels, vec![(m.0, m.1, self.find.with.clone())], Kind::Other, After::Carets);
                self.find.matches = buf.find_all(&self.find.query, self.find.case, self.find.word);
                self.find.key.1 = buf.version;
            }
            self.step_match(tab, true);
        } else if step != 0 {
            self.step_match(tab, step > 0);
        }
        if close {
            self.find.open = false;
            self.want_focus = true;
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some((g, t)) = self.closing else { return };
        let Some(doc_id) = self.groups.get(g).and_then(|group| group.tabs.get(t)).map(|tab| tab.doc) else {
            self.closing = None;
            return;
        };
        let title = self.docs.get(&doc_id).map(|d| d.buf.title.clone()).unwrap_or_default();
        let mut choice = None;
        let modal = egui::Modal::new(Id::new("editor_close")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Unsaved changes");
            ui.label(format!("Save changes to {title} before closing?"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (i, label) in ["Save", "Don't Save", "Cancel"].into_iter().enumerate() {
                    if ui.button(label).clicked() {
                        choice = Some(i);
                    }
                }
            });
        });
        if modal.should_close() {
            choice = Some(2);
        }
        match choice {
            Some(0) => {
                self.closing = None;
                if self.save_doc(doc_id, false) {
                    self.close_tab(g, t);
                }
            }
            Some(1) => {
                self.closing = None;
                self.close_tab(g, t);
            }
            Some(_) => self.closing = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<Event>) {
        let theme = Theme::dark();
        let settings = Settings::default();
        let git = Git::new();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let mut cmds = vec![];
            editors.ui(ui, &theme, &settings, &git, &mut cmds);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn types_into_a_view_and_survives_split_and_close() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        assert!(editors.has_focus());
        frame(&ctx, &mut editors, vec![Event::Text("fn main() {".into())]);
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, &mut editors, vec![enter, Event::Text("x".into())]);
        let text = editors.docs.values().next().unwrap().buf.rope.to_string();
        assert_eq!(text, "fn main() {\n    x");
        let status = editors.status().unwrap();
        assert_eq!((status.line, status.column), (2, 6));
        assert!(editors.any_dirty());

        editors.exec(&Command::SplitEditor, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.groups.len(), 2);
        editors.exec(&Command::Undo, &settings, &ctx);
        editors.exec(&Command::Undo, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        editors.exec(&Command::CloseEditor, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.groups.len(), 1);
        assert_eq!(editors.docs.len(), 1);
    }

    #[test]
    fn opens_read_only_diff_tabs_and_reuses_them() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.open_text("a.txt (changes)", "@@ -1 +1 @@\r\n-one\r\n+two\r\n");
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        editors.open_text("a.txt (changes)", "");
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        assert_eq!(editors.docs.len(), 1);
        assert_eq!(editors.groups[0].tabs.len(), 1);
        assert_eq!(editors.docs.values().next().unwrap().buf.rope.to_string(), "No differences.\n");
        assert_eq!(editors.status().unwrap().language, "Diff");
        assert!(!editors.any_dirty());
    }

    #[test]
    fn reloads_clean_documents_changed_on_disk() {
        let path = std::env::temp_dir().join(format!("halo-reload-test-{}.txt", std::process::id()));
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.open(&path);
        let end = Event::Key { key: Key::End, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![end]);
        std::fs::write(&path, "1\n").unwrap();
        editors.reload_clean();
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
        assert_eq!(text(&editors), "1\nx");

        std::fs::write(&path, "other\n").unwrap();
        editors.reload_clean();
        assert_eq!(text(&editors), "1\nx");
        let _ = std::fs::remove_file(&path);
    }
}
