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
        if name == "light" { Self::light() } else { Self::dark() }
    }

    pub fn of(settings: &Settings) -> Self {
        let mut theme = Self::named(&settings.theme);
        if let Some(custom) = settings.colors.get(&settings.theme) {
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
            bg: rgb(0x1e2024),
            panel: rgb(0x24262b),
            bar: rgb(0x2a2d32),
            raised: rgb(0x31343a),
            hover: rgb(0x383c43),
            border: rgb(0x3d4149),
            text: rgb(0xd3d7de),
            text_dim: rgb(0x8a909b),
            accent: rgb(0x9fdcf7),
            selection: rgb(0x34515f),
            added: rgb(0x7fd38a),
            modified: rgb(0xe6c35c),
            deleted: rgb(0xf0736b),
            error: rgb(0xf0736b),
            warning: rgb(0xe6c35c),
            syntax: Syntax {
                comment: rgb(0x6f7682),
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
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: rgb(0xf5f6f8),
            panel: rgb(0xeaecef),
            bar: rgb(0xdfe2e7),
            raised: rgb(0xffffff),
            hover: rgb(0xd6dae0),
            border: rgb(0xc9cdd4),
            text: rgb(0x2a2d33),
            text_dim: rgb(0x6c727c),
            accent: rgb(0x1f8fc2),
            selection: rgb(0xc4e6f6),
            added: rgb(0x2e9e44),
            modified: rgb(0xb58a00),
            deleted: rgb(0xd0453d),
            error: rgb(0xd0453d),
            warning: rgb(0xb58a00),
            syntax: Syntax {
                comment: rgb(0x8a909b),
                string: rgb(0x3f8a3a),
                keyword: rgb(0x1f7fae),
                number: rgb(0xb5621e),
                function: rgb(0x6b4fc4),
                type_name: rgb(0x1f8a7d),
                constant: rgb(0xb5621e),
                punctuation: rgb(0x555b65),
            },
            ansi: [
                rgb(0x2a2d33), rgb(0xd0453d), rgb(0x2e9e44), rgb(0xb58a00),
                rgb(0x2f6fd0), rgb(0x6b4fc4), rgb(0x1f8a7d), rgb(0x9aa0aa),
                rgb(0x6c727c), rgb(0xe0554d), rgb(0x3bb454), rgb(0xc99a00),
                rgb(0x1f8fc2), rgb(0x8166dc), rgb(0x25a394), rgb(0xf5f6f8),
            ],
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
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;

        for state in [&mut w.hovered, &mut w.open] {
            state.bg_fill = self.hover;
            state.weak_bg_fill = self.hover;
            state.bg_stroke = Stroke::new(1.0, self.border);
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
}
