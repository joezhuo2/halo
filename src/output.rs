use crate::command::Command;
use crate::search::Choice;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, ScrollArea, Sense, Ui, vec2};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

const CAP: usize = 5000;
const STANDARD: [&str; 5] = ["Git", "Language Servers", "Debug", "Tasks", "Halo"];

#[derive(Default)]
struct Channel {
    name: String,
    lines: VecDeque<(u64, String)>,
}

impl Channel {
    fn push(&mut self, time: u64, text: &str) {
        for line in text.lines() {
            if self.lines.len() == CAP {
                self.lines.pop_front();
            }
            self.lines.push_back((time, line.to_owned()));
        }
    }

    fn matching(&self, filter: &str) -> Vec<usize> {
        let needle = filter.to_lowercase();
        (0..self.lines.len()).filter(|&i| needle.is_empty() || self.lines[i].1.to_lowercase().contains(&needle)).collect()
    }

    fn text(&self, filter: &str) -> String {
        self.matching(filter).into_iter().map(|i| self.lines[i].1.as_str()).collect::<Vec<_>>().join("\n")
    }
}

#[derive(Clone)]
struct View {
    channel: String,
    lock: bool,
    filter: String,
}

static CHANNELS: Mutex<Vec<Channel>> = Mutex::new(Vec::new());
static VIEW: Mutex<View> = Mutex::new(View { channel: String::new(), lock: false, filter: String::new() });
static REPAINT: Mutex<Option<egui::Context>> = Mutex::new(None);
static VISIBLE: AtomicBool = AtomicBool::new(false);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn clock(secs: u64) -> String {
    let day = secs % 86400;
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}

pub fn begin_frame() {
    VISIBLE.store(false, Ordering::Relaxed);
}

pub fn log(channel: &str, text: impl AsRef<str>) {
    let time = now();
    {
        let mut channels = lock(&CHANNELS);
        let index = match channels.iter().position(|c| c.name == channel) {
            Some(index) => index,
            None => {
                channels.push(Channel { name: channel.to_owned(), ..Default::default() });
                channels.len() - 1
            }
        };
        channels[index].push(time, text.as_ref());
    }
    if VISIBLE.load(Ordering::Relaxed)
        && let Some(ctx) = &*lock(&REPAINT)
    {
        ctx.request_repaint();
    }
}

fn names() -> Vec<String> {
    let mut names: Vec<String> = STANDARD.iter().map(|n| (*n).to_owned()).collect();
    names.extend(lock(&CHANNELS).iter().map(|c| c.name.clone()).filter(|n| !STANDARD.contains(&n.as_str())));
    names
}

fn current() -> View {
    let mut view = lock(&VIEW).clone();
    if view.channel.is_empty() {
        view.channel = "Halo".into();
    }
    view
}

pub fn select(name: String) {
    lock(&VIEW).channel = name;
}

pub fn picks() -> Vec<Choice> {
    let view = current();
    let channels = lock(&CHANNELS);
    names()
        .into_iter()
        .map(|name| {
            let count = channels.iter().find(|c| c.name == name).map_or(0, |c| c.lines.len());
            let detail = if name == view.channel { format!("current, {count} lines") } else { format!("{count} lines") };
            (name.clone(), detail, Command::SelectOutputChannel(name))
        })
        .collect()
}

pub fn clear_current() {
    let name = current().channel;
    if let Some(channel) = lock(&CHANNELS).iter_mut().find(|c| c.name == name) {
        channel.lines.clear();
    }
}

pub fn current_text() -> String {
    let view = current();
    lock(&CHANNELS).iter().find(|c| c.name == view.channel).map(|c| c.text(&view.filter)).unwrap_or_default()
}

