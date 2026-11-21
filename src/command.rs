use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewFile,
    OpenFile,
    OpenFolder,
    OpenFolderIn(PathBuf),
    OpenPath(PathBuf),
    OpenDiff(String, Vec<String>),
    CompareFile,
    CompareClipboard,
    CompareSaved,
    SelectForCompare,
    CompareWithSelected,
    SelectComparePath(PathBuf),
    ComparePath(PathBuf),
    ToggleInlineBlame,
    OpenLineChanges,
    RestoreSnapshot(PathBuf, PathBuf),
    Save,
    SaveAs,
    SaveAll,
    CloseEditor,
    CloseFolder,
    OpenSettings,
    OpenSettingsJson,
    OpenKeybindings,
    OpenSnippets,
    ImportTheme,
    BrowseThemes,
    SelectTheme,
    ImportThemeFile(PathBuf),
    SetTheme(String),
    ImportIconTheme,
    BrowseIconThemes,
    SelectIconTheme,
    ImportIconThemeFile(PathBuf),
    SetIconTheme(String),
    Exit,

    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Find,
    Replace,
    ToggleComment,
    ToggleBlockComment,
    EmmetExpand,
    MoveLineUp,
    MoveLineDown,
    CopyLineUp,
    CopyLineDown,
    DeleteLine,
    JoinLines,
    InsertLineAbove,
    InsertLineBelow,
    Fold,
    Unfold,
    FoldAll,
    UnfoldAll,

    SelectAll,
    AddCursorAbove,
    AddCursorBelow,
    SelectNextOccurrence,
    SelectAllOccurrences,
    ExpandSelection,
    ShrinkSelection,
    CursorUndo,

    ShowExplorer,
    ShowSourceControl,
    ShowRunAndDebug,
    ShowTesting,
    ToggleSidebar,
    TogglePanel,
    ToggleAi,
    SplitEditor,
    ToggleMinimap,
    ToggleWordWrap,
    ToggleTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,

    GoToFile,
    ShowCommands,
    FindInFiles,
    GoToSymbol,
    GoToLine,
    GoTo(usize, usize),
    NextEditor,
    PrevEditor,
    GoBack,
    GoForward,
    ReopenClosed,
    KeepEditor,
    NextChange,
    PrevChange,
    ShowNextChange,
    ShowPrevChange,
    ToggleInlineDiff,
    StageHunk,
    RevertHunk,
    UnstageHunk,
    StageSelectedRanges,
    RevertSelectedRanges,
    UnstageSelectedRanges,

    GoToDefinition,
    PeekDefinition,
    GoToDeclaration,
    GoToTypeDefinition,
    GoToImplementation,
    FindReferences,
    GoToWorkspaceSymbol,
    ShowCallHierarchy,
    ShowTypeHierarchy,
    ShowHover,
    TriggerSuggest,
    TriggerParameterHints,
    RenameSymbol,
    QuickFix,
    OrganizeImports,
    FormatDocument,
    FormatSelection,
    ShowProblems,
    RestartLanguageServers,

    NewTerminal,
    NewTerminalWith(String),
    SplitTerminal,
    KillTerminal,

    RunFile,
    RunTask(String),
    RunTaskPicker,
    RunBuildTask,
    RunTestTask,
    RerunLastTask,
    TerminateTask,
    ShowRunningTasks,
    ConfigureDefaultBuildTask,
    OpenTasksJson,
    RunTaskLabel(String),
    TerminateTaskLabel(String),
    RevealTask(String),
    SetDefaultBuild(String),
    StartDebugging,
    RunWithoutDebugging,
    StopDebugging,
    RestartDebugging,
    PauseDebugging,
    StepOver,
    StepInto,
    StepOut,
    ToggleBreakpoint,
    EditBreakpoint,
    RemoveAllBreakpoints,
    AddWatch,
    AddConfiguration,
    RunAllTests,
    RunFailedTests,
    RunTestAtCursor,
    DebugTestAtCursor,
    RerunLastTestRun,
    CancelTestRun,
    RefreshTests,
    ShowTestOutput,
    RunAllTestsWithCoverage,
    ToggleCoverageDisplay,

    GitSync,
    Git(GitCommand),
    GitAt(GitCommand, String),
    RefreshExplorer,
    PathChanged(PathBuf, Option<PathBuf>),
    About,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitCommand {
    Amend,
    AmendMessage,
    UndoCommit,
    Merge,
    Rebase,
    Continue,
    Abort,
    Skip,
    CherryPick,
    BranchAt,
    Tags,
    CreateTag,
    DeleteTag,
    PushTag,
    PushTags,
    Remotes,
    AddRemote,
    RemoveRemote,
    RemoteUrl,
    Fetch,
    FetchPrune,
    FetchAll,
    Pull,
    Push,
    Clone,
    Init,
    Worktrees,
    AddWorktree,
    RemoveWorktree,
    OpenWorktree,
    Stash,
    ApplyStash,
    PopStash,
    DropStash,
    ShowStash,
}

