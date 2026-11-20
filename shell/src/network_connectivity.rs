//! NetworkManager connectivity status and explicit captive-portal sign-in.
//! Sign-in uses the configured probe URL, never a network-supplied redirect.

use enzui_shell::i18n::tr;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Unknown,
    None,
    Portal,
    Limited,
    Full,
}

impl State {
    /// From NetworkManager's `NMConnectivityState`.
    #[must_use]
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => Self::None,
            2 => Self::Portal,
            3 => Self::Limited,
            4 => Self::Full,
            _ => Self::Unknown,
        }
    }
    /// From `nmcli -t -f CONNECTIVITY general`.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        match word.trim() {
            "none" => Self::None,
            "portal" => Self::Portal,
            "limited" => Self::Limited,
            "full" => Self::Full,
            _ => Self::Unknown,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => tr("shell-not-checked"),
            Self::None => tr("shell-no-network"),
            Self::Portal => tr("shell-sign-in-required"),
            Self::Limited => tr("shell-no-internet-access"),
            Self::Full => tr("shell-connected-to-the-internet"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connectivity {
    pub state: State,
    /// Whether the check runs; `None` when no check is configured.
    pub checks: Option<bool>,
    /// The configured probe URL (http or https only), for signing in.
    pub uri: Option<String>,
    /// The wired and wireless devices, read with the connectivity so the
    /// network shows its changes as they happen; `None` when unread.
    pub devices: Option<Devices>,
}

/// What the network's devices are doing, as NetworkManager reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub ethernet: bool,
    pub ethernet_present: bool,
    pub wifi_present: bool,
    pub wifi_connected: bool,
    pub wifi_enabled: Option<bool>,
}

impl Connectivity {
    /// The page a person signs in through: the probe URL, only in the portal
    /// state, and only an http(s) address.
    #[must_use]
    pub fn sign_in(&self) -> Option<&str> {
        (self.state == State::Portal).then_some(())?;
        self.uri.as_deref().filter(|uri| valid_uri(uri))
    }
}

#[must_use]
pub fn valid_uri(uri: &str) -> bool {
    (uri.starts_with("http://") || uri.starts_with("https://"))
        && uri.len() <= 2048
        && !uri.chars().any(|c| c.is_control() || c.is_whitespace())
}

const SERVICE: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager";

/// NetworkManager's devices and Wi-Fi radio over D-Bus, without starting
/// `nmcli`; `None` when NetworkManager does not answer.
pub async fn devices(system: &zbus::Connection) -> Option<Devices> {
    let proxy = |path: zbus::zvariant::OwnedObjectPath, interface: &'static str| {
        zbus::proxy::Builder::<zbus::Proxy<'_>>::new(system)
            .destination(SERVICE)
            .and_then(|b| b.path(path))
            .and_then(|b| b.interface(interface))
            .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
    };
    let manager = proxy(
        zbus::zvariant::OwnedObjectPath::try_from(PATH).ok()?,
        SERVICE,
    )
    .ok()?
    .build()
    .await
    .ok()?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = manager.call("GetDevices", &()).await.ok()?;
    let mut devices = Devices {
        wifi_enabled: manager.get_property::<bool>("WirelessEnabled").await.ok(),
        ..Devices::default()
    };
    for path in paths.into_iter().take(64) {
        let Ok(device) = proxy(path, "org.freedesktop.NetworkManager.Device")
            .ok()?
            .build()
            .await
        else {
            continue;
        };
        let (Ok(kind), Ok(state)) = (
            device.get_property::<u32>("DeviceType").await,
            device.get_property::<u32>("State").await,
        ) else {
            continue;
        };
        // NM_DEVICE_STATE_ACTIVATED, which nmcli shows as "connected".
        let connected = state == 100;
        match kind {
            // NM_DEVICE_TYPE_ETHERNET
            1 => {
                devices.ethernet_present = true;
                devices.ethernet |= connected;
            }
            // NM_DEVICE_TYPE_WIFI
            2 => {
                devices.wifi_present = true;
                devices.wifi_connected |= connected;
            }
            _ => {}
        }
    }
    Some(devices)
}

