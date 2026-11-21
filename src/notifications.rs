use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke, Ui, vec2};
use std::sync::{Mutex, MutexGuard};

const HISTORY: usize = 100;
const SHOWN: usize = 3;
const FADE: f32 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Running,
    Done,
    Failed,
}

#[derive(Clone)]
pub struct Notification {
    pub id: u64,
    pub severity: Severity,
    pub text: String,
    pub actions: Vec<(String, Command)>,
    pub time: u64,
    pub read: bool,
    pub toast: bool,
    pub left: f32,
    pub progress: Option<(Option<f32>, State)>,
}

impl Notification {
    fn timeout(&self) -> Option<f32> {
        match (self.progress.map(|p| p.1), self.severity) {
            (Some(State::Running | State::Failed), _) | (None, Severity::Error) => None,
            (Some(State::Done), _) => Some(4.0),
            (None, Severity::Info) => Some(5.0),
            (None, Severity::Warning) => Some(8.0),
        }
    }
}

pub struct Store {
    items: Vec<Notification>,
    next: u64,
    dnd: bool,
    center: bool,
    focus: bool,
    hovered: Option<u64>,
    bell: Option<Rect>,
}

impl Store {
    const fn new() -> Self {
        Self { items: Vec::new(), next: 1, dnd: false, center: false, focus: false, hovered: None, bell: None }
    }

    fn add(&mut self, severity: Severity, text: String, actions: Vec<(String, Command)>, progress: Option<(Option<f32>, State)>, time: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        let mut note = Notification { id, severity, text, actions, time, read: self.center, toast: !self.dnd || severity == Severity::Error, left: 0.0, progress };
        note.left = note.timeout().unwrap_or(0.0);
        self.items.push(note);
        if self.items.len() > HISTORY {
            self.items.remove(0);
        }
        id
    }

    fn get(&mut self, id: u64) -> Option<&mut Notification> {
        self.items.iter_mut().find(|n| n.id == id)
    }

    fn set(&mut self, id: u64, text: String, fraction: Option<f32>) {
        if let Some(note) = self.get(id) {
            note.text = text;
            if let Some((fraction_now, State::Running)) = &mut note.progress {
                *fraction_now = fraction.map(|f| f.clamp(0.0, 1.0));
            }
        }
    }

    fn finish(&mut self, id: u64, text: String, ok: bool) {
        let Some(note) = self.get(id) else { return };
        note.text = text;
        note.progress = Some((ok.then_some(1.0), if ok { State::Done } else { State::Failed }));
        note.severity = if ok { Severity::Info } else { Severity::Error };
        note.left = note.timeout().unwrap_or(0.0);
        if !ok {
            note.toast = true;
            note.read = false;
        }
    }

    fn shown(&self) -> Vec<usize> {
        let toasts: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].toast).collect();
        toasts[toasts.len().saturating_sub(SHOWN)..].to_vec()
    }

    fn hidden(&self) -> usize {
        self.items.iter().filter(|n| n.toast).count().saturating_sub(SHOWN)
    }

    fn tick(&mut self, dt: f32, hovered: Option<u64>) {
        if self.dnd {
            self.items.iter_mut().filter(|n| n.severity != Severity::Error).for_each(|n| n.toast = false);
        }
        for index in self.shown() {
            let note = &mut self.items[index];
            if hovered != Some(note.id) && note.timeout().is_some() {
                note.left -= dt;
                if note.left <= 0.0 {
                    note.toast = false;
                }
            }
        }
    }

    fn close(&mut self, id: u64) {
        if let Some(note) = self.get(id) {
            note.toast = false;
            note.read = true;
        }
    }

    fn remove(&mut self, id: u64) {
        self.items.retain(|n| n.id != id);
    }

    fn read_all(&mut self) {
        self.items.iter_mut().for_each(|n| n.read = true);
    }

    fn unread(&self) -> usize {
        self.items.iter().filter(|n| !n.read).count()
    }

    fn act(&mut self, id: u64, index: usize) -> Option<Command> {
        let command = self.get(id)?.actions.get(index).map(|a| a.1.clone());
        self.close(id);
        command
    }

    fn exec(&mut self, cmd: &Command) {
        match cmd {
            Command::ShowNotifications => {
                self.center ^= true;
                self.focus = false;
            }
            Command::ClearNotifications => self.items.clear(),
            Command::ToggleDoNotDisturb => self.dnd ^= true,
            Command::FocusNotification => (self.focus, self.center) = (true, false),
            _ => {}
        }
    }
}

