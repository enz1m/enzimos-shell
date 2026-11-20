use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use iced::widget::{column, row};

impl SettingsApp {
    pub(in crate::settings_app) fn background_card(&self) -> Element<'_, Parent> {
        let route = "apps/background";
        let state = &self.background;
        let mut body = column![].spacing(8);
        let refresh = enz_button(tr("shell-refresh"), Parent::Background(Message::Refresh))
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .enabled(!state.reading && !state.committing && !state.review.is_open())
            .id("background-refresh".into())
            .into_element();
        // One status row carries Refresh: what is known, or why nothing is.
        let (title, detail) = match &state.snapshot {
            Some(Ok(_)) => (
                tr("shell-background-activity"),
                tr("shell-background-activity-detail").to_owned(),
            ),
            Some(Err(error)) => (tr("shell-background-apps-unavailable"), error.clone()),
            None => (tr("shell-reading-background-apps"), String::new()),
        };
        // The slot fits the whole label: a shrink-sized button in a
        // wrapping row is otherwise elided ("Refre…").
        let button = ButtonSize::Sm.tokens();
        let label = enzui_widgets::text_measure::enz_text_measure_width(
            tr("shell-refresh"),
            enzui_widgets::text_measure::MeasureTypography::new(
                enzui_widgets::text::TextSize::Body,
            )
            .font_family(
                self.tokens()
                    .font_catalog
                    .family(enzui_theme::FontRole::Ui)
                    .to_iced(),
            )
            .weight(enzui_theme::text_defaults::weight(
                enzui_theme::WeightRung::Semibold,
            ))
            .pin_size_px(button.text_size),
        );
        body = body.push(
            self.mark_setting(
                route,
                crate::settings_app::settings_row(title)
                    .subtitle(detail)
                    .subtitle_wrap(true)
                    .icon("app-window")
                    .accessory_width(label + button.padding_x * 2.0 + 8.0)
                    .control(refresh)
                    .into_element(),
            ),
        );
        if let Some(error) = &state.error {
            body = body.push(self.background_note(error.clone()));
        }
        if let Some(notice) = &state.notice {
            body = body.push(self.background_note(notice.clone()));
        }
        let Some(Ok(snapshot)) = &state.snapshot else {
            return self
                .section_card_for(route, body)
                .title(tr("shell-background-apps"))
                .description(tr("shell-background-apps-description"))
                .into_element();
        };
        if !snapshot.catalogue_observation_known || !snapshot.unavailable_declarations.is_empty() {
            body = body.push(self.background_note(tr("shell-some-background-apps-unknown")));
        }
        if let Some(operation) = &snapshot.policy_operation {
            if operation.pending() {
                body = body.push(self.background_note(tr("shell-an-activation-policy-change-is-still-in-progress-wait-before-changing-it-aga")));
            } else if operation.uncertain() {
                body = body.push(self.background_note(tr("shell-a-previous-activation-policy-change-is-unconfirmed-review-the-current-state")));
            }
        }
        if snapshot.apps.is_empty() {
            body = body.push(self.info_row(
                "app-window",
                tr("shell-no-background-apps"),
                tr("shell-apps-that-do-not-report-background-activity-may-still-be-running"),
            ));
        }
        for app in snapshot
            .apps
            .iter()
            .filter(|app| app.running || !app.unit_observation_known)
        {
            body = body.push(self.background_app(app));
        }
        if snapshot
            .apps
            .iter()
            .any(|app| !app.running && app.unit_observation_known)
        {
            body = body.push(
                iced::widget::container(
                    enz_text(tr("shell-not-running-now"))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                )
                .padding(iced::Padding {
                    top: 8.0,
                    left: 10.0,
                    ..Default::default()
                }),
            );
            for app in snapshot
                .apps
                .iter()
                .filter(|app| !app.running && app.unit_observation_known)
            {
                body = body.push(self.background_app(app));
            }
        }
        body = body.push(
            iced::widget::container(
                enz_text(tr("shell-flatpak-background-apps"))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding(iced::Padding {
                top: 8.0,
                left: 10.0,
                ..Default::default()
            }),
        );
        match &snapshot.flatpak_background {
            None => body = body.push(self.background_note(tr("shell-flatpak-background-unknown"))),
            Some(apps) if apps.is_empty() => {
                body = body.push(self.background_note(tr("shell-no-flatpak-background-apps")));
            }
            Some(apps) => {
                for app in apps {
                    let (id, instance) = (app.app_id.clone(), app.instance.clone());
                    let detail = match (app.instance_running, app.message.is_empty()) {
                        (Some(false), _) => {
                            tr("shell-the-portal-still-lists-it-but-it-has-exited").to_owned()
                        }
                        (Some(true), true) => {
                            tr("shell-running-without-a-window-current-work-unknown").to_owned()
                        }
                        (None, true) => {
                            tr("shell-portal-reports-running-without-a-window-current-work-unknown")
                                .to_owned()
                        }
                        (_, false) => {
                            tr!("shell-work-status-reported-by-the-app", "arg0" => app.message.clone())
                        }
                    };
                    let row = crate::settings_app::settings_row(app.app_id.clone())
                        .subtitle(detail)
                        .icon("package");
                    // An exited instance has nothing left to stop.
                    let row = if app.instance_running == Some(false) {
                        row
                    } else {
                        row.control(
                            enz_button(
                                tr("shell-force-quit-2"),
                                Parent::Background(Message::Portal(id, instance.clone())),
                            )
                            .kind(ButtonKind::Quiet)
                            .size(ButtonSize::Sm)
                            .id(format!("background-portal-{instance}").into())
                            .enabled(self.background_live())
                            .into_element(),
                        )
                    };
                    body = body.push(row.into_element());
                }
            }
        }
        self.section_card_for(route, body)
            .title(tr("shell-background-apps"))
            .description(tr("shell-background-apps-description"))
            .into_element()
    }

