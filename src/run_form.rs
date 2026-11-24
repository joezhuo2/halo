use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const LAUNCH: &[(&str, &str, usize)] = &[
    ("Name", "name", 1),
    ("Type, like debugpy, lldb or go", "type", 1),
    ("Request: launch or attach", "request", 1),
    ("Program", "program", 1),
    ("Arguments, one per line", "args", 3),
    ("Working folder", "cwd", 1),
];
const TASK: &[(&str, &str, usize)] = &[
    ("Label", "label", 1),
    ("Type: shell or process", "type", 1),
    ("Command", "command", 1),
    ("Arguments, one per line", "args", 3),
    ("Group: build, test or empty", "group", 1),
];

pub struct Form {
    launch: bool,
    path: PathBuf,
    index: Option<usize>,
    values: Vec<String>,
}

fn spec(launch: bool) -> (&'static [(&'static str, &'static str, usize)], &'static str) {
    if launch { (LAUNCH, "configurations") } else { (TASK, "tasks") }
}

fn path(folder: &Path, launch: bool) -> PathBuf {
    folder.join(".vscode").join(if launch { "launch.json" } else { "tasks.json" })
}

fn file_name(launch: bool) -> &'static str {
    if launch { "launch.json" } else { "tasks.json" }
}

fn read(path: &Path, launch: bool) -> Result<Value, String> {
    let doc = match std::fs::read_to_string(path) {
        Ok(text) => crate::jsonc::parse(&text).map_err(|e| format!("{} could not be read: {e}", file_name(launch)))?,
        Err(_) if launch => json!({"version": "0.2.0", "configurations": []}),
        Err(_) => json!({"version": "2.0.0", "tasks": []}),
    };
    match doc.get(spec(launch).1) {
        None | Some(Value::Array(_)) if doc.is_object() => Ok(doc),
        _ => Err(format!("\"{}\" in {} must be a list.", spec(launch).1, file_name(launch))),
    }
}

fn shown(value: Option<&Value>, key: &str) -> String {
    match value {
        Some(Value::Array(items)) if key == "args" => items.iter().map(|v| v.as_str().map_or(v.to_string(), str::to_owned)).collect::<Vec<_>>().join("\n"),
        Some(Value::Object(group)) if key == "group" => group.get("kind").and_then(Value::as_str).unwrap_or_default().to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn apply(entry: &mut Map<String, Value>, key: &str, text: &str) {
    let text = text.trim();
    match (key, entry.get_mut(key)) {
        _ if text.is_empty() => drop(entry.remove(key)),
        ("args", _) => drop(entry.insert(key.into(), text.lines().map(str::trim).filter(|l| !l.is_empty()).map(Value::from).collect())),
        ("group", Some(Value::Object(group))) => drop(group.insert("kind".into(), json!(text))),
        _ => drop(entry.insert(key.into(), json!(text))),
    }
}

pub fn names(folder: &Path, launch: bool) -> Result<Vec<String>, String> {
    let doc = read(&path(folder, launch), launch)?;
    let name = spec(launch).0[0].1;
    Ok(doc[spec(launch).1].as_array().into_iter().flatten().map(|e| shown(e.get(name), name)).collect())
}

impl Form {
    pub fn open(folder: &Path, launch: bool, index: Option<usize>) -> Result<Self, String> {
        let path = path(folder, launch);
        let doc = read(&path, launch)?;
        let fresh = if launch { json!({"request": "launch", "cwd": "${workspaceFolder}"}) } else { json!({"type": "shell"}) };
        let entry = index.and_then(|i| doc[spec(launch).1].get(i)).unwrap_or(&fresh);
        let values = spec(launch).0.iter().map(|(_, key, _)| shown(entry.get(*key), key)).collect();
        Ok(Self { launch, path, index, values })
    }

    pub fn save(&self) -> Result<(), String> {
        let mut doc = read(&self.path, self.launch)?;
        let (fields, list) = spec(self.launch);
        let list = doc.as_object_mut().unwrap().entry(list).or_insert_with(|| json!([])).as_array_mut().unwrap();
        let entry = match self.index {
            Some(i) => list.get_mut(i).and_then(Value::as_object_mut).ok_or(format!("{} changed on disk; open the form again.", file_name(self.launch)))?,
            None => {
                list.push(json!({}));
                list.last_mut().unwrap().as_object_mut().unwrap()
            }
        };
        for ((_, key, _), text) in fields.iter().zip(&self.values) {
            apply(entry, key, text);
        }
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
        std::fs::create_dir_all(self.path.parent().unwrap())
            .and_then(|_| std::fs::write(&self.path, text))
            .map_err(|e| format!("Could not write {}: {e}", self.path.display()))
    }

    // shortcut: rewriting the file drops its comments, upgrade to an in-place JSONC edit if people keep comments there
    pub fn ui(&mut self, ui: &eframe::egui::Ui) -> bool {
        let (fields, _) = spec(self.launch);
        let heading = match (self.launch, self.index) {
            (true, None) => "New launch configuration",
            (true, Some(_)) => "Edit launch configuration",
            (false, None) => "New task",
            (false, Some(_)) => "Edit task",
        };
        let note = format!("Saved to {}. Other properties are kept; comments are not.", self.path.display());
        let mut rows: Vec<(&str, &mut String, usize)> = fields.iter().zip(self.values.iter_mut()).map(|((hint, _, rows), text)| (*hint, text, *rows)).collect();
        match crate::scm::prompt(ui, heading, &note, &mut rows, "Save") {
            None => false,
            Some(false) => true,
            Some(true) if self.values[0].trim().is_empty() => {
                crate::ui::notify(format!("Give the {} a {}.", if self.launch { "configuration" } else { "task" }, fields[0].0.to_lowercase()));
                false
            }
            Some(true) => match self.save() {
                Ok(()) => true,
                Err(error) => {
                    crate::ui::notify(error);
                    false
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_add_and_edit_entries_keeping_other_properties() {
        let folder = std::env::temp_dir().join(format!("halo_run_form_{}", std::process::id()));
        let mut form = Form::open(&folder, true, None).unwrap();
        form.values = ["Run", "lldb", "launch", "a.exe", "-v\n\n--x", ""].map(String::from).to_vec();
        form.save().unwrap();
        assert_eq!(names(&folder, true).unwrap(), ["Run"]);

        std::fs::write(path(&folder, false), r#"{ // tasks
            "version": "2.0.0", "tasks": [{"label": "b", "command": "make", "group": {"kind": "build", "isDefault": true}, "env": 1}] }"#).unwrap();
        let mut form = Form::open(&folder, false, Some(0)).unwrap();
        assert_eq!(form.values, ["b", "", "make", "", "build"]);
        form.values[2] = "cargo".into();
        form.values[3] = "build".into();
        form.values[4] = "test".into();
        form.save().unwrap();
        let doc = read(&path(&folder, false), false).unwrap();
        assert_eq!(doc["tasks"][0], json!({"label": "b", "command": "cargo", "args": ["build"], "group": {"kind": "test", "isDefault": true}, "env": 1}));
        let launch = read(&path(&folder, true), true).unwrap();
        assert_eq!(launch["configurations"][0]["args"], json!(["-v", "--x"]));
        assert!(launch["configurations"][0].get("cwd").is_none());
        let _ = std::fs::remove_dir_all(folder);
    }
}
