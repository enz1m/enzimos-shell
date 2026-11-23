use super::model::Content;
use super::{
    model::{Message, Pane, Reply},
    state::{bar_center_anchor, message},
};
use crate::{Message as ShellMessage, Shell, resize_chrome, window_settings};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    compositor::{PopupPlacement, ToastPlacement},
    notifications::{
        model::Action,
        preferences::Change,
        presentation::Arrivals,
        runtime::{self, Update},
        server::{self, Command},
    },
    settings::Change as SettingChange,
};
use enzui_widgets::{icon::enz_icon, text::enz_text};
use iced::{Element, Task, widget::row, window};
use std::sync::atomic::Ordering;

const ACTIVITY_FOCUS_RETRIES: u8 = 8;

impl Shell {
    /// The height the center wants for the tab it shows: its fixed rows and
    /// every current card at its real height, or room for the history it
    /// holds, between a comfortable minimum and the usual maximum. The open
    /// center is placed again when this changes.
    fn notification_center_height(&self) -> f32 {
        // A line of its own for each thing that went wrong.
        let notices = [
            self.notification.error.is_some(),
            self.notification.shortcut_error.is_some(),
            self.notification.storage_error.is_some(),
        ]
        .into_iter()
        .filter(|shown| *shown)
        .count() as f32
            * 30.0;
        if self.notification.history {
            // A history with nothing in it is only a note; a short one does
            // not need the whole panel. Its cards run to about 92px under
            // their sender's heading. A search does not change the height:
            // a panel that jumps with every letter is hard to read.
            let senders: std::collections::BTreeMap<&str, usize> = self
                .notification
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    let mut senders = std::collections::BTreeMap::new();
                    for entry in &snapshot.history {
                        *senders.entry(entry.principal.as_str()).or_insert(0) += 1;
                    }
                    senders
                })
                .unwrap_or_default();
            if senders.is_empty() {
                return 300.0 + notices;
            }
            return (super::view::CENTER_CHROME
                + 48.0
                + senders.len() as f32 * 30.0
                + senders.values().sum::<usize>() as f32 * 104.0
                + notices)
                .clamp(300.0, 720.0);
        }
        let entries = self.notification.current_entries();
        if entries.is_empty() {
            return 300.0 + notices;
        }
        // A card under its sender's heading leaves out its own sender line.
        let cards: f32 = entries
            .iter()
            .map(|entry| {
                let grouped = entries
                    .iter()
                    .filter(|other| other.origin.principal == entry.origin.principal)
                    .count()
                    > 1;
                self.notification_card_height(entry) + 12.0 - if grouped { 20.0 } else { 0.0 }
            })
            .sum();
        // The gap counted after every card is not there under the last one.
        (super::view::CENTER_CHROME
            + self.notification.group_headings() as f32 * 30.0
            + cards
            + notices
            - 12.0)
            .clamp(220.0, 720.0)
    }
    /// Places the open center again when what it holds asks for another
    /// height: a notification came or went, the other tab was chosen, or a
    /// notice took a line. It used to keep the height it opened at, which
    /// gave History the few rows one notification needs.
    fn refit_notification_center(&mut self) -> Task<ShellMessage> {
        if self.notification.content != Content::Notifications
            || self.notification.center_opening
            || !self
                .notification
                .center
                .as_ref()
                .is_some_and(|pane| pane.visible)
        {
            return Task::none();
        }
        let wanted = self.notification_center_height();
        if (wanted - self.notification.center_requested).abs() <= 0.5 {
            return Task::none();
        }
        // Recorded before asking, so a placement that fails is not asked
        // for again with every message.
        self.notification.center_requested = wanted;
        self.notification_prepare_center()
    }

    pub(crate) fn notification_subscription(&self) -> iced::Subscription<ShellMessage> {
        if !enzui_shell::desktop_session::mode().managed() {
            return iced::Subscription::none();
        }
        let service = self
            .settings_store
            .as_ref()
            .map_or_else(iced::Subscription::none, |store| {
                runtime::subscription(
                    self.root.clone(),
                    store.path().to_owned(),
                    self.notification.run,
                )
                .map(|event| message(Message::Runtime(event)))
            });
        let renew = if self.notification.toast_hovered
            || self.notification.toast_focused
            || !self.notification.held_keys.is_empty()
        {
            iced::time::every(enzui_shell::notifications::toast::RENEW_INTERVAL)
                .map(|_| message(Message::RenewToasts))
        } else {
            iced::Subscription::none()
        };
        let now = std::time::Instant::now();
        let sliding = self.notification.toast.is_some()
            && self.notification.animating(now, self.toast_timing());
        // At the display's rate, redrawing only the toasts.
        let slide = if sliding {
            iced::window::frames().map(|_| message(Message::SlideFrame))
        } else {
            iced::Subscription::none()
        };
        iced::Subscription::batch([
            service,
            renew,
            slide,
            iced::event::listen_with(|event, _, id| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorEntered) => {
                    Some(message(Message::PopupHover(id, true)))
                }
                iced::Event::Mouse(iced::mouse::Event::CursorLeft) => {
                    Some(message(Message::PopupHover(id, false)))
                }
                _ => None,
            }),
            enzui_shell::shortcuts::subscription().map(|event| message(Message::Shortcut(event))),
        ])
    }
    // The clock, opening its calendar.
    pub(crate) fn notification_clock(&self) -> Element<'_, ShellMessage> {
        enzui_shell::bar_item::trigger(
            enzui_widgets::clickable::enz_clickable(
                row![
                    enz_text(&self.clock)
                        .size_px(f32::from(self.metrics.text))
                        .weight_rung(enzui_theme::WeightRung::Medium)
                        .into_element()
                ]
                .align_y(iced::Alignment::Center),
                message(Message::ToggleCalendar),
            ),
            self.metrics,
        )
        .align_y(iced::Alignment::Center)
        .aria_label(tr!("shell-open-the-calendar", "arg0" => self.clock.clone()))
        .id("clock-trigger".into())
        .into_element()
    }
    // The bell with the unread count, opening the notifications.
    pub(crate) fn notification_bell(&self) -> Element<'_, ShellMessage> {
        let count = self.notification.unread();
        let mut content = row![
            enz_icon(
                if self
                    .desired_settings
                    .notifications
                    .effective_policy(
                        self.system
                            .game_mode
                            .as_ref()
                            .is_some_and(enzui_shell::game_mode::Status::active)
                    )
                    .dnd
                {
                    "bell-off"
                } else {
                    "bell"
                }
            )
            .size(f32::from(self.metrics.icon))
            .into_element(),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        if count > 0 {
            content = content.push(
                enz_text(count.to_string())
                    .size_px(f32::from(self.metrics.text))
                    .into_element(),
            );
        }
        enzui_shell::bar_item::trigger(
            enzui_widgets::clickable::enz_clickable(content, message(Message::Toggle)),
            self.metrics,
        )
        .align_y(iced::Alignment::Center)
        .aria_label(tr!("shell-notifications-unread", "arg0" => format!("{:}", count)))
        .id("notifications-trigger".into())
        .into_element()
    }
    pub(crate) fn notification_geometry_changed(&mut self) -> Task<ShellMessage> {
        self.notification.toast_dirty = true;
        self.notification_refresh_toasts()
    }
    pub(crate) fn notification_escape(&mut self, id: window::Id) -> Task<ShellMessage> {
        if self
            .notification
            .toast
            .as_ref()
            .is_some_and(|pane| pane.window == id)
        {
            let keys = self
                .notification
                .toast_entries()
                .into_iter()
                .take(self.notification.toast_count)
                .map(|entry| entry.key)
                .collect();
            self.notification.send(Command::Read(keys));
            self.notification.clear_toast();
            return window::close(id);
        }
        self.close_notifications()
    }
    pub(crate) fn notification_opened(&mut self, id: window::Id) -> Task<ShellMessage> {
        if self
            .notification
            .center
            .as_ref()
            .is_some_and(|pane| pane.window == id)
        {
            if self.notification.content == Content::Activity {
                if self.notification.activity.passive {
                    if let Some(pane) = self.notification.center.as_mut() {
                        pane.visible = true;
                        pane.started = true;
                    }
                    return Task::none();
                }
                // Opened can arrive before the first buffer maps the Wayland window.
                return self.focus_notification_panel(ACTIVITY_FOCUS_RETRIES);
            }
            let refresh = if self.notification.content == Content::Calendar {
                self.clock_refresh()
            } else {
                Task::none()
            };
            Task::batch([window::gain_focus(id), refresh])
        } else {
            Task::none()
        }
    }
    pub(crate) fn notification_preferences(&mut self) -> Task<ShellMessage> {
        let preferences = std::sync::Arc::clone(&self.desired_settings.notifications);
        let game_active = preferences.game_mode_active(
            self.system
                .game_mode
                .as_ref()
                .map(enzui_shell::game_mode::Status::active),
            self.notification.effective_policy.as_ref(),
        );
        let policy = preferences.effective_policy(game_active);
        if self.notification.preferences.as_ref() == Some(&preferences)
            && self.notification.effective_policy.as_ref() == Some(&policy)
        {
            return Task::none();
        }
        self.notification.preferences = Some(preferences);
        self.notification.effective_policy = Some(policy.clone());
        if self.notification.ready() {
            self.notification.send(Command::Policy(policy));
        }
        self.notification.toast_dirty = true;
        let toast = self.notification_refresh_toasts();
        if self.notification.center.is_some() {
            Task::batch([toast, self.notification_prepare_center()])
        } else {
            toast
        }
    }
    pub(super) fn notification_read_visible(&mut self) {
        if self.notification.center.is_none() || self.notification.content != Content::Notifications
        {
            return;
        }
        let keys = self
            .notification
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .active
                    .iter()
                    .filter(|entry| entry.unread)
                    .map(|entry| entry.key)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !keys.is_empty() {
            self.notification.send(Command::Read(keys));
        }
    }
    fn focus_notification_panel(&self, retries: u8) -> Task<ShellMessage> {
        let compositor = self.compositor.clone();
        let serial = self.notification.center_serial;
        Task::perform(
            async move { compositor.focus_notification_center().await },
            move |result| message(Message::CenterFocused(serial, retries, result)),
        )
    }

    fn activity_focus_pending(&self, serial: u64) -> bool {
        self.notification.center_serial == serial
            && self.notification.content == Content::Activity
            && !self.notification.activity.passive
            && self
                .notification
                .center
                .as_ref()
                .is_some_and(|pane| pane.visible || !pane.started)
    }

    pub(super) fn notification_open_center(&mut self) -> Task<ShellMessage> {
        self.notification.cancel_center_actions();
        let activity = self.notification.content == Content::Activity;
        let open = if let Some(pane) = self.notification.center.as_mut() {
            if pane.started {
                pane.visible = true;
                self.focus_notification_panel(if activity { ACTIVITY_FOCUS_RETRIES } else { 0 })
            } else if activity {
                Task::none()
            } else {
                window::gain_focus(pane.window)
            }
        } else if self.notification.center_opening {
            Task::none()
        } else {
            self.notification.history = false;
            self.notification.page = 0;
            self.notification_prepare_center()
        };
        Task::batch([
            self.force_close_plugin(),
            self.force_close_overview(),
            self.close_controls_for_panel(),
            self.force_close_launcher(),
            open,
            self.notification_refresh_toasts(),
        ])
    }
    /// Re-places an open calendar popup after what it shows grew (several
    /// events or world clocks arriving after it opened); it never shrinks
    /// while open.
    pub(crate) fn refit_calendar_panel(&mut self, before: f32) -> Task<ShellMessage> {
        if self.notification.content != Content::Calendar
            || self.notification.center.is_none()
            || self.notification.center_opening
            || self.clock_panel_height() <= before + 0.5
        {
            return Task::none();
        }
        self.notification_prepare_center()
    }
    pub(super) fn notification_prepare_center(&mut self) -> Task<ShellMessage> {
        self.notification.center_serial = self.notification.center_serial.wrapping_add(1);
        self.notification.center_opening = true;
        let serial = self.notification.center_serial;
        let panel = self.panel;
        let trigger = match self.notification.content {
            Content::Calendar => "clock-trigger",
            Content::Notifications => "notifications-trigger",
            Content::Activity => "bar-activity",
        };
        enzui_shell::widget_bounds::bounds(panel, trigger).then(move |anchor| match anchor {
            Some(anchor) => Task::done(message(Message::Anchor(serial, Some(anchor)))),
            None => window::size(panel)
                .map(move |size| message(Message::Anchor(serial, Some(bar_center_anchor(size))))),
        })
    }
    fn select_notification_content(
        &mut self,
        content: Content,
        toggle: bool,
    ) -> Task<ShellMessage> {
        self.cancel_panel_action();
        let same = self.notification.content == content;
        if content == Content::Activity && !self.has_visible_activity() {
            return Task::none();
        }
        if same && self.notification.activity.passive {
            self.notification.activity.reset_popup();
            return self.notification_open_center();
        }
        if toggle
            && same
            && (self.notification.center_opening
                || self
                    .notification
                    .center
                    .as_ref()
                    .is_some_and(|pane| pane.visible || !pane.started))
        {
            return self.close_notifications();
        }
        let close = if !same {
            self.force_close_notifications()
        } else {
            Task::none()
        };
        self.notification.content = content;
        if self.notification.ready() {
            self.notification.error = None;
        }
        Task::batch([close, self.notification_open_center()])
    }
    pub(crate) fn close_notifications(&mut self) -> Task<ShellMessage> {
        self.notification.activity.reset_popup();
        self.notification.cancel_center_actions();
        self.notification.center_opening = false;
        self.notification.center_serial = self.notification.center_serial.wrapping_add(1);
        if let Some(center) = self.notification.center.as_mut() {
            center.visible = false;
            if center.started {
                return Task::none();
            }
        }
        self.finish_notifications()
    }
    pub(crate) fn force_close_notifications(&mut self) -> Task<ShellMessage> {
        if let Some(pane) = &self.notification.center {
            self.cancel_panel_action_for(pane.window);
        }
        self.finish_notifications()
    }
    pub(super) fn finish_notifications(&mut self) -> Task<ShellMessage> {
        self.notification.activity.reset_popup();
        self.notification.cancel_center_actions();
        self.notification.center_opening = false;
        self.notification.center_serial = self.notification.center_serial.wrapping_add(1);
        self.notification
            .center
            .take()
            .map_or_else(Task::none, |pane| self.finish_panel_close(pane.window))
    }
    pub(super) fn notification_track_leaving(&mut self) -> bool {
        let now = std::time::Instant::now();
        // Gone means no longer a toast at all, not just beyond the count the
        // window was last sized for.
        let current: Vec<_> = self
            .notification
            .toast_entries()
            .into_iter()
            .cloned()
            .collect();
        let window = self.notification.toast.is_some() && !self.notification.blocks_toasts();
        for entry in &self.notification.toast_shown {
            let id = (entry.key.epoch, entry.key.id);
            // A replaced notification keeps its toast; only one gone leaves.
            if window
                && !current.iter().any(|c| (c.key.epoch, c.key.id) == id)
                && self.notification.revealed.contains_key(&id)
            {
                self.notification.toast_leaving.entry(id).or_insert(now);
            }
        }
        let leave = self.toast_timing().leave;
        self.notification
            .toast_leaving
            .retain(|_, at| now.saturating_duration_since(*at) < leave);
        self.notification.toast_shown = self.notification.toast_display();
        !self.notification.toast_leaving.is_empty()
    }
    // Shown toasts that are being taken away start leaving now.
    fn notification_start_leaving(&mut self, keys: &[enzui_shell::notifications::model::Key]) {
        if self.notification.toast.is_none() || self.notification.blocks_toasts() {
            return;
        }
        let now = std::time::Instant::now();
        self.notification.toast_shown = self.notification.toast_display();
        for entry in &self.notification.toast_shown {
            let id = (entry.key.epoch, entry.key.id);
            if keys.contains(&entry.key) && self.notification.revealed.contains_key(&id) {
                self.notification.toast_leaving.entry(id).or_insert(now);
            }
        }
    }
    fn notification_action_with_token(
        &mut self,
        key: enzui_shell::notifications::model::Key,
        action: String,
    ) -> Task<ShellMessage> {
        if self.notification.action_pending(&key)
            || self.notification.current(key).is_none_or(|entry| {
                !entry
                    .draft
                    .actions
                    .iter()
                    .any(|candidate| candidate.key == action)
            })
        {
            return Task::none();
        }
        let Some(id) = self.notification.action_source(key) else {
            return Task::none();
        };
        let Some(ticket) = self
            .notification
            .actions
            .begin(self.notification.run, id, key, action)
        else {
            return Task::none();
        };
        window::request_activation_token(id).map(move |result| {
            message(Message::InvokeWith(
                ticket,
                super::model::Token(result.ok().map(iced::window::ActivationToken::into_raw)),
            ))
        })
    }
    pub(super) fn notification_invoke_with(
        &mut self,
        ticket: super::actions::Ticket,
        token: super::model::Token,
    ) -> Task<ShellMessage> {
        if matches!(
            self.notification.actions.intent(ticket),
            Some(super::actions::Intent::Activate(_))
        ) {
            return self.notification_activate_with(ticket, token);
        }
        let source = self.notification.action_source(ticket.key);
        let Some((key, action)) =
            self.notification
                .actions
                .finish(ticket, self.notification.run, source)
        else {
            return Task::none();
        };
        if self.notification.pending.contains_key(&key)
            || self
                .notification
                .reply
                .as_ref()
                .is_some_and(|reply| reply.key == key && !reply.text.is_empty())
            || self.notification.current(key).is_none_or(|entry| {
                !entry
                    .draft
                    .actions
                    .iter()
                    .any(|candidate| candidate.key == action)
            })
        {
            return Task::none();
        }
        if self.notification.send(Command::Action {
            key,
            action: Action::Invoke(action),
            token: token.0,
        }) {
            self.notification.pending.insert(key, false);
        }
        Task::none()
    }
    pub(super) fn notification_activate_with(
        &mut self,
        ticket: super::actions::Ticket,
        token: super::model::Token,
    ) -> Task<ShellMessage> {
        let Some(entry) = self.notification.current(ticket.key).cloned() else {
            self.notification.actions.retire(ticket);
            return Task::none();
        };
        if self.notification.pending.contains_key(&ticket.key)
            || self
                .notification
                .reply
                .as_ref()
                .is_some_and(|reply| reply.key == ticket.key && !reply.text.is_empty())
            || entry
                .draft
                .actions
                .iter()
                .any(|action| action.key == "default")
        {
            self.notification.actions.retire(ticket);
            return Task::none();
        }
        let source = self.notification.action_source(ticket.key);
        let run = self.notification.run;
        let Some((target, live)) = self
            .notification
            .actions
            .dispatch_activation(ticket, run, source)
        else {
            return Task::none();
        };
        let Some(authority) = self.notification.authority.clone() else {
            self.notification.actions.retire(ticket);
            return Task::none();
        };
        let compositor = self.compositor.clone();
        let root = self.root.clone();
        Task::perform(
            async move {
                enzui_shell::notifications::activation::activate(
                    &compositor,
                    &root,
                    entry.origin.clone(),
                    target,
                    token.0,
                    live,
                    authority,
                )
                .await
            },
            move |result| message(Message::Activated(ticket, result)),
        )
    }
    /// Pauses or resumes toasts. Pausing closes an open toast window at once
    /// (no leave animation) and reports whether one was open.
    pub(crate) fn pause_toasts(&mut self, paused: bool) -> (bool, Task<ShellMessage>) {
        self.notification.toasts_paused = paused;
        if !paused {
            return (false, self.notification_refresh_toasts());
        }
        self.notification.toast_serial = self.notification.toast_serial.wrapping_add(1);
        self.notification.toast_opening = false;
        match self.notification.clear_toast() {
            Some(pane) => (true, window::close(pane.window)),
            None => (false, Task::none()),
        }
    }
    pub(crate) fn notification_refresh_toasts(&mut self) -> Task<ShellMessage> {
        if self.notification_track_leaving() {
            // Resize once they have gone; until then the window keeps room.
            self.notification.toast_dirty = true;
            let leave = self.toast_timing().leave;
            let wait = self
                .notification
                .toast_leaving
                .values()
                .map(|at| leave.saturating_sub(at.elapsed()))
                .max()
                .unwrap_or_default();
            return Task::perform(async move { tokio::time::sleep(wait).await }, |()| {
                message(Message::LeftToasts)
            });
        }
        let count = self
            .notification
            .toast_entries()
            .len()
            .min(self.desired_settings.notifications.maximum_toasts as usize);
        if count == 0 {
            self.notification.toast_serial = self.notification.toast_serial.wrapping_add(1);
            self.notification.toast_opening = false;
            return self
                .notification
                .clear_toast()
                .map_or_else(Task::none, |pane| window::close(pane.window));
        }
        self.notification.sync_hold(false);
        if self.notification.toast_opening {
            self.notification.toast_dirty = true;
            return Task::none();
        }
        let heights: Vec<f32> = self
            .notification
            .toast_entries()
            .into_iter()
            .take(count)
            .map(|entry| self.notification_card_height(entry))
            .collect();
        let overshoot = self.toast_overshoot();
        if !self.notification.toast_dirty
            && count == self.notification.toast_requested
            && overshoot == self.notification.toast_overshoot
            && heights == self.notification.toast_heights
            && self.notification.toast.is_some()
        {
            // The window fits already (a newcomer the same height as the one
            // it replaces): it still has to slide in.
            return self.notification_schedule_reveals(false);
        }
        self.notification.toast_dirty = false;
        self.notification.toast_opening = true;
        let mut stack = [0.0; 5];
        for (slot, height) in stack.iter_mut().zip(&heights) {
            *slot = *height;
        }
        // Two or more toasts get a dismiss-all button under the last one.
        if count >= 2 {
            stack[count - 1] += enzui_shell::notifications::card::DISMISS_ALL_ROW;
        }
        // An open window never shrinks: a smaller window is drawn for a frame
        // with the old picture centred in it. Its spare room takes no input.
        if let Some(pane) = &self.notification.toast {
            let used: f32 = stack[..count].iter().sum::<f32>()
                + enzui_shell::notifications::card::STACK_GAP * (count.saturating_sub(1)) as f32
                + 2.0 * enzui_shell::notifications::card::SHADOW_MARGIN;
            let floor = pane.geometry.height;
            if used < floor {
                stack[count - 1] += floor - used;
            }
        }
        self.notification.toast_heights = heights;
        self.notification.toast_serial = self.notification.toast_serial.wrapping_add(1);
        let serial = self.notification.toast_serial;
        let (left, right) = self.desktop_edges();
        let placement = ToastPlacement {
            bar: self.bar,
            metrics: self.metrics,
            left,
            right,
            position: self.desired_settings.notifications.position,
            width: f32::from(self.desired_settings.notifications.width),
            heights: stack,
            count,
            overshoot,
        };
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.prepare_notification_toasts(placement).await },
            move |result| message(Message::ToastPrepared(serial, count, overshoot, result)),
        )
    }
    // Slides the shown toasts that have not appeared yet in, one after
    // another, top first; a newly opened window first gets time to map.
    fn notification_schedule_reveals(&mut self, opened: bool) -> Task<ShellMessage> {
        let timing = self.toast_timing();
        let shown: Vec<_> = self
            .notification
            .toast_entries()
            .into_iter()
            .take(self.notification.toast_count)
            .map(|entry| (entry.key.epoch, entry.key.id))
            .collect();
        let active: std::collections::BTreeSet<_> = self
            .notification
            .snapshot
            .iter()
            .flat_map(|snapshot| {
                snapshot
                    .active
                    .iter()
                    .map(|entry| (entry.key.epoch, entry.key.id))
            })
            .collect();
        self.notification
            .revealed
            .retain(|key, _| active.contains(key));
        self.notification
            .reveal_scheduled
            .retain(|key| active.contains(key));
        let now = std::time::Instant::now();
        if timing.reduced {
            for key in shown {
                self.notification.reveal_scheduled.insert(key);
                self.notification
                    .revealed
                    .entry(key)
                    .or_insert((now, false));
            }
            self.notification.reveal_last = None;
            return Task::none();
        }
        let mut at = now
            + if opened {
                timing.mapping
            } else {
                timing.update
            };
        if let Some(last) = self.notification.reveal_last {
            at = at.max(last + timing.step);
        }
        let mut tasks = Vec::new();
        for key in shown {
            if !self.notification.reveal_scheduled.insert(key) {
                continue;
            }
            self.notification.reveal_last = Some(at);
            tasks.push(Task::perform(
                async move { tokio::time::sleep_until(at.into()).await },
                move |()| message(Message::Reveal(key)),
            ));
            at += timing.step;
        }
        Task::batch(tasks)
    }
    pub(crate) fn notification_message(&mut self, event: Message) -> Task<ShellMessage> {
        let task = self.notification_event(event);
        Task::batch([task, self.refit_notification_center()])
    }
    fn notification_event(&mut self, event: Message) -> Task<ShellMessage> {
        match event {
            Message::Runtime(event) if event.run != self.notification.run => Task::none(),
            Message::Runtime(event) => match event.update {
                Update::Ready {
                    epoch,
                    commands,
                    authority,
                } => {
                    self.notification.actions.cancel();
                    self.notification.epoch = Some(epoch);
                    self.notification.channel = Some(commands);
                    self.notification.authority = Some(authority);
                    self.notification.error = None;
                    if let Some(policy) = &self.notification.effective_policy {
                        self.notification.send(Command::Policy(policy.clone()));
                    }
                    Task::none()
                }
                Update::Snapshot(snapshot) => {
                    if self.notification.epoch != Some(snapshot.epoch) {
                        return Task::none();
                    }
                    let keys: Vec<_> = snapshot.active.iter().map(|entry| entry.key).collect();
                    self.notification.snapshot = Some(snapshot);
                    let actionable: Vec<_> = keys
                        .iter()
                        .copied()
                        .filter(|key| self.notification.action_source(*key).is_some())
                        .collect();
                    self.notification.actions.retain_keys(&actionable);
                    let visible = self
                        .notification
                        .toast_entries()
                        .first()
                        .map(|entry| entry.key);
                    self.notification.arrivals.sync(keys, visible);
                    self.notification_read_visible();
                    self.notification_refresh_toasts()
                }
                Update::Storage(result) => {
                    self.notification.storage_error = result.err();
                    Task::none()
                }
                Update::Operation(server::Event::Action { key, result }) => {
                    let reply = self.notification.pending.remove(&key) == Some(true);
                    if let Err(error) = result {
                        self.notification.error = Some(error);
                    } else if reply
                        && self
                            .notification
                            .reply
                            .as_ref()
                            .is_some_and(|draft| draft.key == key)
                    {
                        self.notification.reply = None;
                        self.notification.error = None;
                    }
                    Task::none()
                }
                Update::Operation(
                    server::Event::Policy(result) | server::Event::PostedCapture(result),
                ) => {
                    if let Err(error) = result {
                        self.notification.error = Some(error);
                    }
                    Task::none()
                }
                Update::Operation(server::Event::Ready(_)) => Task::none(),
                Update::Stopped(error) => {
                    if let Some(authority) = self.notification.authority.take() {
                        authority.store(false, Ordering::Release);
                    }
                    self.notification.channel = None;
                    self.notification.epoch = None;
                    self.notification.snapshot = None;
                    self.notification.arrivals = Arrivals::default();
                    self.notification.pending.clear();
                    self.notification.actions.cancel();
                    self.notification.error = Some(error);
                    self.notification_refresh_toasts()
                }
            },
            Message::Shortcut(enzui_shell::shortcuts::Event::ReceiverInstalled(receiver)) => {
                self.search.receiver = Some(receiver);
                let compositor = self.compositor.clone();
                Task::batch([
                    Task::perform(
                        async move { compositor.install_notification_shortcut(receiver).await },
                        |result| message(Message::ShortcutBound(result)),
                    ),
                    self.search_bind(),
                ])
            }
            Message::Shortcut(enzui_shell::shortcuts::Event::OpenNotifications) => Task::batch([
                self.close_search(),
                self.select_notification_content(Content::Notifications, false),
            ]),
            Message::Shortcut(enzui_shell::shortcuts::Event::OpenSearch) => {
                self.search_message(crate::quick_search::Message::Toggle)
            }
            Message::Shortcut(enzui_shell::shortcuts::Event::Unavailable(error)) => {
                self.notification.shortcut_error = Some(error);
                Task::none()
            }
            Message::ShortcutBound(result) => {
                self.notification.shortcut_error = result.err();
                Task::none()
            }
            Message::CenterFocused(serial, retries, result) => {
                let Err(error) = result else {
                    return Task::none();
                };
                let activity = self.activity_focus_pending(serial);
                if activity && let Some(remaining) = retries.checked_sub(1) {
                    self.quiet = true;
                    return Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        },
                        move |()| message(Message::RetryActivityFocus(serial, remaining)),
                    );
                }
                if activity
                    || (serial == self.notification.center_serial
                        && self
                            .notification
                            .center
                            .as_ref()
                            .is_some_and(|pane| pane.visible))
                {
                    self.notification.error = Some(error);
                }
                Task::none()
            }
            Message::RetryActivityFocus(serial, remaining) => {
                self.quiet = true;
                if self.activity_focus_pending(serial) {
                    self.focus_notification_panel(remaining)
                } else {
                    Task::none()
                }
            }
            Message::Toggle => self.select_notification_content(Content::Notifications, true),
            Message::ToggleCalendar => self.select_notification_content(Content::Calendar, true),
            Message::ToggleActivity => self.select_notification_content(Content::Activity, true),
            Message::ActivityTriggerHover(inside) => self.activity_hover(true, inside),
            Message::ActivityHoverElapsed(revision) => self.activity_hover_elapsed(revision),
            Message::ActivityPointerChecked(revision, inside) => {
                self.activity_pointer_checked(revision, inside)
            }
            Message::Hidden(id) => {
                if self
                    .notification
                    .center
                    .as_ref()
                    .is_some_and(|pane| pane.window == id && !pane.visible)
                {
                    self.finish_notifications()
                } else {
                    Task::none()
                }
            }
            Message::Anchor(serial, anchor) => {
                if serial != self.notification.center_serial || !self.notification.center_opening {
                    return Task::none();
                }
                let (left, right) = self.desktop_edges();
                let Some(anchor) = anchor else {
                    self.notification.center_opening = false;
                    self.notification.activity.reset_popup();
                    self.notification.error =
                        Some(tr("shell-notification-center-anchor-is-unavailable").into());
                    return Task::none();
                };
                if self.notification.content == Content::Activity {
                    self.notification.activity.trigger_bounds = Some(anchor);
                }
                let placement = PopupPlacement {
                    bar: self.bar,
                    left,
                    right,
                    metrics: self.metrics,
                    anchor: Some(anchor),
                    body: if self.notification.content == Content::Calendar {
                        iced::Size::new(392.0, self.clock_panel_height())
                    } else if self.notification.content == Content::Activity {
                        self.activity_panel_size()
                    } else {
                        self.notification.center_requested = self.notification_center_height();
                        iced::Size::new(
                            (f32::from(self.desired_settings.notifications.width) + 64.0)
                                .min(640.0),
                            self.notification.center_requested,
                        )
                    },
                };
                let compositor = self.compositor.clone();
                // Only a center that is on screen and staying is held in
                // place; one that is closing is about to go.
                let open = self
                    .notification
                    .center
                    .as_ref()
                    .is_some_and(|pane| pane.visible);
                let passive = self.notification.activity.passive;
                Task::perform(
                    async move {
                        compositor
                            .prepare_notification_center(placement, open, passive)
                            .await
                    },
                    move |result| message(Message::CenterPrepared(serial, result)),
                )
            }
            Message::CenterPrepared(serial, result) => {
                if serial != self.notification.center_serial || !self.notification.center_opening {
                    return Task::none();
                }
                self.notification.center_opening = false;
                match result {
                    Err(error) => {
                        self.notification.activity.reset_popup();
                        self.notification.error = Some(error);
                        Task::none()
                    }
                    Ok(geometry) => {
                        let size = self.popover_window(&geometry);
                        if let Some(pane) = self.notification.center.as_mut() {
                            pane.geometry = geometry;
                            return resize_chrome(pane.window, size);
                        }
                        let (id, open) = window::open(window_settings(
                            "dev.enzui.NotificationCenter",
                            size.width,
                            size.height,
                        ));
                        self.notification.center = Some(Pane {
                            window: id,
                            geometry,
                            visible: false,
                            started: false,
                        });
                        self.notification_read_visible();
                        open.map(ShellMessage::Opened)
                    }
                }
            }
            Message::ToastPrepared(serial, count, overshoot, result) => {
                if serial != self.notification.toast_serial || !self.notification.toast_opening {
                    return Task::none();
                }
                self.notification.toast_opening = false;
                match result {
                    Err(error) => {
                        self.notification.error = Some(error);
                        Task::none()
                    }
                    Ok(geometry) => {
                        let size = iced::Size::new(geometry.width, geometry.height);
                        let fitted = enzui_shell::compositor::fitted_toasts(
                            &self.notification.toast_heights,
                            geometry.height,
                        )
                        .max(1);
                        self.notification.toast_count = count.min(fitted);
                        self.notification.toast_shown = self.notification.toast_display();
                        self.notification.toast_requested = count;
                        self.notification.toast_overshoot = overshoot;
                        let task_kind = if self.notification.toast.is_some() {
                            TaskKind::Resized
                        } else {
                            TaskKind::Opened
                        };
                        let task = if let Some(pane) = self.notification.toast.as_mut() {
                            pane.geometry = geometry;
                            resize_chrome(pane.window, size)
                        } else {
                            let (id, open) = window::open(window_settings(
                                "dev.enzui.NotificationToasts",
                                size.width,
                                size.height,
                            ));
                            self.notification.toast = Some(Pane {
                                window: id,
                                geometry,
                                visible: true,
                                started: true,
                            });
                            open.map(ShellMessage::Opened)
                        };
                        self.notification.sync_hold(false);
                        let reveal = self
                            .notification_schedule_reveals(matches!(task_kind, TaskKind::Opened));
                        if self.notification.toast_dirty {
                            Task::batch([task, reveal, self.notification_refresh_toasts()])
                        } else {
                            Task::batch([task, reveal])
                        }
                    }
                }
            }
            Message::PopupHover(id, inside)
                if self.notification.content == Content::Activity
                    && self.notification.owns_center(id) =>
            {
                self.activity_hover(false, inside)
            }
            Message::PopupHover(id, hovered) => {
                if self
                    .notification
                    .toast
                    .as_ref()
                    .is_some_and(|pane| pane.window == id)
                {
                    self.notification.toast_hovered = hovered;
                    self.notification.sync_hold(false);
                }
                Task::none()
            }
            Message::Reveal(key) => {
                if !self.notification.revealed.contains_key(&key) {
                    let push = self.notification.shown_revealed();
                    self.notification
                        .revealed
                        .insert(key, (std::time::Instant::now(), push));
                }
                Task::none()
            }
            Message::SlideFrame => {
                self.redraw_only = self.notification.toast.as_ref().map(|pane| pane.window);
                Task::none()
            }
            Message::LeftToasts => self.notification_refresh_toasts(),
            Message::RenewToasts => {
                self.notification.sync_hold(true);
                Task::none()
            }
            Message::Dnd(value) => {
                self.save_setting(SettingChange::Notification(Change::Dnd(value)))
            }
            Message::Retry => {
                if let Some(authority) = self.notification.authority.take() {
                    authority.store(false, Ordering::Release);
                }
                self.notification.run = self.notification.run.wrapping_add(1);
                self.notification.channel = None;
                self.notification.snapshot = None;
                self.notification.arrivals = Arrivals::default();
                self.notification.epoch = None;
                self.notification.pending.clear();
                self.notification.actions.cancel();
                self.notification.error = None;
                self.notification_refresh_toasts()
            }
            Message::History(history) => {
                self.notification.actions.cancel();
                self.notification.history = history;
                self.notification.page = 0;
                Task::none()
            }
            Message::HistoryQuery(query) => {
                if query.chars().count() > 128 {
                    self.notification.error =
                        Some(tr("shell-history-search-is-limited-to-128-characters").into());
                    return Task::none();
                }
                self.notification.history_query = query;
                self.notification.page = 0;
                self.notification.expanded_history = None;
                Task::none()
            }
            Message::HistoricalDetails(id) => {
                self.notification.expanded_history =
                    if self.notification.expanded_history == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                Task::none()
            }
            Message::Page(page) => {
                self.notification.actions.cancel();
                self.notification.page = page.min(32);
                Task::none()
            }
            Message::Expand(key) => {
                self.notification.expanded = if self.notification.expanded == Some(key) {
                    None
                } else {
                    Some(key)
                };
                Task::none()
            }
            Message::Reply(key) => {
                if self.notification.action_pending(&key) {
                    return Task::none();
                }
                if self
                    .notification
                    .current(key)
                    .is_none_or(|entry| entry.draft.reply.is_none())
                {
                    return Task::none();
                }
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_some_and(|reply| reply.key != key && !reply.text.is_empty())
                {
                    self.notification.error = Some(
                        tr("shell-send-or-discard-the-existing-reply-before-starting-another")
                            .into(),
                    );
                    return Task::none();
                }
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_none_or(|reply| reply.key != key)
                {
                    self.notification.reply = Some(Reply {
                        key,
                        text: String::new(),
                    });
                }
                // A shown toast takes the reply itself; the centre is only
                // for messages that are no longer on screen.
                let shown = self.notification.toast.is_some()
                    && !self.notification.blocks_toasts()
                    && self
                        .notification
                        .toast_entries()
                        .iter()
                        .any(|entry| entry.key == key);
                if shown {
                    self.notification.error = None;
                    let window = self.notification.toast.as_ref().map(|pane| pane.window);
                    let refresh = self.notification_refresh_toasts();
                    return Task::batch([
                        refresh,
                        window.map_or_else(Task::none, |id| {
                            window::gain_focus(id).chain(iced_runtime::widget::operation::focus(
                                "notification-toast-reply",
                            ))
                        }),
                    ]);
                }
                self.notification.history = false;
                self.notification.page = self
                    .notification
                    .current_entries()
                    .iter()
                    .position(|entry| entry.key == key)
                    .unwrap_or(0)
                    / 12;
                self.notification.error = None;
                self.notification.focus_reply = true;
                if let Some(pane) = &self.notification.center {
                    let id = pane.window;
                    self.notification.focus_reply = false;
                    window::gain_focus(id)
                        .chain(iced_runtime::widget::operation::focus("notification-reply"))
                } else {
                    Task::batch([
                        self.force_close_plugin(),
                        self.close_controls_for_panel(),
                        self.force_close_overview(),
                        self.force_close_launcher(),
                        self.notification_prepare_center(),
                        self.notification_refresh_toasts(),
                    ])
                }
            }
            Message::EditReply(key, text) => {
                if text.len() > 16384 || text.chars().count() > 4096 || text.contains('\0') {
                    self.notification.error =
                        Some(tr("shell-reply-exceeds-4096-characters-or-contains-a-nul").into());
                    return Task::none();
                }
                if !self.notification.action_pending(&key)
                    && let Some(reply) = self
                        .notification
                        .reply
                        .as_mut()
                        .filter(|reply| reply.key == key)
                {
                    reply.text = text;
                }
                Task::none()
            }
            Message::SendReply(key) => {
                if self.notification.action_pending(&key)
                    || self.notification.current(key).is_none()
                {
                    self.notification.error = Some(
                        tr("shell-notification-changed-or-closed-the-draft-was-not-sent").into(),
                    );
                    return Task::none();
                }
                if let Some(reply) = self
                    .notification
                    .reply
                    .as_ref()
                    .filter(|reply| reply.key == key && !reply.text.trim().is_empty())
                {
                    let command = Command::Action {
                        key,
                        action: Action::Reply(reply.text.clone()),
                        token: None,
                    };
                    if self.notification.send(command) {
                        self.notification.pending.insert(key, true);
                    }
                }
                Task::none()
            }
            Message::DiscardReply => {
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_none_or(|reply| !self.notification.action_pending(&reply.key))
                {
                    self.notification.reply = None;
                    self.notification.error = None;
                }
                self.notification_refresh_toasts()
            }
            Message::Invoke(key, action) => {
                if self.notification.action_pending(&key) {
                    return Task::none();
                }
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_some_and(|reply| reply.key == key && !reply.text.is_empty())
                {
                    self.notification.error =
                        Some(tr("shell-send-or-discard-the-reply-before-another-action").into());
                    return Task::none();
                }
                self.notification_action_with_token(key, action)
            }
            Message::InvokeWith(ticket, token) => self.notification_invoke_with(ticket, token),
            Message::DismissAll => {
                // A notification with a running action or an unsent reply keeps
                // its card, as a single dismiss would.
                let keys: Vec<_> = self
                    .notification
                    .current_entries()
                    .into_iter()
                    .map(|entry| entry.key)
                    .filter(|key| {
                        !self.notification.action_pending(key)
                            && self
                                .notification
                                .reply
                                .as_ref()
                                .is_none_or(|reply| reply.key != *key || reply.text.is_empty())
                    })
                    .collect();
                let sent: Vec<_> = keys
                    .into_iter()
                    .filter(|key| self.notification.send(Command::Dismiss(*key)))
                    .collect();
                self.notification_start_leaving(&sent);
                Task::none()
            }
            Message::Dismiss(key) => {
                if self.notification.action_pending(&key) {
                    return Task::none();
                }
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_some_and(|reply| reply.key == key && !reply.text.is_empty())
                {
                    self.notification.error = Some(
                        tr("shell-discard-the-reply-before-dismissing-this-notification").into(),
                    );
                    return Task::none();
                }
                // The card starts leaving at the click, not when the server
                // has answered.
                if self.notification.send(Command::Dismiss(key)) {
                    self.notification_start_leaving(&[key]);
                }
                Task::none()
            }
            Message::Activate(key) => {
                let Some(entry) = self.notification.current(key).cloned() else {
                    return Task::none();
                };
                if self.notification.action_source(key).is_none()
                    || self.notification.action_pending(&key)
                {
                    return Task::none();
                }
                let default = entry
                    .draft
                    .actions
                    .iter()
                    .any(|action| action.key == "default");
                if default {
                    return self.notification_action_with_token(key, "default".into());
                }
                if self
                    .notification
                    .reply
                    .as_ref()
                    .is_some_and(|reply| reply.key == key && !reply.text.is_empty())
                {
                    self.notification.error =
                        Some(tr("shell-send-or-discard-the-reply-before-opening-this-app").into());
                    return Task::none();
                }
                let target =
                    match enzui_shell::notifications::activation::target(&self.root, &entry.origin)
                    {
                        Ok(target) => target,
                        Err(error) => {
                            self.notification.error = Some(error);
                            return Task::none();
                        }
                    };
                let Some(id) = self.notification.action_source(key) else {
                    return Task::none();
                };
                let Some(ticket) = self.notification.actions.begin_intent(
                    self.notification.run,
                    id,
                    key,
                    super::actions::Intent::Activate(target),
                ) else {
                    return Task::none();
                };
                window::request_activation_token(id).map(move |result| {
                    message(Message::InvokeWith(
                        ticket,
                        super::model::Token(
                            result.ok().map(iced::window::ActivationToken::into_raw),
                        ),
                    ))
                })
            }
            Message::Activated(ticket, result) => {
                if self.notification.actions.completed(ticket) {
                    match result {
                        Ok(()) => {
                            self.notification.send(Command::Read(vec![ticket.key]));
                        }
                        Err(error) => self.notification.error = Some(error),
                    }
                }
                Task::none()
            }
            Message::HideToasts => {
                let keys: Vec<_> = self
                    .notification
                    .toast_entries()
                    .into_iter()
                    .map(|entry| entry.key)
                    .collect();
                if !keys.is_empty() && self.notification.send(Command::Read(keys.clone())) {
                    self.notification.actions.cancel();
                    self.notification_start_leaving(&keys);
                }
                Task::none()
            }
            Message::ClearHistory => {
                self.save_setting(SettingChange::Notification(Change::ClearHistory))
            }
        }
    }
    pub(crate) fn notification_window(
        &mut self,
        id: window::Id,
        event: &window::Event,
    ) -> Task<ShellMessage> {
        if self
            .notification
            .center
            .as_ref()
            .is_some_and(|pane| pane.window == id)
        {
            match event {
                window::Event::Focused => {
                    if self.notification.content == Content::Activity {
                        self.notification.activity.reset_popup();
                    }
                    if let Some(pane) = self.notification.center.as_mut() {
                        pane.visible = true;
                        pane.started = true;
                    }
                    if self.notification.focus_reply {
                        self.notification.focus_reply = false;
                        return iced_runtime::widget::operation::focus("notification-reply");
                    }
                }
                window::Event::CloseRequested => return self.close_notifications(),
                window::Event::Closed => {
                    self.notification.activity.reset_popup();
                    self.notification.cancel_center_actions();
                    self.notification.center = None;
                    return self.notification_refresh_toasts();
                }
                _ => (),
            }
        } else if self
            .notification
            .toast
            .as_ref()
            .is_some_and(|pane| pane.window == id)
        {
            match event {
                window::Event::Focused | window::Event::Unfocused => {
                    self.notification.toast_focused = matches!(event, window::Event::Focused);
                    self.notification.sync_hold(false);
                }
                window::Event::CloseRequested | window::Event::Closed => {
                    self.notification.clear_toast();
                    return window::close(id);
                }
                _ => (),
            }
        }
        Task::none()
    }
}

enum TaskKind {
    Opened,
    Resized,
}
