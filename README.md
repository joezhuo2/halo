# Halo

An ultra-lightweight native code editor, laid out like VS Code. Written in Rust with egui on OpenGL: no Electron, no web view.

## Status

| Milestone | Scope | State |
|-----------|-------|-------|
| M1 | Window shell, explorer, editor, terminal, status bar, dark and light themes | done |
| M2 | Source control, commit graph, conflict editor, pull request panel, GitHub accounts | done |
| M3 | Claude and Codex CLI panel, WSL mode, search bar, settings page, key bindings, Run | done |
| M4 | Windows installer and release pipeline | planned |

Windows 11 is the tested platform. The code is kept portable; Linux and macOS builds come later.

## Build and run

Needs a Rust toolchain (1.85 or newer) and `git` on `PATH`.

```bash
cargo run --release
```

## Configuration

Settings live in `settings.json` under the user config directory (`%APPDATA%\halo` on Windows). `Ctrl+,` opens the settings page, which has a button for the file; saving the file applies it.

| Key | Meaning |
|-----|---------|
| `theme` | `"dark"` or `"light"` |
| `font_size` | Editor and terminal font size |
| `tab_size`, `insert_spaces` | Indentation |
| `minimap` | Show the editor minimap |
| `terminals` | Terminal profiles: `name`, `program`, `args` |
| `default_terminal` | Name of the profile used for new terminals |
| `run` | Run File commands by file extension; `{file}` is the file's path |
| `tasks` | Entries for the Run menu: `name`, `command` |
| `grammars` | Folder of extra `.sublime-syntax` grammars, loaded at startup |
| `associations` | File extension to grammar (extension or name), loaded at startup |

Key bindings live in `keybindings.json` next to it (`Ctrl+K Ctrl+S` creates and opens it). Each entry is `{"key": "ctrl+k ctrl+o", "command": "openFolder"}`. An entry replaces the default keys of its command; an empty `key` removes them. Saving the file applies it.

## Shortcuts

Halo follows VS Code defaults: `Ctrl+B` side bar, `Ctrl+J` panel, `Ctrl+Alt+B` AI panel, `Ctrl+\` split editor, `Ctrl+Shift+\`` new terminal, `Ctrl+F` find, `Ctrl+H` replace, `Ctrl+G` go to line, `Ctrl+D` add next occurrence, `Ctrl+/` toggle comment, `Ctrl+P` go to file, `Ctrl+Shift+P` commands, `Ctrl+Shift+F` find in files, `Ctrl+Shift+O` go to symbol, `Ctrl+K Ctrl+O` open folder, `F5` run file.

## License

MIT

## Documentation

See [docs/](docs): requirements, design decisions, architecture, milestones and handoff notes.
