use super::buffer::{After, Document, Kind, Sel};
use super::diff::{self, Hunk};
use super::highlight::Highlighter;
use super::{Doc, Editors, MAX_COLS, Tab, job, notice, typography};
use crate::command::{self, Command};
use crate::git::LineChanges;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    self, Align, Align2, CornerRadius, Event, FontId, Id, Key, Layout, Painter, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use ropey::Rope;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub(super) const GAP: f32 = 26.0;
const BAR: f32 = 28.0;
pub(super) const RULER: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Working,
    Staged,
    Fixed,
}

#[derive(Clone, Debug, PartialEq)]
struct Spec {
    rel: String,
    left: String,
    right: Option<String>,
}

impl Spec {
    fn parse(args: &[String]) -> Option<Spec> {
        let at = args.iter().position(|a| a == "--").filter(|at| at + 2 == args.len())?;
        let head: Vec<&str> = args[..at].iter().map(String::as_str).collect();
        let (left, right) = match head[..] {
            ["diff"] => (String::new(), None),
            ["diff", "--cached"] => ("HEAD".to_owned(), Some(String::new())),
            ["show", "--format=", "--diff-merges=first-parent", hash] => (format!("{hash}^"), Some(hash.to_owned())),
            ["diff", left, right] if !left.starts_with('-') && !right.starts_with('-') => (left.to_owned(), Some(right.to_owned())),
            _ => return None,
        };
        Some(Spec { rel: args[at + 1].clone(), left, right })
    }

    fn side(&self) -> Side {
        match self.right.as_deref() {
            None => Side::Working,
            Some("") => Side::Staged,
            Some(_) => Side::Fixed,
        }
    }

    fn label(rev: Option<&str>) -> String {
        match rev {
            None => "Working Tree".to_owned(),
            Some("") => "Index".to_owned(),
            Some(rev) => {
                let (hash, parent) = rev.strip_suffix('^').map_or((rev, ""), |hash| (hash, "^"));
                format!("{}{parent}", &hash[..hash.len().min(8)])
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Model {
    hunks: Vec<Hunk>,
    left: HashMap<usize, Vec<Range<usize>>>,
    right: HashMap<usize, Vec<Range<usize>>>,
}

fn model(left: &str, right: &str) -> Model {
    let hunks = diff::lines(left, right);
    let (a, b) = (diff::split(left), diff::split(right));
    let mut out = Model::default();
    for hunk in &hunks {
        for k in 0..hunk.left.len().min(hunk.right.len()) {
            let (l, r) = (hunk.left.start + k, hunk.right.start + k);
            let (x, y) = (a[l].trim_end_matches('\n'), b[r].trim_end_matches('\n'));
            if x.len() + y.len() < 4000 {
                let (ml, mr) = diff::intra(x, y);
                out.left.insert(l, ml);
                out.right.insert(r, mr);
            }
        }
    }
    out.hunks = hunks;
    out
}

fn eol(text: &str, crlf: bool) -> String {
    if crlf { text.replace('\n', "\r\n") } else { text.to_owned() }
}

fn line_text(rope: &Rope, line: usize) -> String {
    if line >= rope.len_lines() {
        return String::new();
    }
    let text: String = rope.line(line).chars().take(MAX_COLS).collect();
    text.trim_end_matches('\n').to_owned()
}

fn hunk_in(hunks: &[Hunk], line: usize, left: bool) -> Option<&Hunk> {
    let range = |h: &Hunk| if left { h.left.clone() } else { h.right.clone() };
    hunks.get(hunks.partition_point(|h| range(h).end <= line)).filter(|h| range(h).contains(&line))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Viewer {
    change: usize,
    line: Option<usize>,
    focus: bool,
    heard: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
struct ViewLine {
    left: Option<usize>,
    right: Option<usize>,
    text: String,
}

impl ViewLine {
    fn label(&self) -> String {
        match (self.left, self.right) {
            (Some(_), Some(r)) => format!("unchanged line {}: {}", r + 1, self.text),
            (None, Some(r)) => format!("+ {} inserted line: {}", r + 1, self.text),
            (l, None) => format!("- {} removed line: {}", l.unwrap_or_default() + 1, self.text),
        }
    }

    fn cue(&self) -> Option<crate::sound::Cue> {
        match (self.left, self.right) {
            (None, Some(_)) => Some(crate::sound::Cue::Inserted),
            (Some(_), None) => Some(crate::sound::Cue::Deleted),
            _ => None,
        }
    }
}

const CONTEXT: usize = 3;

fn change_lines(hunks: &[Hunk], index: usize, right_len: usize, left: impl Fn(usize) -> String, right: impl Fn(usize) -> String) -> Vec<ViewLine> {
    let Some(h) = hunks.get(index) else { return vec![] };
    let after = index.checked_sub(1).map_or(0, |i| hunks[i].right.end);
    let before = hunks.get(index + 1).map_or(right_len, |n| n.right.start).min(right_len);
    let same = |r: usize, l: usize| ViewLine { left: Some(l), right: Some(r), text: right(r) };
    let mut out: Vec<ViewLine> = (h.right.start.saturating_sub(CONTEXT).max(after)..h.right.start).map(|r| same(r, h.left.start + r - h.right.start)).collect();
    out.extend(h.left.clone().map(|l| ViewLine { left: Some(l), right: None, text: left(l) }));
    out.extend(h.right.clone().map(|r| ViewLine { left: None, right: Some(r), text: right(r) }));
    out.extend((h.right.end..(h.right.end + CONTEXT).min(before)).map(|r| same(r, h.left.end + r - h.right.end)));
    out
}

fn change_header(hunks: &[Hunk], index: usize) -> String {
    let span = |r: &Range<usize>| match r.len() {
        0 => "no lines".to_owned(),
        1 => format!("line {}", r.start + 1),
        _ => format!("lines {}–{}", r.start + 1, r.end),
    };
    let h = &hunks[index];
    format!("Change {} of {}: {} original, {} modified", index + 1, hunks.len(), span(&h.left), span(&h.right))
}

pub(super) struct DiffTab {
    pub title: String,
    top: PathBuf,
    spec: Spec,
    args: Vec<String>,
    left: Arc<String>,
    rope: Rope,
    hl: Highlighter,
    crlf: bool,
    model: Model,
    computed: u64,
    seen: (u64, f64),
    job: Option<Receiver<(u64, Model)>>,
    rows: ((u64, usize), Vec<diff::Row>),
    viewer: Option<Viewer>,
    pub inline: bool,
    pub scroll: f32,
    pub text_top: f32,
    pub line_h: f32,
}

impl DiffTab {
    fn new(load: &Load, left: String) -> Self {
        let crlf = left.contains("\r\n");
        let left = left.replace("\r\n", "\n");
        let rope = Rope::from_str(&left);
        let hl = Highlighter::new(Some(&load.top.join(&load.spec.rel)), &rope);
        Self {
            title: load.title.clone(),
            top: load.top.clone(),
            spec: load.spec.clone(),
            args: load.args.clone(),
            left: Arc::new(left),
            rope,
            hl,
            crlf,
            model: Model::default(),
            computed: 0,
            seen: (0, 0.0),
            job: None,
            rows: ((0, 0), vec![]),
            viewer: None,
            inline: load.inline,
            scroll: 0.0,
            text_top: 0.0,
            line_h: 16.0,
        }
    }

    #[cfg(test)]
    pub fn hunk_count(&self) -> usize {
        self.model.hunks.len()
    }

    pub fn pads(&self) -> Vec<(usize, u32)> {
        if self.inline { diff::inline_pads(&self.model.hunks) } else { diff::pads(&self.model.hunks, false) }
    }

    pub(super) fn parts(&self) -> (&str, &str, Option<&str>, &[Hunk], &[diff::Row]) {
        (&self.spec.rel, &self.spec.left, self.spec.right.as_deref(), &self.model.hunks, &self.rows.1)
    }

    fn sync(&mut self, buf: &Document) {
        if self.computed != buf.version {
            self.model = model(&self.left, &buf.rope.to_string());
            self.computed = buf.version;
            self.job = None;
        }
    }

    fn refresh(&mut self, buf: &Document, now: f64, ctx: &egui::Context) {
        if let Some(found) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            (self.computed, self.model, self.job) = (found.0, found.1, None);
        }
        if self.computed == buf.version || self.job.is_some() {
            return;
        }
        if buf.rope.len_bytes() + self.left.len() < 400_000 {
            return self.sync(buf);
        }
        if self.seen.0 != buf.version {
            self.seen = (buf.version, now);
        }
        let wait = 0.3 - (now - self.seen.1);
        if wait > 0.0 && self.computed != 0 {
            return ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
        let (tx, rx) = channel();
        let (left, right, version, ctx) = (self.left.clone(), buf.rope.to_string(), buf.version, ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send((version, model(&left, &right)));
            ctx.request_repaint();
        });
        self.job = Some(rx);
    }
}

#[derive(Clone)]
struct Load {
    title: String,
    top: PathBuf,
    spec: Spec,
    args: Vec<String>,
    inline: bool,
    focus: bool,
}

pub(super) struct Base {
    top: PathBuf,
    rel: String,
    text: Arc<String>,
    crlf: bool,
    model: Model,
    version: u64,
    at: Instant,
}

impl Base {
    fn sync(&mut self, buf: &Document) {
        if self.version != buf.version {
            self.model = model(&self.text, &buf.rope.to_string());
            self.version = buf.version;
        }
    }
}

enum Msg {
    Loaded(Load, String, Option<String>),
    Text(String, String),
    Base(u64, Result<Base, String>),
    Applied(PathBuf, Result<&'static str, String>),
}

enum Wait {
    Peek(usize),
    Run(Command, usize, Vec<Range<usize>>),
}

pub(super) struct Diffs {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    bases: HashMap<u64, Option<Base>>,
    gutter: HashMap<u64, LineChanges>,
    waiting: Option<(u64, Wait)>,
    peek: Option<(u64, usize, usize)>,
    at: Option<usize>,
    refresh: bool,
}

impl Default for Diffs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, bases: HashMap::new(), gutter: HashMap::new(), waiting: None, peek: None, at: None, refresh: false }
    }
}

impl Diffs {
    fn load(&self, load: Load, ctx: &egui::Context) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let show = |rev: &str| crate::git::exec(&load.top, &["show", &format!("{rev}:{}", load.spec.rel)]);
            let left = show(&load.spec.left);
            let right = load.spec.right.as_deref().map(show);
            let binary = |text: &Result<String, String>| text.as_ref().is_ok_and(|t| t.as_bytes().iter().take(8000).any(|b| *b == 0));
            let missing = match &right {
                Some(right) => left.is_err() && right.is_err(),
                None => !load.top.join(&load.spec.rel).is_file(),
            };
            let msg = if missing || binary(&left) || right.as_ref().is_some_and(binary) {
                let args: Vec<&str> = load.args.iter().map(String::as_str).collect();
                Msg::Text(load.title.clone(), crate::git::exec(&load.top, &args).unwrap_or_else(|error| error))
            } else {
                Msg::Loaded(load, left.unwrap_or_default(), right.map(Result::unwrap_or_default))
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn fresh(&self, doc: u64) -> bool {
        self.bases.get(&doc).is_some_and(|base| base.as_ref().is_some_and(|base| self.gutter.contains_key(&doc) || base.at.elapsed() < Duration::from_secs(3)))
    }

    fn load_base(&mut self, doc: u64, path: &Path, ctx: &egui::Context) {
        if matches!(self.bases.get(&doc), Some(None)) {
            return;
        }
        self.bases.entry(doc).or_insert(None);
        let (tx, ctx, path) = (self.tx.clone(), ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let result = crate::git::exec(&dir, &["rev-parse", "--show-toplevel", "--show-prefix"]).map(|out| {
                let mut lines = out.lines();
                let top = crate::wsl::local(&dir, lines.next().unwrap_or_default().trim());
                let rel = format!("{}{name}", lines.next().unwrap_or_default().trim());
                let text = crate::git::exec(&top, &["show", &format!("HEAD:{rel}")]).unwrap_or_default();
                let crlf = text.contains("\r\n");
                let text = Arc::new(text.replace("\r\n", "\n"));
                Base { top, rel, text, crlf, model: Model::default(), version: 0, at: Instant::now() }
            });
            let _ = tx.send(Msg::Base(doc, result.map_err(|_| format!("{name} is not in a Git repository."))));
            ctx.request_repaint();
        });
    }

    fn apply(&self, top: PathBuf, rel: String, old: &str, new: &str, reverse: bool, ctx: &egui::Context) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let patch = diff::patch(&rel, old, new);
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let name = format!("halo-{}-{}.patch", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
            let result = crate::git::exec(&top, &["rev-parse", "--git-path", &name]).and_then(|found| {
                let found = found.trim().to_owned();
                let file = if found.starts_with('/') || Path::new(&found).is_absolute() { crate::wsl::local(&top, &found) } else { top.join(&found) };
                std::fs::write(&file, &patch).map_err(|e| format!("Could not write the patch: {e}"))?;
                let mut args = vec!["apply", "--cached", "--whitespace=nowarn"];
                args.extend(reverse.then_some("-R"));
                args.push(&found);
                let done = crate::git::exec(&top, &args);
                let _ = std::fs::remove_file(&file);
                done.map(|_| if reverse { "Change unstaged." } else { "Change staged." })
            });
            let _ = tx.send(Msg::Applied(top.join(&rel), result));
            ctx.request_repaint();
        });
    }

    fn anchor(&mut self, doc: u64, buf: &Document, g: usize) -> Option<(usize, usize, usize)> {
        let (_, index, _) = self.peek.filter(|p| p.0 == doc && p.2 == g)?;
        let base = self.bases.get_mut(&doc)?.as_mut()?;
        base.sync(buf);
        let hunk = base.model.hunks.get(index)?;
        let line = if hunk.right.is_empty() { hunk.right.start.saturating_sub(1) } else { hunk.right.end - 1 };
        Some((line, hunk.left.len() + hunk.right.len(), index))
    }

    fn show_peek(&mut self, doc: u64, buf: &Document, line: usize, g: usize) -> Option<usize> {
        let base = self.bases.get_mut(&doc)?.as_mut()?;
        base.sync(buf);
        let index = diff::hunk_at(&base.model.hunks, line).or_else(|| diff::step(&base.model.hunks, line, true))?;
        self.peek = Some((doc, index, g));
        Some(buf.line_start(base.model.hunks[index].right.start))
    }
}

pub(super) fn hatch(painter: &Painter, rect: Rect, theme: &Theme) {
    let rect = rect.intersect(painter.clip_rect());
    if !rect.is_positive() {
        return;
    }
    let p = painter.with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::ZERO, ui::mix(theme.bg, theme.panel, 0.6));
    let stroke = Stroke::new(1.0, ui::with_alpha(theme.border, 0.8));
    let mut c = ((rect.left() + rect.top()) / 8.0).floor() * 8.0;
    while c < rect.right() + rect.bottom() {
        p.line_segment([pos2(c - rect.bottom(), rect.bottom()), pos2(c - rect.top(), rect.top())], stroke);
        c += 8.0;
    }
}

