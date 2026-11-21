use crate::app::Halo;
use crate::command::Command;
use crate::search::Choice;
use crate::settings;
use crate::theme_import::{Done, contributions, extensions_dir, read_json, safe_name};
use crate::ui;
use eframe::egui::{self, Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};
use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    File,
    Folder,
    FolderOpen,
    Root,
    RootOpen,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
struct Assoc {
    file: Option<String>,
    folder: Option<String>,
    folder_expanded: Option<String>,
    root_folder: Option<String>,
    root_folder_expanded: Option<String>,
    file_extensions: HashMap<String, String>,
    file_names: HashMap<String, String>,
    folder_names: HashMap<String, String>,
    folder_names_expanded: HashMap<String, String>,
    language_ids: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Def {
    icon_path: Option<String>,
    font_character: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Raw {
    icon_definitions: HashMap<String, Def>,
    #[serde(flatten)]
    base: Assoc,
    light: Assoc,
}

pub struct IconTheme {
    paths: HashMap<String, PathBuf>,
    base: Assoc,
    light: Assoc,
}

fn lowered(map: &mut HashMap<String, String>) {
    *map = std::mem::take(map).into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
}

fn pick<'a>(sets: &[&'a Assoc], find: impl Fn(&'a Assoc) -> Option<&'a String>) -> Option<&'a String> {
    sets.iter().find_map(|set| find(set))
}

pub fn language_id(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    match lower.as_str() {
        "dockerfile" => return "dockerfile",
        "makefile" => return "makefile",
        _ => {}
    }
    match lower.rsplit_once('.').map_or("", |(_, ext)| ext) {
        "rs" => "rust",
        "py" | "pyw" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "json" => "json",
        "jsonc" => "jsonc",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "go" => "go",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "shellscript",
        "ps1" => "powershell",
        "bat" | "cmd" => "bat",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "sql" => "sql",
        "lua" => "lua",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "vue" => "vue",
        "svelte" => "svelte",
        "txt" => "plaintext",
        _ => "",
    }
}

impl IconTheme {
    fn parse(value: Value, dir: &Path) -> Result<Self, String> {
        let mut raw: Raw = serde_json::from_value(value).map_err(|e| format!("the icon theme is not in the VS Code format: {e}"))?;
        let with_path = raw.icon_definitions.values().filter(|d| d.icon_path.is_some()).count();
        if with_path == 0 {
            return Err(match raw.icon_definitions.values().any(|d| d.font_character.is_some()) {
                true => "this is a font glyph icon theme (fontCharacter, like Seti), and Halo only supports themes whose icons are SVG or PNG files".into(),
                false => "the icon theme defines no icons with an iconPath".into(),
            });
        }
        for assoc in [&mut raw.base, &mut raw.light] {
            for map in [&mut assoc.file_extensions, &mut assoc.file_names, &mut assoc.folder_names, &mut assoc.folder_names_expanded, &mut assoc.language_ids] {
                lowered(map);
            }
        }
        let paths = raw.icon_definitions.into_iter().filter_map(|(id, d)| Some((id, dir.join(d.icon_path?)))).collect();
        Ok(Self { paths, base: raw.base, light: raw.light })
    }

    pub fn lookup(&self, name: &str, kind: Kind, dark: bool, language: &dyn Fn() -> &'static str) -> Option<&PathBuf> {
        let lower = name.to_lowercase();
        let sets: Vec<&Assoc> = if dark { vec![&self.base] } else { vec![&self.light, &self.base] };
        let id = match kind {
            Kind::File => pick(&sets, |a| a.file_names.get(&lower))
                .or_else(|| lower.match_indices('.').find_map(|(at, _)| pick(&sets, |a| a.file_extensions.get(&lower[at + 1..]))))
                .or_else(|| pick(&sets, |a| a.language_ids.get(language())))
                .or_else(|| pick(&sets, |a| a.file.as_ref())),
            Kind::Folder => pick(&sets, |a| a.folder_names.get(&lower)).or_else(|| pick(&sets, |a| a.folder.as_ref())),
            Kind::FolderOpen => pick(&sets, |a| a.folder_names_expanded.get(&lower))
                .or_else(|| pick(&sets, |a| a.folder_expanded.as_ref()))
                .or_else(|| pick(&sets, |a| a.folder.as_ref())),
            Kind::Root => pick(&sets, |a| a.root_folder.as_ref()).or_else(|| pick(&sets, |a| a.folder.as_ref())),
            Kind::RootOpen => pick(&sets, |a| a.root_folder_expanded.as_ref())
                .or_else(|| pick(&sets, |a| a.root_folder.as_ref()))
                .or_else(|| pick(&sets, |a| a.folder_expanded.as_ref()))
                .or_else(|| pick(&sets, |a| a.folder.as_ref())),
        };
        self.paths.get(id?)
    }
}

