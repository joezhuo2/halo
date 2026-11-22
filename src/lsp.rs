use crate::settings::Settings;
use crate::ui::icons;
use eframe::egui;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

pub type Pos = (usize, usize);

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub path: PathBuf,
    pub start: Pos,
    pub end: Pos,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub start: Pos,
    pub end: Pos,
    pub severity: u8,
    pub message: String,
    pub source: String,
    pub raw: Value,
}

pub struct FileProblems {
    pub path: PathBuf,
    server: String,
    pub items: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub start: Pos,
    pub end: Pos,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: u8,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub depth: usize,
    pub container: String,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub kind: u8,
    pub filter: String,
    pub sort: String,
    pub text: String,
    pub range: Option<(Pos, Pos)>,
    pub extra: Vec<Edit>,
    pub snippet: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub label: String,
    pub param: Option<(usize, usize)>,
    pub doc: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub line: usize,
    pub col: usize,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeLens {
    pub line: usize,
    pub title: String,
    pub command: String,
    pub args: Value,
    pub raw: Value,
}

pub fn key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) { text.replace('/', "\\").to_lowercase() } else { text.into_owned() }
}

pub fn worst(severities: &BTreeMap<String, u8>, path: &Path) -> Option<u8> {
    let file = key(path);
    let sep = std::path::MAIN_SEPARATOR;
    let dir = if file.ends_with(sep) { file.clone() } else { format!("{file}{sep}") };
    let inside = severities.range(dir.clone()..).take_while(|(k, _)| k.starts_with(&dir)).map(|(_, s)| *s);
    severities.get(&file).copied().into_iter().chain(inside).min()
}

pub fn uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        let hex = (rest[i] == b'%').then(|| rest.get(i + 1..i + 3)).flatten().and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) => {
                bytes.push(byte);
                i += 3;
            }
            None => {
                bytes.push(rest[i]);
                i += 1;
            }
        }
    }
    let mut text = String::from_utf8(bytes).ok()?;
    if let [b'/', drive, b':', ..] = text.as_bytes()
        && drive.is_ascii_alphabetic()
    {
        text = format!("{}{}", text[1..2].to_uppercase(), &text[2..]);
    }
    Some(PathBuf::from(if cfg!(windows) { text.replace('/', "\\") } else { text }))
}

pub fn language_id(path: &Path) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "h" | "hpp" | "hh" | "cc" | "cxx" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "htm" => "html",
        "md" => "markdown",
        "sh" | "bash" => "shellscript",
        "yml" => "yaml",
        "txt" | "" => "plaintext",
        other => other,
    }
    .to_owned()
}

pub fn utf16_to_char(chars: impl Iterator<Item = char>, col: usize) -> usize {
    let (mut units, mut count) = (0, 0);
    for c in chars {
        if units >= col {
            break;
        }
        units += c.len_utf16();
        count += 1;
    }
    count
}

pub fn char_to_utf16(chars: impl Iterator<Item = char>, col: usize) -> usize {
    chars.take(col).map(char::len_utf16).sum()
}

pub fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (at, c) in text.char_indices() {
        if seen >= units {
            return at;
        }
        seen += c.len_utf16();
    }
    text.len()
}

pub fn shift(line: usize, edited: usize, delta: isize) -> Option<usize> {
    if line <= edited {
        return Some(line);
    }
    let moved = line as isize + delta;
    (moved > edited as isize).then_some(moved as usize)
}

pub fn at(path: &Path, pos: Pos) -> Value {
    json!({"textDocument": {"uri": uri(path)}, "position": {"line": pos.0, "character": pos.1}})
}

pub fn range_json(start: Pos, end: Pos) -> Value {
    json!({"start": {"line": start.0, "character": start.1}, "end": {"line": end.0, "character": end.1}})
}

pub fn symbol_icon(kind: u8) -> &'static str {
    match kind {
        1 => icons::FILE,
        2..=4 => icons::PACKAGE,
        5 | 23 => icons::CUBE,
        6 | 9 | 12 => icons::FUNCTION,
        7 | 8 | 20 => icons::WRENCH,
        10 | 22 => icons::LIST_BULLETS,
        11 => icons::PUZZLE_PIECE,
        14 => icons::HASH,
        26 => icons::TEXT_T,
        _ => icons::BRACKETS_CURLY,
    }
}

pub fn completion_icon(kind: u8) -> &'static str {
    match kind {
        2..=4 => icons::FUNCTION,
        5 | 10 => icons::WRENCH,
        7 | 22 => icons::CUBE,
        8 => icons::PUZZLE_PIECE,
        9 => icons::PACKAGE,
        13 | 20 => icons::LIST_BULLETS,
        14 => icons::KEY,
        21 => icons::HASH,
        25 => icons::TEXT_T,
        _ => icons::BRACKETS_CURLY,
    }
}

pub fn pos(value: &Value) -> Pos {
    (value["line"].as_u64().unwrap_or(0) as usize, value["character"].as_u64().unwrap_or(0) as usize)
}

fn range(value: &Value) -> (Pos, Pos) {
    (pos(&value["start"]), pos(&value["end"]))
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::Array(items) => items,
        Value::Object(_) => std::slice::from_ref(value),
        _ => &[],
    }
}

pub fn locations(value: &Value) -> Vec<Location> {
    list(value)
        .iter()
        .filter_map(|item| {
            let (target, span) = match item.get("targetUri") {
                Some(target) => (target, &item["targetSelectionRange"]),
                None => (&item["uri"], &item["range"]),
            };
            let (start, end) = range(span);
            Some(Location { path: path_of(target.as_str()?)?, start, end, label: String::new() })
        })
        .collect()
}

pub fn highlights(value: &Value) -> Vec<(Pos, Pos)> {
    list(value).iter().map(|item| range(&item["range"])).collect()
}

pub fn hierarchy(value: &Value) -> Vec<Location> {
    list(value)
        .iter()
        .filter_map(|entry| {
            let item = [&entry["from"], &entry["to"]].into_iter().find(|v| v.is_object()).unwrap_or(entry);
            let (start, end) = range(&item["selectionRange"]);
            Some(Location { path: path_of(item["uri"].as_str()?)?, start, end, label: item["name"].as_str().unwrap_or_default().to_owned() })
        })
        .collect()
}

pub fn edits(value: &Value) -> Vec<Edit> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|edit| {
            let (start, end) = range(if edit.get("range").is_some() { &edit["range"] } else { &edit["insert"] });
            Edit { start, end, text: edit["newText"].as_str().unwrap_or_default().replace("\r\n", "\n") }
        })
        .collect()
}

