use crate::app::Halo;
use crate::command::{self, Command};
use crate::settings::{Settings, ShellProfile, Task};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Key, KeyboardShortcut, Layout, Rect, RichText, ScrollArea, Sense,
    Stroke, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const TABS: [(&str, &str); 7] = [
    (icons::TEXT_AA, "Text Editor"),
    (icons::FLOPPY_DISK, "Files"),
    (icons::PALETTE, "Appearance"),
    (icons::TERMINAL_WINDOW, "Terminal"),
    (icons::PLAY, "Run and Tasks"),
    (icons::CODE, "Languages"),
    (icons::KEYBOARD, "Keyboard Shortcuts"),
];

#[derive(Default)]
pub struct SettingsPage {
    pub open: bool,
    pub recording: Option<Command>,
    tab: usize,
    filter: String,
    fonts: Option<Vec<(&'static str, String)>>,
    new_run: (String, String),
    new_association: (String, String),
    new_server: (String, String),
}

impl Halo {
    pub(crate) fn settings_dialog(&mut self, ctx: &egui::Context) {
        let theme = self.theme.clone();
        let mut edited = self.settings.clone();
        let mut close = false;
        let screen = ctx.content_rect();
        let size = vec2((screen.width() - 120.0).clamp(520.0, 920.0), (screen.height() - 120.0).clamp(320.0, 640.0));
        let modal = ui::modal(ctx, "settings_page").show(ctx, |ui| {
            ui::outline(ui);
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  Settings", icons::SLIDERS_HORIZONTAL)).size(15.0).color(theme.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close |= ui::icon_button(ui, &theme, icons::X, 24.0, false, "Close").clicked();
                    if ui::icon_button(ui, &theme, icons::BRACKETS_CURLY, 24.0, false, "Open settings.json").clicked() {
                        self.cmds.push(Command::OpenSettingsJson);
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            let body = ui.available_rect_before_wrap();
            ui.allocate_rect(body, Sense::hover());
            ui.painter().hline(body.x_range(), body.top(), Stroke::new(1.0, theme.border));
            let nav = Rect::from_min_max(body.min + vec2(0.0, 12.0), pos2(body.left() + 196.0, body.bottom()));
            ui.painter().vline(nav.right() + 12.0, body.y_range(), Stroke::new(1.0, theme.border));
            ui.scope_builder(UiBuilder::new().max_rect(nav), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, (icon, name)) in TABS.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(nav.width(), 32.0), Sense::click());
                    if resp.clicked() {
                        self.page.tab = index;
                        self.page.recording = None;
                    }
                    let on = ui::anim(ui, resp.id.with("on"), self.page.tab == index);
                    let h = ui::anim(ui, resp.id.with("hover"), resp.hovered());
                    let fill = ui::mix(ui::mix(Color32::TRANSPARENT, theme.hover, h), theme.selection, on);
                    ui.painter().rect_filled(r, CornerRadius::same(5), fill);
                    let color = ui::mix(theme.text_dim, theme.text, on.max(h));
                    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, *icon, FontId::proportional(15.0), color);
                    ui.painter().text(pos2(r.left() + 38.0, r.center().y), Align2::LEFT_CENTER, *name, FontId::proportional(13.0), color);
                }
            });
            let content = Rect::from_min_max(pos2(nav.right() + 34.0, body.top() + 1.0), body.max);
            ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
                ui.set_clip_rect(content.intersect(ui.clip_rect()));
                ScrollArea::both().id_salt(("settings_tab", self.page.tab)).auto_shrink(false).show(ui, |ui| {
                    ui.set_width(ui.available_width() - 16.0);
                    ui.add_space(14.0);
                    ui.label(RichText::new(TABS[self.page.tab].1).size(22.0).color(theme.text));
                    match self.page.tab {
                        0 => editor_tab(ui, &theme, &mut edited, &mut self.page),
                        1 => {
                            let about = "Save changed files one second after the last edit.";
                            toggle(ui, &theme, "Files", "Auto Save", about, &mut edited.auto_save);
                        }
                        2 => appearance_tab(ui, &theme, &mut edited),
                        3 => terminal_tab(ui, &theme, &mut edited),
                        4 => run_tab(ui, &theme, &mut edited, &mut self.page),
                        5 => languages_tab(ui, &theme, &mut edited, &mut self.page),
                        _ => close |= keys_tab(ui, &theme, &mut self.page, &mut self.cmds),
                    }
                    ui.add_space(18.0);
                });
            });
        });
        if edited != self.settings {
            self.settings = edited;
            self.settings.save();
            self.apply_settings(ctx);
        }
        if close || modal.should_close() {
            self.page.open = false;
            self.page.recording = None;
        }
    }
}

