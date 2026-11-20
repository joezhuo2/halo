# Handoff: end of milestone 3

Written 2026-11-20. Read [requirements.md](requirements.md), [decisions.md](decisions.md) and [architecture.md](architecture.md) first; [milestones.md](milestones.md) has the scope of what comes next.

## State

- Repository: `C:\Users\Joe\Documents\halo`, remote `origin` = https://github.com/joezhuo2/halo.
- Branches: `main` was created from `m1-shell` (all of milestone 1) and pushed. Milestone 2 is on `m2-scm` and milestone 3 on `m3-ai` (branched from `m2-scm`); neither is pushed or merged.
- Build: `cargo build` is warning-free. `cargo test` passes 28 tests.
- Run: `cargo run --release`, or `target\release\halo.exe [folder-or-file]`.
- Config written by the app: `%APPDATA%\halo\settings.json`, `session.json` and, once opened, `keybindings.json`. eframe also stores window size and position under `%APPDATA%\Halo`.

## What works (checked by running the app)

Checked on Windows 11 by launching the build, sending keystrokes and capturing the window:

- Window shell, menus bar, activity bar, status bar with repo/branch, dark and light themes, themed tooltip.
- Explorer tree with git colours (modified folder in yellow with a dot).
- Editor: opening files, Rust highlighting, line numbers, git gutter, minimap, typing, Enter, undo, `Ctrl+End`, save with `Ctrl+S` (verified on disk, CRLF preserved), find bar with match count, split editor, `Ctrl+D` multi-cursor.
- Terminal: PowerShell starts in the folder, accepts typed commands, shows colours, runs `git`.

Checked for milestone 3 in the test mode described in [architecture.md](architecture.md) (window captures and posted keys, no mouse):

- Search: the `>` list with shortcuts, typing a filter, Enter running the picked command (the theme toggled); `%` text search with results, arrow down and Enter opening the file at the hit's line.
- Settings page opening with the current values.
- Run File: `F5`'s command saved and ran a Python file in a new PowerShell terminal, and the Debug Console showed the same output.
- WSL: a folder under `\\wsl.localhost\Ubuntu-24.04` opened with explorer git colours and badges, branch in the status bar, the WSL indicator, a bash terminal started in that folder, and Source Control listing changes and commits.
- AI panel: the Claude tab with its Start session screen.

## Not verified by hand

Mouse interactions were not exercised (only keyboard input and screenshots). Covered by unit tests at most, or not at all:

- Explorer: context menu, new file/folder, rename, delete to recycle bin, keyboard navigation, reveal.
- Editor: mouse selection, double/triple click, Alt+click cursors, minimap click/drag, tab close button and the unsaved-changes dialog, replace/replace all, go to line, comment toggle, light theme colours in code.
- Terminal: split, kill, profile menu, Git Bash and WSL profiles, mouse selection and copy, wheel scrollback, Ports tab.
- Menus: every item dispatches a command, but only keyboard paths were run.
- Window: drag to move, edge resize, maximize/restore, minimize. Whether a maximized frameless window covers the taskbar is unknown.
- Unsaved-changes prompt on exit.
- Source Control (milestone 2): the side bar was checked by screenshot only. Stage, discard prompt and commit are covered by a headless egui test, push/pull/switch by a test against a local bare remote. Not run by hand: the repositories list, stash, clicking a commit, the branch menu, the Create branch and Discard dialogs, moving untracked files to the recycle bin, and push/pull against GitHub with real credentials. The pull request list was seen loading from GitHub (an empty list); pull request rows, their menu, review and create dialogs, the accounts menu and account switching have never run against real data. Diff tabs and the conflict bar are covered by headless tests and one screenshot.
- AI panel: a test runs a missing CLI through the real PTY and checks the ended session is kept. Never run: Start session with the real `claude` or `codex` (on Windows or in WSL), restart, stop, switching tabs with sessions running, copy and paste.
- Search: the file mode with a typed query, `@` symbols and `:` line were only unit-tested; clicking a result and clicking outside to close were never run.
- Key chords and `keybindings.json`: the binding table is unit-tested, but no chord was pressed in the running app, the status bar chord notice was never seen, and editing `keybindings.json` then saving was never run.
- Settings page: no control was changed through the page. Loading a grammar folder and a file association were never run. The notice toasts were never seen.
- Run: tasks, the Run menu, a missing run command, and Run File inside a WSL folder were never run.
- WSL: the Switch to WSL menu (distribution list, Reopen Folder, Open Folder, Reopen in Windows) was never opened; push, pull and pull requests inside WSL were never run.

## Known gaps and limits

**Memory target is missed.** Release build, maximized on a high-DPI display: about 90 MB private working set with a folder open, about 100 MB with a file and a terminal (133 MB working set). The target is 80 MB. The largest single piece is the AMD OpenGL driver (`atio6axx.dll`, 62 MB image) loaded into the process. Ideas:

1. Tried 2026-11-20, rejected: the wgpu renderer with default settings. Private working set with a folder open, same machine and window: OpenGL (glow) 84 MB; wgpu on DX12 273-281 MB, Vulkan 283 MB, GL 160 MB. The binary also grows from 9.6 MB to 14.3 MB. wgpu's low-memory device hints were not tried. Halo stays on glow.
2. Measure at a smaller window size to see how much is framebuffer.
3. Check which GPU is selected on hybrid laptops; an integrated GPU driver may be smaller.
4. Trim the working set when idle (`SetProcessWorkingSetSize`), which improves the Task Manager number but not real use.

