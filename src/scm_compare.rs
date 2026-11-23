use super::{ROW, Scm, buttons};
use crate::command::Command;
use crate::explorer::{state_badge, state_color};
use crate::git::{Change, FileState, Git, exec};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, FontId, Frame, Id, Margin, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

type Files = Result<(String, Vec<Change>), String>;

struct Pick {
    left: Option<String>,
    query: String,
    refs: Option<Vec<String>>,
    rx: Receiver<Vec<String>>,
}

struct View {
    top: PathBuf,
    left: String,
    right: String,
    three: bool,
    files: Option<Files>,
    rx: Receiver<Files>,
}

#[derive(Default)]
pub(super) struct Compare {
    pick: Option<Pick>,
    view: Option<View>,
}

fn refs(top: &Path) -> Vec<String> {
    let args = ["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes", "refs/tags"];
    crate::git::run(top, &args).unwrap_or_default().lines().map(str::to_owned).collect()
}

fn files(top: &Path, left: &str, right: &str, three: bool) -> Files {
    if [left, right].iter().any(|r| r.starts_with('-')) {
        return Err("A reference cannot start with '-'.".into());
    }
    let base = if three { exec(top, &["merge-base", left, right])?.trim_end().to_owned() } else { left.to_owned() };
    let text = exec(top, &["diff", "--name-status", "-z", "--no-renames", &base, right, "--"])?;
    Ok((base, crate::git::parse_name_status(&text)))
}

fn matching<'a>(items: &'a [(String, String)], query: &str) -> Vec<&'a (String, String)> {
    let query = query.trim().to_lowercase();
    items.iter().filter(|(_, label)| label.to_lowercase().contains(&query)).take(200).collect()
}

