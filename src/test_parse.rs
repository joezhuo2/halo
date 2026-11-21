use crate::debug::Config;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    NotRun,
    Queued,
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Kind {
    Rust,
    Python,
    Go,
    Jest,
    Vitest,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Rust => "Rust",
            Kind::Python => "Python (pytest)",
            Kind::Go => "Go",
            Kind::Jest => "Jest",
            Kind::Vitest => "Vitest",
        }
    }

    pub fn program(self) -> &'static str {
        match self {
            Kind::Rust => "cargo",
            Kind::Python if cfg!(windows) => "python",
            Kind::Python => "python3",
            Kind::Go => "go",
            Kind::Jest | Kind::Vitest if cfg!(windows) => "npx.cmd",
            Kind::Jest | Kind::Vitest => "npx",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Test {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    pub group: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub state: State,
    pub duration: Option<f64>,
    pub message: String,
    pub output: String,
    pub fail: Option<(PathBuf, usize)>,
}

impl Test {
    pub fn new(kind: Kind, id: &str, name: &str, group: &str, place: Option<(PathBuf, usize)>) -> Self {
        let (file, line) = place.map_or((None, None), |(f, l)| (Some(f), Some(l)));
        Test { kind, id: id.into(), name: name.into(), group: group.into(), file, line, state: State::NotRun, duration: None, message: String::new(), output: String::new(), fail: None }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outcome {
    pub id: String,
    pub state: State,
    pub duration: Option<f64>,
    pub message: String,
    pub output: String,
    pub at: Option<(String, usize)>,
}

pub type Hits = BTreeMap<usize, u64>;

pub fn detect(root: &Path) -> Vec<Kind> {
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let mut kinds = vec![];
    if root.join("Cargo.toml").is_file() {
        kinds.push(Kind::Rust);
    }
    if ["pytest.ini", "conftest.py"].iter().any(|f| root.join(f).is_file()) || ["pyproject.toml", "setup.cfg", "tox.ini"].iter().any(|f| read(f).contains("pytest")) {
        kinds.push(Kind::Python);
    }
    if root.join("go.mod").is_file() {
        kinds.push(Kind::Go);
    }
    let package = read("package.json");
    if package.contains("\"vitest\"") {
        kinds.push(Kind::Vitest);
    } else if package.contains("\"jest\"") {
        kinds.push(Kind::Jest);
    }
    kinds
}

pub fn list_args(kind: Kind) -> Vec<String> {
    let args: &[&str] = match kind {
        Kind::Rust => &["test", "--", "--list", "--format", "terse"],
        Kind::Python => &["-m", "pytest", "--collect-only", "-q"],
        Kind::Go => &["test", "-list", ".", "./..."],
        Kind::Jest => &["jest", "--listTests", "--json"],
        Kind::Vitest => &["vitest", "list", "--json"],
    };
    args.iter().map(|a| a.to_string()).collect()
}

fn pattern(names: &[String]) -> String {
    let escape = |name: &String| name.chars().flat_map(|c| if r"\^$.|?*+()[]{}/".contains(c) { vec!['\\', c] } else { vec![c] }).collect::<String>();
    format!("^(?:{})$", names.iter().map(escape).collect::<Vec<_>>().join("|"))
}

pub fn run_args(kind: Kind, tests: Option<&[&Test]>, report: &Path, cover: Option<&Path>) -> Vec<String> {
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let unique = |items: Vec<String>| items.into_iter().fold(vec![], |mut acc: Vec<String>, item| {
        if !acc.contains(&item) {
            acc.push(item);
        }
        acc
    });
    let strs = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let ids = |tests: &[&Test]| tests.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let heads = |tests: &[&Test]| unique(tests.iter().map(|t| t.id.split("::").next().unwrap_or_default().to_owned()).collect());
    let tails = |tests: &[&Test]| unique(tests.iter().map(|t| t.id.rsplit_once("::").map_or(t.id.as_str(), |p| p.1).to_owned()).collect());
    let mut args: Vec<String> = vec![];
    match kind {
        Kind::Rust => {
            args.extend(match cover {
                Some(cover) => strs(&["llvm-cov", "--no-fail-fast", "--lcov", "--output-path", &path(cover)]),
                None => strs(&["test", "--no-fail-fast"]),
            });
            if let Some(tests) = tests {
                args.extend(strs(&["--", "--exact"]).into_iter().chain(ids(tests)));
            }
        }
        Kind::Python => {
            args.extend(strs(&["-m", "pytest", "-v", &format!("--junitxml={}", path(report))]));
            if let Some(cover) = cover {
                args.extend(strs(&["--cov", &format!("--cov-report=lcov:{}", path(cover))]));
            }
            args.extend(tests.map(ids).unwrap_or_default());
        }
        Kind::Go => {
            args.extend(strs(&["test", "-json"]));
            if let Some(cover) = cover {
                args.push(format!("-coverprofile={}", path(cover)));
            }
            match tests {
                Some(tests) => args.extend(strs(&["-run", &pattern(&tails(tests))]).into_iter().chain(heads(tests))),
                None => args.push("./...".into()),
            }
        }
        Kind::Jest | Kind::Vitest => {
            let jest = kind == Kind::Jest;
            args.extend(match jest {
                true => strs(&["jest", "--json", &format!("--outputFile={}", path(report))]),
                false => strs(&["vitest", "run", "--reporter=json", &format!("--outputFile={}", path(report))]),
            });
            args.extend(match (cover, jest) {
                (Some(dir), true) => strs(&["--coverage", "--coverageReporters=lcov", &format!("--coverageDirectory={}", path(dir))]),
                (Some(dir), false) => strs(&["--coverage.enabled=true", "--coverage.reporter=lcov", &format!("--coverage.reportsDirectory={}", path(dir))]),
                (None, _) => vec![],
            });
            if let Some(tests) = tests {
                if jest {
                    args.push("--runTestsByPath".into());
                }
                let names = unique(tests.iter().map(|t| t.id.split_once("::").map_or("", |p| p.1).to_owned()).collect());
                args.extend(heads(tests).into_iter().chain(strs(&["-t", &pattern(&names)])));
            }
        }
    }
    args
}

pub fn coverage_check(kind: Kind, root: &Path) -> Option<(Vec<&'static str>, &'static str)> {
    match kind {
        Kind::Rust => Some((vec!["llvm-cov", "--version"], "Run with Coverage needs cargo-llvm-cov. Install it with: cargo install cargo-llvm-cov")),
        Kind::Python => Some((vec!["-c", "import pytest_cov"], "Run with Coverage needs pytest-cov. Install it with: pip install pytest-cov")),
        Kind::Vitest if !std::fs::read_to_string(root.join("package.json")).unwrap_or_default().contains("@vitest/coverage") => {
            Some((vec![], "Run with Coverage needs @vitest/coverage-v8. Install it with: npm install -D @vitest/coverage-v8"))
        }
        _ => None,
    }
}

pub fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

pub fn sources(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let (mut out, mut stack) = (vec![], vec![root.to_path_buf()]);
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && !["target", "node_modules", "__pycache__", "venv", "dist", "build"].contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|e| exts.iter().any(|x| e == *x)) && out.len() < 20_000 {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn find_line(text: &str, needles: &[String]) -> Option<usize> {
    text.lines().position(|line| needles.iter().any(|n| line.contains(n.as_str())))
}

pub fn rust_list(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines().filter_map(|l| l.trim().strip_suffix(": test")).filter(|n| !n.contains(" - ") && seen.insert(n.to_string())).map(str::to_owned).collect()
}

pub fn rust_tests(root: &Path, ids: &[String], files: &[(PathBuf, String)]) -> Vec<Test> {
    ids.iter()
        .map(|id| {
            let (module, leaf) = id.rsplit_once("::").unwrap_or(("", id));
            let hints: Vec<&str> = module.split("::").filter(|s| !s.is_empty()).collect();
            let needles = [format!("fn {leaf}("), format!("fn {leaf}<")];
            let mut best: Option<(usize, &PathBuf, usize)> = None;
            for (path, text) in files {
                if let Some(line) = find_line(text, &needles) {
                    let shown = rel(root, path);
                    let parts: Vec<&str> = shown.trim_end_matches(".rs").split('/').collect();
                    let matched = hints.iter().filter(|h| parts[1..].contains(h)).count();
                    let score = matched * 2 + ((parts[0] == "tests") == hints.is_empty()) as usize;
                    if best.is_none_or(|b| score > b.0) {
                        best = Some((score, path, line));
                    }
                }
            }
            let place = best.map(|b| (b.1.clone(), b.2));
            let group = place.as_ref().map_or_else(|| if module.is_empty() { "(crate)".to_owned() } else { module.to_owned() }, |p| rel(root, &p.0));
            Test::new(Kind::Rust, id, leaf, &group, place)
        })
        .collect()
}

pub fn libtest_line(line: &str) -> Option<Outcome> {
    let (name, status) = line.strip_prefix("test ")?.rsplit_once(" ... ")?;
    let (status, duration) = match status.split_once(" <") {
        Some((s, t)) => (s, t.trim_end().strip_suffix("s>").and_then(|t| t.parse().ok())),
        None => (status, None),
    };
    let state = match status.trim() {
        "ok" => State::Passed,
        "FAILED" => State::Failed,
        s if s.starts_with("ignored") => State::Skipped,
        _ => return None,
    };
    Some(Outcome { id: name.to_owned(), state, duration, ..Default::default() })
}

fn place(text: &str) -> Option<(String, usize)> {
    let mut parts = text.trim().rsplitn(3, ':');
    let (last, middle, first) = (parts.next()?, parts.next()?, parts.next());
    match first {
        Some(file) if middle.parse::<usize>().is_ok() && last.parse::<usize>().is_ok() => Some((file.to_owned(), middle.parse().ok()?)),
        _ => Some((middle.to_owned(), last.parse().ok()?)),
    }
}

pub fn libtest_failures(text: &str) -> Vec<Outcome> {
    let (mut out, mut current): (Vec<Outcome>, Option<Outcome>) = (vec![], None);
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("---- ").and_then(|l| l.strip_suffix(" stdout ----")) {
            out.extend(current.replace(Outcome { id: name.to_owned(), state: State::Failed, ..Default::default() }));
        } else if line == "failures:" || line.starts_with("test result:") {
            out.extend(current.take());
        } else if let Some(o) = current.as_mut() {
            o.output.push_str(line);
            o.output.push('\n');
        }
    }
    out.extend(current);
    for o in &mut out {
        o.output = o.output.trim().to_owned();
        let lines: Vec<&str> = o.output.lines().collect();
        let Some(index) = lines.iter().position(|l| l.contains("panicked at ")) else { continue };
        let after = &lines[index][lines[index].find("panicked at ").unwrap_or(0) + 12..];
        match after.strip_prefix('\'').and_then(|a| a.rsplit_once("', ")) {
            Some((message, at)) => (o.message, o.at) = (message.to_owned(), place(at)),
            None => {
                o.at = place(after.trim_end_matches(':'));
                o.message = lines[index + 1..].iter().take_while(|l| !l.is_empty() && !l.starts_with("note:")).copied().collect::<Vec<_>>().join("\n");
            }
        }
    }
    out
}

pub fn executables(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["reason"] == "compiler-artifact" && v["profile"]["test"] == true)
        .filter_map(|v| v["executable"].as_str().map(PathBuf::from))
        .collect()
}

pub fn python_list(text: &str) -> Vec<String> {
    text.lines().map(str::trim_end).filter(|l| l.contains("::") && !l.starts_with(char::is_whitespace) && !l.contains(' ')).map(str::to_owned).collect()
}

fn leaf_of(id: &str) -> &str {
    let leaf = id.rsplit("::").next().unwrap_or(id);
    leaf.split('[').next().unwrap_or(leaf)
}

pub fn python_tests(root: &Path, ids: &[String]) -> Vec<Test> {
    let mut texts: HashMap<String, String> = HashMap::new();
    ids.iter()
        .map(|id| {
            let (file, name) = id.split_once("::").unwrap_or((id, id));
            let text = texts.entry(file.to_owned()).or_insert_with(|| std::fs::read_to_string(root.join(file)).unwrap_or_default());
            let line = find_line(text, &[format!("def {}(", leaf_of(id))]);
            Test::new(Kind::Python, id, name, file, line.map(|l| (root.join(file), l)))
        })
        .collect()
}

pub fn pytest_line(line: &str) -> Option<Outcome> {
    let (id, rest) = line.split_once(' ')?;
    if !id.contains("::") {
        return None;
    }
    let state = match rest.split_whitespace().next()? {
        "PASSED" | "XPASS" => State::Passed,
        "FAILED" | "ERROR" => State::Failed,
        "SKIPPED" | "XFAIL" => State::Skipped,
        _ => return None,
    };
    Some(Outcome { id: id.to_owned(), state, ..Default::default() })
}

pub fn junit_key(node: &str) -> String {
    let parts: Vec<&str> = node.split("::").collect();
    let module = parts[0].trim_end_matches(".py").replace(['/', '\\'], ".");
    let (name, classes) = parts[1..].split_last().unwrap_or((&"", &[]));
    format!("{}::{name}", std::iter::once(module.as_str()).chain(classes.iter().copied()).collect::<Vec<_>>().join("."))
}

fn unescape(text: &str) -> String {
    let mut out = text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#10;", "\n").replace("&#13;", "");
    while let Some(at) = out.find("&#") {
        let Some(end) = out[at..].find(';').map(|e| at + e) else { break };
        let code = out[at + 2..end].parse::<u32>().ok().and_then(char::from_u32).unwrap_or('?');
        out.replace_range(at..=end, &code.to_string());
    }
    out.replace("&amp;", "&")
}

fn attrs(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].rsplit(char::is_whitespace).next().unwrap_or_default().to_owned();
        let Some(end) = rest[eq + 2..].find('"').map(|e| eq + 2 + e) else { break };
        out.insert(key, unescape(&rest[eq + 2..end]));
        rest = &rest[end + 1..];
    }
    out
}

