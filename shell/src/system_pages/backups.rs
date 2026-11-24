#[path = "backups_view.rs"]
mod view;

use super::*;
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
mod setup;
mod share;
use enzui_shell::system_settings::backups::{
    BackupPreset, BackupRetention, Cleanup, CleanupState, Place, Point, RestorePlan, Retention,
    Snapshot, SnapshotKind, restore_id,
};
use enzui_shell::system_settings::{Backed, LongJob, enzimos::Safety};
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::text_field::enz_text_field;
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_widgets::view_switcher::{SwitcherPage, enz_view_switcher};
use iced::widget::row;

const SHORT_LIST: usize = 6;
/// Snapshots shown before "Show all": the newest few say enough.
const SHORT_SNAPSHOTS: usize = 4;

#[derive(Default)]
pub(super) struct Backups {
    system_tab: bool,
    all_snapshots: bool,
    all_points: bool,
    restore: Option<RestoreForm>,
    // A restore or copy running now: the point it works on.
    working: Option<String>,
    outcome: Option<Outcome>,
    retention: Option<RetentionForm>,
    cleanup_details: bool,
    // A snapshot or retention change waiting for its approval.
    asking: bool,
    // The outcome belongs to the snapshots card, not to restoring.
    outcome_in_history: bool,
    recovery_busy: bool,
    recovery_delivery: Option<enzui_shell::system_settings::backup_recovery::Delivered>,
    recovery_notice: Option<String>,
    setup: setup::Setup,
    share: share::Share,
    // When the status of a backup running elsewhere was last asked.
    running_checked: Option<std::time::Instant>,
}

struct RestoreForm {
    point: String,
    path: String,
    folder: bool,
    error: Option<String>,
    // The backend's single-approval plan, once planned.
    review: Option<RestorePlan>,
    id: String,
}

struct RetentionForm {
    home: Option<Retention>,
    backup: Option<BackupRetention>,
    preset: BackupPreset,
}

#[derive(Clone, Debug)]
enum Outcome {
    Restored(String, bool),
    Copied(bool),
    Snapshot(u64),
    Kept,
    Failed(String),
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum BackupsMessage {
    SystemTab(bool),
    AllSnapshots(bool),
    AllPoints(bool),
    OpenRestore(String),
    Path(String),
    Folder(bool),
    CloseRestore,
    Restore,
    Planned(Result<RestorePlan, String>),
    Restored(Result<String, String>),
    SnapshotNow,
    SnapshotTaken(Result<u64, String>),
    EditRetention(Option<bool>),
    Count(usize, u32),
    CleanupAutomatic(bool),
    CleanupPreset(BackupPreset),
    CleanupDetails(bool),
    ApplyRetention,
    RetentionApplied(Result<(), String>),
    Move(String, bool),
    Moved(bool, Result<(), String>),
    ShowHome,
    Dismiss,
    RecoveryDeliver(enzui_shell::system_settings::backup_recovery::Delivery),
    RecoveryDelivered(Result<enzui_shell::system_settings::backup_recovery::Delivered, String>),
    RecoveryConfirm,
    RecoveryConfirmed(Result<(), String>),
    Setup(setup::SetupMessage),
    Share(share::ShareMessage),
    RunningChecked(Option<serde_json::Value>),
}

/// Opens the network share set-up for a Files location.
pub(in crate::settings_app) fn share_for(id: String) -> Parent {
    send(BackupsMessage::Share(share::ShareMessage::OpenFor(id)))
}

pub(super) fn share_host(event: enzui_widgets::modal_host::ModalHostEvent) -> Parent {
    send(BackupsMessage::Share(share::ShareMessage::Host(event)))
}

fn send(message: BackupsMessage) -> Parent {
    Parent::System(Message::Backups(message))
}

pub(super) fn when(text: &str, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::TimeZone;
    let parsed = chrono::DateTime::parse_from_rfc3339(text)
        .map(|time| time.with_timezone(&chrono::Local))
        .ok()
        .or_else(|| {
            let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
                .ok()?;
            chrono::Local.from_local_datetime(&naive).earliest()
        });
    let Some(time) = parsed else {
        return text.to_owned();
    };
    let minutes = (now - time).num_minutes();
    match minutes {
        m if !(-60 * 24..60 * 48).contains(&m) => format!(
            "{}, {}",
            i18n::date(time.date_naive(), false),
            i18n::time(&time, true)
        ),
        m if m < -59 => i18n::count("relative-future-hours", -m / 60),
        m if m < 0 => i18n::count("relative-future-minutes", -m),
        0 => tr("shell-just-now").into(),
        m if m < 60 => i18n::count("relative-past-minutes", m),
        m if m < 60 * 24 => i18n::count("relative-past-hours", m / 60),
        _ => tr!("relative-yesterday", "time" => i18n::time(&time, true)),
    }
}

fn kind_label(kind: SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::Automatic => tr("shell-automatic"),
        SnapshotKind::Before => tr("shell-before-a-system-change"),
        SnapshotKind::After => tr("shell-after-a-system-change"),
        SnapshotKind::Manual => tr("shell-taken-by-hand"),
    }
}

