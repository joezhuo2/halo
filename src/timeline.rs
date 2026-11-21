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

type Loaded = (Option<PathBuf>, Vec<Item>);

#[derive(Default)]
pub struct Timeline {
    loaded: bool,
    path: Option<PathBuf>,
    entries: Option<Loaded>,
    rx: Option<Receiver<Loaded>>,
    delete: Option<PathBuf>,
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
                name: rest.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned(),
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

    fn load(&mut self, path: Option<&Path>, ctx: &egui::Context, delete: Option<PathBuf>) {
        (self.loaded, self.path, self.entries, self.rx) = (true, path.map(Path::to_path_buf), Some((None, vec![])), None);
        let Some((file, dir, name)) = path.and_then(|p| Some((p.to_path_buf(), p.parent()?.to_path_buf(), p.file_name()?.to_string_lossy().into_owned()))) else { return };
        let (tx, rx) = channel();
        (self.entries, self.rx) = (None, Some(rx));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Some(Err(e)) = delete.map(std::fs::remove_file) {
                ui::notify(format!("Could not delete the saved version: {e}"));
            }
            let log = crate::git::exec(&dir, &["log", "--follow", "--name-only", "-n", "200", FORMAT, "--", &name]);
            let saves = history::snapshots(&file).unwrap_or_default();
            let repo = log.is_ok().then_some(dir);
            let _ = tx.send((repo, merge(log.map(|text| parse(&text)).unwrap_or_default(), saves)));
            ctx.request_repaint();
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, active: Option<&Path>, open: &mut bool, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rect = ui::section(ui, theme, "TIMELINE", open).on_hover_text("Commits and saved versions of the active file").rect;
        self.confirm(ui, theme, active);
        if ui.available_height() < 1.0 {
            return;
        }
        if *open {
            let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(20.0))));
            let refresh = ui::icon_button(&mut child, theme, icons::ARROW_CLOCKWISE, 20.0, false, "Refresh").clicked();
            if refresh || !self.loaded || self.path.as_deref() != active {
                self.load(active, ui.ctx(), None);
            }
        }
        if let Some(entries) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            (self.entries, self.rx) = (Some(entries), None);
        }
        let Some(file) = active else { return note(ui, theme, "No active editor.") };
        let Some((repo, items)) = &self.entries else { return ui::skeleton(ui, theme, 4) };
        if items.is_empty() {
            return note(ui, theme, if repo.is_some() { "No commits changed this file yet." } else { "This file is not in a Git repository." });
        }
        let name = history::name(file);
        let mut delete = None;
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
                            if ui.button("Copy Commit Hash").clicked() {
                                ui.ctx().copy_text(entry.hash.clone());
                                ui.close();
                            }
                        }
                        Item::Saved(_, snap) => {
                            if ui.button("Restore Contents").clicked() {
                                cmds.push(Command::RestoreSnapshot(file.to_path_buf(), snap.clone()));
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                delete = Some(snap.clone());
                                ui.close();
                            }
                        }
                    }
                    if ui.button("Open Changes").clicked() {
                        open(cmds);
                        ui.close();
                    }
                });
            }
        });
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
}
