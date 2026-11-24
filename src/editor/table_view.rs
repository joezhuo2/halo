use super::buffer::Document;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use std::cmp::Ordering;
use std::path::Path;

const MAX_CHARS: usize = 40;

#[derive(Default)]
pub struct TableView {
    version: Option<u64>,
    rows: Vec<Vec<String>>,
    widths: Vec<usize>,
    order: Vec<usize>,
    sort: Option<(usize, bool)>,
}

pub fn is_table(path: Option<&Path>) -> bool {
    path.and_then(Path::extension).is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "csv" | "tsv" | "psv"))
}

pub fn delimiter(path: Option<&Path>, text: &str) -> char {
    match path.and_then(Path::extension).map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("tsv") => return '\t',
        Some("psv") => return '|',
        _ => {}
    }
    let first = text.lines().next().unwrap_or_default();
    [',', ';', '\t', '|'].into_iter().max_by_key(|d| (first.matches(*d).count(), *d == ',')).unwrap_or(',')
}

pub fn parse(text: &str, delim: char) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (vec![], vec![], String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, c) if c == delim => row.push(std::mem::take(&mut field)),
            (false, '\r') if chars.peek() == Some(&'\n') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn compare(a: &str, b: &str) -> Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

impl TableView {
    fn sync(&mut self, buf: &Document) {
        if self.version == Some(buf.version) {
            return;
        }
        let text = buf.rope.to_string();
        self.rows = parse(&text, delimiter(buf.path.as_deref(), &text));
        let cols = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        self.widths = (0..cols).map(|c| self.rows.iter().take(2000).map(|r| r.get(c).map_or(0, |f| f.chars().count())).max().unwrap_or(0).clamp(3, MAX_CHARS)).collect();
        self.version = Some(buf.version);
        self.resort();
    }

    fn resort(&mut self) {
        self.order = (1..self.rows.len()).collect();
        if let Some((col, down)) = self.sort {
            let rows = &self.rows;
            self.order.sort_by(|&a, &b| {
                let o = compare(rows[a].get(col).map_or("", String::as_str), rows[b].get(col).map_or("", String::as_str));
                if down { o.reverse() } else { o }
            });
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, rect: Rect, buf: &Document, font_size: f32) {
        self.sync(buf);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg);
        if self.rows.is_empty() {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "The file has no rows.", FontId::proportional(13.0), theme.text_dim);
            return;
        }
        let font = FontId::monospace(font_size);
        let (cw, row_h) = ui.ctx().fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font) + 6.0));
        let number_w = (self.rows.len().to_string().len() as f32 + 2.0) * cw;
        let col_w: Vec<f32> = self.widths.iter().map(|w| *w as f32 * cw + 16.0).collect();
        let total = vec2(number_w + col_w.iter().sum::<f32>(), (self.order.len() + 1) as f32 * row_h);
        let mut clicked = None;
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ScrollArea::both().id_salt(("table_view", buf.path.as_deref())).auto_shrink(false).show_viewport(ui, |ui, view| {
                let (_, area) = ui.allocate_space(total);
                let origin = area.min;
                let painter = ui.painter().clone();
                let first = ((view.min.y / row_h).floor() as usize).saturating_sub(1);
                let last = ((view.max.y / row_h).ceil() as usize + 1).min(self.order.len());
                let cell = |painter: &egui::Painter, x: f32, y: f32, w: f32, text: &str, color| {
                    let shown: String = text.chars().take(MAX_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect();
                    let clip = Rect::from_min_size(pos2(x, y), vec2(w - 8.0, row_h));
                    painter.with_clip_rect(clip.intersect(painter.clip_rect())).text(pos2(x + 8.0, y + row_h / 2.0), Align2::LEFT_CENTER, shown, font.clone(), color);
                };
                for (i, &row) in self.order.iter().enumerate().take(last).skip(first) {
                    let y = origin.y + (i + 1) as f32 * row_h;
                    if i % 2 == 1 {
                        painter.rect_filled(Rect::from_min_size(pos2(origin.x, y), vec2(total.x, row_h)), CornerRadius::ZERO, ui::mix(theme.bg, theme.text, 0.03));
                    }
                    painter.text(pos2(origin.x + number_w - 8.0, y + row_h / 2.0), Align2::RIGHT_CENTER, (row + 1).to_string(), font.clone(), theme.text_dim);
                    let mut x = origin.x + number_w;
                    for (c, w) in col_w.iter().enumerate() {
                        cell(&painter, x, y, *w, self.rows[row].get(c).map_or("", String::as_str), theme.text);
                        x += w;
                    }
                }
                let head_y = origin.y + view.min.y;
                let head = Rect::from_min_size(pos2(origin.x, head_y), vec2(total.x.max(view.width()), row_h));
                painter.rect_filled(head, CornerRadius::ZERO, theme.panel);
                painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, theme.border));
                let mut x = origin.x + number_w;
                for (c, w) in col_w.iter().enumerate() {
                    let r = Rect::from_min_size(pos2(x, head_y), vec2(*w, row_h));
                    let resp = ui.interact(r, ui.id().with(("table_head", c)), Sense::click()).on_hover_text("Sort by this column");
                    if resp.hovered() {
                        painter.rect_filled(r, CornerRadius::ZERO, ui::with_alpha(theme.accent, 0.1));
                    }
                    let arrow = match self.sort {
                        Some((s, down)) if s == c => if down { " ↓" } else { " ↑" },
                        _ => "",
                    };
                    let name = format!("{}{arrow}", self.rows[0].get(c).map_or("", String::as_str));
                    cell(&painter, x, head_y, *w + 16.0, &name, theme.text);
                    painter.line_segment([pos2(x, head_y + 4.0), pos2(x, head_y + row_h - 4.0)], Stroke::new(1.0, theme.border));
                    if resp.clicked() {
                        clicked = Some(c);
                    }
                    x += w;
                }
            });
        });
        if let Some(c) = clicked {
            self.sort = match self.sort {
                Some((s, false)) if s == c => Some((c, true)),
                Some((s, true)) if s == c => None,
                _ => Some((c, false)),
            };
            self.resort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_delimiters_and_sorts_numbers() {
        let rows = parse("a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\n\"multi\nline\",2\n", ',');
        assert_eq!(rows, [vec!["a", "b"], vec!["x, y", "say \"hi\""], vec!["multi\nline", "2"]]);
        assert_eq!(parse("a\tb\nc", '\t'), [vec!["a", "b"], vec!["c"]]);
        assert_eq!(delimiter(None, "a;b;c\n1;2;3"), ';');
        assert_eq!(delimiter(None, "a,b;c"), ',');
        assert_eq!(delimiter(Some(Path::new("x.tsv")), "a,b"), '\t');
        let mut buf = Document::from_text("n,name\n10,b\n9,a\nx,c\n", "t.csv".into());
        buf.path = Some("t.csv".into());
        let mut table = TableView { sort: Some((0, false)), ..Default::default() };
        table.sync(&buf);
        assert_eq!(table.order, [2, 1, 3]);
        table.sort = Some((1, true));
        table.resort();
        assert_eq!(table.order, [3, 1, 2]);
        assert!(is_table(Some(Path::new("A.CSV"))) && !is_table(Some(Path::new("a.rs"))));
    }
}