impl GitCommand {
    pub const ALL: &[Self] = &[
        Self::Amend, Self::AmendMessage, Self::UndoCommit, Self::Merge, Self::Rebase, Self::Continue,
        Self::Abort, Self::Skip, Self::CherryPick, Self::BranchAt, Self::Tags, Self::CreateTag,
        Self::DeleteTag, Self::PushTag, Self::PushTags, Self::Remotes, Self::AddRemote, Self::RemoveRemote,
        Self::RemoteUrl, Self::Fetch, Self::FetchPrune, Self::FetchAll, Self::Pull, Self::Push, Self::Clone,
        Self::Init, Self::Worktrees, Self::AddWorktree, Self::RemoveWorktree, Self::OpenWorktree,
        Self::Stash, Self::ApplyStash, Self::PopStash, Self::DropStash, Self::ShowStash,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Amend => "Git: Commit (Amend)",
            Self::AmendMessage => "Git: Use Last Commit Message",
            Self::UndoCommit => "Git: Undo Last Commit",
            Self::Merge => "Git: Merge Branch into Current...",
            Self::Rebase => "Git: Rebase Current onto...",
            Self::Continue => "Git: Continue Operation",
            Self::Abort => "Git: Abort Operation...",
            Self::Skip => "Git: Skip Rebase Commit",
            Self::CherryPick => "Git: Cherry Pick...",
            Self::BranchAt => "Git: Create Branch Here...",
            Self::Tags => "Git: Tags...",
            Self::CreateTag => "Git: Create Tag...",
            Self::DeleteTag => "Git: Delete Tag...",
            Self::PushTag => "Git: Push Tag...",
            Self::PushTags => "Git: Push All Tags...",
            Self::Remotes => "Git: Remotes...",
            Self::AddRemote => "Git: Add Remote...",
            Self::RemoveRemote => "Git: Remove Remote...",
            Self::RemoteUrl => "Git: Change Remote URL...",
            Self::Fetch => "Git: Fetch",
            Self::FetchPrune => "Git: Fetch (Prune)",
            Self::FetchAll => "Git: Fetch All",
            Self::Pull => "Git: Pull",
            Self::Push => "Git: Push",
            Self::Clone => "Git: Clone...",
            Self::Init => "Git: Initialize Repository",
            Self::Worktrees => "Git: Worktrees...",
            Self::AddWorktree => "Git: Add Worktree...",
            Self::RemoveWorktree => "Git: Remove Worktree...",
            Self::OpenWorktree => "Git: Open Worktree...",
            Self::Stash => "Git: Stash Changes",
            Self::ApplyStash => "Git: Apply Stash...",
            Self::PopStash => "Git: Pop Stash...",
            Self::DropStash => "Git: Drop Stash...",
            Self::ShowStash => "Git: Show Stash Changes...",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Global,
    Editor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub command: Command,
    pub shortcut: KeyboardShortcut,
    pub chord: Option<KeyboardShortcut>,
    pub scope: Scope,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    key: String,
    command: String,
}

fn defaults() -> Vec<Binding> {
    use Command::*;
    const C: Modifiers = Modifiers::COMMAND;
    const NONE: Modifiers = Modifiers::NONE;
    let cs = C | Modifiers::SHIFT;
    let ca = C | Modifiers::ALT;
    let key = KeyboardShortcut::new;
    let g = |m, k, c| Binding { command: c, shortcut: key(m, k), chord: None, scope: Scope::Global };
    let e = |m, k, c| Binding { command: c, shortcut: key(m, k), chord: None, scope: Scope::Editor };
    let k = |m, k, c| Binding { command: c, shortcut: key(C, Key::K), chord: Some(key(m, k)), scope: Scope::Global };
    let t = |m, k, c| Binding { command: c, shortcut: key(C, Key::Semicolon), chord: Some(key(m, k)), scope: Scope::Global };
    let sa = Modifiers::SHIFT | Modifiers::ALT;
    vec![
        t(NONE, Key::A, RunAllTests),
        t(NONE, Key::E, RunFailedTests),
        t(NONE, Key::C, RunTestAtCursor),
        t(C, Key::C, DebugTestAtCursor),
        t(NONE, Key::L, RerunLastTestRun),
        t(C, Key::X, CancelTestRun),
        k(C, Key::F, FormatSelection),
        k(C, Key::I, ShowHover),
        g(cs, Key::M, ShowProblems),
        g(C, Key::T, GoToWorkspaceSymbol),
        e(cs, Key::Space, TriggerParameterHints),
        e(sa, Key::F, FormatDocument),
        e(sa, Key::O, OrganizeImports),
        e(sa, Key::H, ShowCallHierarchy),
        e(C, Key::F12, GoToImplementation),
        e(Modifiers::SHIFT, Key::F12, FindReferences),
        e(Modifiers::ALT, Key::F12, PeekDefinition),
        e(Modifiers::ALT, Key::ArrowLeft, GoBack),
        e(Modifiers::ALT, Key::ArrowRight, GoForward),
        g(cs, Key::T, ReopenClosed),
        g(sa, Key::F5, PrevChange),
        g(Modifiers::ALT, Key::F5, NextChange),
        e(Modifiers::SHIFT, Key::F7, PrevChange),
        e(NONE, Key::F7, NextChange),
        e(sa, Key::F3, ShowPrevChange),
        e(Modifiers::ALT, Key::F3, ShowNextChange),
        k(NONE, Key::Enter, KeepEditor),
        e(C, Key::Space, TriggerSuggest),
        e(C, Key::Period, QuickFix),
        e(NONE, Key::F12, GoToDefinition),
        e(NONE, Key::F2, RenameSymbol),
        k(C, Key::Num0, FoldAll),
        k(C, Key::J, UnfoldAll),
        e(Modifiers::ALT, Key::ArrowUp, MoveLineUp),
        e(Modifiers::ALT, Key::ArrowDown, MoveLineDown),
        e(sa, Key::ArrowUp, CopyLineUp),
        e(sa, Key::ArrowDown, CopyLineDown),
        e(sa, Key::A, ToggleBlockComment),
        e(sa, Key::ArrowRight, ExpandSelection),
        e(sa, Key::ArrowLeft, ShrinkSelection),
        e(cs, Key::L, SelectAllOccurrences),
        e(C, Key::U, CursorUndo),
        e(cs, Key::K, DeleteLine),
        e(cs, Key::Enter, InsertLineAbove),
        e(C, Key::Enter, InsertLineBelow),
        e(cs, Key::OpenBracket, Fold),
        e(cs, Key::CloseBracket, Unfold),
        k(C, Key::O, OpenFolder),
        k(C, Key::S, OpenKeybindings),
        k(C, Key::T, ToggleTheme),
        k(NONE, Key::S, SaveAll),
        k(NONE, Key::F, CloseFolder),
        g(cs, Key::S, SaveAs),
        g(cs, Key::P, ShowCommands),
        g(cs, Key::F, FindInFiles),
        g(cs, Key::O, GoToSymbol),
        g(cs, Key::E, ShowExplorer),
        g(cs, Key::G, ShowSourceControl),
        g(cs, Key::D, ShowRunAndDebug),
        g(cs, Key::F5, RestartDebugging),
        g(C, Key::F5, RunWithoutDebugging),
        g(Modifiers::SHIFT, Key::F5, StopDebugging),
        g(Modifiers::SHIFT, Key::F11, StepOut),
        g(NONE, Key::F6, PauseDebugging),
        g(NONE, Key::F9, ToggleBreakpoint),
        g(NONE, Key::F10, StepOver),
        g(NONE, Key::F11, StepInto),
        g(cs, Key::Backtick, NewTerminal),
        g(cs, Key::Tab, PrevEditor),
        g(ca, Key::B, ToggleAi),
        e(cs, Key::Z, Redo),
        e(ca, Key::ArrowUp, AddCursorAbove),
        e(ca, Key::ArrowDown, AddCursorBelow),
        g(C, Key::N, NewFile),
        g(C, Key::O, OpenFile),
        g(C, Key::P, GoToFile),
        g(C, Key::S, Save),
        g(C, Key::W, CloseEditor),
        g(C, Key::Comma, OpenSettings),
        g(C, Key::B, ToggleSidebar),
        g(C, Key::J, TogglePanel),
        g(C, Key::Backtick, TogglePanel),
        g(C, Key::Backslash, SplitEditor),
        g(C, Key::Tab, NextEditor),
        g(C, Key::Equals, ZoomIn),
        g(C, Key::Minus, ZoomOut),
        g(C, Key::Num0, ZoomReset),
        g(NONE, Key::F5, StartDebugging),
        g(cs, Key::B, RunBuildTask),
        g(Modifiers::ALT, Key::Z, ToggleWordWrap),
        e(C, Key::Z, Undo),
        e(C, Key::Y, Redo),
        e(C, Key::A, SelectAll),
        e(C, Key::F, Find),
        e(C, Key::H, Replace),
        e(C, Key::G, GoToLine),
        e(C, Key::D, SelectNextOccurrence),
        e(C, Key::Slash, ToggleComment),
    ]
}

pub fn all() -> Vec<Command> {
    use Command::*;
    let mut commands = vec![
        NewFile, OpenFile, OpenFolder, Save, SaveAs, SaveAll, CloseEditor, CloseFolder, OpenSettings, OpenSettingsJson,
        OpenKeybindings, OpenSnippets, CompareFile, CompareClipboard, CompareSaved, SelectForCompare, CompareWithSelected,
        SelectTheme, ImportTheme, BrowseThemes, SelectIconTheme, ImportIconTheme, BrowseIconThemes, Exit,
        Undo, Redo, Cut, Copy, Paste, Find, Replace, ToggleComment, ToggleBlockComment, EmmetExpand, MoveLineUp, MoveLineDown,
        CopyLineUp, CopyLineDown, DeleteLine, JoinLines, InsertLineAbove, InsertLineBelow, Fold, Unfold, FoldAll, UnfoldAll,
        RenameSymbol, QuickFix, OrganizeImports, FormatDocument, FormatSelection,
        SelectAll, AddCursorAbove, AddCursorBelow, SelectNextOccurrence, SelectAllOccurrences, ExpandSelection, ShrinkSelection, CursorUndo,
        ShowExplorer, ShowSourceControl, ShowRunAndDebug, ShowTesting, ToggleSidebar, TogglePanel, ToggleAi, SplitEditor, ToggleMinimap, ToggleWordWrap,
        ToggleTheme, ZoomIn, ZoomOut, ZoomReset, ShowProblems, RefreshExplorer, ToggleInlineDiff,
        GoToFile, ShowCommands, FindInFiles, GoToSymbol, GoToLine, NextEditor, PrevEditor, GoBack, GoForward, ReopenClosed,
        KeepEditor, NextChange, PrevChange, ShowNextChange, ShowPrevChange, GoToDefinition, PeekDefinition, GoToDeclaration, GoToTypeDefinition, GoToImplementation, FindReferences,
        GoToWorkspaceSymbol, ShowCallHierarchy, ShowTypeHierarchy,
        ShowHover, TriggerSuggest, TriggerParameterHints, RestartLanguageServers,
        RunFile, StartDebugging, RunWithoutDebugging, StopDebugging, RestartDebugging, PauseDebugging, StepOver, StepInto, StepOut,
        ToggleBreakpoint, EditBreakpoint, RemoveAllBreakpoints, AddWatch, AddConfiguration,
        RunTaskPicker, RunBuildTask, RunTestTask, RerunLastTask, TerminateTask, ShowRunningTasks, ConfigureDefaultBuildTask,
        OpenTasksJson, RunAllTests, RunFailedTests, RunTestAtCursor, DebugTestAtCursor, RerunLastTestRun, CancelTestRun, RefreshTests,
        ShowTestOutput, RunAllTestsWithCoverage, ToggleCoverageDisplay, NewTerminal, SplitTerminal, KillTerminal, GitSync, ToggleInlineBlame, OpenLineChanges,
        StageHunk, RevertHunk, UnstageHunk, StageSelectedRanges, RevertSelectedRanges, UnstageSelectedRanges,
    ];
    commands.extend(GitCommand::ALL.iter().copied().map(Git));
    commands.push(About);
    commands
}

fn parse_key(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "cmd" | "meta" | "win" => modifiers |= Modifiers::COMMAND,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            name => {
                let named = |k: &&Key| k.name().eq_ignore_ascii_case(name) || k.symbol_or_name().eq_ignore_ascii_case(name);
                key = Some(*Key::ALL.iter().find(named)?);
            }
        }
    }
    Some(KeyboardShortcut::new(modifiers, key?))
}

