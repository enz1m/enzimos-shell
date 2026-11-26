use super::{Message, Shell};
use crate::quick_search;
use enzui_shell::i18n::tr;
use iced::{Task, window};

impl Shell {
    pub(crate) fn update_windows(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Escape(id)
            | Message::Window(id, window::Event::CloseRequested | window::Event::Unfocused)
                if Some(id) == self.shortcut_overlay.window =>
            {
                self.shortcuts_close()
            }
            Message::Window(id, window::Event::Closed)
                if Some(id) == self.shortcut_overlay.window =>
            {
                self.shortcut_overlay.window = None;
                Task::none()
            }

            Message::Window(id, window::Event::Resized(_)) if id == self.panel => {
                // Wayland can configure the bar after its initial widget bounds were sampled.
                self.control_reflow_pending |= self.control_opening;
                self.warm_controls()
            }
            Message::Window(id, window::Event::CloseRequested) if Some(id) == self.dock.window => {
                self.close_input_window(id)
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.dock.window => {
                self.desired_settings.dock.enabled = false;
                self.dock.window = None;
                self.dock.menu = None;
                if let Some(input) = self.surface_inputs.remove(&id) {
                    input.invalidate();
                }
                self.dock_save(enzui_shell::dock::Change::Enabled(false))
            }

            Message::Window(id, event) if self.emoji.owns(id) => self.emoji_window(&event),
            Message::Window(id, event) if self.osk.owns(id) => self.osk_window(&event),
            Message::Escape(id) if self.emoji.owns(id) => self.close_emoji_picker(),
            Message::Window(id, event) if self.capture.owns(id) => self.capture_window(&event),
            Message::Escape(id) if self.capture.owns(id) => self.close_capture(),
            // Nothing is focused at first: Enter and Escape work regardless.
            Message::Opened(id) if self.capture.owns(id) => Task::none(),
            Message::Opened(id) if self.spread.owns(id) => self.spread_opened(id),
            Message::Escape(id) if self.spread.owns(id) => self.close_spread(None),
            Message::Window(id, window::Event::Closed) if self.spread.closes(id) => {
                self.spread_closed(id)
            }
            Message::Opened(id) if self.emoji.owns(id) => self.emoji_opened(id),
            Message::Opened(id)
                if self.owns_osd(id) || self.owns_switcher(id) || self.osk.owns(id) =>
            {
                self.bind_transient_glass(id)
            }
            Message::Opened(id)
                if Some(id) == self.sidebar_rail && self.metrics.islands.is_some() =>
            {
                self.acquire_sidebar_input(id)
            }
            Message::Window(id, window::Event::Focused)
                if self.input_region_window(id) && !self.surface_inputs.contains_key(&id) =>
            {
                self.pending_surface_focus.insert(id);
                Task::none()
            }
            Message::Window(id, window::Event::Unfocused)
                if self.input_region_window(id)
                    && !self.surface_inputs.contains_key(&id)
                    && self.pending_surface_focus.contains(&id) =>
            {
                self.close_input_window(id)
            }
            Message::Opened(id)
                if self.input_region_window(id) && !self.surface_inputs.contains_key(&id) =>
            {
                if !self.pending_surface_requests.insert(id) {
                    Task::none()
                } else {
                    let shared = self.surface_globals.clone();
                    iced_runtime::window::wayland_parent_export(id).then(move |payload| {
                        let shared = shared.clone();
                        Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    enzui_shell::surface_input::acquire(payload, shared)
                                })
                                .await
                                .unwrap_or_else(|_| {
                                    Err(tr("shell-input-region-worker-stopped").into())
                                })
                            },
                            move |result| Message::SurfaceInputReady(id, result),
                        )
                    })
                }
            }
            Message::Opened(id) if Some(id) == self.dock.window => {
                let compositor = self.compositor.clone();
                let geometry = self.dock.geometry.clone();
                Task::perform(
                    async move {
                        if let Some(g) = geometry {
                            compositor.place_dock(&g).await
                        } else {
                            Ok(())
                        }
                    },
                    |r| Message::Dock(crate::dock_ui::Event::Done(0, r)),
                )
            }
            Message::Window(id, event) if self.assistant.owns(id) => {
                self.assistant_window_event(id, &event)
            }
            Message::Escape(id) if self.assistant.owns(id) => self.assistant_escape(id),
            Message::Opened(id) if self.assistant.owns(id) => self.assistant_mapped(id),
            Message::Window(id, event) if self.search.window == Some(id) => {
                self.search_window(id, &event)
            }
            Message::Escape(id) if self.search.window == Some(id) => {
                self.search_message(quick_search::Message::Close)
            }
            Message::Opened(id) if self.search.window == Some(id) => self.search_opened(id),
            Message::Window(id, event) if self.notification.owns_window(id) => {
                self.notification_window(id, &event)
            }
            Message::Escape(id) if self.notification.owns_window(id) => {
                self.notification_escape(id)
            }
            Message::Opened(id) if self.notification.owns_window(id) => {
                self.notification_opened(id)
            }
            Message::Window(id, event) if self.plugin.owns_window(id) => {
                self.plugin_window(id, &event).unwrap_or_else(Task::none)
            }
            Message::Escape(id) if self.plugin.owns_window(id) => self.close_plugin(),
            Message::Opened(id) if self.plugin.owns_window(id) => Task::none(),
            Message::Window(id, window::Event::Focused)
                if Some(id) == self.overview && !self.overview_started =>
            {
                self.overview_started = true;
                self.overview_visible = true;
                let focus = if let Some(desktops) = &self.desktops {
                    iced::widget::operation::focus(format!("desktop-card-{}", desktops.active))
                } else {
                    Task::none()
                };
                Task::batch([focus, self.watch_overview_reveal()])
            }
            Message::Window(id, window::Event::CloseRequested) if Some(id) == self.overview => {
                self.close_overview()
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.overview => {
                self.force_close_overview()
            }
            Message::Window(id, window::Event::Focused) if Some(id) == self.sidebar_rail => {
                self.sidebar_focused = true;
                Task::done(Message::FocusSidebar(id))
            }
            Message::Window(id, window::Event::Unfocused) if Some(id) == self.sidebar_rail => {
                self.sidebar_focused = false;
                Task::none()
            }
            Message::Opened(id) if Some(id) == self.sidebar_pane => {
                window::enable_mouse_passthrough(id)
            }
            Message::Opened(id) if Some(id) == self.sidebar_rail => {
                if self
                    .sidebar
                    .as_ref()
                    .is_some_and(enzui_shell::sidebar::Snapshot::visible)
                {
                    window::disable_mouse_passthrough(id)
                } else {
                    window::enable_mouse_passthrough(id)
                }
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.sidebar_rail => {
                self.sidebar_rail = None;
                self.sidebar_focused = false;
                Task::none()
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.sidebar_pane => {
                self.sidebar_pane = None;
                Task::none()
            }
            Message::Opened(id) if Some(id) == self.control_center => self.controls_ready(),
            Message::Opened(id) if Some(id) == self.overview => Task::none(),
            Message::Escape(id) if Some(id) == self.overview => self.close_overview(),
            Message::Window(id, window::Event::Focused) if Some(id) == self.control_center => {
                Task::none()
            }
            Message::Opened(id) if Some(id) == self.launcher => {
                iced::widget::operation::focus("launcher-search")
            }
            Message::Opened(_) if !std::mem::replace(&mut self.renderer_logged, true) => {
                iced::system::information()
                    .map(|info| Message::Renderer(info.graphics_backend, info.graphics_adapter))
            }
            Message::Opened(_) => Task::none(),
            Message::Window(id, window::Event::CloseRequested | window::Event::Closed)
                if id == self.panel =>
            {
                iced::exit()
            }
            Message::Window(id, window::Event::CloseRequested)
                if Some(id) == self.control_center =>
            {
                self.close_controls()
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.control_center => {
                self.finish_controls()
            }
            Message::Escape(id) if Some(id) == self.control_center => {
                if self.control_details_open {
                    self.update(Message::ControlDetails(None))
                } else {
                    self.close_controls()
                }
            }
            Message::Window(id, window::Event::Focused) if Some(id) == self.launcher => {
                self.launcher_focus_revision = self.launcher_focus_revision.wrapping_add(1);
                if !self.launcher_started {
                    self.launcher_started = true;
                    self.launcher_visible = true;
                    iced::widget::operation::focus("launcher-search")
                } else {
                    Task::none()
                }
            }
            Message::Window(id, window::Event::CloseRequested) if Some(id) == self.launcher => {
                self.close_launcher()
            }
            Message::Window(id, window::Event::Closed) if Some(id) == self.launcher => {
                self.launcher = None;
                self.finish_launcher()
            }
            Message::Window(id, window::Event::Unfocused)
                if Some(id) == self.launcher && !self.launching =>
            {
                self.launcher_focus_revision = self.launcher_focus_revision.wrapping_add(1);
                let revision = self.launcher_focus_revision;
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.launcher_focus_retained().await.unwrap_or(false) },
                    move |retained| Message::LauncherFocusChecked(id, revision, retained),
                )
            }
            Message::Escape(id) if Some(id) == self.launcher => self.close_launcher(),
            Message::Escape(id) if id == self.panel => {
                self.cancel_panel_action();
                Task::batch([self.close_overview(), self.close_controls()])
            }
            Message::Window(_, _) | Message::Escape(_) => Task::none(),
            _ => Task::none(),
        }
    }
}
