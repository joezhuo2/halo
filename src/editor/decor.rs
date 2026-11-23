use super::buffer::{After, Document, Kind, Sel};
use super::intel::{Lang, Want, popup_frame, to_lsp};
use super::{Editors, Tab};
use crate::lsp;
use crate::theme::Theme;
use crate::ui;
use eframe::egui::ecolor::Hsva;
use eframe::egui::widgets::color_picker::{Alpha, color_picker_hsva_2d};
use eframe::egui::{self, Area, Color32, CornerRadius, Id, Key, Order, Rect, Stroke, StrokeKind, vec2};
use serde_json::{Value, json};
use std::path::Path;

pub(super) type Swatch = (usize, usize, [f32; 4]);
pub(super) const SWATCH: &str = "  ";

pub(super) struct Picker {
    pub doc: u64,
    group: usize,
    from: usize,
    text: String,
    original: String,
    hsva: Hsva,
    version: u64,
    changed: bool,
    asked: bool,
    rect: Rect,
    server: bool,
    fresh: bool,
}

fn ext(path: Option<&Path>) -> String {
    path.and_then(Path::extension).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub(super) fn builtin(path: Option<&Path>) -> Option<bool> {
    match ext(path).as_str() {
        "md" | "markdown" => Some(true),
        "css" | "scss" | "less" | "sass" | "styl" | "stylus" | "html" | "htm" | "vue" | "svelte" | "jsx" | "tsx" | "json" | "jsonc" => Some(false),
        _ => None,
    }
}

pub(super) fn prose(path: Option<&Path>) -> bool {
    path.is_none() || matches!(ext(path).as_str(), "txt" | "text" | "md" | "markdown")
}

pub(super) fn swatches(text: &str, line: usize, lang: &Lang, builtin: Option<bool>) -> Vec<Swatch> {
    if let Some(colors) = &lang.colors {
        let col = |units| lsp::utf16_to_char(text.chars(), units);
        let from = colors.partition_point(|c| c.start.0 < line);
        let end = |c: &lsp::ColorInfo| if c.end.0 == line { col(c.end.1) } else { text.chars().count() };
        return colors[from..].iter().take_while(|c| c.start.0 == line).map(|c| (col(c.start.1), end(c), c.rgba)).filter(|s| s.0 < s.1).collect();
    }
    let Some(markdown) = builtin else { return vec![] };
    scan(text, markdown).into_iter().map(|(a, b, c)| (text[..a].chars().count(), text[..b].chars().count(), c)).collect()
}

pub(super) fn scan(text: &str, markdown: bool) -> Vec<(usize, usize, [f32; 4])> {
    let bytes = text.as_bytes();
    let word = |at: usize| at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' || bytes[at] == b'-');
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        let free = i == 0 || !(word(i - 1) || bytes[i - 1] == b'&' || bytes[i - 1] == b'#');
        let end = if !free {
            None
        } else if bytes[i] == b'#' {
            let n = bytes[i + 1..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
            let issue = markdown && n <= 4 && bytes[i + 1..i + 1 + n].iter().all(u8::is_ascii_digit);
            (matches!(n, 3 | 4 | 6 | 8) && !word(i + 1 + n) && !issue).then_some(i + 1 + n)
        } else if ["rgba(", "rgb(", "hsla(", "hsl("].iter().any(|f| bytes[i..].len() >= f.len() && bytes[i..i + f.len()].eq_ignore_ascii_case(f.as_bytes())) {
            text[i..].find(')').map(|at| i + at + 1)
        } else {
            None
        };
        match end.and_then(|end| Some((end, parse(&text[i..end])?))) {
            Some((end, color)) => {
                out.push((i, end, color));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

fn unit(text: &str, scale: f32) -> Option<f32> {
    match text.strip_suffix('%') {
        Some(percent) => Some(percent.parse::<f32>().ok()? / 100.0),
        None => Some(text.parse::<f32>().ok()? / scale),
    }
}

fn split(value: &str) -> Option<(String, &str, Vec<&str>, Option<&str>)> {
    let open = value.find('(')?;
    let inner = value[open + 1..].strip_suffix(')')?;
    let (main, slash) = inner.split_once('/').map_or((inner, None), |(m, a)| (m, Some(a.trim())));
    let mut parts: Vec<&str> = main.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    let alpha = if slash.is_none() && parts.len() == 4 { parts.pop() } else { slash };
    (parts.len() == 3).then(|| (value[..open].to_ascii_lowercase(), inner, parts, alpha))
}

pub(super) fn parse(value: &str) -> Option<[f32; 4]> {
    if let Some(hex) = value.strip_prefix('#') {
        if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digits: Vec<f32> = match hex.len() {
            3 | 4 => hex.chars().map(|c| c.to_digit(16).unwrap_or(0) as f32 * 17.0).collect(),
            _ => (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0) as f32).collect(),
        };
        return Some([digits[0] / 255.0, digits[1] / 255.0, digits[2] / 255.0, digits.get(3).map_or(1.0, |a| a / 255.0)]);
    }
    let (name, _, parts, alpha) = split(value)?;
    let a = alpha.map_or(Some(1.0), |a| unit(a, 1.0))?;
    let [r, g, b] = match name.as_str() {
        "rgb" | "rgba" => [unit(parts[0], 255.0)?, unit(parts[1], 255.0)?, unit(parts[2], 255.0)?],
        "hsl" | "hsla" => from_hsl(parts[0].trim_end_matches("deg").parse().ok()?, unit(parts[1], 100.0)?.clamp(0.0, 1.0), unit(parts[2], 100.0)?.clamp(0.0, 1.0)),
        _ => return None,
    };
    Some([r, g, b, a].map(|v| v.clamp(0.0, 1.0)))
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn num(v: f32, places: usize) -> String {
    let text = format!("{v:.places$}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
    if text == "-0" { "0".into() } else { text.into() }
}

pub(super) fn format(original: &str, [r, g, b, a]: [f32; 4]) -> Option<String> {
    if let Some(hex) = original.strip_prefix('#') {
        let mut bytes = vec![byte(r), byte(g), byte(b)];
        if matches!(hex.len(), 4 | 8) || byte(a) < 255 {
            bytes.push(byte(a));
        }
        let text: String = match hex.len() <= 4 && bytes.iter().all(|b| b % 17 == 0) {
            true => bytes.iter().map(|b| format!("{:x}", b / 17)).collect(),
            false => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        };
        return Some(format!("#{}", if hex.bytes().any(|b| b.is_ascii_uppercase()) { text.to_uppercase() } else { text }));
    }
    let (name, inner, parts, alpha) = split(original)?;
    let pct = |at: usize| parts[at].ends_with('%');
    let values = match name.as_str() {
        "rgb" | "rgba" => [r, g, b].iter().enumerate().map(|(at, v)| if pct(at) { format!("{}%", num(v * 100.0, 0)) } else { num(v * 255.0, 0) }).collect::<Vec<_>>(),
        "hsl" | "hsla" => {
            let (h, s, l) = to_hsl([r, g, b]);
            let deg = if parts[0].ends_with("deg") { "deg" } else { "" };
            vec![format!("{}{deg}", num(h.round() % 360.0, 0)), format!("{}%", num(s * 100.0, 0)), format!("{}%", num(l * 100.0, 0))]
        }
        _ => return None,
    };
    let sep = if inner.contains(", ") { ", " } else if inner.contains(',') { "," } else { " " };
    let mut body = values.join(sep);
    if alpha.is_some() || byte(a) < 255 {
        let shown = if alpha.is_some_and(|t| t.ends_with('%')) { format!("{}%", num(a * 100.0, 0)) } else { num(a, 2) };
        body += &if sep != " " && !inner.contains('/') { format!("{sep}{shown}") } else { format!(" / {shown}") };
    }
    Some(format!("{}({body})", &original[..original.find('(')?]))
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

fn to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((max + min) / 2.0, max - min);
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h * 60.0, d / (1.0 - (2.0 * l - 1.0).abs()), l)
}

fn shape(text: &str) -> (String, usize) {
    match text.strip_prefix('#') {
        Some(hex) => ("#".into(), hex.len()),
        None => (text.split('(').next().unwrap_or_default().trim_end_matches(['a', 'A']).to_ascii_lowercase(), 0),
    }
}

pub(super) fn paint_swatch(painter: &egui::Painter, theme: &Theme, rect: Rect, c: [f32; 4]) {
    let [r, g, b, a] = c.map(byte);
    painter.rect(rect, CornerRadius::same(2), Color32::from_rgba_unmultiplied(r, g, b, a), Stroke::new(1.0, ui::with_alpha(theme.text, 0.45)), StrokeKind::Outside);
}

pub(super) fn confusable(c: char) -> Option<char> {
    const PAIRS: &str = "аaеeоoрpсcуyхxіiјjѕsԁdһhԛqԝwАAВBЕEКKМMНHОOРPСCТTХXІIЈJЅSοoνvαaρpιiΑAΒBΕEΖZΗHΙIΚKΜMΝNΟOΡPΤTΥYΧX";
    Some(match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0)?,
        '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '\u{2018}' | '\u{2019}' | '\u{2032}' => '\'',
        '\u{201C}' | '\u{201D}' | '\u{2033}' => '"',
        '\u{201A}' => ',',
        '\u{037E}' => ';',
        '\u{2039}' => '<',
        '\u{203A}' => '>',
        _ => {
            let mut chars = PAIRS.chars();
            std::iter::from_fn(|| Some((chars.next()?, chars.next()?))).find(|p| p.0 == c)?.1
        }
    })
}

pub(super) fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180E}' | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}'
    )
}

