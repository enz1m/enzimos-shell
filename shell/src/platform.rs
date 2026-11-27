//! What this computer offers the desktop: whether it is enzimOS, and which
//! optional system services are installed. Settings (its sidebar, its
//! search and launch requests) and the shell's quick search show a page or
//! card only where its backend is present.
//!
//! On enzimOS (`ID=enzimos` in `os-release`, or `org.enzimos.System1`
//! answering) every page shows as it always has. Elsewhere a page whose
//! backend is missing is hidden, and the enzimOS-only pages are hidden.
//! Every question asked here is bounded, so a stuck service only makes its
//! feature count as absent.
//!
//! Flatpak (app permissions) and Fcitx5 (input methods) are found where
//! they are used: their pages stay and say what is missing.

use crate::system_controls::SystemControls;
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long one bus question may take.
const PROBE: Duration = Duration::from_secs(1);
/// How long a detection is reused before the next caller detects again.
const FRESH: Duration = Duration::from_secs(30);

/// The firewall service that is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Firewall {
    #[default]
    None,
    Firewalld,
    /// Another firewall's service is active, by name.
    Other(&'static str),
}

/// Firewall services other than firewalld, by unit and name.
const OTHER_FIREWALLS: [(&str, &str); 3] = [
    ("ufw.service", "ufw"),
    ("nftables.service", "nftables"),
    ("iptables.service", "iptables"),
];

/// Network services that manage connections instead of NetworkManager: the
/// bus name each owns while it runs, and its name.
const NETWORK_SERVICES: [(&str, &str); 3] = [
    ("net.connman.iwd", "iwd"),
    ("org.freedesktop.network1", "systemd-networkd"),
    ("net.connman", "ConnMan"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Platform {
    /// `os-release` names enzimOS.
    pub enzimos: bool,
    /// `org.enzimos.System1` answered.
    pub system1: bool,
    /// NetworkManager runs on the system bus.
    pub network_manager: bool,
    /// What manages the network instead, while NetworkManager does not.
    pub network_services: Vec<&'static str>,
    pub firewall: Firewall,
    /// `firewall-cmd` is installed.
    pub firewalld_installed: bool,
    /// BlueZ on the system bus, and `bluetoothctl`.
    pub bluetooth: bool,
    /// PipeWire's `wpctl`.
    pub pipewire: bool,
    /// PulseAudio's `pactl`, which also serves PipeWire through pipewire-pulse.
    pub pulseaudio: bool,
    /// power-profiles-daemon or tuned-ppd.
    pub power_profiles: bool,
    /// CUPS (`lpstat`).
    pub cups: bool,
    pub upower: bool,
    /// The enzimOS background app service.
    pub background_apps: bool,
    /// Feral GameMode on the session bus.
    pub game_mode: bool,
    /// The enzimOS on-device AI service's model tool.
    pub local_ai: bool,
    /// A Thunderbolt or USB4 controller, with bolt to approve devices.
    pub thunderbolt: bool,
    /// UDisks2, which manages disks and partitions.
    pub disks: bool,
}

/// A page, card or control that needs a backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// What the enzimOS system service provides: updates, backups and
    /// recovery, developer tools, remote access, and the critical battery
    /// and memory protection policies.
    SystemService,
    Network,
    /// Public and trusted networks: firewalld zones.
    NetworkTrust,
    /// Turning firewalld on and off.
    FirewallSwitch,
    Bluetooth,
    PhoneLink,
    /// Choosing the sound output and input device.
    AudioDevices,
    PowerModes,
    Printers,
    BackgroundApps,
    GameMode,
    /// Adding an on-device AI model.
    LocalAiModels,
    /// Hardware: a Thunderbolt controller (shown only where there is one).
    Thunderbolt,
    Disks,
}

impl Platform {
    /// Everything offered: the sample pages of Settings' demo mode.
    #[must_use]
    pub fn everything() -> Self {
        Self {
            enzimos: true,
            system1: true,
            thunderbolt: true,
            disks: true,
            ..Self::default()
        }
    }

    /// Whether this is enzimOS, where every page shows as it always has.
    #[must_use]
    pub fn is_enzimos(&self) -> bool {
        self.enzimos || self.system1
    }

    #[must_use]
    pub fn offers(&self, feature: Feature) -> bool {
        match feature {
            Feature::SystemService => self.system1,
            // Hardware, even on enzimOS.
            Feature::Thunderbolt => self.thunderbolt,
            _ if self.is_enzimos() => true,
            Feature::Network => self.network_manager,
            Feature::NetworkTrust => self.firewall == Firewall::Firewalld,
            // Turning firewalld on is offered only while no other firewall runs.
            Feature::FirewallSwitch => {
                self.firewall == Firewall::Firewalld
                    || (self.firewall == Firewall::None && self.firewalld_installed)
            }
            Feature::Bluetooth => self.bluetooth,
            Feature::PhoneLink => crate::phone_link::available(),
            Feature::AudioDevices => self.pipewire || self.pulseaudio,
            Feature::PowerModes => self.power_profiles,
            Feature::Printers => self.cups,
            Feature::BackgroundApps => self.background_apps,
            Feature::GameMode => self.game_mode,
            Feature::LocalAiModels => self.local_ai,
            Feature::Disks => self.disks,
        }
    }

    /// The network services in use, for About, where NetworkManager's pages
    /// are hidden; `None` where the Network page shows.
    #[must_use]
    pub fn network_elsewhere(&self) -> Option<&[&'static str]> {
        (!self.offers(Feature::Network)).then_some(self.network_services.as_slice())
    }
}

/// What a Settings route (a whole page such as `network`, or a setting such
/// as `power/mode`) needs.
#[must_use]
pub fn features(route: &str) -> &'static [Feature] {
    match route {
        "network/firewall" => &[Feature::Network, Feature::FirewallSwitch],
        "sound/output" | "sound/input" => &[Feature::AudioDevices],
        "power/mode" => &[Feature::PowerModes],
        "power/critical" | "power/memory" | "power/temporary_storage" => &[Feature::SystemService],
        "power/game_mode" | "power/game_priority" | "controls/game_mode" => &[Feature::GameMode],
        "apps/background" => &[Feature::BackgroundApps],
        _ => match route.split('/').next().unwrap_or(route) {
            "network" => &[Feature::Network],
            "bluetooth" => &[Feature::Bluetooth],
            "phone" => &[Feature::PhoneLink],
            "printers" => &[Feature::Printers],
            "thunderbolt" => &[Feature::Thunderbolt],
            "disks" => &[Feature::Disks],
            "updates" | "backups" | "developer" | "remote" => &[Feature::SystemService],
            _ => &[],
        },
    }
}

