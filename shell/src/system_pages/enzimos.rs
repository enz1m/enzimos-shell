use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::{Backed, LongJob, enzimos::SystemState};
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::{Space, column};

/// "Version 2" for the second installed version; its raw name otherwise.
fn generation_title(generation: &enzui_shell::system_settings::enzimos::Generation) -> String {
    if generation.ordinal > 0 {
        tr!("shell-version-number", "arg0" => generation.ordinal.to_string())
    } else {
        generation.name.clone()
    }
}

/// What a version's boot state means to the person choosing between them.
fn generation_detail(state: &str, current: bool) -> String {
    match state {
        "booted" => tr("shell-the-version-you-are-using").to_owned(),
        "fallback" => tr("shell-kept-in-case-the-newer-one-has-problems").to_owned(),
        "selected" if !current => tr("shell-starts-after-the-next-restart").to_owned(),
        _ if current => tr("shell-the-version-you-are-using").to_owned(),
        other => other.replace(['-', '_'], " "),
    }
}

impl SettingsApp {
    pub(super) fn enzimos_placeholder<'a, T>(
        &self,
        reading: Option<&Backed<T>>,
        routes: &[&'static str],
    ) -> Element<'a, Parent> {
        let first = match reading {
            None => self.loading_row(),
            Some(Backed::Missing) => self.info_row(
                "info",
                tr("shell-not-available-on-this-system"),
                tr("shell-these-settings-come-from-the-enzimos-system-service"),
            ),
            Some(Backed::Refused(message)) => self.info_row(
                "circle-alert",
                tr("shell-could-not-read-these-settings"),
                message.clone(),
            ),
            Some(Backed::Ready(_)) => Space::new().into(),
        };
        let mut rows = column![self.mark_setting(routes[0], first)].spacing(2);
        for route in &routes[1..] {
            rows = rows.push(self.mark_setting(route, Space::new()));
        }
        rows.into()
    }
    pub(super) fn job_row<'a>(
        &self,
        job: LongJob,
        title: &'static str,
        idle: &'static str,
        start: &'static str,
    ) -> Element<'a, Parent> {
        let state = self.system.jobs.get(&job);
        let writes = self.system_writes();
        let (subtitle, button): (String, Option<(&str, Parent, bool)>) = match (job, state) {
            (_, Some(JobState::Running)) => (
                match job {
                    LongJob::InstallUpdates => {
                        tr("shell-installing-you-can-keep-working-this-can-take-several-minutes")
                            .into()
                    }
                    LongJob::BackupNow => tr("shell-backing-up-you-can-keep-working").into(),
                },
                None,
            ),
            (LongJob::InstallUpdates, Some(JobState::Done(Ok(false)))) => (
                tr("shell-enzimos-is-up-to-date").into(),
                Some((
                    tr("shell-check-again"),
                    Parent::System(Message::StartJob(job)),
                    writes,
                )),
            ),
            (LongJob::InstallUpdates, Some(JobState::Done(Ok(true)))) => (
                tr("shell-the-update-is-installed-as-a-new-version-restart-to-start-using-it")
                    .into(),
                Some((
                    tr("shell-restart-now"),
                    Parent::System(Message::Reboot),
                    writes,
                )),
            ),
            (LongJob::BackupNow, Some(JobState::Done(Ok(_)))) => (
                tr("shell-the-backup-finished").into(),
                Some((start, Parent::System(Message::StartJob(job)), writes)),
            ),
            (_, Some(JobState::Done(Err(error)))) => (
                error.clone(),
                Some((
                    tr("shell-try-again"),
                    Parent::System(Message::StartJob(job)),
                    writes,
                )),
            ),
            (_, None) => (
                idle.into(),
                Some((start, Parent::System(Message::StartJob(job)), writes)),
            ),
        };
        let mut row = crate::settings_app::settings_row(title).subtitle(subtitle);
        if job == LongJob::BackupNow {
            row = row.icon("hard-drive-upload");
        }
        match button {
            Some((label, message, enabled)) => {
                row = row.control(
                    enz_button(label, message)
                        .kind(if label == enzui_shell::i18n::tr("shell-restart-now") {
                            ButtonKind::Primary
                        } else {
                            ButtonKind::Secondary
                        })
                        .size(ButtonSize::Sm)
                        .id(format!("enzimos-job-{job:?}").into())
                        .enabled(enabled)
                        .into_element(),
                );
            }
            None => row = row.control(self.status_badge(tr("shell-working"), BadgeTone::Accent)),
        }
        if job == LongJob::InstallUpdates {
            return marked_row(row.into_element());
        }
        row.into_element()
    }
    fn generation_rows<'a>(&self, state: &SystemState) -> Element<'a, Parent> {
        let mut rows = column![].spacing(2);
        if state.generations.is_empty() {
            rows = rows.push(self.info_row(
                "package",
                tr("shell-the-initial-installation"),
                tr("shell-no-updates-have-been-installed-yet"),
            ));
        }
        let mut generations = state.generations.clone();
        generations.sort_by_key(|g| std::cmp::Reverse(g.ordinal));
        for generation in generations {
            let current = state.current_generation.as_deref() == Some(generation.name.as_str());
            let mut badges = iced::widget::row![].spacing(6);
            if current {
                badges = badges.push(self.status_badge(tr("shell-in-use"), BadgeTone::Success));
            }
            // Confirmed is the normal case; only a version still on trial says so.
            if !generation.confirmed {
                badges = badges
                    .push(self.status_badge(tr("shell-not-confirmed-yet"), BadgeTone::Warning));
            }
            rows = rows.push(
                crate::settings_app::settings_row(generation_title(&generation))
                    .subtitle(generation_detail(&generation.state, current))
                    .icon("package")
                    .control(badges)
                    .into_element(),
            );
        }
        rows.into()
    }
    pub(super) fn updates_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let reading = match self.reading(Panel::Updates) {
            Some(Reading::Updates(r)) => Some(r),
            _ => None,
        };
        let Some(Backed::Ready(state)) = &reading else {
            let routes = ["updates/install", "updates/generations", "updates/recovery"];
            return body
                .push(
                    self.section_card_for(
                        "updates/install",
                        self.enzimos_placeholder(reading.as_ref(), &routes),
                    )
                    .title(tr("shell-enzimos-section"))
                    .into_element(),
                )
                .push(self.firmware_card());
        };
        let install = self.mark_setting(
            "updates/install",
            self.job_row(
                LongJob::InstallUpdates,
                tr("shell-update-enzimos"),
                tr("shell-installs-the-latest-enzimos-as-a-new-version-the-current-one-stays-available"),
                tr("shell-install-updates"),
            ),
        );
        let recovery = self.mark_setting(
            "updates/recovery",
            if state.recovery_images_ready {
                self.info_row(
                    "life-buoy",
                    tr("shell-recovery-image-is-ready"),
                    tr("shell-it-can-repair-this-system-if-an-update-goes-wrong"),
                )
            } else {
                self.info_row(
                    "life-buoy",
                    tr("shell-recovery-image-is-being-prepared"),
                    tr("shell-updates-wait-until-it-is-ready"),
                )
            },
        );
        body.push(
            self.section_card_for("updates/install", column![install].spacing(2))
                .title(tr("shell-enzimos-section"))
                .into_element(),
        )
            .push(
                self.section_card_for("updates/generations", column![self.mark_setting("updates/generations", self.generation_rows(state)), recovery].spacing(2))
                    .title(tr("shell-versions"))
                    .description(tr("shell-each-update-is-a-separate-version-of-the-system-so-an-earlier-one-can-be-sta"))
                    .into_element(),
            )
            .push(self.firmware_card())
    }
}

// The enzimOS update row leads with the system's mark in the row's icon slot.
fn marked_row<'a>(row: Element<'a, Parent>) -> Element<'a, Parent> {
    let tokens = crate::settings_app::settings_row_tokens();
    iced::widget::row![
        iced::widget::container(
            iced::widget::container(
                enzui_shell::brand::mark(tokens.icon_size + 2.0).into_element()
            )
            .center_x(tokens.icon_slot)
        )
        .padding(iced::Padding::ZERO.left(tokens.padding_x)),
        row
    ]
    .align_y(iced::Alignment::Center)
    .into()
}
