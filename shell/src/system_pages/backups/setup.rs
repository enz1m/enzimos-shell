use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
#[cfg(test)]
mod tests;
use enzui_core::form::SecretText;
use enzui_shell::system_settings::backup_setup::{self, Job, Mode, Status};
use enzui_widgets::secure_field::enz_secure_field;
use iced::widget::column;

#[derive(Default)]
pub(super) struct Setup {
    open: bool,
    epoch: u64,
    selected: Option<backup_setup::Candidate>,
    mode: Option<Mode>,
    job: Option<Job>,
    status: Option<Status>,
    password: Option<SecretText>,
    repeat: Option<SecretText>,
    revealed: bool,
    repeat_revealed: bool,
    busy: bool,
    error: Option<String>,
    next_poll: Option<Instant>,
    // Hide ineligible drives unless explicitly requested.
    show_unusable: bool,
}
#[derive(Clone, Debug)]
pub(in crate::settings_app) enum SetupMessage {
    Open,
    Select(String),
    Mode(Mode),
    Review,
    NewReview,
    Reviewed(u64, Result<(Job, Result<Status, String>), String>),
    Password(SecretText),
    Repeat(SecretText),
    Reveal,
    RevealRepeat,
    Enroll,
    Check,
    Cancel,
    Updated(u64, String, Result<Status, String>),
    Close,
    ShowUnusable(bool),
}
fn send(message: SetupMessage) -> Parent {
    super::send(BackupsMessage::Setup(message))
}
impl Setup {
    pub(super) fn read_blocked(&self, force: bool) -> bool {
        self.busy
            || (self.open && self.status.as_ref().is_some_and(Status::waiting))
            || (!force && self.open)
    }
    pub(super) fn job_present(&self) -> bool {
        self.job.is_some()
    }
    pub(super) fn shown(&self, configured: bool) -> bool {
        self.job.is_some() || self.open || !configured
    }
    fn clear_secrets(&mut self) {
        self.password = None;
        self.repeat = None;
        self.revealed = false;
        self.repeat_revealed = false;
    }
    fn input_ready(&self) -> bool {
        !self.busy
            && self.error.is_none()
            && self.status.as_ref().is_some_and(Status::accepts_input)
            && self.password.as_ref().is_some_and(|password| {
                backup_setup::validate_password(password).is_ok()
                    && (self.mode == Some(Mode::Adopt)
                        || self.repeat.as_ref().is_some_and(|repeat| {
                            repeat.expose_secret() == password.expose_secret()
                        }))
            })
    }
    fn settled(&mut self, result: Result<Status, String>) -> bool {
        self.busy = false;
        self.next_poll = None;
        match result {
            Ok(status) => {
                if let Some(job) = &mut self.job {
                    job.observe(&status);
                }
                self.error = None;
                if !status.accepts_input() {
                    self.clear_secrets();
                }
                self.next_poll = status
                    .waiting()
                    .then(|| Instant::now() + Duration::from_secs(2));
                let completed = status.state == "completed";
                self.status = Some(status);
                completed
            }
            Err(error) => {
                self.clear_secrets();
                self.error = Some(error);
                false
            }
        }
    }
}
impl SettingsApp {
    pub(in crate::settings_app) fn backup_setup_leave(&mut self) {
        self.system.backups.share.leave();
        let setup = &mut self.system.backups.setup;
        setup.open = false;
        setup.clear_secrets();
        setup.next_poll = None;
    }
    /// Whether an open backup setup is waiting for its next status check.
    pub(in crate::settings_app::system_pages) fn backup_setup_waiting(&self) -> bool {
        let setup = &self.system.backups.setup;
        self.page == Category::Backups && setup.open && setup.next_poll.is_some()
    }
    pub(in crate::settings_app::system_pages) fn backup_setup_tick(&mut self) -> Task<Parent> {
        let setup = &self.system.backups.setup;
        if self.page == Category::Backups
            && setup.open
            && !setup.busy
            && setup.next_poll.is_some_and(|at| at <= Instant::now())
        {
            return self.backup_setup_message(SetupMessage::Check);
        }
        Task::none()
    }
    pub(super) fn backup_setup_message(&mut self, message: SetupMessage) -> Task<Parent> {
        let safety = self.safety_now();
        let backend = self.system.backend.clone();
        let writable = self.system_writes() && !backend.demo;
        let setup = &mut self.system.backups.setup;
        match message {
            SetupMessage::Open => {
                if !safety
                    .as_ref()
                    .is_some_and(|s| s.setup_available && !s.local_configured)
                    && setup.job.is_none()
                {
                    return Task::none();
                }
                setup.open = true;
                setup.clear_secrets();
                if setup.job.is_some() {
                    return self.backup_setup_message(SetupMessage::Check);
                }
            }
            SetupMessage::Select(token) if setup.job.is_none() && !setup.busy => {
                setup.selected = safety
                    .as_ref()
                    .and_then(|s| s.candidates.as_ref())
                    .and_then(|rows| {
                        rows.iter()
                            .find(|row| row.token.as_deref() == Some(token.as_str()))
                    })
                    .cloned();
                setup.error = None;
            }
            SetupMessage::Mode(mode) if setup.job.is_none() && !setup.busy => {
                setup.mode = Some(mode);
                setup.error = None;
            }
            SetupMessage::NewReview
                if !setup.busy && setup.status.as_ref().is_some_and(Status::restartable) =>
            {
                let epoch = setup.epoch.wrapping_add(1);
                *setup = Setup {
                    open: true,
                    epoch,
                    ..Setup::default()
                };
                return self.read_system(true);
            }
            SetupMessage::Review
                if writable && setup.open && setup.job.is_none() && !setup.busy =>
            {
                let Some(token) = setup.selected.as_ref().and_then(|row| row.token.clone()) else {
                    return Task::none();
                };
                if !safety
                    .as_ref()
                    .and_then(|s| s.candidates.as_ref())
                    .is_some_and(|rows| rows.iter().any(|row| row.token.as_ref() == Some(&token)))
                {
                    setup.error = Some(
                        tr("shell-the-drive-list-changed-select-a-current-drive-before-reviewing")
                            .into(),
                    );
                    return Task::none();
                }
                let Some(mode) = setup.mode else {
                    return Task::none();
                };
                setup.busy = true;
                setup.epoch = setup.epoch.wrapping_add(1);
                setup.error = None;
                let epoch = setup.epoch;
                return Task::perform(
                    async move {
                        let client = backend.backup_setup_client().await?;
                        let job = Job::new(client, mode);
                        let result = job.plan(&token).await;
                        Ok((job, result))
                    },
                    move |result| send(SetupMessage::Reviewed(epoch, result)),
                );
            }
            SetupMessage::Reviewed(epoch, result) if epoch == setup.epoch && setup.busy => {
                match result {
                    Ok((job, status)) => {
                        setup.job = Some(job);
                        setup.settled(status);
                    }
                    Err(error) => {
                        setup.busy = false;
                        setup.error = Some(error);
                    }
                }
                if !setup.open {
                    setup.next_poll = None;
                }
            }
            SetupMessage::Password(secret)
                if setup.open
                    && !setup.busy
                    && setup.error.is_none()
                    && setup.status.as_ref().is_some_and(Status::accepts_input) =>
            {
                setup.password = Some(secret);
                setup.error = None;
            }
            SetupMessage::Repeat(secret)
                if setup.open
                    && !setup.busy
                    && setup.error.is_none()
                    && setup.status.as_ref().is_some_and(Status::accepts_input) =>
            {
                setup.repeat = Some(secret);
                setup.error = None;
            }
            SetupMessage::Reveal if setup.open && !setup.busy => setup.revealed = !setup.revealed,
            SetupMessage::RevealRepeat if setup.open && !setup.busy => {
                setup.repeat_revealed = !setup.repeat_revealed;
            }
            SetupMessage::Enroll if writable && setup.open && setup.input_ready() => {
                let Some(job) = setup.job.clone() else {
                    return Task::none();
                };
                let Some(password) = setup.password.take() else {
                    return Task::none();
                };
                setup.clear_secrets();
                setup.busy = true;
                setup.error = None;
                setup.next_poll = None;
                let (epoch, id) = (setup.epoch, job.id.clone());
                return Task::perform(async move { job.input(password).await }, move |result| {
                    send(SetupMessage::Updated(epoch, id, result))
                });
            }
            SetupMessage::Check if !setup.busy => {
                let Some(job) = setup.job.clone() else {
                    return Task::none();
                };
                setup.busy = true;
                setup.next_poll = None;
                let (epoch, id) = (setup.epoch, job.id.clone());
                return Task::perform(async move { job.status().await }, move |result| {
                    send(SetupMessage::Updated(epoch, id, result))
                });
            }
            SetupMessage::Cancel
                if writable
                    && !setup.busy
                    && setup.status.as_ref().is_some_and(|s| s.cancellable) =>
            {
                let (Some(job), Some(status)) = (setup.job.clone(), setup.status.as_ref()) else {
                    return Task::none();
                };
                let digest = status.digest.clone();
                setup.clear_secrets();
                setup.busy = true;
                setup.next_poll = None;
                let (epoch, id) = (setup.epoch, job.id.clone());
                return Task::perform(async move { job.cancel(&digest).await }, move |result| {
                    send(SetupMessage::Updated(epoch, id, result))
                });
            }
            SetupMessage::Updated(epoch, id, result)
                if epoch == setup.epoch
                    && setup.job.as_ref().is_some_and(|job| job.id == id)
                    && setup.busy =>
            {
                if setup.settled(result) {
                    return self.read_system(true);
                }
                if !setup.open {
                    setup.next_poll = None;
                }
            }
            SetupMessage::ShowUnusable(show) => setup.show_unusable = show,
            SetupMessage::Close => {
                setup.open = false;
                setup.clear_secrets();
                setup.next_poll = None;
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn backup_setup_view<'a>(&'a self, safety: &Safety) -> Element<'a, Parent> {
        let setup = &self.system.backups.setup;
        let writable = self.system_writes() && !self.system.backend.demo;
        let button = |label: String, message, enabled, id: String| {
            enz_button(label, send(message))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(id.into())
                .enabled(enabled)
                .into_element()
        };
        if !setup.open {
            return button(
                if setup.job.is_some() {
                    tr("shell-view-enrollment-status")
                } else {
                    tr("shell-set-up-a-backup-drive")
                }
                .into(),
                SetupMessage::Open,
                writable
                    && (setup.job.is_some() || safety.setup_available && !safety.local_configured),
                "backup-setup-open".into(),
            );
        }
        let mut body = column![self.caption_row(tr(
            "shell-use-an-existing-independent-btrfs-drive-this-does-not-format-it-delete-files"
        ))]
        .spacing(8);
        if let Some(selected) = &setup.selected {
            body = body.push(self.info_row(
                "hard-drive",
                selected.model.clone(),
                selected.label.clone(),
            ));
        }
        if setup.job.is_none() {
            match &safety.candidates {
                Some(rows) if rows.is_empty() => body = body.push(self.caption_row(tr("shell-no-eligible-backup-drives-were-found-connect-an-independent-unmounted-btrfs"))),
                Some(rows) => {
                    let row_for = |index: usize, candidate: &backup_setup::Candidate| -> Element<'a, Parent> {
                        let label = if candidate.label.is_empty() { candidate.model.clone() } else { format!("{} · {}", candidate.model, candidate.label) };
                        let label = if label.trim().is_empty() { tr("shell-unnamed-drive").to_owned() } else { label };
                        let size = candidate.bytes.map_or_else(|| tr("shell-size-unknown").into(), |bytes| tr!("shell-gib-2", "arg0" => format!("{:}", bytes / (1 << 30))));
                        let reason = match candidate.reason.as_str() {
                            "unsupported-device-type" => tr("shell-this-device-type-is-not-supported"),
                            "existing-btrfs-required" => tr("shell-an-existing-btrfs-filesystem-is-required"),
                            "duplicate-filesystem-uuid" => tr("shell-the-filesystem-identity-is-ambiguous"),
                            "already-mounted" => tr("shell-in-use-unmount-or-eject-it-before-reviewing"),
                            "read-only" => tr("shell-this-drive-is-read-only"),
                            "identity-or-independence-unavailable" => tr("shell-drive-identity-or-separation-from-the-system-cannot-be-verified"),
                            _ if candidate.token.is_some() => tr("shell-filesystem-needs-read-only-review"),
                            _ => tr("shell-unavailable-for-enrollment"),
                        };
                        let row = crate::settings_app::settings_row(label).id_prefix(format!("backup-drive-{index}"))
                            .subtitle(format!("{size} · {reason}"));
                        if let Some(token) = &candidate.token {
                            row.control(button(if setup.selected.as_ref().and_then(|s| s.token.as_ref()) == Some(token) { tr("shell-selected") } else { tr("shell-choose") }.into(), SetupMessage::Select(token.clone()), !setup.busy, format!("backup-drive-choose-{index}"))).into_element()
                        } else { row.into_element() }
                    };
                    let usable: Vec<_> = rows.iter().enumerate().filter(|(_, candidate)| candidate.token.is_some()).collect();
                    let other: Vec<_> = rows.iter().enumerate().filter(|(_, candidate)| candidate.token.is_none()).collect();
                    if usable.is_empty() {
                        body = body.push(self.caption_row(tr("shell-no-drive-can-be-enrolled-right-now-connect-an-independent-btrfs-drive-a-driv")));
                    }
                    for (index, candidate) in usable { body = body.push(row_for(index, candidate)); }
                    if !other.is_empty() {
                        let shown = setup.show_unusable;
                        body = body.push(crate::settings_app::settings_row(match other.len() { 1 => tr("shell-1-other-drive-can-t-be-used").to_owned(), count => tr!("shell-other-drives-can-t-be-used", "arg0" => format!("{:}", count)) })
                            .subtitle(tr("shell-system-disks-drives-in-use-and-unsupported-devices"))
                            .control(button(if shown { tr("shell-hide") } else { tr("shell-show") }.into(), SetupMessage::ShowUnusable(!shown), true, "backup-drive-others".into()))
                            .into_element());
                        if shown { for (index, candidate) in other { body = body.push(row_for(index, candidate)); } }
                    }
                }
                None => body = body.push(self.caption_row(tr("shell-the-drive-list-is-unavailable-refresh-settings-to-check-it-again"))),
            }
            let chip = |label: &'static str, mode: Mode, id: &'static str| {
                enzui_widgets::button::enz_filter_chip(
                    label,
                    setup.mode == Some(mode),
                    send(SetupMessage::Mode(mode)),
                )
                .id(id.into())
                .enabled(!setup.busy)
                .into_element()
            };
            body = body.push(
                iced::widget::row![
                    chip(
                        tr("shell-create-a-new-repository"),
                        Mode::Initialize,
                        "backup-mode-new"
                    ),
                    chip(
                        tr("shell-use-an-existing-repository"),
                        Mode::Adopt,
                        "backup-mode-existing"
                    ),
                ]
                .spacing(8)
                .wrap(),
            );
            if let Some(mode) = setup.mode {
                body = body.push(self.caption_row(match mode { Mode::Initialize => tr("shell-chosen-create-a-new-repository-the-read-only-review-refuses-a-repository-wit"), Mode::Adopt => tr("shell-chosen-use-an-existing-repository-its-password-must-authenticate-before-cred") }));
            }
            body = body.push(button(
                tr("shell-review-drive").into(),
                SetupMessage::Review,
                writable && !setup.busy && setup.selected.is_some() && setup.mode.is_some(),
                "backup-setup-review".into(),
            ));
        }
        if let Some(status) = &setup.status {
            body = body.push(self.caption_row(match setup.mode {
                Some(Mode::Initialize) => {
                    tr("shell-reviewed-operation-create-a-new-repository-on-the-selected-drive")
                }
                Some(Mode::Adopt) => tr(
                    "shell-reviewed-operation-enroll-the-existing-repository-on-the-selected-drive",
                ),
                None => tr("shell-reviewed-operation-is-unavailable"),
            }));
            body = body.push(self.caption_row(status.label()));
            if status.accepts_input() && setup.error.is_none() {
                // Reserve field-caption height so errors do not move action buttons.
                let entered = setup
                    .password
                    .as_ref()
                    .filter(|password| !password.expose_secret().is_empty());
                let invalid = entered
                    .is_some_and(|password| backup_setup::validate_password(password).is_err());
                let mismatch = !invalid
                    && setup.mode == Some(Mode::Initialize)
                    && entered.is_some_and(|password| {
                        setup.repeat.as_ref().is_some_and(|repeat| {
                            !repeat.expose_secret().is_empty()
                                && repeat.expose_secret() != password.expose_secret()
                        })
                    });
                let password = setup
                    .password
                    .as_ref()
                    .map_or("", SecretText::expose_secret);
                let field =
                    enz_secure_field(
                        password,
                        |text| send(SetupMessage::Password(SecretText::new(text))),
                        setup.revealed,
                        send(SetupMessage::Reveal),
                    )
                    .label(if setup.mode == Some(Mode::Initialize) {
                        enzui_shell::i18n::current().text("shell-new-repository-password")
                    } else {
                        enzui_shell::i18n::current().text("shell-existing-repository-password")
                    })
                    .description(enzui_shell::i18n::current().text(
                        "shell-one-line-up-to-4096-bytes-you-need-it-to-restore-from-this-drive",
                    ));
                body = body.push(
                    if invalid {
                        field.error(
                            enzui_shell::i18n::current()
                                .text("shell-enter-a-single-line-password-of-1-4096-utf-8-bytes"),
                        )
                    } else {
                        field
                    }
                    .id("backup-setup-password".into())
                    .enabled(!setup.busy)
                    .into_element(),
                );
                if setup.mode == Some(Mode::Initialize) {
                    let repeat = setup.repeat.as_ref().map_or("", SecretText::expose_secret);
                    let field = enz_secure_field(
                        repeat,
                        |text| send(SetupMessage::Repeat(SecretText::new(text))),
                        setup.repeat_revealed,
                        send(SetupMessage::RevealRepeat),
                    )
                    .label(enzui_shell::i18n::current().text("shell-repeat-the-new-password"))
                    .description(
                        enzui_shell::i18n::current().text("shell-type-the-same-password-again"),
                    );
                    body = body.push(
                        if mismatch {
                            field.error(
                                enzui_shell::i18n::current()
                                    .text("shell-the-passwords-don-t-match"),
                            )
                        } else {
                            field
                        }
                        .id("backup-setup-repeat".into())
                        .enabled(!setup.busy)
                        .into_element(),
                    );
                }
                body = body.push(self.caption_row(tr("shell-confirm-enrollment-for-this-reviewed-drive-a-separate-administrator-approval")));
                body = body.push(button(
                    tr("shell-enroll-repository").into(),
                    SetupMessage::Enroll,
                    writable && setup.input_ready(),
                    "backup-setup-enroll".into(),
                ));
            }
            if status.restartable() {
                body = body.push(button(
                    tr("shell-start-a-new-drive-review").into(),
                    SetupMessage::NewReview,
                    !setup.busy,
                    "backup-setup-new-review".into(),
                ));
            }
            if status.cancellable && !status.settled() {
                body = body.push(button(
                    tr("shell-cancel-before-entry").into(),
                    SetupMessage::Cancel,
                    writable && !setup.busy,
                    "backup-setup-cancel".into(),
                ));
            }
        }
        if setup.busy {
            body = body.push(self.caption_row(tr(
                "shell-waiting-for-the-current-request-no-mutation-will-be-retried-automatically",
            )));
        }
        if let Some(error) = &setup.error {
            body = body.push(self.caption_row(error.clone()));
        }
        if setup.job.is_some() && !setup.status.as_ref().is_some_and(Status::settled) {
            body = body.push(button(
                tr("shell-check-status").into(),
                SetupMessage::Check,
                !setup.busy,
                "backup-setup-check".into(),
            ));
        }
        body.push(button(
            tr("shell-hide-setup").into(),
            SetupMessage::Close,
            true,
            "backup-setup-hide".into(),
        ))
        .into()
    }
}