Other limits:

- No hover, completion or diagnostics (deferred by decision 18). Problems tab is an empty state.
- Editor: no word wrap, no regex find, no auto-closing brackets, no IME composition, no word-wise drag after double click; a tab renders as four spaces wide rather than aligning to tab stops; git gutter marks reflect the file as last saved; lines are drawn up to 5000 characters.
- Syntax: syntect's bundled grammars only. TypeScript/JSX fall back to JavaScript, `.ino`/headers to C++; TOML and some others show as plain text. Add grammars with the `grammars` folder and map extensions with `associations`; both are read once at startup.
- Terminal: no bold or italic font variants, no mouse reporting to terminal programs, no scrollbar, wide characters are placed per cell without width correction. Shortcuts bound globally (`Ctrl+B`, `Ctrl+J`, `Ctrl+W`, `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+,`) do not reach the shell.
- Source Control: only one repository is active at a time, so explorer colours follow the repository selected in the side bar. The selection is not saved in the session. Repository discovery skips dot folders, `node_modules` and `target`. The graph shows the first 8 lanes; wider histories draw on top of the last lane. `git pull` uses the user's git config, and after a pull that conflicts, resolve each file with the conflict bar, stage it and commit. Diffs are unified text only and binary files show git's one-line notice. Creating a pull request always pushes to `origin` and uses the repository's default base branch. `gh` commands inherit the side bar's busy state and errors. Editors with unsaved changes are not reloaded after a git operation. A git command that waits for input (an SSH passphrase, a GPG pinentry) keeps the side bar busy until it ends.
- AI panel: the commands are fixed (`claude`, `codex`) with no extra arguments; in a WSL folder they start through `bash -lic` in the distribution. A session keeps the folder it was started in when another folder is opened. A CLI that is not installed shows the shell's "not recognized" message and the session-ended notice.
- Explorer: no file watcher (it refreshes when the window regains focus and after saves), no drag and drop, rename does not preselect the name stem.
- Key bindings: a chord has exactly two keys. `Ctrl+K` starts a chord even in the terminal, so the shell never receives it. A key that cannot be parsed in `keybindings.json` drops that entry without a notice. The shortcut text in menus comes from egui (`Ctrl+Comma`).
- Search: the file list and text search need `git` on `PATH` (without a repository the file list falls back to walking the folder). Text search is case-insensitive plain text, needs two characters, and shows at most 240 hits. Symbols depend on the grammar marking names, so some languages list few or none. Commands that need a parameter (a task, a terminal profile) are not in the `>` list.
- Run: the command is typed into a shell, so quoting follows that shell; the defaults suit PowerShell and bash. The Debug Console is a text copy of the last 500 lines and is not interactive. There are no breakpoints (decision 33).
- WSL: Windows only. Deleting cannot use the recycle bin on a WSL folder, so it fails with an error. `gh` must be installed inside the distribution for pull requests; the Accounts menu still manages the Windows `gh`. Opening a WSL folder starts the distribution, which can take a few seconds. A file from a WSL folder opened while a Windows folder is open runs with its Windows path.
- Settings page: lists and maps are edited in `settings.json`; grammar and association changes need a restart.
- The caret blink repaints at 20 fps for 30 seconds after the last input, then stops.
- Linux and macOS: code is portable in intent but has never been compiled there.

## Notes from this session

- Subagents with worktree isolation were tried and abandoned: the harness created worktrees of the parent `Documents` repository instead of Halo. Those stray worktrees and branches were removed; nothing else in `Documents` was touched.
- Milestone 3 testing first launched the build in front of the owner's work; some of the owner's key presses and clicks landed in those test windows (files opened in the test instance, nothing was saved). One test toggled the theme, which rewrote `settings.json` in the new format before the theme was set back to dark. Later runs used the test mode, which stays out of the way.
- A throwaway repository was created and removed under the home folder of the `Ubuntu-24.04` distribution.
- During testing the debug build was launched many times and closed by process id. Early on, one cleanup command stopped every `halo` process by name, which would also have closed a copy you were running.
- `%APPDATA%\halo\session.json` was rewritten once during testing to drop two bad entries; `settings.json` was switched to the light theme for a screenshot and restored.

## Suggested next steps

1. Run the app by hand through the "Not verified" list and fix what breaks.
2. Memory: the renderer stays on glow (see above); the remaining ideas are 2 to 4.
3. Milestone 2 is complete on `m2-scm`. Run it by hand against a real GitHub repository (see "Not verified").
4. Milestone 3 is complete on `m3-ai`. Run it by hand through the milestone 3 entries in "Not verified", then merge `m2-scm` and `m3-ai` into `main` and push.
5. App icon: the owner will supply one. Set it with `ViewportBuilder::with_icon` in `src/main.rs` and use it in place of the ring in the title bar, the welcome page and the About dialog.
6. Milestone 4 from [milestones.md](milestones.md): installer and release pipeline.
