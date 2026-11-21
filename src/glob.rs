pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    braces(pattern).iter().any(|p| matches(&p.chars().collect::<Vec<_>>(), &text))
}

pub(crate) fn excluded(patterns: &[String], relative: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, relative))
}

fn braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else { return vec![pattern.to_owned()] };
    let mut depth = 0;
    let mut cuts = vec![open];
    let mut close = None;
    for (i, c) in pattern[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            ',' if depth == 1 => cuts.push(open + i),
            _ => {}
        }
    }
    let Some(close) = close else { return vec![pattern.to_owned()] };
    cuts.push(close);
    cuts.windows(2).flat_map(|w| braces(&format!("{}{}{}", &pattern[..open], &pattern[w[0] + 1..w[1]], &pattern[close + 1..]))).collect()
}

fn class(set: &[char], c: char) -> bool {
    let (negate, set) = match set.first() {
        Some('!' | '^') => (true, &set[1..]),
        _ => (false, set),
    };
    let mut found = false;
    let mut i = 0;
    while i < set.len() {
        if set.get(i + 1) == Some(&'-') && i + 2 < set.len() {
            found |= (set[i]..=set[i + 2]).contains(&c);
            i += 3;
        } else {
            found |= set[i] == c;
            i += 1;
        }
    }
    found != negate
}

fn matches(p: &[char], t: &[char]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => {
            let rest = &p[2..];
            if rest.first() == Some(&'/') {
                matches(&rest[1..], t) || (0..t.len()).any(|i| t[i] == '/' && matches(&rest[1..], &t[i + 1..]))
            } else {
                (0..=t.len()).any(|i| matches(rest, &t[i..]))
            }
        }
        Some('*') => {
            let slash = t.iter().position(|c| *c == '/').unwrap_or(t.len());
            (0..=slash).any(|i| matches(&p[1..], &t[i..]))
        }
        Some('?') => t.first().is_some_and(|c| *c != '/') && matches(&p[1..], &t[1..]),
        Some('[') => match p.iter().skip(2).position(|c| *c == ']') {
            Some(end) => t.first().is_some_and(|c| *c != '/' && class(&p[1..end + 2], *c)) && matches(&p[end + 3..], &t[1..]),
            None => t.first() == Some(&'[') && matches(&p[1..], &t[1..]),
        },
        Some(c) => t.first() == Some(c) && matches(&p[1..], &t[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_stars_questions_classes_and_braces() {
        assert!(glob_match("*.rs", "main.rs") && !glob_match("*.rs", "src/main.rs") && !glob_match("*.rs", "main.rsx"));
        assert!(glob_match("**/.git", ".git") && glob_match("**/.git", "a/b/.git") && !glob_match("**/.git", "a/.github"));
        assert!(glob_match("src/**", "src/a/b.rs") && glob_match("src/**/*.rs", "src/a/b/c.rs") && glob_match("src/**/*.rs", "src/c.rs"));
        assert!(glob_match("a?c", "abc") && !glob_match("a?c", "a/c") && !glob_match("a?c", "ac"));
        assert!(glob_match("[abc]x", "bx") && !glob_match("[abc]x", "dx") && glob_match("[a-c]x", "cx"));
        assert!(glob_match("[!a-c]x", "dx") && !glob_match("[!a-c]x", "ax") && glob_match("[]x", "[]x"));
        assert!(glob_match("*.{ts,tsx}", "a.tsx") && glob_match("*.{ts,tsx}", "a.ts") && !glob_match("*.{ts,tsx}", "a.js"));
        assert!(glob_match("{a,b}/{c,d}", "b/c") && glob_match("{a,{b,c}}", "c") && glob_match("{open", "{open"));
        assert!(glob_match("**/Thumbs.db", "x/Thumbs.db") && !glob_match("**/Thumbs.db", "thumbs.db"));
        assert!(glob_match("**", "a/b") && glob_match("*", "a") && !glob_match("*", "a/b"));
    }

    #[test]
    fn excluded_checks_every_pattern() {
        let patterns = ["**/.git".to_owned(), "**/*.log".to_owned()];
        assert!(excluded(&patterns, "a/.git") && excluded(&patterns, "x.log") && !excluded(&patterns, "src"));
        assert!(!excluded(&[], "src"));
    }
}
