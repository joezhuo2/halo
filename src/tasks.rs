use crate::app::Halo;
use crate::command::Command;
use crate::lsp::Diagnostic;
use crate::settings::Settings;
use crate::terminal::BottomPanel;
use crate::ui;
use eframe::egui;
use fancy_regex::{Captures, Regex};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(150);

#[derive(Clone, Default)]
struct Vars {
    workspace: PathBuf,
    file: Option<PathBuf>,
}

fn substitute(text: &str, vars: &Vars) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('}') else {
            rest = &rest[start..];
            break;
        };
        let name = &rest[start + 2..start + len];
        let file = || vars.file.clone().ok_or_else(|| format!("${{{name}}} needs an open file."));
        let name_of = |p: Option<&std::ffi::OsStr>| p.map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let value = match name {
            "workspaceFolder" => vars.workspace.to_string_lossy().into_owned(),
            "file" => file()?.to_string_lossy().into_owned(),
            "fileBasename" => name_of(file()?.file_name()),
            "fileBasenameNoExtension" => name_of(file()?.file_stem()),
            "fileDirname" => file()?.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "relativeFile" => {
                let file = file()?;
                file.strip_prefix(&vars.workspace).unwrap_or(&file).to_string_lossy().into_owned()
            }
            _ => match name.strip_prefix("env:") {
                Some(var) => std::env::var(var).unwrap_or_default(),
                None => rest[start..start + len + 1].to_owned(),
            },
        };
        out.push_str(&value);
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[derive(Clone)]
struct Pattern {
    re: Regex,
    groups: [usize; 10],
    looped: bool,
}

const FILE: usize = 0;
const LINE: usize = 1;
const COLUMN: usize = 2;
const END_LINE: usize = 3;
const END_COLUMN: usize = 4;
const SEVERITY: usize = 5;
const CODE: usize = 6;
const MESSAGE: usize = 7;
const LOCATION: usize = 8;

#[derive(Clone)]
struct Background {
    active: bool,
    begins: Regex,
    ends: Regex,
}

#[derive(Clone)]
struct Matcher {
    owner: String,
    patterns: Vec<Pattern>,
    base: Option<String>,
    severity: u8,
    background: Option<Background>,
}

fn severity_of(text: &str) -> u8 {
    let text = text.to_lowercase();
    if text.contains("warn") {
        2
    } else if text.contains("err") || text.contains("fatal") {
        1
    } else {
        3
    }
}

fn parse_matcher(v: &Value) -> Result<Matcher, String> {
    let list: Vec<&Value> = match &v["pattern"] {
        Value::Array(list) => list.iter().collect(),
        one @ Value::Object(_) => vec![one],
        _ => return Err("A problem matcher needs a pattern.".into()),
    };
    let names = ["file", "line", "column", "endLine", "endColumn", "severity", "code", "message", "location"];
    let mut patterns = vec![];
    for p in list {
        let source = p["regexp"].as_str().ok_or("A problem pattern needs a regexp.")?;
        let re = Regex::new(source).map_err(|e| format!("Invalid problem pattern {source}: {e}"))?;
        let mut groups = [0; 10];
        for (slot, name) in groups.iter_mut().zip(names) {
            *slot = p[name].as_u64().unwrap_or(0) as usize;
        }
        patterns.push(Pattern { re, groups, looped: p["loop"].as_bool().unwrap_or(false) });
    }
    if patterns.is_empty() {
        return Err("A problem matcher needs a pattern.".into());
    }
    let compile = |key: &str| v["background"][key].as_str().map(|s| Regex::new(s).map_err(|e| e.to_string())).transpose();
    let (begins, ends) = (compile("beginsPattern")?, compile("endsPattern")?);
    let never = || Regex::new(r"[^\s\S]").unwrap();
    let background = (begins.is_some() || ends.is_some()).then(|| Background {
        active: v["background"]["activeOnStart"].as_bool().unwrap_or(false),
        begins: begins.unwrap_or_else(never),
        ends: ends.unwrap_or_else(never),
    });
    let base = match &v["fileLocation"] {
        Value::Array(list) if list.first().and_then(Value::as_str) == Some("relative") => {
            Some(list.get(1).and_then(Value::as_str).unwrap_or("${workspaceFolder}").to_owned())
        }
        Value::String(s) if s == "relative" => Some("${workspaceFolder}".to_owned()),
        _ => None,
    };
    Ok(Matcher {
        owner: v["owner"].as_str().unwrap_or("external").to_owned(),
        patterns,
        base,
        severity: v["severity"].as_str().map_or(1, severity_of),
        background,
    })
}

fn builtin(name: &str) -> Option<Matcher> {
    let one = |regexp: &str, groups: Value| {
        let mut p = groups;
        p["regexp"] = json!(regexp);
        p
    };
    let relative = json!(["relative", "${workspaceFolder}"]);
    let tsc = r"^([^\s].*)[\(:](\d+)[,:](\d+)(?:\):\s+|\s+-\s+)(error|warning|info)\s+(TS\d+)\s*:\s*(.*)$";
    let tsc_groups = json!({"file": 1, "line": 2, "column": 3, "severity": 4, "code": 5, "message": 6});
    let spec = match name.trim_start_matches('$') {
        "rustc" => json!({"owner": "rustc", "fileLocation": relative, "pattern": [
            one(r"^(warning|warn|error)(\[(.*)\])?: (.*)$", json!({"severity": 1, "code": 3, "message": 4})),
            one(r"^[\s\->=]*(.*?):(\d*):(\d*)\s*$", json!({"file": 1, "line": 2, "column": 3})),
        ]}),
        "tsc" => json!({"owner": "typescript", "fileLocation": relative, "pattern": one(tsc, tsc_groups)}),
        "tsc-watch" => json!({
            "owner": "typescript", "fileLocation": relative, "pattern": one(tsc, tsc_groups),
            "background": {
                "activeOnStart": true,
                "beginsPattern": r"^\s*(?:message TS6032:|\[?\d{1,2}:\d{1,2}:\d{1,2}(?: AM| PM)?\]? (?:- )?File change detected\. Starting incremental compilation\.\.\.)",
                "endsPattern": r"^\s*(?:message TS6042:|\[?\d{1,2}:\d{1,2}:\d{1,2}(?: AM| PM)?\]? (?:- )?Found \d+ errors?\. Watching for file changes\.)",
            }}),
        "eslint-stylish" => json!({"owner": "eslint", "pattern": [
            one(r"^([^\s].*)$", json!({"file": 1})),
            one(r"^\s+(\d+):(\d+)\s+(error|warning|info)\s+(.+?)(?:\s\s+(.*))?$", json!({"line": 1, "column": 2, "severity": 3, "message": 4, "code": 5, "loop": true})),
        ]}),
        "eslint-compact" => json!({"owner": "eslint", "pattern": one(
            r"^(.+):\sline\s(\d+),\scol\s(\d+),\s(Error|Warning|Info)\s-\s(.+)\s\((.+)\)$",
            json!({"file": 1, "line": 2, "column": 3, "severity": 4, "message": 5, "code": 6}))}),
        "gcc" => json!({"owner": "cpp", "fileLocation": relative, "pattern": one(
            r"^(.*):(\d+):(\d+):\s+(warning|error|fatal error):\s+(.*)$",
            json!({"file": 1, "line": 2, "column": 3, "severity": 4, "message": 5}))}),
        "msCompile" => json!({"owner": "msCompile", "pattern": one(
            r"^\s*(?:\s*\d+>)?\s*(\S.*?)\((\d+|\d+,\d+|\d+,\d+,\d+,\d+)\)\s*:\s+((?:fatal +)?error|warning|info)\s+(\w{1,2}\d+)\s*:\s*(.*)$",
            json!({"file": 1, "location": 2, "severity": 3, "code": 4, "message": 5}))}),
        "go" => json!({"owner": "go", "fileLocation": relative, "pattern": one(
            r"^\s*(\S.*?\.go):(\d+)(?::(\d+))?:\s+(.*)$",
            json!({"file": 1, "line": 2, "column": 3, "message": 4}))}),
        _ => return None,
    };
    parse_matcher(&spec).ok()
}