/// Whether `route` is shown. Until the platform is known, only the enzimOS
/// system service's pages wait for it.
#[must_use]
pub fn shows(platform: Option<&Platform>, route: &str) -> bool {
    features(route).iter().all(|feature| match platform {
        Some(platform) => platform.offers(*feature),
        None => *feature != Feature::SystemService,
    })
}

static LATEST: Mutex<Option<(Instant, Platform)>> = Mutex::new(None);

/// The platform detected last in this process, if any.
#[must_use]
pub fn latest() -> Option<Platform> {
    LATEST
        .lock()
        .ok()
        .and_then(|latest| latest.as_ref().map(|(_, platform)| platform.clone()))
}

fn remember(platform: &Platform) {
    if let Ok(mut latest) = LATEST.lock() {
        *latest = Some((Instant::now(), platform.clone()));
    }
}

/// The platform, detected again when the last detection is older than 30 s.
/// Once `org.enzimos.System1` has answered it is not asked again.
pub async fn current() -> Platform {
    let previous = LATEST.lock().ok().and_then(|latest| latest.clone());
    if let Some((at, platform)) = &previous
        && at.elapsed() < FRESH
    {
        return platform.clone();
    }
    let system1 = previous.is_some_and(|(_, platform)| platform.system1)
        || tokio::time::timeout(
            Duration::from_secs(3),
            crate::system_settings::enzimos::Backend::connect(),
        )
        .await
        .ok()
        .flatten()
        .is_some();
    detect(&SystemControls::from_environment(), system1).await
}

