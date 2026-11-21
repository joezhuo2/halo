use super::buffer::{After, Document, Kind, Sel};
use super::diff;
use super::diff_view::hatch;
use super::highlight::Highlighter;
use super::merge::{self, Merge, Row, State};
use super::{Doc, Editors, MAX_COLS, Tab, job, notice};
use crate::command::{self, Command};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::text::{CCursor, TextFormat};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Layout, Painter, Rect, RichText, Stroke, Ui, UiBuilder, pos2, vec2};
use ropey::Rope;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const BAR: f32 = 28.0;
const HEAD: f32 = 22.0;

struct Areas {
    bar: Rect,
    top: Rect,
    panes: Vec<(usize, Rect, Rect)>,
    head: Rect,
    result: Rect,
}

fn areas(body: Rect, base: bool) -> Areas {
    let bar = Rect::from_min_size(body.min, vec2(body.width(), BAR));
    let split = (bar.bottom() + (body.bottom() - bar.bottom()) * 0.45).round();
    let top = Rect::from_min_max(pos2(body.left(), bar.bottom()), pos2(body.right(), split));
    let order: &[usize] = if base { &[0, 2, 1] } else { &[0, 1] };
    let w = top.width() / order.len() as f32;
    let panes = order
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            let r = Rect::from_x_y_ranges(top.left() + i as f32 * w..=top.left() + (i + 1) as f32 * w, top.y_range());
            (*pane, r.with_max_y(r.top() + HEAD), r.with_min_y(r.top() + HEAD))
        })
        .collect();
    let head = Rect::from_x_y_ranges(body.x_range(), split..=split + HEAD);
    Areas { bar, top, panes, head, result: body.with_min_y(head.bottom()) }
}

fn side_color(theme: &Theme, pane: usize) -> Color32 {
    [theme.accent, theme.added, theme.deleted][pane]
}

fn state_color(theme: &Theme, state: State) -> Color32 {
    match state {
        State::Base => theme.warning,
        State::Current => theme.added,
        State::Incoming => theme.accent,
        State::Both(_) => ui::mix(theme.added, theme.accent, 0.5),
        State::Edited => theme.modified,
    }
}

fn tip(cmd: &Command, ctx: &egui::Context) -> String {
    let alias = match cmd {
        Command::MergeNextConflict => Command::NextChange,
        Command::MergePrevConflict => Command::PrevChange,
        other => other.clone(),
    };
    match command::shortcut_text(ctx, &alias) {
        Some(keys) => format!("{} ({keys})", cmd.label()),
        None => cmd.label().to_owned(),
    }
}

