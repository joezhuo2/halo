# Feature gaps

Two lists: what VS Code has that Halo does not, and what VS Code users keep
asking for that VS Code itself does not have. Checked against the README,
`docs/milestones.md` and a grep of `src/` on 2026-11-24. Items marked
*(verify)* had no clear hit in the source but may exist under another name.

## In VS Code, missing from Halo

- **Live Share** or any shared editing session.
- **Remote SSH**, **Dev Containers** and **Tunnels**.
- **Notebooks** (`.ipynb`) and the Python **Interactive Window**.
- **Extensions** (out of scope by design; README Limits).
- **Screencast mode**: show pressed keys and commands on screen for demos and
  recordings.
  
- **Language detection** for untitled files: guess the language from the
  content and offer to switch *(verify)*.

- **Continue On / Edit Sessions**: carry uncommitted changes to another
  machine through the same gist used by Settings Sync.

## Wanted in VS Code, missing from both

- **Built-in spell checker** for comments, strings and Markdown.
- **True terminal persistence** on Windows (tmux-like): close the window,
  reopen, and the running process is still there.
- **Remote editing without a server on the host**: SSH or SFTP editing for
  small machines where VS Code Server will not install.
- **Structural search and replace** and tree-sitter based selection and
  motion (JetBrains SSR, Helix, Zed).

- **Rewrap / reflow**: hard-wrap a comment or Markdown paragraph to the ruler,
  keeping comment prefixes (`gq` in Vim, the Rewrap extension).
- **Elastic tabstops**: align tab-separated columns across lines in
  proportional fonts.
- **Persistent word highlights**: mark a word in one of several colours and
  keep it highlighted across the file and its minimap (Notepad++ mark styles).
- **Hide secrets on screen**: mask values in `.env` and similar files until
  the cursor is on the line, for streaming and screen sharing.
- **Built-in HTTP client**: run requests from `.http` files and show the
  response beside them (JetBrains HTTP Client, REST Client extension).
- **Database browser**: connect, browse tables and run queries
  (JetBrains DataGrip).
- **File watchers**: run a task when a matching file is saved, with its
  output parsed by the task's problem matcher (JetBrains File Watchers).
- **Editable menus and toolbars**: add, remove and reorder commands in the
  title bar, context menus and editor toolbar from Settings.
- **Detachable panels**: move the terminal, Explorer or any side-bar view into
  its own window, not only editors.