enum Mode {
    Builtin,
    Hidden,
    Theme(Box<IconTheme>),
}

struct State {
    name: String,
    mode: Mode,
    found: HashMap<(String, Kind, bool), Option<PathBuf>>,
    textures: HashMap<(PathBuf, u32), Option<TextureHandle>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn dir(config: &Path) -> PathBuf {
    config.join("icon-themes")
}

pub fn installed(config: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir(config))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("theme.json").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

fn load(config: &Path, name: &str) -> Result<IconTheme, String> {
    if safe_name(name) != name {
        return Err("the name is not valid".into());
    }
    let folder = dir(config).join(name);
    IconTheme::parse(read_json(&folder.join("theme.json"))?, &folder)
}

pub fn select(name: &str, config: &Path) {
    let mut state = STATE.lock().unwrap();
    if state.as_ref().map_or("builtin", |s| s.name.as_str()) == name {
        return;
    }
    *state = make(name, config);
}

fn make(name: &str, config: &Path) -> Option<State> {
    let mode = match name {
        "builtin" => return None,
        "none" => Mode::Hidden,
        name => load(config, name).map(|t| Mode::Theme(Box::new(t))).unwrap_or_else(|error| {
            ui::notify(format!("The icon theme {name} could not be loaded, the built-in icons are in use: {error}"));
            Mode::Builtin
        }),
    };
    Some(State { name: name.to_owned(), mode, found: HashMap::new(), textures: HashMap::new() })
}

pub fn forget() {
    *STATE.lock().unwrap() = None;
}

pub fn active() -> bool {
    STATE.lock().unwrap().as_ref().is_some_and(|s| !matches!(s.mode, Mode::Builtin))
}

fn rasterize(path: &Path, px: u32) -> Option<ColorImage> {
    let data = std::fs::read(path).ok()?;
    let mut target = Pixmap::new(px, px)?;
    let fit = |w: f32, h: f32| {
        let scale = (px as f32 / w).min(px as f32 / h);
        Transform::from_translate((px as f32 - w * scale) / 2.0, (px as f32 - h * scale) / 2.0).pre_scale(scale, scale)
    };
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        resvg::render(&tree, fit(tree.size().width(), tree.size().height()), &mut target.as_mut());
    } else {
        let source = Pixmap::decode_png(&data).ok()?;
        let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() };
        target.draw_pixmap(0, 0, source.as_ref(), &paint, fit(source.width() as f32, source.height() as f32), None);
    }
    Some(ColorImage::from_rgba_premultiplied([px as usize; 2], target.data()))
}

pub fn paint(painter: &Painter, center: Pos2, size: f32, name: &str, kind: Kind, alpha: f32) -> bool {
    let mut guard = STATE.lock().unwrap();
    let Some(state) = guard.as_mut() else { return false };
    draw(state, painter, center, size, name, kind, alpha)
}

fn draw(state: &mut State, painter: &Painter, center: Pos2, size: f32, name: &str, kind: Kind, alpha: f32) -> bool {
    let theme = match &state.mode {
        Mode::Builtin => return false,
        Mode::Hidden => return true,
        Mode::Theme(theme) => theme,
    };
    let ctx = painter.ctx();
    let dark = ctx.global_style().visuals.dark_mode;
    let key = (name.to_owned(), kind, dark);
    let path = state.found.entry(key).or_insert_with(|| theme.lookup(name, kind, dark, &|| language_id(name)).cloned());
    let Some(path) = path.clone() else { return false };
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as u32;
    let texture = state.textures.entry((path.clone(), px)).or_insert_with(|| {
        rasterize(&path, px).map(|image| ctx.load_texture(format!("icon {}", path.display()), image, TextureOptions::LINEAR))
    });
    let Some(texture) = texture else { return false };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(texture.id(), Rect::from_center_size(center, vec2(size, size)), uv, Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8));
    true
}

