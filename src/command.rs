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
    Save,
    SaveAs,
    SaveAll,
    CloseEditor,
    CloseFolder,
    OpenSettings,
    OpenSettingsJson,
    OpenKeybindings,
    Exit,

    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Find,
    Replace,
    ToggleComment,
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

    ShowExplorer,
    ShowSourceControl,
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
    let sa = Modifiers::SHIFT | Modifiers::ALT;
    vec![
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
        g(NONE, Key::F5, RunFile),
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
    vec![
        NewFile, OpenFile, OpenFolder, Save, SaveAs, SaveAll, CloseEditor, CloseFolder, OpenSettings, OpenSettingsJson,
        OpenKeybindings, Exit, Undo, Redo, Cut, Copy, Paste, Find, Replace, ToggleComment, MoveLineUp, MoveLineDown, CopyLineUp, CopyLineDown, DeleteLine, JoinLines, InsertLineAbove,
        InsertLineBelow, Fold, Unfold, FoldAll, UnfoldAll, SelectAll, AddCursorAbove,
        AddCursorBelow, SelectNextOccurrence, ShowExplorer, ShowSourceControl, ToggleSidebar, TogglePanel, ToggleAi,
        SplitEditor, ToggleMinimap, ToggleWordWrap, ToggleTheme, ZoomIn, ZoomOut, ZoomReset, GoToFile, ShowCommands, FindInFiles,
        GoToSymbol, GoToLine, NextEditor, PrevEditor, GoBack, GoToDefinition, PeekDefinition, GoToDeclaration,
        GoToTypeDefinition, GoToImplementation, FindReferences, GoToWorkspaceSymbol, ShowCallHierarchy, ShowTypeHierarchy,
        ShowHover, TriggerSuggest, TriggerParameterHints, RenameSymbol, QuickFix, OrganizeImports, FormatDocument,
        FormatSelection, ShowProblems, RestartLanguageServers, NewTerminal, SplitTerminal, KillTerminal, RunFile, GitSync,
        RefreshExplorer, About,
    ]
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
        let name = format!("{self:?}");
        name[..1].to_lowercase() + &name[1..]
    }

    pub fn label(&self) -> &'static str {
        use Command::*;
        match self {
            NewFile => "New File",
            OpenFile => "Open File...",
            OpenFolder | OpenFolderIn(_) => "Open Folder...",
            OpenPath(_) => "Open",
            OpenDiff(..) => "Open Changes",
            Save => "Save",
            SaveAs => "Save As...",
            SaveAll => "Save All",
            CloseEditor => "Close Editor",
            CloseFolder => "Close Folder",
            OpenSettings => "Settings",
            OpenSettingsJson => "Open settings.json",
            OpenKeybindings => "Keyboard Shortcuts (keybindings.json)",
            Exit => "Exit",
            Undo => "Undo",
            Redo => "Redo",
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            Find => "Find",
            Replace => "Replace",
            ToggleComment => "Toggle Line Comment",
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
            ShowExplorer => "Explorer",
            ShowSourceControl => "Source Control",
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