pub(super) fn tint(painter: &Painter, theme: &Theme, m: &MergeTab, line: usize, band: Rect) {
    let k = m.ranges.partition_point(|r| r.end <= line);
    let Some(chunk) = m.model.chunks.get(k).filter(|c| c.kind == merge::Kind::Conflict && m.ranges[k].start <= line) else { return };
    let alpha = if chunk.state == State::Base { 0.16 } else { 0.07 };
    painter.rect_filled(band, CornerRadius::ZERO, ui::with_alpha(state_color(theme, chunk.state), alpha));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_pads(painter: &Painter, theme: &Theme, pads: &[(usize, u32)], starts: &[u32], (first, last): (usize, usize), top: f32, clip: Rect, line_h: f32) {
    for &(line, count) in pads.iter().filter(|p| p.0 >= first && p.0 <= last + 1) {
        let y = top + (starts[line.min(starts.len() - 1)] - count) as f32 * line_h;
        hatch(painter, Rect::from_min_size(pos2(clip.left(), y), vec2(clip.width(), count as f32 * line_h)), theme);
    }
}

pub(super) struct Stages {
    top: PathBuf,
    rel: String,
    texts: [String; 3],
    names: [String; 2],
}

fn stages(path: &Path) -> Result<Stages, String> {
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let out = crate::git::exec(&dir, &["rev-parse", "--show-toplevel", "--show-prefix"]).map_err(|_| format!("{name} is not in a Git repository."))?;
    let mut lines = out.lines();
    let top = crate::wsl::local(&dir, lines.next().unwrap_or_default().trim());
    let rel = format!("{}{name}", lines.next().unwrap_or_default().trim());
    let show = |stage: u8| crate::git::exec(&top, &["show", &format!(":{stage}:{rel}")]);
    let (base, current, incoming) = (show(1), show(2), show(3));
    if current.is_err() && incoming.is_err() {
        return Err(format!("{name} has no merge conflict in Git."));
    }
    let texts = [base, current, incoming].map(Result::unwrap_or_default);
    if texts.iter().any(|t| t.as_bytes().iter().take(8000).any(|b| *b == 0)) {
        return Err(format!("{name} is a binary file, so the merge editor cannot show it."));
    }
    let label = |rev: &str| {
        let hash = crate::git::exec(&top, &["rev-parse", "-q", "--verify", rev]).ok()?;
        let named = crate::git::exec(&top, &["name-rev", "--name-only", "--no-undefined", rev]).unwrap_or_default();
        let named = named.trim().trim_end_matches("^0").trim_end_matches("~0");
        Some(if named.is_empty() { hash.trim().chars().take(8).collect() } else { named.to_owned() })
    };
    let incoming = ["MERGE_HEAD", "REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"].into_iter().find_map(label).unwrap_or_default();
    let branch = crate::git::exec(&top, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default().trim().to_owned();
    let current = if branch.is_empty() || branch == "HEAD" { label("HEAD").unwrap_or_default() } else { branch };
    Ok(Stages { top, rel, texts, names: [incoming, current] })
}

enum Msg {
    Loaded(PathBuf, Result<Stages, String>),
    Staged(PathBuf, Result<String, String>),
}

pub(super) struct Merges {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    confirm: Option<u64>,
    at: Option<usize>,
    refresh: bool,
}

impl Default for Merges {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, confirm: None, at: None, refresh: false }
    }
}

struct Pane {
    rope: Rope,
    hl: Highlighter,
}

pub(super) struct MergeTab {
    pub title: String,
    path: PathBuf,
    top: PathBuf,
    rel: String,
    names: [String; 2],
    model: Merge,
    panes: [Pane; 3],
    marks: HashMap<(usize, usize), Vec<Range<usize>>>,
    synced: u64,
    ranges: Vec<Range<usize>>,
    rows: Vec<Row>,
    pub pads: Vec<(usize, u32)>,
    pub base: bool,
    pub scroll: f32,
    pub text_top: f32,
    pub line_h: f32,
}

impl MergeTab {
    fn new(path: &Path, stages: Stages) -> Self {
        let model = Merge::new(&stages.texts[0], &stages.texts[1], &stages.texts[2]);
        let pane = |lines: &[String]| {
            let rope = Rope::from_str(&lines.concat());
            Pane { hl: Highlighter::new(Some(path), &rope), rope }
        };
        let panes = [pane(&model.incoming), pane(&model.current), pane(&model.base)];
        let mut marks = HashMap::new();
        for k in model.conflicts() {
            let c = &model.chunks[k];
            let base = &model.base[c.base.clone()];
            for (side, lines, range) in [(0, &model.incoming, &c.incoming), (1, &model.current, &c.current)] {
                let changed = &lines[range.clone()];
                for h in diff::diff(base, changed) {
                    for (j, r) in h.right.clone().enumerate() {
                        let text = changed[r].trim_end_matches('\n');
                        let found = match base.get(h.left.start + j).filter(|_| j < h.left.len()) {
                            Some(old) if old.len() + text.len() < 4000 => diff::intra(old.trim_end_matches('\n'), text).1,
                            _ => std::iter::once(0..text.chars().count()).collect(),
                        };
                        marks.insert((side, range.start + r), found);
                    }
                }
            }
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Self {
            title: format!("Merging: {name}"),
            path: path.to_path_buf(),
            top: stages.top,
            rel: stages.rel,
            names: stages.names,
            model,
            panes,
            marks,
            synced: 0,
            ranges: vec![],
            rows: vec![],
            pads: vec![],
            base: false,
            scroll: 0.0,
            text_top: 0.0,
            line_h: 16.0,
        }
    }

    pub fn sync(&mut self, buf: &Document) {
        if self.synced != buf.version {
            self.ranges = self.model.sync(&buf.rope.to_string());
            (self.rows, self.pads) = self.model.align(&self.ranges, self.base);
            self.synced = buf.version;
        }
    }

    fn conflict_at(&self, line: usize) -> Option<usize> {
        let inside = |k: &usize| self.ranges[*k].start <= line && line < self.ranges[*k].end.max(self.ranges[*k].start + 1);
        self.model.conflicts().find(inside).or_else(|| self.model.conflicts().find(|k| self.ranges[*k].start >= line)).or_else(|| self.model.conflicts().last())
    }

    fn step(&self, line: usize, forward: bool) -> Option<usize> {
        let open: Vec<usize> = self.model.conflicts().filter(|k| self.model.chunks[*k].state == State::Base).collect();
        match forward {
            true => open.iter().find(|k| self.ranges[**k].start > line).or(open.first()).copied(),
            false => open.iter().rev().find(|k| self.ranges[**k].start < line).or(open.last()).copied(),
        }
    }

    fn set(&mut self, buf: &mut Document, sels: &mut Vec<Sel>, picks: &[(usize, State)]) {
        self.sync(buf);
        let total = diff::split(&buf.rope.to_string()).len();
        let mut edits = vec![];
        for &(k, state) in picks {
            let r = self.ranges[k].clone();
            edits.push((buf.line_start(r.start), buf.line_start(r.end), self.model.text(k, state, r.end < total)));
            self.model.chunks[k].state = state;
        }
        buf.apply(sels, edits, Kind::Other, After::Map);
    }
}

impl Editors {
    pub fn open_merge(&mut self, path: &Path, ctx: &egui::Context) {
        let (tx, ctx, path) = (self.merges.tx.clone(), ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let found = stages(&path);
            let _ = tx.send(Msg::Loaded(path, found));
            ctx.request_repaint();
        });
    }

