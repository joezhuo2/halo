# Changelog

All notable changes to Halo are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Halo uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.18] - 2026-11-24

Checked with unit tests and a start-up smoke test. Not yet looked at on
screen: the log viewer, hex editor, table view, the profile and trust
dialogs. Settings Sync was not run against a real gist.

### Added

- **Vim mode** (Toggle Vim Mode or Settings > Text Editor > Vim Mode, off by
  default): normal, insert, visual and visual line modes, counts, motions
  (`w b e W B E 0 ^ $ gg G f t F T ; , { } % n N * #`), operators
  `d c y > <`, `x X D C s S Y p P r J ~ u .`, `/` and `?` search and `:w`,
  `:q`, `:wq`, `:wa` and `:<line>`. The status bar shows the mode and a
  block cursor marks normal mode.
- **Log viewer for huge files:** files over 256 MB open in a viewer that
  streams them from disk with a background line index, whole-file search
  and Follow for growing logs. View: Open Active File in Log Viewer opens
  smaller saved files the same way.
- **Hex editor:** Open in Hex Editor on the binary file notice, or View:
  Reopen Active File in Hex Editor, edits bytes in place (hex or text
  column), with undo and `Ctrl+S`.
- **CSV table view:** CSV: Toggle Table View shows delimited files as a
  table with a fixed header; click a header to sort.
- **Line history:** Git: Show History of Selected Lines narrows the
  Timeline to the commits that changed the selection (`git log -L`).
- **Profiles:** named sets of settings, keyboard shortcuts, snippets and
  theme, with New, Switch and Delete commands and the active profile in the
  status bar.
- **Settings Sync:** upload settings and every profile to one secret GitHub
  gist through `gh`, and download them on another machine with a backup of
  the local files.
- **Workspace Trust:** untrusted folders open in Restricted Mode, without
  their workspace settings, tasks or launch configurations, until you trust
  them. Folders in the recent list are trusted on upgrade.

### Changed

- Files over 256 MB open in the log viewer instead of a notice page.

## [0.1.17] - 2026-11-23

Checked with unit tests. Not yet looked at on screen: the Open Editors tree,
the hidden tab strip and the run configuration form.

### Added

- **Scratch files:** Scratch: New Scratch File... creates `scratch-N.<ext>`
  for the chosen language in a per-folder directory under Halo's settings
  folder, outside the project and git; Scratch: Open Scratch File... lists
  them newest first.
- **Vertical tabs:** Open Editors can group open files by folder (header
  button, View: Toggle Open Editors Tree or the Open Editors Tree setting),
  and the Show Tabs setting (`workbench.editor.showTabs`) hides the tab
  strip above editor groups.
- **Run configurations in a form:** Debug: Edit Configuration in a Form...
  and Tasks: Edit Task in a Form... edit name, type, request, program,
  arguments and folder of a launch configuration, or label, type, command,
  arguments and group of a task, and write them back to `launch.json` or
  `tasks.json`. Properties the form does not show are kept; comments are
  dropped when the file is rewritten.

## [0.1.16] - 2026-11-23

Checked with unit tests. Not yet looked at on screen: the title bar tint and
the zoomed editor group.

### Added

- **TODO list:** Search: Find TODO and FIXME Comments runs a workspace
  search for TODO, FIXME, HACK, XXX and BUG and shows the hits in the Search
  view, grouped by file.
- **Window color:** Window: Set Color... picks a title bar tint for the open
  folder and writes it to the folder settings as `peacock.color`; the
  Window Color setting does the same from the settings page.
- **Editor group zoom:** `Ctrl+K Ctrl+=` and `Ctrl+K Ctrl+-` change the font
  size of the active editor group only; View: Reset Editor Group Zoom resets
  it.
- **Named layouts:** Layouts: Save Open Editors As..., Layouts: Switch...
  and Layouts: Delete... save and restore the open tabs and splits under a
  name, per folder. Tabs with unsaved changes and terminal tabs stay open
  when switching.

## [0.1.15] - 2026-11-23

