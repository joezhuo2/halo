use super::buffer::{After, Document, Kind, Sel, is_word};
use super::typography::FontStyle;
use super::{Editors, Tab};
use crate::command::Command;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::TextFormat;
use eframe::egui::{self, Align2, CornerRadius, Event, FontId, Key, Pos2, Rect, Stroke, Ui, pos2};
use ropey::Rope;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const BEFORE: usize = 6000;
const AFTER: usize = 2000;
const WINDOW: usize = 30;
const PAUSE: f64 = 0.6;

#[derive(Default)]
pub(super) struct Ai {
    job: Option<Job>,
    ghost: Option<Ghost>,
    edit: Option<NextEdit>,
    seen: (u64, u64),
    due: bool,
    base: Option<(u64, Rope)>,
    recent: Vec<(u64, String, String)>,
    failed: Option<String>,
}

struct Job {
    rx: Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
    doc: u64,
    version: u64,
    at: usize,
    edit: bool,
    manual: bool,
    before: String,
    after: String,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Ghost {
    doc: u64,
    version: u64,
    at: usize,
    len: usize,
    text: String,
}

struct NextEdit {
    doc: u64,
    version: u64,
    start: usize,
    end: usize,
    new: String,
}

pub(super) struct Shown {
    pub line: usize,
    pub col: usize,
    pub first: String,
    pub below: Vec<String>,
    pub arrow: Option<usize>,
    strike: Option<(usize, usize)>,
    cursor_line: usize,
    busy: bool,
}

fn start_job(dir: PathBuf, prompt: String, ctx: &egui::Context) -> (Receiver<Result<String, String>>, Arc<AtomicBool>) {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (flag, ctx) = (cancel.clone(), ctx.clone());
    std::thread::spawn(move || {
        let result = crate::commit_ai::invocation(&dir).and_then(|(program, mut args)| {
            if args.last().is_some_and(|a| a == "-p") {
                args.extend(["--model".into(), "haiku".into()]);
            }
            crate::commit_ai::run(&dir, (program, args), &prompt, &flag)
        });
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    (rx, cancel)
}

pub(super) fn tail(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut cut = text.len() - budget;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    let rest = &text[cut..];
    rest.find('\n').map_or(rest, |i| &rest[i + 1..])
}

pub(super) fn head(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut cut = budget;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let part = &text[..cut];
    part.rfind('\n').map_or(part, |i| &part[..i + 1])
}

fn completion_prompt(language: &str, path: &str, before: &str, after: &str) -> String {
    format!(
        "You are a code completion engine. Continue the {language} file {path} at the <CURSOR> mark. Reply with only the text to insert \
        at the cursor, wrapped in <insert></insert>. Do not repeat text that is already before or after the cursor. Keep it short: finish \
        the current statement or block, at most about 10 lines. If nothing useful fits, reply <insert></insert>.\n\n{before}<CURSOR>{after}"
    )
}

fn edit_prompt(language: &str, path: &str, recent: &[(&str, &str)], window: &str) -> String {
    let mut prompt = format!("You predict the next edit a programmer will make in the {language} file {path}. Their recent edits, oldest first:\n\n");
    for (before, after) in recent {
        prompt.push_str(&format!("Before:\n{before}\nAfter:\n{after}\n\n"));
    }
    prompt.push_str(&format!(
        "The file around the cursor (<CURSOR> marks the cursor and is not part of the file):\n\n{window}\n\n\
        If a further edit clearly follows from the recent ones, such as the same change somewhere else or code that the change affects, reply \
        with exactly this and nothing else:\n<<<<<<< OLD\nthe current lines to change, copied exactly from the file, enough of them to be unique\n\
        =======\nthe same lines after the edit\n>>>>>>> NEW\nOtherwise reply NONE."
    ));
    prompt
}

fn strip_echo(before: &str, reply: &str) -> String {
    let starts: Vec<usize> = std::iter::once(0).chain(before.match_indices('\n').map(|(i, _)| i + 1)).collect();
    for &start in &starts[starts.len().saturating_sub(30)..] {
        for echo in [&before[start..], before[start..].trim_start()] {
            if echo.chars().any(char::is_alphanumeric) && let Some(rest) = reply.strip_prefix(echo) {
                return rest.to_owned();
            }
        }
    }
    reply.to_owned()
}

pub(super) fn insertion(reply: &str, before: &str, after: &str) -> String {
    let reply = reply.replace("\r\n", "\n");
    let inner = match (reply.find("<insert>"), reply.rfind("</insert>")) {
        (Some(a), Some(b)) if b >= a + 8 => &reply[a + 8..b],
        (Some(a), None) => &reply[a + 8..],
        _ => reply.as_str(),
    };
    let text = strip_echo(before, inner);
    let rest = after.lines().next().unwrap_or_default().trim();
    match text.trim_end().strip_suffix(rest) {
        Some(kept) if !rest.is_empty() => kept.to_owned(),
        _ => text,
    }
}

pub(super) fn next_word(text: &str) -> usize {
    let lead = text.len() - text.trim_start().len();
    let rest = &text[lead..];
    let Some(first) = rest.chars().next() else { return text.len() };
    let word = is_word(first);
    lead + rest.find(|c: char| c.is_whitespace() || is_word(c) != word).unwrap_or(rest.len())
}

pub(super) fn parse_edit(reply: &str) -> Option<(String, String)> {
    let reply = reply.replace("\r\n", "\n");
    let (_, rest) = reply.split_once("<<<<<<< OLD\n")?;
    let (old, rest) = rest.split_once("\n=======\n")?;
    let new = match rest.strip_prefix(">>>>>>> NEW") {
        Some(_) => "",
        None => rest.split_once("\n>>>>>>> NEW")?.0,
    };
    (!old.trim().is_empty() && old != new).then(|| (old.to_owned(), new.to_owned()))
}

pub(super) fn locate(text: &str, old: &str, new: &str) -> Option<(usize, usize, String)> {
    let mut found = text.match_indices(old).map(|(i, _)| i);
    let at = found.next()?;
    if found.next().is_some() {
        return None;
    }
    let lead = common(old.chars(), new.chars());
    let trail = common(old[lead..].chars().rev(), new[lead..].chars().rev());
    Some((at + lead, at + old.len() - trail, new[lead..new.len() - trail].to_owned()))
}

fn common(a: impl Iterator<Item = char>, b: impl Iterator<Item = char>) -> usize {
    a.zip(b).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum()
}

pub(super) fn changed(old: &str, new: &str) -> Option<(String, String)> {
    let start = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    if start == old.len() && start == new.len() {
        return None;
    }
    let room = old.len().min(new.len()) - start;
    let end = old.bytes().rev().zip(new.bytes().rev()).take(room).take_while(|(a, b)| a == b).count();
    let lines = |s: &str| {
        let to = s.len() - end;
        let from = s.as_bytes()[..start].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let until = s.as_bytes()[to..].iter().position(|b| *b == b'\n').map_or(s.len(), |i| to + i);
        s[from..until].lines().take(8).collect::<Vec<_>>().join("\n")
    };
    Some((lines(old), lines(new)))
}

fn replace(buf: &mut Document, sels: &mut Vec<Sel>, start: usize, end: usize, text: &str) -> bool {
    buf.apply(sels, vec![(start, end, text.to_owned())], Kind::Other, After::Carets)
}

pub(super) fn ghost_format(font: &FontId, theme: &Theme) -> TextFormat {
    TextFormat { font_id: font.clone(), color: ui::with_alpha(theme.text_dim, 0.85), ..Default::default() }
}

pub(super) fn pad(pads: &mut Vec<(usize, u32)>, shown: Option<&Shown>) {
    let Some(s) = shown.filter(|s| !s.below.is_empty()) else { return };
    match pads.binary_search_by_key(&(s.line + 1), |p| p.0) {
        Ok(at) => pads[at].1 += s.below.len() as u32,
        Err(at) => pads.insert(at, (s.line + 1, s.below.len() as u32)),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_line(
    ui: &Ui,
    theme: &Theme,
    s: &Shown,
    line: usize,
    (start, len, bottom): (usize, usize, f32),
    (first, last, line_h): (usize, usize, f32),
    at: &dyn Fn(usize) -> Pos2,
    spans: &dyn Fn(usize, usize) -> Vec<(f32, f32, f32)>,
    (font, style): (&FontId, FontStyle),
) {
    let painter = ui.painter();
    if let Some((a, b)) = s.strike.filter(|(a, b)| *b > start && *a <= start + len) {
        for (x0, x1, y) in spans(a.saturating_sub(start), (b - start).min(len)) {
            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h)), CornerRadius::same(2), ui::with_alpha(theme.deleted, 0.25));
            painter.hline(x0..=x1, y + line_h / 2.0, Stroke::new(1.0, theme.deleted));
        }
    }
    if line == s.line && !s.below.is_empty() {
        let color = if s.strike.is_some() { theme.text } else { ui::with_alpha(theme.text_dim, 0.85) };
        let galleys: Vec<_> = s.below.iter().map(|text| painter.layout_job(style.apply(super::job(text, 0..text.len(), None, font, color, &[], &[], &TextFormat::default())))).collect();
        let x = at(0).x;
        if s.strike.is_some() {
            let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
            let r = Rect::from_min_size(pos2(x - 3.0, bottom), egui::vec2(width + 8.0, galleys.len() as f32 * line_h));
            painter.rect(r, CornerRadius::same(3), ui::with_alpha(theme.added, 0.15), Stroke::new(1.0, ui::with_alpha(theme.added, 0.6)), egui::StrokeKind::Inside);
        }
        for (i, galley) in galleys.into_iter().enumerate() {
            painter.galley(pos2(x, bottom + i as f32 * line_h + (line_h - galley.size().y) / 2.0), galley, color);
        }
    }
    if line == s.cursor_line {
        let end = at(len);
        let mid = end.y + line_h / 2.0;
        if s.busy {
            ui::spinner(ui, pos2(end.x + 14.0, mid), (line_h * 0.22).max(3.0), theme.text_dim);
        } else if let Some(target) = s.arrow.filter(|l| !(first..last).contains(l)) {
            let arrow = if target < line { icons::ARROW_UP } else { icons::ARROW_DOWN };
            let label = format!("{arrow} Tab to jump to the suggested edit");
            painter.text(pos2(end.x + 16.0, mid), Align2::LEFT_CENTER, label, FontId::proportional((font.size * 0.9).round()), theme.accent);
        }
    }
}

impl Ai {
    fn dismiss(&mut self) {
        (self.job, self.ghost, self.edit) = (None, None, None);
    }

    pub(super) fn shown(&mut self, doc: u64, buf: &Document, sels: &[Sel]) -> Option<Shown> {
        let caret = match sels {
            [s] if s.is_empty() => Some(s.head),
            _ => None,
        };
        let mut keep = true;
        if let Some(g) = self.ghost.as_mut().filter(|g| g.doc == doc && g.version != buf.version) {
            let typed = buf.len().saturating_sub(g.len);
            let text = buf.text_of(g.at, g.at + typed);
            keep = typed > 0 && caret == Some(g.at + typed) && g.text.len() > text.len() && g.text.starts_with(&text);
            if keep {
                g.text.drain(..text.len());
                (g.at, g.version, g.len) = (g.at + typed, buf.version, buf.len());
            }
        }
        if !keep || self.ghost.as_ref().is_some_and(|g| g.doc == doc && caret != Some(g.at)) {
            self.ghost = None;
        }
        let cursor_line = buf.line_of(sels.first()?.head);
        if let Some(g) = self.ghost.as_ref().filter(|g| g.doc == doc) {
            let mut lines = g.text.split('\n');
            let line = buf.line_of(g.at);
            let first = lines.next().unwrap_or_default().to_owned();
            let below = lines.map(str::to_owned).collect();
            return Some(Shown { line, col: g.at - buf.line_start(line), first, below, arrow: None, strike: None, cursor_line, busy: false });
        }
        let busy = self.job.as_ref().is_some_and(|j| j.doc == doc);
        if let Some(e) = self.edit.as_ref().filter(|e| e.doc == doc && e.version == buf.version) {
            let (top, line) = (buf.line_of(e.start), buf.line_of(e.end));
            let result = format!("{}{}{}", buf.text_of(buf.line_start(top), e.start), e.new, buf.text_of(e.end, buf.line_end(line)));
            let below = if result.is_empty() { vec![] } else { result.split('\n').map(str::to_owned).collect() };
            return Some(Shown { line, col: 0, first: String::new(), below, arrow: Some(top), strike: Some((e.start, e.end)), cursor_line, busy });
        }
        busy.then(|| Shown { line: cursor_line, col: 0, first: String::new(), below: vec![], arrow: None, strike: None, cursor_line, busy })
    }

    fn record(&mut self, doc: u64, buf: &Document) {
        if let Some((_, base)) = self.base.as_ref().filter(|b| b.0 == doc)
            && let Some((before, after)) = changed(&base.to_string(), &buf.rope.to_string())
        {
            self.recent.push((doc, before, after));
            let extra = self.recent.len().saturating_sub(12);
            self.recent.drain(..extra);
        }
        self.base = Some((doc, buf.rope.clone()));
    }
}

impl Editors {
    pub(super) fn ai_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        let edit = match cmd {
            Command::TriggerInlineSuggestion => false,
            Command::SuggestNextEdit => true,
            _ => return false,
        };
        let Some((g, t)) = self.active_tab() else { return false };
        if !self.docs.get(&self.groups[g].tabs[t].doc).is_some_and(|d| d.content.is_text()) {
            return false;
        }
        self.ai.dismiss();
        self.ai_start(edit, true, ctx);
        self.want_focus = true;
        true
    }

    fn ai_start(&mut self, edit: bool, manual: bool, ctx: &egui::Context) {
        let Some((g, t)) = self.active_tab() else { return };
        let tab = &self.groups[g].tabs[t];
        let Some(doc) = self.docs.get(&tab.doc) else { return };
        let buf = &doc.buf;
        let at = match tab.sels[..] {
            [s] if s.is_empty() && !buf.read_only && !buf.large => s.head,
            _ => {
                if manual {
                    ui::notify_info("AI suggestions need a single cursor in an editable file.");
                }
                return;
            }
        };
        let path = buf.path.as_ref().map_or(buf.title.clone(), |p| p.display().to_string());
        let language = &doc.hl.language;
        let before = tail(&buf.text_of(at.saturating_sub(BEFORE), at), BEFORE).to_owned();
        let after = head(&buf.text_of(at, (at + AFTER).min(buf.len())), AFTER).to_owned();
        let prompt = match edit {
            true => {
                self.ai.record(tab.doc, buf);
                let line = buf.line_of(at);
                let (from, to) = (buf.line_start(line.saturating_sub(WINDOW)), buf.line_end((line + WINDOW).min(buf.lines() - 1)));
                let window = format!("{}<CURSOR>{}", buf.text_of(from, at), buf.text_of(at, to));
                let recent: Vec<(&str, &str)> = self.ai.recent.iter().filter(|r| r.0 == tab.doc).map(|r| (r.1.as_str(), r.2.as_str())).collect();
                edit_prompt(language, &path, &recent[recent.len().saturating_sub(5)..], &window)
            }
            false => completion_prompt(language, &path, &before, &after),
        };
        let dir = buf.path.as_deref().and_then(Path::parent).map(Path::to_path_buf).or_else(|| self.root.clone()).unwrap_or_else(std::env::temp_dir);
        let (rx, cancel) = start_job(dir, prompt, ctx);
        self.ai.job = Some(Job { rx, cancel, doc: tab.doc, version: buf.version, at, edit, manual, before, after });
    }

    pub(super) fn ai_poll(&mut self, settings: &Settings, ctx: &egui::Context) {
        let Some((g, t)) = self.active_tab() else { return };
        let tab = &self.groups[g].tabs[t];
        let id = tab.doc;
        let Some(doc) = self.docs.get(&id).filter(|d| d.content.is_text() && tab.diff.is_none() && tab.merge.is_none()) else { return };
        let buf = &doc.buf;
        let settings = settings.of(buf.path.as_deref());
        let caret = match tab.sels[..] {
            [s] if s.is_empty() => Some(s.head),
            _ => None,
        };
        let ai = &mut self.ai;
        let mut next = None;
        if let Some(result) = ai.job.as_ref().and_then(|j| j.rx.try_recv().ok())
            && let Some(job) = ai.job.take()
        {
            let fresh = job.doc == id && job.version == buf.version && caret == Some(job.at);
            match result {
                Ok(reply) if fresh && job.edit => {
                    ai.edit = parse_edit(&reply).and_then(|(old, new)| locate(&buf.rope.to_string(), &old, &new)).map(|(start, end, new)| {
                        NextEdit { doc: id, version: buf.version, start: buf.rope.byte_to_char(start), end: buf.rope.byte_to_char(end), new }
                    });
                    if ai.edit.is_none() && job.manual {
                        ui::notify_info("No next edit to suggest here.");
                    }
                }
                Ok(reply) if fresh => {
                    let text = insertion(&reply, &job.before, &job.after);
                    if !text.trim().is_empty() {
                        ai.ghost = Some(Ghost { doc: id, version: buf.version, at: job.at, len: buf.len(), text });
                    } else if job.manual {
                        ui::notify_info("No inline suggestion here.");
                    } else if settings.inline_edits {
                        next = Some(true);
                    }
                }
                Ok(_) => {}
                Err(e) if e == "Cancelled." => {}
                Err(e) => {
                    if job.manual || ai.failed.as_ref() != Some(&e) {
                        ui::notify(e.clone());
                    }
                    ai.failed = Some(e);
                }
            }
        }
        if ai.job.as_ref().is_some_and(|j| j.doc != id || j.version != buf.version || caret != Some(j.at)) {
            ai.job = None;
        }
        if ai.edit.as_ref().is_some_and(|e| e.doc != id || e.version != buf.version) {
            ai.edit = None;
        }
        if ai.ghost.as_ref().is_some_and(|g| g.doc != id) {
            ai.ghost = None;
        }
        if ai.seen != (id, buf.version) {
            ai.due = ai.seen.0 == id && buf.edited.is_some_and(|e| e.0 == buf.version);
            ai.seen = (id, buf.version);
        }
        if ai.base.as_ref().is_none_or(|b| b.0 != id) {
            ai.base = Some((id, buf.rope.clone()));
        }
        if ai.due && caret.is_some() && !buf.read_only && !buf.large {
            let idle = ctx.input(|i| i.time) - self.last_input;
            if idle < PAUSE {
                ctx.request_repaint_after(Duration::from_secs_f64(PAUSE - idle));
            } else {
                ai.due = false;
                ai.record(id, buf);
                if ai.ghost.is_none() && ai.edit.is_none() && ai.job.is_none() {
                    next = next.or((settings.inline_suggest || settings.inline_edits).then_some(!settings.inline_suggest));
                }
            }
        }
        if let Some(edit) = next {
            self.ai_start(edit, false, ctx);
        }
    }

    pub(super) fn ai_keys(&mut self, events: &mut Vec<Event>, doc: u64, buf: &mut Document, tab: &mut Tab) {
        let ai = &mut self.ai;
        events.retain(|event| {
            let Event::Key { key, pressed: true, modifiers, .. } = event else { return true };
            let ghost = ai.ghost.as_ref().is_some_and(|g| g.doc == doc && g.version == buf.version);
            let edit = ai.edit.as_ref().filter(|e| e.doc == doc && e.version == buf.version);
            match key {
                Key::Escape if !modifiers.any() && (ghost || edit.is_some() || ai.job.as_ref().is_some_and(|j| j.doc == doc)) => ai.dismiss(),
                Key::Tab if !modifiers.any() && ghost => {
                    let g = ai.ghost.take().unwrap();
                    replace(buf, &mut tab.sels, g.at, g.at, &g.text);
                }
                Key::ArrowRight if modifiers.command && !modifiers.shift && !modifiers.alt && ghost => {
                    let g = ai.ghost.as_mut().unwrap();
                    let word: String = g.text.drain(..next_word(&g.text)).collect();
                    replace(buf, &mut tab.sels, g.at, g.at, &word);
                    (g.at, g.version, g.len) = (g.at + word.chars().count(), buf.version, buf.len());
                    if g.text.is_empty() {
                        ai.ghost = None;
                    }
                }
                Key::Tab if !modifiers.any() && edit.is_some() => {
                    let (start, lines) = edit.map(|e| (e.start, buf.line_of(e.start)..=buf.line_of(e.end))).unwrap();
                    if tab.sels.len() == 1 && lines.contains(&buf.line_of(tab.sels[0].head)) {
                        let e = ai.edit.take().unwrap();
                        replace(buf, &mut tab.sels, e.start, e.end, &e.new);
                    } else {
                        tab.sels = vec![Sel::caret(start)];
                    }
                }
                _ => return true,
            }
            tab.reveal = tab.sels.last().map(|s| s.head);
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_the_prompt_to_whole_lines_within_budget() {
        let before = "first line\nsecond line\nthird";
        assert_eq!(tail(before, 100), before);
        assert_eq!(tail(before, 15), "third");
        assert_eq!(tail(before, 20), "second line\nthird");
        let after = "a();\nb();\nc();";
        assert_eq!(head(after, 100), after);
        assert_eq!(head(after, 8), "a();\n");
        assert_eq!(head("ééé\né", 3), "é");
        assert!(tail("éééé", 3).len() <= 3);
        let prompt = completion_prompt("Rust", "src/main.rs", "fn main() {", "\n}");
        assert!(prompt.contains("Rust file src/main.rs") && prompt.ends_with("fn main() {<CURSOR>\n}"));
        let prompt = edit_prompt("Rust", "a.rs", &[("let a = 1;", "let b = 1;")], "x<CURSOR>y");
        assert!(prompt.contains("Before:\nlet a = 1;\nAfter:\nlet b = 1;") && prompt.contains("x<CURSOR>y") && prompt.contains("<<<<<<< OLD"));
    }

    #[test]
    fn strips_echoed_prefix_tags_and_suffix() {
        let before = "fn main() {\n    let total = ";
        assert_eq!(insertion("<insert>a + b;</insert>", before, ""), "a + b;");
        assert_eq!(insertion("let total = a + b;", before, ""), "a + b;");
        assert_eq!(insertion("    let total = a + b;", before, ""), "a + b;");
        assert_eq!(insertion("fn main() {\n    let total = a + b;", before, ""), "a + b;");
        assert_eq!(insertion("<insert>\n    body();\n}</insert>", "fn f() {", ""), "\n    body();\n}");
        assert_eq!(insertion("<insert>a, b)</insert>", "call(", ")\nnext"), "a, b");
        assert_eq!(insertion("}\n", "    }", ""), "}\n");
        assert_eq!(insertion("<insert></insert>", before, ""), "");
    }

    #[test]
    fn accepts_the_next_word() {
        assert_eq!(next_word("foo bar"), 3);
        assert_eq!(next_word(" bar baz"), 4);
        assert_eq!(next_word("(a, b)"), 1);
        assert_eq!(next_word("\n    x"), 6);
        assert_eq!(next_word("   "), 3);
        assert_eq!(next_word("é1 x"), "é1".len());
    }

    #[test]
    fn parses_next_edits_and_matches_them_uniquely() {
        let reply = "Here:\n<<<<<<< OLD\n    let a = 1;\n=======\n    let b = 1;\n>>>>>>> NEW\n";
        assert_eq!(parse_edit(reply), Some(("    let a = 1;".into(), "    let b = 1;".into())));
        assert_eq!(parse_edit("<<<<<<< OLD\r\nx();\r\n=======\r\n>>>>>>> NEW"), Some(("x();".into(), String::new())));
        assert_eq!(parse_edit("NONE"), None);
        assert_eq!(parse_edit("<<<<<<< OLD\nsame\n=======\nsame\n>>>>>>> NEW"), None);
        assert_eq!(parse_edit("<<<<<<< OLD\n \n=======\nx\n>>>>>>> NEW"), None);

        let text = "fn f() {\n    let a = 1;\n    g(a);\n}\n";
        assert_eq!(locate(text, "    let a = 1;", "    let b = 1;"), Some((17, 18, "b".into())));
        assert_eq!(locate(text, "    g(a);", "    g(a, 2);"), Some((31, 31, ", 2".into())));
        assert_eq!(locate(text, "x", "y"), None);
        assert_eq!(locate("a();\na();\n", "a();", "b();"), None);
        assert_eq!(locate("é = 1;", "é = 1;", "è = 1;"), Some((0, 2, "è".into())));
    }

    #[test]
    fn records_edits_as_line_snippets() {
        assert_eq!(changed("a\nb\nc\n", "a\nb\nc\n"), None);
        assert_eq!(changed("one\nlet a = 1;\nthree\n", "one\nlet ab = 1;\nthree\n"), Some(("let a = 1;".into(), "let ab = 1;".into())));
        assert_eq!(changed("x\ny\n", "x\n"), Some(("y".into(), "".into())));
        assert_eq!(changed("é\n", "è\n"), Some(("é".into(), "è".into())));
    }

    #[test]
    fn accepting_and_applying_are_single_undo_steps() {
        let mut buf = Document::from_text("let x = \nfoo(a);\n", "t".into());
        let mut sels = vec![Sel::caret(8)];
        assert!(replace(&mut buf, &mut sels, 8, 8, "1;"));
        assert_eq!((sels.len(), sels[0].head), (1, 10));
        assert!(replace(&mut buf, &mut sels, 15, 16, "b"));
        assert_eq!(buf.rope.to_string(), "let x = 1;\nfoo(b);\n");
        assert!(buf.undo(&mut sels));
        assert_eq!(buf.rope.to_string(), "let x = 1;\nfoo(a);\n");
        assert!(buf.undo(&mut sels));
        assert_eq!(buf.rope.to_string(), "let x = \nfoo(a);\n");
    }

    #[test]
    fn ghost_follows_matching_typing_and_drops_otherwise() {
        let mut buf = Document::from_text("let x = ", "t".into());
        let mut sels = vec![Sel::caret(8)];
        let mut ai = Ai { ghost: Some(Ghost { doc: 1, version: buf.version, at: 8, len: buf.len(), text: "42;\nnext".into() }), ..Default::default() };
        let s = ai.shown(1, &buf, &sels).unwrap();
        assert_eq!((s.line, s.col, s.first.as_str(), s.below.clone()), (0, 8, "42;", vec!["next".to_owned()]));
        buf.insert_text(&mut sels, "4");
        assert_eq!(ai.shown(1, &buf, &sels).unwrap().first, "2;");
        buf.insert_text(&mut sels, "x");
        assert!(ai.shown(1, &buf, &sels).is_none());

        let mut ai = Ai { edit: Some(NextEdit { doc: 1, version: buf.version, start: 4, end: 5, new: "y".into() }), ..Default::default() };
        let s = ai.shown(1, &buf, &sels).unwrap();
        assert_eq!((s.arrow, s.below.clone(), s.strike), (Some(0), vec!["let y = 4x".to_owned()], Some((4, 5))));
    }
}
