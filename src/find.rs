use crate::command::Command;
use crate::editor::{Editors, Pattern};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align, Align2, Color32, FontId, Key, Layout, RichText, ScrollArea, Sense, TextEdit, Ui, UiBuilder, pos2, vec2};
use fancy_regex::Regex;
use crate::workspace::resolve;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

const ROW: f32 = 22.0;
pub(crate) const MAX_HITS: usize = 5000;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub(crate) line: usize,
    pub(crate) col: usize,
    text: String,
    mark: (usize, usize),
}

struct FileHits {
    path: String,
    hits: Vec<Hit>,
    open: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Query {
    pub(crate) text: String,
    pub(crate) case: bool,
    pub(crate) word: bool,
    pub(crate) regex: bool,
    pub(crate) include: String,
    pub(crate) exclude: String,
}

#[derive(Default)]
pub struct FindPanel {
    query: Query,
    with: String,
    preserve: bool,
    replace: bool,
    focus: bool,
    searched: Query,
    results: Vec<FileHits>,
    note: String,
    rx: Option<Receiver<Vec<FileHits>>>,
    confirm: Option<Option<usize>>,
    focused: Option<u64>,
}

fn glob(text: &str) -> Option<Regex> {
    let text = text.trim().replace('\\', "/");
    let text = text.trim_start_matches("./").trim_matches('/');
    let mut source = String::from("(?:^|/)");
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                let dirs = chars.next_if_eq(&'/').is_some();
                source.push_str(if dirs { "(?:.*/)?" } else { ".*" });
            }
            '*' => source.push_str("[^/]*"),
            '?' => source.push_str("[^/]"),
            c => source.push_str(&fancy_regex::escape(c.encode_utf8(&mut [0; 4]))),
        }
    }
    Regex::new(&(source + "(?:/|$)")).ok().filter(|_| !text.is_empty())
}

fn globs(text: &str) -> Vec<Regex> {
    text.split(',').filter_map(glob).collect()
}

fn listed(globs: &[Regex], path: &str) -> bool {
    globs.iter().any(|glob| glob.is_match(path).unwrap_or(false))
}

fn scan(text: &str, pattern: &Pattern, limit: usize) -> Vec<Hit> {
    let (mut line, mut line_start, mut seen) = (1, 0, 0);
    let mut hits = vec![];
    for (from, to) in pattern.find(text).into_iter().take(limit) {
        for (at, _) in text[seen..from].match_indices('\n') {
            line += 1;
            line_start = seen + at + 1;
        }
        seen = from;
        let line_end = text[from..].find('\n').map_or(text.len(), |at| from + at);
        let whole = &text[line_start..from];
        let shown = line_start + (whole.len() - whole.trim_start().len());
        let before = text[shown..from].chars().count();
        let length = text[from..to.min(line_end)].chars().count();
        hits.push(Hit {
            line,
            col: whole.chars().count() + 1,
            text: text[shown..line_end].chars().take(240).collect(),
            mark: (before.min(240), (before + length).min(240)),
        });
    }
    hits
}

// shortcut: files are read from disk, so unsaved edits are not searched, upgrade to reading open buffers if that matters
fn search(roots: &[(PathBuf, String)], query: &Query) -> Vec<FileHits> {
    let mut out = vec![];
    each_file(roots, query, |path, _, hits| out.push(FileHits { path, hits, open: true }));
    out
}

pub(crate) fn each_file(roots: &[(PathBuf, String)], query: &Query, mut found: impl FnMut(String, &str, Vec<Hit>)) {
    let Ok(pattern) = Pattern::new(&query.text, query.case, query.word, query.regex) else { return };
    let (include, exclude) = (globs(&query.include), globs(&query.exclude));
    let mut total = 0;
    let files = roots.iter().flat_map(|(root, prefix)| crate::search::list_files(root).into_iter().map(move |p| (root.join(&p), format!("{prefix}{p}"))));
    for (full, path) in files {
        if (!include.is_empty() && !listed(&include, &path)) || listed(&exclude, &path) {
            continue;
        }
        let Ok(bytes) = std::fs::read(full) else { continue };
        if bytes.len() > 4 * 1024 * 1024 || bytes.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        let hits = scan(&text, &pattern, MAX_HITS - total);
        if !hits.is_empty() {
            total += hits.len();
            found(path, &text, hits);
        }
        if total >= MAX_HITS {
            break;
        }
    }
}

