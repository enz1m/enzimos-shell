use super::{
    model::{Content, Message},
    state::message,
};
use crate::{Message as ShellMessage, Shell};
use iced::{Task, window};
use std::time::Duration;

const OPEN_DELAY: Duration = Duration::from_millis(360);
const CLOSE_DELAY: Duration = Duration::from_millis(280);

impl Shell {
    fn can_preview_activity(&self) -> bool {
        !self.notification.center_opening
            && self.notification.center.is_none()
            && !self.control_requested
            && !self.control_visible
            && !self.search.showing
            && !self.launcher_visible
            && !self.launcher_opening
            && self.overview.is_none()
            && !self.overview_opening
            && self.plugin.pane.is_none()
            && self.has_visible_activity()
    }

    pub(super) fn activity_hover(&mut self, trigger: bool, inside: bool) -> Task<ShellMessage> {
        self.quiet = true;
        let may_open = self.can_preview_activity();
        let preview = &mut self.notification.activity;
        let hovered = if trigger {
            &mut preview.trigger_hovered
        } else {
            &mut preview.panel_hovered
        };
        if std::mem::replace(hovered, inside) == inside {
            return Task::none();
        }
        preview.revision = preview.revision.wrapping_add(1);
        let revision = preview.revision;
        let delay = if trigger && inside && may_open {
            OPEN_DELAY
        } else if preview.passive && !preview.trigger_hovered && !preview.panel_hovered {
            CLOSE_DELAY
        } else {
            return Task::none();
        };
        hover_delay(revision, delay)
    }

    pub(super) fn activity_hover_elapsed(&mut self, revision: u64) -> Task<ShellMessage> {
        let preview = &mut self.notification.activity;
        if revision == preview.revision {
            if preview.passive && !preview.trigger_hovered && !preview.panel_hovered {
                let Some(anchor) = preview.trigger_bounds else {
                    return self.close_notifications();
                };
                // Mapping the panel can clear bar hover while the pointer stays on the trigger.
                let compositor = self.compositor.clone();
                self.quiet = true;
                return Task::perform(
                    async move { compositor.activity_pointer_inside(anchor).await },
                    move |inside| message(Message::ActivityPointerChecked(revision, inside)),
                );
            }
            if preview.trigger_hovered && self.can_preview_activity() {
                self.notification.content = Content::Activity;
                self.notification.activity.passive = true;
                return self.notification_prepare_center();
            }
        }
        self.quiet = true;
        Task::none()
    }

    pub(super) fn activity_pointer_checked(
        &mut self,
        revision: u64,
        inside: bool,
    ) -> Task<ShellMessage> {
        let activity = &self.notification.activity;
        if revision == activity.revision && activity.passive {
            if !inside {
                return self.close_notifications();
            }
            self.quiet = true;
            return hover_delay(revision, CLOSE_DELAY);
        }
        self.quiet = true;
        Task::none()
    }

    pub(crate) fn activity_panel_window(&self) -> Option<window::Id> {
        self.notification.center.as_ref().and_then(|pane| {
            (self.notification.content == Content::Activity && pane.visible).then_some(pane.window)
        })
    }

    pub(crate) fn sync_activity_panel(&mut self) -> Task<ShellMessage> {
        if self.has_visible_activity() {
            return Task::none();
        }
        self.notification.activity.trigger_hovered = false;
        if self.notification.content == Content::Activity
            && (self.notification.center.is_some() || self.notification.center_opening)
        {
            return self.close_notifications();
        }
        Task::none()
    }
}

fn hover_delay(revision: u64, delay: Duration) -> Task<ShellMessage> {
    Task::perform(async move { tokio::time::sleep(delay).await }, move |()| {
        message(Message::ActivityHoverElapsed(revision))
    })
}
