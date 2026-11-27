use super::{Message, Shell, window_settings};
use iced::{Task, window};
use std::time::{Duration, Instant};

/// How long windows stay unchanged before previews are taken while the
/// overview is closed.
const PREVIEW_SETTLE: Duration = Duration::from_millis(1500);
const REVEAL_QUIET: Duration = Duration::from_millis(120);
// Some overview styles continue drawing while idle.
const REVEAL_LIMIT: Duration = Duration::from_millis(1500);

impl Shell {
    pub(crate) fn invalidate_previews(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.preview_dirty = true;
    }

    pub(crate) fn preview_cache_current(&self) -> bool {
        self.preview_cached_revision == Some(self.preview_revision)
            && !self.preview_failed
            && self.desktops.as_ref().is_some_and(|d| {
                d.entries
                    .iter()
                    .all(|d| self.previews.iter().any(|p| p.desktop == d.id))
            })
    }

    pub(crate) fn defer_previews_to_reveal(&mut self) {
        self.preview_after_reveal = true;
        self.overview_reveal_at = None;
        self.overview_frame_at = None;
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.preview_busy {
            self.preview_busy = false;
            self.preview_generation = self.preview_generation.wrapping_add(1);
        }
    }

    fn previews_wait_for_reveal(&self) -> bool {
        self.preview_after_reveal && self.preview_cache_current()
    }

    pub(crate) fn watch_overview_reveal(&mut self) -> Task<Message> {
        if !self.preview_after_reveal {
            return Task::none();
        }
        self.overview_reveal_at = Some(Instant::now());
        self.overview_frame_at = None;
        self.overview_reveal_check(REVEAL_QUIET)
    }

    fn overview_reveal_check(&self, after: Duration) -> Task<Message> {
        let generation = self.overview_generation;
        Task::perform(async move { tokio::time::sleep(after).await }, move |()| {
            Message::OverviewRevealCheck(generation)
        })
    }

    pub(crate) fn check_overview_reveal(&mut self, generation: u64) -> Task<Message> {
        if generation != self.overview_generation
            || !self.preview_after_reveal
            || self.overview.is_none()
        {
            return Task::none();
        }
        let Some(start) = self.overview_reveal_at else {
            return Task::none();
        };
        let now = Instant::now();
        let last = self.overview_frame_at.map_or(start, |at| at.max(start));
        let quiet = now.saturating_duration_since(last);
        if quiet < REVEAL_QUIET && now.saturating_duration_since(start) < REVEAL_LIMIT {
            return self.overview_reveal_check(REVEAL_QUIET - quiet);
        }
        self.preview_after_reveal = false;
        self.start_previews()
    }

    pub(crate) fn overview_frame_subscription(&self) -> iced::Subscription<Message> {
        if !self.preview_after_reveal || self.overview.is_none() {
            return iced::Subscription::none();
        }
        iced::event::listen_raw(|event, _, id| match event {
            iced::Event::Window(window::Event::RedrawRequested(_)) => {
                Some(Message::OverviewFrame(id))
            }
            _ => None,
        })
    }

    pub(crate) fn open_ready_overview(&mut self) -> Task<Message> {
        if !self.overview_opening
            || self.overview_prepared.is_none()
            || self.desktops.is_none()
            || self.desktop_busy
            || self.desktop_dirty
            || self.preview_cached_revision != Some(self.preview_revision)
        {
            return Task::none();
        }
        let ready = self.desktops.as_ref().is_some_and(|d| {
            d.entries
                .iter()
                .all(|d| self.previews.iter().any(|p| p.desktop == d.id))
        });
        if !ready && !self.preview_failed {
            return Task::none();
        }
        let geometry = self
            .overview_prepared
            .take()
            .expect("the overview preparation completed before opening");
        self.overview_size = iced::Size::new(geometry.width, geometry.height);
        self.overview_attachment = geometry.attachment;
        self.overview_desktops = self.desktops.clone();
        self.overview_previews = self.previews.clone();
        let size = if self.metrics.islands.is_some() {
            self.popover_window(&geometry)
        } else {
            iced::Size::new(geometry.width, geometry.height)
        };
        let (id, open) = window::open(window_settings(
            "dev.enzui.WorkspaceOverview",
            size.width,
            size.height,
        ));
        self.overview = Some(id);
        self.overview_opening = false;
        self.overview_visible = false;
        self.overview_started = false;
        open.map(Message::Opened)
    }

    /// Takes workspace previews: at once while the overview opens or is
    /// open, otherwise once windows have settled for [`PREVIEW_SETTLE`], so
    /// a burst of windows opening and closing copies every window once
    /// rather than once per event.
    pub(crate) fn start_previews(&mut self) -> Task<Message> {
        if self.preview_busy
            || self.desktop_busy
            || self.desktop_dirty
            || self.previews_wait_for_reveal()
        {
            return Task::none();
        }
        if self.overview.is_none() && !self.overview_opening {
            self.preview_wait = self.preview_wait.wrapping_add(1);
            let wait = self.preview_wait;
            return Task::perform(tokio::time::sleep(PREVIEW_SETTLE), move |()| {
                Message::PreviewsDue(wait)
            });
        }
        self.take_previews()
    }

