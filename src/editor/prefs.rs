use eframe::egui::{Rect, pos2};

pub fn line_label(mode: &str, line: usize, cursor: usize) -> Option<usize> {
    match mode {
        "off" => None,
        "relative" if line != cursor => Some(line.abs_diff(cursor)),
        _ => Some(line + 1),
    }
}

pub fn line_height(setting: f32, row_h: f32, font_size: f32) -> f32 {
    match setting {
        s if s <= 0.0 => (row_h * 1.25).round(),
        s if s < 8.0 => (font_size * s).round().max(row_h),
        s => s.round().max(row_h),
    }
}

pub fn wrap_columns(mode: &str, column: usize, fits: usize) -> Option<usize> {
    match mode {
        "on" => Some(fits),
        "wordWrapColumn" => Some(column.max(1)),
        "bounded" => Some(fits.min(column.max(1))),
        _ => None,
    }
}

pub fn show_whitespace(mode: &str, line: &[char], column: usize, selected: bool) -> bool {
    match mode {
        "all" => true,
        "selection" => selected,
        "trailing" => line[column..].iter().all(|c| c.is_whitespace()),
        "boundary" => {
            let single = line[column] == ' '
                && column > 0
                && !line[column - 1].is_whitespace()
                && line.get(column + 1).is_some_and(|c| !c.is_whitespace());
            !single
        }
        _ => false,
    }
}

pub fn caret_alpha(mode: &str, idle: f64) -> f32 {
    match mode {
        "solid" => 1.0,
        _ if !(0.5..=30.0).contains(&idle) => 1.0,
        "blink" => ((idle - 0.5) % 1.2 < 0.6) as u8 as f32,
        "phase" | "expand" | "smooth" => 0.5 + 0.5 * ((idle - 0.5) * std::f64::consts::TAU / 1.2).cos() as f32,
        _ => 1.0,
    }
}

pub fn caret_rect(style: &str, x: f32, top: f32, line_h: f32, char_w: f32) -> (Rect, bool) {
    let bottom = top + line_h - 1.0;
    match style {
        "block" => (Rect::from_min_max(pos2(x, top + 1.0), pos2(x + char_w, bottom)), true),
        "block-outline" => (Rect::from_min_max(pos2(x, top + 1.0), pos2(x + char_w, bottom)), false),
        "underline" => (Rect::from_min_max(pos2(x, bottom - 2.0), pos2(x + char_w, bottom)), true),
        "underline-thin" => (Rect::from_min_max(pos2(x, bottom - 1.0), pos2(x + char_w, bottom)), true),
        "line-thin" => (Rect::from_min_max(pos2(x - 0.5, top + 1.0), pos2(x + 0.5, bottom)), true),
        _ => (Rect::from_min_max(pos2(x - 0.75, top + 1.0), pos2(x + 0.75, bottom)), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_relative_to_the_cursor() {
        assert_eq!([0, 4, 5, 9].map(|l| line_label("relative", l, 4)), [Some(4), Some(5), Some(1), Some(5)]);
        assert_eq!(line_label("on", 4, 0), Some(5));
        assert_eq!(line_label("off", 4, 0), None);
    }

    #[test]
    fn picks_wrap_columns_heights_whitespace_and_carets() {
        assert_eq!(wrap_columns("off", 80, 120), None);
        assert_eq!(wrap_columns("on", 80, 120), Some(120));
        assert_eq!(wrap_columns("wordWrapColumn", 80, 120), Some(80));
        assert_eq!(wrap_columns("bounded", 80, 60), Some(60));
        assert_eq!(line_height(0.0, 16.0, 13.0), 20.0);
        assert_eq!(line_height(2.0, 16.0, 13.0), 26.0);
        assert_eq!(line_height(30.0, 16.0, 13.0), 30.0);
        assert_eq!(line_height(4.0, 16.0, 13.0), 52.0);
        let line: Vec<char> = "  a b  c ".chars().collect();
        let shown = |mode: &str| (0..line.len()).filter(|&c| line[c] == ' ' && show_whitespace(mode, &line, c, c == 0)).collect::<Vec<_>>();
        assert_eq!(shown("all"), [0, 1, 3, 5, 6, 8]);
        assert_eq!(shown("boundary"), [0, 1, 5, 6, 8]);
        assert_eq!(shown("trailing"), [8]);
        assert_eq!(shown("selection"), [0]);
        assert!(shown("none").is_empty());
        assert_eq!(caret_alpha("solid", 5.0), 1.0);
        assert_eq!((caret_alpha("blink", 0.6), caret_alpha("blink", 1.2)), (1.0, 0.0));
        assert_eq!(caret_rect("block", 10.0, 0.0, 20.0, 8.0).0.width(), 8.0);
        assert!(!caret_rect("block-outline", 10.0, 0.0, 20.0, 8.0).1);
        assert_eq!(caret_rect("underline", 10.0, 0.0, 20.0, 8.0).0.height(), 2.0);
    }
}
