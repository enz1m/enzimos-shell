//! Opt-in restoration of installed desktop-entry IDs and window placement.
//! Persists no command lines or window contents; temporary rules place new windows without
//! focusing them.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// At most this many apps are remembered.
pub const MAX: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    /// A desktop entry ID such as `org.gnome.TextEditor.desktop`.
    pub id: String,
    pub workspace: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

/// Where a window was: its display by connector name, and for a floating
/// window its position on that display and its size, in layout pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub monitor: String,
    pub floating: bool,
    pub at: [i32; 2],
    pub size: [i32; 2],
}
impl Place {
    fn valid(&self) -> bool {
        (1..=64).contains(&self.monitor.len())
            && self
                .monitor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && self.at.iter().all(|v| (-16384..=16384).contains(v))
            && self.size.iter().all(|v| (1..=16384).contains(v))
    }
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    apps: Vec<Saved>,
}

fn valid(saved: &Saved) -> bool {
    saved.id.ends_with(".desktop")
        && (9..=255).contains(&saved.id.len())
        && saved
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && (1..=1000).contains(&saved.workspace)
        && saved.place.as_ref().is_none_or(Place::valid)
}

#[must_use]
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(state.join("enzui/session-apps.json"))
}

/// Claims this login's one restore: true the first time per logind session
/// (a marker in the runtime directory, which ends with the login).
#[must_use]
pub fn first_restore_of_login() -> bool {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    else {
        return false;
    };
    let session = std::env::var("XDG_SESSION_ID").unwrap_or_default();
    if session.is_empty() || !session.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let marker = runtime.join(format!("enzui-apps-reopened-{session}"));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
        .is_ok()
}

/// Remembers one eligible window per installed app, preserving client-list order.
#[must_use]
pub fn choose(
    clients: &serde_json::Value,
    monitors: &serde_json::Value,
    by_class: &std::collections::HashMap<String, (String, PathBuf, String)>,
) -> Vec<Saved> {
    let pair = |value: &serde_json::Value| -> Option<[i32; 2]> {
        Some([
            i32::try_from(value.get(0)?.as_i64()?).ok()?,
            i32::try_from(value.get(1)?.as_i64()?).ok()?,
        ])
    };
    let mut chosen: Vec<Saved> = Vec::new();
    for client in clients.as_array().into_iter().flatten() {
        let class = client
            .get("class")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let workspace = client
            .get("workspace")
            .and_then(|w| w.get("id"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1);
        if class.is_empty()
            || client.get("mapped").and_then(serde_json::Value::as_bool) == Some(false)
        {
            continue;
        }
        let Some((id, _, _)) = by_class.get(&class.to_lowercase()) else {
            continue;
        };
        let monitor = monitors.as_array().into_iter().flatten().find(|monitor| {
            monitor
                .get("id")
                .is_some_and(|id| Some(id) == client.get("monitor"))
        });
        let place = monitor.and_then(|monitor| {
            let origin = [
                i32::try_from(monitor.get("x")?.as_i64()?).ok()?,
                i32::try_from(monitor.get("y")?.as_i64()?).ok()?,
            ];
            let at = pair(client.get("at")?)?;
            Some(Place {
                monitor: monitor.get("name")?.as_str()?.to_owned(),
                floating: client.get("floating").and_then(serde_json::Value::as_bool) == Some(true),
                at: [at[0] - origin[0], at[1] - origin[1]],
                size: pair(client.get("size")?)?,
            })
        });
        let mut saved = Saved {
            id: id.clone(),
            workspace,
            place,
        };
        if saved.place.as_ref().is_some_and(|place| !place.valid()) {
            saved.place = None;
        }
        if valid(&saved) && !chosen.iter().any(|known| known.id == saved.id) {
            chosen.push(saved);
        }
        if chosen.len() == MAX {
            break;
        }
    }
    chosen
}

pub fn save(path: &Path, apps: &[Saved]) -> std::io::Result<()> {
    let text = serde_json::to_vec(&Document {
        version: 1,
        apps: apps.iter().take(MAX).cloned().collect(),
    })?;
    enzui_core::atomic_state::save_atomic_raw(path, &text)
}

#[must_use]
pub fn load(path: &Path) -> Vec<Saved> {
    let Ok(text) = std::fs::read(path) else {
        return Vec::new();
    };
    if text.len() > 64 * 1024 {
        return Vec::new();
    }
    match serde_json::from_slice::<Document>(&text) {
        Ok(document) if document.version == 1 => {
            document.apps.into_iter().filter(valid).take(MAX).collect()
        }
        _ => Vec::new(),
    }
}

/// How long the placement rules wait for the reopened windows.
pub const PLACEMENT: std::time::Duration = std::time::Duration::from_secs(20);

/// Builds temporary placement rules until END_LUA; `start` launches non-D-Bus entries.
#[must_use]
pub fn launch_lua(
    entry: &Path,
    class: &str,
    saved: &Saved,
    connected: &[String],
    index: usize,
    start: bool,
) -> Option<String> {
    let text = entry.to_str()?;
    let workspace = saved.workspace;
    if !entry.is_absolute() || text.chars().any(char::is_control) || !valid(saved) {
        return None;
    }
    let place = match saved
        .place
        .as_ref()
        .filter(|place| connected.contains(&place.monitor))
    {
        Some(place) if place.floating => format!(
            ", monitor = {}, float = true, move = \"{} {}\", size = \"{} {}\"",
            serde_json::to_string(&place.monitor).ok()?,
            place.at[0],
            place.at[1],
            place.size[0],
            place.size[1]
        ),
        Some(place) => format!(
            ", monitor = {}",
            serde_json::to_string(&place.monitor).ok()?
        ),
        None => String::new(),
    };
    if class.is_empty()
        || class.len() > 255
        || !class
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return None;
    }
    let pattern = format!("^{}$", class.replace('.', "[.]"));

    let command = format!(
        "exec {}/usr/bin/gio launch '{}'",
        crate::app_scope::shell_prefix(class),
        text.replace('\'', "'\\''")
    );
    let launch = if start {
        format!(
            "return hl.dispatch(hl.dsp.exec_cmd({}))",
            serde_json::to_string(&command).ok()?
        )
    } else {
        "return \"ok\"".to_owned()
    };
    Some(format!(
        "enzui_restore_rules = enzui_restore_rules or {{}}\n\
         table.insert(enzui_restore_rules, hl.window_rule({{name = \"enzui-restore-{index}\", match = {{class = {}}}, workspace = \"{workspace} silent\"{place}}}))\n\
         {launch}",
        serde_json::to_string(&pattern).ok()?,
    ))
}

