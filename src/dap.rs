use eframe::egui;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Response { request: i64, command: String, success: bool, message: String, body: Value },
    Event(String, Value),
    Request { seq: i64, command: String, args: Value },
    Stderr(String),
    Closed(Option<String>),
}

fn error_text(message: &Value) -> String {
    let error = &message["body"]["error"];
    let Some(format) = error["format"].as_str() else { return message["message"].as_str().unwrap_or_default().to_owned() };
    let mut text = format.to_owned();
    for (name, value) in error["variables"].as_object().into_iter().flatten() {
        text = text.replace(&format!("{{{name}}}"), value.as_str().unwrap_or_default());
    }
    text
}

pub fn parse(message: &Value) -> Option<Incoming> {
    let text = |key: &str| message[key].as_str().unwrap_or_default().to_owned();
    Some(match message["type"].as_str()? {
        "response" => Incoming::Response {
            request: message["request_seq"].as_i64()?,
            command: text("command"),
            success: message["success"] == true,
            message: error_text(message),
            body: message["body"].clone(),
        },
        "event" => Incoming::Event(text("event"), message["body"].clone()),
        "request" => Incoming::Request { seq: message["seq"].as_i64()?, command: text("command"), args: message["arguments"].clone() },
        _ => return None,
    })
}

fn write_loop(mut writer: impl Write, out: Receiver<String>) {
    for text in out {
        if writer.write_all(text.as_bytes()).and_then(|_| writer.flush()).is_err() {
            break;
        }
    }
}

fn read_loop(reader: impl Read, tx: Sender<Incoming>, ctx: egui::Context) {
    let mut reader = BufReader::new(reader);
    while let Some(message) = crate::lsp::read_message(&mut reader) {
        if let Some(incoming) = parse(&message) {
            if tx.send(incoming).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
    let _ = tx.send(Incoming::Closed(None));
    ctx.request_repaint();
}

fn lines_loop(reader: impl Read + Send + 'static, tx: Sender<Incoming>) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if tx.send(Incoming::Stderr(line)).is_err() {
                break;
            }
        }
    });
}

fn tcp(port: u16, out: Receiver<String>, tx: Sender<Incoming>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    let _ = tx.send(Incoming::Closed(Some(format!("Could not connect to the debug adapter on port {port}: {error}"))));
                    return ctx.request_repaint();
                }
            }
        };
        match stream.try_clone() {
            Ok(writer) => drop(std::thread::spawn(move || write_loop(writer, out))),
            Err(error) => {
                let _ = tx.send(Incoming::Closed(Some(error.to_string())));
                return ctx.request_repaint();
            }
        }
        read_loop(stream, tx, ctx);
    });
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())
}

pub struct Client {
    tx: Sender<String>,
    rx: Receiver<Incoming>,
    seq: i64,
    child: Option<Child>,
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(1));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

impl Client {
    #[cfg(test)]
    fn over(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static, ctx: &egui::Context) -> Self {
        let (tx, out) = channel();
        let (in_tx, rx) = channel();
        std::thread::spawn(move || write_loop(writer, out));
        let ctx = ctx.clone();
        std::thread::spawn(move || read_loop(reader, in_tx, ctx));
        Self { tx, rx, seq: 0, child: None }
    }

    pub fn connect(port: u16, ctx: &egui::Context) -> Self {
        let (tx, out) = channel();
        let (in_tx, rx) = channel();
        tcp(port, out, in_tx, ctx.clone());
        Self { tx, rx, seq: 0, child: None }
    }

