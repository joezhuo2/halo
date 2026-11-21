use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const HELP: &str = "Halo - an ultra-lightweight native code editor

Usage: halo [options] [paths...]

  paths                     Files open as tabs, a folder opens as the folder.
                            file:line and file:line:column place the caret.

Options:
  -g, --goto <file:line[:column]>  Open a file at a line and column.
  -d, --diff <left> <right>        Compare two files.
  -a, --add <folder>               Add a folder to the running window.
  -n, --new-window                 Always start a new instance.
  -r, --reuse-window               Use the running instance.
  -w, --wait                       Return when the opened files are closed.
  -                                Read standard input into an untitled file.
  -h, --help                       Print this help and exit.
  -v, --version                    Print the version and exit.
  --                               End of options.
";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Args {
    pub paths: Vec<Target>,
    pub diff: Option<(PathBuf, PathBuf)>,
    pub add: Vec<PathBuf>,
    pub new_window: bool,
    pub reuse_window: bool,
    pub wait: bool,
    pub stdin: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Run(Args),
    Help,
    Version,
}

pub fn version() -> String {
    format!("halo {}\n", env!("CARGO_PKG_VERSION"))
}

pub fn parse(argv: impl IntoIterator<Item = String>, exists: &dyn Fn(&Path) -> bool) -> Result<Parsed, String> {
    let mut args = Args::default();
    let mut argv = argv.into_iter();
    let mut options = true;
    while let Some(arg) = argv.next() {
        let mut value = |name: &str| argv.next().ok_or_else(|| format!("option '{name}' needs a value"));
        match arg.as_str() {
            _ if !options || !arg.starts_with('-') => args.paths.push(target(&arg, exists)),
            "--" => options = false,
            "-" => args.stdin = Some(String::new()),
            "-h" | "--help" => return Ok(Parsed::Help),
            "-v" | "--version" => return Ok(Parsed::Version),
            "-n" | "--new-window" => args.new_window = true,
            "-r" | "--reuse-window" => args.reuse_window = true,
            "-w" | "--wait" => args.wait = true,
            "-g" | "--goto" => args.paths.push(target(&value(&arg)?, exists)),
            "-a" | "--add" => args.add.push(PathBuf::from(value(&arg)?)),
            "-d" | "--diff" => {
                let left = value(&arg)?;
                let right = value(&arg).map_err(|_| format!("option '{arg}' needs two files"))?;
                args.diff = Some((left.into(), right.into()));
            }
            _ => return Err(format!("unknown option '{arg}'")),
        }
    }
    if args.new_window && args.reuse_window {
        return Err("--new-window and --reuse-window cannot be used together".into());
    }
    Ok(Parsed::Run(args))
}

fn target(arg: &str, exists: &dyn Fn(&Path) -> bool) -> Target {
    let whole = Target { path: arg.into(), line: None, column: None };
    if exists(Path::new(arg)) {
        return whole;
    }
    let mut path = arg;
    let mut numbers = vec![];
    while numbers.len() < 2
        && let Some((head, tail)) = path.rsplit_once(':')
        && !head.is_empty()
        && !tail.is_empty()
        && let Ok(n) = tail.parse::<usize>()
    {
        numbers.insert(0, n);
        path = head;
    }
    Target { path: path.into(), line: numbers.first().copied(), column: numbers.get(1).copied() }
}

pub fn resolve(args: &mut Args) {
    let abs = |p: &Path| {
        let p = p.canonicalize().or_else(|_| std::path::absolute(p)).unwrap_or_else(|_| p.to_path_buf());
        crate::app::strip_unc(p)
    };
    args.paths.iter_mut().for_each(|t| t.path = abs(&t.path));
    args.add.iter_mut().for_each(|p| *p = abs(p));
    if let Some((left, right)) = &mut args.diff {
        (*left, *right) = (abs(left), abs(right));
    }
}

pub fn install(cmd: &crate::command::Command) {
    use crate::ui::{Severity, notify_with};
    if cfg!(windows) {
        return crate::ui::notify_info("The Halo installer already puts the 'halo' command in PATH. Open a new terminal to use it.");
    }
    let (Some(home), Ok(exe)) = (dirs::home_dir(), std::env::current_exe()) else {
        return crate::ui::notify_error("Halo could not find your home folder or its own executable.");
    };
    let link = home.join(".local").join("bin").join("halo");
    if *cmd == crate::command::Command::InstallInPath {
        let text = format!("Create the 'halo' command as a link at {} to {}?", link.display(), exe.display());
        return notify_with(Severity::Info, text, vec![("Install".into(), crate::command::Command::ConfirmInstallInPath)]);
    }
    match link_exe(&exe, &link) {
        Ok(()) if std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| link.parent() == Some(d.as_path()))) => {
            crate::ui::notify_info(format!("The 'halo' command is installed at {}.", link.display()))
        }
        Ok(()) => crate::ui::notify_warning(format!("The 'halo' command is installed at {}, but that folder is not in PATH yet.", link.display())),
        Err(error) => crate::ui::notify_error(format!("The 'halo' command could not be installed: {error}")),
    }
}

#[cfg(unix)]
fn link_exe(exe: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(link.parent().unwrap_or(Path::new("/")))?;
    if link.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(link)?;
    }
    std::os::unix::fs::symlink(exe, link)
}

