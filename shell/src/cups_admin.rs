//! Printer administration through cups-pk-helper and polkit.
//! Account-local defaults, queue reads and own-job cancellation use local CUPS tools.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::time::Duration;

const SERVICE: &str = "org.opensuse.CupsPkHelper.Mechanism";

// How long a change may wait, including the administrator dialog.
const DEADLINE: Duration = Duration::from_secs(150);

/// Whether cups-pk-helper is installed (activatable on the system bus).
pub async fn available() -> bool {
    let Ok(system) = zbus::Connection::system().await else {
        return false;
    };
    let Ok(bus) = zbus::fdo::DBusProxy::new(&system).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(SERVICE) else {
        return false;
    };
    bus.list_activatable_names()
        .await
        .is_ok_and(|names| names.iter().any(|known| known.as_str() == SERVICE))
        || bus.name_has_owner(name).await.unwrap_or(false)
}

async fn call<B>(method: &'static str, body: &B) -> Result<(), String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let proxy = zbus::Proxy::new(&system, SERVICE, "/", SERVICE)
        .await
        .map_err(|_| tr("shell-printer-administration-is-not-installed-cups-pk-helper"))?;
    let reply = tokio::time::timeout(
        DEADLINE,
        proxy.call_with_flags::<_, _, String>(
            method,
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            body,
        ),
    )
    .await
    .map_err(|_| tr("shell-the-change-took-too-long-nothing-was-confirmed"))?;
    match reply {
        Ok(error) => match error.unwrap_or_default().trim() {
            "" => Ok(()),
            error => Err(explain(error)),
        },
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str().ends_with("NotPrivileged")
                || name.as_str().contains("AccessDenied") =>
        {
            Err(tr("shell-an-administrator-did-not-approve-this-change").into())
        }
        Err(zbus::Error::MethodError(name, message, _))
            if name.as_str().contains("ServiceUnknown") =>
        {
            let _ = message;
            Err(tr("shell-printer-administration-is-not-installed-cups-pk-helper").into())
        }
        Err(_) => Err(tr("shell-the-printing-system-did-not-accept-the-change").into()),
    }
}

// CUPS's answer, without echoing addresses or queue internals.
fn explain(error: &str) -> String {
    // IPP status names are hyphenated (`client-error-not-authorized`).
    let lower = error.to_lowercase().replace('-', " ");
    if lower.contains("not authorized")
        || lower.contains("forbidden")
        || lower.contains("not privileged")
    {
        tr("shell-an-administrator-did-not-approve-this-change").into()
    } else if lower.contains("not found") || lower.contains("does not exist") {
        tr("shell-that-printer-is-no-longer-available").into()
    } else if lower.contains("everywhere") || lower.contains("driver") || lower.contains("ppd") {
        tr("shell-this-printer-does-not-describe-itself-ipp-everywhere-so-it-cannot-be-added-w")
            .into()
    } else if lower.contains("unable to connect")
        || lower.contains("timed out")
        || lower.contains("connection refused")
    {
        tr("shell-the-printer-did-not-answer-check-that-it-is-on-and-on-this-network").into()
    } else {
        tr("shell-the-printing-system-could-not-complete-the-change").into()
    }
}

/// Adds a queue using IPP Everywhere or a driver name from `lpinfo -m`.
pub async fn add_with_driver(
    queue: &str,
    uri: &str,
    driver: &str,
    description: &str,
) -> Result<(), String> {
    call("PrinterAdd", &(queue, uri, driver, description, "")).await?;
    // A new queue starts stopped in some CUPS versions; it is meant to print.
    call("PrinterSetEnabled", &(queue, true)).await?;
    call("PrinterSetAcceptJobs", &(queue, true, "")).await
}

pub async fn remove(queue: &str) -> Result<(), String> {
    call("PrinterDelete", &(queue,)).await
}

/// Pauses (`false`) or resumes a queue; paused queues keep their jobs.
pub async fn set_enabled(queue: &str, enabled: bool) -> Result<(), String> {
    call("PrinterSetEnabled", &(queue, enabled)).await
}

/// A queue's state from `lpstat -l -p`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    Idle,
    Printing,
    Paused,
    Offline,
    Stopped,
}

