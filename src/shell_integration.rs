use alacritty_terminal::event::{EventListener, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use alacritty_terminal::vte::ansi::{Processor, Timeout};
use eframe::egui;
use crate::terminal_images::{Found, Picture};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static FINISHED: AtomicBool = AtomicBool::new(false);

// true once after any terminal command finished, so Git and the Explorer can catch up right away
pub fn take_finished() -> bool {
    FINISHED.swap(false, Ordering::Relaxed)
}
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    PromptStart,
    CommandStart,
    Executed,
    Finished(Option<i32>),
    Line(String),
    Cwd(String),
}

fn unescape(text: &str) -> String {
    let (mut out, mut bytes) = (vec![], text.bytes());
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(b'x') => {
                let hex: Vec<u8> = bytes.by_ref().take(2).collect();
                out.extend(std::str::from_utf8(&hex).ok().and_then(|h| u8::from_str_radix(h, 16).ok()));
            }
            Some(other) => out.push(other),
            None => out.push(b),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse(body: &[u8]) -> Option<Mark> {
    let rest = std::str::from_utf8(body).ok()?.strip_prefix("633;")?;
    let (kind, args) = rest.split_once(';').unwrap_or((rest, ""));
    let first = args.split(';').next().unwrap_or("");
    Some(match kind {
        "A" => Mark::PromptStart,
        "B" => Mark::CommandStart,
        "C" => Mark::Executed,
        "D" => Mark::Finished(first.parse().ok()),
        "E" => Mark::Line(unescape(first)),
        "P" => Mark::Cwd(unescape(first.strip_prefix("Cwd=")?)),
        _ => return None,
    })
}

#[derive(Default)]
pub struct Scanner {
    state: u8,
    body: Vec<u8>,
}

impl Scanner {
    pub fn feed(&mut self, bytes: &[u8], mut found: impl FnMut(usize, Mark)) {
        for (i, &b) in bytes.iter().enumerate() {
            match (self.state, b) {
                (_, 0x18 | 0x1a) => self.state = 0,
                (2, 0x07 | 0x1b) => {
                    if let Some(mark) = parse(&self.body) {
                        found(i + 1, mark);
                    }
                    self.state = if b == 0x1b { 1 } else { 0 };
                }
                (2, _) => {
                    if self.body.len() < 1 << 16 {
                        self.body.push(b);
                    }
                }
                (_, 0x1b) => self.state = 1,
                (1, b']') => {
                    self.state = 2;
                    self.body.clear();
                }
                _ => self.state = 0,
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub line: String,
    pub prompt: i64,
    pub command: Option<i64>,
    pub column: usize,
    pub output: Option<i64>,
    pub end: Option<(i64, usize)>,
    pub exit: Option<i32>,
    pub started: Option<Instant>,
    pub duration: Option<Duration>,
}

impl Record {
    pub fn ran(&self) -> bool {
        !self.line.is_empty() || self.output.is_some() || self.end.is_some()
    }

    pub fn failed(&self) -> bool {
        self.exit.is_some_and(|code| code != 0)
    }

    pub fn row(&self) -> i64 {
        self.command.unwrap_or(self.prompt)
    }
}

#[derive(Default)]
pub struct Integration {
    pub commands: Vec<Record>,
    pub cwd: Option<String>,
    pub dropped: i64,
    pub seen: bool,
    pub restored: Vec<String>,
    pub cell: Option<(f32, f32)>,
    pub pictures: Vec<Picture>,
}

impl Integration {
    pub fn apply(&mut self, mark: Mark, (line, col): (i64, usize), now: Instant) {
        self.seen = true;
        if let Mark::PromptStart = mark {
            if self.commands.last().is_some_and(|r| !r.ran()) {
                self.commands.pop();
            }
            self.commands.push(Record { prompt: line, ..Default::default() });
            if self.commands.len() > 1000 {
                self.commands.remove(0);
            }
            return;
        }
        if let Mark::Cwd(dir) = mark {
            self.cwd = Some(dir);
            return;
        }
        let Some(last) = self.commands.last_mut() else { return };
        match mark {
            Mark::CommandStart => (last.command, last.column) = (Some(line), col),
            Mark::Executed => (last.output, last.started) = (Some(line), Some(now)),
            Mark::Finished(code) if last.end.is_none() && (last.output.is_some() || code.is_some()) => {
                FINISHED.store(true, Ordering::Relaxed);
                (last.end, last.exit, last.duration) = (Some((line, col)), code, last.started.map(|s| now - s));
            }
            Mark::Line(text) => last.line = text,
            _ => {}
        }
    }

    pub fn last_finished(&self) -> Option<&Record> {
        self.commands.iter().rev().find(|r| r.end.is_some())
    }

    pub fn recent(&self) -> Vec<String> {
        let mut seen: Vec<String> = vec![];
        for line in self.commands.iter().rev().map(|r| &r.line).chain(&self.restored).filter(|l| !l.trim().is_empty()) {
            if !seen.contains(line) {
                seen.push(line.clone());
            }
        }
        seen
    }
}

#[derive(Default)]
pub struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _: Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

pub struct Tap<T: EventListener> {
    pty: tty::Pty,
    term: Arc<FairMutex<Term<T>>>,
    integration: Arc<Mutex<Integration>>,
    ctx: egui::Context,
    pump: Pump,
}

impl<T: EventListener> Tap<T> {
    pub fn new(pty: tty::Pty, term: Arc<FairMutex<Term<T>>>, integration: Arc<Mutex<Integration>>, ctx: egui::Context) -> Self {
        Self { pty, term, integration, ctx, pump: Pump::default() }
    }
}

#[derive(Default)]
pub struct Pump {
    parser: Processor<NoSync>,
    scanner: Scanner,
    images: crate::terminal_images::Scanner,
    dropped: i64,
}

impl Pump {
    fn advance<T: EventListener>(&mut self, term: &mut Term<T>, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let (room, cols) = (term.screen_lines().saturating_sub(1).max(1), term.columns().max(1));
            let (mut lines, mut run, mut cut) = (0, 0, bytes.len());
            for (i, &b) in bytes.iter().enumerate() {
                run += 1;
                if b == b'\n' || run >= cols {
                    (lines, run) = (lines + 1, 0);
                }
                if lines >= room {
                    cut = i + 1;
                    break;
                }
            }
            let main = !term.mode().contains(TermMode::ALT_SCREEN);
            let before = term.grid().history_size();
            term.vi_mode_cursor.point.line = term.bottommost_line();
            self.parser.advance(term, &bytes[..cut]);
            let after = term.grid().history_size();
            if main && !term.mode().contains(TermMode::ALT_SCREEN) {
                let scrolled = (term.bottommost_line().0 - term.vi_mode_cursor.point.line.0) as i64;
                self.dropped += if after < before { (before - after) as i64 } else { (scrolled - (after - before) as i64).max(0) };
            }
            bytes = &bytes[cut..];
        }
    }

    fn place<T: EventListener>(&mut self, term: &mut Term<T>, integration: &Mutex<Integration>, found: Found, (line, column): (i64, usize)) {
        let Some(cell) = integration.lock().unwrap().cell.filter(|_| !term.mode().contains(TermMode::ALT_SCREEN)) else { return };
        let Some((image, size)) = crate::terminal_images::decode(found, cell, (term.columns(), term.screen_lines())) else { return };
        let rows = ((size.1 / cell.1).ceil() as usize).max(1);
        self.advance(term, "\r\n".repeat(rows).as_bytes());
        let mut integration = integration.lock().unwrap();
        integration.pictures.push(Picture { line, column, size, image: Some(image), texture: None });
        if integration.pictures.len() > 64 {
            integration.pictures.remove(0);
        }
    }

    pub fn feed<T: EventListener>(&mut self, term: &mut Term<T>, integration: &Mutex<Integration>, raw: &[u8]) -> Vec<u8> {
        let (mut bytes, mut found) = (Vec::with_capacity(raw.len()), vec![]);
        self.images.feed(raw, &mut bytes, &mut found);
        let mut events: Vec<(usize, Result<Mark, Found>)> = vec![];
        self.scanner.feed(&bytes, |at, mark| events.push((at, Ok(mark))));
        events.extend(found.into_iter().map(|(at, f)| (at, Err(f))));
        events.sort_by_key(|e| e.0);
        let (mut from, mut replies) = (0, vec![]);
        for (to, event) in events {
            self.advance(term, &bytes[from..to]);
            from = to;
            let cursor = term.grid().cursor.point;
            let line = self.dropped + term.grid().history_size() as i64 + cursor.line.0 as i64;
            match event {
                Ok(mark) => {
                    let mut integration = integration.lock().unwrap();
                    integration.dropped = self.dropped;
                    integration.apply(mark, (line, cursor.column.0), Instant::now());
                }
                Err(Found::Attributes) => {
                    let images = integration.lock().unwrap().cell.is_some();
                    replies.extend_from_slice(if images { b"\x1b[?62;4c" } else { b"\x1b[?6c" });
                }
                Err(found) => self.place(term, integration, found, (line, cursor.column.0)),
            }
        }
        self.advance(term, &bytes[from..]);
        integration.lock().unwrap().dropped = self.dropped;
        replies
    }
}

impl<T: EventListener> Read for Tap<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (cap, mut processed) = (buf.len().min(1 << 16), 0);
        while processed < 1 << 16 {
            match self.pty.reader().read(&mut buf[..cap]) {
                Ok(0) => break,
                Ok(n) => {
                    let replies = self.pump.feed(&mut self.term.lock_unfair(), &self.integration, &buf[..n]);
                    if !replies.is_empty() {
                        let _ = self.pty.writer().write_all(&replies);
                    }
                    processed += n;
                }
                Err(e) if processed > 0 && matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => break,
                Err(e) => return Err(e),
            }
        }
        if processed > 0 {
            self.ctx.request_repaint();
        }
        Ok(0)
    }
}

impl<T: EventListener> EventedReadWrite for Tap<T> {
    type Reader = Self;
    type Writer = <tty::Pty as EventedReadWrite>::Writer;

    unsafe fn register(&mut self, poll: &Arc<polling::Poller>, event: polling::Event, mode: polling::PollMode) -> io::Result<()> {
        unsafe { self.pty.register(poll, event, mode) }
    }

    fn reregister(&mut self, poll: &Arc<polling::Poller>, event: polling::Event, mode: polling::PollMode) -> io::Result<()> {
        self.pty.reregister(poll, event, mode)
    }

    fn deregister(&mut self, poll: &Arc<polling::Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.pty.writer()
    }
}

impl<T: EventListener> EventedPty for Tap<T> {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl<T: EventListener> OnResize for Tap<T> {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size);
    }
}

