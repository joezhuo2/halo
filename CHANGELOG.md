# Changelog

All notable changes to Halo are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Halo uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Windows jump list**: right-click the taskbar icon for recent folders,
  recent files and a New Window task. Items can be pinned to the list.
- **Install the AI CLIs**: when the Claude or Codex CLI is missing, the AI
  panel offers an Install button that runs the official installer in the
  panel's terminal.

### Fixed

- **macOS window controls**: Halo now uses the native traffic lights,
  resizing and rounded corners. The green button opens a fullscreen Space.
- Startup no longer flashes a white window when the window was last
  maximized: windows are cloaked from creation until the first frames are
  drawn.

## [0.1.4] - 2026-11-21

Covered by tests and by starting the release build off screen. Most of it
has not been used by hand yet, so expect rough edges.

### Added

- **Diff editor**: changes open side by side, with the working tree file on
  the right as the live, editable document. Inline view toggle per tab and a
  "Side by Side Diff" setting. Next and Previous Change (`Alt+F5`,
  `Shift+Alt+F5`, `F7` and `Shift+F7` in a diff) with an "n of m" count and an
  overview ruler.
- Stage, unstage and revert single hunks or selected lines from a diff tab or
  the command palette.
- Click a change mark in the gutter to peek at the change, then revert or
  stage it. Show Next and Previous Change are on `Alt+F3` and `Shift+Alt+F3`.
- **Merge editor**: Incoming, Current, optional Base and an editable Result,
  with accept per conflict, Accept All, conflict navigation and Complete
  Merge, which saves and stages the file. Opens from the conflict bar and
  from conflicted files in Source Control.
- **Blame** for the current line in the status bar, and optionally inline on
  the caret line ("Git: Toggle Inline Blame").
- **Local history**: every save keeps a version of the file. The Timeline
  lists them with the commits and can diff, restore or delete them.
- Compare the active file with another file, the clipboard or its saved
  version, and compare two files from the Explorer.
- Source Control: Amend, Undo Last Commit, Merge and Rebase with a branch
  picker, Continue, Abort and Skip while an operation is in progress,
  Cherry Pick, tags, remotes, worktrees, Clone, Initialize Repository, and a
  Stashes section with Show, Apply, Pop and Drop.
- **Debugger** over the Debug Adapter Protocol: Run and Debug view with
  variables, watch, call stack and breakpoints, a debug toolbar, and `F5`,
  `F9`, `F10` and `F11`. Reads `.vscode/launch.json`, including
  `preLaunchTask`. Conditional breakpoints, hit counts and logpoints in the
  gutter. The Debug Console evaluates expressions while paused. Adapters are
  set in `debug_adapters`, with defaults for codelldb, lldb-dap, gdb, debugpy
  and dlv.
- **Test explorer** for Rust, pytest, Go, Jest and Vitest: Testing view, Run
  All, Run Failed, Run Test at Cursor (`Ctrl+;` chords), gutter marks with
  the last result, failure messages at the failing line, Debug Test, and Run
  with Coverage.
- **Tasks**: `.vscode/tasks.json` with dependencies, background tasks and
  problem matchers that feed the Problems panel. npm, yarn, pnpm, bun and
  cargo tasks are detected. `Ctrl+Shift+B` runs the build task.
- **Explorer**: select several files, cut, copy and paste, drag and drop to
  move, drop files from the operating system, an Open Editors section, type
  to filter (`Ctrl+Alt+F`), the `files_exclude` setting and compact folders.
- **Terminal**: `Ctrl+Click` opens URLs and file paths with line and column,
  find with `Ctrl+F`, shell integration for PowerShell, bash and zsh (command
  marks with exit status, `Ctrl+Up` and `Ctrl+Down` between commands, Run
  Recent Command), rename and colour, and terminals in the editor area.
- Markdown preview (`Ctrl+Shift+V`, `Ctrl+K V`) with scroll sync.
- Image tabs for png, jpg, gif, webp, bmp, ico and svg, with zoom and pan.
- Import VS Code color themes and file icon themes, from a file or from
  installed VS Code extensions.
- **Settings**: workspace `.vscode/settings.json` and `[language]` blocks,
  VS Code key names in both files, User and Workspace tabs, and search with
  `@modified` and `@lang:`. 41 new editor, editing, workbench and terminal
  settings, including auto save modes, cursor style, line numbers, word wrap
  column and terminal font.
- **Keybindings**: `when` clauses, removal of a default with `-command`,
  notices for entries that cannot be read, and a Keyboard Shortcuts page that
  records chords, shows conflicts and edits `when`.
