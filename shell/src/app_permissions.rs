//! Bounded Flatpak inventory and portal summaries; static sandbox edits use the permissions
//! worker.

use enzui_shell::i18n::tr;
use std::collections::HashMap;

const STORE: (&str, &str, &str) = (
    "org.freedesktop.impl.portal.PermissionStore",
    "/org/freedesktop/impl/portal/PermissionStore",
    "org.freedesktop.impl.portal.PermissionStore",
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Notifications,
    Background,
    Camera,
    Location,
    Screenshot,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Notifications,
        Self::Background,
        Self::Camera,
        Self::Location,
        Self::Screenshot,
    ];
    /// The permission store's table and entry.
    #[must_use]
    pub fn entry(self) -> (&'static str, &'static str) {
        match self {
            Self::Notifications => ("notifications", "notification"),
            Self::Background => ("background", "background"),
            Self::Camera => ("devices", "camera"),
            Self::Location => ("location", "location"),
            Self::Screenshot => ("screenshot", "screenshot"),
        }
    }
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Notifications => tr("shell-notifications"),
            Self::Background => tr("shell-run-in-the-background"),
            Self::Camera => tr("shell-camera"),
            Self::Location => tr("shell-location"),
            Self::Screenshot => tr("shell-take-screenshots"),
        }
    }
    fn values(self, allowed: bool) -> Vec<String> {
        match (self, allowed) {
            (Self::Location, true) => vec!["EXACT".into(), "0".into()],
            (Self::Location, false) => vec!["NONE".into(), "0".into()],
            (_, true) => vec!["yes".into()],
            (_, false) => vec!["no".into()],
        }
    }
    fn allowed(self, values: &[String]) -> bool {
        match self {
            Self::Location => values.first().is_some_and(|accuracy| accuracy != "NONE"),
            _ => values.first().is_some_and(|value| value == "yes"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// Recognized existing yes/no grants only; absent and unknown formats are not denials.
    pub grants: Vec<(Kind, bool)>,
    /// What the sandbox itself allows, in words.
    pub sandbox: Vec<&'static str>,
    /// The app's icon file, from its desktop entry or the icon theme.
    pub icon: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    /// `None` before a successful inventory and without Flatpak; failures
    /// are reported separately.
    pub apps: Option<Vec<App>>,
    pub error: Option<String>,
    pub portal_error: Option<String>,
    /// Whether a location provider (GeoClue) can be started on this system.
    pub location_provider: bool,
}

/// The apps whose name or id contains `query`, ignoring case: names that
/// start with it first, then by name.
#[must_use]
pub fn matching<'a>(apps: &'a [App], query: &str) -> Vec<&'a App> {
    let query = query.trim().to_lowercase();
    let mut shown: Vec<&App> = apps
        .iter()
        .filter(|app| {
            app.name.to_lowercase().contains(&query) || app.id.to_lowercase().contains(&query)
        })
        .collect();
    shown.sort_by_cached_key(|app| {
        (
            !query.is_empty() && !app.name.to_lowercase().starts_with(&query),
            app.name.to_lowercase(),
        )
    });
    shown
}

fn valid_app(id: &str) -> bool {
    (3..=255).contains(&id.len())
        && id.contains('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

async fn flatpak(args: &[&str]) -> Result<String, String> {
    flatpak_at(std::path::Path::new("flatpak"), args)
        .await
        .map_err(|error| error.unwrap_or_else(|| tr("shell-flatpak-did-not-start").into()))
}

// Runs `program` (Flatpak); `Err(None)` when it is not installed.
async fn flatpak_at(program: &std::path::Path, args: &[&str]) -> Result<String, Option<String>> {
    use tokio::io::AsyncReadExt;
    let failed = |key: &'static str| Some(tr(key).to_owned());
    let work = async {
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                (error.kind() != std::io::ErrorKind::NotFound)
                    .then(|| tr("shell-flatpak-did-not-start").to_owned())
            })?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| failed("shell-flatpak-output-unavailable"))?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| failed("shell-flatpak-output-failed"))?;
        if bytes.len() > 1024 * 1024 {
            return Err(failed("shell-flatpak-output-exceeds-the-inventory-bound"));
        }
        if !child
            .wait()
            .await
            .map_err(|_| failed("shell-flatpak-stopped-without-a-result"))?
            .success()
        {
            return Err(failed("shell-flatpak-inventory-failed"));
        }
        String::from_utf8(bytes).map_err(|_| failed("shell-invalid-flatpak-inventory"))
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), work)
        .await
        .map_err(|_| failed("shell-flatpak-inventory-took-too-long"))?
}

