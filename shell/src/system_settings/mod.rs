//! Read-mostly system settings with missing tools reported as unavailable.
//! Host mutations beyond audio require explicit permission through writes_allowed.

use enzui_shell::i18n::tr;
mod actions;
pub mod backup_recovery;
pub mod backup_remote;
pub mod backup_setup;
pub mod backups;
mod demo;
pub mod display;
pub mod enzimos;
pub mod files_shares;
mod nm_bus;
pub mod pairing;
mod parse;
pub mod phone_network;
mod read;
mod types;

pub use actions::printer_queue_name;
pub use parse::{valid_vpn_username, xkb_layouts};
pub use types::*;

use crate::system_controls::SystemControls;
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    Display(display::Displays),
    Network(Network),
    Bluetooth(Bluetooth),
    Sound(Sound),
    Power(Power),
    Printers(Printers),
    DateTime(Clock),
    About(Box<About>),
    Updates(Backed<enzimos::SystemState>),
    Safety(Backed<Box<enzimos::Safety>>),
    Agents(Agents),
    Apps(Apps),
}

impl Reading {
    /// Preserve the last valid reading when a busy backend refuses refresh.
    #[must_use]
    pub fn refused_over(&self, previous: &Reading) -> bool {
        matches!(
            (self, previous),
            (
                Reading::Updates(Backed::Refused(_)),
                Reading::Updates(Backed::Ready(_))
            ) | (
                Reading::Safety(Backed::Refused(_)),
                Reading::Safety(Backed::Ready(_))
            )
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Display(Vec<display::Change>),
    OutputVolume(u8),
    OutputMute(bool),
    InputVolume(u8),
    InputMute(bool),
    AudioDevice(crate::audio_devices::Device),
    WifiRadio(bool),
    Vpn(String, bool),
    VpnRemove(String),
    VpnRouteAll {
        uuid: String,
        all: bool,
    },
    VpnMetered {
        uuid: String,
        metered: bool,
    },
    VpnUsername {
        uuid: String,
        name: String,
    },
    ConnectivityCheck,
    ConnectivityChecks(bool),
    /// Opens the captive portal's sign-in page (the configured probe URL).
    PortalSignIn,
    WifiJoin {
        ssid: String,
        password: Option<Secret>,
        hidden: bool,
        enterprise: Option<Enterprise>,
    },
    Forget(String),
    WifiDisconnect,
    Airplane(bool),
    Trust {
        connection: String,
        trusted: bool,
    },
    Dns {
        connection: String,
        servers: Vec<String>,
    },
    Ipv4 {
        connection: String,
        manual: Option<ManualIpv4>,
    },
    VpnImport(String),
    /// A VPN added by hand from the Settings form.
    VpnAdd(Box<crate::vpn_manual::Spec>),
    Firewall(bool),
    Bluetooth(bool),
    BluetoothDevice(String, bool),
    BluetoothSearch,
    BluetoothPair(String),
    BluetoothRemove(String),
    PowerProfile(String),
    GameModeOption(crate::game_mode::preferences::OptionKey, u8),
    DefaultPrinter(String),
    PrinterAdd(Discovered),
    PrinterRemove(String),
    PrinterTest(String),
    /// Pauses (`true`) or resumes a queue, through cups-pk-helper.
    PrinterPause(String, bool),
    /// Cancels one of this account's own jobs (`queue-id`).
    JobCancel(String),
    Ntp(bool),
    Timezone(String),
    Hostname(String),
    HomeHistory(bool),
    BackupSchedule(bool),
    /// enzimOS: the critical battery action from the next boot.
    /// `risk_acknowledged` only from the user's own confirmation.
    CriticalBattery {
        action: String,
        risk_acknowledged: bool,
    },
    MemoryPolicy {
        mode: String,
        risk_acknowledged: bool,
    },
    TemporaryStorage(String),
    AgentRevokeIdentity(String),
    AgentRevokePermission {
        agent: String,
        app: String,
        function: String,
    },
    AgentClearActivity,
    DefaultApp {
        kind: crate::default_apps::Kind,
        id: String,
    },
    StartupApp {
        id: String,
        review: crate::startup_apps::Review,
        enabled: bool,
        adding: bool,
    },
    AppPermission {
        app: String,
        kind: crate::app_permissions::Kind,
        allowed: bool,
    },
    /// Forgets a screen-sharing grant kept after a restart, by its id.
    ScreenSharingRevoke(String),
}

/// enzimOS changes that can run for minutes; they run beside the request
/// queue so other settings stay usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LongJob {
    BackupNow,
    InstallUpdates,
}

impl Request {
    /// Requests that start long tool runs; the UI shows them as progress.
    pub fn slow(&self) -> bool {
        matches!(
            self,
            Self::WifiJoin { .. }
                | Self::BluetoothSearch
                | Self::BluetoothPair(_)
                | Self::PrinterAdd(_)
        )
    }
    /// Audio and compositor-local displays may change without the host-system opt-in.
    pub fn audio(&self) -> bool {
        // The broker is the account's own service, so its changes are the
        // user's as well.
        matches!(
            self,
            Self::Display(_)
                | Self::OutputVolume(_)
                | Self::OutputMute(_)
                | Self::InputVolume(_)
                | Self::InputMute(_)
                | Self::AudioDevice(_)
                | Self::AgentRevokeIdentity(_)
                | Self::AgentRevokePermission { .. }
                | Self::AgentClearActivity
                | Self::DefaultApp { .. }
                | Self::AppPermission { .. }
                | Self::ScreenSharingRevoke(_)
        )
    }
    pub fn same_slot(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::AudioDevice(a), Self::AudioDevice(b)) => a.kind == b.kind,
            (Self::DefaultApp { kind: a, .. }, Self::DefaultApp { kind: b, .. }) => a == b,
            (Self::StartupApp { id: a, .. }, Self::StartupApp { id: b, .. }) => a == b,
            (Self::GameModeOption(a, _), Self::GameModeOption(b, _)) => a == b,
            (
                Self::AppPermission {
                    app: a, kind: x, ..
                },
                Self::AppPermission {
                    app: b, kind: y, ..
                },
            ) => a == b && x == y,
            (Self::Trust { connection: a, .. }, Self::Trust { connection: b, .. })
            | (Self::Dns { connection: a, .. }, Self::Dns { connection: b, .. })
            | (Self::Ipv4 { connection: a, .. }, Self::Ipv4 { connection: b, .. }) => a == b,
            (
                Self::Display(_)
                | Self::OutputVolume(_)
                | Self::OutputMute(_)
                | Self::InputVolume(_)
                | Self::InputMute(_)
                | Self::WifiRadio(_)
                | Self::Airplane(_)
                | Self::Firewall(_)
                | Self::Bluetooth(_)
                | Self::PowerProfile(_)
                | Self::DefaultPrinter(_)
                | Self::Ntp(_)
                | Self::Timezone(_)
                | Self::Hostname(_)
                | Self::HomeHistory(_)
                | Self::BackupSchedule(_)
                | Self::CriticalBattery { .. }
                | Self::MemoryPolicy { .. }
                | Self::TemporaryStorage(_),
                _,
            ) => std::mem::discriminant(self) == std::mem::discriminant(other),
            // One-off actions on different targets must all run.
            _ => false,
        }
    }
}

