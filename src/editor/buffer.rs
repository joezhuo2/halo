use ropey::Rope;
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

pub fn same(a: &[Sel], b: &[Sel]) -> bool {
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
    pub encoding: Encoding,
    pub indent: Option<(bool, usize)>,
    pub disk: Option<std::time::SystemTime>,
    pub version: u64,
    saved: u64,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    pub dirty_from: Option<usize>,
    pub note: Option<String>,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Latin1,
}

impl Encoding {
    pub const ALL: [Self; 5] = [Self::Utf8, Self::Utf8Bom, Self::Utf16Le, Self::Utf16Be, Self::Latin1];

    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Latin1 => "Western (ISO 8859-1)",
        }
    }

    pub(crate) fn detect(bytes: &[u8]) -> Self {
        match bytes {
            [0xEF, 0xBB, 0xBF, ..] => Self::Utf8Bom,
            [0xFF, 0xFE, ..] => Self::Utf16Le,
            [0xFE, 0xFF, ..] => Self::Utf16Be,
            _ => Self::Utf8,
        }
    }

    pub(crate) fn decode(self, bytes: &[u8]) -> (String, bool) {
        let little = self == Self::Utf16Le;
        match self {
            Self::Utf8 | Self::Utf8Bom => {
                let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
                match std::str::from_utf8(bytes) {
                    Ok(text) => (text.to_owned(), false),
                    Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
                }
            }
            Self::Utf16Le | Self::Utf16Be => {
                let bytes = bytes.strip_prefix(if little { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] }).unwrap_or(bytes);
                let unit = |pair: &[u8; 2]| if little { u16::from_le_bytes(*pair) } else { u16::from_be_bytes(*pair) };
                let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(unit).collect();
                match String::from_utf16(&units) {
                    Ok(text) => (text, bytes.len() % 2 == 1),
                    Err(_) => (String::from_utf16_lossy(&units), true),
                }
            }
            Self::Latin1 => (bytes.iter().map(|b| *b as char).collect(), false),
        }
    }

    fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
            Self::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
            Self::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
            Self::Latin1 => text.chars().map(|c| u8::try_from(c as u32).unwrap_or(b'?')).collect(),
        }
    }
}

pub struct Pattern {
    re: fancy_regex::Regex,
    regex: bool,
}

impl Pattern {
    pub fn new(text: &str, case: bool, word: bool, regex: bool) -> Result<Self, String> {
        let mut source = if regex { text.to_owned() } else { fancy_regex::escape(text).into_owned() };
        if word {
            source = format!(r"\b(?:{source})\b");
        }
        let flags = if case { "(?m)" } else { "(?mi)" };
        fancy_regex::Regex::new(&format!("{flags}{source}")).map(|re| Self { re, regex }).map_err(|e| e.to_string())
    }

    pub fn find(&self, hay: &str) -> Vec<(usize, usize)> {
        let found = self.re.find_iter(hay).map_while(Result::ok).filter(|m| m.end() > m.start());
        found.map(|m| (m.start(), m.end())).take(20_000).collect()
    }

    pub fn replacement(&self, found: &str, with: &str, preserve: bool) -> String {
        let mut text = with.to_owned();
        // shortcut: groups come from matching the found text alone, upgrade to whole-buffer captures if look-around replaces go wrong
        if self.regex
            && let Ok(Some(groups)) = self.re.captures(found)
        {
            text.clear();
            groups.expand(with, &mut text);
        }
        let letters = || found.chars().filter(|c| c.is_alphabetic());
        match letters().next() {
            Some(_) if !preserve => text,
            Some(_) if letters().all(char::is_uppercase) => text.to_uppercase(),
            Some(_) if letters().all(char::is_lowercase) => text.to_lowercase(),
            Some(first) if first.is_uppercase() => {
                let mut chars = text.chars();
                chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
            }
            _ => text,
        }
    }
}

const PAIRS: [(char, char); 6] = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];

