use crate::app::note;
use crate::command::Command;
use crate::history;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, FontId, Rect, RichText, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

const FORMAT: &str = "--format=%x1e%H%x1f%an%x1f%ar%x1f%aI%x1f%at%x1f%s";

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub hash: String,
    pub author: String,
    pub ago: String,
    pub date: String,
    pub time: u64,
    pub subject: String,
    pub name: String,
}

#[derive(Debug, PartialEq)]
pub enum Item {
    Commit(Entry),
    Saved(u64, PathBuf),
}

impl Item {
    fn time(&self) -> u64 {
        match self {
            Item::Commit(entry) => entry.time,
            Item::Saved(time, _) => *time,
        }
    }
}

const FIRST: usize = 10;
const PAGE: usize = 30;

type Loaded = (Option<PathBuf>, Vec<Item>);

#[derive(Default)]
pub struct Timeline {
    loaded: bool,
    path: Option<PathBuf>,
    entries: Option<Loaded>,
    rx: Option<Receiver<(Loaded, bool)>>,
    more: bool,
    delete: Option<PathBuf>,
    range: Option<(PathBuf, usize, usize)>,
}

fn fetch(dir: &Path, name: &str, lines: Option<(usize, usize)>, skip: usize, count: usize) -> Result<Vec<Entry>, String> {
    let (skip, n) = (format!("--skip={skip}"), count.to_string());
    match lines {
        Some((a, b)) => crate::git::exec(dir, &["log", &format!("-L{a},{b}:{name}"), &skip, "-n", &n, FORMAT]),
        None => crate::git::exec(dir, &["log", "--follow", "--name-only", &skip, "-n", &n, FORMAT, "--", name]),
    }
    .map(|text| parse(&text))
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.split('\x1e')
        .filter_map(|record| {
            let (head, rest) = record.split_once('\n').unwrap_or((record, ""));
            let mut field = head.trim_end_matches('\r').splitn(6, '\x1f').map(str::to_owned);
            Some(Entry {
                hash: field.next()?,
                author: field.next()?,
                ago: field.next()?,
                date: field.next()?,
                time: field.next()?.parse().ok()?,
                subject: field.next()?,
                name: match rest.lines().find_map(|line| line.trim_end().strip_prefix("+++ b/")) {
                    Some(name) => name.to_owned(),
                    None => rest.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned(),
                },
            })
        })
        .collect()
}

pub fn merge(commits: Vec<Entry>, saves: Vec<(u128, PathBuf)>) -> Vec<Item> {
    let saves = saves.into_iter().map(|(time, path)| Item::Saved((time / 1_000_000_000) as u64, path));
    let mut items: Vec<Item> = commits.into_iter().map(Item::Commit).chain(saves).collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.time()));
    items
}

impl Timeline {
    pub fn reload(&mut self) {
        self.loaded = false;
    }

    pub fn show_lines(&mut self, path: PathBuf, first: usize, last: usize) {
        (self.range, self.loaded) = (Some((path, first, last)), false);
    }

    fn lines(&self) -> Option<(usize, usize)> {
        self.range.as_ref().map(|r| (r.1, r.2))
    }

    fn load(&mut self, path: Option<&Path>, ctx: &egui::Context, delete: Option<PathBuf>) {
        (self.loaded, self.path, self.entries, self.rx, self.more) = (true, path.map(Path::to_path_buf), Some((None, vec![])), None, false);
        let Some((file, dir, name)) = path.and_then(|p| Some((p.to_path_buf(), p.parent()?.to_path_buf(), p.file_name()?.to_string_lossy().into_owned()))) else { return };
        let (tx, rx) = channel();
        (self.entries, self.rx) = (None, Some(rx));
        let (ctx, lines) = (ctx.clone(), self.lines());
        std::thread::spawn(move || {
            if let Some(Err(e)) = delete.map(std::fs::remove_file) {
                ui::notify(format!("Could not delete the saved version: {e}"));
            }
            let log = fetch(&dir, &name, lines, 0, FIRST);
            let saves = if lines.is_some() { vec![] } else { history::snapshots(&file).unwrap_or_default() };
            let more = log.as_ref().is_ok_and(|log| log.len() >= FIRST);
            let repo = log.is_ok().then_some(dir);
            let _ = tx.send(((repo, merge(log.unwrap_or_default(), saves)), more));
            ctx.request_repaint();
        });
    }