async fn installed(program: &std::path::Path) -> Result<Vec<(String, String)>, Option<String>> {
    let text = flatpak_at(program, &["list", "--app", "--columns=application,name"]).await?;
    let mut apps: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t').unwrap_or((line, line));
            let id = id.trim();
            valid_app(id).then(|| {
                (
                    id.to_owned(),
                    name.trim()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(64)
                        .collect(),
                )
            })
        })
        .take(500)
        .collect();
    apps.dedup_by(|a, b| a.0 == b.0);
    Ok(apps)
}

/// Static sandbox permissions from `flatpak info --show-permissions` output.
#[must_use]
pub fn sandbox(metadata: &str) -> Vec<&'static str> {
    let mut context = false;
    let mut found = Vec::new();
    let mut push = |what: &'static str| {
        if !found.contains(&what) {
            found.push(what);
        }
    };
    for line in metadata.lines().map(str::trim) {
        if line.starts_with('[') {
            context = line == "[Context]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| context) else {
            continue;
        };
        let items: Vec<&str> = value
            .split(';')
            .map(str::trim)
            .filter(|item| !item.is_empty() && !item.starts_with('!'))
            .collect();
        for item in items {
            match (key.trim(), item) {
                ("shared", "network") => push("Network"),
                ("sockets", "pulseaudio") => push(tr("shell-microphone-and-speakers")),
                ("devices", "all") => push(tr("shell-all-devices-including-cameras")),
                ("filesystems", "host" | "host-os") => push(tr("shell-all-files")),
                ("filesystems", "home") => push(tr("shell-home-folder")),
                ("filesystems", path) if path.starts_with("xdg-run/pipewire") => {
                    push(tr("shell-microphone-and-speakers"));
                }
                ("filesystems", path)
                    if !path.starts_with("xdg-run/")
                        && !path.starts_with("/run/")
                        && path != "host-etc" =>
                {
                    push(tr("shell-some-folders"));
                }
                _ => {}
            }
        }
    }
    found
}

/// The files `flatpak info --show-permissions` merges for `id`: its
/// metadata in each installation and the overrides that apply to it.
fn permission_sources(id: &str) -> Vec<std::path::PathBuf> {
    let system = std::env::var_os("FLATPAK_SYSTEM_DIR")
        .map_or_else(|| "/var/lib/flatpak".into(), std::path::PathBuf::from);
    let user = std::env::var_os("FLATPAK_USER_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| std::path::Path::new(&home).join(".local/share"))
                })
                .map(|data| data.join("flatpak"))
        });
    let mut paths = Vec::new();
    for root in std::iter::once(system).chain(user) {
        paths.push(root.join("app").join(id).join("current/active/metadata"));
        paths.push(root.join("overrides/global"));
        paths.push(root.join("overrides").join(id));
    }
    paths
}

/// What identifies the sources' contents without reading them: each file's
/// inode, size and modification time (an update installs a new metadata
/// file; an override edit changes its time). `None` when no metadata was
/// found, so a layout this does not know is always asked of flatpak.
fn permission_stamp(paths: &[std::path::PathBuf]) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let stamp: Vec<_> = paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|m| (m.ino(), m.len(), m.mtime(), m.mtime_nsec()))
        })
        .collect();
    stamp
        .iter()
        .step_by(3)
        .any(Option::is_some)
        .then_some(stamp)
}

/// Inode, size and modification time of each source; `None` where absent.
type Stamp = Vec<Option<(u64, u64, i64, i64)>>;
type Stamped = (Stamp, Vec<&'static str>);

/// Each app's sandbox as `flatpak info` last gave it, while its sources are
/// unchanged: the Apps page rereads every 30 s, and a flatpak start per app
/// (95 here) cost over a second of CPU each time.
fn permission_cache() -> &'static std::sync::Mutex<HashMap<String, Stamped>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Stamped>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

async fn static_permissions(id: &str) -> Result<Vec<&'static str>, String> {
    let stamp = permission_stamp(&permission_sources(id));
    if let Some(stamp) = &stamp
        && let Ok(cache) = permission_cache().lock()
        && let Some((cached, sandbox)) = cache.get(id)
        && cached == stamp
    {
        return Ok(sandbox.clone());
    }
    let sandbox = flatpak(&["info", "--show-permissions", "--", id])
        .await
        .map(|text| sandbox(&text))?;
    if let Some(stamp) = stamp
        && let Ok(mut cache) = permission_cache().lock()
    {
        cache.insert(id.to_owned(), (stamp, sandbox.clone()));
    }
    Ok(sandbox)
}

