use crate::settings::{Settings, font_files};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(windows)]
const OS: &str = "windows";
#[cfg(target_os = "macos")]
const OS: &str = "osx";
#[cfg(not(any(windows, target_os = "macos")))]
const OS: &str = "linux";

pub const VSCODE: &[(&str, &str)] = &[
    ("theme", "workbench.colorTheme"),
    ("font_family", "editor.fontFamily"),
    ("font_size", "editor.fontSize"),
    ("tab_size", "editor.tabSize"),
    ("insert_spaces", "editor.insertSpaces"),
    ("minimap", "editor.minimap.enabled"),
    ("auto_save", "files.autoSave"),
    ("associations", "files.associations"),
    ("code_lens", "editor.codeLens"),
    ("inlay_hints", "editor.inlayHints.enabled"),
    ("semantic_highlighting", "editor.semanticHighlighting.enabled"),
    ("breadcrumbs", "breadcrumbs.enabled"),
    ("inline_blame", "git.blame.editorDecoration.enabled"),
    ("local_history", "workbench.localHistory.enabled"),
    ("format_on_save", "editor.formatOnSave"),
    ("format_on_paste", "editor.formatOnPaste"),
    ("word_wrap", "editor.wordWrap"),
    ("auto_close", "editor.autoClosingBrackets"),
    ("auto_close_quotes", "editor.autoClosingQuotes"),
    ("auto_surround", "editor.autoSurround"),
    ("auto_indent", "editor.autoIndent"),
    ("trim_final_newlines", "files.trimFinalNewlines"),
    ("eol", "files.eol"),
    ("auto_save_delay", "files.autoSaveDelay"),
    ("trim_trailing_whitespace", "files.trimTrailingWhitespace"),
    ("insert_final_newline", "files.insertFinalNewline"),
    ("render_whitespace", "editor.renderWhitespace"),
    ("rulers", "editor.rulers"),
    ("sticky_scroll", "editor.stickyScroll.enabled"),
    ("bracket_colors", "editor.bracketPairColorization.enabled"),
    ("guides", "editor.guides.indentation"),
    ("word_wrap_column", "editor.wordWrapColumn"),
    ("line_numbers", "editor.lineNumbers"),
    ("cursor_style", "editor.cursorStyle"),
    ("cursor_blinking", "editor.cursorBlinking"),
    ("line_height", "editor.lineHeight"),
    ("render_line_highlight", "editor.renderLineHighlight"),
    ("scroll_beyond_last_line", "editor.scrollBeyondLastLine"),
    ("smooth_scrolling", "editor.smoothScrolling"),
    ("cursor_surrounding_lines", "editor.cursorSurroundingLines"),
    ("mouse_wheel_zoom", "editor.mouseWheelZoom"),
    ("multi_cursor_modifier", "editor.multiCursorModifier"),
    ("word_separators", "editor.wordSeparators"),
    ("match_brackets", "editor.matchBrackets"),
    ("selection_highlight", "editor.selectionHighlight"),
    ("minimap_side", "editor.minimap.side"),
    ("detect_indentation", "editor.detectIndentation"),
    ("hot_exit", "files.hotExit"),
    ("tab_close_button", "workbench.editor.tabCloseButton"),
    ("enable_preview", "workbench.editor.enablePreview"),
    ("confirm_delete", "explorer.confirmDelete"),
    ("explorer_sort", "explorer.sortOrder"),
    ("zoom_level", "window.zoomLevel"),
    ("restore_session", "window.restoreWindows"),
    ("auto_update", "update.mode"),
    ("terminal_font_family", "terminal.integrated.fontFamily"),
    ("terminal_font_size", "terminal.integrated.fontSize"),
    ("terminal_line_height", "terminal.integrated.lineHeight"),
    ("terminal_cursor_style", "terminal.integrated.cursorStyle"),
    ("terminal_cursor_blinking", "terminal.integrated.cursorBlinking"),
    ("terminal_scrollback", "terminal.integrated.scrollback"),
    ("terminal_copy_on_selection", "terminal.integrated.copyOnSelection"),
    ("terminal_right_click", "terminal.integrated.rightClickBehavior"),
];

pub fn vscode_key(key: &str) -> Option<String> {
    match key {
        "default_terminal" => Some(format!("terminal.integrated.defaultProfile.{OS}")),
        "terminals" => Some(format!("terminal.integrated.profiles.{OS}")),
        _ => VSCODE.iter().find(|(halo, _)| *halo == key).map(|(_, code)| code.to_string()),
    }
}

