//! Desktop preferences published through the Settings portal.
use super::{PATH, owned};
use enzui_shell::i18n::tr;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use zbus::zvariant::{OwnedValue, Value};

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
        Self::from_settings(&settings)
    }

    fn from_settings(settings: &crate::settings::Settings) -> Self {
        Self {
            light: settings.light,
            text_scale: f64::from(settings.effective_metrics().text_scale()),
            clock_24h: settings.session.clock_24h,
            fonts: settings.fonts.clone(),
        }
    }

    fn values(&self) -> BTreeMap<String, BTreeMap<String, OwnedValue>> {
        BTreeMap::from([
            (
                APPEARANCE.to_owned(),
                BTreeMap::from([(
                    "color-scheme".to_owned(),
                    OwnedValue::from(color_scheme(self.light)),
                )]),
            ),
            (
                "org.gnome.desktop.interface".to_owned(),
                BTreeMap::from([
                    (
                        "font-name".to_owned(),
                        owned(Value::from(
                            self.fonts
                                .description(crate::font_settings::Slot::Interface),
                        )),
                    ),
                    (
                        "document-font-name".to_owned(),
                        owned(Value::from(
                            self.fonts.description(crate::font_settings::Slot::Document),
                        )),
                    ),
                    (
                        "monospace-font-name".to_owned(),
                        owned(Value::from(
                            self.fonts
                                .description(crate::font_settings::Slot::Monospace),
                        )),
                    ),
                    (
                        "text-scaling-factor".to_owned(),
                        OwnedValue::from(self.text_scale),
                    ),
                    (
                        "clock-format".to_owned(),
                        owned(Value::from(if self.clock_24h { "24h" } else { "12h" })),
                    ),
                ]),
            ),
        ])
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
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}

struct PreferenceChanges(Arc<tokio::sync::Notify>);
impl enzui_core::file_tree_watch::EventSink for PreferenceChanges {
    fn push(&self, _: enzui_core::file_tree_watch::RawEvent) {
        self.0.notify_one();
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
    let path = crate::settings::Store::from_environment().map(|store| store.path().to_owned());
    let stamp = |path: &Option<std::path::PathBuf>| {
        path.as_ref().and_then(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
    };
    // Watch the directory: settings are atomically replaced, so watching the
    // old file inode would miss later appearance changes. Polling delayed
    // every framework app independently of the shell's theme transition.
    let changed = Arc::new(tokio::sync::Notify::new());
    let watch = path.as_ref().and_then(|path| {
        let backend = enzui_core::file_tree_watch::native_directory_watch()?;
        backend
            .watch(
                path.parent()?,
                enzui_core::file_tree_watch::WatchToken::standalone(),
                Arc::new(PreferenceChanges(Arc::clone(&changed))),
            )
            .ok()
    });
    let fallback = std::time::Duration::from_millis(if watch.is_some() { 60_000 } else { 750 });
    let mut seen = None;
    changed.notify_one();
    loop {
        tokio::select! {
            () = changed.notified() => {},
            () = tokio::time::sleep(fallback) => {},
        }
        tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        let now = stamp(&path);
        if now == seen {
            continue;
        }
        seen = now;
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
        // The backend's own windows follow the same changes as portal clients.
        let _ = events.send(super::Event::Appearance {
            light: preferences.light,
        });
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

    // This portal is a shared app boundary: preserve namespace filtering and
    // the wire types consumed by toolkits, alongside the desktop's choices.
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
            preferences: Mutex::new(Preferences::from_settings(&settings)),
        };
        assert_eq!(
            u32::try_from(portal.read(APPEARANCE, "color-scheme").await.unwrap()).unwrap(),
            2
        );
        let interface = "org.gnome.desktop.interface";
        let mut values = portal.read_all(vec!["org.gnome.*".into()]).await;
        assert_eq!(
            values.len(),
            1,
            "ReadAll must respect the requested namespace"
        );
        let mut values = values.remove(interface).unwrap();
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
            *portal.preferences.lock().unwrap() = Preferences::from_settings(&changed);
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
