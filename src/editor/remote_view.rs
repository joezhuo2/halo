use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Align2, CornerRadius, FontId, Key, Layout, Rect, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Debug, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub reference: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub dir: bool,
}

enum Msg {
    Tree(String, Vec<Entry>, bool),
    File(String, Result<String, String>),
    Failed(String),
}

pub fn parse_repo(input: &str) -> Option<Repo> {
    let text = input.trim().trim_end_matches('/');
    let text = ["https://", "http://", "git@"].iter().fold(text, |t, p| t.strip_prefix(p).unwrap_or(t));
    let text = ["github.com/", "github.com:", "www.github.com/"].iter().fold(text, |t, p| t.strip_prefix(p).unwrap_or(t));
    let (text, at) = text.split_once('@').map_or((text, None), |(t, r)| (t, Some(r.to_owned())));
    let mut parts = text.splitn(4, '/');
    let owner = parts.next().filter(|s| !s.is_empty())?.to_owned();
    let name = parts.next().filter(|s| !s.is_empty())?.trim_end_matches(".git").to_owned();
    let reference = match (parts.next(), parts.next()) {
        (Some("tree" | "blob"), Some(rest)) => Some(rest.to_owned()),
        _ => at,
    };
    let valid = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (valid(&owner) && valid(&name)).then_some(Repo { owner, name, reference: reference.filter(|r| !r.is_empty()) })
}

pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn entries(tree: &Value) -> (Vec<Entry>, bool) {
    let list = tree["tree"].as_array().into_iter().flatten();
    let mut out: Vec<Entry> = list
        .filter_map(|e| Some(Entry { path: e["path"].as_str()?.to_owned(), dir: e["type"].as_str()? == "tree" }))
        .collect();
    let key = |e: &Entry| {
        let parts: Vec<&str> = e.path.split('/').collect();
        let last = parts.len() - 1;
        parts.iter().enumerate().map(|(i, p)| (i == last && !e.dir, p.to_lowercase())).collect::<Vec<_>>()
    };
    out.sort_by_cached_key(key);
    (out, tree["truncated"].as_bool().unwrap_or(false))
}

fn gh(args: &[&str]) -> Result<String, String> {
    crate::git::command("gh", &std::env::temp_dir(), args).map_err(|e| match e.starts_with("Could not run") {
        true => "The GitHub CLI (gh) is needed. Install it and run gh auth login.".to_owned(),
        false => e.lines().next().unwrap_or_default().to_owned(),
    })
}

fn load_tree(repo: &Repo) -> Result<(String, Vec<Entry>, bool), String> {
    let base = format!("repos/{}/{}", repo.owner, repo.name);
    let reference = match &repo.reference {
        Some(r) => r.clone(),
        None => gh(&["api", &base, "--jq", ".default_branch"])?.trim().to_owned(),
    };
    let text = gh(&["api", &format!("{base}/git/trees/{}?recursive=1", encode(&reference))])?;
    let tree: Value = serde_json::from_str(&text).map_err(|e| format!("GitHub sent an unexpected reply: {e}"))?;
    let (list, truncated) = entries(&tree);
    Ok((reference, list, truncated))
}

pub struct RemoteView {
    input: String,
    repo: Option<Repo>,
    reference: String,
    entries: Vec<Entry>,
    truncated: bool,
    open: HashSet<String>,
    filter: String,
    fetching: HashSet<String>,
    loading: bool,
    error: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    focus: bool,
}

