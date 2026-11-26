use enzui_shell::i18n::tr;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Output,
    Input,
}
impl Kind {
    pub fn class(self) -> &'static str {
        match self {
            Self::Output => "Audio/Sink",
            Self::Input => "Audio/Source",
        }
    }
    pub fn default_target(self) -> &'static str {
        match self {
            Self::Output => "@DEFAULT_AUDIO_SINK@",
            Self::Input => "@DEFAULT_AUDIO_SOURCE@",
        }
    }
    /// PulseAudio's word for the device: `sink` or `source`.
    pub fn pulse_noun(self) -> &'static str {
        match self {
            Self::Output => "sink",
            Self::Input => "source",
        }
    }
    pub fn pulse_default(self) -> &'static str {
        match self {
            Self::Output => "@DEFAULT_SINK@",
            Self::Input => "@DEFAULT_SOURCE@",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: u32,
    pub serial: u64,
    pub name: String,
    pub label: String,
    pub kind: Kind,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub nodes: Vec<Device>,
    pub output: Option<u64>,
    pub input: Option<u64>,
}
impl Devices {
    pub fn selected(&self, device: &Device) -> bool {
        (match device.kind {
            Kind::Output => self.output,
            Kind::Input => self.input,
        }) == Some(device.serial)
    }
}
pub fn parse_dump(text: &str) -> Result<Vec<Device>, String> {
    let data: Value =
        serde_json::from_str(text).map_err(|_| tr("shell-invalid-audio-device-response"))?;
    let nodes = data
        .as_array()
        .ok_or_else(|| tr("shell-invalid-audio-device-list"))?;
    parse_nodes(nodes)
}

