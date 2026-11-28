mod cursor;
pub use cursor::CursorCommands;

use crate::app::{Halo, note};
use crate::command::Command;
use crate::dap::{self, Incoming};
use crate::settings::Settings;
use crate::tasks::{Outcome, Tasks};
use crate::terminal::BottomPanel;
use crate::theme::Theme;
use crate::ui::{self, icons};
use crate::workspace::Workspace;
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Key, Pos2, Rect, Response, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    pos2, vec2,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

mod procs;
mod views;
mod breakpoints;
pub use breakpoints::{BreakpointCommands, FunctionBreakpoint};
use breakpoints::{DataBreakpoint, Verification};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Breakpoint {
    pub line: usize,
    pub enabled: bool,
    pub condition: String,
    pub hit: String,
    pub log: String,
    #[serde(skip)]
    pub verified: bool,
    #[serde(skip)]
    pub id: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    Condition,
    Hit,
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gutter {
    Toggle(usize),
    Ask(usize, Ask),
    Enable(usize, bool),
    Remove(usize),
}

struct Hover {
    owner: (usize, u64),
    word: String,
    since: f64,
    asked: bool,
    value: Option<String>,
}

#[derive(Default)]
pub struct Marks {
    pub files: Vec<(PathBuf, Vec<Breakpoint>)>,
    functions: Vec<FunctionBreakpoint>,
    functions_changed: bool,
    changed: Vec<PathBuf>,
    folder: Option<PathBuf>,
    pub live: bool,
    pub stopped: Option<(PathBuf, usize)>,
    pub paused: Vec<(PathBuf, usize)>,
    ask: Option<(PathBuf, usize, Ask)>,
    hover: Option<Hover>,
    pub cursor: CursorCommands,
    inline: HashMap<String, String>,
    pub band: Option<(String, usize)>,
}

fn same(a: &Path, b: &Path) -> bool {
    crate::lsp::key(a) == crate::lsp::key(b)
}

impl Marks {
    pub fn of(&self, path: &Path) -> &[Breakpoint] {
        self.files.iter().find(|f| same(&f.0, path)).map_or(&[], |f| f.1.as_slice())
    }

    pub fn stopped_line(&self, path: &Path) -> Option<usize> {
        self.stopped.as_ref().filter(|s| same(&s.0, path)).map(|s| s.1)
    }

    pub fn stopped_lines(&self, path: &Path) -> Vec<usize> {
        self.paused.iter().filter(|s| same(&s.0, path)).map(|s| s.1).collect()
    }

    fn list(&mut self, path: &Path) -> &mut Vec<Breakpoint> {
        let at = match self.files.iter().position(|f| same(&f.0, path)) {
            Some(at) => at,
            None => {
                self.files.push((path.to_path_buf(), vec![]));
                self.files.len() - 1
            }
        };
        if !self.changed.iter().any(|c| same(c, path)) {
            self.changed.push(self.files[at].0.clone());
        }
        &mut self.files[at].1
    }

    fn tidy(&mut self) {
        for (_, list) in &mut self.files {
            list.sort_by_key(|b| b.line);
            list.dedup_by_key(|b| b.line);
        }
        self.files.retain(|f| !f.1.is_empty());
    }

    pub fn toggle(&mut self, path: &Path, line: usize) {
        self.apply(path, Gutter::Toggle(line));
    }

    pub fn apply(&mut self, path: &Path, action: Gutter) {
        let list = self.list(path);
        let at = |list: &Vec<Breakpoint>, line: usize| list.iter().position(|b| b.line == line);
        match action {
            Gutter::Toggle(line) => match at(list, line) {
                Some(found) => drop(list.remove(found)),
                None => list.push(Breakpoint { line, enabled: true, ..Default::default() }),
            },
            Gutter::Enable(line, on) => list.iter_mut().filter(|b| b.line == line).for_each(|b| b.enabled = on),
            Gutter::Remove(line) => list.retain(|b| b.line != line),
            Gutter::Ask(line, ask) => self.ask = Some((path.to_path_buf(), line, ask)),
        }
        self.tidy();
    }

    fn set(&mut self, path: &Path, line: usize, texts: [String; 3]) {
        let list = self.list(path);
        if !list.iter().any(|b| b.line == line) {
            list.push(Breakpoint { line, enabled: true, ..Default::default() });
        }
        if let Some(b) = list.iter_mut().find(|b| b.line == line) {
            [b.condition, b.hit, b.log] = texts;
        }
        self.tidy();
    }

    pub fn clear(&mut self) {
        self.functions.clear();
        self.functions_changed = true;
        for (path, _) in std::mem::take(&mut self.files) {
            self.changed.push(path);
        }
    }

    pub fn shift(&mut self, path: &Path, edited: usize, delta: isize) {
        let Some(at) = self.files.iter().position(|f| same(&f.0, path)) else { return };
        let mut moved = false;
        for b in self.files[at].1.iter_mut().filter(|b| b.line > edited) {
            let line = (b.line as isize + delta).max(edited as isize) as usize;
            moved |= line != b.line;
            b.line = line;
        }
        if moved {
            self.list(path);
            self.tidy();
        }
    }

    fn verify(&mut self, path: &Path, body: &Value) {
        let Some(at) = self.files.iter().position(|f| same(&f.0, path)) else { return };
        let reported = body["breakpoints"].as_array().map_or(&[] as &[Value], Vec::as_slice);
        let mut reported = reported.iter();
        for b in self.files[at].1.iter_mut() {
            b.verified = false;
            b.id = None;
            if let Some(found) = b.enabled.then(|| reported.next()).flatten() {
                b.verified = found["verified"] == true;
                b.id = found["id"].as_i64();
            }
        }
    }

    fn update(&mut self, found: &Value) {
        let Some(id) = found["id"].as_i64() else { return };
        for b in self.files.iter_mut().flat_map(|f| f.1.iter_mut()).filter(|b| b.id == Some(id)) {
            b.verified = found["verified"] == true;
        }
    }

    fn reset(&mut self, live: bool) {
        self.live = live;
        self.cursor = CursorCommands::default();
        self.inline.clear();
        self.stopped = None;
        self.paused.clear();
        self.hover = None;
        for b in self.files.iter_mut().flat_map(|f| f.1.iter_mut()) {
            (b.verified, b.id) = (false, None);
        }
    }
}

pub fn store(session: &mut crate::settings::Session, marks: &Marks) {
    let Some(folder) = &marks.folder else { return };
    let files: BTreeMap<PathBuf, Vec<Breakpoint>> = marks.files.iter().filter(|f| !f.1.is_empty()).cloned().collect();
    if marks.functions.is_empty() {
        session.function_breakpoints.remove(folder);
    } else {
        session.function_breakpoints.insert(folder.clone(), marks.functions.clone());
    }
    if files.is_empty() {
        session.breakpoints.remove(folder);
    } else {
        session.breakpoints.insert(folder.clone(), files);
    }
}

pub fn restore(session: &crate::settings::Session, marks: &mut Marks) {
    marks.folder = session.folder.clone();
    marks.functions = marks.folder.as_ref().and_then(|f| session.function_breakpoints.get(f)).cloned().unwrap_or_default();
    marks.functions_changed = true;
    marks.files = marks.folder.as_ref().and_then(|f| session.breakpoints.get(f)).map(|m| m.clone().into_iter().collect()).unwrap_or_default();
    marks.changed.clear();
}

pub fn source_breakpoints(path: &Path, list: &[Breakpoint]) -> Value {
    let sent: Vec<Value> = list
        .iter()
        .filter(|b| b.enabled)
        .map(|b| {
            let mut item = json!({"line": b.line + 1});
            for (key, text) in [("condition", &b.condition), ("hitCondition", &b.hit), ("logMessage", &b.log)] {
                if !text.trim().is_empty() {
                    item[key] = json!(text.trim());
                }
            }
            item
        })
        .collect();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    json!({"source": {"name": name, "path": path.to_string_lossy()}, "breakpoints": sent, "sourceModified": false})
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub name: String,
    pub kind: String,
    pub request: String,
    pub pre_launch_task: Option<String>,
    pub raw: Map<String, Value>,
    pub folder: Option<PathBuf>,
}

pub fn parse_launch(text: &str) -> Result<Vec<Config>, String> {
    let value = crate::jsonc::parse(text).map_err(|e| format!("launch.json could not be read: {e}"))?;
    let configs = value["configurations"].as_array().into_iter().flatten().filter_map(|item| {
        let raw = item.as_object()?.clone();
        let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Config {
            name: text("name").unwrap_or_else(|| "Unnamed".into()),
            kind: text("type").unwrap_or_default(),
            request: text("request").unwrap_or_else(|| "launch".into()),
            pre_launch_task: text("preLaunchTask"),
            raw,
            folder: None,
        })
    });
    Ok(configs.collect())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Compound {
    pub name: String,
    pub configurations: Vec<String>,
    pub stop_all: bool,
    pub pre_launch_task: Option<String>,
}

pub fn parse_compounds(text: &str) -> Vec<Compound> {
    let Ok(value) = crate::jsonc::parse(text) else { return vec![] };
    let compounds = value["compounds"].as_array().into_iter().flatten().map(|item| Compound {
        name: item["name"].as_str().unwrap_or("Unnamed").to_owned(),
        configurations: item["configurations"].as_array().into_iter().flatten().filter_map(|c| c.as_str().or(c["name"].as_str())).map(str::to_owned).collect(),
        stop_all: item["stopAll"] == true,
        pre_launch_task: item["preLaunchTask"].as_str().map(str::to_owned),
    });
    compounds.collect()
}

struct Vars {
    folder: PathBuf,
    file: Option<PathBuf>,
    all: Workspace,
}

fn launch_configs(texts: &[(PathBuf, String, String)]) -> (Vec<Config>, Vec<Compound>, Option<String>) {
    let (mut configs, mut compounds, mut error) = (vec![], vec![], None);
    for (folder, tag, text) in texts {
        match parse_launch(text) {
            Ok(found) => {
                configs.extend(found.into_iter().map(|c| (Config { folder: Some(folder.clone()), ..c }, tag)));
                compounds.extend(parse_compounds(text).into_iter().map(|c| (c, folder)));
            }
            Err(e) => error = error.or(Some(e)),
        }
    }
    if error.is_some() {
        return (vec![], vec![], error);
    }
    let clash = |name: &str| texts.len() > 1 && configs.iter().filter(|(c, _)| c.name == name).count() > 1;
    let shown = |c: &Config, tag: &str| if clash(&c.name) { format!("{} ({tag})", c.name) } else { c.name.clone() };
    let compounds = compounds.into_iter().map(|(c, folder)| {
        let find = |name: &String| configs.iter().find(|(x, _)| x.name == *name && x.folder.as_ref() == Some(folder)).or_else(|| configs.iter().find(|(x, _)| x.name == *name));
        Compound { configurations: c.configurations.iter().map(|name| find(name).map_or(name.clone(), |(x, tag)| shown(x, tag))).collect(), ..c }
    });
    let compounds = compounds.collect();
    let named = configs.iter().map(|(c, tag)| Config { name: shown(c, tag), ..c.clone() });
    (named.collect(), compounds, None)
}

fn expand(text: &str, vars: &Vars) -> String {
    let shown = |path: &Path| path.to_string_lossy().into_owned();
    let file = vars.file.as_deref();
    let value = |name: &str| -> Option<String> {
        Some(match name {
            "workspaceFolder" => shown(&vars.folder),
            "file" => shown(file?),
            "fileBasename" => file?.file_name()?.to_string_lossy().into_owned(),
            "fileBasenameNoExtension" => file?.file_stem()?.to_string_lossy().into_owned(),
            "fileDirname" => shown(file?.parent()?),
            "relativeFile" => shown(file?.strip_prefix(&vars.folder).unwrap_or(file?)),
            _ if name.starts_with("workspaceFolder:") => shown(&vars.all.folders[vars.all.find(&name[16..])?].path),
            _ => std::env::var(name.strip_prefix("env:")?).unwrap_or_default(),
        })
    };
    let (mut out, mut rest) = (String::new(), text);
    while let Some(at) = rest.find("${") {
        let Some(end) = rest[at..].find('}').map(|end| at + end) else { break };
        out.push_str(&rest[..at]);
        match value(&rest[at + 2..end]) {
            Some(found) => out.push_str(&found),
            None => out.push_str(&rest[at..=end]),
        }
        rest = &rest[end + 1..];
    }
    out + rest
}

fn substitute(value: &mut Value, vars: &Vars) {
    match value {
        Value::String(text) => *text = expand(text, vars),
        Value::Array(items) => items.iter_mut().for_each(|item| substitute(item, vars)),
        Value::Object(map) => map.values_mut().for_each(|item| substitute(item, vars)),
        _ => {}
    }
}

impl Config {
    fn resolve(&self, vars: &Vars) -> Config {
        let mut raw = Value::Object(self.raw.clone());
        substitute(&mut raw, vars);
        let Value::Object(raw) = raw else { unreachable!() };
        Config { raw, ..self.clone() }
    }
}

pub fn launch_path(folder: &Path) -> PathBuf {
    folder.join(".vscode").join("launch.json")
}

fn template(folder: &Path, file: Option<&Path>) -> Value {
    let ext = file.and_then(|f| f.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let exe = std::env::consts::EXE_SUFFIX;
    let crate_name = folder.file_name().unwrap_or_default().to_string_lossy();
    match ext.as_str() {
        "py" => json!({"type": "debugpy", "request": "launch", "name": "Python: Current File", "program": "${file}", "console": "integratedTerminal", "cwd": "${workspaceFolder}"}),
        "go" => json!({"type": "go", "request": "launch", "name": "Go: Launch Package", "mode": "debug", "program": "${fileDirname}"}),
        "rs" => json!({"type": "lldb", "request": "launch", "name": "Rust: Debug", "program": format!("${{workspaceFolder}}/target/debug/{crate_name}{exe}"), "args": [], "cwd": "${workspaceFolder}"}),
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" | "zig" => {
            json!({"type": "lldb-dap", "request": "launch", "name": "Debug Program", "program": format!("${{fileDirname}}/${{fileBasenameNoExtension}}{exe}"), "args": [], "cwd": "${workspaceFolder}"})
        }
        _ => json!({"type": "gdb", "request": "launch", "name": "Debug Program", "program": format!("${{fileDirname}}/${{fileBasenameNoExtension}}{exe}"), "args": [], "cwd": "${workspaceFolder}"}),
    }
}

pub fn add_configuration(folder: &Path, file: Option<&Path>) -> Result<PathBuf, String> {
    let path = launch_path(folder);
    let entry = serde_json::to_string_pretty(&template(folder, file)).unwrap_or_default().replace('\n', "\n    ");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let list = text.find("\"configurations\"").and_then(|at| text[at..].find('[').map(|open| at + open + 1));
            let at = list.ok_or("launch.json has no \"configurations\" list to add to.")?;
            let tail = if text[at..].trim_start().starts_with(']') { "\n  " } else { "," };
            format!("{}\n    {entry}{tail}{}", &text[..at], &text[at..])
        }
        Err(_) => format!("{{\n  \"version\": \"0.2.0\",\n  \"configurations\": [\n    {entry}\n  ]\n}}\n"),
    };
    std::fs::create_dir_all(folder.join(".vscode")).and_then(|_| std::fs::write(&path, text)).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path)
}

