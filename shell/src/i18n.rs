use enzui_core::localization::{
    self, Arguments, Catalog, CatalogBuilder, DateStyle, Locale, Localizer,
};
use std::{collections::BTreeMap, sync::OnceLock};

const ENGLISH: &str = include_str!("locales/en-US.ftl");
const SPANISH: &str = include_str!("locales/es.ftl");
const WELCOME_ENGLISH: &str = include_str!("locales/welcome/en-US.ftl");
const WELCOME_SPANISH: &str = include_str!("locales/welcome/es.ftl");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Automatic,
    English,
    Spanish,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::English, Self::Spanish];

    pub fn key(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::English => "en-US",
            Self::Spanish => "es",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|language| language.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => tr("language-automatic"),
            Self::English => "English",
            Self::Spanish => "Español",
        }
    }

    fn preferred(self) -> Vec<Locale> {
        match self {
            Self::Automatic => localization::preferred_locales(),
            Self::English => vec![Locale::default()],
            Self::Spanish => vec!["es".parse().expect("a supported locale")],
        }
    }
}

pub(crate) fn catalog() -> Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let mut builder = CatalogBuilder::new();
            builder
                .add_resource("en-US", ENGLISH)
                .expect("valid embedded English messages");
            builder
                .add_resource("es", SPANISH)
                .expect("valid embedded Spanish messages");
            builder
                .add_resource("en-US", WELCOME_ENGLISH)
                .expect("valid embedded English Welcome messages");
            builder
                .add_resource("es", WELCOME_SPANISH)
                .expect("valid embedded Spanish Welcome messages");
            builder.build()
        })
        .clone()
}

pub fn localizer() -> &'static Localizer {
    static LOCALIZER: OnceLock<Localizer> = OnceLock::new();
    LOCALIZER.get_or_init(|| {
        Localizer::new(
            catalog(),
            Language::Automatic.preferred(),
            Locale::default(),
        )
    })
}

pub fn initialize() {
    if let Some(settings) =
        crate::settings::Store::from_environment().and_then(|store| store.load().ok())
    {
        select(settings.language);
    }
}

pub fn select(language: Language) -> bool {
    localizer().select(language.preferred())
}

// Installed declarations must remain identical to the packaged metadata in every UI language.
pub fn canonical<R>(build: impl FnOnce() -> R) -> R {
    static ENGLISH_LOCALIZER: OnceLock<Localizer> = OnceLock::new();
    ENGLISH_LOCALIZER
        .get_or_init(|| Localizer::new(catalog(), vec![Locale::default()], Locale::default()))
        .scope(build)
}

pub fn current() -> Localizer {
    localization::scoped().unwrap_or_else(|| localizer().clone())
}

// Metadata getters borrow labels; immutable catalog caches keep those borrows valid across language switches.
fn texts() -> &'static [BTreeMap<&'static str, String>; 2] {
    static TEXT: OnceLock<[BTreeMap<&'static str, String>; 2]> = OnceLock::new();
    TEXT.get_or_init(|| {
        [Locale::default(), "es".parse().expect("a supported locale")].map(|locale| {
            let localizer = Localizer::new(catalog(), vec![locale], Locale::default());
            ENGLISH
                .lines()
                .chain(WELCOME_ENGLISH.lines())
                .filter_map(|line| {
                    line.split_once(" = ")
                        .filter(|(_, value)| !value.contains('$'))
                        .map(|(key, _)| key)
                })
                .map(|key| (key, localizer.text(key)))
                .collect()
        })
    })
}

pub fn english(key: &str) -> &'static str {
    texts()[0].get(key).map_or("…", String::as_str)
}

