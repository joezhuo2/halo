use crate::app::Halo;
use crate::command::Command;
use crate::ui;
use eframe::egui;
use std::path::{Path, PathBuf};

pub const FLAG: &str = "--new-window";

pub fn secondary() -> bool {
    std::env::args().any(|a| a == FLAG)
}

pub fn path_arg() -> Option<PathBuf> {
    std::env::args().skip(1).find(|a| a != FLAG).map(PathBuf::from)
}

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