Checked with unit tests and headless UI tests. Not yet looked at on screen:
the gutter bookmark icon, the jump labels and the macro status bar item.

### Added

- **Bookmarks:** `Ctrl+Alt+K` toggles a bookmark on the cursor line, shown
  in the gutter. `Ctrl+Alt+L` and `Ctrl+Alt+J` jump to the next and previous
  bookmark across files; Bookmarks: Label names one, Bookmarks: List picks
  one from all files, and Bookmarks: Clear All removes them. Bookmarks move
  with edits and are saved per folder.
- **Jump to a visible word** (`Alt+J`): each word on screen gets a
  two-letter label; typing it moves the cursor there and `Esc` cancels.
- **Clipboard history** (`Ctrl+K Ctrl+V`): pick one of the last 20 texts
  copied or cut in an editor and paste it.
- **Macros:** `Ctrl+K Ctrl+R` starts and stops recording keystrokes and
  commands, with a Recording macro item in the status bar, and
  `Ctrl+K Ctrl+P` plays the recording back.
- **Compare Folders...:** picks two folders and lists the files that differ
  or exist on one side only; picking a changed file opens its diff. Compare
  Folders: Show Last Comparison scans the same pair again.

## [0.1.14] - 2026-11-23

Checked with unit tests and headless UI tests, with fake programs standing in
for the AI CLIs. None of the AI features has been run against a real Claude,
Codex, Gemini or OpenCode CLI or looked at on screen yet, and the installer's
new Open with entries have not been installed and tried.

### Added