impl FindPanel {
    pub fn focus(&mut self) {
        self.focus = true;
    }

    fn start(&mut self, roots: &[(PathBuf, String)], ctx: &egui::Context) {
        self.searched = self.query.clone();
        self.results.clear();
        self.rx = None;
        self.note.clear();
        if self.query.text.is_empty() {
            return;
        }
        if let Err(error) = Pattern::new(&self.query.text, self.query.case, self.query.word, self.query.regex) {
            self.note = error;
            return;
        }
        let (tx, rx) = channel();
        let (roots, query, ctx) = (roots.to_vec(), self.query.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(search(&roots, &query));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    pub fn editor_query(&self, results: bool) -> Query {
        if results && !self.results.is_empty() { self.searched.clone() } else { self.query.clone() }
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.focused.is_some_and(|pass| pass + 1 >= ctx.cumulative_pass_nr())
    }

    fn run_replace(&mut self, target: Option<usize>, roots: &[(PathBuf, String)], editors: &mut Editors, cmds: &mut Vec<Command>, ctx: &egui::Context) {
        let q = &self.searched;
        let Ok(pattern) = Pattern::new(&q.text, q.case, q.word, q.regex) else { return };
        let files = self.results.iter().enumerate().filter(|(index, _)| target.is_none_or(|only| only == *index));
        let paths: Vec<PathBuf> = files.filter_map(|(_, file)| resolve(roots, &file.path)).collect();
        let failed: Vec<String> = paths.iter().filter_map(|path| editors.replace_in(path, &pattern, &self.with, self.preserve).err()).collect();
        if !failed.is_empty() {
            ui::notify(failed.join("\n"));
        }
        cmds.push(Command::RefreshExplorer);
        self.start(roots, ctx);
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, roots: &[(PathBuf, String)], editors: &mut Editors, cmds: &mut Vec<Command>) {
        let ctx = ui.ctx().clone();
        if let Some(found) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let (hits, files) = (found.iter().map(|file| file.hits.len()).sum::<usize>(), found.len());
            self.note = match hits {
                0 => "No results".to_owned(),
                n if n >= MAX_HITS => format!("First {n} results in {files} files"),
                n => format!("{n} results in {files} files"),
            };
            self.results = found;
            self.rx = None;
        }
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        ui.painter().rect_filled(rect, 0, ui::mix(theme.panel, theme.bar, 0.5));
        ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, "SEARCH", FontId::proportional(11.5), theme.text);
        if roots.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("   Open a folder to search in files.").color(theme.text_dim));
            return;
        }
        let mut go = false;
        let body = ui.available_rect_before_wrap().shrink2(vec2(8.0, 6.0));
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 5.0);
            let width = ui.available_width() - 16.0;
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let (pass, mut focused) = (ui.ctx().cumulative_pass_nr(), None);
            let mut field = |ui: &mut Ui, text: &mut String, hint: &str, focus: bool| {
                let resp = ui.add(TextEdit::singleline(text).margin(ui::FIELD).desired_width(width).hint_text(hint));
                focused = focused.or(resp.has_focus().then_some(pass));
                if focus || (resp.lost_focus() && enter) {
                    resp.request_focus();
                }
                resp.lost_focus() && enter
            };
            go |= field(ui, &mut self.query.text, "Search", std::mem::take(&mut self.focus));
            let (mut open, mut new, mut collapse, mut clear) = (false, false, false, false);
            ui.horizontal(|ui| {
                let q = &mut self.query;
                for (icon, value, tip) in [
                    (icons::TEXT_AA, &mut q.case, "Match Case"),
                    (icons::TEXT_UNDERLINE, &mut q.word, "Match Whole Word"),
                    (icons::ASTERISK, &mut q.regex, "Use Regular Expression"),
                    (icons::SWAP, &mut self.replace, "Toggle Replace"),
                ] {
                    if ui::icon_button(ui, theme, icon, 22.0, *value, tip).clicked() {
                        *value ^= true;
                        go |= icon != icons::SWAP && !q.text.is_empty();
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    clear = ui::icon_button(ui, theme, icons::X, 22.0, false, "Clear Results").clicked();
                    collapse = ui::icon_button(ui, theme, icons::ARROWS_IN_LINE_VERTICAL, 22.0, false, "Collapse All").clicked();
                    open = ui::icon_button(ui, theme, icons::ARROW_SQUARE_OUT, 22.0, false, "Open Results in Editor").clicked();
                    new = ui::icon_button(ui, theme, icons::FILE_PLUS, 22.0, false, "New Search Editor").clicked();
                });
            });
            if self.replace {
                go |= field(ui, &mut self.with, "Replace", false);
                ui.horizontal(|ui| {
                    if ui::icon_button(ui, theme, icons::TEXT_T, 22.0, self.preserve, "Preserve Case").clicked() {
                        self.preserve ^= true;
                    }
                    if ui.add_enabled(!self.results.is_empty(), egui::Button::new("Replace All")).clicked() {
                        self.confirm = Some(None);
                    }
                });
            }
            go |= field(ui, &mut self.query.include, "Files to include, like src, *.rs", false);
            go |= field(ui, &mut self.query.exclude, "Files to exclude", false);
            self.focused = focused;
            if self.rx.is_none() {
                ui.label(RichText::new(&self.note).size(12.0).color(theme.text_dim));
            }
            if clear {
                self.results.clear();
                self.note.clear();
            }
            if collapse {
                self.results.iter_mut().for_each(|file| file.open = false);
            }
            if open || new {
                cmds.push(if open { Command::OpenSearchEditor } else { Command::NewSearchEditor });
            }
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ScrollArea::vertical().id_salt("find_results").auto_shrink(false).show(ui, |ui| {
                if self.rx.is_some() && self.results.is_empty() {
                    let rows = ui.scope(|ui| ui::skeleton(ui, theme, 5)).response;
                    rows.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), "Searching"));
                }
                for (index, file) in self.results.iter_mut().enumerate() {
                    let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    ui::row_bg(ui, theme, r, row.id, row.hovered(), false);
                    let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
                    ui::tree_item(ui, &row, &format!("{name} {dir}, {} matches", file.hits.len()), false, 0, Some(file.open));
                    let painter = ui.painter().with_clip_rect(r.with_max_x(r.right() - 52.0));
                    let caret = if file.open { icons::CARET_DOWN } else { icons::CARET_RIGHT };
                    painter.text(pos2(r.left() + 8.0, r.center().y), Align2::CENTER_CENTER, caret, FontId::proportional(10.0), theme.text_dim);
                    let end = painter.text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.5), theme.text);
                    painter.text(pos2(end.right() + 8.0, r.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
                    let side = r.with_min_x(r.right() - 50.0);
                    if self.replace && ui.rect_contains_pointer(r) {
                        ui.scope_builder(UiBuilder::new().max_rect(side).layout(Layout::right_to_left(Align::Center)), |ui| {
                            if ui::icon_button(ui, theme, icons::SWAP, 20.0, false, "Replace All in This File").clicked() {
                                self.confirm = Some(Some(index));
                            }
                        });
                    } else {
                        ui.painter().text(side.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, file.hits.len(), FontId::proportional(11.5), theme.text_dim);
                    }
                    if row.clicked() {
                        file.open ^= true;
                    }
                    for hit in file.hits.iter().filter(|_| file.open) {
                        let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                        ui::row_bg(ui, theme, r, row.id, row.hovered(), false);
                        let byte = |col: usize| hit.text.char_indices().nth(col).map_or(hit.text.len(), |c| c.0);
                        ui::tree_item(ui, &row, hit.text.trim(), false, 1, None);
                        let (from, to) = (byte(hit.mark.0), byte(hit.mark.1));
                        let plain = TextFormat::simple(FontId::monospace(12.0), theme.text);
                        let found = TextFormat { background: ui::with_alpha(theme.accent, 0.3), ..plain.clone() };
                        let mut job = LayoutJob::default();
                        job.append(&hit.text[..from], 0.0, plain.clone());
                        job.append(&hit.text[from..to], 0.0, found);
                        job.append(&hit.text[to..], 0.0, plain);
                        let painter = ui.painter().with_clip_rect(r);
                        let galley = painter.layout_job(job);
                        painter.galley(pos2(r.left() + 26.0, r.center().y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
                        if row.on_hover_text(format!("{}:{}", file.path, hit.line)).clicked() {
                            cmds.extend(resolve(roots, &file.path).map(Command::OpenPath));
                            cmds.push(Command::GoTo(hit.line, hit.col));
                        }
                    }
                }
            });
        });
        if go {
            self.start(roots, &ctx);
        }
        let Some(target) = self.confirm else { return };
        let chosen = || self.results.iter().enumerate().filter(|(index, _)| target.is_none_or(|only| only == *index)).map(|(_, file)| file.hits.len());
        let about = format!(
            "Replace {} occurrences in {} files with \"{}\"? Files that are not open in the editor are changed on disk, and that cannot be undone.",
            chosen().sum::<usize>(),
            chosen().count(),
            self.with
        );
        let mut choice = None;
        let modal = ui::modal(&ctx, "find_replace", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(380.0);
            ui.heading("Replace in files");
            ui.add(egui::Label::new(about).wrap());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (label, value) in [("Replace", true), ("Cancel", false)] {
                    if keys.choice(ui, label, value) {
                        choice = Some(value);
                    }
                }
            });
        });
        if modal.should_close() {
            choice = Some(false);
        }
        if let Some(replace) = choice {
            self.confirm = None;
            if replace {
                self.run_replace(target, roots, editors, cmds, &ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_globs_scans_lines_and_replaces_on_disk() {
        let globs = globs("src, *.md,docs/**/*.txt ");
        let hit = |path: &str| listed(&globs, path);
        assert!(hit("src/a.rs") && hit("x/src/b.rs") && hit("README.md") && hit("docs/a/b/c.txt") && hit("docs/c.txt"));
        assert!(!hit("srcs/a.rs") && !hit("a.mdx") && !hit("docs/c.rs"));

        let pattern = Pattern::new("foo", true, false, false).unwrap();
        let hits = scan("a\n  x foo foo\nfoo", &pattern, 10);
        assert_eq!(hits[0], Hit { line: 2, col: 5, text: "x foo foo".into(), mark: (2, 5) });
        assert_eq!(hits.iter().map(|h| (h.line, h.col)).collect::<Vec<_>>(), vec![(2, 5), (2, 9), (3, 1)]);

        let dir = std::env::temp_dir().join(format!("halo-find-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), "one foo\r\nFoo two\r\n").unwrap();
        std::fs::write(dir.join("b.md"), "foo").unwrap();
        let query = Query { text: "foo".into(), include: "*.txt".into(), ..Default::default() };
        let found = search(&[(dir.clone(), String::new())], &query);
        assert_eq!((found.len(), found[0].path.as_str(), found[0].hits.len()), (1, "sub/a.txt", 2));
        let other = std::env::temp_dir().join(format!("halo-find-other-{}", std::process::id()));
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("c.txt"), "foo").unwrap();
        let roots = [(dir.clone(), "api/".to_owned()), (other.clone(), "web/".to_owned())];
        let found = search(&roots, &Query { text: "foo".into(), ..Default::default() });
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.starts_with(&["api/b.md"]) && paths.contains(&"api/sub/a.txt") && paths.last() == Some(&"web/c.txt"));
        let only = search(&roots, &Query { text: "foo".into(), include: "./web/**".into(), ..Default::default() });
        assert_eq!(only.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["web/c.txt"]);
        assert_eq!(resolve(&roots, "web/c.txt"), Some(other.join("c.txt")));
        let _ = std::fs::remove_dir_all(&other);
        let mut editors = Editors::new();
        let any = Pattern::new("foo", false, false, false).unwrap();
        assert_eq!(editors.replace_in(&dir.join("sub/a.txt"), &any, "bar", true), Ok(2));
        assert_eq!(std::fs::read_to_string(dir.join("sub/a.txt")).unwrap(), "one bar\r\nBar two\r\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
