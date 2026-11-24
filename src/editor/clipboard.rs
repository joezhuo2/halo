use super::buffer::{Document, Sel};
use super::highlight::Highlighter;
use crate::{settings::Settings, theme::Theme};
use eframe::egui::Context;
use std::sync::Mutex;

static HISTORY: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(super) fn history() -> Vec<String> {
    HISTORY.lock().map(|h| h.clone()).unwrap_or_default()
}

fn remember(text: &str) {
    if let Ok(mut history) = HISTORY.lock()
        && !text.is_empty()
    {
        history.retain(|t| t != text);
        history.insert(0, text.to_owned());
        history.truncate(20);
    }
}

pub(super) fn copy(ctx: &Context, buf: &Document, sels: &[Sel], hl: &mut Highlighter, lang: &super::Lang, theme: &Theme, settings: &Settings) {
    let ranges = buf.copy_ranges(sels);
    let text = ranges.iter().map(|r| buf.text_of(r.start, r.end)).collect::<Vec<_>>().join("\n");
    remember(&text);
    #[cfg(windows)]
    {
        let rich = settings.copy_with_syntax_highlighting && text.len() <= 64 * 1024;
        let mut missing = false;
        let font = if rich { native::font(ctx) } else { String::new() };
        let mut tokens = std::collections::HashMap::new();
        let html = rich.then(|| {
            if let Some(line) = buf.dirty_from { hl.invalidate(line); }
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(4);
            rich::html(&buf.rope, &ranges, &font, settings.font_size, theme, |line| {
                if hl.get(line).is_none() && let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                    hl.ensure(&buf.rope, line..line + 1, theme, left);
                }
                let spans = hl.get(line).cloned();
                missing |= spans.is_none() && hl.language != "Plain Text";
                if let Some(row) = lang.tokens.get(&line).filter(|_| settings.semantic_highlighting) {
                    tokens.entry(line).or_insert_with(|| row.clone());
                }
                rich::colors(&buf.rope, line, spans, tokens.get(&line), theme)
            })
        });
        if let Some((owner, sequence)) = native::write(&text, html.as_deref()) {
            if missing {
                let (rope, language, theme, size) = (buf.rope.clone(), hl.language.clone(), theme.clone(), settings.font_size);
                std::thread::spawn(move || {
                    let mut hl = Highlighter::named(&language, &rope);
                    let html = rich::html(&rope, &ranges, &font, size, &theme, |line| {
                        if hl.get(line).is_none() {
                            hl.ensure(&rope, line..line + 64, &theme, std::time::Duration::MAX);
                        }
                        rich::colors(&rope, line, hl.get(line).cloned(), tokens.get(&line), &theme)
                    });
                    native::finish(owner, sequence, &html);
                });
            }
            return;
        }
    }
    #[cfg(not(windows))]
    let _ = (hl, lang, theme, settings);
    ctx.copy_text(text);
}

#[cfg(any(windows, test))]
mod rich {
    use super::super::highlight::Spans;
    use crate::theme::Theme;
    use eframe::egui::Color32;
    use std::fmt::Write;
    use std::ops::Range;
    use ropey::Rope;

