#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod app;
mod command;
mod editor;
mod explorer;
mod git;
mod scm;
mod search;
mod settings;
mod statusbar;
mod terminal;
mod theme;
mod titlebar;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Halo")
            .with_app_id("halo")
            .with_decorations(false)
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Halo",
        options,
        Box::new(|cc| Ok(Box::new(app::Halo::new(cc)))),
    )
}
