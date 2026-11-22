use super::buffer::{Document, Sel};
use eframe::egui::{Event, Key};
use fancy_regex::Regex;

const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

#[derive(Default)]
pub struct Link {
    pub ranges: Vec<(usize, usize)>,
    word: Option<Regex>,
    version: u64,
    seen: (u64, usize),
}

fn tag_char(c: char) -> bool {
    c.is_alphanumeric() || "-_:.".contains(c)
}

fn starts(text: &[char], at: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(k, c)| text.get(at + k).is_some_and(|t| t.eq_ignore_ascii_case(&c)))
}

fn tags(text: &[char], html: bool) -> Vec<((usize, usize), bool, bool)> {
    let (mut out, mut i) = (vec![], 0);
    while i < text.len() {
        if text[i] != '<' {
            i += 1;
            continue;
        }
        if starts(text, i, "<!--") {
            i = (i + 4..text.len()).find(|k| starts(text, *k, "-->")).map_or(text.len(), |k| k + 3);
            continue;
        }
        let close = text.get(i + 1) == Some(&'/');
        let start = i + 1 + close as usize;
        let end = start + text[start..].iter().take_while(|c| tag_char(**c)).count();
        if !(end > start && text[start].is_alphabetic() || end == start && text.get(end) == Some(&'>')) {
            i += 1;
            continue;
        }
        let (mut j, mut quote, mut braces) = (end, None, 0usize);
        while j < text.len() {
            match (quote, text[j]) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, c @ ('"' | '\'' | '`')) => quote = Some(c),
                (None, '{') => braces += 1,
                (None, '}') => braces = braces.saturating_sub(1),
                (None, '>' | '<') if braces == 0 => break,
                _ => {}
            }
            j += 1;
        }
        let name: String = text[start..end].iter().collect::<String>().to_lowercase();
        let single = !close && (j > end && text.get(j) == Some(&'>') && text[j - 1] == '/' || html && VOID.contains(&name.as_str()));
        out.push(((start, end), close, single));
        i = j + (text.get(j) == Some(&'>')) as usize;
        if html && !close && !single && (name == "script" || name == "style") {
            let closer = format!("</{name}");
            i = (i..text.len()).find(|k| starts(text, *k, &closer)).unwrap_or(text.len());
        }
    }
    out
}

pub fn pair(text: &[char], pos: usize, html: bool) -> Option<[(usize, usize); 2]> {
    let tags = tags(text, html);
    let at = tags.iter().position(|t| t.0.0 <= pos && pos <= t.0.1 && !t.2)?;
    let ((start, end), close, _) = tags[at];
    let mut depth = 0;
    let mut matching = |t: &&((usize, usize), bool, bool)| {
        if t.2 || text[t.0.0..t.0.1] != text[start..end] {
            return false;
        }
        match (t.1 == close, depth) {
            (true, _) => depth += 1,
            (false, 0) => return true,
            (false, _) => depth -= 1,
        }
        false
    };
    let found = match close {
        false => tags[at + 1..].iter().find(&mut matching),
        true => tags[..at].iter().rev().find(&mut matching),
    }?;
    Some(if close { [found.0, (start, end)] } else { [(start, end), found.0] })
}

pub fn builtin(buf: &Document, head: usize) -> Option<Vec<(usize, usize)>> {
    let ext = buf.path.as_ref()?.extension()?.to_string_lossy().to_lowercase();
    let html = matches!(ext.as_str(), "html" | "htm" | "xhtml" | "vue" | "svelte");
    if !html && !matches!(ext.as_str(), "xml" | "svg" | "jsx" | "tsx") || buf.len() > 1_000_000 {
        return None;
    }
    let text: Vec<char> = buf.rope.chars().collect();
    pair(&text, head, html).map(|found| found.to_vec())
}

impl Link {
    fn at(&self, buf: &Document, sels: &[Sel]) -> Option<usize> {
        let s = sels.first().filter(|_| sels.len() == 1 && self.version == buf.version && self.ranges.len() > 1)?;
        self.ranges.iter().position(|r| r.0 <= s.start() && s.end() <= r.1)
    }

    pub fn shown(&self, buf: &Document, sels: &[Sel]) -> &[(usize, usize)] {
        if self.at(buf, sels).is_some() { &self.ranges } else { &[] }
    }

    pub fn stale(&mut self, buf: &Document, sels: &[Sel]) -> Option<usize> {
        let head = sels.first().filter(|_| sels.len() == 1)?.head;
        if self.seen == (buf.version, head) || self.at(buf, sels).is_some() {
            return None;
        }
        self.seen = (buf.version, head);
        self.ranges.clear();
        Some(head)
    }

    pub fn set(&mut self, buf: &Document, mut ranges: Vec<(usize, usize)>, word: Option<&str>) {
        ranges.sort();
        self.ranges = ranges;
        self.word = word.and_then(|pattern| Regex::new(&format!("^(?:{pattern})$")).ok());
        self.version = buf.version;
    }