async fn store() -> Result<zbus::Proxy<'static>, String> {
    let session = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    zbus::Proxy::new(&session, STORE.0, STORE.1, STORE.2)
        .await
        .map_err(|_| tr("shell-the-permission-store-is-not-available").into())
}

type Entry = HashMap<String, Vec<String>>;

async fn lookup(store: &zbus::Proxy<'_>, kind: Kind) -> Result<Entry, String> {
    let (table, id) = kind.entry();
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        store.call::<_, _, (Entry, zbus::zvariant::OwnedValue)>("Lookup", &(table, id)),
    )
    .await
    {
        Ok(Ok((entry, _))) => Ok(entry),
        Ok(Err(zbus::Error::MethodError(name, _, _)))
            if name.as_str() == "org.freedesktop.portal.Error.NotFound" =>
        {
            Ok(Entry::new())
        }
        _ => Err(tr("shell-remembered-portal-grants-could-not-be-read").into()),
    }
}

fn location_provider() -> bool {
    [
        "/usr/lib/geoclue-2.0/demos",
        "/usr/libexec/geoclue",
        "/usr/lib/geoclue",
    ]
    .iter()
    .any(|path| std::path::Path::new(path).exists())
        || std::path::Path::new(
            "/usr/share/dbus-1/system-services/org.freedesktop.GeoClue2.service",
        )
        .exists()
}

pub async fn read() -> Permissions {
    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        read_now(std::path::Path::new("flatpak")),
    )
    .await
    {
        Ok(permissions) => permissions,
        Err(_) => Permissions {
            apps: Some(Vec::new()),
            error: Some(tr("shell-app-inventory-took-too-long").into()),
            portal_error: None,
            location_provider: location_provider(),
        },
    }
}

async fn read_now(flatpak: &std::path::Path) -> Permissions {
    let location_provider = location_provider();
    let installed = match installed(flatpak).await {
        Ok(apps) => apps,
        // Without Flatpak the page says so instead of reporting an error.
        Err(None) => {
            return Permissions {
                apps: None,
                error: None,
                portal_error: None,
                location_provider,
            };
        }
        Err(Some(error)) => {
            return Permissions {
                apps: Some(Vec::new()),
                error: Some(error),
                portal_error: None,
                location_provider,
            };
        }
    };
    let mut portal_error = None;
    let mut entries = HashMap::new();
    match store().await {
        Ok(store) => {
            for kind in Kind::ALL {
                match lookup(&store, kind).await {
                    Ok(entry) => {
                        entries.insert(kind, entry);
                    }
                    Err(error) => portal_error = Some(error),
                }
            }
        }
        Err(error) => portal_error = Some(error),
    }
    let mut sandboxes = HashMap::new();
    let read: HashMap<String, Result<Vec<&'static str>, String>> = {
        use iced::futures::{StreamExt, stream};
        let ids: Vec<String> = installed.iter().map(|(id, _)| id.clone()).collect();
        stream::iter(ids.into_iter().map(|id| async move {
            let sandbox = static_permissions(&id).await;
            (id, sandbox)
        }))
        .buffer_unordered(4)
        .collect()
        .await
    };
    for (id, _) in &installed {
        let sandbox = match read.get(id).cloned().unwrap_or_else(|| Ok(Vec::new())) {
            Ok(sandbox) => sandbox,
            Err(error) => {
                return Permissions {
                    apps: Some(Vec::new()),
                    error: Some(error),
                    portal_error: None,
                    location_provider,
                };
            }
        };
        sandboxes.insert(
            id.clone(),
            (sandbox, crate::search::catalogue::icon_for_app_id(id)),
        );
    }
    let apps = installed
        .into_iter()
        .map(|(id, name)| {
            let grants = Kind::ALL
                .into_iter()
                .filter_map(
                    |kind| match entries.get(&kind).and_then(|entry| entry.get(&id)) {
                        Some(values)
                            if kind != Kind::Location
                                && (values == &["yes"] || values == &["no"]) =>
                        {
                            Some((kind, kind.allowed(values)))
                        }
                        _ => None,
                    },
                )
                .collect();
            let (sandbox, icon) = sandboxes.remove(&id).unwrap_or_default();
            App {
                sandbox,
                icon,
                id,
                name,
                grants,
            }
        })
        .collect();
    Permissions {
        apps: Some(apps),
        error: None,
        portal_error,
        location_provider,
    }
}

