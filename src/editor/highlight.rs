use crate::theme::Theme;
use eframe::egui::Color32;
use ropey::Rope;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use syntect::highlighting::{
    Color, HighlightState, Highlighter as SynHighlighter, RangedHighlightIterator, ScopeSelectors, StyleModifier,
    Theme as SynTheme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

const STEP: usize = 64;
const MAX_LINE: usize = 2000;
const MAX_FILE: usize = 5 * 1024 * 1024;

pub type Spans = Arc<Vec<(usize, Color32)>>;

pub struct Highlighter {
    syntax: Option<&'static SyntaxReference>,
    pub language: String,
    checkpoints: Vec<(ParseState, HighlightState)>,
    cache: HashMap<usize, Spans>,
    key: Option<Key>,
}

type Key = ([Color32; 11], usize);

fn key(theme: &Theme) -> Key {
    let s = &theme.syntax;
    (
        [theme.text, s.comment, s.string, s.number, s.constant, s.keyword, s.function, s.type_name, s.punctuation, theme.added, theme.deleted],
        theme.tokens.id,
    )
}

pub fn languages() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list: Vec<(String, String)> =
            syntaxes().syntaxes().iter().filter(|s| !s.hidden).map(|s| (s.name.clone(), s.file_extensions.join(", "))).collect();
        list.sort();
        list
    })
}

static CONFIG: OnceLock<(String, BTreeMap<String, String>)> = OnceLock::new();

pub fn configure(grammars: &str, associations: &BTreeMap<String, String>) {
    let _ = CONFIG.set((grammars.to_owned(), associations.clone()));
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        let set = SyntaxSet::load_defaults_newlines();
        let Some(folder) = CONFIG.get().map(|c| Path::new(&c.0)).filter(|f| f.is_dir()) else { return set };
        let mut builder = set.into_builder();
        if let Err(error) = builder.add_from_folder(folder, true) {
            crate::ui::notify(format!("Could not load grammars from {}: {error}", folder.display()));
        }
        builder.build()
    })
}

fn color(c: Color32) -> Color {
    Color { r: c.r(), g: c.g(), b: c.b(), a: 255 }
}

fn build_theme(theme: &Theme) -> SynTheme {
    let item = |selector: &str, c: Color32| ThemeItem {
        scope: ScopeSelectors::from_str(selector).unwrap_or_default(),
        style: StyleModifier { foreground: Some(color(c)), background: None, font_style: None },
    };
    let s = &theme.syntax;
    let mut built = SynTheme {
        name: None,
        author: None,
        settings: ThemeSettings { foreground: Some(color(theme.text)), ..Default::default() },
        scopes: vec![
            item("comment, punctuation.definition.comment", s.comment),
            item("string, punctuation.definition.string", s.string),
            item("constant.numeric", s.number),
            item("constant.language, constant.character, constant.other, support.constant", s.constant),
            item("keyword, storage, entity.name.tag", s.keyword),
            item("entity.name.function, support.function, variable.function, entity.other.attribute-name", s.function),
            item(
                "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, support.type, support.class, storage.type, entity.other.inherited-class",
                s.type_name,
            ),
            item("punctuation, keyword.operator", s.punctuation),
            item("markup.heading, markup.bold", s.keyword),
            item("markup.raw, markup.inline.raw", s.string),
            item("meta.diff", s.keyword),
            item("markup.inserted", theme.added),
            item("markup.deleted", theme.deleted),
        ],
    };
    built.scopes.extend(theme.tokens.rules.iter().cloned());
    built
}

fn syn_theme(theme: &Theme) -> Arc<SynTheme> {
    static BUILT: Mutex<Option<(Key, Arc<SynTheme>)>> = Mutex::new(None);
    let mut built = BUILT.lock().unwrap();
    match &*built {
        Some((made_for, ready)) if *made_for == key(theme) => ready.clone(),
        _ => {
            let ready = Arc::new(build_theme(theme));
            *built = Some((key(theme), ready.clone()));
            ready
        }
    }
}

