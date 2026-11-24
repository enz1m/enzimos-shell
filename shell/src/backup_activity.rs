//! A running encrypted backup in the activity bar, whoever started it.
//!
//! The shell is the only publisher, so a backup started from Settings, by the
//! schedule or by plugging in the drive shows exactly once, also after
//! Settings is closed.
use crate::i18n::tr;
use enzui_activity::{Job, Outcome, Publisher, Status, Visibility};
use serde_json::Value;
use std::time::Duration;

const IDLE: Duration = Duration::from_secs(15);
const RUNNING: Duration = Duration::from_secs(3);
const UNAVAILABLE: Duration = Duration::from_secs(120);

/// What `Query backups` says under `running`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub since: i64,
    pub phase: String,
    pub target: String,
}

impl Running {
    pub fn from_status(status: &Value) -> Option<Self> {
        let running = status.get("running").filter(|value| value.is_object())?;
        Some(Self {
            since: running["since"].as_i64()?,
            phase: running["phase"].as_str().unwrap_or_default().to_owned(),
            target: running["target"].as_str().unwrap_or_default().to_owned(),
        })
    }

    pub fn status(&self) -> Status {
        let label = match (self.phase.as_str(), self.target.as_str()) {
            ("copying", _) => tr("shell-backup-activity-copying"),
            (_, "share") => tr("shell-backup-activity-to-share"),
            _ => tr("shell-backup-activity-to-drive"),
        };
        let detail = match self.phase.as_str() {
            "capturing" => tr("shell-backup-activity-capturing"),
            "transferring" | "copying" => tr("shell-backup-activity-transferring"),
            "finishing" => tr("shell-backup-activity-finishing"),
            _ => tr("shell-backup-activity-preparing"),
        };
        Status {
            detail: detail.to_owned(),
            ..Status::running(label)
        }
    }
}

/// How a run that is no longer reported ended, from the status read after it.
pub fn outcome(status: &Value, since: i64) -> Outcome {
    let run = &status["run"];
    let failed_after = run["state"] == "failed"
        && run["at"]
            .as_i64()
            .or_else(|| run["since"].as_i64())
            .unwrap_or(0)
            >= since;
    if failed_after {
        Outcome::Failed
    } else {
        Outcome::Succeeded
    }
}

struct Tracker {
    publisher: Option<Publisher>,
    job: Option<(Job, Running)>,
}

impl Tracker {
    fn apply(&mut self, status: &Value) {
        match (Running::from_status(status), self.job.take()) {
            (Some(running), Some((job, shown))) if shown.since == running.since => {
                if shown != running {
                    let _ = job.update(running.status());
                }
                self.job = Some((job, running));
            }
            (Some(running), previous) => {
                if let Some((job, shown)) = previous {
                    let _ = job.finish(outcome(status, shown.since));
                }
                if self.publisher.is_none() {
                    self.publisher = Publisher::new("dev.enzui.Settings").ok();
                }
                self.job = self.publisher.as_ref().and_then(|publisher| {
                    publisher
                        .start_with_visibility(running.status(), Visibility::Always)
                        .ok()
                        .map(|job| (job, running))
                });
            }
            (None, Some((job, shown))) => {
                let _ = job.finish(outcome(status, shown.since));
            }
            (None, None) => {}
        }
    }
}

/// Follows the backup status for the whole session; it never yields.
pub fn subscription() -> iced::Subscription<()> {
    iced::Subscription::run(|| iced::futures::stream::once(follow()))
}

async fn follow() {
    let mut tracker = Tracker {
        publisher: None,
        job: None,
    };
    loop {
        // System1 exits when idle; asking it while no backup unit runs would
        // keep it alive for the whole session. The last read after a tracked
        // run ends still goes to System1 for that run's outcome.
        if tracker.job.is_none() && backup_unit_active().await == Some(false) {
            tokio::time::sleep(IDLE).await;
            continue;
        }
        let wait = match crate::system_settings::enzimos::Backend::connect().await {
            Some(backend) => match backend.query("backups").await {
                Ok(status) => {
                    tracker.apply(&status);
                    if tracker.job.is_some() { RUNNING } else { IDLE }
                }
                Err(_) => UNAVAILABLE,
            },
            None => UNAVAILABLE,
        };
        tokio::time::sleep(wait).await;
    }
}

/// What systemd's `ListUnitsByPatterns` returns for each unit.
type Unit = (
    String,
    String,
    String,
    String,
    String,
    String,
    zbus::zvariant::OwnedObjectPath,
    u32,
    String,
    zbus::zvariant::OwnedObjectPath,
);
type UnitQuery = (&'static [&'static str], &'static [&'static str]);
const BACKUP_UNITS: UnitQuery = (
    &["active", "activating", "deactivating", "reloading"],
    &["enzimos-backup*.service"],
);

/// Whether a backup service runs: scheduled, manual and drive-attached runs
/// are all `enzimos-backup*` units. `None` when systemd cannot be asked.
async fn backup_unit_active() -> Option<bool> {
    let units: Vec<Unit> = crate::bus::system()
        .await?
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "ListUnitsByPatterns",
            &BACKUP_UNITS,
        )
        .await
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    Some(!units.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn running_backups_and_their_outcome() {
        assert_eq!(Running::from_status(&json!({"running": null})), None);
        let running = Running::from_status(&json!({
            "running": {"since": 100, "phase": "capturing", "target": "share"}
        }))
        .unwrap();
        assert_eq!(running.since, 100);
        assert_eq!(running.status().label, tr("shell-backup-activity-to-share"));
        assert_eq!(
            running.status().detail,
            tr("shell-backup-activity-capturing")
        );
        assert_eq!(
            outcome(&json!({"run": {"state": "failed", "at": 120}}), 100),
            Outcome::Failed
        );
        assert_eq!(
            outcome(&json!({"run": {"state": "failed", "at": 50}}), 100),
            Outcome::Succeeded,
            "an older failure does not mark this run"
        );
    }
}
