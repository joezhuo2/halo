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
- Language servers (hover, completion, go to definition, Problems tab) were deferred out of v1 by the owner and are not built; schedule them after M4.

## M4: packaging

- Windows installer with Start menu entry and "Open with Halo".
- GitHub Actions: build and attach binaries to releases on version tags.
- Linux and macOS builds once Windows is finished.
