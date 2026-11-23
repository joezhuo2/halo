use eframe::egui::{Color32, ColorImage, TextureHandle};

const MAX_BODY: usize = 48 << 20;
const MAX_SIDE: usize = 8192;

#[derive(Debug, PartialEq)]
pub enum Found {
    Sixel(Vec<u8>),
    Iterm(Vec<u8>),
    Attributes,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Ground,
    Esc,
    Dcs,
    Osc,
    Csi,
    Body(bool),
    End(bool),
}

pub struct Scanner {
    state: State,
    held: Vec<u8>,
    body: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self { state: State::Ground, held: vec![], body: vec![] }
    }
}

const ITERM: &[u8] = b"1337;File=";

impl Scanner {
    fn bail(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.held);
        self.state = State::Ground;
    }

    fn finish(&mut self, iterm: bool, out: &[u8], found: &mut Vec<(usize, Found)>) {
        let body = std::mem::take(&mut self.body);
        if body.len() < MAX_BODY {
            found.push((out.len(), if iterm { Found::Iterm(body) } else { Found::Sixel(body) }));
        }
        self.state = State::Ground;
    }

    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>, found: &mut Vec<(usize, Found)>) {
        let mut i = 0;
        while i < input.len() {
            let b = input[i];
            match self.state {
                State::Ground if b == 0x1b => {
                    self.held.push(b);
                    self.state = State::Esc;
                }
                State::Ground => out.push(b),
                State::Body(iterm) => match b {
                    0x1b => self.state = State::End(iterm),
                    0x07 if iterm => self.finish(true, out, found),
                    0x18 | 0x1a => {
                        self.body.clear();
                        self.state = State::Ground;
                    }
                    _ if self.body.len() < MAX_BODY => self.body.push(b),
                    _ => {}
                },
                State::End(iterm) => {
                    self.finish(iterm, out, found);
                    if b != b'\\' {
                        continue;
                    }
                }
                _ if b == 0x1b => {
                    self.bail(out);
                    continue;
                }
                _ => {
                    self.held.push(b);
                    let tail = &self.held[2.min(self.held.len())..];
                    self.state = match (self.state, b) {
                        (State::Esc, b'P') => State::Dcs,
                        (State::Esc, b']') => State::Osc,
                        (State::Esc, b'[') => State::Csi,
                        (State::Dcs, b'q') => State::Body(false),
                        (State::Dcs, b'0'..=b'9' | b';') if tail.len() < 16 => State::Dcs,
                        (State::Osc, _) if tail == ITERM => State::Body(true),
                        (State::Osc, _) if ITERM.starts_with(tail) => State::Osc,
                        (State::Csi, b'0') if tail == b"0" => State::Csi,
                        (State::Csi, b'c') => {
                            found.push((out.len(), Found::Attributes));
                            State::Ground
                        }
                        _ => {
                            self.bail(out);
                            i += 1;
                            continue;
                        }
                    };
                    if matches!(self.state, State::Body(_) | State::Ground) {
                        self.held.clear();
                        self.body.clear();
                    }
                }
            }
            i += 1;
        }
    }
}

pub struct Picture {
    pub line: i64,
    pub column: usize,
    pub size: (f32, f32),
    pub image: Option<ColorImage>,
    pub texture: Option<TextureHandle>,
}

fn numbers(data: &[u8], i: &mut usize) -> Vec<usize> {
    let mut out = vec![0usize];
    while let Some(&b) = data.get(*i) {
        match b {
            b'0'..=b'9' => {
                let last = out.last_mut().unwrap();
                *last = last.saturating_mul(10).saturating_add((b - b'0') as usize);
            }
            b';' => out.push(0),
            _ => break,
        }
        *i += 1;
    }
    out
}

