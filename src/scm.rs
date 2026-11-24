#[path = "scm_ops.rs"]
mod ops;
#[path = "scm_rebase.rs"]
mod rebase;
#[path = "scm_compare.rs"]
mod compare;

use crate::command::{Command, GitCommand};
use crate::explorer::{file_icon, state_badge, state_color};
use crate::git::{Change, Commit, FileState, Git};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, pos2, vec2,
};
use std::path::{Path, PathBuf};

const ROW: f32 = 22.0;
const STAGED: [(&str, &str); 2] = [(icons::MINUS, "Unstage"), (icons::FILE, "Open File")];
const STAGED_ALL: [(&str, &str); 1] = [(icons::MINUS, "Unstage All")];
const UNSTAGED: [(&str, &str); 3] =
    [(icons::PLUS, "Stage"), (icons::ARROW_COUNTER_CLOCKWISE, "Discard Changes"), (icons::FILE, "Open File")];
const CONFLICTED: [(&str, &str); 4] = [
    (icons::PLUS, "Stage"),
    (icons::ARROW_COUNTER_CLOCKWISE, "Discard Changes"),
    (icons::GIT_MERGE, "Resolve in Merge Editor"),
    (icons::FILE, "Open File"),
];
const UNSTAGED_ALL: [(&str, &str); 2] =
    [(icons::PLUS, "Stage All"), (icons::ARROW_COUNTER_CLOCKWISE, "Discard All Changes")];

enum Action {
    Stage(Option<String>),
    Unstage(Option<String>),
    Discard(Option<String>),
    Commit,
    Switch(String),
    NewBranch,
    Refresh,
    SelectRepo(PathBuf),
    ShowCommit(String),
    Pr(Vec<String>),
    ReviewPr(u64, bool),
    NewPr,
    LoadMore,
}

pub struct Scm {
    open: [bool; 3],
    sizes: [Option<f32>; 4],
    message: String,
    committing: bool,
    discard: Option<Option<String>>,
    new_branch: Option<String>,
    commit: Option<String>,
    review: Option<(u64, bool, String)>,
    new_pr: Option<(String, String)>,
    ops: ops::Ops,
    rebase: Option<rebase::Editor>,
    compare: compare::Compare,
    ai: Option<crate::commit_ai::Job>,
}

