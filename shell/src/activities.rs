//! The Island bar's overview, opened by Super on its own: Search, the
//! desktops and the current desktop's windows side by side, as one view.
//!
//! It composes the existing surfaces: Search's card (centred under the bar,
//! keeping the keyboard) over the window spread, whose overlay also draws the
//! desktops strip. Closing either closes both.

use super::{Message, Shell};
use crate::quick_search::Message as SearchMessage;
use crate::window_spread::{OverviewBand, SpreadMessage};
use enzui_shell::desktop_preferences::SuperKey;
use iced::Task;

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

    // The centred card can join the bar with rounded corners that add the
    // panel radius below its resting height.
    fn overview_band(&self) -> OverviewBand {
        OverviewBand {
            search: self.search_resting_height() + f32::from(self.metrics.radius),
        }
    }

    fn close_activities(&mut self) -> Task<Message> {
        if !std::mem::take(&mut self.activities) {
            return Task::none();
        }
        Task::batch([self.close_search(), self.close_spread(None)])
    }

    /// Search is closing: so is the rest of the overview.
    pub(crate) fn overview_search_closing(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.activities) {
            self.close_spread(None)
        } else {
            Task::none()
        }
    }

    /// The spread is closing, a window chosen or not: so is Search.
    pub(crate) fn overview_spread_closing(&mut self) -> Task<Message> {
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
