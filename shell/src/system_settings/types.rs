use crate::audio_devices::Devices;
use enzui_shell::i18n::tr;
use enzui_shell::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Display,
    Network,
    Bluetooth,
    Sound,
    Power,
    Printers,
    DateTime,
    About,
    Updates,
    Safety,
    Agents,
    Apps,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Apps {
    pub defaults: crate::default_apps::Choices,
    pub startup: crate::startup_apps::Inventory,
    pub permissions: crate::app_permissions::Permissions,
    /// Standard accounts a guardian can supervise; `None` where the OS has
    /// no parental controls.
    pub supervised: Option<Vec<crate::parental::Account>>,
    /// This account's screen time, with app names for window classes.
    pub screen_time: crate::screen_time::History,
    pub app_names: std::collections::BTreeMap<String, String>,
    /// Screen-sharing grants kept after a restart, with app names.
    pub screen_sharing: crate::screencast_grants::Listing,
    pub screen_sharing_names: std::collections::BTreeMap<String, String>,
}

/// A reading that only exists on enzimOS, through its system backend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Backed<T> {
    #[default]
    Missing,
    Refused(String),
    Ready(T),
}

impl<T> Backed<T> {
    pub fn boxed(self) -> Backed<Box<T>> {
        match self {
            Self::Missing => Backed::Missing,
            Self::Refused(error) => Backed::Refused(error),
            Self::Ready(value) => Backed::Ready(Box::new(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub percent: u8,
    pub muted: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound {
    pub output: Option<Level>,
    pub input: Option<Level>,
    pub devices: Option<Devices>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceKind {
    Ethernet,
    Wifi,
    Vpn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Connected,
    Connecting,
    Disconnected,
    Unavailable,
}
impl LinkState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => tr("shell-connected"),
            Self::Connecting => tr("shell-connecting"),
            Self::Disconnected => tr("shell-disconnected"),
            Self::Unavailable => tr("shell-cable-unplugged"),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub hardware: Option<String>,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub device: String,
    pub kind: InterfaceKind,
    pub state: LinkState,
    pub connection: String,
    /// The active profile's UUID, when NetworkManager reported it: profiles
    /// are changed by it, since a name can belong to several.
    pub connection_uuid: String,
    pub details: Option<Details>,
    /// The active connection's own settings, when it has one.
    pub profile: Option<Profile>,
}
/// Settings the user can change on a saved connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// Trusted (home/work) networks get the firewall's local-network allowance.
    pub trusted: bool,
    /// Custom DNS servers; empty uses the ones the network provides.
    pub dns: Vec<String>,
    /// A fixed IPv4 address; `None` gets one from the network (DHCP).
    pub ipv4: Option<ManualIpv4>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManualIpv4 {
    /// Address with its prefix length, as in `192.168.1.20/24`.
    pub address: String,
    /// Empty for a network without a router.
    pub gateway: String,
}
impl ManualIpv4 {
    /// Checks the address, prefix and gateway, returning the canonical form.
    pub fn validated(&self) -> Result<Self, String> {
        let (ip, prefix) =
            self.address.trim().split_once('/').ok_or_else(|| {
                tr("shell-write-the-address-with-its-prefix-like-192-168-1-20-24")
            })?;
        let ip: std::net::Ipv4Addr = ip
            .trim()
            .parse()
            .map_err(|_| tr!("shell-is-not-an-ipv4-address-2", "arg0" => ip.trim().to_owned()))?;
        let prefix: u8 = prefix
            .trim()
            .parse()
            .ok()
            .filter(|p| (1..=32).contains(p))
            .ok_or_else(|| tr("shell-the-prefix-is-a-number-from-1-to-32"))?;
        let gateway = self.gateway.trim();
        if !gateway.is_empty() {
            let gateway: std::net::Ipv4Addr = gateway
                .parse()
                .map_err(|_| tr!("shell-is-not-an-ipv4-address-2", "arg0" => gateway.to_owned()))?;
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            if u32::from(gateway) & mask != u32::from(ip) & mask {
                return Err(
                    tr("shell-the-gateway-must-be-on-the-same-network-as-the-address").into(),
                );
            }
        }
        Ok(Self {
            address: format!("{ip}/{prefix}"),
            gateway: gateway.to_owned(),
        })
    }
}
pub fn trusted_zone(zone: &str) -> bool {
    matches!(
        zone,
        "home" | "work" | "trusted" | "internal" | "enzimos-trusted"
    )
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    /// WPA Enterprise (802.1X): signs in with a username and password.
    pub enterprise: bool,
    pub active: bool,
    pub saved: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Wifi,
    Vpn,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedConnection {
    pub name: String,
    pub kind: ConnectionKind,
    pub active: bool,
    /// The profile's identity: names can repeat, a UUID cannot.
    pub uuid: String,
    /// A Wi-Fi profile's network name, which need not be its own name;
    /// `None` when it was not read (the `nmcli` fallback lists names only).
    pub ssid: Option<String>,
    /// A VPN's provider type and routing, once read.
    pub vpn: Option<Vpn>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnService {
    WireGuard,
    OpenVpn,
    OpenConnect,
    Vpnc,
    L2tp,
    Ikev2,
    Pptp,
    Other,
}
impl VpnService {
    pub fn label(self) -> &'static str {
        match self {
            Self::WireGuard => "WireGuard",
            Self::OpenVpn => "OpenVPN",
            Self::OpenConnect => enzui_shell::i18n::tr("shell-cisco-compatible-openconnect"),
            Self::Vpnc => enzui_shell::i18n::tr("shell-vpn-kind-vpnc"),
            Self::L2tp => enzui_shell::i18n::tr("shell-vpn-kind-l2tp"),
            Self::Ikev2 => enzui_shell::i18n::tr("shell-vpn-kind-ikev2"),
            Self::Pptp => enzui_shell::i18n::tr("shell-vpn-kind-pptp"),
            Self::Other => "VPN",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vpn {
    pub service: VpnService,
    /// All traffic goes through the tunnel (rather than only its networks).
    pub route_all: bool,
    pub metered: bool,
    /// A WireGuard profile whose peers carry only their own networks: the
    /// provider's AllowedIPs, not a switch here, decide that.
    pub split_by_peers: bool,
    /// OpenVPN password sign-in: outer None means unavailable; inner None means no username is
    /// saved.
    pub sign_in: Option<Option<String>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Network {
    /// `None` when NetworkManager is not available.
    pub networking: Option<bool>,
    pub firewall: Option<Firewall>,
    pub wifi_radio: Option<bool>,
    pub interfaces: Vec<Interface>,
    pub access_points: Vec<AccessPoint>,
    pub vpns: Vec<SavedConnection>,
    /// Saved Wi-Fi profiles.
    pub wifi: Vec<SavedConnection>,
    pub connectivity: crate::network_connectivity::Connectivity,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Firewall {
    pub running: bool,
    pub default_zone: String,
}
impl Network {
    pub fn airplane(&self) -> bool {
        self.wifi_radio == Some(false)
    }
    pub fn has_wifi(&self) -> bool {
        self.interfaces
            .iter()
            .any(|i| i.kind == InterfaceKind::Wifi)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub powered: bool,
    pub discoverable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Computer,
    Other,
}
impl DeviceKind {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Headphones => "headphones",
            Self::Speaker => "speaker",
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Gamepad => "gamepad-2",
            Self::Phone => "smartphone",
            Self::Computer => "laptop",
            Self::Other => "bluetooth",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub kind: DeviceKind,
    pub paired: bool,
    pub connected: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bluetooth {
    /// `None` when no adapter is present.
    pub adapter: Option<Adapter>,
    pub devices: Vec<BluetoothDevice>,
    /// Unpaired devices seen by the last search.
    pub nearby: Vec<BluetoothDevice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatteryState {
    pub percent: u8,
    pub charging: bool,
    pub state: String,
    pub remaining: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profiles {
    pub active: String,
    pub available: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Power {
    pub thermal: Option<crate::thermal::Level>,
    pub battery: Option<BatteryState>,
    pub game_mode: Option<crate::game_mode::Status>,
    pub game_preferences: crate::game_mode::preferences::Preferences,
    /// A laptop lid is present.
    pub lid: bool,
    /// `None` when neither power-profiles-daemon nor its UPower successor runs.
    pub profiles: Option<Profiles>,
    /// enzimOS: what UPower does when the battery is critically low.
    pub critical: Backed<CriticalBattery>,
    /// enzimOS: the memory exhaustion guard and whether it really runs.
    pub memory: Backed<MemoryPolicy>,
    pub tmp_storage: Backed<TemporaryStorage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryStorage {
    pub active: String,
    pub configured: Option<String>,
}

/// enzimOS `memory-policy`: the configured preset and the guard's actual
/// state, read from the running service rather than its configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryPolicy {
    pub mode: String,
    pub available: Vec<String>,
    /// The service's `ActiveState` (`active`, `inactive`, `failed`, …).
    pub active_state: String,
    /// The preset the running guard was started with.
    pub running_mode: Option<String>,
    pub memory_bytes: Option<u64>,
    pub protected_names: Option<Vec<String>>,
}

pub const MEMORY_MODES: [&str; 3] = ["balanced", "responsive", "off"];

pub fn memory_label(mode: &str) -> &str {
    match mode {
        "balanced" => tr("shell-balanced"),
        "responsive" => tr("shell-more-responsive"),
        "off" => tr("shell-off"),
        other => other,
    }
}

pub fn memory_detail(mode: &str) -> &'static str {
    match mode {
        "balanced" => tr("shell-recommended-closes-an-app-only-when-memory-is-nearly-exhausted"),
        "responsive" => {
            tr("shell-acts-sooner-so-the-desktop-stays-smoother-but-may-close-a-large-app-earlier")
        }
        "off" => {
            tr("shell-nothing-steps-in-before-the-kernel-does-the-desktop-may-become-unresponsive")
        }
        _ => "",
    }
}

/// enzimOS `power-policy`: the configured critical battery action and the
/// ones this computer can use. A change applies at the next boot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CriticalBattery {
    pub configured: String,
    pub supported: Vec<String>,
    /// Actions the user must explicitly accept the risk of.
    pub risky: Vec<String>,
}

pub const CRITICAL_ACTIONS: [&str; 5] = ["auto", "suspend", "hibernate", "power-off", "ignore"];

pub fn critical_label(action: &str) -> &str {
    match action {
        "auto" => tr("shell-automatic"),
        "suspend" => tr("shell-suspend"),
        "hibernate" => tr("shell-hibernate"),
        "power-off" => tr("shell-power-off-3"),
        "ignore" => tr("shell-do-nothing"),
        other => other,
    }
}

/// The question and the accepting button for a risky action.
pub fn critical_confirmation(action: &str) -> (&'static str, &'static str) {
    match action {
        "suspend" => (
            tr("shell-suspend-when-the-battery-is-almost-empty"),
            tr("shell-i-understand-suspend"),
        ),
        "ignore" => (
            tr("shell-do-nothing-when-the-battery-is-almost-empty"),
            tr("shell-i-understand-do-nothing"),
        ),
        _ => (
            tr("shell-use-this-action-when-the-battery-is-almost-empty"),
            tr("shell-i-understand"),
        ),
    }
}

/// Why a risky critical battery action needs the user's acknowledgement.
pub fn critical_risk(action: &str) -> &'static str {
    match action {
        "suspend" => {
            tr("shell-a-suspended-computer-keeps-your-work-only-in-memory-if-the-battery-runs-out")
        }
        "ignore" => {
            tr("shell-nothing-protects-the-computer-it-can-switch-off-without-warning-when-the-bat")
        }
        _ => tr("shell-this-choice-carries-a-risk-to-unsaved-work"),
    }
}
pub fn profile_label(profile: &str) -> &str {
    match profile {
        "power-saver" => tr("shell-power-saver"),
        "balanced" => tr("shell-balanced"),
        "performance" => tr("shell-performance"),
        other => other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrinterState {
    Idle,
    Printing,
    Paused,
    Offline,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub state: PrinterState,
    pub default: bool,
}
/// What the MCP Center broker reports; `available` is false when this
/// account has no broker or no control credential.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Agents {
    pub available: bool,
    pub error: Option<String>,
    pub identities: Vec<crate::agent_center::Credential>,
    pub permissions: Vec<crate::agent_center::Permission>,
    pub activity: Vec<crate::agent_center::Activity>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printers {
    /// `None` when CUPS is not installed; `Some(false)` when it is stopped.
    pub scheduler: Option<bool>,
    pub printers: Vec<Printer>,
    /// This account's unfinished jobs; other accounts' jobs are not read.
    pub jobs: Vec<crate::cups_admin::Job>,
    /// cups-pk-helper is there to add, remove, pause and resume queues.
    pub admin: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    pub zones: Vec<String>,
    pub timezone: Option<String>,
    pub ntp: Option<bool>,
    pub can_ntp: bool,
    pub synchronized: Option<bool>,
    pub local_rtc: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    pub hostname: Option<String>,
    /// The Wayland compositor the session runs on.
    pub compositor: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub cpu: Option<String>,
    pub threads: Option<usize>,
    pub memory_kib: Option<u64>,
    pub gpus: Vec<String>,
    pub disk: Option<(u64, u64)>,
    /// What the enzimOS backend reports, on enzimOS.
    pub enzimos: Option<super::enzimos::SystemState>,
    /// Recent crashes of this account's apps; `None` without the OS helper.
    pub crashes: Option<std::sync::Arc<Vec<crate::crash_reports::Report>>>,
}

/// The account and server check for joining a WPA Enterprise network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enterprise {
    pub identity: String,
    pub server: ServerCheck,
}

/// How a WPA Enterprise join recognises the network's sign-in server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerCheck {
    /// A certificate from a system-trusted authority that names this domain.
    Domain(String),
    /// A certificate from the authority in this file that names this
    /// domain, when one is given.
    Authority { file: String, domain: String },
    /// Any server; a look-alike network can collect the password.
    Unchecked,
}

/// A password or key on its way to a system tool; never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub String);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// A printer found on the network that is not yet added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovered {
    pub name: String,
    pub uri: String,
    /// The CUPS driver for a printer that does not print driverless (an
    /// older USB or network printer), and its name.
    pub driver: Option<(String, String)>,
    /// An older printer for which no installed driver fits.
    pub unsupported: bool,
}