pub fn key_text(shortcut: &KeyboardShortcut) -> String {
    let m = shortcut.modifiers;
    let mut text = String::new();
    for (on, name) in [(m.command || m.ctrl, "ctrl+"), (m.shift, "shift+"), (m.alt, "alt+")] {
        if on {
            text.push_str(name);
        }
    }
    text + &shortcut.logical_key.name().to_lowercase()
}

fn build(mut bindings: Vec<Binding>, user: &[Entry]) -> Vec<Binding> {
    let commands = all();
    let user: Vec<(Command, &str)> =
        user.iter().filter_map(|e| Some((commands.iter().find(|c| c.id() == e.command)?.clone(), e.key.as_str()))).collect();
    let editor: Vec<Command> = bindings.iter().filter(|b| b.scope == Scope::Editor).map(|b| b.command.clone()).collect();
    bindings.retain(|b| !user.iter().any(|(c, _)| *c == b.command));
    for (command, keys) in user {
        let mut keys = keys.split_whitespace().map(parse_key);
        if let (Some(Some(shortcut)), chord) = (keys.next(), keys.next()) {
            let scope = if editor.contains(&command) { Scope::Editor } else { Scope::Global };
            bindings.push(Binding { command, shortcut, chord: chord.flatten(), scope });
        }
    }
    let weight = |b: &Binding| {
        let m = b.shortcut.modifiers;
        b.chord.is_none() as u8 * 4 + 3 - (m.command || m.ctrl) as u8 - m.shift as u8 - m.alt as u8
    };
    bindings.sort_by_key(weight);
    bindings
}

