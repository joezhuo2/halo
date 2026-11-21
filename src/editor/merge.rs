use super::diff::{self, Hunk};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Same,
    Current,
    Incoming,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Base,
    Current,
    Incoming,
    Both(bool),
    Edited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub kind: Kind,
    pub base: Range<usize>,
    pub current: Range<usize>,
    pub incoming: Range<usize>,
    pub state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub chunk: usize,
    pub lines: [Option<usize>; 4],
}

#[derive(Default)]
pub struct Merge {
    pub base: Vec<String>,
    pub current: Vec<String>,
    pub incoming: Vec<String>,
    pub chunks: Vec<Chunk>,
}

fn lines(text: &str) -> Vec<String> {
    diff::split(&text.replace("\r\n", "\n")).into_iter().map(str::to_owned).collect()
}

fn same(a: &str, b: &str) -> bool {
    a.strip_suffix('\n').unwrap_or(a) == b.strip_suffix('\n').unwrap_or(b)
}

fn locate(hunks: &[Hunk], s: usize, e: usize) -> (Range<usize>, bool) {
    let (mut before, mut after, mut lo, mut hi, mut touched) = (0isize, 0isize, usize::MAX, 0, false);
    for h in hunks {
        let d = h.right.len() as isize - h.left.len() as isize;
        let touch = match h.left.is_empty() {
            true => s <= h.left.start && h.left.start <= e,
            false => h.left.start < e && s < h.left.end,
        };
        if touch {
            (touched, lo, hi, after) = (true, lo.min(h.right.start), hi.max(h.right.end), after + d);
        } else if h.left.start < s {
            (before, after) = (before + d, after + d);
        } else {
            break;
        }
    }
    (((s as isize + before) as usize).min(lo)..((e as isize + after) as usize).max(hi), touched)
}

pub fn toggle(state: State, current: bool) -> State {
    use State::*;
    match (state, current) {
        (Both(_), true) => Incoming,
        (Both(_), false) => Current,
        (Current, true) | (Incoming, false) => Base,
        (Incoming, true) => Both(false),
        (Current, false) => Both(true),
        (_, true) => Current,
        (_, false) => Incoming,
    }
}

impl Merge {
    pub fn new(base: &str, current: &str, incoming: &str) -> Self {
        let (b, c, i) = (lines(base), lines(current), lines(incoming));
        let (ours, theirs) = (diff::diff(&b, &c), diff::diff(&b, &i));
        let mut tagged: Vec<(bool, &Hunk)> = ours.iter().map(|h| (true, h)).chain(theirs.iter().map(|h| (false, h))).collect();
        tagged.sort_by_key(|(_, h)| (h.left.start, h.left.end));
        let (mut chunks, mut at, mut dc, mut di, mut k) = (vec![], 0, 0isize, 0isize, 0);
        let shift = |r: &Range<usize>, d: isize| (r.start as isize + d) as usize..(r.end as isize + d) as usize;
        while k < tagged.len() {
            let (start, mut end, first) = (tagged[k].1.left.start, tagged[k].1.left.end, k);
            k += 1;
            while k < tagged.len() && tagged[k].1.left.start <= end {
                end = end.max(tagged[k].1.left.end);
                k += 1;
            }
            let group = &tagged[first..k];
            if start > at {
                chunks.push(Chunk { kind: Kind::Same, base: at..start, current: shift(&(at..start), dc), incoming: shift(&(at..start), di), state: State::Base });
            }
            let span = |ours: bool, d: isize| {
                let grow: isize = group.iter().filter(|g| g.0 == ours).map(|g| g.1.right.len() as isize - g.1.left.len() as isize).sum();
                (start as isize + d) as usize..(end as isize + d + grow) as usize
            };
            let (current, incoming) = (span(true, dc), span(false, di));
            dc += current.len() as isize - (end - start) as isize;
            di += incoming.len() as isize - (end - start) as isize;
            let has = |ours: bool| group.iter().any(|g| g.0 == ours);
            let kind = match (has(true), has(false)) {
                (true, false) => Kind::Current,
                (false, true) => Kind::Incoming,
                _ if c[current.clone()] == i[incoming.clone()] => Kind::Current,
                _ => Kind::Conflict,
            };
            chunks.push(Chunk { kind, base: start..end, current, incoming, state: State::Base });
            at = end;
        }
        if at < b.len() {
            chunks.push(Chunk { kind: Kind::Same, base: at..b.len(), current: shift(&(at..b.len()), dc), incoming: shift(&(at..b.len()), di), state: State::Base });
        }
        Self { base: b, current: c, incoming: i, chunks }
    }

