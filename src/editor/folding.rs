use super::buffer::Document;
use super::intel::Lang;
use super::{block_tokens, comment_tokens};
use crate::lsp::{FoldKind, FoldRange};
use std::borrow::Cow;
use std::cmp::Reverse;

pub(super) type Cache = (Option<u64>, Vec<FoldRange>);

pub(super) fn cached<'a>(buf: &Document, lang: &Lang, cache: &'a mut Cache) -> &'a [FoldRange] {
    if cache.0 != Some(buf.version) {
        let lines: Vec<Cow<str>> = buf.rope.lines().map(Cow::from).collect();
        let (open, close) = comment_tokens(buf.path.as_deref());
        let line_comment = close.is_none().then_some(open);
        *cache = (Some(buf.version), ranges(&lines, lang.folding.as_deref(), line_comment, block_tokens(buf.path.as_deref())));
    }
    &cache.1
}

fn ranges(lines: &[Cow<str>], server: Option<&[FoldRange]>, line_comment: Option<&str>, block: Option<(&str, &str)>) -> Vec<FoldRange> {
    let mut found = regions(lines);
    match server {
        Some(server) => found.extend_from_slice(server),
        None => {
            found.extend(indentation(lines));
            found.extend(comments(lines, line_comment, block));
        }
    }
    found.retain(|r| r.0 < r.1 && r.1 < lines.len());
    found.sort_unstable_by_key(|r| (r.0, Reverse(r.1), r.2));
    found.dedup_by_key(|r| r.0);
    found
}

fn marker(line: &str) -> Option<bool> {
    let line = line.trim_start();
    let rest = ["//", "<!--", "/*", "#pragma", "#"].iter().find_map(|open| line.strip_prefix(open))?.trim_start();
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    let end = rest.get(..3).is_some_and(|word| word.eq_ignore_ascii_case("end"));
    let rest = if end { &rest[3..] } else { rest };
    let named = rest.get(..6).is_some_and(|word| word.eq_ignore_ascii_case("region"));
    (named && !rest[6..].starts_with(|c: char| c.is_alphanumeric() || c == '_')).then_some(!end)
}

fn regions(lines: &[Cow<str>]) -> Vec<FoldRange> {
    let (mut open, mut found) = (vec![], vec![]);
    for (at, line) in lines.iter().enumerate() {
        match marker(line) {
            Some(true) => open.push(at),
            Some(false) => found.extend(open.pop().map(|start| (start, at, FoldKind::Region))),
            None => {}
        }
    }
    found
}

fn indentation(lines: &[Cow<str>]) -> Vec<FoldRange> {
    let (mut open, mut found): (Vec<(usize, usize)>, _) = (vec![], vec![]);
    let mut last = 0;
    for (at, line) in lines.iter().enumerate() {
        let text = line.trim_end();
        let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
        if indent == text.len() {
            continue;
        }
        while let Some(&(start, _)) = open.last().filter(|o| o.1 >= indent) {
            open.pop();
            found.extend((last > start).then_some((start, last, FoldKind::Other)));
        }
        open.push((at, indent));
        last = at;
    }
    found.extend(open.into_iter().filter(|o| last > o.0).map(|o| (o.0, last, FoldKind::Other)));
    found
}

fn comments(lines: &[Cow<str>], line_comment: Option<&str>, block: Option<(&str, &str)>) -> Vec<FoldRange> {
    let (mut found, mut run) = (vec![], None);
    for (at, line) in lines.iter().enumerate().chain([(lines.len(), &Cow::Borrowed(""))]) {
        let text = line.trim();
        if line_comment.is_some_and(|open| text.starts_with(open)) && marker(text).is_none() {
            run.get_or_insert(at);
            continue;
        }
        found.extend(run.take().filter(|start| at - start >= 2).map(|start| (start, at - 1, FoldKind::Comment)));
    }
    let Some((open, close)) = block else { return found };
    let mut at = 0;
    while at < lines.len() {
        if let Some(rest) = lines[at].trim_start().strip_prefix(open)
            && !rest.contains(close)
            && let Some(end) = (at + 1..lines.len()).find(|l| lines[*l].contains(close))
        {
            found.push((at, end, FoldKind::Comment));
            at = end;
        }
        at += 1;
    }
    found
}

pub(super) fn innermost(spans: impl Iterator<Item = (usize, usize)>, line: usize) -> Option<(usize, usize)> {
    spans.filter(|s| s.0 <= line && line <= s.1).max_by_key(|s| (s.0, Reverse(s.1)))
}

