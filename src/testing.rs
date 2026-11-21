use crate::app::{Halo, note};
use crate::command::Command;
use crate::debug::Config;
use crate::test_parse::{self as tp, GoRun, Hits, Kind, Outcome, State, Test};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, ScrollArea, Sense, TextEdit, Ui, UiBuilder, Vec2, pos2, vec2};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Ctl {
    cancelled: bool,
    pid: Option<u32>,
}

type Shared = Arc<Mutex<Ctl>>;

enum Msg {
    Listed(Kind, Result<Vec<Test>, String>),
    Kinds(Vec<Kind>),
    Line(String),
    Done(Result<Finished, String>),
    Debug(Result<Config, String>),
}

struct Finished {
    report: String,
    coverage: Vec<(PathBuf, Hits)>,
}

struct Proc {
    rx: Receiver<Msg>,
    ctl: Shared,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    kind: Kind,
    ids: Option<Vec<String>>,
    coverage: bool,
}

struct Active {
    job: Job,
    proc: Proc,
    output: String,
    go: GoRun,
}

enum Action {
    Run(Vec<Job>),
    Debug(Kind, String),
}

pub fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) { text.to_lowercase() } else { text }
}

#[derive(Default, Clone)]
pub struct FileMarks {
    pub tests: Vec<(usize, State, Kind, String)>,
    pub failures: Vec<(usize, String)>,
    pub coverage: Option<Hits>,
}

#[derive(Default)]
pub struct Marks {
    files: HashMap<String, FileMarks>,
    pub clicked: Option<(Kind, String, bool)>,
    edited: Vec<(PathBuf, usize, isize)>,
}

impl Marks {
    pub fn of(&self, path: &Path) -> Option<&FileMarks> {
        self.files.get(&key(path))
    }

    pub fn edited(&mut self, path: &Path, line: usize, delta: isize) {
        if let Some(file) = self.files.get_mut(&key(path)) {
            file.failures.clear();
            file.coverage = None;
            file.tests.iter_mut().filter(|t| t.0 > line).for_each(|t| t.0 = (t.0 as isize + delta).max(0) as usize);
        }
        self.edited.push((path.to_path_buf(), line, delta));
    }

    #[cfg(test)]
    pub fn put(&mut self, path: &Path, file: FileMarks) {
        self.files.insert(key(path), file);
    }
}

pub fn paint_glyph(painter: &egui::Painter, theme: &Theme, state: State, center: egui::Pos2, size: f32) {
    let (icon, color) = match state {
        State::NotRun => (icons::PLAY, theme.added),
        State::Running => (icons::CIRCLE_NOTCH, theme.accent),
        other => state_icon(theme, other),
    };
    painter.text(center, Align2::CENTER_CENTER, icon, FontId::proportional(size), color);
}

fn state_icon(theme: &Theme, state: State) -> (&'static str, Color32) {
    match state {
        State::NotRun => (icons::CIRCLE, theme.text_dim),
        State::Queued => (icons::CLOCK, theme.text_dim),
        State::Running => (icons::CIRCLE_NOTCH, theme.accent),
        State::Passed => (icons::CHECK_CIRCLE, theme.added),
        State::Failed => (icons::X_CIRCLE, theme.error),
        State::Skipped => (icons::CIRCLE_DASHED, theme.text_dim),
    }
}

pub fn summary(states: impl Iterator<Item = State>) -> State {
    let mut seen = [false; 6];
    states.for_each(|s| seen[s as usize] = true);
    [State::Running, State::Queued, State::Failed, State::Passed, State::Skipped].into_iter().find(|s| seen[*s as usize]).unwrap_or(State::NotRun)
}

pub type Tree = Vec<(Kind, Vec<(String, Vec<usize>)>)>;

pub fn tree(tests: &[Test], filter: &str) -> Tree {
    let filter = filter.trim().to_lowercase();
    let mut out: BTreeMap<Kind, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
    for (index, test) in tests.iter().enumerate() {
        let failed = filter == "@failed" && test.state == State::Failed;
        if filter.is_empty() || failed || [&test.name, &test.group, &test.id].iter().any(|s| s.to_lowercase().contains(&filter)) {
            out.entry(test.kind).or_default().entry(test.group.clone()).or_default().push(index);
        }
    }
    out.into_iter().map(|(kind, groups)| (kind, groups.into_iter().collect())).collect()
}

fn picks<'a>(job: &'a Job) -> impl Fn(&Test) -> bool + 'a {
    move |t: &Test| t.kind == job.kind && job.ids.as_ref().is_none_or(|ids| ids.contains(&t.id))
}

fn begin(tests: &mut [Test], job: &Job, state: State) {
    for test in tests.iter_mut().filter(|t| picks(job)(t)) {
        if state == State::Queued {
            (test.duration, test.fail) = (None, None);
            test.message.clear();
            test.output.clear();
        }
        test.state = state;
    }
}

fn settle(tests: &mut [Test], kind: Kind) {
    for test in tests.iter_mut().filter(|t| t.kind == kind && matches!(t.state, State::Queued | State::Running)) {
        test.state = State::NotRun;
    }
}

fn resolve(root: Option<&Path>, test_file: Option<&Path>, file: &str) -> PathBuf {
    let path = PathBuf::from(file);
    if path.is_absolute() {
        return path;
    }
    match (root.map(|r| r.join(&path)), test_file.and_then(Path::parent)) {
        (Some(full), _) if full.is_file() => full,
        (_, Some(dir)) if dir.join(&path).is_file() => dir.join(&path),
        (Some(full), _) => full,
        _ => path,
    }
}

fn apply(tests: &mut [Test], kind: Kind, outcome: Outcome, root: Option<&Path>) -> bool {
    let Some(test) = tests.iter_mut().find(|t| t.kind == kind && t.id == outcome.id) else { return false };
    test.state = outcome.state;
    test.duration = outcome.duration.or(test.duration);
    if !outcome.message.is_empty() {
        test.message = outcome.message;
    }
    if !outcome.output.is_empty() {
        test.output = outcome.output;
    }
    if let Some((file, line)) = outcome.at {
        test.fail = Some((resolve(root, test.file.as_deref(), &file), line.saturating_sub(1)));
    }
    true
}

fn merge(tests: &mut Vec<Test>, kind: Kind, found: Vec<Test>) {
    let old: HashMap<String, Test> = tests.iter().filter(|t| t.kind == kind).map(|t| (t.id.clone(), t.clone())).collect();
    tests.retain(|t| t.kind != kind);
    tests.extend(found.into_iter().map(|mut test| {
        if let Some(prev) = old.get(&test.id) {
            (test.state, test.duration, test.fail) = (prev.state, prev.duration, prev.fail.clone());
            (test.message, test.output) = (prev.message.clone(), prev.output.clone());
        }
        test
    }));
}

