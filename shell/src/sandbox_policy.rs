//! Static Flatpak sandbox edits through the shared OS permissions worker.
//! Computed policy stays authoritative; only advertised user-override edits are writable.

use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

const DEADLINE: Duration = Duration::from_secs(30);
const MAX_OUTPUT: usize = 1 << 20;
const SESSION_BUS_POLICY: &str = "Session Bus Policy";
const SYSTEM_BUS_POLICY: &str = "System Bus Policy";

struct WorkerGroup(Option<rustix::process::Pid>);
impl Drop for WorkerGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
    }
}

/// The installed worker, if this system has one. A preview on a system
/// without it names a stand-in with `ENZUI_APP_PERMISSIONS_WORKER`.
#[must_use]
pub fn worker() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ENZUI_APP_PERMISSIONS_WORKER")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
    {
        return Some(path);
    }
    [
        "/usr/bin",
        "/usr/local/bin",
        "/usr/libexec/enzimos",
        "/usr/lib/enzimos",
    ]
    .iter()
    .map(|dir| PathBuf::from(dir).join("enzimos-app-permissions"))
    .find(|path| path.is_file())
}

async fn call(request: Value) -> Result<Value, String> {
    call_worker(
        worker().ok_or_else(|| tr("shell-the-permissions-service-is-not-installed"))?,
        request,
    )
    .await
}

async fn call_worker(worker: PathBuf, request: Value) -> Result<Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(worker);
    command
        .env_clear()
        .envs(
            [
                "HOME",
                "PATH",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
            ]
            .iter()
            .filter_map(|key| Some((key, std::env::var_os(key)?))),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    let run = async {
        let mut child = command
            .spawn()
            .map_err(|_| tr("shell-the-permissions-service-did-not-start"))?;
        // The unreaped child pins its process-group ID until this guard is disarmed.
        let mut group = WorkerGroup(
            child
                .id()
                .and_then(|pid| rustix::process::Pid::from_raw(pid as i32)),
        );
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| tr("shell-the-permissions-service-did-not-start"))?;
        input
            .write_all(request.to_string().as_bytes())
            .await
            .map_err(|_| tr("shell-the-permissions-service-did-not-answer"))?;
        drop(input);
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| tr("shell-the-permissions-service-did-not-answer"))?
            .take(MAX_OUTPUT as u64 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| tr("shell-the-permissions-service-did-not-answer"))?;
        if output.len() > MAX_OUTPUT {
            return Err(tr("shell-the-permissions-service-answered-too-much").into());
        }
        let status = child
            .wait()
            .await
            .map_err(|_| tr("shell-the-permissions-service-stopped-without-a-result"))?;
        group.0 = None;
        if !status.success() {
            return Err(
                tr("shell-the-permissions-service-failed-the-change-may-need-readback").into(),
            );
        }
        Ok::<_, String>(output)
    };
    let output = tokio::time::timeout(DEADLINE, run)
        .await
        .map_err(|_| tr("shell-the-permissions-service-took-too-long"))??;
    if output.len() > MAX_OUTPUT {
        return Err(tr("shell-the-permissions-service-answered-too-much").into());
    }
    let reply: Value = serde_json::from_slice(&output)
        .map_err(|_| tr("shell-the-permissions-service-answered-in-an-unknown-form"))?;
    match reply.get("ok") {
        Some(Value::Bool(true)) => Ok(reply["result"].clone()),
        _ => Err(tr("shell-the-permissions-could-not-be-read-or-changed").into()),
    }
}

