use enzui_shell::i18n::tr;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktop {
    pub id: u32,
    pub windows: u32,
    pub placeholder: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktops {
    pub active: u32,
    pub entries: Vec<Desktop>,
}
impl Desktops {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 32768 {
            return Err(tr("shell-desktop-snapshot-exceeds-its-bound").into());
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|_| tr("shell-invalid-desktop-list"))?;
        let integer =
            |value: &serde_json::Value| value.as_u64().and_then(|v| u32::try_from(v).ok());
        let valid_id = |id: u32| id > 0 && id < 2_147_483_000;
        let active = integer(&value["active"])
            .filter(|id| valid_id(*id))
            .ok_or_else(|| tr("shell-invalid-active-desktop"))?;
        let rows = value["desktops"]
            .as_array()
            .filter(|rows| rows.len() >= 2 && rows.len() <= 128)
            .ok_or_else(|| tr("shell-invalid-desktop-count"))?;
        let mut ids = BTreeSet::new();
        let mut entries = Vec::new();
        for row in rows {
            let id = integer(&row["id"])
                .filter(|id| valid_id(*id))
                .ok_or_else(|| tr("shell-invalid-desktop-identity"))?;
            let windows =
                integer(&row["windows"]).ok_or_else(|| tr("shell-invalid-window-count"))?;
            let placeholder = row["placeholder"]
                .as_bool()
                .ok_or_else(|| tr("shell-missing-empty-desktop-state"))?;
            if !ids.insert(id) || (placeholder && windows != 0) {
                return Err(tr("shell-inconsistent-desktop-list").into());
            }
            entries.push(Desktop {
                id,
                windows,
                placeholder,
            });
        }
        if !ids.contains(&active)
            || entries.iter().filter(|d| d.placeholder).count() != 1
            || !entries.last().is_some_and(|d| d.placeholder)
        {
            return Err(tr("shell-desktop-list-lacks-its-active-or-trailing-empty-slot").into());
        }
        Ok(Self { active, entries })
    }
    pub fn active_index(&self) -> usize {
        self.entries
            .iter()
            .position(|d| d.id == self.active)
            .unwrap_or(0)
    }
}

#[derive(Default)]
pub struct WheelNavigation {
    pixels: f32,
    last: Option<std::time::Instant>,
}
impl WheelNavigation {
    pub fn step(
        &mut self,
        delta: iced::mouse::ScrollDelta,
        now: std::time::Instant,
    ) -> Option<i32> {
        let (x, y, pixels) = match delta {
            iced::mouse::ScrollDelta::Lines { x, y } => (x, y, false),
            iced::mouse::ScrollDelta::Pixels { x, y } => (x, y, true),
        };
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let amount = if y.abs() >= x.abs() { -y } else { x };
        if amount == 0.0 {
            return None;
        }
        if self.last.is_some_and(|last| {
            now.saturating_duration_since(last) > std::time::Duration::from_millis(180)
        }) {
            self.pixels = 0.0;
        }
        self.last = Some(now);
        if !pixels {
            self.pixels = 0.0;
            return Some(if amount > 0.0 { 1 } else { -1 });
        }
        if self.pixels.signum() != amount.signum() {
            self.pixels = 0.0;
        }
        self.pixels = (self.pixels + amount).clamp(-90.0, 90.0);
        if self.pixels.abs() >= 45.0 {
            let direction = if self.pixels > 0.0 { 1 } else { -1 };
            self.pixels = 0.0;
            Some(direction)
        } else {
            None
        }
    }
}

pub fn changes(socket: Option<std::path::PathBuf>) -> iced::Subscription<()> {
    iced::Subscription::run_with((socket, Events::Desktop), event_stream)
}

/// Active window changes and windows opening or closing, the moments the
/// work status of another window may show or hide.
pub fn focus_changes(socket: Option<std::path::PathBuf>) -> iced::Subscription<()> {
    iced::Subscription::run_with((socket, Events::Focus), event_stream)
}

pub(crate) fn keyboard_events(
    socket: Option<std::path::PathBuf>,
) -> impl iced::futures::Stream<Item = ()> {
    event_stream(&(socket, Events::Keyboard))
}

/// Windows of apps opening or closing, or being moved or parked in the sidebar:
/// what changes which apps may show or hide.
pub fn window_events(socket: Option<std::path::PathBuf>) -> impl iced::futures::Stream<Item = ()> {
    event_stream(&(socket, Events::Windows))
}

