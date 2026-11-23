use super::{Scm, prompt};
use crate::command::{Command, GitCommand as Op};
use crate::git::{Git, exec, ops};
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Frame, Margin, RichText, ScrollArea, Sense, Ui};
use std::path::{Path, PathBuf};

type Choices = Vec<(String, String)>;

enum Dialog {
    List(Op),
    Pick(Op, Choices, String),
    Remote(Op, String),
    Form(Op, String, String, String),
    Confirm(Op, String, String),
    OpenClone(PathBuf),
}

#[derive(Default)]
pub(super) struct Ops {
    root: Option<PathBuf>,
    pending: Option<(Op, Option<String>)>,
    dialog: Option<Dialog>,
    pub(super) stashes_open: bool,
}

fn entry(ui: &mut Ui, op: Op, cmds: &mut Vec<Command>) {
    if ui.button(op.label().trim_start_matches("Git: ")).clicked() {
        cmds.push(Command::Git(op));
        ui.close();
    }
}

pub(super) fn menu(ui: &mut Ui, git: &Git, cmds: &mut Vec<Command>) {
    use Op::*;
    for op in [AmendMessage, Merge, Rebase, CherryPick, Stash, PopStash, Tags, Remotes, Worktrees, Clone] {
        entry(ui, op, cmds);
    }
    if git.details().and_then(|d| d.as_ref().ok()).is_some_and(|d| d.has_parent && d.operation.is_none()) {
        entry(ui, UndoCommit, cmds);
    }
    if git.can_init() {
        entry(ui, Init, cmds);
    }
}

pub(super) fn in_progress(ui: &mut Ui, theme: &Theme, git: &Git, cmds: &mut Vec<Command>) {
    let operation = match git.details() {
        Some(Ok(d)) => d.operation,
        Some(Err(e)) => return drop(ui.label(RichText::new(e).color(theme.error))),
        None => None,
    };
    let Some(operation) = operation else { return };
    Frame::new().inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.label(RichText::new(format!("{} in progress", operation[..1].to_uppercase() + &operation[1..])).color(theme.warning));
        ui.add_enabled_ui(!git.busy(), |ui| ui.horizontal_wrapped(|ui| {
            for (op, label) in [(Op::Continue, "Continue"), (Op::Abort, "Abort..."), (Op::Skip, "Skip")] {
                if (op != Op::Skip || operation == "rebase") && ui.button(label).clicked() {
                    cmds.push(Command::Git(op));
                }
            }
        }));
    });
}

fn choices(op: Op, git: &Git, d: &ops::Details) -> Choices {
    use Op::*;
    match op {
        Merge | Rebase => git.branches().iter().filter(|b| Some(b.as_str()) != git.branch())
            .map(|b| (b.clone(), b.clone())).collect(),
        CherryPick | BranchAt => git.log().iter().map(|c| {
            (c.hash.clone(), format!("{}  {}", &c.hash[..8.min(c.hash.len())], c.subject))
        }).collect(),
        Tags | PushTag | DeleteTag => d.tags.iter().map(|t| (t.clone(), t.clone())).collect(),
        Remotes | RemoteUrl | RemoveRemote => d.remotes.iter().map(|r| {
            let url = if r.fetch_url == r.push_url { r.fetch_url.clone() } else { format!("{} → {}", r.fetch_url, r.push_url) };
            (r.name.clone(), format!("{}  {url}", r.name))
        }).collect(),
        Worktrees | RemoveWorktree | OpenWorktree => d.worktrees.iter().enumerate()
            .filter(|(i, w)| op == Worktrees || (!w.bare && (op != RemoveWorktree || (*i != 0 && Some(w.path.as_path()) != git.top()))))
            .map(|(_, w)| (w.path.to_string_lossy().into_owned(), format!("{}  {}", w.branch, w.path.display()))).collect(),
        ApplyStash | DropStash | PopStash | ShowStash => d.stashes.iter().map(|s| {
            (s.reference.clone(), format!("{}  {}  ({}, {})", s.reference, s.message, s.branch, s.age))
        }).collect(),
        _ => vec![],
    }
}

