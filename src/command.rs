use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    AddFolderToWorkspace,
    RemoveFolderFromWorkspace,
    RemoveWorkspaceFolder(PathBuf),
    AddWorkspaceFolder(PathBuf),
    MoveWorkspaceFolder(usize, usize),
    SaveWorkspaceAs,
    OpenWorkspaceFromFile,
    OpenWorkspace(PathBuf),
    CloseWorkspace,
    NewTerminalIn(PathBuf),
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
    AddCursorsToLineEnds,
    SelectNextOccurrence,
    MoveSelectionToNextFindMatch,
    SelectAllOccurrences,
    ExpandSelection,
    ShrinkSelection,
    SelectToBracket,
    CursorUndo,
    ToggleColumnSelection,
    CursorColumnSelectLeft,
    CursorColumnSelectRight,
    CursorColumnSelectUp,
    CursorColumnSelectDown,

    ShowExplorer,
    ShowSourceControl,
    ShowRunAndDebug,
    ShowTesting,
    ToggleSidebar,
    TogglePanel,
    ToggleAi,
    SplitEditor,
    SplitEditorDown,
    SplitEditorLeft,
    SplitEditorUp,
    LayoutSingle,
    LayoutTwoColumns,
    LayoutThreeColumns,
    LayoutTwoRows,
    LayoutThreeRows,
    LayoutGrid,
    LayoutTwoRowsRight,
    LayoutTwoColumnsBottom,
    ToggleMaximizeGroup,
    ResetGroupSizes,
    FocusGroupLeft,
    FocusGroupRight,
    FocusGroupUp,
    FocusGroupDown,
    MoveEditorLeft,
    MoveEditorRight,
    MoveEditorUp,
    MoveEditorDown,
    CloseGroup,
    MoveEditorToNewWindow,
    CopyEditorToNewWindow,
    MoveEditorToMainWindow,
    NewWindow,
    OpenFolderInNewWindow,
    DuplicateWorkspace,
    ToggleMinimap,
    ToggleWordWrap,
    ToggleTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleFullScreen,
    ToggleZenMode,
    ToggleCenteredLayout,
    ToggleSidebarPosition,
    ToggleSecondarySidebar,
    MovePanelLeft,
    MovePanelRight,
    MovePanelBottom,
    MovePanelTop,
    TogglePanelPosition,
    ToggleMaximizedPanel,
    AlignPanelCenter,
    AlignPanelJustify,
    AlignPanelLeft,
    AlignPanelRight,
    ActivityBarSide,
    ActivityBarTop,
    ActivityBarHidden,
    MoveView,
    MoveViewTo(crate::workbench::View),
    MoveViewPlace(crate::workbench::View, crate::workbench::Place),
    ResetViewLocations,

    GoToFile,
    ShowCommands,
    FindInFiles,
    GoToSymbol,
    GoToLine,
    GoToBracket,
    GoTo(usize, usize),
    NextEditor,
    PrevEditor,
    GoBack,
    GoForward,
    GoToLastEditLocation,
    ReopenClosed,
    KeepEditor,
    NextChange,
    PrevChange,
    ShowNextChange,
    ShowPrevChange,
    ToggleInlineDiff,
    MarkdownPreview,
    MarkdownPreviewSide,
    StageHunk,
    RevertHunk,
    UnstageHunk,
    StageSelectedRanges,
    RevertSelectedRanges,
    UnstageSelectedRanges,
    MergeOpen,
    OpenMerge(PathBuf),
    MergeAcceptIncoming,
    MergeAcceptCurrent,
    MergeAcceptBoth,
    MergeAcceptAllIncoming,
    MergeAcceptAllCurrent,
    MergeNextConflict,
    MergePrevConflict,
    MergeToggleBase,
    MergeComplete,

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
    ScrollToPrevMark,
    ScrollToNextMark,
    RunRecentLine,
    RunCommandLine(String),
    CopyLastCommandOutput,
    RenameTerminal,
    ChangeTerminalColor,
    SetTerminalColor(Option<usize>),
    NewTerminalInEditor,
    MoveTerminalToEditor,
    MoveTerminalToPanel,
    ClearRestoredHistory,

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

    ShowOutput,
    ShowOutputChannels,
    ClearOutput,
    OpenOutputInEditor,
    SelectOutputChannel(String),
    ShowNotifications,
    ClearNotifications,
    ToggleDoNotDisturb,
    FocusNotification,
    FocusNextRegion,
    FocusPrevRegion,
    ToggleTabFocus,
    ShowAccessibilityHelp,
    AnnounceCursorPosition,
    ToggleScreenReaderMode,

    GitSync,
    Git(GitCommand),
    GitAt(GitCommand, String),
    RefreshExplorer,
    CutFiles,
    CopyFiles,
    PasteFiles,
    FilterExplorer,
    ToggleFileNesting,
    FocusOpenEditors,
    CloseAllEditors,
    ShowTab(usize, usize),
    CloseTab(usize, usize),
    PathChanged(PathBuf, Option<PathBuf>),
    About,
    InstallInPath,
    ConfirmInstallInPath,
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

#[derive(Clone, Debug)]
pub struct Binding {
    pub command: Command,
    pub shortcut: KeyboardShortcut,
    pub chord: Option<KeyboardShortcut>,
    pub scope: Scope,
    pub when: String,
    pub user: Option<usize>,
    cond: Option<Arc<crate::when::Expr>>,
}

impl Binding {
    pub fn active(&self, keys: &Map<String, Value>) -> bool {
        self.cond.as_ref().is_none_or(|c| c.eval(keys))
    }

