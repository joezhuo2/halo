use crate::command::{self, Command, Scope};
use crate::editor::Editors;
use crate::explorer::Explorer;
use crate::git::Git;
use crate::scm::Scm;
use crate::settings::{Session, Settings};
use crate::terminal::BottomPanel;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Panel, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, ViewportCommand, vec2,
};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Editor,
    Terminal,
    Explorer,
}

pub struct Halo {
    pub theme: Theme,
    pub settings: Settings,
    pub session: Session,
    pub git: Git,
    pub explorer: Explorer,
    pub scm: Scm,
    pub editors: Editors,
    pub bottom: BottomPanel,
    pub zone: Zone,
    pub cmds: Vec<Command>,
    about: bool,
    confirm_exit: bool,
    exiting: bool,
    focused: bool,
}

impl Halo {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);

        let settings = Settings::load();
        let theme = Theme::named(&settings.theme);
        theme.apply(ctx);

        let session = Session::load();
        let mut app = Self {
            theme,
            settings,
            session,
            git: Git::new(),
            explorer: Explorer::new(),
            scm: Scm::new(),
            editors: Editors::new(),
            bottom: BottomPanel::new(),
            zone: Zone::Editor,
            cmds: vec![],
            about: false,
            confirm_exit: false,
            exiting: false,
            focused: true,
        };

        let arg = std::env::args().nth(1).map(PathBuf::from).and_then(|p| p.canonicalize().ok()).map(strip_unc);
        let (folder, files) = match arg {
            Some(path) if path.is_dir() => (Some(path), vec![]),
            Some(path) => (app.session.folder.clone().filter(|f| f.is_dir()), vec![path]),
            None => (app.session.folder.clone().filter(|f| f.is_dir()), app.session.files.clone()),
        };
        app.set_folder(folder, ctx);
        for file in files.iter().filter(|f| f.is_file()) {
            app.editors.open(file);
        }
        app
    }

    fn set_folder(&mut self, folder: Option<PathBuf>, ctx: &egui::Context) {
        let folder = folder.map(strip_unc);
        if let Some(folder) = &folder {
            self.session.remember(folder);
        }
        let title = match &folder {
            Some(f) => format!("{} - Halo", f.file_name().unwrap_or(f.as_os_str()).to_string_lossy()),
            None => "Halo".to_owned(),
        };
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
        self.explorer.set_root(folder.clone());
        self.git.set_root(folder.clone(), ctx);
        self.session.folder = folder;
        self.session.save();
    }

    pub fn folder_name(&self) -> Option<String> {
        let folder = self.session.folder.as_ref()?;
        Some(folder.file_name().unwrap_or(folder.as_os_str()).to_string_lossy().into_owned())
    }

    fn save_session(&mut self) {
        self.session.files = self.editors.open_paths();
        self.session.save();
    }

    fn reload_settings(&mut self, ctx: &egui::Context) {
        self.settings = Settings::load();
        self.theme = Theme::named(&self.settings.theme);
        self.theme.apply(ctx);
    }

    fn dispatch(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        let cwd = self.session.folder.clone();
        match cmd {
            OpenFolder => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.set_folder(Some(folder), ctx);
                }
            }
            OpenFile => {
                if let Some(file) = rfd::FileDialog::new().pick_file() {
                    self.editors.open(&file);
                }
            }
            OpenPath(path) => {
                if path.is_dir() {
                    self.set_folder(Some(path), ctx);
                } else {
                    self.editors.open(&path);
                }
            }
            OpenDiff(title, args) => self.git.diff(title, args),
            CloseFolder => self.set_folder(None, ctx),
            OpenSettings => {
                let path = Settings::path();
                if !path.exists() {
                    self.settings.save();
                }
                self.editors.open(&path);
            }
            Exit => ctx.send_viewport_cmd(ViewportCommand::Close),
            ShowExplorer | ShowSourceControl => {
                let source_control = cmd == ShowSourceControl;
                if self.session.sidebar && self.session.source_control == source_control {
                    self.session.sidebar = false;
                } else {
                    self.session.sidebar = true;
                    self.session.source_control = source_control;
                }
            }
            ToggleSidebar => self.session.sidebar ^= true,
            TogglePanel => self.session.panel ^= true,
            ToggleAi => self.session.ai ^= true,
            ToggleMinimap => {
                self.settings.minimap ^= true;
                self.settings.save();
            }
            ToggleTheme => {
                self.settings.theme = if self.theme.dark { "light" } else { "dark" }.into();
                self.settings.save();
                self.theme = Theme::named(&self.settings.theme);
                self.theme.apply(ctx);
            }
            ZoomIn => ctx.set_zoom_factor((ctx.zoom_factor() + 0.1).min(3.0)),
            ZoomOut => ctx.set_zoom_factor((ctx.zoom_factor() - 0.1).max(0.5)),
            ZoomReset => ctx.set_zoom_factor(1.0),
            About => self.about = true,
            GitSync => self.git.sync(),
            RefreshExplorer => {
                self.explorer.refresh();
                self.git.refresh(ctx);
            }
            PathChanged(old, new) => {
                self.editors.path_changed(&old, new.as_deref());
                self.explorer.refresh();
                self.git.refresh(ctx);
            }
            NewTerminal | NewTerminalWith(_) | SplitTerminal => {
                self.session.panel = true;
                self.bottom.exec(&cmd, &self.settings, cwd.as_deref(), ctx);
            }
            Save | SaveAs | SaveAll => {
                self.editors.exec(&cmd, &self.settings, ctx);
                self.git.refresh(ctx);
                self.explorer.refresh();
                if self.editors.active_path().is_some_and(|p| p == Settings::path()) {
                    self.reload_settings(ctx);
                }
            }
            other => {
                if self.zone == Zone::Terminal {
                    let _ = self.bottom.exec(&other, &self.settings, cwd.as_deref(), ctx)
                        || self.editors.exec(&other, &self.settings, ctx);
                } else {
                    let _ = self.editors.exec(&other, &self.settings, ctx)
                        || self.bottom.exec(&other, &self.settings, cwd.as_deref(), ctx);
                }
            }
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            for binding in command::bindings() {
                if binding.scope == Scope::Global && i.consume_shortcut(&binding.shortcut) {
                    self.cmds.push(binding.command.clone());
                }
            }
        });
    }

    fn handle_window_events(&mut self, ctx: &egui::Context) {
        let (close_requested, focused) = ctx.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.focused.unwrap_or(true))
        });
        if close_requested && !self.exiting && self.editors.any_dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_exit = true;
        }
        if focused && !self.focused {
            self.cmds.push(Command::RefreshExplorer);
        }
        self.focused = focused;
    }

    fn activity_bar(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        Panel::left("activity_bar")
            .exact_size(46.0)
            .resizable(false)
            .frame(Frame::new().fill(theme.bar))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let explorer = self.session.sidebar && !self.session.source_control;
                let source = self.session.sidebar && self.session.source_control;
                if activity_item(ui, &theme, icons::FILES, explorer, "Explorer (Ctrl+Shift+E)").clicked() {
                    self.cmds.push(Command::ShowExplorer);
                }
                if activity_item(ui, &theme, icons::GIT_BRANCH, source, "Source Control (Ctrl+Shift+G)").clicked() {
                    self.cmds.push(Command::ShowSourceControl);
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if activity_item(ui, &theme, icons::GEAR, false, "Settings (Ctrl+,)").clicked() {
                        self.cmds.push(Command::OpenSettings);
                    }
                    activity_item(ui, &theme, icons::USER_CIRCLE, false, "Accounts - arrives in milestone 2");
                    if cfg!(windows) {
                        activity_item(ui, &theme, icons::LINUX_LOGO, false, "Switch to WSL - arrives in milestone 3");
                    }
                });
            });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let mut open = self.session.sidebar;
        let active = self.editors.active_path();
        Panel::left("sidebar")
            .resizable(true)
            .default_size(260.0)
            .size_range(170.0..=640.0)
            .frame(Frame::new().fill(self.theme.panel))
            .show_collapsible(ui, &mut open, |ui| {
                if self.session.source_control {
                    self.scm.ui(ui, &self.theme, &mut self.git, &mut self.cmds);
                } else {
                    self.explorer.ui(ui, &self.theme, &self.git, active.as_deref(), &mut self.cmds);
                }
            });
        self.session.sidebar = open;
    }

    fn ai_panel(&mut self, ui: &mut Ui) {
        let mut open = self.session.ai;
        Panel::right("ai_panel")
            .resizable(true)
            .default_size(380.0)
            .size_range(240.0..=800.0)
            .frame(Frame::new().fill(self.theme.panel))
            .show_collapsible(ui, &mut open, |ui| {
                ui::section_header(ui, &self.theme, "Claude  |  Codex");
                placeholder(ui, &self.theme, icons::ROBOT, "Claude and Codex sessions arrive in milestone 3.");
            });
        self.session.ai = open;
    }

    fn bottom_panel(&mut self, ui: &mut Ui) {
        let mut open = self.session.panel;
        let cwd = self.session.folder.clone();
        Panel::bottom("bottom_panel")
            .resizable(true)
            .default_size(240.0)
            .size_range(90.0..=900.0)
            .frame(Frame::new().fill(self.theme.panel))
            .show_collapsible(ui, &mut open, |ui| {
                self.bottom.ui(ui, &self.theme, &self.settings, cwd.as_deref(), &mut self.cmds);
            })
            .is_none()
            .then(|| self.bottom.blur());
        self.session.panel = open;
    }

    fn central(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(self.theme.bg))
            .show(ui, |ui| {
                if self.editors.is_empty() {
                    self.welcome(ui);
                } else {
                    self.editors.ui(ui, &self.theme, &self.settings, &self.git, &mut self.cmds);
                }
            });
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let theme = &self.theme;
        let t = ui::fade_in(ui, Id::new("welcome_fade"));
        ui.set_opacity(t);
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.22).max(20.0));
            ui.label(RichText::new(icons::CIRCLE_NOTCH).size(54.0).color(theme.accent));
            ui.add_space(6.0);
            ui.label(RichText::new("Halo").size(30.0).color(theme.text));
            ui.label(RichText::new("ultra-lightweight editor").color(theme.text_dim));
            ui.add_space(22.0);
            for cmd in [Command::OpenFolder, Command::OpenFile, Command::NewFile] {
                let shortcut = command::shortcut(&cmd).map(|s| ui.ctx().format_shortcut(&s)).unwrap_or_default();
                let button = egui::Button::new(RichText::new(cmd.label()).color(theme.accent))
                    .shortcut_text(shortcut)
                    .min_size(vec2(260.0, 28.0));
                if ui.add(button).clicked() {
                    self.cmds.push(cmd);
                }
                ui.add_space(2.0);
            }
            if !self.session.recent.is_empty() {
                ui.add_space(18.0);
                ui.label(RichText::new("RECENT").size(11.0).color(theme.text_dim));
                ui.add_space(4.0);
                for folder in self.session.recent.iter().take(6) {
                    let name = folder.file_name().unwrap_or(folder.as_os_str()).to_string_lossy();
                    let resp = ui
                        .add(egui::Button::new(RichText::new(name).color(theme.text)).frame(false))
                        .on_hover_text(folder.to_string_lossy());
                    if resp.clicked() {
                        self.cmds.push(Command::OpenPath(folder.clone()));
                    }
                }
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.about {
            let modal = egui::Modal::new(Id::new("about")).show(ctx, |ui| {
                ui.set_width(280.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icons::CIRCLE_NOTCH).size(40.0).color(self.theme.accent));
                    ui.heading("Halo");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.colored_label(self.theme.text_dim, "An ultra-lightweight native code editor.");
                    ui.add_space(8.0);
                    ui.button("Close").clicked()
                })
                .inner
            });
            if modal.inner || modal.should_close() {
                self.about = false;
            }
        }
        if self.confirm_exit {
            let mut choice = None;
            let modal = egui::Modal::new(Id::new("confirm_exit")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.heading("Unsaved changes");
                ui.label("Some files have unsaved changes. Save them before closing?");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save All").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_exit = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_exit = false;
            }
            if let Some(save) = choice {
                self.confirm_exit = false;
                if save {
                    self.editors.exec(&Command::SaveAll, &self.settings, ctx);
                }
                if !save {
                    self.editors.discard_all();
                }
                if !self.editors.any_dirty() {
                    self.exiting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }
}

impl eframe::App for Halo {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let window = ui.max_rect();
        if self.git.poll() {
            self.explorer.refresh();
            self.editors.reload_clean();
        }
        for (title, text) in self.git.take_texts() {
            self.editors.open_text(&title, &text);
        }
        self.handle_window_events(&ctx);
        self.global_shortcuts(&ctx);

        self.titlebar(ui);
        self.statusbar(ui);
        self.activity_bar(ui);
        self.sidebar(ui);
        self.ai_panel(ui);
        self.bottom_panel(ui);
        self.central(ui);
        self.dialogs(&ctx);

        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false));
        if !maximized {
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("window_border"))).rect_stroke(
                window,
                CornerRadius::ZERO,
                Stroke::new(1.0, self.theme.border),
                StrokeKind::Inside,
            );
            resize_edges(&ctx, window);
        }

        if self.bottom.has_focus() {
            self.zone = Zone::Terminal;
        } else if self.editors.has_focus() {
            self.zone = Zone::Editor;
        } else if self.explorer.has_focus() {
            self.zone = Zone::Explorer;
        }

        for cmd in std::mem::take(&mut self.cmds) {
            self.dispatch(cmd, &ctx);
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_session();
    }
}

