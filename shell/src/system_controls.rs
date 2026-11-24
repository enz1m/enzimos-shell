use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::{
    collections::VecDeque,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Audio {
    pub percent: u8,
    pub muted: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub ethernet: bool,
    pub ethernet_present: bool,
    pub wifi_present: bool,
    pub wifi_connected: bool,
    pub wifi_enabled: Option<bool>,
    /// NetworkManager's internet check, kept by the shell as it changes.
    pub connectivity: crate::network_connectivity::State,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Brightness {
    pub device: String,
    pub percent: u8,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Mains or USB power is connected, whether or not the battery charges.
    pub external_power: bool,
}
/// The glyph for a battery's charge. Lucide's plain `battery` is an empty
/// outline, so every level names one that shows how full it is.
#[must_use]
pub fn battery_icon(percent: u8, charging: bool) -> &'static str {
    match (charging, percent) {
        (true, _) => "battery-charging",
        (false, 0..=15) => "battery-low",
        (false, 16..=60) => "battery-medium",
        _ => "battery-full",
    }
}
impl Battery {
    #[must_use]
    pub fn icon(&self) -> &'static str {
        battery_icon(self.percent, self.charging)
    }
}
#[derive(Clone, Debug, Default)]
pub struct SystemSnapshot {
    pub audio: Option<Audio>,
    /// The default input's level and mute.
    pub microphone: Option<Audio>,
    pub network: Network,
    pub bluetooth: Option<bool>,
    pub phone: Option<crate::phone_link::Presence>,
    pub brightness: Option<Brightness>,
    pub battery: Option<Battery>,
    pub screenshot: bool,
    /// Whether voice typing (`enzimos-dictation`) is installed.
    pub dictation: bool,
    pub lock: bool,
    pub power: bool,
    pub power_mode: Option<crate::power_mode::PowerMode>,
    pub game_mode: Option<crate::game_mode::Status>,
    /// Removable drives connected now; `None` without UDisks2.
    pub drives: Option<Vec<crate::drives::Mounted>>,
    /// Whether the power mode may be changed and drives ejected from here:
    /// an installed login, not a preview on someone's own computer.
    pub system_writes: bool,
    /// What this computer offers; `None` until known.
    pub platform: Option<crate::platform::Platform>,
}