/// Edits a recognized existing yes/no grant; never establishes a first answer.
pub async fn set(app: &str, kind: Kind, allowed: bool) -> Result<(), String> {
    if !valid_app(app) {
        return Err(tr("shell-unknown-app").into());
    }
    let (table, id) = kind.entry();
    let store = store().await?;
    let entry = lookup(&store, kind).await?;
    let values = entry
        .get(app)
        .ok_or_else(|| tr("shell-this-app-has-not-requested-that-permission"))?;
    if kind == Kind::Location || !(values == &["yes"] || values == &["no"]) {
        return Err(tr("shell-this-portal-format-needs-its-own-review").into());
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        store.call::<_, _, ()>(
            "SetPermission",
            &(table, false, id, app, kind.values(allowed)),
        ),
    )
    .await
    .map_err(|_| tr("shell-the-permission-change-outcome-is-unknown"))?
    .map_err(|_| tr("shell-the-permission-could-not-be-changed").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without Flatpak the page says Flatpak is not installed; a Flatpak
    /// that fails is still reported as an error.
    ///
    /// Sabotage: map a missing program to `Some(..)` in `flatpak_at`.
    #[tokio::test]
    async fn app_permissions_behaviour() {
        // App permissions behaviour.
        {
            // Grants and sandbox permissions read as they are.
            {
                assert!(
                    Kind::Location.allowed(&["EXACT".into(), "0".into()])
                        && !Kind::Location.allowed(&["NONE".into(), "0".into()])
                );
                assert!(
                    Kind::Camera.allowed(&["yes".into()])
                        && !Kind::Background.allowed(&["no".into()])
                        && !Kind::Screenshot.allowed(&[])
                );
                let metadata = "[Context]\nshared=network;ipc;\nsockets=x11;wayland;pulseaudio;\ndevices=dri;\nfilesystems=xdg-run/gvfsd;home;!host;xdg-documents;\n\n[Session Bus Policy]\norg.gtk.vfs.*=talk\n";
                assert_eq!(
                    sandbox(metadata),
                    [
                        "Network",
                        "Microphone and speakers",
                        "Home folder",
                        "Some folders"
                    ]
                );
                assert!(
                    valid_app("org.gnome.TextEditor")
                        && !valid_app("x;rm")
                        && !valid_app("noperiod")
                );
            }
            // Apps match by name or id with name prefixes first.
            {
                let app = |id: &str, name: &str| App {
                    id: id.into(),
                    name: name.into(),
                    grants: Vec::new(),
                    sandbox: Vec::new(),
                    icon: None,
                };
                let apps = [
                    app("org.gnome.Maps", "Maps"),
                    app("io.example.Zap", "Zap"),
                    app("org.mozilla.firefox", "Firefox"),
                    app("org.example.Amazing", "Amazing"),
                ];
                let names = |query: &str| {
                    matching(&apps, query)
                        .iter()
                        .map(|app| app.name.as_str())
                        .collect::<Vec<_>>()
                };
                assert_eq!(names(""), ["Amazing", "Firefox", "Maps", "Zap"]);
                assert_eq!(names(" MA "), ["Maps", "Amazing"]);
                assert_eq!(names("mozilla"), ["Firefox"]);
                assert!(names("nothing").is_empty());
            }
            // Sabotage: return a stamp when no metadata was found. A layout this
            // does not know would then be cached instead of asked of flatpak.
            {
                let root = tempfile::tempdir().unwrap();
                let metadata = root.path().join("app/org.Test/current/active/metadata");
                let overrides = root.path().join("overrides/org.Test");
                let paths = vec![
                    metadata.clone(),
                    root.path().join("overrides/global"),
                    overrides.clone(),
                ];
                assert_eq!(permission_stamp(&paths), None);
                std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
                std::fs::write(&metadata, "[Context]\nshared=network;\n").unwrap();
                let before = permission_stamp(&paths).expect("metadata found");
                std::fs::create_dir_all(overrides.parent().unwrap()).unwrap();
                std::fs::write(&overrides, "[Context]\nshared=!network;\n").unwrap();
                assert_ne!(permission_stamp(&paths), Some(before));
            }
        }

        let root = tempfile::tempdir().unwrap();
        let missing = read_now(&root.path().join("flatpak")).await;
        assert_eq!((missing.apps, missing.error), (None, None));
        let broken = root.path().join("broken");
        std::fs::write(&broken, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(
            &broken,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        let failed = read_now(&broken).await;
        assert_eq!(failed.apps, Some(Vec::new()));
        assert!(failed.error.is_some());
    }
}
