# Halo

An ultra-lightweight native code editor, laid out like VS Code. Written in Rust with egui on OpenGL: no Electron, no web view.

## Status

| Milestone | Scope | State |
|-----------|-------|-------|
| M1 | Window shell, explorer, editor, terminal, status bar, dark and light themes | done |
| M2 | Source control, commit graph, conflict editor, pull request panel, GitHub accounts | done |
| M3 | Claude and Codex CLI panel, WSL mode, search bar, settings page | planned |
| M4 | Windows installer and release pipeline | planned |

Windows 11 is the tested platform. The code is kept portable; Linux and macOS builds come later.

## Build and run

Needs a Rust toolchain (1.85 or newer) and `git` on `PATH`.

```bash
cargo run --release
```

## Configuration

Settings live in `settings.json` under the user config directory (`%APPDATA%\halo` on Windows). Open it with `Ctrl+,`; saving the file applies it.

| Key | Meaning |
|-----|---------|
| `theme` | `"dark"` or `"light"` |
| `font_size` | Editor and terminal font size |
| `tab_size`, `insert_spaces` | Indentation |
| `minimap` | Show the editor minimap |
| `terminals` | Terminal profiles: `name`, `program`, `args` |
| `default_terminal` | Name of the profile used for new terminals |

## Shortcuts

Halo follows VS Code defaults: `Ctrl+B` side bar, `Ctrl+J` panel, `Ctrl+Alt+B` AI panel, `Ctrl+\` split editor, `Ctrl+Shift+\`` new terminal, `Ctrl+F` find, `Ctrl+H` replace, `Ctrl+G` go to line, `Ctrl+D` add next occurrence, `Ctrl+/` toggle comment. Open Folder is `Ctrl+Shift+O`.

## License

MIT

## Documentation

See [docs/](docs): requirements, design decisions, architecture, milestones and handoff notes.