    pub(crate) fn take_previews(&mut self) -> Task<Message> {
        self.preview_dirty = false;
        self.preview_after_reveal = false;
        let Some(desktops) = self.desktops.clone() else {
            return Task::none();
        };
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.preview_cancel = Some(std::sync::Arc::clone(&cancel));
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_busy = true;
        let generation = self.preview_generation;
        let revision = self.preview_revision;
        let root = self.root.clone();
        let compositor = self.compositor.clone();
        let wallpaper = self.desired_settings.wallpaper.clone();
        Task::perform(
            async move {
                enzui_shell::previews::capture(root, compositor, desktops, wallpaper, cancel).await
            },
            move |result| Message::PreviewsReady(generation, revision, result),
        )
    }

    pub(crate) fn close_overview(&mut self) -> Task<Message> {
        if self.overview.is_none() && !self.overview_opening {
            return Task::none();
        }
        self.overview_visible = false;
        if self.overview_started {
            Task::none()
        } else {
            self.force_close_overview()
        }
    }

    pub(crate) fn force_close_overview(&mut self) -> Task<Message> {
        if self.overview.is_none() && !self.overview_opening {
            return Task::none();
        }
        self.overview_visible = false;
        self.overview_started = false;
        self.overview_prepared = None;
        self.overview_previews.clear();
        self.overview_desktops = None;
        self.overview_opening = false;
        let close = self.overview.take().map_or_else(Task::none, window::close);
        let previews = if std::mem::take(&mut self.preview_after_reveal) && self.preview_dirty {
            self.start_previews()
        } else {
            Task::none()
        };
        Task::batch([close, previews])
    }

    pub(crate) fn refresh_desktops(&mut self, direction: Option<i32>) -> Task<Message> {
        self.desktop_dirty = false;
        self.desktop_busy = true;
        self.desktop_generation = self.desktop_generation.wrapping_add(1);
        let generation = self.desktop_generation;
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                match direction {
                    Some(step) => compositor.desktop_step(step).await,
                    None => compositor.desktop_snapshot().await,
                }
            },
            move |result| Message::DesktopsRead(generation, result),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_shell::workspaces::{Desktop, Desktops};

    fn overview_shell(cached: bool) -> Shell {
        let compositor = enzui_shell::compositor::Compositor::new(
            std::path::PathBuf::from("/usr/bin/true"),
            "fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, std::path::PathBuf::from("/")).0;
        shell.desktops = Some(Desktops {
            active: 1,
            entries: vec![Desktop {
                id: 1,
                windows: 1,
                placeholder: false,
            }],
        });
        shell.desktop_busy = false;
        shell.desktop_dirty = false;
        shell.preview_busy = false;
        shell.preview_failed = false;
        if cached {
            shell.previews = vec![enzui_shell::previews::Preview {
                desktop: 1,
                image: iced::widget::image::Handle::from_rgba(1, 1, vec![0; 4]),
                unavailable: 0,
            }];
            shell.preview_cached_revision = Some(shell.preview_revision);
        }
        shell
    }

    fn reveal(shell: &mut Shell) -> window::Id {
        let id = window::Id::unique();
        shell.overview = Some(id);
        shell.overview_opening = false;
        let _ = shell.update(Message::Window(id, window::Event::Focused));
        id
    }

    #[test]
    fn a_cached_overview_captures_only_after_its_reveal_settles() {
        let mut shell = overview_shell(true);
        let _ = shell.take_previews();
        let cancel = shell.preview_cancel.clone().unwrap();
        let generation = shell.preview_generation;
        let _ = shell.update(Message::ToggleOverview);
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert_ne!(shell.preview_generation, generation);
        assert!(shell.overview_opening);
        assert!(!shell.preview_busy);
        assert!(shell.preview_after_reveal);

        let id = reveal(&mut shell);
        let _ = shell.update(Message::OverviewFrame(id));
        let generation = shell.overview_generation;
        let _ = shell.update(Message::OverviewRevealCheck(generation));
        assert!(!shell.preview_busy);

        let quiet = Instant::now() - REVEAL_QUIET;
        shell.overview_reveal_at = Some(quiet);
        shell.overview_frame_at = Some(quiet);
        let _ = shell.update(Message::OverviewRevealCheck(generation.wrapping_sub(1)));
        assert!(!shell.preview_busy);
        let _ = shell.update(Message::OverviewRevealCheck(generation));
        assert!(shell.preview_busy);
        assert!(!shell.preview_after_reveal);
    }

    #[test]
    fn an_overview_closed_during_its_reveal_waits_for_settled_windows() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let _context = runtime.enter();
        let mut shell = overview_shell(true);
        let _ = shell.update(Message::ToggleOverview);
        reveal(&mut shell);
        let wait = shell.preview_wait;
        let _ = shell.force_close_overview();
        assert!(!shell.preview_after_reveal);
        assert!(!shell.preview_busy);
        assert_ne!(shell.preview_wait, wait);
    }
}