impl Scm {
    pub fn new() -> Self {
        Self {
            open: [true; 3], sizes: [None; 4], message: String::new(), committing: false, discard: None,
            new_branch: None, commit: None, review: None, new_pr: None, ops: ops::Ops::default(), ai: None,
            rebase: None, compare: compare::Compare::default(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, git: &mut Git, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui::section_header(ui, theme, "Source Control");
        self.operation_dialogs(ui, theme, git, cmds);
        self.rebase_dialog(ui, theme, git);
        self.compare_picker(ui, theme, git);
        let mut action = None;
        repositories(ui, theme, git, &mut action);
        let Some(top) = git.top().map(Path::to_path_buf) else {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                if git.loading() || git.busy() {
                    ui::skeleton(ui, theme, 2);
                } else {
                    ui.label(RichText::new("Open or initialize a Git repository.").color(theme.text_dim));
                    if let Some(parent) = git.parent_repo().map(Path::to_path_buf) {
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("A Git repository was found in the parent folder {}.", parent.display())).color(theme.text_dim));
                        if ui.button("Open Repository").clicked() {
                            git.open_parent_repo();
                        }
                    }
                    if git.can_init() && ui.button("Initialize Repository").clicked() {
                        cmds.push(Command::Git(GitCommand::Init));
                    }
                    if ui.button("Clone Repository...").clicked() {
                        cmds.push(Command::Git(GitCommand::Clone));
                    }
                }
                if let Some(error) = git.error() {
                    ui.colored_label(theme.error, error);
                }
            });
            self.apply(action, git, ui);
            return;
        };
        if let Some(result) = self.ai.as_ref().and_then(|job| job.poll()) {
            self.ai = None;
            match result {
                Ok(message) => self.message = message,
                Err(error) => ui::notify_error(format!("Could not generate a commit message: {error}")),
            }
        }
        if self.committing && !git.busy() {
            self.committing = false;
            if git.error().is_none() {
                self.message.clear();
            }
        }

        let busy = git.busy();
        git.load_prs();
        toolbar(ui, theme, git, &mut action, cmds);
        ops::in_progress(ui, theme, git, cmds);
        Frame::new().inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            let id = Id::new("scm_message");
            let can_commit = !busy && !(git.changes(true).is_empty() && git.changes(false).is_empty()) && !self.message.trim().is_empty();
            let submit = ui.memory(|m| m.has_focus(id))
                && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
            let hint = format!("Message (Ctrl+Enter to commit on \"{}\")", git.branch().unwrap_or_default());
            ui.add(
                egui::TextEdit::multiline(&mut self.message)
                    .id(id)
                    .hint_text(hint)
                    .desired_rows(2)
                    .margin(ui::FIELD)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            let label = RichText::new(format!("{}  Commit", icons::CHECK)).color(theme.accent);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 91.0).max(50.0);
                let button = ui.add_enabled(can_commit, egui::Button::new(label).min_size(vec2(width, 26.0)))
                    .on_disabled_hover_text("Type a message to commit.");
                if can_commit && (button.clicked() || submit) {
                    action = Some(Action::Commit);
                }
                let tip = if self.ai.is_some() { "Generating a commit message. Click to cancel." } else { "Generate Commit Message with AI" };
                let sparkle = ui::icon_button(ui, theme, if self.ai.is_some() { "" } else { icons::SPARKLE }, 26.0, false, tip);
                if self.ai.is_some() {
                    ui::spinner(ui, sparkle.rect.center(), 6.0, theme.accent);
                }
                if sparkle.clicked() {
                    cmds.push(Command::Git(GitCommand::GenerateMessage));
                }
                let amend = git.details().and_then(|d| d.as_ref().ok()).is_some_and(|d| !d.head.is_empty() && d.operation.is_none());
                if ui.add_enabled(amend && !busy, egui::Button::new("Amend"))
                    .on_hover_text("Amend HEAD. Leave the message empty to keep the last message.").clicked() {
                    cmds.push(Command::Git(GitCommand::Amend));
                }
            });
            if let Some(error) = git.error() {
                ui.add_space(6.0);
                ui.label(RichText::new(error).color(theme.error));
            }
            ui.add_space(4.0);
        });

        let stashes = git.details().and_then(|d| d.as_ref().ok()).map_or(1, |d| d.stashes.len().max(1));
        let stash_height = (stashes as f32 * ui.spacing().interact_size.y).min(132.0);
        let staged = git.changes(true);
        let unstaged = git.changes(false);
        let pr_title = match git.prs() {
            Some(Ok(prs)) => format!("PULL REQUESTS  {}", prs.len()),
            _ => "PULL REQUESTS".to_owned(),
        };
        let titles = [format!("CHANGES  {}", staged.len() + unstaged.len()), pr_title, "COMMITS".to_owned()];
        let panes = [(self.ops.stashes_open, Some(stash_height)), (self.open[0], None), (self.open[1], None), (self.open[2], None)];
        let mut sizes = self.sizes;
        ui::stack(ui, theme, Id::new("scm_stack"), &panes, &mut sizes, |ui, pane| {
            let Some(i) = pane.checked_sub(1) else { return self.stashes(ui, theme, git, cmds) };
            let title = &titles[i];
            ui.push_id(("scm_section", i), |ui| {
                let header = ui::section(ui, theme, title, &mut self.open[i]);
                let rect = header.rect;
                if i == 0 {
                    let mut end = rect;
                    if !unstaged.is_empty() {
                        match buttons(ui, theme, rect, !busy, &UNSTAGED_ALL) {
                            Some(0) => action = Some(Action::Stage(None)),
                            Some(_) => action = Some(Action::Discard(None)),
                            None => {}
                        }
                        end.max.x -= 40.0;
                    }
                    if !staged.is_empty() && buttons(ui, theme, end, !busy, &STAGED_ALL).is_some() {
                        action = Some(Action::Unstage(None));
                    }
                } else if i == 1 && buttons(ui, theme, rect, !busy, &[(icons::PLUS, "Create Pull Request")]).is_some() {
                    action = Some(Action::NewPr);
                }
                if ui.available_height() < 1.0 {
                    return;
                }
                ScrollArea::vertical().id_salt("body").min_scrolled_height(0.0).auto_shrink(false).show(ui, |ui| {
                    match i {
                        0 => changes(ui, theme, git, &top, &mut action, cmds),
                        1 => pull_requests(ui, theme, git, busy, &mut action, cmds),
                        _ => {
                            self.compare_view(ui, theme, git, cmds);
                            graph(ui, theme, git, self.commit.as_deref(), &mut action, cmds);
                        }
                    }
                });
            });
        });
        self.sizes = sizes;
        self.dialogs(ui, git, &top, cmds);
        self.apply(action, git, ui);
    }

    fn apply(&mut self, action: Option<Action>, git: &mut Git, ui: &Ui) {
        match action {
            Some(Action::Stage(path)) => git.stage(path.as_deref()),
            Some(Action::Unstage(path)) => git.unstage(path.as_deref()),
            Some(Action::Discard(path)) => self.discard = Some(path),
            Some(Action::Commit) => {
                git.commit(self.message.trim());
                self.committing = true;
            }
            Some(Action::Switch(branch)) => git.switch(&branch, false),
            Some(Action::NewBranch) => self.new_branch = Some(String::new()),
            Some(Action::Refresh) => {
                git.refresh_prs();
                git.refresh(ui.ctx());
            }
            Some(Action::Pr(args)) => git.pr(&args.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(Action::ReviewPr(number, changes)) => self.review = Some((number, changes, String::new())),
            Some(Action::NewPr) => {
                let title = git.log().first().map(|c| c.subject.clone()).unwrap_or_default();
                self.new_pr = Some((title, String::new()));
            }
            Some(Action::SelectRepo(repo)) => {
                self.commit = None;
                git.select(Some(repo), ui.ctx());
            }
            Some(Action::ShowCommit(hash)) if self.commit.as_deref() == Some(&hash) => self.commit = None,
            Some(Action::ShowCommit(hash)) => {
                git.show(&hash);
                self.commit = Some(hash);
            }
            Some(Action::LoadMore) => git.load_more_log(),
            None => {}
        }
    }

    fn dialogs(&mut self, ui: &mut Ui, git: &mut Git, top: &Path, cmds: &mut Vec<Command>) {
        if let Some(target) = self.discard.clone() {
            let mut choice = None;
            let modal = ui::modal(ui.ctx(), "scm_discard", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(360.0);
                ui.heading("Discard changes");
                ui.label(match &target {
                    Some(path) => format!("Discard the changes in {path}?"),
                    None => format!("Discard all {} changes?", git.changes(false).len()),
                });
                ui.label("Tracked files are restored and this cannot be undone. Untracked files move to the recycle bin.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if keys.primary(ui, "Discard", true) {
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
                self.discard = None;
                if confirmed {
                    discard(git, top, target.as_deref(), cmds);
                }
            }
        }

        if let Some(mut name) = self.new_branch.take() {
            let note = format!("Starts from {}.", git.branch().unwrap_or("the current commit"));
            match prompt(ui, "Create branch", &note, &mut [("Branch name", &mut name, 1)], "Create") {
                Some(true) => git.switch(name.trim(), true),
                Some(false) => {}
                None => self.new_branch = Some(name),
            }
        }
        if let Some((number, changes, mut body)) = self.review.take() {
            let (heading, flag) = if changes { ("Request changes", "--request-changes") } else { ("Comment", "--comment") };
            let note = format!("Posts a review on pull request #{number}.");
            match prompt(ui, heading, &note, &mut [("Review comment", &mut body, 4)], "Submit") {
                Some(true) => git.pr(&["review", &number.to_string(), flag, "--body", body.trim()]),
                Some(false) => {}
                None => self.review = Some((number, changes, body)),
            }
        }
        if let Some((mut title, mut body)) = self.new_pr.take() {
            let note = format!("Pushes {} to origin, then opens a pull request on GitHub.", git.branch().unwrap_or("HEAD"));
            let fields = &mut [("Title", &mut title, 1), ("Description", &mut body, 5)];
            match prompt(ui, "Create pull request", &note, fields, "Create") {
                Some(true) => git.create_pr(title.trim(), body.trim()),
                Some(false) => {}
                None => self.new_pr = Some((title, body)),
            }
        }
    }
}

pub(crate) fn prompt(ui: &Ui, heading: &str, note: &str, fields: &mut [(&str, &mut String, usize)], ok: &str) -> Option<bool> {
    let mut choice = None;
    let field_id = |i: usize| Id::new(("scm_field", heading, i));
    let multiline = fields.iter().enumerate().any(|(i, (_, _, rows))| *rows > 1 && ui.memory(|m| m.has_focus(field_id(i))));
    let mode = if multiline { ui::Enter::Multiline } else { ui::Enter::Primary };
    let modal = ui::modal(ui.ctx(), ("scm_prompt", heading), mode, |ui, keys| {
        ui::outline(ui);
        ui.set_width(360.0);
        ui.heading(heading);
        ui.label(note);
        for (i, (hint, text, rows)) in fields.iter_mut().enumerate() {
            ui.add_space(6.0);
            let edit = if *rows == 1 {
                egui::TextEdit::singleline(&mut **text)
            } else {
                egui::TextEdit::multiline(&mut **text).desired_rows(*rows)
            };
            let resp = ui.add(edit.id(field_id(i)).hint_text(*hint).margin(ui::FIELD).desired_width(f32::INFINITY));
            if i == 0 && ui.memory(|m| m.focused().is_none()) {
                resp.request_focus();
            }
        }
        let valid = !fields[0].1.trim().is_empty();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if keys.primary(ui, ok, valid) {
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
    choice
}

fn pull_requests(ui: &mut Ui, theme: &Theme, git: &Git, busy: bool, action: &mut Option<Action>, cmds: &mut Vec<Command>) {
    let note = |ui: &mut Ui, text: &str| {
        Frame::new().inner_margin(Margin::symmetric(14, 5)).show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(theme.text_dim));
        });
    };
    let prs = match git.prs() {
        None => return ui::skeleton(ui, theme, 3),
        Some(Err(error)) if error.starts_with("Could not run gh") => {
            return note(ui, "Install the GitHub CLI (gh) to see pull requests.");
        }
        Some(Err(error)) => return note(ui, error.lines().next().unwrap_or_default()),
        Some(Ok(prs)) if prs.is_empty() => return note(ui, "No open pull requests."),
        Some(Ok(prs)) => prs,
    };
    for pr in prs {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        ui::row_bg(ui, theme, rect, resp.id, resp.hovered(), false);
        ui::item(ui, &resp, ui::Role::ListItem, &format!("#{} {}", pr.number, pr.title), false);
        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 28.0));
        let (x, y) = (rect.left() + 14.0, rect.center().y);
        let font = FontId::proportional(13.0);
        let number = painter.text(pos2(x, y), Align2::LEFT_CENTER, format!("#{}", pr.number), font.clone(), theme.accent);
        let color = if pr.is_draft { theme.text_dim } else { theme.text };
        let title = painter.text(pos2(number.right() + 6.0, y), Align2::LEFT_CENTER, &pr.title, font.clone(), color);
        painter.text(pos2(title.right() + 8.0, y), Align2::LEFT_CENTER, &pr.head_ref_name, FontId::proportional(11.5), theme.text_dim);
        let (badge, color, merge) = match pr.mergeable.as_str() {
            "CONFLICTING" => (icons::WARNING, theme.deleted, "conflicts with the base branch"),
            "MERGEABLE" => (icons::CHECK, theme.added, "can be merged"),
            _ => ("", theme.text_dim, "merge state not known yet"),
        };
        ui.painter().text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, font, color);
        let review = match pr.review_decision.as_str() {
            "APPROVED" => "approved",
            "CHANGES_REQUESTED" => "changes requested",
            "REVIEW_REQUIRED" => "review required",
            _ => "no review yet",
        };
        let draft = if pr.is_draft { ", draft" } else { "" };
        let resp = resp.on_hover_text(format!("{}: {merge}, {review}{draft}", pr.head_ref_name));
        let number = pr.number.to_string();
        egui::Popup::menu(&resp).show(|ui| {
            ui::menu_pad(ui);
            let run = |args: &[&str]| Some(Action::Pr(args.iter().map(|a| a.to_string()).collect()));
            if ui.add_enabled(!busy, egui::Button::new("Check Out")).clicked() {
                *action = run(&["checkout", &number]);
            }
            if ui.add_enabled(!busy, egui::Button::new("Approve")).clicked() {
                *action = run(&["review", &number, "--approve"]);
            }
            if ui.button("Request Changes...").clicked() {
                *action = Some(Action::ReviewPr(pr.number, true));
            }
            if ui.button("Comment...").clicked() {
                *action = Some(Action::ReviewPr(pr.number, false));
            }
            if ui.button("Review in Diff").clicked() {
                cmds.push(Command::ReviewPullRequest(pr.number));
            }
            if ui.add_enabled(!busy, egui::Button::new("Open on GitHub")).clicked() {
                *action = run(&["view", &number, "--web"]);
            }
        });
    }
}