#[derive(Clone, Debug, PartialEq)]
struct Found {
    owner: String,
    file: String,
    line: usize,
    column: usize,
    end: Option<(usize, usize)>,
    severity: u8,
    code: Option<String>,
    message: String,
}

#[derive(Debug, PartialEq)]
enum Event {
    Found(Found),
    Begin,
    End,
}

#[derive(Clone, Default)]
struct Fields {
    file: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    end_line: Option<usize>,
    end_column: Option<usize>,
    severity: Option<u8>,
    code: Option<String>,
    message: Option<String>,
}

impl Fields {
    fn extract(p: &Pattern, c: &Captures) -> Self {
        let text = |slot: usize| {
            let group = p.groups[slot];
            (group > 0).then(|| c.get(group)).flatten().map(|m| m.as_str().to_owned()).filter(|s| !s.is_empty())
        };
        let number = |slot| text(slot).and_then(|s| s.trim().parse().ok());
        let mut fields = Self {
            file: text(FILE),
            line: number(LINE),
            column: number(COLUMN),
            end_line: number(END_LINE),
            end_column: number(END_COLUMN),
            severity: text(SEVERITY).map(|s| severity_of(&s)),
            code: text(CODE),
            message: text(MESSAGE),
        };
        if let Some(location) = text(LOCATION) {
            let n: Vec<usize> = location.split([',', ':']).filter_map(|s| s.trim().parse().ok()).collect();
            fields.line = n.first().copied();
            fields.column = n.get(1).copied();
            fields.end_line = n.get(2).copied();
            fields.end_column = n.get(3).copied();
        }
        fields
    }

    fn over(&mut self, f: &Fields) {
        self.file = f.file.clone().or(self.file.take());
        self.line = f.line.or(self.line);
        self.column = f.column.or(self.column);
        self.end_line = f.end_line.or(self.end_line);
        self.end_column = f.end_column.or(self.end_column);
        self.severity = f.severity.or(self.severity);
        self.code = f.code.clone().or(self.code.take());
        self.message = f.message.clone().or(self.message.take());
    }
}

struct Scan {
    matcher: Matcher,
    index: usize,
    fields: Vec<Fields>,
    active: bool,
}

impl Scan {
    fn new(mut matcher: Matcher, background: bool) -> Self {
        if !background {
            matcher.background = None;
        }
        let active = matcher.background.as_ref().is_none_or(|b| b.active);
        let fields = vec![Fields::default(); matcher.patterns.len()];
        Self { matcher, index: 0, fields, active }
    }

    fn reset(&mut self) {
        self.index = 0;
        self.fields.fill(Fields::default());
    }

    fn feed(&mut self, raw: &str) -> Vec<Event> {
        let line = strip_ansi(raw);
        let mut events = vec![];
        if let Some(bg) = &self.matcher.background {
            let (begins, ends) = (bg.begins.is_match(&line).unwrap_or(false), bg.ends.is_match(&line).unwrap_or(false));
            if begins {
                self.active = true;
                self.reset();
                events.push(Event::Begin);
            }
            if ends {
                self.active = false;
                events.push(Event::End);
                return events;
            }
        }
        if self.active {
            events.extend(self.find(&line).map(Event::Found));
        }
        events
    }

    fn find(&mut self, line: &str) -> Option<Found> {
        let last = self.matcher.patterns.len() - 1;
        let mut at = self.index;
        loop {
            let pattern = &self.matcher.patterns[at];
            if let Ok(Some(c)) = pattern.re.captures(line) {
                self.fields[at] = Fields::extract(pattern, &c);
                self.fields[at + 1..].fill(Fields::default());
                if at < last {
                    self.index = at + 1;
                    return None;
                }
                self.index = if pattern.looped { last } else { 0 };
                return self.found();
            }
            if at == 0 {
                self.index = 0;
                return None;
            }
            (at, self.index) = (0, 0);
        }
    }

    fn found(&mut self) -> Option<Found> {
        let mut all = Fields::default();
        self.fields.iter().for_each(|f| all.over(f));
        let found = Found {
            owner: self.matcher.owner.clone(),
            file: all.file?,
            line: all.line.unwrap_or(1),
            column: all.column.unwrap_or(1),
            end: all.end_column.map(|c| (all.end_line.or(all.line).unwrap_or(1), c)),
            severity: all.severity.unwrap_or(self.matcher.severity),
            code: all.code,
            message: all.message.unwrap_or_default(),
        };
        if self.matcher.patterns.last().is_some_and(|p| !p.looped) {
            self.reset();
        }
        Some(found)
    }
}

fn fresh(old: &[String], new: &[String]) -> usize {
    (0..=old.len()).find_map(|skip| {
        let keep = old.len() - skip;
        (new.len() >= keep && old[skip..] == new[..keep]).then_some(keep)
    })
    .unwrap_or(0)
}

fn locate(file: &str, base: Option<&str>, cwd: &Path, vars: &Vars) -> PathBuf {
    let root = base.and_then(|b| substitute(b, vars).ok()).map_or_else(|| cwd.to_path_buf(), PathBuf::from);
    root.join(file.trim()).components().collect()
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Origin {
    Config,
    Settings,
    Detected,
}

#[derive(Clone)]
struct Def {
    label: String,
    command: String,
    args: Vec<String>,
    process: bool,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
    group: Option<String>,
    default: bool,
    depends: Vec<String>,
    sequence: bool,
    background: bool,
    matchers: Vec<Matcher>,
    reveal: bool,
    origin: Origin,
    raw: Value,
}

fn os_key() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn merge(into: &mut Value, over: &Value) {
    match (into, over) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (into, over) => *into = over.clone(),
    }
}

fn parse_def(raw: &Value, origin: Origin, os: &str) -> Result<Def, String> {
    let mut v = raw.clone();
    if let Some(over) = raw.get(os).filter(|o| o.is_object()) {
        merge(&mut v, over);
    }
    let text = |k: &str| v[k].as_str().map(str::to_owned);
    let mut args: Vec<String> =
        v["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().or_else(|| a["value"].as_str())).map(str::to_owned).collect();
    let kind = text("type").unwrap_or_else(|| "shell".into());
    let (label, command, process) = match kind.as_str() {
        "npm" => {
            let script = text("script").ok_or("An npm task needs a script.")?;
            args.splice(0..0, ["run".to_owned(), script.clone()]);
            (text("label").unwrap_or(format!("npm: {script}")), "npm".to_owned(), false)
        }
        "cargo" => {
            let sub = text("command").ok_or("A cargo task needs a command.")?;
            args.insert(0, sub.clone());
            (text("label").unwrap_or(format!("cargo {sub}")), "cargo".to_owned(), false)
        }
        _ => (text("label").ok_or("A task needs a label.")?, text("command").unwrap_or_default(), kind == "process"),
    };
    let depends: Vec<String> = match &v["dependsOn"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(list) => list.iter().filter_map(|d| d.as_str().or_else(|| d["task"].as_str())).map(str::to_owned).collect(),
        other => other["task"].as_str().map(str::to_owned).into_iter().collect(),
    };
    if command.is_empty() && depends.is_empty() {
        return Err(format!("The task '{label}' has no command."));
    }
    let (group, default) = match &v["group"] {
        Value::String(s) => (Some(s.clone()), false),
        Value::Object(o) => (o.get("kind").and_then(Value::as_str).map(str::to_owned), o.get("isDefault").and_then(Value::as_bool).unwrap_or(false)),
        _ => (None, false),
    };
    let matchers = match &v["problemMatcher"] {
        Value::Null => vec![],
        Value::Array(list) => list.iter().map(matcher_of).collect::<Result<_, _>>()?,
        one => vec![matcher_of(one)?],
    };
    let env = v["options"]["env"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, val)| (k.clone(), val.as_str().map_or_else(|| val.to_string(), str::to_owned)))
        .collect();
    Ok(Def {
        cwd: v["options"]["cwd"].as_str().map(str::to_owned),
        env,
        group,
        default,
        sequence: v["dependsOrder"].as_str() == Some("sequence"),
        background: v["isBackground"].as_bool().unwrap_or(false),
        reveal: !matches!(v["presentation"]["reveal"].as_str(), Some("never" | "silent")),
        matchers,
        depends,
        label,
        command,
        args,
        process,
        origin,
        raw: raw.clone(),
    })
}

