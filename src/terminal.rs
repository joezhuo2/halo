use crate::command::Command;
use crate::lsp::{Diagnostic, Lsp};
use crate::settings::{Settings, ShellProfile};
use crate::shell_integration::{Integration, Record, Tap};
use crate::terminal_find::FindBar;
use crate::terminal_links::{Link, Target};
use crate::terminal_quickfix::Fix;
use crate::terminal_restore::{Place, Saved};
use crate::terminal_suggest::{Item, Kind};
use crate::theme::Theme;
use crate::ui::{self, icons};
use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Event, EventFilter, FontId, Id, Key, Layout, Modifiers, Pos2, Rect,
    RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

struct Shared {
    exited: AtomicBool,
    title: Mutex<Option<String>>,
    sender: Mutex<Option<EventLoopSender>>,
    code: Mutex<Option<i32>>,
    window: Mutex<WindowSize>,
}

#[derive(Clone)]
struct Proxy {
    ctx: egui::Context,
    shared: Arc<Shared>,
}

impl Proxy {
    fn reply(&self, text: String) {
        if let Some(sender) = self.shared.sender.lock().unwrap().as_ref() {
            let _ = sender.send(Msg::Input(text.into_bytes().into()));
        }
    }
}

impl EventListener for Proxy {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Title(title) => *self.shared.title.lock().unwrap() = Some(title),
            TermEvent::ResetTitle => *self.shared.title.lock().unwrap() = None,
            TermEvent::ChildExit(status) => *self.shared.code.lock().unwrap() = status.code(),
            TermEvent::Exit => self.shared.exited.store(true, Ordering::Relaxed),
            TermEvent::PtyWrite(text) => self.reply(text),
            TermEvent::TextAreaSizeRequest(format) => self.reply(format(*self.shared.window.lock().unwrap())),
            _ => {}
        }
        self.ctx.request_repaint();
    }
}

enum Choice {
    Complete(Item),
    Fix(Fix),
}

struct Pick {
    rows: Vec<(String, &'static str, &'static str, Choice)>,
    selected: Option<usize>,
    reveal: bool,
    fixes: bool,
    at: Option<Pos2>,
}

fn kind_look(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::History => (icons::CLOCK_COUNTER_CLOCKWISE, "history"),
        Kind::Command => (icons::TERMINAL_WINDOW, "command"),
        Kind::Sub => (icons::CARET_RIGHT, "subcommand"),
        Kind::Flag => (icons::FLAG, "flag"),
        Kind::Folder => (icons::FOLDER, "folder"),
        Kind::File => (icons::FILE, "file"),
    }
}

fn free_port(port: u16) {
    std::thread::spawn(move || {
        let Some(row) = list_ports().into_iter().find(|r| r.port == port && !r.pid.is_empty()) else {
            return ui::notify(format!("No process was found listening on port {port}."));
        };
        let (program, args) = if cfg!(windows) { ("taskkill", vec!["/PID", row.pid.as_str(), "/F"]) } else { ("kill", vec![row.pid.as_str()]) };
        let stopped = hidden_command(program).args(args).status().is_ok_and(|s| s.success());
        ui::notify(match stopped {
            true => format!("Stopped {} (PID {}) to free port {port}.", row.process, row.pid),
            false => format!("Could not stop PID {} on port {port}.", row.pid),
        });
    });
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub fn xterm_color(index: u8) -> Color32 {
    if index >= 232 {
        let level = 8 + 10 * (index - 232);
        return Color32::from_rgb(level, level, level);
    }
    let i = index.saturating_sub(16);
    let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
    Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
}

fn resolve(color: AnsiColor, theme: &Theme) -> Option<Color32> {
    Some(match color {
        AnsiColor::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) if i < 16 => theme.ansi[i as usize],
        AnsiColor::Indexed(i) => xterm_color(i),
        AnsiColor::Named(name) => match name as usize {
            i @ 0..=15 => theme.ansi[i],
            i @ 259..=266 => ui::mix(theme.ansi[i - 259], theme.panel, 0.35),
            i if i == NamedColor::Background as usize => return None,
            i if i == NamedColor::Cursor as usize => theme.accent,
            i if i == NamedColor::DimForeground as usize => theme.text_dim,
            _ => theme.text,
        },
    })
}

fn word_step(line: &[char], col: usize, forward: bool, word: bool) -> usize {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let len = line.len();
    let mut i = col.min(len);
    match (word, forward) {
        (false, true) => return (i + 1).min(len),
        (false, false) => return i.saturating_sub(1),
        (true, true) => {
            while i < len && !is_word(line[i]) {
                i += 1;
            }
            while i < len && is_word(line[i]) {
                i += 1;
            }
        }
        (true, false) => {
            while i > 0 && !is_word(line[i - 1]) {
                i -= 1;
            }
            while i > 0 && is_word(line[i - 1]) {
                i -= 1;
            }
        }
    }
    i
}

fn windows_word_erase(program: &str) -> bool {
    let name = Path::new(program).file_name().and_then(|s| s.to_str()).unwrap_or_default();
    let name = name.to_ascii_lowercase();
    cfg!(windows) && matches!(name.strip_suffix(".exe").unwrap_or(&name), "cmd" | "powershell" | "pwsh")
}