- **Editor grid**: split right, down, left and up; layouts from Single to
  Grid (2x2); resizable splitters; maximize a group (`Ctrl+K Ctrl+M`, or
  double-click the tab strip); focus and move editors by direction
  (`Ctrl+K Ctrl+Arrow`, `Ctrl+Alt+Arrow`). Dropping a tab on any edge of a
  group splits that way. The layout comes back after a restart.
- **Windows**: New Window (`Ctrl+Shift+N`), Open Folder in New Window, and
  floating editor windows (Move or Copy Editor into New Window, `Ctrl+K O`,
  or drop a tab outside the window).
- **Workbench layout**: full screen (`F11` when not debugging), zen mode
  (`Ctrl+K Z`, `Esc Esc` to leave), centered layout, the side bar on the left
  or right, the panel on any side, maximized or aligned, and the activity bar
  at the side, at the top or hidden.
- **Secondary side bar** (`Ctrl+Alt+B`) holding the AI view. Views can move
  between the side bars and the panel with "View: Move View..." or by
  dragging.
- **Accessibility**: screen reader support through AccessKit (roles, names
  and states for rows, trees, tabs, buttons and notifications; the caret
  line of the editor, terminal, diff and merge editor as text), High
  Contrast Dark and High Contrast Light themes that follow Windows high
  contrast mode, a reduce motion setting, `F6` and `Shift+F6` to cycle the
  editor, side bar, panel and status bar, visible focus rings, Accessibility
  Help (`Alt+F1`), and spoken announcements for notifications, problems
  after a save, debugger stops, test results and find counts.
- **Multi-root workspaces**: add several folders to a workspace, save it as
  a `.code-workspace` file and open it again. The Explorer shows each folder
  as a root; quick open, find and replace in files, Source Control, tasks,
  launch configurations and language servers cover every folder. Settings
  gain a Folder layer, and `${workspaceFolder:name}` works in tasks and
  launch configurations.
- **File nesting** in the Explorer, like VS Code's
  (`explorer.fileNesting.enabled`, `.expand`, `.patterns`), off by default,
  with patterns for TypeScript and JavaScript outputs, `tsconfig`, the
  `package.json` lock files and `Cargo.lock`.
- **Command line**: `halo [options] [paths...]` with `file:line[:column]`,
  `--goto`, `--diff`, `--new-window`, `--reuse-window`, `--wait`, `--add`,
  `-` for standard input, `--help` and `--version`. A second `halo` hands its
  files to the running window. `halo --wait` works as a git editor. The
  Windows installer puts `halo` on the user PATH.
- **Persistent terminals**: terminals come back after a restart in their
  places, with their name, colour, folder, recent commands and the last 1000
  lines of history.
- Output panel (`Ctrl+Shift+U`) with Git, Language Servers, Debug, Tasks and
  Halo channels.
- Notifications with severity, actions and progress, a bell in the status
  bar and a Notification Center with Do Not Disturb.

### Changed

- Dragging a tab moves the tab itself, and the other tabs slide apart to show
  where it will land. `Esc` cancels the drag.
- Files open off the interface thread. Binary files show a notice instead of
  text, and files above `large_mb` (20 MB by default) open in a light mode.
- Pinned tabs are restored after a restart.
- The reference counts above declarations shimmer their own text while they
  load.
- `F5` starts debugging when the folder has a `launch.json`, and runs the file
  otherwise. `Ctrl+F5` runs without debugging.
- Entries in `keybindings.json` now add to the default bindings instead of
  replacing them, as in VS Code.
- Dim text, comments and several colours of the Dark and Light themes are
  darker or lighter, so that text reaches a 4.5:1 contrast ratio.
- `F6` pauses the debugger only while the program is running; otherwise it
  moves focus to the next part of the window.
- `Ctrl+Alt+B` toggles the secondary side bar, where the AI view lives.
  "Toggle AI Panel" stays in the command list without a default key.
- The theme button in the status bar opens the theme picker. `Ctrl+T` still
  switches between light and dark.

### Fixed

- Long commit subjects in the Timeline are shortened with an ellipsis, so the
  author and age stay visible.
- The window no longer flickers while Halo starts: it stays hidden until its
  first frames are drawn.

## [0.1.3] - 2026-11-20

### Added

- **Timeline** section under the outline: the commits that changed the active
  file, newest first, with a refresh button. Click a commit to open its diff
  of the file.
- The source control sections (changes, pull requests and commits) can be
  minimized and resized by dragging the line between them.
- Keyboard Shortcuts in the settings are grouped by category.
- Links in hover descriptions open when clicked.
- Loading placeholders in the outline, the timeline, source control, search
  results, the reference counts above declarations, and the accounts and WSL
  lists. The outline shows a spinner while its symbols load.
- Animations for collapsing folders in the explorer, for the explorer root,
  and for opening and closing the side bar sections. Settings checkboxes are
  animated.