fn foreign(chars: &[char], at: usize) -> bool {
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    if !word(&chars[at]) {
        return false;
    }
    let from = chars[..at].iter().rposition(|c| !word(c)).map_or(0, |p| p + 1);
    let to = chars[at..].iter().position(|c| !word(c)).map_or(chars.len(), |p| at + p);
    let found = &chars[from..to];
    !found.iter().any(char::is_ascii) && found.iter().any(|c| confusable(*c).is_none())
}

pub(super) fn flags(text: &str, invisibles: bool, ambiguous: bool, first: bool) -> Vec<(usize, char, Option<char>)> {
    if text.is_ascii() {
        return vec![];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    for (at, &c) in chars.iter().enumerate().filter(|c| !c.1.is_ascii()) {
        let joiner = c == '\u{200D}' && at > 0 && chars[at - 1] >= '\u{2600}';
        if invisible(c) {
            if invisibles && !(c == '\u{FEFF}' && at == 0 && first) && !joiner {
                out.push((at, c, None));
            }
        } else if let Some(ascii) = confusable(c).filter(|_| ambiguous && !foreign(&chars, at)) {
            out.push((at, c, Some(ascii)));
        }
    }
    out
}

pub(super) fn describe(c: char, ascii: Option<char>) -> String {
    match ascii {
        Some(a) => format!(
            "The character U+{:04X} \"{c}\" could be confused with the ASCII character U+{:04X} \"{a}\", which is more common in source code.",
            c as u32, a as u32
        ),
        None => format!("The character U+{:04X} is invisible.", c as u32),
    }
}

fn recolor(buf: &mut Document, sels: &mut Vec<Sel>, p: &mut Picker, text: String) {
    if text == p.text || buf.read_only {
        return;
    }
    if p.changed {
        buf.undo(sels);
    }
    let end = p.from + p.original.chars().count();
    p.changed = text != p.original && buf.apply(sels, vec![(p.from, end, text.clone())], Kind::Other, After::Map);
    p.text = if p.changed { text } else { p.original.clone() };
    p.version = buf.version;
}

impl Editors {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn picker_ui(&mut self, ctx: &egui::Context, theme: &Theme, (g, id): (usize, u64), buf: &mut Document, tab: &mut Tab, opened: Option<(usize, usize, [f32; 4], Rect)>, server: bool) {
        if let Some((from, to, rgba, rect)) = opened {
            let text = buf.text_of(from, to);
            let hsva = Hsva::from_srgba_unmultiplied(rgba.map(byte));
            self.intel.picker = Some(Picker { doc: id, group: g, from, original: text.clone(), text, hsva, version: buf.version, changed: false, asked: true, rect, server, fresh: true });
        }
        let Some(p) = self.intel.picker.as_mut().filter(|p| p.doc == id && p.group == g) else { return };
        if p.version != buf.version {
            self.intel.picker = None;
            return;
        }
        let mut changed = false;
        let area = Area::new(Id::new("color_picker")).order(Order::Foreground).fixed_pos(p.rect.left_bottom() + vec2(-4.0, 6.0)).show(ctx, |ui| {
            popup_frame(theme).inner_margin(8.0).show(ui, |ui| changed = color_picker_hsva_2d(ui, &mut p.hsva, Alpha::OnlyBlend));
        });
        let away = ctx.input(|i| i.pointer.any_pressed()) && !ctx.pointer_interact_pos().is_some_and(|pos| area.response.rect.contains(pos));
        if ctx.input(|i| i.key_pressed(Key::Escape)) || (away && !std::mem::take(&mut p.fresh)) {
            self.intel.picker = None;
            return;
        }
        p.fresh = false;
        let rgba = p.hsva.to_srgba_unmultiplied().map(|v| v as f32 / 255.0);
        if changed {
            p.asked = !p.server;
            if let Some(text) = format(&p.original, rgba) {
                recolor(buf, &mut tab.sels, p, text);
            }
        }
        if !p.asked && !ctx.input(|i| i.pointer.any_down())
            && let Some(path) = buf.path.clone()
        {
            p.asked = true;
            let range = lsp::range_json(to_lsp(buf, p.from), to_lsp(buf, p.from + p.text.chars().count()));
            let color = json!({"red": rgba[0], "green": rgba[1], "blue": rgba[2], "alpha": rgba[3]});
            let params = json!({"textDocument": {"uri": lsp::uri(&path)}, "color": color, "range": range});
            if let Some(request) = self.lsp.request(&path, "textDocument/colorPresentation", params) {
                self.intel.wants.push((request, Want::Present { doc: id, version: p.version }));
            }
        }
    }

    pub(super) fn presented(&mut self, doc: u64, version: u64, value: &Value) {
        let Some(d) = self.docs.get_mut(&doc).filter(|d| d.buf.version == version) else { return };
        let Some(p) = self.intel.picker.as_mut().filter(|p| p.doc == doc && p.version == version) else { return };
        let Some(tab) = self.groups.get_mut(p.group).and_then(|group| group.tabs.iter_mut().find(|tab| tab.doc == doc)) else { return };
        let options = lsp::presentations(value);
        let own = format(&p.original, [0.0; 4]).is_some();
        if let Some(text) = options.iter().find(|o| shape(o) == shape(&p.original)).or(options.first().filter(|_| !own)) {
            recolor(&mut d.buf, &mut tab.sels, p, text.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.003)
    }

    #[test]
    fn parses_and_formats_colors_keeping_their_format() {
        for text in [
            "#fff", "#FFF8", "#1e90ff", "#1E90FF80", "rgb(30, 144, 255)", "rgba(30,144,255,0.5)", "rgb(30 144 255 / 50%)", "rgb(10% 20% 30%)",
            "hsl(210, 50%, 40%)", "hsla(210deg 50% 40% / 0.25)", "RGB(0, 0, 0)",
        ] {
            let color = parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(format(text, color).as_deref(), Some(text), "{text}");
        }
        assert!(close(parse("#1e90ff").unwrap(), [30.0 / 255.0, 144.0 / 255.0, 1.0, 1.0]));
        assert!(close(parse("hsl(0, 100%, 50%)").unwrap(), [1.0, 0.0, 0.0, 1.0]));
        assert!(close(parse("rgba(255, 0, 0, 50%)").unwrap(), [1.0, 0.0, 0.0, 0.5]));
        assert!(close(parse("hsl(120 100% 25% / .5)").unwrap(), [0.0, 0.5, 0.0, 0.5]));
        assert_eq!(parse("#ggg"), None);
        assert_eq!(parse("#12345"), None);
        assert_eq!(parse("rgb(1, 2)"), None);
        let red = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(format("#abc", [0.1, 0.2, 0.3, 1.0]).as_deref(), Some("#1a334d"));
        assert_eq!(format("#abc", red).as_deref(), Some("#f00"));
        assert_eq!(format("#abc", [1.0, 0.0, 0.0, 0.5]).as_deref(), Some("#ff000080"));
        assert_eq!(format("#ABCD", red).as_deref(), Some("#F00F"));
        assert_eq!(format("rgb(1, 2, 3)", [1.0, 0.0, 0.0, 0.5]).as_deref(), Some("rgb(255, 0, 0, 0.5)"));
        assert_eq!(format("rgb(1 2 3)", [1.0, 0.0, 0.0, 0.5]).as_deref(), Some("rgb(255 0 0 / 0.5)"));
        assert_eq!(format("hsl(0deg, 0%, 0%)", [0.0, 0.0, 1.0, 1.0]).as_deref(), Some("hsl(240deg, 100%, 50%)"));
        assert_eq!(format("blue", red), None);
    }

    #[test]
    fn scans_a_css_sample_for_colors() {
        let line = "a { color: #f00; background: rgba(0, 0, 255, .5) url(#x); border: 1px solid HSL(120 100% 25%); fill: #abcd12; }";
        let found: Vec<&str> = scan(line, false).iter().map(|(a, b, _)| &line[*a..*b]).collect();
        assert_eq!(found, ["#f00", "rgba(0, 0, 255, .5)", "HSL(120 100% 25%)", "#abcd12"]);
        assert!(scan("#header, a#bcd, &#123; #12345 #abcdefg srgb(1,2,3)", false).is_empty());
        assert_eq!(scan("fixes #123 and #fff", true).len(), 1);
        assert_eq!(scan("fixes #123", false).len(), 1);
    }

    #[test]
    fn classifies_invisible_and_confusable_characters() {
        assert!(['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{00AD}', '\u{202E}', '\u{2066}'].iter().all(|c| invisible(*c)));
        assert!(!invisible('a') && !invisible(' ') && !invisible('é'));
        assert_eq!(['а', 'О', 'ο', 'ν', 'Ａ', '–', '“', '’', '\u{00A0}', 'é'].map(confusable), [
            Some('a'), Some('O'), Some('o'), Some('v'), Some('A'), Some('-'), Some('"'), Some('\''), Some(' '), None
        ]);
        assert_eq!(flags("x\u{200B}y", true, true, false), [(1, '\u{200B}', None)]);
        assert!(flags("x\u{200B}y", false, true, false).is_empty());
        assert!(flags("\u{FEFF}abc", true, true, true).is_empty());
        assert_eq!(flags("a\u{FEFF}bc", true, true, true).len(), 1);
        assert!(flags("👩\u{200D}💻", true, true, false).is_empty());
        assert_eq!(describe('а', Some('a')), "The character U+0430 \"а\" could be confused with the ASCII character U+0061 \"a\", which is more common in source code.");
        assert_eq!(describe('\u{200B}', None), "The character U+200B is invisible.");
    }

    #[test]
    fn clicking_a_swatch_opens_the_picker_and_its_edits_undo_in_one_step() {
        use crate::command::Command;
        use crate::settings::Settings;
        use eframe::egui::{Event, FontId, PointerButton, pos2};
        let frame = super::super::tests::frame;
        let (ctx, settings, mut editors) = (egui::Context::default(), Settings::default(), Editors::new());
        editors.exec(&Command::NewFile, &settings, &ctx);
        frame(&ctx, &mut editors, vec![]);
        frame(&ctx, &mut editors, vec![Event::Paste("a { color: #f00; }\n".into())]);
        editors.docs.values_mut().next().unwrap().lang.colors = Some(vec![lsp::ColorInfo { start: (0, 11), end: (0, 15), rgba: [1.0, 0.0, 0.0, 1.0] }]);
        frame(&ctx, &mut editors, vec![]);
        let font = FontId::monospace(settings.font_size);
        let (char_w, row_h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'm'), f.row_height(&font)));
        let pos = pos2(8.0 * char_w + 6.0 + 11.0 * char_w + 2.0 + settings.font_size * 0.35, super::super::TAB_H + 22.0 + 0.5 * (row_h * 1.25).round());
        let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&ctx, &mut editors, vec![Event::PointerMoved(pos)]);
        frame(&ctx, &mut editors, vec![button(true)]);
        frame(&ctx, &mut editors, vec![button(false)]);
        let p = editors.intel.picker.as_ref().expect("a click on the swatch opens the picker");
        assert_eq!((p.from, p.text.as_str(), p.server), (11, "#f00", true));
        frame(&ctx, &mut editors, vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        assert!(editors.intel.picker.is_none());

        let mut buf = Document::from_text("a { color: #f00; }\n", "t".into());
        let mut sels = vec![Sel::caret(0)];
        let rect = Rect::NOTHING;
        let mut p = Picker { doc: 1, group: 0, from: 11, text: "#f00".into(), original: "#f00".into(), hsva: Hsva::default(), version: buf.version, changed: false, asked: true, rect, server: false, fresh: false };
        recolor(&mut buf, &mut sels, &mut p, "#0f0".into());
        recolor(&mut buf, &mut sels, &mut p, "#00ff0080".into());
        assert_eq!(buf.rope.to_string(), "a { color: #00ff0080; }\n");
        assert_eq!(p.version, buf.version);
        buf.undo(&mut sels);
        assert_eq!(buf.rope.to_string(), "a { color: #f00; }\n");
    }

    #[test]
    fn skips_confusables_in_words_of_another_script() {
        assert_eq!(flags("let vаlue = 1;", true, true, false), [(5, 'а', Some('a'))]);
        assert!(flags("let привет = 1;", true, true, false).is_empty());
        assert_eq!(flags("x = \"оса\";", true, true, false).len(), 3);
        assert_eq!(flags("a – b", true, true, false), [(2, '–', Some('-'))]);
        assert!(flags("a – b", true, false, false).is_empty());
        assert!(flags("Привет, мир", true, true, false).is_empty());
    }
}