- **Inline suggestions:** `Alt+\` asks the AI CLI for a completion at the
  cursor and shows it as ghost text, with further lines below the cursor
  line. `Tab` accepts it, `Ctrl+Right` accepts a word, `Esc` dismisses it.
  Setting: Editor > Inline Suggestions (off by default, since each request
  uses the CLI's quota).
- **Next edit suggestions:** `Alt+]` predicts the next edit in the file from
  your recent ones and shows it in place; `Tab` jumps to it and `Tab` again
  applies it. Setting: Editor > Next Edit Suggestions (off by default).
- **Inline chat:** `Ctrl+I` in an editor asks the CLI to rewrite the
  selected lines, or add code at the cursor, and shows the change as an
  inline diff to accept (`Ctrl+Enter`), discard (`Esc`) or refine with a
  follow-up prompt.
- **Agent Changes** in the AI panel: the files a session added, edited or
  deleted since it started (untracked files included), each with its diff
  and Keep or Undo, or Keep All and Undo All. Files you edit yourself during
  a session are listed too. Commands: Agent Changes: Keep All, Undo All,
  Show, Reset Baseline.
- **Gemini and OpenCode in the AI panel:** two more tabs beside Claude and
  Codex that run the `gemini` and `opencode` CLIs, with an Install button
  (`npm install -g`) when the CLI is missing.
- **Open with Halo from Windows:** the installer lists Halo in the "Open
  with" menu for common text and code files, so it can be picked once or set
  as the default app, also under Settings > Apps > Default apps.

### Fixed

- Opening a folder that sits inside a bigger Git repository (for example a
  project folder under a versioned `Documents`) no longer makes that outer
  repository the active one. Source Control says a repository was found in
  the parent folder and offers **Open Repository**, as VS Code does.

## [0.1.13] - 2026-11-23

Checked with unit tests and against a real PowerShell with shell
integration on Windows (suggestions, accepting one, and the git typo quick
fix). The popups, lightbulb and sticky bar have not been checked on screen,
the push and port quick fixes have not been run for real, and images were
only tested through the decoder, since Windows drops them (see below).

### Added

- **Terminal suggestions** as you type in a terminal with shell integration:
  commands from PATH and history, subcommands and flags from a built-in list
  for git, cargo, npm, pnpm, yarn, docker, pip, go, kubectl, gh and common
  shell tools, and folders and files from the shell's folder. `Tab` accepts,
  `Down`/`Up` pick, `Enter` accepts a picked row, `Escape` closes and
  `Ctrl+Space` opens the list. Setting: Terminal > Suggest.
- **Terminal quick fixes:** a lightbulb on the last finished command, or
  `Ctrl+.`, offers to run `git push --set-upstream ...` after a push without
  an upstream, run the command git suggests for a mistyped one, open the
  pull request page that `git push` printed, or stop the process listening
  on a port reported as already in use.
- **Terminal sticky scroll:** the command whose output fills the terminal
  stays pinned to the top row; click it to scroll to the command. Setting:
  Terminal > Sticky Scroll.
- **Terminal images:** Sixel and iTerm inline images (`imgcat`,
  `img2sixel`, `chafa -f sixel`) are shown where they are printed. The
  terminal answers device attribute and pixel size queries so such programs
  can detect it. Setting: Terminal > Images. Linux and macOS only: the
  Windows pseudo console drops Sixel and iTerm image sequences.

## [0.1.12] - 2026-11-23

Checked with unit tests and headless UI tests against temporary repositories
on Windows; the `gh` review calls and the `claude`/`codex` message calls have
not yet been run against the real services.

### Added

- **Interactive rebase** from a commit in the graph ("Interactive Rebase
  from Here...") or "Git: Interactive Rebase...": reorder commits by
  dragging or with `Alt+Up`/`Alt+Down`, and set pick, reword, edit, squash,
  fixup or drop per commit. Edit stops and conflicts use the existing
  Continue, Abort and Skip controls.
- **Compare branches or commits:** "Git: Compare References...", "Compare
  with..." on a graph commit, or "Compare Branch with..." in the branch
  menu lists the files changed between any two refs; click one for its
  diff. Switch between two-dot and three-dot.
- **Incoming and Outgoing Changes** rows with counts above the commit graph
  (click to pull or push), and markers on commits not yet pulled or pushed.
- **Hover blame:** hover the inline blame or the status bar blame item for
  the full commit message, Open Changes, and Open on GitHub, GitLab or
  Bitbucket. Also "Git: Open Current Line's Commit on Remote".
- **Generate commit message** from the staged changes (or all changes) with
  the Claude or Codex CLI, from the sparkle button next to Commit or "Git:
  Generate Commit Message". Click again to cancel.
- **Pull request review in the diff:** "Review in Diff" on a pull request
  opens its changed files as diff tabs with its review threads on their
  lines. Reply, resolve or unresolve threads, and comment on any line with
  the hover `+` or "Pull Request: Add Comment on Line".

## [0.1.11] - 2026-11-23

Checked with unit tests and fake debug adapters on Windows; not yet tried
against real adapters.

### Added

- **Several debug sessions at once.** Starting a configuration while
  another runs starts a second session. The Call Stack groups threads by
  session, the debug toolbar gets a session picker, and Debug Console lines
  are tagged with the session name. Variables, Watch, stepping and hover act
  on the focused session; breakpoint changes go to all of them.
- **Compound launch configurations:** `compounds` in `launch.json`, with
  `stopAll` and `preLaunchTask`.
- **Inline values:** while paused, `name = value` appears after the lines of
  the current frame. Turn it off with `debug.inlineValues`.
- **Run to Cursor** and **Jump to Cursor** (set next statement) in the Run
  menu, the editor context menu and the command palette.
- **Function breakpoints** from the Breakpoints section's `+` or
  "Debug: Add Function Breakpoint…", saved per workspace.
- **Data breakpoints:** "Break on Value Change" on a variable stops when its
  value changes; they last for the session.
- **Disassembly view** from the Call Stack or "Debug: Open Disassembly
  View", with the current instruction highlighted; stepping while it is
  focused steps one instruction.
- **Memory view:** "View Binary Data" on a variable, or "Debug: Open Memory
  View…" with an address or expression, shows a hex dump refreshed on each
  stop.
- **Attach to a process:** `${command:pickProcess}` and
  `${command:pickMyProcess}` in `launch.json` open a filterable list of
  running processes and fill in the chosen pid.

### Fixed

- A keyboard test could fail at random when the test suite ran in parallel.

## [0.1.10] - 2026-11-22

Checked with tests and timed startup runs on Windows.

### Changed

- **The Windows program is now `halo-ide.exe`.** The NVIDIA driver ran any
  `halo.exe` on the discrete GPU through a game profile, which cost about
  3 s at startup and 100-150 MB of memory. The installer installs the new
  name and removes the old one; shortcuts pinned to `halo.exe` need pinning
  again. Linux and macOS archives still contain `halo`.
- **Faster startup with many or large workspace folders.** The explorer no
  longer reads collapsed folders in full, folders without their own settings
  file reuse the base settings, and shell profiles are probed once instead
  of on every settings load. A session with 20 folders and 50 tabs settles
  in 1.17 s instead of 1.65 s.
- **Restored tabs load their file when first shown,** so a session with many
  open tabs uses 15-20 MB less memory. A tab not yet shown shows a spinner
  for one frame the first time you switch to it.
- **Explorer refresh** after a save or a finished terminal command skips
  folders that have not changed.

### Fixed

- The Windows installer's folder for the `halo` command held a stray control
  character where `\bin` belonged.

## [0.1.9] - 2026-11-22

Checked with tests on Windows. Not yet tried by hand in the running app or
against real language servers.

### Added

- **Folding from language servers**, plus `#region` / `#endregion` markers.
  Fold Level 1–7 (`Ctrl+K Ctrl+1` to `Ctrl+K Ctrl+7`), Fold and Unfold
  Recursively (`Ctrl+K Ctrl+[`, `Ctrl+K Ctrl+]`) and Fold All Block Comments
  (`Ctrl+K Ctrl+/`). Expand Selection uses the server's selection ranges
  when it has them.
