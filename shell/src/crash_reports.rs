//! Metadata-only crash reports from the OS helper; core dumps and environments are never
//! loaded.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::time::Duration;

pub const HELPER: &str = "/usr/lib/enzimos-native/enzimos-policy";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub timestamp: String,
    pub name: String,
    pub executable: String,
    pub signal: String,
    pub pid: String,
}

impl Report {
    /// Identifies one crash across polls.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}", self.timestamp, self.pid)
    }
    /// The app's name: an enzUI app by its catalog name, else the process name.
    #[must_use]
    pub fn app(&self) -> String {
        let binary = self.executable.rsplit('/').next().unwrap_or_default();
        crate::catalog::APPS
            .iter()
            .find(|app| app.id == binary)
            .map_or_else(|| self.name.clone(), |app| app.display_name().to_owned())
    }
}

/// Reads the helper's JSON (schema 1).
#[must_use]
pub fn parse(text: &str) -> Option<Vec<Report>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("schema").and_then(serde_json::Value::as_u64) != Some(1) {
        return None;
    }
    let field = |row: &serde_json::Value, key: &str, limit: usize| -> String {
        row.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(limit)
            .collect()
    };
    Some(
        value
            .get("reports")?
            .as_array()?
            .iter()
            .take(64)
            .map(|row| Report {
                timestamp: field(row, "timestamp", 40),
                name: field(row, "name", 64),
                executable: field(row, "executable", 256),
                signal: field(row, "signal", 16),
                pid: field(row, "pid", 20),
            })
            .filter(|report| !report.timestamp.is_empty() && !report.name.is_empty())
            .collect(),
    )
}

/// The account's recent crashes; `None` without the OS helper.
pub async fn read() -> Option<Vec<Report>> {
    read_from(std::path::Path::new(HELPER)).await
}

async fn read_from(helper: &std::path::Path) -> Option<Vec<Report>> {
    if !helper.is_file() {
        return None;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(helper)
            .args(["desktop", "crash-reports"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

/// Tells the user, through the desktop's notification service.
pub async fn notify(report: &Report) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .map_err(|_| tr("shell-no-notification-service"))?;
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert("category", zbus::zvariant::Value::from("x-enzui.crash"));
    let summary = tr!("shell-quit-unexpectedly", "arg0" => report.app());
    let signal = if report.signal.is_empty() {
        String::new()
    } else {
        format!(" ({})", report.signal)
    };
    let body = tr!("shell-it-stopped-because-of-an-error-the-report-stays-on-this-computer-see-s", "arg0" => signal.clone());
    let actions: Vec<&str> = Vec::new();
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "Problem reports",
                0u32,
                "bug",
                summary.as_str(),
                body.as_str(),
                actions,
                hints,
                -1i32,
            ),
        )
        .await
        .map_err(|_| tr("shell-the-notification-was-not-accepted"))?;
    Ok(())
}
