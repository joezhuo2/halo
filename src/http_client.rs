use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

const METHODS: [&str; 9] = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE", "CONNECT"];

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub body_file: Option<PathBuf>,
}

pub fn is_http(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("http") || e.eq_ignore_ascii_case("rest"))
}

fn separator(line: &str) -> bool {
    line.trim_start().starts_with("###")
}

fn comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') || t.starts_with("//")
}

fn variable(line: &str) -> Option<(String, String)> {
    let (name, value) = line.trim().strip_prefix('@')?.split_once('=')?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| (name.to_owned(), value.trim().to_owned()))
}

fn blocks(text: &str) -> Vec<(usize, Vec<&str>)> {
    let mut out = vec![(0, vec![])];
    for (i, line) in text.lines().enumerate() {
        if separator(line) {
            out.push((i + 1, vec![]));
        } else {
            out.last_mut().unwrap().1.push(line);
        }
    }
    out
}

fn request_index(lines: &[&str]) -> Option<usize> {
    lines.iter().position(|l| !l.trim().is_empty() && !comment(l) && variable(l).is_none())
}

pub fn request_lines(text: &str) -> Vec<usize> {
    blocks(text).into_iter().filter_map(|(start, lines)| Some(start + request_index(&lines)?)).collect()
}

fn substitute(text: &str, vars: &HashMap<String, String>, depth: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find("}}") else { break };
        let name = rest[open + 2..open + close].trim();
        let value = match name.split_once(' ').map_or((name, ""), |(a, b)| (a, b.trim())) {
            ("$guid", _) => Some(guid()),
            ("$timestamp", _) => Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string()),
            ("$processEnv", var) => Some(std::env::var(var).unwrap_or_default()),
            _ => vars.get(name).map(|v| if depth < 8 { substitute(v, vars, depth + 1) } else { v.clone() }),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &rest[open + close + 2..];
    }
    out.push_str(rest);
    out
}

fn guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for half in bytes.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        half.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

pub fn parse(text: &str, line: usize, dir: Option<&Path>) -> Result<Request, String> {
    let vars: HashMap<String, String> = text.lines().filter_map(variable).collect();
    let blocks = blocks(text);
    let (_, lines) = blocks.iter().rev().find(|b| b.0 <= line).ok_or("No request here.")?;
    let at = request_index(lines).ok_or("No request under the cursor. Write a line like `GET https://example.com`.")?;
    let first = substitute(lines[at].trim(), &vars, 0);
    let mut words = first.split_whitespace();
    let head = words.next().unwrap_or_default();
    let (method, mut url) = match METHODS.iter().find(|m| m.eq_ignore_ascii_case(head)) {
        Some(m) => (m.to_string(), words.next().ok_or("The request line has no URL.")?.to_owned()),
        None => ("GET".to_owned(), head.to_owned()),
    };
    let mut i = at + 1;
    while let Some(l) = lines.get(i).map(|l| l.trim()).filter(|l| l.starts_with('?') || l.starts_with('&')) {
        url.push_str(&substitute(l, &vars, 0));
        i += 1;
    }
    let mut headers = vec![];
    while let Some(l) = lines.get(i).filter(|l| !l.trim().is_empty()) {
        i += 1;
        if comment(l) { continue }
        let (name, value) = l.split_once(':').ok_or_else(|| format!("Expected a header `Name: value`, found `{}`.", l.trim()))?;
        headers.push((name.trim().to_owned(), substitute(value.trim(), &vars, 0)));
    }
    let mut body: Vec<&str> = lines.get(i + 1..).unwrap_or_default().to_vec();
    while body.last().is_some_and(|l| l.trim().is_empty()) { body.pop(); }
    let body_file = match body.as_slice() {
        [only] if only.trim_start().starts_with("< ") => Some(dir.unwrap_or(Path::new(".")).join(only.trim_start()[2..].trim())),
        _ => None,
    };
    let body = if body_file.is_some() { String::new() } else { substitute(&body.join("\n"), &vars, 0) };
    Ok(Request { method, url, headers, body, body_file })
}

