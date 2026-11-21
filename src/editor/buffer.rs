use ropey::Rope;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

fn next_version() -> u64 {
    NEXT_VERSION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
    pub goal: Option<usize>,
}

impl Sel {
    pub fn caret(pos: usize) -> Self {
        Self { anchor: pos, head: pos, goal: None }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, goal: None }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

fn same(a: &[Sel], b: &[Sel]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.anchor == y.anchor && x.head == y.head)
}

pub fn merge_sels(sels: &mut Vec<Sel>) {
    sels.sort_by_key(|s| (s.start(), s.end()));
    let mut out: Vec<Sel> = Vec::with_capacity(sels.len());
    for s in sels.drain(..) {
        if let Some(last) = out.last_mut()
            && (s.start() < last.end() || (s.start() == last.end() && (s.is_empty() || last.is_empty())))
        {
            let (start, end) = (last.start(), last.end().max(s.end()));
            *last = if last.head < last.anchor { Sel::new(end, start) } else { Sel::new(start, end) };
            continue;
        }
        out.push(s);
    }
    *sels = out;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Type,
    Delete,
    Other,
}

#[derive(Clone, Copy)]
pub enum Take {
    Current,
    Incoming,
    Both,
}

pub enum After {
    Carets,
    Map,
}

#[derive(Clone, Copy)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    Home,
    End,
    DocStart,
    DocEnd,
    PageUp(usize),
    PageDown(usize),
}

struct Change {
    pos: usize,
    removed: String,
    inserted: String,
}

struct Transaction {
    changes: Vec<Change>,
    before: Vec<Sel>,
    after: Vec<Sel>,
    kind: Kind,
    from: u64,
    to: u64,
}

type EditList = Vec<(usize, usize, String)>;

pub struct Document {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub title: String,
    pub crlf: bool,
    bom: bool,
    pub version: u64,
    saved: u64,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    pub dirty_from: Option<usize>,
    pub note: Option<String>,
    pub read_only: bool,
}

const PAIRS: [(char, char); 6] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn map_pos(edits: &EditList, pos: usize) -> usize {
    let mut delta = 0isize;
    for (start, end, text) in edits {
        let inserted = text.chars().count();
        if pos >= *end {
            delta += inserted as isize - (end - start) as isize;
        } else if pos > *start {
            return (*start as isize + delta) as usize + inserted;
        } else {
            break;
        }
    }
    (pos as isize + delta) as usize
}

pub fn wrap(text: impl Iterator<Item = char>, cols: usize, mut cut: impl FnMut(usize)) {
    let (mut width, mut soft) = (0, None::<(usize, usize)>);
    for (i, c) in text.enumerate() {
        // shortcut: widths are guessed (tab 4, CJK 2), upgrade to measured glyphs if rows overflow
        let w = match c {
            '\t' => 4,
            c if c as u32 >= 0x1100 => 2,
            _ => 1,
        };
        if width + w > cols && width > 0 {
            match soft.take() {
                Some((at, upto)) if at <= i => {
                    cut(at);
                    width -= upto;
                }
                _ => {
                    cut(i);
                    width = 0;
                }
            }
        }
        width += w;
        if c == ' ' || c == '\t' {
            soft = Some((i + 1, width));
        }
    }
}

pub fn hidden(folds: &[(usize, usize)], line: usize) -> Option<usize> {
    folds.iter().position(|f| f.0 < line && line <= f.1)
}

