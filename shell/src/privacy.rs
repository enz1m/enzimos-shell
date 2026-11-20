//! PipeWire capture indicators for running microphone, camera and screen streams.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Apps capturing a microphone (or other audio source, not a monitor).
    pub microphone: BTreeSet<String>,
    pub camera: BTreeSet<String>,
    pub screen: BTreeSet<String>,
    /// The default output's level and mute, as `wpctl` reports them;
    /// `None` until PipeWire names one.
    pub output: Option<crate::system_controls::Audio>,
    /// The default input's.
    pub input: Option<crate::system_controls::Audio>,
}

impl Usage {
    #[must_use]
    pub fn any(&self) -> bool {
        !(self.microphone.is_empty() && self.camera.is_empty() && self.screen.is_empty())
    }
    /// "Microphone: Firefox. Screen: OBS" for assistive technology.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (what, apps) in [
            (tr("shell-microphone"), &self.microphone),
            (tr("shell-camera"), &self.camera),
            (tr("shell-screen"), &self.screen),
        ] {
            if !apps.is_empty() {
                parts.push(tr!("shell-in-use-by", "arg0" => what.to_owned(), "arg1" => apps.iter().cloned().collect::<Vec<_>>().join(", ").clone()));
            }
        }
        parts.join(". ")
    }
}

type Objects = BTreeMap<u64, serde_json::Value>;

/// Applies one `pw-dump` array: changed objects replace, `"info": null`
/// removes. Metadata objects carry their entries instead of `info`.
pub fn merge(objects: &mut Objects, update: &serde_json::Value) {
    for object in update.as_array().into_iter().flatten() {
        let Some(id) = object.get("id").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let info = object.get("info");
        if info.is_some_and(|info| !info.is_null())
            || (info.is_none() && object.get("metadata").is_some())
        {
            objects.insert(id, object.clone());
        } else {
            objects.remove(&id);
        }
    }
}

