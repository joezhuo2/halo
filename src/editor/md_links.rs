use super::buffer::Document;
use super::markdown::{is_markdown, slug, unescape};
use super::{Doc, Editors};
use crate::lsp::{self, Completion, Diagnostic};
use eframe::egui;
use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use serde_json::json;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

type Found = (u64, u64, PathBuf, Vec<Diagnostic>);

pub(super) struct Links {
    done: HashMap<u64, u64>,
    epoch: u64,
    tx: Sender<Found>,
    rx: Receiver<Found>,
}

impl Default for Links {
    fn default() -> Self {
        let (tx, rx) = channel();
        Links { done: HashMap::new(), epoch: 0, tx, rx }
    }
}

impl Links {
    pub fn stale(&mut self) {
        self.done.clear();
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Target {
    Path(String),
    Anchor(String),
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

fn external(target: &str) -> bool {
    target.starts_with("//") || target.find(':').is_some_and(|at| at > 1 && target[..at].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)))
}

fn in_code(text: &str, at: usize) -> bool {
    Parser::new_ext(text, options()).into_offset_iter().any(|(event, range)| match event {
        Event::Code(_) => range.start < at && at < range.end,
        Event::Start(Tag::CodeBlock(_)) => range.start < at && (at < range.end || range.end == text.len()),
        _ => false,
    })
}

pub(super) fn context(text: &str, at: usize) -> Option<(usize, Target)> {
    let begin = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[begin..at];
    let inline = line.rfind("](").map(|i| i + 2 + usize::from(line[i + 2..].starts_with('<')));
    let html = ["src=\"", "href=\"", "src='", "href='"].iter().filter_map(|key| line.rfind(key).map(|i| i + key.len())).max();
    let indent = line.len() - line.trim_start().len();
    let definition = (indent < 4 && line[indent..].starts_with('['))
        .then(|| line.find("]:"))
        .flatten()
        .map(|i| i + 2 + line[i + 2..].len() - line[i + 2..].trim_start().len());
    let from = [inline, html, definition].into_iter().flatten().max()?;
    let typed = &line[from..];
    if typed.contains(|c: char| c.is_whitespace() || "()<>\"'`".contains(c)) || external(typed) || in_code(text, at) {
        return None;
    }
    Some(match typed.find('#') {
        Some(hash) => (begin + from + hash + 1, Target::Anchor(typed[..hash].to_owned())),
        None => {
            let cut = typed.rfind('/').map_or(0, |i| i + 1);
            (begin + from + cut, Target::Path(typed[..cut].to_owned()))
        }
    })
}

fn resolve(path: &Path, root: Option<&Path>, rel: &str) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    let rel = unescape(rel);
    match rel.strip_prefix('/') {
        Some(rest) => root.unwrap_or(dir).join(rest),
        None => dir.join(rel),
    }
}

fn attrs<'a>(html: &'a str, names: &[&str]) -> Vec<(usize, &'a str)> {
    let mut out = vec![];
    for name in names {
        for quote in ['"', '\''] {
            let key = format!("{name}={quote}");
            for (at, _) in html.match_indices(&key).filter(|(at, _)| html[..*at].ends_with(char::is_whitespace)) {
                let from = at + key.len();
                out.extend(html[from..].find(quote).map(|len| (from, &html[from..from + len])));
            }
        }
    }
    out
}

pub(super) fn headings(text: &str) -> Vec<(String, String)> {
    let (mut out, mut seen, mut title) = (vec![], HashMap::new(), None::<String>);
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => title = Some(String::new()),
            Event::Text(part) | Event::Code(part) => title.iter_mut().for_each(|title| title.push_str(&part)),
            Event::End(TagEnd::Heading(_)) => {
                let Some(title) = title.take() else { continue };
                let base = slug(&title);
                let count = seen.entry(base.clone()).or_insert(0);
                out.push((if *count == 0 { base } else { format!("{base}-{count}") }, title));
                *count += 1;
            }
            Event::Html(_) | Event::InlineHtml(_) => out.extend(attrs(&text[range], &["id", "name"]).into_iter().map(|(_, id)| (id.to_owned(), String::new()))),
            _ => {}
        }
    }
    out
}

fn anchors(text: &str) -> Vec<String> {
    headings(text).into_iter().map(|h| h.0).collect()
}

