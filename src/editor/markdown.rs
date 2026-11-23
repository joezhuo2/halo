use super::buffer::{Document, Sel};
use super::highlight::Highlighter;
use super::image_view;
use super::Editors;
use super::layout::Dir;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2, pos2, vec2};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ropey::Rope;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const DEBOUNCE: f64 = 0.2;
const CELL: Vec2 = vec2(12.0, 6.0);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text(String, Style),
    Break(bool),
    Image { url: String, alt: String },
    Note(String),
}

#[derive(Debug, PartialEq)]
pub struct Item {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, PartialEq)]
pub enum Kind {
    Heading(u8, String, Vec<Span>),
    Para(Vec<Span>),
    Code(String, String),
    Quote(Vec<Block>),
    List(Option<u64>, Vec<Item>),
    Rule,
    Table(Vec<Alignment>, Vec<Vec<Span>>, Vec<Vec<Vec<Span>>>),
    Note(String, Vec<Block>),
    Html(String),
}

#[derive(Debug, PartialEq)]
pub struct Block {
    pub line: usize,
    pub kind: Kind,
}

type Events<'a> = std::iter::Peekable<pulldown_cmark::OffsetIter<'a>>;

struct Parse {
    starts: Vec<usize>,
}

fn inline(tag: &Tag) -> bool {
    matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. })
}

pub fn plain(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Text(text, _) => text.as_str(),
            Span::Image { alt, .. } => alt.as_str(),
            Span::Break(_) => " ",
            Span::Note(_) => "",
        })
        .collect()
}

pub fn slug(text: &str) -> String {
    text.trim().to_lowercase().chars().filter_map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { Some(c) } else if c == ' ' { Some('-') } else { None }).collect()
}

impl Parse {
    fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|s| *s <= offset).saturating_sub(1)
    }

    fn blocks(&self, it: &mut Events) -> Vec<Block> {
        let mut out = vec![];
        while let Some((event, range)) = it.peek().cloned() {
            let line = self.line(range.start);
            match event {
                Event::End(_) => break,
                Event::Start(tag) if !inline(&tag) => {
                    it.next();
                    out.extend(self.container(tag, it).map(|kind| Block { line, kind }));
                }
                Event::Rule => {
                    it.next();
                    out.push(Block { line, kind: Kind::Rule });
                }
                Event::Html(text) => {
                    it.next();
                    out.push(Block { line, kind: Kind::Html(text.to_string()) });
                }
                _ => {
                    let spans = self.spans(it, Style::default(), true);
                    out.push(Block { line, kind: Kind::Para(spans) });
                }
            }
        }
        out
    }

    fn container(&self, tag: Tag, it: &mut Events) -> Option<Kind> {
        Some(match tag {
            Tag::Paragraph => Kind::Para(self.spans(it, Style::default(), false)),
            Tag::Heading { level, .. } => {
                let spans = self.spans(it, Style::default(), false);
                Kind::Heading(level as u8, slug(&plain(&spans)), spans)
            }
            Tag::BlockQuote(_) => {
                let blocks = self.blocks(it);
                it.next();
                Kind::Quote(blocks)
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or_default().to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                Kind::Code(lang, self.text(it))
            }
            Tag::HtmlBlock => Kind::Html(self.text(it)),
            Tag::List(first) => {
                let mut items = vec![];
                while let Some((Event::Start(Tag::Item), _)) = it.peek().cloned() {
                    it.next();
                    let task = match it.peek() {
                        Some((Event::TaskListMarker(done), _)) => Some(*done),
                        _ => None,
                    };
                    if task.is_some() {
                        it.next();
                    }
                    let blocks = self.blocks(it);
                    it.next();
                    items.push(Item { task, blocks });
                }
                it.next();
                Kind::List(first, items)
            }
            Tag::FootnoteDefinition(label) => {
                let blocks = self.blocks(it);
                it.next();
                Kind::Note(label.to_string(), blocks)
            }
            Tag::Table(aligns) => {
                let (mut head, mut rows) = (vec![], vec![]);
                while let Some((event, _)) = it.next() {
                    match event {
                        Event::Start(Tag::TableHead) => head = self.cells(it),
                        Event::Start(Tag::TableRow) => rows.push(self.cells(it)),
                        Event::End(TagEnd::Table) => break,
                        _ => {}
                    }
                }
                Kind::Table(aligns, head, rows)
            }
            _ => return None,
        })
    }

    fn text(&self, it: &mut Events) -> String {
        let mut text = String::new();
        for (event, _) in it.by_ref() {
            match event {
                Event::Text(t) | Event::Html(t) => text.push_str(&t),
                Event::End(_) => break,
                _ => {}
            }
        }
        text
    }

    fn cells(&self, it: &mut Events) -> Vec<Vec<Span>> {
        let mut cells = vec![];
        while let Some((event, _)) = it.next() {
            match event {
                Event::Start(Tag::TableCell) => cells.push(self.spans(it, Style::default(), false)),
                Event::End(_) => break,
                _ => {}
            }
        }
        cells
    }

    fn spans(&self, it: &mut Events, style: Style, tight: bool) -> Vec<Span> {
        let mut out = vec![];
        while let Some((event, _)) = it.peek().cloned() {
            match &event {
                Event::End(_) => {
                    if !tight {
                        it.next();
                    }
                    break;
                }
                Event::Start(tag) if !inline(tag) => break,
                Event::Rule => break,
                _ => {}
            }
            it.next();
            let styled = |change: &dyn Fn(&mut Style)| {
                let mut next = style.clone();
                change(&mut next);
                next
            };
            match event {
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => out.push(Span::Text(text.to_string(), style.clone())),
                Event::Code(text) => out.push(Span::Text(text.to_string(), styled(&|s| s.code = true))),
                Event::SoftBreak => out.push(Span::Break(false)),
                Event::HardBreak => out.push(Span::Break(true)),
                Event::FootnoteReference(label) => out.push(Span::Note(label.to_string())),
                Event::Start(Tag::Emphasis) => out.extend(self.spans(it, styled(&|s| s.italic = true), false)),
                Event::Start(Tag::Strong) => out.extend(self.spans(it, styled(&|s| s.bold = true), false)),
                Event::Start(Tag::Strikethrough) => out.extend(self.spans(it, styled(&|s| s.strike = true), false)),
                Event::Start(Tag::Link { dest_url, .. }) => out.extend(self.spans(it, styled(&|s| s.link = Some(dest_url.to_string())), false)),
                Event::Start(Tag::Image { dest_url, .. }) => {
                    let alt = plain(&self.spans(it, style.clone(), false));
                    out.push(Span::Image { url: dest_url.to_string(), alt });
                }
                _ => {}
            }
        }
        out
    }
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let options = Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut it = Parser::new_ext(text, options).into_offset_iter().peekable();
    let parse = Parse { starts };
    let mut out = vec![];
    while it.peek().is_some() {
        out.extend(parse.blocks(&mut it));
        it.next();
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum Link {
    Web(String),
    Anchor(String),
    File(PathBuf),
    None,
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => (out.push(byte), i += 3),
            (byte, _) => (out.push(byte), i += 1),
        };
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub fn resolve(dir: &Path, url: &str) -> Link {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        return Link::Web(url.to_owned());
    }
    if let Some(anchor) = url.strip_prefix('#') {
        return Link::Anchor(slug(&unescape(anchor)));
    }
    let scheme = url.split(['/', '\\']).next().unwrap_or_default();
    if scheme.contains(':') && scheme.len() > 2 {
        return Link::None;
    }
    let file = unescape(url.split(['#', '?']).next().unwrap_or_default());
    if file.is_empty() {
        return Link::None;
    }
    let path = Path::new(&file);
    Link::File(if path.is_absolute() { path.to_path_buf() } else { clean(&dir.join(path)) })
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|e| ["md", "markdown", "mdown", "mkd"].contains(&e.to_string_lossy().to_lowercase().as_str()))
}

