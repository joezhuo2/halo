use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub left: Range<usize>,
    pub right: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub changed: bool,
}

pub fn split(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

pub fn lines(left: &str, right: &str) -> Vec<Hunk> {
    diff(&split(left), &split(right))
}

pub fn diff<T: Hash + Eq>(a: &[T], b: &[T]) -> Vec<Hunk> {
    let mut ids: HashMap<&T, u32> = HashMap::new();
    let mut id = |item| {
        let next = ids.len() as u32;
        *ids.entry(item).or_insert(next)
    };
    let a: Vec<u32> = a.iter().map(&mut id).collect();
    let b: Vec<u32> = b.iter().map(&mut id).collect();
    let mut matches = Vec::with_capacity(a.len().min(b.len()));
    conquer(&a, &b, (0, a.len()), (0, b.len()), &mut matches);
    let (mut out, mut i, mut j) = (vec![], 0, 0);
    for (x, y) in matches.into_iter().chain([(a.len(), b.len())]) {
        if x > i || y > j {
            out.push(Hunk { left: i..x, right: j..y });
        }
        (i, j) = (x + 1, y + 1);
    }
    out
}

fn conquer(a: &[u32], b: &[u32], (mut a0, mut a1): (usize, usize), (mut b0, mut b1): (usize, usize), out: &mut Vec<(usize, usize)>) {
    while a0 < a1 && b0 < b1 && a[a0] == b[b0] {
        out.push((a0, b0));
        (a0, b0) = (a0 + 1, b0 + 1);
    }
    let mut tail = 0;
    while a1 > a0 && b1 > b0 && a[a1 - 1] == b[b1 - 1] {
        (a1, b1, tail) = (a1 - 1, b1 - 1, tail + 1);
    }
    if a0 < a1 && b0 < b1 {
        let (x, y) = middle(&a[a0..a1], &b[b0..b1]);
        conquer(a, b, (a0, a0 + x), (b0, b0 + y), out);
        conquer(a, b, (a0 + x, a1), (b0 + y, b1), out);
    }
    out.extend((0..tail).map(|k| (a1 + k, b1 + k)));
}

fn middle(a: &[u32], b: &[u32]) -> (usize, usize) {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let delta = n - m;
    let odd = delta & 1 != 0;
    let max = (n + m + 1) / 2;
    let off = max + 1;
    let (mut vf, mut vb) = (vec![0isize; (2 * max + 3) as usize], vec![0isize; (2 * max + 3) as usize]);
    for d in 0..=max {
        for k in (-d..=d).step_by(2) {
            let i = (k + off) as usize;
            let mut x = if k == -d || (k != d && vf[i - 1] < vf[i + 1]) { vf[i + 1] } else { vf[i - 1] + 1 };
            let (start, mut y) = (x, x - k);
            while x < n && y >= 0 && y < m && a[x as usize] == b[y as usize] {
                (x, y) = (x + 1, y + 1);
            }
            vf[i] = x;
            let c = delta - k;
            if odd && c.abs() < d && vf[i] + vb[(c + off) as usize] >= n {
                return (start as usize, (start - k) as usize);
            }
        }
        for k in (-d..=d).step_by(2) {
            let i = (k + off) as usize;
            let mut x = if k == -d || (k != d && vb[i - 1] < vb[i + 1]) { vb[i + 1] } else { vb[i - 1] + 1 };
            let mut y = x - k;
            while x < n && y >= 0 && y < m && a[(n - 1 - x) as usize] == b[(m - 1 - y) as usize] {
                (x, y) = (x + 1, y + 1);
            }
            vb[i] = x;
            let c = delta - k;
            if !odd && c.abs() <= d && vf[(c + off) as usize] + vb[i] >= n {
                return ((n - x) as usize, (m - y) as usize);
            }
        }
    }
    (a.len(), b.len())
}

pub fn tokens(text: &str) -> Vec<&str> {
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
    let mut out = vec![];
    let mut start = 0;
    let mut last = None;
    for (at, c) in text.char_indices() {
        let kind = class(c);
        if at > start && (last != Some(kind) || kind == 2) {
            out.push(&text[start..at]);
            start = at;
        }
        last = Some(kind);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

pub fn intra(left: &str, right: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (a, b) = (tokens(left), tokens(right));
    let cols = |parts: &[&str]| {
        let mut at = vec![0];
        for part in parts {
            at.push(at.last().unwrap_or(&0) + part.chars().count());
        }
        at
    };
    let (ca, cb) = (cols(&a), cols(&b));
    let mut out = (vec![], vec![]);
    for hunk in diff(&a, &b) {
        if !hunk.left.is_empty() {
            out.0.push(ca[hunk.left.start]..ca[hunk.left.end]);
        }
        if !hunk.right.is_empty() {
            out.1.push(cb[hunk.right.start]..cb[hunk.right.end]);
        }
    }
    out
}

pub fn align(hunks: &[Hunk], left_len: usize, right_len: usize) -> Vec<Row> {
    let mut out = Vec::with_capacity(left_len.max(right_len));
    let (mut i, mut j) = (0, 0);
    let end = Hunk { left: left_len..left_len, right: right_len..right_len };
    for hunk in hunks.iter().chain([&end]) {
        while i < hunk.left.start && j < hunk.right.start {
            out.push(Row { left: Some(i), right: Some(j), changed: false });
            (i, j) = (i + 1, j + 1);
        }
        for k in 0..hunk.left.len().max(hunk.right.len()) {
            let left = (k < hunk.left.len()).then_some(hunk.left.start + k);
            let right = (k < hunk.right.len()).then_some(hunk.right.start + k);
            out.push(Row { left, right, changed: true });
        }
        (i, j) = (hunk.left.end, hunk.right.end);
    }
    out
}

pub fn pads(hunks: &[Hunk], left: bool) -> Vec<(usize, u32)> {
    let side = |h: &Hunk| if left { (h.left.clone(), h.right.len()) } else { (h.right.clone(), h.left.len()) };
    hunks.iter().map(side).filter(|(mine, other)| *other > mine.len()).map(|(mine, other)| (mine.end, (other - mine.len()) as u32)).collect()
}

pub fn inline_pads(hunks: &[Hunk]) -> Vec<(usize, u32)> {
    hunks.iter().filter(|h| !h.left.is_empty()).map(|h| (h.right.start, h.left.len() as u32)).collect()
}

pub fn touches(hunk: &Hunk, lines: &Range<usize>) -> bool {
    match hunk.right.is_empty() {
        true => lines.start <= hunk.right.start && hunk.right.start <= lines.end,
        false => lines.start < hunk.right.end && hunk.right.start < lines.end.max(lines.start + 1),
    }
}

pub fn hunk_at(hunks: &[Hunk], line: usize) -> Option<usize> {
    hunks.iter().position(|h| touches(h, &(line..line + 1)))
}

pub fn current(hunks: &[Hunk], line: usize) -> Option<usize> {
    hunk_at(hunks, line).or_else(|| hunks.iter().rposition(|h| h.right.start <= line))
}

pub fn step(hunks: &[Hunk], line: usize, forward: bool) -> Option<usize> {
    if hunks.is_empty() {
        return None;
    }
    Some(match forward {
        true => hunks.iter().position(|h| h.right.start > line).unwrap_or(0),
        false => hunks.iter().rposition(|h| h.right.start < line).unwrap_or(hunks.len() - 1),
    })
}

fn join(lines: Vec<&str>) -> String {
    let mut out = String::new();
    let count = lines.len();
    for (index, line) in lines.into_iter().enumerate() {
        out.push_str(line);
        if index + 1 < count && !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn kept<'a>(out: &mut Vec<&'a str>, left: &[&'a str], right: &[&'a str], hunk: &Hunk, keep_left: &impl Fn(usize) -> bool, keep_right: &impl Fn(usize) -> bool) {
    for k in 0..hunk.left.len().max(hunk.right.len()) {
        let (l, r) = (hunk.left.start + k, hunk.right.start + k);
        if l < hunk.left.end && keep_left(l) {
            out.push(left[l]);
        }
        if r < hunk.right.end && keep_right(r) {
            out.push(right[r]);
        }
    }
}

pub fn hunk_text(left: &[&str], right: &[&str], hunk: &Hunk, keep_left: impl Fn(usize) -> bool, keep_right: impl Fn(usize) -> bool) -> String {
    let mut out = vec![];
    kept(&mut out, left, right, hunk, &keep_left, &keep_right);
    join(out)
}

pub fn splice(left: &[&str], right: &[&str], hunks: &[Hunk], keep_left: impl Fn(usize) -> bool, keep_right: impl Fn(usize) -> bool) -> String {
    let mut out: Vec<&str> = vec![];
    let mut j = 0;
    for hunk in hunks {
        out.extend(&right[j..hunk.right.start]);
        kept(&mut out, left, right, hunk, &keep_left, &keep_right);
        j = hunk.right.end;
    }
    out.extend(&right[j..]);
    join(out)
}

pub fn patch(path: &str, old: &str, new: &str) -> String {
    let (a, b) = (split(old), split(new));
    let hunks = diff(&a, &b);
    if hunks.is_empty() {
        return String::new();
    }
    let mut out = format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n");
    let line = |out: &mut String, mark: char, text: &str| {
        out.push(mark);
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push_str("\n\\ No newline at end of file\n");
        }
    };
    let mut index = 0;
    while index < hunks.len() {
        let mut last = index;
        while last + 1 < hunks.len() && hunks[last + 1].left.start - hunks[last].left.end <= 6 {
            last += 1;
        }
        let (first, end) = (&hunks[index], &hunks[last]);
        let from = first.left.start.saturating_sub(3);
        let to = (end.left.end + 3).min(a.len());
        let (b_from, b_to) = (first.right.start - (first.left.start - from), end.right.end + (to - end.left.end));
        let head = |start: usize, count: usize| if count == 0 { format!("{start},0") } else { format!("{},{count}", start + 1) };
        out.push_str(&format!("@@ -{} +{} @@\n", head(from, to - from), head(b_from, b_to - b_from)));
        let mut i = from;
        for hunk in &hunks[index..=last] {
            a[i..hunk.left.start].iter().for_each(|text| line(&mut out, ' ', text));
            a[hunk.left.clone()].iter().for_each(|text| line(&mut out, '-', text));
            b[hunk.right.clone()].iter().for_each(|text| line(&mut out, '+', text));
            i = hunk.left.end;
        }
        a[i..to].iter().for_each(|text| line(&mut out, ' ', text));
        index = last + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcs(a: &[char], b: &[char]) -> usize {
        let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                table[i][j] = if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
            }
        }
        table[0][0]
    }

    fn hunk(left: Range<usize>, right: Range<usize>) -> Hunk {
        Hunk { left, right }
    }

    #[test]
    fn finds_minimal_hunks_that_rebuild_both_sides() {
        let mut seed = 7u64;
        let mut next = |limit: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % limit
        };
        for round in 0..600 {
            let (size, kinds) = [(14, 4), (60, 3), (40, 8)][round % 3];
            let make = |next: &mut dyn FnMut(u64) -> u64| (0..next(size)).map(|_| (b'a' + next(kinds) as u8) as char).collect::<Vec<_>>();
            let (a, b) = (make(&mut next), make(&mut next));
            let hunks = diff(&a, &b);
            let edits: usize = hunks.iter().map(|h| h.left.len() + h.right.len()).sum();
            assert_eq!(edits, a.len() + b.len() - 2 * lcs(&a, &b), "{a:?} {b:?} {hunks:?}");
            assert!(hunks.windows(2).all(|w| w[0].left.end < w[1].left.start && w[0].right.end < w[1].right.start));
            let (sa, sb): (Vec<String>, Vec<String>) = (a.iter().map(|c| format!("{c}\n")).collect(), b.iter().map(|c| format!("{c}\n")).collect());
            let (ra, rb): (Vec<&str>, Vec<&str>) = (sa.iter().map(String::as_str).collect(), sb.iter().map(String::as_str).collect());
            assert_eq!(splice(&ra, &rb, &hunks, |_| false, |_| true), sb.concat());
            assert_eq!(splice(&ra, &rb, &hunks, |_| true, |_| false), sa.concat());
            let widest: usize = hunks.iter().map(|h| h.left.len().max(h.right.len())).sum();
            assert_eq!(align(&hunks, a.len(), b.len()).len(), lcs(&a, &b) + widest);
        }
    }

    #[test]
    fn diffs_lines_with_their_endings() {
        assert!(lines("a\nb\n", "a\nb\n").is_empty());
        assert_eq!(lines("a\nb\nc\n", "a\nc\n"), vec![hunk(1..2, 1..1)]);
        assert_eq!(lines("a\nc\n", "a\nb\nc\n"), vec![hunk(1..1, 1..2)]);
        assert_eq!(lines("a\nb\nc\n", "a\nB\nc\nd\n"), vec![hunk(1..2, 1..2), hunk(3..3, 3..4)]);
        assert_eq!(lines("a\nb", "a\nb\n"), vec![hunk(1..2, 1..2)]);
        assert_eq!(lines("", "x\n"), vec![hunk(0..0, 0..1)]);
        assert_eq!(lines("x\ny\n", ""), vec![hunk(0..2, 0..0)]);
        let big: String = (0..3000).map(|i| format!("line {i}\n")).collect();
        let edited = big.replace("line 1500\n", "changed\n").replace("line 10\n", "");
        assert_eq!(lines(&big, &edited), vec![hunk(10..11, 10..10), hunk(1500..1501, 1499..1500)]);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn marks_changed_words_inside_a_line() {
        assert_eq!(tokens("let x_1 = foo(a,  b);"), vec!["let", " ", "x_1", " ", "=", " ", "foo", "(", "a", ",", "  ", "b", ")", ";"]);
        assert_eq!(intra("let x = 1;", "let y = 1;"), (vec![4..5], vec![4..5]));
        assert_eq!(intra("call(a)", "call(a, b)"), (vec![], vec![6..9]));
        assert_eq!(intra("é ab", "é cd"), (vec![2..4], vec![2..4]));
        assert_eq!(intra("same", "same"), (vec![], vec![]));
    }

    #[test]
    fn aligns_two_columns_with_filler_rows() {
        let hunks = lines("a\nb\nc\nd\n", "a\nx\ny\nz\nd\ne\n");
        assert_eq!(hunks, vec![hunk(1..3, 1..4), hunk(4..4, 5..6)]);
        let rows = align(&hunks, 5, 7);
        let pairs: Vec<(Option<usize>, Option<usize>)> = rows.iter().map(|r| (r.left, r.right)).collect();
        assert_eq!(pairs, vec![
            (Some(0), Some(0)),
            (Some(1), Some(1)),
            (Some(2), Some(2)),
            (None, Some(3)),
            (Some(3), Some(4)),
            (None, Some(5)),
            (Some(4), Some(6)),
        ]);
        assert_eq!(rows.iter().filter(|r| r.changed).count(), 4);
        assert_eq!(pads(&hunks, true), vec![(3, 1), (4, 1)]);
        assert!(pads(&hunks, false).is_empty());
        let removed = lines("a\nb\nc\n", "a\n");
        assert_eq!(pads(&removed, false), vec![(1, 2)]);
        assert_eq!(inline_pads(&hunks), vec![(1, 2)]);
        assert_eq!(inline_pads(&removed), vec![(1, 2)]);
    }

    #[test]
    fn steps_through_changes_in_order_and_wraps() {
        let hunks = vec![hunk(2..3, 2..3), hunk(6..6, 7..9), hunk(12..14, 15..15)];
        assert_eq!(step(&hunks, 0, true), Some(0));
        assert_eq!(step(&hunks, 2, true), Some(1));
        assert_eq!(step(&hunks, 8, true), Some(2));
        assert_eq!(step(&hunks, 15, true), Some(0));
        assert_eq!(step(&hunks, 15, false), Some(1));
        assert_eq!(step(&hunks, 8, false), Some(1));
        assert_eq!(step(&hunks, 7, false), Some(0));
        assert_eq!(step(&hunks, 1, false), Some(2));
        assert_eq!(step(&[], 1, false), None);
        assert_eq!(hunk_at(&hunks, 8), Some(1));
        assert_eq!(hunk_at(&hunks, 15), Some(2));
        assert_eq!(hunk_at(&hunks, 4), None);
        assert_eq!(current(&hunks, 4), Some(0));
        assert_eq!(current(&hunks, 1), None);
        assert!(touches(&hunks[2], &(14..15)) && !touches(&hunks[2], &(16..18)));
        assert!(touches(&hunks[1], &(3..8)) && !touches(&hunks[1], &(9..12)));
    }

    #[test]
    fn rebuilds_selected_lines_and_hunk_text() {
        let (old, new) = ("a\nb\nc\nd\n", "a\nB\nC\nd\ne");
        let (left, right) = (split(old), split(new));
        let hunks = diff(&left, &right);
        assert_eq!(hunks, vec![hunk(1..3, 1..3), hunk(4..4, 4..5)]);
        assert_eq!(splice(&left, &right, &hunks, |l| l != 1, |r| r == 1), "a\nB\nc\nd\n");
        assert_eq!(splice(&left, &right, &hunks, |_| false, |_| true), new);
        assert_eq!(hunk_text(&left, &right, &hunks[0], |_| true, |_| false), "b\nc\n");
        assert_eq!(hunk_text(&split("a\nc"), &split("a\nb\n"), &hunk(1..2, 1..2), |_| true, |_| true), "c\nb\n");
    }

    #[test]
    fn writes_unified_patches_with_context_and_missing_newlines() {
        assert_eq!(patch("f.txt", "a\n", "a\n"), "");
        let old: String = (1..=20).map(|i| format!("{i}\n")).collect();
        let edit = |i: usize| match i {
            2 => "two\n".to_owned(),
            6 => String::new(),
            18 => "18\n18b\n".to_owned(),
            _ => format!("{i}\n"),
        };
        let new: String = (1..=20).map(edit).collect();
        assert_eq!(
            patch("src/f.txt", &old, &new),
            "diff --git a/src/f.txt b/src/f.txt\n--- a/src/f.txt\n+++ b/src/f.txt\n\
             @@ -1,9 +1,8 @@\n 1\n-2\n+two\n 3\n 4\n 5\n-6\n 7\n 8\n 9\n\
             @@ -16,5 +15,6 @@\n 16\n 17\n 18\n+18b\n 19\n 20\n"
        );
        assert_eq!(
            patch("f", "a\nb", "a\nc\n"),
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n"
        );
        assert_eq!(patch("f", "", "x\n"), "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -0,0 +1,1 @@\n+x\n");
    }
}