impl Scm {
    pub(super) fn compare_start(&mut self, left: Option<String>, git: &mut Git, ctx: &egui::Context) {
        let Some(top) = git.top().map(Path::to_path_buf) else {
            return git.set_error("Open a Git repository to use this command.".into());
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(refs(&top));
            ctx.request_repaint();
        });
        self.compare.pick = Some(Pick { left, query: String::new(), refs: None, rx });
    }

    fn compare_open(&mut self, left: String, right: String, three: bool, git: &Git, ctx: &egui::Context) {
        let Some(top) = git.top().map(Path::to_path_buf) else { return };
        let (tx, rx) = channel();
        let (dir, l, r, ctx) = (top.clone(), left.clone(), right.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(files(&dir, &l, &r, three));
            ctx.request_repaint();
        });
        self.compare.view = Some(View { top, left, right, three, files: None, rx });
        self.open[2] = true;
    }

    pub(super) fn compare_picker(&mut self, ui: &Ui, theme: &Theme, git: &Git) {
        let Some(pick) = self.compare.pick.as_mut() else { return };
        if let Ok(refs) = pick.rx.try_recv() {
            pick.refs = Some(refs);
        }
        let mut chosen = None;
        let mut done = false;
        let modal = ui::modal(ui.ctx(), "scm_compare_pick", ui::Enter::Primary, |ui, keys| {
            ui::outline(ui);
            ui.set_width(420.0);
            ui.heading(pick.left.as_ref().map_or_else(|| "Compare: select a reference".to_owned(), |left| format!("Compare {left} with...")));
            let edit = egui::TextEdit::singleline(&mut pick.query).id(Id::new("scm_compare_query")).hint_text("Branch, tag or commit");
            let resp = ui.add(edit.margin(ui::FIELD).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                resp.request_focus();
            }
            let commits = git.log().iter().map(|c| (c.hash.clone(), format!("{}  {}", c.hash.get(..8).unwrap_or(&c.hash), c.subject)));
            let items: Vec<(String, String)> = pick.refs.iter().flatten().map(|r| (r.clone(), r.clone())).chain(commits)
                .filter(|(value, _)| Some(value) != pick.left.as_ref()).collect();
            let shown = matching(&items, &pick.query);
            let query = pick.query.trim();
            ui.add_space(6.0);
            if !query.is_empty() && keys.choice(ui, &format!("Use {query}"), shown.is_empty()) {
                chosen = Some(query.to_owned());
            }
            if pick.refs.is_none() {
                ui::skeleton(ui, theme, 2);
            }
            ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (i, (value, label)) in shown.iter().enumerate() {
                    if keys.choice(ui, label, i == 0) {
                        chosen = Some(value.clone());
                    }
                }
            });
            done = ui.button("Cancel").clicked();
        });
        done |= modal.should_close();
        match chosen {
            Some(right) => match pick.left.take() {
                None => {
                    pick.left = Some(right);
                    pick.query.clear();
                }
                Some(left) => {
                    self.compare.pick = None;
                    self.compare_open(left, right, false, git, ui.ctx());
                }
            },
            None if done => self.compare.pick = None,
            None => {}
        }
    }

    pub(super) fn compare_view(&mut self, ui: &mut Ui, theme: &Theme, git: &Git, cmds: &mut Vec<Command>) {
        let Some(view) = self.compare.view.as_mut() else { return };
        if git.top() != Some(view.top.as_path()) {
            self.compare.view = None;
            return;
        }
        if let Ok(files) = view.rx.try_recv() {
            view.files = Some(files);
        }
        let dots = if view.three { "..." } else { ".." };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
        let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 48.0));
        let (x, y) = (rect.left() + 14.0, rect.center().y);
        painter.text(pos2(x, y), Align2::LEFT_CENTER, icons::GIT_DIFF, FontId::proportional(14.0), theme.accent);
        let title = painter.text(pos2(x + 20.0, y), Align2::LEFT_CENTER, format!("{}{dots}{}", view.left, view.right), FontId::proportional(13.0), theme.text);
        if let Some(Ok((_, files))) = &view.files {
            painter.text(pos2(title.right() + 8.0, y), Align2::LEFT_CENTER, format!("{} files", files.len()), FontId::proportional(11.5), theme.text_dim);
        }
        let toggle = if view.three { "Compare the trees (two-dot)" } else { "Compare with the merge base (three-dot)" };
        let hit = buttons(ui, theme, rect, true, &[(icons::X, "Close Comparison"), (icons::ARROWS_LEFT_RIGHT, toggle)]);
        match &view.files {
            None => ui::skeleton(ui, theme, 1),
            Some(Err(error)) => drop(Frame::new().inner_margin(Margin::symmetric(14, 4)).show(ui, |ui| {
                ui.label(RichText::new(error.lines().next().unwrap_or_default()).color(theme.error));
            })),
            Some(Ok((_, files))) if files.is_empty() => drop(Frame::new().inner_margin(Margin::symmetric(14, 4)).show(ui, |ui| {
                ui.label(RichText::new("No differences.").color(theme.text_dim));
            })),
            Some(Ok((base, files))) => {
                for change in files {
                    let (_, rect) = ui.allocate_space(vec2(ui.available_width(), ROW));
                    let id = Id::new(("scm_compare_file", &change.path));
                    let resp = ui.interact(rect, id, Sense::click());
                    ui::row_bg(ui, theme, rect, id, resp.hovered(), false);
                    ui::item(ui, &resp, ui::Role::ListItem, &change.path, false);
                    let painter = ui.painter().with_clip_rect(rect.with_max_x(rect.right() - 8.0));
                    let (dir, name) = change.path.rsplit_once('/').unwrap_or(("", &change.path));
                    let color = state_color(theme, Some(change.state));
                    let (x, y) = (rect.left() + 34.0, rect.center().y);
                    let drawn = painter.text(pos2(x, y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), color);
                    painter.text(pos2(drawn.right() + 8.0, y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
                    let badge = if change.state == FileState::Added { "A" } else { state_badge(Some(change.state)) };
                    painter.text(pos2(rect.right() - 14.0, y), Align2::CENTER_CENTER, badge, FontId::proportional(11.5), color);
                    if resp.clicked() {
                        let title = format!("{name} ({}{dots}{})", view.left, view.right);
                        cmds.push(Command::OpenDiff(title, ["diff", base.as_str(), view.right.as_str(), "--", change.path.as_str()].map(str::to_owned).to_vec()));
                    }
                }
            }
        }
        ui.add_space(4.0);
        match hit {
            Some(0) => self.compare.view = None,
            Some(_) => {
                let (left, right, three) = (view.left.clone(), view.right.clone(), !view.three);
                self.compare_open(left, right, three, git, ui.ctx());
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_changed_files_between_two_refs() {
        let dir = std::env::temp_dir().join(format!("halo-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| exec(&dir, args).unwrap();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"]] {
            git(args);
        }
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        git(&["switch", "-qc", "side"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/c.txt"), "c\n").unwrap();
        std::fs::write(dir.join("a.txt"), "side\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "side"]);
        git(&["switch", "-q", "trunk"]);
        std::fs::remove_file(dir.join("b.txt")).unwrap();
        git(&["commit", "-qam", "drop b"]);

        let names = |files: Files| files.unwrap().1.into_iter().map(|c| (c.path, c.state)).collect::<Vec<_>>();
        assert_eq!(names(files(&dir, "trunk", "side", false)), [("a.txt".into(), FileState::Modified), ("b.txt".into(), FileState::Added),
            ("src/c.txt".into(), FileState::Added)]);
        let (base, three) = files(&dir, "trunk", "side", true).unwrap();
        assert_eq!(base, git(&["rev-parse", "trunk~1"]).trim_end());
        assert_eq!(three.into_iter().map(|c| c.path).collect::<Vec<_>>(), ["a.txt", "src/c.txt"]);
        assert!(files(&dir, "trunk", "nope", false).is_err());
        assert!(files(&dir, "--output=x", "side", false).is_err());

        let items = [("trunk".to_owned(), "trunk".to_owned()), ("side".to_owned(), "side".to_owned()), ("abc".to_owned(), "abc  Side fix".to_owned())];
        assert_eq!(matching(&items, " SIDE").iter().map(|i| i.0.as_str()).collect::<Vec<_>>(), ["side", "abc"]);
        assert_eq!(matching(&items, "").len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
