use super::*;
use enzui_shell::firmware::{Installed, Overview};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::column;

/// Device firmware (fwupd) shown on the Updates page.
#[derive(Default)]
pub(super) struct Firmware {
    overview: Option<Result<Overview, String>>,
    loading: bool,
    epoch: u64,
    checking: bool,
    check_error: Option<String>,
    /// The device being updated.
    installing: Option<String>,
    /// The last installation: device name and outcome.
    outcome: Option<(String, Result<Installed, String>)>,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum FirmwareMessage {
    Read(u64, Result<Overview, String>),
    Check,
    Checked(Result<(), String>),
    Install(String),
    Installed(String, Result<Installed, String>),
}

fn wrap(message: FirmwareMessage) -> Parent {
    Parent::System(Message::Firmware(message))
}

fn checked_text(checked: Option<u64>) -> String {
    let Some(seconds) = checked.and_then(|s| i64::try_from(s).ok()) else {
        return tr("shell-firmware-never-checked").to_owned();
    };
    let Some(when) = chrono::DateTime::from_timestamp(seconds, 0) else {
        return tr("shell-firmware-never-checked").to_owned();
    };
    let date = when.with_timezone(&chrono::Local).date_naive();
    tr!("shell-firmware-last-checked", "arg0" => enzui_shell::i18n::date(date, false))
}

impl SettingsApp {
    pub(super) fn firmware_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.firmware;
        if self.system.backend.demo {
            if state.overview.is_none() {
                state.overview = Some(Ok(enzui_shell::firmware::demo()));
            }
            return Task::none();
        }
        if state.loading
            || state.checking
            || state.installing.is_some()
            || !(force || state.overview.is_none())
        {
            return Task::none();
        }
        let Some(epoch) = state.epoch.checked_add(1) else {
            return Task::none();
        };
        state.epoch = epoch;
        state.loading = true;
        Task::perform(enzui_shell::firmware::read(), move |result| {
            wrap(FirmwareMessage::Read(epoch, result))
        })
    }

    pub(super) fn firmware_message(&mut self, message: FirmwareMessage) -> Task<Parent> {
        let demo = self.system.backend.demo;
        // A read-only preview shows this computer's hardware without changing it.
        if !self.system.backend.writes_allowed && matches!(message, FirmwareMessage::Install(_)) {
            self.error = Some(tr("shell-system-changes-are-off-in-this-preview").into());
            return Task::none();
        }
        let state = &mut self.system.firmware;
        match message {
            FirmwareMessage::Read(epoch, result) => {
                if epoch == state.epoch {
                    state.loading = false;
                    state.overview = Some(result);
                }
                Task::none()
            }
            FirmwareMessage::Check => {
                if state.checking || state.installing.is_some() || self.closing {
                    return Task::none();
                }
                state.checking = true;
                state.check_error = None;
                if demo {
                    return Task::done(wrap(FirmwareMessage::Checked(Ok(()))));
                }
                Task::perform(enzui_shell::firmware::check(), |result| {
                    wrap(FirmwareMessage::Checked(result))
                })
            }
            FirmwareMessage::Checked(result) => {
                state.checking = false;
                state.check_error = result.err();
                self.firmware_read(true)
            }
            FirmwareMessage::Install(id) => {
                if state.checking || state.installing.is_some() || self.closing {
                    return Task::none();
                }
                state.installing = Some(id.clone());
                state.outcome = None;
                if demo {
                    return Task::done(wrap(FirmwareMessage::Installed(
                        id,
                        Ok(Installed::RestartNeeded),
                    )));
                }
                let target = id.clone();
                Task::perform(
                    async move { enzui_shell::firmware::install(&target).await },
                    move |result| wrap(FirmwareMessage::Installed(id, result)),
                )
            }
            FirmwareMessage::Installed(id, result) => {
                state.installing = None;
                let name = state
                    .overview
                    .as_ref()
                    .and_then(|o| o.as_ref().ok())
                    .and_then(|o| o.devices.iter().find(|d| d.id == id))
                    .map(|d| d.name.clone())
                    .unwrap_or_default();
                if demo
                    && result.is_ok()
                    && let Some(Ok(overview)) = &mut state.overview
                    && let Some(device) = overview.devices.iter_mut().find(|d| d.id == id)
                {
                    if let Some(update) = device.update.take() {
                        device.version = update.version;
                    }
                    device.pending_restart = true;
                }
                state.outcome = Some((name, result));
                if demo {
                    Task::none()
                } else {
                    self.firmware_read(true)
                }
            }
        }
    }

