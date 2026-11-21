use super::buffer::{After, Document, Kind, Sel};
use std::collections::BTreeMap;
use std::iter::Peekable;
use std::path::PathBuf;
use std::str::Chars;
use std::sync::Mutex;

type Stops = Vec<Vec<(usize, usize)>>;
type Library = BTreeMap<String, BTreeMap<String, String>>;

const SAMPLE: &str = r##"{
  "*": {
    "todo": "TODO: $0"
  },
  "rs": {
    "test": "#[test]\nfn ${1:name}() {\n\t$0\n}"
  }
}"##;

static USER: Mutex<Option<Library>> = Mutex::new(None);

pub struct Session {
    stops: Stops,
    at: usize,
    len: usize,
}

pub fn path() -> PathBuf {
    crate::settings::dir().join("snippets.json")
}

pub fn reload() {
    *USER.lock().unwrap() = None;
}

pub fn write_sample() {
    if !path().exists() {
        let _ = std::fs::create_dir_all(crate::settings::dir());
        let _ = std::fs::write(path(), SAMPLE);
    }
}

pub fn user(ext: &str) -> Vec<(String, String)> {
    let mut library = USER.lock().unwrap();
    let library = library.get_or_insert_with(|| {
        let text = if cfg!(test) { SAMPLE.to_owned() } else { std::fs::read_to_string(path()).unwrap_or_else(|_| "{}".into()) };
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
            crate::ui::notify(format!("snippets.json could not be read: {e}"));
            Library::new()
        })
    });
    let found = ["*", ext].into_iter().filter_map(|key| library.get(key)).flatten();
    found.map(|(name, body)| (name.clone(), body.clone())).collect()
}

type Found = BTreeMap<u32, Vec<(usize, usize)>>;

fn walk(chars: &mut Peekable<Chars>, out: &mut String, count: &mut usize, stops: &mut Found, filled: &BTreeMap<u32, String>, nested: bool) {
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.extend(chars.next());
                *count += 1;
            }
            '}' if nested => return,
            '$' => {
                let braced = chars.next_if_eq(&'{').is_some();
                let digits = chars.peek().is_some_and(char::is_ascii_digit);
                let mut name = String::new();
                while let Some(c) = chars.next_if(|c| if digits { c.is_ascii_digit() } else { c.is_alphanumeric() || *c == '_' }) {
                    name.push(c);
                }
                let start = *count;
                let mirror = |out: &mut String, count: &mut usize| {
                    let text = name.parse().ok().and_then(|number: u32| filled.get(&number)).map(String::as_str).unwrap_or_default();
                    out.push_str(text);
                    *count += text.chars().count();
                };
                match (braced, name.is_empty()) {
                    (false, true) => {
                        out.push('$');
                        *count += 1;
                    }
                    (false, false) => mirror(out, count),
                    (true, _) => match chars.next() {
                        Some(':') => walk(chars, out, count, stops, filled, true),
                        Some('|') => {
                            let choices: String = chars.by_ref().take_while(|c| *c != '|').collect();
                            let first = choices.split(',').next().unwrap_or_default();
                            chars.next_if_eq(&'}');
                            out.push_str(first);
                            *count += first.chars().count();
                        }
                        _ => mirror(out, count),
                    },
                }
                if let Ok(number) = name.parse() {
                    stops.entry(number).or_default().push((start, *count));
                }
            }
            c => {
                out.push(c);
                *count += 1;
            }
        }
    }
}

pub fn parse(body: &str) -> (String, Stops) {
    let run = |filled: &BTreeMap<u32, String>| {
        let (mut out, mut count, mut found) = (String::new(), 0, Found::new());
        walk(&mut body.chars().peekable(), &mut out, &mut count, &mut found, filled, false);
        (out, count, found)
    };
    let (plain, _, found) = run(&BTreeMap::new());
    let chars: Vec<char> = plain.chars().collect();
    let given = |ranges: &Vec<(usize, usize)>| ranges.iter().find(|r| r.1 > r.0).map(|r| chars[r.0..r.1].iter().collect::<String>());
    let filled: BTreeMap<u32, String> = found.iter().filter_map(|(number, ranges)| Some((*number, given(ranges)?))).collect();
    let (out, count, mut found) = run(&filled);
    let last = found.remove(&0).unwrap_or_else(|| vec![(count, count)]);
    let mut stops: Stops = found.into_values().collect();
    stops.push(last);
    (out, stops)
}

