use super::*;
use crate::audio_devices::Devices;
use std::sync::Mutex;

struct State {
    display: display::Displays,
    network: Network,
    bluetooth: Bluetooth,
    sound: Sound,
    power: Power,
    printers: Printers,
    clock: Clock,
    updates: enzimos::SystemState,
    safety: enzimos::Safety,
    agents: Agents,
    apps: Apps,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn initial() -> State {
    use crate::audio_devices::{Device, Kind};
    let device = |id, serial, label: &str, kind| Device {
        id,
        serial,
        name: format!("demo.{id}"),
        label: label.into(),
        kind,
    };
    let point = |ssid: &str, signal, secure, active, saved| AccessPoint {
        ssid: ssid.into(),
        signal,
        secure,
        enterprise: false,
        active,
        saved,
    };
    let paired = |address: &str, name: &str, kind, connected| BluetoothDevice {
        address: address.into(),
        name: name.into(),
        kind,
        paired: true,
        connected,
    };
    let mode = |width, height, refresh_mhz, preferred| display::Mode {
        width,
        height,
        refresh_mhz,
        preferred,
    };
    State {
        display: display::Displays {
            supported: true,
            outputs: vec![
                display::Output {
                    name: "eDP-1".into(),
                    make: "BOE".into(),
                    model: "NE140QDM".into(),
                    enabled: true,
                    modes: vec![
                        mode(2880, 1800, 120_000, true),
                        mode(2880, 1800, 60_000, false),
                        mode(1920, 1200, 60_000, false),
                    ],
                    current: Some(0),
                    scale: 1.75,
                    position: (0, 360),
                    physical_mm: (302, 189),
                    ..Default::default()
                },
                display::Output {
                    name: "DP-2".into(),
                    make: "Dell".into(),
                    model: "U2723QE".into(),
                    enabled: true,
                    modes: vec![
                        mode(3840, 2160, 60_000, true),
                        mode(2560, 1440, 59_951, false),
                        mode(1920, 1080, 60_000, false),
                    ],
                    current: Some(0),
                    scale: 1.5,
                    position: (1646, 0),
                    physical_mm: (597, 336),
                    ..Default::default()
                },
            ],
        },
        network: Network {
            networking: Some(true),
            firewall: Some(Firewall {
                running: true,
                default_zone: "public".into(),
            }),
            wifi_radio: Some(true),
            interfaces: vec![
                Interface {
                    device: "wlan0".into(),
                    kind: InterfaceKind::Wifi,
                    state: LinkState::Connected,
                    connection: "Harbour Studio".into(),
                    connection_uuid: String::new(),
                    profile: Some(Profile {
                        trusted: true,
                        dns: Vec::new(),
                        ipv4: None,
                    }),
                    details: Some(Details {
                        hardware: Some("3C:A9:F4:12:7E:0B".into()),
                        ipv4: Some("10.0.4.23/24".into()),
                        ipv6: None,
                        gateway: Some("10.0.4.1".into()),
                        dns: vec!["10.0.4.1".into()],
                    }),
                },
                Interface {
                    device: "enp3s0".into(),
                    kind: InterfaceKind::Ethernet,
                    state: LinkState::Unavailable,
                    connection: String::new(),
                    connection_uuid: String::new(),
                    details: None,
                    profile: None,
                },
            ],
            access_points: vec![
                point("Harbour Studio", 82, true, true, true),
                point("Harbour Guest", 74, false, false, true),
                point("Lighthouse 5G", 58, true, false, false),
                point("Pier Café", 41, true, false, false),
                AccessPoint {
                    enterprise: true,
                    ..point("Harbour University", 36, true, false, false)
                },
                point("DIRECT-printer", 22, true, false, false),
            ],
            wifi: vec![SavedConnection {
                name: "Harbour Studio".into(),
                kind: ConnectionKind::Wifi,
                active: true,
                uuid: "0b3c1a52-7f1e-4c2d-9b8a-5e6f7a8b9c0d".into(),
                ssid: Some("Harbour Studio".into()),
                vpn: None,
            }],
            vpns: vec![SavedConnection {
                name: "Office WireGuard".into(),
                kind: ConnectionKind::Vpn,
                active: false,
                uuid: "6f2b1c1e-6d7c-4a39-9a2e-1d6f3c9b2a11".into(),
                ssid: None,
                vpn: Some(Vpn {
                    service: VpnService::WireGuard,
                    route_all: true,
                    metered: false,
                    split_by_peers: false,
                    sign_in: None,
                }),
            }],
            connectivity: crate::network_connectivity::Connectivity {
                state: crate::network_connectivity::State::Full,
                checks: Some(true),
                uri: Some("http://ping.archlinux.org/nm-check.txt".into()),
                devices: None,
            },
        },
        bluetooth: Bluetooth {
            adapter: Some(Adapter {
                name: "enzimOS laptop".into(),
                powered: true,
                discoverable: false,
            }),
            devices: vec![
                paired(
                    "AC:80:0A:11:22:33",
                    "Soundcore Liberty 4",
                    DeviceKind::Headphones,
                    true,
                ),
                paired("E4:17:D8:44:55:66", "MX Keys", DeviceKind::Keyboard, true),
                paired(
                    "F0:1D:BC:77:88:99",
                    "MX Anywhere 3",
                    DeviceKind::Mouse,
                    false,
                ),
                paired("54:2B:57:AA:BB:CC", "Pixel 9", DeviceKind::Phone, false),
            ],
            nearby: vec![BluetoothDevice {
                address: "C8:7B:23:10:20:30".into(),
                name: "Studio Speaker".into(),
                kind: DeviceKind::Other,
                paired: false,
                connected: false,
            }],
        },
        sound: Sound {
            output: Some(Level {
                percent: 64,
                muted: false,
            }),
            input: Some(Level {
                percent: 48,
                muted: false,
            }),
            devices: Some(Devices {
                nodes: vec![
                    device(41, 1, "Speakers", Kind::Output),
                    device(42, 2, "Soundcore Liberty 4", Kind::Output),
                    device(43, 3, "HDMI — Studio Display", Kind::Output),
                    device(51, 4, "Internal microphone", Kind::Input),
                    device(52, 5, "Soundcore Liberty 4", Kind::Input),
                ],
                output: Some(1),
                input: Some(4),
            }),
            error: None,
        },
        power: Power {
            game_mode: Some(crate::game_mode::Status {
                clients: 1,
                requested: false,
            }),
            game_preferences: Default::default(),
            lid: true,
            battery: Some(BatteryState {
                percent: 68,
                charging: false,
                state: "On battery".into(),
                remaining: Some("4 h 10 min".into()),
            }),
            profiles: Some(Profiles {
                active: "balanced".into(),
                available: vec![
                    "power-saver".into(),
                    "balanced".into(),
                    "performance".into(),
                ],
            }),
            critical: Backed::Ready(CriticalBattery {
                configured: "auto".into(),
                supported: vec![
                    "auto".into(),
                    "suspend".into(),
                    "hibernate".into(),
                    "power-off".into(),
                    "ignore".into(),
                ],
                risky: vec!["suspend".into(), "ignore".into()],
            }),
            tmp_storage: Backed::Ready(TemporaryStorage {
                active: "ram".into(),
                configured: Some("ram".into()),
            }),
            memory: Backed::Ready(MemoryPolicy {
                mode: "balanced".into(),
                available: vec!["balanced".into(), "responsive".into(), "off".into()],
                active_state: "active".into(),
                running_mode: Some("balanced".into()),
                memory_bytes: Some(1_530_000),
                protected_names: Some(vec![
                    "Hyprland".into(),
                    "enzui-shell".into(),
                    "enzui-lock".into(),
                    "enzui-polkit-ag".into(),
                ]),
            }),
        },
        agents: {
            use crate::agent_center::{Activity, Credential, Permission};
            let at = 1_790_700_000_000;
            let credential = |id: &str, name: &str, kind: &str| Credential {
                id: id.into(),
                display_name: name.into(),
                kind: kind.into(),
                revoked: false,
                created_unix_ms: at,
            };
            let permission = |agent: &str, app: &str, function: &str, allowed| Permission {
                agent_id: agent.into(),
                agent_name: Some("Claude Code".into()),
                app_id: app.into(),
                app_name: Some("Mail".into()),
                function_id: function.into(),
                allowed,
                updated_unix_ms: at,
            };
            let event = |id, function: &str, outcome: &str| Activity {
                id,
                request_id: Some(format!("demo-{id}")),
                created_unix_ms: at + id * 60_000,
                agent_id: "agent.claude-code".into(),
                agent_name: Some("Claude Code".into()),
                app_id: "dev.enzui.Mail".into(),
                app_name: Some("Mail".into()),
                function_id: function.into(),
                outcome: outcome.into(),
            };
            Agents {
                available: true,
                error: None,
                identities: vec![
                    credential("agent.claude-code", "Claude Code", "agent"),
                    credential("dev.enzui.Mail", "Mail", "app"),
                ],
                permissions: vec![
                    permission(
                        "agent.claude-code",
                        "dev.enzui.Mail",
                        "search_messages",
                        true,
                    ),
                    permission(
                        "agent.claude-code",
                        "dev.enzui.Mail",
                        "delete_message",
                        false,
                    ),
                ],
                activity: vec![
                    event(3, "search_messages", "completed"),
                    event(2, "delete_message", "permission_denied"),
                    event(1, "search_messages", "completed"),
                ],
            }
        },
        printers: Printers {
            scheduler: Some(true),
            printers: vec![
                Printer {
                    name: "Studio_LaserJet".into(),
                    state: PrinterState::Idle,
                    default: true,
                },
                Printer {
                    name: "Photo_Inkjet".into(),
                    state: PrinterState::Offline,
                    default: false,
                },
            ],
            jobs: vec![crate::cups_admin::Job {
                id: "Studio_LaserJet-41".into(),
                queue: "Studio_LaserJet".into(),
                size: "212 KB".into(),
                held: false,
            }],
            admin: true,
        },
        updates: enzimos::SystemState {
            os: "enzimOS".into(),
            kernel: "6.18.2-arch1-1".into(),
            firmware: "uefi".into(),
            packages: vec![
                ("enzimos-native-policy".into(), "0.1.0-102".into()),
                ("enzimos-system-backend".into(), "0.1.0-5".into()),
                ("linux".into(), "6.18.2.arch1-1".into()),
                ("linux-lts".into(), "6.12.40-1".into()),
            ],
            generations: vec![
                enzimos::Generation {
                    name: "enzimos-2".into(),
                    state: "booted".into(),
                    ordinal: 2,
                    kernels: vec!["linux".into(), "linux-lts".into()],
                    confirmed: true,
                },
                enzimos::Generation {
                    name: "enzimos-1".into(),
                    state: "fallback".into(),
                    ordinal: 1,
                    kernels: vec!["linux".into(), "linux-lts".into()],
                    confirmed: true,
                },
            ],
            current_generation: Some("enzimos-2".into()),
            recovery_images_ready: true,
        },
        safety: demo_safety(),
        clock: Clock {
            zones: [
                "Europe/Budapest",
                "Europe/Dublin",
                "Europe/London",
                "America/New_York",
                "Asia/Tokyo",
            ]
            .map(String::from)
            .to_vec(),
            timezone: Some("Europe/Budapest".into()),
            ntp: Some(true),
            can_ntp: true,
            synchronized: Some(true),
            local_rtc: false,
        },
        apps: demo_apps(),
    }
}

fn demo_apps() -> Apps {
    use crate::app_permissions::{App, Kind};
    let flatpak = |id: &str, name: &str, icon: &str, grants, sandbox| App {
        id: id.into(),
        name: name.into(),
        grants,
        sandbox,
        icon: crate::icon_theme::lookup(icon, 32),
    };
    Apps {
        defaults: crate::default_apps::demo(),
        startup: crate::startup_apps::demo(),
        // A location provider, so the preview shows the Location choice.
        permissions: crate::app_permissions::Permissions {
            apps: Some(vec![
                flatpak(
                    "com.obsproject.Studio",
                    "OBS Studio",
                    "com.obsproject.Studio",
                    vec![(Kind::Camera, true), (Kind::Screenshot, true)],
                    vec![tr("shell-microphone-and-speakers"), tr("shell-home-folder")],
                ),
                flatpak(
                    "org.signal.Signal",
                    "Signal",
                    "org.signal.Signal",
                    vec![(Kind::Notifications, true), (Kind::Background, false)],
                    vec!["Network", tr("shell-some-folders")],
                ),
            ]),
            location_provider: true,
            ..Default::default()
        },
        ..Apps::default()
    }
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(initial))
}

