use super::{ControlBodyKey, Message, Shell, resize_chrome};
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::i18n::tr;
use iced::{Subscription, Task};
use std::time::{Duration, Instant};

impl Shell {
    pub(crate) fn warm_controls(&mut self) -> Task<Message> {
        if self.bar_busy || !self.bar_ready || self.control_center.is_some() || self.control_opening
        {
            return Task::none();
        }
        self.control_opening = true;
        self.control_placing = false;
        self.control_generation = self.control_generation.wrapping_add(1);
        let generation = self.control_generation;
        self.confirm = None;
        self.power_menu = false;
        self.control_details = None;
        self.control_details_open = false;
        self.control_detail_capacity = 0.0;
        enzui_shell::widget_bounds::bounds(self.panel, self.control_trigger())
            .map(move |rect| Message::ControlsAnchor(generation, rect))
    }

    pub(crate) fn controls_ready(&mut self) -> Task<Message> {
        if self.control_warming {
            let compositor = self.compositor.clone();
            return Task::perform(
                async move { compositor.prepare_new_control_center_frame().await },
                Message::FocusReturned,
            );
        }
        self.control_started = true;
        let reflow = std::mem::take(&mut self.control_reflow_pending);
        self.control_visible = self.control_requested && !reflow;
        let compositor = self.compositor.clone();
        let wanted = self.control_visible;
        if wanted {
            self.control_frames = 2;
        }
        let geometry = if reflow {
            self.reflow_controls()
        } else {
            Task::none()
        };
        Task::perform(
            async move {
                if wanted {
                    compositor.prepare_new_control_center_frame().await
                } else {
                    compositor.park_control_center().await
                }
            },
            Message::FocusReturned,
        )
        .chain(geometry)
    }

    pub(crate) fn controls_need_shrink(
        &self,
        before: &enzui_shell::control_layout::VisibleLayout,
        after: &enzui_shell::control_layout::VisibleLayout,
    ) -> bool {
        let radius = f32::from(self.metrics.radius);
        let padding = enzui_shell::panel_style::INSET;
        let (left, right) = self.control_attachment.insets(radius);
        let width = (self.control_size.width - left - right - padding * 2.0).max(1.0);
        let row = enzui_shell::control_layout::row_height(f32::from(self.metrics.text));
        let capacity = (self.control_size.height
            - padding * 2.0
            - enzui_shell::control_layout::ACTIONS_ROW
            - radius)
            .max(before.height(width, row));
        self.control_started
            && self.control_visible
            && self.confirm.is_none()
            && !self.power_menu
            && !after.actions.is_empty()
            && after.height(width, row) < capacity
    }

    pub(crate) fn visible_controls(&self) -> enzui_shell::control_layout::VisibleLayout {
        enzui_shell::control_layout::VisibleLayout {
            layout: std::sync::Arc::clone(&self.desired_settings.controls),
            actions: enzui_shell::settings::QuickAction::ALL
                .into_iter()
                .filter(|action| {
                    self.desired_settings.shows(*action) && self.system.available(*action)
                })
                .collect(),
            detail: self
                .control_details
                .filter(|_| self.control_details_open)
                .map(|action| (action, enzui_shell::control_tile::detail_height(action))),
        }
    }

    pub(crate) fn refresh_bar_glass(&mut self) -> Task<Message> {
        let before = self.bar_glass_on;
        let task = self.apply_bar_glass();
        // Glass islands cast their own shadow; the frame's is dropped.
        if before != self.bar_glass_on {
            self.update_frame();
        }
        task
    }

