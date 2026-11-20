# Milestones

## M1: shell, explorer, editor, terminal (done)

- Frameless window with custom title bar, menus, window buttons, edge resizing.
- Activity bar, collapsible side bar, right AI panel placeholder, bottom panel, status bar.
- Dark and light themes with ice blue accent; themed tooltips; hover and slide animations.
- Explorer: lazy tree, git colours and badges, keyboard navigation, new file/folder, rename, delete to recycle bin, copy path, reveal.
- Editor: tabs, split groups, multi-cursor, undo/redo, find/replace, go to line, comment toggle, auto-indent, bracket matching, syntax highlighting, git gutter, minimap.
- Terminal: PowerShell, Command Prompt, Git Bash, WSL profiles; new, split, kill; selection, copy/paste, scrollback; Ports tab.
- Session restore (folder, open files, panel visibility), `settings.json`.

## M2: source control

- Done: Source Control side bar with staged and unstaged changes, stage/unstage and discard (one file or all), commit box, click a file to open it.
- Done: push (publishes to `origin` when there is no upstream), pull, fetch, refresh, switch/create branch; status bar sync button does pull + push; unmodified open editors reload after each git operation.
- Done: repositories list (the open folder's repository plus nested ones two levels deep; the selected one drives the side bar, status bar and explorer colours), stash and pop stash.
- Done: commit graph of the last 200 commits with lanes for the branch and its upstream; click a commit to list its files.
- Remaining here: per-file diffs for a commit, which arrive with the diff view.
- Diff view and inline conflict editor (accept current, incoming, both).
- Pull request panel through `gh`: list, check out, create, review, mergeable/conflict state.
- Accounts button: list and switch `gh auth` accounts.

## M3: AI panel, WSL, search, settings

- Right panel: Claude and Codex tabs, each with Start session, restart and stop, reusing `terminal::Terminal`.
- Switch to WSL: pick a distro and folder; files, git, terminal, AI CLIs run through `wsl.exe`.
- Title bar search: files by default, `>` commands, `%` text in files, `@` symbols, `:` line.
- Settings page, `keybindings.json`, key chords, extra syntect grammars from a folder.
- Run menu: per-language run command or task in the terminal; output mirrored in Debug Console.
- Language servers (hover, completion, go to definition, Problems tab) were deferred out of v1 by the owner; schedule them here or after M4.

## M4: packaging

- Windows installer with Start menu entry and "Open with Halo".
- GitHub Actions: build and attach binaries to releases on version tags.
- Linux and macOS builds once Windows is finished.
