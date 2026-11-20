//! Settings presets for the shared theme document; writes preserve unmanaged studio tokens.

use enzui_core::theme_file::ThemeDocument;
use enzui_core::theme_file::registry::TokenValue;
use enzui_shell::i18n::tr;
use enzui_theme::EnzThemeMode;
use iced::Color;

pub use crate::theme_palette::{Scheme, Seed, Style};

/// Corner presets affect unpinned framework controls; explicit widget radii remain unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corners {
    Sharp,
    #[default]
    Rounded,
    Soft,
}

impl Corners {
    pub const ALL: &'static [Corners] = &[Corners::Sharp, Corners::Rounded, Corners::Soft];

    pub fn label(self) -> &'static str {
        match self {
            Corners::Sharp => tr("shell-square"),
            Corners::Rounded => tr("shell-rounded"),
            Corners::Soft => tr("shell-soft"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Corners::Sharp => tr("shell-straight-edges-on-cards-buttons-and-fields"),
            Corners::Rounded => tr("shell-the-enzui-default"),
            Corners::Soft => tr("shell-generously-rounded-cards-and-controls"),
        }
    }

    pub fn id_str(self) -> &'static str {
        match self {
            Corners::Sharp => "square",
            Corners::Rounded => "rounded",
            Corners::Soft => "soft",
        }
    }

    pub fn from_id_str(s: &str) -> Option<Corners> {
        Self::ALL.iter().copied().find(|c| c.id_str() == s)
    }

    /// The `radius.*` rungs this style writes, in registry order
    /// (`xs, sm, md, lg, xl, xxl`). `none` and `full` keep their meaning.
    pub fn rungs(self) -> [f32; 6] {
        match self {
            Corners::Sharp => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            Corners::Rounded => [3.0, 5.0, 6.0, 10.0, 12.0, 20.0],
            Corners::Soft => [6.0, 9.0, 12.0, 16.0, 20.0, 28.0],
        }
    }

    /// The window corner radius this style gives the compositor's windows.
    pub fn window_radius(self) -> u16 {
        match self {
            Corners::Sharp => 0,
            Corners::Rounded => 22,
            Corners::Soft => 32,
        }
    }

    /// The nearest style for a document, used to show what the file holds.
    pub fn nearest(md: f32) -> Corners {
        Self::ALL
            .iter()
            .copied()
            .min_by(|a, b| {
                let da = (a.rungs()[2] - md).abs();
                let db = (b.rungs()[2] - md).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_default()
    }
}

/// Density presets write control height and minimum hit targets; read-only row height is only
/// cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Spacious,
}

impl Density {
    pub const ALL: &'static [Density] =
        &[Density::Compact, Density::Comfortable, Density::Spacious];

    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => tr("shell-compact"),
            Density::Comfortable => tr("shell-comfortable"),
            Density::Spacious => tr("shell-spacious"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Density::Compact => tr("shell-more-rows-fit-on-screen"),
            Density::Comfortable => tr("shell-the-enzui-default"),
            Density::Spacious => tr("shell-larger-targets-and-more-space-between-rows"),
        }
    }

    pub fn id_str(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Spacious => "spacious",
        }
    }

    pub fn from_id_str(s: &str) -> Option<Density> {
        Self::ALL.iter().copied().find(|d| d.id_str() == s)
    }

    /// Control height, row height and minimum hit target in logical pixels.
    pub fn sizing(self) -> (f32, f32, f32) {
        match self {
            Density::Compact => (28.0, 28.0, 24.0),
            Density::Comfortable => (32.0, 32.0, 32.0),
            Density::Spacious => (40.0, 40.0, 44.0),
        }
    }

    /// How the spacing ladder scales: tighter for compact, roomier for spacious.
    pub fn spacing_factor(self) -> f32 {
        match self {
            Density::Compact => 0.75,
            Density::Comfortable => 1.0,
            Density::Spacious => 1.25,
        }
    }

    pub fn nearest(control_height: f32) -> Density {
        Self::ALL
            .iter()
            .copied()
            .min_by(|a, b| {
                let da = (a.sizing().0 - control_height).abs();
                let db = (b.sizing().0 - control_height).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_default()
    }
}

/// Shadow presets scale elevation blur and offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shadows {
    None,
    Soft,
    #[default]
    Standard,
    Strong,
}

