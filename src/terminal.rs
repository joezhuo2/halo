use crate::command::Command;
use crate::settings::Settings;
use crate::theme::Theme;
use eframe::egui;
use std::path::Path;

pub struct BottomPanel {}

impl BottomPanel {
    pub fn new() -> Self {
        Self {}
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        _settings: &Settings,
        _cwd: Option<&Path>,
        _cmds: &mut Vec<Command>,
    ) {
        ui.colored_label(theme.text_dim, "Terminal");
    }

    pub fn exec(
        &mut self,
        _cmd: &Command,
        _settings: &Settings,
        _cwd: Option<&Path>,
        _ctx: &egui::Context,
    ) -> bool {
        false
    }

    pub fn has_focus(&self) -> bool {
        false
    }
}