/// Detects the platform now; `system1` is whether `org.enzimos.System1`
/// answered, which the caller has already asked.
pub async fn detect(controls: &SystemControls, system1: bool) -> Platform {
    let (system, session) = tokio::join!(
        tokio::time::timeout(PROBE, crate::bus::system()),
        tokio::time::timeout(PROBE, crate::bus::session()),
    );
    let os_release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok();
    let platform = detect_on(
        controls,
        system.ok().flatten(),
        session.ok().flatten(),
        os_release.as_deref(),
        system1,
    )
    .await;
    remember(&platform);
    platform
}

/// The names a bus has owners for, and the names it can start.
async fn names(connection: Option<zbus::Connection>) -> (BTreeSet<String>, BTreeSet<String>) {
    let Some(connection) = connection else {
        return Default::default();
    };
    let Ok(Ok(bus)) = tokio::time::timeout(PROBE, zbus::fdo::DBusProxy::new(&connection)).await
    else {
        return Default::default();
    };
    let (owned, activatable) = tokio::join!(
        tokio::time::timeout(PROBE, bus.list_names()),
        tokio::time::timeout(PROBE, bus.list_activatable_names()),
    );
    let set = |names: Option<Vec<zbus::names::OwnedBusName>>| {
        names
            .unwrap_or_default()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<BTreeSet<_>>()
    };
    (
        set(owned.ok().and_then(Result::ok)),
        set(activatable.ok().and_then(Result::ok)),
    )
}

fn os_release_id(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("ID="))
        .map(|id| id.trim().trim_matches(['"', '\'']))
}

/// The firewall service that is active now, asked of systemd.
pub async fn firewall(controls: &SystemControls) -> Firewall {
    let active = |unit: &'static str| async move {
        controls
            .run("systemctl", &["is-active", unit])
            .await
            .is_ok_and(|state| state.trim() == "active")
    };
    let (firewalld, ufw, nftables, iptables) = tokio::join!(
        active("firewalld.service"),
        active(OTHER_FIREWALLS[0].0),
        active(OTHER_FIREWALLS[1].0),
        active(OTHER_FIREWALLS[2].0),
    );
    if firewalld {
        return Firewall::Firewalld;
    }
    [ufw, nftables, iptables]
        .into_iter()
        .zip(OTHER_FIREWALLS)
        .find_map(|(active, (_, name))| active.then_some(Firewall::Other(name)))
        .unwrap_or_default()
}

pub(crate) async fn detect_on(
    controls: &SystemControls,
    system: Option<zbus::Connection>,
    session: Option<zbus::Connection>,
    os_release: Option<&str>,
    system1: bool,
) -> Platform {
    // enzimOS offers every page; nothing else needs asking there.
    let enzimos = os_release.and_then(os_release_id) == Some("enzimos");
    let thunderbolt = crate::thunderbolt::present();
    if enzimos || system1 {
        return Platform {
            enzimos,
            system1,
            thunderbolt,
            disks: true,
            ..Platform::default()
        };
    }
    let ((system_owned, system_startable), (session_owned, session_startable), firewall) =
        tokio::join!(names(system), names(session), firewall(controls));
    let on_system = |name: &str| system_owned.contains(name) || system_startable.contains(name);
    let on_session = |name: &str| session_owned.contains(name) || session_startable.contains(name);
    let tool = |name: &str| controls.tool(name).is_some();
    let network_manager = system_owned.contains("org.freedesktop.NetworkManager");
    Platform {
        enzimos,
        system1,
        network_manager,
        network_services: NETWORK_SERVICES
            .iter()
            .filter(|(name, _)| system_owned.contains(*name))
            .map(|(_, service)| *service)
            .collect(),
        firewall,
        firewalld_installed: tool("firewall-cmd"),
        bluetooth: on_system("org.bluez") && tool("bluetoothctl"),
        pipewire: tool("wpctl"),
        pulseaudio: tool("pactl"),
        power_profiles: on_system("org.freedesktop.UPower.PowerProfiles")
            || on_system("net.hadess.PowerProfiles"),
        cups: tool("lpstat"),
        upower: on_system("org.freedesktop.UPower"),
        background_apps: on_session("org.enzimos.Background1"),
        game_mode: on_session("com.feralinteractive.GameMode"),
        local_ai: tool("enzimos-inference-models"),
        thunderbolt: thunderbolt && on_system("org.freedesktop.bolt"),
        disks: on_system("org.freedesktop.UDisks2"),
    }
}
