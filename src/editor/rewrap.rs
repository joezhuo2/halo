use super::{After, Editors, Kind, Settings, block_tokens, comment_tokens};
use eframe::egui;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender};

type Edit = (usize, usize, String);

pub(super) struct ResultEdits {
    doc: u64,
    version: u64,
    group: usize,
    edits: Vec<Edit>,
}

pub(super) type Channel = (Sender<ResultEdits>, Receiver<ResultEdits>);

#[derive(Clone, Copy)]
struct Mode {
    prose: bool,
    line: Option<&'static str>,
    stars: bool,
}

fn columns(text: &str, tab: usize) -> usize {
    text.chars().map(|c| if c == '\t' { tab } else { 1 }).sum()
}

fn trim_indent(s: &str) -> &str {
    s.trim_start_matches([' ', '\t'])
}

fn marker(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let n = match s.as_bytes().first()? {
        b'-' | b'*' | b'+' => 1,
        _ if (1..=9).contains(&digits) && s[digits..].starts_with(['.', ')']) => digits + 1,
        _ => return None,
    };
    s[n..].starts_with([' ', '\t']).then(|| &s[..s.len() - trim_indent(&s[n..]).len()])
}

struct Line<'a> {
    offset: usize,
    raw: &'a str,
    leader: Option<&'a str>,
    prefix: &'a str,
    hanging: String,
    body: &'a str,
    base: &'a str,
}

fn parse_line<'a>(raw: &'a str, offset: usize, leader: Option<&'a str>, tab: usize) -> Line<'a> {
    let content = &raw[leader.map_or(0, str::len)..];
    let mut body = trim_indent(content);
    while let Some(rest) = body.strip_prefix('>') {
        body = trim_indent(rest);
    }
    let base = &raw[..raw.len() - body.len()];
    let bullet = marker(body).unwrap_or("");
    body = &body[bullet.len()..];
    let prefix = &raw[..raw.len() - body.len()];
    let hanging = format!("{base}{}", " ".repeat(columns(bullet, tab)));
    Line { offset, raw, leader, prefix, hanging, body, base }
}

fn lines(text: &str, mode: Mode, tab: usize) -> Vec<Line<'_>> {
    let (mut offset, mut block) = (0, false);
    let mut lines: Vec<_> = text.split('\n').map(|raw| {
        let s = trim_indent(raw);
        let leader = if mode.prose {
            Some("")
        } else if block {
            if s.contains("*/") {
                block = false;
                None
            } else {
                s.strip_prefix('*').filter(|r| r.is_empty() || r.starts_with([' ', '\t']))
                    .map(|rest| &raw[..raw.len() - rest.len()])
            }
        } else if let Some(token) = mode.line.filter(|t| s.starts_with(t)) {
            let n = if token == "//" && (s.starts_with("///") || s.starts_with("//!")) { 3 } else { token.len() };
            Some(&raw[..raw.len() - s.len() + n])
        } else {
            if mode.stars && let Some((_, tail)) = s.split_once("/*") {
                block = !tail.contains("*/");
            }
            None
        };
        let line = parse_line(raw, offset, leader, tab);
        offset += raw.len() + 1;
        line
    }).collect();
    for group in lines.chunk_by_mut(|a, b| a.leader == b.leader) {
        let Some(leader) = group[0].leader else { continue };
        let mut markdown = String::new();
        let mut starts = Vec::with_capacity(group.len());
        for line in &*group {
            starts.push(markdown.len());
            let s = &line.raw[leader.len()..];
            markdown.push_str(if leader.is_empty() { s } else { s.strip_prefix(' ').unwrap_or(s) });
            markdown.push('\n');
        }
        for (event, range) in Parser::new_ext(&markdown, Options::ENABLE_TABLES).into_offset_iter() {
            if matches!(event, Event::Rule | Event::Start(Tag::Heading { .. } | Tag::Table(_) | Tag::CodeBlock(CodeBlockKind::Fenced(_)))) {
                let start = starts.partition_point(|p| *p <= range.start) - 1;
                let end = starts.partition_point(|p| *p < range.end);
                for line in &mut group[start..end] {
                    line.leader = None;
                }
            }
        }
    }
    lines
}