fn encode_key(key: Key, m: Modifiers, app_cursor: bool, windows_word_erase: bool) -> Option<Vec<u8>> {
    let mods = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    let arrow = |c: char| match (mods > 1, app_cursor) {
        (true, _) => format!("\x1b[1;{mods}{c}"),
        (false, true) => format!("\x1bO{c}"),
        (false, false) => format!("\x1b[{c}"),
    };
    let tilde = |n: u8| if mods > 1 { format!("\x1b[{n};{mods}~") } else { format!("\x1b[{n}~") };
    let low_f = |c: char| if mods > 1 { format!("\x1b[1;{mods}{c}") } else { format!("\x1bO{c}") };
    let alt = |s: &str| if m.alt { format!("\x1b{s}") } else { s.to_owned() };
    let text = match key {
        Key::ArrowUp => arrow('A'),
        Key::ArrowDown => arrow('B'),
        Key::ArrowRight => arrow('C'),
        Key::ArrowLeft => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::Insert => tilde(2),
        // readline and PSReadLine both delete the word to the right on Alt+D
        Key::Delete if m.ctrl && !m.shift && !m.alt => "d".to_owned(),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => low_f('P'),
        Key::F2 => low_f('Q'),
        Key::F3 => low_f('R'),
        Key::F4 => low_f('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter => alt("\r"),
        Key::Tab if m.shift => "\x1b[Z".to_owned(),
        Key::Tab => "\t".to_owned(),
        Key::Backspace if m.ctrl => alt(if windows_word_erase { "\x08" } else { "\x17" }),
        Key::Backspace => alt("\x7f"),
        Key::Escape => "\x1b".to_owned(),
        Key::Space if m.ctrl => "\0".to_owned(),
        other => {
            let name = other.name();
            let byte = match name.as_bytes() {
                [b] if b.is_ascii_alphabetic() => *b,
                _ => match other {
                    Key::OpenBracket => b'[',
                    Key::Backslash => b'\\',
                    Key::CloseBracket => b']',
                    _ => return None,
                },
            };
            if m.ctrl {
                alt(&((byte.to_ascii_uppercase() & 0x1f) as char).to_string())
            } else if m.alt && byte.is_ascii_alphabetic() {
                let c = if m.shift { byte.to_ascii_uppercase() } else { byte.to_ascii_lowercase() };
                format!("\x1b{}", c as char)
            } else {
                return None;
            }
        }
    };
    Some(text.into_bytes())
}

#[derive(Clone, Default)]
pub struct Look {
    font: bool,
    size: f32,
    line_height: f32,
    cursor: String,
    blink: bool,
    scrollback: usize,
    copy_on_select: bool,
    right_click: String,
    suggest: bool,
    sticky: bool,
    images: bool,
}

static LOOK: Mutex<Option<Look>> = Mutex::new(None);

pub fn configure(s: &Settings) {
    *LOOK.lock().unwrap() = Some(Look {
        font: !s.terminal_font_family.is_empty(),
        size: s.terminal_font_size,
        line_height: s.terminal_line_height,
        cursor: s.terminal_cursor_style.clone(),
        blink: s.terminal_cursor_blinking && !crate::a11y::reduced(),
        scrollback: s.terminal_scrollback,
        copy_on_select: s.terminal_copy_on_selection,
        right_click: s.terminal_right_click.clone(),
        suggest: s.terminal_suggest,
        sticky: s.terminal_sticky_scroll,
        images: s.terminal_images,
    });
}

fn look() -> Look {
    let fallback = Look { line_height: 1.15, blink: true, scrollback: 5000, suggest: true, sticky: true, images: true, ..Default::default() };
    LOOK.lock().unwrap().clone().unwrap_or(fallback)
}

type HoveredLink = (i32, Vec<(usize, usize)>, Link);

fn grid_rows(term: &Term<Proxy>, from: i32, to: i32) -> Vec<(Vec<char>, bool)> {
    let last = term.last_column();
    let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
    (from..=to)
        .map(|line| {
            let row = &term.grid()[Line(line)];
            let chars = (0..=last.0).map(|c| if row[Column(c)].flags.intersects(spacer) { '\0' } else { row[Column(c)].c }).collect();
            (chars, row[last].flags.contains(Flags::WRAPLINE))
        })
        .collect()
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    shared: Arc<Shared>,
    pub name: String,
    id: u64,
    size: (usize, usize),
    focused: bool,
    want_focus: bool,
    wheel: f32,
    last_input: f64,
    task: Option<String>,
    pub pid: Option<u32>,
    start_dir: Option<PathBuf>,
    integration: Arc<Mutex<Integration>>,
    links: (i32, String, Vec<(usize, usize)>, Vec<Link>),
    find: FindBar,
    pub label: Option<String>,
    pub color: Option<usize>,
    pub profile: Option<ShellProfile>,
    restored: bool,
    key_sel: Option<Point>,
    windows_word_erase: bool,
    pick: Option<Pick>,
    suggest_for: Option<String>,
    force_suggest: bool,
    fixes: (i64, Vec<(String, Fix)>),
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

impl Terminal {
    pub fn spawn(id: u64, name: &str, program: &str, args: &[String], cwd: Option<&Path>, ctx: egui::Context) -> std::io::Result<Self> {
        Self::start(id, name, program, args, &HashMap::new(), cwd, None, "", ctx)
    }

    pub fn spawn_with(id: u64, name: &str, program: &str, args: &[String], cwd: Option<&Path>, env: &[(String, String)], ctx: egui::Context) -> std::io::Result<Self> {
        Self::start(id, name, program, args, &env.iter().cloned().collect(), cwd, None, "", ctx)
    }

    pub fn spawn_shell(id: u64, profile: &ShellProfile, cwd: Option<&Path>, integrate: bool, prelude: &str, ctx: egui::Context) -> std::io::Result<Self> {
        let (args, env) = match integrate {
            true => crate::shell_integration::prepare(&profile.program, &profile.args),
            false => (profile.args.clone(), vec![]),
        };
        let mut terminal = Self::start(id, &profile.name, &profile.program, &args, &env.into_iter().collect(), cwd, None, prelude, ctx)?;
        terminal.profile = Some(profile.clone());
        Ok(terminal)
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        id: u64,
        name: &str,
        program: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&Path>,
        task: Option<&str>,
        prelude: &str,
        ctx: egui::Context,
    ) -> std::io::Result<Self> {
        let mut options = tty::Options {
            shell: Some(tty::Shell::new(program.to_owned(), args.to_vec())),
            working_directory: cwd.map(Path::to_path_buf).or_else(dirs::home_dir),
            env: env.clone(),
            ..Default::default()
        };
        options.env.insert("TERM".to_owned(), "xterm-256color".to_owned());
        options.env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
        let window = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = tty::new(&options, window, id)?;
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|p| p.get());
        #[cfg(not(windows))]
        let pid = Some(pty.child().id());
        let shared = Arc::new(Shared {
            exited: AtomicBool::new(false),
            title: Mutex::new(None),
            sender: Mutex::new(None),
            code: Mutex::new(None),
            window: Mutex::new(window),
        });
        let proxy = Proxy { ctx: ctx.clone(), shared: shared.clone() };
        let config = Config { scrolling_history: look().scrollback, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &Size { cols: 80, rows: 24 }, proxy.clone())));
        crate::terminal_restore::print(&mut term.lock(), prelude);
        let integration = Arc::new(Mutex::new(Integration::default()));
        crate::terminal_suggest::warm();
        let tap =Tap::new(pty, term.clone(), integration.clone(), ctx);
        let event_loop = EventLoop::new(term.clone(), proxy, tap, task.is_some(), false)?;
        let sender = event_loop.channel();
        *shared.sender.lock().unwrap() = Some(sender.clone());
        event_loop.spawn();
        Ok(Self {
            term,
            sender,
            shared,
            name: name.to_owned(),
            id,
            size: (80, 24),
            focused: false,
            want_focus: true,
            wheel: 0.0,
            last_input: 0.0,
            task: task.map(str::to_owned),
            pid,
            start_dir: options.working_directory,
            integration,
            links: Default::default(),
            find: FindBar::default(),
            label: None,
            color: None,
            profile: None,
            restored: !prelude.is_empty(),
            key_sel: None,
            windows_word_erase: windows_word_erase(program),
            pick: None,
            suggest_for: None,
            force_suggest: false,
            fixes: (i64::MIN, vec![]),
        })
    }

    pub fn is_alive(&self) -> bool {
        !self.shared.exited.load(Ordering::Relaxed)
    }

    pub fn exit_code(&self) -> Option<i32> {
        (!self.is_alive()).then(|| self.shared.code.lock().unwrap().unwrap_or(0))
    }

    pub fn snapshot(&self, finished: bool) -> (Vec<String>, usize) {
        let term = self.term.lock();
        let (top, bottom) = (term.topmost_line(), term.bottommost_line());
        let text = term.bounds_to_string(Point::new(top, Column(0)), Point::new(bottom, term.last_column()));
        let partial = !finished && term.grid().cursor.point.column.0 > 0;
        let mut lines: Vec<String> = text.lines().map(|l| l.trim_end().to_owned()).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        let complete = lines.len().saturating_sub(partial as usize);
        (lines, complete)
    }

    pub fn note(&self, text: &str) {
        let mut processor = alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::default();
        processor.advance(&mut *self.term.lock(), text.as_bytes());
    }

    fn caret_line(&self) -> (String, usize, usize, (usize, usize), usize) {
        let term = self.term.lock();
        let point = term.grid().cursor.point;
        let (text, _) = grid_rows(&term, point.line.0, point.line.0).pop().unwrap_or_default();
        let text: String = text.into_iter().filter(|c| *c != '\0').collect::<String>().trim_end().to_owned();
        let column = point.column.0.min(text.chars().count());
        (text, point.line.0.max(0) as usize, column, (column, column), 0)
    }

    pub fn title(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        if self.task.is_some() {
            return self.name.clone();
        }
        self.shared.title.lock().unwrap().clone().filter(|t| !t.is_empty()).unwrap_or_else(|| self.name.clone())
    }

    pub fn has_focus(&self) -> bool {
        self.focused || self.find.has_focus
    }

    pub fn exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        match cmd {
            Command::Find | Command::Replace => self.find.show(),
            Command::Copy => {
                if let Some(text) = self.selection() {
                    ctx.copy_text(text);
                }
            }
            Command::Paste => {
                self.focus();
                ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
            }
            Command::ScrollToPrevMark | Command::ScrollToNextMark => self.scroll_to_command(*cmd == Command::ScrollToNextMark),
            Command::CopyLastCommandOutput => match self.last_output() {
                Some(text) => ctx.copy_text(text),
                None => ui::notify("No finished command output was found. It needs shell integration in the active terminal."),
            },
            Command::RunCommandLine(line) => {
                self.write(format!("{line}\r"));
                self.focus();
            }
            Command::SetTerminalColor(color) => self.color = *color,
            Command::ClearRestoredHistory => self.clear_restored(),
            _ => return false,
        }
        ctx.request_repaint();
        true
    }

    pub fn tint(&self, theme: &Theme) -> Color32 {
        match (self.failed(), self.color) {
            (true, _) => theme.error,
            (_, Some(color)) => theme.ansi[color],
            _ => theme.text_dim,
        }
    }

    fn reveal(&self, line: i32) {
        let mut term = self.term.lock();
        let (rows, offset) = (term.screen_lines() as i32, term.grid().display_offset() as i32);
        if !(0..rows).contains(&(line + offset)) {
            let target = (rows / 2 - line).clamp(0, term.history_size() as i32);
            term.scroll_display(Scroll::Delta(target - offset));
        }
    }

    pub fn focus(&mut self) {
        self.want_focus = true;
    }

    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let _ = self.sender.send(Msg::Input(bytes.into().into()));
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.write(format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")));
        } else {
            self.write(text.replace("\r\n", "\r").replace('\n', "\r"));
        }
    }

    pub fn text(&self) -> String {
        let term = self.term.lock();
        let first = term.topmost_line().max(term.bottommost_line() - 500);
        let text = term.bounds_to_string(Point::new(first, Column(0)), Point::new(term.bottommost_line(), term.last_column()));
        text.trim_end().to_owned()
    }

    fn shell_selects(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| {
            let program = p.program.to_lowercase();
            program.contains("pwsh") || program.contains("powershell")
        })
    }

    fn clear_key_selection(&mut self) {
        if self.key_sel.take().is_some() {
            self.term.lock().selection = None;
        }
    }

    fn select_by_key(&mut self, forward: bool, word: bool) {
        let mut term = self.term.lock();
        let cursor = term.grid().cursor.point;
        let from = self.key_sel.filter(|_| term.selection.is_some()).unwrap_or(cursor);
        let cols = term.columns();
        let line: Vec<char> = (0..cols).map(|c| term.grid()[from.line][Column(c)].c).collect();
        let to = word_step(&line, from.column.0, forward, word);
        let (end, side) = if to >= cols { (Point::new(from.line, Column(cols - 1)), Side::Right) } else { (Point::new(from.line, Column(to)), Side::Left) };
        if self.key_sel.is_none() || term.selection.is_none() {
            term.selection = Some(Selection::new(SelectionType::Simple, cursor, Side::Left));
        }
        if let Some(selection) = term.selection.as_mut() {
            selection.update(end, side);
        }
        self.key_sel = Some(Point::new(from.line, Column(to)));
    }

    pub fn selection(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    fn point_at(&self, rect: Rect, pos: Pos2, cell: egui::Vec2, offset: usize) -> (Point, Side) {
        let x = ((pos.x - rect.left()) / cell.x).max(0.0);
        let column = (x as usize).min(self.size.0.saturating_sub(1));
        let row = (((pos.y - rect.top()) / cell.y).max(0.0) as usize).min(self.size.1.saturating_sub(1));
        let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
        (Point::new(Line(row as i32 - offset as i32), Column(column)), side)
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        let reported = self.integration.lock().unwrap().cwd.clone();
        reported.map(|dir| crate::shell_integration::native_dir(&dir, self.start_dir.as_deref())).or_else(|| self.start_dir.clone())
    }

    pub fn failed(&self) -> bool {
        self.integration.lock().unwrap().last_finished().is_some_and(Record::failed)
    }

    pub fn recent(&self) -> Vec<String> {
        self.integration.lock().unwrap().recent()
    }

    fn clear_restored(&mut self) {
        self.integration.lock().unwrap().restored.clear();
        let mut term = self.term.lock();
        let (top, bottom, cols) = (term.topmost_line().0, term.bottommost_line().0, term.columns());
        let marker = crate::terminal_restore::MARKER;
        let starts = |line: i32| (0..marker.len().min(cols)).map(|c| term.grid()[Line(line)][Column(c)].c).eq(marker.chars().take(cols));
        match (top..=bottom).rev().find(|&line| self.restored && starts(line)) {
            Some(end) => (top..=end).for_each(|line| term.grid_mut()[Line(line)].reset(&alacritty_terminal::term::cell::Cell::default())),
            None => ui::notify("The active terminal has no restored history."),
        }
        self.restored = false;
    }

    pub fn persist(&self, limit: usize) -> Option<Saved> {
        if self.task.is_some() || !self.is_alive() {
            return None;
        }
        let text = crate::terminal_restore::serialize(&mut self.term.lock(), limit);
        let title = self.shared.title.lock().unwrap().clone().unwrap_or_default();
        Some(Saved { profile: self.profile.clone(), cwd: self.cwd(), label: self.label.clone(), title, color: self.color, recent: self.recent(), text, ..Default::default() })
    }

    pub fn scroll_to_command(&self, forward: bool) {
        let mut term = self.term.lock();
        let (offset, history) = (term.grid().display_offset() as i64, term.history_size() as i64);
        let integration = self.integration.lock().unwrap();
        let base = integration.dropped + history;
        let lines = integration.commands.iter().filter(|r| r.ran()).map(|r| r.prompt - base);
        let target = if forward { lines.filter(|l| *l > -offset).min() } else { lines.filter(|l| *l < -offset).max() };
        drop(integration);
        match target {
            Some(line) => term.scroll_display(Scroll::Delta(((-line).clamp(0, history) - offset) as i32)),
            None if forward => term.scroll_display(Scroll::Bottom),
            None => {}
        }
    }

    pub fn last_output(&self) -> Option<String> {
        let record = self.integration.lock().unwrap().commands.iter().rev().find(|r| r.output.is_some() && r.end.is_some())?.clone();
        self.output_of(&record)
    }

    fn typed(&self) -> Option<String> {
        let term = self.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) || term.grid().display_offset() != 0 {
            return None;
        }
        let integration = self.integration.lock().unwrap();
        let record = integration.commands.last().filter(|r| r.output.is_none() && r.end.is_none())?;
        let start = Point::new(Line((record.command? - integration.dropped - term.history_size() as i64) as i32), Column(record.column));
        let cursor = term.grid().cursor.point;
        if cursor < start || start.line < term.topmost_line() || cursor.line.0 - start.line.0 > 20 {
            return None;
        }
        let rows = grid_rows(&term, start.line.0, cursor.line.0);
        let last = rows.len() - 1;
        let mut text = String::new();
        for (i, (chars, _)) in rows.iter().enumerate() {
            let (from, to) = (if i == 0 { start.column.0 } else { 0 }, if i == last { cursor.column.0 } else { chars.len() });
            text.extend(chars[from.min(to)..to.min(chars.len())].iter().filter(|c| **c != '\0'));
        }
        Some(text)
    }

    fn path_sep(&self) -> char {
        let program = self.profile.as_ref().map(|p| p.program.to_lowercase()).unwrap_or_default();
        match ["wsl", "bash", "zsh", "sh.exe", "fish"].iter().any(|s| program.contains(s)) {
            true => '/',
            false => std::path::MAIN_SEPARATOR,
        }
    }

    fn refresh_suggest(&mut self) {
        let typed = self.typed();
        if typed == self.suggest_for && !self.force_suggest {
            return;
        }
        let force = std::mem::take(&mut self.force_suggest);
        let items = match typed.as_deref() {
            Some(text) => crate::terminal_suggest::complete(text, &self.recent(), self.cwd().as_deref(), self.path_sep(), force),
            None => vec![],
        };
        let rows: Vec<_> = items
            .into_iter()
            .map(|item| {
                let (icon, detail) = kind_look(item.kind);
                (item.label.clone(), icon, detail, Choice::Complete(item))
            })
            .collect();
        self.pick = (!rows.is_empty()).then_some(Pick { rows, selected: None, reveal: true, fixes: false, at: None });
        self.suggest_for = typed;
    }

    fn open_fixes(&mut self, at: Option<Pos2>) {
        let rows = self.fixes.1.iter().map(|(label, fix)| (label.clone(), icons::LIGHTBULB, "", Choice::Fix(fix.clone()))).collect();
        self.pick = Some(Pick { rows, selected: Some(0), reveal: true, fixes: true, at });
    }

    fn accept(&mut self, index: usize) {
        let Some(pick) = self.pick.take() else { return };
        match pick.rows.into_iter().nth(index).map(|row| row.3) {
            Some(Choice::Complete(item)) => self.write(format!("{}{}", "\x7f".repeat(item.replace), item.insert)),
            Some(Choice::Fix(Fix::Run(line))) => self.write(format!("{line}\r")),
            Some(Choice::Fix(Fix::Open(url))) => ui::open_url(&url),
            Some(Choice::Fix(Fix::FreePort(port))) => free_port(port),
            None => {}
        }
        self.want_focus = true;
    }

    fn output_of(&self, record: &Record) -> Option<String> {
        let term = self.term.lock();
        let base = self.integration.lock().unwrap().dropped + term.history_size() as i64;
        let (from, (end, col)) = ((record.output? - base).max(term.topmost_line().0 as i64), record.end?);
        let end = end - base;
        if end < term.topmost_line().0 as i64 || end > term.bottommost_line().0 as i64 {
            return None;
        }
        let last = match col {
            0 if end <= from => return Some(String::new()),
            0 => Point::new(Line(end as i32 - 1), term.last_column()),
            _ => Point::new(Line(end as i32), Column(col - 1)),
        };
        Some(term.bounds_to_string(Point::new(Line(from as i32), Column(0)), last).trim_end().to_owned())
    }

    fn hovered_link(&mut self, point: Point) -> Option<HoveredLink> {
        let term = self.term.lock();
        let wrapped = |line: i32| term.grid()[Line(line)][term.last_column()].flags.contains(Flags::WRAPLINE);
        let (top, bottom, at) = (term.topmost_line().0, term.bottommost_line().0, point.line.0);
        let (mut from, mut to) = (at, at);
        while from > top && at - from < 40 && wrapped(from - 1) {
            from -= 1;
        }
        while to < bottom && to - at < 40 && wrapped(to) {
            to += 1;
        }
        let (text, cells) = crate::terminal_links::join(&grid_rows(&term, from, to));
        drop(term);
        if self.links.0 != from || self.links.1 != text {
            let (cwd, home) = (self.cwd(), dirs::home_dir());
            let links = crate::terminal_links::find(&text, cwd.as_deref(), home.as_deref(), Path::is_file);
            self.links = (from, text, cells, links);
        }
        let index = self.links.2.iter().position(|&(r, c)| from + r as i32 == at && c == point.column.0)?;
        let link = self.links.3.iter().find(|l| l.start <= index && index < l.end)?.clone();
        Some((from, self.links.2[link.start..link.end].to_vec(), link))
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, font_size: f32, cmds: &mut Vec<Command>) {
        let ctx = ui.ctx().clone();
        let rect = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
        let id = Id::new(("terminal", self.id));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        crate::a11y::line_node(ui, id, ui::Role::Terminal, &self.title(), || self.caret_line());
        let look = look();
        let size = if look.size > 0.0 { look.size } else { font_size };
        let font = if look.font { FontId::new(size, egui::FontFamily::Name("terminal".into())) } else { FontId::monospace(size) };
        let (cw, rh) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.row_height(&font)));
        let cell = vec2(cw, (rh * look.line_height.clamp(1.0, 3.0)).round());
        let cols = ((rect.width() / cell.x) as usize).max(2);
        let rows = ((rect.height() / cell.y) as usize).max(1);
        if (cols, rows) != self.size {
            self.size = (cols, rows);
            self.term.lock().resize(Size { cols, rows });
            let window = WindowSize { num_lines: rows as u16, num_cols: cols as u16, cell_width: cell.x as u16, cell_height: cell.y as u16 };
            *self.shared.window.lock().unwrap() = window;
            let _ = self.sender.send(Msg::Resize(window));
        }
        self.integration.lock().unwrap().cell = look.images.then_some((cell.x, cell.y));
        if self.want_focus || resp.clicked() || resp.drag_started() {
            resp.request_focus();
            self.want_focus = false;
        }
        self.focused = resp.has_focus();
        let now = ui.input(|i| i.time);
        let mode = *self.term.lock().mode();
        let mut step = 0;

        if ui.input(|i| i.pointer.any_pressed()) {
            self.clear_key_selection();
        }
        if self.focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true });
            });
            let events = ui.input(|i| i.events.clone());
            for event in events {
                match &event {
                    Event::Key { key: key @ (Key::ArrowLeft | Key::ArrowRight), pressed: true, modifiers, .. }
                        if modifiers.shift && !modifiers.alt && !self.shell_selects() =>
                    {
                        self.select_by_key(*key == Key::ArrowRight, modifiers.ctrl);
                        continue;
                    }
                    Event::Key { pressed: true, .. } | Event::Text(_) | Event::Paste(_) | Event::Cut => self.clear_key_selection(),
                    _ => {}
                }
                if let Event::Key { key, pressed: true, modifiers, .. } = &event {
                    if *key == Key::Space && modifiers.ctrl && !modifiers.shift && !modifiers.alt && look.suggest {
                        self.force_suggest = true;
                        continue;
                    }
                    if *key == Key::Period && modifiers.command && !self.fixes.1.is_empty() {
                        self.open_fixes(None);
                        continue;
                    }
                    let (mut chosen, mut close) = (None, false);
                    if let Some(pick) = self.pick.as_mut().filter(|_| modifiers.is_none()) {
                        let last = pick.rows.len().saturating_sub(1);
                        match key {
                            Key::ArrowDown => pick.selected = Some(pick.selected.map_or(0, |s| (s + 1).min(last))),
                            Key::ArrowUp if pick.selected.is_some() => pick.selected = pick.selected.map(|s| s.saturating_sub(1)),
                            Key::Escape => close = true,
                            Key::Tab => chosen = Some(pick.selected.unwrap_or(0)),
                            Key::Enter if pick.selected.is_some() => chosen = pick.selected,
                            Key::Enter => self.pick = None,
                            _ if pick.fixes => self.pick = None,
                            _ => {}
                        }
                        if let Some(pick) = self.pick.as_mut() {
                            pick.reveal = true;
                        }
                    }
                    if close {
                        self.pick = None;
                        continue;
                    }
                    if let Some(index) = chosen {
                        self.accept(index);
                        continue;
                    }
                    if matches!(key, Key::ArrowDown | Key::ArrowUp) && self.pick.as_ref().is_some_and(|p| p.selected.is_some()) {
                        continue;
                    }
                }
                match event {
                    Event::Text(text) => self.write(text),
                    Event::Paste(text) => self.paste(&text),
                    Event::Copy => {
                        let selection = self.selection();
                        self.clear_key_selection();
                        match selection {
                            Some(text) => {
                                ctx.copy_text(text);
                                self.term.lock().selection = None;
                            }
                            None => self.write([3u8]),
                        }
                    }
                    Event::Cut => self.write([0x18u8]),
                    Event::Key { key: Key::F, pressed: true, modifiers, .. } if modifiers.command && !modifiers.alt => {
                        self.find.show();
                        continue;
                    }
                    Event::Key { key: key @ (Key::ArrowUp | Key::ArrowDown), pressed: true, modifiers, .. }
                        if modifiers.command && !modifiers.shift && !modifiers.alt && self.integration.lock().unwrap().seen =>
                    {
                        self.scroll_to_command(key == Key::ArrowDown);
                        continue;
                    }
                    Event::Key { key: Key::F3, pressed: true, modifiers, .. } if self.find.open => {
                        step = if modifiers.shift { -1 } else { 1 };
                        continue;
                    }
                    Event::Key { key, pressed: true, modifiers, .. } => {
                        if let Some(bytes) = encode_key(key, modifiers, mode.contains(TermMode::APP_CURSOR), self.windows_word_erase) {
                            self.write(bytes);
                            self.term.lock().scroll_display(Scroll::Bottom);
                        }
                    }
                    _ => continue,
                }
                self.last_input = now;
            }
        }

        if look.suggest && self.focused && !self.pick.as_ref().is_some_and(|p| p.fixes) {
            self.refresh_suggest();
        } else if !self.focused && self.pick.as_ref().is_some_and(|p| !p.fixes) {
            (self.pick, self.suggest_for) = (None, None);
        }
        if resp.clicked() {
            self.pick = None;
        }

        let mut scroll_to = None;
        if let Some(fresh) = self.find.due(now) {
            let term = self.term.lock();
            let top = term.topmost_line().0;
            let (text, cells) = crate::terminal_links::join(&grid_rows(&term, top, term.bottommost_line().0));
            drop(term);
            self.find.run(&text, &cells, top, now, fresh);
            if fresh {
                scroll_to = self.find.current.and_then(|c| self.find.matches[c].first()).map(|s| s.0);
            }
        }
        if self.find.open {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        let offset = self.term.lock().grid().display_offset();
        let link = match (ui.input(|i| i.modifiers.command), resp.hover_pos()) {
            (true, Some(pos)) => self.hovered_link(self.point_at(rect, pos, cell, offset).0),
            _ => None,
        };
        if let Some((_, _, link)) = link.as_ref().filter(|_| resp.clicked()) {
            match &link.target {
                Target::Url(url) => ui::open_url(url),
                Target::File(path, place) => {
                    cmds.push(Command::OpenPath(path.clone()));
                    cmds.extend(place.map(|(line, col)| Command::GoTo(line, col)));
                }
            }
        }
        if let Some(pos) = resp.interact_pointer_pos() {
            let (point, side) = self.point_at(rect, pos, cell, offset);
            let mut term = self.term.lock();
            if resp.drag_started() {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                drop(term);
                let (start, start_side) = self.point_at(rect, origin, cell, offset);
                term = self.term.lock();
                term.selection = Some(Selection::new(SelectionType::Simple, start, start_side));
            }
            if resp.dragged() && let Some(selection) = term.selection.as_mut() {
                selection.update(point, side);
            } else if resp.triple_clicked() {
                term.selection = Some(Selection::new(SelectionType::Lines, point, side));
            } else if resp.double_clicked() {
                term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
            } else if resp.clicked() && link.is_none() {
                term.selection = None;
            }
            if resp.secondary_clicked() {
                match look.right_click.as_str() {
                    "selectWord" => term.selection = Some(Selection::new(SelectionType::Semantic, point, side)),
                    "copyPaste" if term.selection.is_some() => {
                        ctx.copy_text(term.selection_to_string().unwrap_or_default());
                        term.selection = None;
                    }
                    "copyPaste" | "paste" => {
                        resp.request_focus();
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    }
                    _ => {}
                }
            }
        }
        if look.copy_on_select && resp.drag_stopped() && let Some(text) = self.selection() {
            ctx.copy_text(text);
        }
        if resp.hovered() {
            ctx.set_cursor_icon(if link.is_some() { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Text });
            self.wheel += ui.input(|i| i.smooth_scroll_delta.y);
            let lines = (self.wheel / cell.y) as i32;
            if lines != 0 {
                self.wheel -= lines as f32 * cell.y;
                if mode.contains(TermMode::ALT_SCREEN) && !mode.intersects(TermMode::MOUSE_MODE) {
                    let key = if lines > 0 { Key::ArrowUp } else { Key::ArrowDown };
                    for _ in 0..lines.unsigned_abs() {
                        if let Some(bytes) = encode_key(key, Modifiers::NONE, mode.contains(TermMode::APP_CURSOR), self.windows_word_erase) {
                            self.write(bytes);
                        }
                    }
                } else {
                    self.term.lock().scroll_display(Scroll::Delta(lines));
                }
            }
        }

        let fade = ui::fade_in(ui, id.with("born"));
        if let Some(color) = self.color {
            ui.painter().vline(rect.left() - 9.0, rect.expand2(vec2(0.0, 4.0)).y_range(), Stroke::new(2.0, theme.ansi[color]));
        }
        let painter = ui.painter().with_clip_rect(rect.expand(2.0));
        let term = self.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cell_pos = |row: i32, column: usize| pos2(rect.left() + column as f32 * cell.x, rect.top() + row as f32 * cell.y);
        let text_dy = (cell.y - rh) / 2.0;
        let mut run = String::new();
        let mut run_at = (0i32, 0usize, Color32::TRANSPARENT);
        let mut bg_run: Option<(i32, usize, usize, Color32)> = None;
        let flush = |run: &mut String, at: (i32, usize, Color32)| {
            if !run.trim_end().is_empty() {
                let pos = cell_pos(at.0, at.1) + vec2(0.0, text_dy);
                painter.text(pos, Align2::LEFT_TOP, run.trim_end(), font.clone(), ui::with_alpha(at.2, fade));
            }
            run.clear();
        };
        let under_text = painter.add(egui::Shape::Noop);
        let mut backgrounds = vec![];
        let mut flush_bg = |bg: &mut Option<(i32, usize, usize, Color32)>| {
            if let Some((row, from, to, color)) = bg.take() {
                backgrounds.push(egui::Shape::rect_filled(Rect::from_min_max(cell_pos(row, from), cell_pos(row + 1, to)), CornerRadius::ZERO, color));
            }
        };
        for indexed in content.display_iter {
            let flags = indexed.flags;
            if flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let row = indexed.point.line.0 + offset;
            let column = indexed.point.column.0;
            let mut fg = resolve(indexed.fg, theme).unwrap_or(theme.text);
            let mut bg = resolve(indexed.bg, theme).filter(|_| indexed.bg != AnsiColor::Named(NamedColor::Background));
            if flags.contains(Flags::INVERSE) {
                (fg, bg) = (bg.unwrap_or(theme.panel), Some(fg));
            }
            if content.selection.is_some_and(|s| s.contains(indexed.point)) {
                bg = Some(theme.selection);
            }
            if flags.intersects(Flags::DIM) {
                fg = ui::mix(fg, theme.panel, 0.4);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = Color32::TRANSPARENT;
            }
            let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            match (&mut bg_run, bg) {
                (Some((r, _, to, c)), Some(color)) if *r == row && *to == column && *c == color => *to += width,
                (_, bg) => {
                    flush_bg(&mut bg_run);
                    bg_run = bg.map(|color| (row, column, column + width, color));
                }
            }
            let c = indexed.c;
            let contiguous = !run.is_empty() && run_at.0 == row && run_at.1 + run.len() == column;
            if c == ' ' || c == '\0' {
                if contiguous {
                    run.push(' ');
                }
                continue;
            }
            if !c.is_ascii() {
                flush(&mut run, run_at);
                painter.text(cell_pos(row, column) + vec2(0.0, text_dy), Align2::LEFT_TOP, c, font.clone(), ui::with_alpha(fg, fade));
                continue;
            }
            if !contiguous || run_at.2 != fg {
                flush(&mut run, run_at);
                run_at = (row, column, fg);
            }
            run.push(c);
            if flags.intersects(Flags::ALL_UNDERLINES) {
                let y = cell_pos(row + 1, column).y - 2.0;
                painter.hline(cell_pos(row, column).x..=cell_pos(row, column + 1).x, y, Stroke::new(1.0, fg));
            }
        }
        flush(&mut run, run_at);
        flush_bg(&mut bg_run);
        for (index, segments) in self.find.matches.iter().enumerate() {
            let alpha = if self.find.current == Some(index) { 0.6 } else { 0.25 };
            for &(line, from, to) in segments.iter().filter(|s| (0..self.size.1 as i32).contains(&(s.0 + offset))) {
                let r = Rect::from_min_max(cell_pos(line + offset, from), cell_pos(line + offset + 1, to));
                backgrounds.push(egui::Shape::rect_filled(r, CornerRadius::same(2), ui::with_alpha(theme.accent, alpha)));
            }
        }
        painter.set(under_text, egui::Shape::Vec(backgrounds));
        if let Some((top, cells, _)) = &link {
            for &(r, c) in cells {
                let row = top + r as i32 + offset;
                painter.hline(cell_pos(row, c).x..=cell_pos(row, c + 1).x, cell_pos(row + 1, c).y - 2.0, Stroke::new(1.0, theme.accent));
            }
        }

        if !content.mode.contains(TermMode::ALT_SCREEN) {
            let mut integration = self.integration.lock().unwrap();
            let (dropped, history) = (integration.dropped, term.history_size() as i64);
            integration.pictures.retain(|p| p.line + (p.size.1 / cell.y).ceil() as i64 > dropped);
            for (n, picture) in integration.pictures.iter_mut().enumerate() {
                let row = picture.line - dropped - history + offset as i64;
                if row > self.size.1 as i64 || (row as f32) * cell.y + picture.size.1 < 0.0 {
                    continue;
                }
                let texture = picture
                    .texture
                    .get_or_insert_with(|| ctx.load_texture(format!("terminal-image-{}-{n}", self.id), picture.image.take().unwrap_or_default(), egui::TextureOptions::LINEAR));
                let area = Rect::from_min_size(cell_pos(row as i32, picture.column), vec2(picture.size.0, picture.size.1));
                painter.image(texture.id(), area, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), ui::with_alpha(Color32::WHITE, fade));
            }
        }

        if content.mode.contains(TermMode::SHOW_CURSOR) && content.cursor.shape != CursorShape::Hidden {
            let point = content.cursor.point;
            let min = cell_pos(point.line.0 + offset, point.column.0);
            let block = Rect::from_min_size(min, cell);
            let idle = now - self.last_input;
            let blink = if !look.blink || idle < 0.5 || idle > 30.0 { 1.0 } else { 0.55 + 0.45 * ((idle - 0.5) * std::f64::consts::TAU / 1.2).cos() as f32 };
            if !self.focused {
                painter.rect_stroke(block, CornerRadius::ZERO, Stroke::new(1.0, theme.text_dim), StrokeKind::Inside);
            } else {
                let shape = match (content.cursor.shape, look.cursor.as_str()) {
                    (CursorShape::Beam, _) | (CursorShape::Block, "line") => Rect::from_min_size(min, vec2(2.0, cell.y)),
                    (CursorShape::Underline, _) | (CursorShape::Block, "underline") => Rect::from_min_max(pos2(min.x, block.bottom() - 2.0), block.max),
                    _ => block,
                };
                let strength = if shape == block { 0.55 } else { 1.0 };
                painter.rect_filled(shape, CornerRadius::ZERO, ui::with_alpha(theme.accent, blink * strength));
                if idle <= 30.0 && look.blink {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
        let marks: Vec<(i64, Record)> = {
            let integration = self.integration.lock().unwrap();
            let base = integration.dropped + term.history_size() as i64 - offset as i64;
            let shown = |r: &&Record| r.ran() && (0..self.size.1 as i64).contains(&(r.row() - base));
            integration.commands.iter().filter(shown).map(|r| (r.row() - base, r.clone())).collect()
        };
        let caret = cell_pos(content.cursor.point.line.0 + offset, content.cursor.point.column.0) + vec2(0.0, cell.y + 2.0);
        let sticky = match look.sticky && !content.mode.contains(TermMode::ALT_SCREEN) {
            true => {
                let integration = self.integration.lock().unwrap();
                let (base, top) = (integration.dropped + term.history_size() as i64, -(offset as i64));
                let record = integration.commands.iter().rev().find(|r| r.ran() && r.row() - base < top);
                record.filter(|r| r.end.is_none_or(|(end, _)| end - base > top)).map(|r| {
                    let line = r.row() - base;
                    let text = match line >= term.topmost_line().0 as i64 {
                        true => grid_rows(&term, line as i32, line as i32).pop().map(|row| row.0.into_iter().filter(|c| *c != '\0').collect::<String>()),
                        false => None,
                    };
                    (line, text.map(|t| t.trim_end().to_owned()).unwrap_or_else(|| r.line.clone()))
                })
            }
            false => None,
        };
        drop(term);
        let last = self.integration.lock().unwrap().last_finished().cloned();
        if let Some(record) = last.filter(|r| r.row() != self.fixes.0) {
            let output = self.output_of(&record).unwrap_or_default();
            self.fixes = (record.row(), crate::terminal_quickfix::find(&record.line, &output));
        }
        for (row, record) in marks {
            let y = rect.top() + row as f32 * cell.y;
            if record.row() == self.fixes.0 && !self.fixes.1.is_empty() {
                let bulb = Rect::from_center_size(pos2(rect.right() - 8.0, y + cell.y / 2.0), vec2(16.0, cell.y.max(16.0)));
                let hit = ui.interact(bulb, id.with("bulb"), Sense::click()).on_hover_text("Quick fixes (Ctrl+.)");
                ui.painter().text(bulb.center(), Align2::CENTER_CENTER, icons::LIGHTBULB, FontId::proportional(13.0), theme.warning);
                if hit.clicked() {
                    self.open_fixes(Some(pos2((bulb.right() - 462.0).max(rect.left()), bulb.bottom())));
                }
            }
            let mark = Rect::from_min_max(pos2(rect.left() - 8.0, y + 2.0), pos2(rect.left() - 5.0, y + cell.y - 2.0));
            let color = match (record.end, record.failed()) {
                (None, _) => theme.text_dim,
                (_, true) => theme.error,
                _ => theme.accent,
            };
            ui.painter().rect_filled(mark, CornerRadius::same(1), color);
            let mut tip = if record.line.is_empty() { "Command".to_owned() } else { record.line.clone() };
            match (record.end, record.duration, record.started) {
                (None, _, Some(started)) => tip += &format!("\nRunning for {:.1} s", started.elapsed().as_secs_f32()),
                (_, Some(took), _) => tip += &format!("\nTook {:.1} s", took.as_secs_f32()),
                _ => {}
            }
            if let Some(code) = record.exit {
                tip += &format!("\nExit code {code}");
            }
            ui.interact(mark.expand2(vec2(3.0, 0.0)), id.with(("mark", row)), Sense::hover()).on_hover_text(tip);
        }
        if let Some((line, text)) = sticky {
            let bar = Rect::from_min_size(rect.min, vec2(rect.width(), cell.y));
            let hit = ui.interact(bar, id.with("sticky"), Sense::click()).on_hover_text("Scroll to this command").on_hover_cursor(egui::CursorIcon::PointingHand);
            let fill = if hit.hovered() { ui::mix(theme.panel, theme.text, 0.06) } else { theme.panel };
            ui.painter().rect_filled(bar.expand2(vec2(4.0, 0.0)), CornerRadius::ZERO, fill);
            ui.painter().hline(bar.x_range(), bar.bottom(), Stroke::new(1.0, theme.border));
            ui.painter().with_clip_rect(bar).text(bar.min + vec2(0.0, text_dy), Align2::LEFT_TOP, text, font.clone(), theme.text);
            if hit.clicked() {
                let mut term = self.term.lock();
                let (history, offset) = (term.history_size() as i64, term.grid().display_offset() as i64);
                term.scroll_display(Scroll::Delta(((-line).clamp(0, history) - offset) as i32));
            }
        }
        let clicked = self.pick.as_mut().and_then(|pick| {
            let rows: Vec<(&str, &str, &str)> = pick.rows.iter().map(|r| (r.1, r.0.as_str(), r.2)).collect();
            let reveal = std::mem::take(&mut pick.reveal);
            let selected = (pick.selected.unwrap_or(usize::MAX), 8);
            crate::editor::list_popup(&ctx, theme, &format!("terminal_pick_{}", self.id), (pick.at.unwrap_or(caret), cell.y), &rows, selected, reveal, &font).0
        });
        if let Some(index) = clicked {
            self.accept(index);
        }
        if self.find.open {
            let (bar_step, closed) = self.find.ui(ui, rect.expand2(vec2(10.0, 4.0)), theme);
            step += bar_step;
            self.want_focus |= closed;
        }
        if step != 0 {
            scroll_to = self.find.step(step > 0);
        }
        if let Some(line) = scroll_to {
            self.reveal(line);
            ctx.request_repaint();
        }
    }
}

pub fn color_names() -> [(Option<usize>, &'static str); 9] {
    [
        (None, "None"), (Some(1), "Red"), (Some(2), "Green"), (Some(3), "Yellow"), (Some(4), "Blue"),
        (Some(5), "Magenta"), (Some(6), "Cyan"), (Some(7), "White"), (Some(0), "Black"),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelTab {
    Problems,
    Output,
    Debug,
    Terminal,
    Ports,
    Tests,
}

struct Group {
    panes: Vec<Terminal>,
    active: usize,
    profile: ShellProfile,
}

struct PortRow {
    port: u16,
    address: String,
    pid: String,
    process: String,
}

#[cfg_attr(not(windows), allow(unused_mut))]
pub fn hidden_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn output_of(program: &str, args: &[&str]) -> String {
    hidden_command(program).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

#[cfg(any(windows, test))]
fn parse_netstat(text: &str) -> Vec<(String, u16, String)> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 || parts[0] != "TCP" || parts[3] != "LISTENING" {
                return None;
            }
            let (address, port) = parts[1].rsplit_once(':')?;
            Some((address.to_owned(), port.parse().ok()?, parts[4].to_owned()))
        })
        .collect()
}

#[cfg(windows)]
fn list_ports() -> Vec<PortRow> {
    let names: HashMap<String, String> = output_of("tasklist", &["/FO", "CSV", "/NH"])
        .lines()
        .filter_map(|line| {
            let mut parts = line.split("\",\"");
            let name = parts.next()?.trim_start_matches('"').to_owned();
            Some((parts.next()?.trim_matches('"').to_owned(), name))
        })
        .collect();
    let mut rows: Vec<PortRow> = parse_netstat(&output_of("netstat", &["-ano", "-p", "TCP"]))
        .into_iter()
        .map(|(address, port, pid)| PortRow { port, address, process: names.get(&pid).cloned().unwrap_or_default(), pid })
        .collect();
    rows.sort_by_key(|r| r.port);
    rows.dedup_by_key(|r| r.port);
    rows
}

#[cfg(not(windows))]
fn list_ports() -> Vec<PortRow> {
    let mut rows: Vec<PortRow> = output_of("ss", &["-ltnpH"])
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let (address, port) = parts.get(3)?.rsplit_once(':')?;
            let process = parts.get(5).and_then(|p| p.split('"').nth(1)).unwrap_or("").to_owned();
            let pid = parts.get(5).and_then(|p| p.split("pid=").nth(1)).and_then(|p| p.split(',').next()).unwrap_or("").to_owned();
            Some(PortRow { port: port.parse().ok()?, address: address.to_owned(), pid, process })
        })
        .collect();
    rows.sort_by_key(|r| r.port);
    rows.dedup_by_key(|r| r.port);
    rows
}

