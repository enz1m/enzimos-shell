use enzui_shell::i18n::tr;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::{
    BarEdge, Change, Metric, Metrics, NotificationState, QuickAction, Settings, SidebarEdge,
    model::parse,
};

pub(super) const MAX_SETTINGS_BYTES: usize = 65536;

#[derive(Clone, Debug)]
pub struct Store {
    pub(super) path: PathBuf,
}

impl Store {
    pub fn at(path: PathBuf) -> Result<Self, String> {
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(tr("shell-settings-path-must-name-an-absolute-file").into());
        }
        Ok(Self { path })
    }

    pub fn from_environment() -> Option<Self> {
        std::env::var_os("ENZUI_SHELL_SETTINGS")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
                    .map(|p| p.join("enzui-shell/settings.json"))
            })
            .and_then(|path| Self::at(path).ok())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(super) fn document(&self) -> Result<serde_json::Value, String> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match File::open(self.path.with_file_name("appearance.json")) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(serde_json::json!({}));
                    }
                    Err(_) => {
                        return Err(tr("shell-cannot-read-previous-shell-appearance").into());
                    }
                }
            }
            Err(_) => return Err(tr("shell-cannot-read-shell-settings").into()),
        };
        let mut bytes = Vec::new();
        file.take((MAX_SETTINGS_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| tr("shell-cannot-read-shell-settings"))?;
        if bytes.len() > MAX_SETTINGS_BYTES {
            return Err(tr("shell-shell-settings-exceed-their-size-limit").into());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| tr("shell-shell-settings-are-invalid-json"))?;
        if !value.is_object() {
            return Err(tr("shell-shell-settings-must-be-an-object").into());
        }
        Ok(value)
    }

    pub fn load(&self) -> Result<Settings, String> {
        parse(&self.document()?)
    }

    pub(crate) fn import_fonts_if_absent(
        &self,
        fonts: &crate::font_settings::Fonts,
    ) -> Result<(), String> {
        self.mutate_document(|value| {
            if value.get("fonts").is_none() {
                value["fonts"] =
                    serde_json::to_value(fonts).map_err(|_| "Invalid desktop fonts")?;
            }
            Ok(())
        })
        .map(|_| ())
    }

    pub fn update(&self, change: Change) -> Result<Settings, String> {
        self.mutate_document(|value| {
            match change {
                Change::Font(ref change) => {
                    crate::font_settings::Fonts::update_json(value, change)?;
                }
                Change::Dock(change) => crate::dock::Preferences::update_json(value, &change)?,
                Change::Search(change) => {
                    crate::search::preferences::Preferences::update_json(value, &change)?;
                }
                Change::Notification(change) => {
                    let previous = crate::notifications::preferences::Preferences::from_json(
                        value.get("notifications"),
                    )?;
                    crate::notifications::preferences::Preferences::update_json(value, &change)?;
                    let next = crate::notifications::preferences::Preferences::from_json(
                        value.get("notifications"),
                    )?;
                    if change.erases_history()
                        || previous.policy.history_generation != next.policy.history_generation
                    {
                        crate::notifications::archive::Storage::for_settings(&self.path)?
                            .erase()?;
                    }
                }
                Change::Light(light) => value["light"] = light.into(),
                Change::Language(language) => value["language"] = language.key().into(),
                Change::ShellStyle(style) => value["shell_style"] = style.key().into(),
                Change::Island(key, setting) => {
                    crate::shell_style::Islands::update_json(value, key, setting)?;
                }
                Change::IslandsJoinedBar(joined) => {
                    crate::shell_style::Islands::update_joined_bar(value, joined)?;
                }
                Change::SidebarPinScope(scope) => {
                    value["sidebar_pin_scope"] = scope.key().into();
                }
                Change::SidebarEnabled(enabled) => value["sidebar_enabled"] = enabled.into(),
                Change::ActivityStatus(enabled) => value["activity_status"] = enabled.into(),
                Change::BarShadow(level) => value["bar_shadow"] = level.min(4).into(),
                Change::PanelShadow(level) => value["panel_shadow"] = level.min(4).into(),
                Change::Chrome(change) => {
                    crate::chrome_style::ChromeStyle::update_json(value, change)?;
                }
                Change::PanelGlass(style) => {
                    for change in [
                        crate::chrome_style::Change::GlassTint(None),
                        crate::chrome_style::Change::GlassBlur(None),
                        crate::chrome_style::Change::GlassBending(None),
                        crate::chrome_style::Change::GlassHighlight(None),
                    ] {
                        crate::chrome_style::ChromeStyle::update_json(value, change)?;
                    }
                    value["panel_glass"] = style
                        .min(crate::reveal::PANEL_MATERIALS.len() as u8 - 1)
                        .into();
                }
                Change::AssistantEnabled(enabled) => value["assistant_enabled"] = enabled.into(),
                Change::ControllerNotifications(enabled) => {
                    value["controller_notifications"] = enabled.into();
                }
                Change::AssistantCorner(corner) => value["assistant_corner"] = corner.key().into(),
                Change::AssistantWidth(width) => {
                    value["assistant_width"] =
                        super::model::clamp_assistant_width(width.into()).into();
                }
                Change::ThemeScheme(ref id) => {
                    if crate::theme_palette::Scheme::from_id_str(id).is_none() {
                        return Err(tr("shell-invalid-theme-style").into());
                    }
                    value["theme_scheme"] = id.clone().into();
                }
                Change::IconTheme(ref theme) => match theme {
                    Some(id) if super::model::icon_theme_id(id) => {
                        value["icon_theme"] = id.clone().into();
                    }
                    Some(_) => return Err(tr("shell-invalid-icon-theme").into()),
                    None => {
                        if let Some(object) = value.as_object_mut() {
                            object.remove("icon_theme");
                        }
                    }
                },
                Change::SidebarVisibility(mode) => {
                    let mut preferences = crate::sidebar_preferences::Preferences::from_json(
                        value.get("sidebar_behavior"),
                    )?;
                    preferences.visibility = mode;
                    preferences.write_json(value)?;
                }
                Change::FullscreenPadding(padding) => {
                    let mut preferences = crate::sidebar_preferences::Preferences::from_json(
                        value.get("sidebar_behavior"),
                    )?;
                    preferences.set_padding(padding)?;
                    preferences.write_json(value)?;
                }
                Change::ResetIslandSizing => {
                    crate::sidebar_preferences::Preferences::default().write_json(value)?;
                    for key in crate::shell_style::IslandOption::ALL {
                        crate::shell_style::Islands::update_json(
                            value,
                            key,
                            crate::shell_style::Islands::default().get(key),
                        )?;
                    }
                    crate::shell_style::Islands::update_joined_bar(value, false)?;
                }
                Change::ResetFrameSizing => {
                    for key in [Metric::Frame, Metric::Radius, Metric::Bar, Metric::Rail] {
                        value[key.key()] = Metrics::default().get(key).into();
                    }
                }
                Change::Compositor(key, setting) => {
                    crate::desktop_preferences::CompositorPreferences::update_json(
                        value, key, setting,
                    )?;
                }
                Change::Session(ref change) => {
                    crate::session_preferences::SessionPreferences::update_json(value, change)?;
                }
                Change::NightLight(change) => {
                    crate::night_light::Preferences::update_json(value, change)?;
                }
                Change::Capture(ref change) => {
                    crate::capture::Preferences::update_json(value, change.clone())?;
                }
                Change::Displays(ref layout) => {
                    let mut saved =
                        crate::system_settings::display::saved_from_json(value.get("displays"))?;
                    crate::system_settings::display::remember(&mut saved, layout)?;
                    value["displays"] = serde_json::to_value(saved)
                        .map_err(|_| tr("shell-cannot-encode-display-layouts"))?;
                }
                Change::Display(ref change) => {
                    crate::desktop_preferences::CompositorPreferences::update_display_json(
                        value, change,
                    )?;
                }
                Change::Terminal(ref program) => {
                    crate::desktop_preferences::CompositorPreferences::update_terminal_json(
                        value, program,
                    )?;
                }
                Change::Shortcut(ref id, ref chord) => {
                    let mut next =
                        crate::desktop_preferences::CompositorPreferences::shortcuts_from_account(
                            value,
                        )?;
                    next.set_shortcut(id, chord.as_deref())?;
                    next.write_shortcuts(value)?;
                }
                Change::CustomShortcut(ref id, ref shortcut) => {
                    let mut next =
                        crate::desktop_preferences::CompositorPreferences::shortcuts_from_account(
                            value,
                        )?;
                    next.set_custom_shortcut(id, shortcut.as_ref())?;
                    next.write_shortcuts(value)?;
                }
                Change::KeyboardLayouts(ref layouts) => {
                    crate::desktop_preferences::CompositorPreferences::update_layouts_json(
                        value, layouts,
                    )?;
                }
                Change::Wallpaper(change) => {
                    let mut wallpaper =
                        crate::desktop_preferences::Wallpaper::from_json(value.get("wallpaper"))?;
                    wallpaper.apply(change)?;
                    let update = serde_json::to_value(wallpaper)
                        .map_err(|_| tr("shell-cannot-encode-wallpaper-preferences"))?;
                    if value.get("wallpaper").is_none() {
                        value["wallpaper"] = serde_json::json!({});
                    }
                    for (key, setting) in update
                        .as_object()
                        .ok_or_else(|| tr("shell-invalid-wallpaper-preferences"))?
                    {
                        value["wallpaper"][key] = setting.clone();
                    }
                }
                Change::Bar(edge) => {
                    value["bar"] = match edge {
                        BarEdge::Top => "top",
                        BarEdge::Bottom => "bottom",
                    }
                    .into();
                }
                Change::Sidebar(edge) => {
                    value["sidebar"] = match edge {
                        SidebarEdge::Left => "left",
                        SidebarEdge::Right => "right",
                    }
                    .into();
                }
                Change::Tiling(tiling) => value["tiling"] = tiling.into(),
                Change::DesktopTilingReset => {
                    if let Some(object) = value.as_object_mut() {
                        object.remove("desktop_tiling");
                    }
                }
                Change::DesktopTiling(desktop, mode) => {
                    let mut modes =
                        super::model::desktop_tiling_from_json(value.get("desktop_tiling"))
                            .unwrap_or_default();
                    super::model::set_desktop_tiling(&mut modes, desktop, mode)?;
                    value["desktop_tiling"] = modes
                        .into_iter()
                        .map(|(desktop, mode)| (desktop.to_string(), serde_json::Value::Bool(mode)))
                        .collect::<serde_json::Map<_, _>>()
                        .into();
                }
                Change::InputMethod(on) => value["input_method"] = on.into(),
                Change::OskButton(on) => value["osk_button"] = on.into(),
                Change::AccessibilityMenu(on) => value["accessibility_menu"] = on.into(),
                Change::RestoreApps(on) => value["restore_apps"] = on.into(),
                Change::DisplayMirror(on) => value["display_mirror"] = on.into(),
                Change::ScreenTime(on) => value["screen_time"] = on.into(),
                Change::WorldClockAdd(ref zone) => {
                    let mut clocks =
                        super::model::world_clocks_from_json(value.get("world_clocks"));
                    super::model::add_world_clock(&mut clocks, zone)?;
                    value["world_clocks"] = clocks.into();
                }
                Change::WorldClockRemove(ref zone) => {
                    let mut clocks =
                        super::model::world_clocks_from_json(value.get("world_clocks"));
                    clocks.retain(|known| known != zone);
                    value["world_clocks"] = clocks.into();
                }
                Change::Metric(metric, pixels) => value[metric.key()] = pixels.into(),
                Change::QuickAction(action, shown) => {
                    let mut settings = parse(value)?;
                    if shown {
                        settings.quick_actions |= action.bit();
                    } else {
                        settings.quick_actions &= !action.bit();
                    }
                    value["quick_actions"] = serde_json::json!(
                        QuickAction::ALL
                            .into_iter()
                            .filter(|a| settings.shows(*a))
                            .map(QuickAction::key)
                            .collect::<Vec<_>>()
                    );
                }
                Change::ControlsLayout {
                    expected,
                    value: layout,
                } => {
                    let current = parse(value)?.controls;
                    if current != expected && current != layout {
                        return Err(
                            tr("shell-control-center-changed-elsewhere-reload-and-retry").into(),
                        );
                    }
                    layout.write_json(&mut value["control_center_layout"])?;
                }
                Change::Layout {
                    expected,
                    value: layout,
                } => {
                    let current = parse(value)?.layout;
                    if current != expected && current != layout {
                        return Err(
                            tr("shell-bar-layout-changed-elsewhere-reload-and-retry").into()
                        );
                    }
                    value["bar_layout"] = layout.to_json();
                }
                Change::ResetSizing => {
                    for metric in Metric::ALL {
                        value[metric.key()] = Metrics::default().get(metric).into();
                    }
                }
            }
            Ok(())
        })
    }

    pub fn read_notification_state(&self, now_ms: u64) -> Result<NotificationState, String> {
        let mut state = None;
        self.mutate_document(|value| {
            let settings = parse(value)?;
            let history = crate::notifications::archive::Storage::for_settings(&self.path)?
                .load(&settings.notifications.policy, now_ms);
            state = Some(NotificationState { settings, history });
            Ok(())
        })?;
        state.ok_or_else(|| tr("shell-notification-history-reader-did-not-run").into())
    }
    pub fn read_notification_history(
        &self,
        now_ms: u64,
    ) -> Result<Vec<crate::notifications::model::Archived>, String> {
        self.read_notification_state(now_ms)?.history
    }
    pub fn save_notification_history(
        &self,
        snapshot: &crate::notifications::server::Snapshot,
        now_ms: u64,
    ) -> Result<bool, String> {
        self.save_notification_history_for(
            snapshot,
            now_ms,
            &std::sync::atomic::AtomicBool::new(true),
        )
    }
    pub fn observe_notification_applications(
        &self,
        snapshot: &crate::notifications::server::Snapshot,
        authority: &std::sync::atomic::AtomicBool,
    ) -> Result<Settings, String> {
        self.mutate_document(|value| {
            if !authority.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            for notification in snapshot
                .active
                .iter()
                .take(crate::notifications::model::MAX_ACTIVE)
            {
                use crate::notifications::preferences::{Application, Change, Preferences};
                Preferences::update_json(
                    value,
                    &Change::Observe {
                        principal: notification.origin.principal.clone(),
                        application: Application {
                            label: notification.origin.label.clone(),
                            icon: notification.origin.icon.clone(),
                        },
                    },
                )?;
            }
            Ok(())
        })
    }
    pub fn save_notification_history_for(
        &self,
        snapshot: &crate::notifications::server::Snapshot,
        now_ms: u64,
        authority: &std::sync::atomic::AtomicBool,
    ) -> Result<bool, String> {
        let mut saved = false;
        self.mutate_document(|value| {
            if !authority.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            let preferences = crate::notifications::preferences::Preferences::from_json(
                value.get("notifications"),
            )?;
            let storage = crate::notifications::archive::Storage::for_settings(&self.path)?;
            if !preferences.policy.history {
                storage.erase()?;
                return Ok(());
            }
            if !snapshot.policy.history
                || preferences.policy.history_generation != snapshot.policy.history_generation
            {
                return Ok(());
            }
            storage.save(&preferences.policy, snapshot, now_ms)?;
            saved = true;
            Ok(())
        })?;
        Ok(saved)
    }
    pub(crate) fn transform_layout(
        &self,
        transform: impl FnOnce(
            &crate::bar_layout::BarLayout,
        ) -> Result<crate::bar_layout::BarLayout, String>,
    ) -> Result<Settings, String> {
        self.mutate_document(|value| {
            let current = parse(value)?.layout;
            let layout = transform(&current)?;
            if layout != *current {
                value["bar_layout"] = layout.to_json();
            }
            Ok(())
        })
    }

    fn mutate_document(
        &self,
        apply: impl FnOnce(&mut serde_json::Value) -> Result<(), String>,
    ) -> Result<Settings, String> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| tr("shell-invalid-shell-settings-path"))?;
        std::fs::create_dir_all(directory)
            .map_err(|_| tr("shell-cannot-create-shell-settings-directory"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.path.with_extension("lock"))
            .map_err(|_| tr("shell-cannot-open-shell-settings-lock"))?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    return Err(tr("shell-shell-settings-are-busy-or-cannot-be-locked").into());
                }
            }
        }
        let mut value = self.document()?;
        parse(&value)?;
        let original = value.clone();
        apply(&mut value)?;
        let settings = parse(&value)?;
        if original == value {
            return Ok(settings);
        }
        let bytes =
            serde_json::to_vec(&value).map_err(|_| tr("shell-cannot-encode-shell-settings"))?;
        if bytes.len() > MAX_SETTINGS_BYTES {
            return Err(tr("shell-shell-settings-exceed-their-size-limit").into());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(directory)
            .map_err(|_| tr("shell-cannot-create-temporary-shell-settings"))?;
        temporary
            .write_all(&bytes)
            .map_err(|_| tr("shell-cannot-save-shell-settings"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| tr("shell-cannot-sync-shell-settings"))?;
        temporary
            .persist(&self.path)
            .map_err(|_| tr("shell-cannot-replace-shell-settings"))?;
        enzui_security::durable::sync_parent_dir(&self.path)
            .map_err(|_| tr("shell-cannot-sync-shell-settings"))?;
        Ok(settings)
    }
}