#[derive(Default)]
pub struct Console {
    pub active: bool,
    pub shown: bool,
    lines: Vec<(String, String)>,
    input: String,
    history: Vec<String>,
    at: Option<usize>,
    submitted: Vec<String>,
}

impl Console {
    fn begin(&mut self, name: &str) {
        (self.active, self.shown) = (true, true);
        self.lines.clear();
        self.push("console", &format!("Starting {name}\n"));
    }

    pub fn push(&mut self, category: &str, text: &str) {
        self.tagged("", category, text);
    }

    fn tagged(&mut self, tag: &str, category: &str, text: &str) {
        let head = if tag.is_empty() { String::new() } else { format!("[{tag}] ") };
        let last = if tag.is_empty() { self.lines.last_mut() } else { self.lines.iter_mut().rev().find(|l| l.1.starts_with(&head)) };
        match last.filter(|l| l.0 == category && !l.1.ends_with('\n')) {
            Some(last) => last.1.push_str(text),
            None => self.lines.push((category.to_owned(), head + text)),
        }
        let extra = self.lines.len().saturating_sub(5000);
        self.lines.drain(..extra);
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.lines.iter().map(|l| l.1.as_str()).collect()
    }

    pub fn ui(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, font_size: f32) {
        let font = FontId::monospace(font_size);
        let input_h = if self.active { 30.0 } else { 0.0 };
        let list = body.shrink2(vec2(14.0, 4.0)).with_max_y(body.bottom() - input_h);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ScrollArea::vertical().id_salt("debug_console").auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                for (category, text) in &self.lines {
                    let color = match category.as_str() {
                        "stderr" => theme.error,
                        "important" => theme.warning,
                        "console" => theme.text_dim,
                        "input" => theme.accent,
                        _ => theme.text,
                    };
                    ui.add(egui::Label::new(RichText::new(text.trim_end_matches('\n')).font(font.clone()).color(color)).wrap());
                }
            });
        });
        if !self.active {
            return;
        }
        let line = Rect::from_min_max(pos2(body.left(), body.bottom() - input_h), body.max);
        ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, theme.border));
        let id = Id::new("debug_console_input");
        ui::recall(ui, id, &self.history, &mut self.at, &mut self.input);
        ui.painter().text(pos2(line.left() + 14.0, line.center().y), Align2::LEFT_CENTER, icons::CARET_RIGHT, FontId::proportional(13.0), theme.accent);
        let edit = TextEdit::singleline(&mut self.input).id(id).font(font).hint_text("Evaluate an expression").frame(egui::Frame::NONE).desired_width(f32::INFINITY);
        let resp = ui.put(line.shrink2(vec2(14.0, 6.0)).with_min_x(line.left() + 30.0), edit);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let text = std::mem::take(&mut self.input);
            if !text.trim().is_empty() {
                self.history.retain(|h| *h != text);
                self.history.push(text.clone());
                self.push("input", &format!("> {text}\n"));
                self.submitted.push(text);
            }
            self.at = None;
            resp.request_focus();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Starting,
    Running,
    Stopped,
}

#[derive(Clone, Debug)]
struct Frame {
    id: i64,
    name: String,
    path: Option<PathBuf>,
    line: usize,
    column: usize,
    ip: Option<String>,
}

#[derive(Clone, Debug)]
struct Var {
    name: String,
    value: String,
    kind: String,
    reference: i64,
    expensive: bool,
    memory: Option<String>,
}

fn vars(list: &Value, key: &str) -> Vec<Var> {
    let items = list[key].as_array().map_or(&[] as &[Value], Vec::as_slice);
    let text = |item: &Value, key: &str| item[key].as_str().unwrap_or_default().to_owned();
    let var = |item: &Value| Var {
        name: text(item, "name"),
        value: text(item, "value"),
        kind: text(item, "type"),
        reference: item["variablesReference"].as_i64().unwrap_or(0),
        expensive: item["expensive"] == true,
        memory: item["memoryReference"].as_str().map(str::to_owned),
    };
    items.iter().map(var).collect()
}

enum Want {
    Initialize,
    Launch,
    Breakpoints(PathBuf),
    Functions(Vec<String>),
    DataInfo(String),
    Data(Vec<String>),
    Plain(String),
    Threads,
    Stack,
    Scopes(i64),
    Variables(i64),
    Watch(String),
    Repl,
    Hover(String),
    SetVariable(i64, String),
    Run,
    RunTo(i64, usize),
    GotoTargets(i64),
    Goto,
    Disconnect,
    Restart,
    Disassemble(String),
    ReadMemory(String),
    MemoryAt(String),
}

struct Live {
    client: dap::Client,
    config: Config,
    no_debug: bool,
    caps: Value,
    state: State,
    reason: String,
    threads: Vec<(i64, String)>,
    thread: Option<i64>,
    frames: Vec<Frame>,
    frame: usize,
    scopes: Vec<Var>,
    children: HashMap<i64, Vec<Var>>,
    asked: HashSet<i64>,
    watch: HashMap<String, Result<String, String>>,
    wants: HashMap<i64, Want>,
    function_status: HashMap<String, Verification>,
    data: Vec<DataBreakpoint>,
    configured: bool,
    stopping: u8,
    restart: bool,
    temporary: Option<(PathBuf, usize)>,
    id: u64,
    group: Option<String>,
    place: Option<(PathBuf, usize)>,
}

impl Live {
    fn shown_state(&self) -> &'static str {
        match self.state {
            State::Starting => "starting",
            State::Running => "running",
            State::Stopped => "paused",
        }
    }

    fn ask(&mut self, command: &str, args: Value, want: Want) {
        let request = self.client.request(command, args);
        self.wants.insert(request, want);
    }

    fn can(&self, capability: &str) -> bool {
        self.caps[capability] == true
    }

    fn frame_id(&self) -> Option<i64> {
        self.frames.get(self.frame).map(|f| f.id).filter(|_| self.state == State::Stopped)
    }

    fn evaluate(&mut self, expression: &str, context: &str, want: Want) {
        let mut args = json!({"expression": expression, "context": context});
        if let Some(frame) = self.frame_id() {
            args["frameId"] = json!(frame);
        }
        self.ask("evaluate", args, want);
    }

    fn watch_all(&mut self, watches: &[String]) {
        self.watch.clear();
        if self.state != State::Stopped {
            return;
        }
        for expression in watches {
            self.evaluate(expression, "watch", Want::Watch(expression.clone()));
        }
    }

    fn select(&mut self, index: usize, watches: &[String], marks: &mut Marks, cmds: &mut Vec<Command>) {
        self.place = None;
        marks.inline.clear();
        marks.stopped = None;
        marks.cursor = self.cursor_commands();
        self.wants.retain(|_, w| !matches!(w, Want::Scopes(_) | Want::Variables(_) | Want::SetVariable(..)));
        self.frame = index;
        (self.scopes, self.children, self.asked) = (vec![], HashMap::new(), HashSet::new());
        let Some(frame) = self.frames.get(index).cloned() else { return };
        marks.hover = None;
        if let Some(path) = frame.path.filter(|p| p.is_file()) {
            self.place = Some((path.clone(), frame.line.saturating_sub(1)));
            marks.stopped.clone_from(&self.place);
            cmds.extend([Command::OpenPath(path), Command::GoTo(frame.line.max(1), frame.column.max(1))]);
        }
        self.ask("scopes", json!({"frameId": frame.id}), Want::Scopes(frame.id));
        self.watch_all(watches);
    }

    fn focus_marks(&self, marks: &mut Marks) {
        marks.stopped.clone_from(&self.place);
        marks.cursor = self.cursor_commands();
        self.refresh_inline(marks);
        marks.hover = None;
    }

    fn running(&mut self, marks: Option<&mut Marks>) {
        self.wants.retain(|_, w| !matches!(w, Want::Threads | Want::Stack | Want::Scopes(_) | Want::Variables(_) | Want::SetVariable(..)
            | Want::Run | Want::RunTo(..) | Want::GotoTargets(_) | Want::Goto));
        self.state = State::Running;
        (self.frames, self.scopes, self.children, self.asked) = (vec![], vec![], HashMap::new(), HashSet::new());
        self.watch.clear();
        self.place = None;
        if let Some(marks) = marks {
            self.focus_marks(marks);
        }
    }

    fn send_breakpoints(&mut self, path: &Path, list: &[Breakpoint]) {
        let mut args = source_breakpoints(path, list);
        if let Some((_, line)) = self.temporary.as_ref().filter(|(p, _)| same(p, path)) {
            args = cursor::with_cursor(args, *line).0;
        }
        self.ask("setBreakpoints", args, Want::Breakpoints(path.to_path_buf()));
    }

    fn stack(&mut self, thread: i64) {
        self.ask("stackTrace", json!({"threadId": thread, "startFrame": 0, "levels": 50}), Want::Stack);
    }

    fn send_filters(&mut self, filters: &[(String, String, bool)]) {
        let known = self.caps["exceptionBreakpointFilters"].as_array().map_or(&[] as &[Value], Vec::as_slice);
        let on: Vec<&str> = filters.iter().filter(|f| f.2 && known.iter().any(|k| k["filter"] == f.0.as_str())).map(|f| f.0.as_str()).collect();
        self.ask("setExceptionBreakpoints", json!({"filters": on}), Want::Plain("setExceptionBreakpoints".into()));
    }

    fn configure(&mut self, marks: &mut Marks, filters: &[(String, String, bool)]) {
        for (path, list) in marks.files.clone() {
            if list.iter().any(|b| b.enabled) {
                self.send_breakpoints(&path, &list);
            }
        }
        marks.changed.clear();
        self.send_functions(&marks.functions);
        if !filters.is_empty() {
            self.send_filters(filters);
        }
        if self.can("supportsConfigurationDoneRequest") {
            self.ask("configurationDone", json!({}), Want::Plain("configurationDone".into()));
        }
        self.configured = true;
    }

    fn fetch(&mut self, expanded: &HashSet<String>, inline: bool) {
        fn walk(children: &HashMap<i64, Vec<Var>>, vars: &[Var], prefix: &str, expanded: &HashSet<String>, wanted: &mut Vec<i64>) {
            for var in vars.iter().filter(|v| v.reference > 0) {
                let path = format!("{prefix}/{}", var.name);
                if expanded.contains(&path) {
                    match children.get(&var.reference) {
                        Some(found) => walk(children, found, &path, expanded, wanted),
                        None => wanted.push(var.reference),
                    }
                }
            }
        }
        let mut wanted = vec![];
        walk(&self.children, &self.scopes, "", expanded, &mut wanted);
        if inline {
            wanted.extend(self.scopes.iter().filter(|s| !s.expensive && s.reference > 0).map(|s| s.reference));
        }
        for reference in wanted {
            if self.asked.insert(reference) {
                self.ask("variables", json!({"variablesReference": reference}), Want::Variables(reference));
            }
        }
    }
}

pub struct Env<'a> {
    pub folder: Option<&'a Path>,
    pub workspace: &'a Workspace,
    pub file: Option<PathBuf>,
    pub cursor_line: Option<usize>,
    pub settings: &'a Settings,
    pub marks: &'a mut Marks,
    pub bottom: &'a mut BottomPanel,
    pub tasks: &'a mut Tasks,
    pub panel: &'a mut bool,
    pub cmds: &'a mut Vec<Command>,
    pub ctx: &'a egui::Context,
}

struct Dialog {
    path: PathBuf,
    line: usize,
    ask: Ask,
    texts: [String; 3],
    focus: bool,
}

enum Editing {
    Watch(Option<usize>, String),
    FunctionName(String),
    FunctionCondition(String, Ask, String),
    PickData(u64),
    Value(i64, String, String),
}

type Pending = (Vec<Config>, bool, Option<String>, std::sync::mpsc::Receiver<Outcome>);
type Picking = (Vec<Config>, bool, Option<String>, Option<std::sync::mpsc::Receiver<Vec<procs::Process>>>);

pub struct Debug {
    configs: Vec<Config>,
    error: Option<String>,
    present: bool,
    stamp: (Option<PathBuf>, Option<SystemTime>, f64),
    selected: String,
    live: Vec<Live>,
    focus: usize,
    next: u64,
    compounds: Vec<Compound>,
    pending: Vec<Pending>,
    watches: Vec<String>,
    filters: Vec<(String, String, bool)>,
    open: [bool; 4],
    expanded: HashSet<String>,
    editing: Option<Editing>,
    dialog: Option<Dialog>,
    confirm_clear: bool,
    views: views::Views,
    picking: Option<Picking>,
    choices: Option<Vec<crate::search::Choice>>,
}

impl Default for Debug {
    fn default() -> Self {
        Self {
            configs: vec![],
            error: None,
            present: false,
            stamp: (None, None, f64::NEG_INFINITY),
            selected: String::new(),
            live: vec![],
            focus: 0,
            next: 0,
            compounds: vec![],
            pending: vec![],
            watches: vec![],
            filters: vec![],
            open: [true; 4],
            expanded: HashSet::new(),
            editing: None,
            dialog: None,
            confirm_clear: false,
            views: Default::default(),
            picking: None,
            choices: None,
        }
    }
}

