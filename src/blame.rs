use crate::command::Command;
use crate::history::{Temp, age, no_index};
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, RichText, Ui};
use ropey::Rope;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc::{Receiver, channel}};
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub struct Commit {
    hash: String,
    author: String,
    time: u64,
    zone: String,
    subject: String,
    file: String,
}

pub fn changes(dir: &Path, file: &str, hash: &str) -> Command {
    let short = &hash[..hash.len().min(8)];
    let title = format!("{} ({short})", Path::new(file).file_name().unwrap_or_default().to_string_lossy());
    let args = ["-C", &dir.to_string_lossy(), "show", "--format=", "--diff-merges=first-parent", hash, "--",
        &format!(":(top,literal){file}")].map(str::to_owned);
    Command::OpenDiff(title, args.to_vec())
}

impl Commit {
    pub fn label(&self) -> String { format!("{}, {}", self.author, age(self.time)) }

    fn date(&self) -> String {
        let offset = self.zone.parse::<i32>().unwrap_or(0);
        let local = self.time as i64 + (offset / 100 * 3600 + offset % 100 * 60) as i64;
        let day = local.div_euclid(86400) + 719468;
        let era = day.div_euclid(146097);
        let day = day.rem_euclid(146097);
        let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
        let doy = day - (365 * year + year / 4 - year / 100);
        let m = (5 * doy + 2) / 153;
        let d = doy - (153 * m + 2) / 5 + 1;
        let month = if m < 10 { m + 3 } else { m - 9 };
        let year = year + era * 400 + i64::from(month <= 2);
        let hm = local.rem_euclid(86400) / 60;
        format!("{year:04}-{month:02}-{d:02} {:02}:{:02} {}", hm / 60, hm % 60, self.zone)
    }

    pub fn tip(&self) -> String {
        format!("{}\n{}, {}\n\n{}", self.hash, self.author, self.date(), self.subject)
    }

    pub fn open_remote(&self, path: &Path) {
        let (dir, hash) = (path.parent().unwrap_or(path).to_owned(), self.hash.clone());
        std::thread::spawn(move || match web_url(&remote(&dir), &hash) {
            Some((_, url)) => ui::open_url(&url),
            None => ui::notify("The remote of this repository is not a GitHub, GitLab or Bitbucket host."),
        });
    }

    pub fn diff(&self, path: &Path) -> Command {
        changes(path.parent().unwrap_or(path), &self.file, &self.hash)
    }
}

fn remote(dir: &Path) -> String {
    crate::git::read(dir, &["ls-remote", "--get-url"]).unwrap_or_default().trim().to_owned()
}

pub fn web_url(remote: &str, hash: &str) -> Option<(&'static str, String)> {
    let remote = remote.trim().trim_end_matches('/');
    let remote = remote.strip_suffix(".git").unwrap_or(remote);
    let (authority, path) = match remote.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        None => remote.split_once(':')?,
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.trim_start_matches("ssh.").trim_start_matches("altssh.");
    let path = path.trim_matches('/');
    let lower = host.to_ascii_lowercase();
    let (name, route) = match () {
        _ if lower.contains("github") => ("GitHub", "commit"),
        _ if lower.contains("gitlab") => ("GitLab", "-/commit"),
        _ if lower.contains("bitbucket") => ("Bitbucket", "commits"),
        _ => return None,
    };
    (!path.is_empty()).then(|| (name, format!("https://{host}/{path}/{route}/{hash}")))
}

