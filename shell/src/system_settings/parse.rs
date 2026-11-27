use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::tr;

pub(super) fn terse_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields
                        .last_mut()
                        .expect("the parser starts with a field and never removes fields")
                        .push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields
                .last_mut()
                .expect("the parser starts with a field and never removes fields")
                .push(c),
        }
    }
    fields
}

pub(super) fn level(text: &str) -> Option<Level> {
    let value: f32 = text
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    if !value.is_finite() || !(0.0..=2.0).contains(&value) {
        return None;
    }
    Some(Level {
        percent: (value * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: text.contains("[MUTED]"),
    })
}

/// A level from `pactl get-sink-volume` and `pactl get-sink-mute`.
pub(super) fn pulse_level(volume: &str, mute: &str) -> Option<Level> {
    let (percent, muted) = crate::audio_devices::pulse_level(volume, mute)?;
    Some(Level { percent, muted })
}

pub(super) fn interfaces(text: &str) -> Vec<Interface> {
    text.lines()
        .filter_map(|line| {
            let fields = terse_fields(line);
            let [device, kind, state, connection] = fields.as_slice() else {
                return None;
            };
            let kind = match kind.as_str() {
                "ethernet" => InterfaceKind::Ethernet,
                "wifi" => InterfaceKind::Wifi,
                "wireguard" | "tun" | "vpn" => InterfaceKind::Vpn,
                _ => return None,
            };
            if state == "unmanaged"
                || (kind == InterfaceKind::Ethernet && device.starts_with("veth"))
            {
                return None;
            }
            let state = match state.split_whitespace().next().unwrap_or_default() {
                "connected" => LinkState::Connected,
                "connecting" => LinkState::Connecting,
                "disconnected" => LinkState::Disconnected,
                _ => LinkState::Unavailable,
            };
            Some(Interface {
                device: device.clone(),
                kind,
                state,
                connection: connection.clone(),
                connection_uuid: String::new(),
                details: None,
                profile: None,
            })
        })
        .take(16)
        .collect()
}

pub(super) fn details(text: &str) -> Details {
    let mut details = Details::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.split('[').next().unwrap_or(key);
        let value = value.trim().to_owned();
        if value.is_empty() || value == "--" {
            continue;
        }
        match key {
            "GENERAL.HWADDR" => details.hardware = Some(value),
            "IP4.ADDRESS" if details.ipv4.is_none() => details.ipv4 = Some(value),
            "IP4.GATEWAY" => details.gateway = Some(value),
            "IP4.DNS" if details.dns.len() < 3 => details.dns.push(value),
            "IP6.ADDRESS" if details.ipv6.is_none() && !value.starts_with("fe80") => {
                details.ipv6 = Some(value);
            }
            _ => {}
        }
    }
    details
}

pub(super) fn access_points(text: &str, saved: &[String]) -> Vec<AccessPoint> {
    let mut points: Vec<AccessPoint> = Vec::new();
    for line in text.lines() {
        let fields = terse_fields(line);
        let [in_use, ssid, signal, security] = fields.as_slice() else {
            continue;
        };
        if ssid.is_empty() || ssid.len() > 64 {
            continue;
        }
        let point = AccessPoint {
            ssid: ssid.clone(),
            signal: signal.parse::<u8>().unwrap_or(0).min(100),
            secure: !security.trim().is_empty() && security != "--",
            enterprise: security.contains("802.1X"),
            active: in_use == "*",
            saved: saved.iter().any(|name| name == ssid),
        };
        match points.iter_mut().find(|p| p.ssid == point.ssid) {
            Some(existing) => {
                existing.active |= point.active;
                existing.signal = existing.signal.max(point.signal);
            }
            None => points.push(point),
        }
    }
    points.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.saved.cmp(&a.saved))
            .then(b.signal.cmp(&a.signal))
    });
    points.truncate(24);
    points
}

