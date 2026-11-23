use crate::command::Command;
use crate::explorer::{file_icon, state_color};
use crate::git::{Change, FileState};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, CornerRadius, FontId, Id, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

const BASE: &str = "halo-agent-base";
const CURRENT: &str = "halo-agent-current";
const POLL: Duration = Duration::from_secs(2);
const NOTE: f32 = 26.0;
const HEADER: f32 = 26.0;
const ROW: f32 = 22.0;
const NOT_GIT: &str = "Reviewing agent changes needs a Git folder.";
const ALL: [(&str, &str); 3] =
    [(icons::ARROW_CLOCKWISE, "Refresh"), (icons::CHECK, "Keep All"), (icons::ARROW_COUNTER_CLOCKWISE, "Undo All")];
const ONE: [(&str, &str); 2] = [(icons::CHECK, "Keep"), (icons::ARROW_COUNTER_CLOCKWISE, "Undo")];

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Reset,
    Refresh,
    Keep(Option<Vec<String>>),
    Undo(Option<Vec<String>>),
}

#[derive(Debug, Default)]
struct Snapshot {
    tree: String,
    entries: Vec<(Change, Option<(usize, usize)>)>,
    undone: Vec<PathBuf>,
}

struct Repo {
    top: PathBuf,
    local: PathBuf,
    git_dir: String,
}