### Changed

- The outline opens and closes without an active file and says why it is
  empty.
- Notices such as "No definitions found." go away after a few seconds. File
  errors stay until they are dismissed.
- Saving a file, or pressing the save key on an unchanged one, keeps its
  preview tab.
- The problems item in the status bar, and `Ctrl+Shift+M`, close the Problems
  panel when it is already showing.

## [0.1.2] - 2026-11-20

### Added

- Word wrap, code folding by indentation, sticky scroll, rulers, visible
  whitespace, bracket pair colors and indentation guides.
- Brackets and quotes close as they are typed and wrap a selection.
- Find and replace: regular expressions, find in selection and preserve case.
- Search in files in the side bar: replace in files, include and exclude
  globs, a results tree and the results as a tab.
- Line operations: move, copy, delete, join, and insert a line above or below.
  Block comment toggle.
- Box selection, select all occurrences, expand and shrink selection, and
  cursor undo.
- Snippets with tab stops from `snippets.json` and from language servers,
  word completion in files without a server, and Emmet abbreviations.
- Status bar pickers for the language, indentation, encoding and line ending.
- Trim trailing whitespace and insert a final newline on save.
- Hot exit: unsaved and untitled files are restored after a restart.
- A bar to compare, reload or overwrite when a changed file changes on disk.
- Preview tabs, a recent-file switcher on `Ctrl+Tab`, reopen closed editor,
  and Go Forward.
- Links in the text open with `Ctrl+Click`.
- Auto update: Halo installs the latest release in the background when it
  opens and uses it the next time.

### Changed

- `Ctrl+Shift+F` opens the search view in the side bar. The `%` search in the
  command bar is still there.
- A file opened from the explorer or the search bar is a preview until it is
  edited or its tab is double-clicked.
- `Ctrl+Tab` switches between recently used files instead of the next tab.
- Closing Halo with unsaved files no longer asks to save them while hot exit
  is on.

## [0.1.1] - 2026-11-20

### Added

- Language server support. Halo starts the server set for a file's extension
  when it is on `PATH`, with defaults for Rust, Python, TypeScript and
  JavaScript, Go, C and C++, C#, Java, Lua, Zig, JSON, HTML and CSS.
- Diagnostics: underlines in the text, marks in the minimap, the Problems
  panel and error and warning counts in the status bar.
- Completion, signature help and hover.
- Go to definition, declaration, type definition and implementations, peek
  definition, find all references and Go Back, with a peek list that previews
  each place.
- Reference counts on a line above functions, methods and types, which open
  the peek list when clicked, and implementation counts on interfaces.
- Rename symbol, quick fixes and refactorings, and organize imports.
- Format document and selection, and optional format on save and on paste.
- Workspace symbol search with `#` in the search bar (`Ctrl+T`), an Outline
  section under the explorer, and breadcrumbs above the text.
- Semantic highlighting, inlay hints, and call and type hierarchy.
- The Output panel shows language server messages, and the status bar shows
  server activity.
- A context menu in the editor.
- Settings for each language feature and for the server commands.
- SECURITY.md, CODE_OF_CONDUCT.md, CONTRIBUTING.md, and issue and pull request
  templates.

### Fixed

- Opening a file whose path differs only in letter case or slashes from an
  open one no longer opens it twice on Windows.

## [0.1.0] - 2026-11-20

First release.

### Added

- Window shell laid out like VS Code: title bar menus and search, activity
  bar, side bar, editor area, bottom panel and status bar, in a dark and a
  light theme.
- Explorer with git state colors and file operations.
- Editor with multi-cursor editing, find and replace, split views, a minimap,
  syntax highlighting for 68 languages, and a merge conflict bar.
- Tabs that can be dragged to reorder, moved between groups, dropped to split
  the editor, and pinned.
- Integrated terminal with profiles, split panes, a ports list and a debug
  console for Run File and tasks.
- Source control: stage, discard, commit, branches, push, pull, fetch, stash,
  a commit graph, pull requests through `gh`, and GitHub account switching.
- AI panel that runs the Claude and Codex CLIs in the open folder.
- Search bar for files, commands, text in files, symbols and lines.
- WSL mode that reopens a folder inside a WSL distro.
- Settings page with categories for the editor, files, colors, terminal
  profiles, run commands, languages and keyboard shortcuts, including an
  editor font, auto save and custom colors for each theme.
- Windows installer with a Start menu entry and Open with Halo, a portable
  Windows executable, and Linux and macOS builds.

[Unreleased]: https://github.com/joezhuo2/halo/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/joezhuo2/halo/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/joezhuo2/halo/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/joezhuo2/halo/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/joezhuo2/halo/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/joezhuo2/halo/releases/tag/v0.1.0
