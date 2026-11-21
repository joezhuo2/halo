#[path = "git_ops.rs"]
pub mod ops;

use eframe::egui;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileState {
    Added,
    Modified,
    Deleted,
    Conflicted,
    Ignored,
}

impl FileState {
    pub fn strength(self) -> u8 {
        match self {
            FileState::Ignored => 0,
            FileState::Added => 1,
            FileState::Modified => 2,
            FileState::Deleted => 3,
            FileState::Conflicted => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineChange {
    Added,
    Modified,
    Deleted,
}

pub type LineChanges = Arc<Vec<(usize, LineChange, bool)>>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub path: String,
    pub state: FileState,
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub refs: String,
    pub subject: String,
    pub col: usize,
    pub up: Vec<usize>,
    pub through: Vec<usize>,
    pub down: Vec<usize>,
}

#[derive(Clone, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub head_ref_name: String,
    pub mergeable: String,
    pub review_decision: String,
    pub is_draft: bool,
}

const PR_FIELDS: &str = "number,title,headRefName,mergeable,reviewDecision,isDraft";

#[derive(PartialEq, Eq, Debug)]
struct Entry {
    path: String,
    state: FileState,
    staged: Option<FileState>,
    unstaged: Option<FileState>,
}

struct Status {
    details: Option<Result<ops::Details, String>>,
    top: PathBuf,
    branch: String,
    tracking: Option<(usize, usize)>,
    branches: Vec<String>,
    log: Vec<Commit>,
    staged: Vec<Change>,
    unstaged: Vec<Change>,
    files: HashMap<String, FileState>,
    folders: HashMap<String, FileState>,
}

#[allow(clippy::large_enum_variant)]
enum Msg {
    Status(PathBuf, Option<Status>),
    Diff(PathBuf, Vec<(usize, LineChange, bool)>),
    Done(Option<PathBuf>, Result<Option<PathBuf>, String>),
    Repos(PathBuf, Vec<PathBuf>, bool),
    Files(String, Vec<Change>),
    Text(String, String),
    Prs(PathBuf, Result<Vec<Pr>, String>),
}

pub struct Git {
    folder: Option<PathBuf>,
    folder_repo: Option<bool>,
    repos: Vec<PathBuf>,
    followed: Option<PathBuf>,
    shown: Option<(String, Vec<Change>)>,
    texts: Vec<(String, String)>,
    prs: Option<Result<Vec<Pr>, String>>,
    prs_loading: bool,
    prs_stale: bool,
    root: Option<PathBuf>,
    ctx: Option<egui::Context>,
    status: Option<Status>,
    repo_name: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    running: bool,
    pending: bool,
    details_requested: bool,
    cloned: Option<PathBuf>,
    busy: bool,
    error: Option<String>,
    diffs: RefCell<HashMap<PathBuf, (Option<LineChanges>, bool)>>,
}

pub fn run(dir: &Path, args: &[&str]) -> Option<String> {
    exec(dir, args).ok()
}

pub fn exec(dir: &Path, args: &[&str]) -> Result<String, String> {
    command("git", dir, args)
}

pub fn tracked<T>(title: Option<String>, work: impl FnOnce(&dyn Fn(String)) -> Result<T, String>) -> Result<T, String> {
    let Some(title) = title else { return work(&|_| {}) };
    let progress = crate::ui::progress(title.clone());
    let result = work(&|text| progress.set(text, None));
    match &result {
        Ok(_) => progress.done(format!("{title} finished")),
        Err(error) => progress.failed(format!("{title} failed: {}", error.lines().next().unwrap_or_default())),
    }
    result
}

pub fn command(program: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (program, args) = crate::wsl::wrap(Some(dir), program, &args);
    let shown = format!("{program} {}", args.join(" ").replace('\n', " "));
    let mut command = std::process::Command::new(&program);
    command.args(args).current_dir(dir).env("GIT_TERMINAL_PROMPT", "0").env("WSLENV", "GIT_TERMINAL_PROMPT");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let started = std::time::Instant::now();
    let output = command.output().map_err(|e| {
        crate::output::log("Git", format!("$ {shown} could not run: {e}"));
        format!("Could not run {program}: {e}")
    })?;
    let exit = output.status.code().map_or("none".to_owned(), |code| code.to_string());
    crate::output::log("Git", format!("$ {shown} ({} ms) exit {exit}", started.elapsed().as_millis()));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    crate::output::log("Git", &stderr);
    Err(if stderr.is_empty() { stdout.trim().to_owned() } else { stderr })
}

fn key(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    if cfg!(windows) { text.to_lowercase() } else { text }
}

fn parse_branch(header: &str) -> Option<String> {
    let rest = header.strip_prefix("## ")?;
    if let Some(name) = rest.strip_prefix("No commits yet on ") {
        return Some(name.to_owned());
    }
    if rest.starts_with("HEAD (no branch)") {
        return None;
    }
    let name = rest.split("...").next()?.split(' ').next()?;
    Some(name.to_owned())
}

fn parse_tracking(header: &str) -> Option<(usize, usize)> {
    let rest = header.split_once("...")?.1;
    let count = |key: &str| {
        let digits = rest.split_once(key)?.1.split(|c: char| !c.is_ascii_digit()).next()?;
        digits.parse().ok()
    };
    Some((count("ahead ").unwrap_or(0), count("behind ").unwrap_or(0)))
}

fn parse_status(text: &str) -> (Option<String>, Option<(usize, usize)>, Vec<Entry>) {
    let mut branch = None;
    let mut tracking = None;
    let mut files = vec![];
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if entry.starts_with("## ") {
            branch = parse_branch(entry);
            tracking = parse_tracking(entry);
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry.as_bytes()[0], entry.as_bytes()[1]);
        let path = entry[3..].to_owned();
        if x == b'R' || x == b'C' || y == b'R' || y == b'C' {
            entries.next();
        }
        let state = match (x, y) {
            (b'?', b'?') => FileState::Added,
            (b'!', b'!') => FileState::Ignored,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileState::Conflicted,
            (b'D', _) | (_, b'D') => FileState::Deleted,
            (b'A', _) => FileState::Added,
            _ => FileState::Modified,
        };
        let side = |c: u8| match c {
            b' ' => None,
            b'A' => Some(FileState::Added),
            b'D' => Some(FileState::Deleted),
            _ => Some(FileState::Modified),
        };
        let (staged, unstaged) = match state {
            FileState::Ignored => (None, None),
            FileState::Conflicted => (None, Some(state)),
            _ if x == b'?' => (None, Some(state)),
            _ => (side(x), side(y)),
        };
        files.push(Entry { path, state, staged, unstaged });
    }
    (branch, tracking, files)
}

