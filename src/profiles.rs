use crate::command::Command;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

pub const FILES: [&str; 3] = ["settings.json", "keybindings.json", "snippets.json"];
pub const DEFAULT: &str = "Default";

pub fn root(base: &Path) -> PathBuf {
    base.join("profiles")
}

pub fn active(base: &Path) -> String {
    std::fs::read_to_string(base.join("profile")).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT.to_owned())
}

pub fn list(base: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root(base)).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.extend([DEFAULT.to_owned(), active(base)]);
    names.sort_by_key(|n| (n != DEFAULT, n.to_lowercase()));
    names.dedup();
    names
}

pub fn valid(name: &str) -> Result<(), &'static str> {
    match name {
        "" => Err("Type a name."),
        n if n.len() > 40 => Err("Use at most 40 characters."),
        n if n.starts_with('.') || n.ends_with(' ') || !n.chars().all(|c| c.is_alphanumeric() || " -_.".contains(c)) => Err("Use letters, digits, spaces, dots, dashes and underscores."),
        _ => Ok(()),
    }
}

fn copy_files(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for file in FILES {
        let (src, dst) = (from.join(file), to.join(file));
        match src.is_file() {
            true => drop(std::fs::copy(&src, &dst).map_err(|e| format!("{file}: {e}"))?),
            false if dst.is_file() => std::fs::remove_file(&dst).map_err(|e| format!("{file}: {e}"))?,
            false => {}
        }
    }
    Ok(())
}

pub fn switch(base: &Path, to: &str) -> Result<(), String> {
    valid(to)?;
    let current = active(base);
    if current == to {
        return Ok(());
    }
    copy_files(base, &root(base).join(&current))?;
    copy_files(&root(base).join(to), base)?;
    std::fs::write(base.join("profile"), to).map_err(|e| e.to_string())
}

pub fn create(base: &Path, name: &str, copy: bool) -> Result<(), String> {
    valid(name)?;
    if list(base).iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return Err(format!("A profile named {name} already exists."));
    }
    let dir = root(base).join(name);
    match copy {
        true => copy_files(base, &dir)?,
        false => std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?,
    }
    switch(base, name)
}

pub fn delete(base: &Path, name: &str) -> Result<(), String> {
    if name == active(base) || name == DEFAULT {
        return Err("Switch to another profile before deleting this one; the Default profile is kept.".into());
    }
    trash::delete(root(base).join(name)).map_err(|e| e.to_string())
}

pub fn picks(base: &Path) -> Vec<(String, String, Command)> {
    let current = active(base);
    let mut items: Vec<_> = list(base).into_iter().map(|n| (n.clone(), if n == current { "active".into() } else { String::new() }, Command::SwitchProfileTo(n))).collect();
    items.push(("New Profile...".into(), String::new(), Command::NewProfile));
    items
}

#[derive(Default)]
pub struct Dialog {
    current: Option<String>,
    naming: Option<(String, bool, Option<&'static str>)>,
    deleting: Option<String>,
}

impl Dialog {
    pub fn current(&mut self) -> &str {
        self.current.get_or_insert_with(|| active(&crate::settings::dir()))
    }

    pub fn changed(&mut self) {
        self.current = None;
    }

    pub fn name(&mut self) {
        self.naming = Some((String::new(), true, None));
    }

    pub fn delete(&mut self, name: String) {
        self.deleting = Some(name);
    }

    pub fn ui(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<Command> {
        let mut out = None;
        if let Some((name, copy, error)) = self.naming.as_mut() {
            let mut done = None;
            let modal = ui::modal(ctx, "new_profile", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(360.0);
                ui.heading("New Profile");
                ui.label("A profile keeps its own settings, keyboard shortcuts, snippets and theme.");
                ui.add_space(8.0);
                let edit = ui.add(egui::TextEdit::singleline(name).hint_text("Profile name").desired_width(f32::INFINITY));
                if edit.gained_focus() || ui.memory(|m| m.focused().is_none()) {
                    edit.request_focus();
                }
                ui.checkbox(copy, "Start from the current settings");
                if let Some(error) = error {
                    ui.colored_label(theme.deleted, *error);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if keys.primary(ui, "Create", !name.trim().is_empty()) {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
            if modal.should_close() {
                done = Some(false);
            }
            match done {
                Some(true) => match valid(name.trim()) {
                    Ok(()) => out = self.naming.take().map(|(name, copy, _)| Command::CreateProfile(name.trim().to_owned(), copy)),
                    Err(e) => *error = Some(e),
                },
                Some(false) => self.naming = None,
                None => {}
            }
        }
        if let Some(name) = self.deleting.clone() {
            let mut choice = None;
            let modal = ui::modal(ctx, "delete_profile", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(340.0);
                ui.heading("Delete Profile");
                ui.label(format!("Move the profile {name} and its settings to the trash?"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if keys.primary(ui, RichText::new("Delete").color(theme.deleted), true) {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if modal.should_close() {
                choice = Some(false);
            }
            if let Some(delete) = choice {
                self.deleting = None;
                if delete {
                    out = Some(Command::RemoveProfile(name));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_swap_the_settings_files() {
        let base = std::env::temp_dir().join(format!("halo-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("settings.json"), "{\"theme\":\"dark\"}").unwrap();
        std::fs::write(base.join("keybindings.json"), "[]").unwrap();
        assert_eq!((active(&base), list(&base)), (DEFAULT.to_owned(), vec![DEFAULT.to_owned()]));
        create(&base, "Work", false).unwrap();
        assert_eq!(active(&base), "Work");
        assert!(!base.join("settings.json").exists() && !base.join("keybindings.json").exists());
        std::fs::write(base.join("settings.json"), "{\"theme\":\"light\"}").unwrap();
        switch(&base, DEFAULT).unwrap();
        assert_eq!(std::fs::read_to_string(base.join("settings.json")).unwrap(), "{\"theme\":\"dark\"}");
        assert!(base.join("keybindings.json").exists());
        switch(&base, "Work").unwrap();
        assert_eq!(std::fs::read_to_string(base.join("settings.json")).unwrap(), "{\"theme\":\"light\"}");
        create(&base, "Copy", true).unwrap();
        assert_eq!(std::fs::read_to_string(root(&base).join("Copy").join("settings.json")).unwrap(), "{\"theme\":\"light\"}");
        assert_eq!(list(&base), ["Default", "Copy", "Work"]);
        assert!(create(&base, "work", false).is_err() && create(&base, "../x", false).is_err() && valid("a/b").is_err());
        assert!(delete(&base, "Copy").is_err() && delete(&base, DEFAULT).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