#[cfg(not(unix))]
fn link_exe(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(all(windows, not(debug_assertions)))]
pub fn attach_console() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
    }
    let which = [-10i32, -11, -12].map(|n| n as u32);
    unsafe {
        let inherited = which.map(|n| GetStdHandle(n));
        let attached = AttachConsole(u32::MAX) != 0;
        for (n, handle) in which.into_iter().zip(inherited).filter(|(_, h)| *h != 0 && *h != -1) {
            SetStdHandle(n, handle);
        }
        attached
    }
}

#[cfg(all(unix, not(debug_assertions)))]
pub fn attach_console() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}

#[cfg(any(debug_assertions, not(any(windows, unix))))]
pub fn attach_console() -> bool {
    false
}

#[cfg(windows)]
pub fn allow_foreground() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(pid: u32) -> i32;
    }
    unsafe { AllowSetForegroundWindow(u32::MAX) };
}

#[cfg(not(windows))]
pub fn allow_foreground() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(argv: &[&str], existing: &[&str]) -> Args {
        match parse(argv.iter().map(|s| s.to_string()), &|p| existing.iter().any(|e| Path::new(e) == p)) {
            Ok(Parsed::Run(args)) => args,
            other => panic!("{other:?}"),
        }
    }

    fn at(path: &str, line: Option<usize>, column: Option<usize>) -> Target {
        Target { path: path.into(), line, column }
    }

    #[test]
    fn paths_and_positions() {
        assert_eq!(run(&["a.rs", "src"], &[]).paths, [at("a.rs", None, None), at("src", None, None)]);
        assert_eq!(run(&["a.rs:12"], &[]).paths, [at("a.rs", Some(12), None)]);
        assert_eq!(run(&["a.rs:12:3"], &[]).paths, [at("a.rs", Some(12), Some(3))]);
        assert_eq!(run(&["a.rs:x"], &[]).paths, [at("a.rs:x", None, None)]);
        assert_eq!(run(&["a.rs:"], &[]).paths, [at("a.rs:", None, None)]);
        assert_eq!(run(&["a:1:2:3"], &[]).paths, [at("a:1", Some(2), Some(3))]);
        assert_eq!(run(&["notes:12"], &["notes:12"]).paths, [at("notes:12", None, None)]);
        assert_eq!(run(&["notes:12:4"], &["notes:12"]).paths, [at("notes", Some(12), Some(4))]);
    }

    #[test]
    fn windows_drives_and_spaces() {
        assert_eq!(run(&[r"C:\a\b.rs:12:3"], &[]).paths, [at(r"C:\a\b.rs", Some(12), Some(3))]);
        assert_eq!(run(&[r"C:\a\b.rs:12"], &[]).paths, [at(r"C:\a\b.rs", Some(12), None)]);
        assert_eq!(run(&[r"C:\a\b.rs"], &[]).paths, [at(r"C:\a\b.rs", None, None)]);
        assert_eq!(run(&[r"C:\"], &[]).paths, [at(r"C:\", None, None)]);
        assert_eq!(run(&[r"C:\My Files\a b.rs:7"], &[]).paths, [at(r"C:\My Files\a b.rs", Some(7), None)]);
        assert_eq!(run(&["/home/me/my notes.txt:2:9"], &[]).paths, [at("/home/me/my notes.txt", Some(2), Some(9))]);
    }

    #[test]
    fn options() {
        let args = run(&["-g", "a.rs:3:4", "--goto", "b.rs:5"], &[]);
        assert_eq!(args.paths, [at("a.rs", Some(3), Some(4)), at("b.rs", Some(5), None)]);
        assert_eq!(run(&["-d", "l.rs", "r b.rs"], &[]).diff, Some(("l.rs".into(), "r b.rs".into())));
        assert_eq!(run(&["--diff", "l", "r"], &[]).diff, Some(("l".into(), "r".into())));
        assert_eq!(run(&["-a", "x", "--add", "y"], &[]).add, [PathBuf::from("x"), "y".into()]);
        let flags = run(&["-n", "-w"], &[]);
        assert!(flags.new_window && flags.wait && !flags.reuse_window);
        let flags = run(&["--reuse-window", "--wait"], &[]);
        assert!(flags.reuse_window && flags.wait && !flags.new_window);
        assert!(run(&["--new-window"], &[]).new_window);
        assert_eq!(run(&["-"], &[]).stdin, Some(String::new()));
        assert_eq!(run(&[], &[]), Args::default());
    }

    #[test]
    fn end_of_options() {
        let args = run(&["--", "-n", "--help", "-"], &[]);
        assert_eq!(args.paths, [at("-n", None, None), at("--help", None, None), at("-", None, None)]);
        assert!(!args.new_window && args.stdin.is_none());
    }

    #[test]
    fn help_version_and_errors() {
        let parse = |argv: &[&str]| parse(argv.iter().map(|s| s.to_string()), &|_| false);
        assert_eq!(parse(&["-h"]), Ok(Parsed::Help));
        assert_eq!(parse(&["a", "--help"]), Ok(Parsed::Help));
        assert_eq!(parse(&["-v"]), Ok(Parsed::Version));
        assert_eq!(parse(&["--version"]), Ok(Parsed::Version));
        assert_eq!(parse(&["--frobnicate"]), Err("unknown option '--frobnicate'".into()));
        assert_eq!(parse(&["-x"]), Err("unknown option '-x'".into()));
        assert_eq!(parse(&["-g"]), Err("option '-g' needs a value".into()));
        assert_eq!(parse(&["-d", "a"]), Err("option '-d' needs two files".into()));
        assert!(parse(&["-n", "-r"]).is_err());
    }
}
