//! Session-bus window inventory attributed by verified per-app unit membership.
//! Mapped counts include minimized windows; known is false until a complete successful
//! observation.

use enzui_shell::i18n::tr;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SERVICE: &str = "dev.enzui.Shell";
pub const PATH: &str = "/dev/enzui/Shell/Windows";
const LIMIT: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppWindows {
    pub app_id: String,
    pub unit: String,
    pub main_pid: u32,
    pub start_ticks: u64,
    pub mapped: u32,
    pub minimized: u32,
}

/// The app unit a process runs in, from its `/proc/PID/cgroup` text:
/// (unit, app id). Only units under the user manager's `app.slice`.
#[must_use]
pub fn unit_of(cgroup: &str) -> Option<(String, String)> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let (slice, unit) = path.rsplit_once('/')?;
    if !slice.ends_with("/app.slice") || !slice.contains("/user@") {
        return None;
    }
    let app = if let Some(id) = unit
        .strip_prefix("enzui-agent-")
        .and_then(|rest| rest.strip_suffix(".service"))
    {
        id
    } else if let Some(rest) = unit
        .strip_prefix("app-flatpak-")
        .and_then(|rest| rest.strip_suffix(".scope"))
    {
        // Flatpak names its instance scopes `app-flatpak-<APP_ID>-<number>`.
        let (id, instance) = rest.rsplit_once('-')?;
        if instance.is_empty() || !instance.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        id
    } else {
        let rest = unit.strip_prefix("app-enzui-")?.strip_suffix(".scope")?;
        rest.rsplit_once('-')?.0
    };
    let valid = !app.is_empty()
        && app.len() <= 255
        && app
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    valid.then(|| (unit.to_owned(), app.to_owned()))
}

/// Field 22 of `/proc/PID/stat`: the process's start time in clock ticks.
#[must_use]
pub fn start_ticks(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

fn minimized(window: &Value) -> bool {
    window["hidden"].as_bool() == Some(true)
        || window["workspace"]["id"].as_i64().is_some_and(|id| id < 1)
        || window["workspace"]["type"].as_str() == Some("special")
        || window["workspace"]["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("special:"))
        || window["tags"].as_array().is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_str()
                    .is_some_and(|tag| tag.starts_with("enzui-sidebar-"))
            })
        })
}

/// Groups the compositor's clients by app unit. `cgroup` and `ticks` read a
/// process; `main_pid` asks systemd for a service's main process.
pub fn observe(
    clients: &[Value],
    own_pid: u32,
    cgroup: impl Fn(u32) -> Option<String>,
    ticks: impl Fn(u32) -> Option<u64>,
    main_pid: impl Fn(&str) -> Option<u32>,
) -> Vec<AppWindows> {
    let mut apps: Vec<AppWindows> = Vec::new();
    for window in clients.iter().take(1024) {
        let Some(pid) = window["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 1 && *pid != own_pid)
        else {
            continue;
        };
        if window["mapped"].as_bool() != Some(true) {
            continue;
        }
        let Some((unit, app_id)) = cgroup(pid).as_deref().and_then(unit_of) else {
            continue;
        };
        let minimized = u32::from(minimized(window));
        if let Some(known) = apps.iter_mut().find(|app| app.unit == unit) {
            known.mapped += 1;
            known.minimized += minimized;
            continue;
        }
        if apps.len() >= LIMIT {
            continue;
        }
        // A service's owner is its main process; a scope has none, so the
        // window's own process (verified to be in the scope) stands for it.
        let owner = if unit.ends_with(".service") {
            main_pid(&unit)
        } else {
            Some(pid)
        };
        let Some(owner) = owner else { continue };
        let Some(start) = ticks(owner) else { continue };
        apps.push(AppWindows {
            app_id,
            unit,
            main_pid: owner,
            start_ticks: start,
            mapped: 1,
            minimized,
        });
    }
    apps.sort_by(|a, b| a.unit.cmp(&b.unit));
    apps
}

#[derive(Default)]
struct State {
    generation: u64,
    known: bool,
    apps: Vec<AppWindows>,
}

struct Feed(Arc<Mutex<State>>);

// Wire tuple: app ID, unit, main PID, start ticks, mapped count, minimized count.
type AppRow = (String, String, u32, u64, u32, u32);

#[zbus::interface(name = "dev.enzui.Shell.Windows1")]
impl Feed {
    fn get_apps(&self) -> (u64, bool, Vec<AppRow>) {
        let state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let apps = state
            .apps
            .iter()
            .map(|a| {
                (
                    a.app_id.clone(),
                    a.unit.clone(),
                    a.main_pid,
                    a.start_ticks,
                    a.mapped,
                    a.minimized,
                )
            })
            .collect();
        (state.generation, state.known, apps)
    }
    #[zbus(signal)]
    async fn changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        generation: u64,
    ) -> zbus::Result<()>;
}