pub fn workspace_edit(value: &Value) -> Vec<(PathBuf, Vec<Edit>)> {
    let mut out = vec![];
    for (target, changes) in value["changes"].as_object().into_iter().flatten() {
        out.extend(path_of(target).map(|path| (path, edits(changes))));
    }
    for change in value["documentChanges"].as_array().into_iter().flatten() {
        out.extend(change["textDocument"]["uri"].as_str().and_then(path_of).map(|path| (path, edits(&change["edits"]))));
    }
    out
}

pub fn completions(value: &Value) -> (Vec<Completion>, bool) {
    let (items, incomplete) = match value {
        Value::Array(items) => (items.as_slice(), false),
        _ => (value["items"].as_array().map_or(&[] as &[Value], Vec::as_slice), value["isIncomplete"] == true),
    };
    let found = items
        .iter()
        .take(3000)
        .map(|item| {
            let label = item["label"].as_str().unwrap_or_default().to_owned();
            let edit = &item["textEdit"];
            let span = if edit.get("range").is_some() { &edit["range"] } else { &edit["insert"] };
            let text = [&edit["newText"], &item["insertText"]].into_iter().find_map(Value::as_str).unwrap_or(&label);
            let details = [&item["detail"], &item["labelDetails"]["detail"], &item["labelDetails"]["description"]];
            Completion {
                detail: details.into_iter().find_map(Value::as_str).unwrap_or_default().lines().next().unwrap_or_default().to_owned(),
                kind: item["kind"].as_u64().unwrap_or(1) as u8,
                filter: item["filterText"].as_str().unwrap_or(&label).to_owned(),
                sort: item["sortText"].as_str().unwrap_or(&label).to_owned(),
                text: text.to_owned(),
                snippet: item["insertTextFormat"] == 2,
                range: edit.is_object().then(|| range(span)),
                extra: edits(&item["additionalTextEdits"]),
                label,
            }
        })
        .collect();
    (found, incomplete)
}

pub fn hover(value: &Value) -> String {
    fn part(value: &Value) -> String {
        let text = value["value"].as_str().unwrap_or_default();
        match (value, value["language"].as_str()) {
            (Value::String(text), _) => text.clone(),
            (_, Some(language)) => format!("```{language}\n{text}\n```"),
            _ => text.to_owned(),
        }
    }
    match &value["contents"] {
        Value::Array(items) => items.iter().map(part).collect::<Vec<_>>().join("\n\n"),
        other => part(other),
    }
}

pub fn blocks(text: &str) -> Vec<(bool, String)> {
    let mut out = vec![];
    let (mut code, mut current) = (false, String::new());
    let mut flush = |code: bool, current: &mut String| {
        if !current.trim().is_empty() {
            out.push((code, current.trim_matches('\n').to_owned()));
        }
        current.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(code, &mut current);
            code = !code;
        } else if code {
            current.push_str(line);
            current.push('\n');
        } else {
            current.push_str(&line.trim_start_matches('#').trim_start().replace("**", "").replace('`', ""));
            current.push('\n');
        }
    }
    flush(code, &mut current);
    out
}

pub fn signature(value: &Value) -> Option<Signature> {
    let all = value["signatures"].as_array()?;
    let sig = all.get(value["activeSignature"].as_u64().unwrap_or(0) as usize).or(all.first())?;
    let label = sig["label"].as_str()?.to_owned();
    let active = sig["activeParameter"].as_u64().or(value["activeParameter"].as_u64()).unwrap_or(0) as usize;
    let param = sig["parameters"].get(active).and_then(|p| match &p["label"] {
        Value::String(name) => label.find(name.as_str()).map(|at| (at, at + name.len())),
        Value::Array(pair) => {
            let offset = |i: usize| Some(utf16_to_byte(&label, pair.get(i)?.as_u64()? as usize));
            Some((offset(0)?, offset(1)?))
        }
        _ => None,
    });
    let doc = match &sig["documentation"] {
        Value::String(text) => text.clone(),
        other => other["value"].as_str().unwrap_or_default().to_owned(),
    };
    Some(Signature { label, param, doc })
}

pub fn symbols(value: &Value) -> Vec<Symbol> {
    fn walk(items: &[Value], depth: usize, container: &str, out: &mut Vec<Symbol>) {
        for item in items {
            let name = item["name"].as_str().unwrap_or_default().to_owned();
            let located = item.get("location");
            let (whole, picked) = match located {
                Some(location) => (&location["range"], &location["range"]),
                None => (&item["range"], &item["selectionRange"]),
            };
            let ((line, col), (end_line, _)) = (pos(&picked["start"]), pos(&whole["end"]));
            out.push(Symbol {
                name: name.clone(),
                kind: item["kind"].as_u64().unwrap_or(0) as u8,
                line,
                col,
                end_line: end_line.max(line),
                depth,
                container: item["containerName"].as_str().unwrap_or(container).to_owned(),
                path: located.and_then(|l| l["uri"].as_str()).and_then(path_of),
            });
            if let Some(children) = item["children"].as_array() {
                walk(children, depth + 1, &name, out);
            }
        }
    }
    let mut out = vec![];
    walk(value.as_array().map_or(&[], Vec::as_slice), 0, "", &mut out);
    out
}

pub fn tokens(value: &Value) -> Vec<(usize, usize, usize, usize)> {
    let data: Vec<usize> = value["data"].as_array().into_iter().flatten().map(|n| n.as_u64().unwrap_or(0) as usize).collect();
    let (mut line, mut col) = (0, 0);
    data.as_chunks::<5>()
        .0
        .iter()
        .map(|t| {
            col = if t[0] > 0 { t[1] } else { col + t[1] };
            line += t[0];
            (line, col, t[2], t[3])
        })
        .collect()
}

pub fn hints(value: &Value) -> Vec<Hint> {
    let mut out: Vec<Hint> = value
        .as_array()
        .into_iter()
        .flatten()
        .map(|hint| {
            let (line, col) = pos(&hint["position"]);
            let mut label = match &hint["label"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
                _ => String::new(),
            };
            if hint["paddingLeft"] == true {
                label.insert(0, ' ');
            }
            if hint["paddingRight"] == true {
                label.push(' ');
            }
            Hint { line, col, label: label.replace('\n', " ") }
        })
        .filter(|hint| !hint.label.trim().is_empty())
        .collect();
    out.sort_by_key(|h| (h.line, h.col));
    out
}

pub fn actions(value: &Value) -> Vec<(String, Value)> {
    let titled = |item: &Value| Some((item["title"].as_str()?.to_owned(), item.clone()));
    value.as_array().into_iter().flatten().filter_map(titled).collect()
}

