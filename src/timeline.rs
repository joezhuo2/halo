use crate::app::note;
use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, FontId, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

const FORMAT: &str = "--format=%x1e%H%x1f%an%x1f%ar%x1f%aI%x1f%s";

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub hash: String,
    pub author: String,
    pub ago: String,
    pub date: String,
    pub subject: String,
    pub name: String,
}

#[derive(Default)]
pub struct Timeline {
    loaded: bool,
    path: Option<PathBuf>,
    entries: Option<Option<Vec<Entry>>>,
    rx: Option<Receiver<Option<Vec<Entry>>>>,
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.split('\x1e')
        .filter_map(|record| {
            let (head, rest) = record.split_once('\n').unwrap_or((record, ""));
            let mut field = head.trim_end_matches('\r').splitn(5, '\x1f').map(str::to_owned);
            Some(Entry {
                hash: field.next()?,
                author: field.next()?,
                ago: field.next()?,
                date: field.next()?,
                subject: field.next()?,
                name: rest.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

impl Timeline {
    fn load(&mut self, path: Option<&Path>, ctx: &egui::Context) {
        (self.loaded, self.path, self.entries, self.rx) = (true, path.map(Path::to_path_buf), Some(None), None);
        let Some((dir, name)) = path.and_then(|p| Some((p.parent()?.to_path_buf(), p.file_name()?.to_string_lossy().into_owned()))) else { return };
        let (tx, rx) = channel();
        (self.entries, self.rx) = (None, Some(rx));
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let log = crate::git::exec(&dir, &["log", "--follow", "--name-only", "-n", "200", FORMAT, "--", &name]);
            let _ = tx.send(log.ok().map(|text| parse(&text)));
            ctx.request_repaint();
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, active: Option<&Path>, open: &mut bool, cmds: &mut Vec<Command>) {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rect = ui::section(ui, theme, "TIMELINE", open).on_hover_text("Commits that changed the active file").rect;
        if ui.available_height() < 1.0 {
            return;
        }
        if *open {
            let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), Vec2::splat(20.0))));
            let refresh = ui::icon_button(&mut child, theme, icons::ARROW_CLOCKWISE, 20.0, false, "Refresh").clicked();
            if refresh || !self.loaded || self.path.as_deref() != active {
                self.load(active, ui.ctx());
            }
        }
        if let Some(entries) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            (self.entries, self.rx) = (Some(entries), None);
        }
        let Some(file) = active else { return note(ui, theme, "No active editor.") };
        let Some(entries) = &self.entries else { return ui::skeleton(ui, theme, 4) };
        let Some(entries) = entries else { return note(ui, theme, "This file is not in a Git repository.") };
        if entries.is_empty() {
            return note(ui, theme, "No commits changed this file yet.");
        }
        let file = file.file_name().unwrap_or(file.as_os_str()).to_string_lossy();
        ScrollArea::vertical().id_salt("timeline").auto_shrink(false).show(ui, |ui| {
            for entry in entries {
                let (r, row) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                ui::row_bg(ui, theme, r, row.id, row.hovered(), false);
                let painter = ui.painter().with_clip_rect(r);
                let meta = painter.layout_no_wrap(format!("{}, {}", entry.author, entry.ago), FontId::proportional(11.5), theme.text_dim);
                let mut job = egui::text::LayoutJob::simple_singleline(entry.subject.clone(), FontId::proportional(12.5), theme.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 32.0 - meta.size().x).max(60.0));
                let subject = painter.layout_job(job);
                let after = r.left() + 20.0 + subject.size().x;
                painter.galley(pos2(r.left() + 12.0, r.center().y - subject.size().y / 2.0), subject, theme.text);
                painter.galley(pos2(after, r.center().y - meta.size().y / 2.0), meta, theme.text_dim);
                let tip = format!("{}\n{}, {}\n\n{}", entry.hash, entry.author, entry.date, entry.subject);
                if row.on_hover_text(tip).clicked() {
                    let short = &entry.hash[..entry.hash.len().min(8)];
                    let args = ["show", "--format=", "--diff-merges=first-parent", &entry.hash, "--", &entry.name];
                    cmds.push(Command::OpenDiff(format!("{file} ({short})"), args.map(str::to_owned).to_vec()));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_followed_log() {
        let text = "\x1eabc123\x1fAda\x1f2 days ago\x1f2026-11-18T10:00:00+01:00\x1fRename: a \x1f b\n\nsrc/new.rs\n\x1edef456\x1fBob\x1f3 weeks ago\x1f2026-10-30T09:00:00+01:00\x1fFirst\n\nsrc/old.rs\n";
        let entries = parse(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], Entry {
            hash: "abc123".into(),
            author: "Ada".into(),
            ago: "2 days ago".into(),
            date: "2026-11-18T10:00:00+01:00".into(),
            subject: "Rename: a \x1f b".into(),
            name: "src/new.rs".into(),
        });
        assert_eq!((entries[1].subject.as_str(), entries[1].name.as_str()), ("First", "src/old.rs"));
        assert!(parse("").is_empty());
    }
}
