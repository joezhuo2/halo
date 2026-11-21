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
pub fn conceal(on: bool) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumThreadWindows(thread: u32, each: unsafe extern "system" fn(isize, isize) -> i32, param: isize) -> i32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
        fn SetWindowLongW(hwnd: isize, index: i32, value: i32) -> i32;
        fn SetLayeredWindowAttributes(hwnd: isize, key: u32, alpha: u8, flags: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    const EXSTYLE: i32 = -20;
    const LAYERED: i32 = 0x80000;
    const ALPHA: u32 = 2;
    static HIDDEN: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(vec![]);
    unsafe extern "system" fn each(hwnd: isize, _: isize) -> i32 {
        unsafe {
            let style = GetWindowLongW(hwnd, EXSTYLE);
            if style & LAYERED == 0 {
                SetWindowLongW(hwnd, EXSTYLE, style | LAYERED);
                SetLayeredWindowAttributes(hwnd, 0, 0, ALPHA);
                HIDDEN.lock().unwrap().push(hwnd);
            }
        }
        1
    }
    if on {
        unsafe { EnumThreadWindows(GetCurrentThreadId(), each, 0) };
    } else {
        for hwnd in HIDDEN.lock().unwrap().drain(..) {
            unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, ALPHA) };
        }
    }
}

#[cfg(not(windows))]
pub fn conceal(_on: bool) {}

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
