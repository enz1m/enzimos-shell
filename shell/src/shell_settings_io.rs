use super::{Message, Request, Shell};
use enzui_shell::i18n::{self, tr};
use enzui_shell::settings::{Change as SettingChange, Settings};
use iced::Task;
use std::time::{Duration, Instant};

impl Shell {
    pub(crate) fn read_settings(&mut self) -> Task<Message> {
        let Some(store) = self.settings_store.clone() else {
            return Task::none();
        };
        self.settings_read_busy = true;
        self.settings_read_at = Some(Instant::now());
        let generation = self.settings_generation;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || store.load())
                    .await
                    .map_err(|_| tr("shell-settings-reader-stopped").to_owned())?
            },
            move |result| Message::SettingsLoaded(generation, result),
        )
    }

    pub(crate) fn save_setting(&mut self, change: SettingChange) -> Task<Message> {
        self.settings_generation = self.settings_generation.wrapping_add(1);
        if self.settings_save_busy {
            if self.settings_queue.len() < 32 {
                self.settings_queue.push_back(change);
            }
            return Task::none();
        }
        let Some(store) = self.settings_store.clone() else {
            self.notice = Some(tr("shell-no-shell-settings-directory").into());
            return Task::none();
        };
        self.settings_save_busy = true;
        let compositor = self.compositor.clone();
        let transition = matches!(
            &change,
            SettingChange::Light(_) | SettingChange::ThemeScheme(_)
        ) && !self.tokens().motion.reduced_motion;
        Task::perform(
            async move {
                if transition {
                    compositor.appearance_transition(false).await;
                }
                let result = tokio::task::spawn_blocking(move || store.update(change))
                    .await
                    .map_err(|_| tr("shell-settings-writer-stopped").to_owned())?;
                if transition {
                    compositor.appearance_transition(true).await;
                }
                result
            },
            Message::SettingsSaved,
        )
    }

    pub(crate) fn accept_settings(&mut self, result: Result<Settings, String>) -> Task<Message> {
        let settings = match result {
            Ok(settings) => settings,
            Err(error) => {
                self.notice = Some(error);
                // Unreadable settings leave the default style to show.
                self.chrome_styled = true;
                self.update_frame();
                return Task::none();
            }
        };
        let language_changed = i18n::select(settings.language);
        let changed = settings != self.desired_settings;
        let layout_changed = settings.layout != self.desired_settings.layout;
        // Desktop previews are drawn over the wallpaper.
        if settings.wallpaper != self.desired_settings.wallpaper {
            self.invalidate_previews();
        }
        let sidebar_enabled_changed =
            settings.sidebar_enabled != self.desired_settings.sidebar_enabled;
        // A new shell colour or glass reaches windows whose own state did not
        // change (the rail kept its old tint).
        let chrome_changed = settings.chrome != self.desired_settings.chrome
            || settings.panel_glass != self.desired_settings.panel_glass;
        if settings.controller_notifications != self.desired_settings.controller_notifications {
            self.controllers_generation = self.controllers_generation.wrapping_add(1);
            self.controllers = Default::default();
        }
        self.desired_settings = settings.clone();
        // A changed clock format shows now, not at the next minute.
        self.clock = super::shell_window_settings::clock(&self.desired_settings.session);
        self.activity_settings_changed();
        let activity = if !settings.activity_status {
            self.sync_activity_panel()
        } else {
            Task::none()
        };
        self.sync_night_light();
        if self.display_layouts.1.as_ref() != Some(&settings.displays) {
            let _ = self.display_layouts.0.send(settings.displays.clone());
            self.display_layouts.1 = Some(settings.displays.clone());
        }
        self.light = settings.light;
        // Re-read on the same poll that carries Settings' own changes, so a
        // saved accent reaches the bar without restarting the shell.
        self.theme_document =
            enzui_shell::theme_store::Store::read(&enzui_core::theme_file::ThemePaths::from_env())
                .into_document();
        let icons = if enzui_shell::icon_theme::select(settings.icon_theme.as_deref())
            || language_changed
        {
            self.dock.apps_busy = false;
            Task::batch([self.search_icons_changed(), self.dock_load_apps()])
        } else {
            Task::none()
        };
        self.update_frame();
        // The default mode and the desktops with their own go to the
        // compositor together whenever either changes.
        let modes = (
            settings.tiling.unwrap_or(false),
            settings.desktop_tiling.clone(),
        );
        let tiling = if changed && self.desktop_modes_sent.as_ref() != Some(&modes) {
            self.desktop_modes_sent = Some(modes.clone());
            self.enqueue(Request::DesktopModes(modes.0, modes.1))
        } else {
            Task::none()
        };
        // Hide the bar while applying frame/island changes to avoid presenting mixed geometry.
        let switching = self.bar_ready
            && self.metrics.islands.is_some() != settings.effective_metrics().islands.is_some();
        let bar = if self.bar_style_fade || self.bar_busy {
            Task::none()
        } else if switching {
            self.bar_style_fade = true;
            Task::batch([
                // The new style comes in once the old one has fully faded
                // (sooner, the new bar showed through its last few percent).
                Task::perform(
                    tokio::time::sleep(Duration::from_millis(
                        u64::from(self.chrome_fade_ms()) + 40,
                    )),
                    |()| Message::BarStyleHidden,
                ),
                self.start_frame_fade(true),
            ])
        } else if !self.bar_ready
            || self.bar != settings.bar
            || self.metrics != settings.effective_metrics()
        {
            self.apply_bar()
        } else {
            Task::none()
        };
        let maximize = self.drag_maximize();
        let sidebar = if sidebar_enabled_changed
            || self.sidebar_configured != Some(self.sidebar_configuration())
        {
            self.refresh_sidebar(None)
        } else {
            Task::none()
        };
        let regions = if layout_changed {
            self.cancel_panel_action();
            Task::batch([
                self.close_search(),
                self.force_close_notifications(),
                self.force_close_plugin(),
                self.force_close_overview(),
                self.force_close_controls(),
                self.close_launcher(),
            ])
        } else {
            Task::none()
        };
        let redraw = if chrome_changed {
            Task::batch([
                iced_runtime::task::effect(iced_runtime::Action::Window(
                    iced_runtime::window::Action::RedrawAll,
                )),
                self.refresh_bar_glass(),
            ])
        } else {
            Task::none()
        };
        Task::batch([
            redraw,
            icons,
            tiling,
            bar,
            maximize,
            sidebar,
            regions,
            activity,
            self.notification_preferences(),
            self.search_preferences(),
            self.sync_input_method(),
        ])
    }

    // Follows the night light schedule; checked on every tick, so a
    // scheduled change happens within a few seconds of its minute.
    pub(crate) fn sync_night_light(&mut self) {
        use enzui_shell::night_light::{NightLight, local_minutes};
        if !enzui_shell::desktop_session::mode().managed() {
            return;
        }
        let target = self.desired_settings.night_light.target_at(local_minutes());
        if target == self.night_light_target {
            return;
        }
        self.night_light_target = target;
        self.night_light
            .get_or_insert_with(|| {
                NightLight::start(|error| eprintln!("enzui-shell: night light: {error}"))
            })
            .set(target);
    }

    fn sync_input_method(&mut self) -> Task<Message> {
        use enzui_shell::input_method;
        if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed
            || !input_method::available()
        {
            return Task::none();
        }
        let on = input_method::wanted(self.desired_settings.input_method);
        if self.input_method == Some(on) {
            return Task::none();
        }
        self.input_method = Some(on);
        Task::perform(input_method::apply(on), Message::InputMethodApplied)
    }

    pub(crate) fn sync_desktop_preferences(&mut self) -> Task<Message> {
        if self.preferences_busy
            || self.desktop_applied.as_ref() == Some(&self.desired_settings.compositor)
            || self
                .desktop_retry_at
                .is_some_and(|time| Instant::now() < time)
        {
            return Task::none();
        }
        let wanted = self.desired_settings.compositor.clone();
        let input = wanted.clone();
        let compositor = self.compositor.clone();
        let store = self.settings_store.clone();
        self.preferences_busy = true;
        Task::perform(
            async move {
                let result = compositor.apply_desktop_preferences(&input).await;
                if let Some(store) = store {
                    let islands = compositor.islands_supported().await;
                    let colors = compositor.color_support().await.unwrap_or_default();
                    let status = serde_json::json!({"preferences": input, "error": result.as_ref().err(), "outputs": colors});
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut status = status;
                        status["motion_version"] = serde_json::json!(1);
                        // Settings offers Islands only when the running desktop's scene
                        // can publish island input regions.
                        if islands {
                            status["shell_style_version"] = serde_json::json!(1);
                        }
                        status["owner"] =
                            serde_json::to_value(enzui_shell::motion::Owner::current())?;
                        let path = store.path().with_file_name("compositor-status.json");
                        let mut file = tempfile::NamedTempFile::new_in(
                            path.parent().ok_or(std::io::ErrorKind::NotFound)?,
                        )?;
                        serde_json::to_writer(&mut file, &status)?;
                        file.persist(path).map_err(|error| error.error)?;
                        Ok::<_, std::io::Error>(())
                    })
                    .await;
                }
                result
            },
            move |result| Message::DesktopPreferencesApplied(wanted, result),
        )
    }
    pub(crate) fn apply_bar(&mut self) -> Task<Message> {
        self.cancel_panel_action();
        self.bar_busy = true;
        self.bar_output_stale = false;
        self.bar_epoch = self.bar_epoch.wrapping_add(1);
        let epoch = self.bar_epoch;
        let edge = self.desired_settings.bar;
        let metrics = self.desired_settings.effective_metrics();
        let compositor = self.compositor.clone();
        Task::batch([
            self.discard_search_surface(),
            self.force_close_plugin(),
            self.force_close_overview(),
            self.force_close_controls(),
            self.force_close_notifications(),
            self.force_close_launcher(),
            Task::perform(
                async move { compositor.apply_bar_position(edge, metrics, epoch).await },
                move |result| Message::BarApplied(edge, metrics, result),
            ),
        ])
    }
}

pub(crate) fn watch(
    store: Option<&enzui_shell::settings::Store>,
) -> Option<enzui_core::file_tree_watch::DirectoryWatchService> {
    use enzui_core::file_tree_watch::{DirectoryWatchService, linux::LinuxInotifyBackend};
    let settings = store?.path().parent()?.to_path_buf();
    let theme = enzui_core::theme_file::ThemePaths::from_env()
        .candidates()
        .into_iter()
        .next()?
        .parent()?
        .to_path_buf();
    let service = DirectoryWatchService::new(LinuxInotifyBackend::new().ok()?);
    service.watch(settings).ok()?;
    watch_plugins(Some(&service), store);
    if theme.is_dir() {
        service.watch(theme).ok()?;
    }
    Some(service)
}

pub(crate) fn watch_plugins(
    service: Option<&enzui_core::file_tree_watch::DirectoryWatchService>,
    store: Option<&enzui_shell::settings::Store>,
) {
    let Some((service, directory)) = service.zip(store.and_then(|store| store.path().parent()))
    else {
        return;
    };
    let plugins = directory.join("plugins");
    if !service.watched_paths().contains(&plugins) && plugins.is_dir() {
        let _ = service.watch(plugins);
    }
}
