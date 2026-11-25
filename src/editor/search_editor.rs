use super::Editors;
use super::buffer::{After, Document, Kind, Pattern, Sel};
use crate::command::Command;
use crate::find::{self, MAX_HITS, Query};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, Color32, Event, FontId, Key, Rect, Ui, pos2, vec2};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{TryRecvError, channel};

const MAX_LINE: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Header {
    pub query: Query,
    pub context: usize,
}

impl Header {
    pub(crate) fn print(&self) -> String {
        let q = &self.query;
        let mut out = format!("# Query: {}\n", q.text.replace('\\', "\\\\").replace('\n', "\\n"));
        let flags: Vec<&str> = [(q.case, "CaseSensitive"), (q.word, "WordMatch"), (q.regex, "RegExp")].iter().filter(|f| f.0).map(|f| f.1).collect();
        if !flags.is_empty() {
            out += &format!("# Flags: {}\n", flags.join(" "));
        }
        for (key, value) in [("Including", &q.include), ("Excluding", &q.exclude)].into_iter().filter(|f| !f.1.is_empty()) {
            out += &format!("# {key}: {value}\n");
        }
        if self.context > 0 {
            out += &format!("# ContextLines: {}\n", self.context);
        }
        out
    }

    pub(crate) fn parse<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> Self {
        let mut header = Self::default();
        for line in lines {
            let Some((key, value)) = line.as_ref().strip_prefix("# ").and_then(|l| l.split_once(':')) else { continue };
            let value = value.strip_prefix(' ').unwrap_or(value);
            let q = &mut header.query;
            match key {
                "Query" => q.text = unescape(value),
                "Including" => q.include = value.to_owned(),
                "Excluding" => q.exclude = value.to_owned(),
                "ContextLines" => header.context = value.trim().parse().unwrap_or(0),
                "Flags" => (q.case, q.word, q.regex) = (value.contains("CaseSensitive"), value.contains("WordMatch"), value.contains("RegExp")),
                _ => {}
            }
        }
        header
    }

    fn pattern(&self) -> Option<Pattern> {
        let q = &self.query;
        Pattern::new(&q.text, q.case, q.word, q.regex).ok().filter(|_| !q.text.is_empty())
    }
}

fn unescape(text: &str) -> String {
    let (mut out, mut chars) = (String::new(), text.chars());
    while let Some(c) = chars.next() {
        match (c, (c == '\\').then(|| chars.next()).flatten()) {
            (_, Some('n')) => out.push('\n'),
            (_, Some('\\')) => out.push('\\'),
            (_, Some(other)) => out.extend(['\\', other]),
            (c, None) => out.push(c),
        }
    }
    out
}

pub(super) fn is(buf: &Document) -> bool {
    !buf.read_only
        && match &buf.path {
            Some(path) => path.extension().is_some_and(|e| e.eq_ignore_ascii_case("code-search")),
            None => buf.title.starts_with("Search: "),
        }
}

fn header_lines(buf: &Document) -> usize {
    (0..buf.lines()).take_while(|line| buf.rope.line(*line).chars().next() == Some('#')).count()
}

fn header_of(buf: &Document) -> (Header, usize) {
    let top = header_lines(buf);
    (Header::parse((0..top).map(|line| buf.line_text(line))), top)
}

