#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod app;
mod blame;
mod command;
mod dap;
mod debug;
mod editor;
mod explorer;
mod explorer_ops;
mod find;
mod git;
mod glob;
mod history;
mod jsonc;
mod layers;
mod icon_theme;
mod lsp;
mod notifications;
mod output;
mod scm;
mod search;
mod settings;
mod settings_page;
mod shortcuts;
mod shell_integration;
mod statusbar;
mod tasks;
mod terminal;
mod terminal_find;
mod terminal_links;
mod test_parse;
mod testing;
mod theme;
mod theme_import;
mod timeline;
mod titlebar;
mod ui;
mod update;
mod when;
mod windows;
mod wsl;

use eframe::egui;

fn main() -> eframe::Result {
    let test = std::env::var_os("HALO_COMMANDS").is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Halo")
        .with_app_id("halo")
        .with_decorations(false)
        .with_inner_size([1180.0, 720.0])
        .with_min_inner_size([640.0, 400.0]);
    if test {
        viewport = viewport.with_active(false).with_position([-1177.0, 150.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: !test,
        persistence_path: test.then(|| std::env::temp_dir().join("halo-test.ron")),
        ..Default::default()
    };
    eframe::run_native(
        "Halo",
        options,
        Box::new(|cc| Ok(Box::new(app::Halo::new(cc)))),
    )
}