pub fn junit(xml: &str) -> Vec<Outcome> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(start) = rest.find("<testcase") {
        rest = &rest[start..];
        let Some(close) = rest.find('>') else { break };
        let head = &rest[..close];
        let a = attrs(head);
        let body = match head.ends_with('/') {
            true => "",
            false => &rest[close + 1..rest.find("</testcase>").unwrap_or(rest.len())],
        };
        rest = &rest[close + 1 + body.len()..];
        let mut outcome = Outcome {
            id: format!("{}::{}", a.get("classname").map_or("", |s| s), a.get("name").map_or("", |s| s)),
            state: State::Passed,
            duration: a.get("time").and_then(|t| t.parse().ok()),
            ..Default::default()
        };
        for (tag, state) in [("<failure", State::Failed), ("<error", State::Failed), ("<skipped", State::Skipped)] {
            let Some(at) = body.find(tag) else { continue };
            let child = &body[at..];
            let end = child.find('>').unwrap_or(child.len());
            outcome.state = state;
            outcome.message = attrs(&child[..end]).remove("message").unwrap_or_default();
            if !child[..end].ends_with('/') {
                let name = &tag[1..];
                let text = &child[(end + 1).min(child.len())..child.find(&format!("</{name}>")).unwrap_or(child.len())];
                outcome.output = unescape(text).trim().to_owned();
            }
            outcome.at = outcome.output.lines().rev().find_map(|l| {
                let (file, rest) = l.split_once(".py:")?;
                Some((format!("{file}.py"), rest.split(':').next()?.parse().ok()?))
            });
            break;
        }
        out.push(outcome);
    }
    out
}

