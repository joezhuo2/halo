use crate::app::Halo;
use crate::command::Command;
use crate::search::Choice;
use crate::settings;
use crate::theme::{Theme, Tokens};
use crate::ui;
use eframe::egui::{self, Color32};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc::{Receiver, Sender, channel};
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, ThemeItem};
use syntect::parsing::ScopeStack;

pub enum Done {
    Browse(&'static str, Vec<Choice>),
    Imported(bool, Result<String, String>),
}

pub struct Jobs {
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}

pub fn jsonc(text: &str) -> Result<Value, String> {
    let chars: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.get(i + 1) {
                    out.push(*next);
                    i += 1;
                }
            } else if c == '"' {
                string = false;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            out.push(' ');
            continue;
        } else {
            if matches!(c, '}' | ']') && out.trim_end().ends_with(',') {
                out.truncate(out.trim_end().len() - 1);
            }
            string = c == '"';
            out.push(c);
        }
        i += 1;
    }
    serde_json::from_str(&out).map_err(|e| e.to_string())
}

pub fn color(text: &str) -> Option<Color32> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let part = |at: usize, len: usize| u8::from_str_radix(&hex[at..at + len], 16).ok();
    let [r, g, b, a] = match hex.len() {
        3 | 4 => {
            let d = |i| part(i, 1).map(|v| v * 17);
            [d(0)?, d(1)?, d(2)?, if hex.len() == 4 { d(3)? } else { 255 }]
        }
        6 | 8 => [part(0, 2)?, part(2, 2)?, part(4, 2)?, if hex.len() == 8 { part(6, 2)? } else { 255 }],
        _ => return None,
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn over(c: Color32, under: Color32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { c } else { ui::mix(under, Color32::from_rgb(r, g, b), f32::from(a) / 255.0) }
}

pub fn read_json(path: &Path) -> Result<Value, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    jsonc(&text).map_err(|e| {
        let hint = if name.to_lowercase().ends_with(".tmtheme") { " (.tmTheme files are not supported)" } else { "" };
        format!("{name} is not valid JSON{hint}: {e}")
    })
}

fn merge(mut base: Value, child: Value) -> Value {
    let (Some(target), Value::Object(child)) = (base.as_object_mut(), child) else { return Value::Null };
    for (key, value) in child {
        match (target.get_mut(&key), value) {
            _ if key == "include" => {}
            (Some(Value::Object(old)), Value::Object(new)) if key == "colors" || key == "semanticTokenColors" => old.extend(new),
            (Some(Value::Array(old)), Value::Array(new)) if key == "tokenColors" => old.extend(new),
            (_, value) => {
                target.insert(key, value);
            }
        }
    }
    base
}

pub fn resolve(path: &Path) -> Result<Value, String> {
    fn at(path: &Path, depth: usize) -> Result<Value, String> {
        if depth > 8 {
            return Err("the include chain is too deep".into());
        }
        let theme = read_json(path)?;
        let Some(include) = theme.get("include").and_then(Value::as_str) else {
            return if theme.is_object() { Ok(theme) } else { Err("the file is not a theme object".into()) };
        };
        let base = at(&path.parent().unwrap_or(Path::new(".")).join(include), depth + 1)?;
        Ok(merge(base, theme))
    }
    at(path, 0)
}

fn tokens(v: &Value) -> Result<(Vec<ThemeItem>, Option<Color32>), String> {
    let rules = match v.get("tokenColors") {
        None | Some(Value::Null) => return Ok((vec![], None)),
        Some(Value::String(path)) => {
            return Err(format!("tokenColors points to the file {path}; .tmTheme token files are not supported, inline the rules in the theme"));
        }
        Some(Value::Array(rules)) => rules,
        Some(_) => return Err("tokenColors must be a list".into()),
    };
    let (mut items, mut default) = (vec![], None);
    for rule in rules {
        let Some(settings) = rule.get("settings") else { continue };
        let foreground = settings.get("foreground").and_then(Value::as_str).and_then(color);
        let font_style = settings.get("fontStyle").and_then(Value::as_str).map(|text| {
            text.split_whitespace().fold(FontStyle::empty(), |style, word| match word {
                "bold" => style | FontStyle::BOLD,
                "italic" => style | FontStyle::ITALIC,
                "underline" => style | FontStyle::UNDERLINE,
                _ => style,
            })
        });
        let scope = match rule.get("scope") {
            Some(Value::String(scope)) => scope.clone(),
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            _ => {
                default = default.or(foreground);
                continue;
            }
        };
        let Ok(scope) = ScopeSelectors::from_str(&scope) else { continue };
        if foreground.is_some() || font_style.is_some() {
            let foreground = foreground.map(|c| Color { r: c.r(), g: c.g(), b: c.b(), a: 255 });
            items.push(ThemeItem { scope, style: StyleModifier { foreground, background: None, font_style } });
        }
    }
    Ok((items, default))
}