fn item(ui: &mut Ui, theme: &Theme, group: &str, name: &str, about: &str) {
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(RichText::new(format!("{group}:")).size(13.5).color(theme.text_dim));
        ui.label(RichText::new(name).size(13.5).color(theme.text));
    });
    if !about.is_empty() {
        ui.add(egui::Label::new(RichText::new(about).color(theme.text_dim)).wrap());
    }
    ui.add_space(5.0);
}

fn toggle(ui: &mut Ui, theme: &Theme, group: &str, name: &str, about: &str, value: &mut bool) {
    item(ui, theme, group, name, "");
    ui.checkbox(value, RichText::new(about).color(theme.text_dim));
}

fn field(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> egui::Response {
    ui.add(TextEdit::singleline(text).margin(ui::FIELD).desired_width(width).min_size(vec2(width, 0.0)).hint_text(hint))
}

fn grid<R>(ui: &mut Ui, id: &str, columns: usize, rows: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id).num_columns(columns).spacing(vec2(8.0, 6.0)).show(ui, rows).inner
}

fn remove_button(ui: &mut Ui, theme: &Theme) -> bool {
    ui::icon_button(ui, theme, icons::TRASH, 24.0, false, "Remove").clicked()
}

fn font_files() -> Vec<(&'static str, String)> {
    let windows = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_default().join("Fonts");
    [
        ("Consolas", windows.join("consola.ttf")),
        ("Cascadia Code", windows.join("CascadiaCode.ttf")),
        ("Cascadia Mono", windows.join("CascadiaMono.ttf")),
        ("Courier New", windows.join("cour.ttf")),
        ("Lucida Console", windows.join("lucon.ttf")),
        ("Menlo", "/System/Library/Fonts/Menlo.ttc".into()),
        ("Monaco", "/System/Library/Fonts/Monaco.ttf".into()),
        ("DejaVu Sans Mono", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".into()),
        ("Liberation Mono", "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf".into()),
        ("Ubuntu Mono", "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf".into()),
    ]
    .into_iter()
    .filter(|(_, file)| file.is_file())
    .map(|(name, file)| (name, file.to_string_lossy().into_owned()))
    .collect()
}

fn editor_tab(ui: &mut Ui, theme: &Theme, s: &mut Settings, page: &mut SettingsPage) {
    item(ui, theme, "Editor", "Font Family", "The font of the editor and the terminal.");
    let fonts = page.fonts.get_or_insert_with(font_files);
    let shown = match fonts.iter().find(|font| font.1 == s.font_family) {
        Some(font) => font.0.to_owned(),
        None if s.font_family.is_empty() => "Default (Hack)".to_owned(),
        None => Path::new(&s.font_family).file_name().unwrap_or_default().to_string_lossy().into_owned(),
    };
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("font_family").width(240.0).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.font_family, String::new(), "Default (Hack)");
            for (name, file) in fonts.iter() {
                ui.selectable_value(&mut s.font_family, file.clone(), *name);
            }
        });
        if ui.button("Browse...").clicked()
            && let Some(file) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_file()
        {
            s.font_family = file.to_string_lossy().into_owned();
        }
    });
    item(ui, theme, "Editor", "Font Size", "The font size in pixels.");
    ui.add(egui::DragValue::new(&mut s.font_size).range(8.0..=32.0).speed(0.25));
    item(ui, theme, "Editor", "Tab Size", "The number of spaces a tab is equal to.");
    ui.add(egui::DragValue::new(&mut s.tab_size).range(1..=8).speed(0.1));
    toggle(ui, theme, "Editor", "Insert Spaces", "Insert spaces when pressing Tab.", &mut s.insert_spaces);
    toggle(ui, theme, "Editor", "Minimap", "Show the minimap beside the editor.", &mut s.minimap);
    toggle(ui, theme, "Editor", "Breadcrumbs", "Show the path and the enclosing symbols above the text.", &mut s.breadcrumbs);
    toggle(ui, theme, "Editor", "Code Lens", "Show reference counts above declarations. Click one to list the places.", &mut s.code_lens);
    toggle(ui, theme, "Editor", "Inlay Hints", "Show inferred types and parameter names inside the line.", &mut s.inlay_hints);
    toggle(ui, theme, "Editor", "Semantic Highlighting", "Color types, functions and constants as the language server reports them.", &mut s.semantic_highlighting);
    toggle(ui, theme, "Editor", "Format On Save", "Format the file with its language server when it is saved by hand.", &mut s.format_on_save);
    toggle(ui, theme, "Editor", "Format On Paste", "Format pasted text with the language server.", &mut s.format_on_paste);
    toggle(ui, theme, "Problems", "Diagnostics", "Underline errors and warnings, and list them in the Problems panel.", &mut s.diagnostics);
    toggle(ui, theme, "Problems", "Status Bar Counts", "Show the number of errors and warnings in the status bar.", &mut s.status_problems);
}