- **Quick fix lightbulb** in the gutter when the cursor line has code
  actions (`editor.lightbulb.enabled`).
- **Server code lenses** such as rust-analyzer's Run and Debug show beside
  Halo's reference counts. Run types the command into the terminal, Debug
  debugs a single cargo test.
- **Linked editing:** renaming an HTML, XML, SVG, Vue, Svelte, JSX or TSX tag
  renames its partner, from the server or a built-in matcher
  (`editor.linkedEditing`, on by default).
- **Format on type** from the language server (`editor.formatOnType`, off by
  default).
- **Color decorators:** a swatch before each color value, from the server or
  a built-in scan of hex, `rgb()` and `hsl()`. Click it, or run Show Color
  Picker, to change the color in the value's own format
  (`editor.colorDecorators`).
- **Unicode highlighting** of invisible characters and ASCII look-alikes,
  with VS Code's tooltips (`editor.unicodeHighlight.invisibleCharacters`,
  `editor.unicodeHighlight.ambiguousCharacters`).
- **Inline peek:** the peek list and the change peek open between lines and
  push the text below down instead of covering it.
- **Font ligatures** (`editor.fontLigatures`), **letter spacing**
  (`editor.letterSpacing`) and a **smooth caret** animation
  (`editor.cursorSmoothCaretAnimation`).
- **Subword navigation:** Cursor Word Part Left/Right and their Select forms,
  and Delete Word Part Left/Right (`Ctrl+Alt+Backspace`, `Ctrl+Alt+Delete`).
- **Copy with syntax highlighting** on Windows: copied code keeps its colors
  in Word, Outlook and OneNote (`editor.copyWithSyntaxHighlighting`).
- **Markdown:** path and heading completions inside links, warnings for
  broken links (`markdown.validate.enabled`), `Ctrl+V` of a clipboard image
  saves `image.png` next to the file and links it (Windows), and `Shift`+drop
  inserts links to the dropped files.
- **Search editor:** search results open as an editable tab whose header
  reruns the search (`Enter` or `Ctrl+Shift+R`), with `Alt+C/W/R/L` toggles
  and VS Code `.code-search` files. New Search Editor is in the Go menu and
  the search panel; Open Results in Editor is `Alt+Enter` there.
