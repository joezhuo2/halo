use super::{Live, Marks, State, Want};
use crate::app::Halo;
use crate::ui;
use eframe::egui::{self, TextEdit};
use serde_json::{Value, json};
use std::fmt::Write;

pub const DISASSEMBLY: &str = "Disassembly";
pub const MEMORY: &str = "Memory";
const BEFORE: i64 = 50;
const BYTES: u64 = 1024;

#[derive(Default)]
pub struct Views {
    disasm: bool,
    memory: Option<(String, String)>,
    seen: Option<(i64, Option<String>)>,
    asking: Option<(String, bool)>,
    pub(super) out: Vec<(&'static str, String, Option<usize>, bool)>,
    pub focused: bool,
}

pub fn disassemble_args(ip: &str) -> Value {
    json!({"memoryReference": ip, "offset": 0, "instructionOffset": -BEFORE, "instructionCount": 200, "resolveSymbols": true})
}

pub fn read_args(reference: &str) -> Value {
    json!({"memoryReference": reference, "offset": 0, "count": BYTES})
}

pub fn address(text: &str) -> Option<u64> {
    let text = text.trim();
    u64::from_str_radix(text.strip_prefix("0x").or(text.strip_prefix("0X"))?, 16).ok()
}

pub fn base64(text: &str) -> Option<Vec<u8>> {
    let (mut out, mut acc, mut bits) = (vec![], 0u32, 0);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        (acc, bits) = ((acc << 6 | value as u32) & 0xff_ffff, bits + 6);
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

pub fn hex_dump(base: u64, data: &[u8], unreadable: usize) -> String {
    let total = data.len() + unreadable;
    let width = if base.saturating_add(total as u64) > u32::MAX as u64 { 16 } else { 8 };
    let mut out = String::new();
    for row in (0..total).step_by(16) {
        let (mut hex, mut text) = (String::new(), String::new());
        for i in row..row + 16 {
            match (data.get(i), i < total) {
                (Some(b), _) => write!(hex, "{b:02X} ").unwrap(),
                (None, true) => hex.push_str("?? "),
                (None, false) => hex.push_str("   "),
            }
            if i == row + 7 {
                hex.push(' ');
            }
            match data.get(i) {
                Some(b) if b.is_ascii_graphic() || *b == b' ' => text.push(*b as char),
                Some(_) => text.push('.'),
                None if i < total => text.push('?'),
                None => {}
            }
        }
        writeln!(out, "{:0width$X}  {hex} {text}", base + row as u64).unwrap();
    }
    out
}

pub fn disasm_text(body: &Value, ip: &str) -> (String, usize) {
    let list = body["instructions"].as_array().map_or(&[] as &[Value], Vec::as_slice);
    let text = |item: &Value, key: &str| item[key].as_str().unwrap_or_default().to_owned();
    let address_width = list.iter().map(|i| text(i, "address").len()).max().unwrap_or(0);
    let bytes_width = list.iter().map(|i| text(i, "instructionBytes").len()).max().unwrap_or(0);
    let (mut out, mut lines, mut current) = (String::new(), vec![], None);
    let (mut symbol, mut file, mut place) = (String::new(), String::new(), String::new());
    for item in list {
        let name = text(item, "symbol");
        if !name.is_empty() && name != symbol {
            writeln!(out, "{name}:").unwrap();
            lines.push(usize::MAX);
            symbol = name;
        }
        let line = lines.len();
        lines.push(line);
        if current.is_none() && address(&text(item, "address")).is_some_and(|a| Some(a) == address(ip)) {
            current = Some(line);
        }
        let source = &item["location"];
        if let Some(path) = source["path"].as_str().or(source["name"].as_str()) {
            file = std::path::Path::new(path).file_name().map_or(path.to_owned(), |n| n.to_string_lossy().into_owned());
        }
        let here = match item["line"].as_u64() {
            Some(line) if !file.is_empty() => format!("{file}:{line}"),
            _ => String::new(),
        };
        let at = if !here.is_empty() && here != place { format!("  ; {here}") } else { String::new() };
        place = here;
        let bytes = if bytes_width > 0 { format!("{:bytes_width$}  ", text(item, "instructionBytes")) } else { String::new() };
        writeln!(out, "  {:address_width$}  {bytes}{}{at}", text(item, "address"), text(item, "instruction")).unwrap();
    }
    if list.is_empty() {
        out.push_str("The debug adapter returned no instructions.\n");
    }
    let rows: Vec<usize> = lines.into_iter().filter(|l| *l != usize::MAX).collect();
    let fallback = rows.get(BEFORE as usize).or(rows.last()).copied().unwrap_or(0);
    (out, current.unwrap_or(fallback))
}

impl Views {
    pub fn open_disassembly(&mut self, live: Option<&Live>) {
        let Some(live) = live.filter(|l| l.state == State::Stopped) else { return ui::notify("The disassembly shows while the program is paused.") };
        if !live.can("supportsDisassembleRequest") {
            return ui::notify("This debug adapter does not support disassembly.");
        }
        if live.frames.get(live.frame).is_none_or(|f| f.ip.is_none()) {
            return ui::notify("The selected stack frame has no instruction pointer.");
        }
        self.disasm = true;
        self.seen = None;
        self.out.push((DISASSEMBLY, "Loading disassembly...\n".into(), None, true));
    }

    pub fn ask_memory(&mut self, live: Option<&Live>) {
        match live {
            Some(live) if !live.can("supportsReadMemoryRequest") => ui::notify("This debug adapter does not support reading memory."),
            Some(live) if live.state == State::Stopped => self.asking = Some((String::new(), true)),
            _ => ui::notify("Memory can be read while the program is paused."),
        }
    }

    pub fn open_memory(&mut self, live: &mut Live, reference: String, label: String) {
        live.ask("readMemory", read_args(&reference), Want::ReadMemory(reference.clone()));
        self.memory = Some((reference, label));
        self.out.push((MEMORY, "Loading memory...\n".into(), None, true));
    }

    pub fn poll(&mut self, live: &mut Live, marks: &mut Marks) {
        let key = live.frames.get(live.frame).filter(|_| live.state == State::Stopped).map(|f| (f.id, f.ip.clone()));
        if key == self.seen {
            return;
        }
        self.seen = key.clone();
        let Some((_, ip)) = key else {
            marks.band = None;
            return;
        };
        if self.disasm {
            match ip {
                Some(ip) => live.ask("disassemble", disassemble_args(&ip), Want::Disassemble(ip)),
                None => {
                    marks.band = None;
                    self.out.push((DISASSEMBLY, "The selected stack frame has no instruction pointer.\n".into(), None, false));
                }
            }
        }
        if let Some((reference, _)) = &self.memory {
            live.ask("readMemory", read_args(reference), Want::ReadMemory(reference.clone()));
        }
    }

    pub fn disassembled(&mut self, live: &Live, ip: &str, success: bool, message: &str, body: &Value, marks: &mut Marks) {
        if live.frames.get(live.frame).and_then(|f| f.ip.as_deref()) != Some(ip) {
            return;
        }
        if !success {
            marks.band = None;
            return self.out.push((DISASSEMBLY, format!("The disassembly is not available: {message}\n"), None, false));
        }
        let (text, line) = disasm_text(body, ip);
        marks.band = Some((DISASSEMBLY.into(), line));
        self.out.push((DISASSEMBLY, text, Some(line), false));
    }

    pub fn memory_read(&mut self, reference: &str, success: bool, message: &str, body: &Value) {
        let Some((_, label)) = self.memory.as_ref().filter(|m| m.0 == reference) else { return };
        let text = match success {
            true => {
                let data = base64(body["data"].as_str().unwrap_or_default()).unwrap_or_default();
                let unreadable = (body["unreadableBytes"].as_u64().unwrap_or(0) as usize).min((BYTES as usize).saturating_sub(data.len()));
                let at = body["address"].as_str().unwrap_or(reference);
                let base = address(at).or(address(reference)).unwrap_or(0);
                format!("{label} at {at}, {} bytes\n\n{}", data.len() + unreadable, hex_dump(base, &data, unreadable))
            }
            false => format!("{label}: the memory could not be read: {message}\n"),
        };
        self.out.push((MEMORY, text, None, false));
    }

    pub fn memory_found(&mut self, live: &mut Live, expression: &str, success: bool, message: &str, body: &Value) {
        match body["memoryReference"].as_str() {
            Some(reference) if success => self.open_memory(live, reference.to_owned(), expression.to_owned()),
            _ if success => ui::notify(format!("\"{expression}\" has no memory address.")),
            _ => ui::notify(format!("\"{expression}\" could not be evaluated: {message}")),
        }
    }

    pub fn dialog(&mut self, ctx: &egui::Context, live: Option<&mut Live>) {
        let Some((text, focus)) = &mut self.asking else { return };
        let mut done = None;
        let modal = ui::modal(ctx, "memory_address_dialog", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(380.0);
            ui.heading("Open Memory View");
            ui.add_space(6.0);
            let resp = ui.add(TextEdit::singleline(text).hint_text("Address (0x...) or expression").desired_width(f32::INFINITY));
            if std::mem::take(focus) {
                resp.request_focus();
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if keys.primary(ui, "Open", true) {
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
        let Some(open) = done else { return };
        let text = self.asking.take().map(|a| a.0.trim().to_owned()).unwrap_or_default();
        let Some(live) = live.filter(|_| open && !text.is_empty()) else { return };
        match address(&text) {
            Some(_) => self.open_memory(live, text.clone(), text),
            None => live.evaluate(&text, "watch", Want::MemoryAt(text.clone())),
        }
    }
}

impl Halo {
    pub(crate) fn debug_views(&mut self) {
        let views = &mut self.debug.views;
        for (title, text, line, focus) in std::mem::take(&mut views.out) {
            self.editors.show_text(title, &text, line, focus);
        }
        views.disasm &= self.editors.text_view(DISASSEMBLY).is_some();
        views.focused = self.editors.text_view(DISASSEMBLY) == Some(true);
        if self.editors.text_view(MEMORY).is_none() || self.debug.live.is_none() {
            views.memory = None;
        }
        if self.debug.live.is_none() {
            views.seen = None;
            self.editors.marks.band = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(base64("SGVsbG8sIHdvcmxkIQ==").unwrap(), b"Hello, world!");
        assert_eq!(base64("AAEC/w==").unwrap(), [0, 1, 2, 255]);
        assert_eq!(base64("AAEC_w").unwrap(), [0, 1, 2, 255]);
        assert_eq!(base64("TWFu\r\nTWE=").unwrap(), b"ManMa");
        assert_eq!(base64("").unwrap(), b"");
        assert!(base64("a*b").is_none());
        let all: Vec<u8> = (0..=255).collect();
        let encoded = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w==";
        assert_eq!(base64(encoded).unwrap(), all);
    }

    #[test]
    fn formats_hex_dump_rows() {
        let data = b"Hello, world!\x00\x01\xffABCDEFGH";
        let text = hex_dump(0x1000, data, 3);
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], "00001000  48 65 6C 6C 6F 2C 20 77  6F 72 6C 64 21 00 01 FF  Hello, world!...");
        assert_eq!(rows[1], "00001010  41 42 43 44 45 46 47 48  ?? ?? ??                 ABCDEFGH???");
        assert!(hex_dump(0x7ff6_0000_0000, &[1], 0).starts_with("00007FF600000000  01 "));
        assert_eq!(hex_dump(0, &[], 0), "");
        assert_eq!(address("0x7FF6a0"), Some(0x7ff6a0));
        assert_eq!((address("  0X10 "), address("16"), address("0xzz")), (Some(16), None, None));
    }

    #[test]
    fn builds_requests_and_reads_disassembly() {
        assert_eq!(disassemble_args("0x1000"), json!({"memoryReference": "0x1000", "offset": 0, "instructionOffset": -50, "instructionCount": 200, "resolveSymbols": true}));
        assert_eq!(read_args("0x2000"), json!({"memoryReference": "0x2000", "offset": 0, "count": 1024}));
        let body = json!({"instructions": [
            {"address": "0x0ffc", "instructionBytes": "55", "instruction": "push rbp", "symbol": "main", "location": {"path": "/src/main.c"}, "line": 3},
            {"address": "0x0ffd", "instructionBytes": "48 89 e5", "instruction": "mov rbp, rsp", "symbol": "main", "line": 3},
            {"address": "0x1000", "instruction": "ret", "symbol": "helper", "line": 9},
        ]});
        let (text, line) = disasm_text(&body, "0x00001000");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, ["main:", "  0x0ffc  55        push rbp  ; main.c:3", "  0x0ffd  48 89 e5  mov rbp, rsp", "helper:", "  0x1000            ret  ; main.c:9"]);
        assert_eq!(line, 4);
        let (text, line) = disasm_text(&json!({"instructions": [{"address": "0x1", "instruction": "nop"}]}), "0x99");
        assert_eq!((text.as_str(), line), ("  0x1  nop\n", 0));
        assert_eq!(disasm_text(&json!({}), "0x1").1, 0);
    }
}