pub fn go_module(root: &Path) -> String {
    let text = std::fs::read_to_string(root.join("go.mod")).unwrap_or_default();
    text.lines().find_map(|l| l.trim().strip_prefix("module ")).map(|m| m.trim().trim_matches('"').to_owned()).unwrap_or_default()
}

pub fn go_dir(root: &Path, module: &str, package: &str) -> PathBuf {
    match package.strip_prefix(module) {
        Some(rest) if !module.is_empty() => root.join(rest.trim_start_matches('/')),
        _ => root.to_path_buf(),
    }
}

pub fn go_list(text: &str) -> Vec<(String, String)> {
    let (mut pending, mut out) = (vec![], vec![]);
    for line in text.lines().map(str::trim_end) {
        if let Some(rest) = line.strip_prefix("ok ") {
            let package = rest.split_whitespace().next().unwrap_or_default();
            out.extend(pending.drain(..).map(|name: String| (package.to_owned(), name)));
        } else if !line.is_empty() && !line.contains(char::is_whitespace) {
            if ["Test", "Example", "Fuzz"].iter().any(|p| line.starts_with(p)) {
                pending.push(line.to_owned());
            }
        } else {
            pending.clear();
        }
    }
    out
}

pub fn go_tests(root: &Path, found: &[(String, String)]) -> Vec<Test> {
    let module = go_module(root);
    let mut texts: HashMap<PathBuf, Vec<(PathBuf, String)>> = HashMap::new();
    found
        .iter()
        .map(|(package, name)| {
            let dir = go_dir(root, &module, package);
            let files = texts.entry(dir.clone()).or_insert_with(|| {
                let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path());
                entries.filter(|p| p.extension().is_some_and(|e| e == "go")).map(|p| (p.clone(), std::fs::read_to_string(&p).unwrap_or_default())).collect()
            });
            let needle = [format!("func {name}(")];
            let place = files.iter().find_map(|(path, text)| Some((path.clone(), find_line(text, &needle)?)));
            Test::new(Kind::Go, &format!("{package}::{name}"), name, package, place)
        })
        .collect()
}

