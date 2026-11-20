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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub font_size: f32,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub minimap: bool,
    pub default_terminal: String,
    pub terminals: Vec<ShellProfile>,
    pub grammars: String,
    pub associations: BTreeMap<String, String>,
    pub run: BTreeMap<String, String>,
    pub tasks: Vec<Task>,
}

impl Default for Settings {
    fn default() -> Self {
        let terminals = default_terminals();
        Self {
            theme: "dark".into(),
            font_size: 13.0,
            tab_size: 4,
            insert_spaces: true,
            minimap: true,
            default_terminal: terminals.first().map(|p| p.name.clone()).unwrap_or_default(),
            terminals,
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

fn write<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, text);
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        dir().join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if !path.exists() {
            let settings = Self::default();
            settings.save();
            return settings;
        }
        read(&path).unwrap_or_default()
    }

    pub fn save(&self) {
        write(&Self::path(), self);
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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Session {
    pub folder: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub sidebar: bool,
    pub panel: bool,
    pub ai: bool,
    pub source_control: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            folder: None,
            files: vec![],
            recent: vec![],
            sidebar: true,
            panel: true,
            ai: false,
            source_control: false,
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

    pub fn save(&self) {
        write(&Self::path(), self);
    }

    pub fn remember(&mut self, folder: &Path) {
        self.recent.retain(|p| p != folder);
        self.recent.insert(0, folder.to_path_buf());
        self.recent.truncate(10);
    }
}