pub(super) fn natural_key(name: &str) -> Vec<(String, u64)> {
    let mut key = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for c in name.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            key.push((
                std::mem::take(&mut text),
                digits.parse().unwrap_or(u64::MAX),
            ));
            digits.clear();
        }
        text.extend(c.to_lowercase());
    }
    key.push((text, 0));
    key
}

pub(super) fn connections(text: &str) -> Vec<SavedConnection> {
    let mut connections: Vec<SavedConnection> = text
        .lines()
        .filter_map(|line| {
            let fields = terse_fields(line);
            let [name, uuid, kind, active] = fields.as_slice() else {
                return None;
            };
            let kind = match kind.as_str() {
                "802-11-wireless" => ConnectionKind::Wifi,
                "vpn" | "wireguard" => ConnectionKind::Vpn,
                _ => return None,
            };
            Some(SavedConnection {
                name: name.clone(),
                kind,
                active: active == "yes",
                uuid: uuid.clone(),
                ssid: None,
                vpn: None,
            })
        })
        .take(256)
        .collect();
    connections.sort_by_cached_key(|c| (!c.active, natural_key(&c.name)));
    connections
}

// Parses NetworkManager connection type, VPN service, route-all and metering settings.
pub(super) fn vpn(text: &str) -> Vpn {
    let mut vpn = Vpn {
        service: VpnService::Other,
        route_all: true,
        metered: false,
        split_by_peers: false,
        sign_in: None,
    };
    let mut data = std::collections::HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match (key, value.trim()) {
            ("connection.type", "wireguard") => vpn.service = VpnService::WireGuard,
            ("vpn.service-type", service) if service.ends_with(".openvpn") => {
                vpn.service = VpnService::OpenVpn;
            }
            ("vpn.service-type", service) if service.ends_with(".openconnect") => {
                vpn.service = VpnService::OpenConnect;
            }
            ("vpn.service-type", service) if service.ends_with(".vpnc") => {
                vpn.service = VpnService::Vpnc;
            }
            ("vpn.service-type", service) if service.ends_with(".l2tp") => {
                vpn.service = VpnService::L2tp;
            }
            ("vpn.service-type", service) if service.ends_with(".strongswan") => {
                vpn.service = VpnService::Ikev2;
            }
            ("vpn.service-type", service) if service.ends_with(".pptp") => {
                vpn.service = VpnService::Pptp;
            }
            ("ipv4.never-default", value) => vpn.route_all = value != "yes",
            ("connection.metered", value) => vpn.metered = matches!(value, "yes" | "guess-yes"),
            ("vpn.data", value) => data = vpn_data_items(value),
            _ => {}
        }
    }
    if vpn.service == VpnService::OpenVpn
        && matches!(
            data.get("connection-type").map(String::as_str),
            Some("password" | "password-tls")
        )
    {
        vpn.sign_in = Some(
            data.get("username")
                .filter(|name| !name.is_empty())
                .cloned(),
        );
    }
    vpn
}

// nmcli escapes commas within vpn.data values with a backslash.
fn vpn_data_items(text: &str) -> std::collections::HashMap<String, String> {
    let mut items = std::collections::HashMap::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    let push = |item: &str, items: &mut std::collections::HashMap<String, String>| {
        if let Some((key, value)) = item.split_once(" = ") {
            items.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ',' => {
                push(&current, &mut items);
                current.clear();
            }
            _ => current.push(c),
        }
    }
    push(&current, &mut items);
    items
}

/// A sign-in name NetworkManager can store in a VPN profile's data.
#[must_use]
pub fn valid_vpn_username(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= 128
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ',' | '=' | '\\'))
}

