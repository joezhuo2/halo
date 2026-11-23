use super::Scm;
use crate::git::{Git, exec, ops, run};
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Color32, CursorIcon, Frame, Key, Margin, Modifiers, RichText, ScrollArea, Sense, Stroke, Ui};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

const ACTS: [(Act, &str, Key); 6] = [
    (Act::Pick, "pick", Key::P),
    (Act::Reword, "reword", Key::R),
    (Act::Edit, "edit", Key::E),
    (Act::Squash, "squash", Key::S),
    (Act::Fixup, "fixup", Key::F),
    (Act::Drop, "drop", Key::D),
];

fn name(act: Act) -> &'static str {
    ACTS.iter().find(|a| a.0 == act).map_or("", |a| a.1)
}

#[derive(Clone, Debug)]
struct Row {
    hash: String,
    message: String,
    act: Act,
    text: String,
}

impl Row {
    fn set(&mut self, act: Act) {
        if self.act != act {
            self.act = act;
            self.text = if act == Act::Reword { self.message.clone() } else { String::new() };
        }
    }
}

#[derive(Clone)]
struct Plan {
    head: String,
    parent: Option<String>,
    rows: Vec<Row>,
}

pub(super) struct Editor {
    top: PathBuf,
    base: String,
    rx: Receiver<Result<Plan, String>>,
    plan: Option<Result<Plan, String>>,
    selected: usize,
    dragging: Option<usize>,
    error: Option<String>,
}

fn short(rev: &str) -> &str {
    rev.get(..8).unwrap_or(rev)
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn parse(text: &str) -> Vec<Row> {
    text.split('\x1e').filter_map(|record| {
        let (hash, message) = record.trim_start_matches(['\r', '\n']).split_once('\0')?;
        Some(Row { hash: hash.to_owned(), message: message.trim_end().to_owned(), act: Act::Pick, text: String::new() })
    }).collect()
}

fn load(top: &Path, base: &str) -> Result<Plan, String> {
    if base.starts_with('-') {
        return Err("A revision cannot start with '-'.".into());
    }
    let head = exec(top, &["rev-parse", "HEAD"])?.trim_end().to_owned();
    let base = exec(top, &["rev-parse", "--verify", &format!("{base}^{{commit}}")])?.trim_end().to_owned();
    if run(top, &["merge-base", "--is-ancestor", &base, "HEAD"]).is_none() {
        return Err("Pick a commit on the current branch.".into());
    }
    let parent = run(top, &["rev-parse", "--verify", "--quiet", &format!("{base}^")]).map(|p| p.trim_end().to_owned());
    let range = parent.as_ref().map_or_else(|| "HEAD".to_owned(), |p| format!("{p}..HEAD"));
    let rows = parse(&exec(top, &["log", "--reverse", "--no-merges", "--format=%H%x00%B%x1e", &range])?);
    Ok(Plan { head, parent, rows })
}

fn todo(rows: &[Row], dir: &str) -> Result<(String, Vec<String>), String> {
    let (mut out, mut messages, mut group) = (String::new(), Vec::new(), None::<String>);
    for row in rows {
        let custom = Some(row.text.trim_end()).filter(|t| !t.trim().is_empty()).map(str::to_owned);
        let (word, message) = match row.act {
            Act::Drop => ("drop", None),
            Act::Squash | Act::Fixup => {
                let Some(prev) = group.as_mut() else {
                    return Err("The first commit cannot be squashed or fixed up. Pick it, or move it below another commit.".into());
                };
                if row.act == Act::Fixup {
                    ("fixup", None)
                } else {
                    *prev = custom.unwrap_or_else(|| format!("{}\n\n{}", prev, row.message));
                    ("fixup", Some(prev.clone()))
                }
            }
            Act::Reword => {
                let message = custom.filter(|t| *t != row.message);
                group = Some(message.clone().unwrap_or_else(|| row.message.clone()));
                ("pick", message)
            }
            Act::Pick | Act::Edit => {
                group = Some(row.message.clone());
                (name(row.act), None)
            }
        };
        out += &format!("{word} {}\n", row.hash);
        if let Some(message) = message {
            out += &format!("exec git commit --amend --allow-empty -q -F {}\n", quote(&format!("{dir}/{}.txt", messages.len())));
            messages.push(message + "\n");
        }
    }
    Ok((out, messages))
}

fn start(top: &Path, plan: &Plan) -> Result<Option<PathBuf>, String> {
    if ops::operation(top)?.is_some() {
        return Err("Finish the current Git operation first.".into());
    }
    if exec(top, &["rev-parse", "HEAD"])?.trim_end() != plan.head {
        return Err("HEAD changed. Open the interactive rebase again.".into());
    }
    let shell = exec(top, &["rev-parse", "--absolute-git-dir"])?.trim_end().to_owned() + "/halo-rebase";
    let local = crate::wsl::local(top, &shell);
    let (todo, messages) = todo(&plan.rows, &shell)?;
    let _ = std::fs::remove_dir_all(&local);
    let write = |name: String, text: &str| {
        std::fs::create_dir_all(&local).and_then(|_| std::fs::write(local.join(name), text))
            .map_err(|e| format!("Could not write the rebase plan: {e}"))
    };
    write("todo".into(), &todo)?;
    for (i, message) in messages.iter().enumerate() {
        write(format!("{i}.txt"), message)?;
    }
    let editor = format!("sequence.editor=cp {}", quote(&format!("{shell}/todo")));
    exec(top, &["-c", &editor, "rebase", "-i", plan.parent.as_deref().unwrap_or("--root")]).map(|_| None)
}

impl Editor {
    pub(super) fn open(top: &Path, base: String, ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let (dir, rev, ctx) = (top.to_path_buf(), base.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(load(&dir, &rev));
            ctx.request_repaint();
        });
        Self { top: top.to_path_buf(), base, rx, plan: None, selected: 0, dragging: None, error: None }
    }
}

