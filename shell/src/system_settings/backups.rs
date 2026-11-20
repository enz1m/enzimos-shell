//! Local Snapper history and reviewed OS backup/restore jobs.
//! Missing backend queries or fields remain unavailable rather than becoming empty results.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
mod retention;
pub use retention::{BackupPreset, BackupRetention, Cleanup, CleanupState};

use super::{SystemSettings, demo, enzimos};
use serde_json::{Value, json};
use std::time::Duration;

/// One local Snapper snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub number: u64,
    pub date: String,
    pub kind: SnapshotKind,
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotKind {
    Automatic,
    Before,
    After,
    Manual,
}

/// Where a recovery point is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Local,
    Remote,
    Both,
}

/// One recovery point in the encrypted repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    /// restic's full snapshot ID: 64 lowercase hex digits.
    pub id: String,
    pub time: String,
    pub place: Place,
    pub complete: bool,
    pub size: Option<u64>,
}

/// How the last backup ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastBackup {
    pub time: String,
    pub state: String,
    pub point: Option<String>,
}

/// How many home snapshots of each kind future cleanup keeps (schema 30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    pub hourly: u32,
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
    pub manual: u32,
}
impl Retention {
    pub const MAX: u32 = 1000;
    pub fn from_json(value: &Value) -> Option<Self> {
        let count = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
        };
        Some(Self {
            hourly: count("hourly")?,
            daily: count("daily")?,
            weekly: count("weekly")?,
            monthly: count("monthly")?,
            yearly: count("yearly")?,
            manual: count("manual")?,
        })
    }
    /// The backend's bounds: each count at most 1000, at least one snapshot
    /// taken by hand, and 1–1000 timeline snapshots in all.
    pub fn valid(self) -> bool {
        let timeline: u64 = [
            self.hourly,
            self.daily,
            self.weekly,
            self.monthly,
            self.yearly,
        ]
        .iter()
        .map(|n| u64::from(*n))
        .sum();
        [
            self.hourly,
            self.daily,
            self.weekly,
            self.monthly,
            self.yearly,
            self.manual,
        ]
        .iter()
        .all(|n| *n <= Self::MAX)
            && self.manual >= 1
            && (1..=u64::from(Self::MAX)).contains(&timeline)
    }
    fn to_json(self) -> Value {
        json!({"hourly": self.hourly, "daily": self.daily, "weekly": self.weekly, "monthly": self.monthly, "yearly": self.yearly, "manual": self.manual})
    }
}

/// A single-approval restore planned by the backend: the review to show
/// before an administrator approves it.
#[derive(Clone, Debug)]
pub struct RestorePlan {
    pub filename: String,
    planned: Option<enzimos::Planned>,
}

const LISTED: usize = 100;

pub fn full_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(|s| s.chars().take(200).collect())
}

fn snapshot_rows(value: Option<&Value>) -> Vec<Snapshot> {
    let Some(rows) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut rows: Vec<Snapshot> = rows
        .iter()
        .filter_map(|row| {
            let number = row.get("number")?.as_u64().filter(|n| *n > 0)?;
            let kind = match row.get("kind").and_then(Value::as_str) {
                Some("pre") => SnapshotKind::Before,
                Some("post") => SnapshotKind::After,
                Some("single")
                    if row.get("cleanup").and_then(Value::as_str) == Some("timeline") =>
                {
                    SnapshotKind::Automatic
                }
                Some("single")
                    if row
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some_and(|d| d.starts_with("enzimOS required update fallback")) =>
                {
                    SnapshotKind::Before
                }
                Some("single") => SnapshotKind::Manual,
                Some("timeline") => SnapshotKind::Automatic,
                _ => return None,
            };
            Some(Snapshot {
                number,
                date: row.get("date").and_then(text)?,
                kind,
                description: row.get("description").and_then(text).unwrap_or_default(),
            })
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.number));
    rows.truncate(LISTED);
    rows
}

/// The `snapshots` query: newest first, (system, home).
pub fn snapshots(value: &Value) -> (Vec<Snapshot>, Vec<Snapshot>) {
    (
        snapshot_rows(value.get("root")),
        snapshot_rows(value.get("home")),
    )
}

/// The `backup-points` query: newest first; rows without a full ID are dropped.
pub fn points(value: &Value) -> Vec<Point> {
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    let mut points: Vec<Point> = rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str().filter(|id| full_id(id))?.to_owned();
            let place = match row.get("where").and_then(Value::as_str) {
                Some("remote") => Place::Remote,
                Some("both") => Place::Both,
                _ => Place::Local,
            };
            Some(Point {
                id,
                time: row.get("time").and_then(text)?,
                place,
                complete: row
                    .get("complete")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                size: row.get("size_bytes").and_then(Value::as_u64),
            })
        })
        .collect();
    points.sort_by(|a, b| b.time.cmp(&a.time));
    points.truncate(LISTED);
    points
}