    pub(super) fn firmware_card<'a>(&'a self) -> Element<'a, Parent> {
        let route = "updates/firmware";
        let state = &self.system.firmware;
        let busy = state.checking || state.installing.is_some();
        let mut rows = column![].spacing(2);
        match &state.overview {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(Err(error)) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row("cpu", tr("shell-firmware-service-did-not-answer"), error),
                ));
            }
            Some(Ok(overview)) if !overview.available => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "cpu",
                        tr("shell-firmware-updates-are-not-available"),
                        tr("shell-install-fwupd-to-update-device-firmware"),
                    ),
                ));
            }
            Some(Ok(overview)) => {
                let subtitle = if state.checking {
                    tr("shell-firmware-checking").to_owned()
                } else if let Some(error) = &state.check_error {
                    error.clone()
                } else {
                    checked_text(overview.checked)
                };
                let check =
                    enz_button(tr("shell-firmware-check-now"), wrap(FirmwareMessage::Check))
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("firmware-check".into())
                        .enabled(!busy)
                        .into_element();
                let summary = match overview.updates() {
                    0 => tr("shell-firmware-all-up-to-date").to_owned(),
                    count => tr!("shell-firmware-updates-available", "arg0" => format!("{count}")),
                };
                rows = rows.push(
                    self.mark_setting(
                        route,
                        crate::settings_app::settings_row(summary)
                            .subtitle(subtitle)
                            .icon("refresh-cw")
                            .control(check)
                            .into_element(),
                    ),
                );
                for (index, device) in overview
                    .devices
                    .iter()
                    .filter(|d| d.updatable || d.update.is_some())
                    .enumerate()
                {
                    let installing = state.installing.as_deref() == Some(device.id.as_str());
                    let mut subtitle = if device.vendor.is_empty() {
                        tr!("shell-firmware-version", "arg0" => device.version.clone())
                    } else {
                        tr!("shell-firmware-vendor-version", "arg0" => device.vendor.clone(), "arg1" => device.version.clone())
                    };
                    if installing {
                        tr("shell-firmware-installing-keep-connected").clone_into(&mut subtitle);
                    } else if device.pending_restart {
                        tr("shell-firmware-restart-to-finish").clone_into(&mut subtitle);
                    } else if let Some(update) = &device.update {
                        subtitle = if update.size > 0 {
                            tr!("shell-firmware-update-to", "arg0" => update.version.clone(), "arg1" => developer::size_text(update.size))
                        } else {
                            tr!("shell-firmware-update-to-unsized", "arg0" => update.version.clone())
                        };
                        if device.needs_power {
                            subtitle =
                                format!("{subtitle} · {}", tr("shell-firmware-needs-charger"));
                        }
                    }
                    let control: Element<'a, Parent> = match &device.update {
                        Some(_) if !device.pending_restart => enz_button(
                            if installing {
                                tr("shell-firmware-installing")
                            } else {
                                tr("shell-firmware-install")
                            },
                            wrap(FirmwareMessage::Install(device.id.clone())),
                        )
                        .kind(ButtonKind::Primary)
                        .size(ButtonSize::Sm)
                        .id(format!("firmware-install-{index}").into())
                        .enabled(!busy)
                        .into_element(),
                        _ if device.pending_restart => {
                            self.status_badge(tr("shell-firmware-restart"), BadgeTone::Warning)
                        }
                        _ => self.status_badge(tr("shell-firmware-current"), BadgeTone::Default),
                    };
                    if let Some(update) = device
                        .update
                        .as_ref()
                        .filter(|u| !u.notes.is_empty() && !installing && !device.pending_restart)
                    {
                        subtitle = format!("{subtitle}\n{}", update.notes);
                    }
                    let row = crate::settings_app::settings_row(device.name.clone())
                        .subtitle(subtitle)
                        .subtitle_wrap(true)
                        .icon(if device.internal { "cpu" } else { "usb" })
                        .control(control);
                    rows = rows.push(row.into_element());
                }
                if overview
                    .devices
                    .iter()
                    .all(|d| !d.updatable && d.update.is_none())
                {
                    rows = rows.push(self.info_row(
                        "cpu",
                        tr("shell-firmware-no-updatable-devices"),
                        tr("shell-firmware-no-updatable-devices-detail"),
                    ));
                }
            }
        }
        let mut card = column![].spacing(8);
        if let Some((name, outcome)) = &state.outcome {
            let (tone, text) = match outcome {
                Ok(Installed::Done) => (
                    BannerTone::Success,
                    tr!("shell-firmware-updated", "arg0" => name.clone()),
                ),
                Ok(Installed::RestartNeeded) => (
                    BannerTone::Info,
                    tr!("shell-firmware-updated-restart", "arg0" => name.clone()),
                ),
                Err(error) => (BannerTone::Danger, error.clone()),
            };
            card = card.push(enz_banner(text).tone(tone).into_element());
        }
        card = card.push(rows);
        self.section_card_for(route, card)
            .title(tr("shell-device-firmware"))
            .description(tr("shell-device-firmware-description"))
            .into_element()
    }
}
