//! Explicitly enables the account's local rootless Podman socket for Dev Monitor.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

const PODMAN: &str = "/usr/bin/podman";
const SYSTEMCTL: &str = "/usr/bin/systemctl";

/// Podman is installed, but this account's API socket is not listening.
#[must_use]
pub fn api_off() -> bool {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    socket_missing(Path::new(PODMAN), runtime.as_deref())
}

fn socket_missing(binary: &Path, runtime: Option<&Path>) -> bool {
    binary.is_file() && runtime.is_some_and(|runtime| !runtime.join("podman/podman.sock").exists())
}

/// Enables and starts the account's own `podman.socket`.
pub async fn enable_api() -> Result<(), String> {
    let run = tokio::process::Command::new(SYSTEMCTL)
        .args(["--user", "enable", "--now", "podman.socket"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(15), run)
        .await
        .map_err(|_| tr("shell-turning-on-the-podman-api-timed-out").to_owned())?
        .map_err(|error| tr!("shell-systemctl-could-not-start", "arg0" => format!("{:}", error)))?;
    if output.status.success() {
        return Ok(());
    }
    let reason: String = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .unwrap_or_else(|| tr("shell-systemctl-failed"))
        .chars()
        .take(200)
        .collect();
    Err(reason)
}