fn appearance_tab(ui: &mut Ui, theme: &Theme, s: &mut Settings) {
    item(ui, theme, "Workbench", "Color Theme", "The base theme. Custom colors are kept for each theme.");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut s.theme, "dark".to_owned(), "Dark");
        ui.selectable_value(&mut s.theme, "light".to_owned(), "Light");
    });
    item(ui, theme, "Workbench", "Colors", "Interface colors on the left, syntax colors on the right.");
    let mut custom = s.colors.get(&s.theme).cloned().unwrap_or_default();
    let list: Vec<(&str, Color32)> = theme.clone().colors().into_iter().map(|(name, color)| (name, *color)).collect();
    grid(ui, "settings_colors", 7, |ui| {
        for row in 0..10 {
            for at in [row, row + 10] {
                let Some((name, mut color)) = list.get(at).copied() else { continue };
                if ui.color_edit_button_srgba(&mut color).changed() {
                    custom.insert(name.to_owned(), color.to_hex());
                }
                ui.label(RichText::new(name).color(theme.text));
                if !custom.contains_key(name) {
                    ui.allocate_space(vec2(24.0, 24.0));
                } else if ui::icon_button(ui, theme, icons::ARROW_COUNTER_CLOCKWISE, 24.0, false, "Reset").clicked() {
                    custom.remove(name);
                }
                ui.allocate_space(vec2(24.0, 1.0));
            }
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    if ui.add_enabled(!custom.is_empty(), egui::Button::new("Reset All Colors")).clicked() {
        custom.clear();
    }
    if custom.is_empty() {
        s.colors.remove(&s.theme);
    } else {
        s.colors.insert(s.theme.clone(), custom);
    }
}

fn args_edit(ui: &mut Ui, id: Id, args: &mut Vec<String>, width: f32) {
    let kept = ui.data_mut(|d| d.get_temp::<String>(id)).filter(|text| text.split_whitespace().eq(args.iter().map(String::as_str)));
    let mut text = kept.unwrap_or_else(|| args.join(" "));
    if field(ui, &mut text, width, "arguments").changed() {
        *args = text.split_whitespace().map(str::to_owned).collect();
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

fn terminal_tab(ui: &mut Ui, theme: &Theme, s: &mut Settings) {
    item(ui, theme, "Terminal", "Default Profile", "The shell a new terminal starts with.");
    let names: Vec<String> = s.terminals.iter().map(|p| p.name.clone()).collect();
    egui::ComboBox::from_id_salt("default_terminal").width(240.0).selected_text(s.default_terminal.clone()).show_ui(ui, |ui| {
        for name in names {
            ui.selectable_value(&mut s.default_terminal, name.clone(), name);
        }
    });
    item(ui, theme, "Terminal", "Profiles", "The name, program and arguments of each shell.");
    let mut remove = None;
    grid(ui, "settings_profiles", 4, |ui| {
        for (index, profile) in s.terminals.iter_mut().enumerate() {
            field(ui, &mut profile.name, 130.0, "name");
            field(ui, &mut profile.program, 250.0, "program");
            args_edit(ui, Id::new(("settings_profile_args", index)), &mut profile.args, 140.0);
            if remove_button(ui, theme) {
                remove = Some(index);
            }
            ui.end_row();
        }
    });
    if let Some(index) = remove {
        s.terminals.remove(index);
    }
    ui.add_space(6.0);
    if ui.button("Add Profile").clicked() {
        s.terminals.push(ShellProfile { name: String::new(), program: String::new(), args: vec![] });
    }
}

fn map_editor(ui: &mut Ui, theme: &Theme, id: &str, map: &mut BTreeMap<String, String>, new: &mut (String, String), hint: &str) {
    let mut remove = None;
    grid(ui, id, 3, |ui| {
        for (key, value) in map.iter_mut() {
            ui.label(RichText::new(key).color(theme.text));
            field(ui, value, 380.0, hint);
            if remove_button(ui, theme) {
                remove = Some(key.clone());
            }
            ui.end_row();
        }
        field(ui, &mut new.0, 90.0, "extension");
        field(ui, &mut new.1, 380.0, hint);
        let key = new.0.trim().trim_start_matches('.').to_lowercase();
        if ui.add_enabled(!key.is_empty() && !new.1.trim().is_empty(), egui::Button::new("Add")).clicked() {
            map.insert(key, new.1.trim().to_owned());
            *new = Default::default();
        }
        ui.end_row();
    });
    if let Some(key) = remove {
        map.remove(&key);
    }
}

fn run_tab(ui: &mut Ui, theme: &Theme, s: &mut Settings, page: &mut SettingsPage) {
    let about = "The command Run File uses for each file extension. {file} is replaced by the path of the file.";
    item(ui, theme, "Run", "Commands", about);
    map_editor(ui, theme, "settings_run", &mut s.run, &mut page.new_run, "command");
    item(ui, theme, "Run", "Tasks", "Named commands that are listed in the Run menu.");
    let mut remove = None;
    grid(ui, "settings_tasks", 3, |ui| {
        for (index, task) in s.tasks.iter_mut().enumerate() {
            field(ui, &mut task.name, 130.0, "name");
            field(ui, &mut task.command, 340.0, "command");
            if remove_button(ui, theme) {
                remove = Some(index);
            }
            ui.end_row();
        }
    });
    if let Some(index) = remove {
        s.tasks.remove(index);
    }
    ui.add_space(6.0);
    if ui.button("Add Task").clicked() {
        s.tasks.push(Task { name: String::new(), command: String::new() });
    }
}

fn languages_tab(ui: &mut Ui, theme: &Theme, s: &mut Settings, page: &mut SettingsPage) {
    let about = "A folder of .sublime-syntax grammars. New grammars and associations are loaded when Halo starts.";
    item(ui, theme, "Languages", "Grammars Folder", about);
    ui.horizontal(|ui| {
        field(ui, &mut s.grammars, 320.0, "folder");
        if ui.button("Browse...").clicked()
            && let Some(folder) = rfd::FileDialog::new().pick_folder()
        {
            s.grammars = folder.to_string_lossy().into_owned();
        }
        if ui.button("Add Grammar...").clicked()
            && let Some(file) = rfd::FileDialog::new().add_filter("Grammars", &["sublime-syntax"]).pick_file()
        {
            let target = Path::new(&s.grammars).join(file.file_name().unwrap_or_default());
            let copied = std::fs::create_dir_all(&s.grammars).and_then(|_| std::fs::copy(&file, &target));
            if let Err(error) = copied {
                ui::notify(format!("Could not add {}: {error}", file.display()));
            }
        }
    });
    let about = "Open files with an extension as another language, given by its name or its extension.";
    item(ui, theme, "Languages", "File Associations", about);
    map_editor(ui, theme, "settings_associations", &mut s.associations, &mut page.new_association, "language");
    let about = "Start the language server of a file for diagnostics, completion, hover, navigation, rename and formatting.";
    toggle(ui, theme, "Language Servers", "Enabled", about, &mut s.lsp);
    let about = "The command that starts the server for each file extension. It must be on PATH and speak LSP over standard input and output.";
    item(ui, theme, "Language Servers", "Commands", about);
    map_editor(ui, theme, "settings_servers", &mut s.servers, &mut page.new_server, "command");
    let languages = crate::editor::languages();
    item(ui, theme, "Languages", "Installed", &format!("{} languages are installed.", languages.len()));
    for (name, extensions) in languages {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        let names = ui.painter().with_clip_rect(rect.with_max_x(rect.left() + 218.0));
        names.text(rect.left_center(), Align2::LEFT_CENTER, name, FontId::proportional(13.0), theme.text);
        let at = rect.left_center() + vec2(230.0, 0.0);
        ui.painter().with_clip_rect(rect).text(at, Align2::LEFT_CENTER, extensions, FontId::proportional(12.5), theme.text_dim);
    }
}

fn keys_tab(ui: &mut Ui, theme: &Theme, page: &mut SettingsPage, cmds: &mut Vec<Command>) -> bool {
    let ctx = ui.ctx().clone();
    if let Some(cmd) = page.recording.clone() {
        let pressed = ctx.input_mut(|i| {
            let found = i.events.iter().find_map(|event| match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some(KeyboardShortcut::new(*modifiers, *key)),
                _ => None,
            });
            i.events.retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_)));
            found
        });
        if let Some(shortcut) = pressed {
            page.recording = None;
            if shortcut.logical_key != Key::Escape || shortcut.modifiers.any() {
                command::rebind(&cmd, Some(command::key_text(&shortcut)));
            }
        }
    }
    let mut close = false;
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        field(ui, &mut page.filter, 300.0, "Search commands");
        if ui.button("Open keybindings.json").clicked() {
            cmds.push(Command::OpenKeybindings);
            close = true;
        }
    });
    ui.add_space(4.0);
    let about = "Click a key and press the new one. Escape cancels. Two-step chords are edited in keybindings.json.";
    ui.add(egui::Label::new(RichText::new(about).color(theme.text_dim)).wrap());
    ui.add_space(10.0);
    let filter = page.filter.to_lowercase();
    grid(ui, "settings_keys", 3, |ui| {
        for cmd in command::all().into_iter().filter(|c| c.label().to_lowercase().contains(&filter)) {
            ui.label(RichText::new(cmd.label()).color(theme.text));
            let recording = page.recording.as_ref() == Some(&cmd);
            let (text, color) = match command::shortcut_text(&ctx, &cmd) {
                _ if recording => ("Press a key...".to_owned(), theme.accent),
                Some(keys) => (keys, theme.text),
                None => ("Not set".to_owned(), theme.text_dim),
            };
            if ui.add(egui::Button::new(RichText::new(text).color(color)).min_size(vec2(170.0, 0.0))).clicked() {
                page.recording = (!recording).then(|| cmd.clone());
            }
            ui.horizontal(|ui| {
                if ui::icon_button(ui, theme, icons::X, 24.0, false, "Remove this key").clicked() {
                    command::rebind(&cmd, Some(String::new()));
                }
                if ui::icon_button(ui, theme, icons::ARROW_COUNTER_CLOCKWISE, 24.0, false, "Reset to the default key").clicked() {
                    command::rebind(&cmd, None);
                }
            });
            ui.end_row();
        }
    });
    close
}
