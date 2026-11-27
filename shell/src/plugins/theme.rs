use enzui_shell::i18n::tr;
use enzui_theme::EnzTheme;
use iced::Color;
use std::collections::BTreeMap;

#[derive(Clone, Default, PartialEq)]
pub struct Colors(BTreeMap<String, [u8; 3]>);
impl Colors {
    pub fn new(colors: BTreeMap<String, [u8; 3]>) -> Result<Self, String> {
        if colors.len() > 4
            || colors
                .keys()
                .any(|key| !matches!(key.as_str(), "background" | "surface" | "text" | "accent"))
        {
            return Err(tr("shell-unknown-plugin-color-target").into());
        }
        Ok(Self(colors))
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn apply(&self, base: &EnzTheme) -> Result<EnzTheme, String> {
        let mut theme = base.clone();
        let p = &mut theme.palette;
        for (key, [r, g, b]) in &self.0 {
            let color = Color::from_rgb8(*r, *g, *b);
            match key.as_str() {
                "background" => p.background = color,
                "surface" => p.surface = color,
                "text" => p.text = color,
                "accent" => {
                    p.accent = color;
                    p.accent_hover = color.mix(p.text, 0.12);
                    p.accent_pressed = color.mix(p.background, 0.12);
                    p.focus_ring = color;
                }
                _ => return Err(tr("shell-unknown-plugin-color-target").into()),
            }
        }
        for surface in [
            p.background,
            p.surface,
            p.surface_alt,
            p.surface_elevated,
            p.surface_hover,
            p.surface_selected,
            p.button_primary_bg,
            p.button_primary_hover_bg,
            p.button_primary_pressed_bg,
        ] {
            if contrast(p.text, surface) < 4.5 {
                return Err(tr("shell-plugin-colors-need-readable-text-on-shell-surfaces").into());
            }
        }
        for surface in [p.background, p.surface] {
            if contrast(p.focus_ring, surface) < 3.0 {
                return Err(tr("shell-plugin-colors-need-a-visible-focus-indicator").into());
            }
        }
        Ok(theme)
    }
}
fn contrast(a: Color, b: Color) -> f64 {
    fn luminance(c: Color) -> f64 {
        let channel = |v: f32| {
            let v = f64::from(v);
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
