use crate::command::Command;
use crate::editor::OpenTab;
use crate::explorer_ops::{self as ops, Done, Job, Selection};
use crate::git::{FileState, Git};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Modifiers, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui,
    UiBuilder, pos2, text::CCursor, vec2,
};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

mod nesting;
mod panels;
#[cfg(test)]
mod tests;

const ROW: f32 = 22.0;
const INDENT: f32 = 12.0;
const FOLDER_TIME: f64 = (ui::ANIM * 1.4) as f64;
const OPEN_ROWS: f32 = 8.0;
const HOVER_EXPAND: f64 = 0.7;

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

#[derive(Clone, Default)]
struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    is_dir: bool,
    editing: bool,
    chain: Vec<PathBuf>,
    hits: Vec<usize>,
    kids: Vec<PathBuf>,
}

impl Row {
    fn holds(&self, path: &Path) -> bool {
        self.path == path || self.chain.iter().any(|p| p == path)
    }
}

struct FolderAnimation {
    dir: Option<PathBuf>,
    start: f64,
    outgoing: Vec<Row>,
    spans: Vec<(Range<usize>, bool)>,
}

impl FolderAnimation {
    fn progress(&self, now: f64) -> f32 {
        let t = ((now - self.start) / FOLDER_TIME).clamp(0.0, 1.0) as f32;
        if self.outgoing.is_empty() { t } else { 1.0 - t }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    NewFile,
    NewFolder,
    Rename,
}

struct Edit {
    kind: EditKind,
    target: PathBuf,
    text: String,
    focus: bool,
}

struct Filter {
    open: bool,
    text: String,
    hide: bool,
    focus: bool,
    matches: usize,
}

impl Filter {
    fn active(&self) -> bool {
        self.open && !self.text.trim().is_empty()
    }
}

enum Action {
    Toggle(PathBuf),
    Expand(PathBuf),
    Open(PathBuf),
    Select(PathBuf),
    Pick(PathBuf),
    Extend(PathBuf),
    SelectAll,
    New(EditKind, PathBuf),
    Rename(PathBuf),
    Delete(Vec<PathBuf>),
    Clip(bool),
    Paste,
    Drop(Vec<PathBuf>, PathBuf),
    Import(Vec<PathBuf>, PathBuf),
    Match(bool),
    CloseFilter,
    Copy(String),
    Reveal(PathBuf),
    Compare(Command),
    Refresh,
    CollapseAll,
}

pub struct Explorer {
    root: Option<PathBuf>,
    roots: Vec<(PathBuf, String)>,
    title: Option<String>,
    children: HashMap<PathBuf, Vec<Entry>>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    dirty: bool,
    sel: Selection,
    pub compare: Option<PathBuf>,
    pub problems: std::collections::BTreeMap<String, u8>,
    last_active: Option<PathBuf>,
    scroll_to: Option<PathBuf>,
    edit: Option<Edit>,
    confirm_delete: Vec<PathBuf>,
    confirm_permanent: std::collections::VecDeque<Vec<PathBuf>>,
    confirm_drop: Option<(Vec<PathBuf>, PathBuf)>,
    clash: Option<Vec<Job>>,
    clipboard: Option<(Vec<PathBuf>, bool)>,
    work: Vec<Receiver<Done>>,
    drag: Option<Vec<PathBuf>>,
    hover_dir: Option<(PathBuf, f64)>,
    dropped: Option<(Vec<PathBuf>, Pos2)>,
    filter: Filter,
    focus_tree: bool,
    open_editors: bool,
    open_focus: bool,
    open_cursor: usize,
    focus_open: bool,
    compact: bool,
    exclude: Vec<String>,
    nest: bool,
    nest_expand: bool,
    patterns: std::collections::BTreeMap<String, String>,
    flipped: HashSet<PathBuf>,
    confirm_drag: bool,
    confirm: bool,
    sort: String,
    error: Option<String>,
    focused: bool,
    collapsed: bool,
    animation: Option<FolderAnimation>,
}

fn sort_entries(entries: &mut [Entry], order: &str) {
    let modified = |e: &Entry| std::cmp::Reverse(std::fs::metadata(&e.path).and_then(|m| m.modified()).ok());
    let ext = |e: &Entry| Path::new(&e.name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    match order {
        "mixed" => entries.sort_by_cached_key(|e| e.name.to_lowercase()),
        "filesFirst" => entries.sort_by_cached_key(|e| (e.is_dir, e.name.to_lowercase())),
        "type" => entries.sort_by_cached_key(|e| (!e.is_dir, ext(e), e.name.to_lowercase())),
        "modified" => entries.sort_by_cached_key(|e| (!e.is_dir, modified(e), e.name.to_lowercase())),
        _ => entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase())),
    }
}

fn validate(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("A name is required.".into());
    }
    if name == "." || name == ".." || name.chars().any(|c| "/\\:*?\"<>|".contains(c)) {
        return Err(format!("\"{name}\" is not a valid name."));
    }
    Ok(())
}

pub fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "ino" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "cs"
        | "rb" | "php" | "swift" | "kt" | "lua" | "sh" | "ps1" | "bat" | "html" | "css" | "scss" | "sql" | "vue" => {
            icons::FILE_CODE
        }
        "md" | "txt" | "rst" | "log" => icons::FILE_TEXT,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" => icons::FILE_IMAGE,
        "json" | "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "xml" | "env" => icons::GEAR_SIX,
        "lock" => icons::LOCK_SIMPLE,
        "zip" | "tar" | "gz" | "7z" | "rar" => icons::FILE_ZIP,
        "pdf" => icons::FILE_PDF,
        _ => icons::FILE,
    }
}

pub fn state_color(theme: &Theme, state: Option<FileState>) -> Color32 {
    match state {
        Some(FileState::Added) => theme.added,
        Some(FileState::Modified) => theme.modified,
        Some(FileState::Deleted | FileState::Conflicted) => theme.deleted,
        Some(FileState::Ignored) => theme.text_dim,
        None => theme.text,
    }
}

pub fn name_color(theme: &Theme, severity: Option<u8>, state: Option<FileState>) -> Color32 {
    match severity {
        Some(1) => theme.error,
        Some(_) => theme.warning,
        None => state_color(theme, state),
    }
}

pub fn state_badge(state: Option<FileState>) -> &'static str {
    match state {
        Some(FileState::Added) => "U",
        Some(FileState::Modified) => "M",
        Some(FileState::Deleted) => "D",
        Some(FileState::Conflicted) => "C",
        _ => "",
    }
}

