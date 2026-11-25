use crate::command::{self, Command};
use std::collections::{BTreeMap, HashMap};

pub const MARK_STYLE: &str = "halo.markStyleMenu";
pub const APPEARANCE: &str = "halo.appearanceMenu";
pub const TASKS: &str = "halo.taskList";
pub const PROFILES: &str = "halo.terminalProfiles";
const SPECIAL: [(&str, &str); 4] = [(MARK_STYLE, "Mark Style submenu"), (APPEARANCE, "Appearance submenu"), (TASKS, "Configured tasks"), (PROFILES, "Terminal profiles")];

pub const LOCATIONS: [(&str, &str); 10] = [
    ("menubar.file", "File menu"),
    ("menubar.edit", "Edit menu"),
    ("menubar.selection", "Selection menu"),
    ("menubar.view", "View menu"),
    ("menubar.go", "Go menu"),
    ("menubar.run", "Run menu"),
    ("menubar.terminal", "Terminal menu"),
    ("menubar.help", "Help menu"),
    ("editor.context", "Editor context menu"),
    ("editor.toolbar", "Editor toolbar"),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Cmd(Command),
    Sep,
    Special(&'static str),
}

pub fn appearance() -> Vec<Option<Command>> {
    use Command::*;
    vec![
        Some(ToggleFullScreen), Some(ToggleZenMode), Some(ToggleCenteredLayout), None,
        Some(ToggleSidebar), Some(ToggleSecondarySidebar), Some(TogglePanel), Some(ToggleSidebarPosition), None,
        Some(MovePanelLeft), Some(MovePanelRight), Some(MovePanelBottom), Some(MovePanelTop), Some(TogglePanelPosition), Some(ToggleMaximizedPanel), None,
        Some(AlignPanelCenter), Some(AlignPanelJustify), Some(AlignPanelLeft), Some(AlignPanelRight), None,
        Some(ActivityBarSide), Some(ActivityBarTop), Some(ActivityBarHidden), None,
        Some(MoveView), Some(MoveViewToNewWindow), Some(MoveViewBackToMain), Some(ResetViewLocations),
    ]
}

pub fn defaults(location: &str) -> Vec<Entry> {
    use Command::*;
    let (special, items): (&[&'static str], Vec<Option<Command>>) = match location {
        "menubar.file" => (&[], vec![
            Some(NewFile), Some(NewWindow), Some(OpenFile), Some(OpenFolder), Some(OpenFolderInNewWindow), Some(OpenWorkspaceFromFile), Some(OpenRecent), None,
            Some(AddFolderToWorkspace), Some(SaveWorkspaceAs), None,
            Some(Save), Some(SaveAs), Some(SaveAll), None,
            Some(CompareFile), Some(CompareClipboard), Some(CompareSaved), None,
            Some(CloseEditor), Some(CloseFolder), Some(CloseWorkspace), None,
            Some(OpenSettings), Some(OpenSettingsJson), Some(OpenKeybindings), Some(CustomizeMenus), None,
            Some(SelectTheme), Some(SelectIconTheme), None,
            Some(Exit),
        ]),
        "menubar.edit" => (&[MARK_STYLE], vec![
            None,
            Some(Undo), Some(Redo), None,
            Some(Cut), Some(Copy), Some(Paste), None,
            Some(Find), Some(Replace), None,
            Some(ToggleComment), Some(RewrapComment), None,
            Some(DeleteWordPartLeft), Some(DeleteWordPartRight), None,
            Some(RenameSymbol), Some(QuickFix), Some(OrganizeImports), Some(FormatDocument), Some(FormatSelection),
        ]),
        "menubar.selection" => (&[], vec![
            Some(SelectAll), None,
            Some(AddCursorAbove), Some(AddCursorBelow), Some(AddCursorsToLineEnds), Some(SelectNextOccurrence), None,
            Some(CursorWordPartLeftSelect), Some(CursorWordPartRightSelect),
            Some(ToggleColumnSelection),
        ]),
        "menubar.view" => (&[APPEARANCE], vec![
            None,
            Some(ShowExplorer), Some(ShowSourceControl), Some(ShowRunAndDebug), Some(ShowTesting), None,
            Some(ToggleSidebar), Some(TogglePanel), Some(ToggleAi), Some(ShowProblems), Some(ShowOutput), Some(ShowNotifications), None,
            Some(SplitEditor), Some(SplitEditorDown), Some(ToggleMaximizeGroup), Some(ResetGroupSizes), Some(ToggleMinimap), Some(ToggleHideSecrets), Some(MarkdownPreview), Some(MarkdownPreviewSide), None,
            Some(ToggleTheme), Some(ZoomIn), Some(ZoomOut), Some(ZoomReset),
        ]),
        "menubar.go" => (&[], vec![
            Some(GoToFile), Some(GoToSymbol), Some(GoToWorkspaceSymbol), Some(GoToLine), Some(GoToBracket), Some(FindInFiles), Some(NewSearchEditor), Some(ShowCommands), None,
            Some(GoToDefinition), Some(GoToDeclaration), Some(GoToTypeDefinition), Some(GoToImplementation), Some(FindReferences),
            Some(ShowCallHierarchy), Some(ShowTypeHierarchy), None,
            Some(CursorWordPartLeft), Some(CursorWordPartRight),
            Some(GoBack), Some(GoToLastEditLocation), Some(NextEditor), Some(PrevEditor), None,
            Some(NextChange), Some(PrevChange),
        ]),
        "menubar.run" => (&[], vec![
            Some(StartDebugging), Some(RunWithoutDebugging), Some(StopDebugging), Some(RestartDebugging), None,
            Some(StepOver), Some(StepInto), Some(StepOut), Some(PauseDebugging), None,
            Some(RunToCursor), Some(JumpToCursor),
            Some(ToggleBreakpoint), Some(EditBreakpoint), Some(AddFunctionBreakpoint), Some(BreakOnValueChange), Some(RemoveAllBreakpoints), Some(AddWatch), Some(AddConfiguration),
            Some(OpenDisassembly), Some(OpenMemory), None,
            Some(RunAllTests), Some(RunFailedTests), Some(RunTestAtCursor), Some(DebugTestAtCursor), Some(RunAllTestsWithCoverage), Some(CancelTestRun), None,
            Some(RunFile), Some(RunTaskPicker), Some(RunBuildTask), Some(RunTestTask), Some(RerunLastTask), Some(TerminateTask), Some(ShowRunningTasks), Some(ConfigureDefaultBuildTask), Some(OpenTasksJson), None,
            Some(RestRequest), Some(RestRerun), Some(RestCancel), None,
        ]),
        "menubar.terminal" => (&[], vec![
            Some(NewTerminal), Some(SplitTerminal), Some(KillTerminal), Some(NewTerminalInEditor), Some(MoveTerminalToEditor), Some(MoveTerminalToPanel), None,
            Some(RenameTerminal), Some(ChangeTerminalColor), Some(RunRecentLine), Some(CopyLastCommandOutput), Some(ClearRestoredHistory), None,
        ]),
        "menubar.help" => (&[], vec![Some(InstallInPath), None, Some(About)]),
        "editor.context" => (&[MARK_STYLE], vec![
            Some(GoToDefinition), Some(PeekDefinition), Some(GoToImplementation), Some(FindReferences), Some(ShowCallHierarchy), None,
            Some(RenameSymbol), Some(QuickFix), Some(FormatDocument), None,
            Some(InlineChat), None,
            Some(Cut), Some(Copy), Some(Paste),
        ]),
        "editor.toolbar" => (&[], vec![Some(MarkdownPreview), Some(ToggleAi), Some(SplitEditor)]),
        _ => (&[], vec![]),
    };
    let mut out: Vec<Entry> = special.iter().map(|s| Entry::Special(s)).collect();
    out.extend(items.into_iter().map(|i| i.map_or(Entry::Sep, Entry::Cmd)));
    match location {
        "menubar.run" => out.push(Entry::Special(TASKS)),
        "menubar.terminal" => out.push(Entry::Special(PROFILES)),
        _ => {}
    }
    out
}

thread_local! {
    static BY_ID: HashMap<String, Command> = command::all().into_iter().map(|c| (c.id(), c)).collect();
}

pub fn parse(id: &str) -> Option<Entry> {
    if id == "-" { return Some(Entry::Sep) }
    if let Some((s, _)) = SPECIAL.iter().find(|s| s.0 == id) { return Some(Entry::Special(s)) }
    BY_ID.with(|map| map.get(id).cloned()).map(Entry::Cmd)
}

pub fn id(entry: &Entry) -> String {
    match entry {
        Entry::Cmd(c) => c.id(),
        Entry::Sep => "-".into(),
        Entry::Special(s) => s.to_string(),
    }
}

pub fn label(entry: &Entry) -> String {
    match entry {
        Entry::Cmd(c) => c.label().to_string(),
        Entry::Sep => "Separator".into(),
        Entry::Special(s) => SPECIAL.iter().find(|x| x.0 == *s).map_or(s.to_string(), |x| x.1.to_owned()),
    }
}

pub fn ids(location: &str, menus: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    menus.get(location).cloned().unwrap_or_else(|| defaults(location).iter().map(id).collect())
}

pub fn resolve(location: &str, menus: &BTreeMap<String, Vec<String>>) -> Vec<Entry> {
    let entries = match menus.get(location) {
        Some(list) => list.iter().filter_map(|i| parse(i)).collect(),
        None => defaults(location),
    };
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    for e in entries {
        if e == Entry::Sep && out.last().is_none_or(|l| *l == Entry::Sep) { continue }
        out.push(e);
    }
    if out.last() == Some(&Entry::Sep) { out.pop(); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_and_every_entry_round_trips_through_its_id() {
        for (location, _) in LOCATIONS {
            let entries = resolve(location, &BTreeMap::new());
            assert!(!entries.is_empty(), "{location}");
            for e in &entries {
                assert_eq!(parse(&id(e)).as_ref(), Some(e), "{location}: {}", id(e));
            }
        }
    }

    #[test]
    fn overrides_skip_unknown_ids_and_tidy_separators() {
        let menus = BTreeMap::from([("menubar.help".to_owned(), ["-", "showCommands", "-", "-", "no.such.command", MARK_STYLE, "-"].map(String::from).to_vec())]);
        assert_eq!(resolve("menubar.help", &menus), [Entry::Cmd(Command::ShowCommands), Entry::Sep, Entry::Special(MARK_STYLE)]);
        assert_eq!(ids("menubar.help", &menus).len(), 7);
        assert_eq!(ids("menubar.selection", &menus)[0], Command::SelectAll.id());
    }

    #[test]
    fn settings_round_trip() {
        let user = serde_json::json!({"halo.menus": {"editor.toolbar": ["splitEditor", "-"]}});
        let mut errors = vec![];
        let (s, _, _) = crate::layers::resolve(&[(user.as_object().unwrap(), "settings.json")], &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(resolve("editor.toolbar", &s.menus), [Entry::Cmd(Command::SplitEditor)]);
    }
}
