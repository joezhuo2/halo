use fancy_regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Url(String),
    File(PathBuf, Option<(usize, usize)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub target: Target,
}

pub fn join(rows: &[(Vec<char>, bool)]) -> (String, Vec<(usize, usize)>) {
    let (mut text, mut cells) = (String::new(), vec![]);
    for (r, (row, wrapped)) in rows.iter().enumerate() {
        let len = if *wrapped { row.len() } else { row.iter().rposition(|c| *c != ' ' && *c != '\0').map_or(0, |i| i + 1) };
        for (col, c) in row[..len].iter().enumerate().filter(|(_, c)| **c != '\0') {
            text.push(*c);
            cells.push((r, col));
        }
        if !wrapped && r + 1 < rows.len() {
            text.push('\n');
            cells.push((r, len));
        }
    }
    (text, cells)
}

static CANDIDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?<url>\b(?:https?|file)://[^\s"'<>`]+)|"(?<dq>[^"\n]+)"|'(?<sq>[^'\n]+)'|(?<path>(?:[A-Za-z]:[\\/]|~[\\/]|\.{1,2}[\\/]|[\\/])?[^\s"'`<>|:*?()\[\]{},;=]+)"#,
    )
    .unwrap()
});

static SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?::(\d+)(?::(\d+))?|\((\d+)(?:,\s*(\d+))?\)|, line (\d+))").unwrap());

fn suffix(rest: &str) -> (usize, Option<(usize, usize)>) {
    let Ok(Some(caps)) = SUFFIX.captures(rest) else { return (0, None) };
    let num = |i: usize| caps.get(i).and_then(|m| m.as_str().parse::<usize>().ok());
    let line = num(1).or(num(3)).or(num(5));
    let col = num(2).or(num(4)).unwrap_or(1);
    (caps.get(0).map_or(0, |m| m.end()), line.map(|line| (line, col)))
}

fn resolve(raw: &str, cwd: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return Some(home?.join(rest));
    }
    let path = Path::new(raw);
    if path.is_absolute() || path.has_root() {
        return Some(path.to_path_buf());
    }
    Some(cwd?.join(path))
}

