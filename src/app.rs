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
use crate::workspace::Workspace;
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
    pub(crate) profiles: crate::profiles::Dialog,
    sync: crate::sync::Sync,
    pub(crate) trust: crate::trust::Prompt,
    exiting: bool,
    focused: bool,
    pub chord: Option<egui::KeyboardShortcut>,
    pub macros: crate::macros::Macro,
    run_form: Option<crate::run_form::Form>,
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
    pub inbox: Option<std::sync::mpsc::Receiver<crate::ipc::Request>>,
    pub waits: Vec<crate::ipc::Wait>,
    reveal_at: Option<u64>,
    jump: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
    pub(crate) startup: Option<Startup>,
}

pub(crate) struct Startup {
    pub(crate) arg: Option<PathBuf>,
    restore: bool,
    fresh: bool,
    args: crate::cli::Args,
    cmds: Vec<Command>,
    pub(crate) tabs: bool,
    loaded: std::sync::mpsc::Receiver<()>,
}

impl Halo {
    pub fn new(cc: &eframe::CreationContext<'_>, mut args: crate::cli::Args, server: Option<crate::ipc::Server>) -> Self {
        let ctx = &cc.egui_ctx;
        let session = Session::load();
        crate::trust::seed(session.recent.iter().cloned().chain(session.folder.clone()));
        let mut app = Self::build(ctx, Settings::load(), session);
        app.reveal_at = Some(ctx.cumulative_frame_nr() + 3);
        if app.session.layout.zen.is_some() && !app.settings.zen_restore {
            app.toggle_zen();
        }
        if app.settings.auto_update && !cfg!(debug_assertions) && std::env::var_os("HALO_COMMANDS").is_none() {
            app.update = Some(crate::update::start(ctx));
        }

        let arg = args.paths.iter().position(|t| t.path.is_dir()).map(|i| args.paths.remove(i).path);
        let restore = app.settings.restore_session && !crate::windows::secondary();
        let fresh = arg.is_none() && args.paths.is_empty() && args.add.is_empty() && args.diff.is_none() && args.stdin.is_none();
        let startup = std::env::var("HALO_COMMANDS").unwrap_or_default();
        let cmds = command::all().into_iter().filter(|c| startup.split(',').any(|id| id == c.id())).collect();
        let tabs = (fresh && restore && !app.session.files.is_empty()) || !app.session.backups.is_empty() || !args.paths.is_empty() || args.diff.is_some() || args.stdin.is_some();
        let folder = app.session.workspace.file.clone().or(app.session.folder.clone()).filter(|_| restore && arg.is_none());
        let (wake, (tx, rx)) = (ctx.clone(), std::sync::mpsc::channel::<()>());
        std::thread::spawn(move || {
            let _ = folder.is_some_and(|f| f.exists());
            if tabs {
                crate::editor::warm();
            }
            drop(tx);
            wake.request_repaint();
        });
        app.startup = Some(Startup { arg, restore, fresh, args, cmds, tabs, loaded: rx });
        let wake = ctx.clone();
        app.inbox = server.map(|server| crate::ipc::serve(server, move || wake.request_repaint()));
        app
    }

    fn finish_startup(&mut self, startup: Startup, ctx: &egui::Context) {
        let Startup { arg, restore, fresh, args, cmds, .. } = startup;
        if let Some(key) = self.session.workspace_key() {
            self.session.folder_states.entry(key).or_insert_with(|| crate::settings::FolderState {
                grid: self.session.grid.clone(), files: self.session.files.clone(), pinned: self.session.pinned.clone(), tree: None,
            });
        }
        let saved = self.session.workspace.clone();
        let last = match &saved.file {
            _ if !restore => Workspace::default(),
            Some(file) => Workspace::open(file).unwrap_or_else(|error| {
                ui::notify(error);
                Workspace::default()
            }),
            None if saved.folders.len() > 1 => saved,
            None => self.session.folder.clone().filter(|f| f.is_dir()).map(Workspace::single).unwrap_or_default(),
        };
        let explicit = arg.is_some();
        let workspace = arg.map_or(last, |path| Workspace::single(strip_unc(path)));
        self.configure_workspace(workspace, ctx);
        if restore && (fresh || explicit) {
            if let Some(state) = self.session.workspace_key().and_then(|key| self.session.folder_states.get(&key)).cloned() {
                self.restore_folder_state(&state);
            } else if fresh {
                self.restore_folder_state(&crate::settings::FolderState {
                    grid: self.session.grid.clone(), files: self.session.files.clone(), pinned: self.session.pinned.clone(), tree: None,
                });
            }
        }
        self.editors.restore(&self.session.backups);
        crate::terminal_restore::restore(&mut self.bottom, &mut self.editors, &self.settings, self.session.folder.as_deref(), ctx);
        self.open_args(args, None, ctx);
        self.cmds.extend(cmds);
    }