/// Retires the placement rules.
pub const END_LUA: &str = "for _, rule in ipairs(enzui_restore_rules or {}) do rule:set_enabled(false) end\nenzui_restore_rules = {}\nreturn \"ok\"";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_restore_behaviour() {
        // One window per app with a desktop entry is remembered with its place.
        {
            let by_class = std::collections::HashMap::from([
                (
                    "org.gnome.texteditor".to_owned(),
                    (
                        "org.gnome.TextEditor.desktop".to_owned(),
                        PathBuf::from("/x/org.gnome.TextEditor.desktop"),
                        "org.gnome.TextEditor".to_owned(),
                    ),
                ),
                (
                    "io.github.enzui.files".to_owned(),
                    (
                        "io.github.enzui.Files.desktop".to_owned(),
                        PathBuf::from("/x/io.github.enzui.Files.desktop"),
                        "io.github.enzui.Files".to_owned(),
                    ),
                ),
            ]);
            let monitors = json!([{"id": 0, "name": "DP-1", "x": 0, "y": 0}, {"id": 1, "name": "HDMI-A-1", "x": 1920, "y": 0}]);
            let clients = json!([
                {"class": "dev.enzui.Desktop", "workspace": {"id": 1}, "mapped": true},
                {"class": "org.gnome.TextEditor", "workspace": {"id": 2}, "mapped": true, "monitor": 1, "floating": true, "at": [2020, 80], "size": [800, 600]},
                {"class": "org.gnome.TextEditor", "workspace": {"id": 3}, "mapped": true},
                {"class": "io.github.enzui.Files", "workspace": {"id": -98}, "mapped": true},
                {"class": "io.github.enzui.Files", "workspace": {"id": 1}, "mapped": true, "monitor": 0, "floating": false, "at": [0, 36], "size": [960, 1044]},
                {"class": "unknown.App", "workspace": {"id": 1}, "mapped": true},
            ]);
            let chosen = choose(&clients, &monitors, &by_class);
            assert_eq!(
                chosen,
                [
                    Saved {
                        id: "org.gnome.TextEditor.desktop".into(),
                        workspace: 2,
                        place: Some(Place {
                            monitor: "HDMI-A-1".into(),
                            floating: true,
                            at: [100, 80],
                            size: [800, 600]
                        }),
                    },
                    Saved {
                        id: "io.github.enzui.Files.desktop".into(),
                        workspace: 1,
                        place: Some(Place {
                            monitor: "DP-1".into(),
                            floating: false,
                            at: [0, 36],
                            size: [960, 1044]
                        }),
                    },
                ]
            );
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("enzui/session-apps.json");
            save(&file, &chosen).unwrap();
            assert_eq!(load(&file), chosen);
            std::fs::write(&file, r#"{"version":1,"apps":[{"id":"x;rm.desktop","workspace":1},{"id":"a.b.desktop","workspace":0},{"id":"c.d.desktop","workspace":1,"place":{"monitor":"a b","floating":true,"at":[0,0],"size":[1,1]}}]}"#).unwrap();
            assert!(
                load(&file).is_empty(),
                "invalid IDs, workspaces and displays are dropped"
            );
            std::fs::write(
                &file,
                r#"{"version":1,"apps":[{"id":"a.b.desktop","workspace":4}]}"#,
            )
            .unwrap();
            assert_eq!(
                load(&file)[0].place,
                None,
                "lists saved before places were remembered still load"
            );
        }
        // Launching quotes the entry and places it silently.
        {
            let saved = |workspace, place| Saved {
                id: "a.b.desktop".into(),
                workspace,
                place,
            };
            let connected = ["DP-1".to_owned()];
            let lua = launch_lua(
                Path::new("/usr/share/applications/it's.desktop"),
                "org.gnome.TextEditor",
                &saved(3, None),
                &connected,
                0,
                true,
            )
            .unwrap();
            assert!(
                !launch_lua(
                    Path::new("/a.desktop"),
                    "a",
                    &saved(1, None),
                    &connected,
                    0,
                    false
                )
                .unwrap()
                .contains("exec_cmd")
            );
            assert!(
                lua.contains("workspace = \"3 silent\"}")
                    && lua.contains(r"'\\''")
                    && lua.contains("^org[.]gnome[.]TextEditor$")
            );
            assert!(
                launch_lua(
                    Path::new("relative.desktop"),
                    "a",
                    &saved(1, None),
                    &connected,
                    0,
                    true
                )
                .is_none()
            );
            assert!(
                launch_lua(
                    Path::new("/a.desktop"),
                    "a",
                    &saved(0, None),
                    &connected,
                    0,
                    true
                )
                .is_none()
            );
            assert!(
                launch_lua(
                    Path::new("/a.desktop"),
                    "a$|.*",
                    &saved(1, None),
                    &connected,
                    0,
                    true
                )
                .is_none(),
                "classes are plain names"
            );
            let floating = Place {
                monitor: "DP-1".into(),
                floating: true,
                at: [100, 80],
                size: [800, 600],
            };
            let lua = launch_lua(
                Path::new("/a.desktop"),
                "a",
                &saved(2, Some(floating.clone())),
                &connected,
                0,
                true,
            )
            .unwrap();
            assert!(lua.contains(r#"workspace = "2 silent", monitor = "DP-1", float = true, move = "100 80", size = "800 600"}"#), "{lua}");
            let tiled = Place {
                floating: false,
                ..floating.clone()
            };
            let lua = launch_lua(
                Path::new("/a.desktop"),
                "a",
                &saved(2, Some(tiled)),
                &connected,
                0,
                true,
            )
            .unwrap();
            assert!(
                lua.contains(r#"workspace = "2 silent", monitor = "DP-1"}"#),
                "{lua}"
            );
            let elsewhere = Place {
                monitor: "HDMI-A-1".into(),
                ..floating
            };
            let lua = launch_lua(
                Path::new("/a.desktop"),
                "a",
                &saved(2, Some(elsewhere)),
                &connected,
                0,
                true,
            )
            .unwrap();
            assert!(
                lua.contains("workspace = \"2 silent\"}"),
                "a disconnected display is not named: {lua}"
            );
        }
    }
}
