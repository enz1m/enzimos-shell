//! Client for `org.enzimos.History1` (enzimos-history): space measurement,
//! what backups skip, removals waiting for the backup drive, and the free-space
//! floor of home history.
use crate::i18n::tr;
use serde_json::Value;

const NAME: &str = "org.enzimos.History1";
const PATH: &str = "/org/enzimos/History1";

/// Where the space goes, as last measured in the background.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// When it was measured (seconds since the epoch); `None` = never.
    pub time: Option<i64>,
    /// False when the time box ended the measurement early.
    pub complete: bool,
    /// Data only home history keeps alive.
    pub history_bytes: Option<u64>,
    /// Home history snapshots and the oldest one's time.
    pub history_count: u64,
    pub oldest: Option<i64>,
    /// Data only system snapshots keep alive.
    pub system_bytes: Option<u64>,
    pub system_count: u64,
    /// The caller's home: total and its folders (one or two levels deep).
    pub home_total: u64,
    pub folders: Vec<(String, u64)>,
    /// What backups skip in the caller's home, by reason (`build`, `cache`,
    /// `games`, `trash`, `excluded`), and the largest skipped folders.
    pub skipped: Vec<(String, u64)>,
    pub skipped_folders: Vec<(String, String, u64)>,
    /// Free space on the home disk now, and the floor history keeps.
    pub free_percent: Option<f64>,
    pub keep_free_percent: Option<u32>,
}

impl Usage {
    /// Parse the service's JSON document.
    pub fn from_json(value: &Value) -> Self {
        let mine = &value["mine"];
        let pairs = |key: &str| -> Vec<(String, u64)> {
            mine[key]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| {
                            Some((r["path"].as_str()?.to_owned(), r["bytes"].as_u64()?))
                        })
                        .take(200)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut skipped: Vec<(String, u64)> = mine["skipped"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                    .collect()
            })
            .unwrap_or_default();
        skipped.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        Self {
            time: value["time"].as_i64(),
            complete: value["complete"].as_bool().unwrap_or(false),
            history_bytes: value["home"]["history_bytes"].as_u64(),
            history_count: value["home"]["snapshots"].as_u64().unwrap_or(0),
            oldest: value["home"]["oldest"].as_i64(),
            system_bytes: value["system"]["snapshot_bytes"].as_u64(),
            system_count: value["system"]["snapshots"].as_u64().unwrap_or(0),
            home_total: mine["total"].as_u64().unwrap_or(0),
            folders: pairs("folders"),
            skipped,
            skipped_folders: mine["skipped_folders"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| {
                            Some((
                                r["path"].as_str()?.to_owned(),
                                r["reason"].as_str()?.to_owned(),
                                r["bytes"].as_u64()?,
                            ))
                        })
                        .take(100)
                        .collect()
                })
                .unwrap_or_default(),
            free_percent: value["disk"]["free_percent"].as_f64(),
            keep_free_percent: value["disk"]["keep_free_percent"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok()),
        }
    }
    /// Top-level folders of the home, largest first.
    pub fn top_folders(&self, count: usize) -> Vec<(String, u64)> {
        let mut top: Vec<(String, u64)> = self
            .folders
            .iter()
            .filter(|(p, _)| !p.contains('/'))
            .cloned()
            .collect();
        top.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        top.truncate(count);
        top
    }
    /// Subfolders of one top-level folder, largest first.
    pub fn children(&self, parent: &str, count: usize) -> Vec<(String, u64)> {
        let prefix = format!("{parent}/");
        let mut rows: Vec<(String, u64)> = self
            .folders
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .cloned()
            .collect();
        rows.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
        rows.truncate(count);
        rows
    }
}

/// What a person's backups skip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    /// Build output, caches, game files and the Trash are skipped.
    pub automatic: bool,
    /// Absolute paths excluded by the person.
    pub excluded: Vec<String>,
}

fn friendly(error: &zbus::Error) -> String {
    let text = match error {
        zbus::Error::MethodError(_, Some(detail), _) => detail.clone(),
        zbus::Error::FDO(e) => e.to_string(),
        other => other.to_string(),
    };
    if text.contains("not-authorized") {
        tr("shell-history-not-allowed").to_owned()
    } else if text.contains("no-history")
        || text.contains("ServiceUnknown")
        || text.contains("was not provided")
    {
        tr("shell-history-unavailable").to_owned()
    } else if text.contains("no-backup-drive") {
        tr("shell-history-no-backup-drive").to_owned()
    } else {
        text.chars().filter(|c| !c.is_control()).take(200).collect()
    }
}

async fn proxy() -> Result<zbus::Proxy<'static>, String> {
    let connection = zbus::Connection::system().await.map_err(|e| friendly(&e))?;
    zbus::Proxy::new_owned(connection, NAME, PATH, NAME)
        .await
        .map_err(|e| friendly(&e))
}

