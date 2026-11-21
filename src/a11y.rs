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
    static TAB_FOCUS: Cell<bool> = const { Cell::new(false) };
    static MARKS: RefCell<(Marks, Marks)> = RefCell::default();
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
    REDUCED.set(match settings.reduce_motion.as_str() {
        "on" => true,
        "off" => false,
        _ => os::reduce_motion(),
    });
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