impl Reading {
    /// Shows a request's expected outcome until the next reading confirms it.
    pub fn apply(&mut self, request: &Request) {
        match (self, request) {
            (Self::Agents(a), Request::AgentRevokeIdentity(id)) => {
                for identity in a.identities.iter_mut().filter(|i| i.id == *id) {
                    identity.revoked = true;
                }
            }
            (
                Self::Agents(a),
                Request::AgentRevokePermission {
                    agent,
                    app,
                    function,
                },
            ) => {
                a.permissions.retain(|p| {
                    !(p.agent_id == *agent && p.app_id == *app && p.function_id == *function)
                });
            }
            (Self::Agents(a), Request::AgentClearActivity) => a.activity.clear(),
            (Self::Apps(a), Request::ScreenSharingRevoke(id)) => {
                a.screen_sharing.grants.retain(|grant| grant.id != *id);
            }
            (Self::Apps(a), Request::DefaultApp { kind, id }) => {
                a.defaults.current.insert(*kind, id.clone());
            }
            (Self::Apps(a), Request::StartupApp { id, enabled, .. }) => {
                let startup = &mut a.startup;
                if let Some(at) = startup.candidates.iter().position(|c| c.id == *id) {
                    let candidate = startup.candidates.remove(at);
                    startup.apps.push(crate::startup_apps::App {
                        id: candidate.id,
                        name: candidate.name,
                        command: String::new(),
                        enabled: *enabled,
                        reason: None,
                        user_override: true,
                        can_enable: true,
                        review: candidate.review,
                    });
                }
                for app in startup.apps.iter_mut().filter(|app| app.id == *id) {
                    app.enabled = *enabled;
                    app.user_override = true;
                }
            }
            (Self::Apps(a), Request::AppPermission { app, kind, allowed }) => {
                let grants = a
                    .permissions
                    .apps
                    .iter_mut()
                    .flatten()
                    .filter(|known| known.id == *app)
                    .flat_map(|known| known.grants.iter_mut());
                for grant in grants.filter(|grant| grant.0 == *kind) {
                    grant.1 = *allowed;
                }
            }
            (Self::Display(d), Request::Display(layout)) => {
                for change in layout {
                    for output in d.outputs.iter_mut().filter(|o| o.name == change.output) {
                        output.current = output
                            .modes
                            .iter()
                            .position(|m| *m == change.mode)
                            .or(output.current);
                        output.scale = change.scale;
                        output.rotation = change.rotation;
                        output.position = change.position;
                        output.enabled = change.enabled;
                    }
                }
            }
            (Self::Sound(s), Request::OutputVolume(v)) => {
                s.output.get_or_insert_default().percent = *v;
            }
            (Self::Sound(s), Request::OutputMute(m)) => s.output.get_or_insert_default().muted = *m,
            (Self::Sound(s), Request::InputVolume(v)) => {
                s.input.get_or_insert_default().percent = *v;
            }
            (Self::Sound(s), Request::InputMute(m)) => s.input.get_or_insert_default().muted = *m,
            (Self::Sound(s), Request::AudioDevice(device)) => {
                if let Some(devices) = s.devices.as_mut() {
                    match device.kind {
                        crate::audio_devices::Kind::Output => devices.output = Some(device.serial),
                        crate::audio_devices::Kind::Input => devices.input = Some(device.serial),
                    }
                }
            }
            (Self::Network(n), Request::WifiRadio(on)) => {
                n.wifi_radio = Some(*on);
                if !on {
                    for point in &mut n.access_points {
                        point.active = false;
                    }
                }
            }
            (Self::Network(n), Request::Vpn(uuid, on)) => {
                for vpn in n.vpns.iter_mut().filter(|v| v.uuid == *uuid) {
                    vpn.active = *on;
                }
            }
            (Self::Network(n), Request::VpnRemove(uuid)) => n.vpns.retain(|v| v.uuid != *uuid),
            (Self::Network(n), Request::VpnRouteAll { uuid, all }) => {
                for vpn in n
                    .vpns
                    .iter_mut()
                    .filter(|v| v.uuid == *uuid)
                    .filter_map(|v| v.vpn.as_mut())
                {
                    vpn.route_all = *all;
                }
            }
            (Self::Network(n), Request::VpnMetered { uuid, metered }) => {
                for vpn in n
                    .vpns
                    .iter_mut()
                    .filter(|v| v.uuid == *uuid)
                    .filter_map(|v| v.vpn.as_mut())
                {
                    vpn.metered = *metered;
                }
            }
            (Self::Network(n), Request::VpnUsername { uuid, name }) => {
                for vpn in n
                    .vpns
                    .iter_mut()
                    .filter(|v| v.uuid == *uuid)
                    .filter_map(|v| v.vpn.as_mut())
                {
                    if vpn.sign_in.is_some() {
                        vpn.sign_in = Some(Some(name.trim().to_owned()));
                    }
                }
            }
            (Self::Network(n), Request::ConnectivityChecks(on)) => {
                if n.connectivity.checks.is_some() {
                    n.connectivity.checks = Some(*on);
                }
            }
            (Self::Bluetooth(b), Request::Bluetooth(on)) => {
                if let Some(adapter) = b.adapter.as_mut() {
                    adapter.powered = *on;
                }
            }
            (Self::Network(n), Request::Airplane(on)) => {
                n.wifi_radio = Some(!on);
            }
            (
                Self::Network(n),
                Request::Trust {
                    connection,
                    trusted,
                },
            ) => {
                for i in n
                    .interfaces
                    .iter_mut()
                    .filter(|i| i.connection == *connection)
                {
                    i.profile.get_or_insert_default().trusted = *trusted;
                }
            }
            (
                Self::Network(n),
                Request::Dns {
                    connection,
                    servers,
                },
            ) => {
                for i in n
                    .interfaces
                    .iter_mut()
                    .filter(|i| i.connection == *connection)
                {
                    i.profile.get_or_insert_default().dns.clone_from(servers);
                }
            }
            (Self::Network(n), Request::Ipv4 { connection, manual }) => {
                for i in n
                    .interfaces
                    .iter_mut()
                    .filter(|i| i.connection == *connection)
                {
                    i.profile.get_or_insert_default().ipv4.clone_from(manual);
                }
            }
            (Self::Network(n), Request::Firewall(on)) => {
                if let Some(f) = n.firewall.as_mut() {
                    f.running = *on;
                }
            }
            (Self::Network(n), Request::Forget(name)) => {
                n.wifi
                    .retain(|c| c.name != *name && c.ssid.as_deref() != Some(name.as_str()));
                for p in n.access_points.iter_mut().filter(|p| p.ssid == *name) {
                    p.saved = false;
                    p.active = false;
                }
            }
            (Self::Bluetooth(b), Request::BluetoothRemove(address)) => {
                b.devices.retain(|d| d.address != *address);
            }
            (Self::Bluetooth(b), Request::BluetoothPair(address)) => {
                if let Some(index) = b.nearby.iter().position(|d| d.address == *address) {
                    let mut device = b.nearby.remove(index);
                    device.paired = true;
                    device.connected = true;
                    b.devices.insert(0, device);
                }
            }
            (Self::Printers(p), Request::PrinterAdd(found)) => {
                let name = printer_queue_name(&found.name);
                if !p.printers.iter().any(|x| x.name == name) {
                    p.printers.push(Printer {
                        name,
                        state: PrinterState::Idle,
                        default: p.printers.is_empty(),
                    });
                }
            }
            (Self::Printers(p), Request::PrinterRemove(name)) => {
                p.printers.retain(|x| x.name != *name);
            }
            (Self::Printers(p), Request::PrinterPause(name, paused)) => {
                for printer in p.printers.iter_mut().filter(|x| x.name == *name) {
                    printer.state = if *paused {
                        PrinterState::Paused
                    } else {
                        PrinterState::Idle
                    };
                }
            }
            (Self::Printers(p), Request::JobCancel(id)) => p.jobs.retain(|job| job.id != *id),
            (Self::DateTime(c), Request::Timezone(zone)) => c.timezone = Some(zone.clone()),
            (Self::Bluetooth(b), Request::BluetoothDevice(address, on)) => {
                for device in b.devices.iter_mut().filter(|d| d.address == *address) {
                    device.connected = *on;
                }
            }
            (Self::Power(p), Request::PowerProfile(profile)) => {
                if let Some(profiles) = p.profiles.as_mut() {
                    profiles.active.clone_from(profile);
                }
            }
            (Self::Printers(p), Request::DefaultPrinter(name)) => {
                for printer in &mut p.printers {
                    printer.default = printer.name == *name;
                }
            }
            (Self::DateTime(c), Request::Ntp(on)) => c.ntp = Some(*on),
            (Self::Safety(Backed::Ready(s)), Request::HomeHistory(on)) => s.home_history = *on,
            (Self::Safety(Backed::Ready(s)), Request::BackupSchedule(on)) => {
                s.schedule_enabled = *on;
            }
            (
                Self::Power(Power {
                    critical: Backed::Ready(c),
                    ..
                }),
                Request::CriticalBattery { action, .. },
            ) => c.configured.clone_from(action),
            (
                Self::Power(Power {
                    tmp_storage: Backed::Ready(storage),
                    ..
                }),
                Request::TemporaryStorage(mode),
            ) => storage.configured = Some(mode.clone()),
            _ => {}
        }
    }
    pub fn panel(&self) -> Panel {
        match self {
            Self::Display(_) => Panel::Display,
            Self::Network(_) => Panel::Network,
            Self::Bluetooth(_) => Panel::Bluetooth,
            Self::Sound(_) => Panel::Sound,
            Self::Power(_) => Panel::Power,
            Self::Printers(_) => Panel::Printers,
            Self::DateTime(_) => Panel::DateTime,
            Self::About(_) => Panel::About,
            Self::Updates(_) => Panel::Updates,
            Self::Safety(_) => Panel::Safety,
            Self::Agents(_) => Panel::Agents,
            Self::Apps(_) => Panel::Apps,
        }
    }
}

