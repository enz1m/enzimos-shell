//! The desktop overview combines Search, desktop previews and the current windows.
//!
//! It composes the existing surfaces: Search's card (centred under the bar,
//! keeping the keyboard) over the window spread, whose overlay also draws the
//! desktops strip. Closing either closes both.

use super::{Message, Shell};
use crate::quick_search::Message as SearchMessage;
use crate::window_spread::{OverviewBand, SpreadMessage};
use enzui_shell::{commands::OverviewSwipe, desktop_preferences::SuperKey};
use iced::Task;

#[derive(Debug, Clone, Copy)]
enum SwipeMode {
    Discrete,
    Search,
    Spread { from: f32 },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Swipe {
    opening: bool,
    mode: SwipeMode,
}

impl Swipe {
    fn position(self, from: f32, progress: f32) -> f32 {
        let delta = if self.opening { progress } else { -progress };
        (from + delta).clamp(0.0, 1.0)
    }
}

impl Shell {
    /// A lone Super press; `spread` is false on a tiling desktop, where the
    /// overview is Search alone. Over a fullscreen app the compositor sends none.
    pub(crate) fn activities_command(&mut self, spread: bool) -> Task<Message> {
        // The framed bar keeps Super for shortcuts.
        if self.metrics.islands.is_none() {
            return Task::none();
        }
        match SuperKey::from_value(self.desired_settings.compositor.super_key.into()) {
            SuperKey::Nothing => Task::none(),
            _ if self.activities => self.close_activities(),
            SuperKey::Search => self.search_message(SearchMessage::Toggle),
            _ if self.search.showing => self.search_message(SearchMessage::Toggle),
            _ if self.spread_active() => self.close_spread(None),
            _ if !spread => self.search_message(SearchMessage::Toggle),
            SuperKey::Overview => self.open_activities(),
        }
    }

    pub(crate) fn show_activities(&mut self) -> Task<Message> {
        if self.activities {
            return Task::none();
        }
        if self.overview_uses_search_only() {
            return if self.search.showing {
                Task::none()
            } else {
                self.search_message(SearchMessage::Show(None))
            };
        }
        let old_spread = self.abort_spread();
        Task::batch([old_spread, self.open_activities()])
    }

    pub(crate) fn activities_swipe(&mut self, event: OverviewSwipe) -> Task<Message> {
        match event {
            OverviewSwipe::Begin { opening } => self.begin_activities_swipe(opening),
            OverviewSwipe::End {
                progress,
                velocity,
                cancelled,
            } => self.finish_activities_swipe(progress, velocity, cancelled),
            OverviewSwipe::Cancel => self.finish_activities_swipe(0.0, 0.0, true),
            OverviewSwipe::Update(_) => Task::none(),
        }
    }

    fn begin_activities_swipe(&mut self, opening: bool) -> Task<Message> {
        let interrupted = if self.activities_swipe.is_some() {
            self.finish_activities_swipe(0.0, 0.0, true)
        } else {
            Task::none()
        };
        if (opening && self.activities) || (!opening && !self.activities && !self.search.showing) {
            self.quiet = true;
            return interrupted;
        }
        let interactive =
            !self.window_tokens(None).motion.reduced_motion && !self.overview_uses_search_only();
        enzui_shell::power_boost::request();
        let mut mode = SwipeMode::Discrete;
        let prepare = if interactive && opening && !self.search.showing {
            let (from, prepare) = self.prepare_overview_swipe();
            mode = SwipeMode::Spread { from };
            prepare
        } else if interactive && self.activities {
            if let Some(from) = self.spread.begin_swipe() {
                mode = SwipeMode::Spread { from };
            }
            Task::none()
        } else {
            Task::none()
        };
        self.activities_swipe = Some(Swipe { opening, mode });
        Task::batch([interrupted, prepare])
    }

    fn overview_uses_search_only(&self) -> bool {
        self.metrics.islands.is_some()
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.tiling)
    }

    fn prepare_overview_swipe(&mut self) -> (f32, Task<Message>) {
        if let Some(from) = self.spread.reverse_swipe_close() {
            self.activities = true;
            return (from, Task::none());
        }
        let previous = self.abort_spread();
        self.activities = true;
        self.activities_following = false;
        let band = self.overview_band();
        let load = self.spread_message(SpreadMessage::Overview(band));
        let _ = self.spread.begin_swipe();
        (0.0, Task::batch([previous, load]))
    }

    pub(crate) fn update_activities_swipe(&self, progress: f32) {
        if let Some(
            swipe @ Swipe {
                mode: SwipeMode::Spread { from },
                ..
            },
        ) = self.activities_swipe
        {
            self.spread.update_swipe(swipe.position(from, progress));
        }
    }