fn matcher_of(v: &Value) -> Result<Matcher, String> {
    match v.as_str() {
        Some(name) => builtin(name).ok_or_else(|| format!("Unknown problem matcher {name}.")),
        None => parse_matcher(v),
    }
}

fn defs_from(text: &str, origin: Origin, os: &str) -> (Vec<Def>, Vec<String>) {
    let doc = match crate::jsonc::parse(text) {
        Ok(doc) => doc,
        Err(e) => return (vec![], vec![format!("tasks.json could not be read: {e}")]),
    };
    let (mut defs, mut errors) = (vec![], vec![]);
    for raw in doc["tasks"].as_array().into_iter().flatten() {
        match parse_def(raw, origin, os) {
            Ok(def) => defs.push(def),
            Err(e) => errors.push(format!("tasks.json: {e}")),
        }
    }
    (defs, errors)
}

fn package_manager(root: &Path) -> &'static str {
    let has = |name: &str| root.join(name).exists();
    if has("bun.lockb") || has("bun.lock") {
        "bun"
    } else if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else {
        "npm"
    }
}

fn detect(root: &Path) -> Vec<Value> {
    let mut out = vec![];
    let package = std::fs::read_to_string(root.join("package.json")).ok().and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok());
    if let Some(scripts) = package.as_ref().and_then(|p| p["scripts"].as_object()) {
        let pm = package_manager(root);
        for name in scripts.keys() {
            let mut task = json!({"label": format!("npm: {name}"), "type": "shell", "command": format!("{pm} run {name}")});
            if name == "build" || name == "test" {
                task["group"] = json!(name);
            }
            out.push(task);
        }
    }
    if root.join("Cargo.toml").exists() {
        for sub in ["build", "check", "test", "run", "clippy"] {
            let mut task = json!({"label": format!("cargo {sub}"), "type": "shell", "command": "cargo", "args": [sub], "problemMatcher": ["$rustc"]});
            if sub == "build" || sub == "test" {
                task["group"] = json!(sub);
            }
            out.push(task);
        }
    }
    out
}

fn load(root: &Path, settings: &Settings, os: &str) -> (Vec<Def>, Vec<String>) {
    let (mut defs, mut errors) = match std::fs::read_to_string(tasks_path(root)) {
        Ok(text) => defs_from(&text, Origin::Config, os),
        Err(_) => (vec![], vec![]),
    };
    let from_settings = settings.tasks.iter().map(|t| (json!({"label": t.name, "type": "shell", "command": t.command}), Origin::Settings));
    let detected = detect(root).into_iter().map(|raw| (raw, Origin::Detected));
    for (raw, origin) in from_settings.chain(detected) {
        match parse_def(&raw, origin, os) {
            Ok(def) => defs.push(def),
            Err(e) => errors.push(e),
        }
    }
    let mut seen = std::collections::HashSet::new();
    defs.retain(|d| seen.insert(d.label.clone()));
    (defs, errors)
}