fn kill(pid: u32) {
    #[cfg(windows)]
    let _ = crate::terminal::hidden_command("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output();
}

fn stop(ctl: &Shared) {
    let mut ctl = ctl.lock().unwrap();
    ctl.cancelled = true;
    if let Some(pid) = ctl.pid.take() {
        std::thread::spawn(move || kill(pid));
    }
}

fn stream(program: &str, args: &[String], cwd: &Path, ctl: &Shared, mut each: impl FnMut(bool, &str)) -> Result<Option<i32>, String> {
    if ctl.lock().unwrap().cancelled {
        return Err("Cancelled.".into());
    }
    let spawned = crate::terminal::hidden_command(program).args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = spawned.map_err(|e| format!("Could not run {program}: {e}"))?;
    {
        let mut ctl = ctl.lock().unwrap();
        match ctl.cancelled {
            true => drop(child.kill()),
            false => ctl.pid = Some(child.id()),
        }
    }
    let (tx, rx) = channel();
    let readers: [(bool, Option<Box<dyn Read + Send>>); 2] =
        [(false, child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>)), (true, child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>))];
    for (err, reader) in readers {
        let (tx, Some(reader)) = (tx.clone(), reader) else { continue };
        std::thread::spawn(move || {
            for line in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
                let _ = tx.send((err, String::from_utf8_lossy(&line).trim_end_matches('\r').to_owned()));
            }
        });
    }
    drop(tx);
    for (err, line) in rx {
        each(err, &line);
    }
    let status = child.wait().map_err(|e| format!("{program} failed: {e}"))?;
    ctl.lock().unwrap().pid = None;
    Ok(status.code())
}

fn capture(program: &str, args: &[String], cwd: &Path, ctl: &Shared) -> Result<(Option<i32>, String, String), String> {
    let (mut out, mut err) = (String::new(), String::new());
    let code = stream(program, args, cwd, ctl, |is_err, line| {
        let target = if is_err { &mut err } else { &mut out };
        target.push_str(line);
        target.push('\n');
    })?;
    Ok((code, out, err))
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.trim().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn discover(root: &Path, kind: Kind, ctl: &Shared) -> Result<Vec<Test>, String> {
    let (code, out, err) = capture(kind.program(), &tp::list_args(kind), root, ctl)?;
    let tests = match kind {
        Kind::Rust => {
            let files: Vec<(PathBuf, String)> = tp::sources(root, &["rs"]).into_iter().filter_map(|p| Some((p.clone(), std::fs::read_to_string(&p).ok()?))).collect();
            tp::rust_tests(root, &tp::rust_list(&out), &files)
        }
        Kind::Python => tp::python_tests(root, &tp::python_list(&out)),
        Kind::Go => tp::go_tests(root, &tp::go_list(&out)),
        Kind::Jest => tp::js_tests(kind, root, &tp::jest_list(&out)),
        Kind::Vitest => tp::vitest_list(root, &out),
    };
    if tests.is_empty() && code != Some(0) {
        return Err(format!("{}: listing tests failed.\n{}", kind.label(), tail(&format!("{out}\n{err}"), 12)));
    }
    Ok(tests)
}

fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!("halo-tests-{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn read_coverage(kind: Kind, root: &Path, cover: &Path) -> Vec<(PathBuf, Hits)> {
    let file = if matches!(kind, Kind::Jest | Kind::Vitest) { cover.join("lcov.info") } else { cover.to_path_buf() };
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let _ = if cover.is_dir() { std::fs::remove_dir_all(cover) } else { std::fs::remove_file(cover) };
    if kind == Kind::Go {
        let module = tp::go_module(root);
        return tp::cover_profile(&text)
            .into_iter()
            .map(|(file, hits)| {
                let (package, name) = file.rsplit_once('/').unwrap_or(("", &file));
                (tp::go_dir(root, &module, package).join(name), hits)
            })
            .collect();
    }
    tp::lcov(&text).into_iter().map(|(file, hits)| (root.join(file), hits)).collect()
}

fn run_job(root: &Path, job: &Job, picked: &[Test], tx: &Sender<Msg>, ctl: &Shared, ctx: &egui::Context) -> Result<Finished, String> {
    let program = job.kind.program();
    if job.coverage && let Some((args, notice)) = tp::coverage_check(job.kind, root) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        if args.is_empty() || !capture(program, &args, root, ctl).is_ok_and(|r| r.0 == Some(0)) {
            return Err(notice.into());
        }
    }
    let (report, cover) = (temp_path("report"), job.coverage.then(|| temp_path("cover")));
    let refs: Vec<&Test> = picked.iter().collect();
    let args = tp::run_args(job.kind, job.ids.as_ref().map(|_| refs.as_slice()), &report, cover.as_deref());
    let _ = tx.send(Msg::Line(format!("> {program} {}", args.join(" "))));
    let code = stream(program, &args, root, ctl, |_, line| {
        let _ = tx.send(Msg::Line(line.to_owned()));
        ctx.request_repaint();
    })?;
    let _ = tx.send(Msg::Line(format!("[{} finished with exit code {}]", job.kind.label(), code.map_or("none".into(), |c| c.to_string()))));
    let text = std::fs::read_to_string(&report).unwrap_or_default();
    let _ = std::fs::remove_file(&report);
    let coverage = cover.map(|c| read_coverage(job.kind, root, &c)).unwrap_or_default();
    Ok(Finished { report: text, coverage })
}

fn debug_build(root: &Path, test: &Test, ctl: &Shared) -> Result<Config, String> {
    let args: Vec<String> = ["test", "--no-run", "--message-format=json"].map(str::to_owned).to_vec();
    let (code, out, err) = capture("cargo", &args, root, ctl)?;
    if code != Some(0) {
        return Err(format!("The tests could not be built for debugging.\n{}", tail(&err, 12)));
    }
    let list: Vec<String> = [test.id.as_str(), "--exact", "--list", "--format", "terse"].map(str::to_owned).to_vec();
    let wanted = format!("{}: test", test.id);
    let exe = tp::executables(&out).into_iter().find(|exe| capture(&exe.to_string_lossy(), &list, root, ctl).is_ok_and(|r| r.1.lines().any(|l| l.trim() == wanted)));
    tp::debug_config(test, root, exe.as_deref())
}

