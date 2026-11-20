use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileState {
    Added,
    Modified,
    Deleted,
    Conflicted,
    Ignored,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineChange {
    Added,
    Modified,
    Deleted,
}

pub struct Git {
    root: Option<PathBuf>,
}

impl Git {
    pub fn new() -> Self {
        Self { root: None }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, ctx: &egui::Context) {
        self.root = root;
        self.refresh(ctx);
    }

    pub fn refresh(&mut self, _ctx: &egui::Context) {}

    pub fn poll(&mut self) {}

    pub fn branch(&self) -> Option<&str> {
        None
    }

    pub fn repo_name(&self) -> Option<&str> {
        None
    }

    pub fn state(&self, _path: &Path) -> Option<FileState> {
        None
    }

    pub fn line_changes(&self, _path: &Path) -> Option<Arc<Vec<(usize, LineChange)>>> {
        None
    }
}