fn marks(painter: &Painter, galley: &egui::Galley, x: f32, row: Rect, ranges: Option<&Vec<Range<usize>>>, color: egui::Color32) {
    for r in ranges.into_iter().flatten() {
        let at = |col: usize| x + typography::x(galley, col);
        painter.rect_filled(Rect::from_x_y_ranges(at(r.start)..=at(r.end), row.y_range()), CornerRadius::ZERO, color);
    }
}

pub(super) fn caret_prefix(d: &DiffTab, line: usize) -> &'static str {
    match hunk_in(&d.model.hunks, line, false) {
        Some(h) if !h.left.is_empty() => "changed: ",
        Some(_) => "added: ",
        None if d.model.hunks.iter().any(|h| h.right.is_empty() && h.right.start == line) => "removed above: ",
        None => "",
    }
}

pub(super) fn tint(painter: &Painter, theme: &Theme, d: &DiffTab, line: usize, band: Rect, x: &dyn Fn(usize) -> f32) {
    if hunk_in(&d.model.hunks, line, false).is_none() {
        return;
    }
    painter.rect_filled(band, CornerRadius::ZERO, ui::with_alpha(theme.added, 0.2));
    ui::contrast_border(painter, theme, band, theme.added);
    for r in d.model.right.get(&line).into_iter().flatten() {
        painter.rect_filled(Rect::from_x_y_ranges(x(r.start)..=x(r.end), band.y_range()), CornerRadius::ZERO, ui::with_alpha(theme.added, 0.45));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_pads(
    painter: &Painter,
    theme: &Theme,
    d: &mut DiffTab,
    pads: &[(usize, u32)],
    top: &dyn Fn(usize) -> f32,
    lines: (usize, usize),
    clip: Rect,
    (text_x, number_x): (f32, f32),
    font: &FontId,
    style: typography::FontStyle,
    line_h: f32,
) {
    let number_font = FontId::monospace(font.size - 1.0);
    for &(line, count) in pads.iter().filter(|p| p.0 >= lines.0 && p.0 <= lines.1) {
        let y = top(line);
        let rect = Rect::from_min_max(pos2(clip.left(), y), pos2(clip.right(), y + count as f32 * line_h));
        if !d.inline {
            hatch(painter, rect, theme);
            continue;
        }
        let Some(hunk) = d.model.hunks.iter().find(|h| h.right.start == line && !h.left.is_empty()).cloned() else { continue };
        if !d.hl.ensure(&d.rope, hunk.left.clone(), theme, Duration::from_millis(4)) {
            painter.ctx().request_repaint();
        }
        for (k, l) in hunk.left.clone().enumerate() {
            let row = Rect::from_min_size(pos2(clip.left(), y + k as f32 * line_h), vec2(clip.width(), line_h));
            painter.rect_filled(row, CornerRadius::ZERO, ui::with_alpha(theme.deleted, 0.2));
            ui::contrast_border(painter, theme, row, theme.deleted);
            let text = line_text(&d.rope, l);
            let galley = painter.layout_job(style.apply(job(&text, 0..text.len(), d.hl.get(l), font, theme.text, &[], &[], &TextFormat::default())));
            marks(painter, &galley, text_x, row, d.model.left.get(&l), ui::with_alpha(theme.deleted, 0.45));
            painter.text(pos2(number_x, row.center().y), Align2::RIGHT_CENTER, l + 1, number_font.clone(), ui::with_alpha(theme.deleted, 0.9));
            painter.galley(pos2(text_x, row.top() + (line_h - galley.size().y) / 2.0), galley, theme.text);
        }
    }
}

fn paint_left(ui: &Ui, theme: &Theme, d: &mut DiffTab, rect: Rect, right_len: usize, settings: &Settings) {
    let font = &FontId::monospace(settings.font_size);
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(font, 'm'));
    let style = typography::FontStyle::new(settings, char_w);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, CornerRadius::ZERO, theme.bg);
    let total = d.rope.len_lines();
    if d.rows.0 != (d.computed, right_len) {
        d.rows = ((d.computed, right_len), diff::align(&d.model.hunks, total, right_len));
    }
    let line_h = d.line_h;
    let first = (d.scroll / line_h).floor().max(0.0) as usize;
    let shown = &d.rows.1[first.min(d.rows.1.len())..(first + (rect.height() / line_h) as usize + 2).min(d.rows.1.len())];
    let lines = shown.iter().filter_map(|r| r.left);
    let range = lines.clone().min().unwrap_or(0)..lines.max().map_or(0, |l| l + 1);
    if !d.hl.ensure(&d.rope, range, theme, Duration::from_millis(6)) {
        ui.ctx().request_repaint();
    }
    let gutter = (total.to_string().len().max(3) as f32 + 2.0) * char_w + 14.0;
    let number_font = FontId::monospace(font.size - 1.0);
    for (index, found) in shown.iter().enumerate() {
        let y = d.text_top + (first + index) as f32 * line_h - d.scroll;
        let row = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), line_h));
        let Some(line) = found.left else {
            hatch(&painter, row, theme);
            continue;
        };
        let text = line_text(&d.rope, line);
        let galley = painter.layout_job(style.apply(job(&text, 0..text.len(), d.hl.get(line), font, theme.text, &[], &[], &TextFormat::default())));
        if found.changed {
            painter.rect_filled(row, CornerRadius::ZERO, ui::with_alpha(theme.deleted, 0.2));
            ui::contrast_border(&painter, theme, row, theme.deleted);
            marks(&painter, &galley, rect.left() + gutter, row, d.model.left.get(&line), ui::with_alpha(theme.deleted, 0.45));
        }
        let number = if found.changed { theme.text } else { ui::with_alpha(theme.text_dim, 0.75) };
        painter.text(pos2(rect.left() + gutter - 10.0 - char_w, row.center().y), Align2::RIGHT_CENTER, line + 1, number_font.clone(), number);
        painter.galley(pos2(rect.left() + gutter, row.top() + (line_h - galley.size().y) / 2.0), galley, theme.text);
    }
}