pub(super) fn reading(panel: Panel) -> Reading {
    with(|s| match panel {
        Panel::Display => Reading::Display(s.display.clone()),
        Panel::Network => Reading::Network(s.network.clone()),
        Panel::Bluetooth => Reading::Bluetooth(s.bluetooth.clone()),
        Panel::Sound => Reading::Sound(s.sound.clone()),
        Panel::Power => Reading::Power(s.power.clone()),
        Panel::Printers => Reading::Printers(s.printers.clone()),
        Panel::DateTime => Reading::DateTime(s.clock.clone()),
        Panel::About => Reading::About(Box::new(About {
            hostname: Some("enzimos-laptop".into()),
            compositor: Some("Hyprland".into()),
            os: Some("enzimOS 0.1".into()),
            kernel: Some("6.18.2-enzim".into()),
            cpu: Some("AMD Ryzen 7 8840U".into()),
            threads: Some(16),
            memory_kib: Some(32 * 1024 * 1024),
            gpus: vec!["AMD Radeon 780M".into()],
            disk: Some((1_000_000_000_000, 612_000_000_000)),
            enzimos: Some(s.updates.clone()),
            crashes: Some(std::sync::Arc::default()),
        })),
        Panel::Updates => Reading::Updates(Backed::Ready(s.updates.clone())),
        Panel::Safety => Reading::Safety(Backed::Ready(Box::new(s.safety.clone()))),
        Panel::Agents => Reading::Agents(s.agents.clone()),
        Panel::Apps => Reading::Apps(s.apps.clone()),
    })
}