pub fn tr(key: &'static str) -> &'static str {
    let index = usize::from(current().locale().to_string().split('-').next() == Some("es"));
    texts()[index].get(key).map_or(key, String::as_str)
}

pub fn message(key: &str, arguments: &Arguments<'_>) -> String {
    current().text_with(key, arguments)
}

pub fn date(date: chrono::NaiveDate, full: bool) -> String {
    use chrono::Datelike;
    enzui_core::civil_date::Date::new(date.year(), date.month(), date.day())
        .and_then(|date| {
            current()
                .locale()
                .format_date(
                    date,
                    if full {
                        DateStyle::Full
                    } else {
                        DateStyle::Medium
                    },
                )
                .ok()
        })
        .unwrap_or_else(|| date.to_string())
}

pub fn time(time: &impl chrono::Timelike, clock_24h: bool) -> String {
    let cycle = if clock_24h { "h23" } else { "h12" };
    let locale = current().locale().to_string();
    let base = locale.split("-u-").next().unwrap_or("en-US");
    let locale = format!("{base}-u-hc-{cycle}")
        .parse::<Locale>()
        .unwrap_or_default();
    enzui_core::time_of_day::TimeOfDay::new(time.hour(), time.minute(), time.second())
        .and_then(|time| locale.format_time(time, false).ok())
        .unwrap_or_default()
}

#[macro_export]
macro_rules! tr {
    ($key:expr $(, $name:literal => $value:expr)* $(,)?) => {{
        let mut arguments = enzui_core::localization::Arguments::new();
        $(arguments.set($name, $value);)*
        $crate::i18n::message($key, &arguments)
    }};
}

pub fn clock_date(date: chrono::NaiveDate, short: bool) -> String {
    struct CachedDate {
        day: chrono::NaiveDate,
        short: bool,
        locale: String,
        generation: u64,
        text: String,
    }
    thread_local! { static CACHED: std::cell::RefCell<Option<CachedDate>> = const { std::cell::RefCell::new(None) }; }
    let localizer = current();
    let locale = localizer.locale().to_string();
    let generation = localizer.generation();
    CACHED.with(|cached| {
        let mut cached = cached.borrow_mut();
        if let Some(value) = cached.as_ref().filter(|value| {
            value.day == date
                && value.short == short
                && value.locale == locale
                && value.generation == generation
        }) {
            return value.text.clone();
        }
        let text = format_clock_date(date, short);
        *cached = Some(CachedDate {
            day: date,
            short,
            locale,
            generation,
            text: text.clone(),
        });
        text
    })
}

fn format_clock_date(date: chrono::NaiveDate, short: bool) -> String {
    use chrono::Datelike;
    use enzui_core::{calendar_semantics::CalendarLabels, localization::LocaleCalendarLabels};
    let localizer = current();
    let Some(day) = enzui_core::civil_date::Date::new(date.year(), date.month(), date.day()) else {
        return date.to_string();
    };
    let Ok(labels) = LocaleCalendarLabels::new(&localizer) else {
        return date.to_string();
    };
    let mut arguments = Arguments::new();
    arguments.set(
        "weekday",
        if short {
            labels.weekday_short(day.weekday())
        } else {
            labels.weekday_name(day.weekday())
        },
    );
    arguments.set("day", day.day());
    arguments.set(
        "month",
        if short {
            labels.month_short(day.month())
        } else {
            labels.month_name(day.month())
        },
    );
    localizer.text_with("clock-date", &arguments)
}

pub fn label(value: &str) -> &str {
    static KEYS: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    let keys = KEYS.get_or_init(|| {
        texts()
            .iter()
            .flat_map(|catalog| catalog.iter().map(|(key, text)| (text.as_str(), *key)))
            .collect()
    });
    keys.get(value).map_or(value, |key| tr(key))
}

pub fn desktop_text<'a>(fields: &BTreeMap<&str, &'a str>, key: &str) -> Option<&'a str> {
    let locale = current().locale().to_string().replace('-', "_");
    let language = locale.split('_').next().unwrap_or("en");
    fields
        .get(format!("{key}[{locale}]").as_str())
        .or_else(|| fields.get(format!("{key}[{language}]").as_str()))
        .or_else(|| fields.get(key))
        .copied()
}

pub fn count(key: &str, count: impl Into<localization::Value<'static>>) -> String {
    let mut arguments = Arguments::new();
    arguments.set("count", count);
    current().text_with(key, &arguments)
}

pub fn number(text: String) -> String {
    text.parse::<localization::Decimal>()
        .ok()
        .and_then(|decimal| current().locale().format_decimal(&decimal).ok())
        .unwrap_or(text)
}