- **Accessible diff viewer:** `F7` / `Shift+F7` in a diff tab list one change
  at a time, line by line, with screen reader labels; `Enter` jumps to the
  line.
- **Audio cues** on Windows for errors, warnings, breakpoints and folds on
  the cursor line, and for inserted and removed lines in the accessible diff
  viewer (`accessibility.signals.*.sound`,
  `accessibility.signalOptions.volume`).

### Fixed

- Markdown preview tables size their columns to the content, wrap long cells
  and stay inside the preview.
- Markdown preview code blocks have a Copy code button.
- `Ctrl+Shift+V` in a Markdown editor opens the preview instead of pasting;
  it still pastes elsewhere.

## [0.1.8] - 2026-11-22

Checked with tests on Windows, including WSL tests against a real clangd and
git in Ubuntu. Not yet tried by hand in the running app.

### Added

- **Open Recent** (`Ctrl+R`, or File > Open Recent) picks a recent folder or
  file.
- **Line commands**: Sort Lines Ascending / Descending, Delete Duplicate
  Lines and Reverse Lines.
- **Transform case**: Transform to Uppercase, Lowercase, Title Case, Snake
  Case, Camel Case and Kebab Case.
- **More cursors**: `Shift+Alt+I` adds a cursor at the end of every selected
  line, and `Ctrl+K Ctrl+D` moves the last added occurrence to the next
  match.
