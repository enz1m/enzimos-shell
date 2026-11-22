use super::*;
use enzui_shell::i18n::tr;

impl SettingsApp {
    /// Shows the part of a page a route lives in when the page builds it only
    /// for the current choice, such as the wallpaper kind.
    fn show_route_section(&mut self, route: &str) {
        use enzui_shell::desktop_preferences::WallpaperKind;
        if route == "wallpaper/color" {
            self.wallpaper.reveal_color_editor();
        }
        let shown = self
            .wallpaper
            .view_kind
            .unwrap_or(self.settings.wallpaper.kind);
        if matches!(
            route,
            "wallpaper/live"
                | "wallpaper/motion"
                | "wallpaper/pause_idle"
                | "wallpaper/hide_clock"
                | "wallpaper/hide_date"
        ) {
            self.wallpaper.view_kind = Some(WallpaperKind::Live);
        } else if route == "wallpaper/fit"
            && matches!(shown, WallpaperKind::Color | WallpaperKind::Live)
        {
            self.wallpaper.view_kind = Some(WallpaperKind::Image);
        }
    }
    // Suppress redraws when periodic reads leave both stored and visible state unchanged.
    pub(super) fn update(&mut self, message: Message) -> Task<Message> {
        let poll = match &message {
            // A display change's countdown is the only thing the tick moves on screen.
            Message::Tick if !self.display_confirm_pending() => Some(("tick", String::new())),
            Message::FilesChanged => Some(("tick", String::new())),
            Message::Loaded(_, result) => Some(("settings", format!("{result:?}"))),
            Message::DesktopStatus(status) => Some(("desktop", format!("{status:?}"))),
            Message::Wallpaper(wallpaper_settings::Message::Status(status)) => {
                Some(("wallpaper", format!("{status:?}")))
            }
            Message::Plugin(plugins_settings::Message::Read(result)) => Some((
                "plugins",
                result
                    .as_ref()
                    .map_or_else(Clone::clone, plugins_settings::Data::fingerprint),
            )),
            Message::Controls(control_editor::Message::Snapshot(system)) => {
                Some(("controls", format!("{system:?}")))
            }
            Message::System(system_pages::Message::Loaded(reading)) => {
                Some(("system", format!("{reading:?}")))
            }
            _ => None,
        };
        let task = self.update_message(message);
        let Some((kind, answer)) = poll else {
            return task;
        };
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            answer,
            &self.error,
            &self.wallpaper.status,
            &self.desktop_status,
            self.page,
        )
            .hash(&mut hasher);
        let shown = hasher.finish();
        if self.polls.insert(kind, shown) == Some(shown) || kind == "tick" {
            Task::batch([task, window::invalidate(window::Invalidation::none())])
        } else {
            task
        }
    }

    fn update_message(&mut self, message: Message) -> Task<Message> {
        if matches!(
            &message,
            Message::Change(_)
                | Message::Theme(_)
                | Message::ThemePrepared(_, _)
                | Message::AppStyle(_)
                | Message::System(
                    system_pages::Message::Request(_)
                        | system_pages::Message::SetDisplay(_)
                        | system_pages::Message::SelectDisplay(_)
                )
        ) {
            self.menu = None;
        }
        let saved = match &message {
            Message::Saved(result) => Some(result.as_ref().err().cloned()),
            _ => None,
        };
        let applied = match &message {
            Message::System(system_pages::Message::Applied(result)) => self.agent_applied(result),
            _ => Task::none(),
        };
        let task = match message {
            Message::FileChooser(message) => self.file_chooser_message(message),
            Message::Startup(message) => self.startup_message(message),
            Message::Center(event) => self.center_event(event),
            Message::ApplicationWake => self.application_wake(),
            Message::BackgroundTaskRefused(error) => {
                eprintln!("enzui-settings: windowless task refused: {error}");
                Task::none()
            }
            Message::AgentClock(reading) => self.agent_clock_read(*reading),
            Message::AgentApproved(id, result, authority) => {
                self.agent_approved(id, result, authority)
            }
            Message::AgentCommitted(id, result) => self.agent_committed(id, result),
            Message::AgentDeadline => self.agent_deadline(),
            Message::Controls(message) => self.controls_message(message),
            Message::Numeric(message) => self.numeric_message(message),
            Message::System(message) => self.system_message(message),
            Message::Users(message) => self.users_message(message),
            Message::Background(message) => self.background_message(message),
            Message::OpenDevTools => self.open_dev_tools(),
            Message::DevToolsToken(epoch, window, token) => {
                self.dev_tools_token(epoch, window, token)
            }
            Message::Opened(id) => {
                if self.window.is_some_and(|current| current != id) {
                    return Task::none();
                }
                if self.window != Some(id) {
                    self.dev_tools_launch.cancel(false);
                }
                self.window = Some(id);
                self.chrome.bind(id);
                Task::batch([
                    self.chrome
                        .update(enzui_widgets::app_shell::ShellWindowEvent::Observed(id))
                        .map(Message::WindowState),
                    window::size(id).map(move |size| Message::Viewport(id, size)),
                ])
            }
            Message::Viewport(id, size) => {
                if self.window == Some(id)
                    && size.width.is_finite()
                    && size.height.is_finite()
                    && size.width > 0.0
                    && size.height > 0.0
                {
                    self.viewport = size;
                }
                Task::none()
            }
            Message::Category(category, expanded) if !self.closing => {
                if expanded {
                    self.update(Message::Page(category))
                } else {
                    self.navigation.expanded = None;
                    Task::none()
                }
            }
            Message::Category(_, _) => Task::none(),
            Message::Collapse => {
                self.navigation.expanded = None;
                Task::none()
            }
            Message::Section(route) => {
                self.sandbox_leave();
                self.developer_leave();
                self.remote_leave();
                self.phone_leave();
                self.dev_tools_launch.cancel(false);
                self.section(route)
            }
            Message::PageScrolled if !self.closing => self.observe_scrolled_section(),
            Message::PageScrolled => Task::none(),
            Message::SectionObserved(revision, route) => {
                if revision == self.navigation.spy_revision
                    && self.filter.trim().is_empty()
                    && !self.navigation.moving()
                    && let Some((route, offset)) = route.filter(|(route, _)| {
                        self.sections(self.page)
                            .iter()
                            .any(|section| section.route == *route)
                    })
                    && let Some(route) = self.navigation.observed(route, offset)
                {
                    self.navigation.selected = Some(route);
                }
                Task::none()
            }
            Message::SectionLocated(epoch, position) => {
                let ms = self.tokens().motion.ms(enzui_theme::MotionToken::Normal) * 1.5;
                self.navigation.located(
                    epoch,
                    position,
                    Duration::from_secs_f32((ms / 1000.0).clamp(0.0, 2.0)),
                );
                Task::none()
            }
            Message::InterruptScroll(reason) => {
                let relocate = matches!(reason, navigation::Interruption::Resize)
                    && self.highlighted.is_some()
                    && self.navigation.moving();
                self.navigation.interrupt();
                if matches!(reason, navigation::Interruption::User) {
                    self.reveal_pending = false;
                } else if relocate {
                    self.reveal_pending = true;
                }
                Task::none()
            }
            Message::FindSetting => return iced::widget::operation::focus(SEARCH_FIELD),
            Message::Filter(value) => {
                self.users_leave();
                self.sandbox_leave();
                self.developer_leave();
                self.remote_leave();
                self.phone_leave();
                self.backup_setup_leave();
                self.dev_tools_launch.cancel(false);
                self.navigation.reset_scroll();
                self.navigation.cancel();
                self.navigation.selected = None;
                self.controls.navigation();
                self.numeric.cancel();
                self.nav.dismiss_overlay();
                self.picker = None;
                if !self.wallpaper.busy {
                    self.image_purpose = super::ImagePurpose::Desktop;
                }
                self.filter = value.chars().take(128).collect();
                self.highlighted = None;
                self.reveal_pending = true;
                self.menu = None;
                Task::none()
            }
            Message::Menu(key, open) => {
                // The trigger opens and, pressed again, closes its menu.
                if open && self.menu.as_ref() == Some(&key) {
                    self.menu = None;
                } else if open {
                    self.menu = Some(key);
                } else if self.menu.as_ref() == Some(&key) {
                    self.menu = None;
                }
                Task::none()
            }
            Message::Reveal(route) => {
                self.dev_tools_launch.cancel(false);
                if let Some(entry) = enzui_shell::settings_catalog::find(route)
                    .filter(|entry| !self.route_refused(entry.route))
                {
                    self.sandbox_leave();
                    self.developer_leave();
                    self.remote_leave();
                    self.phone_leave();
                    self.backup_setup_leave();
                    self.navigation.reset_scroll();
                    self.navigation.cancel();
                    self.navigation.selected = None;
                    self.controls.navigation();
                    self.numeric.cancel();
                    self.nav.dismiss_overlay();
                    self.users_leave();
                    self.file_chooser_leave();
                    self.startup.close();
                    self.page = entry.category();
                    self.navigation.expanded = Some(self.page);
                    self.highlighted = Some(entry.route);
                    self.show_route_section(entry.route);
                    self.reveal_pending = true;
                    self.filter.clear();
                    self.menu = None;
                }
                Task::batch([self.read_controls(), self.read_system(true)])
            }
            Message::RevealFrame => {
                let selected = self.navigation.selected;
                let navigation = self.navigation_frame();
                let focus = if let Some(id) = self.window {
                    self.numeric
                        .focus_pending
                        .take()
                        .map_or_else(Task::none, |session| {
                            let Some(editor) = self
                                .numeric
                                .editor
                                .as_ref()
                                .filter(|e| e.session == session)
                            else {
                                return Task::none();
                            };
                            let name = editor.spec.input_id();
                            Task::batch([
                                iced::widget::operation::focus(name.clone()),
                                iced_runtime::task::widget_in(
                                    id,
                                    enzui_core::focus::reveal_id(name.into()),
                                ),
                            ])
                        })
                } else {
                    Task::none()
                };
                let numeric_reveal = if let Some(id) = self.window {
                    self.numeric
                        .reveal_pending
                        .take()
                        .map_or_else(Task::none, |key| {
                            iced_runtime::task::widget_in(
                                id,
                                enzui_core::focus::reveal_id(format!("setting-{key}").into()),
                            )
                        })
                } else {
                    Task::none()
                };
                let reveal = if !self.reveal_pending || !self.reveal_ready() {
                    Task::none()
                } else if self.window.is_some() {
                    self.reveal_pending = false;
                    if let Some(route) = self.highlighted {
                        self.reveal_route(route)
                    } else {
                        iced::widget::operation::snap_to(
                            "settings-scroll",
                            iced::widget::scrollable::RelativeOffset::START,
                            iced::widget::operation::Animation::Instant,
                        )
                    }
                } else {
                    Task::none()
                };
                let control_focus =
                    self.controls
                        .focus_pending
                        .take()
                        .map_or_else(Task::none, |name| {
                            let reveal = self.window.map_or_else(Task::none, |id| {
                                iced_runtime::task::widget_in(
                                    id,
                                    enzui_core::focus::reveal_id(name.clone().into()),
                                )
                            });
                            Task::batch([iced::widget::operation::focus(name), reveal])
                        });
                let view = if self.navigation.selected == selected {
                    window::invalidate(window::Invalidation::none())
                } else {
                    Task::none()
                };
                Task::batch([
                    navigation,
                    focus,
                    numeric_reveal,
                    reveal,
                    control_focus,
                    view,
                ])
            }
            Message::SearchSettings(message) => self.search_settings_message(message),
            Message::Wallpaper(message) => self.wallpaper_message(message),
            Message::WallpaperFrame => {
                self.wallpaper.settle_preview();
                Task::none()
            }
            Message::WallpaperPicker(message) => self.wallpaper_picker_message(message),
            Message::PreviewNotification
                if self.layout_preview && !self.closing && !self.preview_notification_busy =>
            {
                if self.settings.notifications.policy.dnd {
                    self.error = Some(
                        tr("shell-turn-off-do-not-disturb-to-see-a-preview-notification").into(),
                    );
                    return Task::none();
                }
                self.preview_notification_busy = true;
                Task::perform(
                    notifications_settings::send_preview(),
                    Message::PreviewNotificationSent,
                )
            }
            Message::PreviewNotificationSent(result) => {
                self.preview_notification_busy = false;
                if let Err(error) = result {
                    self.error = Some(error);
                }
                Task::none()
            }
            Message::PreviewNotification => Task::none(),
            Message::DesktopStatus(status) => {
                self.desktop_status_busy = false;
                self.motion_supported = status.available;
                self.display_colors = status
                    .value
                    .as_ref()
                    .filter(|_| status.available)
                    .and_then(|value| serde_json::from_value(value["outputs"].clone()).ok())
                    .unwrap_or_default();
                self.islands_supported = status.available
                    && status
                        .value
                        .as_ref()
                        .is_some_and(|value| value["shell_style_version"] == 1);
                self.desktop_status = status.value.and_then(|value| {
                    if enzui_shell::desktop_preferences::CompositorPreferences::from_json(Some(
                        value.get("preferences")?,
                    ))
                    .ok()?
                        != self.settings.compositor
                    {
                        return None;
                    }
                    Some(value["error"].as_str()?.chars().take(512).collect())
                });
                Task::none()
            }
            Message::Plugin(message) => self.plugin_settings_message(message),
            Message::Shortcuts(event) => self.shortcuts_message(event),
            Message::Sandbox(message) => self.sandbox_message(message),
            Message::Assistant(message) => self.assistant_message(message),
            Message::LocalAi(message) => self.local_ai_message(message),
            Message::Noop => Task::none(),
            Message::DismissError(error) => {
                if self.error.as_ref() == Some(&error) {
                    self.error = None;
                }
                Task::none()
            }
            Message::Page(Page::Notifications) if !self.layout_preview => Task::none(),
            // A launch request for a page this computer cannot show.
            Message::Page(page) if self.route_refused(page.page()) => Task::none(),
            Message::Page(page) => {
                self.file_chooser_leave();
                self.startup.close();
                self.sandbox_leave();
                self.developer_leave();
                self.remote_leave();
                self.phone_leave();
                self.dev_tools_launch.cancel(false);
                if page != Page::Backups {
                    self.backup_setup_leave();
                }
                self.navigation.reset_scroll();
                self.navigation.cancel();
                self.navigation.selected = None;
                self.controls.navigation();
                self.numeric.cancel();
                self.nav.dismiss_overlay();
                self.picker = None;
                if !self.wallpaper.busy {
                    self.image_purpose = super::ImagePurpose::Desktop;
                }
                self.system.layout_query = None;
                self.users_leave();
                self.page = page;
                self.navigation.expanded = Some(page);
                self.filter.clear();
                self.navigation.selected = self.sections(page).first().map(|section| section.route);
                self.menu = None;
                self.highlighted = None;
                self.reveal_pending = false;
                self.controls.focus_pending = None;
                Task::batch([
                    self.read_controls(),
                    self.read_wallpaper_gallery(),
                    self.warm_wallpaper_preview(),
                    self.read_system(true),
                    self.read_users(),
                ])
            }
            Message::Sidebar(nav) => {
                self.nav = nav;
                Task::none()
            }
            Message::Layout(enzui_shell::settings_layout::Action::Reset)
                if !self.closing && self.layout_preview && !self.busy && !self.plugins.busy() =>
            {
                self.busy = true;
                self.resetting_layout = true;
                self.generation = self.generation.wrapping_add(1);
                self.error = None;
                let settings = self.store.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            enzui_shell::plugins::store::Store::for_settings(settings)?
                                .reconcile_layout(true)
                        })
                        .await
                        .map_err(|_| tr("shell-layout-reset-writer-stopped").to_owned())?
                    },
                    Message::Saved,
                )
            }
            Message::Layout(enzui_shell::settings_layout::Action::Reset) => Task::none(),
            Message::Layout(action)
                if !self.closing && self.layout_preview && !self.resetting_layout =>
            {
                let mut layout = (*self.settings.layout).clone();
                match action.apply(&mut layout) {
                    Ok(()) => self.update(Message::Change(Change::Layout {
                        expected: std::sync::Arc::clone(&self.settings.layout),
                        value: std::sync::Arc::new(layout),
                    })),
                    Err(error) => {
                        self.error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::Layout(_) => Task::none(),
            Message::Change(Change::Layout { .. }) if self.resetting_layout => Task::none(),
            Message::Change(Change::ShellStyle(enzui_shell::shell_style::ShellStyle::Islands))
                if !self.islands_supported =>
            {
                Task::none()
            }
            Message::Change(Change::Compositor(
                enzui_shell::desktop_preferences::OptionKey::AnimationRate,
                _,
            )) if !self.motion_supported && !self.closing => {
                self.error = Some(tr("shell-animation-speed-requires-a-current-managed-desktop-this-session-will-not-be").into());
                Task::none()
            }
            Message::Change(change) if !self.closing => {
                if matches!(change, Change::Font(_)) {
                    self.font_query = None;
                }
                // The compositor rounds windows and every application draws
                // its own corners from the theme: one radius for both.
                if let Change::Compositor(
                    enzui_shell::desktop_preferences::OptionKey::Rounding,
                    radius,
                ) = change
                    && let Ok(radius) = u16::try_from(radius)
                {
                    if self.themes.is_some() {
                        self.theme = enzui_shell::theme_store::Store::read(&theme_paths());
                    }
                    if let Err(error) = theme_settings::update(
                        &mut self.theme,
                        theme_settings::Action::WindowRadius(radius),
                    ) {
                        self.error = Some(error);
                        return Task::none();
                    }
                }
                if matches!(change, Change::ShellStyle(_)) {
                    self.numeric.cancel();
                    self.highlighted = None;
                    self.reveal_pending = false;
                    self.navigation.cancel();
                }
                self.generation = self.generation.wrapping_add(1);
                if let Err(error) = self.settings.preview(change.clone()) {
                    self.wallpaper.native_drop = None;
                    self.wallpaper.native_saved_path = None;
                    self.error = Some(error);
                    Task::none()
                } else {
                    if self
                        .wallpaper
                        .native_saved_path
                        .as_deref()
                        .is_some_and(|path| {
                            self.settings.wallpaper.kind
                                != enzui_shell::desktop_preferences::WallpaperKind::Image
                                || self.settings.wallpaper.image.as_deref() != Some(path)
                        })
                    {
                        self.wallpaper.native_drop = None;
                        self.wallpaper.native_saved_path = None;
                    }
                    self.publish_text_size();
                    self.error = None;
                    if self.busy {
                        let change =
                            if let Some(index) =
                                self.pending.iter().position(|old| old.same_key(&change))
                            {
                                change.coalesce(&self.pending.remove(index).expect(
                                    "the pending index was found immediately before removal",
                                ))
                            } else {
                                change
                            };
                        self.pending.push_back(change);
                        Task::none()
                    } else {
                        self.write(change)
                    }
                }
            }
            Message::Change(_) => Task::none(),
            Message::Saved(result) => {
                self.busy = false;
                self.resetting_layout = false;
                match result {
                    Ok(mut settings) => {
                        // A native import is complete only once its cached image is persisted.
                        if self
                            .wallpaper
                            .native_saved_path
                            .as_deref()
                            .is_some_and(|path| {
                                if self.wallpaper.native_saved_lock {
                                    settings.session.lock_clock.background.as_deref() == Some(path)
                                } else {
                                    settings.wallpaper.kind
                                        == enzui_shell::desktop_preferences::WallpaperKind::Image
                                        && settings.wallpaper.image.as_deref() == Some(path)
                                }
                            })
                        {
                            self.wallpaper.native_saved_path = None;
                            if let Some(drop) = self.wallpaper.native_drop.take() {
                                drop.finish();
                            }
                        } else if self.pending.is_empty()
                            && self.wallpaper.native_saved_path.is_some()
                        {
                            self.wallpaper.native_drop = None;
                            self.wallpaper.native_saved_path = None;
                        }
                        for change in &self.pending {
                            if let Err(error) = settings.preview(change.clone()) {
                                return self.update(Message::Saved(Err(error)));
                            }
                        }
                        self.settings = settings;
                        self.publish_text_size();
                        if let Some(next) = self.pending.pop_front() {
                            self.write(next)
                        } else if self.closing
                            && !self.plugins.busy()
                            && !self.wallpaper.pending()
                            && !self.users_pending()
                        {
                            iced::exit()
                        } else {
                            Task::none()
                        }
                    }
                    Err(error) => {
                        self.wallpaper.native_drop = None;
                        self.wallpaper.native_saved_path = None;
                        self.closing = false;
                        self.pending.clear();
                        self.error = Some(error);
                        self.reading = true;
                        self.read()
                    }
                }
            }
            Message::Loaded(generation, result) => {
                if generation != self.generation {
                    // An edit made while the newest read was in flight outdates its
                    // snapshot; no later read will arrive to end the pending state.
                    if generation == self.read_generation && self.reading {
                        if self.busy {
                            self.reading = false;
                        } else {
                            return self.read();
                        }
                    }
                    return Task::none();
                }
                self.reading = false;
                let mut agent = Task::none();
                if !self.busy {
                    match result {
                        Ok(settings) => {
                            self.settings = settings;
                            self.publish_text_size();
                            self.sync_wallpaper_drafts();
                            // A route Settings was opened at waits for this read.
                            if self.reveal_pending
                                && let Some(route) = self.highlighted
                            {
                                self.show_route_section(route);
                            }
                            agent = self.agent_settings_read(true);
                        }
                        Err(error) => {
                            self.error = Some(error);
                            agent = self.agent_settings_read(false);
                        }
                    }
                }
                let preview = if self.page == Page::Appearance {
                    self.warm_wallpaper_preview()
                } else {
                    Task::none()
                };
                Task::batch([agent, preview])
            }
            Message::FilesChanged if !self.busy && !self.reading => self.read_files(),
            Message::FilesChanged => Task::none(),
            // With the folder watched, the files are read again only as a
            // safety net for a change the watch missed.
            Message::Tick
                if !self.busy
                    && !self.reading
                    && (self.files_watch.is_none()
                        || self.files_read_at.elapsed() >= Duration::from_secs(30)) =>
            {
                Task::batch([self.read_files(), self.system_tick()])
            }
            Message::Tick => Task::batch([self.read_controls(), self.system_tick()]),
            Message::Connectivity => self.network_changed(),
            Message::Close | Message::Window(WindowCommand::Close) => {
                self.file_chooser_leave();
                self.startup.close();
                self.sandbox_leave();
                self.developer_leave();
                self.remote_leave();
                self.phone_leave();
                self.dev_tools_launch.cancel(true);
                self.backup_setup_leave();
                self.navigation.cancel();
                let numeric = self
                    .numeric
                    .flush(&self.settings)
                    .map_or_else(Task::none, |change| self.update(Message::Change(change)));
                self.numeric.close();
                self.picker = None;
                if !self.wallpaper.busy {
                    self.image_purpose = super::ImagePurpose::Desktop;
                }
                self.wallpaper.cancel();
                self.agent.shutdown();
                let close = if self.busy
                    || self.plugins.busy()
                    || self.wallpaper.pending()
                    || self.users_pending()
                {
                    self.closing = true;
                    Task::none()
                } else {
                    iced::exit()
                };
                Task::batch([numeric, close])
            }
            Message::Window(command) => self.chrome.run_command(command),
            Message::WindowState(event) => self.chrome.update(event).map(Message::WindowState),
            Message::LockFontOpen(open) => self.lock_font_open(open),
            Message::LockFontMove(movement) => self.lock_font_move(movement),
            Message::LockFontSelect(family) => {
                self.menu = None;
                let change = if family == "Fredoka" {
                    enzui_shell::session_preferences::SessionChange::LockFontReset
                } else {
                    enzui_shell::session_preferences::SessionChange::LockFont(Some(family))
                };
                self.update(Message::Change(Change::Session(change)))
            }
            Message::LockBackgroundDropped(drop) => {
                if self.page != Page::Appearance
                    || !self.controls_live()
                    || self.wallpaper.pending()
                    || self.wallpaper.native_drop.is_some()
                    || drop.paths().len() != 1
                    || drop
                        .action()
                        .is_some_and(|action| action != enzui_widgets::file_drop::DragAction::Copy)
                {
                    return Task::none();
                }
                let Some(path) = drop.paths()[0].to_str() else {
                    return Task::none();
                };
                path.clone_into(&mut self.wallpaper.path);
                self.wallpaper.expected_digest = None;
                self.wallpaper.import_choice = None;
                self.wallpaper.native_saved_lock = true;
                self.wallpaper.native_drop = Some(drop);
                self.image_purpose = super::ImagePurpose::Lock;
                let task = self.wallpaper_import();
                if !self.wallpaper.busy {
                    self.wallpaper.native_drop = None;
                }
                task
            }
            Message::LockBackgroundChoose => self.start_image_picker(super::ImagePurpose::Lock),
            Message::FontQuery(query) => {
                self.font_query = query.map(|(slot, text)| (slot, text.chars().take(64).collect()));
                Task::none()
            }
            Message::ReduceMotion(reduced) => {
                self.theme_editor_generation = self.theme_editor_generation.wrapping_add(1);
                if self.themes.is_some() {
                    self.theme = enzui_shell::theme_store::Store::read(&theme_paths());
                }
                if let Err(error) = theme_settings::update(
                    &mut self.theme,
                    theme_settings::Action::ReducedMotion(reduced),
                ) {
                    self.error = Some(error);
                    return Task::none();
                }
                let animations = enzui_shell::desktop_preferences::OptionKey::Animations;
                self.update(Message::Change(enzui_shell::settings::Change::Compositor(
                    animations,
                    i32::from(!reduced),
                )))
            }
            Message::SharedTheme(event) => {
                let Some(themes) = &mut self.themes else {
                    return Task::none();
                };
                let generation = themes.theme_generation();
                let task = themes
                    .update(*event)
                    .map(|event| Message::SharedTheme(Box::new(event)));
                if themes.theme_generation() == generation {
                    return task;
                }
                self.app_style.refresh();
                self.theme_editor_generation = self.theme_editor_generation.wrapping_add(1);
                let generation = self.theme_editor_generation;
                let reload = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(|| {
                            enzui_shell::theme_store::Store::read(&theme_paths())
                        })
                        .await
                        .ok()
                    },
                    move |store| {
                        store.map_or(Message::Noop, |store| {
                            Message::ThemeEditorRead(generation, Box::new(store))
                        })
                    },
                );
                // A window the new theme turns to glass (window material) is
                // only bound on an update after its view was rebuilt; an idle
                // Settings window would otherwise stay see-through, unbound.
                let rebind = Task::perform(
                    async { tokio::time::sleep(std::time::Duration::from_millis(150)).await },
                    |()| Message::Noop,
                );
                Task::batch([task, reload, rebind])
            }
            Message::ThemeEditorRead(generation, store) => {
                if generation == self.theme_editor_generation && !self.closing {
                    self.theme = *store;
                }
                Task::none()
            }
            Message::Theme(action) => {
                self.theme_editor_generation = self.theme_editor_generation.wrapping_add(1);
                let generation = self.theme_editor_generation;
                let compositor = (matches!(&action, theme_settings::Action::Choose { .. })
                    && !self.tokens().motion.reduced_motion)
                    .then(enzui_shell::compositor::Compositor::from_environment)
                    .and_then(Result::ok);
                Task::perform(
                    async move {
                        if let Some(compositor) = compositor {
                            compositor.appearance_transition(false).await;
                        }
                        action
                    },
                    move |action| Message::ThemePrepared(generation, action),
                )
            }
            Message::ThemePrepared(generation, _) if generation != self.theme_editor_generation => {
                Task::none()
            }
            Message::ThemePrepared(_, action) => {
                self.theme_editor_generation = self.theme_editor_generation.wrapping_add(1);
                if self.themes.is_some() {
                    self.theme = enzui_shell::theme_store::Store::read(&theme_paths());
                }
                let chosen = match &action {
                    theme_settings::Action::Choose { before, after } => Some((*before, *after)),
                    _ => None,
                };
                let saved = match theme_settings::update(&mut self.theme, action) {
                    Ok(()) => true,
                    Err(error) => {
                        self.error = Some(error);
                        false
                    }
                };
                let mut tasks = Vec::new();
                if chosen.is_some()
                    && let Ok(compositor) = enzui_shell::compositor::Compositor::from_environment()
                {
                    tasks.push(Task::perform(
                        async move {
                            compositor.appearance_transition(true).await;
                        },
                        |()| Message::Noop,
                    ));
                }
                if let Some((before, choices)) = chosen.filter(|_| saved) {
                    let id = choices.scheme.to_id_string();
                    if self.settings.theme_scheme.as_ref() != Some(&id) {
                        tasks.push(self.update(Message::Change(
                            enzui_shell::settings::Change::ThemeScheme(id),
                        )));
                    }
                    // Windows take the corner style too; their own radius
                    // stays adjustable on the Windows page.
                    if choices.corners != before.corners {
                        let rounding = enzui_shell::desktop_preferences::OptionKey::Rounding;
                        tasks.push(self.update(Message::Change(
                            enzui_shell::settings::Change::Compositor(
                                rounding,
                                i32::from(choices.corners.window_radius()),
                            ),
                        )));
                    }
                }
                Task::batch(tasks)
            }
            Message::ThemeEdit(edit) => self.theme_edit(edit),
            Message::AppStyle(action) => self.app_style_update(action),
        };
        self.controller_context_changed();
        let tokens = self.tokens();
        let published =
            self.themes.is_none() && enzui_shell::font_settings::publish_text_defaults(&tokens);
        let font = enzui_shell::font_settings::renderer_default(&tokens);
        let renderer_font =
            if self.themes.is_some() || self.renderer_font == Some(font) && !published {
                Task::none()
            } else {
                self.renderer_font = Some(font);
                iced::font::set_defaults(
                    font,
                    enzui_shell::font_settings::renderer_text_size(&tokens),
                )
            };
        if let Some(failure) = saved {
            self.agent_saved(failure);
        }
        self.sync_shortcuts();
        self.sync_night_labels();
        let follow = self.follow_wallpaper_colour();
        let background = self.background_sync();
        Task::batch([task, renderer_font, applied, follow, background])
    }
}
