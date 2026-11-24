use crate::command::Command;
use crate::theme::Theme;
use crate::ui;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static TRUSTED: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn file() -> PathBuf {
    crate::settings::dir().join("trust.json")
}

fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/');
    if cfg!(windows) { text.to_lowercase() } else { text.to_owned() }
}

pub fn covers(trusted: &[PathBuf], folder: &Path) -> bool {
    let folder = key(folder);
    trusted.iter().map(|t| key(t)).any(|t| folder == t || folder.strip_prefix(&t).is_some_and(|rest| rest.starts_with('/')))
}

fn with<R>(f: impl FnOnce(&mut Vec<PathBuf>) -> R) -> R {
    let mut guard = TRUSTED.lock().unwrap();
    let list = guard.get_or_insert_with(|| std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    f(list)
}

pub fn seed(folders: impl IntoIterator<Item = PathBuf>) {
    if file().exists() {
        return;
    }
    let folders: Vec<PathBuf> = folders.into_iter().collect();
    crate::settings::write(&file(), &folders);
    *TRUSTED.lock().unwrap() = Some(folders);
}

pub fn trusted(folder: &Path) -> bool {
    cfg!(test) || with(|list| covers(list, folder))
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn set(folder: &Path, trust: bool) {
    with(|list| {
        match trust {
            true if !covers(list, folder) => list.push(folder.to_path_buf()),
            false => {
                let me = [folder.to_path_buf()];
                list.retain(|t| !covers(&me, t) && !covers(std::slice::from_ref(t), folder));
            }
            _ => {}
        }
        crate::settings::write(&file(), list);
    });
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

#[derive(Default)]
pub struct Prompt {
    pub folder: Option<PathBuf>,
    asked: Vec<PathBuf>,
}

impl Prompt {
    pub fn check(&mut self, folder: Option<&Path>) {
        if let Some(folder) = folder.filter(|f| !trusted(f) && !self.asked.iter().any(|a| a == f)) {
            self.asked.push(folder.to_path_buf());
            self.folder = Some(folder.to_path_buf());
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Command> {
        let folder = self.folder.clone()?;
        let mut choice = None;
        let modal = ui::modal(ctx, "workspace_trust", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(420.0);
            ui.heading("Do you trust the authors of this folder?");
            ui.label(egui::RichText::new(folder.display().to_string()).color(theme.text_dim));
            ui.add_space(6.0);
            ui.label("In Restricted Mode, Halo does not apply the folder's .vscode/settings.json, run its tasks (including detected build tasks) or start its launch.json configurations, since they can run code from the folder. Editing and browsing work as usual.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if keys.primary(ui, "Trust Folder", true) {
                    choice = Some(true);
                }
                if ui.button("Stay in Restricted Mode").clicked() {
                    choice = Some(false);
                }
            });
        });
        if modal.should_close() {
            choice = Some(false);
        }
        let trust = choice?;
        self.folder = None;
        Some(Command::SetFolderTrust(folder, trust))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_covers_subfolders_but_not_siblings() {
        let list = vec![PathBuf::from("/home/a/code")];
        assert!(covers(&list, Path::new("/home/a/code")));
        assert!(covers(&list, Path::new("/home/a/code/x/")));
        assert!(!covers(&list, Path::new("/home/a/codex")));
        assert!(!covers(&list, Path::new("/home/a")));
        assert_eq!(covers(&[PathBuf::from("C:\\Work")], Path::new("c:/work/app")), cfg!(windows));
    }
}