static STORE: Mutex<Store> = Mutex::new(Store::new());
static REPAINT: Mutex<Option<egui::Context>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn touch() {
    if let Some(ctx) = &*lock(&REPAINT) {
        ctx.request_repaint();
    }
}

pub fn push(severity: Severity, text: String, actions: Vec<(String, Command)>) -> u64 {
    crate::output::log("Halo", format!("[{}] {text}", severity.label()));
    let id = lock(&STORE).add(severity, text, actions, None, crate::output::now());
    touch();
    id
}

pub fn exec(cmd: &Command) {
    lock(&STORE).exec(cmd);
    touch();
}

pub fn summary() -> (usize, bool) {
    let store = lock(&STORE);
    (store.unread(), store.dnd)
}

pub fn set_bell(rect: Rect) {
    lock(&STORE).bell = Some(rect);
}

pub struct Progress(u64);

pub fn progress(title: String) -> Progress {
    crate::output::log("Halo", format!("[info] {title}"));
    let id = lock(&STORE).add(Severity::Info, title, vec![], Some((None, State::Running)), crate::output::now());
    touch();
    Progress(id)
}

impl Progress {
    pub fn set(&self, text: impl Into<String>, fraction: Option<f32>) {
        lock(&STORE).set(self.0, text.into(), fraction);
        touch();
    }

    pub fn done(self, text: impl Into<String>) {
        self.end(text.into(), true);
    }

    pub fn failed(self, text: impl Into<String>) {
        self.end(text.into(), false);
    }

    fn end(self, text: String, ok: bool) {
        crate::output::log("Halo", format!("[{}] {text}", if ok { "info" } else { "error" }));
        lock(&STORE).finish(self.0, text, ok);
        touch();
    }
}

pub fn show(ctx: &egui::Context, theme: &Theme, cmds: &mut Vec<Command>) {
    crate::output::begin_frame();
    *lock(&REPAINT) = Some(ctx.clone());
    show_in(ctx, theme, cmds, &STORE);
}

fn show_in(ctx: &egui::Context, theme: &Theme, cmds: &mut Vec<Command>, store: &Mutex<Store>) {
    let dt = ctx.input(|i| i.unstable_dt).min(0.5);
    {
        let mut s = lock(store);
        let hovered = s.hovered.take();
        s.tick(dt, hovered);
        if s.center {
            s.read_all();
        }
        let target = s.shown().last().map(|&i| s.items[i].id);
        if s.focus && !s.center {
            match target {
                None => s.focus = false,
                Some(id) => {
                    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                        if let Some(cmd) = s.act(id, 0) {
                            cmds.push(cmd);
                        }
                        s.focus = false;
                    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                        s.close(id);
                        s.focus = false;
                    }
                }
            }
        } else if s.center && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            s.center = false;
        }
    }
    if lock(store).center {
        center_ui(ctx, theme, cmds, store);
    } else {
        toasts_ui(ctx, theme, cmds, store);
    }
}

fn mark(note: &Notification, theme: &Theme) -> (&'static str, Color32) {
    match (note.progress.map(|p| p.1), note.severity) {
        (Some(State::Done), _) => (icons::CHECK_CIRCLE, theme.accent),
        (_, Severity::Info) => (icons::INFO, theme.accent),
        (_, Severity::Warning) => (icons::WARNING, theme.warning),
        (_, Severity::Error) => (icons::X_CIRCLE, theme.error),
    }
}

fn pill(ui: &mut Ui, theme: &Theme, id: Id, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.0), theme.text);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let hover = ui::anim(ui, id.with("hover"), resp.hovered());
    ui.painter().rect_filled(rect, CornerRadius::same(4), ui::mix(theme.hover, theme.selection, hover));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, theme.text);
    resp
}

fn body(ui: &mut Ui, theme: &Theme, note: &Notification, width: f32) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
        let (icon, color) = mark(note, theme);
        if matches!(note.progress, Some((None, State::Running))) {
            ui::spinner(ui, rect.center(), 7.0, theme.accent);
        } else {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(16.0), color);
        }
        ui.vertical(|ui| {
            ui.set_width(width);
            let label = ui.add(egui::Label::new(RichText::new(&note.text).color(theme.text)).wrap().sense(Sense::click()));
            label.context_menu(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(note.text.clone());
                    ui.close();
                }
            });
            if let Some((Some(fraction), _)) = note.progress {
                let (bar, _) = ui.allocate_exact_size(vec2(width, 4.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(2), theme.hover);
                let filled = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
                ui.painter().rect_filled(filled, CornerRadius::same(2), theme.accent);
            }
            if !note.actions.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for (index, (label, _)) in note.actions.iter().enumerate() {
                        if pill(ui, theme, Id::new(("notification_action", note.id, index)), label).clicked() {
                            clicked = Some(index);
                        }
                    }
                });
            }
        });
    });
    clicked
}

