use super::*;
use enzui_shell::i18n::tr;

impl SystemSettings {
    pub(super) async fn sound(&self) -> Sound {
        let run = &self.controls;
        // PulseAudio without PipeWire's tools: `pactl` for the levels and devices.
        if run.tool("wpctl").is_none() && run.tool("pactl").is_some() {
            let level = |kind: &'static str, target: &'static str| async move {
                let (volume_verb, mute_verb) =
                    (format!("get-{kind}-volume"), format!("get-{kind}-mute"));
                let (volume_args, mute_args) =
                    ([volume_verb.as_str(), target], [mute_verb.as_str(), target]);
                let (volume, mute) =
                    tokio::join!(run.run("pactl", &volume_args), run.run("pactl", &mute_args),);
                parse::pulse_level(&volume.ok()?, &mute.ok()?)
            };
            let (output, input, devices) = tokio::join!(
                level("sink", "@DEFAULT_SINK@"),
                level("source", "@DEFAULT_SOURCE@"),
                run.audio_devices(),
            );
            return Sound {
                error: output
                    .is_none()
                    .then(|| tr("shell-no-audio-output-is-available").into()),
                output,
                input,
                devices: devices.ok(),
            };
        }
        let (output, input, devices) = tokio::join!(
            run.run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]),
            run.run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
            run.audio_devices(),
        );
        let error = match (&output, run.tool("wpctl")) {
            (_, None) => Some(tr("shell-pipewire-s-wpctl-is-not-installed").into()),
            (Err(_), _) => Some(tr("shell-no-audio-output-is-available").into()),
            _ => None,
        };
        Sound {
            output: output.ok().and_then(|t| parse::level(&t)),
            input: input.ok().and_then(|t| parse::level(&t)),
            devices: devices.ok(),
            error,
        }
    }
    pub(super) async fn network(&self) -> Network {
        let run = &self.controls;
        use iced::futures::{StreamExt, stream};
        let trust = async {
            if let Some(backend) = self.enzimos().await
                && let Ok(value) = backend.query("network-trust").await
            {
                enzimos::trust_rows(&value)
            } else {
                Vec::new()
            }
        };
        let (overview, trust, connectivity) =
            tokio::join!(self.network_overview(), trust, self.connectivity());
        let Some(nm_bus::Overview {
            networking,
            wifi_radio,
            mut interfaces,
            connections: mut saved,
        }) = overview
        else {
            return Network::default();
        };
        let detailed: Vec<_> = interfaces
            .iter()
            .enumerate()
            .filter(|(_, i)| i.state == LinkState::Connected && i.kind != InterfaceKind::Vpn)
            .take(4)
            .map(|(index, interface)| (index, interface.clone()))
            .collect();
        let detailed = stream::iter(detailed.into_iter().map(|(index, mut interface)| async move {
            let args = [
                "-t",
                "-f",
                "GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP6.ADDRESS",
                "device",
                "show",
                &interface.device,
            ];
            let profile = async {
                if interface.connection.is_empty() { return None; }
                // By UUID when known: another profile may share the name.
                let (by, key) = if interface.connection_uuid.is_empty() {
                    ("id", interface.connection.as_str())
                } else {
                    ("uuid", interface.connection_uuid.as_str())
                };
                run.run("nmcli", &[
                    "-t", "-f", "connection.zone,ipv4.dns,ipv6.dns,ipv4.method,ipv4.addresses,ipv4.gateway",
                    "connection", "show", by, key,
                ]).await.ok().map(|text| parse::profile(&text))
            };
            let (details, profile) = tokio::join!(run.run("nmcli", &args), profile);
            interface.details = details.ok().map(|text| parse::details(&text));
            interface.profile = profile;
            (index, interface)
        })).buffer_unordered(4).collect::<Vec<_>>().await;
        for (index, interface) in detailed {
            interfaces[index] = interface;
        }
        // The installed backend owns trust (D-072); its answer wins.
        for interface in &mut interfaces {
            if let (Some(profile), Some(row)) = (
                interface.profile.as_mut(),
                trust.iter().find(|row| row.name == interface.connection),
            ) {
                profile.trusted = row.trusted;
            }
        }
        let vpns: Vec<_> = saved
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == ConnectionKind::Vpn)
            .take(16)
            .map(|(index, vpn)| (index, vpn.uuid.clone()))
            .collect();
        // One `nmcli` for every VPN: each start loads all of NetworkManager's
        // objects, which for a dozen VPNs read one by one cost more than the
        // rest of the page.
        if !vpns.is_empty() {
            let mut args = vec![
                "-t",
                "-f",
                "connection.uuid,connection.type,vpn.service-type,ipv4.never-default,connection.metered,vpn.data,wireguard.peers,wireguard.peer-routes",
                "connection",
                "show",
            ];
            for (_, uuid) in &vpns {
                args.extend(["uuid", uuid.as_str()]);
            }
            if let Ok(text) = run.run("nmcli", &args).await {
                let details = parse::vpns(&text);
                for (index, uuid) in &vpns {
                    saved[*index].vpn = details.get(uuid).cloned();
                }
            }
        }
        let access_points = if interfaces.iter().any(|i| i.kind == InterfaceKind::Wifi) {
            let points = self.access_points(&saved, false).await.unwrap_or_default();
            // An empty list usually means no scan has run since the radio
            // came up; ask for one, at most every 30 seconds.
            static LAST_SCAN: std::sync::Mutex<Option<std::time::Instant>> =
                std::sync::Mutex::new(None);
            let due = {
                let mut last = LAST_SCAN
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let due = last.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(30));
                if points.is_empty() && due {
                    *last = Some(std::time::Instant::now());
                }
                points.is_empty() && due
            };
            if due {
                let controls = self.controls.clone();
                // The next refresh consumes scan results; radio scans must not hold the page open.
                tokio::spawn(async move {
                    let _ = controls.run("nmcli", &["device", "wifi", "rescan"]).await;
                });
            }
            points
        } else {
            Vec::new()
        };
        // Read without asking firewalld: its D-Bus reads can need an
        // administrator on locked-down systems, and a page must never prompt.
        let firewall = match run.tool("firewall-cmd") {
            None => None,
            Some(_) => {
                let running = run
                    .run("systemctl", &["is-active", "firewalld.service"])
                    .await
                    .is_ok_and(|t| t.trim() == "active");
                let config =
                    std::fs::read_to_string("/etc/firewalld/firewalld.conf").unwrap_or_default();
                let default_zone = config
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("DefaultZone="))
                    .map(|z| z.trim().chars().take(64).collect())
                    .filter(|z: &String| !z.is_empty())
                    .unwrap_or_else(|| "public".to_owned());
                Some(Firewall {
                    running,
                    default_zone,
                })
            }
        };
        Network {
            networking: Some(networking),
            firewall,
            wifi_radio,
            interfaces,
            access_points,
            wifi: saved
                .iter()
                .filter(|c| c.kind == ConnectionKind::Wifi)
                .cloned()
                .collect(),
            vpns: saved
                .into_iter()
                .filter(|c| c.kind == ConnectionKind::Vpn)
                .collect(),
            connectivity,
        }
    }
    async fn network_overview(&self) -> Option<nm_bus::Overview> {
        let run = &self.controls;
        if run.uses_bus()
            && let Some(bus) = crate::bus::system().await
            && let Some(overview) = nm_bus::overview(&bus).await
        {
            return Some(overview);
        }
        let (state, devices, radio, connections) = tokio::join!(
            run.run("nmcli", &["-t", "-f", "NETWORKING", "general"]),
            run.run(
                "nmcli",
                &[
                    "-t",
                    "-f",
                    "DEVICE,TYPE,STATE,CONNECTION",
                    "device",
                    "status"
                ]
            ),
            run.run("nmcli", &["-t", "-f", "WIFI", "radio"]),
            run.run(
                "nmcli",
                &["-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"]
            ),
        );
        Some(nm_bus::Overview {
            networking: state.ok()?.trim() == "enabled",
            wifi_radio: radio.ok().and_then(|v| match v.trim() {
                "enabled" => Some(true),
                "disabled" => Some(false),
                _ => None,
            }),
            interfaces: parse::interfaces(devices.as_deref().unwrap_or_default()),
            connections: parse::connections(connections.as_deref().unwrap_or_default()),
        })
    }

    async fn access_points(
        &self,
        saved: &[SavedConnection],
        rescan: bool,
    ) -> Result<Vec<AccessPoint>, String> {
        let names = saved
            .iter()
            .filter(|c| c.kind == ConnectionKind::Wifi)
            .map(|c| c.ssid.clone().unwrap_or_else(|| c.name.clone()))
            .collect::<Vec<_>>();
        let args = [
            "-t",
            "-f",
            "IN-USE,SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "--rescan",
            if rescan { "yes" } else { "no" },
        ];
        let output = if rescan {
            self.controls
                .run_with("nmcli", &args, None, std::time::Duration::from_secs(12))
                .await
                .map_err(|_| tr("shell-network-settings-are-unavailable").to_owned())?
        } else {
            self.controls.run("nmcli", &args).await?
        };
        Ok(parse::access_points(&output, &names))
    }

    /// Read the links and access points used by the Control Center.
    pub async fn control_network(&self, rescan: bool) -> Result<Network, String> {
        let nm_bus::Overview {
            networking,
            wifi_radio,
            interfaces,
            connections,
        } = self
            .network_overview()
            .await
            .ok_or_else(|| tr("shell-network-settings-are-unavailable").to_owned())?;
        let access_points = if wifi_radio == Some(true)
            && interfaces
                .iter()
                .any(|item| item.kind == InterfaceKind::Wifi)
        {
            self.access_points(&connections, rescan && self.writes_allowed)
                .await?
        } else {
            Vec::new()
        };
        Ok(Network {
            networking: Some(networking),
            wifi_radio,
            interfaces,
            access_points,
            ..Network::default()
        })
    }

    // Connectivity reads do not trigger NetworkManager probes.
    pub(super) async fn connectivity(&self) -> crate::network_connectivity::Connectivity {
        use crate::network_connectivity::{Connectivity, State, busctl_value, valid_uri};
        let run = &self.controls;
        // One bus read instead of an nmcli and three busctl processes.
        if run.uses_bus()
            && let Some(system) = crate::bus::system().await
            && let Some(connectivity) = crate::network_connectivity::current(&system).await
        {
            return Connectivity {
                devices: None,
                ..connectivity
            };
        }
        let property = |name: &'static str| async move {
            let args = [
                "--system",
                "get-property",
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                name,
            ];
            run.run("busctl", &args)
                .await
                .ok()
                .and_then(|text| busctl_value(&text))
        };
        let (state, available, enabled, uri) = tokio::join!(
            run.run("nmcli", &["-t", "-f", "CONNECTIVITY", "general"]),
            property("ConnectivityCheckAvailable"),
            property("ConnectivityCheckEnabled"),
            property("ConnectivityCheckUri"),
        );
        Connectivity {
            state: state
                .map(|text| State::from_word(&text))
                .unwrap_or_default(),
            checks: (available.as_deref() == Some("true"))
                .then(|| enabled.as_deref() == Some("true")),
            uri: uri.filter(|uri| valid_uri(uri)),
            devices: None,
        }
    }
    pub(super) async fn bluetooth(&self) -> Bluetooth {
        let present =
            std::fs::read_dir("/sys/class/bluetooth").is_ok_and(|mut e| e.next().is_some());
        if !present {
            return Bluetooth::default();
        }
        let run = &self.controls;
        let Some(adapter) = run
            .run("bluetoothctl", &["--timeout", "1", "show"])
            .await
            .ok()
            .and_then(|t| parse::adapter(&t))
        else {
            return Bluetooth::default();
        };
        let (paired, connected) = tokio::join!(
            run.run("bluetoothctl", &["devices", "Paired"]),
            run.run("bluetoothctl", &["devices", "Connected"]),
        );
        let connected = parse::device_lines(connected.as_deref().unwrap_or_default());
        let mut devices = Vec::new();
        for (address, name) in parse::device_lines(paired.as_deref().unwrap_or_default())
            .into_iter()
            .take(16)
        {
            let kind = run
                .run("bluetoothctl", &["info", &address])
                .await
                .map_or(DeviceKind::Other, |t| parse::device_icon(&t));
            devices.push(BluetoothDevice {
                connected: connected.iter().any(|(a, _)| *a == address),
                address,
                name,
                kind,
                paired: true,
            });
        }
        devices.sort_by_key(|d| !d.connected);
        let all = run.run("bluetoothctl", &["devices"]).await;
        let nearby = parse::device_lines(all.as_deref().unwrap_or_default())
            .into_iter()
            .filter(|(address, name)| {
                !devices.iter().any(|d| d.address == *address)
                    && !name.is_empty()
                    && name.replace('-', ":") != *address
            })
            .take(12)
            .map(|(address, name)| BluetoothDevice {
                address,
                name,
                kind: DeviceKind::Other,
                paired: false,
                connected: false,
            })
            .collect();
        Bluetooth {
            adapter: Some(adapter),
            devices,
            nearby,
        }
    }
    pub(super) async fn power(&self) -> Power {
        let run = &self.controls;
        let battery = match run
            .run(
                "upower",
                &["-i", "/org/freedesktop/UPower/devices/DisplayDevice"],
            )
            .await
        {
            Ok(text) => parse::upower_battery(&text),
            Err(_) => run.battery().await.map(|b| BatteryState {
                percent: b.percent,
                charging: b.charging,
                state: if b.charging {
                    tr("shell-charging")
                } else {
                    tr("shell-on-battery")
                }
                .into(),
                remaining: None,
            }),
        };
        let mut profiles = None;
        for (service, path, interface) in PROFILE_SERVICES {
            let active = run
                .run(
                    "busctl",
                    &[
                        "--system",
                        "get-property",
                        service,
                        path,
                        interface,
                        "ActiveProfile",
                    ],
                )
                .await;
            if let Some(active) = active.ok().and_then(|t| parse::busctl_string(&t)) {
                let list = run
                    .run(
                        "busctl",
                        &[
                            "--system",
                            "--json=short",
                            "get-property",
                            service,
                            path,
                            interface,
                            "Profiles",
                        ],
                    )
                    .await;
                let mut available = parse::busctl_profiles(list.as_deref().unwrap_or_default());
                if available.is_empty() {
                    available = vec!["power-saver".into(), "balanced".into()];
                }
                profiles = Some(Profiles { active, available });
                break;
            }
        }
        let lid = run
            .run(
                "busctl",
                &[
                    "--system",
                    "get-property",
                    "org.freedesktop.UPower",
                    "/org/freedesktop/UPower",
                    "org.freedesktop.UPower",
                    "LidIsPresent",
                ],
            )
            .await
            .is_ok_and(|t| t.trim() == "b true");
        let critical = match self.enzimos().await {
            None => Backed::Missing,
            Some(backend) => match backend.query("power-policy").await {
                Ok(value) => enzimos::critical_battery(&value).map_or_else(
                    || Backed::Refused(enzimos::Refusal::Invalid.message()),
                    Backed::Ready,
                ),
                // An enzimOS backend from before this preference.
                Err(enzimos::Refusal::Code(code)) if code == "operation-unavailable" => {
                    Backed::Missing
                }
                Err(refusal) => Backed::Refused(refusal.message()),
            },
        };
        let memory = match self.enzimos().await {
            None => Backed::Missing,
            Some(backend) => match backend.query("memory-policy").await {
                Ok(value) => enzimos::memory_policy(&value).map_or_else(
                    || Backed::Refused(enzimos::Refusal::Invalid.message()),
                    Backed::Ready,
                ),
                Err(enzimos::Refusal::Code(code)) if code == "operation-unavailable" => {
                    Backed::Missing
                }
                Err(refusal) => Backed::Refused(refusal.message()),
            },
        };
        let tmp_storage = match self.enzimos().await {
            None => Backed::Missing,
            Some(backend) => match backend.query("tmp-storage").await {
                Ok(value) => enzimos::tmp_storage(&value).map_or_else(
                    || Backed::Refused(enzimos::Refusal::Invalid.message()),
                    Backed::Ready,
                ),
                Err(enzimos::Refusal::Code(code)) if code == "operation-unavailable" => {
                    Backed::Missing
                }
                Err(refusal) => Backed::Refused(refusal.message()),
            },
        };
        let (game_mode, game_preferences) = tokio::join!(
            crate::game_mode::read(),
            tokio::task::spawn_blocking(crate::game_mode::preferences::read)
        );
        Power {
            battery,
            lid,
            profiles,
            critical,
            memory,
            tmp_storage,
            game_mode,
            game_preferences: game_preferences.unwrap_or_default(),
        }
    }
    pub(super) async fn discover_printers(&self) -> Vec<Discovered> {
        // Both plain and TLS IPP; ipp-usb publishes USB printers on localhost.
        let find = |service: &'static str| async move {
            let args = [
                "-T",
                "4",
                service,
                "--exec",
                "echo",
                "{service_name}\t{}",
                ";",
            ];
            self.controls
                .run_with("ippfind", &args, None, std::time::Duration::from_secs(8))
                .await
                .unwrap_or_default()
        };
        // Older USB and network printers come from CUPS's own backends.
        let legacy = async {
            self.controls
                .run_with(
                    "lpinfo",
                    &["-l", "--timeout", "8", "-v"],
                    None,
                    std::time::Duration::from_secs(20),
                )
                .await
                .unwrap_or_default()
        };
        let ((secure, plain), legacy) = tokio::join!(
            async { tokio::join!(find("_ipps._tcp"), find("_ipp._tcp")) },
            legacy
        );
        let added = self.printers().await.printers;
        // TLS first, so a printer offering both is added over ipps.
        let mut found: Vec<Discovered> = parse::discovered(&format!("{secure}\n{plain}"));
        for (name, uri, id) in parse::legacy_devices(&legacy) {
            // A USB printer that also prints driverless over IPP (ipp-usb)
            // is already listed under its own name.
            if found.iter().any(|d| d.name == name) {
                continue;
            }
            let mut drivers = String::new();
            if !id.is_empty() {
                drivers = self
                    .controls
                    .run_with(
                        "lpinfo",
                        &["--device-id", &id, "-m"],
                        None,
                        std::time::Duration::from_secs(15),
                    )
                    .await
                    .unwrap_or_default();
            }
            if drivers.trim().is_empty() {
                drivers = self
                    .controls
                    .run_with(
                        "lpinfo",
                        &["--make-and-model", &name, "-m"],
                        None,
                        std::time::Duration::from_secs(15),
                    )
                    .await
                    .unwrap_or_default();
            }
            let driver = parse::best_driver(&drivers);
            found.push(Discovered {
                name,
                uri,
                unsupported: driver.is_none(),
                driver,
            });
        }
        found
            .into_iter()
            .filter(|d| !added.iter().any(|p| p.name == printer_queue_name(&d.name)))
            .collect()
    }
    pub(super) async fn printers(&self) -> Printers {
        let run = &self.controls;
        if run.tool("lpstat").is_none() {
            return Printers::default();
        }
        let running = run
            .run("lpstat", &["-r"])
            .await
            .is_ok_and(|t| t.contains("scheduler is running"));
        if !running {
            return Printers {
                scheduler: Some(false),
                ..Printers::default()
            };
        }
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        // `-u` alone: lpstat lists once per listing option, so `-o` beside
        // it would repeat every job.
        let job_args = ["-W", "not-completed", "-l", "-u", user.as_str()];
        let (status, default, jobs, admin) = tokio::join!(
            run.run("lpstat", &["-l", "-p"]),
            run.run("lpstat", &["-d"]),
            run.run("lpstat", &job_args),
            crate::cups_admin::available(),
        );
        Printers {
            scheduler: Some(true),
            printers: parse::printers(
                status.as_deref().unwrap_or_default(),
                default.as_deref().unwrap_or_default(),
            ),
            jobs: if user.is_empty() {
                Vec::new()
            } else {
                crate::cups_admin::jobs(jobs.as_deref().unwrap_or_default(), &user)
            },
            admin,
        }
    }
    pub(super) async fn clock(&self) -> Clock {
        static ZONES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let zones = match ZONES.get() {
            Some(zones) => zones.clone(),
            None => {
                let listed = self.controls.run("timedatectl", &["list-timezones"]).await;
                let zones: Vec<String> = listed
                    .as_deref()
                    .unwrap_or_default()
                    .lines()
                    .filter(|z| parse::valid_zone(z))
                    .map(str::to_owned)
                    .collect();
                if !zones.is_empty() {
                    let _ = ZONES.set(zones.clone());
                }
                zones
            }
        };
        let mut clock = self.clock_state().await;
        clock.zones = zones;
        clock
    }
    async fn clock_state(&self) -> Clock {
        match self.controls.run("timedatectl", &["show"]).await {
            Ok(text) => parse::clock(&text),
            Err(_) => Clock {
                timezone: std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                    let text = p.to_string_lossy().into_owned();
                    text.split_once("zoneinfo/")
                        .map(|(_, zone)| zone.to_owned())
                }),
                ..Clock::default()
            },
        }
    }
    pub(super) async fn about(&self) -> About {
        let read = |path: &str| std::fs::read_to_string(path).ok();
        let gpus = self
            .controls
            .run("lspci", &["-mm", "-d", "::0300"])
            .await
            .map(|t| parse::gpus(&t))
            .unwrap_or_default();
        let gpus = if gpus.is_empty() {
            let three_d = self.controls.run("lspci", &["-mm", "-d", "::0302"]).await;
            three_d.map(|t| parse::gpus(&t)).unwrap_or_default()
        } else {
            gpus
        };
        let disk = rustix::fs::statvfs("/").ok().map(|s| {
            let block = s.f_frsize.max(1);
            (s.f_blocks * block, s.f_bavail * block)
        });
        let compositor = if std::env::var_os("ENZUI_HYPR_RUNROOT").is_some() {
            Some("enz-hyprland".to_owned())
        } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            Some("Hyprland".to_owned())
        } else if std::env::var_os("SWAYSOCK").is_some()
            || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|_| {
                std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.eq_ignore_ascii_case("sway"))
            })
        {
            Some("Sway".to_owned())
        } else {
            std::env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .filter(|d| !d.is_empty() && d.len() <= 32 && !d.eq_ignore_ascii_case("enzui"))
        };
        let enzimos = match self.enzimos().await {
            Some(backend) => backend
                .query("system")
                .await
                .ok()
                .map(|v| enzimos::SystemState::from_json(&v)),
            None => None,
        };
        About {
            enzimos,
            crashes: crate::crash_reports::read().await.map(std::sync::Arc::new),
            compositor,
            hostname: read("/proc/sys/kernel/hostname")
                .map(|t| t.trim().chars().take(64).collect()),
            os: read("/etc/os-release")
                .or_else(|| read("/usr/lib/os-release"))
                .and_then(|t| parse::os_release(&t)),
            kernel: read("/proc/sys/kernel/osrelease").map(|t| t.trim().to_owned()),
            cpu: read("/proc/cpuinfo").and_then(|t| parse::cpu_model(&t)),
            threads: std::thread::available_parallelism().ok().map(usize::from),
            memory_kib: read("/proc/meminfo").and_then(|t| parse::memory_kib(&t)),
            gpus,
            disk,
        }
    }
    pub(super) async fn updates(&self) -> Backed<enzimos::SystemState> {
        let Some(backend) = self.enzimos().await else {
            return Backed::Missing;
        };
        match backend.query("system").await {
            Ok(value) => Backed::Ready(enzimos::SystemState::from_json(&value)),
            Err(refusal) => Backed::Refused(refusal.message()),
        }
    }
    pub(super) async fn safety(&self) -> Backed<enzimos::Safety> {
        let Some(backend) = self.enzimos().await else {
            return Backed::Missing;
        };
        let (history, backups, queries, operations, capabilities) = tokio::join!(
            backend.query("history"),
            backend.query("backups"),
            backend.queries(),
            backend.operations(),
            backend.capabilities()
        );
        let mut safety = match (history, backups) {
            (Ok(history), Ok(backups)) => enzimos::Safety::from_json(&history, &backups),
            (Err(refusal), _) | (_, Err(refusal)) => return Backed::Refused(refusal.message()),
        };
        safety.operations = operations;
        let capabilities = capabilities.ok();
        safety.setup_available = capabilities
            .as_ref()
            .is_some_and(super::backup_setup::available);
        if safety.setup_available
            && !safety.local_configured
            && let Ok(value) = backend.query("backup-candidates").await
        {
            safety.candidates = super::backup_setup::candidates(&value).ok();
        }
        safety.recovery_available = capabilities.as_ref().is_some_and(|caps| {
            caps.pointer("/backup_recovery/schema")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
                && caps
                    .pointer("/backup_recovery/export")
                    .and_then(serde_json::Value::as_str)
                    == Some("BackupRecoveryExport(s)->sh")
                && caps
                    .pointer("/backup_recovery/confirm")
                    .and_then(serde_json::Value::as_str)
                    == Some("BackupRecoveryConfirm(s)->s")
        });
        // Lists only from releases that offer them; a failed list leaves that part out.
        let offered = |name: &str| queries.iter().any(|query| query == name);
        if offered("snapshots")
            && let Ok(value) = backend.query("snapshots").await
        {
            safety.lists = Some(super::backups::snapshots(&value));
        }
        if offered("backup-points") {
            match backend.query("backup-points").await {
                Ok(value) => safety.points = Some(super::backups::points(&value)),
                Err(refusal) => safety.points_error = Some(refusal.message()),
            }
        }
        Backed::Ready(safety)
    }
}
