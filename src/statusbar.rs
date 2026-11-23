use crate::app::Halo;
use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use crate::editor::Encoding;
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Frame, Layout, Panel, Response, RichText, ScrollArea, Sense, Ui, vec2};

enum Pick {
    Language(String),
    Indent(Option<(bool, usize)>),
    Convert(bool),
    Encoding(Encoding, bool),
    Eol(bool),
}

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

                    if let Some(name) = self.session.workspace.name().filter(|_| self.session.workspace.is_multi())
                        && item(ui, &theme, &format!("{} {name}", icons::FOLDERS), false, "Workspace folders: show the Explorer").clicked()
                    {
                        self.cmds.push(Command::ShowExplorer);
                    }

                    if self.settings.status_problems {
                        let (errors, warnings) = if self.settings.diagnostics { self.editors.lsp.counts() } else { (0, 0) };
                        let (task_errors, task_warnings) = self.tasks.counts();
                        let (errors, warnings) = (errors + task_errors, warnings + task_warnings);
                        let problems = format!("{} {errors}   {} {warnings}", icons::X_CIRCLE, icons::WARNING);
                        let tip = match errors + warnings {
                            0 => "No problems".to_owned(),
                            _ => format!("{errors} errors, {warnings} warnings"),
                        };
                        if item(ui, &theme, &problems, false, &tip).clicked() {
                            self.cmds.push(Command::ShowProblems);
                        }
                    }

                    let running = self.tasks.running_count();
                    if running > 0 {
                        let tip = "Running tasks (click to list)";
                        if item(ui, &theme, &format!("{} {running}", icons::GEAR_SIX), true, tip).clicked() {
                            self.cmds.push(Command::ShowRunningTasks);
                        }
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

                    if let Some((label, tip, diff)) = self.editors.blame_item(ui.ctx()) {
                        let resp = item(ui, &theme, &label, false, if diff.is_some() { "" } else { &tip });
                        let resp = match diff.is_some() {
                            true => resp.on_hover_ui(|ui| self.editors.blame_card(ui, &theme, &mut self.cmds)),
                            false => resp,
                        };
                        if resp.clicked() && let Some(cmd) = diff {
                            self.cmds.push(cmd);
                        }
                    } else if self.editors.blame_loading() {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, HEIGHT), Sense::hover());
                        ui::spinner(ui, r.center(), 5.0, theme.text_dim);
                    }

                    if let Some(version) = &self.updated {
                        let tip = "This version is installed and is used the next time Halo opens";
                        item(ui, &theme, &format!("{} {version}", icons::ARROW_CIRCLE_UP), true, tip);
                    }

                    if let Some((label, tip)) = self.debug.status()
                        && item(ui, &theme, &label, true, &tip).clicked()
                    {
                        self.cmds.push(Command::ShowRunAndDebug);
                    }

                    if let Some((label, tip)) = self.testing.status()
                        && item(ui, &theme, &label, false, &tip).clicked()
                    {
                        self.cmds.push(Command::ShowTesting);
                    }

                    if let Some(first) = self.chord {
                        let label = format!("({}) was pressed. Waiting for the second key...", ui.ctx().format_shortcut(&first));
                        item(ui, &theme, &label, true, "Key chord");
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(6.0);
                        let (unread, dnd) = crate::notifications::summary();
                        let bell = format!("{}{}", if dnd { icons::BELL_SLASH } else { icons::BELL }, if unread > 0 { format!(" {unread}") } else { String::new() });
                        let tip = if dnd { "Notifications (Do Not Disturb is on)" } else { "Notifications" };
                        let resp = item(ui, &theme, &bell, unread > 0, tip);
                        crate::notifications::set_bell(resp.rect);
                        if resp.clicked() {
                            self.cmds.push(Command::ShowNotifications);
                        }
                        let icon = if theme.dark { icons::SUN } else { icons::MOON };
                        if item(ui, &theme, icon, false, "Color theme").clicked() {
                            self.cmds.push(Command::SelectTheme);
                        }
                        if let Some(text) = self.editors.image_status() {
                            item(ui, &theme, &text, false, "Image size, file size and zoom");
                        } else if let Some(status) = self.editors.status() {
                            let mut pick = None;
                            let language = item(ui, &theme, &status.language, false, "Select language mode");
                            egui::Popup::menu(&language).show(|ui| {
                                ui::menu_pad(ui);
                                ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                                    for name in std::iter::once("Plain Text").chain(crate::editor::languages().iter().map(|l| l.0.as_str())) {
                                        if ui.button(name).clicked() {
                                            pick = Some(Pick::Language(name.to_owned()));
                                        }
                                    }
                                });
                            });
                            let scoped = self.settings.of(self.editors.active_path().as_deref());
                            let fallback = (scoped.insert_spaces, scoped.tab_size);
                            let (spaces, size) = status.indent.map_or(fallback, |(spaces, size)| (spaces, if size == 0 { fallback.1 } else { size }));
                            let label = format!("{}: {size}", if spaces { "Spaces" } else { "Tab Size" });
                            let indent = item(ui, &theme, &label, false, "Select indentation");
                            egui::Popup::menu(&indent).show(|ui| {
                                ui::menu_pad(ui);
                                for (spaces, size) in [(true, 2), (true, 4), (true, 8), (false, 2), (false, 4), (false, 8)] {
                                    let label = format!("Indent Using {}: {size}", if spaces { "Spaces" } else { "Tabs" });
                                    if ui.button(label).clicked() {
                                        pick = Some(Pick::Indent(Some((spaces, size))));
                                    }
                                }
                                ui.separator();
                                if ui.button("Detect Indentation from Content").clicked() {
                                    pick = Some(Pick::Indent(None));
                                }
                                for (label, spaces) in [("Convert Indentation to Spaces", true), ("Convert Indentation to Tabs", false)] {
                                    if ui.button(label).clicked() {
                                        pick = Some(Pick::Convert(spaces));
                                    }
                                }
                            });
                            let encoding = item(ui, &theme, status.encoding, false, "Select encoding");
                            egui::Popup::menu(&encoding).show(|ui| {
                                ui::menu_pad(ui);
                                for (reopen, title) in [(true, "Reopen with Encoding"), (false, "Save with Encoding")] {
                                    ui.label(RichText::new(title).size(11.0).color(theme.text_dim));
                                    for encoding in Encoding::ALL {
                                        if ui.button(encoding.label()).clicked() {
                                            pick = Some(Pick::Encoding(encoding, reopen));
                                        }
                                    }
                                }
                            });
                            let eol = item(ui, &theme, if status.crlf { "CRLF" } else { "LF" }, false, "Select end of line sequence");
                            egui::Popup::menu(&eol).show(|ui| {
                                ui::menu_pad(ui);
                                for (label, crlf) in [("LF", false), ("CRLF", true)] {
                                    if ui.button(label).clicked() {
                                        pick = Some(Pick::Eol(crlf));
                                    }
                                }
                            });
                            match pick {
                                Some(Pick::Language(name)) => self.editors.set_language(&name),
                                Some(Pick::Indent(indent)) => self.editors.set_indent(indent),
                                Some(Pick::Convert(spaces)) => self.editors.convert_indent(spaces, &self.settings),
                                Some(Pick::Encoding(encoding, reopen)) => self.editors.set_encoding(encoding, reopen),
                                Some(Pick::Eol(crlf)) => self.editors.set_eol(crlf),
                                None => {}
                            }
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
                        if let Some((status, tip)) = self.editors.lsp.status() {
                            item(ui, &theme, &format!("{} {status}", icons::PLUGS_CONNECTED), false, &tip);
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
    ui::access(ui, resp.id, ui::Role::Button, &ui::plain(text), |n| n.set_description(tip));
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(Color32::TRANSPARENT, theme.hover, h));
        let color = ui::mix(ui::mix(theme.text_dim, theme.text, h), theme.accent, a);
        let pos = rect.center() - galley.size() / 2.0;
        ui.painter().galley(pos, galley, color);
        ui::focus_ring(ui, theme, rect, resp.id);
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}