    pub(super) fn take_merge_refresh(&mut self) -> bool {
        std::mem::take(&mut self.merges.refresh)
    }

    pub(super) fn poll_merges(&mut self) {
        while let Ok(msg) = self.merges.rx.try_recv() {
            match msg {
                Msg::Loaded(path, Ok(stages)) => self.place_merge(&path, stages),
                Msg::Staged(path, Ok(rel)) => {
                    while let Some((g, t)) = self.merge_tab_of(&path) {
                        self.close_tab(g, t);
                    }
                    self.merges.refresh = true;
                    self.banner = notice(format!("Merge completed, {rel} is staged."));
                }
                Msg::Loaded(_, Err(error)) | Msg::Staged(_, Err(error)) => self.banner = Some((error, None)),
            }
        }
    }

    fn merge_tab_of(&self, path: &Path) -> Option<(usize, usize)> {
        let found = |group: &super::Group| group.tabs.iter().position(|t| t.merge.as_ref().is_some_and(|m| m.path == path));
        self.groups.iter().enumerate().find_map(|(g, group)| Some((g, found(group)?)))
    }

    fn place_merge(&mut self, path: &Path, stages: Stages) {
        let id = match self.find_doc(path) {
            Some(id) => id,
            None => match Document::load(path) {
                Ok(buf) => self.insert_doc(buf),
                Err(error) => {
                    self.banner = Some((error, None));
                    return;
                }
            },
        };
        self.active = self.active.min(self.groups.len() - 1);
        let g = self.active;
        let t = match self.groups[g].tabs.iter().position(|t| t.doc == id && t.diff.is_none()) {
            Some(t) => t,
            None => {
                self.add_tab(id, vec![Sel::caret(0)]);
                self.groups[g].tabs.len() - 1
            }
        };
        let mut m = MergeTab::new(path, stages);
        let (Some(doc), tab) = (self.docs.get_mut(&id), &mut self.groups[g].tabs[t]) else { return };
        if !doc.buf.conflicts().is_empty() {
            let result = m.model.render();
            doc.buf.apply(&mut tab.sels, vec![(0, doc.buf.len(), result)], Kind::Other, After::Carets);
        }
        m.sync(&doc.buf);
        let first = m.step(0, true).or_else(|| m.model.conflicts().next()).map_or(0, |k| doc.buf.line_start(m.ranges[k].start));
        (tab.sels, tab.reveal, tab.preview, tab.merge) = (vec![Sel::caret(first)], Some(first), false, Some(Box::new(m)));
        (self.groups[g].active, self.want_focus) = (t, true);
    }

