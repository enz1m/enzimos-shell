//! Bounded enzUI and GNOME D-Bus search providers; declarations follow
//! docs/search-providers.md.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

pub const ENZUI_INTERFACE: &str = "dev.enzui.SearchProvider1";
pub const GNOME_INTERFACE: &str = "org.gnome.Shell.SearchProvider2";
/// The most results Search shows from one provider.
pub const SHOWN: usize = 5;
/// The most a provider may be asked for.
pub const LIMIT: usize = 20;
/// How long Search waits for one provider's answer.
pub const DEADLINE: Duration = Duration::from_millis(1500);
const FILE_MAX: u64 = 4096;
// Explicit opening can start the app's service; typing keeps the shorter deadline.
const OPEN_DEADLINE: Duration = Duration::from_secs(10);
const ID_MAX: usize = 256;
const TITLE_MAX: usize = 200;
const SUBTITLE_MAX: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    Enzui,
    Gnome,
}

/// One declared provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The app's desktop entry ID, e.g. `io.github.enzui.Mail.desktop`.
    pub desktop_id: String,
    pub bus_name: String,
    pub object_path: String,
    pub protocol: Protocol,
    pub min_query: usize,
}

/// One result, already bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: Option<String>,
    pub score: Option<f64>,
}

fn bus_name(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    value.len() <= 255
        && parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && !part.starts_with(|c: char| c.is_ascii_digit())
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

fn object_path(value: &str) -> bool {
    value.len() <= 255
        && value.starts_with('/')
        && value.len() > 1
        && value.strip_prefix('/').is_some_and(|path| {
            path.split('/').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
}

fn desktop_id(value: &str) -> bool {
    value.len() <= 255
        && value.strip_suffix(".desktop").is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
}

/// A declaration file's contents, or `None` when it is not a valid one.
pub fn parse(text: &str, protocol: Protocol) -> Option<Declaration> {
    let group = match protocol {
        Protocol::Enzui => "[enzUI Search Provider]",
        Protocol::Gnome => "[Shell Search Provider]",
    };
    let mut inside = false;
    let mut keys: HashMap<&str, &str> = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line.eq_ignore_ascii_case(group);
            continue;
        }
        if !inside || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            keys.entry(key.trim()).or_insert_with(|| value.trim());
        }
    }
    let version = keys.get("Version")?.parse::<u32>().ok()?;
    let wanted = match protocol {
        Protocol::Enzui => 1,
        Protocol::Gnome => 2,
    };
    // GNOME's `DefaultDisabled` providers stay out until the person enables them.
    if version != wanted
        || keys
            .get("DefaultDisabled")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return None;
    }
    let declaration = Declaration {
        desktop_id: keys.get("DesktopId")?.to_string(),
        bus_name: keys.get("BusName")?.to_string(),
        object_path: keys.get("ObjectPath")?.to_string(),
        protocol,
        min_query: keys
            .get("MinQueryLength")
            .and_then(|v| v.parse::<usize>().ok())
            .map_or(2, |n| n.clamp(1, 16)),
    };
    (desktop_id(&declaration.desktop_id)
        && bus_name(&declaration.bus_name)
        && object_path(&declaration.object_path)
        && owned_by_app(&declaration))
    .then_some(declaration)
}

// The provider bus name must belong to its declared app; GNOME permits subnames.
fn owned_by_app(declaration: &Declaration) -> bool {
    let Some(app) = declaration.desktop_id.strip_suffix(".desktop") else {
        return false;
    };
    match declaration.protocol {
        Protocol::Enzui => declaration.bus_name == app,
        Protocol::Gnome => {
            declaration.bus_name == app
                || declaration
                    .bus_name
                    .strip_prefix(app)
                    .is_some_and(|rest| rest.starts_with('.'))
        }
    }
}

/// Why a provider gave no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Silence {
    /// Its app is not running; Search does not start apps to ask them.
    NotRunning,
    Failed(String),
}

