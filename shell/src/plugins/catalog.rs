//! Signed plugin catalogue client using OS-verified downloads.
//! Downloaded bytes must match the signed digest; installation still requires local permission
//! review.

use super::manifest::{MAX_PACKAGE_BYTES, Package};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

// Verification and transfer can take minutes on a slow link.
const PATIENCE: Duration = Duration::from_secs(200);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub sha256: String,
    pub bytes: u64,
}

#[must_use]
pub fn config() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("enzui/plugin-catalog.json")).filter(|path| path.is_file())
}

fn tool() -> Option<PathBuf> {
    ["/usr/bin/enzimos-updates", "/usr/local/bin/enzimos-updates"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// Whether this system can use a catalogue, and whether one is configured.
#[must_use]
pub fn available() -> (bool, bool) {
    (tool().is_some(), config().is_some())
}

fn text(value: &Value, key: &str, limit: usize) -> Option<String> {
    let text = value.get(key)?.as_str()?;
    (text.chars().count() <= limit && !text.chars().any(char::is_control)).then(|| text.to_owned())
}

fn hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The plugins a `plugin-check` answer lists.
pub fn entries(answer: &Value) -> Result<Vec<Entry>, String> {
    if answer["schema"] != 1
        || answer["installed"] != false
        || answer["enabled"] != false
        || answer["permissions_granted"] != false
    {
        return Err(tr("shell-the-catalogue-client-gave-an-unexpected-answer").into());
    }
    let plugins = answer["catalog"]["plugins"]
        .as_array()
        .ok_or_else(|| tr("shell-the-catalogue-lists-no-plugins"))?;
    plugins
        .iter()
        .take(32)
        .map(|row| {
            let entry = Entry {
                id: text(row, "id", 128).ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
                name: text(row, "name", 64).ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
                version: text(row, "version", 64)
                    .ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
                description: text(row, "description", 512)
                    .ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
                sha256: text(row, "sha256", 64)
                    .filter(|digest| hex64(digest))
                    .ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
                bytes: row["bytes"]
                    .as_u64()
                    .filter(|bytes| (1..=MAX_PACKAGE_BYTES as u64).contains(bytes))
                    .ok_or_else(|| tr("shell-invalid-catalogue-entry"))?,
            };
            Ok(entry)
        })
        .collect()
}

async fn run(arguments: &[&str]) -> Result<Value, String> {
    let tool = tool().ok_or_else(|| tr("shell-this-system-has-no-plugin-catalogue-client"))?;
    let config = config().ok_or_else(|| tr("shell-no-plugin-catalogue-is-configured"))?;
    let mut command = tokio::process::Command::new(tool);
    command
        .args(arguments)
        .arg("--config")
        .arg(&config)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PATIENCE, command.output())
        .await
        .map_err(|_| tr("shell-the-catalogue-did-not-answer-in-time"))?
        .map_err(|_| tr("shell-the-catalogue-client-could-not-start"))?;
    if !output.status.success() {
        // The client answers a refusal with `{"status": "failed", "message"}`
        // (signature, expiry, rollback, length, …) on stdout.
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        let reason: String = answer["message"]
            .as_str()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        return Err(if reason.is_empty() {
            tr("shell-the-catalogue-could-not-be-verified").into()
        } else {
            tr!("shell-the-catalogue-could-not-be-verified-2", "arg0" => reason)
        });
    }
    if output.stdout.len() > 256 * 1024 {
        return Err(tr("shell-the-catalogue-answer-is-too-large").into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| tr("shell-the-catalogue-client-gave-an-unexpected-answer").into())
}

/// Verifies the configured catalogue and lists its plugins.
pub async fn check() -> Result<Vec<Entry>, String> {
    entries(&run(&["plugin-check"]).await?)
}

/// Checks a downloaded package against the catalogue entry it was chosen
/// from, then parses it.
pub fn verify(entry: &Entry, download: &Value, bytes: &[u8]) -> Result<Package, String> {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let listed = |key: &str, value: &str| download[key].as_str() == Some(value);
    if digest != entry.sha256
        || !listed("sha256", &entry.sha256)
        || !listed("id", &entry.id)
        || !listed("version", &entry.version)
        || download["bytes"].as_u64() != Some(bytes.len() as u64)
    {
        return Err(tr("shell-the-downloaded-plugin-does-not-match-the-signed-catalogue").into());
    }
    let package = Package::parse(bytes)?;
    let manifest = package.manifest();
    if manifest.id != entry.id || manifest.version != entry.version {
        return Err(tr("shell-the-plugin-package-does-not-match-its-catalogue-entry").into());
    }
    Ok(package)
}

fn read_private(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| tr("shell-the-downloaded-plugin-is-missing"))?;
    let meta = file
        .metadata()
        .map_err(|_| tr("shell-the-downloaded-plugin-is-missing"))?;
    if !meta.is_file()
        || meta.uid() != rustix::process::getuid().as_raw()
        || meta.len() > MAX_PACKAGE_BYTES as u64
    {
        return Err(tr("shell-the-downloaded-plugin-is-not-this-account-s-private-file").into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PACKAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| tr("shell-the-downloaded-plugin-could-not-be-read"))?;
    Ok(bytes)
}

/// Downloads `entry` through the verified catalogue and returns its package.
pub async fn download(entry: &Entry) -> Result<Package, String> {
    let answer = run(&["plugin-download", "--id", &entry.id]).await?;
    let download = answer
        .get("download")
        .cloned()
        .ok_or_else(|| tr("shell-the-catalogue-did-not-return-the-plugin"))?;
    let path = PathBuf::from(
        download["path"]
            .as_str()
            .ok_or_else(|| tr("shell-the-catalogue-did-not-return-the-plugin"))?,
    );
    if !path.is_absolute() {
        return Err(tr("shell-the-catalogue-returned-an-unexpected-path").into());
    }
    let entry = entry.clone();
    tokio::task::spawn_blocking(move || verify(&entry, &download, &read_private(&path)?))
        .await
        .map_err(|_| tr("shell-the-plugin-could-not-be-checked").to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_bytes_matching_the_signed_entry_are_accepted() {
        let answer = json!({"schema": 1, "installed": false, "enabled": false, "permissions_granted": false,
            "catalog": {"schema": 1, "sequence": 3, "expires": "2026-10-08T00:00:00Z", "plugins": [
                {"id": "example", "name": "Example", "version": "1.0.0", "description": "An example", "target": "plugins/example/x.json", "bytes": 10, "sha256": "a".repeat(64)}]}});
        let listed = entries(&answer).unwrap();
        assert_eq!(listed[0].id, "example");
        assert!(entries(&json!({"schema": 1, "installed": true, "enabled": false, "permissions_granted": false, "catalog": {"plugins": []}})).is_err(), "a client claiming to have installed is refused");
        let bytes = b"not a package";
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let entry = Entry {
            sha256: digest.clone(),
            bytes: bytes.len() as u64,
            ..listed[0].clone()
        };
        let download = json!({"path": "/x", "bytes": bytes.len(), "sha256": digest, "id": "example", "version": "1.0.0"});
        // Matching bytes still have to be a valid package.
        assert!(verify(&entry, &download, bytes).is_err());
        assert_eq!(
            verify(&entry, &download, b"tampered!!!!!").err().as_deref(),
            Some("The downloaded plugin does not match the signed catalogue")
        );
    }
}
