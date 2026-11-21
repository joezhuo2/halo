use crate::command::{self, Command};
use crate::editor::Editors;
use crate::git;
use crate::lsp;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, CornerRadius, FontId, Frame, Id, Key, Modifiers, Order, Rect, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

const ROW: f32 = 26.0;
const SHOWN: usize = 60;

#[derive(Clone, Debug, PartialEq)]
enum Item {
    File(String, usize),
    Command(Command),
    Hit(String, usize, String),
    Symbol(usize, String),
    Workspace(lsp::Symbol),
    Line(usize, usize),
    Note(&'static str),
}

#[derive(Default)]
pub struct Search {
    open: bool,
    focus: bool,
    query: String,
    selected: usize,
    reveal: bool,
    files: Option<(PathBuf, Vec<String>)>,
    files_rx: Option<Receiver<(PathBuf, Vec<String>)>>,
    hits: (String, Vec<Item>),
    hits_rx: Option<(String, Receiver<Vec<Item>>)>,
    symbols: Option<Vec<(usize, String)>>,
    globals: Vec<Item>,
    asked: Option<String>,
    waiting: Vec<u64>,
    servers: bool,
    items: (String, Vec<Item>),
    stale: bool,
    opened: u32,
}

pub(crate) fn score(query: &str, text: &str) -> Option<i32> {
    let chars: Vec<char> = text.chars().collect();
    let (mut at, mut total) = (0, 0);
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let found = (at..chars.len()).find(|&i| chars[i].to_lowercase().eq(q.to_lowercase()))?;
        if found == at && at > 0 {
            total += 8;
        }
        if found == 0 || !chars[found - 1].is_alphanumeric() {
            total += 6;
        }
        total -= (found - at).min(4) as i32;
        at = found + 1;
    }
    Some(total - chars.len() as i32 / 8)
}

fn score_path(query: &str, path: &str) -> Option<i32> {
    let name = path.rsplit('/').next().unwrap_or(path);
    score(query, name).map(|s| s + 20).max(score(query, path))
}

fn rank<T>(query: &str, items: impl Iterator<Item = T>, text: impl Fn(&T) -> Option<i32>) -> Vec<T> {
    let mut scored: Vec<(i32, T)> = items.filter_map(|item| Some((text(&item)?, item))).collect();
    if !query.trim().is_empty() {
        scored.sort_by_key(|(score, _)| -score);
    }
    scored.into_iter().take(SHOWN).map(|(_, item)| item).collect()
}

pub(crate) fn list_files(folder: &Path) -> Vec<String> {
    if let Some(text) = git::run(folder, &["ls-files", "-co", "--exclude-standard"]) {
        return text.lines().map(str::to_owned).collect();
    }
    let mut out = vec![];
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
            if path.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && name != "target" {
                    stack.push(path);
                }
            } else if let Ok(rel) = path.strip_prefix(folder) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        if out.len() > 50_000 {
            break;
        }
    }
    out.sort();
    out
}

fn parse_grep(text: &str) -> Vec<Item> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\0');
            let (path, number, text) = (parts.next()?, parts.next()?.parse().ok()?, parts.next()?);
            Some(Item::Hit(path.to_owned(), number, text.trim().chars().take(200).collect()))
        })
        .take(SHOWN * 4)
        .collect()
}

fn grep(folder: &Path, needle: &str) -> Vec<Item> {
    let args = ["grep", "--no-index", "--exclude-standard", "-n", "-I", "-i", "-F", "-z", "-e", needle];
    parse_grep(&git::run(folder, &args).unwrap_or_default())
}

fn line_suffix(query: &str) -> (&str, usize) {
    match query.rsplit_once(':') {
        Some((head, line)) if !head.is_empty() => line.parse().map_or((query, 0), |line| (head, line)),
        _ => (query, 0),
    }
}

