use crate::command::{self, Binding, Command, Entry};
use eframe::egui::{Key, KeyboardShortcut};

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
