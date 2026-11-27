use super::*;
use enzui_shell::default_apps::Kind;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    icon::enz_icon,
};
use iced::widget::column;

impl SettingsApp {
    pub(super) fn apps_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "apps/defaults";
        let Some(Reading::Apps(apps)) = self.reading(Panel::Apps) else {
            let loading = |route: &'static str, title: &'static str| {
                self.section_card_for(route, column![self.mark_setting(route, self.loading_row())])
                    .title(title)
                    .into_element()
            };
            return self
                .push_background_card(
                    body.push(loading(route, tr("shell-default-apps")))
                        .push(loading("apps/startup", tr("shell-startup-apps"))),
                )
                .push(loading("apps/permissions", tr("shell-app-permissions")))
                .push(loading("apps/location", tr("shell-location")))
                .push(loading("apps/screen_sharing", tr("shell-screen-sharing")))
                .push(loading("apps/parental", tr("shell-parental-controls")))
                .push(loading("apps/screen_time", tr("shell-screen-time")));
        };
        let mut rows = column![].spacing(2);
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            let row = self.default_app_row(&apps, kind);
            rows = rows.push(if index == 0 {
                self.mark_setting(route, row)
            } else {
                row
            });
        }
        let body = body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-default-apps"))
                .description(tr("shell-which-app-opens-links-and-files-apps-can-still-offer-to-open-other-things"))
                .into_element(),
        )
        .push(self.startup_card(&apps.startup));
        self.push_background_card(body)
            .push(self.permissions_card(&apps.permissions))
            .push(self.location_card(&apps.permissions))
            .push(self.screen_sharing_card(&apps.screen_sharing, &apps.screen_sharing_names))
            .push(self.parental_card(apps.supervised.as_deref()))
            .push(self.screen_time_card(&apps.screen_time, &apps.app_names))
    }
    fn push_background_card<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        if self.offers(enzui_shell::platform::Feature::BackgroundApps) {
            body.push(self.background_card())
        } else {
            body
        }
    }
    fn default_app_row<'a>(
        &'a self,
        apps: &enzui_shell::system_settings::Apps,
        kind: Kind,
    ) -> Element<'a, Parent> {
        let candidates = apps.defaults.apps.get(&kind).map_or(&[][..], Vec::as_slice);
        // The saved choice wins over what this window was started with.
        let current = if kind == Kind::Terminal && !self.settings.compositor.terminal.is_empty() {
            candidates.iter().position(|app| {
                app.program.as_deref() == Some(self.settings.compositor.terminal.as_str())
            })
        } else {
            apps.defaults
                .current
                .get(&kind)
                .and_then(|id| candidates.iter().position(|app| app.id == *id))
        };
        let (glyph, subtitle) = match kind {
            Kind::Web => ("globe", tr("shell-links-and-web-pages")),
            Kind::Mail => ("mail", tr("shell-email-links")),
            Kind::Calendar => ("calendar", tr("shell-invitations-and-calendar-files")),
            Kind::Music => ("music", tr("shell-songs-and-other-audio")),
            Kind::Video => ("film", tr("shell-films-and-clips")),
            Kind::Photos => ("image", tr("shell-pictures")),
            Kind::Files => ("folder", tr("shell-folders")),
            Kind::Text => ("file-text", tr("shell-plain-text-and-notes")),
            Kind::Terminal => ("terminal", tr("shell-super-return-opens-it")),
        };
        let label = |index: usize| {
            let app = &candidates[index];
            if candidates
                .iter()
                .enumerate()
                .any(|(other, candidate)| other != index && candidate.name == app.name)
            {
                format!("{} ({})", app.name, app.id.trim_end_matches(".desktop"))
            } else {
                app.name.clone()
            }
        };
        let key = format!("apps/defaults/{kind:?}");
        let row = crate::settings_app::settings_row(kind.label())
            .subtitle(subtitle)
            .id_prefix(format!("default-app-{kind:?}"));
        let row = if candidates.is_empty() {
            row.control(
                enz_text(tr("shell-none-installed"))
                    .size(TextSize::Footnote)
                    .muted_on(self.tokens().palette.surface)
                    .into_element(),
            )
        } else {
            let open = self.menu.as_deref() == Some(key.as_str());
            let choices = if open {
                candidates
                    .iter()
                    .enumerate()
                    .map(|(index, app)| {
                        let message = match (kind, &app.program) {
                            (Kind::Terminal, Some(program)) => Parent::Change(
                                enzui_shell::settings::Change::Terminal(program.clone()),
                            ),
                            _ => Self::request(Request::DefaultApp {
                                kind,
                                id: app.id.clone(),
                            }),
                        };
                        (label(index), message)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            row.value_menu(
                current.map_or_else(|| tr("shell-choose-2").to_owned(), label),
                open,
                Parent::Menu(key.clone(), true),
                Parent::Menu(key, false),
                choices,
            )
        };
        // The chosen app's own icon leads the row; the type's glyph stands in.
        let palette = self.tokens().palette;
        let icon: Element<'a, Parent> = match current.and_then(|at| candidates[at].icon.as_deref())
        {
            Some(path) => enzui_shell::icon_theme::source(path).into_element(26.0),
            None => enz_icon(glyph).size(18.0).tint(palette.text).into_element(),
        };
        Self::with_app_icon(icon, row)
    }
    /// A settings row led by an app's own icon, aligned with glyph rows.
    fn with_app_icon<'a>(
        icon: Element<'a, Parent>,
        row: enzui_widgets::preferences_row::EnzPreferencesRow<'a, Parent>,
    ) -> Element<'a, Parent> {
        let mut tokens = crate::settings_app::settings_row_tokens();
        tokens.padding_x = 0.0;
        iced::widget::row![
            iced::widget::container(icon).center_x(tokens.icon_slot),
            row.tokens(tokens).into_element(),
        ]
        .spacing(tokens.gap)
        .padding([0.0, 10.0])
        .align_y(iced::Alignment::Center)
        .into()
    }
    fn startup_card<'a>(
        &'a self,
        inventory: &enzui_shell::startup_apps::Inventory,
    ) -> Element<'a, Parent> {
        let route = "apps/startup";
        let mut rows = column![].spacing(2);
        if let Some(error) = &inventory.error {
            rows = rows.push(self.info_row(
                "triangle-alert",
                tr("shell-some-startup-settings-could-not-be-read"),
                error.clone(),
            ));
        }
        for app in &inventory.apps {
            let id = app.id.clone();
            let review = app.review.clone();
            let request = move |enabled| {
                Self::request(Request::StartupApp {
                    id: id.clone(),
                    review: review.clone(),
                    enabled,
                    adding: false,
                })
            };
            let row = crate::settings_app::settings_row(app.name.clone())
                .subtitle(app.reason.clone().unwrap_or_default())
                .icon("app-window")
                .id_prefix(format!("startup-{}", app.id))
                .switch(app.enabled, request)
                .enabled(self.controls_live() && (app.enabled || app.can_enable))
                .into_element();
            rows = rows.push(row);
        }
        if inventory.apps.is_empty() {
            rows = rows.push(self.info_row(
                "app-window",
                tr("shell-no-startup-apps"),
                tr("shell-no-startup-apps-detail"),
            ));
        }
        if !inventory.candidates.is_empty() {
            let picker = enz_button(
                tr("shell-choose-an-app"),
                Parent::Startup(super::super::settings_startup::Message::Open(
                    inventory.candidates.clone(),
                )),
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .id("startup-add-app".into())
            .enabled(self.controls_live() && !self.system.busy)
            .into_element();
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-add-an-application"))
                    .subtitle(tr("shell-add-startup-app-detail"))
                    .icon("plus")
                    .control(picker)
                    .into_element(),
            );
        }
        self.section_card_for(route, column![self.mark_setting(route, rows)])
            .title(tr("shell-startup-apps"))
            .description(tr("shell-startup-apps-description"))
            .into_element()
    }
    fn screen_time_card<'a>(
        &'a self,
        history: &enzui_shell::screen_time::History,
        names: &std::collections::BTreeMap<String, String>,
    ) -> Element<'a, Parent> {
        use enzui_shell::screen_time::duration_text;
        let route = "apps/screen_time";
        let on = self.settings.screen_time;
        let mut rows = column![self.switch_setting(route, on, |on| Parent::Change(
            enzui_shell::settings::Change::ScreenTime(on)
        ))]
        .spacing(2);
        if on {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            match history.days.get(&today) {
                None => {
                    rows = rows.push(self.info_row(
                        "clock",
                        tr("shell-nothing-recorded-today-yet"),
                        tr("shell-time-is-added-each-minute-the-desktop-is-unlocked"),
                    ));
                }
                Some(day) => {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr("shell-today"))
                            .subtitle(tr("shell-unlocked"))
                            .icon("chart-column")
                            .control(enz_text(duration_text(day.total)).into_element())
                            .into_element(),
                    );
                    let mut apps: Vec<(&String, &u32)> = day.apps.iter().collect();
                    apps.sort_by(|a, b| b.1.cmp(a.1));
                    for (class, seconds) in apps.into_iter().take(5) {
                        let name = names.get(class).cloned().unwrap_or_else(|| class.clone());
                        rows = rows.push(
                            iced::widget::container(
                                crate::settings_app::settings_row(name)
                                    .control(
                                        enz_text(duration_text(*seconds))
                                            .size(TextSize::Footnote)
                                            .into_element(),
                                    )
                                    .into_element(),
                            )
                            .padding(iced::Padding {
                                left: 34.0,
                                ..Default::default()
                            }),
                        );
                    }
                }
            }
            let week: Vec<String> = history
                .days
                .iter()
                .rev()
                .filter_map(|(date, day)| {
                    enzui_shell::screen_time::date_label(date)
                        .map(|label| format!("{label} {}", duration_text(day.total)))
                })
                .take(7)
                .collect();
            if week.len() > 1 {
                rows = rows.push(
                    iced::widget::container(
                        self.caption(tr!("shell-last-days", "arg0" => week.join(" · "))),
                    )
                    .padding([6, 12]),
                );
            }
        }
        self.section_card_for(route, rows)
            .title(tr("shell-screen-time"))
            .description(tr("shell-screen-time-description"))
            .into_element()
    }
    fn parental_card<'a>(
        &'a self,
        supervised: Option<&[enzui_shell::parental::Account]>,
    ) -> Element<'a, Parent> {
        use enzui_widgets::button::{ButtonKind, enz_button};
        let route = "apps/parental";
        let mut rows = column![].spacing(2);
        match (supervised, &self.system.parental) {
            (None, _) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "users",
                        tr("shell-parental-controls-are-not-available"),
                        tr("shell-this-system-s-accounts-service-has-no-parental-controls"),
                    ),
                ));
            }
            (Some([]), _) => {
                rows = rows.push(self.mark_setting(route, self.info_row("users", tr("shell-no-standard-accounts"), tr("shell-parental-controls-apply-to-standard-accounts-administrators-are-never-restri"))));
            }
            (Some(accounts), None) => {
                for (index, account) in accounts.iter().enumerate() {
                    let title = if account.real_name.is_empty() {
                        account.name.clone()
                    } else {
                        account.real_name.clone()
                    };
                    let row = crate::settings_app::settings_row(title)
                        .subtitle(account.name.clone())
                        .icon("user")
                        .control(
                            enz_button(
                                tr("shell-set-up"),
                                Parent::System(Message::ParentalOpen(account.clone())),
                            )
                            .kind(ButtonKind::Secondary)
                            .id(format!("parental-open-{index}").into())
                            .into_element(),
                        )
                        .into_element();
                    rows = rows.push(if index == 0 {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                }
            }
            (Some(_), Some(edit)) => {
                let name = if edit.account.real_name.is_empty() {
                    edit.account.name.clone()
                } else {
                    edit.account.real_name.clone()
                };
                rows = rows.push(
                    self.mark_setting(
                        route,
                        crate::settings_app::settings_row(
                            tr!("shell-apps-may-open", "arg0" => name),
                        )
                        .icon("user")
                        .control(
                            enz_button(tr("shell-done"), Parent::System(Message::ParentalClose))
                                .id("parental-done".into())
                                .kind(ButtonKind::Secondary)
                                .into_element(),
                        )
                        .into_element(),
                    ),
                );
                match &edit.controls {
                    None => {
                        rows = rows.push(self.info_row(
                            "shield",
                            tr("shell-reading-the-account-s-controls"),
                            tr("shell-an-administrator-may-be-asked-to-approve"),
                        ));
                    }
                    Some(Err(error)) => {
                        rows = rows.push(self.info_row(
                            "triangle-alert",
                            tr("shell-the-controls-could-not-be-read"),
                            error.clone(),
                        ));
                    }
                    Some(Ok(controls)) => {
                        for (index, (app, key)) in edit.apps.iter().enumerate() {
                            let allowed = controls
                                .filter
                                .allows(app.flatpak.as_deref(), app.program.as_deref());
                            let key = key.clone();
                            rows = rows.push(
                                crate::settings_app::settings_row(app.name.clone())
                                    .id_prefix(format!("parental-app-{index}"))
                                    .switch(allowed, move |on| {
                                        Parent::System(Message::ParentalApp(key.clone(), on))
                                    })
                                    .into_element(),
                            );
                        }
                        rows = rows.push(
                            crate::settings_app::settings_row(tr("shell-install-apps"))
                                .subtitle(tr("shell-let-this-account-install-apps-for-itself"))
                                .id_prefix("parental-installing")
                                .switch(controls.allow_installing, |on| {
                                    Parent::System(Message::ParentalInstalling(on))
                                })
                                .into_element(),
                        );
                        let status = match &edit.saved {
                            Some(Ok(())) => {
                                tr("shell-saved-the-desktop-applies-it-within-a-minute").to_owned()
                            }
                            Some(Err(error)) => error.clone(),
                            None => String::new(),
                        };
                        rows = rows.push(
                            iced::widget::row![
                                enz_text(status)
                                    .size(TextSize::Footnote)
                                    .muted_on(self.tokens().palette.surface)
                                    .width(iced::Length::Fill)
                                    .into_element(),
                                enz_button(
                                    if edit.saving {
                                        enzui_shell::i18n::tr("shell-saving")
                                    } else {
                                        enzui_shell::i18n::tr("shell-save")
                                    },
                                    Parent::System(Message::ParentalSave)
                                )
                                .id("parental-save".into())
                                .enabled(!edit.saving)
                                .into_element(),
                            ]
                            .spacing(12)
                            .padding([8, 12])
                            .align_y(iced::Alignment::Center),
                        );
                    }
                }
            }
        }
        self.section_card_for(route, rows)
            .title(tr("shell-parental-controls"))
            .description(tr("shell-parental-controls-description"))
            .into_element()
    }
    fn permissions_card<'a>(
        &'a self,
        permissions: &enzui_shell::app_permissions::Permissions,
    ) -> Element<'a, Parent> {
        let route = "apps/permissions";
        let mut rows = column![].spacing(2);
        match &permissions.apps {
            None => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "package",
                        tr("shell-flatpak-is-not-installed"),
                        tr("shell-permissions-apply-to-apps-installed-with-flatpak"),
                    ),
                ));
            }
            Some(_) if permissions.error.is_some() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "triangle-alert",
                        tr("shell-permissions-cannot-be-read"),
                        permissions.error.clone().unwrap_or_default(),
                    ),
                ));
            }
            Some(apps) if apps.is_empty() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "package",
                        tr("shell-no-flatpak-apps"),
                        tr("shell-apps-installed-with-flatpak-appear-here"),
                    ),
                ));
            }
            Some(apps) => {
                if let Some(error) = &permissions.portal_error {
                    rows = rows.push(self.info_row(
                        "shield-alert",
                        tr("shell-remembered-grants-unavailable"),
                        error.clone(),
                    ));
                }
                let field = enzui_widgets::search_field::enz_search_field(
                    self.sandbox_filter(),
                    |filter| {
                        Parent::Sandbox(super::super::sandbox_settings::Message::Filter(filter))
                    },
                )
                .placeholder(tr("shell-find-an-app"))
                .id("app-permissions-find")
                .into_element();
                rows = rows.push(
                    iced::widget::container(self.mark_setting(route, field)).padding(
                        iced::Padding {
                            bottom: 6.0,
                            ..Default::default()
                        },
                    ),
                );
                let shown = enzui_shell::app_permissions::matching(apps, self.sandbox_filter());
                if shown.is_empty() {
                    rows = rows.push(self.info_row(
                        "search",
                        tr("shell-no-app-matches"),
                        tr("shell-try-part-of-the-app-s-name"),
                    ));
                }
                for app in shown {
                    rows = rows.push(self.permission_app_row(app));
                }
            }
        }
        self.section_card_for(route, rows)
            .title(tr("shell-app-permissions"))
            .description(tr("shell-app-permissions-description"))
            .into_element()
    }
    fn location_card<'a>(
        &'a self,
        permissions: &enzui_shell::app_permissions::Permissions,
    ) -> Element<'a, Parent> {
        use enzui_shell::location::Mode;
        let route = "apps/location";
        if !permissions.location_provider {
            let row = self.info_row(
                "map-pin-off",
                tr("shell-location-services-are-off"),
                tr("shell-no-location-provider-is-installed-so-no-app-can-get-your-location"),
            );
            return self
                .section_card_for(route, column![self.mark_setting(route, row)])
                .title(tr("shell-location"))
                .into_element();
        }
        let mode = self.system.location;
        let label = |mode: Mode| match mode {
            Mode::Off => tr("shell-location-off"),
            Mode::Approximate => tr("shell-location-approximate"),
            Mode::Precise => tr("shell-location-precise"),
        };
        let detail = match mode {
            Mode::Off => tr("shell-location-off-detail"),
            Mode::Approximate => tr("shell-location-approximate-detail"),
            Mode::Precise => tr("shell-location-precise-detail"),
        };
        let choices: Vec<(String, Parent)> = Mode::ALL
            .into_iter()
            .map(|choice| {
                (
                    label(choice).to_owned(),
                    Parent::System(Message::Location(choice)),
                )
            })
            .collect();
        let main = crate::settings_app::settings_row(tr("shell-location-services"))
            .subtitle(detail)
            .subtitle_wrap(true)
            .icon(if mode == Mode::Off {
                "map-pin-off"
            } else {
                "map-pin"
            })
            .value_menu(
                label(mode).to_owned(),
                self.menu.as_deref() == Some(route),
                Parent::Menu(route.into(), true),
                Parent::Menu(route.into(), false),
                choices,
            )
            .into_element();
        let mut rows = column![self.mark_setting(route, main)].spacing(2);
        if mode != Mode::Off {
            rows = rows.push(self.info_row(
                "shield",
                tr("shell-location-apps-ask"),
                tr("shell-location-apps-ask-detail"),
            ));
        }
        if let Some(error) = &self.system.location_error {
            rows = rows.push(self.info_row(
                "triangle-alert",
                tr("shell-location-could-not-be-saved"),
                error,
            ));
        }
        self.section_card_for(route, rows)
            .title(tr("shell-location"))
            .description(tr("shell-location-description"))
            .into_element()
    }
    fn screen_sharing_card<'a>(
        &'a self,
        listing: &enzui_shell::screencast_grants::Listing,
        names: &std::collections::BTreeMap<String, String>,
    ) -> Element<'a, Parent> {
        use enzui_shell::screencast_grants::Source;
        let route = "apps/screen_sharing";
        let mut rows = column![].spacing(2);
        if let Some(error) = &listing.error {
            rows = rows.push(self.mark_setting(
                route,
                self.info_row(
                    "triangle-alert",
                    tr("shell-screen-sharing-grants-unavailable"),
                    error.clone(),
                ),
            ));
        } else if listing.grants.is_empty() {
            rows = rows.push(self.mark_setting(
                route,
                self.info_row(
                    "monitor-check",
                    tr("shell-no-app-shares-your-screen-after-a-restart"),
                    tr("shell-no-app-shares-your-screen-after-a-restart-detail"),
                ),
            ));
        }
        for (index, grant) in listing.grants.iter().enumerate() {
            let app = names
                .get(&grant.app_id)
                .cloned()
                .unwrap_or_else(|| grant.app_id.clone());
            // The backend's marker for outputs that report no make, model or serial.
            let monitor =
                if grant.monitor.trim().is_empty() || grant.monitor == "unidentified output" {
                    grant.output.clone()
                } else {
                    grant.monitor.clone()
                };
            let mut subtitle = match &grant.source {
                Source::Screen => tr!("shell-whole-screen-of", "arg0" => monitor),
                Source::Region { w, h, .. } => {
                    tr!("shell-region-of", "arg0" => format!("{w}×{h}"), "arg1" => monitor)
                }
            };
            if grant.keyboard() || grant.pointer() {
                subtitle = format!("{subtitle} · {}", tr("shell-and-remote-control"));
            }
            let revoke = enz_button(
                tr("shell-revoke"),
                Self::request(Request::ScreenSharingRevoke(grant.id.clone())),
            )
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .enabled(self.controls_live())
            .aria_label(tr!("shell-a11y-revoke-screen-sharing-for", "arg0" => app.clone()))
            .id(format!("screen-sharing-revoke-{index}").into())
            .into_element();
            let row = crate::settings_app::settings_row(app)
                .subtitle(subtitle)
                .subtitle_wrap(true)
                .icon("monitor-up")
                .id_prefix(format!("screen-sharing-row-{index}"))
                .control(revoke)
                .into_element();
            rows = rows.push(if index == 0 {
                self.mark_setting(route, row)
            } else {
                row
            });
        }
        self.section_card_for(route, rows)
            .title(tr("shell-screen-sharing"))
            .description(tr("shell-screen-sharing-description"))
            .into_element()
    }
    fn permission_app_row<'a>(
        &'a self,
        app: &enzui_shell::app_permissions::App,
    ) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        let icon: Element<'a, Parent> = match &app.icon {
            Some(path) => enzui_shell::icon_theme::source(path).into_element(26.0),
            None => enz_icon("package")
                .size(18.0)
                .tint(palette.text)
                .into_element(),
        };
        let asked: Vec<&str> = app
            .grants
            .iter()
            .filter(|(_, allowed)| *allowed)
            .map(|(kind, _)| kind.label())
            .collect();
        let summary = match (asked.is_empty(), app.sandbox.is_empty()) {
            (true, true) => {
                tr("shell-no-common-access-listed-review-permissions-for-full-details").to_owned()
            }
            (true, false) => app.sandbox.join(", "),
            (false, true) => asked.join(", "),
            (false, false) => format!("{} · {}", asked.join(", "), app.sandbox.join(", ")),
        };
        let open = Parent::Sandbox(super::super::sandbox_settings::Message::Toggle(
            app.id.clone(),
        ));
        let button = enz_button(tr("shell-permissions"), open)
            .kind(ButtonKind::Secondary)
            .size(ButtonSize::Sm)
            .enabled(self.controls_live())
            .id(format!("app-permissions-{}", app.id).into())
            .into_element();
        Self::with_app_icon(
            icon,
            crate::settings_app::settings_row(app.name.clone())
                .subtitle(summary)
                .id_prefix(format!("app-permissions-row-{}", app.id))
                .control(button),
        )
    }
    pub(in crate::settings_app) fn app_permissions_dialog(&self) -> Option<Element<'_, Parent>> {
        let id = self.sandbox_open()?;
        let Some(Reading::Apps(apps)) = self.reading(Panel::Apps) else {
            return None;
        };
        let app = apps
            .permissions
            .apps
            .as_ref()?
            .iter()
            .find(|app| app.id == id)?
            .clone();
        let palette = self.tokens().palette;
        let worker = enzui_shell::sandbox_policy::worker().is_some();
        let close = Parent::Sandbox(super::super::sandbox_settings::Message::Host(
            self.sandbox_generation(),
            enzui_widgets::modal_host::ModalHostEvent::Dismissed(
                enzui_core::modal::DismissCause::CloseButton,
            ),
        ));
        let icon: Element<'_, Parent> = match &app.icon {
            Some(path) => enzui_shell::icon_theme::source(path).into_element(40.0),
            None => enz_icon("package")
                .size(32.0)
                .tint(palette.text_muted)
                .into_element(),
        };
        let header: Element<'_, Parent> = iced::widget::row![
            icon,
            column![
                enz_text(app.name.clone())
                    .size_px(self.text_px(18.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
                enz_text(app.id.clone())
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .into_element(),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .into();
        let body: Element<'_, Parent> = match self.sandbox_details().filter(|_| worker) {
            Some(details) => details,
            None => {
                let mut body = column![
                    enz_text(tr("shell-saved-portal-choices"))
                        .size_px(self.text_px(15.0))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element()
                ]
                .spacing(4);
                for kind in enzui_shell::app_permissions::Kind::ALL {
                    let row = crate::settings_app::settings_row(kind.label())
                        .id_prefix(format!("permission-{kind:?}"));
                    let row = match app.grants.iter().find(|(known, _)| *known == kind) {
                        Some((_, allowed)) => {
                            row.subtitle(if *allowed { enzui_shell::i18n::tr("shell-remembered-allowed") } else { enzui_shell::i18n::tr("shell-remembered-denied") })
                        }
                        None => row.subtitle(if apps.permissions.portal_error.is_some() { tr("shell-remembered-answer-unavailable") } else { tr("shell-no-recognized-yes-no-answer-full-details-need-the-permissions-service") }),
                    };
                    body = body.push(row.into_element());
                }
                body = body.push(
                    iced::widget::container(
                        enz_text(tr("shell-installed-access"))
                            .size_px(self.text_px(15.0))
                            .weight_rung(enzui_theme::WeightRung::Semibold)
                            .into_element(),
                    )
                    .padding(iced::Padding {
                        top: 8.0,
                        ..Default::default()
                    }),
                );
                if app.sandbox.is_empty() {
                    body = body.push(crate::settings_app::settings_row(tr("shell-no-common-access-listed-complete-policy-needs-the-permissions-service")).into_element());
                }
                for access in &app.sandbox {
                    body = body.push(crate::settings_app::settings_row(*access).into_element());
                }
                body = body.push(
                    enz_text(tr("shell-reviewing-or-changing-access-needs-the-enzimos-app-permissions-service-which"))
                        .size(TextSize::Footnote)
                        .muted_on(palette.surface)
                        .wrap(true)
                        .into_element(),
                );
                body.into()
            }
        };
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(Element::from(
            iced::widget::container(body).padding(iced::Padding {
                right: 12.0,
                ..Default::default()
            }),
        ))
        .id("app-permissions-body".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .into_element();
        let footer: Element<'_, Parent> = iced::widget::row![
            iced::widget::space::horizontal(),
            enz_button(tr("shell-done"), close.clone())
                .kind(ButtonKind::Primary)
                .id("app-permissions-done".into())
                .into_element(),
        ]
        .into();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![header],
            scroll,
            vec![footer],
            16.0,
            (self.viewport_height - 64.0).clamp(120.0, 640.0),
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(560.0)
                .padding(20.0)
                .on_dismiss(move || close.clone())
                .close_label(tr!("shell-close-permissions", "arg0" => app.name.clone()))
                .into_element(),
        )
    }
}