pub struct Testing {
    root: Option<PathBuf>,
    parked: Vec<Testing>,
    loaded: bool,
    kinds: Vec<Kind>,
    tests: Vec<Test>,
    listing: Vec<Proc>,
    errors: Vec<String>,
    queue: VecDeque<Job>,
    active: Option<Active>,
    last: Vec<Job>,
    output: String,
    coverage: BTreeMap<String, (PathBuf, Hits)>,
    show_coverage: bool,
    filter: String,
    collapsed: HashSet<String>,
    selected: Option<(Kind, String)>,
    cancelled: bool,
    ran: bool,
    open: [bool; 2],
    actions: Vec<Action>,
    debug: Vec<Config>,
    dirty: bool,
    output_dirty: bool,
}

impl Default for Testing {
    fn default() -> Self {
        Self {
            root: None,
            parked: vec![],
            loaded: false,
            kinds: vec![],
            tests: vec![],
            listing: vec![],
            errors: vec![],
            queue: VecDeque::new(),
            active: None,
            last: vec![],
            output: String::new(),
            coverage: BTreeMap::new(),
            show_coverage: true,
            filter: String::new(),
            collapsed: HashSet::new(),
            selected: None,
            cancelled: false,
            ran: false,
            open: [true; 2],
            actions: vec![],
            debug: vec![],
            dirty: true,
            output_dirty: true,
        }
    }
}

impl Testing {
    fn set_root(&mut self, root: Option<&Path>, park: bool) {
        if self.root.as_deref() == root {
            return;
        }
        let show_coverage = self.show_coverage;
        let mut parked = std::mem::take(&mut self.parked);
        if park && self.root.is_some() {
            parked.push(std::mem::take(self));
        } else if !park {
            self.cancel();
            parked.iter_mut().for_each(Testing::cancel);
            parked.clear();
        }
        let found = parked.iter().position(|t| t.root.as_deref() == root).map(|i| parked.remove(i));
        *self = found.unwrap_or_else(|| Testing { root: root.map(Path::to_path_buf), show_coverage, ..Default::default() });
        self.parked = parked;
    }

