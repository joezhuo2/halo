use crate::command::Command;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Bookmark {
    pub line: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

pub type Files = BTreeMap<PathBuf, Vec<Bookmark>>;

#[derive(Default)]
pub struct Bookmarks {
    pub files: Files,
    pub asking: Option<(PathBuf, usize, String)>,
    folder: Option<PathBuf>,
}

impl Bookmarks {
    pub fn lines(&self, path: &Path) -> Vec<usize> {
        self.files.get(path).map(|list| list.iter().map(|b| b.line).collect()).unwrap_or_default()
    }

    pub fn label_of(&self, path: &Path, line: usize) -> String {
        self.files.get(path).and_then(|list| list.iter().find(|b| b.line == line)).map(|b| b.label.clone()).unwrap_or_default()
    }

    pub fn toggle(&mut self, path: &Path, line: usize) {
        let list = self.files.entry(path.to_path_buf()).or_default();
        match list.iter().position(|b| b.line == line) {
            Some(at) => drop(list.remove(at)),
            None => list.push(Bookmark { line, label: String::new() }),
        }
        self.tidy();
    }

    pub fn label(&mut self, path: &Path, line: usize, label: String) {
        let list = self.files.entry(path.to_path_buf()).or_default();
        match list.iter_mut().find(|b| b.line == line) {
            Some(b) => b.label = label,
            None => list.push(Bookmark { line, label }),
        }
        self.tidy();
    }

    pub fn shift(&mut self, path: &Path, edited: usize, delta: isize) {
        let Some(list) = self.files.get_mut(path) else { return };
        for b in list.iter_mut().filter(|b| b.line > edited) {
            b.line = (b.line as isize + delta).max(edited as isize) as usize;
        }
        self.tidy();
    }

    pub fn path_changed(&mut self, old: &Path, new: Option<&Path>) {
        let moved: Vec<PathBuf> = self.files.keys().filter(|p| p.starts_with(old)).cloned().collect();
        for path in moved {
            let list = self.files.remove(&path).unwrap_or_default();
            if let (Some(new), Ok(rest)) = (new, path.strip_prefix(old)) {
                self.files.insert(if rest.as_os_str().is_empty() { new.to_path_buf() } else { new.join(rest) }, list);
            }
        }
    }

    fn tidy(&mut self) {
        for list in self.files.values_mut() {
            list.sort_by_key(|b| b.line);
            list.dedup_by_key(|b| b.line);
        }
        self.files.retain(|_, list| !list.is_empty());
    }

    pub fn next(&self, path: Option<&Path>, line: usize, forward: bool) -> Option<(PathBuf, usize)> {
        let all: Vec<(&Path, usize)> = self.files.iter().flat_map(|(p, list)| list.iter().map(move |b| (p.as_path(), b.line))).collect();
        let here = (path, line);
        let found = match forward {
            true => all.iter().find(|b| (Some(b.0), b.1) > here).or(all.first()),
            false => all.iter().rev().find(|b| (Some(b.0), b.1) < here).or(all.last()),
        };
        found.map(|b| (b.0.to_path_buf(), b.1))
    }

    pub fn choices(&self) -> Vec<crate::search::Choice> {
        let item = |path: &PathBuf, b: &Bookmark| {
            let place = format!("{}:{}", crate::history::name(path), b.line + 1);
            let label = if b.label.is_empty() { place } else { format!("{} ({place})", b.label) };
            (label, path.to_string_lossy().into_owned(), Command::GoToBookmark(path.clone(), b.line))
        };
        self.files.iter().flat_map(|(path, list)| list.iter().map(move |b| item(path, b))).collect()
    }

    pub fn store(&self, saved: &mut BTreeMap<PathBuf, Files>) {
        let Some(folder) = &self.folder else { return };
        match self.files.is_empty() {
            true => drop(saved.remove(folder)),
            false => drop(saved.insert(folder.clone(), self.files.clone())),
        }
    }

    pub fn restore(&mut self, saved: &BTreeMap<PathBuf, Files>, folder: Option<PathBuf>) {
        self.files = folder.as_ref().and_then(|f| saved.get(f)).cloned().unwrap_or_default();
        self.folder = folder;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_toggle_shift_with_edits_and_cycle_across_files() {
        let (a, b) = (PathBuf::from("a.rs"), PathBuf::from("b.rs"));
        let mut marks = Bookmarks::default();
        marks.toggle(&a, 10);
        marks.toggle(&a, 2);
        marks.label(&b, 5, "entry".into());
        assert_eq!(marks.lines(&a), [2, 10]);
        assert_eq!(marks.next(Some(&a), 2, true), Some((a.clone(), 10)));
        assert_eq!(marks.next(Some(&a), 10, true), Some((b.clone(), 5)));
        assert_eq!(marks.next(Some(&b), 5, true), Some((a.clone(), 2)));
        assert_eq!(marks.next(Some(&a), 2, false), Some((b.clone(), 5)));
        assert_eq!(marks.next(None, 0, true), Some((a.clone(), 2)));
        marks.shift(&a, 3, 4);
        assert_eq!(marks.lines(&a), [2, 14]);
        marks.shift(&a, 1, -20);
        assert_eq!(marks.lines(&a), [1]);
        marks.toggle(&a, 1);
        assert!(!marks.files.contains_key(&a));
        assert_eq!(marks.choices()[0].0, "entry (b.rs:6)");
        marks.path_changed(&b, Some(&a));
        assert_eq!(marks.label_of(&a, 5), "entry");
        let mut saved = BTreeMap::new();
        marks.restore(&saved, Some(PathBuf::from("w")));
        assert!(marks.files.is_empty());
        marks.toggle(&a, 0);
        marks.store(&mut saved);
        let mut other = Bookmarks::default();
        other.restore(&saved, Some(PathBuf::from("w")));
        assert_eq!(other.lines(&a), [0]);
    }
}