impl Shadows {
    pub const ALL: &'static [Shadows] = &[
        Shadows::None,
        Shadows::Soft,
        Shadows::Standard,
        Shadows::Strong,
    ];
    const BASE: [(f32, f32); 5] = [
        (4.0, 1.0),
        (8.0, 2.0),
        (16.0, 4.0),
        (24.0, 8.0),
        (32.0, 12.0),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Shadows::None => tr("shell-none"),
            Shadows::Soft => tr("shell-soft"),
            Shadows::Standard => tr("shell-standard"),
            Shadows::Strong => tr("shell-strong"),
        }
    }

    fn darkness(self) -> f32 {
        match self {
            Shadows::None => 0.0,
            Shadows::Soft => 0.6,
            Shadows::Standard => 1.0,
            Shadows::Strong => 1.8,
        }
    }

    fn factor(self) -> f32 {
        match self {
            Shadows::None => 0.0,
            Shadows::Soft => 0.6,
            Shadows::Standard => 1.0,
            Shadows::Strong => 1.6,
        }
    }

    /// The window shadow's reach and its active and inactive darkness; `None`
    /// keeps only the outline hairline.
    pub fn window(self) -> [f32; 3] {
        let base = enzui_theme::WindowShadow::default();
        [
            base.size * self.factor(),
            (base.opacity * self.darkness()).min(0.6),
            (base.inactive_opacity * self.darkness()).min(0.4),
        ]
    }

    /// `(blur, y offset)` for levels 1 to 5.
    pub fn tiers(self) -> [(f32, f32); 5] {
        Self::BASE.map(|(blur, y)| (blur * self.factor(), y * self.factor()))
    }

    fn nearest(level3_blur: f32) -> Shadows {
        Self::ALL
            .iter()
            .copied()
            .min_by(|a, b| {
                (a.tiers()[2].0 - level3_blur)
                    .abs()
                    .total_cmp(&(b.tiers()[2].0 - level3_blur).abs())
            })
            .unwrap_or_default()
    }
}

/// How wide scrollbars are drawn and grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scrollbars {
    Thin,
    #[default]
    Standard,
    Wide,
}

impl Scrollbars {
    pub const ALL: &'static [Scrollbars] =
        &[Scrollbars::Thin, Scrollbars::Standard, Scrollbars::Wide];

    pub fn label(self) -> &'static str {
        match self {
            Scrollbars::Thin => tr("shell-thin"),
            Scrollbars::Standard => tr("shell-standard"),
            Scrollbars::Wide => tr("shell-wide"),
        }
    }

    /// Drawn width and grab width.
    pub fn widths(self) -> (f32, f32) {
        match self {
            Scrollbars::Thin => (3.0, 12.0),
            Scrollbars::Standard => (5.0, 15.0),
            Scrollbars::Wide => (9.0, 18.0),
        }
    }

    fn nearest(width: f32) -> Scrollbars {
        Self::ALL
            .iter()
            .copied()
            .min_by(|a, b| {
                (a.widths().0 - width)
                    .abs()
                    .total_cmp(&(b.widths().0 - width).abs())
            })
            .unwrap_or_default()
    }
}

/// The whole simplified selection.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Choices {
    pub scheme: Scheme,
    pub corners: Corners,
    pub density: Density,
    pub shadows: Shadows,
    pub scrollbars: Scrollbars,
}

const RADIUS: [&str; 6] = [
    "radius.xs",
    "radius.sm",
    "radius.md",
    "radius.lg",
    "radius.xl",
    "radius.xxl",
];
const SIZING: [&str; 3] = [
    "sizing.control_height",
    "sizing.row_height",
    "sizing.min_hit_target",
];
// The framework's spacing ladder (paddings and gaps in every app). Rows have
// no shared height token (`sizing.row_height` is read-only), so density reaches
// the apps through their spacing.
const SPACING: [&str; 16] = [
    "spacing.space0",
    "spacing.space1",
    "spacing.space2",
    "spacing.space3",
    "spacing.space4",
    "spacing.space5",
    "spacing.space6",
    "spacing.space7",
    "spacing.space8",
    "spacing.space9",
    "spacing.space10",
    "spacing.space11",
    "spacing.space12",
    "spacing.space13",
    "spacing.space14",
    "spacing.space15",
];
/// The default spacing ladder, rung by rung.
const SPACING_DEFAULT: [f32; 16] = [
    0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 56.0, 64.0,
];
const ELEVATION: [[&str; 2]; 5] = [
    ["elevation.level1.blur", "elevation.level1.y_offset"],
    ["elevation.level2.blur", "elevation.level2.y_offset"],
    ["elevation.level3.blur", "elevation.level3.y_offset"],
    ["elevation.level4.blur", "elevation.level4.y_offset"],
    ["elevation.level5.blur", "elevation.level5.y_offset"],
];
const SCROLLBAR: [&str; 2] = ["scrollbar.width", "scrollbar.hit_width"];
// Window shadows follow the same preset: apps draw their own frames (and
// shadows) on enzimOS.
const WINDOW_SHADOW: [&str; 3] = [
    "window_shadow.size",
    "window_shadow.opacity",
    "window_shadow.inactive_opacity",
];