    fn ensure(&mut self, ctx: &egui::Context) {
        if !self.loaded && self.root.is_some() {
            self.refresh(ctx);
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        let Some(root) = self.root.clone() else { return };
        self.listing.iter().for_each(|p| stop(&p.ctl));
        (self.loaded, self.errors) = (true, vec![]);
        let (tx, rx) = channel();
        let ctl = Shared::default();
        self.listing.push(Proc { rx, ctl: ctl.clone() });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let kinds = tp::detect(&root);
            let _ = tx.send(Msg::Kinds(kinds.clone()));
            ctx.request_repaint();
            for kind in kinds {
                let _ = tx.send(Msg::Listed(kind, discover(&root, kind, &ctl)));
                ctx.request_repaint();
            }
        });
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.queue.clear();
        self.listing.iter().for_each(|p| stop(&p.ctl));
        if let Some(active) = &self.active {
            stop(&active.proc.ctl);
        }
    }

    fn busy(&self) -> bool {
        self.active.is_some()
    }

    fn listing(&self) -> bool {
        !self.listing.is_empty()
    }

    fn start(&mut self, jobs: Vec<Job>, ctx: &egui::Context) {
        if self.busy() {
            return ui::notify("A test run is already in progress. Cancel it first (Ctrl+; Ctrl+X).");
        }
        let jobs: Vec<Job> = jobs.into_iter().filter(|j| j.ids.as_ref().is_none_or(|ids| !ids.is_empty())).collect();
        if jobs.is_empty() {
            return ui::notify(if self.listing() { "Tests are still being discovered." } else { "There are no tests to run." });
        }
        (self.last, self.cancelled, self.output_dirty) = (jobs.clone(), false, true);
        self.output.clear();
        for job in &jobs {
            begin(&mut self.tests, job, State::Queued);
        }
        self.queue = jobs.into();
        self.next(ctx);
    }

    fn next(&mut self, ctx: &egui::Context) {
        self.dirty = true;
        let (Some(root), Some(job)) = (self.root.clone(), self.queue.pop_front()) else {
            self.ran = true;
            return;
        };
        begin(&mut self.tests, &job, State::Running);
        let picked: Vec<Test> = self.tests.iter().filter(|t| picks(&job)(t)).cloned().collect();
        let (tx, rx) = channel();
        let ctl = Shared::default();
        let (thread_job, thread_ctl, ctx) = (job.clone(), ctl.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = run_job(&root, &thread_job, &picked, &tx, &thread_ctl, &ctx);
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
        self.active = Some(Active { job, proc: Proc { rx, ctl }, output: String::new(), go: GoRun::default() });
    }

    fn jobs_for(&self, ids: &[(Kind, String)], coverage: bool) -> Vec<Job> {
        let mut jobs: Vec<Job> = vec![];
        for (kind, id) in ids {
            match jobs.iter_mut().find(|j| j.kind == *kind) {
                Some(job) => job.ids.get_or_insert_with(Vec::new).push(id.clone()),
                None => jobs.push(Job { kind: *kind, ids: Some(vec![id.clone()]), coverage }),
            }
        }
        jobs
    }

    fn all_jobs(&self, coverage: bool) -> Vec<Job> {
        let mut kinds: Vec<Kind> = self.tests.iter().map(|t| t.kind).collect();
        kinds.dedup();
        kinds.into_iter().map(|kind| Job { kind, ids: None, coverage }).collect()
    }

    fn failed(&self) -> Vec<(Kind, String)> {
        self.tests.iter().filter(|t| t.state == State::Failed).map(|t| (t.kind, t.id.clone())).collect()
    }

    fn at(&self, path: &Path, line: usize) -> Option<&Test> {
        let file = key(path);
        let here = self.tests.iter().filter(|t| t.file.as_deref().is_some_and(|f| key(f) == file) && t.line.is_some_and(|l| l <= line));
        here.max_by_key(|t| t.line)
    }

    fn debug_test(&mut self, kind: Kind, id: &str, ctx: &egui::Context) {
        let (Some(root), Some(test)) = (self.root.clone(), self.tests.iter().find(|t| t.kind == kind && t.id == id).cloned()) else { return };
        if kind != Kind::Rust {
            return match tp::debug_config(&test, &root, None) {
                Ok(config) => self.debug.push(config),
                Err(error) => ui::notify(error),
            };
        }
        let (tx, rx) = channel();
        let ctl = Shared::default();
        self.listing.push(Proc { rx, ctl: ctl.clone() });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Debug(debug_build(&root, &test, &ctl)));
            ctx.request_repaint();
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut procs = std::mem::take(&mut self.listing);
        procs.retain(|proc| {
            let messages: Vec<Msg> = proc.rx.try_iter().collect();
            for message in messages {
                match message {
                    Msg::Kinds(kinds) => self.kinds = kinds,
                    Msg::Listed(kind, Ok(found)) => {
                        merge(&mut self.tests, kind, found);
                        self.dirty = true;
                    }
                    Msg::Listed(_, Err(error)) => self.errors.extend((!proc.ctl.lock().unwrap().cancelled).then_some(error)),
                    Msg::Debug(Ok(config)) => self.debug.push(config),
                    Msg::Debug(Err(error)) => ui::notify(error),
                    _ => {}
                }
            }
            !matches!(proc.rx.try_recv(), Err(TryRecvError::Disconnected))
        });
        procs.append(&mut self.listing);
        self.listing = procs;
        let Some(active) = self.active.as_mut() else { return };
        let mut messages: Vec<Msg> = active.proc.rx.try_iter().collect();
        if !messages.iter().any(|m| matches!(m, Msg::Done(_))) && matches!(active.proc.rx.try_recv(), Err(TryRecvError::Disconnected)) {
            messages.push(Msg::Done(Err("The test run stopped unexpectedly.".into())));
        }
        for message in messages {
            match message {
                Msg::Line(line) => self.line(line),
                Msg::Done(result) => self.finish(result, ctx),
                _ => {}
            }
        }
    }

    fn line(&mut self, line: String) {
        let Some(active) = self.active.as_mut() else { return };
        let kind = active.job.kind;
        let shown = match kind {
            Kind::Go => serde_json::from_str::<serde_json::Value>(&line).map(|v| v["Output"].as_str().unwrap_or_default().to_owned()).unwrap_or(line.clone() + "\n"),
            _ => line.clone() + "\n",
        };
        active.output.push_str(&line);
        active.output.push('\n');
        self.output.push_str(&shown);
        self.output_dirty = true;
        let outcome = match kind {
            Kind::Rust => tp::libtest_line(&line),
            Kind::Python => tp::pytest_line(&line),
            Kind::Go => active.go.feed(&line),
            Kind::Jest | Kind::Vitest => None,
        };
        if let Some(outcome) = outcome {
            self.dirty |= apply(&mut self.tests, kind, outcome, self.root.as_deref());
        }
    }

    fn finish(&mut self, result: Result<Finished, String>, ctx: &egui::Context) {
        let Some(active) = self.active.take() else { return };
        let kind = active.job.kind;
        let (root, ok) = (self.root.clone(), result.is_ok());
        match result {
            Ok(finished) => {
                let outcomes = match kind {
                    Kind::Rust => tp::libtest_failures(&active.output),
                    Kind::Python => {
                        let nodes: HashMap<String, String> = self.tests.iter().filter(|t| t.kind == kind).map(|t| (tp::junit_key(&t.id), t.id.clone())).collect();
                        tp::junit(&finished.report).into_iter().filter_map(|o| Some(Outcome { id: nodes.get(&o.id)?.clone(), ..o })).collect()
                    }
                    Kind::Jest | Kind::Vitest => root.as_deref().map(|r| tp::js_results(r, &finished.report)).unwrap_or_default(),
                    Kind::Go => vec![],
                };
                for outcome in outcomes {
                    apply(&mut self.tests, kind, outcome, root.as_deref());
                }
                if active.job.coverage && finished.coverage.is_empty() && !self.cancelled {
                    ui::notify(format!("{} did not write a coverage report.", kind.label()));
                }
                for (path, hits) in finished.coverage {
                    self.coverage.insert(key(&path), (path, hits));
                }
            }
            Err(error) if !self.cancelled => {
                self.output.push_str(&format!("{error}\n"));
                ui::notify(error);
            }
            Err(_) => {}
        }
        if self.cancelled {
            self.output.push_str("Test run cancelled.\n");
        } else if ok && self.tests.iter().filter(|t| picks(&active.job)(t)).all(|t| t.state == State::Running) {
            ui::notify(format!("{} reported no test results. Test: Show Output has the details.", kind.label()));
        }
        self.output_dirty = true;
        settle(&mut self.tests, kind);
        self.next(ctx);
    }

    fn edited(&mut self, path: &Path, line: usize, delta: isize) {
        let file = key(path);
        self.coverage.remove(&file);
        for test in self.tests.iter_mut().filter(|t| t.file.as_deref().is_some_and(|f| key(f) == file)) {
            test.line = test.line.map(|l| if l > line { (l as isize + delta).max(0) as usize } else { l });
        }
        for test in self.tests.iter_mut().filter(|t| t.fail.as_ref().is_some_and(|f| key(&f.0) == file)) {
            test.fail = None;
        }
        self.dirty = true;
    }

    fn sync(&mut self, marks: &mut Marks) {
        self.dirty = false;
        marks.files.clear();
        for test in &self.tests {
            if let (Some(file), Some(line)) = (&test.file, test.line) {
                marks.files.entry(key(file)).or_default().tests.push((line, test.state, test.kind, test.id.clone()));
            }
            if let Some((file, line)) = &test.fail {
                let message = test.message.lines().next().unwrap_or("Test failed.").to_owned();
                marks.files.entry(key(file)).or_default().failures.push((*line, message));
            }
        }
        for (file, (_, hits)) in self.coverage.iter().filter(|_| self.show_coverage) {
            marks.files.entry(file.clone()).or_default().coverage = Some(hits.clone());
        }
    }

    pub fn status(&self) -> Option<(String, String)> {
        if self.busy() {
            return Some((format!("{} Running tests...", icons::FLASK), "Tests are running (click to show the Testing view)".into()));
        }
        if !self.ran {
            return None;
        }
        let count = |state| self.tests.iter().filter(|t| t.state == state).count();
        let (passed, failed) = (count(State::Passed), count(State::Failed));
        Some((format!("{} {passed}  {} {failed}", icons::CHECK_CIRCLE, icons::X_CIRCLE), format!("{passed} passed, {failed} failed (click to show the Testing view)")))
    }

    fn toolbar(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let buttons = [
                (icons::PLAY, "Run All Tests (Ctrl+; A)", Command::RunAllTests),
                (icons::ARROW_COUNTER_CLOCKWISE, "Run Failed Tests (Ctrl+; E)", Command::RunFailedTests),
                (icons::BUG, "Debug Selected Test, or the Test at the Cursor", Command::DebugTestAtCursor),
                (icons::UMBRELLA, "Run All Tests with Coverage", Command::RunAllTestsWithCoverage),
                (icons::ARROWS_CLOCKWISE, "Refresh Tests", Command::RefreshTests),
            ];
            for (icon, tip, cmd) in buttons {
                if ui::icon_button(ui, theme, icon, 24.0, false, tip).clicked() {
                    match (&cmd, &self.selected) {
                        (Command::DebugTestAtCursor, Some((kind, id))) => self.actions.push(Action::Debug(*kind, id.clone())),
                        _ => cmds.push(cmd),
                    }
                }
            }
            let busy = self.busy() || self.listing();
            if ui::icon_button(ui, theme, icons::STOP, 24.0, busy, "Cancel Test Run (Ctrl+; Ctrl+X)").clicked() && busy {
                cmds.push(Command::CancelTestRun);
            }
            if ui::icon_button(ui, theme, icons::TERMINAL_WINDOW, 24.0, false, "Show Output").clicked() {
                cmds.push(Command::ShowTestOutput);
            }
            if ui::icon_button(ui, theme, icons::ARROWS_IN_LINE_VERTICAL, 24.0, false, "Collapse All").clicked() {
                for test in &self.tests {
                    self.collapsed.insert(format!("{:?}", test.kind));
                    self.collapsed.insert(format!("{:?}/{}", test.kind, test.group));
                }
            }
        });
    }

    fn row(ui: &mut Ui, theme: &Theme, depth: usize, state: State, label: &str, detail: &str, selected: bool) -> (Rect, egui::Response) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        ui::row_bg(ui, theme, r, resp.id, resp.hovered(), selected);
        let x = r.left() + 12.0 + depth as f32 * 14.0;
        let painter = ui.painter().with_clip_rect(r);
        let icon_at = pos2(x + 20.0, r.center().y);
        match state {
            State::Running => ui::spinner(ui, icon_at, 5.0, theme.accent),
            other => {
                let (icon, color) = state_icon(theme, other);
                painter.text(icon_at, Align2::CENTER_CENTER, icon, FontId::proportional(13.0), color);
            }
        }
        let end = painter.text(pos2(x + 32.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.5), theme.text);
        if !resp.hovered() && !detail.is_empty() {
            painter.text(pos2(end.right() + 8.0, r.center().y), Align2::LEFT_CENTER, detail, FontId::proportional(11.5), theme.text_dim);
        }
        (r, resp)
    }

    fn tree_ui(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        let tree = tree(&self.tests, &self.filter);
        let filtered = !self.filter.trim().is_empty();
        if tree.is_empty() && filtered {
            return note(ui, theme, "No tests match the filter.");
        }
        let duration = |d: Option<f64>| d.map_or(String::new(), |d| if d < 1.0 { format!("{:.0} ms", d * 1000.0) } else { format!("{d:.1} s") });
        for (kind, groups) in tree {
            let pkey = format!("{kind:?}");
            let open = !self.collapsed.contains(&pkey);
            let state = summary(groups.iter().flat_map(|g| &g.1).map(|i| self.tests[*i].state));
            let count: usize = groups.iter().map(|g| g.1.len()).sum();
            let (r, resp) = Self::row(ui, theme, 0, state, kind.label(), &format!("{count}"), false);
            ui::caret(ui.painter(), pos2(r.left() + 12.0, r.center().y), ui::anim(ui, resp.id.with("open"), open), theme.text_dim);
            if resp.hovered() && hover_button(ui, theme, r, 0, icons::PLAY, &format!("Run {} Tests", kind.label())) {
                let ids: Vec<(Kind, String)> = groups.iter().flat_map(|g| &g.1).map(|i| (kind, self.tests[*i].id.clone())).collect();
                self.actions.push(Action::Run(if filtered { self.jobs_for(&ids, false) } else { vec![Job { kind, ids: None, coverage: false }] }));
            } else if resp.clicked() {
                self.toggle(pkey);
            }
            if !open {
                continue;
            }
            for (group, indices) in groups {
                let gkey = format!("{kind:?}/{group}");
                let open = !self.collapsed.contains(&gkey);
                let state = summary(indices.iter().map(|i| self.tests[*i].state));
                let (r, resp) = Self::row(ui, theme, 1, state, &group, &format!("{}", indices.len()), false);
                ui::caret(ui.painter(), pos2(r.left() + 26.0, r.center().y), ui::anim(ui, resp.id.with("open"), open), theme.text_dim);
                let file = self.tests[indices[0]].file.clone().filter(|_| self.tests[indices[0]].group != group || indices.iter().all(|i| self.tests[*i].file == self.tests[indices[0]].file));
                let mut used = false;
                if resp.hovered() {
                    if hover_button(ui, theme, r, 0, icons::PLAY, "Run Tests") {
                        let ids: Vec<(Kind, String)> = indices.iter().map(|i| (kind, self.tests[*i].id.clone())).collect();
                        self.actions.push(Action::Run(self.jobs_for(&ids, false)));
                        used = true;
                    }
                    if let Some(file) = &file
                        && hover_button(ui, theme, r, 1, icons::FILE_CODE, "Go to File")
                    {
                        cmds.push(Command::OpenPath(file.clone()));
                        used = true;
                    }
                }
                if resp.clicked() && !used {
                    self.toggle(gkey);
                }
                if !open {
                    continue;
                }
                for index in indices {
                    self.test_row(ui, theme, index, &duration(self.tests[index].duration), cmds);
                }
            }
        }
    }

    fn test_row(&mut self, ui: &mut Ui, theme: &Theme, index: usize, detail: &str, cmds: &mut Vec<Command>) {
        let test = self.tests[index].clone();
        let id = (test.kind, test.id.clone());
        let selected = self.selected.as_ref() == Some(&id);
        let (r, resp) = Self::row(ui, theme, 2, test.state, &test.name, detail, selected);
        let goto = |cmds: &mut Vec<Command>| {
            if let Some(file) = &test.file {
                cmds.push(Command::OpenPath(file.clone()));
                cmds.push(Command::GoTo(test.line.unwrap_or(0) + 1, 1));
            }
        };
        let mut used = false;
        if resp.hovered() {
            if hover_button(ui, theme, r, 0, icons::PLAY, "Run Test") {
                self.actions.push(Action::Run(self.jobs_for(std::slice::from_ref(&id), false)));
                used = true;
            }
            if hover_button(ui, theme, r, 1, icons::BUG, "Debug Test") {
                self.actions.push(Action::Debug(test.kind, test.id.clone()));
                used = true;
            }
            if test.file.is_some() && hover_button(ui, theme, r, 2, icons::FILE_CODE, "Go to Test") {
                goto(cmds);
                used = true;
            }
        }
        let resp = resp.on_hover_text(&test.id);
        if resp.clicked() && !used {
            self.selected = Some(id.clone());
            goto(cmds);
            if let Some((file, line)) = &test.fail {
                cmds.push(Command::OpenPath(file.clone()));
                cmds.push(Command::GoTo(line + 1, 1));
            }
        }
        if test.state != State::Failed {
            return;
        }
        egui::Frame::new().inner_margin(egui::Margin { left: 60, right: 8, top: 2, bottom: 4 }).show(ui, |ui| {
            let message = if test.message.is_empty() { "Test failed." } else { &test.message };
            ui.add(egui::Label::new(RichText::new(message).size(12.0).color(theme.error)).wrap());
            if selected && !test.output.is_empty() {
                let shown: Vec<&str> = test.output.lines().take(40).collect();
                ui.add(egui::Label::new(RichText::new(shown.join("\n")).font(FontId::monospace(11.5)).color(theme.text_dim)).wrap());
            }
        });
    }

    fn toggle(&mut self, node: String) {
        if !self.collapsed.remove(&node) {
            self.collapsed.insert(node);
        }
    }

    fn coverage_ui(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        if self.coverage.is_empty() {
            return;
        }
        let rect = ui::section(ui, theme, "COVERAGE", &mut self.open[1]).on_hover_text("Lines covered by the last run with coverage").rect;
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(20.0))));
        let (icon, tip) = if self.show_coverage { (icons::EYE, "Hide Coverage in the Editor") } else { (icons::EYE_SLASH, "Show Coverage in the Editor") };
        if ui::icon_button(&mut child, theme, icon, 20.0, false, tip).clicked() {
            cmds.push(Command::ToggleCoverageDisplay);
        }
        if !self.open[1] {
            return;
        }
        let root = self.root.clone().unwrap_or_default();
        let mut files: Vec<(f32, &PathBuf, usize, usize)> = self
            .coverage
            .values()
            .map(|(path, hits)| {
                let covered = hits.values().filter(|h| **h > 0).count();
                (if hits.is_empty() { 1.0 } else { covered as f32 / hits.len() as f32 }, path, covered, hits.len())
            })
            .collect();
        files.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        for (share, path, covered, total) in files {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            ui::row_bg(ui, theme, r, resp.id, resp.hovered(), false);
            let painter = ui.painter().with_clip_rect(r);
            let color = match share {
                s if s >= 0.8 => theme.added,
                s if s >= 0.5 => theme.warning,
                _ => theme.error,
            };
            let percent = painter.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, format!("{:.0}%", share * 100.0), FontId::proportional(12.0), color);
            let bar = Rect::from_min_size(pos2(percent.left() - 56.0, r.center().y - 2.0), vec2(48.0, 4.0));
            painter.rect_filled(bar, 2.0, ui::with_alpha(theme.text_dim, 0.25));
            painter.rect_filled(Rect::from_min_size(bar.min, vec2(48.0 * share, 4.0)), 2.0, color);
            painter.with_clip_rect(r.with_max_x(bar.left() - 6.0)).text(pos2(r.left() + 24.0, r.center().y), Align2::LEFT_CENTER, tp::rel(&root, path), FontId::proportional(12.5), theme.text);
            if resp.on_hover_text(format!("{covered} of {total} lines covered")).clicked() {
                cmds.push(Command::OpenPath(path.clone()));
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, theme: &Theme, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui::section_header(ui, theme, "Testing");
        if self.root.is_none() {
            return note(ui, theme, "Open a folder to discover its tests.");
        }
        self.ensure(ui.ctx());
        self.toolbar(ui, theme, cmds);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.add(TextEdit::singleline(&mut self.filter).hint_text("Filter (name, file, or @failed)").desired_width(f32::INFINITY).margin(ui::FIELD));
        });
        let status = match (self.busy(), self.listing()) {
            (true, _) => Some("Running tests..."),
            (_, true) => Some("Discovering tests..."),
            _ => None,
        };
        if let Some(text) = status {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            ui::spinner(ui, pos2(r.left() + 18.0, r.center().y), 5.0, theme.accent);
            ui.painter().text(pos2(r.left() + 30.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), theme.text_dim);
        } else if self.ran {
            let count = |state| self.tests.iter().filter(|t| t.state == state).count();
            note(ui, theme, &format!("{} passed, {} failed, {} skipped", count(State::Passed), count(State::Failed), count(State::Skipped)));
        }
        for error in &self.errors {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(error).size(12.0).color(theme.error)).wrap());
            });
        }
        ScrollArea::vertical().id_salt("testing").auto_shrink(false).show(ui, |ui| {
            if self.tests.is_empty() {
                match (self.listing(), self.kinds.is_empty()) {
                    (true, _) => ui::skeleton(ui, theme, 5),
                    (false, true) => note(ui, theme, "No test framework found (Cargo.toml, pytest, go.mod, or jest/vitest in package.json)."),
                    (false, false) => note(ui, theme, "No tests were found."),
                }
            } else {
                self.tree_ui(ui, theme, cmds);
            }
            ui.add_space(6.0);
            self.coverage_ui(ui, theme, cmds);
        });
    }
}

