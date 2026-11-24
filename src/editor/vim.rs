use super::buffer::{After, Document, Kind, Sel, is_word};
use crate::command::Command;
use eframe::egui::{Event, Key};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::VisualLine => "VISUAL LINE",
        }
    }
}

#[derive(Default)]
pub struct Vim {
    pub mode: Mode,
    pub keys: String,
    pub line: Option<String>,
    reg: String,
    reg_lines: bool,
    anchor: usize,
    head: usize,
    shown: Option<Sel>,
    search: Option<(String, bool)>,
    find: Option<(char, char)>,
    last: Option<(String, String)>,
    change: Option<String>,
    inserted: String,
}

#[derive(Default)]
pub struct Out {
    pub cmds: Vec<Command>,
    pub copy: Option<String>,
    pub moved: bool,
}

enum Step<T> {
    Wait,
    Bad,
    Got(T),
}

#[derive(Clone, Copy)]
struct Span {
    to: usize,
    lines: bool,
    incl: bool,
}

fn class(c: char, big: bool) -> u8 {
    match c {
        c if c.is_whitespace() => 0,
        _ if big => 1,
        c if is_word(c) => 1,
        _ => 2,
    }
}

fn count(s: &str) -> (Option<usize>, &str) {
    let digits = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    match s.starts_with('0') || digits == 0 {
        true => (None, s),
        false => (s[..digits].parse().ok(), &s[digits..]),
    }
}

struct Ctx<'a> {
    buf: &'a mut Document,
    sels: &'a mut Vec<Sel>,
    unit: &'a str,
    tab: usize,
}

impl Ctx<'_> {
    fn ch(&self, i: usize) -> char {
        self.buf.rope.char(i)
    }

    fn first_nonblank(&self, line: usize) -> usize {
        self.buf.line_start(line) + self.buf.indent_of(line).min(self.buf.line_len(line))
    }

    fn next_word(&self, mut i: usize, big: bool) -> usize {
        let len = self.buf.len();
        if i >= len {
            return len;
        }
        let first = class(self.ch(i), big);
        if first != 0 {
            while i < len && class(self.ch(i), big) == first {
                i += 1;
            }
        }
        while i < len && class(self.ch(i), big) == 0 {
            i += 1;
        }
        i
    }

    fn prev_word(&self, mut i: usize, big: bool) -> usize {
        while i > 0 && class(self.ch(i - 1), big) == 0 {
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let c = class(self.ch(i - 1), big);
        while i > 0 && class(self.ch(i - 1), big) == c {
            i -= 1;
        }
        i
    }

    fn word_end(&self, i: usize, big: bool) -> usize {
        let len = self.buf.len();
        let mut i = i + 1;
        while i < len && class(self.ch(i), big) == 0 {
            i += 1;
        }
        if i >= len {
            return len.saturating_sub(1);
        }
        let c = class(self.ch(i), big);
        while i + 1 < len && class(self.ch(i + 1), big) == c {
            i += 1;
        }
        i
    }

    fn line_at(&self, line: usize, col_from: usize) -> usize {
        let col = col_from - self.buf.line_start(self.buf.line_of(col_from));
        let line = line.min(self.buf.lines() - 1);
        self.buf.line_start(line) + col.min(self.buf.line_len(line).saturating_sub(1))
    }

    fn paragraph(&self, head: usize, down: bool, n: usize) -> usize {
        let mut line = self.buf.line_of(head);
        let last = self.buf.lines() - 1;
        for _ in 0..n {
            loop {
                match down {
                    true if line < last => line += 1,
                    false if line > 0 => line -= 1,
                    _ => break,
                }
                if self.buf.line_len(line) == 0 {
                    break;
                }
            }
        }
        match (down, line == last) {
            (true, true) => self.buf.len(),
            _ => self.buf.line_start(line),
        }
    }

    fn find_char(&self, head: usize, kind: char, target: char, n: usize) -> Option<usize> {
        let line = self.buf.line_of(head);
        let (start, end) = (self.buf.line_start(line), self.buf.line_end(line));
        let mut at = head;
        for _ in 0..n {
            at = match kind {
                'f' | 't' => (at + 1..end).find(|&p| self.ch(p) == target)?,
                _ => (start..at).rev().find(|&p| self.ch(p) == target)?,
            };
        }
        Some(match kind {
            't' => at - 1,
            'T' => at + 1,
            _ => at,
        })
    }

    fn search(&self, head: usize, text: &str, back: bool) -> Option<usize> {
        if text.is_empty() {
            return None;
        }
        let hay = self.buf.rope.to_string();
        let at = self.buf.rope.char_to_byte(head.min(self.buf.len()));
        let step = hay[at..].chars().next().map_or(0, char::len_utf8);
        let from = (at + step).min(hay.len());
        let hit = match back {
            false => hay[from..].find(text).map(|i| i + from).or_else(|| hay.find(text)),
            true => hay[..at].rfind(text).or_else(|| hay.rfind(text)),
        }?;
        Some(self.buf.rope.byte_to_char(hit))
    }

    fn set(&mut self, pos: usize) {
        *self.sels = vec![Sel::caret(pos.min(self.buf.len()))];
    }

    fn edit(&mut self, a: usize, b: usize, text: String) -> bool {
        *self.sels = vec![Sel::caret(a)];
        self.buf.apply(self.sels, vec![(a, b, text)], Kind::Other, After::Carets)
    }
}

