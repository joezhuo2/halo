use crate::theme::Theme;
use crate::ui;
use eframe::egui::{CornerRadius, FontId, Painter, Pos2, Rect, pos2, vec2};

const KEYS: &[u8] = b"asdfjklghqwertyuiopzxcvbnm";
pub const MAX: usize = KEYS.len() * KEYS.len();

pub fn label(index: usize) -> String {
    [KEYS[index / KEYS.len() % KEYS.len()], KEYS[index % KEYS.len()]].iter().map(|b| *b as char).collect()
}

pub fn words(line: &str) -> Vec<usize> {
    let mut prev = None;
    let mut out = vec![];
    for (column, c) in line.chars().enumerate() {
        let word = c.is_alphanumeric() || c == '_';
        if word && !prev.is_some_and(|p: char| p.is_alphanumeric() || p == '_') {
            out.push(column);
        }
        prev = Some(c);
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum Pick {
    Wait,
    Go(usize),
    Cancel,
}

pub fn pick(typed: &str, targets: &[usize]) -> Pick {
    let typed: String = typed.chars().take(2).collect();
    match typed.chars().count() {
        0 => Pick::Wait,
        1 if (0..targets.len()).any(|i| label(i).starts_with(&typed)) => Pick::Wait,
        1 => Pick::Cancel,
        _ => (0..targets.len()).find(|i| label(*i) == typed).map_or(Pick::Cancel, |i| Pick::Go(targets[i])),
    }
}

pub fn paint(painter: &Painter, theme: &Theme, at: Pos2, label: &str, typed: &str, font: &FontId, line_h: f32) {
    let live = label.starts_with(typed);
    let shown = if live { &label[typed.len().min(1)..] } else { label };
    let galley = painter.layout_no_wrap(shown.to_owned(), font.clone(), theme.bg);
    let r = Rect::from_min_size(at, vec2(galley.size().x + 2.0, line_h));
    painter.rect_filled(r, CornerRadius::same(3), if live { theme.accent } else { ui::with_alpha(theme.text_dim, 0.25) });
    if live {
        painter.galley(pos2(r.left() + 1.0, r.center().y - galley.size().y / 2.0), galley, theme.bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_labels_are_unique_and_pick_needs_two_keys() {
        let labels: std::collections::HashSet<String> = (0..MAX).map(label).collect();
        assert_eq!(labels.len(), MAX);
        assert_eq!(words("let x_1 = foo(bar);"), [0, 4, 10, 14]);
        let targets = [3, 9, 20];
        assert_eq!(pick("", &targets), Pick::Wait);
        assert_eq!(pick("a", &targets), Pick::Wait);
        assert_eq!(pick("as", &targets), Pick::Go(9));
        assert_eq!(pick("ad", &targets), Pick::Go(20));
        assert_eq!(pick("az", &targets), Pick::Cancel);
        assert_eq!(pick("q", &targets), Pick::Cancel);
    }
}