fn hover_button(ui: &mut Ui, theme: &Theme, r: Rect, slot: usize, icon: &str, tip: &str) -> bool {
    let rect = Rect::from_center_size(pos2(r.right() - 14.0 - slot as f32 * 22.0, r.center().y), vec2(20.0, 20.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ui::icon_button(&mut child, theme, icon, 20.0, false, tip).clicked()
}

impl Halo {
    fn testing_act(&mut self, action: Action, ctx: &egui::Context) {
        self.editors.exec(&Command::SaveAll, &self.settings, ctx);
        match action {
            Action::Run(jobs) => self.testing.start(jobs, ctx),
            Action::Debug(kind, id) => self.testing.debug_test(kind, &id, ctx),
        }
    }

    pub(crate) fn testing_frame(&mut self, ctx: &egui::Context) {
        let folder = self.active_root();
        self.testing.set_root(folder.as_deref(), self.session.workspace.is_multi());
        self.testing.poll(ctx);
        for (path, line, delta) in std::mem::take(&mut self.editors.tests.edited) {
            self.testing.edited(&path, line, delta);
        }
        if let Some((kind, id, debug)) = self.editors.tests.clicked.take() {
            self.testing.actions.push(if debug { Action::Debug(kind, id) } else { Action::Run(self.testing.jobs_for(&[(kind, id)], false)) });
        }
        for action in std::mem::take(&mut self.testing.actions) {
            self.testing_act(action, ctx);
        }
        if self.testing.dirty {
            self.testing.sync(&mut self.editors.tests);
        }
        if self.testing.output_dirty {
            self.testing.output_dirty = false;
            let (out, shown) = (&self.testing.output, &mut self.bottom.test_output);
            match out.len() >= shown.len() {
                true => shown.push_str(&out[shown.len()..]),
                false => *shown = out.clone(),
            }
        }
        for config in std::mem::take(&mut self.testing.debug) {
            if self.debug.active() {
                ui::notify("A debug session is already running.");
                continue;
            }
            let (debug, mut env) = self.debug_env(ctx);
            debug.begin(config, false, &mut env);
        }
    }

    pub(crate) fn testing_view(&mut self, ui: &mut Ui) {
        let theme = self.theme.clone();
        self.testing.ui(ui, &theme, &mut self.cmds);
    }

    pub(crate) fn testing_exec(&mut self, cmd: &Command, ctx: &egui::Context) {
        use Command::*;
        self.testing.ensure(ctx);
        let cursor = || Some((self.editors.active_path()?, self.editors.status()?.line.saturating_sub(1)));
        let at_cursor = cursor().and_then(|(path, line)| self.testing.at(&path, line).map(|t| (t.kind, t.id.clone())));
        let action = match cmd {
            RunAllTests | RunAllTestsWithCoverage => Some(Action::Run(self.testing.all_jobs(*cmd == RunAllTestsWithCoverage))),
            RunFailedTests if self.testing.failed().is_empty() => return ui::notify("No tests failed in the last run."),
            RunFailedTests => Some(Action::Run(self.testing.jobs_for(&self.testing.failed(), false))),
            RerunLastTestRun if self.testing.last.is_empty() => return ui::notify("No tests have been run yet."),
            RerunLastTestRun => Some(Action::Run(self.testing.last.clone())),
            RunTestAtCursor | DebugTestAtCursor => {
                let Some((kind, id)) = at_cursor else { return ui::notify("No test was found at the cursor.") };
                Some(if *cmd == RunTestAtCursor { Action::Run(self.testing.jobs_for(&[(kind, id)], false)) } else { Action::Debug(kind, id) })
            }
            CancelTestRun => {
                self.testing.cancel();
                None
            }
            RefreshTests => {
                self.testing.refresh(ctx);
                None
            }
            ShowTestOutput => {
                self.session.panel = true;
                self.bottom.show_tests();
                None
            }
            ToggleCoverageDisplay => {
                self.testing.show_coverage ^= true;
                self.testing.dirty = true;
                None
            }
            _ => None,
        };
        if let Some(action) = action {
            self.testing_act(action, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Test> {
        let mut tests = vec![
            Test::new(Kind::Rust, "tests::adds", "adds", "src/lib.rs", Some((PathBuf::from("/w/src/lib.rs"), 5))),
            Test::new(Kind::Rust, "tests::fails", "fails", "src/lib.rs", Some((PathBuf::from("/w/src/lib.rs"), 9))),
            Test::new(Kind::Rust, "outer", "outer", "tests/it.rs", Some((PathBuf::from("/w/tests/it.rs"), 1))),
            Test::new(Kind::Python, "tests/test_a.py::test_one", "test_one", "tests/test_a.py", None),
        ];
        tests[1].state = State::Failed;
        tests
    }

    #[test]
    fn builds_and_filters_the_tree() {
        let tests = sample();
        let built = tree(&tests, "");
        assert_eq!(built.len(), 2);
        assert_eq!(built[0].0, Kind::Rust);
        assert_eq!(built[0].1, [("src/lib.rs".to_owned(), vec![0, 1]), ("tests/it.rs".to_owned(), vec![2])]);
        assert_eq!(built[1].1, [("tests/test_a.py".to_owned(), vec![3])]);
        let filtered = tree(&tests, "  FAIL ");
        assert_eq!(filtered, vec![(Kind::Rust, vec![("src/lib.rs".to_owned(), vec![1])])]);
        assert_eq!(tree(&tests, "it.rs")[0].1[0].1, [2]);
        assert_eq!(tree(&tests, "@failed")[0].1[0].1, [1]);
        assert!(tree(&tests, "nothing").is_empty());
        assert_eq!(summary(tests.iter().map(|t| t.state)), State::Failed);
        assert_eq!(summary([State::Passed, State::Skipped, State::NotRun].into_iter()), State::Passed);
        assert_eq!(summary([State::Failed, State::Running].into_iter()), State::Running);
        assert_eq!(summary(std::iter::empty()), State::NotRun);
    }

    #[test]
    fn moves_tests_through_run_states() {
        let mut tests = sample();
        tests[1].message = "old".into();
        let job = Job { kind: Kind::Rust, ids: Some(vec!["tests::adds".into(), "tests::fails".into()]), coverage: false };
        begin(&mut tests, &job, State::Queued);
        assert_eq!(tests.iter().map(|t| t.state).collect::<Vec<_>>(), [State::Queued, State::Queued, State::NotRun, State::NotRun]);
        assert!(tests[1].message.is_empty());
        begin(&mut tests, &job, State::Running);
        assert!(apply(&mut tests, Kind::Rust, Outcome { id: "tests::adds".into(), state: State::Passed, duration: Some(0.5), ..Default::default() }, None));
        assert!(!apply(&mut tests, Kind::Python, Outcome { id: "tests::adds".into(), state: State::Failed, ..Default::default() }, None));
        let failure = Outcome { id: "tests::fails".into(), state: State::Failed, message: "boom".into(), at: Some(("/w/src/lib.rs".into(), 11)), ..Default::default() };
        apply(&mut tests, Kind::Rust, failure, Some(Path::new("/w")));
        assert_eq!((tests[0].state, tests[0].duration), (State::Passed, Some(0.5)));
        assert_eq!((tests[1].state, tests[1].message.as_str()), (State::Failed, "boom"));
        assert_eq!(tests[1].fail.as_ref().map(|f| f.1), Some(10));

        let all = Job { kind: Kind::Rust, ids: None, coverage: false };
        begin(&mut tests, &all, State::Queued);
        assert!(tests[..3].iter().all(|t| t.state == State::Queued) && tests[1].fail.is_none());
        settle(&mut tests, Kind::Rust);
        assert!(tests[..3].iter().all(|t| t.state == State::NotRun));

        tests[0].state = State::Passed;
        let found = vec![Test::new(Kind::Rust, "tests::adds", "adds", "src/lib.rs", None), Test::new(Kind::Rust, "tests::new", "new", "src/lib.rs", None)];
        merge(&mut tests, Kind::Rust, found);
        assert_eq!(tests.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["tests/test_a.py::test_one", "tests::adds", "tests::new"]);
        assert_eq!(tests[1].state, State::Passed);
    }

    #[test]
    fn each_root_keeps_its_own_tests_while_another_is_active() {
        let mut testing = Testing { tests: sample(), root: Some(PathBuf::from("/w")), loaded: true, ..Default::default() };
        testing.set_root(Some(Path::new("/api")), true);
        assert!(testing.tests.is_empty() && !testing.loaded && testing.parked.len() == 1);
        testing.set_root(Some(Path::new("/w")), true);
        assert_eq!((testing.tests.len(), testing.loaded, testing.parked.len()), (sample().len(), true, 1));
        testing.set_root(Some(Path::new("/other")), false);
        assert!(testing.tests.is_empty() && testing.parked.is_empty());
    }

    #[test]
    fn finds_tests_at_the_cursor_and_follows_edits() {
        let mut testing = Testing { tests: sample(), root: Some(PathBuf::from("/w")), ..Default::default() };
        let lib = Path::new("/w/src/lib.rs");
        assert_eq!(testing.at(lib, 7).map(|t| t.id.as_str()), Some("tests::adds"));
        assert_eq!(testing.at(lib, 12).map(|t| t.id.as_str()), Some("tests::fails"));
        assert!(testing.at(lib, 2).is_none());
        let jobs = testing.jobs_for(&[(Kind::Rust, "a".into()), (Kind::Python, "p".into()), (Kind::Rust, "b".into())], true);
        assert_eq!(jobs, [Job { kind: Kind::Rust, ids: Some(vec!["a".into(), "b".into()]), coverage: true }, Job { kind: Kind::Python, ids: Some(vec!["p".into()]), coverage: true }]);
        assert_eq!(testing.all_jobs(false).len(), 2);

        testing.tests[1].fail = Some((lib.to_path_buf(), 10));
        testing.coverage.insert(key(lib), (lib.to_path_buf(), Hits::from([(1, 1)])));
        let mut marks = Marks::default();
        testing.sync(&mut marks);
        let file = marks.of(lib).unwrap();
        assert_eq!(file.tests.iter().map(|t| t.0).collect::<Vec<_>>(), [5, 9]);
        assert_eq!(file.failures, [(10, "Test failed.".to_owned())]);
        assert!(file.coverage.is_some());

        marks.edited(lib, 6, 2);
        assert_eq!(marks.of(lib).unwrap().tests.iter().map(|t| t.0).collect::<Vec<_>>(), [5, 11]);
        assert!(marks.of(lib).unwrap().coverage.is_none());
        for (path, line, delta) in std::mem::take(&mut marks.edited) {
            testing.edited(&path, line, delta);
        }
        assert_eq!((testing.tests[1].line, testing.tests[1].fail.clone()), (Some(11), None));
        assert!(testing.coverage.is_empty());
        testing.show_coverage = false;
        testing.sync(&mut marks);
        assert_eq!(marks.of(lib).unwrap().tests[1].0, 11);
    }

    #[test]
    fn view_runs_toolbar_commands_and_opens_tests() {
        let mut testing = Testing { tests: sample(), root: Some(PathBuf::from("/w")), loaded: true, filter: "fails".into(), ..Default::default() };
        testing.tests[1].message = "assertion failed".into();
        let ctx = egui::Context::default();
        let frame = |testing: &mut Testing, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 500.0))), events, focused: true, ..Default::default() };
            let mut cmds = vec![];
            let mut out = ctx.run_ui(input, |ui| testing.ui(ui, &Theme::dark(), &mut cmds));
            out.textures_delta.clear();
            cmds
        };
        let click = |testing: &mut Testing, x: f32, y: f32| {
            let (pos, button) = (pos2(x, y), |pressed| egui::Event::PointerButton { pos: pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
            frame(testing, vec![egui::Event::PointerMoved(pos)]);
            frame(testing, vec![button(true)]);
            frame(testing, vec![button(false)])
        };
        frame(&mut testing, vec![]);
        let toolbar = (20..70).step_by(4).map(|y| click(&mut testing, 20.0, y as f32)).find(|c| !c.is_empty());
        assert_eq!(toolbar, Some(vec![Command::RunAllTests]));
        let opened = (60..400).rev().step_by(5).map(|y| click(&mut testing, 150.0, y as f32)).find(|c| !c.is_empty());
        assert_eq!(opened, Some(vec![Command::OpenPath("/w/src/lib.rs".into()), Command::GoTo(10, 1)]));
        assert_eq!(testing.selected, Some((Kind::Rust, "tests::fails".into())));
        assert!(testing.actions.is_empty());
    }

    #[test]
    #[ignore]
    fn runs_cargo_test_on_a_temporary_crate() {
        let root = std::env::temp_dir().join(format!("halo-testing-crate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() { assert_eq!(1 + 1, 2); }\n    #[test]\n    fn fails() { assert_eq!(1 + 1, 3); }\n}\n").unwrap();
        let ctx = egui::Context::default();
        let mut testing = Testing::default();
        testing.set_root(Some(&root), false);
        testing.refresh(&ctx);
        let wait = |testing: &mut Testing, done: &dyn Fn(&Testing) -> bool| {
            let start = std::time::Instant::now();
            while !done(testing) && start.elapsed() < std::time::Duration::from_secs(300) {
                testing.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        };
        wait(&mut testing, &|t| !t.listing());
        assert_eq!(testing.kinds, [Kind::Rust]);
        assert_eq!(testing.tests.iter().map(|t| (t.id.as_str(), t.line)).collect::<Vec<_>>(), [("tests::adds", Some(3)), ("tests::fails", Some(5))]);
        testing.start(testing.all_jobs(false), &ctx);
        wait(&mut testing, &|t| !t.busy());
        assert_eq!(testing.tests.iter().map(|t| t.state).collect::<Vec<_>>(), [State::Passed, State::Failed]);
        let failed = &testing.tests[1];
        assert!(failed.message.contains("left == right"), "{}", failed.message);
        assert_eq!(failed.fail.as_ref().map(|f| (key(&f.0), f.1)), Some((key(&root.join("src").join("lib.rs")), 5)));
        assert!(testing.output.contains("test tests::fails ... FAILED"));
        let config = debug_build(&root, &testing.tests[0], &Shared::default()).unwrap();
        assert_eq!(config.raw["args"], serde_json::json!(["tests::adds", "--exact"]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
