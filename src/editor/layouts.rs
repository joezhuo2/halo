use super::{Editors, Sel};
use crate::command::Command;
use crate::ui;
use eframe::egui::Ui;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Saved = BTreeMap<PathBuf, BTreeMap<String, Value>>;

#[derive(Default)]
pub struct Layouts {
    named: BTreeMap<String, Value>,
    folder: PathBuf,
    naming: Option<String>,
}

impl Layouts {
    pub fn store(&self, saved: &mut Saved) {
        match self.named.is_empty() {
            true => saved.remove(&self.folder),
            false => saved.insert(self.folder.clone(), self.named.clone()),
        };
    }

    pub fn restore(&mut self, saved: &Saved, folder: Option<PathBuf>) {
        self.folder = folder.unwrap_or_default();
        self.named = saved.get(&self.folder).cloned().unwrap_or_default();
    }

    pub fn names(&self) -> Vec<String> {
        self.named.keys().cloned().collect()
    }
}

impl Editors {
    pub(super) fn layout_exec(&mut self, cmd: &Command) -> bool {
        use Command::*;
        let g = self.active.min(self.groups.len() - 1);
        let zoom = self.groups[g].zoom;
        match cmd {
            ZoomInEditor => self.groups[g].zoom = (zoom + 1).min(10),
            ZoomOutEditor => self.groups[g].zoom = (zoom - 1).max(-5),
            ZoomResetEditor => self.groups[g].zoom = 0,
            SaveLayout => self.layouts.naming = Some(String::new()),
            OpenLayout(name) => self.open_layout(name),
            RemoveLayout(name) => drop(self.layouts.named.remove(name)),
            _ => return false,
        }
        true
    }

    fn open_layout(&mut self, name: &str) {
        let Some(saved) = self.layouts.named.get(name).cloned() else { return };
        let before: Vec<u64> = self.docs.keys().chain(self.terminals.keys()).copied().collect();
        let keep: Vec<u64> = self.groups.iter().flat_map(|g| &g.tabs).map(|t| t.doc)
            .filter(|doc| self.terminals.contains_key(doc) || self.docs.get(doc).is_some_and(|d| d.buf.is_dirty()))
            .collect();
        if !self.restore_grid(&saved) {
            return ui::notify(format!("The layout {name} could not be restored."));
        }
        (self.closing, self.switcher, self.drag, self.drop) = (None, None, None, None);
        self.previews.clear();
        for doc in keep {
            if self.references(doc) == 0 {
                self.add_tab(doc, vec![Sel::caret(0)]);
            }
        }
        before.into_iter().for_each(|doc| self.drop_doc(doc));
        self.want_focus = true;
    }

    pub(super) fn layout_dialog(&mut self, ui: &Ui) {
        let Some(mut name) = self.layouts.naming.take() else { return };
        let note = "Saves the open editors and splits of this folder under a name. Layouts: Switch... brings them back.";
        match crate::scm::prompt(ui, "Save layout", note, &mut [("Name", &mut name, 1)], "Save") {
            Some(true) if !name.trim().is_empty() => {
                self.layouts.named.insert(name.trim().to_owned(), self.grid_state());
                self.want_focus = true;
            }
            Some(_) => self.want_focus = true,
            None => self.layouts.naming = Some(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use eframe::egui;

    #[test]
    fn layouts_switch_tabs_and_keep_unsaved_ones() {
        let dir = std::env::temp_dir().join(format!("halo_layouts_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let [a, b, c] = ["a.txt", "b.txt", "c.txt"].map(|f| dir.join(f));
        for p in [&a, &b, &c] {
            std::fs::write(p, "x").unwrap();
        }
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        editors.open_kept(&a);
        editors.open_kept(&b);
        editors.layouts.named.insert("two".into(), editors.grid_state());
        editors.close_all_tabs();
        editors.open_kept(&c);
        editors.exec(&Command::NewFile, &settings, &ctx);
        editors.exec(&Command::PasteText("y".into()), &settings, &ctx);
        editors.exec(&Command::OpenLayout("two".into()), &settings, &ctx);
        let mut open = editors.open_paths();
        open.sort();
        assert_eq!(open, vec![a.clone(), b.clone()]);
        assert_eq!(editors.open_tabs().iter().map(Vec::len).sum::<usize>(), 3);
        editors.exec(&Command::ZoomInEditor, &settings, &ctx);
        assert_eq!(editors.groups[editors.active].zoom, 1);

        let mut saved = Saved::new();
        editors.layouts.restore(&saved, Some(dir.clone()));
        editors.layouts.named.insert("x".into(), Value::Null);
        editors.layouts.store(&mut saved);
        assert!(saved[&dir].contains_key("x"));
        editors.exec(&Command::RemoveLayout("x".into()), &settings, &ctx);
        editors.layouts.store(&mut saved);
        assert!(saved.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
