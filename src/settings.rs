use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShellProfile {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub name: String,
    pub command: String,
}

pub const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub icon_theme: String,
    pub font_family: String,
    pub font_size: f32,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub minimap: bool,
    pub auto_save: String,
    pub auto_save_delay: u64,
    pub colors: BTreeMap<String, BTreeMap<String, String>>,
    pub default_terminal: String,
    pub terminal_font_family: String,
    pub terminal_font_size: f32,
    pub terminal_line_height: f32,
    pub terminal_cursor_style: String,
    pub terminal_cursor_blinking: bool,
    pub terminal_scrollback: usize,
    pub terminal_copy_on_selection: bool,
    pub terminal_right_click: String,
    pub send_keybindings_to_shell: bool,
    pub terminal_persistent_sessions: bool,
    pub terminal_persistent_scrollback: usize,
    pub terminals: Vec<ShellProfile>,
    pub shell_integration: bool,
    pub grammars: String,
    pub associations: BTreeMap<String, String>,
    pub run: BTreeMap<String, String>,
    pub tasks: Vec<Task>,
    pub lsp: bool,
    pub servers: BTreeMap<String, String>,
    pub debug_adapters: BTreeMap<String, String>,
    pub diagnostics: bool,
    pub status_problems: bool,
    pub font_ligatures: bool,
    pub letter_spacing: f32,
    pub cursor_smooth_caret_animation: String,
    pub copy_with_syntax_highlighting: bool,
    pub code_lens: bool,
    pub lightbulb: bool,
    pub inlay_hints: bool,
    pub color_decorators: bool,
    pub unicode_invisible: bool,
    pub unicode_ambiguous: bool,
    pub semantic_highlighting: bool,
    pub suggest_rows: usize,
    pub breadcrumbs: bool,
    pub inline_blame: bool,
    pub local_history: bool,
    pub format_on_save: bool,
    pub format_on_paste: bool,
    pub format_on_type: bool,
    pub linked_editing: bool,
    pub word_wrap: String,
    pub word_wrap_column: usize,
    pub line_numbers: String,
    pub cursor_style: String,
    pub cursor_blinking: String,
    pub line_height: f32,
    pub render_line_highlight: String,
    pub scroll_beyond_last_line: bool,
    pub smooth_scrolling: bool,
    pub cursor_surrounding_lines: usize,
    pub peek_context_lines: usize,
    pub mouse_wheel_zoom: bool,
    pub multi_cursor_modifier: String,
    pub column_selection: bool,
    pub word_separators: String,
    pub match_brackets: String,
    pub selection_highlight: bool,
    pub minimap_side: String,
    pub detect_indentation: bool,
    pub auto_close: bool,
    pub auto_close_quotes: bool,
    pub auto_surround: bool,
    pub auto_indent: String,
    pub trim_final_newlines: bool,
    pub eol: String,
    pub trim_trailing_whitespace: bool,
    pub insert_final_newline: bool,
    pub render_whitespace: String,
    pub rulers: Vec<usize>,
    pub sticky_scroll: bool,
    pub bracket_colors: bool,
    pub guides: bool,
    pub diff_side_by_side: bool,
    pub hot_exit: bool,
    pub tab_close_button: String,
    pub enable_preview: bool,
    pub wrap_tabs: bool,
    pub confirm_delete: bool,
    pub explorer_sort: String,
    pub zoom_level: f32,
    pub restore_session: bool,
    pub large_mb: u64,
    pub auto_update: bool,
    pub files_exclude: Vec<String>,
    pub compact_folders: bool,
    pub confirm_drag_and_drop: bool,
    pub file_nesting: bool,
    pub file_nesting_expand: bool,
    pub file_nesting_patterns: BTreeMap<String, String>,
    pub auto_detect_high_contrast: bool,
    pub preferred_high_contrast_theme: String,
    pub preferred_high_contrast_light_theme: String,
    pub reduce_motion: String,
    pub accessibility_support: String,
    pub sidebar_location: String,
    pub panel_location: String,
    pub panel_alignment: String,
    pub activity_bar_location: String,
    pub zen_full_screen: bool,
    pub zen_center_layout: bool,
    pub zen_show_tabs: String,
    pub zen_hide_line_numbers: bool,
    pub zen_restore: bool,
    #[serde(skip)]
    pub langs: BTreeMap<String, Settings>,
    #[serde(skip)]
    pub folders: BTreeMap<PathBuf, Settings>,
}