// Default WireGuard routes require peer-routes and an allowed 0.0.0.0/0 or ::/0 peer.
/// [`vpn`] for each connection in one `nmcli -t connection show` of several
/// (blocks start at `connection.uuid`), keyed by UUID; a WireGuard block's
/// peers and peer routes give its split routing as [`wireguard_routes_all`].
pub(super) fn vpns(text: &str) -> std::collections::HashMap<String, Vpn> {
    let mut found = std::collections::HashMap::new();
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line.starts_with("connection.uuid:") {
            blocks.push(Vec::new());
        }
        if let Some(block) = blocks.last_mut() {
            block.push(line);
        }
    }
    for block in blocks {
        let field = |name: &str| {
            block
                .iter()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                .map(str::trim)
        };
        let Some(uuid) = field("connection.uuid").filter(|uuid| !uuid.is_empty()) else {
            continue;
        };
        let mut details = vpn(&block.join("\n"));
        if details.service == VpnService::WireGuard {
            let peers = format!(
                "{}\n{}",
                field("wireguard.peers").unwrap_or_default(),
                field("wireguard.peer-routes").unwrap_or_default()
            );
            let routes_all = wireguard_routes_all(&peers);
            details.split_by_peers = !routes_all;
            details.route_all &= routes_all;
        }
        found.insert(uuid.to_owned(), details);
    }
    found
}

pub(super) fn wireguard_routes_all(text: &str) -> bool {
    let mut lines = text.lines();
    let peers = lines.next().unwrap_or_default();
    let peer_routes = lines.next().map(str::trim) != Some("no");
    let default = peers
        .split_whitespace()
        .filter_map(|field| field.strip_prefix("allowed-ips="))
        .any(|ips| {
            ips.trim_end_matches(',')
                .split(';')
                .any(|ip| matches!(ip.trim(), "0.0.0.0/0" | "::/0"))
        });
    peer_routes && default
}

pub(super) fn adapter(text: &str) -> Option<Adapter> {
    let mut adapter = Adapter {
        name: String::new(),
        powered: false,
        discoverable: false,
    };
    let mut found = false;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "Alias" => adapter.name = value.chars().take(64).collect(),
            "Name" if adapter.name.is_empty() => adapter.name = value.chars().take(64).collect(),
            "Powered" => {
                found = true;
                adapter.powered = value == "yes";
            }
            "Discoverable" => adapter.discoverable = value == "yes",
            _ => {}
        }
    }
    found.then_some(adapter)
}

pub(super) fn device_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, ""));
            (address.len() == 17 && address.bytes().all(|b| b.is_ascii_hexdigit() || b == b':'))
                .then(|| (address.to_owned(), name.trim().chars().take(64).collect()))
        })
        .take(32)
        .collect()
}

pub(super) fn device_icon(info: &str) -> DeviceKind {
    let icon = info
        .lines()
        .find_map(|line| line.trim().strip_prefix("Icon:"))
        .map(str::trim)
        .unwrap_or_default();
    match icon {
        i if i.starts_with("audio-head") => DeviceKind::Headphones,
        i if i.starts_with("audio") => DeviceKind::Speaker,
        "input-keyboard" => DeviceKind::Keyboard,
        "input-mouse" | "input-tablet" => DeviceKind::Mouse,
        "input-gaming" => DeviceKind::Gamepad,
        "phone" => DeviceKind::Phone,
        "computer" => DeviceKind::Computer,
        _ => DeviceKind::Other,
    }
}

// upower durations use fractional minutes or hours.
fn friendly_duration(value: &str) -> Option<String> {
    let mut words = value.split_whitespace();
    let amount: f64 = words
        .next()?
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let minutes = match words.next()? {
        u if u.starts_with("second") => amount / 60.0,
        u if u.starts_with("minute") => amount,
        u if u.starts_with("hour") => amount * 60.0,
        u if u.starts_with("day") => amount * 1440.0,
        _ => return None,
    };
    let minutes = minutes.round().max(1.0) as u64;
    Some(match (minutes / 60, minutes % 60) {
        (0, m) => tr!("shell-min-3", "arg0" => format!("{:}", m)),
        (h, 0) => format!("{h} h"),
        (h, m) => {
            tr!("shell-h-min-2", "arg0" => format!("{:}", h), "arg1" => format!("{:}", m))
        }
    })
}

