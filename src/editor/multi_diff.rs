use crate::command::Command;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

#[derive(Debug, PartialEq)]
pub struct Line {
    pub kind: char,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct FileDiff {
    pub path: String,
    pub from: Option<String>,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
    pub lines: Vec<Line>,
}

pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = vec![];
    let (mut old, mut new) = (1, 1);
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("diff --git ") {
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b);
            files.push(FileDiff { path: path.to_owned(), ..FileDiff::default() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some(rest) = raw.strip_prefix("@@ ") {
            let mut starts = rest.split(' ').take(2).map(|part| part.get(1..).and_then(|p| p.split(',').next()?.parse().ok()).unwrap_or(1));
            (old, new) = (starts.next().unwrap_or(1), starts.next().unwrap_or(1));
            file.lines.push(Line { kind: '@', old: None, new: None, text: raw.to_owned() });
        } else if file.lines.is_empty() {
            if let Some(from) = raw.strip_prefix("rename from ") {
                file.from = Some(from.to_owned());
            } else if let Some(to) = raw.strip_prefix("rename to ") {
                file.path = to.to_owned();
            } else if raw.starts_with("Binary files") || raw == "GIT binary patch" {
                file.binary = true;
            }
        } else {
            let text = raw.get(1..).unwrap_or_default().replace('\t', "    ");
            match raw.chars().next() {
                Some('+') => {
                    file.added += 1;
                    file.lines.push(Line { kind: '+', old: None, new: Some(new), text });
                    new += 1;
                }
                Some('-') => {
                    file.removed += 1;
                    file.lines.push(Line { kind: '-', old: Some(old), new: None, text });
                    old += 1;
                }
                Some(' ') | None => {
                    file.lines.push(Line { kind: ' ', old: Some(old), new: Some(new), text });
                    (old, new) = (old + 1, new + 1);
                }
                _ => {}
            }
        }
    }
    files
}

pub fn single(args: &[String], path: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = match args.first().map(String::as_str) {
        Some("diff" | "show") => args.iter().take_while(|a| *a != "--").cloned().collect(),
        Some("stash") => {
            let hash = args.last()?;
            vec!["diff".into(), format!("{hash}^"), hash.clone()]
        }
        _ => return None,
    };
    out.extend(["--".to_owned(), path.to_owned()]);
    Some(out)
}

pub struct MultiDiff {
    title: String,
    top: PathBuf,
    args: Vec<String>,
    rx: Option<Receiver<Result<String, String>>>,
    files: Vec<FileDiff>,
    error: Option<String>,
    closed: HashSet<usize>,
}

impl MultiDiff {
    pub fn load(title: &str, top: PathBuf, args: Vec<String>, ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let (dir, list, ctx) = (top.clone(), args.clone(), ctx.clone());
        std::thread::spawn(move || {
            let list: Vec<&str> = list.iter().map(String::as_str).collect();
            let _ = tx.send(crate::git::exec(&dir, &list));
            ctx.request_repaint();
        });
        Self { title: title.to_owned(), top, args, rx: Some(rx), files: vec![], error: None, closed: HashSet::new() }
    }