fn agents() -> Agents {
    use crate::agent_center;
    if !agent_center::available() {
        return Agents::default();
    }
    let read = || -> Result<Agents, String> {
        Ok(Agents {
            available: true,
            error: None,
            identities: agent_center::credentials()?,
            permissions: agent_center::permissions()?,
            activity: agent_center::activity(100)?,
        })
    };
    read().unwrap_or_else(|error| Agents {
        available: true,
        error: Some(error),
        ..Agents::default()
    })
}

/// XKB layouts this system offers, and the system-wide default layout.
pub async fn keyboard_layouts(
    settings: &SystemSettings,
) -> (Vec<(String, String)>, Option<String>) {
    let rules = [
        "/usr/share/X11/xkb/rules/base.lst",
        "/usr/share/xkeyboard-config-2/rules/base.lst",
    ];
    let mut layouts = rules
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .map(|t| xkb_layouts(&t))
        .unwrap_or_default();
    layouts.sort_by(|a, b| a.1.cmp(&b.1));
    let system = match settings.controls.run("localectl", &["status"]).await {
        Ok(text) => text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("X11 Layout:")?.trim();
            let first = value.split(',').next()?;
            valid_layout(first).then(|| first.to_owned())
        }),
        Err(_) => None,
    };
    (layouts, system)
}