fn default_profile(settings: &Settings, cwd: Option<&Path>) -> Option<ShellProfile> {
    match cwd.and_then(crate::wsl::parse) {
        Some((distro, dir)) => Some(ShellProfile { name: distro.clone(), program: "wsl.exe".into(), args: vec!["-d".into(), distro, "--cd".into(), dir] }),
        None => settings.default_profile().cloned(),
    }
}

pub struct BottomPanel {
    pub tab: PanelTab,
    groups: Vec<Group>,
    active: usize,
    next_id: u64,
    started: bool,
    error: Option<String>,
    focused: bool,
    ports: Vec<PortRow>,
    ports_rx: Option<Receiver<Vec<PortRow>>>,
    ports_loaded: bool,
    run_id: Option<u64>,
    run_line: String,
    debug: String,
    pub console: crate::debug::Console,
    pub test_output: String,
    integrate: bool,
    renaming: Option<(u64, String, bool)>,
    problems: ProblemFilter,
}

#[derive(Default)]
struct ProblemFilter {
    text: String,
    hide: [bool; 3],
    active_only: bool,
}

impl ProblemFilter {
    fn shows(&self, path: &Path, problem: &Diagnostic) -> bool {
        let needle = self.text.trim().to_lowercase();
        let found = |text: &str| text.to_lowercase().contains(&needle);
        problem.severity <= 3
            && !self.hide[(problem.severity as usize).clamp(1, 3) - 1]
            && (needle.is_empty() || found(&problem.message) || found(&problem.source) || found(&path.to_string_lossy()))
    }
}

