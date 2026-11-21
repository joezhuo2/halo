use crate::settings::Settings;
use eframe::egui::{Context, Id, Ui};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Editor,
    Sidebar,
    Panel,
    Status,
}

impl Region {
    pub fn label(self) -> &'static str {
        match self {
            Region::Editor => "Editor",
            Region::Sidebar => "Side Bar",
            Region::Panel => "Panel",
            Region::Status => "Status Bar",
        }
    }
}

const REGIONS: [Region; 4] = [Region::Editor, Region::Sidebar, Region::Panel, Region::Status];

#[derive(Default, Clone)]
struct Marks {
    pass: u64,
    current: Option<usize>,
    first: [Option<Id>; 4],
    anchored: [bool; 4],
    focused: Option<usize>,
}

thread_local! {
    static REDUCED: Cell<bool> = const { Cell::new(false) };
    static OPTIMIZED: Cell<bool> = const { Cell::new(false) };
    static READER: Cell<bool> = const { Cell::new(false) };
    static HELP: Cell<bool> = const { Cell::new(false) };
    static TAB_FOCUS: Cell<bool> = const { Cell::new(false) };
    static MARKS: RefCell<(Marks, Marks)> = RefCell::default();
    static SPEECH: RefCell<Speech> = RefCell::default();
}

#[derive(Default)]
struct Speech {
    text: String,
    assertive: bool,
    watched: std::collections::HashMap<&'static str, String>,
    saved: Option<f64>,
}

fn speech<R>(f: impl FnOnce(&mut Speech) -> R) -> R {
    SPEECH.with_borrow_mut(f)
}

pub fn announce(text: impl Into<String>, assertive: bool) {
    let text = text.into();
    speech(|s| {
        let repeat = s.text.trim_end_matches('\u{a0}') == text && !s.text.ends_with('\u{a0}');
        s.text = if repeat { format!("{text}\u{a0}") } else { text };
        s.assertive = assertive;
    });
}

pub fn watch(key: &'static str, value: &str) {
    watch_with(key, value, false);
}

pub fn watch_with(key: &'static str, value: &str, assertive: bool) {
    let changed = speech(|s| s.watched.insert(key, value.to_owned()).is_none_or(|old| old != value));
    if changed && !value.is_empty() {
        announce(value, assertive);
    }
}

pub fn saved(now: f64) {
    speech(|s| s.saved = Some(now));
}

pub fn spoken() -> (String, bool) {
    speech(|s| (s.text.clone(), s.assertive))
}

pub fn optimized() -> bool {
    OPTIMIZED.get()
}

pub fn frame(ctx: &Context, (errors, warnings): (usize, usize)) -> bool {
    let reader = ctx.accesskit_node_builder(eframe::egui::accesskit_root_id(), |_| ()).is_some();
    let now = ctx.input(|i| i.time);
    if speech(|s| s.saved.take_if(|t| now - *t > 1.0).is_some()) {
        announce(format!("{errors} errors, {warnings} warnings"), false);
    } else if speech(|s| s.saved.is_some()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }
    let (text, assertive) = spoken();
    if !text.is_empty() {
        ctx.accesskit_node_builder(Id::new("halo_announcement"), |n| {
            n.set_role(crate::ui::Role::Status);
            n.set_live(if assertive { eframe::egui::accesskit::Live::Assertive } else { eframe::egui::accesskit::Live::Polite });
            n.set_label(text);
        });
    }
    READER.replace(reader) != reader
}

pub fn region_name(ctx: &Context) -> Option<Region> {
    marks(ctx, |now, last| now.focused.or(last.focused)).map(|at| REGIONS[at])
}

pub fn show_help() {
    HELP.set(true);
}