fn numbered(text: &str) -> Option<(usize, bool, usize)> {
    let rest = text.strip_prefix("  ")?;
    let digits = rest.trim_start_matches(' ');
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    let number = digits[..count].parse().ok()?;
    let matched = digits[count..].starts_with(':');
    (matched || digits[count..].starts_with(' ')).then(|| (number, matched, (text.len() - digits.len() + count + 2).min(text.len())))
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

pub(crate) fn file_block(path: &str, text: &str, lines: &[usize], context: usize) -> String {
    let source: Vec<&str> = text.lines().collect();
    let mut shown = BTreeMap::new();
    for &line in lines {
        for at in line.saturating_sub(context).max(1)..=(line + context).min(source.len()) {
            *shown.entry(at).or_insert(false) |= at == line;
        }
    }
    let width = shown.keys().next_back().map_or(1, |last| last.to_string().len());
    let (mut out, mut last) = (format!("\n{path}:\n"), None);
    for (at, matched) in shown {
        if last.is_some_and(|last| at != last + 1) {
            out.push('\n');
        }
        let line: String = source[at - 1].chars().take(MAX_LINE).collect();
        out += &format!("  {at:>width$}{} {line}\n", if matched { ':' } else { ' ' });
        last = Some(at);
    }
    out
}

pub(crate) fn results(roots: &[(PathBuf, String)], header: &Header) -> String {
    let q = &header.query;
    if q.text.is_empty() {
        return "\n".to_owned();
    }
    if let Err(error) = Pattern::new(&q.text, q.case, q.word, q.regex) {
        return format!("\n{error}\n");
    }
    let (mut body, mut hits, mut files) = (String::new(), 0, 0);
    find::each_file(roots, q, |path, text, found| {
        (hits, files) = (hits + found.len(), files + 1);
        body += &file_block(&path, text, &found.iter().map(|hit| hit.line).collect::<Vec<_>>(), header.context);
    });
    let summary = match hits {
        0 => "No Results".to_owned(),
        _ => format!("{} - {}", plural(hits, "result"), plural(files, "file")),
    };
    let limit = if hits >= MAX_HITS { "\nThe result set only contains a subset of all matches. Be more specific in your search to narrow down the results." } else { "" };
    format!("\n{summary}{limit}\n{body}")
}

pub(super) fn target(buf: &Document, line: usize) -> Option<(String, usize, usize)> {
    let (header, top) = header_of(buf);
    let text = buf.line_text(line);
    let (number, matched, prefix) = numbered(&text).filter(|_| line >= top)?;
    let file = (top..line).rev().map(|above| buf.line_text(above)).find(|above| !above.is_empty() && !above.starts_with(' '))?;
    let found = header.pattern().filter(|_| matched).and_then(|p| p.find(&text[prefix..]).first().copied());
    Some((file.strip_suffix(':')?.to_owned(), number, found.map_or(0, |m| text[prefix..prefix + m.0].chars().count()) + 1))
}

pub(super) fn enter_reruns(buf: &Document, sels: &[Sel], event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.is_none())
        && is(buf)
        && sels.iter().all(|s| buf.line_of(s.head) < header_lines(buf))
}

pub(super) fn file_name(buf: &Document) -> Option<String> {
    let query = buf.title.strip_prefix("Search: ").filter(|_| is(buf) && buf.path.is_none())?;
    let safe: String = query.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).collect();
    Some(format!("{}.code-search", if safe.trim().is_empty() { "Untitled" } else { safe.trim() }))
}

#[derive(Default)]
pub(super) struct Sources {
    version: Option<u64>,
    headers: Vec<(usize, Option<crate::secrets::Kind>)>,
}

impl Sources {
    pub(super) fn update(&mut self, buf: &Document) {
        if self.version == Some(buf.version) { return }
        self.headers.clear();
        for (line, slice) in buf.rope.lines().enumerate() {
            if matches!(slice.chars().next(), None | Some(' ' | '\n')) { continue }
            let text = slice.to_string();
            let kind = text.trim_end().strip_suffix(':').and_then(|p| crate::secrets::kind(std::path::Path::new(p)));
            self.headers.push((line, kind));
        }
        self.version = Some(buf.version);
    }
}

pub(super) struct Marks {
    top: usize,
    pattern: Option<Pattern>,
}

impl Marks {
    pub(super) fn of(buf: &Document) -> Option<Self> {
        is(buf).then(|| {
            let (header, top) = header_of(buf);
            Self { top, pattern: header.pattern() }
        })
    }

    pub(super) fn masked(&self, buf: &Document, line: usize, sources: &Sources) -> Option<String> {
        if line < self.top { return None }
        let index = sources.headers.partition_point(|h| h.0 < line).checked_sub(1)?;
        let kind = sources.headers[index].1?;
        let text = buf.line_text(line);
        let (_, _, prefix) = numbered(&text)?;
        let source = &text[prefix..];
        if kind == crate::secrets::Kind::Url && source.chars().count() >= MAX_LINE
            && let Some((scheme, rest)) = source.split_once("://")
            && let Some((user, tail)) = rest.split_once(':')
            && !rest.contains(['/', '?', '#'])
        {
            return Some(format!("{}{scheme}://{user}:{}", &text[..prefix], "\u{2022}".repeat(tail.chars().count())));
        }
        match crate::secrets::fragment(Some(kind), source) {
            std::borrow::Cow::Owned(display) => Some(format!("{}{display}", &text[..prefix])),
            _ => None,
        }
    }