static SEPARATORS: [std::sync::atomic::AtomicU64; 2] = [std::sync::atomic::AtomicU64::new(0xfc00_fffe_0000_0000), std::sync::atomic::AtomicU64::new(0x7800_0001_7800_0001)];

pub fn separator_mask(separators: &str) -> [u64; 2] {
    let mut mask = [0u64; 2];
    for c in separators.chars().filter(char::is_ascii) {
        mask[c as usize / 64] |= 1 << (c as usize % 64);
    }
    mask
}

pub fn set_word_separators(separators: &str) {
    for (slot, bits) in SEPARATORS.iter().zip(separator_mask(separators)) {
        slot.store(bits, std::sync::atomic::Ordering::Relaxed);
    }
}

fn is_word_in(c: char, mask: [u64; 2]) -> bool {
    match c.is_ascii() {
        true => !c.is_ascii_whitespace() && !c.is_ascii_control() && mask[c as usize / 64] & (1 << (c as usize % 64)) == 0,
        false => c.is_alphanumeric(),
    }
}

pub fn is_word(c: char) -> bool {
    is_word_in(c, SEPARATORS.each_ref().map(|s| s.load(std::sync::atomic::Ordering::Relaxed)))
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

pub fn disk_time(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
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
            encoding: Encoding::Utf8,
            indent: None,
            disk: None,
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
        Self::load_as(path, None)
    }

    pub fn load_as(path: &Path, encoding: Option<Encoding>) -> Result<Self, String> {
        let name = title_of(path);
        let size = std::fs::metadata(path).map_err(|e| format!("Cannot open {name}: {e}"))?.len();
        if size > 50 * 1024 * 1024 {
            return Err(format!("{name} is larger than 50 MB and was not opened."));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot open {name}: {e}"))?;
        let encoding = encoding.unwrap_or_else(|| Encoding::detect(&bytes));
        let wide = matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be);
        if !wide && bytes.iter().take(8192).any(|b| *b == 0) {
            return Err(format!("{name} looks like a binary file and was not opened."));
        }
        let (mut text, lossy) = encoding.decode(&bytes);
        let crlf = text.contains("\r\n");
        if crlf {
            text = text.replace("\r\n", "\n");
        }
        let mut doc = Self::from_text(&text, name);
        doc.path = Some(path.to_path_buf());
        doc.crlf = crlf;
        doc.encoding = encoding;
        doc.disk = disk_time(path);
        doc.indent = doc.detect_indent();
        if lossy {
            doc.note = Some(format!("This file is not valid {}; invalid bytes were replaced.", encoding.label()));
        }
        Ok(doc)
    }

    pub fn save(&mut self) -> Result<Vec<u8>, String> {
        let Some(path) = self.path.clone() else { return Err("No file name.".into()) };
        let mut text = self.rope.to_string();
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        let bytes = self.encoding.encode(&text);
        std::fs::write(&path, &bytes).map_err(|e| format!("Cannot save {}: {e}", title_of(&path)))?;
        self.saved = self.version;
        self.disk = disk_time(&path);
        self.title = title_of(&path);
        Ok(bytes)
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

    pub fn detect_indent(&self) -> Option<(bool, usize)> {
        let (mut tabs, mut widths) = (0, [0usize; 9]);
        for line in self.rope.lines().take(2000) {
            match line.get_char(0) {
                Some('\t') => tabs += 1,
                Some(' ') => widths[line.chars().take(8).take_while(|c| *c == ' ').count()] += 1,
                _ => {}
            }
        }
        let spaced: usize = widths[2..].iter().sum();
        match (2..9).find(|width| widths[*width] > 0) {
            _ if tabs > spaced => Some((false, 0)),
            Some(width) => Some((true, width)),
            None => None,
        }
    }

    pub fn convert_indent(&mut self, sels: &mut Vec<Sel>, spaces: bool, size: usize) -> bool {
        let size = size.max(1);
        let mut edits: EditList = vec![];
        for line in 0..self.lines() {
            let lead: String = self.rope.line(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let width = lead.chars().fold(0, |width, c| if c == '\t' { (width / size + 1) * size } else { width + 1 });
            let fresh = if spaces { " ".repeat(width) } else { "\t".repeat(width / size) + &" ".repeat(width % size) };
            if fresh != lead {
                let start = self.line_start(line);
                edits.push((start, start + lead.chars().count(), fresh));
            }
        }
        self.indent = Some((spaces, size));
        self.apply(sels, edits, Kind::Other, After::Map)
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

    pub fn type_char(&mut self, sels: &mut Vec<Sel>, c: char, pairs: bool, surround: bool) -> bool {
        let at = |pos: usize| self.rope.get_char(pos);
        // shortcut: any matching closer is typed over, upgrade to tracking the inserted ones if that swallows wanted input
        if pairs && PAIRS.iter().any(|p| p.1 == c) && sels.iter().all(|s| s.is_empty() && at(s.head) == Some(c)) {
            sels.iter_mut().for_each(|s| *s = Sel::caret(s.head + 1));
            return true;
        }
        let Some(close) = PAIRS.iter().find(|p| p.0 == c).map(|p| p.1) else { return self.insert_text(sels, &c.to_string()) };
        let selected = sels.iter().all(|s| !s.is_empty());
        if (selected && !surround) || (!selected && !pairs) {
            return self.insert_text(sels, &c.to_string());
        }
        if selected {
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

    pub fn newline(&mut self, sels: &mut Vec<Sel>, unit: &str, auto: &str) -> bool {
        let (keep, smart) = (auto != "none", !matches!(auto, "none" | "keep"));
        let mut back = vec![];
        let edits: EditList = sels
            .iter()
            .map(|s| {
                let (start, end) = (s.start(), s.end());
                let line = self.line_of(start);
                let column = start - self.line_start(line);
                let indent: String = self.rope.line(line).chars().take(column * keep as usize).take_while(|c| *c == ' ' || *c == '\t').collect();
                let prev = (start > 0).then(|| self.rope.char(start - 1));
                let next = (end < self.len()).then(|| self.rope.char(end));
                let opens = smart && matches!(prev, Some('{' | '(' | '[' | ':'));
                let closes = smart && matches!((prev, next), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')));
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

    fn blocks(&self, sels: &[Sel]) -> Vec<(usize, usize)> {
        let mut out: Vec<(usize, usize)> = vec![];
        for line in self.touched_lines(sels) {
            match out.last_mut() {
                Some(last) if last.1 + 1 == line => last.1 = line,
                _ => out.push((line, line)),
            }
        }
        out
    }

    pub fn move_lines(&mut self, sels: &mut Vec<Sel>, down: bool) -> bool {
        let mut edits: EditList = vec![];
        let mut moved = sels.clone();
        for (a, b) in self.blocks(sels) {
            let (first, last) = if down { (a, b + 1) } else { (a.saturating_sub(1), b) };
            if (!down && a == 0) || last >= self.lines() {
                continue;
            }
            let mut lines: Vec<String> = (first..=last).map(|l| self.line_text(l)).collect();
            let other = if down { lines.pop() } else { Some(lines.remove(0)) }.unwrap_or_default();
            let shift = (other.chars().count() as isize + 1) * if down { 1 } else { -1 };
            lines.insert(if down { 0 } else { lines.len() }, other);
            edits.push((self.line_start(first), self.line_end(last), lines.join("\n")));
            let (from, to) = (self.line_start(a), self.line_end(b));
            for s in moved.iter_mut().filter(|s| s.start() >= from && s.start() <= to) {
                (s.anchor, s.head) = ((s.anchor as isize + shift) as usize, (s.head as isize + shift) as usize);
            }
        }
        let changed = self.apply(sels, edits, Kind::Other, After::Map);
        if changed {
            *sels = moved;
            self.fix_after(sels);
        }
        changed
    }

    pub fn copy_lines(&mut self, sels: &mut Vec<Sel>, down: bool) -> bool {
        let blocks = self.blocks(sels);
        let texts: Vec<String> = blocks.iter().map(|(a, b)| self.text_of(self.line_start(*a), self.line_end(*b)) + "\n").collect();
        let own: Vec<usize> = sels
            .iter()
            .map(|s| {
                let line = self.line_of(s.start());
                blocks.iter().position(|b| b.0 <= line && line <= b.1).map_or(0, |at| texts[at].chars().count())
            })
            .collect();
        let edits = blocks.iter().zip(texts).map(|(block, text)| (self.line_start(block.0), self.line_start(block.0), text)).collect();
        let changed = self.apply(sels, edits, Kind::Other, After::Map);
        if changed && !down && sels.len() == own.len() {
            for (s, back) in sels.iter_mut().zip(own) {
                (s.anchor, s.head) = (s.anchor - back, s.head - back);
            }
            self.fix_after(sels);
        }
        changed
    }

    pub fn delete_lines(&mut self, sels: &mut Vec<Sel>) -> bool {
        let edits = self
            .blocks(sels)
            .into_iter()
            .map(|(a, b)| match b + 1 < self.lines() {
                true => (self.line_start(a), self.line_start(b + 1), String::new()),
                false => (self.line_end(a.saturating_sub(1)).min(self.line_start(a)), self.len(), String::new()),
            })
            .collect();
        self.apply(sels, edits, Kind::Other, After::Carets)
    }

    pub fn join_lines(&mut self, sels: &mut Vec<Sel>) -> bool {
        let mut edits: EditList = vec![];
        for (a, b) in self.blocks(sels) {
            for line in (a..b.max(a + 1)).filter(|line| line + 1 < self.lines()) {
                let from = self.line_start(line) + self.line_text(line).trim_end().chars().count();
                let to = self.line_start(line + 1) + self.indent_of(line + 1);
                let bare = from == self.line_start(line) || to == self.line_end(line + 1);
                edits.push((from, to, if bare { String::new() } else { " ".to_owned() }));
            }
        }
        self.apply(sels, edits, Kind::Other, After::Map)
    }

    pub fn insert_line(&mut self, sels: &mut Vec<Sel>, above: bool) -> bool {
        let mut lines: Vec<usize> = sels.iter().map(|s| self.line_of(s.head)).collect();
        lines.dedup();
        let edits: EditList = lines
            .into_iter()
            .map(|line| {
                let indent: String = self.rope.line(line).chars().take(self.indent_of(line)).collect();
                match above {
                    true => (self.line_start(line), self.line_start(line), format!("{indent}\n")),
                    false => (self.line_end(line), self.line_end(line), format!("\n{indent}")),
                }
            })
            .collect();
        let changed = self.apply(sels, edits, Kind::Other, After::Carets);
        if changed && above {
            sels.iter_mut().for_each(|s| *s = Sel::caret(s.head - 1));
            self.fix_after(sels);
        }
        changed
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

    pub fn toggle_block_comment(&mut self, sels: &mut Vec<Sel>, open: &str, close: &str) -> bool {
        let (o, c) = (open.chars().count(), close.chars().count());
        let mut edits: EditList = vec![];
        let mut widen = vec![];
        for s in sels.iter() {
            let (mut from, mut to) = (s.start(), s.end());
            if s.is_empty() {
                let line = self.line_of(from);
                (from, to) = (self.line_start(line) + self.indent_of(line), self.line_end(line));
            }
            let text = self.text_of(from, to);
            let body = text.trim();
            let start = from + text.chars().count() - text.trim_start().chars().count();
            let end = start + body.chars().count();
            widen.push(0);
            if body.len() >= open.len() + close.len() && body.starts_with(open) && body.ends_with(close) {
                let inner = &body[open.len()..body.len() - close.len()];
                edits.push((start, start + o + usize::from(inner.starts_with(' ')), String::new()));
                edits.push((end - c - usize::from(inner.len() > 1 && inner.ends_with(' ')), end, String::new()));
            } else if from < to {
                edits.push((from, from, format!("{open} ")));
                edits.push((to, to, format!(" {close}")));
                widen.pop();
                widen.push(if s.is_empty() { 0 } else { o + 1 });
            }
        }
        let changed = self.apply(sels, edits, Kind::Other, After::Map);
        if changed && sels.len() == widen.len() {
            for (s, by) in sels.iter_mut().zip(widen) {
                match s.head > s.anchor {
                    true => s.anchor -= by,
                    false => s.head -= by,
                }
            }
            self.fix_after(sels);
        }
        changed
    }

    // shortcut: whitespace right before a caret is kept so auto save does not eat what is being typed
    pub fn tidy(&mut self, sels: &mut Vec<Sel>, trim: bool, newline: bool, final_newlines: bool) -> bool {
        let mut edits: EditList = vec![];
        for line in (0..self.lines()).filter(|_| trim) {
            let end = self.line_end(line);
            let mut before = self.rope.chars_at(end);
            let blanks = std::iter::from_fn(|| before.prev()).take_while(|c| *c == ' ' || *c == '\t').count();
            if blanks > 0 && !sels.iter().any(|s| s.head == end) {
                edits.push((end - blanks, end, String::new()));
            }
        }
        let open = newline && self.len() > 0 && self.rope.char(self.len() - 1) != '\n';
        if open {
            edits.push((self.len(), self.len(), "\n".to_owned()));
        }
        let len = self.len();
        let extra = (0..len).rev().take_while(|&i| self.rope.char(i) == '\n').count().saturating_sub(1);
        if final_newlines && extra > 0 && !sels.iter().any(|s| s.head > len - extra) {
            edits.push((len - extra, len, String::new()));
        }
        let changed = self.apply(sels, edits, Kind::Other, After::Map);
        let len = self.len();
        for s in sels.iter_mut().filter(|_| open && changed) {
            *s = Sel::new(s.anchor - usize::from(s.anchor == len), s.head - usize::from(s.head == len));
        }
        changed
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

    pub fn find_all(&self, pattern: &Pattern, within: Option<(usize, usize)>) -> Vec<(usize, usize)> {
        let hay = self.rope.to_string();
        let mut out = vec![];
        let (mut byte, mut chars) = (0, 0);
        for (from, to) in pattern.find(&hay) {
            chars += hay[byte..from].chars().count();
            byte = from;
            let end = chars + hay[from..to].chars().count();
            if within.is_none_or(|(a, b)| chars >= a && end <= b) {
                out.push((chars, end));
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
        let matches = self.find_all(&Pattern::new(&needle, true, false, false).ok()?, None);
        let taken = |m: &(usize, usize)| sels.iter().any(|s| s.start() == m.0 && s.end() == m.1);
        let next = matches.iter().find(|m| m.0 >= last.end() && !taken(m)).or_else(|| matches.iter().find(|m| !taken(m)))?;
        sels.push(Sel::new(next.0, next.1));
        merge_sels(sels);
        Some(next.1)
    }

    // shortcut: box columns count characters, upgrade to visual columns if tabs or wide glyphs skew the box
    pub fn box_sels(&self, anchor: usize, head: usize) -> Vec<Sel> {
        let (first, last) = (self.line_of(anchor), self.line_of(head));
        let (from, to) = (anchor - self.line_start(first), head - self.line_start(last));
        let on = |line: usize| Sel::new(self.line_start(line) + from.min(self.line_len(line)), self.line_start(line) + to.min(self.line_len(line)));
        (first.min(last)..=first.max(last)).map(on).collect()
    }

    pub fn select_all_occurrences(&self, sels: &mut Vec<Sel>) {
        let Some(first) = sels.first().copied() else { return };
        let (from, to) = if first.is_empty() { self.word_at(first.head) } else { (first.start(), first.end()) };
        let text = self.text_of(from, to);
        let word = first.is_empty() && text.chars().all(is_word);
        let found = Pattern::new(&text, true, word, false).map(|pattern| self.find_all(&pattern, None)).unwrap_or_default();
        if !found.is_empty() {
            *sels = found.into_iter().map(|m| Sel::new(m.0, m.1)).collect();
        }
    }

    pub fn enclosing(&self, from: usize, to: usize) -> Option<(usize, usize)> {
        let (mut at, mut depth) = (from.min(self.len()), 0);
        while at > 0 && from - at < 20_000 {
            at -= 1;
            match self.rope.char(at) {
                ')' | ']' | '}' => depth += 1,
                '(' | '[' | '{' if depth > 0 => depth -= 1,
                '(' | '[' | '{' => {
                    if let Some(pair) = self.matching_bracket(at).filter(|pair| pair.0 == at && pair.1 >= to) {
                        return Some(pair);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn grow(&self, s: Sel) -> Sel {
        let (from, to) = (s.start(), s.end());
        let (first, last) = (self.line_of(from), self.line_of(to));
        let mut options = vec![
            (self.line_start(first) + self.indent_of(first), self.line_end(last)),
            (self.line_start(first), self.line_start(last + 1)),
            (0, self.len()),
        ];
        let word = self.word_at(s.head);
        if self.rope.get_char(word.0).is_some_and(is_word) {
            options.push(word);
        }
        if let Some((open, close)) = self.enclosing(from, to) {
            options.extend([(open + 1, close), (open, close + 1)]);
        }
        let wider = options.into_iter().filter(|o| o.0 <= from && o.1 >= to && o.1 - o.0 > to - from);
        wider.min_by_key(|o| o.1 - o.0).map_or(s, |o| Sel::new(o.0, o.1))
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
        n.newline(&mut sels, "    ", "full");
        assert_eq!(n.rope.to_string(), "    if x {\n        \n    }");
        assert_eq!(sels[0].head, 19);
        let mut k = doc("    if x {}");
        let mut sels = vec![Sel::caret(10)];
        k.newline(&mut sels, "    ", "keep");
        assert_eq!(k.rope.to_string(), "    if x {\n    }");
        let mut sels = vec![Sel::caret(10)];
        let mut none = doc("    if x {}");
        none.newline(&mut sels, "    ", "none");
        assert_eq!((none.rope.to_string().as_str(), sels[0].head), ("    if x {\n}", 11));
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
        let find = |text: &str, case, word, regex, within| f.find_all(&Pattern::new(text, case, word, regex).unwrap(), within);
        assert_eq!(find("foo", false, false, false, None).len(), 4);
        assert_eq!(find("foo", true, true, false, None), vec![(4, 7), (13, 16)]);
        assert_eq!(find("fo+d?$", true, false, true, None), vec![(8, 12), (13, 16)]);
        assert_eq!(find("foo", false, false, false, Some((3, 12))), vec![(4, 7), (8, 11)]);
        assert_eq!(find("o.o", true, false, false, None), vec![]);
        assert!(Pattern::new("(", true, false, true).is_err());
        let swap = Pattern::new(r"(\w+)=(\w+)", true, false, true).unwrap();
        assert_eq!(swap.replacement("a=b", "$2=$1", false), "b=a");
        let plain = Pattern::new("foo", false, false, false).unwrap();
        assert_eq!(["foo", "Foo", "FOO"].map(|found| plain.replacement(found, "bar", true)), ["bar", "Bar", "BAR"]);
        assert_eq!(plain.replacement("Foo", "$1", false), "$1");
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
    fn word_separators_decide_what_a_word_is() {
        let default = separator_mask(crate::settings::WORD_SEPARATORS);
        assert_eq!(default, SEPARATORS.each_ref().map(|s| s.load(std::sync::atomic::Ordering::Relaxed)));
        for c in (0..128u8).map(char::from).chain(['\u{e9}', '\u{b7}', '\u{4e2d}']) {
            assert_eq!(is_word_in(c, default), c.is_alphanumeric() || c == '_', "{c:?}");
        }
        let dashes = separator_mask("()");
        assert!(is_word_in('-', dashes) && is_word_in('$', dashes) && !is_word_in('(', dashes) && !is_word_in(' ', dashes));
    }

    #[test]
    fn closes_surrounds_and_types_over_pairs() {
        let mut d = doc("");
        let mut sels = vec![Sel::caret(0)];
        for c in "f(\"a".chars() {
            d.type_char(&mut sels, c, true, true);
        }
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("f(\"a\")", 4));
        d.type_char(&mut sels, '"', true, true);
        d.type_char(&mut sels, ')', true, true);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("f(\"a\")", 6));
        d.undo(&mut sels);
        assert_eq!(d.rope.to_string(), "");

        let mut d = doc("don word x()");
        let mut sels = vec![Sel::caret(3)];
        d.type_char(&mut sels, '\'', true, true);
        assert_eq!(d.rope.to_string(), "don' word x()");
        let mut sels = vec![Sel::new(5, 9)];
        d.type_char(&mut sels, '[', true, true);
        assert_eq!((d.rope.to_string().as_str(), sels[0].start(), sels[0].end()), ("don' [word] x()", 6, 10));
        let mut plain = doc("ab");
        let mut sels = vec![Sel::caret(1)];
        plain.type_char(&mut sels, '(', false, true);
        assert_eq!(plain.rope.to_string(), "a(b");
        let mut sels = vec![Sel::new(0, 1)];
        plain.type_char(&mut sels, '"', true, false);
        assert_eq!(plain.rope.to_string(), "\"(b");
        let mut sels = vec![Sel::new(2, 3)];
        plain.type_char(&mut sels, '[', false, true);
        assert_eq!(plain.rope.to_string(), "\"([b]");
        let mut sels = vec![Sel::caret(14)];
        d.backspace(&mut sels, false, true);
        assert_eq!(d.rope.to_string(), "don' [word] x");
    }

    #[test]
    fn moves_copies_deletes_joins_and_inserts_lines() {
        let mut d = doc("a\nbb\nccc\nd");
        let mut sels = vec![Sel::new(3, 6)];
        d.move_lines(&mut sels, true);
        assert_eq!((d.rope.to_string().as_str(), sels[0].start(), sels[0].end()), ("a\nd\nbb\nccc", 5, 8));
        d.move_lines(&mut sels, true);
        d.move_lines(&mut sels, false);
        d.move_lines(&mut sels, false);
        assert_eq!((d.rope.to_string().as_str(), sels[0].start()), ("bb\nccc\na\nd", 1));
        d.undo(&mut sels);
        assert_eq!((d.rope.to_string().as_str(), sels[0].start()), ("a\nbb\nccc\nd", 3));

        let mut sels = vec![Sel::caret(3)];
        d.copy_lines(&mut sels, true);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("a\nbb\nbb\nccc\nd", 6));
        d.copy_lines(&mut sels, false);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("a\nbb\nbb\nbb\nccc\nd", 6));
        let mut sels = vec![Sel::new(2, 9)];
        d.delete_lines(&mut sels);
        assert_eq!(d.rope.to_string(), "a\nccc\nd");
        let mut sels = vec![Sel::caret(7)];
        d.delete_lines(&mut sels);
        assert_eq!(d.rope.to_string(), "a\nccc");

        let mut d = doc("let x =  \n    1;\n\nend");
        let mut sels = vec![Sel::caret(0)];
        d.join_lines(&mut sels);
        assert_eq!(d.rope.to_string(), "let x = 1;\n\nend");
        let mut d = doc("  one\n  two");
        let mut sels = vec![Sel::caret(9)];
        d.insert_line(&mut sels, true);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("  one\n  \n  two", 8));
        d.insert_line(&mut sels, false);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("  one\n  \n  \n  two", 11));
    }

    #[test]
    fn toggles_block_comments() {
        let mut d = doc("a = b + c;\n  next();");
        let mut sels = vec![Sel::new(4, 9), Sel::caret(15)];
        d.toggle_block_comment(&mut sels, "/*", "*/");
        assert_eq!(d.rope.to_string(), "a = /* b + c */;\n  /* next(); */");
        assert_eq!((sels[0].start(), sels[0].end()), (4, 15));
        d.toggle_block_comment(&mut sels, "/*", "*/");
        assert_eq!(d.rope.to_string(), "a = b + c;\n  next();");
        assert_eq!((sels[0].start(), sels[0].end()), (4, 9));
    }

    #[test]
    fn selects_boxes_occurrences_and_growing_ranges() {
        let d = doc("let a = f(x, [yy, z]);\nlet yy = 1;\nyyy");
        let spans = |sels: &[Sel]| sels.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>();
        assert_eq!(spans(&d.box_sels(4, 37)), vec![(2, 4), (25, 27), (37, 38)]);
        let mut sels = vec![Sel::caret(15)];
        d.select_all_occurrences(&mut sels);
        assert_eq!(spans(&sels), vec![(14, 16), (27, 29)]);
        let mut sels = vec![Sel::new(14, 16)];
        d.select_all_occurrences(&mut sels);
        assert_eq!(sels.len(), 3);

        assert_eq!(d.enclosing(15, 15), Some((13, 19)));
        let mut s = Sel::caret(15);
        let mut seen = vec![];
        for _ in 0..8 {
            s = d.grow(s);
            seen.push((s.start(), s.end()));
        }
        assert_eq!(seen, vec![(14, 16), (14, 19), (13, 20), (10, 20), (9, 21), (0, 22), (0, 23), (0, 34)]);
    }

    #[test]
    fn tidies_trailing_whitespace_and_the_final_newline() {
        let mut d = doc("a  \n\tb \t\n  \nc ");
        let mut sels = vec![Sel::caret(14)];
        assert!(d.tidy(&mut sels, true, true, false));
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("a\n\tb\n\nc \n", 8));
        assert!(!d.tidy(&mut sels, true, true, false));
        let mut sels = vec![Sel::caret(0)];
        assert!(d.tidy(&mut sels, true, false, false));
        assert_eq!(d.rope.to_string(), "a\n\tb\n\nc\n");
        let mut f = doc("a\n\n\n");
        let mut sels = vec![Sel::caret(4)];
        assert!(!f.tidy(&mut sels, false, true, true));
        let mut sels = vec![Sel::caret(0)];
        assert!(f.tidy(&mut sels, false, true, true));
        assert_eq!(f.rope.to_string(), "a\n");
        assert!(!f.tidy(&mut sels, false, false, true));
    }

    #[test]
    fn detects_and_converts_indentation_and_round_trips_encodings() {
        assert_eq!(doc("a\n  b\n    c\n").detect_indent(), Some((true, 2)));
        assert_eq!(doc("a\n\tb\n\t\tc\n    d").detect_indent(), Some((false, 0)));
        assert_eq!(doc("a\nb").detect_indent(), None);
        let mut d = doc("a\n\tb\n\t  c\n");
        let mut sels = vec![Sel::caret(3)];
        d.convert_indent(&mut sels, true, 4);
        assert_eq!((d.rope.to_string().as_str(), sels[0].head), ("a\n    b\n      c\n", 6));
        d.convert_indent(&mut sels, false, 4);
        assert_eq!(d.rope.to_string(), "a\n\tb\n\t  c\n");

        let path = std::env::temp_dir().join(format!("halo-encoding-test-{}.txt", std::process::id()));
        std::fs::write(&path, [0xFF, 0xFE, b'h', 0, 0xE9, 0, b'\r', 0, b'\n', 0]).unwrap();
        let mut d = Document::load(&path).unwrap();
        assert_eq!((d.rope.to_string().as_str(), d.encoding, d.crlf), ("h\u{e9}\n", Encoding::Utf16Le, true));
        d.encoding = Encoding::Latin1;
        d.crlf = false;
        d.save().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [b'h', 0xE9, b'\n']);
        assert!(Document::load(&path).unwrap().note.is_some());
        assert_eq!(Document::load_as(&path, Some(Encoding::Latin1)).unwrap().rope.to_string(), "h\u{e9}\n");
        for encoding in Encoding::ALL {
            assert_eq!(encoding.decode(&encoding.encode("a\u{e9}")), ("a\u{e9}".to_owned(), false));
        }
        std::fs::remove_file(&path).unwrap();
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
