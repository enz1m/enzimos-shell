//! XDG Desktop Portal backend for file selection, access consent, background apps and
//! appearance.
//!
//! The frontend grants document access. This backend writes sandbox autostart entries
//! and publishes color scheme, text scale and clock preferences from desktop settings.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.enzui";
const PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open,
    Save,
    SaveFiles,
}

/// One pattern of a filter: a shell glob such as `*.png`, or a MIME type
/// such as `image/png` or `image/*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    /// Whether a file of this name (and MIME type) passes the filter.
    pub fn accepts(&self, name: &str, mime: Option<&str>) -> bool {
        self.patterns.iter().any(|pattern| match pattern {
            Pattern::Glob(glob) => glob_matches(&glob.to_lowercase(), &name.to_lowercase()),
            Pattern::Mime(wanted) => mime.is_some_and(|mime| match wanted.strip_suffix("/*") {
                Some(family) => mime.split('/').next() == Some(family),
                None => is_a(mime, wanted, subclasses()),
            }),
        })
    }
}

// shared-mime-info's parent types (`mime/subclasses` in every data folder):
// a filter for `text/plain` also offers `text/x-todo-txt`.
fn subclasses() -> &'static HashMap<String, Vec<String>> {
    static PARENTS: std::sync::OnceLock<HashMap<String, Vec<String>>> = std::sync::OnceLock::new();
    PARENTS.get_or_init(|| {
        let dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut parents: HashMap<String, Vec<String>> = HashMap::new();
        for dir in std::env::split_paths(&dirs)
            .filter(|d| d.is_absolute())
            .take(32)
        {
            let Ok(text) = std::fs::read_to_string(dir.join("mime/subclasses")) else {
                continue;
            };
            for line in text.lines().take(10_000) {
                if let Some((child, parent)) = line.split_once(' ') {
                    parents
                        .entry(child.to_owned())
                        .or_default()
                        .push(parent.to_owned());
                }
            }
        }
        parents
    })
}

fn is_a(mime: &str, wanted: &str, parents: &HashMap<String, Vec<String>>) -> bool {
    let mut seen = vec![mime.to_owned()];
    let mut at = 0;
    while at < seen.len() && seen.len() < 64 {
        let current = seen[at].clone();
        at += 1;
        // Every text type is also plain text (shared-mime-info spec).
        if current == wanted || (wanted == "text/plain" && current.starts_with("text/")) {
            return true;
        }
        for parent in parents.get(&current).into_iter().flatten() {
            if !seen.contains(parent) {
                seen.push(parent.clone());
            }
        }
    }
    false
}

// `*` and `?` shell globbing, the only syntax portal filters use.
fn glob_matches(glob: &str, name: &str) -> bool {
    let (glob, name): (Vec<char>, Vec<char>) = (glob.chars().collect(), name.chars().collect());
    let (mut g, mut n, mut star, mut mark) = (0, 0, None, 0);
    while n < name.len() {
        if g < glob.len() && (glob[g] == '?' || glob[g] == name[n]) {
            g += 1;
            n += 1;
        } else if g < glob.len() && glob[g] == '*' {
            star = Some(g);
            mark = n;
            g += 1;
        } else if let Some(at) = star {
            g = at + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    glob[g..].iter().all(|c| *c == '*')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub kind: Kind,
    pub app_id: String,
    pub title: String,
    pub accept_label: Option<String>,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub current_name: Option<String>,
    pub current_folder: Option<PathBuf>,
    pub current_file: Option<PathBuf>,
    /// SaveFiles: the names to save.
    pub files: Vec<String>,
    /// Choices the app offers (id, default value), returned unchanged.
    pub choices: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Chosen {
        paths: Vec<PathBuf>,
        filter: Option<usize>,
    },
    Cancelled,
}

/// The frontend asking whether an app may do something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRequest {
    pub id: u64,
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub grant_label: String,
    pub deny_label: String,
}

#[derive(Debug)]
pub enum Event {
    Begin(Request, oneshot::Sender<Outcome>),
    Access(AccessRequest, oneshot::Sender<bool>),
    Close(u64),
    Appearance { light: bool },
}

fn text(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .filter(|s| !s.is_empty())
}
fn flag(options: &HashMap<String, OwnedValue>, key: &str) -> bool {
    options
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}
// A path sent as a NUL-terminated byte string.
fn bytes_path(value: &OwnedValue) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = Vec::<u8>::try_from(value.try_clone().ok()?).ok()?;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
    path.is_absolute().then_some(path)
}
fn filter_of(value: (String, Vec<(u32, String)>)) -> Filter {
    Filter {
        name: value.0,
        patterns: value
            .1
            .into_iter()
            .filter_map(|(kind, pattern)| match kind {
                0 => Some(Pattern::Glob(pattern)),
                1 => Some(Pattern::Mime(pattern)),
                _ => None,
            })
            .collect(),
    }
}

