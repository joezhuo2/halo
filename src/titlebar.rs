use crate::app::{Halo, Zone};
use crate::command::{self, Command};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Panel, PointerButton, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, Ui, ViewportCommand, vec2,
};

const HEIGHT: f32 = 36.0;

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

                let drag = ui.interact(bar, Id::new("titlebar_drag"), Sense::click_and_drag());
                if drag.double_clicked() {
                    ui.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                } else if drag.drag_started_by(PointerButton::Primary) {
                    ui.send_viewport_cmd(ViewportCommand::StartDrag);
                }

                let mut menus_right = bar.left();
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(12.0);
                    ui.label(RichText::new(icons::CIRCLE_NOTCH).size(18.0).color(theme.accent));
                    ui.add_space(8.0);
                    menus_right = egui::MenuBar::new()
                        .ui(ui, |ui| {
                            self.menus(ui);
                            ui.cursor().left()
                        })
                        .inner;

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                search.max.x = search.max.x.min(bar.right() - 44.0 * 3.0 - 12.0);
                if self.search.is_open() {
                    let rect = Rect::from_center_size(bar.center(), vec2((bar.width() - 24.0).min(600.0), 26.0));
                    let folder = self.session.folder.clone();
                    if self.search.ui(ui, &theme, rect, folder.as_deref(), &self.editors, &mut self.cmds) && self.zone == Zone::Editor {
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
            .on_hover_text("Search files, > commands, % text, @ symbols, : line (Ctrl+P)");
        if resp.clicked() {
            self.cmds.push(Command::GoToFile);
        }
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
            Some(NewFile), Some(OpenFile), Some(OpenFolder), None,
            Some(Save), Some(SaveAs), Some(SaveAll), None,
            Some(CloseEditor), Some(CloseFolder), None,
            Some(OpenSettings), Some(Exit),
        ]);
        self.menu(ui, "Edit", &[
            Some(Undo), Some(Redo), None,
            Some(Cut), Some(Copy), Some(Paste), None,
            Some(Find), Some(Replace), None,
            Some(ToggleComment),
        ]);
        self.menu(ui, "Selection", &[
            Some(SelectAll), None,
            Some(AddCursorAbove), Some(AddCursorBelow), Some(SelectNextOccurrence),
        ]);
        self.menu(ui, "View", &[
            Some(ShowExplorer), Some(ShowSourceControl), None,
            Some(ToggleSidebar), Some(TogglePanel), Some(ToggleAi), None,
            Some(SplitEditor), Some(ToggleMinimap), None,
            Some(ToggleTheme), Some(ZoomIn), Some(ZoomOut), Some(ZoomReset),
        ]);
        self.menu(ui, "Go", &[Some(GoToLine), None, Some(NextEditor), Some(PrevEditor)]);
        ui.menu_button("Run", |ui| {
            ui.set_min_width(230.0);
            ui.add_enabled(false, egui::Button::new("Run File - arrives in milestone 3"));
        });
        ui.menu_button("Terminal", |ui| {
            ui.set_min_width(230.0);
            self.menu_item(ui, NewTerminal);
            self.menu_item(ui, SplitTerminal);
            self.menu_item(ui, KillTerminal);
            ui.separator();
            let names: Vec<String> = self.settings.terminals.iter().map(|p| p.name.clone()).collect();
            for name in names {
                if ui.button(format!("New {name}")).clicked() {
                    self.cmds.push(NewTerminalWith(name));
                    ui.close();
                }
            }
        });
        self.menu(ui, "Help", &[Some(About)]);
    }

    fn menu(&mut self, ui: &mut Ui, title: &str, items: &[Option<Command>]) {
        ui.menu_button(title, |ui| {
            ui.set_min_width(230.0);
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
