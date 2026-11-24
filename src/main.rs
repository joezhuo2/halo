#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod a11y;
mod accounts;
mod agent_changes;
mod ai;
mod app;
mod blame;
mod cli;
mod command;
mod commit_ai;
mod dap;
mod debug;
mod editor;
mod explorer;
mod explorer_ops;
mod find;
mod git;
mod glob;
mod history;
mod ipc;
mod jumplist;
mod macros;
mod jsonc;
mod layers;
mod icon_theme;
mod lsp;
mod notifications;
mod output;
mod run_form;
mod scm;
mod scratch;
mod search;
mod settings;
mod settings_page;
mod shortcuts;
mod sound;
mod shell_integration;
mod statusbar;
mod tasks;
mod terminal;
mod terminal_find;
mod terminal_links;
mod terminal_images;
mod terminal_quickfix;
mod terminal_restore;
mod terminal_suggest;
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
mod workbench;
mod workspace;
mod wsl;

use eframe::egui;

fn main() -> eframe::Result {
    let child = std::env::var("HALO_ARGS").ok();
    if child.is_some() {
        unsafe { std::env::remove_var("HALO_ARGS") };
    }
    let console = child.is_none() && cli::attach_console();
    let mut args = match &child {
        Some(json) => serde_json::from_str(json).unwrap_or_default(),
        None => match cli::parse(std::env::args().skip(1), &|p| p.exists()) {
            Ok(cli::Parsed::Run(args)) => args,
            Ok(cli::Parsed::Help) => exit(cli::HELP, 0),
            Ok(cli::Parsed::Version) => exit(&cli::version(), 0),
            Err(error) => {
                eprintln!("halo: {error}\n");
                exit(cli::HELP, 2)
            }
        },
    };
    cli::resolve(&mut args);
    if let Some(text) = &mut args.stdin {
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), text);
    }
    let test = std::env::var_os("HALO_COMMANDS").is_some();
    let dir = settings::dir();
    if !test && child.is_none() {
        cli::allow_foreground();
        if !args.new_window && ipc::send(&dir, &args).is_ok() {
            return Ok(());
        }
        if args.wait || console {
            std::process::exit(launch(&args, &dir));
        }
    }
    windows::set_secondary(args.new_window);
    let server = if test { None } else { ipc::listen(&dir).ok().flatten() };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Halo")
        .with_app_id("halo")
        .with_inner_size([1180.0, 720.0])
        .with_min_inner_size([640.0, 400.0]);
    // macOS keeps its own traffic lights, resizing and fullscreen Spaces over Halo's titlebar
    viewport = if cfg!(target_os = "macos") {
        viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false)
    } else {
        viewport.with_decorations(false)
    };
    if test {
        viewport = viewport.with_active(false).with_position([-1177.0, 150.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: !test,
        persistence_path: test.then(|| std::env::temp_dir().join("halo-test.ron")),
        ..Default::default()
    };
    // stay cloaked until the first frames are drawn
    windows::conceal(true);
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(5));
        windows::conceal(false);
    });
    eframe::run_native(
        "Halo",
        options,
        Box::new(|cc| Ok(Box::new(app::Halo::new(cc, args, server)))),
    )
}

fn exit(text: &str, code: i32) -> ! {
    if code == 0 {
        print!("{text}");
    } else {
        eprint!("{text}");
    }
    std::process::exit(code)
}

fn launch(args: &cli::Args, dir: &std::path::Path) -> i32 {
    use std::process::{Command, Stdio};
    let first = cli::Args { wait: false, stdin: args.stdin.as_ref().map(|_| String::new()), ..args.clone() };
    let mut command = Command::new(std::env::current_exe().unwrap_or_else(|_| "halo".into()));
    command.env("HALO_ARGS", serde_json::to_string(&first).unwrap_or_default());
    command.stdin(if args.stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x8 | 0x200);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("halo: could not start a window: {error}");
            return 1;
        }
    };
    if let (Some(text), Some(mut pipe)) = (&args.stdin, child.stdin.take()) {
        let _ = std::io::Write::write_all(&mut pipe, text.as_bytes());
    }
    if !args.wait {
        return 0;
    }
    if args.new_window {
        let _ = child.wait();
        return 0;
    }
    let paths = args.paths.iter().map(|t| cli::Target { path: t.path.clone(), ..Default::default() }).collect();
    let follow = cli::Args { paths, wait: true, ..Default::default() };
    for _ in 0..300 {
        if ipc::send(dir, &follow).is_ok() || child.try_wait().is_ok_and(|status| status.is_some()) {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("halo: the new window did not answer");
    1
}
