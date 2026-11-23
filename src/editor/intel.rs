use super::buffer::{After, Document, Kind, Sel, is_word};
use super::load::Place;
use super::{Editors, Tab, decor, linked, links, notice, snippet};
use crate::command::Command;
use crate::lsp::{self, Completion, FoldRange, Hint, Location, Lsp, Pos, Signature, Symbol};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Align, Align2, Area, Color32, CornerRadius, Event, FontId, Frame, Galley, Id, Key, Layout, Order, Pos2, Rect, RichText,
    ScrollArea, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct Lens {
    pub symbol: usize,
    pub refs: Option<Vec<Location>>,
    pub impls: Option<Vec<Location>>,
    pub asked: bool,
}

#[derive(Default)]
pub(super) struct Lang {
    pub synced: u64,
    pub attached: bool,
    pub lines: usize,
    pub requested: u64,
    pub retry: f64,
    pub settled: u64,
    pub symbols: Vec<Symbol>,
    pub generation: u64,
    pub lens: Vec<Lens>,
    pub tokens: HashMap<usize, Vec<(usize, usize, u8)>>,
    pub hints: Vec<Hint>,
    pub hinted: (u64, usize, usize),
    pub colors: Option<Vec<lsp::ColorInfo>>,
    pub visible: (usize, usize),
    pub marked: Option<(u64, usize)>,
    pub marks: Vec<(usize, usize)>,
    pub code_lens: Vec<(lsp::CodeLens, bool)>,
    pub lensed: u64,
    pub lens_gen: u64,
    pub bulb_at: Option<(u64, usize)>,
    pub bulb: Option<(bool, Vec<(String, Value)>)>,
    pub folding: Option<Vec<FoldRange>>,
}

impl Lang {
    pub fn edited(&mut self, line: usize, now: usize) -> isize {
        let delta = now as isize - self.lines as isize;
        self.lines = now;
        for symbol in &mut self.symbols {
            symbol.line = lsp::shift(symbol.line, line, delta).unwrap_or(line);
            symbol.end_line = lsp::shift(symbol.end_line, line, delta).unwrap_or(line).max(symbol.line);
        }
        for (lens, _) in &mut self.code_lens {
            lens.line = lsp::shift(lens.line, line, delta).unwrap_or(line);
        }
        for range in self.folding.iter_mut().flatten() {
            (range.0, range.1) = (lsp::shift(range.0, line, delta).unwrap_or(line), lsp::shift(range.1, line, delta).unwrap_or(line));
        }
        self.hints.retain_mut(|hint| hint.line != line && lsp::shift(hint.line, line, delta).map(|moved| hint.line = moved).is_some());
        if let Some(colors) = &mut self.colors {
            colors.retain_mut(|c| c.start.0 != line && lsp::shift(c.start.0, line, delta).map(|moved| (c.end.0, c.start.0) = (c.end.0 - c.start.0 + moved, moved)).is_some());
        }
        if delta == 0 {
            self.tokens.remove(&line);
        } else {
            let old = std::mem::take(&mut self.tokens);
            self.tokens = old.into_iter().filter(|(at, _)| *at != line).filter_map(|(at, found)| Some((lsp::shift(at, line, delta)?, found))).collect();
        }
        delta
    }
}

pub(super) struct Piece {
    pub start: usize,
    pub galley: Arc<Galley>,
    pub hints: Vec<(usize, usize)>,
}

pub(super) struct Row {
    pub len: usize,
    pub pieces: Vec<Piece>,
    pub swatches: Vec<decor::Swatch>,
    pub odd: Vec<(usize, char, Option<char>)>,
}

impl Row {
    pub fn piece(&self, col: usize) -> usize {
        self.pieces.partition_point(|p| p.start <= col).saturating_sub(1)
    }

    pub fn end(&self, piece: usize) -> usize {
        self.pieces.get(piece + 1).map_or(self.len, |next| next.start)
    }

    pub fn x(&self, piece: usize, col: usize) -> f32 {
        let p = &self.pieces[piece];
        super::typography::x(&p.galley, display(&p.hints, col.saturating_sub(p.start)))
    }

    pub fn hit(&self, piece: usize, x: f32) -> usize {
        let piece = piece.min(self.pieces.len() - 1);
        let p = &self.pieces[piece];
        let shown = super::typography::hit(&p.galley, x);
        let last = if piece + 1 < self.pieces.len() { self.end(piece) - 1 } else { self.len };
        (p.start + undisplay(&p.hints, shown)).min(last)
    }
}

pub(super) fn display(hints: &[(usize, usize)], col: usize) -> usize {
    col + hints.iter().filter(|hint| hint.0 < col).map(|hint| hint.1).sum::<usize>()
}

pub(super) fn undisplay(hints: &[(usize, usize)], shown: usize) -> usize {
    let mut shift = 0;
    for (at, len) in hints {
        let start = at + shift;
        if shown <= start {
            break;
        }
        if shown < start + len {
            return *at;
        }
        shift += len;
    }
    shown - shift
}

pub(super) fn to_lsp(buf: &Document, pos: usize) -> Pos {
    let pos = pos.min(buf.len());
    let line = buf.line_of(pos);
    (line, lsp::char_to_utf16(buf.rope.line(line).chars(), pos - buf.line_start(line)))
}

pub(super) fn from_lsp(buf: &Document, (line, col): Pos) -> usize {
    if line >= buf.lines() {
        return buf.len();
    }
    buf.line_start(line) + lsp::utf16_to_char(buf.rope.line(line).chars(), col).min(buf.line_len(line))
}

fn on_word(buf: &Document, head: usize) -> bool {
    let (a, b) = buf.word_at(head);
    b > a && is_word(buf.rope.char(a))
}

pub(super) fn sync(lsp: &mut Lsp, buf: &Document, lang: &mut Lang, ctx: &egui::Context) {
    let Some(path) = buf.path.as_deref().filter(|_| !buf.read_only && !buf.large) else { return };
    if lang.synced == 0 {
        lang.attached = buf.rope.len_bytes() < 2_000_000 && lsp.open(path, || buf.rope.to_string(), ctx);
        lang.lines = buf.lines();
    } else if lang.attached && lang.synced != buf.version {
        lsp.change(path, buf.rope.to_string());
    }
    lang.synced = buf.version;
}

fn tone(name: &str) -> u8 {
    match name {
        "namespace" | "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" | "builtinType" | "typeAlias" | "union"
        | "selfTypeKeyword" => 1,
        "function" | "method" | "macro" | "decorator" => 2,
        "enumMember" | "constParameter" | "const" | "static" => 3,
        _ => 0,
    }
}

pub(super) fn semantic_colors(text: &str, tokens: &[(usize, usize, u8)], theme: &Theme) -> Vec<(usize, usize, Color32)> {
    let mut over = vec![];
    for &(col, len, tone) in tokens {
        let (from, to) = (lsp::utf16_to_byte(text, col), lsp::utf16_to_byte(text, col.saturating_add(len)));
        if from < to && over.last().is_none_or(|last: &(usize, usize, Color32)| last.1 <= from) {
            over.push((from, to, tone_color(theme, tone)));
        }
    }
    over
}

pub(super) fn tone_color(theme: &Theme, tone: u8) -> Color32 {
    let own = theme.semantic[(tone as usize).clamp(1, 3) - 1];
    own.unwrap_or(match tone {
        1 => theme.syntax.type_name,
        2 => theme.syntax.function,
        _ => theme.syntax.constant,
    })
}

pub(super) fn severity_color(theme: &Theme, severity: u8) -> Color32 {
    match severity {
        1 => theme.error,
        2 => theme.warning,
        3 => theme.accent,
        _ => theme.text_dim,
    }
}

pub(super) fn squiggle(painter: &egui::Painter, from: f32, to: f32, y: f32, color: Color32) {
    let mut points = vec![];
    let (mut x, mut up) = (from, true);
    while x <= to {
        points.push(pos2(x, if up { y } else { y + 2.0 }));
        x += 2.0;
        up = !up;
    }
    painter.add(Shape::line(points, Stroke::new(1.0, color)));
}

pub(super) fn popup_frame(theme: &Theme) -> Frame {
    Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.border)).corner_radius(CornerRadius::same(4))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn list_popup(
    ctx: &egui::Context,
    theme: &Theme,
    id: &str,
    (at, line_h): (Pos2, f32),
    rows: &[(&str, &str, &str)],
    (selected, max_rows): (usize, usize),
    reveal: bool,
    font: &FontId,
) -> (Option<usize>, Rect) {
    let mut clicked = None;
    let height = 22.0 * rows.len().clamp(1, max_rows) as f32;
    let at = if at.y + height + 2.0 > ctx.content_rect().bottom() { at - vec2(0.0, line_h + height + 6.0) } else { at };
    let area = Area::new(Id::new(id)).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        popup_frame(theme).show(ui, |ui| {
            ui.set_width(460.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ScrollArea::vertical().min_scrolled_height(height).max_height(height).show(ui, |ui| {
                for (index, (icon, label, detail)) in rows.iter().enumerate() {
                    let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                    ui::row_bg(ui, theme, r, row.id, row.hovered(), index == selected);
                    ui::list_item(ui, &row, label, index == selected, (index, rows.len()));
                    if index == selected {
                        crate::a11y::watch("completion", label);
                    }
                    let painter = ui.painter().with_clip_rect(r.shrink2(vec2(6.0, 0.0)));
                    painter.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, *icon, FontId::proportional(13.0), theme.accent);
                    let end = painter.text(r.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, *label, font.clone(), theme.text);
                    painter.text(pos2(end.right() + 12.0, r.center().y), Align2::LEFT_CENTER, *detail, FontId::proportional(11.5), theme.text_dim);
                    if index == selected && reveal {
                        ui.scroll_to_rect(r, None);
                    }
                    if row.clicked() {
                        clicked = Some(index);
                    }
                }
            });
        });
    });
    (clicked, area.response.rect)
}

fn local_items(buf: &Document, words: bool) -> Vec<Completion> {
    let ext = buf.path.as_deref().and_then(Path::extension).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let item = |label: &str, text: &str, kind: u8, detail: &str| Completion {
        label: label.to_owned(),
        detail: detail.to_owned(),
        kind,
        filter: label.to_owned(),
        sort: label.to_owned(),
        text: text.to_owned(),
        range: None,
        extra: vec![],
        snippet: kind == 15,
    };
    let mut items: Vec<Completion> = snippet::user(&ext).iter().map(|(name, body)| item(name, body, 15, "snippet")).collect();
    if words && buf.rope.len_bytes() < 2_000_000 {
        let text = buf.rope.to_string();
        let starts = |word: &&str| word.len() >= 3 && !word.starts_with(|c: char| c.is_ascii_digit());
        let found: std::collections::BTreeSet<&str> = text.split(|c: char| !is_word(c)).filter(starts).collect();
        items.extend(found.into_iter().take(3000).map(|word| item(word, word, 1, "")));
    }
    items
}

fn md_suggest(suggest: &mut Option<Suggest>, root: Option<&Path>, id: u64, buf: &Document, head: usize, typed: Option<char>) -> bool {
    let Some((start, target)) = super::md_links::target(buf, head) else { return false };
    if typed.is_none_or(|c| matches!(c, '(' | '/' | '#')) {
        let items = super::md_links::complete(buf, root, &target);
        *suggest = (!items.is_empty()).then(|| Suggest { doc: id, start, items, shown: vec![], selected: 0, prefix: None, incomplete: false, dot: true, reveal: true });
    } else if !suggest.as_ref().is_some_and(|p| p.doc == id && p.start == start) {
        *suggest = None;
    }
    true
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

fn problem_blocks(lsp: &Lsp, path: &Path, start: Pos, end: Pos) -> Vec<(u8, String)> {
    let touching = lsp.diagnostics(path).iter().filter(|d| d.start <= end && d.end >= start && d.severity <= 3);
    touching.map(|d| (d.severity + 1, if d.source.is_empty() { d.message.clone() } else { format!("{}  {}", d.message, d.source) })).collect()
}

pub(super) fn problem_at(problems: &[lsp::Diagnostic], at: Pos) -> bool {
    problems.iter().any(|d| d.severity <= 3 && d.start <= at && (at < d.end || at == d.start))
}

fn hover_links(text: &str) -> (String, Vec<(usize, usize, String)>) {
    let (mut out, mut found, mut rest) = (String::new(), vec![], text);
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|mid| Some((mid, mid + 2 + after[mid + 2..].find(')')?)));
        match link.filter(|&(mid, end)| !after[..mid].contains(['[', ']', '\n']) && links(&after[mid + 2..end]) == [(0, after[mid + 2..end].chars().count())]) {
            Some((mid, end)) => {
                let from = out.chars().count();
                out.push_str(&after[..mid]);
                found.push((from, out.chars().count(), after[mid + 2..end].to_owned()));
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    for (from, to) in links(&out) {
        if !found.iter().any(|link| link.0 < to && from < link.1) {
            found.push((from, to, out.chars().skip(from).take(to - from).collect()));
        }
    }
    found.sort();
    (out, found)
}

fn link_text(ui: &mut Ui, theme: &Theme, text: &str, found: &[(usize, usize, String)]) {
    let (font, width, painter) = (egui::TextStyle::Body.resolve(ui.style()), ui.available_width(), ui.painter().clone());
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let layout = |under: Option<usize>| {
        let mut job = LayoutJob { wrap: egui::text::TextWrapping::wrap_at_width(width), ..Default::default() };
        let mut at = 0;
        for (index, (from, to, _)) in found.iter().enumerate() {
            job.append(&text[byte(at)..byte(*from)], 0.0, TextFormat::simple(font.clone(), theme.text));
            let underline = if under == Some(index) { Stroke::new(1.0, theme.accent) } else { Stroke::NONE };
            job.append(&text[byte(*from)..byte(*to)], 0.0, TextFormat { underline, ..TextFormat::simple(font.clone(), theme.accent) });
            at = *to;
        }
        job.append(&text[byte(at)..], 0.0, TextFormat::simple(font.clone(), theme.text));
        painter.layout_job(job)
    };
    let galley = layout(None);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let at = resp.hover_pos().map(|p| usize::from(galley.cursor_from_pos(p - rect.min).index));
    let over = at.and_then(|c| found.iter().position(|link| link.0 <= c && c < link.1));
    if let Some(index) = over {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            ui::open_url(&found[index].2);
        }
    }
    painter.galley(rect.min, if over.is_some() { layout(over) } else { galley }, theme.text);
}

fn hierarchy_title(call: bool, outgoing: bool) -> &'static str {
    match (call, outgoing) {
        (true, false) => "Incoming Calls",
        (true, true) => "Outgoing Calls",
        (false, false) => "Supertypes",
        (false, true) => "Subtypes",
    }
}