pub fn y_for_line(marks: &[(usize, f32)], line: f32) -> f32 {
    let at = marks.partition_point(|m| m.0 as f32 <= line);
    let Some(before) = at.checked_sub(1).map(|i| marks[i]) else { return 0.0 };
    match marks[at..].iter().find(|m| m.0 > before.0) {
        Some(after) => before.1 + (after.1 - before.1) * (line - before.0 as f32) / (after.0 - before.0) as f32,
        None => before.1,
    }
}

fn columns(natural: &[f32], width: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by(|a, b| natural[*a].total_cmp(&natural[*b]));
    let (mut out, mut left) = (natural.to_vec(), width);
    for (n, &c) in order.iter().enumerate() {
        out[c] = natural[c].min(left / (order.len() - n) as f32);
        left -= out[c];
    }
    out
}

pub enum Out {
    Open(PathBuf),
    Preview(PathBuf),
    Reveal(usize),
}

enum Img {
    Loading(Receiver<Result<image_view::Decoded, String>>),
    Ready(TextureHandle, [u32; 2]),
    Failed,
}

#[derive(Default)]
pub struct MdTab {
    blocks: Vec<Block>,
    shown: Option<u64>,
    seen: u64,
    changed: f64,
    marks: Vec<(usize, f32)>,
    anchors: Vec<(String, f32)>,
    followed: Option<usize>,
    scroll_to: Option<f32>,
    images: HashMap<PathBuf, Img>,
    code: HashMap<usize, Vec<Vec<(usize, Color32)>>>,
    code_key: Option<[Color32; 4]>,
}

fn highlight(code: &str, lang: &str, theme: &Theme) -> Vec<Vec<(usize, Color32)>> {
    let rope = Rope::from_str(code);
    let mut hl = Highlighter::fenced(lang, &rope);
    hl.ensure(&rope, 0..rope.len_lines(), theme, Duration::from_millis(200));
    (0..rope.len_lines()).map(|line| hl.get(line).map(|spans| spans.to_vec()).unwrap_or_default()).collect()
}

struct Paint<'a> {
    theme: &'a Theme,
    size: f32,
    dir: &'a Path,
    top: f32,
    gap: f32,
    marks: Vec<(usize, f32)>,
    anchors: Vec<(String, f32)>,
    old: &'a [(String, f32)],
    images: &'a mut HashMap<PathBuf, Img>,
    code: &'a mut HashMap<usize, Vec<Vec<(usize, Color32)>>>,
    out: Vec<Out>,
    scroll_to: Option<f32>,
}