impl RemoteView {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let empty = HashSet::new();
        Self { input: String::new(), repo: None, reference: String::new(), entries: vec![], truncated: false, open: empty.clone(), filter: String::new(), fetching: empty, loading: false, error: None, tx, rx, focus: true }
    }

    fn submit(&mut self, ctx: &egui::Context) {
        let Some(repo) = parse_repo(&self.input) else {
            self.error = Some("Type a repository as owner/name, optionally with @branch, or paste its GitHub URL.".into());
            return;
        };
        (self.repo, self.loading, self.error, self.entries, self.open) = (Some(repo.clone()), true, None, vec![], HashSet::new());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(match load_tree(&repo) {
                Ok((reference, list, truncated)) => Msg::Tree(reference, list, truncated),
                Err(e) => Msg::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    fn fetch(&mut self, path: &str, ctx: &egui::Context) {
        let Some(repo) = self.repo.clone() else { return };
        if !self.fetching.insert(path.to_owned()) {
            return;
        }
        let (tx, ctx, path, reference) = (self.tx.clone(), ctx.clone(), path.to_owned(), self.reference.clone());
        std::thread::spawn(move || {
            let url = format!("repos/{}/{}/contents/{}?ref={}", repo.owner, repo.name, encode(&path), encode(&reference));
            let text = gh(&["api", "-H", "Accept: application/vnd.github.raw", &url]);
            let text = text.and_then(|t| if t.contains('\0') || t.contains('\u{FFFD}') { Err("This is a binary file.".to_owned()) } else { Ok(t) });
            let _ = tx.send(Msg::File(path, text));
            ctx.request_repaint();
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, font_size: f32, cmds: &mut Vec<Command>) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        let ctx = ui.ctx().clone();
        for msg in self.rx.try_iter().collect::<Vec<_>>() {
            match msg {
                Msg::Tree(reference, list, truncated) => (self.reference, self.entries, self.truncated, self.loading) = (reference, list, truncated, false),
                Msg::Failed(e) => (self.error, self.loading) = (Some(e), false),
                Msg::File(path, Ok(text)) => {
                    self.fetching.remove(&path);
                    let name = path.rsplit('/').next().unwrap_or(&path);
                    let repo = self.repo.as_ref().map_or(String::new(), |r| format!("{}/{}@{}", r.owner, r.name, self.reference));
                    cmds.push(Command::OpenText(format!("{name} ({repo})"), text, PathBuf::from(&path)));
                }
                Msg::File(path, Err(e)) => {
                    self.fetching.remove(&path);
                    ui::notify_error(format!("Could not open {path}: {e}"));
                }
            }
        }
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.panel);
        ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, theme.border));
        ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 5.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
            let resp = ui.add(TextEdit::singleline(&mut self.input).hint_text("owner/name, owner/name@branch or a GitHub URL").desired_width((ui.available_width() - 260.0).max(160.0)));
            if std::mem::take(&mut self.focus) {
                resp.request_focus();
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.add_enabled(!self.loading, egui::Button::new("Open")).clicked() || enter) && !self.loading {
                self.submit(&ctx);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(TextEdit::singleline(&mut self.filter).hint_text("Filter files").desired_width(160.0));
            });
        });
        let body = rect.with_min_y(bar.bottom());
        let message = |text: &str| {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), theme.text_dim);
        };
        if self.loading {
            return ui::spinner(ui, body.center(), 10.0, theme.text_dim);
        }
        if let Some(error) = &self.error {
            return message(error);
        }
        if self.repo.is_none() {
            return message("Browse a GitHub repository read-only, without cloning it. Files load when you open them.");
        }
        let needle = self.filter.to_lowercase();
        let shown: Vec<&Entry> = match needle.is_empty() {
            true => self.entries.iter().filter(|e| ancestors(&e.path).all(|a| self.open.contains(a))).collect(),
            false => self.entries.iter().filter(|e| !e.dir && e.path.to_lowercase().contains(&needle)).collect(),
        };
        let row_h = 22.0;
        let font = FontId::proportional(font_size.min(13.0));
        let status = Rect::from_min_size(body.min, vec2(body.width(), 24.0));
        let count = self.entries.iter().filter(|e| !e.dir).count();
        let note = if self.truncated { "  ·  GitHub cut the listing short; some files are missing" } else { "" };
        let repo = self.repo.as_ref().map_or(String::new(), |r| format!("{}/{}", r.owner, r.name));
        ui.painter().text(status.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, format!("{repo} @ {}  ·  {count} files  ·  read-only{note}", self.reference), FontId::proportional(12.0), theme.text_dim);
        let mut clicked = None;
        ui.scope_builder(UiBuilder::new().max_rect(body.with_min_y(status.bottom())), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ScrollArea::vertical().id_salt(("remote_view", &repo)).auto_shrink(false).show_rows(ui, row_h, shown.len(), |ui, range| {
                for entry in &shown[range] {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    ui::row_bg(ui, theme, r, resp.id, resp.hovered(), false);
                    let depth = if needle.is_empty() { entry.path.matches('/').count() } else { 0 };
                    let x = r.left() + 12.0 + depth as f32 * 14.0;
                    let icon = match (entry.dir, self.open.contains(&entry.path)) {
                        (true, true) => icons::CARET_DOWN,
                        (true, false) => icons::CARET_RIGHT,
                        _ if self.fetching.contains(&entry.path) => icons::HOURGLASS,
                        _ => icons::FILE,
                    };
                    let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                    painter.text(pos2(x, r.center().y), Align2::LEFT_CENTER, icon, font.clone(), theme.text_dim);
                    let label = if needle.is_empty() { entry.path.rsplit('/').next().unwrap_or(&entry.path) } else { &entry.path };
                    painter.text(pos2(x + 18.0, r.center().y), Align2::LEFT_CENTER, label, font.clone(), theme.text);
                    if resp.clicked() {
                        clicked = Some((*entry).clone());
                    }
                }
            });
        });
        match clicked {
            Some(Entry { path, dir: true }) if !self.open.remove(&path) => drop(self.open.insert(path)),
            Some(Entry { path, dir: false }) => self.fetch(&path, &ctx),
            _ => {}
        }
    }
}

fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(at, _)| &path[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_repositories_from_names_and_urls() {
        let repo = |owner: &str, name: &str, reference: Option<&str>| Some(Repo { owner: owner.into(), name: name.into(), reference: reference.map(Into::into) });
        assert_eq!(parse_repo("rust-lang/rust"), repo("rust-lang", "rust", None));
        assert_eq!(parse_repo("rust-lang/rust@beta"), repo("rust-lang", "rust", Some("beta")));
        assert_eq!(parse_repo("https://github.com/a/b.git"), repo("a", "b", None));
        assert_eq!(parse_repo("github.com/a/b/tree/feature/x"), repo("a", "b", Some("feature/x")));
        assert_eq!(parse_repo("git@github.com:a/b.git"), repo("a", "b", None));
        assert_eq!(parse_repo("just-a-name"), None);
        assert_eq!(parse_repo("a/b c"), None);
        assert_eq!(encode("src/a b+c.rs"), "src/a%20b%2Bc.rs");
    }

    #[test]
    fn sorts_folders_before_files_at_every_level() {
        let tree = serde_json::json!({"truncated": false, "tree": [
            {"path": "README.md", "type": "blob"}, {"path": "src", "type": "tree"}, {"path": "src/z.rs", "type": "blob"},
            {"path": "src/a", "type": "tree"}, {"path": "src/a/m.rs", "type": "blob"}, {"path": "Cargo.toml", "type": "blob"}]});
        let (list, truncated) = entries(&tree);
        let paths: Vec<&str> = list.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["src", "src/a", "src/a/m.rs", "src/z.rs", "Cargo.toml", "README.md"]);
        assert!(!truncated);
        assert_eq!(ancestors("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
    }
}
