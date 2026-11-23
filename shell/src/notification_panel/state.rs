use super::model::Content;
use super::model::{Message, Pane, State};
use crate::Message as ShellMessage;
use enzui_shell::i18n::tr;
use enzui_shell::{
    compositor::ControlGeometry,
    notifications::{
        model::{Archived, Key, Notification},
        server::{Command, Snapshot},
    },
    settings::BarEdge,
};
use iced::window;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

pub(super) fn message(value: Message) -> ShellMessage {
    ShellMessage::Notification(value)
}
pub(super) fn bar_center_anchor(size: iced::Size) -> iced::Rectangle {
    iced::Rectangle::new(
        iced::Point::new((size.width - 1.0) / 2.0, 0.0),
        iced::Size::new(1.0, size.height),
    )
}
pub(super) fn center_padding(
    geometry: &ControlGeometry,
    bar: BarEdge,
    radius: f32,
) -> iced::Padding {
    let size = iced::Size::new(geometry.width, geometry.height);
    let clip = enzui_shell::reveal::Reveal::popup(
        bar == BarEdge::Bottom,
        geometry.attachment,
        radius,
        iced::Color::TRANSPARENT,
        enzui_shell::reveal::PanelShadow {
            room: true,
            ..Default::default()
        },
    )
    .panel_frame(size, 1.0)
    .clip;
    iced::Padding {
        top: clip.y + enzui_shell::panel_style::INSET,
        right: size.width - clip.x - clip.width + enzui_shell::panel_style::INSET,
        bottom: size.height - clip.y - clip.height + enzui_shell::panel_style::INSET,
        left: clip.x + enzui_shell::panel_style::INSET,
    }
}
pub(super) fn key_id(key: Key) -> String {
    format!("notification:{}:{}:{}", key.epoch, key.id, key.revision)
}
impl State {
    pub(crate) fn center_open(&self) -> bool {
        self.center.is_some()
    }
    pub(super) fn blocks_toasts(&self) -> bool {
        self.content != Content::Activity && (self.center.is_some() || self.center_opening)
    }
    pub(super) fn cancel_center_actions(&mut self) {
        if self.content != Content::Activity {
            self.actions.cancel();
        }
    }
    pub(crate) fn owns_center(&self, id: window::Id) -> bool {
        self.center.as_ref().is_some_and(|pane| pane.window == id)
    }
    pub(crate) fn passive_activity(&self, id: window::Id) -> bool {
        self.activity.passive && self.owns_center(id)
    }
    pub(crate) fn owns_window(&self, id: window::Id) -> bool {
        self.center.as_ref().is_some_and(|pane| pane.window == id)
            || self.toast.as_ref().is_some_and(|pane| pane.window == id)
    }
    pub(crate) fn ready(&self) -> bool {
        self.channel.is_some()
            && self
                .authority
                .as_ref()
                .is_some_and(|authority| authority.load(Ordering::Acquire))
    }
    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        if self.ready() {
            self.snapshot
                .as_deref()
                .filter(|snapshot| Some(snapshot.epoch) == self.epoch)
        } else {
            None
        }
    }
    pub(super) fn action_pending(&self, key: &Key) -> bool {
        self.pending.contains_key(key) || self.actions.waiting(key)
    }
    pub(super) fn action_source(&self, key: Key) -> Option<window::Id> {
        if let Some(pane) = self.center.as_ref().filter(|_| self.blocks_toasts()) {
            let entries = self.current_entries();
            let page = self.page.min(entries.len().saturating_sub(1) / 12);
            return (pane.visible
                && self.content == Content::Notifications
                && !self.history
                && entries
                    .iter()
                    .skip(page * 12)
                    .take(12)
                    .any(|entry| entry.key == key))
            .then_some(pane.window);
        }
        self.toast
            .as_ref()
            .filter(|pane| {
                pane.visible
                    && !self.blocks_toasts()
                    && self
                        .toast_entries()
                        .iter()
                        .take(self.toast_count)
                        .any(|entry| entry.key == key)
            })
            .map(|pane| pane.window)
    }
    pub(super) fn current(&self, key: Key) -> Option<&Arc<Notification>> {
        self.snapshot()?
            .active
            .iter()
            .find(|entry| entry.key == key && entry.expires.is_none_or(|at| at > Instant::now()))
    }
    pub(super) fn unread(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot.active.iter().filter(|entry| entry.unread).count()
        })
    }
    /// Rough height of the current list's groups and cards, for sizing the
    /// panel to what it holds.
    // Senders with two or more current notifications: each gets a heading.
    pub(super) fn group_headings(&self) -> usize {
        let entries = self.current_entries();
        let mut groups: Vec<&str> = entries
            .iter()
            .map(|n| n.origin.principal.as_str())
            .collect();
        groups.dedup();
        groups
            .iter()
            .filter(|principal| {
                entries
                    .iter()
                    .filter(|n| n.origin.principal == **principal)
                    .count()
                    > 1
            })
            .count()
    }
    pub(super) fn current_entries(&self) -> Vec<&Arc<Notification>> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        enzui_shell::notifications::presentation::grouped_order(
            snapshot
                .active
                .iter()
                .map(|entry| entry.origin.principal.as_str()),
        )
        .into_iter()
        .map(|index| &snapshot.active[index])
        .collect()
    }
    pub(super) fn history_entries(&self) -> Vec<&Archived> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let entries: Vec<_> = snapshot
            .history
            .iter()
            .filter(|entry| {
                enzui_shell::notifications::presentation::history_matches(
                    entry,
                    &self.history_query,
                )
            })
            .collect();
        enzui_shell::notifications::presentation::grouped_order(
            entries.iter().map(|entry| entry.principal.as_str()),
        )
        .into_iter()
        .map(|index| entries[index])
        .collect()
    }
    // Whether a shown toast has already come in, so a new one opens its row
    // before sliding instead of pushing the shown ones aside at once.
    pub(super) fn shown_revealed(&self) -> bool {
        self.toast_entries()
            .into_iter()
            .take(self.toast_count)
            .any(|entry| self.revealed.contains_key(&(entry.key.epoch, entry.key.id)))
    }
    pub(super) fn animating(&self, now: Instant, timing: super::motion::Timing) -> bool {
        !timing.reduced
            && (self.revealed.values().any(|(at, push)| {
                now.saturating_duration_since(*at)
                    < timing.slide
                        + if *push {
                            timing.push
                        } else {
                            Default::default()
                        }
            }) || self
                .toast_leaving
                .values()
                .any(|at| now.saturating_duration_since(*at) < timing.leave)
                || self.dismiss_all_opacity(now, timing).1)
    }
    // Fades dismiss-all after arrivals and hides it when fewer than two toasts remain.
    pub(super) fn dismiss_all_opacity(
        &self,
        now: Instant,
        timing: super::motion::Timing,
    ) -> (f32, bool) {
        let display = self.toast_display();
        let id = |entry: &Arc<Notification>| (entry.key.epoch, entry.key.id);
        if timing.reduced {
            return (
                if display
                    .iter()
                    .filter(|entry| !self.toast_leaving.contains_key(&id(entry)))
                    .count()
                    >= 2
                {
                    1.0
                } else {
                    0.0
                },
                false,
            );
        }
        // When the newest of `entries` has finished arriving and waited.
        let appears = |entries: &mut dyn Iterator<Item = &Arc<Notification>>| -> Option<Instant> {
            let mut latest: Option<Instant> = None;
            for entry in entries {
                let (at, push) = *self.revealed.get(&id(entry))?;
                let done = at
                    + timing.slide
                    + if push {
                        timing.push
                    } else {
                        Default::default()
                    }
                    + timing.delay;
                latest = Some(latest.map_or(done, |l| l.max(done)));
            }
            latest
        };
        let fade_in = |appear: Instant, at: Instant| {
            at.saturating_duration_since(appear).as_secs_f32() / timing.fade_in.as_secs_f32()
        };
        let staying: Vec<_> = display
            .iter()
            .filter(|entry| !self.toast_leaving.contains_key(&id(entry)))
            .collect();
        if staying.len() >= 2 {
            let Some(appear) = appears(&mut staying.iter().copied()) else {
                return (0.0, true);
            };
            let t = fade_in(appear, now);
            return (t.clamp(0.0, 1.0), t < 1.0);
        }
        // Fading out from however far it had come when the first card left.
        let Some(left) = self.toast_leaving.values().max().copied() else {
            return (0.0, false);
        };
        let had = appears(&mut display.iter())
            .map_or(0.0, |appear| fade_in(appear, left).clamp(0.0, 1.0));
        let t = now.saturating_duration_since(left).as_secs_f32() / timing.fade_out.as_secs_f32();
        ((had * (1.0 - t)).clamp(0.0, 1.0), t < 1.0 && had > 0.0)
    }
    pub(super) fn toast_display(&self) -> Vec<Arc<Notification>> {
        // The first `toast_count`, and, while the window is being resized for
        // a newcomer, any already drawn.
        let mut shown: Vec<Arc<Notification>> = self
            .toast_entries()
            .into_iter()
            .enumerate()
            .filter(|(index, entry)| {
                *index < self.toast_count
                    || (self.toast_opening
                        && self.toast_shown.iter().any(|kept| {
                            (kept.key.epoch, kept.key.id) == (entry.key.epoch, entry.key.id)
                        }))
            })
            .map(|(_, entry)| std::sync::Arc::clone(entry))
            .collect();
        for (index, entry) in self.toast_shown.iter().enumerate() {
            if self
                .toast_leaving
                .contains_key(&(entry.key.epoch, entry.key.id))
                && !shown
                    .iter()
                    .any(|c| (c.key.epoch, c.key.id) == (entry.key.epoch, entry.key.id))
            {
                shown.insert(index.min(shown.len()), std::sync::Arc::clone(entry));
            }
        }
        shown
    }
    pub(super) fn toast_entries(&self) -> Vec<&Arc<Notification>> {
        if self.blocks_toasts() || self.toasts_paused {
            return Vec::new();
        }
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .active
                    .iter()
                    .filter(|entry| {
                        let now = Instant::now();
                        entry.unread
                            && entry.expires.is_none_or(|until| until > now)
                            && entry.toast.visible(now)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn hold_projection(&self, entries: impl Iterator<Item = Key>) -> Vec<Key> {
        if self.toast.is_some()
            && !self.blocks_toasts()
            && (self.toast_hovered || self.toast_focused)
        {
            entries.take(self.toast_count).collect()
        } else {
            Vec::new()
        }
    }
    pub(super) fn sync_hold(&mut self, renew: bool) {
        let keys = self.hold_projection(self.toast_entries().into_iter().map(|entry| entry.key));
        self.send_hold(keys, renew);
    }
    pub(super) fn send_hold(&mut self, keys: Vec<Key>, renew: bool) {
        if !self.ready() {
            self.held_keys.clear();
            return;
        }
        if (!renew && keys == self.held_keys) || (keys.is_empty() && self.held_keys.is_empty()) {
            return;
        }
        if self
            .channel
            .as_ref()
            .is_some_and(|channel| channel.try_send(Command::HoldToasts(keys.clone())).is_ok())
        {
            self.held_keys = keys;
        }
    }
    pub(super) fn clear_toast(&mut self) -> Option<Pane> {
        let pane = self.toast.take();
        if pane.is_some() {
            self.actions.cancel();
        }
        self.toast_count = 0;
        self.toast_shown.clear();
        self.toast_leaving.clear();
        self.toast_hovered = false;
        self.toast_focused = false;
        self.sync_hold(false);
        pane
    }
    pub(crate) fn send(&mut self, command: Command) -> bool {
        if !self.ready() {
            self.error = Some(tr("shell-notification-service-is-unavailable").into());
            return false;
        }
        if self
            .channel
            .as_ref()
            .is_some_and(|channel| channel.try_send(command).is_ok())
        {
            true
        } else {
            self.error = Some(tr("shell-notification-service-is-busy-try-again").into());
            false
        }
    }
}