pub fn halo_key(code: &str) -> Option<&'static str> {
    match code.strip_prefix("terminal.integrated.") {
        Some(rest) if rest == format!("defaultProfile.{OS}") => Some("default_terminal"),
        Some(rest) if rest == format!("profiles.{OS}") => Some("terminals"),
        _ => VSCODE.iter().find(|(_, c)| *c == code).map(|(halo, _)| *halo),
    }
}

fn font_path(name: &str) -> Value {
    if name.is_empty() || name.contains(['/', '\\']) {
        return json!(name);
    }
    let fonts = font_files();
    let found = name.split(',').map(|n| n.trim().trim_matches(['\'', '"'])).find_map(|n| fonts.iter().find(|f| f.0.eq_ignore_ascii_case(n)));
    json!(found.map(|f| f.1.clone()).unwrap_or_default())
}

pub fn to_halo(key: &str, value: Value) -> Value {
    match (key, value) {
        ("word_wrap", Value::Bool(b)) => json!(if b { "on" } else { "off" }),
        ("render_whitespace", Value::Bool(b)) => json!(if b { "all" } else { "none" }),
        ("auto_save", Value::Bool(b)) => json!(if b { "afterDelay" } else { "off" }),
        ("auto_close" | "auto_close_quotes" | "auto_surround", Value::String(s)) => json!(s != "never"),
        ("hot_exit", Value::String(s)) => json!(s != "off"),
        ("restore_session", Value::String(s)) => json!(s != "none"),
        ("auto_update", Value::String(s)) => json!(!matches!(s.as_str(), "none" | "manual")),
        ("inlay_hints", Value::String(s)) => json!(s.starts_with("on")),
        ("semantic_highlighting", Value::String(_)) => json!(true),
        ("rulers", Value::Array(list)) => list.into_iter().map(|r| r.get("column").cloned().unwrap_or(r)).collect(),
        ("associations", Value::Object(map)) => Value::Object(map.into_iter().map(|(k, v)| (k.trim_start_matches("*.").to_owned(), v)).collect()),
        ("font_family" | "terminal_font_family", Value::String(s)) => font_path(&s),
        ("terminals", Value::Object(map)) => map
            .into_iter()
            .map(|(name, p)| {
                let program = p.get("path").map(|p| p.as_array().and_then(|a| a.first()).unwrap_or(p).clone()).unwrap_or(json!(""));
                json!({"name": name, "program": program, "args": p.get("args").cloned().unwrap_or(json!([]))})
            })
            .collect(),
        (_, value) => value,
    }
}

pub fn to_vscode(key: &str, value: Value) -> Value {
    let on = |v: &Value, yes: &str, no: &str| json!(if v.as_bool().unwrap_or(false) { yes } else { no });
    match key {
        "auto_close" | "auto_close_quotes" | "auto_surround" => on(&value, "languageDefined", "never"),
        "hot_exit" => on(&value, "onExit", "off"),
        "restore_session" => on(&value, "all", "none"),
        "auto_update" => on(&value, "default", "none"),
        "inlay_hints" => on(&value, "on", "off"),
        "associations" => match value {
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (format!("*.{k}"), v)).collect()),
            other => other,
        },
        "font_family" | "terminal_font_family" => match value.as_str().and_then(|path| font_files().into_iter().find(|f| f.1 == path)) {
            Some((name, _)) => json!(name),
            None => value,
        },
        "terminals" => match value {
            Value::Array(list) => Value::Object(
                list.iter().map(|p| (p["name"].as_str().unwrap_or_default().to_owned(), json!({"path": p["program"], "args": p["args"]}))).collect(),
            ),
            other => other,
        },
        _ => value,
    }
}

pub fn split(raw: &Map<String, Value>) -> (Map<String, Value>, BTreeMap<String, Map<String, Value>>) {
    let defaults = defaults();
    let (mut plain, mut langs) = (Map::new(), BTreeMap::<String, Map<String, Value>>::new());
    for (key, value) in raw {
        if let Some(ids) = key.strip_prefix('[').and_then(|k| k.strip_suffix(']')) {
            let inner = value.as_object().map(|o| split(o).0).unwrap_or_default();
            for id in ids.split("][") {
                langs.entry(id.trim().to_owned()).or_default().extend(inner.clone());
            }
        } else if let Some(halo) = halo_key(key) {
            plain.entry(halo.to_owned()).or_insert_with(|| to_halo(halo, value.clone()));
        } else if defaults.contains_key(key) {
            plain.insert(key.clone(), to_halo(key, value.clone()));
        }
    }
    (plain, langs)
}