fn mark_staged(all: Vec<(usize, LineChange)>, unstaged: &[(usize, LineChange)]) -> Vec<(usize, LineChange, bool)> {
    all.into_iter().map(|(line, kind)| (line, kind, !unstaged.iter().any(|u| u.0 == line))).collect()
}

fn parse_diff(text: &str) -> Vec<(usize, LineChange)> {
    let mut out = vec![];
    for line in text.lines().filter(|l| l.starts_with("@@ ")) {
        let mut parts = line.split(' ');
        let (Some(old), Some(new)) = (parts.nth(1), parts.next()) else { continue };
        let range = |s: &str| {
            let mut it = s[1..].split(',');
            let start = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
            let count = it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
            (start, count)
        };
        let (_, removed) = range(old);
        let (start, added) = range(new);
        if added == 0 {
            out.push((start, LineChange::Deleted));
        } else {
            let kind = if removed == 0 { LineChange::Added } else { LineChange::Modified };
            out.extend((start - 1..start - 1 + added).map(|l| (l, kind)));
        }
    }
    out
}

const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ar%x1f%D%x1f%s";

fn parse_log(text: &str) -> Vec<Commit> {
    let mut commits: Vec<Commit> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\x1f').map(str::to_owned);
            Some(Commit {
                hash: fields.next()?,
                parents: fields.next()?.split_whitespace().map(str::to_owned).collect(),
                author: fields.next()?,
                date: fields.next()?,
                refs: fields.next()?,
                subject: fields.next()?,
                ..Default::default()
            })
        })
        .collect();
    let mut lanes: Vec<Option<String>> = vec![];
    let free = |lanes: &mut Vec<Option<String>>| {
        lanes.iter().position(Option::is_none).unwrap_or_else(|| {
            lanes.push(None);
            lanes.len() - 1
        })
    };
    for commit in &mut commits {
        let waiting: Vec<usize> = (0..lanes.len()).filter(|i| lanes[*i].as_deref() == Some(&commit.hash)).collect();
        commit.col = waiting.first().copied().unwrap_or_else(|| free(&mut lanes));
        commit.through = (0..lanes.len()).filter(|i| lanes[*i].is_some() && !waiting.contains(i)).collect();
        for i in &waiting {
            lanes[*i] = None;
        }
        commit.up = waiting;
        for (n, parent) in commit.parents.iter().enumerate() {
            let lane = lanes.iter().position(|l| l.as_ref() == Some(parent)).unwrap_or_else(|| {
                let lane = if n == 0 { commit.col } else { free(&mut lanes) };
                lanes[lane] = Some(parent.clone());
                lane
            });
            commit.down.push(lane);
        }
    }
    commits
}

