use super::layout::{self, Dir, Node, line};
use super::{Editors, Group, Sel, TAB_H};
use crate::command::Command;
use crate::git::Git;
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{
    self, Align2, CornerRadius, CursorIcon, FontId, Frame, Id, Key, PointerButton, Rect, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MIN_GROUP: f32 = 120.0;

pub(super) struct Win {
    pub node: Node,
    pub id: u64,
    start: Option<Rect>,
    rect: Option<Rect>,
}

pub(super) struct Grid {
    pub wins: Vec<Win>,
    pub maximized: Option<usize>,
    pub corner: Option<usize>,
    pub os_focus: bool,
    next: u64,
}

impl Default for Grid {
    fn default() -> Self {
        Self { wins: vec![Win { node: Node::Leaf(0), id: 0, start: None, rect: None }], maximized: None, corner: None, os_focus: true, next: 1 }
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    wins: Vec<(Node, Option<[f32; 4]>)>,
    groups: Vec<Vec<(PathBuf, bool)>>,
    active: usize,
    #[serde(default)]
    locked: Vec<usize>,
}

fn direction(cmd: &Command) -> Dir {
    use Command::*;
    match cmd {
        SplitEditorLeft | FocusGroupLeft | MoveEditorLeft => Dir::Left,
        SplitEditorUp | FocusGroupUp | MoveEditorUp => Dir::Up,
        SplitEditorDown | FocusGroupDown | MoveEditorDown => Dir::Down,
        _ => Dir::Right,
    }
}

fn preset(cmd: &Command) -> Option<Node> {
    use Command::*;
    use Node::Leaf;
    Some(match cmd {
        LayoutSingle => Leaf(0),
        LayoutTwoColumns => line(true, [Leaf(0), Leaf(1)]),
        LayoutThreeColumns => line(true, [Leaf(0), Leaf(1), Leaf(2)]),
        LayoutTwoRows => line(false, [Leaf(0), Leaf(1)]),
        LayoutThreeRows => line(false, [Leaf(0), Leaf(1), Leaf(2)]),
        LayoutGrid => line(false, [line(true, [Leaf(0), Leaf(1)]), line(true, [Leaf(2), Leaf(3)])]),
        LayoutTwoRowsRight => line(true, [Leaf(0), line(false, [Leaf(1), Leaf(2)])]),
        LayoutTwoColumnsBottom => line(false, [Leaf(0), line(true, [Leaf(1), Leaf(2)])]),
        _ => return None,
    })
}

impl Editors {
    pub(super) fn win_of(&self, g: usize) -> usize {
        self.grid.wins.iter().position(|w| w.node.contains(g)).unwrap_or(0)
    }

    pub(super) fn split_group(&mut self, g: usize, dir: Dir) -> usize {
        let new = self.groups.len();
        self.groups.push(Group::default());
        let w = self.win_of(g);
        let node = &mut self.grid.wins[w].node;
        if !node.split(g, new, dir) {
            let first = node.leaves()[0];
            node.split(first, new, dir);
        }
        self.grid.maximized = None;
        new
    }

    pub(super) fn avoid_locked(&mut self) {
        let g = self.active.min(self.groups.len() - 1);
        if !self.groups[g].locked {
            return;
        }
        let free = self.grid.wins[self.win_of(g)].node.leaves().into_iter().find(|&l| !self.groups[l].locked);
        self.active = free.unwrap_or_else(|| self.split_group(g, Dir::Right));
    }

    pub(super) fn neighbor(&self, g: usize, dir: Dir) -> Option<usize> {
        self.grid.wins[self.win_of(g)].node.neighbor(g, dir)
    }

    pub(super) fn remove_group(&mut self, g: usize) {
        let owner = self.grid.wins[self.win_of(g)].id;
        self.groups.remove(g);
        let shift = |x: usize| if x > g { x - 1 } else { x };
        for (i, mut win) in std::mem::take(&mut self.grid.wins).into_iter().enumerate() {
            match (std::mem::replace(&mut win.node, Node::Leaf(0)).without(g), i) {
                (Some(node), _) => win.node = node,
                (None, 0) => {
                    win.node = Node::Leaf(self.groups.len());
                    self.groups.push(Group::default());
                }
                (None, _) => continue,
            }
            self.grid.wins.push(win);
        }
        self.active = match self.active == g {
            true => self.grid.wins.iter().find(|w| w.id == owner).unwrap_or(&self.grid.wins[0]).node.leaves()[0],
            false => shift(self.active),
        };
        self.grid.maximized = self.grid.maximized.filter(|m| *m != g).map(shift);
        self.switcher = self.switcher.filter(|s| s.0 != g).map(|(a, b)| (shift(a), b));
        self.closing = self.closing.filter(|c| c.0 != g).map(|(a, b)| (shift(a), b));
        self.previews.retain(|p| p.0 != g);
        self.previews.iter_mut().for_each(|p| p.0 = shift(p.0));
        if self.drag.as_ref().is_some_and(|d| d.group == g) {
            self.drag = None;
        }
        if let Some(drag) = &mut self.drag {
            drag.group = shift(drag.group);
        }
    }

    pub fn close_empty_groups(&mut self) {
        if self.closing.is_some() {
            return;
        }
        for g in (0..self.groups.len()).rev() {
            let lone = self.grid.wins[0].node == Node::Leaf(g);
            if self.groups[g].tabs.is_empty() && self.groups[g].used && !lone {
                self.remove_group(g);
                self.want_focus = true;
            }
        }
    }

    fn copy_tab(&mut self, g: usize, t: usize, to: usize) {
        let tab = &self.groups[g].tabs[t];
        if self.terminals.contains_key(&tab.doc) {
            return self.place_tab((g, t), to, usize::MAX);
        }
        let (doc, sels, md) = (tab.doc, tab.sels.clone(), tab.md.is_some());
        self.active = to;
        self.add_tab(doc, sels);
        if md {
            self.groups[to].tabs.last_mut().into_iter().for_each(|tab| tab.md = Some(Default::default()));
        }
    }

    fn apply_layout(&mut self, node: Node) {
        let want = node.leaves().len();
        let have = self.grid.wins[0].node.leaves();
        if let Some(&keep) = have.get(want - 1) {
            for &extra in have[want..].iter().rev() {
                while !self.groups[extra].tabs.is_empty() {
                    self.place_tab((extra, 0), keep, usize::MAX);
                }
            }
            let mut extra = have[want..].to_vec();
            extra.sort_unstable();
            extra.into_iter().rev().for_each(|g| self.remove_group(g));
        }
        let mut ids = self.grid.wins[0].node.leaves();
        while ids.len() < want {
            ids.push(self.groups.len());
            self.groups.push(Group::default());
        }
        self.grid.wins[0].node = node.map(&|i| ids[i]);
        self.grid.maximized = None;
    }

    fn close_group(&mut self, g: usize) {
        for t in (0..self.groups[g].tabs.len()).rev() {
            let doc = self.groups[g].tabs[t].doc;
            if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                self.close_tab(g, t);
            }
        }
        match self.groups[g].tabs.is_empty() {
            true if self.grid.wins[0].node != Node::Leaf(g) => self.remove_group(g),
            true => {}
            false => self.request_close(g, 0),
        }
    }

    pub(super) fn move_to_window(&mut self, g: usize, t: usize, copy: bool, at: Option<Rect>) {
        let new = self.groups.len();
        self.groups.push(Group::default());
        let id = self.grid.next;
        self.grid.next += 1;
        self.grid.wins.push(Win { node: Node::Leaf(new), id, start: at, rect: at });
        match copy {
            true => self.copy_tab(g, t, new),
            false => self.place_tab((g, t), new, 0),
        }
        self.active = new;
        self.want_focus = true;
    }

    pub(super) fn grid_exec(&mut self, cmd: &Command) -> bool {
        use Command::*;
        let g = self.active.min(self.groups.len() - 1);
        let w = self.win_of(g);
        let dir = direction(cmd);
        match cmd {
            SplitEditor | SplitEditorDown | SplitEditorLeft | SplitEditorUp => {
                let to = self.split_group(g, dir);
                match self.active_tab() {
                    Some((g, t)) => self.copy_tab(g, t, to),
                    None => self.active = to,
                }
            }
            ToggleMaximizeGroup => {
                self.grid.maximized = match self.grid.maximized {
                    Some(_) => None,
                    None => (self.grid.wins[w].node.leaves().len() > 1).then_some(g),
                }
            }
            ResetGroupSizes => self.grid.wins[w].node.even(None),
            ToggleEditorGroupLock => self.groups[g].locked ^= true,
            FocusGroupLeft | FocusGroupRight | FocusGroupUp | FocusGroupDown => {
                self.grid.maximized = None;
                if let Some(n) = self.neighbor(g, dir) {
                    self.active = n;
                }
            }
            MoveEditorLeft | MoveEditorRight | MoveEditorUp | MoveEditorDown => {
                let Some((g, t)) = self.active_tab() else { return true };
                let to = self.neighbor(g, dir).unwrap_or_else(|| self.split_group(g, dir));
                self.place_tab((g, t), to, usize::MAX);
            }
            CloseGroup => self.close_group(g),
            MoveEditorToNewWindow | CopyEditorToNewWindow => {
                if let Some((g, t)) = self.active_tab() {
                    self.move_to_window(g, t, *cmd == CopyEditorToNewWindow, None);
                }
            }
            MoveEditorToMainWindow => {
                if let Some((g, t)) = self.active_tab().filter(|(g, _)| self.win_of(*g) != 0) {
                    let to = self.grid.wins[0].node.leaves()[0];
                    self.place_tab((g, t), to, usize::MAX);
                }
            }
            cmd => match preset(cmd) {
                Some(node) => self.apply_layout(node),
                None => return false,
            },
        }
        self.want_focus = true;
        true
    }

    pub(super) fn run_pending(&mut self, pending: Vec<Command>, settings: &Settings, ctx: &egui::Context, cmds: &mut Vec<Command>) {
        let mut outer = false;
        for cmd in pending {
            outer |= matches!(cmd, Command::ToggleAi | Command::OpenPath(_));
            if outer {
                cmds.push(cmd);
            } else {
                self.exec(&cmd, settings, ctx);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn area_ui(&mut self, ui: &mut Ui, w: usize, theme: &Theme, settings: &Settings, git: &Git, pending: &mut Vec<Command>) {
        let rect = ui.available_rect_before_wrap();
        self.grid.os_focus = self.grid.wins.len() == 1 || ui.input(|i| i.viewport().focused.unwrap_or(true));
        let here = self.drag.as_ref().is_some_and(|d| self.win_of(d.group) == w);
        let parked = if here { None } else { self.drag.take() };
        if here {
            self.drop = None;
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                self.drag = None;
            }
        }
        let node = &self.grid.wins[w].node;
        let maximized = self.grid.maximized.filter(|g| node.contains(*g));
        let (rects, splitters) = match maximized {
            Some(g) => (vec![(g, rect)], vec![]),
            None => (node.rects(rect), node.splitters(rect)),
        };
        if w == 0 {
            self.grid.corner = rects.iter().find(|(_, r)| r.right() >= rect.right() - 0.5 && r.top() <= rect.top() + 0.5).map(|r| r.0);
        }
        let count = self.groups.len();
        for &(g, area) in &rects {
            let fade = ui::fade_in(ui, Id::new(("editor_group_fade", g, count)));
            ui.scope_builder(UiBuilder::new().max_rect(area).id_salt(("editor_group", g)), |ui| {
                ui.set_clip_rect(area.intersect(ui.clip_rect()));
                if count > 1 && g + 1 == count {
                    ui.set_opacity(fade);
                }
                self.group_ui(ui, g, area, theme, settings, git, pending);
            });
        }
        let id = self.grid.wins[w].id;
        for s in splitters {
            self.splitter(ui, w, id, s, theme);
        }
        ui.allocate_rect(rect, Sense::hover());
        if self.switcher.is_some_and(|(g, _)| self.win_of(g) == w) {
            self.switcher_ui(ui, &rects, rect, theme);
        }
        self.drag_ui(ui, &rects, theme);
        if parked.is_some() {
            self.drag = parked;
        }
        if self.closing.is_some_and(|(g, _)| self.win_of(g) == w) {
            self.dialogs(ui.ctx());
        }
    }

    fn splitter(&mut self, ui: &mut Ui, w: usize, win: u64, s: layout::Splitter, theme: &Theme) {
        let hit = s.rect.expand2(if s.row { vec2(3.0, 0.0) } else { vec2(0.0, 3.0) });
        let id = Id::new(("editor_splitter", win, &s.path, s.index));
        let resp = ui.interact(hit, id, Sense::click_and_drag());
        let active = resp.hovered() || resp.dragged();
        let h = ui::anim(ui, id.with("hover"), active);
        if active {
            ui.ctx().set_cursor_icon(if s.row { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
        }
        let node = &mut self.grid.wins[w].node;
        if resp.double_clicked() {
            node.even(Some(&s.path));
        } else if resp.dragged() {
            let d = resp.drag_delta();
            node.resize(&s.path, s.index, if s.row { d.x } else { d.y } / s.extent, MIN_GROUP / s.extent);
        }
        let stroke = Stroke::new(1.0 + h, ui::mix(theme.border, theme.accent, h));
        match s.row {
            true => ui.painter().vline(s.rect.left(), s.rect.y_range(), stroke),
            false => ui.painter().hline(s.rect.x_range(), s.rect.top(), stroke),
        };
    }

    fn switcher_ui(&mut self, ui: &mut Ui, rects: &[(usize, Rect)], rect: Rect, theme: &Theme) {
        let Some((g, at)) = self.switcher else { return };
        let order = self.recent_order(g);
        if order.is_empty() || !ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
            let doc = order.get(at.min(order.len().saturating_sub(1)));
            if let Some(t) = doc.and_then(|doc| self.groups[g].tabs.iter().position(|tab| tab.doc == *doc)) {
                self.groups[g].active = t;
                self.active = g;
            }
            self.switcher = None;
            self.want_focus = true;
            return;
        }
        let area = rects.iter().find(|r| r.0 == g).map_or(rect, |r| r.1);
        let top = area.center_top() + vec2(0.0, 70.0);
        egui::Area::new(Id::new("editor_switcher")).order(egui::Order::Foreground).pivot(Align2::CENTER_TOP).fixed_pos(top).show(ui.ctx(), |ui| {
            Frame::new().fill(theme.raised).stroke(Stroke::new(1.0, theme.accent)).corner_radius(CornerRadius::same(6)).inner_margin(4.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (index, doc) in order.iter().enumerate() {
                    let (r, row) = ui.allocate_exact_size(vec2(320.0, 24.0), Sense::hover());
                    ui::row_bg(ui, theme, r, row.id, false, index == at);
                    let title = self.docs.get(doc).map(|d| d.buf.title.clone()).or_else(|| self.terminals.get(doc).map(|t| t.title())).unwrap_or_default();
                    ui.painter().with_clip_rect(r).text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(13.0), theme.text);
                }
            });
        });
    }

    fn drag_ui(&mut self, ui: &mut Ui, rects: &[(usize, Rect)], theme: &Theme) {
        let Some(drag) = &self.drag else { return };
        let ctx = ui.ctx().clone();
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        let pos = ctx.pointer_latest_pos();
        if let Some(pos) = pos {
            let strip = rects.iter().map(|r| r.1).find(|r| r.contains(pos) && pos.y < r.top() + TAB_H);
            let mut origin = pos - drag.grab;
            let mut painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("editor_tab_drag")));
            match strip {
                Some(strip) => origin.y = strip.top(),
                None => painter.set_opacity(0.78),
            }
            let tab = Rect::from_min_size(origin, vec2(drag.face.width(), TAB_H));
            painter.rect(tab, CornerRadius::same(3), theme.raised, Stroke::new(1.0, theme.border), StrokeKind::Inside);
            drag.face.paint(&painter, tab, theme, 1.0, 0.0, None);
        }
        if ctx.input(|i| i.pointer.primary_down()) {
            return;
        }
        let (group, doc) = (drag.group, drag.doc);
        let source = self.groups.get(group).and_then(|group| group.tabs.iter().position(|t| t.doc == doc));
        let released = ctx.input(|i| i.pointer.button_released(PointerButton::Primary));
        let outside = pos.is_none_or(|p| !ctx.content_rect().contains(p));
        match (source, self.drop.take()) {
            (Some(t), Some(at)) if released => self.drop_tab((group, t), at),
            (Some(t), None) if released && outside => {
                let window = ctx.input(|i| i.viewport().inner_rect);
                let at = window.zip(pos).map(|(w, p)| Rect::from_min_size(w.min + p.to_vec2() - vec2(40.0, 16.0), w.size().min(vec2(900.0, 640.0))));
                self.move_to_window(group, t, false, at);
            }
            _ => {}
        }
        self.drag = None;
        ctx.request_repaint();
    }

    pub fn floating(&self) -> Vec<(u64, ViewportId, ViewportBuilder)> {
        self.grid.wins.iter().skip(1).map(|win| {
            let leaves = win.node.leaves();
            let g = leaves.iter().copied().find(|g| *g == self.active).unwrap_or(leaves[0]);
            let tab = self.groups.get(g).and_then(|group| group.tabs.get(group.active));
            let name = tab.and_then(|tab| self.docs.get(&tab.doc).map(|d| d.buf.title.clone()).or_else(|| self.terminals.get(&tab.doc).map(|t| t.title())));
            let title = name.map_or("Halo".to_owned(), |name| format!("{name} - Halo"));
            let size = win.start.map_or(vec2(900.0, 640.0), |r| r.size());
            let mut builder = ViewportBuilder::default().with_title(title).with_inner_size(size).with_min_inner_size([320.0, 200.0]);
            if let Some(r) = win.start {
                builder = builder.with_position(r.min);
            }
            (win.id, ViewportId::from_hash_of(("halo_editor_window", win.id)), builder)
        }).collect()
    }

    pub fn window_ui(&mut self, ui: &mut Ui, id: u64, theme: &Theme, settings: &Settings, git: &Git, cmds: &mut Vec<Command>) {
        let Some(w) = self.grid.wins.iter().position(|win| win.id == id) else { return };
        let (close, outer, inner) = ui.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(outer), Some(inner)) = (outer, inner) {
            self.grid.wins[w].rect = Some(Rect::from_min_size(outer.min, inner.size()));
        }
        let mut pending = vec![];
        egui::CentralPanel::default().frame(Frame::new().fill(theme.bg)).show(ui, |ui| self.area_ui(ui, w, theme, settings, git, &mut pending));
        self.run_pending(pending, settings, ui.ctx(), cmds);
        if close {
            let groups = self.grid.wins[w].node.leaves();
            for &g in groups.iter().rev() {
                for t in (0..self.groups[g].tabs.len()).rev() {
                    let doc = self.groups[g].tabs[t].doc;
                    if !self.docs.get(&doc).is_some_and(|d| d.buf.is_dirty()) || self.references(doc) > 1 {
                        self.close_tab(g, t);
                    }
                }
                self.groups[g].used = true;
            }
            if let Some(&g) = groups.iter().find(|g| !self.groups[**g].tabs.is_empty()) {
                ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                self.closing = Some((g, self.groups[g].active.min(self.groups[g].tabs.len() - 1)));
            }
        }
    }

    pub fn grid_state(&self) -> serde_json::Value {
        let groups = self.groups.iter().map(|group| {
            group.tabs.iter().filter(|t| !t.preview && t.diff.is_none() && t.merge.is_none() && t.md.is_none())
                .filter_map(|t| Some((self.docs.get(&t.doc)?.buf.path.clone()?, t.pinned)))
                .collect()
        }).collect();
        let wins = self.grid.wins.iter().map(|w| (w.node.clone(), w.rect.map(|r| [r.left(), r.top(), r.width(), r.height()]))).collect();
        let locked = (0..self.groups.len()).filter(|&g| self.groups[g].locked).collect();
        serde_json::to_value(Saved { wins, groups, active: self.active, locked }).unwrap_or_default()
    }

    pub fn restore_grid(&mut self, value: &serde_json::Value) -> bool {
        let Ok(saved) = serde_json::from_value::<Saved>(value.clone()) else { return false };
        let mut leaves: Vec<usize> = saved.wins.iter().flat_map(|w| w.0.leaves()).collect();
        leaves.sort_unstable();
        if saved.groups.is_empty() || leaves != (0..saved.groups.len()).collect::<Vec<_>>() {
            return false;
        }
        self.groups = saved.groups.iter().map(|_| Group { used: true, ..Default::default() }).collect();
        self.grid.wins = saved.wins.into_iter().enumerate().map(|(i, (node, rect))| {
            let rect = rect.filter(|_| i > 0).map(|[x, y, w, h]| Rect::from_min_size(pos2(x, y), vec2(w, h)));
            Win { node, id: i as u64, start: rect, rect }
        }).collect();
        self.grid.next = self.grid.wins.len() as u64;
        for (g, tabs) in saved.groups.iter().enumerate() {
            self.active = g;
            for (path, pinned) in tabs.iter().filter(|t| t.0.is_file()) {
                let id = self.find_doc(path).unwrap_or_else(|| self.insert_deferred(path));
                self.add_tab(id, vec![Sel::caret(0)]);
                self.groups[g].tabs.last_mut().into_iter().for_each(|tab| tab.pinned = *pinned);
            }
        }
        for (g, group) in self.groups.iter_mut().enumerate() {
            group.locked = saved.locked.contains(&g);
        }
        self.active = saved.active.min(self.groups.len() - 1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Git;
    use eframe::egui::{Event, Pos2};
    use Command::*;
    use Node::Leaf;

    fn input(events: Vec<Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))), events, focused: true, ..Default::default() }
    }

    fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<Event>) {
        let (theme, git, settings) = (Theme::dark(), Git::new(), Settings::default());
        let mut out = ctx.run_ui(input(events), |ui| editors.ui(ui, &theme, &settings, &git, &mut vec![]));
        out.textures_delta.clear();
    }

    fn window(ctx: &egui::Context, editors: &mut Editors, id: u64) {
        let (theme, git, settings) = (Theme::dark(), Git::new(), Settings::default());
        let mut out = ctx.run_ui(input(vec![]), |ui| editors.window_ui(ui, id, &theme, &settings, &git, &mut vec![]));
        out.textures_delta.clear();
    }

    fn run(editors: &mut Editors, ctx: &egui::Context, cmds: &[Command]) {
        cmds.iter().for_each(|cmd| {
            editors.exec(cmd, &Settings::default(), ctx);
        });
    }

    fn docs(editors: &Editors, g: usize) -> Vec<u64> {
        editors.groups[g].tabs.iter().map(|t| t.doc).collect()
    }

    #[test]
    fn splits_down_moves_a_tab_into_the_new_group_and_closes_it() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        run(&mut editors, &ctx, &[NewFile, NewFile, SplitEditorDown]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.grid.wins[0].node, line(false, [Leaf(0), Leaf(1)]));
        assert_eq!((docs(&editors, 1), editors.active), (vec![2], 1));
        run(&mut editors, &ctx, &[FocusGroupUp]);
        assert_eq!(editors.active, 0);
        editors.groups[0].active = 0;
        run(&mut editors, &ctx, &[MoveEditorDown]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((docs(&editors, 0), docs(&editors, 1), editors.active), (vec![2], vec![2, 1], 1));
        assert!(ctx.read_response(Id::new(("editor_splitter", 0u64, &Vec::<usize>::new(), 0usize))).is_some());
        run(&mut editors, &ctx, &[CloseGroup]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.groups.len(), editors.grid.wins[0].node.clone(), editors.active), (1, Leaf(0), 0));
        assert_eq!(docs(&editors, 0), vec![2]);
        assert_eq!(editors.docs.len(), 1);
    }

    #[test]
    fn applies_layouts_and_focuses_and_moves_by_direction_in_a_grid() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        run(&mut editors, &ctx, &[NewFile, LayoutGrid]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.groups.len(), 4);
        assert_eq!(editors.grid.wins[0].node, preset(&LayoutGrid).unwrap());
        let step = |editors: &mut Editors, cmd: Command| {
            run(editors, &ctx, &[cmd]);
            editors.active
        };
        assert_eq!([step(&mut editors, FocusGroupRight), step(&mut editors, FocusGroupDown), step(&mut editors, FocusGroupLeft), step(&mut editors, FocusGroupUp)], [1, 3, 2, 0]);
        assert_eq!(step(&mut editors, FocusGroupUp), 0);
        run(&mut editors, &ctx, &[MoveEditorRight, MoveEditorDown]);
        assert_eq!((docs(&editors, 3), editors.active), (vec![1], 3));
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.grid.wins[0].node.clone(), docs(&editors, 1)), (line(true, [Leaf(0), Leaf(1)]), vec![1]));
        run(&mut editors, &ctx, &[LayoutThreeColumns]);
        assert_eq!(editors.grid.wins[0].node.leaves().len(), 3);
        run(&mut editors, &ctx, &[LayoutSingle]);
        assert_eq!((editors.groups.len(), docs(&editors, 0)), (1, vec![1]));
    }

    #[test]
    fn maximizes_a_group_and_restores_the_same_layout() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        run(&mut editors, &ctx, &[NewFile, SplitEditor, SplitEditorDown]);
        frame(&ctx, &mut editors, vec![]);
        let before = editors.grid.wins[0].node.clone();
        let strip = |g: usize| ctx.read_response(Id::new(("editor_strip", g))).map(|r| r.rect.width());
        assert!(strip(2).unwrap() < 500.0);
        run(&mut editors, &ctx, &[ToggleMaximizeGroup]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!(editors.grid.maximized, Some(2));
        assert!(strip(2).unwrap() > 800.0 && strip(0).is_none(), "{:?} {:?}", strip(2), strip(0));
        run(&mut editors, &ctx, &[ToggleMaximizeGroup]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.grid.maximized, editors.grid.wins[0].node.clone()), (None, before));
        assert!(strip(2).unwrap() < 500.0 && strip(0).is_some());
    }

    #[test]
    fn a_tab_moved_to_a_floating_window_and_back_keeps_its_document_caret_and_history() {
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        run(&mut editors, &ctx, &[NewFile]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Text("hello".into())]);
        let carets = |e: &Editors, g: usize| e.groups[g].tabs[0].sels.iter().map(|s| (s.start(), s.end(), s.head)).collect::<Vec<_>>();
        let sels = carets(&editors, 0);
        run(&mut editors, &ctx, &[MoveEditorToNewWindow]);
        assert_eq!((editors.grid.wins.len(), docs(&editors, 1), editors.active), (2, vec![1], 1));
        let id = editors.floating()[0].0;
        window(&ctx, &mut editors, id);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.groups.len(), carets(&editors, 1)), (2, sels.clone()));
        run(&mut editors, &ctx, &[MoveEditorToMainWindow]);
        frame(&ctx, &mut editors, vec![]);
        assert_eq!((editors.grid.wins.len(), editors.groups.len(), docs(&editors, 0)), (1, 1, vec![1]));
        assert_eq!(carets(&editors, 0), sels);
        run(&mut editors, &ctx, &[Undo]);
        assert_eq!(editors.docs[&1].buf.rope.to_string(), "");
        run(&mut editors, &ctx, &[CopyEditorToNewWindow]);
        assert_eq!((editors.grid.wins.len(), docs(&editors, 0), docs(&editors, 1)), (2, vec![1], vec![1]));
    }

    #[test]
    fn saves_the_grid_and_floating_windows_and_restores_them() {
        let dir = std::env::temp_dir().join(format!("halo-grid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| dir.join(n)).collect();
        files.iter().for_each(|f| std::fs::write(f, "x").unwrap());
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        files.iter().for_each(|f| editors.open_kept(f));
        run(&mut editors, &ctx, &[SplitEditorDown]);
        editors.move_to_window(0, 0, false, Some(Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0))));
        editors.grid.wins[0].node.resize(&[], 0, 0.2, 0.1);
        editors.groups[0].tabs[0].pinned = true;
        let state = serde_json::from_str(&serde_json::to_string(&editors.grid_state()).unwrap()).unwrap();
        let mut back = Editors::new();
        assert!(back.restore_grid(&state));
        back.finish_loads();
        assert_eq!(back.grid.wins[0].node, editors.grid.wins[0].node);
        assert_eq!((back.grid.wins.len(), back.grid.wins[1].start.map(|r| r.min)), (2, Some(pos2(10.0, 20.0))));
        let paths = |e: &Editors, g: usize| e.groups[g].tabs.iter().map(|t| (e.docs[&t.doc].buf.path.clone().unwrap(), t.pinned)).collect::<Vec<_>>();
        for g in 0..3 {
            assert_eq!(paths(&back, g), paths(&editors, g));
        }
        assert_eq!(back.active, editors.active);
        assert!(!Editors::new().restore_grid(&serde_json::json!({"wins": [[{"Leaf": 3}, null]], "groups": [[]], "active": 0})));
        assert!(!Editors::new().restore_grid(&serde_json::json!(["old"])));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_locked_group_sends_new_files_to_another_group_and_stays_locked_after_restore() {
        let dir = std::env::temp_dir().join(format!("halo-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| dir.join(n)).collect();
        files.iter().for_each(|f| std::fs::write(f, "x").unwrap());
        let ctx = egui::Context::default();
        let mut editors = Editors::new();
        editors.open_kept(&files[0]);
        run(&mut editors, &ctx, &[SplitEditor, ToggleEditorGroupLock]);
        assert!(editors.groups[1].locked && editors.active == 1);
        editors.open(&files[1]);
        assert_eq!((editors.active, docs(&editors, 0).len(), docs(&editors, 1).len()), (0, 2, 1));
        editors.active = 1;
        editors.open_kept(&files[0]);
        assert_eq!((editors.active, docs(&editors, 1).len()), (1, 1));
        run(&mut editors, &ctx, &[NewFile]);
        assert_eq!((editors.active, docs(&editors, 0).len()), (0, 3));
        editors.groups[0].locked = true;
        editors.active = 1;
        editors.open_kept(&files[2]);
        assert_eq!((editors.groups.len(), editors.active, docs(&editors, 2).len()), (3, 2, 1));
        let state = serde_json::from_str(&serde_json::to_string(&editors.grid_state()).unwrap()).unwrap();
        let mut back = Editors::new();
        assert!(back.restore_grid(&state));
        assert_eq!(back.groups.iter().map(|g| g.locked).collect::<Vec<_>>(), [true, true, false]);
        run(&mut back, &ctx, &[ToggleEditorGroupLock]);
        assert!(back.groups.iter().all(|g| g.locked));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