fn find_syntax(path: Option<&Path>, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    let alias = match ext.as_str() {
        "ts" | "tsx" | "jsx" | "mjs" | "cjs" => "js",
        "ino" | "h" | "hpp" | "hh" | "cc" | "cxx" => "cpp",
        "jsonc" | "json5" => "json",
        "ps1" | "zsh" | "bashrc" | "env" => "sh",
        "htm" | "vue" | "svelte" => "html",
        "svg" | "csproj" | "xaml" => "xml",
        other => other,
    };
    let user = CONFIG.get().and_then(|c| c.1.get(&ext));
    user.and_then(|target| set.find_syntax_by_extension(target).or_else(|| set.find_syntax_by_name(target)))
        .or_else(|| set.find_syntax_by_extension(&name))
        .or_else(|| set.find_syntax_by_extension(&ext))
        .or_else(|| set.find_syntax_by_extension(alias))
        .or_else(|| set.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

pub fn symbols(path: Option<&Path>, rope: &Rope) -> Vec<(usize, String)> {
    let first_line: String = rope.line(0).chars().take(200).collect();
    let Some(syntax) = find_syntax(path, &first_line).filter(|_| rope.len_bytes() <= MAX_FILE) else { return vec![] };
    let names = ScopeSelectors::from_str("entity.name - entity.name.tag").unwrap_or_default();
    let (mut parse, mut stack) = (ParseState::new(syntax), ScopeStack::new());
    let mut out = vec![];
    for (number, slice) in rope.lines().enumerate().filter(|(_, slice)| slice.len_bytes() <= MAX_LINE) {
        let text: std::borrow::Cow<str> = slice.into();
        let (mut at, mut name) = (0, String::new());
        for (pos, op) in parse.parse_line(&text, syntaxes()).unwrap_or_default() {
            if names.does_match(stack.as_slice()).is_some() {
                name.push_str(&text[at..pos]);
                name.push(' ');
            }
            let _ = stack.apply(&op);
            at = pos;
        }
        if !name.trim().is_empty() {
            out.push((number, name.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
    }
    out
}

impl Highlighter {
    pub fn new(path: Option<&Path>, rope: &Rope) -> Self {
        let first_line: String = rope.line(0).chars().take(200).collect();
        let syntax = if rope.len_bytes() > MAX_FILE { None } else { find_syntax(path, &first_line) };
        Self {
            syntax,
            language: syntax.map_or("Plain Text", |s| s.name.as_str()).to_owned(),
            checkpoints: vec![],
            cache: HashMap::new(),
            key: None,
        }
    }

    pub fn named(name: &str, rope: &Rope) -> Self {
        let syntax = syntaxes().find_syntax_by_name(name).filter(|s| s.name != "Plain Text" && rope.len_bytes() <= MAX_FILE);
        Self { syntax, ..Self::new(None, &Rope::new()) }.labelled()
    }

    fn labelled(mut self) -> Self {
        self.language = self.syntax.map_or("Plain Text", |s| s.name.as_str()).to_owned();
        self
    }

    pub fn invalidate(&mut self, line: usize) {
        self.checkpoints.truncate(line / STEP + 1);
        self.cache.retain(|l, _| *l < line);
    }

    pub fn get(&self, line: usize) -> Option<&Spans> {
        self.cache.get(&line)
    }

    pub fn ensure(&mut self, rope: &Rope, lines: Range<usize>, theme: &Theme, budget: Duration) -> bool {
        let Some(syntax) = self.syntax else { return true };
        if self.key != Some(key(theme)) {
            self.checkpoints.clear();
            self.cache.clear();
            self.key = Some(key(theme));
        }
        let end = lines.end.min(rope.len_lines());
        let Some(first) = (lines.start..end).find(|l| !self.cache.contains_key(l)) else { return true };
        let set = syntaxes();
        let colors = syn_theme(theme);
        let highlighter = SynHighlighter::new(&colors);
        if self.checkpoints.is_empty() {
            self.checkpoints.push((ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new())));
        }
        let index = (first / STEP).min(self.checkpoints.len() - 1);
        let (mut parse, mut state) = self.checkpoints[index].clone();
        let started = Instant::now();
        let mut line = index * STEP;
        while line < end {
            if line.is_multiple_of(STEP) && line / STEP >= self.checkpoints.len() {
                self.checkpoints.push((parse.clone(), state.clone()));
                if started.elapsed() > budget {
                    return false;
                }
            }
            let slice = rope.line(line);
            let mut spans: Vec<(usize, Color32)> = vec![];
            if slice.len_bytes() <= MAX_LINE {
                let text: std::borrow::Cow<str> = slice.into();
                let ops = parse.parse_line(&text, set).unwrap_or_default();
                for (style, _, range) in RangedHighlightIterator::new(&mut state, &ops, &text, &highlighter) {
                    let c = style.foreground;
                    let c = Color32::from_rgb(c.r, c.g, c.b);
                    match spans.last_mut() {
                        Some(last) if last.1 == c => last.0 += range.len(),
                        _ => spans.push((range.len(), c)),
                    }
                }
            }
            if line >= lines.start {
                self.cache.insert(line, Arc::new(spans));
            }
            line += 1;
        }
        if self.cache.len() > 3000 {
            let (low, high) = (lines.start.saturating_sub(300), end + 300);
            self.cache.retain(|l, _| *l >= low && *l < high);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_invalidates_and_respects_budget() {
        let theme = Theme::dark();
        let mut text = String::from("// note\nfn main() { let x = \"s\"; }\n");
        for i in 0..50_000 {
            text.push_str(&format!("let v{i} = {i};\n"));
        }
        let mut rope = Rope::from_str(&text);
        let mut h = Highlighter::new(Some(Path::new("a.rs")), &rope);
        assert_eq!(h.language, "Rust");
        assert!(h.ensure(&rope, 0..40, &theme, Duration::from_millis(500)));
        let comment = h.get(0).unwrap().clone();
        assert_eq!(comment[0].1, theme.syntax.comment);
        assert!(h.get(1).unwrap().iter().any(|s| s.1 == theme.syntax.string));

        rope.insert(0, "/* ");
        h.invalidate(0);
        assert!(h.get(1).is_none());
        assert!(h.ensure(&rope, 0..40, &theme, Duration::from_millis(500)));
        assert!(h.get(1).unwrap().iter().all(|s| s.1 == theme.syntax.comment));

        let started = Instant::now();
        let done = h.ensure(&rope, 49_000..49_040, &theme, Duration::from_millis(5));
        assert!(!done && started.elapsed() < Duration::from_millis(400));
        let mut rounds = 0;
        while !h.ensure(&rope, 49_000..49_040, &theme, Duration::from_millis(50)) {
            rounds += 1;
            assert!(rounds < 2000);
        }
        assert!(h.get(49_010).is_some());

        let found = symbols(Some(Path::new("a.rs")), &Rope::from_str("struct Foo;

fn bar() {
    baz();
}
"));
        assert_eq!(found, vec![(0, "Foo".to_owned()), (2, "bar".to_owned())]);

        let plain = Highlighter::new(Some(Path::new("notes.unknownext")), &Rope::from_str("hello"));
        assert_eq!(plain.language, "Plain Text");
    }
}