fn toasts_ui(ctx: &egui::Context, theme: &Theme, cmds: &mut Vec<Command>, store: &Mutex<Store>) {
    let (notes, hidden, focus) = {
        let s = lock(store);
        (s.shown().into_iter().map(|i| s.items[i].clone()).collect::<Vec<_>>(), s.hidden(), s.focus)
    };
    if notes.is_empty() {
        return;
    }
    let (mut close, mut action, mut hovered) = (None, None, None);
    egui::Area::new(Id::new("notification_toasts")).order(egui::Order::Foreground).anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -34.0)).show(ctx, |ui| {
        if hidden > 0 {
            ui.label(RichText::new(format!("+{hidden} more")).size(11.0).color(theme.text_dim));
        }
        for (position, note) in notes.iter().enumerate() {
            let fade = ui::fade_in(ui, Id::new(("notification_fade", note.id))) * note.timeout().map_or(1.0, |_| (note.left / FADE).clamp(0.0, 1.0));
            if fade < 1.0 {
                ctx.request_repaint();
            }
            let focused = focus && position == notes.len() - 1;
            let out = ui.scope(|ui| {
                ui.set_opacity(fade);
                ui::access(ui, ui.unique_id(), ui::Role::Alert, &format!("{}: {}", note.severity.label(), note.text), |_| {});
                let border = match note.severity {
                    Severity::Info => theme.border,
                    Severity::Warning => theme.warning,
                    Severity::Error => theme.error,
                };
                let stroke = if focused { Stroke::new(2.0, theme.accent) } else { Stroke::new(1.0, border) };
                Frame::new().fill(theme.raised).stroke(stroke).corner_radius(CornerRadius::same(6)).inner_margin(12.0).show(ui, |ui| {
                    ui.set_width(380.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui::icon_button(ui, theme, icons::X, 20.0, false, "Dismiss").clicked() {
                            close = Some(note.id);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                            if let Some(index) = body(ui, theme, note, 300.0) {
                                action = Some((note.id, index));
                            }
                        });
                    });
                })
            });
            if ui.rect_contains_pointer(out.inner.response.rect) {
                hovered = Some(note.id);
            }
            ui.add_space(4.0);
        }
    });
    if notes.iter().any(|n| n.timeout().is_some()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut s = lock(store);
    s.hovered = hovered;
    if let Some(id) = close {
        s.close(id);
    }
    if let Some((id, index)) = action
        && let Some(cmd) = s.act(id, index)
    {
        cmds.push(cmd);
    }
}