pub fn quick_fix(actions: &[(String, Value)]) -> bool {
    actions.iter().any(|(_, action)| action["isPreferred"] == true || action["kind"].as_str().is_some_and(|kind| kind.starts_with("quickfix")))
}

pub fn code_lens(item: &Value) -> CodeLens {
    let (command, text) = (&item["command"], |value: &Value| value.as_str().unwrap_or_default().to_owned());
    CodeLens { line: pos(&item["range"]["start"]).0, title: text(&command["title"]), command: text(&command["command"]), args: command["arguments"].clone(), raw: item.clone() }
}

pub fn code_lenses(value: &Value) -> Vec<CodeLens> {
    let mut out: Vec<CodeLens> = list(value).iter().map(code_lens).collect();
    out.sort_by_key(|lens| lens.line);
    out
}

pub fn runnable(value: &Value) -> Option<(String, Option<PathBuf>, Option<String>)> {
    let args = &value["args"];
    let words = |key: &str| -> Vec<String> { args[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect() };
    let (cargo, run) = ([words("cargoArgs"), words("cargoExtraArgs")].concat(), words("executableArgs"));
    if value["kind"] != "cargo" || cargo.is_empty() {
        return None;
    }
    let mut line = vec![args["overrideCargo"].as_str().unwrap_or("cargo").to_owned()];
    line.extend(cargo.iter().cloned());
    if !run.is_empty() {
        line.push("--".into());
        line.extend(run.iter().cloned());
    }
    let line: Vec<String> = line.iter().map(|word| crate::tasks::quote(word)).collect();
    let cwd = args["cwd"].as_str().or(args["workspaceRoot"].as_str()).map(PathBuf::from);
    let exact = cargo[0] == "test" && run.iter().any(|word| word == "--exact");
    Some((line.join(" "), cwd, run.first().filter(|name| exact && !name.starts_with('-')).cloned()))
}

fn diagnostic(value: &Value) -> Diagnostic {
    let (start, end) = range(&value["range"]);
    let code = match &value["code"] {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    };
    let source = match (value["source"].as_str().unwrap_or_default(), code.as_str()) {
        ("", code) => code.to_owned(),
        (source, "") => source.to_owned(),
        (source, code) => format!("{source}({code})"),
    };
    Diagnostic {
        start,
        end,
        severity: value["severity"].as_u64().unwrap_or(1) as u8,
        message: value["message"].as_str().unwrap_or_default().to_owned(),
        source,
        raw: value.clone(),
    }
}

fn map_strings(value: &mut Value, f: &dyn Fn(&str) -> Option<String>) {
    match value {
        Value::String(text) => {
            if let Some(mapped) = f(text) {
                *text = mapped;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| map_strings(v, f)),
        Value::Object(map) => {
            *map = std::mem::take(map)
                .into_iter()
                .map(|(key, mut v)| {
                    map_strings(&mut v, f);
                    (f(&key).unwrap_or(key), v)
                })
                .collect();
        }
        _ => {}
    }
}

fn to_wsl(distro: &str, text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let distro = distro.to_ascii_lowercase();
    for host in ["wsl.localhost", "wsl%24"] {
        let prefix = format!("file:////{host}/{distro}/");
        if lower.starts_with(&prefix) {
            return Some(format!("file:///{}", &text[prefix.len()..]));
        }
    }
    crate::wsl::parse(Path::new(text)).filter(|(d, _)| d.eq_ignore_ascii_case(&distro)).map(|(_, linux)| linux)
}

fn from_wsl(distro: &str, text: &str) -> Option<String> {
    let rest = text.strip_prefix("file:///").filter(|rest| !rest.starts_with('/'))?;
    Some(format!("file:////wsl.localhost/{distro}/{rest}"))
}

pub(crate) fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim().to_ascii_lowercase();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("content-length:") {
            length = value.trim().parse().ok()?;
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(serde_json::from_slice(&body).unwrap_or(Value::Null))
}

pub(crate) fn frame(message: &Value) -> String {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    let direct = Path::new(name);
    if direct.components().count() > 1 {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    let extensions: &[&str] = if cfg!(windows) && direct.extension().is_none() { &[".exe", ".cmd", ".bat"] } else { &[""] };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| extensions.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file()))
}

fn workspace_folders(folders: &[(PathBuf, String)]) -> Value {
    Value::Array(folders.iter().map(|(path, name)| json!({"uri": uri(path), "name": name})).collect())
}

fn folders_changed(old: &[(PathBuf, String)], new: &[(PathBuf, String)]) -> Option<Value> {
    let added: Vec<(PathBuf, String)> = new.iter().filter(|f| !old.iter().any(|o| o.0 == f.0)).cloned().collect();
    let removed: Vec<(PathBuf, String)> = old.iter().filter(|f| !new.iter().any(|n| n.0 == f.0)).cloned().collect();
    let event = json!({"added": workspace_folders(&added), "removed": workspace_folders(&removed)});
    (!added.is_empty() || !removed.is_empty()).then(|| json!({"event": event}))
}

fn initialize(id: u64, root: &Path, folders: &[(PathBuf, String)]) -> Value {
    let name = root.file_name().unwrap_or(root.as_os_str()).to_string_lossy();
    let link = json!({"linkSupport": true});
    let completion = json!({
        "contextSupport": true,
        "completionItem": {"snippetSupport": true, "documentationFormat": ["plaintext", "markdown"], "labelDetailsSupport": true},
    });
    let signature = json!({
        "documentationFormat": ["plaintext", "markdown"],
        "parameterInformation": {"labelOffsetSupport": true},
        "activeParameterSupport": true,
    });
    let kinds = json!(["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports"]);
    let types = json!([
        "namespace", "type", "class", "enum", "interface", "struct", "typeParameter", "parameter", "variable", "property",
        "enumMember", "event", "function", "method", "macro", "keyword", "modifier", "comment", "string", "number", "regexp",
        "operator", "decorator",
    ]);
    let tokens = json!({"requests": {"full": true}, "formats": ["relative"], "tokenModifiers": [], "tokenTypes": types});
    let document = json!({
        "synchronization": {"didSave": true},
        "publishDiagnostics": {},
        "completion": completion,
        "hover": {"contentFormat": ["markdown", "plaintext"]},
        "signatureHelp": {"signatureInformation": signature},
        "definition": link,
        "declaration": link,
        "typeDefinition": link,
        "implementation": link,
        "references": {},
        "documentHighlight": {},
        "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
        "codeAction": {"codeActionLiteralSupport": {"codeActionKind": {"valueSet": kinds}}},
        "rename": {},
        "formatting": {},
        "rangeFormatting": {},
        "semanticTokens": tokens,
        "inlayHint": {},
        "callHierarchy": {},
        "typeHierarchy": {},
        "codeLens": {},
    });
    let workspace = json!({
        "applyEdit": true,
        "configuration": true,
        "workspaceFolders": true,
        "workspaceEdit": {"documentChanges": false},
        "symbol": {},
        "executeCommand": {},
    });
    let capabilities = json!({
        "general": {"positionEncodings": ["utf-16"]},
        "window": {"workDoneProgress": true},
        "workspace": workspace,
        "textDocument": document,
    });
    let params = json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "Halo", "version": env!("CARGO_PKG_VERSION")},
        "rootPath": root.to_string_lossy(),
        "rootUri": uri(root),
        "workspaceFolders": if folders.is_empty() { json!([{"uri": uri(root), "name": name}]) } else { workspace_folders(folders) },
        "capabilities": capabilities,
    });
    json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": params})
}