pub fn find(text: &str, cwd: Option<&Path>, home: Option<&Path>, exists: impl Fn(&Path) -> bool) -> Vec<Link> {
    let chars = |byte: usize| text[..byte].chars().count();
    let (mut out, mut pos) = (vec![], 0);
    while pos < text.len() {
        let Ok(Some(caps)) = CANDIDATE.captures_from_pos(text, pos) else { break };
        let whole = caps.get(0).unwrap();
        pos = whole.end().max(pos + 1);
        if let Some(url) = caps.name("url") {
            let trimmed = url.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'']);
            out.push(Link { start: chars(url.start()), end: chars(url.start() + trimmed.len()), target: Target::Url(trimmed.to_owned()) });
            pos = url.start() + trimmed.len();
            continue;
        }
        let quoted = caps.name("dq").or(caps.name("sq"));
        let body = quoted.or(caps.name("path")).unwrap();
        let mut raw = body.as_str();
        if !raw.contains(['/', '\\', '.']) || raw.chars().all(|c| c == '.') {
            if quoted.is_some() {
                pos = whole.start() + 1;
            }
            continue;
        }
        let mut path = resolve(raw, cwd, home).filter(|p| exists(p));
        if path.is_none() && quoted.is_none() && raw.ends_with('.') {
            raw = raw.trim_end_matches('.');
            path = resolve(raw, cwd, home).filter(|p| exists(p));
        }
        let Some(path) = path else {
            if quoted.is_some() {
                pos = whole.start() + 1;
            }
            continue;
        };
        let after = if quoted.is_some() { whole.end() } else { body.start() + raw.len() };
        let (len, place) = suffix(&text[after..]);
        let start = if quoted.is_some() { whole.start() } else { body.start() };
        out.push(Link { start: chars(start), end: chars(after + len), target: Target::File(path, place) });
        pos = after + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(text: &str, files: &[&str]) -> Vec<(String, Target)> {
        let cwd = if cfg!(windows) { Path::new(r"C:\work") } else { Path::new("/work") };
        let home = if cfg!(windows) { Path::new(r"C:\home") } else { Path::new("/home/me") };
        let known: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        find(text, Some(cwd), Some(home), |p| known.iter().any(|k| k == p))
            .into_iter()
            .map(|l| (text.chars().skip(l.start).take(l.end - l.start).collect(), l.target))
            .collect()
    }

    fn file(path: &str, place: Option<(usize, usize)>) -> Target {
        Target::File(PathBuf::from(path), place)
    }

    #[test]
    fn finds_urls() {
        let found = links("see https://example.com/a?b=1, and (http://x.org/y). done", &[]);
        assert_eq!(found[0], ("https://example.com/a?b=1".into(), Target::Url("https://example.com/a?b=1".into())));
        assert_eq!(found[1].0, "http://x.org/y");
        assert_eq!(found.len(), 2);
    }

    #[cfg(not(windows))]
    #[test]
    fn finds_unix_paths_in_every_form() {
        let files = ["/work/src/main.rs", "/abs/lib.rs", "/home/me/notes.txt", "/work/a b.py", "/work/Cargo.toml", "/work/x.py"];
        let found = links("error at src/main.rs:12:5 here", &files);
        assert_eq!(found, vec![("src/main.rs:12:5".into(), file("/work/src/main.rs", Some((12, 5))))]);
        assert_eq!(links("/abs/lib.rs:7", &files)[0].1, file("/abs/lib.rs", Some((7, 1))));
        assert_eq!(links("open ~/notes.txt.", &files)[0], ("~/notes.txt".into(), file("/home/me/notes.txt", None)));
        assert_eq!(links("in ./src/main.rs(3,9)", &files)[0], ("./src/main.rs(3,9)".into(), file("/work/./src/main.rs", Some((3, 9)))));
        assert_eq!(links("Cargo.toml(4)", &files)[0].1, file("/work/Cargo.toml", Some((4, 1))));
        let py = links(r#"  File "a b.py", line 31, in <module>"#, &files);
        assert_eq!(py, vec![(r#""a b.py", line 31"#.into(), file("/work/a b.py", Some((31, 1))))]);
        assert_eq!(links("'x.py':2", &files)[0], ("'x.py':2".into(), file("/work/x.py", Some((2, 1)))));
        assert!(links("missing.rs:1 and hello world", &files).is_empty());
        assert_eq!(links(r#"echo "see src/main.rs" now"#, &files)[0].0, "src/main.rs");
    }

    #[cfg(windows)]
    #[test]
    fn finds_windows_paths_in_every_form() {
        let files = [r"C:\work\src\main.rs", r"D:\abs\lib.rs", r"C:\home\notes.txt", r"C:\work\a b.py", r"C:\work\Cargo.toml", r"C:\Program Files\x.py"];
        let found = links(r"error at src\main.rs:12:5 here", &files);
        assert_eq!(found, vec![(r"src\main.rs:12:5".into(), file(r"C:\work\src\main.rs", Some((12, 5))))]);
        assert_eq!(links("src/main.rs:3", &files)[0].1, file(r"C:\work\src/main.rs", Some((3, 1))));
        assert_eq!(links(r"D:\abs\lib.rs:7", &files)[0].1, file(r"D:\abs\lib.rs", Some((7, 1))));
        assert_eq!(links("D:/abs/lib.rs", &files)[0].1, file("D:/abs/lib.rs", None));
        assert_eq!(links(r"open ~\notes.txt.", &files)[0], (r"~\notes.txt".into(), file(r"C:\home\notes.txt", None)));
        assert_eq!(links(r"in .\src\main.rs(3,9)", &files)[0], (r".\src\main.rs(3,9)".into(), file(r"C:\work\.\src\main.rs", Some((3, 9)))));
        assert_eq!(links("Cargo.toml(4, 2)", &files)[0].1, file(r"C:\work\Cargo.toml", Some((4, 2))));
        let py = links(r#"  File "C:\Program Files\x.py", line 31, in <module>"#, &files);
        assert_eq!(py, vec![(r#""C:\Program Files\x.py", line 31"#.into(), file(r"C:\Program Files\x.py", Some((31, 1))))]);
        assert_eq!(links("'a b.py':2:4", &files)[0], ("'a b.py':2:4".into(), file(r"C:\work\a b.py", Some((2, 4)))));
        assert!(links("missing.rs:1 and hello world", &files).is_empty());
    }

    #[test]
    fn joins_wrapped_rows_and_links_across_them() {
        let row = |s: &str, wrapped| (s.chars().collect::<Vec<_>>(), wrapped);
        let (text, cells) = join(&[row("at src/ma", true), row("in.rs:4   ", false), row("next", false)]);
        assert_eq!(text, "at src/main.rs:4\nnext");
        assert_eq!(cells[9], (1, 0));
        assert_eq!(cells[16], (1, 7));
        let main = if cfg!(windows) { r"C:\work\src/main.rs" } else { "/work/src/main.rs" };
        let found = links(&text, &[main]);
        assert_eq!(found[0].1, file(main, Some((4, 1))));
        let wide = join(&[(vec!['a', '漢', '\0', 'b'], false)]);
        assert_eq!(wide, ("a漢b".into(), vec![(0, 0), (0, 1), (0, 3)]));
    }
}