fn strip_unc(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

fn activity_item(ui: &mut Ui, theme: &Theme, icon: &str, active: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(46.0, 44.0), Sense::click());
    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
    let a = ui::anim(ui, resp.id.with("active"), active);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if a > 0.0 {
            let mut bar = rect;
            bar.set_width(2.0);
            painter.rect_filled(bar.shrink2(vec2(0.0, 8.0 * (1.0 - a))), CornerRadius::ZERO, ui::with_alpha(theme.accent, a));
        }
        let color = ui::mix(theme.text_dim, theme.text, h.max(a));
        painter.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(23.0), color);
    }
    resp.on_hover_text(tip)
}

fn placeholder(ui: &mut Ui, theme: &Theme, icon: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(28.0);
        ui.label(RichText::new(icon).size(30.0).color(theme.text_dim));
        ui.add_space(4.0);
        ui.label(RichText::new(text).color(theme.text_dim));
    });
}

fn resize_edges(ctx: &egui::Context, window: egui::Rect) {
    use egui::viewport::ResizeDirection as D;
    let Some(pos) = ctx.pointer_hover_pos() else { return };
    let margin = 5.0;
    let left = pos.x <= window.left() + margin;
    let right = pos.x >= window.right() - margin;
    let top = pos.y <= window.top() + margin;
    let bottom = pos.y >= window.bottom() - margin;
    let (direction, cursor) = match (left, right, top, bottom) {
        (true, _, true, _) => (D::NorthWest, CursorIcon::ResizeNorthWest),
        (_, true, true, _) => (D::NorthEast, CursorIcon::ResizeNorthEast),
        (true, _, _, true) => (D::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (D::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (D::West, CursorIcon::ResizeWest),
        (_, true, ..) => (D::East, CursorIcon::ResizeEast),
        (_, _, true, _) => (D::North, CursorIcon::ResizeNorth),
        (_, _, _, true) => (D::South, CursorIcon::ResizeSouth),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}
