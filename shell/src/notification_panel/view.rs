use super::model::Content;
use super::{
    model::Message,
    state::{center_padding, key_id, message},
};
use crate::{Message as ShellMessage, Shell, region};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    compositor::ControlGeometry,
    notifications::{
        model::{Archived, Notification},
        presentation::announcement_label,
    },
};
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    keyed_column::keyed_column,
    scroll_view::enz_scroll_view,
    section_header::enz_section_header,
    semantics_container::SemanticsContainer,
    status_page::{StatusPageVariant, enz_status_page},
    text::enz_text,
    text_field::enz_text_field,
    toggle::enz_toggle,
    toolbar_icon::enz_toolbar_icon,
    view_switcher::{SwitcherPage, enz_view_switcher},
};
use iced::{
    Element, Length,
    widget::{column, container, row},
    window,
};

impl Shell {
    pub(crate) fn notification_view(&self, id: window::Id) -> Option<Element<'_, ShellMessage>> {
        if let Some(pane) = self
            .notification
            .center
            .as_ref()
            .filter(|pane| pane.window == id)
        {
            return Some(region::popup(
                self.notification_center_view(&pane.geometry),
                self.popup(
                    id,
                    pane.visible,
                    pane.geometry.attachment,
                    message(Message::Hidden(id)),
                ),
            ));
        }
        if self
            .notification
            .toast
            .as_ref()
            .is_some_and(|pane| pane.window == id)
        {
            // Glass toasts: the compositor paints each card's body; the cards
            // slide in layout, so their glass follows them.
            let input = self.surface_inputs.get(&id).cloned();
            let glass = self.panel_glass().filter(|_| {
                self.metrics.islands.is_some()
                    && input
                        .as_ref()
                        .is_some_and(enzui_shell::surface_input::SurfaceInput::glass_supported)
            });
            let shown = self.notification.toast_display();
            let rows = shown
                .iter()
                .filter_map(|entry| {
                    // Built from the snapshot when current, else the copy kept while leaving.
                    let entry = self
                        .notification
                        .toast_shown
                        .iter()
                        .find(|kept| kept.key == entry.key)
                        .filter(|_| {
                            self.notification
                                .toast_leaving
                                .contains_key(&(entry.key.epoch, entry.key.id))
                        })
                        .or_else(|| {
                            self.notification
                                .toast_entries()
                                .into_iter()
                                .find(|current| current.key == entry.key)
                        })?;
                    Some((
                        key_id(entry.key),
                        self.notification_toast(entry, glass.is_some()),
                    ))
                })
                .collect();
            let several = shown.len() >= 2;
            let (opacity, _) = self
                .notification
                .dismiss_all_opacity(std::time::Instant::now(), self.toast_timing());
            let mut rows: Vec<(String, Element<'_, ShellMessage>)> = rows;
            if several {
                rows.push(("dismiss-all".into(), self.dismiss_all_toasts(opacity)));
            }
            let content = keyed_column(rows)
                .spacing(enzui_shell::notifications::card::STACK_GAP)
                .width(Length::Fill);
            let tint = self.glass_tint(
                id,
                self.card_surface()
                    .unwrap_or_else(|| self.tokens().palette.surface_elevated),
            );
            let look = glass.map(|style| enzui_shell::glass_rows::Look {
                tokens: enzui_shell::reveal::glass_tokens(
                    style,
                    self.tokens().motion.reduced_motion,
                ),
                tint,
                radius: 12.0,
                inset_left: enzui_shell::notifications::card::badge_overhang(
                    self.desired_settings.notifications.style,
                ),
                shadow: enzui_shell::reveal::glass_shadow(
                    enzui_shell::notifications::card::elevation(
                        self.desired_settings.notifications.style,
                    ),
                    tint,
                    false,
                ),
            });
            let button = enzui_shell::notifications::card::DISMISS_ALL_ROW
                - enzui_shell::notifications::card::STACK_GAP;
            // A faded-out button takes no input; its row stays for the layout.
            let trailing = if several {
                Some(if opacity >= 0.5 { button } else { 0.0 })
            } else {
                None
            };
            let content = if several && opacity < 0.5 {
                enzui_shell::glass_rows::glass_rows_input_without_last(content.into(), input, look)
            } else {
                enzui_shell::glass_rows::glass_rows_input(content.into(), input, look, trailing)
            };
            // The window keeps its size while toasts come and go; the cards
            // hug the edge they grow from.
            use enzui_shell::notifications::preferences::Position;
            let lower = matches!(
                self.desired_settings.notifications.position,
                Position::BottomLeft | Position::BottomRight
            );
            let content: Element<'_, ShellMessage> = container(content)
                .height(Length::Fill)
                .align_y(if lower {
                    iced::alignment::Vertical::Bottom
                } else {
                    iced::alignment::Vertical::Top
                })
                .into();
            // An overshooting slide travels into the inner side's extra room.
            let margin = enzui_shell::notifications::card::SHADOW_MARGIN;
            let inner = margin + self.notification.toast_overshoot;
            let start = matches!(
                self.desired_settings.notifications.position,
                Position::TopLeft | Position::BottomLeft
            );
            return Some(
                container(content)
                    .padding(iced::Padding {
                        top: margin,
                        bottom: margin,
                        left: if start { margin } else { inner },
                        right: if start { inner } else { margin },
                    })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_| container::Style {
                        background: None,
                        ..Default::default()
                    })
                    .into(),
            );
        }
        None
    }
    // Open occupied rows before sliding new toast cards in from the screen edge.
    fn notification_toast<'a>(
        &'a self,
        entry: &'a Notification,
        glass: bool,
    ) -> Element<'a, ShellMessage> {
        use enzui_shell::notifications::preferences::Position;
        let from_left = matches!(
            self.desired_settings.notifications.position,
            Position::TopLeft | Position::BottomLeft
        );
        let travel = f32::from(self.desired_settings.notifications.width)
            + 2.0 * enzui_shell::notifications::card::SHADOW_MARGIN;
        let timing = self.toast_timing();
        let (open, progress) = match self
            .notification
            .revealed
            .get(&(entry.key.epoch, entry.key.id))
        {
            _ if timing.reduced => (1.0, 1.0),
            Some((at, push)) => timing.entrance(at.elapsed(), *push),
            None => (
                if self.notification.shown_revealed() {
                    0.0
                } else {
                    1.0
                },
                0.0,
            ),
        };
        let mut dx = travel * (1.0 - progress) * if from_left { -1.0 } else { 1.0 };
        let mut open = open;
        // Going away: out the way it came in, while the row closes.
        if let Some(at) = self
            .notification
            .toast_leaving
            .get(&(entry.key.epoch, entry.key.id))
        {
            let t =
                1.0 - (1.0 - super::motion::Timing::progress(at.elapsed(), timing.leave)).powi(3);
            dx = travel * t * if from_left { -1.0 } else { 1.0 };
            open = 1.0 - t;
        }
        let card = if glass {
            self.notification_card_glass(entry, true, true, true)
        } else {
            self.notification_card(entry, true)
        };
        enzui_shell::glass_rows::shifted(card, dx, open)
    }
    // The reply field written on the toast itself.
    fn toast_reply_row<'a>(&'a self, entry: &'a Notification) -> Element<'a, ShellMessage> {
        let text = self
            .notification
            .reply
            .as_ref()
            .map_or("", |reply| reply.text.as_str());
        container(self.reply_field(entry.key, Some(entry), text, "notification-toast-reply"))
            .height(enzui_shell::notifications::card::REPLY_ROW)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill)
            .into()
    }
    // One rounded reply field with its send and cancel actions inside its
    // trailing edge, the way a message composer has them.
    fn reply_field<'a>(
        &'a self,
        key: enzui_shell::notifications::model::Key,
        entry: Option<&'a Notification>,
        text: &'a str,
        id: &'static str,
    ) -> Element<'a, ShellMessage> {
        let pending = self.notification.action_pending(&key);
        let options = entry.and_then(|entry| entry.draft.reply.as_ref());
        let send = enz_toolbar_icon("send", message(Message::SendReply(key)))
            .size(26.0, 15.0)
            .aria_label(options.map_or_else(
                || enzui_shell::i18n::tr("shell-send"),
                |o| o.submit.as_str(),
            ))
            .id(format!("{id}-send").into())
            .enabled(
                !pending && entry.is_some() && !text.trim().is_empty() && self.notification.ready(),
            )
            .into_element();
        let cancel = enz_toolbar_icon("x", message(Message::DiscardReply))
            .size(26.0, 15.0)
            .aria_label(tr("shell-cancel-reply"))
            .id(format!("{id}-cancel").into())
            .enabled(!pending)
            .into_element();
        enz_text_field(
            "",
            options.map_or_else(
                || {
                    enzui_shell::i18n::current().text(if entry.is_some() {
                        "shell-reply-2"
                    } else {
                        "shell-notification-changed-draft-retained"
                    })
                },
                |o| o.placeholder.clone(),
            ),
            text,
            move |value| message(Message::EditReply(key, value)),
        )
        .aria_label(enzui_shell::i18n::tr("shell-reply"))
        .id(id.into())
        .radius(enzui_shell::notifications::card::REPLY_ROW / 2.0)
        .trailing(
            row![send, cancel]
                .spacing(2)
                .align_y(iced::Alignment::Center),
        )
        .enabled(!pending)
        .on_submit(message(Message::SendReply(key)))
        .into_element()
    }
    // A floating, see-through round button under the stack.
    fn dismiss_all_toasts(&self, opacity: f32) -> Element<'_, ShellMessage> {
        let palette = self.tokens().palette;
        let size = enzui_shell::notifications::card::DISMISS_ALL_ROW
            - enzui_shell::notifications::card::STACK_GAP;
        let fade = move |c: iced::Color| iced::Color {
            a: c.a * opacity,
            ..c
        };
        let face = container(
            enzui_widgets::icon::enz_icon("brush-cleaning")
                .size(16.0)
                .tint(fade(palette.text))
                .into_element(),
        )
        .center_x(size)
        .center_y(size)
        .style(move |_| container::Style {
            background: Some(
                fade(iced::Color {
                    a: 0.55,
                    ..palette.surface_elevated
                })
                .into(),
            ),
            border: iced::Border {
                radius: (size / 2.0).into(),
                ..Default::default()
            },
            shadow: iced::Shadow {
                color: fade(palette.shadow),
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
            ..Default::default()
        });
        // Its hover and focus marks follow the disc, not a box around it.
        let button = enzui_widgets::clickable::enz_clickable(face, message(Message::HideToasts))
            .padding(0)
            .radius(size / 2.0)
            .min_hit(size)
            .width(size)
            .height(size)
            .enabled(opacity >= 0.5)
            .aria_label(tr("shell-dismiss-all-notifications"))
            .id("notification-toasts-dismiss-all".into())
            .into_element();
        // An icon alone does not say what it clears.
        let button = enzui_widgets::tooltip::enz_tooltip(
            button,
            tr("shell-dismiss-all-notifications"),
            iced::widget::tooltip::Position::Top,
        );
        container(button).center_x(Length::Fill).into()
    }
    fn toast_body_lines(&self, toast: bool, expanded: bool) -> usize {
        let style = self.desired_settings.notifications.style;
        if expanded {
            0
        } else if toast {
            style.toast_lines()
        } else {
            style.toast_lines() + 1
        }
    }
    // The bar's colour for cards when the bar colour is chosen.
    fn card_surface(&self) -> Option<iced::Color> {
        self.desired_settings
            .notifications
            .bar_colour
            .then(|| self.tokens().palette.surface)
    }
    // The toast card's exact height at the configured toast width.
    // A reply is being written on this toast, not in the centre.
    pub(super) fn toast_replying(&self, key: enzui_shell::notifications::model::Key) -> bool {
        self.notification.center.is_none()
            && self
                .notification
                .reply
                .as_ref()
                .is_some_and(|reply| reply.key == key)
    }
    pub(super) fn notification_card_height(&self, entry: &Notification) -> f32 {
        let actions = entry
            .draft
            .actions
            .iter()
            .any(|action| action.key != "inline-reply" && action.key != "default")
            || entry.draft.reply.is_some();
        enzui_shell::notifications::card::height(&enzui_shell::notifications::card::Extent {
            style: self.desired_settings.notifications.style,
            width: f32::from(self.desired_settings.notifications.width),
            text: f32::from(self.metrics.text),
            summary: &entry.draft.summary,
            body: &entry.draft.body,
            body_lines: self.toast_body_lines(true, false),
            image: self.desired_settings.notifications.images && entry.draft.image.is_some(),
            actions,
            reply: self.toast_replying(entry.key),
            progress: entry.draft.value.is_some(),
        })
    }
    pub(super) fn notification_card<'a>(
        &'a self,
        entry: &'a Notification,
        toast: bool,
    ) -> Element<'a, ShellMessage> {
        self.notification_card_glass(entry, toast, false, toast)
    }
    pub(super) fn notification_card_glass<'a>(
        &'a self,
        entry: &'a Notification,
        toast: bool,
        glass: bool,
        named: bool,
    ) -> Element<'a, ShellMessage> {
        let key = entry.key;
        let identifier = key_id(key);
        let pending = self.notification.action_pending(&key);
        let draft = self
            .notification
            .reply
            .as_ref()
            .is_some_and(|reply| reply.key == key && !reply.text.is_empty());
        let expanded = !toast && self.notification.expanded == Some(key);
        let palette = self.tokens().palette;
        let mut actions = Vec::new();
        for action in entry
            .draft
            .actions
            .iter()
            // The default action is the card's own click.
            .filter(|action| action.key != "inline-reply" && action.key != "default")
            .take(if toast { 3 } else { 8 })
        {
            actions.push(
                enz_button(
                    action.label.clone(),
                    message(Message::Invoke(key, action.key.clone())),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .width(Length::Fill)
                .id(format!("{identifier}:action:{}", action.key).into())
                .enabled(!pending && !draft && self.notification.ready())
                .into_element(),
            );
        }
        if entry.draft.reply.is_some() {
            actions.push(
                enz_button(tr("shell-reply"), message(Message::Reply(key)))
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("{identifier}:reply").into())
                    .enabled(!pending && self.notification.ready())
                    .into_element(),
            );
        }
        let text = f32::from(self.metrics.text);
        let long = !toast
            && enzui_shell::notifications::card::height(
                &enzui_shell::notifications::card::Extent {
                    style: self.desired_settings.notifications.style,
                    width: f32::from(self.desired_settings.notifications.width),
                    text,
                    summary: &entry.draft.summary,
                    body: &entry.draft.body,
                    body_lines: 0,
                    image: false,
                    actions: false,
                    reply: false,
                    progress: false,
                },
            ) > enzui_shell::notifications::card::height(
                &enzui_shell::notifications::card::Extent {
                    style: self.desired_settings.notifications.style,
                    width: f32::from(self.desired_settings.notifications.width),
                    text,
                    summary: &entry.draft.summary,
                    body: &entry.draft.body,
                    body_lines: self.toast_body_lines(false, false),
                    image: false,
                    actions: false,
                    reply: false,
                    progress: false,
                },
            );
        if long {
            actions.insert(
                0,
                enz_button(
                    if expanded {
                        tr("shell-show-less")
                    } else {
                        tr("shell-show-more")
                    },
                    message(Message::Expand(key)),
                )
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Sm)
                .id(format!("{identifier}:details").into())
                .into_element(),
            );
        }
        if toast && self.toast_replying(key) {
            actions = vec![self.toast_reply_row(entry)];
        }
        let image = (self.desired_settings.notifications.images)
            .then_some(entry.draft.image.as_ref())
            .flatten()
            .map(|image| {
                iced::widget::image(image.handle())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            });
        let card = enzui_shell::notifications::card::view(
            enzui_shell::notifications::card::Card {
                style: self.desired_settings.notifications.style,
                surface: self.card_surface(),
                text,
                radius: 12.0,
                icon: &entry.origin.icon,
                app_icon: entry.origin.app_icon.as_deref(),
                // In the panel a group header names the sender of two or more.
                source: if named { &entry.origin.label } else { "" },
                time: Some(relative_time(entry.received_ms)),
                summary: &entry.draft.summary,
                body: if toast || expanded {
                    &entry.draft.body
                } else {
                    entry
                        .draft
                        .body
                        .get(
                            ..entry
                                .draft
                                .body
                                .char_indices()
                                .nth(400)
                                .map_or(entry.draft.body.len(), |(at, _)| at),
                        )
                        .unwrap_or(&entry.draft.body)
                },
                body_lines: self.toast_body_lines(toast, expanded),
                critical: entry.draft.urgency
                    == enzui_shell::notifications::payload::Urgency::Critical,
                unread: !toast && entry.unread,
                image,
                dismiss: Some(
                    enz_toolbar_icon("x", message(Message::Dismiss(key)))
                        .size(22.0, 14.0)
                        .aria_label(tr("shell-dismiss-notification"))
                        .id(format!("{identifier}:dismiss").into())
                        .enabled(!pending && !draft)
                        .into_element(),
                ),
                actions,
                height: toast.then(|| self.notification_card_height(entry)),
                glass,
                progress: entry.draft.value,
            },
            &palette,
        );
        // A click anywhere a button did not take opens what the message is
        // about: its default action, or the app that sent it.
        let card: Element<'_, ShellMessage> = iced::widget::mouse_area(card)
            .on_press(message(Message::Activate(key)))
            .interaction(iced::mouse::Interaction::Pointer)
            .into();
        let announcement = if toast {
            self.notification
                .arrivals
                .announcement(key, entry.draft.urgency)
        } else {
            enzui_core::live_region::Announcement::Silent
        };
        SemanticsContainer::new(
            card,
            announcement.region(announcement_label(
                &entry.origin.label,
                &entry.draft.summary,
                &entry.draft.body,
                expanded,
            )),
        )
        .id(format!("{identifier}:region").into())
        .into()
    }
    pub(super) fn historical_card<'a>(&'a self, entry: &'a Archived) -> Element<'a, ShellMessage> {
        let expanded = self.notification.expanded_history == Some(entry.id);
        let image = (self.desired_settings.notifications.images)
            .then_some(entry.image.as_ref())
            .flatten()
            .map(|image| {
                iced::widget::image(image.handle())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            });
        let details = enz_button(
            if expanded {
                tr("shell-show-less")
            } else {
                tr("shell-read-full-message")
            },
            message(Message::HistoricalDetails(entry.id)),
        )
        .kind(ButtonKind::Quiet)
        .size(ButtonSize::Sm)
        .id(format!("historical-details:{}", entry.id).into())
        .into_element();
        let card = enzui_shell::notifications::card::view(
            enzui_shell::notifications::card::Card {
                style: self.desired_settings.notifications.style,
                surface: self.card_surface(),
                text: f32::from(self.metrics.text),
                radius: 12.0,
                icon: "history",
                app_icon: None,
                // The group header names the sender.
                source: "",
                time: Some(relative_time(entry.received_ms)),
                summary: &entry.summary,
                body: &entry.body,
                body_lines: if expanded { 0 } else { 4 },
                critical: false,
                unread: false,
                image,
                dismiss: None,
                actions: vec![details],
                height: None,
                glass: false,
                progress: None,
            },
            &self.tokens().palette,
        );
        SemanticsContainer::new(
            card,
            enzui_core::live_region::Announcement::Silent.region(announcement_label(
                &entry.source,
                &entry.summary,
                &entry.body,
                expanded,
            )),
        )
        .id(format!("historical-region:{}", entry.id).into())
        .into()
    }
    pub(super) fn notification_center_view(
        &self,
        geometry: &ControlGeometry,
    ) -> Element<'_, ShellMessage> {
        if self.notification.content == Content::Activity {
            return container(self.activity_view())
                .padding(center_padding(
                    geometry,
                    self.bar,
                    f32::from(self.metrics.radius),
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        // Opened from the clock: its calendar alone.
        if self.notification.content == Content::Calendar {
            return container(self.clock_view())
                .padding(center_padding(
                    geometry,
                    self.bar,
                    f32::from(self.metrics.radius),
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        }
        let history = self.notification.history;
        let dnd = self.desired_settings.notifications.policy.dnd;
        // The title and the one switch people reach for share a row: Do Not
        // Disturb on a row of its own took as much room as a notification.
        let header = row![
            enz_text(enzui_shell::i18n::tr("shell-notifications"))
                .size_px(enzui_shell::panel_style::title_px(f32::from(
                    self.metrics.text
                )))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            iced::widget::space().width(Length::Fill),
            enz_toggle(
                dnd,
                enzui_shell::i18n::current().text("shell-do-not-disturb"),
                |value| message(Message::Dnd(value)),
            )
            .id("notification-dnd".into())
            .into_element(),
        ]
        .spacing(12)
        .height(HEADER)
        .align_y(iced::Alignment::Center);
        let tabs = enz_view_switcher(
            vec![
                SwitcherPage::new("current", enzui_shell::i18n::tr("shell-current")),
                SwitcherPage::new("history", enzui_shell::i18n::tr("shell-history-2")),
            ],
            usize::from(history),
            |index| message(Message::History(index == 1)),
        )
        .stretch(true)
        // A view choice, not a switch: a soft tinted segment (a solid
        // accent tab read as something switched on).
        .selection_indicator(enzui_widgets::view_switcher::TabSelectionStyle::TranslucentAccent)
        .aria_label(tr("shell-notification-view"))
        .tab_id_prefix("notification-view")
        .into_element();
        let current = if history {
            Vec::new()
        } else {
            self.notification.current_entries()
        };
        let archive = if history {
            self.notification.history_entries()
        } else {
            Vec::new()
        };
        let total = if history {
            archive.len()
        } else {
            current.len()
        };
        let page = self.notification.page.min(total.saturating_sub(1) / 12);
        let mut cards = Vec::new();
        let mut group = None;
        if history {
            for entry in archive.iter().skip(page * 12).take(12) {
                if group != Some(&entry.principal) {
                    group = Some(&entry.principal);
                    let count = archive
                        .iter()
                        .filter(|other| other.principal == entry.principal)
                        .count();
                    cards.push((
                        format!("group:{}", entry.principal),
                        enz_section_header(format!("{} · {count}", entry.source))
                            .heading_level(2)
                            .into_element(),
                    ));
                }
                cards.push((
                    format!("historical:{}", entry.id),
                    self.historical_card(entry),
                ));
            }
        } else {
            for entry in current.iter().skip(page * 12).take(12) {
                let count = current
                    .iter()
                    .filter(|other| other.origin.principal == entry.origin.principal)
                    .count();
                if group != Some(&entry.origin.principal) && count > 1 {
                    group = Some(&entry.origin.principal);
                    cards.push((
                        format!("group:{}", entry.origin.principal),
                        enz_section_header(format!("{} · {count}", entry.origin.label))
                            .heading_level(2)
                            .into_element(),
                    ));
                }
                // A lone notification names its sender itself.
                cards.push((
                    key_id(entry.key),
                    self.notification_card_glass(entry, false, false, count == 1),
                ));
            }
        }
        let list = if cards.is_empty() {
            let (title, description, icon) = if history
                && !self.notification.history_query.trim().is_empty()
            {
                (
                    tr("shell-no-matches"),
                    tr("shell-try-another-source-title-or-message"),
                    "search",
                )
            } else if history && !self.desired_settings.notifications.policy.history {
                (
                    tr("shell-history-is-off"),
                    tr("shell-enable-saved-history-in-notification-settings-to-keep-past-messages"),
                    "history",
                )
            } else if history {
                (
                    tr("shell-no-saved-notifications"),
                    tr("shell-past-notifications-will-appear-here-while-history-is-enabled"),
                    "history",
                )
            } else {
                (
                    tr("shell-you-re-all-caught-up"),
                    tr("shell-new-notifications-will-appear-here"),
                    "bell",
                )
            };
            container(
                enz_status_page(title)
                    .description(description)
                    .icon(icon)
                    .variant(StatusPageVariant::Inline)
                    .compact(true)
                    .into_element(),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        } else {
            enz_scroll_view(keyed_column(cards).spacing(12.0).width(Length::Fill))
                .height(Length::Fill)
                .width(Length::Fill)
                .id("notification-list".into())
                .scrollbar_tokens(self.tokens().scrollbar)
                .into_element()
        };
        let mut regions = vec![("header".into(), header.into()), ("tabs".into(), tabs)];
        // Nothing to search while history is off and holds nothing.
        if history
            && (self.desired_settings.notifications.policy.history
                || !self.notification.history_entries().is_empty()
                || !self.notification.history_query.trim().is_empty())
        {
            regions.push((
                "history-search".into(),
                enzui_widgets::search_field::enz_search_field(
                    &self.notification.history_query,
                    |query| message(Message::HistoryQuery(query)),
                )
                .placeholder(tr("shell-search-source-title-or-message"))
                .id("notification-history-search")
                .radius(SEARCH_HEIGHT / 2.0)
                .width(Length::Fill)
                .focus_on_mount(true)
                .into_element(),
            ));
        }
        if let Some(error) = &self.notification.error {
            regions.push((
                "error".into(),
                enz_text(error.clone())
                    .size(enzui_widgets::text::TextSize::Footnote)
                    .color(self.tokens().palette.danger)
                    .wrap(true)
                    .into_element(),
            ));
        }
        if let Some(error) = &self.notification.shortcut_error {
            regions.push((
                "shortcut-error".into(),
                enz_text(tr!("shell-shortcut", "arg0" => error.clone()))
                    .wrap(true)
                    .into_element(),
            ));
        }
        if let Some(error) = &self.notification.storage_error {
            regions.push((
                "storage-error".into(),
                enz_text(tr!("shell-history", "arg0" => error.clone()))
                    .wrap(true)
                    .into_element(),
            ));
        }
        if !self.notification.ready() {
            regions.push((
                "retry".into(),
                enz_button(
                    tr("shell-start-notification-service"),
                    message(Message::Retry),
                )
                .into_element(),
            ));
        }
        // Clearing sits directly above what it clears.
        let can_clear = history
            && self
                .notification
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.history.is_empty());
        let clear = if history {
            can_clear.then(|| {
                enz_button(
                    tr("shell-clear-all-history"),
                    message(Message::ClearHistory),
                )
                .kind(ButtonKind::QuietDestructive)
                .size(enzui_widgets::button::ButtonSize::Sm)
                .id("notification-clear-history".into())
                .into_element()
            })
        } else {
            (!current.is_empty()).then(|| {
                enz_button(tr("shell-clear-all"), message(Message::DismissAll))
                    .kind(ButtonKind::Quiet)
                    .size(enzui_widgets::button::ButtonSize::Sm)
                    .id("notification-clear-all".into())
                    .into_element()
            })
        };
        let list = match clear {
            Some(clear) => column![
                row![iced::widget::space().width(Length::Fill), clear]
                    .height(CLEAR_ROW)
                    .align_y(iced::Alignment::Center),
                list
            ]
            .spacing(CLEAR_GAP)
            .height(Length::Fill)
            .into(),
            None => list,
        };
        regions.push(("list".into(), list));
        let mut footer = row![].spacing(6).align_y(iced::Alignment::Center);
        if total > 12 {
            footer = footer
                .push(
                    enz_button(
                        tr("shell-previous"),
                        message(Message::Page(page.saturating_sub(1))),
                    )
                    .enabled(page > 0)
                    .kind(ButtonKind::Quiet)
                    .into_element(),
                )
                .push(enz_text(format!("{} / {}", page + 1, total.div_ceil(12))).into_element())
                .push(
                    enz_button(tr("shell-next"), message(Message::Page(page + 1)))
                        .enabled((page + 1) * 12 < total)
                        .kind(ButtonKind::Quiet)
                        .into_element(),
                );
        }
        if total > 12 {
            regions.push(("footer".into(), footer.into()));
        }
        if let Some(reply) = &self.notification.reply {
            regions.push((
                "reply".into(),
                self.reply_field(
                    reply.key,
                    self.notification.current(reply.key).map(|entry| &**entry),
                    &reply.text,
                    "notification-reply",
                ),
            ));
        }
        container(
            keyed_column(regions)
                .spacing(REGION_GAP)
                .height(Length::Fill)
                .width(Length::Fill),
        )
        .padding(center_padding(
            geometry,
            self.bar,
            f32::from(self.metrics.radius),
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

// The center's fixed rows, shared with the height its window is opened at.
pub(super) const HEADER: f32 = 32.0;
pub(super) const TABS: f32 = 36.0;
pub(super) const REGION_GAP: f32 = enzui_shell::panel_style::GAP;
pub(super) const CLEAR_ROW: f32 = 26.0;
pub(super) const CLEAR_GAP: f32 = 6.0;
const SEARCH_HEIGHT: f32 = 36.0;
/// Everything in the center but its list, with the padding inside the panel.
pub(super) const CENTER_CHROME: f32 = 2.0 * enzui_shell::panel_style::INSET
    + HEADER
    + REGION_GAP
    + TABS
    + REGION_GAP
    + CLEAR_ROW
    + CLEAR_GAP;

// "now", "5 min", "3 h" or "2 d" since `received_ms` (Unix milliseconds).
fn relative_time(received_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(received_ms, |elapsed| elapsed.as_millis() as u64);
    let minutes = now.saturating_sub(received_ms) / 60_000;
    match minutes {
        0 => tr("shell-now").into(),
        1..=59 => tr!("shell-min-3", "arg0" => format!("{:}", minutes)),
        60..=1439 => format!("{} h", minutes / 60),
        _ => format!("{} d", minutes / 1440),
    }
}