pub(super) fn items(text: &str, path: &Path, root: Option<&Path>, target: &Target) -> Vec<Completion> {
    let item = |label: String, kind: u8, detail: String, sort: String, text: String| Completion {
        filter: label.clone(),
        label,
        detail,
        kind,
        sort,
        text,
        range: None,
        extra: vec![],
        snippet: false,
    };
    let found = match target {
        Target::Anchor(file) if file.is_empty() => headings(text),
        Target::Anchor(file) => {
            let target = resolve(path, root, file);
            let read = is_markdown(&target).then(|| std::fs::read_to_string(&target).ok()).flatten();
            read.map(|text| headings(&text)).unwrap_or_default()
        }
        Target::Path(dir) => {
            let Ok(entries) = std::fs::read_dir(resolve(path, root, dir)) else { return vec![] };
            return entries
                .flatten()
                .take(5000)
                .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path().is_dir()))
                .filter(|(name, _)| name != ".git")
                .map(|(name, folder)| {
                    let text = name.replace(' ', "%20") + if folder { "/" } else { "" };
                    item(name.clone(), if folder { 19 } else { 17 }, String::new(), format!("{}{name}", u8::from(!folder)), text)
                })
                .collect();
        }
    };
    found.into_iter().enumerate().map(|(index, (anchor, title))| item(anchor.clone(), 18, title, format!("{index:05}"), anchor)).collect()
}

pub(super) fn target(buf: &Document, head: usize) -> Option<(usize, Target)> {
    buf.path.as_deref().filter(|p| is_markdown(p) && buf.rope.len_bytes() < 2_000_000)?;
    let (start, target) = context(&buf.rope.to_string(), buf.rope.char_to_byte(head.min(buf.rope.len_chars())))?;
    Some((buf.rope.byte_to_char(start), target))
}

pub(super) fn complete(buf: &Document, root: Option<&Path>, target: &Target) -> Vec<Completion> {
    let Some(path) = buf.path.as_deref() else { return vec![] };
    items(&buf.rope.to_string(), path, root, target)
}

fn problem(dest: &str, path: &Path, root: Option<&Path>, own: &[String], cache: &mut HashMap<PathBuf, Option<Vec<String>>>) -> Option<String> {
    let dest = dest.trim();
    if dest.is_empty() || external(dest) {
        return None;
    }
    let (file, fragment) = dest.split_once('#').unwrap_or((dest, ""));
    let (file, fragment) = (file.split('?').next().unwrap_or_default(), unescape(fragment));
    let found = if file.is_empty() {
        own
    } else {
        let mut target = resolve(path, root, file);
        if !target.exists() && target.extension().is_none() && target.with_extension("md").exists() {
            target.set_extension("md");
        }
        if !target.exists() {
            return Some(format!("File does not exist at path: {}", target.display()));
        }
        if fragment.is_empty() || !is_markdown(&target) {
            return None;
        }
        cache.entry(target).or_insert_with_key(|target| std::fs::read_to_string(target).ok().map(|text| anchors(&text))).as_deref()?
    };
    let line = fragment.strip_prefix('L').is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    let known = fragment.is_empty() || line || found.iter().any(|a| *a == fragment || *a == slug(&fragment));
    (!known).then(|| format!("No header found: '{fragment}'"))
}

fn span_of(text: &str, range: Range<usize>, after: &str, dest: &str) -> Range<usize> {
    let whole = &text[range.clone()];
    let from = whole.rfind(after).map_or(0, |at| at + after.len());
    whole[from..].find(dest).filter(|_| !dest.is_empty()).map_or(range.clone(), |at| range.start + from + at..range.start + from + at + dest.len())
}