fn scoped(rules: &[ThemeItem], probes: &[&str]) -> Option<Color32> {
    probes.iter().find_map(|probe| {
        let stack = ScopeStack::from_str(probe).ok()?;
        let mut best = None;
        for item in rules {
            let (Some(c), Some(power)) = (item.style.foreground, item.scope.does_match(stack.as_slice())) else { continue };
            if best.as_ref().is_none_or(|(b, _)| power >= *b) {
                best = Some((power, Color32::from_rgb(c.r, c.g, c.b)));
            }
        }
        best.map(|(_, c)| c)
    })
}

fn semantic(v: &Value, names: &[&str]) -> Option<Color32> {
    let map = v.get("semanticTokenColors")?.as_object()?;
    names.iter().find_map(|name| {
        let value = map.get(*name)?;
        color(value.as_str().or_else(|| value.get("foreground")?.as_str())?)
    })
}

pub fn build(v: &Value) -> Result<Theme, String> {
    let kind = v.get("type").or_else(|| v.get("uiTheme")).and_then(Value::as_str).unwrap_or("dark").to_lowercase();
    let mut theme = if kind.contains("light") { Theme::light() } else { Theme::dark() };
    let (rules, default) = tokens(v)?;
    let colors = v.get("colors").and_then(Value::as_object);
    let get = |keys: &[&str]| keys.iter().find_map(|key| color(colors?.get(*key)?.as_str()?));
    if let Some(bg) = get(&["editor.background"]) {
        theme.bg = over(bg, theme.bg);
    }
    let bg = theme.bg;
    let set = |slot: &mut Color32, keys: &[&str]| {
        if let Some(c) = get(keys) {
            *slot = over(c, bg);
        }
    };
    set(&mut theme.panel, &["sideBar.background", "panel.background", "editorGroup.background"]);
    set(&mut theme.bar, &["activityBar.background", "statusBar.background", "titleBar.activeBackground"]);
    set(&mut theme.raised, &["editorWidget.background", "quickInput.background", "menu.background", "input.background", "dropdown.background"]);
    set(&mut theme.hover, &["list.hoverBackground", "list.inactiveSelectionBackground", "toolbar.hoverBackground", "editor.lineHighlightBackground"]);
    set(&mut theme.border, &["panel.border", "editorGroup.border", "sideBar.border", "tab.border", "contrastBorder", "input.border"]);
    set(&mut theme.text, &["editor.foreground", "foreground"]);
    if get(&["editor.foreground", "foreground"]).is_none() {
        theme.text = default.unwrap_or(theme.text);
    }
    set(&mut theme.text_dim, &["descriptionForeground", "editorLineNumber.foreground", "tab.inactiveForeground", "sideBarTitle.foreground"]);
    set(&mut theme.accent, &["focusBorder", "button.background", "activityBarBadge.background", "textLink.foreground"]);
    set(&mut theme.selection, &["editor.selectionBackground", "list.activeSelectionBackground", "selection.background"]);
    set(&mut theme.added, &["gitDecoration.addedResourceForeground", "editorGutter.addedBackground"]);
    set(&mut theme.modified, &["gitDecoration.modifiedResourceForeground", "editorGutter.modifiedBackground"]);
    set(&mut theme.deleted, &["gitDecoration.deletedResourceForeground", "editorGutter.deletedBackground"]);
    set(&mut theme.error, &["editorError.foreground", "errorForeground", "list.errorForeground"]);
    set(&mut theme.warning, &["editorWarning.foreground", "list.warningForeground"]);
    const NAMES: [&str; 8] = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
    for (i, slot) in theme.ansi.iter_mut().enumerate() {
        let key = format!("terminal.ansi{}{}", if i < 8 { "" } else { "Bright" }, NAMES[i % 8]);
        set(slot, &[&key]);
    }
    let s = &mut theme.syntax;
    for (slot, probes) in [
        (&mut s.comment, &["comment.line", "comment"][..]),
        (&mut s.string, &["string.quoted", "string"]),
        (&mut s.keyword, &["keyword.control", "keyword", "storage.type"]),
        (&mut s.number, &["constant.numeric"]),
        (&mut s.function, &["entity.name.function", "support.function"]),
        (&mut s.type_name, &["entity.name.type", "entity.name.class", "support.type", "support.class"]),
        (&mut s.constant, &["constant.language", "constant.other", "variable.other.constant"]),
        (&mut s.punctuation, &["punctuation.separator", "punctuation"]),
    ] {
        if let Some(c) = scoped(&rules, probes) {
            *slot = c;
        }
    }
    theme.semantic = [
        semantic(v, &["type", "class", "struct", "enum", "interface", "namespace", "typeParameter"]),
        semantic(v, &["function", "method", "macro"]),
        semantic(v, &["enumMember", "variable.readonly", "constant"]),
    ];
    theme.dark = !kind.contains("light");
    theme.tokens = Tokens::new(rules);
    Ok(theme)
}

