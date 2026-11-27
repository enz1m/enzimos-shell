use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::Deserialize;
use std::time::Duration;

mod client;
mod details;
pub mod podman;
mod shortcuts;
mod url;
pub use client::Client;
pub use details::{Detail, Lookup};
pub use shortcuts::{Launch, ShortcutKind, Shortcuts};
pub use url::UrlLaunch;

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const NAME: &str = "org.enzimos.DevMonitor1";
const PATH: &str = "/org/enzimos/DevMonitor1";
const VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub system: bool,
    pub udp: bool,
    pub containers: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
pub struct ProcessId {
    pub pid: u32,
    pub start_time: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ContainerId {
    pub engine: String,
    pub socket: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
}

impl ContainerId {
    pub fn valid(&self) -> bool {
        matches!(self.engine.as_str(), "docker" | "podman")
            && std::path::Path::new(&self.socket).is_absolute()
            && self.id.len() == 64
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Port {
    pub protocol: String,
    pub address: String,
    pub port: u16,
    pub exposure: String,
    pub own: bool,
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    pub source: String,
    #[serde(default)]
    pub published_by: Vec<ContainerId>,
    pub url: Option<String>,
}

impl Port {
    pub fn process_id(&self) -> Option<ProcessId> {
        if self.source != "socket" || !self.own {
            return None;
        }
        Some(ProcessId {
            pid: self.pid?,
            start_time: self.start_time?,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Launcher {
    #[serde(flatten)]
    pub identity: ProcessId,
    pub label: String,
    pub processes: usize,
    pub tree_limited: bool,
    pub rss_bytes: Option<u64>,
    pub cpu_percent: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Process {
    #[serde(flatten)]
    pub identity: ProcessId,
    pub label: String,
    pub cmdline: Vec<String>,
    pub cwd: Option<String>,
    pub cpu_percent: Option<f64>,
    pub rss_bytes: Option<u64>,
    pub launcher: Launcher,
    pub project: Option<Project>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Project {
    pub root: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Endpoint {
    pub engine: String,
    pub socket: String,
    pub rootless: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub health: Option<String>,
    pub autostart: Option<bool>,
    pub startup: Option<Startup>,
    pub compose: Option<Compose>,
    #[serde(default)]
    pub ports: Vec<PublishedPort>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    pub started_at: Option<String>,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub detail_error: Option<String>,
    pub stats_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Startup {
    pub on_engine_start: Option<bool>,
    pub unit_file_state: Option<String>,
    pub user_unit: Option<bool>,
    pub linger: Option<bool>,
    pub verification_age_ms: Option<u64>,
    pub at_login: Option<bool>,
    pub at_boot: Option<bool>,
    pub unit: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Compose {
    pub project: String,
    pub service: Option<String>,
    pub working_dir: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PublishedPort {
    pub protocol: String,
    pub host_address: String,
    pub host_port: u16,
    pub container_port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Mount {
    pub kind: String,
    pub source: String,
    pub destination: String,
    pub writable: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Engine {
    pub endpoint: Endpoint,
    pub containers: Vec<Container>,
    pub error: Option<String>,
    pub stale: bool,
    pub truncated: bool,
}

impl Engine {
    pub fn identity(&self, container: &Container) -> Option<ContainerId> {
        let id = ContainerId {
            engine: self.endpoint.engine.clone(),
            socket: self.endpoint.socket.clone(),
            id: container.id.clone(),
            name: container.name.clone(),
        };
        (!self.stale && !self.truncated && id.valid()).then_some(id)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProjectGroup {
    pub key: String,
    pub name: String,
    pub directory: Option<String>,
    pub processes: Vec<ProcessId>,
    pub launchers: Vec<ProcessId>,
    pub containers: Vec<ContainerId>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub scan_id: u64,
    pub taken_at_unix_ms: u64,
    pub ports: Vec<Port>,
    pub processes: Vec<Process>,
    pub engines: Vec<Engine>,
    #[serde(default)]
    pub projects: Vec<ProjectGroup>,
    pub warnings: Vec<String>,
}

impl Snapshot {
    /// The exact identity is still shown, so an action on it may be offered.
    #[must_use]
    pub fn shows_process(&self, id: ProcessId) -> bool {
        self.processes.iter().any(|process| process.identity == id)
    }
    pub fn parse(json: &str) -> Result<Self, String> {
        let data: Self = parse(json)?;
        if data.version != VERSION {
            return Err(
                tr!("shell-unsupported-dev-monitor-version-2", "arg0" => format!("{:}", data.version)),
            );
        }
        if data.ports.len() > 8192
            || data.processes.len() > 4096
            || data.engines.len() > 8
            || data.projects.len() > 4096
            || data.projects.iter().any(|group| {
                group.processes.len() > 4096
                    || group.launchers.len() > 4096
                    || group.containers.len() > 2048
            })
            || data
                .projects
                .iter()
                .map(|group| group.key.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len()
                != data.projects.len()
            || data
                .engines
                .iter()
                .any(|engine| engine.containers.len() > 256)
        {
            return Err(tr("shell-dev-monitor-inventory-exceeds-the-display-limit").into());
        }
        Ok(data)
    }

    pub fn project(&self, key: &str) -> Option<&ProjectGroup> {
        self.projects.iter().find(|group| group.key == key)
    }
    pub fn group_inventory(&self, group: &ProjectGroup) -> (usize, usize, usize) {
        let processes = group
            .processes
            .iter()
            .filter(|id| {
                self.processes
                    .iter()
                    .any(|process| process.identity == **id)
            })
            .collect::<std::collections::HashSet<_>>();
        let launchers = group
            .launchers
            .iter()
            .filter(|id| {
                self.processes
                    .iter()
                    .any(|process| process.launcher.identity == **id)
            })
            .collect::<std::collections::HashSet<_>>();
        let containers = group
            .containers
            .iter()
            .filter(|id| {
                self.engines.iter().any(|engine| {
                    !engine.stale
                        && !engine.truncated
                        && engine.endpoint.engine == id.engine
                        && engine.endpoint.socket == id.socket
                        && engine
                            .containers
                            .iter()
                            .any(|container| container.id == id.id)
                })
            })
            .map(|id| (&id.engine, &id.socket, &id.id))
            .collect::<std::collections::HashSet<_>>();
        (processes.len(), launchers.len(), containers.len())
    }
    pub fn in_project(&self, key: &str) -> Option<Self> {
        let group = self.project(key)?;
        let mut snapshot = self.clone();
        snapshot
            .processes
            .retain(|process| group.processes.contains(&process.identity));
        snapshot.ports.retain(|port| {
            port.process_id()
                .is_some_and(|id| group.processes.contains(&id))
                || port.published_by.iter().any(|claim| {
                    group.containers.iter().any(|id| {
                        id.engine == claim.engine && id.socket == claim.socket && id.id == claim.id
                    })
                })
        });
        for engine in &mut snapshot.engines {
            engine.containers.retain(|container| {
                group.containers.iter().any(|id| {
                    id.engine == engine.endpoint.engine
                        && id.socket == engine.endpoint.socket
                        && id.id == container.id
                })
            });
        }
        snapshot.projects.clear();
        Some(snapshot)
    }
    pub fn process(&self, port: &Port) -> Option<&Process> {
        let id = port.process_id()?;
        self.processes.iter().find(|p| p.identity == id)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SignalReport {
    pub signal: String,
    pub delivered: Vec<u32>,
    pub skipped: Vec<Skipped>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Skipped {
    pub pid: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Process(ProcessId),
    Container(ContainerId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Stop,
    ForceStop,
    Start,
    Restart,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stop => tr("shell-stop"),
            Self::ForceStop => tr("shell-force-stop"),
            Self::Start => tr("shell-start"),
            Self::Restart => tr("shell-restart"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshTicket {
    epoch: u64,
    serial: u64,
    pub options: Options,
}

#[derive(Default)]
pub struct RefreshGate {
    visible: bool,
    epoch: u64,
    serial: u64,
    pending: Option<RefreshTicket>,
}

impl RefreshGate {
    pub fn open(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.visible = false;
        self.pending = None;
    }

    pub fn begin(&mut self, options: Options) -> Option<RefreshTicket> {
        if !self.visible || self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        let ticket = RefreshTicket {
            epoch: self.epoch,
            serial: self.serial,
            options,
        };
        self.pending = Some(ticket);
        Some(ticket)
    }

    pub fn finish(&mut self, ticket: RefreshTicket, options: Options) -> bool {
        if self.pending != Some(ticket) {
            return false;
        }
        self.pending = None;
        self.visible && ticket.epoch == self.epoch && ticket.options == options
    }
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, String> {
    if json.len() > MAX_RESPONSE {
        return Err(tr("shell-dev-monitor-response-is-too-large").into());
    }
    serde_json::from_str(json)
        .map_err(|error| tr!("shell-invalid-dev-monitor-response", "arg0" => format!("{:}", error)))
}