pub(super) fn at_level(ranges: &[FoldRange], level: usize, lines: &[usize]) -> Vec<(usize, usize)> {
    let (mut outer, mut found) = (vec![], vec![]);
    for r in ranges {
        while outer.last().is_some_and(|end| *end < r.1) {
            outer.pop();
        }
        outer.push(r.1);
        if outer.len() == level && !lines.iter().any(|l| r.0 <= *l && *l <= r.1) {
            found.push((r.0, r.1));
        }
    }
    found
}

pub(super) fn recursive(ranges: &[FoldRange], folds: &mut Vec<(usize, usize)>, line: usize, fold: bool) {
    let Some(outer) = innermost(ranges.iter().map(|r| (r.0, r.1)).chain(folds.iter().copied()), line) else { return };
    let inside = |f: &(usize, usize)| outer.0 <= f.0 && f.1 <= outer.1;
    match fold {
        true => folds.extend(ranges.iter().map(|r| (r.0, r.1)).chain([outer]).filter(inside)),
        false => folds.retain(|f| !inside(f)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FoldKind::*;

    fn lines(text: &str) -> Vec<Cow<'_, str>> {
        text.split_inclusive('\n').map(Cow::Borrowed).collect()
    }

    #[test]
    fn finds_region_markers_of_every_language() {
        let opens = "//#region a\n// #region\n  # region Name\n#pragma region x\n<!-- #region -->\n/* #region b */\n";
        let text = opens.to_owned() + "/* #endregion */\n<!-- #endregion -->\n#pragma endregion\n  #endregion\n// #endregion\n//#endregion tail\n";
        let found = regions(&lines(&text));
        assert_eq!(found, (0..6).rev().map(|start| (start, 11 - start, Region)).collect::<Vec<_>>());
        assert_eq!(marker("#Region Foo"), Some(true));
        assert_eq!(marker("# regional settings"), None);
        assert_eq!(marker("region"), None);
        assert_eq!(marker("#pragma once"), None);
        assert_eq!(regions(&lines("// #endregion\n// #region\n")), vec![]);
    }

    #[test]
    fn falls_back_to_indentation_comments_and_markers() {
        let text = "// one\n// two\nfn a() {\n    if x {\n\n        y();\n    }\n}\n/* long\n   comment */\n// #region r\nfn b() {}\n// #endregion\n";
        let found = ranges(&lines(text), None, Some("//"), Some(("/*", "*/")));
        assert_eq!(found, vec![(0, 1, Comment), (2, 6, Other), (3, 5, Other), (8, 9, Comment), (10, 12, Region)]);
        let server = [(2, 6, Other), (2, 7, Imports), (9, 30, Other)];
        let found = ranges(&lines(text), Some(&server), Some("//"), Some(("/*", "*/")));
        assert_eq!(found, vec![(2, 7, Imports), (10, 12, Region)]);
    }

    #[test]
    fn folds_by_level_except_around_a_cursor() {
        let ranges = [(0, 20, Other), (1, 5, Other), (2, 4, Other), (6, 10, Comment), (7, 8, Other), (12, 20, Other), (21, 25, Other)];
        assert_eq!(at_level(&ranges, 1, &[]), vec![(0, 20), (21, 25)]);
        assert_eq!(at_level(&ranges, 2, &[3]), vec![(6, 10), (12, 20)]);
        assert_eq!(at_level(&ranges, 3, &[]), vec![(2, 4), (7, 8)]);
        assert!(at_level(&ranges, 4, &[]).is_empty());
    }

    #[test]
    fn folds_and_unfolds_recursively() {
        let ranges = [(0, 20, Other), (1, 5, Other), (2, 4, Other), (6, 10, Other), (21, 25, Other)];
        let mut folds = vec![(21, 25)];
        let mut step = |line, fold| {
            recursive(&ranges, &mut folds, line, fold);
            folds.sort_unstable();
            folds.dedup();
            folds.clone()
        };
        assert_eq!(step(3, true), vec![(2, 4), (21, 25)]);
        assert_eq!(step(0, true), vec![(0, 20), (1, 5), (2, 4), (6, 10), (21, 25)]);
        assert_eq!(step(1, false), vec![(0, 20), (6, 10), (21, 25)]);
        assert_eq!(step(0, false), vec![(21, 25)]);
        assert_eq!(step(30, true), vec![(21, 25)]);
    }
}