impl BottomPanel {
    pub fn new() -> Self {
        Self {
            tab: PanelTab::Terminal,
            groups: vec![],
            active: 0,
            next_id: 1,
            started: false,
            error: None,
            focused: false,
            ports: vec![],
            ports_rx: None,
            ports_loaded: false,
            run_id: None,
            run_line: String::new(),
            debug: String::new(),
            console: Default::default(),
            test_output: String::new(),
            integrate: true,
            renaming: None,
            problems: ProblemFilter::default(),
        }
    }

    pub fn has_focus(&self) -> bool {
        self.focused
    }

    pub fn blur(&mut self) {
        self.focused = false;
    }

    pub fn show_tests(&mut self) {
        self.tab = PanelTab::Tests;
    }

    pub fn show_debug(&mut self) {
        self.tab = PanelTab::Debug;
    }

    pub fn launch(&mut self, name: &str, program: &str, args: &[String], cwd: Option<&Path>, env: &[(String, String)], ctx: &egui::Context) -> Result<Option<u32>, String> {
        let id = self.next_id;
        self.next_id += 1;
        let terminal = Terminal::spawn_with(id, name, program, args, cwd, env, ctx.clone()).map_err(|e| format!("Could not start {program}: {e}"))?;
        let pid = terminal.pid;
        let profile = ShellProfile { name: name.to_owned(), program: program.to_owned(), args: args.to_vec() };
        self.groups.push(Group { panes: vec![terminal], active: 0, profile });
        (self.active, self.started, self.tab) = (self.groups.len() - 1, true, PanelTab::Terminal);
        Ok(pid)
    }

