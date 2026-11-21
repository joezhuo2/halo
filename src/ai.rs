use crate::command::Command;
use crate::terminal::Terminal;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Align2, CornerRadius, FontId, Id, Layout, Rect, RichText, Sense, Ui, UiBuilder, ViewportCommand, pos2, vec2};
use std::path::Path;

const CLIS: [(&str, &str); 2] = [("Claude", "claude"), ("Codex", "codex")];
const TERMINAL_ID: u64 = 1 << 40;

fn launch(cli: &str, cwd: Option<&Path>) -> (String, Vec<String>) {
    if cwd.and_then(crate::wsl::parse).is_some() {
        crate::wsl::wrap(cwd, "bash", &["-lic".into(), cli.into()])
    } else if cfg!(windows) {
        ("cmd.exe".into(), vec!["/c".into(), cli.into()])
    } else {
        (cli.into(), vec![])
    }
}

#[derive(Default)]
pub struct AiPanel {
    tab: usize,
    sessions: [Option<Terminal>; 2],
    errors: [Option<String>; 2],
    focused: bool,
}

impl AiPanel {
    pub fn has_focus(&self) -> bool {
        self.focused
    }

    pub fn blur(&mut self) {
        self.focused = false;
    }

    fn spawn(&mut self, cli: &str, cwd: Option<&Path>, ctx: &egui::Context) {
        let (program, args) = launch(cli, cwd);
        match Terminal::spawn(TERMINAL_ID + self.tab as u64, CLIS[self.tab].0, &program, &args, cwd, ctx.clone()) {
            Ok(terminal) => {
                self.sessions[self.tab] = Some(terminal);
                self.errors[self.tab] = None;
            }
            Err(error) => self.errors[self.tab] = Some(format!("Could not start {cli}: {error}")),
        }
    }

    pub fn exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        let Some(session) = self.sessions[self.tab].as_mut() else { return false };
        match cmd {
            Command::Paste => {
                session.focus();
                ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
            }
            Command::Copy => {
                if let Some(text) = session.selection() {
                    ctx.copy_text(text);
                }
            }
            _ => return false,
        }
        true
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, font_size: f32, cwd: Option<&Path>, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let header = Rect::from_min_size(full.min, vec2(full.width(), 32.0));
        let mut body = Rect::from_min_max(pos2(full.left(), header.bottom()), full.max);
        ui.allocate_rect(full, Sense::hover());

        let before = self.tab;
        let mut start = false;
        ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(10.0, 0.0))), |ui| {
            ui.horizontal_centered(|ui| {
                ui::tabs(ui, theme, Id::new("ai_tab"), &CLIS.map(|c| c.0), &mut self.tab);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui::icon_button(ui, theme, icons::X, 24.0, false, "Close AI Panel (Ctrl+Alt+B)").clicked() {
                        cmds.push(Command::ToggleAi);
                    }
                    if self.sessions[self.tab].is_some() {
                        if ui::icon_button(ui, theme, icons::STOP, 24.0, false, "Stop Session").clicked() {
                            self.sessions[self.tab] = None;
                        }
                        start |= ui::icon_button(ui, theme, icons::ARROW_CLOCKWISE, 24.0, false, "Restart Session").clicked();
                    }
                });
            });
        });
        if self.tab != before
            && let Some(session) = &mut self.sessions[self.tab]
        {
            session.focus();
        }

        let (name, cli) = CLIS[self.tab];
        let ended = self.sessions[self.tab].as_ref().is_some_and(|s| !s.is_alive());
        let note = match &self.errors[self.tab] {
            Some(error) => Some((error.clone(), theme.error)),
            None => ended.then(|| (format!("The {name} session ended."), theme.text_dim)),
        };
        if let Some((text, color)) = note {
            let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - 28.0), body.max);
            body.max.y = bar.top();
            ui.painter().rect_filled(bar, CornerRadius::ZERO, ui::mix(theme.panel, color, 0.18));
            let clip = ui.painter().with_clip_rect(bar.shrink2(vec2(10.0, 0.0)));
            clip.text(pos2(bar.left() + 10.0, bar.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), color);
        }

        self.focused = false;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            match &mut self.sessions[self.tab] {
                Some(session) => {
                    session.ui(ui, theme, font_size, cmds);
                    self.focused = session.has_focus();
                }
                None => {
                    ui.set_opacity(ui::fade_in(ui, Id::new(("ai_start", self.tab))));
                    ui.vertical_centered(|ui| {
                        ui.add_space(28.0);
                        ui.label(RichText::new(icons::ROBOT).size(30.0).color(theme.text_dim));
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("Runs the {cli} CLI in the open folder.")).color(theme.text_dim));
                        ui.add_space(10.0);
                        start |= ui.button(RichText::new("Start session").color(theme.accent)).clicked();
                    });
                }
            }
        });
        if start {
            self.spawn(cli, cwd, &ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_an_ended_session_and_starts_nothing_unasked() {
        let ctx = egui::Context::default();
        let mut panel = AiPanel::default();
        let mut cmds = vec![];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| panel.ui(ui, &Theme::dark(), 13.0, None, &mut cmds));
        out.textures_delta.clear();
        assert!(panel.sessions.iter().all(Option::is_none));
        assert!(!panel.exec(&Command::Copy, &ctx));

        panel.spawn("halo-no-such-cli-xyz", None, &ctx);
        let started = std::time::Instant::now();
        let ended = |p: &AiPanel| p.errors[0].is_some() || p.sessions[0].as_ref().is_some_and(|s| !s.is_alive());
        while !ended(&panel) && started.elapsed() < std::time::Duration::from_secs(15) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(ended(&panel));
        assert_eq!(panel.sessions[0].is_some(), cfg!(windows));
    }
}