pub(super) fn validate(text: &str, path: &Path, root: Option<&Path>) -> Vec<Diagnostic> {
    let mut missing: Vec<(Range<usize>, String)> = vec![];
    let mut targets: Vec<(Range<usize>, String)> = vec![];
    let callback = |link: BrokenLink<'_>| -> Option<(CowStr<'_>, CowStr<'_>)> {
        if !matches!(link.link_type, LinkType::Shortcut) && !missing.iter().any(|m| m.0 == link.span) {
            missing.push((link.span, link.reference.to_string()));
        }
        None
    };
    let mut it = Parser::new_with_broken_link_callback(text, options(), Some(callback)).into_offset_iter();
    for (event, range) in it.by_ref() {
        match event {
            Event::Start(Tag::Link { link_type: LinkType::Inline, dest_url, .. } | Tag::Image { link_type: LinkType::Inline, dest_url, .. }) => {
                targets.push((span_of(text, range, "](", &dest_url), dest_url.to_string()));
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                let found = attrs(&text[range.clone()], &["src", "href"]);
                targets.extend(found.into_iter().map(|(at, value)| (range.start + at..range.start + at + value.len(), value.to_owned())));
            }
            _ => {}
        }
    }
    for (_, def) in it.reference_definitions().iter() {
        targets.push((span_of(text, def.span.clone(), "]:", &def.dest), def.dest.to_string()));
    }
    drop(it);
    let own = anchors(text);
    let mut cache = HashMap::new();
    let mut found: Vec<(Range<usize>, String)> =
        targets.into_iter().filter_map(|(span, dest)| Some((span, problem(&dest, path, root, &own, &mut cache)?))).collect();
    found.extend(missing.into_iter().map(|(span, id)| (span, format!("No link definition found: '{id}'"))));
    found.sort_by_key(|(span, _)| span.start);
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
    let pos = |at: usize| {
        let line = starts.partition_point(|s| *s <= at) - 1;
        (line, text[starts[line]..at].encode_utf16().count())
    };
    found
        .into_iter()
        .map(|(span, message)| {
            let (start, end) = (pos(span.start), pos(span.end));
            let raw = json!({"range": lsp::range_json(start, end), "severity": 2, "message": message, "source": "markdown"});
            Diagnostic { start, end, severity: 2, message, source: "markdown".into(), raw }
        })
        .collect()
}

