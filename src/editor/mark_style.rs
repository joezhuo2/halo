use super::buffer::{Document, Sel, is_word_in, separator_mask};
use crate::{command::Command, theme::Theme};
use eframe::egui::{self, Context, Ui};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style { One, Two, Three, Four, Five }

impl Style {
    pub const ALL: [Self; 5] = [Self::One, Self::Two, Self::Three, Self::Four, Self::Five];

    pub fn mark_label(self) -> &'static str {
        ["Mark All Occurrences with Style 1", "Mark All Occurrences with Style 2", "Mark All Occurrences with Style 3",
         "Mark All Occurrences with Style 4", "Mark All Occurrences with Style 5"][self as usize]
    }

    pub fn clear_label(self) -> &'static str {
        ["Clear Style 1 Marks", "Clear Style 2 Marks", "Clear Style 3 Marks", "Clear Style 4 Marks", "Clear Style 5 Marks"][self as usize]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    text: String,
    word: bool,
    style: Style,
}

pub type Files = BTreeMap<PathBuf, Vec<Mark>>;

fn boundaries(rope: &Rope, a: usize, b: usize, mask: [u64; 2]) -> bool {
    (a == 0 || !is_word_in(rope.char(a - 1), mask)) && (b == rope.len_chars() || !is_word_in(rope.char(b), mask))
}

pub fn apply(marks: &mut Vec<Mark>, cmd: &Command, buf: &Document, sel: Sel, separators: &str) {
    let mask = separator_mask(separators);
    match cmd {
        Command::UnmarkAll => marks.clear(),
        Command::UnmarkStyle(style) => marks.retain(|m| m.style != *style),
        Command::MarkStyle(style) => {
            let word = sel.is_empty();
            let (a, b) = if word { buf.word_at_with(sel.head, |c| is_word_in(c, mask)) } else { (sel.start(), sel.end()) };
            let text = buf.text_of(a, b);
            if !text.is_empty() && (!word || text.chars().all(|c| is_word_in(c, mask))) {
                marks.retain(|m| m.text != text);
                marks.push(Mark { text, word, style: *style });
            }
        }
        Command::UnmarkAtCursor => {
            let selected = (!sel.is_empty()).then(|| buf.text_of(sel.start(), sel.end()));
            let index = marks.iter().rposition(|m| selected.as_ref() == Some(&m.text)).or_else(|| {
                let p = sel.head;
                marks.iter().rposition(|m| {
                    let n = m.text.chars().count();
                    if n == 0 { return false }
                    let a = p.saturating_sub(n);
                    let b = (p + n).min(buf.len());
                    let near = buf.text_of(a, b);
                    occurrences(&near, &m.text).any(|i| {
                        let start = a + near[..i].chars().count();
                        start <= p && (p < start + n || (m.word && p == start + n)) &&
                            (!m.word || boundaries(&buf.rope, start, start + n, mask))
                    })
                })
            });
            if let Some(i) = index { marks.remove(i); }
        }
        _ => {}
    }
}

fn occurrences<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    let mut from = 0;
    std::iter::from_fn(move || {
        if needle.is_empty() { return None }
        let at = from + text[from..].find(needle)?;
        from = at + text[at..].chars().next()?.len_utf8();
        Some(at)
    })
}

#[derive(Default)]
pub struct Hits {
    pub lines: BTreeMap<usize, Vec<(usize, usize, Style)>>,
    pub runs: [Vec<(usize, usize)>; 5],
}

fn find(rope: &Rope, marks: &[Mark], mask: [u64; 2]) -> Hits {
    let text = rope.to_string();
    let mut hits = Hits::default();
    for m in marks {
        let n = m.text.chars().count();
        for byte in occurrences(&text, &m.text) {
            let a = rope.byte_to_char(byte);
            let b = a + n;
            if m.word && !boundaries(rope, a, b, mask) { continue }
            for line in rope.char_to_line(a)..=rope.char_to_line(b - 1) {
                let base = rope.line_to_char(line);
                hits.lines.entry(line).or_default().push((a.saturating_sub(base), (b - base).min(rope.line(line).len_chars()), m.style));
            }
        }
    }
    for (&line, spans) in &hits.lines {
        let mut added = [false; 5];
        for &(_, _, style) in spans {
            let index = style as usize;
            if std::mem::replace(&mut added[index], true) { continue }
            match hits.runs[index].last_mut() {
                Some((_, end)) if *end == line => *end += 1,
                _ => hits.runs[index].push((line, line + 1)),
            }
        }
    }
    hits
}

#[derive(Default)]
pub struct Cache {
    version: u64,
    marks: Vec<Mark>,
    mask: [u64; 2],
    pending: Option<Receiver<Hits>>,
    hits: Hits,
}