impl Repo {
    fn open(dir: &Path) -> Result<Repo, String> {
        let out = crate::git::exec(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir"]).map_err(|_| NOT_GIT.to_owned())?;
        let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
        let (Some(top), Some(git_dir)) = (lines.next(), lines.next()) else { return Err(NOT_GIT.into()) };
        Ok(Repo { top: crate::wsl::local(dir, top), local: crate::wsl::local(dir, git_dir), git_dir: git_dir.to_owned() })
    }

    fn git(&self, index: &str, args: &[&str]) -> Result<String, String> {
        let index = format!("{}/{index}", self.git_dir);
        crate::git::exec_env(&self.top, &[("GIT_INDEX_FILE", &index), ("GIT_LITERAL_PATHSPECS", "1")], args)
    }

    fn each(&self, index: &str, head: &[&str], paths: &[&str]) -> Result<(), String> {
        for chunk in paths.chunks(100) {
            self.git(index, &[head, &["--"], chunk].concat())?;
        }
        Ok(())
    }
}

fn reset(repo: &Repo) -> Result<(), String> {
    for name in [BASE, CURRENT] {
        if std::fs::copy(repo.local.join("index"), repo.local.join(name)).is_err() {
            repo.git(name, &["read-tree", "--empty"])?;
        }
    }
    repo.git(BASE, &["add", "-A"]).map(drop)
}

fn numstat(text: &str) -> HashMap<String, Option<(usize, usize)>> {
    let parse = |line: &str| {
        let mut parts = line.splitn(3, '\t');
        let (added, removed, path) = (parts.next()?, parts.next()?, parts.next()?);
        Some((path.to_owned(), added.parse().ok().zip(removed.parse().ok())))
    };
    text.split('\0').filter_map(parse).collect()
}

fn snapshot(repo: &Repo) -> Result<Snapshot, String> {
    repo.git(CURRENT, &["add", "-A"])?;
    let tree = repo.git(BASE, &["write-tree"])?.trim().to_owned();
    let diff = |format: &str| repo.git(CURRENT, &["diff", "--cached", "--no-renames", "--no-ext-diff", "--no-color", "-z", format, &tree]);
    let counts = numstat(&diff("--numstat")?);
    let changes = crate::git::parse_name_status(&diff("--name-status")?);
    let entries = changes.into_iter().map(|c| {
        let lines = counts.get(&c.path).copied().flatten();
        (c, lines)
    });
    Ok(Snapshot { tree, entries: entries.collect(), undone: vec![] })
}

fn undo(repo: &Repo, paths: Option<Vec<String>>, remove: fn(&Path) -> Result<(), String>) -> Result<Snapshot, String> {
    let before = snapshot(repo)?;
    let chosen: Vec<&Change> = before.entries.iter().map(|e| &e.0).filter(|c| paths.as_ref().is_none_or(|p| p.contains(&c.path))).collect();
    let mut undone = vec![];
    for change in chosen.iter().filter(|c| c.state == FileState::Added) {
        let full = repo.top.join(&change.path);
        remove(&full).map_err(|e| format!("Could not remove {}: {e}", change.path))?;
        undone.push(full);
    }
    let restore: Vec<&str> = chosen.iter().filter(|c| c.state != FileState::Added).map(|c| c.path.as_str()).collect();
    repo.each(BASE, &["checkout-index", "-f"], &restore)?;
    undone.extend(restore.iter().map(|p| repo.top.join(p)));
    Ok(Snapshot { undone, ..snapshot(repo)? })
}

fn run(dir: &Path, op: Op, remove: fn(&Path) -> Result<(), String>) -> Result<Snapshot, String> {
    let repo = Repo::open(dir)?;
    match op {
        Op::Reset => reset(&repo)?,
        _ if !repo.local.join(BASE).is_file() => return Err("The agent changes baseline is missing. Run Agent Changes: Reset Baseline.".into()),
        Op::Refresh => {}
        Op::Keep(None) => _ = repo.git(BASE, &["add", "-A"])?,
        Op::Keep(Some(paths)) => repo.each(BASE, &["add", "-A"], &paths.iter().map(String::as_str).collect::<Vec<_>>())?,
        Op::Undo(paths) => return undo(&repo, paths, remove),
    }
    snapshot(&repo)
}

fn trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

#[derive(Default)]
pub struct Review {
    dir: Option<PathBuf>,
    snapshot: Option<Snapshot>,
    error: Option<String>,
    job: Option<Receiver<Result<Snapshot, String>>>,
    queued: Option<Op>,
    polled: Option<Instant>,
    alive: bool,
    confirm: Option<Option<String>>,
}

impl Review {
    fn run(&mut self, op: Op, ctx: &egui::Context) {
        let Some(dir) = self.dir.clone() else {
            self.error = Some(NOT_GIT.into());
            return;
        };
        if self.job.is_some() {
            if op != Op::Refresh || self.queued.is_none() {
                self.queued = Some(op);
            }
            return;
        }
        self.polled = Some(Instant::now());
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(run(&dir, op, trash));
            ctx.request_repaint();
        });
        self.job = Some(rx);
    }

    fn reset(&mut self, dir: Option<&Path>, ctx: &egui::Context) {
        self.dir = dir.map(Path::to_path_buf);
        (self.snapshot, self.error, self.queued) = (None, None, None);
        self.run(Op::Reset, ctx);
    }

    fn count(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |s| s.entries.len())
    }

    pub fn started(&mut self, dir: Option<&Path>, ctx: &egui::Context) {
        if dir.is_some() && self.dir.as_deref() == dir && self.count() > 0 {
            self.run(Op::Refresh, ctx);
        } else {
            self.reset(dir, ctx);
        }
    }

    pub fn exec(&mut self, cmd: &Command, folder: Option<&Path>, ctx: &egui::Context) {
        let tracking = self.snapshot.is_some();
        match cmd {
            Command::AgentChangesKeepAll if tracking => self.run(Op::Keep(None), ctx),
            Command::AgentChangesUndoAll if self.count() > 0 => self.confirm = Some(None),
            Command::AgentChangesShow if tracking => self.run(Op::Refresh, ctx),
            Command::AgentChangesResetBaseline => self.reset(folder, ctx),
            _ => {}
        }
    }

    pub fn tick(&mut self, alive: bool, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        let done = match self.job.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Disconnected)) => Some(Err("Agent change review stopped unexpectedly.".into())),
            _ => None,
        };
        if let Some(result) = done {
            self.job = None;
            match result {
                Ok(mut snapshot) => {
                    let undone = std::mem::take(&mut snapshot.undone);
                    cmds.extend(undone.iter().filter(|p| !p.exists()).map(|p| Command::PathChanged(p.clone(), None)));
                    if !undone.is_empty() {
                        cmds.push(Command::RefreshExplorer);
                    }
                    (self.snapshot, self.error) = (Some(snapshot), None);
                }
                Err(error) => self.error = Some(error),
            }
            if let Some(op) = self.queued.take() {
                self.run(op, ctx);
            }
        }
        let ended = std::mem::replace(&mut self.alive, alive) && !alive;
        if self.snapshot.is_some() && (ended || alive && self.polled.is_none_or(|t| t.elapsed() >= POLL)) {
            self.run(Op::Refresh, ctx);
        }
        if alive && self.snapshot.is_some() {
            ctx.request_repaint_after(POLL);
        }
    }

    pub fn height(&self, max: f32) -> f32 {
        let note = if self.error.is_some() { NOTE } else { 0.0 };
        let list = match self.count() {
            0 => 0.0,
            rows => HEADER + rows as f32 * ROW + 4.0,
        };
        (note + list).min((max * 0.45).max(note))
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme.border));
        if let Some(error) = &self.error {
            let (_, bar) = ui.allocate_space(vec2(rect.width(), NOTE));
            let color = if error == NOT_GIT { theme.text_dim } else { theme.error };
            ui.painter().rect_filled(bar, CornerRadius::ZERO, ui::mix(theme.panel, color, 0.12));
            let clip = ui.painter().with_clip_rect(bar.shrink2(vec2(10.0, 0.0)));
            clip.text(pos2(bar.left() + 10.0, bar.center().y), Align2::LEFT_CENTER, error, FontId::proportional(12.5), color);
        }
        let Some(snapshot) = self.snapshot.as_ref().filter(|s| !s.entries.is_empty()) else { return };
        let busy = self.job.is_some();
        let (_, head) = ui.allocate_space(vec2(rect.width(), HEADER));
        let y = head.center().y;
        let title = format!("AGENT CHANGES  {}", snapshot.entries.len());
        ui.painter().text(pos2(head.left() + 12.0, y), Align2::LEFT_CENTER, title, FontId::proportional(11.0), theme.text_dim);
        if busy {
            ui::spinner(ui, pos2(head.right() - 80.0, y), 5.0, theme.text_dim);
        }
        let (mut op, mut confirm) = (None, None);
        match crate::scm::buttons(ui, theme, head, !busy, &ALL) {
            Some(0) => op = Some(Op::Refresh),
            Some(1) => op = Some(Op::Keep(None)),
            Some(_) => confirm = Some(None),
            None => {}
        }
        ScrollArea::vertical().id_salt("agent_changes").auto_shrink([false; 2]).show(ui, |ui| {
            for (change, lines) in &snapshot.entries {
                let (_, rect) = ui.allocate_space(vec2(ui.available_width(), ROW));
                let id = Id::new(("agent_change", &change.path));
                let resp = ui.interact(rect, id, Sense::click());
                let hovered = ui.rect_contains_pointer(rect);
                ui::row_bg(ui, theme, rect, id, hovered, false);
                ui::item(ui, &resp, ui::Role::ListItem, &format!("{}, {:?}", change.path, change.state), false);
                let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
                let color = state_color(theme, Some(change.state));
                let badge = match change.state {
                    FileState::Added => "A",
                    FileState::Deleted => "D",
                    _ => "M",
                };
                let y = rect.center().y;
                let small = FontId::proportional(11.5);
                ui.painter().text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, small.clone(), color);
                let mut right = rect.right() - 30.0;
                if hovered {
                    right -= 44.0;
                    match crate::scm::buttons(ui, theme, rect.with_max_x(rect.right() - 22.0), !busy, &ONE) {
                        Some(0) => op = Some(Op::Keep(Some(vec![change.path.clone()]))),
                        Some(_) => confirm = Some(Some(change.path.clone())),
                        None => {}
                    }
                } else if let Some((added, removed)) = lines {
                    let drawn = ui.painter().text(pos2(right, y), Align2::RIGHT_CENTER, format!("−{removed}"), small.clone(), theme.deleted);
                    let drawn = ui.painter().text(pos2(drawn.left() - 5.0, y), Align2::RIGHT_CENTER, format!("+{added}"), small.clone(), theme.added);
                    right = drawn.left() - 6.0;
                }
                let painter = ui.painter().with_clip_rect(rect.with_max_x(right));
                let x = rect.left() + 14.0;
                if !crate::icon_theme::paint(&painter, pos2(x + 7.5, y), 15.0, name, crate::icon_theme::Kind::File, 1.0) {
                    painter.text(pos2(x, y), Align2::LEFT_CENTER, file_icon(name), FontId::proportional(15.0), theme.text_dim);
                }
                let drawn = painter.text(pos2(x + 21.0, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
                painter.text(pos2(drawn.right() + 8.0, y), Align2::LEFT_CENTER, dir, small, theme.text_dim);
                if resp.clicked() {
                    let args = ["diff", &snapshot.tree, "--", &change.path].map(str::to_owned).to_vec();
                    cmds.push(Command::OpenDiff(format!("{name} (Agent Changes)"), args));
                }
            }
        });
        if let Some(op) = op {
            self.run(op, ui.ctx());
        }
        if confirm.is_some() {
            self.confirm = confirm;
        }
    }

    pub fn dialog(&mut self, ui: &Ui) {
        let Some(target) = self.confirm.clone() else { return };
        let count = self.count();
        let mut choice = None;
        let modal = ui::modal(ui.ctx(), "agent_changes_undo", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(360.0);
            ui.heading("Undo agent changes");
            ui.label(match &target {
                Some(path) => format!("Undo the agent's changes to {path}?"),
                None => format!("Undo all {count} agent changes?"),
            });
            ui.label("Edited and deleted files get back their content from when the session started. Files the agent created move to the recycle bin.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if keys.primary(ui, "Undo", true) {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if choice.is_none() && modal.should_close() {
            choice = Some(false);
        }
        if let Some(confirmed) = choice {
            self.confirm = None;
            if confirmed {
                self.run(Op::Undo(target.map(|p| vec![p])), ui.ctx());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput, Rect};

    fn remove(path: &Path) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }

    fn states(snapshot: &Snapshot) -> Vec<(&str, FileState)> {
        snapshot.entries.iter().map(|(c, _)| (c.path.as_str(), c.state)).collect()
    }

    #[test]
    fn reviews_keeps_and_undoes_agent_edits_without_touching_the_real_index() {
        let dir = std::env::temp_dir().join(format!("halo-agent-changes-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let git = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        git(&["init", "-q", "-b", "trunk"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        git(&["config", "core.autocrlf", "false"]);
        write(".gitignore", "out/\n");
        write("a.txt", "one\ntwo\n");
        write("sub/b.txt", "bee\n");
        write("staged.txt", "s\n");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        write("staged.txt", "stash me\n");
        git(&["stash", "-q"]);
        write("staged.txt", "staged\n");
        git(&["add", "staged.txt"]);
        write("untracked.txt", "mine\n");
        let real = || (git(&["ls-files", "-s"]), git(&["stash", "list"]), git(&["diff", "--cached", "--name-status"]));
        let before = real();

        assert!(run(&dir, Op::Refresh, remove).unwrap_err().contains("missing"));
        assert!(run(&dir.join("sub"), Op::Reset, remove).unwrap().entries.is_empty());
        write("a.txt", "one\nTWO\nthree\n");
        std::fs::remove_file(dir.join("sub/b.txt")).unwrap();
        write("sub/new.txt", "fresh\n");
        write("untracked.txt", "changed\n");
        std::fs::create_dir_all(dir.join("out")).unwrap();
        write("out/build.log", "ignored\n");
        let seen = run(&dir, Op::Refresh, remove).unwrap();
        let mut list = states(&seen);
        list.sort_by_key(|e| e.0);
        assert_eq!(
            list,
            [("a.txt", FileState::Modified), ("sub/b.txt", FileState::Deleted), ("sub/new.txt", FileState::Added), ("untracked.txt", FileState::Modified)]
        );
        assert_eq!(seen.entries.iter().find(|(c, _)| c.path == "a.txt").unwrap().1, Some((2, 1)));
        assert_eq!(git(&["show", &format!("{}:untracked.txt", seen.tree)]), "mine\n");

        let kept = run(&dir, Op::Keep(Some(vec!["a.txt".into()])), remove).unwrap();
        assert!(states(&kept).iter().all(|(p, _)| *p != "a.txt"));
        assert_eq!(kept.entries.len(), 3);

        let undone = run(&dir, Op::Undo(Some(vec!["untracked.txt".into()])), remove).unwrap();
        assert_eq!(read("untracked.txt"), "mine\n");
        assert_eq!(undone.undone, [dir.join("untracked.txt")]);
        assert_eq!(undone.entries.len(), 2);

        let all = run(&dir, Op::Undo(None), remove).unwrap();
        assert!(all.entries.is_empty());
        assert_eq!(read("sub/b.txt"), "bee\n");
        assert!(!dir.join("sub/new.txt").exists());
        assert_eq!(read("a.txt"), "one\nTWO\nthree\n");
        assert!(dir.join("out/build.log").exists());

        write("a.txt", "again\n");
        assert_eq!(states(&run(&dir, Op::Refresh, remove).unwrap()), [("a.txt", FileState::Modified)]);
        assert!(run(&dir, Op::Keep(None), remove).unwrap().entries.is_empty());
        assert_eq!(real(), before);
        assert_eq!(git(&["stash", "list"]).lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_git_folders_get_a_note() {
        let dir = std::env::temp_dir().join(format!("halo-agent-changes-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        if crate::git::exec(&dir, &["rev-parse", "--show-toplevel"]).is_err() {
            assert_eq!(run(&dir, Op::Reset, remove).unwrap_err(), NOT_GIT);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rows_open_the_baseline_diff_and_undo_all_asks_first() {
        let ctx = egui::Context::default();
        let mut review = Review::default();
        let change = Change { path: "src/a.rs".into(), state: FileState::Modified };
        review.snapshot = Some(Snapshot { tree: "abc123".into(), entries: vec![(change, Some((3, 1)))], undone: vec![] });
        assert_eq!(review.height(600.0), HEADER + ROW + 4.0);
        let frame = |review: &mut Review, events: Vec<Event>| {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 600.0))), events, focused: true, ..Default::default() };
            let mut cmds = vec![];
            let mut out = ctx.run_ui(input, |ui| review.ui(ui, &Theme::dark(), &mut cmds));
            out.textures_delta.clear();
            cmds
        };
        let click = |review: &mut Review, x: f32, y: f32| {
            let pos = pos2(x, y);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(review, vec![Event::PointerMoved(pos)]);
            frame(review, vec![button(true)]);
            frame(review, vec![button(false)])
        };
        let cmds = click(&mut review, 60.0, HEADER + ROW / 2.0);
        let args = ["diff", "abc123", "--", "src/a.rs"].map(str::to_owned).to_vec();
        assert_eq!(cmds, [Command::OpenDiff("a.rs (Agent Changes)".into(), args)]);
        assert!(review.confirm.is_none());
        click(&mut review, 244.0, HEADER / 2.0);
        assert_eq!(review.confirm, Some(None));
    }
}
