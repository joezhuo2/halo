use crate::accounts::Accounts;
use crate::ai::AiPanel;
use crate::command::{self, Command, Scope};
use crate::editor::Editors;
use crate::explorer::Explorer;
use crate::find::FindPanel;
use crate::git::Git;
use crate::lsp;
use crate::scm::Scm;
use crate::search::Search;
use crate::settings::{Session, Settings};
use crate::terminal::BottomPanel;
use crate::theme::Theme;
use crate::wsl::{self, Wsl};
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Panel, Response, RichText, ScrollArea,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Editor,
    Terminal,
    Explorer,
    Ai,
}

pub struct Halo {
    pub theme: Theme,
    pub settings: Settings,
    pub session: Session,
    pub git: Git,
    pub explorer: Explorer,
    pub scm: Scm,
    pub accounts: Accounts,
    pub editors: Editors,
    pub bottom: BottomPanel,
    pub ai: AiPanel,
    pub search: Search,
    pub find: FindPanel,
    pub wsl: Wsl,
    notices: Vec<String>,
    pub page: crate::settings_page::SettingsPage,
    font: String,
    pub zone: Zone,
    pub cmds: Vec<Command>,
    about: bool,
    confirm_exit: bool,
    exiting: bool,
    focused: bool,
    pub chord: Option<egui::KeyboardShortcut>,
    theme_fade: Option<(f64, egui::Color32)>,
    icon_dark: Option<bool>,
    update: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    pub updated: Option<String>,
}

impl Halo {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let settings = Settings::load();
        ctx.set_fonts(crate::theme::fonts(&settings.font_family));
        crate::editor::configure(&settings);
        let theme = Theme::of(&settings);
        theme.apply(ctx);

        let session = Session::load();
        let font = settings.font_family.clone();
        let mut app = Self {
            theme,
            settings,
            session,
            git: Git::new(),
            explorer: Explorer::new(),
            scm: Scm::new(),
            accounts: Accounts::default(),
            editors: Editors::new(),
            bottom: BottomPanel::new(),
            ai: AiPanel::default(),
            search: Search::default(),
            find: FindPanel::default(),
            wsl: Wsl::default(),
            notices: vec![],
            page: Default::default(),
            font,
            zone: Zone::Editor,
            cmds: vec![],
            about: false,
            confirm_exit: false,
            exiting: false,
            focused: true,
            chord: None,
            theme_fade: None,
            icon_dark: None,
            update: None,
            updated: None,
        };
        if app.settings.auto_update && !cfg!(debug_assertions) && std::env::var_os("HALO_COMMANDS").is_none() {
            app.update = Some(crate::update::start(ctx));
        }

