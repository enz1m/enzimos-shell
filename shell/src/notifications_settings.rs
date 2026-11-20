use super::{Message, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::{
    notifications::{
        model::Rule,
        preferences::{Change, Position},
    },
    settings,
};
use enzui_widgets::button::{ButtonKind, enz_button};
use iced::{Element, widget::column};

fn change(value: Change) -> Message {
    Message::Change(settings::Change::Notification(value))
}
pub(super) async fn send_preview() -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let connection = zbus::Connection::session()
            .await
            .map_err(|_| tr("shell-desktop-notification-service-is-unavailable").to_owned())?;
        let proxy = zbus::Proxy::new(
            &connection,
            enzui_shell::notifications::server::SERVICE,
            enzui_shell::notifications::server::PATH,
            enzui_shell::notifications::server::SERVICE,
        )
        .await
        .map_err(|_| tr("shell-cannot-connect-to-this-desktop-s-notification-service").to_owned())?;
        // A varied set, so every style is judged against actions, a reply, a
        // critical message and a plain one.
        let samples: [(&str, &str, &str, &[&str], u8); 4] = [
            (tr("shell-calendar"), tr("shell-design-review-in-10-minutes"), tr("shell-room-4-with-maya-jon-and-three-others-notes-are-in-the-shared-doc"), &["snooze", tr("shell-snooze"), "open", tr("shell-open")], 1),
            (tr("shell-messages"), "Maya", tr("shell-could-you-look-at-the-toast-spacing-before-the-review-i-left-a-few-notes"), &["inline-reply", tr("shell-reply"), "read", tr("shell-mark-as-read")], 1),
            (tr("shell-power"), tr("shell-battery-low"), tr("shell-12-remaining-plug-in-soon-to-keep-working"), &[], 2),
            (tr("shell-enzui-settings"), tr("shell-notification-preview"), tr("shell-this-is-how-a-plain-notification-appears-on-your-desktop"), &[], 1),
        ];
        for (app, summary, body, actions, urgency) in samples {
            let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
            hints.insert("urgency", zbus::zvariant::Value::U8(urgency));
            let _: u32 = proxy
                .call("Notify", &(app, 0_u32, "dialog-information", summary, body, actions.to_vec(), hints, -1_i32))
                .await
                .map_err(|error| format!("{}: {error}", tr("shell-the-desktop-did-not-accept-the-preview-notification")))?;
        }
        Ok(())
    })
    .await
    .map_err(|_| tr("shell-preview-notification-timed-out").to_owned())?
}
impl SettingsApp {
    pub(super) fn notification_settings(&self) -> Element<'_, Message> {
        let p = &self.settings.notifications;
        let policy = &p.policy;
        let enabled = !self.closing;
        let delivery = column![
            self.switch_setting("notifications/dnd", policy.dnd, |value| change(
                Change::Dnd(value)
            )),
            // Only Do Not Disturb holds alerts back, so this matters only then.
            self.dependent(policy.dnd, || self.switch_setting_noted(
                "notifications/critical",
                policy.allow_critical,
                tr("shell-urgent-alerts-like-a-nearly-empty-battery-still-appear"),
                |value| change(Change::Critical(value)),
            )),
        ]
        .spacing(2);
        let positions = [
            (Position::BarEnd, tr("shell-follow-bar")),
            (Position::TopLeft, tr("shell-top-left")),
            (Position::TopRight, tr("shell-top-right")),
            (Position::BottomLeft, tr("shell-bottom-left")),
            (Position::BottomRight, tr("shell-bottom-right")),
        ];
        let appearance = column![
            self.toast_placement_preview(),
            self.choice_setting(
                "notifications/position",
                positions
                    .iter()
                    .find(|(value, _)| *value == p.position)
                    .expect("all notification positions have a display label")
                    .1,
                positions
                    .into_iter()
                    .map(|(value, label)| (label.into(), change(Change::Position(value))))
                    .collect()
            ),
            self.notification_style_preview(),
            self.number_setting(
                "notifications/duration",
                (policy.toast_ms / 1000) as i32,
                1,
                30,
                " s",
                |value| change(Change::Duration(value as u32 * 1000))
            ),
            self.number_setting(
                "notifications/width",
                p.width.into(),
                300,
                640,
                " px",
                |value| change(Change::Width(value as u16))
            ),
            self.number_setting(
                "notifications/count",
                p.maximum_toasts.into(),
                1,
                5,
                "",
                |value| change(Change::MaximumToasts(value as u8))
            ),
            self.switch_setting("notifications/images", p.images, |value| change(
                Change::Images(value)
            )),
            crate::settings_app::settings_row(tr("shell-try-it"))
                .icon("bell-ring")
                .subtitle(tr(
                    "shell-shows-a-few-sample-notifications-with-these-settings"
                ))
                .control(
                    enz_button(tr("shell-show-preview"), Message::PreviewNotification)
                        .kind(ButtonKind::Secondary)
                        .enabled(enabled && !self.preview_notification_busy)
                        .id("notifications-preview".into())
                        .into_element(),
                )
                .into_element(),
        ]
        .spacing(2);
        // One control for how long history is kept; a value stored by an
        // older version still shows under its own label.
        let mut retention = vec![60, 1440, 10080, 43200];
        if !retention.contains(&policy.history_minutes) {
            retention.push(policy.history_minutes);
            retention.sort_unstable();
        }
        let history = column![
            self.switch_setting("notifications/history", policy.history, |value| change(
                Change::History(value)
            )),
            self.dependent(policy.history, || column![
                self.choice_setting(
                    "notifications/retention",
                    retention_label(policy.history_minutes),
                    retention
                        .into_iter()
                        .map(|value| (
                            retention_label(value),
                            change(Change::HistoryMinutes(value))
                        ))
                        .collect()
                ),
                self.number_setting(
                    "notifications/budget",
                    policy.history_mib as i32,
                    1,
                    16,
                    " MiB",
                    |value| change(Change::HistoryMiB(value as u32))
                ),
                // Turning history off already empties it.
                self.action_setting(
                    "notifications/clear",
                    enzui_shell::i18n::tr("shell-clear"),
                    "trash-2",
                    ButtonKind::QuietDestructive,
                    change(Change::ClearHistory),
                ),
            ]
            .spacing(2)),
        ]
        .spacing(2);
        // One row per app that has notified: what happens to its
        // notifications, chosen from a menu (or forgotten).
        let mut apps = column![].spacing(2);
        for (principal, application) in &p.applications {
            let rule = policy.rule(principal);
            let key = format!("notifications/app/{principal}");
            let label = |rule: Rule| match rule {
                Rule::Allow => tr("shell-show"),
                Rule::Block => tr("shell-mute"),
                Rule::Priority => tr("shell-always-show"),
            };
            let subtitle = match rule {
                Rule::Allow => tr("shell-shows-notifications"),
                Rule::Block => tr("shell-notifications-are-muted"),
                Rule::Priority => tr("shell-shows-notifications-even-during-do-not-disturb"),
            };
            let mut choices: Vec<(String, Message)> = [Rule::Allow, Rule::Block, Rule::Priority]
                .into_iter()
                .map(|choice| {
                    (
                        label(choice).to_owned(),
                        change(Change::Application {
                            principal: principal.clone(),
                            application: application.clone(),
                            rule: choice,
                        }),
                    )
                })
                .collect();
            choices.push((
                tr("shell-remove-from-list").to_owned(),
                change(Change::Forget(principal.clone())),
            ));
            apps = apps.push(
                crate::settings_app::settings_row(application.label.clone())
                    .subtitle(subtitle)
                    .icon(application.icon.as_str())
                    .id_prefix(format!("notification-app-{principal}"))
                    .value_menu(
                        label(rule),
                        self.menu.as_deref() == Some(key.as_str()),
                        Message::Menu(key.clone(), true),
                        Message::Menu(key, false),
                        choices,
                    )
                    .enabled(enabled)
                    .into_element(),
            );
        }
        let app_controls = if p.applications.is_empty() {
            column![
                crate::settings_app::settings_row(tr("shell-no-apps-have-sent-notifications-yet"))
                    .icon("bell")
                    .subtitle(tr("shell-apps-appear-here-after-their-first-notification"))
                    .into_element()
            ]
        } else {
            apps
        };
        column![
            self.section_card_for("notifications/dnd", delivery)
                .title(enzui_shell::i18n::tr("shell-delivery"))
                .description(tr(
                    "shell-do-not-disturb-hides-pop-ups-they-still-wait-in-the-notification-center"
                ))
                .into_element(),
            self.section_card_for("notifications/position", appearance)
                .title(tr("shell-toast-appearance"))
                .description(tr(
                    "shell-how-notification-pop-ups-look-and-where-they-appear"
                ))
                .into_element(),
            self.section_card_for("notifications/history", history)
                .title(enzui_shell::i18n::tr("shell-history-privacy"))
                .description(tr(
                    "shell-keeps-closed-notifications-text-and-pictures-so-you-can-find-them-again"
                ))
                .into_element(),
            self.mark_setting(
                "notifications/applications",
                self.section_card_for("notifications/applications", app_controls)
                    .title(enzui_shell::i18n::tr("shell-apps"))
                    .description(tr("shell-choose-how-each-app-notifies-you"))
                    .into_element()
            ),
        ]
        .spacing(32)
        .into()
    }
}

fn retention_label(minutes: u32) -> String {
    use enzui_shell::i18n::count;
    match minutes {
        m if m >= 1440 && m % 1440 == 0 => count("duration-days", m / 1440),
        m if m >= 60 && m % 60 == 0 => count("duration-hours", m / 60),
        m => count("duration-minutes", m),
    }
}