/// Emits on compositor config reload and when the event socket connects.
pub fn config_reloads(socket: Option<std::path::PathBuf>) -> iced::Subscription<()> {
    iced::Subscription::run_with((socket, Events::Config), event_stream)
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum Events {
    Desktop,
    Focus,
    Windows,
    Keyboard,
    Dock,
    Config,
    DeferredDialogs,
}

pub(crate) fn deferred_dialog_changes(
    socket: Option<std::path::PathBuf>,
) -> impl iced::futures::Stream<Item = ()> {
    event_stream(&(socket, Events::DeferredDialogs))
}

fn event_stream(
    (socket, events): &(Option<std::path::PathBuf>, Events),
) -> impl iced::futures::Stream<Item = ()> + use<> {
    use iced::futures::SinkExt;
    use std::os::unix::fs::MetadataExt;
    let socket = socket.clone();
    let wanted: fn(&[u8]) -> bool = match events {
        Events::Desktop => relevant_event,
        Events::Focus => focus_event,
        Events::Windows => window_event,
        Events::Keyboard => {
            |line| line.starts_with(b"activelayout>>") || line == b"configreloaded>>\n"
        }
        Events::Dock => dock_event,
        Events::Config => |line| line == b"configreloaded>>\n",
        Events::DeferredDialogs => |line| {
            line.starts_with(b"fullscreen_dialog_deferred>>")
                || line.starts_with(b"fullscreen_dialog_released>>")
                || line.starts_with(b"fullscreen_dialog_closed>>")
        },
    };
    iced::stream::channel(4, async move |mut output| {
        let Some(socket) = socket else {
            iced::futures::future::pending::<()>().await;
            return;
        };
        let uid = std::fs::metadata("/proc/self").ok().map(|m| m.uid());
        loop {
            if let Ok(Ok(stream)) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tokio::net::UnixStream::connect(&socket),
            )
            .await
                && uid.is_some()
                && stream.peer_cred().ok().map(|c| c.uid()) == uid
            {
                let mut reader = tokio::io::BufReader::new(stream);
                if output.send(()).await.is_err() {
                    return;
                }
                let mut shell = ShellWindows::default();
                while let Ok(line) = read_event(&mut reader).await {
                    if starts_motion(&line) {
                        crate::power_boost::request();
                    }
                    if !shell.owns(&line) && wanted(&line) && output.send(()).await.is_err() {
                        return;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    })
}

// Events whose compositor animation starts as they are sent.
fn starts_motion(line: &[u8]) -> bool {
    [
        b"openwindow>>".as_slice(),
        b"closewindow>>",
        b"workspacev2>>",
        b"activespecialv2>>",
        b"movewindowv2>>",
        b"fullscreen>>",
        b"changefloatingmode>>",
        b"custom>>enzui-sidebar",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn focus_event(line: &[u8]) -> bool {
    if let Some(window) = line.strip_prefix(b"activewindow>>") {
        let class = window
            .split(|byte| *byte == b',')
            .next()
            .unwrap_or_default();
        return !SHELL_CLASSES.contains(&class);
    }
    line.starts_with(b"openwindow>>") || line.starts_with(b"closewindow>>")
}

pub fn dock_events(socket: Option<std::path::PathBuf>) -> impl iced::futures::Stream<Item = ()> {
    event_stream(&(socket, Events::Dock))
}
fn dock_event(line: &[u8]) -> bool {
    relevant_event(line)
        || [
            b"fullscreen>>".as_slice(),
            b"windowtitlev2>>",
            b"windowclass>>",
            b"urgent>>",
            b"custom>>enzui-dock-changed",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}
fn window_event(line: &[u8]) -> bool {
    [
        b"openwindow>>".as_slice(),
        b"closewindow>>",
        b"movewindow>>",
        b"movewindowv2>>",
        b"custom>>enzui-sidebar",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn relevant_event(line: &[u8]) -> bool {
    [
        b"workspace>>".as_slice(),
        b"workspacev2>>",
        b"openwindow>>",
        b"closewindow>>",
        b"movewindow>>",
        b"movewindowv2>>",
        b"monitoradded>>",
        b"monitorremoved>>",
        b"focusedmon>>",
        b"activelayout>>",
        b"pin>>",
        b"custom>>enzui-sidebar",
        b"custom>>enzui-output-changed",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

// Shell surfaces do not trigger desktop refreshes during their own opening or focus changes.
const SHELL_CLASSES: &[&[u8]] = &[
    b"dev.enzui.AssistantPanel",
    b"dev.enzui.AssistantTrigger",
    b"dev.enzui.Capture",
    b"dev.enzui.ControlCenter",
    b"dev.enzui.Dock",
    b"dev.enzui.Desktop",
    b"dev.enzui.WritingTools",
    b"dev.enzui.EmojiPicker",
    b"dev.enzui.Keyboard",
    b"dev.enzui.Launcher",
    b"dev.enzui.NotificationCenter",
    b"dev.enzui.NotificationToasts",
    b"dev.enzui.Osd",
    b"dev.enzui.PluginPanel",
    b"dev.enzui.QuickSearch",
    b"dev.enzui.Sidebar",
    b"dev.enzui.SidebarBacking",
    b"dev.enzui.SidebarPane",
    b"dev.enzui.Switcher",
    b"dev.enzui.WindowSpread",
    b"dev.enzui.Shortcuts",
    b"dev.enzui.WorkspaceOverview",
];

/// Whether a window class belongs to one of the Shell's own surfaces.
pub fn is_shell_surface(class: &str) -> bool {
    SHELL_CLASSES.contains(&class.as_bytes())
}

#[derive(Default)]
struct ShellWindows(std::collections::HashSet<Vec<u8>>);

impl ShellWindows {
    fn owns(&mut self, line: &[u8]) -> bool {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let field =
            |rest: &[u8], index: usize| rest.split(|b| *b == b',').nth(index).map(<[u8]>::to_vec);
        if let Some(rest) = line.strip_prefix(b"openwindow>>") {
            let (Some(address), Some(class)) = (field(rest, 0), field(rest, 2)) else {
                return false;
            };
            if SHELL_CLASSES.contains(&class.as_slice()) {
                if self.0.len() < 256 {
                    self.0.insert(address);
                }
                return true;
            }
            false
        } else if let Some(address) = line.strip_prefix(b"closewindow>>") {
            self.0.remove(address)
        } else if let Some(rest) = line.strip_prefix(b"activewindowv2>>") {
            self.0.contains(rest)
        } else {
            false
        }
    }
}

async fn read_event(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncBufReadExt;
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let end = bytes.iter().position(|b| *b == b'\n');
        let count = end.map_or(bytes.len(), |end| end + 1);
        if line.len() + count > 65536 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        line.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_desktop_changes_start_motion_but_focus_and_titles_do_not() {
        assert!(starts_motion(
            b"openwindow>>5a1,2,io.github.enzui.Files,Files\n"
        ));
        assert!(starts_motion(b"workspacev2>>3,3\n"));
        assert!(!starts_motion(b"workspace>>3\n"));
        assert!(!starts_motion(
            b"activewindow>>io.github.enzui.Files,Files\n"
        ));
        assert!(!starts_motion(b"windowtitlev2>>5a1,Files\n"));
    }

    #[test]
    fn focus_events_ignore_shell_chrome_without_a_prior_mapping_event() {
        assert!(!focus_event(
            b"activewindow>>dev.enzui.Desktop,enzUI Desktop\n"
        ));
        assert!(!focus_event(
            b"activewindow>>dev.enzui.NotificationCenter,Notifications\n"
        ));
        assert!(focus_event(b"activewindow>>io.github.enzui.Files,Files\n"));
        assert!(focus_event(b"activewindow>>,\n"));
        assert!(!focus_event(b"activewindowv2>>56420f233200\n"));
    }

    #[test]
    fn desktop_snapshot_preserves_sparse_ids_and_rejects_false_placeholders() {
        let good = r#"{"active":3,"desktops":[{"id":1,"windows":2,"placeholder":false},{"id":3,"windows":0,"placeholder":false},{"id":7,"windows":0,"placeholder":true}]}"#;
        let state = Desktops::parse(good).unwrap();
        assert_eq!(state.active_index(), 1);
        assert_eq!(state.entries[2].id, 7);
        for bad in [
            good.replace("\"active\":3", "\"active\":4"),
            good.replace("\"id\":7", "\"id\":3"),
            good.replace(
                "\"windows\":0,\"placeholder\":true",
                "\"windows\":1,\"placeholder\":true",
            ),
            good.replace("\"placeholder\":true", "\"placeholder\":false"),
        ] {
            assert!(Desktops::parse(&bad).is_err());
        }
        assert!(Desktops::parse("{}").is_err());
    }
}
