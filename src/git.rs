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
    fn strength(self) -> u8 {
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

pub type LineChanges = Arc<Vec<(usize, LineChange)>>;

struct Status {
    top: PathBuf,
    branch: String,
    files: HashMap<String, FileState>,
    folders: HashMap<String, FileState>,
}

enum Msg {
    Status(PathBuf, Option<Status>),
    Diff(PathBuf, Vec<(usize, LineChange)>),
}

pub struct Git {
    root: Option<PathBuf>,
    ctx: Option<egui::Context>,
    status: Option<Status>,
    repo_name: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    running: bool,
    pending: bool,
    diffs: RefCell<HashMap<PathBuf, Option<LineChanges>>>,
}

pub fn run(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
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

fn parse_status(text: &str) -> (Option<String>, Vec<(String, FileState)>) {
    let mut branch = None;
    let mut files = vec![];
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if entry.starts_with("## ") {
            branch = parse_branch(entry);
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
        files.push((path, state));
    }
    (branch, files)
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

fn load_status(root: &Path) -> Option<Status> {
    let top = PathBuf::from(run(root, &["rev-parse", "--show-toplevel"])?.trim());
    let text = run(root, &["status", "--porcelain=v1", "-z", "--branch", "--ignored"])?;
    let (branch, entries) = parse_status(&text);
    let branch = branch
        .or_else(|| run(root, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_owned()))
        .unwrap_or_default();
    let top_key = key(&top);
    let mut files = HashMap::new();
    let mut folders: HashMap<String, FileState> = HashMap::new();
    for (path, state) in entries {
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
    Some(Status { top, branch, files, folders })
}

impl Git {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            root: None,
            ctx: None,
            status: None,
            repo_name: None,
            tx,
            rx,
            running: false,
            pending: false,
            diffs: RefCell::new(HashMap::new()),
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, ctx: &egui::Context) {
        self.root = root;
        self.status = None;
        self.repo_name = None;
        self.diffs.borrow_mut().clear();
        self.refresh(ctx);
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
        let Some(root) = self.root.clone() else { return };
        if self.running {
            self.pending = true;
            return;
        }
        self.running = true;
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let status = load_status(&root);
            let _ = tx.send(Msg::Status(root, status));
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) {
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
                        self.diffs.borrow_mut().clear();
                    }
                    if (self.pending || !current)
                        && let Some(ctx) = self.ctx.clone()
                    {
                        self.pending = false;
                        self.refresh(&ctx);
                    }
                }
                Msg::Diff(path, changes) => {
                    self.diffs.borrow_mut().insert(path, Some(Arc::new(changes)));
                }
            }
        }
    }

    pub fn branch(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.branch.as_str())
    }

    pub fn repo_name(&self) -> Option<&str> {
        self.repo_name.as_deref()
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
            match status.files.get(parent) {
                Some(state @ (FileState::Ignored | FileState::Added)) => return Some(*state),
                _ => {}
            }
        }
        None
    }

    pub fn line_changes(&self, path: &Path) -> Option<LineChanges> {
        let mut diffs = self.diffs.borrow_mut();
        if let Some(cached) = diffs.get(path) {
            return cached.clone();
        }
        diffs.insert(path.to_path_buf(), None);
        let (Some(status), Some(ctx)) = (&self.status, &self.ctx) else { return None };
        let (top, tx, ctx, path) = (status.top.clone(), self.tx.clone(), ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let file = path.to_string_lossy().into_owned();
            let text = run(&top, &["diff", "--no-color", "-U0", "HEAD", "--", &file]).unwrap_or_default();
            let _ = tx.send(Msg::Diff(path, parse_diff(&text)));
            ctx.request_repaint();
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_entries_and_renames() {
        let text = "## main...origin/main [ahead 1]\0 M src/a.rs\0?? new.txt\0R  b.rs\0old.rs\0!! target/\0UU c.rs\0 D gone.rs\0";
        let (branch, files) = parse_status(text);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(
            files,
            vec![
                ("src/a.rs".into(), FileState::Modified),
                ("new.txt".into(), FileState::Added),
                ("b.rs".into(), FileState::Modified),
                ("target/".into(), FileState::Ignored),
                ("c.rs".into(), FileState::Conflicted),
                ("gone.rs".into(), FileState::Deleted),
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

        let status = load_status(&dir.join("sub")).unwrap();
        assert_eq!(status.branch, "trunk");
        let mut g = Git::new();
        g.status = Some(status);
        assert_eq!(g.state(&dir.join("sub/a.txt")), Some(FileState::Modified));
        assert_eq!(g.state(&dir.join("sub")), Some(FileState::Modified));
        assert_eq!(g.state(&dir.join("new.txt")), Some(FileState::Added));
        assert_eq!(g.state(&dir.join("out/o.bin")), Some(FileState::Ignored));
        assert_eq!(g.state(&dir.join(".gitignore")), None);

        let diff = git(&["diff", "--no-color", "-U0", "HEAD", "--", "sub/a.txt"]);
        assert_eq!(parse_diff(&diff), vec![(1, LineChange::Modified), (2, LineChange::Modified)]);
        assert!(load_status(&std::env::temp_dir()).is_none() || std::env::temp_dir().join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
