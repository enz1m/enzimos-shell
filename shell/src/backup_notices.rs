//! Backup notifications that never nag.
//!
//! The shell checks the backup status a couple of minutes after login and then
//! every half hour. It speaks only when something needs the person:
//! - the backup drive is missing while a backup is owed: once per episode;
//! - a backup failed: once per failure episode;
//! - no successful backup for a week: a reminder, at most once a week;
//! - the space limit is smaller than one backup: once until it changes.
//!
//! Nothing is said for a successful backup. What was already said is kept in
//! `~/.local/state/enzui/backup-notices.json`, so restarts never repeat it.
use crate::i18n::tr;
use crate::tr;
use serde_json::{Value, json};

const WEEK: i64 = 7 * 86_400;

/// One notification to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub summary: String,
    pub body: String,
}

/// What was already said (the persisted document).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Said {
    /// `since` of the waiting-for-drive episode already announced.
    pub waiting: Option<i64>,
    /// `since` of the failure episode already announced.
    pub failed: Option<i64>,
    /// When the last "no recent backup" reminder was shown.
    pub reminded: Option<i64>,
    /// The space limit problem was announced (cleared when it goes away).
    pub budget: bool,
}
impl Said {
    fn from_json(v: &Value) -> Self {
        Self {
            waiting: v["waiting"].as_i64(),
            failed: v["failed"].as_i64(),
            reminded: v["reminded"].as_i64(),
            budget: v["budget"].as_bool().unwrap_or(false),
        }
    }
    fn to_json(&self) -> Value {
        json!({"waiting": self.waiting, "failed": self.failed, "reminded": self.reminded, "budget": self.budget})
    }
}

fn days(seconds: i64) -> i64 {
    (seconds / 86_400).max(1)
}

/// Decide what to say for the `backups` status, given what was said.
pub fn decide(status: &Value, said: &mut Said, now: i64) -> Option<Notice> {
    if status["local_configured"] != true || status["schedule_enabled"] != true {
        return None;
    }
    let drive = status["destination_label"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map_or_else(
            || tr("shell-backup-notice-the-backup-drive").to_owned(),
            |s| format!("“{s}”"),
        );
    let run = &status["run"];
    let state = run["state"].as_str().unwrap_or_default();
    let since = run["since"].as_i64();
    let last_success = run["last_success"].as_i64();
    // The space limit is smaller than one backup.
    let over = status["budget"]["over"] == true;
    if !over {
        said.budget = false;
    } else if !said.budget {
        said.budget = true;
        return Some(Notice {
            summary: tr("shell-backup-notice-limit-title").to_owned(),
            body: tr("shell-backup-notice-limit-body").to_owned(),
        });
    }
    match state {
        "waiting-for-drive" if since.is_some() && said.waiting != since => {
            said.waiting = since;
            said.reminded = Some(now);
            return Some(Notice {
                summary: tr!("shell-backup-notice-connect-title", "arg0" => drive),
                body: tr("shell-backup-notice-connect-body").to_owned(),
            });
        }
        "failed" if since.is_some() && said.failed != since => {
            said.failed = since;
            said.reminded = Some(now);
            return Some(Notice {
                summary: tr("shell-backup-notice-failed-title").to_owned(),
                body: tr("shell-backup-notice-failed-body").to_owned(),
            });
        }
        _ => {}
    }
    // A gentle weekly reminder when nothing succeeded for a week.
    let stale = last_success.is_none_or(|t| now - t >= WEEK);
    let quiet = said.reminded.is_none_or(|t| now - t >= WEEK);
    if stale && quiet && state != "ok" && !state.is_empty() {
        said.reminded = Some(now);
        let summary = match last_success {
            Some(t) => tr!("shell-backup-notice-stale-title", "arg0" => days(now - t).to_string()),
            None => tr("shell-backup-notice-never-title").to_owned(),
        };
        let body = if state == "waiting-for-drive" {
            tr!("shell-backup-notice-stale-connect", "arg0" => drive)
        } else {
            tr("shell-backup-notice-stale-body").to_owned()
        };
        return Some(Notice { summary, body });
    }
    None
}

fn state_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/state"))
        })?;
    Some(base.join("enzui/backup-notices.json"))
}

/// Check once: query the status, decide, remember and notify.
pub async fn check() -> Result<(), String> {
    let Some(backend) = crate::system_settings::enzimos::Backend::connect().await else {
        return Ok(());
    };
    let status = backend
        .query("backups")
        .await
        .map_err(|e| format!("{e:?}"))?;
    let path = state_path().ok_or("no state directory")?;
    let mut said = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|v| Said::from_json(&v))
        .unwrap_or_default();
    let before = said.clone();
    let now = chrono::Utc::now().timestamp();
    let notice = decide(&status, &mut said, now);
    if said != before {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, said.to_json().to_string()).map_err(|e| e.to_string())?;
    }
    if let Some(notice) = notice {
        notify(&notice).await?;
    }
    Ok(())
}

