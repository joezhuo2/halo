use crate::command::{self, Binding, Command, Entry};
use crate::theme::Theme;
use crate::ui::icons;
use eframe::egui::{self, CornerRadius, Id, Key, KeyboardShortcut, RichText, TextEdit, Ui, vec2};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Recorder {
    pub keys: Vec<KeyboardShortcut>,
}

#[derive(Debug, PartialEq)]
pub enum Recorded {
    Pending,
    Accept(Vec<KeyboardShortcut>),
    Cancel,
}

impl Recorder {
    pub fn press(&mut self, shortcut: KeyboardShortcut) -> Recorded {
        let plain = shortcut.modifiers.is_none();
        match shortcut.logical_key {
            Key::Enter if plain && !self.keys.is_empty() => return Recorded::Accept(std::mem::take(&mut self.keys)),
            Key::Escape if plain => return Recorded::Cancel,
            Key::Backspace if plain => self.keys.clear(),
            _ if self.keys.len() == 2 => self.keys = vec![shortcut],
            _ => self.keys.push(shortcut),
        }
        Recorded::Pending
    }

    pub fn text(&self) -> String {
        keys_text(&self.keys)
    }
}

pub fn keys_text(keys: &[KeyboardShortcut]) -> String {
    keys.iter().map(command::key_text).collect::<Vec<_>>().join(" ")
}

fn entry(key: String, command: String, when: &str) -> Entry {
    Entry { key, command, when: when.to_owned(), ..Default::default() }
}

fn unbind(b: &Binding) -> Entry {
    entry(keys_text(&b.keys()), format!("-{}", b.command.id()), &b.when)
}

pub fn change(entries: &mut Vec<Entry>, b: &Binding, keys: &[KeyboardShortcut], when: &str) {
    match b.user.and_then(|i| entries.get_mut(i)) {
        Some(e) => (e.key, e.when) = (keys_text(keys), when.to_owned()),
        None => {
            entries.push(unbind(b));
            entries.push(entry(keys_text(keys), b.command.id(), when));
        }
    }
}

pub fn add(entries: &mut Vec<Entry>, command: &Command, keys: &[KeyboardShortcut]) {
    entries.push(entry(keys_text(keys), command.id(), ""));
}

pub fn remove(entries: &mut Vec<Entry>, b: &Binding) {
    match b.user.filter(|&i| i < entries.len()) {
        Some(i) => drop(entries.remove(i)),
        None => entries.push(unbind(b)),
    }
}

pub fn reset(entries: &mut Vec<Entry>, command: &Command) {
    let id = command.id();
    entries.retain(|e| e.command != id && e.command.strip_prefix('-') != Some(id.as_str()));
}

pub fn matches(b: &Binding, query: &str, pressed: &[KeyboardShortcut]) -> bool {
    if !pressed.is_empty() {
        return b.keys().starts_with(pressed);
    }
    let query = query.to_lowercase();
    let keys = keys_text(&b.keys());
    b.command.label().to_lowercase().contains(&query) || b.command.id().to_lowercase().contains(&query) || keys.contains(&query)
}

pub fn conflicts<'a>(bindings: &'a [Binding], keys: &[KeyboardShortcut]) -> Vec<&'a Binding> {
    match keys.first() {
        Some(first) => bindings.iter().filter(|b| b.shortcut == *first && (keys.len() == 1 || b.chord.is_none() || b.chord == keys.get(1).copied())).collect(),
        None => vec![],
    }
}

pub enum Target {
    Change(Binding),
    Add(Command),
}

#[derive(Default)]
pub struct KeysPage {
    filter: String,
    searching_keys: Option<Recorder>,
    editing: Option<(Target, Recorder)>,
}

impl KeysPage {
    pub fn recording(&self) -> bool {
        self.editing.is_some() || self.searching_keys.is_some()
    }

    pub fn stop(&mut self) {
        (self.editing, self.searching_keys) = (None, None);
    }
}