fn dock(d: &DiffTab, body: Rect) -> f32 {
    if d.viewer.is_some() { ((body.height() - BAR) / 3.0).max(0.0).round() } else { 0.0 }
}

fn tip(cmd: &Command, ctx: &egui::Context) -> String {
    match command::shortcut_text(ctx, cmd) {
        Some(keys) => format!("{} ({keys})", cmd.label()),
        None => cmd.label().to_owned(),
    }
}

impl Editors {
    pub fn compare(&mut self, title: &str, args: &[String], top: &Path, inline: bool, ctx: &egui::Context) -> bool {
        let Some(spec) = Spec::parse(args) else { return false };
        let load = Load { title: title.to_owned(), top: top.to_path_buf(), spec, args: args.to_vec(), inline, focus: true };
        self.diffs.load(load, ctx);
        true
    }

    pub(super) fn poll_diffs(&mut self, ctx: &egui::Context) {
        self.poll_review(ctx);
        while let Ok(msg) = self.diffs.rx.try_recv() {
            match msg {
                Msg::Loaded(load, left, right) => self.place_diff(load, left, right),
                Msg::Text(title, text) => self.open_text(&title, &text, Some(Path::new("changes.diff"))),
                Msg::Base(doc, Ok(base)) => {
                    self.diffs.bases.insert(doc, Some(base));
                    if let Some((_, wait)) = self.diffs.waiting.take_if(|w| w.0 == doc) {
                        match wait {
                            Wait::Peek(line) => {
                                let shown = self.docs.get(&doc).and_then(|d| self.diffs.show_peek(doc, &d.buf, line, self.active));
                                if let (Some(pos), Some((g, t))) = (shown, self.active_tab()) {
                                    self.groups[g].tabs[t].reveal = Some(pos);
                                }
                            }
                            Wait::Run(cmd, line, ranges) => self.run_change(doc, &cmd, line, ranges, ctx),
                        }
                    }
                }
                Msg::Base(doc, Err(error)) => {
                    self.diffs.bases.remove(&doc);
                    self.diffs.waiting = None;
                    self.banner = notice(error);
                }
                Msg::Applied(path, Ok(done)) => {
                    self.diffs.refresh = true;
                    self.banner = notice(done.to_owned());
                    let key = crate::lsp::key(&path);
                    let tabs = self.groups.iter().flat_map(|g| &g.tabs).filter_map(|t| t.diff.as_deref());
                    let loads: Vec<Load> = tabs
                        .filter(|d| crate::lsp::key(&d.top.join(&d.spec.rel)) == key)
                        .map(|d| Load { title: d.title.clone(), top: d.top.clone(), spec: d.spec.clone(), args: d.args.clone(), inline: d.inline, focus: false })
                        .collect();
                    loads.into_iter().for_each(|load| self.diffs.load(load, ctx));
                }
                Msg::Applied(_, Err(error)) => self.banner = Some((format!("Git could not apply the change: {error}"), None)),
            }
        }
    }

    pub(super) fn take_git_refresh(&mut self) -> bool {
        std::mem::take(&mut self.diffs.refresh)
    }

    fn place_diff(&mut self, load: Load, left: String, right: Option<String>) {
        let path = load.top.join(&load.spec.rel);
        let found = self
            .groups
            .iter()
            .enumerate()
            .find_map(|(g, group)| Some((g, group.tabs.iter().position(|t| t.diff.as_ref().is_some_and(|d| d.title == load.title))?)));
        let doc = match &right {
            None => match self.find_doc(&path) {
                Some(id) => {
                    self.load_now(id);
                    id
                }
                None => match Document::load(&path) {
                    Ok(buf) => self.insert_doc(buf),
                    Err(error) => {
                        self.banner = Some((error, None));
                        return;
                    }
                },
            },
            Some(text) => {
                let mut buf = Document::from_text(&text.replace("\r\n", "\n"), load.title.clone());
                buf.read_only = true;
                let hl = Highlighter::new(Some(&path), &buf.rope);
                let old = found.map(|(g, t)| self.groups[g].tabs[t].doc).filter(|id| self.docs.get(id).is_some_and(|d| d.buf.read_only));
                let id = old.unwrap_or_else(|| self.insert_doc(Document::from_text("", String::new())));
                if let Some(doc) = self.docs.get_mut(&id) {
                    (doc.hl, doc.max_cols, doc.buf) = (hl, (0, 0), buf);
                }
                id
            }
        };
        let mut fresh = DiffTab::new(&load, left);
        if let Some(d) = self.docs.get(&doc) {
            fresh.sync(&d.buf);
        }
        let first = fresh.model.hunks.first().map(|h| h.right.start);
        match found {
            Some((g, t)) => {
                let tab = &mut self.groups[g].tabs[t];
                fresh.inline = tab.diff.as_ref().map_or(fresh.inline, |d| d.inline);
                fresh.viewer = tab.diff.as_ref().and_then(|d| d.viewer);
                tab.diff = Some(Box::new(fresh));
                let old = std::mem::replace(&mut tab.doc, doc);
                if load.focus {
                    (self.active, self.groups[g].active, self.want_focus) = (g, t, true);
                }
                if old != doc {
                    self.drop_doc(old);
                }
            }
            None => {
                self.add_tab(doc, vec![Sel::caret(0)]);
                let pos = first.zip(self.docs.get(&doc)).map(|(line, d)| d.buf.line_start(line));
                if let Some(tab) = self.groups[self.active].tabs.last_mut() {
                    tab.diff = Some(Box::new(fresh));
                    if let Some(pos) = pos {
                        (tab.sels, tab.reveal) = (vec![Sel::caret(pos)], Some(pos));
                    }
                }
            }
        }
    }

