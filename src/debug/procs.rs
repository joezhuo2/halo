use crate::command::Command;
use crate::search::Choice;
use eframe::egui;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

pub type Process = (u32, String, String);

const PICKS: [&str; 2] = ["${command:pickProcess}", "${command:pickMyProcess}"];

pub fn wants(value: &Value) -> bool {
    match value {
        Value::String(text) => PICKS.iter().any(|p| text.contains(p)),
        Value::Array(items) => items.iter().any(wants),
        Value::Object(map) => map.values().any(wants),
        _ => false,
    }
}

pub fn fill(value: &mut Value, pid: u32) {
    match value {
        Value::String(text) if PICKS.contains(&text.as_str()) => *value = pid.into(),
        Value::String(text) => PICKS.iter().for_each(|p| *text = text.replace(p, &pid.to_string())),
        Value::Array(items) => items.iter_mut().for_each(|item| fill(item, pid)),
        Value::Object(map) => map.values_mut().for_each(|item| fill(item, pid)),
        _ => {}
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_tasklist(text: &str) -> Vec<Process> {
    let row = |line: &str| {
        let mut fields = line.trim().trim_start_matches('"').split("\",\"");
        let name = fields.next()?.to_owned();
        Some((fields.next()?.parse().ok()?, name, String::new()))
    };
    text.lines().filter_map(row).collect()
}

#[cfg_attr(windows, allow(dead_code))]
fn split_pid(line: &str) -> Option<(u32, &str)> {
    let line = line.trim_start();
    let (pid, rest) = line.split_once(' ').unwrap_or((line, ""));
    Some((pid.parse().ok()?, rest.trim()))
}

#[cfg_attr(windows, allow(dead_code))]
pub fn parse_ps(names: &str, args: &str) -> Vec<Process> {
    let args: HashMap<u32, &str> = args.lines().filter_map(split_pid).collect();
    let row = |(pid, comm): (u32, &str)| {
        let name = if comm.starts_with('/') { comm.rsplit('/').next().unwrap_or(comm) } else { comm };
        (pid, name.to_owned(), args.get(&pid).copied().unwrap_or_default().to_owned())
    };
    names.lines().filter_map(split_pid).map(row).collect()
}

fn run(program: &str, args: &[&str]) -> String {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    command.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn processes() -> Vec<Process> {
    #[cfg(windows)]
    let mut list = parse_tasklist(&run("tasklist", &["/fo", "csv", "/nh"]));
    #[cfg(not(windows))]
    let mut list = parse_ps(&run("ps", &["-axo", "pid=,comm="]), &run("ps", &["-axo", "pid=,args="]));
    list.sort_by_cached_key(|p| (p.1.to_lowercase(), p.0));
    list
}

pub fn list(ctx: &egui::Context) -> Receiver<Vec<Process>> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(processes());
        ctx.request_repaint();
    });
    rx
}

pub fn choices(list: Vec<Process>) -> Vec<Choice> {
    list.into_iter().map(|(pid, name, args)| (format!("{name} ({pid})"), args.chars().take(200).collect(), Command::AttachProcess(pid))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_tasklist_and_ps_output() {
        let text = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"halo.exe\",\"4242\",\"Console\",\"1\",\"120,512 K\"\r\n\r\nINFO: junk\r\n";
        assert_eq!(parse_tasklist(text), [(0, "System Idle Process".into(), String::new()), (4242, "halo.exe".into(), String::new())]);
        let names = "    1 systemd\n  812 /Applications/Visual Studio Code.app/Contents/MacOS/Electron\n  913 kworker/0:1H\n bad line\n";
        let args = "    1 /sbin/init splash\n  812 /Applications/Visual Studio Code.app/Contents/MacOS/Electron --type=gpu\n";
        let found = parse_ps(names, args);
        assert_eq!(found[0], (1, "systemd".into(), "/sbin/init splash".into()));
        assert_eq!(found[1].1, "Electron");
        assert!(found[1].2.ends_with("Electron --type=gpu"));
        assert_eq!(found[2], (913, "kworker/0:1H".into(), String::new()));
        assert_eq!(found.len(), 3);
        let shown = choices(found);
        assert_eq!((shown[0].0.as_str(), &shown[0].2), ("systemd (1)", &Command::AttachProcess(1)));
    }

    #[test]
    fn substitutes_the_picked_process() {
        let mut config = json!({"processId": "${command:pickProcess}", "pid": "${command:pickMyProcess}", "args": ["--pid=${command:pickProcess}"], "other": "${command:other}", "n": 3});
        assert!(wants(&config) && !wants(&json!({"x": ["${command:other}", 1]})));
        fill(&mut config, 1234);
        assert_eq!(config, json!({"processId": 1234, "pid": 1234, "args": ["--pid=1234"], "other": "${command:other}", "n": 3}));
        assert!(!wants(&config));
    }
}