fn hls(h: usize, l: usize, s: usize) -> Color32 {
    let (h, l, s) = (((h + 240) % 360) as f32 / 60.0, l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
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
    let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(byte(r), byte(g), byte(b))
}

pub fn sixel(data: &[u8]) -> Option<ColorImage> {
    let mut palette: Vec<Color32> = (0..256).map(|i| if i < 16 { crate::terminal::xterm_color(i as u8) } else { Color32::GRAY }).collect();
    let mut rows: Vec<Vec<Color32>> = vec![];
    let (mut x, mut y, mut color, mut i) = (0usize, 0usize, 0usize, 0usize);
    let (mut width, mut height) = (0usize, 0usize);
    while i < data.len() {
        let b = data[i];
        i += 1;
        let mut count = 1;
        let bits = match b {
            b'"' => {
                let n = numbers(data, &mut i);
                if n.len() >= 4 {
                    (width, height) = (width.max(n[2].min(MAX_SIDE)), height.max(n[3].min(MAX_SIDE)));
                }
                continue;
            }
            b'#' => {
                let n = numbers(data, &mut i);
                color = n[0] % 256;
                if n.len() >= 5 {
                    let pct = |v: usize| (v.min(100) * 255 / 100) as u8;
                    palette[color] = if n[1] == 1 { hls(n[2], n[3], n[4]) } else { Color32::from_rgb(pct(n[2]), pct(n[3]), pct(n[4])) };
                }
                continue;
            }
            b'!' => {
                count = numbers(data, &mut i)[0].max(1);
                let Some(&next) = data.get(i) else { break };
                i += 1;
                next
            }
            b'$' => {
                x = 0;
                continue;
            }
            b'-' => {
                (x, y) = (0, y + 6);
                continue;
            }
            _ => b,
        };
        if !(0x3f..=0x7e).contains(&bits) {
            continue;
        }
        let bits = bits - 0x3f;
        let end = (x + count).min(MAX_SIDE);
        if bits != 0 && y + 6 <= MAX_SIDE {
            for k in (0..6).filter(|k| bits & (1 << k) != 0) {
                if rows.len() <= y + k {
                    rows.resize(y + k + 1, vec![]);
                }
                let row = &mut rows[y + k];
                if row.len() < end {
                    row.resize(end, Color32::TRANSPARENT);
                }
                row[x.min(end)..end].fill(palette[color]);
            }
        }
        x = end;
    }
    let width = width.max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let height = height.max(rows.len());
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    for (r, row) in rows.iter().enumerate().take(height) {
        pixels[r * width..r * width + row.len().min(width)].copy_from_slice(&row[..row.len().min(width)]);
    }
    Some(ColorImage::new([width, height], pixels))
}

fn base64(text: &[u8]) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(text.len() * 3 / 4), 0u32, 0);
    for v in text.iter().filter_map(|&c| value(c)) {
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn decode_file(bytes: &[u8]) -> Option<ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE as u32), Some(MAX_SIDE as u32));
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn extent(spec: Option<&str>, cell: f32, whole: f32) -> Option<f32> {
    let spec = spec.filter(|s| *s != "auto")?;
    if let Some(px) = spec.strip_suffix("px") {
        return px.parse().ok();
    }
    if let Some(pct) = spec.strip_suffix('%') {
        return pct.parse::<f32>().ok().map(|p| whole * p / 100.0);
    }
    spec.parse::<f32>().ok().map(|n| n * cell)
}

