use super::*;
use crate::system_controls::{Action, Failure};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::time::Duration;

// polkit may ask an administrator for their password first.
const ADMIN_SECONDS: u64 = 120;

impl SystemSettings {
    pub async fn apply(&self, request: &Request) -> Result<(), String> {
        if !self.allows(request) {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            demo::apply(request);
            return Ok(());
        }
        let run = &self.controls;
        match request {
            Request::AgentRevokeIdentity(_)
            | Request::AgentRevokePermission { .. }
            | Request::AgentClearActivity => {
                let request = request.clone();
                tokio::task::spawn_blocking(move || match request {
                    Request::AgentRevokeIdentity(id) => crate::agent_center::revoke_credential(&id),
                    Request::AgentRevokePermission {
                        agent,
                        app,
                        function,
                    } => crate::agent_center::revoke_permission(&agent, &app, &function),
                    _ => crate::agent_center::clear_activity(),
                })
                .await
                .map_err(|_| tr("shell-the-mcp-center-did-not-answer").to_owned())?
            }
            Request::DefaultApp { kind, id } => {
                let (kind, id) = (*kind, id.clone());
                tokio::task::spawn_blocking(move || crate::default_apps::set(kind, &id))
                    .await
                    .map_err(|_| tr("shell-the-default-could-not-be-changed").to_owned())?
            }
            Request::StartupApp {
                id,
                review,
                enabled,
                adding,
            } => {
                let (id, review, enabled, adding) = (id.clone(), review.clone(), *enabled, *adding);
                tokio::task::spawn_blocking(move || {
                    crate::startup_apps::Store::from_environment()?
                        .set(&id, &review, enabled, adding)
                })
                .await
                .map_err(|_| tr("shell-startup-settings-save-stopped").to_owned())?
            }
            Request::AppPermission { app, kind, allowed } => {
                crate::app_permissions::set(app, *kind, *allowed).await
            }
            Request::Display(layout) => {
                let layout = layout.clone();
                // A new scale lays the desktop's bar out again: the shell hides
                // it before the output changes and shows it once it is placed.
                let rescaling = {
                    let layout = layout.clone();
                    tokio::task::spawn_blocking(move || {
                        display::rescales(&display::read(), &layout)
                    })
                    .await
                    .unwrap_or(false)
                };
                let shell = rescaling
                    .then(crate::compositor::Compositor::from_environment)
                    .and_then(Result::ok);
                if let Some(shell) = &shell {
                    shell.hide_chrome_for_rescale().await;
                }
                let applied = tokio::task::spawn_blocking(move || display::apply(&layout))
                    .await
                    .map_err(|_| tr("shell-display-change-stopped").to_owned());
                if let Some(shell) = &shell {
                    shell.rescale_finished().await;
                }
                applied?
            }
            Request::GameModeOption(key, value) => {
                let (key, value) = (*key, *value);
                tokio::task::spawn_blocking(move || crate::game_mode::preferences::set(key, value))
                    .await
                    .map_err(|_| tr("shell-gamemode-configuration-save-stopped").to_owned())?
            }
            Request::OutputVolume(_)
            | Request::OutputMute(_)
            | Request::InputVolume(_)
            | Request::InputMute(_)
                if run.tool("wpctl").is_none() && run.tool("pactl").is_some() =>
            {
                let (verb, target, value) = match request {
                    Request::OutputVolume(v) if *v <= 100 => {
                        ("set-sink-volume", "@DEFAULT_SINK@", format!("{v}%"))
                    }
                    Request::InputVolume(v) if *v <= 100 => {
                        ("set-source-volume", "@DEFAULT_SOURCE@", format!("{v}%"))
                    }
                    Request::OutputMute(m) => {
                        ("set-sink-mute", "@DEFAULT_SINK@", u8::from(*m).to_string())
                    }
                    Request::InputMute(m) => (
                        "set-source-mute",
                        "@DEFAULT_SOURCE@",
                        u8::from(*m).to_string(),
                    ),
                    _ => return Err(tr("shell-volume-is-outside-its-allowed-range").into()),
                };
                run.run("pactl", &[verb, target, &value]).await.map(drop)
            }
            Request::OutputVolume(v) => run.apply(&Action::Volume(*v)).await,
            Request::OutputMute(m) => run.apply(&Action::Mute(*m)).await,
            Request::InputVolume(v) if *v <= 100 => run
                .run(
                    "wpctl",
                    &[
                        "set-volume",
                        "--limit",
                        "1.0",
                        "@DEFAULT_AUDIO_SOURCE@",
                        &format!("{v}%"),
                    ],
                )
                .await
                .map(drop),
            Request::InputVolume(_) => Err(tr("shell-volume-is-outside-its-allowed-range").into()),
            Request::InputMute(m) => run
                .run(
                    "wpctl",
                    &[
                        "set-mute",
                        "@DEFAULT_AUDIO_SOURCE@",
                        if *m { "1" } else { "0" },
                    ],
                )
                .await
                .map(drop),
            Request::AudioDevice(device) => run.apply(&Action::AudioDevice(device.clone())).await,
            Request::WifiRadio(on) => run.apply(&Action::Wifi(*on)).await,
            Request::Vpn(uuid, on) => {
                self.known_vpn(uuid).await?;
                // Waits for the tunnel (and any password prompt) to finish.
                self.tool(
                    "nmcli",
                    &[
                        "--wait",
                        "90",
                        "connection",
                        if *on { "up" } else { "down" },
                        "uuid",
                        uuid,
                    ],
                    100,
                )
                .await
            }
            Request::VpnRemove(uuid) => {
                self.known_vpn(uuid).await?;
                self.tool("nmcli", &["connection", "delete", "uuid", uuid], 10)
                    .await
            }
            Request::VpnRouteAll { uuid, all } => {
                self.known_vpn(uuid).await?;
                let never = if *all { "no" } else { "yes" };
                self.tool(
                    "nmcli",
                    &[
                        "connection",
                        "modify",
                        "uuid",
                        uuid,
                        "ipv4.never-default",
                        never,
                        "ipv6.never-default",
                        never,
                    ],
                    10,
                )
                .await
            }
            Request::VpnMetered { uuid, metered } => {
                self.known_vpn(uuid).await?;
                self.tool(
                    "nmcli",
                    &[
                        "connection",
                        "modify",
                        "uuid",
                        uuid,
                        "connection.metered",
                        if *metered { "yes" } else { "unknown" },
                    ],
                    10,
                )
                .await
            }
            Request::VpnUsername { uuid, name } => {
                self.known_vpn(uuid).await?;
                if !parse::valid_vpn_username(name) {
                    return Err(tr(
                        "shell-enter-a-sign-in-name-without-commas-equals-signs-or-backslashes",
                    )
                    .into());
                }
                let item = format!("username={}", name.trim());
                self.tool(
                    "nmcli",
                    &["connection", "modify", "uuid", uuid, "+vpn.data", &item],
                    10,
                )
                .await
            }
            Request::ConnectivityCheck => {
                self.tool("nmcli", &["networking", "connectivity", "check"], 30)
                    .await
            }
            Request::ConnectivityChecks(on) => {
                if self.connectivity().await.checks.is_none() {
                    return Err(tr("shell-this-system-has-no-internet-check-configured").into());
                }
                let args = [
                    "--system",
                    "set-property",
                    "org.freedesktop.NetworkManager",
                    "/org/freedesktop/NetworkManager",
                    "org.freedesktop.NetworkManager",
                    "ConnectivityCheckEnabled",
                    "b",
                    if *on { "true" } else { "false" },
                ];
                self.tool("busctl", &args, 30).await
            }
            Request::PortalSignIn => {
                // Only NetworkManager's own probe address, only while it
                // reports a portal, and only because the person asked.
                let connectivity = self.connectivity().await;
                let uri = connectivity
                    .sign_in()
                    .ok_or_else(|| tr("shell-this-network-does-not-ask-you-to-sign-in"))?
                    .to_owned();
                open_in_browser(&uri).await
            }
            Request::Bluetooth(on) => run.apply(&Action::Bluetooth(*on)).await,
            Request::BluetoothDevice(address, on) => {
                let known = self
                    .bluetooth()
                    .await
                    .devices
                    .iter()
                    .any(|d| d.address == *address);
                if !known {
                    return Err(tr("shell-that-device-is-no-longer-paired").into());
                }
                // Headsets and speakers often take several seconds to connect.
                self.tool(
                    "bluetoothctl",
                    &[if *on { "connect" } else { "disconnect" }, address],
                    20,
                )
                .await
            }
            Request::PowerProfile(profile) => {
                let current = self
                    .power()
                    .await
                    .profiles
                    .ok_or_else(|| tr("shell-power-profiles-are-not-available"))?;
                if !current.available.contains(profile) {
                    return Err(tr("shell-that-power-mode-is-not-offered-on-this-computer").into());
                }
                for (service, path, interface) in PROFILE_SERVICES {
                    let args = [
                        "--system",
                        "set-property",
                        service,
                        path,
                        interface,
                        "ActiveProfile",
                        "s",
                        profile,
                    ];
                    if run.run("busctl", &args).await.is_ok() {
                        return Ok(());
                    }
                }
                Err(tr("shell-the-power-mode-could-not-be-changed").into())
            }
            Request::DefaultPrinter(name) => {
                let known = self
                    .printers()
                    .await
                    .printers
                    .iter()
                    .any(|p| p.name == *name);
                if !known {
                    return Err(tr("shell-that-printer-is-no-longer-available").into());
                }
                run.run("lpoptions", &["-d", name]).await.map(drop)
            }
            Request::WifiJoin {
                ssid,
                password,
                hidden,
                identity: Some(identity),
            } => {
                self.join_enterprise(ssid, identity, password.as_ref(), *hidden)
                    .await
            }
            Request::WifiJoin {
                ssid,
                password,
                hidden,
                identity: None,
            } => self.join(ssid, password.as_ref(), *hidden).await,
            Request::Ipv4 { connection, manual } => {
                let (by, key) = self.known_connection(connection).await?;
                let manual = manual.as_ref().map(ManualIpv4::validated).transpose()?;
                let (method, address, gateway) = match &manual {
                    Some(m) => ("manual", m.address.as_str(), m.gateway.as_str()),
                    None => ("auto", "", ""),
                };
                let mut args = vec![
                    "connection",
                    "modify",
                    by,
                    &key,
                    "ipv4.method",
                    method,
                    "ipv4.addresses",
                    address,
                    "ipv4.gateway",
                    gateway,
                ];
                // A manual address gets no DNS servers from the network. The
                // ones in use now are kept as the profile's own, where the DNS
                // row shows them, unless the profile already names its own:
                // otherwise name lookups stop and nothing says why.
                let network = self.network().await;
                let current_dns = manual
                    .as_ref()
                    .and_then(|_| {
                        network
                            .interfaces
                            .iter()
                            .find(|i| i.connection == *connection)
                    })
                    .filter(|i| i.profile.as_ref().is_none_or(|p| p.dns.is_empty()))
                    .and_then(|i| i.details.as_ref())
                    .map(|details| {
                        details
                            .dns
                            .iter()
                            .filter(|server| server.parse::<std::net::Ipv4Addr>().is_ok())
                            .take(4)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .filter(|servers| !servers.is_empty());
                if let Some(servers) = &current_dns {
                    args.extend(["ipv4.dns", servers.as_str()]);
                }
                self.tool("nmcli", &args, 10).await?;
                // A new address needs the connection brought up again.
                self.tool("nmcli", &["--wait", "20", "connection", "up", by, &key], 25)
                    .await
            }
            Request::Forget(name) => {
                // A Wi-Fi network, named by its network name or its profile's
                // name. Only Wi-Fi profiles go, each by UUID: `delete id` took
                // every profile of that name, a VPN called "Office" included.
                let network = self.network().await;
                let profiles: Vec<String> = network
                    .wifi
                    .iter()
                    .filter(|c| c.name == *name || c.ssid.as_deref() == Some(name.as_str()))
                    .map(|c| c.uuid.clone())
                    .collect();
                if profiles.is_empty() {
                    return Err(tr("shell-that-connection-is-no-longer-saved").into());
                }
                for uuid in profiles {
                    self.tool("nmcli", &["connection", "delete", "uuid", &uuid], 10)
                        .await?;
                }
                Ok(())
            }
            Request::WifiDisconnect => {
                let network = self.network().await;
                let device = network
                    .interfaces
                    .iter()
                    .find(|i| i.kind == InterfaceKind::Wifi && i.state == LinkState::Connected)
                    .ok_or_else(|| tr("shell-wi-fi-is-not-connected"))?;
                self.tool("nmcli", &["device", "disconnect", &device.device], 10)
                    .await
            }
            Request::Airplane(on) => {
                // Remembers whether Bluetooth was on, so leaving airplane mode
                // restores it rather than leaving it off.
                let marker = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(|d| std::path::PathBuf::from(d).join("enzui-airplane-bluetooth"));
                let bluetooth = self.bluetooth().await.adapter;
                self.tool(
                    "nmcli",
                    &["radio", "all", if *on { "off" } else { "on" }],
                    10,
                )
                .await?;
                match (on, bluetooth, marker) {
                    (true, Some(adapter), marker) if adapter.powered => {
                        if let Some(marker) = marker {
                            let _ = std::fs::write(marker, b"1");
                        }
                        let _ = run.apply(&Action::Bluetooth(false)).await;
                    }
                    (false, Some(_), Some(marker)) if marker.exists() => {
                        let _ = std::fs::remove_file(&marker);
                        let _ = run.apply(&Action::Bluetooth(true)).await;
                    }
                    _ => {}
                }
                Ok(())
            }
            Request::Trust {
                connection,
                trusted,
            } if let Some(backend) = self.enzimos().await => {
                let rows = backend
                    .query("network-trust")
                    .await
                    .map(|v| enzimos::trust_rows(&v))
                    .map_err(|r| r.message())?;
                let row = rows
                    .iter()
                    .find(|r| r.name == *connection)
                    .ok_or_else(|| tr("shell-that-connection-is-no-longer-known"))?;
                let selection =
                    serde_json::json!({"connection": row.connection, "trusted": trusted});
                backend
                    .run(
                        25,
                        "network-trust-set",
                        Some(selection),
                        Duration::from_secs(180),
                    )
                    .await
                    .map(drop)
                    .map_err(|r| r.message())
            }
            Request::CriticalBattery {
                action,
                risk_acknowledged,
            } => {
                if !CRITICAL_ACTIONS.contains(&action.as_str()) {
                    return Err(tr("shell-unknown-battery-action").into());
                }
                let backend = self
                    .enzimos()
                    .await
                    .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
                let selection =
                    serde_json::json!({"action": action, "risk_acknowledged": risk_acknowledged});
                backend
                    .run(
                        28,
                        "battery-action-set",
                        Some(selection),
                        Duration::from_secs(180),
                    )
                    .await
                    .map(drop)
                    .map_err(|r| r.message())
            }
            Request::MemoryPolicy {
                mode,
                risk_acknowledged,
            } => {
                if !super::MEMORY_MODES.contains(&mode.as_str()) {
                    return Err(tr("shell-unknown-memory-protection-preset").into());
                }
                let backend = self
                    .enzimos()
                    .await
                    .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
                // The acknowledgement is required for Off and refused for the presets.
                let selection = serde_json::json!({"mode": mode, "risk_acknowledged": *risk_acknowledged && mode == "off"});
                backend
                    .run(
                        32,
                        "memory-policy-set",
                        Some(selection),
                        Duration::from_secs(180),
                    )
                    .await
                    .map(drop)
                    .map_err(|r| r.message())
            }
            Request::TemporaryStorage(mode) => {
                let backend = self
                    .enzimos()
                    .await
                    .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
                backend
                    .run(
                        33,
                        "tmp-storage-set",
                        Some(serde_json::json!({"mode": mode})),
                        Duration::from_secs(180),
                    )
                    .await
                    .map(drop)
                    .map_err(|r| r.message())
            }
            Request::HomeHistory(on) => {
                let backend = self
                    .enzimos()
                    .await
                    .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
                let selection = serde_json::json!({"enabled": on});
                backend
                    .run(
                        26,
                        "home-history-set",
                        Some(selection),
                        Duration::from_secs(180),
                    )
                    .await
                    .map(drop)
                    .map_err(|r| r.message())
            }
            Request::BackupSchedule(on) => {
                let backend = self
                    .enzimos()
                    .await
                    .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
                let selection = serde_json::json!({"enabled": on});
                match backend.run(27, "backup-schedule-set", Some(selection), Duration::from_secs(180)).await {
                    Ok(_) => Ok(()),
                    Err(enzimos::Refusal::Code(code)) if *on && code == "job-plan-changed" => Err(tr("shell-the-backup-setup-or-recovery-copy-confirmation-changed-review-the-destinatio").into()),
                    Err(refusal) => Err(refusal.message()),
                }
            }
            Request::Trust {
                connection,
                trusted,
            } => {
                let (by, key) = self.known_connection(connection).await?;
                let zone = if *trusted { "home" } else { "public" };
                self.tool(
                    "nmcli",
                    &["connection", "modify", by, &key, "connection.zone", zone],
                    10,
                )
                .await
            }
            Request::Dns {
                connection,
                servers,
            } => {
                let (by, key) = self.known_connection(connection).await?;
                let mut v4 = Vec::new();
                let mut v6 = Vec::new();
                for server in servers.iter().take(4) {
                    match server.parse::<std::net::IpAddr>() {
                        Ok(std::net::IpAddr::V4(ip)) => v4.push(ip.to_string()),
                        Ok(std::net::IpAddr::V6(ip)) => v6.push(ip.to_string()),
                        Err(_) => {
                            return Err(
                                tr!("shell-is-not-an-ip-address", "arg0" => server.clone()),
                            );
                        }
                    }
                }
                let custom = !servers.is_empty();
                let (v4, v6) = (v4.join(","), v6.join(","));
                let ignore = if custom { "yes" } else { "no" };
                self.tool(
                    "nmcli",
                    &[
                        "connection",
                        "modify",
                        by,
                        &key,
                        "ipv4.dns",
                        &v4,
                        "ipv6.dns",
                        &v6,
                        "ipv4.ignore-auto-dns",
                        ignore,
                        "ipv6.ignore-auto-dns",
                        ignore,
                    ],
                    10,
                )
                .await?;
                // Applies the new servers without dropping the connection.
                let device = self
                    .network()
                    .await
                    .interfaces
                    .into_iter()
                    .find(|i| i.connection == *connection)
                    .map(|i| i.device);
                if let Some(device) = device {
                    let _ = self
                        .tool("nmcli", &["device", "reapply", &device], 10)
                        .await;
                }
                Ok(())
            }
            Request::VpnAdd(spec) => {
                crate::vpn_manual::validate(spec).map_err(String::from)?;
                self.add_vpn(&crate::vpn_manual::profile(spec)).await
            }
            Request::VpnImport(path) => {
                use crate::vpn_profile::{Kind, MAX_PROFILE, imported_uuid, problem};
                let path = std::path::Path::new(path);
                let kind = Kind::from_path(path)
                    .ok_or_else(|| tr("shell-choose-a-wireguard-conf-or-openvpn-ovpn-file"))?;
                if !path.is_absolute() || !path.is_file() {
                    return Err(tr("shell-that-file-does-not-exist").into());
                }
                let metadata =
                    std::fs::metadata(path).map_err(|_| tr("shell-that-file-cannot-be-read"))?;
                if metadata.len() > MAX_PROFILE {
                    return Err(tr("shell-that-file-is-too-large-to-be-a-vpn-profile").into());
                }
                let text = std::fs::read_to_string(path)
                    .map_err(|_| tr("shell-that-file-is-not-a-text-vpn-profile"))?;
                if let Some(problem) = problem(kind, &text) {
                    return Err(problem);
                }
                if kind == Kind::Vpnc {
                    // NetworkManager's .pcf importer leaves required settings
                    // out; the profile becomes an ordinary Cisco IPsec VPN.
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("VPN");
                    let spec = crate::vpn_manual::from_pcf(&text, stem, |hex| {
                        std::process::Command::new("cisco-decrypt")
                            .arg(hex)
                            .stdin(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .output()
                            .ok()
                            .filter(|o| o.status.success())
                            .and_then(|o| String::from_utf8(o.stdout).ok())
                            .map(|s| s.trim().to_owned())
                    })
                    .map_err(String::from)?;
                    crate::vpn_manual::validate(&spec).map_err(String::from)?;
                    return self.add_vpn(&crate::vpn_manual::profile(&spec)).await;
                }
                let file = path
                    .to_str()
                    .ok_or_else(|| tr("shell-that-file-name-is-not-valid-text"))?;
                let output = match self
                    .controls
                    .run_with(
                        "nmcli",
                        &["connection", "import", "type", kind.nm(), "file", file],
                        None,
                        Duration::from_secs(20),
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(Failure::Rejected(line)) => return Err(explain("nmcli", &line)),
                    Err(Failure::Other(message)) => return Err(message),
                };
                // NetworkManager starts an imported WireGuard tunnel at once and
                // at every boot; it starts only when the user turns it on.
                if kind == Kind::WireGuard {
                    let manual = tr("shell-vpn-imported-autostart-on");
                    let uuid = imported_uuid(&output).ok_or(manual)?;
                    self.tool(
                        "nmcli",
                        &[
                            "connection",
                            "modify",
                            "uuid",
                            &uuid,
                            "connection.autoconnect",
                            "no",
                        ],
                        10,
                    )
                    .await
                    .map_err(|_| manual)?;
                    let _ = self
                        .tool("nmcli", &["connection", "down", "uuid", &uuid], 10)
                        .await;
                }
                Ok(())
            }
            Request::Firewall(on) => {
                // Two firewalls fight over the same rules.
                if *on
                    && let crate::platform::Firewall::Other(other) =
                        crate::platform::firewall(&self.controls).await
                {
                    return Err(tr!("shell-another-firewall-is-active", "arg0" => other));
                }
                let verb = if *on { "enable" } else { "disable" };
                self.tool("systemctl", &[verb, "--now", "firewalld.service"], 20)
                    .await
            }
            Request::BluetoothSearch => {
                self.bluetooth()
                    .await
                    .adapter
                    .filter(|a| a.powered)
                    .ok_or_else(|| tr("shell-turn-bluetooth-on-to-search-for-devices"))?;
                // Discovery lasts as long as the requesting client, so the
                // search is one bounded run rather than a background process.
                match run
                    .run_with(
                        "bluetoothctl",
                        &["--timeout", "10", "scan", "on"],
                        None,
                        Duration::from_secs(14),
                    )
                    .await
                {
                    Ok(_) | Err(Failure::Other(_)) => Ok(()),
                    Err(Failure::Rejected(line)) => Err(if line.is_empty() {
                        tr("shell-could-not-search-for-devices").into()
                    } else {
                        line
                    }),
                }
            }
            Request::BluetoothPair(address) => {
                super::pairing::pair(address, |prompt| {
                    prompt.answer(super::pairing::Answer::Reject);
                })
                .await
            }
            Request::BluetoothRemove(address) => {
                let known = self
                    .bluetooth()
                    .await
                    .devices
                    .iter()
                    .any(|d| d.address == *address);
                if !known {
                    return Err(tr("shell-that-device-is-no-longer-paired").into());
                }
                self.tool("bluetoothctl", &["remove", address], 10).await
            }
            Request::PrinterAdd(printer) if printer.driver.is_some() => {
                let Some((driver, _)) = &printer.driver else {
                    return Err(tr("shell-printer-no-driver").into());
                };
                if !crate::cups_admin::valid_device_uri(&printer.uri) {
                    return Err(tr("shell-printer-no-driver").into());
                }
                let name = crate::cups_admin::display_name(&printer.name)
                    .ok_or_else(|| tr("shell-that-printer-has-no-usable-name"))?;
                let queue = printer_queue_name(&name);
                let first = self.printers().await.printers.is_empty();
                crate::cups_admin::add_with_driver(&queue, &printer.uri, driver, &name).await?;
                if first {
                    let _ = self.tool("lpoptions", &["-d", &queue], 10).await;
                }
                Ok(())
            }
            Request::PrinterAdd(printer) => {
                if !crate::cups_admin::valid_uri(&printer.uri) {
                    return Err(
                        tr("shell-only-network-printers-that-support-ipp-can-be-added").into(),
                    );
                }
                let name = crate::cups_admin::display_name(&printer.name)
                    .ok_or_else(|| tr("shell-that-printer-has-no-usable-name"))?;
                let queue = printer_queue_name(&name);
                let first = self.printers().await.printers.is_empty();
                // IPP Everywhere: the printer describes itself, no driver
                // needed; polkit decides whether an administrator must approve.
                crate::cups_admin::add(&queue, &printer.uri, &name).await?;
                if first {
                    // This account's default; changing it needs nobody else.
                    let _ = self.tool("lpoptions", &["-d", &queue], 10).await;
                }
                Ok(())
            }
            Request::PrinterRemove(name) => {
                self.known_printer(name).await?;
                crate::cups_admin::remove(name).await
            }
            Request::PrinterPause(name, paused) => {
                self.known_printer(name).await?;
                crate::cups_admin::set_enabled(name, !*paused).await
            }
            Request::JobCancel(id) => {
                // Only this account's own jobs are listed, and CUPS lets an
                // owner cancel them without an administrator.
                if !self.printers().await.jobs.iter().any(|job| job.id == *id) {
                    return Err(tr("shell-that-print-job-has-already-finished").into());
                }
                self.tool("cancel", &[id], 10).await
            }
            Request::PrinterTest(name) => {
                self.known_printer(name).await?;
                let page = [
                    "/usr/share/cups/data/default-testpage.pdf",
                    "/usr/share/cups/data/testprint",
                ]
                .into_iter()
                .find(|p| std::path::Path::new(p).is_file())
                .ok_or_else(|| tr("shell-no-test-page-is-installed"))?;
                self.tool("lp", &["-d", name, "-t", tr("shell-test-page"), page], 10)
                    .await
            }
            Request::Timezone(zone) => {
                if !self.clock().await.zones.contains(zone) {
                    return Err(tr("shell-unknown-time-zone").into());
                }
                self.tool("timedatectl", &["set-timezone", zone], ADMIN_SECONDS)
                    .await
            }
            Request::Hostname(name) => {
                if !valid_hostname(name) {
                    return Err(
                        tr("shell-use-letters-numbers-and-hyphens-up-to-63-characters").into(),
                    );
                }
                self.tool("hostnamectl", &["set-hostname", name], ADMIN_SECONDS)
                    .await
            }
            Request::Ntp(on) => {
                self.tool(
                    "timedatectl",
                    &["set-ntp", if *on { "true" } else { "false" }],
                    ADMIN_SECONDS,
                )
                .await
            }
        }
    }
    /// Adds a VPN profile through NetworkManager's settings service on the
    /// system bus. Its group or pre-shared secret goes in the same call:
    /// given to `nmcli` it would sit on the command line, which every account
    /// on the computer can read from the process list.
    async fn add_vpn(&self, profile: &crate::vpn_manual::Profile<'_>) -> Result<(), String> {
        if !self.controls.uses_bus() {
            return Err(tr("shell-networkmanager-could-not-be-reached").into());
        }
        let system = crate::bus::system()
            .await
            .ok_or_else(|| tr("shell-networkmanager-could-not-be-reached"))?;
        let settings = (crate::vpn_manual::connection_settings(profile),);
        let proxy = zbus::Proxy::new(
            &system,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager/Settings",
            "org.freedesktop.NetworkManager.Settings",
        )
        .await
        .map_err(|_| tr("shell-networkmanager-could-not-be-reached"))?;
        // Adding a system connection may need an administrator: the desktop's
        // own polkit agent asks, as it did for nmcli.
        let call = proxy.call_with_flags::<_, _, zbus::zvariant::OwnedObjectPath>(
            "AddConnection",
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            &settings,
        );
        match tokio::time::timeout(Duration::from_secs(120), call).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(zbus::Error::MethodError(_, Some(detail), _))) => Err(explain("nmcli", &detail)),
            Ok(Err(_)) => Err(tr("shell-networkmanager-refused-the-vpn").into()),
            Err(_) => Err(tr("shell-networkmanager-did-not-answer").into()),
        }
    }
    async fn known_vpn(&self, uuid: &str) -> Result<(), String> {
        if self.network().await.vpns.iter().any(|v| v.uuid == uuid) {
            Ok(())
        } else {
            Err(tr("shell-that-vpn-connection-no-longer-exists").into())
        }
    }
    async fn tool(&self, name: &str, args: &[&str], seconds: u64) -> Result<(), String> {
        match self
            .controls
            .run_with(name, args, None, Duration::from_secs(seconds))
            .await
        {
            Ok(_) => Ok(()),
            Err(Failure::Rejected(line)) => Err(explain(name, &line)),
            Err(Failure::Other(message)) => Err(message),
        }
    }
    async fn join(
        &self,
        ssid: &str,
        password: Option<&Secret>,
        hidden: bool,
    ) -> Result<(), String> {
        if ssid.is_empty() || ssid.len() > 32 {
            return Err(tr("shell-network-names-are-1-to-32-characters").into());
        }
        let network = self.network().await;
        let point = network.access_points.iter().find(|p| p.ssid == ssid);
        if !hidden && point.is_none() {
            return Err(tr("shell-that-network-is-out-of-range").into());
        }
        let saved = wifi_profiles(&network, ssid);
        if point.is_some_and(|p| p.saved)
            && password.is_none()
            && let Some(uuid) = saved.first()
        {
            return self
                .tool("nmcli", &["connection", "up", "uuid", uuid], 45)
                .await;
        }
        if ssid.starts_with('-') {
            // nmcli reads an argument starting with "-" as one of its own
            // options (a network called "-a" would be taken for --ask).
            return self.join_by_bus(&network, ssid, password, hidden).await;
        }
        let before: Vec<String> = network.wifi.iter().map(|c| c.uuid.clone()).collect();
        let mut args = vec!["--wait", "20", "--ask", "device", "wifi", "connect", ssid];
        if hidden {
            args.extend(["hidden", "yes"]);
        }
        // nmcli --ask reads the key from standard input, so it never appears
        // in the process list.
        let input = password.map_or_else(|| "\n".into(), |p| format!("{}\n", p.0));
        let result = self
            .controls
            .run_with("nmcli", &args, Some(&input), Duration::from_secs(30))
            .await;
        if result.is_err() {
            // A failed first join leaves a saved profile with the wrong key;
            // remove it so the next attempt asks again. Only the profile this
            // join made goes: `delete id` also took any other profile with
            // that name, a VPN or a wired one.
            self.forget_new_profiles(&before, ssid).await;
        }
        match result {
            Ok(_) => Ok(()),
            // A wrong WPA key shows up as the access point never completing
            // the handshake, not as an explicit refusal.
            Err(Failure::Rejected(line))
                if password.is_some() && line.to_lowercase().contains("timeout") =>
            {
                Err(tr("shell-the-password-is-incorrect-or-the-network-did-not-answer").into())
            }
            Err(Failure::Rejected(line)) => Err(explain("nmcli", &line)),
            Err(Failure::Other(_)) if password.is_some() => {
                Err(tr("shell-the-password-is-incorrect-or-the-network-did-not-answer").into())
            }
            Err(Failure::Other(message)) => Err(message),
        }
    }
    // Enterprise Wi-Fi uses PEAP/MSCHAPv2.
    async fn join_enterprise(
        &self,
        ssid: &str,
        identity: &str,
        password: Option<&Secret>,
        hidden: bool,
    ) -> Result<(), String> {
        if ssid.is_empty() || ssid.len() > 32 {
            return Err(tr("shell-network-names-are-1-to-32-characters").into());
        }
        let identity = identity.trim();
        if identity.is_empty() || identity.len() > 128 || identity.chars().any(char::is_control) {
            return Err(tr("shell-enter-your-username").into());
        }
        let password = password.ok_or_else(|| tr("shell-enter-your-password"))?;
        let network = self.network().await;
        if !hidden && !network.access_points.iter().any(|p| p.ssid == ssid) {
            return Err(tr("shell-that-network-is-out-of-range").into());
        }
        // The new profile replaces this network's old ones, each by UUID.
        for uuid in wifi_profiles(&network, ssid) {
            let _ = self
                .tool("nmcli", &["connection", "delete", "uuid", &uuid], 10)
                .await;
        }
        let uuid = new_uuid().ok_or_else(|| tr("shell-the-network-could-not-be-added"))?;
        let mut add = vec![
            "connection",
            "add",
            "type",
            "wifi",
            "con-name",
            ssid,
            "ssid",
            ssid,
            "connection.uuid",
            &uuid,
            "wifi-sec.key-mgmt",
            "wpa-eap",
            "802-1x.eap",
            "peap",
            "802-1x.phase2-auth",
            "mschapv2",
            "802-1x.identity",
            identity,
        ];
        if hidden {
            add.extend(["802-11-wireless.hidden", "yes"]);
        }
        self.tool("nmcli", &add, 10).await?;
        if password.0.chars().any(char::is_control) {
            return Err(tr("shell-the-password-cannot-contain-line-breaks").into());
        }
        // NetworkManager asks for the identity as a secret too. Both go in a
        // passwd-file read from standard input, never on the command line.
        let input = format!(
            "802-1x.identity:{identity}\n802-1x.password:{}\n",
            password.0
        );
        let result = self
            .controls
            .run_with(
                "nmcli",
                &[
                    "--wait",
                    "45",
                    "connection",
                    "up",
                    "uuid",
                    &uuid,
                    "passwd-file",
                    "/dev/stdin",
                ],
                Some(&input),
                Duration::from_secs(55),
            )
            .await;
        if result.is_err() {
            let _ = self
                .tool("nmcli", &["connection", "delete", "uuid", &uuid], 10)
                .await;
        }
        match result {
            Ok(_) => Ok(()),
            Err(Failure::Rejected(line))
                if line.to_lowercase().contains("timeout") || line.contains("secrets") =>
            {
                Err(
                    tr("shell-the-username-or-password-is-incorrect-or-the-network-did-not-answer")
                        .into(),
                )
            }
            Err(Failure::Rejected(line)) => Err(explain("nmcli", &line)),
            Err(Failure::Other(_)) => Err(tr(
                "shell-the-username-or-password-is-incorrect-or-the-network-did-not-answer",
            )
            .into()),
        }
    }
    /// Removes the Wi-Fi profiles for `ssid` that were not saved before a
    /// join that failed.
    async fn forget_new_profiles(&self, before: &[String], ssid: &str) {
        let network = self.network().await;
        for uuid in wifi_profiles(&network, ssid) {
            if !before.contains(&uuid) {
                let _ = self
                    .tool("nmcli", &["connection", "delete", "uuid", &uuid], 10)
                    .await;
            }
        }
    }
    /// Joins a network through NetworkManager's bus, for a name `nmcli`
    /// would read as an option. The key goes in the call, never on a command
    /// line; a join that does not come up removes its profile again.
    async fn join_by_bus(
        &self,
        network: &Network,
        ssid: &str,
        password: Option<&Secret>,
        hidden: bool,
    ) -> Result<(), String> {
        let device = network
            .interfaces
            .iter()
            .find(|i| i.kind == InterfaceKind::Wifi)
            .ok_or_else(|| tr("shell-wi-fi-is-not-connected"))?
            .device
            .clone();
        let system = crate::bus::system()
            .await
            .filter(|_| self.controls.uses_bus())
            .ok_or_else(|| tr("shell-networkmanager-could-not-be-reached"))?;
        let manager = zbus::Proxy::new(
            &system,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        .map_err(|_| tr("shell-networkmanager-could-not-be-reached"))?;
        let device: zbus::zvariant::OwnedObjectPath = manager
            .call("GetDeviceByIpIface", &(device.as_str(),))
            .await
            .map_err(|_| tr("shell-wi-fi-is-not-connected"))?;
        let settings = (
            wifi_settings(ssid, password.map(|p| p.0.as_str()), hidden),
            device,
            zbus::zvariant::ObjectPath::from_static_str_unchecked("/"),
        );
        let (profile, active): (
            zbus::zvariant::OwnedObjectPath,
            zbus::zvariant::OwnedObjectPath,
        ) = manager
            .call_with_flags(
                "AddAndActivateConnection",
                zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                &settings,
            )
            .await
            .map_err(|_| tr("shell-networkmanager-refused-the-network"))?
            .ok_or_else(|| tr("shell-networkmanager-refused-the-network"))?;
        // Waits as `nmcli --wait 20` does: activated (2) or given up (4).
        let state = async {
            let active = zbus::Proxy::new(
                &system,
                "org.freedesktop.NetworkManager",
                active.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .await
            .ok()?;
            for _ in 0..80 {
                match active.get_property::<u32>("State").await {
                    Ok(2) => return Some(true),
                    Ok(4) | Err(_) => return Some(false),
                    _ => tokio::time::sleep(Duration::from_millis(250)).await,
                }
            }
            Some(false)
        };
        if state.await == Some(true) {
            return Ok(());
        }
        if let Ok(profile) = zbus::Proxy::new(
            &system,
            "org.freedesktop.NetworkManager",
            profile.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        {
            let _ = profile.call::<_, _, ()>("Delete", &()).await;
        }
        Err(if password.is_some() {
            tr("shell-the-password-is-incorrect-or-the-network-did-not-answer").into()
        } else {
            tr("shell-the-network-did-not-answer").into()
        })
    }
    /// How `nmcli` names the profile in use under `name`: by UUID when it is
    /// known, since several profiles can share a name and `modify id` would
    /// change whichever comes first.
    async fn known_connection(&self, name: &str) -> Result<(&'static str, String), String> {
        let network = self.network().await;
        if let Some(interface) = network.interfaces.iter().find(|i| i.connection == name) {
            return Ok(if interface.connection_uuid.is_empty() {
                ("id", name.to_owned())
            } else {
                ("uuid", interface.connection_uuid.clone())
            });
        }
        let vpns: Vec<_> = network.vpns.iter().filter(|v| v.name == name).collect();
        match vpns.as_slice() {
            [vpn] => Ok(("uuid", vpn.uuid.clone())),
            [] => Err(tr("shell-that-connection-is-no-longer-active").into()),
            // Several VPNs of that name: changing one by name is a guess.
            _ => Err(tr("shell-several-connections-share-that-name").into()),
        }
    }
    async fn known_printer(&self, name: &str) -> Result<(), String> {
        if self
            .printers()
            .await
            .printers
            .iter()
            .any(|p| p.name == name)
        {
            Ok(())
        } else {
            Err(tr("shell-that-printer-is-no-longer-available").into())
        }
    }
}

fn valid_hostname(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// A CUPS queue name from a printer's display name.
pub fn printer_queue_name(name: &str) -> String {
    let queue: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if queue.is_empty() {
        tr("shell-printer").into()
    } else {
        queue
    }
}

// Reap slow browser startup asynchronously; report only immediate handler failures.
async fn open_in_browser(uri: &str) -> Result<(), String> {
    let mut child = tokio::process::Command::new("/usr/bin/gio")
        .args(["open", uri])
        .current_dir("/")
        .env_remove("XDG_ACTIVATION_TOKEN")
        .env_remove("DESKTOP_STARTUP_ID")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(
            |error| tr!("shell-could-not-open-the-sign-in-page-2", "arg0" => format!("{:}", error)),
        )?;
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(_)) => Err(tr("shell-no-browser-could-open-the-sign-in-page").into()),
        Ok(Err(error)) => {
            Err(tr!("shell-could-not-open-the-sign-in-page-2", "arg0" => format!("{:}", error)))
        }
        Err(_) => {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

fn explain(tool: &str, line: &str) -> String {
    let lower = line.to_lowercase();
    if lower.contains("secrets were required")
        || lower.contains("802-11-wireless-security.psk")
        || lower.contains("invalid passphrase")
    {
        tr("shell-the-password-is-incorrect").into()
    } else if lower.contains("not authorized")
        || lower.contains("interactive authentication required")
        || lower.contains("forbidden")
    {
        tr("shell-you-are-not-allowed-to-change-this-an-administrator-must-approve-it").into()
    } else if lower.contains("vpn")
        && (lower.contains("plugin") || lower.contains("service") && lower.contains("not"))
        && (lower.contains("not") || lower.contains("fail"))
    {
        tr("shell-the-vpn-software-for-this-profile-is-not-installed").into()
    } else if lower.contains("user canceled")
        || lower.contains("no agents were available")
        || lower.contains("secrets were required, but not provided") && lower.contains("vpn")
    {
        tr("shell-the-connection-needs-a-password-or-key-it-was-not-entered").into()
    } else if tool == "gio" && (lower.contains("not supported") || lower.contains("no application"))
    {
        tr("shell-no-web-browser-is-set-up-to-open-the-sign-in-page").into()
    } else if lower.contains("vpn") && lower.contains("activation failed") {
        // Not the plugin's own text: it can name servers and addresses.
        tr("shell-the-vpn-did-not-connect-check-the-profile-and-your-sign-in-details").into()
    } else if tool == "nmcli" && lower.contains("timeout was reached") {
        tr("shell-the-check-for-internet-access-did-not-answer-in-time-the-network-may-still-b")
            .into()
    } else if lower.contains("no network with ssid") {
        tr("shell-that-network-is-out-of-range").into()
    } else if tool == "bluetoothctl"
        && (lower.contains("authenticationfailed") || lower.contains("authentication"))
    {
        tr("shell-pairing-was-rejected-put-the-device-in-pairing-mode-and-try-again").into()
    } else if line.is_empty() {
        tr!("shell-could-not-complete-the-change", "arg0" => tool.to_owned())
    } else {
        line.trim_start_matches("Error: ").to_owned()
    }
}

/// The UUIDs of the saved Wi-Fi profiles for the network `ssid`: by the
/// network name each profile stores, or its own name when that was not read.
fn wifi_profiles(network: &Network, ssid: &str) -> Vec<String> {
    network
        .wifi
        .iter()
        .filter(|c| c.ssid.as_deref().unwrap_or(&c.name) == ssid)
        .map(|c| c.uuid.clone())
        .collect()
}

/// A random profile UUID from the kernel, so a profile added with `nmcli` is
/// then named by it rather than by a name another profile may share.
fn new_uuid() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .ok()
        .map(|uuid| uuid.trim().to_owned())
        .filter(|uuid| uuid.len() == 36)
}

/// The settings `AddAndActivateConnection` takes for a personal network:
/// the name as bytes, and WPA2/WPA3 personal when there is a key.
fn wifi_settings(
    ssid: &str,
    key: Option<&str>,
    hidden: bool,
) -> std::collections::HashMap<
    &'static str,
    std::collections::HashMap<&'static str, zbus::zvariant::OwnedValue>,
> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    let owned = |value: Value<'_>| OwnedValue::try_from(value).expect("plain values convert");
    let mut connection = HashMap::new();
    connection.insert("id", owned(Value::from(ssid)));
    connection.insert("type", owned(Value::from("802-11-wireless")));
    let mut wireless = HashMap::new();
    wireless.insert("ssid", owned(Value::from(ssid.as_bytes().to_vec())));
    wireless.insert("mode", owned(Value::from("infrastructure")));
    if hidden {
        wireless.insert("hidden", owned(Value::from(true)));
    }
    let mut settings = HashMap::from([("connection", connection), ("802-11-wireless", wireless)]);
    if let Some(key) = key {
        let mut security = HashMap::new();
        security.insert("key-mgmt", owned(Value::from("wpa-psk")));
        security.insert("psk", owned(Value::from(key)));
        settings.insert("802-11-wireless-security", security);
    }
    settings
}