// Query calls never auto-start providers; any OS-approved activation is handled separately.
async fn quiet<B, R>(
    connection: &zbus::Connection,
    declaration: &Declaration,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<R, Silence>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let failed = |error: zbus::Error| match &error {
        zbus::Error::MethodError(name, _, _)
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
            ) =>
        {
            Silence::NotRunning
        }
        _ => Silence::Failed(error.to_string()),
    };
    let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(connection)
        .destination(declaration.bus_name.as_str())
        .and_then(|b| b.path(declaration.object_path.as_str()))
        .and_then(|b| b.interface(interface))
        .map_err(failed)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(failed)?;
    proxy
        .call_with_flags(method, zbus::proxy::MethodFlags::NoAutoStart.into(), body)
        .await
        .map_err(failed)?
        .ok_or_else(|| Silence::Failed(tr("shell-no-reply").into()))
}

/// Earlier declaration directories win; enzUI providers supersede GNOME declarations for the
/// same app.
pub fn discover(data_dirs: &[PathBuf]) -> Vec<Declaration> {
    let mut seen_files = std::collections::HashSet::new();
    let mut found: Vec<Declaration> = Vec::new();
    for (sub, protocol) in [
        ("enzui/search-providers", Protocol::Enzui),
        ("gnome-shell/search-providers", Protocol::Gnome),
    ] {
        for dir in data_dirs {
            let Ok(listing) = std::fs::read_dir(dir.join(sub)) else {
                continue;
            };
            let mut names: Vec<_> = listing
                .flatten()
                .map(|item| item.path())
                .filter(|path| path.extension().is_some_and(|e| e == "ini"))
                .collect();
            names.sort();
            for path in names.into_iter().take(256) {
                let key = (
                    protocol,
                    path.file_name().map(std::borrow::ToOwned::to_owned),
                );
                if !seen_files.insert(key) {
                    continue;
                }
                let small =
                    std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= FILE_MAX);
                let Some(declaration) = small
                    .then(|| std::fs::read_to_string(&path).ok())
                    .flatten()
                    .and_then(|text| parse(&text, protocol))
                else {
                    continue;
                };
                if !found
                    .iter()
                    .any(|known| known.desktop_id == declaration.desktop_id)
                {
                    found.push(declaration);
                }
            }
        }
    }
    found
}

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`, as the desktop sees them.
pub fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(data_home)
        .chain(std::env::split_paths(&dirs).filter(|p| p.is_absolute()))
        .collect()
}

fn clip(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() > max {
        trimmed
            .chars()
            .take(max - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        trimmed
    }
}

fn string(map: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    map.get(key).and_then(|value| match &**value {
        Value::Str(text) => Some(text.as_str().to_owned()),
        _ => None,
    })
}

fn themed_icon(value: Option<String>) -> Option<String> {
    // A GIcon string: a plain name, or ". GThemedIcon name fallback…".
    let value = value?;
    let name = value
        .strip_prefix(". GThemedIcon ")
        .map_or(value.as_str(), |rest| {
            rest.split_whitespace().next().unwrap_or("")
        });
    (!name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
    .then(|| name.to_owned())
}

/// The results Search may show from one answer, in the provider's order.
pub fn bounded(rows: Vec<HashMap<String, OwnedValue>>, protocol: Protocol) -> Vec<Found> {
    let (title_key, subtitle_key, icon_key) = match protocol {
        Protocol::Enzui => ("title", "subtitle", "icon"),
        Protocol::Gnome => ("name", "description", "gicon"),
    };
    let mut found = Vec::new();
    for row in rows.into_iter().take(LIMIT) {
        let (Some(id), Some(title)) = (string(&row, "id"), string(&row, title_key)) else {
            continue;
        };
        // Line breaks and tabs become spaces; other control characters mean the title is not text.
        let odd = |c: char| c.is_control() && !c.is_whitespace();
        if id.is_empty() || id.len() > ID_MAX || title.trim().is_empty() || title.chars().any(odd) {
            continue;
        }
        let score = row.get("score").and_then(|value| match &**value {
            Value::F64(score) if score.is_finite() => Some(score.clamp(0.0, 1.0)),
            _ => None,
        });
        found.push(Found {
            id,
            title: clip(&title, TITLE_MAX),
            subtitle: string(&row, subtitle_key)
                .map(|s| clip(&s, SUBTITLE_MAX))
                .unwrap_or_default(),
            icon: themed_icon(string(&row, icon_key)),
            score,
        });
    }
    if found.iter().all(|f| f.score.is_some()) {
        found.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    found
}

fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .take(16)
        .map(str::to_owned)
        .collect()
}

fn timestamp() -> u32 {
    0
}

/// The desktop's session bus, connected once and shared by every call.
pub async fn session() -> Option<zbus::Connection> {
    static BUS: tokio::sync::OnceCell<Option<zbus::Connection>> =
        tokio::sync::OnceCell::const_new();
    BUS.get_or_init(|| async { zbus::Connection::session().await.ok() })
        .await
        .clone()
}

/// Whether the provider's bus name can answer: it runs, or the bus can start it.
pub async fn reachable(connection: &zbus::Connection, declaration: &Declaration) -> bool {
    let Ok(proxy) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(name) = zbus::names::BusName::try_from(declaration.bus_name.as_str()) else {
        return false;
    };
    if proxy.name_has_owner(name).await.unwrap_or(false) {
        return true;
    }
    proxy
        .list_activatable_names()
        .await
        .is_ok_and(|names| names.iter().any(|n| n.as_str() == declaration.bus_name))
}

/// Asks one provider for results, within [`DEADLINE`], without starting it.
pub async fn search(
    connection: &zbus::Connection,
    declaration: &Declaration,
    query: &str,
) -> Result<Vec<Found>, Silence> {
    let call = async {
        match declaration.protocol {
            Protocol::Enzui => {
                let options: HashMap<&str, Value<'_>> =
                    HashMap::from([("limit", Value::U32(SHOWN as u32))]);
                let rows: Vec<HashMap<String, OwnedValue>> = quiet(
                    connection,
                    declaration,
                    ENZUI_INTERFACE,
                    "Search",
                    &(query, options),
                )
                .await?;
                Ok(bounded(rows, Protocol::Enzui))
            }
            Protocol::Gnome => {
                let ids: Vec<String> = quiet(
                    connection,
                    declaration,
                    GNOME_INTERFACE,
                    "GetInitialResultSet",
                    &(terms(query),),
                )
                .await?;
                let ids: Vec<String> = ids
                    .into_iter()
                    .filter(|id| !id.is_empty() && id.len() <= ID_MAX)
                    .take(SHOWN)
                    .collect();
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                let rows: Vec<HashMap<String, OwnedValue>> = quiet(
                    connection,
                    declaration,
                    GNOME_INTERFACE,
                    "GetResultMetas",
                    &(ids,),
                )
                .await?;
                Ok(bounded(rows, Protocol::Gnome))
            }
        }
    };
    tokio::time::timeout(DEADLINE, call)
        .await
        .map_err(|_| Silence::Failed(tr("shell-did-not-answer-in-time").to_owned()))?
}

/// Asks the app to open one of its results without an activation token.
pub async fn activate(
    connection: &zbus::Connection,
    declaration: &Declaration,
    id: &str,
    query: &str,
) -> Result<(), String> {
    activate_with_token(connection, declaration, id, query, None).await
}

async fn ready_for_open(
    connection: &zbus::Connection,
    declaration: &Declaration,
) -> Result<(), String> {
    // A newly activated service may own its name before exporting this interface.
    // Retry only the read-only readiness probe, never an explicit open action.
    loop {
        match connection
            .call_method(
                Some(declaration.bus_name.as_str()),
                declaration.object_path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(ENZUI_INTERFACE, "Version"),
            )
            .await
        {
            Ok(reply) => {
                let version = reply
                    .body()
                    .deserialize::<OwnedValue>()
                    .map_err(|error| error.to_string())?;
                return if u32::try_from(version).ok() == Some(1) {
                    Ok(())
                } else {
                    Err(tr("shell-unsupported-provider-version").into())
                };
            }
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.UnknownInterface"
                        | "org.freedesktop.DBus.Error.UnknownObject"
                        | "org.freedesktop.DBus.Error.UnknownMethod"
                ) =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Opens a result with a fresh user-activation token when available; GNOME's
/// legacy interface has no token field and retains its timestamp framing.
pub async fn activate_with_token(
    connection: &zbus::Connection,
    declaration: &Declaration,
    id: &str,
    query: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let call = async {
        match declaration.protocol {
            Protocol::Enzui => {
                ready_for_open(connection, declaration).await?;
                let mut options: HashMap<&str, Value<'_>> =
                    HashMap::from([("timestamp", Value::U32(timestamp()))]);
                if let Some(token) = token.filter(|token| !token.is_empty() && token.len() <= 4096)
                {
                    options.insert("activation-token", Value::from(token));
                }
                connection
                    .call_method(
                        Some(declaration.bus_name.as_str()),
                        declaration.object_path.as_str(),
                        Some(ENZUI_INTERFACE),
                        "Activate",
                        &(id, query, options),
                    )
                    .await
            }
            Protocol::Gnome => {
                connection
                    .call_method(
                        Some(declaration.bus_name.as_str()),
                        declaration.object_path.as_str(),
                        Some(GNOME_INTERFACE),
                        "ActivateResult",
                        &(id, terms(query), timestamp()),
                    )
                    .await
            }
        }
        .map(drop)
        .map_err(|e| e.to_string())
    };
    tokio::time::timeout(OPEN_DEADLINE, call)
        .await
        .map_err(|_| tr("shell-did-not-answer-in-time").to_owned())?
}

/// Asks the app to show its own search without an activation token.
pub async fn launch_search(
    connection: &zbus::Connection,
    declaration: &Declaration,
    query: &str,
) -> Result<(), String> {
    launch_search_with_token(connection, declaration, query, None).await
}

/// Opens the app's search with a fresh user-activation token when available;
/// GNOME's legacy interface retains its timestamp framing.
pub async fn launch_search_with_token(
    connection: &zbus::Connection,
    declaration: &Declaration,
    query: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let call = async {
        match declaration.protocol {
            Protocol::Enzui => {
                ready_for_open(connection, declaration).await?;
                let mut options: HashMap<&str, Value<'_>> = HashMap::new();
                if let Some(token) = token.filter(|token| !token.is_empty() && token.len() <= 4096)
                {
                    options.insert("activation-token", Value::from(token));
                }
                connection
                    .call_method(
                        Some(declaration.bus_name.as_str()),
                        declaration.object_path.as_str(),
                        Some(ENZUI_INTERFACE),
                        "LaunchSearch",
                        &(query, options),
                    )
                    .await
            }
            Protocol::Gnome => {
                connection
                    .call_method(
                        Some(declaration.bus_name.as_str()),
                        declaration.object_path.as_str(),
                        Some(GNOME_INTERFACE),
                        "LaunchSearch",
                        &(terms(query), timestamp()),
                    )
                    .await
            }
        }
        .map(drop)
        .map_err(|e| e.to_string())
    };
    tokio::time::timeout(OPEN_DEADLINE, call)
        .await
        .map_err(|_| tr("shell-did-not-answer-in-time").to_owned())?
}

/// Returns an eligible installed provider app's display name, respecting visibility and
/// parental policy.
pub fn app_name(desktop_id: &str, data_dirs: &[PathBuf]) -> Option<String> {
    let path = data_dirs
        .iter()
        .map(|dir| dir.join("applications").join(desktop_id))
        .find(|path| path.is_file())?;
    if std::fs::metadata(&path).ok()?.len() > 256 * 1024 || !crate::parental::entry_allowed(&path) {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let mut inside = false;
    let mut name = None;
    let (mut only, mut not) = (None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            continue;
        }
        match line.split_once('=').filter(|_| inside) {
            Some(("Hidden" | "NoDisplay", value)) if value.trim() == "true" => return None,
            Some(("Name", value)) if name.is_none() => name = Some(clip(value, 80)),
            Some(("OnlyShowIn", value)) => only = Some(value.trim().to_owned()),
            Some(("NotShowIn", value)) => not = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    // Another desktop's own tools (GNOME's Settings) are not offered here,
    // as Search's app list leaves them out.
    let current = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "enzUI".into());
    if !crate::search::catalogue::shown_in(only.as_deref(), not.as_deref(), &current) {
        return None;
    }
    name.filter(|n| !n.is_empty())
}

/// The declaration file a path names, for the probe tool.
pub fn read_declaration(path: &Path) -> Option<Declaration> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text, Protocol::Enzui).or_else(|| parse(&text, Protocol::Gnome))
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