impl Search {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, prefix: &str) {
        self.open = true;
        self.focus = true;
        self.query = prefix.to_owned();
        self.selected = 0;
        self.files = None;
        self.symbols = None;
        self.asked = None;
        self.waiting.clear();
        self.stale = true;
        self.opened += 1;
    }

    fn load_globals(&mut self, editors: &mut Editors, ctx: &egui::Context) {
        let Some(query) = self.query.strip_prefix('#').map(|q| q.trim().to_owned()) else { return };
        self.servers = editors.lsp.status().is_some();
        if self.asked.as_deref() != Some(query.as_str()) && self.waiting.is_empty() {
            self.waiting = editors.lsp.workspace_symbols(&query);
            self.asked = Some(query);
            self.globals.clear();
            self.stale = true;
        }
        let before = self.waiting.len();
        let mut replies = vec![];
        self.waiting.retain(|id| editors.lsp.take(*id).map(|reply| replies.push(reply)).is_none());
        for reply in replies.into_iter().flatten() {
            let found = lsp::symbols(&reply).into_iter().filter(|symbol| symbol.path.is_some());
            self.globals.extend(found.take(SHOWN * 3).map(Item::Workspace));
        }
        self.stale |= before != self.waiting.len();
        if !self.waiting.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
    }

    fn load(&mut self, folder: Option<&Path>, editors: &mut Editors, ctx: &egui::Context) {
        self.load_globals(editors, ctx);
        if let Some((folder, files)) = self.files_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.files = Some((folder, files));
            self.files_rx = None;
            self.stale = true;
        }
        if let Some((query, rx)) = &self.hits_rx
            && let Ok(hits) = rx.try_recv()
        {
            self.hits = (query.clone(), hits);
            self.hits_rx = None;
            self.stale = true;
        }
        let Some(folder) = folder else { return };
        let mode = self.query.chars().next();
        let listed = self.files.as_ref().is_some_and(|(f, _)| f == folder);
        if !listed && self.files_rx.is_none() && !matches!(mode, Some('>' | '%' | '@' | '#' | ':')) {
            let (tx, rx) = channel();
            let (folder, ctx) = (folder.to_path_buf(), ctx.clone());
            std::thread::spawn(move || {
                let files = list_files(&folder);
                let _ = tx.send((folder, files));
                ctx.request_repaint();
            });
            self.files_rx = Some(rx);
        }
        let needle = self.query.strip_prefix('%').map(str::trim).unwrap_or_default();
        if needle.chars().count() >= 2 && self.hits.0 != needle && self.hits_rx.is_none() {
            let (tx, rx) = channel();
            let (folder, ctx, text) = (folder.to_path_buf(), ctx.clone(), needle.to_owned());
            std::thread::spawn(move || {
                let _ = tx.send(grep(&folder, &text));
                ctx.request_repaint();
            });
            self.hits_rx = Some((needle.to_owned(), rx));
        }
        if mode == Some('@') && self.symbols.is_none() {
            self.symbols = Some(editors.symbols());
            self.stale = true;
        }
    }

    fn compute(&self, folder: Option<&Path>) -> Vec<Item> {
        let query = self.query.as_str();
        let none = |items: Vec<Item>, note| if items.is_empty() { vec![Item::Note(note)] } else { items };
        if let Some(q) = query.strip_prefix('>') {
            return none(rank(q, command::all().into_iter().map(Item::Command), |i| match i {
                Item::Command(c) => score(q, c.label()),
                _ => None,
            }), "No matching commands");
        }
        if let Some(q) = query.strip_prefix('%') {
            let q = q.trim();
            return match (folder, q.chars().count() >= 2) {
                (None, _) => vec![Item::Note("Open a folder to search text in files")],
                (_, false) => vec![Item::Note("Type at least two characters to search text in files")],
                _ if self.hits.0 != q => vec![Item::Note("Searching...")],
                _ => none(self.hits.1.clone(), "No results"),
            };
        }
        if let Some(q) = query.strip_prefix('@') {
            let symbols = self.symbols.iter().flatten().map(|(line, name)| Item::Symbol(*line, name.clone()));
            return none(rank(q, symbols, |i| match i {
                Item::Symbol(_, name) => score(q, name),
                _ => None,
            }), "No symbols found in the active editor");
        }
        if query.starts_with('#') {
            return match (self.servers, self.waiting.is_empty()) {
                (false, _) => vec![Item::Note("No language server is running yet. Open a source file of the project first")],
                (_, false) if self.globals.is_empty() => vec![Item::Note("Searching...")],
                _ => none(self.globals.clone(), "No symbols found"),
            };
        }
        if let Some(q) = query.strip_prefix(':') {
            let mut parts = q.split(':').map(|p| p.trim().parse::<usize>().ok());
            return match parts.next().flatten() {
                Some(line) => vec![Item::Line(line, parts.next().flatten().unwrap_or(1))],
                None => vec![Item::Note("Type a line number, optionally :column")],
            };
        }
        let (q, line) = line_suffix(query);
        match (&self.files, folder) {
            (_, None) => vec![Item::Note("Open a folder to search files. > commands, @ symbols, : line")],
            (Some((listed, files)), Some(folder)) if listed == folder => none(
                rank(q, files.iter().map(|f| Item::File(f.clone(), line)), |i| match i {
                    Item::File(path, _) => score_path(q, path),
                    _ => None,
                }),
                "No matching files",
            ),
            _ => vec![Item::Note("Listing files...")],
        }
    }

    fn accept(&mut self, item: &Item, folder: Option<&Path>, cmds: &mut Vec<Command>) {
        let full = |rel: &str| folder.map(|f| f.join(rel));
        match item {
            Item::File(rel, line) => {
                cmds.extend(full(rel).map(Command::OpenPath));
                if *line > 0 {
                    cmds.push(Command::GoTo(*line, 1));
                }
            }
            Item::Hit(rel, line, _) => {
                cmds.extend(full(rel).map(Command::OpenPath));
                cmds.push(Command::GoTo(*line, 1));
            }
            Item::Command(command) => cmds.push(command.clone()),
            Item::Symbol(line, _) => cmds.push(Command::GoTo(line + 1, 1)),
            Item::Workspace(symbol) => {
                cmds.extend(symbol.path.clone().map(Command::OpenPath));
                cmds.push(Command::GoTo(symbol.line + 1, symbol.col + 1));
            }
            Item::Line(line, column) => cmds.push(Command::GoTo(*line, *column)),
            Item::Note(_) => return,
        }
        self.open = false;
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, folder: Option<&Path>, editors: &mut Editors, cmds: &mut Vec<Command>) -> bool {
        let ctx = ui.ctx().clone();
        self.load(folder, editors, &ctx);
        let typed = self.items.0 != self.query;
        if typed || std::mem::take(&mut self.stale) {
            self.items = (self.query.clone(), self.compute(folder));
            self.selected = if typed { 0 } else { self.selected.min(self.items.1.len().saturating_sub(1)) };
            ctx.request_repaint();
        }
        let count = self.items.1.len();
        let (down, up, escape) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if (down || up) && count > 0 {
            self.selected = (self.selected + if down { 1 } else { count - 1 }) % count;
            self.reveal = true;
        }

        ui.painter().rect(rect, CornerRadius::same(6), theme.raised, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
        let hint = "Search files (> commands, % text, @ symbols, # workspace symbols, : line)";
        let edit = TextEdit::singleline(&mut self.query).hint_text(hint).frame(Frame::NONE).desired_width(f32::INFINITY);
        let resp = ui.put(rect.shrink2(vec2(8.0, 3.0)), edit);
        if std::mem::take(&mut self.focus) {
            resp.request_focus();
        }
        let mut accepted = (resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))).then_some(self.selected);

        let t = ui::fade_in(ui, Id::new(("search_fade", self.opened)));
        let area = egui::Area::new(Id::new("search_results")).order(Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, 4.0)).show(&ctx, |ui| {
            ui.set_opacity(t);
            Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.border)).corner_radius(CornerRadius::same(6)).show(ui, |ui| {
                ui.set_width(rect.width());
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                ScrollArea::vertical().max_height(ROW * 14.0).show(ui, |ui| {
                    for (index, item) in self.items.1.iter().enumerate() {
                        let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                        let note = matches!(item, Item::Note(_));
                        ui::row_bg(ui, theme, r, row.id, row.hovered() && !note, index == self.selected && !note);
                        let (icon, label, detail) = match item {
                            Item::File(path, _) => {
                                let (dir, name) = path.rsplit_once('/').map_or(("", path.as_str()), |(d, n)| (d, n));
                                (icons::FILE, name.to_owned(), dir.to_owned())
                            }
                            Item::Command(c) => (icons::CARET_RIGHT, c.label().to_owned(), command::shortcut_text(&ctx, c).unwrap_or_default()),
                            Item::Hit(path, line, text) => (icons::TEXT_ALIGN_LEFT, text.clone(), format!("{path}:{line}")),
                            Item::Symbol(line, name) => (icons::CUBE, name.clone(), format!("line {}", line + 1)),
                            Item::Workspace(symbol) => {
                                let file = symbol.path.as_deref().and_then(Path::file_name).unwrap_or_default().to_string_lossy();
                                (lsp::symbol_icon(symbol.kind), symbol.name.clone(), format!("{}  {file}:{}", symbol.container, symbol.line + 1))
                            }
                            Item::Line(line, column) => (icons::ARROW_RIGHT, format!("Go to line {line}, column {column}"), String::new()),
                            Item::Note(text) => (icons::INFO, (*text).to_owned(), String::new()),
                        };
                        let painter = ui.painter().with_clip_rect(r.shrink2(vec2(8.0, 0.0)));
                        let color = if note { theme.text_dim } else { theme.text };
                        painter.text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(14.0), theme.text_dim);
                        let end = painter.text(r.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(12.5), color);
                        painter.text(egui::pos2(end.right() + 10.0, r.center().y), Align2::LEFT_CENTER, detail, FontId::proportional(11.5), theme.text_dim);
                        if index == self.selected && std::mem::take(&mut self.reveal) {
                            ui.scroll_to_rect(r, None);
                        }
                        if row.clicked() {
                            accepted = Some(index);
                        }
                    }
                });
            });
        });

        if let Some(item) = accepted.and_then(|index| self.items.1.get(index).cloned()) {
            self.accept(&item, folder, cmds);
            self.focus = true;
            return false;
        }
        if resp.lost_focus() && area.response.contains_pointer() {
            self.focus = true;
        } else if resp.lost_focus() || escape {
            self.open = false;
        }
        escape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_parses_and_picks_modes() {
        assert!(score("xyz", "src/app.rs").is_none());
        assert!(score_path("app", "src/app.rs") > score_path("app", "docs/a/p/p.md"));
        assert!(score_path("edmod", "src/editor/mod.rs").is_some());
        assert_eq!(line_suffix("app.rs:42"), ("app.rs", 42));
        assert_eq!(line_suffix("app.rs"), ("app.rs", 0));
        assert_eq!(parse_grep("src/a.rs\x0012\0  let x = 1;\nbad line\n"), vec![Item::Hit("src/a.rs".into(), 12, "let x = 1;".into())]);

        let mut search = Search::default();
        search.open(">toggle ai");
        assert_eq!(search.compute(None)[0], Item::Command(Command::ToggleAi));
        search.query = ":12:4".into();
        assert_eq!(search.compute(None), vec![Item::Line(12, 4)]);
        let folder = Path::new("x");
        search.files = Some((folder.into(), vec!["src/app.rs".into(), "README.md".into()]));
        search.query = "app:7".into();
        let found = search.compute(Some(folder));
        assert_eq!(found, vec![Item::File("src/app.rs".into(), 7)]);
        let mut cmds = vec![];
        search.accept(&found[0], Some(folder), &mut cmds);
        assert_eq!(cmds, vec![Command::OpenPath(folder.join("src/app.rs")), Command::GoTo(7, 1)]);
        assert!(!search.is_open());

        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(list_files(here).contains(&"src/search.rs".to_owned()));
        assert!(grep(here, "fn ranks_parses_and_picks_modes").iter().any(|i| matches!(i, Item::Hit(p, _, _) if p == "src/search.rs")));
    }
}
