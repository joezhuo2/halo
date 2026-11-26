use crate::app::Halo;
use crate::cli::Args;
use crate::command::Command;
use eframe::egui::{self, ViewportCommand};
use std::hash::BuildHasher;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub struct Server {
    listener: TcpListener,
    token: String,
    _lock: std::fs::File,
}

pub struct Request {
    pub args: Args,
    pub reply: Option<TcpStream>,
}

pub type Wait = (Vec<PathBuf>, TcpStream);

fn file(dir: &Path) -> PathBuf {
    dir.join("instance")
}

fn connect(dir: &Path) -> io::Result<(TcpStream, String)> {
    #[cfg(windows)]
    if std::fs::OpenOptions::new().write(true).open(file(dir)).is_ok() {
        return Err(io::Error::new(io::ErrorKind::NotConnected, "no running instance holds the instance file"));
    }
    let text = std::fs::read_to_string(file(dir))?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "the instance file is damaged");
    let (port, token) = text.trim().split_once(' ').ok_or_else(bad)?;
    let port: u16 = port.parse().map_err(|_| bad())?;
    let stream = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(1))?;
    Ok((stream, token.to_owned()))
}

pub fn send(dir: &Path, args: &Args) -> io::Result<()> {
    let (mut stream, token) = connect(dir)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    writeln!(stream, "{token}\n{}", serde_json::to_string(args)?)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim_end() != "ok" {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the running instance refused the request"));
    }
    if args.wait {
        reader.get_ref().set_read_timeout(None)?;
        reader.read_line(&mut line)?;
    }
    Ok(())
}

pub fn listen(dir: &Path) -> io::Result<Option<Server>> {
    #[cfg(not(windows))]
    if connect(dir).is_ok() {
        return Ok(None);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let random = || std::collections::hash_map::RandomState::new().hash_one(std::time::SystemTime::now());
    let token = format!("{:016x}{:016x}", random(), random());
    std::fs::create_dir_all(dir)?;
    let path = file(dir);
    let mut options = std::fs::OpenOptions::new();
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::share_mode(options.write(true).create(true).truncate(true), 1);
    #[cfg(not(windows))]
    {
        let _ = std::fs::remove_file(&path);
        options.write(true).create_new(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut lock = match options.open(&path) {
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => return Ok(None),
        result => result?,
    };
    write!(lock, "{} {token}", listener.local_addr()?.port())?;
    Ok(Some(Server { listener, token, _lock: lock }))
}

pub fn serve(server: Server, wake: impl Fn() + Send + 'static) -> Receiver<Request> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let server = server;
        for stream in server.listener.incoming().flatten() {
            if let Some(request) = accept(stream, &server.token) {
                if tx.send(request).is_err() {
                    break;
                }
                wake();
            }
        }
    });
    rx
}

fn accept(mut stream: TcpStream, token: &str) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    (&mut reader).take(256).read_line(&mut line).ok()?;
    if line.trim_end() != token {
        let _ = reader.take(1 << 20).read_line(&mut line);
        let _ = stream.write_all(b"refused\n");
        return None;
    }
    line.clear();
    reader.read_line(&mut line).ok()?;
    let args: Args = serde_json::from_str(&line).ok()?;
    stream.write_all(b"ok\n").ok()?;
    Some(Request { reply: args.wait.then_some(stream), args })
}

pub fn release(waits: &mut Vec<Wait>, open: &[PathBuf]) {
    waits.retain_mut(|(paths, stream)| {
        let still_open = paths.iter().any(|p| open.contains(p));
        if !still_open {
            let _ = stream.write_all(b"done\n");
        }
        still_open
    });
}