pub(super) fn upower_battery(text: &str) -> Option<BatteryState> {
    let mut percent = None;
    let mut present = false;
    let mut state = String::new();
    let mut remaining = None;
    let mut battery_kind = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "battery" {
            battery_kind = true;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "present" | "is-present" => present = value == "yes",
            "state" => value.clone_into(&mut state),
            "percentage" => percent = value.trim_end_matches('%').parse::<f32>().ok(),
            "time to empty" | "time to full" => remaining = friendly_duration(value),
            _ => {}
        }
    }
    if !(present || battery_kind) {
        return None;
    }
    let percent = percent.filter(|p| (0.0..=100.0).contains(p))?.round() as u8;
    Some(BatteryState {
        percent,
        charging: matches!(
            state.as_str(),
            "charging" | "fully-charged" | "pending-charge"
        ),
        state: match state.as_str() {
            "charging" => tr("shell-charging"),
            "discharging" => tr("shell-on-battery"),
            "fully-charged" => tr("shell-fully-charged"),
            "pending-charge" => tr("shell-plugged-in-not-charging"),
            _ => tr("shell-battery"),
        }
        .into(),
        remaining,
    })
}

pub(super) fn busctl_string(text: &str) -> Option<String> {
    let value = text.trim().strip_prefix("s ")?.trim().trim_matches('"');
    (!value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    .then(|| value.to_owned())
}

pub(super) fn busctl_profiles(text: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|profile| profile["Profile"]["data"].as_str())
        .filter(|name| name.len() <= 32)
        .map(str::to_owned)
        .take(8)
        .collect()
}

pub(super) fn printers(status: &str, default: &str) -> Vec<Printer> {
    use crate::cups_admin::QueueState;
    let default = default
        .lines()
        .find_map(|line| line.strip_prefix("system default destination:"))
        .map(|name| name.trim().to_owned());
    crate::cups_admin::queues(status)
        .into_iter()
        .map(|(name, state)| Printer {
            default: default.as_deref() == Some(name.as_str()),
            name,
            state: match state {
                QueueState::Idle => PrinterState::Idle,
                QueueState::Printing => PrinterState::Printing,
                QueueState::Paused => PrinterState::Paused,
                QueueState::Offline => PrinterState::Offline,
                QueueState::Stopped => PrinterState::Stopped,
            },
        })
        .collect()
}

pub(super) fn clock(text: &str) -> Clock {
    let mut clock = Clock::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let yes = value == "yes";
        match key {
            "Timezone" => clock.timezone = Some(value.chars().take(64).collect()),
            "NTP" => clock.ntp = Some(yes),
            "CanNTP" => clock.can_ntp = yes,
            "NTPSynchronized" => clock.synchronized = Some(yes),
            "LocalRTC" => clock.local_rtc = yes,
            _ => {}
        }
    }
    clock
}

pub(super) fn os_release(text: &str) -> Option<String> {
    let mut name = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key {
            "PRETTY_NAME" => return Some(value.chars().take(64).collect()),
            "NAME" => name = Some(value.chars().take(64).collect()),
            _ => {}
        }
    }
    name
}