    fn load_more(&mut self, ctx: &egui::Context) {
        let Some((Some(dir), items)) = self.entries.as_ref().filter(|_| self.more && self.rx.is_none()) else { return };
        let Some(name) = self.path.as_deref().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()) else { return };
        let (dir, skip) = (dir.clone(), items.iter().filter(|item| matches!(item, Item::Commit(_))).count());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let (ctx, lines) = (ctx.clone(), self.lines());
        std::thread::spawn(move || {
            let log = fetch(&dir, &name, lines, skip, PAGE).unwrap_or_default();
            let more = log.len() >= PAGE;
            let _ = tx.send(((Some(dir), log.into_iter().map(Item::Commit).collect()), more));
            ctx.request_repaint();
        });
    }

    fn receive(&mut self, ((repo, page), more): (Loaded, bool)) {
        let (_, items) = self.entries.get_or_insert((repo, vec![]));
        items.extend(page);
        items.sort_by_key(|item| std::cmp::Reverse(item.time()));
        (self.rx, self.more) = (None, more);
    }

    fn shown(&self) -> &[Item] {
        let items = self.entries.as_ref().map_or(&[][..], |(_, items)| items);
        let oldest = items.iter().filter(|item| matches!(item, Item::Commit(_))).map(Item::time).min();
        match oldest.filter(|_| self.more) {
            Some(oldest) => &items[..items.partition_point(|item| item.time() >= oldest)],
            None => items,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, active: Option<&Path>, open: &mut bool, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rect = ui::section(ui, theme, "TIMELINE", open).on_hover_text("Commits and saved versions of the active file").rect;
        self.confirm(ui, theme, active);
        if self.range.as_ref().is_some_and(|r| Some(r.0.as_path()) != active) {
            (self.range, self.loaded) = (None, false);
        }
        if ui.available_height() < 1.0 {
            return;
        }
        if let Some((_, first, last)) = self.range.clone().filter(|_| *open) {
            let label = if first == last { format!("Line {first}") } else { format!("Lines {first}-{last}") };
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            ui.painter().text(pos2(r.left() + 20.0, r.center().y), egui::Align2::LEFT_CENTER, format!("{label} of the file"), FontId::proportional(11.5), theme.text_dim);
            let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), Vec2::splat(20.0))));
            if ui::icon_button(&mut child, theme, icons::X, 18.0, false, "Show the whole file").clicked() {
                (self.range, self.loaded) = (None, false);
            }
        }
        if *open {
            let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(20.0))));
            let refresh = ui::icon_button(&mut child, theme, icons::ARROW_CLOCKWISE, 20.0, false, "Refresh").clicked();
            if refresh || !self.loaded || self.path.as_deref() != active {
                self.load(active, ui.ctx(), None);
            }
        }
        if let Some(page) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.receive(page);
        }
        let Some(file) = active else { return note(ui, theme, "No active editor.") };
        let Some((repo, _)) = &self.entries else { return ui::skeleton(ui, theme, 4) };
        let items = self.shown();
        if items.is_empty() {
            let empty = if self.range.is_some() { "No commits changed these lines." } else { "No commits changed this file yet." };
            return note(ui, theme, if repo.is_some() { empty } else { "This file is not in a Git repository." });
        }
        let name = history::name(file);
        let mut delete = None;
        let mut end = false;
        ScrollArea::vertical().id_salt("timeline").auto_shrink(false).show(ui, |ui| {
            for item in items {
                let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                ui::row_bg(ui, theme, r, row.id, row.hovered(), false);
                let painter = ui.painter().with_clip_rect(r);
                let (icon, title, meta, tip) = match item {
                    Item::Commit(entry) => {
                        let tip = format!("{}\n{}, {}\n\n{}", entry.hash, entry.author, entry.date, entry.subject);
                        (icons::GIT_COMMIT, entry.subject.clone(), format!("{}, {}", entry.author, entry.ago), tip)
                    }
                    Item::Saved(time, _) => (icons::FLOPPY_DISK, "File Saved".to_owned(), history::age(*time), "A version saved in Halo".to_owned()),
                };
                ui::item(ui, &row, ui::Role::ListItem, &format!("{title}, {meta}"), false);
                painter.text(pos2(r.left() + 20.0, r.center().y), egui::Align2::CENTER_CENTER, icon, FontId::proportional(13.0), theme.text_dim);
                let meta = painter.layout_no_wrap(meta, FontId::proportional(11.5), theme.text_dim);
                let mut job = egui::text::LayoutJob::simple_singleline(title, FontId::proportional(12.5), theme.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 50.0 - meta.size().x).max(60.0));
                let subject = painter.layout_job(job);
                let after = r.left() + 38.0 + subject.size().x;
                painter.galley(pos2(r.left() + 30.0, r.center().y - subject.size().y / 2.0), subject, theme.text);
                painter.galley(pos2(after, r.center().y - meta.size().y / 2.0), meta, theme.text_dim);
                let row = row.on_hover_text(tip);
                let open = |cmds: &mut Vec<Command>| match item {
                    Item::Commit(entry) => cmds.push(crate::blame::changes(repo.as_deref().unwrap_or(Path::new(".")), &entry.name, &entry.hash)),
                    Item::Saved(_, snap) => cmds.push(Command::OpenDiff(format!("{name} (Saved) ↔ {name}"), history::diff_args(snap, file))),
                };
                if row.clicked() {
                    open(cmds);
                }
                row.context_menu(|ui| {
                    ui.set_min_width(170.0);
                    ui::menu_pad(ui);
                    match item {
                        Item::Commit(entry) => {
                            if ui::menu_item(ui, "Copy Commit Hash").clicked() {
                                ui.ctx().copy_text(entry.hash.clone());
                                ui.close();
                            }
                        }
                        Item::Saved(_, snap) => {
                            if ui::menu_item(ui, "Restore Contents").clicked() {
                                cmds.push(Command::RestoreSnapshot(file.to_path_buf(), snap.clone()));
                                ui.close();
                            }
                            if ui::menu_item(ui, "Delete").clicked() {
                                delete = Some(snap.clone());
                                ui.close();
                            }
                        }
                    }
                    if ui::menu_item(ui, "Open Changes").clicked() {
                        open(cmds);
                        ui.close();
                    }
                });
            }
            end = ui::list_end(ui, theme, self.rx.is_some());
        });
        if end {
            self.load_more(ui.ctx());
        }
        self.delete = delete.or(self.delete.take());
    }

    fn confirm(&mut self, ui: &mut Ui, theme: &Theme, active: Option<&Path>) {
        if self.delete.is_none() {
            return;
        }
        let mut choice = None;
        let modal = ui::modal(ui.ctx(), "timeline_delete", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(340.0);
            ui.heading("Delete");
            ui.label("Delete this saved version? This cannot be undone.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if keys.primary(ui, RichText::new("Delete").color(theme.deleted), true) {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if modal.should_close() {
            choice = Some(false);
        }
        if let Some(delete) = choice {
            let snap = self.delete.take();
            if delete {
                self.load(active, ui.ctx(), snap);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_followed_log() {
        let text = "\x1eabc123\x1fAda\x1f2 days ago\x1f2026-11-18T10:00:00+01:00\x1f1795000000\x1fRename: a \x1f b\n\nsrc/new.rs\n\x1edef456\x1fBob\x1f3 weeks ago\x1f2026-10-30T09:00:00+01:00\x1f1793000000\x1fFirst\n\nsrc/old.rs\n";
        let entries = parse(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], Entry {
            hash: "abc123".into(),
            author: "Ada".into(),
            ago: "2 days ago".into(),
            date: "2026-11-18T10:00:00+01:00".into(),
            time: 1795000000,
            subject: "Rename: a \x1f b".into(),
            name: "src/new.rs".into(),
        });
        assert_eq!((entries[1].subject.as_str(), entries[1].name.as_str()), ("First", "src/old.rs"));
        assert!(parse("").is_empty());
        let items = merge(entries, vec![(1_794_000_000_000_000_000, "s".into()), (1_796_000_000_000_000_000, "t".into())]);
        let times: Vec<u64> = items.iter().map(Item::time).collect();
        assert_eq!(times, [1796000000, 1795000000, 1794000000, 1793000000]);
        assert_eq!(items[0], Item::Saved(1796000000, "t".into()));
    }

    fn commit(time: u64) -> Entry {
        Entry { hash: format!("h{time}"), author: "a".into(), ago: String::new(), date: String::new(), time, subject: format!("c{time}"), name: "f".into() }
    }

    #[test]
    fn pages_keep_the_merged_order() {
        let mut t = Timeline::default();
        let saves = vec![(95_000_000_000, "s95".into()), (50_000_000_000, "s50".into())];
        t.receive(((Some("repo".into()), merge((91..=100).rev().map(commit).collect(), saves)), true));
        let times = |t: &Timeline| t.shown().iter().map(Item::time).collect::<Vec<_>>();
        assert_eq!(times(&t), [100, 99, 98, 97, 96, 95, 95, 94, 93, 92, 91]);
        t.receive(((Some("repo".into()), (60..=90).rev().map(|n| Item::Commit(commit(n))).collect()), true));
        assert_eq!((t.shown().len(), t.shown().last().map(Item::time)), (42, Some(60)));
        t.receive(((Some("repo".into()), vec![Item::Commit(commit(40))]), false));
        assert_eq!(times(&t)[41..], [60, 50, 40]);
        assert!(t.rx.is_none() && !t.more);
        t.load(None, &egui::Context::default(), None);
        assert_eq!((t.shown().len(), t.more), (0, false));
    }

    #[test]
    fn follows_a_file_page_by_page() {
        let dir = std::env::temp_dir().join(format!("halo-timeline-page-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        for n in 0..25 {
            std::fs::write(dir.join("a.txt"), n.to_string()).unwrap();
            git(&["add", "a.txt"]);
            git(&["commit", "-q", "-m", &format!("c{n}")]);
        }
        let subjects = |log: Vec<Entry>| log.into_iter().map(|e| e.subject).collect::<Vec<_>>();
        let first = subjects(fetch(&dir, "a.txt", None, 0, FIRST).unwrap());
        assert_eq!(first, (15..25).rev().map(|n| format!("c{n}")).collect::<Vec<_>>());
        let next = subjects(fetch(&dir, "a.txt", None, FIRST, PAGE).unwrap());
        assert_eq!(next, (0..15).rev().map(|n| format!("c{n}")).collect::<Vec<_>>());
        assert!(next.len() < PAGE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_history_lists_only_commits_that_touched_the_range() {
        let dir = std::env::temp_dir().join(format!("halo-timeline-lines-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| crate::git::exec(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        for (text, message) in [("a\nb\nc\n", "add"), ("A\nb\nc\n", "first line"), ("A\nb\nC\n", "last line")] {
            std::fs::write(dir.join("a.txt"), text).unwrap();
            git(&["add", "a.txt"]);
            git(&["commit", "-q", "-m", message]);
        }
        let log = fetch(&dir, "a.txt", Some((3, 3)), 0, FIRST).unwrap();
        assert_eq!(log.iter().map(|e| e.subject.as_str()).collect::<Vec<_>>(), ["last line", "add"]);
        assert_eq!(log[0].name, "a.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headless_scrolling_to_the_end_requests_the_next_page() {
        use egui::{Event, MouseWheelUnit, RawInput};
        let mut t = Timeline { loaded: true, path: Some("missing/a.txt".into()), more: true, ..Default::default() };
        t.entries = Some((Some("missing".into()), (1..=30).rev().map(|n| Item::Commit(commit(n))).collect()));
        let (ctx, theme) = (egui::Context::default(), Theme::dark());
        let mut time = 0.0;
        let mut run = |t: &mut Timeline, events: Vec<Event>| {
            time += 0.1;
            let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 260.0))), time: Some(time), events, ..Default::default() };
            ctx.run_ui(input, |ui| t.ui(ui, &theme, Some(Path::new("missing/a.txt")), &mut true, &mut vec![])).textures_delta.clear();
        };
        run(&mut t, vec![]);
        run(&mut t, vec![]);
        assert!(t.rx.is_none());
        let mut requested = false;
        for _ in 0..20 {
            let wheel = Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, -400.0), modifiers: Default::default(), phase: egui::TouchPhase::Move };
            run(&mut t, vec![Event::PointerMoved(pos2(150.0, 150.0)), wheel]);
            if t.rx.is_some() {
                requested = true;
                break;
            }
        }
        assert!(requested);
    }
}