/// The radius tokens a corner style sets, the window's included.
pub fn corner_tokens(corners: Corners) -> impl Iterator<Item = (&'static str, f32)> {
    RADIUS
        .into_iter()
        .zip(corners.rungs())
        .chain(std::iter::once((
            "window.radius",
            f32::from(corners.window_radius()),
        )))
}

/// Every token path the simplified Settings surface writes.
pub fn managed_paths() -> impl Iterator<Item = &'static str> {
    crate::theme_palette::PATHS
        .into_iter()
        .chain(RADIUS)
        .chain(SIZING)
        .chain(SPACING)
        .chain(ELEVATION.into_iter().flatten())
        .chain(SCROLLBAR)
        .chain(WINDOW_SHADOW)
        .chain([
            "window.radius",
            "window.buttons",
            "stroke.focus_ring",
            "window.frame",
        ])
        .chain(crate::theme_store::BUTTON_SHAPES)
}

// Compare the displayed choices so unrelated edits preserve custom and future values.
pub(crate) fn apply_changes(
    document: &mut ThemeDocument,
    before: Choices,
    choices: Choices,
) -> usize {
    let mut written = 0;
    if before.scheme != choices.scheme {
        for mode in [EnzThemeMode::Dark, EnzThemeMode::Light] {
            written += apply_scheme(document, choices.scheme, mode);
        }
    }
    if before.corners != choices.corners {
        written += apply_corners(document, choices.corners);
    }
    if before.density != choices.density {
        written += apply_density(document, choices.density);
    }
    if before.shadows != choices.shadows {
        written += apply_shadows(document, choices.shadows);
    }
    if before.scheme != choices.scheme || before.shadows != choices.shadows {
        for mode in [EnzThemeMode::Dark, EnzThemeMode::Light] {
            written += apply_shadow_color(document, choices, mode);
        }
    }
    if before.scrollbars != choices.scrollbars {
        written += apply_scrollbars(document, choices.scrollbars);
    }
    written
}

#[cfg(test)]
fn apply(document: &mut ThemeDocument, choices: Choices) -> usize {
    let before = detect(document);
    apply_changes(document, before, choices)
}

fn apply_scheme(document: &mut ThemeDocument, scheme: Scheme, mode: EnzThemeMode) -> usize {
    if scheme.is_builtin() {
        // Clearing restores the built-in value instead of pinning a copy of
        // it, so a later framework retune still reaches the user.
        return crate::theme_palette::PATHS
            .iter()
            .filter(|p| document.clear(p, mode))
            .count();
    }
    crate::theme_palette::generate(scheme, mode)
        .tokens()
        .iter()
        .filter(|(path, color)| document.set(path, mode, TokenValue::Color(*color)))
        .count()
}