/// What kind of access an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Shared,
    Sockets,
    Devices,
    Features,
    Filesystems,
    Persistent,
    SessionBus,
    SystemBus,
    Environment,
}
impl Category {
    const LISTS: [(Self, &'static str); 6] = [
        (Self::Shared, "shared"),
        (Self::Sockets, "sockets"),
        (Self::Devices, "devices"),
        (Self::Features, "features"),
        (Self::Filesystems, "filesystems"),
        (Self::Persistent, "persistent"),
    ];
    /// The worker's name for edits.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Sockets => "sockets",
            Self::Devices => "devices",
            Self::Features => "features",
            Self::Filesystems => "filesystems",
            Self::Persistent => "persistent",
            Self::SessionBus => "session_bus",
            Self::SystemBus => "system_bus",
            Self::Environment => "environment",
        }
    }
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Shared => tr("shell-network-and-sharing"),
            Self::Sockets => tr("shell-display-sound-and-services"),
            Self::Devices => tr("shell-devices"),
            Self::Features => tr("shell-features"),
            Self::Filesystems => tr("shell-files-and-folders"),
            Self::Persistent => tr("shell-kept-folders"),
            Self::SessionBus => tr("shell-other-apps-session-bus"),
            Self::SystemBus => tr("shell-system-services-system-bus"),
            Self::Environment => tr("shell-environment"),
        }
    }
}

// Values the worker can switch for the list categories.
const SWITCHABLE: &[(Category, &[&str])] = &[
    (Category::Shared, &["network", "ipc"]),
    (
        Category::Sockets,
        &[
            "x11",
            "fallback-x11",
            "wayland",
            "pulseaudio",
            "session-bus",
            "system-bus",
            "ssh-auth",
            "pcsc",
            "cups",
            "gpg-agent",
        ],
    ),
    (
        Category::Devices,
        &["all", "dri", "kvm", "shm", "input", "usb"],
    ),
    (
        Category::Features,
        &[
            "devel",
            "multiarch",
            "bluetooth",
            "canbus",
            "per-app-dev-shm",
        ],
    ),
];

/// Which policy layer decides an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    App,
    SystemAll,
    SystemApp,
    UserAll,
    UserApp,
    /// Present in Flatpak's computed context but in no layer read here
    /// (a runtime or launch-time addition).
    Computed,
    /// Not granted anywhere; listed so it can be allowed.
    Absent,
}
impl Source {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::App => tr("shell-requested-by-the-app"),
            Self::SystemAll => tr("shell-system-policy-for-all-apps"),
            Self::SystemApp => tr("shell-system-policy-for-this-app"),
            Self::UserAll => tr("shell-your-policy-for-all-apps"),
            Self::UserApp => tr("shell-your-change"),
            Self::Computed => tr("shell-added-by-flatpak"),
            Self::Absent => tr("shell-not-requested"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub category: Category,
    /// The value as Flatpak names it (`network`, `xdg-download:ro`,
    /// `org.example.Service`, `NAME`).
    pub value: String,
    pub allowed: bool,
    /// For a bus name, `talk` or `own`; for a variable, its value.
    pub detail: Option<String>,
    pub source: Source,
    /// The worker can change this entry.
    pub editable: bool,
    /// For a denied folder, the value that allows it again: its last
    /// granted mode, or read-only when no layer grants it.
    pub restore: Option<String>,
}
impl Entry {
    /// What a broad entry means beyond its name.
    #[must_use]
    pub fn caveat(&self) -> Option<&'static str> {
        if !self.allowed {
            return None;
        }
        Some(match (self.category, self.value.as_str()) {
            (Category::Sockets, "pulseaudio") => tr(
                "shell-direct-access-to-speakers-and-microphones-the-microphone-permission-does-not",
            ),
            (Category::Sockets, "x11" | "fallback-x11") => {
                tr("shell-x11-apps-can-see-other-x11-windows-and-keystrokes")
            }
            (Category::Sockets, "session-bus") => {
                tr("shell-can-talk-to-every-app-and-service-in-your-session")
            }
            (Category::Sockets, "system-bus") => tr("shell-can-talk-to-every-system-service"),
            (Category::Devices, "all") => {
                tr("shell-every-device-including-cameras-and-usb-devices")
            }
            (Category::Filesystems, "host" | "host-os" | "host:rw" | "host:ro") => {
                tr("shell-every-file-on-this-computer")
            }
            (Category::Filesystems, "home" | "home:rw" | "home:ro") => {
                tr("shell-your-whole-home-folder")
            }
            _ => return None,
        })
    }
}

/// One layer's rows (`name`, `value`, `decided by`); `None` when the layer
/// does not apply to this installation.
pub type LayerRows = Option<Vec<(String, String, String)>>;