    fn background_note<'a>(&self, text: impl Into<String>) -> Element<'a, Parent> {
        iced::widget::container(self.caption(text))
            .padding([4, 10])
            .into()
    }

    fn background_live(&self) -> bool {
        self.controls_live()
            && self.page == Page::Apps
            && self.background.client.is_some()
            && self.background.snapshot.as_ref().is_some_and(Result::is_ok)
            && !self.background.committing
            && !self.background.review.is_open()
    }

    fn background_app<'a>(&'a self, app: &'a App) -> Element<'a, Parent> {
        crate::settings_app::settings_row(app.title.clone())
            .subtitle(format!("{} · {}", app.status(), app.work_status()))
            .icon("app-window")
            .control(
                enz_button(
                    tr("shell-manage"),
                    Parent::Background(Message::Inspect(Some(app.app_id.clone()))),
                )
                .kind(ButtonKind::Quiet)
                .size(ButtonSize::Sm)
                .id(format!("background-manage-{}", app.app_id).into())
                .into_element(),
            )
            .into_element()
    }
    pub(in crate::settings_app) fn background_details_dialog(&self) -> Option<Element<'_, Parent>> {
        let id = self.background.inspected.as_ref()?;
        let app = self
            .background
            .snapshot
            .as_ref()?
            .as_ref()
            .ok()?
            .apps
            .iter()
            .find(|app| &app.app_id == id)?;
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![enz_text(app.title.clone()).into_element()],
            iced::widget::scrollable(self.background_app_details(app)).id("background-app-details"),
            vec![
                enz_button(
                    tr("shell-close"),
                    Parent::Background(Message::Inspect(None)),
                )
                .kind(ButtonKind::Secondary)
                .into_element(),
            ],
            12.0,
            560.0,
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(640.0)
                .padding(20.0)
                .on_dismiss(|| Parent::Background(Message::Inspect(None)))
                .close_label(tr("shell-close"))
                .into_element(),
        )
    }

    fn background_app_details<'a>(&'a self, app: &'a App) -> Element<'a, Parent> {
        let mut controls = row![].spacing(8);
        let live = self.background_live();
        let graceful = self
            .background
            .caps
            .as_ref()
            .is_some_and(|caps| caps.graceful_lifecycle_adapter);
        if app.running {
            for mode in [StopMode::Finish, StopMode::Cancel, StopMode::Force] {
                if mode != StopMode::Force && !(graceful && app.can_stop(mode)) {
                    continue;
                }
                let id = app.app_id.clone();
                controls = controls.push(
                    enz_button(
                        format!("{}…", mode.label()),
                        Parent::Background(Message::Stop(id.clone(), mode)),
                    )
                    .kind(ButtonKind::Quiet)
                    .size(ButtonSize::Sm)
                    .enabled(live && app.can_stop(mode))
                    .id(format!("background-stop-{id}-{}", mode.key()).into())
                    .into_element(),
                );
            }
        }
        let status = format!("{} · {}", app.status(), app.work_status());
        let mut body = column![
            crate::settings_app::settings_row(app.title.clone())
                .subtitle(status)
                .icon("app-window")
                .into_element(),
            self.caption(app.app_id.clone()),
            self.caption(app.purpose.clone())
        ]
        .spacing(4);
        if app.running {
            body = body.push(controls);
        }
        if app.running && !app.identity_verified {
            body = body.push(self.caption(tr("shell-stopping-is-unavailable-because-this-app-s-current-process-could-not-be-veri")));
        } else if app.running && !app.graceful_stop_supported {
            body = body.push(self.caption(tr(
                "shell-normal-shutdown-is-unavailable-for-this-app-force-quit-can-interrupt-work",
            )));
        }
        for (key, enabled) in &app.activation_policy {
            let label = match key.as_str() {
                "user" => tr("shell-open-when-you-choose"),
                "autostart" => tr("shell-start-automatically-at-login"),
                "agent" => tr("shell-allow-agent-activation"),
                "scheduled" => tr("shell-allow-scheduled-activation"),
                "search" => tr("shell-allow-search-activation"),
                "ongoing-work" => tr("shell-allow-activation-for-ongoing-work"),
                _ => continue,
            };
            let supported = self
                .background
                .caps
                .as_ref()
                .is_some_and(|caps| policy_available(app, key, caps));
            let detail = if key == "user" {
                tr("shell-opening-the-app-yourself-remains-available")
            } else if !supported {
                tr("shell-saved-preference-changing-this-source-is-unavailable-on-this-system")
            } else {
                tr(
                    "shell-applies-to-future-activation-does-not-stop-current-work-or-grant-tool-access",
                )
            };
            let (id, key) = (app.app_id.clone(), key.clone());
            body = body.push(
                crate::settings_app::settings_row(label)
                    .subtitle(detail)
                    .id(format!("background-policy-{id}-{key}").into())
                    .switch(*enabled, move |enabled| {
                        Parent::Background(Message::Policy(id.clone(), key.clone(), enabled))
                    })
                    .enabled(
                        live && supported
                            && self.background.snapshot.as_ref().is_some_and(|snapshot| {
                                snapshot.as_ref().is_ok_and(|snapshot| {
                                    !snapshot.policy_operation.as_ref().is_some_and(
                                        enzui_shell::background_apps::PolicyOperation::pending,
                                    )
                                })
                            }),
                    )
                    .into_element(),
            );
        }
        iced::widget::container(body)
            .padding(iced::Padding {
                bottom: 8.0,
                ..Default::default()
            })
            .into()
    }

    pub(in crate::settings_app) fn background_dialog(&self) -> Option<Element<'_, Parent>> {
        let pending = self.background.review.value()?;
        let operation = pending.operation;
        let mut body = column![
            enz_text(pending.action.title().to_owned())
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element()
        ]
        .spacing(12);
        let (title, confirm) = match &pending.action {
            Action::Stop { mode, .. } => (mode.label(), mode.label()),
            Action::Portal(_) => (
                tr("shell-force-quit-background-app"),
                tr("shell-force-quit"),
            ),
            Action::Policy { .. } => (tr("shell-change-future-activation"), tr("shell-apply")),
        };
        let warning = match &pending.action {
            Action::Stop {
                mode: StopMode::Force,
                ..
            }
            | Action::Portal(_) => tr(
                "shell-force-quit-can-interrupt-work-and-close-windows-future-activation-settings-s",
            ),
            Action::Stop {
                mode: StopMode::Finish,
                ..
            } => tr(
                "shell-the-app-will-be-asked-to-finish-its-work-then-quit-it-may-remain-running-unt",
            ),
            Action::Stop { .. } => tr(
                "shell-the-app-will-be-asked-to-cancel-its-work-then-quit-partial-results-may-remai",
            ),
            Action::Policy { .. } => tr(
                "shell-only-future-activation-will-change-running-work-and-agent-tool-consent-stay",
            ),
        };
        if pending.action.title() != pending.action.app_id() {
            body = body.push(self.caption(pending.action.app_id().to_owned()));
        }
        body = body.push(self.caption(warning));
        match &pending.result {
            None => body = body.push(self.caption(tr("shell-preparing-a-fresh-review"))),
            Some(Err(error)) => body = body.push(self.caption(error.clone())),
            Some(Ok(review)) => {
                if let Action::Policy { app, requested } = &review.action {
                    for (key, value) in requested
                        .iter()
                        .filter(|(key, value)| app.activation_policy.get(*key) != Some(*value))
                    {
                        let name = match key.as_str() {
                            "autostart" => tr("shell-start-automatically-at-login"),
                            "agent" => tr("shell-allow-agent-activation"),
                            _ => key,
                        };
                        body = body.push(self.caption(format!(
                            "{name}: {}",
                            if *value {
                                enzui_shell::i18n::tr("shell-on")
                            } else {
                                enzui_shell::i18n::tr("shell-off")
                            }
                        )));
                    }
                }
                if let Some(rows) = review.plan["lifecycle"]["consequences"].as_array() {
                    for row in rows {
                        body = body.push(
                            crate::settings_app::settings_row(
                                row["title"].as_str().unwrap_or_default().to_owned(),
                            )
                            .subtitle(row["detail"].as_str().unwrap_or_default().to_owned())
                            .into_element(),
                        );
                    }
                }
                body = body.push(self.caption(tr("shell-this-review-expires-after-30-seconds-the-service-checks-the-current-app-agai")));
            }
        }
        let busy = self.background.committing;
        let destructive = matches!(
            &pending.action,
            Action::Stop {
                mode: StopMode::Force | StopMode::Cancel,
                ..
            } | Action::Portal(_)
        );
        let accept = enz_button(
            if busy { tr("shell-sending") } else { confirm },
            Parent::Background(Message::Confirm(operation)),
        )
        .kind(if destructive {
            ButtonKind::Destructive
        } else {
            ButtonKind::Primary
        })
        .enabled(!busy && pending.result.as_ref().is_some_and(Result::is_ok))
        .id("background-review-confirm".into())
        .into_element();
        let cancel = enz_button(
            tr("shell-keep-current-state"),
            Parent::Background(Message::Cancel(operation)),
        )
        .kind(ButtonKind::Quiet)
        .enabled(!busy)
        .id("background-review-cancel".into())
        .into_element();
        let footer: Element<'_, Parent> = row![iced::widget::space::horizontal(), accept, cancel]
            .spacing(8)
            .into();
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(body)
            .id("background-review-body".into())
            .scrollbar_tokens(self.tokens().scrollbar)
            .into_element();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![
                enz_text(title)
                    .size_px(self.text_px(18.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            ],
            scroll,
            vec![footer],
            16.0,
            (self.viewport.height - 64.0).clamp(120.0, 600.0),
        )
        .into_element();
        let surface = enzui_widgets::modal_surface::enz_modal_surface(content)
            .width(560.0)
            .padding(20.0)
            .close_label(tr("shell-keep-current-state"));
        Some(if busy {
            surface.into_element()
        } else {
            surface
                .on_dismiss(move || Parent::Background(Message::Cancel(operation)))
                .into_element()
        })
    }

    pub(in crate::settings_app) fn background_host<'a>(
        &'a self,
        page: Element<'a, Parent>,
    ) -> Element<'a, Parent> {
        let operation = self
            .background
            .review
            .value()
            .map_or(0, |review| review.operation);
        let mut host = enzui_widgets::modal_host::enz_modal_host(
            page,
            self.background_dialog(),
            move |event| Parent::Background(Message::Host(operation, event)),
        )
        .label(tr("shell-background-app-review"))
        .presentation_open(self.background.review.is_open());
        if self.background.committing {
            host = host.policy(enzui_widgets::modal_host::alert_dismiss_policy());
        }
        if let Some(key) = self.background.review.key() {
            host = host.on_exit_settled_with(move || Parent::Background(Message::Settled(key)));
        }
        if let Some(review) = self.background.review.value() {
            host = host.scoped_to(&review.focus);
        }
        host.into_element()
    }
}
