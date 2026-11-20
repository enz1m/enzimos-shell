use super::{Message, Shell, SidebarConfiguration, resize_chrome, window_settings};
use enzui_shell::{desktop_frame, settings::BarEdge};
use iced::{Task, window};

/// How long a new style's rail stays hidden before it fades in: long enough
/// for the rail it rebuilds to be on screen.
const RAIL_SETTLE: std::time::Duration = std::time::Duration::from_millis(40);

impl Shell {
    pub(crate) fn sidebar_configuration(&self) -> SidebarConfiguration {
        SidebarConfiguration {
            side: self.desired_settings.sidebar,
            bar: self.bar,
            metrics: self.metrics,
            motion: self.desired_settings.compositor.motion(),
            enabled: self.desired_settings.sidebar_enabled,
            behavior: self.desired_settings.sidebar_behavior,
            pin_scope: self.desired_settings.sidebar_pin_scope,
        }
    }

    pub(crate) fn refresh_sidebar(
        &mut self,
        action: Option<enzui_shell::sidebar::Action>,
    ) -> Task<Message> {
        if self.sidebar_busy || self.bar_busy || !self.bar_ready {
            if let Some(action) = action
                && self.sidebar_queue.len() < 16
            {
                self.sidebar_queue.push_back(action);
            }
            self.sidebar_dirty = true;
            return Task::none();
        }
        let desired = self.sidebar_configuration();
        let placement = (self.sidebar_configured != Some(desired)).then_some(desired);
        let action = if placement.is_some() {
            if let Some(action) = action {
                self.sidebar_queue.push_back(action);
            }
            None
        } else {
            action
        };
        self.sidebar_busy = true;
        self.sidebar_dirty = false;
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                if let Some(config) = placement {
                    compositor
                        .sidebar_configure(
                            config.side,
                            config.bar,
                            config.metrics,
                            config.enabled,
                            config.motion,
                            Some(config.behavior),
                            config.pin_scope,
                        )
                        .await
                } else if let Some(action) = action {
                    compositor.sidebar_action(action).await
                } else {
                    compositor.sidebar_snapshot().await
                }
            },
            move |result| Message::SidebarRead(placement, result),
        )
    }

    pub(crate) fn sidebar_read(
        &mut self,
        placement: Option<SidebarConfiguration>,
        result: Result<enzui_shell::sidebar::Snapshot, String>,
    ) -> Task<Message> {
        self.sidebar_busy = false;
        let state = match result {
            Ok(state) => state,
            Err(error) => {
                eprintln!("enzui-shell sidebar: {error}");
                // A placement turned down while a pane is still closing is
                // asked for again; telling the user each time posted a raw
                // compositor error whenever the bar moved with a pane open.
                // A refused action is still reported.
                if placement.is_none() {
                    self.notice = Some(error);
                }
                if self.sidebar_configured.take().is_some() {
                    self.bar_ready = false;
                    return self.apply_bar();
                }
                return Task::none();
            }
        };
        let placement_unchanged = placement
            .as_ref()
            .is_none_or(|placement| self.sidebar_configured.as_ref() == Some(placement));
        if let Some(placement) = placement {
            self.sidebar_configured = Some(placement);
        }
        let mut tasks = Vec::new();
        if let Some(id) = self.sidebar_rail {
            let source = state.source_rect();
            if self.sidebar.as_ref().is_none_or(|old| {
                let previous = old.source_rect();
                previous.width != source.width || previous.height != source.height
            }) {
                tasks.push(resize_chrome(
                    id,
                    iced::Size::new(source.width as f32, source.height as f32),
                ));
            }
            // Islands owns a finite Wayland input mask; winit hit-testing would overwrite it.
            if self.metrics.islands.is_none()
                && self
                    .sidebar
                    .as_ref()
                    .is_none_or(|old| old.visible() != state.visible())
            {
                tasks.push(if state.visible() {
                    window::disable_mouse_passthrough(id)
                } else {
                    window::enable_mouse_passthrough(id)
                });
            }
        } else {
            let source = state.source_rect();
            let (id, open) = window::open(window_settings(
                "dev.enzui.Sidebar",
                source.width as f32,
                source.height as f32,
            ));
            self.sidebar_rail = Some(id);
            tasks.push(open.map(Message::Opened));
        }
        if self
            .sidebar
            .as_ref()
            .is_some_and(|old| old.backing_class() != state.backing_class())
            && let Some(id) = self.sidebar_pane.take()
        {
            tasks.push(window::close(id));
        }
        if let Some(id) = self.sidebar_pane {
            if self.sidebar.as_ref().is_none_or(|old| {
                old.pane.width != state.pane.width || old.pane.height != state.pane.height
            }) {
                tasks.push(resize_chrome(
                    id,
                    iced::Size::new(state.pane.width as f32, state.pane.height as f32),
                ));
            }
        } else {
            let (id, open) = window::open(window_settings(
                state.backing_class(),
                state.pane.width as f32,
                state.pane.height as f32,
            ));
            self.sidebar_pane = Some(id);
            tasks.push(open.map(Message::Opened));
        }
        self.quiet = placement_unchanged
            && self
                .sidebar
                .as_ref()
                .is_some_and(|old| format!("{old:?}") == format!("{state:?}"));
        self.sidebar = Some(state);
        self.sidebar_read_at = Some(std::time::Instant::now());
        // The frame treats a showing rail as its own band.
        self.update_frame();
        if let Some(action) = self.sidebar_queue.pop_front() {
            tasks.push(self.refresh_sidebar(Some(action)));
        } else if self.sidebar_dirty
            || self.sidebar_configured != Some(self.sidebar_configuration())
        {
            tasks.push(self.refresh_sidebar(None));
        }
        Task::batch(tasks)
    }

    pub(crate) fn desktop_edges(&self) -> (u32, u32) {
        self.frame_appearance_value().edges()
    }

    pub(crate) fn frame_appearance_value(&self) -> desktop_frame::Appearance {
        let chrome = self.chrome_color().map(|color| {
            let [r, g, b, _] = color.into_rgba8();
            [r, g, b]
        });
        let mut appearance = desktop_frame::Appearance {
            light: self.chrome_light(),
            surface: chrome.or_else(|| {
                self.plugin
                    .theme_surface(&self.base_tokens())
                    .or_else(|| self.themed_surface())
            }),
            transparency: 100 - self.desired_settings.chrome.opacity.clamp(50, 100),
            hidden: self.frame_hidden(),
            fade_ms: self.chrome_fade_ms(),
            fade_curve: self.chrome_fade_curve(),
            bottom: self.bar == BarEdge::Bottom,
            metrics: self.metrics,
            // A rail that shows is part of the frame: the frame's edge and
            // its shadow begin past it, not across its icons.
            sidebar: self
                .sidebar
                .as_ref()
                .filter(|s| s.visible())
                .map(|s| s.side),
            // On the frame the rail window paints its side whenever it is
            // there, a frame-wide strip of it while hidden.
            rail: self
                .sidebar
                .as_ref()
                .filter(|s| {
                    s.enabled && self.sidebar_rail.is_some() && self.metrics.islands.is_none()
                })
                .map(|s| s.side),
            ..Default::default()
        };
        appearance.bar_shadow = self.desired_settings.bar_shadow;
        // The islands' shadows: the connected bar as one shape, or each island.
        // Glass islands cast their own with the glass.
        if let Some(style) = self
            .metrics
            .islands
            .filter(|_| self.desired_settings.bar_shadow > 0 && !self.bar_glass_on)
        {
            let units = |value: f32| (value.max(0.0) * desktop_frame::SHAPE_UNITS).round() as u16;
            let shape = |x: f32, width: f32| {
                [
                    units(x),
                    units(f32::from(style.edge_gap)),
                    units(width),
                    units(f32::from(self.metrics.bar)),
                ]
            };
            let shapes: Vec<[u16; 4]> = if style.joined_bar {
                vec![shape(
                    f32::from(style.side_gap),
                    self.bar_size.width - 2.0 * f32::from(style.side_gap),
                )]
            } else {
                self.bar_geometry
                    .iter()
                    .flat_map(|(_, geometry)| {
                        geometry
                            .islands
                            .iter()
                            .map(|island| shape(island.bounds.x, island.bounds.width))
                    })
                    .take(8)
                    .collect()
            };
            for (slot, shape) in appearance.shapes.iter_mut().zip(&shapes) {
                *slot = *shape;
            }
            appearance.shape_count = shapes.len().min(8) as u8;
        }
        appearance
    }

    // Whether the frame is faded out while the shell changes style: 0 or
    // 100. The frame helper fades between them at the display's pace.
    fn frame_hidden(&self) -> u8 {
        if self.chrome_shown() { 0 } else { 100 }
    }

    /// How long the bar's fade takes, in milliseconds (its fade follows the
    /// theme's normal motion): the frame and the rail fade with it.
    pub(crate) fn chrome_fade_ms(&self) -> u16 {
        let motion = self.tokens().motion;
        if motion.reduced_motion {
            0
        } else {
            motion.normal_ms.round().clamp(0.0, 1000.0) as u16
        }
    }

    // The bar's fade curve, for the frame's (`Appearance::fade_curve`).
    fn chrome_fade_curve(&self) -> u8 {
        match self.tokens().motion.easing_or(None) {
            enzui_theme::MotionEasing::Linear => 1,
            enzui_theme::MotionEasing::EaseIn => 2,
            enzui_theme::MotionEasing::EaseInOut => 3,
            _ => 0,
        }
    }

    /// Whether the chrome (bar, frame and rail) shows while the shell
    /// changes style: it all fades out together, and comes back together once
    /// the new style is in place, the sidebar included, so nothing arrives
    /// ahead of the rest.
    pub(crate) fn chrome_shown(&self) -> bool {
        // Coming back, the bar waits for the tick that starts the frame's
        // and the rail's fades too: deciding on its own, it led them by up
        // to a tick.
        !self.bar_style_fade && self.frame_fade.is_none()
    }

    // The new style is on screen: applied, and the sidebar has taken it
    // (bounded, should it never answer).
    fn new_style_ready(&self, since: std::time::Instant) -> bool {
        // The rail's new picture comes with the first snapshot read after
        // the sidebar took the style (released at the configuration, the
        // islands' pill came in a beat after the bar).
        since.elapsed() >= RAIL_SETTLE
            && (self.sidebar_configured.is_some()
                && self.sidebar_read_at.is_some_and(|at| at > since)
                || !self.desired_settings.sidebar_enabled
                || since.elapsed() >= std::time::Duration::from_secs(1))
    }

    /// Fades the frame out (`out`) or back in, with the bar's own fade.
    pub(crate) fn start_frame_fade(&mut self, out: bool) -> Task<Message> {
        self.frame_fade = Some((std::time::Instant::now(), out));
        self.update_frame();
        let rail = if out {
            self.start_rail_fade(0.0)
        } else {
            Task::none()
        };
        let compositor = self.compositor.clone();
        // The rail repaints through the scene, which sends it frames only
        // when it draws: keep them coming for the fade.
        let frames = u64::from(self.chrome_fade_ms()) + 200;
        let wake =
            Task::future(async move { compositor.wake_sidebar_rail(frames).await }).discard();
        if out {
            Task::batch([wake, rail])
        } else {
            Task::batch([
                wake,
                Task::perform(tokio::time::sleep(RAIL_SETTLE), |()| Message::FrameFadeTick),
            ])
        }
    }

    // The rail fades in the compositor, like the frame, on the bar's curve
    // and time (an in-window fade reached the screen a frame or two late:
    // the rail repaints through the scene). Returns the task stepping it.
    fn start_rail_fade(&mut self, to: f32) -> Task<Message> {
        let from = self.rail_alpha_value();
        self.rail_fade = Some((
            std::time::Instant::now() + desktop_frame::fade_lag(to),
            from,
            to,
        ));
        self.rail_alpha_tick()
    }

    fn rail_alpha_value(&self) -> f32 {
        let Some((start, from, to)) = self.rail_fade else {
            return self.rail_alpha;
        };
        let length = f32::from(self.chrome_fade_ms()).max(1.0) / 1000.0;
        let t = std::time::Instant::now()
            .saturating_duration_since(start)
            .as_secs_f32()
            / length;
        from + (to - from) * desktop_frame::fade_eased(self.chrome_fade_curve(), t)
    }

    pub(crate) fn rail_alpha_tick(&mut self) -> Task<Message> {
        let mut value = self.rail_alpha_value();
        if let Some((start, _, to)) = self.rail_fade {
            let length = std::time::Duration::from_millis(u64::from(self.chrome_fade_ms()));
            if std::time::Instant::now().saturating_duration_since(start) >= length {
                self.rail_fade = None;
                self.rail_alpha = to;
                // The last step lands exactly on the end, not a step short.
                value = to;
            }
        }
        let acquire = self.apply_rail_alpha(value);
        let next = self.rail_alpha_next();
        Task::batch([acquire, next])
    }

    // Sets the rail's alpha on its surface, asking for that surface first
    // when the rail window has none yet.
    fn apply_rail_alpha(&mut self, value: f32) -> Task<Message> {
        let Some(id) = self.sidebar_rail else {
            return Task::none();
        };
        if let Some((held, input)) = &self.rail_alpha_surface
            && *held == id
        {
            input.set_alpha(value);
            return Task::none();
        }
        if std::mem::replace(&mut self.rail_alpha_pending, true) {
            return Task::none();
        }
        let shared = self.surface_globals.clone();
        iced_runtime::window::wayland_parent_export(id).then(move |payload| {
            let shared = shared.clone();
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        enzui_shell::surface_input::acquire_for_glass(payload, shared)
                    })
                    .await
                    .unwrap_or_else(|_| Err("surface worker stopped".into()))
                },
                move |result| Message::RailAlphaSurface(id, result),
            )
        })
    }

    pub(crate) fn rail_alpha_surface_ready(
        &mut self,
        id: window::Id,
        result: Result<enzui_shell::surface_input::SurfaceInput, String>,
    ) -> Task<Message> {
        self.rail_alpha_pending = false;
        if let Ok(input) = result
            && Some(id) == self.sidebar_rail
        {
            input.set_alpha(self.rail_alpha_value());
            self.rail_alpha_surface = Some((id, input));
        }
        Task::none()
    }

    /// Whether the rail may draw: while the style changes, only once its
    /// compositor fade has a hold of its surface (a rail window the change
    /// rebuilt showed at full strength for a frame before that).
    pub(crate) fn rail_paintable(&self) -> bool {
        self.frame_fade.is_none()
            || self
                .rail_alpha_surface
                .as_ref()
                .is_some_and(|(id, _)| Some(*id) == self.sidebar_rail)
    }

    fn rail_alpha_next(&self) -> Task<Message> {
        // While a style change is under way a rail window it rebuilds takes
        // the current value too, instead of appearing at full strength.
        if self.rail_fade.is_some() || self.frame_fade.is_some() {
            return Task::perform(
                tokio::time::sleep(std::time::Duration::from_millis(8)),
                |()| Message::RailAlphaTick,
            );
        }
        Task::none()
    }

    pub(crate) fn frame_fade_tick(&mut self) -> Task<Message> {
        let Some((since, false)) = self.frame_fade else {
            return Task::none();
        };
        if self.new_style_ready(since) {
            self.frame_fade = None;
            self.update_frame();
            return self.start_rail_fade(1.0);
        }
        Task::perform(
            tokio::time::sleep(std::time::Duration::from_millis(15)),
            |()| Message::FrameFadeTick,
        )
    }

    pub(crate) fn update_frame(&self) {
        enzui_shell::reveal::set_glass_tweaks(enzui_shell::reveal::GlassTweaks {
            still: self.tokens().motion.reduced_motion,
            ..self.desired_settings.chrome.glass()
        });
        let appearance = self.frame_appearance_value();
        self.frame_appearance.send_if_modified(|value| {
            if *value == appearance {
                false
            } else {
                *value = appearance;
                true
            }
        });
    }
}