fn reveal(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

fn count_text(paths: &[PathBuf]) -> String {
    match paths {
        [one] => format!("\"{}\"", name_of(one)),
        many => format!("{} items", many.len()),
    }
}

impl Explorer {
    pub fn new() -> Self {
        Self {
            root: None,
            roots: vec![],
            title: None,
            children: HashMap::new(),
            expanded: HashSet::new(),
            rows: vec![],
            dirty: true,
            sel: Selection::default(),
            compare: None,
            problems: Default::default(),
            last_active: None,
            scroll_to: None,
            edit: None,
            confirm_delete: vec![],
            confirm_permanent: Default::default(),
            confirm_drop: None,
            clash: None,
            clipboard: None,
            work: vec![],
            drag: None,
            hover_dir: None,
            dropped: None,
            filter: Filter { open: false, text: String::new(), hide: true, focus: false, matches: 0 },
            focus_tree: false,
            open_editors: true,
            open_focus: false,
            open_cursor: 0,
            focus_open: false,
            compact: false,
            exclude: vec![],
            nest: false,
            nest_expand: true,
            patterns: Default::default(),
            flipped: HashSet::new(),
            confirm_drag: true,
            confirm: true,
            sort: String::new(),
            error: None,
            focused: false,
            collapsed: false,
            animation: None,
        }
    }

    pub fn set_roots(&mut self, roots: Vec<PathBuf>, names: Vec<String>, title: Option<String>) {
        if title.is_none() || !roots.iter().any(|r| self.roots.iter().any(|(p, _)| p == r)) {
            *self = Self::new();
        }
        let added: Vec<PathBuf> = roots.iter().filter(|r| self.title.is_none() || !self.roots.iter().any(|(p, _)| p == *r)).cloned().collect();
        self.root = roots.first().filter(|_| title.is_none()).cloned();
        self.roots = roots.into_iter().zip(names).collect();
        if title.is_some() {
            added.iter().for_each(|r| self.expand(r));
        }
        self.title = title;
        self.dirty = true;
    }

    fn root_of(&self, path: &Path) -> Option<&PathBuf> {
        self.roots.iter().map(|(r, _)| r).filter(|r| path.starts_with(r)).max_by_key(|r| r.components().count())
    }

    fn is_root(&self, path: &Path) -> bool {
        self.title.is_some() && self.roots.iter().any(|(r, _)| r == path)
    }

    fn read_dir(&self, dir: &Path) -> Vec<Entry> {
        let Ok(reader) = std::fs::read_dir(dir) else { return vec![] };
        let root = self.root_of(dir).map_or(dir, |r| r.as_path());
        let mut entries: Vec<Entry> = reader
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let kind = e.file_type().ok()?;
                let path = e.path();
                let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                if crate::glob::excluded(&self.exclude, &relative) {
                    return None;
                }
                let is_dir = if kind.is_symlink() { path.is_dir() } else { kind.is_dir() };
                Some(Entry { path, name, is_dir })
            })
            .collect();
        sort_entries(&mut entries, &self.sort);
        entries
    }

    fn load(&mut self, dir: &Path) {
        if !self.children.contains_key(dir) {
            let entries = self.read_dir(dir);
            self.children.insert(dir.to_path_buf(), entries);
        }
    }

    pub fn refresh(&mut self) {
        let loaded: Vec<PathBuf> = self.children.keys().cloned().collect();
        for dir in loaded {
            if dir.is_dir() {
                let entries = self.read_dir(&dir);
                self.children.insert(dir, entries);
            } else {
                self.children.remove(&dir);
                self.expanded.remove(&dir);
            }
        }
        self.flipped.retain(|p| p.parent().is_some_and(|d| self.children.contains_key(d)));
        self.dirty = true;
    }

    pub fn has_focus(&self) -> bool {
        self.focused || self.open_focus
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn take_drop(&mut self) -> Option<(Vec<PathBuf>, Pos2)> {
        self.dropped.take()
    }

    pub fn open_filter(&mut self) {
        self.filter.open = true;
        self.filter.focus = true;
        self.collapsed = false;
    }

    pub fn focus_open_editors(&mut self) {
        self.open_editors = true;
        self.focus_open = true;
    }

    pub fn exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        match cmd {
            Command::Cut | Command::CutFiles => self.clip(true, ctx),
            Command::Copy | Command::CopyFiles => self.clip(false, ctx),
            Command::Paste | Command::PasteFiles => self.paste(ctx),
            Command::SelectAll => self.select_all(),
            _ => return false,
        }
        true
    }

    fn expand(&mut self, dir: &Path) {
        self.load(dir);
        self.expanded.insert(dir.to_path_buf());
        self.dirty = true;
    }

    fn row_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|r| r.holds(path))
    }

    fn is_open(&self, dir: &Path) -> bool {
        self.expanded.contains(dir) || (self.filter.active() && self.filter.hide && self.children.contains_key(dir))
    }

    fn build(&mut self) {
        self.dirty = false;
        self.animation = None;
        let mut rows = vec![];
        if let Some(root) = self.root.clone() {
            self.load(&root);
            self.push_rows(&root, 0, &mut rows);
            self.apply_filter(&mut rows);
        } else if self.title.is_some() {
            for (root, name) in self.roots.clone() {
                rows.push(Row { path: root.clone(), name, depth: 0, is_dir: true, ..Row::default() });
                if self.is_open(&root) {
                    self.push_rows(&root, 1, &mut rows);
                }
            }
            self.apply_filter(&mut rows);
        }
        self.rows = rows;
        self.prune_selection();
    }

    fn apply_filter(&mut self, rows: &mut Vec<Row>) {
        self.filter.matches = 0;
        if !self.filter.active() {
            return;
        }
        let names: Vec<(usize, &str)> = rows.iter().map(|r| (r.depth, r.name.as_str())).collect();
        let (hits, keep) = ops::filter_rows(self.filter.text.trim(), &names);
        self.filter.matches = hits.iter().flatten().count();
        let mut keep = keep.into_iter();
        let hide = self.filter.hide;
        for (row, hit) in rows.iter_mut().zip(hits) {
            row.hits = hit.unwrap_or_default();
        }
        rows.retain(|row| keep.next().unwrap_or(true) || row.editing || !hide);
    }

    fn prune_selection(&mut self) {
        if self.filter.active() {
            return;
        }
        let rows = std::mem::take(&mut self.rows);
        let visible = |p: &Path| rows.iter().any(|r| r.holds(p));
        self.sel.paths.retain(|p| visible(p));
        if let Some(cursor) = self.sel.cursor.clone() && !visible(&cursor) {
            match rows.iter().rev().find(|r| cursor.starts_with(&r.path)).map(|r| r.path.clone()) {
                Some(path) => self.sel.set(path),
                None => self.sel.clear(),
            }
        }
        if self.sel.anchor.as_deref().is_some_and(|a| !visible(a)) {
            self.sel.anchor = self.sel.cursor.clone();
        }
        self.rows = rows;
    }

    fn push_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        if let Some(edit) = &self.edit
            && edit.kind != EditKind::Rename
            && edit.target == dir
        {
            rows.push(Row { path: dir.join("\u{0}"), depth, is_dir: edit.kind == EditKind::NewFolder, editing: true, ..Row::default() });
        }
        let Some(entries) = self.children.get(dir).cloned() else { return };
        let owners = self.owners(dir, &entries);
        let mut nested = vec![vec![]; entries.len()];
        for (child, parent) in owners.iter().enumerate() {
            if let Some(parent) = parent {
                nested[*parent].push(child);
            }
        }
        for i in (0..entries.len()).filter(|i| owners[*i].is_none()) {
            self.push_entry(&entries, &nested, i, depth, rows);
        }
    }

    fn owners(&self, dir: &Path, entries: &[Entry]) -> Vec<Option<usize>> {
        let mut out = vec![None; entries.len()];
        if !self.nest {
            return out;
        }
        let files: Vec<usize> = (0..entries.len()).filter(|i| !entries[*i].is_dir).collect();
        let names: Vec<&str> = files.iter().map(|i| entries[*i].name.as_str()).collect();
        let patterns: Vec<(&str, &str)> = self.patterns.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
        for (k, parent) in nesting::nest(&names, &name_of(dir), &patterns).into_iter().enumerate() {
            out[files[k]] = parent.map(|p| files[p]);
        }
        out
    }

    fn group_open(&self, path: &Path) -> bool {
        self.nest_expand != self.flipped.contains(path)
    }

    fn opened(&self, row: &Row) -> bool {
        if row.is_dir { self.expanded.contains(&row.path) } else { !row.kids.is_empty() && self.group_open(&row.path) }
    }

    fn push_entry(&mut self, entries: &[Entry], nested: &[Vec<usize>], i: usize, depth: usize, rows: &mut Vec<Row>) {
        let entry = &entries[i];
        let editing = self.edit.as_ref().is_some_and(|e| e.kind == EditKind::Rename && e.target == entry.path);
        let (mut last, mut name, mut chain) = (entry.path.clone(), entry.name.clone(), vec![]);
        while self.compact && entry.is_dir {
            if self.edit.as_ref().is_some_and(|e| e.kind != EditKind::Rename && e.target == last) {
                break;
            }
            self.load(&last);
            let only = match self.children[&last].as_slice() {
                [only] if only.is_dir => only.clone(),
                _ => break,
            };
            if self.edit.as_ref().is_some_and(|e| e.kind == EditKind::Rename && e.target == only.path) {
                break;
            }
            if chain.is_empty() {
                chain.push(entry.path.clone());
            }
            chain.push(only.path.clone());
            name = format!("{name}/{}", only.name);
            last = only.path;
        }
        let (mut kids, mut stack) = (vec![], nested[i].clone());
        while let Some(k) = stack.pop() {
            kids.push(entries[k].path.clone());
            stack.extend(&nested[k]);
        }
        let open = !kids.is_empty() && (self.group_open(&entry.path) || self.filter.active() && self.filter.hide);
        rows.push(Row { path: last.clone(), name, depth, is_dir: entry.is_dir, editing, chain, kids, ..Row::default() });
        if entry.is_dir && self.is_open(&last) {
            self.push_rows(&last, depth + 1, rows);
        }
        if open {
            for &k in &nested[i] {
                self.push_entry(entries, nested, k, depth + 1, rows);
            }
        }
    }

    fn parents(&self, path: &Path) -> Vec<PathBuf> {
        let Some(root) = self.root_of(path) else { return vec![] };
        let single = self.root.is_some();
        path.ancestors().skip(1).take_while(|a| a.starts_with(root) && !(single && a == root)).map(Path::to_path_buf).collect()
    }

    fn reveal_active(&mut self, active: &Path) {
        if self.root_of(active).is_none() {
            return;
        }
        let mut dirs = self.parents(active);
        dirs.reverse();
        for dir in dirs {
            self.expand(&dir);
        }
        if let Some(dir) = active.parent() {
            self.load(dir);
            let entries = self.children[dir].clone();
            let owners = self.owners(dir, &entries);
            let mut up = entries.iter().position(|e| e.path == active).and_then(|i| owners[i]);
            while let Some(i) = up {
                if !self.group_open(&entries[i].path) && !self.flipped.remove(&entries[i].path) {
                    self.flipped.insert(entries[i].path.clone());
                }
                up = owners[i];
            }
            self.dirty = true;
        }
        self.sel.set(active.to_path_buf());
        self.scroll_to = Some(active.to_path_buf());
    }

    fn start_edit(&mut self, kind: EditKind, target: PathBuf) {
        self.filter.open = false;
        self.filter.text.clear();
        for dir in self.parents(&target) {
            self.expand(&dir);
        }
        let text = match kind {
            EditKind::Rename => target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            _ => {
                if Some(&target) != self.root.as_ref() {
                    self.expand(&target);
                }
                String::new()
            }
        };
        self.edit = Some(Edit { kind, target, text, focus: true });
        self.error = None;
        self.dirty = true;
    }

    fn commit_edit(&mut self, cmds: &mut Vec<Command>) {
        let Some(mut edit) = self.edit.take() else { return };
        self.dirty = true;
        let name = edit.text.trim().to_owned();
        let result = validate(&name).and_then(|_| match edit.kind {
            EditKind::NewFile | EditKind::NewFolder => {
                let path = edit.target.join(&name);
                if path.exists() {
                    return Err(format!("\"{name}\" already exists here."));
                }
                if edit.kind == EditKind::NewFile {
                    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
                    cmds.push(Command::OpenPath(path.clone()));
                } else {
                    std::fs::create_dir(&path).map_err(|e| e.to_string())?;
                }
                cmds.push(Command::RefreshExplorer);
                self.sel.set(path);
                Ok(())
            }
            EditKind::Rename => {
                let new = edit.target.with_file_name(&name);
                if new == edit.target {
                    return Ok(());
                }
                let same = new.to_string_lossy().to_lowercase() == edit.target.to_string_lossy().to_lowercase();
                if new.exists() && !same {
                    return Err(format!("\"{name}\" already exists here."));
                }
                std::fs::rename(&edit.target, &new).map_err(|e| e.to_string())?;
                let moved: Vec<PathBuf> = self.expanded.iter().filter(|p| p.starts_with(&edit.target)).cloned().collect();
                for old in moved {
                    self.expanded.remove(&old);
                    self.children.remove(&old);
                    if let Ok(rest) = old.strip_prefix(&edit.target) {
                        self.expand(&new.join(rest));
                    }
                }
                self.sel.set(new.clone());
                cmds.push(Command::PathChanged(edit.target.clone(), Some(new)));
                Ok(())
            }
        });
        if let Err(error) = result {
            self.error = Some(error);
            edit.focus = true;
            self.edit = Some(edit);
        }
    }

    fn toggle_folders(&mut self, dir: Option<PathBuf>, now: f64) {
        if dir.is_none() && self.expanded.is_empty() {
            return;
        }
        if self.dirty {
            self.build();
        }
        let group = dir.as_ref().and_then(|d| self.rows.iter().find(|r| r.path == *d)).is_some_and(|r| !r.is_dir && !r.kids.is_empty());
        let opening = dir.as_ref().is_some_and(|d| if group { !self.group_open(d) } else { !self.expanded.contains(d) });
        let elapsed = self.animation.as_ref().filter(|a| a.dir == dir).map_or(0.0, |a| {
            let t = a.progress(now);
            f64::from(if opening { t } else { 1.0 - t }) * FOLDER_TIME
        });
        let outgoing = if opening { vec![] } else { std::mem::take(&mut self.rows) };
        if let Some(path) = &dir {
            self.sel.set(path.clone());
            if group {
                if !self.flipped.remove(path) {
                    self.flipped.insert(path.clone());
                }
            } else if opening {
                self.expand(path);
            } else {
                self.expanded.remove(path);
            }
        } else {
            self.expanded.clear();
        }
        self.build();
        if self.edit.is_some() && !self.rows.iter().any(|r| r.editing) {
            self.edit = None;
        }
        let rows = if opening { &self.rows } else { &outgoing };
        let at = dir.as_ref().and_then(|p| rows.iter().position(|r| r.holds(p)));
        let until = at.map_or(0, |a| rows[a + 1..].iter().position(|r| r.depth <= rows[a].depth).map_or(rows.len(), |n| a + 1 + n));
        let moving: Vec<bool> = (0..rows.len()).map(|i| if dir.is_none() { rows[i].depth > 0 } else { at.is_some_and(|a| i > a && i < until) }).collect();
        let mut first = 0;
        let mut spans = Vec::new();
        for group in moving.chunk_by(|a, b| a == b) {
            let end = first + group.len();
            spans.push((first..end, group[0]));
            first = end;
        }
        if spans.iter().any(|(_, moving)| *moving) {
            self.animation = Some(FolderAnimation { dir, start: now - elapsed, outgoing, spans });
        }
    }

    fn extend(&mut self, to: PathBuf) {
        let order: Vec<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        if let Some(row) = self.sel.anchor.as_deref().and_then(|a| self.row_of(a)) {
            self.sel.anchor = Some(order[row].clone());
        }
        let to = self.row_of(&to).map_or(to, |i| order[i].clone());
        self.sel.range(&order, to);
    }

    fn select_all(&mut self) {
        let order: Vec<PathBuf> = self.rows.iter().filter(|r| !r.editing).map(|r| r.path.clone()).collect();
        self.sel.all(&order);
    }

    fn go_match(&mut self, forward: bool) {
        let hits: Vec<usize> = (0..self.rows.len()).filter(|i| !self.rows[*i].hits.is_empty()).collect();
        let (Some(first), Some(last)) = (hits.first().copied(), hits.last().copied()) else { return };
        let current = self.sel.cursor.as_deref().and_then(|c| self.row_of(c));
        let next = match (current, forward) {
            (Some(c), true) => hits.iter().copied().find(|i| *i > c).unwrap_or(first),
            (Some(c), false) => hits.iter().rev().copied().find(|i| *i < c).unwrap_or(last),
            (None, true) => first,
            (None, false) => last,
        };
        let path = self.rows[next].path.clone();
        self.sel.set(path.clone());
        self.scroll_to = Some(path);
    }

    fn clip(&mut self, cut: bool, ctx: &egui::Context) {
        let mut paths = ops::top_level(&self.sel.paths);
        paths.retain(|p| !cut || !self.is_root(p));
        if paths.is_empty() {
            return;
        }
        ctx.copy_text(paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"));
        self.clipboard = Some((paths, cut));
    }

    fn paste(&mut self, ctx: &egui::Context) {
        let Some((sources, cut)) = self.clipboard.clone() else { return };
        let dir = self.dir_for_new(self.sel.cursor.as_deref());
        self.transfer(sources, dir, cut, ctx);
    }

    fn request_move(&mut self, sources: Vec<PathBuf>, dir: PathBuf, ctx: &egui::Context) {
        if !self.confirm_drag {
            return self.transfer(sources, dir, true, ctx);
        }
        let (jobs, errors) = ops::plan(&sources, &dir, true);
        errors.into_iter().for_each(ui::notify);
        if !jobs.is_empty() {
            self.confirm_drop = Some((sources, dir));
        }
    }

    fn transfer(&mut self, sources: Vec<PathBuf>, dir: PathBuf, cut: bool, ctx: &egui::Context) {
        let (jobs, errors) = ops::plan(&sources, &dir, cut);
        errors.into_iter().for_each(ui::notify);
        if jobs.is_empty() {
            return;
        }
        if Some(&dir) != self.root.as_ref() {
            self.expand(&dir);
        }
        if cut && jobs.iter().any(|j| j.replace) {
            self.clash = Some(jobs);
        } else {
            self.spawn(jobs, cut, ctx);
        }
    }

    fn spawn(&mut self, jobs: Vec<Job>, cut: bool, ctx: &egui::Context) {
        self.background(ctx, move || ops::run(jobs, cut));
    }

    fn background(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Done + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut done = work();
            done.errors.drain(..).for_each(ui::notify_error);
            let _ = tx.send(done);
            ctx.request_repaint();
        });
        self.work.push(rx);
    }

    fn poll(&mut self, cmds: &mut Vec<Command>) {
        let mut finished = vec![];
        self.work.retain(|rx| match rx.try_recv() {
            Ok(done) => {
                finished.push(done);
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for done in finished {
            if !done.remaining.is_empty() {
                self.confirm_permanent.push_back(done.remaining);
            }
            if done.cut {
                self.clipboard = None;
            }
            if !done.removed.is_empty() {
                self.sel.paths.retain(|p| !done.removed.iter().any(|r| p.starts_with(r)));
                cmds.extend(done.removed.into_iter().map(|p| Command::PathChanged(p, None)));
                continue;
            }
            if done.done.is_empty() {
                continue;
            }
            let created: Vec<PathBuf> = done.done.iter().map(|(_, to)| to.clone()).collect();
            for (from, to) in done.done {
                if done.cut {
                    cmds.push(Command::PathChanged(from, Some(to)));
                }
            }
            cmds.push(Command::RefreshExplorer);
            self.refresh();
            self.scroll_to = created.last().cloned();
            self.sel = Selection { anchor: created.first().cloned(), cursor: created.last().cloned(), paths: created };
        }
    }

    fn delete(&mut self, ctx: &egui::Context) {
        let paths = std::mem::take(&mut self.confirm_delete);
        self.background(ctx, move || ops::trash(paths));
    }

    fn apply(&mut self, action: Action, ui: &Ui, cmds: &mut Vec<Command>) {
        match action {
            Action::Toggle(path) => self.toggle_folders(Some(path), ui.input(|i| i.time)),
            Action::Expand(path) => self.expand(&path),
            Action::Open(path) => {
                self.sel.set(path.clone());
                self.last_active = Some(path.clone());
                cmds.push(Command::OpenPath(path));
            }
            Action::Select(path) => self.sel.set(path),
            Action::Pick(path) => self.sel.toggle(path),
            Action::Extend(path) => self.extend(path),
            Action::SelectAll => self.select_all(),
            Action::New(kind, dir) => self.start_edit(kind, dir),
            Action::Rename(path) if self.is_root(&path) => {}
            Action::Rename(path) => self.start_edit(EditKind::Rename, path),
            Action::Delete(paths) => self.confirm_delete = ops::top_level(&paths).into_iter().filter(|p| !self.is_root(p)).collect(),
            Action::Clip(cut) => self.clip(cut, ui.ctx()),
            Action::Paste => self.paste(ui.ctx()),
            Action::Drop(paths, dir) if paths.iter().any(|p| self.is_root(p)) => {
                let index = |path: &Path| self.roots.iter().position(|(r, _)| r == path);
                if let ([from], Some(to)) = (paths.as_slice(), self.root_of(&dir).and_then(|r| index(r)))
                    && let Some(from) = index(from)
                    && from != to
                {
                    cmds.push(Command::MoveWorkspaceFolder(from, to));
                }
            }
            Action::Drop(paths, dir) => self.request_move(paths, dir, ui.ctx()),
            Action::Import(files, dir) => self.transfer(files, dir, false, ui.ctx()),
            Action::Match(forward) => self.go_match(forward),
            Action::CloseFilter => {
                self.filter.open = false;
                self.filter.text.clear();
                self.focus_tree = true;
                self.dirty = true;
            }
            Action::Copy(text) => ui.ctx().copy_text(text),
            Action::Reveal(path) => reveal(&path),
            Action::Compare(cmd) => cmds.push(cmd),
            Action::Refresh => cmds.push(Command::RefreshExplorer),
            Action::CollapseAll => self.toggle_folders(None, ui.input(|i| i.time)),
        }
    }

    fn dir_for_new(&self, path: Option<&Path>) -> PathBuf {
        let root = self.root.clone().or_else(|| self.roots.first().map(|(r, _)| r.clone())).unwrap_or_default();
        match path {
            Some(p) if p.is_dir() => p.to_path_buf(),
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or(root),
            None => root,
        }
    }

    fn menu(&self, ui: &mut Ui, path: Option<&Path>, action: &mut Option<Action>) {
        ui.set_min_width(190.0);
        ui::menu_pad(ui);
        let dir = self.dir_for_new(path);
        if ui.button("New File...").clicked() {
            *action = Some(Action::New(EditKind::NewFile, dir.clone()));
            ui.close();
        }
        if ui.button("New Folder...").clicked() {
            *action = Some(Action::New(EditKind::NewFolder, dir.clone()));
            ui.close();
        }
        if let Some(root) = path.filter(|p| self.is_root(p)) {
            ui.separator();
            if self.clipboard.is_some() && ui.button("Paste").clicked() {
                *action = Some(Action::Paste);
                ui.close();
            }
            if ui.button("Add Folder to Workspace...").clicked() {
                *action = Some(Action::Compare(Command::AddFolderToWorkspace));
                ui.close();
            }
            if ui.button("Remove Folder from Workspace").clicked() {
                *action = Some(Action::Compare(Command::RemoveWorkspaceFolder(root.to_path_buf())));
                ui.close();
            }
            if ui.button("Copy Path").clicked() {
                *action = Some(Action::Copy(root.to_string_lossy().into_owned()));
                ui.close();
            }
            if ui.button("Reveal in File Explorer").clicked() {
                *action = Some(Action::Reveal(root.to_path_buf()));
                ui.close();
            }
            return;
        }
        let many = path.is_some_and(|p| self.sel.is_multiple() && self.sel.contains(p));
        let targets: Vec<PathBuf> = match path {
            Some(_) if many => self.sel.paths.clone(),
            Some(p) => vec![p.to_path_buf()],
            None => vec![dir],
        };
        if let Some(path) = path {
            ui.separator();
            if !many && !path.is_dir() {
                if ui.button("Select for Compare").clicked() {
                    *action = Some(Action::Compare(Command::SelectComparePath(path.to_owned())));
                    ui.close();
                }
                if self.compare.as_deref().is_some_and(|p| p != path) && ui.button("Compare with Selected").clicked() {
                    *action = Some(Action::Compare(Command::ComparePath(path.to_owned())));
                    ui.close();
                }
                ui.separator();
            }
            if ui.button("Cut").clicked() {
                *action = Some(Action::Clip(true));
                ui.close();
            }
            if ui.button("Copy").clicked() {
                *action = Some(Action::Clip(false));
                ui.close();
            }
        }
        if self.clipboard.is_some() && ui.button("Paste").clicked() {
            *action = Some(Action::Paste);
            ui.close();
        }
        if let Some(path) = path {
            ui.separator();
            if !many && ui.button("Rename...").clicked() {
                *action = Some(Action::Rename(path.to_path_buf()));
                ui.close();
            }
            let label = if many { format!("Delete {} Items", targets.len()) } else { "Delete".to_owned() };
            if ui.button(label).clicked() {
                *action = Some(Action::Delete(targets.clone()));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Copy Path").clicked() {
            let text = targets.iter().map(|t| t.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
            *action = Some(Action::Copy(text));
            ui.close();
        }
        if ui.button("Copy Relative Path").clicked() {
            let rel = |t: &PathBuf| self.root_of(t).and_then(|r| t.strip_prefix(r).ok()).unwrap_or(t).to_string_lossy().replace('\\', "/");
            *action = Some(Action::Copy(targets.iter().map(rel).collect::<Vec<_>>().join("\n")));
            ui.close();
        }
        if ui.button("Reveal in File Explorer").clicked() {
            *action = Some(Action::Reveal(targets[0].clone()));
            ui.close();
        }
    }

    fn keyboard(&mut self, ui: &Ui, action: &mut Option<Action>) {
        let (cut, copy, paste, typed) = ui.input_mut(|i| {
            let mut found = (false, false, false, String::new());
            i.events.retain(|e| match e {
                egui::Event::Cut => {
                    found.0 = true;
                    false
                }
                egui::Event::Copy => {
                    found.1 = true;
                    false
                }
                egui::Event::Paste(_) => {
                    found.2 = true;
                    false
                }
                egui::Event::Text(t) if !t.chars().any(char::is_control) => {
                    found.3.push_str(t);
                    false
                }
                _ => true,
            });
            found
        });
        if !typed.trim().is_empty() {
            self.filter.open = true;
            self.filter.focus = true;
            self.filter.text.push_str(&typed);
            self.dirty = true;
        }
        let pressed = |modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
        if cut {
            *action = Some(Action::Clip(true));
        } else if copy {
            *action = Some(Action::Clip(false));
        } else if paste {
            *action = Some(Action::Paste);
        }
        if pressed(Modifiers::NONE, Key::Escape) {
            if self.filter.open {
                *action = Some(Action::CloseFilter);
            } else {
                self.clipboard = None;
            }
        }
        if self.filter.open && pressed(Modifiers::SHIFT, Key::F3) {
            *action = Some(Action::Match(false));
        } else if self.filter.open && pressed(Modifiers::NONE, Key::F3) {
            *action = Some(Action::Match(true));
        }
        let index = self.sel.cursor.as_deref().and_then(|c| self.row_of(c));
        let step = |i: Option<usize>, down: bool| match (i, down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(self.rows.len().saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        };
        let at = |i: usize| self.rows.get(i).map(|r| r.path.clone());
        let moved = if pressed(Modifiers::COMMAND, Key::A) {
            *action = Some(Action::SelectAll);
            None
        } else if pressed(Modifiers::SHIFT, Key::ArrowDown) {
            let to = at(step(index, true));
            *action = to.clone().map(Action::Extend);
            to
        } else if pressed(Modifiers::SHIFT, Key::ArrowUp) {
            let to = at(step(index, false));
            *action = to.clone().map(Action::Extend);
            to
        } else if pressed(Modifiers::NONE, Key::ArrowDown) {
            let to = at(step(index, true));
            *action = to.clone().map(Action::Select);
            to
        } else if pressed(Modifiers::NONE, Key::ArrowUp) {
            let to = at(step(index, false));
            *action = to.clone().map(Action::Select);
            to
        } else {
            None
        };
        if moved.is_some() {
            self.scroll_to = moved;
        } else if let (Some(i), Some(cursor)) = (index, self.sel.cursor.clone()) {
            let row = self.rows[i].clone();
            let open = self.opened(&row);
            let folds = row.is_dir || !row.kids.is_empty();
            if pressed(Modifiers::NONE, Key::ArrowRight) && folds {
                *action = if open { at(i + 1).map(Action::Select) } else { Some(Action::Toggle(row.path)) };
            } else if pressed(Modifiers::NONE, Key::ArrowLeft) {
                *action = if folds && open && cursor == row.path {
                    Some(Action::Toggle(row.path))
                } else if let Some(owner) = self.rows[..i].iter().rev().find(|r| r.depth < row.depth).filter(|r| !r.is_dir && cursor == row.path) {
                    Some(Action::Select(owner.path.clone()))
                } else {
                    cursor.parent().filter(|p| self.row_of(p).is_some()).map(|p| Action::Select(p.to_path_buf()))
                };
            } else if pressed(Modifiers::NONE, Key::Enter) {
                *action = Some(if row.is_dir { Action::Toggle(row.path) } else { Action::Open(row.path) });
            } else if pressed(Modifiers::NONE, Key::F2) {
                *action = Some(Action::Rename(cursor));
            } else if pressed(Modifiers::NONE, Key::Delete) {
                let paths = if self.sel.paths.is_empty() { vec![cursor] } else { self.sel.paths.clone() };
                *action = Some(Action::Delete(paths));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        git: &Git,
        active: Option<&Path>,
        settings: &Settings,
        tabs: &[Vec<OpenTab>],
        cmds: &mut Vec<Command>,
    ) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui::section_header(ui, theme, "Explorer");
        let Some(root) = self.root.clone().or_else(|| self.roots.first().map(|(r, _)| r.clone())) else {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("You have not opened a folder yet.").color(theme.text_dim));
                ui.add_space(8.0);
                if ui.button(RichText::new("Open Folder").color(theme.accent)).clicked() {
                    cmds.push(Command::OpenFolder);
                }
            });
            return;
        };

        self.confirm_drag = settings.confirm_drag_and_drop;
        self.confirm = settings.confirm_delete;
        if self.exclude != settings.files_exclude || self.compact != settings.compact_folders || self.sort != settings.explorer_sort {
            self.exclude = settings.files_exclude.clone();
            self.compact = settings.compact_folders;
            self.sort = settings.explorer_sort.clone();
            self.refresh();
        }
        if (self.nest, self.nest_expand, &self.patterns) != (settings.file_nesting, settings.file_nesting_expand, &settings.file_nesting_patterns) {
            (self.nest, self.nest_expand, self.patterns) = (settings.file_nesting, settings.file_nesting_expand, settings.file_nesting_patterns.clone());
            self.dirty = true;
        }
        self.poll(cmds);
        if active != self.last_active.as_deref() {
            self.last_active = active.map(Path::to_path_buf);
            if let Some(active) = active {
                self.reveal_active(active);
            }
        }
        if self.dirty {
            self.build();
        }

        let mut action = None;
        self.open_editors(ui, theme, &root, tabs, cmds);
        self.header(ui, theme, &root, &mut action);
        self.tree(ui, theme, git, active, &root, &mut action, cmds);
        if let Some(action) = action {
            self.apply(action, ui, cmds);
            ui.ctx().request_repaint();
        }
        self.overlays(ui, theme);
    }

    fn header(&mut self, ui: &mut Ui, theme: &Theme, root: &Path, action: &mut Option<Action>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let hover = ui.rect_contains_pointer(rect);
        let h = ui::anim(ui, resp.id.with("hover"), hover);
        let open = ui::anim(ui, resp.id.with("open"), !self.collapsed);
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0, ui::mix(theme.panel, theme.bar, 0.5 + 0.5 * h));
        painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, theme.border));
        ui::caret(&painter, pos2(rect.left() + 12.0, rect.center().y), open, theme.text);
        let folder = self.title.clone().unwrap_or_else(|| name_of(root));
        let kind = if self.collapsed { crate::icon_theme::Kind::Root } else { crate::icon_theme::Kind::RootOpen };
        let icon = crate::icon_theme::paint(&painter, pos2(rect.left() + 32.0, rect.center().y), 14.0, &folder, kind, 1.0);
        let name = folder.to_uppercase();
        painter.text(pos2(rect.left() + if icon { 44.0 } else { 24.0 }, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.5), theme.text);
        if resp.clicked() {
            self.collapsed ^= true;
        }
        let root = match self.root.is_some() {
            true => root.to_path_buf(),
            false => self.dir_for_new(self.sel.cursor.as_deref()),
        };
        if h > 0.0 {
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 1.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                ui.set_opacity(h);
                ui.spacing_mut().item_spacing.x = 0.0;
                if ui::icon_button(ui, theme, icons::ARROWS_IN_LINE_VERTICAL, 22.0, false, "Collapse All").clicked() {
                    *action = Some(Action::CollapseAll);
                }
                if ui::icon_button(ui, theme, icons::ARROWS_CLOCKWISE, 22.0, false, "Refresh").clicked() {
                    *action = Some(Action::Refresh);
                }
                if ui::icon_button(ui, theme, icons::FOLDER_PLUS, 22.0, false, "New Folder").clicked() {
                    *action = Some(Action::New(EditKind::NewFolder, root.to_path_buf()));
                }
                if ui::icon_button(ui, theme, icons::FILE_PLUS, 22.0, false, "New File").clicked() {
                    *action = Some(Action::New(EditKind::NewFile, root.to_path_buf()));
                }
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tree(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        git: &Git,
        active: Option<&Path>,
        root: &Path,
        action: &mut Option<Action>,
        cmds: &mut Vec<Command>,
    ) {
        let mut area = ui.available_rect_before_wrap();
        if self.error.is_some() {
            area.max.y -= 30.0;
        }
        let tree_id = Id::new("explorer_tree");
        let now = ui.input(|i| i.time);
        if self.animation.as_ref().is_some_and(|a| now - a.start >= FOLDER_TIME) {
            self.animation = None;
        }
        let reveal = ui::reveal(ui, tree_id.with("reveal"), !self.collapsed);
        if reveal < 1.0 {
            let count = self.animation.as_ref().map_or(0, |a| a.outgoing.len()).max(self.rows.len());
            area.set_height(area.height().min(count as f32 * ROW) * reveal);
        }
        if area.height() <= 0.0 {
            self.focused = false;
            return;
        }
        let pointer = ui.ctx().pointer_latest_pos().filter(|p| area.contains(*p) && !self.collapsed);
        let files: Vec<PathBuf> = match pointer {
            Some(_) => ui.input_mut(|i| std::mem::take(&mut i.raw.dropped_files)).into_iter().map(|f| f.path().to_path_buf()).collect(),
            None => vec![],
        };
        let bg = ui.interact(area, tree_id, if self.collapsed { Sense::hover() } else { Sense::click() });
        if std::mem::take(&mut self.focus_tree) {
            bg.request_focus();
        }
        self.focused = !self.collapsed && bg.has_focus();
        if bg.clicked() {
            bg.request_focus();
            self.sel.clear();
        }
        if !self.collapsed {
            bg.context_menu(|ui| self.menu(ui, None, action));
        }
        if self.focused && self.edit.is_none() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(tree_id, egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() });
            });
            self.keyboard(ui, action);
        }
        let dragging = self.drag.is_some();
        if dragging && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.drag = None;
        }

        let animation = self.animation.take();
        let closing = animation.as_ref().is_some_and(|a| !a.outgoing.is_empty());
        let live = std::mem::take(&mut self.rows);
        let rows = if let Some(a) = &animation && closing { &a.outgoing } else { &live };
        let full = [(0..rows.len(), false)];
        let spans = animation.as_ref().map_or(full.as_slice(), |a| a.spans.as_slice());
        let progress = animation.as_ref().map_or(1.0, |a| a.progress(now));
        let scroll_to = if self.collapsed { None } else { self.scroll_to.take().and_then(|p| rows.iter().position(|r| r.holds(&p))) };
        let cut: Vec<PathBuf> = self.clipboard.as_ref().filter(|(_, cut)| *cut).map(|(p, _)| p.clone()).unwrap_or_default();
        let mut commit = false;
        let mut cancel = false;
        let mut over_row = false;
        let mut over_dir: Option<PathBuf> = None;
        ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
            ui.shrink_clip_rect(area);
            ScrollArea::vertical().min_scrolled_height(0.0).auto_shrink(false).show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                let width = ui.available_width();
                let mut bottom = 0.0;
                for (span, moving) in spans {
                    let fade = if *moving { progress } else { 1.0 };
                    let top = bottom;
                    bottom += span.len() as f32 * ROW * fade;
                    if *moving && top <= viewport.max.y && ui.is_rect_visible(area) {
                        ui.ctx().request_repaint();
                    }
                    if let Some(i) = scroll_to.filter(|i| span.contains(i)) {
                        let y = (top + (i - span.start) as f32 * ROW).min((bottom - ROW).max(top));
                        ui.scroll_to_rect(Rect::from_min_size(origin + vec2(0.0, y), vec2(10.0, ROW)), None);
                    }
                    if top >= viewport.max.y || bottom <= viewport.min.y || fade <= 0.0 {
                        continue;
                    }
                    let first = ((viewport.min.y - top) / ROW).floor().max(0.0) as usize;
                    let end = ((viewport.max.y.min(bottom) - top) / ROW).ceil() as usize;
                    for offset in first..end.min(span.len()) {
                        let row = &rows[span.start + offset];
                        let y = top + offset as f32 * ROW;
                        let rect = Rect::from_min_size(origin + vec2(0.0, y), vec2(width, ROW.min(bottom - y)));
                        let id = Id::new(("explorer_row", &row.path));
                        let interactive = !(self.collapsed || closing && *moving);
                        let resp = ui.interact(rect, id, if interactive { Sense::click_and_drag() } else { Sense::hover() });
                        let selected = self.sel.contains(&row.path) || row.chain.iter().any(|p| self.sel.contains(p));
                        let is_active = active == Some(row.path.as_path());
                        ui::row_bg(ui, theme, rect, id, resp.hovered(), selected || is_active);
                        ui::tree_item(ui, &resp, &row.name, selected, row.depth, match row.is_dir {
                            true => Some(self.is_open(&row.path)),
                            false => (!row.kids.is_empty()).then(|| self.opened(row)),
                        });
                        let under = interactive && pointer.is_some_and(|p| rect.contains(p));
                        over_row |= under;
                        if under && row.is_dir && dragging {
                            over_dir = Some(row.path.clone());
                            ui.painter().rect_filled(rect, 0, ui::with_alpha(theme.accent, 0.18));
                            ui.painter().rect_stroke(rect, 0, Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
                        }

                        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 8.0));
                        for level in 0..row.depth {
                            let x = rect.left() + 13.0 + level as f32 * INDENT;
                            painter.vline(x, rect.y_range(), Stroke::new(1.0, ui::with_alpha(theme.border, 0.7 * fade)));
                        }
                        let x = rect.left() + 12.0 + row.depth as f32 * INDENT - (1.0 - fade) * 6.0;
                        let y = rect.center().y;
                        let open = self.opened(row);
                        let state = match open || row.kids.is_empty() {
                            true => git.state(&row.path),
                            false => row.kids.iter().chain([&row.path]).filter_map(|p| git.state(p)).max_by_key(|s| s.strength()),
                        };
                        let dimmed = if cut.iter().any(|c| row.path.starts_with(c)) { 0.45 } else { 1.0 };
                        let color = ui::with_alpha(state_color(theme, state), fade * dimmed);
                        let severity = row.kids.iter().filter(|_| !open).chain([&row.path]).filter_map(|p| crate::lsp::worst(&self.problems, p)).min();
                        let tint = ui::with_alpha(name_color(theme, severity, state), fade * dimmed);
                        let dim = ui::with_alpha(theme.text_dim, fade * dimmed);
                        if row.is_dir || !row.kids.is_empty() {
                            ui::caret(&painter, pos2(x, y), ui::anim(ui, id.with("caret"), open), dim);
                        }
                        let icon = match (row.is_dir, open) {
                            (true, true) => icons::FOLDER_OPEN,
                            (true, false) => icons::FOLDER,
                            _ => file_icon(&row.name),
                        };
                        let icon_color = if row.is_dir { ui::with_alpha(theme.accent, fade * 0.85 * dimmed) } else { dim };
                        let kind = match (row.is_dir, open) {
                            _ if row.depth == 0 && self.root.is_none() => match open {
                                true => crate::icon_theme::Kind::RootOpen,
                                false => crate::icon_theme::Kind::Root,
                            },
                            (true, true) => crate::icon_theme::Kind::FolderOpen,
                            (true, false) => crate::icon_theme::Kind::Folder,
                            _ => crate::icon_theme::Kind::File,
                        };
                        if !crate::icon_theme::paint(&painter, pos2(x + 17.0, y), 16.0, &row.name, kind, fade * dimmed) {
                            painter.text(pos2(x + 10.0, y), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), icon_color);
                        }

                        if row.editing && interactive {
                            if let Some(edit) = &mut self.edit {
                                let field = Rect::from_min_max(pos2(x + 30.0, rect.top() + 1.0), pos2(rect.right() - 6.0, rect.bottom() - 1.0));
                                let out = ui.put(
                                    field,
                                    egui::TextEdit::singleline(&mut edit.text)
                                        .font(FontId::proportional(13.0))
                                        .margin(egui::Margin::symmetric(4, 1)),
                                );
                                if edit.focus {
                                    out.request_focus();
                                    edit.focus = false;
                                } else if out.lost_focus() {
                                    if ui.input(|i| i.key_pressed(Key::Enter)) { commit = true } else { cancel = true }
                                }
                            }
                            continue;
                        }

                        let galley = painter.layout_no_wrap(row.name.clone(), FontId::proportional(13.0), tint);
                        let left = x + 31.0;
                        let at = |index: usize| left + galley.pos_from_cursor(CCursor::new(index)).min.x;
                        let mut spans_x: Vec<(f32, f32)> = vec![];
                        let mut start = 0;
                        for part in row.chain.iter().map(|p| name_of(p).chars().count()) {
                            spans_x.push((at(start), at(start + part)));
                            start += part + 1;
                        }
                        for (p, (a, b)) in row.chain.iter().zip(&spans_x) {
                            if self.sel.cursor.as_ref() == Some(p) && *p != row.path {
                                let mark = Rect::from_min_max(pos2(*a - 2.0, y - 8.0), pos2(*b + 2.0, y + 8.0));
                                painter.rect_filled(mark, 3, ui::with_alpha(theme.accent, 0.25 * fade));
                            }
                        }
                        for &index in &row.hits {
                            let mark = Rect::from_min_max(pos2(at(index), y - 8.0), pos2(at(index + 1), y + 8.0));
                            painter.rect_filled(mark, 2, ui::with_alpha(theme.accent, 0.35 * fade));
                        }
                        painter.galley(pos2(left, y - galley.size().y / 2.0), galley, tint);
                        let badge = state_badge(state);
                        if row.is_dir && !badge.is_empty() {
                            painter.circle_filled(pos2(rect.right() - 14.0, y), 3.0, ui::with_alpha(color, 0.8));
                        } else if !badge.is_empty() {
                            painter.text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);
                        }

                        if !interactive { continue; }
                        let hit = |px: Option<Pos2>| -> PathBuf {
                            let Some(px) = px else { return row.path.clone() };
                            row.chain.iter().zip(&spans_x).find(|(_, (_, b))| px.x <= *b + 4.0).map_or(row.path.clone(), |(p, _)| p.clone())
                        };
                        if resp.drag_started() {
                            bg.request_focus();
                            let hit = hit(resp.interact_pointer_pos());
                            if !self.sel.contains(&hit) {
                                self.sel.set(hit);
                            }
                            self.drag = Some(ops::top_level(&self.sel.paths));
                        }
                        if resp.double_clicked() && !row.is_dir {
                            cmds.push(Command::KeepEditor);
                        }
                        if resp.clicked() {
                            bg.request_focus();
                            let hit = hit(resp.interact_pointer_pos());
                            let modifiers = ui.input(|i| i.modifiers);
                            *action = Some(if modifiers.command {
                                Action::Pick(hit)
                            } else if modifiers.shift {
                                Action::Extend(hit)
                            } else if hit != row.path {
                                Action::Select(hit)
                            } else if row.is_dir || !row.kids.is_empty() && resp.interact_pointer_pos().is_some_and(|p| p.x < x + 8.0) {
                                Action::Toggle(row.path.clone())
                            } else {
                                Action::Open(row.path.clone())
                            });
                        }
                        if resp.secondary_clicked() {
                            let hit = hit(resp.interact_pointer_pos());
                            if !self.sel.contains(&hit) {
                                self.sel.set(hit);
                            }
                        }
                        resp.context_menu(|ui| {
                            let target = self.sel.cursor.clone().filter(|c| row.chain.contains(c)).unwrap_or_else(|| row.path.clone());
                            self.menu(ui, Some(&target), action)
                        });
                    }
                }
                ui.set_min_height(bottom);
            });
        });
        self.rows = live;
        self.animation = animation;
        if commit {
            self.commit_edit(cmds);
        } else if cancel {
            self.edit = None;
            self.dirty = true;
        }
        if self.filter.open && !self.collapsed {
            self.filter_box(ui, theme, area, action);
        }
        if !files.is_empty() {
            *action = Some(Action::Import(files, over_dir.clone().unwrap_or_else(|| root.to_path_buf())));
        }
        self.drag_state(ui, theme, area, pointer, over_row, over_dir, action);
    }

}