    pub fn spawn(command: &[String], cwd: Option<&Path>, ctx: &egui::Context) -> Result<Self, String> {
        let (name, args) = command.split_first().ok_or("The debug adapter command is empty.")?;
        let program = crate::lsp::find_program(name).ok_or_else(|| format!("The debug adapter {name} was not found on PATH."))?;
        let port = command.iter().any(|a| a.contains("{port}")).then(free_port).transpose()?;
        let mut process = crate::terminal::hidden_command(&program.to_string_lossy());
        process.args(args.iter().map(|a| port.map_or(a.clone(), |p| a.replace("{port}", &p.to_string()))));
        process.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(cwd) = cwd.filter(|c| c.is_dir()) {
            process.current_dir(cwd);
        }
        let mut child = process.spawn().map_err(|e| format!("The debug adapter {name} could not be started: {e}"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            return Err(format!("The pipes of the debug adapter {name} could not be opened."));
        };
        let (tx, out) = channel();
        let (in_tx, rx) = channel();
        lines_loop(stderr, in_tx.clone());
        match port {
            Some(port) => {
                drop(stdin);
                lines_loop(stdout, in_tx.clone());
                tcp(port, out, in_tx, ctx.clone());
            }
            None => {
                std::thread::spawn(move || write_loop(stdin, out));
                let ctx = ctx.clone();
                std::thread::spawn(move || read_loop(stdout, in_tx, ctx));
            }
        }
        Ok(Self { tx, rx, seq: 0, child: Some(child) })
    }

    fn send(&mut self, mut message: Value) -> i64 {
        self.seq += 1;
        message["seq"] = json!(self.seq);
        let _ = self.tx.send(crate::lsp::frame(&message));
        self.seq
    }

    pub fn request(&mut self, command: &str, args: Value) -> i64 {
        self.send(json!({"type": "request", "command": command, "arguments": args}))
    }

    pub fn respond(&mut self, request: i64, command: &str, result: Result<Value, String>) {
        let message = match result {
            Ok(body) => json!({"type": "response", "request_seq": request, "command": command, "success": true, "body": body}),
            Err(text) => json!({"type": "response", "request_seq": request, "command": command, "success": false, "message": text}),
        };
        self.send(message);
    }

    pub fn poll(&self) -> Vec<Incoming> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn frames(bytes: &[u8]) -> Vec<Value> {
        let mut reader = BufReader::new(bytes);
        std::iter::from_fn(|| crate::lsp::read_message(&mut reader)).collect()
    }

    fn wait(client: &Client, count: usize) -> Vec<Incoming> {
        let mut got = vec![];
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < count && Instant::now() < deadline {
            got.extend(client.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn frames_round_trip_with_multibyte_bodies() {
        let first = json!({"seq": 1, "type": "event", "event": "output", "body": {"output": "héllo ✓\n"}});
        let second = json!({"seq": 2, "type": "response", "request_seq": 5, "command": "threads", "success": true, "body": {"threads": []}});
        let text = crate::lsp::frame(&first) + "Content-Type: application/vscode-jsonrpc\r\n" + &crate::lsp::frame(&second);
        assert_eq!(frames(text.as_bytes()), vec![first.clone(), second]);
        let body = first.to_string();
        assert!(crate::lsp::frame(&first).starts_with(&format!("Content-Length: {}\r\n\r\n", body.len())));
        assert!(body.len() > body.chars().count());
    }

    #[test]
    fn parses_responses_events_and_reverse_requests() {
        let failed = json!({"type": "response", "request_seq": 3, "command": "launch", "success": false, "message": "x",
            "body": {"error": {"id": 1, "format": "Cannot find {file}", "variables": {"file": "a.out"}}}});
        assert_eq!(parse(&failed), Some(Incoming::Response {
            request: 3,
            command: "launch".into(),
            success: false,
            message: "Cannot find a.out".into(),
            body: failed["body"].clone(),
        }));
        let plain = json!({"type": "response", "request_seq": 4, "command": "next", "success": false, "message": "notStopped"});
        assert!(matches!(parse(&plain), Some(Incoming::Response { message, .. }) if message == "notStopped"));
        let event = json!({"type": "event", "seq": 9, "event": "stopped", "body": {"threadId": 1}});
        assert_eq!(parse(&event), Some(Incoming::Event("stopped".into(), json!({"threadId": 1}))));
        let request = json!({"type": "request", "seq": 7, "command": "runInTerminal", "arguments": {"args": ["a"]}});
        assert_eq!(parse(&request), Some(Incoming::Request { seq: 7, command: "runInTerminal".into(), args: json!({"args": ["a"]}) }));
        assert_eq!(parse(&json!({"type": "other"})), None);
        assert_eq!(parse(&json!({"seq": 1})), None);
    }

    #[test]
    fn numbers_requests_and_correlates_replies() {
        let replies = [
            json!({"seq": 1, "type": "response", "request_seq": 2, "command": "threads", "success": true, "body": {"threads": [{"id": 1, "name": "main"}]}}),
            json!({"seq": 2, "type": "event", "event": "initialized"}),
            json!({"seq": 3, "type": "response", "request_seq": 1, "command": "initialize", "success": true, "body": {"supportsSetVariable": true}}),
        ];
        let input: String = replies.iter().map(crate::lsp::frame).collect();
        let written = Shared::default();
        let mut client = Client::over(std::io::Cursor::new(input.into_bytes()), written.clone(), &egui::Context::default());
        assert_eq!(client.request("initialize", json!({"adapterID": "x"})), 1);
        assert_eq!(client.request("threads", Value::Null), 2);
        client.respond(7, "runInTerminal", Ok(json!({"processId": 42})));
        let got = wait(&client, 4);
        assert!(matches!(&got[0], Incoming::Response { request: 2, command, success: true, .. } if command == "threads"));
        assert_eq!(got[1], Incoming::Event("initialized".into(), Value::Null));
        assert!(matches!(&got[2], Incoming::Response { request: 1, body, .. } if body["supportsSetVariable"] == true));
        assert_eq!(got[3], Incoming::Closed(None));
        let deadline = Instant::now() + Duration::from_secs(5);
        while frames(&written.0.lock().unwrap()).len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let sent = frames(&written.0.lock().unwrap());
        assert_eq!(sent[0], json!({"seq": 1, "type": "request", "command": "initialize", "arguments": {"adapterID": "x"}}));
        assert_eq!((sent[1]["seq"].clone(), sent[1]["command"].clone()), (json!(2), json!("threads")));
        assert_eq!(sent[2], json!({"seq": 3, "type": "response", "request_seq": 7, "command": "runInTerminal", "success": true, "body": {"processId": 42}}));
    }

    #[test]
    fn missing_adapters_name_the_command() {
        let error = Client::spawn(&["halo-no-such-adapter".into()], None, &egui::Context::default()).err().unwrap();
        assert_eq!(error, "The debug adapter halo-no-such-adapter was not found on PATH.");
        assert!(Client::spawn(&[], None, &egui::Context::default()).is_err());
    }
}