pub fn help_ui(ctx: &Context, theme: &crate::theme::Theme) {
    if !HELP.get() {
        return;
    }
    use crate::command::Command::*;
    let region = region_name(ctx);
    let (title, commands) = match region {
        Some(Region::Editor) | None => ("Editor", vec![ToggleTabFocus, AnnounceCursorPosition, Find, GoToLine, ShowHover, TriggerSuggest]),
        Some(Region::Sidebar) => ("Side Bar", vec![ShowExplorer, ShowSourceControl, FilterExplorer, RefreshExplorer]),
        Some(Region::Panel) => ("Panel", vec![TogglePanel, ShowProblems, NewTerminal]),
        Some(Region::Status) => ("Status Bar", vec![ShowNotifications, ShowProblems]),
    };
    let common = [FocusNextRegion, FocusPrevRegion, ShowCommands, ToggleScreenReaderMode, ShowAccessibilityHelp];
    let modal = crate::ui::modal(ctx, "accessibility_help", crate::ui::Enter::Primary, |ui, keys| {
        crate::ui::access(ui, ui.unique_id(), crate::ui::Role::Dialog, "Accessibility Help", |_| {});
        ui.set_width(460.0);
        ui.label(eframe::egui::RichText::new(format!("Accessibility Help: {title}")).size(15.0).color(theme.text));
        ui.add_space(6.0);
        let lines = ["Tab and Shift+Tab move between controls, arrow keys move inside lists and trees.", "Enter or Space activates the focused control, Escape closes dialogs."];
        for line in lines {
            ui.label(eframe::egui::RichText::new(line).color(theme.text_dim));
        }
        ui.add_space(6.0);
        eframe::egui::Grid::new("accessibility_keys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for cmd in commands.iter().chain(&common) {
                ui.label(eframe::egui::RichText::new(cmd.label()).color(theme.text));
                ui.label(eframe::egui::RichText::new(crate::command::shortcut_text(ctx, cmd).unwrap_or_else(|| "Command palette".into())).color(theme.text_dim));
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        keys.primary(ui, "Close", true)
    });
    if modal.inner || modal.should_close() {
        HELP.set(false);
    }
}

fn marks<R>(ctx: &Context, f: impl FnOnce(&mut Marks, &Marks) -> R) -> R {
    let pass = ctx.cumulative_pass_nr();
    MARKS.with_borrow_mut(|(now, last)| {
        if now.pass != pass {
            *last = std::mem::replace(now, Marks { pass, ..Marks::default() });
        }
        f(now, last)
    })
}

pub fn region(ctx: &Context, region: Option<Region>) {
    marks(ctx, |now, _| now.current = region.and_then(|r| REGIONS.iter().position(|x| *x == r)));
}

pub fn mark(ui: &Ui, id: Id) {
    note(ui, id, false);
}

pub fn anchor(ui: &Ui, id: Id) {
    note(ui, id, true);
}

fn note(ui: &Ui, id: Id, anchor: bool) {
    let focused = ui.memory(|m| m.has_focus(id));
    marks(ui.ctx(), |now, _| {
        if let Some(at) = now.current {
            if anchor && !now.anchored[at] {
                (now.first[at], now.anchored[at]) = (Some(id), true);
            }
            now.first[at].get_or_insert(id);
            if focused {
                now.focused = Some(at);
            }
        }
    });
}

pub fn tab_moves_focus() -> bool {
    TAB_FOCUS.get()
}

pub fn toggle_tab_focus() -> bool {
    TAB_FOCUS.set(!TAB_FOCUS.get());
    TAB_FOCUS.get()
}

pub fn text_node(ui: &mut Ui, id: Id, name: &str, rope: &ropey::Rope, (anchor, head): (usize, usize), prefix: &str) {
    line_node(ui, id, crate::ui::Role::MultilineTextInput, name, || {
        let head = head.min(rope.len_chars());
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let text: String = rope.line(line).chars().take_while(|c| !matches!(c, '\n' | '\r')).collect();
        let shift = prefix.chars().count();
        let at = |pos: usize| pos.saturating_sub(start).min(text.chars().count()) + shift;
        (format!("{prefix}{text}"), line, head - start, (at(anchor), at(head)), anchor.abs_diff(head))
    });
}

pub fn line_node(ui: &mut Ui, id: Id, role: crate::ui::Role, name: &str, caret: impl FnOnce() -> (String, usize, usize, (usize, usize), usize)) {
    anchor(ui, id);
    if ui.ctx().accesskit_node_builder(id, |_| ()).is_none() {
        return;
    }
    let (text, line, column, (anchor, head), selected) = caret();
    let run = ui.new_child(eframe::egui::UiBuilder::new().id_salt((id, "caret_line")).accessibility_parent(id)).unique_id();
    ui.ctx().accesskit_node_builder(run, |n| {
        n.set_role(crate::ui::Role::TextRun);
        n.set_value(text.clone());
        n.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<u8>>());
    });
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(role);
        n.set_label(name);
        n.set_value(text);
        let more = if selected > 0 { format!(", {selected} selected") } else { String::new() };
        n.set_description(format!("Line {}, Column {}{more}", line + 1, column + 1));
        let position = |at| eframe::egui::accesskit::TextPosition { node: run.accesskit_id(), character_index: at };
        n.set_text_selection(eframe::egui::accesskit::TextSelection { anchor: position(anchor), focus: position(head) });
    });
}

pub fn cycle(ctx: &Context, forward: bool) -> Option<Region> {
    let (first, focused) = marks(ctx, |now, last| if now.first.iter().any(Option::is_some) { (now.first, now.focused) } else { (last.first, last.focused) });
    let n = REGIONS.len();
    let start = focused.unwrap_or(if forward { n - 1 } else { 0 });
    let next = (1..=n).map(|step| if forward { (start + step) % n } else { (start + n - step) % n }).find(|&i| first[i].is_some())?;
    let id = first[next]?;
    ctx.memory_mut(|m| m.request_focus(id));
    Some(REGIONS[next])
}

