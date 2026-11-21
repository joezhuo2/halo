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
    use std::sync::atomic::{AtomicIsize, Ordering};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowsHookExW(id: i32, hook: unsafe extern "system" fn(i32, usize, isize) -> isize, module: isize, thread: u32) -> isize;
        fn UnhookWindowsHookEx(hook: isize) -> i32;
        fn CallNextHookEx(hook: isize, code: i32, w: usize, l: isize) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attribute: u32, value: *const i32, size: u32) -> i32;
    }
    #[repr(C)]
    struct Message {
        l: isize,
        w: usize,
        id: u32,
        hwnd: isize,
    }
    const CALLWNDPROC: i32 = 4;
    const NCCREATE: u32 = 0x81;
    const CLOAK: u32 = 13;
    static HOOK: AtomicIsize = AtomicIsize::new(0);
    static PENDING: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(vec![]);
    static HIDDEN: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(vec![]);
    fn cloak(hwnd: isize, on: bool) -> bool {
        let on = on as i32;
        unsafe { DwmSetWindowAttribute(hwnd, CLOAK, &on, 4) >= 0 }
    }
    // winit shows a restored maximized window while creating it, and DWM rejects a cloak until the window has
    // been set up, so cloak each new top-level window on the first message DWM accepts it
    unsafe extern "system" fn hook(code: i32, w: usize, l: isize) -> isize {
        let message = unsafe { &*(l as *const Message) };
        let mut pending = PENDING.lock().unwrap();
        if message.id == NCCREATE && unsafe { *(message.l as *const isize).add(3) } == 0 {
            pending.push(message.hwnd);
        } else if let Some(i) = pending.iter().position(|h| *h == message.hwnd) {
            if cloak(message.hwnd, true) {
                HIDDEN.lock().unwrap().push(pending.swap_remove(i));
            }
        }
        unsafe { CallNextHookEx(0, code, w, l) }
    }
    if on {
        if HOOK.load(Ordering::Relaxed) == 0 {
            HOOK.store(unsafe { SetWindowsHookExW(CALLWNDPROC, hook, 0, GetCurrentThreadId()) }, Ordering::Relaxed);
        }
    } else {
        let hook = HOOK.swap(0, Ordering::Relaxed);
        if hook != 0 {
            unsafe { UnhookWindowsHookEx(hook) };
        }
        PENDING.lock().unwrap().clear();
        for hwnd in HIDDEN.lock().unwrap().drain(..) {
            cloak(hwnd, false);
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
