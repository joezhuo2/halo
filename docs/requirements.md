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

## Interface rework before 0.1.0 (2026-11-20)

- More comfortable spacing and hover highlights for the title bar menus; consistent padding for buttons, text boxes, menus and dialogs across the app; thinner scroll bars.
- The focused terminal is highlighted, not the last one clicked.
- Tabs can be dragged to reorder or to create side by side views, and can be pinned.
- A theme switch at the bottom right with a cheap animation; the AI button moves beside Split Editor.
- Terminal cells with a background must not black out their glyphs (the Claude logo).
- Settings follows the VS Code layout with tabs on the left: font and font size, auto save, a key binding editor for `keybindings.json`, every `settings.json` option by category, installed languages, and color options.
- The icon is a rounded hexagon with a four-petal star, centered; the star is black when the system is in dark mode.
- Action buttons must not overlap text, and row text needs padding before it is cut off.
- The README follows the style of https://github.com/joezhuo2/tickr, with screenshots and a GIF tour.

## Standing rules from the owner's global instructions

- Small commits, no secrets.
- No explanatory code comments or docstrings unless asked; reasoning goes in chat or in these docs.
