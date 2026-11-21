use crate::app::{Halo, note};
use crate::command::Command;
use crate::dap::{self, Incoming};
use crate::settings::Settings;
use crate::terminal::BottomPanel;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Key, Pos2, Rect, Response, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui,
    pos2, vec2,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    changed: Vec<PathBuf>,
    folder: Option<PathBuf>,
    pub live: bool,
    pub stopped: Option<(PathBuf, usize)>,
    ask: Option<(PathBuf, usize, Ask)>,
    hover: Option<Hover>,
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
        self.stopped = None;
        self.hover = None;
        for b in self.files.iter_mut().flat_map(|f| f.1.iter_mut()) {
            (b.verified, b.id) = (false, None);
        }
    }
}

pub fn store(session: &mut crate::settings::Session, marks: &Marks) {
    let Some(folder) = &marks.folder else { return };
    let files: BTreeMap<PathBuf, Vec<Breakpoint>> = marks.files.iter().filter(|f| !f.1.is_empty()).cloned().collect();
    if files.is_empty() {
        session.breakpoints.remove(folder);
    } else {
        session.breakpoints.insert(folder.clone(), files);
    }
}

pub fn restore(session: &crate::settings::Session, marks: &mut Marks) {
    marks.folder = session.folder.clone();
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
}

fn jsonc(text: &str) -> String {
    fn strings(text: &str, mut other: impl FnMut(&[char], usize, &mut String) -> usize) -> String {
        let chars: Vec<char> = text.chars().collect();
        let (mut out, mut i) = (String::with_capacity(text.len()), 0);
        while i < chars.len() {
            if chars[i] == '"' {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    match chars[i - 1] {
                        '\\' if i < chars.len() => {
                            out.push(chars[i]);
                            i += 1;
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            } else {
                i = other(&chars, i, &mut out);
            }
        }
        out
    }
    let plain = strings(text.trim_start_matches('\u{feff}'), |chars, i, out| match (chars[i], chars.get(i + 1)) {
        ('/', Some('/')) => (i..chars.len()).find(|j| chars[*j] == '\n').unwrap_or(chars.len()),
        ('/', Some('*')) => (i + 2..chars.len()).find(|j| chars[*j] == '*' && chars.get(j + 1) == Some(&'/')).map_or(chars.len(), |j| j + 2),
        (c, _) => {
            out.push(c);
            i + 1
        }
    });
    strings(&plain, |chars, i, out| {
        let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
        if chars[i] != ',' || !matches!(next, Some('}' | ']')) {
            out.push(chars[i]);
        }
        i + 1
    })
}

pub fn parse_launch(text: &str) -> Result<Vec<Config>, String> {
    let value: Value = serde_json::from_str(&jsonc(text)).map_err(|e| format!("launch.json could not be read: {e}"))?;
    let configs = value["configurations"].as_array().into_iter().flatten().filter_map(|item| {
        let raw = item.as_object()?.clone();
        let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Config {
            name: text("name").unwrap_or_else(|| "Unnamed".into()),
            kind: text("type").unwrap_or_default(),
            request: text("request").unwrap_or_else(|| "launch".into()),
            pre_launch_task: text("preLaunchTask"),
            raw,
        })
    });
    Ok(configs.collect())
}

struct Vars {
    folder: PathBuf,
    file: Option<PathBuf>,
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
        match self.lines.last_mut().filter(|l| l.0 == category && !l.1.ends_with('\n')) {
            Some(last) => last.1.push_str(text),
            None => self.lines.push((category.to_owned(), text.to_owned())),
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
        if ui.memory(|m| m.has_focus(id)) && !self.history.is_empty() {
            let (up, down) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, Key::ArrowDown)));
            let last = self.history.len() - 1;
            if up {
                self.at = Some(self.at.map_or(last, |at| at.saturating_sub(1)));
            }
            if down {
                self.at = self.at.and_then(|at| (at < last).then_some(at + 1));
            }
            if up || down {
                self.input = self.at.map(|at| self.history[at].clone()).unwrap_or_default();
            }
        }
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
}

#[derive(Clone, Debug)]
struct Var {
    name: String,
    value: String,
    kind: String,
    reference: i64,
    expensive: bool,
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
    };
    items.iter().map(var).collect()
}

enum Want {
    Initialize,
    Launch,
    Breakpoints(PathBuf),
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
    Disconnect,
    Restart,
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
    configured: bool,
    stopping: u8,
    restart: bool,
}