    pub fn lines_of(&self, chunk: &Chunk, state: State) -> Vec<&str> {
        fn side<'a>(lines: &'a [String], r: &Range<usize>) -> Vec<&'a str> {
            lines[r.clone()].iter().map(String::as_str).collect()
        }
        let (base, current, incoming) = (side(&self.base, &chunk.base), side(&self.current, &chunk.current), side(&self.incoming, &chunk.incoming));
        match (chunk.kind, state) {
            (Kind::Same, _) | (Kind::Conflict, State::Base | State::Edited) => base,
            (Kind::Current, _) | (Kind::Conflict, State::Current) => current,
            (Kind::Incoming, _) | (Kind::Conflict, State::Incoming) => incoming,
            (Kind::Conflict, State::Both(true)) => [current, incoming].concat(),
            (Kind::Conflict, State::Both(false)) => [incoming, current].concat(),
        }
    }

    pub fn render(&self) -> String {
        diff::join(self.chunks.iter().flat_map(|c| self.lines_of(c, c.state)).collect())
    }

    pub fn text(&self, chunk: usize, state: State, more: bool) -> String {
        let mut text = diff::join(self.lines_of(&self.chunks[chunk], state));
        if more && !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    pub fn conflicts(&self) -> impl Iterator<Item = usize> + '_ {
        self.chunks.iter().enumerate().filter(|(_, c)| c.kind == Kind::Conflict).map(|(k, _)| k)
    }

    pub fn remaining(&self) -> usize {
        self.chunks.iter().filter(|c| c.kind == Kind::Conflict && c.state == State::Base).count()
    }

    pub fn sync(&mut self, result: &str) -> Vec<Range<usize>> {
        let expected = self.render();
        let (a, b) = (diff::split(&expected), diff::split(result));
        let hunks = diff::diff(&a, &b);
        let (mut s, mut end, mut ranges, mut states) = (0, 0, Vec::with_capacity(self.chunks.len()), vec![]);
        for (k, chunk) in self.chunks.iter().enumerate() {
            let len = self.lines_of(chunk, chunk.state).len();
            let (found, _) = locate(&hunks, s, s + len);
            let last = k + 1 == self.chunks.len();
            let range = end..found.end.max(end).max(if last { b.len() } else { 0 });
            (s, end) = (s + len, range.end);
            if chunk.kind == Kind::Conflict {
                let actual = b[range.clone()].concat();
                let tries = [chunk.state, State::Base, State::Current, State::Incoming, State::Both(true), State::Both(false)];
                let state = tries.into_iter().find(|st| *st != State::Edited && same(&diff::join(self.lines_of(chunk, *st)), &actual));
                states.push((k, state.unwrap_or(State::Edited)));
            }
            ranges.push(range);
        }
        for (k, state) in states {
            self.chunks[k].state = state;
        }
        ranges
    }

    pub fn align(&self, ranges: &[Range<usize>], base: bool) -> (Vec<Row>, Vec<(usize, u32)>) {
        let (mut rows, mut pads): (Vec<Row>, Vec<(usize, u32)>) = (vec![], vec![]);
        for (k, (chunk, result)) in self.chunks.iter().zip(ranges).enumerate() {
            let shown = [chunk.incoming.clone(), chunk.current.clone(), if base { chunk.base.clone() } else { 0..0 }, result.clone()];
            let height = shown.iter().map(Range::len).max().unwrap_or(0);
            if height > result.len() {
                match pads.last_mut() {
                    Some(last) if last.0 == result.end => last.1 += (height - result.len()) as u32,
                    _ => pads.push((result.end, (height - result.len()) as u32)),
                }
            }
            for r in 0..height {
                rows.push(Row { chunk: k, lines: shown.clone().map(|s| (r < s.len()).then_some(s.start + r)) });
            }
        }
        (rows, pads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(m: &Merge) -> Vec<Kind> {
        m.chunks.iter().map(|c| c.kind).collect()
    }

    fn accept(m: &mut Merge, result: &str, chunk: usize, state: State) -> String {
        let ranges = m.sync(result);
        let lines = diff::split(result);
        let r = ranges[chunk].clone();
        let text = m.text(chunk, state, r.end < lines.len());
        m.chunks[chunk].state = state;
        let out = lines[..r.start].concat() + &text + &lines[r.end..].concat();
        m.sync(&out);
        out
    }

    #[test]
    fn merges_one_sided_and_identical_changes_automatically() {
        let m = Merge::new("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\nf\n");
        assert_eq!(kinds(&m), [Kind::Same, Kind::Current, Kind::Same, Kind::Incoming, Kind::Same, Kind::Incoming]);
        assert_eq!(m.render(), "a\nB\nc\nD\ne\nf\n");
        assert_eq!(m.remaining(), 0);
        let same = Merge::new("a\nb\nc\n", "a\nX\nc\n", "a\nX\nc\n");
        assert_eq!(kinds(&same), [Kind::Same, Kind::Current, Kind::Same]);
        assert_eq!(same.render(), "a\nX\nc\n");
        assert!(Merge::new("", "", "").chunks.is_empty());
    }

    #[test]
    fn finds_conflicts_and_accepts_every_combination() {
        let (base, current, incoming) = ("a\nb\nc\n", "a\nours\nc\n", "a\ntheirs\nmore\nc\n");
        let m = Merge::new(base, current, incoming);
        assert_eq!(kinds(&m), [Kind::Same, Kind::Conflict, Kind::Same]);
        assert_eq!((m.chunks[1].base.clone(), m.chunks[1].current.clone(), m.chunks[1].incoming.clone()), (1..2, 1..2, 1..3));
        assert_eq!(m.conflicts().collect::<Vec<_>>(), [1]);
        assert_eq!((m.render(), m.remaining()), (base.to_owned(), 1));
        for (state, want) in [
            (State::Current, "a\nours\nc\n"),
            (State::Incoming, "a\ntheirs\nmore\nc\n"),
            (State::Both(true), "a\nours\ntheirs\nmore\nc\n"),
            (State::Both(false), "a\ntheirs\nmore\nours\nc\n"),
            (State::Base, base),
        ] {
            let mut m = Merge::new(base, current, incoming);
            assert_eq!(accept(&mut m, base, 1, state), want);
            assert_eq!(m.chunks[1].state, state);
            assert_eq!(m.remaining(), usize::from(state == State::Base));
        }
        use State::*;
        assert_eq!([toggle(Base, true), toggle(Base, false), toggle(Edited, false)], [Current, Incoming, Incoming]);
        assert_eq!([toggle(Current, false), toggle(Incoming, true)], [Both(true), Both(false)]);
        assert_eq!([toggle(Both(true), true), toggle(Both(false), false), toggle(Current, true), toggle(Incoming, false)], [Incoming, Current, Base, Base]);
    }

    #[test]
    fn keeps_hand_edits_and_follows_undo() {
        let base = "a\nb\nc\nd\ne\n";
        let mut m = Merge::new(base, "a\nB1\nc\nD1\ne\n", "a\nB2\nc\nD2\ne\n");
        assert_eq!(kinds(&m), [Kind::Same, Kind::Conflict, Kind::Same, Kind::Conflict, Kind::Same]);
        let picked = accept(&mut m, base, 1, State::Current);
        assert_eq!(picked, "a\nB1\nc\nd\ne\n");
        let edited = picked.replace("d\n", "mine\nhand\n").replace("a\n", "top\n");
        let ranges = m.sync(&edited);
        assert_eq!(ranges, [0..1, 1..2, 2..3, 3..5, 5..6]);
        assert_eq!((m.chunks[1].state, m.chunks[3].state, m.remaining()), (State::Current, State::Edited, 0));
        let both = accept(&mut m, &edited, 3, State::Both(false));
        assert_eq!(both, "top\nB1\nc\nD2\nD1\ne\n");
        assert_eq!(accept(&mut m, &both, 3, State::Base), "top\nB1\nc\nd\ne\n");
        m.sync(&picked.replace("B1\n", "b\n"));
        assert_eq!((m.chunks[1].state, m.remaining()), (State::Base, 2));
        let ranges = m.sync(base);
        let (rows, pads) = m.align(&ranges, false);
        assert_eq!((rows.len(), pads), (5, vec![]));
    }

    #[test]
    fn aligns_panes_with_filler_rows() {
        let mut m = Merge::new("a\nb\nc\n", "a\nx\ny\nc\n", "a\nz\nc\nw\n");
        let ranges = m.sync(&m.render());
        assert_eq!(ranges, [0..1, 1..2, 2..3, 3..4]);
        let (rows, pads) = m.align(&ranges, true);
        assert_eq!(pads, vec![(2, 1)]);
        let lines: Vec<[Option<usize>; 4]> = rows.iter().map(|r| r.lines).collect();
        assert_eq!(lines, vec![
            [Some(0), Some(0), Some(0), Some(0)],
            [Some(1), Some(1), Some(1), Some(1)],
            [None, Some(2), None, None],
            [Some(2), Some(3), Some(2), Some(2)],
            [Some(3), None, None, Some(3)],
        ]);
        let (rows, pads) = m.align(&ranges, false);
        assert_eq!((rows.len(), pads, rows[2].lines), (5, vec![(2, 1)], [None, Some(2), None, None]));
        assert_eq!(rows.iter().map(|r| r.chunk).collect::<Vec<_>>(), [0, 1, 1, 2, 3]);
    }

    #[test]
    fn handles_adjacent_conflicts_missing_final_newlines_crlf_and_added_files() {
        let mut m = Merge::new("1\n2\n3\n", "1x\n2\n3x\n", "1y\n2\n3y\n");
        assert_eq!(kinds(&m), [Kind::Conflict, Kind::Same, Kind::Conflict]);
        let first = accept(&mut m, "1\n2\n3\n", 0, State::Incoming);
        assert_eq!(accept(&mut m, &first, 2, State::Current), "1y\n2\n3x\n");
        assert_eq!(Merge::new("1\n2\n", "1x\n2x\n", "1y\n2y\n").chunks.len(), 1);

        let base = "a\nb";
        let mut m = Merge::new(base, "a\nB", "a\nC");
        assert_eq!(kinds(&m), [Kind::Same, Kind::Conflict]);
        assert_eq!(accept(&mut m, base, 1, State::Both(true)), "a\nB\nC");
        let mut m = Merge::new("a\nb\n", "a\nb\nc", "x\na\nb\n");
        assert_eq!(m.render(), "x\na\nb\nc");
        let mut m2 = Merge::new("a\nb", "z", "a\nb\nc\n");
        assert_eq!(kinds(&m2), [Kind::Conflict]);
        assert_eq!(accept(&mut m2, "a\nb", 0, State::Both(true)), "z\na\nb\nc\n");
        assert_eq!(m.sync("x\na\nb\nc").len(), 3);

        let mut crlf = Merge::new("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n", "a\r\nC\r\nc\r\n");
        assert_eq!(kinds(&crlf), [Kind::Same, Kind::Conflict, Kind::Same]);
        assert_eq!(accept(&mut crlf, "a\nb\nc\n", 1, State::Both(false)), "a\nC\nB\nc\n");

        let mut added = Merge::new("", "ours\n", "theirs\n");
        assert_eq!((kinds(&added), added.render()), (vec![Kind::Conflict], String::new()));
        let ranges = added.sync("");
        assert_eq!((ranges.len(), ranges[0].clone()), (1, 0..0));
        assert_eq!(accept(&mut added, "", 0, State::Both(true)), "ours\ntheirs\n");
        let mut deleted = Merge::new("a\nb\n", "", "a\nB\n");
        assert_eq!(kinds(&deleted), [Kind::Conflict]);
        assert_eq!(accept(&mut deleted, "a\nb\n", 0, State::Current), "");
        assert_eq!(deleted.chunks[0].state, State::Current);
    }
}
