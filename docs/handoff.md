# Handoff: end of milestone 1

Written 2026-11-20. Read [requirements.md](requirements.md), [decisions.md](decisions.md) and [architecture.md](architecture.md) first; [milestones.md](milestones.md) has the scope of what comes next.

## State

- Repository: `C:\Users\Joe\Documents\halo`, remote `origin` = https://github.com/joezhuo2/halo.
- Branches: `main` was created from `m1-shell` (all of milestone 1) and pushed. Milestone 2 work is on `m2-scm`, not pushed yet.
- Build: `cargo build` is warning-free. `cargo test` passes 23 tests.
- Run: `cargo run --release`, or `target\release\halo.exe [folder-or-file]`.
- Config written by the app: `%APPDATA%\halo\settings.json` and `session.json`. eframe also stores window size and position under `%APPDATA%\Halo`.

## What works (checked by running the app)

Checked on Windows 11 by launching the build, sending keystrokes and capturing the window:

- Window shell, menus bar, activity bar, status bar with repo/branch, dark and light themes, themed tooltip.
- Explorer tree with git colours (modified folder in yellow with a dot).
- Editor: opening files, Rust highlighting, line numbers, git gutter, minimap, typing, Enter, undo, `Ctrl+End`, save with `Ctrl+S` (verified on disk, CRLF preserved), find bar with match count, split editor, `Ctrl+D` multi-cursor.
- Terminal: PowerShell starts in the folder, accepts typed commands, shows colours, runs `git`.

## Not verified by hand

Mouse interactions were not exercised (only keyboard input and screenshots). Covered by unit tests at most, or not at all:

- Explorer: context menu, new file/folder, rename, delete to recycle bin, keyboard navigation, reveal.
- Editor: mouse selection, double/triple click, Alt+click cursors, minimap click/drag, tab close button and the unsaved-changes dialog, replace/replace all, go to line, comment toggle, light theme colours in code.
- Terminal: split, kill, profile menu, Git Bash and WSL profiles, mouse selection and copy, wheel scrollback, Ports tab.
- Menus: every item dispatches a command, but only keyboard paths were run.
- Window: drag to move, edge resize, maximize/restore, minimize. Whether a maximized frameless window covers the taskbar is unknown.
- Unsaved-changes prompt on exit.
- Source Control (milestone 2): the side bar was checked by screenshot only. Stage, discard prompt and commit are covered by a headless egui test, push/pull/switch by a test against a local bare remote. Not run by hand: the repositories list, stash, clicking a commit, the branch menu, the Create branch and Discard dialogs, moving untracked files to the recycle bin, and push/pull against GitHub with real credentials. The pull request list was seen loading from GitHub (an empty list); pull request rows, their menu, review and create dialogs, the accounts menu and account switching have never run against real data. Diff tabs and the conflict bar are covered by headless tests and one screenshot.
- AI panel (milestone 3): the Claude tab with its Start session screen was checked by screenshot. A test runs a missing CLI through the real PTY and checks the ended session is kept. Never run: Start session with the real `claude` or `codex`, restart, stop, switching tabs with sessions running, copy and paste.

## Known gaps and limits

**Memory target is missed.** Release build, maximized on a high-DPI display: about 90 MB private working set with a folder open, about 100 MB with a file and a terminal (133 MB working set). The target is 80 MB. The largest single piece is the AMD OpenGL driver (`atio6axx.dll`, 62 MB image) loaded into the process. Ideas:

1. Tried 2026-11-20, rejected: the wgpu renderer with default settings. Private working set with a folder open, same machine and window: OpenGL (glow) 84 MB; wgpu on DX12 273-281 MB, Vulkan 283 MB, GL 160 MB. The binary also grows from 9.6 MB to 14.3 MB. wgpu's low-memory device hints were not tried. Halo stays on glow.
2. Measure at a smaller window size to see how much is framebuffer.
3. Check which GPU is selected on hybrid laptops; an integrated GPU driver may be smaller.
4. Trim the working set when idle (`SetProcessWorkingSetSize`), which improves the Task Manager number but not real use.

Placeholders that say which milestone they arrive in: title bar search box, Switch to WSL button, Run menu.

Other limits:

- No hover, completion or diagnostics (deferred by decision 18). Problems tab is an empty state.
- A `settings.json` that fails to parse falls back to defaults silently; show a banner instead.
- Editor: no word wrap, no regex find, no auto-closing brackets, no IME composition, no word-wise drag after double click; a tab renders as four spaces wide rather than aligning to tab stops; git gutter marks reflect the file as last saved; lines are drawn up to 5000 characters.
- Syntax: syntect's bundled grammars only. TypeScript/JSX fall back to JavaScript, `.ino`/headers to C++; TOML and some others show as plain text. Loading extra grammars from settings is M3.
- Terminal: no bold or italic font variants, no mouse reporting to terminal programs, no scrollbar, wide characters are placed per cell without width correction. Shortcuts bound globally (`Ctrl+B`, `Ctrl+J`, `Ctrl+W`, `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+,`) do not reach the shell.
- Source Control: only one repository is active at a time, so explorer colours follow the repository selected in the side bar. The selection is not saved in the session. Repository discovery skips dot folders, `node_modules` and `target`. The graph shows the first 8 lanes; wider histories draw on top of the last lane. `git pull` uses the user's git config, and after a pull that conflicts, resolve each file with the conflict bar, stage it and commit. Diffs are unified text only and binary files show git's one-line notice. Creating a pull request always pushes to `origin` and uses the repository's default base branch. `gh` commands inherit the side bar's busy state and errors. Editors with unsaved changes are not reloaded after a git operation. A git command that waits for input (an SSH passphrase, a GPG pinentry) keeps the side bar busy until it ends.
- AI panel: the commands are fixed (`claude`, `codex`) with no extra arguments. A session keeps the folder it was started in when another folder is opened. A CLI that is not installed shows the shell's "not recognized" message and the session-ended notice.
- Explorer: no file watcher (it refreshes when the window regains focus and after saves), no drag and drop, rename does not preselect the name stem.
- Key chords are not supported; Open Folder is `Ctrl+Shift+O`.
- The caret blink repaints at 20 fps for 30 seconds after the last input, then stops.
- Linux and macOS: code is portable in intent but has never been compiled there.

## Notes from this session

- Subagents with worktree isolation were tried and abandoned: the harness created worktrees of the parent `Documents` repository instead of Halo. Those stray worktrees and branches were removed; nothing else in `Documents` was touched.
- During testing the debug build was launched many times and closed by process id. Early on, one cleanup command stopped every `halo` process by name, which would also have closed a copy you were running.
- `%APPDATA%\halo\session.json` was rewritten once during testing to drop two bad entries; `settings.json` was switched to the light theme for a screenshot and restored.

## Suggested next steps

1. Run the app by hand through the "Not verified" list and fix what breaks.
2. Memory: the renderer stays on glow (see above); the remaining ideas are 2 to 4.
3. Milestone 2 is complete on `m2-scm`. Run it by hand against a real GitHub repository (see "Not verified").
4. Milestone 3 is on `m3-ai` (branched from `m2-scm`): the AI panel is built; next in [milestones.md](milestones.md) are Switch to WSL, title bar search, settings and key chords, and the Run menu.