pub(crate) fn valid_layout(code: &str) -> bool {
    (2..=12).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

const PROFILE_SERVICES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

#[derive(Clone)]
pub struct SystemSettings {
    pub(super) controls: SystemControls,
    /// Sample data for reviewing the pages on machines without the hardware;
    /// never touches the host.
    pub demo: bool,
    pub writes_allowed: bool,
    // Connected once, on first use: activation starts the backend.
    pub(super) enzimos: std::sync::Arc<tokio::sync::OnceCell<Option<enzimos::Backend>>>,
}
impl SystemSettings {
    pub fn from_environment() -> Self {
        let preview = crate::desktop_session::mode().nested();
        let demo = std::env::var("ENZUI_SYSTEM_SETTINGS_DEMO").as_deref() == Ok("1");
        Self {
            controls: SystemControls::from_environment(),
            demo,
            writes_allowed: demo
                || !preview
                || std::env::var("ENZUI_ALLOW_SYSTEM_CHANGES").as_deref() == Ok("1"),
            enzimos: Default::default(),
        }
    }
    /// The enzimOS system backend, when this system has one.
    pub async fn enzimos(&self) -> Option<enzimos::Backend> {
        if self.demo {
            return None;
        }
        self.enzimos
            .get_or_init(enzimos::Backend::connect)
            .await
            .clone()
    }
    /// Whether the enzimOS pages apply here (always with sample data).
    pub async fn has_enzimos(&self) -> bool {
        self.demo || self.enzimos().await.is_some()
    }
    /// What this computer offers, with `org.enzimos.System1` asked once
    /// for this backend (everything with sample data).
    pub async fn platform(&self) -> crate::platform::Platform {
        if self.demo {
            return crate::platform::Platform::everything();
        }
        // Absent, it answers at once; starting, it may take until bus
        // activation gives up. Until then every other page shows as before.
        let system1 = tokio::time::timeout(std::time::Duration::from_secs(30), self.has_enzimos())
            .await
            .unwrap_or(false);
        crate::platform::detect(&self.controls, system1).await
    }
    /// Restarts the computer, after an update has been staged.
    pub async fn reboot(&self) -> Result<(), String> {
        if !self.writes_allowed || self.demo {
            return Err(tr("shell-restarting-is-off-in-this-preview").into());
        }
        self.controls.run("systemctl", &["reboot"]).await.map(drop)
    }
    /// Runs a backup or update to its end. `Ok(false)`: there was nothing
    /// to do (already up to date).
    pub async fn long_job(&self, job: LongJob) -> Result<bool, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            demo::long_job(job);
            return Ok(true);
        }
        let backend = self
            .enzimos()
            .await
            .ok_or_else(|| enzimos::Refusal::Unavailable.message())?;
        let hours = std::time::Duration::from_secs(3 * 3600);
        let (schema, operation) = match job {
            LongJob::BackupNow => (3, "backup-create"),
            LongJob::InstallUpdates => (5, "update-apply"),
        };
        let started = chrono::Utc::now().timestamp();
        match backend.run(schema, operation, None, hours).await {
            Ok(result) => {
                Ok(result.get("status").and_then(serde_json::Value::as_str) != Some("up-to-date"))
            }
            // update-apply plans only against an update the system has already
            // been given (a signed local package set); without one it refuses.
            Err(enzimos::Refusal::Code(code))
                if job == LongJob::InstallUpdates && code == "job-plan-invalid" =>
            {
                Err(tr("shell-no-update-is-ready-to-install-on-this-computer").into())
            }
            Err(enzimos::Refusal::NotPrepared) if job == LongJob::InstallUpdates => {
                Err(tr("shell-no-update-could-be-prepared-so-nothing-was-changed").into())
            }
            // A backup refused before it captured anything ends `uncertain`
            // in the job ledger; its run status says why.
            Err(enzimos::Refusal::Uncertain) if job == LongJob::BackupNow => {
                let recorded = backend.query("backups").await.ok().and_then(|status| {
                    let run = &status["run"];
                    (run["state"] == "failed"
                        && run["at"]
                            .as_i64()
                            .or_else(|| run["since"].as_i64())
                            .is_some_and(|at| at >= started))
                    .then(|| run["code"].as_str().map(backup_remote::error_message))
                    .flatten()
                });
                Err(recorded.unwrap_or_else(|| enzimos::Refusal::Uncertain.message()))
            }
            Err(refusal) => Err(refusal.message()),
        }
    }
    pub fn allows(&self, request: &Request) -> bool {
        request.audio() || self.writes_allowed
    }
    /// Pairs a nearby device, sending BlueZ's questions to `ask`.
    pub async fn pair_device(
        &self,
        address: &str,
        ask: impl Fn(pairing::Prompt) + Send + Sync + 'static,
    ) -> Result<(), String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            demo::apply(&Request::BluetoothPair(address.to_owned()));
            return Ok(());
        }
        pairing::pair(address, ask).await
    }
    pub async fn printers_nearby(&self) -> Vec<Discovered> {
        if self.demo {
            return vec![
                Discovered {
                    name: tr("shell-office-colour-mfp").into(),
                    uri: "ipp://office-mfp.local:631/ipp/print".into(),
                    driver: None,
                    unsupported: false,
                },
                Discovered {
                    name: "EPSON Stylus Photo R220".into(),
                    uri: "usb://EPSON/Stylus%20Photo%20R220".into(),
                    driver: Some((
                        "gutenprint.5.3://escp2-r220/expert".into(),
                        "Epson Stylus Photo R220 - CUPS+Gutenprint v5.3.5".into(),
                    )),
                    unsupported: false,
                },
            ];
        }
        self.discover_printers().await
    }
    pub async fn read(&self, panel: Panel) -> Reading {
        if self.demo {
            return demo::reading(panel);
        }
        match panel {
            Panel::Display => Reading::Display(
                tokio::task::spawn_blocking(display::read)
                    .await
                    .unwrap_or_default(),
            ),
            Panel::Network => Reading::Network(self.network().await),
            Panel::Bluetooth => Reading::Bluetooth(self.bluetooth().await),
            Panel::Sound => Reading::Sound(self.sound().await),
            Panel::Power => Reading::Power(self.power().await),
            Panel::Printers => Reading::Printers(self.printers().await),
            Panel::DateTime => Reading::DateTime(self.clock().await),
            Panel::About => Reading::About(Box::new(self.about().await)),
            Panel::Updates => Reading::Updates(self.updates().await),
            Panel::Safety => Reading::Safety(self.safety().await.boxed()),
            Panel::Agents => Reading::Agents(
                tokio::task::spawn_blocking(agents)
                    .await
                    .unwrap_or_default(),
            ),
            Panel::Apps => {
                let screen_sharing = tokio::task::spawn_blocking(screen_sharing)
                    .await
                    .unwrap_or_default();
                Reading::Apps(Apps {
                    screen_time: crate::screen_time::path()
                        .map(|path| crate::screen_time::load(&path))
                        .unwrap_or_default(),
                    app_names: tokio::task::spawn_blocking(|| {
                        crate::default_apps::by_window_class()
                            .into_values()
                            .map(|(id, _, class)| {
                                (class, crate::default_apps::name(&id).unwrap_or(id))
                            })
                            .collect()
                    })
                    .await
                    .unwrap_or_default(),
                    defaults: tokio::task::spawn_blocking(|| crate::default_apps::read(""))
                        .await
                        .unwrap_or_default(),
                    startup: tokio::task::spawn_blocking(crate::startup_apps::read)
                        .await
                        .unwrap_or_default(),
                    permissions: crate::app_permissions::read().await,
                    screen_sharing: screen_sharing.0,
                    screen_sharing_names: screen_sharing.1,
                    supervised: if crate::parental::available().await {
                        Some(
                            crate::parental::standard_accounts()
                                .await
                                .unwrap_or_default(),
                        )
                    } else {
                        None
                    },
                })
            }
        }
    }
}