impl Paint<'_> {
    fn blocks(&mut self, ui: &mut Ui, blocks: &[Block]) {
        for block in blocks {
            self.marks.push((block.line, ui.cursor().top() - self.top));
            self.block(ui, block);
            ui.add_space(self.gap);
        }
    }

    fn block(&mut self, ui: &mut Ui, block: &Block) {
        let theme = self.theme;
        match &block.kind {
            Kind::Heading(level, anchor, spans) => {
                self.anchors.push((anchor.clone(), ui.cursor().top() - self.top));
                let size = self.size * [1.9, 1.6, 1.35, 1.18, 1.05, 1.0][(*level as usize).clamp(1, 6) - 1];
                let bold = spans.iter().map(|s| match s {
                    Span::Text(t, style) => Span::Text(t.clone(), Style { bold: true, ..style.clone() }),
                    other => other.clone(),
                });
                self.rich(ui, &bold.collect::<Vec<_>>(), size, theme.text, block.line);
                if *level <= 2 {
                    ui.add_space(3.0);
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().rect_filled(line, CornerRadius::ZERO, theme.border);
                }
            }
            Kind::Para(spans) => self.paragraph(ui, spans, block.line),
            Kind::Code(lang, text) => self.code_block(ui, lang, text, block.line),
            Kind::Quote(blocks) => {
                ui.indent(("md_quote", block.line), |ui| self.blocks(ui, blocks));
            }
            Kind::List(first, items) => {
                for (n, item) in items.iter().enumerate() {
                    let gap = std::mem::replace(&mut self.gap, 3.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(6.0);
                        let marker = match (item.task, first) {
                            (Some(true), _) => icons::CHECK_SQUARE.to_owned(),
                            (Some(false), _) => icons::SQUARE.to_owned(),
                            (None, Some(start)) => format!("{}.", start + n as u64),
                            (None, None) => "\u{2022}".to_owned(),
                        };
                        ui.label(RichText::new(marker).size(self.size).color(theme.text_dim));
                        ui.vertical(|ui| self.blocks(ui, &item.blocks));
                    });
                    self.gap = gap;
                    ui.add_space(2.0);
                }
            }
            Kind::Rule => {
                ui.add_space(6.0);
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, theme.border);
            }
            Kind::Table(aligns, head, rows) => self.table(ui, aligns, head, rows, block.line),
            Kind::Note(label, blocks) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(format!("[{label}]")).size(self.size * 0.9).color(theme.accent));
                    ui.vertical(|ui| self.blocks(ui, blocks));
                });
            }
            Kind::Html(text) => {
                let spans = [Span::Text(text.trim_end().to_owned(), Style { code: true, ..Style::default() })];
                self.rich(ui, &spans, self.size, theme.text_dim, block.line);
            }
        }
    }

    fn paragraph(&mut self, ui: &mut Ui, spans: &[Span], line: usize) {
        let mut run: Vec<Span> = vec![];
        for span in spans {
            if let Span::Image { url, alt } = span {
                if !run.is_empty() {
                    self.rich(ui, &std::mem::take(&mut run), self.size, self.theme.text, line);
                }
                self.image(ui, url, alt, line);
            } else {
                run.push(span.clone());
            }
        }
        if !run.is_empty() {
            self.rich(ui, &run, self.size, self.theme.text, line);
        }
    }

    fn table(&mut self, ui: &mut Ui, aligns: &[Alignment], head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], line: usize) {
        let theme = self.theme;
        let bold = |cell: &Vec<Span>| -> Vec<Span> {
            cell.iter().map(|s| match s {
                Span::Text(t, style) => Span::Text(t.clone(), Style { bold: true, ..style.clone() }),
                other => other.clone(),
            }).collect()
        };
        let head: Vec<Vec<Span>> = head.iter().map(bold).collect();
        let all: Vec<&[Vec<Span>]> = std::iter::once(head.as_slice()).chain(rows.iter().map(Vec::as_slice)).collect();
        let mut natural = vec![0.0f32; all.iter().map(|r| r.len()).max().unwrap_or(0)];
        for row in &all {
            for (c, cell) in row.iter().enumerate() {
                natural[c] = natural[c].max(self.natural(ui, cell) + 2.0 * CELL.x);
            }
        }
        let widths = columns(&natural, ui.available_width());
        let (left, start) = (ui.cursor().left(), ui.cursor().top());
        let (mut top, clip) = (start, ui.clip_rect());
        for (r, row) in all.iter().enumerate() {
            let bg = ui.painter().add(Shape::Noop);
            let (mut x, mut height, mut cells) = (left, self.size, vec![]);
            for (c, w) in widths.iter().enumerate() {
                if let Some(cell) = row.get(c) {
                    let align = match aligns.get(c) {
                        Some(Alignment::Center) => Align::Center,
                        Some(Alignment::Right) => Align::Max,
                        _ => Align::Min,
                    };
                    let inner = Rect::from_min_size(pos2(x, top) + CELL, vec2((w - 2.0 * CELL.x).max(1.0), f32::INFINITY));
                    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(align)));
                    child.set_clip_rect(Rect::from_x_y_ranges(x..=x + w, clip.y_range()).intersect(clip));
                    self.paragraph(&mut child, cell, line);
                    height = height.max(child.min_rect().height());
                }
                cells.push(x..=x + w);
                x += w;
            }
            let rect = Rect::from_min_size(pos2(left, top), vec2(x - left, height + 2.0 * CELL.y));
            let fill = match r {
                0 => theme.panel,
                r if r % 2 == 0 => ui::mix(theme.bg, theme.panel, 0.5),
                _ => Color32::TRANSPARENT,
            };
            let stroke = Stroke::new(1.0, theme.border);
            let mut shapes = vec![Shape::rect_filled(rect, CornerRadius::ZERO, fill)];
            shapes.extend(cells.into_iter().map(|xs| Shape::rect_stroke(Rect::from_x_y_ranges(xs, rect.y_range()), CornerRadius::ZERO, stroke, StrokeKind::Middle)));
            if r == 0 {
                shapes.push(Shape::line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, theme.border)));
            }
            ui.painter().set(bg, Shape::Vec(shapes));
            top = rect.bottom();
        }
        ui.allocate_rect(Rect::from_min_max(pos2(left, start), pos2(left + widths.iter().sum::<f32>(), top)), Sense::hover());
    }

    fn natural(&self, ui: &Ui, spans: &[Span]) -> f32 {
        let text: Vec<Span> = spans.iter().filter(|s| !matches!(s, Span::Image { .. })).cloned().collect();
        let images = spans.iter().filter_map(|s| match s {
            Span::Image { url, .. } => Some(match resolve(self.dir, url) {
                Link::File(path) => match self.images.get(&path) {
                    Some(Img::Ready(_, size)) => size[0] as f32,
                    _ => 32.0,
                },
                _ => 0.0,
            }),
            _ => None,
        });
        let (job, _) = self.job(&text, self.size, self.theme.text, f32::INFINITY);
        images.fold(ui.painter().layout_job(job).size().x, f32::max)
    }

    fn job(&self, spans: &[Span], size: f32, color: Color32, width: f32) -> (LayoutJob, Vec<(usize, usize, String)>) {
        let theme = self.theme;
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        let (mut links, mut chars) = (vec![], 0);
        for span in spans {
            let (text, style) = match span {
                Span::Text(text, style) => (text.clone(), style.clone()),
                Span::Break(hard) => (if *hard { "\n" } else { " " }.to_owned(), Style::default()),
                Span::Note(label) => (format!("[{label}]"), Style { link: Some(String::new()), ..Style::default() }),
                Span::Image { alt, .. } => (alt.clone(), Style::default()),
            };
            let shade = match (&style.link, style.code, style.bold) {
                (Some(_), _, _) => theme.accent,
                (_, true, _) => theme.syntax.string,
                (_, _, true) => ui::mix(color, theme.accent, 0.35),
                _ => color,
            };
            let font = if style.code { FontId::monospace(size * 0.92) } else { FontId::proportional(size) };
            let format = TextFormat {
                font_id: font,
                color: shade,
                italics: style.italic,
                strikethrough: if style.strike { Stroke::new(1.0, shade) } else { Stroke::NONE },
                underline: if style.link.is_some() { Stroke::new(1.0, shade) } else { Stroke::NONE },
                background: if style.code { theme.panel } else { Color32::TRANSPARENT },
                ..Default::default()
            };
            let count = text.chars().count();
            if let Some(url) = style.link {
                links.push((chars, chars + count, url));
            }
            chars += count;
            job.append(&text, 0.0, format);
        }
        (job, links)
    }

    fn rich(&mut self, ui: &mut Ui, spans: &[Span], size: f32, color: Color32, line: usize) {
        let (job, links) = self.job(spans, size, color, ui.available_width());
        let galley = ui.painter().layout_job(job);
        let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
        ui.painter().galley(rect.min, galley.clone(), color);
        let hit = resp.hover_pos().map(|pos| usize::from(galley.cursor_from_pos(pos - rect.min).index)).and_then(|at| links.iter().find(|l| l.0 <= at && at < l.1));
        if let Some((_, _, url)) = hit {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                let url = url.clone();
                self.follow(&url);
            }
        }
        if resp.double_clicked() {
            self.out.push(Out::Reveal(line));
        }
    }

    fn follow(&mut self, url: &str) {
        match resolve(self.dir, url) {
            Link::Web(url) => ui::open_url(&url),
            Link::Anchor(anchor) => self.scroll_to = self.old.iter().find(|a| a.0 == anchor).map(|a| a.1),
            Link::File(path) if is_markdown(&path) => self.out.push(Out::Preview(path)),
            Link::File(path) => self.out.push(Out::Open(path)),
            Link::None => {}
        }
    }

    fn code_block(&mut self, ui: &mut Ui, lang: &str, text: &str, line: usize) {
        let theme = self.theme;
        let lines = self.code.entry(line).or_insert_with(|| highlight(text, lang, theme));
        let font = FontId::monospace(self.size * 0.92);
        let mut job = LayoutJob::default();
        for (n, row) in text.split_inclusive('\n').enumerate() {
            let (mut at, spans) = (0, lines.get(n).map(Vec::as_slice).unwrap_or_default());
            for (len, color) in spans {
                let end = (at + len).min(row.len());
                if end > at && row.is_char_boundary(end) {
                    job.append(&row[at..end], 0.0, TextFormat::simple(font.clone(), *color));
                    at = end;
                }
            }
            if at < row.len() {
                job.append(&row[at..], 0.0, TextFormat::simple(font.clone(), theme.text));
            }
        }
        job.wrap.max_width = (ui.available_width() - 20.0).max(40.0);
        let galley = ui.painter().layout_job(job);
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, galley.size().y + 20.0), Sense::click());
        ui.painter().rect_filled(rect, CornerRadius::same(4), theme.panel);
        ui.painter().galley(rect.min + vec2(10.0, 10.0), galley, theme.text);
        if resp.double_clicked() {
            self.out.push(Out::Reveal(line));
        }
        let key = ui.id().with(("md_copy", line));
        let now = ui.input(|i| i.time);
        let since = ui.data(|d| d.get_temp::<f64>(key)).map(|t| now - t).filter(|s| *s < 1.5);
        if let Some(since) = since {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(1.5 - since));
        }
        let button = Rect::from_min_size(pos2(rect.right() - 30.0, rect.top() + 4.0), vec2(26.0, 26.0));
        ui.scope_builder(UiBuilder::new().max_rect(button).id_salt(key), |ui| {
            let focused = ui.memory(|m| m.has_focus(ui.next_auto_id()));
            ui.set_opacity(ui::anim(ui, key.with("shown"), focused || since.is_some() || ui.rect_contains_pointer(rect)));
            let icon = if since.is_some() { icons::CHECK } else { icons::COPY };
            if ui::icon_button(ui, theme, icon, 26.0, false, "Copy code").clicked() {
                ui.ctx().copy_text(text.strip_suffix('\n').unwrap_or(text).to_owned());
                ui.data_mut(|d| d.insert_temp(key, now));
            }
        });
    }

    fn image(&mut self, ui: &mut Ui, url: &str, alt: &str, line: usize) {
        let label = if alt.is_empty() { url } else { alt };
        let Link::File(path) = resolve(self.dir, url) else {
            let style = Style { link: Some(url.to_owned()), ..Style::default() };
            return self.rich(ui, &[Span::Text(label.to_owned(), style)], self.size, self.theme.text, line);
        };
        let entry = self.images.entry(path.clone()).or_insert_with(|| {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(image_view::decode(&path));
            });
            Img::Loading(rx)
        });
        if let Img::Loading(rx) = entry
            && let Ok(done) = rx.try_recv()
        {
            *entry = match done {
                Ok(done) => Img::Ready(ui.ctx().load_texture(url, done.image, TextureOptions::LINEAR), done.size),
                Err(_) => Img::Failed,
            };
        }
        match entry {
            Img::Ready(tex, size) => {
                let natural = vec2(size[0] as f32, size[1] as f32);
                let scale = (ui.available_width() / natural.x).min(1.0);
                ui.image((tex.id(), natural * scale));
            }
            Img::Loading(_) => {
                let (rect, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                ui::spinner(ui, rect.center(), 8.0, self.theme.text_dim);
            }
            Img::Failed => self.rich(ui, &[Span::Text(label.to_owned(), Style::default())], self.size, self.theme.text_dim, line),
        }
    }
}