#[derive(Default)]
pub struct GoRun {
    outputs: HashMap<String, String>,
}

impl GoRun {
    pub fn feed(&mut self, line: &str) -> Option<Outcome> {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        let (package, test, action) = (v["Package"].as_str()?, v["Test"].as_str()?, v["Action"].as_str()?);
        let id = format!("{package}::{}", test.split('/').next().unwrap_or(test));
        if action == "output" {
            self.outputs.entry(id).or_default().push_str(v["Output"].as_str().unwrap_or_default());
            return None;
        }
        if test.contains('/') {
            return None;
        }
        let state = match action {
            "run" => return Some(Outcome { id, state: State::Running, ..Default::default() }),
            "pass" => State::Passed,
            "fail" => State::Failed,
            "skip" => State::Skipped,
            _ => return None,
        };
        let output = self.outputs.remove(&id).unwrap_or_default().trim_end().to_owned();
        let found = output.lines().find_map(|l| {
            let (at, message) = l.trim().split_once(": ")?;
            let (file, line) = at.rsplit_once(':')?;
            file.ends_with(".go").then_some(((file.to_owned(), line.parse().ok()?), message.to_owned()))
        });
        let (at, message) = found.map_or((None, String::new()), |(a, m)| (Some(a), m));
        let message = if state == State::Failed && message.is_empty() { "Test failed.".to_owned() } else { message };
        Some(Outcome { id, state, duration: v["Elapsed"].as_f64(), message, output, at })
    }
}

pub fn js_scan(text: &str) -> Vec<(Vec<String>, usize)> {
    let mut stack: Vec<(usize, String)> = vec![];
    let mut out: Vec<(Vec<String>, usize)> = vec![];
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let Some(open) = trimmed.find('(') else { continue };
        let head = trimmed[..open].split('.').next().unwrap_or_default();
        if !["describe", "it", "test"].contains(&head) || !trimmed[..open].chars().all(|c| c.is_alphanumeric() || c == '.') {
            continue;
        }
        let args = trimmed[open + 1..].trim_start();
        let Some(quote) = args.chars().next().filter(|c| "'\"`".contains(*c)) else { continue };
        let Some(end) = args[1..].find(quote) else { continue };
        let title = args[1..1 + end].to_owned();
        while stack.last().is_some_and(|s| s.0 >= indent) {
            stack.pop();
        }
        match head {
            "describe" => stack.push((indent, title)),
            _ => out.push((stack.iter().map(|s| s.1.clone()).chain(std::iter::once(title)).collect(), index)),
        }
    }
    out
}

fn json_start(text: &str, open: char) -> Option<Value> {
    serde_json::from_str(&text[text.find(open)?..]).ok()
}

pub fn jest_list(text: &str) -> Vec<PathBuf> {
    json_start(text, '[').and_then(|v| v.as_array().cloned()).into_iter().flatten().filter_map(|v| v.as_str().map(PathBuf::from)).collect()
}

pub fn js_tests(kind: Kind, root: &Path, files: &[PathBuf]) -> Vec<Test> {
    let mut out = vec![];
    for file in files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let group = rel(root, file);
        for (titles, line) in js_scan(&text) {
            let id = format!("{group}::{}", titles.join(" "));
            out.push(Test::new(kind, &id, &titles.join(" › "), &group, Some((file.clone(), line))));
        }
    }
    out
}

pub fn vitest_list(root: &Path, text: &str) -> Vec<Test> {
    let mut texts: HashMap<PathBuf, String> = HashMap::new();
    let items = json_start(text, '[').and_then(|v| v.as_array().cloned()).unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let (name, file) = (item["name"].as_str()?, PathBuf::from(item["file"].as_str()?));
            let file = if file.is_absolute() { file } else { root.join(file) };
            let titles: Vec<&str> = name.split(" > ").collect();
            let group = rel(root, &file);
            let text = texts.entry(file.clone()).or_insert_with(|| std::fs::read_to_string(&file).unwrap_or_default());
            let leaf = titles.last().copied().unwrap_or_default();
            let line = ["'", "\"", "`"].iter().find_map(|q| find_line(text, &[format!("{q}{leaf}{q}")]));
            Some(Test::new(Kind::Vitest, &format!("{group}::{}", titles.join(" ")), &titles.join(" › "), &group, line.map(|l| (file.clone(), l))))
        })
        .collect()
}