async fn call<B, R>(method: &str, body: &B) -> Result<R, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let proxy = proxy().await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        proxy.call(method, body),
    )
    .await
    .map_err(|_| tr("shell-history-timed-out").to_owned())?
    .map_err(|e| friendly(&e))
}

/// The cached measurement.
pub async fn usage() -> Result<Usage, String> {
    let text: String = call("Usage", &()).await?;
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(Usage::from_json(&value))
}
/// The caller's rules.
pub async fn rules() -> Result<Rules, String> {
    let (automatic, excluded): (bool, Vec<String>) = call("Rules", &()).await?;
    Ok(Rules {
        automatic,
        excluded,
    })
}
/// Turn the automatic rules on or off.
pub async fn set_automatic(on: bool) -> Result<(), String> {
    call("SetAutomatic", &(on,)).await
}
/// Exclude (or include again) one of the caller's folders.
pub async fn set_excluded(path: String, excluded: bool) -> Result<(), String> {
    call("SetExcluded", &(path, excluded)).await
}
/// Files waiting to be removed from the backup drive.
pub async fn removals() -> Result<Vec<(String, i64)>, String> {
    call("Removals", &()).await
}
/// Ask for a new measurement.
pub async fn measure() -> Result<(), String> {
    call("Measure", &()).await
}
/// Change the free space home history leaves (administrator).
pub async fn set_free_percent(percent: u32) -> Result<(), String> {
    call("SetFreePercent", &(percent,)).await
}

/// Demo data for screenshots and tests.
pub fn demo() -> (Usage, Rules, Vec<(String, i64)>) {
    let now = chrono::Utc::now().timestamp();
    let gib = 1u64 << 30;
    let usage = Usage {
        time: Some(now - 3 * 3600),
        complete: true,
        history_bytes: Some(6 * gib + gib / 3),
        history_count: 41,
        oldest: Some(now - 180 * 86_400),
        system_bytes: Some(3 * gib + gib / 2),
        system_count: 6,
        home_total: 142 * gib,
        folders: vec![
            ("Videos".into(), 61 * gib),
            ("Videos/Holiday 2026".into(), 38 * gib),
            ("Videos/Screen recordings".into(), 19 * gib),
            ("Code".into(), 34 * gib),
            ("Code/enzim".into(), 29 * gib),
            ("Code/site".into(), 4 * gib),
            ("Pictures".into(), 22 * gib),
            ("Documents".into(), 9 * gib),
            (".local".into(), 8 * gib),
            ("Music".into(), 5 * gib),
            ("Downloads".into(), 3 * gib),
        ],
        skipped: vec![
            ("build".into(), 27 * gib),
            ("games".into(), 6 * gib),
            ("cache".into(), 3 * gib),
            ("excluded".into(), 19 * gib),
            ("trash".into(), gib / 2),
        ],
        skipped_folders: vec![
            ("Code/enzim/target".into(), "build".into(), 24 * gib),
            (
                "Videos/Screen recordings".into(),
                "excluded".into(),
                19 * gib,
            ),
            (
                ".local/share/Steam/steamapps/common".into(),
                "games".into(),
                6 * gib,
            ),
            (".cache".into(), "cache".into(), 3 * gib),
            ("Code/site/node_modules".into(), "build".into(), 2 * gib),
        ],
        free_percent: Some(38.0),
        keep_free_percent: Some(15),
    };
    let rules = Rules {
        automatic: true,
        excluded: vec!["/home/alice/Videos/Screen recordings".into()],
    };
    (
        usage,
        rules,
        vec![(
            "/home/alice/Documents/old-passport-scan.pdf".into(),
            now - 3600,
        )],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_parses_the_service_document() {
        let value = serde_json::json!({"time": 10, "complete": true,
            "home": {"history_bytes": 500, "snapshots": 4, "oldest": 1},
            "system": {"snapshot_bytes": 70, "snapshots": 2},
            "mine": {"total": 900, "folders": [{"path": "A", "bytes": 300}, {"path": "A/b", "bytes": 200}, {"path": "C", "bytes": 600}],
                "skipped": {"build": 40, "cache": 60}, "skipped_folders": [{"path": "A/b", "reason": "build", "bytes": 40}]},
            "disk": {"free_percent": 33.3, "keep_free_percent": 15}});
        let usage = Usage::from_json(&value);
        assert_eq!(
            usage.top_folders(5),
            vec![("C".to_owned(), 600), ("A".to_owned(), 300)],
            "largest first"
        );
        assert_eq!(
            usage.children("A", 5),
            vec![("A/b".to_owned(), 200)],
            "children"
        );
        assert_eq!(
            usage.skipped[0],
            ("cache".to_owned(), 60),
            "reasons by size"
        );
        assert_eq!(
            (usage.history_bytes, usage.keep_free_percent),
            (Some(500), Some(15)),
            "totals"
        );
        assert_eq!(
            Usage::from_json(&serde_json::json!({"time": null})).time,
            None,
            "never measured"
        );
    }
}
