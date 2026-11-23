//! The shell chrome's own colour and see-through: the frame, the bar and its
//! islands, the rail and the panels that open from them. A theme's colour
//! need not be the chrome's: a green theme can sit in a pitch black frame
//! that meets the monitor's bezel.

use crate::i18n::tr;
use serde_json::Value;

/// Solid chrome's opacity, in percent. Below half, text on panels over busy
/// windows stopped being readable.
pub const OPACITY: (u8, u8) = (50, 100);
/// Glass tint, in percent: below 10 the islands all but vanished over dark
/// wallpaper; the frosted glass itself starts at 82–94.
pub const GLASS_TINT: (u8, u8) = (10, 95);
/// Glass blur, in logical pixels, up to what the compositor's material
/// takes (the frosted glass starts at 20).
pub const GLASS_BLUR: (u8, u8) = (0, 24);
/// How strongly the glass's rim bends the backdrop, in percent of a full
/// lens: more smeared the islands' own labels.
pub const GLASS_BENDING: (u8, u8) = (0, 100);
/// Bending until the person sets their own: a full lens rim.
pub const GLASS_BENDING_DEFAULT: u8 = 100;
/// The glass rim's extra light, in percent; the rim keeps its own faint
/// reflection at zero.
pub const GLASS_HIGHLIGHT: (u8, u8) = (0, 100);
/// Highlight until the person sets their own: minimal.
pub const GLASS_HIGHLIGHT_DEFAULT: u8 = 0;

/// The chrome's colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChromeColor {
    /// The theme's own surface.
    #[default]
    Theme,
    /// The theme's accent.
    Accent,
    /// A colour of its own, as 0xRRGGBB.
    Custom(u32),
}

