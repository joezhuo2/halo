use super::buffer::{After, Document, Kind, Sel};
use super::diff::{self, Hunk};
use super::intel::popup_frame;
use super::typography::FontStyle;
use super::{Editors, Tab, job, notice};
use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, Area, CornerRadius, FontId, Id, Key, Modifiers, Order, Rect, RichText, Sense, TextEdit, TextFormat, Ui, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

const START: &str = "<<<SELECTION START>>>";
const END: &str = "<<<SELECTION END>>>";
const CONTEXT: usize = 40;

#[derive(Default)]
pub(super) struct Chat {
    sessions: HashMap<u64, Session>,
    history: Vec<String>,
}

struct Session {
    version: u64,
    range: (usize, usize),
    original: String,
    cursor: bool,
    language: String,
    text: String,
    focus: bool,
    at: Option<usize>,
    asked: Vec<String>,
    job: Option<Job>,
    reply: Option<String>,
    hunks: Vec<Hunk>,
    rows: Vec<(usize, u32, Range<usize>)>,
}

impl Session {
    fn new(buf: &Document, sel: Sel, language: &str) -> Self {
        let range = target(buf, sel);
        Self {
            version: buf.version,
            range,
            original: buf.text_of(range.0, range.1),
            cursor: sel.is_empty(),
            language: language.to_owned(),
            text: String::new(),
            focus: true,
            at: None,
            asked: vec![],
            job: None,
            reply: None,
            hunks: vec![],
            rows: vec![],
        }
    }

    fn track(&mut self, buf: &Document) -> bool {
        if buf.version == self.version {
            return true;
        }
        let (from, to) = buf.mapped(self.version, self.range.0).zip(buf.mapped(self.version, self.range.1)).unwrap_or(self.range);
        if from > to || to > buf.len() || buf.text_of(from, to) != self.original {
            return false;
        }
        (self.range, self.version) = ((from, to), buf.version);
        true
    }

    fn answer(&mut self, reply: &str) {
        let reply = cleanup(reply, &self.original);
        self.hunks = diff::lines(&self.original, &reply);
        self.reply = Some(reply);
    }

    fn accept(&mut self, buf: &mut Document, sels: &mut Vec<Sel>) -> bool {
        let Some(reply) = self.reply.clone().filter(|r| *r != self.original && self.track(buf)) else { return false };
        buf.apply(sels, vec![(self.range.0, self.range.1, reply)], Kind::Other, After::Carets)
    }
}