// The kept screen-sharing grants and the names of their apps.
fn screen_sharing() -> (
    crate::screencast_grants::Listing,
    std::collections::BTreeMap<String, String>,
) {
    let listing = crate::screencast_grants::path()
        .map(|path| crate::screencast_grants::read(&path))
        .unwrap_or_default();
    let names = listing
        .grants
        .iter()
        .filter_map(|grant| {
            crate::default_apps::name(&grant.app_id).map(|name| (grant.app_id.clone(), name))
        })
        .collect();
    (listing, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn system_settings_behaviour() {
        // System settings behaviour.
        {
            // A refused refresh keeps the last good backend reading.
            {
                let ready = Reading::Safety(Backed::Ready(Box::default()));
                let refused = Reading::Safety(Backed::Refused("busy".into()));
                assert!(refused.refused_over(&ready));
                assert!(!refused.refused_over(&refused), "nothing good to keep");
                assert!(
                    !ready.refused_over(&refused),
                    "a good reading always replaces"
                );
                assert!(
                    !Reading::Updates(Backed::Refused("busy".into())).refused_over(&ready),
                    "only the same panel"
                );
                assert!(
                    Reading::Updates(Backed::Refused("busy".into())).refused_over(
                        &Reading::Updates(Backed::Ready(enzimos::SystemState::default()))
                    )
                );
                assert!(
                    !Reading::Safety(Backed::Missing).refused_over(&ready),
                    "a missing backend is not a refusal"
                );
            }
            // Drafts show the requested outcome.
            {
                let mut reading = demo::reading(Panel::Network);
                reading.apply(&Request::WifiRadio(false));
                let Reading::Network(network) = reading else {
                    unreachable!()
                };
                assert_eq!(network.wifi_radio, Some(false));
                assert!(network.access_points.iter().all(|p| !p.active));
                let mut sound = Reading::Sound(Sound::default());
                sound.apply(&Request::OutputVolume(30));
                assert!(matches!(
                    sound,
                    Reading::Sound(Sound {
                        output: Some(Level { percent: 30, .. }),
                        ..
                    })
                ));
            }
        }

        // Only audio changes the host without the opt in.
        {
            let gated = SystemSettings {
                controls: SystemControls::from_environment(),
                demo: false,
                writes_allowed: false,
                enzimos: Default::default(),
            };
            assert!(gated.allows(&Request::OutputVolume(40)));
            assert!(gated.allows(&Request::InputMute(true)));
            for request in [
                Request::WifiRadio(false),
                Request::Vpn("x".into(), true),
                Request::Bluetooth(false),
                Request::PowerProfile("performance".into()),
                Request::DefaultPrinter("x".into()),
                Request::Ntp(false),
            ] {
                assert!(!gated.allows(&request));
                assert!(gated.apply(&request).await.is_err());
            }
        }
        // firewalld is never turned on while another firewall is active, and
        // is turned on where none is.
        //
        // Sabotage: drop the other-firewall check from `Request::Firewall`.
        {
            let root = tempfile::tempdir().unwrap();
            let settings = fixture(root.path());
            let log = root.path().join("log");
            let systemctl = |active: &str| {
                format!(
                    "[ \"$1\" = is-active ] && {{ [ \"$2\" = {active} ] && echo active && exit 0; echo inactive; exit 3; }}; echo \"$*\" >> '{}'",
                    log.display()
                )
            };
            script(root.path(), "systemctl", &systemctl("ufw.service"));
            let refused = settings.apply(&Request::Firewall(true)).await.unwrap_err();
            assert!(refused.contains("ufw"), "{refused}");
            assert!(!log.exists(), "firewalld was enabled beside ufw");
            script(root.path(), "systemctl", &systemctl("none"));
            settings.apply(&Request::Firewall(true)).await.unwrap();
            assert_eq!(
                std::fs::read_to_string(&log).unwrap(),
                "enable --now firewalld.service\n"
            );
        }
        // Wi-Fi profiles are changed by UUID and only Wi-Fi profiles go: a VPN
        // that shares a network's name survives forgetting it and a failed
        // join, and a manual address keeps the DNS servers in use.
        //
        // Sabotage: delete by `id` in `Request::Forget` or after a failed join,
        // or drop the DNS carried over in `Request::Ipv4`.
        {
            let root = tempfile::tempdir().unwrap();
            let settings = fixture(root.path());
            let log = root.path().join("log");
            let joined = root.path().join("joined");
            // A fake nmcli: Office is both a Wi-Fi network and a VPN; a failed
            // join to Cafe leaves a Cafe profile behind, as nmcli does.
            script(
                root.path(),
                "nmcli",
                &format!(
                    r#"case "$*" in
"-t -f NETWORKING general") echo enabled;;
"-t -f WIFI radio") echo enabled;;
"-t -f DEVICE,TYPE,STATE,CONNECTION device status") echo 'wlan0:wifi:connected:Home';;
"-t -f NAME,UUID,TYPE,ACTIVE connection show")
  echo 'Home:aaaa-home:802-11-wireless:yes'
  echo 'Office:aaaa-wifi:802-11-wireless:no'
  echo 'Office:aaaa-vpn:vpn:no'
  [ -e '{joined}' ] && echo 'Cafe:aaaa-new:802-11-wireless:no'; true;;
*"device wifi list"*) printf '*:Home:80:WPA2
 :Office:60:WPA2
 :Cafe:50:WPA2
';;
*"device show wlan0") printf 'IP4.ADDRESS[1]:192.168.2.40/24
IP4.DNS[1]:192.168.2.1
IP4.DNS[2]:fd00::1
';;
*"connection show id Home") printf 'connection.zone:--
ipv4.dns:
ipv6.dns:
ipv4.method:auto
';;
*"device wifi connect"*) touch '{joined}'; echo 'Error: Connection activation failed: Secrets were required'; exit 4;;
*) echo "$*" >> '{log}';;
esac"#,
                    joined = joined.display(),
                    log = log.display()
                ),
            );
            let network = settings.control_network(false).await.unwrap();
            assert!(network.access_points.iter().any(|point| {
                point.ssid == "Office" && point.saved && point.secure && !point.active
            }));
            assert!(
                network
                    .access_points
                    .iter()
                    .any(|point| { point.ssid == "Home" && point.active })
            );
            settings
                .apply(&Request::Forget("Office".into()))
                .await
                .unwrap();
            let wrong = Secret("wrong-key".into());
            assert!(
                settings
                    .apply(&Request::WifiJoin {
                        ssid: "Cafe".into(),
                        password: Some(wrong),
                        hidden: false,
                        enterprise: None,
                    })
                    .await
                    .is_err()
            );
            settings
                .apply(&Request::Ipv4 {
                    connection: "Home".into(),
                    manual: Some(ManualIpv4 {
                        address: "192.168.2.50/24".into(),
                        gateway: "192.168.2.1".into(),
                    }),
                })
                .await
                .unwrap();
            let log = std::fs::read_to_string(&log).unwrap();
            // The changes only, not the reads.
            let lines: Vec<&str> = log.lines().filter(|l| !l.starts_with("-t ")).collect();
            let log = lines.join("\n");
            assert!(lines.contains(&"connection delete uuid aaaa-wifi"), "{log}");
            assert!(lines.contains(&"connection delete uuid aaaa-new"), "{log}");
            assert!(
                !log.contains("aaaa-vpn") && !log.contains("delete id"),
                "{log}"
            );
            assert!(
                lines.contains(&"connection modify id Home ipv4.method manual ipv4.addresses 192.168.2.50/24 ipv4.gateway 192.168.2.1 ipv4.dns 192.168.2.1"),
                "{log}"
            );
        }
        // An enterprise join trusts only a server certified for the domain it
        // names, refuses a bad form before changing any profile, and replaces
        // the network's saved profile only once the new one has connected.
        //
        // Sabotage: drop the server settings from `join_enterprise`, or delete
        // the network's old profiles before `connection up`.
        {
            let root = tempfile::tempdir().unwrap();
            let settings = fixture(root.path());
            let log = root.path().join("log");
            script(
                root.path(),
                "nmcli",
                &format!(
                    r#"case "$*" in
"-t -f NETWORKING general") echo enabled;;
"-t -f WIFI radio") echo enabled;;
"-t -f DEVICE,TYPE,STATE,CONNECTION device status") echo 'wlan0:wifi:connected:Home';;
"-t -f NAME,UUID,TYPE,ACTIVE connection show")
  echo 'Home:aaaa-home:802-11-wireless:yes'
  echo 'Campus:aaaa-old:802-11-wireless:no';;