impl Scm {
    fn ops_root(&mut self, git: &Git) {
        if self.ops.root.as_deref() != git.root() {
            self.ops = Ops { root: git.root().map(Path::to_path_buf), ..Ops::default() };
        }
    }

    fn generate(&mut self, git: &Git, ctx: &egui::Context) {
        if self.ai.take().is_some() {
            return;
        }
        match git.top() {
            Some(top) => self.ai = Some(crate::commit_ai::Job::start(top, !git.changes(true).is_empty(), ctx)),
            None => ui::notify_warning("Open a Git repository to generate a commit message."),
        }
    }

    pub fn start(&mut self, op: Op, target: Option<String>, git: &mut Git, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        use Op::*;
        self.ops_root(git);
        if op == GenerateMessage {
            self.generate(git, ctx);
            return;
        }
        if git.busy() {
            return;
        }
        if op == Clone {
            self.ops.dialog = Some(Dialog::Form(op, String::new(), String::new(), String::new()));
            return;
        }
        if op == Init && git.can_init() {
            git.init();
            return;
        }
        git.request_details(ctx);
        let Some(result) = git.details() else {
            if git.loading() {
                self.ops.pending = Some((op, target));
            } else {
                git.set_error("Open a Git repository to use this command.".into());
            }
            return;
        };
        let d = match result {
            Ok(d) => d,
            Err(e) => { git.set_error(e.clone()); return; }
        };
        let top = git.top().unwrap().to_path_buf();
        if d.operation.is_some() && matches!(op, Amend | UndoCommit | Merge | Rebase | CherryPick) {
            git.set_error("Finish the current Git operation first.".into());
            return;
        }
        match op {
            Init => git.set_error("The open folder already belongs to a repository.".into()),
            Clone | GenerateMessage => unreachable!(),
            Amend | AmendMessage if d.head.is_empty() => git.set_error("There is no commit to amend.".into()),
            AmendMessage => self.message.clone_from(&d.message),
            Amend => {
                let message = self.message.clone();
                git.job(move || ops::amend(&top, &message).map(|_| None));
                self.committing = true;
            }
            UndoCommit if !d.has_parent => git.set_error("HEAD has no parent to reset to.".into()),
            UndoCommit => {
                let head = d.head.clone();
                if git.tracking().is_some_and(|(ahead, _)| ahead == 0) {
                    self.ops.dialog = Some(Dialog::Confirm(op, head, String::new()));
                } else {
                    git.job(move || ops::undo(&top, &head, false).map(|_| None));
                }
            }
            Continue | Abort | Skip => {
                let Some(kind) = d.operation else { return git.set_error("No operation is in progress.".into()) };
                if op == Abort {
                    self.ops.dialog = Some(Dialog::Confirm(op, kind.into(), String::new()));
                } else if op == Continue || kind == "rebase" {
                    git.act(&[&["-c", "core.editor=true", kind, if op == Skip { "--skip" } else { "--continue" }]]);
                } else {
                    git.set_error("Skip is available during a rebase.".into());
                }
            }
            Tags | Remotes | Worktrees => self.ops.dialog = Some(Dialog::List(op)),
            Stash => git.stash(false),
            FetchAll => git.act(&[&["fetch", "--all"]]),
            Fetch | FetchPrune | PushTags | Pull | Push => self.choose_remote(op, String::new(), git),
            AddRemote | AddWorktree | CreateTag => {
                self.ops.dialog = Some(Dialog::Form(op, target.unwrap_or_else(|| d.head.clone()), String::new(), String::new()));
            }
            _ => {
                let Some(target) = target else {
                    self.ops.dialog = Some(Dialog::Pick(op, choices(op, git, d), String::new()));
                    return;
                };
                match op {
                    Merge => git.act(&[&["merge", "--no-edit", "--", &target]]),
                    Rebase => git.act(&[&["rebase", "--", &target]]),
                    CherryPick => git.act(&[&["cherry-pick", "--", &target]]),
                    BranchAt => self.ops.dialog = Some(Dialog::Form(op, target, String::new(), String::new())),
                    PushTag => self.choose_remote(op, target, git),
                    RemoteUrl => {
                        if let Some(r) = d.remotes.iter().find(|r| r.name == target) {
                            self.ops.dialog = Some(Dialog::Form(op, target, r.fetch_url.clone(), String::new()));
                        }
                    }
                    DeleteTag | RemoveRemote | RemoveWorktree => self.ops.dialog = Some(Dialog::Confirm(op, target, String::new())),
                    OpenWorktree => cmds.push(Command::OpenPath(PathBuf::from(target))),
                    ApplyStash | PopStash | DropStash | ShowStash => {
                        let Some(stash) = d.stashes.iter().find(|s| s.reference == target) else { return };
                        let hash = stash.hash.clone();
                        match op {
                            DropStash => self.ops.dialog = Some(Dialog::Confirm(op, target, hash)),
                            ShowStash => cmds.push(Command::OpenDiff(format!("{target} ({})", stash.message),
                                ["stash", "show", "--include-untracked", "--patch", "--no-color", &hash].map(str::to_owned).to_vec())),
                            _ => {
                                let verb = if op == ApplyStash { "apply" } else { "pop" };
                                git.job(move || ops::stash_action(&top, verb, &target, &hash).map(|_| None));
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn choose_remote(&mut self, op: Op, target: String, git: &mut Git) {
        let Some(Ok(d)) = git.details() else { return };
        match d.remotes.as_slice() {
            [] => git.set_error("Add a remote first.".into()),
            [remote] => {
                let name = remote.name.clone();
                remote_action(op, &target, &name, git);
            }
            _ => self.ops.dialog = Some(Dialog::Remote(op, target)),
        }
    }

    pub(super) fn stashes(&mut self, ui: &mut Ui, theme: &Theme, git: &Git, cmds: &mut Vec<Command>) {
        let d = git.details().and_then(|d| d.as_ref().ok());
        let title = d.map_or_else(|| String::from("STASHES"), |d| format!("STASHES  {}", d.stashes.len()));
        ui::section(ui, theme, &title, &mut self.ops.stashes_open);
        if !self.ops.stashes_open {
            return;
        }
        let Some(d) = d else { return ui::skeleton(ui, theme, 1) };
        if d.stashes.is_empty() {
            ui.weak("No stashes.");
        }
        ScrollArea::vertical().id_salt("stash_rows").min_scrolled_height(0.0).auto_shrink(false).show(ui, |ui| {
            for s in &d.stashes {
                let row = ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(RichText::new(&s.message).color(theme.text));
                    ui.label(RichText::new(format!("{}  {}", s.branch, s.age)).color(theme.text_dim));
                }).response.interact(Sense::click()).on_hover_text(format!("{}: click to show changes", s.reference));
                if row.clicked() {
                    cmds.push(Command::GitAt(Op::ShowStash, s.reference.clone()));
                }
                row.context_menu(|ui| {
                    ui::menu_pad(ui);
                    for op in [Op::ShowStash, Op::ApplyStash, Op::PopStash, Op::DropStash] {
                        if ui.add_enabled(!git.busy(), egui::Button::new(op.label().trim_start_matches("Git: "))).clicked() {
                            cmds.push(Command::GitAt(op, s.reference.clone()));
                            ui.close();
                        }
                    }
                });
            }
        });
    }
}

impl Scm {
    pub(super) fn operation_dialogs(&mut self, ui: &Ui, theme: &Theme, git: &mut Git, cmds: &mut Vec<Command>) {
        self.ops_root(git);
        git.request_details(ui.ctx());
        if (git.details().is_some() || !git.loading())
            && let Some((op, target)) = self.ops.pending.take() {
            self.start(op, target, git, ui.ctx(), cmds);
        }
        if let Some(path) = git.take_cloned() {
            self.ops.dialog = Some(Dialog::OpenClone(path));
        }
        let Some(mut dialog) = self.ops.dialog.take() else { return };
        let mut next = None;
        let mut done = false;
        match &mut dialog {
            Dialog::Form(op, target, first, second) => {
                let (heading, note, name, other) = match op {
                    Op::AddRemote => ("Add remote", "Enter a remote name and URL.", "Remote name", Some(("URL", 1))),
                    Op::RemoteUrl => ("Change remote URL", target.as_str(), "URL", None),
                    Op::AddWorktree => ("Add worktree", "Use an existing branch or create one from HEAD.", "Branch", Some(("Folder path", 1))),
                    Op::CreateTag => ("Create tag", target.as_str(), "Tag name", Some(("Annotation (optional)", 2))),
                    Op::BranchAt => ("Create branch", target.as_str(), "Branch name", None),
                    Op::Clone => ("Clone repository", "Next, choose the parent folder.", "Repository URL", None),
                    _ => unreachable!(),
                };
                let mut fields = vec![(name, &mut *first, 1)];
                if let Some((label, rows)) = other {
                    fields.push((label, &mut *second, rows));
                }
                if let Some(ok) = prompt(ui, heading, note, &mut fields, if *op == Op::Clone { "Choose Folder..." } else { "OK" }) {
                    done = true;
                    if ok {
                        submit_form(*op, target, first.trim(), second.trim(), git);
                    }
                }
            }
            Dialog::List(op) => {
                let modal = ui::modal(ui.ctx(), "scm_list", ui::Enter::Widget, |ui, _| {
                    ui::outline(ui);
                    ui.set_width(440.0);
                    ui.heading(op.label().trim_start_matches("Git: ").trim_end_matches('.'));
                    let Some(Ok(d)) = git.details() else { return ui::skeleton(ui, theme, 3) };
                    let (add, actions): (_, &[Op]) = match op {
                        Op::Tags => (Op::CreateTag, &[Op::PushTag, Op::DeleteTag]),
                        Op::Remotes => (Op::AddRemote, &[Op::RemoteUrl, Op::RemoveRemote]),
                        Op::Worktrees => (Op::AddWorktree, &[Op::OpenWorktree, Op::RemoveWorktree]),
                        _ => unreachable!(),
                    };
                    let items = choices(*op, git, d);
                    if items.is_empty() {
                        ui.weak("No entries.");
                    }
                    ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for (value, label) in items {
                            ui.label(label);
                            ui.horizontal_wrapped(|ui| {
                                for action in actions {
                                    if *op == Op::Worktrees && !choices(*action, git, d).iter().any(|(v, _)| v == &value) {
                                        continue;
                                    }
                                    if ui.button(action.label().trim_start_matches("Git: ")).clicked() {
                                        next = Some((*action, Some(value.clone())));
                                    }
                                }
                            });
                            ui.separator();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let extras: &[Op] = match op {
                            Op::Tags => &[Op::PushTags],
                            Op::Remotes => &[Op::Fetch, Op::FetchPrune, Op::FetchAll],
                            _ => &[],
                        };
                        for action in std::iter::once(&add).chain(extras) {
                            if ui.button(action.label().trim_start_matches("Git: ")).clicked() {
                                next = Some((*action, None));
                            }
                        }
                        done = ui.button("Close").clicked();
                    });
                });
                done |= modal.should_close();
            }
            Dialog::Pick(op, choices, revision) => {
                let modal = ui::modal(ui.ctx(), "scm_picker", ui::Enter::Primary, |ui, keys| {
                    ui::outline(ui);
                    ui.set_width(400.0);
                    ui.heading(op.label());
                    let reference = matches!(op, Op::CherryPick | Op::BranchAt);
                    if reference {
                        ui.add(egui::TextEdit::singleline(revision).hint_text("Commit hash or reference"));
                        let valid = !revision.trim().is_empty();
                        if keys.primary(ui, "Use Reference", valid) {
                            next = Some((*op, Some(revision.trim().to_owned())));
                        }
                    }
                    if choices.is_empty() {
                        ui.weak("No entries available.");
                    }
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (i, (value, label)) in choices.iter().enumerate() {
                            if keys.choice(ui, label.as_str(), i == 0 && !reference) {
                                next = Some((*op, Some(value.clone())));
                            }
                        }
                    });
                    done = ui.button("Cancel").clicked();
                });
                done |= modal.should_close();
            }
            Dialog::Remote(op, target) => {
                let mut pick = None;
                let modal = ui::modal(ui.ctx(), "scm_remote", ui::Enter::Primary, |ui, keys| {
                    ui::outline(ui);
                    ui.heading("Select a remote");
                    if let Some(Ok(d)) = git.details() {
                        for (i, r) in d.remotes.iter().enumerate() {
                            if keys.choice(ui, &r.name, i == 0) { pick = Some(r.name.clone()); }
                        }
                    }
                    done = ui.button("Cancel").clicked();
                });
                if let Some(remote) = pick {
                    remote_action(*op, target, &remote, git);
                    done = true;
                }
                done |= modal.should_close();
            }
            Dialog::Confirm(op, target, hash) => {
                let detail = match op {
                    Op::UndoCommit => format!("{} is on the upstream branch. Undo it locally and keep its changes staged?", &target[..8.min(target.len())]),
                    Op::Abort => format!("Abort this {target} and discard its in-progress resolution?"),
                    Op::DeleteTag => format!("Delete the local tag {target}?"),
                    Op::RemoveRemote => format!("Remove remote {target} and its remote-tracking branches?"),
                    Op::RemoveWorktree => format!("Remove the worktree folder {target}?"),
                    Op::DropStash => format!("Drop {target} ({})?", &hash[..8.min(hash.len())]),
                    _ => unreachable!(),
                };
                if let Some(ok) = confirm(ui, op.label(), &detail, "Confirm") {
                    done = true;
                    if ok && let Some(top) = git.top().map(Path::to_path_buf) {
                        let target = target.clone();
                        match op {
                            Op::UndoCommit => git.job(move || ops::undo(&top, &target, true).map(|_| None)),
                            Op::Abort => git.job(move || {
                                if ops::operation(&top)? != Some(target.as_str()) {
                                    return Err("The in-progress operation changed.".into());
                                }
                                exec(&top, &[&target, "--abort"]).map(|_| None)
                            }),
                            Op::DeleteTag => git.act(&[&["tag", "-d", "--", &target]]),
                            Op::RemoveRemote => git.act(&[&["remote", "remove", "--", &target]]),
                            Op::RemoveWorktree => git.act(&[&["worktree", "remove", "--", &crate::wsl::shell_path(Path::new(&target))]]),
                            Op::DropStash => {
                                let hash = hash.clone();
                                git.job(move || ops::stash_action(&top, "drop", &target, &hash).map(|_| None));
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            Dialog::OpenClone(path) => {
                if let Some(ok) = confirm(ui, "Repository cloned", &format!("Open {} in Halo?", path.display()), "Open Folder") {
                    done = true;
                    if ok {
                        cmds.push(Command::OpenPath(path.clone()));
                    }
                }
            }
        }
        if let Some((op, target)) = next {
            self.start(op, target, git, ui.ctx(), cmds);
        } else if !done {
            self.ops.dialog = Some(dialog);
        }
    }
}

fn confirm(ui: &Ui, heading: &str, message: &str, accept: &str) -> Option<bool> {
    let mut choice = None;
    let modal = ui::modal(ui.ctx(), "scm_confirm_op", ui::Enter::Primary, |ui, keys| {
        ui::outline(ui);
        ui.set_width(360.0);
        ui.heading(heading);
        ui.label(message);
        ui.horizontal(|ui| {
            if keys.primary(ui, accept, true) { choice = Some(true); }
            if ui.button("Cancel").clicked() { choice = Some(false); }
        });
    });
    choice.or_else(|| modal.should_close().then_some(false))
}

fn submit_form(op: Op, target: &str, first: &str, second: &str, git: &mut Git) {
    use Op::*;
    match op {
        Clone => {
            let name = match ops::clone_name(first) {
                Ok(name) => name.to_owned(),
                Err(error) => return git.set_error(error),
            };
            let Some(parent) = rfd::FileDialog::new().set_title("Clone into parent folder").pick_folder() else { return };
            let url = first.to_owned();
            git.job(move || crate::git::tracked(Some(format!("Cloning {url}")), |_| {
                exec(&parent, &["clone", "--", &url, &name])?;
                Ok(Some(parent.join(name)))
            }));
        }
        AddRemote if second.is_empty() => git.set_error("Enter a remote URL.".into()),
        AddRemote => git.act(&[&["remote", "add", "--", first, second]]),
        RemoteUrl => git.act(&[&["remote", "set-url", "--", target, first]]),
        BranchAt => git.act(&[&["branch", "--", first, target]]),
        CreateTag => {
            let Some(top) = git.top().map(Path::to_path_buf) else { return };
            let (name, message, target) = (first.to_owned(), second.to_owned(), target.to_owned());
            git.job(move || ops::create_tag(&top, &name, &message, &target).map(|_| None));
        }
        AddWorktree if second.is_empty() => git.set_error("Enter a worktree folder path.".into()),
        AddWorktree => {
            let path = crate::wsl::shell_path(Path::new(second));
            if git.branches().iter().any(|b| b == first) {
                git.act(&[&["worktree", "add", "--", &path, first]]);
            } else {
                git.act(&[&["worktree", "add", "-b", first, "--", &path, "HEAD"]]);
            }
        }
        _ => unreachable!(),
    }
}

fn remote_action(op: Op, target: &str, remote: &str, git: &mut Git) {
    use Op::*;
    let branch = git.branch().unwrap_or_default().to_owned();
    match op {
        Fetch => git.act(&[&["fetch", "--", remote]]),
        FetchPrune => git.act(&[&["fetch", "--prune", "--", remote]]),
        PushTags => git.act(&[&["push", "--tags", "--", remote]]),
        PushTag => git.act(&[&["push", "--", remote, &format!("refs/tags/{target}:refs/tags/{target}")]]),
        Push | Pull => {
            let Some(top) = git.top().map(Path::to_path_buf) else { return };
            let remote = remote.to_owned();
            let title = Some(format!("Git {} {remote}", if op == Pull { "pull from" } else { "push to" }));
            git.job(move || crate::git::tracked(title, |_| {
                let current = exec(&top, &["symbolic-ref", "--short", "HEAD"])?;
                if current.trim_end() != branch {
                    return Err("The current branch changed. Select the remote again.".into());
                }
                let info = exec(&top, &["for-each-ref", "--format=%(upstream:remotename)%00%(upstream:remoteref)",
                    &format!("refs/heads/{branch}")])?;
                let upstream = info.trim_end().split_once('\0').filter(|(name, path)| *name == remote && !path.is_empty());
                let dest = format!("refs/heads/{branch}");
                let dest = upstream.map_or(dest.as_str(), |(_, path)| path);
                match op {
                    Pull => exec(&top, &["pull", "--no-edit", "--", &remote, dest])?,
                    _ if upstream.is_some() => exec(&top, &["push", "--", &remote, &format!("HEAD:{dest}")])?,
                    _ => exec(&top, &["push", "-u", "--", &remote, &format!("HEAD:{dest}")])?,
                };
                Ok(None)
            }));
        }
        _ => unreachable!(),
    }
}