    fn escaped(out: &mut String, text: &str) {
        for c in text.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                _ => out.push(c),
            }
        }
    }

    fn color(c: Color32) -> String { format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()) }

    #[cfg(windows)]
    pub(super) fn colors(rope: &Rope, line: usize, base: Option<Spans>, tokens: Option<&Vec<(usize, usize, u8)>>, theme: &Theme) -> Option<Spans> {
        let Some(tokens) = tokens else { return base };
        let text: String = rope.line(line).chars().take(super::super::MAX_COLS).collect();
        let over = super::super::intel::semantic_colors(&text, tokens, theme);
        let format = eframe::egui::TextFormat::default();
        let job = super::super::job(&text, 0..text.len(), base.as_ref(), &format.font_id, theme.text, &over, &[], &format);
        Some(std::sync::Arc::new(job.sections.iter().map(|s| (s.byte_range.end.0 - s.byte_range.start.0, s.format.color)).collect()))
    }

    pub(super) fn html(rope: &Rope, ranges: &[Range<usize>], font: &str, size: f32, theme: &Theme, mut spans: impl FnMut(usize) -> Option<Spans>) -> String {
        let mut body = String::from("<pre style=\"margin:0;white-space:pre;font-family:'");
        escaped(&mut body, &font.replace('\\', "\\\\").replace('\'', "\\'"));
        write!(body, "',monospace;font-size:{size}px;color:{};background-color:{}\">", color(theme.text), color(theme.bg)).unwrap();
        for (index, range) in ranges.iter().enumerate() {
            if index > 0 { body.push('\n'); }
            if range.is_empty() { continue; }
            let first = rope.char_to_line(range.start);
            let mut offset = rope.char_to_byte(range.start) - rope.line_to_byte(first);
            for (i, slice) in rope.slice(range.clone()).lines().enumerate() {
                if slice.len_bytes() == 0 { continue; }
                let text: std::borrow::Cow<'_, str> = slice.into();
                let mut end = 0;
                let mut from = 0;
                if let Some(spans) = spans(first + i) {
                    for &(len, shade) in spans.iter() {
                        end += len;
                        let to = end.saturating_sub(offset).min(text.len());
                        if to > from && text.is_char_boundary(to) {
                            write!(body, "<span style=\"color:{}\">", color(shade)).unwrap();
                            escaped(&mut body, &text[from..to]);
                            body.push_str("</span>");
                            from = to;
                        }
                        if from == text.len() { break; }
                    }
                }
                escaped(&mut body, &text[from..]);
                offset = 0;
            }
        }
        body.push_str("</pre>");
        let before = "<html><body><!--StartFragment-->";
        let after = "<!--EndFragment--></body></html>";
        let header = |start, end, first, last| format!("Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{first:010}\r\nEndFragment:{last:010}\r\n");
        let start = header(0, 0, 0, 0).len();
        let first = start + before.len();
        let last = first + body.len();
        format!("{}{before}{body}{after}", header(start, last + after.len(), first, last))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::super::{Document, Sel};

        #[test]
        fn clipboard_offsets_are_utf8_bytes_and_selections_match_plain_text() {
            let d = Document::from_text("let élève = \"<&>\";\nlast", "a.rs".into());
            let theme = Theme::dark();
            let ranges = d.copy_ranges(&[Sel::new(0, 3), Sel::new(4, 17)]);
            let html = html(&d.rope, &ranges, "Joe's Mono", 14.0, &theme, |_| Some(std::sync::Arc::new(vec![(3, theme.syntax.keyword)])));
            let offsets: Vec<usize> = html.lines().skip(1).take(4).map(|s| s.split_once(':').unwrap().1.parse().unwrap()).collect();
            let [start, end, first, last] = offsets[..] else { panic!() };
            assert_eq!(end, html.len());
            assert!(html[start..first].ends_with("<!--StartFragment-->"));
            assert!(html[last..end].starts_with("<!--EndFragment-->"));
            let fragment = &html[first..last];
            assert!(fragment.starts_with("<pre") && fragment.ends_with("</pre>"));
            assert!(fragment.contains(&format!("color:{}\">let</span>\nélève = &quot;&lt;&amp;&gt;&quot;", color(theme.syntax.keyword))));
            let lines = d.copy_ranges(&[Sel::caret(0), Sel::caret(2), Sel::caret(d.len())]);
            assert_eq!(lines.iter().map(|r| d.text_of(r.start, r.end)).collect::<Vec<_>>().join("\n"), format!("{}\n", d.rope));
        }
    }
}

#[cfg(windows)]
mod native {
    use eframe::egui::{Context, FontFamily};
    use skrifa::{MetadataProvider, string::StringId};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::core::w;

    pub(super) fn font(ctx: &Context) -> String {
        ctx.fonts(|f| {
            let defs = f.definitions();
            let data = defs.font_data.get(defs.families.get(&FontFamily::Monospace)?.first()?)?;
            let face = skrifa::FontRef::from_index(&data.font, data.index).ok()?;
            face.localized_strings(StringId::FAMILY_NAME).english_or_first().map(|s| s.to_string())
        }).unwrap_or_else(|| "monospace".into())
    }

    fn set(format: u32, bytes: &[u8]) -> bool {
        unsafe {
            let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
            let ptr = GlobalLock(mem).cast::<u8>();
            if !ptr.is_null() {
                ptr.copy_from_nonoverlapping(bytes.as_ptr(), bytes.len());
                let _ = GlobalUnlock(mem);
                if SetClipboardData(format, Some(HANDLE(mem.0))).is_ok() { return true; }
            }
            let _ = GlobalFree(Some(mem));
            false
        }
    }

    fn set_html(text: &str) {
        let format = unsafe { RegisterClipboardFormatW(w!("HTML Format")) };
        if format != 0 {
            let mut bytes = text.as_bytes().to_vec();
            bytes.push(0);
            set(format, &bytes);
        }
    }

    pub(super) fn write(text: &str, html: Option<&str>) -> Option<(usize, u32)> {
        let bytes: Vec<u8> = text.replace('\n', "\r\n").encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        unsafe {
            let owner = GetActiveWindow();
            if owner.0.is_null() || OpenClipboard(Some(owner)).is_err() { return None; }
            let ok = EmptyClipboard().is_ok() && {
                if let Some(html) = html { set_html(html); }
                set(CF_UNICODETEXT.0.into(), &bytes)
            };
            let sequence = GetClipboardSequenceNumber();
            let _ = CloseClipboard();
            ok.then_some((owner.0 as usize, sequence))
        }
    }

    pub(super) fn finish(owner: usize, sequence: u32, html: &str) {
        let owner = HWND(owner as *mut _);
        unsafe {
            if OpenClipboard(Some(owner)).is_ok() {
                if GetClipboardSequenceNumber() == sequence && GetClipboardOwner().ok() == Some(owner) { set_html(html); }
                let _ = CloseClipboard();
            }
        }
    }
}
