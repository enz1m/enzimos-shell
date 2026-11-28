use super::{
    model::{HeightMotion, Message},
    results,
    support::{
        CARD_OUTER_PADDING, CARD_PADDING, FIELD_SIZE, RECOMMENDED_HEIGHT, SEARCH_CHROME, message,
        perform, result_capacity, row_pitch, source_chord,
    },
};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::search::{
    Category,
    compositor::{IslandPlacement, SearchGeometry},
};
use iced::{Subscription, Task, window};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

impl Shell {
    pub(super) fn search_failure(&self) -> Option<&str> {
        let state = &self.search;
        state
            .model
            .error
            .as_deref()
            .or(state.file_error.as_deref())
            .or_else(|| {
                if state.model.category == Some(Category::Clipboard) {
                    state
                        .clipboard
                        .error
                        .as_deref()
                        .or(state.clipboard.notice.as_deref())
                } else {
                    None
                }
            })
    }
    /// The card's height apart from its result rows; the full app list
    /// keeps the recommended apps above it.
    pub(super) fn search_chrome(&self) -> f32 {
        SEARCH_CHROME
            + if self.search.browse_all_apps {
                RECOMMENDED_HEIGHT
            } else {
                0.0
            }
    }
    pub(super) fn search_expanded(&self) -> bool {
        !self.search.model.query.trim().is_empty()
            || self.search.model.category.is_some()
            || self.search.browse_all_apps
            || self.search.alias.is_some()
            || self.search.preedit.is_some()
    }
    /// The tallest the search body grows: room for the chosen number of
    /// result rows and two group headings, never less than the 520 the alias
    /// and composition views use. A fixed 520 held six rows whatever the
    /// Rows setting said.
    pub(super) fn search_height_limit(&self) -> f32 {
        self.search_height_limit_with(self.search_chrome())
    }
    fn search_height_limit_with(&self, chrome: f32) -> f32 {
        let row = row_pitch(self.search.preferences.compact);
        (chrome
            + row * f32::from(self.search.preferences.rows)
            + 2.0 * results::GROUP_HEADING_HEIGHT)
            .max(520.0)
    }
    pub(super) fn search_surface_size(&self) -> iced::Size {
        let maximum = self
            .search
            .geometry
            .as_ref()
            .map_or(iced::Size::new(620.0, 760.0), |g| g.0);
        let height: f32 = if self.search.alias.is_some() || self.search.preedit.is_some() {
            520.0
        } else if self.search_expanded() {
            let window = enzui_core::palette::window_from(
                self.search.model.results.len(),
                self.search_capacity(),
                self.search.model.selected_index(),
                self.search.model.window_start,
            );
            let headings = self
                .search
                .model
                .heading_count(&(window.start..window.end()));
            let feedback = self
                .search_failure()
                .or(self.search.feedback.as_deref())
                .is_some();
            (self.search_chrome()
                + row_pitch(self.search.preferences.compact) * window.len.max(1) as f32
                + results::GROUP_HEADING_HEIGHT * headings as f32
                + if feedback { 40.0 } else { 0.0 })
            .min(self.search_height_limit())
        } else {
            self.search_resting_height()
        };
        iced::Size::new(
            maximum.width,
            height.min(maximum.height)
                + self.search.island.map_or(0.0, |island| {
                    island.attachment.join(f32::from(self.metrics.radius))
                }),
        )
    }
    pub(crate) fn overview_search_height() -> f32 {
        FIELD_SIZE.tokens().height + CARD_PADDING.y() + 2.0 * CARD_OUTER_PADDING
    }