impl MdTab {
    pub fn sync(&mut self, buf: &Document, now: f64) -> Option<f64> {
        if self.shown == Some(buf.version) {
            return None;
        }
        if self.shown.is_some() {
            if self.seen != buf.version {
                (self.seen, self.changed) = (buf.version, now);
            }
            let wait = DEBOUNCE - (now - self.changed);
            if wait > 0.0 {
                return Some(wait);
            }
        }
        self.blocks = parse(&buf.rope.to_string());
        self.shown = Some(buf.version);
        self.code.clear();
        None
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, buf: &Document, top: usize, size: f32) -> Vec<Out> {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if let Some(wait) = self.sync(buf, ui.input(|i| i.time)) {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
        }
        let key = [theme.text, theme.syntax.keyword, theme.syntax.string, theme.syntax.comment];
        if self.code_key.replace(key) != Some(key) {
            self.code.clear();
        }
        let mut area = ScrollArea::vertical().id_salt("markdown_preview").auto_shrink(false);
        if let Some(y) = self.scroll_to.take() {
            area = area.vertical_scroll_offset(y);
        } else if self.followed != Some(top) {
            self.followed = Some(top);
            area = area.vertical_scroll_offset(y_for_line(&self.marks, top as f32).max(0.0));
        }
        let dir = buf.path.as_deref().and_then(Path::parent).unwrap_or(Path::new("")).to_path_buf();
        let blocks = std::mem::take(&mut self.blocks);
        let mut paint = Paint {
            theme,
            size,
            dir: &dir,
            top: 0.0,
            gap: 8.0,
            marks: vec![],
            anchors: vec![],
            old: &self.anchors,
            images: &mut self.images,
            code: &mut self.code,
            out: vec![],
            scroll_to: None,
        };
        let pad = ((rect.width() - 860.0) / 2.0).max(24.0);
        let column = Rect::from_min_max(pos2(rect.left() + pad, rect.top()), pos2(rect.right() - pad, rect.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(column), |ui| {
            area.show(ui, |ui| {
                ui.add_space(20.0);
                paint.top = ui.min_rect().top();
                paint.blocks(ui, &blocks);
                ui.add_space(40.0);
            });
        });
        let (marks, anchors, scroll_to, out) = (paint.marks, paint.anchors, paint.scroll_to, paint.out);
        (self.marks, self.anchors, self.blocks) = (marks, anchors, blocks);
        if scroll_to.is_some() {
            self.scroll_to = scroll_to;
            ui.ctx().request_repaint();
        }
        out
    }
}

impl Editors {
    pub(super) fn open_preview(&mut self, side: bool) {
        let Some((g, t)) = self.active_tab() else { return };
        let doc = self.groups[g].tabs[t].doc;
        let markdown = self.docs.get(&doc).is_some_and(|d| d.buf.path.as_deref().is_some_and(is_markdown));
        if self.groups[g].tabs[t].md.is_some() || !markdown {
            return;
        }
        let to = match side {
            true => self.neighbor(g, Dir::Right).unwrap_or_else(|| self.split_group(g, Dir::Right)),
            false => g,
        };
        self.show_preview(doc, to);
    }

