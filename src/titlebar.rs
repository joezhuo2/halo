use crate::app::{Halo, Zone};
use crate::command::{self, Command};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Panel, PointerButton, Rect, Response,
    Sense, Stroke, StrokeKind, Ui, ViewportCommand, vec2,
};

const HEIGHT: f32 = 36.0;
const NATIVE_BUTTONS: bool = cfg!(target_os = "macos");

impl Halo {
    pub(crate) fn titlebar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        Panel::top("titlebar")
            .exact_size(HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(theme.bar))
            .show(ui, |ui| {
                let bar = ui.max_rect();
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                let fullscreen = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));

                let drag = ui.interact(bar, Id::new("titlebar_drag"), Sense::click_and_drag());
                if drag.double_clicked() {
                    ui.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started_by(PointerButton::Primary) {
                    ui.send_viewport_cmd(ViewportCommand::StartDrag);
                }

                let mut menus_right = bar.left();
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(if NATIVE_BUTTONS && !fullscreen { 78.0 } else { 14.0 });
                    ui::logo(ui, 20.0, theme.dark);
                    ui.add_space(10.0);
                    menus_right = egui::MenuBar::new()
                        .ui(ui, |ui| {
                            ui.spacing_mut().button_padding = vec2(9.0, 5.0);
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let widgets = &mut ui.visuals_mut().widgets;
                            for state in [&mut widgets.hovered, &mut widgets.open, &mut widgets.active] {
                                state.corner_radius = CornerRadius::same(5);
                            }
                            if self.session.layout.zen.is_none() {
                                self.menus(ui);
                            }
                            ui.cursor().left()
                        })
                        .inner;

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if NATIVE_BUTTONS {
                            return;
                        }
                        if window_button(ui, &theme, icons::X, true, "Close").clicked() {
                            ui.send_viewport_cmd(ViewportCommand::Close);
                        }
                        let (icon, tip) = if maximized { (icons::CORNERS_IN, "Restore") } else { (icons::SQUARE, "Maximize") };
                        if window_button(ui, &theme, icon, false, tip).clicked() {
                            ui.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                        }
                        if window_button(ui, &theme, icons::MINUS, false, "Minimize").clicked() {
                            ui.send_viewport_cmd(ViewportCommand::Minimized(true));
                        }
                    });
                });

                let width = (bar.width() * 0.34).clamp(160.0, 560.0);
                let mut search = Rect::from_center_size(bar.center(), vec2(width, 24.0));
                search.min.x = search.min.x.max(menus_right + 12.0);
                search.max.x = search.max.x.min(bar.right() - if NATIVE_BUTTONS { 0.0 } else { 44.0 * 3.0 } - 12.0);
                if self.search.is_open() {
                    let rect = Rect::from_center_size(bar.center(), vec2((bar.width() - 24.0).min(600.0), 26.0));
                    let roots = self.session.workspace.prefixed();
                    if self.search.ui(ui, &theme, rect, &roots, &mut self.editors, &mut self.cmds) && self.zone == Zone::Editor {
                        self.editors.focus();
                    }
                } else if search.width() > 90.0 {
                    self.search_box(ui, &theme, search);
                }
            });
    }

    fn search_box(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect) {
        let resp = ui
            .interact(rect, Id::new("titlebar_search"), Sense::click())
            .on_hover_text("Search files, > commands, % text, @ symbols, # workspace symbols, : line (Ctrl+P)");
        if resp.clicked() {
            self.cmds.push(Command::GoToFile);
        }
        ui::access(ui, resp.id, ui::Role::Button, "Search files, commands, text and symbols (Ctrl+P)", |_| {});
        let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(6),
            ui::mix(theme.raised, theme.hover, h),
            Stroke::new(1.0, ui::mix(theme.border, theme.accent, h * 0.6)),
            StrokeKind::Inside,
        );
        let label = format!("{}  {}", icons::MAGNIFYING_GLASS, self.folder_name().unwrap_or_else(|| "Halo".into()));
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.5),
            ui::mix(theme.text_dim, theme.text, h),
        );
    }

    fn menus(&mut self, ui: &mut Ui) {
        use Command::*;
        self.menu(ui, "File", &[
            Some(NewFile), Some(NewWindow), Some(OpenFile), Some(OpenFolder), Some(OpenFolderInNewWindow), Some(OpenWorkspaceFromFile), None,
            Some(AddFolderToWorkspace), Some(SaveWorkspaceAs), None,
            Some(Save), Some(SaveAs), Some(SaveAll), None,
            Some(CompareFile), Some(CompareClipboard), Some(CompareSaved), None,
            Some(CloseEditor), Some(CloseFolder), Some(CloseWorkspace), None,
            Some(OpenSettings), Some(OpenSettingsJson), Some(OpenKeybindings), None,
            Some(SelectTheme), Some(SelectIconTheme), None,
            Some(Exit),
        ]);
        self.menu(ui, "Edit", &[
            Some(Undo), Some(Redo), None,
            Some(Cut), Some(Copy), Some(Paste), None,
            Some(Find), Some(Replace), None,
            Some(ToggleComment), None,
            Some(RenameSymbol), Some(QuickFix), Some(OrganizeImports), Some(FormatDocument), Some(FormatSelection),
        ]);
        self.menu(ui, "Selection", &[
            Some(SelectAll), None,
            Some(AddCursorAbove), Some(AddCursorBelow), Some(SelectNextOccurrence),
        ]);
        self.menu_with(ui, "View", &[
            Some(ShowExplorer), Some(ShowSourceControl), Some(ShowRunAndDebug), Some(ShowTesting), None,
            Some(ToggleSidebar), Some(TogglePanel), Some(ToggleAi), Some(ShowProblems), Some(ShowOutput), Some(ShowNotifications), None,
            Some(SplitEditor), Some(SplitEditorDown), Some(ToggleMaximizeGroup), Some(ResetGroupSizes), Some(ToggleMinimap), Some(MarkdownPreview), Some(MarkdownPreviewSide), None,
            Some(ToggleTheme), Some(ZoomIn), Some(ZoomOut), Some(ZoomReset),
        ], &[
            Some(ToggleFullScreen), Some(ToggleZenMode), Some(ToggleCenteredLayout), None,
            Some(ToggleSidebar), Some(ToggleSecondarySidebar), Some(TogglePanel), Some(ToggleSidebarPosition), None,
            Some(MovePanelLeft), Some(MovePanelRight), Some(MovePanelBottom), Some(MovePanelTop), Some(TogglePanelPosition), Some(ToggleMaximizedPanel), None,
            Some(AlignPanelCenter), Some(AlignPanelJustify), Some(AlignPanelLeft), Some(AlignPanelRight), None,
            Some(ActivityBarSide), Some(ActivityBarTop), Some(ActivityBarHidden), None,
            Some(MoveView), Some(ResetViewLocations),
        ]);
        self.menu(ui, "Go", &[
            Some(GoToFile), Some(GoToSymbol), Some(GoToWorkspaceSymbol), Some(GoToLine), Some(FindInFiles), Some(ShowCommands), None,
            Some(GoToDefinition), Some(GoToDeclaration), Some(GoToTypeDefinition), Some(GoToImplementation), Some(FindReferences),
            Some(ShowCallHierarchy), Some(ShowTypeHierarchy), None,
            Some(GoBack), Some(NextEditor), Some(PrevEditor), None,
            Some(NextChange), Some(PrevChange),
        ]);
        ui.menu_button("Run", |ui| {
            ui.set_min_width(230.0);
            ui::menu_pad(ui);
            for cmd in [StartDebugging, RunWithoutDebugging, StopDebugging, RestartDebugging] {
                self.menu_item(ui, cmd);
            }
            ui.separator();
            for cmd in [StepOver, StepInto, StepOut, PauseDebugging] {
                self.menu_item(ui, cmd);
            }
            ui.separator();
            for cmd in [ToggleBreakpoint, EditBreakpoint, RemoveAllBreakpoints, AddWatch, AddConfiguration] {
                self.menu_item(ui, cmd);
            }
            ui.separator();
            for cmd in [RunAllTests, RunFailedTests, RunTestAtCursor, DebugTestAtCursor, RunAllTestsWithCoverage, CancelTestRun] {
                self.menu_item(ui, cmd);
            }
            ui.separator();
            self.menu_item(ui, RunFile);
            for command in [RunTaskPicker, RunBuildTask, RunTestTask, RerunLastTask, TerminateTask, ShowRunningTasks, ConfigureDefaultBuildTask, OpenTasksJson] {
                self.menu_item(ui, command);
            }
            ui.separator();
            let tasks: Vec<String> = self.settings.tasks.iter().map(|t| t.name.clone()).collect();
            for name in tasks {
                if ui.button(format!("Run Task: {name}")).clicked() {
                    self.cmds.push(RunTask(name));
                    ui.close();
                }
            }
            if ui.button("Configure Run Commands and Tasks...").clicked() {
                self.cmds.push(OpenSettingsJson);
                ui.close();
            }
        });
        ui.menu_button("Terminal", |ui| {
            ui.set_min_width(230.0);
            ui::menu_pad(ui);
            self.menu_item(ui, NewTerminal);
            self.menu_item(ui, SplitTerminal);
            self.menu_item(ui, KillTerminal);
            self.menu_item(ui, NewTerminalInEditor);
            self.menu_item(ui, MoveTerminalToEditor);
            self.menu_item(ui, MoveTerminalToPanel);
            ui.separator();
            self.menu_item(ui, RenameTerminal);
            self.menu_item(ui, ChangeTerminalColor);
            self.menu_item(ui, RunRecentLine);
            self.menu_item(ui, CopyLastCommandOutput);
            self.menu_item(ui, ClearRestoredHistory);
            ui.separator();
            let names: Vec<String> = self.settings.terminals.iter().map(|p| p.name.clone()).collect();
            for name in names {
                if ui.button(format!("New {name}")).clicked() {
                    self.cmds.push(NewTerminalWith(name));
                    ui.close();
                }
            }
        });
        self.menu(ui, "Help", &[Some(InstallInPath), None, Some(About)]);
    }

    fn menu(&mut self, ui: &mut Ui, title: &str, items: &[Option<Command>]) {
        self.menu_with(ui, title, items, &[]);
    }

    fn menu_with(&mut self, ui: &mut Ui, title: &str, items: &[Option<Command>], appearance: &[Option<Command>]) {
        ui.menu_button(title, |ui| {
            ui.set_min_width(230.0);
            ui::menu_pad(ui);
            if !appearance.is_empty() {
                self.menu(ui, "Appearance", appearance);
                ui.separator();
            }
            for item in items {
                match item {
                    Some(cmd) => self.menu_item(ui, cmd.clone()),
                    None => {
                        ui.separator();
                    }
                }
            }
        });
    }

    fn menu_item(&mut self, ui: &mut Ui, cmd: Command) {
        let mut button = egui::Button::new(cmd.label());
        if let Some(shortcut) = command::shortcut_text(ui.ctx(), &cmd) {
            button = button.shortcut_text(shortcut);
        }
        if ui.add(button).clicked() {
            self.cmds.push(cmd);
            ui.close();
        }
    }
}

fn window_button(ui: &mut Ui, theme: &Theme, icon: &str, danger: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(44.0, HEIGHT), Sense::click());
    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
    ui::access(ui, resp.id, ui::Role::Button, tip, |_| {});
    ui::focus_ring(ui, theme, rect, resp.id);
    let (bg, fg) = if danger { (theme.deleted, Color32::WHITE) } else { (theme.hover, theme.text) };
    ui.painter().rect_filled(rect, CornerRadius::ZERO, ui::mix(Color32::TRANSPARENT, bg, h));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(15.0),
        ui::mix(theme.text_dim, fg, h),
    );
    resp.on_hover_text(tip)
}