impl Default for Settings {
    fn default() -> Self {
        let terminals = default_terminals();
        Self {
            theme: "dark".into(),
            icon_theme: "builtin".into(),
            font_family: String::new(),
            font_size: 13.0,
            tab_size: 4,
            insert_spaces: true,
            minimap: true,
            auto_save: "off".into(),
            auto_save_delay: 1000,
            colors: BTreeMap::new(),
            default_terminal: terminals.first().map(|p| p.name.clone()).unwrap_or_default(),
            terminal_font_family: String::new(),
            terminal_font_size: 0.0,
            terminal_line_height: 1.15,
            terminal_cursor_style: "block".into(),
            terminal_cursor_blinking: true,
            terminal_scrollback: 5000,
            terminal_copy_on_selection: false,
            terminal_right_click: "default".into(),
            send_keybindings_to_shell: false,
            terminal_persistent_sessions: true,
            terminal_persistent_scrollback: 1000,
            terminals,
            shell_integration: true,
            grammars: dir().join("grammars").to_string_lossy().into_owned(),
            associations: BTreeMap::new(),
            run: [
                ("py", r#"python "{file}""#),
                ("js", r#"node "{file}""#),
                ("mjs", r#"node "{file}""#),
                ("ts", r#"npx tsx "{file}""#),
                ("rs", "cargo run"),
                ("go", r#"go run "{file}""#),
                ("rb", r#"ruby "{file}""#),
                ("php", r#"php "{file}""#),
                ("lua", r#"lua "{file}""#),
                ("sh", r#"bash "{file}""#),
                ("ps1", r#"powershell -File "{file}""#),
                ("java", r#"java "{file}""#),
            ]
            .map(|(ext, command)| (ext.to_owned(), command.to_owned()))
            .into(),
            tasks: vec![],
            lsp: true,
            servers: [
                ("rs", "rust-analyzer"),
                ("py", "pyright-langserver --stdio"),
                ("ts", "typescript-language-server --stdio"),
                ("tsx", "typescript-language-server --stdio"),
                ("js", "typescript-language-server --stdio"),
                ("jsx", "typescript-language-server --stdio"),
                ("mjs", "typescript-language-server --stdio"),
                ("cjs", "typescript-language-server --stdio"),
                ("go", "gopls"),
                ("c", "clangd"),
                ("h", "clangd"),
                ("cpp", "clangd"),
                ("hpp", "clangd"),
                ("cc", "clangd"),
                ("cs", "csharp-ls"),
                ("java", "jdtls"),
                ("lua", "lua-language-server"),
                ("zig", "zls"),
                ("json", "vscode-json-language-server --stdio"),
                ("html", "vscode-html-language-server --stdio"),
                ("css", "vscode-css-language-server --stdio"),
            ]
            .map(|(ext, command)| (ext.to_owned(), command.to_owned()))
            .into(),
            debug_adapters: [
                ("lldb", "codelldb"),
                ("codelldb", "codelldb"),
                ("lldb-dap", "lldb-dap"),
                ("gdb", "gdb --interpreter=dap"),
                ("python", "python -m debugpy.adapter"),
                ("debugpy", "python -m debugpy.adapter"),
                ("go", "dlv dap --listen=127.0.0.1:{port}"),
            ]
            .map(|(kind, command)| (kind.to_owned(), command.to_owned()))
            .into(),
            diagnostics: true,
            status_problems: true,
            font_ligatures: false,
            letter_spacing: 0.0,
            cursor_smooth_caret_animation: "off".into(),
            copy_with_syntax_highlighting: true,
            code_lens: true,
            lightbulb: true,
            inlay_hints: true,
            color_decorators: true,
            unicode_invisible: true,
            unicode_ambiguous: true,
            semantic_highlighting: true,
            suggest_rows: 5,
            breadcrumbs: true,
            inline_blame: false,
            local_history: true,
            format_on_save: false,
            format_on_paste: false,
            format_on_type: false,
            linked_editing: true,
            word_wrap: "off".into(),
            word_wrap_column: 80,
            line_numbers: "on".into(),
            cursor_style: "line".into(),
            cursor_blinking: "smooth".into(),
            line_height: 0.0,
            render_line_highlight: "line".into(),
            scroll_beyond_last_line: true,
            smooth_scrolling: true,
            cursor_surrounding_lines: 0,
            peek_context_lines: 7,
            mouse_wheel_zoom: false,
            multi_cursor_modifier: "alt".into(),
            column_selection: false,
            word_separators: WORD_SEPARATORS.into(),
            match_brackets: "always".into(),
            selection_highlight: true,
            minimap_side: "right".into(),
            detect_indentation: true,
            auto_close: true,
            auto_close_quotes: true,
            auto_surround: true,
            auto_indent: "full".into(),
            trim_final_newlines: false,
            eol: "auto".into(),
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            render_whitespace: "none".into(),
            rulers: vec![],
            sticky_scroll: true,
            bracket_colors: true,
            guides: true,
            diff_side_by_side: true,
            hot_exit: true,
            tab_close_button: "right".into(),
            enable_preview: true,
            wrap_tabs: false,
            confirm_delete: true,
            explorer_sort: "default".into(),
            zoom_level: 0.0,
            restore_session: true,
            large_mb: 20,
            auto_update: true,
            files_exclude: ["**/.git", "**/.svn", "**/.hg", "**/.DS_Store", "**/Thumbs.db"].map(str::to_owned).into(),
            compact_folders: true,
            confirm_drag_and_drop: true,
            file_nesting: false,
            file_nesting_expand: true,
            file_nesting_patterns: [
                ("*.ts", "${capture}.js"),
                ("*.js", "${capture}.js.map, ${capture}.min.js, ${capture}.d.ts"),
                ("*.jsx", "${capture}.js"),
                ("*.tsx", "${capture}.ts"),
                ("tsconfig.json", "tsconfig.*.json"),
                ("package.json", "package-lock.json, yarn.lock, pnpm-lock.yaml, bun.lockb, bun.lock"),
                ("Cargo.toml", "Cargo.lock"),
            ]
            .map(|(parent, children)| (parent.to_owned(), children.to_owned()))
            .into(),
            auto_detect_high_contrast: true,
            preferred_high_contrast_theme: "hc-dark".into(),
            preferred_high_contrast_light_theme: "hc-light".into(),
            reduce_motion: "auto".into(),
            accessibility_support: "auto".into(),
            sidebar_location: "left".into(),
            panel_location: "bottom".into(),
            panel_alignment: "center".into(),
            activity_bar_location: "default".into(),
            zen_full_screen: true,
            zen_center_layout: true,
            zen_show_tabs: "multiple".into(),
            zen_hide_line_numbers: true,
            zen_restore: true,
            langs: BTreeMap::new(),
            folders: BTreeMap::new(),
        }
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

fn profile(name: &str, program: &str, args: &[&str]) -> ShellProfile {
    ShellProfile {
        name: name.into(),
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
    }
}

#[cfg(windows)]
fn default_terminals() -> Vec<ShellProfile> {
    let mut out = vec![];
    if on_path("pwsh.exe") {
        out.push(profile("PowerShell", "pwsh.exe", &["-NoLogo"]));
    } else {
        out.push(profile("PowerShell", "powershell.exe", &["-NoLogo"]));
    }
    out.push(profile("Command Prompt", "cmd.exe", &[]));
    let git_bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
    if git_bash.is_file() {
        out.push(profile("Git Bash", &git_bash.to_string_lossy(), &["--login", "-i"]));
    }
    if on_path("wsl.exe") {
        out.push(profile("WSL", "wsl.exe", &[]));
    }
    out
}

#[cfg(not(windows))]
fn default_terminals() -> Vec<ShellProfile> {
    let mut out = vec![];
    if let Ok(shell) = std::env::var("SHELL") {
        let name = Path::new(&shell).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "shell".into());
        out.push(profile(&name, &shell, &["-l"]));
    }
    for (name, program) in [("bash", "bash"), ("zsh", "zsh")] {
        if on_path(program) && !out.iter().any(|p: &ShellProfile| p.name == name) {
            out.push(profile(name, program, &["-l"]));
        }
    }
    out
}

pub fn dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("halo")
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| crate::ui::notify(format!("{name} could not be read, defaults are in use: {e}")))
        .ok()
}

pub(crate) fn write<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value)
        && let Err(error) = std::fs::write(path, text)
    {
        crate::ui::notify(format!("{} could not be written: {error}", path.display()));
    }
}

pub fn font_files() -> Vec<(&'static str, String)> {
    let windows = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_default().join("Fonts");
    [
        ("Consolas", windows.join("consola.ttf")),
        ("Cascadia Code", windows.join("CascadiaCode.ttf")),
        ("Cascadia Mono", windows.join("CascadiaMono.ttf")),
        ("Fira Code", windows.join("FiraCode-Regular.ttf")),
        ("Fira Code", "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf".into()),
        ("Fira Code", "/Library/Fonts/FiraCode-Regular.ttf".into()),
        ("Courier New", windows.join("cour.ttf")),
        ("Lucida Console", windows.join("lucon.ttf")),
        ("Menlo", "/System/Library/Fonts/Menlo.ttc".into()),
        ("Monaco", "/System/Library/Fonts/Monaco.ttf".into()),
        ("DejaVu Sans Mono", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".into()),
        ("Liberation Mono", "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf".into()),
        ("Ubuntu Mono", "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf".into()),
    ]
    .into_iter()
    .filter(|(_, file)| file.is_file())
    .map(|(name, file)| (name, file.to_string_lossy().into_owned()))
    .collect()
}

impl Settings {
    pub fn path() -> PathBuf {
        dir().join("settings.json")
    }

    pub fn load() -> Self {
        crate::layers::load()
    }

    pub fn save(&mut self) {
        *self = crate::layers::save(self);
    }

    pub fn lang(&self, id: &str) -> &Settings {
        self.langs.get(id).unwrap_or(self)
    }

    pub fn of(&self, path: Option<&Path>) -> &Settings {
        let folder = path.and_then(|p| self.folders.iter().filter(|(f, _)| p.starts_with(f)).max_by_key(|(f, _)| f.components().count()));
        let base = folder.map_or(self, |(_, s)| s);
        match path {
            Some(path) if !base.langs.is_empty() => base.lang(&crate::lsp::language_id(path)),
            _ => base,
        }
    }

    pub fn terminal(&self, name: &str) -> Option<&ShellProfile> {
        self.terminals.iter().find(|p| p.name == name)
    }

    pub fn default_profile(&self) -> Option<&ShellProfile> {
        self.terminal(&self.default_terminal).or_else(|| self.terminals.first())
    }

    pub fn run_command(&self, file: &Path, shell_path: &str) -> Option<String> {
        let ext = file.extension()?.to_string_lossy().to_lowercase();
        Some(self.run.get(&ext)?.replace("{file}", shell_path))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Backup {
    pub path: Option<PathBuf>,
    pub title: String,
    pub file: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Session {
    pub folder: Option<PathBuf>,
    pub workspace: crate::workspace::Workspace,
    pub files: Vec<PathBuf>,
    pub pinned: Vec<(usize, PathBuf)>,
    pub grid: Option<serde_json::Value>,
    pub backups: Vec<Backup>,
    pub recent: Vec<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    pub sidebar: bool,
    pub panel: bool,
    pub ai: bool,
    pub outline: bool,
    pub outline_sort: u8,
    pub timeline: bool,
    pub explorer_sizes: [Option<f32>; 4],
    pub breakpoints: BTreeMap<PathBuf, BTreeMap<PathBuf, Vec<crate::debug::Breakpoint>>>,
    pub layout: crate::workbench::Workbench,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            folder: None,
            workspace: Default::default(),
            files: vec![],
            pinned: vec![],
            grid: None,
            backups: vec![],
            recent: vec![],
            recent_files: vec![],
            sidebar: true,
            panel: true,
            ai: false,
            outline: true,
            outline_sort: 0,
            timeline: false,
            explorer_sizes: [None; 4],
            breakpoints: BTreeMap::new(),
            layout: Default::default(),
        }
    }
}

impl Session {
    fn path() -> PathBuf {
        dir().join("session.json")
    }

    pub fn load() -> Self {
        read(&Self::path()).unwrap_or_default()
    }

    pub fn save(&mut self) {
        if crate::windows::secondary() || cfg!(test) {
            return;
        }
        if Self::shares_recent() {
            let disk = Self::load();
            (self.recent, self.recent_files) = (disk.recent, disk.recent_files);
        }
        write(&Self::path(), self);
    }

    fn shares_recent() -> bool {
        !cfg!(test) && std::env::var_os("HALO_COMMANDS").is_none()
    }

    // every window writes its recents straight to disk so the jump list and other windows see them
    fn update_recent(&mut self, update: impl Fn(&mut Session)) {
        update(self);
        if Self::shares_recent() {
            let mut disk = Self::load();
            update(&mut disk);
            (self.recent, self.recent_files) = (disk.recent.clone(), disk.recent_files.clone());
            write(&Self::path(), &disk);
        }
    }

    pub fn remember(&mut self, folder: &Path) {
        self.update_recent(|s| push_recent(&mut s.recent, folder));
    }

    pub fn remember_file(&mut self, file: &Path) {
        self.update_recent(|s| push_recent(&mut s.recent_files, file));
    }
}

fn push_recent(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(10);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_run_commands_and_keeps_old_settings_files_readable() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"light","tasks":[{"name":"build","command":"cargo build"}]}"#).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.tasks[0].command, "cargo build");
        assert!(settings.lsp && settings.code_lens && !settings.format_on_save);
        assert_eq!(settings.servers.get("rs").map(String::as_str), Some("rust-analyzer"));
        assert_eq!(settings.run_command(Path::new("a/b.PY"), "x y.py").as_deref(), Some("python \"x y.py\""));
        assert_eq!(settings.run_command(Path::new("notes.txt"), "notes.txt"), None);
    }

    #[test]
    fn sessions_keep_pinned_tabs_and_old_files_still_load() {
        let session = Session { pinned: vec![(1, PathBuf::from("a.rs"))], ..Session::default() };
        let back: Session = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(back.pinned, vec![(1, PathBuf::from("a.rs"))]);
        let old: Session = serde_json::from_str(r#"{"folder":null,"files":["x.rs"],"sidebar":false}"#).unwrap();
        assert!(old.pinned.is_empty() && old.files.len() == 1 && !old.sidebar);
    }
}