    fn show_preview(&mut self, doc: u64, group: usize) {
        self.active = group.min(self.groups.len() - 1);
        match self.groups[self.active].tabs.iter().position(|t| t.doc == doc && t.md.is_some()) {
            Some(t) => {
                self.groups[self.active].active = t;
                self.want_focus = true;
            }
            None => {
                self.add_tab(doc, vec![Sel::caret(0)]);
                self.groups[self.active].tabs.last_mut().into_iter().for_each(|tab| tab.md = Some(Default::default()));
            }
        }
    }

    pub(super) fn after_previews(&mut self) {
        for (g, doc, out) in std::mem::take(&mut self.previews) {
            match out {
                Out::Open(path) => {
                    self.active = g.min(self.groups.len() - 1);
                    self.open(&path);
                }
                Out::Preview(path) => {
                    let id = self.find_doc(&path).unwrap_or_else(|| self.insert_pending(&path));
                    self.show_preview(id, g);
                }
                Out::Reveal(line) => {
                    let Some(at) = self.docs.get(&doc).map(|d| d.buf.line_start(line)) else { continue };
                    for tab in self.groups.iter_mut().flat_map(|g| &mut g.tabs).filter(|t| t.doc == doc && t.md.is_none() && t.diff.is_none()) {
                        tab.sels = vec![Sel::caret(at)];
                        tab.reveal = Some(at);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        plain(spans)
    }

    fn only(source: &str) -> Block {
        let mut blocks = parse(source);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        blocks.remove(0)
    }

    #[test]
    fn parses_headings_inline_styles_links_and_breaks() {
        let blocks = parse("# Hello *World*\n\nSome **bold** ~~gone~~ `code` [site](https://a.b/c) and ![alt](img.png)  \nnext\\\nline\n");
        let Kind::Heading(1, anchor, spans) = &blocks[0].kind else { panic!("{blocks:?}") };
        assert_eq!((anchor.as_str(), text(spans).as_str()), ("hello-world", "Hello World"));
        assert!(matches!(&spans[1], Span::Text(t, s) if t == "World" && s.italic));
        let Kind::Para(spans) = &blocks[1].kind else { panic!() };
        assert_eq!(blocks[1].line, 2);
        assert!(spans.iter().any(|s| matches!(s, Span::Text(t, st) if t == "bold" && st.bold)));
        assert!(spans.iter().any(|s| matches!(s, Span::Text(t, st) if t == "gone" && st.strike)));
        assert!(spans.iter().any(|s| matches!(s, Span::Text(t, st) if t == "code" && st.code)));
        assert!(spans.iter().any(|s| matches!(s, Span::Text(t, st) if t == "site" && st.link.as_deref() == Some("https://a.b/c"))));
        assert!(spans.iter().any(|s| matches!(s, Span::Image { url, alt } if url == "img.png" && alt == "alt")));
        assert_eq!(spans.iter().filter(|s| matches!(s, Span::Break(true))).count(), 2);
    }

    #[test]
    fn parses_nested_and_task_lists() {
        let Kind::List(None, items) = only("- one\n  - inner\n- [x] done\n- [ ] todo\n").kind else { panic!() };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].task, None);
        assert!(matches!(&items[0].blocks[0].kind, Kind::Para(s) if text(s) == "one"));
        assert!(matches!(&items[0].blocks[1].kind, Kind::List(None, inner) if inner.len() == 1));
        assert_eq!((items[1].task, items[2].task), (Some(true), Some(false)));
        let Kind::List(Some(3), items) = only("3. a\n4. b\n").kind else { panic!() };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn parses_code_quotes_rules_html_and_footnotes() {
        assert_eq!(only("```rust\nfn a() {}\n```\n").kind, Kind::Code("rust".into(), "fn a() {}\n".into()));
        assert_eq!(only("    indented\n").kind, Kind::Code(String::new(), "indented\n".into()));
        let Kind::Quote(inner) = only("> quoted\n> > deeper\n").kind else { panic!() };
        assert!(matches!(&inner[0].kind, Kind::Para(s) if text(s) == "quoted"));
        assert!(matches!(&inner[1].kind, Kind::Quote(_)));
        assert_eq!(only("---\n").kind, Kind::Rule);
        assert!(matches!(only("<div>raw</div>\n").kind, Kind::Html(h) if h.contains("<div>")));
        let blocks = parse("Text[^1]\n\n[^1]: The note\n");
        assert!(matches!(&blocks[0].kind, Kind::Para(s) if s.contains(&Span::Note("1".into()))));
        assert!(matches!(&blocks[1].kind, Kind::Note(label, inner) if label == "1" && inner.len() == 1));
    }

    #[test]
    fn parses_tables_with_alignment() {
        let Kind::Table(aligns, head, rows) = only("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n| x | y | z |\n").kind else { panic!() };
        assert_eq!(aligns, [Alignment::Left, Alignment::Center, Alignment::Right]);
        assert_eq!(head.iter().map(|c| text(c)).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(text(&rows[1][2]), "z");
    }

    #[test]
    fn maps_blocks_to_source_lines() {
        let blocks = parse("# One\n\npara\n\n- a\n- b\n\n```\ncode\n```\n");
        assert_eq!(blocks.iter().map(|b| b.line).collect::<Vec<_>>(), [0, 2, 4, 7]);
        let marks = [(0, 0.0), (2, 100.0), (6, 300.0)];
        assert_eq!(y_for_line(&marks, 0.0), 0.0);
        assert_eq!(y_for_line(&marks, 1.0), 50.0);
        assert_eq!(y_for_line(&marks, 4.0), 200.0);
        assert_eq!(y_for_line(&marks, 99.0), 300.0);
        assert_eq!(y_for_line(&[], 5.0), 0.0);
    }

    #[test]
    fn resolves_links() {
        let dir = Path::new("docs/guide");
        assert_eq!(resolve(dir, "https://x.y/z"), Link::Web("https://x.y/z".into()));
        assert_eq!(resolve(dir, "#My Heading!"), Link::Anchor("my-heading".into()));
        assert_eq!(resolve(dir, "../img/a%20b.png#top"), Link::File(PathBuf::from("docs/img/a b.png")));
        assert_eq!(resolve(dir, "./other.md"), Link::File(PathBuf::from("docs/guide/other.md")));
        assert_eq!(resolve(dir, "mailto:a@b.c"), Link::None);
        assert_eq!(resolve(dir, ""), Link::None);
        assert!(is_markdown(Path::new("a/README.MD")) && !is_markdown(Path::new("a.txt")));
    }

    #[test]
    fn debounces_edits_and_parses_the_first_time_at_once() {
        let mut buf = Document::from_text("# a\n", "a.md".into());
        let mut tab = MdTab::default();
        assert_eq!(tab.sync(&buf, 0.0), None);
        assert_eq!(tab.blocks.len(), 1);
        buf.apply(&mut vec![super::super::buffer::Sel::caret(3)], vec![(3, 3, "b".into())], super::super::buffer::Kind::Type, super::super::buffer::After::Carets);
        assert!(tab.sync(&buf, 1.0).is_some_and(|w| w > 0.0));
        assert!(tab.sync(&buf, 1.1).is_some());
        assert!(matches!(&tab.blocks[0].kind, Kind::Heading(_, _, s) if text(s) == "a"));
        assert_eq!(tab.sync(&buf, 1.3), None);
        assert!(matches!(&tab.blocks[0].kind, Kind::Heading(_, _, s) if text(s) == "ab"));
    }

    #[test]
    fn renders_every_construct_without_panicking() {
        let source = "# Title\n\npara **b** *i* `c` [l](#title) ![x](https://e/x.png)\n\n- [x] a\n  1. n\n\n> q\n\n```rust\nfn a() {}\n```\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\n---\n\nnote[^1]\n\n[^1]: text\n\n<b>raw</b>\n";
        let ctx = egui::Context::default();
        let (theme, buf, mut tab) = (Theme::dark(), Document::from_text(source, "a.md".into()), MdTab::default());
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                tab.ui(ui, &theme, Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0)), &buf, 0, 14.0);
            });
            out.textures_delta.clear();
        }
        assert!(tab.marks.len() >= 8 && tab.anchors.iter().any(|a| a.0 == "title"));
        assert!(tab.marks.windows(2).all(|w| w[0].1 <= w[1].1));
    }

