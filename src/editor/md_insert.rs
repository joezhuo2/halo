use super::buffer::{Document, Sel};
use super::image_view;
use crate::ui;
use eframe::egui::{self, Pos2};
use std::path::{Component, Path, PathBuf};

pub fn free_name(dir: &Path) -> PathBuf {
    (0..).map(|n| dir.join(if n == 0 { "image.png".to_owned() } else { format!("image-{n}.png") })).find(|p| !p.exists()).unwrap_or_default()
}

fn relative(dir: &Path, file: &Path) -> Option<String> {
    let (from, to): (Vec<Component>, Vec<Component>) = (dir.components().collect(), file.components().collect());
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return None;
    }
    let up = from[common..].iter().map(|_| "..".to_owned());
    Some(up.chain(to[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned())).collect::<Vec<_>>().join("/"))
}

pub fn link(dir: Option<&Path>, file: &Path) -> String {
    let path = dir.and_then(|d| relative(d, file)).unwrap_or_else(|| file.to_string_lossy().replace('\\', "/"));
    let target = if path.contains(|c: char| c.is_whitespace() || c == '(' || c == ')') { format!("<{path}>") } else { path };
    let image = image_view::is_image(file);
    let name = if image { file.file_stem() } else { file.file_name() }.unwrap_or_default().to_string_lossy();
    format!("{}[{}]({target})", if image { "!" } else { "" }, name.replace('[', "\\[").replace(']', "\\]"))
}

pub fn links(dir: Option<&Path>, files: &[PathBuf]) -> String {
    files.iter().map(|f| link(dir, f)).collect::<Vec<_>>().join(" ")
}

pub fn paste_image(buf: &mut Document, sels: &mut Vec<Sel>) {
    let Some(png) = clipboard_png() else { return };
    let Some(dir) = buf.path.as_deref().and_then(Path::parent) else {
        return ui::notify_warning("Save the file first to paste images");
    };
    let file = free_name(dir);
    match std::fs::write(&file, png) {
        Ok(()) => drop(buf.insert_text(sels, &link(Some(dir), &file))),
        Err(e) => ui::notify_error(format!("Could not save {}: {e}", file.display())),
    }
}

pub fn png_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    let decoder = image::codecs::bmp::BmpDecoder::new_without_file_header(std::io::Cursor::new(dib)).ok()?;
    let image = image::DynamicImage::from_decoder(decoder).ok()?;
    let mut png = std::io::Cursor::new(vec![]);
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

#[cfg(windows)]
fn clipboard_png() -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_DIB: u32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            return None;
        }
        let png = RegisterClipboardFormatW(windows::core::w!("PNG"));
        let format = [png, CF_DIB].into_iter().find(|f| *f != 0 && IsClipboardFormatAvailable(*f).is_ok())?;
        OpenClipboard(None).ok()?;
        let bytes = GetClipboardData(format).ok().and_then(|h| {
            let global = HGLOBAL(h.0);
            let data = GlobalLock(global);
            (!data.is_null()).then(|| {
                let bytes = std::slice::from_raw_parts(data as *const u8, GlobalSize(global)).to_vec();
                let _ = GlobalUnlock(global);
                bytes
            })
        });
        let _ = CloseClipboard();
        if format == png { bytes } else { png_from_dib(&bytes?) }
    }
}

#[cfg(not(windows))]
fn clipboard_png() -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
pub fn drop_pos(ctx: &egui::Context) -> Option<Pos2> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut point = POINT::default();
    if unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } >= 0 || unsafe { GetCursorPos(&mut point) }.is_err() {
        return None;
    }
    let inner = ctx.input(|i| i.viewport().inner_rect)?;
    Some(egui::pos2(point.x as f32, point.y as f32) / ctx.pixels_per_point() - inner.min.to_vec2())
}

#[cfg(not(windows))]
pub fn drop_pos(ctx: &egui::Context) -> Option<Pos2> {
    ctx.input(|i| i.modifiers.shift).then(|| ctx.pointer_latest_pos()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_next_free_image_name() {
        let dir = std::env::temp_dir().join(format!("halo-md-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir), dir.join("image.png"));
        std::fs::write(dir.join("image.png"), "").unwrap();
        std::fs::write(dir.join("image-1.png"), "").unwrap();
        assert_eq!(free_name(&dir), dir.join("image-2.png"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_relative_links_for_dropped_files() {
        let dir = Path::new("/w/docs");
        assert_eq!(link(Some(dir), Path::new("/w/docs/shot.png")), "![shot](shot.png)");
        assert_eq!(link(Some(dir), Path::new("/w/docs/img/a.PNG")), "![a](img/a.PNG)");
        assert_eq!(link(Some(dir), Path::new("/w/src/main.rs")), "[main.rs](../src/main.rs)");
        assert_eq!(link(Some(dir), Path::new("/w/docs/my notes.txt")), "[my notes.txt](<my notes.txt>)");
        assert_eq!(link(Some(dir), Path::new("/w/a [b].pdf")), "[a \\[b\\].pdf](<../a [b].pdf>)");
        assert_eq!(link(None, Path::new("/w/docs/x.md")), "[x.md](/w/docs/x.md)");
        assert_eq!(links(Some(dir), &["/w/docs/a.md".into(), "/w/docs/b.gif".into()]), "[a.md](a.md) ![b](b.gif)");
    }

    #[test]
    fn converts_clipboard_bitmaps_to_png() {
        let mut dib = vec![];
        for v in [40u32, 2, 1] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(24u16.to_le_bytes());
        dib.extend([0u8; 24]);
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        let png = png_from_dib(&dib).unwrap();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap().to_rgb8();
        assert_eq!((image.width(), image.height(), image.get_pixel(0, 0).0, image.get_pixel(1, 0).0), (2, 1, [255, 0, 0], [0, 255, 0]));
        assert!(png_from_dib(&[1, 2, 3]).is_none());
    }
}