const SCRIPTS: [(&str, &str); 7] = [
    ("halo.ps1", include_str!("shell_integration/halo.ps1")),
    ("halo.bash", include_str!("shell_integration/halo.bash")),
    ("halo-wsl.sh", include_str!("shell_integration/halo-wsl.sh")),
    ("zsh/.zshenv", include_str!("shell_integration/zshenv.zsh")),
    ("zsh/.zprofile", include_str!("shell_integration/zprofile.zsh")),
    ("zsh/.zshrc", include_str!("shell_integration/zshrc.zsh")),
    ("zsh/.zlogin", include_str!("shell_integration/zlogin.zsh")),
];

pub fn write_scripts(dir: &Path) -> bool {
    std::fs::create_dir_all(dir.join("zsh")).is_ok()
        && SCRIPTS.iter().all(|(name, text)| {
            let (path, text) = (dir.join(name), text.replace("\r\n", "\n"));
            std::fs::read_to_string(&path).is_ok_and(|old| old == text) || std::fs::write(&path, text).is_ok()
        })
}

type Launch = (Vec<String>, Vec<(String, String)>);

pub fn inject(program: &str, args: &[String], dir: &Path) -> Option<Launch> {
    let name = Path::new(program).file_stem()?.to_string_lossy().to_lowercase();
    let has = |flags: &[&str]| args.iter().any(|a| flags.contains(&a.to_lowercase().as_str()));
    let quoted = |text: String| if cfg!(windows) && text.contains(' ') { format!("\"{text}\"") } else { text };
    let script = |name: &str| {
        let path = dir.join(name).to_string_lossy().into_owned();
        quoted(if cfg!(windows) { path.replace('\\', "/") } else { path })
    };
    let runs_command = || args.iter().any(|a| a == "-c" || !a.starts_with('-'));
    match name.as_str() {
        "pwsh" | "powershell" if !has(&["-command", "-c", "-file", "-f", "-encodedcommand", "-ec", "-noninteractive"]) => {
            let command = format!(". '{}'", dir.join("halo.ps1").to_string_lossy().replace('\'', "''"));
            let command = if cfg!(windows) { format!("\"{command}\"") } else { command };
            Some((args.iter().cloned().chain(["-NoExit".into(), "-Command".into(), command]).collect(), vec![]))
        }
        "bash" if !runs_command() => {
            let rest = args.iter().filter(|a| !matches!(a.as_str(), "-l" | "--login" | "-i")).cloned();
            let login = has(&["-l", "--login"]).then(|| ("HALO_SHELL_LOGIN".to_owned(), "1".to_owned()));
            Some((["--init-file".to_owned(), script("halo.bash")].into_iter().chain(rest).chain(["-i".to_owned()]).collect(), login.into_iter().collect()))
        }
        "zsh" if !runs_command() => {
            let user = std::env::var("ZDOTDIR").ok().or_else(|| Some(dirs::home_dir()?.to_string_lossy().into_owned()))?;
            let env = [("ZDOTDIR".to_owned(), dir.join("zsh").to_string_lossy().into_owned()), ("HALO_ZDOTDIR".to_owned(), user)];
            Some((args.to_vec(), env.into()))
        }
        "wsl" if !has(&["-e", "--exec", "--"]) => {
            let (drive, rest) = dir.to_str()?.split_once(":\\")?;
            let path = format!("/mnt/{}/{}/halo-wsl.sh", drive.to_lowercase(), rest.replace('\\', "/"));
            Some((args.iter().cloned().chain(["-e".into(), "sh".into(), quoted(path)]).collect(), vec![]))
        }
        _ => None,
    }
}