fn parse_name_status(text: &str) -> Vec<Change> {
    let mut out = vec![];
    let mut parts = text.split('\0');
    while let (Some(code), Some(mut path)) = (parts.next(), parts.next()) {
        let code = code.trim_start();
        if code.starts_with(['R', 'C']) {
            path = parts.next().unwrap_or(path);
        }
        let state = match code.as_bytes().first() {
            Some(b'A') => FileState::Added,
            Some(b'D') => FileState::Deleted,
            _ => FileState::Modified,
        };
        out.push(Change { path: path.to_owned(), state });
    }
    out
}

fn find_repos(folder: &Path) -> (Vec<PathBuf>, bool) {
    let own = run(folder, &["rev-parse", "--show-toplevel"]).is_some();
    let mut repos = if own { vec![folder.to_path_buf()] } else { vec![] };
    let children = |dir: &Path| {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                p.is_dir() && !name.starts_with('.') && name != "node_modules" && name != "target"
            })
            .collect();
        dirs.sort();
        dirs
    };
    for child in children(folder) {
        if child.join(".git").exists() {
            repos.push(child.clone());
        }
        repos.extend(children(&child).into_iter().filter(|p| p.join(".git").exists()));
    }
    (repos, own)
}

fn load_status(root: &Path, details: bool) -> Option<Status> {
    let top = crate::wsl::local(root, run(root, &["rev-parse", "--show-toplevel"])?.trim());
    let text = run(root, &["--no-optional-locks", "status", "--porcelain=v1", "-z", "--branch", "--ignored"])?;
    let (branch, tracking, entries) = parse_status(&text);
    let branches = run(root, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).unwrap_or_default();
    let branches = branches.lines().map(str::to_owned).collect();
    let log = ["log", "--topo-order", "-n", "200", LOG_FORMAT, "HEAD", "@{upstream}"];
    let log = run(root, &log[..5 + tracking.map_or(1, |_| 2)]).or_else(|| run(root, &log[..6])).unwrap_or_default();
    let log = parse_log(&log);
    let branch = branch
        .or_else(|| run(root, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_owned()))
        .unwrap_or_default();
    let top_key = key(&top);
    let mut files = HashMap::new();
    let mut folders: HashMap<String, FileState> = HashMap::new();
    let (mut staged, mut unstaged) = (vec![], vec![]);
    for Entry { path, state, staged: x, unstaged: y } in entries {
        if let Some(state) = x {
            staged.push(Change { path: path.clone(), state });
        }
        if let Some(state) = y {
            unstaged.push(Change { path: path.clone(), state });
        }
        let full = key(&top.join(&path));
        if state != FileState::Ignored {
            let mut parent = full.as_str();
            while let Some(i) = parent.rfind('/') {
                parent = &parent[..i];
                if parent.len() < top_key.len() {
                    break;
                }
                let slot = folders.entry(parent.to_owned()).or_insert(state);
                if state.strength() > slot.strength() {
                    *slot = state;
                }
            }
        }
        files.insert(full, state);
    }
    let details = details.then(|| ops::load(&top));
    Some(Status { details, top, branch, tracking, branches, log, staged, unstaged, files, folders })
}

