use crate::command::Command;
use crate::git::{FileState, Git};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, Layout, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke, Ui,
    UiBuilder, pos2, vec2,
};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

const ROW: f32 = 22.0;
const INDENT: f32 = 12.0;
const FOLDER_TIME: f64 = (ui::ANIM * 1.4) as f64;

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

#[derive(Clone)]
struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    is_dir: bool,
    editing: bool,
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

enum Action {
    Toggle(PathBuf),
    Open(PathBuf),
    Select(PathBuf),
    New(EditKind, PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    Copy(String),
    Reveal(PathBuf),
    Refresh,
    CollapseAll,
}

pub struct Explorer {
    root: Option<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    dirty: bool,
    selected: Option<PathBuf>,
    last_active: Option<PathBuf>,
    scroll_to: Option<PathBuf>,
    edit: Option<Edit>,
    confirm_delete: Option<PathBuf>,
    error: Option<String>,
    focused: bool,
    collapsed: bool,
    animation: Option<FolderAnimation>,
}

fn read_dir(dir: &Path) -> Vec<Entry> {
    let Ok(reader) = std::fs::read_dir(dir) else { return vec![] };
    let mut entries: Vec<Entry> = reader
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let kind = e.file_type().ok()?;
            let path = e.path();
            let is_dir = if kind.is_symlink() { path.is_dir() } else { kind.is_dir() };
            Some(Entry { path, name, is_dir })
        })
        .collect();
    entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
    entries
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

