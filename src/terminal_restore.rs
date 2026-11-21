use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, StdSyncHandler};

const STYLES: [(Flags, u8); 7] =
    [(Flags::BOLD, 1), (Flags::DIM, 2), (Flags::ITALIC, 3), (Flags::UNDERLINE, 4), (Flags::INVERSE, 7), (Flags::HIDDEN, 8), (Flags::STRIKEOUT, 9)];

fn push_color(out: &mut String, color: Color, base: u8) {
    match color {
        Color::Named(name) if (name as usize) < 8 => *out += &format!(";{}", base + name as u8),
        Color::Named(name) if (name as usize) < 16 => *out += &format!(";{}", base + 52 + name as u8),
        Color::Indexed(i) => *out += &format!(";{};5;{i}", base + 8),
        Color::Spec(rgb) => *out += &format!(";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b),
        Color::Named(_) => {}
    }
}

fn style(cell: &Cell) -> (Color, Color, Flags) {
    let mask = STYLES.iter().fold(Flags::empty(), |all, (flag, _)| all | *flag);
    (cell.fg, cell.bg, cell.flags & mask)
}

fn sgr((fg, bg, flags): (Color, Color, Flags)) -> String {
    let mut out = "\x1b[0".to_owned();
    for (flag, code) in STYLES.iter().filter(|(flag, _)| flags.contains(*flag)) {
        out += &format!(";{code}");
    }
    push_color(&mut out, fg, 30);
    push_color(&mut out, bg, 40);
    out + "m"
}

fn blank(cell: &Cell) -> bool {
    cell.c == ' ' && cell.bg == Color::Named(NamedColor::Background) && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
}

pub fn serialize<T: EventListener>(term: &mut Term<T>, limit: usize) -> String {
    if term.mode().contains(TermMode::ALT_SCREEN) {
        let alt = term.grid().clone();
        term.swap_alt();
        let text = serialize(term, limit);
        term.swap_alt();
        *term.grid_mut() = alt;
        return text;
    }
    let (top, last, grid) = (term.topmost_line().0, term.last_column().0, term.grid());
    let empty = |line: i32| (0..=last).all(|c| blank(&grid[Line(line)][Column(c)]));
    let mut bottom = term.bottommost_line().0;
    while bottom >= top && empty(bottom) {
        bottom -= 1;
    }
    let mut first = top.max(bottom + 1 - limit as i32);
    while first <= bottom && empty(first) {
        first += 1;
    }
    let plain = (Color::Named(NamedColor::Foreground), Color::Named(NamedColor::Background), Flags::empty());
    let spacer = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
    let (mut out, mut current) = (String::new(), plain);
    for line in first..=bottom {
        let row = &grid[Line(line)];
        let wrapped = line < bottom && row[Column(last)].flags.contains(Flags::WRAPLINE);
        let end = if wrapped { last + 1 } else { (0..=last).rev().find(|&c| !blank(&row[Column(c)])).map_or(0, |c| c + 1) };
        for cell in (0..end).map(|c| &row[Column(c)]).filter(|cell| !cell.flags.intersects(spacer)) {
            if style(cell) != current {
                current = style(cell);
                out += &sgr(current);
            }
            out.push(cell.c);
            out.extend(cell.zerowidth().into_iter().flatten());
        }
        if !wrapped {
            out += "\r\n";
        }
    }
    if current != plain {
        out += "\x1b[0m";
    }
    out
}

pub fn print<T: EventListener>(term: &mut Term<T>, text: &str) {
    Processor::<StdSyncHandler>::default().advance(term, text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Rgb;

    struct Size(usize, usize);

    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.0
        }

        fn screen_lines(&self) -> usize {
            self.0
        }

        fn columns(&self) -> usize {
            self.1
        }
    }

    fn term(text: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config { scrolling_history: 200, ..Default::default() }, &Size(5, 10), VoidListener);
        print(&mut term, text);
        term
    }

    fn find(term: &Term<VoidListener>, c: char) -> Cell {
        let (top, bottom) = (term.topmost_line().0, term.bottommost_line().0);
        let cells = (top..=bottom).flat_map(|l| (0..term.columns()).map(move |col| (l, col)));
        cells.map(|(l, col)| term.grid()[Line(l)][Column(col)].clone()).find(|cell| cell.c == c).unwrap()
    }

    #[test]
    fn keeps_colours_wide_characters_and_wrapped_lines() {
        let mut original = term("\x1b[31mred\x1b[0m plain\r\n\x1b[1;38;5;200mX\x1b[48;2;1;2;3m漢字\x1b[0mq\u{301}\r\nabcdefghijKLM\r\n\x1b[7m zz \x1b[0m\r\n1\r\n2\r\n3\r\n");
        let text = serialize(&mut original, 1000);
        assert!(text.starts_with("\x1b[0;31mred\x1b[0m plain\r\n"), "{text:?}");
        assert!(text.contains("abcdefghijKLM\r\n"));
        let mut restored = term(&text);
        assert_eq!(serialize(&mut restored, 1000), text);
        let wide = find(&restored, '漢');
        assert!(wide.flags.contains(Flags::WIDE_CHAR | Flags::BOLD));
        assert_eq!((wide.fg, wide.bg), (Color::Indexed(200), Color::Spec(Rgb { r: 1, g: 2, b: 3 })));
        assert_eq!(find(&restored, 'q').zerowidth(), Some(&['\u{301}'][..]));
        assert!(find(&restored, 'z').flags.contains(Flags::INVERSE));
        assert_eq!(find(&restored, 'r').fg, Color::Named(NamedColor::Red));
    }

    #[test]
    fn trims_to_the_scrollback_setting_and_skips_the_alternate_screen() {
        let lines: String = (0..50).map(|i| format!("line {i}\r\n")).collect();
        let mut long = term(&lines);
        let text = serialize(&mut long, 10);
        assert_eq!(text.lines().count(), 10);
        assert!(text.starts_with("line 40\r\n") && text.ends_with("line 49\r\n"));
        let mut full = term("normal\r\n\x1b[?1049hfullscreen");
        let text = serialize(&mut full, 1000);
        assert_eq!(text, "normal\r\n");
        assert!(full.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(find(&full, 'f').c, 'f');
        assert_eq!(serialize(&mut term(""), 1000), "");
    }
}