pub fn path() -> PathBuf {
    crate::settings::dir().join("keybindings.json")
}

pub fn write_defaults() {
    let entries: Vec<Entry> = defaults()
        .iter()
        .map(|b| Entry {
            key: [Some(b.shortcut), b.chord].iter().flatten().map(key_text).collect::<Vec<_>>().join(" "),
            command: b.command.id(),
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::create_dir_all(crate::settings::dir());
        let _ = std::fs::write(path(), text);
    }
}

static BINDINGS: Mutex<Option<Arc<Vec<Binding>>>> = Mutex::new(None);

pub fn bindings() -> Arc<Vec<Binding>> {
    BINDINGS
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            let user = match std::fs::read_to_string(path()) {
                Ok(text) if !cfg!(test) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
                    crate::ui::notify(format!("keybindings.json could not be read, default keys are in use: {e}"));
                    vec![]
                }),
                _ => vec![],
            };
            Arc::new(build(defaults(), &user))
        })
        .clone()
}

pub fn reload() {
    *BINDINGS.lock().unwrap() = None;
}

pub fn rebind(command: &Command, key: Option<String>) {
    let mut entries: Vec<Entry> = match std::fs::read_to_string(path()) {
        Ok(text) => match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
            Ok(entries) => entries,
            Err(error) => return crate::ui::notify(format!("keybindings.json could not be read, so it was not changed: {error}")),
        },
        Err(_) => vec![],
    };
    entries.retain(|e| e.command != command.id());
    entries.extend(key.map(|key| Entry { key, command: command.id() }));
    if let Ok(text) = serde_json::to_string_pretty(&entries) {
        let _ = std::fs::create_dir_all(crate::settings::dir());
        let _ = std::fs::write(path(), text);
    }
    reload();
}