pub fn js_results(root: &Path, text: &str) -> Vec<Outcome> {
    let Some(report) = json_start(text, '{') else { return vec![] };
    let mut out = vec![];
    for file in report["testResults"].as_array().into_iter().flatten() {
        let path = file["name"].as_str().unwrap_or_default();
        let group = rel(root, Path::new(path));
        for result in file["assertionResults"].as_array().into_iter().flatten() {
            let titles: Vec<&str> = result["ancestorTitles"].as_array().into_iter().flatten().filter_map(Value::as_str).chain(result["title"].as_str()).collect();
            let state = match result["status"].as_str().unwrap_or_default() {
                "passed" => State::Passed,
                "failed" => State::Failed,
                _ => State::Skipped,
            };
            let output = result["failureMessages"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
            let at = output.find(&format!("{path}:")).and_then(|at| {
                let rest = &output[at + path.len() + 1..];
                Some((path.to_owned(), rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?))
            });
            let message = output.lines().next().unwrap_or_default().to_owned();
            let duration = result["duration"].as_f64().map(|ms| ms / 1000.0);
            out.push(Outcome { id: format!("{group}::{}", titles.join(" ")), state, duration, message, output, at });
        }
    }
    out
}

pub fn lcov(text: &str) -> Vec<(String, Hits)> {
    let mut out: Vec<(String, Hits)> = vec![];
    let mut current: Option<(String, Hits)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(file) = line.strip_prefix("SF:") {
            out.extend(current.replace((file.to_owned(), Hits::new())));
        } else if let (Some(da), Some((_, hits))) = (line.strip_prefix("DA:"), current.as_mut()) {
            let mut parts = da.split(',');
            if let (Some(Ok(number)), Some(Ok(count))) = (parts.next().map(str::parse::<usize>), parts.next().map(str::parse::<u64>)) {
                let entry = hits.entry(number.saturating_sub(1)).or_default();
                *entry = (*entry).max(count);
            }
        } else if line == "end_of_record" {
            out.extend(current.take());
        }
    }
    out.extend(current);
    out
}

pub fn cover_profile(text: &str) -> Vec<(String, Hits)> {
    let mut files: BTreeMap<String, Hits> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with("mode:")) {
        let mut parts = line.trim().rsplitn(3, ' ');
        let (Some(count), Some(_), Some(span)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some((file, range)) = span.rsplit_once(':') else { continue };
        let Some((from, to)) = range.split_once(',') else { continue };
        let line_of = |p: &str| p.split('.').next().and_then(|n| n.parse::<usize>().ok());
        let (Some(from), Some(to), Ok(count)) = (line_of(from), line_of(to), count.parse::<u64>()) else { continue };
        let hits = files.entry(file.to_owned()).or_default();
        for number in from..=to {
            let entry = hits.entry(number.saturating_sub(1)).or_default();
            *entry = (*entry).max(count);
        }
    }
    files.into_iter().collect()
}

