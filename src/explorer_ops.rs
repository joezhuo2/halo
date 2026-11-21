use std::path::{Path, PathBuf};

#[derive(Default, Debug, PartialEq)]
pub struct Selection {
    pub paths: Vec<PathBuf>,
    pub anchor: Option<PathBuf>,
    pub cursor: Option<PathBuf>,
}

impl Selection {
    pub fn set(&mut self, path: PathBuf) {
        self.paths = vec![path.clone()];
        self.anchor = Some(path.clone());
        self.cursor = Some(path);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn toggle(&mut self, path: PathBuf) {
        match self.paths.iter().position(|p| *p == path) {
            Some(i) => drop(self.paths.remove(i)),
            None => self.paths.push(path.clone()),
        }
        self.anchor = Some(path.clone());
        self.cursor = Some(path);
    }

    pub fn range(&mut self, order: &[PathBuf], to: PathBuf) {
        let at = |p: &PathBuf| order.iter().position(|o| o == p);
        let (Some(from), Some(end)) = (self.anchor.as_ref().and_then(at), at(&to)) else { return self.set(to) };
        self.paths = order[from.min(end)..=from.max(end)].to_vec();
        self.cursor = Some(to);
    }

    pub fn all(&mut self, order: &[PathBuf]) {
        self.paths = order.to_vec();
        self.anchor = order.first().cloned();
        self.cursor = order.last().cloned();
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    pub fn is_multiple(&self) -> bool {
        self.paths.len() > 1
    }
}

pub fn top_level(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().filter(|p| !paths.iter().any(|o| o != *p && p.starts_with(o))).cloned().collect()
}

pub fn unique_name(dir: &Path, name: &str, is_dir: bool, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let path = Path::new(name);
    let (stem, ext) = match path.extension().filter(|_| !is_dir) {
        Some(ext) => (path.file_stem().unwrap_or_default().to_string_lossy().into_owned(), format!(".{}", ext.to_string_lossy())),
        None => (name.to_owned(), String::new()),
    };
    (1..)
        .map(|n| if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") })
        .map(|candidate| dir.join(candidate))
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub from: PathBuf,
    pub to: PathBuf,
    pub replace: bool,
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

pub fn plan(sources: &[PathBuf], dir: &Path, cut: bool) -> (Vec<Job>, Vec<String>) {
    let mut jobs: Vec<Job> = vec![];
    let mut errors = vec![];
    for from in top_level(sources) {
        let Some(name) = from.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if !exists(&from) {
            errors.push(format!("\"{name}\" no longer exists."));
        } else if dir.starts_with(&from) && (cut || from.is_dir()) {
            let verb = if cut { "move" } else { "copy" };
            errors.push(format!("Cannot {verb} \"{name}\" into itself."));
        } else if cut {
            let to = dir.join(&name);
            if to == from {
                continue;
            }
            if jobs.iter().any(|j| j.to == to) {
                errors.push(format!("Another item named \"{name}\" is already being moved here."));
                continue;
            }
            jobs.push(Job { replace: exists(&to), from, to });
        } else {
            let to = unique_name(dir, &name, from.is_dir(), &|p| exists(p) || jobs.iter().any(|j| j.to == p));
            jobs.push(Job { from, to, replace: false });
        }
    }
    (jobs, errors)
}

pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.starts_with(from) {
        return Err(std::io::Error::other("a folder cannot be copied into itself"));
    }
    if !from.symlink_metadata()?.is_dir() {
        return std::fs::copy(from, to).map(drop);
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_tree(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

fn remove(path: &Path) -> std::io::Result<()> {
    let meta = path.symlink_metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        const DIRECTORY: u32 = 0x10;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return if meta.file_attributes() & DIRECTORY != 0 { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
        }
    }
    if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

fn move_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_tree(from, to)?;
    remove(from)
}

#[derive(Default, Debug)]
pub struct Done {
    pub cut: bool,
    pub done: Vec<(PathBuf, PathBuf)>,
    pub removed: Vec<PathBuf>,
    pub remaining: Vec<PathBuf>,
    pub errors: Vec<String>,
}

pub fn trash(paths: Vec<PathBuf>) -> Done {
    let (mut unavailable, recyclable): (Vec<_>, Vec<_>) =
        paths.into_iter().partition(|path| cfg!(windows) && crate::wsl::parse(path).is_some());
    if !recyclable.is_empty() && trash::delete_all(&recyclable).is_ok() {
        let mut out = trash_failed(unavailable);
        out.removed.extend(recyclable);
        return out;
    }
    unavailable.extend(recyclable);
    trash_failed(unavailable)
}

pub fn trash_failed(paths: Vec<PathBuf>) -> Done {
    let mut out = Done::default();
    for path in paths {
        match path.symlink_metadata() {
            Ok(_) => out.remaining.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => out.removed.push(path),
            Err(e) => out.errors.push(format!("Could not check {} after moving it to the trash failed: {e}", path.display())),
        }
    }
    out
}

pub fn delete_permanently(paths: Vec<PathBuf>) -> Done {
    let mut out = Done::default();
    for path in paths {
        match remove(&path) {
            Ok(()) => out.removed.push(path),
            Err(e) => out.errors.push(format!("Could not permanently delete {}: {e}", path.display())),
        }
    }
    out
}

pub fn run(jobs: Vec<Job>, cut: bool) -> Done {
    let mut out = Done { cut, ..Done::default() };
    for job in jobs {
        let name = job.from.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let result = (|| {
            if job.replace {
                trash::delete(&job.to).map_err(std::io::Error::other)?;
            }
            if cut { move_tree(&job.from, &job.to) } else { copy_tree(&job.from, &job.to) }
        })();
        match result {
            Ok(()) => out.done.push((job.from, job.to)),
            Err(error) => out.errors.push(format!("Could not {} \"{name}\": {error}", if cut { "move" } else { "copy" })),
        }
    }
    out
}

pub fn fuzzy(query: &str, name: &str) -> Option<Vec<usize>> {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let query: Vec<char> = query.chars().map(lower).collect();
    let name: Vec<char> = name.chars().map(lower).collect();
    if query.is_empty() {
        return None;
    }
    if let Some(start) = name.windows(query.len()).position(|w| w == query.as_slice()) {
        return Some((start..start + query.len()).collect());
    }
    let mut hits = Vec::with_capacity(query.len());
    let mut wanted = query.iter();
    let mut next = wanted.next();
    for (i, c) in name.iter().enumerate() {
        if Some(c) == next {
            hits.push(i);
            next = wanted.next();
        }
    }
    next.is_none().then_some(hits)
}

pub fn filter_rows(query: &str, rows: &[(usize, &str)]) -> (Vec<Option<Vec<usize>>>, Vec<bool>) {
    let hits: Vec<Option<Vec<usize>>> = rows.iter().map(|(_, name)| fuzzy(query, name)).collect();
    let mut keep = vec![false; rows.len()];
    let mut below = vec![false; rows.iter().map(|r| r.0).max().map_or(0, |d| d + 1)];
    for i in (0..rows.len()).rev() {
        let depth = rows[i].0;
        keep[i] = hits[i].is_some() || below[depth];
        below[depth] = false;
        if keep[i] {
            below[..depth].fill(true);
        }
    }
    (hits, keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-explorer-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn selection_toggles_and_extends_ranges() {
        let order = paths(&["a", "b", "c", "d", "e"]);
        let mut sel = Selection::default();
        sel.set("b".into());
        sel.toggle("d".into());
        assert_eq!(sel.paths, paths(&["b", "d"]));
        sel.toggle("b".into());
        assert_eq!(sel.paths, paths(&["d"]));
        assert!(sel.contains(Path::new("d")) && !sel.contains(Path::new("b")));
        sel.set("b".into());
        sel.range(&order, "d".into());
        assert_eq!(sel.paths, paths(&["b", "c", "d"]));
        sel.range(&order, "a".into());
        assert_eq!(sel.paths, paths(&["a", "b"]));
        assert_eq!(sel.anchor, Some("b".into()));
        assert_eq!(sel.cursor, Some("a".into()));
        sel.all(&order);
        assert_eq!(sel.paths.len(), 5);
        let mut none = Selection::default();
        none.range(&order, "c".into());
        assert_eq!(none.paths, paths(&["c"]));
    }

    #[test]
    fn permanently_deletes_files_and_nonempty_folders_and_reports_failures() {
        let dir = temp("permanent");
        let (file, folder, missing) = (dir.join("file"), dir.join("folder"), dir.join("missing"));
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("child"), "data").unwrap();
        std::fs::write(&file, "data").unwrap();
        let done = delete_permanently(vec![file.clone(), missing, folder.clone()]);
        assert_eq!(done.removed, [file, folder]);
        assert_eq!(done.errors.len(), 1);
        assert!(done.errors[0].contains("missing"));
        assert!(done.remaining.is_empty() && dir.read_dir().unwrap().next().is_none());
        std::fs::remove_dir(dir).unwrap();
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn permanent_delete_leaves_link_targets_intact() {
        let dir = temp("permanent-link");
        let (target, link) = (dir.join("target"), dir.join("link"));
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("child"), "keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd").args(["/d", "/c", "mklink", "/J"]).arg(&link).arg(&target).output().unwrap().status.success());
        let done = delete_permanently(vec![link.clone()]);
        assert!(done.errors.is_empty());
        assert_eq!(done.removed, std::slice::from_ref(&link));
        assert!(link.symlink_metadata().is_err() && target.join("child").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nested_selections_keep_only_the_outer_item() {
        let all = paths(&["x/a", "x/a/b", "x/c", "x/ab"]);
        assert_eq!(top_level(&all), paths(&["x/a", "x/c", "x/ab"]));
    }

    #[test]
    fn names_get_copy_suffixes() {
        let dir = temp("names");
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::write(dir.join(".env"), "").unwrap();
        std::fs::create_dir(dir.join("v1.2")).unwrap();
        let taken = |p: &Path| exists(p);
        assert_eq!(unique_name(&dir, "new.txt", false, &taken), dir.join("new.txt"));
        assert_eq!(unique_name(&dir, "a.txt", false, &taken), dir.join("a copy.txt"));
        std::fs::write(dir.join("a copy.txt"), "").unwrap();
        assert_eq!(unique_name(&dir, "a.txt", false, &taken), dir.join("a copy 2.txt"));
        assert_eq!(unique_name(&dir, ".env", false, &taken), dir.join(".env copy"));
        assert_eq!(unique_name(&dir, "v1.2", true, &taken), dir.join("v1.2 copy"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_folders_and_numbers_clashes() {
        let dir = temp("copy");
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("src/deep/x.rs"), "x").unwrap();
        std::fs::write(dir.join("src/a.rs"), "a").unwrap();
        std::fs::create_dir(dir.join("out")).unwrap();

        let (jobs, errors) = plan(&[dir.join("src"), dir.join("src/a.rs")], &dir.join("out"), false);
        assert!(errors.is_empty() && jobs.len() == 1);
        let done = run(jobs, false);
        assert!(done.errors.is_empty() && done.done.len() == 1);
        assert_eq!(std::fs::read_to_string(dir.join("out/src/deep/x.rs")).unwrap(), "x");
        assert!(dir.join("src/a.rs").is_file());

        let (jobs, _) = plan(&[dir.join("src")], &dir.join("out"), false);
        assert_eq!(jobs[0].to, dir.join("out/src copy"));
        run(jobs, false);
        let (jobs, _) = plan(&[dir.join("src"), dir.join("out/src")], &dir.join("out"), false);
        assert_eq!(jobs.iter().map(|j| j.to.clone()).collect::<Vec<_>>(), [dir.join("out/src copy 2"), dir.join("out/src copy 3")]);

        let (jobs, _) = plan(&[dir.join("src/a.rs")], &dir.join("src"), false);
        assert_eq!(jobs[0].to, dir.join("src/a copy.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moves_refuse_into_itself_and_flag_clashes() {
        let dir = temp("move");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir(dir.join("c")).unwrap();
        std::fs::write(dir.join("a/f.txt"), "from a").unwrap();
        std::fs::write(dir.join("c/f.txt"), "in c").unwrap();
        std::fs::write(dir.join("g.txt"), "g").unwrap();

        let (jobs, errors) = plan(&[dir.join("a")], &dir.join("a/b"), true);
        assert!(jobs.is_empty() && errors.len() == 1 && errors[0].contains("itself"));
        let (jobs, errors) = plan(&[dir.join("a")], &dir.join("a"), true);
        assert!(jobs.is_empty() && errors.len() == 1);
        let (jobs, errors) = plan(&[dir.join("a/b")], &dir.join("a"), true);
        assert!(jobs.is_empty() && errors.is_empty());
        let (jobs, _) = plan(&[dir.join("a/f.txt")], &dir.join("c"), true);
        assert_eq!(jobs, [Job { from: dir.join("a/f.txt"), to: dir.join("c/f.txt"), replace: true }]);
        let (jobs, _) = plan(&[dir.join("g.txt"), dir.join("a/b")], &dir.join("c"), true);
        assert!(jobs.iter().all(|j| !j.replace));
        let done = run(jobs, true);
        assert!(done.errors.is_empty() && done.done.len() == 2);
        assert!(!dir.join("g.txt").exists() && dir.join("c/g.txt").is_file() && dir.join("c/b").is_dir());
        assert!(copy_tree(&dir.join("c"), &dir.join("c/b")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fuzzy_prefers_substrings_and_needs_every_letter() {
        assert_eq!(fuzzy("main", "src_main.rs"), Some(vec![4, 5, 6, 7]));
        assert_eq!(fuzzy("MRS", "main.rs"), Some(vec![0, 5, 6]));
        assert_eq!(fuzzy("xyz", "main.rs"), None);
        assert_eq!(fuzzy("", "main.rs"), None);
        assert_eq!(fuzzy("sm", "main.rs"), None);
    }

    #[test]
    fn filter_keeps_the_parents_of_matches() {
        let rows = [(0, "src"), (1, "ui"), (2, "mod.rs"), (1, "lib.rs"), (0, "docs"), (1, "guide.md"), (0, "README.md")];
        let (hits, keep) = filter_rows("lib", &rows);
        assert_eq!(keep, [true, false, false, true, false, false, false]);
        assert_eq!(hits.iter().flatten().count(), 1);
        let (_, keep) = filter_rows("guide", &rows);
        assert_eq!(keep, [false, false, false, false, true, true, false]);
        let (_, keep) = filter_rows("md", &rows);
        assert_eq!(keep, [true, true, true, false, true, true, true]);
        let (_, keep) = filter_rows("rs", &rows);
        assert_eq!(keep, [true, true, true, true, false, false, false]);
        let (_, keep) = filter_rows("", &rows);
        assert!(keep.iter().all(|k| !k));
    }
}
