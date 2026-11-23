use super::{Message, Shell, clock, window_settings};
use enzui_shell::i18n::tr;
use enzui_shell::{settings::Change as SettingChange, system_controls::Action};
use iced::{Task, window};
use std::time::{Duration, Instant};

/// How long the chrome may stay hidden for an announced scale change, should
/// Settings never say it finished or the output never settle.
const RESCALE_LIMIT: Duration = Duration::from_secs(6);
/// How long, after Settings applied a scale, the compositor may take to pin
/// it as monitor rules before the bar is placed anyway.
const RESCALE_SETTLE_LIMIT: Duration = Duration::from_secs(3);
/// How often the output is checked while the compositor settles it.
const RESCALE_POLL: Duration = Duration::from_millis(50);

/// A display scale change announced by Settings. The chrome is hidden before
/// the output changes; the compositor resizes the output, then pins it as a
/// monitor rule about 400 ms later, which moves windows on it again. The bar
/// comes back once it has been placed for the settled output.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RescaleHold {
    pub(crate) generation: u64,
    /// When Settings said the change was applied (or failed).
    pub(crate) applied: Option<Instant>,
    /// The output has settled and the bar is being placed for it.
    pub(crate) placing: bool,
}

impl Shell {
    pub(crate) fn update_chrome(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleOverview | Message::ToggleControls | Message::ToggleAccessibility
                if self.bar_busy =>
            {
                Task::none()
            }
            Message::SettingsTick => {
                let expired = self.clipboard_expire();
                let search_expired = self.search.clipboard.expire();
                if search_expired {
                    self.search_recompute();
                }
                self.quiet = !expired && !search_expired;
                // With a folder watch, the poll is only a slow safety net
                // for a change the watch missed.
                let due = self.settings_watch.is_none()
                    || self
                        .settings_read_at
                        .is_none_or(|at| at.elapsed() >= Duration::from_secs(30));
                let settings = if !due || self.settings_read_busy || self.settings_save_busy {
                    Task::none()
                } else {
                    self.read_settings()
                };
                // Outputs added or removed are checked on the compositor's
                // events (DesktopChanged); this is the safety net.
                let outputs = if self
                    .outputs_checked
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
                {
                    self.check_outputs()
                } else {
                    Task::none()
                };
                Task::batch([settings, self.publish_bar_regions(), outputs])
            }
            Message::FrameStopped(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: {error}");
                }
                // The frame helper ends with its output.
                self.frame_lost = true;
                self.check_outputs()
            }
            Message::OutputsChecked(change) => {
                use enzui_shell::compositor::OutputChange;
                match change {
                    OutputChange::Moved => enzui_shell::compositor::restart_for_new_output(),
                    OutputChange::Same => {
                        self.quiet = true;
                        Task::none()
                    }
                    // A new scale on the same output keeps the shell, so the
                    // chrome stays on screen: restarting hid the whole bar for
                    // about two seconds. The bar and sidebar are placed again
                    // at the new logical size. A bar being placed already, or
                    // a style change under way, places it once done (it may
                    // have read the old scale), and so does the first
                    // placement in the saved style.
                    // An announced change places the bar once the compositor
                    // has settled the output (see `rescale_checked`).
                    OutputChange::Rescaled => {
                        self.quiet = false;
                        self.bar_output_stale = true;
                        if self.chrome_styled
                            && !self.bar_style_fade
                            && self.rescale_hold.is_none_or(|hold| hold.placing)
                        {
                            self.update_bar_preferences()
                        } else {
                            Task::none()
                        }
                    }
                }
            }
            Message::RescaleHidden(generation) => {
                if self
                    .rescale_hold
                    .is_some_and(|hold| hold.generation == generation)
                {
                    let compositor = self.compositor.clone();
                    Task::batch([
                        Task::future(async move { compositor.chrome_hidden_for_rescale().await })
                            .discard(),
                        Task::done(Message::RescaleFrames(generation)),
                    ])
                } else {
                    Task::none()
                }
            }
            // The compositor applies an output change on its next frame, and
            // with the chrome hidden nothing may be drawing: keep frames
            // coming until Settings says the change was applied.
            Message::RescaleFrames(generation) => {
                if !self
                    .rescale_hold
                    .is_some_and(|hold| hold.generation == generation && hold.applied.is_none())
                {
                    return Task::none();
                }
                let compositor = self.compositor.clone();
                Task::batch([
                    Task::future(async move {
                        let _ = compositor.wake_sidebar_rail(100).await;
                    })
                    .discard(),
                    Task::perform(tokio::time::sleep(RESCALE_POLL), move |()| {
                        Message::RescaleFrames(generation)
                    }),
                ])
            }
            Message::RescaleSettle(generation) => {
                if !self
                    .rescale_hold
                    .is_some_and(|hold| hold.generation == generation)
                {
                    return Task::none();
                }
                let compositor = self.compositor.clone();
                let frame_lost = self.frame_lost;
                Task::perform(
                    async move {
                        let change = compositor.primary_change(frame_lost).await;
                        (change, compositor.outputs_settled().await)
                    },
                    move |(change, settled)| Message::RescaleChecked(generation, change, settled),
                )
            }
            Message::RescaleChecked(generation, change, settled) => {
                self.rescale_checked(generation, change, settled)
            }
            Message::RescaleLimit(generation) => {
                if self
                    .rescale_hold
                    .is_some_and(|hold| hold.generation == generation)
                {
                    self.bar_output_stale = true;
                    let place = if self.chrome_styled && !self.bar_style_fade {
                        self.update_bar_preferences()
                    } else {
                        Task::none()
                    };
                    Task::batch([self.release_rescale_hold(), place])
                } else {
                    Task::none()
                }
            }
            Message::DesktopChanged => {
                // Only starts reads: what they find changes what is shown.
                self.quiet = true;
                self.desktop_dirty = true;
                self.invalidate_previews();
                let desktops = if self.desktop_busy {
                    Task::none()
                } else {
                    self.refresh_desktops(None)
                };
                let snapshot = if self.busy {
                    self.snapshot_dirty = true;
                    Task::none()
                } else {
                    self.refresh(None)
                };
                Task::batch([
                    desktops,
                    snapshot,
                    self.refresh_sidebar(None),
                    self.check_outputs(),
                ])
            }
            Message::SidebarScroll(offset) => {
                self.sidebar_scroll = offset;
                Task::none()
            }
            Message::DesktopsRead(generation, _) if generation != self.desktop_generation => {
                Task::none()
            }
            Message::DesktopsRead(_, result) => {
                self.desktop_busy = false;
                self.quiet = result.as_ref().ok() == self.desktops.as_ref()
                    && self.desktop_steps == 0
                    && !self.desktop_dirty
                    && !self.overview_opening;
                let mut spread = Task::none();
                match result {
                    Ok(desktops) => {
                        if self.desktops.as_ref().map(|d| &d.entries) != Some(&desktops.entries) {
                            self.invalidate_previews();
                        }
                        // Another desktop came up under the spread: the
                        // compositor has shown its windows again.
                        if self
                            .spread_desktop()
                            .is_some_and(|desktop| i64::from(desktops.active) != desktop)
                        {
                            self.quiet = false;
                            spread = self.abort_spread();
                        }
                        self.desktops = Some(desktops);
                    }
                    Err(error) => eprintln!("enzui-shell desktops: {error}"),
                }
                let refresh = if self.desktop_steps != 0 {
                    let step = self.desktop_steps.signum();
                    self.desktop_steps -= step;
                    self.refresh_desktops(Some(step))
                } else if self.desktop_dirty {
                    self.refresh_desktops(None)
                } else {
                    Task::none()
                };
                let previews = if self.preview_dirty && !self.preview_busy {
                    self.start_previews()
                } else {
                    Task::none()
                };
                let open = self.open_ready_overview();
                Task::batch([refresh, previews, open, spread])
            }
            Message::DesktopWheel(delta) => {
                if let Some(step) = self.desktop_wheel.step(delta, Instant::now()) {
                    if self.desktop_busy {
                        self.desktop_steps = (self.desktop_steps + step).clamp(-8, 8);
                        Task::none()
                    } else {
                        self.refresh_desktops(Some(step))
                    }
                } else {
                    Task::none()
                }
            }
            Message::DismissRegions => {
                self.cancel_panel_action();
                Task::batch([
                    self.close_search(),
                    self.close_overview(),
                    self.close_controls(),
                    self.close_plugin(),
                    self.close_notifications(),
                ])
            }
            Message::OverviewHidden(id) => {
                if self.overview == Some(id) && !self.overview_visible {
                    self.force_close_overview()
                } else {
                    Task::none()
                }
            }
            Message::ControlsHidden(id) => {
                if self.control_center == Some(id) && !self.control_visible {
                    self.park_controls()
                } else {
                    Task::none()
                }
            }
            Message::ToggleOverview => {
                if let Some(id) = self.overview {
                    if self.overview_visible
                        || (self.metrics.islands.is_some()
                            && !self.surface_inputs.contains_key(&id))
                    {
                        self.close_overview()
                    } else {
                        self.overview_visible = true;
                        let compositor = self.compositor.clone();
                        Task::perform(
                            async move { compositor.focus_overview().await },
                            Message::FocusReturned,
                        )
                    }
                } else if self.overview_opening {
                    self.force_close_overview()
                } else {
                    self.overview_opening = true;
                    self.overview_generation = self.overview_generation.wrapping_add(1);
                    let generation = self.overview_generation;
                    // Window contents can change without compositor events.
                    self.preview_dirty = true;
                    let previews = if self.preview_cache_current() {
                        self.defer_previews_to_reveal();
                        Task::none()
                    } else {
                        self.start_previews()
                    };
                    Task::batch([
                        previews,
                        self.close_controls_for_panel(),
                        self.force_close_launcher(),
                        enzui_shell::widget_bounds::bounds(self.panel, "desktop-overview")
                            .map(move |rect| Message::OverviewAnchor(generation, rect)),
                    ])
                }
            }
            Message::OverviewAnchor(generation, anchor) => {
                if !self.overview_opening || generation != self.overview_generation {
                    Task::none()
                } else {
                    let edge = self.bar;
                    let metrics = self.metrics;
                    let compositor = self.compositor.clone();
                    // Wide enough for its pictures: each takes 252 with its gap, the
                    // trailing "new desktop" action less than half of that.
                    let (desktops, spare) = self.desktops.as_ref().map_or((2, false), |d| {
                        (
                            d.entries.len().max(1),
                            d.entries.len() > 1
                                && d.entries.last().is_some_and(|last| last.windows == 0),
                        )
                    });
                    let full = desktops - usize::from(spare);
                    let width =
                        (full as f32 * 252.0 + if spare { 122.0 } else { 0.0 } + 20.0).min(776.0);
                    Task::perform(
                        async move {
                            compositor
                                .prepare_overview(edge, metrics, anchor, width)
                                .await
                        },
                        move |result| Message::OverviewPrepared(generation, result),
                    )
                }
            }
            Message::PreviewsDue(wait) => {
                self.quiet = true;
                if wait == self.preview_wait
                    && self.preview_dirty
                    && !self.preview_busy
                    && !self.desktop_busy
                    && !self.desktop_dirty
                {
                    self.take_previews()
                } else {
                    Task::none()
                }
            }
            Message::OverviewFrame(id) => {
                self.quiet = true;
                if self.overview == Some(id) && self.preview_after_reveal {
                    self.overview_frame_at = Some(Instant::now());
                }
                Task::none()
            }
            Message::OverviewRevealCheck(generation) => {
                self.quiet = true;
                self.check_overview_reveal(generation)
            }
            Message::PreviewsReady(generation, _, _) if generation != self.preview_generation => {
                Task::none()
            }
            Message::PreviewsReady(_, revision, result) => {
                self.preview_busy = false;
                self.quiet = !self.overview_opening;
                if revision != self.preview_revision {
                    self.preview_dirty = true;
                } else {
                    self.preview_cached_revision = Some(revision);
                    match result {
                        Ok(previews) => {
                            // An open overview of the same desktops takes the
                            // fresh copies in place.
                            if self.overview.is_some() && self.overview_desktops == self.desktops {
                                self.overview_previews = previews.clone();
                                self.quiet = false;
                            }
                            self.previews = previews;
                            self.preview_failed = false;
                        }
                        Err(error) => {
                            self.previews.clear();
                            self.preview_failed = true;
                            eprintln!("enzui-shell preview: {error}");
                        }
                    }
                }
                let open = self.open_ready_overview();
                let refresh = if self.preview_dirty {
                    self.start_previews()
                } else {
                    Task::none()
                };
                Task::batch([open, refresh])
            }
            Message::OverviewPrepared(generation, result) => {
                if !self.overview_opening || generation != self.overview_generation {
                    Task::none()
                } else {
                    match result {
                        Ok(geometry) => {
                            self.overview_prepared = Some(geometry);
                            let open = self.open_ready_overview();
                            let refresh = if self.overview_opening && !self.preview_busy {
                                self.start_previews()
                            } else {
                                Task::none()
                            };
                            Task::batch([open, refresh])
                        }
                        Err(error) => {
                            self.overview_opening = false;
                            eprintln!("enzui-shell overview: {error}");
                            Task::none()
                        }
                    }
                }
            }
            Message::SelectDesktop(id) => {
                self.desktop_steps = 0;
                self.desktop_wheel = Default::default();
                let compositor = self.compositor.clone();
                self.desktop_busy = true;
                self.desktop_generation = self.desktop_generation.wrapping_add(1);
                let generation = self.desktop_generation;
                Task::batch([
                    self.close_overview(),
                    Task::perform(
                        async move { compositor.desktop_focus(id).await },
                        move |result| Message::DesktopsRead(generation, result),
                    ),
                ])
            }
            Message::SharingChanged => {
                let sharing = Self::read_sharing_grant();
                self.quiet = sharing == self.remote_sharing;
                self.remote_sharing = sharing;
                Task::none()
            }
            Message::Tick => {
                // Its reads answer as their own messages; the tick itself
                // shows only a new minute, a countdown or the calendar.
                self.next_tick_at = Instant::now() + self.idle_tick_period();
                let clock_text = clock(&self.desired_settings.session);
                let sharing = if self.sharing_watch.is_some() {
                    self.remote_sharing
                } else {
                    Self::read_sharing_grant()
                };
                let sharing_changed = sharing != self.remote_sharing;
                self.remote_sharing = sharing;
                let others_unchanged = !sharing_changed
                    && self.confirm_deadline.is_none()
                    && !self.notification.center_open();
                self.quiet = others_unchanged && clock_text == self.clock;
                // A new minute alone shows only in the bar.
                if others_unchanged && clock_text != self.clock {
                    self.redraw_only = Some(self.panel);
                }
                self.clock = clock_text;
                self.sync_night_light();
                let countdown = match &self.confirm_deadline {
                    Some((action, at))
                        if Instant::now() >= *at
                            && self.confirm.as_ref() == Some(action)
                            && self.confirm_inhibitors.as_ref().is_some_and(Vec::is_empty) =>
                    {
                        let action = action.clone();
                        self.confirm_deadline = None;
                        Task::done(Message::SystemAction(action))
                    }
                    _ => Task::none(),
                };
                let calendar = if self.notification.center_open() {
                    self.clock_tick()
                } else {
                    Task::none()
                };
                let now = Instant::now();
                // Every 30 s, on an installed login: crashes since the last look.
                let crashes = if self.schedule.crashes.due(now)
                    && enzui_shell::desktop_session::mode()
                        == enzui_shell::desktop_session::Mode::Installed
                {
                    Task::perform(enzui_shell::crash_reports::read(), Message::Crashes)
                } else {
                    Task::none()
                };
                // The system's state, every ten seconds.
                let system_due = self.schedule.system.due(now);
                let session_apps = self.session_apps_tick(now);
                let mirror = self.mirror_tick(system_due);
                let screen_time = self.screen_time_tick(now);
                // Parental controls a guardian changes apply within a minute.
                let parental = if self.schedule.parental.due(now)
                    && enzui_shell::desktop_session::mode()
                        == enzui_shell::desktop_session::Mode::Installed
                {
                    Task::perform(enzui_shell::parental::own_filter(), Message::ParentalFilter)
                } else {
                    Task::none()
                };
                // Desktops, the sidebar and the active workspace change with
                // the compositor's events (DesktopChanged); polling them is a
                // safety net every thirty seconds, and every tick only while a
                // panel showing them is open. Previews are taken on those
                // events and when the overview opens, never on a timer: each
                // copies every window.
                let poll = self.schedule.desktops.due(now)
                    || self.control_visible
                    || self.overview.is_some()
                    || self.overview_opening;
                let desktop = if self.busy || !poll {
                    Task::none()
                } else {
                    self.refresh(None)
                };
                let system = if !self.system_busy && (self.control_visible || system_due) {
                    self.refresh_system(None)
                } else {
                    Task::none()
                };
                let desktops = if self.desktop_busy || !poll {
                    Task::none()
                } else {
                    self.refresh_desktops(None)
                };
                let sidebar = if poll {
                    self.refresh_sidebar(None)
                } else {
                    Task::none()
                };
                let shortcut = if self.setup_errors.shortcut.is_some() {
                    self.settings_shortcut()
                } else {
                    Task::none()
                };
                Task::batch([
                    desktop,
                    system,
                    desktops,
                    session_apps,
                    mirror,
                    parental,
                    screen_time,
                    sidebar,
                    shortcut,
                    calendar,
                    countdown,
                    crashes,
                ])
            }
            Message::ShowLauncher if self.overview.is_some() || self.overview_opening => {
                Task::batch([
                    self.force_close_overview(),
                    Task::done(Message::ShowLauncher),
                ])
            }
            Message::ShowLauncher if self.control_visible || self.control_requested => {
                if let Some(id) = self.control_center {
                    self.queue_panel_action(id, Message::ShowLauncher)
                } else {
                    self.control_requested = false;
                    Task::done(Message::ShowLauncher)
                }
            }
            Message::ShowLauncher
                if self.search.configured
                    && self
                        .search
                        .preferences
                        .shows(enzui_shell::search::Category::Apps) =>
            {
                self.search_trigger()
            }
            Message::ShowLauncher => {
                if self.bar_busy {
                    Task::none()
                } else if let Some(id) = self.launcher {
                    if self.launcher_geometry.is_some() {
                        if self.launcher_visible || !self.launcher_started {
                            self.close_launcher()
                        } else {
                            self.launcher_visible = true;
                            let compositor = self.compositor.clone();
                            let revision = self.launcher_focus_revision;
                            Task::perform(
                                async move { compositor.focus_launcher().await },
                                move |result| Message::LauncherFocused(id, revision, result),
                            )
                        }
                    } else {
                        window::gain_focus(id)
                    }
                } else if self.launcher_opening {
                    self.force_close_launcher()
                } else {
                    self.launcher_placing = false;
                    self.query.clear();
                    self.error = None;
                    self.launcher_opening = true;
                    self.launcher_generation = self.launcher_generation.wrapping_add(1);
                    let generation = self.launcher_generation;
                    Task::batch([
                        self.close_controls_for_panel(),
                        self.force_close_overview(),
                        enzui_shell::widget_bounds::bounds(self.panel, "applications")
                            .map(move |rect| Message::LauncherAnchor(generation, rect)),
                    ])
                }
            }
            Message::LauncherAnchor(generation, anchor) => {
                if !self.launcher_opening
                    || generation != self.launcher_generation
                    || self.launcher_placing
                {
                    Task::none()
                } else {
                    self.launcher_placing = true;
                    let placement = enzui_shell::compositor::PopupPlacement {
                        bar: self.bar,
                        left: 0,
                        right: 0,
                        metrics: self.metrics,
                        anchor,
                        body: iced::Size::new(640.0, 500.0),
                    };
                    let compositor = self.compositor.clone();
                    Task::perform(
                        async move { compositor.prepare_launcher(placement).await },
                        move |result| Message::LauncherPrepared(generation, result),
                    )
                }
            }
            Message::LauncherPrepared(generation, result) => {
                if generation != self.launcher_generation {
                    return Task::none();
                }
                self.launcher_placing = false;
                if !self.launcher_opening {
                    Task::none()
                } else {
                    self.launcher_opening = false;
                    match result {
                        Ok(geometry) => {
                            let size = if matches!(
                                geometry.attachment,
                                enzui_shell::reveal::Attachment::Island { .. }
                            ) {
                                self.popover_window(&geometry)
                            } else {
                                iced::Size::new(geometry.width, geometry.height)
                            };
                            let (id, open) = window::open(window_settings(
                                "dev.enzui.Launcher",
                                size.width,
                                size.height,
                            ));
                            self.launcher_geometry = matches!(
                                geometry.attachment,
                                enzui_shell::reveal::Attachment::Island { .. }
                            )
                            .then_some(geometry);
                            self.launcher = Some(id);
                            self.launcher_visible = false;
                            self.launcher_started = false;
                            open.map(Message::Opened)
                        }
                        Err(error) => {
                            self.notice = Some(error);
                            Task::none()
                        }
                    }
                }
            }
            Message::LauncherFocused(id, revision, result) => {
                if self.launcher == Some(id)
                    && revision == self.launcher_focus_revision
                    && self.launcher_visible
                    && let Err(error) = result
                {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::LauncherFocusChecked(id, revision, retained) => {
                if self.launcher == Some(id)
                    && revision == self.launcher_focus_revision
                    && self.launcher_visible
                    && !self.launching
                    && !retained
                {
                    self.close_launcher()
                } else {
                    Task::none()
                }
            }
            Message::LauncherHidden(id) => {
                if self.launcher == Some(id) && !self.launcher_visible {
                    self.finish_launcher()
                } else {
                    Task::none()
                }
            }
            Message::Search(query) => {
                self.query = query;
                Task::none()
            }
            Message::Launch(id) => {
                if self.launching {
                    Task::none()
                } else {
                    self.launching = true;
                    let compositor = self.compositor.clone();
                    let root = self.root.clone();
                    let serial = self.panel_action_serial;
                    let owner = self.launcher;
                    self.launch_inflight = Some((serial, owner));
                    Task::perform(
                        async move { compositor.launch_app(&root, id).await },
                        move |result| Message::Launched(serial, owner, result),
                    )
                }
            }
            Message::Launched(serial, owner, result) => {
                if self.launch_inflight != Some((serial, owner)) {
                    Task::none()
                } else {
                    self.launch_inflight = None;
                    self.launching = false;
                    if serial != self.panel_action_serial || owner != self.launcher {
                        Task::none()
                    } else {
                        match result {
                            Ok(()) => Task::batch([
                                self.close_launcher(),
                                self.close_controls(),
                                self.force_close_overview(),
                            ]),
                            Err(error) => {
                                self.error = Some(error);
                                Task::none()
                            }
                        }
                    }
                }
            }
            Message::ToggleTheme => {
                self.notice = None;
                self.light = !self.light;
                self.update_frame();
                self.save_setting(SettingChange::Light(self.light))
            }
            Message::DragMaximizeReady(result) => {
                self.drag_maximize_busy = false;
                match result {
                    Ok(()) => {
                        self.drag_maximize_ready = true;
                        self.drag_maximize_retry_at = None;
                    }
                    Err(error) => {
                        eprintln!("drag-to-maximize: {error}");
                        self.drag_maximize_ready = false;
                        self.drag_maximize_retry_at = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
                Task::none()
            }
            Message::ToggleControls if self.overview.is_some() || self.overview_opening => {
                Task::batch([
                    self.force_close_overview(),
                    Task::done(Message::ToggleControls),
                ])
            }
            message @ (Message::ToggleControls | Message::ToggleAccessibility) => {
                let accessibility = matches!(message, Message::ToggleAccessibility);
                let content_changed = self.accessibility_menu != accessibility;
                self.accessibility_menu = accessibility;
                if accessibility {
                    self.screen_reader = crate::shell_accessibility::screen_reader();
                }
                if self.control_opening
                    || self.control_center.is_some_and(|_| !self.control_started)
                {
                    self.control_requested = !self.control_requested;
                    Task::none()
                } else if self.control_center.is_some() {
                    if self.control_visible {
                        self.close_controls()
                    } else {
                        self.control_requested = true;
                        if content_changed {
                            return self.reflow_controls();
                        }
                        self.control_visible = true;
                        self.control_frames = 2;
                        let compositor = self.compositor.clone();
                        Task::perform(
                            async move { compositor.prepare_control_center_frame().await },
                            Message::FocusReturned,
                        )
                    }
                } else {
                    self.control_requested = true;
                    Task::batch([self.force_close_launcher(), self.warm_controls()])
                }
            }

            Message::WarmControls => self.warm_controls(),
            Message::ControlsFrame(id) => {
                if self.control_center != Some(id)
                    || self.control_frames == 0
                    || !self.surface_inputs.contains_key(&id)
                {
                    self.quiet = true;
                    return Task::none();
                }
                self.control_frames -= 1;
                if self.control_frames != 0 {
                    let compositor = self.compositor.clone();
                    return Task::batch([
                        iced::window::invalidate(iced::window::Invalidation::only(id)),
                        Task::perform(
                            async move { compositor.prepare_control_center_frame().await },
                            Message::FocusReturned,
                        ),
                    ]);
                }
                if self.control_warming {
                    Task::done(Message::ControlsWarmed(id))
                } else if !self.control_started {
                    Task::done(Message::ControlsWarmHidden(id))
                } else if self.control_visible && self.control_requested {
                    let compositor = self.compositor.clone();
                    Task::perform(
                        async move { compositor.focus_control_center().await },
                        Message::FocusReturned,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ControlsWarmed(id) => {
                if self.control_center != Some(id) || !self.control_warming {
                    return Task::none();
                }
                enzui_shell::widget_bounds::natural_size(id, "control-body-extent")
                    .map(move |size| Message::ControlsWarmMeasured(id, size))
            }
            Message::ControlsWarmMeasured(id, size) => {
                if self.control_center != Some(id) || !self.control_warming {
                    return Task::none();
                }
                if let Some(size) = size.filter(|size| {
                    size.width.is_finite()
                        && size.height.is_finite()
                        && (size.width - self.control_body_width()).abs() <= 0.75
                        && (1.0..=65536.0).contains(&size.height)
                }) {
                    self.control_body = Some((self.control_body_key(), size));
                    self.control_reflow_pending = true;
                }
                self.control_warming = false;
                // Park only after the empty resting frame has been drawn, so
                // preparing the renderer cannot expose the full warm-up body.
                self.control_frames = 1;
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.prepare_control_center_frame().await },
                    Message::FocusReturned,
                )
            }
            Message::ControlsWarmHidden(id) => {
                if self.control_center != Some(id) || self.control_started {
                    return Task::none();
                }
                self.controls_ready()
            }
            Message::ControlsAnchor(generation, anchor) => {
                if !self.control_opening || generation != self.control_generation {
                    Task::none()
                } else if let Some(anchor) = anchor {
                    self.control_anchor = Some(anchor);
                    self.control_placing = true;
                    let compositor = self.compositor.clone();
                    let edge = self.bar;
                    let (left, right) = self.desktop_edges();
                    let metrics = self.metrics;
                    let controls = self.control_content();
                    Task::perform(
                        async move {
                            compositor
                                .prepare_control_center(
                                    edge, left, right, metrics, anchor, controls,
                                )
                                .await
                        },
                        move |result| Message::ControlsPrepared(generation, result),
                    )
                } else {
                    self.control_opening = false;
                    if self.control_requested {
                        self.notice =
                            Some(tr("shell-the-control-center-trigger-is-unavailable").into());
                    }
                    Task::none()
                }
            }
            Message::ControlsPrepared(generation, result) => {
                if generation != self.control_generation {
                    return Task::none();
                }
                self.control_placing = false;
                if !self.control_opening || generation != self.control_generation {
                    Task::none()
                } else if std::mem::take(&mut self.control_reflow_pending) {
                    enzui_shell::widget_bounds::bounds(self.panel, self.control_trigger())
                        .map(move |rect| Message::ControlsAnchor(generation, rect))
                } else {
                    self.control_opening = false;
                    match result {
                        Ok(geometry) => {
                            self.control_size = iced::Size::new(geometry.width, geometry.height);
                            self.control_attachment = geometry.attachment;
                            self.control_output = Some(geometry.output.clone());
                            let size = self.popover_window(&geometry);
                            let mut settings =
                                window_settings("dev.enzui.ControlCenter", size.width, size.height);
                            settings.resizable = true;
                            let (id, open) = window::open(settings);
                            self.control_center = Some(id);
                            self.control_visible = false;
                            self.control_started = false;
                            self.control_warming = true;
                            self.control_frames = 2;
                            open.map(Message::Opened)
                        }
                        Err(error) => {
                            if self.control_requested {
                                self.notice = Some(error);
                            }
                            Task::none()
                        }
                    }
                }
            }
            Message::ControlsReflowed(generation, result) => {
                if generation != self.control_generation || self.control_center.is_none() {
                    return Task::none();
                }
                self.control_placing = false;
                let resize = match result {
                    Ok(geometry) => self.apply_control_geometry(geometry),
                    Err(error) => {
                        self.notice = Some(error);
                        self.control_details_open = false;
                        Task::none()
                    }
                };
                let next = if std::mem::take(&mut self.control_reflow_pending) {
                    self.reflow_controls()
                } else {
                    Task::none()
                };
                let show =
                    if self.control_requested && !self.control_visible && self.control_started {
                        self.control_visible = true;
                        self.control_frames = 2;
                        let compositor = self.compositor.clone();
                        Task::perform(
                            async move { compositor.prepare_control_center_frame().await },
                            Message::FocusReturned,
                        )
                    } else {
                        Task::none()
                    };
                resize.chain(next).chain(show)
            }
            Message::ControlBodyMeasured(id, key, size) => {
                if self
                    .control_body_pending
                    .as_ref()
                    .is_none_or(|(pending_id, pending_key)| {
                        *pending_id != id || *pending_key != key
                    })
                {
                    Task::none()
                } else {
                    self.control_body_pending = None;
                    let settling = self
                        .control_detail_since
                        .map(|at| Duration::from_millis(700).saturating_sub(at.elapsed()))
                        .filter(|left| !left.is_zero());
                    if self.control_center != Some(id) || !self.control_visible {
                        Task::none()
                    } else if self.control_deferred_reflow
                        || settling.is_some()
                        || self.control_body_key() != key
                    {
                        control_body_retry(settling.unwrap_or(Duration::from_millis(50)))
                    } else if let Some(size) = size.filter(|size| {
                        size.width.is_finite()
                            && size.height.is_finite()
                            && (size.width - self.control_body_width()).abs() <= 0.75
                            && (1.0..=65536.0).contains(&size.height)
                    }) {
                        self.control_body = Some((key, size));
                        self.control_body_retry = None;
                        let radius = f32::from(self.metrics.radius);
                        let preferred = self.control_content().preferred_height(
                            size.width,
                            enzui_shell::control_layout::row_height(f32::from(self.metrics.text)),
                            enzui_shell::panel_style::INSET,
                            self.control_attachment.join(radius),
                        );
                        if (preferred - self.control_size.height).abs() > 0.75 {
                            self.reflow_controls()
                        } else {
                            Task::none()
                        }
                    } else {
                        self.control_body_retry = Some(Instant::now() + Duration::from_millis(50));
                        control_body_retry(Duration::from_millis(50))
                    }
                }
            }
            Message::Volume(value) => {
                if !self.control_placing && value.is_finite() && self.system.audio.is_some() {
                    let value = value.round().clamp(0.0, 100.0) as u8;
                    self.volume_draft = Some(value);
                    self.enqueue_system(Action::Volume(value))
                } else {
                    Task::none()
                }
            }
            Message::Brightness(value) => {
                if !self.control_placing && value.is_finite() && self.system.brightness.is_some() {
                    let value = value.round().clamp(5.0, 100.0) as u8;
                    self.brightness_draft = Some(value);
                    self.enqueue_system(Action::Brightness(value))
                } else {
                    Task::none()
                }
            }
            Message::RefreshAudioDevices => {
                if !self.control_details_open
                    || self.control_details != Some(enzui_shell::settings::QuickAction::Volume)
                {
                    return Task::none();
                }
                let controls = self.controls.clone();
                self.audio_request = self.audio_request.wrapping_add(1);
                let epoch = self.audio_request;
                self.audio_devices = None;
                Task::perform(
                    async move { controls.audio_devices().await },
                    move |result| Message::AudioDevices(epoch, result),
                )
            }
            Message::AudioDevices(epoch, result) => {
                if epoch == self.audio_request
                    && self.control_details_open
                    && self.control_details == Some(enzui_shell::settings::QuickAction::Volume)
                {
                    self.audio_devices = Some(result);
                }
                Task::none()
            }
            Message::IdleInhibited(result) => {
                self.idle_inhibition_ready = true;
                match result {
                    Ok(active) => self.idle_inhibited = active,
                    Err(error) => {
                        self.idle_inhibited = false;
                        eprintln!("enzui-shell: idle inhibition unavailable: {error}");
                    }
                }
                self.quiet = true;
                Task::none()
            }
            Message::Idle(config, notice) if config != self.idle_config() || !notice.current() => {
                Task::none()
            }
            Message::Idle(
                _,
                enzui_shell::idle::Notice {
                    event: enzui_shell::idle::Event::Lock,
                    ..
                },
            ) if !self.idle_inhibited => self.enqueue_system(Action::Lock),
            Message::Idle(
                _,
                enzui_shell::idle::Notice {
                    event: enzui_shell::idle::Event::Lock,
                    ..
                },
            ) => Task::none(),
            Message::Idle(
                _,
                enzui_shell::idle::Notice {
                    event: enzui_shell::idle::Event::Suspend,
                    ..
                },
            ) => {
                if self.controls.allow_power && !self.idle_inhibited {
                    self.enqueue_system(Action::Suspend)
                } else {
                    Task::none()
                }
            }
            Message::Idle(
                _,
                enzui_shell::idle::Notice {
                    event: enzui_shell::idle::Event::Awake,
                    ..
                },
            ) => Task::none(),
            Message::Idle(
                _,
                enzui_shell::idle::Notice {
                    event: enzui_shell::idle::Event::Unavailable(error),
                    ..
                },
            ) => {
                eprintln!("enzui-shell: idle policy unavailable: {error}");
                Task::none()
            }
            Message::PowerPolicy(error) => {
                eprintln!("enzui-shell: sleep and lid policy unavailable: {error}");
                Task::none()
            }
            Message::SystemAction(action) => {
                if matches!(action, Action::Wifi(_) | Action::Bluetooth(_))
                    && (!self.system.system_writes || self.control_connections.busy())
                {
                    return Task::none();
                }
                if self.control_placing && self.control_visible {
                    return Task::none();
                }
                self.confirm = None;
                self.power_menu = false;
                // The lock screen captures the desktop, so it should not show this panel.
                if action == Action::Lock
                    && let Some(id) = self
                        .control_center
                        .filter(|_| self.control_visible || self.control_requested)
                {
                    return self.queue_panel_action(id, Message::SystemAction(Action::Lock));
                }
                let leaving = matches!(
                    action,
                    Action::LogOut
                        | Action::PowerOff
                        | Action::Restart
                        | Action::PowerOffAnyway
                        | Action::RestartAnyway
                );
                let task = self.enqueue_system(action);
                if leaving
                    && self.desired_settings.restore_apps
                    && enzui_shell::desktop_session::mode()
                        == enzui_shell::desktop_session::Mode::Installed
                {
                    // The open apps as they are now, before the session ends.
                    let compositor = self.compositor.clone();
                    return Task::perform(
                        async move { compositor.remember_session_apps().await },
                        Message::SessionApps,
                    )
                    .chain(task);
                }
                task
            }
            Message::ControlSettings => {
                if let Some(id) = self.control_center.filter(|_| self.control_visible) {
                    self.queue_panel_action(id, Message::SettingsPage("controls"))
                } else {
                    Task::none()
                }
            }
            Message::ControlDetailsSettled(generation, epoch) => {
                if generation == self.control_generation && epoch == self.control_detail_epoch {
                    self.control_detail_since = None;
                }
                if generation == self.control_generation
                    && epoch == self.control_detail_epoch
                    && self.control_deferred_reflow
                {
                    self.control_deferred_reflow = false;
                    self.control_detail_capacity = self
                        .control_details
                        .filter(|_| self.control_details_open)
                        .map_or(0.0, enzui_shell::control_tile::detail_height);
                    self.reflow_controls()
                } else {
                    Task::none()
                }
            }
            Message::ControlDetails(action) => {
                if action.is_some_and(|action| {
                    !self.system.available(action)
                        || !enzui_shell::control_tile::has_disclosure(
                            self.desired_settings.controls.shape(action),
                        )
                }) {
                    return Task::none();
                }
                let before = self.visible_controls();
                self.control_detail_since = Some(Instant::now());
                self.audio_request = self.audio_request.wrapping_add(1);
                self.control_connections.reset();
                self.control_detail_epoch = self.control_detail_epoch.wrapping_add(1);
                self.control_details_open = action.is_some()
                    && !(self.control_details == action && self.control_details_open);
                if action.is_some() {
                    self.control_details = action;
                }
                self.confirm = None;
                self.power_menu = false;
                let next_height = self
                    .control_details
                    .filter(|_| self.control_details_open)
                    .map_or(0.0, enzui_shell::control_tile::detail_height);
                self.control_deferred_reflow =
                    self.controls_need_shrink(&before, &self.visible_controls());
                let devices = if self.control_details_open {
                    match action {
                        Some(enzui_shell::settings::QuickAction::Volume) => {
                            self.audio_devices = None;
                            Task::done(Message::RefreshAudioDevices)
                        }
                        Some(
                            enzui_shell::settings::QuickAction::Network
                            | enzui_shell::settings::QuickAction::Bluetooth,
                        ) => Task::done(Message::ControlConnections(
                            super::control_center::connections::Event::Refresh(false),
                        )),
                        _ => Task::none(),
                    }
                } else {
                    Task::none()
                };
                let reflow = if self.control_deferred_reflow {
                    Task::none()
                } else {
                    self.control_detail_capacity = next_height;
                    self.reflow_controls()
                };
                Task::batch([reflow, devices])
            }
            Message::Confirm(action) => {
                self.control_connections.clear_forms();
                self.notice = None;
                // A power action's confirmation steps back to the power menu.
                self.power_menu &=
                    matches!(action, Action::Restart | Action::PowerOff | Action::LogOut);
                self.confirm = Some(action.clone());
                self.confirm_inhibitors = None;
                self.confirm_deadline = None;
                if !matches!(action, Action::Restart | Action::PowerOff | Action::LogOut) {
                    return Task::none();
                }
                // As on other desktops: it goes ahead in a minute unless
                // cancelled, and never past an app that blocks it.
                self.confirm_deadline =
                    Some((action.clone(), Instant::now() + Duration::from_secs(60)));
                Task::perform(
                    async move { enzui_shell::inhibitors::list().await.unwrap_or_default() },
                    move |locks| Message::Inhibitors(action, locks),
                )
            }
            Message::CancelConfirm => {
                if self.confirm.take().is_none() {
                    self.power_menu = false;
                }
                Task::none()
            }
            Message::ShowPower => {
                self.control_connections.clear_forms();
                self.notice = None;
                self.power_menu = !self.power_menu;
                self.confirm = None;
                if self.power_menu && self.system.power {
                    Task::perform(
                        enzui_shell::user_switch::available(),
                        Message::SwitchUserAvailable,
                    )
                } else {
                    Task::none()
                }
            }
            Message::SystemRefreshed(action, result, snapshot) => {
                let devices = if matches!(action, Some(Action::AudioDevice(_))) {
                    Task::done(Message::RefreshAudioDevices)
                } else {
                    Task::none()
                };
                self.system_busy = false;
                self.system_action_busy = false;
                if let Some(Action::Volume(value)) = action
                    && self.volume_draft == Some(value)
                {
                    self.volume_draft = None;
                }
                if let Some(Action::Brightness(value)) = action
                    && self.brightness_draft == Some(value)
                {
                    self.brightness_draft = None;
                }
                let result_ok = result.is_ok();
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                let before = self.visible_controls();
                let mut snapshot = snapshot;
                if self.network_watched && action.is_none() {
                    snapshot.network = self.system.network.clone();
                }
                if self.audio_watched && action.is_none() {
                    snapshot.audio = self.system.audio;
                    snapshot.microphone = self.system.microphone;
                }
                snapshot.network.connectivity = self.system.network.connectivity;
                let unchanged = action.is_none()
                    && result_ok
                    && format!("{:?}", self.system) == format!("{snapshot:?}");
                let radio_changed = match self.control_details {
                    Some(enzui_shell::settings::QuickAction::Network) => {
                        self.system.network.wifi_enabled != snapshot.network.wifi_enabled
                            || matches!(action, Some(Action::Wifi(_)))
                    }
                    Some(enzui_shell::settings::QuickAction::Bluetooth) => {
                        self.system.bluetooth != snapshot.bluetooth
                            || matches!(action, Some(Action::Bluetooth(_)))
                    }
                    _ => false,
                };
                self.system = snapshot;
                let connections = if radio_changed && self.control_details_open {
                    self.control_connections.clear_forms();
                    Task::done(Message::ControlConnections(
                        super::control_center::connections::Event::Refresh(false),
                    ))
                } else {
                    Task::none()
                };
                let warning = self
                    .battery_warnings
                    .step(self.system.battery.as_ref())
                    .map_or_else(Task::none, |warning| {
                        Task::perform(
                            async move {
                                let action = enzui_shell::battery_warnings::critical_action().await;
                                let (summary, body, urgent) =
                                    enzui_shell::battery_warnings::message(warning, action);
                                enzui_shell::battery_warnings::notify(summary, body, urgent).await
                            },
                            Message::BatteryWarned,
                        )
                    });
                if self
                    .control_details
                    .is_some_and(|action| !self.system.available(action))
                {
                    self.control_connections.reset();
                    self.control_details = None;
                    self.control_details_open = false;
                }
                let after = self.visible_controls();
                let reflow = if before.actions != after.actions || before.detail != after.detail {
                    if !self.control_visible {
                        self.control_body = None;
                    }
                    self.control_detail_epoch = self.control_detail_epoch.wrapping_add(1);
                    self.control_deferred_reflow = self.controls_need_shrink(&before, &after);
                    if !self.control_deferred_reflow {
                        self.control_detail_capacity =
                            after.detail.map_or(0.0, |(_, height)| height);
                    }
                    self.reflow_controls()
                } else {
                    Task::none()
                };
                self.quiet =
                    unchanged && before.actions == after.actions && before.detail == after.detail;
                let next = if let Some(action) = self.system_queue.pop() {
                    self.refresh_system(Some(action))
                } else {
                    Task::none()
                };
                let notifications = self.notification_preferences();
                Task::batch([reflow, next, devices, connections, warning, notifications])
            }
            Message::Screenshot => {
                let next = Message::CaptureScreenshot;
                if let Some(id) = self
                    .control_center
                    .filter(|_| self.control_visible || self.control_requested)
                {
                    self.queue_panel_action(id, next)
                } else {
                    Task::done(next)
                }
            }
            // The capture interface opens once the Control Center is gone.
            Message::CaptureScreenshot => self.open_capture(),
            Message::Dictate => {
                let next = Message::ToggleDictation;
                if let Some(id) = self
                    .control_center
                    .filter(|_| self.control_visible || self.control_requested)
                {
                    self.queue_panel_action(id, next)
                } else {
                    Task::done(next)
                }
            }
            Message::ToggleDictation => {
                let controls = self.controls.clone();
                Task::future(async move {
                    if let Err(error) = controls.toggle_dictation().await {
                        eprintln!("enzui-shell dictation: {error}");
                    }
                })
                .discard()
            }
            Message::Refreshed(result) => {
                self.busy = false;
                self.quiet =
                    self.requests.is_empty() && result.as_ref().ok() == self.snapshot.as_ref();
                match result {
                    Ok(snapshot) => {
                        self.snapshot = Some(snapshot);
                    }
                    Err(error) => {
                        eprintln!("enzui-shell: {error}");
                        self.snapshot = None;
                    }
                }
                if let Some(request) = self.requests.pop_front() {
                    self.refresh(Some(request))
                } else if std::mem::take(&mut self.snapshot_dirty) {
                    self.refresh(None)
                } else {
                    Task::none()
                }
            }
            Message::FocusSidebar(id) => {
                if Some(id) == self.sidebar_rail
                    && self.sidebar_focused
                    && let Some(entry) = self
                        .sidebar
                        .as_ref()
                        .filter(|s| s.keyboard_reveal && s.visible())
                        .and_then(|s| s.entries.first())
                {
                    iced_runtime::task::widget_in(
                        id,
                        iced::advanced::widget::operation::focusable::focus(
                            format!("miniapp-{}", entry.id).into(),
                        ),
                    )
                } else {
                    Task::none()
                }
            }
            Message::Renderer(backend, adapter) => {
                eprintln!("renderer: backend={backend} adapter={adapter}");
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

impl Shell {
    // Every 2 s tick: once a minute, with screen time on and the session
    // unlocked, adds the minute (and the focused app) to the local history.
    fn screen_time_tick(&mut self, now: Instant) -> Task<Message> {
        if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed {
            return Task::none();
        }
        if !self.desired_settings.screen_time {
            if !std::mem::replace(&mut self.screen_time_forgotten, true) {
                enzui_shell::screen_time::forget();
            }
            return Task::none();
        }
        self.screen_time_forgotten = false;
        if !self.schedule.screen_time.due(now) {
            return Task::none();
        }
        // Time behind the lock screen is not screen time.
        let seconds = self.schedule.screen_time_seconds(now);
        if enzui_shell::session_lock::locked() {
            return Task::none();
        }
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                let class = compositor.active_class().await;
                tokio::task::spawn_blocking(move || {
                    enzui_shell::screen_time::record(class.as_deref(), seconds)
                })
                .await
                .unwrap_or_else(|_| Err(tr("shell-screen-time-stopped").into()))
            },
            Message::ScreenTimeRecorded,
        )
    }

    // Each tick: gives the compositor the mirroring setting when it changed,
    // and while mirroring every 10 s (`system_due`) so a new display joins.
    fn mirror_tick(&mut self, system_due: bool) -> Task<Message> {
        let wanted = self.desired_settings.display_mirror;
        if self.mirror_busy || (self.mirror_applied == Some(wanted) && !(wanted && system_due)) {
            return Task::none();
        }
        // Nothing to undo on a desktop that never mirrored.
        if self.mirror_applied.is_none() && !wanted {
            self.mirror_applied = Some(false);
            return Task::none();
        }
        self.mirror_busy = true;
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.set_mirroring(wanted).await },
            move |result| Message::Mirrored(wanted, result),
        )
    }

    // Restore once after login, then remember app placement each minute while enabled.
    fn session_apps_tick(&mut self, now: Instant) -> Task<Message> {
        if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed {
            return Task::none();
        }
        let restore_due = now >= self.schedule.session_restore_at;
        if !self.desired_settings.restore_apps {
            // A list turned on later starts from that moment, not from this login.
            self.session_restored |= restore_due;
            if !std::mem::replace(&mut self.session_forgotten, true)
                && let Some(path) = enzui_shell::session_restore::path()
            {
                let _ = std::fs::remove_file(path);
            }
            return Task::none();
        }
        self.session_forgotten = false;
        let compositor = self.compositor.clone();
        if !self.session_restored {
            if !restore_due {
                return Task::none();
            }
            self.session_restored = true;
            return Task::perform(
                async move { compositor.restore_session_apps().await.map(drop) },
                Message::SessionApps,
            );
        }
        if self.schedule.session_apps.due(now) {
            return Task::perform(
                async move { compositor.remember_session_apps().await },
                Message::SessionApps,
            );
        }
        Task::none()
    }

    // An installed shell follows primary-output changes by exiting for supervisor restart; apps
    // remain running.
    fn check_outputs(&mut self) -> Task<Message> {
        if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed {
            return Task::none();
        }
        self.outputs_checked = Some(Instant::now());
        let compositor = self.compositor.clone();
        let frame_lost = self.frame_lost;
        Task::perform(
            async move { compositor.primary_change(frame_lost).await },
            Message::OutputsChecked,
        )
    }
}

fn control_body_retry(after: Duration) -> Task<Message> {
    Task::perform(tokio::time::sleep(after), |()| Message::ControlBodyRetry)
}

impl Shell {
    /// Settings is about to change a display's scale: hide the chrome, and
    /// answer once its fade has reached the screen. A change announced while
    /// another is under way starts the wait again.
    pub(crate) fn begin_rescale_hold(&mut self) -> Task<Message> {
        self.rescale_generation = self.rescale_generation.wrapping_add(1);
        let generation = self.rescale_generation;
        let full = Duration::from_millis(u64::from(self.chrome_fade_ms()) + 40);
        let (fade, wait) = match self.frame_fade {
            // Already fading out, for a style change or an earlier rescale.
            Some((since, true)) => (Task::none(), full.saturating_sub(since.elapsed())),
            _ if self.chrome_shown() => (self.start_frame_fade(true), full),
            // Fading in, or hidden at startup: from wherever it is now.
            Some((_, false)) => (self.start_frame_fade(true), full),
            None => (Task::none(), Duration::ZERO),
        };
        self.rescale_hold = Some(RescaleHold {
            generation,
            applied: None,
            placing: false,
        });
        self.quiet = false;
        Task::batch([
            fade,
            Task::perform(tokio::time::sleep(wait), move |()| {
                Message::RescaleHidden(generation)
            }),
            Task::perform(tokio::time::sleep(RESCALE_LIMIT), move |()| {
                Message::RescaleLimit(generation)
            }),
        ])
    }

    /// Settings has applied the announced change, or failed to: wait for the
    /// compositor to settle the output.
    pub(crate) fn rescale_applied(&mut self) -> Task<Message> {
        match &mut self.rescale_hold {
            Some(hold) if hold.applied.is_none() => {
                hold.applied = Some(Instant::now());
                Task::done(Message::RescaleSettle(hold.generation))
            }
            _ => Task::none(),
        }
    }

    fn rescale_checked(
        &mut self,
        generation: u64,
        change: enzui_shell::compositor::OutputChange,
        settled: bool,
    ) -> Task<Message> {
        use enzui_shell::compositor::OutputChange;
        let Some(hold) = self
            .rescale_hold
            .filter(|hold| hold.generation == generation)
        else {
            return Task::none();
        };
        if change == OutputChange::Moved {
            enzui_shell::compositor::restart_for_new_output();
        }
        let waited = hold.applied.map_or(Duration::ZERO, |at| at.elapsed());
        if !settled && waited < RESCALE_SETTLE_LIMIT {
            return Task::perform(tokio::time::sleep(RESCALE_POLL), move |()| {
                Message::RescaleSettle(generation)
            });
        }
        // The compositor may move an earlier placement while pinning the output.
        self.rescale_hold = Some(RescaleHold {
            placing: true,
            ..hold
        });
        self.bar_output_stale = true;
        if self.chrome_styled && !self.bar_style_fade {
            self.update_bar_preferences()
        } else {
            Task::none()
        }
    }

    /// Ends a rescale hold; the chrome fades back in with the frame and rail.
    pub(crate) fn release_rescale_hold(&mut self) -> Task<Message> {
        if self.rescale_hold.take().is_some() {
            self.start_frame_fade(false)
        } else {
            Task::none()
        }
    }
}
