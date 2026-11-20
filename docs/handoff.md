# Handoff: end of milestone 1

Written 2026-11-20. Read [requirements.md](requirements.md), [decisions.md](decisions.md) and [architecture.md](architecture.md) first; [milestones.md](milestones.md) has the scope of what comes next.

## State

- Repository: `C:\Users\Joe\Documents\halo`, remote `origin` = https://github.com/joezhuo2/halo.
- Branch: `m1-shell` holds all of milestone 1. There is no `main` yet: the GitHub repository was empty, so `m1-shell` is the first branch on it. Decide whether to rename it to `main` or create `main` from it before opening pull requests for M2.
- Build: `cargo build` is warning-free. `cargo test` passes 14 tests.
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

## Known gaps and limits

**Memory target is missed.** Release build, maximized on a high-DPI display: about 90 MB private working set with a folder open, about 100 MB with a file and a terminal (133 MB working set). The target is 80 MB. The largest single piece is the AMD OpenGL driver (`atio6axx.dll`, 62 MB image) loaded into the process. Ideas, none tried yet:

1. Try the wgpu renderer on DX12 and compare.
2. Measure at a smaller window size to see how much is framebuffer.
3. Check which GPU is selected on hybrid laptops; an integrated GPU driver may be smaller.
4. Trim the working set when idle (`SetProcessWorkingSetSize`), which improves the Task Manager number but not real use.

Placeholders that say which milestone they arrive in: title bar search box, Source Control side bar, AI panel, Accounts button, Switch to WSL button, Run menu.

Other limits:

- No hover, completion or diagnostics (deferred by decision 18). Problems tab is an empty state.
- A `settings.json` that fails to parse falls back to defaults silently; show a banner instead.
- Editor: no word wrap, no regex find, no auto-closing brackets, no IME composition, no word-wise drag after double click; a tab renders as four spaces wide rather than aligning to tab stops; git gutter marks reflect the file as last saved; lines are drawn up to 5000 characters.
- Syntax: syntect's bundled grammars only. TypeScript/JSX fall back to JavaScript, `.ino`/headers to C++; TOML and some others show as plain text. Loading extra grammars from settings is M3.
- Terminal: no bold or italic font variants, no mouse reporting to terminal programs, no scrollbar, wide characters are placed per cell without width correction. Shortcuts bound globally (`Ctrl+B`, `Ctrl+J`, `Ctrl+W`, `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+,`) do not reach the shell.
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
2. Decide the memory plan (see above) before adding more features, since the renderer choice affects everything.
3. Start M2 in `src/git.rs`: add staged/unstaged lists, then a `src/scm.rs` side bar; keep every git call inside `git::run` on a background thread.
