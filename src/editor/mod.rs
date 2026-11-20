use crate::command::Command;
use crate::git::Git;
use crate::settings::Settings;
use crate::theme::Theme;
use eframe::egui;
use std::path::{Path, PathBuf};

pub struct EditorStatus {
    pub line: usize,
    pub column: usize,
    pub selected: usize,
    pub cursors: usize,
    pub language: String,
}

pub struct Editors {}

impl Editors {
    pub fn new() -> Self {
        Self {}
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        _settings: &Settings,
        _git: &Git,
        _cmds: &mut Vec<Command>,
    ) {
        ui.colored_label(theme.text_dim, "Editor");
    }

    pub fn open(&mut self, _path: &Path) {}

    pub fn exec(&mut self, _cmd: &Command, _settings: &Settings, _ctx: &egui::Context) -> bool {
        false
    }

    pub fn is_empty(&self) -> bool {
        true
    }

    pub fn open_paths(&self) -> Vec<PathBuf> {
        vec![]
    }

    pub fn active_path(&self) -> Option<PathBuf> {
        None
    }

    pub fn status(&self) -> Option<EditorStatus> {
        None
    }

    pub fn has_focus(&self) -> bool {
        false
    }

    pub fn any_dirty(&self) -> bool {
        false
    }

    pub fn discard_all(&mut self) {}

    pub fn path_changed(&mut self, _old: &Path, _new: Option<&Path>) {}
}