fn edit_entries(f: impl FnOnce(&mut Vec<Entry>)) {
    match command::entries() {
        Ok(mut entries) => {
            f(&mut entries);
            command::write_entries(&entries);
        }
        Err(error) => crate::ui::notify(format!("keybindings.json could not be read, so it was not changed: {error}")),
    }
}

fn pressed_keys(ctx: &egui::Context) -> Vec<KeyboardShortcut> {
    ctx.input_mut(|i| {
        let keys = i
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some(KeyboardShortcut::new(*modifiers, *key)),
                _ => None,
            })
            .collect();
        i.events.retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_)));
        keys
    })
}

fn when_text(b: &Binding) -> String {
    if b.when.is_empty() { String::new() } else { format!(" when {}", b.when) }
}

pub fn page(ui: &mut Ui, theme: &Theme, page: &mut KeysPage, cmds: &mut Vec<Command>) -> bool {
    let ctx = ui.ctx().clone();
    let bindings = command::bindings();
    if page.recording() {
        for key in pressed_keys(&ctx) {
            if let Some(search) = page.searching_keys.as_mut() {
                match search.press(key) {
                    Recorded::Accept(keys) => (page.filter, page.searching_keys) = (keys_text(&keys), None),
                    Recorded::Cancel => page.searching_keys = None,
                    Recorded::Pending => {}
                }
            } else if let Some((target, recorder)) = page.editing.as_mut() {
                match recorder.press(key) {
                    Recorded::Accept(keys) => {
                        match target {
                            Target::Change(b) => edit_entries(|e| change(e, b, &keys, &b.when)),
                            Target::Add(cmd) => edit_entries(|e| add(e, cmd, &keys)),
                        }
                        page.editing = None;
                    }
                    Recorded::Cancel => page.editing = None,
                    Recorded::Pending => {}
                }
            }
        }
    }
    let mut close = false;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let typing = page.searching_keys.is_none();
        let hint = if typing { "Search by command or keys" } else { "Press keys to search, Escape to stop" };
        let mut shown = match &page.searching_keys {
            Some(r) => r.text(),
            None => page.filter.clone(),
        };
        let field = TextEdit::singleline(&mut shown).margin(crate::ui::FIELD).desired_width(300.0).hint_text(hint).interactive(typing);
        if ui.add(field).changed() && typing {
            page.filter = shown;
        }
        let on = page.searching_keys.is_some();
        if crate::ui::icon_button(ui, theme, icons::KEYBOARD, 26.0, on, "Record Keys: search by pressing the keys").clicked() {
            page.searching_keys = (!on).then(Recorder::default);
            page.editing = None;
        }
        if ui.button("Open keybindings.json").clicked() {
            cmds.push(Command::OpenKeybindings);
            close = true;
        }
    });
    ui.add_space(4.0);
    let about = "Change records one key or a two-key chord: press the keys, Enter accepts, Escape cancels and Backspace clears. Changes are written to keybindings.json.";
    ui.add(egui::Label::new(RichText::new(about).color(theme.text_dim)).wrap());
    if let Some((target, recorder)) = &page.editing {
        let name = match target {
            Target::Change(b) => b.command.label(),
            Target::Add(cmd) => cmd.label(),
        };
        egui::Frame::new().fill(theme.raised).stroke(egui::Stroke::new(1.0, theme.accent)).corner_radius(CornerRadius::same(6)).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("Press the keys for {name}")).color(theme.text));
            let keys = if recorder.keys.is_empty() { "...".to_owned() } else { recorder.text() };
            ui.label(RichText::new(keys).size(18.0).color(theme.accent));
            let same = conflicts(&bindings, &recorder.keys);
            if !same.is_empty() {
                ui.label(RichText::new(format!("{} existing binding(s) use these keys:", same.len())).color(theme.warning));
                for b in same {
                    let text = format!("{}  {}{}", keys_text(&b.keys()), b.command.label(), when_text(b));
                    ui.label(RichText::new(text).color(theme.text_dim));
                }
            }
        });
    }
    ui.add_space(10.0);
    let pressed: Vec<KeyboardShortcut> = page.searching_keys.as_ref().map(|r| r.keys.clone()).unwrap_or_default();
    let query = page.filter.trim().to_owned();
    let mut start = None;
    egui::Grid::new("settings_keys").num_columns(5).spacing(vec2(10.0, 6.0)).striped(true).show(ui, |ui| {
        for label in ["Command", "Keybinding", "When", "Source", ""] {
            ui.label(RichText::new(label).size(12.0).color(theme.text_dim));
        }
        ui.end_row();
        let mut category = "";
        for cmd in command::all() {
            let rows: Vec<&Binding> = bindings.iter().filter(|b| b.command == cmd).collect();
            let unbound = rows.is_empty() && pressed.is_empty() && cmd.label().to_lowercase().contains(&query.to_lowercase());
            let shown: Vec<&Binding> = rows.into_iter().filter(|b| matches(b, &query, &pressed)).collect();
            if (unbound || !shown.is_empty()) && cmd.category() != category {
                category = cmd.category();
                ui.vertical(|ui| crate::ui::section_header(ui, theme, category));
                ui.end_row();
            }
            for b in shown {
                ui.label(RichText::new(cmd.label()).color(theme.text)).on_hover_text(cmd.id());
                ui.label(RichText::new(keys_text(&b.keys())).color(theme.accent).monospace());
                let id = Id::new(("when", cmd.id(), keys_text(&b.keys()), b.user));
                let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| b.when.clone());
                let resp = ui.add(TextEdit::singleline(&mut text).margin(crate::ui::FIELD).desired_width(220.0).hint_text("always"));
                if resp.lost_focus() && text.trim() != b.when {
                    let wanted = text.trim().to_owned();
                    let valid = wanted.is_empty()
                        || crate::when::parse(&wanted).map_err(|e| crate::ui::notify(format!("The when clause \"{wanted}\" is not understood: {e}"))).is_ok();
                    if valid {
                        edit_entries(|e| change(e, b, &b.keys(), &wanted));
                    }
                    ui.data_mut(|d| d.remove::<String>(id));
                } else if resp.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                } else {
                    ui.data_mut(|d| d.remove::<String>(id));
                }
                ui.label(RichText::new(if b.user.is_some() { "User" } else { "Default" }).color(theme.text_dim));
                ui.horizontal(|ui| {
                    if crate::ui::icon_button(ui, theme, icons::PENCIL_SIMPLE, 22.0, false, "Change Keybinding").clicked() {
                        start = Some(Target::Change(b.clone()));
                    }
                    if crate::ui::icon_button(ui, theme, icons::PLUS, 22.0, false, "Add Another Keybinding").clicked() {
                        start = Some(Target::Add(cmd.clone()));
                    }
                    if crate::ui::icon_button(ui, theme, icons::X, 22.0, false, "Remove Keybinding").clicked() {
                        edit_entries(|e| remove(e, b));
                    }
                    if crate::ui::icon_button(ui, theme, icons::ARROW_COUNTER_CLOCKWISE, 22.0, false, "Reset Keybinding").clicked() {
                        edit_entries(|e| reset(e, &cmd));
                    }
                });
                ui.end_row();
            }
            if unbound {
                ui.label(RichText::new(cmd.label()).color(theme.text)).on_hover_text(cmd.id());
                ui.label(RichText::new("Not set").color(theme.text_dim));
                ui.label("");
                ui.label("");
                ui.horizontal(|ui| {
                    if crate::ui::icon_button(ui, theme, icons::PLUS, 22.0, false, "Add Keybinding").clicked() {
                        start = Some(Target::Add(cmd.clone()));
                    }
                    if crate::ui::icon_button(ui, theme, icons::ARROW_COUNTER_CLOCKWISE, 22.0, false, "Reset Keybinding").clicked() {
                        edit_entries(|e| reset(e, &cmd));
                    }
                });
                ui.end_row();
            }
        }
    });
    if let Some(target) = start {
        page.editing = Some((target, Recorder::default()));
        page.searching_keys = None;
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    fn k(m: Modifiers, key: Key) -> KeyboardShortcut {
        KeyboardShortcut::new(m, key)
    }

    #[test]
    fn records_one_key_or_a_chord_and_accepts_cancels_and_clears() {
        let c = Modifiers::COMMAND;
        let mut r = Recorder::default();
        assert_eq!(r.press(k(Modifiers::NONE, Key::Enter)), Recorded::Pending);
        assert_eq!(r.keys, [k(Modifiers::NONE, Key::Enter)]);
        assert_eq!(r.press(k(Modifiers::NONE, Key::Backspace)), Recorded::Pending);
        assert!(r.keys.is_empty());
        r.press(k(c, Key::K));
        r.press(k(c, Key::S));
        assert_eq!(r.text(), "ctrl+k ctrl+s");
        r.press(k(c, Key::B));
        assert_eq!(r.text(), "ctrl+b");
        r.press(k(c, Key::Backspace));
        assert_eq!(r.text(), "ctrl+b ctrl+backspace");
        assert_eq!(r.press(k(Modifiers::NONE, Key::Enter)), Recorded::Accept(vec![k(c, Key::B), k(c, Key::Backspace)]));
        assert!(r.keys.is_empty());
        r.press(k(c, Key::Escape));
        assert_eq!(r.press(k(Modifiers::NONE, Key::Escape)), Recorded::Cancel);
    }

    #[test]
    fn edits_entries_like_vs_code() {
        let defaults = command::bindings();
        let save = defaults.iter().find(|b| b.command == Command::Save).unwrap().clone();
        let mut entries = vec![];
        let alt_s = [k(Modifiers::ALT, Key::S)];
        change(&mut entries, &save, &alt_s, "editorTextFocus");
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].key.as_str(), entries[0].command.as_str(), entries[0].when.as_str()), ("ctrl+s", "-save", command::SEND_TO_SHELL));
        assert_eq!((entries[1].key.as_str(), entries[1].command.as_str(), entries[1].when.as_str()), ("alt+s", "save", "editorTextFocus"));
        let mut user = save.clone();
        user.user = Some(1);
        change(&mut entries, &user, &[k(Modifiers::ALT, Key::D)], "");
        assert_eq!((entries.len(), entries[1].key.as_str(), entries[1].when.as_str()), (2, "alt+d", ""));
        add(&mut entries, &Command::Save, &alt_s);
        remove(&mut entries, &user);
        assert_eq!(entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["ctrl+s", "alt+s"]);
        let find = defaults.iter().find(|b| b.command == Command::Find).unwrap();
        remove(&mut entries, find);
        assert_eq!(entries[2].command, "-find");
        reset(&mut entries, &Command::Save);
        assert_eq!(entries.len(), 1);
        assert!(matches(&save, "SAVE", &[]) && matches(&save, "ctrl+s", &[]) && !matches(&save, "find", &[]));
        assert!(matches(&save, "", &[k(Modifiers::COMMAND, Key::S)]) && !matches(&save, "", &[k(Modifiers::ALT, Key::S)]));
        let on_s = conflicts(&defaults, &[k(Modifiers::COMMAND, Key::S)]);
        assert!(on_s.iter().any(|b| b.command == Command::Save));
        let chord = conflicts(&defaults, &[k(Modifiers::COMMAND, Key::K), k(Modifiers::NONE, Key::S)]);
        assert_eq!(chord.iter().map(|b| b.command.clone()).collect::<Vec<_>>(), [Command::SaveAll]);
    }
}
