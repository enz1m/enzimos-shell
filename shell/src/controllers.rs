//! Seat-authorized controller discovery and event-driven input testing.
//! Only an explicit controller test reads input; device permissions remain OS-owned.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[path = "controller_battery.rs"]
mod bluez;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
    Virtual,
    Other,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Bluetooth => tr("shell-bluetooth"),
            Self::Virtual => tr("shell-virtual"),
            Self::Other => tr("shell-wired"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Battery {
    Percent(u8),
    Full,
    High,
    Normal,
    Low,
    Critical,
}

impl Battery {
    pub fn label(self) -> String {
        match self {
            Self::Percent(percent) => {
                tr!("shell-battery-2", "arg0" => format!("{:}", percent))
            }
            Self::Full => tr("shell-battery-full").into(),
            Self::High => tr("shell-battery-high").into(),
            Self::Normal => tr("shell-battery-normal").into(),
            Self::Low => tr("shell-low-battery").into(),
            Self::Critical => tr("shell-battery-critical").into(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Controller {
    /// The physical device (its sysfs parent), shared by all its nodes.
    pub id: String,
    pub name: String,
    pub transport: Transport,
    /// Reported battery percentage or categorical level.
    pub battery: Option<Battery>,
    /// The controller supports rumble (force feedback).
    pub rumble: bool,
    /// The event node a test reads.
    pub node: PathBuf,
    /// This account may read it (the seat's active session).
    pub accessible: bool,
    bluetooth_address: Option<String>,
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Controller")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("transport", &self.transport)
            .field("battery", &self.battery)
            .field("rumble", &self.rumble)
            .field("node", &self.node)
            .field("accessible", &self.accessible)
            .finish()
    }
}

// Whether bit `bit` is set in a sysfs capability bitmap (hex words,
// most significant first).
fn has_bit(bitmap: &str, bit: usize) -> bool {
    let words: Vec<&str> = bitmap.split_whitespace().collect();
    let index = bit / 64;
    let Some(word) = words
        .len()
        .checked_sub(index + 1)
        .and_then(|i| words.get(i))
    else {
        return false;
    };
    u64::from_str_radix(word, 16).is_ok_and(|value| value & (1 << (bit % 64)) != 0)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

// A gamepad or joystick: udev says joystick, and it has gamepad/joystick
// buttons and an axis (keyboards' media keys can carry the joystick tag).
fn is_controller(input: &Path, udev: &str) -> bool {
    if !udev.lines().any(|line| line == "E:ID_INPUT_JOYSTICK=1") {
        return false;
    }
    let keys = read(&input.join("capabilities/key"));
    let axes = read(&input.join("capabilities/abs"));
    let buttons = has_bit(&keys, 0x130) || (0x120..0x130).any(|bit| has_bit(&keys, bit));
    buttons && axes.split_whitespace().any(|word| word != "0")
}

fn battery(parent: &Path) -> Option<Battery> {
    // A controller's own battery is a power supply below its HID device.
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let supply = entry.path();
        if read(&supply.join("scope")) != "Device" {
            continue;
        }
        let Ok(device) = std::fs::canonicalize(supply.join("device")) else {
            continue;
        };
        if device.starts_with(parent) || parent.starts_with(&device) {
            return battery_report(
                &read(&supply.join("capacity")),
                &read(&supply.join("capacity_level")),
            );
        }
    }
    None
}

fn battery_report(capacity: &str, level: &str) -> Option<Battery> {
    if let Ok(capacity) = capacity.parse::<u8>()
        && capacity <= 100
    {
        return Some(Battery::Percent(capacity));
    }
    match level {
        "Full" => Some(Battery::Full),
        "High" => Some(Battery::High),
        "Normal" => Some(Battery::Normal),
        "Low" => Some(Battery::Low),
        "Critical" => Some(Battery::Critical),
        _ => None,
    }
}

fn controller_parent(input: &Path) -> PathBuf {
    input
        .parent()
        .filter(|parent| *parent != Path::new("/sys/devices/virtual/input"))
        .unwrap_or(input)
        .to_owned()
}

/// The controllers present now, one per physical device.
#[must_use]
pub fn list() -> Vec<Controller> {
    let mut found: Vec<Controller> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return found;
    };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    nodes.sort();
    for event in nodes {
        let number = read(&event.join("dev"));
        let Some((major, minor)) = number.split_once(':') else {
            continue;
        };
        let udev =
            std::fs::read_to_string(format!("/run/udev/data/c{major}:{minor}")).unwrap_or_default();
        let Ok(input) = std::fs::canonicalize(event.join("device")) else {
            continue;
        };
        if !is_controller(&input, &udev) {
            continue;
        }
        // The HID (or USB interface) device groups a controller's nodes.
        let parent = controller_parent(&input);
        let id = parent.to_string_lossy().into_owned();
        if found.iter().any(|known| known.id == id) {
            continue;
        }
        let transport = match read(&input.join("id/bustype")).as_str() {
            "0003" => Transport::Usb,
            "0005" => Transport::Bluetooth,
            "0006" => Transport::Virtual,
            _ => Transport::Other,
        };
        let bluetooth_address = (transport == Transport::Bluetooth)
            .then(|| bluez::address(&read(&input.join("uniq"))))
            .flatten();
        if bluetooth_address.as_ref().is_some_and(|address| {
            found.iter().any(|known| {
                known.transport == Transport::Bluetooth
                    && known.bluetooth_address.as_ref() == Some(address)
            })
        }) {
            continue;
        }
        let name: String = read(&input.join("name"))
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        let node = PathBuf::from("/dev/input").join(event.file_name().unwrap_or_default());
        let accessible = std::fs::File::open(&node).is_ok();
        found.push(Controller {
            id,
            name: if name.is_empty() {
                tr("shell-game-controller").into()
            } else {
                name
            },
            transport,
            bluetooth_address,
            battery: battery(&parent),
            rumble: has_bit(&read(&input.join("capabilities/ff")), 0x50),
            node,
            accessible,
        });
    }
    found.truncate(16);
    found
}

fn device_changes() -> impl iced::futures::Stream<Item = Vec<Controller>> {
    use iced::futures::SinkExt;
    use tokio::io::AsyncBufReadExt;
    iced::stream::channel(
        4,
        async move |mut output: iced::futures::channel::mpsc::Sender<Vec<Controller>>| {
            let mut last = tokio::task::spawn_blocking(list).await.unwrap_or_default();
            let _ = output.send(last.clone()).await;
            loop {
                let child = tokio::process::Command::new("udevadm")
                    .args([
                        "monitor",
                        "--udev",
                        "--subsystem-match=input",
                        "--subsystem-match=power_supply",
                    ])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn();
                let Ok(mut child) = child else {
                    // Without udevadm the list stays as read at start.
                    std::future::pending::<()>().await;
                    return;
                };
                let Some(stdout) = child.stdout.take() else {
                    return;
                };
                let mut lines = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.starts_with("UDEV") {
                        continue;
                    }
                    // One plug-in sends several events; settle, then read once.
                    let settle = tokio::time::sleep(Duration::from_millis(500));
                    tokio::pin!(settle);
                    loop {
                        tokio::select! {
                            () = &mut settle => break,
                            next = lines.next_line() => if !matches!(next, Ok(Some(_))) { break },
                        }
                    }
                    let now = tokio::task::spawn_blocking(list).await.unwrap_or_default();
                    if now != last {
                        last = now.clone();
                        let _ = output.send(now).await;
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        },
    )
}

fn with_batteries(controllers: &[Controller], batteries: &bluez::Snapshot) -> Vec<Controller> {
    controllers
        .iter()
        .cloned()
        .map(|mut controller| {
            if controller.transport == Transport::Bluetooth
                && !matches!(controller.battery, Some(Battery::Percent(_)))
                && let Some(address) = &controller.bluetooth_address
                && let Some(percent) = batteries.get(address)
            {
                controller.battery = Some(Battery::Percent(*percent));
            }
            controller
        })
        .collect()
}

/// Controller and battery updates, with no healthy idle polling.
pub fn subscription() -> iced::Subscription<Vec<Controller>> {
    use iced::futures::{SinkExt, StreamExt};
    iced::Subscription::run(|| {
        iced::stream::channel(
            4,
            async move |mut output: iced::futures::channel::mpsc::Sender<Vec<Controller>>| {
                let devices = device_changes();
                let battery = bluez::changes();
                tokio::pin!(devices, battery);
                let mut controllers = None;
                let mut batteries = bluez::Snapshot::new();
                let mut last = None;
                loop {
                    tokio::select! {
                        next = devices.next() => match next { Some(next) => controllers = Some(next), None => return },
                        next = battery.next() => match next { Some(next) => batteries = next, None => return },
                    }
                    let Some(controllers) = &controllers else {
                        continue;
                    };
                    let now = with_batteries(controllers, &batteries);
                    if last.as_ref() != Some(&now) {
                        last = Some(now.clone());
                        if output.send(now).await.is_err() {
                            return;
                        }
                    }
                }
            },
        )
    })
}

#[derive(Default)]
pub struct Notifications {
    present: std::collections::BTreeSet<String>,
    recent: std::collections::HashMap<String, std::time::Instant>,
    initialized: bool,
}

impl Notifications {
    pub fn connected(
        &mut self,
        controllers: &[Controller],
        enabled: bool,
        now: std::time::Instant,
    ) -> Vec<Controller> {
        let mut added = Vec::new();
        let mut present = std::collections::BTreeSet::new();
        for controller in controllers {
            let key = controller.bluetooth_address.as_ref().map_or_else(
                || controller.id.clone(),
                |address| format!("bluetooth:{address}"),
            );
            if !present.insert(key.clone()) || self.present.contains(&key) {
                continue;
            }
            let recent = self
                .recent
                .get(&key)
                .is_some_and(|at| now.saturating_duration_since(*at) < Duration::from_secs(60));
            if self.initialized && enabled && !recent {
                added.push(controller.clone());
            }
            self.recent.insert(key, now);
        }
        self.present = present;
        self.initialized = true;
        self.recent.retain(|id, at| {
            self.present.contains(id)
                || now.saturating_duration_since(*at) < Duration::from_secs(60)
        });
        added
    }
}

/// What a test sees: a button's state or an axis position (0–1000).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Button {
        code: u16,
        pressed: bool,
    },
    Axis {
        code: u16,
        value: i32,
        min: i32,
        max: i32,
    },
    Gone(String),
}

/// Reads only the selected node; dropping the stream closes its nonblocking reader.
pub fn test(node: PathBuf) -> impl iced::futures::Stream<Item = Input> {
    use iced::futures::SinkExt;
    iced::stream::channel(
        64,
        async move |mut output: iced::futures::channel::mpsc::Sender<Input>| {
            let opened = rustix::fs::open(
                &node,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NONBLOCK,
                rustix::fs::Mode::empty(),
            )
            .map_err(std::io::Error::from)
            .and_then(evdev::Device::from_fd);
            let Ok(device) = opened else {
                let _ = output.send(Input::Gone(tr("shell-this-controller-cannot-be-read-it-may-have-disconnected-or-belong-to-another").into())).await;
                return;
            };
            let ranges = match device.get_absinfo() {
                Ok(axes) => axes
                    .filter(|(_, info)| info.maximum() > info.minimum())
                    .map(|(axis, info)| (axis.0, (info.value(), info.minimum(), info.maximum())))
                    .collect::<std::collections::BTreeMap<_, _>>(),
                Err(_) => {
                    let _ = output
                        .send(Input::Gone(
                            tr("shell-this-controller-s-axis-limits-could-not-be-read").into(),
                        ))
                        .await;
                    return;
                }
            };
            let Ok(mut events) = device.into_event_stream() else {
                let _ = output
                    .send(Input::Gone(
                        tr("shell-this-controller-cannot-be-tested-safely").into(),
                    ))
                    .await;
                return;
            };
            for (&code, &(value, min, max)) in &ranges {
                if output
                    .send(Input::Axis {
                        code,
                        value,
                        min,
                        max,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            loop {
                let Ok(event) = events.next_event().await else {
                    let _ = output
                        .send(Input::Gone(
                            tr("shell-the-controller-was-disconnected-or-its-access-ended").into(),
                        ))
                        .await;
                    return;
                };
                let input = match event.destructure() {
                    evdev::EventSummary::Key(_, code, value) if code.0 >= 0x100 => Input::Button {
                        code: code.0,
                        pressed: value != 0,
                    },
                    evdev::EventSummary::AbsoluteAxis(_, code, value) => {
                        let Some(&(_, min, max)) = ranges.get(&code.0) else {
                            continue;
                        };
                        Input::Axis {
                            code: code.0,
                            value,
                            min,
                            max,
                        }
                    }
                    _ => continue,
                };
                if output.send(input).await.is_err() {
                    return;
                }
            }
        },
    )
}

/// A button's name for the test view.
#[must_use]
pub fn button_name(code: u16) -> String {
    match code {
        0x130 => "A".into(),
        0x131 => "B".into(),
        0x133 => "X".into(),
        0x134 => "Y".into(),
        0x136 => "LB".into(),
        0x137 => "RB".into(),
        0x138 => "LT".into(),
        0x139 => "RT".into(),
        0x13a => tr("shell-back").into(),
        0x13b => tr("shell-start").into(),
        0x13c => tr("shell-home").into(),
        0x13d => "L3".into(),
        0x13e => "R3".into(),
        0x220..=0x223 => [
            tr("shell-up"),
            tr("shell-down"),
            tr("shell-left"),
            tr("shell-right"),
        ][usize::from(code - 0x220)]
        .into(),
        other => tr!("shell-button", "arg0" => format!("{:}", other - 0x100)),
    }
}

/// An axis's name for the test view.
#[must_use]
pub fn axis_name(code: u16) -> &'static str {
    match code {
        0x00 => tr("shell-left-stick-x"),
        0x01 => tr("shell-left-stick-y"),
        0x02 => tr("shell-left-trigger"),
        0x03 => tr("shell-right-stick-x"),
        0x04 => tr("shell-right-stick-y"),
        0x05 => tr("shell-right-trigger"),
        0x10 => tr("shell-d-pad-x"),
        0x11 => tr("shell-d-pad-y"),
        _ => tr("shell-axis"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires the owned public UHID fixture in a disposable enzimOS guest"]
    async fn installed_controller_reader_uses_kernel_limits_and_closes_while_quiet() {
        use iced::futures::StreamExt;
        let node = PathBuf::from(std::env::var("ENZUI_CONTROLLER_FIXTURE_NODE").unwrap());
        let name = node.file_name().unwrap();
        assert_eq!(
            read(&Path::new("/sys/class/input").join(name).join("device/name")),
            "EnzUI battery protocol fixture"
        );
        let count = || {
            std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .flatten()
                .filter(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == node))
                .count()
        };
        let baseline = count();
        let mut stream = Box::pin(test(node.clone()));
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(
                    event,
                    Input::Axis {
                        min: -127,
                        max: 127,
                        ..
                    }
                ),
                "{event:?}"
            );
        }
        assert_eq!(count(), baseline + 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), stream.next())
                .await
                .is_err()
        );
        drop(stream);
        assert_eq!(
            count(),
            baseline,
            "quiet read must not retain a blocking file clone"
        );
    }
}
