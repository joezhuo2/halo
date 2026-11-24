use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Raw {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub cpu: f64,
    pub mem: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub pid: u32,
    pub depth: usize,
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

pub fn tree(all: &[Raw], root: u32) -> Vec<(usize, &Raw)> {
    let mut kids: HashMap<u32, Vec<&Raw>> = HashMap::new();
    for p in all.iter().filter(|p| p.pid != p.ppid) {
        kids.entry(p.ppid).or_default().push(p);
    }
    let mut out = vec![];
    let mut stack: Vec<(usize, &Raw)> = all.iter().filter(|p| p.pid == root).map(|p| (0, p)).collect();
    while let Some((depth, p)) = stack.pop() {
        if out.len() > 4096 {
            break;
        }
        out.push((depth, p));
        let mut children = kids.get(&p.pid).cloned().unwrap_or_default();
        children.sort_by_key(|c| std::cmp::Reverse(c.pid));
        stack.extend(children.into_iter().map(|c| (depth + 1, c)));
    }
    out
}

#[cfg(windows)]
fn sample(root: u32) -> Vec<Raw> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut all = vec![];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return all };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            all.push(Raw { pid: entry.th32ProcessID, ppid: entry.th32ParentProcessID, name: String::from_utf16_lossy(&entry.szExeFile[..len]), cpu: 0.0, mem: 0 });
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    let mine: Vec<u32> = tree(&all, root).into_iter().map(|(_, p)| p.pid).collect();
    let seconds = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7;
    for p in all.iter_mut().filter(|p| mine.contains(&p.pid)) {
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, p.pid) else { continue };
            let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = Default::default();
            if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok() {
                p.cpu = seconds(kernel) + seconds(user);
            }
            let mut counters = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            if GetProcessMemoryInfo(handle, &mut counters, counters.cb).is_ok() {
                p.mem = counters.WorkingSetSize as u64;
            }
            let _ = CloseHandle(handle);
        }
    }
    all
}

#[cfg(target_os = "linux")]
pub fn parse_stat(text: &str) -> Option<(u32, String, u32, f64)> {
    let (head, rest) = text.split_once(" (")?;
    let (name, rest) = rest.rsplit_once(") ")?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some((head.trim().parse().ok()?, name.to_owned(), fields.get(1)?.parse().ok()?, ticks / 100.0))
}

#[cfg(target_os = "linux")]
fn sample(_root: u32) -> Vec<Raw> {
    let entries = std::fs::read_dir("/proc").into_iter().flatten().flatten();
    let stat = |pid: &str| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|t| parse_stat(&t));
    let rss = |pid: u32| {
        let text = std::fs::read_to_string(format!("/proc/{pid}/statm")).unwrap_or_default();
        text.split_whitespace().nth(1).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0) * 4096
    };
    entries
        .filter_map(|e| stat(e.file_name().to_str()?))
        .map(|(pid, name, ppid, cpu)| Raw { pid, ppid, name, cpu, mem: rss(pid) })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn parse_ps(text: &str) -> Vec<Raw> {
    let time = |t: &str| t.split([':', '-']).rev().zip([1.0, 60.0, 3600.0, 86400.0]).map(|(part, unit)| part.parse::<f64>().unwrap_or(0.0) * unit).sum();
    let row = |line: &str| {
        let mut parts = line.split_whitespace();
        let (pid, ppid, rss, cpu) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse::<u64>().ok()?, time(parts.next()?));
        let name = parts.collect::<Vec<_>>().join(" ");
        Some(Raw { pid, ppid, name: name.rsplit('/').next().unwrap_or(&name).to_owned(), cpu, mem: rss * 1024 })
    };
    text.lines().filter_map(row).collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn sample(_root: u32) -> Vec<Raw> {
    let out = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,rss=,time=,comm="]).output();
    parse_ps(&out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default())
}

fn rows(all: &[Raw], root: u32, before: &HashMap<u32, f64>, elapsed: f64, cores: f64) -> Vec<Row> {
    let usage = |p: &Raw| before.get(&p.pid).map_or(0.0, |was| ((p.cpu - was).max(0.0) / elapsed / cores * 100.0) as f32);
    tree(all, root).into_iter().map(|(depth, p)| Row { pid: p.pid, depth, name: p.name.clone(), cpu: usage(p), mem: p.mem }).collect()
}