struct Job {
    rx: Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn start(top: PathBuf, program: Option<(String, Vec<String>)>, prompt: String, ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (flag, ctx) = (cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let program = program.map_or_else(|| crate::commit_ai::invocation(&top), Ok);
            let _ = tx.send(program.and_then(|program| crate::commit_ai::run(&top, program, &prompt, &flag)));
            ctx.request_repaint();
        });
        Self { rx, cancel }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn target(buf: &Document, sel: Sel) -> (usize, usize) {
    let first = buf.line_of(sel.start());
    let mut last = buf.line_of(sel.end());
    if !sel.is_empty() && last > first && buf.line_start(last) == sel.end() {
        last -= 1;
    }
    (buf.line_start(first), if last + 1 < buf.lines() { buf.line_start(last + 1) } else { buf.len() })
}

fn prompt(buf: &Document, language: &str, (from, to): (usize, usize), cursor: bool, asked: &[String], previous: Option<&str>) -> String {
    let (first, last) = (buf.line_of(from), buf.line_of(to));
    let name = buf.path.as_ref().map_or(buf.title.clone(), |p| p.to_string_lossy().into_owned());
    let what = match cursor {
        true => "the line with the cursor, between the markers. To add new code, keep that line's text where it belongs and add the code next to it",
        false => "the selected lines between the markers",
    };
    let mut text = format!(
        "You are editing {name} ({language}). Rewrite {what}, as the instruction says. Reply with the replacement lines only, with their \
        full indentation: no code fences, no markers, no explanations.\n\n"
    );
    let (latest, earlier) = asked.split_last().map_or(("", &[][..]), |(latest, earlier)| (latest.as_str(), earlier));
    for instruction in earlier {
        text.push_str(&format!("Earlier instruction: {instruction}\n"));
    }
    if let Some(previous) = previous {
        text.push_str(&format!("Your previous replacement was:\n{previous}\nRevise that replacement.\n"));
    }
    let original = buf.text_of(from, to);
    let newline = if original.is_empty() || original.ends_with('\n') { "" } else { "\n" };
    let (before, after) = (buf.text_of(buf.line_start(first.saturating_sub(CONTEXT)), from), buf.text_of(to, buf.line_start(last + CONTEXT)));
    text.push_str(&format!("Instruction: {latest}\n\n{before}{START}\n{original}{newline}{END}\n{after}"));
    text
}

fn cleanup(reply: &str, original: &str) -> String {
    let mut lines: Vec<&str> = reply.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.contains("<<<SELECTION ")).collect();
    if lines.first().is_some_and(|l| l.trim_start().starts_with("```")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.pop();
    }
    let mut text = lines.join("\n");
    let indent: String = original.lines().find(|l| !l.trim().is_empty()).map(|l| l.chars().take_while(|c| c.is_whitespace()).collect()).unwrap_or_default();
    if !text.is_empty() && !text.starts_with(char::is_whitespace) {
        text.insert_str(0, &indent);
    }
    if !text.is_empty() && original.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn preview(first: usize, hunks: &[Hunk]) -> (Vec<usize>, Vec<(usize, u32, Range<usize>)>) {
    let removed = hunks.iter().flat_map(|h| h.left.clone()).map(|l| first + l).collect();
    let added = hunks.iter().filter(|h| !h.right.is_empty()).map(|h| (first + h.left.end, h.right.len() as u32, h.right.clone())).collect();
    (removed, added)
}

impl Editors {
    pub(super) fn chat_exec(&mut self, cmd: &Command) -> bool {
        if *cmd != Command::InlineChat {
            return false;
        }
        let Some((g, t)) = self.active_tab() else { return true };
        let tab = &self.groups[g].tabs[t];
        let Some(doc) = self.docs.get(&tab.doc).filter(|d| d.content.is_text()) else { return true };
        if let Some(session) = self.chat.sessions.get_mut(&tab.doc) {
            session.focus = true;
        } else if doc.buf.read_only {
            self.banner = notice("This editor is read-only.".into());
        } else {
            self.chat.sessions.insert(tab.doc, Session::new(&doc.buf, tab.sels.first().copied().unwrap_or_default(), &doc.hl.language));
        }
        true
    }

