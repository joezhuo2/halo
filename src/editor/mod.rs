mod buffer;
mod emmet;
mod highlight;
mod intel;
mod snippet;

use crate::command::{self, Command, Scope};
use crate::git::{Git, LineChange};
use crate::lsp::{self, Lsp};
use crate::settings::{Backup, Settings};
use crate::theme::Theme;
use crate::ui::{self, icons};
use buffer::{After, Document, Kind, Motion, Sel, Take, hidden, is_word, merge_sels};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Event, EventFilter, FontId, Id, Key, Layout, PointerButton,
    Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, ViewportCommand, pos2,
    scroll_area::ScrollBarVisibility, vec2,
};
use highlight::{Highlighter, Spans};
use intel::{Intel, Lang, Piece, Queued, Row, from_lsp};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn configure(settings: &Settings) {
    highlight::configure(&settings.grammars, &settings.associations);
}

pub use buffer::{Encoding, Pattern};
pub use snippet::{path as snippets_path, reload as reload_snippets, write_sample as write_sample_snippets};
pub use highlight::languages;

const TAB_H: f32 = 34.0;
const MAX_COLS: usize = 5000;

pub struct EditorStatus {
    pub line: usize,
    pub column: usize,
    pub selected: usize,
    pub cursors: usize,
    pub language: String,
    pub encoding: &'static str,
    pub crlf: bool,
    pub indent: Option<(bool, usize)>,
}

fn indent_unit(buf: &Document, settings: &Settings) -> (bool, usize, String) {
    let (spaces, size) = match buf.indent {
        Some((spaces, size)) => (spaces, if size == 0 { settings.tab_size } else { size }),
        None => (settings.insert_spaces, settings.tab_size),
    };
    (spaces, size, if spaces { " ".repeat(size) } else { "\t".to_owned() })
}

struct Doc {
    buf: Document,
    hl: Highlighter,
    max_cols: (u64, usize),
    conflicts: (u64, Vec<[usize; 4]>),
    lang: Lang,
    folds: Vec<(usize, usize)>,
    depths: (u64, Vec<u32>),
    backed: u64,
    stale: bool,
}

#[derive(Default)]
struct Tab {
    doc: u64,
    sels: Vec<Sel>,
    reveal: Option<usize>,
    set_scroll: Option<f32>,
    drag_anchor: Option<usize>,
    pinned: bool,
    rows: (u64, Vec<u32>),
    preview: bool,
    pair: ((u64, usize), Option<(usize, usize)>),
    stuck: usize,
    box_anchor: Option<usize>,
    snippet: Option<snippet::Session>,
    grown: Vec<(Vec<Sel>, Vec<Sel>)>,
    trail: Vec<Vec<Sel>>,
    seen: (u64, Vec<Sel>),
}

#[derive(Default)]
struct Group {
    tabs: Vec<Tab>,
    active: usize,
    recent: Vec<u64>,
}

#[derive(Default)]
struct Find {
    open: bool,
    replace: bool,
    query: String,
    with: String,
    case: bool,
    word: bool,
    regex: bool,
    preserve: bool,
    scope: Option<(usize, usize)>,
    error: Option<String>,
    focus: bool,
    matches: Vec<(usize, usize)>,
    key: (u64, u64, String, [bool; 3], Option<(usize, usize)>),
}

enum TabAction {
    Activate(usize),
    Close(usize),
    Keep(usize),
    CloseOthers(usize),
    CloseAll,
    CopyPath(usize),
    Pin(usize),
}

#[derive(Clone, Copy)]
enum DropAt {
    Tab(usize, usize),
    Split(usize, bool),
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
    banner: Option<(String, Option<Instant>)>,
    last_input: f64,
    drag: Option<(usize, usize)>,
    drop: Option<DropAt>,
    pub ai_open: bool,
    pub lsp: Lsp,
    pub marks: crate::debug::Marks,
    root: Option<PathBuf>,
    intel: Intel,
    closed: Vec<PathBuf>,
    switcher: Option<(usize, usize)>,
    tidy: (bool, bool),
    disk_jobs: Vec<(u64, bool)>,
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

fn block_tokens(path: Option<&Path>) -> Option<(&'static str, &'static str)> {
    let ext = path.and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match (ext.as_str(), comment_tokens(path)) {
        ("py", _) => Some(("\"\"\"", "\"\"\"")),
        ("lua", _) => Some(("--[[", "]]")),
        ("hs", _) => Some(("{-", "-}")),
        ("ps1", _) => Some(("<#", "#>")),
        ("sql", _) => Some(("/*", "*/")),
        (_, (open, Some(close))) => Some((open, close)),
        (_, ("//", None)) => Some(("/*", "*/")),
        _ => None,
    }
}

fn search_target(buf: &Document, line: usize) -> Option<(String, usize)> {
    let text = buf.line_text(line);
    let number = text.strip_prefix("  ").filter(|_| buf.read_only && buf.title.starts_with("Search: "))?.split(':').next()?.parse().ok()?;
    let file = (0..line).rev().map(|above| buf.line_text(above)).find(|above| !above.is_empty() && !above.starts_with(' '))?;
    Some((file, number))
}

fn emmet_expand(buf: &mut Document, tab: &mut Tab, unit: &str, strict: bool) -> bool {
    let [s] = tab.sels[..] else { return false };
    let ext = buf.path.as_deref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    // shortcut: Tab expands only in markup files and only known tags, so code is not hijacked, the command expands anywhere
    if !s.is_empty() || (strict && !matches!(ext.as_str(), "html" | "htm" | "vue" | "svelte" | "php")) {
        return false;
    }
    let before = buf.text_of(buf.line_start(buf.line_of(s.head)), s.head);
    let abbr = emmet::abbreviation(&before);
    let Some(body) = emmet::expand(abbr, matches!(ext.as_str(), "jsx" | "tsx"), strict) else { return false };
    tab.snippet = snippet::insert(buf, &mut tab.sels, s.head - abbr.chars().count(), s.head, &body, unit);
    true
}

fn links(text: &str) -> Vec<(usize, usize)> {
    let (mut out, mut from) = (vec![], 0);
    while let Some(at) = text[from..].find("http").map(|found| from + found) {
        let rest = &text[at..];
        let scheme = if rest.starts_with("https://") { 8 } else { 7 };
        let end = rest.find(|c: char| c.is_whitespace() || "\"'<>`()[]{}|\\^".contains(c)).unwrap_or(rest.len());
        let len = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']).len();
        if len > scheme && (scheme == 8 || rest.starts_with("http://")) {
            out.push((text[..at].chars().count(), text[..at + len].chars().count()));
        }
        from = at + len.max(4);
    }
    out
}

fn notice(text: String) -> Option<(String, Option<Instant>)> {
    Some((text, Some(Instant::now())))
}

fn visible_text(buf: &Document, line: usize) -> String {
    let text = buf.line_text(line);
    if text.len() <= MAX_COLS {
        return text;
    }
    text.chars().take(MAX_COLS).collect()
}

fn build_rows(buf: &Document, wrap: Option<usize>, folds: &[(usize, usize)], lens: &[(usize, usize)]) -> Vec<u32> {
    let mut starts = Vec::with_capacity(buf.lines() + 1);
    let (mut at, mut f, mut l, mut hide_to) = (0u32, 0, 0, 0);
    for (line, slice) in buf.rope.lines().enumerate() {
        starts.push(at);
        while f < folds.len() && folds[f].0 < line {
            hide_to = hide_to.max(folds[f].1);
            f += 1;
        }
        while l < lens.len() && lens[l].0 < line {
            l += 1;
        }
        if line > 0 && line <= hide_to {
            continue;
        }
        at += (l < lens.len() && lens[l].0 == line) as u32 + 1;
        if let Some(cols) = wrap.filter(|cols| slice.len_bytes() * 4 > *cols) {
            buffer::wrap(slice.chars().filter(|c| *c != '\n').take(MAX_COLS), cols, |_| at += 1);
        }
    }
    starts.push(at);
    starts
}

#[allow(clippy::too_many_arguments)]
fn job(
    text: &str,
    range: std::ops::Range<usize>,
    spans: Option<&Spans>,
    font: &FontId,
    color: Color32,
    over: &[(usize, usize, Color32)],
    inserts: &[(usize, &str)],
    hint: &TextFormat,
) -> LayoutJob {
    let spans = spans.filter(|s| !s.is_empty());
    if spans.is_none() && over.is_empty() && inserts.is_empty() {
        return LayoutJob::simple_singleline(text[range].to_owned(), font.clone(), color);
    }
    let mut cuts = vec![range.start, range.end];
    let mut base: Vec<(usize, Color32)> = vec![];
    let mut at = 0;
    for (len, c) in spans.into_iter().flat_map(|s| s.iter()) {
        at = (at + len).min(text.len());
        base.push((at, *c));
        cuts.push(at);
    }
    cuts.extend(over.iter().flat_map(|o| [o.0, o.1]));
    cuts.extend(inserts.iter().map(|i| i.0));
    cuts.retain(|cut| range.contains(cut) || *cut == range.end);
    cuts.retain(|cut| text.is_char_boundary(*cut));
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = LayoutJob::default();
    let (mut b, mut o, mut i) = (0, 0, 0);
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while i < inserts.len() && inserts[i].0 <= from {
            job.append(inserts[i].1, 0.0, hint.clone());
            i += 1;
        }
        while b < base.len() && base[b].0 <= from {
            b += 1;
        }
        while o < over.len() && over[o].1 <= from {
            o += 1;
        }
        let shade = match over.get(o) {
            Some(found) if found.0 <= from => found.2,
            _ => base.get(b).map_or(color, |span| span.1),
        };
        job.append(&text[from..to], 0.0, TextFormat::simple(font.clone(), shade));
    }
    for insert in &inserts[i..] {
        job.append(insert.1, 0.0, hint.clone());
    }
    job
}

fn evict(buf: &Document, folds: &[(usize, usize)], sels: &mut Vec<Sel>) {
    for s in sels.iter_mut() {
        if let Some(fold) = hidden(folds, buf.line_of(s.head.min(buf.len()))) {
            *s = Sel::caret(buf.line_end(folds[fold].0));
        }
    }
    merge_sels(sels);
}

fn fold_command(doc: &mut Doc, tab: &mut Tab, cmd: &Command) {
    let (buf, folds) = (&doc.buf, &mut doc.folds);
    buf.clamp(&mut tab.sels);
    let line = buf.line_of(tab.sels[0].head);
    match cmd {
        Command::Fold => {
            for start in (0..=line).rev() {
                let found = buf.fold_end(start).filter(|end| *end >= line && !folds.contains(&(start, *end)));
                if let Some(end) = found {
                    folds.push((start, end));
                    break;
                }
                if buf.indent_of(start) == 0 && buf.line_len(start) > 0 {
                    break;
                }
            }
        }
        Command::Unfold => folds.retain(|f| !(f.0 <= line && line <= f.1)),
        Command::FoldAll => *folds = (0..buf.lines()).filter_map(|start| Some((start, buf.fold_end(start)?))).collect(),
        _ => folds.clear(),
    }
    folds.sort_unstable();
    evict(buf, folds, &mut tab.sels);
    tab.reveal = tab.sels.first().map(|s| s.head);
}

