# Milestones

## M1: shell, explorer, editor, terminal (done)

- Frameless window with custom title bar, menus, window buttons, edge resizing.
- Activity bar, collapsible side bar, right AI panel placeholder, bottom panel, status bar.
- Dark and light themes with ice blue accent; themed tooltips; hover and slide animations.
- Explorer: lazy tree, git colours and badges, keyboard navigation, new file/folder, rename, delete to recycle bin, copy path, reveal.
- Editor: tabs, split groups, multi-cursor, undo/redo, find/replace, go to line, comment toggle, auto-indent, bracket matching, syntax highlighting, git gutter, minimap.
- Terminal: PowerShell, Command Prompt, Git Bash, WSL profiles; new, split, kill; selection, copy/paste, scrollback; Ports tab.
- Session restore (folder, open files, panel visibility), `settings.json`.

## M2: source control (done)

- Source Control side bar with staged and unstaged changes, stage/unstage and discard (one file or all), commit box; click a change to open its diff.
- Push (publishes to `origin` when there is no upstream), pull, fetch, refresh, switch/create branch; status bar sync button does pull + push; unmodified open editors reload after each git operation.
- Repositories list (the open folder's repository plus nested ones two levels deep; the selected one drives the side bar, status bar and explorer colours), stash and pop stash.
- Commit graph of the last 200 commits with lanes for the branch and its upstream; click a commit to list its files, click a file for its diff.
- Diff view: a read-only tab with the unified diff, coloured by the Diff grammar. A side-by-side view is not built.
- Conflict editor: a bar at the bottom of any editor whose text has conflict markers, with Accept Current, Accept Incoming, Accept Both and Next; staging the file marks it resolved.
- Pull requests through `gh` in the side bar: list with mergeable/conflict and review state, check out, approve, request changes, comment, open on GitHub, create (pushes the branch first).
- Accounts button: lists `gh auth` accounts and switches the active one.

## M3: AI panel, WSL, search, settings (done)

- Right panel with Claude and Codex tabs, each with Start session, restart and stop, reusing `terminal::Terminal`. Nothing starts until Start session is clicked.
- Switch to WSL: the activity bar button lists the installed distributions; reopen the current folder in one (through `/mnt/<drive>`) or pick a folder inside it. Files are read through `\\wsl.localhost`, and git, `gh`, new terminals, Run and the AI CLIs run through `wsl.exe` in that distribution.
- Title bar search (`Ctrl+P`): files by default, `>` commands (`Ctrl+Shift+P`), `%` text in files (`Ctrl+Shift+F`), `@` symbols of the active editor (`Ctrl+Shift+O`), `:` line. `name:12` opens a file at a line.
- Settings page (`Ctrl+,`) for the common settings, `keybindings.json` (`Ctrl+K Ctrl+S`), two-key chords, extra `.sublime-syntax` grammars from a folder and file associations.
- Run menu: Run File (`F5`) runs the per-extension command from `run` in `settings.json`, and tasks from `tasks`, in one reused terminal; the Debug Console tab mirrors that terminal's output.
- Language servers (hover, completion, go to definition, Problems tab) were deferred out of v1 by the owner; they were built for 0.1.1, see M5.

## M4: packaging (written, not yet run)

- Windows installer (`installer/halo.iss`, Inno Setup): per-user, Start menu entry, "Open with Halo" on files, folders and folder backgrounds, uninstaller. Never compiled or installed: Inno Setup is not on the development machine.
- GitHub Actions (`.github/workflows/release.yml`): on a `v*` tag, builds every platform and attaches the installer, a portable `halo.exe` and the `tar.gz` binaries to a GitHub release. It can also be started by hand, which builds without releasing. Never run: nothing from this milestone is pushed.
- Linux x64 and macOS (arm64, x64) builds in the same workflow. `cargo check` passes for both targets from Windows; neither has been linked or run.
- Not built: code signing, an app icon in the executable, a macOS `.app` bundle, Linux packages (`.deb`, AppImage) and a `.desktop` entry.

## M5: language servers (0.1.1, done)

- A language server client (`lsp.rs`) and the editor features on top of it (`editor/intel.rs`): diagnostics with the Problems panel and status bar counts, completion, signature help, hover, go to definition and its relatives, peek, find references, reference counts above declarations, rename, code actions, organize imports, formatting, workspace symbols, outline, breadcrumbs, semantic highlighting, inlay hints, and call and type hierarchy.
- Not built: server code lenses, the quick fix lightbulb, incremental document sync, a hierarchy tree with more than one level, `completionItem/resolve` and `codeAction/resolve`, and language servers in WSL folders.

## M6: editing (0.1.2, done)

- Editor: word wrap, folding by indentation, auto-closing pairs, line operations, block comments, box selection, select all occurrences, expand and shrink selection, cursor undo, snippets with tab stops, word completion, Emmet, visible whitespace, rulers, sticky scroll, bracket colors, guides and clickable links.
- Find: regular expressions, find in selection and preserve case in the file, and a search view with replace in files, globs, a results tree and results in a tab (`find.rs`).
- Files: status bar pickers for language, indentation, encoding and line ending, trim and final newline on save, hot exit, and a compare, reload or overwrite bar when a changed file changes on disk.
- Tabs: preview tabs, the recent switcher, reopen closed editor, back and forward.
- Auto update (`update.rs`): installs the latest release at startup through `curl`, `tar` and the system hash tool. Never run against a real newer release.
- Not built: folding ranges and selection ranges from language servers, CSS Emmet, keyboard box selection, visual-row Home and End under word wrap, searching unsaved buffers in the search view, pickers in the command list, and encodings beyond UTF-8, UTF-16 and ISO 8859-1.

## M7: interface polish (0.1.3, done)

- Shared loaders and animation helpers in `ui.rs`: spinner, shimmer, skeleton rows, an animated checkbox, a collapsible section header and a reveal easing.
- Side bar: the outline is always collapsible with loading and empty states, and a timeline of the commits of the active file (`timeline.rs`).
- Source control: collapsible and resizable sections. Explorer: collapse animations.
- Editor: clickable links in hover text, notices that expire, a shimmer for loading reference counts, and saving keeps a preview tab.
- Settings: keyboard shortcuts grouped by category.
- Not built: skeleton rows in the explorer (folders are listed synchronously), resizing the outline and timeline, and timeline diffs for files outside the open repository.

## M8: IDE features (0.1.4, done)

Built by parallel workers on branches merged into `m7-ide`; covered by unit and headless tests, not checked by hand in the running app.

- Diffs and merges: a line diff engine (`editor/diff.rs`), the side-by-side editable diff editor with hunk staging and a gutter change peek (`editor/diff_view.rs`), and a three-way merge editor (`editor/merge.rs`, `editor/merge_view.rs`).
- Git: blame (`blame.rs`), local history in the Timeline (`history.rs`), compare, and amend, undo, merge, rebase, cherry-pick, tags, remotes, clone, init, worktrees and stashes (`git_ops.rs`, `scm_ops.rs`).
- Run: a debugger over the Debug Adapter Protocol (`dap.rs`, `debug.rs`), tasks with problem matchers (`tasks.rs`), and a test explorer with debug and coverage (`testing.rs`, `test_parse.rs`).
- Editor: live tab drag, a grid of editor groups (`editor/layout.rs`, `editor/grid.rs`), floating editor windows and new windows (`windows.rs`), Markdown preview (`editor/markdown.rs`), image tabs (`editor/image_view.rs`), background loading with binary and large file modes (`editor/load.rs`), pinned tabs in the session.
- Workbench: zen mode, full screen, centered layout, movable side bar, panel and activity bar, a secondary side bar and movable views (`workbench.rs`); output channels (`output.rs`) and notifications (`notifications.rs`).
- Explorer: multi-select, cut, copy and paste, drag and drop, Open Editors, type to filter, `files_exclude`, compact folders, file nesting (`explorer/nesting.rs`) and multi-root workspaces with `.code-workspace` files (`workspace.rs`).
- Terminal: links, find, shell integration, rename and colour, terminals in the editor area (`terminal_links.rs`, `terminal_find.rs`, `shell_integration.rs`) and persistent sessions (`terminal_restore.rs`).
- Settings: layers with workspace, folder and language settings and VS Code key names (`layers.rs`), settings search, about 50 new settings, keybinding `when` clauses (`when.rs`) and a Keyboard Shortcuts page that records chords (`shortcuts.rs`); imported VS Code color and icon themes (`theme_import.rs`, `icon_theme.rs`).
- Accessibility: AccessKit, high contrast themes, reduced motion, region cycling and announcements (`a11y.rs`).
- Command line: argument parsing and a single-instance hand-off (`cli.rs`, `ipc.rs`), with a `halo` command installed on PATH on Windows.
- Not built: font ligatures, letter spacing, format on type, tab sizing, font-glyph icon themes, `.tmTheme` themes, dragging a tab between windows, a combined Testing view across workspace roots, keeping terminal processes alive across restarts, and VS Code extensions.