    #[test]
    fn sizes_table_columns_to_content_up_to_a_fair_share() {
        assert_eq!(columns(&[40.0, 100.0], 500.0), [40.0, 100.0]);
        assert_eq!(columns(&[40.0, 900.0, 600.0], 500.0), [40.0, 230.0, 230.0]);
        assert_eq!(columns(&[300.0, 300.0], 200.0), [100.0, 100.0]);
        assert!(columns(&[], 100.0).is_empty());
    }

    fn texts(shapes: &[egui::Shape], out: &mut Vec<(String, Rect, usize)>, rects: &mut Vec<Rect>) {
        for shape in shapes {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.job.text.clone(), t.galley.rect.translate(t.pos.to_vec2()), t.galley.rows.len())),
                egui::Shape::Rect(r) => rects.push(r.rect),
                egui::Shape::Vec(inner) => texts(inner, out, rects),
                _ => {}
            }
        }
    }

    #[test]
    fn lays_tables_out_inside_the_preview_at_any_width() {
        let long = "a long description that keeps going well past any narrow preview width so it has to wrap";
        let source = format!("| id | description | n |\n|:--|:--|--:|\n| 1 | {long} | **7** |\n| 2 | `short` | 8 |\n");
        for width in [320.0, 1000.0] {
            let ctx = egui::Context::default();
            let (theme, buf, mut tab) = (Theme::dark(), Document::from_text(&source, "a.md".into()), MdTab::default());
            let preview = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0));
            let (mut found, mut rects) = (vec![], vec![]);
            for _ in 0..2 {
                let input = egui::RawInput { screen_rect: Some(preview), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    tab.ui(ui, &theme, preview, &buf, 0, 14.0);
                });
                out.textures_delta.clear();
                (found, rects) = (vec![], vec![]);
                texts(&out.shapes.into_iter().map(|c| c.shape).collect::<Vec<_>>(), &mut found, &mut rects);
            }
            for rect in found.iter().map(|f| f.1).chain(rects) {
                assert!(rect.left() >= preview.left() - 0.5 && rect.right() <= preview.right() + 0.5, "{width}: {rect:?}");
            }
            let at = |text: &str| found.iter().find(|f| f.0 == text).unwrap_or_else(|| panic!("{text} in {found:?}"));
            let (id, desc, n) = (at("id").1.left(), at("description").1.left(), at("n").1.right());
            assert!(desc - id < n - desc, "{width}: {id} {desc} {n}");
            assert_eq!(at(long).2 > 1, width < 900.0);
            assert!(at("7").1.right() > at("n").1.right() - 1.0);
        }
    }

    #[test]
    fn ctrl_shift_v_opens_the_preview_in_markdown_and_pastes_elsewhere() {
        let dir = std::env::temp_dir().join(format!("halo-md-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (md, rs) = (dir.join("a.md"), dir.join("b.rs"));
        std::fs::write(&md, "# A\n").unwrap();
        std::fs::write(&rs, "fn a() {}\n").unwrap();
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        let theme = Theme::dark();
        let (git, settings) = (crate::git::Git::new(), crate::settings::Settings::default());
        let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, command: true, ..Default::default() };
        let release = egui::Event::Key { key: egui::Key::V, physical_key: Some(egui::Key::V), pressed: false, repeat: false, modifiers: ctrl_shift };
        let run = |editors: &mut Editors, events: Vec<egui::Event>| {
            let mut keys = serde_json::Map::new();
            editors.when_keys(&mut keys);
            crate::command::set_context(keys);
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))), focused: true, events, ..Default::default() };
            let mut cmds = vec![];
            let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut cmds));
            out.textures_delta.clear();
        };
        for (path, before) in [(&md, "# A\n"), (&rs, "fn a() {}\n")] {
            editors.open(path);
            editors.finish_loads();
            run(&mut editors, vec![]);
            run(&mut editors, vec![]);
            run(&mut editors, vec![egui::Event::ModifiersChanged(ctrl_shift), egui::Event::Paste("pasted".into())]);
            run(&mut editors, vec![release.clone(), egui::Event::ModifiersChanged(Default::default())]);
            let doc = editors.find_doc(path).unwrap();
            let text = editors.docs[&doc].buf.rope.to_string();
            let markdown = path == &md;
            assert_eq!(editors.groups.iter().flat_map(|g| &g.tabs).any(|t| t.doc == doc && t.md.is_some()), markdown, "{path:?}");
            assert_eq!(text == before, markdown, "{path:?} {text:?}");
        }
        crate::command::set_context(Default::default());
        drop(editors);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn frame(ctx: &egui::Context, editors: &mut Editors) {
        let (theme, git, settings) = (Theme::dark(), crate::git::Git::new(), crate::settings::Settings::default());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))), focused: true, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut vec![]));
        out.textures_delta.clear();
    }

    #[test]
    fn opens_previews_in_place_and_to_the_side_and_scrolls_the_source() {
        use crate::command::Command;
        let dir = std::env::temp_dir().join(format!("halo-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.md"), dir.join("b.md"));
        std::fs::write(&a, "# A

[b](b.md)

text

more
").unwrap();
        std::fs::write(&b, "# B
").unwrap();
        let (ctx, settings) = (egui::Context::default(), crate::settings::Settings::default());
        let mut editors = Editors::new();
        editors.open(&a);
        editors.finish_loads();
        editors.exec(&Command::MarkdownPreview, &settings, &ctx);
        editors.exec(&Command::MarkdownPreview, &settings, &ctx);
        assert_eq!((editors.groups.len(), editors.groups[0].tabs.len()), (1, 2));
        assert!(editors.groups[0].tabs[0].md.is_none() && editors.groups[0].tabs[1].md.is_some());
        editors.groups[0].active = 0;
        editors.exec(&Command::MarkdownPreviewSide, &settings, &ctx);
        assert_eq!(editors.groups.len(), 2);
        assert!(editors.groups[1].tabs[0].md.is_some());
        assert_eq!(editors.pinned_paths(), vec![]);
        frame(&ctx, &mut editors);
        editors.exec(&Command::Paste, &settings, &ctx);
        editors.previews.push((1, editors.groups[1].tabs[0].doc, Out::Reveal(4)));
        editors.after_previews();
        assert_eq!(editors.groups[0].tabs[0].sels[0].head, 16);
        editors.previews.push((1, editors.groups[1].tabs[0].doc, Out::Preview(b.clone())));
        editors.after_previews();
        editors.finish_loads();
        frame(&ctx, &mut editors);
        assert_eq!(editors.groups[1].tabs.len(), 2);
        assert!(editors.groups[1].tabs[1].md.is_some() && editors.docs.len() == 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