fn discard(git: &mut Git, top: &Path, target: Option<&str>, cmds: &mut Vec<Command>) {
    let targets: Vec<Change> =
        git.changes(false).iter().filter(|c| target.is_none_or(|t| t == c.path)).cloned().collect();
    let mut tracked = false;
    for change in targets {
        if change.state != FileState::Added {
            tracked = true;
            continue;
        }
        let full = top.join(&change.path);
        match trash::delete(&full) {
            Ok(()) => cmds.push(Command::PathChanged(full, None)),
            Err(error) => {
                git.set_error(format!("Could not move {} to the recycle bin: {error}", change.path));
                return;
            }
        }
    }
    if tracked {
        git.discard(target);
    }
}

fn toolbar(ui: &mut Ui, theme: &Theme, git: &Git, action: &mut Option<Action>, cmds: &mut Vec<Command>) {
    let busy = git.busy();
    Frame::new().inner_margin(Margin::symmetric(10, 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let branch = git.branch().unwrap_or_default();
            let label = RichText::new(format!("{} {branch} {}", icons::GIT_BRANCH, icons::CARET_DOWN)).color(theme.text);
            ui.add_enabled_ui(!busy, |ui| {
                ui.set_max_width((ui.available_width() - 126.0).max(48.0));
                egui::containers::menu::MenuButton::from_button(egui::Button::new(label).truncate()).ui(ui, |ui| {
                    ui::menu_pad(ui);
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for name in git.branches() {
                            if ui.selectable_label(name == branch, name).clicked() && name != branch {
                                *action = Some(Action::Switch(name.clone()));
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Create Branch...").clicked() {
                        *action = Some(Action::NewBranch);
                    }
                    if ui.button("Compare Branch with...").clicked() {
                        cmds.push(Command::GitAt(GitCommand::Compare, branch.to_owned()));
                    }
                })
                .0
                .on_hover_text("Switch branch");
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if busy {
                    ui.spinner();
                    return;
                }
                ui.menu_button(RichText::new(icons::DOTS_THREE).color(theme.text_dim), |ui| {
                    ui::menu_pad(ui);
                    ops::menu(ui, git, cmds);
                });
                let tracking = git.tracking();
                let (ahead, behind) = tracking.unwrap_or_default();
                let push = match tracking {
                    Some(_) => format!("Push ({ahead} ahead)"),
                    None => "Publish Branch".to_owned(),
                };
                if ui::icon_button(ui, theme, icons::ARROW_UP, 22.0, ahead > 0, &push).clicked() {
                    cmds.push(Command::Git(GitCommand::Push));
                }
                if ui::icon_button(ui, theme, icons::ARROW_DOWN, 22.0, behind > 0, &format!("Pull ({behind} behind)")).clicked() {
                    cmds.push(Command::Git(GitCommand::Pull));
                }
                if ui::icon_button(ui, theme, icons::CLOUD_ARROW_DOWN, 22.0, false, "Fetch").clicked() {
                    cmds.push(Command::Git(GitCommand::Fetch));
                }
                if git.loading() {
                    ui.add_sized(vec2(22.0, 22.0), egui::Spinner::new().size(13.0).color(theme.text_dim)).on_hover_text("Refreshing Git status");
                } else if ui::icon_button(ui, theme, icons::ARROWS_CLOCKWISE, 22.0, false, "Refresh").clicked() {
                    *action = Some(Action::Refresh);
                }
            });
        });
    });
}

