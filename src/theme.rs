use crate::settings::Settings;
use eframe::egui::{self, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle};

#[derive(Clone)]
pub struct Syntax {
    pub comment: Color32,
    pub string: Color32,
    pub keyword: Color32,
    pub number: Color32,
    pub function: Color32,
    pub type_name: Color32,
    pub constant: Color32,
    pub punctuation: Color32,
}

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub contrast: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub bar: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub added: Color32,
    pub modified: Color32,
    pub deleted: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub syntax: Syntax,
    pub ansi: [Color32; 16],
    pub tokens: Tokens,
    pub semantic: [Option<Color32>; 3],
}

#[derive(Clone)]
pub struct Tokens {
    pub id: usize,
    pub rules: std::sync::Arc<Vec<syntect::highlighting::ThemeItem>>,
}

impl Tokens {
    pub fn new(rules: Vec<syntect::highlighting::ThemeItem>) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self { id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed), rules: rules.into() }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub fn fonts(file: &str, terminal: &str) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    for (name, file) in [("editor", file), ("terminal", terminal)].into_iter().filter(|f| !f.1.is_empty()) {
        match std::fs::read(file) {
            Ok(bytes) if skrifa::FontRef::from_index(&bytes, 0).is_ok() => {
                fonts.font_data.insert(name.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                if name == "editor" {
                    fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, name.into());
                }
            }
            Ok(_) => crate::ui::notify(format!("{file} is not a font Halo can load, the default font is in use.")),
            Err(error) => crate::ui::notify(format!("Could not read the font {file}: {error}")),
        }
    }
    let mut mono = fonts.families.get(&egui::FontFamily::Monospace).cloned().unwrap_or_default();
    if fonts.font_data.contains_key("terminal") {
        mono.insert(0, "terminal".into());
    }
    fonts.families.insert(egui::FontFamily::Name("terminal".into()), mono);
    fonts
}

