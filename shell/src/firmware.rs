//! Device firmware updates through fwupd (the Linux Vendor Firmware Service).
//!
//! Devices and available releases are read over fwupd's D-Bus interface.
//! Checking for new firmware and installing it run `fwupdmgr`, which
//! downloads and verifies the signed files itself and asks polkit; nothing
//! here contacts the internet on its own, and nothing runs in the background:
//! the metadata is only refreshed when the user asks.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

const SERVICE: &str = "org.freedesktop.fwupd";
const READ_DEADLINE: Duration = Duration::from_secs(20);
/// Checking downloads the catalogue of all vendors' firmware (a few MB).
const REFRESH_DEADLINE: Duration = Duration::from_secs(180);
/// Flashing can take minutes, plus the administrator dialog.
const INSTALL_DEADLINE: Duration = Duration::from_secs(1200);

// fwupd device flags (fwupd-enums.h).
const FLAG_INTERNAL: u64 = 1 << 0;
const FLAG_UPDATABLE: u64 = 1 << 1;
const FLAG_REQUIRE_AC: u64 = 1 << 3;
const FLAG_LOCKED: u64 = 1 << 4;
// fwupd update states: an update waits for a restart. The needs-reboot and
// needs-shutdown device flags only say how any update would be applied.
const UPDATE_STATE_PENDING: u64 = 1;
const UPDATE_STATE_NEEDS_REBOOT: u64 = 4;

/// One device fwupd knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Built into the computer (not a dock, mouse or other accessory).
    pub internal: bool,
    /// fwupd can update it.
    pub updatable: bool,
    /// Plug in the charger first.
    pub needs_power: bool,
    /// An update was installed and finishes at the next restart.
    pub pending_restart: bool,
    /// The newest release offered for it, if newer than installed.
    pub update: Option<Release>,
}

/// A firmware release offered for a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub summary: String,
    /// Plain text, from the release notes' paragraphs and list items.
    pub notes: String,
    pub size: u64,
    /// fwupd urgency: 0 unknown, 1 low, 2 medium, 3 high, 4 critical.
    pub urgency: u32,
}

/// What the firmware page shows.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Overview {
    /// fwupd is installed.
    pub available: bool,
    pub devices: Vec<Device>,
    /// When the firmware catalogue was last downloaded (seconds since the epoch).
    pub checked: Option<u64>,
}

impl Overview {
    /// Devices with an update, in display order.
    #[must_use]
    pub fn updates(&self) -> usize {
        self.devices.iter().filter(|d| d.update.is_some()).count()
    }
}

/// Outcome of an installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Done,
    /// Restart the computer to finish (for example UEFI firmware).
    RestartNeeded,
}

type Dict = HashMap<String, OwnedValue>;

fn text(dict: &Dict, key: &str) -> String {
    dict.get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

fn number(dict: &Dict, key: &str) -> u64 {
    dict.get(key)
        .and_then(|v| {
            u64::try_from(v.clone())
                .ok()
                .or_else(|| u32::try_from(v.clone()).ok().map(u64::from))
        })
        .unwrap_or_default()
}

/// Plain text from fwupd's AppStream description markup (`<p>`, `<li>`).
#[must_use]
pub fn plain_notes(markup: &str) -> String {
    let mut out = String::new();
    let mut rest = markup;
    while let Some((before, after)) = rest.split_once('<') {
        out.push_str(before);
        let Some((tag, tail)) = after.split_once('>') else {
            rest = "";
            break;
        };
        let tag = tag.trim().to_ascii_lowercase();
        if tag == "li" {
            out.push_str("\n• ");
        } else if tag == "/p" || tag == "/ul" || tag == "/ol" {
            out.push('\n');
        }
        rest = tail;
    }
    out.push_str(rest);
    let decoded = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let lines: Vec<&str> = decoded
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n").chars().take(2000).collect()
}

fn release(dict: &Dict) -> Release {
    Release {
        version: text(dict, "Version"),
        summary: text(dict, "Summary"),
        notes: plain_notes(
            &dict
                .get("Description")
                .and_then(|v| String::try_from(v.clone()).ok())
                .unwrap_or_default(),
        ),
        size: number(dict, "Size"),
        urgency: u32::try_from(number(dict, "Urgency")).unwrap_or(0),
    }
}

fn device(dict: &Dict) -> Device {
    let flags = number(dict, "Flags");
    Device {
        id: text(dict, "DeviceId"),
        name: text(dict, "Name"),
        vendor: text(dict, "Vendor"),
        version: text(dict, "Version"),
        internal: flags & FLAG_INTERNAL != 0,
        updatable: flags & FLAG_UPDATABLE != 0 && flags & FLAG_LOCKED == 0,
        needs_power: flags & FLAG_REQUIRE_AC != 0,
        pending_restart: matches!(
            number(dict, "UpdateState"),
            UPDATE_STATE_PENDING | UPDATE_STATE_NEEDS_REBOOT
        ),
        update: None,
    }
}

async fn proxy() -> Result<zbus::Proxy<'static>, String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    zbus::Proxy::new_owned(system, SERVICE, "/", SERVICE)
        .await
        .map_err(|_| tr("shell-firmware-service-unavailable").to_owned())
}

