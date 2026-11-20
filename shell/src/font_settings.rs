//! Desktop font choices persisted with shell settings and published by the Settings portal.

use enzui_core::theme_file::{ThemeDocument, registry::TokenValue};
use enzui_shell::i18n::tr;
use enzui_theme::EnzThemeMode;
use std::sync::OnceLock;

/// Which family a choice sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Interface,
    Document,
    Monospace,
}

impl Slot {
    pub const ALL: [Slot; 3] = [Slot::Interface, Slot::Document, Slot::Monospace];

    pub fn path(self) -> &'static str {
        match self {
            Slot::Interface => "fonts.ui",
            Slot::Document => "fonts.document",
            Slot::Monospace => "fonts.monospace",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Slot::Interface => tr("shell-interface-font"),
            Slot::Document => tr("shell-document-font"),
            Slot::Monospace => tr("shell-monospace-font"),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Slot::Interface => tr("shell-menus-panels-buttons-and-labels"),
            Slot::Document => tr("shell-long-form-reading-text-in-documents-and-articles"),
            Slot::Monospace => tr("shell-terminals-code-and-fixed-width-text"),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Slot::Interface => "ui",
            Slot::Document => "document",
            Slot::Monospace => "monospace",
        }
    }
}

/// How heavy all text reads: every weight rung shifts by the same step, so
/// headings stay heavier than body text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    Thin,
    Light,
    #[default]
    Regular,
    Medium,
    Heavy,
}

const RUNGS: [&str; 5] = [
    "typography.weight_regular",
    "typography.weight_medium",
    "typography.weight_semibold",
    "typography.weight_bold",
    "typography.weight_extrabold",
];
const DEFAULT_RUNGS: [f32; 5] = [400.0, 500.0, 600.0, 700.0, 800.0];

impl Weight {
    pub const ALL: [Weight; 5] = [
        Weight::Thin,
        Weight::Light,
        Weight::Regular,
        Weight::Medium,
        Weight::Heavy,
    ];

    fn offset(self) -> f32 {
        match self {
            Weight::Thin => -200.0,
            Weight::Light => -100.0,
            Weight::Regular => 0.0,
            Weight::Medium => 100.0,
            Weight::Heavy => 200.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Weight::Thin => tr("shell-thin"),
            Weight::Light => tr("font-weight-light"),
            Weight::Regular => tr("shell-regular"),
            Weight::Medium => tr("font-weight-medium"),
            Weight::Heavy => tr("shell-heavy"),
        }
    }

    pub fn rungs(self) -> [f32; 5] {
        DEFAULT_RUNGS.map(|rung| (rung + self.offset()).clamp(100.0, 900.0))
    }

    /// Offers presets resolving to distinct upright faces; unknown families retain all presets.
    pub fn available(faces: &[u16]) -> Vec<Weight> {
        if faces.is_empty() {
            return Self::ALL.to_vec();
        }
        let nearest = |target: f32| {
            faces
                .iter()
                .copied()
                .min_by(|a, b| {
                    (f32::from(*a) - target)
                        .abs()
                        .total_cmp(&(f32::from(*b) - target).abs())
                })
                .unwrap_or(400)
        };
        let regular = nearest(400.0);
        let mut lighter: Vec<(Weight, u16)> = Vec::new();
        for weight in [Weight::Light, Weight::Thin] {
            let face = nearest(weight.rungs()[0]);
            if face < regular && lighter.last().is_none_or(|(_, last)| face < *last) {
                lighter.push((weight, face));
            }
        }
        let mut heavier: Vec<(Weight, u16)> = Vec::new();
        for weight in [Weight::Medium, Weight::Heavy] {
            let face = nearest(weight.rungs()[0]);
            if face > regular && heavier.last().is_none_or(|(_, last)| face > *last) {
                heavier.push((weight, face));
            }
        }
        lighter
            .iter()
            .rev()
            .map(|(w, _)| *w)
            .chain([Weight::Regular])
            .chain(heavier.iter().map(|(w, _)| *w))
            .collect()
    }