fn kill(pid: u32) -> std::io::Result<std::process::Child> {
    if cfg!(windows) {
        crate::terminal::hidden_command("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).spawn()
    } else {
        std::process::Command::new("kill").arg(pid.to_string()).spawn()
    }
}

pub struct ProcessView {
    rx: Receiver<Vec<Row>>,
    rows: Vec<Row>,
}

impl ProcessView {
    pub fn open(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let root = std::process::id();
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
            let (mut before, mut at) = (HashMap::new(), Instant::now());
            loop {
                let all = sample(root);
                let elapsed = at.elapsed().as_secs_f64().max(0.001);
                let list = rows(&all, root, &before, elapsed, cores);
                (before, at) = (all.iter().map(|p| (p.pid, p.cpu)).collect(), Instant::now());
                if tx.send(list).is_err() {
                    break;
                }
                ctx.request_repaint();
                std::thread::sleep(Duration::from_millis(1500));
            }
        });
        Self { rx, rows: vec![] }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, font_size: f32) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if let Some(rows) = self.rx.try_iter().last() {
            self.rows = rows;
        }
        if self.rows.is_empty() {
            ui::spinner(ui, rect.center(), 10.0, theme.text_dim);
            return;
        }
        let font = FontId::proportional(font_size.min(13.0));
        let row_h = 24.0;
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), row_h + 4.0));
        let columns = [("CPU %", 90.0), ("Memory", 110.0), ("PID", 80.0)];
        let right = |index: usize| header.right() - 40.0 - columns[index + 1..].iter().map(|c| c.1).sum::<f32>();
        ui.painter().rect_filled(header, CornerRadius::ZERO, theme.panel);
        ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, theme.border));
        ui.painter().text(header.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, "Name", font.clone(), theme.text_dim);
        for (index, (label, _)) in columns.iter().enumerate() {
            ui.painter().text(pos2(right(index) - 8.0, header.center().y), Align2::RIGHT_CENTER, *label, font.clone(), theme.text_dim);
        }
        let body = rect.with_min_y(header.bottom());
        let mut killed = None;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ScrollArea::vertical().id_salt("process_view").auto_shrink(false).show_rows(ui, row_h, self.rows.len(), |ui, range| {
                for (index, row) in self.rows[range.clone()].iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                    ui::row_bg(ui, theme, r, resp.id, resp.hovered(), false);
                    let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                    let name = if range.start + index == 0 { format!("{} (Halo)", row.name) } else { row.name.clone() };
                    painter.text(pos2(r.left() + 12.0 + row.depth as f32 * 16.0, r.center().y), Align2::LEFT_CENTER, name, font.clone(), theme.text);
                    let cells = [format!("{:.1}", row.cpu), super::image_view::size_label(row.mem), row.pid.to_string()];
                    for (k, cell) in cells.into_iter().enumerate() {
                        painter.text(pos2(right(k) - 8.0, r.center().y), Align2::RIGHT_CENTER, cell, font.clone(), theme.text_dim);
                    }
                    if row.depth == 0 {
                        continue;
                    }
                    let button = Rect::from_center_size(pos2(r.right() - 20.0, r.center().y), vec2(22.0, 22.0));
                    if ui.scope_builder(UiBuilder::new().max_rect(button), |ui| ui::icon_button(ui, theme, icons::X, 20.0, false, "Kill Process").clicked()).inner {
                        killed = Some((row.pid, row.name.clone()));
                    }
                }
            });
        });
        if let Some((pid, name)) = killed {
            match kill(pid) {
                Ok(_) => ui::notify_info(format!("Stopped {name} ({pid}) and its child processes.")),
                Err(e) => ui::notify_error(format!("Could not stop {name} ({pid}): {e}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pid: u32, ppid: u32, cpu: f64) -> Raw {
        Raw { pid, ppid, name: format!("p{pid}"), cpu, mem: 0 }
    }

    #[test]
    fn lists_only_descendants_in_tree_order_with_cpu_share() {
        let all = [raw(1, 0, 0.0), raw(10, 1, 2.0), raw(11, 10, 1.0), raw(12, 1, 0.0), raw(20, 2, 9.0)];
        let before = HashMap::from([(10, 1.0), (11, 1.0)]);
        let list = rows(&all, 1, &before, 2.0, 2.0);
        let shape: Vec<(u32, usize)> = list.iter().map(|r| (r.pid, r.depth)).collect();
        assert_eq!(shape, [(1, 0), (10, 1), (11, 2), (12, 1)]);
        assert_eq!(list[1].cpu, 25.0);
        assert_eq!(list[2].cpu, 0.0);
        assert_eq!(tree(&[raw(5, 5, 0.0)], 5).len(), 1);
    }

    #[test]
    fn measures_this_process() {
        let all = sample(std::process::id());
        let me = all.iter().find(|p| p.pid == std::process::id()).expect("own process listed");
        assert!(me.mem > 0 && !me.name.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_proc_stat_with_spaces_in_the_name() {
        let line = "42 (my (odd) name) S 7 42 42 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 1 0 100 0 0";
        assert_eq!(parse_stat(line), Some((42, "my (odd) name".into(), 7, 2.0)));
    }
}