*"device wifi list"*) printf '*:Home:80:WPA2
 :Campus:60:WPA2 802.1X
';;
-t*) ;;
*"connection up"*)
  credentials=$(cat) || exit 1
  case "$credentials" in
    '802-1x.identity:alice
802-1x.password:wrong') result=4;;
    '802-1x.identity:alice
802-1x.password:secret') result=0;;
    *) exit 1;;
  esac
  echo "$*" >> '{log}'
  if [ "$result" = 4 ]; then
    echo 'Error: Connection activation failed: Secrets were required' >&2
  fi
  exit "$result";;
*) echo "$*" >> '{log}';;
esac"#,
                    log = log.display()
                ),
            );
            let join = |password: &str, server: ServerCheck| Request::WifiJoin {
                ssid: "Campus".into(),
                password: Some(Secret(password.into())),
                hidden: false,
                enterprise: Some(Enterprise {
                    identity: "alice".into(),
                    server,
                }),
            };
            let campus = || ServerCheck::Domain("radius.example.org".into());
            for refused in [
                join("line\nbreak", campus()),
                join("secret", ServerCheck::Domain(String::new())),
                join(
                    "secret",
                    ServerCheck::Authority {
                        file: "ca.pem".into(),
                        domain: String::new(),
                    },
                ),
            ] {
                assert!(settings.apply(&refused).await.is_err());
            }
            assert!(!log.exists(), "a refused form changed a profile");
            let error = settings.apply(&join("wrong", campus())).await.unwrap_err();
            assert_eq!(error, tr("shell-the-password-is-incorrect"));
            settings.apply(&join("secret", campus())).await.unwrap();
            let log = std::fs::read_to_string(&log).unwrap();
            let lines: Vec<&str> = log.lines().collect();
            let adds: Vec<&&str> = lines
                .iter()
                .filter(|l| l.starts_with("connection add"))
                .collect();
            assert_eq!(adds.len(), 2, "{log}");
            assert!(
                adds.iter().all(|add| add.contains(
                    "802-1x.system-ca-certs yes 802-1x.domain-suffix-match radius.example.org"
                )),
                "{log}"
            );
            assert_eq!(
                lines.iter().filter(|l| l.contains("connection up")).count(),
                2,
                "{log}"
            );
            let old = lines.iter().position(|l| l.contains("aaaa-old"));
            let connected = lines.iter().rposition(|l| l.contains("connection up"));
            assert_eq!(
                lines.iter().filter(|l| l.contains("aaaa-old")).count(),
                1,
                "{log}"
            );
            assert!(old > connected, "{log}");
        }
        // Without PipeWire's tools the sound levels and devices are read and
        // set with PulseAudio's `pactl`.
        //
        // Sabotage: drop the `pactl` branch from `sound` or from `apply`, or the
        // device list from the `pactl` branch of `sound`.
        {
            let root = tempfile::tempdir().unwrap();
            let settings = fixture(root.path());
            let log = root.path().join("log");
            script(
                root.path(),
                "pactl",
                &format!(
                    "case \"$1\" in get-sink-volume) echo 'Volume: front-left: 42598 /  65% / -11.23 dB,   front-right: 42598 /  65% / -11.23 dB';; get-sink-mute) echo 'Mute: no';; get-source-volume) echo 'Volume: mono: 32768 /  50% / -18.06 dB';; get-source-mute) echo 'Mute: yes';; --format=json) echo '[{{\"index\":2,\"name\":\"out\",\"description\":\"Speakers\",\"properties\":{{}}}}]';; get-default-sink) echo out;; get-default-source) exit 1;; *) echo \"$*\" >> '{}';; esac",
                    log.display()
                ),
            );
            let sound = settings.sound().await;
            assert_eq!(
                sound.output,
                Some(Level {
                    percent: 65,
                    muted: false
                })
            );
            assert_eq!(
                sound.input,
                Some(Level {
                    percent: 50,
                    muted: true
                })
            );
            assert!(sound.error.is_none());
            let devices = sound.devices.expect("pactl lists the devices");
            assert!(
                devices
                    .nodes
                    .iter()
                    .any(|d| devices.selected(d) && d.label == "Speakers")
            );
            settings.apply(&Request::OutputVolume(30)).await.unwrap();
            settings.apply(&Request::InputMute(false)).await.unwrap();
            assert_eq!(
                std::fs::read_to_string(&log).unwrap(),
                "set-sink-volume @DEFAULT_SINK@ 30%\nset-source-mute @DEFAULT_SOURCE@ 0\n"
            );
        }
    }

    #[tokio::test]
    async fn adding_a_printer_refuses_to_replace_a_print_queue() {
        let root = tempfile::tempdir().unwrap();
        let settings = fixture(root.path());
        let request = Request::PrinterAdd(Discovered {
            name: "Office Printer".into(),
            uri: "ipp://second.local/ipp/print".into(),
            unsupported: false,
            driver: None,
        });

        script(
            root.path(),
            "lpstat",
            "echo 'office_printer not accepting requests'",
        );
        assert_eq!(
            settings.apply(&request).await.unwrap_err(),
            tr("shell-that-print-queue-already-exists")
        );
        assert!(!settings.check_new_printer_queue("Warehouse").await.unwrap());

        script(
            root.path(),
            "lpstat",
            "echo 'lpstat: No destinations added.' >&2; exit 1",
        );
        assert!(
            settings
                .check_new_printer_queue("Office_Printer")
                .await
                .unwrap()
        );

        script(
            root.path(),
            "lpstat",
            "echo 'lpstat: Connection refused' >&2; exit 1",
        );
        assert_eq!(
            settings.apply(&request).await.unwrap_err(),
            tr("shell-could-not-check-existing-print-queues")
        );
    }

    fn fixture(root: &std::path::Path) -> SystemSettings {
        SystemSettings {
            controls: SystemControls::fixture(root),
            demo: false,
            writes_allowed: true,
            enzimos: std::sync::Arc::new(tokio::sync::OnceCell::new_with(Some(None))),
        }
    }

    fn script(root: &std::path::Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}
