//! Desktop preferences published through the Settings portal.
use super::{PATH, owned};
use enzui_shell::i18n::tr;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use zbus::zvariant::{OwnedValue, Value};

use enzui_core::theme_file::{ThemePaths, registry::TokenValue};
use enzui_theme::{EnzThemeMode, WindowButtons};

const APPEARANCE: &str = "org.freedesktop.appearance";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

/// `color-scheme`: 1 prefers dark, 2 prefers light. The desktop is dark
/// unless the account chose light.
#[must_use]
pub fn color_scheme(light: bool) -> u32 {
    if light { 2 } else { 1 }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Preferences {
    light: bool,
    text_scale: f64,
    clock_24h: bool,
    fonts: crate::font_settings::Fonts,
    accent: (f64, f64, f64),
    contrast: bool,
    reduced_motion: bool,
    cursor_theme: String,
    cursor_size: i32,
    icon_theme: String,
    buttons: WindowButtons,
    screen_keyboard: bool,
}

impl Preferences {
    pub(super) fn load() -> Self {
        let settings = crate::settings::Store::from_environment()
            .and_then(|store| {
                if let Err(error) = crate::font_settings::migrate_legacy(
                    &store,
                    &enzui_core::theme_file::ThemePaths::from_env(),
                ) {
                    eprintln!("desktop font migration: {error}");
                }
                store.load().ok()
            })
            .unwrap_or_default();
        Self::from_settings(
            &settings,
            &crate::theme_store::Store::read(&ThemePaths::from_env()),
        )
    }

    fn from_settings(
        settings: &crate::settings::Settings,
        theme: &crate::theme_store::Store,
    ) -> Self {
        let scheme = settings
            .theme_scheme
            .as_deref()
            .and_then(crate::theme_palette::Scheme::from_id_str)
            .unwrap_or_else(|| theme.choices().scheme);
        let mode = if settings.light {
            EnzThemeMode::Light
        } else {
            EnzThemeMode::Dark
        };
        let accent = match theme.document().get("palette.accent", mode) {
            Some(TokenValue::Color(color)) => *color,
            _ => crate::theme_palette::shown(scheme, mode).get("accent"),
        };
        let buttons = match theme.document().get("window.buttons", EnzThemeMode::Dark) {
            Some(TokenValue::Enum(name)) => WindowButtons::from_name(name).unwrap_or_default(),
            _ => WindowButtons::Desktop,
        };
        Self {
            light: settings.light,
            text_scale: f64::from(settings.effective_metrics().text_scale()),
            clock_24h: settings.session.clock_24h,
            fonts: settings.fonts.clone(),
            accent: (
                f64::from(accent.r),
                f64::from(accent.g),
                f64::from(accent.b),
            ),
            contrast: scheme.style == crate::theme_palette::Style::Contrast,
            reduced_motion: theme.reduced_motion(),
            cursor_theme: std::env::var("XCURSOR_THEME")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "default".into()),
            cursor_size: i32::from(settings.compositor.cursor_size),
            icon_theme: settings
                .icon_theme
                .clone()
                .unwrap_or_else(crate::icon_theme::default_theme),
            buttons,
            screen_keyboard: settings.osk_button,
        }
    }

    fn values(&self) -> BTreeMap<String, BTreeMap<String, OwnedValue>> {
        let mut buttons = Vec::new();
        if self.buttons.minimize() == Some(true) {
            buttons.push("minimize");
        }
        if self.buttons.maximize() == Some(true) {
            buttons.push("maximize");
        }
        buttons.push("close");
        let layout = format!(":{}", buttons.join(","));
        let str_value = |value: &str| owned(Value::from(value));
        let groups = [
            (
                APPEARANCE,
                vec![
                    ("color-scheme", color_scheme(self.light).into()),
                    ("accent-color", owned(Value::from(self.accent))),
                    ("contrast", u32::from(self.contrast).into()),
                    ("reduced-motion", u32::from(self.reduced_motion).into()),
                    (
                        "button-placement",
                        owned(Value::from((
                            Vec::<String>::new(),
                            buttons.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                        ))),
                    ),
                ],
            ),
            (
                "org.gnome.desktop.interface",
                vec![
                    (
                        "font-name",
                        str_value(
                            &self
                                .fonts
                                .description(crate::font_settings::Slot::Interface),
                        ),
                    ),
                    (
                        "document-font-name",
                        str_value(&self.fonts.description(crate::font_settings::Slot::Document)),
                    ),
                    (
                        "monospace-font-name",
                        str_value(
                            &self
                                .fonts
                                .description(crate::font_settings::Slot::Monospace),
                        ),
                    ),
                    ("text-scaling-factor", self.text_scale.into()),
                    ("cursor-theme", str_value(&self.cursor_theme)),
                    ("cursor-size", self.cursor_size.into()),
                    ("icon-theme", str_value(&self.icon_theme)),
                    (
                        "gtk-theme",
                        str_value(if self.contrast {
                            "HighContrast"
                        } else {
                            "Adwaita"
                        }),
                    ),
                    ("gtk-enable-primary-paste", true.into()),
                    ("gtk-enable-animations", (!self.reduced_motion).into()),
                    ("enable-animations", (!self.reduced_motion).into()),
                    (
                        "clock-format",
                        str_value(if self.clock_24h { "24h" } else { "12h" }),
                    ),
                    (
                        "color-scheme",
                        str_value(if self.light {
                            "prefer-light"
                        } else {
                            "prefer-dark"
                        }),
                    ),
                    ("overlay-scrolling", true.into()),
                    ("enable-hot-corners", false.into()),
                    ("cursor-blink", true.into()),
                    ("cursor-blink-time", 1200i32.into()),
                    ("cursor-blink-timeout", 10i32.into()),
                    (
                        "gtk-im-module",
                        str_value(&std::env::var("GTK_IM_MODULE").unwrap_or_default()),
                    ),
                    ("font-rendering", str_value("automatic")),
                    ("font-antialiasing", str_value("grayscale")),
                    ("font-hinting", str_value("slight")),
                    ("font-rgba-order", str_value("rgb")),
                    ("gtk-key-theme", str_value("Default")),
                ],
            ),
            (
                "org.gnome.desktop.wm.preferences",
                vec![
                    ("button-layout", str_value(&layout)),
                    (
                        "titlebar-font",
                        str_value(
                            &self
                                .fonts
                                .description(crate::font_settings::Slot::Interface),
                        ),
                    ),
                    ("action-double-click-titlebar", str_value("toggle-maximize")),
                    ("action-middle-click-titlebar", str_value("none")),
                    ("action-right-click-titlebar", str_value("menu")),
                ],
            ),
            (
                "org.gnome.desktop.privacy",
                vec![
                    ("recent-files-max-age", 30i32.into()),
                    ("remember-recent-files", true.into()),
                    ("disable-camera", false.into()),
                    ("disable-microphone", false.into()),
                    ("disable-sound-output", false.into()),
                ],
            ),
            (
                "org.gnome.desktop.a11y",
                vec![("always-show-text-caret", false.into())],
            ),
            (
                "org.gnome.desktop.a11y.interface",
                vec![
                    ("high-contrast", self.contrast.into()),
                    ("show-status-shapes", false.into()),
                    ("keyboard-focus-visible-timeout", (-1i32).into()),
                ],
            ),
            (
                "org.gnome.desktop.a11y.applications",
                vec![
                    ("screen-reader-enabled", false.into()),
                    ("screen-keyboard-enabled", self.screen_keyboard.into()),
                    ("screen-magnifier-enabled", false.into()),
                ],
            ),
            (
                "org.gnome.desktop.peripherals.mouse",
                vec![
                    ("double-click", 400i32.into()),
                    ("drag-threshold", 8i32.into()),
                ],
            ),
            (
                "org.gnome.settings-daemon.peripherals.mouse",
                vec![
                    ("double-click", 400i32.into()),
                    ("drag-threshold", 8i32.into()),
                ],
            ),
            (
                "org.kde.kdeglobals.KDE",
                vec![
                    ("DoubleClickInterval", 400i32.into()),
                    ("StartDragDist", 8i32.into()),
                    ("CursorBlinkRate", 1200i32.into()),
                ],
            ),
            (
                "org.gnome.desktop.sound",
                vec![
                    ("theme-name", str_value("freedesktop")),
                    ("event-sounds", false.into()),
                    ("input-feedback-sounds", false.into()),
                ],
            ),
        ];
        groups
            .into_iter()
            .map(|(namespace, entries)| {
                (
                    namespace.to_owned(),
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value))
                        .collect(),
                )
            })
            .collect()
    }
}

