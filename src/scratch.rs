use std::path::{Path, PathBuf};

pub const KINDS: &[(&str, &str)] = &[
    ("txt", "Plain Text"),
    ("md", "Markdown"),
    ("json", "JSON"),
    ("rs", "Rust"),
    ("py", "Python"),
    ("js", "JavaScript"),
    ("ts", "TypeScript"),
    ("sql", "SQL"),
    ("sh", "Shell"),
    ("ps1", "PowerShell"),
];

pub fn dir(folder: Option<&Path>) -> PathBuf {
    let name: String = match folder {
        Some(folder) => folder.to_string_lossy().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect(),
        None => "no-folder".to_owned(),
    };
    crate::settings::dir().join("scratch").join(name)
}

pub fn create(dir: &Path, ext: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let free = (1..).map(|n| dir.join(format!("scratch-{n}.{ext}"))).find(|p| !p.exists()).unwrap();
    std::fs::write(&free, "")?;
    Ok(free)
}

pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<_> = entries.flatten().filter(|e| e.path().is_file()).map(|e| (e.metadata().and_then(|m| m.modified()).ok(), e.path())).collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|f| f.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_files_get_free_names_per_folder() {
        assert_ne!(dir(Some(Path::new("/a/b"))), dir(Some(Path::new("/a/c"))));
        assert!(dir(Some(Path::new("C:\\work\\x"))).ends_with("C__work_x"));
        let at = std::env::temp_dir().join(format!("halo_scratch_{}", std::process::id()));
        let first = create(&at, "md").unwrap();
        let second = create(&at, "md").unwrap();
        assert!(first.ends_with("scratch-1.md") && second.ends_with("scratch-2.md"));
        assert_eq!(list(&at).len(), 2);
        let _ = std::fs::remove_dir_all(at);
    }
}
