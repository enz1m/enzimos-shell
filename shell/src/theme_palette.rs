//! OKLCH-derived palettes with surface and text contrast constraints.

use enzui_shell::i18n::tr;
use enzui_theme::EnzThemeMode;
use iced::Color;

/// How strongly a palette carries its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    /// Quiet grey planes; the colour marks buttons, selection and focus.
    #[default]
    Neutral,
    /// Planes and lines take a soft wash of the colour.
    Tinted,
    /// Rich planes and a saturated accent.
    Vibrant,
    /// Deeper planes, brighter text and stronger lines, in the colour.
    Contrast,
}

impl Style {
    pub const ALL: [Style; 4] = [
        Style::Neutral,
        Style::Tinted,
        Style::Vibrant,
        Style::Contrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Style::Neutral => tr("shell-neutral"),
            Style::Tinted => tr("shell-tinted"),
            Style::Vibrant => tr("shell-vibrant"),
            Style::Contrast => tr("shell-high-contrast"),
        }
    }

    pub fn id_str(self) -> &'static str {
        match self {
            Style::Neutral => "neutral",
            Style::Tinted => "tinted",
            Style::Vibrant => "vibrant",
            Style::Contrast => "contrast",
        }
    }

    pub fn from_id_str(s: &str) -> Option<Style> {
        Self::ALL.into_iter().find(|style| style.id_str() == s)
    }

    // Tuple order: dark/light neutral chroma, accent chroma floor, wash.
    fn params(self) -> (f32, f32, f32, f32) {
        match self {
            // True greys: the colour marks only the accent family.
            Style::Neutral => (0.0, 0.0, 0.0, 0.30),
            Style::Tinted => (0.022, 0.017, 0.08, 0.40),
            Style::Vibrant => (0.038, 0.030, 0.14, 0.50),
            Style::Contrast => (0.010, 0.008, 0.10, 0.42),
        }
    }
}

/// Where the colour comes from.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Seed {
    /// The enzUI blue; with [`Style::Neutral`] the built-in palette itself.
    #[default]
    Default,
    Color(Color),
    Wallpaper(Color),
}

/// The default seed colour.
pub const DEFAULT_SEED: Color = Color::from_rgb(10.0 / 255.0, 132.0 / 255.0, 1.0);

/// Ready-made colours offered next to the wallpaper and a custom colour.
pub const PRESETS: [(&str, u32); 10] = [
    ("Blue", 0x0a84ff),
    ("Indigo", 0x5c6bc0),
    ("Purple", 0x9c27b0),
    ("Pink", 0xec407a),
    ("Red", 0xe53935),
    ("Orange", 0xf57c00),
    ("Amber", 0xffb300),
    ("Green", 0x34a853),
    ("Teal", 0x00a5a8),
    ("Slate", 0x78838f),
];

pub fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Seed {
    pub fn color(self) -> Color {
        match self {
            Seed::Default => DEFAULT_SEED,
            Seed::Color(color) | Seed::Wallpaper(color) => color,
        }
    }

    /// `default`, `#rrggbb` or `wallpaper:#rrggbb`.
    pub fn to_id_string(self) -> String {
        match self {
            Seed::Default => "default".into(),
            Seed::Color(color) => hex(color),
            Seed::Wallpaper(color) => format!("wallpaper:{}", hex(color)),
        }
    }

    pub fn from_id_str(s: &str) -> Option<Seed> {
        if s == "default" {
            return Some(Seed::Default);
        }
        match s.strip_prefix("wallpaper:") {
            Some(color) => parse_hex(color).map(Seed::Wallpaper),
            None => parse_hex(s).map(Seed::Color),
        }
    }
}

/// A style and the colour that drives it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scheme {
    pub style: Style,
    pub seed: Seed,
}

impl Scheme {
    /// `"<style> <seed>"`, as kept in the shell's settings.
    pub fn to_id_string(self) -> String {
        format!("{} {}", self.style.id_str(), self.seed.to_id_string())
    }

    pub fn from_id_str(s: &str) -> Option<Scheme> {
        let (style, seed) = s.split_once(' ')?;
        Some(Scheme {
            style: Style::from_id_str(style)?,
            seed: Seed::from_id_str(seed)?,
        })
    }

    /// The built-in palette, written as no tokens at all.
    pub fn is_builtin(self) -> bool {
        self == Scheme::default()
    }
}

pub fn hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#')?;
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
        .map(rgb)
}