    fn spawn(&mut self, profile: &ShellProfile, cwd: Option<&Path>, ctx: &egui::Context) -> Option<Terminal> {
        let id = self.next_id;
        self.next_id += 1;
        match Terminal::spawn_shell(id, profile, cwd, self.integrate, "", ctx.clone()) {
            Ok(terminal) => {
                self.error = None;
                Some(terminal)
            }
            Err(error) => {
                self.error = Some(format!("Could not start {} ({}): {error}", profile.name, profile.program));
                None
            }
        }
    }

    fn new_group(&mut self, profile: Option<&ShellProfile>, cwd: Option<&Path>, ctx: &egui::Context) {
        self.tab = PanelTab::Terminal;
        self.started = true;
        let Some(profile) = profile.cloned() else {
            self.error = Some("No terminal profile is configured in settings.json.".into());
            return;
        };
        if let Some(terminal) = self.spawn(&profile, cwd, ctx) {
            self.groups.push(Group { panes: vec![terminal], active: 0, profile });
            self.active = self.groups.len() - 1;
        }
    }

    fn split(&mut self, settings: &Settings, cwd: Option<&Path>, ctx: &egui::Context) {
        let Some(profile) = self.groups.get(self.active).filter(|g| g.panes.iter().all(|p| p.task.is_none())).map(|g| g.profile.clone()) else {
            self.new_group(default_profile(settings, cwd).as_ref(), cwd, ctx);
            return;
        };
        let here = self.active_pane().and_then(|p| p.cwd());
        if let Some(terminal) = self.spawn(&profile, here.as_deref().or(cwd), ctx) {
            let group = &mut self.groups[self.active];
            group.panes.push(terminal);
            group.active = group.panes.len() - 1;
        }
    }

    pub fn run(&mut self, line: &str, settings: &Settings, cwd: Option<&Path>, ctx: &egui::Context) {
        let find = |groups: &[Group], id: Option<u64>| {
            let pane = |g: &Group| g.panes.iter().position(|p| Some(p.id) == id && p.is_alive());
            groups.iter().enumerate().find_map(|(index, g)| Some((index, pane(g)?)))
        };
        self.integrate = settings.shell_integration;
        if find(&self.groups, self.run_id).is_none() {
            self.run_id = Some(self.next_id);
            self.new_group(default_profile(settings, cwd).as_ref(), cwd, ctx);
        }
        let Some((index, pane)) = find(&self.groups, self.run_id) else { return };
        self.tab = PanelTab::Terminal;
        self.active = index;
        self.groups[index].active = pane;
        self.groups[index].panes[pane].write(format!("{line}\r"));
        self.groups[index].panes[pane].focus();
        self.run_line = line.to_owned();
        self.console.shown = false;
    }

    fn task_position(&self, matches: impl Fn(&Terminal) -> bool) -> Option<(usize, usize)> {
        self.groups.iter().enumerate().find_map(|(g, group)| Some((g, group.panes.iter().position(&matches)?)))
    }

