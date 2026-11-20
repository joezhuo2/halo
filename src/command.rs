use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    NewFile,
    OpenFile,
    OpenFolder,
    OpenPath(PathBuf),
    OpenDiff(String, Vec<String>),
    Save,
    SaveAs,
    SaveAll,
    CloseEditor,
    CloseFolder,
    OpenSettings,
    Exit,

    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Find,
    Replace,
    ToggleComment,

    SelectAll,
    AddCursorAbove,
    AddCursorBelow,
    SelectNextOccurrence,

    ShowExplorer,
    ShowSourceControl,
    ToggleSidebar,
    TogglePanel,
    ToggleAi,
    SplitEditor,
    ToggleMinimap,
    ToggleTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,

    GoToLine,
    NextEditor,
    PrevEditor,

    NewTerminal,
    NewTerminalWith(String),
    SplitTerminal,
    KillTerminal,

    GitSync,
    RefreshExplorer,
    PathChanged(PathBuf, Option<PathBuf>),
    About,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Global,
    Editor,
}

pub struct Binding {
    pub command: Command,
    pub shortcut: KeyboardShortcut,
    pub scope: Scope,
}

pub fn bindings() -> &'static [Binding] {
    static BINDINGS: OnceLock<Vec<Binding>> = OnceLock::new();
    BINDINGS.get_or_init(|| {
        use Command::*;
        const C: Modifiers = Modifiers::COMMAND;
        let cs = C | Modifiers::SHIFT;
        let ca = C | Modifiers::ALT;
        let g = |m, k, c| Binding { command: c, shortcut: KeyboardShortcut::new(m, k), scope: Scope::Global };
        let e = |m, k, c| Binding { command: c, shortcut: KeyboardShortcut::new(m, k), scope: Scope::Editor };
        vec![
            g(cs, Key::S, SaveAs),
            g(cs, Key::O, OpenFolder),
            g(cs, Key::E, ShowExplorer),
            g(cs, Key::G, ShowSourceControl),
            g(cs, Key::Backtick, NewTerminal),
            g(cs, Key::Tab, PrevEditor),
            g(ca, Key::B, ToggleAi),
            e(cs, Key::Z, Redo),
            e(ca, Key::ArrowUp, AddCursorAbove),
            e(ca, Key::ArrowDown, AddCursorBelow),
            g(C, Key::N, NewFile),
            g(C, Key::O, OpenFile),
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
            e(C, Key::Z, Undo),
            e(C, Key::Y, Redo),
            e(C, Key::A, SelectAll),
            e(C, Key::F, Find),
            e(C, Key::H, Replace),
            e(C, Key::G, GoToLine),
            e(C, Key::D, SelectNextOccurrence),
            e(C, Key::Slash, ToggleComment),
        ]
    })
}

pub fn shortcut(command: &Command) -> Option<KeyboardShortcut> {
    bindings().iter().find(|b| &b.command == command).map(|b| b.shortcut)
}

impl Command {
    pub fn label(&self) -> &'static str {
        use Command::*;
        match self {
            NewFile => "New File",
            OpenFile => "Open File...",
            OpenFolder => "Open Folder...",
            OpenPath(_) => "Open",
            OpenDiff(..) => "Open Changes",
            Save => "Save",
            SaveAs => "Save As...",
            SaveAll => "Save All",
            CloseEditor => "Close Editor",
            CloseFolder => "Close Folder",
            OpenSettings => "Settings",
            Exit => "Exit",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            Find => "Find",
            Replace => "Replace",
            ToggleComment => "Toggle Line Comment",
            SelectAll => "Select All",
            AddCursorAbove => "Add Cursor Above",
            AddCursorBelow => "Add Cursor Below",
            SelectNextOccurrence => "Add Next Occurrence",
            ShowExplorer => "Explorer",
            ShowSourceControl => "Source Control",
            ToggleSidebar => "Toggle Side Bar",
            TogglePanel => "Toggle Panel",
            ToggleAi => "Toggle AI Panel",
            SplitEditor => "Split Editor Right",
            ToggleMinimap => "Toggle Minimap",
            ToggleTheme => "Toggle Light/Dark Theme",
            ZoomIn => "Zoom In",
            ZoomOut => "Zoom Out",
            ZoomReset => "Reset Zoom",
            GoToLine => "Go to Line...",
            NextEditor => "Next Editor",
            PrevEditor => "Previous Editor",
            NewTerminal => "New Terminal",
            NewTerminalWith(_) => "New Terminal",
            SplitTerminal => "Split Terminal",
            KillTerminal => "Kill Terminal",
            GitSync => "Synchronize Changes",
            RefreshExplorer => "Refresh Explorer",
            PathChanged(..) => "Path Changed",
            About => "About Halo",
        }
    }
}