    pub(super) fn chat_pads(&mut self, doc: u64, buf: &Document, pads: &mut Vec<(usize, u32)>) {
        let Some(s) = self.chat.sessions.get_mut(&doc) else { return };
        if let Some(result) = s.job.as_ref().and_then(|j| j.rx.try_recv().ok()) {
            s.job = None;
            match result {
                Ok(reply) => s.answer(&reply),
                Err(error) => ui::notify(error),
            }
        }
        if !s.track(buf) {
            self.chat.sessions.remove(&doc);
            self.banner = notice("Inline chat closed because its lines were edited.".into());
            return;
        }
        s.rows = preview(buf.line_of(s.range.0), &s.hunks).1.into_iter().map(|(line, count, lines)| {
            let total = match pads.binary_search_by_key(&line, |p| p.0) {
                Ok(at) => {
                    pads[at].1 += count;
                    pads[at].1
                }
                Err(at) => {
                    pads.insert(at, (line, count));
                    count
                }
            };
            (line, total, lines)
        }).collect();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn chat_ui(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        (doc, active): (u64, bool),
        buf: &mut Document,
        tab: &mut Tab,
        text_rect: Rect,
        (scroll, gutter_w, line_h): (Vec2, f32, f32),
        (font, style): (&FontId, FontStyle),
    ) {
        let Some(s) = self.chat.sessions.get_mut(&doc) else { return };
        let starts = &tab.rows.1;
        let first = buf.line_of(s.range.0);
        let end = first + diff::split(&s.original).len();
        if end >= starts.len() {
            return;
        }
        let y = |row: u32| text_rect.top() - scroll.y + row as f32 * line_h;
        let clip = text_rect.with_min_y(text_rect.top() + tab.stuck as f32 * line_h).intersect(ui.clip_rect());
        let text_x = text_rect.left() + gutter_w;
        let (painter, words) = (ui.painter().with_clip_rect(clip), ui.painter().with_clip_rect(clip.with_min_x(text_x)));
        let pad = |line: usize| s.rows.iter().find(|r| r.0 == line).map_or(0, |r| r.1);
        for line in preview(first, &s.hunks).0.into_iter().filter(|l| l + 1 < starts.len()) {
            let r = Rect::from_min_max(pos2(clip.left(), y(starts[line])), pos2(clip.right(), y(starts[line + 1].saturating_sub(pad(line + 1)))));
            painter.rect_filled(r, CornerRadius::ZERO, ui::with_alpha(theme.deleted, 0.2));
        }
        let lines = s.reply.as_deref().map(diff::split).unwrap_or_default();
        for (line, total, range) in s.rows.iter().filter(|r| r.0 < starts.len()) {
            let top = y(starts[*line].saturating_sub(*total));
            for (k, index) in range.clone().enumerate() {
                let row = Rect::from_min_size(pos2(clip.left(), top + k as f32 * line_h), vec2(clip.width(), line_h));
                painter.rect_filled(row, CornerRadius::ZERO, ui::with_alpha(theme.added, 0.2));
                ui::contrast_border(&painter, theme, row, theme.added);
                let text = lines.get(index).map_or("", |l| l.trim_end_matches('\n'));
                let galley = words.layout_job(style.apply(job(text, 0..text.len(), None, font, theme.text, &[], &[], &TextFormat::default())));
                words.galley(pos2(text_x - scroll.x, row.top() + (line_h - galley.size().y) / 2.0), galley, theme.text);
            }
        }
        if !active {
            return;
        }

        let (above, below) = (y(starts[first]), y(starts[end]));
        let (pivot, at) = match above - clip.top() > 70.0 {
            true => (Align2::LEFT_BOTTOM, above.min(clip.bottom()) - 2.0),
            false => (Align2::LEFT_TOP, below.clamp(clip.top(), (clip.bottom() - 70.0).max(clip.top())) + 2.0),
        };
        let (waiting, shown) = (s.job.is_some(), s.reply.is_some());
        let field_id = Id::new(("inline_chat", doc));
        let (mut send, mut escape, mut accept, mut close, mut focused) = (false, false, false, false, false);
        let history = &self.chat.history;
        Area::new(field_id.with("area")).order(Order::Foreground).pivot(pivot).fixed_pos(pos2(text_x, at)).show(ui.ctx(), |ui| {
            popup_frame(theme).inner_margin(6.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    match waiting {
                        true => ui::spinner(ui, r.center(), 6.0, theme.accent),
                        false => {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::SPARKLE, FontId::proportional(14.0), theme.accent);
                        }
                    }
                    accept = shown && !waiting && ui.memory(|m| m.has_focus(field_id)) && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
                    ui::recall(ui, field_id, history, &mut s.at, &mut s.text);
                    let hint = match (shown, s.cursor) {
                        (true, _) => "Ask a follow-up to refine the change",
                        (false, true) => "Ask to edit or generate code at the cursor",
                        (false, false) => "Ask to edit the selected code",
                    };
                    let field = ui.add(TextEdit::singleline(&mut s.text).id(field_id).margin(ui::FIELD).desired_width(420.0).hint_text(hint));
                    if std::mem::take(&mut s.focus) {
                        field.request_focus();
                    }
                    focused = field.has_focus();
                    if field.lost_focus() {
                        (send, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    }
                });
                ui.horizontal(|ui| {
                    if shown && !waiting {
                        accept |= ui.button("Accept").on_hover_text("Ctrl+Enter").clicked();
                        close |= ui.button("Discard").on_hover_text("Escape").clicked();
                    }
                    let status = match (waiting, shown) {
                        (true, _) => "Working... Escape to cancel",
                        (false, true) => "Ctrl+Enter to accept, Escape to discard, Enter to refine",
                        (false, false) => "Enter to send, Escape to close",
                    };
                    ui.label(RichText::new(status).size(11.0).color(theme.text_dim));
                });
            });
        });
        self.focused |= focused;
        match (escape, waiting) {
            (true, true) => (s.job, s.focus) = (None, true),
            (true, false) => close = true,
            _ => {}
        }
        let instruction = s.text.trim().to_owned();
        if send && !waiting && !instruction.is_empty() {
            self.chat.history.retain(|h| *h != instruction);
            self.chat.history.push(instruction.clone());
            s.asked.push(instruction);
            let text = prompt(buf, &s.language, s.range, s.cursor, &s.asked, s.reply.as_deref());
            let top = buf.path.as_deref().and_then(Path::parent).map(Path::to_path_buf).or_else(|| self.root.clone()).unwrap_or_else(std::env::temp_dir);
            s.job = Some(Job::start(top, None, text, ui.ctx()));
            (s.text, s.at) = (String::new(), None);
        }
        s.focus |= send;
        if accept {
            if s.accept(buf, &mut tab.sels) {
                tab.reveal = tab.sels.last().map(|sel| sel.head);
            }
            close = true;
        }
        if close {
            self.chat.sessions.remove(&doc);
            self.want_focus = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text, "main.rs".into())
    }

    #[test]
    fn targets_whole_selected_lines_or_the_cursor_line() {
        let buf = doc("fn a() {\n    one();\n    two();\n}\n");
        assert_eq!(target(&buf, Sel::new(13, 25)), (9, 31));
        assert_eq!(target(&buf, Sel::new(31, 9)), (9, 31));
        assert_eq!(target(&buf, Sel::caret(15)), (9, 20));
        assert_eq!(target(&buf, Sel::caret(buf.len())), (buf.len(), buf.len()));
        let tail = doc("a\nb");
        assert_eq!(target(&tail, Sel::caret(3)), (2, 3));
    }

    #[test]
    fn builds_prompts_for_a_selection_and_for_the_cursor() {
        let buf = doc("fn a() {\n    one();\n    two();\n}\n");
        let asked = vec!["use a loop".to_owned()];
        let text = prompt(&buf, "Rust", (9, 31), false, &asked, None);
        assert!(text.contains("main.rs (Rust)") && text.contains("selected lines") && text.contains("Instruction: use a loop"));
        assert!(text.contains(&format!("fn a() {{\n{START}\n    one();\n    two();\n{END}\n}}\n")));
        assert!(!text.contains("previous"));

        let asked = vec!["add logging".to_owned(), "use eprintln".to_owned()];
        let text = prompt(&buf, "Rust", (9, 20), true, &asked, Some("    log();\n    one();\n"));
        assert!(text.contains("line with the cursor") && text.contains("Earlier instruction: add logging") && text.contains("Instruction: use eprintln"));
        assert!(text.contains("previous replacement was:\n    log();\n    one();\n"));
        assert!(text.contains(&format!("{START}\n    one();\n{END}\n    two();\n}}\n")));
        let tail = doc("a\nb");
        assert!(prompt(&tail, "Plain Text", (2, 3), true, &asked, None).ends_with(&format!("a\n{START}\nb\n{END}\n")));
    }

    #[test]
    fn cleans_replies_and_keeps_indentation_and_newlines() {
        assert_eq!(cleanup("```rust\r\nfor x in xs {\r\n        f(x);\r\n    }\r\n```", "    one();\n    two();\n"), "    for x in xs {\n        f(x);\n    }\n");
        assert_eq!(cleanup(&format!("{START}\n  b\n{END}"), "a\n"), "  b\n");
        assert_eq!(cleanup("b", "a"), "b");
        assert_eq!(cleanup("", "a\n"), "");
        assert_eq!(cleanup("x = 1", "\n"), "x = 1\n");
    }

    #[test]
    fn previews_removed_lines_and_added_rows() {
        let hunks = diff::lines("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(preview(10, &hunks), (vec![11], vec![(12, 1, 1..2), (13, 1, 3..4)]));
        assert_eq!(preview(0, &diff::lines("", "x\n")), (vec![], vec![(0, 1, 0..1)]));
    }

    #[test]
    fn follows_edits_elsewhere_and_closes_on_edits_inside() {
        let mut buf = doc("a\nb\nc\n");
        let mut s = Session::new(&buf, Sel::caret(2), "Plain Text");
        assert_eq!((s.range, s.original.as_str()), ((2, 4), "b\n"));
        let mut sels = vec![Sel::caret(0)];
        buf.apply(&mut sels, vec![(0, 0, "top\n".into())], Kind::Other, After::Carets);
        buf.apply(&mut sels, vec![(10, 10, "!".into())], Kind::Other, After::Carets);
        assert!(s.track(&buf));
        assert_eq!(s.range, (6, 8));
        buf.apply(&mut sels, vec![(6, 7, "B".into())], Kind::Other, After::Carets);
        assert!(!s.track(&buf));
    }

    #[test]
    fn shows_the_box_and_preview_rows_then_accepts_with_ctrl_enter() {
        use super::super::tests::frame;
        use egui::Event;
        let (ctx, settings, mut editors) = (egui::Context::default(), crate::settings::Settings::default(), Editors::new());
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("a\nb\nc\n".into())]);
        editors.with_active(|_, tab| tab.sels = vec![Sel::caret(2)]);
        editors.exec(&Command::InlineChat, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        assert!(ctx.memory(|m| m.has_focus(Id::new(("inline_chat", 1u64)))));
        editors.chat.sessions.get_mut(&1).unwrap().answer("B\nb2");
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.with_active(|_, tab| tab.rows.1.clone()), Some(vec![0, 1, 4, 5, 6]));
        let ctrl_enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
        frame(&ctx, &mut editors, vec![ctrl_enter]);
        assert!(editors.chat.sessions.is_empty());
        assert_eq!(editors.with_active(|buf, _| buf.rope.to_string()).as_deref(), Some("a\nB\nb2\nc\n"));
        editors.exec(&Command::Undo, &settings, &ctx);
        assert_eq!(editors.with_active(|buf, _| buf.rope.to_string()).as_deref(), Some("a\nb\nc\n"));
    }

    #[test]
    fn accepts_as_one_undo_step() {
        let mut buf = doc("fn a() {\n    one();\n}\n");
        let mut s = Session::new(&buf, Sel::caret(12), "Rust");
        s.answer("log();\n    one();");
        assert_eq!(s.reply.as_deref(), Some("    log();\n    one();\n"));
        let mut sels = vec![Sel::caret(12)];
        assert!(s.accept(&mut buf, &mut sels));
        assert_eq!(buf.rope.to_string(), "fn a() {\n    log();\n    one();\n}\n");
        assert!(buf.undo(&mut sels));
        assert_eq!(buf.rope.to_string(), "fn a() {\n    one();\n}\n");
        s.answer("one();");
        assert!(!s.accept(&mut buf, &mut sels));
    }

    #[test]
    fn runs_the_request_on_a_worker() {
        let echo = if cfg!(windows) { ("cmd.exe".to_owned(), vec!["/c".to_owned(), "more".to_owned()]) } else { ("cat".to_owned(), vec![]) };
        let job = Job::start(std::env::temp_dir(), Some(echo), "```\nfixed();\n```\n".into(), &egui::Context::default());
        let reply = job.rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap().unwrap();
        assert_eq!(cleanup(&reply, "    broken();\n"), "    fixed();\n");
    }
}
