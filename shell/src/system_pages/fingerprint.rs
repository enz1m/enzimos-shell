use super::*;
use enzui_shell::fingerprint::{FINGERS, Overview, Progress, finger_label};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::Length;
use iced::widget::{column, row};

/// An enrollment in progress.
pub(super) struct Enrolling {
    finger: String,
    stages: i32,
    passed: i32,
    hint: Option<&'static str>,
}

#[derive(Default)]
pub(super) struct Fingerprint {
    overview: Option<Result<Overview, String>>,
    loading: bool,
    epoch: u64,
    /// The finger chosen for the next enrollment.
    finger: Option<String>,
    enrolling: Option<Enrolling>,
    deleting: bool,
    error: Option<String>,
    added: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum FingerprintMessage {
    Read(u64, Result<Overview, String>),
    Choose(String),
    Enroll,
    Progress(Progress),
    Delete(Option<String>),
    Deleted(Result<(), String>),
}

fn wrap(message: FingerprintMessage) -> Parent {
    Parent::System(Message::Fingerprint(message))
}

impl SettingsApp {
    pub(super) fn fingerprint_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.fingerprint;
        if self.system.backend.demo {
            if state.overview.is_none() {
                state.overview = Some(Ok(enzui_shell::fingerprint::demo()));
            }
            return Task::none();
        }
        if state.loading
            || state.enrolling.is_some()
            || state.deleting
            || !(force || state.overview.is_none())
        {
            return Task::none();
        }
        let Some(epoch) = state.epoch.checked_add(1) else {
            return Task::none();
        };
        state.epoch = epoch;
        state.loading = true;
        Task::perform(enzui_shell::fingerprint::read(), move |r| {
            wrap(FingerprintMessage::Read(epoch, r))
        })
    }

    pub(super) fn fingerprint_message(&mut self, message: FingerprintMessage) -> Task<Parent> {
        let demo = self.system.backend.demo;
        // A read-only preview shows this computer's hardware without changing it.
        if !self.system.backend.writes_allowed
            && matches!(
                message,
                FingerprintMessage::Enroll | FingerprintMessage::Delete(_)
            )
        {
            self.error = Some(tr("shell-system-changes-are-off-in-this-preview").into());
            return Task::none();
        }
        let state = &mut self.system.fingerprint;
        match message {
            FingerprintMessage::Read(epoch, result) => {
                if epoch == state.epoch {
                    state.loading = false;
                    state.overview = Some(result);
                }
                Task::none()
            }
            FingerprintMessage::Choose(finger) => {
                self.menu = None;
                if FINGERS.contains(&finger.as_str()) {
                    self.system.fingerprint.finger = Some(finger);
                }
                Task::none()
            }
            FingerprintMessage::Enroll => {
                if state.enrolling.is_some() || state.deleting || self.closing {
                    return Task::none();
                }
                let enrolled = state
                    .overview
                    .as_ref()
                    .and_then(|o| o.as_ref().ok())
                    .map(|o| o.enrolled.clone())
                    .unwrap_or_default();
                let Some(finger) = state
                    .finger
                    .clone()
                    .filter(|f| !enrolled.contains(f))
                    .or_else(|| {
                        FINGERS
                            .iter()
                            .find(|f| !enrolled.iter().any(|e| e == **f))
                            .map(|f| (*f).to_owned())
                    })
                else {
                    return Task::none();
                };
                state.error = None;
                state.added = None;
                state.enrolling = Some(Enrolling {
                    finger: finger.clone(),
                    stages: -1,
                    passed: 0,
                    hint: None,
                });
                if demo {
                    return Task::batch([
                        Task::done(wrap(FingerprintMessage::Progress(Progress::Started(5)))),
                        Task::done(wrap(FingerprintMessage::Progress(Progress::Stage))),
                        Task::done(wrap(FingerprintMessage::Progress(Progress::Stage))),
                    ]);
                }
                Task::run(enzui_shell::fingerprint::enroll(finger), |progress| {
                    wrap(FingerprintMessage::Progress(progress))
                })
            }
            FingerprintMessage::Progress(progress) => {
                let Some(enrolling) = state.enrolling.as_mut() else {
                    return Task::none();
                };
                match progress {
                    Progress::Started(stages) => enrolling.stages = stages,
                    Progress::Stage => {
                        enrolling.passed += 1;
                        enrolling.hint = None;
                    }
                    Progress::Retry(hint) => enrolling.hint = Some(hint),
                    Progress::Done => {
                        state.added = Some(finger_label(&enrolling.finger).to_owned());
                        state.enrolling = None;
                        state.finger = None;
                        return self.fingerprint_read(true);
                    }
                    Progress::Failed(error) => {
                        state.error = Some(error);
                        state.enrolling = None;
                        return self.fingerprint_read(true);
                    }
                }
                Task::none()
            }
            FingerprintMessage::Delete(finger) => {
                if state.enrolling.is_some() || state.deleting {
                    return Task::none();
                }
                state.deleting = true;
                state.error = None;
                state.added = None;
                if demo {
                    if let Some(Ok(overview)) = state.overview.as_mut() {
                        overview
                            .enrolled
                            .retain(|f| finger.as_ref().is_some_and(|d| d != f));
                    }
                    return Task::done(wrap(FingerprintMessage::Deleted(Ok(()))));
                }
                Task::perform(
                    async move { enzui_shell::fingerprint::delete(finger.as_deref()).await },
                    |r| wrap(FingerprintMessage::Deleted(r)),
                )
            }
            FingerprintMessage::Deleted(result) => {
                state.deleting = false;
                state.error = result.err();
                if demo {
                    Task::none()
                } else {
                    self.fingerprint_read(true)
                }
            }
        }
    }

