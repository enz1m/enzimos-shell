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
    /// A lone Super press; `spread` is false on a tiling desktop or over a
    /// fullscreen app, where the overview is Search alone.
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
        self.search.overview = true;
        let band = OverviewBand {
            search: self.search_resting_height(),
        };
        Task::batch([
            self.search_message(SearchMessage::Show(None)),
            self.spread_message(SpreadMessage::Overview(band)),
        ])
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

    /// A desktop was chosen in the strip: Search goes at once, before its
    /// desktop slides away.
    pub(crate) fn overview_leaving_desktop(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.activities) {
            self.close_search_at_once()
        } else {
            Task::none()
        }
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
    fn super_opens_nothing_with_the_framed_bar() {
        let mut shell = island_shell();
        shell.metrics.islands = None;
        let _ = shell.activities_command(true);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }

    #[test]
    fn super_opens_the_spread_with_search_and_closes_both() {
        let mut shell = island_shell();
        let _ = shell.activities_command(true);
        assert!(shell.activities);
        assert!(shell.search.overview);
        assert!(shell.spread_active());

        let _ = shell.activities_command(true);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }

    #[test]
    fn super_on_a_tiling_desktop_leaves_the_windows_in_place() {
        let mut shell = island_shell();
        let _ = shell.activities_command(false);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }

    #[test]
    fn closing_the_spread_closes_the_overview() {
        let mut shell = island_shell();
        let _ = shell.activities_command(true);
        let _ = shell.spread_message(SpreadMessage::Dismiss);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }

    #[test]
    fn the_super_key_preference_can_turn_the_overview_off() {
        let mut shell = island_shell();
        shell.desired_settings.compositor.super_key = 2;
        let _ = shell.activities_command(true);
        assert!(!shell.activities);
        assert!(!shell.spread_active());
    }
}
