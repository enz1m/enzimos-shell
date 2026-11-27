//! The Network page's overview from NetworkManager over the system bus: the
//! networking switch, the Wi-Fi radio, the devices and the saved connections
//! that `nmcli` would list, without starting it (each `nmcli` loads every
//! NetworkManager object).

use super::parse::natural_key;
use super::types::{ConnectionKind, Interface, InterfaceKind, LinkState, SavedConnection};
use std::collections::{HashMap, HashSet};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.freedesktop.NetworkManager";

pub(super) struct Overview {
    pub networking: bool,
    pub wifi_radio: Option<bool>,
    pub interfaces: Vec<Interface>,
    pub connections: Vec<SavedConnection>,
}

/// `None` when NetworkManager does not answer; the caller then runs `nmcli`.
pub(super) async fn overview(system: &zbus::Connection) -> Option<Overview> {
    let manager = proxy(system, "/org/freedesktop/NetworkManager", SERVICE).await?;
    let networking = manager
        .get_property::<bool>("NetworkingEnabled")
        .await
        .ok()?;
    let wifi_radio = manager.get_property::<bool>("WirelessEnabled").await.ok();
    let devices: Vec<OwnedObjectPath> = manager.call("GetDevices", &()).await.ok()?;
    let mut interfaces = Vec::new();
    for path in devices.into_iter().take(64) {
        let Some(device) = proxy(
            system,
            path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )
        .await
        else {
            continue;
        };
        let (Ok(name), Ok(kind), Ok(state)) = (
            device.get_property::<String>("Interface").await,
            device.get_property::<u32>("DeviceType").await,
            device.get_property::<u32>("State").await,
        ) else {
            continue;
        };
        let (connection, connection_uuid) = match device
            .get_property::<OwnedObjectPath>("ActiveConnection")
            .await
        {
            Ok(active) if active.as_str() != "/" => {
                match proxy(
                    system,
                    active.as_str(),
                    "org.freedesktop.NetworkManager.Connection.Active",
                )
                .await
                {
                    Some(active) => (
                        active
                            .get_property::<String>("Id")
                            .await
                            .unwrap_or_default(),
                        active
                            .get_property::<String>("Uuid")
                            .await
                            .unwrap_or_default(),
                    ),
                    None => (String::new(), String::new()),
                }
            }
            _ => (String::new(), String::new()),
        };
        if let Some(mut interface) = interface(name, kind, state, connection) {
            interface.connection_uuid = connection_uuid;
            interfaces.push(interface);
        }
        if interfaces.len() == 16 {
            break;
        }
    }
    let active: HashSet<String> = {
        let mut uuids = HashSet::new();
        let paths: Vec<OwnedObjectPath> = manager
            .get_property("ActiveConnections")
            .await
            .unwrap_or_default();
        for path in paths.into_iter().take(64) {
            if let Some(active) = proxy(
                system,
                path.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .await
                // ActiveConnections also includes activating and deactivating profiles.
                && matches!(active.get_property::<u32>("State").await, Ok(2))
                && let Ok(uuid) = active.get_property::<String>("Uuid").await
            {
                uuids.insert(uuid);
            }
        }
        uuids
    };
    let settings = proxy(
        system,
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await?;
    let saved: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).await.ok()?;
    let mut connections = Vec::new();
    for path in saved {
        let Some(profile) = proxy(
            system,
            path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        else {
            continue;
        };
        let Ok(values) = profile
            .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
            .await
        else {
            continue;
        };
        let field = |name: &str| {
            values
                .get("connection")
                .and_then(|section| section.get(name))
                .and_then(|value| String::try_from(value.clone()).ok())
        };
        if let (Some(name), Some(uuid), Some(kind)) = (field("id"), field("uuid"), field("type"))
            && let Some(mut connection) = saved_connection(name, uuid, &kind, &active)
        {
            // The network name is bytes; the Network page lists names as text.
            connection.ssid = values
                .get("802-11-wireless")
                .and_then(|section| section.get("ssid"))
                .and_then(|value| Vec::<u8>::try_from(value.clone()).ok())
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            connections.push(connection);
            if connections.len() == 256 {
                break;
            }
        }
    }
    connections.sort_by_cached_key(|c| (!c.active, natural_key(&c.name)));
    Some(Overview {
        networking,
        wifi_radio,
        interfaces,
        connections,
    })
}

async fn proxy<'a>(
    system: &zbus::Connection,
    path: &str,
    interface: &'static str,
) -> Option<zbus::Proxy<'a>> {
    zbus::proxy::Builder::<zbus::Proxy<'_>>::new(system)
        .destination(SERVICE)
        .ok()?
        .path(path.to_owned())
        .ok()?
        .interface(interface)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

/// A device as [`super::parse::interfaces`] reads `nmcli device status`:
/// the same kinds, the same states, unmanaged and container links left out.
fn interface(device: String, kind: u32, state: u32, connection: String) -> Option<Interface> {
    let kind = match kind {
        1 => InterfaceKind::Ethernet,
        2 => InterfaceKind::Wifi,
        // NM_DEVICE_TYPE_TUN and NM_DEVICE_TYPE_WIREGUARD.
        16 | 29 => InterfaceKind::Vpn,
        _ => return None,
    };
    // NM_DEVICE_STATE_UNMANAGED.
    if state == 10 || (kind == InterfaceKind::Ethernet && device.starts_with("veth")) {
        return None;
    }
    let state = match state {
        100 => LinkState::Connected,
        40..=90 => LinkState::Connecting,
        30 => LinkState::Disconnected,
        _ => LinkState::Unavailable,
    };
    Some(Interface {
        device,
        kind,
        state,
        connection,
        connection_uuid: String::new(),
        details: None,
        profile: None,
    })
}

/// A saved connection as [`super::parse::connections`] reads it.
fn saved_connection(
    name: String,
    uuid: String,
    kind: &str,
    active: &HashSet<String>,
) -> Option<SavedConnection> {
    let kind = match kind {
        "802-11-wireless" => ConnectionKind::Wifi,
        "vpn" | "wireguard" => ConnectionKind::Vpn,
        _ => return None,
    };
    Some(SavedConnection {
        active: active.contains(&uuid),
        name,
        kind,
        uuid,
        ssid: None,
        vpn: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use zbus::zvariant::Str;

    const PATH: &str = "/org/freedesktop/NetworkManager";
    const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
    const PROFILE: &str = "/org/freedesktop/NetworkManager/Settings/1";
    const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
    const UUID: &str = "36ecc274-fb78-416f-bdde-e754549f079f";

    struct Manager;
    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl Manager {
        #[zbus(property)]
        fn networking_enabled(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn active_connections(&self) -> Vec<OwnedObjectPath> {
            vec![OwnedObjectPath::try_from(ACTIVE).unwrap()]
        }
        fn get_devices(&self) -> Vec<OwnedObjectPath> {
            Vec::new()
        }
    }

    struct ConnectionSettings;
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
    impl ConnectionSettings {
        fn list_connections(&self) -> Vec<OwnedObjectPath> {
            vec![OwnedObjectPath::try_from(PROFILE).unwrap()]
        }
    }

    struct VpnProfile;
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
    impl VpnProfile {
        fn get_settings(&self) -> HashMap<&str, HashMap<&str, OwnedValue>> {
            HashMap::from([(
                "connection",
                HashMap::from([
                    ("id", OwnedValue::from(Str::from("Work tunnel"))),
                    ("uuid", OwnedValue::from(Str::from(UUID))),
                    ("type", OwnedValue::from(Str::from("wireguard"))),
                ]),
            )])
        }
    }

    struct Tunnel(Arc<AtomicU32>);
    #[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
    impl Tunnel {
        #[zbus(property)]
        fn uuid(&self) -> &str {
            UUID
        }
        #[zbus(property)]
        fn state(&self) -> u32 {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[tokio::test]
    async fn a_listed_vpn_is_connected_only_when_activated() {
        let bus = crate::test_bus::Bus::start();
        let nm = bus.serve(SERVICE, PATH, Manager).await;
        nm.object_server()
            .at(SETTINGS, ConnectionSettings)
            .await
            .unwrap();
        nm.object_server().at(PROFILE, VpnProfile).await.unwrap();
        let state = Arc::new(AtomicU32::new(1));
        nm.object_server()
            .at(ACTIVE, Tunnel(Arc::clone(&state)))
            .await
            .unwrap();

        let client = bus.client().await;
        for (code, connected) in [(1, false), (2, true), (3, false)] {
            state.store(code, Ordering::Relaxed);
            let overview = overview(&client).await.unwrap();
            let [vpn] = overview.connections.as_slice() else {
                panic!("expected the saved VPN profile");
            };
            assert_eq!(vpn.uuid, UUID);
            assert_eq!(vpn.kind, ConnectionKind::Vpn);
            assert_eq!(vpn.active, connected, "NetworkManager state {code}");
        }
    }
}