// Mode-independent tokens use the dark storage key.
fn set_or_clear(
    document: &mut ThemeDocument,
    values: impl IntoIterator<Item = (&'static str, f32)>,
    default: bool,
) -> usize {
    values
        .into_iter()
        .filter(|(path, value)| {
            if default {
                document.clear(path, EnzThemeMode::Dark)
            } else {
                document.set(path, EnzThemeMode::Dark, TokenValue::Number(*value))
            }
        })
        .count()
}

fn apply_corners(document: &mut ThemeDocument, corners: Corners) -> usize {
    set_or_clear(
        document,
        RADIUS.into_iter().zip(corners.rungs()),
        corners == Corners::Rounded,
    )
}

fn apply_density(document: &mut ThemeDocument, density: Density) -> usize {
    let (control, _, hit) = density.sizing();
    // `sizing.row_height` is read-only in the framework: only ever cleared.
    let cleared = usize::from(document.clear(SIZING[1], EnzThemeMode::Dark));
    // Control height alone left every list, toolbar and keypad where it was;
    // the spacing ladder is what apps pad and space their rows with.
    let factor = density.spacing_factor();
    cleared
        + set_or_clear(
            document,
            [(SIZING[0], control), (SIZING[2], hit)],
            density == Density::Comfortable,
        )
        + set_or_clear(
            document,
            SPACING
                .into_iter()
                .zip(SPACING_DEFAULT.map(|rung| (rung * factor).round())),
            density == Density::Comfortable,
        )
}

fn apply_shadows(document: &mut ThemeDocument, shadows: Shadows) -> usize {
    let values = ELEVATION
        .into_iter()
        .zip(shadows.tiers())
        .flat_map(|([blur, y], (b, o))| [(blur, b), (y, o)]);
    set_or_clear(document, values, shadows == Shadows::Standard)
        + set_or_clear(
            document,
            WINDOW_SHADOW.into_iter().zip(shadows.window()),
            shadows == Shadows::Standard,
        )
}

// Longer shadow falloff also needs stronger colour to retain visible density.
fn apply_shadow_color(document: &mut ThemeDocument, choices: Choices, mode: EnzThemeMode) -> usize {
    if choices.shadows == Shadows::Standard {
        return if choices.scheme.is_builtin() {
            usize::from(document.clear("palette.shadow", mode))
        } else {
            let color = crate::theme_palette::shown(choices.scheme, mode).get("shadow");
            usize::from(document.set("palette.shadow", mode, TokenValue::Color(color)))
        };
    }
    let base = crate::theme_palette::shown(choices.scheme, mode).get("shadow");
    let color = Color {
        a: (base.a * choices.shadows.darkness()).min(0.6),
        ..base
    };
    usize::from(document.set("palette.shadow", mode, TokenValue::Color(color)))
}

fn apply_scrollbars(document: &mut ThemeDocument, scrollbars: Scrollbars) -> usize {
    let (width, hit) = scrollbars.widths();
    set_or_clear(
        document,
        [(SCROLLBAR[0], width), (SCROLLBAR[1], hit)],
        scrollbars == Scrollbars::Standard,
    )
}

/// Detects nearest presets, using defaults for unpinned tokens and neutral planes for legacy
/// accent-only files.
pub fn detect(document: &ThemeDocument) -> Choices {
    let scheme = match color_at(document, "palette.accent", EnzThemeMode::Dark) {
        Some(accent) => Scheme {
            style: Style::Neutral,
            seed: Seed::Color(accent),
        },
        None => Scheme::default(),
    };
    Choices {
        scheme,
        corners: number_at(document, "radius.md").map_or_else(Corners::default, Corners::nearest),
        density: number_at(document, "sizing.control_height")
            .map_or_else(Density::default, Density::nearest),
        shadows: number_at(document, "elevation.level3.blur")
            .map_or_else(Shadows::default, Shadows::nearest),
        scrollbars: number_at(document, "scrollbar.width")
            .map_or_else(Scrollbars::default, Scrollbars::nearest),
    }
}

fn number_at(document: &ThemeDocument, path: &str) -> Option<f32> {
    match document.get(path, EnzThemeMode::Dark) {
        Some(TokenValue::Number(n)) => Some(*n),
        _ => None,
    }
}

fn color_at(document: &ThemeDocument, path: &str, mode: EnzThemeMode) -> Option<Color> {
    match document.get(path, mode) {
        Some(TokenValue::Color(c)) => Some(*c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(choices: Choices) -> Choices {
        let mut document = ThemeDocument::empty();
        apply(&mut document, choices);
        let parsed = ThemeDocument::parse(&document.to_toml()).expect("generated theme parses");
        Choices {
            scheme: choices.scheme,
            ..detect(&parsed)
        }
    }

    #[test]
    fn theme_presets_behaviour() {
        // Every shape combination survives a write and reread.
        {
            for &corners in Corners::ALL {
                for &density in Density::ALL {
                    for &shadows in Shadows::ALL {
                        for &scrollbars in Scrollbars::ALL {
                            let choices = Choices {
                                corners,
                                density,
                                shadows,
                                scrollbars,
                                ..Choices::default()
                            };
                            assert_eq!(round_trip(choices), choices, "{choices:?}");
                        }
                    }
                }
            }
        }
        // Settings writes leave unrelated studio tokens alone.
        {
            let mut document = ThemeDocument::parse(
                "version = 1\n[typography]\nbody_size = 17.0\n[dark.palette]\ndanger = \"#ff0000\"\n",
            )
            .expect("fixture parses");
            let scheme = Scheme {
                style: Style::Vibrant,
                seed: Seed::Color(crate::theme_palette::rgb(0x00a5a8)),
            };
            apply(
                &mut document,
                Choices {
                    scheme,
                    corners: Corners::Soft,
                    density: Density::Spacious,
                    shadows: Shadows::Strong,
                    scrollbars: Scrollbars::Wide,
                },
            );
            assert_eq!(
                document.get("typography.body_size", EnzThemeMode::Dark),
                Some(&TokenValue::Number(17.0))
            );
            assert_eq!(
                color_at(&document, "palette.danger", EnzThemeMode::Dark),
                Some(Color::from_rgb(1.0, 0.0, 0.0))
            );
        }
    }
}
