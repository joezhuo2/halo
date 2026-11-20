# Architecture

Halo is a single Rust binary: eframe/egui 0.36 on the OpenGL (glow) renderer. No web view, no async runtime. Background work runs on plain threads that report back over `std::sync::mpsc` and call `ctx.request_repaint()`.

## Modules

| File | Role |
|------|------|
| `src/main.rs` | Window options (frameless, sizes) and `eframe::run_native`. |
| `src/app.rs` | `Halo`: owns every component, lays out the panels each frame, dispatches commands, session save, exit confirmation, window edge resizing. |
| `src/titlebar.rs` | Title bar: menus, centered search box (placeholder until M3), window buttons, drag to move. |
| `src/statusbar.rs` | Status bar items. |
| `src/command.rs` | `Command` enum, default key bindings with a `Scope` (Global or Editor), menu labels. |
| `src/settings.rs` | `Settings` (`settings.json`) and `Session` (`session.json`) under the user config dir; terminal profile defaults. |
| `src/theme.rs` | `Theme` colours for dark and light, and `apply`, which also styles egui widgets, popups and tooltips. |
| `src/ui.rs` | Shared helpers: `anim`, `mix`, `row_bg`, `icon_button`, `fade_in`, `with_alpha`, `section_header`, and the `icons` re-export (Phosphor). |
| `src/git.rs` | Git layer over the `git` and `gh` CLIs: status, change lists, commit log with lanes, nested repositories, pull requests, and the background operations (`Git::act`). |
| `src/scm.rs` | Source Control side bar: repositories, branch toolbar, commit box, changes, pull requests, commit graph, and its dialogs. |
| `src/accounts.rs` | Accounts menu on the activity bar: lists and switches `gh auth` accounts. |
| `src/explorer.rs` | Explorer side bar. |
| `src/editor/mod.rs` | `Editors`: groups (splits), tabs, the editor view, find bar, go to line, minimap, close dialog. |
| `src/editor/buffer.rs` | `Document`: rope, selections, edit engine, undo/redo, motions, find, load/save. |
| `src/editor/highlight.rs` | syntect highlighting with checkpoints and a per-frame time budget. |
| `src/terminal.rs` | `Terminal` (reusable PTY terminal widget) and `BottomPanel` (panel tabs, terminal groups, Ports). |

## Frame flow

`Halo::ui` runs once per repaint:

1. `git.poll()` drains background results; window events (close request, focus) are handled.
2. Global shortcuts are consumed from input and queued as `Command`s.
3. Panels are drawn in this order: title bar, status bar, activity bar, side bar, AI panel, bottom panel, central editor area. Components push `Command`s into `Halo::cmds`.
4. Queued commands are dispatched at the end of the frame (`Halo::dispatch`). Commands that are not app-level go to the editor or the terminal depending on `zone` (which component last had keyboard focus).

## Commands and shortcuts

- Everything a menu or shortcut can do is a `Command`. Menus, the welcome page and shortcuts share labels and bindings from `command.rs`.
- `Scope::Global` bindings are consumed by the app before any widget sees the key. `Scope::Editor` bindings are consumed by the editor view only while it has focus, so the terminal still receives `Ctrl+Z`, `Ctrl+D`, `Ctrl+A` and friends.
- `bindings()` is ordered most-specific first because `consume_shortcut` matches modifiers loosely.
- Cut, copy and paste arrive from egui as `Event::Cut/Copy/Paste`, not as key events. Menu paste sends `ViewportCommand::RequestPaste` after refocusing the target.

## Component contracts