pub(super) fn apply(request: &Request) {
    with(|s| {
        for reading in [
            Reading::Display(std::mem::take(&mut s.display)),
            Reading::Network(std::mem::take(&mut s.network)),
            Reading::Bluetooth(std::mem::take(&mut s.bluetooth)),
            Reading::Sound(std::mem::take(&mut s.sound)),
            Reading::Power(std::mem::take(&mut s.power)),
            Reading::Printers(std::mem::take(&mut s.printers)),
            Reading::DateTime(std::mem::take(&mut s.clock)),
            Reading::Safety(Backed::Ready(Box::new(std::mem::take(&mut s.safety)))),
            Reading::Agents(std::mem::take(&mut s.agents)),
            Reading::Apps(std::mem::take(&mut s.apps)),
        ] {
            let mut reading = reading;
            reading.apply(request);
            match reading {
                Reading::Display(v) => s.display = v,
                Reading::Network(v) => s.network = v,
                Reading::Bluetooth(v) => s.bluetooth = v,
                Reading::Sound(v) => s.sound = v,
                Reading::Power(v) => s.power = v,
                Reading::Printers(v) => s.printers = v,
                Reading::DateTime(v) => s.clock = v,
                Reading::Safety(Backed::Ready(v)) => s.safety = *v,
                Reading::Agents(v) => s.agents = v,
                Reading::Apps(v) => s.apps = v,
                Reading::About(_) | Reading::Updates(_) | Reading::Safety(_) => {}
            }
        }
    });
}