    pub(super) fn merge_frame(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, doc: &Doc, tab: &mut Tab, pending: &mut Vec<Command>) -> Rect {
        let Some(m) = tab.merge.as_mut() else { return body };
        m.sync(&doc.buf);
        let a = areas(body, m.base);
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let border = Stroke::new(1.0, theme.border);
        painter.rect_filled(a.bar, CornerRadius::ZERO, theme.panel);
        painter.hline(a.bar.x_range(), a.bar.bottom() - 0.5, border);
        let small = FontId::proportional(12.0);
        let titles = [("Incoming", m.names[0].as_str()), ("Current", m.names[1].as_str()), ("Base", "")];
        for (pane, head, _) in &a.panes {
            painter.rect_filled(*head, CornerRadius::ZERO, ui::mix(theme.panel, side_color(theme, *pane), 0.12));
            let (label, name) = titles[*pane];
            let x = painter.text(head.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, small.clone(), theme.text).right();
            painter.text(pos2(x + 8.0, head.center().y), Align2::LEFT_CENTER, name, small.clone(), theme.text_dim);
            if head.left() > a.top.left() {
                painter.vline(head.left(), a.top.y_range(), border);
            }
        }
        painter.rect_filled(a.head, CornerRadius::ZERO, theme.panel);
        painter.hline(a.head.x_range(), a.head.top() + 0.5, border);
        let x = painter.text(a.head.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, "Result", small.clone(), theme.text).right();
        painter.text(pos2(x + 8.0, a.head.center().y), Align2::LEFT_CENTER, &doc.buf.title, small, theme.text_dim);
        let status = match (m.model.conflicts().count(), m.model.remaining()) {
            (0, _) => "No conflicts".to_owned(),
            (_, 1) => "1 conflict remaining".to_owned(),
            (_, left) => format!("{left} conflicts remaining"),
        };
        let base_icon = if m.base { icons::EYE_SLASH } else { icons::EYE };
        ui.scope_builder(UiBuilder::new().max_rect(a.bar.shrink2(vec2(8.0, 0.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.button("Complete Merge").on_hover_text(tip(&Command::MergeComplete, &ctx)).clicked() {
                pending.push(Command::MergeComplete);
            }
            ui.add_space(6.0);
            ui.label(RichText::new(status).size(12.0).color(theme.text_dim));
            for (icon, cmd) in [(icons::ARROW_DOWN, Command::MergeNextConflict), (icons::ARROW_UP, Command::MergePrevConflict), (base_icon, Command::MergeToggleBase)] {
                if ui::icon_button(ui, theme, icon, 22.0, false, &tip(&cmd, &ctx)).clicked() {
                    pending.push(cmd);
                }
            }
            ui.add_space(6.0);
            for (label, cmd) in [("Accept All Current", Command::MergeAcceptAllCurrent), ("Accept All Incoming", Command::MergeAcceptAllIncoming)] {
                if ui.button(RichText::new(label).size(12.0)).clicked() {
                    pending.push(cmd);
                }
            }
        });
        a.result
    }

    pub(super) fn merge_after(&mut self, ui: &mut Ui, body: Rect, theme: &Theme, settings: &Settings, doc: &mut Doc, tab: &mut Tab) {
        let Some(m) = tab.merge.as_mut() else { return };
        let a = areas(body, m.base);
        let font = FontId::monospace(settings.font_size);
        let number_font = FontId::monospace(settings.font_size - 1.0);
        let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'm'));
        let (line_h, scroll) = (m.line_h, m.scroll);
        let first = (scroll / line_h).floor().max(0.0) as usize;
        let mut first_row = vec![None; m.model.chunks.len()];
        for (r, row) in m.rows.iter().enumerate() {
            first_row[row.chunk].get_or_insert(r);
        }
        for (pane, _, rect) in &a.panes {
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            painter.rect_filled(*rect, CornerRadius::ZERO, theme.bg);
            let shown = &m.rows[first.min(m.rows.len())..(first + (rect.height() / line_h) as usize + 2).min(m.rows.len())];
            let lines = shown.iter().filter_map(|r| r.lines[*pane]);
            let range = lines.clone().min().unwrap_or(0)..lines.max().map_or(0, |l| l + 1);
            let p = &mut m.panes[*pane];
            if !p.hl.ensure(&p.rope, range, theme, Duration::from_millis(4)) {
                ui.ctx().request_repaint();
            }
            let side: &[String] = [&m.model.incoming, &m.model.current, &m.model.base][*pane];
            let gutter = (side.len().to_string().len().max(3) as f32 + 2.0) * char_w + 14.0;
            for (i, row) in shown.iter().enumerate() {
                let band = Rect::from_min_size(pos2(rect.left(), rect.top() + (first + i) as f32 * line_h - scroll), vec2(rect.width(), line_h));
                let Some(line) = row.lines[*pane] else {
                    hatch(&painter, band, theme);
                    continue;
                };
                let chunk = &m.model.chunks[row.chunk];
                let text: String = side[line].trim_end_matches('\n').chars().take(MAX_COLS).collect();
                let galley = painter.layout_job(job(&text, 0..text.len(), p.hl.get(line), &font, theme.text, &[], &[], &TextFormat::default()));
                let alpha = match chunk.kind {
                    merge::Kind::Conflict if chunk.state == State::Base => Some(0.14),
                    merge::Kind::Conflict => Some(0.06),
                    merge::Kind::Incoming if *pane == 0 => Some(0.06),
                    merge::Kind::Current if *pane == 1 => Some(0.06),
                    _ => None,
                };
                if let Some(alpha) = alpha {
                    let color = side_color(theme, *pane);
                    painter.rect_filled(band, CornerRadius::ZERO, ui::with_alpha(color, alpha));
                    for r in m.marks.get(&(*pane, line)).into_iter().flatten() {
                        let at = |col: usize| rect.left() + gutter + galley.pos_from_cursor(CCursor::new(col)).min.x;
                        painter.rect_filled(Rect::from_x_y_ranges(at(r.start)..=at(r.end), band.y_range()), CornerRadius::ZERO, ui::with_alpha(color, alpha * 2.2));
                    }
                }
                let number = ui::with_alpha(theme.text_dim, 0.75);
                painter.text(pos2(rect.left() + gutter - 10.0 - char_w, band.center().y), Align2::RIGHT_CENTER, line + 1, number_font.clone(), number);
                painter.galley(pos2(rect.left() + gutter, band.top() + (line_h - galley.size().y) / 2.0), galley, theme.text);
            }
        }
        let mut chosen = None;
        for k in m.model.conflicts() {
            let (Some(r0), state) = (first_row[k], m.model.chunks[k].state) else { continue };
            for (pane, _, rect) in a.panes.iter().filter(|p| p.0 < 2) {
                let y = rect.top() + r0 as f32 * line_h - scroll;
                if y < rect.top() - 1.0 || y + line_h > rect.bottom() {
                    continue;
                }
                let current = *pane == 1;
                let checked = matches!((state, current), (State::Current | State::Both(_), true) | (State::Incoming | State::Both(_), false));
                let r = Rect::from_min_size(pos2(rect.right() - 236.0, y + 1.0), vec2(230.0, line_h - 2.0));
                ui.painter().rect_filled(r, CornerRadius::same(4), theme.raised);
                ui.scope_builder(UiBuilder::new().max_rect(r.shrink2(vec2(6.0, 0.0))).layout(Layout::right_to_left(Align::Center)).id_salt(("merge_pick", k, *pane)), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if ui.small_button(RichText::new("Accept Both").size(12.0)).clicked() {
                        chosen = Some((k, State::Both(current)));
                    }
                    let mut on = checked;
                    if ui.checkbox(&mut on, RichText::new(if current { "Accept Current" } else { "Accept Incoming" }).size(12.0)).clicked() {
                        chosen = Some((k, merge::toggle(state, current)));
                    }
                });
            }
            let y = m.text_top + r0 as f32 * line_h - scroll;
            if state != State::Base && y >= a.result.top() - 1.0 && y + line_h <= a.result.bottom() {
                let r = Rect::from_min_size(pos2(a.result.right() - 124.0, y + 1.0), vec2(104.0, line_h - 2.0));
                if ui.put(r, egui::Button::new(RichText::new("Reset to Base").size(12.0)).small()).clicked() {
                    chosen = Some((k, State::Base));
                }
            }
        }
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0.0 && ui.rect_contains_pointer(a.top) {
            tab.set_scroll = Some((scroll - delta).max(0.0));
        }
        if let Some((k, state)) = chosen {
            m.set(&mut doc.buf, &mut tab.sels, &[(k, state)]);
            self.want_focus = true;
        }
    }

    pub(super) fn merge_exec(&mut self, cmd: &Command, ctx: &egui::Context) -> bool {
        use Command::*;
        if *cmd == MergeOpen {
            match self.active_path() {
                Some(path) => self.open_merge(&path, ctx),
                None => self.banner = notice("Save the file to open it in the merge editor.".to_owned()),
            }
            return true;
        }
        let ours = matches!(
            cmd,
            MergeAcceptIncoming | MergeAcceptCurrent | MergeAcceptBoth | MergeAcceptAllIncoming | MergeAcceptAllCurrent | MergeNextConflict
                | MergePrevConflict | MergeToggleBase | MergeComplete
        );
        if !ours && !matches!(cmd, NextChange | PrevChange) {
            return false;
        }
        let at = self.merges.at.take();
        let Some((g, t)) = self.active_tab().filter(|(g, t)| self.groups[*g].tabs[*t].merge.is_some()) else {
            if ours {
                self.banner = notice("Open a conflicted file in the merge editor first.".to_owned());
            }
            return ours;
        };
        let tab = &mut self.groups[g].tabs[t];
        let doc_id = tab.doc;
        let (Some(doc), Some(m)) = (self.docs.get_mut(&doc_id), tab.merge.as_mut()) else { return true };
        let buf = &mut doc.buf;
        buf.clamp(&mut tab.sels);
        m.sync(buf);
        let line = buf.line_of(tab.sels[0].head);
        let pick = match cmd {
            MergeAcceptIncoming => Some(State::Incoming),
            MergeAcceptCurrent => Some(State::Current),
            MergeAcceptBoth => Some(State::Both(true)),
            _ => None,
        };
        let mut finish = false;
        match cmd {
            MergeToggleBase => (m.base, m.synced) = (!m.base, 0),
            MergeComplete => match m.model.remaining() {
                0 => finish = true,
                _ => self.merges.confirm = Some(doc_id),
            },
            MergeAcceptAllIncoming | MergeAcceptAllCurrent => {
                let state = if *cmd == MergeAcceptAllIncoming { State::Incoming } else { State::Current };
                let picks: Vec<(usize, State)> = m.model.conflicts().map(|k| (k, state)).collect();
                m.set(buf, &mut tab.sels, &picks);
            }
            _ if pick.is_some() => match at.or_else(|| m.conflict_at(line)) {
                Some(k) => m.set(buf, &mut tab.sels, &[(k, pick.unwrap_or(State::Base))]),
                None => self.banner = notice("There are no conflicts in this file.".to_owned()),
            },
            _ => match m.step(line, matches!(cmd, MergeNextConflict | NextChange)) {
                Some(k) => {
                    let pos = buf.line_start(m.ranges[k].start);
                    (tab.sels, tab.reveal) = (vec![Sel::caret(pos)], Some(pos));
                }
                None => self.banner = notice("No unresolved conflicts.".to_owned()),
            },
        }
        self.want_focus = true;
        if finish {
            self.complete_merge(doc_id, ctx);
        }
        true
    }

    fn complete_merge(&mut self, doc_id: u64, ctx: &egui::Context) {
        let tabs = self.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.doc == doc_id);
        let Some((top, rel, path)) = tabs.filter_map(|t| t.merge.as_ref()).map(|m| (m.top.clone(), m.rel.clone(), m.path.clone())).next() else { return };
        if !self.save_doc(doc_id, false) {
            return;
        }
        let (tx, ctx) = (self.merges.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let staged = crate::git::exec(&top, &["add", "--", &rel]).map(|_| rel.clone()).map_err(|e| format!("Git could not stage {rel}: {e}"));
            let _ = tx.send(Msg::Staged(path, staged));
            ctx.request_repaint();
        });
    }

    pub(super) fn merge_dialog(&mut self, ctx: &egui::Context) {
        let Some(doc_id) = self.merges.confirm else { return };
        let tabs = self.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.doc == doc_id);
        let Some(left) = tabs.filter_map(|t| t.merge.as_ref()).map(|m| m.model.remaining()).next() else {
            self.merges.confirm = None;
            return;
        };
        let mut choice = None;
        let modal = ui::modal(ctx, "merge_complete").show(ctx, |ui| {
            ui::outline(ui);
            ui.set_width(360.0);
            ui.heading("Unresolved conflicts");
            let count = if left == 1 { "1 conflict is".to_owned() } else { format!("{left} conflicts are") };
            ui.label(format!("{count} still unresolved. Complete the merge and stage the file anyway?"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (i, label) in ["Complete Merge", "Cancel"].into_iter().enumerate() {
                    if ui.button(label).clicked() {
                        choice = Some(i);
                    }
                }
            });
        });
        if modal.should_close() {
            choice = Some(1);
        }
        if let Some(choice) = choice {
            self.merges.confirm = None;
            if choice == 0 {
                self.complete_merge(doc_id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Event;

    fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<Event>) -> Vec<Command> {
        let (theme, git, settings) = (Theme::dark(), crate::git::Git::new(), Settings::default());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))), events, focused: true, ..Default::default() };
        let mut cmds = vec![];
        let mut out = ctx.run_ui(input, |ui| editors.ui(ui, &theme, &settings, &git, &mut cmds));
        out.textures_delta.clear();
        cmds
    }

    fn settle(ctx: &egui::Context, editors: &mut Editors, what: &str, done: impl Fn(&Editors) -> bool) -> Vec<Command> {
        let mut cmds = vec![];
        for _ in 0..800 {
            cmds.extend(frame(ctx, editors, vec![]));
            if done(editors) {
                return cmds;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}: {:?}", editors.banner);
    }

    fn conflict_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halo-merge-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| crate::git::exec(&dir, args);
        for args in [&["init", "-q", "-b", "trunk"][..], &["config", "user.email", "t@example.com"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"]] {
            run(args).unwrap();
        }
        let commit = |text: &str, message: &str| {
            std::fs::write(dir.join("a.txt"), text).unwrap();
            run(&["commit", "-q", "-am", message]).unwrap();
        };
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        run(&["add", "."]).unwrap();
        run(&["commit", "-q", "-m", "base"]).unwrap();
        run(&["checkout", "-q", "-b", "feature"]).unwrap();
        commit("one\nTWO theirs\nand more\nthree\nfour\nFIVE\n", "theirs");
        run(&["checkout", "-q", "trunk"]).unwrap();
        commit("one\nTWO ours\nthree\nfour\nfive\n", "ours");
        assert!(run(&["merge", "-q", "feature"]).is_err());
        dir
    }

    #[test]
    fn loads_the_three_stages_and_branch_names_of_a_real_conflict() {
        let dir = conflict_repo("stages");
        let found = stages(&dir.join("a.txt")).unwrap();
        assert_eq!(found.rel, "a.txt");
        assert_eq!(found.texts, ["one\ntwo\nthree\nfour\nfive\n", "one\nTWO ours\nthree\nfour\nfive\n", "one\nTWO theirs\nand more\nthree\nfour\nFIVE\n"].map(str::to_owned));
        assert_eq!(found.names, ["feature".to_owned(), "trunk".to_owned()]);
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        assert!(stages(&dir.join("b.txt")).err().is_some_and(|e| e.contains("no merge conflict")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn completes_a_merge_by_saving_and_staging_the_result() {
        let dir = conflict_repo("complete");
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        let file = dir.join("a.txt");
        editors.open(&file);
        let marked = std::fs::read_to_string(&file).unwrap();
        assert!(marked.contains("<<<<<<<"));
        editors.open_merge(&file, &ctx);
        let wait = |editors: &mut Editors, done: &dyn Fn(&Editors) -> bool| {
            for _ in 0..500 {
                editors.poll_merges();
                if done(editors) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out: {:?}", editors.banner);
        };
        wait(&mut editors, &|e| e.groups[0].tabs[0].merge.is_some());
        assert_eq!(editors.groups[0].tabs.len(), 1);
        let text = |e: &Editors| e.docs[&e.groups[0].tabs[0].doc].buf.rope.to_string();
        assert_eq!(text(&editors), "one\ntwo\nthree\nfour\nFIVE\n");
        editors.exec(&Command::Undo, &settings, &ctx);
        assert_eq!(text(&editors), marked.replace("\r\n", "\n"));
        editors.exec(&Command::Redo, &settings, &ctx);
        editors.exec(&Command::MergeComplete, &settings, &ctx);
        assert_eq!(editors.merges.confirm, Some(editors.groups[0].tabs[0].doc));
        editors.merges.confirm = None;
        editors.exec(&Command::MergeAcceptCurrent, &settings, &ctx);
        assert_eq!(text(&editors), "one\nTWO ours\nthree\nfour\nFIVE\n");
        editors.exec(&Command::MergeComplete, &settings, &ctx);
        wait(&mut editors, &|e| e.groups[0].tabs.is_empty());
        assert!(editors.take_merge_refresh());
        assert_eq!(crate::git::exec(&dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap(), "");
        assert_eq!(crate::git::exec(&dir, &["show", ":a.txt"]).unwrap(), "one\nTWO ours\nthree\nfour\nFIVE\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_conflict_in_the_merge_editor_tab_and_completes() {
        let dir = conflict_repo("editor");
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let mut editors = Editors::new();
        editors.open_merge(&dir.join("a.txt"), &ctx);
        settle(&ctx, &mut editors, "the merge tab", |e| e.groups[0].tabs.first().is_some_and(|t| t.merge.is_some()));
        frame(&ctx, &mut editors, vec![]);
        let text = |e: &Editors| e.docs[&e.groups[0].tabs[0].doc].buf.rope.to_string();
        let tab = &editors.groups[0].tabs[0];
        let m = tab.merge.as_ref().unwrap();
        assert_eq!((m.title.as_str(), m.names.clone(), m.model.remaining()), ("Merging: a.txt", ["feature".to_owned(), "trunk".to_owned()], 1));
        assert_eq!((tab.rows.1.clone(), m.rows.len(), m.pads.clone()), (vec![0, 1, 3, 4, 5, 6, 7], 6, vec![(2, 1)]));
        assert_eq!(editors.docs[&tab.doc].buf.line_of(tab.sels[0].head), 1);
        assert_eq!(text(&editors), "one\ntwo\nthree\nfour\nFIVE\n");

        editors.exec(&Command::MergeToggleBase, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert!(editors.groups[0].tabs[0].merge.as_ref().is_some_and(|m| m.base && m.rows[1].lines == [Some(1), Some(1), Some(1), Some(1)]));
        editors.exec(&Command::MergeAcceptIncoming, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(text(&editors), "one\nTWO theirs\nand more\nthree\nfour\nFIVE\n");
        let tab = &editors.groups[0].tabs[0];
        assert_eq!((tab.rows.1.clone(), tab.merge.as_ref().unwrap().model.remaining()), (vec![0, 1, 2, 3, 4, 5, 6, 7], 0));

        editors.exec(&Command::GoTo(3, 1), &settings, &ctx);
        frame(&ctx, &mut editors, vec![Event::Text("X".into())]);
        frame(&ctx, &mut editors, vec![]);
        let resolved = "one\nTWO theirs\nXand more\nthree\nfour\nFIVE\n";
        assert_eq!(text(&editors), resolved);
        let m = editors.groups[0].tabs[0].merge.as_ref().unwrap();
        assert_eq!((m.model.chunks[1].state, m.model.remaining()), (State::Edited, 0));
        editors.exec(&Command::NextChange, &settings, &ctx);
        assert!(editors.banner.as_ref().is_some_and(|b| b.0 == "No unresolved conflicts."));

        editors.exec(&Command::MergeComplete, &settings, &ctx);
        let cmds = settle(&ctx, &mut editors, "the staged merge", |e| e.groups[0].tabs.is_empty());
        let cmds = [cmds, frame(&ctx, &mut editors, vec![])].concat();
        assert!(cmds.contains(&Command::RefreshExplorer), "{cmds:?}");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), resolved);
        assert_eq!(crate::git::exec(&dir, &["show", ":a.txt"]).unwrap(), resolved);
        assert_eq!(crate::git::exec(&dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