pub fn import(source: &Path, config: &Path) -> Result<String, String> {
    let mut value = read_json(source)?;
    let from = source.parent().unwrap_or(Path::new("."));
    IconTheme::parse(value.clone(), from)?;
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match safe_name(&stem).as_str() {
        n @ ("builtin" | "none") => format!("{n} icons"),
        n => n.to_owned(),
    };
    let target = dir(config).join(&name);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(target.join("icons")).map_err(|e| format!("Could not create {}: {e}", target.display()))?;
    let mut copied: HashMap<PathBuf, String> = HashMap::new();
    if let Some(defs) = value.get_mut("iconDefinitions").and_then(Value::as_object_mut) {
        for def in defs.values_mut() {
            let Some(path) = def.get("iconPath").and_then(Value::as_str).map(|p| from.join(p)) else { continue };
            let relative = match copied.get(&path) {
                Some(done) => done.clone(),
                None => {
                    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let relative = format!("icons/{}-{file}", copied.len());
                    if std::fs::copy(&path, target.join(&relative)).is_err() {
                        continue;
                    }
                    copied.insert(path, relative.clone());
                    relative
                }
            };
            def["iconPath"] = Value::String(relative);
        }
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(target.join("theme.json"), text).map_err(|e| format!("Could not save the icon theme: {e}"))?;
    Ok(name)
}

pub fn remove(config: &Path, name: &str) {
    if safe_name(name) == name && !name.is_empty() {
        let _ = std::fs::remove_dir_all(dir(config).join(name));
    }
}

impl Halo {
    pub(crate) fn icon_command(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        match cmd {
            ImportIconTheme => {
                if let Some(file) = rfd::FileDialog::new().add_filter("JSON", &["json", "jsonc"]).pick_file() {
                    self.spawn_import(ctx, file);
                }
            }
            ImportIconThemeFile(file) => self.spawn_import(ctx, file),
            BrowseIconThemes => self.spawn_browse(ctx),
            SetIconTheme(name) => {
                forget();
                self.settings.icon_theme = name;
                self.settings.save();
                self.apply_settings(ctx);
            }
            SelectIconTheme => {
                let current = |name: &str| if self.settings.icon_theme == name { "current" } else { "" }.to_owned();
                let mut items: Vec<Choice> = vec![
                    ("Halo (default)".into(), current("builtin"), SetIconTheme("builtin".into())),
                    ("None".into(), current("none"), SetIconTheme("none".into())),
                ];
                items.extend(installed(&settings::dir()).into_iter().map(|n| (n.clone(), format!("imported {}", current(&n)).trim().to_owned(), SetIconTheme(n))));
                items.push(("Import VS Code Icon Theme...".into(), String::new(), ImportIconTheme));
                items.push(("Browse Installed VS Code Icon Themes...".into(), String::new(), BrowseIconThemes));
                self.search.pick("Select the file icon theme", items);
            }
            _ => {}
        }
    }

    fn spawn_import(&self, ctx: &egui::Context, file: PathBuf) {
        self.spawn(ctx, move || Done::Imported(true, import(&file, &settings::dir())));
    }

    fn spawn_browse(&self, ctx: &egui::Context) {
        self.spawn(ctx, || {
            let items = contributions(&extensions_dir(), "iconThemes")
                .into_iter()
                .map(|(label, extension, path)| (label, extension, Command::ImportIconThemeFile(path)))
                .collect();
            Done::Browse("Select a VS Code icon theme to import", items)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> IconTheme {
        let value: Value = serde_json::from_str(
            r#"{
            "iconDefinitions": {
                "file": {"iconPath": "f.svg"}, "folder": {"iconPath": "d.svg"}, "open": {"iconPath": "o.svg"},
                "root": {"iconPath": "r.svg"}, "ts": {"iconPath": "ts.svg"}, "dts": {"iconPath": "dts.svg"},
                "rs": {"iconPath": "rs.svg"}, "cargo": {"iconPath": "cargo.svg"}, "src": {"iconPath": "src.svg"},
                "lightfile": {"iconPath": "lf.svg"}, "lightts": {"iconPath": "lts.svg"}
            },
            "file": "file", "folder": "folder", "folderExpanded": "open", "rootFolder": "root",
            "fileExtensions": {"ts": "ts", "D.TS": "dts"},
            "fileNames": {"Cargo.toml": "cargo"},
            "languageIds": {"rust": "rs"},
            "folderNames": {"SRC": "src"},
            "light": {"file": "lightfile", "fileExtensions": {"ts": "lightts"}}
        }"#,
        )
        .unwrap();
        IconTheme::parse(value, Path::new("base")).unwrap()
    }

    fn find(t: &IconTheme, name: &str, kind: Kind, dark: bool) -> Option<String> {
        t.lookup(name, kind, dark, &|| language_id(name)).map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
    }

    #[test]
    fn looks_icons_up_by_name_longest_extension_language_and_default() {
        let t = theme();
        assert_eq!(find(&t, "cargo.TOML", Kind::File, true).as_deref(), Some("cargo.svg"));
        assert_eq!(find(&t, "index.ts", Kind::File, true).as_deref(), Some("ts.svg"));
        assert_eq!(find(&t, "types.d.ts", Kind::File, true).as_deref(), Some("dts.svg"));
        assert_eq!(find(&t, "TYPES.D.TS", Kind::File, true).as_deref(), Some("dts.svg"));
        assert_eq!(find(&t, "main.rs", Kind::File, true).as_deref(), Some("rs.svg"));
        assert_eq!(find(&t, "notes.xyz", Kind::File, true).as_deref(), Some("f.svg"));
        assert_eq!(find(&t, "Makefile", Kind::File, true).as_deref(), Some("f.svg"));
    }

    #[test]
    fn picks_folder_expanded_root_and_light_variants() {
        let t = theme();
        assert_eq!(find(&t, "src", Kind::Folder, true).as_deref(), Some("src.svg"));
        assert_eq!(find(&t, "lib", Kind::Folder, true).as_deref(), Some("d.svg"));
        assert_eq!(find(&t, "src", Kind::FolderOpen, true).as_deref(), Some("o.svg"));
        assert_eq!(find(&t, "halo", Kind::Root, true).as_deref(), Some("r.svg"));
        assert_eq!(find(&t, "halo", Kind::RootOpen, true).as_deref(), Some("r.svg"));
        assert_eq!(find(&t, "index.ts", Kind::File, false).as_deref(), Some("lts.svg"));
        assert_eq!(find(&t, "types.d.ts", Kind::File, false).as_deref(), Some("dts.svg"));
        assert_eq!(find(&t, "notes.xyz", Kind::File, false).as_deref(), Some("lf.svg"));
        assert_eq!(find(&t, "Cargo.toml", Kind::File, false).as_deref(), Some("cargo.svg"));
        assert_eq!(find(&t, "src", Kind::Folder, false).as_deref(), Some("src.svg"));
    }

    #[test]
    fn rejects_font_glyph_themes() {
        let glyphs: Value = serde_json::from_str(r#"{"fonts": [], "iconDefinitions": {"a": {"fontCharacter": "\\E001"}}, "file": "a"}"#).unwrap();
        assert!(IconTheme::parse(glyphs, Path::new(".")).err().unwrap().contains("fontCharacter"));
        assert!(IconTheme::parse(serde_json::json!({}), Path::new(".")).is_err());
    }

    fn square(color: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="{color}"/></svg>"#)
    }

    #[test]
    fn imports_into_the_config_folder_and_rasterizes_at_the_requested_size() {
        let dir = std::env::temp_dir().join(format!("halo-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (source, config) = (dir.join("ext"), dir.join("config"));
        std::fs::create_dir_all(source.join("icons")).unwrap();
        std::fs::write(source.join("icons").join("a.svg"), square("#ff0000")).unwrap();
        std::fs::write(source.join("icons").join("b.svg"), square("#0000ff")).unwrap();
        std::fs::write(
            source.join("my-icons.json"),
            r##"{
                // comment
                "iconDefinitions": {"a": {"iconPath": "./icons/a.svg"}, "b": {"iconPath": "./icons/b.svg"}, "a2": {"iconPath": "./icons/a.svg"}},
                "file": "a", "fileExtensions": {"b": "b", "c": "a2"},
            }"##,
        )
        .unwrap();
        assert_eq!(import(&source.join("my-icons.json"), &config).unwrap(), "my-icons");
        std::fs::remove_dir_all(&source).unwrap();
        assert_eq!(installed(&config), ["my-icons"]);
        let loaded = load(&config, "my-icons").unwrap();
        let path = loaded.lookup("x.b", Kind::File, true, &|| "").unwrap().clone();
        assert!(path.starts_with(config.join("icon-themes").join("my-icons")));
        let image = rasterize(&path, 32).unwrap();
        assert_eq!(image.size, [32, 32]);
        assert_eq!(image.pixels[16 * 32 + 16], Color32::from_rgb(0, 0, 255));
        assert_ne!(loaded.lookup("x.c", Kind::File, true, &|| ""), loaded.lookup("x.b", Kind::File, true, &|| ""));
        assert!(load(&config, "missing").is_err());
        remove(&config, "my-icons");
        assert!(installed(&config).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paints_cached_textures_only_while_a_theme_is_selected() {
        let dir = std::env::temp_dir().join(format!("halo-icons-paint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.svg"), square("#00ff00")).unwrap();
        std::fs::write(dir.join("t.json"), r#"{"iconDefinitions": {"a": {"iconPath": "a.svg"}}, "file": "a"}"#).unwrap();
        let config = dir.join("config");
        let name = import(&dir.join("t.json"), &config).unwrap();
        let ctx = egui::Context::default();
        let mut state = make(&name, &config).unwrap();
        let mut painted = |state: &mut State, name: &str, kind: Kind| {
            let mut result = false;
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0))), ..Default::default() };
            ctx.run_ui(input, |ui| result = draw(state, ui.painter(), pos2(20.0, 20.0), 16.0, name, kind, 1.0)).textures_delta.clear();
            result
        };
        assert!(!painted(&mut state, "src", Kind::Folder));
        assert!(painted(&mut state, "a.rs", Kind::File));
        assert!(painted(&mut state, "b.rs", Kind::File));
        assert_eq!(state.textures.len(), 1);
        let mut hidden = make("none", &config).unwrap();
        assert!(painted(&mut hidden, "a.rs", Kind::File) && hidden.textures.is_empty());
        assert!(make("builtin", &config).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