pub(super) fn long_job(job: LongJob) {
    with(|s| match job {
        LongJob::InstallUpdates => {
            let next = s
                .updates
                .generations
                .iter()
                .map(|g| g.ordinal)
                .max()
                .unwrap_or(0)
                + 1;
            let staged = enzimos::Generation {
                name: format!("enzimos-{next}"),
                state: "selected".into(),
                ordinal: next,
                kernels: vec!["linux".into()],
                confirmed: false,
            };
            s.updates.generations.insert(0, staged);
        }
        LongJob::BackupNow => {
            let id = format!("{:064x}", s.safety.points.as_ref().map_or(0, Vec::len) + 1);
            let time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            s.safety.last_backup = Some(backups::LastBackup {
                time: time.clone(),
                state: "completed".into(),
                point: Some(id.clone()),
            });
            if let Some(points) = s.safety.points.as_mut() {
                points.insert(
                    0,
                    backups::Point {
                        id,
                        time,
                        place: backups::Place::Local,
                        complete: true,
                        size: Some(18 << 30),
                    },
                );
            }
        }
    });
}

pub(super) fn set_retention(retention: backups::Retention) {
    with(|s| s.safety.home_retention = Some(retention));
}

pub(super) fn set_backup_retention(policy: backups::BackupRetention) {
    with(|s| s.safety.backup_retention = Some(policy));
}

