use super::geometry::BLANK_HEIGHT;

/// How long the card holds its height after a key for the results other
/// sources send (190 ms, plus the resize, lagged behind typing).
const HEIGHT_SETTLE: Duration = Duration::from_millis(60);
use super::{
    model::Message,
    support::{
        clipboard_connection, focus_if_unfocused, focus_query_unless_source_focused, message,
        perform, source_chord,
    },
};
use crate::{Message as ShellMessage, Shell, notification_panel};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    search::{self, Category, DesktopAction, Item, Target, Ticket, preferences::Change},
    settings::Change as SettingChange,
};
use iced::{Task, window};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// How far apart the compositor's report of a press and the press reaching
// Search's window may be and still be one press.
const OUTSIDE_WINDOW: Duration = Duration::from_millis(250);
// How long to wait for the window's own report after the compositor's.
const OUTSIDE_SETTLE: Duration = Duration::from_millis(120);
// The longest press that still ends in the click that closed Search.
const CLICK_HOLD: Duration = Duration::from_secs(2);

impl Shell {
    /// The bar's Applications click. A press held past the outside check has
    /// already closed Search, and its click must not open it again.
    pub(crate) fn search_trigger(&mut self) -> Task<ShellMessage> {
        let closed_by = self.search.closed_by.take();
        if !self.search.showing
            && closed_by.is_some_and(|press| {
                self.search.bar_press == Some(press) && press.elapsed() < CLICK_HOLD
            })
        {
            return Task::none();
        }
        self.search_message(Message::Toggle)
    }

    pub(crate) fn search_pointer_press(&mut self) -> Task<ShellMessage> {
        if !self.search.showing || !self.search.started {
            return Task::none();
        }
        let epoch = self.search.epoch;
        let at = Instant::now();
        perform(
            async move { tokio::time::sleep(OUTSIDE_SETTLE).await },
            move |()| Message::OutsideCheck(epoch, at),
        )
    }
}

