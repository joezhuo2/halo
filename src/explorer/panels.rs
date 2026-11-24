use super::*;

fn confirm(ctx: &egui::Context, id: &str, title: &str, text: &str, ok: RichText) -> Option<bool> {
    let mut choice = None;
    let modal = ui::modal(ctx, id, ui::Enter::Primary, |ui, keys| {
        ui::outline(ui);
        ui.set_width(340.0);
        ui.heading(title);
        ui.label(text);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if keys.primary(ui, ok, true) {
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
    choice
}

impl Explorer {
    pub(super) fn open_editors(&mut self, ui: &mut Ui, theme: &Theme, root: &Path, tabs: &[Vec<OpenTab>], tree: bool, cmds: &mut Vec<Command>) {
        let dir_of = |tab: &OpenTab| {
            let dir = tab.path.as_deref().and_then(Path::parent).map(|p| p.strip_prefix(root).unwrap_or(p));
            dir.map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default()
        };
        let tabs: Vec<Vec<&OpenTab>> = tabs.iter().map(|group| {
            let mut sorted: Vec<&OpenTab> = group.iter().collect();
            if tree {
                sorted.sort_by_cached_key(|tab| dir_of(tab));
            }
            sorted
        }).collect();
        let count: usize = tabs.iter().map(Vec::len).sum();
        let header = ui::section(ui, theme, "OPEN EDITORS", &mut self.open_editors).rect;
        if ui.rect_contains_pointer(header) && count > 0 {
            ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(6.0, 1.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if ui::icon_button(ui, theme, icons::X, 22.0, false, "Close All").clicked() {
                    cmds.push(Command::CloseAllEditors);
                }
                if ui::icon_button(ui, theme, icons::FLOPPY_DISK, 22.0, false, "Save All").clicked() {
                    cmds.push(Command::SaveAll);
                }
                let (icon, tip) = if tree { (icons::LIST, "Show as List") } else { (icons::TREE_VIEW, "Show as Tree") };
                if ui::icon_button(ui, theme, icon, 22.0, false, tip).clicked() {
                    cmds.push(Command::ToggleOpenEditorsTree);
                }
            });
        }
        let groups = tabs.iter().filter(|g| !g.is_empty()).count();
        let height = ui.available_height();
        if height < 1.0 {
            self.open_focus = false;
            return;
        }
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let list_id = Id::new("explorer_open_list");
        let list = ui.interact(rect, list_id, Sense::click());
        if std::mem::take(&mut self.focus_open) {
            list.request_focus();
        }
        self.open_focus = list.has_focus() && self.open_editors;
        let flat: Vec<(usize, usize)> = tabs.iter().flatten().map(|tab| (tab.group, tab.index)).collect();
        self.open_cursor = self.open_cursor.min(flat.len().saturating_sub(1));
        if self.open_focus {
            ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
            let pressed = |key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            if pressed(Key::ArrowDown) {
                self.open_cursor = (self.open_cursor + 1).min(flat.len().saturating_sub(1));
            } else if pressed(Key::ArrowUp) {
                self.open_cursor = self.open_cursor.saturating_sub(1);
            } else if let Some(&(g, i)) = flat.get(self.open_cursor) {
                if pressed(Key::Enter) {
                    cmds.push(Command::ShowTab(g, i));
                } else if pressed(Key::Delete) {
                    cmds.push(Command::CloseTab(g, i));
                }
            }
        }
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.shrink_clip_rect(rect);
            ScrollArea::vertical().id_salt("explorer_open_editors").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut flat_index = 0;
                for (g, group) in tabs.iter().enumerate().filter(|(_, group)| !group.is_empty()) {
                    if groups > 1 {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                        let text = format!("GROUP {}", g + 1);
                        ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(10.5), theme.text_dim);
                    }
                    let mut folder = String::new();
                    for &tab in group {
                        let dir = dir_of(tab);
                        if tree && dir != folder && !dir.is_empty() {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                            let painter = ui.painter().with_clip_rect(r);
                            painter.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, icons::FOLDER_OPEN, FontId::proportional(14.0), theme.text_dim);
                            painter.text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, &dir, FontId::proportional(12.5), theme.text_dim);
                        }
                        folder.clone_from(&dir);
                        let indent = if tree && !dir.is_empty() { 14.0 } else { 0.0 };
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                        let focused = self.open_focus && flat_index == self.open_cursor;
                        flat_index += 1;
                        ui::row_bg(ui, theme, r, resp.id, resp.hovered(), tab.active || focused);
                        ui::list_item(ui, &resp, &tab.title, tab.active, (flat_index - 1, tabs.iter().map(Vec::len).sum()));
                        let close = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(18.0, 18.0));
                        let x = ui.interact(close, resp.id.with("close"), Sense::click());
                        let painter = ui.painter().with_clip_rect(r.with_max_x(close.left() - 2.0));
                        painter.text(pos2(r.left() + 12.0 + indent, r.center().y), Align2::LEFT_CENTER, file_icon(&tab.title), FontId::proportional(15.0), theme.text_dim);
                        let tint = name_color(theme, tab.severity, None);
                        let title = painter.layout_no_wrap(tab.title.clone(), FontId::proportional(13.0), tint);
                        let after = r.left() + 34.0 + indent + title.size().x + 8.0;
                        painter.galley(pos2(r.left() + 34.0 + indent, r.center().y - title.size().y / 2.0), title, tint);
                        if !tree && !dir.is_empty() {
                            painter.text(pos2(after, r.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme.text_dim);
                        }
                        if resp.hovered() || x.hovered() {
                            let hover = ui::anim(ui, x.id.with("hover"), x.hovered());
                            ui.painter().rect_filled(close, 4, ui::mix(Color32::TRANSPARENT, theme.hover, hover));
                            ui.painter().text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(12.0), theme.text);
                        } else if tab.dirty {
                            ui.painter().circle_filled(close.center(), 4.0, theme.text_dim);
                        }
                        if x.clicked() || resp.middle_clicked() {
                            cmds.push(Command::CloseTab(tab.group, tab.index));
                        } else if resp.clicked() {
                            list.request_focus();
                            self.open_cursor = flat_index - 1;
                            cmds.push(Command::ShowTab(tab.group, tab.index));
                        }
                    }
                }
            });
        });
    }

    pub(super) fn filter_box(&mut self, ui: &mut Ui, theme: &Theme, area: Rect, action: &mut Option<Action>) {
        let rect = Rect::from_min_size(pos2(area.right() - 248.0, area.top() + 4.0), vec2(240.0, 26.0));
        if rect.left() < area.left() {
            return;
        }
        ui.painter().rect_filled(rect, 4, theme.raised);
        ui.painter().rect_stroke(rect, 4, Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 2.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui::icon_button(ui, theme, icons::X, 20.0, false, "Close (Escape)").clicked() {
                *action = Some(Action::CloseFilter);
            }
            let tip = if self.filter.hide { "Showing only matches. Click to highlight instead." } else { "Highlighting matches. Click to hide the rest." };
            let icon = if self.filter.hide { icons::FUNNEL } else { icons::HIGHLIGHTER };
            if ui::icon_button(ui, theme, icon, 20.0, true, tip).clicked() {
                self.filter.hide ^= true;
                self.dirty = true;
            }
            let count = match (self.filter.text.trim().is_empty(), self.filter.matches) {
                (true, _) => String::new(),
                (_, 0) => "None".to_owned(),
                (_, n) => n.to_string(),
            };
            ui.label(RichText::new(count).size(11.5).color(theme.text_dim));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let edit = egui::TextEdit::singleline(&mut self.filter.text)
                    .hint_text("Type to filter")
                    .frame(egui::Frame::NONE)
                    .font(FontId::proportional(13.0))
                    .desired_width(ui.available_width());
                let out = ui.add(edit);
                if std::mem::take(&mut self.filter.focus) {
                    out.request_focus();
                }
                if out.changed() {
                    self.dirty = true;
                }
                if out.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    *action = Some(Action::Match(!ui.input(|i| i.modifiers.shift)));
                    out.request_focus();
                } else if (out.has_focus() || out.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape)) {
                    *action = Some(Action::CloseFilter);
                } else if out.has_focus() {
                    if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                        *action = Some(Action::Match(false));
                    } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                        *action = Some(Action::Match(true));
                    }
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn drag_state(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        area: Rect,
        pointer: Option<Pos2>,
        over_row: bool,
        over_dir: Option<PathBuf>,
        action: &mut Option<Action>,
    ) {
        let Some(paths) = self.drag.clone() else {
            self.hover_dir = None;
            return;
        };
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        let root_target = pointer.is_some() && !over_row;
        if root_target {
            ui.painter().rect_stroke(area, 0, Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
        }
        match &over_dir {
            Some(dir) if !self.expanded.contains(dir) => match &self.hover_dir {
                Some((hovered, since)) if hovered == dir => {
                    if now - since > HOVER_EXPAND {
                        *action = Some(Action::Expand(dir.clone()));
                        self.hover_dir = None;
                    }
                }
                _ => self.hover_dir = Some((dir.clone(), now)),
            },
            _ => self.hover_dir = None,
        }
        if let Some(pos) = ctx.pointer_latest_pos() {
            let label = if paths.len() == 1 { name_of(&paths[0]) } else { format!("{} items", paths.len()) };
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("explorer_drag")));
            let galley = painter.layout_no_wrap(label, FontId::proportional(12.5), theme.text);
            let rect = Rect::from_min_size(pos + vec2(14.0, 8.0), galley.size() + vec2(16.0, 8.0));
            painter.rect(rect, 4, theme.raised, Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
            painter.galley(rect.min + vec2(8.0, 4.0), galley, theme.text);
        }
        if ui.input(|i| i.pointer.any_released()) {
            self.drag = None;
            self.hover_dir = None;
            match (over_dir, root_target, ctx.pointer_latest_pos()) {
                (Some(dir), _, _) => *action = Some(Action::Drop(paths, dir)),
                (None, true, _) => *action = self.root.clone().or_else(|| self.roots.first().map(|(r, _)| r.clone())).map(|root| Action::Drop(paths, root)),
                (None, false, Some(pos)) => self.dropped = Some((paths, pos)),
                _ => {}
            }
        }
    }

    pub(super) fn overlays(&mut self, ui: &mut Ui, theme: &Theme) {
        if let Some(error) = self.error.clone() {
            let rect = Rect::from_min_max(pos2(ui.max_rect().left(), ui.max_rect().bottom() - 30.0), ui.max_rect().max);
            ui.painter().rect_filled(rect, 0, ui::mix(theme.panel, theme.error, 0.18));
            ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 3.0))).layout(Layout::right_to_left(Align::Center)), |ui| {
                if ui::icon_button(ui, theme, icons::X, 20.0, false, "Dismiss").clicked() {
                    self.error = None;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&error).color(theme.error).size(12.0)).truncate()).on_hover_text(&error);
                });
            });
        }
        let ctx = ui.ctx().clone();
        if !self.confirm_delete.is_empty() && !self.confirm {
            self.delete(&ctx);
        } else if !self.confirm_delete.is_empty() {
            let text = format!("Move {} to the recycle bin?", count_text(&self.confirm_delete));
            match confirm(&ctx, "explorer_delete", "Delete", &text, RichText::new("Move to Recycle Bin").color(theme.deleted)) {
                Some(true) => self.delete(&ctx),
                Some(false) => self.confirm_delete.clear(),
                None => {}
            }
        }
        if let Some(paths) = self.confirm_permanent.front() {
            let pronoun = if paths.len() == 1 { "it" } else { "them" };
            let text = format!("{} cannot be moved to the trash. Permanently delete {pronoun}? This cannot be undone.", count_text(paths));
            let ok = RichText::new("Delete Permanently").color(theme.deleted);
            if let Some(delete) = confirm(&ctx, "explorer_permanent", "Delete permanently", &text, ok) {
                let paths = self.confirm_permanent.pop_front().unwrap();
                if delete {
                    self.background(&ctx, move || ops::delete_permanently(paths));
                }
            }
        }
        if let Some((paths, dir)) = self.confirm_drop.clone() {
            let text = format!("Move {} into \"{}\"?", count_text(&paths), name_of(&dir));
            match confirm(&ctx, "explorer_drop", "Move", &text, RichText::new("Move").color(theme.accent)) {
                Some(true) => {
                    self.confirm_drop = None;
                    self.transfer(paths, dir, true, &ctx);
                }
                Some(false) => self.confirm_drop = None,
                None => {}
            }
        }
        if let Some(jobs) = self.clash.clone() {
            let names: Vec<PathBuf> = jobs.iter().filter(|j| j.replace).map(|j| j.to.clone()).collect();
            let text = format!("{} already exist{} in the destination. Replace {}?", count_text(&names), if names.len() == 1 { "s" } else { "" }, if names.len() == 1 { "it" } else { "them" });
            match confirm(&ctx, "explorer_replace", "Replace", &text, RichText::new("Replace").color(theme.deleted)) {
                Some(true) => {
                    self.clash = None;
                    self.spawn(jobs, true, &ctx);
                }
                Some(false) => self.clash = None,
                None => {}
            }
        }
    }
}
