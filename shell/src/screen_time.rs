//! Opt-in account-local screen-time history; retains two weeks and deletes history when
//! disabled.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const DAYS: usize = 14;
const APPS: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    /// Seconds unlocked.
    pub total: u32,
    /// Seconds each app (by window class) had the focus.
    pub apps: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    version: u32,
    /// By local date, `YYYY-MM-DD`.
    pub days: BTreeMap<String, Day>,
}

#[must_use]
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(state.join("enzui/screen-time.json"))
}

#[must_use]
pub fn load(path: &Path) -> History {
    let read = || -> std::io::Result<Vec<u8>> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let mut bytes = Vec::new();
        file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    let mut history = read()
        .ok()
        .filter(|bytes| bytes.len() <= 256 * 1024)
        .and_then(|bytes| serde_json::from_slice::<History>(&bytes).ok())
        .filter(|history| history.version == 1)
        .unwrap_or_default();
    history.days.retain(|date, day| {
        if date_label(date).is_none() {
            return false;
        }
        day.total = day.total.min(86_400);
        day.apps.retain(|class, seconds| {
            *seconds = (*seconds).min(86_400);
            valid_class(class)
        });
        while day.apps.len() > APPS {
            day.apps.pop_last();
        }
        true
    });
    while history.days.len() > DAYS {
        history.days.pop_first();
    }
    history
}

fn valid_class(class: &str) -> bool {
    !class.is_empty() && class.len() <= 128 && !class.chars().any(char::is_control)
}

/// Returns `MM-DD` only for canonical, valid `YYYY-MM-DD` calendar dates.
pub fn date_label(date: &str) -> Option<&str> {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
        || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
    {
        return None;
    }
    date.get(5..)
}

/// Adds `seconds` of unlocked time on a valid local `YYYY-MM-DD` date; invalid dates are ignored.
pub fn add(history: &mut History, date: &str, app: Option<&str>, seconds: u32) {
    if date_label(date).is_none() {
        return;
    }
    history.version = 1;
    let day = history.days.entry(date.to_owned()).or_default();
    day.total = day.total.saturating_add(seconds).min(86_400);
    if let Some(app) = app.filter(|class| valid_class(class))
        && (day.apps.contains_key(app) || day.apps.len() < APPS)
    {
        let used = day.apps.entry(app.to_owned()).or_default();
        *used = used.saturating_add(seconds).min(86_400);
    }
    while history.days.len() > DAYS {
        let oldest = history.days.keys().next().cloned().unwrap_or_default();
        history.days.remove(&oldest);
    }
}

pub fn save(path: &Path, history: &History) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, history)?;
    temporary
        .persist(path)
        .map(drop)
        .map_err(|error| error.error)
}

/// The desktop's own surfaces (bar, launcher, dialogs) are not apps.
#[must_use]
pub fn counts_as_app(class: &str) -> bool {
    !class.starts_with("dev.enzui.") || class == "dev.enzui.Settings"
}

/// Records one interval (blocking).
pub fn record(app: Option<&str>, seconds: u32) -> Result<(), String> {
    let path = path().ok_or_else(|| tr("shell-no-state-folder"))?;
    let mut history = load(&path);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    add(
        &mut history,
        &today,
        app.filter(|class| counts_as_app(class)),
        seconds,
    );
    save(&path, &history).map_err(|_| tr("shell-screen-time-could-not-be-saved").into())
}

/// Deletes the history.
pub fn forget() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}

/// "2 h 05 min" / "35 min".
#[must_use]
pub fn duration_text(seconds: u32) -> String {
    let minutes = seconds / 60;
    if minutes >= 60 {
        tr!("shell-h-min-2", "arg0" => format!("{:}", minutes / 60), "arg1" => format!("{:02}", minutes % 60))
    } else {
        tr!("shell-min-3", "arg0" => format!("{:}", minutes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_time_behaviour() {
        // Loaded history is bounded and malformed dates are ignored.
        {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("history.json");
            let mut history = History {
                version: 1,
                ..History::default()
            };
            for date in ["x", "ééééé", "2026-02-30"] {
                history.days.insert(date.into(), Day::default());
            }
            for day in 1..=20 {
                history.days.insert(
                    format!("2026-09-{day:02}"),
                    Day {
                        total: u32::MAX,
                        apps: (0..50)
                            .map(|i| (format!("app-{i}"), u32::MAX))
                            .chain([(String::new(), 1), ("bad\nclass".into(), 1)])
                            .collect(),
                    },
                );
            }
            std::fs::write(&file, serde_json::to_vec(&history).unwrap()).unwrap();
            let loaded = load(&file);
            assert_eq!(loaded.days.len(), DAYS);
            assert!(loaded.days.keys().all(|date| date_label(date).is_some()));
            assert!(loaded.days.values().all(|day| {
                day.total == 86_400
                    && day.apps.len() == APPS
                    && day
                        .apps
                        .iter()
                        .all(|(class, seconds)| valid_class(class) && *seconds == 86_400)
            }));
        }
        // Nonregular and oversized history are refused.
        {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("history.json");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &file,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            assert!(load(&file).days.is_empty());
            std::fs::remove_file(&file).unwrap();
            std::fs::write(&file, vec![b' '; 256 * 1024 + 1]).unwrap();
            assert!(load(&file).days.is_empty());
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(load(&link).days.is_empty());
            assert!(load(directory.path()).days.is_empty());
        }
    }
}