    pub fn run_task(
        &mut self,
        label: &str,
        (program, args): (&str, &[String]),
        env: &std::collections::BTreeMap<String, String>,
        cwd: Option<&Path>,
        reveal: bool,
        ctx: &egui::Context,
    ) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        let env: HashMap<String, String> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let terminal = Terminal::start(id, label, program, args, &env, cwd, Some(label), "", ctx.clone())
            .map_err(|e| format!("Could not start task {label} ({program}): {e}"))?;
        self.started = true;
        match self.task_position(|p| p.task.as_deref() == Some(label)) {
            Some((g, p)) => {
                self.groups[g].panes[p] = terminal;
                self.groups[g].active = p;
                self.active = g;
            }
            None => {
                let profile = ShellProfile { name: label.to_owned(), program: program.to_owned(), args: args.to_vec() };
                self.groups.push(Group { panes: vec![terminal], active: 0, profile });
                self.active = self.groups.len() - 1;
            }
        }
        if reveal {
            self.tab = PanelTab::Terminal;
        }
        Ok(id)
    }

    pub fn reveal_task(&mut self, id: u64) {
        if let Some((g, p)) = self.task_position(|t| t.id == id) {
            (self.tab, self.active, self.groups[g].active) = (PanelTab::Terminal, g, p);
            self.groups[g].panes[p].focus();
        }
    }

    pub fn task_pane(&self, id: u64) -> Option<&Terminal> {
        self.groups.iter().flat_map(|g| &g.panes).find(|p| p.id == id)
    }

    pub fn finish_task(&mut self, id: u64, label: &str, code: Option<i32>) {
        let Some((g, p)) = self.task_position(|t| t.id == id) else { return };
        let pane = &mut self.groups[g].panes[p];
        let status = code.map_or("terminated".to_owned(), |c| format!("exit {c}"));
        pane.name = format!("{label} ({status})");
        pane.note(&format!("\r\n\x1b[2m[Task {label} finished: {status}]\x1b[0m\r\n"));
    }

    pub fn kill_task(&mut self, id: u64) {
        if let Some((g, p)) = self.task_position(|t| t.id == id) {
            self.groups[g].panes.remove(p);
            self.prune();
        }
    }

    fn debug_ui(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, font_size: f32) {
        if self.console.shown {
            return self.console.ui(ui, body, theme, font_size);
        }
        if let Some(pane) = self.groups.iter().flat_map(|g| &g.panes).find(|p| Some(p.id) == self.run_id) {
            self.debug = pane.text();
        }
        if self.run_line.is_empty() {
            return empty(ui, theme, "Output of Run File and tasks is mirrored here.");
        }
        ui.scope_builder(UiBuilder::new().max_rect(body.shrink2(vec2(14.0, 4.0))), |ui| {
            ui.label(RichText::new(format!("{}  {}", icons::PLAY, self.run_line)).color(theme.accent));
            ui.add_space(4.0);
            ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                ui.label(RichText::new(&self.debug).font(FontId::monospace(font_size)).color(theme.text));
            });
        });
    }

    fn kill_active(&mut self) {
        if let Some(group) = self.groups.get_mut(self.active)
            && group.active < group.panes.len()
        {
            group.panes.remove(group.active);
            group.active = group.active.saturating_sub(1);
            if let Some(pane) = group.panes.get_mut(group.active) {
                pane.focus();
            }
        }
        self.prune();
    }

    fn prune(&mut self) {
        for group in &mut self.groups {
            group.panes.retain(|p| p.is_alive() || p.task.is_some());
            group.active = group.active.min(group.panes.len().saturating_sub(1));
        }
        self.groups.retain(|g| !g.panes.is_empty());
        self.active = self.active.min(self.groups.len().saturating_sub(1));
    }

    fn active_pane(&mut self) -> Option<&mut Terminal> {
        let group = self.groups.get_mut(self.active)?;
        group.panes.get_mut(group.active)
    }

    pub fn recent(&mut self) -> Vec<String> {
        self.active_pane().map(|p| p.recent()).unwrap_or_default()
    }

    pub fn persisted(&self, limit: usize) -> Vec<Saved> {
        let panes = self.groups.iter().enumerate().flat_map(|(g, group)| group.panes.iter().enumerate().map(move |(p, pane)| (g, p, group.active == p, pane)));
        panes.filter_map(|(g, p, active, pane)| Some(Saved { place: Place::Panel(g, p), active: active && g == self.active, ..pane.persist(limit)? })).collect()
    }

    pub fn create(&mut self, settings: &Settings, cwd: Option<&Path>, ctx: &egui::Context) -> Result<Terminal, String> {
        self.integrate = settings.shell_integration;
        let profile = default_profile(settings, cwd).ok_or("No terminal profile is configured in settings.json.")?;
        self.spawn(&profile, cwd, ctx).ok_or_else(|| self.error.take().unwrap_or_default())
    }

    pub fn take_active(&mut self) -> Option<Terminal> {
        let group = self.groups.get_mut(self.active).filter(|g| g.panes.get(g.active).is_some_and(|p| p.task.is_none()))?;
        let pane = group.panes.remove(group.active);
        self.prune();
        Some(pane)
    }

    pub fn adopt(&mut self, mut terminal: Terminal) {
        let profile = terminal.profile.clone().unwrap_or_else(|| ShellProfile { name: terminal.name.clone(), program: String::new(), args: vec![] });
        terminal.focus();
        self.groups.push(Group { panes: vec![terminal], active: 0, profile });
        (self.active, self.started, self.tab) = (self.groups.len() - 1, true, PanelTab::Terminal);
    }

    fn revive_one(&mut self, saved: &Saved, settings: &Settings, root: Option<&Path>, ctx: &egui::Context) -> Option<Terminal> {
        let cwd = saved.cwd.clone().filter(|dir| dir.is_dir()).or_else(|| root.map(Path::to_path_buf));
        let wanted = saved.profile.as_ref().map(|p| p.name.as_str());
        let profile = wanted.and_then(|name| settings.terminal(name)).cloned().or_else(|| default_profile(settings, cwd.as_deref()))?;
        let note = match wanted.filter(|name| *name != profile.name) {
            Some(name) => format!("{} (the profile {name} no longer exists, {} is used)", crate::terminal_restore::MARKER, profile.name),
            None => crate::terminal_restore::MARKER.to_owned(),
        };
        let id = self.next_id;
        self.next_id += 1;
        let prelude = format!("{}\x1b[0;2m{note}\x1b[0m\r\n", saved.text);
        let mut terminal = match Terminal::spawn_shell(id, &profile, cwd.as_deref(), self.integrate, &prelude, ctx.clone()) {
            Ok(terminal) => terminal,
            Err(error) => {
                self.error = Some(format!("Could not restore {} ({}): {error}", profile.name, profile.program));
                return None;
            }
        };
        (terminal.label, terminal.color, terminal.want_focus) = (saved.label.clone(), saved.color, false);
        *terminal.shared.title.lock().unwrap() = Some(saved.title.clone()).filter(|t| !t.is_empty());
        terminal.integration.lock().unwrap().restored = saved.recent.clone();
        Some(terminal)
    }

    pub fn revive(&mut self, all: &[Saved], settings: &Settings, root: Option<&Path>, ctx: &egui::Context) -> Vec<(Place, bool, Terminal)> {
        self.integrate = settings.shell_integration;
        let (mut elsewhere, mut last) = (vec![], None);
        for saved in all {
            let Some(terminal) = self.revive_one(saved, settings, root, ctx) else { continue };
            let Place::Panel(group, _) = saved.place else {
                elsewhere.push((saved.place, saved.active, terminal));
                continue;
            };
            match self.groups.last_mut().filter(|_| last == Some(group)) {
                Some(open) => open.panes.push(terminal),
                None => {
                    let profile = terminal.profile.clone().unwrap_or_else(|| ShellProfile { name: terminal.name.clone(), program: String::new(), args: vec![] });
                    self.groups.push(Group { panes: vec![terminal], active: 0, profile });
                }
            }
            (last, self.started) = (Some(group), true);
            let (g, p) = (self.groups.len() - 1, self.groups.last().map_or(0, |g| g.panes.len() - 1));
            if saved.active {
                (self.active, self.groups[g].active) = (g, p);
            }
        }
        elsewhere
    }

    pub fn exec(&mut self, cmd: &Command, settings: &Settings, cwd: Option<&Path>, ctx: &egui::Context) -> bool {
        self.integrate = settings.shell_integration;
        match cmd {
            Command::ScrollToPrevMark | Command::ScrollToNextMark | Command::CopyLastCommandOutput | Command::RunCommandLine(_) | Command::SetTerminalColor(_) | Command::ClearRestoredHistory => {
                let Some(pane) = self.active_pane() else { return false };
                pane.exec(cmd, ctx);
                self.tab = PanelTab::Terminal;
            }
            Command::RenameTerminal => {
                let Some(pane) = self.active_pane() else { return false };
                let rename = (pane.id, pane.title(), true);
                (self.renaming, self.tab) = (Some(rename), PanelTab::Terminal);
            }
            Command::NewTerminal => self.new_group(default_profile(settings, cwd).as_ref(), cwd, ctx),
            Command::NewTerminalWith(name) => self.new_group(settings.terminal(name), cwd, ctx),
            Command::SplitTerminal => {
                self.tab = PanelTab::Terminal;
                self.split(settings, cwd, ctx);
            }
            Command::KillTerminal => self.kill_active(),
            Command::Copy | Command::Paste if self.focused => {
                let paste = *cmd == Command::Paste;
                let Some(pane) = self.active_pane() else { return false };
                if paste {
                    pane.focus();
                    ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
                } else if let Some(text) = pane.selection() {
                    ctx.copy_text(text);
                }
            }
            _ => return false,
        }
        ctx.request_repaint();
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn header(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        settings: &Settings,
        cwd: Option<&Path>,
        labels: &[&str],
        index: &mut usize,
        tab: Option<PanelTab>,
        cmds: &mut Vec<Command>,
    ) -> (Rect, Option<usize>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let ctx = ui.ctx().clone();
        let header = Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), 32.0));
        let body = Rect::from_min_max(pos2(header.left(), header.bottom()), ui.max_rect().max);
        ui.allocate_rect(ui.max_rect(), Sense::hover());
        let mut dragged = None;

        ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(10.0, 0.0))), |ui| {
            ui.horizontal_centered(|ui| {
                dragged = ui::tabs(ui, theme, Id::new("panel_tab"), labels, index);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let behind = ui.painter().add(egui::Shape::Noop);
                    if ui::icon_button(ui, theme, icons::X, 24.0, false, "Close Panel (Ctrl+J)").clicked() {
                        cmds.push(Command::TogglePanel);
                    }
                    if tab == Some(PanelTab::Terminal) {
                        if ui::icon_button(ui, theme, icons::TRASH, 24.0, false, "Kill Terminal").clicked() {
                            self.kill_active();
                        }
                        if ui::icon_button(ui, theme, icons::SQUARE_SPLIT_HORIZONTAL, 24.0, false, "Split Terminal").clicked() {
                            self.split(settings, cwd, &ctx);
                        }
                        ui.spacing_mut().button_padding = vec2(5.0, 5.0);
                        ui.menu_button(RichText::new(icons::CARET_DOWN).size(12.0).color(theme.text_dim), |ui| {
                            ui.set_min_width(200.0);
                            ui::menu_pad(ui);
                            for profile in &settings.terminals {
                                if ui.button(&profile.name).on_hover_text(format!("{} {}", profile.program, profile.args.join(" "))).clicked() {
                                    cmds.push(Command::NewTerminalWith(profile.name.clone()));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Launch Profile...");
                        if ui::icon_button(ui, theme, icons::PLUS, 24.0, false, "New Terminal (Ctrl+Shift+`)").clicked() {
                            self.new_group(default_profile(settings, cwd).as_ref(), cwd, &ctx);
                        }
                    } else if tab == Some(PanelTab::Ports)
                        && ui::icon_button(ui, theme, icons::ARROWS_CLOCKWISE, 24.0, false, "Refresh Ports").clicked()
                    {
                        self.ports_loaded = false;
                    }
                    let used = ui.min_rect().expand2(vec2(8.0, 0.0));
                    ui.painter().set(behind, egui::Shape::rect_filled(used, CornerRadius::ZERO, theme.panel));
                });
            });
        });
        (body, dragged)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn body(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        tab: PanelTab,
        theme: &Theme,
        settings: &Settings,
        cwd: Option<&Path>,
        lsp: &Lsp,
        task_files: &[(PathBuf, Vec<Diagnostic>)],
        active: Option<&Path>,
        cmds: &mut Vec<Command>,
    ) {
        self.integrate = settings.shell_integration;
        self.prune();
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            match tab {
                PanelTab::Terminal => self.terminals(ui, body, theme, settings, cwd, cmds),
                PanelTab::Ports => self.ports_ui(ui, body, theme),
                PanelTab::Problems => problems_ui(ui, theme, lsp, task_files, cwd, active, &mut self.problems, cmds),
                PanelTab::Output => crate::output::ui(ui, theme, settings.font_size, cmds),
                PanelTab::Debug => self.debug_ui(ui, body, theme, settings.font_size),
                PanelTab::Tests => test_output_ui(ui, theme, &self.test_output, settings.font_size),
            }
        });
    }

    fn terminals(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, settings: &Settings, cwd: Option<&Path>, cmds: &mut Vec<Command>) {
        let ctx = ui.ctx().clone();
        if self.groups.is_empty() && !self.started {
            self.new_group(default_profile(settings, cwd).as_ref(), cwd, &ctx);
        }
        let mut area = body;
        if let Some(error) = self.error.clone() {
            let bar = Rect::from_min_max(pos2(body.left(), body.bottom() - 28.0), body.max);
            area.max.y = bar.top();
            ui.painter().rect_filled(bar, CornerRadius::ZERO, ui::mix(theme.panel, theme.error, 0.18));
            ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(10.0, 2.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                if ui::icon_button(ui, theme, icons::X, 22.0, false, "Dismiss").clicked() {
                    self.error = None;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&error).color(theme.error)).truncate());
                });
            });
        }
        if self.groups.is_empty() {
            ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(18.0);
                    ui.label(RichText::new("No terminal is open.").color(theme.text_dim));
                    ui.add_space(6.0);
                    if ui.button(RichText::new("New Terminal").color(theme.accent)).clicked() {
                        self.new_group(default_profile(settings, cwd).as_ref(), cwd, &ctx);
                    }
                });
            });
            return;
        }

        let tasks = self.groups.iter().flat_map(|g| &g.panes).any(|p| p.task.is_some());
        if tasks || self.renaming.is_some() || self.groups.iter().map(|g| g.panes.len()).sum::<usize>() > 1 {
            let list = Rect::from_min_max(pos2(area.right() - 190.0, area.top()), area.max);
            area.max.x = list.left();
            ui.painter().vline(list.left(), list.y_range(), Stroke::new(1.0, theme.border));
            let mut kill = None;
            ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    for (index, group) in self.groups.iter_mut().enumerate() {
                        let count = group.panes.len();
                        for at in 0..count {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                            ui::row_bg(ui, theme, r, resp.id, resp.hovered(), index == self.active && at == group.active);
                            ui::item(ui, &resp, ui::Role::ListItem, &group.panes[at].title(), index == self.active && at == group.active);
                            let (icon, font) = match at {
                                _ if count == 1 => (icons::TERMINAL_WINDOW, FontId::proportional(14.0)),
                                0 => ("┌", FontId::monospace(13.0)),
                                _ if at + 1 == count => ("└", FontId::monospace(13.0)),
                                _ => ("├", FontId::monospace(13.0)),
                            };
                            let p = ui.painter().with_clip_rect(r.shrink2(vec2(26.0, 0.0)).translate(vec2(-16.0, 0.0)));
                            let pane = &mut group.panes[at];
                            let failed = pane.failed();
                            ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, icon, font, pane.tint(theme));
                            match self.renaming.as_mut().filter(|(id, _, _)| *id == pane.id) {
                                Some((_, text, focus)) => {
                                    let field = ui.put(r.shrink2(vec2(28.0, 2.0)).translate(vec2(2.0, 0.0)), TextEdit::singleline(text).margin(ui::FIELD));
                                    if std::mem::take(focus) {
                                        field.request_focus();
                                    }
                                    if field.lost_focus() {
                                        if !ui.input(|i| i.key_pressed(Key::Escape)) {
                                            pane.label = Some(text.trim().to_owned()).filter(|t| !t.is_empty());
                                        }
                                        self.renaming = None;
                                        pane.focus();
                                    }
                                }
                                None => {
                                    p.text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, pane.title(), FontId::proportional(12.5), theme.text);
                                }
                            }
                            let mut to_editor = false;
                            resp.context_menu(|ui| {
                                ui::menu_pad(ui);
                                if ui.button("Rename...").clicked() {
                                    self.renaming = Some((pane.id, pane.title(), true));
                                }
                                ui.menu_button("Change Color", |ui| {
                                    for (color, name) in color_names() {
                                        if ui.button(name).clicked() {
                                            pane.color = color;
                                        }
                                    }
                                });
                                if pane.task.is_none() && ui.button("Move Terminal into Editor Area").clicked() {
                                    to_editor = true;
                                }
                                if ui.button("Kill Terminal").clicked() {
                                    kill = Some((index, at));
                                }
                            });
                            if to_editor {
                                (self.active, group.active) = (index, at);
                                cmds.push(Command::MoveTerminalToEditor);
                            }
                            let trash = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 20.0));
                            if resp.hovered() {
                                let over = ui.rect_contains_pointer(trash);
                                ui.painter().text(trash.center(), Align2::CENTER_CENTER, icons::TRASH, FontId::proportional(13.0), if over { theme.text } else { theme.text_dim });
                                if resp.clicked() && over {
                                    kill = Some((index, at));
                                }
                            }
                            if resp.clicked() && kill.is_none() {
                                self.active = index;
                                group.active = at;
                                group.panes[at].focus();
                            }
                            let mut tip = group.panes[at].cwd().map(|dir| dir.display().to_string()).unwrap_or_default();
                            if failed {
                                tip += "\nThe last command failed";
                            }
                            resp.on_hover_text(tip.trim());
                        }
                    }
                });
            });
            if let Some((index, at)) = kill {
                self.groups[index].panes.remove(at);
                self.prune();
            }
        }

        let Some(group) = self.groups.get_mut(self.active) else { return };
        let count = group.panes.len();
        let width = area.width() / count as f32;
        for (index, pane) in group.panes.iter_mut().enumerate() {
            let rect = Rect::from_min_size(pos2(area.left() + index as f32 * width, area.top()), vec2(width, area.height()));
            ui.scope_builder(UiBuilder::new().max_rect(rect).id_salt(("terminal_pane", pane.id)), |ui| {
                pane.ui(ui, theme, settings.font_size, cmds);
            });
            if index > 0 {
                ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, theme.border));
            }
            if pane.has_focus() {
                group.active = index;
                self.focused = true;
            }
        }
        if count > 1 {
            let left = area.left() + group.active as f32 * width;
            let color = if self.focused { theme.accent } else { theme.text_dim };
            ui.painter().hline(left..=left + width, area.top() + 1.0, Stroke::new(2.0, color));
        }
    }

    fn ports_ui(&mut self, ui: &mut Ui, body: Rect, theme: &Theme) {
        if !self.ports_loaded && self.ports_rx.is_none() {
            let (tx, rx) = channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = tx.send(list_ports());
                ctx.request_repaint();
            });
            self.ports_rx = Some(rx);
        }
        if let Some(rows) = self.ports_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.ports = rows;
            self.ports_rx = None;
            self.ports_loaded = true;
        }
        if !self.ports_loaded {
            empty(ui, theme, "Looking for listening ports...");
            return;
        }
        if self.ports.is_empty() {
            empty(ui, theme, "No listening TCP ports were found.");
            return;
        }
        let columns = [(0.0, "PORT"), (90.0, "ADDRESS"), (290.0, "PID"), (380.0, "PROCESS")];
        ui.scope_builder(UiBuilder::new().max_rect(body.shrink2(vec2(14.0, 2.0))), |ui| {
            let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            for (x, title) in columns {
                ui.painter().text(pos2(head.left() + x, head.center().y), Align2::LEFT_CENTER, title, FontId::proportional(11.0), theme.text_dim);
            }
            ScrollArea::vertical().auto_shrink(false).show_rows(ui, 22.0, self.ports.len(), |ui, range| {
                for row in &self.ports[range] {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                    ui::row_bg(ui, theme, r, resp.id, resp.hovered(), false);
                    let values = [row.port.to_string(), row.address.clone(), row.pid.clone(), row.process.clone()];
                    ui::item(ui, &resp, ui::Role::ListItem, &format!("Port {} {}", row.port, row.process), false);
                    for ((x, _), value) in columns.iter().zip(values) {
                        let color = if *x == 0.0 { theme.accent } else { theme.text };
                        ui.painter().text(pos2(r.left() + x, r.center().y), Align2::LEFT_CENTER, value, FontId::proportional(12.5), color);
                    }
                }
            });
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn problems_ui(
    ui: &mut Ui,
    theme: &Theme,
    lsp: &Lsp,
    task_files: &[(PathBuf, Vec<Diagnostic>)],
    cwd: Option<&Path>,
    active: Option<&Path>,
    filter: &mut ProblemFilter,
    cmds: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.add(TextEdit::singleline(&mut filter.text).hint_text("Filter (e.g. text, path, source)").desired_width(220.0));
        ui.add_space(4.0);
        for (index, (icon, tip)) in [(icons::X_CIRCLE, "Show Errors"), (icons::WARNING, "Show Warnings"), (icons::INFO, "Show Infos")].into_iter().enumerate() {
            if ui::icon_button(ui, theme, icon, 22.0, !filter.hide[index], tip).clicked() {
                filter.hide[index] ^= true;
            }
        }
        if ui::icon_button(ui, theme, icons::FILE, 22.0, filter.active_only, "Show Active File Only").clicked() {
            filter.active_only ^= true;
        }
    });
    let from_tasks = task_files.iter().map(|(path, items)| (path, items));
    let all: Vec<_> = lsp.problems().map(|file| (&file.path, &file.items)).chain(from_tasks).filter(|(_, items)| items.iter().any(|d| d.severity <= 3)).collect();
    if all.is_empty() {
        return empty(ui, theme, "No problems have been detected in the workspace.");
    }
    let active = active.map(crate::lsp::key);
    let files: Vec<_> = all
        .into_iter()
        .filter(|(path, _)| !filter.active_only || active.as_ref().is_some_and(|a| *a == crate::lsp::key(path)))
        .map(|(path, items)| (path, items.iter().filter(|d| filter.shows(path, d)).collect::<Vec<_>>()))
        .filter(|(_, items)| !items.is_empty())
        .collect();
    if files.is_empty() {
        return empty(ui, theme, "No results found with provided filter criteria.");
    }
    ScrollArea::vertical().id_salt("problems").auto_shrink(false).show(ui, |ui| {
        ui.add_space(4.0);
        for (path, items) in files {
            let shown = cwd.and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
            let name = shown.file_name().unwrap_or(shown.as_os_str()).to_string_lossy();
            let folder = shown.parent().map(|dir| dir.to_string_lossy()).unwrap_or_default();
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            let painter = ui.painter().with_clip_rect(r);
            let title = format!("{}  {name}", icons::FILE_CODE);
            let end = painter.text(r.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(12.5), theme.text);
            painter.text(pos2(end.right() + 10.0, r.center().y), Align2::LEFT_CENTER, folder, FontId::proportional(11.5), theme.text_dim);
            for problem in items {
                let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                ui::row_bg(ui, theme, r, row.id, row.hovered(), false);
                ui::tree_item(ui, &row, &format!("{}: {}, line {}", ["Error", "Warning", "Info"][(problem.severity as usize).clamp(1, 3) - 1], problem.message.lines().next().unwrap_or_default(), problem.start.0 + 1), false, 1, None);
                let (icon, color) = match problem.severity {
                    1 => (icons::X_CIRCLE, theme.error),
                    2 => (icons::WARNING, theme.warning),
                    _ => (icons::INFO, theme.accent),
                };
                let painter = ui.painter().with_clip_rect(r);
                painter.text(r.left_center() + vec2(30.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), color);
                let message = problem.message.lines().next().unwrap_or_default();
                let end = painter.text(r.left_center() + vec2(50.0, 0.0), Align2::LEFT_CENTER, message, FontId::proportional(12.5), theme.text);
                let place = format!("{}  [Ln {}, Col {}]", problem.source, problem.start.0 + 1, problem.start.1 + 1);
                painter.text(pos2(end.right() + 10.0, r.center().y), Align2::LEFT_CENTER, place, FontId::proportional(11.5), theme.text_dim);
                if row.clicked() {
                    cmds.push(Command::OpenPath(path.clone()));
                    cmds.push(Command::GoTo(problem.start.0 + 1, problem.start.1 + 1));
                }
                row.on_hover_text(&problem.message);
            }
        }
    });
}

