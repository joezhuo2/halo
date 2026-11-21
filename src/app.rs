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
use crate::timeline::Timeline;
use crate::workbench::View;
use crate::wsl::{self, Wsl};
use crate::ui;
use eframe::egui::{
    self, Align2, CornerRadius, CursorIcon, FontId, Frame, Id, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, ViewportCommand,
    pos2, vec2,
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
    pub(crate) timeline: Timeline,
    pub compare: crate::history::Compare,
    pub accounts: Accounts,
    pub editors: Editors,
    pub bottom: BottomPanel,
    pub tasks: crate::tasks::Tasks,
    pub ai: AiPanel,
    pub search: Search,
    pub find: FindPanel,
    pub wsl: Wsl,
    pub jobs: crate::theme_import::Jobs,
    pub page: crate::settings_page::SettingsPage,
    font: String,
    pub zone: Zone,
    pub cmds: Vec<Command>,
    about: bool,
    confirm_exit: bool,
    exiting: bool,
    focused: bool,
    pub chord: Option<egui::KeyboardShortcut>,
    auto_save_focus: (bool, bool),
    zoom_level: Option<f32>,
    theme_fade: Option<(f64, egui::Color32)>,
    icon_dark: Option<bool>,
    update: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    pub updated: Option<String>,
    pub debug: crate::debug::Debug,
    pub testing: crate::testing::Testing,
    pub(crate) dragging: Option<crate::workbench::View>,
    pub(crate) seen_panel: (crate::terminal::PanelTab, bool),
    last_escape: f64,
    fullscreen: Option<bool>,
}

impl Halo {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let mut app = Self::build(ctx, Settings::load(), Session::load());
        if app.session.layout.zen.is_some() && !app.settings.zen_restore {
            app.toggle_zen();
        }
        if app.settings.auto_update && !cfg!(debug_assertions) && std::env::var_os("HALO_COMMANDS").is_none() {
            app.update = Some(crate::update::start(ctx));
        }