impl ChromeColor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => tr("shell-chrome-color-theme"),
            Self::Accent => tr("shell-chrome-color-accent"),
            Self::Custom(_) => tr("shell-chrome-color-custom"),
        }
    }

    /// Resolves a custom Shell tint against the theme's mode and accent.
    pub fn tint(self, light: bool, accent: iced::Color) -> Option<iced::Color> {
        match self {
            Self::Theme => None,
            Self::Accent => {
                let [r, g, b] = crate::desktop_frame::color(light);
                let base = iced::Color::from_rgb8(r, g, b);
                let mix = |from: f32, to: f32| from + (to - from) * 0.45;
                Some(iced::Color::from_rgb(
                    mix(base.r, accent.r),
                    mix(base.g, accent.g),
                    mix(base.b, accent.b),
                ))
            }
            Self::Custom(rgb) => Some(crate::theme_palette::rgb(rgb)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChromeStyle {
    pub color: ChromeColor,
    /// The last custom colour, kept while another choice is made.
    pub custom: u32,
    /// Solid chrome's opacity, in percent ([`OPACITY`]).
    pub opacity: u8,
    /// Glass tint ([`GLASS_TINT`]); `None` keeps the material's own.
    pub glass_tint: Option<u8>,
    /// Glass blur ([`GLASS_BLUR`]); `None` keeps the material's own.
    pub glass_blur: Option<u8>,
    /// Rim bending ([`GLASS_BENDING`]); `None` is [`GLASS_BENDING_DEFAULT`].
    pub glass_bending: Option<u8>,
    /// Rim light ([`GLASS_HIGHLIGHT`]); `None` is [`GLASS_HIGHLIGHT_DEFAULT`].
    pub glass_highlight: Option<u8>,
}

impl Default for ChromeStyle {
    fn default() -> Self {
        Self {
            color: ChromeColor::Theme,
            custom: 0x000000,
            opacity: OPACITY.1,
            glass_tint: None,
            glass_blur: None,
            glass_bending: None,
            glass_highlight: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Chooses the colour; a custom choice carries its value.
    Color(ChromeColor),
    Opacity(u8),
    GlassTint(Option<u8>),
    GlassBlur(Option<u8>),
    GlassBending(Option<u8>),
    GlassHighlight(Option<u8>),
}

fn within(value: u8, (min, max): (u8, u8)) -> Result<u8, String> {
    (min..=max)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| tr("shell-invalid-chrome-style").into())
}

fn hex(value: u32) -> String {
    format!("#{:06x}", value & 0xff_ffff)
}

fn parse_hex(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

impl ChromeStyle {
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        let mut style = Self::default();
        let Some(value) = value else {
            return Ok(style);
        };
        let bad = || -> String { tr("shell-invalid-chrome-style").into() };
        let object = value.as_object().ok_or_else(bad)?;
        if let Some(custom) = object.get("custom") {
            style.custom = custom.as_str().and_then(parse_hex).ok_or_else(bad)?;
        }
        style.color = match object.get("color").map(Value::as_str) {
            None => ChromeColor::Theme,
            Some(Some("theme")) => ChromeColor::Theme,
            Some(Some("accent")) => ChromeColor::Accent,
            Some(Some(text)) => ChromeColor::Custom(parse_hex(text).ok_or_else(bad)?),
            Some(None) => return Err(bad()),
        };
        if let ChromeColor::Custom(value) = style.color {
            style.custom = value;
        }
        let number = |key: &str, range: (u8, u8)| -> Result<Option<u8>, String> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => {
                    let value = value
                        .as_u64()
                        .and_then(|value| u8::try_from(value).ok())
                        .ok_or_else(bad)?;
                    within(value, range).map(Some)
                }
            }
        };
        style.opacity = number("opacity", OPACITY)?.unwrap_or(OPACITY.1);
        style.glass_tint = number("glass_tint", GLASS_TINT)?;
        style.glass_blur = number("glass_blur", GLASS_BLUR)?;
        style.glass_bending = number("glass_bending", GLASS_BENDING)?;
        style.glass_highlight = number("glass_highlight", GLASS_HIGHLIGHT)?;
        Ok(style)
    }

    pub fn to_json(self) -> Value {
        let mut object = serde_json::Map::new();
        object.insert(
            "color".into(),
            match self.color {
                ChromeColor::Theme => "theme".into(),
                ChromeColor::Accent => "accent".into(),
                ChromeColor::Custom(value) => hex(value).into(),
            },
        );
        object.insert("custom".into(), hex(self.custom).into());
        object.insert("opacity".into(), self.opacity.into());
        for (key, value) in [
            ("glass_tint", self.glass_tint),
            ("glass_blur", self.glass_blur),
            ("glass_bending", self.glass_bending),
            ("glass_highlight", self.glass_highlight),
        ] {
            if let Some(value) = value {
                object.insert(key.into(), value.into());
            }
        }
        Value::Object(object)
    }

    pub fn apply(&mut self, change: Change) -> Result<(), String> {
        let mut next = *self;
        match change {
            Change::Color(color) => {
                next.color = match color {
                    ChromeColor::Custom(value) => ChromeColor::Custom(value & 0xff_ffff),
                    other => other,
                };
                if let ChromeColor::Custom(value) = next.color {
                    next.custom = value;
                }
            }
            Change::Opacity(value) => next.opacity = within(value, OPACITY)?,
            Change::GlassTint(value) => {
                next.glass_tint = value.map(|value| within(value, GLASS_TINT)).transpose()?;
            }
            Change::GlassBlur(value) => {
                next.glass_blur = value.map(|value| within(value, GLASS_BLUR)).transpose()?;
            }
            Change::GlassBending(value) => {
                next.glass_bending = value
                    .map(|value| within(value, GLASS_BENDING))
                    .transpose()?;
            }
            Change::GlassHighlight(value) => {
                next.glass_highlight = value
                    .map(|value| within(value, GLASS_HIGHLIGHT))
                    .transpose()?;
            }
        }
        *self = next;
        Ok(())
    }

    /// Writes `change` into a settings document's `chrome` object.
    pub fn update_json(root: &mut Value, change: Change) -> Result<(), String> {
        let mut style = Self::from_json(root.get("chrome"))?;
        style.apply(change)?;
        root["chrome"] = style.to_json();
        Ok(())
    }

    /// The glass adjustments this style asks for.
    pub fn glass(self) -> crate::reveal::GlassTweaks {
        crate::reveal::GlassTweaks {
            tint: self.glass_tint.map(|value| f32::from(value) / 100.0),
            blur: self.glass_blur.map(f32::from),
            bending: Some(f32::from(self.glass_bending.unwrap_or(GLASS_BENDING_DEFAULT)) / 100.0),
            highlight: Some(
                f32::from(self.glass_highlight.unwrap_or(GLASS_HIGHLIGHT_DEFAULT)) / 100.0,
            ),
            still: false,
        }
    }

    /// The colour a custom choice starts from: the last one used.
    pub fn custom_color(self) -> u32 {
        match self.color {
            ChromeColor::Custom(value) => value,
            _ => self.custom,
        }
    }
}

/// Relative luminance of an sRGB colour, `0..=1`.
pub fn luminance(color: iced::Color) -> f32 {
    let linear = |c: f32| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// Whether text on `color` should be dark (the colour is light).
pub fn is_light(color: iced::Color) -> bool {
    // Black text out-contrasts white text above this luminance.
    luminance(color) > 0.179
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_bounded_roundtrip_and_custom_is_remembered() {
        let mut style = ChromeStyle::default();
        assert_eq!(ChromeStyle::from_json(None).unwrap(), style);
        for (change, ok) in [
            (Change::Opacity(49), false),
            (Change::Opacity(101), false),
            (Change::Opacity(70), true),
            (Change::GlassTint(Some(9)), false),
            (Change::GlassTint(Some(96)), false),
            (Change::GlassTint(Some(30)), true),
            (Change::GlassBlur(Some(25)), false),
            (Change::GlassBlur(Some(0)), true),
            (Change::GlassBending(Some(101)), false),
            (Change::GlassBending(Some(40)), true),
            (Change::GlassHighlight(Some(101)), false),
            (Change::GlassHighlight(Some(5)), true),
            (Change::Color(ChromeColor::Custom(0x101010)), true),
        ] {
            let before = style;
            assert_eq!(style.apply(change).is_ok(), ok, "{change:?}");
            if !ok {
                assert_eq!(style, before);
            }
        }
        let json = style.to_json();
        assert_eq!(ChromeStyle::from_json(Some(&json)).unwrap(), style);
        // Leaving Custom keeps the colour for coming back.
        style.apply(Change::Color(ChromeColor::Accent)).unwrap();
        assert_eq!(style.custom_color(), 0x101010);
        style.apply(Change::Color(ChromeColor::Custom(0))).unwrap();
        assert_eq!(style.color, ChromeColor::Custom(0));
        let mut root = serde_json::json!({"chrome": {"opacity": 49}});
        assert!(ChromeStyle::update_json(&mut root, Change::Opacity(80)).is_err());
        let mut root = serde_json::json!({});
        ChromeStyle::update_json(&mut root, Change::GlassBlur(Some(4))).unwrap();
        assert_eq!(root["chrome"]["glass_blur"], 4);
        assert!(ChromeStyle::from_json(Some(&serde_json::json!({"color": "red"}))).is_err());
        assert!(is_light(iced::Color::WHITE) && !is_light(iced::Color::BLACK));
    }
}
