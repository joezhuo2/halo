use super::buffer::{After, Document, Kind, Sel};
use super::diff;
use super::highlight::Highlighter;
use super::merge::{Merge, Row, State};
use super::{Editors, notice};
use crate::command::Command;
use crate::ui;
use eframe::egui;
use ropey::Rope;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

pub(super) struct Stages {
    top: PathBuf,
    rel: String,
    texts: [String; 3],
    names: [String; 2],
}

fn stages(path: &Path) -> Result<Stages, String> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let out = crate::git::exec(&dir, &["rev-parse", "--show-toplevel", "--show-prefix"]).map_err(|_| format!("{name} is not in a Git repository."))?;
    let mut lines = out.lines();
    let top = crate::wsl::local(&dir, lines.next().unwrap_or_default().trim());
    let rel = format!("{}{name}", lines.next().unwrap_or_default().trim());
    let show = |stage: u8| crate::git::exec(&top, &["show", &format!(":{stage}:{rel}")]);
    let (base, current, incoming) = (show(1), show(2), show(3));
    if current.is_err() && incoming.is_err() {
        return Err(format!("{name} has no merge conflict in Git."));
    }
    let texts = [base, current, incoming].map(Result::unwrap_or_default);
    if texts.iter().any(|t| t.as_bytes().iter().take(8000).any(|b| *b == 0)) {
        return Err(format!("{name} is a binary file, so the merge editor cannot show it."));
    }
    let label = |rev: &str| {
        let hash = crate::git::exec(&top, &["rev-parse", "-q", "--verify", rev]).ok()?;
        let named = crate::git::exec(&top, &["name-rev", "--name-only", "--no-undefined", rev]).unwrap_or_default();
        let named = named.trim().trim_end_matches("^0").trim_end_matches("~0");
        Some(if named.is_empty() { hash.trim().chars().take(8).collect() } else { named.to_owned() })
    };
    let incoming = ["MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"].into_iter().find_map(label).unwrap_or_default();
    let branch = crate::git::exec(&top, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default().trim().to_owned();
    let current = if branch.is_empty() || branch == "HEAD" { label("HEAD").unwrap_or_default() } else { branch };
    Ok(Stages { top, rel, texts, names: [incoming, current] })
}

enum Msg {
    Loaded(PathBuf, Result<Stages, String>),
    Staged(PathBuf, Result<String, String>),
}

pub(super) struct Merges {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    confirm: Option<u64>,
    at: Option<usize>,
    refresh: bool,
}

impl Default for Merges {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, confirm: None, at: None, refresh: false }
    }
}

struct Pane {
    rope: Rope,
    hl: Highlighter,
}

pub(super) struct MergeTab {
    pub title: String,
    path: PathBuf,
    top: PathBuf,
    rel: String,
    names: [String; 2],
    model: Merge,
    panes: [Pane; 3],
    marks: HashMap<(usize, usize), Vec<Range<usize>>>,
    synced: u64,
    ranges: Vec<Range<usize>>,
    rows: Vec<Row>,
    pub pads: Vec<(usize, u32)>,
    pub base: bool,
    pub scroll: f32,
    pub text_top: f32,
    pub line_h: f32,
}