fn args(req: &Request, has_body: bool) -> Vec<String> {
    let mut out: Vec<String> = ["-sS", "-i", "--max-time", "60", "-w", "\n%{time_total}", "-H", "Expect:"].map(String::from).to_vec();
    match req.method.as_str() {
        "HEAD" => out.push("--head".into()),
        m => out.extend(["-X".into(), m.into()]),
    }
    for (name, value) in &req.headers {
        out.extend(["-H".into(), format!("{name}: {value}")]);
    }
    if has_body {
        out.extend(["--data-binary".into(), "@-".into()]);
    }
    out.extend(["--url".into(), req.url.clone()]);
    out
}

pub struct Response {
    pub title: String,
    pub text: String,
    pub syntax: &'static str,
}

pub fn format(raw: &str) -> Response {
    let raw = raw.replace("\r\n", "\n");
    let (raw, time) = raw.rsplit_once('\n').unwrap_or((&raw, ""));
    let ms = time.trim().parse::<f64>().map(|s| (s * 1000.0).round() as u64);
    let mut rest = raw;
    let (head, body) = loop {
        let (head, body) = rest.split_once("\n\n").unwrap_or((rest, ""));
        let interim = head.split_whitespace().nth(1).is_some_and(|code| code.starts_with('1') || code.starts_with('3'));
        if interim && body.starts_with("HTTP/") { rest = body; continue }
        break (head, body);
    };
    let kind = head.lines().find_map(|l| l.split_once(':').filter(|h| h.0.eq_ignore_ascii_case("content-type")).map(|h| h.1.to_ascii_lowercase())).unwrap_or_default();
    let syntax = if kind.contains("json") { "response.json" } else if kind.contains("html") { "response.html" } else if kind.contains("xml") { "response.xml" } else { "response.txt" };
    let pretty = (syntax == "response.json").then(|| serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| serde_json::to_string_pretty(&v).ok())).flatten();
    let title = match ms { Ok(ms) => format!("Response ({ms} ms)"), Err(_) => "Response".into() };
    Response { title, text: format!("{head}\n\n{}\n", pretty.as_deref().unwrap_or(body)), syntax }
}

type Held = Arc<Mutex<Option<Child>>>;

#[derive(Default)]
pub struct Client {
    pub last: Option<Request>,
    running: Option<(Receiver<Response>, Held)>,
}

