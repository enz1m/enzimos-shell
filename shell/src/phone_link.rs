//! Phone companion control in the current user's graphical session.

use crate::system_controls::SystemControls;
use enzimos_phone_client::{ControlRequest, Reply};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{fmt, path::Path, time::Duration};

pub use enzimos_phone_client::ControlRequest as Request;

#[derive(Clone, Deserialize, Default)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
}

#[derive(Clone, Deserialize, Default)]
pub struct Device {
    pub device_id: String,
    pub name: String,
    #[serde(default)]
    pub connected: bool,
    pub battery: Option<Battery>,
    #[serde(default)]
    pub permissions: Map<String, Value>,
}

#[derive(Clone, Deserialize, Default)]
pub struct Pending {
    pub name: String,
    pub code: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct Session {
    pub session_id: String,
    pub app_name: String,
    pub device_name: String,
    #[serde(default)]
    pub streaming: bool,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub running: bool,
    pub installed: bool,
    pub devices: Vec<Device>,
    pub pending: Vec<Pending>,
    pub sessions: Vec<Session>,
    pub settings: Map<String, Value>,
    pub schema: Vec<Value>,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhoneSnapshot")
            .field("running", &self.running)
            .field("devices", &self.devices.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Presence {
    pub connected: usize,
    pub paired: usize,
    pub battery: Option<u8>,
}

pub fn available() -> bool {
    service_installed() || enzimos_phone_client::socket_path().is_some_and(|path| path.exists())
}

pub fn service_installed() -> bool {
    Path::new("/usr/lib/systemd/user/enzimos-phone-link.service").is_file()
}

pub fn can_control_service() -> bool {
    service_installed() && crate::desktop_session::mode() == crate::desktop_session::Mode::Installed
}

pub async fn command(request: Request) -> Result<Reply, String> {
    let path =
        enzimos_phone_client::socket_path().ok_or("Phone Link is unavailable in this session")?;
    let reply = enzimos_phone_client::request(&path, &request)
        .await
        .map_err(|error| error.to_string())?;
    if reply.0["ok"] == true {
        Ok(reply)
    } else {
        Err(reply.0["error"]
            .as_str()
            .unwrap_or("The phone request was refused")
            .to_owned())
    }
}

pub async fn snapshot() -> Result<Snapshot, String> {
    let path =
        enzimos_phone_client::socket_path().ok_or("Phone Link is unavailable in this session")?;
    let devices = match enzimos_phone_client::request(&path, &ControlRequest::Devices).await {
        Ok(reply) => reply,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(Snapshot {
                installed: service_installed(),
                ..Snapshot::default()
            });
        }
        Err(error) => return Err(error.to_string()),
    };
    let (settings, pending, sessions) = tokio::try_join!(
        command(Request::SettingsGet),
        command(Request::Pending),
        command(Request::Sessions)
    )?;
    Ok(Snapshot {
        running: true,
        installed: service_installed(),
        devices: decode_list(&devices.0, "devices")?,
        pending: decode_list(&pending.0, "pending")?,
        sessions: decode_list(&sessions.0, "sessions")?,
        settings: settings.0["settings"]
            .as_object()
            .cloned()
            .ok_or("Invalid phone settings")?,
        schema: settings.0["schema"]
            .as_array()
            .cloned()
            .ok_or("Invalid phone settings schema")?,
    })
}

fn decode_list<T: serde::de::DeserializeOwned>(
    reply: &Value,
    name: &str,
) -> Result<Vec<T>, String> {
    if reply["ok"] != true || !reply[name].as_array().is_some_and(|list| list.len() <= 256) {
        return Err("Invalid phone host reply".into());
    }
    serde_json::from_value(reply[name].clone()).map_err(|_| "Invalid phone host data".into())
}

pub async fn presence() -> Option<Presence> {
    let reply = command(Request::Devices).await.ok()?;
    let devices: Vec<Device> = decode_list(&reply.0, "devices").ok()?;
    Some(Presence {
        paired: devices.len(),
        connected: devices.iter().filter(|phone| phone.connected).count(),
        battery: devices
            .iter()
            .find(|phone| phone.connected)
            .and_then(|phone| phone.battery.as_ref())
            .map(|battery| battery.level.min(100)),
    })
}

pub async fn set_running(enabled: bool) -> Result<(), String> {
    if !can_control_service() {
        return Err("Phone Link service changes require an installed enzimOS session".into());
    }
    let controls = SystemControls::from_environment();
    if enabled
        && controls
            .run(
                "systemctl",
                &["--user", "--quiet", "is-active", "enzremote-host.service"],
            )
            .await
            .is_ok()
    {
        return Err("The previous phone host is running. Save its remote work and stop it before enabling Phone Link.".into());
    }
    if !enabled {
        // Off and Restart are confirmed in Settings because both end live work.
        // Retire a legacy checkout unit too, so Off really withdraws phone access.
        let units = controls
            .run(
                "systemctl",
                &[
                    "--user",
                    "list-unit-files",
                    "enzremote-host.service",
                    "--no-legend",
                ],
            )
            .await?;
        if units
            .lines()
            .any(|line| line.split_whitespace().next() == Some("enzremote-host.service"))
        {
            tokio::time::timeout(
                Duration::from_secs(12),
                controls.run(
                    "systemctl",
                    &["--user", "disable", "--now", "enzremote-host.service"],
                ),
            )
            .await
            .map_err(|_| "The previous phone host did not stop".to_owned())??;
        }
    }
    let operation = if enabled { "enable" } else { "disable" };
    tokio::time::timeout(
        Duration::from_secs(12),
        controls.run(
            "systemctl",
            &["--user", operation, "--now", "enzimos-phone-link.service"],
        ),
    )
    .await
    .map_err(|_| "Phone Link service did not respond".to_owned())??;
    Ok(())
}
