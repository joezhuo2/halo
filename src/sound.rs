use crate::debug::Breakpoint;
use crate::lsp::Diagnostic;
use crate::settings::Settings;
use std::cell::Cell;

const RATE: u32 = 22050;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cue {
    Error,
    Warning,
    Breakpoint,
    Folded,
    Inserted,
    Deleted,
}

impl Cue {
    fn tones(self) -> &'static [(f32, u32)] {
        match self {
            Cue::Error => &[(660.0, 60), (440.0, 80)],
            Cue::Warning => &[(560.0, 100)],
            Cue::Breakpoint => &[(990.0, 70)],
            Cue::Folded => &[(520.0, 40), (780.0, 40)],
            Cue::Inserted => &[(523.0, 50), (784.0, 60)],
            Cue::Deleted => &[(784.0, 50), (523.0, 60)],
        }
    }

    fn setting(self, s: &Settings) -> &str {
        match self {
            Cue::Error => &s.signal_line_error,
            Cue::Warning => &s.signal_line_warning,
            Cue::Breakpoint => &s.signal_line_breakpoint,
            Cue::Folded => &s.signal_line_folded,
            Cue::Inserted => &s.signal_diff_inserted,
            Cue::Deleted => &s.signal_diff_deleted,
        }
    }
}

pub fn wav(tones: &[(f32, u32)], volume: u32) -> Vec<u8> {
    let gain = 16000.0 * volume.min(100) as f32 / 100.0;
    let fade = (RATE / 200) as f32;
    let samples: Vec<i16> = tones
        .iter()
        .flat_map(|&(freq, ms)| {
            let n = RATE * ms / 1000;
            (0..n).map(move |i| {
                let edge = (i.min(n - 1 - i) as f32 / fade).min(1.0);
                ((i as f32 / RATE as f32 * freq * std::f32::consts::TAU).sin() * edge * gain) as i16
            })
        })
        .collect();
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend([1u16, 1].iter().flat_map(|v| v.to_le_bytes()));
    out.extend([RATE, RATE * 2].iter().flat_map(|v| v.to_le_bytes()));
    out.extend([2u16, 16].iter().flat_map(|v| v.to_le_bytes()));
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    out.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    out
}

fn wanted(choice: &str) -> bool {
    match choice {
        "on" => true,
        "auto" => crate::a11y::optimized(),
        _ => false,
    }
}

pub fn play(settings: &Settings, cue: Cue) {
    if wanted(cue.setting(settings)) {
        os::play(cue, settings.signal_volume.min(100));
    }
}

pub fn line_cue(settings: &Settings, error: bool, warning: bool, breakpoint: bool, folded: bool) -> Option<Cue> {
    [(error, Cue::Error), (warning && !error, Cue::Warning), (breakpoint, Cue::Breakpoint), (folded, Cue::Folded)]
        .into_iter()
        .find(|(on, cue)| *on && wanted(cue.setting(settings)))
        .map(|(_, cue)| cue)
}

pub fn arrived(last: &mut Option<(u64, usize, u64)>, now: (u64, usize, u64)) -> bool {
    let moved = last.is_some_and(|(doc, line, version)| doc == now.0 && line != now.1 && version == now.2);
    *last = Some(now);
    moved
}

thread_local! {
    static AT: Cell<Option<(u64, usize, u64)>> = const { Cell::new(None) };
}

pub fn cursor(settings: &Settings, now: (u64, usize, u64), problems: &[Diagnostic], breakpoints: &[Breakpoint], folds: &[(usize, usize)]) {
    let mut last = AT.get();
    let moved = arrived(&mut last, now);
    AT.set(last);
    let line = now.1;
    let on = |severity| problems.iter().any(|p| p.severity == severity && p.start.0 <= line && line <= p.end.0);
    if let Some(cue) = line_cue(settings, on(1), on(2), breakpoints.iter().any(|b| b.line == line), folds.iter().any(|f| f.0 == line)).filter(|_| moved) {
        play(settings, cue);
    }
}

#[cfg(windows)]
mod os {
    use super::Cue;
    use std::sync::Mutex;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    type Made = Vec<(Cue, u32, Box<[u8]>)>;
    static MADE: Mutex<Made> = Mutex::new(Vec::new());