pub fn argument(value: &impl std::fmt::Display) -> String {
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i18n_behaviour() {
        // Catalogs resolve without fallback or format errors.
        {
            let mut arguments = Arguments::new();
            for source in [ENGLISH, SPANISH, WELCOME_ENGLISH, WELCOME_SPANISH] {
                for word in source.split('$').skip(1) {
                    let name: String = word
                        .chars()
                        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '_' | '-'))
                        .collect();
                    arguments.set(name, 2);
                }
            }
            for (locale, sources) in [
                ("en-US", [ENGLISH, WELCOME_ENGLISH]),
                ("es", [SPANISH, WELCOME_SPANISH]),
            ] {
                let localizer =
                    Localizer::new(catalog(), vec![locale.parse().unwrap()], Locale::default());
                for source in sources {
                    for key in source
                        .lines()
                        .filter_map(|line| line.split_once(" = ").map(|(key, _)| key))
                    {
                        let resolved = localizer.resolve(key, Some(&arguments));
                        assert!(
                            resolved.errors.is_empty(),
                            "{locale}/{key}: {:?}",
                            resolved.errors
                        );
                        assert_eq!(
                            resolved.locale.as_deref(),
                            Some(locale),
                            "{locale}/{key} fell back"
                        );
                    }
                }
            }
        }
        // Spanish scope translates ui and dates without changing shortcut protocols.
        {
            let localizer =
                Localizer::new(catalog(), vec!["es-MX".parse().unwrap()], Locale::default());
            localizer.scope(|| {
                assert_eq!(
                    crate::settings_catalog::Category::Appearance.title(),
                    "Apariencia"
                );
                assert_eq!(Language::Spanish.key(), "es");
                assert_eq!(tr("shell-restart"), "Reiniciar");
                assert_eq!(count("duration-hours", 1), "1 hora");
                assert!(count("duration-hours", 2).contains("horas"));
                assert_eq!(number("12.5".into()), "12.5");
                let capture_directory = tempfile::tempdir().unwrap();
                let (path, file) =
                    crate::capture::new_file(capture_directory.path(), "Screenshot", "png")
                        .unwrap();
                drop(file);
                assert!(
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("Screenshot from ")
                );
                assert!(path.file_name().unwrap().to_string_lossy().is_ascii());
                let lua = crate::session_restore::launch_lua(
                    std::path::Path::new("/usr/share/applications/demo.desktop"),
                    "demo.app",
                    &crate::session_restore::Saved {
                        id: "demo.desktop".into(),
                        workspace: 1,
                        place: None,
                    },
                    &["DP-1".into()],
                    0,
                    true,
                )
                .unwrap();
                assert!(lua.contains("gio launch"));
                assert!(lua.is_ascii());
                assert!(
                    clock_date(
                        chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
                        false
                    )
                    .contains("octubre")
                );
                assert!(
                    crate::settings_catalog::search("contraseña")
                        .iter()
                        .any(|entry| entry.category == crate::settings_catalog::Category::Power)
                );
                assert!(
                    crate::settings_catalog::search("wallpaper")
                        .iter()
                        .any(|entry| entry.route == "wallpaper/source")
                );
                for chord in ["SUPER + Return", "ALT + Left", "CTRL + Delete"] {
                    let shortcut = crate::shortcut_map::from_chord(chord).unwrap();
                    assert_eq!(
                        crate::shortcut_map::to_chord(&shortcut).as_deref(),
                        Some(chord)
                    );
                }
            });
            let directory = tempfile::tempdir().unwrap();
            let store = crate::settings::Store::at(directory.path().join("settings.json")).unwrap();
            store
                .update(crate::settings::Change::Language(Language::Spanish))
                .unwrap();
            assert_eq!(store.load().unwrap().language, Language::Spanish);
            let agent = crate::settings_agent::find("desktop.language").unwrap();
            assert_eq!(
                agent.value(&store.load().unwrap(), &Default::default()),
                Some(serde_json::json!("es"))
            );
            assert!(
                agent
                    .target(&serde_json::json!("fr"), &Default::default())
                    .is_err()
            );
            assert_eq!(Language::from_key("es"), Some(Language::Spanish));
            assert_eq!(Language::from_key("invalid"), None);
        }
    }
}