pub fn reduced() -> bool {
    REDUCED.get()
}

pub fn blinking(style: &str) -> &str {
    if reduced() { "solid" } else { style }
}

pub fn configure(settings: &Settings) {
    let optimized = match settings.accessibility_support.as_str() {
        "on" => true,
        "off" => false,
        _ => READER.get(),
    };
    OPTIMIZED.set(optimized);
    REDUCED.set(
        optimized
            || match settings.reduce_motion.as_str() {
                "on" => true,
                "off" => false,
                _ => os::reduce_motion(),
            },
    );
}

pub fn theme_name(settings: &Settings) -> &str {
    match high_contrast() {
        Some(dark) if settings.auto_detect_high_contrast => {
            if dark { &settings.preferred_high_contrast_theme } else { &settings.preferred_high_contrast_light_theme }
        }
        _ => &settings.theme,
    }
}

#[cfg(windows)]
mod os {
    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut core::ffi::c_void, ini: u32) -> i32;
        fn GetSysColor(index: i32) -> u32;
    }

    pub fn high_contrast() -> Option<bool> {
        let mut hc = HighContrast { size: size_of::<HighContrast>() as u32, flags: 0, scheme: std::ptr::null_mut() };
        let ok = unsafe { SystemParametersInfoW(0x0042, hc.size, (&raw mut hc).cast(), 0) } != 0;
        if !ok || hc.flags & 1 == 0 {
            return None;
        }
        let window = unsafe { GetSysColor(5) };
        let luma = [0, 8, 16].map(|shift| window >> shift & 0xff);
        Some(luma[0] * 3 + luma[1] * 6 + luma[2] < 128 * 10)
    }

    pub fn reduce_motion() -> bool {
        let mut animate = 1i32;
        let ok = unsafe { SystemParametersInfoW(0x1042, 0, (&raw mut animate).cast(), 0) } != 0;
        ok && animate == 0
    }
}

#[cfg(not(windows))]
mod os {
    pub fn high_contrast() -> Option<bool> {
        None
    }

    pub fn reduce_motion() -> bool {
        false
    }
}

pub use os::high_contrast;

