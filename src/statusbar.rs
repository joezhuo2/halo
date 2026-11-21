use crate::app::Halo;
use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, Panel, Response, Sense, Ui, vec2};

const HEIGHT: f32 = 24.0;

impl Halo {
    pub(crate) fn statusbar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        Panel::bottom("statusbar")
            .exact_size(HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(theme.bar))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(6.0);

                    if let Some((distro, _)) = self.session.folder.as_deref().and_then(crate::wsl::parse) {
                        item(ui, &theme, &format!("{} WSL: {distro}", icons::LINUX_LOGO), true, "This folder is open in WSL");
                    }

                    let problems = format!("{} 0   {} 0", icons::X_CIRCLE, icons::WARNING);
                    if item(ui, &theme, &problems, false, "No problems").clicked() {
                        self.cmds.push(Command::TogglePanel);
                    }

                    if let Some(branch) = self.git.branch() {
                        let repo = self.git.repo_name().unwrap_or("repo");
                        let label = format!("{} {} / {}", icons::GIT_BRANCH, repo, branch);
                        if item(ui, &theme, &label, false, "Repository / branch").clicked() {
                            self.cmds.push(Command::ShowSourceControl);
                        }
                        let (label, tip) = match self.git.tracking() {
                            Some((ahead, behind)) => (
                                format!("{} {behind}{} {ahead}{}", icons::ARROWS_CLOCKWISE, icons::ARROW_DOWN, icons::ARROW_UP),
                                "Synchronize changes: pull, then push",
                            ),
                            None => (icons::CLOUD_ARROW_UP.to_owned(), "Publish branch to origin"),
                        };
                        if item(ui, &theme, &label, self.git.busy(), tip).clicked() {
                            self.cmds.push(Command::GitSync);
                        }
                    }

                    if let Some(first) = self.chord {
                        let label = format!("({}) was pressed. Waiting for the second key...", ui.ctx().format_shortcut(&first));
                        item(ui, &theme, &label, true, "Key chord");
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(6.0);
                        let icon = if theme.dark { icons::SUN } else { icons::MOON };
                        if item(ui, &theme, icon, false, "Switch theme").clicked() {
                            self.cmds.push(Command::ToggleTheme);
                        }
                        if let Some(status) = self.editors.status() {
                            item(ui, &theme, &status.language, false, "Language");
                            let mut position = format!("Ln {}, Col {}", status.line, status.column);
                            if status.cursors > 1 {
                                position = format!("{} cursors", status.cursors);
                            } else if status.selected > 0 {
                                position.push_str(&format!(" ({} selected)", status.selected));
                            }
                            if item(ui, &theme, &position, false, "Go to line (Ctrl+G)").clicked() {
                                self.cmds.push(Command::GoToLine);
                            }
                        }
                    });
                });
            });
    }
}

fn item(ui: &mut Ui, theme: &Theme, text: &str, active: bool, tip: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.0), Color32::PLACEHOLDER);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, HEIGHT), Sense::click());
    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
    let a = ui::anim(ui, resp.id.with("active"), active);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(Color32::TRANSPARENT, theme.hover, h));
        let color = ui::mix(ui::mix(theme.text_dim, theme.text, h), theme.accent, a);
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, color);
    }
    resp.on_hover_text(tip)
}