pub(super) fn move_point(id: &str, to_share: bool) {
    with(|s| {
        if let Some(point) = s
            .safety
            .points
            .iter_mut()
            .flatten()
            .find(|point| point.id == id)
        {
            point.place = match (point.place, to_share) {
                (backups::Place::Local, true) | (backups::Place::Remote, false) => {
                    backups::Place::Both
                }
                (place, _) => place,
            };
        }
    });
}

/// The sample network share. ENZUI_DEMO_BACKUP_SHARE=none shows the
/// not-set-up state, `old` an SFTP setup that has to be redone.
fn demo_remote() -> backup_remote::RemoteShare {
    let state = match std::env::var("ENZUI_DEMO_BACKUP_SHARE").as_deref() {
        Ok("none") => "not-configured",
        Ok("old") => "reconfigure-as-smb",
        _ => "configured",
    };
    if state != "configured" {
        return backup_remote::RemoteShare {
            state: state.into(),
            password_source: "local-backup".into(),
            ..Default::default()
        };
    }
    configured_share(&backup_remote::ShareInput {
        server: "nas.local".into(),
        share: "backups".into(),
        folder: "enzimos/enzimos-laptop".into(),
        user: "alice".into(),
        domain: String::new(),
    })
}

fn configured_share(input: &backup_remote::ShareInput) -> backup_remote::RemoteShare {
    let folder = input.folder.trim().trim_matches('/');
    backup_remote::RemoteShare {
        state: "configured".into(),
        password_source: "local-backup".into(),
        server: input.server.trim().into(),
        share: input.share.trim().into(),
        folder: folder.into(),
        user: input.user.trim().into(),
        domain: input.domain.trim().into(),
        location: backup_remote::address(input.server.trim(), input.share.trim(), folder),
        share_password_ready: true,
        repository_password_ready: true,
        repository_ready: true,
    }
}

/// Sample setup: "offline" servers do not answer and the password "wrong"
/// is refused, so each outcome can be shown.
pub(super) fn remote_share_setup(
    input: &backup_remote::ShareInput,
    password: &str,
) -> Result<String, String> {
    if input.server.contains("offline") {
        return Err("remote-share-unreachable".into());
    }
    if password == "wrong" {
        return Err("remote-share-sign-in-refused".into());
    }
    with(|s| {
        s.safety.remote = configured_share(input);
        s.safety.remote_configured = true;
    });
    Ok("created".into())
}

pub(super) fn remote_share_test() -> Result<backup_remote::Tested, String> {
    with(|s| {
        if s.safety.remote.configured() {
            Ok(backup_remote::Tested {
                reachable: true,
                signed_in: true,
                repository: "ready".into(),
                reason: None,
            })
        } else {
            Err("remote-share-not-configured".into())
        }
    })
}

pub(super) fn remote_share_remove() -> Result<(), String> {
    with(|s| {
        s.safety.remote = backup_remote::RemoteShare {
            state: "not-configured".into(),
            password_source: "local-backup".into(),
            ..Default::default()
        };
        s.safety.remote_configured = false;
    });
    Ok(())
}