fn settle(buf: &mut Document, hl: &mut Highlighter, lang: &mut Lang, lsp: &mut Lsp, folds: &mut Vec<(usize, usize)>) -> Option<(usize, isize)> {
    let line = buf.dirty_from.take()?;
    hl.invalidate(line);
    let delta = lang.edited(line, buf.lines());
    folds.retain_mut(|f| {
        let touched = f.0 <= line && line <= f.1;
        if f.0 > line {
            *f = ((f.0 as isize + delta).max(0) as usize, (f.1 as isize + delta).max(0) as usize);
        }
        !touched && buf.fold_end(f.0) == Some(f.1)
    });
    if let Some(path) = &buf.path {
        lsp.shift_diagnostics(path, line, delta);
    }
    Some((line, delta))
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    buf: &mut Document,
    tab: &mut Tab,
    event: &Event,
    settings: &Settings,
    page: usize,
    ctx: &egui::Context,
    wrap: Option<usize>,
    folds: &[(usize, usize)],
) -> bool {
    let (spaces, size, unit) = indent_unit(buf, settings);
    let sels = &mut tab.sels;
    match event {
        Event::Text(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if settings.auto_close => buf.type_char(sels, c),
                _ => buf.insert_text(sels, text),
            };
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
                match motion {
                    Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) if wrap.is_some() || !folds.is_empty() => {
                        let count = if matches!(motion, Motion::Up | Motion::Down) { 1 } else { page };
                        buf.move_rows(sels, matches!(motion, Motion::Down | Motion::PageDown(_)), count, extend, wrap, folds);
                    }
                    _ => buf.move_sels(sels, motion, extend),
                }
                return true;
            }
            match key {
                Key::Enter => buf.newline(sels, &unit),
                Key::Tab if modifiers.shift => buf.outdent(sels, size),
                Key::Tab => buf.tab(sels, size, spaces),
                Key::Backspace => buf.backspace(sels, word, settings.auto_close),
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
            drag: None,
            drop: None,
            ai_open: false,
            lsp: Lsp::default(),
            marks: Default::default(),
            root: None,
            intel: Intel::default(),
            closed: vec![],
            switcher: None,
            tidy: (false, false),
            disk_jobs: vec![],
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
            self.banner = Some((note.clone(), None));
        }
        let hl = Highlighter::new(buf.path.as_deref(), &buf.rope);
        self.docs.insert(id, Doc { buf, hl, max_cols: (0, 0), conflicts: (0, vec![]), lang: Lang::default(), folds: vec![], depths: (0, vec![]), backed: 0, stale: false });
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
        if self.active_path().as_deref() != Some(path) {
            self.remember();
        }
        self.open_as(path, true);
    }

    pub fn open_kept(&mut self, path: &Path) {
        self.open_as(path, false);
    }

    fn open_as(&mut self, path: &Path, preview: bool) {
        self.active = self.active.min(self.groups.len() - 1);
        if let Some(id) = self.find_doc(path) {
            return self.show_doc(id);
        }
        match Document::load(path) {
            Ok(buf) => {
                let id = self.insert_doc(buf);
                self.place(id, preview);
            }
            Err(error) => self.banner = Some((error, None)),
        }
    }

    fn place(&mut self, id: u64, preview: bool) {
        let group = &mut self.groups[self.active];
        let clean = |tab: &Tab| tab.preview && !self.docs.get(&tab.doc).is_some_and(|d| d.buf.is_dirty());
        let Some(at) = group.tabs.iter().position(clean).filter(|_| preview) else {
            self.add_tab(id, vec![Sel::caret(0)]);
            self.groups[self.active].tabs.last_mut().into_iter().for_each(|tab| tab.preview = preview);
            return;
        };
        let fresh = Tab { doc: id, sels: vec![Sel::caret(0)], preview: true, ..Default::default() };
        let old = std::mem::replace(&mut group.tabs[at], fresh).doc;
        group.active = at;
        self.want_focus = true;
        self.drop_doc(old);
    }

    fn drop_doc(&mut self, doc: u64) {
        if self.references(doc) == 0
            && let Some(path) = self.docs.remove(&doc).and_then(|d| d.buf.path)
        {
            self.lsp.close(&path);
            self.closed.retain(|p| *p != path);
            self.closed.push(path);
            let extra = self.closed.len().saturating_sub(30);
            self.closed.drain(..extra);
        }
    }

    fn recent_order(&self, g: usize) -> Vec<u64> {
        let Some(group) = self.groups.get(g) else { return vec![] };
        let open = |doc: &u64| group.tabs.iter().any(|tab| tab.doc == *doc);
        let mut order: Vec<u64> = group.recent.iter().copied().filter(open).collect();
        let rest: Vec<u64> = group.tabs.iter().map(|tab| tab.doc).filter(|doc| !order.contains(doc)).collect();
        order.extend(rest);
        order
    }

    fn show_doc(&mut self, id: u64) {
        let group = &mut self.groups[self.active];
        match group.tabs.iter().position(|t| t.doc == id) {
            Some(t) => {
                group.active = t;
                self.want_focus = true;
            }
            None => self.place(id, true),
        }
    }

    pub fn replace_in(&mut self, path: &Path, pattern: &Pattern, with: &str, preserve: bool) -> Result<usize, String> {
        let replace = |buf: &mut Document, sels: &mut Vec<Sel>| {
            let found = buf.find_all(pattern, None);
            let edits = found.iter().map(|m| (m.0, m.1, pattern.replacement(&buf.text_of(m.0, m.1), with, preserve))).collect();
            buf.apply(sels, edits, Kind::Other, After::Map);
            found.len()
        };
        let Some(id) = self.find_doc(path) else {
            let mut buf = Document::load(path)?;
            let count = replace(&mut buf, &mut vec![Sel::caret(0)]);
            return if count > 0 { buf.save().map(|_| count) } else { Ok(0) };
        };
        let Some(doc) = self.docs.get_mut(&id) else { return Ok(0) };
        let clean = !doc.buf.is_dirty();
        let count = match self.groups.iter_mut().flat_map(|group| &mut group.tabs).find(|tab| tab.doc == id) {
            Some(tab) => replace(&mut doc.buf, &mut tab.sels),
            None => replace(&mut doc.buf, &mut vec![Sel::caret(0)]),
        };
        if clean && count > 0 {
            self.save_doc(id, false);
        }
        Ok(count)
    }

    pub fn open_text(&mut self, title: &str, text: &str, syntax: Option<&Path>) {
        self.active = self.active.min(self.groups.len() - 1);
        let text = if text.is_empty() { "No differences.\n".to_owned() } else { text.replace("\r\n", "\n") };
        let mut buf = Document::from_text(&text, title.to_owned());
        buf.read_only = true;
        let existing = self.docs.iter().find(|(_, d)| d.buf.read_only && d.buf.title == title).map(|(id, _)| *id);
        let id = existing.unwrap_or_else(|| self.insert_doc(Document::from_text("", String::new())));
        if let Some(doc) = self.docs.get_mut(&id) {
            doc.hl = Highlighter::new(syntax, &buf.rope);
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
        self.drop_doc(doc);
        self.want_focus = true;
    }

    fn place_tab(&mut self, (sg, st): (usize, usize), tg: usize, at: usize) {
        let tab = self.groups[sg].tabs.remove(st);
        let source = &mut self.groups[sg];
        if source.active >= st && source.active > 0 {
            source.active -= 1;
        }
        let at = if sg == tg && at > st { at - 1 } else { at };
        let group = &mut self.groups[tg];
        group.tabs.retain(|t| t.doc != tab.doc);
        let pinned = group.tabs.iter().filter(|t| t.pinned).count();
        let at = if tab.pinned { at.min(pinned) } else { at.clamp(pinned, group.tabs.len()) };
        group.tabs.insert(at, tab);
        group.active = at;
        self.active = tg;
        self.want_focus = true;
    }

    fn drop_tab(&mut self, (sg, st): (usize, usize), at: DropAt) {
        if self.groups.get(sg).is_none_or(|group| st >= group.tabs.len()) {
            return;
        }
        match at {
            DropAt::Tab(g, index) => self.place_tab((sg, st), g, index),
            DropAt::Split(g, right) => {
                let at = g + right as usize;
                self.groups.insert(at, Group::default());
                self.place_tab((sg + (at <= sg) as usize, st), at, 0);
            }
        }
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
            if let Some(old) = doc.buf.path.replace(path) {
                self.lsp.close(&old);
            }
            doc.lang = Lang::default();
        }
        let moved = |path: &PathBuf| buffer::disk_time(path).is_some_and(|now| doc.buf.disk.is_some_and(|then| then != now));
        if !ask && doc.buf.path.as_ref().is_some_and(moved) {
            doc.stale = true;
            return false;
        }
        doc.stale = false;
        let (trim, newline) = self.tidy;
        let mut none = vec![];
        let sels = self.groups.iter_mut().flat_map(|group| &mut group.tabs).find(|tab| tab.doc == id).map_or(&mut none, |tab| &mut tab.sels);
        doc.buf.tidy(sels, trim, newline);
        match doc.buf.save() {
            Ok(()) => {
                self.lsp.save(doc.buf.path.as_deref().unwrap_or(Path::new("")));
                self.groups.iter_mut().flat_map(|group| &mut group.tabs).filter(|tab| tab.doc == id).for_each(|tab| tab.preview = false);
                true
            }
            Err(error) => {
                self.banner = Some((error, None));
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
            ReopenClosed => {
                while let Some(path) = self.closed.pop() {
                    if path.is_file() && self.find_doc(&path).is_none() {
                        self.open_kept(&path);
                        break;
                    }
                }
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
        if self.intel_exec(cmd, settings, ctx) {
            self.last_input = ctx.input(|i| i.time);
            self.want_focus |= *cmd != RenameSymbol;
            return true;
        }
        let Some((g, t)) = self.active_tab() else { return false };
        let doc_id = self.groups[g].tabs[t].doc;
        let mut refocus = true;
        match cmd {
            Save => {
                self.groups[g].tabs[t].preview = false;
                if !self.save_formatted(doc_id, settings, ctx) {
                    self.save_doc(doc_id, false);
                }
            }
            SaveAs => {
                self.save_doc(doc_id, true);
            }
            CloseEditor if self.groups[g].tabs[t].pinned => {}
            CloseEditor => self.request_close(g, t),
            SplitEditor => {
                let sels = self.groups[g].tabs[t].sels.clone();
                self.groups.insert(g + 1, Group::default());
                self.active = g + 1;
                self.add_tab(doc_id, sels);
            }
            NextEditor | PrevEditor => {
                let count = self.recent_order(g).len().max(1);
                let at = self.switcher.filter(|open| open.0 == g).map_or(0, |open| open.1);
                self.switcher = Some((g, if *cmd == NextEditor { (at + 1) % count } else { (at + count - 1) % count }));
                refocus = false;
            }
            KeepEditor => self.groups[g].tabs[t].preview = false,
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
            Fold | Unfold | FoldAll | UnfoldAll => {
                if let Some(doc) = self.docs.get_mut(&doc_id) {
                    fold_command(doc, &mut self.groups[g].tabs[t], cmd);
                }
            }
            ToggleBreakpoint => {
                if let Some((Some(path), line)) = self.with_active(|buf, tab| (buf.path.clone(), buf.line_of(tab.sels[0].head))) {
                    self.marks.toggle(&path, line);
                }
            }
            EditBreakpoint => {
                if let Some((Some(path), line)) = self.with_active(|buf, tab| (buf.path.clone(), buf.line_of(tab.sels[0].head))) {
                    self.marks.apply(&path, crate::debug::Gutter::Ask(line, crate::debug::Ask::Condition));
                }
            }
            GoTo(line, column) => {
                if self.groups[g].tabs[t].sels.first().is_some_and(|s| s.head > 0) {
                    self.remember();
                }
                let (line, column) = (*line, *column);
                self.with_active(|buf, tab| {
                    let line = line.clamp(1, buf.lines()) - 1;
                    let pos = buf.line_start(line) + (column.max(1) - 1).min(buf.line_len(line));
                    tab.sels = vec![Sel::caret(pos)];
                });
            }
            Undo | Redo | Cut | Copy | ToggleComment | SelectAll | AddCursorAbove | AddCursorBelow | SelectNextOccurrence | MoveLineUp
            | EmmetExpand | SelectAllOccurrences | ExpandSelection | ShrinkSelection | CursorUndo | ToggleBlockComment | MoveLineDown | CopyLineUp | CopyLineDown | DeleteLine | JoinLines | InsertLineAbove | InsertLineBelow => {
                let cmd = cmd.clone();
                let ctx = ctx.clone();
                self.with_active(|buf, tab| {
                    if cmd == EmmetExpand {
                        emmet_expand(buf, tab, &indent_unit(buf, settings).2, false);
                    }
                    let sels = &mut tab.sels;
                    match cmd {
                        EmmetExpand => {}
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
                        SelectAllOccurrences => buf.select_all_occurrences(sels),
                        ExpandSelection => {
                            if tab.grown.last().is_some_and(|step| !buffer::same(&step.1, sels)) {
                                tab.grown.clear();
                            }
                            let mut wider: Vec<Sel> = sels.iter().map(|s| buf.grow(*s)).collect();
                            merge_sels(&mut wider);
                            tab.grown.push((std::mem::replace(sels, wider.clone()), wider));
                        }
                        ShrinkSelection => match tab.grown.pop() {
                            Some((before, after)) if buffer::same(&after, sels) => *sels = before,
                            _ => tab.grown.clear(),
                        },
                        CursorUndo => {
                            if let Some(before) = tab.trail.pop() {
                                tab.seen.1 = before.clone();
                                *sels = before;
                            }
                        }
                        ToggleBlockComment => {
                            let line = comment_tokens(buf.path.as_deref());
                            match block_tokens(buf.path.as_deref()) {
                                Some((open, close)) => buf.toggle_block_comment(sels, open, close),
                                None => buf.toggle_comment(sels, line.0, line.1),
                            };
                        }
                        MoveLineUp | MoveLineDown => drop(buf.move_lines(sels, cmd == MoveLineDown)),
                        CopyLineUp | CopyLineDown => drop(buf.copy_lines(sels, cmd == CopyLineDown)),
                        DeleteLine => drop(buf.delete_lines(sels)),
                        JoinLines => drop(buf.join_lines(sels)),
                        InsertLineAbove | InsertLineBelow => drop(buf.insert_line(sels, cmd == InsertLineAbove)),
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
            encoding: doc.buf.encoding.label(),
            crlf: doc.buf.crlf,
            indent: doc.buf.indent,
        })
    }

    fn active_doc(&mut self) -> Option<(&mut Doc, &mut Tab)> {
        let (g, t) = self.active_tab()?;
        let tab = &mut self.groups[g].tabs[t];
        Some((self.docs.get_mut(&tab.doc).filter(|doc| !doc.buf.read_only)?, tab))
    }

    pub fn set_eol(&mut self, crlf: bool) {
        if let Some((doc, _)) = self.active_doc().filter(|(doc, _)| doc.buf.crlf != crlf) {
            doc.buf.crlf = crlf;
            doc.buf.mark_dirty();
        }
    }

    pub fn set_language(&mut self, name: &str) {
        if let Some((doc, _)) = self.active_doc() {
            doc.hl = Highlighter::named(name, &doc.buf.rope);
        }
    }

    pub fn set_indent(&mut self, indent: Option<(bool, usize)>) {
        if let Some((doc, _)) = self.active_doc() {
            doc.buf.indent = indent.or_else(|| doc.buf.detect_indent());
        }
    }

    pub fn convert_indent(&mut self, spaces: bool, settings: &Settings) {
        if let Some((doc, tab)) = self.active_doc() {
            let size = indent_unit(&doc.buf, settings).1;
            doc.buf.convert_indent(&mut tab.sels, spaces, size);
        }
    }

    pub fn set_encoding(&mut self, encoding: Encoding, reopen: bool) {
        let Some((doc, tab)) = self.active_doc() else { return };
        let banner = match (reopen, doc.buf.path.clone()) {
            (false, _) if doc.buf.encoding == encoding => None,
            (false, _) => {
                doc.buf.encoding = encoding;
                doc.buf.mark_dirty();
                None
            }
            (true, None) => None,
            (true, Some(_)) if doc.buf.is_dirty() => Some("Save the file before reopening it with another encoding.".to_owned()),
            (true, Some(path)) => match Document::load_as(&path, Some(encoding)) {
                Ok(buf) => {
                    doc.hl = Highlighter::new(Some(&path), &buf.rope);
                    doc.max_cols = (0, 0);
                    doc.folds.clear();
                    tab.sels = vec![Sel::caret(0)];
                    doc.buf = buf;
                    doc.buf.note.clone()
                }
                Err(error) => Some(error),
            },
        };
        self.banner = banner.map(|text| (text, None)).or(self.banner.take());
    }

    pub fn has_focus(&self) -> bool {
        self.focused
    }

    pub fn focus(&mut self) {
        self.want_focus = true;
    }

    pub fn symbols(&self) -> Vec<(usize, String)> {
        let Some(doc) = self.active_tab().and_then(|(g, t)| self.docs.get(&self.groups[g].tabs[t].doc)) else { return vec![] };
        if doc.lang.symbols.is_empty() {
            return highlight::symbols(doc.buf.path.as_deref(), &doc.buf.rope);
        }
        doc.lang.symbols.iter().map(|s| (s.line, format!("{}{}", "  ".repeat(s.depth), s.name))).collect()
    }

    pub fn auto_save_due(&mut self, now: f64) -> bool {
        let due = now - self.last_input > 1.0 && self.docs.values().any(|d| d.buf.is_dirty() && d.buf.path.is_some());
        if due {
            self.last_input = now;
        }
        due
    }

    pub fn backup(&mut self, dir: &Path) -> Vec<Backup> {
        let _ = std::fs::create_dir_all(dir);
        let mut ids: Vec<u64> = self.groups.iter().flat_map(|group| &group.tabs).map(|tab| tab.doc).collect();
        ids.dedup();
        let mut out: Vec<Backup> = vec![];
        for id in ids {
            let Some(doc) = self.docs.get_mut(&id).filter(|doc| doc.buf.is_dirty() && !doc.buf.read_only) else { continue };
            let file = dir.join(format!("{id}.bak"));
            if out.iter().any(|kept| kept.file == file) || (doc.backed != doc.buf.version && std::fs::write(&file, doc.buf.rope.to_string()).is_err()) {
                continue;
            }
            doc.backed = doc.buf.version;
            out.push(Backup { path: doc.buf.path.clone(), title: doc.buf.title.clone(), file });
        }
        for stale in std::fs::read_dir(dir).into_iter().flatten().flatten().map(|entry| entry.path()) {
            if !out.iter().any(|kept| kept.file == stale) {
                let _ = std::fs::remove_file(stale);
            }
        }
        out
    }

    pub fn restore(&mut self, backups: &[Backup]) {
        let texts: Vec<(&Backup, String)> = backups.iter().filter_map(|backup| Some((backup, std::fs::read_to_string(&backup.file).ok()?))).collect();
        for (backup, text) in texts {
            let on_disk = backup.path.as_deref().filter(|path| path.is_file());
            let id = on_disk.and_then(|path| {
                self.open_kept(path);
                self.find_doc(path)
            });
            match id.and_then(|id| self.docs.get_mut(&id)) {
                Some(doc) if doc.buf.rope == text.as_str() => {}
                Some(doc) => drop(doc.buf.apply(&mut vec![Sel::caret(0)], vec![(0, doc.buf.len(), text)], Kind::Other, After::Carets)),
                None => {
                    let mut buf = Document::from_text(&text, backup.title.clone());
                    buf.path = backup.path.clone();
                    buf.mark_dirty();
                    let number = backup.title.strip_prefix("Untitled-").and_then(|n| n.parse().ok()).unwrap_or(0);
                    self.untitled = self.untitled.max(number);
                    let id = self.insert_doc(buf);
                    self.add_tab(id, vec![Sel::caret(0)]);
                }
            }
        }
    }

    // shortcut: the comparison is git's unified diff of the file and the buffer, so it needs git on PATH
    fn disk_diff(&mut self, id: u64) {
        let Some(doc) = self.docs.get(&id) else { return };
        let Some(path) = doc.buf.path.clone() else { return };
        let dir = std::env::temp_dir();
        let mine = dir.join(format!("halo-{}-{}", std::process::id(), doc.buf.title));
        let text = doc.buf.rope.to_string();
        let title = format!("{} (disk and editor)", doc.buf.title);
        if std::fs::write(&mine, if doc.buf.crlf { text.replace('\n', "\r\n") } else { text }).is_err() {
            return;
        }
        let (theirs, ours) = (path.to_string_lossy(), mine.to_string_lossy());
        let out = match crate::git::exec(&dir, &["diff", "--no-index", "--no-color", "--", theirs.as_ref(), ours.as_ref()]) {
            Ok(out) | Err(out) => out,
        };
        let _ = std::fs::remove_file(&mine);
        self.open_text(&title, &out, Some(Path::new("changes.diff")));
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
            match Document::load(&path) {
                Ok(buf) if buf.rope != doc.buf.rope => {
                    doc.hl = Highlighter::new(Some(&path), &buf.rope);
                    doc.max_cols = (0, 0);
                    doc.buf = buf;
                }
                Ok(buf) => doc.buf.disk = buf.disk,
                Err(_) => {}
            }
        }
        for doc in self.docs.values_mut().filter(|d| d.buf.is_dirty() && d.buf.disk.is_some()) {
            let now = doc.buf.path.as_deref().and_then(buffer::disk_time);
            doc.stale |= now.is_some() && now != doc.buf.disk;
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
            self.lsp.close(&path);
            doc.lang = Lang::default();
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

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, settings: &Settings, git: &Git, cmds: &mut Vec<Command>) {
        self.focused = false;
        self.drop = None;
        self.active = self.active.min(self.groups.len().saturating_sub(1));
        self.tidy = (settings.trim_trailing_whitespace, settings.insert_final_newline);
        self.process(settings, ui.ctx());
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
        let mut outer = false;
        for cmd in pending {
            outer |= matches!(cmd, Command::ToggleAi | Command::OpenPath(_));
            if outer {
                cmds.push(cmd);
            } else {
                self.exec(&cmd, settings, ui.ctx());
            }
        }
        self.after_views();
        for (id, compare) in std::mem::take(&mut self.disk_jobs) {
            match compare {
                true => self.disk_diff(id),
                false => drop(self.save_doc(id, false)),
            }
        }
        if let Some((g, at)) = self.switcher {
            let order = self.recent_order(g);
            if order.is_empty() || !ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
                let doc = order.get(at.min(order.len().saturating_sub(1)));
                if let Some(t) = doc.and_then(|doc| self.groups[g].tabs.iter().position(|tab| tab.doc == *doc)) {
                    self.groups[g].active = t;
                    self.active = g;
                }
                self.switcher = None;
                self.want_focus = true;
            } else {
                let top = pos2(rect.left() + (g as f32 + 0.5) * width, rect.top() + 70.0);
                egui::Area::new(Id::new("editor_switcher")).order(egui::Order::Foreground).pivot(Align2::CENTER_TOP).fixed_pos(top).show(ui.ctx(), |ui| {
                    egui::Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.accent)).corner_radius(CornerRadius::same(6)).inner_margin(4.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for (index, doc) in order.iter().enumerate() {
                            let (r, row) = ui.allocate_exact_size(vec2(320.0, 24.0), Sense::hover());
                            ui::row_bg(ui, theme, r, row.id, false, index == at);
                            let title = self.docs.get(doc).map(|d| d.buf.title.as_str()).unwrap_or_default();
                            ui.painter().with_clip_rect(r).text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme.text);
                        }
                    });
                });
            }
        }
        if let Some((g, t)) = self.drag {
            let ctx = ui.ctx();
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            let doc = self.groups.get(g).and_then(|group| group.tabs.get(t)).and_then(|tab| self.docs.get(&tab.doc));
            if let (Some(pos), Some(doc)) = (ctx.pointer_latest_pos(), doc) {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("editor_tab_drag")));
                let galley = painter.layout_no_wrap(doc.buf.title.clone(), FontId::proportional(13.0), theme.text);
                let ghost = Rect::from_min_size(pos + vec2(12.0, 12.0), galley.size() + vec2(20.0, 10.0));
                painter.rect(ghost, CornerRadius::same(4), theme.raised, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
                painter.galley(ghost.min + vec2(10.0, 5.0), galley, theme.text);
            }
            if !ctx.input(|i| i.pointer.any_down()) {
                self.drag = None;
                if let Some(at) = self.drop.take() {
                    self.drop_tab((g, t), at);
                }
            }
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

    #[allow(clippy::too_many_arguments)]
    fn group_ui(&mut self, ui: &mut Ui, g: usize, rect: Rect, theme: &Theme, settings: &Settings, git: &Git, pending: &mut Vec<Command>) {
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), TAB_H));
        self.tab_strip(ui, g, strip, theme, pending);
        let mut body = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
        if settings.breadcrumbs && !self.groups[g].tabs.is_empty() {
            let bar = Rect::from_min_size(body.min, vec2(body.width(), 22.0));
            self.breadcrumbs(ui, bar, theme, g);
            body.min.y = bar.bottom();
        }
        if self.groups[g].tabs.is_empty() {
            ui.painter().rect_filled(body, CornerRadius::ZERO, theme.bg);
        } else {
            let t = self.groups[g].active.min(self.groups[g].tabs.len() - 1);
            self.groups[g].active = t;
            let mut tab = std::mem::take(&mut self.groups[g].tabs[t]);
            let doc_id = tab.doc;
            let recent = &mut self.groups[g].recent;
            if self.switcher.is_none() && recent.first() != Some(&doc_id) {
                recent.retain(|doc| *doc != doc_id);
                recent.insert(0, doc_id);
            }
            if let Some(mut doc) = self.docs.remove(&doc_id) {
                self.view(ui, g, body, theme, settings, git, pending, &mut doc, &mut tab);
                self.docs.insert(doc_id, doc);
            }
            self.groups[g].tabs[t] = tab;
        }
        if let Some(pos) = self.drag.and_then(|_| ui.ctx().pointer_latest_pos()).filter(|pos| body.contains(*pos)) {
            let x = (pos.x - body.left()) / body.width();
            let middle = body.center().x;
            let (at, zone) = match x {
                x if x < 0.3 => (DropAt::Split(g, false), body.with_max_x(middle)),
                x if x > 0.7 => (DropAt::Split(g, true), body.with_min_x(middle)),
                _ => (DropAt::Tab(g, self.groups[g].tabs.len()), body),
            };
            self.drop = Some(at);
            ui.painter().rect_filled(zone, CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.12));
        }
    }

    fn tab_strip(&mut self, ui: &mut Ui, g: usize, strip: Rect, theme: &Theme, pending: &mut Vec<Command>) {
        ui.painter().rect_filled(strip, CornerRadius::ZERO, theme.panel);
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, theme.border));
        let infos: Vec<(String, bool, Option<PathBuf>, bool, bool)> = self.groups[g]
            .tabs
            .iter()
            .filter_map(|t| self.docs.get(&t.doc).map(|d| (d.buf.title.clone(), d.buf.is_dirty(), d.buf.path.clone(), t.pinned, t.preview)))
            .collect();
        let active = self.groups[g].active;
        let active_group = g == self.active;
        let mut action = None;
        let last = g + 1 == self.groups.len();
        let tabs_rect = Rect::from_min_max(strip.min, pos2(strip.right() - if last { 60.0 } else { 34.0 }, strip.bottom()));
        let drag_pos = self.drag.and_then(|_| ui.ctx().pointer_latest_pos()).filter(|pos| strip.contains(*pos));
        if drag_pos.is_some() {
            self.drop = Some(DropAt::Tab(g, infos.len()));
        }
        ui.scope_builder(UiBuilder::new().max_rect(tabs_rect), |ui| {
            ScrollArea::horizontal().id_salt(("editor_tabs", g)).scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (t, (title, dirty, path, pinned, preview)) in infos.iter().enumerate() {
                        let pinned = *pinned;
                        let mut name = LayoutJob::simple_singleline(title.clone(), FontId::proportional(13.0), Color32::PLACEHOLDER);
                        name.sections[0].format.italics = *preview && !*dirty;
                        let galley = ui.painter().layout_job(name);
                        let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 66.0, TAB_H), Sense::click_and_drag());
                        if resp.drag_started_by(PointerButton::Primary) {
                            self.drag = Some((g, t));
                        }
                        if let Some(pos) = drag_pos.filter(|pos| r.contains(*pos)) {
                            let after = pos.x > r.center().x;
                            self.drop = Some(DropAt::Tab(g, t + after as usize));
                            let x = if after { r.right() - 1.0 } else { r.left() + 1.0 };
                            ui.painter().vline(x, r.y_range(), Stroke::new(2.0, theme.accent));
                        }
                        let id = Id::new(("editor_tab", g, t));
                        let a = ui::anim(ui, id.with("active"), t == active);
                        let h = ui::anim(ui, id.with("hover"), resp.hovered());
                        let close = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(20.0, 20.0));
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
                            pos2(r.left() + 14.0, r.center().y),
                            Align2::LEFT_CENTER,
                            icons::FILE_CODE,
                            FontId::proportional(14.0),
                            ui::mix(theme.text_dim, theme.accent, a * 0.8),
                        );
                        let text_pos = pos2(r.left() + 34.0, r.center().y - galley.size().y / 2.0);
                        p.galley(text_pos, galley, ui::mix(theme.text_dim, theme.text, a.max(h)));
                        if *dirty && !over_close {
                            p.circle_filled(close.center(), 4.0, ui::mix(theme.text_dim, theme.text, a));
                        } else if pinned || h > 0.0 || t == active {
                            p.rect_filled(close, CornerRadius::same(4), ui::mix(Color32::TRANSPARENT, theme.hover, ch));
                            let shown = if pinned { 1.0 } else { h.max(a) };
                            let color = ui::with_alpha(ui::mix(theme.text_dim, theme.text, ch), shown);
                            let icon = if pinned { icons::PUSH_PIN } else { icons::X };
                            p.text(close.center(), Align2::CENTER_CENTER, icon, FontId::proportional(12.0), color);
                        }
                        if resp.clicked() {
                            action = Some(match (over_close, pinned) {
                                (true, true) => TabAction::Pin(t),
                                (true, false) => TabAction::Close(t),
                                _ => TabAction::Activate(t),
                            });
                        }
                        if resp.double_clicked() && !over_close {
                            action = Some(TabAction::Keep(t));
                        }
                        if resp.middle_clicked() && !pinned {
                            action = Some(TabAction::Close(t));
                        }
                        let tip = path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| title.clone());
                        resp.on_hover_text(tip).context_menu(|ui| {
                            ui.set_min_width(160.0);
                            ui::menu_pad(ui);
                            for (label, choice) in [
                                (if pinned { "Unpin" } else { "Pin" }, TabAction::Pin(t)),
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
        if last {
            let ai = Rect::from_center_size(pos2(strip.right() - 43.0, strip.center().y), vec2(24.0, 24.0));
            ui.scope_builder(UiBuilder::new().max_rect(ai), |ui| {
                if ui::icon_button(ui, theme, icons::ROBOT, 24.0, self.ai_open, "Toggle AI Panel (Ctrl+Alt+B)").clicked() {
                    pending.push(Command::ToggleAi);
                }
            });
        }
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
            Some(TabAction::Keep(t)) => {
                self.groups[g].tabs[t].preview = false;
                self.groups[g].active = t;
                self.active = g;
                self.want_focus = true;
            }
            Some(TabAction::CloseOthers(keep)) => {
                for t in (0..self.groups[g].tabs.len()).rev().filter(|t| *t != keep) {
                    if self.groups[g].tabs[t].pinned {
                        continue;
                    }
                    let doc = self.groups[g].tabs[t].doc;
                    if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                        self.close_tab(g, t);
                    }
                }
            }
            Some(TabAction::CloseAll) => {
                for t in (0..self.groups[g].tabs.len()).rev() {
                    if self.groups[g].tabs[t].pinned {
                        continue;
                    }
                    let doc = self.groups[g].tabs[t].doc;
                    if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                        self.close_tab(g, t);
                    }
                }
            }
            Some(TabAction::Pin(t)) => {
                let tab = &mut self.groups[g].tabs[t];
                tab.preview = false;
                tab.pinned ^= true;
                let at = if tab.pinned { usize::MAX } else { 0 };
                self.place_tab((g, t), g, at);
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
        let (key, flags) = (&f.key, [f.case, f.word, f.regex]);
        f.scope = f.scope.map(|(from, to)| (from.min(buf.len()), to.min(buf.len())));
        if key.0 == doc_id && key.1 == buf.version && key.2 == f.query && key.3 == flags && key.4 == f.scope {
            return;
        }
        let typed = key.2 != f.query || key.3 != flags;
        (f.matches, f.error) = match Pattern::new(&f.query, f.case, f.word, f.regex) {
            Ok(pattern) => (buf.find_all(&pattern, f.scope), None),
            Err(error) => (vec![], Some(error)),
        };
        f.key = (doc_id, buf.version, f.query.clone(), flags, f.scope);
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
        let Doc { buf, hl, max_cols, conflicts, lang, folds, depths, stale, .. } = doc;
        if let (Some((line, delta)), Some(path)) = (settle(buf, hl, lang, &mut self.lsp, folds), &buf.path) {
            self.marks.shift(path, line, delta);
        }
        buf.clamp(&mut tab.sels);

        ui.painter().rect_filled(body, CornerRadius::ZERO, theme.bg);
        let minimap_w = if settings.minimap && body.width() > 420.0 { 78.0 } else { 0.0 };
        let text_rect = Rect::from_min_max(body.min, pos2(body.right() - minimap_w, body.bottom()));
        let gutter_of = |lines: usize| (lines.to_string().len().max(3) as f32 + 5.0) * char_w + 6.0;
        let wrap_cols = settings.word_wrap.then(|| (((text_rect.width() - gutter_of(buf.lines()) - 16.0) / char_w) as usize).max(20));

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
            for binding in command::bindings().iter().filter(|b| b.scope == Scope::Editor && b.chord.is_none()) {
                if ui.input_mut(|i| i.consume_shortcut(&binding.shortcut)) {
                    pending.push(binding.command.clone());
                }
            }
            let page = ((text_rect.height() / line_h) as usize).saturating_sub(2).max(1);
            let mut events = ui.input(|i| i.events.clone());
            let unit = indent_unit(buf, settings).2;
            self.intel.unit = unit.clone();
            self.popup_keys(&mut events, doc_id, buf, tab);
            let (mut typed, mut pasted) = (None, None);
            for event in &events {
                let from = tab.sels.first().map_or(0, |s| s.start());
                let stepped = match event {
                    Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => {
                        snippet::step(&mut tab.snippet, &mut tab.sels, !modifiers.shift) || (!modifiers.shift && emmet_expand(buf, tab, &unit, true))
                    }
                    Event::Key { key: Key::Escape, pressed: true, .. } => tab.snippet.take().is_some() && false,
                    _ => false,
                };
                if stepped || handle_event(buf, tab, event, settings, page, &ctx, wrap_cols, folds) {
                    snippet::track(&mut tab.snippet, buf.len(), &tab.sels);
                    self.last_input = now;
                    tab.reveal = tab.sels.last().map(|s| s.head);
                    match event {
                        Event::Text(text) if tab.sels.len() == 1 => typed = text.chars().next().filter(|_| text.chars().count() == 1),
                        Event::Paste(_) if tab.sels.len() == 1 => pasted = Some((from, tab.sels[0].head)),
                        _ => {}
                    }
                }
            }
            if let (Some((line, delta)), Some(path)) = (settle(buf, hl, lang, &mut self.lsp, folds), &buf.path) {
                self.marks.shift(path, line, delta);
            }
            if let Some(c) = typed {
                let prose = matches!(hl.language.as_str(), "Plain Text" | "Markdown");
                self.typed(c, doc_id, buf, lang, tab, prose, &ctx);
            }
            if let Some((from, to)) = pasted.filter(|_| settings.format_on_paste) {
                self.format_range(doc_id, buf, lang, from, to, settings, &ctx);
            }
        }
        let find_open = self.find.open && is_active;
        if find_open {
            self.refresh_matches(doc_id, buf, tab);
        }
        folds.retain(|f| !tab.sels.iter().any(|s| (f.0 + 1..=f.1).contains(&buf.line_of(s.head))));
        tab.preview &= !buf.is_dirty();

        let total = buf.lines();
        if max_cols.0 != buf.version {
            *max_cols = (buf.version, (0..total).map(|l| buf.line_len(l)).max().unwrap_or(0).min(MAX_COLS));
        }
        // shortcut: nesting counts brackets in strings and comments too, upgrade to syntax scopes if the colors drift
        let colored = settings.bracket_colors && buf.rope.len_bytes() < 1_000_000;
        if colored && depths.0 != buf.version {
            let mut depth = 0u32;
            let per_line = buf.rope.lines().map(|line| {
                let before = depth;
                for c in line.chars() {
                    match c {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                }
                before
            });
            *depths = (buf.version, per_line.collect());
        }
        let depths: &[u32] = if colored { &depths.1 } else { &[] };
        let hex = |rgb: u32| Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        let brackets = if theme.dark { [0xFFD700, 0xDA70D6, 0x179FFF] } else { [0x0431FA, 0x319331, 0x7B3814] }.map(hex);
        let lead_cols = |buf: &Document, line: usize| {
            let lead = buf.rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t');
            lead.fold(0, |width, c| width + if c == '\t' { 4 } else { 1 })
        };
        let head = tab.sels[0].head;
        if tab.pair.0 != (buf.version, head) {
            tab.pair = ((buf.version, head), buf.enclosing(head, head));
        }
        let pair_lines = tab.pair.1.map(|(open, close)| (buf.line_of(open), buf.line_of(close))).filter(|(open, close)| close > open);
        let active_guide = pair_lines.map(|(open, close)| (open, close, lead_cols(buf, open)));
        let guide_step = match indent_unit(buf, settings) {
            (true, size, _) => size.max(1),
            _ => 4,
        };
        let stuck = tab.stuck;
        let mut lens_rows: Vec<(usize, usize)> = match settings.code_lens && lang.attached {
            true => lang.lens.iter().enumerate().filter_map(|(index, lens)| Some((lang.symbols.get(lens.symbol)?.line.min(total - 1), index))).collect(),
            false => vec![],
        };
        lens_rows.sort_unstable();
        lens_rows.dedup_by_key(|row| row.0);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (buf.version, wrap_cols, &*folds, &lens_rows).hash(&mut hasher);
        if tab.rows.0 != hasher.finish() || tab.rows.1.len() != total + 1 {
            tab.rows = (hasher.finish(), build_rows(buf, wrap_cols, folds, &lens_rows));
        }
        let starts = std::mem::take(&mut tab.rows.1);
        let rows = starts[total] as usize;
        let lens_at = |line: usize| match starts[line + 1] > starts[line] {
            true => lens_rows.binary_search_by_key(&line, |row| row.0).ok().map(|at| lens_rows[at].1),
            false => None,
        };
        let top_of = |line: usize| starts[line] as usize + lens_at(line).is_some() as usize;
        let locate = |row: usize| -> (usize, Option<usize>, usize) {
            let line = starts[..total].partition_point(|start| *start as usize <= row).saturating_sub(1);
            match lens_at(line) {
                Some(lens) if row == starts[line] as usize => (line, Some(lens), 0),
                _ => (line, None, row.saturating_sub(top_of(line))),
            }
        };
        let gutter_w = gutter_of(total);
        let spare = if lang.hints.is_empty() { 10.0 } else { 40.0 };
        let wide = if wrap_cols.is_some() { 0.0 } else { gutter_w + (max_cols.1 as f32 + spare) * char_w };
        let content = vec2(wide, rows as f32 * line_h + (text_rect.height() * 0.5).max(line_h));
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
        let scope = self.find.scope.filter(|_| find_open);
        let path = buf.path.clone();
        let problems = match &path {
            Some(path) if settings.diagnostics => self.lsp.diagnostics(path),
            _ => &[],
        };
        let hover_word = self.intel.hover.as_ref().filter(|h| h.doc == doc_id && h.group == g).map(|h| h.word);
        let marks = path.as_deref().map(|p| (self.marks.of(p).to_vec(), self.marks.stopped_line(p), self.marks.live));
        let (mut bp_action, bp_w) = (None, char_w * 2.2 + 2.0);
        let peek_pos = self.intel.peek.as_ref().filter(|p| p.doc == doc_id && is_active).map(|p| p.pos);
        let hint_format = TextFormat {
            font_id: FontId::monospace((settings.font_size * 0.9).round()),
            color: theme.text_dim,
            background: ui::mix(theme.bg, theme.hover, 0.6),
            valign: Align::Center,
            ..Default::default()
        };
        let lens_font = FontId::proportional((settings.font_size * 0.88).round());
        let (mut activated, mut touched, mut pressed_text) = (false, false, false);
        let (mut caret_at, mut hover_at, mut peek_y, mut over_word, mut lens_open) = (None, None, None, None, None);
        let mut toggle_fold = None;
        let folded: &[(usize, usize)] = folds;
        let mut visible = (0, 0);
        let mut area = ScrollArea::both().id_salt(id.with("scroll")).auto_shrink(false);
        if let Some(y) = tab.set_scroll.take() {
            area = area.vertical_scroll_offset(y.max(0.0));
        }
        let buf_ref: &Document = buf;
        let lang_ref: &Lang = lang;
        let root = self.root.clone();
        let out = ui
            .scope_builder(UiBuilder::new().max_rect(text_rect), |ui| {
                area.show_viewport(ui, |ui, viewport| {
                    let (content_rect, _) = ui.allocate_exact_size(vec2(content.x.max(viewport.width()), content.y), Sense::hover());
                    let origin = content_rect.min;
                    let clip = ui.clip_rect();
                    let resp = ui.interact(clip, id, Sense::click_and_drag());
                    let text_x = origin.x + gutter_w;
                    let row_at = |y: f32| locate((((y - origin.y) / line_h).floor().max(0.0) as usize).min(rows - 1));
                    let first = row_at(origin.y + viewport.min.y).0;
                    let last = (row_at(origin.y + viewport.max.y + line_h).0 + 1).min(total);
                    visible = (first, last);
                    if !hl.ensure(&buf_ref.rope, first..last, theme, Duration::from_millis(8)) {
                        ui.ctx().request_repaint();
                    }
                    let hl: &Highlighter = hl;
                    let layout = |painter: &egui::Painter, line: usize| -> Row {
                        let text = visible_text(buf_ref, line);
                        let mut over: Vec<(usize, usize, Color32)> = vec![];
                        for (col, len, tone) in lang_ref.tokens.get(&line).into_iter().flatten().filter(|_| settings.semantic_highlighting) {
                            let (from, to) = (lsp::utf16_to_byte(&text, *col), lsp::utf16_to_byte(&text, col + len));
                            if from < to && over.last().is_none_or(|last| last.1 <= from) {
                                over.push((from, to, intel::tone_color(theme, *tone)));
                            }
                        }
                        if let Some(mut depth) = depths.get(line).copied() {
                            let spans = hl.get(line).map(|spans| spans.as_slice()).unwrap_or_default();
                            let (mut index, mut end) = (0, 0);
                            for (at, c) in text.char_indices().filter(|c| "([{)]}".contains(c.1)) {
                                let open = "([{".contains(c);
                                depth = if open { depth } else { depth.saturating_sub(1) };
                                let shade = brackets[depth as usize % 3];
                                depth += open as u32;
                                while index < spans.len() && end + spans[index].0 <= at {
                                    end += spans[index].0;
                                    index += 1;
                                }
                                let syntax = spans.get(index).map(|span| span.1);
                                if syntax != Some(theme.syntax.string) && syntax != Some(theme.syntax.comment) {
                                    over.push((at, at + 1, shade));
                                }
                            }
                            over.sort_unstable_by_key(|o| o.0);
                            over.dedup_by(|next, kept| next.0 < kept.1);
                        }
                        let (mut inserts, mut hints) = (vec![], vec![]);
                        let from = lang_ref.hints.partition_point(|h| h.line < line);
                        for hint in lang_ref.hints[from..].iter().take_while(|h| h.line == line && settings.inlay_hints) {
                            let col = lsp::utf16_to_char(text.chars(), hint.col);
                            inserts.push((text.char_indices().nth(col).map_or(text.len(), |c| c.0), hint.label.as_str()));
                            hints.push((col, hint.label.chars().count()));
                        }
                        let len = text.chars().count();
                        let mut cuts = vec![0];
                        if let Some(cols) = wrap_cols {
                            buffer::wrap(text.chars(), cols, |at| cuts.push(at));
                        }
                        let byte = |col: usize| if col >= len { text.len() } else { text.char_indices().nth(col).map_or(text.len(), |c| c.0) };
                        let pieces = (0..cuts.len())
                            .map(|i| {
                                let (start, end, last) = (cuts[i], cuts.get(i + 1).copied().unwrap_or(len), i + 1 == cuts.len());
                                let span = byte(start)..byte(end);
                                let within: Vec<(usize, &str)> = inserts.iter().filter(|x| x.0 >= span.start && (x.0 < span.end || last)).copied().collect();
                                let hints = hints.iter().filter(|h| h.0 >= start && (h.0 < end || last)).map(|h| (h.0 - start, h.1)).collect();
                                let galley = painter.layout_job(job(&text, span, hl.get(line), &font, theme.text, &over, &within, &hint_format));
                                Piece { start, galley, hints }
                            })
                            .collect();
                        Row { len, pieces }
                    };
                    let hit = |painter: &egui::Painter, pos: Pos2| -> usize {
                        let (line, lens, piece) = row_at(pos.y);
                        if lens.is_some() {
                            return buf_ref.line_start(line);
                        }
                        buf_ref.line_start(line) + layout(painter, line).hit(piece, pos.x - text_x)
                    };

                    let painter = ui.painter().clone();
                    let pointer = ui.input(|i| i.pointer.interact_pos());
                    let (pressed, mods, down) = ui.input(|i| (i.pointer.primary_pressed(), i.modifiers, i.pointer.any_down()));
                    let on_lens = pointer.is_some_and(|pos| row_at(pos.y).1.is_some());
                    let fold_x = clip.left() + gutter_w - 12.0 - char_w;
                    let over_gutter = resp.hovered() && pointer.is_some_and(|pos| pos.x < clip.left() + gutter_w);
                    let can_fold = |line: usize| folded.iter().any(|f| f.0 == line) || buf_ref.foldable(line);
                    let mut lens_press = false;
                    if let Some(pos) = pointer {
                        if pressed && resp.is_pointer_button_down_on() {
                            resp.request_focus();
                            activated = true;
                            touched = true;
                            pressed_text = true;
                            let p = hit(&painter, pos);
                            let boxed = (mods.alt && mods.shift && !on_lens).then(|| tab.sels.last().map_or(p, |s| s.anchor));
                            tab.box_anchor = boxed;
                            if let Some(anchor) = boxed {
                                tab.sels = buf_ref.box_sels(anchor, p);
                                tab.drag_anchor = None;
                            } else if on_lens {
                                lens_press = true;
                                tab.drag_anchor = None;
                            } else if marks.is_some() && pos.x < clip.left() + bp_w {
                                bp_action = Some(crate::debug::Gutter::Toggle(row_at(pos.y).0));
                                tab.drag_anchor = None;
                            } else if pos.x >= fold_x && pos.x < clip.left() + gutter_w && can_fold(buf_ref.line_of(p)) {
                                toggle_fold = Some(buf_ref.line_of(p));
                                tab.drag_anchor = None;
                            } else if mods.alt {
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
                                if mods.command {
                                    let line = buf_ref.line_of(p);
                                    let (text, column) = (buf_ref.line_text(line), p - buf_ref.line_start(line));
                                    match links(&text).into_iter().find(|link| link.0 <= column && column < link.1) {
                                        Some((from, to)) => ui::open_url(&text.chars().skip(from).take(to - from).collect::<String>()),
                                        None => pending.push(Command::GoToDefinition),
                                    }
                                }
                            }
                        } else if resp.dragged_by(PointerButton::Primary) {
                            let p = hit(&painter, pos);
                            if let Some(anchor) = tab.box_anchor {
                                tab.sels = buf_ref.box_sels(anchor, p);
                            } else if let Some(anchor) = tab.drag_anchor
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
                        if resp.double_clicked() && !on_lens {
                            let (a, b) = buf_ref.word_at(hit(&painter, pos));
                            tab.sels = vec![Sel::new(a, b)];
                            if let (Some((file, line)), Some(root)) = (search_target(buf_ref, buf_ref.line_of(a)), &root) {
                                pending.extend([Command::OpenPath(root.join(file)), Command::GoTo(line, 1)]);
                            }
                        }
                        if resp.triple_clicked() && !on_lens {
                            let line = buf_ref.line_of(hit(&painter, pos));
                            tab.sels = vec![Sel::new(buf_ref.line_start(line), buf_ref.line_start(line + 1).min(buf_ref.len()))];
                        }
                        if resp.secondary_clicked() && !on_lens {
                            let line = (marks.is_some() && pos.x < clip.left() + bp_w).then(|| row_at(pos.y).0);
                            ui.data_mut(|d| d.insert_temp(id.with("bp_menu"), line));
                            let p = hit(&painter, pos);
                            if !tab.sels.iter().any(|s| s.start() <= p && p <= s.end()) {
                                tab.sels = vec![Sel::caret(p)];
                            }
                        }
                        if resp.hovered() && pos.x > clip.left() + gutter_w && !on_lens {
                            ui.ctx().set_cursor_icon(CursorIcon::Text);
                        }
                        if resp.hovered() && !down && !on_lens && pos.x > text_x && pos.y < origin.y + rows as f32 * line_h {
                            let p = hit(&painter, pos);
                            let (a, b) = buf_ref.word_at(p);
                            let (line, _, piece) = row_at(pos.y);
                            let row = layout(&painter, line);
                            let inside = pos.x - text_x <= row.pieces[piece.min(row.pieces.len() - 1)].galley.size().x;
                            if inside && a < b && buf_ref.rope.get_char(a).is_some_and(is_word) {
                                over_word = Some((a, b));
                            }
                        }
                    }
                    if resp.drag_stopped() {
                        merge_sels(&mut tab.sels);
                    }
                    resp.context_menu(|ui| {
                        use Command::*;
                        if let (Some(line), Some((list, ..))) = (ui.data(|d| d.get_temp::<Option<usize>>(id.with("bp_menu"))).flatten(), &marks) {
                            bp_action = crate::debug::gutter_menu(ui, list, line).or(bp_action);
                            return;
                        }
                        ui.set_min_width(230.0);
                        ui::menu_pad(ui);
                        let items = [
                            Some(GoToDefinition), Some(PeekDefinition), Some(GoToImplementation), Some(FindReferences), Some(ShowCallHierarchy), None,
                            Some(RenameSymbol), Some(QuickFix), Some(FormatDocument), None,
                            Some(Cut), Some(Copy), Some(Paste),
                        ];
                        for item in items {
                            let Some(cmd) = item else {
                                ui.separator();
                                continue;
                            };
                            let keys = command::shortcut_text(ui.ctx(), &cmd).unwrap_or_default();
                            if ui.add(egui::Button::new(cmd.label()).shortcut_text(keys)).clicked() {
                                pending.push(cmd);
                                ui.close();
                            }
                        }
                    });

                    let single = tab.sels.len() == 1 && tab.sels[0].is_empty();
                    let current_line = single.then(|| buf_ref.line_of(tab.sels[0].head));
                    let bracket = if single { buf_ref.matching_bracket(tab.sels[0].head) } else { None };

                    for column in &settings.rulers {
                        painter.vline(text_x + *column as f32 * char_w, clip.y_range(), Stroke::new(1.0, ui::with_alpha(theme.border, 0.8)));
                    }
                    for line in (first..last).filter(|line| starts[line + 1] > starts[*line]) {
                        let y = origin.y + top_of(line) as f32 * line_h;
                        let start = buf_ref.line_start(line);
                        let row = layout(&painter, line);
                        let len = row.len;
                        let bottom = y + row.pieces.len() as f32 * line_h;
                        let at = |column: usize| {
                            let piece = row.piece(column.min(len));
                            pos2(text_x + row.x(piece, column.min(len)), y + piece as f32 * line_h)
                        };
                        let spans = |from: usize, to: usize| -> Vec<(f32, f32, f32)> {
                            let part = |piece: usize| {
                                let (a, b) = (from.max(row.pieces[piece].start), to.min(row.end(piece)));
                                (a < b).then(|| (text_x + row.x(piece, a), text_x + row.x(piece, b), y + piece as f32 * line_h))
                            };
                            (0..row.pieces.len()).filter_map(part).collect()
                        };
                        let indent = buf_ref.rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').count();
                        if let Some(index) = lens_at(line) {
                            let lens = &lang_ref.lens[index];
                            let interface = lang_ref.symbols.get(lens.symbol).is_some_and(|s| s.kind == 11);
                            let mut x = at(indent).x;
                            for (found, noun, impls) in [(&lens.refs, "reference", false), (&lens.impls, "implementation", true)] {
                                if impls && !interface {
                                    continue;
                                }
                                let text = match found.as_ref().map(Vec::len) {
                                    Some(1) => format!("1 {noun}"),
                                    Some(count) => format!("{count} {noun}s"),
                                    None => format!("... {noun}s"),
                                };
                                let galley = painter.layout_no_wrap(text, lens_font.clone(), Color32::PLACEHOLDER);
                                let r = Rect::from_min_size(pos2(x, y - line_h + (line_h - galley.size().y) / 2.0 + 1.0), galley.size());
                                let over = on_lens && resp.hovered() && pointer.is_some_and(|p| r.expand(3.0).contains(p));
                                if over && found.is_some() {
                                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                    if lens_press {
                                        lens_open = Some((index, impls));
                                    }
                                }
                                x = r.right() + 16.0;
                                if found.is_none() {
                                    ui::shimmer_text(&painter, theme, r.min, galley);
                                    continue;
                                }
                                painter.galley(r.min, galley, if over && found.is_some() { theme.accent } else { ui::with_alpha(theme.text_dim, 0.85) });
                            }
                        }
                        if current_line == Some(line) {
                            let band = Rect::from_min_max(pos2(clip.left(), y), pos2(clip.right(), bottom));
                            painter.rect_filled(band, CornerRadius::ZERO, ui::mix(theme.bg, theme.hover, 0.4));
                        }
                        if marks.as_ref().is_some_and(|m| m.1 == Some(line)) {
                            let band = Rect::from_min_max(pos2(clip.left(), y), pos2(clip.right(), bottom));
                            painter.rect_filled(band, CornerRadius::ZERO, ui::with_alpha(theme.warning, 0.14));
                        }
                        if settings.guides {
                            let blank = |l: &usize| buf_ref.indent_of(*l) >= buf_ref.line_len(*l);
                            let model = if blank(&line) { (line + 1..(line + 40).min(total)).find(|l| !blank(l)) } else { Some(line) };
                            for column in (0..model.map_or(0, |l| lead_cols(buf_ref, l))).step_by(guide_step) {
                                let lit = active_guide.is_some_and(|(open, close, at)| line > open && line < close && at == column);
                                let color = if lit { ui::with_alpha(theme.accent, 0.7) } else { ui::with_alpha(theme.border, 0.7) };
                                painter.vline(text_x + column as f32 * char_w, y..=bottom, Stroke::new(1.0, color));
                            }
                        }
                        for (x0, x1, y) in scope.into_iter().flat_map(|(a, b)| spans(a.saturating_sub(start), b.saturating_sub(start))) {
                            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h)), CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.09));
                        }
                        let from = matches.partition_point(|m| m.1 <= start);
                        for m in matches[from..].iter().take_while(|m| m.0 <= start + len) {
                            for (x0, x1, y) in spans(m.0.saturating_sub(start), m.1 - start) {
                                let r = Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h));
                                painter.rect_filled(r, CornerRadius::same(2), ui::with_alpha(theme.accent, 0.25));
                            }
                        }
                        for s in tab.sels.iter().filter(|s| !s.is_empty() && s.end() >= start && s.start() <= start + len) {
                            for (x0, x1, y) in spans(s.start().saturating_sub(start), s.end() - start) {
                                painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h)), CornerRadius::same(2), theme.selection);
                            }
                            if s.end() > start + len {
                                painter.rect_filled(Rect::from_min_size(at(len), vec2(char_w * 0.6, line_h)), CornerRadius::same(2), theme.selection);
                            }
                        }
                        if let Some((a, b)) = bracket {
                            for p in [a, b].into_iter().filter(|p| *p >= start && *p < start + len) {
                                for (x0, x1, y) in spans(p - start, p - start + 1) {
                                    let r = Rect::from_min_max(pos2(x0, y + 1.0), pos2(x1, y + line_h - 1.0));
                                    painter.rect(r, CornerRadius::same(2), ui::with_alpha(theme.accent, 0.12), Stroke::new(1.0, ui::with_alpha(theme.accent, 0.7)), StrokeKind::Inside);
                                }
                            }
                        }
                        for (index, piece) in row.pieces.iter().enumerate() {
                            let top = y + index as f32 * line_h + (line_h - piece.galley.size().y) / 2.0;
                            painter.galley(pos2(text_x, top), piece.galley.clone(), theme.text);
                        }
                        let linked = if len >= 10 { buf_ref.line_text(line) } else { String::new() };
                        for (x0, x1, y) in links(&linked).into_iter().flat_map(|link| spans(link.0, link.1)) {
                            painter.hline(x0..=x1, y + line_h - 2.0, Stroke::new(1.0, ui::with_alpha(theme.accent, 0.7)));
                            if mods.command && resp.hovered() && pointer.is_some_and(|pos| pos.x >= x0 && pos.x <= x1 && pos.y >= y && pos.y < y + line_h) {
                                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            }
                        }
                        if settings.render_whitespace {
                            let dim = Stroke::new(1.0, ui::with_alpha(theme.text_dim, 0.5));
                            for (column, c) in buf_ref.rope.line(line).chars().take(len).enumerate().filter(|c| c.1 == ' ' || c.1 == '\t') {
                                let piece = row.piece(column);
                                let (x0, x1) = (text_x + row.x(piece, column), text_x + row.x(piece, column + 1));
                                let mid = y + piece as f32 * line_h + line_h / 2.0;
                                if c == ' ' {
                                    painter.circle_filled(pos2((x0 + x1) / 2.0, mid), 1.0, dim.color);
                                } else {
                                    painter.hline(x0 + 2.0..=x1 - 2.0, mid, dim);
                                    painter.line_segment([pos2(x1 - 5.0, mid - 3.0), pos2(x1 - 2.0, mid)], dim);
                                    painter.line_segment([pos2(x1 - 5.0, mid + 3.0), pos2(x1 - 2.0, mid)], dim);
                                }
                            }
                        }
                        if folded.iter().any(|f| f.0 == line) {
                            let end = at(len);
                            let mark = Rect::from_min_size(pos2(end.x + char_w * 0.5, end.y + 3.0), vec2(char_w * 2.5, line_h - 6.0));
                            painter.rect_filled(mark, CornerRadius::same(3), ui::mix(theme.bg, theme.hover, 0.9));
                            painter.text(mark.center(), Align2::CENTER_CENTER, icons::DOTS_THREE, FontId::proportional(settings.font_size), theme.text_dim);
                        }
                        let upto = problems.partition_point(|d| d.start.0 <= line);
                        for problem in problems[..upto].iter().filter(|d| d.end.0 >= line) {
                            let column = |at: lsp::Pos| lsp::utf16_to_char(buf_ref.rope.line(line).chars(), at.1).min(len);
                            let from = if problem.start.0 == line { column(problem.start) } else { indent.min(len) };
                            let to = if problem.end.0 == line { column(problem.end) } else { len };
                            let first = at(from);
                            let color = intel::severity_color(theme, problem.severity);
                            match problem.severity {
                                4.. if problem.start.0 == line => intel::squiggle(&painter, first.x, first.x + 5.0, first.y + line_h - 2.5, color),
                                4.. => {}
                                _ if to <= from => intel::squiggle(&painter, first.x, first.x + char_w, first.y + line_h - 2.5, color),
                                _ => spans(from, to).into_iter().for_each(|(x0, x1, y)| intel::squiggle(&painter, x0, x1, y + line_h - 2.5, color)),
                            }
                        }
                        for s in tab.sels.iter().filter(|s| s.head >= start && s.head <= start + len) {
                            let p = at(s.head - start);
                            let caret = Rect::from_min_max(pos2(p.x - 0.75, p.y + 1.0), pos2(p.x + 0.75, p.y + line_h - 1.0));
                            painter.rect_filled(caret, CornerRadius::ZERO, ui::with_alpha(theme.accent, caret_alpha));
                        }
                        let within = |pos: usize| pos >= start && pos <= start + len;
                        if let Some(head) = tab.sels.first().map(|s| s.head).filter(|head| within(*head)) {
                            caret_at = Some(at(head - start));
                        }
                        if let Some((word, _)) = hover_word.filter(|word| within(word.0)) {
                            hover_at = Some(at(word - start));
                        }
                        if peek_pos.is_some_and(within) {
                            peek_y = Some(bottom);
                        }
                    }

                    let gutter = Rect::from_min_max(clip.min, pos2(clip.left() + gutter_w - 4.0, clip.bottom()));
                    painter.rect_filled(gutter, CornerRadius::ZERO, theme.bg);
                    let number_font = FontId::monospace(settings.font_size - 1.0);
                    let bp_hover = pointer.filter(|pos| resp.hovered() && pos.x < clip.left() + bp_w).map(|pos| row_at(pos.y).0);
                    for line in (first..last).filter(|line| starts[line + 1] > starts[*line]) {
                        let y = origin.y + top_of(line) as f32 * line_h + line_h / 2.0;
                        if let Some((list, stopped, live)) = &marks {
                            let center = pos2(clip.left() + bp_w / 2.0 + 1.0, y);
                            crate::debug::paint_gutter(&painter, theme, list, *live, *stopped, line, center, (line_h * 0.26).min(char_w * 0.8), bp_hover == Some(line));
                        }
                        let color = if current_line == Some(line) { theme.text } else { ui::with_alpha(theme.text_dim, 0.75) };
                        painter.text(pos2(gutter.right() - 10.0 - char_w, y), Align2::RIGHT_CENTER, line + 1, number_font.clone(), color);
                        let closed = folded.iter().any(|f| f.0 == line);
                        if closed || (over_gutter && buf_ref.foldable(line)) {
                            let icon = if closed { icons::CARET_RIGHT } else { icons::CARET_DOWN };
                            painter.text(pos2(fold_x + char_w / 2.0, y), Align2::CENTER_CENTER, icon, FontId::proportional(settings.font_size - 2.0), theme.text_dim);
                        }
                    }
                    if let Some(changes) = path.as_deref().and_then(|p| git.line_changes(p)) {
                        let from = changes.partition_point(|c| c.0 < first);
                        for (line, kind) in changes[from..].iter().take_while(|c| c.0 <= last) {
                            let (top, end) = if *line < total { (top_of(*line), starts[line + 1] as usize) } else { (rows, rows) };
                            let (y, below) = (origin.y + top as f32 * line_h, origin.y + end as f32 * line_h);
                            let x = gutter.right() - 6.0;
                            if end <= top && *line < total {
                                continue;
                            }
                            let (r, color) = match kind {
                                LineChange::Added => (Rect::from_min_max(pos2(x, y), pos2(x + 3.0, below)), theme.added),
                                LineChange::Modified => (Rect::from_min_max(pos2(x, y), pos2(x + 3.0, below)), theme.modified),
                                LineChange::Deleted => (Rect::from_min_max(pos2(x, y - 2.0), pos2(x + 6.0, y + 2.0)), theme.deleted),
                            };
                            painter.rect_filled(r, CornerRadius::ZERO, color);
                        }
                    }

                    if let Some(pos) = tab.reveal.take() {
                        let line = buf_ref.line_of(pos);
                        let column = pos - buf_ref.line_start(line);
                        let cuts = buf_ref.cuts(line, wrap_cols);
                        let piece = cuts.partition_point(|cut| *cut <= column) - 1;
                        let x = text_x + (column - cuts[piece]) as f32 * char_w;
                        let top = origin.y + (top_of(line) + piece) as f32 * line_h;
                        let above = top - line_h * (stuck + 1) as f32;
                        let target = Rect::from_min_max(pos2(x - gutter_w - char_w * 3.0, above), pos2(x + char_w * 4.0, top + line_h * 2.0));
                        ui.scroll_to_rect(target, None);
                    }
                })
            })
            .inner;
        let scroll_y = out.state.offset.y;
        // shortcut: sticky scopes follow indentation, upgrade to document symbols if a language indents oddly
        tab.stuck = 0;
        if settings.sticky_scroll && scroll_y > 0.0 {
            let blank = |l: usize| buf.indent_of(l) >= buf.line_len(l);
            let scopes = |from: usize| {
                let mut indent = (from..total.min(from + 40)).find(|l| !blank(*l)).map_or(0, |l| buf.indent_of(l));
                let mut chain = vec![];
                for l in (0..from).rev().take(5000).filter(|l| !blank(*l)) {
                    if indent == 0 {
                        break;
                    }
                    if buf.indent_of(l) < indent {
                        indent = buf.indent_of(l);
                        chain.push(l);
                    }
                }
                chain.reverse();
                chain.truncate(5);
                chain
            };
            let top_row = (scroll_y / line_h) as usize;
            let covered = scopes(visible.0).len();
            let chain = scopes(locate((top_row + covered).min(rows - 1)).0);
            let number_font = FontId::monospace(settings.font_size - 1.0);
            for (index, line) in chain.iter().enumerate() {
                let r = Rect::from_min_size(pos2(text_rect.left(), text_rect.top() + index as f32 * line_h), vec2(text_rect.width() - 14.0, line_h));
                let resp = ui.interact(r, id.with(("sticky", index)), Sense::click());
                hl.ensure(&buf.rope, *line..line + 1, theme, Duration::from_millis(2));
                let p = ui.painter().with_clip_rect(r);
                p.rect_filled(r, CornerRadius::ZERO, ui::mix(theme.bg, theme.hover, if resp.hovered() { 0.8 } else { 0.3 }));
                let text = visible_text(buf, *line);
                let galley = p.layout_job(job(&text, 0..text.len(), hl.get(*line), &font, theme.text, &[], &[], &hint_format));
                let left = r.left() + gutter_w;
                p.with_clip_rect(r.with_min_x(left - 4.0)).galley(pos2(left - out.state.offset.x, r.center().y - galley.size().y / 2.0), galley, theme.text);
                p.text(pos2(left - 14.0 - char_w, r.center().y), Align2::RIGHT_CENTER, line + 1, number_font.clone(), ui::with_alpha(theme.text_dim, 0.75));
                if resp.clicked() {
                    tab.sels = vec![Sel::caret(buf.line_start(*line))];
                    tab.set_scroll = Some(top_of(*line).saturating_sub(index) as f32 * line_h);
                    self.want_focus = true;
                }
            }
            if !chain.is_empty() {
                let y = text_rect.top() + chain.len() as f32 * line_h;
                ui.painter().hline(text_rect.left()..=text_rect.right() - 14.0, y, Stroke::new(1.0, theme.border));
            }
            tab.stuck = chain.len();
        }
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
            for problem in problems.iter().filter(|d| d.severity <= 2) {
                let y = mm.top() + problem.start.0 as f32 * scale;
                let mark = Rect::from_min_max(pos2(mm.right() - 5.0, y), pos2(mm.right() - 1.0, y + 2.0));
                p.rect_filled(mark, CornerRadius::ZERO, intel::severity_color(theme, problem.severity));
            }
            let visible_lines = text_rect.height() / line_h;
            let top_row = scroll_y / line_h;
            let top = mm.top() + (locate((top_row as usize).min(rows - 1)).0 as f32 + top_row.fract()) * scale;
            let slider = Rect::from_min_max(pos2(mm.left() + 1.0, top), pos2(mm.right(), top + (visible_lines * scale).max(12.0)));
            let h = ui::anim(ui, id.with("minimap_hover"), resp.hovered() || resp.dragged());
            p.rect_filled(slider, CornerRadius::ZERO, ui::with_alpha(theme.text, 0.06 + 0.08 * h));
            if (resp.clicked() || resp.dragged())
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let line = (((pos.y - mm.top()) / scale - visible_lines / 2.0).max(0.0) as usize).min(total - 1);
                tab.set_scroll = Some(top_of(line) as f32 * line_h);
            }
        }
        tab.rows.1 = starts;
        if tab.seen.0 != buf.version {
            tab.trail.clear();
            tab.seen = (buf.version, tab.sels.clone());
        } else if !ctx.input(|i| i.pointer.any_down()) && !buffer::same(&tab.seen.1, &tab.sels) {
            let before = std::mem::replace(&mut tab.seen.1, tab.sels.clone());
            tab.trail.push(before);
            let extra = tab.trail.len().saturating_sub(100);
            tab.trail.drain(..extra);
        }
        if let (Some(action), Some(path)) = (bp_action, &path) {
            self.marks.apply(path, action);
        }
        if let Some(line) = toggle_fold {
            match folds.iter().position(|f| f.0 == line) {
                Some(at) => drop(folds.remove(at)),
                None => folds.extend(buf.fold_end(line).map(|end| (line, end))),
            }
            folds.sort_unstable();
            evict(buf, folds, &mut tab.sels);
        }

        lang.visible = visible;
        if pressed_text {
            self.intel.dismiss();
        }
        if let Some((index, impls)) = lens_open
            && let Some(lens) = lang.lens.get(index)
            && let Some(symbol) = lang.symbols.get(lens.symbol)
            && let Some(found) = if impls { &lens.impls } else { &lens.refs }
        {
            let title = if impls { "Implementations" } else { "References" };
            self.intel.queued.push(Queued::Peek(doc_id, from_lsp(buf, (symbol.line, symbol.col)), title, found.clone()));
        }
        self.track_hover(over_word, g, doc_id, buf, lang, now, &ctx);
        crate::debug::hover_ui(&ctx, theme, &mut self.marks, (g, doc_id), over_word.map(|(a, b)| buf.text_of(a, b)), now);
        self.hover_ui(&ctx, theme, &font, g, doc_id, hover_at, line_h);
        if is_active {
            self.popups(ui, theme, doc_id, buf, tab, caret_at, line_h, &font);
            self.peek_ui(ui, theme, &font, text_rect, peek_y, doc_id);
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

        if conflicts.0 != buf.version {
            *conflicts = (buf.version, buf.conflicts());
        }
        if !conflicts.1.is_empty() {
            let line = buf.line_of(tab.sels.first().map_or(0, |s| s.head.min(buf.len())));
            let index = conflicts.1.iter().position(|c| c[3] >= line).unwrap_or(0);
            let bottom = body.bottom() - if is_active && self.banner.is_some() { 30.0 } else { 0.0 };
            let rect = Rect::from_min_max(pos2(body.left(), bottom - 30.0), pos2(text_rect.right(), bottom));
            ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(theme.raised, theme.accent, 0.15));
            let (mut take, mut next) = (None, false);
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 3.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.label(RichText::new(format!("Conflict {} of {}", index + 1, conflicts.1.len())).color(theme.text));
                for (label, choice) in [("Accept Current", Take::Current), ("Accept Incoming", Take::Incoming), ("Accept Both", Take::Both)] {
                    if ui.button(label).clicked() {
                        take = Some(choice);
                    }
                }
                next = ui.button("Next").clicked();
            });
            if let Some(take) = take {
                buf.resolve(&mut tab.sels, conflicts.1[index], take);
                tab.reveal = tab.sels.last().map(|s| s.head);
                self.want_focus = true;
            } else if next {
                let pos = buf.line_start(conflicts.1[(index + 1) % conflicts.1.len()][0]);
                tab.sels = vec![Sel::caret(pos)];
                tab.reveal = Some(pos);
                self.want_focus = true;
            }
        }

        if *stale {
            let lower = usize::from(is_active && self.banner.is_some()) + usize::from(!conflicts.1.is_empty());
            let bottom = body.bottom() - 30.0 * lower as f32;
            let rect = Rect::from_min_max(pos2(body.left(), bottom - 30.0), pos2(text_rect.right(), bottom));
            ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(theme.raised, theme.warning, 0.25));
            let mut choice = None;
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 3.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.label(RichText::new("This file changed on disk.").color(theme.text));
                for (index, label) in ["Compare", "Use Disk Version", "Overwrite"].into_iter().enumerate() {
                    if ui.button(label).clicked() {
                        choice = Some(index);
                    }
                }
            });
            match (choice, buf.path.clone()) {
                (Some(0), _) => self.disk_jobs.push((doc_id, true)),
                (Some(1), Some(path)) => match Document::load(&path) {
                    Ok(fresh) => {
                        *hl = Highlighter::new(Some(&path), &fresh.rope);
                        *max_cols = (0, 0);
                        folds.clear();
                        *buf = fresh;
                        tab.sels = vec![Sel::caret(0)];
                        *stale = false;
                    }
                    Err(error) => self.banner = Some((error, None)),
                },
                (Some(2), Some(path)) => {
                    buf.disk = buffer::disk_time(&path);
                    *stale = false;
                    self.disk_jobs.push((doc_id, false));
                }
                _ => {}
            }
        }

        if is_active && let Some((message, since)) = self.banner.clone() {
            let left = since.map_or(f32::INFINITY, |since| 4.0 - since.elapsed().as_secs_f32());
            let t = ui::anim(ui, Id::new("editor_banner"), left > 0.0);
            if left.is_finite() && left > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f32(left));
            } else if left <= 0.0 && t == 0.0 {
                self.banner = None;
            }
            let rect = Rect::from_min_max(pos2(body.left(), body.bottom() - 30.0), pos2(text_rect.right(), body.bottom()));
            ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::with_alpha(ui::mix(theme.raised, theme.warning, 0.2), t));
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 3.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                ui.set_opacity(t);
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
        let size = vec2(500.0f32.min(area.width() - 30.0), 10.0 + rows * 31.0);
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
                let field = ui.add(TextEdit::singleline(&mut self.find.query).margin(ui::FIELD).desired_width(170.0).hint_text("Find"));
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
                    _ if self.find.error.is_some() => ("Bad pattern".to_owned(), theme.error),
                    (_, 0, _) => ("No results".to_owned(), theme.error),
                    (_, n, Some(i)) => (format!("{} of {n}", i + 1), theme.text_dim),
                    (_, n, None) => (format!("{n} found"), theme.text_dim),
                };
                ui.label(RichText::new(label).size(12.0).color(color)).on_hover_text(self.find.error.clone().unwrap_or_default());
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
                    if ui::icon_button(ui, theme, icons::SELECTION, 22.0, self.find.scope.is_some(), "Find in Selection").clicked() {
                        let selected = tab.sels.first().filter(|s| !s.is_empty()).map(|s| (s.start(), s.end()));
                        self.find.scope = if self.find.scope.is_some() { None } else { selected };
                    }
                    if ui::icon_button(ui, theme, icons::ASTERISK, 22.0, self.find.regex, "Use Regular Expression").clicked() {
                        self.find.regex ^= true;
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
                    let field = ui.add(TextEdit::singleline(&mut self.find.with).margin(ui::FIELD).desired_width(170.0).hint_text("Replace"));
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
                    if ui::icon_button(ui, theme, icons::TEXT_T, 22.0, self.find.preserve, "Preserve Case").clicked() {
                        self.find.preserve ^= true;
                    }
                });
            }
        });
        let f = &mut self.find;
        let targets: &[(usize, usize)] = match (replace_all, replace_one, current) {
            (true, ..) => &f.matches,
            (_, true, Some(index)) => &f.matches[index..index + 1],
            _ => &[],
        };
        if let Some(pattern) = Pattern::new(&f.query, f.case, f.word, f.regex).ok().filter(|_| !targets.is_empty()) {
            let before = buf.len();
            let edits = targets.iter().map(|m| (m.0, m.1, pattern.replacement(&buf.text_of(m.0, m.1), &f.with, f.preserve))).collect();
            buf.apply(&mut tab.sels, edits, Kind::Other, After::Carets);
            tab.sels.truncate(1);
            f.scope = f.scope.map(|(from, to)| (from, (to + buf.len()).saturating_sub(before)));
            f.matches = buf.find_all(&pattern, f.scope);
            f.key.1 = buf.version;
            f.key.4 = f.scope;
        }
        if replace_one {
            self.step_match(tab, true);
        } else if step != 0 {
            self.step_match(tab, step > 0);
        }
        if close {
            self.find.open = false;
            self.find.scope = None;
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
        let modal = ui::modal(ctx, "editor_close").show(ctx, |ui| {
            ui::outline(ui);
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
        frame_with(ctx, editors, events, &Settings::default());
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn frame_with(ctx: &egui::Context, editors: &mut Editors, events: Vec<Event>, settings: &Settings) {
        let theme = Theme::dark();
        let git = Git::new();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let mut cmds = vec![];
            editors.ui(ui, &theme, settings, &git, &mut cmds);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn wraps_long_lines_and_moves_through_visual_rows() {
        let ctx = egui::Context::default();
        let settings = Settings { word_wrap: true, ..Settings::default() };
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Paste(format!("{}
end", "word ".repeat(100)))], &settings);
        frame_with(&ctx, &mut editors, vec![key(Key::Home, egui::Modifiers::COMMAND)], &settings);
        frame_with(&ctx, &mut editors, vec![key(Key::ArrowDown, Default::default())], &settings);
        let tab = &editors.groups[0].tabs[0];
        let rows = tab.rows.1.clone();
        assert!(rows.len() == 3 && rows[1] > 1 && rows[2] == rows[1] + 1, "{rows:?}");
        assert!(tab.sels[0].head > 0 && tab.sels[0].head < 500);
        frame_with(&ctx, &mut editors, vec![key(Key::ArrowDown, Default::default()); rows[1] as usize], &settings);
        assert_eq!(editors.status().unwrap().line, 2);
    }

    #[test]
    fn banner_expires_on_its_own() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &Settings::default(), &ctx);
        editors.banner = notice("fresh".into());
        (0..3).for_each(|_| frame(&ctx, &mut editors, vec![]));
        assert!(editors.banner.is_some());
        editors.banner = Some(("old".into(), Instant::now().checked_sub(Duration::from_secs(5))));
        (0..30).for_each(|_| frame(&ctx, &mut editors, vec![]));
        assert!(editors.banner.is_none());
        editors.banner = Some(("error".into(), None));
        (0..30).for_each(|_| frame(&ctx, &mut editors, vec![]));
        assert!(editors.banner.is_some());
    }

    #[test]
    fn finds_links_in_a_line() {
        let text = "see (https://example.com/a?b=1#c). and http://x.io, not httpx or http://";
        let found: Vec<String> = links(text).into_iter().map(|(from, to)| text.chars().skip(from).take(to - from).collect()).collect();
        assert_eq!(found, vec!["https://example.com/a?b=1#c", "http://x.io"]);
        assert!(links("no links here").is_empty());
    }

    #[test]
    fn flags_a_dirty_file_that_changed_on_disk_and_overwrites_on_request() {
        let ctx = egui::Context::default();
        let settings = Settings { lsp: false, ..Settings::default() };
        let path = std::env::temp_dir().join(format!("halo-stale-test-{}.txt", std::process::id()));
        std::fs::write(&path, "one\n").unwrap();
        let mut editors = Editors::new();
        editors.open(&path);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Text("mine ".into())], &settings);
        std::fs::write(&path, "theirs\n").unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(90);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        editors.reload_clean();
        let doc = |e: &Editors| e.docs.values().next().map(|d| (d.stale, d.buf.is_dirty(), d.buf.rope.to_string())).unwrap();
        assert_eq!(doc(&editors), (true, true, "mine one\n".to_owned()));
        editors.exec(&Command::Save, &settings, &ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");

        let click = |editors: &mut Editors, x: f32| {
            let pos = pos2(x, 585.0);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame_with(&ctx, editors, vec![Event::PointerMoved(pos)], &settings);
            frame_with(&ctx, editors, vec![button(true)], &settings);
            frame_with(&ctx, editors, vec![button(false)], &settings);
        };
        let saved = (60..700).rev().step_by(10).any(|x| {
            click(&mut editors, x as f32);
            !doc(&editors).0
        });
        assert!(saved, "a button in the changed-on-disk bar");
        assert_eq!((std::fs::read_to_string(&path).unwrap().as_str(), doc(&editors).1), ("mine one\n", false));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn backs_up_unsaved_files_and_restores_them() {
        let ctx = egui::Context::default();
        let settings = Settings { lsp: false, ..Settings::default() };
        let dir = std::env::temp_dir().join(format!("halo-backup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "disk\n").unwrap();
        std::fs::write(dir.join("clean.txt"), "same\n").unwrap();
        let mut editors = Editors::new();
        editors.open_kept(&dir.join("clean.txt"));
        editors.open_kept(&dir.join("a.txt"));
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Text("mine ".into())], &settings);
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Text("draft".into())], &settings);
        let backups = editors.backup(&dir.join("backups"));
        assert_eq!(backups.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), vec!["a.txt", "Untitled-1"]);
        assert_eq!(editors.backup(&dir.join("backups")), backups);

        let mut again = Editors::new();
        again.open_kept(&dir.join("clean.txt"));
        again.restore(&backups);
        let texts = |e: &Editors| {
            let mut all: Vec<(String, String, bool)> = e.docs.values().map(|d| (d.buf.title.clone(), d.buf.rope.to_string(), d.buf.is_dirty())).collect();
            all.sort();
            all
        };
        let expect = [("Untitled-1", "draft", true), ("a.txt", "mine disk\n", true), ("clean.txt", "same\n", false)];
        assert_eq!(texts(&again), expect.map(|(title, text, dirty)| (title.to_owned(), text.to_owned(), dirty)));
        again.exec(&Command::NewFile, &settings, &ctx);
        assert!(again.docs.values().any(|d| d.buf.title == "Untitled-2"));
        again.discard_all();
        assert!(again.backup(&dir.join("backups")).is_empty());
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draws_whitespace_rulers_guides_and_sticky_scopes() {
        let ctx = egui::Context::default();
        let settings = Settings { render_whitespace: true, rulers: vec![20, 40], ..Settings::default() };
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        let body: String = (0..200).map(|i| format!("        call({i}, [a, \"(\"]);\n")).collect();
        frame_with(&ctx, &mut editors, vec![Event::Paste(format!("mod m {{\n    fn f() {{\n\t{body}    }}\n}}\n"))], &settings);
        editors.exec(&Command::GoTo(100, 5), &settings, &ctx);
        for _ in 0..4 {
            frame_with(&ctx, &mut editors, vec![], &settings);
        }
        let tab = &editors.groups[0].tabs[0];
        assert_eq!(tab.stuck, 2);
        assert!(tab.pair.1.is_some());
        let click = Event::PointerButton { pos: pos2(200.0, TAB_H + 22.0 + 6.0), button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
        frame_with(&ctx, &mut editors, vec![Event::PointerMoved(pos2(200.0, TAB_H + 22.0 + 6.0))], &settings);
        frame_with(&ctx, &mut editors, vec![click.clone()], &settings);
        let Event::PointerButton { pos, button, modifiers, .. } = click else { unreachable!() };
        frame_with(&ctx, &mut editors, vec![Event::PointerButton { pos, button, pressed: false, modifiers }], &settings);
        assert_eq!(editors.status().unwrap().line, 1);
    }

    #[test]
    fn previews_replace_each_other_and_history_moves_both_ways() {
        let ctx = egui::Context::default();
        let settings = Settings { lsp: false, ..Settings::default() };
        let dir = std::env::temp_dir().join(format!("halo-preview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = |name: &str| dir.join(name);
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(file(name), format!("{name}\nsecond\n")).unwrap();
        }
        let mut editors = Editors::new();
        let titles = |e: &Editors| e.groups[0].tabs.iter().map(|t| (e.docs[&t.doc].buf.title.clone(), t.preview)).collect::<Vec<_>>();
        editors.open(&file("a.txt"));
        frame_with(&ctx, &mut editors, vec![], &settings);
        editors.open(&file("b.txt"));
        assert_eq!(titles(&editors), vec![("b.txt".to_owned(), true)]);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Text("x".into())], &settings);
        editors.open(&file("c.txt"));
        assert_eq!(titles(&editors), vec![("b.txt".to_owned(), false), ("c.txt".to_owned(), true)]);
        editors.exec(&Command::KeepEditor, &settings, &ctx);
        editors.open(&file("a.txt"));
        assert_eq!(titles(&editors).len(), 3);

        editors.exec(&Command::GoBack, &settings, &ctx);
        assert_eq!(editors.active_path(), Some(file("c.txt")));
        editors.exec(&Command::GoBack, &settings, &ctx);
        assert_eq!(editors.active_path(), Some(file("b.txt")));
        editors.exec(&Command::GoForward, &settings, &ctx);
        assert_eq!(editors.active_path(), Some(file("c.txt")));

        frame_with(&ctx, &mut editors, vec![], &settings);
        editors.exec(&Command::NextEditor, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        assert_eq!(editors.active_path(), Some(file("b.txt")));
        editors.exec(&Command::CloseEditor, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        editors.closing = None;
        editors.exec(&Command::NextEditor, &settings, &ctx);
        frame_with(&ctx, &mut editors, vec![], &settings);
        let before = editors.active_path().unwrap();
        editors.exec(&Command::CloseEditor, &settings, &ctx);
        assert_eq!(titles(&editors).len(), 2);
        editors.exec(&Command::ReopenClosed, &settings, &ctx);
        assert_eq!((titles(&editors).len(), editors.active_path()), (3, Some(before)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_keeps_a_preview_tab() {
        let ctx = egui::Context::default();
        let settings = Settings { lsp: false, ..Settings::default() };
        let dir = std::env::temp_dir().join(format!("halo-save-preview-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), "text\n").unwrap();
        }
        let mut editors = Editors::new();
        let previews = |e: &Editors| e.groups[0].tabs.iter().map(|t| t.preview).collect::<Vec<_>>();
        editors.open(&dir.join("a.txt"));
        editors.exec(&Command::Save, &settings, &ctx);
        editors.open(&dir.join("b.txt"));
        assert_eq!(previews(&editors), vec![false, true]);
        editors.active_doc().unwrap().0.buf.mark_dirty();
        editors.exec(&Command::SaveAll, &settings, &ctx);
        editors.open(&dir.join("c.txt"));
        assert_eq!(previews(&editors), vec![false, false, true]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completes_snippets_and_expands_emmet_with_tab_stops() {
        let ctx = egui::Context::default();
        let settings = Settings { lsp: false, ..Settings::default() };
        let dir = std::env::temp_dir().join(format!("halo-snippet-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
        let plain = |k: Key| key(k, Default::default());
        let typing = |text: &str| text.chars().map(|c| Event::Text(c.to_string())).collect::<Vec<_>>();

        std::fs::write(dir.join("a.sh"), "").unwrap();
        let mut editors = Editors::new();
        editors.open(&dir.join("a.sh"));
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        for event in typing("tod") {
            frame_with(&ctx, &mut editors, vec![event], &settings);
        }
        frame_with(&ctx, &mut editors, vec![plain(Key::Enter)], &settings);
        assert_eq!(text(&editors), "TODO: ");

        std::fs::write(dir.join("b.html"), "").unwrap();
        let mut editors = Editors::new();
        editors.open(&dir.join("b.html"));
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![], &settings);
        frame_with(&ctx, &mut editors, vec![Event::Text("a.x+p".into())], &settings);
        frame_with(&ctx, &mut editors, vec![plain(Key::Tab)], &settings);
        assert_eq!(text(&editors), "<a class=\"x\" href=\"\"></a>\n<p></p>");
        frame_with(&ctx, &mut editors, typing("u"), &settings);
        frame_with(&ctx, &mut editors, vec![plain(Key::Tab)], &settings);
        frame_with(&ctx, &mut editors, typing("t"), &settings);
        frame_with(&ctx, &mut editors, vec![plain(Key::Tab), plain(Key::Tab)], &settings);
        frame_with(&ctx, &mut editors, typing("!"), &settings);
        assert_eq!(text(&editors), "<a class=\"x\" href=\"u\">t</a>\n<p></p>!");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_by_pattern_inside_a_selection_and_replaces_with_groups() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("a1 b2\nc3 d4\ne5".into())]);
        editors.exec(&Command::Replace, &settings, &ctx);
        editors.find = Find { query: r"([a-z])(\d)".into(), with: "$2$1".into(), regex: true, scope: Some((3, 11)), ..std::mem::take(&mut editors.find) };
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.find.matches, vec![(3, 5), (6, 8), (9, 11)]);
        let click = |editors: &mut Editors, x: f32, y: f32| {
            let pos = pos2(x, y);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&ctx, editors, vec![Event::PointerMoved(pos)]);
            frame(&ctx, editors, vec![button(true)]);
            frame(&ctx, editors, vec![button(false)]);
        };
        let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
        let replaced = (500..880).step_by(8).any(|x| {
            click(&mut editors, x as f32, TAB_H + 22.0 + 51.0);
            text(&editors) != "a1 b2\nc3 d4\ne5"
        });
        assert!(replaced, "a replace button in the find bar");
        assert!(["a1 2b\nc3 d4\ne5", "a1 2b\n3c 4d\ne5"].contains(&text(&editors).as_str()), "{}", text(&editors));
    }

    #[test]
    fn folds_hide_lines_and_carets_step_over_them() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("fn a() {\n    one();\n    two();\n}\nlast".into())]);
        frame(&ctx, &mut editors, vec![key(Key::Home, egui::Modifiers::COMMAND), key(Key::ArrowDown, Default::default())]);
        editors.exec(&Command::Fold, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        let folds = |e: &Editors| e.docs.values().next().unwrap().folds.clone();
        assert_eq!((folds(&editors), editors.status().unwrap().line, editors.groups[0].tabs[0].rows.1.clone()), (vec![(0, 2)], 1, vec![0, 1, 1, 1, 2, 3]));
        frame(&ctx, &mut editors, vec![key(Key::ArrowDown, Default::default())]);
        assert_eq!(editors.status().unwrap().line, 4);
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        assert_eq!(folds(&editors), vec![(0, 2)]);
        editors.exec(&Command::GoTo(2, 1), &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert!(folds(&editors).is_empty());
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
    fn moves_pins_and_splits_tabs() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        for _ in 0..3 {
            editors.exec(&Command::NewFile, &settings, &ctx);
        }
        let docs = |e: &Editors, g: usize| e.groups[g].tabs.iter().map(|t| t.doc).collect::<Vec<_>>();
        editors.drop_tab((0, 0), DropAt::Tab(0, 3));
        assert_eq!((docs(&editors, 0), editors.groups[0].active), (vec![2, 3, 1], 2));

        editors.groups[0].tabs[2].pinned = true;
        editors.place_tab((0, 2), 0, usize::MAX);
        assert_eq!(docs(&editors, 0), vec![1, 2, 3]);
        editors.drop_tab((0, 0), DropAt::Tab(0, 3));
        editors.drop_tab((0, 2), DropAt::Tab(0, 0));
        assert_eq!(docs(&editors, 0), vec![1, 3, 2]);
        editors.groups[0].active = 0;
        editors.exec(&Command::CloseEditor, &settings, &ctx);
        assert_eq!(docs(&editors, 0).len(), 3);

        editors.drop_tab((0, 2), DropAt::Split(0, false));
        assert_eq!((docs(&editors, 0), docs(&editors, 1), editors.active), (vec![2], vec![1, 3], 0));
        editors.drop_tab((0, 0), DropAt::Tab(1, 1));
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.groups.len(), docs(&editors, 0)), (1, vec![1, 2, 3]));
    }

    #[test]
    fn opens_read_only_diff_tabs_and_reuses_them() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        let diff = Some(Path::new("changes.diff"));
        editors.open_text("a.txt (changes)", "@@ -1 +1 @@\r\n-one\r\n+two\r\n", diff);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        editors.open_text("a.txt (changes)", "", diff);
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        assert_eq!(editors.docs.len(), 1);
        assert_eq!(editors.groups[0].tabs.len(), 1);
        assert_eq!(editors.docs.values().next().unwrap().buf.rope.to_string(), "No differences.\n");
        assert_eq!(editors.status().unwrap().language, "Diff");
        assert!(!editors.any_dirty());
    }

    #[test]
    fn accepts_a_conflict_side_from_the_bar() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &Settings::default(), &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nend\n".into())]);
        frame(&ctx, &mut editors, vec![]);
        let click = |editors: &mut Editors, x: f32| {
            let pos = pos2(x, 585.0);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&ctx, editors, vec![Event::PointerMoved(pos)]);
            frame(&ctx, editors, vec![button(true)]);
            frame(&ctx, editors, vec![button(false)]);
        };
        let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
        let resolved = (60..500).step_by(10).any(|x| {
            click(&mut editors, x as f32);
            !text(&editors).contains("<<<<<<<")
        });
        assert!(resolved, "an accept button in the conflict bar");
        assert_eq!(text(&editors), "ours\nend\n");
    }

    #[test]
    fn draws_lens_rows_and_inlay_hints_and_maps_clicks_through_them() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("fn a() {}
fn b() {}
".into())]);
        let symbol = |name: &str, line| lsp::Symbol { name: name.into(), kind: 12, line, col: 3, end_line: line, depth: 0, container: String::new(), path: None };
        let place = lsp::Location { path: "x.rs".into(), start: (0, 0), end: (0, 1), label: String::new() };
        let lang = &mut editors.docs.values_mut().next().unwrap().lang;
        lang.attached = true;
        lang.symbols = vec![symbol("a", 0), symbol("b", 1)];
        lang.lens = (0..2).map(|symbol| intel::Lens { symbol, refs: Some(vec![place.clone()]), impls: None, asked: true }).collect();
        lang.hints = vec![lsp::Hint { line: 1, col: 2, label: ": T".into() }];
        lang.tokens.insert(1, vec![(3, 1, 2)]);
        frame(&ctx, &mut editors, vec![]);

        let font = FontId::monospace(settings.font_size);
        let small = FontId::monospace((settings.font_size * 0.9).round());
        let (char_w, hint_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.glyph_width(&small, 'm'), f.row_height(&font)));
        let line_h = (row_h * 1.25).round();
        let top = TAB_H + 22.0;
        let text_x = 8.0 * char_w + 6.0;
        let click = |editors: &mut Editors, pos: Pos2| {
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&ctx, editors, vec![Event::PointerMoved(pos)]);
            frame(&ctx, editors, vec![button(true)]);
            frame(&ctx, editors, vec![button(false)]);
        };
        click(&mut editors, pos2(text_x + 3.0 * char_w + 3.0 * hint_w + 1.0, top + 3.5 * line_h));
        frame(&ctx, &mut editors, vec![Event::Text("x".into())]);
        let text = |e: &Editors| e.docs.values().next().unwrap().buf.rope.to_string();
        assert_eq!(text(&editors), "fn a() {}
fn xb() {}
");
        assert_eq!(editors.docs.values().next().unwrap().lang.hints.len(), 0);

        click(&mut editors, pos2(text_x + 4.0, top + 2.5 * line_h));
        frame(&ctx, &mut editors, vec![]);
        let peek = editors.intel.peek.as_ref().expect("a click on the lens opens the peek list");
        assert_eq!((peek.doc, peek.pos), (1, 13));
        assert_eq!(text(&editors), "fn a() {}
fn xb() {}
");
        let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, &mut editors, vec![escape]);
        assert!(editors.intel.peek.is_none());
    }

    #[test]
    fn gutter_clicks_toggle_breakpoints_that_follow_edits() {
        let path = std::env::temp_dir().join(format!("halo-breakpoint-test-{}.txt", std::process::id()));
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.open(&path);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        let row_h = ctx.fonts_mut(|f| f.row_height(&FontId::monospace(settings.font_size)));
        let pos = pos2(5.0, TAB_H + 22.0 + 1.5 * (row_h * 1.25).round());
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&ctx, &mut editors, vec![Event::PointerMoved(pos)]);
        frame(&ctx, &mut editors, vec![button(true)]);
        frame(&ctx, &mut editors, vec![button(false)]);
        let lines = |e: &Editors| e.marks.of(&path).iter().map(|b| b.line).collect::<Vec<_>>();
        assert_eq!(lines(&editors), [1]);
        assert_eq!(editors.docs.values().next().unwrap().buf.rope.to_string(), "one\ntwo\nthree\n");
        editors.exec(&Command::GoTo(3, 1), &settings, &ctx);
        editors.exec(&Command::ToggleBreakpoint, &settings, &ctx);
        assert_eq!(lines(&editors), [1, 2]);
        editors.exec(&Command::GoTo(1, 1), &settings, &ctx);
        frame(&ctx, &mut editors, vec![key(Key::Enter, Default::default())]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(lines(&editors), [2, 3]);
        let _ = std::fs::remove_file(&path);
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