impl Shell {
    pub(crate) fn search_message(&mut self, event: Message) -> Task<ShellMessage> {
        match event {
            Message::FocusChecked(epoch, revision, kept)
                if epoch == self.search.epoch && revision == self.search.focus_revision =>
            {
                if kept {
                    Task::none()
                } else {
                    self.close_search()
                }
            }
            Message::Toggle if self.search.showing => self.close_search(),
            Message::Toggle => self.search_message(Message::Show(None)),
            Message::Show(category) => {
                if !self.search.configured || !self.search.preferences.enabled || self.bar_busy {
                    return Task::none();
                }
                self.search.epoch = self.search.epoch.saturating_add(1);
                self.search.showing = true;
                self.search.overview = self.activities;
                self.search.activation = None;
                self.search.activation_checked = false;
                self.search.activation_tried.clear();
                self.search.overview_refresh_pending = self.search.overview;
                self.search.preparing = true;
                self.search.presented = false;
                if self.metrics.islands.is_some() {
                    self.search.visible = false;
                    self.search.started = false;
                }
                if let Some(live) = &self.search.presentation {
                    live.store(false, Ordering::Release);
                }
                self.search.presentation = Some(Arc::new(AtomicBool::new(true)));
                self.search.action_busy = false;
                self.search.copy_epoch = None;
                self.search.feedback = None;
                self.search.alias = None;
                self.search.preedit = None;
                self.search.browse_all_apps = false;
                self.search.app_scroll_carry = 0.0;
                self.search.model.error = None;
                self.search.model.set_query(String::new());
                self.search.model.set_category(category);
                self.search.file_query = None;
                self.search.file_results.clear();
                self.search_recompute();
                let epoch = self.search.epoch;
                let refresh = if self.search.overview {
                    Task::none()
                } else {
                    self.refresh_search_sources()
                };
                Task::batch([
                    refresh,
                    self.close_controls_for_panel(),
                    self.force_close_overview(),
                    self.force_close_notifications(),
                    self.force_close_plugin(),
                    self.force_close_launcher(),
                    if self.search.overview {
                        Task::done(message(Message::Anchor(epoch, None)))
                    } else {
                        enzui_shell::widget_bounds::bounds(self.panel, "applications")
                            .map(move |rect| message(Message::Anchor(epoch, rect)))
                    },
                ])
            }
            Message::Anchor(epoch, anchor) if epoch == self.search.epoch && self.search.showing => {
                let compositor = self.compositor.clone();
                let metrics = self.metrics;
                let edge = self.bar;
                perform(
                    async move { compositor.search_geometry(metrics, edge, anchor).await },
                    move |result| Message::Prepared(epoch, result),
                )
            }
            Message::WarmPrepared(bar_epoch, epoch, Ok(geometry))
                if bar_epoch == self.bar_applied_epoch
                    && epoch == self.search.epoch
                    && !self.bar_busy
                    && self.search.preferences.enabled
                    && !self.search.showing
                    && self.search.window.is_none() =>
            {
                self.search.epoch = self.search.epoch.saturating_add(1);
                let size = self.set_search_geometry(geometry);
                self.create_search_window(size)
            }
            Message::RefreshApps(epoch) if epoch == self.search.epoch && self.search.showing => {
                self.search_scan_apps(true)
            }
            Message::Prepared(epoch, result)
                if epoch == self.search.epoch && self.search.showing =>
            {
                match result {
                    Ok(geometry) => {
                        let output = geometry.output.clone();
                        let size = self.set_search_geometry(geometry);
                        self.search.preparing = false;
                        self.search.layout_busy = false;
                        self.search.layout_revision = self.search.layout_revision.saturating_add(1);
                        let open = match self.search.window {
                            Some(id) if !self.surface_inputs.contains_key(&id) => {
                                Task::done(ShellMessage::Opened(id))
                            }
                            Some(_) => Task::none(),
                            None => self.create_search_window(size),
                        };
                        let compositor = self.compositor.clone();
                        let live = self
                            .search
                            .presentation
                            .clone()
                            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
                        let placement = self.search.island;
                        let fade = self.search.preferences.background_fade
                            && !self.tokens().motion.reduced_motion;
                        let present = perform(
                            async move {
                                compositor
                                    .present_search(&output, size, epoch, live, placement, fade)
                                    .await
                            },
                            move |result| Message::Presented(epoch, result),
                        );
                        Task::batch([open, present])
                    }
                    Err(error) => {
                        self.search.feedback = Some(error.clone());
                        self.notice = Some(error);
                        self.close_search()
                    }
                }
            }
            Message::Presented(epoch, result)
                if epoch == self.search.epoch && self.search.showing =>
            {
                if let Err(error) = result {
                    eprintln!("quick-search presentation: {error}");
                    self.notice = Some(error);
                    self.search.showing = false;
                    self.search.overview = false;
                    self.search.visible = false;
                    self.search.started = false;
                    return self
                        .search
                        .window
                        .take()
                        .map_or_else(Task::none, window::close);
                }
                self.search.presented = true;
                let resize = self
                    .search
                    .window
                    .zip(self.search.surface_size)
                    .map_or_else(Task::none, |(id, size)| crate::resize_chrome(id, size));
                Task::batch([resize, self.search_reveal_ready()])
            }
            Message::Resized(epoch, revision, result)
                if epoch == self.search.epoch && revision == self.search.layout_revision =>
            {
                self.search.layout_busy = false;
                if let Err(error) = result {
                    self.notice = Some(error);
                    return self.close_search();
                }
                self.search
                    .window
                    .zip(self.search.surface_size)
                    .map_or_else(Task::none, |(id, size)| crate::resize_chrome(id, size))
            }
            Message::Dismiss => self.close_search(),
            Message::FocusInput if self.search.showing && self.search.alias.is_none() => {
                self.search.window.map_or_else(Task::none, |window| {
                    iced_runtime::task::widget_in(window, focus_query_unless_source_focused())
                })
            }
            Message::Close if self.search.preedit.is_none() => {
                if self.search.alias.is_some() {
                    self.search_message(Message::AliasCancel)
                } else {
                    self.close_search()
                }
            }
            Message::DimHidden(epoch, Err(error)) if epoch == self.search.epoch => {
                self.notice = Some(error);
                Task::none()
            }
            Message::Hidden(id, epoch)
                if self.search.window == Some(id)
                    && epoch == self.search.epoch
                    && !self.search.showing =>
            {
                self.search.started = false;
                self.search.overview = false;
                self.search.model.set_query(String::new());
                self.search.model.results.clear();
                self.search.model.selected = None;
                let compositor = self.compositor.clone();
                perform(
                    async move { compositor.park_search(epoch).await },
                    move |result| Message::Parked(epoch, result),
                )
            }
            Message::Parked(epoch, Err(error)) if epoch == self.search.epoch => {
                self.notice = Some(error);
                if !self.search.showing {
                    self.search
                        .window
                        .take()
                        .map_or_else(Task::none, window::close)
                } else {
                    Task::none()
                }
            }
            Message::Preedit {
                window,
                epoch,
                alias,
                text,
            } if self.search.showing
                && self.search.window == Some(window)
                && self.search.epoch == epoch
                && alias == self.search.alias.as_ref().map(|value| value.0.clone()) =>
            {
                let text = text.filter(|value| !value.is_empty()).map(|value| {
                    value
                        .chars()
                        .take(search::MAX_QUERY_CHARS)
                        .collect::<String>()
                });
                if self.search.preedit != text {
                    self.search.preedit = text;
                    if self.search.alias.is_none() {
                        self.search.model.invalidate();
                    }
                }
                Task::none()
            }
            Message::Query(query) if self.search.showing && self.search.alias.is_none() => {
                let from = self.search_paint_height();
                // A resize under way turns toward the new results' height. Otherwise
                // the card holds a moment for the results that come in from other
                // sources, so it does not jump twice. A later key keeps that moment:
                // restarting it held the card while someone kept typing.
                let resizing = self.search.height_motion.is_some();
                let wait = if resizing || self.search.height_due.is_some() {
                    None
                } else {
                    self.search.height_hold = Some(from);
                    self.search.height_due = Some(Instant::now() + HEIGHT_SETTLE);
                    self.search.height_epoch = self.search.height_epoch.wrapping_add(1);
                    Some(self.search.height_epoch)
                };
                self.search.pending_browse_collapse = false;
                self.search.feedback = None;
                if self.search.browse_all_apps && !query.trim().is_empty() {
                    self.search.browse_all_apps = false;
                    self.search.model.set_category(None);
                }
                self.search.model.set_query(query);
                self.search.file_results.clear();
                self.search.file_query = None;
                let refresh = self.refresh_overview_search();
                let providers = self.search_ask_providers();
                self.search_recompute();
                if resizing {
                    self.animate_search_height(from, self.search_surface_size().height);
                }
                Task::batch([
                    self.search_file_query(),
                    providers,
                    refresh,
                    wait.map_or_else(Task::none, |height_epoch| {
                        Task::perform(
                            async move {
                                tokio::time::sleep(HEIGHT_SETTLE).await;
                                height_epoch
                            },
                            |epoch| message(Message::HeightDue(epoch)),
                        )
                    }),
                ])
            }
            Message::Category(category) if self.search.showing => {
                self.search.browse_all_apps = false;
                if self.search.alias.is_some()
                    || self.search.action_busy
                    || self.search.preedit.is_some()
                {
                    return Task::none();
                }
                let from = self.search_paint_height();
                self.search.height_motion = None;
                self.search.pending_browse_collapse = false;
                let Some(revision) = self.search.source_revision.checked_add(1) else {
                    self.search.feedback =
                        Some(tr("shell-search-source-switches-exhausted").into());
                    return Task::none();
                };
                self.search.source_revision = revision;
                if let Some(category) = category
                    && !self.search.preferences.shows(category)
                {
                    self.search.feedback = Some(
                        tr!("shell-search-is-off-enable-it-in-settings", "arg0" => category.label().to_owned()),
                    );
                    return iced::widget::operation::focus("quick-search-input");
                }
                self.search.model.set_category(category);
                self.search.feedback = None;
                self.search.file_query = None;
                self.search.file_results.clear();
                let refresh = self.refresh_overview_search();
                let providers = self.search_ask_providers();
                self.search_recompute();
                self.animate_search_height(from, self.search_surface_size().height);
                Task::batch([
                    iced::widget::operation::focus("quick-search-input"),
                    self.search_file_query(),
                    providers,
                    refresh,
                ])
            }
            Message::BrowseApps(expanded)
                if self.search.showing
                    && self.search.blank_apps()
                    && self.search.model.query.trim().is_empty() =>
            {
                if !expanded && self.search.pending_browse_collapse {
                    return iced::widget::operation::focus("quick-search-input");
                }
                let from = self.search_paint_height();
                if !expanded
                    && self.search.browse_all_apps
                    && self.search.visible
                    && self.search_height_motion_enabled()
                {
                    self.search.pending_browse_collapse = true;
                    // A frame-attached card keeps its join at the closed height.
                    let compact = self
                        .search
                        .geometry
                        .as_ref()
                        .map_or(BLANK_HEIGHT, |geometry| geometry.0.height.min(BLANK_HEIGHT))
                        + self.search.island.map_or(0.0, |island| {
                            island.attachment.join(f32::from(self.metrics.radius))
                        });
                    self.animate_search_height(from, compact);
                    return iced::widget::operation::focus("quick-search-input");
                }
                self.search.pending_browse_collapse = false;
                self.search.app_scroll_carry = 0.0;
                self.search.browse_all_apps = expanded;
                self.search
                    .model
                    .set_category(expanded.then_some(Category::Apps));
                self.search_recompute();
                self.animate_search_height(from, self.search_surface_size().height);
                iced::widget::operation::focus("quick-search-input")
            }
            Message::ScrollApps(delta)
                if self.search.showing
                    && self.search.browse_all_apps
                    && !self.search.pending_browse_collapse
                    && self.search.model.category == Some(Category::Apps)
                    && self.search.model.query.trim().is_empty() =>
            {
                if delta.is_finite() {
                    let total = self.search.app_scroll_carry + delta.clamp(-24.0, 24.0);
                    let rows = total.trunc() as isize;
                    self.search.app_scroll_carry = total.fract();
                    let last = self
                        .search
                        .model
                        .results
                        .len()
                        .saturating_sub(self.search_capacity());
                    self.search.model.window_start = self
                        .search
                        .model
                        .window_start
                        .saturating_add_signed(rows)
                        .min(last);
                }
                Task::none()
            }
            Message::Navigate(delta)
                if self.search.showing
                    && self.search.alias.is_none()
                    && self.search.preedit.is_none() =>
            {
                if self.search.browse_all_apps && self.search.model.query.trim().is_empty() {
                    let capacity = self.search_capacity();
                    let start = self.search.model.window_start;
                    if self.search.model.selected_index().is_none_or(|index| {
                        index < start || index >= start.saturating_add(capacity)
                    }) && !self.search.model.results.is_empty()
                    {
                        let index = if delta < 0 {
                            (start + capacity.saturating_sub(1))
                                .min(self.search.model.results.len() - 1)
                        } else {
                            start.min(self.search.model.results.len() - 1)
                        };
                        self.search.model.selected =
                            Some(self.search.model.results[index].id.clone());
                        self.search.model.explicit_selection = true;
                        self.search.model.invalidate();
                        self.search.model.visible(capacity);
                        return iced::widget::operation::focus("quick-search-input");
                    }
                }
                let at = self.search.model.selected_index().unwrap_or(0);
                let next = at.saturating_add_signed(delta);
                if delta > 0
                    && !self.search.browse_all_apps
                    && self.search.model.query.trim().is_empty()
                    && self.search.model.category.is_none()
                    && self.search.blank_apps()
                    && next >= search::TOP_APPS
                    && search::browse_installed_apps(
                        self.search.apps.iter(),
                        &self.search.preferences,
                        search::TOP_APPS + 1,
                    )
                    .len()
                        > search::TOP_APPS
                {
                    let from = self.search_paint_height();
                    self.search.browse_all_apps = true;
                    self.search.model.set_category(Some(Category::Apps));
                    self.search_recompute();
                    self.search.model.move_selection(next as isize);
                    self.animate_search_height(from, self.search_surface_size().height);
                } else if delta < 0
                    && at == 0
                    && self.search.browse_all_apps
                    && self.search.model.query.trim().is_empty()
                    && self.search.model.category == Some(Category::Apps)
                {
                    return self.search_message(Message::BrowseApps(false));
                } else {
                    self.search.model.move_selection(delta);
                }
                let capacity = self.search_capacity();
                self.search.model.visible(capacity);
                iced::widget::operation::focus("quick-search-input")
            }
            Message::Key(id, generation, key, modifiers, repeat)
                if self.search.window == Some(id) && self.search.showing =>
            {
                use iced::keyboard::{Key, key::Named};
                if self.search.preedit.is_some() {
                    return Task::none();
                }
                if let Some(category) = source_chord(&key, modifiers) {
                    return if repeat {
                        Task::none()
                    } else {
                        self.search_message(Message::Category(category))
                    };
                }
                if modifiers.control() || modifiers.alt() || modifiers.logo() {
                    return Task::none();
                }
                if let Some(action) = self.activities_window_key(&key, modifiers) {
                    return if repeat
                        && matches!(
                            key,
                            Key::Character(_) | Key::Named(Named::Enter | Named::Delete)
                        ) {
                        Task::none()
                    } else {
                        self.spread_message(action)
                    };
                }
                // On the blank page the top apps are one row of icons: the
                // side arrows move along it and Down opens every app below.
                let top_row = self.search.model.query.trim().is_empty()
                    && !self.search.browse_all_apps
                    && self.search.model.category.is_none()
                    && self.search.blank_apps();
                match key {
                    Key::Named(Named::ArrowRight) if top_row => {
                        self.search_message(Message::Navigate(1))
                    }
                    Key::Named(Named::ArrowLeft) if top_row => {
                        self.search_message(Message::Navigate(-1))
                    }
                    Key::Named(Named::ArrowDown) if top_row => {
                        // The list opens with its first app chosen.
                        let open = self.search_message(Message::BrowseApps(true));
                        self.search.model.selected = None;
                        Task::batch([open, self.search_message(Message::Navigate(1))])
                    }
                    Key::Named(Named::ArrowDown) => self.search_message(Message::Navigate(1)),
                    Key::Named(Named::ArrowUp) => self.search_message(Message::Navigate(-1)),
                    Key::Named(Named::PageDown) => {
                        self.search_message(Message::Navigate(self.search_capacity() as isize))
                    }
                    Key::Named(Named::PageUp) => {
                        self.search_message(Message::Navigate(-(self.search_capacity() as isize)))
                    }
                    Key::Named(Named::Enter) if !repeat && self.search.alias.is_some() => {
                        self.search_message(Message::AliasSave)
                    }
                    Key::Named(Named::Enter) if !repeat => {
                        if self.search.browse_all_apps
                            && self.search.model.selected_index().is_some_and(|index| {
                                index < self.search.model.window_start
                                    || index
                                        >= self.search.model.window_start + self.search_capacity()
                            })
                        {
                            return Task::none();
                        }
                        if let Some(item) = self.search.model.selected_item() {
                            self.search_message(Message::Activate(Ticket {
                                generation,
                                id: item.id.clone(),
                            }))
                        } else {
                            self.search.feedback =
                                Some(tr("shell-no-available-result-selected").into());
                            Task::none()
                        }
                    }
                    _ => Task::none(),
                }
            }
            Message::Activate(ticket)
                if self.search.showing
                    && !self.search.action_busy
                    && self.search.preedit.is_none()
                    && self.search.alias.is_none() =>
            {
                let target = match self.search.model.activate(&ticket) {
                    Ok(target) => target,
                    Err(error) => {
                        self.search.feedback = Some(error);
                        return iced::widget::operation::focus("quick-search-input");
                    }
                };
                let usage = if ticket.id.starts_with("app:")
                    || ticket.id.starts_with("setting:")
                    || ticket.id.starts_with("action:")
                {
                    self.save_setting(SettingChange::Search(Change::RecordUse(ticket.id.clone())))
                } else {
                    Task::none()
                };
                let action = match target {
                    Target::Copy(value) => {
                        self.search.feedback = Some(tr("shell-copying-result").into());
                        self.search.action_busy = true;
                        let epoch = self.search.epoch;
                        iced::clipboard::write(value.clone()).then(move |result| {
                            if result.is_err() {
                                return Task::done(message(Message::Copied(epoch, false)));
                            }
                            let expected = value.clone();
                            iced::clipboard::read_text().map(move |actual| {
                                message(Message::Copied(
                                    epoch,
                                    actual.is_ok_and(|actual| actual.as_str() == expected),
                                ))
                            })
                        })
                    }
                    Target::Clipboard(id) => {
                        match self.search.clipboard.copy(id) {
                            Ok(()) => {
                                self.search.action_busy = true;
                                self.search.copy_epoch = Some(self.search.epoch);
                                self.search.feedback = Some(tr("shell-copying").into());
                            }
                            Err(error) => self.search.feedback = Some(error),
                        }
                        Task::none()
                    }
                    Target::Desktop(action) => match action {
                        DesktopAction::Light(value) => {
                            self.search_preference_action(SettingChange::Light(value))
                        }
                        DesktopAction::Tiling(value) => match self.snapshot.as_ref() {
                            Some(snapshot) => self.search_preference_action(
                                SettingChange::DesktopTiling(snapshot.workspace, Some(value)),
                            ),
                            None => Task::none(),
                        },
                        DesktopAction::Dnd(value) => {
                            self.search_preference_action(SettingChange::Notification(
                                enzui_shell::notifications::preferences::Change::Dnd(value),
                            ))
                        }
                        DesktopAction::NotificationCenter => {
                            let close = self.close_search();
                            Task::batch([
                                close,
                                self.notification_message(notification_panel::Message::Shortcut(
                                    enzui_shell::shortcuts::Event::OpenNotifications,
                                )),
                            ])
                        }
                        DesktopAction::ControlCenter => {
                            let close = self.close_search();
                            Task::batch([close, self.update(ShellMessage::ToggleControls)])
                        }
                    },
                    target @ (Target::Provider { .. } | Target::ProviderSearch { .. }) => self
                        .search_open_in_app(
                            Ticket {
                                generation: self.search.model.generation,
                                id: ticket.id.clone(),
                            },
                            target,
                        ),
                    target => {
                        self.search.action_busy = true;
                        if !matches!(
                            target,
                            Target::App(_)
                                | Target::Window(_)
                                | Target::Settings(_)
                                | Target::SettingsSection(_)
                        ) {
                            self.search.feedback = Some(tr("shell-opening").into());
                        }
                        let compositor = self.compositor.clone();
                        let root = self.root.clone();
                        let epoch = self.search.epoch;
                        perform(
                            async move { compositor.execute_search_target(&root, target).await },
                            move |result| Message::Done(epoch, result),
                        )
                    }
                };
                let focus = if self.search.showing && !self.search.action_busy {
                    iced::widget::operation::focus("quick-search-input")
                } else {
                    Task::none()
                };
                Task::batch([usage, action, focus])
            }
            Message::ProviderToken(epoch, window, ticket, target, token)
                if epoch == self.search.epoch
                    && self.search.window == Some(window)
                    && self.search.showing
                    && self.search.action_busy =>
            {
                if self.search.model.activate(&ticket).ok().as_ref() != Some(&target) {
                    self.search.action_busy = false;
                    self.search.feedback =
                        Some(tr("shell-the-search-changed-choose-the-result-again").into());
                    return Task::none();
                }
                self.search_open_in_app_with_token(epoch, target, token)
            }
            Message::Copied(epoch, confirmed) if epoch == self.search.epoch => {
                self.search.action_busy = false;
                self.search.feedback = Some(
                    if confirmed {
                        tr("shell-copied-result-to-the-clipboard")
                    } else {
                        tr("shell-clipboard-copy-could-not-be-confirmed-try-again")
                    }
                    .into(),
                );
                iced::widget::operation::focus("quick-search-input")
            }
            Message::Done(epoch, result) if epoch == self.search.epoch => {
                self.search.action_busy = false;
                match result {
                    Ok(()) => self.close_search(),
                    Err(error) => {
                        self.search.feedback = Some(error);
                        iced::widget::operation::focus("quick-search-input")
                    }
                }
            }
            Message::Apps(generation, result) => {
                self.search.apps_busy = false;
                if generation != self.search.apps_generation {
                    return self.search_scan_apps(false);
                }
                if self.search.preferences.enabled {
                    match result {
                        Ok(apps) => {
                            self.search.apps = apps;
                            self.search.apps_ready = true;
                            self.search_find_providers();
                            let check = self.search_check_activation();
                            self.search_recompute();
                            if !self.search.model.query.trim().is_empty() {
                                return Task::batch([check, self.search_ask_providers()]);
                            }
                            return check;
                        }
                        Err(error) => self.search.feedback = Some(error),
                    }
                    self.search_recompute();
                    if !self.search.model.query.trim().is_empty() {
                        return self.search_ask_providers();
                    }
                }
                Task::none()
            }
            Message::Windows(epoch, result) => {
                self.search.windows_busy = false;
                if epoch != self.search.epoch {
                    return self.search_read_windows();
                }
                if self.search.showing && self.search.preferences.shows(Category::Windows) {
                    match result {
                        Ok(windows) => self.search.windows = windows.items,
                        Err(error) => self.search.feedback = Some(error),
                    }
                    self.search_recompute();
                }
                Task::none()
            }
            Message::Indexed(key, result) => {
                self.search.file_busy = false;
                if self.search.file_wanted.as_ref() == Some(&key) {
                    self.search.file_built = Some(key);
                    match result {
                        Ok(index) => self.search.file_index = Some(index),
                        Err(error) => self.search.file_error = Some(error),
                    }
                }
                Task::batch([self.search_index(), self.search_file_query()])
            }
            Message::Pressed => {
                self.search.last_press = Some(Instant::now());
                Task::none()
            }
            Message::BarPressed => {
                self.search.bar_press = Some(Instant::now());
                Task::none()
            }
            // A press elsewhere (the desktop, the bar, another window) closes
            // Search; one in its own window did not reach this far apart.
            Message::OutsideCheck(epoch, at)
                if epoch == self.search.epoch
                    && self.search.showing
                    && !self.search.action_busy
                    // The overview's spread covers the output and handles every
                    // press there, including the ones that close it.
                    && !self.activities =>
            {
                let inside = self.search.last_press.is_some_and(|press| {
                    press + OUTSIDE_WINDOW >= at && at + OUTSIDE_WINDOW >= press
                });
                if inside {
                    Task::none()
                } else {
                    self.search.closed_by = self.search.bar_press.filter(|press| {
                        *press + OUTSIDE_WINDOW >= at && at + OUTSIDE_WINDOW >= *press
                    });
                    self.close_search()
                }
            }
            Message::OutsideCheck(..) => Task::none(),
            Message::Provided(generation, desktop_id, query, answer) => {
                self.search_provided(generation, desktop_id, query, answer)
            }
            Message::Activation(epoch, client)
                if epoch == self.search.epoch && self.search.showing =>
            {
                self.search.activation = client;
                if self.search.activation.is_some() && !self.search.model.query.trim().is_empty() {
                    self.search_ask_providers()
                } else {
                    Task::none()
                }
            }
            Message::Activation(..) => Task::none(),
            Message::Activated(generation, desktop_id, query, started) => {
                self.search_activated(generation, desktop_id, query, started)
            }
            Message::FileQuery(key, query, category, result) => {
                self.search.file_query_busy = false;
                if self.search.file_wanted.as_ref() == Some(&key)
                    && self.search.model.query == query
                    && self.search.model.category == category
                {
                    self.search.file_query = Some((query, category));
                    match result {
                        Ok(items) => self.search.file_results = items,
                        Err(error) => self.search.file_error = Some(error),
                    }
                    self.search_recompute();
                }
                self.search_file_query()
            }
            Message::ClipboardTarget(epoch, result)
                if self.search.clipboard.enabled && epoch == self.search.clipboard.epoch =>
            {
                match result {
                    Ok(target) => self.search.clipboard.target = Some(target),
                    Err(error) => self.search.clipboard.error = Some(error),
                }
                Task::none()
            }
            Message::Clipboard(event) => {
                let mut focus = Task::none();
                if let Some(result) = self.search.clipboard.message(event)
                    && self.search.copy_epoch == Some(self.search.epoch)
                {
                    self.search.copy_epoch = None;
                    self.search.action_busy = false;
                    focus = iced::widget::operation::focus("quick-search-input");
                    self.search.feedback = Some(match result {
                        Ok(()) => {
                            tr("shell-copied-to-clipboard-paste-explicitly-in-your-app").into()
                        }
                        Err(error) => error,
                    });
                }
                self.search_recompute();
                focus
            }
            Message::Refresh if self.search.preedit.is_none() => {
                self.search.apps_ready = false;
                self.search.apps_generation = self.search.apps_generation.saturating_add(1);
                if let Some(cancel) = &self.search.apps_cancel {
                    cancel.store(true, Ordering::Release);
                }
                self.search_file_request(self.search.file_wanted.clone(), true);
                self.search.model.set_category(self.search.model.category);
                self.search.feedback = None;
                self.search_recompute();
                let clipboard = self
                    .search
                    .clipboard
                    .retry()
                    .map_or_else(Task::none, clipboard_connection);
                Task::batch([
                    self.search_scan_apps(false),
                    self.search_index(),
                    clipboard,
                    iced::widget::operation::focus("quick-search-input"),
                ])
            }
            Message::Favorite(ticket) if self.search.showing && self.search.preedit.is_none() => {
                if let Err(error) = self.search.model.activate(&ticket) {
                    self.search.feedback = Some(error);
                    return Task::none();
                }
                let key = ticket.id;
                let change = Change::Favorite(
                    key.clone(),
                    !self.search.preferences.favorites.contains(&key),
                );
                Task::batch([
                    self.search_save_preference(change),
                    iced::widget::operation::focus("quick-search-input"),
                ])
            }
            Message::Alias(ticket) if self.search.showing && self.search.preedit.is_none() => {
                if let Err(error) = self.search.model.activate(&ticket) {
                    self.search.feedback = Some(error);
                    return Task::none();
                }
                if let Some(item) = self.search.model.selected_item() {
                    if !["app:", "setting:", "action:"]
                        .iter()
                        .any(|prefix| item.id.starts_with(prefix))
                    {
                        return Task::none();
                    }
                    self.search.alias = Some((item.id.clone(), item.title.clone()));
                    self.search.alias_draft.clear();
                    return iced::widget::operation::focus("quick-search-alias");
                }
                Task::none()
            }
            Message::AliasDraft(value) => {
                self.search.alias_draft = value.chars().take(16).collect();
                Task::none()
            }
            Message::AliasSave if self.search.preedit.is_none() => {
                let Some((id, _)) = self.search.alias.clone() else {
                    return Task::none();
                };
                let change = Change::Alias(self.search.alias_draft.trim().to_lowercase(), Some(id));
                let mut test = self.search.preferences.clone();
                if let Err(error) = test.preview(&change) {
                    self.search.feedback = Some(error);
                    return self.search.window.map_or_else(Task::none, |window| {
                        iced_runtime::task::widget_in(
                            window,
                            focus_if_unfocused("quick-search-alias".into()),
                        )
                    });
                }
                self.search.alias = None;
                self.search_recompute();
                Task::batch([
                    self.search_save_preference(change),
                    iced::widget::operation::focus("quick-search-input"),
                ])
            }
            Message::AliasCancel => {
                self.search.preedit = None;
                self.search.alias = None;
                self.search_recompute();
                iced::widget::operation::focus("quick-search-input")
            }
            Message::Settings => {
                let item = Item::new(
                    "setting:quick-search",
                    Category::Actions,
                    tr("shell-quick-search-settings"),
                    "",
                    tr("shell-open"),
                    "settings",
                    Some(Target::Settings("quick-search")),
                );
                if !self
                    .search
                    .model
                    .results
                    .iter()
                    .any(|entry| entry.id == item.id)
                {
                    self.search.model.results.push(item.clone());
                }
                let ticket = self.search.model.ticket(&item);
                self.search_message(Message::Activate(ticket))
            }
            Message::Tick => {
                if self.search.clipboard.expire() {
                    self.search_recompute();
                }
                Task::none()
            }
            Message::HeightDue(epoch)
                if self.search.showing
                    && epoch == self.search.height_epoch
                    && self.search.height_due.is_some() =>
            {
                self.search.height_due = Some(Instant::now() - Duration::from_millis(1));
                self.search_message(Message::HeightFrame)
            }
            Message::HeightDue(_) => Task::none(),
            Message::HeightFrame => {
                self.redraw_only = self.search.window;
                if self
                    .search
                    .height_due
                    .is_some_and(|due| Instant::now() >= due)
                {
                    let from = self
                        .search
                        .height_hold
                        .take()
                        .unwrap_or_else(|| self.search_surface_size().height);
                    self.animate_search_height(from, self.search_surface_size().height);
                }
                if self
                    .search
                    .height_motion
                    .is_some_and(|motion| motion.finished(Instant::now()))
                    || !self.search_height_motion_enabled()
                        && (self.search.height_motion.is_some()
                            || self.search.pending_browse_collapse)
                {
                    self.search.height_motion = None;
                    if self.search.pending_browse_collapse {
                        self.search.pending_browse_collapse = false;
                        self.search.app_scroll_carry = 0.0;
                        self.search.browse_all_apps = false;
                        self.search.model.set_category(None);
                        self.search_recompute();
                    }
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }
    pub(super) fn search_preference_action(&mut self, change: SettingChange) -> Task<ShellMessage> {
        self.search.feedback = Some(tr("shell-applying-desktop-preference").into());
        self.search.pending_desktop = Some(change.clone());
        self.save_setting(change)
    }
    pub(super) fn search_save_preference(&mut self, change: Change) -> Task<ShellMessage> {
        let mut value = self.search.preferences.clone();
        if let Err(error) = value.preview(&change) {
            self.search.feedback = Some(error);
            return Task::none();
        }
        self.search.pending_preference = Some(change.clone());
        self.search.feedback = Some(tr("shell-saving-preference").into());
        self.save_setting(SettingChange::Search(change))
    }
}