pub fn prepare(program: &str, args: &[String]) -> Launch {
    static WRITTEN: OnceLock<bool> = OnceLock::new();
    let dir = crate::settings::dir().join("shell-integration");
    match inject(program, args, &dir) {
        Some(injected) if *WRITTEN.get_or_init(|| write_scripts(&dir)) => injected,
        _ => (args.to_vec(), vec![]),
    }
}

pub fn native_dir(dir: &str, start: Option<&Path>) -> PathBuf {
    if let Some(start) = start.filter(|s| crate::wsl::parse(s).is_some()) {
        return crate::wsl::local(start, dir);
    }
    let bytes = dir.as_bytes();
    if cfg!(windows) && bytes.len() >= 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && (bytes.len() == 2 || bytes[2] == b'/') {
        return PathBuf::from(format!("{}:\\{}", (bytes[1] as char).to_ascii_uppercase(), dir.get(3..).unwrap_or("").replace('/', "\\")));
    }
    PathBuf::from(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(chunks: &[&[u8]]) -> Vec<(usize, usize, Mark)> {
        let mut scanner = Scanner::default();
        let mut out = vec![];
        for (index, chunk) in chunks.iter().enumerate() {
            scanner.feed(chunk, |at, mark| out.push((index, at, mark)));
        }
        out
    }

    #[test]
    fn parses_osc_633_split_across_reads_and_mixed_with_other_sequences() {
        let found = marks(&[b"\x1b]0;title\x07\x1b]633;A\x07$ \x1b]63", b"3;B\x1b\\ls\r\n\x1b]633;E;ls -la\\x3b echo \\\\x\x07", b"\x1b]633;C\x07out\x1b[31mred\x1b]8;;http://a\x07x\x1b]8;;\x07\x1b]633;D;2\x07\x1b]633;P;Cwd=C:\\\\work\x07"]);
        let kinds: Vec<Mark> = found.iter().map(|f| f.2.clone()).collect();
        assert_eq!(kinds, vec![
            Mark::PromptStart,
            Mark::CommandStart,
            Mark::Line(r"ls -la; echo \x".into()),
            Mark::Executed,
            Mark::Finished(Some(2)),
            Mark::Cwd(r"C:\work".into()),
        ]);
        assert_eq!((found[0].0, found[0].1), (0, 18));
        assert_eq!((found[1].0, found[1].1), (1, 4));
        assert_eq!(marks(&[b"\x1b]633;D\x07\x1b]633;Z\x07\x1b]633;A\x18\x1b]633;B\x07"]).iter().map(|f| f.2.clone()).collect::<Vec<_>>(), vec![Mark::Finished(None), Mark::CommandStart]);
        assert_eq!(unescape(r"a\x0ab\\c\x3b"), "a\nb\\c;");
    }

    struct Size(usize, usize);

    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.0
        }

        fn screen_lines(&self) -> usize {
            self.0
        }

        fn columns(&self) -> usize {
            self.1
        }
    }

    #[test]
    fn marks_stay_on_their_rows_after_the_scrollback_is_full() {
        use alacritty_terminal::index::{Column, Line, Point};
        let config = alacritty_terminal::term::Config { scrolling_history: 3, ..Default::default() };
        let mut term = Term::new(config, &Size(5, 10), alacritty_terminal::event::VoidListener);
        let (mut pump, integration) = (Pump::default(), Mutex::new(Integration::default()));
        pump.feed(&mut term, &integration, b"a\r\nb\r\n\x1b]633;A\x07$ cmd\x1b]633;E;cmd\x07\r\n\x1b]633;C\x07");
        pump.feed(&mut term, &integration, b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n");
        pump.feed(&mut term, &integration, b"a long line that wraps over rows\r\n\x1b]633;D;0\x07");
        let integration = integration.into_inner().unwrap();
        let base = integration.dropped + term.history_size() as i64;
        let text = |line: i64| term.bounds_to_string(Point::new(Line(line as i32), Column(0)), Point::new(Line(line as i32), Column(9)));
        let record = &integration.commands[0];
        assert_eq!(integration.dropped, 7);
        assert_eq!(record.prompt - base, -8);
        assert_eq!(text(record.output.unwrap() - base + 4).trim(), "5");
        assert_eq!(record.end.unwrap().0 - base, term.grid().cursor.point.line.0 as i64);
    }

    #[test]
    fn places_images_at_the_cursor_and_moves_below_them() {
        use alacritty_terminal::index::{Column, Line, Point};
        let mut term = Term::new(alacritty_terminal::term::Config::default(), &Size(10, 20), alacritty_terminal::event::VoidListener);
        let (mut pump, integration) = (Pump::default(), Mutex::new(Integration::default()));
        assert_eq!(pump.feed(&mut term, &integration, b"\x1b[c\x1bPq~\x1b\\"), b"\x1b[?6c");
        assert!(integration.lock().unwrap().pictures.is_empty());
        integration.lock().unwrap().cell = Some((8.0, 4.0));
        assert_eq!(pump.feed(&mut term, &integration, b"ab\x1bPq!3~-~\x1b\\$ \x1b[0c"), b"\x1b[?62;4c");
        let integration = integration.into_inner().unwrap();
        let picture = &integration.pictures[0];
        assert_eq!((picture.line, picture.column, picture.size), (0, 2, (3.0, 12.0)));
        assert_eq!(term.grid().cursor.point, Point::new(Line(3), Column(2)));
    }

    #[test]
    fn keeps_command_bookkeeping() {
        let mut integration = Integration::default();
        let t0 = Instant::now();
        let mut feed = |mark: Mark, line: i64, ms: u64| integration.apply(mark, (line, 0), t0 + Duration::from_millis(ms));
        feed(Mark::Finished(None), 0, 0);
        feed(Mark::Cwd("/home".into()), 0, 0);
        feed(Mark::PromptStart, 0, 0);
        feed(Mark::CommandStart, 0, 0);
        feed(Mark::Finished(None), 1, 0);
        feed(Mark::PromptStart, 1, 0);
        feed(Mark::CommandStart, 1, 0);
        feed(Mark::Line("make".into()), 1, 0);
        feed(Mark::Executed, 2, 100);
        feed(Mark::Finished(Some(2)), 5, 350);
        feed(Mark::PromptStart, 5, 400);
        feed(Mark::Line("ls".into()), 5, 400);
        feed(Mark::Executed, 6, 400);
        feed(Mark::Finished(Some(0)), 8, 500);
        feed(Mark::Cwd("/tmp".into()), 8, 500);
        feed(Mark::PromptStart, 8, 500);
        feed(Mark::Line("ls".into()), 8, 500);
        feed(Mark::Executed, 9, 600);
        assert_eq!(integration.cwd.as_deref(), Some("/tmp"));
        let c = &integration.commands;
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].line.as_str(), c[0].row(), c[0].output, c[0].end, c[0].exit), ("make", 1, Some(2), Some((5, 0)), Some(2)));
        assert_eq!(c[0].duration, Some(Duration::from_millis(250)));
        assert!(c[0].failed() && !c[1].failed());
        assert_eq!((c[2].end, c[2].output), (None, Some(9)));
        assert_eq!(integration.last_finished().map(|r| r.line.as_str()), Some("ls"));
        assert_eq!(integration.recent(), vec!["ls".to_owned(), "make".to_owned()]);
        integration.restored = vec!["make".into(), "git status".into()];
        assert_eq!(integration.recent(), vec!["ls".to_owned(), "make".to_owned(), "git status".to_owned()]);
    }

    #[test]
    fn picks_the_script_for_each_shell() {
        let dir = if cfg!(windows) { Path::new(r"C:\cfg\shell-integration") } else { Path::new("/cfg/shell-integration") };
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (pwsh, env) = inject("pwsh.exe", &args(&["-NoLogo"]), dir).unwrap();
        assert_eq!(&pwsh[..3], &args(&["-NoLogo", "-NoExit", "-Command"])[..]);
        assert!(pwsh[3].contains("halo.ps1") && env.is_empty());
        assert!(inject("powershell.exe", &args(&["-Command", "dir"]), dir).is_none());
        let (bash, env) = inject("/usr/bin/bash", &args(&["--login", "-i"]), dir).unwrap();
        assert_eq!((bash[0].as_str(), bash.last().unwrap().as_str(), bash.len()), ("--init-file", "-i", 3));
        assert!(bash[1].ends_with("halo.bash"));
        assert_eq!(env, vec![("HALO_SHELL_LOGIN".to_owned(), "1".to_owned())]);
        assert!(inject("bash", &args(&[]), dir).unwrap().1.is_empty());
        assert!(inject("bash", &args(&["-c", "ls"]), dir).is_none());
        assert!(inject("bash", &args(&["script.sh"]), dir).is_none());
        let (zsh, env) = inject("zsh", &args(&["-l"]), dir).unwrap();
        assert_eq!(zsh, args(&["-l"]));
        assert!(env[0].0 == "ZDOTDIR" && env[0].1.ends_with("zsh") && env[1].0 == "HALO_ZDOTDIR");
        assert!(inject("cmd.exe", &args(&[]), dir).is_none());
        assert!(inject("fish", &args(&[]), dir).is_none());
        if cfg!(windows) {
            let (wsl, _) = inject("wsl.exe", &args(&["-d", "Ubuntu"]), dir).unwrap();
            assert_eq!(wsl, args(&["-d", "Ubuntu", "-e", "sh", "/mnt/c/cfg/shell-integration/halo-wsl.sh"]));
            assert!(inject("wsl.exe", &args(&["-e", "top"]), dir).is_none());
            assert_eq!(native_dir("/c/Users/joe", None), PathBuf::from(r"C:\Users\joe"));
            assert_eq!(native_dir(r"D:\x", None), PathBuf::from(r"D:\x"));
            let start = Path::new(r"\\wsl.localhost\Ubuntu\home");
            assert_eq!(native_dir("/home/joe", Some(start)), PathBuf::from(r"\\wsl.localhost\Ubuntu\home\joe"));
        }
        for (name, text) in [SCRIPTS[0], SCRIPTS[1], SCRIPTS[5]] {
            assert!(text.contains("633;") && text.contains("Cwd="), "{name}");
        }
        assert!(SCRIPTS.iter().skip(2).all(|(_, text)| text.contains("HALO_ZDOTDIR")));
    }
}
