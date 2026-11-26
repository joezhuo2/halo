use eframe::egui;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

const REPO: &str = env!("CARGO_PKG_REPOSITORY");

fn asset_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x64.exe"),
        ("linux", "x86_64") => Some("linux-x64.tar.gz"),
        ("macos", "aarch64") => Some("macos-arm64.tar.gz"),
        ("macos", "x86_64") => Some("macos-x64.tar.gz"),
        _ => None,
    }
}

fn numbers(version: &str) -> Vec<u64> {
    let core = version.trim_start_matches('v').split(['-', '+']).next().unwrap_or_default();
    core.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

fn pick(release: &Value, current: &str, suffix: &str) -> Option<(String, String, String)> {
    let version = release["tag_name"].as_str()?.trim_start_matches('v');
    if numbers(version) <= numbers(current) {
        return None;
    }
    let named = |asset: &&Value| asset["name"].as_str().is_some_and(|name| name.ends_with(suffix));
    let asset = release["assets"].as_array()?.iter().find(named)?;
    let url = asset["browser_download_url"].as_str().filter(|url| url.starts_with(&format!("{REPO}/releases/download/")))?;
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
    Some((version.to_owned(), url.to_owned(), digest.to_lowercase()))
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|e| format!("{program}: {e}"))?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(format!("{program}: {}", String::from_utf8_lossy(&output.stderr).trim())),
    }
}

fn sha256(file: &Path) -> Result<String, String> {
    let path = file.to_string_lossy();
    let out = match cfg!(windows) {
        true => run("certutil", &["-hashfile", &path, "SHA256"])?,
        false => run("shasum", &["-a", "256", &path]).or_else(|_| run("sha256sum", &[&path]))?,
    };
    let words = out.lines().map(|line| line.replace(' ', "")).chain(out.split_whitespace().map(str::to_owned));
    let hash = words.into_iter().find(|word| word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit()));
    hash.map(|hash| hash.to_lowercase()).ok_or_else(|| "the checksum of the download could not be computed".to_owned())
}

fn signature(file: &Path) -> Result<(String, String), String> {
    let path = file.to_string_lossy().replace('\'', "''");
    let script = format!("$s = Get-AuthenticodeSignature -LiteralPath '{path}'; [string]$s.Status; [string]$s.SignerCertificate.Subject");
    let out = run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])?;
    let mut lines = out.lines().map(str::trim);
    Ok((lines.next().unwrap_or_default().to_owned(), lines.next().unwrap_or_default().to_owned()))
}

fn same_publisher(running: (&str, &str), download: (&str, &str)) -> Result<(), String> {
    match (running, download) {
        (("Valid", _), (status, _)) if status != "Valid" => Err("the download does not have a valid signature".to_owned()),
        (("Valid", mine), (_, theirs)) if mine != theirs => Err(format!("the download is signed by {theirs}, not by {mine}")),
        _ => Ok(()),
    }
}

fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| e.to_string())?;
    std::fs::rename(fresh, exe).map_err(|e| {
        let _ = std::fs::rename(&old, exe);
        e.to_string()
    })
}

// shortcut: curl, tar and the system hash tool do the network and archive work, upgrade to crates if a platform lacks them
fn install(exe: &Path, version: &str, url: &str, digest: &str) -> Result<(), String> {
    let dir = exe.parent().ok_or("the program has no folder")?;
    let part = |kind: &str| dir.join(format!(".halo-{version}.{kind}"));
    let (download, fresh, unpacked) = (part("download"), part("new"), part("unpacked"));
    let done = (|| {
        run("curl", &["-fsSL", "--max-time", "600", "-A", "halo-updater", "-o", &download.to_string_lossy(), url])?;
        if sha256(&download)? != digest {
            return Err("the download does not match the checksum of the release".to_owned());
        }
        if url.ends_with(".tar.gz") {
            std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
            run("tar", &["-xzf", &download.to_string_lossy(), "-C", &unpacked.to_string_lossy()])?;
            std::fs::rename(unpacked.join("halo"), &fresh).map_err(|e| e.to_string())?;
        } else {
            std::fs::rename(&download, &fresh).map_err(|e| e.to_string())?;
        }
        if cfg!(windows) {
            let (running, fresh) = (signature(exe)?, signature(&fresh)?);
            same_publisher((&running.0, &running.1), (&fresh.0, &fresh.1))?;
        }
        swap(exe, &fresh)
    })();
    let _ = std::fs::remove_file(&download);
    let _ = std::fs::remove_file(&fresh);
    let _ = std::fs::remove_dir_all(&unpacked);
    done
}