impl SystemSnapshot {
    pub fn available(&self, action: crate::settings::QuickAction) -> bool {
        use crate::settings::QuickAction;
        match action {
            QuickAction::Network => self.network.ethernet_present || self.network.wifi_present,
            QuickAction::Bluetooth => self.bluetooth.is_some(),
            QuickAction::Phone => self.phone.is_some() || crate::phone_link::available(),
            QuickAction::Volume => self.audio.is_some(),
            QuickAction::Brightness => self.brightness.is_some(),
            QuickAction::Screenshot => self.screenshot,
            QuickAction::Dictation => self.dictation,
            QuickAction::Lock => self.lock,
            QuickAction::Power => self.power,
            QuickAction::PowerMode => self.power_mode.is_some(),
            QuickAction::GameMode => self
                .platform
                .as_ref()
                .is_none_or(|platform| platform.offers(crate::platform::Feature::GameMode)),
            QuickAction::Drives => self.drives.is_some(),
            QuickAction::Appearance
            | QuickAction::Tiling
            | QuickAction::Dnd
            | QuickAction::NightLight
            | QuickAction::Keyboard => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Volume(u8),
    Mute(bool),
    MicrophoneMute(bool),
    AudioDevice(crate::audio_devices::Device),
    Brightness(u8),
    Wifi(bool),
    Bluetooth(bool),
    Lock,
    /// Locks, then suspends; only where system power actions are allowed.
    Suspend,
    PowerOff,
    Restart,
    /// Power off or restart although an app's inhibitor lock blocks it;
    /// polkit decides whether that is allowed.
    PowerOffAnyway,
    RestartAnyway,
    LogOut,
    PowerMode(crate::power_mode::Profile),
    GameMode(bool),
    Mount(crate::drives::Mounted),
    Eject(crate::drives::Mounted),
}

#[derive(Default)]
pub struct ActionQueue(VecDeque<Action>);
impl ActionQueue {
    pub fn push(&mut self, action: Action) {
        self.0.retain(|old| match (old, &action) {
            (Action::AudioDevice(a), Action::AudioDevice(b)) => a.kind != b.kind,
            _ => std::mem::discriminant(old) != std::mem::discriminant(&action),
        });
        if self.0.len() < 8 {
            self.0.push_back(action);
        }
    }
    pub fn pop(&mut self) -> Option<Action> {
        self.0.pop_front()
    }
}

// Why a system tool call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The tool ran and refused; carries its last error line, if any.
    Rejected(String),
    Other(String),
}

#[derive(Clone)]
pub struct SystemControls {
    tools: Vec<PathBuf>,
    sysfs: PathBuf,
    pub allow_power: bool,
    /// Reads NetworkManager and BlueZ over the system bus; without it, or
    /// when they do not answer there, their command-line tools are run.
    bus: bool,
    login_compositor: Option<crate::compositor::Compositor>,
}
impl SystemControls {
    /// Whether NetworkManager and BlueZ are read over the system bus.
    pub(crate) fn uses_bus(&self) -> bool {
        self.bus
    }
    pub fn from_environment() -> Self {
        Self {
            tools: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|p| p.is_absolute())
                .collect(),
            sysfs: PathBuf::from("/sys"),
            allow_power: std::env::var("ENZUI_ALLOW_SYSTEM_POWER").as_deref() == Ok("1"),
            bus: true,
            login_compositor: std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()))
                .and_then(|_| crate::compositor::Compositor::from_environment().ok()),
        }
    }
    /// Tools and sysfs from `root` only, and no bus: for tests.
    #[cfg(test)]
    pub(crate) fn fixture(root: &Path) -> Self {
        Self {
            tools: vec![root.into()],
            sysfs: root.into(),
            allow_power: false,
            bus: false,
            login_compositor: None,
        }
    }
    pub(crate) fn tool(&self, name: &str) -> Option<PathBuf> {
        self.tools.iter().map(|dir| dir.join(name)).find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    }
    /// Starts or stops a hands-free voice typing take.
    pub async fn toggle_dictation(&self) -> Result<String, String> {
        self.run("enzimos-dictation", &["toggle"]).await
    }
    pub(crate) async fn run(&self, name: &str, args: &[&str]) -> Result<String, String> {
        self.run_with(name, args, None, Duration::from_secs(3))
            .await
            .map_err(|failure| match failure {
                Failure::Rejected(_) => tr!("shell-rejected-the-request-check-device-permissions", "arg0" => name.to_owned()),
                Failure::Other(message) => message,
            })
    }
    // Runs a bounded tool with optional stdin for secrets; reports its last error line on
    // refusal.
    pub(crate) async fn run_with(
        &self,
        name: &str,
        args: &[&str],
        input: Option<&str>,
        limit: Duration,
    ) -> Result<String, Failure> {
        let executable = self.tool(name).ok_or_else(|| {
            Failure::Other(tr!("shell-is-not-installed", "arg0" => name.to_owned()))
        })?;
        let result = tokio::time::timeout(limit, async {
            let mut child = Command::new(executable)
                .args(args)
                .env("LC_ALL", "C")
                .env("NO_COLOR", "1")
                .stdin(if input.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| {
                    Failure::Other(tr!("shell-could-not-start", "arg0" => name.to_owned()))
                })?;
            if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
                use tokio::io::AsyncWriteExt;
                stdin
                    .write_all(input.as_bytes())
                    .await
                    .map_err(|_| Failure::Other(tr("shell-could-not-send-the-request").into()))?;
            }
            let bound = if name == "pw-dump" {
                1024 * 1024
            } else {
                65536
            };
            let mut bytes = Vec::new();
            let mut errors = Vec::new();
            let mut stdout = child
                .stdout
                .take()
                .ok_or_else(|| Failure::Other(tr("shell-missing-command-output").into()))?
                .take(bound + 1);
            let mut stderr = child
                .stderr
                .take()
                .ok_or_else(|| Failure::Other(tr("shell-missing-command-output").into()))?
                .take(4096);
            let (out, _) = tokio::join!(
                stdout.read_to_end(&mut bytes),
                stderr.read_to_end(&mut errors)
            );
            out.map_err(|_| Failure::Other(tr("shell-could-not-read-control-response").into()))?;
            if bytes.len() as u64 > bound {
                return Err(Failure::Other(
                    tr("shell-control-response-exceeded-its-bound").into(),
                ));
            }
            if !child
                .wait()
                .await
                .map_err(|_| {
                    Failure::Other(tr("shell-could-not-wait-for-control-response").into())
                })?
                .success()
            {
                let text = String::from_utf8_lossy(&errors);
                // nmcli ends with a journalctl hint naming the connection's
                // UUID and device; the error itself is the line before it.
                let line = text
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with("Hint:"))
                    .unwrap_or("");
                return Err(Failure::Rejected(line.chars().take(200).collect()));
            }
            String::from_utf8(bytes)
                .map_err(|_| Failure::Other(tr("shell-invalid-control-response").into()))
        })
        .await;
        result.unwrap_or_else(|_| {
            Err(Failure::Other(
                tr!("shell-did-not-respond", "arg0" => name.to_owned()),
            ))
        })
    }
    /// PulseAudio without PipeWire's tools: `pactl` reads and sets the
    /// levels and the default devices.
    pub(crate) fn pulse_only(&self) -> bool {
        self.tool("wpctl").is_none() && self.tool("pactl").is_some()
    }
    async fn pulse_level(&self, kind: crate::audio_devices::Kind) -> Result<Audio, String> {
        let (volume_verb, mute_verb) = (
            format!("get-{}-volume", kind.pulse_noun()),
            format!("get-{}-mute", kind.pulse_noun()),
        );
        let volume_args = [volume_verb.as_str(), kind.pulse_default()];
        let mute_args = [mute_verb.as_str(), kind.pulse_default()];
        let (volume, mute) = tokio::join!(
            self.run("pactl", &volume_args),
            self.run("pactl", &mute_args),
        );
        let (percent, muted) = crate::audio_devices::pulse_level(&volume?, &mute?)
            .ok_or_else(|| tr("shell-invalid-audio-device-response").to_owned())?;
        Ok(Audio { percent, muted })
    }
    async fn pulse_devices(&self) -> Result<crate::audio_devices::Devices, String> {
        use crate::audio_devices::{self, Kind};
        let (sink_args, source_args) = (
            ["--format=json", "list", "sinks"],
            ["--format=json", "list", "sources"],
        );
        let (sinks, sources, sink, source) = tokio::join!(
            self.run("pactl", &sink_args),
            self.run("pactl", &source_args),
            self.run("pactl", &["get-default-sink"]),
            self.run("pactl", &["get-default-source"]),
        );
        let nodes = audio_devices::parse_pulse(&sinks?, &sources?)?;
        let default = |kind: Kind, name: Result<String, String>| {
            let name = name.ok()?;
            nodes
                .iter()
                .find(|d| d.kind == kind && d.name == name.trim())
                .map(|d| d.serial)
        };
        let (output, input) = (default(Kind::Output, sink), default(Kind::Input, source));
        Ok(audio_devices::Devices {
            nodes,
            output,
            input,
        })
    }
    async fn pulse_set_default(&self, device: &crate::audio_devices::Device) -> Result<(), String> {
        let current = self.pulse_devices().await?;
        if !current
            .nodes
            .iter()
            .any(|d| d.kind == device.kind && d.id == device.id && d.name == device.name)
        {
            return Err(tr("shell-audio-device-changed-refresh-the-device-list").into());
        }
        let verb = format!("set-default-{}", device.kind.pulse_noun());
        self.run("pactl", &[&verb, &device.name]).await?;
        let verb = format!("get-default-{}", device.kind.pulse_noun());
        if self.run("pactl", &[&verb]).await?.trim() != device.name {
            return Err(tr("shell-audio-device-did-not-become-the-default").into());
        }
        Ok(())
    }
    pub async fn audio_devices(&self) -> Result<crate::audio_devices::Devices, String> {
        use crate::audio_devices::{self, Kind};
        if self.pulse_only() {
            return self.pulse_devices().await;
        }
        let output_args = ["inspect", Kind::Output.default_target()];
        let input_args = ["inspect", Kind::Input.default_target()];
        let (dump, output, input) = tokio::join!(
            self.run("pw-dump", &[]),
            self.run("wpctl", &output_args),
            self.run("wpctl", &input_args),
        );
        Ok(audio_devices::Devices {
            nodes: audio_devices::parse_dump(&dump?)?,
            output: output.ok().and_then(|text| audio_devices::serial(&text)),
            input: input.ok().and_then(|text| audio_devices::serial(&text)),
        })
    }
    pub async fn snapshot(&self) -> SystemSnapshot {
        self.snapshot_with(true, true).await
    }
    /// The system's state; without `network` or `audio`, those are left at
    /// their defaults for a caller that follows NetworkManager's signals or
    /// PipeWire's changes instead.
    pub async fn snapshot_with(&self, network: bool, audio: bool) -> SystemSnapshot {
        let read_network = network;
        let volume = |kind: crate::audio_devices::Kind| async move {
            if !audio {
                None
            } else if self.pulse_only() {
                self.pulse_level(kind).await.ok()
            } else {
                let node = kind.default_target();
                parse_audio(&self.run("wpctl", &["get-volume", node]).await.ok()?)
            }
        };
        let bluetooth = async {
            let mut entries = tokio::fs::read_dir(self.sysfs.join("class/bluetooth"))
                .await
                .map_err(|_| tr("shell-no-bluetooth-adapter"))?;
            if entries
                .next_entry()
                .await
                .map_err(|_| tr("shell-cannot-inspect-bluetooth-adapters"))?
                .is_none()
            {
                return Err(tr("shell-no-bluetooth-adapter").to_owned());
            }
            if self.bus
                && let Some(powered) = bluetooth_powered().await
            {
                return Ok(format!("Powered: {}", if powered { "yes" } else { "no" }));
            }
            self.run("bluetoothctl", &["--timeout", "1", "show"]).await
        };
        // The bus answers in microseconds; each tool is a process.
        let network = async {
            if !read_network {
                return Network::default();
            }
            if self.bus
                && let Some(system) = crate::bus::system().await
                && let Some(devices) = crate::network_connectivity::devices(&system).await
            {
                return Network::from(devices);
            }
            let (devices, radio) = tokio::join!(
                self.run("nmcli", &["-t", "-f", "TYPE,STATE", "device", "status"]),
                self.run("nmcli", &["-t", "-f", "WIFI", "radio"]),
            );
            let mut network = parse_network(devices.as_deref().unwrap_or_default());
            network.wifi_enabled = radio.ok().and_then(|v| match v.trim() {
                "enabled" => Some(true),
                "disabled" => Some(false),
                _ => None,
            });
            network
        };
        let (audio, microphone, network, bluetooth) = tokio::join!(
            volume(crate::audio_devices::Kind::Output),
            volume(crate::audio_devices::Kind::Input),
            network,
            bluetooth
        );
        let platform = async {
            if self.bus {
                Some(crate::platform::current().await)
            } else {
                None
            }
        };
        let (power_mode, drives, game_mode, platform) = tokio::join!(
            crate::power_mode::read(),
            crate::drives::mounted(),
            crate::game_mode::read(),
            platform
        );
        let phone = tokio::time::timeout(Duration::from_millis(300), crate::phone_link::presence())
            .await
            .ok()
            .flatten();
        SystemSnapshot {
            phone,
            platform,
            power_mode,
            drives,
            game_mode,
            system_writes: self.allow_power
                || crate::desktop_session::mode() == crate::desktop_session::Mode::Installed,
            audio,
            microphone,
            network,
            bluetooth: bluetooth.ok().and_then(|s| parse_bluetooth(&s)),
            brightness: self.brightness().await,
            battery: self.battery().await,
            screenshot: self.tool("grim").is_some(),
            dictation: self.tool("enzimos-dictation").is_some(),
            lock: crate::session_lock::command().is_some() || self.tool("swaylock").is_some(),
            power: self.allow_power && self.tool("systemctl").is_some(),
        }
    }
    async fn brightness(&self) -> Option<Brightness> {
        let mut entries = tokio::fs::read_dir(self.sysfs.join("class/backlight"))
            .await
            .ok()?;
        for _ in 0..16 {
            let Some(entry) = entries.next_entry().await.ok()? else {
                break;
            };
            let device = entry.file_name().to_str()?.to_owned();
            if !device_name(&device) {
                continue;
            }
            let current = number(&entry.path().join("brightness")).await;
            let maximum = number(&entry.path().join("max_brightness")).await;
            if let (Some(current), Some(maximum)) = (current, maximum)
                && maximum > 0
                && current <= maximum
            {
                return Some(Brightness {
                    device,
                    percent: ((current as f64 / maximum as f64) * 100.0).round() as u8,
                });
            }
        }
        None
    }
    pub(crate) async fn battery(&self) -> Option<Battery> {
        let mut entries = tokio::fs::read_dir(self.sysfs.join("class/power_supply"))
            .await
            .ok()?;
        let mut battery = None;
        let mut adapters_online = Vec::new();
        for _ in 0..32 {
            let Some(entry) = entries.next_entry().await.ok()? else {
                break;
            };
            let p = entry.path();
            if small_text(&p.join("scope")).await.as_deref().map(str::trim) == Some("Device") {
                continue;
            }
            match small_text(&p.join("type")).await.as_deref().map(str::trim) {
                Some(kind) if kind == "Mains" || kind.starts_with("USB") => {
                    if let Some(online) = number(&p.join("online")).await {
                        adapters_online.push(online > 0);
                    }
                }
                Some("Battery") if battery.is_none() => {
                    // An empty battery bay keeps its supply entry and reads as 0%.
                    if small_text(&p.join("present"))
                        .await
                        .as_deref()
                        .map(str::trim)
                        == Some("0")
                    {
                        continue;
                    }
                    if let Some(percent) = number(&p.join("capacity")).await.filter(|v| *v <= 100) {
                        let status = small_text(&p.join("status")).await;
                        battery = Some((percent as u8, status.map(|s| s.trim().to_owned())));
                    }
                }
                _ => {}
            }
        }
        let (percent, status) = battery?;
        Some(Battery {
            percent,
            charging: status.as_deref() == Some("Charging"),
            external_power: external_power(&adapters_online, status.as_deref()),
        })
    }
    pub async fn apply(&self, action: &Action) -> Result<(), String> {
        use crate::audio_devices::Kind;
        let pulse = |verb: &str, kind: Kind, value: String| {
            let verb = format!("set-{}-{verb}", kind.pulse_noun());
            async move {
                self.run("pactl", &[&verb, kind.pulse_default(), &value])
                    .await
                    .map(drop)
            }
        };
        match action {
            Action::Volume(value) if *value <= 100 && self.pulse_only() => {
                pulse("volume", Kind::Output, format!("{value}%")).await?;
            }
            Action::Mute(value) if self.pulse_only() => {
                pulse("mute", Kind::Output, u8::from(*value).to_string()).await?;
            }
            Action::MicrophoneMute(value) if self.pulse_only() => {
                pulse("mute", Kind::Input, u8::from(*value).to_string()).await?;
            }
            Action::AudioDevice(device) if self.pulse_only() => {
                self.pulse_set_default(device).await?;
            }
            Action::Volume(value) if *value <= 100 => {
                self.run(
                    "wpctl",
                    &[
                        "set-volume",
                        "--limit",
                        "1.0",
                        "@DEFAULT_AUDIO_SINK@",
                        &format!("{value}%"),
                    ],
                )
                .await?;
            }
            Action::Mute(value) => {
                self.run(
                    "wpctl",
                    &[
                        "set-mute",
                        "@DEFAULT_AUDIO_SINK@",
                        if *value { "1" } else { "0" },
                    ],
                )
                .await?;
            }
            Action::MicrophoneMute(value) => {
                self.run(
                    "wpctl",
                    &[
                        "set-mute",
                        "@DEFAULT_AUDIO_SOURCE@",
                        if *value { "1" } else { "0" },
                    ],
                )
                .await?;
            }
            Action::AudioDevice(device) => {
                let id = device.id.to_string();
                let current = self.run("wpctl", &["inspect", &id]).await?;
                if !crate::audio_devices::matches(device, &current) {
                    return Err(tr("shell-audio-device-changed-refresh-the-device-list").into());
                }
                self.run("wpctl", &["set-default", &id]).await?;
                let selected = self
                    .run("wpctl", &["inspect", device.kind.default_target()])
                    .await?;
                if !crate::audio_devices::matches(device, &selected) {
                    return Err(tr("shell-audio-device-did-not-become-the-default").into());
                }
            }
            Action::Brightness(value) if (5..=100).contains(value) => {
                let device = self
                    .brightness()
                    .await
                    .ok_or_else(|| tr("shell-no-controllable-backlight-is-available"))?
                    .device;
                if self.tool("brightnessctl").is_some() {
                    self.run(
                        "brightnessctl",
                        &["--device", &device, "set", &format!("{value}%")],
                    )
                    .await?;
                } else if self.allow_power && self.sysfs == Path::new("/sys") {
                    let maximum = number(
                        &self
                            .sysfs
                            .join("class/backlight")
                            .join(&device)
                            .join("max_brightness"),
                    )
                    .await
                    .ok_or_else(|| tr("shell-no-controllable-backlight-is-available"))?;
                    let maximum = u32::try_from(maximum)
                        .map_err(|_| tr("shell-no-controllable-backlight-is-available"))?;
                    let raw = ((u64::from(maximum) * u64::from(*value) + 50) / 100).max(1);
                    crate::session_activity::set_brightness(
                        &device,
                        u32::try_from(raw)
                            .map_err(|_| tr("shell-no-controllable-backlight-is-available"))?,
                    )
                    .await?;
                } else {
                    return Err(tr("shell-no-controllable-backlight-is-available").into());
                }
            }
            Action::Wifi(value) => {
                let snapshot = self.snapshot().await;
                if !snapshot.network.wifi_present || snapshot.network.wifi_enabled.is_none() {
                    return Err(tr("shell-no-wi-fi-radio-is-available").into());
                }
                self.run(
                    "nmcli",
                    &["radio", "wifi", if *value { "on" } else { "off" }],
                )
                .await?;
            }
            Action::Bluetooth(value) => {
                self.run(
                    "bluetoothctl",
                    &["power", if *value { "on" } else { "off" }],
                )
                .await?;
                if parse_bluetooth(&self.run("bluetoothctl", &["show"]).await?) != Some(*value) {
                    return Err(tr("shell-bluetooth-did-not-reach-the-requested-state").into());
                }
            }
            Action::Lock if crate::session_lock::command().is_some() => {
                if let Err(error) = crate::session_lock::lock().await {
                    if self.tool("swaylock").is_none() {
                        return Err(error);
                    }
                    self.run("swaylock", &["--daemonize", "--color", "202124"])
                        .await?;
                }
            }
            Action::Lock => {
                self.run("swaylock", &["--daemonize", "--color", "202124"])
                    .await?;
            }
            Action::PowerMode(_) | Action::Mount(_) | Action::Eject(_)
                if !(self.allow_power
                    || crate::desktop_session::mode()
                        == crate::desktop_session::Mode::Installed) =>
            {
                return Err(tr("shell-this-preview-does-not-change-your-computer").into());
            }
            Action::PowerMode(profile) => crate::power_mode::set(*profile).await?,
            Action::GameMode(enabled)
                if self.allow_power
                    || crate::desktop_session::mode()
                        == crate::desktop_session::Mode::Installed =>
            {
                crate::game_mode::set(*enabled).await?;
            }
            Action::GameMode(_) => {
                return Err(tr("shell-game-mode-changes-are-off-in-this-preview").into());
            }
            Action::Mount(drive) => {
                crate::drives::mount_drive(drive).await?;
            }
            Action::Eject(drive) => crate::drives::eject_mounted(drive).await?,
            Action::Suspend if self.allow_power => {
                if crate::session_lock::command().is_some() {
                    crate::session_lock::lock().await?;
                }
                self.run("systemctl", &["suspend"]).await?;
            }
            Action::Suspend => {
                return Err(
                    tr("shell-system-power-actions-are-disabled-in-this-nested-preview").into(),
                );
            }
            Action::PowerOff | Action::Restart if self.allow_power => {
                self.run(
                    "systemctl",
                    &[if *action == Action::PowerOff {
                        "poweroff"
                    } else {
                        "reboot"
                    }],
                )
                .await?;
            }
            Action::PowerOffAnyway | Action::RestartAnyway if self.allow_power => {
                let verb = if *action == Action::PowerOffAnyway {
                    "poweroff"
                } else {
                    "reboot"
                };
                self.run("systemctl", &[verb, "--check-inhibitors=no"])
                    .await?;
            }
            Action::PowerOff | Action::Restart | Action::PowerOffAnyway | Action::RestartAnyway => {
                return Err(
                    tr("shell-system-power-actions-are-disabled-in-this-nested-preview").into(),
                );
            }
            Action::LogOut if self.allow_power => {
                let compositor = self
                    .login_compositor
                    .as_ref()
                    .ok_or_else(|| tr("shell-this-desktop-is-not-running-in-a-login-session"))?;
                // Let the session command return to its display manager. Killing
                // the login scope also kills SDDM's PAM helper before it can close.
                compositor.request(&["dispatch", "hl.dsp.exit()"]).await?;
            }
            Action::LogOut => {
                return Err(
                    tr("shell-system-power-actions-are-disabled-in-this-nested-preview").into(),
                );
            }
            _ => {
                return Err(tr("shell-control-value-is-outside-its-allowed-range").into());
            }
        }
        Ok(())
    }
    pub async fn screenshot(&self, output: &str, directory: &Path) -> Result<PathBuf, String> {
        if output.is_empty()
            || output.len() > 180
            || output.starts_with("ENZREMOTE-")
            || output.chars().any(char::is_control)
        {
            return Err(tr("shell-invalid-desktop-capture-output").into());
        }
        let tool = self
            .tool("grim")
            .ok_or_else(|| tr("shell-grim-is-not-installed"))?;
        if !directory.is_absolute() {
            return Err(tr("shell-screenshot-directory-must-be-absolute").into());
        }
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|_| tr("shell-cannot-create-screenshot-directory"))?;
        let path = directory.join(format!(
            "Screenshot-{}-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%f"),
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| tr("shell-cannot-create-screenshot-file"))?;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(tool)
                .args(["-o", output, "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::from(file))
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
        if !matches!(result, Ok(Ok(status)) if status.success()) {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(
                tr("shell-screenshot-failed-or-timed-out-no-other-output-was-captured").into(),
            );
        }
        Ok(path)
    }
}

