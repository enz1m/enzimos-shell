use super::{Message, Request, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::system_controls::Action;
use iced::{Task, window};
use std::time::Instant;

impl Shell {
    pub(crate) fn settings_shortcut(&mut self) -> Task<Message> {
        if self.settings_shortcut_busy
            || self
                .settings_shortcut_retry_at
                .is_some_and(|at| Instant::now() < at)
        {
            return Task::none();
        }
        self.settings_shortcut_busy = true;
        self.settings_shortcut_retry_at = None;
        let compositor = self.compositor.clone();
        let root = self.root.clone();
        Task::perform(
            async move { compositor.install_settings_shortcut(&root).await },
            Message::SettingsShortcutReady,
        )
    }

    pub(crate) fn drag_maximize(&mut self) -> Task<Message> {
        let accent = self.base_tokens().palette.accent;
        let radius = self.metrics.radius;
        let theme = (
            [
                (accent.r * 255.0).round().clamp(0.0, 255.0) as u8,
                (accent.g * 255.0).round().clamp(0.0, 255.0) as u8,
                (accent.b * 255.0).round().clamp(0.0, 255.0) as u8,
            ],
            radius,
        );
        if self.drag_maximize_ready {
            if self.drag_maximize_theme == Some(theme) {
                return Task::none();
            }
            self.drag_maximize_theme = Some(theme);
            let compositor = self.compositor.clone();
            return Task::perform(
                async move {
                    compositor
                        .update_drag_maximize_theme(theme.0, theme.1)
                        .await
                },
                Message::DragMaximizeReady,
            );
        }
        if self.drag_maximize_busy
            || self
                .drag_maximize_retry_at
                .is_some_and(|at| Instant::now() < at)
        {
            return Task::none();
        }
        self.drag_maximize_busy = true;
        self.drag_maximize_retry_at = None;
        self.drag_maximize_theme = Some(theme);
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.install_drag_maximize(theme.0, theme.1).await },
            Message::DragMaximizeReady,
        )
    }

    // Compositor-drawn title bars take the theme enzUI applications draw theirs with.
    pub(crate) fn titlebar_theme(&mut self) -> Task<Message> {
        let theme = enzui_shell::titlebar_theme::TitleBarTheme::new(
            &self.app_theme(),
            enzui_shell::titlebar_theme::buttons(),
        );
        if self.titlebar_theme.as_ref() == Some(&theme) {
            return Task::none();
        }
        self.titlebar_theme = Some(theme.clone());
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.apply_titlebar_theme(&theme).await },
            Message::TitleBarThemeApplied,
        )
    }

    pub(crate) fn refresh(&mut self, request: Option<Request>) -> Task<Message> {
        self.busy = true;
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                match request {
                    Some(Request::DesktopModes(default, modes)) => {
                        compositor.set_desktop_modes(default, &modes).await
                    }
                    None => compositor.snapshot().await,
                }
            },
            Message::Refreshed,
        )
    }

    pub(crate) fn close_launcher(&mut self) -> Task<Message> {
        if self.launcher.is_some() && self.launcher_geometry.is_some() && self.launcher_started {
            // The island popup slides out first; LauncherHidden closes the window.
            self.launcher_visible = false;
            return Task::none();
        }
        self.launcher.take().map_or_else(Task::none, window::close)
    }

    pub(crate) fn force_close_launcher(&mut self) -> Task<Message> {
        if let Some(id) = self.launcher {
            self.cancel_panel_action_for(id);
        }
        self.finish_launcher()
    }

    pub(crate) fn finish_launcher(&mut self) -> Task<Message> {
        if self.launcher.is_none() && !self.launcher_opening {
            return Task::none();
        }
        self.launcher_visible = false;
        self.launcher_started = false;
        self.launcher_geometry = None;
        self.launcher_placing = false;
        self.launcher_opening = false;
        self.launcher
            .take()
            .map_or_else(Task::none, |id| self.finish_panel_close(id))
    }

    pub(crate) fn island_panel(&self, id: window::Id) -> bool {
        self.metrics.islands.is_some()
            && (Some(id) == self.control_center
                || Some(id) == self.overview
                || Some(id) == self.launcher)
    }

    pub(crate) fn input_region_window(&self, id: window::Id) -> bool {
        self.dock.window == Some(id)
            || self.control_center == Some(id)
            || self.search.window == Some(id)
            || self.plugin.owns_window(id)
            || self.notification.owns_window(id)
            || self.assistant.owns(id)
            || self.island_panel(id)
    }

    pub(crate) fn close_input_window(&mut self, id: window::Id) -> Task<Message> {
        self.pending_surface_focus.remove(&id);
        self.pending_surface_requests.remove(&id);
        if let Some(input) = self.surface_inputs.remove(&id) {
            input.invalidate();
        }
        if self.dock.window == Some(id) {
            self.desired_settings.dock.enabled = false;
            self.dock.window = None;
            self.dock.menu = None;
            window::close(id).chain(self.dock_save(enzui_shell::dock::Change::Enabled(false)))
        } else if !self.island_panel(id) {
            Task::none()
        } else if Some(id) == self.control_center {
            self.force_close_controls()
        } else if Some(id) == self.overview {
            self.force_close_overview()
        } else {
            self.force_close_launcher()
        }
    }

    pub(crate) fn sync_sidebar_input(&mut self) -> Task<Message> {
        let Some(id) = self.sidebar_rail else {
            return Task::none();
        };
        if self.metrics.islands.is_some() {
            if self.surface_inputs.contains_key(&id) {
                Task::none()
            } else {
                self.acquire_sidebar_input(id)
            }
        } else {
            if let Some(input) = self.surface_inputs.remove(&id) {
                input.reset();
                input.invalidate();
            }
            Task::none()
        }
    }

    pub(crate) fn acquire_sidebar_input(&mut self, id: window::Id) -> Task<Message> {
        if self.metrics.islands.is_none() || Some(id) != self.sidebar_rail {
            return Task::none();
        }
        if !self.pending_surface_requests.insert(id) {
            return Task::none();
        }
        let shared = self.surface_globals.clone();
        iced_runtime::window::wayland_parent_export(id).then(move |payload| {
            let shared = shared.clone();
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        enzui_shell::surface_input::acquire(payload, shared)
                    })
                    .await
                    .unwrap_or_else(|_| Err(tr("shell-input-region-worker-stopped").into()))
                },
                move |result| Message::SidebarSurfaceReady(id, result),
            )
        })
    }

    pub(crate) fn cancel_panel_action(&mut self) {
        self.panel_action = None;
        self.panel_action_serial = self.panel_action_serial.wrapping_add(1);
    }
    pub(crate) fn cancel_panel_action_for(&mut self, id: window::Id) {
        if self
            .panel_action
            .as_ref()
            .is_some_and(|(owner, _, _)| *owner == id)
        {
            self.cancel_panel_action();
        }
    }
    pub(crate) fn queue_panel_action(&mut self, id: window::Id, next: Message) -> Task<Message> {
        let controls = self.control_center == Some(id);
        let plugin = self.plugin.pane.as_ref().is_some_and(|p| p.window == id);
        let notifications = self.notification.owns_center(id);
        if !controls && !plugin && !notifications {
            return Task::none();
        }
        self.cancel_panel_action();
        self.panel_action = Some((id, self.panel_action_serial, Box::new(next)));
        if controls {
            self.close_controls()
        } else if plugin {
            self.close_plugin()
        } else {
            self.close_notifications()
        }
    }
    pub(crate) fn finish_panel_close(&mut self, id: window::Id) -> Task<Message> {
        self.pending_surface_focus.remove(&id);
        self.pending_surface_requests.remove(&id);
        if let Some(input) = self.surface_inputs.remove(&id) {
            input.invalidate();
        }
        window::close(id).chain(self.take_panel_action(id))
    }

    fn take_panel_action(&mut self, id: window::Id) -> Task<Message> {
        if self
            .panel_action
            .as_ref()
            .is_some_and(|(owner, _, _)| *owner == id)
        {
            let (_, serial, next) = self
                .panel_action
                .take()
                .expect("the pending panel action was checked above");
            Task::done(Message::PanelActionReady(serial, next))
        } else {
            Task::none()
        }
    }

    pub(crate) fn park_controls(&mut self) -> Task<Message> {
        let Some(id) = self.control_center else {
            return Task::none();
        };
        if let Some(input) = self.surface_inputs.get(&id) {
            input.finish_hidden();
        }
        let reflow =
            self.control_details.is_some() || self.control_details_open || self.accessibility_menu;
        self.accessibility_menu = false;
        self.confirm = None;
        self.power_menu = false;
        self.control_connections.reset();
        self.control_details = None;
        self.control_details_open = false;
        self.control_menu = None;
        let compositor = self.compositor.clone();
        let park = Task::perform(
            async move { compositor.park_control_center().await },
            Message::FocusReturned,
        );
        let refresh = if reflow {
            self.reflow_controls()
        } else {
            Task::none()
        };
        park.chain(self.take_panel_action(id)).chain(refresh)
    }

    pub(crate) fn close_controls(&mut self) -> Task<Message> {
        self.control_requested = false;
        self.control_frames = 0;
        if self.control_center.is_none() && !self.control_opening {
            return Task::none();
        }
        self.control_visible = false;
        self.control_connections.clear_forms();
        if self.control_started {
            Task::none()
        } else {
            self.finish_controls()
        }
    }

    /// Other panels can leave the already hidden controls ready for reuse.
    pub(crate) fn close_controls_for_panel(&mut self) -> Task<Message> {
        self.cancel_panel_action();
        if self.control_visible || self.control_requested {
            self.force_close_controls()
        } else {
            Task::none()
        }
    }
    pub(crate) fn force_close_controls(&mut self) -> Task<Message> {
        if let Some(id) = self.control_center {
            self.cancel_panel_action_for(id);
        }
        let restore = self.control_started && self.control_center.is_some();
        let close = self.finish_controls();
        if restore {
            let compositor = self.compositor.clone();
            Task::perform(
                async move { compositor.park_control_center().await },
                Message::FocusReturned,
            )
            .chain(close)
        } else {
            close
        }
    }
    pub(crate) fn finish_controls(&mut self) -> Task<Message> {
        if self.control_center.is_none() && !self.control_opening {
            return Task::none();
        }
        self.control_requested = false;
        self.control_frames = 0;
        self.control_visible = false;
        self.control_started = false;
        self.control_warming = false;
        self.control_output = None;
        self.notice = None;
        self.control_opening = false;
        self.confirm = None;
        self.power_menu = false;
        self.control_connections.reset();
        self.control_details = None;
        self.control_details_open = false;
        self.control_menu = None;
        self.control_reflow_pending = false;
        self.control_placing = false;
        self.control_center
            .take()
            .map_or_else(Task::none, |id| self.finish_panel_close(id))
    }

    pub(crate) fn refresh_system(&mut self, action: Option<Action>) -> Task<Message> {
        self.system_busy = true;
        self.system_action_busy = action.is_some();
        let controls = self.controls.clone();
        // NetworkManager's signals keep the network current; an action reads
        // it at once, for the control that changed it.
        let network = !self.network_watched || action.is_some();
        // PipeWire's changes keep the levels current the same way.
        let audio = !self.audio_watched || action.is_some();
        Task::perform(
            async move {
                let result = if let Some(action) = &action {
                    controls.apply(action).await
                } else {
                    Ok(())
                };
                let snapshot = controls.snapshot_with(network, audio).await;
                (action, result, snapshot)
            },
            |(action, result, snapshot)| Message::SystemRefreshed(action, result, snapshot),
        )
    }

    pub(crate) fn enqueue_system(&mut self, action: Action) -> Task<Message> {
        self.notice = None;
        if self.system_busy {
            self.system_queue.push(action);
            Task::none()
        } else {
            self.refresh_system(Some(action))
        }
    }

    pub(crate) fn enqueue(&mut self, request: Request) -> Task<Message> {
        if !self.busy {
            return self.refresh(Some(request));
        }
        if self.requests.len() < 8 {
            self.requests.push_back(request);
        }
        Task::none()
    }
}
