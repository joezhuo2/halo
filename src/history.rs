use crate::app::Halo;
use crate::command::Command;
use crate::editor::Encoding;
use crate::{settings, ui};
use eframe::egui;
use ropey::Rope;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIMIT: usize = 1024 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub fn now() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
}

pub fn age(seconds: u64) -> String {
    let elapsed = ((now() / 1_000_000_000) as u64).saturating_sub(seconds);
    for (size, unit) in [(31_536_000, "year"), (2_592_000, "month"), (86_400, "day"), (3600, "hour"), (60, "minute")] {
        if elapsed >= size {
            let count = elapsed / size;
            return format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
        }
    }
    "just now".into()
}

fn directory(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let hash = path.as_os_str().as_encoded_bytes().iter().fold(0xcbf29ce484222325u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    Ok(root.join(format!("{hash:016x}")))
}

fn entries(dir: &Path) -> io::Result<Vec<(u128, PathBuf)>> {
    let reader = match fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        other => other?,
    };
    let mut list = vec![];
    for item in reader {
        let path = item?.path();
        if matches!(path.extension().and_then(|x| x.to_str()), Some("snap" | "latin1"))
            && let Some(time) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok())
        {
            list.push((time, path));
        }
    }
    list.sort_unstable();
    Ok(list)
}

pub fn snapshots(file: &Path) -> io::Result<Vec<(u128, PathBuf)>> {
    entries(&directory(&settings::dir().join("history"), file)?)
}