pub(super) enum Want {
    Completion { doc: u64, start: usize, dot: bool },
    Signature { doc: u64 },
    Hover { doc: u64, word: (usize, usize) },
    Locations { doc: u64, pos: usize, title: &'static str, peek: bool },
    Lens { doc: u64, generation: u64, symbol: usize, impls: bool },
    Rename,
    Actions { doc: u64, apply: bool },
    Format { doc: u64, version: u64, save: bool },
    Symbols { doc: u64, version: u64 },
    Tokens { doc: u64, version: u64 },
    Hints { doc: u64, version: u64 },
    Colors { doc: u64, version: u64 },
    Present { doc: u64, version: u64 },
    Marks { doc: u64, version: u64, head: usize },
    Folding { doc: u64, version: u64 },
    Grow { group: usize, doc: u64, version: u64, sels: Vec<Sel> },
    Prepare { doc: u64, pos: usize, call: bool, outgoing: bool },
    Hierarchy { doc: u64, pos: usize, call: bool, outgoing: bool },
    CodeLens { doc: u64, version: u64 },
    Resolve { doc: u64, generation: u64, index: usize },
    Bulb { doc: u64, version: u64, line: usize },
    Linked { doc: u64, version: u64, head: usize },
}

pub(super) enum Queued {
    Jump(PathBuf, Pos),
    Peek(u64, usize, &'static str, Vec<Location>),
    Action(u64, Value),
    Hierarchy(u64, usize, bool, bool),
    Lens(u64, lsp::CodeLens),
}

#[derive(Clone, Copy)]
pub(super) enum LensClick {
    Own(usize, bool),
    Server(usize),
}

const LENS_COMMANDS: [&str; 5] =
    ["rust-analyzer.runSingle", "rust-analyzer.debugSingle", "rust-analyzer.showReferences", "editor.action.showReferences", "rust-analyzer.gotoLocation"];

pub(super) fn runs_lens(command: &str, offered: &Value) -> bool {
    LENS_COMMANDS.contains(&command) || offered["commands"].as_array().is_some_and(|list| list.iter().any(|name| name == command))
}

struct Suggest {
    doc: u64,
    start: usize,
    items: Vec<Completion>,
    shown: Vec<usize>,
    selected: usize,
    prefix: Option<String>,
    incomplete: bool,
    dot: bool,
    reveal: bool,
}

pub(super) struct Hover {
    pub group: usize,
    pub doc: u64,
    pub word: (usize, usize),
    since: f64,
    asked: bool,
    pinned: bool,
    blocks: Vec<(u8, String)>,
    drawn: usize,
}

pub(super) struct Peek {
    pub doc: u64,
    pub pos: usize,
    pub group: usize,
    anchor: (u64, usize),
    title: String,
    items: Vec<Location>,
    selected: usize,
    files: HashMap<String, Vec<String>>,
    hierarchy: Option<(bool, bool)>,
    reveal: bool,
    scroll: isize,
}

struct Rename {
    doc: u64,
    pos: usize,
    text: String,
    focus: bool,
}

struct Actions {
    doc: u64,
    items: Vec<(String, Value)>,
    selected: usize,
    reveal: bool,
}

#[derive(Default)]
pub(super) struct Intel {
    pub wants: Vec<(u64, Want)>,
    epoch: u64,
    suggest: Option<Suggest>,
    signature: Option<(u64, Signature)>,
    pub hover: Option<Hover>,
    hover_rect: Option<Rect>,
    pub peek: Option<Peek>,
    rename: Option<Rename>,
    actions: Option<Actions>,
    saving: Option<(u64, f64)>,
    saved: bool,
    history: Vec<(PathBuf, usize)>,
    future: Vec<(PathBuf, usize)>,
    rects: Vec<Rect>,
    last_rects: Vec<Rect>,
    pub queued: Vec<Queued>,
    pub unit: String,
    pub picker: Option<decor::Picker>,
    pub pick_caret: bool,
}

impl Intel {
    pub fn suggesting(&self) -> bool {
        self.suggest.is_some()
    }

    pub fn dismiss(&mut self) {
        self.suggest = None;
        self.signature = None;
        self.actions = None;
        self.hover = None;
    }

    pub fn show_actions(&mut self, doc: u64, items: Vec<(String, Value)>) {
        self.actions = Some(Actions { doc, items, selected: 0, reveal: true });
    }
}

impl Editors {
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        self.root = root;
    }

    pub fn take_saved(&mut self) -> bool {
        std::mem::take(&mut self.intel.saved)
    }