impl Git {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            folder: None,
            folder_repo: None,
            repos: vec![],
            followed: None,
            shown: None,
            texts: vec![],
            prs: None,
            prs_loading: false,
            prs_stale: false,
            root: None,
            ctx: None,
            status: None,
            repo_name: None,
            tx,
            rx,
            running: false,
            pending: false,
            details_requested: false,
            cloned: None,
            busy: false,
            error: None,
            diffs: RefCell::new(HashMap::new()),
        }
    }

    pub fn set_roots(&mut self, roots: Vec<PathBuf>, ctx: &egui::Context) {
        let root = roots.first().cloned();
        self.folder = root.clone();
        self.folder_repo = None;
        self.repos.clear();
        if let Some(folder) = root.clone() {
            let (tx, ctx) = (self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let (mut repos, own) = find_repos(&folder);
                for (more, _) in roots[1..].iter().map(|r| find_repos(r)) {
                    let more: Vec<PathBuf> = more.into_iter().filter(|r| !repos.contains(r)).collect();
                    repos.extend(more);
                }
                let _ = tx.send(Msg::Repos(folder, repos, own));
                ctx.request_repaint();
            });
        }
        self.select(root, ctx);
    }

    pub fn follow(&mut self, active: Option<&Path>, ctx: &egui::Context) {
        if active == self.followed.as_deref() {
            return;
        }
        self.followed = active.map(Path::to_path_buf);
        let owner = active.and_then(|a| self.repos.iter().filter(|r| a.starts_with(r)).max_by_key(|r| r.components().count()));
        if let Some(repo) = owner.cloned().filter(|r| self.root.as_ref() != Some(r)) {
            self.select(Some(repo), ctx);
        }
    }

    pub fn select(&mut self, root: Option<PathBuf>, ctx: &egui::Context) {
        self.root = root;
        self.status = None;
        self.repo_name = None;
        self.shown = None;
        self.prs = None;
        self.error = None;
        self.diffs.borrow_mut().clear();
        self.refresh(ctx);
    }

    pub fn can_init(&self) -> bool {
        self.folder_repo == Some(false)
    }

    pub fn init(&mut self) {
        let (Some(folder), Some(ctx)) = (self.folder.clone(), self.ctx.clone()) else { return };
        if !self.can_init() || self.busy {
            return;
        }
        self.select(Some(folder.clone()), &ctx);
        let tx = self.tx.clone();
        self.job(move || {
            exec(&folder, &["init"])?;
            let (repos, own) = find_repos(&folder);
            let _ = tx.send(Msg::Repos(folder, repos, own));
            Ok(None)
        });
    }

    pub fn repos(&self) -> &[PathBuf] {
        &self.repos
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn log(&self) -> &[Commit] {
        self.status.as_ref().map(|s| s.log.as_slice()).unwrap_or_default()
    }

    pub fn files(&self, hash: &str) -> Option<&[Change]> {
        self.shown.as_ref().filter(|(shown, _)| shown == hash).map(|(_, files)| files.as_slice())
    }

    pub fn show(&mut self, hash: &str) {
        let (Some(status), Some(ctx)) = (&self.status, &self.ctx) else { return };
        let (top, tx, ctx, hash) = (status.top.clone(), self.tx.clone(), ctx.clone(), hash.to_owned());
        std::thread::spawn(move || {
            let args = ["show", "--format=", "--name-status", "-z", "--diff-merges=first-parent", &hash];
            let files = parse_name_status(&run(&top, &args).unwrap_or_default());
            let _ = tx.send(Msg::Files(hash, files));
            ctx.request_repaint();
        });
    }

    pub fn diff(&mut self, title: String, args: Vec<String>) {
        let Some(ctx) = &self.ctx else { return };
        let top = self.status.as_ref().map_or_else(std::env::temp_dir, |status| status.top.clone());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let text = match exec(&top, &args) {
                Err(error) if error.starts_with("Could not run") => return crate::ui::notify(error),
                result => result.unwrap_or_else(|error| error),
            };
            let _ = tx.send(Msg::Text(title, text));
            ctx.request_repaint();
        });
    }

    pub fn take_texts(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.texts)
    }

    pub fn prs(&self) -> Option<&Result<Vec<Pr>, String>> {
        self.prs.as_ref()
    }

    pub fn refresh_prs(&mut self) {
        self.prs = None;
    }

    pub fn load_prs(&mut self) {
        let (Some(status), Some(ctx)) = (&self.status, &self.ctx) else { return };
        if self.prs.is_some() || self.prs_loading {
            return;
        }
        self.prs_loading = true;
        let (top, tx, ctx) = (status.top.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let prs = command("gh", &top, &["pr", "list", "--limit", "30", "--json", PR_FIELDS])
                .and_then(|text| serde_json::from_str(&text).map_err(|e| format!("Could not read the pull request list: {e}")));
            let _ = tx.send(Msg::Prs(top, prs));
            ctx.request_repaint();
        });
    }

    pub fn pr(&mut self, args: &[&str]) {
        self.act(&[&[&["gh", "pr"], args].concat()]);
    }

    pub fn create_pr(&mut self, title: &str, body: &str) {
        self.act(&[&["push", "-u", "origin", "HEAD"], &["gh", "pr", "create", "--title", title, "--body", body]]);
    }

    pub fn stash(&mut self, pop: bool) {
        match pop {
            true => self.act(&[&["stash", "pop"]]),
            false => self.act(&[&["stash", "push", "--include-untracked"]]),
        }
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
        let Some(root) = self.root.clone() else { return };
        if self.running {
            self.pending = true;
            return;
        }
        self.running = true;
        let details = self.details_requested;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let status = load_status(&root, details);
            let _ = tx.send(Msg::Status(root, status));
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) -> bool {
        let mut acted = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(root, status) => {
                    self.running = false;
                    let current = self.root.as_ref() == Some(&root);
                    if current {
                        self.repo_name = status
                            .as_ref()
                            .and_then(|s| s.top.file_name())
                            .map(|n| n.to_string_lossy().into_owned());
                        self.status = status;
                        // keep the old marks on screen until the new diff arrives so the gutter does not flash
                        self.diffs.borrow_mut().values_mut().for_each(|d| d.1 = false);
                    }
                    if (self.pending || !current)
                        && let Some(ctx) = self.ctx.clone()
                    {
                        self.pending = false;
                        self.refresh(&ctx);
                    }
                }
                Msg::Diff(path, changes) => {
                    self.diffs.borrow_mut().insert(path, (Some(Arc::new(changes)), true));
                }
                Msg::Repos(folder, repos, own) => {
                    if self.folder.as_ref() == Some(&folder) {
                        self.folder_repo = Some(own);
                        if !own
                            && self.root == self.folder
                            && let (Some(first), Some(ctx)) = (repos.first(), self.ctx.clone())
                        {
                            self.select(Some(first.clone()), &ctx);
                        }
                        self.repos = repos;
                        self.followed = None;
                    }
                }
                Msg::Files(hash, files) => self.shown = Some((hash, files)),
                Msg::Text(title, text) => self.texts.push((title, text)),
                Msg::Prs(top, prs) => {
                    self.prs_loading = false;
                    if self.top() == Some(top.as_path()) {
                        self.prs = Some(prs);
                    }
                }
                Msg::Done(root, result) => {
                    acted = true;
                    self.busy = false;
                    if std::mem::take(&mut self.prs_stale) {
                        self.prs = None;
                    }
                    match result {
                        Ok(Some(path)) => self.cloned = Some(path),
                        Ok(None) => {}
                        Err(error) if root == self.root => self.error = Some(error),
                        Err(error) => crate::ui::notify(error),
                    }
                    if let Some(ctx) = self.ctx.clone() {
                        self.refresh(&ctx);
                    }
                }
            }
        }
        acted
    }

    pub fn branch(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.branch.as_str())
    }

    pub fn repo_name(&self) -> Option<&str> {
        self.repo_name.as_deref()
    }

    pub fn top(&self) -> Option<&Path> {
        self.status.as_ref().map(|s| s.top.as_path())
    }

    pub fn changes(&self, staged: bool) -> &[Change] {
        match &self.status {
            Some(s) if staged => &s.staged,
            Some(s) => &s.unstaged,
            None => &[],
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn tracking(&self) -> Option<(usize, usize)> {
        self.status.as_ref()?.tracking
    }

    pub fn branches(&self) -> &[String] {
        self.status.as_ref().map(|s| s.branches.as_slice()).unwrap_or_default()
    }

    pub fn stage(&mut self, path: Option<&str>) {
        self.act(&[&["add", "-A", "--", path.unwrap_or(".")]]);
    }

    pub fn unstage(&mut self, path: Option<&str>) {
        self.act(&[&["reset", "-q", "--", path.unwrap_or(".")]]);
    }

    pub fn discard(&mut self, path: Option<&str>) {
        self.act(&[&["checkout", "--", path.unwrap_or(".")]]);
    }

    pub fn commit(&mut self, message: &str) {
        match self.changes(true).is_empty() {
            true => self.act(&[&["add", "-A", "--", "."], &["commit", "-q", "-m", message]]),
            false => self.act(&[&["commit", "-q", "-m", message]]),
        }
    }

    pub fn push(&mut self) {
        match self.tracking() {
            Some(_) => self.act(&[&["push"]]),
            None => self.act(&[&["push", "-u", "origin", "HEAD"]]),
        }
    }

    pub fn sync(&mut self) {
        match self.tracking() {
            Some(_) => self.act(&[&["pull"], &["push"]]),
            None => self.push(),
        }
    }

    pub fn switch(&mut self, branch: &str, create: bool) {
        match create {
            true => self.act(&[&["switch", "-c", branch]]),
            false => self.act(&[&["switch", branch]]),
        }
    }

    pub fn request_details(&mut self, ctx: &egui::Context) {
        if !self.details_requested {
            self.details_requested = true;
            self.refresh(ctx);
        }
    }

    pub fn loading(&self) -> bool {
        self.running
    }

    pub fn details(&self) -> Option<&Result<ops::Details, String>> {
        self.status.as_ref()?.details.as_ref()
    }

    pub fn take_cloned(&mut self) -> Option<PathBuf> {
        self.cloned.take()
    }

    pub fn job(&mut self, work: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static) {
        let Some(ctx) = self.ctx.clone() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let (tx, root) = (self.tx.clone(), self.root.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Done(root, work()));
            ctx.request_repaint();
        });
    }

    pub fn act(&mut self, steps: &[&[&str]]) {
        let Some(top) = self.top().map(Path::to_path_buf) else { return };
        if self.busy {
            return;
        }
        self.prs_stale = steps.iter().any(|s| ["gh", "push", "pull", "fetch"].contains(&s[0]));
        let steps: Vec<Vec<String>> = steps.iter().map(|s| s.iter().map(|a| a.to_string()).collect()).collect();
        let names: Vec<&str> = steps.iter().filter_map(|s| s.first().map(String::as_str)).filter(|n| ["fetch", "pull", "push"].contains(n)).collect();
        let title = (!names.is_empty()).then(|| format!("Git {}", names.join(" + ")));
        self.job(move || tracked(title, |step| {
            for args in &steps {
                step(format!("Git {}", args.join(" ")));
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                match args.split_first() {
                    Some((&"gh", rest)) => command("gh", &top, rest)?,
                    _ => exec(&top, &args)?,
                };
            }
            Ok(None)
        }));
    }

    #[cfg(test)]
    pub fn wait(&mut self, done: impl Fn(&Git) -> bool) {
        for _ in 0..1000 {
            self.poll();
            if done(self) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("git did not settle: {:?}", self.error);
    }

    pub fn state(&self, path: &Path) -> Option<FileState> {
        let status = self.status.as_ref()?;
        let full = key(path);
        if let Some(state) = status.files.get(&full).or_else(|| status.folders.get(&full)) {
            return Some(*state);
        }
        let top_len = status.top.as_os_str().len();
        let mut parent = full.as_str();
        while let Some(i) = parent.rfind('/') {
            parent = &parent[..i];
            if parent.len() < top_len {
                break;
            }
            if let Some(state @ (FileState::Ignored | FileState::Added)) = status.files.get(parent) {
                return Some(*state);
            }
        }
        None
    }

    pub fn line_changes(&self, path: &Path) -> Option<LineChanges> {
        let mut diffs = self.diffs.borrow_mut();
        let (shown, fresh) = diffs.entry(path.to_path_buf()).or_insert((None, false));
        if std::mem::replace(fresh, true) {
            return shown.clone();
        }
        let shown = shown.clone();
        let (Some(status), Some(ctx)) = (&self.status, &self.ctx) else { return shown };
        let (top, tx, ctx, path) = (status.top.clone(), self.tx.clone(), ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let file = crate::wsl::shell_path(&path);
            let all = run(&top, &["diff", "--no-color", "-U0", "HEAD", "--", &file]).unwrap_or_default();
            let unstaged = run(&top, &["diff", "--no-color", "-U0", "--", &file]).unwrap_or_default();
            let _ = tx.send(Msg::Diff(path, mark_staged(parse_diff(&all), &parse_diff(&unstaged))));
            ctx.request_repaint();
        });
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_entries_and_renames() {
        let text = "## main...origin/main [ahead 1]\0 M src/a.rs\0?? new.txt\0R  b.rs\0old.rs\0!! target/\0UU c.rs\0 D gone.rs\0AM both.rs\0";
        let (branch, tracking, files) = parse_status(text);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(tracking, Some((1, 0)));
        assert_eq!(parse_tracking("## dev...origin/dev [ahead 12, behind 3]"), Some((12, 3)));
        assert_eq!(parse_tracking("## dev...origin/dev"), Some((0, 0)));
        assert_eq!(parse_tracking("## dev"), None);
        use FileState::*;
        let entry = |path: &str, state, staged, unstaged| Entry { path: path.into(), state, staged, unstaged };
        assert_eq!(
            files,
            vec![
                entry("src/a.rs", Modified, None, Some(Modified)),
                entry("new.txt", Added, None, Some(Added)),
                entry("b.rs", Modified, Some(Modified), None),
                entry("target/", Ignored, None, None),
                entry("c.rs", Conflicted, None, Some(Conflicted)),
                entry("gone.rs", Deleted, None, Some(Deleted)),
                entry("both.rs", Added, Some(Added), Some(Modified)),
            ]
        );
        assert_eq!(parse_branch("## No commits yet on dev").as_deref(), Some("dev"));
        assert_eq!(parse_branch("## HEAD (no branch)"), None);
    }

    #[test]
    fn parses_diff_hunks() {
        let text = "@@ -3,0 +4,2 @@ ctx\n+a\n+b\n@@ -10 +12 @@\n-x\n+y\n@@ -20,3 +21,0 @@\n-a\n";
        assert_eq!(
            parse_diff(text),
            vec![
                (3, LineChange::Added),
                (4, LineChange::Added),
                (11, LineChange::Modified),
                (21, LineChange::Deleted),
            ]
        );
    }

    #[test]
    fn reads_a_real_repository() {
        let dir = std::env::temp_dir().join(format!("halo-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let git = |args: &[&str]| run(&dir, args).unwrap();
        git(&["init", "-q", "-b", "trunk"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("sub/a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "out/\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("sub/a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        std::fs::write(dir.join("out/o.bin"), "x").unwrap();

        let status = load_status(&dir.join("sub"), false).unwrap();
        assert_eq!(status.branch, "trunk");
        let paths = |list: &[Change]| list.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert!(status.staged.is_empty());
        assert_eq!(paths(&status.unstaged), ["sub/a.txt", "new.txt"]);
        git(&["add", "-A", "--", "new.txt"]);
        let staged = load_status(&dir, false).unwrap();
        assert_eq!(staged.staged, [Change { path: "new.txt".into(), state: FileState::Added }]);
        assert_eq!(paths(&staged.unstaged), ["sub/a.txt"]);
        git(&["reset", "-q", "--", "new.txt"]);
        assert!(exec(&dir, &["commit", "-q", "-m", "nothing staged"]).is_err());
        let mut g = Git::new();
        g.status = Some(status);
        assert_eq!(g.state(&dir.join("sub/a.txt")), Some(FileState::Modified));
        assert_eq!(g.state(&dir.join("sub")), Some(FileState::Modified));
        assert_eq!(g.state(&dir.join("new.txt")), Some(FileState::Added));
        assert_eq!(g.state(&dir.join("out/o.bin")), Some(FileState::Ignored));
        assert_eq!(g.state(&dir.join(".gitignore")), None);

        let diff = git(&["diff", "--no-color", "-U0", "HEAD", "--", "sub/a.txt"]);
        assert_eq!(parse_diff(&diff), vec![(1, LineChange::Modified), (2, LineChange::Modified)]);
        let marked = mark_staged(parse_diff(&diff), &[(2, LineChange::Modified)]);
        assert_eq!(marked, vec![(1, LineChange::Modified, true), (2, LineChange::Modified, false)]);
        assert!(load_status(&std::env::temp_dir(), false).is_none() || std::env::temp_dir().join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lays_out_lanes_and_parses_commit_files() {
        let text = "a\x1fc\x1fme\x1fnow\x1fHEAD -> main\x1flocal\nb\x1fc\x1fyou\x1fnow\x1forigin/main\x1fremote\n\
                    m\x1fc x\x1fme\x1fnow\x1f\x1fmerge\nx\x1fd\x1fme\x1fnow\x1f\x1fside\nc\x1fd\x1fme\x1fnow\x1f\x1fbase\nd\x1f\x1fme\x1fnow\x1f\x1froot\n";
        let log = parse_log(text);
        let lanes = |i: usize| (log[i].col, log[i].up.clone(), log[i].through.clone(), log[i].down.clone());
        assert_eq!(log[0].refs, "HEAD -> main");
        assert_eq!(log[5].subject, "root");
        assert_eq!(lanes(0), (0, vec![], vec![], vec![0]));
        assert_eq!(lanes(1), (1, vec![], vec![0], vec![0]));
        assert_eq!(lanes(2), (1, vec![], vec![0], vec![0, 1]));
        assert_eq!(lanes(3), (1, vec![1], vec![0], vec![1]));
        assert_eq!(lanes(4), (0, vec![0], vec![1], vec![1]));
        assert_eq!(lanes(5), (1, vec![1], vec![], vec![]));

        let json = r#"[{"headRefName":"fix","isDraft":false,"mergeable":"CONFLICTING","number":7,"reviewDecision":"","title":"Fix it"}]"#;
        let prs: Vec<Pr> = serde_json::from_str(json).unwrap();
        assert_eq!((prs[0].number, prs[0].head_ref_name.as_str(), prs[0].mergeable.as_str()), (7, "fix", "CONFLICTING"));

        let files = parse_name_status("M\0src/a.rs\0A\0new.txt\0R100\0old.rs\0b.rs\0D\0gone.rs\0");
        let expect = [("src/a.rs", FileState::Modified), ("new.txt", FileState::Added), ("b.rs", FileState::Modified), ("gone.rs", FileState::Deleted)];
        assert_eq!(files, expect.map(|(path, state)| Change { path: path.into(), state }));
    }

    #[test]
    fn pushes_pulls_switches_and_discards() {
        let base = std::env::temp_dir().join(format!("halo-git-sync-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, remote) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        exec(&remote, &["init", "-q", "--bare"]).unwrap();
        let git = |args: &[&str]| exec(&dir, args).unwrap();
        git(&["init", "-q", "-b", "trunk"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let mut g = Git::new();
        g.set_roots(vec![dir.clone()], &egui::Context::default());
        g.wait(|g| g.branch() == Some("trunk"));
        assert_eq!(g.tracking(), None);
        g.push();
        g.wait(|g| g.tracking() == Some((0, 0)));

        git(&["commit", "-q", "--allow-empty", "-m", "second"]);
        g.sync();
        g.wait(|g| !g.busy() && g.tracking() == Some((0, 0)));
        assert_eq!(g.error(), None);
        assert_eq!(exec(&remote, &["log", "--format=%s", "trunk"]).unwrap(), "second\ninit\n");

        g.switch("feature", true);
        g.wait(|g| g.branch() == Some("feature"));
        assert_eq!(g.branches(), ["feature", "trunk"]);
        g.switch("trunk", false);
        g.wait(|g| g.branch() == Some("trunk"));

        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
        g.refresh(&egui::Context::default());
        g.wait(|g| g.changes(false).len() == 1);
        g.diff("a.txt (changes)".into(), vec!["diff".into(), "--".into(), "a.txt".into()]);
        g.wait(|g| !g.texts.is_empty());
        let texts = g.take_texts();
        assert_eq!(texts[0].0, "a.txt (changes)");
        assert!(texts[0].1.contains("-one") && texts[0].1.contains("+changed"));
        g.discard(Some("a.txt"));
        g.wait(|g| !g.busy() && g.changes(false).is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap().replace('\r', ""), "one\n");

        assert_eq!(g.log().iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["second", "init"]);
        assert!(g.log()[0].refs.contains("origin/trunk"));
        let first = g.log()[1].hash.clone();
        g.show(&first);
        g.wait(|g| g.files(&first).is_some());
        assert_eq!(g.files(&first).unwrap(), [Change { path: "a.txt".into(), state: FileState::Added }]);

        std::fs::write(dir.join("a.txt"), "stashed\n").unwrap();
        std::fs::write(dir.join("extra.txt"), "x").unwrap();
        g.refresh(&egui::Context::default());
        g.wait(|g| g.changes(false).len() == 2);
        g.stash(false);
        g.wait(|g| !g.busy() && g.changes(false).is_empty());
        g.stash(true);
        g.wait(|g| !g.busy() && g.changes(false).len() == 2);
        assert_eq!(g.error(), None);
        g.commit("everything");
        g.wait(|g| !g.busy() && g.changes(false).is_empty() && g.changes(true).is_empty());
        assert_eq!(exec(&dir, &["log", "-1", "--format=%s"]).unwrap(), "everything\n");
        git(&["reset", "-q", "HEAD~1"]);
        g.refresh(&egui::Context::default());
        g.wait(|g| g.changes(false).len() == 2);

        let nested = dir.join("libs").join("inner");
        std::fs::create_dir_all(&nested).unwrap();
        exec(&nested, &["init", "-q"]).unwrap();
        assert_eq!(find_repos(&dir), (vec![dir.clone(), nested.clone()], true));
        assert_eq!(find_repos(&base), (vec![dir.clone()], false));
        g.set_roots(vec![base.clone()], &egui::Context::default());
        g.wait(|g| g.branch() == Some("trunk") && g.repos().len() == 1);
        assert_eq!(g.root(), Some(dir.as_path()));
        g.set_roots(vec![nested.clone(), dir.clone()], &egui::Context::default());
        g.wait(|g| g.repos().len() == 2);
        assert_eq!(g.repos(), [nested.clone(), dir.clone()]);
        g.follow(Some(&dir.join("a.txt")), &egui::Context::default());
        assert_eq!(g.root(), Some(dir.as_path()));
        g.follow(Some(&nested.join("x.rs")), &egui::Context::default());
        assert_eq!(g.root(), Some(nested.as_path()));
        g.set_roots(vec![base.clone()], &egui::Context::default());
        g.wait(|g| g.branch() == Some("trunk") && g.repos().len() == 1);

        g.switch("missing", false);
        g.wait(|g| g.error().is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