pub fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.".contains(c) { c } else { '_' }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    match cleaned.to_lowercase().as_str() {
        "" => "theme".into(),
        "dark" | "light" => format!("{cleaned} imported"),
        _ => cleaned,
    }
}

fn themes_dir(config: &Path) -> PathBuf {
    config.join("themes")
}

pub fn load(config: &Path, name: &str) -> Result<Theme, String> {
    if safe_name(name) != name {
        return Err("the name is not valid".into());
    }
    build(&resolve(&themes_dir(config).join(format!("{name}.json")))?)
}

pub fn installed(config: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(themes_dir(config))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|_| e.path().extension().is_some_and(|x| x == "json")))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

pub fn import(source: &Path, config: &Path) -> Result<String, String> {
    let resolved = resolve(source)?;
    build(&resolved)?;
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = safe_name(&stem);
    let dir = themes_dir(config);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&resolved).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.json")), text).map_err(|e| format!("Could not save the theme: {e}"))?;
    Ok(name)
}

pub fn remove(config: &Path, name: &str) {
    if safe_name(name) == name {
        let _ = std::fs::remove_file(themes_dir(config).join(format!("{name}.json")));
    }
}

pub fn contributions(root: &Path, key: &str) -> Vec<(String, String, PathBuf)> {
    let mut out = vec![];
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let dir = entry.path();
        let Ok(package) = read_json(&dir.join("package.json")) else { continue };
        let nls = read_json(&dir.join("package.nls.json")).ok();
        let text = |v: &Value| {
            let raw = v.as_str().unwrap_or_default();
            match raw.strip_prefix('%').and_then(|r| r.strip_suffix('%')) {
                Some(id) => nls.as_ref().and_then(|n| n.get(id)).and_then(|t| t.as_str().or_else(|| t.get("message")?.as_str())).unwrap_or(raw).to_owned(),
                None => raw.to_owned(),
            }
        };
        let extension = entry.file_name().to_string_lossy().into_owned();
        for item in package.pointer(&format!("/contributes/{key}")).and_then(Value::as_array).into_iter().flatten() {
            let Some(path) = item.get("path").and_then(Value::as_str) else { continue };
            let label = item.get("label").or_else(|| item.get("id")).map(&text).unwrap_or_default();
            out.push((label, extension.clone(), dir.join(path)));
        }
    }
    out.sort_by_key(|(label, ..)| label.to_lowercase());
    out
}

pub fn extensions_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".vscode").join("extensions")
}

fn pick_json() -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("JSON", &["json", "jsonc"]).pick_file()
}

impl Halo {
    pub(crate) fn poll_imports(&mut self) {
        let done: Vec<Done> = self.jobs.rx.try_iter().collect();
        for job in done {
            match job {
                Done::Browse(hint, items) if items.is_empty() => ui::notify(format!("{hint}: nothing was found in {}.", extensions_dir().display())),
                Done::Browse(hint, items) => self.search.pick(hint, items),
                Done::Imported(icons, Ok(name)) => {
                    self.page.themes = None;
                    self.cmds.push(if icons { Command::SetIconTheme(name) } else { Command::SetTheme(name) });
                }
                Done::Imported(_, Err(error)) => ui::notify(error),
            }
        }
    }