impl Client {
    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    pub fn send(&mut self, req: Request, ctx: &eframe::egui::Context) {
        self.cancel();
        let body = match &req.body_file {
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) => return self.fail(format!("Could not read {}: {e}", path.display()), ctx),
            },
            None => req.body.clone().into_bytes(),
        };
        let mut command = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
        command.args(args(&req, !body.is_empty())).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        self.last = Some(req);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return self.fail("curl was not found on PATH. Install curl to send requests.".into(), ctx),
            Err(e) => return self.fail(format!("Could not start curl: {e}"), ctx),
        };
        let (mut stdin, mut stdout, mut stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let child = Arc::new(Mutex::new(Some(child)));
        let (tx, rx) = channel();
        let (held, ctx) = (child.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Some(mut stdin) = stdin.take() {
                let _ = stdin.write_all(&body);
            }
            let (mut out, mut err) = (vec![], String::new());
            if let Some(s) = stdout.as_mut() { let _ = s.read_to_end(&mut out); }
            if let Some(s) = stderr.as_mut() { let _ = s.read_to_string(&mut err); }
            let Some(mut child) = held.lock().unwrap().take() else { return };
            let ok = child.wait().is_ok_and(|s| s.success());
            let response = match ok {
                true => format(&String::from_utf8_lossy(&out)),
                false => Response { title: "Response".into(), text: format!("Request failed.\n\n{}\n", err.trim()), syntax: "response.txt" },
            };
            let _ = tx.send(response);
            ctx.request_repaint();
        });
        self.running = Some((rx, child));
    }

    fn fail(&mut self, text: String, ctx: &eframe::egui::Context) {
        let (tx, rx) = channel();
        let _ = tx.send(Response { title: "Response".into(), text: format!("{text}\n"), syntax: "response.txt" });
        self.running = Some((rx, Arc::new(Mutex::new(None))));
        ctx.request_repaint();
    }

    pub fn cancel(&mut self) {
        if let Some((_, child)) = self.running.take()
            && let Some(mut child) = child.lock().unwrap().take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn poll(&mut self) -> Option<Response> {
        let response = self.running.as_ref()?.0.try_recv().ok()?;
        self.running = None;
        Some(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "@host = https://api.test\n@token = abc\n\n### list\n# @name list\nGET {{host}}/items\n  ?page=2\n  &size=10\nAuthorization: Bearer {{token}}\n\n###\n\nPOST {{host}}/items HTTP/1.1\nContent-Type: application/json\n\n{\"name\": \"{{missing}}\"}\n\n\n###\nhttps://plain.test\n###\nPUT /upload\n\n< ./body.json\n";

    #[test]
    fn finds_request_lines_per_block() {
        assert_eq!(request_lines(FILE), [5, 12, 19, 21]);
        assert_eq!(request_lines("### only a comment\n# hi\n"), Vec::<usize>::new());
    }

    #[test]
    fn parses_variables_query_continuations_headers_and_bodies() {
        let get = parse(FILE, 6, None).unwrap();
        assert_eq!((get.method.as_str(), get.url.as_str()), ("GET", "https://api.test/items?page=2&size=10"));
        assert_eq!(get.headers, [("Authorization".to_owned(), "Bearer abc".to_owned())]);
        assert!(get.body.is_empty());
        let post = parse(FILE, 14, None).unwrap();
        assert_eq!((post.method.as_str(), post.url.as_str()), ("POST", "https://api.test/items"));
        assert_eq!(post.body, "{\"name\": \"{{missing}}\"}");
        assert_eq!(parse(FILE, 19, None).unwrap().url, "https://plain.test");
        let put = parse(FILE, 22, Some(Path::new("dir"))).unwrap();
        assert_eq!(put.body_file, Some(Path::new("dir").join("./body.json")));
        assert!(parse("GET\n", 0, None).is_err());
        assert!(parse("GET https://x\nnot a header\n", 0, None).is_err());
    }

    #[test]
    fn dynamic_variables() {
        let vars = HashMap::from([("a".to_owned(), "{{b}}".to_owned()), ("b".to_owned(), "two".to_owned())]);
        assert_eq!(substitute("x{{a}}y{{ $processEnv HALO_NOT_SET_ANYWHERE }}z", &vars, 0), "xtwoyz");
        let id = substitute("{{$guid}}", &vars, 0);
        assert_eq!((id.len(), id.as_bytes()[14]), (36, b'4'));
        assert!(substitute("{{$timestamp}}", &vars, 0).parse::<u64>().is_ok());
    }

    #[test]
    fn curl_arguments_never_let_the_url_be_a_flag() {
        let req = Request { method: "HEAD".into(), url: "-o/etc/x".into(), headers: vec![("A".into(), "b".into())], body: String::new(), body_file: None };
        let args = args(&req, false);
        assert_eq!(&args[args.len() - 2..], ["--url", "-o/etc/x"]);
        assert!(args.contains(&"--head".to_owned()) && args.contains(&"A: b".to_owned()) && !args.contains(&"@-".to_owned()));
    }

    #[test]
    fn sends_through_curl_to_a_local_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut got = vec![0; 4096];
            let mut seen = String::new();
            while !seen.contains("hello") {
                let n = stream.read(&mut got).unwrap();
                if n == 0 { break }
                seen.push_str(&String::from_utf8_lossy(&got[..n]));
            }
            stream.write_all(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\n{\"ok\":1}").unwrap();
            seen
        });
        let text = format!("@port = {port}\nPOST http://127.0.0.1:{{{{port}}}}/x\nX-Test: yes\n\nhello\n");
        let mut client = Client::default();
        client.send(parse(&text, 1, None).unwrap(), &eframe::egui::Context::default());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let response = loop {
            if let Some(r) = client.poll() { break r }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /x HTTP/1.1") && seen.contains("X-Test: yes") && !seen.contains("Expect:"), "{seen}");
        assert!(response.text.starts_with("HTTP/1.1 201 Created") && response.text.contains("\"ok\": 1"), "{}", response.text);
        assert!(response.title.starts_with("Response (") && !client.running());
    }

    #[test]
    fn formats_responses() {
        let r = format("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}\n0.123456");
        assert_eq!(r.title, "Response (123 ms)");
        assert_eq!(r.syntax, "response.json");
        assert_eq!(r.text, "HTTP/1.1 200 OK\nContent-Type: application/json\n\n{\n  \"a\": 1\n}\n");
        let r = format("HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n<p>no</p>\n0.5");
        assert_eq!((r.syntax, r.text.as_str()), ("response.html", "HTTP/1.1 404 Not Found\nContent-Type: text/html\n\n<p>no</p>\n"));
    }
}