    pub fn keys(&self) -> Vec<KeyboardShortcut> {
        [Some(self.shortcut), self.chord].into_iter().flatten().collect()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub key: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

pub const SEND_TO_SHELL: &str = "!terminalFocus || !config.terminal.integrated.sendKeybindingsToShell";
const NOT_TEXT: &str = "!editorTextFocus";
const TEXT: &str = "editorTextFocus";

fn defaults() -> Vec<Binding> {
    use Command::*;
    const C: Modifiers = Modifiers::COMMAND;
    const NONE: Modifiers = Modifiers::NONE;
    let cs = C | Modifiers::SHIFT;
    let ca = C | Modifiers::ALT;
    let key = KeyboardShortcut::new;
    let bind = |shortcut, chord, scope, command| Binding { command, shortcut, chord, scope, when: String::new(), user: None, cond: None };
    let g = |m, k, c| bind(key(m, k), None, Scope::Global, c);
    let e = |m, k, c| bind(key(m, k), None, Scope::Editor, c);
    let k = |m, k, c| bind(key(C, Key::K), Some(key(m, k)), Scope::Global, c);
    let t = |m, k, c| bind(key(C, Key::Semicolon), Some(key(m, k)), Scope::Global, c);
    let sa = Modifiers::SHIFT | Modifiers::ALT;
    let csa = cs | Modifiers::ALT;
    let mut list = vec![
        g(csa, Key::ArrowLeft, CursorColumnSelectLeft),
        g(csa, Key::ArrowRight, CursorColumnSelectRight),
        g(csa, Key::ArrowUp, CursorColumnSelectUp),
        g(csa, Key::ArrowDown, CursorColumnSelectDown),
        t(NONE, Key::A, RunAllTests),
        t(NONE, Key::E, RunFailedTests),
        t(NONE, Key::C, RunTestAtCursor),
        t(C, Key::C, DebugTestAtCursor),
        t(NONE, Key::L, RerunLastTestRun),
        t(C, Key::X, CancelTestRun),
        k(C, Key::F, FormatSelection),
        k(NONE, Key::V, MarkdownPreviewSide),
        e(cs, Key::V, MarkdownPreview),
        k(C, Key::I, ShowHover),
        g(cs, Key::M, ShowProblems),
        g(cs, Key::U, ShowOutput),
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
        k(C, Key::D, MoveSelectionToNextFindMatch),
        k(C, Key::Q, GoToLastEditLocation),
        e(Modifiers::ALT, Key::ArrowUp, MoveLineUp),
        e(Modifiers::ALT, Key::ArrowDown, MoveLineDown),
        e(sa, Key::ArrowUp, CopyLineUp),
        e(sa, Key::ArrowDown, CopyLineDown),
        e(sa, Key::A, ToggleBlockComment),
        e(sa, Key::ArrowRight, ExpandSelection),
        e(sa, Key::ArrowLeft, ShrinkSelection),
        e(sa, Key::I, AddCursorsToLineEnds),
        e(cs, Key::Pipe, GoToBracket),
        e(cs, Key::Backslash, GoToBracket),
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
        k(NONE, Key::Z, ToggleZenMode),
        g(cs, Key::S, SaveAs),
        g(cs, Key::P, ShowCommands),
        g(cs, Key::F, FindInFiles),
        g(ca, Key::F, FilterExplorer),
        g(cs, Key::O, GoToSymbol),
        g(cs, Key::E, ShowExplorer),
        g(cs, Key::G, ShowSourceControl),
        g(cs, Key::D, ShowRunAndDebug),
        g(cs, Key::F5, RestartDebugging),
        g(C, Key::F5, RunWithoutDebugging),
        g(Modifiers::SHIFT, Key::F5, StopDebugging),
        g(Modifiers::SHIFT, Key::F11, StepOut),
        g(NONE, Key::F6, PauseDebugging),
        g(NONE, Key::F6, FocusNextRegion),
        g(Modifiers::SHIFT, Key::F6, FocusPrevRegion),
        e(C, Key::M, ToggleTabFocus),
        g(Modifiers::ALT, Key::F1, ShowAccessibilityHelp),
        g(NONE, Key::F9, ToggleBreakpoint),
        g(NONE, Key::F10, StepOver),
        g(NONE, Key::F11, StepInto),
        g(NONE, Key::F11, ToggleFullScreen),
        g(cs, Key::Backtick, NewTerminal),
        g(cs, Key::Tab, PrevEditor),
        g(ca, Key::B, ToggleSecondarySidebar),
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
        k(C, Key::Backslash, SplitEditorDown),
        k(C, Key::M, ToggleMaximizeGroup),
        k(C, Key::ArrowLeft, FocusGroupLeft),
        k(C, Key::ArrowRight, FocusGroupRight),
        k(C, Key::ArrowUp, FocusGroupUp),
        k(C, Key::ArrowDown, FocusGroupDown),
        k(NONE, Key::O, CopyEditorToNewWindow),
        g(ca, Key::ArrowLeft, MoveEditorLeft),
        g(ca, Key::ArrowRight, MoveEditorRight),
        g(ca, Key::ArrowUp, MoveEditorUp),
        g(ca, Key::ArrowDown, MoveEditorDown),
        g(cs, Key::N, NewWindow),
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
    ];
    let shell = crate::when::parse(SEND_TO_SHELL).ok().map(Arc::new);
    for b in list.iter_mut().filter(|b| b.scope == Scope::Global && b.shortcut.modifiers == C) {
        if [Key::K, Key::B, Key::J, Key::W, Key::N, Key::O, Key::S].contains(&b.shortcut.logical_key) {
            (b.when, b.cond) = (SEND_TO_SHELL.to_owned(), shell.clone());
        }
    }
    for b in list.iter_mut().filter(|b| b.command == PauseDebugging) {
        (b.when, b.cond) = ("debugState == 'running'".to_owned(), crate::when::parse("debugState == 'running'").ok().map(Arc::new));
    }
    let text = crate::when::parse(NOT_TEXT).ok().map(Arc::new);
    for b in list.iter_mut().filter(|b| matches!(b.command, MoveEditorUp | MoveEditorDown)) {
        (b.when, b.cond) = (NOT_TEXT.to_owned(), text.clone());
    }
    let typing = crate::when::parse(TEXT).ok().map(Arc::new);
    for b in list.iter_mut().filter(|b| matches!(b.command, CursorColumnSelectLeft | CursorColumnSelectRight | CursorColumnSelectUp | CursorColumnSelectDown)) {
        (b.when, b.cond) = (TEXT.to_owned(), typing.clone());
    }
    for b in list.iter_mut().filter(|b| b.shortcut == key(NONE, Key::F11)) {
        b.when = if b.command == StepInto { "inDebugMode" } else { "!inDebugMode" }.to_owned();
        b.cond = crate::when::parse(&b.when).ok().map(Arc::new);
    }
    list
}

pub fn all() -> Vec<Command> {
    use Command::*;
    let mut commands = vec![
        NewFile, NewWindow, OpenFile, OpenFolder, OpenFolderInNewWindow, DuplicateWorkspace, Save, SaveAs, SaveAll, CloseEditor, CloseFolder,
        AddFolderToWorkspace, RemoveFolderFromWorkspace, SaveWorkspaceAs, OpenWorkspaceFromFile, CloseWorkspace, OpenSettings, OpenSettingsJson,
        OpenKeybindings, OpenSnippets, CompareFile, CompareClipboard, CompareSaved, SelectForCompare, CompareWithSelected,
        SelectTheme, ImportTheme, BrowseThemes, SelectIconTheme, ImportIconTheme, BrowseIconThemes, CutFiles, CopyFiles, PasteFiles, CloseAllEditors, Exit,
        Undo, Redo, Cut, Copy, Paste, Find, Replace, ToggleComment, ToggleBlockComment, EmmetExpand, MoveLineUp, MoveLineDown,
        CopyLineUp, CopyLineDown, DeleteLine, JoinLines, InsertLineAbove, InsertLineBelow, Fold, Unfold, FoldAll, UnfoldAll,
        RenameSymbol, QuickFix, OrganizeImports, FormatDocument, FormatSelection,
        SelectAll, AddCursorAbove, AddCursorBelow, AddCursorsToLineEnds, SelectNextOccurrence, MoveSelectionToNextFindMatch, SelectAllOccurrences, ExpandSelection, ShrinkSelection,
        SelectToBracket, CursorUndo, ToggleColumnSelection, CursorColumnSelectLeft, CursorColumnSelectRight, CursorColumnSelectUp,
        CursorColumnSelectDown, ShowExplorer, ShowSourceControl, ShowRunAndDebug, ShowTesting, ToggleSidebar, TogglePanel, ToggleAi, SplitEditor, ToggleMinimap, ToggleWordWrap,
        SplitEditorDown, SplitEditorLeft, SplitEditorUp, LayoutSingle, LayoutTwoColumns, LayoutThreeColumns, LayoutTwoRows, LayoutThreeRows,
        LayoutGrid, LayoutTwoRowsRight, LayoutTwoColumnsBottom, ToggleMaximizeGroup, ResetGroupSizes, FocusGroupLeft, FocusGroupRight,
        FocusGroupUp, FocusGroupDown, MoveEditorLeft, MoveEditorRight, MoveEditorUp, MoveEditorDown, CloseGroup, MoveEditorToNewWindow,
        CopyEditorToNewWindow, MoveEditorToMainWindow,
        ToggleTheme, ZoomIn, ZoomOut, ZoomReset, ShowProblems, RefreshExplorer, ToggleInlineDiff, FilterExplorer, ToggleFileNesting, FocusOpenEditors,
        MarkdownPreview, MarkdownPreviewSide, ToggleFullScreen, ToggleZenMode, ToggleCenteredLayout, ToggleSidebarPosition, ToggleSecondarySidebar,
        MovePanelLeft, MovePanelRight, MovePanelBottom, MovePanelTop, TogglePanelPosition, ToggleMaximizedPanel, AlignPanelCenter, AlignPanelJustify,
        AlignPanelLeft, AlignPanelRight, ActivityBarSide, ActivityBarTop, ActivityBarHidden, MoveView, ResetViewLocations,
        GoToFile, ShowCommands, FindInFiles, GoToSymbol, GoToLine, GoToBracket, NextEditor, PrevEditor, GoBack, GoForward, GoToLastEditLocation, ReopenClosed,
        KeepEditor, NextChange, PrevChange, ShowNextChange, ShowPrevChange, GoToDefinition, PeekDefinition, GoToDeclaration, GoToTypeDefinition, GoToImplementation, FindReferences,
        GoToWorkspaceSymbol, ShowCallHierarchy, ShowTypeHierarchy,
        ShowHover, TriggerSuggest, TriggerParameterHints, RestartLanguageServers,
        RunFile, StartDebugging, RunWithoutDebugging, StopDebugging, RestartDebugging, PauseDebugging, StepOver, StepInto, StepOut,
        ToggleBreakpoint, EditBreakpoint, RemoveAllBreakpoints, AddWatch, AddConfiguration,
        RunTaskPicker, RunBuildTask, RunTestTask, RerunLastTask, TerminateTask, ShowRunningTasks, ConfigureDefaultBuildTask,
        OpenTasksJson, RunAllTests, RunFailedTests, RunTestAtCursor, DebugTestAtCursor, RerunLastTestRun, CancelTestRun, RefreshTests,
        ShowTestOutput, RunAllTestsWithCoverage, ToggleCoverageDisplay, NewTerminal, SplitTerminal, KillTerminal, ScrollToPrevMark, ScrollToNextMark, RunRecentLine,
        CopyLastCommandOutput, RenameTerminal, ChangeTerminalColor, NewTerminalInEditor, MoveTerminalToEditor, MoveTerminalToPanel, ClearRestoredHistory,
        GitSync, ToggleInlineBlame, OpenLineChanges,
        StageHunk, RevertHunk, UnstageHunk, StageSelectedRanges, RevertSelectedRanges, UnstageSelectedRanges,
        MergeOpen, MergeAcceptIncoming, MergeAcceptCurrent, MergeAcceptBoth, MergeAcceptAllIncoming, MergeAcceptAllCurrent,
        MergeNextConflict, MergePrevConflict, MergeToggleBase, MergeComplete,
    ];
    commands.extend(GitCommand::ALL.iter().copied().map(Git));
    commands.extend([ShowOutput, ShowOutputChannels, ClearOutput, OpenOutputInEditor, ShowNotifications, ClearNotifications, ToggleDoNotDisturb, FocusNotification]);
    commands.extend([FocusNextRegion, FocusPrevRegion, ToggleTabFocus, ShowAccessibilityHelp, AnnounceCursorPosition, ToggleScreenReaderMode]);
    commands.extend([About, InstallInPath]);
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

fn build(mut bindings: Vec<Binding>, user: &[Entry]) -> (Vec<Binding>, Vec<String>) {
    let commands = all();
    let editor: Vec<Command> = bindings.iter().filter(|b| b.scope == Scope::Editor).map(|b| b.command.clone()).collect();
    let mut errors = vec![];
    for (index, entry) in user.iter().enumerate() {
        let (remove, id) = entry.command.strip_prefix('-').map_or((false, entry.command.as_str()), |id| (true, id));
        let Some(command) = commands.iter().find(|c| c.id() == id) else { continue };
        let name = format!("entry {} (\"{}\" for {})", index + 1, entry.key, entry.command);
        let keys: Option<Vec<KeyboardShortcut>> = entry.key.split_whitespace().map(parse_key).collect();
        let Some(keys) = keys.filter(|k| k.len() <= 2) else {
            errors.push(format!("keybindings.json {name}: the key is not understood, so the entry is not used."));
            continue;
        };
        let when = entry.when.trim().to_owned();
        let cond = match when.is_empty() {
            true => None,
            false => match crate::when::parse(&when) {
                Ok(expr) => Some(Arc::new(expr)),
                Err(error) => {
                    errors.push(format!("keybindings.json {name}: the when clause \"{when}\" is not understood ({error}), so the entry is not used."));
                    continue;
                }
            },
        };
        let same_keys = |b: &Binding| keys.is_empty() || b.keys() == keys;
        if remove || keys.is_empty() {
            bindings.retain(|b| !(b.command == *command && same_keys(b) && (when.is_empty() || b.when == when)));
            continue;
        }
        if bindings.iter().any(|b| b.command == *command && same_keys(b) && b.when == when) {
            continue;
        }
        let scope = if editor.contains(command) { Scope::Editor } else { Scope::Global };
        bindings.push(Binding { command: command.clone(), shortcut: keys[0], chord: keys.get(1).copied(), scope, when, user: Some(index), cond });
    }
    let weight = |b: &Binding| {
        let m = b.shortcut.modifiers;
        b.chord.is_none() as u8 * 4 + 3 - (m.command || m.ctrl) as u8 - m.shift as u8 - m.alt as u8
    };
    bindings.reverse();
    bindings.sort_by_key(weight);
    (bindings, errors)
}

pub fn path() -> PathBuf {
    crate::settings::dir().join("keybindings.json")
}

pub fn write_defaults() {
    let mut text = String::from("// Entries here are added to the default key bindings, and later entries win.\n// \"-command\" removes a default binding. The defaults are:\n");
    for b in defaults() {
        let key = b.keys().iter().map(key_text).collect::<Vec<_>>().join(" ");
        let when = if b.when.is_empty() { String::new() } else { format!(", \"when\": \"{}\"", b.when) };
        text.push_str(&format!("// {{ \"key\": \"{key}\", \"command\": \"{}\"{when} }},\n", b.command.id()));
    }
    text.push_str("[\n]\n");
    let _ = std::fs::create_dir_all(crate::settings::dir());
    let _ = std::fs::write(path(), text);
}

pub fn entries() -> Result<Vec<Entry>, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => crate::jsonc::parse(&text).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())),
        Err(_) => Ok(vec![]),
    }
}