fn capability(method: &str) -> Option<&'static str> {
    Some(match method {
        "textDocument/completion" => "completionProvider",
        "textDocument/hover" => "hoverProvider",
        "textDocument/signatureHelp" => "signatureHelpProvider",
        "textDocument/definition" => "definitionProvider",
        "textDocument/declaration" => "declarationProvider",
        "textDocument/typeDefinition" => "typeDefinitionProvider",
        "textDocument/implementation" => "implementationProvider",
        "textDocument/references" => "referencesProvider",
        "textDocument/documentHighlight" => "documentHighlightProvider",
        "textDocument/documentSymbol" => "documentSymbolProvider",
        "textDocument/codeAction" => "codeActionProvider",
        "textDocument/rename" => "renameProvider",
        "textDocument/formatting" => "documentFormattingProvider",
        "textDocument/rangeFormatting" => "documentRangeFormattingProvider",
        "textDocument/semanticTokens/full" => "semanticTokensProvider",
        "textDocument/inlayHint" => "inlayHintProvider",
        "textDocument/codeLens" => "codeLensProvider",
        "textDocument/prepareCallHierarchy" => "callHierarchyProvider",
        "textDocument/prepareTypeHierarchy" => "typeHierarchyProvider",
        "workspace/symbol" => "workspaceSymbolProvider",
        _ => return None,
    })
}

struct Server {
    command: String,
    name: String,
    wsl: Option<String>,
    child: Option<Child>,
    tx: Sender<String>,
    rx: Receiver<Value>,
    init: u64,
    ready: bool,
    dead: bool,
    queued: Vec<Value>,
    caps: Value,
    open: HashMap<String, (String, i64)>,
    progress: BTreeMap<String, (String, String)>,
}

impl Server {
    fn write(&self, message: &Value) {
        let Some(distro) = &self.wsl else { return drop(self.tx.send(frame(message))) };
        let mut message = message.clone();
        map_strings(&mut message, &|text| to_wsl(distro, text));
        let _ = self.tx.send(frame(&message));
    }

    fn send(&mut self, message: Value) {
        if self.ready {
            self.write(&message);
        } else {
            self.queued.push(message);
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn can(&self, method: &str) -> bool {
        let Some(name) = capability(method) else { return true };
        !self.ready || !matches!(self.caps[name], Value::Null | Value::Bool(false))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.write(&json!({"jsonrpc": "2.0", "id": 0, "method": "shutdown"}));
        self.write(&json!({"jsonrpc": "2.0", "method": "exit"}));
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

fn spawn(command: &str, root: &Path, ctx: &egui::Context) -> Result<Server, String> {
    let mut parts = command.split_whitespace();
    let name = parts.next().ok_or("the command is empty")?;
    let wsl = crate::wsl::parse(root).map(|(distro, _)| distro);
    let mut process = match &wsl {
        // a login shell so servers installed under ~/.local/bin or by version managers are on PATH
        Some(_) => {
            let args: Vec<String> = ["-lc", "exec \"$0\" \"$@\"", name].into_iter().chain(parts).map(str::to_owned).collect();
            let (program, args) = crate::wsl::wrap(Some(root), "sh", &args);
            let mut process = crate::terminal::hidden_command(&program);
            process.args(args);
            process
        }
        None => {
            let program = find_program(name).ok_or_else(|| format!("{name} was not found on PATH"))?;
            let mut process = crate::terminal::hidden_command(&program.to_string_lossy());
            process.args(parts).current_dir(root);
            process
        }
    };
    let mut child = process
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (mut stdin, stdout, stderr) = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
        (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
        _ => return Err("its pipes could not be opened".into()),
    };
    let (tx, out_rx) = channel::<String>();
    let (in_tx, rx) = channel::<Value>();
    std::thread::spawn(move || {
        for text in out_rx {
            if stdin.write_all(text.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                break;
            }
        }
    });
    let (messages, repaint, distro) = (in_tx.clone(), ctx.clone(), wsl.clone());
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Some(mut message) = read_message(&mut reader) {
            if let Some(distro) = &distro {
                map_strings(&mut message, &|text| from_wsl(distro, text));
            }
            if messages.send(message).is_err() {
                return;
            }
            repaint.request_repaint();
        }
        let _ = messages.send(json!({"method": "$/exit"}));
        repaint.request_repaint();
    });
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if in_tx.send(json!({"method": "$/stderr", "params": line})).is_err() {
                break;
            }
        }
    });
    Ok(Server {
        command: command.to_owned(),
        name: Path::new(name).file_stem().unwrap_or_default().to_string_lossy().into_owned(),
        wsl,
        child: Some(child),
        tx,
        rx,
        init: 0,
        ready: false,
        dead: false,
        queued: vec![],
        caps: Value::Null,
        open: HashMap::new(),
        progress: BTreeMap::new(),
    })
}

#[derive(Default)]
pub struct Lsp {
    root: Option<PathBuf>,
    folders: Vec<(PathBuf, String)>,
    config: (bool, BTreeMap<String, String>),
    servers: Vec<Server>,
    failed: HashSet<String>,
    next_id: u64,
    replies: HashMap<u64, Option<Value>>,
    problems: BTreeMap<String, FileProblems>,
    pub edits: Vec<Value>,
    pub log: Vec<String>,
    pub epoch: u64,
    pub settled: u64,
}

impl Lsp {
    pub fn configure(&mut self, settings: &Settings, root: Option<&Path>) {
        if self.config.0 == settings.lsp && self.config.1 == settings.servers && self.root.as_deref() == root {
            return;
        }
        self.config = (settings.lsp, settings.servers.clone());
        self.root = root.map(Path::to_path_buf);
        self.restart();
    }

