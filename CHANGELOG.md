# Changelog

All notable changes to Halo are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Halo uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-11-20

### Added

- **Timeline** section under the outline: the commits that changed the active
  file, newest first, with a refresh button. Click a commit to open its diff
  of the file.
- The source control sections (changes, pull requests and commits) can be
  minimized and resized by dragging the line between them.
- Keyboard Shortcuts in the settings are grouped by category.
- Links in hover descriptions open when clicked.
- Loading placeholders in the outline, the timeline, source control, search
  results, the reference counts above declarations, and the accounts and WSL
  lists. The outline shows a spinner while its symbols load.
- Animations for collapsing folders in the explorer, for the explorer root,
  and for opening and closing the side bar sections. Settings checkboxes are
  animated.

### Changed

- The outline opens and closes without an active file and says why it is
  empty.
- Notices such as "No definitions found." go away after a few seconds. File
  errors stay until they are dismissed.
- Saving a file, or pressing the save key on an unchanged one, keeps its
  preview tab.
- The problems item in the status bar, and `Ctrl+Shift+M`, close the Problems
  panel when it is already showing.

## [0.1.2] - 2026-11-20

### Added

- Word wrap, code folding by indentation, sticky scroll, rulers, visible
  whitespace, bracket pair colors and indentation guides.
- Brackets and quotes close as they are typed and wrap a selection.
- Find and replace: regular expressions, find in selection and preserve case.
- Search in files in the side bar: replace in files, include and exclude
  globs, a results tree and the results as a tab.
- Line operations: move, copy, delete, join, and insert a line above or below.
  Block comment toggle.
- Box selection, select all occurrences, expand and shrink selection, and
  cursor undo.
- Snippets with tab stops from `snippets.json` and from language servers,
  word completion in files without a server, and Emmet abbreviations.
- Status bar pickers for the language, indentation, encoding and line ending.
- Trim trailing whitespace and insert a final newline on save.
- Hot exit: unsaved and untitled files are restored after a restart.
- A bar to compare, reload or overwrite when a changed file changes on disk.
- Preview tabs, a recent-file switcher on `Ctrl+Tab`, reopen closed editor,
  and Go Forward.
- Links in the text open with `Ctrl+Click`.
- Auto update: Halo installs the latest release in the background when it
  opens and uses it the next time.

### Changed

- `Ctrl+Shift+F` opens the search view in the side bar. The `%` search in the
  command bar is still there.
- A file opened from the explorer or the search bar is a preview until it is
  edited or its tab is double-clicked.
- `Ctrl+Tab` switches between recently used files instead of the next tab.
- Closing Halo with unsaved files no longer asks to save them while hot exit
  is on.

## [0.1.1] - 2026-11-20

### Added

- Language server support. Halo starts the server set for a file's extension
  when it is on `PATH`, with defaults for Rust, Python, TypeScript and
  JavaScript, Go, C and C++, C#, Java, Lua, Zig, JSON, HTML and CSS.
- Diagnostics: underlines in the text, marks in the minimap, the Problems
  panel and error and warning counts in the status bar.
- Completion, signature help and hover.
- Go to definition, declaration, type definition and implementations, peek
  definition, find all references and Go Back, with a peek list that previews
  each place.
- Reference counts on a line above functions, methods and types, which open
  the peek list when clicked, and implementation counts on interfaces.
- Rename symbol, quick fixes and refactorings, and organize imports.
- Format document and selection, and optional format on save and on paste.
- Workspace symbol search with `#` in the search bar (`Ctrl+T`), an Outline
  section under the explorer, and breadcrumbs above the text.
- Semantic highlighting, inlay hints, and call and type hierarchy.
- The Output panel shows language server messages, and the status bar shows
  server activity.
- A context menu in the editor.
- Settings for each language feature and for the server commands.
- SECURITY.md, CODE_OF_CONDUCT.md, CONTRIBUTING.md, and issue and pull request
  templates.

### Fixed

- Opening a file whose path differs only in letter case or slashes from an
  open one no longer opens it twice on Windows.

## [0.1.0] - 2026-11-20

First release.

### Added

- Window shell laid out like VS Code: title bar menus and search, activity
  bar, side bar, editor area, bottom panel and status bar, in a dark and a
  light theme.
- Explorer with git state colors and file operations.
- Editor with multi-cursor editing, find and replace, split views, a minimap,
  syntax highlighting for 68 languages, and a merge conflict bar.
- Tabs that can be dragged to reorder, moved between groups, dropped to split
  the editor, and pinned.
- Integrated terminal with profiles, split panes, a ports list and a debug
  console for Run File and tasks.
- Source control: stage, discard, commit, branches, push, pull, fetch, stash,
  a commit graph, pull requests through `gh`, and GitHub account switching.
- AI panel that runs the Claude and Codex CLIs in the open folder.
- Search bar for files, commands, text in files, symbols and lines.
- WSL mode that reopens a folder inside a WSL distro.
- Settings page with categories for the editor, files, colors, terminal
  profiles, run commands, languages and keyboard shortcuts, including an
  editor font, auto save and custom colors for each theme.
- Windows installer with a Start menu entry and Open with Halo, a portable
  Windows executable, and Linux and macOS builds.

[Unreleased]: https://github.com/joezhuo2/halo/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/joezhuo2/halo/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/joezhuo2/halo/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/joezhuo2/halo/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/joezhuo2/halo/releases/tag/v0.1.0