fn demo_safety() -> enzimos::Safety {
    use backups::{LastBackup, Place, Point, Snapshot, SnapshotKind};
    let ago = |hours: i64| {
        (chrono::Utc::now() - chrono::Duration::hours(hours))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    let snapshot = |number, hours, kind, description: &str| Snapshot {
        number,
        date: ago(hours),
        kind,
        description: description.into(),
    };
    let point = |n: u8, hours, place, size| Point {
        id: format!("{n:x}").repeat(64),
        time: ago(hours),
        place,
        complete: true,
        size: Some(size),
    };
    let home: Vec<Snapshot> = (0..12)
        .map(|i| snapshot(140 - i, i as i64 + 1, SnapshotKind::Automatic, ""))
        .chain([snapshot(
            120,
            30,
            SnapshotKind::Manual,
            "Before reorganising Photos",
        )])
        .collect();
    let system = vec![
        snapshot(18, 26, SnapshotKind::After, "After update enzimos-3"),
        snapshot(17, 26, SnapshotKind::Before, "Before update enzimos-3"),
        snapshot(12, 170, SnapshotKind::After, "After update enzimos-2"),
        snapshot(11, 170, SnapshotKind::Before, "Before update enzimos-2"),
    ];
    let points = vec![
        point(3, 5, Place::Both, 21 << 30),
        point(2, 29, Place::Local, 21 << 30),
        point(1, 53, Place::Both, 20 << 30),
        point(9, 24 * 9, Place::Remote, 19 << 30),
    ];
    enzimos::Safety {
        home_history: true,
        home_history_state: "active".into(),
        root_snapshots: Some(system.len() as u64),
        home_snapshots: Some(home.len() as u64),
        latest_home: home.first().map(|s| s.date.clone()),
        local_configured: true,
        remote_configured: demo_remote().configured(),
        remote: demo_remote(),
        schedule_enabled: true,
        recovery_copy_state: "user-attested".into(),
        recovery_available: true,
        setup_available: false,
        candidates: None,
        keep_last: None,
        backup_retention: backups::BackupPreset::Recommended.policy(true),
        local_cleanup: Some(backups::Cleanup {
            state: backups::CleanupState::Removed,
            removed: Some(4),
            at_unix: chrono::Utc::now().timestamp() - 7200,
            reason: None,
        }),
        remote_cleanup: None,
        last_backup: Some(LastBackup {
            time: ago(5),
            state: "completed".into(),
            point: Some(points[0].id.clone()),
        }),
        next_run: Some(ago(-19)),
        destination_label: Some("Samsung T7 Shield".into()),
        destination_free: Some(612 << 30),
        repository_bytes: Some(212 << 30),
        lists: Some((system, home)),
        points_error: None,
        points: Some(points),
        home_retention: Some(backups::Retention {
            hourly: 24,
            daily: 7,
            weekly: 4,
            monthly: 12,
            yearly: 0,
            manual: 10,
        }),
        operations: [
            "home-retention-set",
            "backup-retention-set",
            "own-file-restore",
            "home-history-create",
        ]
        .map(String::from)
        .to_vec(),
        // ENZUI_DEMO_BACKUP_WAITING=1 shows the drive-not-connected state.
        run_state: if std::env::var_os("ENZUI_DEMO_BACKUP_WAITING").is_some() {
            "waiting-for-drive".into()
        } else {
            "ok".into()
        },
        run_since: Some(chrono::Utc::now().timestamp() - 3 * 86_400),
        last_success: Some(
            chrono::Utc::now().timestamp()
                - if std::env::var_os("ENZUI_DEMO_BACKUP_WAITING").is_some() {
                    3 * 86_400
                } else {
                    5 * 3600
                },
        ),
        run_code: None,
        repository_measured: Some(212 << 30),
        budget_over: false,
        pending_removals: 1,
        destination_available: std::env::var_os("ENZUI_DEMO_BACKUP_WAITING").is_none(),
    }
}
