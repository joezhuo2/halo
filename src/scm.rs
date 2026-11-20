use crate::command::Command;
use crate::explorer::{file_icon, state_badge, state_color};
use crate::git::{Change, FileState, Git};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, FontId, Frame, Id, Key, Layout, Margin, Modifiers, RichText, ScrollArea, Sense, Ui,
    UiBuilder, pos2, vec2,
};
use std::path::Path;

const ROW: f32 = 22.0;

enum Action {
    Stage(Option<String>),
    Unstage(Option<String>),
    Commit,
}

pub struct Scm {
    message: String,
    committing: bool,
}

impl Scm {
    pub fn new() -> Self {
        Self { message: String::new(), committing: false }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, git: &mut Git, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui::section_header(ui, theme, "Source Control");
        let Some(top) = git.top().map(Path::to_path_buf) else {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("The open folder is not a git repository.").color(theme.text_dim));
            });
            return;
        };
        if self.committing && !git.busy() {
            self.committing = false;
            if git.error().is_none() {
                self.message.clear();
            }
        }

        let mut action = None;
        let busy = git.busy();
        Frame::new().inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
            let id = Id::new("scm_message");
            let can_commit = !busy && !git.changes(true).is_empty() && !self.message.trim().is_empty();
            let submit = ui.memory(|m| m.has_focus(id))
                && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
            let hint = format!("Message (Ctrl+Enter to commit on \"{}\")", git.branch().unwrap_or_default());
            ui.add(
                egui::TextEdit::multiline(&mut self.message)
                    .id(id)
                    .hint_text(hint)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            let label = RichText::new(format!("{}  Commit", icons::CHECK)).color(theme.accent);
            let button = ui
                .add_enabled(can_commit, egui::Button::new(label).min_size(vec2(ui.available_width(), 26.0)))
                .on_disabled_hover_text("Stage changes and type a message to commit.");
            if can_commit && (button.clicked() || submit) {
                action = Some(Action::Commit);
            }
            if let Some(error) = git.error() {
                ui.add_space(6.0);
                ui.label(RichText::new(error).color(theme.error));
            }
            ui.add_space(4.0);
        });

        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if git.changes(true).is_empty() && git.changes(false).is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("No changes.").color(theme.text_dim));
                });
            }
            for staged in [true, false] {
                let changes = git.changes(staged);
                if !changes.is_empty() {
                    group(ui, theme, &top, changes, staged, busy, &mut action, cmds);
                }
            }
        });

        match action {
            Some(Action::Stage(path)) => git.stage(path.as_deref()),
            Some(Action::Unstage(path)) => git.unstage(path.as_deref()),
            Some(Action::Commit) => {
                git.commit(self.message.trim());
                self.committing = true;
            }
            None => {}
        }
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
    let (title, icon, tip, all_tip) = if staged {
        ("STAGED CHANGES", icons::MINUS, "Unstage", "Unstage All")
    } else {
        ("CHANGES", icons::PLUS, "Stage", "Stage All")
    };
    let act = |path: Option<String>| if staged { Action::Unstage(path) } else { Action::Stage(path) };
    let buttons = |ui: &mut Ui, rect: egui::Rect, tip: &str| {
        let builder = UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::right_to_left(Align::Center));
        ui.scope_builder(builder, |ui| !busy && ui::icon_button(ui, theme, icon, 20.0, false, tip).clicked()).inner
    };

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0, ui::mix(theme.panel, theme.bar, 0.5));
    let y = rect.center().y;
    let text = format!("{title}  {}", changes.len());
    painter.text(pos2(rect.left() + 12.0, y), Align2::LEFT_CENTER, text, FontId::proportional(11.5), theme.text);
    if buttons(ui, rect, all_tip) {
        *action = Some(act(None));
    }

    for change in changes {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        let id = Id::new(("scm_row", staged, &change.path));
        let hovered = ui.rect_contains_pointer(rect);
        ui::row_bg(ui, theme, rect, id, hovered, false);

        let (dir, name) = match change.path.trim_end_matches('/').rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => ("", change.path.as_str()),
        };
        let color = state_color(theme, Some(change.state));
        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 50.0));
        let y = rect.center().y;
        let x = rect.left() + 14.0;
        painter.text(pos2(x, y), Align2::LEFT_CENTER, file_icon(name), FontId::proportional(15.0), theme.text_dim);
        let drawn = painter.text(pos2(x + 21.0, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
        painter.text(pos2(drawn.right() + 8.0, y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
        let badge = if staged && change.state == FileState::Added { "A" } else { state_badge(Some(change.state)) };
        ui.painter().text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);

        let full = top.join(&change.path);
        if hovered && buttons(ui, rect.with_max_x(rect.right() - 22.0), tip) {
            *action = Some(act(Some(change.path.clone())));
        } else if resp.clicked() && full.is_file() {
            cmds.push(Command::OpenPath(full));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput, Rect, pos2};

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

    fn click(ctx: &egui::Context, scm: &mut Scm, git: &mut Git, x: f32, y: f32) -> Vec<Command> {
        let pos = pos2(x, y);
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, scm, git, vec![Event::PointerMoved(pos)]);
        frame(ctx, scm, git, vec![button(true)]);
        frame(ctx, scm, git, vec![button(false)])
    }

    fn wait(git: &mut Git, done: impl Fn(&Git) -> bool) {
        for _ in 0..500 {
            git.poll();
            if done(git) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("git did not settle: {:?}", git.error());
    }

    #[test]
    fn stages_from_a_row_and_commits_with_ctrl_enter() {
        let dir = std::env::temp_dir().join(format!("halo-scm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        run(&["init", "-q", "-b", "trunk"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        run(&["commit", "-q", "--allow-empty", "-m", "init"]);
        std::fs::write(dir.join("new.txt"), "x").unwrap();

        let ctx = egui::Context::default();
        let (mut scm, mut git) = (Scm::new(), Git::new());
        git.set_root(Some(dir.clone()), &ctx);
        wait(&mut git, |g| g.changes(false).len() == 1);

        let row = (0..150)
            .map(|i| i as f32 * 4.0)
            .find(|y| !click(&ctx, &mut scm, &mut git, 100.0, *y).is_empty())
            .expect("a row that opens its file")
            + 10.0;
        assert_eq!(click(&ctx, &mut scm, &mut git, 100.0, row), [Command::OpenPath(git.top().unwrap().join("new.txt"))]);

        click(&ctx, &mut scm, &mut git, 262.0, row);
        wait(&mut git, |g| g.changes(true).len() == 1 && g.changes(false).is_empty());

        ctx.memory_mut(|m| m.request_focus(Id::new("scm_message")));
        frame(&ctx, &mut scm, &mut git, vec![]);
        frame(&ctx, &mut scm, &mut git, vec![Event::Text("add new".into())]);
        let modifiers = Modifiers { ctrl: true, command: true, ..Default::default() };
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
        frame(&ctx, &mut scm, &mut git, vec![enter]);
        wait(&mut git, |g| !g.busy() && g.changes(true).is_empty());
        frame(&ctx, &mut scm, &mut git, vec![]);
        assert_eq!(git.error(), None);
        assert_eq!(scm.message, "");
        assert_eq!(run(&["log", "--format=%s"]), "add new\ninit\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