fn repositories(ui: &mut Ui, theme: &Theme, git: &Git, action: &mut Option<Action>) {
    if git.repos().len() < 2 {
        return;
    }
    for repo in git.repos() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        let selected = git.root() == Some(repo.as_path());
        ui::row_bg(ui, theme, rect, resp.id, resp.hovered(), selected);
        ui::item(ui, &resp, ui::Role::ListItem, &repo.to_string_lossy(), selected);
        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 8.0));
        let (x, y) = (rect.left() + 14.0, rect.center().y);
        let name = repo.file_name().unwrap_or(repo.as_os_str()).to_string_lossy();
        let color = if selected { theme.text } else { theme.text_dim };
        painter.text(pos2(x, y), Align2::LEFT_CENTER, icons::FOLDER, FontId::proportional(15.0), ui::with_alpha(theme.accent, 0.85));
        painter.text(pos2(x + 21.0, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
        if resp.clicked() && !selected {
            *action = Some(Action::SelectRepo(repo.clone()));
        }
    }
    ui.add_space(4.0);
}

fn graph(
    ui: &mut Ui,
    theme: &Theme,
    git: &Git,
    open: Option<&str>,
    action: &mut Option<Action>,
    cmds: &mut Vec<Command>,
) {
    let log = git.log();
    if log.is_empty() {
        return;
    }
    let palette = [theme.accent, theme.modified, theme.added, theme.syntax.keyword, theme.syntax.string, theme.deleted];
    let widest = |c: &Commit| c.through.iter().chain(&c.down).copied().fold(c.col, usize::max);
    let lanes = log.iter().map(widest).max().unwrap_or(0).min(7) + 1;
    let text_x = 20.0 + lanes as f32 * 10.0;
    let lane_x = |rect: Rect, lane: usize| rect.left() + 14.0 + lane.min(7) as f32 * 10.0;
    let stroke = |lane: usize| Stroke::new(1.5, palette[lane % palette.len()]);
    let (ahead, behind) = git.tracking().unwrap_or_default();
    let groups = [(behind, icons::ARROW_DOWN, "Incoming Changes", "on the upstream branch, not here yet. Click to pull.", GitCommand::Pull),
        (ahead, icons::ARROW_UP, "Outgoing Changes", "here, not pushed yet. Click to push.", GitCommand::Push)];
    for (count, icon, label, tip, op) in groups.into_iter().filter(|g| g.0 > 0) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        ui::row_bg(ui, theme, rect, resp.id, resp.hovered(), false);
        ui::item(ui, &resp, ui::Role::Button, &format!("{label}, {count}"), false);
        let (x, y) = (rect.left() + 14.0, rect.center().y);
        let color = if op == GitCommand::Pull { theme.modified } else { theme.added };
        ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), color);
        ui.painter().text(pos2(rect.left() + text_x, y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), theme.text);
        ui.painter().text(pos2(rect.right() - 14.0, y), Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(11.5), color);
        let plural = if count == 1 { "commit" } else { "commits" };
        if resp.on_hover_text(format!("{count} {plural} {tip}")).clicked() && !git.busy() {
            cmds.push(Command::Git(op));
        }
    }

    for commit in log {
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), ROW));
        let id = Id::new(("scm_commit", &commit.hash));
        let resp = ui.interact(rect, id, Sense::click());
        let is_open = open == Some(commit.hash.as_str());
        let side = git.unsynced(&commit.hash).map(|incoming| match incoming {
            true => (icons::ARROW_DOWN, theme.modified, "incoming"),
            false => (icons::ARROW_UP, theme.added, "outgoing"),
        });
        if ui.is_rect_visible(rect) {
            ui::row_bg(ui, theme, rect, id, resp.hovered(), is_open);
            let marked = side.map_or_else(String::new, |s| format!(", {}", s.2));
            ui::tree_item(ui, &resp, &format!("{}, {}{marked}", commit.subject, commit.author), is_open, 0, Some(is_open));
            if let Some((icon, color, _)) = side {
                ui.painter().text(pos2(rect.right() - 14.0, rect.center().y), Align2::CENTER_CENTER, icon, FontId::proportional(12.0), color);
            }
            let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - if side.is_some() { 24.0 } else { 8.0 }));
            let dot = pos2(lane_x(rect, commit.col), rect.center().y);
            for lane in &commit.through {
                painter.vline(lane_x(rect, *lane), rect.y_range(), stroke(*lane));
            }
            for lane in &commit.up {
                painter.line_segment([pos2(lane_x(rect, *lane), rect.top()), dot], stroke(commit.col.max(*lane)));
            }
            for lane in &commit.down {
                painter.line_segment([dot, pos2(lane_x(rect, *lane), rect.bottom())], stroke(commit.col.max(*lane)));
            }
            painter.circle_filled(dot, 3.5, palette[commit.col % palette.len()]);
            let mut x = rect.left() + text_x;
            if !commit.refs.is_empty() {
                let refs = commit.refs.replace("HEAD -> ", "");
                x = painter.text(pos2(x, dot.y), Align2::LEFT_CENTER, refs, FontId::proportional(11.5), theme.accent).right() + 6.0;
            }
            painter.text(pos2(x, dot.y), Align2::LEFT_CENTER, &commit.subject, FontId::proportional(13.0), theme.text);
        }
        resp.context_menu(|ui| {
            ui::menu_pad(ui);
            for (op, label) in [(GitCommand::CherryPick, "Cherry Pick..."), (GitCommand::CreateTag, "Create Tag Here..."),
                (GitCommand::BranchAt, "Create Branch Here..."), (GitCommand::InteractiveRebase, "Interactive Rebase from Here..."),
                (GitCommand::Compare, "Compare with...")] {
                if ui.add_enabled(!git.busy(), egui::Button::new(label)).clicked() {
                    cmds.push(Command::GitAt(op, commit.hash.clone()));
                    ui.close();
                }
            }
        });
        let short = &commit.hash[..commit.hash.len().min(8)];
        let marked = side.map_or_else(String::new, |s| format!("  ({})", s.2));
        if resp.on_hover_text(format!("{short}  {}, {}{marked}", commit.author, commit.date)).clicked() {
            *action = Some(Action::ShowCommit(commit.hash.clone()));
        }
        if !is_open {
            continue;
        }

        let Some(files) = git.files(&commit.hash) else {
            ui::skeleton(ui, theme, 1);
            continue;
        };
        for change in files {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            ui::row_bg(ui, theme, rect, resp.id, resp.hovered(), false);
            ui::tree_item(ui, &resp, &change.path, false, 1, None);
            let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 8.0));
            for lane in commit.through.iter().chain(&commit.down) {
                painter.vline(lane_x(rect, *lane), rect.y_range(), stroke(*lane));
            }
            let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
            let color = state_color(theme, Some(change.state));
            let (x, y) = (rect.left() + text_x, rect.center().y);
            let drawn = painter.text(pos2(x, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
            painter.text(pos2(drawn.right() + 8.0, y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
            let badge = if change.state == FileState::Added { "A" } else { state_badge(Some(change.state)) };
            painter.text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);
            if resp.clicked() {
                let args = ["show", "--format=", "--diff-merges=first-parent", &commit.hash, "--", &change.path];
                cmds.push(Command::OpenDiff(format!("{name} ({short})"), args.map(str::to_owned).to_vec()));
            }
        }
    }
    if ui::list_end(ui, theme, git.log_loading()) && action.is_none() {
        *action = Some(Action::LoadMore);
    }
}