async fn notify(notice: &Notice) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus").to_owned())?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .map_err(|_| tr("shell-no-notification-service").to_owned())?;
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert("urgency", zbus::zvariant::Value::U8(1));
    hints.insert("category", zbus::zvariant::Value::from("x-enzui.backup"));
    let actions: Vec<&str> = Vec::new();
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                tr("shell-backups"),
                0u32,
                "shield-alert",
                notice.summary.as_str(),
                notice.body.as_str(),
                actions,
                hints,
                -1i32,
            ),
        )
        .await
        .map_err(|_| tr("shell-the-notification-was-not-accepted").to_owned())?;
    Ok(())
}

/// Shows a problem the shell met while acting for the user (a capture that
/// failed, an emoji that could not be typed, ...) as a notification from the
/// desktop itself. Errors that only `self.notice` held were never shown.
pub async fn post_shell_notice(text: String) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus").to_owned())?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .map_err(|_| tr("shell-no-notification-service").to_owned())?;
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert("urgency", zbus::zvariant::Value::U8(1));
    hints.insert("category", zbus::zvariant::Value::from("x-enzui.desktop"));
    hints.insert("transient", zbus::zvariant::Value::Bool(true));
    let actions: Vec<&str> = Vec::new();
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "enzimOS",
                0u32,
                "triangle-alert",
                text.as_str(),
                "",
                actions,
                hints,
                6000i32,
            ),
        )
        .await
        .map_err(|_| tr("shell-the-notification-was-not-accepted").to_owned())?;
    Ok(())
}

/// Ticks two minutes after start and then every half hour.
pub fn subscription() -> iced::Subscription<()> {
    iced::Subscription::run(|| {
        iced::futures::stream::unfold(true, |first| async move {
            let wait = if first { 120 } else { 30 * 60 };
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            Some(((), false))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(state: &str, since: i64, last: Option<i64>) -> Value {
        json!({"local_configured": true, "schedule_enabled": true, "destination_label": "Backup",
            "run": {"state": state, "since": since, "last_success": last}})
    }
    #[test]
    fn backup_notices_behaviour() {
        // A missing drive is announced once per episode then weekly at most.
        {
            let day = 86_400;
            let mut said = Said::default();
            let first = decide(&status("waiting-for-drive", 100, Some(50)), &mut said, 200);
            assert!(
                first.is_some_and(|n| n.summary.contains("Backup")),
                "first time: connect the drive"
            );
            for hour in 1..=48 {
                assert_eq!(
                    decide(
                        &status("waiting-for-drive", 100, Some(50)),
                        &mut said,
                        200 + hour * 3600
                    ),
                    None,
                    "no repeat after {hour} h"
                );
            }
            assert!(
                decide(
                    &status("waiting-for-drive", 100, Some(50)),
                    &mut said,
                    200 + 7 * day
                )
                .is_some(),
                "one reminder after a week"
            );
            assert_eq!(
                decide(
                    &status("waiting-for-drive", 100, Some(50)),
                    &mut said,
                    200 + 8 * day
                ),
                None,
                "then quiet again"
            );
            assert_eq!(
                decide(
                    &status("ok", 9_000_000, Some(9_000_000)),
                    &mut said,
                    9_000_100
                ),
                None,
                "success is silent"
            );
            assert!(
                decide(
                    &status("waiting-for-drive", 9_500_000, Some(9_000_000)),
                    &mut said,
                    9_500_100
                )
                .is_some(),
                "a new episode is announced"
            );
        }
        // Failures once and nothing without a schedule.
        {
            let mut said = Said::default();
            assert!(
                decide(&status("failed", 10, Some(5)), &mut said, 20).is_some(),
                "failure announced"
            );
            assert_eq!(
                decide(&status("failed", 10, Some(5)), &mut said, 30),
                None,
                "same failure is not repeated"
            );
            let mut off = status("failed", 99, None);
            off["schedule_enabled"] = json!(false);
            assert_eq!(
                decide(&off, &mut Said::default(), 100),
                None,
                "manual backups are never nagged about"
            );
        }
        // A too small limit is said once.
        {
            let mut said = Said::default();
            let mut s = status("ok", 10, Some(10));
            s["budget"] = json!({"over": true});
            assert!(decide(&s, &mut said, 20).is_some(), "limit announced");
            assert_eq!(decide(&s, &mut said, 30), None, "not again");
            s["budget"] = json!({"over": false});
            assert_eq!(decide(&s, &mut said, 40), None, "fixed: quiet and reset");
            assert!(!said.budget, "reset");
        }
    }
}