pub fn ui(ui: &mut Ui, theme: &Theme, font_size: f32, cmds: &mut Vec<Command>) {
    VISIBLE.store(true, Ordering::Relaxed);
    *lock(&REPAINT) = Some(ui.ctx().clone());
    let mut view = current();
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::ComboBox::from_id_salt("output_channel").selected_text(view.channel.clone()).show_ui(ui, |ui| {
            for name in names() {
                ui.selectable_value(&mut view.channel, name.clone(), name);
            }
        });
        ui.add(egui::TextEdit::singleline(&mut view.filter).hint_text("Filter").desired_width(180.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if ui::icon_button(ui, theme, icons::ARROW_SQUARE_OUT, 22.0, false, "Open in Editor").clicked() {
                cmds.push(Command::OpenOutputInEditor);
            }
            if ui::icon_button(ui, theme, icons::COPY, 22.0, false, "Copy All").clicked() {
                let text = current_text();
                ui.ctx().copy_text(text);
            }
            if ui::icon_button(ui, theme, icons::TRASH, 22.0, false, "Clear Output").clicked() {
                cmds.push(Command::ClearOutput);
            }
            let tip = if view.lock { "Turn Auto Scrolling On" } else { "Turn Auto Scrolling Off" };
            if ui::icon_button(ui, theme, if view.lock { icons::LOCK } else { icons::LOCK_OPEN }, 22.0, view.lock, tip).clicked() {
                view.lock ^= true;
            }
        });
    });
    let rows = lock(&CHANNELS).iter().find(|c| c.name == view.channel).map(|c| c.matching(&view.filter)).unwrap_or_default();
    let name = view.channel.clone();
    *lock(&VIEW) = view.clone();
    if rows.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(format!("No output in {name}.")).color(theme.text_dim));
        });
        return;
    }
    let font = FontId::monospace(font_size - 1.0);
    let height = font_size + 4.0;
    ScrollArea::vertical().id_salt(("output", &name)).auto_shrink(false).stick_to_bottom(!view.lock).show_rows(ui, height, rows.len(), |ui, range| {
        let lines: Vec<(u64, String)> = {
            let channels = lock(&CHANNELS);
            let Some(channel) = channels.iter().find(|c| c.name == name) else { return };
            range.clone().filter_map(|row| rows.get(row).and_then(|&i| channel.lines.get(i)).cloned()).collect()
        };
        for (time, text) in lines {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
            let painter = ui.painter().with_clip_rect(rect);
            let stamp = painter.text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, clock(time), font.clone(), theme.text_dim);
            let color = match () {
                _ if text.starts_with("[error]") => theme.error,
                _ if text.starts_with("[warning]") => theme.warning,
                _ => theme.text,
            };
            painter.text(egui::pos2(stamp.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), color);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_keeps_the_newest_lines() {
        let mut channel = Channel::default();
        for i in 0..CAP + 10 {
            channel.push(0, &format!("line {i}"));
        }
        assert_eq!(channel.lines.len(), CAP);
        assert_eq!(channel.lines.front().unwrap().1, "line 10");
        assert_eq!(channel.lines.back().unwrap().1, format!("line {}", CAP + 9));
        channel.push(0, "a\nb\n");
        assert_eq!(channel.lines.len(), CAP);
        assert_eq!(channel.lines.back().unwrap().1, "b");
    }

    #[test]
    fn filters_case_insensitively_and_copies_what_is_shown() {
        let mut channel = Channel::default();
        channel.push(0, "Error: boom\nok\nanother ERROR here");
        assert_eq!(channel.matching("error"), [0, 2]);
        assert_eq!(channel.matching(""), [0, 1, 2]);
        assert!(channel.matching("zzz").is_empty());
        assert_eq!(channel.text("boom"), "Error: boom");
    }

    #[test]
    fn channels_are_created_once_from_many_threads() {
        let threads: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    for i in 0..800 {
                        log("test-shared", format!("{t}:{i}"));
                        log(&format!("test-own-{t}"), format!("{i}"));
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let channels = lock(&CHANNELS);
        assert_eq!(channels.iter().filter(|c| c.name == "test-shared").count(), 1);
        assert_eq!(channels.iter().find(|c| c.name == "test-shared").unwrap().lines.len(), CAP);
        for t in 0..8 {
            let own = channels.iter().find(|c| c.name == format!("test-own-{t}")).unwrap();
            assert_eq!(own.lines.len(), 800);
            assert_eq!(own.lines.back().unwrap().1, "799");
        }
    }

    #[test]
    fn formats_the_clock() {
        assert_eq!(clock(3 * 3600 + 4 * 60 + 5), "03:04:05");
        assert_eq!(clock(86400 + 59), "00:00:59");
    }
}