pub(crate) fn buttons(ui: &mut Ui, theme: &Theme, rect: Rect, enabled: bool, items: &[(&str, &str)]) -> Option<usize> {
    let builder = UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::right_to_left(Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut hit = None;
        for (i, (icon, tip)) in items.iter().enumerate() {
            if ui::icon_button(ui, theme, icon, 20.0, false, tip).clicked() && enabled {
                hit = Some(i);
            }
        }
        hit
    })
    .inner
}

fn changes(ui: &mut Ui, theme: &Theme, git: &Git, top: &Path, action: &mut Option<Action>, cmds: &mut Vec<Command>) {
    if git.changes(true).is_empty() && git.changes(false).is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("No changes.").color(theme.text_dim));
        });
    }
    for staged in [true, false] {
        let changes = git.changes(staged);
        if changes.is_empty() {
            continue;
        }
        if !git.changes(true).is_empty() {
            let name = if staged { "STAGED CHANGES" } else { "CHANGES" };
            ui::section_header(ui, theme, &format!("{name}  {}", changes.len()));
        }
        group(ui, theme, top, changes, staged, git.busy(), action, cmds);
    }
}

#[allow(clippy::too_many_arguments)]
fn group(
    ui: &mut Ui,
    theme: &Theme,
    top: &Path,
    changes: &[Change],
    staged: bool,
    busy: bool,
    action: &mut Option<Action>,
    cmds: &mut Vec<Command>,
) {
    let row_items: &[_] = if staged { &STAGED } else { &UNSTAGED };
    let act = |index: usize, path: Option<String>| match (staged, index) {
        (true, _) => Action::Unstage(path),
        (false, 0) => Action::Stage(path),
        (false, _) => Action::Discard(path),
    };

    for change in changes {
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), ROW));
        let id = Id::new(("scm_row", staged, &change.path));
        let resp = ui.interact(rect, id, Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        ui::row_bg(ui, theme, rect, id, hovered, false);
        ui::item(ui, &resp, ui::Role::ListItem, &format!("{}, {:?}", change.path, change.state), false);

        let (dir, name) = match change.path.trim_end_matches('/').rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => ("", change.path.as_str()),
        };
        let conflicted = !staged && change.state == FileState::Conflicted;
        let row_items: &[_] = if conflicted { &CONFLICTED } else { row_items };
        let color = state_color(theme, Some(change.state));
        let reserved = if hovered { 34.0 + 20.0 * row_items.len() as f32 } else { 28.0 };
        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - reserved));
        let y = rect.center().y;
        let x = rect.left() + 14.0;
        if !crate::icon_theme::paint(&painter, pos2(x + 7.5, y), 15.0, name, crate::icon_theme::Kind::File, 1.0) {
            painter.text(pos2(x, y), Align2::LEFT_CENTER, file_icon(name), FontId::proportional(15.0), theme.text_dim);
        }
        let drawn = painter.text(pos2(x + 21.0, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
        painter.text(pos2(drawn.right() + 8.0, y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
        let badge = if staged && change.state == FileState::Added { "A" } else { state_badge(Some(change.state)) };
        ui.painter().text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);

        let full = top.join(&change.path);
        let hit = if hovered { buttons(ui, theme, rect.with_max_x(rect.right() - 22.0), !busy, row_items) } else { None };
        let whole_file = !staged && matches!(change.state, FileState::Added | FileState::Conflicted);
        if conflicted {
            resp.context_menu(|ui| {
                ui::menu_pad(ui);
                if ui.button("Resolve in Merge Editor").clicked() {
                    cmds.push(Command::OpenMerge(full.clone()));
                    ui.close();
                }
            });
        }
        match hit {
            Some(index) if index + 1 == row_items.len() => cmds.push(Command::OpenPath(full)),
            Some(index) if row_items[index].0 == icons::GIT_MERGE => cmds.push(Command::OpenMerge(full)),
            Some(index) => *action = Some(act(index, Some(change.path.clone()))),
            None if resp.clicked() && whole_file && full.is_file() => cmds.push(Command::OpenPath(full)),
            None if resp.clicked() && !whole_file => {
                let (label, cached) = if staged { ("staged", Some("--cached")) } else { ("changes", None) };
                let args = ["diff"].into_iter().chain(cached).chain(["--", &change.path]).map(str::to_owned).collect();
                cmds.push(Command::OpenDiff(format!("{name} ({label})"), args));
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput};

    fn frame(ctx: &egui::Context, scm: &mut Scm, git: &mut Git, events: Vec<Event>) -> Vec<Command> {
        git.poll();
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 600.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let mut cmds = vec![];
        let mut out = ctx.run_ui(input, |ui| scm.ui(ui, &Theme::dark(), git, &mut cmds));
        out.textures_delta.clear();
        cmds
    }

    fn button(pos: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    fn click(ctx: &egui::Context, scm: &mut Scm, git: &mut Git, x: f32, y: f32) -> Vec<Command> {
        let pos = pos2(x, y);
        frame(ctx, scm, git, vec![Event::PointerMoved(pos)]);
        frame(ctx, scm, git, vec![button(pos, true)]);
        frame(ctx, scm, git, vec![button(pos, false)])
    }

    #[test]
    fn prompt_validates_enter_and_keeps_multiline_newlines() {
        let ctx = egui::Context::default();
        let (mut title, mut body) = (String::new(), String::new());
        let mut frame = |events| {
            let input = RawInput { events, focused: true, ..Default::default() };
            let mut choice = None;
            let mut out = ctx.run_ui(input, |ui| {
                choice = prompt(ui, "Review", "", &mut [("Title", &mut title, 1), ("Body", &mut body, 4)], "Submit");
            });
            out.textures_delta.clear();
            choice
        };
        let enter = |modifiers| [true, false].map(|pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
        frame(vec![]);
        assert_eq!(frame(enter(Modifiers::NONE)), None);
        frame(vec![Event::Text("review".into())]);
        ctx.memory_mut(|m| m.request_focus(Id::new(("scm_field", "Review", 1usize))));
        assert_eq!(frame(enter(Modifiers::NONE)), None);
        assert!(!ctx.input(|i| i.key_pressed(Key::Enter)));
        assert_eq!(frame(enter(Modifiers::CTRL)), Some(true));
        assert_eq!(body, "\n");
    }

    #[test]
    fn stages_discards_and_commits_from_the_side_bar() {
        let dir = std::env::temp_dir().join(format!("halo-scm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        run(&["init", "-q", "-b", "trunk"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("a.txt"), "two").unwrap();

        let ctx = egui::Context::default();
        let (mut scm, mut git) = (Scm::new(), Git::new());
        git.set_roots(vec![dir.clone()], &ctx);
        git.wait(|g| g.changes(false).len() == 1);

        frame(&ctx, &mut scm, &mut git, vec![]);
        let row = ctx.read_response(Id::new(("scm_row", false, "a.txt"))).unwrap().rect.center().y;
        let diff = Command::OpenDiff("a.txt (changes)".into(), ["diff", "--", "a.txt"].map(str::to_owned).to_vec());
        assert_eq!(click(&ctx, &mut scm, &mut git, 100.0, row), [diff]);
        assert_eq!(click(&ctx, &mut scm, &mut git, 222.0, row), [Command::OpenPath(git.top().unwrap().join("a.txt"))]);

        click(&ctx, &mut scm, &mut git, 242.0, row);
        assert_eq!(scm.discard, Some(Some("a.txt".into())));
        frame(&ctx, &mut scm, &mut git, vec![]);
        let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        frame(&ctx, &mut scm, &mut git, vec![key(Key::Escape)]);
        assert!(scm.discard.is_none() && !ctx.input(|i| i.key_pressed(Key::Escape)));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "two");
        frame(&ctx, &mut scm, &mut git, vec![]);

        click(&ctx, &mut scm, &mut git, 262.0, row);
        git.wait(|g| g.changes(true).len() == 1 && g.changes(false).is_empty());

        ctx.memory_mut(|m| m.request_focus(Id::new("scm_message")));
        frame(&ctx, &mut scm, &mut git, vec![]);
        frame(&ctx, &mut scm, &mut git, vec![Event::Text("second".into())]);
        let modifiers = Modifiers { ctrl: true, command: true, ..Default::default() };
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
        let release = Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers };
        frame(&ctx, &mut scm, &mut git, vec![enter, release]);
        git.wait(|g| !g.busy() && g.changes(true).is_empty());
        frame(&ctx, &mut scm, &mut git, vec![]);
        assert_eq!(git.error(), None);
        assert_eq!(scm.message, "");
        assert_eq!(run(&["log", "--format=%s"]), "second\ninit\n");

        std::fs::write(dir.join("a.txt"), "three").unwrap();
        git.refresh(&ctx);
        git.wait(|g| g.changes(false).len() == 1);
        scm.discard = Some(Some("a.txt".into()));
        frame(&ctx, &mut scm, &mut git, vec![]);
        frame(&ctx, &mut scm, &mut git, vec![key(Key::Enter)]);
        assert!(scm.discard.is_none() && !ctx.input(|i| i.key_pressed(Key::Enter)));
        git.wait(|g| !g.busy() && g.changes(false).is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "two");

        let head = run(&["rev-parse", "HEAD"]).trim().to_owned();
        frame(&ctx, &mut scm, &mut git, vec![]);
        let row = ctx.read_response(Id::new(("scm_commit", &head))).unwrap().rect.center();
        click(&ctx, &mut scm, &mut git, row.x, row.y);
        assert_eq!(scm.commit.as_deref(), Some(head.as_str()));
        git.wait(|g| g.files(&head).is_some());
        assert_eq!(git.files(&head).unwrap(), [Change { path: "a.txt".into(), state: FileState::Modified }]);
        frame(&ctx, &mut scm, &mut git, vec![]);
        let split = row - vec2(0.0, ROW * 0.5 + ui::SECTION);
        let moved = split + vec2(0.0, 18.0);
        frame(&ctx, &mut scm, &mut git, vec![Event::PointerMoved(split)]);
        frame(&ctx, &mut scm, &mut git, vec![button(split, true)]);
        frame(&ctx, &mut scm, &mut git, vec![Event::PointerMoved(moved)]);
        frame(&ctx, &mut scm, &mut git, vec![button(moved, false)]);
        assert!(scm.sizes[2] > scm.sizes[3]);
        click(&ctx, &mut scm, &mut git, moved.x, moved.y + ui::SECTION * 0.5);
        assert_eq!(scm.open, [true, true, false]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shows_incoming_and_outgoing_groups_above_the_graph() {
        let base = std::env::temp_dir().join(format!("halo-scm-sides-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, remote) = (base.join("work"), base.join("remote.git"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        crate::git::exec(&remote, &["init", "-q", "--bare"]).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        run(&["init", "-q", "-b", "trunk"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        run(&["remote", "add", "origin", remote.to_str().unwrap()]);
        run(&["commit", "-q", "--allow-empty", "-m", "base"]);
        run(&["push", "-q", "-u", "origin", "trunk"]);
        run(&["commit", "-q", "--allow-empty", "-m", "remote only"]);
        run(&["push", "-q"]);
        run(&["reset", "-q", "--hard", "HEAD~1"]);
        run(&["commit", "-q", "--allow-empty", "-m", "local only"]);

        let ctx = egui::Context::default();
        let (mut scm, mut git) = (Scm::new(), Git::new());
        git.set_roots(vec![dir.clone()], &ctx);
        git.wait(|g| g.tracking() == Some((1, 1)));
        frame(&ctx, &mut scm, &mut git, vec![]);
        let first = git.log()[0].hash.clone();
        assert!(git.unsynced(&first).is_some());
        let row = ctx.read_response(Id::new(("scm_commit", &first))).unwrap().rect.center();
        assert_eq!(click(&ctx, &mut scm, &mut git, row.x, row.y - ROW), [Command::Git(GitCommand::Push)]);
        assert_eq!(click(&ctx, &mut scm, &mut git, row.x, row.y - 2.0 * ROW), [Command::Git(GitCommand::Pull)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn opens_conflicted_files_in_the_merge_editor() {
        let dir = std::env::temp_dir().join(format!("halo-scm-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args);
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"]] {
            run(args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "init"]).unwrap();
        run(&["checkout", "-q", "-b", "feature"]).unwrap();
        std::fs::write(dir.join("a.txt"), "theirs\n").unwrap();
        run(&["commit", "-q", "-am", "theirs"]).unwrap();
        run(&["checkout", "-q", "trunk"]).unwrap();
        std::fs::write(dir.join("a.txt"), "ours\n").unwrap();
        run(&["commit", "-q", "-am", "ours"]).unwrap();
        assert!(run(&["merge", "-q", "feature"]).is_err());

        let ctx = egui::Context::default();
        let (mut scm, mut git) = (Scm::new(), Git::new());
        git.set_roots(vec![dir.clone()], &ctx);
        git.wait(|g| g.changes(false).iter().any(|c| c.state == FileState::Conflicted));
        frame(&ctx, &mut scm, &mut git, vec![]);
        let row = ctx.read_response(Id::new(("scm_row", false, "a.txt"))).unwrap().rect.center().y;
        let merge = Command::OpenMerge(git.top().unwrap().join("a.txt"));
        assert_eq!(click(&ctx, &mut scm, &mut git, 222.0, row), [merge]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