    fn nearest(regular: f32) -> Weight {
        Self::ALL
            .into_iter()
            .min_by(|a, b| {
                (a.rungs()[0] - regular)
                    .abs()
                    .total_cmp(&(b.rungs()[0] - regular).abs())
            })
            .unwrap_or_default()
    }
}

/// What the file currently says about fonts.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Fonts {
    #[serde(rename = "ui")]
    pub interface: Option<String>,
    pub document: Option<String>,
    pub monospace: Option<String>,
    pub weight: Weight,
}

impl Fonts {
    pub fn read(document: &ThemeDocument) -> Self {
        let family = |slot: Slot| match document.get(slot.path(), EnzThemeMode::Dark) {
            Some(TokenValue::FontFamily(name)) => Some(name.clone()),
            _ => None,
        };
        let weight = match document.get(RUNGS[0], EnzThemeMode::Dark) {
            Some(TokenValue::Number(regular)) => Weight::nearest(*regular),
            _ => Weight::Regular,
        };
        Self {
            interface: family(Slot::Interface),
            document: family(Slot::Document),
            monospace: family(Slot::Monospace),
            weight,
        }
    }

    pub fn family(&self, slot: Slot) -> Option<&str> {
        match slot {
            Slot::Interface => self.interface.as_deref(),
            Slot::Document => self.document.as_deref(),
            Slot::Monospace => self.monospace.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontChange {
    Family(Slot, Option<String>),
    Weight(Weight),
}

impl Fonts {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let fonts: Self = match value {
            None => Self::default(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| "Invalid desktop font preferences".to_owned())?,
        };
        for slot in Slot::ALL {
            validate_family(fonts.family(slot))?;
        }
        Ok(fonts)
    }

    pub fn apply(&mut self, change: FontChange) -> Result<(), String> {
        match change {
            FontChange::Family(slot, family) => {
                validate_family(family.as_deref())?;
                let family = family.map(|name| name.trim().to_owned());
                match slot {
                    Slot::Interface => self.interface = family,
                    Slot::Document => self.document = family,
                    Slot::Monospace => self.monospace = family,
                }
            }
            FontChange::Weight(weight) => self.weight = weight,
        }
        Ok(())
    }

    pub fn update_json(value: &mut serde_json::Value, change: &FontChange) -> Result<(), String> {
        let mut next = Self::from_json(value.get("fonts"))?;
        next.apply(change.clone())?;
        if value.get("fonts").is_none() {
            value["fonts"] = serde_json::json!({});
        }
        let fonts = value["fonts"]
            .as_object_mut()
            .ok_or("Invalid desktop font preferences")?;
        match change {
            FontChange::Family(slot, Some(_)) => {
                fonts.insert(
                    slot.key().into(),
                    next.family(*slot).unwrap_or_default().into(),
                );
            }
            FontChange::Family(slot, None) => {
                fonts.remove(slot.key());
            }
            FontChange::Weight(Weight::Regular) => {
                fonts.remove("weight");
            }
            FontChange::Weight(weight) => {
                fonts.insert(
                    "weight".into(),
                    serde_json::to_value(weight).map_err(|_| "Invalid font weight")?,
                );
            }
        }
        Ok(())
    }

    pub fn apply_to_theme(
        &self,
        theme: &mut enzui_theme::EnzTheme,
        document: &ThemeDocument,
        mode: EnzThemeMode,
    ) {
        use enzui_core::font_preferences::{DesktopFontPreferences, FontPreferencePolicy};
        let mut preferences = DesktopFontPreferences::new();
        for (slot, role) in [
            (Slot::Interface, enzui_theme::FontRole::Ui),
            (Slot::Document, enzui_theme::FontRole::Document),
            (Slot::Monospace, enzui_theme::FontRole::Monospace),
        ] {
            preferences = preferences.with(role, self.description(slot));
        }
        let mut policy = FontPreferencePolicy::new();
        policy.set_platform(Some(preferences));
        enzui_core::font_preferences::apply_to_theme(theme, document, mode, &policy);
    }

    pub fn description(&self, slot: Slot) -> String {
        let family = self.family(slot).unwrap_or_else(|| {
            if slot == Slot::Monospace {
                "Monospace"
            } else {
                default_sans()
            }
        });
        let weight = match self.weight {
            Weight::Thin => "ExtraLight",
            Weight::Light => "Light",
            Weight::Regular => "Regular",
            Weight::Medium => "Medium",
            Weight::Heavy => "Semibold",
        };
        // The accessibility text scale is published separately; 9.75 pt is the desktop's 13 px base.
        format!("{family}, {weight} 9.75")
    }
}

fn validate_family(family: Option<&str>) -> Result<(), String> {
    if family.is_some_and(|name| {
        name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control)
    }) {
        return Err("Invalid desktop font family".into());
    }
    Ok(())
}

pub fn migrate_legacy(
    store: &crate::settings::Store,
    paths: &enzui_core::theme_file::ThemePaths,
) -> Result<(), String> {
    use enzui_core::{
        theme_file::MAX_THEME_FILE_BYTES,
        user_file::{DocumentAuthority, DocumentSnapshot},
    };
    let Some(path) = paths.clone().without_app().user_path() else {
        return Ok(());
    };
    let mut authority = match DocumentAuthority::open_existing(&path) {
        Ok(authority) => authority,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(authority_error) => {
            // Linked or otherwise read-only themes can supply legacy choices,
            // but only the retained authority may remove their old tokens.
            let load =
                enzui_core::theme_file::ThemeLoadOp::new(&paths.clone().without_app(), 0).execute();
            if load.path.as_ref() != Some(&path) {
                return Err(authority_error.to_string());
            }
            let document = load.outcome.map_err(|error| error.to_string())?;
            return match legacy_fonts(&document) {
                Some((fonts, _)) => store.import_fonts_if_absent(&fonts),
                None => Ok(()),
            };
        }
    };
    let DocumentSnapshot::Current {
        bytes,
        overflow: false,
        ..
    } = authority
        .read_bounded(MAX_THEME_FILE_BYTES as usize)
        .map_err(|error| error.to_string())?
    else {
        return Err("Cannot migrate changed or oversized desktop theme".into());
    };
    let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let mut document = ThemeDocument::parse(text).map_err(|error| error.to_string())?;
    let Some((fonts, preset)) = legacy_fonts(&document) else {
        return Ok(());
    };
    // Persist the replacement before removing legacy keys. An existing fonts object,
    // including an explicit reset, wins over the old shared theme.
    store.import_fonts_if_absent(&fonts)?;
    for slot in Slot::ALL {
        document.clear(slot.path(), EnzThemeMode::Dark);
    }
    if preset {
        for path in RUNGS {
            document.clear(path, EnzThemeMode::Dark);
        }
    }
    let DocumentSnapshot::Current {
        bytes: current,
        overflow: false,
        ..
    } = authority
        .read_bounded(MAX_THEME_FILE_BYTES as usize)
        .map_err(|error| error.to_string())?
    else {
        return Err("Desktop theme changed during font migration".into());
    };
    if current != bytes {
        return Err("Desktop theme changed during font migration".into());
    }
    authority
        .replace(document.to_toml().as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn legacy_fonts(document: &ThemeDocument) -> Option<(Fonts, bool)> {
    let fonts = Fonts::read(document);
    let preset = Weight::ALL.into_iter().any(|weight| {
        RUNGS.iter().zip(weight.rungs()).all(|(path, value)| {
            document.get(path, EnzThemeMode::Dark) == Some(&TokenValue::Number(value))
        })
    });
    if !preset
        && Slot::ALL
            .into_iter()
            .all(|slot| fonts.family(slot).is_none())
    {
        return None;
    }
    Some((fonts, preset))
}

/// The desktop's own UI family when none is chosen: the first installed of a
/// few with true weights, so shell and apps (which follow the desktop font)
/// share one face; otherwise the generic sans-serif.
pub fn default_sans() -> &'static str {
    static DEFAULT: OnceLock<&'static str> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        const PREFERRED: [&str; 3] = ["Inter", "Adwaita Sans", "Noto Sans"];
        let families = installed();
        PREFERRED
            .into_iter()
            .find(|name| families.iter().any(|f| f.name.eq_ignore_ascii_case(name)))
            .unwrap_or("Sans")
    })
}