pub fn tasks_path(root: &Path) -> PathBuf {
    root.join(".vscode").join("tasks.json")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Status {
    Waiting,
    Running,
    Ready,
    Done(bool),
}

struct Node {
    label: String,
    deps: Vec<usize>,
    sequence: bool,
    runs: bool,
    status: Status,
}

struct Sched {
    nodes: Vec<Node>,
}

impl Sched {
    fn new(defs: &[Def], root: &str) -> Result<Self, String> {
        let mut sched = Self { nodes: vec![] };
        sched.add(defs, root, &mut vec![])?;
        Ok(sched)
    }

    fn add(&mut self, defs: &[Def], label: &str, stack: &mut Vec<String>) -> Result<usize, String> {
        if let Some(at) = stack.iter().position(|l| l == label) {
            return Err(format!("Task dependency cycle: {} -> {label}", stack[at..].join(" -> ")));
        }
        if let Some(index) = self.nodes.iter().position(|n| n.label == label) {
            return Ok(index);
        }
        let def = defs.iter().find(|d| d.label == label).ok_or_else(|| format!("There is no task named '{label}'."))?;
        let index = self.nodes.len();
        self.nodes.push(Node { label: label.to_owned(), deps: vec![], sequence: def.sequence, runs: !def.command.is_empty(), status: Status::Waiting });
        stack.push(label.to_owned());
        for dep in &def.depends {
            let child = self.add(defs, dep, stack)?;
            self.nodes[index].deps.push(child);
        }
        stack.pop();
        Ok(index)
    }

    fn satisfied(&self, index: usize) -> bool {
        matches!(self.nodes[index].status, Status::Ready | Status::Done(true))
    }

    fn release(&self, index: usize, out: &mut [bool]) {
        if std::mem::replace(&mut out[index], true) {
            return;
        }
        let node = &self.nodes[index];
        if node.status != Status::Waiting {
            return;
        }
        for &dep in &node.deps {
            self.release(dep, out);
            if node.sequence && !self.satisfied(dep) {
                break;
            }
        }
    }

    fn start(&mut self) -> Vec<usize> {
        let mut launch = vec![];
        while !self.nodes.iter().any(|n| n.status == Status::Done(false)) {
            let mut released = vec![false; self.nodes.len()];
            self.release(0, &mut released);
            let next = (0..self.nodes.len()).find(|&i| {
                released[i] && self.nodes[i].status == Status::Waiting && self.nodes[i].deps.iter().all(|&d| self.satisfied(d))
            });
            let Some(i) = next else { break };
            if self.nodes[i].runs {
                self.nodes[i].status = Status::Running;
                launch.push(i);
            } else {
                self.nodes[i].status = Status::Done(true);
            }
        }
        launch
    }

    fn ready(&mut self, index: usize) {
        if self.nodes[index].status == Status::Running {
            self.nodes[index].status = Status::Ready;
        }
    }

    fn finish(&mut self, index: usize, ok: bool) {
        self.nodes[index].status = Status::Done(ok);
    }

    fn outcome(&self) -> Option<bool> {
        if self.satisfied(0) {
            Some(true)
        } else {
            self.nodes.iter().any(|n| n.status == Status::Done(false)).then_some(false)
        }
    }

    fn busy(&self) -> bool {
        self.nodes.iter().any(|n| matches!(n.status, Status::Running | Status::Ready))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub error: Option<String>,
}

struct Exec {
    node: usize,
    label: String,
    pane: u64,
    cwd: PathBuf,
    scans: Vec<Scan>,
    seen: Vec<String>,
    polled: Instant,
}

struct Run {
    sched: Sched,
    vars: Vars,
    execs: Vec<Exec>,
    waiters: Vec<Sender<Outcome>>,
    told: bool,
    error: Option<String>,
}

struct Problem {
    owner: String,
    path: PathBuf,
    diagnostic: Diagnostic,
}

#[derive(Default)]
pub struct Tasks {
    defs: Vec<Def>,
    errors: Vec<String>,
    runs: Vec<Run>,
    last: Option<String>,
    problems: Vec<Problem>,
}

fn quote(arg: &str) -> String {
    if arg.is_empty() || (arg.contains(char::is_whitespace) && !arg.starts_with(['"', '\''])) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

fn launch_args(process: bool, command: &str, args: &[String]) -> (String, Vec<String>) {
    let quoted: Vec<String> = args.iter().map(|a| quote(a)).collect();
    if process {
        return (command.to_owned(), if cfg!(windows) { quoted } else { args.to_vec() });
    }
    let line = std::iter::once(command.to_owned()).chain(quoted).collect::<Vec<_>>().join(" ");
    if cfg!(windows) {
        ("cmd.exe".to_owned(), vec!["/d".into(), "/s".into(), "/c".into(), format!("\"{line}\"")])
    } else {
        ("sh".to_owned(), vec!["-c".into(), line])
    }
}

fn with_default_build(text: Option<&str>, label: &str, raw: &Value, os: &str) -> Result<String, String> {
    let mut doc = match text {
        Some(text) => crate::jsonc::parse(text).map_err(|e| format!("tasks.json could not be read, so it was not changed: {e}"))?,
        None => json!({"version": "2.0.0", "tasks": []}),
    };
    let object = doc.as_object_mut().ok_or("tasks.json must contain an object.")?;
    let tasks = object.entry("tasks").or_insert_with(|| json!([]));
    let tasks = tasks.as_array_mut().ok_or("\"tasks\" in tasks.json must be an array.")?;
    for task in tasks.iter_mut() {
        let default = task["group"]["isDefault"].as_bool() == Some(true) && task["group"]["kind"].as_str() == Some("build");
        if default {
            task["group"] = json!("build");
        }
    }
    let group = json!({"kind": "build", "isDefault": true});
    match tasks.iter_mut().find(|t| parse_def(t, Origin::Config, os).is_ok_and(|d| d.label == label)) {
        Some(task) => task["group"] = group,
        None => {
            let mut task = raw.clone();
            task["group"] = group;
            tasks.push(task);
        }
    }
    serde_json::to_string_pretty(&doc).map(|s| s + "\n").map_err(|e| e.to_string())
}

impl Tasks {
    pub fn refresh(&mut self, root: Option<&Path>, settings: &Settings) {
        let Some(root) = root else { return };
        let (defs, errors) = load(root, settings, os_key());
        if errors != self.errors {
            errors.iter().for_each(|e| ui::notify(e.clone()));
        }
        (self.defs, self.errors) = (defs, errors);
    }

    fn def(&self, label: &str) -> Option<&Def> {
        self.defs.iter().find(|d| d.label == label)
    }

    fn picks(&self, kind: Option<&str>) -> Vec<crate::search::Choice> {
        let detail = |d: &Def| match d.origin {
            Origin::Config => "tasks.json",
            Origin::Settings => "settings.json",
            Origin::Detected => "detected",
        };
        let listed = self.defs.iter().filter(|d| kind.is_none_or(|k| d.group.as_deref() == Some(k)));
        listed.map(|d| (d.label.clone(), detail(d).to_owned(), Command::RunTaskLabel(d.label.clone()))).collect()
    }

    fn running(&self) -> Vec<(String, u64)> {
        self.runs.iter().flat_map(|r| &r.execs).map(|e| (e.label.clone(), e.pane)).collect()
    }

    pub fn running_count(&self) -> usize {
        self.runs.iter().map(|r| r.execs.len()).sum()
    }

    pub fn counts(&self) -> (usize, usize) {
        let count = |severity| self.problems.iter().filter(|p| p.diagnostic.severity == severity).count();
        (count(1), count(2))
    }

    pub fn files(&self) -> Vec<(PathBuf, Vec<Diagnostic>)> {
        let mut files: BTreeMap<PathBuf, Vec<Diagnostic>> = BTreeMap::new();
        for p in &self.problems {
            files.entry(p.path.clone()).or_default().push(p.diagnostic.clone());
        }
        files.into_iter().map(|(path, mut items)| (path, { items.sort_by_key(|d| d.start); items })).collect()
    }

    pub fn run_label(
        &mut self,
        label: &str,
        root: Option<&Path>,
        file: Option<PathBuf>,
        settings: &Settings,
        bottom: &mut BottomPanel,
        ctx: &egui::Context,
    ) -> Result<Receiver<Outcome>, String> {
        let root = root.ok_or("Open a folder to run tasks.")?;
        self.refresh(Some(root), settings);
        let sched = Sched::new(&self.defs, label)?;
        for node in &sched.nodes {
            self.terminate(&node.label, bottom);
        }
        let (tx, rx) = channel();
        self.last = Some(label.to_owned());
        let vars = Vars { workspace: root.to_path_buf(), file };
        let mut run = Run { sched, vars, execs: vec![], waiters: vec![tx], told: false, error: None };
        self.step(&mut run, bottom, ctx);
        self.runs.push(run);
        Ok(rx)
    }

    pub fn terminate(&mut self, label: &str, bottom: &mut BottomPanel) {
        for run in &mut self.runs {
            let stopped: Vec<(usize, u64)> = run.execs.iter().filter(|e| e.label == label).map(|e| (e.node, e.pane)).collect();
            for (node, pane) in stopped {
                bottom.kill_task(pane);
                run.sched.finish(node, false);
                run.error = Some(format!("The task '{label}' was terminated."));
            }
            run.execs.retain(|e| e.label != label);
        }
    }

    pub fn poll(&mut self, bottom: &mut BottomPanel, ctx: &egui::Context) {
        if self.runs.is_empty() {
            return;
        }
        let mut runs = std::mem::take(&mut self.runs);
        for run in &mut runs {
            self.step(run, bottom, ctx);
        }
        runs.retain(|r| !r.told || r.sched.busy());
        runs.append(&mut self.runs);
        self.runs = runs;
        if !self.runs.is_empty() {
            ctx.request_repaint_after(POLL);
        }
    }

    fn step(&mut self, run: &mut Run, bottom: &mut BottomPanel, ctx: &egui::Context) {
        let mut exited = vec![];
        let mut ready = vec![];
        for exec in &mut run.execs {
            let Some(pane) = bottom.task_pane(exec.pane) else {
                exited.push((exec.node, exec.pane, None));
                continue;
            };
            let code = pane.exit_code();
            if code.is_none() && exec.polled.elapsed() < POLL {
                continue;
            }
            exec.polled = Instant::now();
            let (lines, complete) = pane.snapshot(code.is_some());
            let new = &lines[..complete];
            for line in &new[fresh(&exec.seen, new)..] {
                for scan in &mut exec.scans {
                    for event in scan.feed(line) {
                        match event {
                            Event::Found(found) => add_problem(&mut self.problems, &found, scan.matcher.base.as_deref(), &exec.cwd, &run.vars),
                            Event::Begin => self.problems.retain(|p| p.owner != scan.matcher.owner),
                            Event::End => ready.push(exec.node),
                        }
                    }
                }
            }
            exec.seen = new.to_vec();
            if code.is_some() {
                exited.push((exec.node, exec.pane, code));
            }
        }
        for node in ready {
            run.sched.ready(node);
        }
        for (node, pane, code) in exited {
            let label = run.sched.nodes[node].label.clone();
            let ok = code == Some(0);
            match code {
                Some(code) => crate::output::log("Tasks", format!("'{label}' ended with exit code {code}")),
                None => crate::output::log("Tasks", format!("'{label}' was terminated")),
            }
            if code.is_some() {
                bottom.finish_task(pane, &label, code);
            }
            if !ok {
                run.error = Some(match code {
                    Some(code) => format!("The task '{label}' terminated with exit code {code}."),
                    None => format!("The task '{label}' was terminated."),
                });
            }
            run.sched.finish(node, ok);
            run.execs.retain(|e| e.node != node);
        }
        loop {
            let nodes = run.sched.start();
            if nodes.is_empty() {
                break;
            }
            for node in nodes {
                crate::output::log("Tasks", format!("Started '{}'", run.sched.nodes[node].label));
                if let Err(error) = self.launch(run, node, bottom, ctx) {
                    crate::output::log("Tasks", format!("[error] {error}"));
                    ui::notify(error.clone());
                    run.error = Some(error);
                    run.sched.finish(node, false);
                }
            }
        }
        if !run.told && let Some(ok) = run.sched.outcome() {
            run.told = true;
            let error = (!ok).then(|| run.error.clone().unwrap_or_else(|| "The task failed.".to_owned()));
            run.waiters.drain(..).for_each(|w| drop(w.send(Outcome { ok, error: error.clone() })));
        }
    }

    fn launch(&mut self, run: &mut Run, node: usize, bottom: &mut BottomPanel, ctx: &egui::Context) -> Result<(), String> {
        let label = run.sched.nodes[node].label.clone();
        let def = self.def(&label).cloned().ok_or_else(|| format!("There is no task named '{label}'."))?;
        let vars = &run.vars;
        let command = substitute(&def.command, vars)?;
        let args = def.args.iter().map(|a| substitute(a, vars)).collect::<Result<Vec<_>, _>>()?;
        let env = def.env.iter().map(|(k, v)| Ok((k.clone(), substitute(v, vars)?))).collect::<Result<BTreeMap<_, _>, String>>()?;
        let cwd = match &def.cwd {
            Some(cwd) => vars.workspace.join(substitute(cwd, vars)?),
            None => vars.workspace.clone(),
        };
        let (program, args) = launch_args(def.process, &command, &args);
        let owners: Vec<&str> = def.matchers.iter().map(|m| m.owner.as_str()).collect();
        self.problems.retain(|p| !owners.contains(&p.owner.as_str()));
        let pane = bottom.run_task(&label, (program.as_str(), args.as_slice()), &env, Some(&cwd), def.reveal, ctx)?;
        let scans: Vec<Scan> = def.matchers.iter().map(|m| Scan::new(m.clone(), def.background)).collect();
        if def.background && !scans.iter().any(|s| s.matcher.background.is_some()) {
            run.sched.ready(node);
        }
        run.execs.push(Exec { node, label, pane, cwd, scans, seen: vec![], polled: Instant::now() });
        Ok(())
    }
}

fn add_problem(problems: &mut Vec<Problem>, found: &Found, base: Option<&str>, cwd: &Path, vars: &Vars) {
    let path = locate(&found.file, base, cwd, vars);
    let start = (found.line.saturating_sub(1), found.column.saturating_sub(1));
    let end = found.end.map_or((start.0, start.1 + 1), |(l, c)| (l.saturating_sub(1), c.saturating_sub(1)));
    let message = match &found.code {
        Some(code) => format!("{} [{code}]", found.message),
        None => found.message.clone(),
    };
    let diagnostic = Diagnostic { start, end, severity: found.severity, message, source: found.owner.clone(), raw: Value::Null };
    let same = |p: &Problem| p.path == path && p.diagnostic.start == start && p.diagnostic.message == diagnostic.message;
    if !problems.iter().any(same) {
        problems.push(Problem { owner: found.owner.clone(), path, diagnostic });
    }
}

impl Halo {
    pub(crate) fn task_command(&mut self, cmd: Command, ctx: &egui::Context) {
        use Command::*;
        let Some(root) = self.session.folder.clone() else { return ui::notify("Open a folder to use tasks.") };
        if matches!(cmd, OpenTasksJson) {
            let path = tasks_path(&root);
            if !path.exists() {
                let _ = std::fs::create_dir_all(root.join(".vscode"));
                let _ = std::fs::write(&path, "{\n  \"version\": \"2.0.0\",\n  \"tasks\": []\n}\n");
            }
            return self.editors.open(&path);
        }
        self.tasks.refresh(Some(&root), &self.settings);
        let pick = |app: &mut Self, hint: &str, items: Vec<crate::search::Choice>, empty: &str| match items.is_empty() {
            true => ui::notify(empty),
            false => app.search.pick(hint, items),
        };
        let retarget = |items: Vec<crate::search::Choice>, to: fn(String) -> Command| {
            items.into_iter().map(|(label, detail, _)| (label.clone(), detail, to(label))).collect::<Vec<_>>()
        };
        match cmd {
            RunTaskPicker => {
                let items = self.tasks.picks(None);
                pick(self, "Select the task to run", items, "No tasks were found in this folder.");
            }
            RunBuildTask | RunTestTask => {
                let kind = if cmd == RunBuildTask { "build" } else { "test" };
                let default = self.tasks.defs.iter().find(|d| d.group.as_deref() == Some(kind) && d.default);
                match default.map(|d| d.label.clone()) {
                    Some(label) => self.task_command(RunTaskLabel(label), ctx),
                    None => {
                        let items = self.tasks.picks(Some(kind));
                        pick(self, &format!("Select the {kind} task to run"), items, &format!("No {kind} task was found. Add a \"group\": \"{kind}\" to a task."));
                    }
                }
            }
            RerunLastTask => match self.tasks.last.clone() {
                Some(label) => self.task_command(RunTaskLabel(label), ctx),
                None => ui::notify("No task has been run yet."),
            },
            TerminateTask | ShowRunningTasks => {
                let to = if cmd == TerminateTask { TerminateTaskLabel } else { RevealTask };
                let items = self.tasks.running().into_iter().map(|(label, _)| (label.clone(), "running".to_owned(), to(label))).collect();
                pick(self, "Select a running task", items, "No task is running.");
            }
            ConfigureDefaultBuildTask => {
                let items = retarget(self.tasks.picks(None), SetDefaultBuild);
                pick(self, "Select the task to use as the default build task", items, "No tasks were found in this folder.");
            }
            RunTaskLabel(label) => {
                let file = self.editors.active_path();
                match self.tasks.run_label(&label, Some(&root), file, &self.settings, &mut self.bottom, ctx) {
                    Ok(_) => self.session.panel |= self.tasks.def(&label).is_none_or(|d| d.reveal),
                    Err(error) => ui::notify(error),
                }
            }
            TerminateTaskLabel(label) => self.tasks.terminate(&label, &mut self.bottom),
            RevealTask(label) => {
                if let Some((_, pane)) = self.tasks.running().into_iter().find(|(l, _)| *l == label) {
                    self.session.panel = true;
                    self.bottom.reveal_task(pane);
                }
            }
            SetDefaultBuild(label) => {
                let Some(raw) = self.tasks.def(&label).map(|d| d.raw.clone()) else { return };
                let path = tasks_path(&root);
                let existing = std::fs::read_to_string(&path).ok();
                match with_default_build(existing.as_deref(), &label, &raw, os_key()) {
                    Ok(text) => {
                        let _ = std::fs::create_dir_all(root.join(".vscode"));
                        match std::fs::write(&path, text) {
                            Ok(()) => ui::notify(format!("{label} is now the default build task.")),
                            Err(e) => ui::notify(format!("tasks.json could not be written: {e}")),
                        }
                    }
                    Err(error) => ui::notify(error),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs(text: &str) -> Vec<Def> {
        let (defs, errors) = defs_from(text, Origin::Config, "linux");
        assert!(errors.is_empty(), "{errors:?}");
        defs
    }

    fn scan(name: &str, lines: &[&str]) -> Vec<Found> {
        let mut scan = Scan::new(builtin(name).unwrap(), false);
        lines.iter().flat_map(|l| scan.feed(l)).filter_map(|e| if let Event::Found(f) = e { Some(f) } else { None }).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_jsonc_and_task_forms() {
        let text = r#"{
            // comment with "quotes" and a trailing comma below
            "version": "2.0.0",
            "tasks": [
                { "label": "build", "type": "shell", "command": "cargo build", "args": ["--release",],
                  "group": { "kind": "build", "isDefault": true }, /* block */
                  "options": { "cwd": "${workspaceFolder}/sub", "env": { "A": "1" } },
                  "problemMatcher": ["$rustc"], "url": "http://x/y",
                  "windows": { "command": "cargo.exe" }, "linux": { "command": "cargo-linux", "options": { "env": { "B": "2" } } } },
                { "label": "t", "command": "x", "group": "test", "isBackground": true, "presentation": { "reveal": "never" } },
                { "label": "all", "dependsOn": ["build", "t"], "dependsOrder": "sequence" },
                { "type": "npm", "script": "lint" },
                { "type": "cargo", "command": "check", "args": ["-q"] },
            ],
        }"#;
        let d = defs(text);
        assert_eq!(d.len(), 5);
        assert_eq!((d[0].command.as_str(), d[0].args.clone()), ("cargo-linux", vec!["--release".to_owned()]));
        assert_eq!(d[0].group.as_deref(), Some("build"));
        assert!(d[0].default);
        assert_eq!(d[0].cwd.as_deref(), Some("${workspaceFolder}/sub"));
        assert_eq!(d[0].env.len(), 2);
        assert_eq!(d[0].matchers[0].owner, "rustc");
        let (windows, _) = defs_from(text, Origin::Config, "windows");
        assert_eq!(windows[0].command, "cargo.exe");
        assert_eq!(d[1].group.as_deref(), Some("test"));
        assert!(d[1].background && !d[1].reveal && !d[1].default);
        assert!(d[2].sequence && d[2].depends == ["build", "t"]);
        assert_eq!((d[3].label.as_str(), d[3].command.as_str(), d[3].args.join(" ")), ("npm: lint", "npm", "run lint".to_owned()));
        assert_eq!((d[4].label.as_str(), d[4].args.join(" ")), ("cargo check", "check -q".to_owned()));
        let (none, errors) = defs_from(r#"{"tasks": [{"label": "x"}, {"command": "y"}]}"#, Origin::Config, "linux");
        assert!(none.is_empty() && errors.len() == 2);
        assert!(defs_from("{", Origin::Config, "linux").1[0].contains("could not be read"));
    }

    #[test]
    fn substitutes_variables() {
        let vars = Vars { workspace: PathBuf::from("/w"), file: Some(PathBuf::from("/w/src/main.rs")) };
        let ok = |t: &str| substitute(t, &vars).unwrap();
        assert_eq!(ok("${workspaceFolder}"), "/w");
        assert_eq!(ok("${file}"), "/w/src/main.rs");
        assert_eq!(ok("${fileBasename}|${fileBasenameNoExtension}|${fileDirname}"), "main.rs|main|/w/src");
        assert_eq!(PathBuf::from(ok("${relativeFile}")), PathBuf::from("src/main.rs"));
        assert_eq!(ok("a-${env:CARGO_PKG_NAME}-${env:HALO_TASKS_MISSING}-b"), "a-halo--b");
        assert_eq!(ok("${unknown} ${open"), "${unknown} ${open");
        assert!(substitute("${file}", &Vars::default()).is_err());
    }

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi("\u{1b}[1m\u{1b}[31merror\u{1b}[0m: x\u{1b}]0;title\u{7}!"), "error: x!");
    }

    #[test]
    fn rustc_matcher() {
        let found = scan("$rustc", &[
            "\u{1b}[1;31merror[E0425]\u{1b}[0m: cannot find value `x` in this scope",
            "  --> src/main.rs:2:5",
            "   |",
            "warning: unused variable: `y`",
            " --> src/lib.rs:10:9",
            "error: aborting due to 1 previous error",
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].file.as_str(), found[0].line, found[0].column, found[0].severity), ("src/main.rs", 2, 5, 1));
        assert_eq!((found[0].code.as_deref(), found[0].message.as_str()), (Some("E0425"), "cannot find value `x` in this scope"));
        assert_eq!((found[1].severity, found[1].line), (2, 10));
    }

    #[test]
    fn tsc_and_watch_matchers() {
        for name in ["$tsc", "$tsc-watch"] {
            let mut scan = Scan::new(builtin(name).unwrap(), true);
            scan.active = true;
            let a = scan.feed("src/a.ts(3,5): error TS2322: Type 'string' is not assignable to type 'number'.");
            let b = scan.feed("src/b.ts:7:1 - warning TS6133: 'x' is declared but never used.");
            let found: Vec<_> = a.into_iter().chain(b).collect();
            assert_eq!(found.len(), 2);
            let Event::Found(first) = &found[0] else { panic!() };
            assert_eq!((first.file.as_str(), first.line, first.column, first.code.as_deref()), ("src/a.ts", 3, 5, Some("TS2322")));
            let Event::Found(second) = &found[1] else { panic!() };
            assert_eq!((second.file.as_str(), second.severity), ("src/b.ts", 2));
        }
    }

    #[test]
    fn eslint_matchers_with_loop() {
        let found = scan("$eslint-stylish", &[
            "/p/src/a.js",
            "  1:10  error    'x' is defined but never used  no-unused-vars",
            "  4:1   warning  Unexpected console statement   no-console",
            "",
            "/p/src/b.js",
            "  2:3  error  Parsing error: Unexpected token",
            "",
            "\u{2716} 3 problems (2 errors, 1 warning)",
        ]);
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].file.as_str(), found[0].line, found[0].column, found[0].code.as_deref()), ("/p/src/a.js", 1, 10, Some("no-unused-vars")));
        assert_eq!((found[1].file.as_str(), found[1].severity, found[1].message.as_str()), ("/p/src/a.js", 2, "Unexpected console statement"));
        assert_eq!((found[2].file.as_str(), found[2].code.clone(), found[2].message.as_str()), ("/p/src/b.js", None, "Parsing error: Unexpected token"));
        let compact = scan("$eslint-compact", &["/p/a.js: line 1, col 10, Error - 'x' is defined but never used. (no-unused-vars)"]);
        assert_eq!((compact[0].file.as_str(), compact[0].line, compact[0].column, compact[0].code.as_deref()), ("/p/a.js", 1, 10, Some("no-unused-vars")));
    }

    #[test]
    fn gcc_mscompile_and_go_matchers() {
        let gcc = scan("$gcc", &["src/main.c:12:7: warning: unused variable 'x' [-Wunused-variable]", "src/main.c:15:3: error: expected ';' before '}' token"]);
        assert_eq!((gcc[0].file.as_str(), gcc[0].line, gcc[0].column, gcc[0].severity), ("src/main.c", 12, 7, 2));
        assert_eq!(gcc[1].severity, 1);
        let ms = scan("$msCompile", &[
            "  1>c:\\src\\a.cpp(10,5): error C2065: 'x': undeclared identifier [c:\\src\\a.vcxproj]",
            "b.cs(7): warning CS0168: The variable 'e' is declared but never used",
        ]);
        assert_eq!((ms[0].file.as_str(), ms[0].line, ms[0].column, ms[0].code.as_deref()), ("c:\\src\\a.cpp", 10, 5, Some("C2065")));
        assert_eq!((ms[1].file.as_str(), ms[1].line, ms[1].severity), ("b.cs", 7, 2));
        let go = scan("$go", &["# command-line-arguments", "./main.go:5:2: undefined: foo", "pkg/util.go:9: missing return"]);
        assert_eq!((go[0].file.as_str(), go[0].line, go[0].column, go[0].message.as_str()), ("./main.go", 5, 2, "undefined: foo"));
        assert_eq!((go[1].line, go[1].column), (9, 1));
        assert_eq!(go[0].severity, 1);
    }

    #[test]
    fn inline_matcher_with_multiline_loop() {
        let spec = json!({
            "owner": "mine", "severity": "warning", "fileLocation": ["relative", "${workspaceFolder}/src"],
            "pattern": [
                {"regexp": r"^File: (.+)$", "file": 1},
                {"regexp": r"^\s+(\d+),(\d+)-(\d+),(\d+): (\w+)?: (.+)$", "line": 1, "column": 2, "endLine": 3, "endColumn": 4, "severity": 5, "message": 6, "loop": true},
            ]
        });
        let mut scan = Scan::new(parse_matcher(&spec).unwrap(), false);
        let lines = ["File: a.x", "  1,2-1,5: error: bad", "  7,1-8,2: : plain", "noise", "  9,9-9,9: error: orphan", "File: b.x", "  3,3-3,4: note: n"];
        let found: Vec<Found> = lines.iter().flat_map(|l| scan.feed(l)).filter_map(|e| if let Event::Found(f) = e { Some(f) } else { None }).collect();
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].file.as_str(), found[0].end, found[0].severity), ("a.x", Some((1, 5)), 1));
        assert_eq!((found[1].line, found[1].severity, found[1].message.as_str()), (7, 2, "plain"));
        assert_eq!((found[2].file.as_str(), found[2].severity), ("b.x", 3));
        let vars = Vars { workspace: PathBuf::from("/w"), file: None };
        assert_eq!(locate("./a.x", scan.matcher.base.as_deref(), Path::new("/cwd"), &vars), PathBuf::from("/w/src/a.x"));
        assert_eq!(locate("a.x", None, Path::new("/cwd"), &vars), PathBuf::from("/cwd/a.x"));
        let single = json!({"pattern": {"regexp": r"^(\S+):(\d+): (.*)$", "file": 1, "line": 2, "message": 3}});
        assert_eq!(parse_matcher(&single).unwrap().owner, "external");
        assert!(parse_matcher(&json!({"pattern": {"regexp": "("}})).is_err());
        assert!(parse_matcher(&json!({})).is_err());
    }

    #[test]
    fn background_begin_and_end() {
        let spec = json!({"owner": "w", "pattern": {"regexp": r"^E (.+) (\d+)$", "file": 1, "line": 2, "message": 1},
            "background": {"beginsPattern": "^start", "endsPattern": "^done"}});
        let mut scan = Scan::new(parse_matcher(&spec).unwrap(), true);
        assert!(!scan.active);
        assert!(scan.feed("E ignored 1").is_empty());
        assert_eq!(scan.feed("start"), vec![Event::Begin]);
        assert!(matches!(scan.feed("E a.rs 3").as_slice(), [Event::Found(_)]));
        assert_eq!(scan.feed("done"), vec![Event::End]);
        assert!(scan.feed("E late 4").is_empty());
        let mut plain = Scan::new(parse_matcher(&spec).unwrap(), false);
        assert!(plain.active && matches!(plain.feed("E a.rs 3").as_slice(), [Event::Found(_)]));
        let watch = Scan::new(builtin("$tsc-watch").unwrap(), true);
        assert!(watch.active);
        let mut watch = watch;
        assert_eq!(watch.feed("[10:01:02 AM] File change detected. Starting incremental compilation..."), vec![Event::Begin]);
        assert_eq!(watch.feed("[10:01:05 AM] Found 0 errors. Watching for file changes."), vec![Event::End]);
    }

    #[test]
    fn finds_fresh_lines() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(fresh(&v(&[]), &v(&["a", "b"])), 0);
        assert_eq!(fresh(&v(&["a", "b"]), &v(&["a", "b", "c"])), 2);
        assert_eq!(fresh(&v(&["a", "b", "c"]), &v(&["b", "c", "d"])), 2);
        assert_eq!(fresh(&v(&["a", "b"]), &v(&["x", "y"])), 0);
    }

    fn graph(text: &str, root: &str) -> Result<Sched, String> {
        Sched::new(&defs(text), root)
    }

    fn started(sched: &mut Sched) -> Vec<String> {
        sched.start().into_iter().map(|n| sched.nodes[n].label.clone()).collect()
    }

    #[test]
    fn orders_parallel_and_sequence_dependencies() {
        let tasks = |order: &str| {
            format!(
                r#"{{"tasks": [
                {{"label": "all", "dependsOn": ["a", "b", "c"], "dependsOrder": "{order}"}},
                {{"label": "a", "command": "x"}}, {{"label": "b", "command": "x", "isBackground": true}}, {{"label": "c", "command": "x", "dependsOn": "d"}},
                {{"label": "d", "command": "x"}}]}}"#
            )
        };
        let mut parallel = graph(&tasks("parallel"), "all").unwrap();
        let first = started(&mut parallel);
        assert_eq!(first, ["a", "b", "d"]);
        let a = parallel.nodes.iter().position(|n| n.label == "a").unwrap();
        let (b, c, d) = (a + 1, a + 2, a + 3);
        parallel.finish(a, true);
        parallel.ready(b);
        assert!(parallel.start().is_empty());
        parallel.finish(d, true);
        assert_eq!(started(&mut parallel), ["c"]);
        assert_eq!(parallel.outcome(), None);
        parallel.finish(c, true);
        assert!(parallel.start().is_empty());
        assert_eq!(parallel.outcome(), Some(true));
        assert!(parallel.busy());

        let mut sequence = graph(&tasks("sequence"), "all").unwrap();
        let first = started(&mut sequence);
        assert_eq!(first, ["a"]);
        assert!(sequence.start().is_empty());
        sequence.finish(1, true);
        assert_eq!(started(&mut sequence), ["b"]);
        assert!(sequence.start().is_empty());
        sequence.ready(2);
        assert_eq!(started(&mut sequence), ["d"]);
        sequence.finish(4, true);
        assert_eq!(started(&mut sequence), ["c"]);
        sequence.finish(3, true);
        assert!(sequence.start().is_empty());
        assert_eq!(sequence.outcome(), Some(true));

        let mut failing = graph(&tasks("sequence"), "all").unwrap();
        failing.start();
        failing.finish(1, false);
        assert!(failing.start().is_empty());
        assert_eq!(failing.outcome(), Some(false));
    }

    #[test]
    fn detects_cycles_and_missing_dependencies() {
        let cyclic = r#"{"tasks": [{"label": "a", "dependsOn": "b"}, {"label": "b", "dependsOn": ["c"]}, {"label": "c", "command": "x", "dependsOn": "a"}]}"#;
        assert_eq!(graph(cyclic, "a").err().unwrap(), "Task dependency cycle: a -> b -> c -> a");
        let selfish = r#"{"tasks": [{"label": "a", "command": "x", "dependsOn": "a"}]}"#;
        assert!(graph(selfish, "a").is_err());
        let diamond = r#"{"tasks": [{"label": "a", "dependsOn": ["b", "c"]}, {"label": "b", "command": "x", "dependsOn": "d"}, {"label": "c", "command": "x", "dependsOn": "d"}, {"label": "d", "command": "x"}]}"#;
        assert_eq!(graph(diamond, "a").unwrap().nodes.len(), 4);
        let missing = r#"{"tasks": [{"label": "a", "dependsOn": "zzz"}]}"#;
        assert!(graph(missing, "a").err().unwrap().contains("zzz"));
    }

    #[test]
    fn detects_package_managers_and_cargo() {
        let dir = temp("detect");
        std::fs::write(dir.join("package.json"), "\u{feff}{\"scripts\": {\"build\": \"tsc\", \"dev\": \"vite\"}}").unwrap();
        for (lock, pm) in [("", "npm"), ("yarn.lock", "yarn"), ("pnpm-lock.yaml", "pnpm"), ("bun.lockb", "bun")] {
            if !lock.is_empty() {
                std::fs::write(dir.join(lock), "").unwrap();
            }
            assert_eq!(package_manager(&dir), pm);
            let found: Vec<(String, String)> = detect(&dir).iter().map(|t| (t["label"].as_str().unwrap().into(), t["command"].as_str().unwrap().into())).collect();
            assert_eq!(found, [("npm: build".to_owned(), format!("{pm} run build")), ("npm: dev".to_owned(), format!("{pm} run dev"))]);
        }
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        let (defs, errors) = load(&dir, &Settings::default(), "linux");
        assert!(errors.is_empty());
        let labels: Vec<&str> = defs.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, ["npm: build", "npm: dev", "cargo build", "cargo check", "cargo test", "cargo run", "cargo clippy"]);
        let cargo = defs.iter().find(|d| d.label == "cargo clippy").unwrap();
        assert_eq!((cargo.matchers[0].owner.as_str(), cargo.origin, cargo.args.clone()), ("rustc", Origin::Detected, vec!["clippy".to_owned()]));
        assert_eq!(defs.iter().find(|d| d.label == "cargo test").unwrap().group.as_deref(), Some("test"));

        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        std::fs::write(tasks_path(&dir), r#"{"tasks": [{"label": "cargo build", "command": "mine"}]}"#).unwrap();
        let mut settings = Settings::default();
        settings.tasks.push(crate::settings::Task { name: "from settings".into(), command: "echo hi".into() });
        let (merged, _) = load(&dir, &settings, "linux");
        assert_eq!(merged[0].command, "mine");
        assert_eq!(merged.iter().filter(|d| d.label == "cargo build").count(), 1);
        assert_eq!(merged.iter().find(|d| d.label == "from settings").unwrap().origin, Origin::Settings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_the_default_build_task() {
        let raw = json!({"label": "cargo build", "type": "shell", "command": "cargo", "args": ["build"]});
        let created = with_default_build(None, "cargo build", &raw, "linux").unwrap();
        let doc: Value = serde_json::from_str(&created).unwrap();
        assert_eq!(doc["version"], "2.0.0");
        assert_eq!(doc["tasks"][0]["group"], json!({"kind": "build", "isDefault": true}));
        assert_eq!(doc["tasks"][0]["command"], "cargo");
        let existing = r#"{"tasks": [ // old
            {"label": "old", "command": "x", "group": {"kind": "build", "isDefault": true}},
            {"label": "mine", "command": "y"},
        ]}"#;
        let doc: Value = serde_json::from_str(&with_default_build(Some(existing), "mine", &json!({}), "linux").unwrap()).unwrap();
        assert_eq!(doc["tasks"][0]["group"], "build");
        assert_eq!(doc["tasks"][1]["group"]["isDefault"], true);
        assert_eq!(doc["tasks"].as_array().unwrap().len(), 2);
        assert!(with_default_build(Some("{"), "x", &raw, "linux").is_err());
    }

    #[test]
    fn builds_launch_lines() {
        let (program, args) = launch_args(false, "echo", &["a b".to_owned(), "c".to_owned()]);
        if cfg!(windows) {
            assert_eq!((program.as_str(), args.last().unwrap().as_str()), ("cmd.exe", "\"echo \"a b\" c\""));
        } else {
            assert_eq!((program.as_str(), args[1].as_str()), ("sh", "echo \"a b\" c"));
        }
        assert_eq!(launch_args(true, "node", &["x".to_owned()]).0, "node");
    }

    fn wait_for(rx: &Receiver<Outcome>, tasks: &mut Tasks, bottom: &mut BottomPanel, ctx: &egui::Context) -> Outcome {
        let started = Instant::now();
        loop {
            tasks.poll(bottom, ctx);
            if let Ok(outcome) = rx.try_recv() {
                return outcome;
            }
            assert!(started.elapsed() < Duration::from_secs(60), "the task did not finish");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn runs_a_task_matches_problems_and_reports_the_exit() {
        let dir = temp("run");
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        let script = if cfg!(windows) {
            r#"echo src/main.rs:3:4: error: boom&& exit 3"#
        } else {
            r#"echo 'src/main.rs:3:4: error: boom'; exit 3"#
        };
        let tasks_json = json!({"version": "2.0.0", "tasks": [
            {"label": "fail", "type": "shell", "command": script,
             "problemMatcher": {"owner": "t", "fileLocation": ["relative", "${workspaceFolder}"],
                "pattern": {"regexp": r"^(\S+):(\d+):(\d+): (\w+): (.*)$", "file": 1, "line": 2, "column": 3, "severity": 4, "message": 5}}},
            {"label": "ok", "type": "shell", "command": "echo fine"},
            {"label": "both", "dependsOn": ["ok", "fail"], "dependsOrder": "sequence"},
            {"label": "after", "dependsOn": ["ok"], "dependsOrder": "sequence", "type": "shell", "command": "echo after"},
        ]});
        std::fs::write(tasks_path(&dir), tasks_json.to_string()).unwrap();
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let (mut tasks, mut bottom) = (Tasks::default(), BottomPanel::new());

        let rx = tasks.run_label("fail", Some(&dir), None, &settings, &mut bottom, &ctx).unwrap();
        assert_eq!(tasks.running_count(), 1);
        let outcome = wait_for(&rx, &mut tasks, &mut bottom, &ctx);
        assert!(!outcome.ok);
        assert!(outcome.error.unwrap().contains("exit code 3"));
        assert_eq!(tasks.counts(), (1, 0));
        let files = tasks.files();
        assert_eq!(files[0].0, dir.join("src").join("main.rs"));
        assert_eq!((files[0].1[0].start, files[0].1[0].message.as_str()), ((2, 3), "boom"));

        let rx = tasks.run_label("both", Some(&dir), None, &settings, &mut bottom, &ctx).unwrap();
        let outcome = wait_for(&rx, &mut tasks, &mut bottom, &ctx);
        assert!(!outcome.ok);
        assert_eq!(tasks.counts(), (1, 0));

        let rx = tasks.run_label("after", Some(&dir), None, &settings, &mut bottom, &ctx).unwrap();
        assert!(wait_for(&rx, &mut tasks, &mut bottom, &ctx).ok);
        assert!(tasks.run_label("nope", Some(&dir), None, &settings, &mut bottom, &ctx).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