fn place_label(place: Place) -> &'static str {
    match place {
        Place::Local => tr("shell-on-the-backup-drive"),
        Place::Remote => tr("shell-only-on-the-network-share"),
        Place::Both => tr("shell-on-the-backup-drive-and-the-network-share"),
    }
}

impl SettingsApp {
    pub(super) fn safety_now(&self) -> Option<Safety> {
        match self.reading(Panel::Safety) {
            Some(Reading::Safety(Backed::Ready(safety))) => Some(*safety),
            _ => None,
        }
    }
    pub(super) fn backup_read_blocked(&self, force: bool) -> bool {
        let state = &self.system.backups;
        state.working.is_some()
            || state.asking
            || state.recovery_busy
            || state.setup.read_blocked(force)
            || state.share.busy()
            || (!force && (state.restore.is_some() || state.retention.is_some()))
    }

    /// While a backup started elsewhere runs, only its status is asked:
    /// listing recovery points would wait on the backup's lock.
    pub(super) fn backup_running_poll(&mut self) -> Task<Parent> {
        let state = &mut self.system.backups;
        if self.system.backend.demo
            || state
                .running_checked
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(5))
        {
            return Task::none();
        }
        state.running_checked = Some(std::time::Instant::now());
        Task::perform(
            async {
                let backend = enzui_shell::system_settings::enzimos::Backend::connect().await?;
                backend.query("backups").await.ok()
            },
            |status| send(BackupsMessage::RunningChecked(status)),
        )
    }

    pub(super) fn backups_message(&mut self, message: BackupsMessage) -> Task<Parent> {
        let message = match message {
            BackupsMessage::RunningChecked(status) => {
                let Some(status) = status else {
                    return Task::none();
                };
                let running = enzui_shell::backup_activity::Running::from_status(&status);
                let ended = running.is_none();
                if let Some(Reading::Safety(Backed::Ready(safety))) =
                    self.system.readings.get_mut(&Panel::Safety)
                {
                    safety.running = running;
                }
                return if ended {
                    self.read_system(true)
                } else {
                    Task::none()
                };
            }
            BackupsMessage::Setup(message) => return self.backup_setup_message(message),
            BackupsMessage::Share(message) => return self.backup_share_message(message),
            message => message,
        };
        let safety = matches!(
            message,
            BackupsMessage::Restore
                | BackupsMessage::RecoveryDeliver(_)
                | BackupsMessage::EditRetention(Some(_))
                | BackupsMessage::ApplyRetention
        )
        .then(|| self.safety_now())
        .flatten();
        let offers = |operation: &str| safety.as_ref().is_some_and(|s| s.offers(operation));
        let backend = self.system.backend.clone();
        let state = &mut self.system.backups;
        match message {
            BackupsMessage::Setup(_)
            | BackupsMessage::Share(_)
            | BackupsMessage::RunningChecked(_) => unreachable!(),
            BackupsMessage::RecoveryDeliver(delivery) => {
                if state.recovery_busy {
                    return Task::none();
                }
                let state = &mut self.system.backups;
                state.recovery_busy = true;
                state.recovery_delivery = None;
                state.recovery_notice = None;
                return Task::perform(
                    async move {
                        backend
                            .deliver_backup_recovery(
                                delivery,
                                safety.and_then(|safety| safety.recovery_exclude_uuid),
                            )
                            .await
                    },
                    |result| send(BackupsMessage::RecoveryDelivered(result)),
                );
            }
            BackupsMessage::RecoveryDelivered(result) => {
                state.recovery_busy = false;
                match result {
                    Ok(delivered) => {
                        state.recovery_notice = Some(if delivered.printed {
                            tr("shell-print-job-submitted-check-the-paper-contains-the-complete-password-before-co")
                        } else {
                            tr("shell-recovery-password-saved-keep-that-drive-separately-from-your-computer-and-ba")
                        }.into());
                        state.recovery_delivery = Some(delivered);
                    }
                    Err(error) => state.recovery_notice = Some(error),
                }
                return self.read_system(true);
            }
            BackupsMessage::RecoveryConfirm => {
                if state.recovery_busy {
                    return Task::none();
                }
                let Some(delivered) = &state.recovery_delivery else {
                    return Task::none();
                };
                let ticket = delivered.ticket.clone();
                state.recovery_busy = true;
                return Task::perform(
                    async move { backend.confirm_backup_recovery(ticket).await },
                    |result| send(BackupsMessage::RecoveryConfirmed(result)),
                );
            }
            BackupsMessage::RecoveryConfirmed(result) => {
                state.recovery_busy = false;
                match result {
                    Ok(()) => {
                        state.recovery_delivery = None;
                        state.recovery_notice = Some(tr("shell-your-separate-copy-confirmation-is-recorded-automatic-backups-are-controlled").into());
                    }
                    Err(error) => state.recovery_notice = Some(error),
                }
                return self.read_system(true);
            }
            BackupsMessage::SystemTab(on) => state.system_tab = on,
            BackupsMessage::AllSnapshots(on) => state.all_snapshots = on,
            BackupsMessage::AllPoints(on) => state.all_points = on,
            BackupsMessage::OpenRestore(point) => {
                state.outcome = None;
                state.restore = Some(RestoreForm {
                    point,
                    path: String::new(),
                    folder: false,
                    error: None,
                    review: None,
                    id: restore_id(),
                });
                return Task::batch([
                    iced::widget::operation::focus(iced::widget::Id::new("backup-restore-path")),
                    Task::done(Parent::Reveal("backups/points")),
                ]);
            }
            BackupsMessage::Path(text) => {
                if let Some(form) = state.restore.as_mut() {
                    form.path = text.chars().take(1024).collect();
                    form.error = None;
                    form.review = None;
                }
            }
            BackupsMessage::Folder(on) => {
                if let Some(form) = state.restore.as_mut() {
                    form.folder = on;
                    form.review = None;
                }
            }
            BackupsMessage::CloseRestore => state.restore = None,
            BackupsMessage::Restore => {
                let Some(form) = state.restore.as_mut() else {
                    return Task::none();
                };
                if state.working.is_some() {
                    return Task::none();
                }
                let home = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default();
                if let Err(error) =
                    enzui_shell::system_settings::backups::source_path(&home, &form.path)
                {
                    form.error = Some(error.into());
                    return Task::none();
                }
                state.working = Some(form.point.clone());
                // One approval for the whole restore where the backend offers
                // it; folders still go through the three reviewed steps.
                let single = !form.folder && offers("own-file-restore");
                if single {
                    if let Some(plan) = form.review.take() {
                        return Task::perform(
                            async move { backend.finish_own_restore(plan).await },
                            |result| send(BackupsMessage::Restored(result)),
                        );
                    }
                    let (point, path, id) =
                        (form.point.clone(), form.path.clone(), form.id.clone());
                    return Task::perform(
                        async move { backend.plan_own_restore(&point, &path, &id).await },
                        |result| send(BackupsMessage::Planned(result)),
                    );
                }
                let (point, path, folder) = (form.point.clone(), form.path.clone(), form.folder);
                return Task::perform(
                    async move {
                        backend
                            .restore_from_backup(&point, &path, folder, &restore_id())
                            .await
                    },
                    |result| send(BackupsMessage::Restored(result)),
                );
            }
            BackupsMessage::Planned(result) => {
                state.working = None;
                if let Some(form) = state.restore.as_mut() {
                    match result {
                        Ok(plan) => form.review = Some(plan),
                        Err(error) => form.error = Some(error),
                    }
                }
            }
            BackupsMessage::Restored(result) => {
                state.working = None;
                state.outcome_in_history = false;
                match result {
                    Ok(name) => {
                        let folder = state.restore.take().is_some_and(|form| form.folder);
                        state.outcome = Some(Outcome::Restored(name, folder));
                    }
                    // The form stays, so a typo can be fixed and tried again.
                    Err(error) => state.outcome = Some(Outcome::Failed(error)),
                }
            }
            BackupsMessage::Move(point, to_share) => {
                if state.working.is_some() {
                    return Task::none();
                }
                state.outcome = None;
                state.working = Some(point.clone());
                return Task::perform(
                    async move { backend.move_point(&point, to_share).await },
                    move |result| send(BackupsMessage::Moved(to_share, result)),
                );
            }
            BackupsMessage::Moved(to_share, result) => {
                state.working = None;
                state.outcome_in_history = false;
                state.outcome = Some(match result {
                    Ok(()) => Outcome::Copied(to_share),
                    Err(error) => Outcome::Failed(error),
                });
                return self.read_system(true);
            }
            BackupsMessage::ShowHome => {
                let home = std::env::var_os("HOME").unwrap_or_default();
                return Task::perform(
                    async move {
                        let _ = tokio::process::Command::new("xdg-open")
                            .arg(home)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await;
                    },
                    |()| Parent::Noop,
                );
            }
            BackupsMessage::SnapshotNow => {
                if state.asking {
                    return Task::none();
                }
                state.asking = true;
                state.outcome = None;
                return Task::perform(async move { backend.snapshot_home().await }, |result| {
                    send(BackupsMessage::SnapshotTaken(result))
                });
            }
            BackupsMessage::SnapshotTaken(result) => {
                state.asking = false;
                state.outcome_in_history = true;
                state.outcome = Some(match result {
                    Ok(number) => Outcome::Snapshot(number),
                    Err(error) => Outcome::Failed(error),
                });
                return self.read_system(true);
            }
            BackupsMessage::EditRetention(open) => {
                state.retention = open.map(|home| RetentionForm {
                    home: safety
                        .as_ref()
                        .and_then(|s| s.home_retention)
                        .filter(|_| home),
                    backup: safety
                        .as_ref()
                        .and_then(|s| s.backup_retention)
                        .filter(|_| !home),
                    preset: safety
                        .as_ref()
                        .and_then(|s| s.backup_retention)
                        .map_or(BackupPreset::Custom, BackupRetention::preset),
                });
                state.outcome = None;
            }
            BackupsMessage::CleanupDetails(open) => state.cleanup_details = open,
            BackupsMessage::CleanupAutomatic(automatic) => {
                if !state.asking
                    && let Some(policy) = state.retention.as_mut().and_then(|f| f.backup.as_mut())
                {
                    policy.automatic = automatic;
                }
            }
            BackupsMessage::CleanupPreset(preset) => {
                if !state.asking
                    && let Some(form) = state.retention.as_mut()
                    && let Some(policy) = form.backup.as_mut()
                {
                    form.preset = preset;
                    if let Some(mut next) = preset.policy(policy.automatic) {
                        // The space limit is set apart; a preset keeps it.
                        next.budget = policy.budget;
                        *policy = next;
                    }
                }
            }
            BackupsMessage::Count(index, value) => {
                if state.asking {
                    return Task::none();
                }
                if let Some(form) = state.retention.as_mut() {
                    let value = value.min(Retention::MAX);
                    match (index, form.home.as_mut()) {
                        (0, Some(home)) => home.hourly = value,
                        (1, Some(home)) => home.daily = value,
                        (2, Some(home)) => home.weekly = value,
                        (3, Some(home)) => home.monthly = value,
                        (4, Some(home)) => home.yearly = value,
                        (5, Some(home)) => home.manual = value,
                        (index @ 6..=10, _) => {
                            if let Some(policy) = form.backup.as_mut() {
                                match index {
                                    6 => policy.last = value.max(1),
                                    7 => policy.daily = value,
                                    8 => policy.weekly = value,
                                    9 => policy.monthly = value,
                                    _ => policy.yearly = value,
                                }
                                form.preset = BackupPreset::Custom;
                            }
                        }
                        _ => {}
                    }
                }
            }
            BackupsMessage::ApplyRetention => {
                let Some(form) = state.retention.as_ref() else {
                    return Task::none();
                };
                if state.asking {
                    return Task::none();
                }
                // Only what changed is sent, so one edit asks once.
                let home = form.home.filter(|home| {
                    safety.as_ref().and_then(|s| s.home_retention) != Some(*home)
                        && offers("home-retention-set")
                });
                let policy = form.backup.filter(|policy| {
                    policy.valid()
                        && safety.as_ref().and_then(|s| s.backup_retention) != Some(*policy)
                        && offers("backup-retention-set")
                });
                if home.is_none() && policy.is_none() {
                    state.retention = None;
                    return Task::none();
                }
                state.asking = true;
                return Task::perform(
                    async move {
                        if let Some(home) = home {
                            backend.set_home_retention(home).await?;
                        }
                        if let Some(policy) = policy {
                            backend.set_backup_retention(policy).await?;
                        }
                        Ok(())
                    },
                    |result| send(BackupsMessage::RetentionApplied(result)),
                );
            }
            BackupsMessage::RetentionApplied(result) => {
                state.asking = false;
                state.outcome_in_history = true;
                match result {
                    Ok(()) => {
                        state.retention = None;
                        state.outcome = Some(Outcome::Kept);
                    }
                    Err(error) => state.outcome = Some(Outcome::Failed(error)),
                }
                return self.read_system(true);
            }
            BackupsMessage::Dismiss => state.outcome = None,
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn backups_behaviour() {
        // Moments read relative to now.
        {
            let now = chrono::Local
                .with_ymd_and_hms(2026, 10, 4, 12, 0, 0)
                .unwrap();
            let at = |h, m| {
                chrono::Local
                    .with_ymd_and_hms(2026, 10, 4, h, m, 0)
                    .unwrap()
                    .to_rfc3339()
            };
            assert_eq!(
                when(&at(12, 0), now).replace(['\u{2068}', '\u{2069}'], ""),
                "Just now"
            );
            assert_eq!(
                when(&at(11, 59), now).replace(['\u{2068}', '\u{2069}'], ""),
                "1 minute ago"
            );
            assert_eq!(
                when(&at(9, 0), now).replace(['\u{2068}', '\u{2069}'], ""),
                "3 hours ago"
            );
            assert_eq!(
                when(&at(15, 0), now).replace(['\u{2068}', '\u{2069}'], ""),
                "in 3 hours"
            );
            assert_eq!(
                when("2026-10-03 08:30:00", now).replace(['\u{2068}', '\u{2069}'], ""),
                "Yesterday, 08:30"
            );
            assert_eq!(
                when("2026-09-28 14:00:00", now).replace(['\u{2068}', '\u{2069}'], ""),
                "Sep 28, 2026, 14:00"
            );
            assert_eq!(
                when("not a date", now).replace(['\u{2068}', '\u{2069}'], ""),
                "not a date"
            );
        }
        // Retention edits preserve schedule and cleanup off and cancel does not save.
        {
            let root = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                enzui_shell::settings::Store::at(root.path().join("settings.json")).unwrap(),
            );
            app.page = Category::Backups;
            let original = BackupPreset::Recommended.policy(true).unwrap();
            let safety = Safety {
                backup_retention: Some(original),
                schedule_enabled: true,
                operations: vec!["backup-retention-set".into()],
                ..Default::default()
            };
            app.system.readings.insert(
                Panel::Safety,
                Reading::Safety(Backed::Ready(Box::new(safety.clone()))),
            );
            let _ = app.backups_message(BackupsMessage::EditRetention(Some(false)));
            let _ = app.backups_message(BackupsMessage::CleanupAutomatic(false));
            let _ = app.backups_message(BackupsMessage::CleanupPreset(BackupPreset::More));
            assert_eq!(
                app.system.backups.retention.as_ref().unwrap().backup,
                BackupPreset::More.policy(false)
            );
            let _ = app.backups_message(BackupsMessage::Count(6, 1));
            assert_eq!(
                app.system.backups.retention.as_ref().unwrap().preset,
                BackupPreset::Custom
            );
            let _ = app.backups_message(BackupsMessage::EditRetention(None));
            assert!(app.system.backups.retention.is_none());
            assert_eq!(app.safety_now(), Some(safety));
        }
        // Pending or failed retention approval cannot mutate the live policy.
        {
            let root = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                enzui_shell::settings::Store::at(root.path().join("settings.json")).unwrap(),
            );
            app.page = Category::Backups;
            let original = BackupPreset::Recommended.policy(true).unwrap();
            app.system.readings.insert(
                Panel::Safety,
                Reading::Safety(Backed::Ready(Box::new(Safety {
                    backup_retention: Some(original),
                    operations: vec!["backup-retention-set".into()],
                    ..Default::default()
                }))),
            );
            let _ = app.backups_message(BackupsMessage::EditRetention(Some(false)));
            let _ = app.backups_message(BackupsMessage::CleanupPreset(BackupPreset::Less));
            let task = app.backups_message(BackupsMessage::ApplyRetention);
            assert!(task.units() > 0);
            assert!(app.system.backups.asking);
            let pending = app.system.backups.retention.as_ref().unwrap().backup;
            let _ = app.backups_message(BackupsMessage::Count(6, 1000));
            let _ = app.backups_message(BackupsMessage::CleanupAutomatic(false));
            let _ = app.backups_message(BackupsMessage::CleanupPreset(BackupPreset::More));
            assert_eq!(
                app.system.backups.retention.as_ref().unwrap().backup,
                pending
            );
            assert_eq!(
                app.backups_message(BackupsMessage::ApplyRetention).units(),
                0
            );
            let _ = app.backups_message(BackupsMessage::RetentionApplied(Err(
                "result unknown".into()
            )));
            assert!(!app.system.backups.asking);
            assert_eq!(app.safety_now().unwrap().backup_retention, Some(original));
            assert!(matches!(
                app.system.backups.outcome,
                Some(Outcome::Failed(_))
            ));
        }
    }
}
