use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

pub const UNTITLED: &str = "Untitled (Workspace)";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Folder {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Workspace {
    pub folders: Vec<Folder>,
    pub file: Option<PathBuf>,
    pub raw: Map<String, Value>,
}

static CURRENT: Mutex<Option<Workspace>> = Mutex::new(None);

pub fn current() -> Workspace {
    CURRENT.lock().unwrap().clone().unwrap_or_default()
}

pub fn set(workspace: &Workspace) {
    *CURRENT.lock().unwrap() = Some(workspace.clone());
}

pub fn is_workspace_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("code-workspace"))
}

fn base_name(path: &Path) -> String {
    path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.components().next_back(), Some(Component::Normal(_))) => drop(out.pop()),
            other => out.push(other),
        }
    }
    out
}

fn relative_to(path: &Path, base: &Path) -> Option<String> {
    let (a, b): (Vec<_>, Vec<_>) = (path.components().collect(), base.components().collect());
    if a.first() != b.first() {
        return None;
    }
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let ups = std::iter::repeat_n("..".to_owned(), b.len() - common);
    let rest = a[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned());
    let joined = ups.chain(rest).collect::<Vec<_>>().join("/");
    Some(if joined.is_empty() { ".".to_owned() } else { joined })
}

impl Workspace {
    pub fn single(path: PathBuf) -> Self {
        Self { folders: vec![Folder { path, name: None }], ..Default::default() }
    }

    pub fn is_multi(&self) -> bool {
        self.folders.len() > 1 || self.file.is_some()
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.folders.iter().map(|f| f.path.clone()).collect()
    }

    pub fn first(&self) -> Option<PathBuf> {
        self.folders.first().map(|f| f.path.clone())
    }

    pub fn name(&self) -> Option<String> {
        match (&self.file, self.folders.as_slice()) {
            (Some(file), _) => Some(format!("{} (Workspace)", file.file_stem().unwrap_or_default().to_string_lossy())),
            (None, []) => None,
            (None, [one]) => Some(one.name.clone().unwrap_or_else(|| base_name(&one.path))),
            (None, _) => Some(UNTITLED.to_owned()),
        }
    }