fn record(dir: &Path, bytes: &[u8], latin: bool, time: u128) -> io::Result<()> {
    if bytes.len() > LIMIT {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let list = entries(dir)?;
    if let Some((_, path)) = list.last()
        && fs::read(path)? == bytes
    {
        return Ok(());
    }
    let mut time = list.last().map_or(time, |last| time.max(last.0 + 1));
    let ext = if latin { "latin1" } else { "snap" };
    let (temp, mut handle) = Temp::new(dir)?;
    handle.write_all(bytes)?;
    drop(handle);
    loop {
        match fs::hard_link(&temp.0, dir.join(format!("{time:020}.{ext}"))) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => time += 1,
            Err(e) => return Err(e),
        }
    }
    let list = entries(dir)?;
    for (_, path) in list.iter().take(list.len().saturating_sub(50)) {
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

struct Save {
    file: PathBuf,
    bytes: Vec<u8>,
    latin: bool,
    time: u128,
    ctx: egui::Context,
}

#[derive(Default)]
pub struct Recorder {
    pub context: Option<egui::Context>,
    worker: Option<Sender<Save>>,
    changed: Option<Receiver<()>>,
}

impl Recorder {
    pub fn save(&mut self, file: &Path, bytes: Vec<u8>, encoding: Encoding) {
        let Some(ctx) = self.context.clone().filter(|_| bytes.len() <= LIMIT) else { return };
        if self.worker.is_none() {
            let (tx, rx) = channel::<Save>();
            let (changed, events) = channel();
            self.worker = Some(tx);
            self.changed = Some(events);
            std::thread::spawn(move || {
                for save in rx {
                    let result = directory(&settings::dir().join("history"), &save.file)
                        .and_then(|dir| record(&dir, &save.bytes, save.latin, save.time));
                    if let Err(e) = result {
                        ui::notify(format!("Could not save local history for {}: {e}", save.file.display()));
                    }
                    let _ = changed.send(());
                    save.ctx.request_repaint();
                }
            });
        }
        let save = Save { file: file.to_owned(), bytes, latin: encoding == Encoding::Latin1, time: now(), ctx };
        let _ = self.worker.as_ref().unwrap().send(save);
    }

    pub fn changed(&self) -> bool {
        self.changed.as_ref().is_some_and(|rx| rx.try_iter().count() > 0)
    }
}

pub struct Temp(pub PathBuf);

impl Temp {
    fn new(dir: &Path) -> io::Result<(Self, File)> {
        loop {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = dir.join(format!("halo-{}-{}-{id}.tmp", std::process::id(), now()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn text(rope: &Rope) -> io::Result<Self> {
        let (temp, mut file) = Self::new(&std::env::temp_dir())?;
        rope.write_to(&mut file)?;
        Ok(temp)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn no_index(args: &[String]) -> Result<String, String> {
    let out = crate::terminal::hidden_command("git").args(args).output().map_err(|e| format!("Could not run git: {e}"))?;
    if matches!(out.status.code(), Some(0 | 1)) {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

pub fn snapshot_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read the saved version: {e}"))?;
    let encoding = if path.extension().is_some_and(|s| s == "latin1") { Encoding::Latin1 } else { Encoding::detect(&bytes) };
    Ok(encoding.decode(&bytes).0.replace("\r\n", "\n"))
}

pub fn name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

pub fn diff_args(left: &Path, right: &Path) -> Vec<String> {
    let args = ["diff", "--no-index", "--no-color", "--no-ext-diff", "--ignore-cr-at-eol", "--"].map(str::to_owned);
    [args.to_vec(), vec![left.to_string_lossy().into_owned(), right.to_string_lossy().into_owned()]].concat()
}

enum Side {
    File(PathBuf),
    Text(Rope),
    Saved(PathBuf, Encoding),
}

impl Side {
    fn prepare(self, temps: &mut Vec<Temp>) -> Result<PathBuf, String> {
        let rope = match self {
            Self::File(path) => return Ok(path),
            Self::Text(text) => text,
            Self::Saved(path, encoding) => {
                let bytes = fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
                Rope::from_str(&encoding.decode(&bytes).0.replace("\r\n", "\n"))
            }
        };
        let temp = Temp::text(&rope).map_err(|e| format!("Cannot prepare the comparison: {e}"))?;
        let path = temp.0.clone();
        temps.push(temp);
        Ok(path)
    }
}

fn files(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, u64>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() && entry.file_name() != ".git" {
            files(root, &entry.path(), out)?;
        } else if kind.is_file() {
            out.insert(entry.path().strip_prefix(root).unwrap_or(&entry.path()).to_path_buf(), entry.metadata()?.len());
        }
    }
    Ok(())
}

// shortcut: walks every folder but .git, so node_modules or target make it slow; upgrade to the explorer's excludes if that bites
fn folder_changes(left: &Path, right: &Path) -> Result<Vec<crate::search::Choice>, String> {
    let scan = |root: &Path| {
        let mut out = std::collections::BTreeMap::new();
        files(root, root, &mut out).map(|_| out).map_err(|e| format!("Cannot read {}: {e}", root.display()))
    };
    let (a, b) = (scan(left)?, scan(right)?);
    let all: std::collections::BTreeSet<&PathBuf> = a.keys().chain(b.keys()).collect();
    let mut items = vec![];
    for rel in all {
        let (l, r) = (left.join(rel), right.join(rel));
        let shown = rel.to_string_lossy().replace('\\', "/");
        items.push(match (a.get(rel), b.get(rel)) {
            (Some(x), Some(y)) if x == y && fs::read(&l).ok() == fs::read(&r).ok() => continue,
            (Some(_), Some(_)) => (shown.clone(), "Changed".into(), Command::OpenDiff(format!("{shown} ({} ↔ {})", name(left), name(right)), diff_args(&l, &r))),
            (Some(_), None) => (shown, format!("Only in {}", name(left)), Command::OpenPath(l)),
            _ => (shown, format!("Only in {}", name(right)), Command::OpenPath(r)),
        });
    }
    Ok(items)
}

enum Event {
    Open(String, Vec<String>, Vec<Temp>),
    Restore(PathBuf, u64, String),
    Folders(String, Vec<crate::search::Choice>),
}

pub struct Compare {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    temps: Vec<Temp>,
    paste: Option<(String, Rope, Instant)>,
    folders: Option<(PathBuf, PathBuf)>,
}

impl Default for Compare {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, temps: vec![], paste: None, folders: None }
    }
}

impl Compare {
    fn job(&self, ctx: &egui::Context, work: impl FnOnce() -> Result<Event, String> + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            match work() {
                Ok(event) => drop(tx.send(event)),
                Err(error) => ui::notify(error),
            }
            ctx.request_repaint();
        });
    }

    fn pair(&self, ctx: &egui::Context, title: String, left: Side, right: Side) {
        self.job(ctx, move || {
            let mut temps = vec![];
            let args = diff_args(&left.prepare(&mut temps)?, &right.prepare(&mut temps)?);
            Ok(Event::Open(title, args, temps))
        });
    }
}

impl Halo {
    pub(crate) fn exec_compare(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        use Command::*;
        let active = || self.editors.active_path().ok_or("Open a saved file first.");
        let result = match cmd {
            CompareFile | CompareClipboard | CompareSaved => match self.editors.buffer() {
                None => Err("Open a saved file first."),
                Some((path, text, encoding, _)) => {
                    let file = name(&path);
                    match cmd {
                        CompareFile => {
                            if let Some(other) = rfd::FileDialog::new().set_directory(path.parent().unwrap_or(&path)).pick_file() {
                                self.compare.pair(ctx, format!("{file} ↔ {}", name(&other)), Side::Text(text), Side::File(other));
                            }
                        }
                        CompareClipboard => {
                            self.compare.paste = Some((file, text, Instant::now()));
                            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                        }
                        _ => self.compare.pair(ctx, format!("{file} (Saved) ↔ {file}"), Side::Saved(path, encoding), Side::Text(text)),
                    }
                    Ok(())
                }
            },
            SelectForCompare | SelectComparePath(_) => {
                let path = if let SelectComparePath(path) = cmd { Ok(path.clone()) } else { active() };
                path.map(|path| self.explorer.compare = Some(path))
            }
            CompareWithSelected | ComparePath(_) => {
                let other = if let ComparePath(path) = cmd { Ok(path.clone()) } else { active() };
                match (self.explorer.compare.clone(), other) {
                    (None, _) => Err("Select a file for compare first."),
                    (_, Err(e)) => Err(e),
                    (Some(left), Ok(right)) => {
                        self.cmds.push(OpenDiff(format!("{} ↔ {}", name(&left), name(&right)), diff_args(&left, &right)));
                        Ok(())
                    }
                }
            }
            CompareFolders => {
                let start = self.active_root();
                let pick = |title: &str| {
                    let dialog = rfd::FileDialog::new().set_title(title);
                    match &start {
                        Some(dir) => dialog.set_directory(dir),
                        None => dialog,
                    }
                    .pick_folder()
                };
                if let Some(left) = pick("Select the first folder")
                    && let Some(right) = pick("Select the folder to compare it with")
                {
                    self.compare.folders = Some((left, right));
                    self.exec_compare(&CompareFoldersAgain, ctx);
                }
                Ok(())
            }
            CompareFoldersAgain => match self.compare.folders.clone() {
                None => Err("Compare two folders first."),
                Some((left, right)) => {
                    let hint = format!("{} ↔ {}: select a file", name(&left), name(&right));
                    self.compare.job(ctx, move || Ok(Event::Folders(hint, folder_changes(&left, &right)?)));
                    Ok(())
                }
            },
            ToggleInlineBlame => {
                self.settings.inline_blame ^= true;
                self.settings.save();
                Ok(())
            }
            ShowLineHistory => match self.editors.selected_lines() {
                Some((path, first, last)) => {
                    self.timeline.show_lines(path, first, last);
                    self.session.timeline = true;
                    self.reveal(crate::workbench::View::Timeline);
                    Ok(())
                }
                None => Err("Open a saved file to see the history of its lines."),
            },
            OpenLineChanges => match self.editors.blame_item(ctx) {
                Some((_, _, Some(diff))) => {
                    self.cmds.push(diff);
                    Ok(())
                }
                _ => Err("The current line has no commit."),
            },
            OpenLineCommitOnRemote => match self.editors.open_line_commit_on_remote() {
                true => Ok(()),
                false => Err("The current line has no commit."),
            },
            RestoreSnapshot(file, snapshot) => match self.editors.buffer().filter(|b| &b.0 == file) {
                Some((_, _, _, version)) => {
                    let (file, snapshot) = (file.clone(), snapshot.clone());
                    self.compare.job(ctx, move || Ok(Event::Restore(file, version, snapshot_text(&snapshot)?)));
                    Ok(())
                }
                None => Err("Open the file to restore it."),
            },
            _ => return false,
        };
        if let Err(error) = result {
            ui::notify(error);
        }
        true
    }

    pub(crate) fn poll_compare(&mut self, ctx: &egui::Context) {
        if let Some((file, text, since)) = self.compare.paste.take() {
            let pasted = ctx.input_mut(|i| {
                let at = i.events.iter().position(|e| matches!(e, egui::Event::Paste(_)))?;
                match i.events.remove(at) {
                    egui::Event::Paste(s) => Some(s),
                    _ => None,
                }
            });
            match pasted {
                Some(clip) => self.compare.pair(ctx, format!("{file} ↔ Clipboard"), Side::Text(text), Side::Text(Rope::from_str(&clip))),
                None if since.elapsed() >= Duration::from_secs(1) => ui::notify("The clipboard has no text."),
                None => {
                    self.compare.paste = Some((file, text, since));
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        while let Ok(event) = self.compare.rx.try_recv() {
            match event {
                Event::Open(title, args, temps) => {
                    self.compare.temps = temps;
                    self.cmds.push(Command::OpenDiff(title, args));
                }
                Event::Restore(file, version, text) => self.editors.restore_snapshot(&file, version, text),
                Event::Folders(_, items) if items.is_empty() => ui::notify_info("The folders have the same files with the same contents."),
                Event::Folders(hint, items) => self.search.pick(&hint, items),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_compare_lists_changed_and_one_sided_files() {
        let root = std::env::temp_dir().join(format!("halo-folders-test-{}", now()));
        let (left, right) = (root.join("left"), root.join("right"));
        for (dir, files) in [(&left, [("same.txt", "x"), ("sub/changed.txt", "a"), ("gone.txt", "g")]), (&right, [("same.txt", "x"), ("sub/changed.txt", "b"), ("new.txt", "n")])] {
            for (file, text) in files {
                fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
                fs::write(dir.join(file), text).unwrap();
            }
        }
        fs::create_dir_all(left.join(".git")).unwrap();
        fs::write(left.join(".git/HEAD"), "ref").unwrap();
        let items = folder_changes(&left, &right).unwrap();
        let shown: Vec<(&str, &str)> = items.iter().map(|i| (i.0.as_str(), i.1.as_str())).collect();
        assert_eq!(shown, [("gone.txt", "Only in left"), ("new.txt", "Only in right"), ("sub/changed.txt", "Changed")]);
        assert!(matches!(&items[2].2, Command::OpenDiff(title, args) if title == "sub/changed.txt (left ↔ right)" && args.contains(&"--no-index".to_owned())));
        assert_eq!(items[0].2, Command::OpenPath(left.join("gone.txt")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn names_sorts_deduplicates_and_prunes_saves() {
        let root = std::env::temp_dir().join(format!("halo-history-test-{}", now()));
        let dir = directory(&root, &root.join("a.txt")).unwrap();
        assert_ne!(dir, directory(&root, &root.join("b.txt")).unwrap());
        record(&dir, &vec![0; LIMIT + 1], false, 20).unwrap();
        assert!(!dir.exists());
        record(&dir, b"a", false, 20).unwrap();
        record(&dir, b"a", false, 22).unwrap();
        record(&dir, b"b", true, 19).unwrap();
        let list = entries(&dir).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].1.file_name().unwrap(), "00000000000000000020.snap");
        assert_eq!((list[1].0, list[1].1.extension().unwrap().to_str()), (21, Some("latin1")));
        assert_eq!(snapshot_text(&list[1].1).unwrap(), "b");
        for n in 0..51 { record(&dir, &[n], false, 30 + n as u128).unwrap(); }
        let list = entries(&dir).unwrap();
        assert_eq!((list.len(), list[0].0, list[49].0), (50, 31, 80));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 50);
        fs::remove_dir_all(root).unwrap();
    }
}