fn defaults() -> Map<String, Value> {
    match serde_json::to_value(Settings::default()) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn same_kind(default: &Value, value: &Value) -> bool {
    match (default, value) {
        (Value::Number(d), Value::Number(v)) => d.is_f64() || v.is_u64(),
        (d, v) => std::mem::discriminant(d) == std::mem::discriminant(v),
    }
}

pub fn valid(map: Map<String, Value>, origin: &str, errors: &mut Vec<String>) -> Map<String, Value> {
    let defaults = defaults();
    let deep = |key: &String, value: &Value| {
        !(value.is_array() || value.is_object()) || serde_json::from_value::<Settings>(Value::Object(Map::from_iter([(key.clone(), value.clone())]))).is_ok()
    };
    let (good, bad): (Map<String, Value>, Map<String, Value>) =
        map.into_iter().partition(|(k, v)| defaults.get(k).is_some_and(|d| same_kind(d, v)) && deep(k, v));
    for key in bad.keys() {
        let name = vscode_key(key).unwrap_or_else(|| key.clone());
        errors.push(format!("{origin}: the value of \"{name}\" is not valid, so it is not used."));
    }
    good
}

fn build(map: Map<String, Value>) -> Settings {
    serde_json::from_value(Value::Object(map)).unwrap_or_default()
}

type Langs = BTreeMap<String, Map<String, Value>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    User,
    Workspace,
}

#[derive(Default)]
struct Layers {
    folder: Option<PathBuf>,
    raw: [Option<Map<String, Value>>; 2],
    plain: [Map<String, Value>; 2],
    langs: [Langs; 2],
    effective: Map<String, Value>,
    scoped: BTreeMap<(usize, String), Settings>,
}

static LAYERS: Mutex<Option<Layers>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Layers) -> R) -> R {
    f(LAYERS.lock().unwrap().get_or_insert_with(Default::default))
}

pub fn workspace_path() -> Option<PathBuf> {
    with(|l| l.folder.as_ref().map(|f| f.join(".vscode").join("settings.json")))
}

pub fn set_folder(folder: Option<&Path>) {
    with(|l| l.folder = folder.map(Path::to_path_buf));
}

fn read_file(path: &Path, errors: &mut Vec<String>) -> Option<Map<String, Value>> {
    let Ok(text) = std::fs::read_to_string(path) else { return Some(Map::new()) };
    match crate::jsonc::parse(&text) {
        Ok(Value::Object(map)) => Some(map),
        Ok(_) => {
            errors.push(format!("{} must hold an object, so it is not used and Halo will not change it.", path.display()));
            None
        }
        Err(e) => {
            errors.push(format!("{} could not be read, so it is not used and Halo will not change it: {e}", path.display()));
            None
        }
    }
}

fn layered(plain: &[Map<String, Value>], langs: &[Langs], lang: Option<&str>) -> Map<String, Value> {
    let mut map = defaults();
    plain.iter().for_each(|p| map.extend(p.clone()));
    for block in lang.map(|id| langs.iter().filter_map(move |l| l.get(id))).into_iter().flatten() {
        map.extend(block.clone());
    }
    map
}

pub fn resolve(user: &Map<String, Value>, workspace: &Map<String, Value>, errors: &mut Vec<String>) -> (Settings, [Map<String, Value>; 2], [Langs; 2]) {
    let mut plain = [Map::new(), Map::new()];
    let mut langs = [Langs::new(), Langs::new()];
    for (index, (raw, origin)) in [(user, "settings.json"), (workspace, ".vscode/settings.json")].into_iter().enumerate() {
        let (p, l) = split(raw);
        plain[index] = valid(p, origin, errors);
        langs[index] = l.into_iter().map(|(id, m)| (id.clone(), valid(m, &format!("{origin} [{id}]"), errors))).collect();
    }
    let mut settings = build(layered(&plain, &langs, None));
    for id in langs.iter().flat_map(|l| l.keys()) {
        settings.langs.insert(id.clone(), build(layered(&plain, &langs, Some(id))));
    }
    (settings, plain, langs)
}