impl Editors {
    pub(super) fn validate_links(&mut self, enabled: bool, now: f64, ctx: &egui::Context) {
        let links = &mut self.links;
        if links.epoch != self.lsp.epoch {
            links.epoch = self.lsp.epoch;
            links.done.clear();
        }
        for (id, version, path, found) in links.rx.try_iter() {
            if enabled && self.docs.get(&id).is_some_and(|d| d.buf.version == version) {
                self.lsp.publish(path, "markdown", found);
            }
        }
        if !enabled {
            for id in std::mem::take(&mut links.done).into_keys() {
                if let Some(path) = self.docs.get(&id).and_then(|d| d.buf.path.clone()) {
                    self.lsp.publish(path, "markdown", vec![]);
                }
            }
            return;
        }
        let wanted = |d: &Doc| {
            !d.lang.attached && d.content.is_text() && !d.buf.large && d.buf.rope.len_bytes() < 2_000_000 && d.buf.path.as_deref().is_some_and(is_markdown)
        };
        let jobs: Vec<(u64, u64, PathBuf, String)> = self
            .docs
            .iter()
            .filter(|(id, d)| wanted(d) && links.done.get(*id) != Some(&d.buf.version))
            .map(|(id, d)| (*id, d.buf.version, d.buf.path.clone().unwrap_or_default(), d.buf.rope.to_string()))
            .collect();
        if jobs.is_empty() {
            return;
        }
        if now - self.last_input <= 0.3 {
            ctx.request_repaint_after(Duration::from_millis(350));
            return;
        }
        for (id, version, path, text) in jobs {
            links.done.insert(id, version);
            let (tx, ctx, root) = (links.tx.clone(), ctx.clone(), self.root.clone());
            std::thread::spawn(move || {
                let found = validate(&text, &path, root.as_deref());
                if tx.send((id, version, path, found)).is_ok() {
                    ctx.request_repaint();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Option<(usize, Target)> {
        let caret = text.find('|').unwrap();
        context(&text.replace('|', ""), caret)
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-md-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/sub folder")).unwrap();
        std::fs::write(dir.join("docs/guide.md"), "# Setup\n\n## Install it!\n\n## Install it!\n<a name=\"top\"></a>\n").unwrap();
        std::fs::write(dir.join("docs/logo.png"), "").unwrap();
        dir
    }

    #[test]
    fn finds_the_link_target_at_the_caret() {
        assert_eq!(at("see [a](|"), Some((8, Target::Path(String::new()))));
        assert_eq!(at("![img](./docs/gu|"), Some((14, Target::Path("./docs/".into()))));
        assert_eq!(at("[a](../|"), Some((7, Target::Path("../".into()))));
        assert_eq!(at("[a](/src/|"), Some((9, Target::Path("/src/".into()))));
        assert_eq!(at("[a](#se|"), Some((5, Target::Anchor(String::new()))));
        assert_eq!(at("[a](other.md#|"), Some((13, Target::Anchor("other.md".into()))));
        assert_eq!(at("  [id]: docs/|"), Some((13, Target::Path("docs/".into()))));
        assert_eq!(at("<img src=\"im|"), Some((10, Target::Path(String::new()))));
        assert_eq!(at("x <a class=\"b\" href='a/|"), Some((23, Target::Path("a/".into()))));
        assert_eq!(at("[a](b) and |"), None);
        assert_eq!(at("[a](b c|"), None);
        assert_eq!(at("[a](https://exa|"), None);
        assert_eq!(at("plain (|"), None);
        assert_eq!(at("`[a](|`"), None);
        assert_eq!(at("```\n[a](|\n```\n"), None);
        assert_eq!(at("```md\n[a](|"), None);
        assert_eq!(at("```\ncode\n```\n[a](|"), Some((17, Target::Path(String::new()))));
    }

    #[test]
    fn heading_slugs_follow_github_and_number_duplicates() {
        let text = "# Hello, World!\n\nSetext Title\n===\n\n## `code` and *em*\n\n## Hello World\n## Hello World\n\n```\n# not a heading\n```\n";
        let slugs: Vec<String> = anchors(text);
        assert_eq!(slugs, ["hello-world", "setext-title", "code-and-em", "hello-world-1", "hello-world-2"]);
    }

    #[test]
    fn lists_files_folders_and_headings() {
        let dir = folder("list");
        let readme = dir.join("README.md");
        let labels = |target: Target| {
            let mut found: Vec<(String, u8, String)> = items("# Top\n", &readme, Some(&dir), &target).into_iter().map(|c| (c.label, c.kind, c.text)).collect();
            found.sort();
            found
        };
        let docs = labels(Target::Path("./docs/".into()));
        assert_eq!(docs, [("guide.md".into(), 17, "guide.md".into()), ("logo.png".into(), 17, "logo.png".into()), ("sub folder".into(), 19, "sub%20folder/".into())]);
        assert_eq!(labels(Target::Path("/docs/".into())), docs);
        assert!(labels(Target::Path(String::new())).iter().any(|item| item.0 == "docs" && item.1 == 19 && item.2 == "docs/"));
        assert!(labels(Target::Path("missing/".into())).is_empty());
        assert_eq!(labels(Target::Anchor(String::new())), [("top".into(), 18, "top".into())]);
        let anchors: Vec<String> = labels(Target::Anchor("docs/guide.md".into())).into_iter().map(|item| item.0).collect();
        assert_eq!(anchors, ["install-it", "install-it-1", "setup", "top"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warns_about_missing_files_headings_and_definitions() {
        let dir = folder("validate");
        let readme = dir.join("README.md");
        let text = [
            "# Intro",
            "[ok](docs/guide.md) [ok](docs/guide.md#install-it-1) [ok](#intro) [ok](docs/guide) [web](https://x.y/z) [mail](mailto:a@b.c)",
            "[broken](docs/nope.md) ![img](docs/missing.png) [frag](docs/guide.md#nowhere) [own](#outro) [ok](docs/guide.md#top)",
            "[ref][good] [ref][bad] [shortcut] [ok](/docs/logo.png) <img src=\"docs/gone.png\">",
            "`[code](nope.md)`",
            "",
            "[good]: docs/sub%20folder",
            "[def]: docs/absent.md",
        ]
        .join("\n");
        let found: Vec<(usize, String)> = validate(&text, &readme, Some(&dir)).into_iter().map(|d| (d.start.0, d.message)).collect();
        let missing = |p: &str| format!("File does not exist at path: {}", dir.join(p).display());
        assert_eq!(
            found,
            [
                (2, missing("docs/nope.md")),
                (2, missing("docs/missing.png")),
                (2, "No header found: 'nowhere'".into()),
                (2, "No header found: 'outro'".into()),
                (3, "No link definition found: 'bad'".into()),
                (3, missing("docs/gone.png")),
                (7, missing("docs/absent.md")),
            ]
        );
        let first = validate(&text, &readme, Some(&dir)).remove(0);
        assert_eq!((first.start, first.end, first.source.as_str(), first.severity), ((2, 9), (2, 21), "markdown", 2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