/// Read devices, their offered updates and when the catalogue was checked.
///
/// # Errors
/// A sentence when fwupd does not answer.
pub async fn read() -> Result<Overview, String> {
    let Ok(proxy) = proxy().await else {
        return Ok(Overview::default());
    };
    let devices: Vec<Dict> =
        match tokio::time::timeout(READ_DEADLINE, proxy.call("GetDevices", &())).await {
            Err(_) => return Err(tr("shell-firmware-service-did-not-answer").into()),
            Ok(Err(zbus::Error::MethodError(name, _, _)))
                if name.as_str().contains("ServiceUnknown")
                    || name.as_str().contains("NameHasNoOwner") =>
            {
                return Ok(Overview::default());
            }
            Ok(Err(zbus::Error::MethodError(name, _, _)))
                if name.as_str().ends_with("NothingToDo") =>
            {
                Vec::new()
            }
            Ok(Err(_)) => return Err(tr("shell-firmware-service-did-not-answer").into()),
            Ok(Ok(devices)) => devices,
        };
    let mut out: Vec<Device> = devices
        .iter()
        .map(device)
        .filter(|d| !d.id.is_empty() && !d.name.is_empty())
        .collect();
    for item in out.iter_mut().filter(|d| d.updatable) {
        let releases: Vec<Dict> = match tokio::time::timeout(
            READ_DEADLINE,
            proxy.call("GetUpgrades", &(item.id.as_str(),)),
        )
        .await
        {
            Ok(Ok(releases)) => releases,
            _ => Vec::new(),
        };
        item.update = releases.first().map(release);
    }
    // Updates first, then the computer's own parts, then accessories.
    out.sort_by(|a, b| {
        (b.update.is_some(), b.internal, b.updatable, &a.name).cmp(&(
            a.update.is_some(),
            a.internal,
            a.updatable,
            &b.name,
        ))
    });
    let remotes: Vec<Dict> =
        match tokio::time::timeout(READ_DEADLINE, proxy.call("GetRemotes", &())).await {
            Ok(Ok(remotes)) => remotes,
            _ => Vec::new(),
        };
    let checked = remotes
        .iter()
        .filter(|r| {
            r.get("Enabled")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false)
        })
        .map(|r| number(r, "ModificationTime"))
        // Local remotes without a downloaded catalogue report impossible times.
        .filter(|t| {
            *t > 0
                && *t
                    < std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs())
                        + 86_400
        })
        .max();
    Ok(Overview {
        available: true,
        devices: out,
        checked,
    })
}