async fn small_text(path: &Path) -> Option<String> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text).await.ok()?;
    (text.len() <= 4096).then_some(text)
}
async fn number(path: &Path) -> Option<u64> {
    small_text(path).await?.trim().parse().ok()
}
// An online adapter means external power. Without a readable adapter, any
// battery state but discharging does, such as Full or a charge limit's Not charging.
fn external_power(adapters_online: &[bool], status: Option<&str>) -> bool {
    adapters_online.contains(&true)
        || match status {
            Some("Charging") => true,
            Some("Discharging") => false,
            _ => adapters_online.is_empty(),
        }
}
fn device_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn parse_audio(text: &str) -> Option<Audio> {
    let value: f32 = text
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    if !value.is_finite() || !(0.0..=2.0).contains(&value) {
        return None;
    }
    Some(Audio {
        percent: (value * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: text.contains("[MUTED]"),
    })
}
impl From<crate::network_connectivity::Devices> for Network {
    fn from(devices: crate::network_connectivity::Devices) -> Self {
        Self {
            ethernet: devices.ethernet,
            ethernet_present: devices.ethernet_present,
            wifi_present: devices.wifi_present,
            wifi_connected: devices.wifi_connected,
            wifi_enabled: devices.wifi_enabled,
            connectivity: crate::network_connectivity::State::default(),
        }
    }
}

fn parse_network(text: &str) -> Network {
    let mut result = Network::default();
    for line in text.lines() {
        if let Some((kind, state)) = line.split_once(':') {
            let connected = state == "connected" || state == "connected (externally)";
            if kind == "ethernet" {
                result.ethernet_present = true;
                result.ethernet |= connected;
            }
            if kind == "wifi" {
                result.wifi_present = true;
                result.wifi_connected |= connected;
            }
        }
    }
    result
}
/// Whether BlueZ's first adapter is powered, read over the system bus;
/// `None` when BlueZ does not answer there.
async fn bluetooth_powered() -> Option<bool> {
    let system = crate::bus::system().await?;
    let objects = zbus::fdo::ObjectManagerProxy::builder(&system)
        .destination("org.bluez")
        .ok()?
        .path("/")
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let managed = tokio::time::timeout(Duration::from_secs(1), objects.get_managed_objects())
        .await
        .ok()?
        .ok()?;
    managed.values().find_map(|interfaces| {
        interfaces
            .iter()
            .find(|(name, _)| name.as_str() == "org.bluez.Adapter1")?
            .1
            .get("Powered")
            .and_then(|value| bool::try_from(value).ok())
    })
}

fn parse_bluetooth(text: &str) -> Option<bool> {
    text.lines().find_map(|line| match line.trim() {
        "Powered: yes" => Some(true),
        "Powered: no" => Some(false),
        _ => None,
    })
}

pub fn screenshot_directory() -> Option<PathBuf> {
    std::env::var_os("ENZUI_SCREENSHOT_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Pictures/Screenshots")))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn backend(root: &Path) -> SystemControls {
        SystemControls::fixture(root)
    }
    fn script(root: &Path, name: &str, body: &str) {
        let path = root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn attribute(root: &Path, name: &str, value: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value).unwrap();
    }
    #[tokio::test]
    async fn system_controls_async_behaviour() {
        // Device and opt in power actions use only the fixture tools.
        {
            let temp = tempfile::tempdir().unwrap();
            let mut b = backend(temp.path());
            let log = temp.path().join("actions");
            for name in ["brightnessctl", "swaylock", "systemctl", "enz-hyprctl"] {
                script(
                    temp.path(),
                    name,
                    &format!("printf '%s\\n' '{name}' \"$*\" >> '{}'", log.display()),
                );
            }
            attribute(
                temp.path(),
                "class/backlight/intel_backlight/brightness",
                "30",
            );
            attribute(
                temp.path(),
                "class/backlight/intel_backlight/max_brightness",
                "100",
            );
            script(
                temp.path(),
                "nmcli",
                &format!(
                    "case \"$*\" in '-t -f TYPE,STATE device status') printf 'wifi:connected\\n';; '-t -f WIFI radio') printf 'enabled\\n';; 'radio wifi off') printf '%s\\n' nmcli \"$*\" >> '{}';; *) exit 1;; esac",
                    log.display()
                ),
            );
            b.apply(&Action::Brightness(42)).await.unwrap();
            b.apply(&Action::Wifi(false)).await.unwrap();
            b.apply(&Action::Lock).await.unwrap();
            assert!(b.apply(&Action::PowerOff).await.is_err());
            assert!(b.apply(&Action::LogOut).await.is_err());
            b.allow_power = true;
            assert!(b.apply(&Action::LogOut).await.is_err());
            b.login_compositor = Some(
                crate::compositor::Compositor::new(
                    temp.path().join("enz-hyprctl"),
                    "logout-fixture".into(),
                )
                .unwrap(),
            );
            b.apply(&Action::LogOut).await.unwrap();
            b.apply(&Action::PowerOff).await.unwrap();
            b.apply(&Action::Restart).await.unwrap();
            assert_eq!(
                std::fs::read_to_string(&log).unwrap(),
                "brightnessctl\n--device intel_backlight set 42%\nnmcli\nradio wifi off\nswaylock\n--daemonize --color 202124\nenz-hyprctl\n-i logout-fixture dispatch hl.dsp.exit()\nsystemctl\npoweroff\nsystemctl\nreboot\n"
            );
            std::fs::remove_file(temp.path().join("brightnessctl")).unwrap();
            assert_eq!(b.brightness().await.unwrap().percent, 30);
            // Reading a real backlight does not depend on a helper; fixtures cannot write host logind.
            assert!(b.apply(&Action::Brightness(80)).await.is_err());
            script(temp.path(), "nmcli", "printf 'ethernet:connected\\n'");
            assert!(b.apply(&Action::Wifi(false)).await.is_err());
        }
        // With PulseAudio and no PipeWire tools, the volume tile, keys and
        // device picker work through `pactl`.
        //
        // Sabotage: drop the `pulse_only` branch from `snapshot_with`, from
        // `apply`, or from `audio_devices`.
        {
            use crate::audio_devices::Kind;
            let temp = tempfile::tempdir().unwrap();
            let b = backend(temp.path());
            let (log, default) = (temp.path().join("log"), temp.path().join("default"));
            std::fs::write(&default, "alsa_output.pci\n").unwrap();
            script(
                temp.path(),
                "pactl",
                &format!(
                    r#"case "$*" in
'get-sink-volume @DEFAULT_SINK@') echo 'Volume: front-left: 42598 /  65% / -11.23 dB';;
'get-sink-mute @DEFAULT_SINK@') echo 'Mute: no';;
'get-source-volume @DEFAULT_SOURCE@') echo 'Volume: mono: 32768 /  50% / -18.06 dB';;
'get-source-mute @DEFAULT_SOURCE@') echo 'Mute: yes';;
'--format=json list sinks') echo '[{{"index":4,"name":"alsa_output.pci","description":"Speakers","properties":{{}}}},{{"index":9,"name":"bluez_output.phones","description":"Headphones","properties":{{}}}}]';;
'--format=json list sources') echo '[{{"index":4,"name":"alsa_output.pci.monitor","monitor_of_sink":"alsa_output.pci","properties":{{"device.class":"monitor"}}}}]';;
get-default-sink) cat '{default}';;
get-default-source) exit 1;;
set-default-sink*) echo "$2" > '{default}'; echo "$*" >> '{log}';;
*) echo "$*" >> '{log}';;
esac"#,
                    default = default.display(),
                    log = log.display()
                ),
            );
            let snapshot = b.snapshot_with(false, true).await;
            assert_eq!(
                snapshot.audio,
                Some(Audio {
                    percent: 65,
                    muted: false
                })
            );
            assert_eq!(
                snapshot.microphone,
                Some(Audio {
                    percent: 50,
                    muted: true
                })
            );
            b.apply(&Action::Volume(30)).await.unwrap();
            b.apply(&Action::Mute(true)).await.unwrap();
            b.apply(&Action::MicrophoneMute(false)).await.unwrap();
            let devices = b.audio_devices().await.unwrap();
            let outputs: Vec<_> = devices
                .nodes
                .iter()
                .filter(|d| d.kind == Kind::Output)
                .collect();
            assert_eq!(outputs.len(), 2);
            assert!(devices.nodes.iter().all(|d| d.kind == Kind::Output));
            assert!(devices.selected(outputs[0]) && !devices.selected(outputs[1]));
            b.apply(&Action::AudioDevice(outputs[1].clone()))
                .await
                .unwrap();
            assert!(b.audio_devices().await.unwrap().selected(outputs[1]));
            let mut gone = outputs[1].clone();
            gone.name = "bluez_output.other".into();
            assert!(b.apply(&Action::AudioDevice(gone)).await.is_err());
            assert_eq!(
                std::fs::read_to_string(&log).unwrap(),
                "set-sink-volume @DEFAULT_SINK@ 30%\nset-sink-mute @DEFAULT_SINK@ 1\nset-source-mute @DEFAULT_SOURCE@ 0\nset-default-sink bluez_output.phones\n"
            );
        }
        // Command timeouts and output bounds do not block the shell.
        {
            let temp = tempfile::tempdir().unwrap();
            let b = backend(temp.path());
            script(temp.path(), "wpctl", "exec sleep 30");
            let start = std::time::Instant::now();
            assert!(
                b.run("wpctl", &[])
                    .await
                    .unwrap_err()
                    .contains("did not respond")
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            script(temp.path(), "wpctl", "head -c 70000 /dev/zero");
            assert!(b.run("wpctl", &[]).await.unwrap_err().contains("bound"));
        }
        // The panel shows the installed battery's charge, whatever its name,
        // past an empty bay and a peripheral's battery.
        {
            let temp = tempfile::tempdir().unwrap();
            let b = backend(temp.path());
            for (name, file, value) in [
                ("BAT0", "type", "Battery"),
                ("BAT0", "present", "0"),
                ("BAT0", "capacity", "0"),
                ("hid-pen-battery", "type", "Battery"),
                ("hid-pen-battery", "scope", "Device"),
                ("hid-pen-battery", "capacity", "5"),
                ("BAT1", "type", "Battery"),
                ("BAT1", "present", "1"),
                ("BAT1", "capacity", "100"),
                ("BAT1", "status", "Full"),
            ] {
                attribute(
                    temp.path(),
                    &format!("class/power_supply/{name}/{file}"),
                    value,
                );
            }
            let battery = b.battery().await.unwrap();
            assert_eq!(
                battery,
                Battery {
                    percent: 100,
                    charging: false,
                    external_power: true,
                }
            );
            assert_eq!(battery.icon(), "battery-full");
            // The adapter, not the charge state, decides the suspend timer: a
            // charge limit holds the battery at Not charging on AC.
            let b = &b;
            let power = |online: &str, status: &str| {
                attribute(temp.path(), "class/power_supply/ADP1/type", "Mains");
                attribute(temp.path(), "class/power_supply/ADP1/online", online);
                attribute(temp.path(), "class/power_supply/BAT1/status", status);
                async move { b.battery().await.unwrap().external_power }
            };
            assert!(power("1", "Not charging").await);
            assert!(!power("0", "Discharging").await);
            assert!(!power("0", "Unknown").await);
        }
    }
}