/// NetworkManager's connectivity once, over the system bus; `None` when it
/// does not answer.
pub async fn current(system: &zbus::Connection) -> Option<Connectivity> {
    let proxy = zbus::Proxy::new(system, SERVICE, PATH, SERVICE)
        .await
        .ok()?;
    proxy.get_property::<u32>("Connectivity").await.ok()?;
    Some(read(&proxy).await)
}

async fn read(proxy: &zbus::Proxy<'_>) -> Connectivity {
    let state = proxy
        .get_property::<u32>("Connectivity")
        .await
        .map(State::from_code)
        .unwrap_or_default();
    let available = proxy
        .get_property::<bool>("ConnectivityCheckAvailable")
        .await
        .unwrap_or(false);
    let enabled = proxy
        .get_property::<bool>("ConnectivityCheckEnabled")
        .await
        .ok();
    let uri = proxy
        .get_property::<String>("ConnectivityCheckUri")
        .await
        .ok()
        .filter(|uri| valid_uri(uri));
    Connectivity {
        state,
        checks: if available { enabled } else { None },
        uri,
        devices: devices(proxy.connection()).await,
    }
}

/// NetworkManager's connectivity as it changes: one value when it starts,
/// then one per change (bursts coalesced), nothing while it is unavailable.
pub fn subscription() -> iced::Subscription<Connectivity> {
    use iced::futures::{SinkExt, StreamExt};
    iced::Subscription::run(|| {
        iced::stream::channel(
            4,
            async move |mut output: iced::futures::channel::mpsc::Sender<Connectivity>| {
                loop {
                    let watched = async {
                        let system = zbus::Connection::system().await.ok()?;
                        let proxy = zbus::Proxy::new(&system, SERVICE, PATH, SERVICE)
                            .await
                            .ok()?;
                        let mut changes = proxy.receive_all_signals().await.ok()?;
                        let properties = zbus::fdo::PropertiesProxy::builder(&system)
                            .destination(SERVICE)
                            .ok()?
                            .path(PATH)
                            .ok()?
                            .build()
                            .await
                            .ok()?;
                        let mut changed = properties.receive_properties_changed().await.ok()?;
                        let mut last = read(&proxy).await;
                        let _ = output.send(last.clone()).await;
                        loop {
                            tokio::select! {
                                signal = changed.next() => { signal?; }
                                signal = changes.next() => { signal?; }
                            }
                            // A connection change sends several updates at once.
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            let now = read(&proxy).await;
                            if now != last {
                                last = now.clone();
                                let _ = output.send(now).await;
                            }
                        }
                    };
                    let _: Option<()> = watched.await;
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            },
        )
    })
}

/// Parses `busctl get-property` output (`b true`, `s "http://…"`).
#[must_use]
pub fn busctl_value(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(value) = text.strip_prefix("b ") {
        return Some(value.trim().to_owned());
    }
    if let Some(value) = text.strip_prefix("s ") {
        let value = value.trim();
        return value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .map(|v| v.replace("\\\"", "\""));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_is_offered_only_for_a_portal_and_a_web_address() {
        let mut connectivity = Connectivity {
            state: State::Portal,
            checks: Some(true),
            uri: Some("http://ping.archlinux.org/nm-check.txt".into()),
            devices: None,
        };
        assert_eq!(
            connectivity.sign_in(),
            Some("http://ping.archlinux.org/nm-check.txt")
        );
        connectivity.state = State::Limited;
        assert_eq!(
            connectivity.sign_in(),
            None,
            "limited is not a definite portal"
        );
        connectivity.state = State::Portal;
        for uri in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://a b",
            "steam://run/1",
        ] {
            connectivity.uri = Some(uri.into());
            assert_eq!(connectivity.sign_in(), None, "{uri}");
        }
        assert_eq!(State::from_code(2), State::Portal);
        assert_eq!(State::from_word("limited\n"), State::Limited);
        assert_eq!(busctl_value("b true\n").as_deref(), Some("true"));
        assert_eq!(
            busctl_value("s \"http://x/y\"").as_deref(),
            Some("http://x/y")
        );
        assert_eq!(busctl_value("u 4"), None);
    }
}