pub fn indented(buf: &Document, at: usize, body: &str, unit: &str) -> String {
    let line = buf.line_of(at);
    let indent: String = buf.rope.line(line).chars().take(buf.indent_of(line)).collect();
    body.replace('\t', unit).replace('\n', &format!("\n{indent}"))
}

pub fn insert(buf: &mut Document, sels: &mut Vec<Sel>, from: usize, to: usize, body: &str, unit: &str) -> Option<Session> {
    let (text, stops) = parse(&indented(buf, from, body, unit));
    let changed = buf.apply(sels, vec![(from, to, text)], Kind::Other, After::Carets);
    changed.then(|| Session::begin(buf, sels, from, stops)).flatten()
}

fn select(group: &[(usize, usize)]) -> Vec<Sel> {
    group.iter().map(|range| Sel::new(range.0, range.1)).collect()
}

impl Session {
    pub fn begin(buf: &mut Document, sels: &mut Vec<Sel>, start: usize, mut stops: Stops) -> Option<Self> {
        for range in stops.iter_mut().flatten() {
            *range = (range.0 + start, range.1 + start);
        }
        *sels = select(&stops[0]);
        buf.fix_after(sels);
        (stops.len() > 1).then(|| Self { stops, at: 0, len: buf.len() })
    }

    // shortcut: a stop that sits exactly where another one ends moves with it, upgrade to per-edit mapping if stops drift
    fn follows(&mut self, len: usize, sels: &[Sel]) -> bool {
        let group = self.stops[self.at].clone();
        let delta = len as isize - self.len as isize;
        let per = delta / group.len() as isize;
        if delta % group.len() as isize != 0 {
            return false;
        }
        self.len = len;
        let moved = |pos: usize, steps: usize| (pos as isize + per * steps as isize).max(0) as usize;
        for (index, stop) in self.stops.iter_mut().enumerate() {
            for (place, range) in stop.iter_mut().enumerate() {
                let before = |pos: usize| group.iter().filter(|g| g.1 <= pos).count();
                *range = match index == self.at {
                    true => (moved(range.0, place), moved(range.1, place + 1)),
                    false => (moved(range.0, before(range.0)), moved(range.1, before(range.1))),
                };
            }
        }
        let now = &self.stops[self.at];
        sels.len() == now.len() && sels.iter().zip(now).all(|(s, range)| range.0 <= s.start() && s.end() <= range.1)
    }
}

pub fn track(session: &mut Option<Session>, len: usize, sels: &[Sel]) {
    if session.as_mut().is_some_and(|snippet| !snippet.follows(len, sels)) {
        *session = None;
    }
}

pub fn step(session: &mut Option<Session>, sels: &mut Vec<Sel>, forward: bool) -> bool {
    let Some(snippet) = session else { return false };
    snippet.at = if forward { snippet.at + 1 } else { snippet.at.saturating_sub(1) };
    *sels = select(&snippet.stops[snippet.at]);
    if snippet.at + 1 == snippet.stops.len() {
        *session = None;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stops_and_follows_typing_through_mirrors() {
        let (text, stops) = parse("for ${1:i} in $2 { $1\\$ ${3|a,b|}$0 } ${VAR:d}$TM");
        assert_eq!(text, "for i in  { i$ a } d");
        assert_eq!(stops, vec![vec![(4, 5), (12, 13)], vec![(9, 9)], vec![(15, 16)], vec![(16, 16)]]);
        assert_eq!(parse("plain").1, vec![vec![(5, 5)]]);

        let mut buf = Document::from_text("  x", "t".into());
        let mut sels = vec![Sel::caret(3)];
        let mut session = insert(&mut buf, &mut sels, 2, 3, "fn ${1:a}($1) {\n\t$0\n}", "    ");
        assert_eq!(buf.rope.to_string(), "  fn a(a) {\n      \n  }");
        assert_eq!(sels.iter().map(|s| (s.start(), s.end())).collect::<Vec<_>>(), vec![(5, 6), (7, 8)]);
        buf.insert_text(&mut sels, "n");
        buf.insert_text(&mut sels, "m");
        track(&mut session, buf.len(), &sels);
        assert_eq!(buf.rope.to_string(), "  fn nm(nm) {\n      \n  }");
        assert!(step(&mut session, &mut sels, true));
        assert_eq!((sels[0].head, session.is_none()), (20, true));
        assert!(!step(&mut session, &mut sels, true));

        let mut session = insert(&mut buf, &mut sels, 20, 20, "${1:one} ${2:two}", "");
        sels = vec![Sel::caret(0)];
        track(&mut session, buf.len(), &sels);
        assert!(session.is_none());
        assert_eq!(user("rs").len(), 2);
    }
}