    pub fn outline(&self) -> Option<(&[Symbol], usize)> {
        let (g, t) = self.active_tab()?;
        let tab = &self.groups[g].tabs[t];
        let doc = self.docs.get(&tab.doc)?;
        let line = doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head));
        (!doc.lang.symbols.is_empty()).then_some((doc.lang.symbols.as_slice(), line))
    }

    pub fn outline_loading(&self) -> bool {
        let Some(id) = self.active_tab().map(|(g, t)| self.groups[g].tabs[t].doc) else { return false };
        let Some(doc) = self.docs.get(&id).filter(|d| d.lang.attached) else { return false };
        doc.lang.requested != doc.buf.version || self.intel.wants.iter().any(|(_, want)| matches!(want, Want::Symbols { doc, .. } if *doc == id))
    }

    pub(super) fn process(&mut self, settings: &Settings, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.intel.last_rects = std::mem::take(&mut self.intel.rects);
        self.lsp.configure(settings, self.root.as_deref());
        self.lsp.poll();
        if self.intel.epoch != self.lsp.epoch {
            self.intel.epoch = self.lsp.epoch;
            self.intel.wants.clear();
            for doc in self.docs.values_mut() {
                doc.lang = Lang::default();
                doc.ranges.0 = None;
            }
            if let Some((doc, _)) = self.intel.saving.take() {
                self.finish_save(doc);
            }
        }
        self.validate_links(settings.markdown_validate, now, ctx);
        for doc in self.docs.values_mut() {
            sync(&mut self.lsp, &doc.buf, &mut doc.lang, ctx);
        }
        for edit in std::mem::take(&mut self.lsp.edits) {
            self.apply_workspace_edit(lsp::workspace_edit(&edit));
        }
        for (id, want) in std::mem::take(&mut self.intel.wants) {
            match self.lsp.take(id) {
                Some(reply) => self.reply(want, reply, now),
                None => self.intel.wants.push((id, want)),
            }
        }
        if self.intel.wants.len() > 600 {
            self.intel.wants.drain(..300);
        }
        if let Some((doc, _)) = self.intel.saving.take_if(|saving| now > saving.1) {
            self.finish_save(doc);
        }
        let caret = |g: usize, tab: &Tab| (g == self.active && tab.sels.len() == 1 && tab.sels[0].is_empty()).then(|| tab.sels[0].head);
        let visible: Vec<(u64, Option<usize>)> =
            self.groups.iter().enumerate().filter_map(|(g, group)| group.tabs.get(group.active).map(|tab| (tab.doc, caret(g, tab)))).collect();
        for (id, head) in visible {
            self.refresh(id, head, settings, now, ctx);
        }
        let outside = |rects: &[Rect]| !ctx.pointer_interact_pos().is_some_and(|pos| rects.iter().any(|r| r.contains(pos)));
        if ctx.input(|i| i.pointer.any_pressed()) && outside(&self.intel.last_rects) {
            self.intel.suggest = None;
            self.intel.actions = None;
            self.intel.signature = None;
        }
    }

    fn refresh(&mut self, id: u64, head: Option<usize>, settings: &Settings, now: f64, ctx: &egui::Context) {
        let Some(doc) = self.docs.get_mut(&id) else { return };
        let (buf, lang) = (&doc.buf, &mut doc.lang);
        let Some(path) = buf.path.as_deref().filter(|_| lang.attached) else { return };
        if lang.settled != self.lsp.settled {
            lang.settled = self.lsp.settled;
            lang.requested = 0;
            lang.lensed = 0;
            lang.hinted = (0, 0, 0);
        }
        let version = buf.version;
        let mark = head.filter(|head| on_word(buf, *head)).map(|head| (version, head));
        let bulb = head.filter(|_| settings.lightbulb).map(|head| (version, buf.line_of(head)));
        if now - self.last_input <= 0.3 || now < lang.retry {
            if lang.requested != version || now < lang.retry || lang.marked != mark || lang.bulb_at != bulb {
                ctx.request_repaint_after(Duration::from_millis(350));
            }
            return;
        }
        if lang.marked != mark {
            lang.marked = mark;
            lang.marks.clear();
            if let Some((_, head)) = mark
                && let Some(request) = self.lsp.request(path, "textDocument/documentHighlight", lsp::at(path, to_lsp(buf, head)))
            {
                self.intel.wants.push((request, Want::Marks { doc: id, version, head }));
            }
        }
        if lang.bulb_at != bulb {
            (lang.bulb_at, lang.bulb) = (bulb, None);
            if let (Some((_, line)), Some(head)) = (bulb, head) {
                let at = to_lsp(buf, head);
                let problems: Vec<Value> = self.lsp.diagnostics(path).iter().filter(|d| d.start.0 <= line && d.end.0 >= line).map(|d| d.raw.clone()).collect();
                let context = json!({"diagnostics": problems, "triggerKind": 2});
                let params = json!({"textDocument": {"uri": lsp::uri(path)}, "range": lsp::range_json(at, at), "context": context});
                if let Some(request) = self.lsp.request(path, "textDocument/codeAction", params) {
                    self.intel.wants.push((request, Want::Bulb { doc: id, version, line }));
                }
            }
        }
        let target = json!({"textDocument": {"uri": lsp::uri(path)}});
        if lang.requested != version {
            lang.requested = version;
            if let Some(request) = self.lsp.request(path, "textDocument/documentSymbol", target.clone()) {
                self.intel.wants.push((request, Want::Symbols { doc: id, version }));
            }
            if let Some(request) = self.lsp.request(path, "textDocument/foldingRange", target.clone()) {
                self.intel.wants.push((request, Want::Folding { doc: id, version }));
            }
            if settings.semantic_highlighting
                && let Some(request) = self.lsp.request(path, "textDocument/semanticTokens/full", target.clone())
            {
                self.intel.wants.push((request, Want::Tokens { doc: id, version }));
            }
            if settings.color_decorators
                && let Some(request) = self.lsp.request(path, "textDocument/documentColor", target.clone())
            {
                self.intel.wants.push((request, Want::Colors { doc: id, version }));
            }
        }
        let (first, last) = lang.visible;
        if settings.inlay_hints && last > first && !(lang.hinted.0 == version && lang.hinted.1 <= first && lang.hinted.2 >= last) {
            let (from, to) = (first.saturating_sub(50), last + 50);
            lang.hinted = (version, from, to);
            let mut params = target.clone();
            params["range"] = lsp::range_json((from, 0), (to, 0));
            if let Some(request) = self.lsp.request(path, "textDocument/inlayHint", params) {
                self.intel.wants.push((request, Want::Hints { doc: id, version }));
            }
        }
        if !settings.code_lens {
            return;
        }
        if lang.lensed != version {
            lang.lensed = version;
            if let Some(request) = self.lsp.request(path, "textDocument/codeLens", target.clone()) {
                self.intel.wants.push((request, Want::CodeLens { doc: id, version }));
            }
        }
        let mut budget = 12;
        let (resolves, generation) = (self.lsp.provider(path, "codeLensProvider")["resolveProvider"] == true, lang.lens_gen);
        let unresolved = |lens: &(lsp::CodeLens, bool)| resolves && !lens.1 && lens.0.raw["command"].is_null() && lens.0.line + 2 >= first && lens.0.line <= last;
        for (index, (lens, asked)) in lang.code_lens.iter_mut().enumerate().filter(|(_, lens)| unresolved(lens)) {
            if budget == 0 {
                ctx.request_repaint();
                break;
            }
            budget -= 1;
            *asked = true;
            if let Some(request) = self.lsp.request(path, "codeLens/resolve", lens.raw.clone()) {
                self.intel.wants.push((request, Want::Resolve { doc: id, generation, index }));
            }
        }
        for lens in lang.lens.iter_mut().filter(|lens| !lens.asked) {
            let Some(symbol) = lang.symbols.get(lens.symbol).filter(|s| s.line + 2 >= first && s.line <= last) else { continue };
            if budget == 0 {
                ctx.request_repaint();
                break;
            }
            budget -= 1;
            lens.asked = true;
            let mut params = lsp::at(path, (symbol.line, symbol.col));
            let want = |impls| Want::Lens { doc: id, generation: lang.generation, symbol: lens.symbol, impls };
            if symbol.kind == 11
                && let Some(request) = self.lsp.request(path, "textDocument/implementation", params.clone())
            {
                self.intel.wants.push((request, want(true)));
            }
            params["context"] = json!({"includeDeclaration": false});
            if let Some(request) = self.lsp.request(path, "textDocument/references", params) {
                self.intel.wants.push((request, want(false)));
            }
        }
    }

    fn reply(&mut self, want: Want, reply: Option<Value>, now: f64) {
        let failed = reply.is_none();
        let value = reply.unwrap_or(Value::Null);
        match want {
            Want::Symbols { doc, version } => {
                let Some(d) = self.docs.get_mut(&doc) else { return };
                if failed {
                    d.lang.requested = 0;
                    d.lang.retry = now + 1.0;
                }
                if failed || d.buf.version != version {
                    return;
                }
                let lang = &mut d.lang;
                let before = std::mem::replace(&mut lang.symbols, lsp::symbols(&value));
                let mut carried: Vec<(String, u8, Lens)> = std::mem::take(&mut lang.lens)
                    .into_iter()
                    .filter_map(|lens| before.get(lens.symbol).map(|s| (s.name.clone(), s.kind, lens)))
                    .collect();
                for (index, symbol) in lang.symbols.iter().enumerate() {
                    let taken = lang.lens.last().is_some_and(|lens| lang.symbols[lens.symbol].line == symbol.line);
                    if taken || !matches!(symbol.kind, 5 | 6 | 9 | 10 | 11 | 12 | 23) {
                        continue;
                    }
                    let kept = carried.iter().position(|c| c.0 == symbol.name && c.1 == symbol.kind).map(|at| carried.remove(at).2);
                    let (refs, impls) = kept.map_or((None, None), |lens| (lens.refs, lens.impls));
                    lang.lens.push(Lens { symbol: index, refs, impls, asked: false });
                }
                lang.generation += 1;
            }
            Want::Tokens { doc, version } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| !failed && d.buf.version == version) else { return };
                let Some(path) = d.buf.path.as_deref() else { return };
                let tones: Vec<u8> = self.lsp.token_types(path).iter().map(|name| tone(name)).collect();
                d.lang.tokens.clear();
                for (line, col, len, kind) in lsp::tokens(&value) {
                    if let Some(tone) = tones.get(kind).copied().filter(|tone| *tone > 0) {
                        d.lang.tokens.entry(line).or_default().push((col, len, tone));
                    }
                }
            }
            Want::Hints { doc, version } => {
                let Some(d) = self.docs.get_mut(&doc) else { return };
                if failed {
                    d.lang.hinted = (0, 0, 0);
                    d.lang.retry = now + 1.0;
                } else if d.buf.version == version {
                    d.lang.hints = lsp::hints(&value);
                }
            }
            Want::Colors { doc, version } => {
                if let Some(d) = self.docs.get_mut(&doc).filter(|d| !failed && d.buf.version == version) {
                    d.lang.colors = Some(lsp::colors(&value));
                }
            }
            Want::Present { doc, version } => self.presented(doc, version, &value),
            Want::Marks { doc, version, head } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| !failed && d.buf.version == version && d.lang.marked == Some((version, head))) else { return };
                let marks = lsp::highlights(&value).into_iter().map(|(a, b)| (from_lsp(&d.buf, a), from_lsp(&d.buf, b))).filter(|(a, b)| a < b).collect();
                d.lang.marks = marks;
            }
            Want::Folding { doc, version } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| !failed && d.buf.version == version) else { return };
                d.lang.folding = lsp::folding_ranges(&value);
                d.ranges.0 = None;
            }
            Want::Grow { group, doc, version, sels } => {
                let Some(d) = self.docs.get(&doc).filter(|d| d.buf.version == version) else { return };
                let same = |t: &&mut Tab| t.doc == doc && super::buffer::same(&t.sels, &sels);
                let Some(tab) = self.groups.get_mut(group).and_then(|g| g.tabs.iter_mut().find(same)) else { return };
                let chains = lsp::selection_ranges(&value);
                let wider = sels.iter().enumerate().map(|(at, s)| {
                    let spans = chains.get(at).into_iter().flatten().map(|(a, b)| (from_lsp(&d.buf, *a), from_lsp(&d.buf, *b)));
                    let larger = spans.filter(|(a, b)| *a <= s.start() && *b >= s.end() && b - a > s.end() - s.start()).min_by_key(|(a, b)| b - a);
                    larger.map_or_else(|| d.buf.grow(*s), |(a, b)| Sel::new(a, b))
                });
                super::grow(tab, wider.collect());
            }
            Want::Lens { doc, generation, symbol, impls } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| d.lang.generation == generation) else { return };
                let Some(lens) = d.lang.lens.iter_mut().find(|lens| lens.symbol == symbol) else { return };
                if failed {
                    lens.asked = false;
                    d.lang.retry = now + 1.0;
                } else if impls {
                    lens.impls = Some(lsp::locations(&value));
                } else {
                    lens.refs = Some(lsp::locations(&value));
                }
            }
            Want::Completion { doc, start, dot } => {
                let (mut items, incomplete) = lsp::completions(&value);
                if let Some(d) = self.docs.get(&doc).filter(|_| !dot) {
                    items.extend(local_items(&d.buf, items.is_empty()));
                }
                let single = self.active_tab().map(|(g, t)| &self.groups[g].tabs[t]).is_some_and(|tab| tab.doc == doc && tab.sels.len() == 1 && tab.snippet.is_none());
                if single && !items.is_empty() {
                    self.intel.suggest = Some(Suggest { doc, start, items, shown: vec![], selected: 0, prefix: None, incomplete, dot, reveal: true });
                }
            }
            Want::Signature { doc } => self.intel.signature = lsp::signature(&value).map(|found| (doc, found)),
            Want::Hover { doc, word } => {
                if let Some(hover) = self.intel.hover.as_mut().filter(|h| h.doc == doc && h.word == word) {
                    hover.blocks.extend(lsp::blocks(&lsp::hover(&value)).into_iter().map(|(code, text)| (code as u8, text)));
                }
            }
            Want::Locations { doc, pos, title, peek } => {
                let found = lsp::locations(&value);
                match found.as_slice() {
                    [] => self.banner = notice(format!("No {} found.", title.to_lowercase())),
                    [only] if !peek => {
                        let (path, start) = (only.path.clone(), only.start);
                        self.jump(&path, start, true);
                    }
                    _ => self.open_peek(doc, pos, title, found, None),
                }
            }
            Want::Rename => match lsp::workspace_edit(&value) {
                changes if changes.is_empty() => self.banner = notice("The language server could not rename this symbol.".into()),
                changes => self.apply_workspace_edit(changes),
            },
            Want::Actions { doc, apply } => {
                let found = lsp::actions(&value);
                match found.first() {
                    Some((_, action)) if apply => self.run_action(doc, action.clone()),
                    Some(_) => self.intel.actions = Some(Actions { doc, items: found, selected: 0, reveal: true }),
                    None if apply => {}
                    None => self.banner = notice("No code actions are available here.".into()),
                }
            }
            Want::Format { doc, version, save } => {
                if !failed && self.docs.get(&doc).is_some_and(|d| d.buf.version == version) {
                    self.edit_doc(doc, &lsp::edits(&value));
                }
                if save && self.intel.saving.take_if(|saving| saving.0 == doc).is_some() {
                    self.finish_save(doc);
                }
            }
            Want::Linked { doc, version, head } => {
                let Some(d) = self.docs.get(&doc).filter(|d| d.buf.version == version) else { return };
                let found = match lsp::linked_ranges(&value) {
                    Some((ranges, word)) => Some((ranges.iter().map(|r| (from_lsp(&d.buf, r.0), from_lsp(&d.buf, r.1))).collect::<Vec<_>>(), word)),
                    None => linked::builtin(&d.buf, head).map(|ranges| (ranges, None)),
                };
                let Some((ranges, word)) = found else { return };
                for tab in self.groups.iter_mut().flat_map(|g| &mut g.tabs).filter(|t| t.doc == doc && t.sels.len() == 1 && t.sels[0].head == head) {
                    tab.link.set(&d.buf, ranges.clone(), word.as_deref());
                }
            }
            Want::Prepare { doc, pos, call, outgoing } => {
                let item = value.as_array().and_then(|items| items.first());
                let path = self.docs.get(&doc).and_then(|d| d.buf.path.clone());
                let (Some(item), Some(path)) = (item, path) else {
                    self.banner = notice("No hierarchy is available here.".into());
                    return;
                };
                let method = match (call, outgoing) {
                    (true, false) => "callHierarchy/incomingCalls",
                    (true, true) => "callHierarchy/outgoingCalls",
                    (false, false) => "typeHierarchy/supertypes",
                    (false, true) => "typeHierarchy/subtypes",
                };
                if let Some(request) = self.lsp.request(&path, method, json!({"item": item})) {
                    self.intel.wants.push((request, Want::Hierarchy { doc, pos, call, outgoing }));
                }
            }
            Want::Hierarchy { doc, pos, call, outgoing } => {
                self.open_peek(doc, pos, hierarchy_title(call, outgoing), lsp::hierarchy(&value), Some((call, outgoing)));
            }
            Want::CodeLens { doc, version } => {
                let Some(d) = self.docs.get_mut(&doc) else { return };
                if failed {
                    d.lang.lensed = 0;
                    d.lang.retry = now + 1.0;
                }
                if failed || d.buf.version != version {
                    return;
                }
                let mut old = std::mem::take(&mut d.lang.code_lens);
                let mut found = lsp::code_lenses(&value);
                for lens in found.iter_mut().filter(|lens| lens.raw["command"].is_null()) {
                    if let Some(at) = old.iter().position(|o| o.0.line == lens.line && o.0.raw["command"].is_null() && !o.0.title.is_empty()) {
                        let kept = old.remove(at).0;
                        (lens.title, lens.command, lens.args) = (kept.title, kept.command, kept.args);
                    }
                }
                d.lang.code_lens = found.into_iter().map(|lens| (lens, false)).collect();
                d.lang.lens_gen += 1;
            }
            Want::Resolve { doc, generation, index } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| d.lang.lens_gen == generation) else { return };
                let Some((lens, asked)) = d.lang.code_lens.get_mut(index) else { return };
                if failed {
                    *asked = false;
                    d.lang.retry = now + 1.0;
                } else {
                    let found = lsp::code_lens(&value);
                    (lens.title, lens.command, lens.args) = (found.title, found.command, found.args);
                }
            }
            Want::Bulb { doc, version, line } => {
                let Some(d) = self.docs.get_mut(&doc).filter(|d| d.lang.bulb_at == Some((version, line))) else { return };
                let found = lsp::actions(&value);
                d.lang.bulb = (!found.is_empty()).then(|| (lsp::quick_fix(&found), found));
            }
        }
    }

    fn lines_of(&self, path: &Path) -> Vec<String> {
        let wanted = lsp::key(path);
        match self.docs.values().find(|d| d.buf.path.as_deref().is_some_and(|p| lsp::key(p) == wanted)) {
            Some(doc) => doc.buf.rope.lines().map(|line| line.to_string().trim_end_matches('\n').to_owned()).collect(),
            None => std::fs::read_to_string(path).unwrap_or_default().lines().map(str::to_owned).collect(),
        }
    }

    fn open_peek(&mut self, doc: u64, pos: usize, title: &str, mut items: Vec<Location>, hierarchy: Option<(bool, bool)>) {
        items.truncate(500);
        let mut files: HashMap<String, Vec<String>> = HashMap::new();
        for item in &mut items {
            let lines = files.entry(lsp::key(&item.path)).or_insert_with(|| self.lines_of(&item.path));
            let text = lines.get(item.start.0).map(|line| line.trim()).unwrap_or_default();
            item.label = if item.label.is_empty() { text.to_owned() } else { format!("{}   {text}", item.label) };
        }
        let anchor = self.docs.get(&doc).map_or((0, 0), |d| {
            let line = d.buf.line_of(pos.min(d.buf.len()));
            (d.buf.version, d.buf.line_start(line) + d.buf.line_len(line))
        });
        let group = self.active;
        self.intel.peek = Some(Peek { doc, pos, group, anchor, title: title.to_owned(), items, selected: 0, files, hierarchy, reveal: true, scroll: 0 });
    }

    pub(super) fn peek_line(&mut self, g: usize, doc: u64, buf: &Document) -> Option<usize> {
        let peek = self.intel.peek.as_mut().filter(|p| p.doc == doc && p.group == g)?;
        let (version, at) = peek.anchor;
        if version != buf.version {
            let line = match (buf.mapped(version, at), buf.mapped(version, 0)) {
                (Some(moved), _) => {
                    peek.pos = buf.mapped(version, peek.pos).unwrap_or(moved);
                    buf.line_of(moved)
                }
                (None, Some(_)) => {
                    self.intel.peek = None;
                    return None;
                }
                (None, None) => {
                    peek.pos = peek.pos.min(buf.len());
                    buf.line_of(at.min(buf.len()))
                }
            };
            peek.anchor = (buf.version, buf.line_start(line) + buf.line_len(line));
        }
        Some(buf.line_of(peek.anchor.1))
    }

    pub(super) fn find_doc(&self, path: &Path) -> Option<u64> {
        let wanted = lsp::key(path);
        self.docs.iter().find(|(_, d)| d.buf.path.as_deref().is_some_and(|p| lsp::key(p) == wanted)).map(|(id, _)| *id)
    }

    fn here(&self) -> Option<(PathBuf, usize)> {
        let (g, t) = self.active_tab()?;
        let tab = &self.groups[g].tabs[t];
        Some((self.docs.get(&tab.doc)?.buf.path.clone()?, tab.sels.first().map_or(0, |s| s.head)))
    }

    pub(super) fn remember(&mut self) {
        let Some(here) = self.here() else { return };
        self.intel.future.clear();
        if self.intel.history.last() != Some(&here) {
            self.intel.history.push(here);
            let extra = self.intel.history.len().saturating_sub(60);
            self.intel.history.drain(..extra);
        }
    }

    fn show_path(&mut self, path: &Path) {
        self.open_as(path, true);
    }

    fn jump(&mut self, path: &Path, at: Pos, record: bool) {
        if record {
            self.remember();
        }
        self.show_path(path);
        if !self.pend_place(Place::Lsp(at)) {
            self.with_active(|buf, tab| tab.sels = vec![Sel::caret(from_lsp(buf, at))]);
        }
    }

    fn apply_workspace_edit(&mut self, changes: Vec<(PathBuf, Vec<lsp::Edit>)>) {
        let active = self.active_tab();
        for (path, edits) in changes {
            let id = match self.find_doc(&path) {
                Some(id) => {
                    self.load_now(id);
                    id
                }
                None => match Document::load(&path) {
                    Ok(buf) => {
                        let id = self.insert_doc(buf);
                        self.add_tab(id, vec![Sel::caret(0)]);
                        id
                    }
                    Err(error) => {
                        self.banner = notice(error);
                        continue;
                    }
                },
            };
            self.edit_doc(id, &edits);
        }
        if let Some((g, t)) = active {
            self.active = g;
            self.groups[g].active = t;
        }
    }

    fn edit_doc(&mut self, id: u64, edits: &[lsp::Edit]) {
        let Some(doc) = self.docs.get_mut(&id) else { return };
        let list = edits.iter().map(|e| (from_lsp(&doc.buf, e.start), from_lsp(&doc.buf, e.end), e.text.clone())).collect();
        match self.groups.iter_mut().flat_map(|group| &mut group.tabs).find(|tab| tab.doc == id) {
            Some(tab) => doc.buf.apply(&mut tab.sels, list, Kind::Other, After::Map),
            None => doc.buf.apply(&mut vec![Sel::caret(0)], list, Kind::Other, After::Map),
        };
    }

    fn run_action(&mut self, doc: u64, action: Value) {
        let changes = lsp::workspace_edit(&action["edit"]);
        if !changes.is_empty() {
            self.apply_workspace_edit(changes);
        }
        let command = if action["command"].is_object() { &action["command"] } else { &action };
        let path = self.docs.get(&doc).and_then(|d| d.buf.path.clone());
        if let (Some(name), Some(path)) = (command["command"].as_str(), path) {
            self.lsp.request(&path, "workspace/executeCommand", json!({"command": name, "arguments": command["arguments"]}));
        }
    }

    fn prepare(&mut self, doc: u64, pos: usize, call: bool, outgoing: bool) {
        let Some(d) = self.docs.get(&doc) else { return };
        let Some(path) = d.buf.path.clone() else { return };
        let method = if call { "textDocument/prepareCallHierarchy" } else { "textDocument/prepareTypeHierarchy" };
        match self.lsp.request(&path, method, lsp::at(&path, to_lsp(&d.buf, pos))) {
            Some(request) => self.intel.wants.push((request, Want::Prepare { doc, pos, call, outgoing })),
            None => self.banner = notice(format!("The language server does not provide {}.", hierarchy_title(call, outgoing).to_lowercase())),
        }
    }

    pub(super) fn after_views(&mut self) {
        for item in std::mem::take(&mut self.intel.queued) {
            match item {
                Queued::Jump(path, at) => self.jump(&path, at, true),
                Queued::Peek(doc, pos, title, items) => self.open_peek(doc, pos, title, items, None),
                Queued::Action(doc, action) => self.run_action(doc, action),
                Queued::Hierarchy(doc, pos, call, outgoing) => self.prepare(doc, pos, call, outgoing),
                Queued::Lens(doc, lens) => self.run_lens(doc, lens),
            }
        }
    }

    fn run_lens(&mut self, doc: u64, lens: lsp::CodeLens) {
        let args = &lens.args;
        match lens.command.as_str() {
            "rust-analyzer.runSingle" | "rust-analyzer.debugSingle" => {
                let Some((line, cwd, test)) = lsp::runnable(&args[0]) else {
                    return ui::notify_info(format!("{} is not a cargo command, so Halo cannot run it.", lens.title.trim()));
                };
                let debug = lens.command.ends_with("debugSingle");
                match test.filter(|_| debug) {
                    Some(id) => self.tests.clicked = Some((crate::test_parse::Kind::Rust, id, true)),
                    None => {
                        if debug {
                            ui::notify_info("Halo can debug single cargo tests only, so this runs without the debugger.");
                        }
                        self.tests.run = Some((line, cwd));
                    }
                }
            }
            "rust-analyzer.showReferences" | "editor.action.showReferences" => {
                let target = args[0].as_str().and_then(lsp::path_of).and_then(|path| self.find_doc(&path)).unwrap_or(doc);
                let Some(d) = self.docs.get(&target) else { return };
                let pos = from_lsp(&d.buf, lsp::pos(&args[1]));
                let title = if lens.title.contains("implementation") { "Implementations" } else { "References" };
                self.open_peek(target, pos, title, lsp::locations(&args[2]), None);
            }
            "rust-analyzer.gotoLocation" => {
                if let Some(found) = lsp::locations(&args[0]).into_iter().next() {
                    self.jump(&found.path, found.start, true);
                }
            }
            name => {
                if let Some(path) = self.docs.get(&doc).and_then(|d| d.buf.path.clone()) {
                    self.lsp.request(&path, "workspace/executeCommand", json!({"command": name, "arguments": args}));
                }
            }
        }
    }

    fn finish_save(&mut self, id: u64) {
        if self.save_doc(id, false) {
            self.intel.saved = true;
        }
    }

    pub(super) fn save_formatted(&mut self, id: u64, settings: &Settings, ctx: &egui::Context) -> bool {
        let Some(doc) = self.docs.get_mut(&id).filter(|d| settings.of(d.buf.path.as_deref()).format_on_save) else { return false };
        let settings = settings.of(doc.buf.path.as_deref());
        sync(&mut self.lsp, &doc.buf, &mut doc.lang, ctx);
        let Some(path) = doc.buf.path.clone().filter(|_| doc.lang.attached) else { return false };
        let options = json!({"tabSize": settings.tab_size, "insertSpaces": settings.insert_spaces});
        let params = json!({"textDocument": {"uri": lsp::uri(&path)}, "options": options});
        let Some(request) = self.lsp.request(&path, "textDocument/formatting", params) else { return false };
        self.intel.wants.push((request, Want::Format { doc: id, version: doc.buf.version, save: true }));
        self.intel.saving = Some((id, ctx.input(|i| i.time) + 1.5));
        ctx.request_repaint_after(Duration::from_millis(1600));
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn format_range(&mut self, id: u64, buf: &Document, lang: &mut Lang, from: usize, to: usize, settings: &Settings, ctx: &egui::Context) {
        let Some(path) = buf.path.clone().filter(|_| lang.attached && from < to) else { return };
        sync(&mut self.lsp, buf, lang, ctx);
        let options = json!({"tabSize": settings.tab_size, "insertSpaces": settings.insert_spaces});
        let span = lsp::range_json(to_lsp(buf, from), to_lsp(buf, to));
        let params = json!({"textDocument": {"uri": lsp::uri(&path)}, "range": span, "options": options});
        if let Some(request) = self.lsp.request(&path, "textDocument/rangeFormatting", params) {
            self.intel.wants.push((request, Want::Format { doc: id, version: buf.version, save: false }));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn format_on_type(&mut self, c: char, id: u64, buf: &Document, lang: &mut Lang, head: usize, settings: &Settings, ctx: &egui::Context) {
        let Some(path) = buf.path.clone().filter(|p| lang.attached && self.lsp.triggers(p, "documentOnTypeFormattingProvider").contains(&c)) else { return };
        sync(&mut self.lsp, buf, lang, ctx);
        let mut params = lsp::at(&path, to_lsp(buf, head));
        params["ch"] = json!(c.to_string());
        params["options"] = json!({"tabSize": settings.tab_size, "insertSpaces": settings.insert_spaces});
        if let Some(request) = self.lsp.request(&path, "textDocument/onTypeFormatting", params) {
            self.intel.wants.push((request, Want::Format { doc: id, version: buf.version, save: false }));
        }
    }

    pub(super) fn ask_linked(&mut self, id: u64, buf: &Document, lang: &mut Lang, tab: &mut Tab, head: usize, ctx: &egui::Context) {
        if let Some(path) = buf.path.clone().filter(|_| lang.attached) {
            sync(&mut self.lsp, buf, lang, ctx);
            if let Some(request) = self.lsp.request(&path, "textDocument/linkedEditingRange", lsp::at(&path, to_lsp(buf, head))) {
                self.intel.wants.push((request, Want::Linked { doc: id, version: buf.version, head }));
                return;
            }
        }
        if let Some(ranges) = linked::builtin(buf, head) {
            tab.link.set(buf, ranges, None);
        }
    }

    pub(super) fn intel_exec(&mut self, cmd: &Command, settings: &Settings, ctx: &egui::Context) -> bool {
        use Command::*;
        if matches!(cmd, GoBack | GoForward) {
            let here = self.here();
            let intel = &mut self.intel;
            let (from, to) = if *cmd == GoBack { (&mut intel.history, &mut intel.future) } else { (&mut intel.future, &mut intel.history) };
            if let Some((path, pos)) = from.pop() {
                to.extend(here);
                self.show_path(&path);
                self.with_active(|_, tab| tab.sels = vec![Sel::caret(pos)]);
            }
            return true;
        }
        if *cmd == ExpandSelection
            && let Some((g, t)) = self.active_tab()
            && let tab = &self.groups[g].tabs[t]
            && let Some(doc) = self.docs.get_mut(&tab.doc)
            && let Some(path) = doc.buf.path.clone().filter(|_| doc.lang.attached)
        {
            sync(&mut self.lsp, &doc.buf, &mut doc.lang, ctx);
            let at = |s: &Sel| to_lsp(&doc.buf, s.head);
            let positions: Vec<Value> = tab.sels.iter().map(at).map(|(line, character)| json!({"line": line, "character": character})).collect();
            let params = json!({"textDocument": {"uri": lsp::uri(&path)}, "positions": positions});
            let Some(request) = self.lsp.request(&path, "textDocument/selectionRange", params) else { return false };
            self.intel.wants.push((request, Want::Grow { group: g, doc: tab.doc, version: doc.buf.version, sels: tab.sels.clone() }));
            return true;
        }
        let known = matches!(
            cmd,
            GoToDefinition | PeekDefinition | GoToDeclaration | GoToTypeDefinition | GoToImplementation | FindReferences
                | ShowCallHierarchy | ShowTypeHierarchy | ShowHover | TriggerSuggest | TriggerParameterHints | RenameSymbol
                | QuickFix | OrganizeImports | FormatDocument | FormatSelection
        );
        let Some((g, t)) = self.active_tab().filter(|_| known) else { return false };
        let tab = &self.groups[g].tabs[t];
        let (id, sel) = (tab.doc, tab.sels.first().copied().unwrap_or_default());
        let Some(doc) = self.docs.get_mut(&id) else { return false };
        sync(&mut self.lsp, &doc.buf, &mut doc.lang, ctx);
        let buf = &doc.buf;
        if *cmd == TriggerSuggest && !doc.lang.attached && md_suggest(&mut self.intel.suggest, self.root.as_deref(), id, buf, sel.head, None) {
            return true;
        }
        let Some(path) = buf.path.clone().filter(|_| doc.lang.attached) else {
            self.banner = notice("No language server is running for this file. Servers are listed under Languages in Settings.".into());
            return true;
        };
        let head = sel.head.min(buf.len());
        let here = lsp::at(&path, to_lsp(buf, head));
        let document = json!({"uri": lsp::uri(&path)});
        let locations = |title, peek| Want::Locations { doc: id, pos: head, title, peek };
        let (method, params, want) = match cmd {
            GoToDefinition => ("textDocument/definition", here, locations("Definitions", false)),
            PeekDefinition => ("textDocument/definition", here, locations("Definitions", true)),
            GoToDeclaration => ("textDocument/declaration", here, locations("Declarations", false)),
            GoToTypeDefinition => ("textDocument/typeDefinition", here, locations("Type Definitions", false)),
            GoToImplementation => ("textDocument/implementation", here, locations("Implementations", false)),
            FindReferences => {
                let mut params = here;
                params["context"] = json!({"includeDeclaration": true});
                ("textDocument/references", params, locations("References", true))
            }
            ShowCallHierarchy => ("textDocument/prepareCallHierarchy", here, Want::Prepare { doc: id, pos: head, call: true, outgoing: false }),
            ShowTypeHierarchy => ("textDocument/prepareTypeHierarchy", here, Want::Prepare { doc: id, pos: head, call: false, outgoing: false }),
            TriggerSuggest => {
                let mut params = here;
                params["context"] = json!({"triggerKind": 1});
                ("textDocument/completion", params, Want::Completion { doc: id, start: buf.word_at(head).0.min(head), dot: true })
            }
            TriggerParameterHints => ("textDocument/signatureHelp", here, Want::Signature { doc: id }),
            ShowHover => {
                let word = buf.word_at(head);
                let blocks = problem_blocks(&self.lsp, &path, to_lsp(buf, word.0), to_lsp(buf, word.1));
                self.intel.hover = Some(Hover { group: g, doc: id, word, since: 0.0, asked: true, pinned: true, blocks, drawn: 0 });
                ("textDocument/hover", here, Want::Hover { doc: id, word })
            }
            RenameSymbol => {
                let (from, to) = buf.word_at(head);
                self.intel.rename = Some(Rename { doc: id, pos: head, text: buf.text_of(from, to), focus: true });
                return true;
            }
            QuickFix | OrganizeImports => {
                let organize = *cmd == OrganizeImports;
                let (from, to) = if organize { (0, buf.len()) } else { (sel.start(), sel.end()) };
                let (start, end) = (to_lsp(buf, from), to_lsp(buf, to));
                let touching: Vec<Value> =
                    self.lsp.diagnostics(&path).iter().filter(|d| d.start <= end && d.end >= start).map(|d| d.raw.clone()).collect();
                let mut context = json!({"diagnostics": touching, "triggerKind": 1});
                if organize {
                    context["only"] = json!(["source.organizeImports"]);
                }
                let params = json!({"textDocument": document, "range": lsp::range_json(start, end), "context": context});
                ("textDocument/codeAction", params, Want::Actions { doc: id, apply: organize })
            }
            _ => {
                let whole = *cmd == FormatDocument || sel.is_empty();
                let options = json!({"tabSize": settings.tab_size, "insertSpaces": settings.insert_spaces});
                let mut params = json!({"textDocument": document, "options": options});
                if !whole {
                    params["range"] = lsp::range_json(to_lsp(buf, sel.start()), to_lsp(buf, sel.end()));
                }
                let method = if whole { "textDocument/formatting" } else { "textDocument/rangeFormatting" };
                (method, params, Want::Format { doc: id, version: buf.version, save: false })
            }
        };
        match self.lsp.request(&path, method, params) {
            Some(request) => self.intel.wants.push((request, want)),
            None => self.banner = notice(format!("The language server does not support {}.", cmd.label())),
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn typed(&mut self, c: char, id: u64, buf: &Document, lang: &mut Lang, tab: &Tab, prose: bool, ctx: &egui::Context) {
        self.intel.hover = None;
        let head = tab.sels[0].head;
        if !lang.attached && md_suggest(&mut self.intel.suggest, self.root.as_deref(), id, buf, head, Some(c)) {
            return;
        }
        let Some(path) = buf.path.clone().filter(|_| lang.attached) else {
            let start = buf.word_at(head).0.min(head);
            if !is_word(c) || prose || tab.snippet.is_some() {
                self.intel.suggest = None;
            } else if !self.intel.suggest.as_ref().is_some_and(|p| p.doc == id && p.start == start) {
                let items = local_items(buf, true);
                self.intel.suggest = Some(Suggest { doc: id, start, items, shown: vec![], selected: 0, prefix: None, incomplete: false, dot: false, reveal: true });
            }
            return;
        };
        let word = is_word(c);
        if word || self.lsp.triggers(&path, "completionProvider").contains(&c) {
            let start = if word { buf.word_at(head).0.min(head) } else { head };
            let open = self.intel.suggest.as_ref().is_some_and(|p| p.doc == id && p.start == start && !p.incomplete);
            let waiting = self.intel.wants.iter().any(|(_, w)| matches!(w, Want::Completion { doc, start: s, .. } if *doc == id && *s == start));
            if !(word && (open || waiting)) {
                sync(&mut self.lsp, buf, lang, ctx);
                let mut params = lsp::at(&path, to_lsp(buf, head));
                params["context"] = if word { json!({"triggerKind": 1}) } else { json!({"triggerKind": 2, "triggerCharacter": c.to_string()}) };
                if let Some(request) = self.lsp.request(&path, "textDocument/completion", params) {
                    self.intel.wants.push((request, Want::Completion { doc: id, start, dot: !word }));
                }
                if !word {
                    self.intel.suggest = None;
                }
            }
        } else {
            self.intel.suggest = None;
        }
        if self.intel.signature.is_some() || self.lsp.triggers(&path, "signatureHelpProvider").contains(&c) {
            sync(&mut self.lsp, buf, lang, ctx);
            if let Some(request) = self.lsp.request(&path, "textDocument/signatureHelp", lsp::at(&path, to_lsp(buf, head))) {
                self.intel.wants.push((request, Want::Signature { doc: id }));
            }
        }
    }

    pub(super) fn popup_keys(&mut self, events: &mut Vec<Event>, id: u64, buf: &mut Document, tab: &mut Tab) {
        let mut accept = false;
        events.retain(|event| {
            let Event::Key { key, pressed: true, modifiers, .. } = event else { return true };
            let step = match key {
                Key::ArrowDown => 1,
                Key::ArrowUp => -1,
                Key::Enter | Key::Tab | Key::Escape => 0,
                _ => return true,
            };
            if modifiers.any() {
                return true;
            }
            let intel = &mut self.intel;
            let moved = |selected: &mut usize, count: usize| *selected = (*selected as isize + step).rem_euclid(count as isize) as usize;
            if let Some(popup) = intel.suggest.as_mut().filter(|p| p.doc == id && !p.shown.is_empty()) {
                match key {
                    Key::Escape => intel.suggest = None,
                    Key::Enter | Key::Tab => accept = true,
                    _ => {
                        moved(&mut popup.selected, popup.shown.len());
                        popup.reveal = true;
                    }
                }
            } else if let Some(popup) = intel.actions.as_mut().filter(|p| p.doc == id && *key != Key::Tab) {
                match key {
                    Key::Escape => {}
                    Key::Enter => intel.queued.extend(popup.items.get(popup.selected).map(|item| Queued::Action(id, item.1.clone()))),
                    _ => {
                        moved(&mut popup.selected, popup.items.len().max(1));
                        popup.reveal = true;
                        return false;
                    }
                }
                intel.actions = None;
            } else if *key == Key::Escape && (intel.signature.is_some() || intel.hover.is_some()) {
                intel.signature = None;
                intel.hover = None;
            } else if let Some(peek) = intel.peek.as_mut().filter(|p| p.doc == id && p.group == self.active && *key != Key::Tab) {
                match key {
                    Key::Escape => {}
                    Key::Enter => intel.queued.extend(peek.items.get(peek.selected).map(|item| Queued::Jump(item.path.clone(), item.start))),
                    _ => {
                        moved(&mut peek.selected, peek.items.len().max(1));
                        (peek.reveal, peek.scroll) = (true, 0);
                        return false;
                    }
                }
                intel.peek = None;
            } else {
                return true;
            }
            false
        });
        if accept {
            self.accept_suggestion(buf, tab);
        }
    }

    fn accept_suggestion(&mut self, buf: &mut Document, tab: &mut Tab) {
        let Some(popup) = self.intel.suggest.take() else { return };
        let Some(item) = popup.shown.get(popup.selected).map(|index| &popup.items[*index]) else { return };
        let head = tab.sels[0].head;
        let from = item.range.map_or(popup.start, |span| from_lsp(buf, span.0)).min(head);
        let (text, stops) = match item.snippet {
            true => snippet::parse(&snippet::indented(buf, from, &item.text, &self.intel.unit)),
            false => (item.text.clone(), vec![]),
        };
        let length = text.chars().count();
        let mut edits = vec![(from, head, text)];
        edits.extend(item.extra.iter().map(|e| (from_lsp(buf, e.start), from_lsp(buf, e.end), e.text.clone())));
        let before = edits.iter().skip(1).filter(|e| e.0 < from).count();
        let mut sels = tab.sels.clone();
        if buf.apply(&mut sels, edits, Kind::Other, After::Carets) {
            tab.sels = vec![sels.get(before).copied().unwrap_or(sels[0])];
            buf.fix_after(&tab.sels);
            if item.snippet {
                let start = tab.sels[0].head - length;
                tab.snippet = snippet::Session::begin(buf, &mut tab.sels, start, stops);
            }
            tab.reveal = Some(tab.sels[0].head);
            if item.kind == 19 && item.text.ends_with('/') {
                md_suggest(&mut self.intel.suggest, self.root.as_deref(), popup.doc, buf, tab.sels[0].head, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn track_hover(&mut self, over: Option<(usize, usize)>, g: usize, id: u64, buf: &Document, lang: &mut Lang, now: f64, ctx: &egui::Context) {
        let on_popup = self.intel.hover_rect.is_some_and(|r| ctx.pointer_latest_pos().is_some_and(|p| r.expand(4.0).contains(p)));
        if !on_popup && ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::MouseWheel { .. }))) {
            self.intel.hover = None;
        }
        match self.intel.hover.as_ref().map(|h| ((h.group, h.doc) == (g, id), h.word, h.pinned)) {
            Some((false, ..)) if over.is_none() => return,
            Some((true, word, pinned)) if Some(word) == over || on_popup || (pinned && over.is_none()) => {}
            None if over.is_none() => return,
            _ => {
                self.intel.hover = over.map(|word| Hover { group: g, doc: id, word, since: now, asked: false, pinned: false, blocks: vec![], drawn: 0 });
                self.intel.hover_rect = None;
                ctx.request_repaint_after(Duration::from_millis(450));
            }
        }
        let Some(hover) = self.intel.hover.as_mut().filter(|h| (h.group, h.doc) == (g, id) && !h.asked && now - h.since >= 0.4) else { return };
        hover.asked = true;
        let Some(path) = buf.path.clone() else { return };
        let (start, end) = (to_lsp(buf, hover.word.0), to_lsp(buf, hover.word.1));
        hover.blocks = problem_blocks(&self.lsp, &path, start, end);
        if !lang.attached {
            return;
        }
        let word = hover.word;
        sync(&mut self.lsp, buf, lang, ctx);
        if let Some(request) = self.lsp.request(&path, "textDocument/hover", lsp::at(&path, start)) {
            self.intel.wants.push((request, Want::Hover { doc: id, word }));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn hover_ui(&mut self, ctx: &egui::Context, theme: &Theme, font: &FontId, g: usize, id: u64, at: Option<Pos2>, line_h: f32) {
        let Some(hover) = self.intel.hover.as_mut().filter(|h| (h.group, h.doc) == (g, id) && !h.blocks.is_empty()) else { return };
        let Some(at) = at else { return };
        let (pivot, pos) = match at.y > ctx.content_rect().center().y {
            true => (Align2::LEFT_BOTTOM, at - vec2(0.0, 2.0)),
            false => (Align2::LEFT_TOP, at + vec2(0.0, line_h + 2.0)),
        };
        let changed = std::mem::replace(&mut hover.drawn, hover.blocks.len()) != hover.blocks.len();
        if changed {
            ctx.request_discard("hover content changed");
        }
        let hover = &*hover;
        let area = Area::new(Id::new("hover_info")).order(Order::Foreground).pivot(pivot).fixed_pos(pos).sizing_pass(changed).show(ctx, |ui| {
            popup_frame(theme).inner_margin(8.0).show(ui, |ui| {
                ui.set_max_width(580.0);
                ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for (kind, text) in &hover.blocks {
                        let (shown, found) = if *kind == 0 { hover_links(text) } else { Default::default() };
                        let rich = match kind {
                            0 if !found.is_empty() => {
                                link_text(ui, theme, &shown, &found);
                                ui.add_space(4.0);
                                continue;
                            }
                            0 => RichText::new(text).color(theme.text),
                            1 => RichText::new(text).font(font.clone()).color(theme.text),
                            severity => RichText::new(text).color(severity_color(theme, severity - 1)),
                        };
                        ui.add(egui::Label::new(rich).wrap());
                        ui.add_space(4.0);
                    }
                });
            });
        });
        self.intel.hover_rect = Some(area.response.rect);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn popups(&mut self, ui: &mut Ui, theme: &Theme, id: u64, buf: &mut Document, tab: &mut Tab, caret: Option<Pos2>, line_h: f32, font: &FontId) {
        let ctx = ui.ctx().clone();
        let head = tab.sels[0].head;
        let mut keep = false;
        if let Some(popup) = self.intel.suggest.as_mut().filter(|p| p.doc == id) {
            let valid = tab.sels.len() == 1 && tab.sels[0].is_empty() && head >= popup.start && buf.line_of(popup.start) == buf.line_of(head);
            let prefix = if valid { buf.text_of(popup.start, head) } else { String::new() };
            let loose = popup.items.first().is_some_and(|item| matches!(item.kind, 17..=19));
            let fits = |c: char| is_word(c) || (loose && !c.is_whitespace() && !"/#()<>\"'`".contains(c));
            if valid && prefix.chars().all(fits) && (popup.dot || !prefix.is_empty()) {
                if popup.prefix.as_deref() != Some(prefix.as_str()) {
                    let lower = prefix.to_lowercase();
                    let rank = |item: &Completion| if prefix.is_empty() { Some(0) } else { crate::search::score(&prefix, &item.filter) };
                    let mut ranked: Vec<(i32, usize)> = popup.items.iter().enumerate().filter_map(|(index, item)| Some((rank(item)?, index))).collect();
                    ranked.sort_by_key(|(score, index)| {
                        let item = &popup.items[*index];
                        (!item.filter.to_lowercase().starts_with(&lower), -score, item.sort.clone())
                    });
                    let shown: Vec<usize> = ranked.into_iter().take(300).map(|found| found.1).collect();
                    (popup.shown, popup.selected, popup.reveal, popup.prefix) = (shown, 0, true, Some(prefix));
                }
                keep = !popup.shown.is_empty();
            }
        }
        if !keep {
            self.intel.suggest = None;
        }
        let _ = self.intel.signature.take_if(|s| s.0 != id);
        let _ = self.intel.actions.take_if(|a| a.doc != id);
        let below = caret.map(|at| at + vec2(0.0, line_h + 2.0));

        let mut accept = false;
        if let (Some(popup), Some(at)) = (self.intel.suggest.as_mut(), below) {
            let reveal = std::mem::take(&mut popup.reveal);
            let rows: Vec<(&str, &str, &str)> = popup
                .shown
                .iter()
                .map(|index| &popup.items[*index])
                .map(|item| (lsp::completion_icon(item.kind), item.label.as_str(), item.detail.as_str()))
                .collect();
            let (clicked, rect) = list_popup(&ctx, theme, "suggest", (at, line_h), &rows, (popup.selected, self.config.suggest_rows.clamp(1, 20)), reveal, font);
            if let Some(index) = clicked {
                popup.selected = index;
                accept = true;
            }
            self.intel.rects.push(rect);
        }
        if accept {
            self.accept_suggestion(buf, tab);
            self.want_focus = true;
        }

        if let (Some(popup), Some(at)) = (self.intel.actions.as_mut(), below) {
            let reveal = std::mem::take(&mut popup.reveal);
            let rows: Vec<(&str, &str, &str)> = popup.items.iter().map(|item| (icons::LIGHTBULB, item.0.as_str(), "")).collect();
            let (clicked, rect) = list_popup(&ctx, theme, "code_actions", (at, line_h), &rows, (popup.selected, 10), reveal, &FontId::proportional(12.5));
            let chosen = clicked.and_then(|index| popup.items.get(index)).map(|item| item.1.clone());
            self.intel.rects.push(rect);
            if let Some(action) = chosen {
                self.intel.queued.push(Queued::Action(id, action));
                self.intel.actions = None;
                self.want_focus = true;
            }
        }

        if let (Some((_, sig)), Some(at)) = (&self.intel.signature, caret) {
            let mut job = LayoutJob::default();
            let plain = TextFormat::simple(font.clone(), theme.text);
            let label = sig.label.as_str();
            match sig.param.filter(|(a, b)| a <= b && label.is_char_boundary(*a) && label.is_char_boundary(*b) && *b <= label.len()) {
                Some((a, b)) => {
                    job.append(&label[..a], 0.0, plain.clone());
                    job.append(&label[a..b], 0.0, TextFormat::simple(font.clone(), theme.accent));
                    job.append(&label[b..], 0.0, plain);
                }
                None => job.append(label, 0.0, plain),
            }
            let note: String = sig.doc.lines().take(3).collect::<Vec<_>>().join("\n");
            let area = Area::new(Id::new("signature_help")).order(Order::Foreground).pivot(Align2::LEFT_BOTTOM).fixed_pos(at - vec2(0.0, 4.0)).show(&ctx, |ui| {
                popup_frame(theme).inner_margin(8.0).show(ui, |ui| {
                    ui.set_max_width(580.0);
                    ui.label(job);
                    if !note.trim().is_empty() {
                        ui.add(egui::Label::new(RichText::new(note).color(theme.text_dim)).wrap());
                    }
                });
            });
            self.intel.rects.push(area.response.rect);
        }

        if let (Some(rename), Some(at)) = (self.intel.rename.as_mut().filter(|r| r.doc == id), below) {
            let (mut done, mut focused) = (None, false);
            Area::new(Id::new("rename_symbol")).order(Order::Foreground).fixed_pos(at).show(&ctx, |ui| {
                popup_frame(theme).inner_margin(6.0).show(ui, |ui| {
                    let field = ui.add(TextEdit::singleline(&mut rename.text).margin(ui::FIELD).desired_width(240.0).font(font.clone()));
                    if std::mem::take(&mut rename.focus) {
                        field.request_focus();
                    }
                    focused = field.has_focus();
                    if field.lost_focus() {
                        done = Some(ui.input(|i| i.key_pressed(Key::Enter)));
                    }
                    ui.label(RichText::new("Enter to rename, Escape to cancel").size(11.0).color(theme.text_dim));
                });
            });
            self.focused |= focused;
            if let Some(submit) = done {
                let (pos, name) = (rename.pos, rename.text.trim().to_owned());
                self.intel.rename = None;
                self.want_focus = true;
                if let Some(path) = buf.path.clone().filter(|_| submit && !name.is_empty()) {
                    let mut params = lsp::at(&path, to_lsp(buf, pos));
                    params["newName"] = json!(name);
                    match self.lsp.request(&path, "textDocument/rename", params) {
                        Some(request) => self.intel.wants.push((request, Want::Rename)),
                        None => self.banner = notice("The language server does not support Rename Symbol.".into()),
                    }
                }
            }
        }
    }

    pub(super) fn peek_ui(&mut self, ui: &mut Ui, theme: &Theme, (font, style): (&FontId, super::typography::FontStyle), zone: Rect, id: u64) {
        let context = self.config.peek_context_lines.max(1);
        let Some(peek) = self.intel.peek.as_mut().filter(|p| p.doc == id) else { return };
        let row_h = (font.size * 1.35).round();
        let rect = Rect::from_min_max(zone.min + vec2(6.0, 2.0), zone.max - vec2(8.0, 2.0));
        ui.interact(rect, Id::new(("peek", id)), Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect(rect, CornerRadius::same(3), theme.panel, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0)).shrink(1.0);
        painter.rect_filled(header, CornerRadius::ZERO, ui::mix(theme.panel, theme.accent, 0.14));
        let (mut close, mut flip, mut jump, mut select) = (false, false, None, None);
        let current = peek.items.get(peek.selected).cloned();
        let name = current.as_ref().map(|item| file_name(&item.path)).unwrap_or_default();
        let folder = current.as_ref().and_then(|item| item.path.parent()).map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
        ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            close = ui::icon_button(ui, theme, icons::X, 22.0, false, "Close (Escape)").clicked();
            if let Some((call, outgoing)) = peek.hierarchy {
                let tip = format!("Show {}", hierarchy_title(call, !outgoing));
                flip = ui::icon_button(ui, theme, icons::ARROWS_LEFT_RIGHT, 22.0, false, &tip).clicked();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(name).color(theme.text));
                let detail = format!("{folder}  -  {} ({})", peek.title, peek.items.len());
                ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(theme.text_dim)).truncate());
            });
        });
        let body = Rect::from_min_max(pos2(rect.left() + 1.0, header.bottom()), rect.max - vec2(1.0, 1.0));
        let split = body.left() + body.width() * 0.62;
        let (preview, list) = (body.with_max_x(split), body.with_min_x(split + 1.0));
        painter.vline(split, body.y_range(), Stroke::new(1.0, theme.border));
        if let Some(item) = &current {
            let lines = peek.files.get(&lsp::key(&item.path));
            let shown = (preview.height() / row_h).ceil() as usize;
            let opened = ui.interact(preview, Id::new(("peek_preview", id)), Sense::click()).on_hover_text("Double-click to open");
            if opened.hovered() {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
                peek.scroll -= (wheel / row_h).round() as isize;
            }
            let total = lines.map_or(0, Vec::len);
            let natural = item.start.0 as isize - context.min(shown / 2) as isize;
            peek.scroll = peek.scroll.clamp(-natural.max(0), (total as isize - natural - 1).max(0));
            let first = (natural + peek.scroll).max(0) as usize;
            let clip = painter.with_clip_rect(preview);
            for (index, line) in (first..first + shown).enumerate() {
                let Some(text) = lines.and_then(|lines| lines.get(line)) else { break };
                let y = preview.top() + index as f32 * row_h;
                if line == item.start.0 {
                    let band = Rect::from_min_size(pos2(preview.left(), y), vec2(preview.width(), row_h));
                    clip.rect_filled(band, CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.16));
                }
                let shown_text: String = text.chars().take(300).collect::<String>().replace('\t', "    ");
                clip.text(pos2(preview.left() + 44.0, y + row_h / 2.0), Align2::RIGHT_CENTER, line + 1, font.clone(), theme.text_dim);
                let galley = clip.layout_job(style.apply(LayoutJob::simple_singleline(shown_text, font.clone(), theme.text)));
                clip.galley(pos2(preview.left() + 56.0, y + (row_h - galley.size().y) / 2.0), galley, theme.text);
            }
            if opened.double_clicked() {
                jump = Some(peek.selected);
            }
        }
        let reveal = std::mem::take(&mut peek.reveal);
        ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ScrollArea::vertical().id_salt(("peek_list", id)).auto_shrink(false).show(ui, |ui| {
                if peek.items.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("   No results").color(theme.text_dim));
                }
                for (index, item) in peek.items.iter().enumerate() {
                    let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                    ui::row_bg(ui, theme, r, row.id, row.hovered(), index == peek.selected);
                    ui::list_item(ui, &row, &item.label, index == peek.selected, (index, peek.items.len()));
                    let p = ui.painter().with_clip_rect(r.shrink2(vec2(6.0, 0.0)));
                    let end = p.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &item.label, FontId::proportional(12.5), theme.text);
                    let place = format!("{}:{}", file_name(&item.path), item.start.0 + 1);
                    p.text(pos2(end.right() + 10.0, r.center().y), Align2::LEFT_CENTER, place, FontId::proportional(11.5), theme.text_dim);
                    if index == peek.selected && reveal {
                        ui.scroll_to_rect(r, None);
                    }
                    if row.double_clicked() {
                        jump = Some(index);
                    } else if row.clicked() {
                        select = Some(index);
                    }
                }
            });
        });
        if let Some(index) = select {
            peek.selected = index;
            peek.scroll = 0;
        }
        let (pos, hierarchy) = (peek.pos, peek.hierarchy);
        let target = jump.and_then(|index| peek.items.get(index)).map(|item| (item.path.clone(), item.start));
        if let Some((path, at)) = target {
            self.intel.queued.push(Queued::Jump(path, at));
        } else if let Some((call, outgoing)) = hierarchy.filter(|_| flip) {
            self.intel.queued.push(Queued::Hierarchy(id, pos, call, !outgoing));
        }
        if close || jump.is_some() {
            self.intel.peek = None;
            self.want_focus = true;
        }
    }

    pub(super) fn breadcrumbs(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme, g: usize) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        let group = &self.groups[g];
        let Some(tab) = group.tabs.get(group.active.min(group.tabs.len().saturating_sub(1))) else { return };
        let Some(doc) = self.docs.get(&tab.doc) else { return };
        let mut parts: Vec<(&str, String, Option<Pos>)> = match &doc.buf.path {
            Some(path) => {
                let workspace = crate::workspace::current();
                match workspace.relative(path).filter(|_| workspace.folders.len() > 1) {
                    Some(relative) => relative.split('/').map(|part| ("", part.to_owned(), None)).collect(),
                    None => {
                        let shown = self.root.as_deref().and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
                        shown.components().map(|part| ("", part.as_os_str().to_string_lossy().into_owned(), None)).collect()
                    }
                }
            }
            None => vec![("", doc.buf.title.clone(), None)],
        };
        let path_parts = if doc.buf.path.is_some() { parts.len() } else { 0 };
        let line = doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head));
        let mut chain: Vec<&Symbol> = vec![];
        for symbol in doc.lang.symbols.iter().filter(|s| s.line <= line && line <= s.end_line) {
            chain.truncate(symbol.depth);
            chain.push(symbol);
        }
        parts.extend(chain.iter().map(|s| (lsp::symbol_icon(s.kind), s.name.clone(), Some((s.line, s.col)))));
        let mut clicked = None;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing.x = 4.0;
            for (index, (icon, name, target)) in parts.iter().enumerate() {
                if index > 0 {
                    ui.label(RichText::new(icons::CARET_RIGHT).size(10.0).color(theme.text_dim));
                }
                if index < path_parts && crate::icon_theme::active() {
                    let kind = if index + 1 == path_parts { crate::icon_theme::Kind::File } else { crate::icon_theme::Kind::Folder };
                    let (slot, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                    crate::icon_theme::paint(ui.painter(), slot.center(), 13.0, name, kind, 1.0);
                }
                let text = if icon.is_empty() { name.clone() } else { format!("{icon} {name}") };
                let label = egui::Label::new(RichText::new(text).size(12.0).color(theme.text_dim));
                let Some(at) = target else {
                    ui.add(label);
                    continue;
                };
                if ui.add(label.sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    clicked = Some(*at);
                }
            }
        });
        if let Some(at) = clicked {
            let pos = from_lsp(&doc.buf, at);
            let group = &mut self.groups[g];
            let active = group.active.min(group.tabs.len().saturating_sub(1));
            group.tabs[active].sels = vec![Sel::caret(pos)];
            group.tabs[active].reveal = Some(pos);
            self.active = g;
            self.want_focus = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_hint_columns_and_shifts_server_data() {
        let hints = [(2, 3), (5, 2)];
        assert_eq!([0, 2, 3, 5, 6].map(|col| display(&hints, col)), [0, 2, 6, 8, 11]);
        assert_eq!([0, 2, 3, 4, 5, 6, 8, 9, 11].map(|shown| undisplay(&hints, shown)), [0, 2, 2, 2, 2, 3, 5, 5, 6]);
        assert!((0..9).all(|col| undisplay(&hints, display(&hints, col)) == col));

        let symbol = |line, end_line| Symbol { name: "s".into(), kind: 12, line, col: 0, end_line, depth: 0, container: String::new(), path: None };
        let hint = |line| Hint { line, col: 1, label: ": u8".into() };
        let mut lang = Lang { lines: 10, symbols: vec![symbol(1, 2), symbol(6, 8)], hints: vec![hint(1), hint(4), hint(6)], ..Default::default() };
        lang.tokens.extend([(1, vec![(0, 2, 1)]), (4, vec![(0, 2, 1)]), (7, vec![(0, 2, 2)])]);
        assert_eq!(lang.edited(4, 12), 2);
        assert_eq!(lang.symbols.iter().map(|s| (s.line, s.end_line)).collect::<Vec<_>>(), vec![(1, 2), (8, 10)]);
        assert_eq!(lang.hints.iter().map(|h| h.line).collect::<Vec<_>>(), vec![1, 8]);
        let mut lines: Vec<usize> = lang.tokens.keys().copied().collect();
        lines.sort();
        assert_eq!(lines, vec![1, 9]);
        assert_eq!(lang.edited(0, 9), -3);
        assert_eq!(lang.symbols.iter().map(|s| s.line).collect::<Vec<_>>(), vec![0, 5]);

        let buf = Document::from_text("a😀b\nsecond", "t".into());
        assert_eq!((to_lsp(&buf, 2), from_lsp(&buf, (0, 3)), from_lsp(&buf, (1, 99)), from_lsp(&buf, (9, 0))), ((0, 3), 2, 10, 10));
    }

    #[test]
    fn extracts_links_from_hover_text() {
        let (text, found) = hover_links("See [the dócs](https://a.io/x) or https://b.io. [x] [y](file:///z) [https://c.io](https://c.io)");
        assert_eq!(text, "See the dócs or https://b.io. [x] [y](file:///z) https://c.io");
        let spans: Vec<(String, &str)> = found.iter().map(|(from, to, url)| (text.chars().skip(*from).take(to - from).collect(), url.as_str())).collect();
        assert_eq!(spans, vec![("the dócs".into(), "https://a.io/x"), ("https://b.io".into(), "https://b.io"), ("https://c.io".into(), "https://c.io")]);
        assert!(hover_links("plain [text]").1.is_empty());
    }

    #[test]
    fn finds_the_problem_under_a_position() {
        let problem = |start, end, severity| lsp::Diagnostic { start, end, severity, message: String::new(), source: String::new(), raw: Value::Null };
        let problems = [problem((0, 0), (0, 0), 1), problem((1, 4), (2, 2), 2), problem((3, 0), (3, 9), 4)];
        assert!([(0, 0), (1, 4), (1, 30), (2, 1)].into_iter().all(|at| problem_at(&problems, at)));
        assert!(![(0, 1), (1, 3), (2, 2), (3, 2)].into_iter().any(|at| problem_at(&problems, at)));
    }

    fn run(ctx: &egui::Context, editors: &mut Editors, settings: &Settings, events: Vec<Event>, t: f64) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))), events, focused: true, time: Some(t), ..Default::default() };
        ctx.run_ui(input, |ui| editors.ui(ui, &Theme::dark(), settings, &crate::git::Git::new(), &mut vec![])).textures_delta.clear();
    }

    fn editor_with_lines(ctx: &egui::Context, settings: &Settings) -> Editors {
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, settings, ctx);
        run(ctx, &mut editors, settings, vec![], 1.0);
        run(ctx, &mut editors, settings, vec![], 1.0);
        let body: String = (0..60).map(|i| format!("int unused_variable_{i} = 1;\n")).collect();
        run(ctx, &mut editors, settings, vec![Event::Paste(body)], 1.0);
        editors
    }

    #[test]
    fn hover_stays_up_while_diagnostic_and_server_parts_arrive() {
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = editor_with_lines(&ctx, &settings);
        let mut t = 2.0;
        for y in [70.0, 330.0, 345.0] {
            let pointer = pos2(150.0, y);
            editors.intel.hover = None;
            let mut since = None;
            for frame in 0..80 {
                t += 0.05;
                run(&ctx, &mut editors, &settings, if frame == 0 { vec![Event::PointerMoved(pointer)] } else { vec![] }, t);
                let Some(hover) = editors.intel.hover.as_mut() else { panic!("the hover closed at frame {frame} for y {y}") };
                if hover.asked && hover.blocks.is_empty() {
                    hover.blocks = vec![(3, "unused variable 'x'  clang(-Wunused-variable)".into())];
                }
                if frame == 40 {
                    hover.blocks.extend([(1, "int unused_variable_1 = 1".into()), (0, "Type: int\nmore\nlines".into())]);
                }
                if hover.asked {
                    assert_eq!(*since.get_or_insert(hover.since), hover.since, "the hover was rebuilt and asked again");
                }
            }
            let rect = editors.intel.hover_rect.unwrap();
            assert!(rect.height() > 60.0 && !rect.contains(pointer), "{rect:?} {pointer:?}");
        }
        run(&ctx, &mut editors, &settings, vec![Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, -1.0), phase: egui::TouchPhase::Move, modifiers: Default::default() }], t + 0.05);
        assert!(editors.intel.hover.as_ref().is_none_or(|h| !h.asked));
    }

    #[test]
    fn keeps_only_fresh_symbol_highlights() {
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = editor_with_lines(&ctx, &settings);
        let group = &mut editors.groups[editors.active];
        let tab = &mut group.tabs[group.active];
        tab.sels = vec![Sel::caret(4)];
        let id = tab.doc;
        let d = editors.docs.get_mut(&id).unwrap();
        assert!(on_word(&d.buf, 4) && on_word(&d.buf, 3) && !on_word(&d.buf, 22));
        let version = d.buf.version;
        d.lang.marked = Some((version, 4));
        let reply = json!([{"range": lsp::range_json((0, 4), (0, 21)), "kind": 3}, {"range": lsp::range_json((2, 4), (2, 21))}]);
        editors.reply(Want::Marks { doc: id, version, head: 5 }, Some(reply.clone()), 0.0);
        editors.reply(Want::Marks { doc: id, version: version + 1, head: 4 }, Some(reply.clone()), 0.0);
        assert!(editors.docs[&id].lang.marks.is_empty());
        editors.reply(Want::Marks { doc: id, version, head: 4 }, Some(reply), 0.0);
        let line2 = editors.docs[&id].buf.line_start(2);
        assert_eq!(editors.docs[&id].lang.marks, vec![(4, 21), (line2 + 4, line2 + 21)]);
        run(&ctx, &mut editors, &settings, vec![], 3.0);
        assert_eq!(editors.docs[&id].lang.marks.len(), 2);
    }

    #[test]
    fn grows_selections_from_the_server_chain_while_they_stand_still() {
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = editor_with_lines(&ctx, &settings);
        let group = editors.active;
        let at = editors.groups[group].active;
        let tab = &mut editors.groups[group].tabs[at];
        tab.sels = vec![Sel::caret(6)];
        let (id, caret) = (tab.doc, tab.sels.clone());
        let version = editors.docs[&id].buf.version;
        let chain = json!([{"range": lsp::range_json((0, 6), (0, 6)), "parent": {"range": lsp::range_json((0, 4), (0, 21)), "parent": {"range": lsp::range_json((0, 0), (0, 26))}}}]);
        let spans = |e: &Editors| e.active_tab().map(|(g, t)| e.groups[g].tabs[t].sels.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>()).unwrap();
        editors.reply(Want::Grow { group, doc: id, version, sels: caret.clone() }, Some(chain.clone()), 0.0);
        assert_eq!(spans(&editors), vec![(4, 21)]);
        editors.reply(Want::Grow { group, doc: id, version, sels: caret }, Some(chain), 0.0);
        assert_eq!(spans(&editors), vec![(4, 21)]);
        let word = vec![Sel::new(4, 21)];
        editors.reply(Want::Grow { group, doc: id, version, sels: word }, None, 0.0);
        assert!(spans(&editors)[0].1 - spans(&editors)[0].0 > 17);
        editors.exec(&Command::ShrinkSelection, &settings, &ctx);
        editors.exec(&Command::ShrinkSelection, &settings, &ctx);
        assert_eq!(spans(&editors), vec![(6, 6)]);
    }

    #[test]
    fn edits_reach_the_language_server_in_the_frame_they_are_typed() {
        let (ctx, settings) = (egui::Context::default(), Settings { lsp: false, ..Settings::default() });
        let path = std::env::temp_dir().join(format!("halo-sync-test-{}.cpp", std::process::id()));
        std::fs::write(&path, "int x;\n").unwrap();
        let mut editors = Editors::new();
        editors.open(&path);
        editors.finish_loads();
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        let start = editors.docs.values().next().unwrap().buf.version;
        run(&ctx, &mut editors, &settings, vec![Event::Text("y".into())], 1.1);
        let doc = editors.docs.values().next().unwrap();
        assert!(doc.buf.version != start && doc.lang.synced == doc.buf.version);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn suggestion_list_shows_the_configured_number_of_rows() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        assert_eq!(settings.suggest_rows, 5);
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        let item = |i: usize| {
            let label = format!("item{i:02}");
            Completion { label: label.clone(), detail: String::new(), kind: 1, filter: label.clone(), sort: label.clone(), text: label, range: None, extra: vec![], snippet: false }
        };
        let height = |editors: &mut Editors, settings: &Settings, count: usize| {
            let doc = editors.groups[0].tabs[0].doc;
            let items = (0..count).map(item).collect();
            editors.intel.suggest = Some(Suggest { doc, start: 0, items, shown: vec![], selected: 0, prefix: None, incomplete: false, dot: true, reveal: true });
            run(&ctx, editors, settings, vec![], 3.0);
            run(&ctx, editors, settings, vec![], 3.0);
            editors.intel.rects[0].height()
        };
        let two = height(&mut editors, &settings, 2);
        let five = height(&mut editors, &settings, 30);
        assert_eq!(five - two, 3.0 * 22.0);
        settings.suggest_rows = 8;
        assert_eq!(height(&mut editors, &settings, 30) - five, 3.0 * 22.0);
        settings.suggest_rows = 99;
        assert_eq!(height(&mut editors, &settings, 30) - five, 15.0 * 22.0);
        let old: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(old.suggest_rows, 5);
    }

    fn accept_with_tab(extra: Vec<lsp::Edit>) -> (f32, f32, String, String, bool) {
        let (ctx, settings) = (egui::Context::default(), Settings { lsp: false, ..Settings::default() });
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        let body: String = (0..300).map(|i| format!("int unused_variable_{i} = 1;\n")).collect();
        run(&ctx, &mut editors, &settings, vec![Event::Paste(body)], 1.0);
        let mut t = 1.0;
        let mut frames = |editors: &mut Editors, events: Vec<Event>, count: usize| {
            for frame in 0..count {
                t += 0.016;
                run(&ctx, editors, &settings, if frame == 0 { events.clone() } else { vec![] }, t);
            }
        };
        frames(&mut editors, vec![], 60);
        editors.groups[0].tabs[0].set_scroll = Some(2000.0);
        frames(&mut editors, vec![], 3);
        let id = editors.groups[0].tabs[0].doc;
        let top = editors.docs[&id].top;
        let line = top + 10;
        let end = editors.docs[&id].buf.line_start(line + 1) - 1;
        editors.groups[0].tabs[0].sels = vec![Sel::caret(end)];
        frames(&mut editors, "bloc".chars().map(|c| Event::Text(c.into())).collect(), 30);
        let item = Completion { label: "block_size_".into(), detail: String::new(), kind: 5, filter: "block_size_".into(), sort: "a".into(), text: "block_size_".into(), range: None, extra, snippet: false };
        editors.intel.suggest = Some(Suggest { doc: id, start: end, items: vec![item], shown: vec![], selected: 0, prefix: None, incomplete: false, dot: false, reveal: true });
        frames(&mut editors, vec![], 3);
        let before = editors.groups[0].tabs[0].anchor.3;
        let doc = &editors.docs[&id];
        let first = doc.buf.text_of(doc.buf.line_start(doc.top), doc.buf.line_start(doc.top + 1));
        let tab = Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frames(&mut editors, vec![tab], 60);
        let doc = &editors.docs[&id];
        assert!(doc.buf.text_of(doc.buf.line_start(line), doc.buf.line_start(line + 2)).contains("= 1;block_size_\n"));
        let after = editors.groups[0].tabs[0].anchor.3;
        let shown = doc.buf.text_of(doc.buf.line_start(doc.top), doc.buf.line_start(doc.top + 1));
        let focused = ctx.memory(|m| m.has_focus(Id::new(("editor_view", 0usize, id))));
        (before, after, first, shown, focused)
    }

    #[test]
    fn accepting_a_completion_with_tab_keeps_the_view_and_focus() {
        let (before, after, first, shown, focused) = accept_with_tab(vec![]);
        assert!(before > 1000.0);
        assert_eq!((before, first.as_str(), focused), (after, shown.as_str(), true));
    }

    #[test]
    fn wheel_scrolling_settles_where_the_input_asked() {
        for (unit, step) in [(egui::MouseWheelUnit::Line, -1.0), (egui::MouseWheelUnit::Line, -0.3), (egui::MouseWheelUnit::Point, -30.0), (egui::MouseWheelUnit::Point, -4.0)] {
            for smooth in [true, false] {
                let (ctx, settings) = (egui::Context::default(), Settings { lsp: false, smooth_scrolling: smooth, ..Settings::default() });
                let mut editors = editor_with_lines(&ctx, &settings);
                let mut t = 1.0;
                for _ in 0..60 {
                    t += 0.016;
                    run(&ctx, &mut editors, &settings, vec![], t);
                }
                editors.groups[0].tabs[0].set_scroll = Some(0.0);
                run(&ctx, &mut editors, &settings, vec![Event::PointerMoved(pos2(400.0, 300.0))], t);
                let start = editors.groups[0].tabs[0].anchor.3;
                let per = if unit == egui::MouseWheelUnit::Line { ctx.options(|o| o.input_options.line_scroll_speed) } else { 1.0 };
                let mut wheel = vec![Event::MouseWheel { unit, delta: vec2(0.0, step), phase: egui::TouchPhase::Move, modifiers: Default::default() }];
                ui::direct_wheel(&mut wheel, per, smooth);
                let mut seen = vec![start];
                for frame in 0..90 {
                    t += 0.016;
                    run(&ctx, &mut editors, &settings, if frame < 6 { wheel.clone() } else { vec![] }, t);
                    seen.push(editors.groups[0].tabs[0].anchor.3);
                }
                let direct = !smooth || if unit == egui::MouseWheelUnit::Line { step.fract() != 0.0 } else { step.abs() < 8.0 };
                assert!(seen.windows(2).all(|w| w[1] >= w[0]), "{unit:?} {smooth} {seen:?}");
                assert!((*seen.last().unwrap() - start + 6.0 * step * per).abs() < 0.01, "{unit:?} {smooth} {seen:?}");
                assert!(seen[if direct { 6 } else { 60 }..].windows(2).all(|w| w[1] == w[0]), "{unit:?} {smooth} {seen:?}");
            }
        }
    }

    #[test]
    fn server_lenses_replace_own_counts_run_on_click_and_the_lightbulb_opens_actions() {
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        run(&ctx, &mut editors, &settings, vec![Event::Paste("fn a() {}\nfn b() {}\n".into())], 1.0);
        let id = editors.groups[0].tabs[0].doc;
        editors.groups[0].tabs[0].sels = vec![Sel::caret(0)];
        let place = Location { path: "x.rs".into(), start: (0, 0), end: (0, 1), label: String::new() };
        let runnable = json!({"kind": "cargo", "args": {"cargoArgs": ["test"], "executableArgs": ["t", "--exact"]}});
        let lens = |command: &str, title: &str, args: Value| {
            let raw = json!({"range": lsp::range_json((0, 0), (0, 2)), "command": {"title": title, "command": command, "arguments": args}});
            (lsp::code_lens(&raw), false)
        };
        let d = editors.docs.get_mut(&id).unwrap();
        let version = d.buf.version;
        let lang = &mut d.lang;
        lang.attached = true;
        lang.symbols = vec![Symbol { name: "a".into(), kind: 12, line: 0, col: 3, end_line: 0, depth: 0, container: String::new(), path: None }];
        lang.lens = vec![Lens { symbol: 0, refs: Some(vec![place]), impls: None, asked: true }];
        lang.code_lens = vec![lens("rust-analyzer.runSingle", "Run Test", json!([runnable])), lens("rust-analyzer.showReferences", "3 references", json!([null, {"line": 0, "character": 3}, []]))];
        lang.bulb_at = Some((version, 0));
        lang.bulb = Some((true, vec![("Fix it".into(), json!({"title": "Fix it", "kind": "quickfix"}))]));
        run(&ctx, &mut editors, &settings, vec![], 1.5);
        let mut t = 2.0;
        let mut click = |editors: &mut Editors, pos: Pos2| {
            for pressed in [None, Some(true), Some(false)] {
                t += 0.05;
                let events = match pressed {
                    None => vec![Event::PointerMoved(pos)],
                    Some(pressed) => vec![Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }],
                };
                run(&ctx, editors, &settings, events, t);
            }
        };
        let font = FontId::monospace(settings.font_size);
        let (char_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.row_height(&font)));
        let line_h = (row_h * 1.25).round();
        let top = super::super::TAB_H + 22.0;
        click(&mut editors, pos2(8.0 * char_w + 6.0 + 4.0, top + 0.5 * line_h));
        assert_eq!(editors.tests.run, Some(("cargo test -- t --exact".into(), None)), "the first lens is the server's Run, not Halo's own count");
        assert!(editors.intel.peek.is_none());

        assert!(editors.intel.actions.is_none());
        click(&mut editors, pos2(char_w * 1.1 + 2.0, top + 1.5 * line_h));
        assert_eq!(editors.intel.actions.as_ref().map(|a| (a.doc, a.items.len())), Some((id, 1)));
        editors.docs.get_mut(&id).unwrap().buf.version += 1;
        editors.intel.actions = None;
        click(&mut editors, pos2(char_w * 1.1 + 2.0, top + 1.5 * line_h));
        assert!(editors.intel.actions.is_none(), "the lightbulb is gone once the document changed");

        assert!(runs_lens("x.fix", &json!({"commands": ["x.fix"]})) && runs_lens("rust-analyzer.gotoLocation", &Value::Null));
        assert!(!runs_lens("x.other", &json!({"commands": ["x.fix"]})) && !runs_lens("", &Value::Null));
    }

    #[test]
    fn a_line_inserted_above_the_view_keeps_the_same_text_on_top() {
        let include = lsp::Edit { start: (0, 0), end: (0, 0), text: "#include <cstddef>\n".into() };
        let (before, after, first, shown, focused) = accept_with_tab(vec![include]);
        assert!(after > before && focused, "{before} {after}");
        assert_eq!(first, shown);
    }

    #[test]
    fn server_snippet_completions_land_on_tab_stops_with_mirrors() {
        let accept = |text: &str, start: usize, head: usize, item: Value| {
            let mut editors = Editors::new();
            editors.intel.unit = "    ".into();
            let mut buf = Document::from_text(text, "t".into());
            let mut tab = Tab { sels: vec![Sel::caret(head)], ..Default::default() };
            let items = lsp::completions(&json!([item])).0;
            editors.intel.suggest = Some(Suggest { doc: 0, start, items, shown: vec![0], selected: 0, prefix: None, incomplete: false, dot: false, reveal: true });
            editors.accept_suggestion(&mut buf, &mut tab);
            (buf, tab)
        };
        let span = |a: usize, b: usize| json!({"start": {"line": 1, "character": a}, "end": {"line": 1, "character": b}});
        let edit = json!({"insert": span(4, 5), "replace": span(4, 5), "newText": "fn ${1:name}($2) {\n\t$0\n}"});
        let top = json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "use a;\n"}]);
        let (mut buf, mut tab) = accept("mod m {\n    f\n}", 12, 13, json!({"label": "fn", "insertTextFormat": 2, "textEdit": edit, "additionalTextEdits": top}));
        assert_eq!(buf.rope.to_string(), "use a;\nmod m {\n    fn name() {\n        \n    }\n}");
        assert_eq!((tab.sels[0].start(), tab.sels[0].end(), tab.sels.len()), (22, 26, 1));
        buf.insert_text(&mut tab.sels, "g");
        buf.insert_text(&mut tab.sels, "o");
        snippet::track(&mut tab.snippet, buf.len(), &tab.sels);
        assert!(snippet::step(&mut tab.snippet, &mut tab.sels, true));
        assert_eq!(tab.sels[0].head, 25);
        assert!(snippet::step(&mut tab.snippet, &mut tab.sels, true));
        assert_eq!((tab.sels[0].head, tab.snippet.is_none()), (37, true));

        let (mut buf, mut tab) = accept("", 0, 0, json!({"label": "for", "insertTextFormat": 2, "insertText": "for (let ${1:i} = 0; $1 < ${2:n}; $1++) {}"}));
        assert_eq!(tab.sels.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>(), vec![(9, 10), (16, 17), (23, 24)]);
        buf.insert_text(&mut tab.sels, "k");
        buf.insert_text(&mut tab.sels, "j");
        snippet::track(&mut tab.snippet, buf.len(), &tab.sels);
        assert_eq!(buf.rope.to_string(), "for (let kj = 0; kj < n; kj++) {}");
        assert!(snippet::step(&mut tab.snippet, &mut tab.sels, true));
        assert_eq!(buf.rope.slice(tab.sels[0].start()..tab.sels[0].end()).to_string(), "n");

        let (buf, tab) = accept("v.p", 2, 3, json!({"label": "push", "insertTextFormat": 2, "insertText": "push($0)"}));
        assert_eq!((buf.rope.to_string().as_str(), tab.sels[0].head, tab.snippet.is_none()), ("v.push()", 7, true));
    }

    #[test]
    fn on_type_formatting_applies_only_to_the_version_it_was_asked_for() {
        let (ctx, settings) = (egui::Context::default(), Settings { lsp: false, ..Settings::default() });
        let mut editors = Editors::new();
        editors.exec(&Command::NewFile, &settings, &ctx);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        run(&ctx, &mut editors, &settings, vec![], 1.0);
        let id = editors.groups[0].tabs[0].doc;
        let old = editors.docs[&id].buf.version;
        run(&ctx, &mut editors, &settings, vec![Event::Paste("a;\n".into())], 1.0);
        let version = editors.docs[&id].buf.version;
        let edits = json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "  "}]);
        editors.reply(Want::Format { doc: id, version: old, save: false }, Some(edits.clone()), 0.0);
        assert_eq!(editors.docs[&id].buf.rope.to_string(), "a;\n");
        editors.reply(Want::Format { doc: id, version, save: false }, Some(edits), 0.0);
        assert_eq!(editors.docs[&id].buf.rope.to_string(), "  a;\n");
        editors.exec(&Command::Undo, &settings, &ctx);
        assert_eq!(editors.docs[&id].buf.rope.to_string(), "a;\n");
    }

    #[test]
    fn markdown_links_complete_paths_and_report_broken_ones() {
        let dir = std::env::temp_dir().join(format!("halo-md-intel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/guide.md"), "# Guide
").unwrap();
        let readme = dir.join("README.md");
        std::fs::write(&readme, "[gone](nope.md)
").unwrap();
        let (ctx, settings) = (egui::Context::default(), Settings { lsp: false, ..Settings::default() });
        let mut editors = Editors::new();
        editors.set_root(Some(dir.clone()));
        editors.open(&readme);
        editors.finish_loads();
        let mut t = 1.0;
        let mut frames = |editors: &mut Editors, events: Vec<Event>| {
            for frame in 0..3 {
                t += 0.016;
                run(&ctx, editors, &settings, if frame == 0 { events.clone() } else { vec![] }, t);
            }
        };
        for _ in 0..300 {
            frames(&mut editors, vec![]);
            if !editors.lsp.diagnostics(&readme).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let problems: Vec<String> = editors.lsp.diagnostics(&readme).iter().map(|d| format!("{} {:?}", d.source, d.start)).collect();
        assert_eq!(problems, ["markdown (0, 7)"]);
        let id = editors.groups[0].tabs[0].doc;
        let end = editors.docs[&id].buf.len();
        editors.groups[0].tabs[0].sels = vec![Sel::caret(end)];
        frames(&mut editors, "[a](".chars().map(|c| Event::Text(c.into())).collect());
        let labels = |editors: &Editors| editors.intel.suggest.as_ref().map(|s| s.shown.iter().map(|i| s.items[*i].label.clone()).collect::<Vec<_>>());
        assert_eq!(labels(&editors), Some(vec!["docs".to_owned(), "README.md".to_owned()]));
        let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frames(&mut editors, vec![key(Key::Tab)]);
        assert_eq!(labels(&editors), Some(vec!["guide.md".to_owned()]));
        frames(&mut editors, vec![key(Key::Tab)]);
        frames(&mut editors, vec![Event::Text("#".into())]);
        assert_eq!(labels(&editors), Some(vec!["guide".to_owned()]));
        frames(&mut editors, vec![key(Key::Enter)]);
        let buf = &editors.docs[&id].buf;
        assert!(buf.text_of(0, buf.len()).contains("[a](docs/guide.md#guide"), "{}", buf.text_of(0, buf.len()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
