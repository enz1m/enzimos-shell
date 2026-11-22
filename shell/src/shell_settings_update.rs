use super::{Message, Shell, resize_chrome};
use enzui_shell::tr;
use iced::{Task, window};
use std::time::{Duration, Instant};

impl Shell {
    pub(crate) fn update_settings(&mut self, message: Message) -> Task<Message> {
        if matches!(
            &message,
            Message::SettingsPage(_) | Message::SettingsPlugin(_)
        ) && let Some(id) = self
            .control_center
            .filter(|_| self.control_visible || self.control_requested)
        {
            return self.queue_panel_action(id, message);
        }
        match message {
            Message::SettingsPlugin(id) => {
                let compositor = self.compositor.clone();
                let root = self.root.clone();
                let serial = self.panel_action_serial;
                Task::perform(
                    async move { compositor.launch_settings_plugin(&root, &id).await },
                    move |result| Message::SettingsLaunched(serial, result),
                )
            }
            Message::SettingsPage(page) => {
                let compositor = self.compositor.clone();
                let root = self.root.clone();
                let serial = self.panel_action_serial;
                Task::perform(
                    async move { compositor.launch_settings_page(&root, page).await },
                    move |result| Message::SettingsLaunched(serial, result),
                )
            }
            Message::SettingsLaunched(serial, result) => {
                if serial == self.panel_action_serial
                    && let Err(error) = result
                {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::DesktopPreferencesApplied(wanted, result) => {
                self.preferences_busy = false;
                match result {
                    Ok(()) => {
                        self.desktop_applied = Some(wanted);
                        self.desktop_retry_at = None;
                        self.setup_errors.preferences = None;
                    }
                    Err(error) => {
                        self.setup_errors.preferences =
                            Some(tr!("shell-desktop-preferences", "arg0" => error));
                        self.desktop_retry_at =
                            Some(Instant::now() + std::time::Duration::from_secs(3));
                    }
                }
                Task::none()
            }
            Message::SidebarSurfaceReady(id, result) => {
                self.pending_surface_requests.remove(&id);
                if Some(id) != self.sidebar_rail {
                    if let Ok(input) = result {
                        input.invalidate();
                    }
                    Task::none()
                } else {
                    match result {
                        Ok(input) => {
                            if self.metrics.islands.is_none()
                                && self
                                    .sidebar
                                    .as_ref()
                                    .is_some_and(enzui_shell::sidebar::Snapshot::visible)
                            {
                                input.reset();
                            }
                            self.surface_globals = Some(input.globals());
                            self.surface_inputs.insert(id, input);
                            self.sidebar_input_failed = false;
                        }
                        Err(error) => {
                            self.sidebar_input_failed = true;
                            self.setup_errors.sidebar = Some(error);
                        }
                    }
                    Task::none()
                }
            }
            Message::SurfaceInputReady(id, result) => {
                self.pending_surface_requests.remove(&id);
                let focused = self.pending_surface_focus.remove(&id);
                if !self.input_region_window(id) {
                    if let Ok(input) = result {
                        input.invalidate();
                    }
                    Task::none()
                } else {
                    match result {
                        Ok(input) => {
                            self.surface_globals = Some(input.globals());
                            self.surface_inputs.insert(id, input);
                            if Some(id) == self.control_center {
                                self.controls_ready()
                            } else if Some(id) == self.dock.window {
                                Task::done(Message::Opened(id))
                            } else if Some(id) == self.search.window {
                                self.search_opened(id)
                            } else if self.assistant.owns_trigger(id) {
                                // The corner button never takes focus.
                                self.assistant_mapped(id)
                            } else {
                                Task::done(Message::Opened(id)).chain(if focused {
                                    Task::done(Message::Window(id, window::Event::Focused))
                                } else {
                                    window::gain_focus(id)
                                })
                            }
                        }
                        Err(error) => {
                            let close = self.close_input_window(id);
                            self.notice = Some(error);
                            close
                        }
                    }
                }
            }
            Message::SettingsLoaded(generation, result) => {
                self.settings_read_busy = false;
                let again = if std::mem::take(&mut self.settings_reread) && !self.settings_save_busy
                {
                    self.read_settings()
                } else {
                    Task::none()
                };
                if self.settings_save_busy || generation != self.settings_generation {
                    self.quiet = again.units() == 0;
                    again
                } else {
                    let same = result
                        .as_ref()
                        .is_ok_and(|settings| *settings == self.desired_settings);
                    let theme = self.theme_document.clone();
                    let night_light = self.night_light_target;
                    let glass = self.bar_glass_on;
                    let task =
                        Task::batch([self.accept_settings(result), self.refresh_bar_glass()]);
                    self.quiet = same
                        && task.units() == 0
                        && again.units() == 0
                        && self.theme_document == theme
                        && self.night_light_target == night_light
                        && glass == self.bar_glass_on;
                    Task::batch([task, again])
                }
            }
            Message::SettingsSaved(result) => {
                self.settings_save_busy = false;
                if let Err(error) = &result {
                    self.search.feedback = Some(error.clone());
                }
                let applied = Task::batch([self.accept_settings(result), self.refresh_bar_glass()]);
                if let Some(change) = self.settings_queue.pop_front() {
                    Task::batch([applied, self.save_setting(change)])
                } else if std::mem::take(&mut self.settings_reread) && !self.settings_read_busy {
                    Task::batch([applied, self.read_settings()])
                } else {
                    applied
                }
            }
            Message::BarGeometry(epoch, serial, geometry) => {
                if self.bar_busy
                    || epoch != self.bar_applied_epoch
                    || self.metrics.islands.is_none()
                    || !geometry.valid()
                    // A serial orders the measurements of one epoch only. The
                    // islands' observer starts counting again when the bar is
                    // rebuilt (Islands, Frame, Islands), and comparing across
                    // that kept the old layout's input and glass regions: a
                    // connected bar then took clicks only where the separate
                    // pills had been.
                    || (self.bar_geometry_epoch == epoch
                        && self
                            .bar_geometry
                            .as_ref()
                            .is_some_and(|(last, _)| serial <= *last))
                {
                    return Task::none();
                }
                self.bar_geometry_epoch = epoch;
                self.bar_geometry = Some((serial, geometry));
                self.update_frame();
                Task::batch([
                    self.publish_bar_regions(),
                    self.refresh_bar_glass(),
                    Task::done(Message::WarmControls),
                ])
            }
            Message::BarGlassReady(result) => {
                self.bar_glass_pending = false;
                match result {
                    Ok(input) => {
                        self.bar_glass = Some(input);
                        self.refresh_bar_glass()
                    }
                    Err(error) => {
                        eprintln!("enzui-shell bar glass: {error}");
                        Task::none()
                    }
                }
            }
            Message::BarRegionsApplied(epoch, serial, result) => {
                self.bar_regions_busy = false;
                if epoch == self.bar_applied_epoch && epoch == self.bar_epoch && !self.bar_busy {
                    match result {
                        Ok(()) => {
                            self.bar_regions_sent = Some((epoch, serial));
                            self.bar_regions_retry = None;
                            self.setup_errors.bar = None;
                        }
                        Err(error) => {
                            self.bar_regions_retry = Some(Instant::now() + Duration::from_secs(1));
                            self.setup_errors.bar = Some(error);
                        }
                    }
                }
                self.publish_bar_regions()
            }
            Message::FrameFadeTick => self.frame_fade_tick(),
            Message::RailAlphaTick => self.rail_alpha_tick(),
            Message::RailAlphaSurface(id, result) => self.rail_alpha_surface_ready(id, result),
            Message::BarStyleHidden if !self.bar_busy => self.apply_bar(),
            Message::BarStyleHidden => {
                self.bar_style_fade = false;
                Task::none()
            }
            Message::BarApplied(edge, metrics, result) => {
                self.bar_busy = false;
                self.bar_style_fade = false;
                match result {
                    Ok(geometry) => {
                        self.setup_errors.bar = None;
                        self.bar = edge;
                        // A new style fades in with its bar, and so does the first one.
                        let restyled = !self.chrome_styled
                            || self.metrics.islands.is_some() != metrics.islands.is_some();
                        self.chrome_styled = true;
                        // During a scale change it waits for the placement on
                        // the settled output; one that output changed under
                        // places it again first.
                        let fade = if let Some(hold) = self.rescale_hold {
                            if hold.placing && !self.bar_output_stale {
                                self.release_rescale_hold()
                            } else {
                                Task::none()
                            }
                        } else if restyled || self.frame_fade.is_some_and(|(_, out)| out) {
                            self.start_frame_fade(false)
                        } else {
                            Task::none()
                        };
                        self.metrics = metrics;
                        self.bar_applied_epoch = self.bar_epoch;
                        self.sidebar_configured = None;
                        self.bar_ready = true;
                        self.update_frame();
                        let size = iced::Size::new(geometry.width, geometry.height);
                        let resize = if self.bar_size != size {
                            self.bar_size = size;
                            resize_chrome(self.panel, size)
                        } else {
                            Task::none()
                        };
                        let next = Task::batch([
                            self.update_bar_preferences(),
                            self.refresh_sidebar(None),
                            self.sync_sidebar_input(),
                        ]);
                        resize.chain(Task::batch([
                            fade,
                            next,
                            self.settings_shortcut(),
                            self.notification_geometry_changed(),
                            Task::done(Message::WarmControls),
                        ]))
                    }
                    Err(error) => {
                        self.setup_errors.bar = Some(tr!("shell-desktop-bar", "arg0" => error));
                        // A bar that cannot be placed still shows.
                        self.chrome_styled = true;
                        self.update_frame();
                        self.release_rescale_hold()
                    }
                }
            }
            Message::SettingsShortcutReady(result) => {
                self.settings_shortcut_busy = false;
                let before = self.setup_errors.shortcut.clone();
                match result {
                    Ok(()) => {
                        self.setup_errors.shortcut = None;
                        self.settings_shortcut_retry_at = None;
                    }
                    Err(error) => {
                        self.setup_errors.shortcut =
                            Some(tr!("shell-settings-shortcut", "arg0" => error));
                        self.settings_shortcut_retry_at =
                            Some(Instant::now() + Duration::from_secs(3));
                    }
                }
                // A retry that ends as the last one did changes nothing shown.
                self.quiet = self.setup_errors.shortcut == before;
                Task::none()
            }
            _ => Task::none(),
        }
    }
}