impl MergeTab {
    fn new(path: &Path, stages: Stages) -> Self {
        let model = Merge::new(&stages.texts[0], &stages.texts[1], &stages.texts[2]);
        let pane = |lines: &[String]| {
            let rope = Rope::from_str(&lines.concat());
            Pane { hl: Highlighter::new(Some(path), &rope), rope }
        };
        let panes = [pane(&model.incoming), pane(&model.current), pane(&model.base)];
        let mut marks = HashMap::new();
        for k in model.conflicts() {
            let c = &model.chunks[k];
            let base = &model.base[c.base.clone()];
            for (side, lines, range) in [(0, &model.incoming, &c.incoming), (1, &model.current, &c.current)] {
                let changed = &lines[range.clone()];
                for h in diff::diff(base, changed) {
                    for (j, r) in h.right.clone().enumerate() {
                        let text = changed[r].trim_end_matches('\n');
                        let found = match base.get(h.left.start + j).filter(|_| j < h.left.len()) {
                            Some(old) if old.len() + text.len() < 4000 => diff::intra(old.trim_end_matches('\n'), text).1,
                            _ => std::iter::once(0..text.chars().count()).collect(),
                        };
                        marks.insert((side, range.start + r), found);
                    }
                }
            }
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self {
            title: format!("Merging: {name}"),
            path: path.to_path_buf(),
            top: stages.top,
            rel: stages.rel,
            names: stages.names,
            model,
            panes,
            marks,
            synced: 0,
            ranges: vec![],
            rows: vec![],
            pads: vec![],
            base: false,
            scroll: 0.0,
            text_top: 0.0,
            line_h: 16.0,
        }
    }

    pub fn sync(&mut self, buf: &Document) {
        if self.synced != buf.version {
            self.ranges = self.model.sync(&buf.rope.to_string());
            (self.rows, self.pads) = self.model.align(&self.ranges, self.base);
            self.synced = buf.version;
        }
    }

    fn conflict_at(&self, line: usize) -> Option<usize> {
        let inside = |k: &usize| self.ranges[*k].start <= line && line < self.ranges[*k].end.max(self.ranges[*k].start + 1);
        self.model.conflicts().find(inside).or_else(|| self.model.conflicts().find(|k| self.ranges[*k].start >= line)).or_else(|| self.model.conflicts().last())
    }

    fn step(&self, line: usize, forward: bool) -> Option<usize> {
        let open: Vec<usize> = self.model.conflicts().filter(|k| self.model.chunks[*k].state == State::Base).collect();
        match forward {
            true => open.iter().find(|k| self.ranges[**k].start > line).or(open.first()).copied(),
            false => open.iter().rev().find(|k| self.ranges[**k].start < line).or(open.last()).copied(),
        }
    }

    fn set(&mut self, buf: &mut Document, sels: &mut Vec<Sel>, picks: &[(usize, State)]) {
        self.sync(buf);
        let total = diff::split(&buf.rope.to_string()).len();
        let mut edits = vec![];
        for &(k, state) in picks {
            let r = self.ranges[k].clone();
            edits.push((buf.line_start(r.start), buf.line_start(r.end), self.model.text(k, state, r.end < total)));
            self.model.chunks[k].state = state;
        }
        buf.apply(sels, edits, Kind::Other, After::Map);
    }
}

impl Editors {
    pub fn open_merge(&mut self, path: &Path, ctx: &egui::Context) {
        let (tx, ctx, path) = (self.merges.tx.clone(), ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let found = stages(&path);
            let _ = tx.send(Msg::Loaded(path, found));
            ctx.request_repaint();
        });
    }

    pub(super) fn take_merge_refresh(&mut self) -> bool {
        std::mem::take(&mut self.merges.refresh)
    }

    pub(super) fn poll_merges(&mut self) {
        while let Ok(msg) = self.merges.rx.try_recv() {
            match msg {
                Msg::Loaded(path, Ok(stages)) => self.place_merge(&path, stages),
                Msg::Staged(path, Ok(rel)) => {
                    while let Some((g, t)) = self.merge_tab_of(&path) {
                        self.close_tab(g, t);
                    }
                    self.merges.refresh = true;
                    self.banner = notice(format!("Merge completed, {rel} is staged."));
                }
                Msg::Loaded(_, Err(error)) | Msg::Staged(_, Err(error)) => self.banner = Some((error, None)),
            }
        }
    }

    fn merge_tab_of(&self, path: &Path) -> Option<(usize, usize)> {
        let found = |group: &super::Group| group.tabs.iter().position(|t| t.merge.as_ref().is_some_and(|m| m.path == path));
        self.groups.iter().enumerate().find_map(|(g, group)| Some((g, found(group)?)))
    }

    fn place_merge(&mut self, path: &Path, stages: Stages) {
        let id = match self.find_doc(path) {
            Some(id) => id,
            None => match Document::load(path) {
                Ok(buf) => self.insert_doc(buf),
                Err(error) => {
                    self.banner = Some((error, None));
                    return;
                }
            },
        };
        self.active = self.active.min(self.groups.len() - 1);
        let g = self.active;
        let t = match self.groups[g].tabs.iter().position(|t| t.doc == id && t.diff.is_none()) {
            Some(t) => t,
            None => {
                self.add_tab(id, vec![Sel::caret(0)]);
                self.groups[g].tabs.len() - 1
            }
        };
        let mut m = MergeTab::new(path, stages);
        let (Some(doc), tab) = (self.docs.get_mut(&id), &mut self.groups[g].tabs[t]) else { return };
        if !doc.buf.conflicts().is_empty() {
            let result = m.model.render();
            doc.buf.apply(&mut tab.sels, vec![(0, doc.buf.len(), result)], Kind::Other, After::Carets);
        }
        m.sync(&doc.buf);
        let first = m.step(0, true).or_else(|| m.model.conflicts().next()).map_or(0, |k| doc.buf.line_start(m.ranges[k].start));
        (tab.sels, tab.reveal, tab.preview, tab.merge) = (vec![Sel::caret(first)], Some(first), false, Some(Box::new(m)));
        (self.groups[g].active, self.want_focus) = (t, true);
    }