fn rewrap(text: &str, width: usize, tab: usize, mode: Mode, targets: &[(Range<usize>, bool)]) -> Vec<Edit> {
    let lines = lines(text, mode, tab);
    let mut edits = Vec::new();
    let mut start = 0;
    let mut target = 0;
    while start < lines.len() && target < targets.len() {
        let first = &lines[start];
        if first.leader.is_none() || first.body.trim().is_empty() {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while let Some(line) = lines.get(end)
            && line.leader == first.leader && line.prefix == line.base && !line.body.trim().is_empty()
            && (line.prefix == first.hanging || line.prefix == first.base)
        {
            end += 1;
        }
        while target < targets.len() && targets[target].0.end <= start {
            target += 1;
        }
        let mut ranges: Vec<Range<usize>> = vec![];
        for (range, caret) in targets[target..].iter().take_while(|(r, _)| r.start < end) {
            if range.end <= start {
                continue;
            }
            if *caret {
                ranges.clear();
                ranges.push(start..end);
                break;
            }
            let r = start.max(range.start)..end.min(range.end);
            if let Some(last) = ranges.last_mut() && last.end > r.start {
                last.end = last.end.max(r.end);
            } else {
                ranges.push(r);
            }
        }
        for r in ranges {
            wrap_words(text, &lines[r], width, tab, &mut edits);
        }
        start = end;
    }
    edits
}

fn wrap_words(text: &str, lines: &[Line<'_>], width: usize, tab: usize, edits: &mut Vec<Edit>) {
    let first = &lines[0];
    let mut col = columns(first.prefix, tab);
    let next_col = columns(&first.hanging, tab);
    let mut previous = None;
    for line in lines {
        let mut pos = line.offset + line.prefix.len();
        for word in line.body.split_whitespace() {
            pos += text[pos..].find(word).unwrap();
            let len = word.chars().count();
            if let Some(end) = previous {
                let separator = if col.saturating_add(len + 1) > width {
                    col = next_col;
                    format!("\n{}", first.hanging)
                } else {
                    col += 1;
                    " ".into()
                };
                if text[end..pos] != separator {
                    edits.push((end, pos, separator));
                }
            }
            col += len;
            pos += word.len();
            previous = Some(pos);
        }
    }
    let last = lines.last().unwrap();
    let end = last.offset + last.raw.len();
    if let Some(pos) = previous && pos < end {
        edits.push((pos, end, String::new()));
    }
}

impl Editors {
    pub(super) fn start_rewrap(&mut self, g: usize, t: usize, settings: &Settings, ctx: &egui::Context) {
        let tab = &self.groups[g].tabs[t];
        let Some(doc) = self.docs.get(&tab.doc) else { return };
        let buf = &doc.buf;
        if buf.read_only || !doc.content.is_text() || tab.md.is_some() {
            return;
        }
        let path = buf.path.as_deref();
        let prefs = settings.of(path);
        let width = prefs.rulers.first().copied().unwrap_or(prefs.word_wrap_column);
        let size = super::indent_unit(buf, settings).1.max(1);
        let (token, close) = comment_tokens(path);
        let prose = doc.hl.language == "Markdown" || path.is_none_or(|p| {
            let id = crate::lsp::language_id(p);
            id == "markdown" || (id == "plaintext" && doc.hl.language == "Plain Text")
        });
        let mode = Mode { prose, line: close.is_none().then_some(token), stars: block_tokens(path) == Some(("/*", "*/")) };
        let mut targets: Vec<_> = tab.sels.iter().map(|s| {
            let start = buf.line_of(s.start());
            let end = buf.line_of(s.end().saturating_sub(usize::from(!s.is_empty())));
            (start..end + 1, s.is_empty())
        }).collect();
        targets.sort_by_key(|(r, _)| r.start);
        let (id, version, rope) = (tab.doc, buf.version, buf.rope.clone());
        let (tx, ctx) = (self.rewrap.0.clone(), ctx.clone());
        std::thread::spawn(move || {
            let edits = rewrap(&rope.to_string(), width, size, mode, &targets).into_iter()
                .map(|(a, b, s)| (rope.byte_to_char(a), rope.byte_to_char(b), s)).collect();
            let _ = tx.send(ResultEdits { doc: id, version, group: g, edits });
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_rewrap(&mut self) {
        while let Ok(result) = self.rewrap.1.try_recv() {
            let Some(doc) = self.docs.get_mut(&result.doc) else { continue };
            if doc.buf.version == result.version
                && let Some(tab) = self.groups.get_mut(result.group)
                    .and_then(|g| g.tabs.iter_mut().find(|t| t.doc == result.doc && t.md.is_none()))
                && doc.buf.apply(&mut tab.sels, result.edits, Kind::Other, After::Map)
            {
                tab.reveal = tab.sels.last().map(|s| s.head);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::editor::buffer::{Document, Sel};

    const TEXT: Mode = Mode { prose: true, line: None, stars: false };
    const RUST: Mode = Mode { prose: false, line: Some("//"), stars: true };

    fn wrapped(text: &str, width: usize, mode: Mode, targets: &[(Range<usize>, bool)]) -> String {
        let mut result = text.to_owned();
        for (a, b, insert) in rewrap(text, width, 4, mode, targets).into_iter().rev() {
            result.replace_range(a..b, &insert);
        }
        result
    }

    fn all(text: &str, width: usize, mode: Mode) -> String {
        wrapped(text, width, mode, &[(0..usize::MAX, false)])
    }

    #[test]
    fn comment_prefixes_and_code_boundaries() {
        let input = "let a = 42;\n\t// one two three\n\t// four\n/// five six seven\n//! end";
        assert_eq!(all(input, 14, RUST), "let a = 42;\n\t// one two\n\t// three\n\t// four\n/// five six\n/// seven\n//! end");
        for token in ["#", "--"] {
            let mode = Mode { line: Some(token), ..RUST };
            let input = format!("{token} one\n{token} two three");
            assert_eq!(all(&input, 10, mode), format!("{token} one two\n{token} three"));
        }
        assert_eq!(all("/*\n * a b\n * c d e\n */\n * ordinary code", 8, RUST),
            "/*\n * a b c\n * d e\n */\n * ordinary code");
    }

    #[test]
    fn quote_list_indent_and_long_words() {
        assert_eq!(all("> two\n> words fit", 12, TEXT), "> two words\n> fit");
        for bullet in ["- ", "* ", "1. "] {
            let pad = " ".repeat(bullet.len());
            let input = format!("> {bullet}one\n> {pad}two three\n> {bullet}four");
            assert_eq!(wrapped(&input, 12, TEXT, &[(1..2, true)]),
                format!("> {bullet}one two\n> {pad}three\n> {bullet}four"));
        }
        assert_eq!(all("- first\nsecond third", 15, TEXT), "- first second\n  third");
        assert_eq!(all("aa abcdefghij z", 5, TEXT), "aa\nabcdefghij\nz");
        assert_eq!(all("\ta b c\n  d\n  e f", 7, TEXT), "\ta b\n\tc\n  d e f");
    }

    #[test]
    fn markdown_boundaries_survive_selections_and_carets_inside_them() {
        let input = "# Heading with spaces\n\nSetext heading\n======\n\n> ~~~~rs\n> x y z\n> ~~~\n> x x x\n> ~~~~\n\na | b\n--- | ---\nx | y\n";
        assert_eq!(all(input, 3, TEXT), input);
        assert_eq!(wrapped(input, 3, TEXT, &[(8..9, true)]), input);
        let docs = "/// ```rs\n/// a very long expression\n/// ```";
        assert_eq!(all(docs, 8, RUST), docs);
    }

    #[test]
    fn command_uses_language_width_and_one_undo_for_multiple_targets() {
        let ctx = egui::Context::default();
        let command = Command::RewrapComment;
        assert_eq!(command.id(), "rewrap.rewrapComment");
        assert_eq!(command.label(), "Rewrap Comment / Text");
        assert!(crate::command::all().contains(&command));
        assert!(crate::command::bindings().iter().any(|b| b.command == command
            && b.shortcut == egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::Q)
            && b.scope == crate::command::Scope::Editor));
        let mut editors = Editors::new();
        let mut buf = Document::from_text("// one\n// two\n\n// three four five\n// leave this alone", "test.rs".into());
        buf.path = Some("test.rs".into());
        let original = buf.rope.to_string();
        let sels = vec![Sel::caret(3), Sel::caret(11), Sel::new(buf.line_start(3), buf.line_start(4))];
        let id = editors.insert_doc(buf);
        editors.add_tab(id, sels.clone());
        for (rulers, word_wrap_column) in [(vec![10, 80], 80), (vec![], 10)] {
            let mut settings = Settings::default();
            settings.langs.insert("rust".into(), Settings { rulers, word_wrap_column, ..Settings::default() });
            assert!(editors.exec(&Command::RewrapComment, &settings, &ctx));
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while editors.docs[&id].buf.rope.to_string() == original {
                editors.poll_background(&ctx);
                assert!(std::time::Instant::now() < until);
                std::thread::yield_now();
            }
            assert_eq!(editors.docs[&id].buf.rope.to_string(), "// one two\n\n// three\n// four\n// five\n// leave this alone");
            assert_eq!(editors.groups[0].tabs[0].sels.len(), 3);
            assert!(editors.exec(&Command::Undo, &settings, &ctx));
            assert_eq!(editors.docs[&id].buf.rope.to_string(), original);
            assert!(crate::editor::buffer::same(&sels, &editors.groups[0].tabs[0].sels));
        }
    }
}