    pub fn add(&mut self, path: PathBuf) -> bool {
        if self.folders.iter().any(|f| f.path == path) {
            return false;
        }
        self.folders.push(Folder { path, name: None });
        true
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.folders.len() {
            self.folders.remove(index);
        }
    }

    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.folders.len() && to < self.folders.len() {
            let folder = self.folders.remove(from);
            self.folders.insert(to, folder);
        }
    }

    pub fn names(&self) -> Vec<String> {
        let base: Vec<String> = self.folders.iter().map(|f| f.name.clone().unwrap_or_else(|| base_name(&f.path))).collect();
        base.iter()
            .zip(&self.folders)
            .map(|(name, folder)| match base.iter().filter(|n| *n == name).count() > 1 {
                true => format!("{name} ({})", folder.path.parent().map(base_name).unwrap_or_default()),
                false => name.clone(),
            })
            .collect()
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        let names = self.names();
        names.iter().position(|n| n == name).or_else(|| self.folders.iter().position(|f| f.name.as_deref().unwrap_or(&base_name(&f.path)) == name))
    }

    pub fn owner(&self, path: &Path) -> Option<usize> {
        let found = self.folders.iter().enumerate().filter(|(_, f)| path.starts_with(&f.path));
        found.max_by_key(|(_, f)| f.path.components().count()).map(|(i, _)| i)
    }

    pub fn owner_path(&self, path: &Path) -> Option<PathBuf> {
        self.owner(path).map(|i| self.folders[i].path.clone())
    }

    pub fn relative(&self, path: &Path) -> Option<String> {
        let index = self.owner(path)?;
        let rest = path.strip_prefix(&self.folders[index].path).ok()?.to_string_lossy().replace('\\', "/");
        Some(match (self.folders.len() > 1, rest.is_empty()) {
            (false, _) => rest,
            (true, true) => self.names()[index].clone(),
            (true, false) => format!("{}/{rest}", self.names()[index]),
        })
    }

    pub fn section(&self, key: &str) -> Option<&Value> {
        self.raw.get(key)
    }

    pub fn settings(&self) -> Map<String, Value> {
        self.raw.get("settings").and_then(Value::as_object).cloned().unwrap_or_default()
    }

    pub fn parse(text: &str, file: &Path) -> Result<Self, String> {
        let Value::Object(raw) = crate::jsonc::parse(text)? else { return Err("a workspace file must hold an object".into()) };
        let dir = file.parent().unwrap_or(Path::new(""));
        let folders = raw.get("folders").and_then(Value::as_array).into_iter().flatten();
        let folders = folders
            .filter_map(|f| {
                let path = Path::new(f.get("path")?.as_str()?);
                let name = f.get("name").and_then(Value::as_str).map(str::to_owned);
                Some(Folder { path: normalize(&dir.join(path)), name })
            })
            .collect();
        Ok(Self { folders, file: Some(file.to_path_buf()), raw })
    }

    pub fn to_text(&self, file: &Path) -> String {
        let dir = file.parent().unwrap_or(Path::new(""));
        let folders: Vec<Value> = self
            .folders
            .iter()
            .map(|f| {
                let path = relative_to(&f.path, dir).unwrap_or_else(|| f.path.to_string_lossy().into_owned());
                let mut entry = json!({ "path": path });
                if let Some(name) = &f.name {
                    entry["name"] = json!(name);
                }
                entry
            })
            .collect();
        let mut raw = self.raw.clone();
        raw.insert("folders".into(), Value::Array(folders));
        if !raw.contains_key("settings") {
            raw.insert("settings".into(), json!({}));
        }
        serde_json::to_string_pretty(&Value::Object(raw)).unwrap_or_default()
    }

    pub fn open(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{} could not be read: {e}", file.display()))?;
        Self::parse(&text, file).map_err(|e| format!("{} could not be read: {e}", file.display()))
    }

    pub fn save_as(&mut self, file: &Path) -> Result<(), String> {
        std::fs::write(file, self.to_text(file)).map_err(|e| format!("{} could not be written: {e}", file.display()))?;
        self.file = Some(file.to_path_buf());
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        match self.file.clone() {
            Some(file) => self.save_as(&file),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(paths: &[&str]) -> Workspace {
        let mut w = Workspace::default();
        paths.iter().for_each(|p| drop(w.add(PathBuf::from(p))));
        w
    }

    #[test]
    fn adds_removes_reorders_and_names_folders() {
        let mut w = ws(&["/a/api", "/a/web"]);
        assert!(!w.add(PathBuf::from("/a/api")));
        assert!(w.add(PathBuf::from("/b/src")) && w.add(PathBuf::from("/c/src")));
        assert_eq!(w.names(), ["api", "web", "src (b)", "src (c)"]);
        assert_eq!(w.find("src (c)"), Some(3));
        assert_eq!(w.find("web"), Some(1));
        w.reorder(3, 0);
        assert_eq!(w.roots()[0], PathBuf::from("/c/src"));
        w.remove(0);
        w.remove(9);
        assert_eq!(w.names(), ["api", "web", "src"]);
        w.folders[1].name = Some("Frontend".into());
        assert_eq!(w.names()[1], "Frontend");
        assert_eq!(w.name().as_deref(), Some(UNTITLED));
        assert_eq!(ws(&["/a/api"]).name().as_deref(), Some("api"));
        assert!(!ws(&["/a/api"]).is_multi() && w.is_multi());
        assert_eq!(Workspace::default().name(), None);
    }

    #[test]
    fn longest_root_owns_a_path_and_relative_paths_start_at_the_root_name() {
        let w = ws(&["/w", "/w/api", "/other"]);
        assert_eq!(w.owner(Path::new("/w/api/src/main.rs")), Some(1));
        assert_eq!(w.owner(Path::new("/w/web/x.rs")), Some(0));
        assert_eq!(w.owner(Path::new("/elsewhere/x.rs")), None);
        assert_eq!(w.owner(Path::new("/wx/a")), None);
        assert_eq!(w.relative(Path::new("/w/api/src/main.rs")).as_deref(), Some("api/src/main.rs"));
        assert_eq!(w.relative(Path::new("/other")).as_deref(), Some("other"));
        assert_eq!(ws(&["/w"]).relative(Path::new("/w/a/b.rs")).as_deref(), Some("a/b.rs"));
    }

    #[test]
    fn reads_and_writes_workspace_files_with_comments_relative_paths_and_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("halo-workspace-test-{}", std::process::id()));
        let file = dir.join("proj").join("all.code-workspace");
        let text = r#"{
            // the folders
            "folders": [{ "path": "." }, { "path": "../api", "name": "API" }, { "path": "sub/./web" },],
            "settings": { "editor.tabSize": 2 }, /* kept */
            "extensions": { "recommendations": ["x.y"] },
            "launch": { "configurations": [] },
        }"#;
        let w = Workspace::parse(text, &file).unwrap();
        assert_eq!(w.roots(), [dir.join("proj"), dir.join("api"), dir.join("proj").join("sub").join("web")]);
        assert_eq!(w.names(), ["proj", "API", "web"]);
        assert_eq!(w.settings()["editor.tabSize"], json!(2));
        assert!(w.section("launch").is_some() && w.section("tasks").is_none());
        assert_eq!(w.name().as_deref(), Some("all (Workspace)"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut w = w;
        w.add(dir.join("proj").join("more"));
        w.save_as(&file).unwrap();
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back["folders"], json!([{"path": "."}, {"path": "../api", "name": "API"}, {"path": "sub/web"}, {"path": "more"}]));
        assert_eq!(back["extensions"], json!({"recommendations": ["x.y"]}));
        assert_eq!(Workspace::open(&file).unwrap().roots(), w.roots());
        assert!(Workspace::parse("[1]", &file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