pub fn debug_config(test: &Test, root: &Path, exe: Option<&Path>) -> Result<Config, String> {
    let name = format!("Debug Test: {}", test.name);
    let cwd = root.to_string_lossy();
    let (kind, raw) = match test.kind {
        Kind::Rust => {
            let exe = exe.ok_or("The test executable was not found in the cargo build output.")?;
            ("lldb", json!({"program": exe.to_string_lossy(), "args": [test.id, "--exact"], "cwd": cwd}))
        }
        Kind::Python => ("debugpy", json!({"module": "pytest", "args": [test.id], "cwd": cwd, "console": "integratedTerminal"})),
        Kind::Go => {
            let dir = test.file.as_deref().and_then(Path::parent).unwrap_or(root);
            let leaf = test.id.rsplit("::").next().unwrap_or_default();
            ("go", json!({"mode": "test", "program": dir.to_string_lossy(), "args": ["-test.run", format!("^{leaf}$")]}))
        }
        Kind::Jest | Kind::Vitest => return Err("Debugging JavaScript and TypeScript tests is not supported yet.".into()),
    };
    let mut raw = raw.as_object().cloned().unwrap_or_default();
    for (key, value) in [("type", kind), ("request", "launch"), ("name", name.as_str())] {
        raw.insert(key.into(), json!(value));
    }
    Ok(Config { name, kind: kind.into(), request: "launch".into(), pre_launch_task: None, raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-test-parse-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rust_lists_runs_and_failures() {
        let list = "tests::adds: test\ntests::fails: test\ntests::skipped: test\nouter: test\nsrc/lib.rs - add (line 3): test\nouter: test\nbench_x: benchmark\n";
        assert_eq!(rust_list(list), ["tests::adds", "tests::fails", "tests::skipped", "outer"]);

        let root = temp("rust");
        let lib = "pub fn add() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {}\n    #[test]\n    fn fails() {}\n}\n";
        let files = vec![(root.join("src/lib.rs"), lib.to_owned()), (root.join("tests/it.rs"), "#[test]\nfn outer() {}\nfn adds() {}\n".to_owned())];
        let tests = rust_tests(&root, &rust_list(list), &files);
        assert_eq!((tests[0].name.as_str(), tests[0].group.as_str(), tests[0].line), ("adds", "src/lib.rs", Some(5)));
        assert_eq!(tests[2].group, "tests");
        assert_eq!((tests[3].group.as_str(), tests[3].line), ("tests/it.rs", Some(1)));

        let output = "running 3 tests\ntest tests::adds ... ok\ntest tests::fails ... FAILED\ntest tests::skipped ... ignored\ntest tests::slow ... ignored, needs network\ntest tests::timed ... ok <0.250s>\ntest result: FAILED. 1 passed; 1 failed;\n";
        let lines: Vec<Outcome> = output.lines().filter_map(libtest_line).collect();
        let states: Vec<(&str, State)> = lines.iter().map(|o| (o.id.as_str(), o.state)).collect();
        assert_eq!(states, [("tests::adds", State::Passed), ("tests::fails", State::Failed), ("tests::skipped", State::Skipped), ("tests::slow", State::Skipped), ("tests::timed", State::Passed)]);
        assert_eq!(lines[4].duration, Some(0.25));

        let failures = "failures:\n\n---- tests::fails stdout ----\nhello out\n\nthread 'tests::fails' (24012) panicked at src\\lib.rs:10:9:\nassertion `left == right` failed\n  left: 2\n right: 3\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n\n---- tests::old stdout ----\nthread 'tests::old' panicked at 'boom', src/old.rs:4:5\n\n\nfailures:\n    tests::fails\n    tests::old\n\ntest result: FAILED. 1 passed; 2 failed; 0 ignored\n";
        let found = libtest_failures(failures);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "tests::fails");
        assert_eq!(found[0].message, "assertion `left == right` failed\n  left: 2\n right: 3");
        assert_eq!(found[0].at, Some(("src\\lib.rs".into(), 10)));
        assert!(found[0].output.starts_with("hello out") && found[0].output.ends_with("backtrace"));
        assert_eq!((found[1].message.as_str(), found[1].at.clone()), ("boom", Some(("src/old.rs".into(), 4))));
        assert_eq!(place(r"C:\x\lib.rs:7:1"), Some((r"C:\x\lib.rs".into(), 7)));

        let json = "{\"reason\":\"compiler-artifact\",\"profile\":{\"test\":false},\"executable\":null}\n{\"reason\":\"compiler-artifact\",\"profile\":{\"test\":true},\"executable\":\"C:\\\\t\\\\deps\\\\it-75.exe\"}\n{\"reason\":\"compiler-artifact\",\"profile\":{\"test\":true},\"executable\":\"C:\\\\t\\\\deps\\\\tiny-05.exe\"}\n{\"reason\":\"build-finished\",\"success\":true}\n";
        assert_eq!(executables(json), [PathBuf::from(r"C:\t\deps\it-75.exe"), PathBuf::from(r"C:\t\deps\tiny-05.exe")]);
    }

    #[test]
    fn python_collects_and_reads_junit() {
        let collected = "tests/test_a.py::test_one\ntests/test_a.py::TestMath::test_two\ntests/test_b.py::test_param[1-2]\n\n3 tests collected in 0.02s\n";
        let ids = python_list(collected);
        assert_eq!(ids, ["tests/test_a.py::test_one", "tests/test_a.py::TestMath::test_two", "tests/test_b.py::test_param[1-2]"]);
        let root = temp("python");
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("tests/test_a.py"), "def test_one():\n    pass\n\nclass TestMath:\n    def test_two(self):\n        assert 1 == 2\n").unwrap();
        let tests = python_tests(&root, &ids);
        assert_eq!((tests[1].name.as_str(), tests[1].group.as_str(), tests[1].line), ("TestMath::test_two", "tests/test_a.py", Some(4)));
        assert_eq!(tests[2].line, None);

        let live = "tests/test_a.py::test_one PASSED                                  [ 33%]\ntests/test_a.py::TestMath::test_two FAILED                        [ 66%]\ntests/test_b.py::test_param[1-2] SKIPPED (no db)                   [100%]\nFAILED tests/test_a.py::TestMath::test_two - assert 1 == 2\n";
        let states: Vec<(String, State)> = live.lines().filter_map(pytest_line).map(|o| (o.id, o.state)).collect();
        assert_eq!(states.len(), 3);
        assert_eq!(states[1], ("tests/test_a.py::TestMath::test_two".into(), State::Failed));
        assert_eq!(states[2].1, State::Skipped);

        assert_eq!(junit_key("tests/test_a.py::TestMath::test_two"), "tests.test_a.TestMath::test_two");
        assert_eq!(junit_key("tests/test_b.py::test_param[1-2]"), "tests.test_b::test_param[1-2]");
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><testsuites><testsuite name="pytest" errors="0" failures="1" skipped="1" tests="3" time="0.051"><testcase classname="tests.test_a" name="test_one" time="0.001" /><testcase classname="tests.test_a.TestMath" name="test_two" time="0.002"><failure message="assert 1 == 2">self = &lt;tests.test_a.TestMath object&gt;

    def test_two(self):
&gt;       assert 1 == 2
E       assert 1 == 2

tests/test_a.py:6: AssertionError</failure></testcase><testcase classname="tests.test_b" name="test_param[1-2]" time="0.000"><skipped type="pytest.skip" message="no db">tests/test_b.py:3: no db</skipped></testcase></testsuite></testsuites>"#;
        let results = junit(xml);
        assert_eq!(results.len(), 3);
        assert_eq!((results[0].id.as_str(), results[0].state, results[0].duration), ("tests.test_a::test_one", State::Passed, Some(0.001)));
        assert_eq!((results[1].id.as_str(), results[1].state, results[1].message.as_str()), ("tests.test_a.TestMath::test_two", State::Failed, "assert 1 == 2"));
        assert!(results[1].output.contains(">       assert 1 == 2"));
        assert_eq!(results[1].at, Some(("tests/test_a.py".into(), 6)));
        assert_eq!((results[2].state, results[2].message.as_str()), (State::Skipped, "no db"));
        assert_eq!(unescape("a&amp;lt;b &#65;"), "a&lt;b A");
    }

    #[test]
    fn go_lists_and_reads_json_events() {
        let listed = "TestAdd\nTestSub\nExampleAdd\nok  \texample.com/calc\t0.002s\n?   \texample.com/calc/cmd\t[no test files]\nTestDeep\nBenchmarkX\nok  \texample.com/calc/deep\t0.001s\n";
        let found = go_list(listed);
        assert_eq!(found, [("example.com/calc".into(), "TestAdd".into()), ("example.com/calc".into(), "TestSub".into()), ("example.com/calc".into(), "ExampleAdd".into()), ("example.com/calc/deep".into(), "TestDeep".into())]);
        let root = temp("go");
        std::fs::write(root.join("go.mod"), "module example.com/calc\n\ngo 1.22\n").unwrap();
        std::fs::create_dir_all(root.join("deep")).unwrap();
        std::fs::write(root.join("deep/deep_test.go"), "package deep\n\nimport \"testing\"\n\nfunc TestDeep(t *testing.T) {}\n").unwrap();
        assert_eq!(go_dir(&root, "example.com/calc", "example.com/calc/deep"), root.join("deep"));
        let tests = go_tests(&root, &found);
        assert_eq!((tests[3].id.as_str(), tests[3].line, tests[3].file.clone()), ("example.com/calc/deep::TestDeep", Some(4), Some(root.join("deep/deep_test.go"))));

        let events = [
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"start","Package":"example.com/calc"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"run","Package":"example.com/calc","Test":"TestAdd"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"output","Package":"example.com/calc","Test":"TestAdd","Output":"=== RUN   TestAdd\n"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"output","Package":"example.com/calc","Test":"TestAdd","Output":"    calc_test.go:12: expected 3, got 4\n"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"output","Package":"example.com/calc","Test":"TestAdd/neg","Output":"    calc_test.go:20: sub output\n"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"fail","Package":"example.com/calc","Test":"TestAdd/neg","Elapsed":0}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"output","Package":"example.com/calc","Test":"TestAdd","Output":"--- FAIL: TestAdd (0.00s)\n"}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"fail","Package":"example.com/calc","Test":"TestAdd","Elapsed":0.01}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"pass","Package":"example.com/calc","Test":"TestSub","Elapsed":0}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"skip","Package":"example.com/calc","Test":"ExampleAdd","Elapsed":0}"#,
            r#"{"Time":"2026-11-20T10:00:00Z","Action":"fail","Package":"example.com/calc","Elapsed":0.02}"#,
        ];
        let mut run = GoRun::default();
        let outcomes: Vec<Outcome> = events.iter().filter_map(|e| run.feed(e)).collect();
        assert_eq!(outcomes.iter().map(|o| o.state).collect::<Vec<_>>(), [State::Running, State::Failed, State::Passed, State::Skipped]);
        let failed = &outcomes[1];
        assert_eq!((failed.id.as_str(), failed.message.as_str(), failed.duration), ("example.com/calc::TestAdd", "expected 3, got 4", Some(0.01)));
        assert_eq!(failed.at, Some(("calc_test.go".into(), 12)));
        assert!(failed.output.contains("sub output") && failed.output.ends_with("--- FAIL: TestAdd (0.00s)"));
    }

    #[test]
    fn javascript_scans_lists_and_reads_reports() {
        let source = "import { add } from './add';\n\ndescribe('math', () => {\n  it('adds', () => {\n    expect(add(1, 2)).toBe(3);\n  });\n  describe(\"nested\", () => {\n    test.skip(`skips`, () => {});\n  });\n  it('after nested', () => {});\n});\n\ntest('top level', () => {});\nconst x = it.each;\n";
        let scanned = js_scan(source);
        let titles: Vec<String> = scanned.iter().map(|s| s.0.join(" ")).collect();
        assert_eq!(titles, ["math adds", "math nested skips", "math after nested", "top level"]);
        assert_eq!(scanned.iter().map(|s| s.1).collect::<Vec<_>>(), [3, 7, 9, 12]);

        let root = temp("js");
        std::fs::write(root.join("add.test.js"), source).unwrap();
        let file = root.join("add.test.js");
        let listing = format!("npm warn exec\n{}", serde_json::to_string(&[file.to_string_lossy()]).unwrap());
        assert_eq!(jest_list(&listing), std::slice::from_ref(&file));
        let tests = js_tests(Kind::Jest, &root, &jest_list(&listing));
        assert_eq!((tests[1].id.as_str(), tests[1].name.as_str(), tests[1].group.as_str()), ("add.test.js::math nested skips", "math › nested › skips", "add.test.js"));

        let vitest = serde_json::to_string(&json!([{"name": "math > adds", "file": file.to_string_lossy()}, {"name": "top level", "file": "add.test.js"}])).unwrap();
        let listed = vitest_list(&root, &vitest);
        assert_eq!((listed[0].id.as_str(), listed[0].line), ("add.test.js::math adds", Some(3)));
        assert_eq!((listed[1].id.as_str(), listed[1].line), ("add.test.js::top level", Some(12)));

        let abs = file.to_string_lossy().into_owned();
        let report = json!({"numFailedTests": 1, "testResults": [{"name": abs, "status": "failed", "assertionResults": [
            {"ancestorTitles": ["math"], "fullName": "math adds", "title": "adds", "status": "failed", "duration": 4, "failureMessages": [format!("Error: expect(received).toBe(expected)\n\nExpected: 3\nReceived: 4\n    at Object.<anonymous> ({abs}:5:23)")]},
            {"ancestorTitles": ["math", "nested"], "fullName": "math nested skips", "title": "skips", "status": "pending", "duration": null, "failureMessages": []},
            {"ancestorTitles": [], "fullName": "top level", "title": "top level", "status": "passed", "duration": 1, "failureMessages": []}
        ]}]});
        let results = js_results(&root, &format!("noise {report}"));
        assert_eq!(results.len(), 3);
        assert_eq!((results[0].id.as_str(), results[0].state, results[0].duration), ("add.test.js::math adds", State::Failed, Some(0.004)));
        assert_eq!(results[0].message, "Error: expect(received).toBe(expected)");
        assert_eq!(results[0].at, Some((abs.clone(), 5)));
        assert_eq!((results[1].state, results[2].state), (State::Skipped, State::Passed));
    }

    #[test]
    fn reads_lcov_and_go_cover_profiles() {
        let info = "TN:\nSF:C:\\w\\src\\lib.rs\nFN:1,add\nDA:1,3\nDA:2,0\nDA:2,1\nLF:2\nLH:2\nend_of_record\nSF:src/other.rs\nDA:5,0\nend_of_record\n";
        let files = lcov(info);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "C:\\w\\src\\lib.rs");
        assert_eq!(files[0].1, Hits::from([(0, 3), (1, 1)]));
        assert_eq!(files[1], ("src/other.rs".into(), Hits::from([(4, 0)])));
        let profile = "mode: set\nexample.com/calc/calc.go:3.24,5.2 1 1\nexample.com/calc/calc.go:7.30,9.16 2 0\nexample.com/calc/calc.go:9.16,11.3 1 1\nexample.com/calc/deep/d.go:1.1,1.9 1 0\n";
        let files = cover_profile(profile);
        assert_eq!(files[0], ("example.com/calc/calc.go".into(), Hits::from([(2, 1), (3, 1), (4, 1), (6, 0), (7, 0), (8, 1), (9, 1), (10, 1)])));
        assert_eq!(files[1].1, Hits::from([(0, 0)]));
    }

    #[test]
    fn builds_commands_and_debug_configurations() {
        let root = Path::new("/w");
        let report = Path::new("/tmp/r.xml");
        let rust = [Test::new(Kind::Rust, "tests::adds", "adds", "src/lib.rs", None), Test::new(Kind::Rust, "outer", "outer", "tests/it.rs", None)];
        let picked: Vec<&Test> = rust.iter().collect();
        assert_eq!(run_args(Kind::Rust, Some(&picked), report, None), ["test", "--no-fail-fast", "--", "--exact", "tests::adds", "outer"]);
        assert_eq!(run_args(Kind::Rust, None, report, Some(Path::new("/tmp/c.info"))), ["llvm-cov", "--no-fail-fast", "--lcov", "--output-path", "/tmp/c.info"]);
        let go = [Test::new(Kind::Go, "example.com/calc::TestAdd", "TestAdd", "example.com/calc", Some((PathBuf::from("/w/calc_test.go"), 3)))];
        let picked: Vec<&Test> = go.iter().collect();
        assert_eq!(run_args(Kind::Go, Some(&picked), report, None), ["test", "-json", "-run", "^(?:TestAdd)$", "example.com/calc"]);
        assert_eq!(run_args(Kind::Go, None, report, Some(Path::new("c.out"))), ["test", "-json", "-coverprofile=c.out", "./..."]);
        let js = [Test::new(Kind::Jest, "add.test.js::math adds (x)", "", "add.test.js", None)];
        let picked: Vec<&Test> = js.iter().collect();
        let args = run_args(Kind::Jest, Some(&picked), report, None);
        assert_eq!(args[3..], ["--runTestsByPath", "add.test.js", "-t", r"^(?:math adds \(x\))$"]);
        let py = run_args(Kind::Python, None, report, Some(Path::new("c.info")));
        assert_eq!(py, ["-m", "pytest", "-v", "--junitxml=/tmp/r.xml", "--cov", "--cov-report=lcov:c.info"]);

        let config = debug_config(&rust[0], root, Some(Path::new("/w/target/debug/deps/tiny-05"))).unwrap();
        assert_eq!((config.kind.as_str(), config.request.as_str()), ("lldb", "launch"));
        assert_eq!(config.raw["args"], json!(["tests::adds", "--exact"]));
        assert_eq!(config.raw["program"], json!("/w/target/debug/deps/tiny-05"));
        assert_eq!(config.raw["type"], json!("lldb"));
        assert!(debug_config(&rust[0], root, None).is_err());
        let config = debug_config(&go[0], root, None).unwrap();
        assert_eq!((config.kind.as_str(), &config.raw["mode"], &config.raw["args"]), ("go", &json!("test"), &json!(["-test.run", "^TestAdd$"])));
        let python = Test::new(Kind::Python, "tests/test_a.py::test_one", "test_one", "tests/test_a.py", None);
        let config = debug_config(&python, root, None).unwrap();
        assert_eq!((config.kind.as_str(), &config.raw["module"], &config.raw["args"]), ("debugpy", &json!("pytest"), &json!(["tests/test_a.py::test_one"])));
        assert!(debug_config(&js[0], root, None).unwrap_err().contains("not supported"));
    }

    #[test]
    fn detects_providers() {
        let root = temp("detect");
        assert!(detect(&root).is_empty());
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(root.join("go.mod"), "module m").unwrap();
        std::fs::write(root.join("pyproject.toml"), "[tool.pytest.ini_options]").unwrap();
        std::fs::write(root.join("package.json"), "{\"devDependencies\": {\"jest\": \"^29\"}}").unwrap();
        assert_eq!(detect(&root), [Kind::Rust, Kind::Python, Kind::Go, Kind::Jest]);
        std::fs::write(root.join("package.json"), "{\"devDependencies\": {\"vitest\": \"^2\", \"jest\": \"1\"}}").unwrap();
        assert_eq!(detect(&root)[3], Kind::Vitest);
    }
}