fn title_of(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

impl Document {
    pub fn from_text(text: &str, title: String) -> Self {
        let version = next_version();
        Self {
            rope: Rope::from_str(text),
            path: None,
            title,
            crlf: cfg!(windows),
            bom: false,
            version,
            saved: version,
            undo: vec![],
            redo: vec![],
            dirty_from: None,
            note: None,
            read_only: false,
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let name = title_of(path);
        let size = std::fs::metadata(path).map_err(|e| format!("Cannot open {name}: {e}"))?.len();
        if size > 50 * 1024 * 1024 {
            return Err(format!("{name} is larger than 50 MB and was not opened."));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot open {name}: {e}"))?;
        if bytes.iter().take(8192).any(|b| *b == 0) {
            return Err(format!("{name} looks like a binary file and was not opened."));
        }
        let (mut text, lossy) = match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
        };
        let bom = text.starts_with('\u{feff}');
        if bom {
            text.remove(0);
        }
        let crlf = text.contains("\r\n");
        if crlf {
            text = text.replace("\r\n", "\n");
        }
        let mut doc = Self::from_text(&text, name);
        doc.path = Some(path.to_path_buf());
        doc.crlf = crlf;
        doc.bom = bom;
        if lossy {
            doc.note = Some("This file is not valid UTF-8; invalid bytes were replaced.".into());
        }
        Ok(doc)
    }

    pub fn save(&mut self) -> Result<(), String> {
        let Some(path) = self.path.clone() else { return Err("No file name.".into()) };
        let write = || -> std::io::Result<()> {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
            if self.bom {
                out.write_all("\u{feff}".as_bytes())?;
            }
            for chunk in self.rope.chunks() {
                if self.crlf {
                    out.write_all(chunk.replace('\n', "\r\n").as_bytes())?;
                } else {
                    out.write_all(chunk.as_bytes())?;
                }
            }
            out.flush()
        };
        write().map_err(|e| format!("Cannot save {}: {e}", title_of(&path)))?;
        self.saved = self.version;
        self.title = title_of(&path);
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved
    }

    pub fn mark_clean(&mut self) {
        self.saved = self.version;
    }

    pub fn mark_dirty(&mut self) {
        self.saved = 0;
    }

    pub fn len(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_of(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len()))
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.lines()))
    }

    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.lines() {
            return 0;
        }
        let slice = self.rope.line(line);
        let len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' { len - 1 } else { len }
    }

    pub fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    pub fn line_text(&self, line: usize) -> String {
        if line >= self.lines() {
            return String::new();
        }
        let mut text: String = self.rope.line(line).into();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    pub fn foldable(&self, line: usize) -> bool {
        let blank = |l: usize| self.indent_of(l) >= self.line_len(l);
        !blank(line) && (line + 1..self.lines()).find(|l| !blank(*l)).is_some_and(|next| self.indent_of(next) > self.indent_of(line))
    }

    // shortcut: folds follow indentation, upgrade to the language server's folding ranges if they are wrong for a language
    pub fn fold_end(&self, line: usize) -> Option<usize> {
        if line >= self.lines() || !self.foldable(line) {
            return None;
        }
        let mut end = line;
        for next in line + 1..self.lines() {
            if self.indent_of(next) >= self.line_len(next) {
                continue;
            }
            if self.indent_of(next) <= self.indent_of(line) {
                break;
            }
            end = next;
        }
        Some(end)
    }

    pub fn indent_of(&self, line: usize) -> usize {
        if line >= self.lines() {
            return 0;
        }
        self.rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').count()
    }

    fn touch(&mut self, line: usize) {
        self.dirty_from = Some(self.dirty_from.map_or(line, |l| l.min(line)));
    }

    pub fn clamp(&self, sels: &mut Vec<Sel>) {
        let len = self.len();
        if sels.is_empty() {
            sels.push(Sel::caret(0));
        }
        for s in sels.iter_mut() {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        }
    }

    pub fn apply(&mut self, sels: &mut Vec<Sel>, mut edits: EditList, kind: Kind, after: After) -> bool {
        edits.retain(|(s, e, t)| s != e || !t.is_empty());
        if edits.is_empty() || self.read_only {
            return false;
        }
        edits.sort_by_key(|e| (e.0, e.1));
        let mut merged: EditList = Vec::with_capacity(edits.len());
        for edit in edits {
            if let Some(last) = merged.last_mut()
                && edit.0 < last.1
            {
                last.1 = last.1.max(edit.1);
                last.2.push_str(&edit.2);
                continue;
            }
            merged.push(edit);
        }
        let before = sels.clone();
        let first_line = self.line_of(merged[0].0);
        let mut delta = 0isize;
        let mut changes = Vec::with_capacity(merged.len());
        let mut ends = Vec::with_capacity(merged.len());
        for (start, end, text) in &merged {
            let at = (*start as isize + delta) as usize;
            let until = at + (end - start);
            let removed = self.rope.slice(at..until).to_string();
            if until > at {
                self.rope.remove(at..until);
            }
            if !text.is_empty() {
                self.rope.insert(at, text);
            }
            let inserted = text.chars().count();
            ends.push(at + inserted);
            delta += inserted as isize - (end - start) as isize;
            changes.push(Change { pos: at, removed, inserted: text.clone() });
        }
        *sels = match after {
            After::Carets => ends.into_iter().map(Sel::caret).collect(),
            After::Map => before.iter().map(|s| Sel::new(map_pos(&merged, s.anchor), map_pos(&merged, s.head))).collect(),
        };
        merge_sels(sels);
        self.touch(first_line);
        self.record(changes, before, sels.clone(), kind);
        true
    }

    fn record(&mut self, changes: Vec<Change>, before: Vec<Sel>, after: Vec<Sel>, kind: Kind) {
        let from = self.version;
        self.version = next_version();
        self.redo.clear();
        if kind != Kind::Other
            && let Some(last) = self.undo.last_mut()
            && last.kind == kind
            && last.to == from
            && same(&last.after, &before)
        {
            last.changes.extend(changes);
            last.after = after;
            last.to = self.version;
            return;
        }
        self.undo.push(Transaction { changes, before, after, kind, from, to: self.version });
    }

    pub fn fix_after(&mut self, sels: &[Sel]) {
        if let Some(last) = self.undo.last_mut() {
            last.after = sels.to_vec();
        }
    }

    pub fn undo(&mut self, sels: &mut Vec<Sel>) -> bool {
        let Some(t) = self.undo.pop() else { return false };
        let mut line = usize::MAX;
        for c in t.changes.iter().rev() {
            let inserted = c.inserted.chars().count();
            if inserted > 0 {
                self.rope.remove(c.pos..c.pos + inserted);
            }
            if !c.removed.is_empty() {
                self.rope.insert(c.pos, &c.removed);
            }
            line = line.min(self.line_of(c.pos));
        }
        *sels = t.before.clone();
        self.version = t.from;
        self.touch(line);
        self.redo.push(t);
        true
    }

    pub fn redo(&mut self, sels: &mut Vec<Sel>) -> bool {
        let Some(t) = self.redo.pop() else { return false };
        let mut line = usize::MAX;
        for c in &t.changes {
            let removed = c.removed.chars().count();
            if removed > 0 {
                self.rope.remove(c.pos..c.pos + removed);
            }
            if !c.inserted.is_empty() {
                self.rope.insert(c.pos, &c.inserted);
            }
            line = line.min(self.line_of(c.pos));
        }
        *sels = t.after.clone();
        self.version = t.to;
        self.touch(line);
        self.undo.push(t);
        true
    }

    pub fn insert_text(&mut self, sels: &mut Vec<Sel>, text: &str) -> bool {
        let lines: Vec<&str> = text.split('\n').collect();
        let distribute = sels.len() > 1 && lines.len() == sels.len();
        let edits = sels
            .iter()
            .enumerate()
            .map(|(i, s)| (s.start(), s.end(), if distribute { lines[i].to_owned() } else { text.to_owned() }))
            .collect();
        let typed = text.chars().count() == 1 && text != "\n";
        self.apply(sels, edits, if typed { Kind::Type } else { Kind::Other }, After::Carets)
    }

    pub fn type_char(&mut self, sels: &mut Vec<Sel>, c: char) -> bool {
        let at = |pos: usize| self.rope.get_char(pos);
        // shortcut: any matching closer is typed over, upgrade to tracking the inserted ones if that swallows wanted input
        if PAIRS.iter().any(|p| p.1 == c) && sels.iter().all(|s| s.is_empty() && at(s.head) == Some(c)) {
            sels.iter_mut().for_each(|s| *s = Sel::caret(s.head + 1));
            return true;
        }
        let Some(close) = PAIRS.iter().find(|p| p.0 == c).map(|p| p.1) else { return self.insert_text(sels, &c.to_string()) };
        if sels.iter().all(|s| !s.is_empty()) {
            let edits = sels.iter().flat_map(|s| [(s.start(), s.start(), c.to_string()), (s.end(), s.end(), close.to_string())]).collect();
            let changed = self.apply(sels, edits, Kind::Other, After::Map);
            for s in sels.iter_mut().filter(|_| changed) {
                match s.head > s.anchor {
                    true => s.head -= 1,
                    false => s.anchor -= 1,
                }
            }
            self.fix_after(sels);
            return changed;
        }
        let mut back = vec![];
        let edits: EditList = sels
            .iter()
            .map(|s| {
                let (prev, next) = (s.start().checked_sub(1).and_then(at), at(s.end()));
                let room = next.is_none_or(|n| n.is_whitespace() || ")]},;".contains(n));
                let pair = s.is_empty() && room && !(c == close && prev.is_some_and(|p| is_word(p) || p == c));
                back.push(pair as usize);
                (s.start(), s.end(), if pair { format!("{c}{close}") } else { c.to_string() })
            })
            .collect();
        let changed = self.apply(sels, edits, Kind::Type, After::Carets);
        if changed && sels.len() == back.len() {
            for (s, b) in sels.iter_mut().zip(&back) {
                *s = Sel::caret(s.head - b);
            }
            self.fix_after(sels);
        }
        changed
    }

    pub fn newline(&mut self, sels: &mut Vec<Sel>, unit: &str) -> bool {
        let mut back = vec![];
        let edits: EditList = sels
            .iter()
            .map(|s| {
                let (start, end) = (s.start(), s.end());
                let line = self.line_of(start);
                let column = start - self.line_start(line);
                let indent: String = self.rope.line(line).chars().take(column).take_while(|c| *c == ' ' || *c == '\t').collect();
                let prev = (start > 0).then(|| self.rope.char(start - 1));
                let next = (end < self.len()).then(|| self.rope.char(end));
                let opens = matches!(prev, Some('{' | '(' | '[' | ':'));
                let closes = matches!((prev, next), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')));
                let mut text = format!("\n{indent}");
                if opens {
                    text.push_str(unit);
                }
                if closes {
                    text.push_str(&format!("\n{indent}"));
                    back.push(1 + indent.chars().count());
                } else {
                    back.push(0);
                }
                (start, end, text)
            })
            .collect();
        let changed = self.apply(sels, edits, Kind::Other, After::Carets);
        if changed && sels.len() == back.len() {
            for (s, b) in sels.iter_mut().zip(&back) {
                *s = Sel::caret(s.head - b);
            }
            self.fix_after(sels);
        }
        changed
    }

    pub fn backspace(&mut self, sels: &mut Vec<Sel>, word: bool, pairs: bool) -> bool {
        let edits = sels
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    (s.start(), s.end(), String::new())
                } else if word {
                    (self.word_left(s.head), s.head, String::new())
                } else {
                    let (before, after) = (s.head.checked_sub(1).and_then(|at| self.rope.get_char(at)), self.rope.get_char(s.head));
                    let pair = pairs && PAIRS.iter().any(|p| Some(p.0) == before && Some(p.1) == after);
                    (s.head.saturating_sub(1), s.head + pair as usize, String::new())
                }
            })
            .collect();
        self.apply(sels, edits, Kind::Delete, After::Carets)
    }

    pub fn delete(&mut self, sels: &mut Vec<Sel>, word: bool) -> bool {
        let len = self.len();
        let edits = sels
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    (s.start(), s.end(), String::new())
                } else if word {
                    (s.head, self.word_right(s.head), String::new())
                } else {
                    (s.head, (s.head + 1).min(len), String::new())
                }
            })
            .collect();
        self.apply(sels, edits, Kind::Delete, After::Carets)
    }

    fn touched_lines(&self, sels: &[Sel]) -> Vec<usize> {
        let mut lines = vec![];
        for s in sels {
            let first = self.line_of(s.start());
            let mut last = self.line_of(s.end());
            if last > first && s.end() == self.line_start(last) {
                last -= 1;
            }
            lines.extend(first..=last);
        }
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    pub fn indent(&mut self, sels: &mut Vec<Sel>, unit: &str) -> bool {
        let edits = self
            .touched_lines(sels)
            .into_iter()
            .filter(|l| self.line_len(*l) > 0)
            .map(|l| (self.line_start(l), self.line_start(l), unit.to_owned()))
            .collect();
        self.apply(sels, edits, Kind::Other, After::Map)
    }

    pub fn outdent(&mut self, sels: &mut Vec<Sel>, tab_size: usize) -> bool {
        let edits = self
            .touched_lines(sels)
            .into_iter()
            .map(|l| {
                let start = self.line_start(l);
                let mut chars = self.rope.line(l).chars();
                let count = match chars.next() {
                    Some('\t') => 1,
                    Some(' ') => 1 + chars.take(tab_size.saturating_sub(1)).take_while(|c| *c == ' ').count(),
                    _ => 0,
                };
                (start, start + count, String::new())
            })
            .collect();
        self.apply(sels, edits, Kind::Other, After::Map)
    }

    pub fn tab(&mut self, sels: &mut Vec<Sel>, tab_size: usize, spaces: bool) -> bool {
        let unit = if spaces { " ".repeat(tab_size) } else { "\t".to_owned() };
        if sels.iter().any(|s| self.line_of(s.start()) != self.line_of(s.end())) {
            return self.indent(sels, &unit);
        }
        let edits = sels
            .iter()
            .map(|s| {
                let column = s.start() - self.line_start(self.line_of(s.start()));
                let text = if spaces { " ".repeat(tab_size - column % tab_size.max(1)) } else { "\t".to_owned() };
                (s.start(), s.end(), text)
            })
            .collect();
        self.apply(sels, edits, Kind::Other, After::Carets)
    }

    pub fn toggle_comment(&mut self, sels: &mut Vec<Sel>, open: &str, close: Option<&str>) -> bool {
        let lines: Vec<usize> = self.touched_lines(sels).into_iter().filter(|l| self.indent_of(*l) < self.line_len(*l)).collect();
        if lines.is_empty() {
            return false;
        }
        let commented = lines.iter().all(|l| self.line_text(*l).trim_start().starts_with(open));
        let min_indent = lines.iter().map(|l| self.indent_of(*l)).min().unwrap_or(0);
        let mut edits: EditList = vec![];
        for line in lines {
            let start = self.line_start(line);
            let text = self.line_text(line);
            if commented {
                let at = start + self.indent_of(line);
                let rest = text.trim_start()[open.len()..].chars().next();
                let extra = usize::from(rest == Some(' '));
                edits.push((at, at + open.chars().count() + extra, String::new()));
                if let Some(close) = close
                    && let Some(body) = text.trim_end().strip_suffix(close)
                {
                    let body_len = body.trim_end().chars().count();
                    edits.push((start + body_len, start + text.trim_end().chars().count(), String::new()));
                }
            } else {
                edits.push((start + min_indent, start + min_indent, format!("{open} ")));
                if let Some(close) = close {
                    let end = start + text.chars().count();
                    edits.push((end, end, format!(" {close}")));
                }
            }
        }
        self.apply(sels, edits, Kind::Other, After::Map)
    }

    pub fn word_left(&self, pos: usize) -> usize {
        let mut i = pos.min(self.len());
        while i > 0 && self.rope.char(i - 1).is_whitespace() && self.rope.char(i - 1) != '\n' {
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let first = self.rope.char(i - 1);
        if first == '\n' {
            return if i == pos { i - 1 } else { i };
        }
        let word = is_word(first);
        while i > 0 {
            let c = self.rope.char(i - 1);
            if c.is_whitespace() || is_word(c) != word {
                break;
            }
            i -= 1;
        }
        i
    }

    pub fn word_right(&self, pos: usize) -> usize {
        let len = self.len();
        let mut i = pos.min(len);
        if i < len && self.rope.char(i) == '\n' {
            return i + 1;
        }
        while i < len && self.rope.char(i).is_whitespace() && self.rope.char(i) != '\n' {
            i += 1;
        }
        if i >= len || self.rope.char(i) == '\n' {
            return i;
        }
        let word = is_word(self.rope.char(i));
        while i < len {
            let c = self.rope.char(i);
            if c.is_whitespace() || is_word(c) != word {
                break;
            }
            i += 1;
        }
        i
    }

    pub fn word_at(&self, pos: usize) -> (usize, usize) {
        let len = self.len();
        let (mut start, mut end) = (pos.min(len), pos.min(len));
        while start > 0 && is_word(self.rope.char(start - 1)) {
            start -= 1;
        }
        while end < len && is_word(self.rope.char(end)) {
            end += 1;
        }
        if start == end && end < len && self.rope.char(end) != '\n' {
            end += 1;
        }
        (start, end)
    }

    pub fn move_sels(&self, sels: &mut Vec<Sel>, motion: Motion, extend: bool) {
        let len = self.len();
        let last_line = self.lines().saturating_sub(1);
        for s in sels.iter_mut() {
            let line = self.line_of(s.head);
            let (head, goal) = match motion {
                Motion::Left if !extend && !s.is_empty() => (s.start(), None),
                Motion::Right if !extend && !s.is_empty() => (s.end(), None),
                Motion::Left => (s.head.saturating_sub(1), None),
                Motion::Right => ((s.head + 1).min(len), None),
                Motion::WordLeft => (self.word_left(s.head), None),
                Motion::WordRight => (self.word_right(s.head), None),
                Motion::Home => {
                    let start = self.line_start(line);
                    let first = start + self.indent_of(line);
                    (if s.head == first { start } else { first }, None)
                }
                Motion::End => (self.line_end(line), None),
                Motion::DocStart => (0, None),
                Motion::DocEnd => (len, None),
                Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) => {
                    let column = s.goal.unwrap_or(s.head - self.line_start(line));
                    let target = match motion {
                        Motion::Up => line as isize - 1,
                        Motion::Down => line as isize + 1,
                        Motion::PageUp(n) => line as isize - n as isize,
                        Motion::PageDown(n) => line as isize + n as isize,
                        _ => line as isize,
                    };
                    if target < 0 {
                        (0, Some(column))
                    } else if target as usize > last_line {
                        (len, Some(column))
                    } else {
                        let target = target as usize;
                        (self.line_start(target) + column.min(self.line_len(target)), Some(column))
                    }
                }
            };
            s.head = head;
            s.goal = goal;
            if !extend {
                s.anchor = head;
            }
        }
        merge_sels(sels);
    }

    pub fn cuts(&self, line: usize, cols: Option<usize>) -> Vec<usize> {
        let mut cuts = vec![0];
        if let Some(cols) = cols.filter(|_| line < self.lines()) {
            wrap(self.rope.line(line).chars().filter(|c| *c != '\n'), cols, |at| cuts.push(at));
        }
        cuts
    }

    pub fn move_rows(&self, sels: &mut Vec<Sel>, down: bool, count: usize, extend: bool, cols: Option<usize>, folds: &[(usize, usize)]) {
        for s in sels.iter_mut() {
            let mut line = self.line_of(s.head);
            let mut cuts = self.cuts(line, cols);
            let column = s.head - self.line_start(line);
            let mut piece = cuts.partition_point(|c| *c <= column) - 1;
            let goal = s.goal.unwrap_or(column - cuts[piece]);
            let mut edge = None;
            for _ in 0..count {
                if down && piece + 1 < cuts.len() {
                    piece += 1;
                } else if !down && piece > 0 {
                    piece -= 1;
                } else {
                    let next = if down {
                        (line + 1..self.lines()).find(|l| hidden(folds, *l).is_none())
                    } else {
                        (0..line).rev().find(|l| hidden(folds, *l).is_none())
                    };
                    let Some(next) = next else {
                        edge = Some(if down { self.len() } else { 0 });
                        break;
                    };
                    line = next;
                    cuts = self.cuts(line, cols);
                    piece = if down { 0 } else { cuts.len() - 1 };
                }
            }
            let last = cuts.get(piece + 1).map_or(self.line_len(line), |next| next - 1);
            s.head = edge.unwrap_or(self.line_start(line) + (cuts[piece] + goal).min(last));
            s.goal = Some(goal);
            if !extend {
                s.anchor = s.head;
            }
        }
        merge_sels(sels);
    }

    pub fn add_cursor(&self, sels: &mut Vec<Sel>, down: bool) -> Option<usize> {
        let reference = if down { *sels.last()? } else { *sels.first()? };
        let line = self.line_of(reference.head);
        let column = reference.goal.unwrap_or(reference.head - self.line_start(line));
        let target = if down { line + 1 } else { line.checked_sub(1)? };
        if target >= self.lines() {
            return None;
        }
        let pos = self.line_start(target) + column.min(self.line_len(target));
        sels.push(Sel { anchor: pos, head: pos, goal: Some(column) });
        merge_sels(sels);
        Some(pos)
    }

    pub fn text_of(&self, start: usize, end: usize) -> String {
        self.rope.slice(start.min(self.len())..end.min(self.len())).to_string()
    }

    pub fn copy_text(&self, sels: &[Sel]) -> String {
        if sels.iter().all(Sel::is_empty) {
            let mut lines: Vec<usize> = sels.iter().map(|s| self.line_of(s.head)).collect();
            lines.dedup();
            return lines.into_iter().map(|l| format!("{}\n", self.line_text(l))).collect();
        }
        sels.iter().filter(|s| !s.is_empty()).map(|s| self.text_of(s.start(), s.end())).collect::<Vec<_>>().join("\n")
    }

    pub fn cut(&mut self, sels: &mut Vec<Sel>) -> bool {
        let edits = if sels.iter().all(Sel::is_empty) {
            sels.iter()
                .map(|s| {
                    let line = self.line_of(s.head);
                    (self.line_start(line), self.line_start(line + 1).min(self.len()), String::new())
                })
                .collect()
        } else {
            sels.iter().map(|s| (s.start(), s.end(), String::new())).collect()
        };
        self.apply(sels, edits, Kind::Other, After::Carets)
    }

    pub fn find_all(&self, needle: &str, case: bool, word: bool) -> Vec<(usize, usize)> {
        if needle.is_empty() {
            return vec![];
        }
        let mut hay = self.rope.to_string();
        let mut needle = needle.to_owned();
        if !case {
            hay.make_ascii_lowercase();
            needle.make_ascii_lowercase();
        }
        let count = needle.chars().count();
        let mut out = vec![];
        let (mut byte, mut chars) = (0, 0);
        for (at, _) in hay.match_indices(&needle) {
            chars += hay[byte..at].chars().count();
            byte = at;
            if word {
                let before = hay[..at].chars().next_back().is_some_and(is_word);
                let after = hay[at + needle.len()..].chars().next().is_some_and(is_word);
                if before || after {
                    continue;
                }
            }
            out.push((chars, chars + count));
            if out.len() >= 20_000 {
                break;
            }
        }
        out
    }

    pub fn select_next_occurrence(&self, sels: &mut Vec<Sel>) -> Option<usize> {
        if sels.iter().any(Sel::is_empty) {
            for s in sels.iter_mut().filter(|s| s.is_empty()) {
                let (start, end) = self.word_at(s.head);
                *s = Sel::new(start, end);
            }
            merge_sels(sels);
            return sels.last().map(|s| s.head);
        }
        let last = *sels.last()?;
        let needle = self.text_of(last.start(), last.end());
        let matches = self.find_all(&needle, true, false);
        let taken = |m: &(usize, usize)| sels.iter().any(|s| s.start() == m.0 && s.end() == m.1);
        let next = matches.iter().find(|m| m.0 >= last.end() && !taken(m)).or_else(|| matches.iter().find(|m| !taken(m)))?;
        sels.push(Sel::new(next.0, next.1));
        merge_sels(sels);
        Some(next.1)
    }

    pub fn conflicts(&self) -> Vec<[usize; 4]> {
        let mut out = vec![];
        let mut open: Option<[usize; 3]> = None;
        for (i, line) in self.rope.lines().enumerate() {
            if !matches!(line.get_char(0), Some('<' | '|' | '=' | '>')) {
                continue;
            }
            let marker: String = line.chars().take(7).collect();
            match (marker.as_str(), &mut open) {
                ("<<<<<<<", _) => open = Some([i, usize::MAX, usize::MAX]),
                ("|||||||", Some(o)) if o[2] == usize::MAX => o[1] = i,
                ("=======", Some(o)) if o[2] == usize::MAX => o[2] = i,
                (">>>>>>>", Some(o)) if o[2] != usize::MAX => {
                    out.push([o[0], o[1].min(o[2]), o[2], i]);
                    open = None;
                }
                _ => {}
            }
        }
        out
    }

    pub fn resolve(&mut self, sels: &mut Vec<Sel>, [start, base, mid, end]: [usize; 4], take: Take) -> bool {
        let lines = |a: usize, b: usize| self.text_of(self.line_start(a), self.line_start(b));
        let text = match take {
            Take::Current => lines(start + 1, base),
            Take::Incoming => lines(mid + 1, end),
            Take::Both => lines(start + 1, base) + &lines(mid + 1, end),
        };
        let from = self.line_start(start);
        let to = if end + 1 < self.lines() { self.line_start(end + 1) } else { self.len() };
        self.apply(sels, vec![(from, to, text)], Kind::Other, After::Carets)
    }

    pub fn matching_bracket(&self, pos: usize) -> Option<(usize, usize)> {
        const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
        let len = self.len();
        for at in [pos, pos.wrapping_sub(1)] {
            if at >= len {
                continue;
            }
            let c = self.rope.char(at);
            if let Some((open, close)) = PAIRS.iter().find(|p| p.0 == c) {
                let mut depth = 0;
                for (i, ch) in self.rope.chars_at(at).enumerate().take(20_000) {
                    if ch == *open {
                        depth += 1;
                    } else if ch == *close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((at, at + i));
                        }
                    }
                }
                return None;
            }
            if let Some((open, close)) = PAIRS.iter().find(|p| p.1 == c) {
                let mut depth = 0;
                let mut i = at + 1;
                while i > 0 && at + 1 - i < 20_000 {
                    i -= 1;
                    let ch = self.rope.char(i);
                    if ch == *close {
                        depth += 1;
                    } else if ch == *open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((i, at));
                        }
                    }
                }
                return None;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::from_text(text, "t".into())
    }

    #[test]
    fn multi_cursor_typing_and_undo_redo() {
        let mut d = doc("ab\ncd\nef");
        let mut sels = vec![Sel::caret(0), Sel::caret(3), Sel::caret(6)];
        d.insert_text(&mut sels, "x");
        d.insert_text(&mut sels, "y");
        assert_eq!(d.rope.to_string(), "xyab\nxycd\nxyef");
        assert_eq!(sels.iter().map(|s| s.head).collect::<Vec<_>>(), [2, 7, 12]);
        assert!(d.is_dirty());
        d.undo(&mut sels);
        assert_eq!(d.rope.to_string(), "ab\ncd\nef");
        assert!(!d.is_dirty());
        assert_eq!(sels.len(), 3);
        d.redo(&mut sels);
        assert_eq!(d.rope.to_string(), "xyab\nxycd\nxyef");
        d.backspace(&mut sels, false, true);
        assert_eq!(d.rope.to_string(), "xab\nxcd\nxef");
    }

    #[test]
    fn selections_merge_and_replace() {
        let mut sels = vec![Sel::new(0, 4), Sel::new(2, 6), Sel::caret(6), Sel::caret(9)];
        merge_sels(&mut sels);
        assert_eq!(sels.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>(), [(0, 6), (9, 9)]);
        let mut d = doc("hello world");
        let mut sels = vec![Sel::new(0, 5)];
        d.insert_text(&mut sels, "bye");
        assert_eq!(d.rope.to_string(), "bye world");
        assert_eq!(sels[0].head, 3);
    }

    #[test]
    fn indent_outdent_comment_and_newline() {
        let mut d = doc("fn a() {\n    b();\n}\n");
        let mut sels = vec![Sel::new(0, 19)];
        d.indent(&mut sels, "    ");
        assert_eq!(d.rope.to_string(), "    fn a() {\n        b();\n    }\n");
        d.outdent(&mut sels, 4);
        assert_eq!(d.rope.to_string(), "fn a() {\n    b();\n}\n");
        d.toggle_comment(&mut sels, "//", None);
        assert_eq!(d.rope.to_string(), "// fn a() {\n//     b();\n// }\n");
        d.toggle_comment(&mut sels, "//", None);
        assert_eq!(d.rope.to_string(), "fn a() {\n    b();\n}\n");

        let mut h = doc("<p>");
        let mut sels = vec![Sel::caret(1)];
        h.toggle_comment(&mut sels, "<!--", Some("-->"));
        assert_eq!(h.rope.to_string(), "<!-- <p> -->");
        h.toggle_comment(&mut sels, "<!--", Some("-->"));
        assert_eq!(h.rope.to_string(), "<p>");

        let mut n = doc("    if x {}");
        let mut sels = vec![Sel::caret(10)];
        n.newline(&mut sels, "    ");
        assert_eq!(n.rope.to_string(), "    if x {\n        \n    }");
        assert_eq!(sels[0].head, 19);
    }

    #[test]
    fn motions_words_find_and_brackets() {
        let d = doc("let foo_bar = (baz + 1);\nnext");
        assert_eq!(d.word_right(0), 3);
        assert_eq!(d.word_right(3), 11);
        assert_eq!(d.word_left(11), 4);
        assert_eq!(d.word_at(6), (4, 11));
        assert_eq!(d.matching_bracket(14), Some((14, 22)));
        assert_eq!(d.matching_bracket(23), Some((14, 22)));
        let mut sels = vec![Sel::caret(20)];
        d.move_sels(&mut sels, Motion::Down, false);
        assert_eq!(sels[0].head, 29);
        d.move_sels(&mut sels, Motion::Up, false);
        assert_eq!(sels[0].head, 20);
        d.move_sels(&mut sels, Motion::Home, true);
        assert_eq!((sels[0].anchor, sels[0].head), (20, 0));

        let f = doc("Foo foo food\nfoo");
        assert_eq!(f.find_all("foo", false, false).len(), 4);
        assert_eq!(f.find_all("foo", true, true), vec![(4, 7), (13, 16)]);
        let mut sels = vec![Sel::caret(5)];
        f.select_next_occurrence(&mut sels);
        f.select_next_occurrence(&mut sels);
        f.select_next_occurrence(&mut sels);
        assert_eq!(sels.iter().map(|s| s.start()).collect::<Vec<_>>(), [4, 8, 13]);
    }

    #[test]
    fn wraps_at_words_and_moves_by_visual_rows() {
        let d = doc("alpha beta gamma\nabcdefghijklmno\nz");
        assert_eq!(d.cuts(0, Some(10)), [0, 6]);
        assert_eq!(d.cuts(1, Some(10)), [0, 10]);
        assert_eq!(d.cuts(0, None), [0]);
        let mut sels = vec![Sel::caret(2)];
        let heads = |d: &Document, sels: &mut Vec<Sel>, down| {
            d.move_rows(sels, down, 1, false, Some(10), &[]);
            sels[0].head
        };
        assert_eq!([true, true, true, true, true].map(|down| heads(&d, &mut sels, down)), [8, 19, 29, 34, 34]);
        assert_eq!([false, false].map(|down| heads(&d, &mut sels, down)), [29, 19]);
        let mut sels = vec![Sel::caret(2)];
        d.move_rows(&mut sels, true, 1, false, None, &[(0, 1)]);
        assert_eq!(sels[0].head, 34);
        assert_eq!((hidden(&[(0, 1)], 1), hidden(&[(0, 1)], 0), hidden(&[(0, 1)], 2)), (Some(0), None, None));
    }

    #[test]
    fn closes_surrounds_and_types_over_pairs() {
        let mut d = doc("");
        let mut sels = vec![Sel::caret(0)];
        for c in "f(\"a".chars() {
            d.type_char(&mut sels, c);
        }
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("f(\"a\")", 4));
        d.type_char(&mut sels, '"');
        d.type_char(&mut sels, ')');
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("f(\"a\")", 6));
        d.undo(&mut sels);
        assert_eq!(d.rope.to_string(), "");

        let mut d = doc("don word x()");
        let mut sels = vec![Sel::caret(3)];
        d.type_char(&mut sels, '\'');
        assert_eq!(d.rope.to_string(), "don' word x()");
        let mut sels = vec![Sel::new(5, 9)];
        d.type_char(&mut sels, '[');
        assert_eq!((d.rope.to_string().as_str(), sels[0].start(), sels[0].end()), ("don' [word] x()", 6, 10));
        let mut sels = vec![Sel::caret(14)];
        d.backspace(&mut sels, false, true);
        assert_eq!(d.rope.to_string(), "don' [word] x");
    }

    #[test]
    fn folds_follow_indentation() {
        let d = doc("fn a() {\n    if x {\n\n        y();\n    }\n}\n\nfn b() {}\n");
        assert_eq!((d.fold_end(0), d.fold_end(1), d.fold_end(3), d.fold_end(7)), (Some(4), Some(3), None, None));
        assert!(d.foldable(0) && !d.foldable(2) && !d.foldable(5));
    }

    #[test]
    fn finds_and_resolves_conflicts() {
        let text = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\nb\n<<<<<<< HEAD\nx\n||||||| base\no\n=======\ny\n>>>>>>> branch";
        let mut d = doc(text);
        let mut sels = vec![Sel::caret(0)];
        assert_eq!(d.conflicts(), [[1, 3, 3, 5], [7, 9, 11, 13]]);
        d.resolve(&mut sels, [7, 9, 11, 13], Take::Both);
        d.resolve(&mut sels, [1, 3, 3, 5], Take::Incoming);
        assert_eq!(d.rope.to_string(), "a\ntheirs\nb\nx\ny\n");
        assert!(d.conflicts().is_empty());
        d.undo(&mut sels);
        assert_eq!(d.conflicts(), [[1, 3, 3, 5]]);
        d.resolve(&mut sels, [1, 3, 3, 5], Take::Current);
        assert_eq!(d.rope.to_string(), "a\nours\nb\nx\ny\n");
    }

    #[test]
    fn crlf_and_bom_round_trip() {
        let path = std::env::temp_dir().join(format!("halo-buffer-test-{}.txt", std::process::id()));
        std::fs::write(&path, "\u{feff}a\r\nb\r\n").unwrap();
        let mut d = Document::load(&path).unwrap();
        assert_eq!(d.rope.to_string(), "a\nb\n");
        let mut sels = vec![Sel::caret(1)];
        d.insert_text(&mut sels, "!");
        d.save().unwrap();
        assert!(!d.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), "\u{feff}a!\r\nb\r\n".as_bytes());
        std::fs::remove_file(&path).unwrap();
    }
}