pub fn load() -> Settings {
    let path = Settings::path();
    if !path.exists() {
        crate::settings::write(&path, &Settings::default());
    }
    let mut errors = vec![];
    let user = read_file(&path, &mut errors);
    let workspace = workspace_path().and_then(|p| read_file(&p, &mut errors));
    let (settings, plain, langs) = resolve(user.as_ref().unwrap_or(&Map::new()), workspace.as_ref().unwrap_or(&Map::new()), &mut errors);
    errors.into_iter().for_each(crate::ui::notify);
    with(|l| {
        l.effective = serde_json::to_value(&settings).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
        (l.raw, l.plain, l.langs, l.scoped) = ([user, workspace], plain, langs, BTreeMap::new());
    });
    settings
}

pub fn changes(before: &Map<String, Value>, after: &Settings) -> Vec<(String, Option<Value>)> {
    let Ok(Value::Object(after)) = serde_json::to_value(after) else { return vec![] };
    after.into_iter().filter(|(k, v)| before.get(k) != Some(v)).map(|(k, v)| (k, Some(v))).collect()
}

pub fn save(settings: &Settings) -> Settings {
    let edits = with(|l| changes(&l.effective, settings));
    write(Layer::User, None, &edits);
    load()
}

pub fn apply(raw: &mut Map<String, Value>, layer: Layer, lang: Option<&str>, edits: &[(String, Option<Value>)]) {
    let block = lang.map(|id| format!("[{id}]"));
    let target = match &block {
        Some(name) => {
            let entry = raw.entry(name.clone()).or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            entry.as_object_mut().unwrap()
        }
        None => &mut *raw,
    };
    for (key, value) in edits {
        let code = vscode_key(key);
        let names: Vec<String> = [Some(key.clone()), code.clone()].into_iter().flatten().collect();
        let existing = names.iter().find(|n| target.contains_key(*n)).cloned();
        let Some(value) = value else {
            names.iter().for_each(|n| drop(target.remove(n)));
            continue;
        };
        let name = existing.unwrap_or_else(|| if layer == Layer::Workspace { code.clone().unwrap_or(key.clone()) } else { key.clone() });
        let value = if Some(&name) == code.as_ref() { to_vscode(key, value.clone()) } else { value.clone() };
        target.insert(name, value);
    }
    if let Some(name) = block
        && raw.get(&name).and_then(Value::as_object).is_some_and(Map::is_empty)
    {
        raw.remove(&name);
    }
}

pub fn write(layer: Layer, lang: Option<&str>, edits: &[(String, Option<Value>)]) {
    if edits.is_empty() {
        return;
    }
    let path = match layer {
        Layer::User => Some(Settings::path()),
        Layer::Workspace => workspace_path(),
    };
    let Some(path) = path else { return crate::ui::notify("Open a folder to change workspace settings.".to_owned()) };
    let Some(mut raw) = with(|l| l.raw[layer as usize].clone()) else {
        return crate::ui::notify(format!("{} could not be read, so it was not changed.", path.display()));
    };
    apply(&mut raw, layer, lang, edits);
    crate::settings::write(&path, &raw);
    with(|l| l.raw[layer as usize] = Some(raw));
}

pub fn scoped(layer: Layer, lang: Option<&str>) -> Settings {
    with(|l| {
        let upto = layer as usize + 1;
        let key = (upto, lang.unwrap_or_default().to_owned());
        if !l.scoped.contains_key(&key) {
            let settings = build(layered(&l.plain[..upto], &l.langs[..upto], lang));
            l.scoped.insert(key.clone(), settings);
        }
        l.scoped[&key].clone()
    })
}

pub fn sets(layer: Layer, lang: Option<&str>, key: &str) -> bool {
    with(|l| match lang {
        Some(id) => l.langs[layer as usize].get(id).is_some_and(|m| m.contains_key(key)),
        None => l.plain[layer as usize].contains_key(key),
    })
}

