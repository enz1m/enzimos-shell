//! The look of the title bar the compositor draws for applications that leave
//! their frame to it, taken from the theme enzUI applications draw their own
//! title bars with.

use enzui_theme::{EnzTheme, FontFamily, FontRole};
use enzui_widgets::window_controls::{
    WindowControlKind, controls_lead, enz_window_controls, frame_shows,
};
use iced::Color;

/// Colours, title font and window controls for compositor-drawn title bars.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleBarTheme {
    background: [u8; 4],
    text: [u8; 4],
    button: [u8; 4],
    button_hover: [u8; 4],
    glyph: [u8; 4],
    glyph_hover: [u8; 4],
    font_family: String,
    font_size: u16,
    font_weight: u16,
    buttons: String,
}

impl TitleBarTheme {
    /// The bar an enzUI application with `theme` would draw, showing `buttons`
    /// (see [`buttons`]).
    #[must_use]
    pub fn new(theme: &EnzTheme, buttons: String) -> Self {
        let p = &theme.palette;
        let font_family = match theme.font_catalog.family(FontRole::Ui) {
            FontFamily::Named(name) => name.as_str(),
            FontFamily::Generic(_) => "Sans",
        };
        Self {
            background: rgba(p.background),
            text: rgba(p.text),
            button: rgba(p.surface_hover),
            button_hover: rgba(p.surface_selected),
            glyph: rgba(p.text_muted),
            glyph_hover: rgba(p.text),
            font_family: lua_safe_family(font_family),
            font_size: (theme.typography.body_size + 1.0).round().clamp(6.0, 48.0) as u16,
            font_weight: theme.typography.weight_bold.clamp(100, 1000),
            buttons,
        }
    }

    /// A compositor `eval` chunk that applies the theme and returns `"ok"`.
    #[must_use]
    pub fn lua(&self) -> String {
        let color =
            |c: [u8; 4]| format!("\"rgba({:02x}{:02x}{:02x}{:02x})\"", c[0], c[1], c[2], c[3]);
        format!(
            "hl.config({{decoration={{titlebar={{font_family=\"{}\",font_size={},font_weight={},buttons=\"{}\",\
             col={{active={},inactive={},text={},text_inactive={},button={},button_hover={},glyph={},glyph_hover={}}}}}}}}}) return \"ok\"",
            self.font_family,
            self.font_size,
            self.font_weight,
            self.buttons,
            color(self.background),
            color(self.background),
            color(self.text),
            color(self.text),
            color(self.button),
            color(self.button_hover),
            color(self.glyph),
            color(self.glyph_hover),
        )
    }
}

/// The window controls an enzUI application shell shows, as a compositor button
/// layout: the controls, in order, after a colon when they trail the title.
#[must_use]
pub fn buttons() -> String {
    let controls = enz_window_controls(())
        .show_minimize(frame_shows(WindowControlKind::Minimize, false), ())
        .show_maximize(frame_shows(WindowControlKind::Maximize, false), (), false)
        .rendered_order();
    layout(&controls, controls_lead())
}

fn layout(controls: &[WindowControlKind], lead: bool) -> String {
    let names = controls
        .iter()
        .map(|kind| kind.id_suffix())
        .collect::<Vec<_>>()
        .join(",");
    if lead {
        format!("{names}:")
    } else {
        format!(":{names}")
    }
}

fn rgba(color: Color) -> [u8; 4] {
    color.into_rgba8()
}

// The family goes into a Lua string literal; anything unusual falls back to the
// generic family rather than being escaped.
fn lua_safe_family(family: &str) -> String {
    let safe = !family.is_empty()
        && family.len() <= 64
        && family
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'));
    if safe {
        family.to_owned()
    } else {
        "Sans".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unusual_font_family_falls_back() {
        assert_eq!(lua_safe_family("Noto Sans"), "Noto Sans");
        assert_eq!(lua_safe_family("x\") os.exit() --"), "Sans");
        assert_eq!(lua_safe_family(""), "Sans");
    }
}