        let arg = std::env::args().nth(1).map(PathBuf::from).and_then(|p| p.canonicalize().or_else(|_| std::path::absolute(&p)).ok()).map(strip_unc);
        let restore = app.settings.restore_session;
        let last = app.session.folder.clone().filter(|f| restore && f.is_dir());
        let arg_is_none = arg.is_none();
        let (folder, files) = match arg {
            Some(path) if path.is_dir() => (Some(path), vec![]),
            Some(path) => (last, vec![path]),
            None => (last, if restore { app.session.files.clone() } else { vec![] }),
        };
        let startup = std::env::var("HALO_COMMANDS").unwrap_or_default();
        app.cmds = command::all().into_iter().filter(|c| startup.split(',').any(|id| id == c.id())).collect();
        app.set_folder(folder, ctx);
        if arg_is_none && restore {
            let pins: Vec<_> = app.session.pinned.iter().filter(|(_, f)| f.is_file()).cloned().collect();
            app.editors.restore_pins(&pins);
        }
        for file in files.iter().filter(|f| f.is_file()) {
            if !app.editors.open_paths().contains(file) {
                app.editors.open_kept(file);
            }
        }
        app.editors.restore(&app.session.backups);
        app
    }

    pub(crate) fn build(ctx: &egui::Context, settings: Settings, session: Session) -> Self {
        ctx.set_fonts(crate::theme::fonts(&settings.font_family, &settings.terminal_font_family));
        crate::terminal::configure(&settings);
        crate::editor::configure(&settings);
        let theme = Theme::of(&settings);
        theme.apply(ctx);
        crate::icon_theme::select(&settings.icon_theme, &crate::settings::dir());

        let font = format!("{}\n{}", settings.font_family, settings.terminal_font_family);
        let mut app = Self {
            theme,
            settings,
            session,
            git: Git::new(),
            explorer: Explorer::new(),
            scm: Scm::new(),
            timeline: Timeline::default(),
            compare: Default::default(),
            accounts: Accounts::default(),
            editors: Editors::new(),
            bottom: BottomPanel::new(),
            tasks: Default::default(),
            ai: AiPanel::default(),
            search: Search::default(),
            find: FindPanel::default(),
            wsl: Wsl::default(),
            jobs: Default::default(),
            page: Default::default(),
            font,
            zone: Zone::Editor,
            cmds: vec![],
            about: false,
            confirm_exit: false,
            exiting: false,
            focused: true,
            chord: None,
            auto_save_focus: (false, true),
            zoom_level: None,
            theme_fade: None,
            icon_dark: None,
            update: None,
            updated: None,
            debug: Default::default(),
            testing: Default::default(),
            dragging: None,
            seen_panel: (crate::terminal::PanelTab::Terminal, false),
            last_escape: f64::NEG_INFINITY,
            fullscreen: None,
        };
        app.editors.config = app.settings.clone();
        app
    }

    fn set_folder(&mut self, folder: Option<PathBuf>, ctx: &egui::Context) {
        let folder = folder.map(strip_unc);
        crate::debug::store(&mut self.session, &self.editors.marks);
        if let Some(folder) = &folder {
            self.session.remember(folder);
        }
        let title = match &folder {
            Some(f) => format!("{} - Halo", f.file_name().unwrap_or(f.as_os_str()).to_string_lossy()),
            None => "Halo".to_owned(),
        };
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
        crate::layers::set_folder(folder.as_deref());
        self.reload_settings(ctx);
        self.explorer.set_root(folder.clone());
        self.editors.set_root(folder.clone());
        self.git.set_root(folder.clone(), ctx);
        self.session.folder = folder;
        crate::debug::restore(&self.session, &mut self.editors.marks);
        self.session.save();
    }

    pub fn folder_name(&self) -> Option<String> {
        let folder = self.session.folder.as_ref()?;
        Some(folder.file_name().unwrap_or(folder.as_os_str()).to_string_lossy().into_owned())
    }

    fn save_session(&mut self) {
        let backups = crate::settings::dir().join("backups");
        self.session.files = self.editors.open_paths();
        self.session.pinned = self.editors.pinned_paths();
        self.session.backups = if self.settings.hot_exit { self.editors.backup(&backups) } else { vec![] };
        if !self.settings.hot_exit {
            let _ = std::fs::remove_dir_all(backups);
        }
        crate::debug::store(&mut self.session, &self.editors.marks);
        self.session.save();
    }

    fn reload_settings(&mut self, ctx: &egui::Context) {
        self.settings = Settings::load();
        self.apply_settings(ctx);
    }

    pub(crate) fn apply_settings(&mut self, ctx: &egui::Context) {
        if self.zoom_level != Some(self.settings.zoom_level) {
            self.zoom_level = Some(self.settings.zoom_level);
            ctx.set_zoom_factor(1.2f32.powf(self.settings.zoom_level).clamp(0.5, 3.0));
        }
        self.theme = Theme::of(&self.settings);
        self.theme.apply(ctx);
        crate::icon_theme::select(&self.settings.icon_theme, &crate::settings::dir());
        crate::terminal::configure(&self.settings);
        let font = format!("{}
{}", self.settings.font_family, self.settings.terminal_font_family);
        if self.font != font {
            self.font = font;
            ctx.set_fonts(crate::theme::fonts(&self.settings.font_family, &self.settings.terminal_font_family));
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
            OpenDiff(title, args) => {
                let inline = !self.settings.diff_side_by_side;
                if !self.git.top().is_some_and(|top| self.editors.compare(&title, &args, top, inline, ctx)) {
                    self.git.diff(title, args);
                }
            }
            OpenMerge(path) => self.editors.open_merge(&path, ctx),
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
            RunTaskPicker | RunBuildTask | RunTestTask | RerunLastTask | TerminateTask | ShowRunningTasks | ConfigureDefaultBuildTask
            | OpenTasksJson | RunTaskLabel(_) | TerminateTaskLabel(_) | RevealTask(_) | SetDefaultBuild(_) => self.task_command(cmd, ctx),
            OpenSettingsJson => {
                let path = Settings::path();
                if !path.exists() {
                    self.settings.save();
                }
                self.editors.open(&path);
            }
            Exit => ctx.send_viewport_cmd(ViewportCommand::Close),
            ShowExplorer => self.toggle_view(View::Explorer),
            ShowSourceControl => self.toggle_view(View::SourceControl),
            ShowRunAndDebug => self.toggle_view(View::RunAndDebug),
            FindInFiles => {
                self.reveal(View::Search);
                self.find.focus();
            }
            ShowTesting => self.reveal(View::Testing),
            RunAllTests | RunFailedTests | RunTestAtCursor | DebugTestAtCursor | RerunLastTestRun | CancelTestRun | RefreshTests | ShowTestOutput
            | RunAllTestsWithCoverage | ToggleCoverageDisplay => self.testing_exec(&cmd, ctx),
            StartDebugging | RunWithoutDebugging | StopDebugging | RestartDebugging | PauseDebugging | StepOver | StepInto | StepOut
            | RemoveAllBreakpoints | AddWatch | AddConfiguration => {
                if cmd == AddWatch {
                    self.reveal(View::RunAndDebug);
                }
                if !self.debug.active() && matches!(cmd, StartDebugging | RunWithoutDebugging) {
                    self.editors.exec(&SaveAll, &self.settings, ctx);
                }
                self.debug_exec(&cmd, ctx);
            }
            ToggleSidebar => self.session.sidebar ^= true,
            TogglePanel => self.session.panel ^= true,
            ToggleAi => self.toggle_view(View::Ai),
            ToggleFullScreen | ToggleZenMode | ToggleCenteredLayout | ToggleSidebarPosition | ToggleSecondarySidebar | MovePanelLeft
            | MovePanelRight | MovePanelBottom | MovePanelTop | TogglePanelPosition | ToggleMaximizedPanel | AlignPanelCenter
            | AlignPanelJustify | AlignPanelLeft | AlignPanelRight | ActivityBarSide | ActivityBarTop | ActivityBarHidden | MoveView
            | MoveViewTo(_) | MoveViewPlace(..) | ResetViewLocations => self.layout_command(cmd),
            ToggleMinimap => {
                self.settings.minimap ^= true;
                self.settings.save();
            }
            ToggleWordWrap => {
                self.settings.word_wrap = if self.settings.word_wrap == "off" { "on" } else { "off" }.into();
                self.settings.save();
            }
            ToggleTheme => {
                self.theme_fade = Some((ctx.input(|i| i.time), self.theme.bg));
                self.settings.theme = if self.theme.dark { "light" } else { "dark" }.into();
                self.settings.save();
                self.apply_settings(ctx);
            }
            SetTheme(name) => {
                self.theme_fade = Some((ctx.input(|i| i.time), self.theme.bg));
                self.settings.theme = name;
                self.settings.save();
                self.apply_settings(ctx);
            }
            ImportTheme | BrowseThemes | SelectTheme | ImportThemeFile(_) | ImportIconTheme | BrowseIconThemes | SelectIconTheme
            | ImportIconThemeFile(_) | SetIconTheme(_) => self.theme_command(cmd, ctx),
            ZoomIn | ZoomOut | ZoomReset if self.zone == Zone::Editor && self.editors.image_zoom(match cmd { ZoomIn => Some(1.25), ZoomOut => Some(0.8), _ => None }) => {}
            ZoomIn => ctx.set_zoom_factor((ctx.zoom_factor() + 0.1).min(3.0)),
            ZoomOut => ctx.set_zoom_factor((ctx.zoom_factor() - 0.1).max(0.5)),
            ZoomReset => ctx.set_zoom_factor(1.0),
            About => self.about = true,
            GoToFile => self.search.open(""),
            ShowCommands => self.search.open(">"),
            GoToSymbol => self.search.open("@"),
            GoToWorkspaceSymbol => self.search.open("#"),
            RestartLanguageServers => self.editors.lsp.restart(),
            ShowProblems => self.toggle_view(View::Problems),
            ShowOutput | SelectOutputChannel(_) => {
                if let SelectOutputChannel(name) = cmd {
                    crate::output::select(name);
                }
                self.reveal(View::Output);
            }
            ShowOutputChannels => self.search.pick("Select an Output Channel", crate::output::picks()),
            ClearOutput => crate::output::clear_current(),
            OpenOutputInEditor => self.editors.open_untitled(&crate::output::current_text()),
            ShowNotifications | ClearNotifications | ToggleDoNotDisturb | FocusNotification => crate::notifications::exec(&cmd),
            Git(op) | GitAt(op, _) => {
                let target = if let GitAt(_, value) = cmd { Some(value) } else { None };
                self.reveal(View::SourceControl);
                self.scm.start(op, target, &mut self.git, ctx, &mut self.cmds);
            }
            GitSync => self.git.sync(),
            CutFiles | CopyFiles | PasteFiles => {
                self.explorer.exec(&cmd, ctx);
            }
            FilterExplorer | FocusOpenEditors => {
                self.reveal(View::Explorer);
                if cmd == FilterExplorer {
                    self.explorer.open_filter();
                } else {
                    self.explorer.focus_open_editors();
                }
            }
            ShowTab(group, tab) => self.editors.show_tab(group, tab),
            CloseTab(group, tab) => self.editors.close_open_tab(group, tab),
            CloseAllEditors => self.editors.close_all_tabs(),
            RefreshExplorer => {
                self.explorer.refresh();
                self.git.refresh(ctx);
            }
            PathChanged(old, new) => {
                self.editors.path_changed(&old, new.as_deref());
                self.explorer.refresh();
                self.git.refresh(ctx);
            }
            RunRecentLine => {
                let lines = match self.editors.active_terminal().filter(|_| self.zone == Zone::Editor) {
                    Some(terminal) => terminal.recent(),
                    None => self.bottom.recent(),
                };
                let items: Vec<_> = lines.into_iter().map(|line| (line.clone(), String::new(), RunCommandLine(line))).collect();
                match items.is_empty() {
                    true => ui::notify("No commands were seen in the active terminal yet. It needs shell integration."),
                    false => self.search.pick("Select a command to run", items),
                }
            }
            ChangeTerminalColor => {
                let items = crate::terminal::color_names().map(|(color, name)| (name.to_owned(), String::new(), SetTerminalColor(color)));
                self.search.pick("Select a color for the terminal", items.into());
            }
            NewTerminalInEditor => match self.bottom.create(&self.settings, cwd.as_deref(), ctx) {
                Ok(terminal) => self.editors.add_terminal(terminal),
                Err(error) => ui::notify(error),
            },
            MoveTerminalToEditor => match self.bottom.take_active() {
                Some(terminal) => self.editors.add_terminal(terminal),
                None => ui::notify("There is no terminal in the panel to move. Task terminals stay in the panel."),
            },
            MoveTerminalToPanel => match self.editors.take_terminal() {
                Some(terminal) => {
                    self.bottom.adopt(terminal);
                    self.session.panel = true;
                }
                None => ui::notify("The active editor is not a terminal."),
            },
            NewTerminal | NewTerminalWith(_) | SplitTerminal => {
                self.session.panel = true;
                self.bottom.exec(&cmd, &self.settings, cwd.as_deref(), ctx);
            }
            Save | SaveAs | SaveAll => {
                self.editors.exec(&cmd, &self.settings, ctx);
                self.after_save(ctx);
            }
            other if self.exec_compare(&other, ctx) => {}
            other => {
                let _ = match self.zone {
                    Zone::Explorer if self.explorer.exec(&other, ctx) => true,
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
        if active.as_ref().is_some_and(|p| *p == Settings::path() || Some(p) == crate::layers::workspace_path().as_ref()) {
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
        let editor = self.zone == Zone::Editor && !self.editors.terminal_focused();
        let keys = command::context();
        ctx.input_mut(|i| {
            let Some(first) = self.chord else {
                for binding in bindings.iter() {
                    let chord = binding.chord.is_some();
                    if (binding.scope == Scope::Global || (chord && editor)) && binding.active(&keys) && i.consume_shortcut(&binding.shortcut) {
                        match chord {
                            true => self.chord = Some(binding.shortcut),
                            false => self.cmds.push(binding.command.clone()),
                        }
                    }
                }
                return;
            };
            let second = bindings.iter().filter(|b| b.shortcut == first && b.active(&keys)).find(|b| b.chord.is_some_and(|c| i.consume_shortcut(&c)));
            if let Some(binding) = second {
                self.cmds.push(binding.command.clone());
            } else if !i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })) {
                return;
            }
            i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
            self.chord = None;
        });
    }

    fn when_keys(&self, ctx: &egui::Context) -> serde_json::Map<String, serde_json::Value> {
        let terminal = self.bottom.has_focus() || self.ai.has_focus() || self.editors.terminal_focused();
        let typing = ctx.memory(|m| m.focused().is_some()) && !terminal;
        let debug = match (self.debug.active(), self.editors.marks.stopped.is_some()) {
            (false, _) => "inactive",
            (true, true) => "stopped",
            (true, false) => "running",
        };
        let explorer = self.explorer.has_focus();
        let side = self.session.sidebar && (explorer || self.zone == Zone::Explorer);
        let mut keys: serde_json::Map<String, serde_json::Value> = [
            ("editorTextFocus", (self.editors.has_focus() && !terminal).into()),
            ("editorFocus", (self.zone == Zone::Editor && !self.editors.is_empty() && !terminal).into()),
            ("terminalFocus", terminal.into()),
            ("inDebugMode", self.debug.active().into()),
            ("debugState", debug.into()),
            ("filesExplorerFocus", explorer.into()),
            ("listFocus", explorer.into()),
            ("sideBarVisible", self.session.sidebar.into()),
            ("sideBarFocus", side.into()),
            ("panelVisible", self.session.panel.into()),
            ("panelFocus", self.bottom.has_focus().into()),
            ("auxiliaryBarVisible", self.session.ai.into()),
            ("panelPosition", format!("{:?}", self.panel_at()).to_lowercase().into()),
            ("panelMaximized", self.session.layout.panel_max.into()),
            ("inZenMode", self.session.layout.zen.is_some().into()),
            ("isCenteredLayout", self.session.layout.centered.into()),
            ("isFullscreen", self.session.layout.fullscreen.into()),
            ("inputFocus", typing.into()),
            ("textInputFocus", typing.into()),
            ("inQuickOpen", self.search.is_open().into()),
            ("isWindows", cfg!(windows).into()),
            ("isLinux", cfg!(target_os = "linux").into()),
            ("isMac", cfg!(target_os = "macos").into()),
            ("config.terminal.integrated.sendKeybindingsToShell", self.settings.send_keybindings_to_shell.into()),
        ]
        .into_iter()
        .map(|(k, v): (&str, serde_json::Value)| (k.to_owned(), v))
        .collect();
        self.editors.when_keys(&mut keys);
        keys
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

    pub(crate) fn drop_files(&mut self, ui: &mut Ui, area: egui::Rect) {
        let ctx = ui.ctx().clone();
        let pointer = ctx.pointer_latest_pos();
        if self.explorer.dragging() && pointer.is_some_and(|p| area.contains(p)) {
            ui.painter().rect_stroke(area, 0, Stroke::new(1.0, self.theme.accent), StrokeKind::Inside);
        }
        let mut files = vec![];
        if let Some((paths, _)) = self.explorer.take_drop().filter(|(_, pos)| area.contains(*pos)) {
            files.extend(paths);
        }
        if pointer.is_none_or(|p| area.contains(p)) {
            let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
            files.extend(dropped.into_iter().map(|f| f.path().to_path_buf()));
        }
        for file in files.iter().filter(|f| f.is_file()) {
            self.editors.open_kept(file);
        }
    }

    pub(crate) fn welcome(&mut self, ui: &mut Ui) {
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

    fn dialogs(&mut self, ctx: &egui::Context) {
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
        self.frame(ui);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_session();
    }
}

impl Halo {
    pub(crate) fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let window = ui.max_rect();
        if self.git.poll() {
            self.explorer.refresh();
            self.editors.reload_clean();
            self.editors.refresh_blame();
            self.timeline.reload();
        }
        self.tasks.poll(&mut self.bottom, &ctx);
        if self.editors.history.changed() {
            self.timeline.reload();
        }
        self.poll_compare(&ctx);
        self.poll_imports();
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
        command::set_context(self.when_keys(&ctx));
        self.page.find_key(&ctx);
        if self.session.layout.zen.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            let now = ctx.input(|i| i.time);
            self.last_escape = if now - self.last_escape < 0.5 { f64::NEG_INFINITY } else { now };
            if self.last_escape == f64::NEG_INFINITY {
                self.cmds.push(Command::ToggleZenMode);
            }
        }
        if !self.page.keys.recording() {
            self.global_shortcuts(&ctx);
        }
        let delay = self.settings.auto_save_delay as f64 / 1000.0;
        let focus = (self.editors.has_focus() && self.focused, self.focused);
        let now = ctx.input(|i| i.time);
        let save = auto_save(&self.settings.auto_save, || self.editors.auto_save_due(now, delay), self.auto_save_focus, focus);
        self.auto_save_focus = focus;
        if save && self.editors.any_dirty() {
            self.editors.exec(&Command::SaveAll, &self.settings, &ctx);
            self.git.refresh(&ctx);
        } else if self.settings.auto_save == "afterDelay" && self.editors.any_dirty() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(delay.max(0.05)));
        }

        crate::notifications::show(&ctx, &self.theme, &mut self.cmds);
        self.titlebar(ui);
        if self.session.layout.zen.is_none() {
            self.statusbar(ui);
        }
        self.workbench(ui);
        self.debug_frame(&ctx, window.top() + 44.0);
        self.testing_frame(&ctx);
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
        if self.fullscreen != Some(self.session.layout.fullscreen) {
            self.fullscreen = Some(self.session.layout.fullscreen);
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.session.layout.fullscreen));
        }
    }
}