    fn apply_bar_glass(&mut self) -> Task<Message> {
        // Islands are glass one by one; the connected bar is one glass shape.
        let style = self
            .chrome_glass()
            .filter(|_| self.metrics.islands.is_some());
        let Some(_style) = style else {
            if let Some(input) = &self.bar_glass {
                input.clear_glass();
            }
            self.bar_glass_on = false;
            return Task::none();
        };
        let Some(input) = self.bar_glass.clone() else {
            if self.bar_glass_pending || !self.bar_ready {
                return Task::none();
            }
            self.bar_glass_pending = true;
            let shared = self.surface_globals.clone();
            return iced_runtime::window::wayland_parent_export(self.panel).then(move |payload| {
                let shared = shared.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            enzui_shell::surface_input::acquire_for_glass(payload, shared)
                        })
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-bar-glass-worker-stopped").into()))
                    },
                    Message::BarGlassReady,
                )
            });
        };
        let Some((_, _geometry)) = &self.bar_geometry else {
            return Task::none();
        };
        // The bar's draw wrapper sends material state immediately before its buffer commit.
        // Sending it here races the redraw, leaving the old tint until another bar commit.
        self.bar_glass_on = input.glass_supported();
        Task::none()
    }

    /// Whether something on screen needs the 2 s tick: a countdown, a panel
    /// that shows live state, or a sharing grant that is not watched.
    fn tick_busy(&self) -> bool {
        self.confirm_deadline.is_some()
            || self.notification.center_open()
            || self.control_visible
            || self.overview.is_some()
            || self.overview_opening
            || (self.sharing_watch.is_none() && Self::sharing_shown())
    }

    /// The idle tick's period from now; see [`shell_schedule::tick_period`].
    pub(crate) fn idle_tick_period(&self) -> Duration {
        use chrono::Timelike as _;
        let now = chrono::Local::now();
        let into_minute =
            f64::from(now.second()) + f64::from(now.nanosecond().min(999_999_999)) / 1e9;
        crate::shell_schedule::tick_period(false, into_minute)
    }

    /// [`Message::Tick`]: every 2 s while [`Self::tick_busy`], otherwise once
    /// at [`Shell::next_tick_at`], which each tick moves on.
    pub(crate) fn tick_subscription(&self) -> Subscription<Message> {
        if self.tick_busy() {
            return iced::time::every(crate::shell_schedule::FAST_TICK).map(|_| Message::Tick);
        }
        Subscription::run_with(self.next_tick_at, |at| {
            let at = tokio::time::Instant::from_std(*at);
            iced::futures::stream::once(async move {
                tokio::time::sleep_until(at).await;
            })
        })
        .map(|()| Message::Tick)
    }

    /// Whether this session can show a sharing grant at all.
    fn sharing_shown() -> bool {
        enzui_shell::desktop_session::mode() == enzui_shell::desktop_session::Mode::Installed
            && !enzui_shell::desktop_session::remote()
    }

    /// A watch on the sharing grant folder, where there is one to watch.
    pub(crate) fn watch_sharing_grant() -> Option<enzui_core::file_tree_watch::DirectoryWatchService>
    {
        use enzui_core::file_tree_watch::{DirectoryWatchService, linux::LinuxInotifyBackend};
        let directory = std::path::Path::new(enzui_shell::remote_access::SHARING_GRANT_DIRECTORY);
        if !Self::sharing_shown() || !directory.is_dir() {
            return None;
        }
        let service = DirectoryWatchService::new(LinuxInotifyBackend::new().ok()?);
        service.watch(directory).ok()?;
        Some(service)
    }

    /// How often [`Message::SettingsTick`] runs. It polls quickly only while
    /// it has something to finish: settings without a folder watch, or bar
    /// regions the compositor has not taken yet. Otherwise it only backs up
    /// the folder watch, expires clipboard history (kept for hours) and
    /// backs up the output check that desktop events run, so an idle
    /// desktop wakes for it every 10 s.
    pub(crate) fn settings_tick_period(&self) -> Duration {
        let regions_pending = self.bar_regions_busy
            || self.bar_regions_retry.is_some()
            || (self.bar_ready
                && self.metrics.islands.is_some()
                && self.bar_epoch != 0
                && self.bar_geometry.as_ref().is_some_and(|(serial, _)| {
                    self.bar_regions_sent != Some((self.bar_epoch, *serial))
                }));
        if self.settings_watch.is_none() || regions_pending {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(10)
        }
    }

    pub(crate) fn publish_bar_regions(&mut self) -> Task<Message> {
        if !self.bar_ready
            || self.bar_busy
            || self.bar_regions_busy
            || self.metrics.islands.is_none()
        {
            return Task::none();
        }
        if self.bar_regions_retry.is_some_and(|at| Instant::now() < at) {
            return Task::none();
        }
        let epoch = self.bar_epoch;
        if epoch == 0 {
            return Task::none();
        }
        let Some((serial, geometry)) = self.bar_geometry.clone() else {
            return Task::none();
        };
        if self.bar_regions_sent == Some((epoch, serial)) {
            return Task::none();
        }
        self.bar_regions_busy = true;
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.set_bar_regions(epoch, serial, &geometry).await },
            move |result| Message::BarRegionsApplied(epoch, serial, result),
        )
    }

    pub(crate) fn update_bar_preferences(&mut self) -> Task<Message> {
        if self.bar_busy {
            return Task::none();
        }
        if !self.bar_ready
            || self.bar_output_stale
            || self.bar != self.desired_settings.bar
            || self.metrics != self.desired_settings.effective_metrics()
        {
            self.apply_bar()
        } else {
            Task::none()
        }
    }

    pub(crate) fn control_content(&self) -> enzui_shell::compositor::ControlContent {
        enzui_shell::compositor::ControlContent {
            grid: self.visible_controls(),
            measured: self.control_body.map(|(_, size)| size),
        }
    }

    // The width the Control Center body is laid out at: the panel less its
    // attachment insets and scroll inset, as the compositor sizes it.
    pub(crate) fn control_body_width(&self) -> f32 {
        let radius = f32::from(self.metrics.radius);
        let (left, right) = self.control_attachment.insets(radius);
        self.control_size.width - left - right - enzui_shell::panel_style::INSET * 2.0
    }

    pub(crate) fn control_body_key(&self) -> ControlBodyKey {
        ControlBodyKey {
            generation: self.control_generation,
            width: self.control_size.width,
        }
    }

    pub(crate) fn measure_control_body(&mut self) -> Task<Message> {
        let Some(id) = self.control_center.filter(|_| self.control_visible) else {
            return Task::none();
        };
        if self.control_body_pending.is_some() {
            return Task::none();
        }
        if self
            .control_body_retry
            .is_some_and(|at| Instant::now() < at)
        {
            return Task::none();
        }
        let key = self.control_body_key();
        self.control_body_pending = Some((id, key));
        enzui_shell::widget_bounds::natural_size(id, "control-body-extent")
            .map(move |size| Message::ControlBodyMeasured(id, key, size))
    }

    pub(crate) fn apply_control_geometry(&mut self, geometry: ControlGeometry) -> Task<Message> {
        let new_window = self.popover_window(&geometry);
        let old_size = self.control_size;
        self.control_size = iced::Size::new(geometry.width, geometry.height);
        self.control_attachment = geometry.attachment;
        self.control_output = Some(geometry.output.clone());
        if let Some(id) = self.control_center.filter(|_| {
            (old_size.width - geometry.width).abs() > 0.75
                || (old_size.height - geometry.height).abs() > 0.75
        }) {
            resize_chrome(id, new_window)
        } else {
            Task::none()
        }
    }

    pub(crate) fn reflow_controls(&mut self) -> Task<Message> {
        if self.control_opening || (self.control_center.is_some() && !self.control_started) {
            self.control_reflow_pending = true;
            return Task::none();
        }
        if self.control_center.is_none() || self.control_deferred_reflow {
            return Task::none();
        }
        if self.control_placing {
            self.control_reflow_pending = true;
            return Task::none();
        }
        let Some(anchor) = self.control_anchor else {
            return Task::none();
        };
        self.control_placing = true;
        let compositor = self.compositor.clone();
        let edge = self.bar;
        let (left, right) = self.desktop_edges();
        let metrics = self.metrics;
        let generation = self.control_generation;
        let controls = self.control_content();
        Task::perform(
            async move {
                compositor
                    .prepare_control_center(edge, left, right, metrics, anchor, controls)
                    .await
            },
            move |result| Message::ControlsReflowed(generation, result),
        )
    }
}
