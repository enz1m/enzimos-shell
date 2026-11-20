use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::thunderbolt::{Overview, Security, Status};
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::column;

#[derive(Default)]
pub(super) struct Thunderbolt {
    overview: Option<Overview>,
    loading: bool,
    epoch: u64,
    /// The device being allowed or forgotten.
    changing: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum ThunderboltMessage {
    Read(u64, Overview),
    Allow(String),
    Forget(String),
    Changed(Result<(), String>),
}

fn wrap(message: ThunderboltMessage) -> Parent {
    Parent::System(Message::Thunderbolt(message))
}

impl SettingsApp {
    pub(super) fn thunderbolt_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.thunderbolt;
        if self.system.backend.demo {
            if state.overview.is_none() {
                state.overview = Some(enzui_shell::thunderbolt::demo());
            }
            return Task::none();
        }
        if state.loading || state.changing.is_some() || !(force || state.overview.is_none()) {
            return Task::none();
        }
        let Some(epoch) = state.epoch.checked_add(1) else {
            return Task::none();
        };
        state.epoch = epoch;
        state.loading = true;
        Task::perform(enzui_shell::thunderbolt::read(), move |overview| {
            wrap(ThunderboltMessage::Read(epoch, overview))
        })
    }

    pub(super) fn thunderbolt_message(&mut self, message: ThunderboltMessage) -> Task<Parent> {
        let demo = self.system.backend.demo;
        // A read-only preview shows this computer's hardware without changing it.
        if !self.system.backend.writes_allowed
            && matches!(
                message,
                ThunderboltMessage::Allow(_) | ThunderboltMessage::Forget(_)
            )
        {
            self.error = Some(tr("shell-system-changes-are-off-in-this-preview").into());
            return Task::none();
        }
        let state = &mut self.system.thunderbolt;
        match message {
            ThunderboltMessage::Read(epoch, overview) => {
                if epoch == state.epoch {
                    state.loading = false;
                    state.overview = Some(overview);
                }
                Task::none()
            }
            ThunderboltMessage::Allow(uid) | ThunderboltMessage::Forget(uid)
                if state.changing.is_some() || uid.is_empty() =>
            {
                let _ = uid;
                Task::none()
            }
            ThunderboltMessage::Allow(uid) => {
                state.changing = Some(uid.clone());
                state.error = None;
                if demo {
                    if let Some(device) = state
                        .overview
                        .as_mut()
                        .and_then(|o| o.devices.iter_mut().find(|d| d.uid == uid))
                    {
                        device.status = Status::Allowed;
                        device.stored = true;
                    }
                    return Task::done(wrap(ThunderboltMessage::Changed(Ok(()))));
                }
                Task::perform(
                    async move { enzui_shell::thunderbolt::allow(&uid).await },
                    |r| wrap(ThunderboltMessage::Changed(r)),
                )
            }
            ThunderboltMessage::Forget(uid) => {
                state.changing = Some(uid.clone());
                state.error = None;
                if demo {
                    if let Some(overview) = state.overview.as_mut() {
                        overview
                            .devices
                            .retain(|d| d.uid != uid || d.status != Status::Disconnected);
                        if let Some(device) = overview.devices.iter_mut().find(|d| d.uid == uid) {
                            device.stored = false;
                        }
                    }
                    return Task::done(wrap(ThunderboltMessage::Changed(Ok(()))));
                }
                Task::perform(
                    async move { enzui_shell::thunderbolt::forget(&uid).await },
                    |r| wrap(ThunderboltMessage::Changed(r)),
                )
            }
            ThunderboltMessage::Changed(result) => {
                state.changing = None;
                state.error = result.err();
                if demo {
                    Task::none()
                } else {
                    self.thunderbolt_read(true)
                }
            }
        }
    }

    pub(super) fn thunderbolt_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "thunderbolt/devices";
        let state = &self.system.thunderbolt;
        let mut rows = column![].spacing(2);
        match &state.overview {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(overview) if !overview.available => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "zap",
                        tr("shell-thunderbolt-service-unavailable"),
                        tr("shell-thunderbolt-install-bolt"),
                    ),
                ));
            }
            Some(overview) => {
                let (title, detail) = match overview.security {
                    Security::User | Security::Secure => (
                        tr("shell-thunderbolt-approval-needed"),
                        tr("shell-thunderbolt-approval-needed-detail"),
                    ),
                    Security::None => (
                        tr("shell-thunderbolt-no-approval"),
                        tr("shell-thunderbolt-no-approval-detail"),
                    ),
                    Security::DisplayOnly | Security::UsbOnly => (
                        tr("shell-thunderbolt-limited"),
                        tr("shell-thunderbolt-limited-detail"),
                    ),
                    Security::Unknown => (tr("shell-thunderbolt"), tr("shell-thunderbolt-summary")),
                };
                rows = rows.push(self.mark_setting(route, self.info_row("shield", title, detail)));
                if overview.devices.is_empty() {
                    rows = rows.push(self.info_row(
                        "zap",
                        tr("shell-thunderbolt-no-devices"),
                        tr("shell-thunderbolt-no-devices-detail"),
                    ));
                }
                for (index, device) in overview.devices.iter().enumerate() {
                    let busy = state.changing.as_deref() == Some(device.uid.as_str());
                    let status = match device.status {
                        Status::Waiting => tr("shell-thunderbolt-waiting"),
                        Status::Authorizing => tr("shell-thunderbolt-authorizing"),
                        Status::Allowed if device.stored => {
                            tr("shell-thunderbolt-allowed-remembered")
                        }
                        Status::Allowed => tr("shell-thunderbolt-allowed"),
                        Status::Disconnected => tr("shell-thunderbolt-remembered-not-connected"),
                        Status::Failed => tr("shell-thunderbolt-failed"),
                    };
                    let subtitle = if device.vendor.is_empty() {
                        status.to_owned()
                    } else {
                        format!("{} · {status}", device.vendor)
                    };
                    let control: Element<'a, Parent> =
                        if device.status == Status::Waiting || device.status == Status::Failed {
                            enz_button(
                                tr("shell-thunderbolt-allow"),
                                wrap(ThunderboltMessage::Allow(device.uid.clone())),
                            )
                            .kind(ButtonKind::Primary)
                            .size(ButtonSize::Sm)
                            .id(format!("thunderbolt-allow-{index}").into())
                            .enabled(!busy && state.changing.is_none())
                            .into_element()
                        } else if device.stored {
                            enz_button(
                                tr("shell-thunderbolt-forget"),
                                wrap(ThunderboltMessage::Forget(device.uid.clone())),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id(format!("thunderbolt-forget-{index}").into())
                            .enabled(!busy && state.changing.is_none())
                            .into_element()
                        } else {
                            self.status_badge(tr("shell-thunderbolt-connected"), BadgeTone::Default)
                        };
                    rows = rows.push(
                        crate::settings_app::settings_row(device.name.clone())
                            .subtitle(subtitle)
                            .icon(if device.status == Status::Waiting {
                                "shield-alert"
                            } else {
                                "zap"
                            })
                            .control(control)
                            .into_element(),
                    );
                }
            }
        }
        let mut card = column![].spacing(8);
        if let Some(error) = &state.error {
            card = card.push(
                enz_banner(error.clone())
                    .tone(BannerTone::Danger)
                    .into_element(),
            );
        }
        card = card.push(rows);
        body.push(
            self.section_card_for(route, card)
                .title(tr("shell-thunderbolt-devices"))
                .description(tr("shell-thunderbolt-description"))
                .into_element(),
        )
    }
}