fn test_output_ui(ui: &mut Ui, theme: &Theme, text: &str, font_size: f32) {
    if text.is_empty() {
        return empty(ui, theme, "The output of the last test run appears here.");
    }
    ScrollArea::both().id_salt("test_output").auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(text).font(FontId::monospace(font_size - 1.0)).color(theme.text));
        });
    });
}

fn empty(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(18.0);
        ui.label(RichText::new(text).color(theme.text_dim));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_problems_by_text_and_severity() {
        let problem = |severity, message: &str| Diagnostic { start: (0, 0), end: (0, 1), severity, message: message.into(), source: "clang".into(), raw: serde_json::Value::Null };
        let path = Path::new("/w/src/main.cpp");
        let (error, warning, hint) = (problem(1, "Unknown type"), problem(2, "unused x"), problem(4, "style"));
        let mut filter = ProblemFilter::default();
        assert!(filter.shows(path, &error) && filter.shows(path, &warning) && !filter.shows(path, &hint));
        filter.hide[1] = true;
        assert!(filter.shows(path, &error) && !filter.shows(path, &warning));
        filter.hide[1] = false;
        filter.text = " UNKNOWN ".into();
        assert!(filter.shows(path, &error) && !filter.shows(path, &warning));
        filter.text = "main.cpp".into();
        assert!(filter.shows(path, &warning));
        filter.text = "clang".into();
        assert!(filter.shows(path, &warning));
    }

    #[test]
    fn encodes_keys() {
        let none = Modifiers::NONE;
        assert_eq!(encode_key(Key::ArrowUp, none, false, false).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(Key::ArrowUp, none, true, false).unwrap(), b"\x1bOA");
        assert_eq!(encode_key(Key::ArrowLeft, Modifiers::CTRL, true, false).unwrap(), b"\x1b[1;5D");
        assert_eq!(encode_key(Key::Delete, Modifiers::SHIFT, false, false).unwrap(), b"\x1b[3;2~");
        assert_eq!(encode_key(Key::Delete, Modifiers::CTRL, false, false).unwrap(), b"\x1bd");
        let line: Vec<char> = "git  commit -m".chars().collect();
        assert_eq!((word_step(&line, 0, true, true), word_step(&line, 3, true, true), word_step(&line, 14, false, true)), (3, 11, 13));
        assert_eq!((word_step(&line, 11, false, true), word_step(&line, 5, true, false), word_step(&line, 0, false, false)), (5, 6, 0));
        assert_eq!(encode_key(Key::F1, none, false, false).unwrap(), b"\x1bOP");
        assert_eq!(encode_key(Key::F5, none, false, false).unwrap(), b"\x1b[15~");
        assert_eq!(encode_key(Key::D, Modifiers::CTRL, false, false).unwrap(), [4]);
        assert_eq!(encode_key(Key::B, Modifiers::ALT, false, false).unwrap(), b"\x1bb");
        assert_eq!(encode_key(Key::Tab, Modifiers::SHIFT, false, false).unwrap(), b"\x1b[Z");
        assert_eq!(encode_key(Key::Backspace, none, false, false).unwrap(), [0x7f]);
        assert!(encode_key(Key::A, none, false, false).is_none());
    }

    #[test]
    fn word_deletion_bytes_follow_the_shell() {
        let native_byte = if cfg!(windows) { 0x08 } else { 0x17 };
        for (program, expected) in [
            ("cmd", native_byte), ("PowerShell.EXE", native_byte), ("C:/Program Files/PowerShell/7/pwsh.exe", native_byte),
            ("wsl.exe", 0x17), ("bash", 0x17), ("zsh", 0x17), ("fish", 0x17), ("ssh.exe", 0x17),
        ] {
            let native = windows_word_erase(program);
            assert_eq!(encode_key(Key::Backspace, Modifiers::CTRL, false, native).unwrap(), [expected], "{program}");
            assert_eq!(encode_key(Key::Delete, Modifiers::CTRL, false, native).unwrap(), b"\x1bd", "{program}");
        }
    }

    #[test]
    fn maps_colours_and_netstat() {
        let theme = Theme::dark();
        assert_eq!(xterm_color(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(xterm_color(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(xterm_color(244), Color32::from_rgb(128, 128, 128));
        assert_eq!(resolve(AnsiColor::Indexed(2), &theme), Some(theme.ansi[2]));
        assert_eq!(resolve(AnsiColor::Named(NamedColor::Background), &theme), None);
        assert_eq!(resolve(AnsiColor::Named(NamedColor::Foreground), &theme), Some(theme.text));
        let rows = parse_netstat("  TCP    0.0.0.0:135   0.0.0.0:0   LISTENING   1234\n  TCP    [::]:8080   [::]:0   LISTENING   77\n  TCP  1.2.3.4:5  6.7.8.9:1  ESTABLISHED  9\n");
        assert_eq!(rows, vec![("0.0.0.0".into(), 135, "1234".into()), ("[::]".into(), 8080, "77".into())]);
    }

    #[test]
    fn runs_a_real_shell() {
        let ctx = egui::Context::default();
        let (program, args): (&str, Vec<String>) = if cfg!(windows) { ("cmd.exe", vec![]) } else { ("sh", vec![]) };
        let terminal = Terminal::spawn(1, "test", program, &args, None, ctx).unwrap();
        terminal.write(b"echo halo-abc123\r".to_vec());
        let started = std::time::Instant::now();
        let mut seen = false;
        while started.elapsed() < std::time::Duration::from_secs(15) && !seen {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let term = terminal.term.lock();
            let text: String = term.renderable_content().display_iter.map(|c| c.c).collect();
            seen = text.matches("halo-abc123").count() >= 2;
        }
        assert!(seen);
        assert!(terminal.is_alive());
        assert!(Terminal::spawn(2, "bad", "halo-no-such-program-xyz", &[], None, egui::Context::default()).is_err());
    }

    #[test]
    fn pastes_selects_with_shift_arrows_and_copies() {
        let ctx = egui::Context::default();
        let (program, args): (&str, Vec<String>) = if cfg!(windows) { ("cmd.exe", vec![]) } else { ("sh", vec![]) };
        let mut terminal = Terminal::spawn(1, "test", program, &args, None, ctx.clone()).unwrap();
        let theme = Theme::dark();
        let frame = |terminal: &mut Terminal, events: Vec<Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 400.0))), events, focused: true, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| terminal.ui(ui, &theme, 14.0, &mut vec![]));
            out.textures_delta.clear();
            out.platform_output
        };
        frame(&mut terminal, vec![]);
        frame(&mut terminal, vec![]);
        let started = std::time::Instant::now();
        let typed = |t: &Terminal| t.text().contains("halo-paste-word");
        frame(&mut terminal, vec![Event::Paste("echo halo-paste-word".into())]);
        while !typed(&terminal) && started.elapsed().as_secs() < 15 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(typed(&terminal), "{}", terminal.text());
        std::thread::sleep(std::time::Duration::from_millis(200));
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let word_left = Modifiers { ctrl: true, shift: true, command: true, ..Default::default() };
        frame(&mut terminal, vec![key(Key::ArrowLeft, word_left)]);
        assert_eq!(terminal.selection().as_deref(), Some("word"));
        frame(&mut terminal, vec![key(Key::ArrowLeft, word_left), key(Key::ArrowRight, Modifiers::SHIFT)]);
        assert_eq!(terminal.selection().as_deref(), Some("aste-word"));
        let out = frame(&mut terminal, vec![Event::Copy]);
        assert!(out.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "aste-word")));
        assert_eq!(terminal.selection(), None);
        for event in [key(Key::ArrowRight, Modifiers::NONE), key(Key::ArrowRight, Modifiers::CTRL), Event::Text("x".into())] {
            frame(&mut terminal, vec![key(Key::ArrowLeft, word_left)]);
            assert!(terminal.selection().is_some());
            frame(&mut terminal, vec![event]);
            assert!(terminal.term.lock().selection.is_none());
        }
    }

    fn integrated(program: &str, base: &[&str], lines: &[&str]) -> (Terminal, Vec<Record>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("halo-shell-{}-{}", std::process::id(), base.len()));
        assert!(crate::shell_integration::write_scripts(&dir));
        let base: Vec<String> = base.iter().map(|s| s.to_string()).collect();
        let (args, env) = crate::shell_integration::inject(program, &base, &dir).unwrap();
        let terminal = Terminal::spawn_with(1, "test", program, &args, Some(&dir.join("zsh")), &env, egui::Context::default()).unwrap();
        let finished = |t: &Terminal| t.integration.lock().unwrap().commands.iter().filter(|r| r.end.is_some()).count();
        let started = std::time::Instant::now();
        for (index, line) in lines.iter().enumerate() {
            while !(finished(&terminal) == index && terminal.integration.lock().unwrap().commands.len() > index) && started.elapsed().as_secs() < 30 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            terminal.write(format!("{line}\r"));
        }
        while finished(&terminal) < lines.len() && started.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let records = terminal.integration.lock().unwrap().commands.iter().filter(|r| r.end.is_some()).cloned().collect();
        (terminal, records, dir)
    }

    fn check_shell(program: &str, base: &[&str], fail: &str, code: i32) {
        let (terminal, done, dir) = integrated(program, base, &["cd ..", fail, "echo halo-out"]);
        let shown = |t: &Terminal| t.text();
        assert_eq!(done.iter().map(|r| (r.line.as_str(), r.exit)).collect::<Vec<_>>(), vec![("cd ..", Some(0)), (fail, Some(code)), ("echo halo-out", Some(0))], "{}", shown(&terminal));
        assert_eq!(terminal.cwd().and_then(|c| c.canonicalize().ok()), dir.canonicalize().ok());
        assert!(done[1].failed() && !terminal.failed());
        assert_eq!(terminal.recent()[0], "echo halo-out");
        assert_eq!(terminal.last_output().as_deref(), Some("halo-out"));
        drop(terminal);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn real_shells_report_commands_exit_codes_and_folders() {
        if cfg!(windows) {
            check_shell("powershell.exe", &["-NoLogo", "-NoProfile"], "cmd /c exit 3", 3);
        } else {
            check_shell("bash", &[], "(exit 3)", 3);
        }
        let git_bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if cfg!(windows) && git_bash.is_file() {
            check_shell(&git_bash.to_string_lossy(), &["--login", "-i"], "false", 1);
        }
    }

    #[test]
    fn renames_and_colours_the_active_terminal() {
        let ctx = egui::Context::default();
        let shell = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let settings = Settings { terminals: vec![ShellProfile { name: "test".into(), program: shell.into(), args: vec![] }], default_terminal: "test".into(), ..Settings::default() };
        let mut panel = BottomPanel::new();
        panel.exec(&Command::NewTerminal, &settings, None, &ctx);
        assert!(panel.exec(&Command::RenameTerminal, &settings, None, &ctx));
        assert_eq!(panel.renaming.as_ref().map(|r| (r.1.as_str(), r.2)), Some(("test", true)));
        panel.exec(&Command::SetTerminalColor(Some(1)), &settings, None, &ctx);
        let theme = Theme::dark();
        let pane = panel.active_pane().unwrap();
        assert_eq!((pane.color, pane.tint(&theme)), (Some(1), theme.ansi[1]));
        pane.label = Some("build".into());
        assert_eq!(pane.title(), "build");
        pane.label = None;
        assert_ne!(pane.title(), "build");
    }

    fn wait_for(what: impl Fn() -> bool) -> bool {
        let started = std::time::Instant::now();
        while !what() && started.elapsed() < std::time::Duration::from_secs(20) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        what()
    }

    #[test]
    fn saves_terminals_and_restores_them_in_place() {
        let ctx = egui::Context::default();
        let shell = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let settings = Settings { terminals: vec![ShellProfile { name: "test".into(), program: shell.into(), args: vec![] }], default_terminal: "test".into(), ..Settings::default() };
        let base = std::env::temp_dir().join(format!("halo-persist-restore-{}", std::process::id()));
        let folder = std::env::temp_dir();
        let (mut panel, mut editors) = (BottomPanel::new(), crate::editor::Editors::new());
        panel.exec(&Command::NewTerminal, &settings, Some(&folder), &ctx);
        panel.exec(&Command::SplitTerminal, &settings, Some(&folder), &ctx);
        panel.exec(&Command::NewTerminal, &settings, Some(&folder), &ctx);
        for (g, p, line) in [(0, 0, "echo halo-one"), (0, 1, "echo halo-two"), (1, 0, "echo halo-three")] {
            panel.groups[g].panes[p].write(format!("{line}\r"));
        }
        let word = |p: &BottomPanel, g: usize, i: usize, w: &str| p.groups[g].panes[i].text().matches(w).count() >= 2;
        assert!(wait_for(|| word(&panel, 0, 0, "halo-one") && word(&panel, 0, 1, "halo-two") && word(&panel, 1, 0, "halo-three")));
        editors.add_terminal(panel.take_active().unwrap());
        let first = &mut panel.groups[0].panes[0];
        (first.label, first.color) = (Some("build".into()), Some(2));
        first.integration.lock().unwrap().restored = vec!["make".into()];
        panel.groups[0].active = 0;
        crate::terminal_restore::save_in(&base, &panel, &editors, &settings, Some(&folder));
        drop((panel, editors));

        let (mut panel, mut editors) = (BottomPanel::new(), crate::editor::Editors::new());
        crate::terminal_restore::restore_in(&base, &mut panel, &mut editors, &settings, Some(&folder), &ctx);
        assert_eq!((panel.groups.len(), panel.groups[0].panes.len(), panel.groups[0].active), (1, 2, 0));
        let first = &panel.groups[0].panes[0];
        assert_eq!((first.label.as_deref(), first.color, first.title()), (Some("build"), Some(2), "build".to_owned()));
        assert!(first.text().contains("halo-one") && first.text().contains(crate::terminal_restore::MARKER));
        assert!(panel.groups[0].panes[1].text().contains("halo-two"));
        let tabs = editors.terminal_tabs();
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].3.text().contains("halo-three") && !tabs[0].3.text().contains("halo-one"));
        assert!(wait_for(|| panel.groups[0].panes[0].is_alive() && panel.groups[0].panes[0].text().lines().count() > 3));
        assert_eq!(panel.recent(), vec!["make".to_owned()]);
        assert!(panel.exec(&Command::ClearRestoredHistory, &settings, None, &ctx));
        let text = panel.groups[0].panes[0].text();
        assert!(!text.contains("halo-one") && !text.contains(crate::terminal_restore::MARKER), "{text}");
        assert!(panel.recent().is_empty());
        drop((panel, editors));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn runs_lines_in_one_reused_terminal_and_mirrors_them() {
        let ctx = egui::Context::default();
        let shell = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let profile = ShellProfile { name: "test".into(), program: shell.into(), args: vec![] };
        let settings = Settings { terminals: vec![profile], default_terminal: "test".into(), ..Settings::default() };
        let mut panel = BottomPanel::new();
        panel.run("echo halo-run-1", &settings, None, &ctx);
        panel.run("echo halo-run-2", &settings, None, &ctx);
        assert_eq!(panel.groups.len(), 1);
        assert_eq!(panel.run_line, "echo halo-run-2");
        let mirrored = |p: &BottomPanel| p.groups[0].panes[0].text().matches("halo-run-2").count() >= 2;
        let started = std::time::Instant::now();
        while !mirrored(&panel) && started.elapsed() < std::time::Duration::from_secs(15) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(mirrored(&panel));
        let wsl = default_profile(&settings, Some(Path::new(r"\\wsl.localhost\Ubuntu\home\joe"))).unwrap();
        assert_eq!((wsl.program.as_str(), wsl.args.join(" ")), ("wsl.exe", "-d Ubuntu --cd /home/joe".to_owned()));
    }
}
