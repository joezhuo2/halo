use crate::command::Command;
use crate::notifications::Severity;
use std::path::{Path, PathBuf};

const KEEP: usize = 5;

fn dir() -> PathBuf {
    crate::settings::dir().join("crashes")
}

pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let _ = write(&dir(), info);
    }));
}

fn write(dir: &Path, info: &std::panic::PanicHookInfo) -> std::io::Result<()> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let thread = std::thread::current();
    let location = info.location().map(|l| l.to_string()).unwrap_or_default();
    let text = format!(
        "Halo {}\nOS: {} {}\nTime: {} UTC, {} seconds since 1970\nThread: {}\nPanic: {}\nLocation: {location}\n\nBacktrace:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::output::clock(now.as_secs()),
        now.as_secs(),
        thread.name().unwrap_or("<unnamed>"),
        info.payload_as_str().unwrap_or("<non-string payload>"),
        std::backtrace::Backtrace::force_capture(),
    );
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("crash-{:013}.log", now.as_millis())), text)?;
    for old in stale(&names(dir)) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

fn names(dir: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let mut names: Vec<String> = entries.filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with("crash-") && n.ends_with(".log")).collect();
    names.sort();
    names
}

fn stale(names: &[String]) -> &[String] {
    &names[..names.len().saturating_sub(KEEP)]
}

fn unreported<'a>(names: &'a [String], reported: &str) -> Option<&'a String> {
    names.last().filter(|name| name.as_str() > reported)
}

pub fn report() {
    let dir = dir();
    let names = names(&dir);
    let marker = dir.join("reported");
    let reported = std::fs::read_to_string(&marker).unwrap_or_default();
    let Some(name) = unreported(&names, reported.trim()) else { return };
    let _ = std::fs::write(&marker, name);
    let open = vec![("Open Crash Log".into(), Command::OpenPath(dir.join(name)))];
    crate::ui::notify_with(Severity::Warning, "Halo closed unexpectedly last time.", open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_five_newest_logs_and_reports_only_a_newer_one() {
        let dir = std::env::temp_dir().join(format!("halo-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for at in [3, 1, 7, 2, 5, 4, 6] {
            std::fs::write(dir.join(format!("crash-{at:013}.log")), "").unwrap();
        }
        std::fs::write(dir.join("reported"), "").unwrap();
        let all = names(&dir);
        assert_eq!(all.len(), 7);
        assert_eq!(stale(&all), &all[..2]);
        assert_eq!(stale(&all[..3]), &[] as &[String]);
        assert_eq!(unreported(&all, ""), Some(&"crash-0000000000007.log".to_owned()));
        assert_eq!(unreported(&all, "crash-0000000000007.log"), None);
        assert_eq!(unreported(&all, "crash-0000000000006.log"), all.last());
        assert_eq!(unreported(&[], ""), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
