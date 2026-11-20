use crate::command::Command;
use crate::git::Git;
use crate::theme::Theme;
use eframe::egui;
use std::path::{Path, PathBuf};

pub struct Explorer {
    root: Option<PathBuf>,
}

impl Explorer {
    pub fn new() -> Self {
        Self { root: None }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>) {
        self.root = root;
    }

    pub fn refresh(&mut self) {}

    pub fn has_focus(&self) -> bool {
        false
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        theme: &Theme,
        _git: &Git,
        _active: Option<&Path>,
        _cmds: &mut Vec<Command>,
    ) {
        ui.colored_label(theme.text_dim, "Explorer");
    }
}