    pub(super) fn color(&self, text: &str, line: usize, theme: &Theme) -> Option<(usize, usize, Color32)> {
        if line < self.top {
            return text.find(':').map(|at| (0, at + 1, theme.text_dim));
        }
        if let Some((_, _, prefix)) = numbered(text) {
            return Some((0, prefix, theme.text_dim));
        }
        (!text.starts_with(' ') && text.ends_with(':')).then_some((0, text.len(), theme.accent))
    }

    pub(super) fn found(&self, text: &str, line: usize) -> Vec<(usize, usize)> {
        let (Some(pattern), Some((_, true, prefix))) = (&self.pattern, numbered(text).filter(|_| line >= self.top)) else { return vec![] };
        let col = |at: usize| text[..prefix + at].chars().count();
        pattern.find(&text[prefix..]).into_iter().map(|(a, b)| (col(a), col(b))).collect()
    }
}

impl Editors {
    fn search_roots(&self) -> Vec<(PathBuf, String)> {
        let roots = crate::workspace::current().prefixed();
        if roots.len() > 1 { roots } else { self.root.iter().map(|root| (root.clone(), String::new())).collect() }
    }

    pub fn open_search(&mut self, mut query: Query, selection: bool, ctx: &egui::Context) {
        let selected = self
            .with_active(|buf, tab| {
                let s = tab.sels[0];
                (!s.is_empty() && buf.line_of(s.start()) == buf.line_of(s.end())).then(|| buf.text_of(s.start(), s.end()))
            })
            .flatten();
        if let Some(text) = selected.filter(|_| selection) {
            query.text = text;
        }
        let header = Header { query, context: 0 };
        let text = header.print();
        let id = self.insert_doc(Document::from_text(&text, format!("Search: {}", header.query.text)));
        self.add_tab(id, vec![Sel::caret(text.lines().next().map_or(0, |line| line.chars().count()))]);
        if !header.query.text.is_empty() {
            self.run_search(id, &header, ctx);
        }
    }