    pub(crate) fn spawn(&self, ctx: &egui::Context, work: impl FnOnce() -> Done + Send + 'static) {
        let (tx, ctx) = (self.jobs.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }

    pub(crate) fn theme_command(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        let config = settings::dir();
        match cmd {
            ImportTheme => {
                if let Some(file) = pick_json() {
                    self.spawn(ctx, move || Done::Imported(false, import(&file, &settings::dir())));
                }
            }
            ImportThemeFile(file) => self.spawn(ctx, move || Done::Imported(false, import(&file, &settings::dir()))),
            BrowseThemes => self.spawn(ctx, || {
                let items = contributions(&extensions_dir(), "themes")
                    .into_iter()
                    .map(|(label, extension, path)| (label, extension, ImportThemeFile(path)))
                    .collect();
                Done::Browse("Select a VS Code theme to import", items)
            }),
            SelectTheme => {
                let current = |name: &str| if self.settings.theme == name { "current" } else { "" }.to_owned();
                let mut items: Vec<Choice> = ["dark", "light"].map(|n| (n[..1].to_uppercase() + &n[1..], current(n), SetTheme(n.into()))).into();
                items.extend(installed(&config).into_iter().map(|n| (n.clone(), format!("imported {}", current(&n)).trim().to_owned(), SetTheme(n))));
                items.push(("Import VS Code Theme...".into(), String::new(), ImportTheme));
                items.push(("Browse Installed VS Code Themes...".into(), String::new(), BrowseThemes));
                self.search.pick("Select the color theme", items);
            }
            _ => self.icon_command(cmd, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r##"{
        // a comment
        "name": "Test", /* block */
        "type": "light",
        "colors": {
            "editor.background": "#fff",
            "editor.foreground": "#112233ff",
            "sideBar.background": "#10203040",
            "focusBorder": "#abcd",
            "terminal.ansiRed": "#ff0000",
            "terminal.ansiBrightBlue": "#0000ff",
            "list.hoverBackground": "not a color",
        },
        "tokenColors": [
            { "settings": { "foreground": "#222222" } },
            { "scope": "comment", "settings": { "foreground": "#00ff00", "fontStyle": "italic bold" } },
            { "scope": ["keyword.control", "storage"], "settings": { "foreground": "#ff00ff" } },
            { "scope": "string", "settings": { "foreground": "#0000ff" } },
            { "scope": "string.quoted.double", "settings": { "fontStyle": "underline" } },
        ],
        "semanticTokenColors": { "function": { "foreground": "#123456" }, "type": "#654321", "variable:rust": "#000000", },
    }"##;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_comments_trailing_commas_and_every_color_form() {
        let value = jsonc(r#"{"a": "// not a comment", /* x */ "b": [1, 2,], "c": {"d": 1,},}"#).unwrap();
        assert_eq!(value["a"], "// not a comment");
        assert_eq!(value["b"].as_array().unwrap().len(), 2);
        assert!(jsonc("{\"a\": ").is_err());
        assert_eq!(color("#f0a"), Some(Color32::from_rgb(255, 0, 170)));
        assert_eq!(color("#f0a8"), Some(Color32::from_rgba_unmultiplied(255, 0, 170, 136)));
        assert_eq!(color("#102030"), Some(Color32::from_rgb(16, 32, 48)));
        assert_eq!(color("#10203080"), Some(Color32::from_rgba_unmultiplied(16, 32, 48, 128)));
        assert_eq!(color("#+f"), None);
        assert_eq!(color("102030"), None);
        assert_eq!(color("#12345"), None);
    }

    #[test]
    fn maps_workbench_colors_with_fallbacks_for_dark_and_light() {
        let theme = build(&jsonc(THEME).unwrap()).unwrap();
        assert!(!theme.dark);
        assert_eq!(theme.bg, Color32::WHITE);
        assert_eq!(theme.text, Color32::from_rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.ansi[1], Color32::RED);
        assert_eq!(theme.ansi[12], Color32::BLUE);
        assert_eq!(theme.ansi[2], Theme::light().ansi[2]);
        assert_eq!(theme.hover, Theme::light().hover);
        assert_ne!(theme.panel, Theme::light().panel);
        assert_ne!(theme.accent, Theme::light().accent);
        assert_eq!(theme.accent.a(), 255);

        let dark = build(&jsonc(r##"{"uiTheme": "hc-black", "colors": {"editor.background": "#000000"}}"##).unwrap()).unwrap();
        assert!(dark.dark);
        assert_eq!(dark.panel, Theme::dark().panel);
        assert_eq!(dark.bg, Color32::BLACK);
    }

    #[test]
    fn maps_token_colors_and_semantic_colors() {
        let theme = build(&jsonc(THEME).unwrap()).unwrap();
        assert_eq!(theme.syntax.comment, Color32::from_rgb(0, 255, 0));
        assert_eq!(theme.syntax.keyword, Color32::from_rgb(255, 0, 255));
        assert_eq!(theme.syntax.string, Color32::BLUE);
        assert_eq!(theme.tokens.rules.len(), 4);
        let comment = &theme.tokens.rules[0];
        assert_eq!(comment.style.font_style, Some(FontStyle::ITALIC | FontStyle::BOLD));
        assert_eq!(comment.style.foreground, Some(Color { r: 0, g: 255, b: 0, a: 255 }));
        assert_eq!(theme.tokens.rules[3].style.font_style, Some(FontStyle::UNDERLINE));
        assert_eq!(theme.semantic, [Some(Color32::from_rgb(0x65, 0x43, 0x21)), Some(Color32::from_rgb(0x12, 0x34, 0x56)), None]);

        let tm = build(&jsonc(r#"{"tokenColors": "./x.tmTheme"}"#).unwrap());
        assert!(tm.err().unwrap().contains(".tmTheme"));
    }

    #[test]
    fn follows_include_chains_and_imports_a_self_contained_copy() {
        let dir = temp("import");
        let (source, config) = (dir.join("src"), dir.join("config"));
        std::fs::create_dir_all(source.join("base")).unwrap();
        std::fs::write(
            source.join("base").join("base.json"),
            r##"{"type": "light", "colors": {"editor.background": "#eeeeee", "focusBorder": "#111111"}, "tokenColors": [{"scope": "comment", "settings": {"foreground": "#010101"}}]}"##,
        )
        .unwrap();
        std::fs::write(
            source.join("child.json"),
            r##"{"include": "./base/base.json", "colors": {"focusBorder": "#222222"}, "tokenColors": [{"scope": "string", "settings": {"foreground": "#020202"}}],}"##,
        )
        .unwrap();
        let merged = resolve(&source.join("child.json")).unwrap();
        assert_eq!(merged["colors"]["editor.background"], "#eeeeee");
        assert_eq!(merged["colors"]["focusBorder"], "#222222");
        assert_eq!(merged["tokenColors"].as_array().unwrap().len(), 2);
        assert!(merged.get("include").is_none());

        assert_eq!(import(&source.join("child.json"), &config).unwrap(), "child");
        std::fs::remove_dir_all(&source).unwrap();
        let theme = load(&config, "child").unwrap();
        assert!(!theme.dark);
        assert_eq!(theme.accent, Color32::from_rgb(0x22, 0x22, 0x22));
        assert_eq!(theme.syntax.comment, Color32::from_rgb(1, 1, 1));
        assert_eq!(installed(&config), ["child"]);
        assert!(load(&config, "../child").is_err());
        assert!(load(&config, "missing").is_err());
        remove(&config, "child");
        assert!(installed(&config).is_empty());
        assert_eq!(safe_name("Light"), "Light imported");
        assert_eq!(safe_name("a/b:c"), "a_b_c");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lists_contributed_themes_with_translated_labels() {
        let dir = temp("contrib");
        let ext = dir.join("pub.ext-1.0.0");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("package.json"),
            r#"{"contributes": {"themes": [{"label": "%dusk%", "uiTheme": "vs-dark", "path": "./t/dusk.json"}], "iconThemes": [{"id": "icons", "label": "Icons", "path": "./i.json"}]}}"#,
        )
        .unwrap();
        std::fs::write(ext.join("package.nls.json"), r#"{"dusk": "Dusk"}"#).unwrap();
        let themes = contributions(&dir, "themes");
        assert_eq!((themes[0].0.as_str(), themes[0].1.as_str()), ("Dusk", "pub.ext-1.0.0"));
        assert!(themes[0].2.ends_with("dusk.json"));
        assert_eq!(contributions(&dir, "iconThemes")[0].0, "Icons");
        let _ = std::fs::remove_dir_all(dir);
    }
}