pub fn last_backup(backups: &Value) -> Option<LastBackup> {
    let last = backups.get("last_backup")?;
    Some(LastBackup {
        time: last.get("time").and_then(text)?,
        state: last.get("state").and_then(text).unwrap_or_default(),
        point: last
            .get("point")
            .and_then(Value::as_str)
            .filter(|id| full_id(id))
            .map(str::to_owned),
    })
}

/// A path typed relative to the home folder, as the backend's selective
/// restore names it (`/backup-source/home/<user>/…`), or why it cannot be.
pub fn source_path(home: &std::path::Path, typed: &str) -> Result<String, &'static str> {
    let user = home
        .strip_prefix("/home")
        .ok()
        .and_then(|rest| rest.to_str())
        .filter(|user| !user.is_empty() && !user.contains('/'))
        .filter(|user| {
            user.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        })
        .ok_or_else(|| tr("shell-restoring-works-for-accounts-whose-home-folder-is-in-home"))?;
    let typed = typed.trim().trim_start_matches("~/").trim_matches('/');
    if typed.is_empty() {
        return Err(tr(
            "shell-type-the-file-or-folder-s-place-in-your-home-folder-like-documents-report-od",
        ));
    }
    let parts: Vec<&str> = typed.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == ".." || *part == ".snapshots")
    {
        return Err(tr(
            "shell-use-a-plain-path-inside-your-home-folder-without-or",
        ));
    }
    if !parts.iter().all(|part| {
        part.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_. -".contains(c))
    }) {
        return Err(tr(
            "shell-this-path-has-characters-the-restore-service-does-not-accept-yet",
        ));
    }
    let path = format!("/backup-source/home/{user}/{typed}");
    if path.len() > 1024 {
        return Err(tr("shell-this-path-is-too-long"));
    }
    Ok(path)
}

/// A fresh identifier for the export and grant of one restore.
pub fn restore_id() -> String {
    format!(
        "settings-{}",
        enzimos::request_id().chars().take(20).collect::<String>()
    )
}

fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}

const STEP: Duration = Duration::from_secs(600);