    /// The Users page's fingerprint card.
    pub(in crate::settings_app) fn fingerprint_card<'a>(&'a self) -> Option<Element<'a, Parent>> {
        let route = "users/fingerprint";
        let state = &self.system.fingerprint;
        let overview = match &state.overview {
            None => {
                return Some(
                    self.section_card_for(
                        route,
                        column![self.mark_setting(route, self.loading_row())],
                    )
                    .title(tr("shell-fingerprint"))
                    .into_element(),
                );
            }
            Some(Ok(Overview { reader: None, .. })) => {
                return Some(
                    self.section_card_for(
                        route,
                        column![self.mark_setting(
                            route,
                            self.info_row(
                                "fingerprint-pattern",
                                tr("shell-fingerprint-no-reader"),
                                tr("shell-fingerprint-no-reader-detail")
                            )
                        )],
                    )
                    .title(tr("shell-fingerprint"))
                    .into_element(),
                );
            }
            Some(Err(error)) => {
                return Some(
                    self.section_card_for(
                        route,
                        column![self.mark_setting(
                            route,
                            self.info_row("fingerprint-pattern", tr("shell-fingerprint"), error)
                        )],
                    )
                    .title(tr("shell-fingerprint"))
                    .into_element(),
                );
            }
            Some(Ok(overview)) => overview,
        };
        let busy = state.enrolling.is_some() || state.deleting;
        let mut rows = column![].spacing(2);
        let summary = if overview.enrolled.is_empty() {
            tr("shell-fingerprint-none").to_owned()
        } else {
            tr("shell-fingerprint-unlocks").to_owned()
        };
        rows = rows.push(self.mark_setting(
            route,
            self.info_row(
                "fingerprint-pattern",
                overview.reader.clone().unwrap_or_default(),
                summary,
            ),
        ));
        for (index, finger) in overview.enrolled.iter().enumerate() {
            rows = rows.push(
                crate::settings_app::settings_row(finger_label(finger))
                    .icon("fingerprint-pattern")
                    .control(
                        enz_button(
                            tr("shell-fingerprint-remove"),
                            wrap(FingerprintMessage::Delete(Some(finger.clone()))),
                        )
                        .kind(ButtonKind::DestructiveOutlined)
                        .size(ButtonSize::Sm)
                        .id(format!("fingerprint-remove-{index}").into())
                        .enabled(!busy)
                        .into_element(),
                    )
                    .into_element(),
            );
        }
        if let Some(enrolling) = &state.enrolling {
            let total = if enrolling.stages > 0 {
                enrolling.stages
            } else {
                5
            };
            let step = if overview.swipe {
                tr("shell-fingerprint-swipe-again")
            } else {
                tr("shell-fingerprint-touch-again")
            };
            let text = match enrolling.hint {
                Some(hint) => hint.to_owned(),
                None if enrolling.passed == 0 => {
                    if overview.swipe {
                        tr("shell-fingerprint-swipe").to_owned()
                    } else {
                        tr("shell-fingerprint-touch").to_owned()
                    }
                }
                None => step.to_owned(),
            };
            let progress: Element<'a, Parent> = enzui_widgets::progress_bar::enz_progress_bar(
                enrolling.passed.min(total) as f32 / total as f32,
            )
            .into();
            rows = rows.push(
                crate::settings_app::settings_row(tr!("shell-fingerprint-adding", "arg0" => finger_label(&enrolling.finger).to_owned()))
                    .subtitle(text)
                    .icon("fingerprint-pattern")
                    .into_element(),
            );
            rows = rows.push(
                iced::widget::container(progress)
                    .padding(iced::Padding {
                        top: 0.0,
                        right: 10.0,
                        bottom: 6.0,
                        left: 44.0,
                    })
                    .width(Length::Fill),
            );
        } else {
            let available: Vec<&str> = FINGERS
                .iter()
                .copied()
                .filter(|f| !overview.enrolled.iter().any(|e| e == f))
                .collect();
            if let Some(first) = available.first() {
                let chosen = state
                    .finger
                    .clone()
                    .filter(|f| available.contains(&f.as_str()))
                    .unwrap_or_else(|| (*first).to_owned());
                let choices: Vec<(String, Parent)> = available
                    .iter()
                    .map(|f| {
                        (
                            finger_label(f).to_owned(),
                            wrap(FingerprintMessage::Choose((*f).to_owned())),
                        )
                    })
                    .collect();
                let menu = "users/fingerprint-finger";
                let add = crate::settings_app::settings_row(tr("shell-fingerprint-add"))
                    .subtitle(tr("shell-fingerprint-add-detail"))
                    .icon("plus")
                    .value_menu(
                        finger_label(&chosen).to_owned(),
                        self.menu.as_deref() == Some(menu),
                        Parent::Menu(menu.into(), true),
                        Parent::Menu(menu.into(), false),
                        choices,
                    )
                    .into_element();
                // The finger choice and Start share one row.
                rows = rows.push(
                    row![
                        iced::widget::container(add).width(Length::Fill),
                        enz_button(
                            tr("shell-fingerprint-start"),
                            wrap(FingerprintMessage::Enroll)
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("fingerprint-enroll".into())
                        .enabled(!busy)
                        .into_element(),
                    ]
                    .spacing(4)
                    .padding(iced::Padding {
                        right: 10.0,
                        ..iced::Padding::ZERO
                    })
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        let mut card = column![].spacing(8);
        if let Some(error) = &state.error {
            card = card.push(
                enz_banner(error.clone())
                    .tone(BannerTone::Danger)
                    .into_element(),
            );
        } else if let Some(added) = &state.added {
            card = card.push(
                enz_banner(tr!("shell-fingerprint-added", "arg0" => added.clone()))
                    .tone(BannerTone::Success)
                    .into_element(),
            );
        }
        card = card.push(rows);
        Some(
            self.section_card_for(route, card)
                .title(tr("shell-fingerprint"))
                .description(tr("shell-fingerprint-description"))
                .into_element(),
        )
    }
}