    pub(super) fn merge_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        use Command::*;
        if *cmd == MergeOpen {
            match self.active_path() {
                Some(path) => self.open_merge(&path, ctx),
                None => self.banner = notice("Save the file to open it in the merge editor.".to_owned()),
            }
            return true;
        }
        let ours = matches!(
            cmd,
            MergeAcceptIncoming | MergeAcceptCurrent | MergeAcceptBoth | MergeAcceptAllIncoming | MergeAcceptAllCurrent | MergeNextConflict
                | MergePrevConflict | MergeToggleBase | MergeComplete
        );
        if !ours && !matches!(cmd, NextChange | PrevChange) {
            return false;
        }
        let at = self.merges.at.take();
        let Some((g, t)) = self.active_tab().filter(|(g, t)| self.groups[*g].tabs[*t].merge.is_some()) else {
            if ours {
                self.banner = notice("Open a conflicted file in the merge editor first.".to_owned());
            }
            return ours;
        };
        let tab = &mut self.groups[g].tabs[t];
        let doc_id = tab.doc;
        let (Some(doc), Some(m)) = (self.docs.get_mut(&doc_id), tab.merge.as_mut()) else { return true };
        let buf = &mut doc.buf;
        buf.clamp(&mut tab.sels);
        m.sync(buf);
        let line = buf.line_of(tab.sels[0].head);
        let pick = match cmd {
            MergeAcceptIncoming => Some(State::Incoming),
            MergeAcceptCurrent => Some(State::Current),
            MergeAcceptBoth => Some(State::Both(true)),
            _ => None,
        };
        let mut finish = false;
        match cmd {
            MergeToggleBase => (m.base, m.synced) = (!m.base, 0),
            MergeComplete => match m.model.remaining() {
                0 => finish = true,
                _ => self.merges.confirm = Some(doc_id),
            },
            MergeAcceptAllIncoming | MergeAcceptAllCurrent => {
                let state = if *cmd == MergeAcceptAllIncoming { State::Incoming } else { State::Current };
                let picks: Vec<(usize, State)> = m.model.conflicts().map(|k| (k, state)).collect();
                m.set(buf, &mut tab.sels, &picks);
            }
            _ if pick.is_some() => match at.or_else(|| m.conflict_at(line)) {
                Some(k) => m.set(buf, &mut tab.sels, &[(k, pick.unwrap_or(State::Base))]),
                None => self.banner = notice("There are no conflicts in this file.".to_owned()),
            },
            _ => match m.step(line, matches!(cmd, MergeNextConflict | NextChange)) {
                Some(k) => {
                    let pos = buf.line_start(m.ranges[k].start);
                    (tab.sels, tab.reveal) = (vec![Sel::caret(pos)], Some(pos));
                }
                None => self.banner = notice("No unresolved conflicts.".to_owned()),
            },
        }
        self.want_focus = true;
        if finish {
            self.complete_merge(doc_id, ctx);
        }
        true
    }

    fn complete_merge(&mut self, doc_id: u64, ctx: &egui::Context) {
        let tabs = self.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.doc == doc_id);
        let Some((top, rel, path)) = tabs.filter_map(|t| t.merge.as_ref()).map(|m| (m.top.clone(), m.rel.clone(), m.path.clone())).next() else { return };
        if !self.save_doc(doc_id, false) {
            return;
        }
        let (tx, ctx) = (self.merges.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let staged = crate::git::exec(&top, &["add", "--", &rel]).map(|_| rel.clone()).map_err(|e| format!("Git could not stage {rel}: {e}"));
            let _ = tx.send(Msg::Staged(path, staged));
            ctx.request_repaint();
        });
    }

    pub(super) fn merge_dialog(&mut self, ctx: &egui::Context) {
        let Some(doc_id) = self.merges.confirm else { return };
        let tabs = self.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.doc == doc_id);
        let Some(left) = tabs.filter_map(|t| t.merge.as_ref()).map(|m| m.model.remaining()).next() else {
            self.merges.confirm = None;
            return;
        };
        let mut choice = None;
        let modal = ui::modal(ctx, "merge_complete").show(ctx, |ui| {
            ui::outline(ui);
            ui.set_width(360.0);
            ui.heading("Unresolved conflicts");
            let count = if left == 1 { "1 conflict is".to_owned() } else { format!("{left} conflicts are") };
            ui.label(format!("{count} still unresolved. Complete the merge and stage the file anyway?"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (i, label) in ["Complete Merge", "Cancel"].into_iter().enumerate() {
                    if ui.button(label).clicked() {
                        choice = Some(i);
                    }
                }
            });
        });
        if modal.should_close() {
            choice = Some(1);
        }
        if let Some(choice) = choice {
            self.merges.confirm = None;
            if choice == 0 {
                self.complete_merge(doc_id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::time::Duration;

    fn conflict_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-merge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args);
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"]] {
            run(args).unwrap();
        }
        let commit = |text: &str, message: &str| {
            std::fs::write(dir.join("a.txt"), text).unwrap();
            run(&["commit", "-q", "-am", message]).unwrap();
        };
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "base"]).unwrap();
        run(&["checkout", "-q", "-b", "feature"]).unwrap();
        commit("one\nTWO theirs\nthree\nfour\nFIVE\n", "theirs");
        run(&["checkout", "-q", "trunk"]).unwrap();
        commit("one\nTWO ours\nthree\nfour\nfive\n", "ours");
        assert!(run(&["merge", "-q", "feature"]).is_err());
        dir
    }

    #[test]
    fn loads_the_three_stages_and_branch_names_of_a_real_conflict() {
        let dir = conflict_repo("stages");
        let found = stages(&dir.join("a.txt")).unwrap();
        assert_eq!(found.rel, "a.txt");
        assert_eq!(found.texts, ["one\ntwo\nthree\nfour\nfive\n", "one\nTWO ours\nthree\nfour\nfive\n", "one\nTWO theirs\nthree\nfour\nFIVE\n"].map(str::to_owned));
        assert_eq!(found.names, ["feature".to_owned(), "trunk".to_owned()]);
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        assert!(stages(&dir.join("b.txt")).err().is_some_and(|e| e.contains("no merge conflict")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completes_a_merge_by_saving_and_staging_the_result() {
        let dir = conflict_repo("complete");
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        let file = dir.join("a.txt");
        editors.open(&file);
        let marked = std::fs::read_to_string(&file).unwrap();
        assert!(marked.contains("<<<<<<<"));
        editors.open_merge(&file, &ctx);
        let wait = |editors: &mut Editors, done: &dyn Fn(&Editors) -> bool| {
            for _ in 0..500 {
                editors.poll_merges();
                if done(editors) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out: {:?}", editors.banner);
        };
        wait(&mut editors, &|e| e.groups[0].tabs[0].merge.is_some());
        assert_eq!(editors.groups[0].tabs.len(), 1);
        let text = |e: &Editors| e.docs[&e.groups[0].tabs[0].doc].buf.rope.to_string();
        assert_eq!(text(&editors), "one\ntwo\nthree\nfour\nFIVE\n");
        editors.exec(&Command::Undo, &settings, &ctx);
        assert_eq!(text(&editors), marked.replace("\r\n", "\n"));
        editors.exec(&Command::Redo, &settings, &ctx);
        editors.exec(&Command::MergeComplete, &settings, &ctx);
        assert_eq!(editors.merges.confirm, Some(editors.groups[0].tabs[0].doc));
        editors.merges.confirm = None;
        editors.exec(&Command::MergeAcceptCurrent, &settings, &ctx);
        assert_eq!(text(&editors), "one\nTWO ours\nthree\nfour\nFIVE\n");
        editors.exec(&Command::MergeComplete, &settings, &ctx);
        wait(&mut editors, &|e| e.groups[0].tabs.is_empty());
        assert!(editors.take_merge_refresh());
        assert_eq!(crate::git::exec(&dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap(), "");
        assert_eq!(crate::git::exec(&dir, &["show", ":a.txt"]).unwrap(), "one\nTWO ours\nthree\nfour\nFIVE\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