    fn reload(&mut self, ctx: &egui::Context) {
        let closed = std::mem::take(&mut self.closed);
        *self = Self::load(&self.title, self.top.clone(), self.args.clone(), ctx);
        self.closed = closed;
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, font_size: f32, cmds: &mut Vec<Command>) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    self.files = parse(&text);
                    self.error = (self.files.is_empty() && !text.trim().is_empty()).then(|| text.trim().to_owned());
                    self.rx = None;
                }
                Ok(Err(error)) => (self.error, self.rx) = (Some(error), None),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui::spinner(ui, rect.center(), 10.0, theme.text_dim);
                    return;
                }
                Err(_) => (self.error, self.rx) = (Some("Git stopped before it listed the changes.".into()), None),
            }
        }
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 28.0));
        ui.painter().rect_filled(bar, CornerRadius::ZERO, theme.panel);
        ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, theme.border));
        let (added, removed) = self.files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let summary = format!("{} files changed  +{added}  \u{2212}{removed}", self.files.len());
        ui.painter().text(bar.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, summary, FontId::proportional(12.0), theme.text_dim);
        let all_closed = !self.files.is_empty() && self.closed.len() == self.files.len();
        let buttons = [
            (icons::ARROW_CLOCKWISE, "Refresh"),
            (if all_closed { icons::ARROWS_OUT_LINE_VERTICAL } else { icons::ARROWS_IN_LINE_VERTICAL }, if all_closed { "Expand All" } else { "Collapse All" }),
        ];
        for (index, (icon, tip)) in buttons.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(bar.right() - 30.0 - 26.0 * index as f32, bar.top() + 3.0), vec2(22.0, 22.0));
            if ui.scope_builder(UiBuilder::new().max_rect(r), |ui| ui::icon_button(ui, theme, icon, 22.0, false, tip).clicked()).inner {
                match index {
                    0 => return self.reload(ui.ctx()),
                    _ if all_closed => self.closed.clear(),
                    _ => self.closed = (0..self.files.len()).collect(),
                }
            }
        }
        let body = rect.with_min_y(bar.bottom());
        if let Some(error) = &self.error {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, error, FontId::proportional(13.0), theme.text_dim);
            return;
        }
        if self.files.is_empty() {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, "No changes.", FontId::proportional(13.0), theme.text_dim);
            return;
        }
        let rows: Vec<(usize, Option<usize>)> = self
            .files
            .iter()
            .enumerate()
            .flat_map(|(i, f)| std::iter::once((i, None)).chain((0..if self.closed.contains(&i) { 0 } else { f.lines.len() }).map(move |j| (i, Some(j)))))
            .collect();
        let font = FontId::monospace(font_size);
        let (char_w, row_h) = ui.ctx().fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font) + 4.0));
        let digits = self.files.iter().flat_map(|f| &f.lines).filter_map(|l| l.new.max(l.old)).max().unwrap_or(1).to_string().len() as f32;
        let number_w = (digits + 1.0) * char_w;
        let mut toggle = None;
        ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ScrollArea::vertical().id_salt(("multi_diff", &self.title)).auto_shrink(false).show_rows(ui, row_h, rows.len(), |ui, range| {
                for &(i, line) in &rows[range] {
                    let file = &self.files[i];
                    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
                    let full = self.top.join(&file.path);
                    let Some(j) = line else {
                        painter.rect_filled(row, CornerRadius::ZERO, if resp.hovered() { theme.hover } else { theme.panel });
                        painter.hline(row.x_range(), row.top() + 0.5, Stroke::new(1.0, theme.border));
                        let caret = if self.closed.contains(&i) { icons::CARET_RIGHT } else { icons::CARET_DOWN };
                        painter.text(row.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, caret, FontId::proportional(12.0), theme.text_dim);
                        let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
                        let drawn = painter.text(row.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(13.0), theme.text);
                        let mut extra = dir.to_owned();
                        if let Some(from) = &file.from {
                            extra = format!("{extra}  renamed from {from}");
                        }
                        if file.binary {
                            extra = format!("{extra}  binary file");
                        }
                        painter.text(pos2(drawn.right() + 8.0, row.center().y), Align2::LEFT_CENTER, extra.trim(), FontId::proportional(11.5), theme.text_dim);
                        let counts = painter.text(row.right_center() - vec2(64.0, 0.0), Align2::RIGHT_CENTER, format!("\u{2212}{}", file.removed), FontId::proportional(12.0), theme.deleted);
                        painter.text(pos2(counts.left() - 8.0, row.center().y), Align2::RIGHT_CENTER, format!("+{}", file.added), FontId::proportional(12.0), theme.added);
                        let mut clicked_button = false;
                        for (index, (icon, tip)) in [(icons::FILE, "Open File"), (icons::GIT_DIFF, "Open Changes")].into_iter().enumerate() {
                            let r = Rect::from_center_size(pos2(row.right() - 16.0 - 24.0 * index as f32, row.center().y), vec2(22.0, 22.0));
                            if ui.scope_builder(UiBuilder::new().max_rect(r), |ui| ui::icon_button(ui, theme, icon, 20.0, false, tip).clicked()).inner {
                                clicked_button = true;
                                match index {
                                    0 => cmds.push(Command::OpenPath(full.clone())),
                                    _ => match single(&self.args, &file.path) {
                                        Some(args) => cmds.push(Command::OpenDiff(format!("{name} ({})", self.title), args)),
                                        None => cmds.push(Command::OpenPath(full.clone())),
                                    },
                                }
                            }
                        }
                        if resp.clicked() && !clicked_button {
                            toggle = Some(i);
                        }
                        continue;
                    };
                    let line = &file.lines[j];
                    let (fill, mark) = match line.kind {
                        '+' => (Some(ui::with_alpha(theme.added, 0.2)), theme.added),
                        '-' => (Some(ui::with_alpha(theme.deleted, 0.2)), theme.deleted),
                        '@' => (Some(ui::with_alpha(theme.text_dim, 0.08)), theme.text_dim),
                        _ => (None, theme.text_dim),
                    };
                    if let Some(fill) = fill {
                        painter.rect_filled(row, CornerRadius::ZERO, fill);
                    }
                    let number_font = FontId::monospace(font_size - 1.0);
                    let dim = ui::with_alpha(theme.text_dim, 0.75);
                    for (k, number) in [line.old, line.new].into_iter().enumerate() {
                        if let Some(n) = number {
                            painter.text(pos2(row.left() + 8.0 + number_w * (k + 1) as f32, row.center().y), Align2::RIGHT_CENTER, n, number_font.clone(), dim);
                        }
                    }
                    let x = row.left() + 16.0 + number_w * 2.0;
                    if line.kind == '@' {
                        painter.text(pos2(x, row.center().y), Align2::LEFT_CENTER, &line.text, font.clone(), theme.text_dim);
                        continue;
                    }
                    painter.text(pos2(x, row.center().y), Align2::LEFT_CENTER, line.kind, font.clone(), mark);
                    painter.text(pos2(x + char_w * 2.0, row.center().y), Align2::LEFT_CENTER, &line.text, font.clone(), theme.text);
                    if resp.double_clicked() {
                        let at = line.new.or(line.old).unwrap_or(1);
                        cmds.extend([Command::OpenPath(full), Command::GoTo(at, 1)]);
                    }
                }
            });
        });
        if let Some(i) = toggle
            && !self.closed.remove(&i)
        {
            self.closed.insert(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_multi_file_patch() {
        let text = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,3 +3,3 @@ fn a()\n keep\n-old\n+new\n keep\n\
diff --git a/b.png b/b.png\nBinary files a/b.png and b/b.png differ\n\
diff --git a/c.txt b/d.txt\nsimilarity index 90%\nrename from c.txt\nrename to d.txt\n@@ -10 +10 @@\n-x\n+y\n\\ No newline at end of file\n";
        let files = parse(text);
        assert_eq!(files.len(), 3);
        assert_eq!((files[0].path.as_str(), files[0].added, files[0].removed), ("src/a.rs", 1, 1));
        let lines: Vec<_> = files[0].lines.iter().map(|l| (l.kind, l.old, l.new)).collect();
        assert_eq!(lines, [('@', None, None), (' ', Some(3), Some(3)), ('-', Some(4), None), ('+', None, Some(4)), (' ', Some(5), Some(5))]);
        assert!(files[1].binary && files[1].lines.is_empty());
        assert_eq!((files[2].path.as_str(), files[2].from.as_deref(), files[2].lines.len()), ("d.txt", Some("c.txt"), 3));
    }

    #[test]
    fn opens_one_file_from_each_kind_of_patch() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(single(&args(&["diff", "HEAD"]), "a"), Some(args(&["diff", "HEAD", "--", "a"])));
        let show = args(&["show", "--format=", "--diff-merges=first-parent", "abc"]);
        assert_eq!(single(&show, "a"), Some(args(&["show", "--format=", "--diff-merges=first-parent", "abc", "--", "a"])));
        let stash = args(&["stash", "show", "--include-untracked", "--patch", "--no-color", "abc"]);
        assert_eq!(single(&stash, "a"), Some(args(&["diff", "abc^", "abc", "--", "a"])));
    }
}
