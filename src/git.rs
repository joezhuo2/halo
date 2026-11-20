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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub path: String,
    pub state: FileState,
}

#[derive(PartialEq, Eq, Debug)]
struct Entry {
    path: String,
    state: FileState,
    staged: Option<FileState>,
    unstaged: Option<FileState>,
}

struct Status {
    top: PathBuf,
    branch: String,
    tracking: Option<(usize, usize)>,
    branches: Vec<String>,
    staged: Vec<Change>,
    unstaged: Vec<Change>,
    files: HashMap<String, FileState>,
    folders: HashMap<String, FileState>,
}

enum Msg {
    Status(PathBuf, Option<Status>),
    Diff(PathBuf, Vec<(usize, LineChange)>),
    Done(Option<String>),
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
    busy: bool,
    error: Option<String>,
    diffs: RefCell<HashMap<PathBuf, Option<LineChanges>>>,
}

pub fn run(dir: &Path, args: &[&str]) -> Option<String> {
    exec(dir, args).ok()
}

pub fn exec(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(dir).env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|e| format!("Could not run git: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
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
    let (branch, tracking, entries) = parse_status(&text);
    let branches = run(root, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).unwrap_or_default();
    let branches = branches.lines().map(str::to_owned).collect();
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
    Some(Status { top, branch, tracking, branches, staged, unstaged, files, folders })
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
            busy: false,
            error: None,
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
                Msg::Done(error) => {
                    acted = true;
                    self.busy = false;
                    self.error = error;
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
        self.act(&[&["commit", "-q", "-m", message]]);
    }

    pub fn fetch(&mut self) {
        self.act(&[&["fetch"]]);
    }

    pub fn pull(&mut self) {
        self.act(&[&["pull"]]);
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

    fn act(&mut self, steps: &[&[&str]]) {
        let (Some(status), Some(ctx)) = (&self.status, &self.ctx) else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let (top, tx, ctx) = (status.top.clone(), self.tx.clone(), ctx.clone());
        let steps: Vec<Vec<String>> = steps.iter().map(|s| s.iter().map(|a| a.to_string()).collect()).collect();
        std::thread::spawn(move || {
            let error = steps.iter().find_map(|args| {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                exec(&top, &args).err()
            });
            let _ = tx.send(Msg::Done(error));
            ctx.request_repaint();
        });
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

        let status = load_status(&dir.join("sub")).unwrap();
        assert_eq!(status.branch, "trunk");
        let paths = |list: &[Change]| list.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert!(status.staged.is_empty());
        assert_eq!(paths(&status.unstaged), ["sub/a.txt", "new.txt"]);
        git(&["add", "-A", "--", "new.txt"]);
        let staged = load_status(&dir).unwrap();
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
        assert!(load_status(&std::env::temp_dir()).is_none() || std::env::temp_dir().join(".git").exists());
        let _ = std::fs::remove_dir_all(&dir);
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
        g.set_root(Some(dir.clone()), &egui::Context::default());
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
        g.discard(Some("a.txt"));
        g.wait(|g| !g.busy() && g.changes(false).is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap().replace('\r', ""), "one\n");

        g.switch("missing", false);
        g.wait(|g| g.error().is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
