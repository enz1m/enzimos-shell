//! Thunderbolt and USB4 devices through bolt (`org.freedesktop.bolt`).
//!
//! On computers whose firmware asks the system to approve each Thunderbolt
//! device (security level "user" or "secure"), a dock or external drive
//! stays switched off until it is allowed. bolt remembers allowed devices
//! and lets them in again when they return; polkit decides who may allow.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.freedesktop.bolt";
const MANAGER: &str = "/org/freedesktop/bolt";
const READ: Duration = Duration::from_secs(10);
/// Allowing includes the administrator dialog.
const CHANGE: Duration = Duration::from_secs(150);

/// Whether the kernel found a Thunderbolt or USB4 controller.
#[must_use]
pub fn present() -> bool {
    std::fs::read_dir("/sys/bus/thunderbolt/devices").is_ok_and(|mut entries| {
        entries.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("domain")))
    })
}

/// How the firmware protects Thunderbolt connections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    /// Every device connects without approval.
    #[default]
    None,
    /// Only DisplayPort; data needs approval through firmware settings.
    DisplayOnly,
    /// Devices need approval here.
    User,
    /// Devices need approval and a stored key.
    Secure,
    /// Only USB; PCIe tunnels are off.
    UsbOnly,
    Unknown,
}

impl Security {
    fn from_text(text: &str) -> Self {
        match text {
            "none" => Self::None,
            "dponly" => Self::DisplayOnly,
            "user" => Self::User,
            "secure" => Self::Secure,
            "usbonly" => Self::UsbOnly,
            _ => Self::Unknown,
        }
    }
}

/// Where a device stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    /// Connected and waiting to be allowed.
    Waiting,
    Authorizing,
    Allowed,
    Failed,
}

/// One Thunderbolt device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub uid: String,
    pub name: String,
    pub vendor: String,
    pub status: Status,
    /// Remembered: allowed again automatically when it returns.
    pub stored: bool,
}

/// What the Thunderbolt page shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overview {
    /// bolt answered.
    pub available: bool,
    pub security: Security,
    /// Devices, without the computer's own controller.
    pub devices: Vec<Device>,
}

type Properties = HashMap<String, OwnedValue>;

fn text(properties: &Properties, key: &str) -> String {
    properties
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect()
}

async fn properties(system: &zbus::Connection, path: &str, interface: &str) -> Option<Properties> {
    let proxy = zbus::fdo::PropertiesProxy::builder(system)
        .destination(SERVICE)
        .ok()?
        .path(path.to_owned())
        .ok()?
        .build()
        .await
        .ok()?;
    let name = zbus::names::InterfaceName::try_from(interface.to_owned()).ok()?;
    tokio::time::timeout(READ, proxy.get_all(name))
        .await
        .ok()?
        .ok()
}

/// Read the controller's security level and the known devices.
pub async fn read() -> Overview {
    let Ok(system) = zbus::Connection::system().await else {
        return Overview::default();
    };
    let Some(manager) = properties(&system, MANAGER, "org.freedesktop.bolt1.Manager").await else {
        return Overview::default();
    };
    let security = Security::from_text(&text(&manager, "SecurityLevel"));
    let Ok(proxy) =
        zbus::Proxy::new(&system, SERVICE, MANAGER, "org.freedesktop.bolt1.Manager").await
    else {
        return Overview {
            available: true,
            security,
            devices: Vec::new(),
        };
    };
    let paths: Vec<OwnedObjectPath> = tokio::time::timeout(READ, proxy.call("ListDevices", &()))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut devices = Vec::new();
    for path in paths.iter().take(64) {
        let Some(device) = properties(&system, path.as_str(), "org.freedesktop.bolt1.Device").await
        else {
            continue;
        };
        if text(&device, "Type") == "host" {
            continue;
        }
        let status = match text(&device, "Status").as_str() {
            "disconnected" => Status::Disconnected,
            "connecting" | "connected" => Status::Waiting,
            "authorizing" => Status::Authorizing,
            "authorized" => Status::Allowed,
            _ => Status::Failed,
        };
        let label = text(&device, "Label");
        devices.push(Device {
            uid: text(&device, "Uid"),
            name: if label.is_empty() {
                text(&device, "Name")
            } else {
                label
            },
            vendor: text(&device, "Vendor"),
            status,
            stored: device
                .get("Stored")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(false),
        });
    }
    devices.sort_by(|a, b| {
        ((a.status == Status::Disconnected), &a.name)
            .cmp(&((b.status == Status::Disconnected), &b.name))
    });
    Overview {
        available: true,
        security,
        devices,
    }
}

fn valid_uid(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= 64 && uid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

async fn manager_call<R>(
    method: &'static str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<(), String>
where
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = zbus::Proxy::new(&system, SERVICE, MANAGER, "org.freedesktop.bolt1.Manager")
        .await
        .map_err(|_| tr("shell-thunderbolt-service-unavailable").to_owned())?;
    match tokio::time::timeout(
        CHANGE,
        proxy.call_with_flags::<_, _, R>(
            method,
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            body,
        ),
    )
    .await
    {
        Err(_) => Err(tr("shell-the-change-took-too-long-nothing-was-confirmed").into()),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(zbus::Error::MethodError(name, _, _)))
            if name.as_str().contains("AccessDenied")
                || name.as_str().contains("NotAuthorized") =>
        {
            Err(tr("shell-an-administrator-did-not-approve-this-change").into())
        }
        Ok(Err(_)) => Err(tr("shell-thunderbolt-device-was-not-allowed").into()),
    }
}

/// Allow device `uid` now and every time it is connected again.
///
/// # Errors
/// A sentence when bolt or the administrator refused.
pub async fn allow(uid: &str) -> Result<(), String> {
    if !valid_uid(uid) {
        return Err(tr("shell-thunderbolt-device-was-not-allowed").into());
    }
    // "auto" (or "iommu" where DMA protection is on): bolt lets the device in
    // again by itself next time.
    manager_call::<OwnedObjectPath>("EnrollDevice", &(uid, "default", "")).await
}

/// Forget device `uid`: it needs approval again next time.
///
/// # Errors
/// A sentence when bolt or the administrator refused.
pub async fn forget(uid: &str) -> Result<(), String> {
    if !valid_uid(uid) {
        return Err(tr("shell-thunderbolt-device-was-not-allowed").into());
    }
    manager_call::<()>("ForgetDevice", &(uid,)).await
}

/// Sample data for the Settings preview.
#[must_use]
pub fn demo() -> Overview {
    let device = |uid: &str, name: &str, vendor: &str, status: Status, stored: bool| Device {
        uid: uid.into(),
        name: name.into(),
        vendor: vendor.into(),
        status,
        stored,
    };
    Overview {
        available: true,
        security: Security::User,
        devices: vec![
            device(
                "00000000-0000-0000-ffff-ffffffffff01",
                "ThinkPad Thunderbolt 4 Dock",
                "Lenovo",
                Status::Allowed,
                true,
            ),
            device(
                "00000000-0000-0000-ffff-ffffffffff02",
                "Thunderbolt 3 NVMe Enclosure",
                "OWC",
                Status::Waiting,
                false,
            ),
            device(
                "00000000-0000-0000-ffff-ffffffffff03",
                "Studio Display",
                "Apple",
                Status::Disconnected,
                true,
            ),
        ],
    }
}