/// The palette tokens a scheme writes, by token path.
pub const PATHS: [&str; 27] = [
    "palette.background",
    "palette.surface",
    "palette.surface_alt",
    "palette.surface_elevated",
    "palette.surface_hover",
    "palette.surface_selected",
    "palette.selected",
    "palette.text",
    "palette.text_muted",
    "palette.text_subtle",
    "palette.border",
    "palette.border_strong",
    "palette.control_border",
    "palette.control_border_strong",
    "palette.field_border",
    "palette.field_border_strong",
    "palette.disabled",
    "palette.button_primary_bg",
    "palette.button_primary_hover_bg",
    "palette.button_primary_pressed_bg",
    "palette.accent",
    "palette.accent_hover",
    "palette.accent_pressed",
    "palette.accent_soft",
    "palette.focus_ring",
    "palette.shadow",
    // Text-entry fields: white in light mode, the elevated step in dark.
    "palette.field",
];

/// A generated palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated(Vec<(&'static str, Color)>);

impl Generated {
    /// The colour for a role such as `"surface"` or `"palette.surface"`.
    pub fn get(&self, role: &str) -> Color {
        let path = role.strip_prefix("palette.").unwrap_or(role);
        self.0
            .iter()
            .find(|(p, _)| p.strip_prefix("palette.") == Some(path))
            .map_or(Color::BLACK, |(_, color)| *color)
    }