/// The level and mute of the default node `key` names
/// (`default.audio.sink`, `default.audio.source`): the cube root of its mean
/// channel volume, as `wpctl get-volume` shows it.
fn default_level(objects: &Objects, key: &str) -> Option<crate::system_controls::Audio> {
    use serde_json::Value;
    let name = objects
        .values()
        .filter(|o| {
            o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Metadata")
                && o.get("props")
                    .and_then(|p| p.get("metadata.name"))
                    .and_then(Value::as_str)
                    == Some("default")
        })
        .flat_map(|o| {
            o.get("metadata")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find(|entry| entry.get("key").and_then(Value::as_str) == Some(key))?
        .get("value")?
        .get("name")?
        .as_str()?;
    let node = objects.values().find(|o| {
        o.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node")
            && props(o).and_then(|p| text(p, "node.name")) == Some(name)
    })?;
    let params = node.get("info")?.get("params")?.get("Props")?.as_array()?;
    let levels = params.iter().find(|p| p.get("channelVolumes").is_some())?;
    let volumes: Vec<f64> = levels
        .get("channelVolumes")?
        .as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let mean = volumes.iter().sum::<f64>() / volumes.len().max(1) as f64;
    let level = mean.max(0.0).cbrt();
    (!volumes.is_empty() && level.is_finite()).then(|| crate::system_controls::Audio {
        percent: (level * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: levels.get("mute").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn props(object: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    object.get("info")?.get("props")?.as_object()
}

fn text<'a>(props: &'a serde_json::Map<String, serde_json::Value>, key: &str) -> Option<&'a str> {
    props
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

fn running(object: &serde_json::Value) -> bool {
    object
        .get("info")
        .and_then(|info| info.get("state"))
        .and_then(serde_json::Value::as_str)
        == Some("running")
}

fn app_name(props: &serde_json::Map<String, serde_json::Value>) -> String {
    let name = text(props, "application.name")
        .or_else(|| text(props, "application.process.binary"))
        .or_else(|| text(props, "node.name"))
        .unwrap_or_else(|| tr("shell-an-app"));
    name.chars().filter(|c| !c.is_control()).take(48).collect()
}

/// Which apps are capturing what, from the merged graph.
#[must_use]
pub fn usage(objects: &Objects) -> Usage {
    let mut found = Usage {
        output: default_level(objects, "default.audio.sink"),
        input: default_level(objects, "default.audio.source"),
        ..Usage::default()
    };
    let nodes: BTreeMap<u64, &serde_json::Value> = objects
        .iter()
        .filter(|(_, o)| {
            o.get("type").and_then(serde_json::Value::as_str) == Some("PipeWire:Interface:Node")
        })
        .map(|(id, o)| (*id, o))
        .collect();
    // Who reads from each source, through the graph's links.
    let mut readers: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for link in objects.values().filter(|o| {
        o.get("type").and_then(serde_json::Value::as_str) == Some("PipeWire:Interface:Link")
    }) {
        let info = link.get("info");
        let field = |key: &str| {
            info.and_then(|i| i.get(key))
                .and_then(serde_json::Value::as_u64)
        };
        if let (Some(output), Some(input)) = (field("output-node-id"), field("input-node-id")) {
            readers.entry(output).or_default().push(input);
        }
    }
    let reader_names = |source: u64| -> Vec<String> {
        readers
            .get(&source)
            .into_iter()
            .flatten()
            .filter_map(|id| nodes.get(id))
            .filter_map(|node| props(node))
            .map(app_name)
            .collect()
    };
    for (id, node) in &nodes {
        let Some(props) = props(node) else { continue };
        if !running(node) {
            continue;
        }
        match text(props, "media.class") {
            // A recorder of what the speakers play is not the microphone.
            Some("Stream/Input/Audio")
                if text(props, "stream.capture.sink") != Some("true")
                    && props.get("stream.capture.sink") != Some(&serde_json::Value::Bool(true)) =>
            {
                found.microphone.insert(app_name(props));
            }
            Some("Video/Source") => {
                let screen = text(props, "node.name")
                    .is_some_and(|n| n.starts_with("xdph") || n.starts_with("xdpw"))
                    || text(props, "media.role") == Some("Screen");
                let camera = text(props, "device.api")
                    .is_some_and(|api| matches!(api, "v4l2" | "libcamera"));
                let mut apps = reader_names(*id);
                if apps.is_empty() {
                    apps.push(if screen {
                        app_name(props)
                    } else {
                        tr("shell-an-app").into()
                    });
                }
                if screen {
                    found.screen.extend(apps);
                } else if camera {
                    found.camera.extend(apps);
                }
            }
            _ => {}
        }
    }
    found
}

/// Follows PipeWire for the lifetime of the session.
pub fn subscription() -> iced::Subscription<Usage> {
    iced::Subscription::run(stream)
}

fn stream() -> impl iced::futures::Stream<Item = Usage> {
    use iced::futures::SinkExt;
    iced::stream::channel(4, async |mut output| {
        loop {
            let mut command = std::process::Command::new("pw-dump");
            command.args(["--monitor", "--no-colors"]);
            // Dropping the subscription drops `observer`, which stops pw-dump
            // even while the graph is idle.
            if let Some((observer, mut latest)) = observe(command) {
                let mut last = Usage::default();
                while latest.changed().await.is_ok() {
                    let usage = latest.borrow_and_update().clone();
                    last = usage.clone();
                    if output.send(usage).await.is_err() {
                        return;
                    }
                }
                drop(observer);
                // PipeWire went away: nothing is capturing through it.
                if last.any() && output.send(Usage::default()).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    })
}

// A running `pw-dump --monitor` and the thread reading it; dropping it
// kills and reaps the child, which ends the reader.
struct Observer {
    child: std::process::Child,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

// Starts `command` and keeps only its latest distinct usage: a slow
// consumer sees the newest state, never a queue.
fn observe(
    mut command: std::process::Command,
) -> Option<(Observer, tokio::sync::watch::Receiver<Usage>)> {
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let (sender, receiver) = tokio::sync::watch::channel(Usage::default());
    let reader = std::thread::spawn(move || {
        let mut objects = Objects::new();
        for update in serde_json::Deserializer::from_reader(std::io::BufReader::new(stdout))
            .into_iter::<serde_json::Value>()
        {
            let Ok(update) = update else { break };
            merge(&mut objects, &update);
            publish(&sender, usage(&objects));
            if sender.is_closed() {
                break;
            }
        }
    });
    Some((
        Observer {
            child,
            reader: Some(reader),
        },
        receiver,
    ))
}

// Stores `usage` if it differs; whether it did.
fn publish(sender: &tokio::sync::watch::Sender<Usage>, usage: Usage) -> bool {
    sender.send_if_modified(|current| {
        let changed = *current != usage;
        if changed {
            *current = usage;
        }
        changed
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: u64, state: &str, props: serde_json::Value) -> serde_json::Value {
        json!({"id": id, "type": "PipeWire:Interface:Node", "info": {"state": state, "props": props}})
    }

    #[test]
    fn privacy_behaviour() {
        // Dropping an idle observer stops its child.
        {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "printf '[]\\n'; exec sleep 30"]);
            let (observer, _latest) = observe(command).unwrap();
            let pid = observer.child.id();
            std::thread::sleep(std::time::Duration::from_millis(200));
            let started = std::time::Instant::now();
            drop(observer);
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "the child was reaped"
            );
        }
        // The default output's and input's levels come from the graph as
        // `wpctl` shows them: the cube root of the mean channel volume, with
        // mute. Metadata objects, which carry no `info`, are kept until removed.
        //
        // Sabotage: drop objects without `info` in `merge` -- no default is
        // named; take the mean without the cube root -- 70 % reads as 34 %.
        {
            let mut objects = Objects::new();
            let sink = |id: u64, name: &str, volumes: serde_json::Value, mute: bool| {
                json!({"id": id, "type": "PipeWire:Interface:Node", "info": {"state": "idle",
                    "props": {"node.name": name, "media.class": "Audio/Sink"},
                    "params": {"Props": [{"volume": 1.0}, {"channelVolumes": volumes, "mute": mute}]}}})
            };
            let defaults = |sink: &str| {
                json!({"id": 9, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
                "metadata": [
                    {"subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON", "value": {"name": sink}},
                    {"subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": {"name": "mic"}}
                ]})
            };
            merge(
                &mut objects,
                &json!([
                    defaults("speakers"),
                    sink(40, "speakers", json!([0.344006, 0.344006]), false),
                    sink(41, "headset", json!([0.125]), false),
                    sink(42, "mic", json!([0.156326]), true),
                ]),
            );
            let found = usage(&objects);
            assert_eq!(
                found.output.map(|a| (a.percent, a.muted)),
                Some((70, false))
            );
            assert_eq!(found.input.map(|a| (a.percent, a.muted)), Some((54, true)));
            merge(&mut objects, &json!([defaults("headset")]));
            assert_eq!(
                usage(&objects).output.map(|a| a.percent),
                Some(50),
                "a new default output"
            );
            merge(&mut objects, &json!([{"id": 9, "info": null}]));
            assert_eq!(usage(&objects).output, None, "no default named");
        }
        // Camera and screen sources name their readers.
        {
            let mut objects = Objects::new();
            merge(
                &mut objects,
                &json!([
                    node(70, "running", json!({"media.class": "Video/Source", "device.api": "v4l2", "node.name": "webcam"})),
                    node(71, "running", json!({"media.class": "Stream/Input/Video", "application.name": "Meet"})),
                    {"id": 72, "type": "PipeWire:Interface:Link", "info": {"output-node-id": 70, "input-node-id": 71}},
                    node(80, "running", json!({"media.class": "Video/Source", "node.name": "xdph-streaming-1"})),
                    node(81, "idle", json!({"media.class": "Video/Source", "device.api": "v4l2", "node.name": "idle-cam"})),
                ]),
            );
            let found = usage(&objects);
            assert_eq!(found.camera.iter().collect::<Vec<_>>(), ["Meet"]);
            assert_eq!(found.screen.len(), 1);
            assert!(found.microphone.is_empty());
        }
    }
}