impl Request {
    pub fn parse(
        id: u64,
        kind: Kind,
        app_id: &str,
        title: &str,
        options: &HashMap<String, OwnedValue>,
    ) -> Self {
        let filters: Vec<Filter> = options
            .get("filters")
            .and_then(|v| Vec::<(String, Vec<(u32, String)>)>::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
            .into_iter()
            .map(filter_of)
            .collect();
        let current_filter = options
            .get("current_filter")
            .and_then(|v| <(String, Vec<(u32, String)>)>::try_from(v.try_clone().ok()?).ok())
            .map(filter_of)
            .and_then(|current| filters.iter().position(|f| *f == current));
        let files = options
            .get("files")
            .and_then(|v| Vec::<Vec<u8>>::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|mut bytes| {
                while bytes.last() == Some(&0) {
                    bytes.pop();
                }
                let name = String::from_utf8(bytes).ok()?;
                Path::new(&name)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .collect();
        let choices = options
            .get("choices")
            .and_then(|v| {
                Vec::<(String, String, Vec<(String, String)>, String)>::try_from(
                    v.try_clone().ok()?,
                )
                .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _, _, default)| (id, default))
            .collect();
        Self {
            id,
            kind,
            app_id: app_id.chars().take(255).collect(),
            title: title.chars().take(200).collect(),
            accept_label: text(options, "accept_label").map(|l| l.replace('_', "")),
            multiple: kind == Kind::Open && flag(options, "multiple"),
            directory: kind == Kind::Open && flag(options, "directory"),
            filters,
            current_filter,
            current_name: text(options, "current_name").map(|n| n.replace('/', "")),
            current_folder: options.get("current_folder").and_then(bytes_path),
            current_file: options.get("current_file").and_then(bytes_path),
            files,
            choices,
        }
    }
}

/// `file://` URIs as the portal returns them, with reserved bytes escaped.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

struct Chooser {
    events: mpsc::UnboundedSender<Event>,
    next: Mutex<u64>,
}

struct PortalRequest {
    id: u64,
    events: mpsc::UnboundedSender<Event>,
    cancel: Option<std::sync::Arc<tokio::sync::Notify>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl PortalRequest {
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn close(&self) {
        let _ = self.events.send(Event::Close(self.id));
        if let Some(cancel) = &self.cancel {
            cancel.notify_one();
        }
    }
}

impl Chooser {
    fn next_id(&self) -> u64 {
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next += 1;
        *next
    }

    async fn ask(
        &self,
        server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        kind: Kind,
        app_id: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let id = self.next_id();
        let request = Request::parse(id, kind, app_id, title, &options);
        let handle: OwnedObjectPath = handle.into();
        let _ = server
            .at(
                &handle,
                PortalRequest {
                    id,
                    events: self.events.clone(),
                    cancel: None,
                },
            )
            .await;
        let (reply, answer) = oneshot::channel();
        let outcome = if self
            .events
            .send(Event::Begin(request.clone(), reply))
            .is_ok()
        {
            answer.await.unwrap_or(Outcome::Cancelled)
        } else {
            Outcome::Cancelled
        };
        let _ = server.remove::<PortalRequest, _>(&handle).await;
        match outcome {
            Outcome::Cancelled => (1, HashMap::new()),
            Outcome::Chosen { paths, filter } => {
                let mut results = HashMap::new();
                let uris: Vec<String> = paths.iter().map(|p| uri(p)).collect();
                results.insert("uris".to_owned(), owned(Value::from(uris)));
                if let Some(filter) = filter.and_then(|i| request.filters.get(i)) {
                    let patterns: Vec<(u32, String)> = filter
                        .patterns
                        .iter()
                        .map(|p| match p {
                            Pattern::Glob(g) => (0, g.clone()),
                            Pattern::Mime(m) => (1, m.clone()),
                        })
                        .collect();
                    results.insert(
                        "current_filter".to_owned(),
                        owned(Value::from((filter.name.clone(), patterns))),
                    );
                }
                if !request.choices.is_empty() {
                    results.insert(
                        "choices".to_owned(),
                        owned(Value::from(request.choices.clone())),
                    );
                }
                if kind != Kind::Open {
                    results.insert("writable".to_owned(), owned(Value::from(true)));
                }
                (0, results)
            }
        }
    }
}

fn owned(value: Value<'_>) -> OwnedValue {
    value
        .try_to_owned()
        .unwrap_or_else(|_| OwnedValue::from(false))
}

struct Files(std::sync::Arc<Chooser>);

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl Files {
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.0
            .ask(server, handle, Kind::Open, app_id, title, options)
            .await
    }
    async fn save_file(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.0
            .ask(server, handle, Kind::Save, app_id, title, options)
            .await
    }
    async fn save_files(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        self.0
            .ask(server, handle, Kind::SaveFiles, app_id, title, options)
            .await
    }
    #[zbus(property)]
    fn version(&self) -> u32 {
        4
    }
}

struct Access {
    chooser: std::sync::Arc<Chooser>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl Access {
    #[allow(
        clippy::too_many_arguments,
        reason = "The view composes independent state and actions supplied by its caller."
    )]
    async fn access_dialog(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        _parent_window: &str,
        title: &str,
        subtitle: &str,
        body: &str,
        options: HashMap<String, OwnedValue>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let chooser = &self.chooser;
        let id = chooser.next_id();
        let bounded = |text: &str, limit: usize| text.chars().take(limit).collect::<String>();
        let request = AccessRequest {
            id,
            app_id: bounded(app_id, 255),
            title: bounded(title, 200),
            subtitle: bounded(subtitle, 400),
            body: bounded(body, 1000),
            grant_label: text(&options, "grant_label").map_or_else(
                || tr("shell-allow").into(),
                |l| bounded(&l.replace('_', ""), 40),
            ),
            deny_label: text(&options, "deny_label").map_or_else(
                || tr("shell-deny").into(),
                |l| bounded(&l.replace('_', ""), 40),
            ),
        };
        let handle: OwnedObjectPath = handle.into();
        let _ = server
            .at(
                &handle,
                PortalRequest {
                    id,
                    events: chooser.events.clone(),
                    cancel: None,
                },
            )
            .await;
        let (reply, answer) = oneshot::channel();
        let granted = chooser.events.send(Event::Access(request, reply)).is_ok()
            && answer.await.unwrap_or(false);
        let _ = server.remove::<PortalRequest, _>(&handle).await;
        // Anything but an explicit grant is a refusal.
        (if granted { 0 } else { 1 }, HashMap::new())
    }
}

/// Whether `id` is a Flatpak application ID: two or more dot-separated
/// elements of ASCII letters, digits, `_` and `-`, not starting with a digit.
#[must_use]
pub fn valid_app_id(id: &str) -> bool {
    let elements: Vec<&str> = id.split('.').collect();
    id.len() <= 255
        && elements.len() >= 2
        && elements.iter().all(|e| {
            !e.is_empty()
                && !e.starts_with(|c: char| c.is_ascii_digit())
                && e.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

// One `Exec` argument: quoted where the Desktop Entry spec reserves a
// character, then escaped once more for the string value it sits in.
fn exec_argument(arg: &str) -> Option<String> {
    if arg.is_empty() || arg.len() > 4096 || arg.chars().any(char::is_control) {
        return None;
    }
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-./=:,+@".contains(c))
    {
        return Some(arg.to_owned());
    }
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str("\\\\\\\\"),
            '%' => quoted.push_str("%%"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    Some(quoted)
}

/// The autostart entry that starts Flatpak app `id` with `commandline`, or
/// `None` for an invalid ID or argument.
#[must_use]
pub fn autostart_entry(id: &str, commandline: &[String]) -> Option<String> {
    if !valid_app_id(id) || commandline.is_empty() || commandline.len() > 64 {
        return None;
    }
    let command = exec_argument(&commandline[0])?;
    let args: Vec<String> = commandline[1..]
        .iter()
        .map(|arg| exec_argument(arg))
        .collect::<Option<_>>()?;
    let exec = std::iter::once(format!("flatpak run --command={command} {id}"))
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ");
    // Asked for in enzUI, so it starts in enzUI; another desktop's portal
    // writes its own entry when the app asks there.
    Some(format!(
        "[Desktop Entry]\nType=Application\nName={id}\nExec={exec}\nX-Flatpak={id}\nOnlyShowIn=enzUI;\n"
    ))
}

fn autostart_path(id: &str) -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
    Some(config.join("autostart").join(format!("{id}.desktop")))
}

fn set_autostart(id: &str, enable: bool, commandline: &[String]) -> bool {
    let Some(path) = autostart_path(id).filter(|_| valid_app_id(id)) else {
        return false;
    };
    if !enable {
        return match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
    }
    let Some(entry) = autostart_entry(id, commandline) else {
        return false;
    };
    let Some(directory) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(directory).is_err() {
        return false;
    }
    let staged = directory.join(format!(".{id}.desktop.new"));
    std::fs::write(&staged, entry).is_ok() && std::fs::rename(&staged, &path).is_ok()
}

type WindowFeedRow = (String, String, u32, u64, u32, u32);

async fn mapped_apps(connection: &zbus::Connection) -> Option<Vec<String>> {
    let (_, known, apps) = connection
        .call_method(
            Some(crate::window_feed::SERVICE),
            crate::window_feed::PATH,
            Some("dev.enzui.Shell.Windows1"),
            "GetApps",
            &(),
        )
        .await
        .ok()?
        .body()
        .deserialize::<(u64, bool, Vec<WindowFeedRow>)>()
        .ok()?;
    known.then(|| {
        apps.into_iter()
            .filter_map(|(app, _, _, _, mapped, _)| {
                (mapped > 0 && valid_app_id(&app)).then_some(app)
            })
            .collect()
    })
}

async fn app_has_mapped_window(connection: &zbus::Connection, app_id: &str) -> bool {
    valid_app_id(app_id)
        && mapped_apps(connection)
            .await
            .is_some_and(|apps| apps.iter().any(|app| app == app_id))
}

// The shell's window feed as app id -> state (1 running); apps it does not
// list have no window, which the frontend counts as background.
async fn app_states(connection: &zbus::Connection) -> HashMap<String, OwnedValue> {
    let mut states = HashMap::new();
    if let Some(apps) = mapped_apps(connection).await {
        for app in apps {
            states.insert(app, OwnedValue::from(1u32));
        }
    }
    states
}

struct Background {
    chooser: std::sync::Arc<Chooser>,
    windows_changed: tokio::sync::watch::Receiver<u64>,
}

fn background_result<E>(choice: Result<bool, E>) -> u32 {
    match choice {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => 2,
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Background")]
impl Background {
    async fn get_app_state(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> HashMap<String, OwnedValue> {
        app_states(connection).await
    }

    // Background response codes: 1 allows, 0 closes the app, and 2 allows only this request.
    async fn notify_background(
        &self,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        app_id: &str,
        name: &str,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // Snapshot the change counter before querying so an intervening window map is not missed.
        let mut windows_changed = self.windows_changed.clone();
        windows_changed.borrow_and_update();
        if app_has_mapped_window(connection, app_id).await {
            return (
                0,
                HashMap::from([("result".to_owned(), OwnedValue::from(2u32))]),
            );
        }

        let chooser = &self.chooser;
        let id = chooser.next_id();
        let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
        let shown = if name.trim().is_empty() {
            app_id.chars().take(80).collect()
        } else {
            name
        };
        let request = AccessRequest {
            id,
            app_id: app_id.chars().take(255).collect(),
            title: tr!("shell-is-running-in-the-background", "arg0" => shown.clone()),
            subtitle: tr("shell-it-has-no-open-windows").into(),
            body: tr("shell-allow-it-to-keep-running-if-you-close-it-any-work-it-is-doing-stops")
                .into(),
            grant_label: tr("shell-allow").into(),
            deny_label: tr("shell-close-it").into(),
        };
        let handle: OwnedObjectPath = handle.into();
        let cancel = std::sync::Arc::new(tokio::sync::Notify::new());
        let _ = server
            .at(
                &handle,
                PortalRequest {
                    id,
                    events: chooser.events.clone(),
                    cancel: Some(std::sync::Arc::clone(&cancel)),
                },
            )
            .await;
        let (reply, mut answer) = oneshot::channel();
        let result = if chooser.events.send(Event::Access(request, reply)).is_ok() {
            loop {
                tokio::select! {
                    biased;
                    choice = &mut answer => {
                        break background_result(choice);
                    }
                    () = cancel.notified() => {
                        break background_result(answer.try_recv());
                    }
                    changed = windows_changed.changed() => {
                        if changed.is_err() {
                            break tokio::select! {
                                biased;
                                choice = &mut answer => background_result(choice),
                                () = cancel.notified() => background_result(answer.try_recv()),
                            };
                        }
                        if app_has_mapped_window(connection, app_id).await {
                            break match answer.try_recv() {
                                Ok(true) => 1u32,
                                Ok(false) => 0,
                                Err(_) => {
                                    let _ = chooser.events.send(Event::Close(id));
                                    2
                                }
                            };
                        }
                    }
                }
            }
        } else {
            2
        };
        let _ = server.remove::<PortalRequest, _>(&handle).await;
        (
            0,
            HashMap::from([("result".to_owned(), OwnedValue::from(result))]),
        )
    }

    async fn enable_autostart(
        &self,
        app_id: &str,
        enable: bool,
        commandline: Vec<String>,
        _flags: u32,
    ) -> bool {
        let app_id = app_id.to_owned();
        tokio::task::spawn_blocking(move || set_autostart(&app_id, enable, &commandline))
            .await
            .unwrap_or(false)
    }

    #[zbus(signal)]
    async fn running_applications_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

#[path = "desktop_portal_settings.rs"]
mod settings_portal;
use settings_portal::{Appearance, Preferences};
pub use settings_portal::{color_scheme, selects};

async fn serve_on(
    builder: zbus::connection::Builder<'_>,
    events: mpsc::UnboundedSender<Event>,
) -> Result<zbus::Connection, String> {
    let chooser = std::sync::Arc::new(Chooser {
        events,
        next: Mutex::new(0),
    });
    let (windows_tx, windows_rx) = tokio::sync::watch::channel(0u64);
    let connection = builder
        .serve_at(PATH, Files(std::sync::Arc::clone(&chooser)))
        .map_err(|_| tr("shell-could-not-publish-the-file-chooser"))?
        .serve_at(
            PATH,
            Access {
                chooser: std::sync::Arc::clone(&chooser),
            },
        )
        .map_err(|_| tr("shell-could-not-publish-the-access-dialog"))?
        .serve_at(
            PATH,
            Background {
                chooser,
                windows_changed: windows_rx,
            },
        )
        .map_err(|_| tr("shell-could-not-publish-the-background-portal"))?
        .serve_at(
            PATH,
            Appearance {
                preferences: Mutex::new(Preferences::load()),
            },
        )
        .map_err(|_| tr("shell-could-not-publish-the-appearance-settings"))?
        .name(BUS_NAME)
        .map_err(|_| tr("shell-invalid-portal-name"))?
        .build()
        .await
        .map_err(|_| tr("shell-another-file-chooser-backend-already-runs"))?;
    // Install the window-feed subscription before returning so no change signal is lost.
    if let Ok(background) = connection
        .object_server()
        .interface::<_, Background>(PATH)
        .await
        && let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("dev.enzui.Shell.Windows1")
            .and_then(|rule| rule.member("Changed"))
        && let Ok(mut changes) =
            zbus::MessageStream::for_match_rule(rule.build(), &connection, Some(16)).await
    {
        tokio::spawn(async move {
            use iced::futures::StreamExt;
            while changes.next().await.is_some() {
                windows_tx.send_modify(|revision| *revision = revision.wrapping_add(1));
                let _ = Background::running_applications_changed(background.signal_emitter()).await;
            }
        });
    }
    Ok(connection)
}

/// Serves the backend on the session bus until the connection drops.
pub async fn run(events: mpsc::UnboundedSender<Event>) -> Result<(), String> {
    let builder = zbus::connection::Builder::session().map_err(|_| tr("shell-no-session-bus"))?;
    let connection = serve_on(builder, events.clone()).await?;
    settings_portal::follow(&connection, events).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_portal_behaviour() {
        // Options are parsed bounded and scoped to their request kind.
        {
            let filter = |name: &str, glob: &str| (name.to_owned(), vec![(0u32, glob.to_owned())]);
            let mut options: HashMap<String, OwnedValue> = HashMap::new();
            options.insert(
                "filters".into(),
                owned(Value::from(vec![
                    filter("Text", "*.txt"),
                    filter("All", "*"),
                ])),
            );
            options.insert(
                "current_filter".into(),
                owned(Value::from(filter("All", "*"))),
            );
            options.insert("multiple".into(), owned(Value::from(true)));
            options.insert("accept_label".into(), owned(Value::from("_Import")));
            options.insert(
                "current_name".into(),
                owned(Value::from("../evil/name.txt")),
            );
            options.insert(
                "current_folder".into(),
                owned(Value::from(b"/home/ann/Documents\0".to_vec())),
            );
            let open = Request::parse(
                1,
                Kind::Open,
                "org.example.App",
                "Open a document",
                &options,
            );
            assert_eq!(open.filters.len(), 2);
            assert_eq!(open.current_filter, Some(1));
            assert!(open.multiple);
            assert_eq!(open.accept_label.as_deref(), Some("Import"));
            assert_eq!(
                open.current_folder,
                Some(PathBuf::from("/home/ann/Documents"))
            );
            let save = Request::parse(2, Kind::Save, "org.example.App", "Save", &options);
            assert!(!save.multiple, "only an open request selects several files");
            assert_eq!(save.current_name.as_deref(), Some("..evilname.txt"));
            options.insert(
                "current_folder".into(),
                owned(Value::from(b"relative\0".to_vec())),
            );
            assert_eq!(
                Request::parse(3, Kind::Open, "", "", &options).current_folder,
                None
            );
        }
        // Autostart entries quote arguments and refuse bad ids.
        {
            assert!(
                valid_app_id("org.example.App")
                    && !valid_app_id("App")
                    && !valid_app_id("org.1bad.App")
                    && !valid_app_id("org.ex ample.App")
            );
            let entry = autostart_entry(
                "org.example.App",
                &[
                    "app".into(),
                    "--background".into(),
                    "a b$c".into(),
                    "50%".into(),
                ],
            )
            .unwrap();
            assert!(entry.contains("Exec=flatpak run --command=app org.example.App --background \"a b\\\\$c\" \"50%%\"\n"), "{entry}");
            assert!(entry.contains("X-Flatpak=org.example.App\n"));
            // Sabotage: drop OnlyShowIn from autostart_entry.
            assert!(entry.contains("OnlyShowIn=enzUI;\n"));
            assert!(autostart_entry("org.example.App", &["app\nX-Evil=1".into()]).is_none());
            assert!(autostart_entry("bad", &["app".into()]).is_none());
            assert!(autostart_entry("org.example.App", &[]).is_none());
        }
    }

    struct PrivateBus {
        address: String,
        child: std::process::Child,
    }

    impl PrivateBus {
        fn start() -> Self {
            use std::io::{BufRead, BufReader};
            use std::process::{Command, Stdio};
            let mut child = Command::new("dbus-daemon")
                .args([
                    "--session",
                    "--nopidfile",
                    "--nosyslog",
                    "--print-address=1",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("dbus-daemon should start for portal tests");
            let mut address = String::new();
            BufReader::new(child.stdout.take().expect("dbus-daemon stdout"))
                .read_line(&mut address)
                .expect("dbus-daemon should print its address");
            Self {
                address: address.trim().to_owned(),
                child,
            }
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[derive(Default)]
    struct MockFeedState {
        generation: u64,
        known: bool,
        apps: Vec<WindowFeedRow>,
    }

    struct MockWindowsFeed(std::sync::Arc<Mutex<MockFeedState>>);

    #[zbus::interface(name = "dev.enzui.Shell.Windows1")]
    impl MockWindowsFeed {
        fn get_apps(&self) -> (u64, bool, Vec<WindowFeedRow>) {
            let state = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (state.generation, state.known, state.apps.clone())
        }

        #[zbus(signal)]
        async fn changed(
            emitter: &zbus::object_server::SignalEmitter<'_>,
            generation: u64,
        ) -> zbus::Result<()>;
    }

    fn calendar_row(app_id: &str, mapped: u32) -> WindowFeedRow {
        (
            app_id.to_owned(),
            format!("app-flatpak-{app_id}-1.scope"),
            414_151,
            13_345_381,
            mapped,
            0,
        )
    }

    fn notify_result(reply: &(u32, HashMap<String, OwnedValue>)) -> u32 {
        assert_eq!(reply.0, 0);
        u32::try_from(reply.1.get("result").expect("result key")).expect("result u32")
    }

    #[tokio::test]
    async fn background_prompt_respects_live_windows_and_close_lifecycle() {
        use std::time::Duration;

        let bus = PrivateBus::start();
        let feed_state = std::sync::Arc::new(Mutex::new(MockFeedState {
            generation: 1,
            known: true,
            apps: vec![calendar_row("io.github.enzui.Calendar", 1)],
        }));
        let feed_conn = zbus::connection::Builder::address(bus.address.as_str())
            .expect("feed address")
            .serve_at(
                crate::window_feed::PATH,
                MockWindowsFeed(std::sync::Arc::clone(&feed_state)),
            )
            .expect("serve mock feed")
            .name(crate::window_feed::SERVICE)
            .expect("feed bus name")
            .build()
            .await
            .expect("connect mock feed");
        let feed_iface = feed_conn
            .object_server()
            .interface::<_, MockWindowsFeed>(crate::window_feed::PATH)
            .await
            .expect("feed interface");

        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let portal_builder =
            zbus::connection::Builder::address(bus.address.as_str()).expect("portal address");
        let _portal_conn = serve_on(portal_builder, events_tx)
            .await
            .expect("serve portal");

        let client = zbus::connection::Builder::address(bus.address.as_str())
            .expect("client address")
            .build()
            .await
            .expect("connect client");

        let call_notify = |client: zbus::Connection, handle: &'static str, app_id: &'static str| {
            tokio::spawn(async move {
                let path = ObjectPath::try_from(handle).expect("handle path");
                client
                    .call_method(
                        Some(BUS_NAME),
                        PATH,
                        Some("org.freedesktop.impl.portal.Background"),
                        "NotifyBackground",
                        &(path, app_id, "Calendar"),
                    )
                    .await
                    .expect("NotifyBackground call")
                    .body()
                    .deserialize::<(u32, HashMap<String, OwnedValue>)>()
                    .expect("NotifyBackground reply")
            })
        };

        let reply = tokio::time::timeout(
            Duration::from_secs(5),
            call_notify(
                client.clone(),
                "/org/freedesktop/portal/desktop/notify/background1",
                "io.github.enzui.Calendar",
            ),
        )
        .await
        .expect("stale prompt timeout")
        .expect("stale prompt join");
        assert_eq!(notify_result(&reply), 2);
        assert!(events_rx.try_recv().is_err());

        {
            let mut state = feed_state.lock().unwrap();
            state.generation = 2;
            state.known = true;
            state.apps.clear();
        }
        let mut pending = call_notify(
            client.clone(),
            "/org/freedesktop/portal/desktop/notify/background2",
            "io.github.enzui.Calendar",
        );
        let (request_id, _kept_reply) =
            match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
                .await
                .expect("access event timeout")
                .expect("access event")
            {
                Event::Access(req, reply) => {
                    assert_eq!(req.app_id, "io.github.enzui.Calendar");
                    (req.id, reply)
                }
                other => panic!("unexpected event: {other:?}"),
            };

        {
            let mut state = feed_state.lock().unwrap();
            state.generation = 3;
            state.apps = vec![calendar_row("io.github.enzui.Notes", 1)];
        }
        MockWindowsFeed::changed(feed_iface.signal_emitter(), 3)
            .await
            .expect("emit changed 3");
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut pending)
                .await
                .is_err(),
            "unrelated app mapping must not dismiss Calendar prompt"
        );
        assert!(events_rx.try_recv().is_err());

        {
            let mut state = feed_state.lock().unwrap();
            state.generation = 4;
            state.apps = vec![
                calendar_row("io.github.enzui.Notes", 1),
                calendar_row("io.github.enzui.Calendar", 1),
            ];
        }
        MockWindowsFeed::changed(feed_iface.signal_emitter(), 4)
            .await
            .expect("emit changed 4");
        let reply = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("auto-dismiss timeout")
            .expect("auto-dismiss join");
        assert_eq!(notify_result(&reply), 2);
        match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("close event timeout")
            .expect("close event")
        {
            Event::Close(closed_id) => assert_eq!(closed_id, request_id),
            other => panic!("expected Close({request_id}), got {other:?}"),
        }

        {
            let mut state = feed_state.lock().unwrap();
            state.generation = 5;
            state.known = false;
            state.apps = vec![calendar_row("io.github.enzui.Calendar", 1)];
        }
        let pending = call_notify(
            client.clone(),
            "/org/freedesktop/portal/desktop/notify/background3",
            "io.github.enzui.Calendar",
        );
        match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("unknown inventory event timeout")
            .expect("unknown inventory event")
        {
            Event::Access(_, reply) => reply.send(true).expect("send allow"),
            other => panic!("unexpected event: {other:?}"),
        }
        let reply = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("allow timeout")
            .expect("allow join");
        assert_eq!(notify_result(&reply), 1);

        {
            let mut state = feed_state.lock().unwrap();
            state.generation = 6;
            state.known = true;
            state.apps.clear();
        }
        let pending = call_notify(
            client.clone(),
            "/org/freedesktop/portal/desktop/notify/background4",
            "io.github.enzui.Calendar",
        );
        match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("deny event timeout")
            .expect("deny event")
        {
            Event::Access(_, reply) => reply.send(false).expect("send deny"),
            other => panic!("unexpected event: {other:?}"),
        }
        let reply = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("deny timeout")
            .expect("deny join");
        assert_eq!(notify_result(&reply), 0);

        let handle5 = "/org/freedesktop/portal/desktop/notify/background5";
        let pending = call_notify(client.clone(), handle5, "io.github.enzui.Calendar");
        let (request_id, _kept_reply) =
            match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
                .await
                .expect("close-test event timeout")
                .expect("close-test event")
            {
                Event::Access(req, reply) => (req.id, reply),
                other => panic!("unexpected event: {other:?}"),
            };
        client
            .call_method(
                Some(BUS_NAME),
                handle5,
                Some("org.freedesktop.impl.portal.Request"),
                "Close",
                &(),
            )
            .await
            .expect("Request.Close call");
        let reply = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("request close timeout")
            .expect("request close join");
        assert_eq!(notify_result(&reply), 2);
        match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("request close event timeout")
            .expect("request close event")
        {
            Event::Close(closed_id) => assert_eq!(closed_id, request_id),
            other => panic!("expected Close({request_id}), got {other:?}"),
        }

        let access_handle = "/org/freedesktop/portal/desktop/request/access1";
        let access_call = {
            let client = client.clone();
            tokio::spawn(async move {
                let path = ObjectPath::try_from(access_handle).expect("access handle");
                let options: HashMap<String, OwnedValue> = HashMap::new();
                client
                    .call_method(
                        Some(BUS_NAME),
                        PATH,
                        Some("org.freedesktop.impl.portal.Access"),
                        "AccessDialog",
                        &(
                            path,
                            "io.github.enzui.Calendar",
                            "",
                            "Allow access?",
                            "",
                            "",
                            options,
                        ),
                    )
                    .await
                    .expect("AccessDialog call")
                    .body()
                    .deserialize::<(u32, HashMap<String, OwnedValue>)>()
                    .expect("AccessDialog reply")
            })
        };
        match tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
            .await
            .expect("access dialog event timeout")
            .expect("access dialog event")
        {
            Event::Access(_, reply) => drop(reply),
            other => panic!("unexpected event: {other:?}"),
        }
        let (access_response, _) = tokio::time::timeout(Duration::from_secs(5), access_call)
            .await
            .expect("access dialog timeout")
            .expect("access dialog join");
        assert_eq!(access_response, 1);
    }
}