/// An installed font family.
#[derive(Clone, Debug)]
pub struct Family {
    pub name: &'static str,
    pub monospace: bool,
}

/// Returns installed font families for the desktop font chooser.
pub fn installed() -> &'static [Family] {
    crate::lock_fonts::load();
    static FAMILIES: OnceLock<Vec<Family>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        // `raw` needs the write lock; this runs once per process.
        let mut system = iced::advanced::graphics::text::font_system()
            .write()
            .expect("font system lock");
        let mut families: std::collections::BTreeMap<String, bool> = Default::default();
        for face in system.raw().db().faces() {
            let Some((name, _)) = face.families.first() else {
                continue;
            };
            let usable = !name.is_empty() && !name.starts_with('.') && !name.contains("Icons");
            if usable {
                *families.entry(name.clone()).or_insert(true) &= face.monospaced;
            }
        }
        families
            .into_iter()
            .map(|(name, monospace)| Family {
                name: Box::leak(name.into_boxed_str()),
                monospace,
            })
            .collect()
    })
}

/// Families matching `query` (case-insensitive), monospaced ones only for the
/// monospace slot, capped at `limit`.
pub fn matches(query: &str, slot: Slot, limit: usize) -> Vec<&'static Family> {
    let needle = query.trim().to_lowercase();
    installed()
        .iter()
        .filter(|family| slot != Slot::Monospace || family.monospace)
        .filter(|family| needle.is_empty() || family.name.to_lowercase().contains(&needle))
        .take(limit)
        .collect()
}