impl Halo {
    pub(crate) fn poll_ipc(&mut self, ctx: &egui::Context) {
        while let Some(request) = self.inbox.as_ref().and_then(|rx| rx.try_recv().ok()) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            self.open_args(request.args, request.reply, ctx);
        }
        if !self.waits.is_empty() {
            release(&mut self.waits, &self.editors.open_paths());
        }
    }

    pub(crate) fn open_args(&mut self, args: Args, reply: Option<TcpStream>, ctx: &egui::Context) {
        let mut files = vec![];
        for target in args.paths {
            if target.path.is_dir() {
                self.cmds.push(Command::OpenPath(target.path));
                continue;
            }
            self.editors.open_kept(&target.path);
            if crate::workspace::is_workspace_file(&target.path) {
                crate::app::offer_workspace(&target.path);
            }
            if let Some(line) = target.line {
                self.editors.exec(&Command::GoTo(line, target.column.unwrap_or(1)), &self.settings, ctx);
            }
            files.push(target.path);
        }
        self.cmds.extend(args.add.into_iter().map(|p| if p.is_dir() { Command::AddWorkspaceFolder(p) } else { Command::OpenPath(p) }));
        if let Some((left, right)) = args.diff {
            let name = |p: &Path| p.file_name().unwrap_or(p.as_os_str()).to_string_lossy().into_owned();
            self.cmds.push(Command::OpenDiff(format!("{} ↔ {}", name(&left), name(&right)), crate::history::diff_args(&left, &right)));
        }
        if let Some(text) = args.stdin {
            self.editors.open_untitled(&text);
        }
        if let Some(stream) = reply {
            self.waits.push((files, stream));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Target;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-ipc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn file_args(path: &str, wait: bool) -> Args {
        Args { paths: vec![Target { path: path.into(), line: Some(3), column: None }], wait, ..Default::default() }
    }

    #[test]
    fn hands_a_request_to_the_running_instance() {
        let dir = temp("send");
        assert!(send(&dir, &Args::default()).is_err());
        let rx = serve(listen(&dir).unwrap().unwrap(), || {});
        assert!(listen(&dir).unwrap().is_none());
        let args = Args { stdin: Some("piped\ntext".into()), ..file_args("a.rs", false) };
        send(&dir, &args).unwrap();
        let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.args, args);
        assert!(request.reply.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_a_request_without_the_token() {
        let dir = temp("token");
        let rx = serve(listen(&dir).unwrap().unwrap(), || {});
        let text = std::fs::read_to_string(file(&dir)).unwrap();
        let port: u16 = text.split(' ').next().unwrap().parse().unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        writeln!(stream, "not-the-token\n{}", serde_json::to_string(&file_args("a.rs", false)).unwrap()).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert_eq!(line, "refused\n");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replaces_a_stale_file() {
        let dir = temp("stale");
        std::fs::create_dir_all(&dir).unwrap();
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        std::fs::write(file(&dir), format!("{port} old")).unwrap();
        let start = std::time::Instant::now();
        assert!(send(&dir, &Args::default()).is_err());
        let server = listen(&dir).expect("listen").expect("stale file replaced");
        assert!(start.elapsed() < Duration::from_millis(200), "took {:?}", start.elapsed());
        let rx = serve(server, || {});
        assert!(!std::fs::read_to_string(file(&dir)).unwrap().ends_with(" old"));
        assert!(listen(&dir).unwrap().is_none());
        send(&dir, &Args::default()).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
        #[cfg(windows)]
        assert!(std::fs::remove_file(file(&dir)).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wait_is_released_when_the_tab_closes() {
        let dir = temp("wait");
        let rx = serve(listen(&dir).unwrap().unwrap(), || {});
        let client = {
            let dir = dir.clone();
            std::thread::spawn(move || send(&dir, &file_args("/w/a.rs", true)))
        };
        let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut waits = vec![(request.args.paths.iter().map(|t| t.path.clone()).collect(), request.reply.unwrap())];
        release(&mut waits, &[PathBuf::from("/w/a.rs"), PathBuf::from("/w/b.rs")]);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(waits.len(), 1);
        assert!(!client.is_finished());
        release(&mut waits, &[PathBuf::from("/w/b.rs")]);
        assert!(waits.is_empty());
        client.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