pub fn decode(found: Found, cell: (f32, f32), screen: (usize, usize)) -> Option<(ColorImage, (f32, f32))> {
    let (whole_w, whole_h) = (screen.0 as f32 * cell.0, screen.1 as f32 * cell.1);
    let (image, (w, h), keep) = match found {
        Found::Sixel(data) => {
            let image = sixel(&data)?;
            let size = (image.size[0] as f32, image.size[1] as f32);
            (image, size, true)
        }
        Found::Iterm(body) => {
            let colon = body.iter().position(|&b| b == b':')?;
            let args = String::from_utf8_lossy(&body[..colon]).into_owned();
            let arg = |key: &str| args.split(';').find_map(|a| a.strip_prefix(key)?.strip_prefix('='));
            if arg("inline") != Some("1") {
                return None;
            }
            let image = decode_file(&base64(&body[colon + 1..]))?;
            let natural = (image.size[0] as f32, image.size[1] as f32);
            let (w, h) = (extent(arg("width"), cell.0, whole_w), extent(arg("height"), cell.1, whole_h));
            let size = match (w, h) {
                (Some(w), None) => (w, natural.1 * w / natural.0),
                (None, Some(h)) => (natural.0 * h / natural.1, h),
                (Some(w), Some(h)) => (w, h),
                (None, None) => natural,
            };
            (image, size, arg("preserveAspectRatio") != Some("0"))
        }
        Found::Attributes => return None,
    };
    let (iw, ih) = (image.size[0] as f32, image.size[1] as f32);
    let (mut w, mut h) = if keep { let s = (w / iw).min(h / ih); (iw * s, ih * s) } else { (w, h) };
    if w > whole_w && whole_w > 0.0 {
        (w, h) = (whole_w, h * whole_w / w);
    }
    (w >= 1.0 && h >= 1.0).then_some((image, (w, h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> (Vec<u8>, Vec<(usize, Found)>) {
        let (mut scanner, mut out, mut found) = (Scanner::default(), vec![], vec![]);
        for chunk in chunks {
            scanner.feed(chunk, &mut out, &mut found);
        }
        (out, found)
    }

    #[test]
    fn strips_images_split_across_reads_and_keeps_other_sequences() {
        let (out, found) = scan(&[b"a\x1b[31mb\x1bP0;1", b";0q#0;2;100;0;0~~\x1b", b"\\c\x1b]1337;Fi", b"le=inline=1:QUJD\x07d\x1b]0;t\x07\x1bPzz\x1b\\\x1b[c"]);
        assert_eq!(out, b"a\x1b[31mbcd\x1b]0;t\x07\x1bPzz\x1b\\");
        assert_eq!(found, vec![(7, Found::Sixel(b"#0;2;100;0;0~~".to_vec())), (8, Found::Iterm(b"inline=1:QUJD".to_vec())), (out.len(), Found::Attributes)]);
        let (out, found) = scan(&[b"\x1b\x1b[0c\x1b[1c\x1b]1338;x\x07"]);
        assert_eq!((out.as_slice(), found.len()), (&b"\x1b\x1b[1c\x1b]1338;x\x07"[..], 1));
    }

    #[test]
    fn decodes_sixel_and_iterm_images() {
        let image = sixel(b"\"1;1;3;7#1;2;100;0;0#2;1;120;50;100!2~$#2-@").unwrap();
        assert_eq!(image.size, [3, 7]);
        assert_eq!((image.pixels[0], image.pixels[1], image.pixels[2]), (Color32::from_rgb(255, 0, 0), Color32::from_rgb(255, 0, 0), Color32::TRANSPARENT));
        assert_eq!(image.pixels[6 * 3], Color32::from_rgb(255, 0, 0));
        assert_eq!(hls(0, 50, 100), Color32::from_rgb(0, 0, 255));
        assert_eq!(base64(b"SGFsbw=="), b"Halo");
        let mut png = vec![];
        image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 255, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let encoded: String = {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            png.chunks(3).flat_map(|c| {
                let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
                (0..=c.len()).map(move |k| A[(n >> (18 - 6 * k) & 63) as usize] as char)
            }).collect()
        };
        let body = format!("name=eA==;width=2;inline=1:{encoded}").into_bytes();
        let (image, size) = decode(Found::Iterm(body), (8.0, 16.0), (80, 24)).unwrap();
        assert_eq!((image.size, size), ([4, 2], (16.0, 8.0)));
        assert!(decode(Found::Iterm(format!("inline=0:{encoded}").into_bytes()), (8.0, 16.0), (80, 24)).is_none());
        let (_, size) = decode(Found::Iterm(format!("width=200%;inline=1:{encoded}").into_bytes()), (8.0, 16.0), (10, 24)).unwrap();
        assert_eq!(size, (80.0, 40.0));
    }
}