    pub(crate) fn build(ctx: &egui::Context, settings: Settings, session: Session) -> Self {
        crate::a11y::configure(&settings);
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
            profiles: Default::default(),
            sync: Default::default(),
            trust: Default::default(),
            exiting: false,
            focused: true,
            chord: None,
            macros: Default::default(),
            run_form: None,
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
            inbox: None,
            waits: vec![],
            reveal_at: None,
            jump: None,
            startup: None,
        };
        app.editors.style_marks.clone_from(&app.session.style_marks);
        app.editors.config = app.settings.clone();
        app
    }

    fn set_folder(&mut self, folder: Option<PathBuf>, ctx: &egui::Context) {
        self.open_workspace(folder.map(strip_unc).map(Workspace::single).unwrap_or_default(), ctx);
    }

    fn store_folder_state(&mut self) {
        if let Some(key) = self.session.workspace_key() {
            self.session.folder_states.insert(key, crate::settings::FolderState {
                grid: Some(self.editors.folder_grid()), tree: Some(self.explorer.tree_state()), ..Default::default()
            });
        }
    }

    fn restore_folder_state(&mut self, state: &crate::settings::FolderState) {
        if !self.editors.switch_grid(state.grid.as_ref()) || state.grid.is_none() {
            let pins: Vec<_> = state.pinned.iter().filter(|(_, f)| f.is_file()).cloned().collect();
            self.editors.restore_pins(&pins);
        }
        for path in state.files.iter().filter(|f| f.is_file()) {
            if !self.editors.open_paths().contains(path) { self.editors.open_restored(path); }
        }
        if let Some(tree) = &state.tree {
            self.explorer.restore_tree(tree, self.editors.active_path());
        }
    }

    fn open_workspace(&mut self, workspace: Workspace, ctx: &egui::Context) {
        if self.session.workspace == workspace { return; }
        let had_folder = self.session.folder.is_some() || self.session.workspace.file.is_some();
        self.store_folder_state();
        self.configure_workspace(workspace, ctx);
        if let Some(state) = self.session.workspace_key().and_then(|key| self.session.folder_states.get(&key)).cloned() {
            self.restore_folder_state(&state);
        } else if had_folder {
            self.editors.switch_grid(None);
        }
        self.session.files = self.editors.open_paths();
        self.session.pinned = self.editors.pinned_paths();
        self.session.grid = Some(self.editors.grid_state());
        self.session.save();
    }

    fn set_workspace(&mut self, workspace: Workspace, ctx: &egui::Context) {
        self.store_folder_state();
        self.configure_workspace(workspace, ctx);
        self.session.save();
    }

    fn configure_workspace(&mut self, workspace: Workspace, ctx: &egui::Context) {
        let folder = workspace.first();
        if self.session.folder != folder {
            crate::terminal_restore::save(&self.bottom, &self.editors, &self.settings, self.session.folder.as_deref());
        }
        crate::debug::store(&mut self.session, &self.editors.marks);
        self.session.style_marks.clone_from(&self.editors.style_marks);
        self.editors.bookmarks.store(&mut self.session.bookmarks);
        self.editors.layouts.store(&mut self.session.layouts);
        if let Some(remembered) = workspace.file.clone().or_else(|| folder.clone().filter(|_| !workspace.is_multi())) {
            self.session.remember(&remembered);
        }
        let title = match workspace.name() {
            Some(name) => format!("{name} - Halo"),
            None => "Halo".to_owned(),
        };
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
        crate::workspace::set(&workspace);
        crate::layers::set_workspace(&workspace);
        self.reload_settings(ctx);
        self.explorer.set_roots(workspace.roots(), workspace.names(), workspace.name().filter(|_| workspace.is_multi()));
        self.editors.set_root(folder.clone());
        self.editors.lsp.set_folders(workspace.roots().into_iter().zip(workspace.names()).collect());
        self.git.set_roots(workspace.roots(), ctx);
        self.session.folder = folder;
        self.session.workspace = workspace;
        crate::debug::restore(&self.session, &mut self.editors.marks);
        self.editors.bookmarks.restore(&self.session.bookmarks, self.session.folder.clone());
        self.editors.layouts.restore(&self.session.layouts, self.session.folder.clone());
    }

    fn edit_workspace(&mut self, edit: impl FnOnce(&mut Workspace), ctx: &egui::Context) {
        let mut workspace = self.session.workspace.clone();
        edit(&mut workspace);
        if let Err(error) = workspace.save() {
            ui::notify(error);
        }
        self.set_workspace(workspace, ctx);
    }

    fn workspace_command(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        let filter = |dialog: rfd::FileDialog| dialog.add_filter("Code Workspace", &["code-workspace"]);
        match cmd {
            AddFolderToWorkspace => {
                let folders = rfd::FileDialog::new().pick_folders().unwrap_or_default();
                if !folders.is_empty() {
                    self.edit_workspace(|w| folders.into_iter().for_each(|f| _ = w.add(strip_unc(f))), ctx);
                }
            }
            RemoveFolderFromWorkspace if self.session.workspace.folders.is_empty() => ui::notify("There are no folders in the workspace."),
            RemoveFolderFromWorkspace => {
                let names = self.session.workspace.names().into_iter();
                let items = names.zip(self.session.workspace.roots()).map(|(n, p)| (n, p.to_string_lossy().into_owned(), RemoveWorkspaceFolder(p)));
                self.search.pick("Select a folder to remove from the workspace", items.collect());
            }
            AddWorkspaceFolder(path) => self.edit_workspace(|w| _ = w.add(strip_unc(path)), ctx),
            RemoveWorkspaceFolder(path) => self.edit_workspace(|w| w.folders.retain(|f| f.path != path), ctx),
            MoveWorkspaceFolder(from, to) => self.edit_workspace(|w| w.reorder(from, to), ctx),
            SaveWorkspaceAs => {
                let dir = self.session.workspace.file.as_ref().and_then(|f| f.parent().map(PathBuf::from)).or(self.session.folder.clone());
                let dialog = filter(rfd::FileDialog::new()).set_file_name("workspace.code-workspace");
                let dialog = match dir {
                    Some(dir) => dialog.set_directory(dir),
                    None => dialog,
                };
                if let Some(file) = dialog.save_file() {
                    let mut workspace = self.session.workspace.clone();
                    match workspace.save_as(&strip_unc(file)) {
                        Ok(()) => self.set_workspace(workspace, ctx),
                        Err(error) => ui::notify(error),
                    }
                }
            }
            OpenWorkspaceFromFile => {
                if let Some(file) = filter(rfd::FileDialog::new()).pick_file() {
                    self.workspace_command(OpenWorkspace(strip_unc(file)), ctx);
                }
            }
            OpenWorkspace(file) => match Workspace::open(&file) {
                Ok(workspace) => self.open_workspace(workspace, ctx),
                Err(error) => ui::notify(error),
            },
            _ => self.set_folder(None, ctx),
        }
    }

    pub fn active_root(&self) -> Option<PathBuf> {
        let workspace = &self.session.workspace;
        self.editors.active_path().and_then(|p| workspace.owner_path(&p)).or_else(|| workspace.first())
    }

    pub fn folder_name(&self) -> Option<String> {
        self.session.workspace.name()
    }

    fn save_session(&mut self) {
        if crate::windows::secondary() || self.startup.is_some() {
            return;
        }
        self.store_folder_state();
        let backups = crate::settings::dir().join("backups");
        self.session.files = self.editors.open_paths();
        self.session.pinned = self.editors.pinned_paths();
        self.session.grid = Some(self.editors.grid_state());
        self.session.backups = if self.settings.hot_exit { self.editors.backup(&backups) } else { vec![] };
        if !self.settings.hot_exit {
            let _ = std::fs::remove_dir_all(backups);
        }
        crate::debug::store(&mut self.session, &self.editors.marks);
        self.session.style_marks.clone_from(&self.editors.style_marks);
        self.editors.bookmarks.store(&mut self.session.bookmarks);
        self.editors.layouts.store(&mut self.session.layouts);
        self.session.save();
        crate::terminal_restore::save(&self.bottom, &self.editors, &self.settings, self.session.folder.as_deref());
    }

    fn profile_changed(&mut self, ctx: &egui::Context) {
        self.profiles.changed();
        self.reload_settings(ctx);
        command::reload();
        crate::editor::reload_snippets();
    }

    fn reload_settings(&mut self, ctx: &egui::Context) {
        self.settings = Settings::load();
        self.apply_settings(ctx);
    }

    pub(crate) fn apply_settings(&mut self, ctx: &egui::Context) {
        crate::a11y::configure(&self.settings);
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
        let cwd = self.active_root();
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
                    if crate::workspace::is_workspace_file(&path) {
                        offer_workspace(&path);
                    }
                }
            }
            AddFolderToWorkspace | AddWorkspaceFolder(_) | RemoveFolderFromWorkspace | RemoveWorkspaceFolder(_) | MoveWorkspaceFolder(..) | SaveWorkspaceAs
            | OpenWorkspaceFromFile | OpenWorkspace(_) | CloseWorkspace => self.workspace_command(cmd, ctx),
            OpenDiff(title, args) => {
                let inline = !self.settings.diff_side_by_side;
                match self.git.top().map(std::path::Path::to_path_buf) {
                    Some(top) if self.editors.compare(&title, &args, &top, inline, ctx) => {}
                    Some(top) => self.editors.open_multi_diff(&title, &args, &top, ctx),
                    None => self.git.diff(title, args),
                }
            }
            OpenProcessExplorer => self.editors.open_process_explorer(ctx),
            OpenRemoteRepository => self.editors.open_remote_repository(),
            OpenText(title, text, path) => self.editors.open_text(&title, &text, Some(&path)),
            OpenAllChanges => match self.git.top().map(std::path::Path::to_path_buf).or_else(|| self.active_root()) {
                Some(top) => self.editors.open_multi_diff("All Changes", &["diff".into(), "HEAD".into()], &top, ctx),
                None => ui::notify_warning("Open a Git repository to see its changes."),
            },
            OpenMerge(path) => self.editors.open_merge(&path, ctx),
            ReviewPullRequest(number) => match self.git.top() {
                Some(top) => self.editors.review_pr(top.to_path_buf(), number, !self.settings.diff_side_by_side, ctx),
                None => ui::notify_warning("Open a Git repository to review a pull request."),
            },
            PickPullRequestReview => match self.git.prs() {
                Some(Ok(prs)) if !prs.is_empty() => {
                    let items = prs.iter().map(|pr| (format!("#{} {}", pr.number, pr.title), pr.head_ref_name.clone(), ReviewPullRequest(pr.number)));
                    self.search.pick("Select a pull request to review", items.collect());
                }
                Some(Ok(_)) => ui::notify_info("There are no open pull requests."),
                Some(Err(error)) => ui::notify_error(error.lines().next().unwrap_or_default().to_owned()),
                None => {
                    self.git.load_prs();
                    ui::notify_info("Loading pull requests. Run the command again in a moment.");
                }
            },
            CloseFolder => self.set_folder(None, ctx),
            NewWindow | OpenFolderInNewWindow | DuplicateWorkspace => self.window_command(&cmd),
            OpenSettings => self.page.open = true,
            CustomizeMenus => self.page.show_menus(),
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
            NewSearchEditor | OpenSearchEditor => self.editors.open_search(self.find.editor_query(cmd == OpenSearchEditor), cmd == NewSearchEditor, ctx),
            ShowTesting => self.reveal(View::Testing),
            RunAllTests | RunFailedTests | RunTestAtCursor | DebugTestAtCursor | RerunLastTestRun | CancelTestRun | RefreshTests | ShowTestOutput
            | RunAllTestsWithCoverage | ToggleCoverageDisplay => self.testing_exec(&cmd, ctx),
            StartDebugging | RunWithoutDebugging | StopDebugging | RestartDebugging | PauseDebugging | StepOver | StepInto | StepOut
            | RunToCursor | JumpToCursor | AddFunctionBreakpoint | BreakOnValueChange | RemoveAllBreakpoints | AddWatch | AddConfiguration => {
                if matches!(cmd, AddWatch | AddFunctionBreakpoint | BreakOnValueChange) {
                    self.reveal(View::RunAndDebug);
                }
                if cmd == RunWithoutDebugging || (cmd == StartDebugging && !self.debug.paused()) {
                    self.editors.exec(&SaveAll, &self.settings, ctx);
                }
                self.debug_exec(&cmd, ctx);
            }
            OpenDisassembly | OpenMemory | AttachProcess(_) => _ = self.debug_exec(&cmd, ctx),
            ToggleSidebar => self.session.sidebar ^= true,
            TogglePanel => self.session.panel ^= true,
            ToggleAi => self.toggle_view(View::Ai),
            AgentChangesKeepAll | AgentChangesUndoAll | AgentChangesShow | AgentChangesResetBaseline => {
                self.reveal(View::Ai);
                let folder = self.session.folder.clone();
                self.ai.review.exec(&cmd, folder.as_deref(), ctx);
            }
            ToggleFullScreen | ToggleZenMode | ToggleCenteredLayout | ToggleSidebarPosition | ToggleSecondarySidebar | MovePanelLeft
            | MovePanelRight | MovePanelBottom | MovePanelTop | TogglePanelPosition | ToggleMaximizedPanel | AlignPanelCenter
            | AlignPanelJustify | AlignPanelLeft | AlignPanelRight | ActivityBarSide | ActivityBarTop | ActivityBarHidden | MoveView
            | MoveViewTo(_) | MoveViewPlace(..) | MoveViewToNewWindow | MoveViewBackToMain | DetachView(_) | AttachView(_) | ResetViewLocations => self.layout_command(cmd),
            ToggleHideSecrets => {
                self.settings.hide_secrets ^= true;
                self.settings.save();
            }
            ToggleElasticTabstops => {
                self.settings.elastic_tabstops ^= true;
                self.settings.save();
            }
            ToggleMinimap => {
                self.settings.minimap ^= true;
                self.settings.save();
            }
            ToggleFileNesting => {
                self.settings.file_nesting ^= true;
                self.settings.save();
            }
            ToggleColumnSelection => {
                self.settings.column_selection ^= true;
                self.settings.save();
            }
            SwitchProfile => self.search.pick("Select a profile", crate::profiles::picks(&crate::settings::dir())),
            NewProfile => self.profiles.name(),
            CreateProfile(name, copy) => match crate::profiles::create(&crate::settings::dir(), &name, copy) {
                Ok(()) => {
                    self.profile_changed(ctx);
                    ui::notify_info(format!("Profile {name} created and active."));
                }
                Err(e) => ui::notify(e),
            },
            SwitchProfileTo(name) => match crate::profiles::switch(&crate::settings::dir(), &name) {
                Ok(()) => {
                    self.profile_changed(ctx);
                    ui::notify_info(format!("Switched to the {name} profile."));
                }
                Err(e) => ui::notify(format!("Could not switch profiles: {e}")),
            },
            DeleteProfile => {
                let base = crate::settings::dir();
                let active = crate::profiles::active(&base);
                let items: Vec<_> = crate::profiles::list(&base).into_iter().filter(|n| *n != active && n != crate::profiles::DEFAULT).map(|n| (n.clone(), String::new(), DeleteProfileNamed(n))).collect();
                match items.is_empty() {
                    true => ui::notify_info("There is no other profile to delete. The active and Default profiles are kept."),
                    false => self.search.pick("Select a profile to delete", items),
                }
            }
            DeleteProfileNamed(name) => self.profiles.delete(name),
            RemoveProfile(name) => match crate::profiles::delete(&crate::settings::dir(), &name) {
                Ok(()) => ui::notify_info(format!("Moved the {name} profile to the trash.")),
                Err(e) => ui::notify(format!("Could not delete the profile: {e}")),
            },
            SyncUpload | SyncDownload => self.sync.start(cmd == SyncDownload, ctx),
            ManageWorkspaceTrust => match self.session.workspace.first() {
                Some(folder) if crate::trust::trusted(&folder) => self.search.pick(
                    "This folder is trusted",
                    vec![("Remove Trust".into(), "restricted mode: no workspace settings, tasks or launch configurations".into(), SetFolderTrust(folder, false))],
                ),
                Some(folder) => self.trust.folder = Some(folder),
                None => ui::notify_info("Open a folder to manage its trust."),
            },
            SetFolderTrust(folder, trust) => {
                crate::trust::set(&folder, trust);
                self.reload_settings(ctx);
            }
            ReopenInHexEditor => {
                if !self.editors.hex_active() {
                    ui::notify("Save the file first: the hex editor reads the file from disk.".to_owned());
                }
            }
            ToggleTableView => {
                self.editors.toggle_table();
            }
            OpenInLogViewer => {
                if !self.editors.stream_active(ctx) {
                    ui::notify("Save the file first: the log viewer reads the file from disk.".to_owned());
                }
            }
            ToggleVimMode => {
                self.settings.vim ^= true;
                self.editors.vim.reset();
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
            InstallInPath | ConfirmInstallInPath => crate::cli::install(&cmd),
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
            UndoTree => match self.editors.undo_choices() {
                items if items.is_empty() => ui::notify_info("This file has no undo history yet."),
                items => self.search.pick("Select an undo state to restore", items),
            },
            ListBookmarks => match self.editors.bookmarks.choices() {
                items if items.is_empty() => ui::notify_info("There are no bookmarks yet."),
                items => self.search.pick("Select a bookmark", items),
            },
            NewScratch => {
                let kinds = crate::scratch::KINDS.iter().map(|(ext, name)| (name.to_string(), format!(".{ext}"), NewScratchAs(ext.to_string())));
                self.search.pick("Select the language of the scratch file", kinds.collect());
            }
            NewScratchAs(ext) => match crate::scratch::create(&crate::scratch::dir(self.session.folder.as_deref()), &ext) {
                Ok(path) => self.editors.open_kept(&path),
                Err(e) => ui::notify(format!("Could not create the scratch file: {e}")),
            },
            OpenScratch => match crate::scratch::list(&crate::scratch::dir(self.session.folder.as_deref())) {
                files if files.is_empty() => ui::notify_info("This folder has no scratch files yet. Use Scratch: New Scratch File..."),
                files => {
                    let item = |p: PathBuf| (crate::history::name(&p), p.to_string_lossy().into_owned(), OpenPath(p));
                    self.search.pick("Select a scratch file", files.into_iter().map(item).collect());
                }
            },
            ToggleOpenEditorsTree => {
                self.settings.open_editors_tree ^= true;
                self.settings.save();
            }
            EditLaunchForm | EditTaskForm => match self.active_root() {
                None => ui::notify_info("Open a folder to edit its run configurations."),
                Some(folder) => match crate::run_form::names(&folder, cmd == EditLaunchForm) {
                    Err(error) => ui::notify(error),
                    Ok(names) => {
                        let launch = cmd == EditLaunchForm;
                        let new = (if launch { "New Configuration..." } else { "New Task..." }.to_owned(), String::new(), RunForm(launch, None));
                        let items = names.into_iter().enumerate().map(|(i, name)| (name, String::new(), RunForm(launch, Some(i))));
                        self.search.pick(if launch { "Select a launch configuration to edit" } else { "Select a task to edit" }, std::iter::once(new).chain(items).collect());
                    }
                },
            },
            RunForm(launch, index) => match self.active_root().map(|folder| crate::run_form::Form::open(&folder, launch, index)) {
                Some(Ok(form)) => self.run_form = Some(form),
                Some(Err(error)) => ui::notify(error),
                None => ui::notify_info("Open a folder to edit its run configurations."),
            },
            ShowTodos => {
                self.reveal(View::Search);
                self.find.search_for(crate::find::Query { text: r"(?:TODO|FIXME|HACK|XXX|BUG)".into(), case: true, regex: true, ..Default::default() });
            }
            SetWindowColor => {
                let colors = crate::titlebar::WINDOW_COLORS.iter().map(|(hex, name)| (name.to_string(), hex.to_string(), SetWindowColorTo(hex.to_string())));
                self.search.pick("Select a title bar color for this folder", colors.collect());
            }
            SetWindowColorTo(hex) => {
                let value = (!hex.is_empty()).then(|| serde_json::json!(hex));
                crate::layers::write(crate::layers::Layer::Workspace, None, &[("window_color".to_owned(), value)]);
                self.settings = Settings::load();
                self.apply_settings(ctx);
            }
            SwitchLayout | DeleteLayout => match self.editors.layouts.names() {
                names if names.is_empty() => ui::notify_info("No layouts are saved for this folder yet. Use Layouts: Save Open Editors As... first."),
                names => {
                    let (hint, pick): (&str, fn(String) -> Command) = match cmd {
                        SwitchLayout => ("Select a layout to switch to", OpenLayout),
                        _ => ("Select a layout to delete", RemoveLayout),
                    };
                    self.search.pick(hint, names.into_iter().map(|name| (name.clone(), String::new(), pick(name))).collect());
                }
            },
            PasteFromHistory => {
                let item = |text: String| {
                    let first: String = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().chars().take(80).collect();
                    let lines = text.lines().count();
                    (first, if lines > 1 { format!("{lines} lines") } else { String::new() }, PasteText(text))
                };
                match crate::editor::clip_history() {
                    clips if clips.is_empty() => ui::notify_info("Nothing has been copied in an editor yet."),
                    clips => self.search.pick("Select text to paste", clips.into_iter().map(item).collect()),
                }
            }
            ToggleMacroRecording => self.macros.toggle(),
            PlayMacro => self.macros.play(),
            ClearOutput => crate::output::clear_current(),
            OpenOutputInEditor => self.editors.open_untitled(&crate::output::current_text()),
            ShowNotifications | ClearNotifications | ToggleDoNotDisturb | FocusNotification => crate::notifications::exec(&cmd),
            FocusNextRegion | FocusPrevRegion => {
                if let Some(region) = crate::a11y::cycle(ctx, cmd == FocusNextRegion) {
                    if region == crate::a11y::Region::Editor {
                        self.editors.focus();
                    }
                    crate::a11y::announce(region.label(), false);
                }
            }
            ToggleTabFocus => crate::a11y::announce(if crate::a11y::toggle_tab_focus() { "Tab moves focus" } else { "Tab inserts a tab" }, true),
            ShowAccessibilityHelp => crate::a11y::show_help(),
            AnnounceCursorPosition => match self.editors.status() {
                Some(s) => {
                    let file = self.editors.active_path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
                    let selected = if s.selected > 0 { format!(", {} selected", s.selected) } else { String::new() };
                    crate::a11y::announce(format!("Line {}, Column {}{selected} {file}", s.line, s.column), true);
                }
                None => crate::a11y::announce("No active editor", true),
            },
            ToggleScreenReaderMode => {
                self.settings.accessibility_support = if crate::a11y::optimized() { "off" } else { "on" }.into();
                self.settings.save();
                self.apply_settings(ctx);
                crate::a11y::announce(if crate::a11y::optimized() { "Screen reader optimized mode on" } else { "Screen reader optimized mode off" }, true);
            }
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
            OpenRecent => {
                let item = |p: &PathBuf| (p.file_name().unwrap_or(p.as_os_str()).to_string_lossy().into_owned(), p.to_string_lossy().into_owned(), open_recent(p));
                let items: Vec<_> = self.session.recent.iter().chain(&self.session.recent_files).map(item).collect();
                match items.is_empty() {
                    true => ui::notify("There are no recent folders or files yet."),
                    false => self.search.pick("Select a folder or file to open", items),
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
            NewTerminal if self.session.workspace.folders.len() > 1 => {
                let workspace = &self.session.workspace;
                let mut items: Vec<_> = workspace.names().into_iter().zip(workspace.roots()).map(|(n, p)| (n, p.to_string_lossy().into_owned(), NewTerminalIn(p))).collect();
                items.sort_by_key(|(_, path, _)| Some(std::path::Path::new(path)) != cwd.as_deref());
                self.search.pick("Select the working directory for the new terminal", items);
            }
            NewTerminalIn(dir) => {
                self.session.panel = true;
                self.bottom.exec(&NewTerminal, &self.settings, Some(&dir), ctx);
            }
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
        crate::a11y::saved(ctx.input(|i| i.time));
        self.git.refresh(ctx);
        self.explorer.refresh();
        let active = self.editors.active_path();
        if active.as_ref().is_some_and(|p| crate::layers::watches(p)) {
            self.reload_settings(ctx);
        }
        if active.as_ref().is_some_and(|p| *p == command::path()) {
            command::reload();
        }
        if active.is_some_and(|p| p == crate::editor::snippets_path()) {
            crate::editor::reload_snippets();
        }
    }

    pub(crate) fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let bindings = command::bindings();
        let editor = self.zone == Zone::Editor && !self.editors.terminal_focused();
        let keys = command::context();
        ctx.input_mut(|i| {
            let Some(first) = self.chord else {
                for binding in bindings.iter() {
                    let chord = binding.chord.is_some();
                    if (binding.scope == Scope::Global || (chord && editor)) && binding.active(&keys) && command::consume(i, &binding.shortcut) {
                        match chord {
                            true => self.chord = Some(binding.shortcut),
                            false => self.cmds.push(binding.command.clone()),
                        }
                    }
                }
                return;
            };
            let second = bindings.iter().filter(|b| b.shortcut == first && b.active(&keys)).find(|b| b.chord.is_some_and(|c| command::consume(i, &c)));
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
            ("searchViewletFocus", self.find.has_focus(ctx).into()),
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
        if close_requested && self.settings.hot_exit && !crate::windows::secondary() && !self.editors.hex_dirty() {
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
                        self.cmds.push(open_recent(folder));
                    }
                }
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(cmd) = self.profiles.ui(ctx, &self.theme) {
            self.cmds.push(cmd);
        }
        self.trust.check(self.session.workspace.first().as_deref());
        if let Some(cmd) = self.trust.ui(ctx, &self.theme) {
            self.cmds.push(cmd);
        }
        if self.sync.poll() == Some(true) {
            self.profile_changed(ctx);
        }
        if self.page.open {
            self.settings_dialog(ctx);
        }
        if self.about {
            let modal = ui::modal(ctx, "about", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(280.0);
                ui.vertical_centered(|ui| {
                    ui::logo(ui, 48.0, self.theme.dark);
                    ui.heading("Halo");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.colored_label(self.theme.text_dim, "An ultra-lightweight native code editor.");
                    ui.add_space(8.0);
                    keys.primary(ui, "Close", true)
                })
                .inner
            });
            if modal.inner || modal.should_close() {
                self.about = false;
            }
        }
        if self.confirm_exit {
            let mut choice = None;
            let modal = ui::modal(ctx, "confirm_exit", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(340.0);
                ui.heading("Unsaved changes");
                ui.label("Some files have unsaved changes. Save them before closing?");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if keys.primary(ui, "Save All", true) {
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

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let line = ctx.options(|o| o.input_options.line_scroll_speed);
        crate::ui::direct_wheel(&mut raw_input.events, line, self.settings.smooth_scrolling && !crate::a11y::reduced());
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_session();
    }
}

impl Halo {
    pub(crate) fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(at) = self.reveal_at {
            if ctx.cumulative_frame_nr() >= at {
                crate::windows::conceal(false);
                self.reveal_at = None;
            }
            ctx.request_repaint();
        }
        if let Some(startup) = self.startup.take_if(|s| self.reveal_at.is_none() && s.loaded.try_recv() != Err(std::sync::mpsc::TryRecvError::Empty)) {
            self.finish_startup(startup, &ctx);
        }
        if let Some(path) = self.editors.active_path().filter(|p| self.session.recent_files.first() != Some(p)) {
            self.session.remember_file(&path);
        }
        if self.jump.as_ref().is_none_or(|(folders, files)| *folders != self.session.recent || *files != self.session.recent_files) {
            let jump = (self.session.recent.clone(), self.session.recent_files.clone());
            if std::env::var_os("HALO_COMMANDS").is_none() && !cfg!(test) {
                crate::jumplist::update(jump.0.clone(), jump.1.clone());
            }
            self.jump = Some(jump);
        }
        let window = ui.max_rect();
        if crate::shell_integration::take_finished() {
            self.git.refresh(&ctx);
            self.explorer.refresh();
        }
        if self.git.poll() {
            self.explorer.refresh();
            self.editors.reload_clean();
            self.editors.refresh_blame();
            self.timeline.reload();
        }
        if self.session.workspace.is_multi() {
            let active = self.editors.active_path();
            self.git.follow(active.as_deref(), &ctx);
        }
        self.tasks.poll(&mut self.bottom, &ctx);
        if self.editors.history.changed() {
            self.timeline.reload();
        }
        self.poll_compare(&ctx);
        if self.startup.is_none() {
            self.poll_ipc(&ctx);
        }
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
        if crate::a11y::frame(&ctx, self.editors.lsp.counts()) {
            self.apply_settings(&ctx);
        }
        crate::a11y::watch("tests", self.testing.status().map(|s| s.1).unwrap_or_default().split(" (").next().unwrap_or_default());
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
        self.macros.frame(&ctx, &mut self.cmds);
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
        if self.run_form.as_mut().is_some_and(|form| form.ui(ui)) {
            self.run_form = None;
        }
        self.titlebar(ui);
        if self.session.layout.zen.is_none() {
            crate::a11y::region(&ctx, Some(crate::a11y::Region::Status));
            self.statusbar(ui);
            crate::a11y::region(&ctx, None);
        }
        self.workbench(ui);
        crate::a11y::region(&ctx, None);
        self.floating(&ctx);
        self.debug_frame(&ctx, window.top() + 44.0);
        self.testing_frame(&ctx);
        self.dialogs(&ctx);
        crate::a11y::help_ui(&ctx, &self.theme);

        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false));
        if !maximized && !cfg!(target_os = "macos") {
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("window_border"))).rect_stroke(
                window,
                CornerRadius::ZERO,
                Stroke::new(1.0, self.theme.border),
                StrokeKind::Inside,
            );
            resize_edges(&ctx, window);
        }

        if let Some((start, color)) = self.theme_fade {
            let t = if crate::a11y::reduced() { 1.0 } else { ((ctx.input(|i| i.time) - start) / 0.3) as f32 };
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

        self.macros.commands(&self.cmds);
        for cmd in std::mem::take(&mut self.cmds) {
            self.dispatch(cmd, &ctx);
        }
        self.macros.derived(self.cmds.len());
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

pub(crate) fn offer_workspace(file: &std::path::Path) {
    let text = format!("{} is a workspace file. Open the workspace?", file.file_name().unwrap_or_default().to_string_lossy());
    ui::notify_with(crate::notifications::Severity::Info, text, vec![("Open Workspace".into(), Command::OpenWorkspace(file.to_path_buf()))]);
}

fn open_recent(path: &std::path::Path) -> Command {
    match crate::workspace::is_workspace_file(path) {
        true => Command::OpenWorkspace(path.to_path_buf()),
        false => Command::OpenPath(path.to_path_buf()),
    }
}

pub(crate) fn strip_unc(path: PathBuf) -> PathBuf {
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

fn outline_order(symbols: &[lsp::Symbol], sort: u8) -> Vec<usize> {
    let key = |i: usize| {
        let s = &symbols[i];
        match sort {
            1 => (0, s.name.to_lowercase(), i),
            2 => (s.kind, s.name.to_lowercase(), i),
            _ => (0, String::new(), i),
        }
    };
    let mut stack = vec![];
    let paths: Vec<Vec<_>> = symbols
        .iter()
        .enumerate()
        .map(|(i, s)| {
            stack.truncate(s.depth);
            stack.push(i);
            stack.iter().map(|&j| key(j)).collect()
        })
        .collect();
    let mut order: Vec<usize> = (0..symbols.len()).collect();
    order.sort_by(|a, b| paths[*a].cmp(&paths[*b]));
    order
}

pub(crate) fn outline(
    ui: &mut Ui,
    theme: &Theme,
    symbols: Option<(&[lsp::Symbol], usize)>,
    active: bool,
    loading: bool,
    session: &mut Session,
    cmds: &mut Vec<Command>,
) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    let rect = ui::section(ui, theme, "OUTLINE", &mut session.outline).on_hover_text("Symbols of the active file, from its language server").rect;
    if session.outline {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(20.0, 20.0))));
        child.spacing_mut().button_padding = vec2(3.0, 2.0);
        child
            .menu_button(RichText::new(ui::icons::SORT_ASCENDING).size(13.0).color(theme.text_dim), |ui| {
                ui.set_min_width(160.0);
                ui::menu_pad(ui);
                for (value, label) in [(0, "Sort By: Position"), (1, "Sort By: Name"), (2, "Sort By: Kind")] {
                    if ui.radio(session.outline_sort == value, label).clicked() {
                        session.outline_sort = value;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Sort Outline");
    }
    if loading {
        ui::spinner(ui, pos2(rect.right() - 36.0, rect.center().y), 5.0, theme.text_dim);
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
    let order = outline_order(symbols, session.outline_sort);
    let followed = Id::new("outline_followed");
    let seen = current.map(|c| (c, symbols.len()));
    let reveal = current.filter(|_| ui.data(|d| d.get_temp(followed)) != seen);
    ui.data_mut(|d| d.insert_temp(followed, seen));
    ScrollArea::vertical().id_salt("outline").auto_shrink(false).show_rows(ui, 22.0, symbols.len(), |ui, range| {
        if let Some(row) = reveal.and_then(|c| order.iter().position(|&i| i == c)) {
            let top = ui.cursor().top() + (row as f32 - range.start as f32) * 22.0;
            ui.scroll_to_rect(egui::Rect::from_min_size(pos2(ui.cursor().left(), top), vec2(1.0, 22.0)), None);
        }
        for (index, symbol) in order[range.clone()].iter().map(|&i| (i, &symbols[i])) {
            let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            ui::row_bg(ui, theme, r, row.id, row.hovered(), current == Some(index));
            ui::tree_item(ui, &row, &symbol.name, current == Some(index), symbol.depth, None);
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
    fn sorts_outline_siblings_inside_their_parents() {
        let symbol = |name: &str, kind, depth| crate::lsp::Symbol { name: name.into(), kind, line: 0, col: 0, end_line: 0, depth, container: String::new(), path: None };
        let symbols = [symbol("Zed", 23, 0), symbol("run", 6, 1), symbol("Alpha", 8, 1), symbol("beta", 12, 0), symbol("Ace", 5, 0)];
        assert_eq!(super::outline_order(&symbols, 0), [0, 1, 2, 3, 4]);
        assert_eq!(super::outline_order(&symbols, 1), [4, 3, 0, 2, 1]);
        assert_eq!(super::outline_order(&symbols, 2), [4, 3, 0, 1, 2]);
    }

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

    #[test]
    fn startup_restore_waits_for_the_window_and_keeps_the_session() {
        use eframe::egui::{self, Rect, pos2, vec2};
        let files = vec![std::path::PathBuf::from("a.txt"), std::path::PathBuf::from("b.txt")];
        let ctx = egui::Context::default();
        let session = crate::settings::Session { files: files.clone(), ..Default::default() };
        let mut app = super::Halo::build(&ctx, crate::settings::Settings::default(), session);
        let (tx, loaded) = std::sync::mpsc::channel();
        let cmds = vec![crate::command::Command::ToggleAi];
        app.startup = Some(super::Startup { arg: None, restore: true, fresh: true, args: Default::default(), cmds, tabs: true, loaded });
        let run = |app: &mut super::Halo| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))), ..Default::default() };
            ctx.run_ui(input, |ui| app.frame(ui)).textures_delta.clear();
        };
        (0..3).for_each(|_| run(&mut app));
        assert!(app.startup.is_some(), "still loading in the background");
        drop(tx);
        app.reveal_at = Some(u64::MAX);
        (0..3).for_each(|_| run(&mut app));
        assert!(app.startup.is_some() && app.editors.is_empty() && !app.session.ai, "loaded but the window is not shown yet");
        app.save_session();
        assert_eq!(app.session.files, files);
        assert_eq!(app.session.grid, None);
    }

    #[test]
    fn switching_and_restarting_restores_each_folders_editors_and_explorer() {
        use crate::settings::{Session, Settings};
        use eframe::egui;
        let base = std::env::temp_dir().join(format!("halo-folder-state-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        let nested = a.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let [first, second, other] = [a.join("first.txt"), nested.join("second.txt"), b.join("other.txt")];
        for path in [&first, &second, &other] { std::fs::write(path, "hello").unwrap(); }

        let ctx = egui::Context::default();
        let mut app = super::Halo::build(&ctx, Settings::default(), Session::default());
        app.set_folder(Some(a.clone()), &ctx);
        app.editors.open_kept(&first);
        app.editors.open_kept(&second);
        app.editors.open_kept(&first);
        let tree = serde_json::from_value(serde_json::json!({
            "expanded": [&nested], "flipped": [], "cursor": &second, "collapsed": false, "open_editors": false
        })).unwrap();
        app.explorer.restore_tree(&tree, Some(first.clone()));
        app.set_folder(Some(b.clone()), &ctx);
        assert!(app.editors.is_empty());
        app.editors.open_kept(&other);
        app.set_folder(Some(a.clone()), &ctx);
        assert_eq!(app.editors.open_paths(), [first.clone(), second.clone()]);
        assert_eq!(app.editors.active_path(), Some(first.clone()));
        assert_eq!(serde_json::to_value(app.explorer.tree_state()).unwrap(), serde_json::to_value(&tree).unwrap());

        app.set_folder(Some(b), &ctx);
        assert_eq!(app.editors.open_paths(), [other]);
        let session = serde_json::from_slice(&serde_json::to_vec(&app.session).unwrap()).unwrap();
        let mut restarted = super::Halo::build(&ctx, Settings::default(), session);
        let (_, loaded) = std::sync::mpsc::channel();
        restarted.finish_startup(super::Startup { arg: Some(a), restore: true, fresh: false, args: Default::default(), cmds: vec![], tabs: true, loaded }, &ctx);
        assert_eq!(restarted.editors.active_path(), Some(first));
        assert_eq!(serde_json::to_value(restarted.explorer.tree_state()).unwrap(), serde_json::to_value(tree).unwrap());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn headless_quick_explorer_clicks_reuse_one_preview_tab() {
        use eframe::egui::{self, Event, Id, PointerButton, Rect, pos2, vec2};
        let dir = std::env::temp_dir().join(format!("halo-quick-clicks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        let ctx = egui::Context::default();
        let mut app = super::Halo::build(&ctx, crate::settings::Settings::default(), crate::settings::Session::default());
        let mut time = 0.0;
        let mut run = |app: &mut super::Halo, events: Vec<Event>| {
            time += 0.05;
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 900.0))), time: Some(time), events, focused: true, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| app.frame(ui));
            out.textures_delta.clear();
        };
        app.set_folder(Some(dir.clone()), &ctx);
        for _ in 0..40 {
            run(&mut app, vec![]);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for name in ["a.txt", "b.txt", "c.txt"] {
            let pos = ctx.read_response(Id::new(("explorer_row", dir.join(name)))).unwrap().rect.center();
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            run(&mut app, vec![Event::PointerMoved(pos), button(true)]);
            run(&mut app, vec![button(false)]);
        }
        assert_eq!(app.editors.active_path(), Some(dir.join("c.txt")));
        assert_eq!(app.editors.open_paths(), Vec::<std::path::PathBuf>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
