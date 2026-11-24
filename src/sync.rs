use crate::profiles::{self, FILES};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};

const DESCRIPTION: &str = "Halo Settings Sync";
const ACTIVE: &str = "active-profile";

pub fn gather(base: &Path) -> BTreeMap<String, String> {
    let read = |path: &Path| std::fs::read_to_string(path).ok().filter(|t| !t.trim().is_empty());
    let mut files = BTreeMap::new();
    for file in FILES {
        if let Some(text) = read(&base.join(file)) {
            files.insert(file.to_owned(), text);
        }
    }
    files.insert(ACTIVE.to_owned(), profiles::active(base));
    for name in profiles::list(base) {
        for file in FILES {
            if let Some(text) = read(&profiles::root(base).join(&name).join(file)) {
                files.insert(format!("profiles__{name}__{file}"), text);
            }
        }
    }
    files
}

pub fn scatter(base: &Path, files: &BTreeMap<String, String>) -> Result<usize, String> {
    let backup = base.join("sync-backup");
    std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    for file in FILES.iter().chain(["profile"].iter()) {
        if base.join(file).is_file() {
            std::fs::copy(base.join(file), backup.join(file)).map_err(|e| e.to_string())?;
        }
    }
    let mut written = 0;
    for (name, text) in files {
        let target = match name.strip_prefix("profiles__").and_then(|rest| rest.rsplit_once("__")) {
            Some((profile, file)) if profiles::valid(profile).is_ok() && FILES.contains(&file) => profiles::root(base).join(profile).join(file),
            Some(_) => continue,
            None if name == ACTIVE && profiles::valid(text.trim()).is_ok() => base.join("profile"),
            None if FILES.contains(&name.as_str()) => base.join(name),
            None => continue,
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&target, text).map_err(|e| format!("{}: {e}", target.display()))?;
        written += 1;
    }
    Ok(written)
}

fn gh(base: &Path, args: &[&str], body: Option<&Value>) -> Result<String, String> {
    let input = std::env::temp_dir().join(format!("halo-sync-{}.json", std::process::id()));
    let input_text = input.to_string_lossy().into_owned();
    let mut all: Vec<&str> = args.to_vec();
    if let Some(body) = body {
        std::fs::write(&input, body.to_string()).map_err(|e| e.to_string())?;
        all.extend(["--input", &input_text]);
    }
    let out = crate::git::command("gh", base, &all);
    let _ = std::fs::remove_file(&input);
    out.map_err(|e| match e.contains("gh auth login") || e.contains("not logged") {
        true => "Sign in to GitHub first: run gh auth login, or use the Accounts menu.".to_owned(),
        false => e,
    })
}

fn gist_id(base: &Path) -> Result<Option<String>, String> {
    let stored = std::fs::read_to_string(base.join("sync.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["gist"].as_str().map(str::to_owned));
    if stored.is_some() {
        return Ok(stored);
    }
    let found = gh(base, &["api", "gists", "--paginate", "--jq", &format!(".[] | select(.description == \"{DESCRIPTION}\") | .id")], None)?;
    Ok(found.lines().next().map(|id| id.trim().to_owned()).filter(|id| !id.is_empty()))
}

fn remember(base: &Path, id: &str) {
    crate::settings::write(&base.join("sync.json"), &json!({ "gist": id }));
}

pub fn upload(base: &Path) -> Result<String, String> {
    let local = gather(base);
    let mut files: Map<String, Value> = local.iter().map(|(name, text)| (name.clone(), json!({ "content": text }))).collect();
    match gist_id(base)? {
        Some(id) => {
            let remote = gh(base, &["api", &format!("gists/{id}"), "--jq", ".files | keys[]"], None)?;
            for name in remote.lines().map(str::trim).filter(|n| !n.is_empty() && !local.contains_key(*n)) {
                files.insert(name.to_owned(), Value::Null);
            }
            gh(base, &["api", "--method", "PATCH", &format!("gists/{id}"), "--jq", ".id"], Some(&json!({ "files": files })))?;
            remember(base, &id);
        }
        None => {
            let id = gh(base, &["api", "--method", "POST", "gists", "--jq", ".id"], Some(&json!({ "description": DESCRIPTION, "public": false, "files": files })))?;
            remember(base, id.trim());
        }
    }
    Ok(format!("Uploaded {} settings files to a secret gist.", local.len()))
}

pub fn download(base: &Path) -> Result<String, String> {
    let Some(id) = gist_id(base)? else { return Err("No synced settings yet. Run Settings Sync: Upload Settings on the other machine first.".into()) };
    let text = gh(base, &["api", &format!("gists/{id}")], None)?;
    let gist: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for (name, file) in gist["files"].as_object().into_iter().flatten() {
        if file["truncated"].as_bool() == Some(true) {
            return Err(format!("{name} is too large to sync."));
        }
        files.insert(name.clone(), file["content"].as_str().unwrap_or_default().to_owned());
    }
    remember(base, &id);
    let written = scatter(base, &files)?;
    Ok(format!("Downloaded {written} settings files. The previous ones are in sync-backup in the settings folder."))
}

#[derive(Default)]
pub struct Sync {
    rx: Option<Receiver<(bool, Result<String, String>)>>,
}

impl Sync {
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    pub fn start(&mut self, pull: bool, ctx: &eframe::egui::Context) {
        if self.busy() {
            return crate::ui::notify_info("Settings Sync is already running.");
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let base = crate::settings::dir();
            let _ = tx.send((pull, if pull { download(&base) } else { upload(&base) }));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    pub fn poll(&mut self) -> Option<bool> {
        let (pull, result) = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        match result {
            Ok(message) => crate::ui::notify_info(message),
            Err(error) => crate::ui::notify(format!("Settings Sync: {error}")),
        }
        Some(pull)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gathers_and_scatters_profiles_with_a_backup() {
        let dir = std::env::temp_dir().join(format!("halo-sync-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("settings.json"), "{\"theme\":\"light\"}").unwrap();
        profiles::create(&a, "Work", true).unwrap();
        std::fs::write(a.join("keybindings.json"), "[1]").unwrap();
        let files = gather(&a);
        assert_eq!(files.keys().collect::<Vec<_>>(), ["active-profile", "keybindings.json", "profiles__Default__settings.json", "profiles__Work__settings.json", "settings.json"]);
        assert_eq!(files["active-profile"], "Work");
        std::fs::write(b.join("settings.json"), "{\"theme\":\"dark\"}").unwrap();
        let mut hostile = files.clone();
        hostile.insert("profiles__..__settings.json".into(), "x".into());
        hostile.insert("evil.txt".into(), "x".into());
        assert_eq!(scatter(&b, &hostile).unwrap(), 5);
        assert_eq!(std::fs::read_to_string(b.join("sync-backup").join("settings.json")).unwrap(), "{\"theme\":\"dark\"}");
        assert_eq!(std::fs::read_to_string(b.join("settings.json")).unwrap(), "{\"theme\":\"light\"}");
        assert_eq!(profiles::active(&b), "Work");
        assert!(profiles::root(&b).join("Default").join("settings.json").is_file() && !b.join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