fn rows(ui: &mut Ui, theme: &Theme, rows: &mut Vec<Row>, selected: &mut usize, dragging: &mut Option<usize>) {
    if rows.is_empty() {
        ui.weak("No commits to rebase.");
        return;
    }
    *selected = (*selected).min(rows.len() - 1);
    let typing = ui.ctx().text_edit_focused();
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::ALT, Key::ArrowUp) && *selected > 0 {
            rows.swap(*selected, *selected - 1);
            *selected -= 1;
        }
        if i.consume_key(Modifiers::ALT, Key::ArrowDown) && *selected + 1 < rows.len() {
            rows.swap(*selected, *selected + 1);
            *selected += 1;
        }
        if typing {
            return;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            *selected = selected.saturating_sub(1);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            *selected = (*selected + 1).min(rows.len() - 1);
        }
        for (act, _, key) in ACTS {
            if i.consume_key(Modifiers::NONE, key) {
                rows[*selected].set(act);
            }
        }
    });
    let mut rects = Vec::with_capacity(rows.len());
    ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        for (i, row) in rows.iter_mut().enumerate() {
            let fill = if i == *selected { theme.selection } else { Color32::TRANSPARENT };
            let frame = Frame::new().fill(fill).inner_margin(Margin::symmetric(4, 2)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let grip = RichText::new(icons::DOTS_SIX_VERTICAL).color(theme.text_dim);
                    let handle = ui.add(egui::Label::new(grip).sense(Sense::click_and_drag())).on_hover_cursor(CursorIcon::Grab);
                    if handle.drag_started() {
                        *dragging = Some(i);
                    }
                    let mut act = row.act;
                    egui::ComboBox::from_id_salt(("scm_rebase_act", i)).width(76.0).selected_text(name(act)).show_ui(ui, |ui| {
                        for (a, label, _) in ACTS {
                            ui.selectable_value(&mut act, a, label);
                        }
                    });
                    if act != row.act {
                        row.set(act);
                        *selected = i;
                    }
                    ui.label(RichText::new(short(&row.hash)).monospace().color(theme.text_dim));
                    let dropped = row.act == Act::Drop;
                    let subject = RichText::new(row.message.lines().next().unwrap_or_default());
                    let subject = if dropped { subject.strikethrough().color(theme.text_dim) } else { subject.color(theme.text) };
                    let label = ui.add(egui::Label::new(subject).truncate().sense(Sense::click()));
                    if label.clicked() || handle.clicked() {
                        *selected = i;
                    }
                });
                if i == *selected && matches!(row.act, Act::Reword | Act::Squash) {
                    let hint = if row.act == Act::Squash { "Message of the combined commit (empty joins both messages)" } else { "Commit message" };
                    let edit = egui::TextEdit::multiline(&mut row.text).desired_rows(3).hint_text(hint).margin(ui::FIELD);
                    ui.add(edit.desired_width(f32::INFINITY));
                }
            });
            rects.push(frame.response.rect);
        }
    });
    let Some(from) = *dragging else { return };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let target = pointer.map_or(from, |p| rects.iter().position(|r| p.y < r.center().y).unwrap_or(rects.len()));
    if ui.input(|i| i.pointer.primary_down()) {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let y = rects.get(target).map_or_else(|| rects.last().map_or(0.0, |r| r.bottom()), |r| r.top());
        ui.painter().hline(rects[0].x_range(), y, Stroke::new(2.0, theme.accent));
    } else {
        *dragging = None;
        let to = if target > from { target - 1 } else { target };
        let row = rows.remove(from);
        rows.insert(to, row);
        *selected = to;
    }
}