/// Parses `lpstat -l -p` into (queue, state).
pub fn queues(text: &str) -> Vec<(String, QueueState)> {
    let mut out: Vec<(String, QueueState)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("printer ") {
            let Some((name, state)) = rest.split_once(' ') else {
                continue;
            };
            let state = if state.starts_with("is idle") {
                QueueState::Idle
            } else if state.starts_with("now printing") {
                QueueState::Printing
            } else if state.starts_with("disabled") {
                QueueState::Paused
            } else {
                QueueState::Stopped
            };
            out.push((name.to_owned(), state));
        } else if let Some(last) = out.last_mut() {
            let lower = line.to_lowercase();
            if lower.contains("offline")
                || lower.contains("unable to connect")
                || lower.contains("printer not responding")
            {
                last.1 = QueueState::Offline;
            }
        }
    }
    out.truncate(32);
    out
}

/// One of this account's jobs that has not finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// `queue-id`, as CUPS names it.
    pub id: String,
    pub queue: String,
    pub size: String,
    pub held: bool,
}

/// Parses `lpstat -W not-completed -l -o -u USER` for that user's own jobs.
pub fn jobs(text: &str, user: &str) -> Vec<Job> {
    let mut out: Vec<Job> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            let mut fields = line.split_whitespace();
            let (Some(id), Some(owner), Some(size)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            // Another account's jobs (and their titles) are not shown.
            if owner != user {
                continue;
            }
            let Some((queue, number)) = id.rsplit_once('-') else {
                continue;
            };
            if number.parse::<u32>().is_err() {
                continue;
            }
            if out.iter().any(|job| job.id == id) {
                continue;
            }
            out.push(Job {
                id: id.to_owned(),
                queue: queue.to_owned(),
                size: human_size(size),
                held: false,
            });
        } else if let Some(last) = out.last_mut()
            && line.to_lowercase().contains("held")
        {
            last.held = true;
        }
    }
    out.truncate(64);
    out
}

fn human_size(bytes: &str) -> String {
    match bytes.parse::<u64>() {
        Ok(b) if b >= 1 << 20 => {
            tr!("shell-mb-5", "arg0" => enzui_shell::i18n::number(format!("{:.1}", b as f64 / f64::from(1 << 20))))
        }
        Ok(b) if b >= 1024 => tr!("shell-kb-3", "arg0" => format!("{:}", b / 1024)),
        Ok(b) => tr!("shell-bytes-2", "arg0" => format!("{:}", b)),
        Err(_) => String::new(),
    }
}

/// Whether an advertised printer address is a plain IPP(S) URI: no
/// credentials, query or other scheme, a sane host, port and path.
#[must_use]
/// A device address from CUPS's own backends for an older printer: USB,
/// JetDirect (`socket://`) or LPD.
pub fn valid_device_uri(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return false;
    };
    matches!(scheme, "usb" | "socket" | "lpd")
        && !rest.is_empty()
        && uri.len() <= 512
        && !uri.contains(['\\', ' ', '\'', '"'])
        && !uri.chars().any(char::is_control)
}

pub fn valid_uri(uri: &str) -> bool {
    let Some(rest) = uri
        .strip_prefix("ipp://")
        .or_else(|| uri.strip_prefix("ipps://"))
    else {
        return false;
    };
    if uri.len() > 256
        || rest.contains(['@', '?', '#', '\\', ' '])
        || rest.chars().any(char::is_control)
    {
        return false;
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host_ok = |host: &str| {
        !host.is_empty()
            && host.len() <= 253
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    };
    let valid = if let Some(v6) = authority.strip_prefix('[') {
        match v6.split_once(']') {
            Some((address, port)) => {
                address.parse::<std::net::Ipv6Addr>().is_ok()
                    && (port.is_empty()
                        || port
                            .strip_prefix(':')
                            .is_some_and(|p| p.parse::<u16>().is_ok()))
            }
            None => false,
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => host_ok(host) && port.parse::<u16>().is_ok(),
            None => host_ok(authority),
        }
    };
    valid
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~%".contains(c))
}

/// An advertised name made safe to show and to derive a queue from.
#[must_use]
pub fn display_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(96)
        .collect();
    (!name.is_empty()).then_some(name)
}