    fn run_search(&mut self, id: u64, header: &Header, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (roots, header, ctx) = (self.search_roots(), header.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(results(&roots, &header));
            ctx.request_repaint();
        });
        self.searches.retain(|search| search.0 != id);
        self.searches.push((id, rx));
    }

    pub(super) fn poll_searches(&mut self) {
        let mut done = vec![];
        self.searches.retain(|(id, rx)| match rx.try_recv() {
            Ok(text) => {
                done.push((*id, text));
                false
            }
            Err(error) => error == TryRecvError::Empty,
        });
        for (id, text) in done {
            let Some(doc) = self.docs.get_mut(&id).filter(|doc| doc.content.is_text()) else { continue };
            let buf = &mut doc.buf;
            let from = buf.line_start(header_lines(buf));
            let lead = if from == buf.len() && from > 0 && buf.rope.char(from - 1) != '\n' { "\n" } else { "" };
            let clean = buf.path.is_none() && !buf.is_dirty();
            let mut none = vec![Sel::caret(0)];
            let sels = self.groups.iter_mut().flat_map(|group| &mut group.tabs).find(|tab| tab.doc == id).map_or(&mut none, |tab| &mut tab.sels);
            buf.apply(sels, vec![(from, buf.len(), format!("{lead}{text}"))], Kind::Other, After::Map);
            if clean {
                buf.mark_clean();
            }
        }
    }

    pub(super) fn search_exec(&mut self, cmd: &Command, settings: &Settings, ctx: &egui::Context) -> bool {
        use Command::*;
        let toggles = [ToggleSearchEditorContextLines, ToggleSearchEditorCaseSensitive, ToggleSearchEditorWholeWord, ToggleSearchEditorRegex];
        if !matches!(cmd, RerunSearchEditorSearch | GoToDefinition) && !toggles.contains(cmd) {
            return false;
        }
        let Some((g, t)) = self.active_tab() else { return false };
        let tab = &mut self.groups[g].tabs[t];
        let Some(doc) = self.docs.get_mut(&tab.doc).filter(|doc| doc.content.is_text() && tab.md.is_none() && is(&doc.buf)) else { return false };
        let (mut header, top) = header_of(&doc.buf);
        if *cmd == GoToDefinition {
            let line = doc.buf.line_of(tab.sels.first().map_or(0, |s| s.head));
            if let Some((file, line, col)) = target(&doc.buf, line) {
                let path = crate::workspace::resolve(&self.search_roots(), &file).filter(|path| path.exists()).unwrap_or_else(|| PathBuf::from(&file));
                self.open(&path);
                self.exec(&GoTo(line, col), settings, ctx);
            }
            return true;
        }
        match cmd {
            ToggleSearchEditorContextLines => header.context = if header.context > 0 { 0 } else { 1 },
            ToggleSearchEditorCaseSensitive => header.query.case ^= true,
            ToggleSearchEditorWholeWord => header.query.word ^= true,
            ToggleSearchEditorRegex => header.query.regex ^= true,
            _ => {}
        }
        if toggles.contains(cmd) {
            let end = doc.buf.line_start(top);
            doc.buf.apply(&mut tab.sels, vec![(0, end, header.print())], Kind::Other, After::Map);
        }
        if doc.buf.path.is_none() {
            doc.buf.title = format!("Search: {}", header.query.text);
        }
        let id = tab.doc;
        self.run_search(id, &header, ctx);
        true
    }

    pub(super) fn search_badge(&self, ui: &Ui, area: Rect, theme: &Theme, doc: u64) {
        if self.searches.iter().any(|search| search.0 == doc) {
            let center = pos2(area.right() - 190.0, area.top() + 14.0);
            ui::spinner(ui, center, 6.0, theme.accent);
            ui.painter().text(center + vec2(12.0, 0.0), Align2::LEFT_CENTER, "Searching...", FontId::proportional(12.0), theme.text_dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_and_parses_the_header_for_every_flag_combination() {
        for bits in 0..64u8 {
            let on = |bit: u8| bits & (1 << bit) != 0;
            let query = Query {
                text: "a\\b\nc".into(),
                case: on(0),
                word: on(1),
                regex: on(2),
                include: if on(3) { "src, *.rs".into() } else { String::new() },
                exclude: if on(4) { "target".into() } else { String::new() },
            };
            let header = Header { query, context: if on(5) { 2 } else { 0 } };
            let text = header.print();
            assert_eq!(Header::parse(text.lines()), header, "{text}");
            assert_eq!(text.contains("# Flags:"), bits & 7 != 0);
            assert_eq!(text.lines().count(), 1 + usize::from(bits & 7 != 0) + usize::from(on(3)) + usize::from(on(4)) + usize::from(on(5)));
        }
        let header = Header::parse(["# Query: foo", "# Flags: RegExp CaseSensitive", "", "# Including: x"]);
        assert_eq!((header.query.text.as_str(), header.query.regex, header.query.case, header.query.word), ("foo", true, true, false));
        assert_eq!((header.query.include.as_str(), header.context), ("x", 0));
        assert_eq!(Header::parse(["# Query:", "# ContextLines: 3", "# Unknown: 1"]), Header { context: 3, ..Default::default() });
        assert_eq!(Header::parse(["# Query: \\d+\\\\n"]).query.text, "\\d+\\n");
        let flags = Header { query: Query { text: "x".into(), case: true, word: true, regex: true, ..Default::default() }, context: 1 };
        assert_eq!(flags.print(), "# Query: x\n# Flags: CaseSensitive WordMatch RegExp\n# ContextLines: 1\n");
    }

    #[test]
    fn writes_results_with_context_lines_and_maps_lines_back() {
        let text = (1..=12).map(|n| format!("line {n}{}", if n == 3 || n == 10 || n == 11 { " foo" } else { "" })).collect::<Vec<_>>().join("\n");
        assert_eq!(file_block("a.rs", &text, &[3], 0), "\na.rs:\n  3: line 3 foo\n");
        assert_eq!(
            file_block("src/a.rs", &text, &[3, 10, 11, 11], 1),
            "\nsrc/a.rs:\n   2  line 2\n   3: line 3 foo\n   4  line 4\n\n   9  line 9\n  10: line 10 foo\n  11: line 11 foo\n  12  line 12\n"
        );
        assert_eq!(file_block("b", "x foo", &[1], 2), "\nb:\n  1: x foo\n");

        let header = Header { query: Query { text: "foo".into(), ..Default::default() }, context: 1 };
        let doc = format!("{}\n12 results - 1 file\n{}", header.print(), file_block("src/a.rs", &text, &[3, 10, 11], 1));
        let buf = Document::from_text(&doc, "Search: foo".into());
        let at = |needle: &str| doc.lines().position(|line| line.contains(needle)).unwrap();
        assert!(is(&buf) && !is(&Document::from_text("", "Untitled-1".into())));
        assert_eq!(target(&buf, at("10: line 10")), Some(("src/a.rs".into(), 10, 9)));
        assert_eq!(target(&buf, at("4  line 4")), Some(("src/a.rs".into(), 4, 1)));
        for none in ["# Query", "12 results", "src/a.rs:"] {
            assert_eq!(target(&buf, at(none)), None, "{none}");
        }
        assert_eq!(target(&buf, at("4  line 4") + 1), None);
        let marks = Marks::of(&buf).unwrap();
        assert_eq!(marks.found("  10: line 10 foo", 9), vec![(14, 17)]);
        assert!(marks.found("   9  line 9 foo", 9).is_empty() && marks.found("  10: foo", 0).is_empty());
        assert_eq!(numbered("  10:"), Some((10, true, 5)));
        assert_eq!(numbered("   9  x"), Some((9, false, 6)));
        assert!(numbered("  x: 1").is_none() && numbered("src/a.rs:").is_none() && numbered("  10x").is_none());
        assert_eq!(file_name(&buf).as_deref(), Some("foo.code-search"));
        let mut saved = Document::from_text("", "a.code-search".into());
        saved.path = Some(PathBuf::from("a.code-search"));
        assert!(is(&saved) && file_name(&saved).is_none());
    }

    #[test]
    fn enter_on_the_query_line_reruns_the_search() {
        let dir = std::env::temp_dir().join(format!("halo-search-editor-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "alpha\nbeta gamma\n").unwrap();
        std::fs::write(dir.join("b.txt"), "gamma\n").unwrap();
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let mut editors = Editors::new();
        editors.set_root(Some(dir.clone()));
        let frame = |editors: &mut Editors, events: Vec<Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))), events, focused: true, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &Theme::dark(), &settings, &crate::git::Git::new(), &mut vec![]));
            out.textures_delta.clear();
        };
        let text = |editors: &Editors| editors.docs.values().next().unwrap().buf.rope.to_string();
        let settle = |editors: &mut Editors| {
            for _ in 0..500 {
                frame(editors, vec![]);
                if editors.searches.is_empty() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the search did not finish");
        };
        editors.open_search(Query { text: "alpha".into(), ..Default::default() }, true, &ctx);
        settle(&mut editors);
        assert_eq!(text(&editors), "# Query: alpha\n\n1 result - 1 file\n\nsrc/a.txt:\n  1: alpha\n");
        assert!(!editors.docs.values().next().unwrap().buf.is_dirty());
        let key = |key: Key, modifiers: egui::Modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        frame(&mut editors, vec![key(Key::End, egui::Modifiers::NONE)]);
        frame(&mut editors, vec![key(Key::Backspace, egui::Modifiers::COMMAND), Event::Text("gamma".into())]);
        frame(&mut editors, vec![key(Key::Enter, egui::Modifiers::NONE)]);
        settle(&mut editors);
        let after = text(&editors);
        assert!(after.starts_with("# Query: gamma\n\n2 results - 2 files\n") && after.contains("src/a.txt:\n  2: beta gamma\n") && after.contains("b.txt:\n  1: gamma\n"), "{after}");
        assert_eq!(editors.docs.values().next().unwrap().buf.title, "Search: gamma");

        editors.exec(&Command::ToggleSearchEditorContextLines, &settings, &ctx);
        settle(&mut editors);
        assert!(text(&editors).contains("# ContextLines: 1\n") && text(&editors).contains("src/a.txt:\n  1  alpha\n  2: beta gamma\n"), "{}", text(&editors));
        editors.exec(&Command::Undo, &settings, &ctx);
        let undone = text(&editors);
        assert!(undone.contains("# ContextLines: 1\n") && !undone.contains("  1  alpha") && undone.contains("src/a.txt:\n  2: beta gamma\n"), "{undone}");

        let line = text(&editors).lines().position(|line| line == "  1: gamma").unwrap();
        editors.with_active(|buf, tab| tab.sels = vec![Sel::caret(buf.line_start(line))]);
        editors.exec(&Command::GoToDefinition, &settings, &ctx);
        editors.finish_loads();
        assert_eq!(editors.active_path(), Some(dir.join("b.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