/// Whether a `ReadAll` namespace pattern (exact, or ending in `*`) selects
/// `namespace`; no patterns select everything.
#[must_use]
pub fn selects(patterns: &[String], namespace: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|pattern| {
            pattern.is_empty()
                || pattern == namespace
                || pattern
                    .strip_suffix('*')
                    .is_some_and(|prefix| namespace.starts_with(prefix))
        })
}

pub(super) struct Appearance {
    pub(super) preferences: Mutex<Preferences>,
}

impl Appearance {
    fn preferences(&self) -> Preferences {
        self.preferences
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Appearance {
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn read_all(
        &self,
        namespaces: Vec<String>,
    ) -> BTreeMap<String, BTreeMap<String, OwnedValue>> {
        self.preferences()
            .values()
            .into_iter()
            .filter(|(namespace, _)| selects(&namespaces, namespace))
            .collect()
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.preferences()
            .values()
            .remove(namespace)
            .and_then(|mut values| values.remove(key))
            .ok_or_else(|| PortalError::NotFound(tr("shell-requested-setting-not-found").into()))
    }
    #[zbus(signal)]
    async fn setting_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

struct PreferenceChanges {
    changed: Arc<tokio::sync::Notify>,
    paths: Arc<[std::path::PathBuf]>,
}

impl enzui_core::file_tree_watch::EventSink for PreferenceChanges {
    fn push(&self, event: enzui_core::file_tree_watch::RawEvent) {
        use enzui_core::file_tree_watch::{DirectoryChange, RawEventKind};

        let relevant =
            |path: &std::path::Path| self.paths.iter().any(|target| target.starts_with(path));
        let changed = match event.kind {
            RawEventKind::Change(DirectoryChange::Renamed { from, to }) => {
                relevant(&from) || relevant(&to)
            }
            RawEventKind::Change(
                DirectoryChange::Created(path)
                | DirectoryChange::Deleted(path)
                | DirectoryChange::Modified(path),
            ) => relevant(&path),
            RawEventKind::Overflow | RawEventKind::Vanished => true,
            RawEventKind::ProbeFailed(_) => false,
        };
        if changed {
            self.changed.notify_one();
        }
    }
}

pub(super) async fn follow(
    connection: &zbus::Connection,
    events: tokio::sync::mpsc::UnboundedSender<super::Event>,
) -> Result<(), String> {
    let appearance = connection
        .object_server()
        .interface::<_, Appearance>(PATH)
        .await
        .map_err(|_| tr("shell-could-not-publish-the-appearance-settings"))?;
    let mut paths = ThemePaths::from_env().candidates();
    paths.extend(crate::settings::Store::from_environment().map(|store| store.path().to_owned()));
    paths.extend(crate::location::path());
    let paths: Arc<[_]> = paths.into();
    let stamp = || {
        paths
            .iter()
            .map(|path| {
                std::fs::metadata(path).ok().and_then(|meta| {
                    Some((
                        std::os::unix::fs::MetadataExt::ino(&meta),
                        meta.modified().ok()?,
                    ))
                })
            })
            .collect::<Vec<_>>()
    };
    let changed = Arc::new(tokio::sync::Notify::new());
    // Watch the nearest existing directory so absent theme roots do not require polling.
    let watcher = enzui_core::file_tree_watch::native_directory_watch();
    let install_watches = || {
        let parents: std::collections::BTreeSet<_> = paths
            .iter()
            .filter_map(|path| path.ancestors().skip(1).find(|parent| parent.is_dir()))
            .collect();
        let sink: Arc<dyn enzui_core::file_tree_watch::EventSink> = Arc::new(PreferenceChanges {
            changed: Arc::clone(&changed),
            paths: Arc::clone(&paths),
        });
        let watches: Vec<_> = parents
            .iter()
            .filter_map(|parent| {
                watcher
                    .as_ref()?
                    .watch(
                        parent,
                        enzui_core::file_tree_watch::WatchToken::standalone(),
                        Arc::clone(&sink),
                    )
                    .ok()
            })
            .collect();
        let complete = watches.len() == parents.len();
        (watches, complete)
    };
    let mut watches = install_watches();
    let lockdown = connection
        .object_server()
        .interface::<_, super::portal_lockdown::Lockdown>(PATH)
        .await
        .map_err(|error| error.to_string())?;
    let mut seen = None;
    changed.notify_one();
    loop {
        let fallback = std::time::Duration::from_millis(if watches.1 { 60_000 } else { 750 });
        tokio::select! {
            () = changed.notified() => {
                tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                watches = install_watches();
            }
            () = tokio::time::sleep(fallback) => {
                if !watches.1 {
                    watches = install_watches();
                }
            }
        }
        let now = stamp();
        if Some(&now) == seen.as_ref() {
            continue;
        }
        seen = Some(now);
        lockdown
            .get()
            .await
            .refresh(lockdown.signal_emitter())
            .await;
        let Ok(preferences) = tokio::task::spawn_blocking(Preferences::load).await else {
            continue;
        };
        let previous = {
            let interface = appearance.get().await;
            std::mem::replace(
                &mut *interface
                    .preferences
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                preferences.clone(),
            )
        };
        if previous.light != preferences.light {
            let _ = events.send(super::Event::Appearance {
                light: preferences.light,
            });
        }
        if previous != preferences {
            let old = previous.values();
            for (namespace, values) in preferences.values() {
                for (key, value) in values {
                    if old.get(&namespace).and_then(|values| values.get(&key)) != Some(&value) {
                        let _ = Appearance::setting_changed(
                            appearance.signal_emitter(),
                            &namespace,
                            &key,
                            Value::from(value),
                        )
                        .await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desktop_choices_are_exposed_with_portal_types_and_namespace_filtering() {
        let settings = crate::settings::Settings {
            light: true,
            metrics: crate::settings::Metrics {
                text: 18,
                ..Default::default()
            },
            session: crate::session_preferences::SessionPreferences {
                clock_24h: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let portal = Appearance {
            preferences: Mutex::new(Preferences::from_settings(&settings, &Default::default())),
        };
        assert_eq!(
            u32::try_from(portal.read(APPEARANCE, "color-scheme").await.unwrap()).unwrap(),
            2
        );
        let interface = "org.gnome.desktop.interface";
        let mut values = portal
            .read_all(vec!["org.gnome.desktop.inter*".into()])
            .await;
        assert_eq!(
            values.len(),
            1,
            "ReadAll must respect the requested namespace"
        );
        let mut values = values.remove(interface).unwrap();
        assert_eq!(values.remove("gtk-enable-primary-paste"), Some(true.into()));
        assert_eq!(values.remove("cursor-size"), Some(24i32.into()));
        assert_eq!(
            String::try_from(values.remove("clock-format").unwrap()).unwrap(),
            "12h"
        );
        assert!(f64::try_from(values.remove("text-scaling-factor").unwrap()).unwrap() > 1.0);
        use enzui_theme::RawFontDescription;
        let ui =
            RawFontDescription::new(String::try_from(values.remove("font-name").unwrap()).unwrap());
        let mono = RawFontDescription::new(
            String::try_from(values.remove("monospace-font-name").unwrap()).unwrap(),
        );
        // The desktop's default UI family: a real installed one when present
        // (font_settings::default_sans), else the generic sans-serif.
        let expected = match crate::font_settings::default_sans() {
            "Sans" => enzui_theme::FontRole::Ui.generic_font_family(),
            name => enzui_theme::FontFamily::Named(name.to_owned()),
        };
        assert_eq!(ui.validate(), Some(expected));
        assert_eq!(
            mono.validate(),
            Some(enzui_theme::FontRole::Monospace.generic_font_family())
        );
        assert_eq!(
            ui.parse().unwrap().size,
            Some(enzui_theme::FontSize::Points(9.75))
        );
        for name in ["Demo Condensed", "Demo 400"] {
            let mut changed = settings.clone();
            changed.fonts.interface = Some(name.into());
            changed.fonts.weight = crate::font_settings::Weight::Heavy;
            *portal.preferences.lock().unwrap() =
                Preferences::from_settings(&changed, &Default::default());
            let description = RawFontDescription::new(
                String::try_from(portal.read(interface, "font-name").await.unwrap()).unwrap(),
            );
            let parsed = description.parse().unwrap();
            assert_eq!(
                parsed.family, name,
                "a family suffix must not become Pango style or size metadata"
            );
            assert_eq!(parsed.weight, Some(iced::font::Weight::Semibold));
        }

        assert!(
            portal
                .read("private.settings", "clock-format")
                .await
                .is_err()
        );
        assert!(portal.read(interface, "unknown").await.is_err());
    }
}