impl Scm {
    pub(super) fn rebase_dialog(&mut self, ui: &Ui, theme: &Theme, git: &mut Git) {
        let Some(editor) = self.rebase.as_mut() else { return };
        if git.top() != Some(editor.top.as_path()) {
            self.rebase = None;
            return;
        }
        if let Ok(plan) = editor.rx.try_recv() {
            editor.plan = Some(plan);
        }
        let mut go = false;
        let mut done = false;
        let busy = git.busy();
        let modal = ui::modal(ui.ctx(), "scm_rebase", ui::Enter::Multiline, |ui, keys| {
            ui::outline(ui);
            ui.set_width(600.0);
            ui.heading("Interactive Rebase");
            let ok = match &mut editor.plan {
                None => {
                    ui::skeleton(ui, theme, 3);
                    false
                }
                Some(Err(error)) => {
                    ui.label(RichText::new(error.as_str()).color(theme.error));
                    false
                }
                Some(Ok(plan)) => {
                    let note = format!("{} commits from {} to HEAD, oldest first. Drag the grip or press Alt+Up/Down to reorder; P, R, E, S, F, D set the action.",
                        plan.rows.len(), short(&editor.base));
                    ui.label(RichText::new(note).color(theme.text_dim));
                    ui.add_space(6.0);
                    rows(ui, theme, &mut plan.rows, &mut editor.selected, &mut editor.dragging);
                    !plan.rows.is_empty()
                }
            };
            if let Some(error) = &editor.error {
                ui.label(RichText::new(error.as_str()).color(theme.error));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                go = keys.primary(ui, "Start Rebase", ok && !busy);
                done = ui.button("Cancel").clicked();
            });
        });
        done |= modal.should_close();
        if go && let Some(Ok(plan)) = &editor.plan {
            match todo(&plan.rows, "") {
                Err(error) => editor.error = Some(error),
                Ok(_) => {
                    let (top, plan) = (editor.top.clone(), plan.clone());
                    git.job(move || start(&top, &plan));
                    done = true;
                }
            }
        }
        if done {
            self.rebase = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, RawInput};

    fn row(hash: &str, message: &str, act: Act, text: &str) -> Row {
        Row { hash: hash.into(), message: message.into(), act, text: text.into() }
    }

    #[test]
    fn writes_the_todo_and_message_files() {
        let rows = [
            row("a", "one", Act::Pick, ""),
            row("b", "two", Act::Squash, ""),
            row("c", "three", Act::Fixup, ""),
            row("d", "four", Act::Reword, "four"),
            row("e", "five", Act::Reword, "FIVE"),
            row("f", "six", Act::Squash, "it's new"),
            row("g", "seven", Act::Drop, ""),
            row("h", "eight", Act::Edit, ""),
        ];
        let (text, messages) = todo(&rows, "/g/halo-rebase").unwrap();
        let exec = |n: usize| format!("exec git commit --amend --allow-empty -q -F '/g/halo-rebase/{n}.txt'\n");
        assert_eq!(text, format!("pick a\nfixup b\n{}fixup c\npick d\npick e\n{}fixup f\n{}drop g\nedit h\n", exec(0), exec(1), exec(2)));
        assert_eq!(messages, ["one\n\ntwo\n", "FIVE\n", "it's new\n"]);
        assert_eq!(quote("/it's"), r"'/it'\''s'");
        assert!(todo(&[row("a", "x", Act::Fixup, "")], "").is_err());
        assert!(todo(&[row("a", "x", Act::Drop, ""), row("b", "y", Act::Squash, "")], "").is_err());
        assert_eq!(parse("abc\0subject\n\nbody\n\x1e\ndef\0two\n\x1e\n").iter().map(|r| (r.hash.as_str(), r.message.as_str())).collect::<Vec<_>>(),
            [("abc", "subject\n\nbody"), ("def", "two")]);
    }