async fn fwupdmgr(args: &[&str], deadline: Duration) -> Result<i32, String> {
    let mut command = tokio::process::Command::new("fwupdmgr");
    command
        .args(args)
        // Never prompt on a terminal, never upload reports, plain output.
        .args([
            "--assume-yes",
            "--no-unreported-check",
            "--no-metadata-check",
        ])
        .env("LANG", "C.UTF-8")
        .env_remove("LANGUAGE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = command
        .spawn()
        .map_err(|_| tr("shell-firmware-tools-not-installed").to_owned())?;
    let output = tokio::time::timeout(deadline, child.wait_with_output())
        .await
        .map_err(|_| tr("shell-the-firmware-step-took-too-long").to_owned())?
        .map_err(|_| tr("shell-firmware-tools-not-installed").to_owned())?;
    let code = output.status.code().unwrap_or(1);
    if code == 0 || code == 2 {
        return Ok(code);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    Err(
        if stderr.contains("not authorized")
            || stderr.contains("authentication")
            || stderr.contains("permission")
        {
            tr("shell-an-administrator-did-not-approve-this-change").into()
        } else if stderr.contains("ac power") || stderr.contains("battery") {
            tr("shell-plug-in-the-charger-before-updating-firmware").into()
        } else if stderr.contains("download")
            || stderr.contains("resolve")
            || stderr.contains("network")
            || stderr.contains("connect")
        {
            tr("shell-the-firmware-could-not-be-downloaded-check-the-internet-connection").into()
        } else {
            tr("shell-the-firmware-update-did-not-finish").into()
        },
    )
}

/// Download the current firmware catalogue from the LVFS.
///
/// # Errors
/// A sentence when the download fails.
pub async fn check() -> Result<(), String> {
    fwupdmgr(&["refresh", "--force"], REFRESH_DEADLINE)
        .await
        .map(|_| ())
}

/// Install the offered update for device `id`.
///
/// # Errors
/// A sentence when it could not be installed.
pub async fn install(id: &str) -> Result<Installed, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(tr("shell-the-firmware-update-did-not-finish").into());
    }
    fwupdmgr(&["update", id, "--no-reboot-check"], INSTALL_DEADLINE).await?;
    // Read the device again: UEFI and some docks finish at the next restart.
    let restart = read()
        .await
        .ok()
        .and_then(|overview| overview.devices.into_iter().find(|d| d.id == id))
        .is_some_and(|d| d.pending_restart);
    Ok(if restart {
        Installed::RestartNeeded
    } else {
        Installed::Done
    })
}

/// Sample data for the Settings preview.
#[must_use]
pub fn demo() -> Overview {
    let device = |id: &str,
                  name: &str,
                  vendor: &str,
                  version: &str,
                  internal: bool,
                  update: Option<(&str, &str)>| Device {
        id: id.into(),
        name: name.into(),
        vendor: vendor.into(),
        version: version.into(),
        internal,
        updatable: true,
        needs_power: internal,
        pending_restart: false,
        update: update.map(|(version, summary)| Release {
            version: version.into(),
            summary: summary.into(),
            notes: "• Improves battery life while asleep\n• Fixes a rare keyboard wake-up problem"
                .into(),
            size: 22 * 1024 * 1024,
            urgency: 2,
        }),
    };
    Overview {
        available: true,
        devices: vec![
            device(
                "a1",
                "System Firmware",
                "Lenovo",
                "0.1.42",
                true,
                Some(("0.1.45", "UEFI firmware for the ThinkPad X1")),
            ),
            device(
                "b2",
                "Thunderbolt Dock",
                "Lenovo",
                "1.0.11",
                false,
                Some(("1.0.12", "Dock firmware")),
            ),
            device(
                "c3",
                "Samsung SSD 990 PRO 2TB",
                "Samsung",
                "4B2QJXD7",
                true,
                None,
            ),
            device(
                "d4",
                "Unifying Receiver",
                "Logitech",
                "RQR12.11",
                false,
                None,
            ),
        ],
        checked: Some(1_793_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_notes_become_plain_text() {
        let markup =
            "<p>This release:</p><ul><li>Fixes &lt;S3&gt; resume</li><li>Updates the EC</li></ul>";
        assert_eq!(
            plain_notes(markup),
            "This release:\n• Fixes <S3> resume\n• Updates the EC"
        );
        assert_eq!(plain_notes("plain"), "plain");
    }
}