    pub fn tokens(&self) -> &[(&'static str, Color)] {
        &self.0
    }
}

/// What `scheme` paints in `mode`: [`generate`]d, or the framework's own
/// palette for the built-in scheme, which writes nothing.
pub fn shown(scheme: Scheme, mode: EnzThemeMode) -> Generated {
    if !scheme.is_builtin() {
        return generate(scheme, mode);
    }
    use enzui_core::theme_file::registry::{TokenValue, token};
    let theme = enzui_core::theme_file::default_theme(mode);
    Generated(
        PATHS
            .iter()
            .filter_map(|path| match token(path)?.read(&theme) {
                TokenValue::Color(color) => Some((*path, color)),
                _ => None,
            })
            .collect(),
    )
}

// Each tone stores lightness and its neutral-chroma multiplier.
type Tones = [(f32, f32); 20];

const DARK: Tones = [
    (0.231, 1.0),
    (0.273, 1.0),
    (0.311, 1.1),
    (0.343, 1.2),
    (0.311, 1.1),
    (0.396, 1.6),
    (0.396, 1.6),
    (0.959, 0.6),
    (0.780, 1.3),
    (0.650, 1.5),
    (0.407, 1.6),
    (0.481, 1.8),
    (0.631, 2.2),
    (0.720, 2.2),
    (0.290, 1.0),
    (0.460, 1.4),
    (0.550, 1.2),
    (0.389, 1.3),
    (0.425, 1.3),
    (0.355, 1.3),
];
const LIGHT: Tones = [
    (0.959, 0.8),
    (1.0, 0.0),
    (0.918, 1.0),
    (0.985, 0.4),
    (0.918, 1.0),
    (0.883, 1.4),
    (0.883, 1.4),
    (0.302, 1.0),
    (0.480, 1.6),
    (0.520, 1.6),
    (0.900, 1.4),
    (0.816, 1.8),
    (0.606, 1.6),
    (0.500, 1.8),
    (0.840, 1.0),
    (0.790, 1.2),
    (0.620, 1.0),
    (1.0, 0.0),
    (0.955, 0.8),
    (0.925, 0.9),
];
const CONTRAST_DARK: Tones = [
    (0.13, 1.0),
    (0.20, 1.0),
    (0.25, 1.0),
    (0.28, 1.0),
    (0.27, 1.0),
    (0.36, 1.5),
    (0.36, 1.5),
    (1.0, 0.0),
    (0.90, 0.8),
    (0.80, 1.0),
    (0.52, 1.5),
    (0.64, 1.5),
    (0.78, 1.5),
    (0.88, 1.5),
    (0.50, 1.0),
    (0.70, 1.0),
    (0.62, 1.0),
    (0.36, 1.2),
    (0.41, 1.2),
    (0.32, 1.2),
];
const CONTRAST_LIGHT: Tones = [
    (0.93, 1.0),
    (1.0, 0.0),
    (0.90, 1.0),
    (1.0, 0.0),
    (0.90, 1.0),
    (0.84, 1.4),
    (0.84, 1.4),
    (0.13, 1.0),
    (0.30, 1.2),
    (0.38, 1.2),
    (0.62, 1.3),
    (0.50, 1.4),
    (0.42, 1.4),
    (0.30, 1.4),
    (0.55, 1.0),
    (0.42, 1.0),
    (0.55, 1.0),
    (1.0, 0.0),
    (0.94, 0.8),
    (0.90, 0.9),
];

/// Generate the palette `scheme` paints in `mode`.
pub fn generate(scheme: Scheme, mode: EnzThemeMode) -> Generated {
    let dark = mode == EnzThemeMode::Dark;
    let (seed_l, seed_c, hue) = lch(scheme.seed.color());
    let grey = seed_c < 0.025;
    let (chroma_dark, chroma_light, floor, wash) = scheme.style.params();
    let neutral = if dark { chroma_dark } else { chroma_light } * if grey { 0.3 } else { 1.0 };
    let tones = match (scheme.style, dark) {
        (Style::Contrast, true) => CONTRAST_DARK,
        (Style::Contrast, false) => CONTRAST_LIGHT,
        (_, true) => DARK,
        (_, false) => LIGHT,
    };
    let mut colors: Vec<Color> = tones
        .iter()
        .map(|&(l, weight)| from_lch(l, neutral * weight, hue))
        .collect();

    let contrast = scheme.style == Style::Contrast;
    let (accent_l, accent_c) = if grey {
        (
            match (contrast, dark) {
                (true, true) => 0.80,
                (false, true) => 0.74,
                (true, false) => 0.30,
                (false, false) => 0.40,
            },
            0.0,
        )
    } else {
        let mut c = seed_c.max(floor);
        if scheme.style == Style::Vibrant {
            c = c.max(0.20);
        }
        // Dark accents keep the colour's own lightness within a band, so amber
        // stays amber rather than turning gold; light ones must carry white.
        (
            match (contrast, dark) {
                (true, true) => seed_l.clamp(0.80, 0.86),
                (false, true) => seed_l.clamp(0.64, 0.80),
                (true, false) => 0.42,
                (false, false) => 0.53,
            },
            c,
        )
    };
    let accent = from_lch(accent_l, accent_c, hue);
    let accent_hover = from_lch(accent_l + if dark { 0.05 } else { -0.04 }, accent_c, hue);
    let accent_pressed = from_lch(accent_l - if dark { 0.06 } else { 0.08 }, accent_c, hue);
    let accent_soft = from_lch(
        if dark {
            0.27 + 0.12 * wash
        } else {
            0.96 - 0.08 * wash
        },
        if grey { 0.0 } else { accent_c * wash * 0.8 },
        hue,
    );
    if scheme.style == Style::Vibrant && !grey {
        let selected = from_lch(
            if dark { 0.40 } else { 0.87 },
            (accent_c * 0.35).max(0.06),
            hue,
        );
        colors[5] = selected;
        colors[6] = selected;
    }
    let shadow = if dark {
        Color::from_rgba(0.0, 0.0, 0.0, 0.23)
    } else {
        let tint = from_lch(0.30, neutral * 2.0, hue);
        Color { a: 0.13, ..tint }
    };
    colors.extend([
        accent,
        accent_hover,
        accent_pressed,
        accent_soft,
        accent,
        shadow,
    ]);

    // Readability comes before the tone tables: push inks and lines away
    // from every plane they sit on until they clear their floor.
    let planes = [0, 1, 2, 3, 5];
    for (ink, floor) in [(7, 7.0), (8, 4.5), (9, 3.0)] {
        colors[ink] = raise(colors[ink], planes.map(|i| colors[i]), floor, dark, hue);
    }
    for (line, floor) in [(12, 3.0), (20, 3.0), (24, 3.0)] {
        colors[line] = raise(
            colors[line],
            [colors[0], colors[1], colors[1], colors[1], colors[1]],
            floor,
            dark,
            hue,
        );
    }
    // A field is the lifted step in dark (a field on a sidebar stays
    // visible) and white in light, as the framework's own palettes draw it.
    let field = if dark {
        colors[3]
    } else {
        from_lch(1.0, 0.0, hue)
    };
    colors.push(field);
    Generated(PATHS.iter().copied().zip(colors).collect())
}

fn raise(ink: Color, planes: [Color; 5], floor: f32, dark: bool, hue: f32) -> Color {
    let (mut l, c, _) = lch(ink);
    let mut out = ink;
    for _ in 0..80 {
        if planes
            .iter()
            .all(|plane| contrast_ratio(out, *plane) >= floor)
        {
            break;
        }
        l = if dark {
            (l + 0.01).min(1.0)
        } else {
            (l - 0.01).max(0.0)
        };
        out = from_lch(l, c, hue);
    }
    out
}

/// WCAG contrast ratio of two opaque colours.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn luminance(c: Color) -> f32 {
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn gamma(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLCH lightness, chroma and hue (degrees) of an sRGB colour.
pub fn lch(color: Color) -> (f32, f32, f32) {
    let (r, g, b) = (linear(color.r), linear(color.g), linear(color.b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    (
        lightness,
        a.hypot(bb),
        bb.atan2(a).to_degrees().rem_euclid(360.0),
    )
}

fn oklab_to_linear(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

/// The sRGB colour at OKLCH `(l, c, hue)`, with chroma reduced until it fits.
pub fn from_lch(l: f32, c: f32, hue: f32) -> Color {
    let l = l.clamp(0.0, 1.0);
    let (sin, cos) = hue.to_radians().sin_cos();
    let at = |c: f32| oklab_to_linear(l, c * cos, c * sin);
    let fits = |rgb: [f32; 3]| rgb.iter().all(|v| (-1e-4..=1.0 + 1e-4).contains(v));
    let mut rgb = at(c);
    if !fits(rgb) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if fits(at(mid)) { lo = mid } else { hi = mid }
        }
        rgb = at(lo);
    }
    // Quantised as the theme file stores it, so a preview is what is written.
    let [r, g, b] = rgb.map(|v| (gamma(v.clamp(0.0, 1.0)).clamp(0.0, 1.0) * 255.0).round() as u8);
    Color::from_rgb8(r, g, b)
}

/// Returns the chroma-weighted dominant hue; achromatic pictures yield grey.
pub fn picture_seed(pixels: impl Iterator<Item = [u8; 3]>) -> Color {
    let mut bins = [(0.0_f32, 0.0_f32, 0.0_f32); 36];
    for [r, g, b] in pixels {
        let (l, c, h) = lch(Color::from_rgb8(r, g, b));
        if !(0.25..=0.92).contains(&l) || c < 0.03 {
            continue;
        }
        let bin = &mut bins[(h / 10.0) as usize % 36];
        let weight = c;
        let (sin, cos) = h.to_radians().sin_cos();
        bin.0 += weight;
        bin.1 += c * cos * weight;
        bin.2 += c * sin * weight;
    }
    // Neighbouring bins count too, so a hue split across a boundary still wins.
    let score = |i: usize| bins[(i + 35) % 36].0 * 0.5 + bins[i].0 + bins[(i + 1) % 36].0 * 0.5;
    let best = (0..36)
        .max_by(|a, b| score(*a).total_cmp(&score(*b)))
        .unwrap_or(0);
    let (weight, a, b) = bins[best];
    if weight < 1.0 {
        return rgb(0x78838f);
    }
    let (a, b) = (a / weight, b / weight);
    from_lch(
        0.62,
        a.hypot(b).max(0.06),
        b.atan2(a).to_degrees().rem_euclid(360.0),
    )
}

/// [`picture_seed`] of an image file, read small.
pub fn file_seed(path: &std::path::Path) -> Option<Color> {
    let image = raster::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let small = image.thumbnail(96, 96).to_rgb8();
    Some(picture_seed(small.pixels().map(|p| p.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_scheme() -> impl Iterator<Item = Scheme> {
        let seeds = PRESETS
            .iter()
            .map(|(_, hex)| Seed::Color(rgb(*hex)))
            .chain([
                Seed::Default,
                Seed::Color(rgb(0xffffff)),
                Seed::Color(rgb(0x101010)),
                Seed::Color(rgb(0xffeb3b)),
            ]);
        seeds.flat_map(|seed| {
            Style::ALL
                .into_iter()
                .map(move |style| Scheme { style, seed })
        })
    }

    #[test]
    fn every_style_and_colour_keeps_text_and_lines_readable() {
        for scheme in every_scheme() {
            for mode in [EnzThemeMode::Dark, EnzThemeMode::Light] {
                let p = generate(scheme, mode);
                for plane in [
                    "background",
                    "surface",
                    "surface_alt",
                    "surface_elevated",
                    "surface_selected",
                ] {
                    for (ink, floor) in [("text", 7.0), ("text_muted", 4.5), ("text_subtle", 3.0)] {
                        let ratio = contrast_ratio(p.get(ink), p.get(plane));
                        assert!(
                            ratio >= floor - 0.01,
                            "{scheme:?} {mode:?} {ink} on {plane}: {ratio}"
                        );
                    }
                }
                for line in ["accent", "focus_ring", "control_border"] {
                    for plane in ["background", "surface"] {
                        let ratio = contrast_ratio(p.get(line), p.get(plane));
                        assert!(
                            ratio >= 2.99,
                            "{scheme:?} {mode:?} {line} on {plane}: {ratio}"
                        );
                    }
                }
            }
        }
    }
}