impl Cache {
    pub fn get(&mut self, buf: &Document, marks: &[Mark], separators: &str, ctx: &Context) -> &Hits {
        if marks.is_empty() {
            if !self.marks.is_empty() { *self = Self::default(); }
            return &self.hits;
        }
        let mask = separator_mask(separators);
        let current = self.version == buf.version && self.mask == mask && self.marks == marks;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(hits) if current => self.hits = hits,
                Err(TryRecvError::Empty) => return &self.hits,
                _ => {}
            }
            self.pending = None;
        }
        if !current {
            self.version = buf.version;
            self.mask = mask;
            marks.clone_into(&mut self.marks);
            self.hits = Hits::default();
            let (tx, rx) = channel();
            self.pending = Some(rx);
            let (rope, marks, ctx) = (buf.rope.clone(), marks.to_vec(), ctx.clone());
            std::thread::spawn(move || {
                if tx.send(find(&rope, &marks, mask)).is_ok() { ctx.request_repaint(); }
            });
        }
        &self.hits
    }
}

pub fn menu(ui: &mut Ui, theme: &Theme) -> Option<Command> {
    let mut chosen = None;
    ui.menu_button("Mark Style", |ui| {
        for clear in [false, true] {
            if clear { ui.separator(); }
            for style in Style::ALL {
                let cmd = if clear { Command::UnmarkStyle(style) } else { Command::MarkStyle(style) };
                if button(ui, theme, &cmd, &[style]) { chosen = Some(cmd); }
            }
        }
        ui.separator();
        for cmd in [Command::UnmarkAtCursor, Command::UnmarkAll] {
            if button(ui, theme, &cmd, &Style::ALL) { chosen = Some(cmd); }
        }
    });
    chosen
}

fn button(ui: &mut Ui, theme: &Theme, cmd: &Command, styles: &[Style]) -> bool {
    let mut text = egui::text::LayoutJob::default();
    let format = egui::TextFormat { font_id: egui::TextStyle::Button.resolve(ui.style()), color: theme.text, ..Default::default() };
    for style in styles {
        text.append("\u{25cf}", 0.0, egui::TextFormat { color: theme.mark_styles[*style as usize], ..format.clone() });
    }
    text.append(cmd.label(), 6.0, format);
    crate::ui::menu_item_with(ui, cmd.label(), text, true, "").clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_literals_overlaps_and_multiline() {
        let mark = |text: &str, word, style| Mark { text: text.into(), word, style };
        let rope = Rope::from_str("é élan éx\né\naaa");
        let hits = find(&rope, &[mark("é", true, Style::One), mark("é\na", false, Style::Two),
            mark("aa", false, Style::Five)], separator_mask(crate::settings::WORD_SEPARATORS));
        assert_eq!(hits.lines[&0], [(0, 1, Style::One)]);
        assert_eq!(hits.lines[&1], [(0, 1, Style::One), (0, 2, Style::Two)]);
        assert_eq!(hits.lines[&2], [(0, 1, Style::Two), (0, 2, Style::Five), (1, 3, Style::Five)]);
        assert_eq!(hits.runs[1], [(1, 3)]);
    }

    #[test]
    fn commands_move_and_clear_marks_in_the_active_editor() {
        use super::super::{Editors, tests::frame};
        use crate::settings::Settings;
        let ctx = Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        let mut buf = Document::from_text("one two twofold", "marks".into());
        let path = PathBuf::from("marks.txt");
        buf.path = Some(path.clone());
        let id = editors.insert_doc(buf);
        editors.add_tab(id, vec![Sel::caret(1)]);
        editors.exec(&Command::MarkStyle(Style::One), &settings, &ctx);
        editors.with_active(|_, tab| tab.sels = vec![Sel::caret(5)]);
        editors.exec(&Command::MarkStyle(Style::One), &settings, &ctx);
        assert_eq!(editors.style_marks[&path].len(), 2);
        editors.with_active(|_, tab| tab.sels = vec![Sel::new(4, 7)]);
        editors.exec(&Command::MarkStyle(Style::Two), &settings, &ctx);
        assert_eq!(editors.style_marks[&path].len(), 2);
        let ready = |editors: &mut Editors, count| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                frame(&ctx, editors, vec![]);
                if editors.docs[&id].style_cache.hits.lines.get(&0).is_some_and(|s| s.len() == count) { break }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        ready(&mut editors, 3);
        editors.exec(&Command::PasteText("three".into()), &settings, &ctx);
        ready(&mut editors, 2);
        editors.with_active(|_, tab| tab.sels = vec![Sel::caret(11)]);
        editors.exec(&Command::UnmarkAtCursor, &settings, &ctx);
        assert_eq!(editors.style_marks[&path][0].style, Style::One);
        assert_eq!(editors.style_marks[&path].len(), 1);
        editors.exec(&Command::UnmarkStyle(Style::One), &settings, &ctx);
        assert!(!editors.style_marks.contains_key(&path));
    }

    #[test]
    fn session_round_trip() {
        let mut session = crate::settings::Session::default();
        let marks = vec![Mark { text: "two\nlines".into(), word: false, style: Style::Four }];
        session.style_marks.insert("a.rs".into(), marks);
        let json = serde_json::to_string(&session).unwrap();
        let back: crate::settings::Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back.style_marks, session.style_marks);
        assert!(serde_json::from_str::<crate::settings::Session>("{}").unwrap().style_marks.is_empty());
    }
}
