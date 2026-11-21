use crate::settings::Settings;

pub fn theme_name(settings: &Settings) -> &str {
    match high_contrast() {
        Some(dark) if settings.auto_detect_high_contrast => {
            if dark { &settings.preferred_high_contrast_theme } else { &settings.preferred_high_contrast_light_theme }
        }
        _ => &settings.theme,
    }
}

#[cfg(windows)]
mod os {
    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut core::ffi::c_void, ini: u32) -> i32;
        fn GetSysColor(index: i32) -> u32;
    }

    pub fn high_contrast() -> Option<bool> {
        let mut hc = HighContrast { size: size_of::<HighContrast>() as u32, flags: 0, scheme: std::ptr::null_mut() };
        let ok = unsafe { SystemParametersInfoW(0x0042, hc.size, (&raw mut hc).cast(), 0) } != 0;
        if !ok || hc.flags & 1 == 0 {
            return None;
        }
        let window = unsafe { GetSysColor(5) };
        let luma = [0, 8, 16].map(|shift| window >> shift & 0xff);
        Some(luma[0] * 3 + luma[1] * 6 + luma[2] < 128 * 10)
    }
}

#[cfg(not(windows))]
mod os {
    pub fn high_contrast() -> Option<bool> {
        None
    }
}

pub use os::high_contrast;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_os_high_contrast_mode_only_when_asked() {
        let mut settings = Settings { theme: "light".into(), auto_detect_high_contrast: false, ..Settings::default() };
        assert_eq!(theme_name(&settings), "light");
        settings.auto_detect_high_contrast = true;
        let expected = match high_contrast() {
            Some(true) => "hc-dark",
            Some(false) => "hc-light",
            None => "light",
        };
        assert_eq!(theme_name(&settings), expected);
    }
}
