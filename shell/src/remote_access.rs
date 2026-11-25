//! Review-bound OS remote-access client; listener policy, firewall and authorization remain
//! OS-owned.

use base64::Engine;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

const SERVICE: &str = "org.enzimos.Remote1";
const PATH: &str = "/org/enzimos/Remote1";
const APPROVAL_MS: u64 = 120_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub ssh: bool,
    pub sftp: bool,
    pub desktop_sharing: bool,
    /// The OS can start approved sharing by itself after sign-in.
    pub desktop_unattended: bool,
}

/// Where a server listens and whom it accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub address: String,
    pub port: u16,
    pub networks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKey {
    pub key: String,
    pub fingerprint: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshSetup {
    pub enabled: bool,
    /// Listens from boot, before anyone signs in.
    pub autostart: bool,
    /// Allow the ordinary account’s password as an explicit alternative to keys.
    pub allow_password: bool,
    /// Explicit local addresses, including the primary endpoint.
    pub addresses: Vec<String>,
    pub endpoint: Endpoint,
    pub keys: Vec<SshKey>,
    pub forwarding: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssh {
    pub setup: Option<SshSetup>,
    pub listening: bool,
    /// The remote address of each connected session.
    pub sessions: Vec<String>,
    /// `SHA256:…`, as `ssh` shows it on first connection.
    pub host_key: Option<String>,
    /// Another account set SSH up; only it can change it.
    pub other_account: bool,
    /// Why an approved setup is not running (the OS's failure detail).
    pub failure: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharingSetup {
    pub endpoint: Endpoint,
    pub view_only: bool,
}

/// Approved sharing the OS starts by itself after sign-in or unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unattended {
    pub endpoint: Endpoint,
    /// The reviewed network interface it runs on.
    pub interface: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sharing {
    pub setup: Option<SharingSetup>,
    pub unattended: Option<Unattended>,
    /// The OS verified the encrypted listener, not only a running unit.
    pub listening: bool,
    pub state: String,
    pub cleanup_pending: bool,
    /// Authenticated clients, or unavailable when observation cannot be verified.
    pub viewers: Option<usize>,
}

/// An address this computer has now, with the network it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub address: String,
    pub interface: String,
    pub network: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub capabilities: Capabilities,
    pub ssh: Result<Ssh, String>,
    pub sharing: Result<Sharing, String>,
    pub addresses: Vec<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Ssh(SshRequest),
    DisconnectSsh,
    Share(SharingRequest),
    StopSharing,
    /// Turns unattended sharing on or off.
    Unattended(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshRequest {
    pub enabled: bool,
    pub autostart: bool,
    /// Allow the ordinary account’s password as an explicit alternative to keys.
    pub allow_password: bool,
    /// Explicit local addresses, including the primary endpoint.
    pub addresses: Vec<String>,
    pub endpoint: Endpoint,
    /// OpenSSH public key lines.
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharingRequest {
    pub endpoint: Endpoint,
    pub view_only: bool,
}

/// The OS's plan for one change and what it will do, in plain words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub change: Change,
    pub plan_sha256: String,
    pub consequences: Vec<String>,
}

impl Change {
    fn configuration(&self) -> Option<Value> {
        match self {
            Self::Ssh(request) => Some(json!({
                "schema": 1,
                "enabled": request.enabled,
                "autostart": request.autostart,
                "allow_password": request.allow_password,
                "listen_address": request.endpoint.address,
                "listen_addresses": request.addresses,
                "port": request.endpoint.port,
                "allowed_networks": request.endpoint.networks,
                "public_keys": request.keys,
                "allow_local_forwarding": false,
            })),
            Self::Share(request) => Some(json!({
                "schema": 1,
                "enabled": true,
                "mode": "session-sharing",
                "listen_address": request.endpoint.address,
                "port": request.endpoint.port,
                "allowed_networks": request.endpoint.networks,
                "view_only": request.view_only,
                "max_fps": 15,
                "output": null,
            })),
            Self::DisconnectSsh | Self::StopSharing | Self::Unattended(_) => None,
        }
    }
    fn methods(&self) -> (&'static str, &'static str) {
        match self {
            Self::Ssh(_) => ("Review", "Apply"),
            Self::DisconnectSsh => ("ReviewDisconnect", "Disconnect"),
            Self::Share(_) => ("ReviewDesktop", "ApplyDesktop"),
            Self::StopSharing => ("ReviewStopDesktop", "StopDesktop"),
            Self::Unattended(_) => ("ReviewDesktopUnattended", "SetDesktopUnattended"),
        }
    }
    fn review_body(&self) -> Value {
        match (self, self.configuration()) {
            (Self::Unattended(on), _) => json!({"schema": 1, "unattended": on}),
            (_, Some(configuration)) => json!({"schema": 1, "configuration": configuration}),
            (_, None) => json!({"schema": 1}),
        }
    }
    fn apply_body(&self, plan_sha256: &str) -> Value {
        let mut body = json!({"schema": 1, "plan_sha256": plan_sha256, "allow_interaction": true, "valid_for_ms": APPROVAL_MS});
        if let Some(configuration) = self.configuration() {
            body["configuration"] = configuration;
        }
        if let Self::Unattended(on) = self {
            body["unattended"] = json!(on);
        }
        body
    }
}

fn explain(code: &str) -> String {
    match code {
        "not-authorized" => tr("shell-an-administrator-did-not-approve-this-change").into(),
        "request-cancelled" => tr("shell-the-approval-was-cancelled-nothing-was-changed").into(),
        "review-stale" | "caller-session-changed" => {
            tr("shell-something-changed-since-the-review-review-again").into()
        }
        "request-expired" | "request-deadline-or-interaction" => {
            tr("shell-the-approval-took-too-long-nothing-was-changed").into()
        }
        "active-local-graphical-session-required" | "desktop-wayland-session-required" => {
            tr("shell-remote-access-can-only-be-changed-from-this-computer-s-own-screen").into()
        }
        "ordinary-account-required" => {
            tr("shell-remote-access-is-for-ordinary-accounts-not-root").into()
        }
        "listen-address-unavailable" => {
            tr("shell-that-address-is-no-longer-on-this-computer-choose-another").into()
        }
        "network-invalid" | "network-bound" => {
            tr("shell-enter-networks-like-192-168-1-0-24-separated-by-commas").into()
        }
        "network-too-broad-or-wrong-family" => {
            tr("shell-a-network-is-too-broad-or-does-not-match-the-address-ipv4-8-or-narrower")
                .into()
        }
        "network-duplicate" => tr("shell-a-network-is-listed-twice").into(),
        "ssh-listen-port" => tr("shell-the-ssh-port-must-be-between-22-and-65535").into(),
        "desktop-listen-port" => tr("shell-the-sharing-port-must-be-between-1024-and-65535").into(),
        "remote-owner-mismatch" | "another-account-has-remote-connections" => {
            tr("shell-another-account-set-up-remote-access-on-this-computer").into()
        }
        "remote-configuration-not-activated" | "remote-unit-transition-unconfirmed" => {
            tr!("shell-enzimos-could-not-start-the-server-so-it-is-not-running-nothing-else-w", "arg0" => code.to_owned())
        }
        "remote-provider-unavailable" | "desktop-provider-unavailable" => {
            tr("shell-remote-service-unavailable").into()
        }
        "desktop-unattended-requires-sharing" => {
            tr("shell-unattended-access-needs-running-sharing").into()
        }
        "desktop-unattended-network-identity-unavailable" => {
            tr("shell-unattended-access-needs-a-managed-network-connection").into()
        }
        "desktop-unattended-provider-unavailable" => {
            tr("shell-unattended-access-is-not-available-for-this-kind-of-sharing").into()
        }
        "desktop-operation-in-progress" => {
            tr("shell-another-sharing-change-is-in-progress-try-again").into()
        }
        code if code.starts_with("public-key") => {
            tr("shell-a-key-is-not-a-valid-openssh-public-key-one-line-starting-with-ssh-ed25519-e")
                .into()
        }
        code if code.starts_with("desktop-session")
            || code.starts_with("desktop-wayland")
            || code.starts_with("desktop-owned") =>
        {
            tr("shell-this-desktop-session-could-not-be-verified-for-sharing-sign-out-and-back-in")
                .into()
        }
        _ => {
            tr!("shell-the-remote-access-service-refused-the-change", "arg0" => code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(48).collect::<String>())
        }
    }
}

async fn call(method: &str, body: Option<&Value>, patience: Duration) -> Result<Value, String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-the-system-bus-is-not-available"))?;
    let proxy = zbus::Proxy::new(&connection, SERVICE, PATH, SERVICE)
        .await
        .map_err(|_| tr("shell-remote-access-is-not-installed"))?;
    let flags = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
    let reply = match body {
        Some(body) => {
            tokio::time::timeout(
                patience,
                proxy.call_with_flags::<_, _, String>(method, flags, &(body.to_string(),)),
            )
            .await
        }
        None => {
            tokio::time::timeout(
                patience,
                proxy.call_with_flags::<_, _, String>(method, flags, &()),
            )
            .await
        }
    }
    .map_err(|_| {
        tr("shell-the-remote-access-service-did-not-answer-check-its-state-before-trying-again")
    })?;
    match reply {
        Ok(Some(text)) if text.len() <= 256 * 1024 => serde_json::from_str(&text)
            .map_err(|_| tr("shell-invalid-reply-from-the-remote-access-service").into()),
        Ok(_) => Err(tr("shell-invalid-reply-from-the-remote-access-service").into()),
        Err(zbus::Error::MethodError(name, message, _))
            if name.as_str() == "org.enzimos.Remote1.Refused" =>
        {
            Err(explain(message.as_deref().unwrap_or_default()))
        }
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().contains("ServiceUnknown") => {
            Err(tr("shell-remote-access-is-not-installed").into())
        }
        Err(_) => Err(tr(
            "shell-the-remote-access-service-failed-check-its-state-before-trying-again",
        )
        .into()),
    }
}

/// Detects the service without starting it.
pub async fn available() -> bool {
    let Ok(system) = zbus::Connection::system().await else {
        return false;
    };
    let Ok(bus) = zbus::fdo::DBusProxy::new(&system).await else {
        return false;
    };
    bus.list_activatable_names()
        .await
        .is_ok_and(|names| names.iter().any(|name| name.as_str() == SERVICE))
}

fn endpoint(value: &Value) -> Option<Endpoint> {
    Some(Endpoint {
        address: value["listen_address"].as_str()?.to_owned(),
        port: u16::try_from(value["port"].as_u64()?).ok()?,
        networks: value["allowed_networks"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .take(16)
            .map(str::to_owned)
            .collect(),
    })
}

fn ssh_setup(value: &Value) -> Option<SshSetup> {
    Some(SshSetup {
        enabled: value["enabled"].as_bool()?,
        autostart: value["autostart"].as_bool().unwrap_or(false),
        allow_password: value["allow_password"].as_bool().unwrap_or(false),
        addresses: match value.get("listen_addresses") {
            Some(value) => {
                let rows = value.as_array()?;
                if !(1..=8).contains(&rows.len()) {
                    return None;
                }
                rows.iter()
                    .map(|row| row.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?
            }
            None => vec![value["listen_address"].as_str()?.to_owned()],
        },
        endpoint: endpoint(value)?,
        keys: value["public_keys"]
            .as_array()?
            .iter()
            .take(64)
            .filter_map(|row| {
                Some(SshKey {
                    key: row["key"].as_str()?.to_owned(),
                    fingerprint: row["fingerprint"].as_str()?.chars().take(80).collect(),
                    kind: row["type"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(40)
                        .collect(),
                })
            })
            .collect(),
        forwarding: value["allow_local_forwarding"].as_bool().unwrap_or(false),
    })
}

pub fn parse_ssh(value: &Value) -> Ssh {
    // The approved setup comes inside the OS's record of its last change.
    let record = &value["configuration"];
    let configuration = if record["configuration"].is_object() {
        &record["configuration"]
    } else {
        record
    };
    Ssh {
        setup: ssh_setup(configuration),
        failure: (record["phase"] == "failed").then(|| {
            record["detail"]
                .as_str()
                .unwrap_or("unknown")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .take(64)
                .collect()
        }),
        listening: value["listening"].as_bool().unwrap_or(false),
        sessions: value["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .take(64)
            .map(
                |row| match (row["remote_address"].as_str(), row["remote_port"].as_u64()) {
                    (Some(address), Some(port)) if address.contains(':') => {
                        format!("[{address}]:{port}")
                    }
                    (Some(address), Some(port)) => format!("{address}:{port}"),
                    (Some(address), None) => address.to_owned(),
                    _ => tr("shell-unknown-address").to_owned(),
                },
            )
            .collect(),
        host_key: value["host_public_key"].as_str().and_then(key_fingerprint),
        other_account: value["owned_by_another_account"].as_bool().unwrap_or(false),
    }
}

pub fn parse_sharing(value: &Value) -> Sharing {
    let configuration = &value["configuration"];
    let unattended = &value["unattended"];
    Sharing {
        setup: endpoint(configuration).map(|endpoint| SharingSetup {
            endpoint,
            view_only: configuration["view_only"].as_bool().unwrap_or(true),
        }),
        unattended: endpoint(&unattended["configuration"]).map(|endpoint| Unattended {
            endpoint,
            interface: unattended["interface"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(32)
                .collect(),
        }),
        listening: value["listening"].as_bool().unwrap_or(false),
        state: value["state"]
            .as_str()
            .unwrap_or("unknown")
            .chars()
            .take(32)
            .collect(),
        cleanup_pending: value["cleanup_pending"].as_bool().unwrap_or(false),
        viewers: viewer_count(&value["viewers"]),
    }
}

fn viewer_count(value: &Value) -> Option<usize> {
    if value["status"].as_str()? != "available" {
        return None;
    }
    let count = usize::try_from(value["connected_count"].as_u64()?).ok()?;
    let clients = value["clients"].as_array()?;
    (count <= 64 && clients.len() == count).then_some(count)
}

/// Invalidates visible Remote Settings on provider changes, loss and bounded refresh.
/// QueryDesktop uses this same connection so directed signals reach the subscriber.
pub fn subscription() -> iced::Subscription<bool> {
    use iced::futures::{SinkExt, StreamExt};
    iced::Subscription::run(|| {
        iced::stream::channel(1, async move |mut output| {
            loop {
                let watched = async {
                    let connection = zbus::Connection::system().await.ok()?;
                    let proxy = zbus::Proxy::new(&connection, SERVICE, PATH, SERVICE)
                        .await
                        .ok()?;
                    let mut owners = proxy.receive_owner_changed().await.ok()?;
                    let mut changes = proxy.receive_signal("DesktopChanged").await.ok()?;
                    loop {
                        // Register before waiting; the service may have exited since the last query.
                        let _ = tokio::time::timeout(
                            Duration::from_secs(10),
                            proxy.call::<_, _, String>("QueryDesktop", &()),
                        )
                        .await;
                        if output.send(false).await.is_err() {
                            return None;
                        }
                        tokio::select! {
                            signal = changes.next() => { signal?; }
                            _ = owners.next() => { let _ = output.send(true).await; return None; }
                            () = tokio::time::sleep(Duration::from_secs(10)) => {}
                        }
                    }
                };
                let _: Option<()> = watched.await;
                if output.send(true).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        })
    })
}

/// An OpenSSH key line's fingerprint, as `ssh-keygen -l` prints it.
pub fn key_fingerprint(line: &str) -> Option<String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(line.split_whitespace().nth(1)?)
        .ok()?;
    let digest = base64::engine::general_purpose::STANDARD_NO_PAD.encode(Sha256::digest(&blob));
    Some(format!("SHA256:{digest}"))
}

/// The addresses a server may listen on: global unicast ones, with their network.
pub fn parse_addresses(json_text: &str) -> Vec<Address> {
    let Ok(Value::Array(links)) = serde_json::from_str::<Value>(json_text) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for link in links.iter().take(64) {
        let interface = link["ifname"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(32)
            .collect::<String>();
        for info in link["addr_info"].as_array().into_iter().flatten().take(32) {
            let (Some(local), Some(prefix)) = (info["local"].as_str(), info["prefixlen"].as_u64())
            else {
                continue;
            };
            if info["scope"].as_str() != Some("global") {
                continue;
            }
            if let Some(network) = network_of(local, prefix) {
                found.push(Address {
                    address: local.to_owned(),
                    interface: interface.clone(),
                    network,
                });
            }
        }
    }
    found
}

fn network_of(address: &str, prefix: u64) -> Option<String> {
    match address.parse::<std::net::IpAddr>().ok()? {
        std::net::IpAddr::V4(ip) if (8..=32).contains(&prefix) => {
            let mask = u32::MAX.checked_shl(32 - prefix as u32).unwrap_or(0);
            Some(format!(
                "{}/{prefix}",
                std::net::Ipv4Addr::from(u32::from(ip) & mask)
            ))
        }
        std::net::IpAddr::V6(ip) if (32..=128).contains(&prefix) => {
            let mask = u128::MAX.checked_shl(128 - prefix as u32).unwrap_or(0);
            Some(format!(
                "{}/{prefix}",
                std::net::Ipv6Addr::from(u128::from(ip) & mask)
            ))
        }
        _ => None,
    }
}

/// Splits typed networks; the OS checks each one.
pub fn networks(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .take(16)
        .map(str::to_owned)
        .collect()
}

async fn addresses() -> Vec<Address> {
    let output = tokio::process::Command::new("ip")
        .args(["-j", "address", "show"])
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(Duration::from_secs(5), output).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_addresses(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

/// What remote access offers and how it is set up now. `None`: this system
/// has no remote access service.
fn listener_reply(value: Value) -> Result<Value, String> {
    if value["schema"] != 1
        || !value["listening"].is_boolean()
        || !(value["configuration"].is_null() || value["configuration"].is_object())
    {
        return Err(tr("shell-invalid-reply-from-the-remote-access-service").into());
    }
    Ok(value)
}

pub async fn read() -> Option<Overview> {
    if !available().await {
        return None;
    }
    let quick = Duration::from_secs(20);
    let capabilities = match call("GetCapabilities", None, quick).await {
        Ok(caps) => Capabilities {
            ssh: caps["ssh"].as_bool().unwrap_or(false),
            sftp: caps["sftp"].as_bool().unwrap_or(false),
            desktop_sharing: caps["desktop_sharing"].as_bool().unwrap_or(false),
            desktop_unattended: caps["desktop_unattended"].as_bool().unwrap_or(false),
        },
        Err(error) => {
            return Some(Overview {
                capabilities: Capabilities::default(),
                ssh: Err(error.clone()),
                sharing: Err(error),
                addresses: Vec::new(),
            });
        }
    };
    let ssh = call("Query", None, quick)
        .await
        .and_then(listener_reply)
        .map(|value| parse_ssh(&value));
    let sharing = if capabilities.desktop_sharing {
        call("QueryDesktop", None, quick)
            .await
            .and_then(listener_reply)
            .map(|value| parse_sharing(&value))
    } else {
        Err(tr("shell-desktop-sharing-is-not-available-on-this-system").into())
    };
    Some(Overview {
        capabilities,
        ssh,
        sharing,
        addresses: addresses().await,
    })
}

pub async fn review(change: Change) -> Result<Review, String> {
    let (method, _) = change.methods();
    let reply = call(method, Some(&change.review_body()), Duration::from_secs(30)).await?;
    let plan_sha256 = reply["plan_sha256"]
        .as_str()
        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| tr("shell-invalid-review-from-the-remote-access-service"))?
        .to_owned();
    let consequences = consequences(&change, &reply["plan"]);
    Ok(Review {
        change,
        plan_sha256,
        consequences,
    })
}

/// Applies a reviewed change; the OS asks an administrator first.
pub async fn apply(review: Review) -> Result<(), String> {
    let (_, method) = review.change.methods();
    call(
        method,
        Some(&review.change.apply_body(&review.plan_sha256)),
        Duration::from_millis(APPROVAL_MS + 30_000),
    )
    .await
    .map(drop)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Plain sentences for what a plan does, from the plan itself where it says.
pub fn consequences(change: &Change, plan: &Value) -> Vec<String> {
    let reviewed = endpoint(&plan["configuration"]);
    match change {
        Change::Ssh(request) if request.enabled => {
            let at = reviewed.unwrap_or_else(|| request.endpoint.clone());
            let interface = plan["interface"]
                .as_str()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default();
            vec![
                format!(
                    "Listens on {} port {}{interface}",
                    if request.addresses.is_empty() {
                        at.address.clone()
                    } else {
                        request.addresses.join(", ")
                    },
                    at.port
                ),
                format!("Accepts connections only from {}", at.networks.join(", ")),
                if request.allow_password {
                    tr("shell-ssh-password-review").into()
                } else {
                    tr!("shell-ssh-key-review", "count" => request.keys.len())
                },
                if request.autostart {
                    tr("shell-starts-with-the-computer-before-anyone-signs-in").into()
                } else {
                    tr("shell-stays-off-after-the-computer-restarts").into()
                },
                tr("shell-remote-firewall-review").into(),
                tr("shell-files-can-be-copied-with-sftp-as-this-account").into(),
                tr("shell-sessions-already-connected-stay-connected").into(),
            ]
        }
        Change::Ssh(_) => vec![
            tr("shell-stops-listening-for-new-connections").into(),
            tr("shell-sessions-already-connected-stay-until-you-disconnect-them").into(),
        ],
        Change::DisconnectSsh => {
            let sessions = plan["sessions"].as_array().map_or(0, Vec::len);
            vec![
                format!(
                    "Ends {}",
                    plural(
                        sessions,
                        tr("shell-connected-session"),
                        tr("shell-connected-sessions")
                    )
                ),
                tr("shell-ssh-keeps-listening-for-new-connections").into(),
            ]
        }
        Change::Share(request) => {
            let at = reviewed.unwrap_or_else(|| request.endpoint.clone());
            let mut lines: Vec<String> = vec![
                tr("shell-remote-firewall-review").into(),
                format!("Shares this desktop on {} port {}", at.address, at.port),
                format!("Accepts viewers only from {}", at.networks.join(", ")),
                if request.view_only { tr("shell-viewers-can-only-watch").into() } else { tr("shell-viewers-can-use-the-mouse-and-keyboard").into() },
                tr("shell-encrypted-viewers-need-the-sharing-password-and-should-check-the-certificate").into(),
                tr("shell-ends-when-you-lock-the-screen-sign-out-or-switch-away").into(),
                tr("shell-viewers-connected-now-are-disconnected-ssh-is-not-affected").into(),
            ];
            if plan["autostart"] == true {
                lines.push(tr("shell-unattended-access-stays-on-with-these-settings").into());
            }
            lines
        }
        Change::StopSharing => {
            let mut lines = vec![
                tr("shell-disconnects-every-viewer-and-stops-sharing").into(),
                tr("shell-ssh-sessions-stay-connected").into(),
            ];
            if plan["unattended_turned_off"] == true {
                lines.push(tr("shell-also-turns-off-unattended-access").into());
            }
            lines
        }
        Change::Unattended(true) => {
            let network = match plan["network"][0]["interface"].as_str() {
                Some(name) => {
                    tr!("shell-unattended-review-network", "arg0" => name.chars().filter(|c| !c.is_control()).take(32).collect::<String>())
                }
                None => tr("shell-unattended-review-approved-network").into(),
            };
            vec![
                tr("shell-unattended-review-starts-by-itself").into(),
                tr("shell-unattended-review-never-before-sign-in").into(),
                network,
                tr("shell-unattended-review-same-access").into(),
                tr("shell-unattended-review-indicator").into(),
            ]
        }
        Change::Unattended(false) => vec![
            tr("shell-unattended-off-review-current-sharing-continues").into(),
            tr("shell-unattended-off-review-final").into(),
        ],
    }
}

/// The sharing password and certificate fingerprint viewers need, from the
/// owner-private files the sharing service creates on first use.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub password: String,
    /// Uppercase colon-separated SHA-256 of the certificate.
    pub fingerprint: String,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

pub fn credentials() -> Option<Credentials> {
    use std::os::unix::fs::MetadataExt;
    let directory = std::path::Path::new(&std::env::var_os("HOME")?)
        .join(".local/share/enzimos/remote-desktop");
    let uid = rustix::process::getuid().as_raw();
    let meta = std::fs::symlink_metadata(&directory).ok()?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o777 != 0o700 {
        return None;
    }
    let read = |name: &str| -> Option<Vec<u8>> {
        let path = directory.join(name);
        let meta = std::fs::symlink_metadata(&path).ok()?;
        (meta.is_file() && meta.uid() == uid && meta.mode() & 0o077 == 0 && meta.len() <= 16384)
            .then(|| std::fs::read(&path).ok())
            .flatten()
    };
    let credential: Value = serde_json::from_slice(&read("credential.json")?).ok()?;
    let password = credential["password"]
        .as_str()
        .filter(|p| {
            (32..=128).contains(&p.len())
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        })?
        .to_owned();
    Some(Credentials {
        password,
        fingerprint: certificate_fingerprint(&read("certificate.pem")?)?,
    })
}

pub fn certificate_fingerprint(pem: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(pem).ok()?;
    let body: String = text
        .lines()
        .skip_while(|line| !line.starts_with("-----BEGIN CERTIFICATE-----"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .ok()?;
    Some(
        Sha256::digest(&der)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// Public keys in `~/.ssh` to offer when SSH is first set up.
pub fn own_keys() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(std::path::Path::new(&home).join(".ssh")) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".pub"))
        .filter_map(|entry| {
            std::fs::read_to_string(entry.path())
                .ok()
                .filter(|text| text.len() <= 16384)
        })
        .filter_map(|text| text.lines().next().map(str::trim).map(str::to_owned))
        .filter(|line| key_fingerprint(line).is_some())
        .take(8)
        .collect();
    keys.sort();
    keys
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharingGrant {
    ViewOnly,
    Control,
}

/// The OS's protected per-account capture grant, for a sharing-enabled warning.
/// This is permission to share, not an observation of connected viewers.
/// The root-owned folder that holds desktop-sharing grants.
pub const SHARING_GRANT_DIRECTORY: &str = "/run/enzimos-compositor-grants";

pub fn sharing_grant() -> Option<SharingGrant> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let directory = std::path::Path::new(SHARING_GRANT_DIRECTORY);
    let parent = std::fs::symlink_metadata(directory).ok()?;
    if !parent.is_dir() || parent.uid() != 0 || parent.mode() & 0o022 != 0 {
        return None;
    }
    let uid = rustix::process::getuid().as_raw();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join(format!("enzimos-desktop-sharing@{uid}.grant")))
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file()
        || meta.uid() != 0
        || meta.mode() & 0o022 != 0
        || meta.nlink() != 1
        || meta.len() > 4096
    {
        return None;
    }
    let mut body = String::new();
    file.take(4097).read_to_string(&mut body).ok()?;
    parse_sharing_grant(&body, uid)
}

fn parse_sharing_grant(body: &str, uid: u32) -> Option<SharingGrant> {
    if body.len() > 4096 {
        return None;
    }
    let mut fields = std::collections::BTreeMap::new();
    for line in body.lines() {
        let (key, value) = line.split_once('=')?;
        if fields.insert(key, value).is_some() {
            return None;
        }
    }
    if fields.get("uid")?.parse::<u32>().ok()? != uid {
        return None;
    }
    let expected = format!("/system.slice/enzimos-desktop-sharing@{uid}.service");
    let cgroup = *fields.get("cgroup")?;
    if cgroup != expected
        && !cgroup
            .strip_prefix(&expected)
            .is_some_and(|tail| tail.starts_with('/'))
    {
        return None;
    }
    match *fields.get("kinds")? {
        "capture" => Some(SharingGrant::ViewOnly),
        "capture,input" => Some(SharingGrant::Control),
        _ => None,
    }
}

#[cfg(test)]
mod sharing_grant_tests {
    use super::*;
    #[test]
    fn remote_access_behaviour() {
        // Viewer observation distinguishes zero from unknown and inconsistent.
        {
            assert!(
                listener_reply(json!({"schema":1,"listening":false,"configuration":null})).is_ok()
            );
            assert!(listener_reply(json!({"schema":1,"configuration":null})).is_err());
            assert!(
                listener_reply(json!({"schema":1,"listening":"false","configuration":null}))
                    .is_err()
            );

            let available = |n: usize| json!({"status":"available","connected_count":n,"clients":vec![json!({"id":1}); n]});
            assert_eq!(
                parse_sharing(&json!({"viewers":available(0)})).viewers,
                Some(0)
            );
            assert_eq!(
                parse_sharing(&json!({"viewers":available(1)})).viewers,
                Some(1)
            );
            for invalid in [
                Value::Null,
                json!({"status":"unavailable","connected_count":null,"clients":null}),
                json!({"status":"available","connected_count":1,"clients":[]}),
                available(65),
            ] {
                assert_eq!(parse_sharing(&json!({"viewers":invalid})).viewers, None);
            }
        }
        // Unattended access is read from its own field and toggled by its own methods.
        {
            let configuration = json!({"listen_address":"192.168.1.37","port":5900,"allowed_networks":["192.168.1.0/24"],"view_only":true});
            let on = parse_sharing(
                &json!({"configuration":null,"unattended":{"configuration":configuration,"interface":"enp1s0\n"}}),
            );
            assert!(on.setup.is_none());
            let unattended = on.unattended.unwrap();
            assert_eq!(unattended.endpoint.address, "192.168.1.37");
            assert_eq!(unattended.interface, "enp1s0");
            for off in [
                json!({}),
                json!({"unattended":null}),
                json!({"unattended":{}}),
            ] {
                assert_eq!(parse_sharing(&off).unattended, None);
            }
            let change = Change::Unattended(true);
            assert_eq!(
                change.methods(),
                ("ReviewDesktopUnattended", "SetDesktopUnattended")
            );
            assert_eq!(change.review_body(), json!({"schema":1,"unattended":true}));
            let digest = "a".repeat(64);
            assert_eq!(
                Change::Unattended(false).apply_body(&digest),
                json!({"schema":1,"unattended":false,"plan_sha256":digest,"allow_interaction":true,"valid_for_ms":APPROVAL_MS})
            );
            let plan = json!({"network":[{"interface":"enp1s0"}]});
            assert!(
                consequences(&change, &plan)
                    .iter()
                    .any(|line| line.contains("enp1s0"))
            );
            let stop = |turned_off: bool| {
                consequences(
                    &Change::StopSharing,
                    &json!({"unattended_turned_off":turned_off}),
                )
                .len()
            };
            assert_eq!(stop(true), stop(false) + 1);
        }
        // Sharing warning is bound to the account unit and capture permission.
        {
            let grant = "uid=1000\ncgroup=/system.slice/enzimos-desktop-sharing@1000.service\nkinds=capture\n";
            assert_eq!(
                parse_sharing_grant(grant, 1000),
                Some(SharingGrant::ViewOnly)
            );
            assert_eq!(
                parse_sharing_grant(&grant.replace("kinds=capture", "kinds=capture,input"), 1000),
                Some(SharingGrant::Control)
            );
            assert_eq!(parse_sharing_grant(grant, 1001), None);
            for invalid in [
                grant.replace(".service", ".service-foreign"),
                grant.replace("kinds=capture", "kinds=input"),
                format!("{grant}uid=1000\n"),
                "x".repeat(4097),
            ] {
                assert_eq!(parse_sharing_grant(&invalid, 1000), None);
            }
        }
    }
}