impl Live {
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
        self.frame = index;
        (self.scopes, self.children, self.asked) = (vec![], HashMap::new(), HashSet::new());
        let Some(frame) = self.frames.get(index).cloned() else { return };
        if let Some(path) = frame.path.filter(|p| p.is_file()) {
            marks.stopped = Some((path.clone(), frame.line.saturating_sub(1)));
            cmds.extend([Command::OpenPath(path), Command::GoTo(frame.line.max(1), frame.column.max(1))]);
        }
        self.ask("scopes", json!({"frameId": frame.id}), Want::Scopes(frame.id));
        self.watch_all(watches);
    }

    fn running(&mut self, marks: &mut Marks) {
        self.state = State::Running;
        (self.frames, self.scopes, self.children, self.asked) = (vec![], vec![], HashMap::new(), HashSet::new());
        self.watch.clear();
        marks.stopped = None;
        marks.hover = None;
    }

    fn send_breakpoints(&mut self, path: &Path, list: &[Breakpoint]) {
        self.ask("setBreakpoints", source_breakpoints(path, list), Want::Breakpoints(path.to_path_buf()));
    }

    fn send_filters(&mut self, filters: &[(String, String, bool)]) {
        let on: Vec<&str> = filters.iter().filter(|f| f.2).map(|f| f.0.as_str()).collect();
        self.ask("setExceptionBreakpoints", json!({"filters": on}), Want::Plain("setExceptionBreakpoints".into()));
    }

    fn configure(&mut self, marks: &mut Marks, filters: &[(String, String, bool)]) {
        for (path, list) in marks.files.clone() {
            if list.iter().any(|b| b.enabled) {
                self.send_breakpoints(&path, &list);
            }
        }
        marks.changed.clear();
        if !filters.is_empty() {
            self.send_filters(filters);
        }
        if self.can("supportsConfigurationDoneRequest") {
            self.ask("configurationDone", json!({}), Want::Plain("configurationDone".into()));
        }
        self.configured = true;
    }

    fn fetch(&mut self, expanded: &HashSet<String>) {
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
        for reference in wanted {
            if self.asked.insert(reference) {
                self.ask("variables", json!({"variablesReference": reference}), Want::Variables(reference));
            }
        }
    }
}

pub struct Env<'a> {
    pub folder: Option<&'a Path>,
    pub file: Option<PathBuf>,
    pub settings: &'a Settings,
    pub marks: &'a mut Marks,
    pub bottom: &'a mut BottomPanel,
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
    Value(i64, String, String),
}

pub struct Debug {
    configs: Vec<Config>,
    error: Option<String>,
    present: bool,
    stamp: (Option<PathBuf>, Option<SystemTime>, f64),
    selected: String,
    live: Option<Live>,
    watches: Vec<String>,
    filters: Vec<(String, String, bool)>,
    open: [bool; 4],
    expanded: HashSet<String>,
    editing: Option<Editing>,
    dialog: Option<Dialog>,
    confirm_clear: bool,
}

impl Default for Debug {
    fn default() -> Self {
        Self {
            configs: vec![],
            error: None,
            present: false,
            stamp: (None, None, f64::NEG_INFINITY),
            selected: String::new(),
            live: None,
            watches: vec![],
            filters: vec![],
            open: [true; 4],
            expanded: HashSet::new(),
            editing: None,
            dialog: None,
            confirm_clear: false,
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
        self.live.is_some()
    }

    fn reload(&mut self, folder: Option<&Path>, now: f64, force: bool) {
        if !force && self.stamp.0.as_deref() == folder && now - self.stamp.2 < 1.0 {
            return;
        }
        let path = folder.map(launch_path);
        let modified = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok());
        let unchanged = self.stamp.0.as_deref() == folder && self.stamp.1 == modified && self.stamp.2 > f64::NEG_INFINITY;
        self.stamp = (folder.map(Path::to_path_buf), modified, now);
        if unchanged {
            return;
        }
        let text = path.and_then(|p| std::fs::read_to_string(p).ok());
        self.present = text.is_some();
        (self.configs, self.error) = match text.as_deref().map(parse_launch) {
            Some(Ok(configs)) => (configs, None),
            Some(Err(error)) => (vec![], Some(error)),
            None => (vec![], None),
        };
        if !self.configs.iter().any(|c| c.name == self.selected) {
            self.selected = self.configs.first().map(|c| c.name.clone()).unwrap_or_default();
        }
    }