fn center_ui(ctx: &egui::Context, theme: &Theme, cmds: &mut Vec<Command>, store: &Mutex<Store>) {
    let (notes, dnd, bell) = {
        let s = lock(store);
        (s.items.iter().rev().cloned().collect::<Vec<_>>(), s.dnd, s.bell)
    };
    let (mut remove, mut action, mut clear, mut toggle, mut close) = (None, None, false, false, false);
    let area = egui::Area::new(Id::new("notification_center")).order(egui::Order::Foreground).anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -34.0)).show(ctx, |ui| {
        Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.border)).corner_radius(CornerRadius::same(6)).inner_margin(12.0).show(ui, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("NOTIFICATIONS").size(11.0).color(theme.text_dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = ui::icon_button(ui, theme, icons::X, 22.0, false, "Close").clicked();
                    clear = ui::icon_button(ui, theme, icons::TRASH, 22.0, false, "Clear All Notifications").clicked();
                    let tip = if dnd { "Do Not Disturb is on" } else { "Do Not Disturb" };
                    toggle = ui::icon_button(ui, theme, if dnd { icons::BELL_SLASH } else { icons::BELL }, 22.0, dnd, tip).clicked();
                });
            });
            ui.add_space(6.0);
            if notes.is_empty() {
                ui.label(RichText::new("No notifications.").color(theme.text_dim));
            }
            ScrollArea::vertical().id_salt("notification_history").max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                for note in &notes {
                    ui.separator();
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(360.0);
                            ui.label(RichText::new(format!("{}  {}", crate::output::clock(note.time), note.severity.label())).size(11.0).color(theme.text_dim));
                            if let Some(index) = body(ui, theme, note, 330.0) {
                                action = Some((note.id, index));
                            }
                        });
                        if ui::icon_button(ui, theme, icons::X, 20.0, false, "Dismiss").clicked() {
                            remove = Some(note.id);
                        }
                    });
                }
            });
        });
    });
    let outside = ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !bell.is_some_and(|b| b.contains(p)))
    });
    let mut s = lock(store);
    s.center &= !(close || outside);
    s.dnd ^= toggle;
    if clear {
        s.items.clear();
    }
    if let Some(id) = remove {
        s.remove(id);
    }
    if let Some((id, index)) = action
        && let Some(cmd) = s.act(id, index)
    {
        cmds.push(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, RawInput, pos2};

    fn add(store: &mut Store, severity: Severity, text: &str) -> u64 {
        store.add(severity, text.to_owned(), vec![], None, 0)
    }

    fn toasts(store: &Store) -> Vec<&str> {
        store.shown().into_iter().map(|i| store.items[i].text.as_str()).collect()
    }

    #[test]
    fn stacks_newest_last_and_limits_to_three() {
        let mut store = Store::new();
        for text in ["a", "b", "c", "d", "e"] {
            add(&mut store, Severity::Error, text);
        }
        assert_eq!(toasts(&store), ["c", "d", "e"]);
        assert_eq!(store.hidden(), 2);
        let id = store.items[4].id;
        store.close(id);
        assert_eq!(toasts(&store), ["b", "c", "d"]);
        assert_eq!(store.hidden(), 1);
    }

    #[test]
    fn times_out_by_severity_and_pauses_on_hover() {
        let mut store = Store::new();
        add(&mut store, Severity::Info, "info");
        let warning = add(&mut store, Severity::Warning, "warning");
        let error = add(&mut store, Severity::Error, "error");
        store.tick(4.9, None);
        assert_eq!(toasts(&store), ["info", "warning", "error"]);
        store.tick(0.2, Some(warning));
        assert_eq!(toasts(&store), ["warning", "error"]);
        store.tick(100.0, Some(warning));
        store.tick(3.0, None);
        assert_eq!(toasts(&store), ["warning", "error"]);
        store.tick(0.2, None);
        assert_eq!(toasts(&store), ["error"]);
        store.tick(1000.0, None);
        assert_eq!(toasts(&store), ["error"]);
        assert_eq!(store.items.len(), 3);
        store.close(error);
        assert!(toasts(&store).is_empty());
    }

    #[test]
    fn keeps_a_capped_history_with_an_unread_count() {
        let mut store = Store::new();
        for i in 0..HISTORY + 5 {
            add(&mut store, Severity::Info, &format!("n{i}"));
        }
        assert_eq!(store.items.len(), HISTORY);
        assert_eq!(store.items[0].text, "n5");
        assert_eq!(store.unread(), HISTORY);
        let id = store.items[0].id;
        store.close(id);
        assert_eq!(store.unread(), HISTORY - 1);
        store.read_all();
        assert_eq!(store.unread(), 0);
        store.remove(id);
        assert_eq!(store.items.len(), HISTORY - 1);
        store.exec(&Command::ClearNotifications);
        assert!(store.items.is_empty());
    }

    #[test]
    fn do_not_disturb_hides_toasts_except_errors_but_records_them() {
        let mut store = Store::new();
        store.exec(&Command::ToggleDoNotDisturb);
        add(&mut store, Severity::Info, "quiet");
        add(&mut store, Severity::Warning, "also quiet");
        add(&mut store, Severity::Error, "loud");
        store.tick(0.0, None);
        assert_eq!(toasts(&store), ["loud"]);
        assert_eq!(store.items.len(), 3);
        assert_eq!(store.unread(), 3);
        store.exec(&Command::ToggleDoNotDisturb);
        add(&mut store, Severity::Info, "back");
        assert_eq!(toasts(&store), ["loud", "back"]);
    }

    #[test]
    fn progress_runs_then_finishes_or_fails() {
        let mut store = Store::new();
        let id = store.add(Severity::Info, "Cloning".into(), vec![], Some((None, State::Running)), 0);
        store.tick(1000.0, None);
        assert_eq!(toasts(&store), ["Cloning"]);
        store.set(id, "Cloning 40%".into(), Some(0.4));
        assert_eq!(store.items[0].progress, Some((Some(0.4), State::Running)));
        store.finish(id, "Cloned".into(), true);
        assert_eq!(store.items[0].progress, Some((Some(1.0), State::Done)));
        store.tick(3.9, None);
        assert_eq!(toasts(&store), ["Cloned"]);
        store.tick(0.2, None);
        assert!(toasts(&store).is_empty());
        let failed = store.add(Severity::Info, "Pushing".into(), vec![], Some((None, State::Running)), 0);
        store.finish(failed, "Push failed".into(), false);
        store.tick(1000.0, None);
        assert_eq!((toasts(&store), store.items[1].severity), (vec!["Push failed"], Severity::Error));
        store.set(999, "x".into(), None);
        store.finish(999, "x".into(), true);
    }

    #[test]
    fn global_progress_handle_updates_its_notification() {
        let handle = progress("Global progress test".into());
        let id = handle.0;
        handle.set("Halfway", Some(0.5));
        let find = || lock(&STORE).items.iter().find(|n| n.id == id).cloned().unwrap();
        assert_eq!((find().text, find().progress), ("Halfway".to_owned(), Some((Some(0.5), State::Running))));
        handle.failed("Broke");
        assert_eq!((find().text, find().severity, find().progress.unwrap().1), ("Broke".to_owned(), Severity::Error, State::Failed));
    }

    fn frame(ctx: &egui::Context, store: &Mutex<Store>, cmds: &mut Vec<Command>, events: Vec<Event>) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), events, focused: true, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show_in(ui.ctx(), &Theme::dark(), cmds, store));
        out.textures_delta.clear();
    }

    fn press(pos: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn clicking_a_toast_action_runs_its_command_and_closes_the_toast() {
        let store = Mutex::new(Store::new());
        let id = lock(&store).add(Severity::Info, "Saved the file".into(), vec![("Open Folder".into(), Command::OpenFolder)], None, 0);
        let (ctx, mut cmds) = (egui::Context::default(), vec![]);
        for _ in 0..3 {
            frame(&ctx, &store, &mut cmds, vec![]);
        }
        let center = ctx.read_response(Id::new(("notification_action", id, 0))).unwrap().rect.center();
        frame(&ctx, &store, &mut cmds, vec![Event::PointerMoved(center)]);
        frame(&ctx, &store, &mut cmds, vec![press(center, true)]);
        frame(&ctx, &store, &mut cmds, vec![press(center, false)]);
        assert_eq!(cmds, [Command::OpenFolder]);
        assert!(lock(&store).shown().is_empty());
        assert_eq!(lock(&store).items.len(), 1);
    }

    #[test]
    fn focused_toast_runs_its_action_on_enter_and_dismisses_on_escape() {
        let store = Mutex::new(Store::new());
        lock(&store).add(Severity::Error, "first".into(), vec![("Go".into(), Command::OpenFile)], None, 0);
        lock(&store).add(Severity::Error, "second".into(), vec![("Go".into(), Command::Save)], None, 0);
        lock(&store).exec(&Command::FocusNotification);
        let (ctx, mut cmds) = (egui::Context::default(), vec![]);
        let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, &store, &mut cmds, vec![]);
        frame(&ctx, &store, &mut cmds, vec![key(Key::Enter)]);
        assert_eq!(cmds, [Command::Save]);
        lock(&store).exec(&Command::FocusNotification);
        frame(&ctx, &store, &mut cmds, vec![key(Key::Escape)]);
        assert!(lock(&store).shown().is_empty());
        assert!(!lock(&store).focus);
    }

    #[test]
    fn opens_the_notification_center_and_marks_everything_read() {
        let store = Mutex::new(Store::new());
        lock(&store).add(Severity::Warning, "Heads up".into(), vec![], None, 0);
        lock(&store).add(Severity::Info, "Done".into(), vec![], None, 0);
        let (ctx, mut cmds) = (egui::Context::default(), vec![]);
        frame(&ctx, &store, &mut cmds, vec![]);
        frame(&ctx, &store, &mut cmds, vec![]);
        assert!(ctx.memory(|m| m.area_rect(Id::new("notification_toasts"))).is_some());
        assert!(ctx.memory(|m| m.area_rect(Id::new("notification_center"))).is_none());
        assert_eq!(lock(&store).unread(), 2);
        lock(&store).exec(&Command::ShowNotifications);
        for _ in 0..2 {
            frame(&ctx, &store, &mut cmds, vec![]);
        }
        let rect = ctx.memory(|m| m.area_rect(Id::new("notification_center"))).unwrap();
        assert!(rect.width() >= 420.0 && rect.height() > 100.0);
        assert_eq!(lock(&store).unread(), 0);
        let key = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, &store, &mut cmds, vec![key]);
        assert!(!lock(&store).center);
    }
}
