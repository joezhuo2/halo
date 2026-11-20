use crate::command::Command;
use eframe::egui::{RichText, Ui};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

pub fn parse(path: &Path) -> Option<(String, String)> {
    let text = path.to_str()?;
    let lower = text.to_ascii_lowercase();
    let rest = [r"\\wsl.localhost\", r"\\wsl$\"].iter().find_map(|p| lower.starts_with(p).then(|| &text[p.len()..]))?;
    let (distro, tail) = rest.split_once('\\').unwrap_or((rest, ""));
    let linux = format!("/{}", tail.trim_end_matches('\\').replace('\\', "/"));
    (!distro.is_empty()).then(|| (distro.to_owned(), linux))
}

pub fn to_windows(distro: &str, linux: &str) -> PathBuf {
    PathBuf::from(format!(r"\\wsl.localhost\{distro}{}", linux.trim_end_matches('/').replace('/', "\\")))
}

pub fn shell_path(path: &Path) -> String {
    parse(path).map_or_else(|| path.to_string_lossy().into_owned(), |(_, linux)| linux)
}

pub fn local(dir: &Path, shell_path: &str) -> PathBuf {
    match parse(dir) {
        Some((distro, _)) => to_windows(&distro, shell_path),
        None => PathBuf::from(shell_path),
    }
}

pub fn wrap(dir: Option<&Path>, program: &str, args: &[String]) -> (String, Vec<String>) {
    let Some((distro, linux)) = dir.and_then(parse) else { return (program.to_owned(), args.to_vec()) };
    let head = ["-d", &distro, "--cd", &linux, "-e", program].map(str::to_owned);
    ("wsl.exe".to_owned(), head.into_iter().chain(args.iter().cloned()).collect())
}

pub fn mount(distro: &str, path: &Path) -> Option<PathBuf> {
    let (drive, rest) = path.to_str()?.split_once(":\\")?;
    (drive.len() == 1).then(|| to_windows(distro, &format!("/mnt/{}/{}", drive.to_lowercase(), rest.replace('\\', "/"))))
}

pub fn unmount(path: &Path) -> Option<PathBuf> {
    let (_, linux) = parse(path)?;
    let rest = linux.strip_prefix("/mnt/")?;
    let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
    (drive.len() == 1).then(|| PathBuf::from(format!("{}:\\{}", drive.to_uppercase(), tail.replace('/', "\\"))))
}

fn decode(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let text = String::from_utf16_lossy(&units);
    text.lines().map(|l| l.trim().trim_start_matches('\u{feff}').to_owned()).filter(|l| !l.is_empty()).collect()
}

fn distros() -> Vec<String> {
    let output = crate::terminal::hidden_command("wsl.exe").args(["-l", "-q"]).output();
    output.map(|o| decode(&o.stdout)).unwrap_or_default()
}

#[derive(Default)]
pub struct Wsl {
    distros: Option<Vec<String>>,
    rx: Option<Receiver<Vec<String>>>,
}

impl Wsl {
    pub fn menu(&mut self, ui: &mut Ui, folder: Option<&Path>, cmds: &mut Vec<Command>) {
        if let Some(list) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.distros = Some(list);
            self.rx = None;
        }
        if self.distros.is_none() && self.rx.is_none() {
            let (tx, rx) = channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = tx.send(distros());
                ctx.request_repaint();
            });
            self.rx = Some(rx);
        }
        ui.set_min_width(260.0);
        ui.label(RichText::new("WSL").size(11.0).weak());
        let mut pick = None;
        if let Some(windows) = folder.and_then(unmount)
            && ui.button("Reopen Folder in Windows").clicked()
        {
            pick = Some(Command::OpenPath(windows));
        }
        match &self.distros {
            None => {
                ui.label("Loading...");
            }
            Some(list) if list.is_empty() => {
                ui.label("No WSL distributions are installed.");
            }
            Some(list) => {
                for distro in list {
                    let mounted = folder.filter(|f| parse(f).is_none()).and_then(|f| mount(distro, f));
                    if let Some(path) = mounted
                        && ui.button(format!("Reopen Folder in {distro}")).clicked()
                    {
                        pick = Some(Command::OpenPath(path));
                    }
                    if ui.button(format!("Open Folder in {distro}...")).clicked() {
                        pick = Some(Command::OpenFolderIn(to_windows(distro, "/home")));
                    }
                }
            }
        }
        ui.separator();
        if ui.button("Refresh").clicked() {
            self.distros = None;
        }
        if let Some(command) = pick {
            cmds.push(command);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_paths_and_wraps_commands() {
        let unc = Path::new(r"\\wsl.localhost\Ubuntu\home\joe\proj");
        assert_eq!(parse(unc), Some(("Ubuntu".into(), "/home/joe/proj".into())));
        assert_eq!(parse(Path::new(r"\\WSL$\Debian\")), Some(("Debian".into(), "/".into())));
        assert_eq!(parse(Path::new(r"C:\Users\joe")), None);
        assert_eq!(to_windows("Ubuntu", "/home/joe/proj"), unc);
        assert_eq!(shell_path(&unc.join("src").join("a.rs")), "/home/joe/proj/src/a.rs");
        assert_eq!(shell_path(Path::new(r"C:\a b\c.py")), r"C:\a b\c.py");
        assert_eq!(local(unc, "/srv/x"), Path::new(r"\\wsl.localhost\Ubuntu\srv\x"));
        assert_eq!(local(Path::new(r"C:\x"), "C:/x/y"), Path::new("C:/x/y"));

        let args = ["status".to_owned()];
        let wrapped = wrap(Some(unc), "git", &args);
        assert_eq!(wrapped.0, "wsl.exe");
        assert_eq!(wrapped.1, ["-d", "Ubuntu", "--cd", "/home/joe/proj", "-e", "git", "status"]);
        assert_eq!(wrap(Some(Path::new(r"C:\x")), "git", &args), ("git".to_owned(), args.to_vec()));

        let mounted = mount("Ubuntu", Path::new(r"C:\Users\joe\proj")).unwrap();
        assert_eq!(mounted, Path::new(r"\\wsl.localhost\Ubuntu\mnt\c\Users\joe\proj"));
        assert_eq!(unmount(&mounted), Some(PathBuf::from(r"C:\Users\joe\proj")));
        assert_eq!(unmount(unc), None);

        let utf16: Vec<u8> = "Ubuntu-24.04\r\n\r\nDebian\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&utf16), ["Ubuntu-24.04", "Debian"]);
    }
}