impl Vim {
    pub fn reset(&mut self) {
        let last = self.last.take();
        let (reg, lines) = (std::mem::take(&mut self.reg), self.reg_lines);
        *self = Vim { last, reg, reg_lines: lines, ..Default::default() };
    }

    pub fn status(&self) -> String {
        match &self.line {
            Some(line) => line.clone(),
            None if self.keys.is_empty() => format!("-- {} --", self.mode.label()),
            None => format!("-- {} --  {}", self.mode.label(), self.keys),
        }
    }

    pub fn keys(&mut self, events: &mut Vec<Event>, buf: &mut Document, sels: &mut Vec<Sel>, unit: &str, tab: usize) -> Out {
        let mut out = Out::default();
        let mut cx = Ctx { buf, sels, unit, tab };
        if matches!(self.mode, Mode::Visual | Mode::VisualLine) && self.shown.map(|s| (s.anchor, s.head)) != cx.sels.first().map(|s| (s.anchor, s.head)) {
            self.mode = Mode::Normal;
        }
        if self.mode == Mode::Normal && cx.sels.len() == 1 && !cx.sels[0].is_empty() {
            let s = cx.sels[0];
            (self.mode, self.anchor, self.head) = (Mode::Visual, s.anchor, if s.head > s.anchor { s.head - 1 } else { s.head });
            self.show(&mut cx);
        }
        let mut keep = vec![];
        for event in std::mem::take(events) {
            if self.mode == Mode::Insert {
                match &event {
                    Event::Key { key: Key::Escape, pressed: true, .. } => self.leave_insert(&mut cx),
                    Event::Text(t) => self.inserted.push_str(t),
                    _ => {}
                }
                keep.push(event);
                continue;
            }
            if self.line.is_some() {
                self.line_event(&event, &mut cx, &mut out);
                continue;
            }
            match &event {
                Event::Text(t) => t.chars().for_each(|c| self.feed(c, &mut cx, &mut out)),
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let plain = !modifiers.ctrl && !modifiers.command && !modifiers.alt;
                    let mapped = match key {
                        Key::ArrowLeft if plain && !modifiers.shift => Some('h'),
                        Key::ArrowRight if plain && !modifiers.shift => Some('l'),
                        Key::ArrowUp if plain && !modifiers.shift => Some('k'),
                        Key::ArrowDown if plain && !modifiers.shift => Some('j'),
                        Key::Enter | Key::Backspace | Key::Delete if plain => Some(match key {
                            Key::Enter => 'j',
                            Key::Backspace => 'h',
                            _ => 'x',
                        }),
                        _ => None,
                    };
                    match (key, mapped) {
                        (_, Some(c)) => self.feed(c, &mut cx, &mut out),
                        (Key::Escape, _) => {
                            self.keys.clear();
                            if self.mode != Mode::Normal {
                                self.mode = Mode::Normal;
                                cx.set(self.head);
                            }
                            keep.push(event);
                        }
                        (Key::Home | Key::End | Key::PageUp | Key::PageDown, _) => keep.push(event),
                        (Key::R, _) if modifiers.command && !modifiers.shift => {
                            cx.buf.redo(cx.sels);
                            out.moved = true;
                        }
                        _ if plain => {}
                        _ => keep.push(event),
                    }
                }
                Event::Paste(_) => {}
                _ => keep.push(event),
            }
        }
        *events = keep;
        if self.mode == Mode::Normal && cx.sels.len() == 1 && cx.sels[0].is_empty() {
            let head = cx.sels[0].head;
            let line = cx.buf.line_of(head);
            if head == cx.buf.line_end(line) && cx.buf.line_len(line) > 0 {
                cx.set(head - 1);
            }
        }
        out
    }

    fn line_event(&mut self, event: &Event, cx: &mut Ctx, out: &mut Out) {
        let Some(line) = self.line.as_mut() else { return };
        match event {
            Event::Text(t) => line.push_str(t),
            Event::Key { key: Key::Backspace, pressed: true, .. } => {
                line.pop();
                if line.is_empty() {
                    self.line = None;
                }
            }
            Event::Key { key: Key::Escape, pressed: true, .. } => self.line = None,
            Event::Key { key: Key::Enter, pressed: true, .. } => {
                let line = self.line.take().unwrap_or_default();
                self.run_line(&line, cx, out);
            }
            _ => {}
        }
    }

    fn run_line(&mut self, line: &str, cx: &mut Ctx, out: &mut Out) {
        let (kind, body) = line.split_at(1);
        if kind != ":" {
            self.search = Some((body.to_owned(), kind == "?"));
            self.feed('n', cx, out);
            return;
        }
        match body.trim() {
            "w" => out.cmds.push(Command::Save),
            "q" | "q!" | "close" => out.cmds.push(Command::CloseEditor),
            "wq" | "x" => out.cmds.extend([Command::Save, Command::CloseEditor]),
            "wa" => out.cmds.push(Command::SaveAll),
            "" | "noh" | "nohlsearch" => {}
            n if n.chars().all(|c| c.is_ascii_digit()) => {
                let line = n.parse::<usize>().unwrap_or(1).max(1) - 1;
                cx.set(cx.first_nonblank(line.min(cx.buf.lines() - 1)));
                out.moved = true;
            }
            other => crate::ui::notify(format!("Not an editor command: {other}")),
        }
    }

    fn leave_insert(&mut self, cx: &mut Ctx) {
        self.mode = Mode::Normal;
        if let Some(keys) = self.change.take() {
            self.last = Some((keys, std::mem::take(&mut self.inserted)));
        }
        if let Some(s) = cx.sels.first().copied() {
            let start = cx.buf.line_start(cx.buf.line_of(s.head));
            cx.set(if s.head > start { s.head - 1 } else { s.head });
        }
    }

    fn feed(&mut self, c: char, cx: &mut Ctx, out: &mut Out) {
        self.keys.push(c);
        let keys = self.keys.clone();
        match self.run(&keys, cx, out) {
            Step::Wait => {}
            Step::Bad => self.keys.clear(),
            Step::Got(repeat) => {
                self.keys.clear();
                out.moved = true;
                if repeat && let Some((keys, text)) = self.last.clone() {
                    keys.chars().for_each(|c| self.feed(c, cx, out));
                    if self.mode == Mode::Insert {
                        cx.buf.insert_text(cx.sels, &text);
                        self.inserted = text;
                        self.leave_insert(cx);
                    }
                }
            }
        }
    }

    fn motion(&mut self, s: &str, n: Option<usize>, cx: &Ctx, head: usize) -> Step<Span> {
        let mut chars = s.chars();
        let Some(m) = chars.next() else { return Step::Wait };
        let arg = chars.next();
        let k = n.unwrap_or(1);
        let line = cx.buf.line_of(head);
        let span = |to, lines, incl| Step::Got(Span { to, lines, incl });
        match m {
            'h' => span(head.saturating_sub(k).max(cx.buf.line_start(line)), false, false),
            'l' | ' ' => span((head + k).min(cx.buf.line_end(line)), false, false),
            'j' | 'k' => {
                let target = if m == 'j' { line + k } else { line.saturating_sub(k) };
                span(cx.line_at(target, head), true, false)
            }
            'w' | 'W' => span((0..k).fold(head, |p, _| cx.next_word(p, m == 'W')), false, false),
            'b' | 'B' => span((0..k).fold(head, |p, _| cx.prev_word(p, m == 'B')), false, false),
            'e' | 'E' => span((0..k).fold(head, |p, _| cx.word_end(p, m == 'E')), false, true),
            '0' => span(cx.buf.line_start(line), false, false),
            '^' => span(cx.first_nonblank(line), false, false),
            '$' => span(cx.buf.line_end((line + k - 1).min(cx.buf.lines() - 1)), false, false),
            'G' => span(cx.first_nonblank(n.map_or(cx.buf.lines() - 1, |n| n.max(1) - 1).min(cx.buf.lines() - 1)), true, false),
            'g' => match arg {
                None => Step::Wait,
                Some('g') => span(cx.first_nonblank(n.map_or(0, |n| n.max(1) - 1).min(cx.buf.lines() - 1)), true, false),
                _ => Step::Bad,
            },
            '{' | '}' => span(cx.paragraph(head, m == '}', k), false, false),
            '%' => match cx.buf.matching_bracket(head) {
                Some((open, close)) => span(if head == open { close } else { open }, false, true),
                None => Step::Bad,
            },
            'f' | 't' | 'F' | 'T' => match arg {
                None => Step::Wait,
                Some(target) => {
                    self.find = Some((m, target));
                    cx.find_char(head, m, target, k).map_or(Step::Bad, |to| span(to, false, matches!(m, 'f' | 't')))
                }
            },
            ';' | ',' => {
                let Some((kind, target)) = self.find else { return Step::Bad };
                let kind = match (m, kind) {
                    (';', kind) => kind,
                    (_, 'f') => 'F',
                    (_, 'F') => 'f',
                    (_, 't') => 'T',
                    _ => 't',
                };
                cx.find_char(head, kind, target, k).map_or(Step::Bad, |to| span(to, false, matches!(kind, 'f' | 't')))
            }
            'n' | 'N' | '*' | '#' => {
                if matches!(m, '*' | '#') {
                    let (a, b) = cx.buf.word_at(head);
                    self.search = Some((cx.buf.text_of(a, b), m == '#'));
                }
                let Some((text, back)) = self.search.clone() else { return Step::Bad };
                let back = back != (m == 'N');
                let mut at = head;
                for _ in 0..k {
                    match cx.search(at, &text, back) {
                        Some(next) => at = next,
                        None => {
                            crate::ui::notify(format!("Pattern not found: {text}"));
                            return Step::Bad;
                        }
                    }
                }
                span(at, false, false)
            }
            _ => Step::Bad,
        }
    }

    fn range(&self, cx: &Ctx, head: usize, span: Span) -> (usize, usize, bool) {
        let len = cx.buf.len();
        match span.lines {
            true => {
                let (a, b) = (cx.buf.line_of(head.min(span.to)), cx.buf.line_of(head.max(span.to)));
                (cx.buf.line_start(a), if b + 1 >= cx.buf.lines() { len } else { cx.buf.line_start(b + 1) }, true)
            }
            false => {
                let (a, b) = (head.min(span.to), head.max(span.to));
                (a, (b + span.incl as usize).min(len), false)
            }
        }
    }

    fn operate(&mut self, op: char, (a, b, lines): (usize, usize, bool), cx: &mut Ctx, out: &mut Out) {
        let mut text = cx.buf.text_of(a, b);
        if lines && !text.ends_with('\n') {
            text.push('\n');
        }
        if matches!(op, 'd' | 'c' | 'y') {
            (self.reg, self.reg_lines) = (text.clone(), lines);
            out.copy = Some(text);
        }
        match op {
            'y' => cx.set(if lines { cx.sels[0].head.max(a).min(b.saturating_sub(1)) } else { a }),
            'd' if lines => {
                let start = if b == cx.buf.len() && a > 0 && !cx.buf.text_of(a, b).ends_with('\n') { a - 1 } else { a };
                cx.edit(start, b, String::new());
                let line = cx.buf.line_of(start.min(cx.buf.len()));
                let line = if start < a { line + 1 } else { line }.min(cx.buf.lines() - 1);
                cx.set(cx.first_nonblank(line));
            }
            'd' => drop(cx.edit(a, b, String::new())),
            'c' => {
                let (a, b) = match lines {
                    true => (cx.first_nonblank(cx.buf.line_of(a)), cx.buf.line_end(cx.buf.line_of(b.saturating_sub(1).max(a)))),
                    false => (a, b),
                };
                cx.edit(a, b, String::new());
                self.mode = Mode::Insert;
            }
            '>' | '<' => {
                *cx.sels = vec![Sel::new(a, b.saturating_sub(1).max(a))];
                match op {
                    '>' => cx.buf.indent(cx.sels, cx.unit),
                    _ => cx.buf.outdent(cx.sels, cx.tab),
                };
                cx.set(cx.first_nonblank(cx.buf.line_of(a)));
            }
            _ => {}
        }
    }

    fn insert(&mut self, keys: &str, at: usize, cx: &mut Ctx) -> Step<bool> {
        cx.set(at);
        self.mode = Mode::Insert;
        self.change = Some(keys.to_owned());
        self.inserted.clear();
        Step::Got(false)
    }

    fn show(&mut self, cx: &mut Ctx) {
        let (a, h) = (self.anchor, self.head);
        let sel = match self.mode {
            Mode::VisualLine => {
                let (la, lh) = (cx.buf.line_of(a), cx.buf.line_of(h));
                match lh >= la {
                    true => Sel::new(cx.buf.line_start(la), cx.buf.line_end(lh)),
                    false => Sel::new(cx.buf.line_end(la), cx.buf.line_start(lh)),
                }
            }
            _ if h >= a => Sel::new(a, (h + 1).min(cx.buf.len())),
            _ => Sel::new((a + 1).min(cx.buf.len()), h),
        };
        *cx.sels = vec![sel];
        self.shown = Some(sel);
    }

    fn visual_range(&self, cx: &Ctx) -> (usize, usize, bool) {
        let lines = self.mode == Mode::VisualLine;
        let span = Span { to: self.head, lines, incl: true };
        self.range(cx, self.anchor, span)
    }

    fn run(&mut self, keys: &str, cx: &mut Ctx, out: &mut Out) -> Step<bool> {
        let (n, rest) = count(keys);
        let Some(c) = rest.chars().next() else { return Step::Wait };
        let k = n.unwrap_or(1);
        let visual = matches!(self.mode, Mode::Visual | Mode::VisualLine);
        let head = if visual { self.head } else { cx.sels.first().map_or(0, |s| s.head) };
        let line = cx.buf.line_of(head);
        if visual {
            let op = match c {
                'x' | 'd' | 'X' | 'D' => Some('d'),
                's' | 'c' | 'S' | 'C' => Some('c'),
                'y' | 'Y' => Some('y'),
                '>' | '<' => Some(c),
                _ => None,
            };
            if let Some(op) = op {
                let mut range = self.visual_range(cx);
                if matches!(c, 'X' | 'D' | 'S' | 'C' | 'Y') {
                    let (a, b) = (cx.buf.line_of(range.0), cx.buf.line_of(range.1.saturating_sub(1).max(range.0)));
                    range = (cx.buf.line_start(a), if b + 1 >= cx.buf.lines() { cx.buf.len() } else { cx.buf.line_start(b + 1) }, true);
                }
                self.mode = Mode::Normal;
                self.operate(op, range, cx, out);
                if op == 'c' {
                    self.change = None;
                    self.inserted.clear();
                }
                return Step::Got(false);
            }
            match c {
                'v' | 'V' => {
                    let mode = if c == 'v' { Mode::Visual } else { Mode::VisualLine };
                    match self.mode == mode {
                        true => {
                            self.mode = Mode::Normal;
                            cx.set(self.head);
                        }
                        false => {
                            self.mode = mode;
                            self.show(cx);
                        }
                    }
                    return Step::Got(false);
                }
                'o' => {
                    std::mem::swap(&mut self.anchor, &mut self.head);
                    self.show(cx);
                    return Step::Got(false);
                }
                'J' | '~' | 'u' | 'U' => {
                    let (a, b, _) = self.visual_range(cx);
                    self.mode = Mode::Normal;
                    match c {
                        'J' => {
                            *cx.sels = vec![Sel::new(a, b.saturating_sub(1).max(a))];
                            cx.buf.join_lines(cx.sels);
                        }
                        _ => {
                            let text = cx.buf.text_of(a, b);
                            let text: String = match c {
                                'u' => text.to_lowercase(),
                                'U' => text.to_uppercase(),
                                _ => text.chars().map(|ch| if ch.is_uppercase() { ch.to_lowercase().collect::<String>() } else { ch.to_uppercase().collect() }).collect(),
                            };
                            cx.edit(a, b, text);
                        }
                    }
                    cx.set(a);
                    return Step::Got(false);
                }
                'p' | 'P' => {
                    let (a, b, _) = self.visual_range(cx);
                    self.mode = Mode::Normal;
                    let text = self.reg.clone();
                    cx.edit(a, b, text);
                    return Step::Got(false);
                }
                _ => {}
            }
            return match self.motion(rest, n, cx, head) {
                Step::Got(span) => {
                    self.head = match span.lines {
                        true => span.to,
                        false => span.to.min(cx.buf.len().saturating_sub(1)),
                    };
                    self.show(cx);
                    Step::Got(false)
                }
                Step::Wait => Step::Wait,
                Step::Bad => Step::Bad,
            };
        }
        let record = |me: &mut Self| me.last = Some((keys.to_owned(), String::new()));
        match c {
            'i' => return self.insert(keys, head, cx),
            'a' => return self.insert(keys, (head + 1).min(cx.buf.line_end(line)), cx),
            'I' => return self.insert(keys, cx.first_nonblank(line), cx),
            'A' => return self.insert(keys, cx.buf.line_end(line), cx),
            'o' | 'O' => {
                cx.set(head);
                cx.buf.insert_line(cx.sels, c == 'O');
                let at = cx.sels[0].head;
                return self.insert(keys, at, cx);
            }
            'v' | 'V' => {
                (self.mode, self.anchor, self.head) = (if c == 'v' { Mode::Visual } else { Mode::VisualLine }, head, head);
                self.show(cx);
                return Step::Got(false);
            }
            'u' => {
                (0..k).for_each(|_| drop(cx.buf.undo(cx.sels)));
                if let Some(s) = cx.sels.first().copied() {
                    cx.set(s.start());
                }
                return Step::Got(false);
            }
            '.' => return Step::Got(true),
            ':' | '/' | '?' => {
                self.line = Some(c.to_string());
                return Step::Got(false);
            }
            'J' => {
                *cx.sels = vec![Sel::new(head, cx.buf.line_start((line + k.max(2) - 1).min(cx.buf.lines() - 1)))];
                cx.buf.join_lines(cx.sels);
                record(self);
                return Step::Got(false);
            }
            '~' => {
                let end = (head + k).min(cx.buf.line_end(line));
                let text: String = cx.buf.text_of(head, end).chars().map(|ch| if ch.is_uppercase() { ch.to_lowercase().collect::<String>() } else { ch.to_uppercase().collect() }).collect();
                cx.edit(head, end, text);
                cx.set(end);
                record(self);
                return Step::Got(false);
            }
            'r' => {
                let Some(with) = rest.chars().nth(1) else { return Step::Wait };
                let end = head + k;
                if end > cx.buf.line_end(line) {
                    return Step::Bad;
                }
                cx.edit(head, end, with.to_string().repeat(k));
                cx.set(end - 1);
                record(self);
                return Step::Got(false);
            }
            'p' | 'P' => {
                let text = self.reg.repeat(k);
                if text.is_empty() {
                    return Step::Bad;
                }
                match self.reg_lines {
                    true => {
                        let target = if c == 'p' { line + 1 } else { line };
                        if target >= cx.buf.lines() {
                            let len = cx.buf.len();
                            cx.edit(len, len, format!("\n{}", text.trim_end_matches('\n')));
                        } else {
                            let at = cx.buf.line_start(target);
                            cx.edit(at, at, text);
                        }
                        cx.set(cx.first_nonblank(target.min(cx.buf.lines() - 1)));
                    }
                    false => {
                        let at = if c == 'p' { (head + 1).min(cx.buf.line_end(line)) } else { head };
                        cx.edit(at, at, text.clone());
                        cx.set(at + text.chars().count() - 1);
                    }
                }
                record(self);
                return Step::Got(false);
            }
            _ => {}
        }
        let alias = match c {
            'x' => Some("dl"),
            'X' => Some("dh"),
            'D' => Some("d$"),
            'C' => Some("c$"),
            's' => Some("cl"),
            'S' => Some("cc"),
            'Y' => Some("yy"),
            _ => None,
        };
        let (op, motion) = match alias {
            Some(alias) => (alias.chars().next().unwrap(), alias[1..].to_owned()),
            None if matches!(c, 'd' | 'c' | 'y' | '>' | '<') => (c, rest[1..].to_owned()),
            None => {
                return match self.motion(rest, n, cx, head) {
                    Step::Got(span) => {
                        cx.set(span.to);
                        Step::Got(false)
                    }
                    Step::Wait => Step::Wait,
                    Step::Bad => Step::Bad,
                };
            }
        };
        let (n2, motion) = count(&motion);
        let n = match (n, n2) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1) * b.unwrap_or(1)),
        };
        let k = n.unwrap_or(1);
        let range = match motion.chars().next() {
            None => return Step::Wait,
            Some(m) if m == op || (op == 'c' && c == 'S') || (op == 'y' && c == 'Y') => {
                let last = (line + k - 1).min(cx.buf.lines() - 1);
                (cx.buf.line_start(line), if last + 1 >= cx.buf.lines() { cx.buf.len() } else { cx.buf.line_start(last + 1) }, true)
            }
            Some(_) => {
                let motion = match (op, motion.chars().next()) {
                    ('c', Some('w')) if cx.buf.len() > head && class(cx.ch(head), false) != 0 => "e".to_owned() + &motion[1..],
                    ('c', Some('W')) if cx.buf.len() > head && class(cx.ch(head), true) != 0 => "E".to_owned() + &motion[1..],
                    _ => motion.to_owned(),
                };
                let mut span = match self.motion(&motion, n, cx, head) {
                    Step::Got(span) => span,
                    Step::Wait => return Step::Wait,
                    Step::Bad => return Step::Bad,
                };
                if matches!(motion.chars().next(), Some('w' | 'W')) && cx.buf.line_of(span.to) > line && head < cx.buf.line_end(line) {
                    span.to = cx.buf.line_end(line);
                }
                self.range(cx, head, span)
            }
        };
        self.operate(op, range, cx, out);
        match self.mode {
            Mode::Insert => {
                self.change = Some(keys.to_owned());
                self.inserted.clear();
            }
            _ if op != 'y' => record(self),
            _ => {}
        }
        Step::Got(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, at: usize, keys: &str) -> (String, usize, Vim) {
        let mut buf = Document::from_text(text, "t".into());
        let mut sels = vec![Sel::caret(at)];
        let mut vim = Vim::default();
        let mut events: Vec<Event> = keys
            .split('|')
            .flat_map(|part| match part {
                "<esc>" => vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                "<cr>" => vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                text => vec![Event::Text(text.into())],
            })
            .collect();
        let mut chunks = vec![];
        while !events.is_empty() {
            chunks.push(vec![events.remove(0)]);
        }
        for mut chunk in chunks {
            vim.keys(&mut chunk, &mut buf, &mut sels, "    ", 4);
            for e in chunk {
                if let (Mode::Insert, Event::Text(t)) = (vim.mode, &e) {
                    buf.insert_text(&mut sels, t);
                }
            }
        }
        (buf.rope.to_string(), sels[0].head, vim)
    }

    #[test]
    fn motions_and_operators() {
        assert_eq!(run("one two three", 0, "w").1, 4);
        assert_eq!(run("one two three", 0, "2w").1, 8);
        assert_eq!(run("one two three", 0, "e").1, 2);
        assert_eq!(run("one two three", 8, "b").1, 4);
        assert_eq!(run("one two three", 0, "$").1, 12);
        assert_eq!(run("one two three", 0, "dw").0, "two three");
        assert_eq!(run("one two three", 0, "d2w").0, "three");
        assert_eq!(run("one two three", 0, "cw|ONE|<esc>").0, "ONE two three");
        assert_eq!(run("one two three", 4, "D").0, "one ");
        assert_eq!(run("abc", 0, "x").0, "bc");
        assert_eq!(run("abc", 0, "3x").0, "");
        assert_eq!(run("a,b,c", 0, "dt,").0, ",b,c");
        assert_eq!(run("a,b,c", 0, "df,").0, "b,c");
        assert_eq!(run("a,b,c", 0, "f,;").1, 3);
        assert_eq!(run("abc", 1, "rx").0, "axc");
        assert_eq!(run("abc", 0, "~~").0, "ABc");
        assert_eq!(run("one\ntwo\n", 0, "dw").0, "\ntwo\n");
    }

    #[test]
    fn lines_yank_paste_and_undo() {
        assert_eq!(run("a\nb\nc", 0, "dd").0, "b\nc");
        assert_eq!(run("a\nb\nc", 2, "dj").0, "a");
        assert_eq!(run("a\nb\nc", 4, "dd").0, "a\nb");
        assert_eq!(run("a\nb\nc", 0, "yyp").0, "a\na\nb\nc");
        assert_eq!(run("a\nb\nc", 4, "yyp").0, "a\nb\nc\nc");
        assert_eq!(run("a\nb\nc", 0, "ddp").0, "b\na\nc");
        assert_eq!(run("ab", 0, "xp").0, "ba");
        assert_eq!(run("a\nb\nc", 0, "2dd").0, "c");
        assert_eq!(run("a\nb", 0, "ddu").0, "a\nb");
        assert_eq!(run("a\nb\nc", 0, "G").1, 4);
        assert_eq!(run("a\nb\nc", 4, "gg").1, 0);
        assert_eq!(run("a\nb\nc", 0, "2G").1, 2);
        assert_eq!(run("a\nb", 0, ">>").0, "    a\nb");
        assert_eq!(run("a\nb", 0, "J").0, "a b");
    }

    #[test]
    fn inserts_visual_search_and_repeat() {
        assert_eq!(run("ab", 0, "a|X|<esc>").0, "aXb");
        assert_eq!(run("ab", 0, "A|!|<esc>").0, "ab!");
        assert_eq!(run("  ab", 3, "I|-|<esc>").0, "  -ab");
        assert_eq!(run("a", 0, "o|b|<esc>").0, "a\nb");
        assert_eq!(run("a", 0, "O|b|<esc>").0, "b\na");
        assert_eq!(run("a b c", 0, "dw.").0, "c");
        assert_eq!(run("a b", 0, "i|x|<esc>|w|.").0, "xa xb");
        assert_eq!(run("one two", 0, "vey").2.reg, "one");
        assert_eq!(run("one two", 0, "vex").0, " two");
        assert_eq!(run("a\nb\nc", 0, "Vjd").0, "c");
        assert_eq!(run("a\nb\nc", 0, "vjd").0, "\nc");
        assert_eq!(run("foo bar foo", 1, "/|foo|<cr>").1, 8);
        assert_eq!(run("foo bar foo", 0, "*").1, 8);
        assert_eq!(run("foo bar foo", 8, "N").1, 8);
        let (_, at, vim) = run("a\nb\nc", 0, ":|3|<cr>");
        assert_eq!((at, vim.line), (4, None));
    }
}