impl SystemSettings {
    /// Restores to a new private home file through separately reviewed export, grant and
    /// delivery jobs.
    pub async fn restore_from_backup(
        &self,
        point: &str,
        path: &str,
        folder: bool,
        id: &str,
    ) -> Result<String, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !full_id(point) {
            return Err(tr("shell-that-recovery-point-is-not-known").into());
        }
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| tr("shell-your-home-folder-is-not-known"))?;
        let source = source_path(&home, path).map_err(String::from)?;
        if self.demo {
            tokio::time::sleep(Duration::from_millis(1800)).await;
            return Ok(format!("enzimos-restored-{id}.bin"));
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        let uid = own_uid().ok_or_else(|| tr("shell-your-account-is-not-known"))?;
        let (schema, export) = if folder {
            (24, "tree-restore-export")
        } else {
            (19, "file-restore-export")
        };
        let stage = |step: &str, refusal: enzimos::Refusal| {
            if refusal.is_uncertain() {
                tr!("shell-may-or-may-not-have-finished-nothing-more-was-done-check-your-home-fol", "arg0" => step.to_owned())
            } else {
                tr!("shell-did-not-happen", "arg0" => step.to_owned(), "arg1" => refusal.message())
            }
        };
        backend
            .run(
                schema,
                export,
                Some(json!({"id": id, "snapshot": point, "path": source})),
                STEP,
            )
            .await
            .map_err(|r| stage(tr("shell-finding-it-in-the-backup"), r))?;
        let grant = backend
            .run(
                20,
                "file-restore-grant",
                Some(json!({"id": id, "export": id, "uid": uid})),
                STEP,
            )
            .await
            .map_err(|r| stage(tr("shell-allowing-your-account-to-receive-it"), r))?;
        let digest = grant
            .get("grant_sha256")
            .and_then(Value::as_str)
            .filter(|d| d.len() == 64)
            .ok_or_else(|| enzimos::Refusal::Invalid.message())?;
        let delivered = backend
            .run(
                21,
                "file-restore-deliver",
                Some(json!({"id": id, "grant_sha256": digest})),
                STEP,
            )
            .await
            .map_err(|r| stage(tr("shell-copying-it-into-your-home-folder"), r))?;
        Ok(delivered
            .get("filename")
            .and_then(Value::as_str)
            .map_or_else(|| format!("enzimos-restored-{id}.bin"), str::to_owned))
    }
    /// Plans restoring one file with `own-file-restore` (schema 29): the
    /// backend names the new file, which is shown before anything is asked.
    pub async fn plan_own_restore(
        &self,
        point: &str,
        path: &str,
        id: &str,
    ) -> Result<RestorePlan, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !full_id(point) {
            return Err(tr("shell-that-recovery-point-is-not-known").into());
        }
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| tr("shell-your-home-folder-is-not-known"))?;
        let source = source_path(&home, path).map_err(String::from)?;
        if self.demo {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let name = source.rsplit('/').next().unwrap_or("file");
            let (stem, ext) = name
                .rsplit_once('.')
                .map_or((name, String::new()), |(stem, ext)| {
                    (stem, format!(".{ext}"))
                });
            return Ok(RestorePlan {
                filename: format!(
                    "{stem} (restored {}){ext}",
                    id.get(id.len().saturating_sub(12)..)
                        .ok_or_else(|| tr("shell-invalid-restore-identifier"))?
                ),
                planned: None,
            });
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        let planned = backend
            .plan(29, "own-file-restore", Some(json!({"id": id, "snapshot": point, "path": source})))
            .await
            .map_err(|refusal| tr!("shell-it-could-not-be-found-in-that-backup", "arg0" => refusal.message()))?;
        let filename = planned
            .reply
            .pointer("/review/filename")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && !name.contains('/') && name.len() <= 255)
            .ok_or_else(|| enzimos::Refusal::Invalid.message())?
            .to_owned();
        Ok(RestorePlan {
            filename,
            planned: Some(planned),
        })
    }
    /// Runs a reviewed restore: one administrator approval covers finding,
    /// granting and copying the file. Returns the new file's name.
    pub async fn finish_own_restore(&self, plan: RestorePlan) -> Result<String, String> {
        let Some(planned) = plan.planned else {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            return Ok(plan.filename);
        };
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        match backend.finish(&planned, STEP).await {
            Ok(result) => Ok(result.get("filename").and_then(Value::as_str).map_or(plan.filename, str::to_owned)),
            Err(refusal) if refusal.is_uncertain() => {
                Err(tr("shell-the-restore-may-or-may-not-have-finished-nothing-more-was-done-check-your-ho").into())
            }
            Err(refusal) => Err(refusal.message()),
        }
    }
    /// Takes a snapshot of the home folder now; returns its number.
    pub async fn snapshot_home(&self) -> Result<u64, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            return Ok(141);
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        backend.snapshot_home().await.map_err(|refusal| {
            if refusal.is_uncertain() {
                tr("shell-the-snapshot-may-or-may-not-have-been-taken-check-the-list-before-taking-ano").into()
            } else {
                refusal.message()
            }
        })
    }
    /// Sets how many home snapshots future cleanup keeps (schema 30). The
    /// snapshots there are now are not removed by this.
    pub async fn set_home_retention(&self, retention: Retention) -> Result<(), String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !retention.valid() {
            return Err(tr(
                "shell-keep-at-least-one-snapshot-taken-by-hand-and-between-1-and-1000-timed-ones",
            )
            .into());
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(800)).await;
            demo::set_retention(retention);
            return Ok(());
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        backend
            .run(30, "home-retention-set", Some(retention.to_json()), STEP)
            .await
            .map(drop)
            .map_err(|refusal| refusal.message())
    }
    pub async fn set_backup_retention(&self, policy: BackupRetention) -> Result<(), String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !policy.valid() {
            return Err(
                tr("shell-keep-1-1000-newest-backups-and-0-1000-in-each-calendar-tier").into(),
            );
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(800)).await;
            demo::set_backup_retention(policy);
            return Ok(());
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        backend
            .run(31, "backup-retention-set", Some(policy.selection()), STEP)
            .await
            .map(drop)
            .map_err(|refusal| refusal.message())
    }
    /// Copies a recovery point to the network share (`to_share`), or brings
    /// one kept only there back to this computer's repository.
    pub async fn move_point(&self, point: &str, to_share: bool) -> Result<(), String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !full_id(point) {
            return Err(tr("shell-that-recovery-point-is-not-known").into());
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            demo::move_point(point, to_share);
            return Ok(());
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        let hours = Duration::from_secs(3 * 3600);
        let result = if to_share {
            backend
                .run(
                    22,
                    "remote-replicate",
                    Some(json!({"snapshot": point})),
                    hours,
                )
                .await
        } else {
            backend
                .run(
                    23,
                    "remote-retrieve",
                    Some(json!({"id": restore_id(), "snapshot": point})),
                    hours,
                )
                .await
        };
        result.map(drop).map_err(|refusal| refusal.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_ids_are_safe_backend_ids() {
        let id = restore_id();
        assert!(id.len() <= 81 && id.starts_with("settings-"));
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        );
        assert_ne!(id, restore_id());
    }
}