    pub fn set_folders(&mut self, folders: Vec<(PathBuf, String)>) {
        if let Some(change) = folders_changed(&self.folders, &folders) {
            self.servers.iter_mut().for_each(|s| s.notify("workspace/didChangeWorkspaceFolders", change.clone()));
        }
        self.folders = folders;
    }

    pub fn restart(&mut self) {
        for server in &self.servers {
            crate::output::log("Language Servers", format!("[{}] stopped", server.name));
        }
        self.servers.clear();
        self.failed.clear();
        self.problems.clear();
        self.replies.clear();
        self.epoch += 1;
    }

    fn note(&mut self, text: String) {
        crate::output::log("Language Servers", &text);
        self.log.push(text);
        if self.log.len() > 1200 {
            self.log.drain(..200);
        }
    }

    fn index_of(&self, path: &Path) -> Option<usize> {
        let key = key(path);
        self.servers.iter().position(|s| s.open.contains_key(&key))
    }

    pub fn open(&mut self, path: &Path, text: impl FnOnce() -> String, ctx: &egui::Context) -> bool {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(program) = self.config.1.get(&ext).filter(|c| self.config.0 && !c.trim().is_empty()).cloned() else { return false };
        let distro = crate::wsl::parse(path).map(|(distro, _)| distro);
        if distro.is_none() && path.to_string_lossy().starts_with(r"\\") {
            return false;
        }
        if self.index_of(path).is_some() {
            self.change(path, text());
            return true;
        }
        let command = match &distro {
            Some(distro) => format!("{program} [WSL: {distro}]"),
            None => program.clone(),
        };
        let index = match self.servers.iter().position(|s| s.command == command) {
            Some(index) => index,
            None if self.failed.contains(&command) => return false,
            None => {
                let same_side = |f: &PathBuf| crate::wsl::parse(f).map(|(d, _)| d) == distro;
                let owner = self.folders.iter().map(|(f, _)| f).filter(|f| path.starts_with(f)).max_by_key(|f| f.components().count());
                let root = owner.cloned().or_else(|| self.root.clone().filter(same_side)).or_else(|| path.parent().map(Path::to_path_buf)).unwrap_or_default();
                match spawn(&program, &root, ctx) {
                    Ok(mut server) => {
                        self.next_id += 1;
                        server.init = self.next_id;
                        server.write(&initialize(server.init, &root, &self.folders));
                        self.note(format!("Started {command} in {}", root.display()));
                        self.servers.push(server);
                        self.servers.len() - 1
                    }
                    Err(error) => {
                        self.note(format!("{command} did not start: {error}"));
                        self.failed.insert(command);
                        return false;
                    }
                }
            }
        };
        let target = uri(path);
        let server = &mut self.servers[index];
        server.open.insert(key(path), (target.clone(), 1));
        let document = json!({"uri": target, "languageId": language_id(path), "version": 1, "text": text()});
        server.notify("textDocument/didOpen", json!({"textDocument": document}));
        true
    }

    pub fn change(&mut self, path: &Path, text: String) {
        let Some(index) = self.index_of(path) else { return };
        let server = &mut self.servers[index];
        let Some(entry) = server.open.get_mut(&key(path)) else { return };
        entry.1 += 1;
        let document = json!({"uri": entry.0, "version": entry.1});
        server.notify("textDocument/didChange", json!({"textDocument": document, "contentChanges": [{"text": text}]}));
    }

    pub fn save(&mut self, path: &Path) {
        if let Some(index) = self.index_of(path) {
            self.servers[index].notify("textDocument/didSave", json!({"textDocument": {"uri": uri(path)}}));
        }
    }

    pub fn close(&mut self, path: &Path) {
        let Some(index) = self.index_of(path) else { return };
        let server = &mut self.servers[index];
        if let Some((target, _)) = server.open.remove(&key(path)) {
            server.notify("textDocument/didClose", json!({"textDocument": {"uri": target}}));
        }
        self.problems.remove(&key(path));
    }

