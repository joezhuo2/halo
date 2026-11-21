# Design decisions

Confirmed by the owner on 2026-11-20 through 24 multiple-choice questions. Where the owner picked something other than the recommendation, the choice is marked **(owner's pick)**.

## Foundations

1. Stack: fully native Rust **(owner's pick)**, egui/eframe with the OpenGL (glow) renderer.
2. Memory: under 80 MB idle **(owner's pick)**; panels, terminals and AI sessions load lazily. See the memory note in [handoff.md](handoff.md): the target is not met yet.
3. Platform: Windows, Linux and macOS were chosen, then narrowed to "Windows now, others later" **(owner's pick)**. Code stays portable.
4. Location: `Documents/halo`, pushed to https://github.com/joezhuo2/halo. MIT license.
5. Packaging: portable `halo.exe`, a Windows installer **(owner's pick)**, and CI releases on version tags.
6. Delivery: four milestones, small commits pushed after each.

## Layout

7. Title bar: custom and frameless. Menus left, search centered, window buttons right.
8. Activity bar: Explorer and Source Control at the top; Settings, Accounts and Switch to WSL at the bottom.
9. Left bar: Explorer or Source Control, laid out like VS Code.
10. Right bar: Claude and Codex tabs, hidden by default, with Start session.
11. Bottom panel: Problems, Output, Debug Console, Terminal, Ports.
12. Status bar: problems/warnings, repo/branch and a sync button on the left; AI toggle on the right.
13. Themes: a dark gray theme and a light gray theme **(owner's pick)**, both with an ice blue accent.
14. Startup: restore the last folder, tabs and layout.

## Editor

15. Editing: tabs, undo/redo, find/replace, multi-cursor, bracket matching, auto-indent, comment toggle.
16. Extras: split editor groups and a minimap **(owner's pick)**.
17. Highlighting: syntect with its bundled grammars; more grammars added through settings later.
18. Hover, completion and diagnostics: not in v1 **(owner's pick: "highlighting only in v1")**. Language servers come in a later milestone.
19. Git colours: green new, yellow modified, red deleted, in the explorer and the editor gutter.

## Source control

20. Backend: the installed `git` CLI.
21. Accounts: reuse and switch `gh` CLI logins.
22. Repositories: the root plus nested repositories two levels deep.
23. Actions: commit, push, pull, fetch, stash, switch branch, refresh.
24. Graph: VS Code style lanes for the current branch and its remote.
25. Conflicts: inline accept current, incoming or both.
26. Pull requests: a full panel to list, check out, create and review **(owner's pick)**, through `gh`.

## Terminal and AI

27. Terminal: tabs with add, kill and split.
28. Profiles: PowerShell, Command Prompt, Git Bash, WSL, bash; custom ones in settings.
29. AI panel: the real `claude` and `codex` CLIs in an embedded terminal, one session per CLI.
30. Switch to WSL: reopen the folder in a distro; files, git, terminal and AI CLIs run there.

## Menus, search, settings

31. Menus: File through Help, covering the commands Halo implements rather than all of VS Code's.
32. Search bar: files by default, `>` commands, `%` text, `@` symbols, `:` line.
33. Run: runs a per-language command or task in the terminal; no breakpoints.
34. Ports: lists local listening ports.
35. Settings: `settings.json` and `keybindings.json` with VS Code default hotkeys, plus a simple settings page.
36. Extensions: no VS Code extension support.

## Decisions made while building milestone 1

- egui is pinned to 0.36 because the icon crate (`egui-phosphor` 0.14) targets it. Move both together.
- The terminal is hand-written on top of `alacritty_terminal`; the `egui_term` crate targets an old egui.
- Open Folder is `Ctrl+Shift+O` for now. VS Code's `Ctrl+K Ctrl+O` chord needs chord support, planned with `keybindings.json` in milestone 3.
- Global shortcuts are taken by Halo before the terminal sees them (for example `Ctrl+B`, `Ctrl+J`, `Ctrl+W`), mirroring VS Code's "commands to skip shell".
- Deleting from the explorer moves items to the recycle bin and never falls back to permanent deletion.
- `halo <path>` on the command line opens that folder or file.

## Decisions made while adding language servers (0.1.1)

- Decision 18 is lifted: language servers were asked for by the owner on 2026-11-20, as items 1 to 7 of the list of VS Code features Halo lacked.
- Servers are found on `PATH` and never bundled or downloaded. A server that is missing is skipped silently and noted in the Output panel, like the other optional tools.
- The client is hand-written on `serde_json`, with no LSP or async crate.
- Server commands are a map from file extension to command line in `settings.json` (`servers`), edited like run commands. Arguments are split on spaces.
- Documents are sent whole on each change. Incremental sync was not needed for files under 2 MB.
- The reference count line is drawn by Halo from document symbols and reference requests, so it works with any server that supports references.
- The peek list, hover, completion and other popups are overlays. The peek list does not push the text down as VS Code's does.
- Completion declares no snippet support, so servers send plain text; snippet syntax that arrives anyway is flattened.
- Format on save applies to Save, not to auto save, and saves unformatted after 1.5 s without an answer.
- Language servers are not started for WSL folders.

## Decisions made while building milestone 3

- WSL mode has no state of its own: a folder under `\\wsl.localhost\<distro>` (or `\\wsl$`) is a WSL folder. Files go through that share with ordinary file calls; only processes (git, `gh`, shells, AI CLIs, Run) are wrapped in `wsl.exe -d <distro> --cd <dir>`.
- Search uses `git ls-files` for the file list and `git grep --no-index --exclude-standard` for text, so both respect `.gitignore` and work outside a repository. Symbols come from syntect scopes (`entity.name`), not a language server.
- `keybindings.json` replaces keys per command instead of VS Code's add/remove (`-command`) rules. Open Folder moved to `Ctrl+K Ctrl+O`, which frees `Ctrl+Shift+O` for Go to Symbol. `Ctrl+K` starts a chord everywhere, including in the terminal.
- The settings page is a dialog over the window, not an editor tab. Lists and maps (terminal profiles, run commands, tasks, associations) are edited in `settings.json`.
- Run types the command into a shell terminal that is reused for later runs, so the shell stays open after the program ends. The Debug Console shows the last 500 lines of that terminal as text.
- The frost blue ring stays as the in-app logo for now. The taskbar shows egui's default icon; the owner will supply an icon (asked 2026-11-20).