    pub fn spread(&self, buf: &Document, sels: &[Sel], event: &Event) -> Option<(Vec<Sel>, usize)> {
        let at = self.at(buf, sels)?;
        let (s, range) = (sels[0], self.ranges[at]);
        let fits = |text: &str| match &self.word {
            Some(word) => {
                let whole = format!("{}{text}{}", buf.rope.slice(range.0..s.start()), buf.rope.slice(s.end()..range.1));
                word.is_match(&whole).unwrap_or(false)
            }
            None => text.chars().all(tag_char),
        };
        let mirrored = match event {
            Event::Text(text) | Event::Paste(text) => fits(text),
            Event::Key { key: Key::Backspace, pressed: true, .. } => !s.is_empty() || s.head > range.0,
            Event::Key { key: Key::Delete, pressed: true, .. } => !s.is_empty() || s.head < range.1,
            _ => false,
        };
        mirrored.then(|| (self.ranges.iter().map(|r| Sel::new(r.0 + s.anchor - range.0, r.0 + s.head - range.0)).collect(), at))
    }

    pub fn follow(&mut self, buf: &Document, sels: &mut Vec<Sel>, at: usize, before: usize) {
        let count = self.ranges.len() as isize;
        let delta = buf.len() as isize - before as isize;
        if sels.len() as isize == count && delta % count == 0 {
            let moved = |pos: usize, steps: usize| (pos as isize + delta / count * steps as isize) as usize;
            for (index, range) in self.ranges.iter_mut().enumerate() {
                *range = (moved(range.0, index), moved(range.1, index + 1));
            }
            self.version = buf.version;
            self.seen = (buf.version, sels[at].head);
        } else {
            self.ranges.clear();
        }
        *sels = vec![sels.get(at).copied().unwrap_or(sels[0])];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, at: &str, html: bool) -> Option<(String, String)> {
        let chars: Vec<char> = text.chars().collect();
        let pos = text.find(at).map(|b| text[..b].chars().count() + 1)?;
        let [a, b] = pair(&chars, pos, html)?;
        Some((format!("{}..{}", a.0, a.1), format!("{}..{}", b.0, b.1)))
    }

    #[test]
    fn matches_tags_with_nesting_attributes_and_skipped_bodies() {
        let page = "<div a=\"x>y\" b={c > d}><div/><br><div><p>t</p></div><!-- </div> --></div>";
        assert_eq!(find(page, "<div a", true), Some(("1..4".into(), "69..72".into())));
        let chars: Vec<char> = page.chars().collect();
        assert_eq!(pair(&chars, 34, true), Some([(34, 37), (48, 51)]));
        assert_eq!(pair(&chars, 50, true), Some([(34, 37), (48, 51)]));
        assert_eq!(find(page, "<div/", true), None);
        assert_eq!(find(page, "<br", true), None);
        assert_eq!(find("<br></br>", "<br", false), Some(("1..3".into(), "6..8".into())));
        assert_eq!(find("<script>if (a<b) x = '</p>'</script>", "<script", true), Some(("1..7".into(), "29..35".into())));
        assert_eq!(find("<p><script></p>", "<p", false), Some(("1..2".into(), "13..14".into())));
        assert_eq!(find("<><a></a></>", "<>", false).map(|r| r.1), Some("11..11".into()));
        assert_eq!(find("let x = a < b; <Foo.Bar>{x}</Foo.Bar>", "<Foo", false), Some(("16..23".into(), "29..36".into())));
        assert_eq!(find("<div><span></div>", "<span", true), None);
    }

    #[test]
    fn mirrors_edits_inside_a_range_and_stops_on_other_characters() {
        let mut buf = Document::from_text("<div>x</div>", "t".into());
        let mut link = Link::default();
        let mut sels = vec![Sel::caret(4)];
        assert_eq!(link.stale(&buf, &sels), Some(4));
        assert_eq!(link.stale(&buf, &sels), None);
        let found = pair(&buf.rope.chars().collect::<Vec<_>>(), 4, false).unwrap();
        link.set(&buf, found.to_vec(), None);
        for event in [Event::Text("v".into()), Event::Key { key: Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }, Event::Text("x".into())] {
            let (mut spread, at) = link.spread(&buf, &sels, &event).unwrap();
            let before = buf.len();
            match &event {
                Event::Text(text) => buf.insert_text(&mut spread, text),
                _ => buf.backspace(&mut spread, false, false),
            };
            link.follow(&buf, &mut spread, at, before);
            sels = spread;
        }
        assert_eq!(buf.rope.to_string(), "<divx>x</divx>");
        assert_eq!((sels.len(), sels[0].head, link.shown(&buf, &sels).to_vec()), (1, 5, vec![(1, 5), (9, 13)]));
        assert!(link.spread(&buf, &sels, &Event::Text(" ".into())).is_none());
        assert!(link.spread(&buf, &[Sel::caret(1)], &Event::Key { key: Key::Backspace, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }).is_none());
        link.set(&buf, vec![(1, 5), (9, 13)], Some("[a-z]+"));
        assert!(link.spread(&buf, &sels, &Event::Text("y".into())).is_some());
        assert!(link.spread(&buf, &sels, &Event::Text("-".into())).is_none());
    }
}