#[cfg(test)]
pub fn tree(out: &mut eframe::egui::FullOutput) -> Vec<eframe::egui::accesskit::Node> {
    out.textures_delta.clear();
    out.platform_output.accesskit_update.take().map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

#[cfg(test)]
pub fn node<'a>(nodes: &'a [eframe::egui::accesskit::Node], role: crate::ui::Role, name: &str) -> &'a eframe::egui::accesskit::Node {
    nodes.iter().find(|n| n.role() == role && n.label() == Some(name)).unwrap_or_else(|| panic!("no {role:?} named {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{self, Role};
    use eframe::egui::accesskit::Toggled;

    #[test]
    fn announcements_reach_a_polite_or_assertive_live_region() {
        watch("find", "1 of 3");
        assert_eq!(spoken(), ("1 of 3".to_owned(), false));
        announce("Saved", false);
        watch("find", "1 of 3");
        assert_eq!(spoken().0, "Saved");
        watch_with("find", "2 of 3", true);
        assert_eq!(spoken(), ("2 of 3".to_owned(), true));
        announce("2 of 3", false);
        assert!(spoken().0 != "2 of 3" && spoken().0.trim_end_matches('\u{a0}') == "2 of 3");
        let ctx = Context::default();
        ctx.enable_accesskit();
        let mut reader = false;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            saved(ui.input(|i| i.time) - 2.0);
            reader = frame(ui.ctx(), (1, 2));
        });
        let nodes = tree(&mut out);
        let live = nodes.iter().find(|n| n.role() == Role::Status).unwrap();
        assert_eq!((live.label(), live.live()), (Some("1 errors, 2 warnings"), Some(eframe::egui::accesskit::Live::Polite)));
        assert!(reader && READER.get());
        configure(&Settings { accessibility_support: "auto".into(), ..Settings::default() });
        assert!(optimized() && reduced());
        configure(&Settings { accessibility_support: "off".into(), reduce_motion: "off".into(), ..Settings::default() });
        assert!(!optimized() && !reduced());
        READER.set(false);
        configure(&Settings::default());
        assert!(!optimized());
    }

    #[test]
    fn the_accessibility_group_lists_every_setting_with_its_vscode_key() {
        use crate::settings_page::{SETTINGS, score};
        let group: Vec<&str> = SETTINGS.iter().filter(|m| m.group == "Accessibility").map(|m| m.key).collect();
        let keys = ["zoom_level", "auto_detect_high_contrast", "preferred_high_contrast_theme", "preferred_high_contrast_light_theme", "accessibility_support", "reduce_motion"];
        assert!(keys.iter().all(|k| group.contains(k)), "{group:?}");
        assert!(group.iter().all(|k| crate::layers::vscode_key(k).is_some()));
        let found = |q: &str| SETTINGS.iter().filter(|m| score(m, &crate::settings_page::parse_query(q).words).is_some()).map(|m| m.key).collect::<Vec<_>>();
        assert!(found("accessibility").len() >= keys.len() && found("reduceMotion").contains(&"reduce_motion"));
    }

    #[test]
    fn f6_cycles_focus_through_every_region() {
        let ctx = Context::default();
        let theme = crate::theme::Theme::dark();
        let draw = |ctx: &Context| {
            let mut out = ctx.run_ui(Default::default(), |ui| {
                let parts = [(None, "Menu"), (Some(Region::Status), "Branch"), (Some(Region::Sidebar), "Files"), (Some(Region::Panel), "Problems"), (Some(Region::Editor), "Tab")];
                for (at, name) in parts {
                    region(ui.ctx(), at);
                    ui::icon_button(ui, &theme, ui::icons::X, 20.0, false, name);
                }
                let text = ui.allocate_response(eframe::egui::vec2(50.0, 50.0), eframe::egui::Sense::click());
                anchor(ui, text.id);
                region(ui.ctx(), None);
            });
            out.textures_delta.clear();
            ctx.memory(|m| m.focused())
        };
        draw(&ctx);
        let mut seen = vec![];
        for _ in 0..5 {
            let region = cycle(&ctx, true).unwrap();
            seen.push((region, draw(&ctx).is_some()));
        }
        assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), [Region::Editor, Region::Sidebar, Region::Panel, Region::Status, Region::Editor]);
        assert!(seen.iter().all(|s| s.1));
        assert_eq!(cycle(&ctx, false), Some(Region::Status));
        draw(&ctx);
        assert_eq!(cycle(&ctx, false), Some(Region::Panel));
        assert!(!tab_moves_focus() && toggle_tab_focus() && !toggle_tab_focus());
    }

    #[test]
    fn shared_widgets_expose_roles_names_and_states() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let theme = crate::theme::Theme::dark();
        let (mut on, mut open, mut tab, mut pick) = (true, false, 1, "b");
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui::checkbox(ui, &theme, &mut on, "Word Wrap");
            ui::section(ui, &theme, "Outline", &mut open);
            ui::tabs(ui, &theme, Id::new("t"), &["Problems", "Output"], &mut tab);
            ui::icon_button(ui, &theme, ui::icons::X, 20.0, false, "Close");
            eframe::egui::ComboBox::from_label("Theme").selected_text(pick).show_ui(ui, |ui| ui.selectable_value(&mut pick, "a", "a"));
        });
        let nodes = tree(&mut out);
        assert_eq!(node(&nodes, Role::CheckBox, "Word Wrap").toggled(), Some(Toggled::True));
        assert_eq!(node(&nodes, Role::Button, "Outline").is_expanded(), Some(false));
        assert_eq!(node(&nodes, Role::Tab, "Output").is_selected(), Some(true));
        assert_eq!(node(&nodes, Role::Tab, "Problems").is_selected(), Some(false));
        node(&nodes, Role::Button, "Close");
        assert!(nodes.iter().any(|n| n.role() == Role::ComboBox));
    }

    #[test]
    fn follows_the_os_high_contrast_mode_only_when_asked() {
        let mut settings = Settings { theme: "light".into(), auto_detect_high_contrast: false, ..Settings::default() };
        assert_eq!(theme_name(&settings), "light");
        settings.auto_detect_high_contrast = true;
        let expected = match high_contrast() {
            Some(true) => "hc-dark",
            Some(false) => "hc-light",
            None => "light",
        };
        assert_eq!(theme_name(&settings), expected);
    }

    #[test]
    fn reduced_motion_jumps_animations_to_their_end() {
        let ctx = eframe::egui::Context::default();
        let frame = |on: bool| {
            let mut value = 0.0;
            let mut out = ctx.run_ui(Default::default(), |ui| value = crate::ui::anim(ui, eframe::egui::Id::new("a"), on));
            out.textures_delta.clear();
            value
        };
        configure(&Settings { reduce_motion: "off".into(), ..Settings::default() });
        frame(false);
        assert!(frame(true) < 1.0);
        frame(false);
        configure(&Settings { reduce_motion: "on".into(), ..Settings::default() });
        assert_eq!(frame(true), 1.0);
        assert_eq!(frame(false), 0.0);
        assert_eq!(blinking("smooth"), "solid");
        configure(&Settings::default());
        assert_eq!(reduced(), os::reduce_motion());
    }
}
