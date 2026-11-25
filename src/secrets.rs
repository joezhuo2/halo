use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Assignment, Netrc, Url, Pem }

pub fn kind(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    match name.as_str() {
        ".netrc" => Some(Kind::Netrc),
        ".git-credentials" => Some(Kind::Url),
        ".npmrc" | ".pypirc" | "credentials" | ".env" => Some(Kind::Assignment),
        n if n.starts_with(".env.") || n.ends_with(".env") => Some(Kind::Assignment),
        n if n.ends_with(".pem") || n.ends_with(".key") => Some(Kind::Pem),
        _ => None,
    }
}

pub fn boundary(line: &str) -> Option<bool> {
    let line = line.trim();
    if !line.ends_with("-----") { return None }
    if line.starts_with("-----BEGIN ") { Some(true) }
    else if line.starts_with("-----END ") { Some(false) }
    else { None }
}

pub fn contextual(kind: Option<Kind>) -> bool {
    matches!(kind, Some(Kind::Pem | Kind::Netrc))
}

pub fn after(kind: Option<Kind>, text: &str, state: bool) -> bool {
    match kind {
        Some(Kind::Pem) => boundary(text).unwrap_or(state),
        Some(Kind::Netrc) => netrc(text, state, |_, _| {}),
        _ => false,
    }
}

pub fn line_states(kind: Option<Kind>, lines: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<bool> {
    if !contextual(kind) { return vec![] }
    let mut state = false;
    lines.into_iter().map(|line| {
        let before = state;
        state = after(kind, line.as_ref(), state);
        before
    }).collect()
}

fn value_end(text: &str) -> usize {
    let (mut quote, mut escaped) = (None, false);
    for (at, c) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() {
            if c == '#' && (at == 0 || text[..at].ends_with(char::is_whitespace)) { return at }
            if matches!(c, '"' | '\'') { quote = Some(c); }
        }
    }
    text.len()
}

fn netrc(line: &str, mut password: bool, mut add: impl FnMut(usize, usize)) -> bool {
    let content = &line[..value_end(line)];
    let mut rest = content;
    while !rest.trim_start().is_empty() {
        rest = rest.trim_start();
        let at = content.len() - rest.len();
        let quote = rest.starts_with(['"', '\'']);
        let mut escaped = false;
        let end = rest.char_indices().skip(usize::from(quote)).find_map(|(i, c)| {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if (quote && rest.starts_with(c)) || (!quote && c.is_whitespace()) { return Some(i) }
            None
        }).unwrap_or(rest.len());
        if password { add(at + usize::from(quote), at + end); }
        password = &rest[..end] == "password";
        rest = &rest[(end + usize::from(quote)).min(rest.len())..];
    }
    password
}

pub fn ranges(kind: Kind, line: &str, continued: bool) -> Vec<Range<usize>> {
    let mut spans = vec![];
    let mut add = |start: usize, end: usize| {
        if start < end { spans.push(line[..start].chars().count()..line[..end].chars().count()); }
    };
    if line.trim_start().starts_with('#') { return spans }
    match kind {
        Kind::Assignment => {
            let separator = if line.trim_start().starts_with("//") { line.find('=') } else { line.find(['=', ':']) };
            if let Some(at) = separator {
                let rhs = &line[at + 1..];
                let val = rhs[..value_end(rhs)].trim();
                let start = at + 1 + rhs.len() - rhs.trim_start().len();
                let quoted = val.starts_with(['"', '\'']);
                let end_quote = quoted && val.len() > 1 && val.as_bytes().first() == val.as_bytes().last();
                add(start + usize::from(quoted), start + val.len() - usize::from(end_quote));
            }
        }
        Kind::Netrc => { netrc(line, continued, add); }
        Kind::Url => {
            if let Some((_, rest)) = line.split_once("://") {
                let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
                if let Some((info, _)) = authority.rsplit_once('@') && let Some(colon) = info.find(':') {
                    let at = line.len() - rest.len();
                    add(at + colon + 1, at + info.len());
                }
            }
        }
        Kind::Pem if continued && boundary(line).is_none() => add(0, line.len()),
        Kind::Pem => {}
    }
    spans
}

pub fn fragment(kind: Option<Kind>, text: &str) -> Cow<'_, str> {
    if kind != Some(Kind::Netrc) { return mask(kind, text, true) }
    let mut spans = ranges(Kind::Netrc, text, false);
    if let Some(initial) = ranges(Kind::Netrc, text, true).into_iter().next()
        && spans.first() != Some(&initial)
    {
        spans.insert(0, initial);
    }
    masked(text, &spans)
}

pub fn mask(kind: Option<Kind>, text: &str, continued: bool) -> Cow<'_, str> {
    let Some(kind) = kind else { return Cow::Borrowed(text) };
    masked(text, &ranges(kind, text, continued))
}

fn masked<'a>(text: &'a str, spans: &[Range<usize>]) -> Cow<'a, str> {
    if spans.is_empty() { return Cow::Borrowed(text) }
    let mut spans = spans.iter().peekable();
    Cow::Owned(text.chars().enumerate().map(|(col, c)| {
        while spans.peek().is_some_and(|r| r.end <= col) { spans.next(); }
        if spans.peek().is_some_and(|r| r.contains(&col)) { '\u{2022}' } else { c }
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_basenames() {
        for (names, expected) in [
            (&[".env", ".env.local", "prod.env", ".npmrc", ".pypirc", "credentials"][..], Kind::Assignment),
            (&[".netrc"][..], Kind::Netrc), (&[".git-credentials"][..], Kind::Url), (&["a.pem", "tls.key"][..], Kind::Pem),
        ] {
            for name in names { assert_eq!(kind(&Path::new("nested").join(name)), Some(expected)); }
        }
        for path in [".environment", "credentials.txt", ".env/file", "key.rs"] { assert_eq!(kind(Path::new(path)), None); }
    }

    #[test]
    fn column_ranges() {
        use Kind::*;
        assert_eq!(ranges(Assignment, "export K='a=b' # note", false), vec![10..13]);
        assert_eq!(ranges(Assignment, "//host/:_authToken=abc", false), vec![19..22]);
        assert_eq!(ranges(Assignment, "\u{e9}: \u{1f512}z", false), vec![3..5]);
        assert_eq!(ranges(Netrc, "password abc password xyz", false), vec![9..12, 22..25]);
        assert_eq!(ranges(Url, "https://me:a:b@host/x", false), vec![11..14]);
        assert_eq!(ranges(Pem, "abc", true), vec![0..3]);
        assert_eq!(ranges(Assignment, "password: xyz==", false), vec![10..15]);
        assert_eq!(ranges(Netrc, r#"password "a\"b c""#, false), vec![10..16]);
        assert_eq!(line_states(Some(Pem), ["note", "-----BEGIN KEY-----", "payload", "-----END KEY-----", "note"]), [false, false, true, true, false]);
        assert_eq!(line_states(Some(Netrc), ["machine h password", "# comment", "abc", "login u"]), [false, true, true, false]);
        assert_eq!(ranges(Netrc, "  abc login u", true), vec![2..5]);
        assert!(ranges(Pem, "abc", false).is_empty());
        for line in ["-----BEGIN KEY-----", "-----END KEY-----", " # abc"] { assert!(ranges(Pem, line, true).is_empty()); }
        assert!(ranges(Assignment, "# K=value", false).is_empty());
        assert!(ranges(Assignment, "K=  ", false).is_empty());
    }
}