pub fn shortcut_text(ctx: &egui::Context, command: &Command) -> Option<String> {
    let bindings = bindings();
    let binding = bindings.iter().find(|b| &b.command == command)?;
    Some([Some(binding.shortcut), binding.chord].iter().flatten().map(|s| ctx.format_shortcut(s)).collect::<Vec<_>>().join(" "))
}

impl Command {
    pub fn id(&self) -> String {
        if let Self::Git(op) | Self::GitAt(op, _) = self {
            return format!("git{op:?}");
        }
        let name = format!("{self:?}");
        name[..1].to_lowercase() + &name[1..]
    }

    pub fn category(&self) -> &'static str {
        use Command::*;
        match self {
            NewFile | OpenFile | OpenFolder | OpenFolderIn(_) | OpenPath(_) | Save | SaveAs | SaveAll | CloseEditor
            | CloseFolder | OpenSettings | OpenSettingsJson | OpenKeybindings | OpenSnippets | PathChanged(..) | Exit | ImportTheme
            | BrowseThemes | SelectTheme | ImportThemeFile(_) | SetTheme(_) | ImportIconTheme | BrowseIconThemes | SelectIconTheme
            | ImportIconThemeFile(_) | SetIconTheme(_) => "File",
            Undo | Redo | Cut | Copy | Paste | Find | Replace | ToggleComment | ToggleBlockComment | EmmetExpand | MoveLineUp
            | MoveLineDown | CopyLineUp | CopyLineDown | DeleteLine | JoinLines | InsertLineAbove | InsertLineBelow | Fold
            | Unfold | FoldAll | UnfoldAll | RenameSymbol | QuickFix | OrganizeImports | FormatDocument | FormatSelection => "Edit",
            SelectAll | AddCursorAbove | AddCursorBelow | SelectNextOccurrence | SelectAllOccurrences | ExpandSelection
            | ShrinkSelection | CursorUndo => "Selection",
            ShowExplorer | ShowSourceControl | ShowRunAndDebug | ShowTesting | ToggleSidebar | TogglePanel | ToggleAi | SplitEditor | ToggleMinimap
            | ToggleWordWrap | ToggleTheme | ZoomIn | ZoomOut | ZoomReset | ShowProblems | RefreshExplorer | ToggleInlineDiff => "View",
            GoToFile | ShowCommands | FindInFiles | GoToSymbol | GoToLine | GoTo(..) | NextEditor | PrevEditor | GoBack
            | GoForward | ReopenClosed | KeepEditor | NextChange | PrevChange | ShowNextChange | ShowPrevChange | GoToDefinition | PeekDefinition | GoToDeclaration | GoToTypeDefinition
            | GoToImplementation | FindReferences | GoToWorkspaceSymbol | ShowCallHierarchy | ShowTypeHierarchy => "Go",
            ShowHover | TriggerSuggest | TriggerParameterHints | RestartLanguageServers => "Language",
            RunFile | RunTask(_) | RunTaskPicker | RunBuildTask | RunTestTask | RerunLastTask | TerminateTask | ShowRunningTasks
            | ConfigureDefaultBuildTask | OpenTasksJson | RunTaskLabel(_) | TerminateTaskLabel(_) | RevealTask(_) | SetDefaultBuild(_)
            | StartDebugging | RunWithoutDebugging | StopDebugging | RestartDebugging | PauseDebugging | StepOver
            | StepInto | StepOut | ToggleBreakpoint | EditBreakpoint | RemoveAllBreakpoints | AddWatch | AddConfiguration | RunAllTests
            | RunFailedTests | RunTestAtCursor | DebugTestAtCursor | RerunLastTestRun | CancelTestRun | RefreshTests | ShowTestOutput
            | RunAllTestsWithCoverage | ToggleCoverageDisplay => "Run",
            NewTerminal | NewTerminalWith(_) | SplitTerminal | KillTerminal => "Terminal",
            CompareFile | CompareClipboard | CompareSaved | SelectForCompare | CompareWithSelected | SelectComparePath(_) | ComparePath(_) => "File",
            OpenDiff(..) | GitSync | Git(_) | GitAt(..) | ToggleInlineBlame | OpenLineChanges | RestoreSnapshot(..) | StageHunk | RevertHunk
            | UnstageHunk | StageSelectedRanges | RevertSelectedRanges | UnstageSelectedRanges => "Source Control",
            About => "Help",
        }
    }

    pub fn label(&self) -> &'static str {
        use Command::*;
        match self {
            NewFile => "New File",
            OpenFile => "Open File...",
            OpenFolder | OpenFolderIn(_) => "Open Folder...",
            OpenPath(_) => "Open",
            OpenDiff(..) => "Open Changes",
            CompareFile => "Compare Active File With…",
            CompareClipboard => "Compare Active File with Clipboard",
            CompareSaved => "Compare Active File with Saved",
            SelectForCompare | SelectComparePath(_) => "Select for Compare",
            CompareWithSelected | ComparePath(_) => "Compare with Selected",
            ToggleInlineBlame => "Git: Toggle Inline Blame",
            OpenLineChanges => "Git: Open Changes of Current Line's Commit",
            RestoreSnapshot(..) => "Restore Contents",
            Save => "Save",
            SaveAs => "Save As...",
            SaveAll => "Save All",
            CloseEditor => "Close Editor",
            CloseFolder => "Close Folder",
            OpenSettings => "Settings",
            OpenSettingsJson => "Open settings.json",
            OpenKeybindings => "Keyboard Shortcuts (keybindings.json)",
            ImportTheme => "Preferences: Import VS Code Theme...",
            BrowseThemes => "Preferences: Browse Installed VS Code Themes...",
            SelectTheme => "Preferences: Color Theme",
            ImportThemeFile(_) => "Preferences: Import VS Code Theme",
            SetTheme(_) => "Preferences: Set Color Theme",
            ImportIconTheme => "Preferences: Import VS Code Icon Theme...",
            BrowseIconThemes => "Preferences: Browse Installed VS Code Icon Themes...",
            SelectIconTheme => "Preferences: File Icon Theme",
            ImportIconThemeFile(_) => "Preferences: Import VS Code Icon Theme",
            SetIconTheme(_) => "Preferences: Set File Icon Theme",
            Exit => "Exit",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            Find => "Find",
            Replace => "Replace",
            ToggleComment => "Toggle Line Comment",
            ToggleBlockComment => "Toggle Block Comment",
            EmmetExpand => "Emmet: Expand Abbreviation",
            OpenSnippets => "Snippets (snippets.json)",
            MoveLineUp => "Move Line Up",
            MoveLineDown => "Move Line Down",
            CopyLineUp => "Copy Line Up",
            CopyLineDown => "Copy Line Down",
            DeleteLine => "Delete Line",
            JoinLines => "Join Lines",
            InsertLineAbove => "Insert Line Above",
            InsertLineBelow => "Insert Line Below",
            Fold => "Fold",
            Unfold => "Unfold",
            FoldAll => "Fold All",
            UnfoldAll => "Unfold All",
            SelectAll => "Select All",
            AddCursorAbove => "Add Cursor Above",
            AddCursorBelow => "Add Cursor Below",
            SelectNextOccurrence => "Add Next Occurrence",
            SelectAllOccurrences => "Select All Occurrences",
            ExpandSelection => "Expand Selection",
            ShrinkSelection => "Shrink Selection",
            CursorUndo => "Cursor Undo",
            ShowExplorer => "Explorer",
            ShowSourceControl => "Source Control",
            ShowRunAndDebug => "Run and Debug",
            ShowTesting => "Test: Focus on Test Explorer View",
            ToggleSidebar => "Toggle Side Bar",
            TogglePanel => "Toggle Panel",
            ToggleAi => "Toggle AI Panel",
            SplitEditor => "Split Editor Right",
            ToggleMinimap => "Toggle Minimap",
            ToggleWordWrap => "Toggle Word Wrap",
            ToggleTheme => "Toggle Light/Dark Theme",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomReset => "Reset Zoom",
            GoToFile => "Go to File...",
            ShowCommands => "Show All Commands",
            FindInFiles => "Find in Files",
            GoToSymbol => "Go to Symbol in Editor...",
            GoToLine => "Go to Line...",
            GoTo(..) => "Go to Position",
            NextEditor => "Next Editor",
            PrevEditor => "Previous Editor",
            GoBack => "Go Back",
            GoForward => "Go Forward",
            ReopenClosed => "Reopen Closed Editor",
            KeepEditor => "Keep Editor Open",
            NextChange => "Go to Next Change",
            PrevChange => "Go to Previous Change",
            ShowNextChange => "Show Next Change",
            ShowPrevChange => "Show Previous Change",
            ToggleInlineDiff => "Toggle Inline Diff View",
            StageHunk => "Stage Hunk",
            RevertHunk => "Revert Hunk",
            UnstageHunk => "Unstage Hunk",
            StageSelectedRanges => "Stage Selected Ranges",
            RevertSelectedRanges => "Revert Selected Ranges",
            UnstageSelectedRanges => "Unstage Selected Ranges",
            GoToDefinition => "Go to Definition",
            PeekDefinition => "Peek Definition",
            GoToDeclaration => "Go to Declaration",
            GoToTypeDefinition => "Go to Type Definition",
            GoToImplementation => "Go to Implementations",
            FindReferences => "Find All References",
            GoToWorkspaceSymbol => "Go to Symbol in Workspace...",
            ShowCallHierarchy => "Show Call Hierarchy",
            ShowTypeHierarchy => "Show Type Hierarchy",
            ShowHover => "Show Hover",
            TriggerSuggest => "Trigger Suggest",
            TriggerParameterHints => "Trigger Parameter Hints",
            RenameSymbol => "Rename Symbol",
            QuickFix => "Quick Fix...",
            OrganizeImports => "Organize Imports",
            FormatDocument => "Format Document",
            FormatSelection => "Format Selection",
            ShowProblems => "Problems",
            RestartLanguageServers => "Restart Language Servers",
            NewTerminal => "New Terminal",
            NewTerminalWith(_) => "New Terminal",
            SplitTerminal => "Split Terminal",
            KillTerminal => "Kill Terminal",
            RunFile => "Run File",
            RunTask(_) => "Run Task",
            RunTaskPicker => "Tasks: Run Task...",
            RunBuildTask => "Tasks: Run Build Task",
            RunTestTask => "Tasks: Run Test Task",
            RerunLastTask => "Tasks: Rerun Last Task",
            TerminateTask => "Tasks: Terminate Task...",
            ShowRunningTasks => "Tasks: Show Running Tasks",
            ConfigureDefaultBuildTask => "Tasks: Configure Default Build Task...",
            OpenTasksJson => "Tasks: Open tasks.json",
            RunTaskLabel(_) => "Tasks: Run Task",
            TerminateTaskLabel(_) => "Tasks: Terminate Task",
            RevealTask(_) => "Tasks: Show Task",
            SetDefaultBuild(_) => "Tasks: Set Default Build Task",
            Git(op) | GitAt(op, _) => op.label(),
            StartDebugging => "Start Debugging",
            RunWithoutDebugging => "Run Without Debugging",
            StopDebugging => "Stop Debugging",
            RestartDebugging => "Restart Debugging",
            PauseDebugging => "Pause",
            StepOver => "Step Over",
            StepInto => "Step Into",
            StepOut => "Step Out",
            ToggleBreakpoint => "Toggle Breakpoint",
            EditBreakpoint => "Edit Breakpoint Condition...",
            RemoveAllBreakpoints => "Remove All Breakpoints",
            AddWatch => "Debug: Add Watch Expression",
            AddConfiguration => "Debug: Add Configuration...",
            RunAllTests => "Test: Run All Tests",
            RunFailedTests => "Test: Run Failed Tests",
            RunTestAtCursor => "Test: Run Test at Cursor",
            DebugTestAtCursor => "Test: Debug Test at Cursor",
            RerunLastTestRun => "Test: Rerun Last Run",
            CancelTestRun => "Test: Cancel Test Run",
            RefreshTests => "Test: Refresh Tests",
            ShowTestOutput => "Test: Show Output",
            RunAllTestsWithCoverage => "Test: Run All Tests with Coverage",
            ToggleCoverageDisplay => "Test: Toggle Coverage Display",
            GitSync => "Synchronize Changes",
            RefreshExplorer => "Refresh Explorer",
            PathChanged(..) => "Path Changed",
            About => "About Halo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_grouped_in_category_order() {
        let mut groups: Vec<_> = all().iter().map(Command::category).collect();
        groups.dedup();
        assert_eq!(groups, ["File", "Edit", "Selection", "View", "Go", "Language", "Run", "Terminal", "Source Control", "Help"]);
    }

    #[test]
    fn keys_round_trip_and_user_entries_rebind() {
        for binding in defaults() {
            assert_eq!(parse_key(&key_text(&binding.shortcut)), Some(binding.shortcut));
        }
        assert_eq!(parse_key("Ctrl+Shift+`"), Some(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Backtick)));
        assert_eq!(parse_key("ctrl+nothing"), None);
        assert_eq!(Command::OpenFolder.id(), "openFolder");

        let entry = |key: &str, command: &str| Entry { key: key.into(), command: command.into() };
        let user = [entry("ctrl+k ctrl+u", "undo"), entry("", "save"), entry("f9", "noSuchCommand"), entry("alt+p", "goToFile")];
        let built = build(defaults(), &user);
        let of = |c: Command| built.iter().filter(|b| b.command == c).cloned().collect::<Vec<_>>();
        let undo = of(Command::Undo);
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].chord, parse_key("ctrl+u"));
        assert_eq!(undo[0].scope, Scope::Editor);
        assert!(of(Command::Save).is_empty());
        assert_eq!(of(Command::GoToFile)[0].shortcut, parse_key("alt+p").unwrap());
        assert_eq!(of(Command::Redo).len(), 2);
        let first_single = built.iter().position(|b| b.chord.is_none()).unwrap();
        assert!(built[first_single..].iter().all(|b| b.chord.is_none()));
        let pos = |c: Command| built.iter().position(|b| b.command == c).unwrap();
        assert!(pos(Command::GoToSymbol) < pos(Command::OpenFile));
    }
}