pub fn languages() -> Vec<String> {
    with(|l| l.langs.iter().flat_map(|m| m.keys().cloned()).collect::<std::collections::BTreeSet<_>>().into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn layers_apply_in_order_with_language_blocks_from_both_files() {
        let user = obj(json!({
            "tab_size": 3, "editor.fontSize": 15, "editor.insertSpaces": false, "unknown.key": 1,
            "[rust]": {"editor.tabSize": 8, "rulers": [100]}, "[markdown][plaintext]": {"word_wrap": true},
        }));
        let workspace = obj(json!({
            "editor.tabSize": 2, "editor.rulers": [{"column": 80}], "files.autoSave": "afterDelay", "editor.insertSpaces": "no",
            "[rust]": {"editor.rulers": [120]}, "[python]": {"editor.tabSize": 4},
        }));
        let mut errors = vec![];
        let (s, _, langs) = resolve(&user, &workspace, &mut errors);
        assert_eq!((s.tab_size, s.font_size, s.insert_spaces, s.auto_save.as_str()), (2, 15.0, false, "afterDelay"));
        assert_eq!(s.rulers, [80]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("editor.insertSpaces"));
        let rust = s.lang("rust");
        assert_eq!((rust.tab_size, rust.rulers.clone(), rust.auto_save.as_str()), (8, vec![120], "afterDelay"));
        assert_eq!(s.lang("python").tab_size, 4);
        assert_eq!((s.lang("markdown").word_wrap.as_str(), s.lang("plaintext").word_wrap.as_str(), s.word_wrap.as_str()), ("on", "on", "off"));
        assert_eq!(s.lang("go").tab_size, 2);
        assert_eq!(langs[0]["rust"]["tab_size"], json!(8));
    }

    #[test]
    fn maps_vs_code_keys_both_ways() {
        let defaults = defaults();
        for (halo, code) in VSCODE {
            assert_eq!(halo_key(code), Some(*halo));
            assert_eq!(vscode_key(halo).as_deref(), Some(*code));
            let value = defaults[*halo].clone();
            assert_eq!(to_halo(halo, to_vscode(halo, value.clone())), value, "{halo}");
            for flipped in [json!(true), json!(false)].into_iter().filter(|_| value.is_boolean()) {
                assert_eq!(to_halo(halo, to_vscode(halo, flipped.clone())), flipped, "{halo}");
            }
        }
        assert_eq!(halo_key(&format!("terminal.integrated.defaultProfile.{OS}")), Some("default_terminal"));
        let profiles = json!([{"name": "a", "program": "a.exe", "args": ["-x"]}, {"name": "b", "program": "b", "args": []}]);
        assert_eq!(to_halo("terminals", to_vscode("terminals", profiles.clone())), profiles);
        assert_eq!(to_vscode("associations", json!({"h": "cpp"})), json!({"*.h": "cpp"}));
        assert_eq!(to_halo("word_wrap", json!("bounded")), json!("bounded"));
        assert_eq!(to_halo("word_wrap", json!(true)), json!("on"));
        assert_eq!(to_halo("render_whitespace", json!(false)), json!("none"));
        assert_eq!(to_halo("auto_close", json!("never")), json!(false));
        assert_eq!(to_halo("auto_save", json!(true)), json!("afterDelay"));
        assert_eq!(to_halo("eol", json!("\r\n")), json!("\r\n"));
        assert_eq!(to_halo("font_family", json!("C:/fonts/x.ttf")), json!("C:/fonts/x.ttf"));
        assert_eq!(to_halo("font_family", json!("'No Such Font', monospace")), json!(""));
    }

    #[test]
    fn writes_keep_unknown_keys_and_pick_the_existing_name() {
        let mut raw = obj(json!({"unknown": {"x": 1}, "editor.tabSize": 2, "minimap": true, "[rust]": {"tab_size": 8}}));
        let edits = [("tab_size".to_owned(), Some(json!(3))), ("minimap".to_owned(), None), ("word_wrap".to_owned(), Some(json!(true)))];
        apply(&mut raw, Layer::User, None, &edits);
        assert_eq!(Value::Object(raw.clone()), json!({"unknown": {"x": 1}, "editor.tabSize": 3, "word_wrap": true, "[rust]": {"tab_size": 8}}));
        apply(&mut raw, Layer::Workspace, None, &[("auto_save".to_owned(), Some(json!("afterDelay")))]);
        assert_eq!(raw["files.autoSave"], json!("afterDelay"));
        apply(&mut raw, Layer::Workspace, Some("rust"), &[("tab_size".to_owned(), None)]);
        assert!(!raw.contains_key("[rust]"));
        apply(&mut raw, Layer::Workspace, Some("go"), &[("tab_size".to_owned(), Some(json!(4)))]);
        assert_eq!(raw["[go]"], json!({"editor.tabSize": 4}));
        let mut before = defaults();
        before.insert("tab_size".into(), json!(9));
        let after = Settings { tab_size: 4, minimap: false, ..Settings::default() };
        let mut found = changes(&before, &after);
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(found, [("minimap".to_owned(), Some(json!(false))), ("tab_size".to_owned(), Some(json!(4)))]);
    }
}
