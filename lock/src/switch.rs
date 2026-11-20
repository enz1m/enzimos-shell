//! User switching through org.enzimos.UserSwitch1, preserving locked sessions across VT
//! changes.

use enzui_shell::i18n::tr;
use std::process::{Command, Stdio};
use std::time::Duration;

const SERVICE: &str = "org.enzimos.UserSwitch1";
const PATH: &str = "/org/enzimos/UserSwitch1";

const DISPATCH: Duration = Duration::from_secs(25);
const QUERY: Duration = Duration::from_secs(10);

fn busctl(args: &[&str], patience: Duration) -> Option<String> {
    let timeout = format!("--timeout={}", patience.as_secs());
    let mut child = Command::new("busctl")
        .arg("--system")
        .arg(&timeout)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // A little past busctl's own timeout, so its answer is not cut off.
    let deadline = std::time::Instant::now() + patience + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                std::io::Read::read_to_string(child.stdout.as_mut()?, &mut out).ok()?;
                return Some(out);
            }
            Ok(Some(_)) => return None,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

// Whether the dispatcher is installed (running or activatable).
#[must_use]
pub(crate) fn available() -> bool {
    busctl(
        &[
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListActivatableNames",
        ],
        QUERY,
    )
    .into_iter()
    .chain(busctl(
        &[
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
        ],
        QUERY,
    ))
    .any(|names| names.contains(&format!("\"{SERVICE}\"")))
}

// Opens a greeter on another VT; only for a locked session.
pub(crate) fn open_greeter() -> Result<(), String> {
    busctl(&["call", SERVICE, PATH, SERVICE, "OpenGreeter"], DISPATCH)
        .map(drop)
        .ok_or_else(|| tr("shell-another-user-cannot-sign-in-right-now").into())
}

// Brings back the existing session `id`, which stays locked.
pub(crate) fn activate_existing(id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(tr("shell-unknown-session").into());
    }
    busctl(
        &["call", SERVICE, PATH, SERVICE, "ActivateExisting", "s", id],
        DISPATCH,
    )
    .map(drop)
    .ok_or_else(|| tr("shell-that-session-could-not-be-shown").into())
}

// `user`'s locked local graphical session on seat0, if it has one.
#[must_use]
pub(crate) fn existing_session(user: &str) -> Option<String> {
    // `--json=short`: `--output` selects journal output, not table JSON.
    let listed = Command::new("loginctl")
        .args(["list-sessions", "--no-legend", "--json=short"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let sessions: serde_json::Value = serde_json::from_slice(&listed.stdout).ok()?;
    sessions.as_array()?.iter().find_map(|row| {
        let id = row.get("session")?.as_str()?;
        if row.get("user")?.as_str()? != user
            || row.get("seat").and_then(serde_json::Value::as_str) != Some("seat0")
            || !id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return None;
        }
        let shown = Command::new("loginctl")
            .args([
                "show-session",
                id,
                "-p",
                "Class",
                "-p",
                "Remote",
                "-p",
                "LockedHint",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let props = String::from_utf8_lossy(&shown.stdout);
        (props.contains("Class=user")
            && props.contains("Remote=no")
            && props.contains("LockedHint=yes"))
        .then(|| id.to_owned())
    })
}