    /// The card's height before anything is typed, as it rests under the bar.
    fn search_resting_height(&self) -> f32 {
        if self.search.overview {
            Self::overview_search_height()
        } else if self.search.blank_apps() {
            BLANK_HEIGHT
        } else {
            112.0
        }
    }
    pub(super) fn search_window_extent(&self) -> iced::Size {
        let mut size = self.search_surface_size();
        // The window keeps room for the full app list under the recommended apps whether or
        // not it is open; the card paints and takes input only over its own height. Resizing
        // the window as All apps opened and closed let the compositor re-centre it for a frame.
        let limit = if self.search.preferences.shows(Category::Apps) {
            self.search_height_limit_with(SEARCH_CHROME + RECOMMENDED_HEIGHT)
        } else {
            self.search_height_limit()
        };
        size.height = self
            .search
            .geometry
            .as_ref()
            .map_or(520.0, |geometry| geometry.0.height.min(limit))
            + self.search.island.map_or(0.0, |island| {
                island.attachment.join(f32::from(self.metrics.radius))
            });
        // A floating card's window keeps room around it for its shadow.
        if let Some(island) = self.search.island {
            let m = enzui_shell::reveal::margins(island.bottom, island.attachment);
            size.width += m.left + m.right;
            size.height += m.top + m.bottom;
        }
        size
    }
    pub(super) fn search_height_motion_enabled(&self) -> bool {
        self.search.preferences.animations && !self.tokens().motion.reduced_motion
    }
    pub(super) fn search_paint_height(&self) -> f32 {
        if !self.search_height_motion_enabled() {
            return self.search_surface_size().height;
        }
        if let Some(held) = self.search.height_hold {
            return held;
        }
        self.search.height_motion.map_or_else(
            || self.search_surface_size().height,
            |motion| motion.height(Instant::now()),
        )
    }
    pub(super) fn animate_search_height(&mut self, from: f32, to: f32) {
        self.search.height_hold = None;
        self.search.height_due = None;
        self.search.height_motion = if self.search.visible
            && self.search_height_motion_enabled()
            && (from - to).abs() > 1.0
        {
            Some(HeightMotion {
                from,
                to,
                started: Instant::now(),
                duration: Duration::from_secs_f32(
                    0.15 * enzui_shell::motion::duration_factor(
                        self.desired_settings.compositor.animation_rate,
                    ),
                ),
            })
        } else {
            None
        };
    }
    pub(crate) fn search_sync_layout(&mut self) -> Task<ShellMessage> {
        if !self.search.showing
            || !self.search.started
            || self.search.preparing
            || self.search.layout_busy
        {
            return Task::none();
        }
        let Some(id) = self.search.window else {
            return Task::none();
        };
        let Some((_, output)) = &self.search.geometry else {
            return Task::none();
        };
        let size = self.search_window_extent();
        if self.search.surface_size == Some(size) {
            return Task::none();
        }
        let output = output.clone();
        // `resize_chrome` pinned the old size as both limits. Hyprland clamps a floating window
        // to its limits on every commit and re-centres it, so the compositor's resize would
        // move the card by half the change until `Resized` pins the new size. Admit both sizes
        // for the duration; the larger limit goes first so the minimum never exceeds it.
        let old = self.search.surface_size.unwrap_or(size);
        let limits = window::set_max_size(
            id,
            Some(iced::Size::new(
                old.width.max(size.width),
                old.height.max(size.height),
            )),
        )
        .chain(window::set_min_size(
            id,
            Some(iced::Size::new(
                old.width.min(size.width),
                old.height.min(size.height),
            )),
        ));
        self.search.surface_size = Some(size);
        self.search.layout_busy = true;
        self.search.layout_revision = self.search.layout_revision.saturating_add(1);
        let revision = self.search.layout_revision;
        let epoch = self.search.epoch;
        let compositor = self.compositor.clone();
        let placement = self.search.island;
        limits.chain(perform(
            async move {
                compositor
                    .resize_search(&output, size, epoch, placement)
                    .await
            },
            move |result| Message::Resized(epoch, revision, result),
        ))
    }
    pub(super) fn search_capacity(&self) -> usize {
        let mut size = self
            .search
            .geometry
            .as_ref()
            .map_or(iced::Size::new(620.0, 760.0), |g| g.0);
        size.height = size.height.min(self.search_height_limit());
        if !self.search_expanded() && self.search.blank_apps() {
            return 4;
        }
        let feedback = self
            .search_failure()
            .or(self.search.feedback.as_deref())
            .is_some();
        let chrome = self.search_chrome();
        let mut rows = result_capacity(
            size,
            chrome,
            self.search.preferences.compact,
            self.search.preferences.rows,
            feedback,
        );
        while rows > 1 {
            let window = enzui_core::palette::window_from(
                self.search.model.results.len(),
                rows,
                self.search.model.selected_index(),
                self.search.model.window_start,
            );
            let headings = self
                .search
                .model
                .heading_count(&(window.start..window.end()));
            let occupied = row_pitch(self.search.preferences.compact) * window.len as f32
                + results::GROUP_HEADING_HEIGHT * headings as f32;
            if occupied <= size.height - chrome - if feedback { 40.0 } else { 0.0 } {
                break;
            }
            rows -= 1;
        }
        rows
    }
    pub(crate) fn search_subscription(&self) -> Subscription<ShellMessage> {
        let keys = if self.search.showing {
            iced::event::listen_with_interest(
                iced::event::Interest::KEYBOARD,
                |event, status, id| match event {
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key,
                        modifiers,
                        repeat,
                        ..
                    }) => {
                        let search_key = status == iced::event::Status::Ignored
                            || source_chord(&key, modifiers).is_some()
                            || matches!(
                                key,
                                iced::keyboard::Key::Named(
                                    iced::keyboard::key::Named::ArrowLeft
                                        | iced::keyboard::key::Named::ArrowRight
                                )
                            );
                        (search_key || crate::window_spread::activities_key_candidate(&key))
                            .then_some((id, key, modifiers, repeat, search_key))
                    }
                    _ => None,
                },
            )
            .with((self.search.model.generation, self.activities))
            .filter_map(
                |((generation, activities), (id, key, modifiers, repeat, search_key))| {
                    (search_key || activities)
                        .then(|| message(Message::Key(id, generation, key, modifiers, repeat)))
                },
            )
        } else {
            Subscription::none()
        };
        // Presses in Search's own window, to tell them from presses elsewhere.
        let presses = match self.search.window.filter(|_| self.search.showing) {
            Some(window) => iced::event::listen_with(|event, _, id| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
                | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => Some(id),
                _ => None,
            })
            .with(window)
            .filter_map(|(window, id)| (id == window).then(|| message(Message::Pressed))),
            None => Subscription::none(),
        };
        // Presses in the bar, where Search's trigger is.
        let bar_presses = if self.search.configured {
            iced::event::listen_with(|event, _, id| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
                | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => Some(id),
                _ => None,
            })
            .with(self.panel)
            .filter_map(|(panel, id)| (id == panel).then(|| message(Message::BarPressed)))
        } else {
            Subscription::none()
        };
        let ime = if self.search.showing {
            iced::event::listen_with(|event, _, id| {
                use iced::advanced::input_method::Event;
                match event {
                    iced::Event::InputMethod(Event::Preedit(text, _)) => Some((id, Some(text))),
                    iced::Event::InputMethod(Event::Opened | Event::Closed | Event::Commit(_)) => {
                        Some((id, None))
                    }
                    _ => None,
                }
            })
            .with((
                self.search.epoch,
                self.search.alias.as_ref().map(|value| value.0.clone()),
            ))
            .map(|((epoch, alias), (window, text))| {
                message(Message::Preedit {
                    window,
                    epoch,
                    alias,
                    text,
                })
            })
        } else {
            Subscription::none()
        };
        // Only while the height moves: a held height is still, and its
        // `HeightDue` task ends the hold.
        // At the display's rate, redrawing only the search.
        let height_frames = if self.search.showing && self.search.height_motion.is_some() {
            iced::window::frames().map(|_| message(Message::HeightFrame))
        } else {
            Subscription::none()
        };
        Subscription::batch([
            keys,
            presses,
            bar_presses,
            ime,
            height_frames,
            self.search
                .clipboard
                .subscription()
                .map(|event| message(Message::Clipboard(event))),
        ])
    }
    pub(crate) fn warm_search(&mut self) -> Task<ShellMessage> {
        let bar_epoch = self.bar_applied_epoch;
        if !self.bar_ready
            || self.bar_busy
            || !self.search.configured
            || !self.search.preferences.enabled
            || self.search.showing
            || self.search.window.is_some()
            || self.search.warmed_bar == Some(bar_epoch)
        {
            return Task::none();
        }
        self.search.warmed_bar = Some(bar_epoch);
        let epoch = self.search.epoch;
        let compositor = self.compositor.clone();
        let (metrics, edge) = (self.metrics, self.bar);
        enzui_shell::widget_bounds::bounds(self.panel, "applications").then(move |anchor| {
            let compositor = compositor.clone();
            perform(
                async move { compositor.search_geometry(metrics, edge, anchor).await },
                move |result| Message::WarmPrepared(bar_epoch, epoch, result),
            )
        })
    }

    pub(super) fn set_search_geometry(&mut self, geometry: SearchGeometry) -> iced::Size {
        self.search.geometry = Some((geometry.body_max, geometry.output));
        self.search.island = geometry.island.map(|island| IslandPlacement {
            dim: island.dim && !self.search.overview,
            overview: self.search.overview,
            ..island
        });
        let size = self.search_window_extent();
        self.search.surface_size = Some(size);
        size
    }

    pub(super) fn create_search_window(&mut self, size: iced::Size) -> Task<ShellMessage> {
        let (id, open) = window::open(crate::window_settings(
            "dev.enzui.QuickSearch",
            size.width,
            size.height,
        ));
        self.search.window = Some(id);
        self.search.started = false;
        self.search.visible = false;
        open.map(ShellMessage::Opened)
    }

    pub(super) fn search_reveal_ready(&mut self) -> Task<ShellMessage> {
        if !self.search.showing
            || !self.search.presented
            || self.search.window.is_none()
            || !self
                .search
                .window
                .is_some_and(|id| self.surface_inputs.contains_key(&id))
        {
            return Task::none();
        }
        let first = !self.search.started;
        self.search.visible = true;
        self.search.started = true;
        let focus = iced::widget::operation::focus("quick-search-input");
        if !first || self.search.overview {
            return focus;
        }
        let epoch = self.search.epoch;
        // Icon resolution and catalogue reads share the cores drawing the card.
        let settle = Duration::from_millis(200);
        Task::batch([
            focus,
            perform(async move { tokio::time::sleep(settle).await }, move |()| {
                Message::RefreshApps(epoch)
            }),
        ])
    }
    pub(crate) fn discard_search_surface(&mut self) -> Task<ShellMessage> {
        let _ = self.close_search();
        self.search.overview = false;
        self.search.started = false;
        self.search.presented = false;
        self.search.geometry = None;
        self.search.island = None;
        self.search.surface_size = None;
        self.search.height_motion = None;
        self.search.height_hold = None;
        self.search.height_due = None;
        self.search.pending_browse_collapse = false;
        self.search
            .window
            .take()
            .map_or_else(Task::none, |id| self.finish_panel_close(id))
    }
    pub(crate) fn close_search(&mut self) -> Task<ShellMessage> {
        let overview = self.overview_search_closing();
        Task::batch([overview, self.close_search_card()])
    }
    fn close_search_card(&mut self) -> Task<ShellMessage> {
        if !self.search.showing && !self.search.preparing {
            return Task::none();
        }
        self.search.showing = false;
        self.search
            .provider_generation
            .fetch_add(1, Ordering::AcqRel);
        self.search.provider_results.clear();
        self.search.activation = None;
        self.search.visible = false;
        self.search.presented = false;
        self.search.preparing = false;
        self.search.height_motion = None;
        self.search.height_hold = None;
        self.search.height_due = None;
        self.search.pending_browse_collapse = false;
        if let Some(id) = self.search.window {
            self.pending_surface_focus.remove(&id);
        }
        if let Some(live) = &self.search.presentation {
            live.store(false, Ordering::Release);
        }
        self.search.epoch = self.search.epoch.saturating_add(1);
        self.search.alias = None;
        self.search.preedit = None;
        self.search.model.invalidate();
        let dim = if self.search.started {
            let compositor = self.compositor.clone();
            let epoch = self.search.epoch;
            let fade =
                self.search.preferences.background_fade && !self.tokens().motion.reduced_motion;
            perform(
                async move { compositor.hide_search_dim(epoch, fade).await },
                move |result| Message::DimHidden(epoch, result),
            )
        } else {
            Task::none()
        };
        if self.search.preferences.animations && self.search.started {
            return dim;
        }
        let Some(id) = self.search.window else {
            return Task::none();
        };
        Task::batch([
            dim,
            self.search_message(Message::Hidden(id, self.search.epoch)),
        ])
    }
    pub(crate) fn search_opened(&mut self, id: window::Id) -> Task<ShellMessage> {
        if self.search.showing {
            self.search_reveal_ready()
        } else {
            self.search_message(Message::Hidden(id, self.search.epoch))
        }
    }
    pub(crate) fn search_window(
        &mut self,
        id: window::Id,
        event: &window::Event,
    ) -> Task<ShellMessage> {
        if self.search.window != Some(id) {
            return Task::none();
        }
        if matches!(event, window::Event::Focused | window::Event::Unfocused) {
            self.search.focus_revision = self.search.focus_revision.wrapping_add(1);
        }
        match event {
            window::Event::Focused if self.search.showing && self.search.island.is_some() => {
                self.search_reveal_ready()
            }
            window::Event::Focused if self.search.showing => {
                self.search.visible = true;
                self.search.started = true;
                iced::widget::operation::focus("quick-search-input")
            }
            window::Event::Unfocused
                if self.search.showing && self.search.started && !self.search.action_busy =>
            {
                let compositor = self.compositor.clone();
                let epoch = self.search.epoch;
                let revision = self.search.focus_revision;
                let overview = self.search.overview;
                let close_prompt_pids = self.spread_close_prompt_pids();
                perform(
                    async move {
                        compositor
                            .search_focus_retained(overview, &close_prompt_pids)
                            .await
                            .unwrap_or(false)
                    },
                    move |kept| Message::FocusChecked(epoch, revision, kept),
                )
            }
            window::Event::CloseRequested => self.close_search(),
            window::Event::Closed => {
                self.search.overview = false;
                self.search.preedit = None;
                self.search.window = None;
                self.search.showing = false;
                self.search.visible = false;
                self.search.started = false;
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

/// The blank page: search field, heading, one row of app icons and the All
/// apps link.
pub(super) const BLANK_HEIGHT: f32 = 216.0;