fn update() -> Option<Result<String, String>> {
    let suffix = asset_suffix()?;
    let exe = std::env::current_exe().ok()?;
    let _ = std::fs::remove_file(exe.with_extension("old"));
    let api = REPO.replace("https://github.com/", "https://api.github.com/repos/") + "/releases/latest";
    let body = run("curl", &["-fsSL", "--max-time", "30", "-A", "halo-updater", "-H", "Accept: application/vnd.github+json", &api]).ok()?;
    let (version, url, digest) = pick(&serde_json::from_str(&body).ok()?, env!("CARGO_PKG_VERSION"), suffix)?;
    Some(install(&exe, &version, &url, &digest).map(|_| version))
}

pub fn start(ctx: &egui::Context) -> Receiver<Result<String, String>> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let Some(result) = update() {
            let _ = tx.send(result);
            ctx.request_repaint();
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_newer_releases_checks_hashes_and_swaps_files() {
        assert!(numbers("v0.10.0") > numbers("0.9.3") && numbers("1.0.0-beta") == numbers("1.0.0") && numbers("0.2") > numbers("0.1.9"));
        let asset = |name: &str, digest: &str| json!({"name": name, "browser_download_url": format!("{REPO}/releases/download/v0.2.0/{name}"), "digest": digest});
        let release = json!({"tag_name": "v0.2.0", "assets": [
            asset("halo-0.2.0-windows-x64-setup.exe", "sha256:AA"),
            asset("halo-0.2.0-windows-x64.exe", "sha256:BB"),
            {"name": "halo-0.2.0-linux-x64.tar.gz", "browser_download_url": "https://example.com/halo.tar.gz", "digest": "sha256:CC"},
        ]});
        let found = pick(&release, "0.1.1", "windows-x64.exe").unwrap();
        assert_eq!(found, ("0.2.0".to_owned(), format!("{REPO}/releases/download/v0.2.0/halo-0.2.0-windows-x64.exe"), "bb".to_owned()));
        assert_eq!(pick(&release, "0.2.0", "windows-x64.exe"), None);
        assert_eq!(pick(&release, "0.1.1", "linux-x64.tar.gz"), None);
        assert_eq!(pick(&release, "0.1.1", "macos-x64.tar.gz"), None);

        let dir = std::env::temp_dir().join(format!("halo-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (exe, fresh) = (dir.join("halo.exe"), dir.join(".halo-0.2.0.new"));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&fresh, "abc").unwrap();
        assert_eq!(sha256(&fresh).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        swap(&exe, &fresh).unwrap();
        assert_eq!((std::fs::read_to_string(&exe).unwrap().as_str(), std::fs::read_to_string(dir.join("halo.old")).unwrap().as_str()), ("abc", "old"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_signed_build_only_accepts_a_download_from_the_same_signer() {
        let me = "CN=Halo Publisher, O=Halo";
        assert_eq!(same_publisher(("NotSigned", ""), ("NotSigned", "")), Ok(()));
        assert_eq!(same_publisher(("NotSigned", ""), ("Valid", "CN=Someone")), Ok(()));
        assert_eq!(same_publisher(("HashMismatch", me), ("NotSigned", "")), Ok(()));
        assert_eq!(same_publisher(("Valid", me), ("Valid", me)), Ok(()));
        assert!(same_publisher(("Valid", me), ("NotSigned", "")).is_err());
        assert!(same_publisher(("Valid", me), ("HashMismatch", me)).is_err());
        assert!(same_publisher(("Valid", me), ("Valid", "CN=Someone Else")).unwrap_err().contains("CN=Someone Else"));
        #[cfg(windows)]
        assert_eq!(signature(&std::env::current_exe().unwrap()).unwrap(), ("NotSigned".to_owned(), String::new()));
    }
}