fn auto_save(mode: &str, due: impl FnOnce() -> bool, was: (bool, bool), now: (bool, bool)) -> bool {
    match mode {
        "afterDelay" => due(),
        "onFocusChange" => was.0 && !now.0,
        "onWindowChange" => was.1 && !now.1,
        _ => false,
    }
}

fn strip_unc(path: PathBuf) -> PathBuf {
    match path.to_str().map(|s| (s.strip_prefix(r"\\?\UNC\"), s.strip_prefix(r"\\?\"))) {
        Some((Some(share), _)) => PathBuf::from(format!(r"\\{share}")),
        Some((_, Some(rest))) => PathBuf::from(rest),
        _ => path,
    }
}

pub fn note(ui: &mut Ui, theme: &Theme, text: &str) {
    Frame::new().inner_margin(egui::Margin::symmetric(14, 5)).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme.text_dim)).truncate());
    });
}

pub(crate) fn outline(ui: &mut Ui, theme: &Theme, symbols: Option<(&[lsp::Symbol], usize)>, active: bool, loading: bool, open: &mut bool, cmds: &mut Vec<Command>) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let rect = ui::section(ui, theme, "OUTLINE", open).on_hover_text("Symbols of the active file, from its language server").rect;
    if loading {
        ui::spinner(ui, pos2(rect.right() - 14.0, rect.center().y), 5.0, theme.text_dim);
    }
    if ui.available_height() < 1.0 {
        return;
    }
    let Some((symbols, line)) = symbols else {
        return match (active, loading) {
            (false, _) => note(ui, theme, "No active editor."),
            (true, true) => ui::skeleton(ui, theme, 4),
            _ => note(ui, theme, "No symbols found in this file."),
        };
    };
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

#[cfg(test)]
mod tests {
    #[test]
    fn auto_save_follows_the_mode() {
        let save = |mode: &str, was, now| super::auto_save(mode, || true, was, now);
        assert!(save("afterDelay", (true, true), (true, true)));
        assert!(!super::auto_save("afterDelay", || false, (true, true), (true, true)));
        assert!(save("onFocusChange", (true, true), (false, true)) && save("onFocusChange", (true, true), (false, false)));
        assert!(!save("onFocusChange", (false, true), (false, true)) && !save("onFocusChange", (true, true), (true, true)));
        assert!(save("onWindowChange", (true, true), (true, false)) && !save("onWindowChange", (true, true), (false, true)));
        assert!(!save("off", (true, true), (false, false)));
    }
}