fn run_in_terminal(args: &Value, env: &mut Env) -> Result<Value, String> {
    let list: Vec<String> = args["args"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let (program, rest) = list.split_first().ok_or("runInTerminal was sent without a command.")?;
    let vars: Vec<(String, String)> = args["env"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect();
    let cwd = args["cwd"].as_str().filter(|c| !c.is_empty()).map(PathBuf::from).or_else(|| env.folder.map(Path::to_path_buf));
    let title = args["title"].as_str().unwrap_or("Debug");
    let pid = env.bottom.launch(title, program, rest, cwd.as_deref(), &vars, env.ctx)?;
    *env.panel = true;
    Ok(pid.map_or(json!({}), |pid| json!({"processId": pid})))
}

impl Debug {
    pub fn active(&self) -> bool {
        !self.live.is_empty()
    }

    pub fn paused(&self) -> bool {
        self.live.get(self.focus).is_some_and(|l| l.state == State::Stopped)
    }

    fn tag(&self, index: usize) -> String {
        if self.live.len() > 1 { self.live[index].config.name.clone() } else { String::new() }
    }

    fn focus_on(&mut self, index: usize, marks: &mut Marks, cmds: &mut Vec<Command>) {
        self.focus = index;
        let live = &self.live[index];
        live.focus_marks(marks);
        let Some(frame) = live.frames.get(live.frame) else { return };
        if let Some(path) = frame.path.clone().filter(|p| p.is_file()) {
            cmds.extend([Command::OpenPath(path), Command::GoTo(frame.line.max(1), frame.column.max(1))]);
        }
    }

    fn reload(&mut self, workspace: &Workspace, folder: Option<&Path>, now: f64, force: bool) {
        if !force && self.stamp.0.as_deref() == folder && now - self.stamp.2 < 1.0 {
            return;
        }
        let multi = workspace.is_multi();
        let roots: Vec<(PathBuf, String)> = match multi {
            true => workspace.roots().into_iter().zip(workspace.names()).collect(),
            false => folder.map(|f| (f.to_path_buf(), String::new())).into_iter().collect(),
        };
        let file = workspace.file.clone().filter(|_| multi);
        let files = roots.iter().map(|(r, _)| launch_path(r)).chain(file.clone());
        let modified = files.filter_map(|p| std::fs::metadata(p).ok()?.modified().ok()).max().map(|t| t + std::time::Duration::from_nanos(crate::trust::generation()));
        let unchanged = self.stamp.0.as_deref() == folder && self.stamp.1 == modified && self.stamp.2 > f64::NEG_INFINITY;
        self.stamp = (folder.map(Path::to_path_buf), modified, now);
        if unchanged {
            return;
        }
        let mut texts: Vec<(PathBuf, String, String)> =
            roots.into_iter().filter(|(root, _)| crate::trust::trusted(root)).filter_map(|(root, tag)| Some((root.clone(), tag, std::fs::read_to_string(launch_path(&root)).ok()?))).collect();
        let shared = file.and_then(|f| Workspace::open(&f).ok()).unwrap_or_else(|| workspace.clone());
        if let (true, Some(launch), Some(first)) = (multi, shared.section("launch"), workspace.first().filter(|f| crate::trust::trusted(f))) {
            texts.push((first, "workspace".into(), launch.to_string()));
        }
        self.present = !texts.is_empty();
        (self.configs, self.compounds, self.error) = launch_configs(&texts);
        if !self.configs.iter().any(|c| c.name == self.selected) && !self.compounds.iter().any(|c| c.name == self.selected) {
            self.selected = self.configs.first().map(|c| c.name.clone()).unwrap_or_default();
        }
    }

    fn start(&mut self, no_debug: bool, env: &mut Env) {
        let now = env.ctx.input(|i| i.time);
        self.picking = None;
        self.reload(env.workspace, env.folder, now, true);
        let Some(folder) = env.folder.filter(|_| self.present) else { return env.cmds.push(Command::RunFile) };
        if let Some(error) = &self.error {
            return ui::notify(error.clone());
        }
        let compound = self.compounds.iter().find(|c| c.name == self.selected && !self.configs.iter().any(|x| x.name == c.name)).cloned();
        let names = match &compound {
            Some(compound) => compound.configurations.clone(),
            None => match self.configs.iter().find(|c| c.name == self.selected).or(self.configs.first()) {
                Some(config) => vec![config.name.clone()],
                None => return ui::notify("launch.json has no configurations. Use Debug: Add Configuration... to add one."),
            },
        };
        let resolve = |c: &Config| c.resolve(&Vars { folder: c.folder.clone().unwrap_or_else(|| folder.to_path_buf()), file: env.file.clone(), all: env.workspace.clone() });
        let configs: Vec<Config> = names.iter().filter_map(|n| self.configs.iter().find(|c| c.name == *n)).map(resolve).collect();
        let Some(compound) = compound else { return self.launch(configs, no_debug, None, env) };
        let missing: Vec<&str> = names.iter().filter(|n| !self.configs.iter().any(|c| c.name == **n)).map(String::as_str).collect();
        if !missing.is_empty() {
            return ui::notify_error(format!("Compound \"{}\" names configurations that launch.json does not have: {}", compound.name, missing.join(", ")));
        }
        let group = compound.stop_all.then(|| compound.name.clone());
        let Some(task) = compound.pre_launch_task else { return self.launch(configs, no_debug, group, env) };
        match env.tasks.run_label(&task, env.workspace, env.file.clone(), env.settings, env.bottom, env.ctx) {
            Ok(done) => self.pending.push((configs, no_debug, group, done)),
            Err(error) => ui::notify(format!("preLaunchTask \"{task}\": {error}")),
        }
    }

    fn launch(&mut self, mut configs: Vec<Config>, no_debug: bool, group: Option<String>, env: &mut Env) {
        let rest = configs.split_off(configs.iter().position(|c| c.raw.values().any(procs::wants)).unwrap_or(configs.len()));
        if !rest.is_empty() {
            self.picking = Some((rest, no_debug, group.clone(), Some(procs::list(env.ctx))));
        }
        for mut config in configs {
            match config.pre_launch_task.take() {
                Some(task) => match env.tasks.run_label(&task, env.workspace, env.file.clone(), env.settings, env.bottom, env.ctx) {
                    Ok(done) => self.pending.push((vec![config], no_debug, group.clone(), done)),
                    Err(error) => ui::notify(format!("preLaunchTask \"{task}\": {error}")),
                },
                None => {
                    let count = self.live.len();
                    self.begin(config, no_debug, env);
                    if let Some(live) = self.live.get_mut(count) {
                        live.group = group.clone();
                    }
                }
            }
        }
    }

    pub(crate) fn begin(&mut self, config: Config, no_debug: bool, env: &mut Env) {
        let client = match config.raw.get("debugServer").and_then(Value::as_u64) {
            Some(port) => dap::Client::connect(port as u16, env.ctx),
            None => {
                let Some(command) = env.settings.debug_adapters.get(&config.kind) else {
                    let text = format!("No debug adapter is set for type \"{}\". Add one under \"debug_adapters\" in settings.json.", config.kind);
                    return ui::notify(text);
                };
                let command: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
                match dap::Client::spawn(&command, config.folder.as_deref().or(env.folder), env.ctx) {
                    Ok(client) => {
                        crate::output::log("Debug", format!("Started debug adapter: {}", command.join(" ")));
                        client
                    }
                    Err(error) => {
                        crate::output::log("Debug", format!("[error] {error}"));
                        return ui::notify(error);
                    }
                }
            }
        };
        let mut live = Live {
            client,
            config,
            no_debug,
            caps: Value::Null,
            state: State::Starting,
            reason: String::new(),
            threads: vec![],
            thread: None,
            frames: vec![],
            frame: 0,
            scopes: vec![],
            children: HashMap::new(),
            asked: HashSet::new(),
            watch: HashMap::new(),
            wants: HashMap::new(),
            function_status: HashMap::new(),
            data: vec![],
            configured: false,
            stopping: 0,
            restart: false,
            temporary: None,
            id: self.next,
            group: None,
            place: None,
        };
        self.next += 1;
        let args = json!({
            "clientID": "halo",
            "clientName": "Halo",
            "adapterID": live.config.kind,
            "locale": "en",
            "pathFormat": "path",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
            "supportsRunInTerminalRequest": true,
            "supportsMemoryReferences": true,
        });
        live.ask("initialize", args, Want::Initialize);
        let name = live.config.name.clone();
        if self.live.is_empty() {
            env.bottom.console.begin(&name);
            env.marks.reset(true);
        } else {
            env.bottom.console.tagged(&name, "console", &format!("Starting {name}\n"));
        }
        env.bottom.show_debug();
        *env.panel = true;
        live.focus_marks(env.marks);
        self.live.push(live);
        self.focus = self.live.len() - 1;
    }

    fn end(&mut self, index: usize, env: &mut Env) {
        if index >= self.live.len() {
            return;
        }
        let tag = self.tag(index);
        let was_focused = self.focus == index;
        let mut live = self.live.remove(index);
        live.clear_temporary(env.marks);
        if live.stopping < 2 {
            live.client.request("disconnect", json!({"restart": false, "terminateDebuggee": live.config.request == "launch"}));
        }
        if self.focus > index {
            self.focus -= 1;
        }
        self.focus = self.focus.min(self.live.len().saturating_sub(1));
        if self.live.is_empty() {
            env.marks.reset(false);
            env.bottom.console.active = false;
        } else if was_focused {
            self.live[self.focus].focus_marks(env.marks);
        }
        env.bottom.console.tagged(&tag, "console", "Debugging ended.\n");
        if live.restart {
            let (config, no_debug, group) = (Config { pre_launch_task: None, ..live.config.clone() }, live.no_debug, live.group.clone());
            drop(live);
            return self.launch(vec![config], no_debug, group, env);
        }
        let Some(group) = live.group else { return };
        for at in 0..self.live.len() {
            if self.live[at].group.as_ref() == Some(&group) && self.live[at].stopping == 0 {
                self.stop(at, env);
            }
        }
    }

    fn stop(&mut self, index: usize, env: &mut Env) {
        let Some(live) = self.live.get_mut(index) else { return };
        live.clear_temporary(env.marks);
        live.stopping += 1;
        match live.stopping {
            1 if live.can("supportsTerminateRequest") && live.config.request == "launch" => live.ask("terminate", json!({}), Want::Plain("terminate".into())),
            1 | 2 => {
                let args = json!({"restart": false, "terminateDebuggee": live.config.request == "launch"});
                live.stopping = 2;
                live.ask("disconnect", args, Want::Disconnect);
            }
            _ => self.end(index, env),
        }
    }

    fn step(&mut self, command: &str) {
        let Some(live) = self.live.get_mut(self.focus).filter(|l| l.state == State::Stopped) else { return };
        if live.moving() {
            return;
        }
        let thread = live.thread.or(live.threads.first().map(|t| t.0)).unwrap_or(0);
        let mut args = json!({"threadId": thread});
        if self.views.focused && command != "continue" && live.can("supportsSteppingGranularity") {
            args["granularity"] = json!("instruction");
        }
        live.ask(command, args, Want::Run);
    }

    pub fn exec(&mut self, cmd: &Command, env: &mut Env) -> bool {
        use Command::*;
        match cmd {
            StartDebugging if self.paused() => self.step("continue"),
            StartDebugging => self.start(false, env),
            RunWithoutDebugging => self.start(true, env),
            StopDebugging => {
                self.pending.clear();
                self.picking = None;
                self.stop(self.focus, env);
            }
            RestartDebugging => match self.live.get_mut(self.focus) {
                None => self.start(false, env),
                Some(live) if live.can("supportsRestartRequest") => {
                    live.clear_temporary(env.marks);
                    let args = json!({"arguments": Value::Object(live.config.raw.clone())});
                    live.ask("restart", args, Want::Restart);
                }
                Some(live) => {
                    live.restart = true;
                    self.stop(self.focus, env);
                }
            },
            PauseDebugging => {
                if let Some(live) = self.live.get_mut(self.focus).filter(|l| l.state == State::Running) {
                    let thread = live.thread.or(live.threads.first().map(|t| t.0)).unwrap_or(0);
                    live.ask("pause", json!({"threadId": thread}), Want::Plain("pause".into()));
                }
            }
            RunToCursor | JumpToCursor => {
                if let (Some(live), Some(path), Some(line)) = (self.live.get_mut(self.focus), env.file.as_deref(), env.cursor_line) {
                    live.start_cursor_command(cmd, path, line, env.marks);
                }
            }
            StepOver => self.step("next"),
            StepInto => self.step("stepIn"),
            StepOut => self.step("stepOut"),
            AddConfiguration => match env.folder {
                Some(folder) => match add_configuration(folder, env.file.as_deref()) {
                    Ok(path) => {
                        self.stamp.2 = f64::NEG_INFINITY;
                        env.cmds.push(OpenPath(path));
                    }
                    Err(error) => ui::notify(error),
                },
                None => ui::notify("Open a folder to add a launch configuration."),
            },
            RemoveAllBreakpoints => self.confirm_clear = self.breakpoint_count(env.marks) > 0,
            AddFunctionBreakpoint | BreakOnValueChange => {
                if self.breakpoint_commands(env.folder.is_some()).allows(cmd) {
                    if *cmd == AddFunctionBreakpoint {
                        self.open[3] = true;
                        self.editing = Some(Editing::FunctionName(String::new()));
                    } else {
                        self.open[0] = true;
                        self.editing = Some(Editing::PickData(self.live[self.focus].id));
                    }
                }
            }
            AddWatch => {
                self.open[1] = true;
                self.editing = Some(Editing::Watch(None, String::new()));
            }
            OpenDisassembly => self.views.open_disassembly(self.live.get(self.focus)),
            OpenMemory => self.views.ask_memory(self.live.get(self.focus)),
            AttachProcess(pid) => {
                if let Some((mut configs, no_debug, group, _)) = self.picking.take() {
                    configs[0].raw.values_mut().for_each(|v| procs::fill(v, *pid));
                    self.launch(configs, no_debug, group, env);
                }
            }
            _ => return false,
        }
        env.marks.cursor = self.live.get(self.focus).map(Live::cursor_commands).unwrap_or_default();
        true
    }

    pub fn poll(&mut self, env: &mut Env) {
        let mut at = 0;
        while at < self.pending.len() {
            let Ok(outcome) = self.pending[at].3.try_recv() else {
                at += 1;
                continue;
            };
            let (configs, no_debug, group, _) = self.pending.remove(at);
            match outcome.ok {
                true => self.launch(configs, no_debug, group, env),
                false => ui::notify(outcome.error.unwrap_or_else(|| "The preLaunchTask failed, so debugging did not start.".to_owned())),
            }
        }
        if let Some(picking) = self.picking.as_mut()
            && let Some(found) = picking.3.as_ref().and_then(|rx| rx.try_recv().ok())
        {
            picking.3 = None;
            match found.is_empty() {
                true => {
                    self.picking = None;
                    ui::notify_error("Could not list the running processes, so debugging did not start.");
                }
                false => self.choices = Some(procs::choices(found)),
            }
        }
        let now = env.ctx.input(|i| i.time);
        if std::mem::take(&mut env.marks.functions_changed) {
            for live in self.live.iter_mut().filter(|l| l.configured) {
                live.send_functions(&env.marks.functions);
            }
        }
        if self.live.is_empty() {
            env.bottom.console.submitted.clear();
            return;
        }
        if self.live.iter().any(|l| l.configured) {
            for path in std::mem::take(&mut env.marks.changed) {
                let list = env.marks.of(&path).to_vec();
                self.live.iter_mut().filter(|l| l.configured).for_each(|l| l.send_breakpoints(&path, &list));
            }
        }
        for id in self.live.iter().map(|l| l.id).collect::<Vec<_>>() {
            let Some(index) = self.live.iter().position(|l| l.id == id) else { continue };
            for message in self.live[index].client.poll() {
                let Some(index) = self.live.iter().position(|l| l.id == id) else { break };
                self.handle(index, message, env);
            }
        }
        env.marks.stopped = self.live.get(self.focus).and_then(|l| l.place.clone());
        env.marks.paused = self.live.iter().filter_map(|l| l.place.clone()).collect();
        let Some(live) = self.live.get_mut(self.focus) else { return };
        for expression in std::mem::take(&mut env.bottom.console.submitted) {
            live.evaluate(&expression, "repl", Want::Repl);
        }
        if let Some(hover) = env.marks.hover.as_mut().filter(|h| !h.asked && now - h.since >= 0.35 && live.state == State::Stopped) {
            hover.asked = true;
            let word = hover.word.clone();
            live.evaluate(&word, "hover", Want::Hover(word.clone()));
        }
        let path = env.marks.stopped.as_ref().map(|s| s.0.as_path());
        live.fetch(&self.expanded, path.is_some() && env.settings.of(path).inline_values);
        env.marks.cursor = live.cursor_commands();
        self.views.poll(live, env.marks);
    }

    fn handle(&mut self, index: usize, message: Incoming, env: &mut Env) {
        let tag = self.tag(index);
        let Some(live) = self.live.get_mut(index) else { return };
        match message {
            Incoming::Response { request, command, success, message, body } => {
                let want = live.wants.remove(&request).unwrap_or(Want::Plain(command.clone()));
                self.reply(index, want, success, message, body, env);
            }
            Incoming::Event(name, body) => self.event(index, &name, &body, env),
            Incoming::Request { seq, command, args } => {
                let result = match command.as_str() {
                    "runInTerminal" => run_in_terminal(&args, env),
                    _ => Err(format!("Halo does not support the {command} request.")),
                };
                if let Err(error) = &result {
                    env.bottom.console.tagged(&tag, "stderr", &format!("{error}\n"));
                }
                live.client.respond(seq, &command, result);
            }
            Incoming::Stderr(line) => {
                crate::output::log("Debug", format!("[{}] {line}", live.config.name));
                env.bottom.console.tagged(&tag, "console", &format!("{line}\n"))
            }
            Incoming::Closed(error) => {
                crate::output::log("Debug", format!("The debug adapter for \"{}\" exited{}", live.config.name, error.as_ref().map_or(String::new(), |e| format!(": {e}"))));
                if let Some(error) = error {
                    ui::notify(error);
                } else if live.state == State::Starting && live.stopping == 0 {
                    ui::notify(format!("The debug adapter for \"{}\" exited before the session started. See the Debug Console.", live.config.name));
                }
                live.stopping = 2;
                self.end(index, env);
            }
        }
    }

    fn reply(&mut self, index: usize, want: Want, success: bool, message: String, body: Value, env: &mut Env) {
        let tag = self.tag(index);
        let Some(live) = self.live.get_mut(index) else { return };
        match want {
            Want::Initialize if !success => {
                ui::notify(format!("The debug adapter could not be initialized: {message}"));
                self.end(index, env);
            }
            Want::Initialize => {
                live.caps = body;
                let mut args = Value::Object(live.config.raw.clone());
                if live.no_debug {
                    args["noDebug"] = json!(true);
                }
                let request = if live.config.request == "attach" { "attach" } else { "launch" };
                live.ask(request, args, Want::Launch);
                let old = std::mem::take(&mut self.filters);
                for filter in self.live.iter().flat_map(|l| l.caps["exceptionBreakpointFilters"].as_array().into_iter().flatten()) {
                    let id = filter["filter"].as_str().unwrap_or_default().to_owned();
                    if self.filters.iter().any(|f| f.0 == id) {
                        continue;
                    }
                    let on = old.iter().find(|f| f.0 == id).map_or(filter["default"] == true, |f| f.2);
                    self.filters.push((id, filter["label"].as_str().unwrap_or_default().to_owned(), on));
                }
            }
            Want::Launch if !success => {
                let text = format!("{} \"{}\" failed: {message}", live.config.request, live.config.name);
                env.bottom.console.tagged(&tag, "stderr", &format!("{text}\n"));
                ui::notify(text);
                self.end(index, env);
            }
            Want::Launch => {
                if live.state == State::Starting {
                    live.state = State::Running;
                }
            }
            Want::Functions(names) => {
                live.function_status = breakpoints::verification(names, &body);
                if !success { ui::notify(message); }
            }
            Want::DataInfo(name) if success => match DataBreakpoint::from_info(name, &body) {
                Ok(point) => live.add_data(point),
                Err(description) => ui::notify(description),
            },
            Want::Data(ids) if success => {
                let mut status = breakpoints::verification(ids, &body);
                for point in &mut live.data {
                    point.status = status.remove(&point.id).unwrap_or_default();
                }
            }
            Want::DataInfo(_) | Want::Data(_) => ui::notify(message),
            Want::Breakpoints(path) if success => env.marks.verify(&path, &body),
            Want::Threads if success => {
                live.threads = body["threads"].as_array().into_iter().flatten().map(|t| (t["id"].as_i64().unwrap_or(0), t["name"].as_str().unwrap_or_default().to_owned())).collect();
                if live.state == State::Stopped && live.thread.is_none() && let Some(&(thread, _)) = live.threads.first() {
                    live.thread = Some(thread);
                    live.stack(thread);
                }
            }
            Want::Stack if success => {
                let frame = |f: &Value| Frame {
                    id: f["id"].as_i64().unwrap_or(0),
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    path: f["source"]["path"].as_str().map(PathBuf::from),
                    line: f["line"].as_u64().unwrap_or(0) as usize,
                    column: f["column"].as_u64().unwrap_or(0) as usize,
                    ip: f["instructionPointerReference"].as_str().map(str::to_owned),
                };
                live.frames = body["stackFrames"].as_array().into_iter().flatten().map(frame).collect();
                if live.state == State::Stopped {
                    self.focus = index;
                    let index = live.frames.iter().position(|f| f.path.as_deref().is_some_and(Path::is_file)).unwrap_or(0);
                    live.select(index, &self.watches, env.marks, env.cmds);
                    let at = live.frames.get(index).map(|f| format!("{}:{}", f.path.as_deref().and_then(Path::file_name).unwrap_or_default().to_string_lossy(), f.line));
                    crate::a11y::announce(format!("Paused on {} at {}", live.reason, at.unwrap_or_default()), true);
                }
            }
            Want::Scopes(frame) if success && live.frame_id() == Some(frame) => {
                live.scopes = vars(&body, "scopes");
                if !live.scopes.iter().any(|s| self.expanded.contains(&format!("/{}", s.name))) {
                    self.expanded.extend(live.scopes.iter().find(|s| !s.expensive).map(|s| format!("/{}", s.name)));
                }
            }
            Want::Variables(reference) => {
                live.children.insert(reference, if success { vars(&body, "variables") } else { vec![] });
                if index == self.focus && live.scopes.iter().any(|s| !s.expensive && s.reference == reference) {
                    live.refresh_inline(env.marks);
                }
            }
            Want::Watch(expression) => {
                live.watch.insert(expression, if success { Ok(body["result"].as_str().unwrap_or_default().to_owned()) } else { Err(message) });
            }
            Want::Repl => match success {
                true => env.bottom.console.tagged(&tag, "repl", &format!("{}\n", body["result"].as_str().unwrap_or_default())),
                false => env.bottom.console.tagged(&tag, "stderr", &format!("{message}\n")),
            },
            Want::Hover(word) if success && index == self.focus => {
                if let Some(hover) = env.marks.hover.as_mut().filter(|h| h.word == word) {
                    hover.value = body["result"].as_str().map(str::to_owned);
                }
            }
            Want::SetVariable(reference, name) if success => {
                if let Some(var) = live.children.get_mut(&reference).into_iter().flatten().find(|v| v.name == name) {
                    var.value = body["value"].as_str().unwrap_or_default().to_owned();
                }
                if index == self.focus {
                    live.refresh_inline(env.marks);
                }
                live.watch_all(&self.watches);
            }
            Want::SetVariable(..) => ui::notify(format!("The value could not be set: {message}")),
            Want::RunTo(thread, index) => {
                let Some((path, _)) = &live.temporary else { return };
                if success {
                    env.marks.verify(path, &body);
                }
                if success && body["breakpoints"][index]["verified"] == true && live.stopping == 0 {
                    live.ask("continue", json!({"threadId": thread}), Want::Run);
                } else {
                    live.clear_temporary(env.marks);
                    let reason = body["breakpoints"][index]["message"].as_str().unwrap_or(&message);
                    ui::notify(format!("Run to Cursor could not set the breakpoint. {reason}"));
                }
            }
            Want::GotoTargets(thread) if success && live.stopping == 0 => {
                if let Some(target) = cursor::first_target(&body) {
                    live.ask("goto", json!({"threadId": thread, "targetId": target}), Want::Goto);
                } else {
                    ui::notify("The debug adapter found no jump target on this line.");
                }
            }
            Want::Goto | Want::GotoTargets(_) if !success => ui::notify(format!("Jump to Cursor failed: {message}")),
            Want::Goto | Want::Run if success => live.running((index == self.focus).then_some(env.marks)),
            Want::Disconnect => self.end(index, env),
            Want::Restart if !success => {
                live.restart = true;
                self.stop(index, env);
            }
            Want::Restart => live.running((index == self.focus).then_some(env.marks)),
            Want::Run => {
                live.clear_temporary(env.marks);
                env.bottom.console.tagged(&tag, "stderr", &format!("{message}\n"));
            }
            Want::Plain(command) if !success => env.bottom.console.tagged(&tag, "stderr", &format!("{command}: {message}\n")),
            Want::Disassemble(ip) => self.views.disassembled(live, &ip, success, &message, &body, env.marks),
            Want::ReadMemory(reference) => self.views.memory_read(&reference, success, &message, &body),
            Want::MemoryAt(expression) => self.views.memory_found(live, &expression, success, &message, &body),
            _ => {}
        }
    }

    fn event(&mut self, index: usize, name: &str, body: &Value, env: &mut Env) {
        let tag = self.tag(index);
        let Some(live) = self.live.get_mut(index) else { return };
        match name {
            "initialized" => live.configure(env.marks, &self.filters),
            "stopped" => {
                live.clear_temporary(env.marks);
                live.running((index == self.focus).then_some(env.marks));
                live.state = State::Stopped;
                live.reason = body["description"].as_str().or(body["reason"].as_str()).unwrap_or("paused").to_owned();
                live.thread = body["threadId"].as_i64();
                live.ask("threads", json!({}), Want::Threads);
                if let Some(thread) = live.thread {
                    live.stack(thread);
                }
            }
            "continued" => live.running((index == self.focus).then_some(env.marks)),
            "exited" => env.bottom.console.tagged(&tag, "console", &format!("Process exited with code {}.\n", body["exitCode"])),
            "terminated" => self.end(index, env),
            "output" if body["category"] != "telemetry" => {
                let category = body["category"].as_str().unwrap_or("console");
                env.bottom.console.tagged(&tag, category, body["output"].as_str().unwrap_or_default());
            }
            "breakpoint" => {
                if !live.update_breakpoint(&body["breakpoint"]) {
                    env.marks.update(&body["breakpoint"]);
                }
            }
            "thread" => live.ask("threads", json!({}), Want::Threads),
            _ => {}
        }
    }

    pub fn status(&self) -> Option<(String, String)> {
        let live = self.live.get(self.focus)?;
        let tip = match self.live.len() {
            1 => format!("Debugging {} ({})", live.config.name, live.config.kind),
            count => format!("Debugging {} ({}), {count} sessions running", live.config.name, live.config.kind),
        };
        Some((format!("{} {} ({})", icons::BUG, live.config.name, live.shown_state()), tip))
    }
}

fn row(ui: &mut Ui, theme: &Theme, selected: bool) -> (Rect, Response) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    ui::row_bg(ui, theme, r, resp.id, resp.hovered(), selected);
    (r, resp)
}

fn pair(ui: &Ui, r: Rect, x: f32, theme: &Theme, name: &str, value: &str, value_color: Color32) {
    let painter = ui.painter().with_clip_rect(r);
    let font = FontId::proportional(12.5);
    let shown = painter.text(pos2(x, r.center().y), Align2::LEFT_CENTER, name, font.clone(), theme.text);
    if !value.is_empty() {
        let first = value.lines().next().unwrap_or_default();
        painter.text(pos2(shown.right() + 6.0, r.center().y), Align2::LEFT_CENTER, first, font, value_color);
    }
}

fn hover_button(ui: &mut Ui, theme: &Theme, r: Rect, slot: usize, icon: &str, tip: &str) -> bool {
    let rect = Rect::from_center_size(pos2(r.right() - 14.0 - slot as f32 * 22.0, r.center().y), vec2(20.0, 20.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    ui::icon_button(&mut child, theme, icon, 20.0, false, tip).clicked()
}

fn inline_edit(ui: &mut Ui, text: &mut String, hint: &str) -> Option<bool> {
    let resp = ui.add(TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY));
    if resp.lost_focus() {
        return Some(!ui.input(|i| i.key_pressed(Key::Escape)));
    }
    if !resp.has_focus() {
        resp.request_focus();
    }
    None
}

enum TreeAction {
    Toggle(String),
    Edit(i64, String, String),
    Commit(i64, String, String),
    Cancel,
    Memory(String, String),
    BreakOnChange(i64, String),
}

struct Tree<'a> {
    live: &'a Live,
    expanded: &'a HashSet<String>,
    editing: &'a mut Option<Editing>,
    settable: bool,
    action: &'a mut Option<TreeAction>,
    memory: bool,
}

impl Tree<'_> {
    fn show(&mut self, ui: &mut Ui, theme: &Theme, vars: &[Var], parent: i64, depth: usize, prefix: &str) {
        for var in vars {
            let path = format!("{prefix}/{}", var.name);
            let open = var.reference > 0 && self.expanded.contains(&path);
            if let Some(Editing::Value(reference, name, text)) = self.editing.as_mut()
                && *reference == parent
                && *name == var.name
            {
                ui.horizontal(|ui| {
                    ui.add_space(24.0 + depth as f32 * 12.0);
                    match inline_edit(ui, text, "New value") {
                        Some(true) => *self.action = Some(TreeAction::Commit(parent, var.name.clone(), text.clone())),
                        Some(false) => *self.action = Some(TreeAction::Cancel),
                        None => {}
                    }
                });
                continue;
            }
            let (r, resp) = row(ui, theme, false);
            let x = r.left() + 12.0 + depth as f32 * 12.0;
            ui::tree_item(ui, &resp, &format!("{} {}", var.name, var.value), false, depth, (var.reference > 0).then_some(open));
            if var.reference > 0 {
                ui::caret(ui.painter(), pos2(x + 4.0, r.center().y), ui::anim(ui, resp.id.with("open"), open), theme.text_dim);
            }
            let picking_data = parent != 0 && matches!(self.editing, Some(Editing::PickData(id)) if *id == self.live.id);
            let mut label = r;
            if picking_data { label.max.x -= 24.0; }
            if parent == 0 {
                pair(ui, label, x + 14.0, theme, &var.name, "", theme.text_dim);
            } else {
                pair(ui, label, x + 14.0, theme, &format!("{}:", var.name), &var.value, theme.text_dim);
            }
            let resp = match (parent, var.kind.is_empty()) {
                (0, _) => resp,
                (_, true) => resp.on_hover_text(&var.value),
                _ => resp.on_hover_text(format!("{}: {}", var.kind, var.value)),
            };
            if resp.clicked() && var.reference > 0 {
                *self.action = Some(TreeAction::Toggle(path.clone()));
            }
            if picking_data && hover_button(ui, theme, r, 0, icons::PLUS, "Break on Value Change") {
                *self.action = Some(TreeAction::BreakOnChange(parent, var.name.clone()));
            }
            if parent != 0 {
                resp.context_menu(|ui| {
                    ui::menu_pad(ui);
                    if self.live.can("supportsDataBreakpoints") && ui::menu_item(ui, "Break on Value Change").clicked() {
                        *self.action = Some(TreeAction::BreakOnChange(parent, var.name.clone()));
                        ui.close();
                    }
                    if self.settable && ui::menu_item(ui, "Set Value").clicked() {
                        *self.action = Some(TreeAction::Edit(parent, var.name.clone(), var.value.clone()));
                        ui.close();
                    }
                    if ui::menu_item(ui, "Copy Value").clicked() {
                        ui.ctx().copy_text(var.value.clone());
                        ui.close();
                    }
                    if let Some(reference) = var.memory.as_ref().filter(|_| self.memory)
                        && ui::menu_item(ui, "View Binary Data").clicked()
                    {
                        *self.action = Some(TreeAction::Memory(reference.clone(), var.name.clone()));
                        ui.close();
                    }
                });
            }
            match self.live.children.get(&var.reference).filter(|_| open) {
                Some(found) => self.show(ui, theme, found, var.reference, depth + 1, &path),
                None if open => ui::skeleton(ui, theme, 1),
                None => {}
            }
        }
    }
}

