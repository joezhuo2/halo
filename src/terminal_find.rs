use crate::theme::Theme;
use crate::ui::{self, icons};
use eframe::egui::{self, Align, CornerRadius, Key, Layout, Rect, RichText, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2};

pub fn search(text: &str, query: &str, case: bool, word: bool, regex: bool) -> Result<Vec<(usize, usize)>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }
    let found = crate::editor::Pattern::new(query, case, word, regex)?.find(text);
    let (mut byte, mut char) = (0, 0);
    let mut out = Vec::with_capacity(found.len());
    for (start, end) in found {
        char += text[byte..start].chars().count();
        let from = char;
        char += text[start..end].chars().count();
        byte = end;
        out.push((from, char));
    }
    Ok(out)
}

pub fn segments(cells: &[(usize, usize)], (from, to): (usize, usize)) -> Vec<(usize, usize, usize)> {
    let mut out: Vec<(usize, usize, usize)> = vec![];
    for &(row, col) in &cells[from..to.min(cells.len())] {
        match out.last_mut() {
            Some(last) if last.0 == row && last.2 == col => last.2 = col + 1,
            _ => out.push((row, col, col + 1)),
        }
    }
    out
}

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub focus: bool,
    pub has_focus: bool,
    query: String,
    case: bool,
    word: bool,
    regex: bool,
    pub error: Option<String>,
    pub matches: Vec<Vec<(i32, usize, usize)>>,
    pub current: Option<usize>,
    key: (String, [bool; 3]),
    searched: f64,
}

impl FindBar {
    pub fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub fn due(&self, now: f64) -> Option<bool> {
        let fresh = self.key != (self.query.clone(), [self.case, self.word, self.regex]);
        (self.open && (fresh || now - self.searched > 0.5)).then_some(fresh)
    }

    pub fn run(&mut self, text: &str, cells: &[(usize, usize)], top: i32, now: f64, fresh: bool) {
        self.key = (self.query.clone(), [self.case, self.word, self.regex]);
        self.searched = now;
        match search(text, &self.query, self.case, self.word, self.regex) {
            Ok(found) => {
                self.error = None;
                let at = |r: usize| top + r as i32;
                self.matches = found.into_iter().map(|m| segments(cells, m).into_iter().map(|(r, a, b)| (at(r), a, b)).collect()).collect();
            }
            Err(error) => (self.error, self.matches) = (Some(error), vec![]),
        }
        self.current = match (self.matches.len(), self.current) {
            (0, _) => None,
            (n, _) if fresh => Some(n - 1),
            (n, current) => current.map(|c| c.min(n - 1)),
        };
    }

    pub fn step(&mut self, forward: bool) -> Option<i32> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        let next = match self.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None => n - 1,
        };
        self.current = Some(next);
        self.matches[next].first().map(|s| s.0)
    }

    pub fn close(&mut self) {
        (self.open, self.matches, self.current, self.key.0) = (false, vec![], None, String::new());
    }

    pub fn ui(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) -> (i32, bool) {
        let size = vec2(420.0f32.min(area.width() - 20.0), 38.0);
        let rect = Rect::from_min_size(pos2(area.right() - size.x - 14.0, area.top()), size);
        let corners = CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 };
        ui.painter().with_clip_rect(area).rect(rect, corners, theme.raised, Stroke::new(1.0, theme.border), StrokeKind::Inside);
        let (enter, shift, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift, i.key_pressed(Key::Escape)));
        let f3 = ui.input(|i| i.key_pressed(Key::F3));
        let (mut step, mut close) = (0, false);
        self.has_focus = false;
        ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 5.0))), |ui| {
            ui.set_clip_rect(area);
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            ui.horizontal(|ui| {
                let field = ui.add(TextEdit::singleline(&mut self.query).margin(ui::FIELD).desired_width(150.0).hint_text("Find"));
                if self.focus {
                    field.request_focus();
                    self.focus = false;
                }
                self.has_focus = field.has_focus();
                if field.has_focus() && f3 {
                    step = if shift { -1 } else { 1 };
                }
                if field.lost_focus() {
                    if enter {
                        step = if shift { -1 } else { 1 };
                        field.request_focus();
                        self.has_focus = true;
                    } else if escape {
                        close = true;
                    }
                }
                ui.add_space(4.0);
                let (label, color) = match (self.query.is_empty(), self.matches.len(), self.current) {
                    (true, ..) => (String::new(), theme.text_dim),
                    _ if self.error.is_some() => ("Bad pattern".to_owned(), theme.error),
                    (_, 0, _) => ("No results".to_owned(), theme.error),
                    (_, n, Some(i)) => (format!("{} of {n}", i + 1), theme.text_dim),
                    (_, n, None) => (format!("{n} found"), theme.text_dim),
                };
                ui.label(RichText::new(label).size(12.0).color(color)).on_hover_text(self.error.clone().unwrap_or_default());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui::icon_button(ui, theme, icons::X, 22.0, false, "Close (Escape)").clicked() {
                        close = true;
                    }
                    if ui::icon_button(ui, theme, icons::ARROW_DOWN, 22.0, false, "Next Match (Enter, F3)").clicked() {
                        step = 1;
                    }
                    if ui::icon_button(ui, theme, icons::ARROW_UP, 22.0, false, "Previous Match (Shift+Enter, Shift+F3)").clicked() {
                        step = -1;
                    }
                    if ui::icon_button(ui, theme, icons::ASTERISK, 22.0, self.regex, "Use Regular Expression").clicked() {
                        self.regex ^= true;
                    }
                    if ui::icon_button(ui, theme, icons::TEXT_UNDERLINE, 22.0, self.word, "Match Whole Word").clicked() {
                        self.word ^= true;
                    }
                    if ui::icon_button(ui, theme, icons::TEXT_AA, 22.0, self.case, "Match Case").clicked() {
                        self.case ^= true;
                    }
                });
            });
        });
        if close {
            self.close();
        }
        if ui.rect_contains_pointer(rect) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }
        (step, close)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_links::join;

    #[test]
    fn searches_scrollback_with_each_toggle() {
        let row = |s: &str, wrapped| (s.chars().collect::<Vec<_>>(), wrapped);
        let (text, cells) = join(&[row("Error: café one", false), row("an error and an err", true), row("or", false), row("errors", false)]);
        let all = |q: &str, case, word, regex| search(&text, q, case, word, regex).unwrap();
        assert_eq!(all("error", false, false, false).len(), 4);
        assert_eq!(all("error", true, false, false).len(), 3);
        assert_eq!(all("error", false, true, false).len(), 3);
        assert_eq!(all("Error", true, true, false), vec![(0, 5)]);
        assert_eq!(all(r"err\w*", false, false, true).len(), 4);
        assert_eq!(all("one", false, false, false), vec![(12, 15)]);
        assert!(search(&text, "(", false, false, true).is_err());
        assert!(all("", false, false, false).is_empty());
        let wrapped = all("an error", false, false, false)[1];
        assert_eq!(segments(&cells, wrapped), vec![(1, 13, 19), (2, 0, 2)]);
        let mut bar = FindBar { open: true, query: "error".into(), ..Default::default() };
        assert_eq!(bar.due(0.0), Some(true));
        bar.run(&text, &cells, -4, 0.0, true);
        assert_eq!((bar.matches.len(), bar.current), (4, Some(3)));
        assert_eq!(bar.matches[3], vec![(-1, 0, 5)]);
        assert_eq!(bar.due(0.1), None);
        assert_eq!(bar.step(true), Some(-4));
        assert_eq!(bar.step(false), Some(-1));
    }
}