    pub fn play(cue: Cue, volume: u32) {
        let mut made = MADE.lock().unwrap();
        let at = match made.iter().position(|m| (m.0, m.1) == (cue, volume)) {
            Some(at) => at,
            None => {
                made.push((cue, volume, super::wav(cue.tones(), volume).into_boxed_slice()));
                made.len() - 1
            }
        };
        unsafe {
            let _ = PlaySoundW(windows::core::PCWSTR(made[at].2.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        }
    }
}

#[cfg(not(windows))]
mod os {
    pub fn play(_: super::Cue, _: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(wav: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(wav[at..at + 4].try_into().unwrap())
    }

    fn half(wav: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(wav[at..at + 2].try_into().unwrap())
    }

    fn peak(wav: &[u8]) -> i32 {
        wav[44..].chunks(2).map(|s| (i16::from_le_bytes([s[0], s[1]]) as i32).abs()).max().unwrap_or(0)
    }

    #[test]
    fn writes_a_16_bit_mono_wave_with_the_samples_scaled_by_volume() {
        let wav = wav(&[(440.0, 100), (660.0, 50)], 100);
        let samples = (RATE * 100 / 1000 + RATE * 50 / 1000) as usize;
        assert_eq!(wav.len(), 44 + samples * 2);
        assert_eq!((&wav[0..4], &wav[8..16], &wav[36..40]), (&b"RIFF"[..], &b"WAVEfmt "[..], &b"data"[..]));
        assert_eq!((word(&wav, 4), word(&wav, 16), word(&wav, 40)), (wav.len() as u32 - 8, 16, samples as u32 * 2));
        assert_eq!((half(&wav, 20), half(&wav, 22), word(&wav, 24), word(&wav, 28), half(&wav, 32), half(&wav, 34)), (1, 1, RATE, RATE * 2, 2, 16));
        assert!(peak(&wav) > 15000 && peak(&wav) <= 16000, "{}", peak(&wav));
        let quiet = peak(&super::wav(&[(440.0, 100)], 50));
        assert!((7500..=8000).contains(&quiet), "{quiet}");
        assert_eq!(peak(&super::wav(&[(440.0, 100)], 0)), 0);
        assert_eq!(super::wav(&[(440.0, 100)], 250), super::wav(&[(440.0, 100)], 100));
        for cue in [Cue::Error, Cue::Warning, Cue::Breakpoint, Cue::Folded, Cue::Inserted, Cue::Deleted] {
            let ms: u32 = cue.tones().iter().map(|t| t.1).sum();
            assert!((60..=150).contains(&ms), "{cue:?}");
        }
    }

    #[test]
    fn picks_one_cue_for_the_line_and_only_when_the_cursor_arrives() {
        let on = |s: &str| s.to_owned();
        let mut settings = Settings {
            signal_line_error: on("on"),
            signal_line_warning: on("on"),
            signal_line_breakpoint: on("on"),
            signal_line_folded: on("on"),
            ..Settings::default()
        };
        assert_eq!(line_cue(&settings, true, true, true, true), Some(Cue::Error));
        assert_eq!(line_cue(&settings, false, true, true, true), Some(Cue::Warning));
        assert_eq!(line_cue(&settings, false, false, true, true), Some(Cue::Breakpoint));
        assert_eq!(line_cue(&settings, false, false, false, true), Some(Cue::Folded));
        assert_eq!(line_cue(&settings, false, false, false, false), None);
        settings.signal_line_error = on("off");
        assert_eq!(line_cue(&settings, true, true, false, false), None);
        assert_eq!(line_cue(&settings, true, true, true, false), Some(Cue::Breakpoint));
        settings.signal_line_breakpoint = on("off");
        assert_eq!(line_cue(&settings, false, false, true, true), Some(Cue::Folded));

        let mut last = None;
        assert!(!arrived(&mut last, (1, 4, 7)));
        assert!(arrived(&mut last, (1, 5, 7)));
        assert!(!arrived(&mut last, (1, 5, 7)));
        assert!(!arrived(&mut last, (1, 5, 8)));
        assert!(!arrived(&mut last, (1, 6, 9)));
        assert!(arrived(&mut last, (1, 2, 9)));
        assert!(!arrived(&mut last, (2, 3, 9)));
    }
}