impl Theme {
    pub fn named(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "hc-dark" => Self::hc_dark(),
            "hc-light" => Self::hc_light(),
            _ => Self::dark(),
        }
    }

    pub fn of(settings: &Settings) -> Self {
        let name = crate::a11y::theme_name(settings);
        let mut theme = match name {
            "dark" | "light" | "hc-dark" | "hc-light" => Self::named(name),
            name => crate::theme_import::load(&crate::settings::dir(), name).unwrap_or_else(|error| {
                crate::ui::notify(format!("The theme {name} could not be loaded, Dark is in use: {error}"));
                Self::dark()
            }),
        };
        if let Some(custom) = settings.colors.get(name) {
            for (name, color) in theme.colors() {
                if let Some(chosen) = custom.get(name).and_then(|hex| Color32::from_hex(hex).ok()) {
                    *color = chosen;
                }
            }
        }
        theme
    }

    pub fn colors(&mut self) -> Vec<(&'static str, &mut Color32)> {
        let s = &mut self.syntax;
        vec![
            ("background", &mut self.bg),
            ("panel", &mut self.panel),
            ("bar", &mut self.bar),
            ("raised", &mut self.raised),
            ("hover", &mut self.hover),
            ("border", &mut self.border),
            ("text", &mut self.text),
            ("dimText", &mut self.text_dim),
            ("accent", &mut self.accent),
            ("selection", &mut self.selection),
            ("comment", &mut s.comment),
            ("string", &mut s.string),
            ("keyword", &mut s.keyword),
            ("number", &mut s.number),
            ("function", &mut s.function),
            ("type", &mut s.type_name),
            ("constant", &mut s.constant),
            ("punctuation", &mut s.punctuation),
        ]
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            contrast: false,
            bg: rgb(0x1e2024),
            panel: rgb(0x24262b),
            bar: rgb(0x2a2d32),
            raised: rgb(0x31343a),
            hover: rgb(0x383c43),
            border: rgb(0x3d4149),
            text: rgb(0xd3d7de),
            text_dim: rgb(0x999ea8),
            accent: rgb(0x9fdcf7),
            selection: rgb(0x34515f),
            added: rgb(0x7fd38a),
            modified: rgb(0xe6c35c),
            deleted: rgb(0xf0736b),
            error: rgb(0xf0736b),
            warning: rgb(0xe6c35c),
            syntax: Syntax {
                comment: rgb(0x838995),
                string: rgb(0xb5d99c),
                keyword: rgb(0x9fdcf7),
                number: rgb(0xf0b98a),
                function: rgb(0xc9b8f5),
                type_name: rgb(0x8fd0c8),
                constant: rgb(0xf0b98a),
                punctuation: rgb(0xa4aab5),
            },
            ansi: [
                rgb(0x1e2024), rgb(0xf0736b), rgb(0x7fd38a), rgb(0xe6c35c),
                rgb(0x7db4f0), rgb(0xc9b8f5), rgb(0x8fd0c8), rgb(0xd3d7de),
                rgb(0x6f7682), rgb(0xff8f87), rgb(0x9be7a5), rgb(0xf5d67d),
                rgb(0x9fdcf7), rgb(0xdccdfb), rgb(0xa9e3dc), rgb(0xf2f4f7),
            ],
            tokens: Tokens::new(vec![]),
            semantic: [None; 3],
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            contrast: false,
            bg: rgb(0xf5f6f8),
            panel: rgb(0xeaecef),
            bar: rgb(0xdfe2e7),
            raised: rgb(0xffffff),
            hover: rgb(0xd6dae0),
            border: rgb(0xc9cdd4),
            text: rgb(0x2a2d33),
            text_dim: rgb(0x5e636c),
            accent: rgb(0x187098),
            selection: rgb(0xc4e6f6),
            added: rgb(0x237834),
            modified: rgb(0x826300),
            deleted: rgb(0xc0362e),
            error: rgb(0xc0362e),
            warning: rgb(0x826300),
            syntax: Syntax {
                comment: rgb(0x6a707c),
                string: rgb(0x397d35),
                keyword: rgb(0x1d77a3),
                number: rgb(0xa85b1c),
                function: rgb(0x6b4fc4),
                type_name: rgb(0x1c7b70),
                constant: rgb(0xa85b1c),
                punctuation: rgb(0x555b65),
            },
            ansi: [
                rgb(0x2a2d33), rgb(0xd0453d), rgb(0x2e9e44), rgb(0xb58a00),
                rgb(0x2f6fd0), rgb(0x6b4fc4), rgb(0x1f8a7d), rgb(0x9aa0aa),
                rgb(0x6c727c), rgb(0xe0554d), rgb(0x3bb454), rgb(0xc99a00),
                rgb(0x1f8fc2), rgb(0x8166dc), rgb(0x25a394), rgb(0xf5f6f8),
            ],
            tokens: Tokens::new(vec![]),
            semantic: [None; 3],
        }
    }

    pub fn hc_dark() -> Self {
        Self {
            contrast: true,
            bg: rgb(0x000000),
            panel: rgb(0x000000),
            bar: rgb(0x000000),
            raised: rgb(0x000000),
            hover: rgb(0x1f1f1f),
            border: rgb(0x6fc3df),
            text: rgb(0xffffff),
            text_dim: rgb(0xd6d6d6),
            accent: rgb(0xf38518),
            selection: rgb(0x0f4a85),
            added: rgb(0x81b88b),
            modified: rgb(0xe2c08d),
            deleted: rgb(0xf48771),
            error: rgb(0xf48771),
            warning: rgb(0xcca700),
            syntax: Syntax {
                comment: rgb(0x7ca668),
                string: rgb(0xce9178),
                keyword: rgb(0x569cd6),
                number: rgb(0xb5cea8),
                function: rgb(0xdcdcaa),
                type_name: rgb(0x4ec9b0),
                constant: rgb(0x4fc1ff),
                punctuation: rgb(0xffffff),
            },
            ..Self::dark()
        }
    }

    pub fn hc_light() -> Self {
        Self {
            contrast: true,
            bg: rgb(0xffffff),
            panel: rgb(0xffffff),
            bar: rgb(0xffffff),
            raised: rgb(0xffffff),
            hover: rgb(0xf2f2f2),
            border: rgb(0x0f4a85),
            text: rgb(0x000000),
            text_dim: rgb(0x292929),
            accent: rgb(0x0f4a85),
            selection: rgb(0xd6ebff),
            added: rgb(0x374e06),
            modified: rgb(0x7d4d03),
            deleted: rgb(0xad0707),
            error: rgb(0xab1e0c),
            warning: rgb(0x7d4d03),
            syntax: Syntax {
                comment: rgb(0x515151),
                string: rgb(0x0f4a85),
                keyword: rgb(0x0f4a85),
                number: rgb(0x086442),
                function: rgb(0x5e2cbc),
                type_name: rgb(0x185e73),
                constant: rgb(0x0451a5),
                punctuation: rgb(0x000000),
            },
            ..Self::light()
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.dark { egui::Theme::Dark } else { egui::Theme::Light });

        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        let shadow = Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 110 } else { 45 }),
        };
        v.panel_fill = self.panel;
        v.window_fill = self.raised;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_shadow = shadow;
        v.popup_shadow = shadow;
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(6);
        v.extreme_bg_color = self.bg;
        v.text_edit_bg_color = Some(self.bg);
        v.faint_bg_color = self.hover;
        v.code_bg_color = self.raised;
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.warning;
        v.error_fg_color = self.error;
        v.selection.bg_fill = self.selection;
        v.selection.stroke = Stroke::new(1.0, self.accent);

        let radius = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = self.hover;
        w.inactive.weak_bg_fill = self.raised;
        w.inactive.bg_stroke = if self.contrast { Stroke::new(1.0, self.border) } else { Stroke::NONE };
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;

        for state in [&mut w.hovered, &mut w.open] {
            state.bg_fill = self.hover;
            state.weak_bg_fill = self.hover;
            state.bg_stroke = Stroke::new(1.0, if self.contrast { self.accent } else { self.border });
            state.fg_stroke = Stroke::new(1.0, self.text);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }

        w.active.bg_fill = self.selection;
        w.active.weak_bg_fill = self.selection;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.0, self.accent);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        ctx.set_visuals(v);
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.14;
            s.interaction.tooltip_delay = 0.35;
            s.interaction.selectable_labels = false;
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.menu_margin = egui::Margin::same(8);
            s.spacing.scroll.bar_width = 6.0;
            s.text_styles = [
                (TextStyle::Small, FontId::proportional(11.0)),
                (TextStyle::Body, FontId::proportional(13.0)),
                (TextStyle::Button, FontId::proportional(13.0)),
                (TextStyle::Heading, FontId::proportional(18.0)),
                (TextStyle::Monospace, FontId::monospace(13.0)),
            ]
            .into();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_custom_colors_to_their_own_theme() {
        let mut settings = Settings::default();
        settings.colors.entry("dark".into()).or_default().insert("accent".into(), "#ff0000ff".into());
        settings.colors.entry("dark".into()).or_default().insert("keyword".into(), "not a color".into());
        let theme = Theme::of(&settings);
        assert_eq!(theme.accent, Color32::RED);
        assert_eq!(theme.syntax.keyword, Theme::dark().syntax.keyword);
        settings.theme = "light".into();
        assert_eq!(Theme::of(&settings).accent, Theme::light().accent);
        assert!(fonts("", "").font_data.len() == fonts("no-such-font.ttf", "nope.ttf").font_data.len());
        assert!(fonts("", "").families.contains_key(&egui::FontFamily::Name("terminal".into())));
    }

    fn ratio(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let channel = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_meets_wcag_contrast_in_every_builtin_theme() {
        assert!((ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        for (name, least) in [("dark", 4.5), ("light", 4.5), ("hc-dark", 7.0), ("hc-light", 7.0)] {
            let t = Theme::named(name);
            let s = &t.syntax;
            let surfaces = [t.bg, t.panel, t.bar, t.raised];
            let mut pairs = vec![(t.text, t.hover), (t.text, t.selection)];
            for fg in [t.text, t.text_dim] {
                pairs.extend(surfaces.map(|bg| (fg, bg)));
            }
            for fg in [t.accent, t.added, t.modified, t.deleted, t.error, t.warning] {
                pairs.extend([(fg, t.bg), (fg, t.panel)]);
            }
            for fg in [s.comment, s.string, s.keyword, s.number, s.function, s.type_name, s.constant, s.punctuation] {
                pairs.push((fg, t.bg));
            }
            for (fg, bg) in pairs {
                assert!(ratio(fg, bg) >= least, "{name}: {fg:?} on {bg:?} is {:.2}:1", ratio(fg, bg));
            }
            assert_eq!(t.contrast, name.starts_with("hc"));
        }
    }
}