/// Returns the selected interface font for unpinned text; publish through
/// iced::font::set_defaults on change.
pub fn renderer_default(theme: &enzui_theme::EnzTheme) -> iced::Font {
    use iced::font::{Stretch, Style};
    let regular = theme
        .typography
        .iced_weight(enzui_theme::WeightRung::Regular);
    theme.resolve_font(
        enzui_theme::FontRole::Ui,
        regular,
        Style::Normal,
        Stretch::Normal,
    )
}

/// Hands the theme's families and weight rungs to the framework's text, which
/// reads them when no weight is pinned; `true` when they changed.
pub fn publish_text_defaults(theme: &enzui_theme::EnzTheme) -> bool {
    enzui_theme::text_defaults::publish(theme)
}

/// The weights `family`'s upright faces come in, lightest first; empty when
/// the family is not installed.
pub fn family_weights(family: &enzui_theme::FontFamily) -> Vec<u16> {
    use iced::advanced::graphics::text::cosmic_text::fontdb;
    let mut system = iced::advanced::graphics::text::font_system()
        .write()
        .expect("font system lock");
    let db = system.raw().db();
    let name = match family.to_iced() {
        iced::font::Family::Name(name) => name.to_owned(),
        iced::font::Family::Serif => db.family_name(&fontdb::Family::Serif).to_owned(),
        iced::font::Family::Monospace => db.family_name(&fontdb::Family::Monospace).to_owned(),
        iced::font::Family::Cursive => db.family_name(&fontdb::Family::Cursive).to_owned(),
        iced::font::Family::Fantasy => db.family_name(&fontdb::Family::Fantasy).to_owned(),
        _ => db.family_name(&fontdb::Family::SansSerif).to_owned(),
    };
    let mut weights: Vec<u16> = db
        .faces()
        .filter(|face| {
            face.style == fontdb::Style::Normal
                && face
                    .families
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case(&name))
        })
        .map(|face| face.weight.0)
        .collect();
    weights.sort_unstable();
    weights.dedup();
    weights
}

pub fn renderer_text_size(theme: &enzui_theme::EnzTheme) -> iced::Pixels {
    iced::Pixels(theme.typography.body_size)
}