- **Go to Bracket** (`Ctrl+Shift+\`) jumps to the matching bracket, and
  Select to Bracket selects the brackets and what is between them.
- **Go to Last Edit Location** (`Ctrl+K Ctrl+Q`).
- **Column selection mode**: drags select a box, and `Ctrl+Shift+Alt+Arrow`
  grows a box from the keyboard.
- **Symbol highlights**: resting the caret on a name highlights its other
  uses in the file, from the language server's document highlights.
- **Find history**: `Up` and `Down` in the find box recall earlier searches.
- **Problems filter**: filter the Problems panel by text, by severity and to
  the active file.
- **Outline** follows the caret and sorts by position, name or kind.
- **Wrap Tabs** setting (`workbench.editor.wrapTabs`) wraps editor tabs onto
  more rows instead of scrolling them.
- **Lock an editor group** so new files open in another group, with a lock
  button and Lock Group on the tab menu. The lock is kept across restarts.
- **Ctrl+hover** underlines what `Ctrl+Click` follows, and `Ctrl+Click` opens
  paths to existing files.
- `docs/measure-startup.ps1` times startup and idle memory of the release
  build.

### Changed

- **Faster startup**: the window opens before the session is restored, and
  the Explorer, editor and terminal show skeletons until their content
  loads. The window is on screen in 570 ms instead of 829 ms with a local
  folder, and in 569 ms instead of 1011 ms with a WSL folder (see
  Performance in the README).
- The workspace folder is checked on a background thread, so a sleeping WSL
  folder no longer holds back the window.

### Fixed

- Hovering an error squiggle that sits on punctuation or has an empty range
  showed nothing; it now shows the message (for example clangd's CUDA error
  at the top of `.cu` files).
- The Explorer could not delete WSL folders that contain Linux symlinks
  (#1); it now deletes them inside the distro. It refuses to delete a distro
  root.

## [0.1.7] - 2026-11-21

Checked with tests on Windows, including WSL tests against a real clangd and
git in Ubuntu. Not yet tried by hand in the running app.

### Added

- **Error markers**: a file with errors shows in red, and one with only
  warnings in yellow, on its editor tab (with a dot), in the Explorer and in
  Open Editors. Folders that contain it are tinted too.
- **Resizable side bar sections**: drag the line between Open Editors, the
  folder tree, Outline and Timeline, or between Stashes, Changes, Pull
  Requests and Commits. Double-click the line to reset. Explorer section
  sizes are remembered across restarts.
- **Suggestion Rows** setting: the suggestion list shows 5 rows by default
  (1 to 20), and opens above the line when there is no room below.
- **Permanent delete**: when an item cannot be moved to the Recycle Bin (for
  example in a WSL folder), Halo asks whether to delete it permanently.

### Changed

- **Popups**: Enter triggers the highlighted default button and Escape
  cancels. Multi-line fields keep Enter for a new line and submit with
  Ctrl+Enter.
- **Commits and Timeline** load the 10 newest entries first and load more as
  you scroll. The Outline draws only the visible rows.
- **Git in WSL folders** refreshes several times faster: status reads run in
  parallel, so the spinner settles and change bars appear soon after saving.
- Clicking a Git change bar, and Next / Previous / Stage / Revert Change, no
  longer wait about half a second in WSL folders.
- Edits reach the language server in the frame they are typed, so errors and
  warnings update as fast as the server allows.
- Touchpad scrolling, and all wheel scrolling with Smooth Scrolling off,
  stops where the input stops instead of drifting on.

### Fixed

- Deleting a file in a WSL folder did nothing.
- A Git status call that fails or runs longer than 30 seconds ends loading
  and shows the error, instead of spinning forever.
- Git letters in the Explorer and the diff view work for folders opened
  through `\\wsl$\`.
- Terminal: Ctrl+Backspace deletes the previous word in WSL, bash, zsh and
  fish, and a keyboard selection clears on the next key press or click.
- The hover tooltip no longer flickers on a token with a warning or error.
- Accepting a completion that adds an `#include`, or any edit above the
  view, no longer shifts the text in the editor.

## [0.1.6] - 2026-11-21

Checked with tests on Windows, including WSL tests against a real clangd and
git in Ubuntu. Not yet tried by hand in the running app.

### Added

- **Language servers in WSL**: files in a WSL folder get their language
  server from inside the distro (for example clangd), with diagnostics while
  you type, Go to Definition and completions such as `size()`.
- **Peek context lines**: the peek definition/references preview shows 7
  lines above the target by default (setting "Peek Context Lines") and
  scrolls with the mouse wheel.
- **Git changes in the minimap**: added, modified and deleted lines show in
  the minimap gutter and its overview lane.
- **Terminal selection keys**: Shift+Arrow and Ctrl+Shift+Arrow select text
  (PowerShell keeps its own handling), and Ctrl+Delete deletes the next word.

### Changed

- **Commit with nothing staged** stages all changes and commits them,
  instead of a disabled button.
- **Staged lines** show dimmed in the gutter.
- **Diff view**: the ruler shows removed and added lines in separate lanes,
  and change highlights are stronger.
- **Source Control** refreshes as soon as a terminal command finishes, with
  a spinner while Git status loads.
- Recent folders and files are shared by all windows, so the jump list keeps
  folders opened in another window, WSL folders included.

### Fixed

- Deleting a large folder no longer freezes the window: it moves to the
  Recycle Bin in the background.
- The gutter change bar no longer flashes while typing or switching windows.
- Clicking the Outline or Timeline header toggles it again, instead of
  starting a drag of the view.
- A file and its diff open side by side no longer clash ("widget ID" error).
- Clicking a change in Source Control opens its diff even when no editor is
  open.

## [0.1.5] - 2026-11-21

Checked on Windows with tests and by watching the window state at startup.
The macOS changes are type-checked but have not been run on a Mac.

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

[Unreleased]: https://github.com/joezhuo2/halo/compare/v0.1.18...HEAD
[0.1.18]: https://github.com/joezhuo2/halo/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/joezhuo2/halo/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/joezhuo2/halo/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/joezhuo2/halo/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/joezhuo2/halo/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/joezhuo2/halo/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/joezhuo2/halo/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/joezhuo2/halo/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/joezhuo2/halo/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/joezhuo2/halo/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/joezhuo2/halo/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/joezhuo2/halo/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/joezhuo2/halo/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/joezhuo2/halo/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/joezhuo2/halo/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/joezhuo2/halo/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/joezhuo2/halo/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/joezhuo2/halo/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/joezhuo2/halo/releases/tag/v0.1.0