pub(super) fn cpu_model(text: &str) -> Option<String> {
    let model = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name")
                .or_else(|| line.strip_prefix("Model"))
        })?
        .split_once(':')?
        .1
        .trim();
    Some(
        model
            .replace("(R)", "")
            .replace("(TM)", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub(super) fn memory_kib(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub(super) fn gpus(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
            let vendor = quoted
                .get(1)?
                .replace(" Corporation", "")
                .replace(", Inc.", "");
            let device = quoted.get(2)?;
            let device = device
                .split_once('[')
                .map_or(*device, |(_, rest)| rest.trim_end_matches(']'));
            Some(format!("{vendor} {device}"))
        })
        .take(4)
        .collect()
}

/// The `! layout` block of an XKB rules list.
pub fn xkb_layouts(text: &str) -> Vec<(String, String)> {
    let mut layouts = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if let Some(header) = line.strip_prefix('!') {
            inside = header.trim() == "layout";
            continue;
        }
        if !inside {
            continue;
        }
        let line = line.trim();
        let Some((code, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if super::valid_layout(code) {
            layouts.push((code.to_owned(), name.trim().to_owned()));
        }
    }
    layouts.truncate(256);
    layouts
}

pub(super) fn profile(text: &str) -> Profile {
    let mut profile = Profile::default();
    let mut manual = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "connection.zone" => profile.trusted = trusted_zone(value),
            "ipv4.method" if value == "manual" => {
                profile.ipv4.get_or_insert_default();
            }
            "ipv4.addresses" | "ipv4.gateway" => manual.push((
                key == "ipv4.gateway",
                value.split(',').next().unwrap_or("").trim().to_owned(),
            )),
            "ipv4.dns" | "ipv6.dns" => profile.dns.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty() && *v != "--")
                    .map(str::to_owned)
                    .take(4),
            ),
            _ => {}
        }
    }
    if let Some(ipv4) = profile.ipv4.as_mut() {
        for (gateway, value) in manual
            .into_iter()
            .filter(|(_, v)| !v.is_empty() && v != "--")
        {
            if gateway {
                ipv4.gateway = value;
            } else {
                ipv4.address = value;
            }
        }
    }
    profile
}

pub(super) fn discovered(text: &str) -> Vec<Discovered> {
    let mut found: Vec<Discovered> = Vec::new();
    for line in text.lines() {
        let Some((name, uri)) = line.split_once('\t') else {
            continue;
        };
        let uri = uri.trim();
        let Some(name) = crate::cups_admin::display_name(name) else {
            continue;
        };
        if !crate::cups_admin::valid_uri(uri) {
            continue;
        }
        if !found.iter().any(|d| d.name == name) {
            found.push(Discovered {
                name,
                uri: uri.to_owned(),
                driver: None,
                unsupported: false,
            });
        }
    }
    found.truncate(16);
    found
}

/// Printers CUPS's own backends found (`lpinfo -l -v`) that need a driver:
/// USB and plain network (JetDirect, LPD) printers. IPP printers are found
/// as driverless ones instead. Returns (name, uri, device id).
pub(super) fn legacy_devices(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut current: Option<(String, String, String)> = None;
    let flush = |device: Option<(String, String, String)>,
                 out: &mut Vec<(String, String, String)>| {
        if let Some((uri, name, id)) = device
            && crate::cups_admin::valid_device_uri(&uri)
            && !name.is_empty()
            && !out.iter().any(|(_, u, _)| *u == uri)
        {
            out.push((name, uri, id));
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(uri) = line.strip_prefix("Device: uri = ") {
            flush(current.take(), &mut out);
            current = Some((uri.trim().to_owned(), String::new(), String::new()));
        } else if let Some(device) = current.as_mut() {
            if let Some(model) = line.strip_prefix("make-and-model = ") {
                device.1 = crate::cups_admin::display_name(model.trim()).unwrap_or_default();
            } else if let Some(id) = line.strip_prefix("device-id = ") {
                device.2 = id
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(512)
                    .collect();
            }
        }
    }
    flush(current, &mut out);
    out.truncate(16);
    out
}

/// The best driver among `lpinfo -m` lines (`name make-and-model`):
/// Gutenprint's full driver over its simplified one, then the first.
pub(super) fn best_driver(text: &str) -> Option<(String, String)> {
    let drivers: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| line.trim().split_once(' '))
        .map(|(name, model)| (name.to_owned(), model.trim().to_owned()))
        .filter(|(name, _)| {
            !name.is_empty() && name.len() <= 256 && !name.contains(char::is_whitespace)
        })
        // Never the driverless pseudo-driver or raw queues here.
        .filter(|(name, _)| {
            name != "everywhere" && name != "raw" && !name.starts_with("driverless")
        })
        .collect();
    drivers
        .iter()
        .find(|(name, _)| name.ends_with("/expert"))
        .or_else(|| drivers.first())
        .cloned()
}

pub(super) fn valid_zone(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
        && !zone.contains("..")
}
