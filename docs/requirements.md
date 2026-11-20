# Requirements

Everything the owner asked for, in their words where possible. Later instructions are listed after the original brief and override it where they differ.

## Original brief

Create Halo: an ultra-lightweight clone of VS Code.

- Use as little memory as possible.
- Native support for the Codex CLI and the Claude CLI: replace the VS Code agents window with a Claude/Codex CLI tab.
- Theme: dark gray, light gray, ice blue accent.
- Native GitHub support: green shows a new file, yellow a modified one, red a deleted one.
- Leftmost side bar (activity bar). Top aligned: Explorer, Source Control. Bottom aligned: Settings (hotkeys), (GitHub) Accounts, Switch to WSL.
- Left bar: Explorer and Source Control, laid out exactly like VS Code. Source Control has the graph, changes and repositories; commit, push, stash, change branch, fetch, pull and refresh buttons must work; pull request conflicts must be handled.
- Right bar (new): a tab at the top switches between the Claude and Codex CLIs. Hidden by default, with a Start session button that launches Claude or Codex.
- Bottom: terminal panel as in VS Code with Problems, Output, Debug Console, Terminal and Ports. Multiple tabs, with working add, kill and split buttons, and a way to pick PowerShell, bash, Command Prompt, WSL, Git Bash or other terminal types (more can be added in settings).
- Native language support: syntax highlighting, hover showing what a class does, and so on. Languages are configurable in settings.
- Bottom status bar. Left aligned: problems/warnings, current GitHub repo/branch, a quick pull + push refresh button. Right aligned: open/hide the AI tab.
- Top bar. Left aligned: File, Edit, Selection, View, Go, Run, Terminal, Help with the same functionality as VS Code. Centered: a global search bar (search for anything). Right aligned: minimize, fullscreen, close.
- Lives in `Documents/halo`; GitHub repository https://github.com/joezhuo2/halo.

## Later instructions

- Ask as many design questions as possible and confirm every decision before building. The answers are in [decisions.md](decisions.md).
- All hover tooltips must be themed.
- Everything should have some simple animations.
- No worktrees and no subagents: build in the main checkout.
- Put all instructions in `docs/` as Markdown files.
- Stop once milestone 1 is finished and leave handoff notes; work continues in a new session. See [handoff.md](handoff.md).

## Standing rules from the owner's global instructions

- Small commits, no secrets.
- No explanatory code comments or docstrings unless asked; reasoning goes in chat or in these docs.