impl Explorer {
    pub fn new() -> Self {
        Self {
            root: None,
            children: HashMap::new(),
            expanded: HashSet::new(),
            rows: vec![],
            dirty: true,
            selected: None,
            last_active: None,
            scroll_to: None,
            edit: None,
            confirm_delete: None,
            error: None,
            focused: false,
            collapsed: false,
            animation: None,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>) {
        *self = Self::new();
        if let Some(root) = &root {
            self.children.insert(root.clone(), read_dir(root));
        }
        self.root = root;
    }

    pub fn refresh(&mut self) {
        let loaded: Vec<PathBuf> = self.children.keys().cloned().collect();
        for dir in loaded {
            if dir.is_dir() {
                self.children.insert(dir.clone(), read_dir(&dir));
            } else {
                self.children.remove(&dir);
                self.expanded.remove(&dir);
            }
        }
        self.dirty = true;
    }

    pub fn has_focus(&self) -> bool {
        self.focused
    }

    fn expand(&mut self, dir: &Path) {
        if !self.children.contains_key(dir) {
            self.children.insert(dir.to_path_buf(), read_dir(dir));
        }
        self.expanded.insert(dir.to_path_buf());
        self.dirty = true;
    }

    fn build(&mut self) {
        self.dirty = false;
        self.animation = None;
        let mut rows = vec![];
        if let Some(root) = self.root.clone() {
            self.push_rows(&root, 0, &mut rows);
        }
        self.rows = rows;
        if let Some(path) = &self.selected && !self.rows.iter().any(|r| r.path == *path) {
            self.selected = self.rows.iter().rev().find(|r| path.starts_with(&r.path)).map(|r| r.path.clone());
        }
    }

    fn push_rows(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        if let Some(edit) = &self.edit
            && edit.kind != EditKind::Rename
            && edit.target == dir
        {
            rows.push(Row {
                path: dir.join("\u{0}"),
                name: String::new(),
                depth,
                is_dir: edit.kind == EditKind::NewFolder,
                editing: true,
            });
        }
        let Some(entries) = self.children.get(dir) else { return };
        for entry in entries {
            let editing = self.edit.as_ref().is_some_and(|e| e.kind == EditKind::Rename && e.target == entry.path);
            rows.push(Row {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                editing,
            });
            if entry.is_dir && self.expanded.contains(&entry.path) {
                self.push_rows(&entry.path, depth + 1, rows);
            }
        }
    }

    fn reveal_active(&mut self, active: &Path) {
        let Some(root) = self.root.clone() else { return };
        if !active.starts_with(&root) {
            return;
        }
        let mut dirs: Vec<PathBuf> = active.ancestors().skip(1).take_while(|a| *a != root).map(Path::to_path_buf).collect();
        dirs.reverse();
        for dir in dirs {
            self.expand(&dir);
        }
        self.selected = Some(active.to_path_buf());
        self.scroll_to = Some(active.to_path_buf());
    }

    fn start_edit(&mut self, kind: EditKind, target: PathBuf) {
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
                self.selected = Some(path);
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
                self.selected = Some(new.clone());
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
        let opening = dir.as_ref().is_some_and(|d| !self.expanded.contains(d));
        let elapsed = self.animation.as_ref().filter(|a| a.dir == dir).map_or(0.0, |a| {
            let t = a.progress(now);
            f64::from(if opening { t } else { 1.0 - t }) * FOLDER_TIME
        });
        let outgoing = if opening { vec![] } else { std::mem::take(&mut self.rows) };
        if let Some(path) = &dir {
            self.selected = Some(path.clone());
            if opening {
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
        let moving = |r: &Row| r.depth > 0 && dir.as_ref().is_none_or(|p| r.path != *p && r.path.starts_with(p));
        let mut first = 0;
        let mut spans = Vec::new();
        for group in rows.chunk_by(|a, b| moving(a) == moving(b)) {
            let end = first + group.len();
            spans.push((first..end, moving(&group[0])));
            first = end;
        }
        if spans.iter().any(|(_, moving)| *moving) {
            self.animation = Some(FolderAnimation { dir, start: now - elapsed, outgoing, spans });
        }
    }

    fn apply(&mut self, action: Action, ui: &Ui, cmds: &mut Vec<Command>) {
        match action {
            Action::Toggle(path) => self.toggle_folders(Some(path), ui.input(|i| i.time)),
            Action::Open(path) => {
                self.selected = Some(path.clone());
                self.last_active = Some(path.clone());
                cmds.push(Command::OpenPath(path));
            }
            Action::Select(path) => self.selected = Some(path),
            Action::New(kind, dir) => self.start_edit(kind, dir),
            Action::Rename(path) => self.start_edit(EditKind::Rename, path),
            Action::Delete(path) => self.confirm_delete = Some(path),
            Action::Copy(text) => ui.ctx().copy_text(text),
            Action::Reveal(path) => reveal(&path),
            Action::Refresh => cmds.push(Command::RefreshExplorer),
            Action::CollapseAll => self.toggle_folders(None, ui.input(|i| i.time)),
        }
    }

    fn dir_for_new(&self, path: Option<&Path>) -> PathBuf {
        let root = self.root.clone().unwrap_or_default();
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
        let target = path.map(Path::to_path_buf).unwrap_or(dir);
        if let Some(path) = path {
            ui.separator();
            if ui.button("Rename...").clicked() {
                *action = Some(Action::Rename(path.to_path_buf()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                *action = Some(Action::Delete(path.to_path_buf()));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Copy Path").clicked() {
            *action = Some(Action::Copy(target.to_string_lossy().into_owned()));
            ui.close();
        }
        if ui.button("Copy Relative Path").clicked() {
            let rel = self.root.as_ref().and_then(|r| target.strip_prefix(r).ok()).unwrap_or(&target);
            *action = Some(Action::Copy(rel.to_string_lossy().replace('\\', "/")));
            ui.close();
        }
        if ui.button("Reveal in File Explorer").clicked() {
            *action = Some(Action::Reveal(target));
            ui.close();
        }
    }

    fn keyboard(&mut self, ui: &Ui, action: &mut Option<Action>) {
        let index = self.selected.as_ref().and_then(|s| self.rows.iter().position(|r| &r.path == s));
        let pressed = |key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        let select = |i: usize| self.rows.get(i).map(|r| Action::Select(r.path.clone()));
        if pressed(Key::ArrowDown) {
            *action = select(index.map_or(0, |i| (i + 1).min(self.rows.len().saturating_sub(1))));
            self.scroll_to = action.as_ref().and_then(|a| if let Action::Select(p) = a { Some(p.clone()) } else { None });
        } else if pressed(Key::ArrowUp) {
            *action = select(index.map_or(0, |i| i.saturating_sub(1)));
            self.scroll_to = action.as_ref().and_then(|a| if let Action::Select(p) = a { Some(p.clone()) } else { None });
        } else if let Some(i) = index {
            let row = self.rows[i].clone();
            let open = self.expanded.contains(&row.path);
            if pressed(Key::ArrowRight) && row.is_dir {
                *action = if open { select(i + 1) } else { Some(Action::Toggle(row.path)) };
            } else if pressed(Key::ArrowLeft) {
                *action = if row.is_dir && open {
                    Some(Action::Toggle(row.path))
                } else {
                    row.path.parent().filter(|p| Some(*p) != self.root.as_deref()).map(|p| Action::Select(p.to_path_buf()))
                };
            } else if pressed(Key::Enter) {
                *action = Some(if row.is_dir { Action::Toggle(row.path) } else { Action::Open(row.path) });
            } else if pressed(Key::F2) {
                *action = Some(Action::Rename(row.path));
            } else if pressed(Key::Delete) {
                *action = Some(Action::Delete(row.path));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, git: &Git, active: Option<&Path>, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui::section_header(ui, theme, "Explorer");
        let Some(root) = self.root.clone() else {
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
        self.header(ui, theme, &root, &mut action);
        self.tree(ui, theme, git, active, &mut action, cmds);
        if let Some(action) = action {
            self.apply(action, ui, cmds);
            ui.ctx().request_repaint();
        }
        self.overlays(ui, theme, cmds);
    }

    fn header(&mut self, ui: &mut Ui, theme: &Theme, root: &Path, action: &mut Option<Action>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let hover = ui.rect_contains_pointer(rect);
        let h = ui::anim(ui, resp.id.with("hover"), hover);
        let open = ui::anim(ui, resp.id.with("open"), !self.collapsed);
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0, ui::mix(theme.panel, theme.bar, 0.5 + 0.5 * h));
        ui::caret(&painter, pos2(rect.left() + 12.0, rect.center().y), open, theme.text);
        let name = root.file_name().unwrap_or(root.as_os_str()).to_string_lossy().to_uppercase();
        painter.text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.5), theme.text);
        if resp.clicked() {
            self.collapsed ^= true;
        }
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

    fn tree(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        git: &Git,
        active: Option<&Path>,
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
        let bg = ui.interact(area, tree_id, if self.collapsed { Sense::hover() } else { Sense::click() });
        self.focused = !self.collapsed && bg.has_focus();
        if bg.clicked() {
            bg.request_focus();
            self.selected = None;
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

        let animation = self.animation.take();
        let closing = animation.as_ref().is_some_and(|a| !a.outgoing.is_empty());
        let live = std::mem::take(&mut self.rows);
        let rows = if let Some(a) = &animation && closing { &a.outgoing } else { &live };
        let full = [(0..rows.len(), false)];
        let spans = animation.as_ref().map_or(full.as_slice(), |a| a.spans.as_slice());
        let progress = animation.as_ref().map_or(1.0, |a| a.progress(now));
        let scroll_to = if self.collapsed { None } else { self.scroll_to.take().and_then(|p| rows.iter().position(|r| r.path == p)) };
        let mut commit = false;
        let mut cancel = false;
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
                        let resp = ui.interact(rect, id, if interactive { Sense::click() } else { Sense::hover() });
                        let selected = self.selected.as_deref() == Some(&row.path);
                        let is_active = active == Some(row.path.as_path());
                        ui::row_bg(ui, theme, rect, id, resp.hovered(), selected || is_active);

                        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 8.0));
                        for level in 0..row.depth {
                            let x = rect.left() + 13.0 + level as f32 * INDENT;
                            painter.vline(x, rect.y_range(), Stroke::new(1.0, ui::with_alpha(theme.border, 0.7 * fade)));
                        }
                        let x = rect.left() + 12.0 + row.depth as f32 * INDENT - (1.0 - fade) * 6.0;
                        let y = rect.center().y;
                        let state = git.state(&row.path);
                        let color = ui::with_alpha(state_color(theme, state), fade);
                        let dim = ui::with_alpha(theme.text_dim, fade);
                        let open = self.expanded.contains(&row.path);
                        if row.is_dir {
                            ui::caret(&painter, pos2(x, y), ui::anim(ui, id.with("caret"), open), dim);
                        }
                        let icon = match (row.is_dir, open) {
                            (true, true) => icons::FOLDER_OPEN,
                            (true, false) => icons::FOLDER,
                            _ => file_icon(&row.name),
                        };
                        let icon_color = if row.is_dir { ui::with_alpha(theme.accent, fade * 0.85) } else { dim };
                        painter.text(pos2(x + 10.0, y), Align2::LEFT_CENTER, icon, FontId::proportional(15.0), icon_color);

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

                        painter.text(pos2(x + 31.0, y), Align2::LEFT_CENTER, &row.name, FontId::proportional(13.0), color);
                        let badge = state_badge(state);
                        if row.is_dir && !badge.is_empty() {
                            painter.circle_filled(pos2(rect.right() - 14.0, y), 3.0, ui::with_alpha(color, 0.8));
                        } else if !badge.is_empty() {
                            painter.text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);
                        }

                        if !interactive { continue; }
                        if resp.double_clicked() && !row.is_dir {
                            cmds.push(Command::KeepEditor);
                        }
                        if resp.clicked() {
                            bg.request_focus();
                            *action = Some(if row.is_dir { Action::Toggle(row.path.clone()) } else { Action::Open(row.path.clone()) });
                        }
                        if resp.secondary_clicked() {
                            self.selected = Some(row.path.clone());
                        }
                        resp.context_menu(|ui| self.menu(ui, Some(&row.path), action));
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
    }

    fn overlays(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        if let Some(error) = self.error.clone() {
            let rect = Rect::from_min_max(pos2(ui.max_rect().left(), ui.max_rect().bottom() - 30.0), ui.max_rect().max);
            ui.painter().rect_filled(rect, 0, ui::mix(theme.panel, theme.error, 0.18));
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 3.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                if ui::icon_button(ui, theme, icons::X, 20.0, false, "Dismiss").clicked() {
                    self.error = None;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&error).color(theme.error).size(12.0)).truncate()).on_hover_text(&error);
                });
            });
        }
        if let Some(path) = self.confirm_delete.clone() {
            let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
            let mut choice = None;
            let modal = ui::modal(ui.ctx(), "explorer_delete").show(ui.ctx(), |ui| {
                ui::outline(ui);
                ui.set_width(340.0);
                ui.heading("Delete");
                ui.label(format!("Move \"{name}\" to the recycle bin?"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Move to Recycle Bin").color(theme.deleted)).clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if modal.should_close() {
                choice = Some(false);
            }
            if let Some(delete) = choice {
                self.confirm_delete = None;
                if delete {
                    match trash::delete(&path) {
                        Ok(()) => {
                            self.selected = None;
                            cmds.push(Command::PathChanged(path, None));
                        }
                        Err(e) => self.error = Some(format!("Could not delete \"{name}\": {e}")),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_rows_are_visual_only_and_reverse_in_place() {
        let mut explorer = Explorer::new();
        let root = PathBuf::from("root");
        let dir = root.join("src");
        let entry = |parent: &Path, name: &str, is_dir| Entry { path: parent.join(name), name: name.into(), is_dir };
        explorer.root = Some(root.clone());
        explorer.children.insert(root.clone(), vec![entry(&root, "src", true), entry(&root, "README", false)]);
        explorer.children.insert(dir.clone(), vec![entry(&dir, "main.rs", false)]);
        explorer.expand(&dir);
        explorer.build();

        explorer.toggle_folders(Some(dir.clone()), 0.0);
        assert_eq!(explorer.rows.len(), 2);
        let closing = explorer.animation.as_ref().unwrap();
        assert_eq!(closing.outgoing[1].path, dir.join("main.rs"));
        assert_eq!(closing.spans, [(0..1, false), (1..2, true), (2..3, false)]);
        assert_eq!(closing.progress(FOLDER_TIME), 0.0);

        let halfway = FOLDER_TIME / 2.0;
        explorer.toggle_folders(Some(dir.clone()), halfway);
        assert_eq!(explorer.rows.len(), 3);
        assert_eq!(explorer.animation.as_ref().unwrap().progress(halfway), 0.5);
        explorer.selected = Some(dir.join("main.rs"));
        explorer.toggle_folders(None, 2.0 * FOLDER_TIME);
        assert_eq!(explorer.selected, Some(dir));
        assert_eq!(explorer.rows.len(), 2);
        assert!(explorer.expanded.is_empty());
    }

    #[test]
    fn sorts_loads_lazily_and_keeps_expansion_on_refresh() {
        let dir = std::env::temp_dir().join(format!("halo-explorer-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        for file in ["b.txt", "A.txt", "src/main.rs", "src/deep/x.rs"] {
            std::fs::write(dir.join(file), "").unwrap();
        }
        let mut explorer = Explorer::new();
        explorer.set_root(Some(dir.clone()));
        explorer.build();
        let names: Vec<&str> = explorer.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["src", "A.txt", "b.txt"]);
        assert_eq!(explorer.children.len(), 1);

        explorer.reveal_active(&dir.join("src/deep/x.rs"));
        std::fs::write(dir.join("src/new.rs"), "").unwrap();
        explorer.refresh();
        explorer.build();
        let names: Vec<&str> = explorer.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["src", "deep", "x.rs", "main.rs", "new.rs", "A.txt", "b.txt"]);

        let mut cmds = vec![];
        explorer.start_edit(EditKind::NewFile, dir.clone());
        explorer.edit.as_mut().unwrap().text = "A.txt".into();
        explorer.commit_edit(&mut cmds);
        assert!(explorer.error.is_some() && cmds.is_empty());
        explorer.edit.as_mut().unwrap().text = "made.txt".into();
        explorer.commit_edit(&mut cmds);
        assert!(dir.join("made.txt").is_file());
        assert_eq!(cmds[0], Command::OpenPath(dir.join("made.txt")));

        assert!(validate("a/b").is_err() && validate("").is_err() && validate("ok.rs").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