pub(crate) fn parse_nodes<'a>(
    nodes: impl IntoIterator<Item = &'a Value>,
) -> Result<Vec<Device>, String> {
    let mut devices = Vec::new();
    for node in nodes {
        if node["type"] != "PipeWire:Interface:Node" {
            continue;
        }
        let props = &node["info"]["props"];
        let kind = match props["media.class"].as_str() {
            Some("Audio/Sink") => Kind::Output,
            Some("Audio/Source") => Kind::Input,
            _ => continue,
        };
        let id = node["id"]
            .as_u64()
            .and_then(|id| u32::try_from(id).ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| tr("shell-invalid-audio-device-id"))?;
        let serial = props["object.serial"]
            .as_u64()
            .ok_or_else(|| tr("shell-missing-audio-device-identity"))?;
        let name = props["node.name"]
            .as_str()
            .filter(|name| valid_text(name, 512))
            .ok_or_else(|| tr("shell-invalid-audio-device-name"))?;
        let label = props["node.description"]
            .as_str()
            .or_else(|| props["node.nick"].as_str())
            .filter(|label| valid_text(label, 512))
            .unwrap_or(name);
        if devices.len() >= 128
            || devices
                .iter()
                .any(|d: &Device| d.id == id || d.serial == serial)
        {
            return Err(tr("shell-ambiguous-or-oversized-audio-device-list").into());
        }
        devices.push(Device {
            id,
            serial,
            name: name.into(),
            label: label.into(),
            kind,
        });
    }
    Ok(devices)
}
/// The devices from `pactl --format=json list sinks` and `... list sources`;
/// a source that monitors a sink is not a microphone. PulseAudio's index is
/// both the id and the serial: it is not reused while the server runs, and
/// sinks and sources count separately.
pub fn parse_pulse(sinks: &str, sources: &str) -> Result<Vec<Device>, String> {
    let mut devices = Vec::new();
    for (text, kind) in [(sinks, Kind::Output), (sources, Kind::Input)] {
        let data: Value =
            serde_json::from_str(text).map_err(|_| tr("shell-invalid-audio-device-response"))?;
        let entries = data
            .as_array()
            .ok_or_else(|| tr("shell-invalid-audio-device-list"))?;
        for entry in entries {
            let props = &entry["properties"];
            if kind == Kind::Input
                && (props["device.class"] == "monitor"
                    || entry["monitor_of_sink"]
                        .as_str()
                        .is_some_and(|s| s != "n/a"))
            {
                continue;
            }
            let Some(index) = entry["index"].as_u64().and_then(|i| u32::try_from(i).ok()) else {
                return Err(tr("shell-invalid-audio-device-id").into());
            };
            let Some(name) = entry["name"].as_str().filter(|name| valid_text(name, 512)) else {
                return Err(tr("shell-invalid-audio-device-name").into());
            };
            let label = entry["description"]
                .as_str()
                .filter(|label| valid_text(label, 512))
                .unwrap_or(name);
            if devices.len() >= 128
                || devices
                    .iter()
                    .any(|d: &Device| d.kind == kind && (d.id == index || d.name == name))
            {
                return Err(tr("shell-ambiguous-or-oversized-audio-device-list").into());
            }
            devices.push(Device {
                id: index,
                serial: u64::from(index),
                name: name.into(),
                label: label.into(),
                kind,
            });
        }
    }
    Ok(devices)
}
/// A level from `pactl get-sink-volume` and `pactl get-sink-mute` (or the
/// source equivalents): the first channel's percentage, capped at 100, and
/// whether it is muted.
pub fn pulse_level(volume: &str, mute: &str) -> Option<(u8, bool)> {
    let percent: u16 = volume
        .split_whitespace()
        .find_map(|word| word.strip_suffix('%'))?
        .parse()
        .ok()?;
    let muted = match mute.trim().strip_prefix("Mute:")?.trim() {
        "yes" => true,
        "no" => false,
        _ => return None,
    };
    Some((percent.min(100) as u8, muted))
}
fn valid_text(text: &str, bound: usize) -> bool {
    !text.is_empty() && text.len() <= bound && !text.chars().any(char::is_control)
}
pub fn property<'a>(inspection: &'a str, key: &str) -> Option<&'a str> {
    inspection.lines().find_map(|line| {
        let (name, value) = line
            .trim()
            .trim_start_matches('*')
            .trim()
            .split_once(" = ")?;
        (name == key).then(|| value.trim().trim_matches('"'))
    })
}
pub fn serial(inspection: &str) -> Option<u64> {
    property(inspection, "object.serial")?.parse().ok()
}
pub fn matches(device: &Device, inspection: &str) -> bool {
    serial(inspection) == Some(device.serial)
        && property(inspection, "node.name") == Some(device.name.as_str())
        && property(inspection, "media.class") == Some(device.kind.class())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_devices_behaviour() {
        // Nodes are bounded and keep server identity.
        {
            let node = serde_json::json!({"id":42,"type":"PipeWire:Interface:Node","info":{"props":{"media.class":"Audio/Sink","object.serial":901,"node.name":"owned.sink","node.description":"Test speakers"}}});
            let nodes = parse_dump(&serde_json::json!([node]).to_string()).unwrap();
            assert_eq!(nodes[0].id, 42);
            assert_eq!(nodes[0].label, "Test speakers");
            assert!(parse_dump(&serde_json::json!([node, node]).to_string()).is_err());
            assert!(parse_dump("{}").is_err());
            let inspection = "id 42\n * media.class = \"Audio/Sink\"\n * node.name = \"owned.sink\"\n object.serial = \"901\"\n";
            assert!(matches(&nodes[0], inspection));
            assert!(!matches(&nodes[0], &inspection.replace("901", "902")));
            assert!(!matches(
                &nodes[0],
                &inspection.replace("Audio/Sink", "Audio/Source")
            ));
            assert!(!matches(
                &nodes[0],
                &inspection.replace("owned.sink", "other.sink")
            ));
        }
        // PulseAudio's sinks and sources become devices; monitor sources are
        // left out, and a sink and a source may share an index.
        //
        // Sabotage: drop the monitor check, or check indexes across kinds.
        {
            let sinks = r#"[{"index":51,"name":"alsa_output.usb","description":"USB speakers","properties":{}}]"#;
            let sources = r#"[{"index":51,"name":"alsa_output.usb.monitor","description":"Monitor of USB speakers","monitor_of_sink":"alsa_output.usb","properties":{"device.class":"monitor"}},{"index":52,"name":"alsa_input.usb","description":"USB microphone","monitor_of_sink":"n/a","properties":{"device.class":"sound"}},{"index":51,"name":"bluez_input.headset","description":"Headset","properties":{}}]"#;
            let devices = parse_pulse(sinks, sources).unwrap();
            let named: Vec<_> = devices
                .iter()
                .map(|d| (d.kind, d.id, d.label.as_str()))
                .collect();
            assert_eq!(
                named,
                [
                    (Kind::Output, 51, "USB speakers"),
                    (Kind::Input, 52, "USB microphone"),
                    (Kind::Input, 51, "Headset"),
                ]
            );
            let twice = r#"[{"index":3,"name":"a","properties":{}},{"index":3,"name":"b","properties":{}}]"#;
            assert!(parse_pulse(twice, "[]").is_err());
            assert_eq!(
                pulse_level("Volume: front-left: 42598 /  65% / -11.23 dB", "Mute: yes"),
                Some((65, true))
            );
        }
    }
}