    pub(super) fn diff_frame(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, doc: &Doc, tab: &mut Tab, pending: &mut Vec<Command>) -> Rect {
        let Some(d) = tab.diff.as_mut() else { return body };
        let ctx = ui.ctx().clone();
        d.refresh(&doc.buf, ui.input(|i| i.time), &ctx);
        let bar = Rect::from_min_size(body.min, vec2(body.width(), BAR));
        let docked = dock(d, body);
        let area = body.with_min_y(bar.bottom()).with_max_y(body.bottom() - docked);
        let right_col = if d.inline { area } else { area.with_min_x(area.center().x + GAP / 2.0) };
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::ZERO, theme.panel);
        painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, theme.border));
        let (left_label, right_label) = (Spec::label(Some(&d.spec.left)), Spec::label(d.spec.right.as_deref()));
        let font = FontId::proportional(12.0);
        if d.inline {
            painter.text(bar.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, format!("{left_label}  vs  {right_label}"), font, theme.text_dim);
        } else {
            painter.text(bar.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, left_label, font.clone(), theme.text_dim);
            painter.text(pos2(right_col.left() + 12.0, bar.center().y), Align2::LEFT_CENTER, right_label, font, theme.text_dim);
        }
        let line = doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head));
        let hunks = &d.model.hunks;
        let status = match (hunks.len(), diff::current(hunks, line)) {
            (0, _) => "No changes".to_owned(),
            (count, at) => format!("{} of {count}", at.map_or(0, |at| at + 1)),
        };
        let selected = tab.sels.iter().any(|s| !s.is_empty());
        let mut actions: Vec<(&str, Command)> = vec![(if d.inline { icons::COLUMNS } else { icons::ROWS }, Command::ToggleInlineDiff)];
        actions.extend([(icons::ARROW_DOWN, Command::NextChange), (icons::ARROW_UP, Command::PrevChange)]);
        let ranges = match d.spec.side() {
            Side::Working => vec![(icons::ARROW_U_UP_LEFT, Command::RevertSelectedRanges), (icons::PLUS, Command::StageSelectedRanges)],
            Side::Staged => vec![(icons::MINUS, Command::UnstageSelectedRanges)],
            Side::Fixed => vec![],
        };
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (icon, cmd) in actions {
                if ui::icon_button(ui, theme, icon, 22.0, false, &tip(&cmd, &ctx)).clicked() {
                    pending.push(cmd);
                }
            }
            ui.label(RichText::new(&status).size(12.0).color(theme.text_dim));
            if selected {
                ui.add_space(8.0);
                for (icon, cmd) in ranges {
                    if ui::icon_button(ui, theme, icon, 22.0, false, &tip(&cmd, &ctx)).clicked() {
                        pending.push(cmd);
                    }
                }
            }
        });
        if docked > 0.0 {
            self.viewer_ui(ui, body.with_min_y(body.bottom() - docked), theme, doc, tab);
        }
        right_col
    }

    pub(super) fn diff_after(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, settings: &Settings, tab: &mut Tab, pending: &mut Vec<Command>) {
        let Some(d) = tab.diff.as_mut() else { return };
        let area = body.with_min_y(body.top() + BAR).with_max_y(body.bottom() - dock(d, body));
        let starts = &tab.rows.1;
        let Some(rows) = starts.last().map(|r| *r as usize) else { return };
        let id = Id::new(("diff_view", &d.title));
        if !d.inline {
            let left = area.with_max_x(area.center().x - GAP / 2.0);
            paint_left(ui, theme, d, left, starts.len() - 1, settings);
            let gap = Rect::from_min_max(pos2(left.right(), area.top()), pos2(area.center().x + GAP / 2.0, area.bottom()));
            ui.painter().rect_filled(gap, CornerRadius::ZERO, theme.bg);
            ui.painter().vline(gap.center().x, gap.y_range(), Stroke::new(1.0, theme.border));
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0.0 && ui.rect_contains_pointer(left.union(gap)) {
                tab.set_scroll = Some((d.scroll - delta).max(0.0));
            }
        }
        let line_h = d.line_h;
        let y_of = |row: usize| d.text_top + row as f32 * line_h - d.scroll;
        let span = |h: &Hunk| if d.inline { h.left.len() + h.right.len() } else { h.left.len().max(h.right.len()) };
        let pads = d.pads();
        let row_of = |line: usize| {
            let pad = pads.iter().find(|p| p.0 == line).map_or(0, |p| p.1 as usize);
            starts.get(line).map_or(rows, |r| *r as usize).saturating_sub(pad)
        };
        let strip = Rect::from_min_max(pos2(area.right() - RULER, area.top()), area.max);
        let painter = ui.painter().with_clip_rect(strip);
        painter.rect_filled(strip, CornerRadius::ZERO, theme.bg);
        painter.vline(strip.left(), strip.y_range(), Stroke::new(1.0, ui::with_alpha(theme.border, 0.6)));
        let scale = strip.height() / rows.max(1) as f32;
        let mid = strip.center().x;
        for h in &d.model.hunks {
            let y = strip.top() + row_of(h.right.start) as f32 * scale;
            let height = |lines: usize| (lines.max(1) as f32 * scale).max(3.0);
            // removed lines on the left lane, added lines on the right, like VS Code's overview ruler
            if !h.left.is_empty() {
                painter.rect_filled(Rect::from_min_max(pos2(strip.left() + 2.0, y), pos2(mid, y + height(h.left.len()))), CornerRadius::ZERO, theme.deleted);
            }
            if !h.right.is_empty() {
                painter.rect_filled(Rect::from_min_max(pos2(mid, y), pos2(strip.right() - 2.0, y + height(h.right.len()))), CornerRadius::ZERO, theme.added);
            }
        }
        let shown = area.height() / line_h;
        let slider = Rect::from_min_size(pos2(strip.left() + 1.0, strip.top() + d.scroll / line_h * scale), vec2(RULER - 1.0, (shown * scale).max(8.0)));
        painter.rect_filled(slider, CornerRadius::ZERO, ui::with_alpha(theme.text, 0.08));
        let resp = ui.interact(strip, id.with("ruler"), Sense::click_and_drag());
        if (resp.clicked() || resp.dragged())
            && let Some(pos) = resp.interact_pointer_pos()
        {
            tab.set_scroll = Some((((pos.y - strip.top()) / scale - shown / 2.0) * line_h).max(0.0));
        }
        let buttons: &[(&str, Command)] = match d.spec.side() {
            Side::Working => &[(icons::PLUS, Command::StageHunk), (icons::ARROW_U_UP_LEFT, Command::RevertHunk)],
            Side::Staged => &[(icons::MINUS, Command::UnstageHunk)],
            Side::Fixed => &[],
        };
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|pos| area.contains(*pos));
        for h in d.model.hunks.iter().filter(|_| !buttons.is_empty()) {
            let (y0, y1) = (y_of(row_of(h.right.start)), y_of(row_of(h.right.start) + span(h).max(1)));
            if !pointer.is_some_and(|pos| pos.y >= y0 && pos.y < y1.max(y0 + 22.0 * buttons.len() as f32)) || y1 < area.top() {
                continue;
            }
            for (index, (icon, cmd)) in buttons.iter().enumerate() {
                let at = match d.inline {
                    true => pos2(area.right() - RULER - 30.0 - 24.0 * index as f32, y0.max(area.top()) + 1.0),
                    false => pos2(area.center().x - 11.0, y0.max(area.top()) + 1.0 + 24.0 * index as f32),
                };
                let r = Rect::from_min_size(at, vec2(22.0, 22.0));
                ui.painter().with_clip_rect(area).rect_filled(r, CornerRadius::same(4), theme.raised);
                let clicked = ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
                    ui.set_clip_rect(area.intersect(ui.clip_rect()));
                    ui::icon_button(ui, theme, icon, 22.0, false, cmd.label()).clicked()
                });
                if clicked.inner {
                    self.diffs.at = Some(h.right.start);
                    pending.push(cmd.clone());
                }
            }
        }
        self.review_ui(ui, area.with_max_x(strip.left()), theme, d, starts, &pads);
    }

    fn viewer_ui(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme, doc: &Doc, tab: &mut Tab) {
        let Some(d) = tab.diff.as_mut() else { return };
        let Some(mut v) = d.viewer else { return };
        let (buf, hunks) = (&doc.buf, &d.model.hunks);
        if hunks.is_empty() {
            d.viewer = None;
            self.want_focus = true;
            return;
        }
        if v.focus {
            self.want_focus = false;
        }
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, theme.panel);
        painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, theme.border));
        let last = buf.lines() - 1;
        let right_len = if buf.rope.line(last).len_chars() == 0 { last } else { last + 1 };
        let lines_of = |change: usize| change_lines(hunks, change, right_len, |l| line_text(&d.rope, l), |r| buf.line_text(r));
        let first = |lines: &[ViewLine]| lines.iter().position(|l| l.left.is_none() || l.right.is_none()).unwrap_or(0);
        v.change = v.change.min(hunks.len() - 1);
        let mut lines = lines_of(v.change);
        let mut line = v.line.unwrap_or_else(|| first(&lines)).min(lines.len().saturating_sub(1));
        let base = Id::new(("diff_viewer", &d.title));
        let (mut close, mut jump) = (false, false);
        if let Some(k) = ui.memory(|m| m.focused()).and_then(|id| (0..lines.len()).find(|k| base.with(*k) == id)) {
            line = k;
            let filter = egui::EventFilter { tab: false, horizontal_arrows: false, vertical_arrows: true, escape: true };
            ui.memory_mut(|m| m.set_focus_lock_filter(base.with(line), filter));
            let step = ui.input_mut(|i| match (i.consume_key(egui::Modifiers::SHIFT, Key::F7), i.consume_key(egui::Modifiers::NONE, Key::F7)) {
                (true, _) => Some(hunks.len() - 1),
                (_, true) => Some(1),
                _ => None,
            });
            if let Some(step) = step {
                v.change = (v.change + step) % hunks.len();
                lines = lines_of(v.change);
                (line, v.focus) = (first(&lines), true);
            }
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    (line, v.focus) = ((line + 1).min(lines.len() - 1), true);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    (line, v.focus) = (line.saturating_sub(1), true);
                }
                jump = i.consume_key(egui::Modifiers::NONE, Key::Enter);
                close = i.consume_key(egui::Modifiers::NONE, Key::Escape);
            });
        }
        let header = Rect::from_min_size(rect.min + vec2(0.0, 1.0), vec2(rect.width(), 26.0));
        let title = change_header(hunks, v.change);
        ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            close |= ui::icon_button(ui, theme, icons::X, 22.0, false, "Close (Escape)").clicked();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| ui.add(egui::Label::new(RichText::new(&title).size(12.0).color(theme.text)).truncate()));
        });
        let size = self.config.font_size;
        let (font, number_font) = (FontId::monospace(size), FontId::monospace(size - 1.0));
        let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'm'));
        let row_h = (size * 1.35).round();
        let digits = right_len.max(d.rope.len_lines()).to_string().len().max(3) as f32 * char_w;
        let focus = std::mem::take(&mut v.focus);
        let mut clicked = None;
        ui.scope_builder(UiBuilder::new().max_rect(rect.with_min_y(header.bottom())).id_salt(base.with("list")), |ui| {
            egui::ScrollArea::vertical().id_salt(base.with("scroll")).auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, l) in lines.iter().enumerate() {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                    let resp = ui.interact(row, base.with(k), Sense::click());
                    let color = match (l.left, l.right) {
                        (None, _) => Some(theme.added),
                        (_, None) => Some(theme.deleted),
                        _ => None,
                    };
                    if let Some(color) = color {
                        ui.painter().rect_filled(row, CornerRadius::ZERO, ui::with_alpha(color, 0.2));
                    }
                    ui::row_bg(ui, theme, row, resp.id, resp.hovered(), k == line);
                    let (x, y) = (row.left() + 8.0, row.center().y);
                    for (n, at) in [(l.left, x + digits), (l.right, x + 2.0 * digits + 8.0)] {
                        if let Some(n) = n {
                            ui.painter().text(pos2(at, y), Align2::RIGHT_CENTER, n + 1, number_font.clone(), theme.text_dim);
                        }
                    }
                    let marker = if l.left.is_none() { "+" } else if l.right.is_none() { "-" } else { " " };
                    let text_x = x + 2.0 * digits + 20.0;
                    ui.painter().text(pos2(text_x, y), Align2::LEFT_CENTER, marker, font.clone(), color.unwrap_or(theme.text_dim));
                    let shown: String = l.text.chars().take(MAX_COLS).collect();
                    ui.painter().text(pos2(text_x + 2.0 * char_w, y), Align2::LEFT_CENTER, shown, font.clone(), theme.text);
                    ui::list_item(ui, &resp, &l.label(), k == line, (k, lines.len()));
                    if resp.clicked() {
                        clicked = Some(k);
                    }
                    if focus && k == line {
                        resp.request_focus();
                        resp.scroll_to_me(None);
                        ui.ctx().request_repaint();
                    }
                }
            });
        });
        if let Some(k) = clicked {
            (line, v.focus) = (k, true);
            ui.ctx().request_repaint();
        }
        if Some((v.change, line)) != v.heard {
            v.heard = Some((v.change, line));
            if let Some(cue) = lines.get(line).and_then(ViewLine::cue) {
                crate::sound::play(&self.config, cue);
            }
        }
        let h = &hunks[v.change];
        let target = lines.get(line).map(|l| l.right.unwrap_or_else(|| h.right.start + (l.left.unwrap_or_default() - h.left.start).min(h.right.len().saturating_sub(1))));
        v.line = Some(line);
        d.viewer = (!close && !jump).then_some(v);
        if jump && let Some(r) = target {
            let pos = buf.line_start(r);
            (tab.sels, tab.reveal) = (vec![Sel::caret(pos)], Some(pos));
        }
        self.want_focus |= close || jump;
    }

    pub(super) fn change_anchor(&mut self, doc: u64, buf: &Document, g: usize) -> Option<(usize, usize, usize)> {
        self.diffs.anchor(doc, buf, g)
    }

    pub(super) fn peek_escape(&mut self, events: &mut Vec<Event>, doc: u64) {
        if self.diffs.peek.is_none_or(|p| p.0 != doc || p.2 != self.active) {
            return;
        }
        let before = events.len();
        events.retain(|e| !matches!(e, Event::Key { key: Key::Escape, pressed: true, .. }));
        if events.len() != before {
            self.diffs.peek = None;
        }
    }

    pub(super) fn watch_changes(&mut self, doc: u64, path: &Path, changes: Option<&LineChanges>, ctx: &egui::Context) {
        match changes {
            Some(c) if c.is_empty() => drop(self.diffs.gutter.remove(&doc)),
            Some(c) if !self.diffs.gutter.get(&doc).is_some_and(|seen| Arc::ptr_eq(seen, c)) => {
                self.diffs.gutter.insert(doc, c.clone());
                self.diffs.load_base(doc, path, ctx);
            }
            _ => {}
        }
    }

    pub(super) fn open_change_peek(&mut self, doc: u64, buf: &Document, tab: &mut Tab, line: usize, ctx: &egui::Context) {
        if self.diffs.fresh(doc) {
            if let Some(pos) = self.diffs.show_peek(doc, buf, line, self.active) {
                tab.reveal = Some(pos);
            }
            return;
        }
        if let Some(path) = buf.path.clone() {
            self.diffs.waiting = Some((doc, Wait::Peek(line)));
            self.diffs.load_base(doc, &path, ctx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn change_peek_ui(&mut self, ui: &mut Ui, theme: &Theme, font: &FontId, style: typography::FontStyle, zone: Rect, doc: u64, buf: &Document, pending: &mut Vec<Command>) {
        let Some((_, index, _)) = self.diffs.peek.filter(|p| p.0 == doc) else { return };
        let Some(base) = self.diffs.bases.get_mut(&doc).and_then(Option::as_mut) else { return };
        base.sync(buf);
        let count = base.model.hunks.len();
        let Some(hunk) = base.model.hunks.get(index).cloned() else {
            self.diffs.peek = None;
            return;
        };
        let row_h = (font.size * 1.35).round();
        let rect = Rect::from_min_max(zone.min + vec2(6.0, 2.0), zone.max - vec2(8.0, 2.0));
        ui.interact(rect, Id::new(("change_peek", doc)), Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect(rect, CornerRadius::same(3), theme.panel, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 26.0)).shrink(1.0);
        painter.rect_filled(header, CornerRadius::ZERO, ui::mix(theme.panel, theme.accent, 0.14));
        let ctx = ui.ctx().clone();
        let mut chosen = None;
        ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui::icon_button(ui, theme, icons::X, 22.0, false, "Close (Escape)").clicked() {
                chosen = Some(None);
            }
            let items = [
                (icons::PLUS, "Stage Change", Command::StageHunk),
                (icons::ARROW_U_UP_LEFT, "Revert Change", Command::RevertHunk),
                (icons::ARROW_DOWN, "Next Change", Command::ShowNextChange),
                (icons::ARROW_UP, "Previous Change", Command::ShowPrevChange),
            ];
            for (icon, label, cmd) in items {
                let keys = command::shortcut_text(&ctx, &cmd).map_or(String::new(), |keys| format!(" ({keys})"));
                if ui::icon_button(ui, theme, icon, 22.0, false, &format!("{label}{keys}")).clicked() {
                    chosen = Some(Some(cmd));
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(&buf.title).color(theme.text));
                let detail = format!("Change {} of {count} since HEAD", index + 1);
                ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(theme.text_dim)).truncate());
            });
        });
        let left = diff::split(&base.text);
        let body = Rect::from_min_max(pos2(rect.left() + 1.0, header.bottom() + 4.0), rect.max - vec2(1.0, 1.0));
        let text_x = body.left() + 28.0;
        let rows = hunk.left.clone().map(|l| (left[l].trim_end_matches('\n').to_owned(), base.model.left.get(&l), true));
        let added = hunk.right.clone().map(|r| (buf.line_text(r), base.model.right.get(&r), false));
        for (k, (text, ranges, removed)) in rows.chain(added).enumerate() {
            let row = Rect::from_min_size(pos2(body.left(), body.top() + k as f32 * row_h), vec2(body.width(), row_h));
            if row.top() > body.bottom() {
                break;
            }
            let color = if removed { theme.deleted } else { theme.added };
            painter.rect_filled(row, CornerRadius::ZERO, ui::with_alpha(color, 0.13));
            let shown: String = text.chars().take(MAX_COLS).collect();
            let galley = painter.layout_job(style.apply(LayoutJob::simple_singleline(shown, font.clone(), theme.text)));
            marks(&painter, &galley, text_x, row, ranges, ui::with_alpha(color, 0.3));
            painter.text(pos2(body.left() + 14.0, row.center().y), Align2::CENTER_CENTER, if removed { "-" } else { "+" }, font.clone(), color);
            painter.galley(pos2(text_x, row.top() + (row_h - galley.size().y) / 2.0), galley, theme.text);
        }
        match chosen {
            Some(None) => {
                self.diffs.peek = None;
                self.want_focus = true;
            }
            Some(Some(cmd)) => {
                self.diffs.at = Some(hunk.right.start);
                pending.push(cmd);
            }
            None => {}
        }
    }

    pub(super) fn diff_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        use Command::*;
        if matches!(cmd, AccessibleDiffViewerNext | AccessibleDiffViewerPrev) {
            let forward = *cmd == AccessibleDiffViewerNext;
            let Some((g, t)) = self.active_tab() else { return true };
            let tab = &mut self.groups[g].tabs[t];
            let line = self.docs.get(&tab.doc).map_or(0, |doc| doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head)));
            let Some(d) = tab.diff.as_mut() else {
                let next = if forward { NextChange } else { PrevChange };
                return self.merge_exec(&next, ctx) || self.diff_exec(&next, ctx);
            };
            let count = d.model.hunks.len();
            let change = match d.viewer {
                _ if count == 0 => {
                    self.banner = notice("No changes.".to_owned());
                    return true;
                }
                Some(v) => (v.change + if forward { 1 } else { count - 1 }) % count,
                None => diff::hunk_at(&d.model.hunks, line).or_else(|| diff::step(&d.model.hunks, line, forward)).unwrap_or(0),
            };
            d.viewer = Some(Viewer { change, focus: true, ..Viewer::default() });
            return true;
        }
        let ranged = matches!(cmd, StageSelectedRanges | RevertSelectedRanges | UnstageSelectedRanges);
        if !ranged && !matches!(cmd, NextChange | PrevChange | ShowNextChange | ShowPrevChange | ToggleInlineDiff | StageHunk | RevertHunk | UnstageHunk) {
            return false;
        }
        let at = self.diffs.at.take();
        let Some((g, t)) = self.active_tab() else { return true };
        let tab = &mut self.groups[g].tabs[t];
        let doc_id = tab.doc;
        let Some(doc) = self.docs.get_mut(&doc_id) else { return true };
        if *cmd == ToggleInlineDiff {
            if let Some(d) = tab.diff.as_mut() {
                d.inline ^= true;
            }
            return true;
        }
        doc.buf.clamp(&mut tab.sels);
        let line = at.unwrap_or_else(|| doc.buf.line_of(tab.sels[0].head));
        let ranges = match ranged && at.is_none() {
            true => tab.sels.iter().map(|s| doc.buf.line_of(s.start())..doc.buf.line_of(s.end()) + 1).collect(),
            false => vec![],
        };
        if tab.diff.is_none() && !self.diffs.fresh(doc_id) {
            match doc.buf.path.clone() {
                Some(path) => {
                    self.diffs.waiting = Some((doc_id, Wait::Run(cmd.clone(), line, ranges)));
                    self.diffs.load_base(doc_id, &path, ctx);
                }
                None => self.banner = notice("Save the file to compare it with Git.".to_owned()),
            }
            return true;
        }
        self.run_change(doc_id, cmd, line, ranges, ctx);
        true
    }

    fn run_change(&mut self, doc_id: u64, cmd: &Command, line: usize, ranges: Vec<Range<usize>>, ctx: &egui::Context) {
        use Command::*;
        let Some((g, t)) = self.active_tab().filter(|(g, t)| self.groups[*g].tabs[*t].doc == doc_id) else { return };
        let tab = &mut self.groups[g].tabs[t];
        let Some(doc) = self.docs.get_mut(&doc_id) else { return };
        let buf = &mut doc.buf;
        let (left, hunks, side, top, rel, crlf) = match tab.diff.as_mut() {
            Some(d) => {
                d.sync(buf);
                (d.left.clone(), d.model.hunks.clone(), d.spec.side(), d.top.clone(), d.spec.rel.clone(), d.crlf)
            }
            None => {
                let Some(Some(base)) = self.diffs.bases.get_mut(&doc_id) else { return };
                base.sync(buf);
                (base.text.clone(), base.model.hunks.clone(), Side::Working, base.top.clone(), base.rel.clone(), base.crlf)
            }
        };
        let in_diff = tab.diff.is_some();
        if matches!(cmd, NextChange | PrevChange | ShowNextChange | ShowPrevChange) {
            let Some(index) = diff::step(&hunks, line, matches!(cmd, NextChange | ShowNextChange)) else {
                self.banner = notice("No changes.".to_owned());
                return;
            };
            let pos = buf.line_start(hunks[index].right.start);
            if in_diff || matches!(cmd, NextChange | PrevChange) {
                tab.sels = vec![Sel::caret(pos)];
            }
            tab.reveal = Some(pos);
            if !in_diff && matches!(cmd, ShowNextChange | ShowPrevChange) {
                self.diffs.peek = Some((doc_id, index, self.active));
            }
            return;
        }
        let whole = ranges.is_empty();
        let chosen: Vec<usize> = match whole {
            true => diff::hunk_at(&hunks, line).into_iter().collect(),
            false => (0..hunks.len()).filter(|i| ranges.iter().any(|r| diff::touches(&hunks[*i], r))).collect(),
        };
        if chosen.is_empty() {
            self.banner = notice(if whole { "There is no change at the cursor." } else { "The selection has no changes." }.to_owned());
            return;
        }
        let picked = |r: usize| whole || ranges.iter().any(|x| x.contains(&r));
        let pick_right = |r: usize| chosen.iter().any(|i| hunks[*i].right.contains(&r)) && picked(r);
        let pick_left = |l: usize| {
            chosen.iter().map(|i| &hunks[*i]).filter(|h| h.left.contains(&l)).any(|h| match l - h.left.start {
                _ if whole => true,
                k if k < h.right.len() => picked(h.right.start + k),
                _ => ranges.iter().any(|x| diff::touches(&Hunk { left: 0..0, right: h.right.end..h.right.end }, x)),
            })
        };
        let right = buf.rope.to_string();
        let (a, b) = (diff::split(&left), diff::split(&right));
        match (cmd, side) {
            (_, Side::Fixed) => self.banner = notice("Both sides of this comparison are fixed.".to_owned()),
            (RevertHunk | RevertSelectedRanges, Side::Working) => {
                let edit = |h: &Hunk| (buf.line_start(h.right.start), buf.line_start(h.right.end), diff::hunk_text(&a, &b, h, pick_left, |r| !pick_right(r)));
                let edits = chosen.iter().map(|i| edit(&hunks[*i])).collect();
                buf.apply(&mut tab.sels, edits, Kind::Other, After::Map);
                tab.reveal = tab.sels.first().map(|s| s.head);
            }
            (StageHunk | StageSelectedRanges, Side::Working) => {
                let target = diff::splice(&a, &b, &hunks, |l| !pick_left(l), pick_right);
                self.diffs.apply(top, rel, &eol(&left, crlf), &eol(&target, crlf), false, ctx);
            }
            (UnstageHunk | UnstageSelectedRanges, Side::Staged) => {
                let target = diff::splice(&a, &b, &hunks, pick_left, |r| !pick_right(r));
                self.diffs.apply(top, rel, &eol(&target, crlf), &eol(&right, crlf), true, ctx);
            }
            (UnstageHunk | UnstageSelectedRanges, _) => self.banner = notice("Open the staged changes of this file to unstage.".to_owned()),
            (RevertHunk | RevertSelectedRanges, _) => self.banner = notice("Unstage the change first, then revert it in the working tree.".to_owned()),
            _ => self.banner = notice("This change is already staged.".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_what_a_diff_compares_from_git_arguments() {
        let working = Spec::parse(&strings(&["diff", "--", "src/a.rs"])).unwrap();
        assert_eq!((working.rel.as_str(), working.left.as_str(), working.right.as_deref(), working.side()), ("src/a.rs", "", None, Side::Working));
        let staged = Spec::parse(&strings(&["diff", "--cached", "--", "a.rs"])).unwrap();
        assert_eq!((staged.left.as_str(), staged.right.as_deref(), staged.side()), ("HEAD", Some(""), Side::Staged));
        let shown = Spec::parse(&strings(&["show", "--format=", "--diff-merges=first-parent", "abcdef1234", "--", "b.rs"])).unwrap();
        assert_eq!((shown.left.as_str(), shown.right.as_deref(), shown.side()), ("abcdef1234^", Some("abcdef1234"), Side::Fixed));
        assert_eq!((Spec::label(Some(&shown.left)), Spec::label(shown.right.as_deref())), ("abcdef12^".to_owned(), "abcdef12".to_owned()));
        let revs = Spec::parse(&strings(&["diff", "base1", "head2", "--", "c.rs"])).unwrap();
        assert_eq!((revs.left.as_str(), revs.right.as_deref(), revs.side()), ("base1", Some("head2"), Side::Fixed));
        assert!(Spec::parse(&strings(&["diff", "--stat", "head2", "--", "c.rs"])).is_none());
        assert!(Spec::parse(&strings(&["log", "--", "a"])).is_none());
        assert!(Spec::parse(&strings(&["diff", "--", "a", "b"])).is_none());
    }

    fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<Event>) -> Vec<Command> {
        let (theme, git, settings) = (Theme::dark(), crate::git::Git::new(), Settings::default());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))), events, focused: true, ..Default::default() };
        let mut cmds = vec![];
        let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut cmds));
        out.textures_delta.clear();
        cmds
    }

    fn settle(ctx: &egui::Context, editors: &mut Editors, what: &str, done: impl Fn(&Editors) -> bool) {
        for _ in 0..800 {
            frame(ctx, editors, vec![]);
            if done(editors) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}: {:?}", editors.banner);
    }

    fn repo(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-diff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"]] {
            crate::git::exec(&dir, args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), text).unwrap();
        crate::git::exec(&dir, &["add", "."]).unwrap();
        crate::git::exec(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        dir
    }

    fn active(editors: &Editors) -> (&Tab, &Document) {
        let (g, t) = editors.active_tab().unwrap();
        let tab = &editors.groups[g].tabs[t];
        (tab, &editors.docs[&tab.doc].buf)
    }

    fn caret_line(editors: &Editors) -> usize {
        let (tab, buf) = active(editors);
        buf.line_of(tab.sels[0].head)
    }

    fn cached(dir: &Path) -> String {
        crate::git::exec(dir, &["diff", "--cached", "--no-color", "-U0"]).unwrap()
    }

    fn hunks(editors: &Editors) -> usize {
        active(editors).0.diff.as_ref().map_or(0, |d| d.model.hunks.len())
    }

    fn click_bar(ctx: &egui::Context, editors: &mut Editors, id: u64, line: usize) -> bool {
        let mut tab = std::mem::take(&mut editors.groups[0].tabs[0]);
        let doc = editors.docs.remove(&id).unwrap();
        editors.open_change_peek(id, &doc.buf, &mut tab, line, ctx);
        editors.docs.insert(id, doc);
        editors.groups[0].tabs[0] = tab;
        editors.diffs.peek.is_some() && editors.diffs.waiting.is_none()
    }

    fn prefetched(dir: &Path, ctx: &egui::Context) -> (Editors, u64) {
        let mut editors = Editors::new();
        editors.open(&dir.join("a.txt"));
        editors.finish_loads();
        let id = editors.find_doc(&dir.join("a.txt")).unwrap();
        let changes: LineChanges = Arc::new(vec![(1, crate::git::LineChange::Modified, false)]);
        editors.watch_changes(id, &dir.join("a.txt"), Some(&changes), ctx);
        settle(ctx, &mut editors, "the prefetched base", |e| e.diffs.bases.get(&id).is_some_and(Option::is_some));
        (editors, id)
    }

    #[test]
    fn a_gutter_click_peeks_from_the_prefetched_base_without_git() {
        let ctx = egui::Context::default();
        let dir = repo("gutter", "one\ntwo\nthree\n");
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        let (mut editors, id) = prefetched(&dir, &ctx);
        if let Some(Some(base)) = editors.diffs.bases.get_mut(&id) {
            base.at = Instant::now() - Duration::from_secs(60);
        }
        assert!(click_bar(&ctx, &mut editors, id, 1), "the peek must open in the click frame");
        assert_eq!(editors.diffs.peek, Some((id, 0, 0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "needs git inside the Ubuntu-24.04 WSL distro"]
    fn times_a_gutter_click_in_wsl() {
        let ctx = egui::Context::default();
        let dir = PathBuf::from(format!(r"\\wsl.localhost\Ubuntu-24.04\tmp\halo-gutter-wsl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"]] {
            crate::git::exec(&dir, args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        crate::git::exec(&dir, &["add", "."]).unwrap();
        crate::git::exec(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();

        let mut cold = Editors::new();
        cold.open(&dir.join("a.txt"));
        cold.finish_loads();
        let id = cold.find_doc(&dir.join("a.txt")).unwrap();
        let started = Instant::now();
        assert!(!click_bar(&ctx, &mut cold, id, 1));
        settle(&ctx, &mut cold, "the cold peek", |e| e.diffs.peek.is_some());
        eprintln!("click without the gutter base: {:?}", started.elapsed());

        let (mut warm, id) = prefetched(&dir, &ctx);
        let started = Instant::now();
        assert!(click_bar(&ctx, &mut warm, id, 1));
        eprintln!("click with the gutter base: {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "needs git inside the Ubuntu-24.04 WSL distro"]
    fn opens_changes_of_a_nested_wsl_file_open_or_not() {
        let ctx = egui::Context::default();
        let dir = PathBuf::from(format!(r"\\wsl.localhost\Ubuntu-24.04\tmp\halo-diff-wsl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src").join("kv")).unwrap();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"]] {
            crate::git::exec(&dir, args).unwrap();
        }
        let file = dir.join("src").join("kv").join("a.cpp");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        crate::git::exec(&dir, &["add", "."]).unwrap();
        crate::git::exec(&dir, &["commit", "-q", "-m", "init"]).unwrap();
        std::fs::write(&file, "one\nTWO\nthree\nfour\n").unwrap();
        let args = strings(&["diff", "--", "src/kv/a.cpp"]);

        let mut editors = Editors::new();
        assert!(editors.compare("a.cpp (changes)", &args, &dir, false, &ctx));
        settle(&ctx, &mut editors, "the diff tab", |e| e.active_tab().is_some_and(|(g, t)| e.groups[g].tabs[t].diff.is_some()));
        assert_eq!(hunks(&editors), 2);

        let mut editors = Editors::new();
        editors.open(&file);
        settle(&ctx, &mut editors, "the file", |e| e.active_tab().is_some());
        assert!(editors.compare("a.cpp (changes)", &args, &dir, false, &ctx));
        settle(&ctx, &mut editors, "the diff tab", |e| e.active_tab().is_some_and(|(g, t)| e.groups[g].tabs[t].diff.is_some()));
        assert_eq!(hunks(&editors), 2);
        assert_eq!(editors.groups[0].tabs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stages_unstages_and_reverts_hunks_and_selected_lines() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let numbers: String = (1..=12).map(|i| format!("{i}\n")).collect();
        let dir = repo("stage", &numbers);
        let changed = numbers.replace("2\n3\n", "TWO\n3\n").replace("11\n", "ELEVEN\n") + "thirteen\nfourteen\n";
        std::fs::write(dir.join("a.txt"), &changed).unwrap();

        let mut editors = Editors::new();
        assert!(!editors.compare("a.txt (changes)", &strings(&["log"]), &dir, false, &ctx));
        assert!(editors.compare("a.txt (changes)", &strings(&["diff", "--", "a.txt"]), &dir, false, &ctx));
        settle(&ctx, &mut editors, "the diff tab", |e| e.active_tab().is_some_and(|(g, t)| e.groups[g].tabs[t].diff.is_some()));
        assert_eq!(hunks(&editors), 3);
        assert_eq!(caret_line(&editors), 1);
        let (tab, buf) = active(&editors);
        assert_eq!(buf.rope.to_string(), changed);
        assert!(!buf.read_only && tab.rows.1.len() == buf.lines() + 1);

        editors.exec(&Command::NextChange, &settings, &ctx);
        assert_eq!(caret_line(&editors), 10);
        editors.exec(&Command::NextChange, &settings, &ctx);
        assert_eq!(caret_line(&editors), 12);
        editors.exec(&Command::NextChange, &settings, &ctx);
        assert_eq!(caret_line(&editors), 1);
        editors.exec(&Command::PrevChange, &settings, &ctx);
        assert_eq!(caret_line(&editors), 12);

        editors.exec(&Command::PrevChange, &settings, &ctx);
        editors.exec(&Command::PrevChange, &settings, &ctx);
        editors.exec(&Command::StageHunk, &settings, &ctx);
        settle(&ctx, &mut editors, "the staged hunk", |e| hunks(e) == 2);
        assert!(cached(&dir).contains("+TWO") && !cached(&dir).contains("ELEVEN"), "{}", cached(&dir));

        let (g, t) = editors.active_tab().unwrap();
        let tab = &mut editors.groups[g].tabs[t];
        let buf = &editors.docs[&tab.doc].buf;
        tab.sels = vec![Sel::new(buf.line_start(12), buf.line_start(12) + 3)];
        editors.exec(&Command::StageSelectedRanges, &settings, &ctx);
        settle(&ctx, &mut editors, "the staged line", |_| cached(&dir).contains("+thirteen"));
        let staged = crate::git::exec(&dir, &["show", ":a.txt"]).unwrap();
        assert!(staged.ends_with("12\nthirteen\n") && staged.contains("TWO\n") && !staged.contains("ELEVEN"), "{staged}");

        editors.exec(&Command::GoTo(11, 1), &settings, &ctx);
        editors.exec(&Command::RevertHunk, &settings, &ctx);
        assert_eq!(active(&editors).1.line_text(10), "11");
        editors.exec(&Command::Undo, &settings, &ctx);
        assert_eq!(active(&editors).1.line_text(10), "ELEVEN");

        assert!(editors.compare("a.txt (staged)", &strings(&["diff", "--cached", "--", "a.txt"]), &dir, false, &ctx));
        settle(&ctx, &mut editors, "the staged diff", |e| active(e).0.diff.as_ref().is_some_and(|d| d.title == "a.txt (staged)"));
        assert!(active(&editors).1.read_only);
        assert_eq!((hunks(&editors), caret_line(&editors)), (2, 1));
        editors.exec(&Command::RevertHunk, &settings, &ctx);
        assert!(editors.banner.as_ref().is_some_and(|b| b.0.contains("Unstage")));
        editors.exec(&Command::UnstageHunk, &settings, &ctx);
        settle(&ctx, &mut editors, "the unstaged hunk", |_| !cached(&dir).contains("TWO"));
        assert!(cached(&dir).contains("+thirteen"));
        settle(&ctx, &mut editors, "the reloaded staged diff", |e| hunks(e) == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_each_change_with_context_and_screen_reader_labels() {
        let (left, right) = ("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n", "a\nB\nc\nd\ne\nf\ng\nh\nj\nk\nl\n");
        let m = model(left, right);
        let (a, b) = (diff::split(left), diff::split(right));
        let text = |lines: &[&str], at: usize| lines[at].trim_end_matches('\n').to_owned();
        let lines = |index| change_lines(&m.hunks, index, b.len(), |l| text(&a, l), |r| text(&b, r));
        assert_eq!(m.hunks.len(), 3);
        let first: Vec<String> = lines(0).iter().map(ViewLine::label).collect();
        assert_eq!(first, ["unchanged line 1: a", "- 2 removed line: b", "+ 2 inserted line: B", "unchanged line 3: c", "unchanged line 4: d", "unchanged line 5: e"]);
        assert_eq!(change_header(&m.hunks, 0), "Change 1 of 3: line 2 original, line 2 modified");
        let second = lines(1);
        assert_eq!(second.iter().map(|l| (l.left, l.right)).collect::<Vec<_>>(), [(Some(5), Some(5)), (Some(6), Some(6)), (Some(7), Some(7)), (Some(8), None), (Some(9), Some(8))]);
        assert_eq!(second[3].label(), "- 9 removed line: i");
        assert_eq!((second[3].cue(), second[0].cue()), (Some(crate::sound::Cue::Deleted), None));
        assert_eq!(change_header(&m.hunks, 1), "Change 2 of 3: line 9 original, no lines modified");
        let third = lines(2);
        assert_eq!(third.iter().map(ViewLine::label).collect::<Vec<_>>(), ["unchanged line 9: j", "+ 10 inserted line: k", "+ 11 inserted line: l"]);
        assert_eq!(third[1].cue(), Some(crate::sound::Cue::Inserted));
        assert_eq!(change_header(&m.hunks, 2), "Change 3 of 3: no lines original, lines 10–11 modified");
        assert!(lines(3).is_empty());
    }

    fn key(key: Key, shift: bool) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: if shift { egui::Modifiers::SHIFT } else { Default::default() } }
    }

    #[test]
    fn f7_opens_the_accessible_diff_viewer_and_enter_jumps_to_the_line() {
        let ctx = egui::Context::default();
        let dir = repo("viewer", "one\ntwo\nthree\nfour\n");
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\nfour\nfive\n").unwrap();
        let mut editors = Editors::new();
        assert!(editors.compare("a.txt (changes)", &strings(&["diff", "--", "a.txt"]), &dir, false, &ctx));
        settle(&ctx, &mut editors, "the diff tab", |e| e.active_tab().is_some_and(|(g, t)| e.groups[g].tabs[t].diff.is_some()));
        frame(&ctx, &mut editors, vec![]);
        let mut keys = serde_json::Map::new();
        editors.when_keys(&mut keys);
        command::set_context(keys);
        let viewer = |e: &Editors| active(e).0.diff.as_ref().unwrap().viewer;
        let press = |editors: &mut Editors, event: Event| {
            frame(&ctx, editors, vec![event]);
            frame(&ctx, editors, vec![]);
            frame(&ctx, editors, vec![]);
        };
        frame(&ctx, &mut editors, vec![key(Key::F7, false)]);
        assert_eq!(viewer(&editors).map(|v| v.change), Some(0));
        ctx.enable_accesskit();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))), focused: true, ..Default::default() };
        let (theme, git, settings) = (Theme::dark(), crate::git::Git::new(), Settings::default());
        let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut vec![]));
        let nodes = crate::a11y::tree(&mut out);
        let mut labels: Vec<&str> = nodes.iter().filter(|n| n.role() == ui::Role::ListItem).filter_map(|n| n.label()).filter(|l| l.contains(" line")).collect();
        labels.sort();
        assert_eq!(labels, ["+ 2 inserted line: TWO", "- 2 removed line: two", "unchanged line 1: one", "unchanged line 3: three", "unchanged line 4: four"]);
        let base = Id::new(("diff_viewer", "a.txt (changes)"));
        assert_eq!(ctx.memory(|m| m.focused()), Some(base.with(1)));
        frame(&ctx, &mut editors, vec![]);
        press(&mut editors, key(Key::ArrowDown, false));
        assert_eq!(viewer(&editors).and_then(|v| v.line), Some(2));
        assert_eq!(ctx.memory(|m| m.focused()), Some(base.with(2)));
        press(&mut editors, key(Key::F7, false));
        assert_eq!(viewer(&editors).map(|v| (v.change, v.line)), Some((1, Some(2))));
        press(&mut editors, key(Key::F7, true));
        assert_eq!(viewer(&editors).map(|v| (v.change, v.line)), Some((0, Some(1))));
        press(&mut editors, key(Key::F7, true));
        press(&mut editors, key(Key::Enter, false));
        assert!(viewer(&editors).is_none());
        assert_eq!(caret_line(&editors), 4);
        assert!(editors.has_focus());
        press(&mut editors, key(Key::F7, false));
        assert_eq!(viewer(&editors).map(|v| v.change), Some(1));
        press(&mut editors, key(Key::Escape, false));
        assert!(viewer(&editors).is_none() && editors.has_focus());
        command::set_context(Default::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_the_right_side_of_a_side_by_side_diff_and_peeks_from_the_gutter() {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let dir = repo("edit", "one\ntwo\nthree\nfour\n");
        std::fs::write(dir.join("a.txt"), "one\nthree\nfour\nfive\n").unwrap();
        let mut editors = Editors::new();
        editors.open(&dir.join("a.txt"));
        editors.finish_loads();
        let plain = editors.find_doc(&dir.join("a.txt")).unwrap();
        assert!(editors.compare("a.txt (changes)", &strings(&["diff", "--", "a.txt"]), &dir, false, &ctx));
        settle(&ctx, &mut editors, "the diff tab", |e| e.groups[0].tabs.len() == 2);
        let (tab, _) = active(&editors);
        assert_eq!(tab.doc, plain);
        assert_eq!(tab.rows.1, vec![0, 2, 3, 4, 5, 6]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Text("2".into())]);
        frame(&ctx, &mut editors, vec![]);
        let (tab, buf) = active(&editors);
        assert_eq!(buf.rope.to_string(), "one\n2three\nfour\nfive\n");
        assert!(buf.is_dirty() && tab.diff.as_ref().unwrap().computed == buf.version);
        assert_eq!(hunks(&editors), 2);
        editors.exec(&Command::ToggleInlineDiff, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(active(&editors).0.rows.1, vec![0, 3, 4, 5, 6, 7]);

        editors.groups[0].active = 0;
        editors.exec(&Command::GoTo(1, 1), &settings, &ctx);
        editors.exec(&Command::ShowNextChange, &settings, &ctx);
        settle(&ctx, &mut editors, "the change peek", |e| e.diffs.peek.is_some());
        assert_eq!(editors.diffs.peek, Some((plain, 0, 0)));
        frame(&ctx, &mut editors, vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert!(editors.diffs.peek.is_none());
        editors.exec(&Command::GoTo(2, 1), &settings, &ctx);
        editors.exec(&Command::RevertHunk, &settings, &ctx);
        assert_eq!(active(&editors).1.rope.to_string(), "one\ntwo\nthree\nfour\nfive\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