        let arg = std::env::args().nth(1).map(PathBuf::from).and_then(|p| p.canonicalize().or_else(|_| std::path::absolute(&p)).ok()).map(strip_unc);
        let (folder, files) = match arg {
            Some(path) if path.is_dir() => (Some(path), vec![]),
            Some(path) => (app.session.folder.clone().filter(|f| f.is_dir()), vec![path]),
            None => (app.session.folder.clone().filter(|f| f.is_dir()), app.session.files.clone()),
        };
        let startup = std::env::var("HALO_COMMANDS").unwrap_or_default();
        app.cmds = command::all().into_iter().filter(|c| startup.split(',').any(|id| id == c.id())).collect();
        app.set_folder(folder, ctx);
        for file in files.iter().filter(|f| f.is_file()) {
            app.editors.open_kept(file);
        }
        app.editors.restore(&app.session.backups);
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
        self.editors.set_root(folder.clone());
        self.git.set_root(folder.clone(), ctx);
        self.session.folder = folder;
        self.session.save();
    }

    pub fn folder_name(&self) -> Option<String> {
        let folder = self.session.folder.as_ref()?;
        Some(folder.file_name().unwrap_or(folder.as_os_str()).to_string_lossy().into_owned())
    }

    fn save_session(&mut self) {
        let backups = crate::settings::dir().join("backups");
        self.session.files = self.editors.open_paths();
        self.session.backups = if self.settings.hot_exit { self.editors.backup(&backups) } else { vec![] };
        if !self.settings.hot_exit {
            let _ = std::fs::remove_dir_all(backups);
        }
        self.session.save();
    }

    fn reload_settings(&mut self, ctx: &egui::Context) {
        self.settings = Settings::load();
        self.apply_settings(ctx);
    }

    pub(crate) fn apply_settings(&mut self, ctx: &egui::Context) {
        self.theme = Theme::of(&self.settings);
        self.theme.apply(ctx);
        if self.font != self.settings.font_family {
            self.font = self.settings.font_family.clone();
            ctx.set_fonts(crate::theme::fonts(&self.font));
        }
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
            OpenFolderIn(start) => {
                if let Some(folder) = rfd::FileDialog::new().set_directory(start).pick_folder() {
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
            OpenSettings => self.page.open = true,
            OpenSnippets => {
                crate::editor::write_sample_snippets();
                self.editors.open(&crate::editor::snippets_path());
            }
            OpenKeybindings => {
                if !command::path().exists() {
                    command::write_defaults();
                }
                self.editors.open(&command::path());
            }
            RunFile | RunTask(_) => {
                let line = match &cmd {
                    RunTask(name) => {
                        let task = self.settings.tasks.iter().find(|t| &t.name == name);
                        task.map(|t| t.command.clone()).ok_or_else(|| format!("There is no task named {name} in settings.json."))
                    }
                    _ => {
                        self.dispatch(Save, ctx);
                        match self.editors.active_path() {
                            Some(path) => {
                                let in_wsl = cwd.as_deref().and_then(wsl::parse).is_some();
                                let shown = if in_wsl { wsl::shell_path(&path) } else { path.to_string_lossy().into_owned() };
                                self.settings.run_command(&path, &shown).ok_or_else(|| {
                                    format!("No run command is set for {}. Add one under \"run\" in settings.json.", path.display())
                                })
                            }
                            None => Err("Open a saved file to run it.".to_owned()),
                        }
                    }
                };
                match line {
                    Ok(line) => {
                        self.session.panel = true;
                        self.bottom.run(&line, &self.settings, cwd.as_deref(), ctx);
                    }
                    Err(error) => ui::notify(error),
                }
            }
            OpenSettingsJson => {
                let path = Settings::path();
                if !path.exists() {
                    self.settings.save();
                }
                self.editors.open(&path);
            }
            Exit => ctx.send_viewport_cmd(ViewportCommand::Close),
            ShowExplorer | ShowSourceControl | FindInFiles => {
                let shown = (self.session.search, self.session.source_control && !self.session.search);
                let wanted = (cmd == FindInFiles, cmd == ShowSourceControl);
                if self.session.sidebar && shown == wanted && cmd != FindInFiles {
                    self.session.sidebar = false;
                } else {
                    self.session.sidebar = true;
                    (self.session.search, self.session.source_control) = wanted;
                }
                if cmd == FindInFiles {
                    self.find.focus();
                }
            }
            ToggleSidebar => self.session.sidebar ^= true,
            TogglePanel => self.session.panel ^= true,
            ToggleAi => self.session.ai ^= true,
            ToggleMinimap => {
                self.settings.minimap ^= true;
                self.settings.save();
            }
            ToggleWordWrap => {
                self.settings.word_wrap ^= true;
                self.settings.save();
            }
            ToggleTheme => {
                self.theme_fade = Some((ctx.input(|i| i.time), self.theme.bg));
                self.settings.theme = if self.theme.dark { "light" } else { "dark" }.into();
                self.settings.save();
                self.apply_settings(ctx);
            }
            ZoomIn => ctx.set_zoom_factor((ctx.zoom_factor() + 0.1).min(3.0)),
            ZoomOut => ctx.set_zoom_factor((ctx.zoom_factor() - 0.1).max(0.5)),
            ZoomReset => ctx.set_zoom_factor(1.0),
            About => self.about = true,
            GoToFile => self.search.open(""),
            ShowCommands => self.search.open(">"),
            GoToSymbol => self.search.open("@"),
            GoToWorkspaceSymbol => self.search.open("#"),
            RestartLanguageServers => self.editors.lsp.restart(),
            ShowProblems => {
                self.session.panel = !(self.session.panel && self.bottom.showing_problems());
                self.bottom.show_problems();
            }
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
                self.after_save(ctx);
            }
            other => {
                let _ = match self.zone {
                    Zone::Ai => {
                        self.ai.exec(&other, ctx)
                            || self.editors.exec(&other, &self.settings, ctx)
                            || self.bottom.exec(&other, &self.settings, cwd.as_deref(), ctx)
                    }
                    Zone::Terminal => {
                        self.bottom.exec(&other, &self.settings, cwd.as_deref(), ctx) || self.editors.exec(&other, &self.settings, ctx)
                    }
                    _ => self.editors.exec(&other, &self.settings, ctx) || self.bottom.exec(&other, &self.settings, cwd.as_deref(), ctx),
                };
            }
        }
    }

    fn after_save(&mut self, ctx: &egui::Context) {
        self.git.refresh(ctx);
        self.explorer.refresh();
        let active = self.editors.active_path();
        if active.as_ref().is_some_and(|p| *p == Settings::path()) {
            self.reload_settings(ctx);
        }
        if active.as_ref().is_some_and(|p| *p == command::path()) {
            command::reload();
        }
        if active.is_some_and(|p| p == crate::editor::snippets_path()) {
            crate::editor::reload_snippets();
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let bindings = command::bindings();
        let editor = self.zone == Zone::Editor;
        ctx.input_mut(|i| {
            let Some(first) = self.chord else {
                for binding in bindings.iter() {
                    let chord = binding.chord.is_some();
                    if (binding.scope == Scope::Global || (chord && editor)) && i.consume_shortcut(&binding.shortcut) {
                        match chord {
                            true => self.chord = Some(binding.shortcut),
                            false => self.cmds.push(binding.command.clone()),
                        }
                    }
                }
                return;
            };
            let second = bindings.iter().filter(|b| b.shortcut == first).find(|b| b.chord.is_some_and(|c| i.consume_shortcut(&c)));
            if let Some(binding) = second {
                self.cmds.push(binding.command.clone());
            } else if !i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })) {
                return;
            }
            i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
            self.chord = None;
        });
    }

    fn handle_window_events(&mut self, ctx: &egui::Context) {
        let (close_requested, focused) = ctx.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.focused.unwrap_or(true))
        });
        if close_requested && self.settings.hot_exit {
            self.save_session();
        } else if close_requested && !self.exiting && self.editors.any_dirty() {
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
                let (search, source) = (self.session.search, self.session.source_control && !self.session.search);
                let explorer = self.session.sidebar && !search && !source;
                let source = self.session.sidebar && source;
                if activity_item(ui, &theme, icons::FILES, explorer, "Explorer (Ctrl+Shift+E)").clicked() {
                    self.cmds.push(Command::ShowExplorer);
                }
                if activity_item(ui, &theme, icons::MAGNIFYING_GLASS, self.session.sidebar && search, "Search (Ctrl+Shift+F)").clicked() {
                    self.cmds.push(Command::FindInFiles);
                }
                if activity_item(ui, &theme, icons::GIT_BRANCH, source, "Source Control (Ctrl+Shift+G)").clicked() {
                    self.cmds.push(Command::ShowSourceControl);
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if activity_item(ui, &theme, icons::GEAR, false, "Settings (Ctrl+,)").clicked() {
                        self.cmds.push(Command::OpenSettings);
                    }
                    let accounts = activity_item(ui, &theme, icons::USER_CIRCLE, false, "Accounts");
                    egui::Popup::menu(&accounts).show(|ui| self.accounts.menu(ui));
                    if cfg!(windows) {
                        let folder = self.session.folder.clone();
                        let in_wsl = folder.as_deref().and_then(wsl::parse).is_some();
                        let button = activity_item(ui, &theme, icons::LINUX_LOGO, in_wsl, "Switch to WSL");
                        egui::Popup::menu(&button).show(|ui| self.wsl.menu(ui, folder.as_deref(), &mut self.cmds));
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
                if self.session.search {
                    let folder = self.session.folder.clone();
                    return self.find.ui(ui, &self.theme, folder.as_deref(), &mut self.editors, &mut self.cmds);
                }
                if self.session.source_control {
                    return self.scm.ui(ui, &self.theme, &mut self.git, &mut self.cmds);
                }
                let full = ui.available_rect_before_wrap();
                let symbols = self.editors.outline();
                let height = if self.session.outline && symbols.is_some() { (full.height() * 0.4).max(24.0) } else { 24.0 };
                let split = full.bottom() - height;
                ui.scope_builder(UiBuilder::new().max_rect(full.with_max_y(split)), |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    self.explorer.ui(ui, &self.theme, &self.git, active.as_deref(), &mut self.cmds);
                });
                ui.scope_builder(UiBuilder::new().max_rect(full.with_min_y(split)), |ui| {
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    outline(ui, &self.theme, symbols, self.editors.outline_loading(), &mut self.session.outline, &mut self.cmds);
                });
            });
        self.session.sidebar = open;
    }

    fn ai_panel(&mut self, ui: &mut Ui) {
        let mut open = self.session.ai;
        let cwd = self.session.folder.clone();
        Panel::right("ai_panel")
            .resizable(true)
            .default_size(380.0)
            .size_range(240.0..=800.0)
            .frame(Frame::new().fill(self.theme.panel))
            .show_collapsible(ui, &mut open, |ui| {
                self.ai.ui(ui, &self.theme, self.settings.font_size, cwd.as_deref(), &mut self.cmds);
            })
            .is_none()
            .then(|| self.ai.blur());
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
                self.bottom.ui(ui, &self.theme, &self.settings, cwd.as_deref(), &self.editors.lsp, &mut self.cmds);
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
                    self.editors.ai_open = self.session.ai;
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
            ui::logo(ui, 64.0, theme.dark);
            ui.add_space(6.0);
            ui.label(RichText::new("Halo").size(30.0).color(theme.text));
            ui.label(RichText::new("ultra-lightweight editor").color(theme.text_dim));
            ui.add_space(22.0);
            for cmd in [Command::OpenFolder, Command::OpenFile, Command::NewFile] {
                let shortcut = command::shortcut_text(ui.ctx(), &cmd).unwrap_or_default();
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

    fn notices(&mut self, ctx: &egui::Context) {
        self.notices.extend(ui::take_notices());
        if self.notices.is_empty() {
            return;
        }
        let theme = self.theme.clone();
        egui::Area::new(Id::new("notices")).order(egui::Order::Foreground).anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -34.0)).show(ctx, |ui| {
            ui.set_opacity(ui::fade_in(ui, Id::new(("notice_fade", self.notices.len()))));
            let mut dismiss = None;
            for (index, text) in self.notices.iter().enumerate() {
                Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.error)).corner_radius(CornerRadius::same(6)).inner_margin(12.0).show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.horizontal(|ui| {
                        if ui::icon_button(ui, &theme, icons::X, 20.0, false, "Dismiss").clicked() {
                            dismiss = Some(index);
                        }
                        ui.add(egui::Label::new(RichText::new(text).color(theme.text)).wrap());
                    });
                });
                ui.add_space(4.0);
            }
            if let Some(index) = dismiss {
                self.notices.remove(index);
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        self.notices(ctx);
        if self.page.open {
            self.settings_dialog(ctx);
        }
        if self.about {
            let modal = ui::modal(ctx, "about").show(ctx, |ui| {
                ui::outline(ui);
                ui.set_width(280.0);
                ui.vertical_centered(|ui| {
                    ui::logo(ui, 48.0, self.theme.dark);
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
            let modal = ui::modal(ctx, "confirm_exit").show(ctx, |ui| {
                ui::outline(ui);
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
        if self.accounts.poll() {
            self.git.refresh_prs();
        }
        for (title, text) in self.git.take_texts() {
            self.editors.open_text(&title, &text, Some(std::path::Path::new("changes.diff")));
        }
        if self.editors.take_saved() {
            self.after_save(&ctx);
        }
        if let Some(result) = self.update.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.update = None;
            match result {
                Ok(version) => self.updated = Some(version),
                Err(error) => ui::notify(format!("Halo could not update itself: {error}")),
            }
        }
        self.handle_window_events(&ctx);
        let dark = ctx.system_theme() == Some(egui::Theme::Dark);
        if self.icon_dark != Some(dark) {
            self.icon_dark = Some(dark);
            let rgba = ui::logo_image(128, dark).pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
            ctx.send_viewport_cmd(ViewportCommand::Icon(Some(std::sync::Arc::new(egui::IconData { rgba, width: 128, height: 128 }))));
        }
        if self.page.recording.is_none() {
            self.global_shortcuts(&ctx);
        }
        if self.settings.auto_save {
            if self.editors.auto_save_due(ctx.input(|i| i.time)) {
                self.editors.exec(&Command::SaveAll, &self.settings, &ctx);
                self.git.refresh(&ctx);
            } else if self.editors.any_dirty() {
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
        }

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

        if let Some((start, color)) = self.theme_fade {
            let t = ((ctx.input(|i| i.time) - start) / 0.3) as f32;
            if t < 1.0 {
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("theme_fade")));
                painter.rect_filled(window, CornerRadius::ZERO, ui::with_alpha(color, (1.0 - t).powi(2)));
                ctx.request_repaint();
            } else {
                self.theme_fade = None;
            }
        }

        if self.ai.has_focus() {
            self.zone = Zone::Ai;
        } else if self.bottom.has_focus() {
            self.zone = Zone::Terminal;
        } else if self.editors.has_focus() {
            self.zone = Zone::Editor;
        } else if self.explorer.has_focus() {
            self.zone = Zone::Explorer;
        } else if self.zone == Zone::Ai {
            self.zone = Zone::Editor;
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
    match path.to_str().map(|s| (s.strip_prefix(r"\\?\UNC\"), s.strip_prefix(r"\\?\"))) {
        Some((Some(share), _)) => PathBuf::from(format!(r"\\{share}")),
        Some((_, Some(rest))) => PathBuf::from(rest),
        _ => path,
    }
}

fn outline(ui: &mut Ui, theme: &Theme, symbols: Option<(&[lsp::Symbol], usize)>, loading: bool, open: &mut bool, cmds: &mut Vec<Command>) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, ui::mix(theme.panel, theme.bar, 0.5));
    painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, theme.border));
    let caret = if *open && symbols.is_some() { icons::CARET_DOWN } else { icons::CARET_RIGHT };
    painter.text(pos2(rect.left() + 12.0, rect.center().y), Align2::CENTER_CENTER, caret, FontId::proportional(10.0), theme.text);
    painter.text(pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, "OUTLINE", FontId::proportional(11.5), theme.text);
    if loading {
        ui::spinner(ui, pos2(rect.right() - 14.0, rect.center().y), 5.0, theme.text_dim);
    }
    if resp.on_hover_text("Symbols of the active file, from its language server").clicked() {
        *open ^= true;
    }
    let Some((symbols, line)) = symbols.filter(|_| *open) else { return };
    let current = symbols.iter().rposition(|s| s.line <= line && line <= s.end_line);
    ScrollArea::vertical().id_salt("outline").auto_shrink(false).show(ui, |ui| {
        for (index, symbol) in symbols.iter().enumerate() {
            let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            ui::row_bg(ui, theme, r, row.id, row.hovered(), current == Some(index));
            let x = r.left() + 12.0 + symbol.depth as f32 * 12.0;
            let painter = ui.painter().with_clip_rect(r);
            painter.text(pos2(x, r.center().y), Align2::LEFT_CENTER, lsp::symbol_icon(symbol.kind), FontId::proportional(13.0), theme.accent);
            painter.text(pos2(x + 20.0, r.center().y), Align2::LEFT_CENTER, &symbol.name, FontId::proportional(12.5), theme.text);
            if row.clicked() {
                cmds.push(Command::GoTo(symbol.line + 1, symbol.col + 1));
            }
        }
    });
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