pub fn write_entries(entries: &[Entry]) {
    if let Ok(text) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::create_dir_all(crate::settings::dir());
        if let Err(error) = std::fs::write(path(), text) {
            crate::ui::notify(format!("keybindings.json could not be written: {error}"));
        }
    }
    reload();
}

static BINDINGS: Mutex<Option<Arc<Vec<Binding>>>> = Mutex::new(None);
static CONTEXT: Mutex<Option<Arc<Map<String, Value>>>> = Mutex::new(None);

pub fn set_context(keys: Map<String, Value>) {
    *CONTEXT.lock().unwrap() = Some(Arc::new(keys));
}

pub fn context() -> Arc<Map<String, Value>> {
    CONTEXT.lock().unwrap().get_or_insert_with(Default::default).clone()
}

pub fn bindings() -> Arc<Vec<Binding>> {
    BINDINGS
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            let user = if cfg!(test) { Ok(vec![]) } else { entries() };
            let user = user.unwrap_or_else(|e| {
                crate::ui::notify(format!("keybindings.json could not be read, default keys are in use: {e}"));
                vec![]
            });
            let (bindings, errors) = build(defaults(), &user);
            errors.into_iter().for_each(crate::ui::notify);
            Arc::new(bindings)
        })
        .clone()
}

pub fn reload() {
    *BINDINGS.lock().unwrap() = None;
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
            | CloseFolder | AddFolderToWorkspace | RemoveFolderFromWorkspace | RemoveWorkspaceFolder(_) | AddWorkspaceFolder(_) | MoveWorkspaceFolder(..)
            | SaveWorkspaceAs | OpenWorkspaceFromFile | OpenWorkspace(_) | CloseWorkspace | OpenSettings | OpenSettingsJson | OpenKeybindings | OpenSnippets | PathChanged(..) | Exit | ImportTheme
            | BrowseThemes | SelectTheme | ImportThemeFile(_) | SetTheme(_) | ImportIconTheme | BrowseIconThemes | SelectIconTheme
            | ImportIconThemeFile(_) | SetIconTheme(_) | CutFiles | CopyFiles | PasteFiles | CloseAllEditors | ShowTab(..) | CloseTab(..)
            | NewWindow | OpenFolderInNewWindow | DuplicateWorkspace => "File",
            Undo | Redo | Cut | Copy | Paste | Find | Replace | ToggleComment | ToggleBlockComment | EmmetExpand | MoveLineUp
            | MoveLineDown | CopyLineUp | CopyLineDown | DeleteLine | JoinLines | InsertLineAbove | InsertLineBelow | Fold
            | Unfold | FoldAll | UnfoldAll | RenameSymbol | QuickFix | OrganizeImports | FormatDocument | FormatSelection => "Edit",
            SelectAll | AddCursorAbove | AddCursorBelow | AddCursorsToLineEnds | SelectNextOccurrence | MoveSelectionToNextFindMatch | SelectAllOccurrences
            | ExpandSelection | ShrinkSelection | SelectToBracket | CursorUndo | ToggleColumnSelection | CursorColumnSelectLeft
            | CursorColumnSelectRight | CursorColumnSelectUp | CursorColumnSelectDown => "Selection",
            ShowExplorer | ShowSourceControl | ShowRunAndDebug | ShowTesting | ToggleSidebar | TogglePanel | ToggleAi | SplitEditor | ToggleMinimap
            | ToggleWordWrap | ToggleTheme | ZoomIn | ZoomOut | ZoomReset | ShowProblems | RefreshExplorer | ToggleInlineDiff | FilterExplorer | ToggleFileNesting
            | FocusOpenEditors | MarkdownPreview | MarkdownPreviewSide | SplitEditorDown | SplitEditorLeft | SplitEditorUp | LayoutSingle
            | LayoutTwoColumns | LayoutThreeColumns | LayoutTwoRows | LayoutThreeRows | LayoutGrid | LayoutTwoRowsRight | LayoutTwoColumnsBottom
            | ToggleMaximizeGroup | ResetGroupSizes | FocusGroupLeft | FocusGroupRight | FocusGroupUp | FocusGroupDown | MoveEditorLeft
            | MoveEditorRight | MoveEditorUp | MoveEditorDown | CloseGroup | MoveEditorToNewWindow | CopyEditorToNewWindow
            | MoveEditorToMainWindow | ToggleFullScreen | ToggleZenMode | ToggleCenteredLayout
            | ToggleSidebarPosition | ToggleSecondarySidebar | MovePanelLeft | MovePanelRight | MovePanelBottom | MovePanelTop
            | TogglePanelPosition | ToggleMaximizedPanel | AlignPanelCenter | AlignPanelJustify | AlignPanelLeft | AlignPanelRight
            | ActivityBarSide | ActivityBarTop | ActivityBarHidden | MoveView | MoveViewTo(_) | MoveViewPlace(..) | ResetViewLocations => "View",
            GoToFile | ShowCommands | FindInFiles | GoToSymbol | GoToLine | GoToBracket | GoTo(..) | NextEditor | PrevEditor | GoBack
            | GoForward | GoToLastEditLocation | ReopenClosed | KeepEditor | NextChange | PrevChange | ShowNextChange | ShowPrevChange | GoToDefinition | PeekDefinition | GoToDeclaration | GoToTypeDefinition
            | GoToImplementation | FindReferences | GoToWorkspaceSymbol | ShowCallHierarchy | ShowTypeHierarchy => "Go",
            ShowHover | TriggerSuggest | TriggerParameterHints | RestartLanguageServers => "Language",
            RunFile | RunTask(_) | RunTaskPicker | RunBuildTask | RunTestTask | RerunLastTask | TerminateTask | ShowRunningTasks
            | ConfigureDefaultBuildTask | OpenTasksJson | RunTaskLabel(_) | TerminateTaskLabel(_) | RevealTask(_) | SetDefaultBuild(_)
            | StartDebugging | RunWithoutDebugging | StopDebugging | RestartDebugging | PauseDebugging | StepOver
            | StepInto | StepOut | ToggleBreakpoint | EditBreakpoint | RemoveAllBreakpoints | AddWatch | AddConfiguration | RunAllTests
            | RunFailedTests | RunTestAtCursor | DebugTestAtCursor | RerunLastTestRun | CancelTestRun | RefreshTests | ShowTestOutput
            | RunAllTestsWithCoverage | ToggleCoverageDisplay => "Run",
            NewTerminal | NewTerminalWith(_) | NewTerminalIn(_) | SplitTerminal | KillTerminal | ScrollToPrevMark | ScrollToNextMark | RunRecentLine
            | RunCommandLine(_) | CopyLastCommandOutput | RenameTerminal | ChangeTerminalColor | SetTerminalColor(_)
            | NewTerminalInEditor | MoveTerminalToEditor | MoveTerminalToPanel | ClearRestoredHistory => "Terminal",
            CompareFile | CompareClipboard | CompareSaved | SelectForCompare | CompareWithSelected | SelectComparePath(_) | ComparePath(_) => "File",
            OpenDiff(..) | GitSync | Git(_) | GitAt(..) | ToggleInlineBlame | OpenLineChanges | RestoreSnapshot(..) | StageHunk | RevertHunk
            | UnstageHunk | StageSelectedRanges | RevertSelectedRanges | UnstageSelectedRanges | MergeOpen | OpenMerge(_)
            | MergeAcceptIncoming | MergeAcceptCurrent | MergeAcceptBoth | MergeAcceptAllIncoming | MergeAcceptAllCurrent
            | MergeNextConflict | MergePrevConflict | MergeToggleBase | MergeComplete => "Source Control",
            ShowOutput | ShowOutputChannels | ClearOutput | OpenOutputInEditor | SelectOutputChannel(_) => "Output",
            ShowNotifications | ClearNotifications | ToggleDoNotDisturb | FocusNotification => "Notifications",
            FocusNextRegion | FocusPrevRegion | ToggleTabFocus | ShowAccessibilityHelp | AnnounceCursorPosition | ToggleScreenReaderMode => "Help",
            About | InstallInPath | ConfirmInstallInPath => "Help",
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
            AddFolderToWorkspace => "Add Folder to Workspace...",
            AddWorkspaceFolder(_) => "Add Folder to Workspace",
            RemoveFolderFromWorkspace | RemoveWorkspaceFolder(_) => "Remove Folder from Workspace",
            MoveWorkspaceFolder(..) => "Move Workspace Folder",
            SaveWorkspaceAs => "Save Workspace As...",
            OpenWorkspaceFromFile => "Open Workspace from File...",
            OpenWorkspace(_) => "Open Workspace",
            CloseWorkspace => "Close Workspace",
            NewTerminalIn(_) => "Terminal: Create New Terminal",
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
            AddCursorsToLineEnds => "Add Cursors to Line Ends",
            SelectNextOccurrence => "Add Next Occurrence",
            MoveSelectionToNextFindMatch => "Move Last Selection to Next Find Match",
            SelectAllOccurrences => "Select All Occurrences",
            ExpandSelection => "Expand Selection",
            ShrinkSelection => "Shrink Selection",
            SelectToBracket => "Select to Bracket",
            CursorUndo => "Cursor Undo",
            ToggleColumnSelection => "Toggle Column Selection Mode",
            CursorColumnSelectLeft => "Column Select Left",
            CursorColumnSelectRight => "Column Select Right",
            CursorColumnSelectUp => "Column Select Up",
            CursorColumnSelectDown => "Column Select Down",
            ShowExplorer => "Explorer",
            ShowSourceControl => "Source Control",
            ShowRunAndDebug => "Run and Debug",
            ShowTesting => "Test: Focus on Test Explorer View",
            ToggleSidebar => "Toggle Side Bar",
            TogglePanel => "Toggle Panel",
            ToggleAi => "Toggle AI Panel",
            SplitEditor => "Split Editor Right",
            SplitEditorDown => "Split Editor Down",
            SplitEditorLeft => "Split Editor Left",
            SplitEditorUp => "Split Editor Up",
            LayoutSingle => "Editor Layout: Single",
            LayoutTwoColumns => "Editor Layout: Two Columns",
            LayoutThreeColumns => "Editor Layout: Three Columns",
            LayoutTwoRows => "Editor Layout: Two Rows",
            LayoutThreeRows => "Editor Layout: Three Rows",
            LayoutGrid => "Editor Layout: Grid (2x2)",
            LayoutTwoRowsRight => "Editor Layout: Two Rows Right",
            LayoutTwoColumnsBottom => "Editor Layout: Two Columns Bottom",
            ToggleMaximizeGroup => "Toggle Maximize Editor Group",
            ResetGroupSizes => "Reset Editor Group Sizes",
            FocusGroupLeft => "Focus Editor Group Left",
            FocusGroupRight => "Focus Editor Group Right",
            FocusGroupUp => "Focus Editor Group Above",
            FocusGroupDown => "Focus Editor Group Below",
            MoveEditorLeft => "Move Editor into Group Left",
            MoveEditorRight => "Move Editor into Group Right",
            MoveEditorUp => "Move Editor into Group Above",
            MoveEditorDown => "Move Editor into Group Below",
            CloseGroup => "Close Editor Group",
            MoveEditorToNewWindow => "Move Editor into New Window",
            CopyEditorToNewWindow => "Copy Editor into New Window",
            MoveEditorToMainWindow => "Move Editor into Main Window",
            NewWindow => "New Window",
            OpenFolderInNewWindow => "Open Folder in New Window...",
            DuplicateWorkspace => "Duplicate Workspace in New Window",
            ToggleMinimap => "Toggle Minimap",
            ToggleWordWrap => "Toggle Word Wrap",
            ToggleTheme => "Toggle Light/Dark Theme",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomReset => "Reset Zoom",
            ToggleFullScreen => "View: Toggle Full Screen",
            ToggleZenMode => "View: Toggle Zen Mode",
            ToggleCenteredLayout => "View: Toggle Centered Layout",
            ToggleSidebarPosition => "View: Toggle Primary Side Bar Position",
            ToggleSecondarySidebar => "View: Toggle Secondary Side Bar Visibility",
            MovePanelLeft => "View: Move Panel Left",
            MovePanelRight => "View: Move Panel Right",
            MovePanelBottom => "View: Move Panel To Bottom",
            MovePanelTop => "View: Move Panel To Top",
            TogglePanelPosition => "View: Toggle Panel Position",
            ToggleMaximizedPanel => "View: Toggle Maximized Panel",
            AlignPanelCenter => "View: Set Panel Alignment to Center",
            AlignPanelJustify => "View: Set Panel Alignment to Justify",
            AlignPanelLeft => "View: Set Panel Alignment to Left",
            AlignPanelRight => "View: Set Panel Alignment to Right",
            ActivityBarSide => "View: Show Activity Bar at the Side",
            ActivityBarTop => "View: Show Activity Bar at the Top",
            ActivityBarHidden => "View: Hide Activity Bar",
            MoveView => "View: Move View...",
            MoveViewTo(_) => "View: Move View To...",
            MoveViewPlace(..) => "View: Move View",
            ResetViewLocations => "View: Reset View Locations",
            GoToFile => "Go to File...",
            ShowCommands => "Show All Commands",
            FindInFiles => "Find in Files",
            GoToSymbol => "Go to Symbol in Editor...",
            GoToLine => "Go to Line...",
            GoToBracket => "Go to Bracket",
            GoTo(..) => "Go to Position",
            NextEditor => "Next Editor",
            PrevEditor => "Previous Editor",
            GoBack => "Go Back",
            GoForward => "Go Forward",
            GoToLastEditLocation => "Go to Last Edit Location",
            ReopenClosed => "Reopen Closed Editor",
            KeepEditor => "Keep Editor Open",
            NextChange => "Go to Next Change",
            PrevChange => "Go to Previous Change",
            ShowNextChange => "Show Next Change",
            ShowPrevChange => "Show Previous Change",
            ToggleInlineDiff => "Toggle Inline Diff View",
            MarkdownPreview => "Markdown: Open Preview",
            MarkdownPreviewSide => "Markdown: Open Preview to the Side",
            StageHunk => "Stage Hunk",
            RevertHunk => "Revert Hunk",
            UnstageHunk => "Unstage Hunk",
            StageSelectedRanges => "Stage Selected Ranges",
            RevertSelectedRanges => "Revert Selected Ranges",
            UnstageSelectedRanges => "Unstage Selected Ranges",
            MergeOpen | OpenMerge(_) => "Merge Editor: Open",
            MergeAcceptIncoming => "Merge Editor: Accept Incoming",
            MergeAcceptCurrent => "Merge Editor: Accept Current",
            MergeAcceptBoth => "Merge Editor: Accept Both",
            MergeAcceptAllIncoming => "Merge Editor: Accept All Incoming",
            MergeAcceptAllCurrent => "Merge Editor: Accept All Current",
            MergeNextConflict => "Merge Editor: Go to Next Unresolved Conflict",
            MergePrevConflict => "Merge Editor: Go to Previous Unresolved Conflict",
            MergeToggleBase => "Merge Editor: Show Base",
            MergeComplete => "Merge Editor: Complete Merge",
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
            ScrollToPrevMark => "Terminal: Scroll to Previous Command",
            ScrollToNextMark => "Terminal: Scroll to Next Command",
            RunRecentLine => "Terminal: Run Recent Command...",
            RunCommandLine(_) => "Terminal: Run Command",
            CopyLastCommandOutput => "Terminal: Copy Last Command Output",
            RenameTerminal => "Terminal: Rename...",
            ChangeTerminalColor => "Terminal: Change Color...",
            SetTerminalColor(_) => "Terminal: Set Color",
            NewTerminalInEditor => "Terminal: Create New Terminal in Editor Area",
            MoveTerminalToEditor => "Terminal: Move Terminal into Editor Area",
            MoveTerminalToPanel => "Terminal: Move Terminal into Panel",
            ClearRestoredHistory => "Terminal: Clear Restored History",
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
            ShowOutput => "Output: Show Output",
            ShowOutputChannels => "Output: Show Output Channels...",
            ClearOutput => "Output: Clear Output",
            OpenOutputInEditor => "Output: Open Output in Editor",
            SelectOutputChannel(_) => "Output: Select Output Channel",
            ShowNotifications => "Notifications: Show Notifications",
            ClearNotifications => "Notifications: Clear All Notifications",
            ToggleDoNotDisturb => "Notifications: Toggle Do Not Disturb",
            FocusNotification => "Notifications: Focus Notification Toast",
            FocusNextRegion => "View: Focus Next Part",
            FocusPrevRegion => "View: Focus Previous Part",
            ToggleTabFocus => "Toggle Tab Key Moves Focus",
            ShowAccessibilityHelp => "Accessibility: Show Accessibility Help",
            AnnounceCursorPosition => "Accessibility: Announce Cursor Position",
            ToggleScreenReaderMode => "Accessibility: Toggle Screen Reader Optimized Mode",
            GitSync => "Synchronize Changes",
            RefreshExplorer => "Refresh Explorer",
            CutFiles => "File: Cut",
            CopyFiles => "File: Copy",
            PasteFiles => "File: Paste",
            CloseAllEditors => "File: Close All Editors",
            ShowTab(..) => "Show Editor",
            CloseTab(..) => "Close Editor",
            FilterExplorer => "Explorer: Filter",
            ToggleFileNesting => "Explorer: Toggle File Nesting",
            FocusOpenEditors => "Explorer: Focus on Open Editors View",
            PathChanged(..) => "Path Changed",
            About => "About Halo",
            InstallInPath | ConfirmInstallInPath => "Shell Command: Install 'halo' command in PATH",
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
        assert_eq!(groups, ["File", "Edit", "Selection", "View", "Go", "Language", "Run", "Terminal", "Source Control", "Output", "Notifications", "Help"]);
    }

    #[test]
    fn keys_round_trip_and_user_entries_rebind() {
        for binding in defaults() {
            assert_eq!(parse_key(&key_text(&binding.shortcut)), Some(binding.shortcut));
        }
        assert_eq!(parse_key("Ctrl+Shift+`"), Some(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Backtick)));
        assert_eq!(parse_key("ctrl+nothing"), None);
        assert_eq!(Command::OpenFolder.id(), "openFolder");

        let entry = |key: &str, command: &str| Entry { key: key.into(), command: command.into(), ..Default::default() };
        let user = [
            entry("ctrl+k ctrl+u", "undo"),
            entry("", "save"),
            entry("f9", "noSuchCommand"),
            entry("alt+p", "goToFile"),
            entry("ctrl+z", "-undo"),
            entry("ctrl+p", "goToFile"),
        ];
        let (built, errors) = build(defaults(), &user);
        assert!(errors.is_empty());
        let of = |c: Command| built.iter().filter(|b| b.command == c).cloned().collect::<Vec<_>>();
        let undo = of(Command::Undo);
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].chord, parse_key("ctrl+u"));
        assert_eq!((undo[0].scope, undo[0].user), (Scope::Editor, Some(0)));
        assert!(of(Command::Save).is_empty());
        let go = of(Command::GoToFile);
        assert_eq!(go.len(), 2);
        assert_eq!((go[0].shortcut, go[0].user), (parse_key("alt+p").unwrap(), Some(3)));
        assert_eq!(go[1].user, None);
        assert_eq!(of(Command::Redo).len(), 2);
        let first_single = built.iter().position(|b| b.chord.is_none()).unwrap();
        assert!(built[first_single..].iter().all(|b| b.chord.is_none()));
        let pos = |c: Command| built.iter().position(|b| b.command == c).unwrap();
        assert!(pos(Command::GoToSymbol) < pos(Command::OpenFile));
        assert!(of(Command::Save).is_empty() && of(Command::SaveAll)[0].when == SEND_TO_SHELL);
    }

    #[test]
    fn later_entries_win_by_when_and_bad_entries_are_reported() {
        let entry = |key: &str, command: &str, when: &str| Entry { key: key.into(), command: command.into(), when: when.into(), ..Default::default() };
        let user = [
            entry("ctrl+alt+u", "find", "editorLangId == rust"),
            entry("ctrl+alt+u", "replace", "editorTextFocus"),
            entry("ctrl+alt+u", "-replace", "other"),
            entry("ctrl+nope", "find", ""),
            entry("ctrl+e", "find", "a &&"),
            entry("ctrl+b", "-toggleSidebar", ""),
            entry("ctrl+b", "toggleSidebar", "sideBarVisible"),
        ];
        let (built, errors) = build(defaults(), &user);
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("entry 4") && errors[0].contains("key"));
        assert!(errors[1].contains("entry 5") && errors[1].contains("a &&"));
        let pick = |keys: serde_json::Value| {
            let keys = keys.as_object().unwrap().clone();
            let shortcut = parse_key("ctrl+alt+u").unwrap();
            built.iter().find(|b| b.shortcut == shortcut && b.active(&keys)).map(|b| b.command.clone())
        };
        assert_eq!(pick(serde_json::json!({"editorTextFocus": true, "editorLangId": "rust"})), Some(Command::Replace));
        assert_eq!(pick(serde_json::json!({"editorLangId": "rust"})), Some(Command::Find));
        assert_eq!(pick(serde_json::json!({})), None);
        let sidebar: Vec<_> = built.iter().filter(|b| b.command == Command::ToggleSidebar).collect();
        assert_eq!(sidebar.len(), 1);
        assert_eq!((sidebar[0].when.as_str(), sidebar[0].user), ("sideBarVisible", Some(6)));
        let focus = |terminal: bool, send: bool| serde_json::json!({"terminalFocus": terminal, "config.terminal.integrated.sendKeybindingsToShell": send});
        let save = built.iter().find(|b| b.command == Command::Save).unwrap();
        assert!(save.active(focus(false, true).as_object().unwrap()) && save.active(focus(true, false).as_object().unwrap()));
        assert!(!save.active(focus(true, true).as_object().unwrap()));
    }
}
