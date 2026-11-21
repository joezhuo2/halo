use crate::app::Halo;
use crate::command::Command;
use crate::ui;
use eframe::egui;
use std::path::Path;

pub const FLAG: &str = "--new-window";

static SECONDARY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn secondary() -> bool {
    SECONDARY.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_secondary(on: bool) {
    SECONDARY.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(windows)]
pub fn cloak(on: bool) {
    use std::sync::atomic::{AtomicU32, Ordering};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumThreadWindows(thread: u32, each: unsafe extern "system" fn(isize, isize) -> i32, param: isize) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const i32, size: u32) -> i32;
    }
    unsafe extern "system" fn each(hwnd: isize, on: isize) -> i32 {
        let on = on as i32;
        unsafe { DwmSetWindowAttribute(hwnd, 13, &on, 4) };
        1
    }
    static THREAD: AtomicU32 = AtomicU32::new(0);
    if on {
        THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
    }
    let thread = THREAD.load(Ordering::Relaxed);
    if thread != 0 {
        unsafe { EnumThreadWindows(thread, each, on as isize) };
    }
}

#[cfg(not(windows))]
pub fn cloak(_on: bool) {}

fn spawn(folder: Option<&Path>) {
    let started = std::env::current_exe().and_then(|exe| {
        let mut command = std::process::Command::new(exe);
        command.arg(FLAG).args(folder);
        command.spawn()
    });
    if let Err(error) = started {
        ui::notify_error(format!("Halo could not open a new window: {error}"));
    }
}

impl Halo {
    pub(crate) fn window_command(&mut self, cmd: &Command) {
        match cmd {
            Command::NewWindow => spawn(None),
            Command::OpenFolderInNewWindow => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    spawn(Some(&folder));
                }
            }
            _ => spawn(self.session.folder.as_deref()),
        }
    }

    pub(crate) fn floating(&mut self, ctx: &egui::Context) {
        for (id, viewport, builder) in self.editors.floating() {
            ctx.show_viewport_immediate(viewport, builder, |ui, _| {
                if !self.page.keys.recording() {
                    self.global_shortcuts(ui.ctx());
                }
                self.editors.window_ui(ui, id, &self.theme, &self.settings, &self.git, &mut self.cmds);
            });
        }
        self.editors.close_empty_groups();
    }
}