    fn finish_activities_swipe(
        &mut self,
        progress: f32,
        velocity: f32,
        cancelled: bool,
    ) -> Task<Message> {
        let Some(swipe) = self.activities_swipe.take() else {
            self.quiet = true;
            return Task::none();
        };
        let travelled = match swipe.mode {
            SwipeMode::Spread { from } if swipe.opening => from + progress,
            SwipeMode::Spread { from } => (1.0 - from) + progress,
            _ => progress,
        };
        let commit = !cancelled && progress >= 0.1 && travelled + 0.12 * velocity >= 0.5;
        let open = swipe.opening == commit;
        let SwipeMode::Spread { from } = swipe.mode else {
            return match (commit, swipe.opening, swipe.mode) {
                (true, true, SwipeMode::Search) => self.search_message(SearchMessage::Show(None)),
                (true, true, _) => self.show_activities(),
                (true, false, _) => self.close_search(),
                _ => Task::none(),
            };
        };
        let position = (!cancelled).then(|| swipe.position(from, progress));
        let speed = if cancelled {
            0.0
        } else if swipe.opening {
            velocity
        } else {
            -velocity
        };
        let spread = self.settle_spread_swipe(position, open, speed);
        if open {
            if !self.search.showing {
                return Task::batch([spread, self.search_message(SearchMessage::Show(None))]);
            }
            spread
        } else {
            self.activities = false;
            Task::batch([spread, self.close_search()])
        }
    }

    fn open_activities(&mut self) -> Task<Message> {
        self.activities = true;
        self.activities_following = false;
        self.search.overview = true;
        let band = self.overview_band();
        Task::batch([
            self.search_message(SearchMessage::Show(None)),
            self.spread_message(SpreadMessage::Overview(band)),
        ])
    }

    fn overview_band(&self) -> OverviewBand {
        OverviewBand {
            search_height: Self::overview_search_height()
                + self
                    .metrics
                    .islands
                    .map_or_else(|| f32::from(self.metrics.radius), |_| 0.0),
        }
    }

    fn close_activities(&mut self) -> Task<Message> {
        self.activities_swipe = None;
        if !std::mem::take(&mut self.activities) {
            return Task::none();
        }
        Task::batch([self.close_search(), self.close_spread(None)])
    }

    /// Search is closing: so is the rest of the overview.
    pub(crate) fn overview_search_closing(&mut self) -> Task<Message> {
        self.activities_swipe = None;
        if std::mem::take(&mut self.activities) {
            self.close_spread(None)
        } else {
            Task::none()
        }
    }

    /// The spread is closing, a window chosen or not: so is Search.
    pub(crate) fn overview_spread_closing(&mut self) -> Task<Message> {
        self.activities_swipe = None;
        if std::mem::take(&mut self.activities) {
            self.close_search()
        } else {
            Task::none()
        }
    }

    /// A desktop was chosen in the strip: it comes up with Search's card on it.
    pub(crate) fn overview_select_desktop(&mut self, id: u32) -> Task<Message> {
        if !self.activities {
            return Task::none();
        }
        self.activities_swipe = None;
        self.activities_following = true;
        self.desktop_steps = 0;
        self.desktop_wheel = Default::default();
        self.desktop_busy = true;
        self.desktop_generation = self.desktop_generation.wrapping_add(1);
        let generation = self.desktop_generation;
        let compositor = self.compositor.clone();
        let search = self.search.shown_epoch();
        Task::perform(
            async move { compositor.overview_desktop_focus(id, search).await },
            move |result| Message::DesktopsRead(generation, result),
        )
    }

    /// The desktop chosen in the strip is up: the overview spreads its windows.
    pub(crate) fn overview_desktop_shown(&mut self) -> Task<Message> {
        if !std::mem::take(&mut self.activities_following) || !self.activities {
            return Task::none();
        }
        self.quiet = false;
        let band = self.overview_band();
        self.spread_message(SpreadMessage::Overview(band))
    }

    /// The spread could not open; Search stays as it would on its own.
    pub(crate) fn overview_spread_failed(&mut self) {
        self.activities = false;
        if let Some(swipe) = &mut self.activities_swipe {
            swipe.mode = SwipeMode::Search;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn island_shell() -> Shell {
        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        shell.metrics.islands = Some(enzui_shell::shell_style::Islands::default());
        shell
    }

    #[test]
    fn framed_swipe_uses_the_spread_without_enabling_super_tap() {
        let mut shell = island_shell();
        shell.metrics.islands = None;
        shell.snapshot = Some(enzui_shell::compositor::Snapshot {
            workspace: 1,
            tiling: true,
            keyboard_layout: None,
            keyboard: None,
        });
        let _ = shell.activities_command(true);
        assert!(!shell.spread_active());

        let _ = shell.activities_swipe(OverviewSwipe::Begin { opening: true });
        assert!(shell.activities);
        assert!(shell.spread_active());
        assert!(matches!(
            shell.activities_swipe,
            Some(Swipe {
                mode: SwipeMode::Spread { .. },
                ..
            })
        ));
    }

    #[test]
    fn choosing_another_desktop_keeps_the_overview_and_spreads_it_once_up() {
        let mut shell = island_shell();
        let _ = shell.activities_command(true);
        let _ = shell.spread_message(SpreadMessage::Desktop(2));
        assert!(shell.activities);
        assert!(shell.search.overview);
        assert!(!shell.spread_active());

        let _ = shell.overview_desktop_shown();
        assert!(shell.activities);
        assert!(shell.spread_active());

        let _ = shell.activities_command(true);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }
}