    fn repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-rebase-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"]] {
            exec(&dir, args).unwrap();
        }
        for n in 1..=5 {
            std::fs::write(dir.join(format!("{n}.txt")), "x\n").unwrap();
            exec(&dir, &["add", "."]).unwrap();
            exec(&dir, &["commit", "-qm", &format!("c{n}")]).unwrap();
        }
        dir
    }

    fn subjects(dir: &Path) -> String {
        exec(dir, &["log", "--format=%s", "--reverse"]).unwrap().trim_end().replace('\n', ",")
    }

    #[test]
    fn reorders_rewords_squashes_and_stops_for_edits() {
        let dir = repo("run");
        let c2 = exec(&dir, &["rev-parse", "HEAD~3"]).unwrap().trim_end().to_owned();
        let mut plan = load(&dir, &c2).unwrap();
        assert_eq!(plan.rows.iter().map(|r| r.message.as_str()).collect::<Vec<_>>(), ["c2", "c3", "c4", "c5"]);
        assert!(load(&dir, "--all").is_err());
        plan.rows.swap(0, 1);
        plan.rows[0].set(Act::Reword);
        plan.rows[0].text = "third\n\nbody".into();
        plan.rows[1].set(Act::Squash);
        plan.rows[2].set(Act::Drop);
        start(&dir, &plan).unwrap();
        assert_eq!(subjects(&dir), "c1,third,c5");
        assert_eq!(exec(&dir, &["log", "-1", "--format=%B", "HEAD~1"]).unwrap().trim_end(), "third\n\nbody\n\nc2");
        assert!(!dir.join("4.txt").exists() && dir.join("3.txt").exists() && dir.join("2.txt").exists());
        assert!(start(&dir, &plan).unwrap_err().contains("HEAD changed"));

        let root = exec(&dir, &["rev-list", "--max-parents=0", "HEAD"]).unwrap().trim_end().to_owned();
        let mut plan = load(&dir, &root).unwrap();
        assert_eq!((plan.parent.as_deref(), plan.rows.len()), (None, 3));
        plan.rows[1].set(Act::Edit);
        start(&dir, &plan).unwrap();
        assert_eq!(ops::operation(&dir), Ok(Some("rebase")));
        exec(&dir, &["-c", "core.editor=true", "rebase", "--continue"]).unwrap();
        assert_eq!(ops::operation(&dir), Ok(None));
        assert_eq!(subjects(&dir), "c1,third,c5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_the_plan_by_keyboard_and_starts_from_the_dialog() {
        let dir = repo("ui");
        let ctx = egui::Context::default();
        let (mut scm, mut git) = (Scm::new(), Git::new());
        git.set_roots(vec![dir.clone()], &ctx);
        git.wait(|g| g.top().is_some());
        let c4 = exec(&dir, &["rev-parse", "HEAD~1"]).unwrap().trim_end().to_owned();
        scm.rebase = Some(Editor::open(git.top().unwrap(), c4, &ctx));
        let frame = |scm: &mut Scm, git: &mut Git, events: Vec<Event>| {
            git.poll();
            let input = RawInput { events, focused: true, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| scm.rebase_dialog(ui, &Theme::dark(), git));
            out.textures_delta.clear();
        };
        for _ in 0..500 {
            frame(&mut scm, &mut git, vec![]);
            if scm.rebase.as_ref().is_some_and(|e| e.plan.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let key = |key, modifiers| [true, false].map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
        frame(&mut scm, &mut git, key(Key::ArrowDown, Modifiers::ALT));
        frame(&mut scm, &mut git, [key(Key::ArrowUp, Modifiers::NONE), key(Key::S, Modifiers::NONE)].concat());
        let editor = scm.rebase.as_ref().unwrap();
        let Some(Ok(plan)) = &editor.plan else { panic!("plan did not load") };
        assert_eq!(plan.rows.iter().map(|r| (r.message.as_str(), r.act)).collect::<Vec<_>>(), [("c5", Act::Squash), ("c4", Act::Pick)]);
        frame(&mut scm, &mut git, key(Key::Enter, Modifiers::CTRL));
        assert!(scm.rebase.as_ref().unwrap().error.is_some());
        frame(&mut scm, &mut git, key(Key::ArrowDown, Modifiers::ALT));
        frame(&mut scm, &mut git, key(Key::Enter, Modifiers::CTRL));
        assert!(scm.rebase.is_none());
        git.wait(|g| !g.busy());
        assert_eq!(git.error(), None);
        assert_eq!(subjects(&dir), "c1,c2,c3,c4");
        assert_eq!(exec(&dir, &["log", "-1", "--format=%B"]).unwrap().trim_end(), "c4\n\nc5");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