impl Debug {
    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, env: &mut Env) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let now = ui.input(|i| i.time);
        self.reload(env.workspace, env.folder, now, false);
        ui::section_header(ui, theme, "Run and Debug");
        if env.folder.is_none() {
            return note(ui, theme, "Open a folder to debug its programs.");
        }
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            if !self.present {
                ui.label(RichText::new("No launch.json in this folder. F5 runs the active file.").size(12.0).color(theme.text_dim));
                if ui.button(RichText::new("Create a launch.json File").color(theme.accent)).clicked() {
                    env.cmds.push(Command::AddConfiguration);
                }
                return;
            }
            if let Some(error) = &self.error {
                ui.add(egui::Label::new(RichText::new(error).size(12.0).color(theme.error)).wrap());
            }
            ui.horizontal(|ui| {
                if ui::icon_button(ui, theme, icons::PLAY, 24.0, false, "Start Debugging (F5)").clicked() {
                    env.cmds.push(Command::SaveAll);
                    self.start(false, env);
                }
                let width = (ui.available_width() - 30.0).max(60.0);
                egui::ComboBox::from_id_salt("debug_config").selected_text(&self.selected).width(width).show_ui(ui, |ui| {
                    for config in &self.configs {
                        ui.selectable_value(&mut self.selected, config.name.clone(), &config.name);
                    }
                    if !self.compounds.is_empty() {
                        ui.separator();
                    }
                    for compound in &self.compounds {
                        ui.selectable_value(&mut self.selected, compound.name.clone(), &compound.name);
                    }
                    ui.separator();
                    if ui.button("Add Configuration...").clicked() {
                        env.cmds.push(Command::AddConfiguration);
                    }
                });
                if ui::icon_button(ui, theme, icons::PENCIL_SIMPLE, 24.0, false, "Edit Configuration in a Form").clicked() {
                    env.cmds.push(Command::EditLaunchForm);
                }
                if ui::icon_button(ui, theme, icons::GEAR, 24.0, false, "Open launch.json").clicked()
                    && let Some(folder) = env.folder
                {
                    env.cmds.push(Command::OpenPath(launch_path(folder)));
                }
            });
        });
        ScrollArea::vertical().id_salt("debug_view").auto_shrink(false).show(ui, |ui| {
            ui::section(ui, theme, "VARIABLES", &mut self.open[0]);
            if self.open[0] {
                self.variables(ui, theme);
            }
            ui::section(ui, theme, "WATCH", &mut self.open[1]);
            if self.open[1] {
                self.watch_ui(ui, theme);
            }
            ui::section(ui, theme, "CALL STACK", &mut self.open[2]);
            if self.open[2] {
                self.stack_ui(ui, theme, env);
            }
            let rect = ui::section(ui, theme, "BREAKPOINTS", &mut self.open[3]).rect;
            if self.breakpoint_commands(true).allows(&Command::AddFunctionBreakpoint)
                && hover_button(ui, theme, rect, 1, icons::PLUS, "Add Function Breakpoint")
            {
                env.cmds.push(Command::AddFunctionBreakpoint);
            }
            if self.breakpoint_count(env.marks) > 0 {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(20.0, 20.0))));
                if ui::icon_button(&mut child, theme, icons::X_SQUARE, 20.0, false, "Remove All Breakpoints").clicked() {
                    self.confirm_clear = true;
                }
            }
            if self.open[3] {
                self.breakpoints_ui(ui, theme, env);
            }
        });
    }

    fn variables(&mut self, ui: &mut Ui, theme: &Theme) {
        let Some(live) = self.live.get(self.focus).filter(|l| l.state == State::Stopped) else {
            return note(ui, theme, if self.active() { "Variables show while the program is paused." } else { "Not debugging." });
        };
        if live.scopes.is_empty() {
            return ui::skeleton(ui, theme, 3);
        }
        if matches!(self.editing, Some(Editing::PickData(id)) if id == live.id) {
            ui.horizontal(|ui| {
                ui.label("Choose + beside a variable.");
                if ui.button("Cancel").clicked() { self.editing = None; }
            });
        }
        let mut action = None;
        let mut editing = self.editing.take();
        let memory = live.can("supportsReadMemoryRequest");
        let mut tree = Tree { live, expanded: &self.expanded, editing: &mut editing, settable: live.can("supportsSetVariable"), action: &mut action, memory };
        tree.show(ui, theme, &live.scopes, 0, 0, "");
        self.editing = editing;
        match action {
            Some(TreeAction::Toggle(path)) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            }
            Some(TreeAction::BreakOnChange(reference, name)) => {
                self.editing = None;
                self.live[self.focus].data_info(reference, name);
            }
            Some(TreeAction::Edit(reference, name, value)) => self.editing = Some(Editing::Value(reference, name, value)),
            Some(TreeAction::Cancel) => self.editing = None,
            Some(TreeAction::Memory(reference, name)) => {
                if let Some(live) = self.live.get_mut(self.focus) {
                    self.views.open_memory(live, reference, name);
                }
            }
            Some(TreeAction::Commit(reference, name, value)) => {
                self.editing = None;
                if let Some(live) = self.live.get_mut(self.focus) {
                    live.ask("setVariable", json!({"variablesReference": reference, "name": name, "value": value}), Want::SetVariable(reference, name));
                }
            }
            None => {}
        }
    }

    fn watch_ui(&mut self, ui: &mut Ui, theme: &Theme) {
        let mut remove = None;
        let mut commit = None;
        let adding = matches!(self.editing, Some(Editing::Watch(None, _)));
        for (index, expression) in self.watches.iter().enumerate() {
            if let Some(Editing::Watch(Some(at), text)) = self.editing.as_mut().filter(|e| matches!(e, Editing::Watch(Some(at), _) if *at == index)) {
                let at = *at;
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    commit = inline_edit(ui, text, "Expression to watch").map(|ok| (Some(at), ok.then(|| text.clone())));
                });
                continue;
            }
            let (r, resp) = row(ui, theme, false);
            let value = self.live.get(self.focus).and_then(|l| l.watch.get(expression));
            ui::item(ui, &resp, ui::Role::ListItem, expression, false);
            let (shown, color) = match value {
                Some(Ok(value)) => (value.clone(), theme.text_dim),
                Some(Err(error)) => (error.clone(), theme.error),
                None => (String::from(if self.paused() { "..." } else { "not available" }), theme.text_dim),
            };
            pair(ui, r, r.left() + 26.0, theme, &format!("{expression}:"), &shown, color);
            if resp.hovered() || ui.rect_contains_pointer(r) {
                if hover_button(ui, theme, r, 0, icons::X, "Remove Expression") {
                    remove = Some(index);
                }
                if hover_button(ui, theme, r, 1, icons::PENCIL_SIMPLE, "Edit Expression") {
                    self.editing = Some(Editing::Watch(Some(index), expression.clone()));
                }
            }
            if resp.double_clicked() {
                self.editing = Some(Editing::Watch(Some(index), expression.clone()));
            }
        }
        if let Some(Editing::Watch(None, text)) = self.editing.as_mut() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                commit = inline_edit(ui, text, "Expression to watch").map(|ok| (None, ok.then(|| text.clone())));
            });
        } else {
            let (r, resp) = row(ui, theme, false);
            ui.painter().with_clip_rect(r).text(pos2(r.left() + 26.0, r.center().y), Align2::LEFT_CENTER, format!("{}  Add Expression", icons::PLUS), FontId::proportional(12.5), theme.text_dim);
            ui::item(ui, &resp, ui::Role::Button, "Add Expression", false);
            if resp.clicked() && !adding {
                self.editing = Some(Editing::Watch(None, String::new()));
            }
        }
        if let Some((at, text)) = commit {
            self.editing = None;
            match (at, text.filter(|t| !t.trim().is_empty())) {
                (Some(at), Some(text)) => self.watches[at] = text,
                (None, Some(text)) => self.watches.push(text),
                _ => {}
            }
            for live in &mut self.live {
                live.watch_all(&self.watches);
            }
        }
        if let Some(index) = remove {
            self.watches.remove(index);
        }
    }

    fn stack_ui(&mut self, ui: &mut Ui, theme: &Theme, env: &mut Env) {
        if self.live.is_empty() {
            return note(ui, theme, "Not debugging.");
        }
        let multi = self.live.len() > 1;
        let (depth, indent) = (multi as usize, if multi { 14.0 } else { 0.0 });
        let (mut pick, mut disassemble) = (None, false);
        for (session, live) in self.live.iter().enumerate() {
            let can_disassemble = live.can("supportsDisassembleRequest");
            if multi {
                let (r, resp) = row(ui, theme, false);
                let focused = session == self.focus;
                ui::tree_item(ui, &resp, &format!("{} {}", live.config.name, live.shown_state()), focused, 0, Some(true));
                pair(ui, r, r.left() + 12.0, theme, &format!("{} {}", icons::BUG, live.config.name), live.shown_state(), if focused { theme.accent } else { theme.text_dim });
                if resp.clicked() {
                    pick = Some((session, None));
                }
            }
            if live.threads.is_empty() {
                note(ui, theme, if live.state == State::Stopped { "Loading threads..." } else { "Running." });
                continue;
            }
            for (id, name) in &live.threads {
                let (r, thread) = row(ui, theme, false);
                let current = live.thread == Some(*id) && live.state == State::Stopped;
                ui::tree_item(ui, &thread, name, current, depth, Some(current));
                let state = if current { format!("Paused on {}", live.reason) } else { String::from(if live.state == State::Stopped { "Paused" } else { "Running" }) };
                pair(ui, r, r.left() + 12.0 + indent, theme, &format!("{} {name}", icons::STACK), &state, theme.text_dim);
                if !current {
                    continue;
                }
                for (index, frame) in live.frames.iter().enumerate() {
                    let selected = index == live.frame && session == self.focus;
                    let (r, resp) = row(ui, theme, selected);
                    let place = frame.path.as_deref().map(|p| format!("{}:{}", p.file_name().unwrap_or_default().to_string_lossy(), frame.line)).unwrap_or_else(|| "unknown source".into());
                    ui::tree_item(ui, &resp, &format!("{} {place}", frame.name), selected, depth + 1, None);
                    pair(ui, r, r.left() + 26.0 + indent, theme, &frame.name, &place, theme.text_dim);
                    if resp.clicked() {
                        pick = Some((session, Some(index)));
                    }
                    if can_disassemble && frame.ip.is_some() {
                        resp.context_menu(|ui| {
                            ui::menu_pad(ui);
                            if ui::menu_item(ui, "Open Disassembly View").clicked() {
                                (pick, disassemble) = (Some((session, Some(index))), true);
                                ui.close();
                            }
                        });
                    }
                }
            }
        }
        match pick {
            Some((session, Some(index))) => {
                self.focus = session;
                self.live[session].select(index, &self.watches, env.marks, env.cmds);
            }
            Some((session, None)) => self.focus_on(session, env.marks, env.cmds),
            None => {}
        }
        if disassemble {
            self.views.open_disassembly(self.live.get(self.focus));
        }
    }

    fn breakpoints_ui(&mut self, ui: &mut Ui, theme: &Theme, env: &mut Env) {
        let mut send_filters = false;
        for filter in &mut self.filters {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                if ui::checkbox(ui, theme, &mut filter.2, &filter.1).changed() {
                    send_filters = true;
                }
            });
            ui.add_space(4.0);
        }
        if send_filters {
            self.live.iter_mut().filter(|l| l.configured).for_each(|l| l.send_filters(&self.filters));
        }
        self.additional_breakpoints_ui(ui, theme, env.marks);
        if self.breakpoint_count(env.marks) == 0 && self.filters.is_empty() && !matches!(self.editing, Some(Editing::FunctionName(_))) {
            return note(ui, theme, "Click left of a line number or press F9 to add a breakpoint.");
        }
        let mut action = None;
        for (path, list) in &env.marks.files {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            for b in list {
                let (r, resp) = row(ui, theme, false);
                let mut on = b.enabled;
                ui::item(ui, &resp, ui::Role::ListItem, &format!("{name}:{}", b.line + 1), false);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(r.left() + 12.0, r.top() + 3.0), vec2(16.0, 16.0))));
                if ui::checkbox(&mut child, theme, &mut on, "").changed() {
                    action = Some((path.clone(), Gutter::Enable(b.line, on)));
                }
                let detail = [&b.condition, &b.hit, &b.log].into_iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join("  ");
                pair(ui, r, r.left() + 36.0, theme, &format!("{name}:{}", b.line + 1), &detail, theme.text_dim);
                let resp = resp.on_hover_text(path.to_string_lossy());
                if (resp.hovered() || ui.rect_contains_pointer(r)) && hover_button(ui, theme, r, 0, icons::X, "Remove Breakpoint") {
                    action = Some((path.clone(), Gutter::Remove(b.line)));
                } else if resp.clicked() {
                    env.cmds.extend([Command::OpenPath(path.clone()), Command::GoTo(b.line + 1, 1)]);
                }
                resp.context_menu(|ui| {
                    if let Some(picked) = gutter_menu(ui, list, b.line) {
                        action = Some((path.clone(), picked));
                    }
                });
            }
        }
        if let Some((path, action)) = action {
            env.marks.apply(&path, action);
        }
    }

    pub fn toolbar(&mut self, ctx: &egui::Context, theme: &Theme, top: f32, marks: &mut Marks, cmds: &mut Vec<Command>) {
        if self.live.is_empty() {
            return;
        }
        let stopped = self.paused();
        let mut focus = self.focus;
        egui::Area::new(Id::new("debug_toolbar")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, vec2(0.0, top)).show(ctx, |ui| {
            ui.set_opacity(ui::fade_in(ui, Id::new("debug_toolbar_fade")));
            egui::Frame::popup(&ctx.global_style()).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if self.live.len() > 1 {
                        egui::ComboBox::from_id_salt("debug_session").selected_text(&self.live[focus].config.name).width(130.0).show_ui(ui, |ui| {
                            for (index, live) in self.live.iter().enumerate() {
                                ui.selectable_value(&mut focus, index, format!("{} ({})", live.config.name, live.shown_state()));
                            }
                        });
                    }
                    let (icon, cmd, tip) = if stopped { (icons::PLAY, Command::StartDebugging, "Continue (F5)") } else { (icons::PAUSE, Command::PauseDebugging, "Pause (F6)") };
                    let buttons = [
                        (icon, cmd, tip, true),
                        (icons::ARROW_U_UP_RIGHT, Command::StepOver, "Step Over (F10)", stopped),
                        (icons::ARROW_LINE_DOWN, Command::StepInto, "Step Into (F11)", stopped),
                        (icons::ARROW_LINE_UP, Command::StepOut, "Step Out (Shift+F11)", stopped),
                        (icons::ARROW_CLOCKWISE, Command::RestartDebugging, "Restart (Ctrl+Shift+F5)", true),
                        (icons::STOP, Command::StopDebugging, "Stop (Shift+F5)", true),
                    ];
                    for (icon, cmd, tip, enabled) in buttons {
                        let resp = ui.add_enabled_ui(enabled, |ui| ui::icon_button(ui, theme, icon, 26.0, false, tip)).inner;
                        if resp.clicked() && enabled {
                            cmds.push(cmd);
                        }
                    }
                });
            });
        });
        if focus != self.focus {
            self.focus_on(focus, marks, cmds);
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, env: &mut Env) {
        if let Some((path, line, ask)) = env.marks.ask.take() {
            let found = env.marks.of(&path).iter().find(|b| b.line == line).cloned().unwrap_or_default();
            self.dialog = Some(Dialog { path, line, ask, texts: [found.condition, found.hit, found.log], focus: true });
        }
        if let Some(dialog) = &mut self.dialog {
            let mut done = None;
            let modal = ui::modal(ctx, "breakpoint_dialog", ui::Enter::Primary, |ui, keys| {
                ui::outline(ui);
                ui.set_width(380.0);
                ui.heading(format!("Breakpoint on line {}", dialog.line + 1));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (ask, label) in [(Ask::Condition, "Expression"), (Ask::Hit, "Hit Count"), (Ask::Log, "Log Message")] {
                        if ui.selectable_label(dialog.ask == ask, label).clicked() {
                            dialog.ask = ask;
                            dialog.focus = true;
                        }
                    }
                });
                ui.add_space(6.0);
                let (index, hint) = match dialog.ask {
                    Ask::Condition => (0, "Break when the expression is true, for example i > 10"),
                    Ask::Hit => (1, "Break when the hit count matches, for example >= 5"),
                    Ask::Log => (2, "Message to log, with {expression} in braces"),
                };
                let resp = ui.add(TextEdit::singleline(&mut dialog.texts[index]).hint_text(hint).desired_width(f32::INFINITY));
                if std::mem::take(&mut dialog.focus) {
                    resp.request_focus();
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if keys.primary(ui, "OK", true) {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
            if modal.should_close() {
                done = done.or(Some(false));
            }
            match done {
                Some(true) => {
                    let dialog = self.dialog.take().unwrap();
                    env.marks.set(&dialog.path, dialog.line, dialog.texts);
                }
                Some(false) => self.dialog = None,
                None => {}
            }
        }
        self.confirm_breakpoint_removal(ctx, env.marks);
        self.views.dialog(ctx, self.live.get_mut(self.focus));
    }
}

pub fn gutter_menu(ui: &mut Ui, list: &[Breakpoint], line: usize) -> Option<Gutter> {
    ui::menu_pad(ui);
    ui.set_min_width(220.0);
    let found = list.iter().find(|b| b.line == line);
    let items: Vec<(&str, Gutter)> = match found {
        Some(b) => vec![
            ("Edit Condition...", Gutter::Ask(line, if b.hit.is_empty() || !b.condition.is_empty() { Ask::Condition } else { Ask::Hit })),
            ("Edit Log Message...", Gutter::Ask(line, Ask::Log)),
            (if b.enabled { "Disable Breakpoint" } else { "Enable Breakpoint" }, Gutter::Enable(line, !b.enabled)),
            ("Remove Breakpoint", Gutter::Remove(line)),
        ],
        None => vec![
            ("Add Breakpoint", Gutter::Toggle(line)),
            ("Add Conditional Breakpoint...", Gutter::Ask(line, Ask::Condition)),
            ("Add Logpoint...", Gutter::Ask(line, Ask::Log)),
        ],
    };
    let mut picked = None;
    for (label, action) in items {
        if ui::menu_item(ui, label).clicked() {
            picked = Some(action);
            ui.close();
        }
    }
    picked
}

#[allow(clippy::too_many_arguments)]
pub fn paint_gutter(painter: &egui::Painter, theme: &Theme, list: &[Breakpoint], live: bool, stopped: bool, line: usize, center: Pos2, radius: f32, hovered: bool) {
    match list.iter().find(|b| b.line == line) {
        Some(b) => {
            let color = if b.enabled { theme.error } else { theme.text_dim };
            let hollow = live && b.enabled && !b.verified;
            let (fill, stroke) = if hollow { (Color32::TRANSPARENT, Stroke::new(1.5, color)) } else { (color, Stroke::NONE) };
            if !b.log.is_empty() {
                let r = radius * 1.15;
                let points = vec![center + vec2(0.0, -r), center + vec2(r, 0.0), center + vec2(0.0, r), center + vec2(-r, 0.0)];
                painter.add(egui::Shape::convex_polygon(points, fill, stroke));
            } else {
                painter.circle(center, radius, fill, stroke);
                if !b.condition.is_empty() || !b.hit.is_empty() {
                    let mark = if hollow { color } else { theme.bg };
                    for dy in [-radius * 0.3, radius * 0.3] {
                        painter.hline(center.x - radius * 0.5..=center.x + radius * 0.5, center.y + dy, Stroke::new(1.2, mark));
                    }
                }
            }
        }
        None if hovered => {
            painter.circle_filled(center, radius, ui::with_alpha(theme.error, 0.35));
        }
        None => {}
    }
    if stopped {
        let (w, h) = (radius * 1.1, radius * 1.1);
        let points = vec![center + vec2(-w, -h * 0.55), center + vec2(0.0, -h * 0.55), center + vec2(w, 0.0), center + vec2(0.0, h * 0.55), center + vec2(-w, h * 0.55)];
        painter.add(egui::Shape::convex_polygon(points, theme.warning, Stroke::new(1.0, theme.bg)));
    }
}

pub fn hover_ui(ctx: &egui::Context, theme: &Theme, marks: &mut Marks, owner: (usize, u64), word: Option<String>, now: f64) {
    if marks.stopped.is_none() {
        return;
    }
    match (&marks.hover, &word) {
        (Some(hover), Some(word)) if hover.word == *word && hover.owner == owner => {}
        (Some(hover), None) if hover.owner != owner => return,
        (None, None) => return,
        _ => {
            marks.hover = word.map(|word| Hover { owner, word, since: now, asked: false, value: None });
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
    }
    let Some(Hover { word, value: Some(value), .. }) = &marks.hover else { return };
    let Some(pos) = ctx.pointer_latest_pos() else { return };
    egui::Area::new(Id::new("debug_hover")).order(egui::Order::Tooltip).fixed_pos(pos + vec2(10.0, 16.0)).show(ctx, |ui| {
        egui::Frame::popup(&ctx.global_style()).inner_margin(8.0).show(ui, |ui| {
            ui.set_max_width(520.0);
            let text = format!("{word} = {value}");
            ui.add(egui::Label::new(RichText::new(text).font(FontId::monospace(12.5)).color(theme.text)).wrap());
        });
    });
}

impl Halo {
    pub(crate) fn debug_env<'a>(&'a mut self, ctx: &'a egui::Context) -> (&'a mut Debug, Env<'a>) {
        let file = self.editors.active_path();
        let workspace = &self.session.workspace;
        let owner = file.as_deref().and_then(|f| workspace.owner(f)).unwrap_or(0);
        let env = Env {
            folder: workspace.folders.get(owner).map(|f| f.path.as_path()),
            workspace,
            file,
            cursor_line: None,
            settings: &self.settings,
            marks: &mut self.editors.marks,
            bottom: &mut self.bottom,
            tasks: &mut self.tasks,
            panel: &mut self.session.panel,
            cmds: &mut self.cmds,
            ctx,
        };
        (&mut self.debug, env)
    }

    pub(crate) fn debug_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        let line = if matches!(cmd, Command::RunToCursor | Command::JumpToCursor) { self.editors.source_cursor().map(|(_, line)| line) } else { None };
        let (debug, mut env) = self.debug_env(ctx);
        env.cursor_line = line;
        debug.exec(cmd, &mut env)
    }

    pub(crate) fn debug_frame(&mut self, ctx: &egui::Context, top: f32) {
        let theme = self.theme.clone();
        let (debug, mut env) = self.debug_env(ctx);
        debug.poll(&mut env);
        debug.dialogs(ctx, &mut env);
        debug.toolbar(ctx, &theme, top, env.marks, env.cmds);
        self.debug_views();
        if let Some(items) = self.debug.choices.take() {
            self.search.pick("Select the process to attach to", items);
        }
    }

    pub(crate) fn debug_view(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        let ctx = ui.ctx().clone();
        let (debug, mut env) = self.debug_env(&ctx);
        debug.ui(ui, &theme, &mut env);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::{Duration, Instant};

    #[test]
    fn reads_launch_json_with_comments_and_trailing_commas() {
        let text = "\u{feff}{\n  // comment with \"quotes\"\n  \"version\": \"0.2.0\", /* block\n comment */\n  \"configurations\": [\n    {\n      \"type\": \"debugpy\",\n      \"request\": \"launch\",\n      \"name\": \"Run // not a comment\",\n      \"program\": \"${file}\",\n      \"args\": [\"a,\", \"b\",],\n      \"env\": {\"X\": \"1\",},\n      \"preLaunchTask\": \"build\",\n      \"justMyCode\": false,\n    },\n    {\"type\": \"go\", \"name\": \"Attach\", \"request\": \"attach\", \"debugServer\": 4711},\n  ],\n}\n";
        let configs = parse_launch(text).unwrap();
        assert_eq!(configs.len(), 2);
        let first = &configs[0];
        assert_eq!((first.name.as_str(), first.kind.as_str(), first.request.as_str()), ("Run // not a comment", "debugpy", "launch"));
        assert_eq!(first.pre_launch_task.as_deref(), Some("build"));
        assert_eq!(first.raw["args"], json!(["a,", "b"]));
        assert_eq!(first.raw["env"], json!({"X": "1"}));
        assert_eq!(first.raw["justMyCode"], json!(false));
        assert_eq!(configs[1].raw["debugServer"], json!(4711));
        assert!(parse_launch("{ not json").unwrap_err().starts_with("launch.json could not be read"));
        assert!(parse_launch("{}").unwrap().is_empty());
    }

    #[test]
    fn substitutes_launch_variables() {
        let folder = PathBuf::from("/work/app");
        let mut all = Workspace::single(folder.clone());
        all.add(PathBuf::from("/work/api"));
        let vars = Vars { folder: folder.clone(), file: Some(folder.join("src").join("main.test.py")), all };
        assert_eq!(PathBuf::from(expand("${workspaceFolder:api}/x", &vars)), PathBuf::from("/work/api/x"));
        let shown = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(expand("${workspaceFolder}/bin", &vars), format!("{}/bin", shown(folder.clone())));
        assert_eq!(expand("${file}", &vars), shown(folder.join("src").join("main.test.py")));
        assert_eq!(expand("${fileBasename}|${fileBasenameNoExtension}", &vars), "main.test.py|main.test");
        assert_eq!(expand("${fileDirname}", &vars), shown(folder.join("src")));
        assert_eq!(expand("${relativeFile}", &vars), shown(PathBuf::from("src").join("main.test.py")));
        let key = if cfg!(windows) { "USERNAME" } else { "HOME" };
        assert_eq!(expand(&format!("${{env:{key}}}"), &vars), std::env::var(key).unwrap_or_default());
        assert_eq!(expand("${unknown} ${env:HALO_NO_SUCH_VAR}x ${unclosed", &vars), "${unknown} x ${unclosed");
        let none = Vars { folder, file: None, all: Workspace::default() };
        assert_eq!(expand("${file}", &none), "${file}");
        let config = parse_launch(r#"{"configurations": [{"name": "a", "type": "x", "program": "${fileBasename}", "args": ["${fileDirname}", 3], "env": {"K": "${fileBasenameNoExtension}"}}]}"#).unwrap();
        let resolved = config[0].resolve(&vars);
        assert_eq!(resolved.raw["program"], json!("main.test.py"));
        assert_eq!(resolved.raw["args"], json!([shown(PathBuf::from("/work/app").join("src")), 3]));
        assert_eq!(resolved.raw["env"]["K"], json!("main.test"));
    }

    #[test]
    fn adds_configurations_to_new_and_existing_files() {
        let dir = std::env::temp_dir().join(format!("halo-debug-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = add_configuration(&dir, Some(&dir.join("a.py"))).unwrap();
        assert_eq!(path, launch_path(&dir));
        add_configuration(&dir, Some(&dir.join("main.go"))).unwrap();
        let configs = parse_launch(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(configs.iter().map(|c| c.kind.as_str()).collect::<Vec<_>>(), ["go", "debugpy"]);
        std::fs::write(&path, "{\n  // mine\n  \"configurations\": []\n}\n").unwrap();
        add_configuration(&dir, Some(&dir.join("x.rs"))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("// mine"));
        assert_eq!(parse_launch(&text).unwrap()[0].kind, "lldb");
        std::fs::write(&path, "{}").unwrap();
        assert!(add_configuration(&dir, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn breakpoints_follow_edits_and_carry_conditions() {
        let path = PathBuf::from("/w/a.rs");
        let mut marks = Marks::default();
        for line in [2, 5, 9] {
            marks.toggle(&path, line);
        }
        marks.toggle(&path, 5);
        assert_eq!(marks.of(&path).iter().map(|b| b.line).collect::<Vec<_>>(), [2, 9]);
        marks.shift(&path, 3, 2);
        assert_eq!(marks.of(&path).iter().map(|b| b.line).collect::<Vec<_>>(), [2, 11]);
        marks.shift(&path, 0, -1);
        assert_eq!(marks.of(&path).iter().map(|b| b.line).collect::<Vec<_>>(), [1, 10]);
        marks.shift(&path, 1, -20);
        assert_eq!(marks.of(&path).iter().map(|b| b.line).collect::<Vec<_>>(), [1]);
        marks.toggle(&path, 4);
        marks.set(&path, 4, ["x > 1".into(), String::new(), String::new()]);
        marks.set(&path, 7, [String::new(), ">= 3".into(), "x is {x}".into()]);
        marks.apply(&path, Gutter::Enable(1, false));
        let sent = source_breakpoints(&path, marks.of(&path));
        assert_eq!(sent["breakpoints"], json!([{"line": 5, "condition": "x > 1"}, {"line": 8, "hitCondition": ">= 3", "logMessage": "x is {x}"}]));
        assert_eq!(sent["source"]["name"], json!("a.rs"));
        marks.verify(&path, &json!({"breakpoints": [{"id": 3, "verified": true}, {"id": 4, "verified": false}]}));
        let list = marks.of(&path);
        assert_eq!(list.iter().map(|b| (b.line, b.verified, b.id)).collect::<Vec<_>>(), [(1, false, None), (4, true, Some(3)), (7, false, Some(4))]);
        marks.update(&json!({"id": 4, "verified": true}));
        assert!(marks.of(&path)[2].verified);
        marks.apply(&path, Gutter::Ask(4, Ask::Log));
        assert_eq!(marks.ask, Some((path.clone(), 4, Ask::Log)));
        let mut session = crate::settings::Session { folder: Some(PathBuf::from("/w")), ..Default::default() };
        restore(&session, &mut Marks::default());
        marks.folder = session.folder.clone();
        store(&mut session, &marks);
        let mut back = Marks::default();
        restore(&session, &mut back);
        assert_eq!(back.of(&path).len(), 3);
        assert_eq!(back.of(&path)[1].condition, "x > 1");
        let text = serde_json::to_string(&session).unwrap();
        let read: crate::settings::Session = serde_json::from_str(&text).unwrap();
        assert_eq!(read.breakpoints[&PathBuf::from("/w")][&path][2].log, "x is {x}");
        marks.apply(&path, Gutter::Remove(7));
        marks.clear();
        assert!(marks.files.is_empty() && marks.changed.iter().any(|c| same(c, &path)));
    }

    pub(super) fn adapter(file: PathBuf, x: &'static str) -> (u16, Receiver<Value>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut seq = 100;
            let mut frame_line = 3;
            let mut continued = false;
            let mut send = |writer: &mut std::net::TcpStream, mut message: Value| {
                seq += 1;
                message["seq"] = json!(seq);
                writer.write_all(crate::lsp::frame(&message).as_bytes()).unwrap();
            };
            while let Some(request) = crate::lsp::read_message(&mut reader) {
                let _ = tx.send(request.clone());
                let command = request["command"].as_str().unwrap_or_default().to_owned();
                let reply = |body: Value| json!({"type": "response", "request_seq": request["seq"], "command": command, "success": true, "body": body});
                let event = |name: &str, body: Value| json!({"type": "event", "event": name, "body": body});
                match command.as_str() {
                    "initialize" => {
                        let caps = json!({"supportsConfigurationDoneRequest": true, "supportsSetVariable": true, "supportsGotoTargetsRequest": true, "supportsConditionalBreakpoints": true,
                            "supportsFunctionBreakpoints": x != "4", "supportsDataBreakpoints": x != "4", "supportsHitConditionalBreakpoints": x != "2",
                            "supportsLogPoints": true, "exceptionBreakpointFilters": [{"filter": "raised", "label": "Raised Exceptions", "default": true}],
                            "supportsDisassembleRequest": true, "supportsReadMemoryRequest": true, "supportsSteppingGranularity": true});
                        send(&mut writer, reply(caps));
                        send(&mut writer, event("initialized", json!({})));
                    }
                    "dataBreakpointInfo" => {
                        let name = &request["arguments"]["name"];
                        let id = name.as_str().filter(|s| *s != "unavailable").map(|s| format!("{x}-{s}"));
                        send(&mut writer, reply(json!({"dataId": id, "accessTypes": ["read", "write"], "description": "No storage"})));
                    }
                    "setFunctionBreakpoints" | "setDataBreakpoints" => {
                        let found: Vec<_> = request["arguments"]["breakpoints"].as_array().unwrap().iter().enumerate()
                            .map(|(i, b)| json!({"id": i + if command == "setFunctionBreakpoints" { 50 } else { 100 }, "verified": b["name"] != "missing"})).collect();
                        send(&mut writer, reply(json!({"breakpoints": found})));
                    }
                    "setBreakpoints" => {
                        let count = request["arguments"]["breakpoints"].as_array().map_or(0, Vec::len);
                        let found: Vec<Value> = (0..count).map(|i| json!({"id": i + 1, "verified": true})).collect();
                        send(&mut writer, reply(json!({"breakpoints": found})));
                    }
                    "configurationDone" => {
                        send(&mut writer, reply(json!({})));
                        send(&mut writer, event("output", json!({"category": "stdout", "output": "hello from "})));
                        send(&mut writer, event("output", json!({"category": "stdout", "output": "the program\n"})));
                        send(&mut writer, event("stopped", json!({"reason": "breakpoint", "threadId": 1})));
                    }
                    "threads" => send(&mut writer, reply(json!({"threads": [{"id": 1, "name": "main"}]}))),
                    "stackTrace" => send(&mut writer, reply(json!({"stackFrames": [
                        {"id": 7, "name": "main", "line": frame_line, "column": 5, "source": {"path": file.to_string_lossy()}, "instructionPointerReference": "0x1000"},
                        {"id": 8, "name": "start", "line": 1, "column": 1},
                    ]}))),
                    "scopes" => send(&mut writer, reply(json!({"scopes": [{"name": "Locals", "variablesReference": 100}, {"name": "Arguments", "variablesReference": 150}, {"name": "Globals", "variablesReference": 200, "expensive": true}]}))),
                    "variables" if request["arguments"]["variablesReference"] == 150 => send(&mut writer, reply(json!({"variables": [{"name": "arg", "value": "5"}]}))),
                    "variables" => send(&mut writer, reply(json!({"variables": [{"name": "x", "value": x, "type": "int", "variablesReference": 0, "memoryReference": "0x2000"}, {"name": "items", "value": "[2]", "variablesReference": 101}]}))),
                    "disassemble" => send(&mut writer, reply(json!({"instructions": [
                        {"address": "0x0ffe", "instructionBytes": "90", "instruction": "nop", "symbol": "main", "location": {"path": file.to_string_lossy()}, "line": 3},
                        {"address": "0x1000", "instructionBytes": "c3", "instruction": "ret", "symbol": "main", "line": 3},
                    ]}))),
                    "readMemory" => send(&mut writer, reply(json!({"address": "0x2000", "data": "SGVsbG8=", "unreadableBytes": 2}))),
                    "next" => {
                        send(&mut writer, reply(json!({})));
                        send(&mut writer, event("stopped", json!({"reason": "step", "threadId": 1})));
                    }
                    "evaluate" => {
                        let expression = request["arguments"]["expression"].as_str().unwrap_or_default();
                        let result = if expression == "x * 2" { "84" } else { "42" };
                        send(&mut writer, reply(json!({"result": result, "variablesReference": 0})));
                    }
                    "gotoTargets" => {
                        let targets = if request["arguments"]["line"] == 2 { json!([{"id": 17}, {"id": 18}]) } else { json!([]) };
                        send(&mut writer, reply(json!({"targets": targets})));
                    }
                    "goto" => {
                        frame_line = 2;
                        send(&mut writer, event("stopped", json!({"reason": "goto"})));
                        send(&mut writer, reply(json!({})));
                    }
                    "continue" if !continued => {
                        continued = true;
                        frame_line = 4;
                        send(&mut writer, reply(json!({"allThreadsContinued": true})));
                        send(&mut writer, event("stopped", json!({"reason": "exception", "threadId": 1})));
                    }
                    "continue" => {
                        send(&mut writer, reply(json!({"allThreadsContinued": true})));
                        send(&mut writer, event("exited", json!({"exitCode": 0})));
                        send(&mut writer, event("terminated", json!({})));
                    }
                    "disconnect" => {
                        send(&mut writer, reply(json!({})));
                        break;
                    }
                    _ => send(&mut writer, reply(json!({}))),
                }
            }
        });
        (port, rx)
    }

    #[test]
    fn runs_a_full_session_against_a_fake_adapter() {
        let dir = std::env::temp_dir().join(format!("halo-debug-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        let file = dir.join("main.py");
        std::fs::write(&file, "a = 1\nb = 2\nx = 42\nprint(x)\n").unwrap();
        let (port, requests) = adapter(file.clone(), "42");
        let launch = format!("{{\n // fake\n \"configurations\": [{{\"type\": \"fake\", \"request\": \"launch\", \"name\": \"Fake\", \"program\": \"${{file}}\", \"debugServer\": {port}, \"custom\": [1,],}},]}}");
        std::fs::write(launch_path(&dir), launch).unwrap();

        let ctx = egui::Context::default();
        let settings = Settings::default();
        let (mut marks, mut bottom, mut panel, mut cmds, mut tasks) = (Marks::default(), BottomPanel::new(), false, vec![], Tasks::default());
        marks.toggle(&file, 2);
        marks.set(&file, 3, ["x > 1".into(), String::new(), String::new()]);
        marks.set(&file, 0, [String::new(), String::new(), "a is {a}".into()]);
        let mut debug = Debug::default();
        let workspace = Workspace::single(dir.clone());
        debug.watches.push("x * 2".into());
        macro_rules! env {
            () => {
                Env { folder: Some(&dir), workspace: &workspace, file: Some(file.clone()), cursor_line: Some(2), settings: &settings, marks: &mut marks, bottom: &mut bottom, tasks: &mut tasks, panel: &mut panel, cmds: &mut cmds, ctx: &ctx }
            };
        }
        macro_rules! pump {
            ($done:expr) => {{
                let deadline = Instant::now() + Duration::from_secs(10);
                while !($done)(&debug, &bottom) {
                    assert!(Instant::now() < deadline, "the session did not reach the expected state");
                    debug.poll(&mut env!());
                    std::thread::sleep(Duration::from_millis(5));
                }
            }};
        }
        assert!(debug.exec(&Command::StartDebugging, &mut env!()));
        assert!(debug.active() && bottom.console.active && marks.live);
        pump!(|d: &Debug, _: &BottomPanel| d.live.first().is_some_and(|l| l.children.contains_key(&100) && l.children.contains_key(&150)));

        let live = &debug.live[0];
        assert_eq!(live.state, State::Stopped);
        assert_eq!(live.threads, [(1, "main".to_owned())]);
        assert_eq!(live.frames.len(), 2);
        assert_eq!(live.scopes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Locals", "Arguments", "Globals"]);
        assert!(debug.expanded.contains("/Locals") && !debug.expanded.contains("/Globals"));
        assert_eq!(live.children[&100].iter().map(|v| (v.name.as_str(), v.value.as_str())).collect::<Vec<_>>(), [("x", "42"), ("items", "[2]")]);
        assert_eq!(marks.inline_label("arg + x + arg"), "arg = 5, x = 42");
        assert_eq!(marks.stopped, Some((file.clone(), 2)));
        assert!(cmds.contains(&Command::OpenPath(file.clone())) && cmds.contains(&Command::GoTo(3, 5)));
        assert_eq!(debug.filters, [("raised".to_owned(), "Raised Exceptions".to_owned(), true)]);
        assert!(marks.of(&file).iter().all(|b| b.verified));
        assert!(bottom.console.text().contains("hello from the program\n"));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            debug.ui(ui, &Theme::dark(), &mut env!());
            debug.toolbar(ui.ctx(), &Theme::dark(), 40.0, &mut marks, &mut vec![]);
            bottom.console.ui(ui, Rect::from_min_size(pos2(0.0, 500.0), vec2(320.0, 200.0)), &Theme::dark(), 13.0);
        });
        out.textures_delta.clear();

        pump!(|d: &Debug, _: &BottomPanel| d.live.first().is_some_and(|l| l.watch.contains_key("x * 2")));
        assert_eq!(debug.live[0].watch["x * 2"], Ok("84".to_owned()));
        bottom.console.submitted.push("x".into());
        pump!(|_: &Debug, b: &BottomPanel| b.console.text().ends_with("the program\n42\n"));

        debug.live[0].caps["supportsGotoTargetsRequest"] = json!(false);
        assert!(debug.exec(&Command::JumpToCursor, &mut env!()));
        assert!(!marks.cursor.allows(&Command::JumpToCursor));
        debug.live[0].caps["supportsGotoTargetsRequest"] = json!(true);
        let mut missing = env!();
        missing.cursor_line = Some(1);
        assert!(debug.exec(&Command::JumpToCursor, &mut missing));
        pump!(|d: &Debug, _: &BottomPanel| d.live[0].cursor_commands().allows(&Command::JumpToCursor));
        assert_eq!(marks.stopped, Some((file.clone(), 2)));
        assert!(debug.exec(&Command::JumpToCursor, &mut env!()));
        pump!(|d: &Debug, _: &BottomPanel| d.live.first().is_some_and(|l| l.reason == "goto" && l.children.contains_key(&100)));
        assert_eq!(marks.stopped, Some((file.clone(), 1)));

        assert!(debug.exec(&Command::RunToCursor, &mut env!()));
        assert!(!marks.of(&file).iter().any(|b| b.line == 1));
        pump!(|d: &Debug, _: &BottomPanel| d.live.first().is_some_and(|l| l.reason == "exception" && l.children.contains_key(&100)));
        assert_eq!(marks.stopped, Some((file.clone(), 3)));
        assert!(debug.live[0].temporary.is_none());

        let shown = |d: &Debug, title: &str, part: &str| d.views.out.iter().any(|o| o.0 == title && o.1.contains(part));
        assert_eq!(debug.live[0].children[&100][0].memory.as_deref(), Some("0x2000"));
        assert!(debug.exec(&Command::OpenDisassembly, &mut env!()));
        assert!(debug.views.out.iter().any(|o| o.0 == views::DISASSEMBLY && o.3));
        pump!(|d: &Debug, _: &BottomPanel| shown(d, views::DISASSEMBLY, "ret"));
        assert!(shown(&debug, views::DISASSEMBLY, "main:\n  0x0ffe  90  nop  ; main.py:3\n  0x1000  c3  ret\n"));
        assert!(debug.views.out.iter().any(|o| o.0 == views::DISASSEMBLY && o.2 == Some(2)));
        assert_eq!(marks.band, Some((views::DISASSEMBLY.to_owned(), 2)));
        let live = &mut debug.live[0];
        debug.views.open_memory(live, "0x2000".into(), "x".into());
        pump!(|d: &Debug, _: &BottomPanel| shown(d, views::MEMORY, "00002000"));
        assert!(shown(&debug, views::MEMORY, "x at 0x2000, 7 bytes\n\n00002000  48 65 6C 6C 6F ?? ?? "));
        assert!(shown(&debug, views::MEMORY, " Hello??\n"));
        debug.views.out.clear();
        debug.views.focused = true;
        assert!(debug.exec(&Command::StepOver, &mut env!()));
        pump!(|d: &Debug, _: &BottomPanel| shown(d, views::DISASSEMBLY, "ret") && shown(d, views::MEMORY, "Hello"));
        debug.views.focused = false;

        assert!(debug.exec(&Command::StartDebugging, &mut env!()));
        pump!(|d: &Debug, _: &BottomPanel| !d.active());
        assert!(marks.inline.is_empty());
        assert!(!marks.live && marks.stopped.is_none() && !bottom.console.active);
        assert!(bottom.console.text().contains("Process exited with code 0."));

        let sent: Vec<Value> = requests.try_iter().collect();
        let commands: Vec<&str> = sent.iter().filter_map(|r| r["command"].as_str()).collect();
        let order = ["initialize", "launch", "setBreakpoints", "setExceptionBreakpoints", "configurationDone", "threads", "stackTrace", "scopes", "continue"];
        let mut at = 0;
        for name in order {
            at += commands[at..].iter().position(|c| *c == name).unwrap_or_else(|| panic!("{name} was not sent in order: {commands:?}"));
        }
        let launch = sent.iter().find(|r| r["command"] == "launch").unwrap();
        assert_eq!(launch["arguments"]["program"], json!(file.to_string_lossy()));
        assert_eq!(launch["arguments"]["custom"], json!([1]));
        let breakpoints = sent.iter().find(|r| r["command"] == "setBreakpoints").unwrap();
        assert_eq!(breakpoints["arguments"]["breakpoints"], json!([{"line": 1, "logMessage": "a is {a}"}, {"line": 3}, {"line": 4, "condition": "x > 1"}]));
        let sets: Vec<_> = sent.iter().filter(|r| r["command"] == "setBreakpoints").map(|r| &r["arguments"]).collect();
        assert_eq!(sets.len(), 3);
        assert_eq!(sets[1]["breakpoints"][3], json!({"line": 2}));
        assert_eq!(sets[0], sets[2]);
        assert!(!sent.iter().any(|r| r["command"] == "variables" && r["arguments"]["variablesReference"] == 200));
        let targets: Vec<_> = sent.iter().filter(|r| r["command"] == "gotoTargets").map(|r| &r["arguments"]).collect();
        assert_eq!(targets.len(), 2);
        assert_eq!(*targets[1], json!({"source": {"path": file}, "line": 2}));
        let jumps: Vec<_> = sent.iter().filter(|r| r["command"] == "goto").collect();
        assert_eq!(jumps.len(), 1);
        assert_eq!(jumps[0]["arguments"], json!({"threadId": 1, "targetId": 17}));
        let exceptions = sent.iter().find(|r| r["command"] == "setExceptionBreakpoints").unwrap();
        assert_eq!(exceptions["arguments"]["filters"], json!(["raised"]));
        assert!(sent.iter().any(|r| r["command"] == "evaluate" && r["arguments"]["context"] == "watch" && r["arguments"]["frameId"] == 7));
        assert!(sent.iter().any(|r| r["command"] == "evaluate" && r["arguments"]["context"] == "repl" && r["arguments"]["expression"] == "x"));
        let count = |name: &str| sent.iter().filter(|r| r["command"] == name).count();
        assert_eq!((count("disassemble"), count("readMemory")), (2, 2));
        let disassemble = sent.iter().find(|r| r["command"] == "disassemble").unwrap();
        assert_eq!(disassemble["arguments"], views::disassemble_args("0x1000"));
        assert_eq!(sent.iter().find(|r| r["command"] == "readMemory").unwrap()["arguments"], views::read_args("0x2000"));
        assert_eq!(sent.iter().find(|r| r["command"] == "next").unwrap()["arguments"], json!({"threadId": 1, "granularity": "instruction"}));
        assert_eq!(sent.iter().find(|r| r["command"] == "continue").unwrap()["arguments"], json!({"threadId": 1}));
        assert_eq!(sent[0]["arguments"]["supportsMemoryReferences"], json!(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_compounds_and_resolves_their_names_across_folders() {
        let text = r#"{"configurations": [{"name": "Run", "type": "x"}, {"name": "Api", "type": "x"}],
            "compounds": [{"name": "All", "configurations": ["Run", {"name": "Api"}, "Gone"], "stopAll": true, "preLaunchTask": "build"}, {"name": "Plain", "configurations": []}]}"#;
        let compounds = parse_compounds(text);
        let all = Compound { name: "All".into(), configurations: vec!["Run".into(), "Api".into(), "Gone".into()], stop_all: true, pre_launch_task: Some("build".into()) };
        assert_eq!(compounds[0], all);
        assert!(!compounds[1].stop_all && compounds[1].pre_launch_task.is_none() && compounds[1].configurations.is_empty());
        assert!(parse_compounds("{ bad").is_empty());
        let other = r#"{"configurations": [{"name": "Run", "type": "y"}], "compounds": [{"name": "Two", "configurations": ["Run"]}]}"#;
        let (configs, compounds, error) = launch_configs(&[(PathBuf::from("/a"), "a".into(), text.into()), (PathBuf::from("/b"), "b".into(), other.into())]);
        assert!(error.is_none());
        assert_eq!(configs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Run (a)", "Api", "Run (b)"]);
        assert_eq!(compounds[0].configurations, ["Run (a)", "Api", "Gone"]);
        assert_eq!(compounds[2].configurations, ["Run (b)"]);
        let mut console = Console::default();
        console.tagged("A", "stdout", "one ");
        console.tagged("B", "stdout", "two\n");
        console.tagged("A", "stdout", "done\n");
        console.push("stdout", "plain\n");
        assert_eq!(console.text(), "[A] one done\n[B] two\nplain\n");
    }

    #[test]
    fn runs_several_sessions_and_compounds_against_fake_adapters() {
        let dir = std::env::temp_dir().join(format!("halo-debug-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        let file = dir.join("main.py");
        std::fs::write(&file, "a = 1\nb = 2\nx = 42\nprint(x)\n").unwrap();
        let adapters: Vec<(u16, Receiver<Value>)> = ["1", "2", "3", "4"].into_iter().map(|x| adapter(file.clone(), x)).collect();
        let entry = |name: &str, port: u16| format!("{{\"type\": \"fake\", \"request\": \"launch\", \"name\": \"{name}\", \"debugServer\": {port}}}");
        let entries: Vec<String> = ["A", "B", "C", "D"].iter().zip(&adapters).map(|(name, (port, _))| entry(name, *port)).collect();
        let compounds = r#"[{"name": "Both", "configurations": ["A", "B"], "stopAll": true}, {"name": "Bad", "configurations": ["A", "Nope"]}]"#;
        std::fs::write(launch_path(&dir), format!("{{\"configurations\": [{}], \"compounds\": {compounds}}}", entries.join(", "))).unwrap();

        let ctx = egui::Context::default();
        let settings = Settings::default();
        let (mut marks, mut bottom, mut panel, mut cmds, mut tasks) = (Marks::default(), BottomPanel::new(), false, vec![], Tasks::default());
        marks.toggle(&file, 2);
        let mut debug = Debug::default();
        let workspace = Workspace::single(dir.clone());
        macro_rules! env {
            () => {
                Env { folder: Some(&dir), workspace: &workspace, file: Some(file.clone()), cursor_line: Some(2), settings: &settings, marks: &mut marks, bottom: &mut bottom, tasks: &mut tasks, panel: &mut panel, cmds: &mut cmds, ctx: &ctx }
            };
        }
        macro_rules! pump {
            ($done:expr) => {{
                let deadline = Instant::now() + Duration::from_secs(10);
                while !($done)(&debug) {
                    assert!(Instant::now() < deadline, "the sessions did not reach the expected state");
                    debug.poll(&mut env!());
                    std::thread::sleep(Duration::from_millis(5));
                }
            }};
        }
        let paused = |d: &Debug, count: usize| d.live.len() == count && d.live.iter().all(|l| l.state == State::Stopped && l.place.is_some());

        debug.selected = "Bad".into();
        debug.exec(&Command::StartDebugging, &mut env!());
        assert_eq!(debug.compounds.len(), 2);
        assert!(!debug.active());

        debug.selected = "Both".into();
        debug.exec(&Command::StartDebugging, &mut env!());
        assert_eq!(debug.live.len(), 2);
        pump!(|d: &Debug| paused(d, 2));
        assert!(debug.live.iter().all(|l| l.group.as_deref() == Some("Both")));
        assert_eq!(marks.stopped_lines(&file), [2, 2]);
        assert_eq!(marks.stopped, Some((file.clone(), 2)));
        pump!(|_: &Debug| bottom.console.text().contains("[A] hello from the program\n") && bottom.console.text().contains("[B] hello from the program\n"));
        marks.toggle(&file, 0);
        debug.poll(&mut env!());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            debug.ui(ui, &Theme::dark(), &mut env!());
            debug.toolbar(ui.ctx(), &Theme::dark(), 40.0, &mut marks, &mut vec![]);
        });
        out.textures_delta.clear();

        debug.focus_on(0, &mut marks, &mut cmds);
        pump!(|d: &Debug| d.live[0].children.contains_key(&100));
        debug.focus_on(1, &mut marks, &mut cmds);
        pump!(|d: &Debug| d.live[1].children.contains_key(&100));
        assert_eq!(marks.inline_label("x"), "x = 2");
        debug.reply(0, Want::Variables(100), true, String::new(), json!({"variables": [{"name": "x", "value": "10"}]}), &mut env!());
        assert_eq!(marks.inline_label("x"), "x = 2");
        debug.event(0, "continued", &Value::Null, &mut env!());
        assert_eq!(marks.inline_label("x"), "x = 2");
        assert!(marks.cursor.allows(&Command::JumpToCursor));
        debug.focus_on(0, &mut marks, &mut cmds);
        assert!(marks.inline.is_empty() && !marks.cursor.allows(&Command::RunToCursor));
        debug.exec(&Command::StopDebugging, &mut env!());
        pump!(|d: &Debug| !d.active());
        assert!(!marks.live && marks.paused.is_empty() && marks.stopped.is_none());
        assert!(bottom.console.text().contains("[A] Debugging ended.\n"));

        debug.selected = "C".into();
        debug.exec(&Command::StartDebugging, &mut env!());
        pump!(|d: &Debug| paused(d, 1));
        debug.selected = "D".into();
        debug.exec(&Command::RunWithoutDebugging, &mut env!());
        pump!(|d: &Debug| paused(d, 2));
        assert_eq!((debug.focus, debug.live[1].config.name.as_str()), (1, "D"));
        pump!(|d: &Debug| d.live[1].children.contains_key(&100));
        debug.focus_on(0, &mut marks, &mut cmds);
        debug.exec(&Command::JumpToCursor, &mut env!());
        debug.focus_on(1, &mut marks, &mut cmds);
        assert!(marks.cursor.allows(&Command::JumpToCursor));
        pump!(|d: &Debug| d.live[0].reason == "goto" && d.live[0].children.contains_key(&100));
        assert_eq!(marks.inline_label("x"), "x = 3");
        debug.exec(&Command::RunToCursor, &mut env!());
        debug.focus_on(1, &mut marks, &mut cmds);
        marks.toggle(&file, 3);
        pump!(|d: &Debug| d.live[0].reason == "exception" && d.live[0].children.contains_key(&100));
        assert_eq!(marks.stopped_lines(&file), [3, 2]);
        debug.focus_on(1, &mut marks, &mut cmds);
        assert_eq!(marks.inline_label("x"), "x = 4");
        debug.exec(&Command::StopDebugging, &mut env!());
        pump!(|d: &Debug| d.live.len() == 1);
        assert_eq!((debug.focus, debug.live[0].config.name.as_str()), (0, "C"));
        assert!(debug.active() && bottom.console.active && marks.live);
        assert_eq!(marks.inline_label("x"), "x = 3");
        assert_eq!(marks.stopped, Some((file.clone(), 3)));
        assert!(marks.cursor.allows(&Command::RunToCursor));
        debug.exec(&Command::StartDebugging, &mut env!());
        pump!(|d: &Debug| !d.active());

        let sent: Vec<Vec<Value>> = adapters.iter().map(|(_, rx)| rx.try_iter().collect()).collect();
        let count = |i: usize, name: &str| sent[i].iter().filter(|r| r["command"] == name).count();
        assert_eq!((count(0, "setBreakpoints"), count(1, "setBreakpoints")), (2, 2));
        assert_eq!((count(0, "disconnect"), count(1, "disconnect")), (1, 1));
        assert_eq!((count(2, "gotoTargets"), count(2, "goto"), count(2, "continue")), (1, 1, 2));
        assert_eq!((count(3, "gotoTargets"), count(3, "goto"), count(3, "continue"), count(3, "disconnect")), (0, 0, 0, 1));
        let points = |i: usize| sent[i].iter().filter(|r| r["command"] == "setBreakpoints").map(|r| &r["arguments"]["breakpoints"]).collect::<Vec<_>>();
        let (c, d) = (points(2), points(3));
        assert_eq!(c.len(), 4);
        assert_eq!(d.len(), 2);
        assert_eq!(c[2], &json!([{"line": 1}, {"line": 3}, {"line": 4}, {"line": 2}]));
        assert_eq!(d[1], &json!([{"line": 1}, {"line": 3}, {"line": 4}]));
        assert_eq!(c[3], d[1]);
        assert!(sent[3].iter().any(|r| r["command"] == "launch" && r["arguments"]["noDebug"] == true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picks_a_process_before_attaching_to_a_fake_adapter() {
        let dir = std::env::temp_dir().join(format!("halo-debug-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".vscode")).unwrap();
        let file = dir.join("main.py");
        std::fs::write(&file, "x = 42\n").unwrap();
        let (port, requests) = adapter(file.clone(), "42");
        let entry = format!("{{\"type\": \"fake\", \"request\": \"attach\", \"name\": \"Attach\", \"debugServer\": {port}, \"processId\": \"${{command:pickProcess}}\", \"note\": \"pid ${{command:pickMyProcess}} in ${{workspaceFolder}}\"}}");
        std::fs::write(launch_path(&dir), format!("{{\"configurations\": [{entry}]}}")).unwrap();

        let ctx = egui::Context::default();
        let settings = Settings::default();
        let (mut marks, mut bottom, mut panel, mut cmds, mut tasks) = (Marks::default(), BottomPanel::new(), false, vec![], Tasks::default());
        let mut debug = Debug::default();
        let workspace = Workspace::single(dir.clone());
        macro_rules! env {
            () => {
                Env { folder: Some(&dir), workspace: &workspace, file: Some(file.clone()), cursor_line: None, settings: &settings, marks: &mut marks, bottom: &mut bottom, tasks: &mut tasks, panel: &mut panel, cmds: &mut cmds, ctx: &ctx }
            };
        }
        macro_rules! pump {
            ($done:expr) => {{
                let deadline = Instant::now() + Duration::from_secs(30);
                while !($done)(&debug) {
                    assert!(Instant::now() < deadline, "the session did not reach the expected state");
                    debug.poll(&mut env!());
                    std::thread::sleep(Duration::from_millis(5));
                }
            }};
        }
        let me = std::process::id();
        debug.exec(&Command::StartDebugging, &mut env!());
        assert!(!debug.active() && debug.picking.is_some());
        debug.exec(&Command::StopDebugging, &mut env!());
        debug.exec(&Command::AttachProcess(me), &mut env!());
        assert!(!debug.active() && debug.picking.is_none());

        debug.exec(&Command::StartDebugging, &mut env!());
        pump!(|d: &Debug| d.choices.is_some());
        let choices = debug.choices.take().unwrap();
        assert!(choices.iter().any(|c| c.2 == Command::AttachProcess(me)));
        debug.exec(&Command::AttachProcess(me), &mut env!());
        assert!(debug.active() && debug.picking.is_none());
        pump!(|d: &Debug| d.live.first().is_some_and(|l| l.state == State::Stopped));
        debug.exec(&Command::StopDebugging, &mut env!());
        pump!(|d: &Debug| !d.active());

        let sent: Vec<Value> = requests.try_iter().collect();
        let attach = sent.iter().find(|r| r["command"] == "attach").unwrap();
        assert_eq!(attach["arguments"]["processId"], json!(me));
        assert_eq!(attach["arguments"]["note"], json!(format!("pid {me} in {}", dir.to_string_lossy())));
        assert!(!sent.iter().any(|r| r["command"] == "launch"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