    pub fn request(&mut self, path: &Path, method: &str, params: Value) -> Option<u64> {
        let index = self.index_of(path).filter(|index| self.servers[*index].can(method))?;
        self.next_id += 1;
        self.servers[index].send(json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}));
        Some(self.next_id)
    }

    pub fn workspace_symbols(&mut self, query: &str) -> Vec<u64> {
        let mut ids = vec![];
        for server in self.servers.iter_mut().filter(|s| s.can("workspace/symbol")) {
            self.next_id += 1;
            server.send(json!({"jsonrpc": "2.0", "id": self.next_id, "method": "workspace/symbol", "params": {"query": query}}));
            ids.push(self.next_id);
        }
        ids
    }

    pub fn take(&mut self, id: u64) -> Option<Option<Value>> {
        self.replies.remove(&id)
    }

    pub fn triggers(&self, path: &Path, provider: &str) -> Vec<char> {
        let Some(index) = self.index_of(path) else { return vec![] };
        let list = self.servers[index].caps[provider]["triggerCharacters"].as_array();
        list.into_iter().flatten().filter_map(|c| c.as_str()?.chars().next()).collect()
    }

    pub fn provider(&self, path: &Path, name: &str) -> Value {
        self.index_of(path).map_or(Value::Null, |index| self.servers[index].caps[name].clone())
    }

    pub fn token_types(&self, path: &Path) -> Vec<String> {
        let Some(index) = self.index_of(path) else { return vec![] };
        let list = self.servers[index].caps["semanticTokensProvider"]["legend"]["tokenTypes"].as_array();
        list.into_iter().flatten().map(|t| t.as_str().unwrap_or_default().to_owned()).collect()
    }

    pub fn status(&self) -> Option<(String, String)> {
        let names: Vec<&str> = self.servers.iter().map(|s| s.name.as_str()).collect();
        let busy = self.servers.iter().find_map(|s| s.progress.values().next().map(|(title, detail)| format!("{}: {title} {detail}", s.name)));
        let starting = self.servers.iter().find(|s| !s.ready).map(|s| format!("{}: starting", s.name));
        (!names.is_empty()).then(|| (busy.or(starting).unwrap_or_else(|| names.join(", ")), format!("Language servers: {}", names.join(", "))))
    }

    pub fn diagnostics(&self, path: &Path) -> &[Diagnostic] {
        self.problems.get(&key(path)).map_or(&[], |p| p.items.as_slice())
    }

    pub fn problems(&self) -> impl Iterator<Item = &FileProblems> {
        self.problems.values()
    }

    pub fn severities(&self) -> BTreeMap<String, u8> {
        let worst = |p: &FileProblems| p.items.iter().map(|d| d.severity).filter(|s| *s <= 2).min();
        self.problems.iter().filter_map(|(k, p)| Some((k.clone(), worst(p)?))).collect()
    }

    pub fn counts(&self) -> (usize, usize) {
        let count = |severity| self.problems.values().flat_map(|p| &p.items).filter(|d| d.severity == severity).count();
        (count(1), count(2))
    }

    pub fn shift_diagnostics(&mut self, path: &Path, edited: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for item in self.problems.get_mut(&key(path)).into_iter().flat_map(|p| &mut p.items) {
            item.start.0 = shift(item.start.0, edited, delta).unwrap_or(edited);
            item.end.0 = shift(item.end.0, edited, delta).unwrap_or(edited).max(item.start.0);
        }
    }

    pub fn poll(&mut self) {
        for index in 0..self.servers.len() {
            let messages: Vec<Value> = self.servers[index].rx.try_iter().collect();
            for message in messages {
                self.handle(index, message);
            }
        }
        if self.servers.iter().any(|s| s.dead) {
            for server in self.servers.iter().filter(|s| s.dead) {
                self.failed.insert(server.command.clone());
                self.problems.retain(|_, p| p.server != server.command);
                crate::ui::notify(format!("The language server {} stopped. Its messages are in the Output panel.", server.name));
            }
            self.servers.retain(|s| !s.dead);
            self.epoch += 1;
        }
        if self.replies.len() > 256 {
            let oldest = self.next_id.saturating_sub(128);
            self.replies.retain(|id, _| *id > oldest);
        }
    }

    fn handle(&mut self, index: usize, message: Value) {
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let params = &message["params"];
        let name = self.servers[index].name.clone();
        match (method.as_str(), message.get("id")) {
            ("", Some(id)) => {
                let id = id.as_u64().unwrap_or(0);
                let server = &mut self.servers[index];
                if id == server.init {
                    server.caps = message["result"]["capabilities"].clone();
                    server.ready = true;
                    server.write(&json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
                    for queued in std::mem::take(&mut server.queued) {
                        server.write(&queued);
                    }
                } else if id > 0 {
                    self.replies.insert(id, message.get("error").is_none().then(|| message["result"].clone()));
                }
            }
            (_, Some(id)) => {
                let result = match method.as_str() {
                    "workspace/configuration" => Value::Array(vec![Value::Null; params["items"].as_array().map_or(0, Vec::len)]),
                    "workspace/workspaceFolders" => workspace_folders(&self.folders),
                    "workspace/applyEdit" => {
                        self.edits.push(params["edit"].clone());
                        json!({"applied": true})
                    }
                    _ => Value::Null,
                };
                self.servers[index].write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
            ("textDocument/publishDiagnostics", None) => {
                let Some(path) = params["uri"].as_str().and_then(path_of) else { return };
                let mut items: Vec<Diagnostic> = params["diagnostics"].as_array().into_iter().flatten().map(diagnostic).collect();
                items.sort_by_key(|d| d.start);
                if items.is_empty() {
                    self.problems.remove(&key(&path));
                } else {
                    let server = self.servers[index].command.clone();
                    self.problems.insert(key(&path), FileProblems { path, server, items });
                }
            }
            ("window/logMessage" | "window/showMessage", None) => {
                self.note(format!("[{name}] {}", params["message"].as_str().unwrap_or_default()));
            }
            ("$/stderr", None) => self.note(format!("[{name}] {}", params.as_str().unwrap_or_default())),
            ("$/progress", None) => {
                let (token, value) = (params["token"].to_string(), &params["value"]);
                let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
                let progress = &mut self.servers[index].progress;
                match value["kind"].as_str() {
                    Some("begin") => {
                        progress.insert(token, (text("title"), text("message")));
                    }
                    Some("report") => {
                        if let Some(entry) = progress.get_mut(&token) {
                            entry.1 = match value["percentage"].as_u64() {
                                Some(percent) => format!("{} {percent}%", text("message")),
                                None => text("message"),
                            };
                        }
                    }
                    Some("end") => {
                        progress.remove(&token);
                        self.settled += 1;
                    }
                    _ => {}
                }
            }
            ("$/exit", None) => {
                self.servers[index].dead = true;
                self.note(format!("[{name}] the server exited"));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_wsl_uris_both_ways() {
        let file = PathBuf::from(r"\\wsl.localhost\Ubuntu\home\joe\a b.cpp");
        let mut out = json!({"textDocument": {"uri": uri(&file)}, "rootPath": r"\\wsl.localhost\Ubuntu\home\joe", "other": "file:///C:/x"});
        map_strings(&mut out, &|t| to_wsl("Ubuntu", t));
        assert_eq!(out, json!({"textDocument": {"uri": "file:///home/joe/a%20b.cpp"}, "rootPath": "/home/joe", "other": "file:///C:/x"}));
        let mut back = json!({"changes": {"file:///home/joe/a%20b.cpp": [{"uri": "file:///usr/include/vector"}]}});
        map_strings(&mut back, &|t| from_wsl("Ubuntu", t));
        let key = uri(&file);
        assert_eq!(path_of(back["changes"][&key][0]["uri"].as_str().unwrap()), Some(PathBuf::from(r"\\wsl.localhost\Ubuntu\usr\include\vector")));
        assert!(back["changes"].get(&key).is_some());
    }

    #[test]
    fn worst_severity_for_files_and_folders() {
        let root = PathBuf::from(if cfg!(windows) { r"\\wsl.localhost\Ubuntu-24.04\home\joe\proj" } else { "/home/joe/proj" });
        let (main, util) = (root.join("src").join("main.cpp"), root.join("src-old").join("util.h"));
        let mut map = BTreeMap::from([(key(&main), 1), (key(&util), 2)]);
        if cfg!(windows) {
            let published = path_of(&from_wsl("Ubuntu-24.04", "file:///home/joe/proj/src/main.cpp").unwrap()).unwrap();
            assert_eq!(worst(&map, &published), Some(1));
            assert_eq!(worst(&map, Path::new(r"\\WSL.localhost\ubuntu-24.04/home/joe/proj/src/main.cpp")), Some(1));
        }
        assert_eq!(worst(&map, &main), Some(1));
        assert_eq!(worst(&map, &root.join("src")), Some(1));
        assert_eq!(worst(&map, &root.join("src-old")), Some(2));
        assert_eq!(worst(&map, &root), Some(1));
        assert_eq!(worst(&map, &root.join("sr")), None);
        assert_eq!(worst(&map, &root.join("src").join("other.cpp")), None);
        map.remove(&key(&main));
        assert_eq!((worst(&map, &root.join("src")), worst(&map, &root)), (None, Some(2)));
    }

    #[test]
    fn sends_workspace_folders_and_their_changes() {
        let (api, web) = (PathBuf::from("/w/api"), PathBuf::from("/w/web"));
        let two = [(api.clone(), "api".to_owned()), (web.clone(), "web".to_owned())];
        let init = initialize(1, &web, &two);
        assert_eq!(init["params"]["rootUri"], json!(uri(&web)));
        assert_eq!(init["params"]["workspaceFolders"], json!([{"uri": uri(&api), "name": "api"}, {"uri": uri(&web), "name": "web"}]));
        assert_eq!(initialize(1, &api, &[])["params"]["workspaceFolders"], json!([{"uri": uri(&api), "name": "api"}]));
        let change = folders_changed(&two[..1], &two[1..]).unwrap();
        assert_eq!(change, json!({"event": {"added": [{"uri": uri(&web), "name": "web"}], "removed": [{"uri": uri(&api), "name": "api"}]}}));
        assert!(folders_changed(&two, &two).is_none());
        let mut lsp = Lsp::default();
        lsp.set_folders(two.to_vec());
        assert_eq!(workspace_folders(&lsp.folders).as_array().unwrap().len(), 2);
    }

    #[test]
    fn maps_paths_positions_and_frames() {
        let path = if cfg!(windows) { PathBuf::from(r"C:\Users\a b\é.rs") } else { PathBuf::from("/home/a b/é.rs") };
        let target = uri(&path);
        assert!(target.starts_with("file:///") && target.contains("a%20b") && target.ends_with("%C3%A9.rs"));
        assert_eq!(path_of(&target), Some(path.clone()));
        if cfg!(windows) {
            assert_eq!(path_of("file:///c%3A/Users/a%20b/%C3%A9.rs"), Some(path.clone()));
            assert_eq!(key(Path::new("C:/Users/X.rs")), r"c:\users\x.rs");
        }
        assert_eq!(utf16_to_char("a😀b".chars(), 3), 2);
        assert_eq!(char_to_utf16("a😀b".chars(), 2), 3);
        assert_eq!(utf16_to_byte("a😀b", 3), 5);
        assert_eq!((shift(3, 5, 2), shift(6, 5, 2), shift(6, 5, -1), shift(8, 5, -1)), (Some(3), Some(8), None, Some(7)));

        let message = json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": "é"}});
        let wire = format!("{}{}", frame(&message), frame(&json!({"method": "x"})));
        let mut reader = BufReader::new(wire.as_bytes());
        assert_eq!(read_message(&mut reader), Some(message));
        assert_eq!(read_message(&mut reader).unwrap()["method"], "x");
        assert_eq!(read_message(&mut reader), None);
    }

    #[test]
    fn parses_server_results() {
        let target = uri(&std::env::temp_dir().join("a.rs"));
        let span = json!({"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 7}});
        let found = locations(&json!([{"uri": target, "range": span}, {"targetUri": target, "targetSelectionRange": span, "targetRange": span}]));
        assert_eq!((found.len(), found[1].start, found[1].end), (2, (2, 4), (2, 7)));
        assert_eq!(locations(&json!({"uri": target, "range": span})).len(), 1);
        assert!(locations(&Value::Null).is_empty());

        let edit = workspace_edit(&json!({"changes": {target.clone(): [{"range": span, "newText": "x\r\ny"}]}}));
        assert_eq!(edit[0].1, vec![Edit { start: (2, 4), end: (2, 7), text: "x\ny".into() }]);
        let edit = workspace_edit(&json!({"documentChanges": [{"textDocument": {"uri": target}, "edits": [{"range": span, "newText": "z"}]}]}));
        assert_eq!(edit[0].1[0].text, "z");

        let (items, incomplete) = completions(&json!({"isIncomplete": true, "items": [
            {"label": "push", "kind": 2, "detail": "fn(T)", "insertText": "push($1)", "insertTextFormat": 2},
            {"label": "len", "textEdit": {"range": span, "newText": "len()"}, "additionalTextEdits": [{"range": span, "newText": "use x;"}]},
        ]}));
        assert!(incomplete);
        assert_eq!((items[0].text.as_str(), items[0].detail.as_str(), items[0].range), ("push($1)", "fn(T)", None));
        assert!(items[0].snippet && !items[1].snippet);
        assert_eq!((items[1].text.as_str(), items[1].range, items[1].extra.len()), ("len()", Some(((2, 4), (2, 7))), 1));

        let text = hover(&json!({"contents": {"kind": "markdown", "value": "```rust\nfn a()\n```\n# Title\nSome **bold** `code`"}}));
        assert_eq!(blocks(&text), vec![(true, "fn a()".to_owned()), (false, "Title\nSome bold code".to_owned())]);
        assert_eq!(hover(&json!({"contents": ["plain", {"language": "c", "value": "int x"}]})), "plain\n\n```c\nint x\n```");

        let help = json!({"activeParameter": 1, "signatures": [{"label": "f(a: i32, b: u8)", "parameters": [{"label": "a: i32"}, {"label": [10, 15]}]}]});
        assert_eq!(signature(&help).unwrap().param, Some((10, 15)));
        assert_eq!(signature(&json!({"signatures": []})), None);

        let tree = json!([{"name": "Foo", "kind": 23, "range": {"start": {"line": 1, "character": 0}, "end": {"line": 9, "character": 1}},
            "selectionRange": {"start": {"line": 1, "character": 7}, "end": {"line": 1, "character": 10}},
            "children": [{"name": "bar", "kind": 6, "range": span, "selectionRange": span}]}]);
        let found = symbols(&tree);
        assert_eq!((found[0].line, found[0].col, found[0].end_line, found[0].depth), (1, 7, 9, 0));
        assert_eq!((found[1].name.as_str(), found[1].depth, found[1].container.as_str()), ("bar", 1, "Foo"));
        let flat = symbols(&json!([{"name": "x", "kind": 12, "containerName": "m", "location": {"uri": target, "range": span}}]));
        assert_eq!((flat[0].line, flat[0].container.as_str(), flat[0].path.is_some()), (2, "m", true));

        assert_eq!(tokens(&json!({"data": [1, 2, 3, 0, 0, 0, 4, 2, 5, 0, 2, 1, 1, 6, 0]})), vec![(1, 2, 3, 0), (1, 6, 2, 5), (3, 1, 1, 6)]);
        let found = hints(&json!([{"position": {"line": 4, "character": 9}, "label": [{"value": ": "}, {"value": "u8"}], "paddingLeft": true},
            {"position": {"line": 1, "character": 2}, "label": "x:", "paddingRight": true}]));
        assert_eq!(found, vec![Hint { line: 1, col: 2, label: "x: ".into() }, Hint { line: 4, col: 9, label: " : u8".into() }]);
        let item = diagnostic(&json!({"range": span, "severity": 2, "code": "E1", "source": "rustc", "message": "unused"}));
        assert_eq!((item.severity, item.source.as_str(), item.start), (2, "rustc(E1)", (2, 4)));
        assert_eq!(hierarchy(&json!([{"from": {"name": "caller", "uri": target, "selectionRange": span}, "fromRanges": []}]))[0].label, "caller");
        assert_eq!(highlights(&json!([{"range": span, "kind": 3}, {"range": span}])), vec![((2, 4), (2, 7)); 2]);
        assert!(highlights(&Value::Null).is_empty());
    }

    #[test]
    fn parses_code_lenses_runnables_and_quick_fixes() {
        let at = |line| json!({"start": {"line": line, "character": 0}, "end": {"line": line, "character": 3}});
        let run = json!({"label": "test t", "kind": "cargo", "args": {"workspaceRoot": "/w", "cwd": "/w/crate", "cargoArgs": ["test", "--package", "a b"], "executableArgs": ["tests::t", "--exact", "--show-output"]}});
        let found = code_lenses(&json!([
            {"range": at(7), "data": {"impls": 1}},
            {"range": at(2), "command": {"title": "Run Test", "command": "rust-analyzer.runSingle", "arguments": [run]}},
        ]));
        assert_eq!(found.iter().map(|l| (l.line, l.title.as_str(), l.command.as_str())).collect::<Vec<_>>(), vec![(2, "Run Test", "rust-analyzer.runSingle"), (7, "", "")]);
        assert_eq!((found[1].args.clone(), found[1].raw["data"]["impls"].as_u64()), (Value::Null, Some(1)));
        assert!(code_lenses(&Value::Null).is_empty());
        let resolved = code_lens(&json!({"range": at(7), "command": {"title": "2 implementations", "command": "rust-analyzer.showReferences", "arguments": ["u", {"line": 7, "character": 0}, []]}}));
        assert_eq!((resolved.title.as_str(), pos(&resolved.args[1])), ("2 implementations", (7, 0)));

        let (line, cwd, test) = runnable(&found[0].args[0]).unwrap();
        assert_eq!(line, "cargo test --package \"a b\" -- tests::t --exact --show-output");
        assert_eq!((cwd, test.as_deref()), (Some(PathBuf::from("/w/crate")), Some("tests::t")));
        let bin = json!({"kind": "cargo", "args": {"workspaceRoot": "/w", "cargoArgs": ["run", "--bin", "a"], "executableArgs": [], "overrideCargo": "cross"}});
        assert_eq!(runnable(&bin), Some(("cross run --bin a".into(), Some(PathBuf::from("/w")), None)));
        let module = json!({"kind": "cargo", "args": {"cargoArgs": ["test"], "executableArgs": ["tests", "--show-output"]}});
        assert_eq!(runnable(&module).unwrap().2, None);
        assert_eq!(runnable(&json!({"kind": "shell", "args": {"program": "x"}})), None);

        let action = |kind: &str, preferred: bool| ("a".to_owned(), json!({"title": "a", "kind": kind, "isPreferred": preferred}));
        assert!(quick_fix(&[action("refactor", false), action("quickfix", false)]));
        assert!(quick_fix(&[action("quickfix.rust", false)]) && quick_fix(&[action("refactor.extract", true)]));
        assert!(!quick_fix(&[action("refactor.inline", false), action("source", false), ("c".into(), json!({"title": "c", "command": "x"}))]));
    }

    #[test]
    #[ignore = "needs clangd inside the Ubuntu-24.04 WSL distro"]
    fn talks_to_clangd_in_wsl() {
        let root = PathBuf::from(r"\\wsl.localhost\Ubuntu-24.04\tmp\halo-lsp");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("a.cpp");
        let text = "int main() { int x = undefined_name; }\n".to_owned();
        std::fs::write(&path, &text).unwrap();
        let mut lsp = Lsp::default();
        lsp.configure(&Settings::default(), Some(&root));
        assert!(lsp.open(&path, || text.clone(), &egui::Context::default()));
        let started = std::time::Instant::now();
        while lsp.diagnostics(&path).is_empty() {
            assert!(started.elapsed().as_secs() < 30, "no diagnostics: {:?}", lsp.log);
            lsp.poll();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(lsp.diagnostics(&path).iter().any(|d| d.message.contains("undefined_name")));
        let text = "#include <vector>\nvoid f(std::vector<int> v) { v.si }\n".to_owned();
        lsp.change(&path, text);
        while lsp.diagnostics(&path).iter().any(|d| d.message.contains("undefined_name")) {
            assert!(started.elapsed().as_secs() < 60, "no reparse: {:?}", lsp.log);
            lsp.poll();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let id = lsp.request(&path, "textDocument/completion", at(&path, (1, 33))).unwrap();
        let reply = loop {
            assert!(started.elapsed().as_secs() < 60, "no completion: {:?}", lsp.log);
            lsp.poll();
            if let Some(reply) = lsp.take(id) {
                break reply.unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let size = reply["items"].as_array().unwrap().iter().find(|i| i["filterText"] == "size").unwrap_or_else(|| panic!("{reply}")).clone();
        assert!(size["textEdit"]["newText"].as_str().unwrap().starts_with("size("), "{size}");
    }

    #[test]
    #[ignore = "needs rust-analyzer on PATH"]
    fn talks_to_rust_analyzer() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("src").join("lsp.rs");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut lsp = Lsp::default();
        lsp.configure(&Settings::default(), Some(root));
        assert!(lsp.open(&path, || text.clone(), &egui::Context::default()));
        let id = lsp.request(&path, "textDocument/documentSymbol", json!({"textDocument": {"uri": uri(&path)}})).unwrap();
        let started = std::time::Instant::now();
        let reply = loop {
            lsp.poll();
            if let Some(reply) = lsp.take(id) {
                break reply;
            }
            assert!(started.elapsed().as_secs() < 120, "{:?}", lsp.log);
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let found = symbols(&reply.unwrap());
        assert!(found.iter().any(|s| s.name == "Lsp" && s.kind == 23), "{found:?}");
        assert!(lsp.status().is_some() && !lsp.triggers(&path, "completionProvider").is_empty());
    }
}