/// One app's reviewed sandbox policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub app_id: String,
    pub scope: String,
    pub review_token: String,
    pub entries: Vec<Entry>,
    /// Groups and keys Settings does not interpret, shown as they are.
    pub unknown: Vec<(String, String, String)>,
    pub limitations: Vec<String>,
    pub writable: bool,
    /// Ordered app, system and user policy layers; `None` marks an inapplicable layer.
    pub layers: Vec<(&'static str, LayerRows)>,
}

fn list(value: &str) -> Vec<String> {
    // GLib string lists; an escaped separator stays inside its item.
    let mut items = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push(match c {
                's' => ' ',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ';' {
            if !current.is_empty() {
                items.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

fn bare(item: &str) -> &str {
    item.strip_prefix('!').unwrap_or(item)
}

// Whether `layer` mentions `value` in `category`, and as what.
fn mention(layer: &Value, category: Category, value: &str) -> Option<bool> {
    let lookup = |group: &str, key: &str| {
        layer
            .get(group)
            .and_then(|g| g.get(key))
            .and_then(Value::as_str)
    };
    match category {
        Category::SessionBus | Category::SystemBus => {
            let group = if category == Category::SessionBus {
                SESSION_BUS_POLICY
            } else {
                SYSTEM_BUS_POLICY
            };
            lookup(group, value).map(|level| level != "none")
        }
        Category::Environment => lookup("Environment", value).map(|_| true),
        _ => {
            let key = Category::LISTS.iter().find(|(c, _)| *c == category)?.1;
            let items = list(lookup("Context", key)?);
            let name = value.split(':').next().unwrap_or(value);
            items
                .iter()
                .rev()
                .find(|item| bare(item).split(':').next() == Some(name))
                .map(|item| !item.starts_with('!'))
        }
    }
}

fn rows(layer: &Value) -> Vec<(String, String, String)> {
    layer
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(group, keys)| {
            keys.as_object()
                .into_iter()
                .flatten()
                .map(move |(key, value)| {
                    (
                        group.clone(),
                        key.clone(),
                        value.as_str().unwrap_or("").to_owned(),
                    )
                })
        })
        .collect()
}

/// Reads the worker's `inspect` result for `app_id` in `scope`; a result of
/// another schema or for another subject is refused.
pub fn parse(result: &Value, app_id: &str, scope: &str) -> Result<Snapshot, String> {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let subject = &result["subject"];
    if result["schema_version"] != json!(1)
        || subject["app_id"].as_str() != Some(app_id)
        || subject["scope"].as_str() != Some(scope)
    {
        return Err(tr("shell-the-permissions-service-answered-for-something-else").into());
    }
    let app_id = app_id.to_owned();
    let computed = &result["flatpak_computed_context"];
    // Layers in the order Flatpak applies them; the last mention decides.
    let mut layers: Vec<(Source, &Value)> = vec![(Source::App, &result["metadata"])];
    for source in result["sources"].as_array().into_iter().flatten() {
        if source["applicable"] != Value::Bool(true) {
            continue;
        }
        let which = match (source["scope"].as_str(), source["target"].as_str()) {
            (Some("system"), Some("global")) => Source::SystemAll,
            (Some("system"), _) => Source::SystemApp,
            (Some("user"), Some("global")) => Source::UserAll,
            (Some("user"), _) => Source::UserApp,
            _ => continue,
        };
        layers.push((which, &source["policy"]));
    }
    let mut shown_layers: Vec<(&'static str, LayerRows)> =
        vec![(Source::App.label(), Some(rows(&result["metadata"])))];
    for source in result["sources"].as_array().into_iter().flatten() {
        let label = match (source["scope"].as_str(), source["target"].as_str()) {
            (Some("system"), Some("global")) => Source::SystemAll.label(),
            (Some("system"), _) => Source::SystemApp.label(),
            (Some("user"), Some("global")) => Source::UserAll.label(),
            _ => Source::UserApp.label(),
        };
        shown_layers.push((
            label,
            (source["applicable"] == Value::Bool(true)).then(|| rows(&source["policy"])),
        ));
    }
    let decide = |category: Category, value: &str| {
        layers
            .iter()
            .rev()
            .find(|(_, layer)| mention(layer, category, value).is_some())
            .map_or(Source::Computed, |(source, _)| *source)
    };
    let mut entries = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (category, key) in Category::LISTS {
        let mut values: Vec<(String, bool)> = computed["Context"][key]
            .as_str()
            .map(list)
            .unwrap_or_default()
            .into_iter()
            .map(|item| (bare(&item).to_owned(), !item.starts_with('!')))
            .collect();
        // A value any layer denies stays listed, so it can be allowed again.
        for (_, layer) in &layers {
            for item in layer["Context"][key].as_str().map(list).unwrap_or_default() {
                let name = bare(&item).to_owned();
                if !values
                    .iter()
                    .any(|(known, _)| known.split(':').next() == name.split(':').next())
                {
                    values.push((name, false));
                }
            }
        }
        if category == Category::Shared && !values.iter().any(|(v, _)| v == "network") {
            values.push(("network".into(), false));
        }
        for (value, allowed) in values {
            if !seen.insert((category, value.clone())) {
                continue;
            }
            let editable = match category {
                Category::Filesystems => true,
                Category::Persistent => false,
                _ => SWITCHABLE
                    .iter()
                    .any(|(c, known)| *c == category && known.contains(&value.as_str())),
            };
            let mentioned = layers
                .iter()
                .any(|(_, layer)| mention(layer, category, &value).is_some());
            let source = if !allowed && !mentioned {
                Source::Absent
            } else {
                decide(category, &value)
            };
            // Allowing a denied folder again restores the mode a layer last
            // granted, never more than read-only by default.
            let restore = (category == Category::Filesystems && !allowed).then(|| {
                let name = value.split(':').next().unwrap_or(&value).to_owned();
                layers
                    .iter()
                    .rev()
                    .flat_map(|(_, layer)| {
                        layer["Context"][key]
                            .as_str()
                            .map(list)
                            .unwrap_or_default()
                            .into_iter()
                            .rev()
                    })
                    .find(|item| {
                        !item.starts_with('!')
                            && item.split(':').next() == Some(name.as_str())
                            && matches!(item.split(':').nth(1), Some("ro" | "rw" | "create"))
                    })
                    .unwrap_or_else(|| format!("{name}:ro"))
            });
            entries.push(Entry {
                source,
                category,
                value,
                allowed,
                detail: None,
                editable,
                restore,
            });
        }
    }
    for (category, group) in [
        (Category::SessionBus, SESSION_BUS_POLICY),
        (Category::SystemBus, SYSTEM_BUS_POLICY),
    ] {
        let mut policy = computed[group].as_object().cloned().unwrap_or_default();
        for (_, layer) in &layers {
            for (name, level) in layer[group].as_object().into_iter().flatten() {
                policy.entry(name.clone()).or_insert_with(|| level.clone());
            }
        }
        for (name, _) in &policy {
            let level = computed[group][name].as_str().unwrap_or("none");
            entries.push(Entry {
                category,
                value: name.clone(),
                allowed: level != "none",
                detail: Some(level.to_owned()),
                source: decide(category, name),
                editable: matches!(level, "talk" | "own" | "none"),
                restore: (level == "none").then(|| {
                    let granted = layers
                        .iter()
                        .rev()
                        .filter_map(|(_, layer)| layer[group][name].as_str())
                        .find(|level| matches!(*level, "talk" | "own"))
                        .unwrap_or("talk");
                    format!("{name}={granted}")
                }),
            });
        }
    }
    for (name, value) in computed["Environment"].as_object().into_iter().flatten() {
        entries.push(Entry {
            category: Category::Environment,
            value: name.clone(),
            allowed: true,
            detail: value.as_str().map(str::to_owned),
            source: decide(Category::Environment, name),
            editable: false,
            restore: None,
        });
    }
    let known_groups = [
        "Context",
        SESSION_BUS_POLICY,
        SYSTEM_BUS_POLICY,
        "Environment",
    ];
    let mut unknown = Vec::new();
    for (group, keys) in computed.as_object().into_iter().flatten() {
        for (key, value) in keys.as_object().into_iter().flatten() {
            let known_key = group == "Context" && Category::LISTS.iter().any(|(_, k)| k == key);
            if !known_groups.contains(&group.as_str()) || (group == "Context" && !known_key) {
                unknown.push((
                    group.clone(),
                    key.clone(),
                    value.as_str().unwrap_or("").to_owned(),
                ));
            }
        }
    }
    Ok(Snapshot {
        app_id,
        scope: scope.to_owned(),
        review_token: text(&result["review_token"])
            .ok_or_else(|| tr("shell-the-permissions-review-is-incomplete"))?,
        entries,
        unknown,
        limitations: result["limitations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(text)
            .collect(),
        writable: result["writable"] == Value::Bool(true),
        layers: shown_layers,
    })
}

// Which installation holds `app_id`: `user` or `system`.
async fn scope(app_id: &str) -> String {
    let user = tokio::process::Command::new("flatpak")
        .args(["info", "--user", "--show-commit", "--", app_id])
        .env_clear()
        .envs(
            [
                "HOME",
                "PATH",
                "XDG_DATA_HOME",
                "XDG_CONFIG_HOME",
                "XDG_RUNTIME_DIR",
            ]
            .iter()
            .filter_map(|key| Some((key, std::env::var_os(key)?))),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .is_ok_and(|status| status.success());
    if user { "user" } else { "system" }.into()
}

// One overall deadline for a whole review, scope detection included.
async fn within<T>(
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(DEADLINE, work)
        .await
        .map_err(|_| tr("shell-the-permissions-service-took-too-long").to_owned())?
}

pub async fn inspect(app_id: String) -> Result<Snapshot, String> {
    within(async {
        let scope = scope(&app_id).await;
        parse(&call(json!({"schema_version": 1, "method": "inspect", "subject": {"app_id": app_id, "scope": scope}})).await?, &app_id, &scope)
    })
    .await
}

/// One edit: allow or deny `value` in `category`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub category: Category,
    pub value: String,
    pub allow: bool,
}

/// Applies `edits` (or, with `reset`, this user's override for the app is
/// removed) against the reviewed snapshot, and returns the verified new one.
pub async fn apply(snapshot: Snapshot, edits: Vec<Edit>, reset: bool) -> Result<Snapshot, String> {
    within(apply_now(snapshot, edits, reset)).await
}

async fn apply_now(snapshot: Snapshot, edits: Vec<Edit>, reset: bool) -> Result<Snapshot, String> {
    let changes: Vec<Value> = edits
        .iter()
        .map(|e| json!({"category": e.category.key(), "value": e.value, "allow": e.allow}))
        .collect();
    let result = call(json!({
        "schema_version": 1,
        "method": "apply",
        "subject": {"app_id": snapshot.app_id, "scope": snapshot.scope},
        "review_token": snapshot.review_token,
        "changes": changes,
        "reset": reset,
    }))
    .await?;
    match result["status"].as_str() {
        Some("applied-readback-unavailable") => Err(tr(
            "shell-the-change-was-made-but-could-not-be-read-back-open-the-app-s-details-again",
        )
        .into()),
        _ if result.get("snapshot").is_some() => {
            parse(&result["snapshot"], &snapshot.app_id, &snapshot.scope)
        }
        _ => parse(&result, &snapshot.app_id, &snapshot.scope),
    }
}

/// One remembered portal answer for the app: its table, resource and the
/// stored values, exactly as the permission store keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub table: String,
    pub resource: String,
    pub values: Vec<String>,
}
impl Grant {
    /// Whether this is a plain yes/no answer the worker can change.
    #[must_use]
    pub fn switchable(&self) -> bool {
        let known = matches!(
            (self.table.as_str(), self.resource.as_str()),
            ("devices", "camera" | "microphone" | "speakers")
                | ("notifications", "notification")
                | ("background", "background")
        );
        known && (self.values == ["yes"] || self.values == ["no"])
    }
    #[must_use]
    pub fn label(&self) -> String {
        match (self.table.as_str(), self.resource.as_str()) {
            ("devices", "camera") => tr("shell-camera").into(),
            ("devices", "microphone") => tr("shell-microphone").into(),
            ("devices", "speakers") => tr("shell-speakers").into(),
            ("notifications", _) => tr("shell-notifications").into(),
            ("background", _) => tr("shell-run-in-the-background").into(),
            ("location", _) => tr("shell-location").into(),
            ("screenshot", _) => tr("shell-take-screenshots").into(),
            ("screencast", _) => tr("shell-record-the-screen").into(),
            (table, resource) => format!("{table} · {resource}"),
        }
    }
}

/// The app's remembered portal answers, reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portals {
    pub app_id: String,
    pub review_token: String,
    pub grants: Vec<Grant>,
    /// Tables the permission store does not have at all.
    pub absent: Vec<String>,
}

fn parse_portals(result: &Value, app_id: &str) -> Result<Portals, String> {
    if result["schema_version"] != json!(1) || result["app_id"].as_str() != Some(app_id) {
        return Err(tr("shell-the-permissions-service-answered-for-something-else").into());
    }
    let text = |value: &Value| value.as_str().map(str::to_owned);
    Ok(Portals {
        app_id: app_id.to_owned(),
        review_token: text(&result["review_token"])
            .ok_or_else(|| tr("shell-the-permissions-review-is-incomplete"))?,
        grants: result["grants"]
            .as_array()
            .filter(|grants| grants.len() <= 512)
            .ok_or_else(|| tr("shell-invalid-portal-grants"))?
            .iter()
            .map(|grant| {
                let table = text(&grant["table"])
                    .filter(|v| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control))
                    .ok_or_else(|| tr("shell-invalid-portal-table"))?;
                let resource = text(&grant["resource"])
                    .filter(|v| {
                        !v.is_empty() && v.len() <= 1024 && !v.chars().any(char::is_control)
                    })
                    .ok_or_else(|| tr("shell-invalid-portal-resource"))?;
                let values = grant["values"]
                    .as_array()
                    .filter(|values| values.len() <= 64)
                    .ok_or_else(|| tr("shell-invalid-portal-values"))?
                    .iter()
                    .map(|value| {
                        text(value)
                            .filter(|value| {
                                value.len() <= 4096 && !value.chars().any(char::is_control)
                            })
                            .ok_or_else(|| tr("shell-invalid-portal-value").to_owned())
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(Grant {
                    table,
                    resource,
                    values,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        absent: result["tables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["status"] == "absent")
            .filter_map(|t| text(&t["table"]))
            .collect(),
    })
}

pub async fn portal_inspect(app_id: String) -> Result<Portals, String> {
    within(async {
        parse_portals(
            &call(json!({"schema_version": 1, "method": "portal.inspect", "app_id": app_id}))
                .await?,
            &app_id,
        )
    })
    .await
}

/// Changes one existing remembered answer to `yes` or `no`, or with `None`
/// forgets only this app's answer, so the app asks again.
pub async fn portal_change(
    portals: Portals,
    grant: Grant,
    allow: Option<bool>,
) -> Result<Portals, String> {
    if !portals.grants.contains(&grant) || (allow.is_some() && !grant.switchable()) {
        return Err(tr("shell-this-answer-is-not-in-the-current-review").into());
    }
    within(async {
        let values = allow.map_or(Value::Null, |on| json!([if on { "yes" } else { "no" }]));
        let result = call(json!({
            "schema_version": 1,
            "method": "portal.change",
            "app_id": portals.app_id,
            "review_token": portals.review_token,
            "table": grant.table,
            "resource": grant.resource,
            "values": values,
        }))
        .await?;
        match result["status"].as_str() {
            Some("applied-readback-unavailable") => Err(tr(
                "shell-the-change-was-made-but-could-not-be-read-back-open-the-app-s-details-again",
            )
            .into()),
            _ => parse_portals(&result["snapshot"], &portals.app_id),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({
            "schema_version": 1,
            "subject": {"app_id": "org.example.App", "scope": "user"},
            "commit": "0".repeat(64),
            "metadata": {"Context": {"shared": "network;ipc;", "sockets": "x11;pulseaudio;", "filesystems": "xdg-download;home:ro;"},
                          "Session Bus Policy": {"org.freedesktop.Notifications": "talk"}},
            "flatpak_computed_context": {
                "Context": {"shared": "ipc;", "sockets": "x11;pulseaudio;", "filesystems": "xdg-download;home:ro;", "unknown-key": "x"},
                "Session Bus Policy": {"org.freedesktop.Notifications": "talk"},
                "Environment": {"LANG": "C"},
                "Extension org.example": {"directory": "lib"},
            },
            "sources": [
                {"scope": "system", "target": "global", "applicable": false, "status": "not-applicable"},
                {"scope": "user", "target": "global", "applicable": true, "policy": {}},
                {"scope": "user", "target": "org.example.App", "applicable": true, "policy": {"Context": {"shared": "!network;"}}},
            ],
            "writable": true,
            "limitations": ["Runtime conditionals and launch-time arguments can alter access."],
            "review_token": "abc",
        })
    }

    #[tokio::test]
    async fn sandbox_policy_behaviour() {
        // Absent computed bus denials and unknown filesystem modes stay conservative;
        // Flatpak group names are file keys that the UI language must not change.
        enzui_core::localization::Localizer::new(
            crate::i18n::catalog(),
            vec!["es".parse().unwrap()],
            Default::default(),
        )
        .scope(|| {
            let mut result = fixture();
            result["flatpak_computed_context"]["Session Bus Policy"] = json!({});
            result["metadata"]["Context"]["filesystems"] = json!("home:future;");
            result["sources"][2]["policy"] = json!({"Context":{"filesystems":"!home;"},"Session Bus Policy":{"org.freedesktop.Notifications":"none"}});
            result["flatpak_computed_context"]["Context"]["filesystems"] = json!("");
            let parsed = parse(&result, "org.example.App", "user").unwrap();
            let denied = parsed
                .entries
                .iter()
                .find(|entry| entry.category == Category::SessionBus)
                .unwrap();
            assert!(!denied.allowed);
            assert_eq!(denied.source, Source::UserApp);
            assert_eq!(
                denied.restore.as_deref(),
                Some("org.freedesktop.Notifications=talk")
            );
            assert_eq!(
                parsed
                    .entries
                    .iter()
                    .find(|entry| entry.category == Category::Filesystems)
                    .unwrap()
                    .restore
                    .as_deref(),
                Some("home:ro")
            );
        });

        // Oversized worker output terminates without waiting for worker exit.
        {
            use std::os::unix::fs::PermissionsExt;
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            std::fs::write(&worker,"#!/usr/bin/python\nimport sys,time\nsys.stdin.read()\nsys.stdout.write('x'*1100000);sys.stdout.flush();time.sleep(20)\n").unwrap();
            std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
            let result =
                tokio::time::timeout(Duration::from_secs(3), call_worker(worker, json!({}))).await;
            assert!(result.unwrap().unwrap_err().contains("too much"));
        }
        // Dropping worker call retires its owned child process group.
        async {
            use std::os::unix::fs::PermissionsExt;
            let directory = tempfile::tempdir().unwrap();
            let worker = directory.path().join("worker");
            let pid = directory.path().join("child.pid");
            std::fs::write(&worker,format!("#!/usr/bin/python\nimport subprocess,sys,time\nsys.stdin.read()\np=subprocess.Popen(['/usr/bin/sleep','20']);open({:?},'w').write(str(p.pid));time.sleep(20)\n",pid.to_str().unwrap())).unwrap();
            std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
            let task = tokio::spawn(call_worker(worker, json!({})));
            for _ in 0..100 {
                if pid.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let child = std::fs::read_to_string(&pid).unwrap();
            let stat = std::fs::read_to_string(format!("/proc/{child}/stat")).unwrap();
            let start = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .to_owned();
            task.abort();
            let _ = task.await;
            for _ in 0..100 {
                let alive = std::fs::read_to_string(format!("/proc/{child}/stat"))
                    .ok()
                    .is_some_and(|stat| {
                        let fields: Vec<_> = stat
                            .rsplit_once(')')
                            .unwrap()
                            .1
                            .split_whitespace()
                            .collect();
                        fields[19] == start && fields[0] != "Z"
                    });
                if !alive {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("owned worker child remained running after cancellation");
        }
        .await;
    }
}
