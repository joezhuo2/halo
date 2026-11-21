fn capture<'a>(pattern: &str, name: &'a str) -> Option<&'a str> {
    match pattern.split_once('*') {
        None => (pattern == name).then_some(""),
        Some((head, tail)) => name.strip_prefix(head)?.strip_suffix(tail),
    }
}

fn wild(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, rest)) => name
            .strip_prefix(head)
            .is_some_and(|name| (0..=name.len()).filter(|i| name.is_char_boundary(*i)).any(|i| wild(rest, &name[i..]))),
    }
}

pub fn nest(files: &[&str], dirname: &str, patterns: &[(&str, &str)]) -> Vec<Option<usize>> {
    let names: Vec<String> = files.iter().map(|f| f.to_lowercase()).collect();
    let dirname = dirname.to_lowercase();
    let mut owner = vec![None; names.len()];
    for (parent, children) in patterns {
        let (parent, children) = (parent.trim().to_lowercase(), children.to_lowercase());
        for p in 0..names.len() {
            let Some(capture) = capture(&parent, &names[p]) else { continue };
            let (base, ext) = names[p].rsplit_once('.').filter(|(b, _)| !b.is_empty()).unwrap_or((&names[p], ""));
            for child in children.split(',').map(str::trim).filter(|c| !c.is_empty()) {
                let child = child.replace("${capture}", capture).replace("${basename}", base).replace("${extname}", ext).replace("${dirname}", &dirname);
                for c in 0..names.len() {
                    if c != p
                        && owner[c].is_none()
                        && wild(&child, &names[c])
                        && !std::iter::successors(Some(p), |a| owner[*a]).any(|a| a == c)
                    {
                        owner[c] = Some(p);
                    }
                }
            }
        }
    }
    owner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(files: &[&str], patterns: &[(&str, &str)]) -> Vec<Option<usize>> {
        nest(files, "src", patterns)
    }

    #[test]
    fn variables_and_stars() {
        assert_eq!(run(&["a.ts", "a.js", "b.js"], &[("*.ts", "${capture}.js")]), [None, Some(0), None]);
        assert_eq!(run(&["x.config.ts", "x.config.d.ts"], &[("*.ts", "${basename}.d.ts")]), [None, Some(0)]);
        assert_eq!(run(&["a.vue", "a.vue.css", "a.css"], &[("a.*", "a.${extname}.css")]), [None, Some(0), None]);
        assert_eq!(run(&["index.ts", "src.ts", "other.ts"], &[("index.ts", "${dirname}.ts")]), [None, Some(0), None]);
        assert_eq!(run(&["README.md", "README.de.md", "readme.txt", "notes.md"], &[("readme.md", "readme.*.md, *.txt")]), [None, Some(0), Some(0), None]);
        assert_eq!(run(&["tsconfig.json", "tsconfig.base.json", "tsconfig.json.bak"], &[("tsconfig.json", "tsconfig.*.json")]), [None, Some(0), None]);
        assert_eq!(run(&[".eslintrc", ".eslintrc.json"], &[(".eslintrc", "${basename}.json")]), [None, Some(0)]);
    }

    #[test]
    fn first_claim_wins_in_pattern_order() {
        let files = ["a.js", "a.ts", "a.map"];
        assert_eq!(run(&files, &[("*.js", "${capture}.map"), ("*.ts", "${capture}.map")]), [None, None, Some(0)]);
        assert_eq!(run(&files, &[("*.ts", "${capture}.map"), ("*.js", "${capture}.map")]), [None, None, Some(1)]);
    }

    #[test]
    fn chains_nest_but_never_under_themselves_or_in_a_cycle() {
        let patterns = [("*.ts", "${capture}.js"), ("*.js", "${capture}.js.map")];
        assert_eq!(run(&["a.ts", "a.js", "a.js.map"], &patterns), [None, Some(0), Some(1)]);
        assert_eq!(run(&["x.a", "x.b"], &[("*.a", "${capture}.b"), ("*.b", "${capture}.a")]), [None, Some(0)]);
        assert_eq!(run(&["a.ts", "b.ts"], &[("*.ts", "*.ts")]), [None, Some(0)]);
        assert_eq!(run(&["a.ts"], &[("*.ts", "*")]), [None]);
    }

    #[test]
    fn matching_ignores_case_and_unmatched_files_stay_top_level() {
        assert_eq!(run(&["Foo.TS", "foo.js"], &[("*.ts", "${capture}.JS")]), [None, Some(0)]);
        assert_eq!(run(&["Cargo.toml", "CARGO.LOCK"], &[("cargo.toml", "Cargo.lock")]), [None, Some(0)]);
        assert_eq!(run(&["main.rs", "lib.rs"], &[("*.ts", "${capture}.js")]), [None, None]);
        assert_eq!(run(&[], &[("*.ts", "${capture}.js")]), Vec::<Option<usize>>::new());
    }
}