- `Editors`: `ui`, `open(path)`, `exec(&Command, &Settings, &Context) -> bool`, `is_empty`, `open_paths`, `active_path`, `status`, `has_focus`, `any_dirty`, `discard_all`, `path_changed(old, new)`.
- `BottomPanel`: `ui`, `exec(...) -> bool`, `has_focus`, `blur`. `Terminal::spawn(id, name, program, args, cwd, ctx)` and `Terminal::ui(ui, theme, font_size)` are meant to be reused by the M3 AI panel.
- `Explorer`: `set_root`, `refresh`, `has_focus`, `ui(ui, theme, git, active_path, cmds)`. File operations are reported through `Command::OpenPath`, `RefreshExplorer` and `PathChanged(old, Option<new>)`.
- `Git`: `set_root`, `refresh(ctx)`, `poll`, `branch`, `repo_name`, `state(path)`, `line_changes(path)`. `git::run(dir, args)` is the one place that shells out (hidden console window on Windows).

## Editor internals

- A document can be shown in several groups. Cursors and scroll live in the `Tab`, text and undo history in the `Document`.
- All edits go through `Document::apply(sels, edits, kind, after)`: edits are sorted and merged, applied in one pass, recorded as one undo transaction, and selections are either placed after each edit (`After::Carets`) or mapped through the edits (`After::Map`). Consecutive typing or deleting merges into one transaction.
- `version` identifies content: undo restores the earlier version id, so "dirty" is simply `version != saved`.
- Only visible lines are laid out (one galley per line, cached by egui) and painted. Caret positions and hit testing use the galley, so tabs and wide characters are right.
- Highlighting keeps a parse checkpoint every 64 lines, caches spans around the viewport, is invalidated from the edited line down, and stops after about 8 ms per frame; unreached lines render plain until a later frame.

## Conventions

- No code comments or docstrings (owner's rule). Explain in commit messages and in `docs/`.
- Every colour comes from `Theme`; never hard-code one. Both themes must stay usable.
- Every interactive element animates through `ui::anim` and friends, and every icon-only button has `.on_hover_text(...)`.
- Icons: `ui::icons::NAME` (Phosphor regular).
- Lazy by default: nothing spawns a process or thread until its panel is first shown.
- Errors from the file system, git or process spawning are shown to the user (banners), never dropped and never panics.

## egui 0.36 notes

The API differs a lot from older egui. Check signatures in `~/.cargo/registry/src/index.crates.io-*/egui-0.36.2/src` before using something new.

- App trait: `fn ui(&mut self, ui: &mut egui::Ui, frame)`; `fn save` is called on exit and periodically.
- Panels: `egui::Panel::left/right/top/bottom(id).exact_size(..).default_size(..).size_range(..).resizable(..).frame(Frame::new().fill(c)).show(ui, ..)`; `.show_collapsible(ui, &mut bool, ..)` animates open/close and returns `None` when collapsed.
- `egui::MenuBar::new().ui(ui, ..)`, `ui.menu_button`, `ui.close()` closes a menu, `response.context_menu(..)`.
- `egui::Modal::new(id).show(ctx, ..)` returns a response with `should_close()`.
- `CornerRadius::same(u8)`, `painter.rect(rect, radius, fill, stroke, StrokeKind::Inside)`.
- `ctx.animate_bool_with_time` returns the target value on its first call for an id, so it cannot fade something in on first appearance; `ui::fade_in` uses a start time instead.
- Fonts: `ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.row_height(&font)))`.
- Text: `LayoutJob::simple_singleline`, `painter.layout_job(job)`, `galley.pos_from_cursor(CCursor::new(col))`, `galley.cursor_from_pos(vec2)`.
- Headless tests: `ctx.run_ui(RawInput { .. }, |ui| ..)`; call `textures_delta.clear()` on the output or it panics on drop in debug builds.

## Testing

`cargo test` runs 14 tests: the edit engine, highlighting, git parsing against a real temporary repository, explorer logic on a temp folder, key encoding and colour mapping for the terminal, a real shell round trip through the PTY, and a headless editor test that types through egui input.

Rendering is checked by eye. During milestone 1 this was done by launching the debug build, sending keystrokes and capturing the window.