    fn start(&mut self, no_debug: bool, env: &mut Env) {
        let now = env.ctx.input(|i| i.time);
        self.reload(env.folder, now, true);
        let Some(folder) = env.folder.filter(|_| self.present) else { return env.cmds.push(Command::RunFile) };
        if let Some(error) = &self.error {
            return ui::notify(error.clone());
        }
        let Some(config) = self.configs.iter().find(|c| c.name == self.selected).or(self.configs.first()).cloned() else {
            return ui::notify("launch.json has no configurations. Use Debug: Add Configuration... to add one.");
        };
        let vars = Vars { folder: folder.to_path_buf(), file: env.file.clone() };
        self.begin(config.resolve(&vars), no_debug, env);
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
                match dap::Client::spawn(&command, env.folder, env.ctx) {
                    Ok(client) => client,
                    Err(error) => return ui::notify(error),
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
            configured: false,
            stopping: 0,
            restart: false,
        };
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
        });
        live.ask("initialize", args, Want::Initialize);
        env.bottom.console.begin(&live.config.name);
        env.bottom.show_debug();
        *env.panel = true;
        env.marks.reset(true);
        self.live = Some(live);
    }

    fn end(&mut self, env: &mut Env) {
        let Some(mut live) = self.live.take() else { return };
        if live.stopping < 2 {
            live.client.request("disconnect", json!({"restart": false, "terminateDebuggee": live.config.request == "launch"}));
        }
        env.marks.reset(false);
        env.bottom.console.active = false;
        env.bottom.console.push("console", "Debugging ended.\n");
        if live.restart {
            let (config, no_debug) = (live.config.clone(), live.no_debug);
            drop(live);
            self.begin(config, no_debug, env);
        }
    }

    fn stop(&mut self, env: &mut Env) {
        let Some(live) = self.live.as_mut() else { return };
        live.stopping += 1;
        match live.stopping {
            1 if live.can("supportsTerminateRequest") && live.config.request == "launch" => live.ask("terminate", json!({}), Want::Plain("terminate".into())),
            1 | 2 => {
                let args = json!({"restart": false, "terminateDebuggee": live.config.request == "launch"});
                live.stopping = 2;
                live.ask("disconnect", args, Want::Disconnect);
            }
            _ => self.end(env),
        }
    }

    fn step(&mut self, command: &str) {
        let Some(live) = self.live.as_mut().filter(|l| l.state == State::Stopped) else { return };
        let thread = live.thread.or(live.threads.first().map(|t| t.0)).unwrap_or(0);
        live.ask(command, json!({"threadId": thread}), Want::Run);
    }

    pub fn exec(&mut self, cmd: &Command, env: &mut Env) -> bool {
        use Command::*;
        match cmd {
            StartDebugging => match self.live.as_ref().map(|l| l.state) {
                Some(State::Stopped) => self.step("continue"),
                Some(_) => {}
                None => self.start(false, env),
            },
            RunWithoutDebugging if self.live.is_none() => self.start(true, env),
            RunWithoutDebugging => {}
            StopDebugging => self.stop(env),
            RestartDebugging => match self.live.as_mut() {
                None => self.start(false, env),
                Some(live) if live.can("supportsRestartRequest") => {
                    let args = json!({"arguments": Value::Object(live.config.raw.clone())});
                    live.ask("restart", args, Want::Restart);
                }
                Some(live) => {
                    live.restart = true;
                    self.stop(env);
                }
            },
            PauseDebugging => {
                if let Some(live) = self.live.as_mut().filter(|l| l.state == State::Running) {
                    let thread = live.thread.or(live.threads.first().map(|t| t.0)).unwrap_or(0);
                    live.ask("pause", json!({"threadId": thread}), Want::Plain("pause".into()));
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
            RemoveAllBreakpoints => self.confirm_clear = !env.marks.files.is_empty(),
            AddWatch => {
                self.open[1] = true;
                self.editing = Some(Editing::Watch(None, String::new()));
            }
            _ => return false,
        }
        true
    }

    pub fn poll(&mut self, env: &mut Env) {
        let now = env.ctx.input(|i| i.time);
        let Some(live) = self.live.as_mut() else {
            env.bottom.console.submitted.clear();
            return;
        };
        let incoming = live.client.poll();
        for message in incoming {
            self.handle(message, env);
        }
        let Some(live) = self.live.as_mut() else { return };
        for expression in std::mem::take(&mut env.bottom.console.submitted) {
            live.evaluate(&expression, "repl", Want::Repl);
        }
        if live.configured {
            for path in std::mem::take(&mut env.marks.changed) {
                let list = env.marks.of(&path).to_vec();
                live.send_breakpoints(&path, &list);
            }
        }
        if let Some(hover) = env.marks.hover.as_mut().filter(|h| !h.asked && now - h.since >= 0.35 && live.state == State::Stopped) {
            hover.asked = true;
            let word = hover.word.clone();
            live.evaluate(&word, "hover", Want::Hover(word.clone()));
        }
        live.fetch(&self.expanded);
    }

    fn handle(&mut self, message: Incoming, env: &mut Env) {
        let Some(live) = self.live.as_mut() else { return };
        match message {
            Incoming::Response { request, command, success, message, body } => {
                let want = live.wants.remove(&request).unwrap_or(Want::Plain(command.clone()));
                self.reply(want, success, message, body, env);
            }
            Incoming::Event(name, body) => self.event(&name, &body, env),
            Incoming::Request { seq, command, args } => {
                let result = match command.as_str() {
                    "runInTerminal" => run_in_terminal(&args, env),
                    _ => Err(format!("Halo does not support the {command} request.")),
                };
                if let Err(error) = &result {
                    env.bottom.console.push("stderr", &format!("{error}\n"));
                }
                live.client.respond(seq, &command, result);
            }
            Incoming::Stderr(line) => env.bottom.console.push("console", &format!("{line}\n")),
            Incoming::Closed(error) => {
                if let Some(error) = error {
                    ui::notify(error);
                } else if live.state == State::Starting && live.stopping == 0 {
                    ui::notify(format!("The debug adapter for \"{}\" exited before the session started. See the Debug Console.", live.config.name));
                }
                live.stopping = 2;
                self.end(env);
            }
        }
    }

    fn reply(&mut self, want: Want, success: bool, message: String, body: Value, env: &mut Env) {
        let Some(live) = self.live.as_mut() else { return };
        match want {
            Want::Initialize if !success => {
                ui::notify(format!("The debug adapter could not be initialized: {message}"));
                self.end(env);
            }
            Want::Initialize => {
                live.caps = body;
                let old = std::mem::take(&mut self.filters);
                for filter in live.caps["exceptionBreakpointFilters"].as_array().into_iter().flatten() {
                    let id = filter["filter"].as_str().unwrap_or_default().to_owned();
                    let on = old.iter().find(|f| f.0 == id).map_or(filter["default"] == true, |f| f.2);
                    self.filters.push((id, filter["label"].as_str().unwrap_or_default().to_owned(), on));
                }
                let mut args = Value::Object(live.config.raw.clone());
                if live.no_debug {
                    args["noDebug"] = json!(true);
                }
                let request = if live.config.request == "attach" { "attach" } else { "launch" };
                live.ask(request, args, Want::Launch);
            }
            Want::Launch if !success => {
                let text = format!("{} \"{}\" failed: {message}", live.config.request, live.config.name);
                env.bottom.console.push("stderr", &format!("{text}\n"));
                ui::notify(text);
                self.end(env);
            }
            Want::Launch => {
                if live.state == State::Starting {
                    live.state = State::Running;
                }
            }
            Want::Breakpoints(path) if success => env.marks.verify(&path, &body),
            Want::Threads if success => {
                live.threads = body["threads"].as_array().into_iter().flatten().map(|t| (t["id"].as_i64().unwrap_or(0), t["name"].as_str().unwrap_or_default().to_owned())).collect();
            }
            Want::Stack if success => {
                let frame = |f: &Value| Frame {
                    id: f["id"].as_i64().unwrap_or(0),
                    name: f["name"].as_str().unwrap_or_default().to_owned(),
                    path: f["source"]["path"].as_str().map(PathBuf::from),
                    line: f["line"].as_u64().unwrap_or(0) as usize,
                    column: f["column"].as_u64().unwrap_or(0) as usize,
                };
                live.frames = body["stackFrames"].as_array().into_iter().flatten().map(frame).collect();
                if live.state == State::Stopped {
                    let index = live.frames.iter().position(|f| f.path.as_deref().is_some_and(Path::is_file)).unwrap_or(0);
                    live.select(index, &self.watches, env.marks, env.cmds);
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
            }
            Want::Watch(expression) => {
                live.watch.insert(expression, if success { Ok(body["result"].as_str().unwrap_or_default().to_owned()) } else { Err(message) });
            }
            Want::Repl => match success {
                true => env.bottom.console.push("repl", &format!("{}\n", body["result"].as_str().unwrap_or_default())),
                false => env.bottom.console.push("stderr", &format!("{message}\n")),
            },
            Want::Hover(word) if success => {
                if let Some(hover) = env.marks.hover.as_mut().filter(|h| h.word == word) {
                    hover.value = body["result"].as_str().map(str::to_owned);
                }
            }
            Want::SetVariable(reference, name) if success => {
                if let Some(var) = live.children.get_mut(&reference).into_iter().flatten().find(|v| v.name == name) {
                    var.value = body["value"].as_str().unwrap_or_default().to_owned();
                }
                live.watch_all(&self.watches);
            }
            Want::SetVariable(..) => ui::notify(format!("The value could not be set: {message}")),
            Want::Run if success => live.running(env.marks),
            Want::Disconnect => self.end(env),
            Want::Restart if !success => {
                live.restart = true;
                self.stop(env);
            }
            Want::Restart => live.running(env.marks),
            Want::Run => env.bottom.console.push("stderr", &format!("{message}\n")),
            Want::Plain(command) if !success => env.bottom.console.push("stderr", &format!("{command}: {message}\n")),
            _ => {}
        }
    }

    fn event(&mut self, name: &str, body: &Value, env: &mut Env) {
        let Some(live) = self.live.as_mut() else { return };
        match name {
            "initialized" => live.configure(env.marks, &self.filters),
            "stopped" => {
                live.running(env.marks);
                live.state = State::Stopped;
                live.reason = body["description"].as_str().or(body["reason"].as_str()).unwrap_or("paused").to_owned();
                live.thread = body["threadId"].as_i64().or(live.thread);
                live.ask("threads", json!({}), Want::Threads);
                let thread = live.thread.unwrap_or(0);
                live.ask("stackTrace", json!({"threadId": thread, "startFrame": 0, "levels": 50}), Want::Stack);
            }
            "continued" => live.running(env.marks),
            "exited" => env.bottom.console.push("console", &format!("Process exited with code {}.\n", body["exitCode"])),
            "terminated" => self.end(env),
            "output" if body["category"] != "telemetry" => {
                let category = body["category"].as_str().unwrap_or("console");
                env.bottom.console.push(category, body["output"].as_str().unwrap_or_default());
            }
            "breakpoint" => env.marks.update(&body["breakpoint"]),
            "thread" => live.ask("threads", json!({}), Want::Threads),
            _ => {}
        }
    }

    pub fn status(&self) -> Option<(String, String)> {
        let live = self.live.as_ref()?;
        let state = match live.state {
            State::Starting => "starting",
            State::Running => "running",
            State::Stopped => "paused",
        };
        Some((format!("{} {} ({state})", icons::BUG, live.config.name), format!("Debugging {} ({})", live.config.name, live.config.kind)))
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
}

struct Tree<'a> {
    live: &'a Live,
    expanded: &'a HashSet<String>,
    editing: &'a mut Option<Editing>,
    settable: bool,
    action: &'a mut Option<TreeAction>,
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
            if var.reference > 0 {
                ui::caret(ui.painter(), pos2(x + 4.0, r.center().y), ui::anim(ui, resp.id.with("open"), open), theme.text_dim);
            }
            if parent == 0 {
                pair(ui, r, x + 14.0, theme, &var.name, "", theme.text_dim);
            } else {
                pair(ui, r, x + 14.0, theme, &format!("{}:", var.name), &var.value, theme.text_dim);
            }
            let resp = match (parent, var.kind.is_empty()) {
                (0, _) => resp,
                (_, true) => resp.on_hover_text(&var.value),
                _ => resp.on_hover_text(format!("{}: {}", var.kind, var.value)),
            };
            if resp.clicked() && var.reference > 0 {
                *self.action = Some(TreeAction::Toggle(path.clone()));
            }
            if parent != 0 {
                resp.context_menu(|ui| {
                    ui::menu_pad(ui);
                    if self.settable && ui.button("Set Value").clicked() {
                        *self.action = Some(TreeAction::Edit(parent, var.name.clone(), var.value.clone()));
                        ui.close();
                    }
                    if ui.button("Copy Value").clicked() {
                        ui.ctx().copy_text(var.value.clone());
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
        self.reload(env.folder, now, false);
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
                let running = self.live.is_some();
                let tip = if running { "Debugging is running" } else { "Start Debugging (F5)" };
                if ui::icon_button(ui, theme, icons::PLAY, 24.0, false, tip).clicked() && !running {
                    env.cmds.push(Command::StartDebugging);
                }
                let width = (ui.available_width() - 30.0).max(60.0);
                egui::ComboBox::from_id_salt("debug_config").selected_text(&self.selected).width(width).show_ui(ui, |ui| {
                    for config in &self.configs {
                        ui.selectable_value(&mut self.selected, config.name.clone(), &config.name);
                    }
                    ui.separator();
                    if ui.button("Add Configuration...").clicked() {
                        env.cmds.push(Command::AddConfiguration);
                    }
                });
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
            if !env.marks.files.is_empty() {
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
        let Some(live) = self.live.as_ref().filter(|l| l.state == State::Stopped) else {
            return note(ui, theme, if self.live.is_some() { "Variables show while the program is paused." } else { "Not debugging." });
        };
        if live.scopes.is_empty() {
            return ui::skeleton(ui, theme, 3);
        }
        let mut action = None;
        let mut editing = self.editing.take();
        let mut tree = Tree { live, expanded: &self.expanded, editing: &mut editing, settable: live.can("supportsSetVariable"), action: &mut action };
        tree.show(ui, theme, &live.scopes, 0, 0, "");
        self.editing = editing;
        match action {
            Some(TreeAction::Toggle(path)) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
            }
            Some(TreeAction::Edit(reference, name, value)) => self.editing = Some(Editing::Value(reference, name, value)),
            Some(TreeAction::Cancel) => self.editing = None,
            Some(TreeAction::Commit(reference, name, value)) => {
                self.editing = None;
                if let Some(live) = self.live.as_mut() {
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
            let value = self.live.as_ref().and_then(|l| l.watch.get(expression));
            let (shown, color) = match value {
                Some(Ok(value)) => (value.clone(), theme.text_dim),
                Some(Err(error)) => (error.clone(), theme.error),
                None => (String::from(if self.live.as_ref().is_some_and(|l| l.state == State::Stopped) { "..." } else { "not available" }), theme.text_dim),
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
            if let Some(live) = self.live.as_mut() {
                live.watch_all(&self.watches);
            }
        }
        if let Some(index) = remove {
            self.watches.remove(index);
        }
    }

    fn stack_ui(&mut self, ui: &mut Ui, theme: &Theme, env: &mut Env) {
        let Some(live) = self.live.as_mut() else { return note(ui, theme, "Not debugging.") };
        if live.threads.is_empty() {
            return note(ui, theme, if live.state == State::Stopped { "Loading threads..." } else { "Running." });
        }
        let mut pick = None;
        for (id, name) in live.threads.clone() {
            let (r, _) = row(ui, theme, false);
            let current = live.thread == Some(id) && live.state == State::Stopped;
            let state = if current { format!("Paused on {}", live.reason) } else { String::from(if live.state == State::Stopped { "Paused" } else { "Running" }) };
            pair(ui, r, r.left() + 12.0, theme, &format!("{} {name}", icons::STACK), &state, theme.text_dim);
            if !current {
                continue;
            }
            for (index, frame) in live.frames.iter().enumerate() {
                let (r, resp) = row(ui, theme, index == live.frame);
                let place = frame.path.as_deref().map(|p| format!("{}:{}", p.file_name().unwrap_or_default().to_string_lossy(), frame.line)).unwrap_or_else(|| "unknown source".into());
                pair(ui, r, r.left() + 26.0, theme, &frame.name, &place, theme.text_dim);
                if resp.clicked() {
                    pick = Some(index);
                }
            }
        }
        if let Some(index) = pick {
            live.select(index, &self.watches, env.marks, env.cmds);
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
        if send_filters && let Some(live) = self.live.as_mut().filter(|l| l.configured) {
            live.send_filters(&self.filters);
        }
        if env.marks.files.is_empty() && self.filters.is_empty() {
            return note(ui, theme, "Click left of a line number or press F9 to add a breakpoint.");
        }
        let mut action = None;
        for (path, list) in &env.marks.files {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            for b in list {
                let (r, resp) = row(ui, theme, false);
                let mut on = b.enabled;
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

    pub fn toolbar(&mut self, ctx: &egui::Context, theme: &Theme, top: f32, cmds: &mut Vec<Command>) {
        let Some(live) = &self.live else { return };
        let stopped = live.state == State::Stopped;
        egui::Area::new(Id::new("debug_toolbar")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, vec2(0.0, top)).show(ctx, |ui| {
            ui.set_opacity(ui::fade_in(ui, Id::new("debug_toolbar_fade")));
            egui::Frame::popup(&ctx.global_style()).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
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
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, env: &mut Env) {
        if let Some((path, line, ask)) = env.marks.ask.take() {
            let found = env.marks.of(&path).iter().find(|b| b.line == line).cloned().unwrap_or_default();
            self.dialog = Some(Dialog { path, line, ask, texts: [found.condition, found.hit, found.log], focus: true });
        }
        if let Some(dialog) = &mut self.dialog {
            let mut done = None;
            let modal = ui::modal(ctx, "breakpoint_dialog").show(ctx, |ui| {
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
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = Some(true);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
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
        if self.confirm_clear {
            let mut choice = None;
            let modal = ui::modal(ctx, "confirm_clear_breakpoints").show(ctx, |ui| {
                ui::outline(ui);
                ui.set_width(340.0);
                ui.heading("Remove all breakpoints?");
                let count: usize = env.marks.files.iter().map(|f| f.1.len()).sum();
                ui.label(format!("This removes {count} breakpoints in {} files.", env.marks.files.len()));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove All").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
            if modal.should_close() {
                choice = choice.or(Some(false));
            }
            if let Some(remove) = choice {
                self.confirm_clear = false;
                if remove {
                    env.marks.clear();
                }
            }
        }
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
        if ui.button(label).clicked() {
            picked = Some(action);
            ui.close();
        }
    }
    picked
}

#[allow(clippy::too_many_arguments)]
pub fn paint_gutter(painter: &egui::Painter, theme: &Theme, list: &[Breakpoint], live: bool, stopped: Option<usize>, line: usize, center: Pos2, radius: f32, hovered: bool) {
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
    if stopped == Some(line) {
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
        let env = Env {
            folder: self.session.folder.as_deref(),
            file,
            settings: &self.settings,
            marks: &mut self.editors.marks,
            bottom: &mut self.bottom,
            panel: &mut self.session.panel,
            cmds: &mut self.cmds,
            ctx,
        };
        (&mut self.debug, env)
    }

    pub(crate) fn debug_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        let (debug, mut env) = self.debug_env(ctx);
        debug.exec(cmd, &mut env)
    }

    pub(crate) fn debug_frame(&mut self, ctx: &egui::Context, top: f32) {
        let theme = self.theme.clone();
        let (debug, mut env) = self.debug_env(ctx);
        debug.poll(&mut env);
        debug.dialogs(ctx, &mut env);
        debug.toolbar(ctx, &theme, top, env.cmds);
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
        assert_eq!(jsonc(r#"{"a": "x\"//y", "b": [1, 2 , ] }"#), r#"{"a": "x\"//y", "b": [1, 2  ] }"#);
        assert!(parse_launch("{ not json").unwrap_err().starts_with("launch.json could not be read"));
        assert!(parse_launch("{}").unwrap().is_empty());
    }

    #[test]
    fn substitutes_launch_variables() {
        let folder = PathBuf::from("/work/app");
        let vars = Vars { folder: folder.clone(), file: Some(folder.join("src").join("main.test.py")) };
        let shown = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(expand("${workspaceFolder}/bin", &vars), format!("{}/bin", shown(folder.clone())));
        assert_eq!(expand("${file}", &vars), shown(folder.join("src").join("main.test.py")));
        assert_eq!(expand("${fileBasename}|${fileBasenameNoExtension}", &vars), "main.test.py|main.test");
        assert_eq!(expand("${fileDirname}", &vars), shown(folder.join("src")));
        assert_eq!(expand("${relativeFile}", &vars), shown(PathBuf::from("src").join("main.test.py")));
        let key = if cfg!(windows) { "USERNAME" } else { "HOME" };
        assert_eq!(expand(&format!("${{env:{key}}}"), &vars), std::env::var(key).unwrap_or_default());
        assert_eq!(expand("${unknown} ${env:HALO_NO_SUCH_VAR}x ${unclosed", &vars), "${unknown} x ${unclosed");
        let none = Vars { folder, file: None };
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

    fn adapter(file: PathBuf) -> (u16, Receiver<Value>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut seq = 100;
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
                        let caps = json!({"supportsConfigurationDoneRequest": true, "supportsSetVariable": true, "supportsConditionalBreakpoints": true,
                            "supportsLogPoints": true, "exceptionBreakpointFilters": [{"filter": "raised", "label": "Raised Exceptions", "default": true}]});
                        send(&mut writer, reply(caps));
                        send(&mut writer, event("initialized", json!({})));
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
                        {"id": 7, "name": "main", "line": 3, "column": 5, "source": {"path": file.to_string_lossy()}},
                        {"id": 8, "name": "start", "line": 1, "column": 1},
                    ]}))),
                    "scopes" => send(&mut writer, reply(json!({"scopes": [{"name": "Locals", "variablesReference": 100}, {"name": "Globals", "variablesReference": 200, "expensive": true}]}))),
                    "variables" => send(&mut writer, reply(json!({"variables": [{"name": "x", "value": "42", "type": "int", "variablesReference": 0}, {"name": "items", "value": "[2]", "variablesReference": 101}]}))),
                    "evaluate" => {
                        let expression = request["arguments"]["expression"].as_str().unwrap_or_default();
                        let result = if expression == "x * 2" { "84" } else { "42" };
                        send(&mut writer, reply(json!({"result": result, "variablesReference": 0})));
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
        let (port, requests) = adapter(file.clone());
        let launch = format!("{{\n // fake\n \"configurations\": [{{\"type\": \"fake\", \"request\": \"launch\", \"name\": \"Fake\", \"program\": \"${{file}}\", \"debugServer\": {port}, \"custom\": [1,],}},]}}");
        std::fs::write(launch_path(&dir), launch).unwrap();

        let ctx = egui::Context::default();
        let settings = Settings::default();
        let (mut marks, mut bottom, mut panel, mut cmds) = (Marks::default(), BottomPanel::new(), false, vec![]);
        marks.toggle(&file, 2);
        marks.set(&file, 3, ["x > 1".into(), String::new(), String::new()]);
        marks.set(&file, 0, [String::new(), String::new(), "a is {a}".into()]);
        let mut debug = Debug::default();
        debug.watches.push("x * 2".into());
        macro_rules! env {
            () => {
                Env { folder: Some(&dir), file: Some(file.clone()), settings: &settings, marks: &mut marks, bottom: &mut bottom, panel: &mut panel, cmds: &mut cmds, ctx: &ctx }
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
        pump!(|d: &Debug, _: &BottomPanel| d.live.as_ref().is_some_and(|l| l.children.contains_key(&100)));

        let live = debug.live.as_ref().unwrap();
        assert_eq!(live.state, State::Stopped);
        assert_eq!(live.threads, [(1, "main".to_owned())]);
        assert_eq!(live.frames.len(), 2);
        assert_eq!(live.scopes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Locals", "Globals"]);
        assert!(debug.expanded.contains("/Locals") && !debug.expanded.contains("/Globals"));
        assert_eq!(live.children[&100].iter().map(|v| (v.name.as_str(), v.value.as_str())).collect::<Vec<_>>(), [("x", "42"), ("items", "[2]")]);
        assert_eq!(marks.stopped, Some((file.clone(), 2)));
        assert!(cmds.contains(&Command::OpenPath(file.clone())) && cmds.contains(&Command::GoTo(3, 5)));
        assert_eq!(debug.filters, [("raised".to_owned(), "Raised Exceptions".to_owned(), true)]);
        assert!(marks.of(&file).iter().all(|b| b.verified));
        assert!(bottom.console.text().contains("hello from the program\n"));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            debug.ui(ui, &Theme::dark(), &mut env!());
            debug.toolbar(ui.ctx(), &Theme::dark(), 40.0, &mut vec![]);
            bottom.console.ui(ui, Rect::from_min_size(pos2(0.0, 500.0), vec2(320.0, 200.0)), &Theme::dark(), 13.0);
        });
        out.textures_delta.clear();

        pump!(|d: &Debug, _: &BottomPanel| d.live.as_ref().is_some_and(|l| l.watch.contains_key("x * 2")));
        assert_eq!(debug.live.as_ref().unwrap().watch["x * 2"], Ok("84".to_owned()));
        bottom.console.submitted.push("x".into());
        pump!(|_: &Debug, b: &BottomPanel| b.console.text().ends_with("the program\n42\n"));

        assert!(debug.exec(&Command::StartDebugging, &mut env!()));
        pump!(|d: &Debug, _: &BottomPanel| !d.active());
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
        let exceptions = sent.iter().find(|r| r["command"] == "setExceptionBreakpoints").unwrap();
        assert_eq!(exceptions["arguments"]["filters"], json!(["raised"]));
        assert!(sent.iter().any(|r| r["command"] == "evaluate" && r["arguments"]["context"] == "watch" && r["arguments"]["frameId"] == 7));
        assert!(sent.iter().any(|r| r["command"] == "evaluate" && r["arguments"]["context"] == "repl" && r["arguments"]["expression"] == "x"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