type Detail = (String, Option<(&'static str, String)>);

fn filename(raw: &str) -> String {
    let Some(quoted) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else { return raw.into() };
    let mut bytes = vec![];
    let mut chars = quoted.bytes();
    while let Some(b) = chars.next() {
        if b != b'\\' { bytes.push(b); continue; }
        let Some(b) = chars.next() else { break };
        let b = match b {
            b'0'..=b'3' => (b - b'0') * 64 + (chars.next().unwrap_or(b'0') & 7) * 8 + (chars.next().unwrap_or(b'0') & 7),
            b'n' => b'\n', b'r' => b'\r', b't' => b'\t', b'a' => 7, b'b' => 8, b'v' => 11, b'f' => 12,
            other => other,
        };
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

type Lines = Vec<Option<Arc<Commit>>>;

fn parse(text: &str) -> Lines {
    let mut seen = HashMap::<&str, Arc<Commit>>::new();
    let mut lines = vec![];
    let mut input = text.lines();
    while let Some(header) = input.next() {
        let mut fields = header.split_ascii_whitespace();
        let Some(hash) = fields.next().filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())) else { continue };
        let Some(line) = fields.nth(1).and_then(|s| s.parse::<usize>().ok()) else { continue };
        if line != lines.len() + 1 { break; }
        let commit = seen.entry(hash).or_insert_with(|| Arc::new(Commit { hash: hash.into(), ..Default::default() }));
        for meta in input.by_ref() {
            if meta.starts_with('\t') {
                lines.push((!hash.bytes().all(|b| b == b'0')).then(|| Arc::clone(commit)));
                break;
            }
            if let Some((key, value)) = meta.split_once(' ') {
                match key {
                    "author" => Arc::make_mut(commit).author = value.into(),
                    "author-time" => Arc::make_mut(commit).time = value.parse().unwrap_or_default(),
                    "author-tz" => Arc::make_mut(commit).zone = value.into(),
                    "summary" => Arc::make_mut(commit).subject = value.into(),
                    "filename" => Arc::make_mut(commit).file = filename(value),
                    _ => {}
                }
            }
        }
    }
    lines
}

fn range(s: &str) -> Option<(usize, usize)> {
    let (start, count) = s.get(1..)?.split_once(',').unwrap_or((s.get(1..)?, "1"));
    let count: usize = count.parse().ok()?;
    Some((start.parse::<usize>().ok()?.saturating_sub(usize::from(count > 0)), count))
}

fn map_edits(lines: &[Option<Arc<Commit>>], patch: &str) -> Lines {
    let mut result = vec![];
    let mut from = 0;
    for header in patch.lines().filter_map(|s| s.strip_prefix("@@ ")) {
        let mut fields = header.split_whitespace();
        let Some(((at, removed), (_, added))) = fields.next().and_then(range).zip(fields.next().and_then(range)) else { continue };
        result.extend_from_slice(&lines[from.min(lines.len())..at.min(lines.len()).max(from.min(lines.len()))]);
        result.resize(result.len() + added, None);
        from = at + removed;
    }
    result.extend_from_slice(&lines[from.min(lines.len())..]);
    result
}

type Reply = Result<(Arc<Lines>, Lines), String>;

#[derive(Default)]
pub struct Blame {
    path: PathBuf,
    disk: Option<Arc<Lines>>,
    lines: Option<Lines>,
    version: Option<u64>,
    pending: Option<(u64, bool, Receiver<Reply>)>,
    wait: Option<(u64, Instant)>,
    failed: bool,
    detail: Option<(String, Receiver<Detail>, Option<Detail>)>,
}

impl Blame {
    pub fn refresh(&mut self) {
        (self.disk, self.version, self.failed) = (None, None, false);
    }

    pub fn update(&mut self, path: &Path, text: &Rope, version: u64, dirty: bool, ctx: &egui::Context) {
        if self.path != path { *self = Self { path: path.into(), ..Default::default() }; }
        if let Some((requested, was_dirty, reply)) = self.pending.as_ref().and_then(|(v, d, rx)| Some((*v, *d, rx.try_recv().ok()?))) {
            self.pending = None;
            self.version = Some(requested);
            match reply {
                Ok((disk, lines)) => {
                    self.lines = Some(if was_dirty { lines } else { disk.to_vec() });
                    self.disk = Some(disk);
                }
                Err(error) => {
                    (self.failed, self.lines) = (true, None);
                    if error.starts_with("Could not run") { crate::ui::notify(error) }
                }
            }
        }
        if self.version == Some(version) || self.pending.is_some() || self.failed { return; }
        if let (false, Some(disk)) = (dirty, &self.disk) {
            (self.lines, self.version) = (Some(disk.to_vec()), Some(version));
            return;
        }
        if dirty {
            let (v, since) = self.wait.get_or_insert_with(|| (version, Instant::now()));
            if *v != version { *v = version; *since = Instant::now(); }
            if since.elapsed() < Duration::from_millis(160) {
                ctx.request_repaint_after(Duration::from_millis(160));
                return;
            }
        }
        let (tx, rx) = channel();
        (self.pending, self.wait) = (Some((version, dirty, rx)), None);
        let (path, text, disk, ctx) = (path.to_owned(), text.clone(), self.disk.clone(), ctx.clone());
        std::thread::spawn(move || {
            let load = || {
                let dir = path.parent().ok_or("No file directory")?;
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let disk = match disk {
                    Some(lines) => lines,
                    None => {
                        let raw = crate::git::exec(dir, &["--literal-pathspecs", "blame", "--porcelain", "--", &file]);
                        match raw {
                            Ok(raw) => Arc::new(parse(&raw)),
                            Err(e) if e.contains("no such path") || e.contains("no such ref: HEAD") => Arc::new(vec![]),
                            Err(e) => return Err(e),
                        }
                    }
                };
                let lines = if dirty && !disk.is_empty() {
                    let temp = Temp::text(&text).map_err(|e| e.to_string())?;
                    let args = ["diff", "--no-index", "--no-ext-diff", "--text", "--ignore-cr-at-eol", "-U0", "--",
                        &path.to_string_lossy(), &temp.0.to_string_lossy()].map(str::to_owned);
                    map_edits(&disk, &no_index(&args)?)
                } else { vec![] };
                Ok((disk, lines))
            };
            let _ = tx.send(load());
            ctx.request_repaint();
        });
    }

    pub fn line(&self, line: usize) -> Option<Option<&Commit>> {
        Some(self.lines.as_ref()?.get(line).and_then(Option::as_deref))
    }

    pub fn inline(&self, line: usize) -> Option<String> {
        self.line(line).map(|c| c.map_or_else(|| "Not committed yet".into(), |c| format!("{} • {}", c.label(), c.subject)))
    }

    fn detail(&mut self, hash: &str, ctx: &egui::Context) -> Option<Detail> {
        if self.detail.as_ref().is_none_or(|d| d.0 != hash) {
            let (tx, rx) = channel();
            let (dir, owned, ctx) = (self.path.parent().unwrap_or(&self.path).to_owned(), hash.to_owned(), ctx.clone());
            std::thread::spawn(move || {
                let message = crate::git::read(&dir, &["show", "-s", "--format=%B", &owned]).unwrap_or_default();
                let _ = tx.send((message.trim().to_owned(), web_url(&remote(&dir), &owned)));
                ctx.request_repaint();
            });
            self.detail = Some((hash.to_owned(), rx, None));
        }
        let (_, rx, detail) = self.detail.as_mut()?;
        if detail.is_none() {
            *detail = rx.try_recv().ok();
        }
        detail.clone()
    }

    pub fn card(&mut self, ui: &mut Ui, theme: &Theme, line: usize, cmds: &mut Vec<Command>) {
        let Some(Some(commit)) = self.line(line).map(|c| c.cloned()) else { return };
        ui.set_max_width(440.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&commit.hash[..commit.hash.len().min(8)]).monospace().color(theme.accent));
            ui.label(RichText::new(&commit.author).strong().color(theme.text));
            ui.label(RichText::new(commit.date()).color(theme.text_dim));
        });
        ui.add_space(4.0);
        let detail = self.detail(&commit.hash, ui.ctx());
        let message = detail.as_ref().map_or(commit.subject.as_str(), |d| d.0.as_str());
        ui.add(egui::Label::new(RichText::new(message).color(theme.text)).wrap());
        if detail.is_none() {
            ui::skeleton(ui, theme, 1);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Open Changes").clicked() {
                cmds.push(commit.diff(&self.path));
            }
            if let Some((host, url)) = detail.and_then(|d| d.1)
                && ui.button(format!("Open on {host}")).on_hover_text(&url).clicked()
            {
                ui::open_url(&url);
            }
        });
    }

    pub fn loading(&self) -> bool { self.lines.is_none() && !self.failed && (self.pending.is_some() || self.wait.is_some()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_suppressed_metadata_zero_hash_and_quoted_path() {
        let hash = "a".repeat(40);
        let data = format!("{hash} 3 1 2\nauthor Ada Lovelace\nauthor-time 0\nauthor-tz -0500\nsummary Add notes\nfilename \"sub/a\\tb\"\n\tone\n{hash} 4 2\n\ttwo\n{} 1 3 1\n\tnew\n", "0".repeat(40));
        let rows = parse(&data);
        let commit = rows[0].as_ref().unwrap();
        assert!(Arc::ptr_eq(commit, rows[1].as_ref().unwrap()));
        assert_eq!((&*commit.author, &*commit.file), ("Ada Lovelace", "sub/a\tb"));
        assert!(commit.tip().contains("1969-12-31 19:00 -0500"));
        assert!(rows[2].is_none());
        assert!(parse("bad header\n\tignored").is_empty());
        let mapped = map_edits(&rows, "@@ -0,0 +1 @@\n+head\n@@ -2 +3,2 @@\n");
        assert_eq!(mapped.len(), 5);
        assert!(mapped[0].is_none() && mapped[1].is_some() && mapped[2..].iter().all(Option::is_none));
    }

    #[test]
    fn builds_commit_urls_for_known_hosts() {
        let url = |remote: &str| web_url(remote, "abc").map(|(host, url)| format!("{host} {url}"));
        assert_eq!(url("git@github.com:o/r.git").as_deref(), Some("GitHub https://github.com/o/r/commit/abc"));
        assert_eq!(url("https://tok:x@github.com/o/r/\n").as_deref(), Some("GitHub https://github.com/o/r/commit/abc"));
        assert_eq!(url("ssh://git@ssh.github.com:443/o/r.git").as_deref(), Some("GitHub https://github.com/o/r/commit/abc"));
        assert_eq!(url("ssh://git@gitlab.example.com:2222/g/sub/r.git").as_deref(), Some("GitLab https://gitlab.example.com/g/sub/r/-/commit/abc"));
        assert_eq!(url("https://ada@bitbucket.org/team/r.git").as_deref(), Some("Bitbucket https://bitbucket.org/team/r/commits/abc"));
        for other in ["origin", "", r"C:\repos\r", "/srv/git/r.git", "https://example.com/o/r", "git@github.com:"] {
            assert_eq!(url(other), None, "{other}");
        }
    }

    #[test]
    fn blames_a_repository_file_and_its_unsaved_edits() {
        let dir = std::env::temp_dir().join(format!("halo-blame-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "Ada"]);
        std::fs::write(dir.join("a b.txt"), "one\ntwo\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "First lines"]);
        std::fs::write(dir.join("a b.txt"), "one\ntwo\nthree\n").unwrap();
        let (path, ctx) = (dir.join("a b.txt"), egui::Context::default());
        let mut blame = Blame::default();
        let mut settle = |text: &str, version: u64, dirty: bool| {
            for _ in 0..500 {
                blame.update(&path, &Rope::from_str(text), version, dirty, &ctx);
                if blame.version == Some(version) { break; }
                std::thread::sleep(Duration::from_millis(10));
            }
            (0..4).map(|n| blame.line(n).map(|c| c.map(|c| (c.author.clone(), c.subject.clone(), c.file.clone())))).collect::<Vec<_>>()
        };
        let commit = Some(Some(("Ada".to_owned(), "First lines".to_owned(), "a b.txt".to_owned())));
        assert_eq!(settle("one\ntwo\nthree\n", 1, false), [commit.clone(), commit.clone(), Some(None), Some(None)]);
        assert_eq!(settle("zero\none\ntwo\nthree\n", 2, true), [Some(None), commit.clone(), commit, Some(None)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
