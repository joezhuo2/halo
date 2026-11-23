use eframe::egui;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const LIMIT: usize = 24_000;
const PROMPT: &str = "Write a git commit message for the changes below. Reply with the message only: a summary line of at most \
    72 characters in the imperative mood, then, only if the change needs it, a blank line and a short body. No quotes, no code \
    fences, no commentary.\n\n";

pub struct Job {
    rx: Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn start(top: &Path, staged: bool, ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (top, flag, ctx) = (top.to_owned(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = diff(&top, staged).and_then(|diff| run(&top, invocation(&top)?, &format!("{PROMPT}{diff}"), &flag));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        Self { rx, cancel }
    }

    pub fn poll(&self) -> Option<Result<String, String>> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn diff(top: &Path, staged: bool) -> Result<String, String> {
    let base = ["diff", "--no-color", "--no-ext-diff", "--patch-with-stat"];
    let mut text = match staged {
        true => crate::git::read(top, &[&base[..], &["--cached"]].concat())?,
        false => crate::git::read(top, &[&base[..], &["HEAD"]].concat()).or_else(|_| crate::git::read(top, &base))?,
    };
    if !staged {
        for file in crate::git::read(top, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default().lines() {
            text.push_str(&format!("New untracked file: {file}\n"));
        }
    }
    if text.trim().is_empty() {
        return Err("There are no changes to describe.".into());
    }
    Ok(truncate(text))
}

fn truncate(mut text: String) -> String {
    if text.len() > LIMIT {
        let end = (0..=LIMIT).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(end);
        text.push_str("\n[diff truncated]\n");
    }
    text
}

fn clean(output: &str) -> String {
    let mut lines: Vec<&str> = output.trim().lines().collect();
    if lines.first().is_some_and(|l| l.starts_with("```")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.pop();
    }
    lines.join("\n").trim().to_owned()
}

fn invocation(top: &Path) -> Result<(String, Vec<String>), String> {
    if crate::wsl::parse(top).is_some() {
        let script = "if command -v claude >/dev/null; then claude -p; else codex exec -s read-only -; fi";
        return Ok(crate::wsl::wrap(Some(top), "bash", &["-lic".into(), script.into()]));
    }
    let clis: [(&str, &[&str]); 2] = [("claude", &["-p"]), ("codex", &["exec", "-s", "read-only", "-"])];
    let (cli, args, exe) = clis.into_iter().find_map(|(cli, args)| Some((cli, args, crate::ai::locate(cli)?)))
        .ok_or("Install the Claude or Codex CLI to generate commit messages.")?;
    let args = args.iter().map(|a| a.to_string());
    Ok(match !cfg!(windows) || exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
        true => (exe.to_string_lossy().into_owned(), args.collect()),
        false => ("cmd.exe".into(), ["/c".to_owned(), cli.to_owned()].into_iter().chain(args).collect()),
    })
}

fn run(top: &Path, (program, args): (String, Vec<String>), input: &str, cancel: &AtomicBool) -> Result<String, String> {
    let mut child = crate::terminal::hidden_command(&program)
        .args(&args)
        .current_dir(top)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run {program}: {e}"))?;
    let (mut stdin, input) = (child.stdin.take(), input.to_owned());
    std::thread::spawn(move || stdin.as_mut().map(|s| s.write_all(input.as_bytes())));
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = vec![];
            pipe.map(|mut p| p.read_to_end(&mut bytes));
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            return Err("Cancelled.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    crate::output::log("Git", format!("$ {program} {} (commit message) exit {}", args.join(" "), status.code().unwrap_or(-1)));
    if !status.success() {
        let reason = err.lines().chain(out.lines()).rfind(|l| !l.trim().is_empty()).unwrap_or("no output");
        return Err(format!("{program} failed: {reason}"));
    }
    Some(clean(&out)).filter(|m| !m.is_empty()).ok_or_else(|| format!("{program} returned an empty message."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_output_truncates_and_reads_changes() {
        assert_eq!(clean("\n```text\nFix the parser\n\nBody\n```\n"), "Fix the parser\n\nBody");
        assert_eq!(clean("  Add a test  "), "Add a test");
        let long = truncate("é".repeat(LIMIT));
        assert!(long.len() <= LIMIT + 20 && long.ends_with("[diff truncated]\n"));

        let dir = std::env::temp_dir().join(format!("halo-commit-ai-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        git(&["init", "-q"]);
        assert!(diff(&dir, false).unwrap_err().contains("no changes"));
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("b.txt"), "two\n").unwrap();
        git(&["add", "a.txt"]);
        let staged = diff(&dir, true).unwrap();
        assert!(staged.contains("+one") && !staged.contains("b.txt"));
        assert!(diff(&dir, false).unwrap().contains("New untracked file: b.txt"));

        let echo = if cfg!(windows) { ("cmd.exe".to_owned(), vec!["/c".to_owned(), "more".to_owned()]) } else { ("cat".to_owned(), vec![]) };
        assert_eq!(run(&dir, echo, "```\nAdd a\n```\n", &AtomicBool::new(false)).unwrap(), "Add a");
        let wait = if cfg!(windows) { ("ping".to_owned(), vec!["-n".to_owned(), "30".to_owned(), "127.0.0.1".to_owned()]) } else { ("sleep".to_owned(), vec!["30".to_owned()]) };
        assert_eq!(run(&dir, wait, "", &AtomicBool::new(true)).unwrap_err(), "Cancelled.");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
