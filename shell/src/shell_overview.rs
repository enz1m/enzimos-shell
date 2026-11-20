use super::{Message, Shell, window_settings};
use iced::{Task, window};

/// How long windows stay unchanged before previews are taken while the
/// overview is closed.
const PREVIEW_SETTLE: std::time::Duration = std::time::Duration::from_millis(1500);

impl Shell {
    pub(crate) fn invalidate_previews(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.preview_dirty = true;
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
        if self.preview_busy || self.desktop_busy || self.desktop_dirty {
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
        self.overview.take().map_or_else(Task::none, window::close)
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