async fn main_pid(system: &zbus::Connection, unit: &str) -> Option<u32> {
    let manager = zbus::Proxy::new(
        system,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .await
    .ok()?;
    let path: zbus::zvariant::OwnedObjectPath = manager.call("GetUnit", &(unit,)).await.ok()?;
    let service = zbus::Proxy::new(
        system,
        "org.freedesktop.systemd1",
        path,
        "org.freedesktop.systemd1.Service",
    )
    .await
    .ok()?;
    service
        .get_property::<u32>("MainPID")
        .await
        .ok()
        .filter(|pid| *pid > 1)
}

async fn snapshot(
    compositor: &crate::compositor::Compositor,
    user: &zbus::Connection,
) -> Option<Vec<AppWindows>> {
    let text = compositor.request(&["-j", "clients"]).await.ok()?;
    if text.len() > 2 * 1024 * 1024 {
        return None;
    }
    let clients: Vec<Value> = serde_json::from_str(&text).ok()?;
    let own = std::process::id();
    let read = |pid: u32| std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok();
    let ticks = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .as_deref()
            .and_then(start_ticks)
    };
    // Services are resolved first (async), then the pure grouping runs.
    let mut services = Vec::new();
    for window in clients.iter().take(1024) {
        if let Some((unit, _)) = window["pid"]
            .as_u64()
            .and_then(|pid| read(u32::try_from(pid).ok()?))
            .as_deref()
            .and_then(unit_of)
            && unit.ends_with(".service")
            && !services.iter().any(|(known, _)| *known == unit)
            && services.len() < LIMIT
        {
            let pid = main_pid(user, &unit).await;
            services.push((unit, pid));
        }
    }
    Some(observe(&clients, own, read, ticks, |unit| {
        services
            .iter()
            .find(|(known, _)| known == unit)
            .and_then(|(_, pid)| *pid)
    }))
}

/// Serves the feed for as long as the stream is held. It never yields:
/// changes go out as the `Changed` signal.
pub fn stream(compositor: crate::compositor::Compositor) -> impl iced::futures::Stream<Item = ()> {
    iced::stream::channel(
        1,
        async move |_output: iced::futures::channel::mpsc::Sender<()>| {
            if let Err(error) = serve(compositor).await {
                eprintln!("enzui-shell windows feed: {error}");
            }
            std::future::pending::<()>().await;
        },
    )
}

async fn serve(compositor: crate::compositor::Compositor) -> Result<(), String> {
    let state = Arc::new(Mutex::new(State::default()));
    let connection = zbus::connection::Builder::session()
        .map_err(|_| tr("shell-no-session-bus"))?
        .serve_at(PATH, Feed(std::sync::Arc::clone(&state)))
        .map_err(|_| tr("shell-cannot-serve-the-windows-feed"))?
        .build()
        .await
        .map_err(|_| tr("shell-cannot-connect-to-the-session-bus"))?;
    let reply = connection
        .request_name_with_flags(SERVICE, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
        .map_err(|_| tr("shell-another-shell-owns-dev-enzui-shell"))?;
    if reply != zbus::fdo::RequestNameReply::PrimaryOwner {
        return Err(tr("shell-another-shell-owns-dev-enzui-shell").into());
    }
    let user = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    let interface = connection
        .object_server()
        .interface::<_, Feed>(PATH)
        .await
        .map_err(|_| tr("shell-windows-feed-is-not-registered"))?;
    let mut events = Box::pin(crate::workspaces::window_events(compositor.event_socket()));
    loop {
        let observed = snapshot(&compositor, &user).await;
        let changed = {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (known, apps) = match observed {
                Some(apps) => (true, apps),
                None => (false, Vec::new()),
            };
            if state.known != known || state.apps != apps {
                state.known = known;
                state.apps = apps;
                state.generation += 1;
                Some(state.generation)
            } else {
                None
            }
        };
        if let Some(generation) = changed {
            let _ = Feed::changed(interface.signal_emitter(), generation).await;
        }
        // The next change, then a short settle so a burst is one update.
        use iced::futures::StreamExt;
        if events.next().await.is_none() {
            return Err(tr("shell-compositor-events-ended").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Some(Some(())) = iced::futures::FutureExt::now_or_never(events.next()) {}
    }
}
