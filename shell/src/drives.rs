//! UDisks2 removable-drive automount and unlock notifications.
//!
//! Automount requires a hotplugged volume in the active, unlocked seat session and HintAuto.
//! Existing/internal volumes stay untouched; polkit owns authorization.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use iced::futures::StreamExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const UDISKS: &str = "org.freedesktop.UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub path: String,
    pub label: String,
    filesystem: bool,
    encrypted: bool,
    ignore: bool,
    auto: bool,
    system: bool,
    cleartext: bool,
    mounted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    Mount,
    Offer,
    Unlock,
}

/// What a newly attached volume calls for.
#[must_use]
pub fn plan(volume: &Volume) -> Option<Plan> {
    if volume.ignore || volume.system || volume.mounted {
        return None;
    }
    // A cleartext device is mounted by the unlock dialog that opened it.
    if volume.filesystem && !volume.cleartext {
        Some(if volume.auto {
            Plan::Mount
        } else {
            Plan::Offer
        })
    } else if volume.encrypted {
        Some(Plan::Unlock)
    } else {
        None
    }
}

fn get<T: TryFrom<OwnedValue>>(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<T> {
    properties
        .get(key)
        .and_then(|value| T::try_from(value.clone()).ok())
}

/// The volume an object's interfaces describe, if it is a block device.
#[must_use]
pub fn volume(path: &str, interfaces: &Interfaces) -> Option<Volume> {
    let block = interfaces.get(BLOCK)?;
    let usage: String = get(block, "IdUsage").unwrap_or_default();
    let label: String = get::<String>(block, "IdLabel")
        .filter(|l| !l.trim().is_empty())
        .or_else(|| get::<String>(block, "HintName").filter(|l| !l.trim().is_empty()))
        .unwrap_or_else(
            || match get::<u64>(block, "Size").filter(|size| *size > 0) {
                Some(size) => {
                    tr!("shell-drive-2", "arg0" => size_text(size))
                }
                None => tr("shell-drive").into(),
            },
        );
    let backing: Option<OwnedObjectPath> = get(block, "CryptoBackingDevice");
    let mounted = interfaces
        .get(FILESYSTEM)
        .and_then(|fs| get::<Vec<Vec<u8>>>(fs, "MountPoints"))
        .is_some_and(|points| !points.is_empty());
    Some(Volume {
        path: path.to_owned(),
        label: label.chars().filter(|c| !c.is_control()).take(64).collect(),
        filesystem: usage == "filesystem" && interfaces.contains_key(FILESYSTEM),
        encrypted: usage == "crypto" && interfaces.contains_key(ENCRYPTED),
        ignore: get(block, "HintIgnore").unwrap_or(false),
        auto: get(block, "HintAuto").unwrap_or(false),
        system: get(block, "HintSystem").unwrap_or(true),
        cleartext: backing.is_some_and(|path| path.as_str() != "/"),
        mounted,
    })
}

fn size_text(bytes: u64) -> String {
    let gigabytes = bytes as f64 / 1e9;
    if gigabytes >= 1.0 {
        tr!("shell-gb-3", "arg0" => enzui_shell::i18n::number(format!("{gigabytes:.0}")))
    } else {
        tr!("shell-mb-5", "arg0" => enzui_shell::i18n::number(format!("{:.0}", bytes as f64 / 1e6)))
    }
}

/// Accepts removable-media mount roots and /mnt paths that are safe for launcher arguments.
#[must_use]
pub fn openable(mount: &Path) -> bool {
    let text = mount.to_string_lossy();
    ["/run/media/", "/media/", "/mnt/"]
        .iter()
        .any(|root| text.starts_with(root))
        && !text.contains("/../")
        && !text.chars().any(char::is_control)
        && mount.is_absolute()
}

#[derive(Clone, Debug)]
pub enum Event {
    Open(PathBuf),
    Error(String),
}

#[derive(Clone, Debug)]
enum Pending {
    // `backing` is the encrypted volume a cleartext one was unlocked from.
    Mounted {
        path: String,
        mount: PathBuf,
        label: String,
        backing: Option<String>,
    },
    Locked {
        path: String,
        label: String,
    },
    Offered {
        path: String,
        label: String,
    },
}

impl Pending {
    fn concerns(&self, object: &str) -> bool {
        match self {
            Pending::Mounted { path, backing, .. } => {
                path == object || backing.as_deref() == Some(object)
            }
            Pending::Locked { path, .. } | Pending::Offered { path, .. } => path == object,
        }
    }
}

// What the unlock dialog reports on its one line of output.
struct Unlocked {
    backing: String,
    cleartext: String,
    mount: PathBuf,
    label: String,
}

/// Watches UDisks2 for the lifetime of the login.
pub fn subscription() -> iced::Subscription<Event> {
    iced::Subscription::run(stream)
}

fn stream() -> impl iced::futures::Stream<Item = Event> {
    use iced::futures::SinkExt;
    iced::stream::channel(4, async |mut output| {
        loop {
            if let Err(error) = watch(&mut output).await {
                let _ = output.send(Event::Error(error)).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    })
}

async fn watch(output: &mut iced::futures::channel::mpsc::Sender<Event>) -> Result<(), String> {
    use iced::futures::SinkExt;
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let session = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    let manager = zbus::Proxy::new(
        &system,
        UDISKS,
        "/org/freedesktop/UDisks2",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    // UDisks2 is started on demand: a call starts it, so its signals reach
    // this subscription. What is already attached is left alone.
    let _: HashMap<OwnedObjectPath, Interfaces> = manager
        .call("GetManagedObjects", &())
        .await
        .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    let mut added = manager
        .receive_signal("InterfacesAdded")
        .await
        .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    let mut removed = manager
        .receive_signal("InterfacesRemoved")
        .await
        .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.Notifications")
        .and_then(|rule| rule.member("ActionInvoked"))
        .map(zbus::match_rule::Builder::build)
        .map_err(|_| tr("shell-notification-actions-are-unavailable"))?;
    let mut actions = zbus::MessageStream::for_match_rule(rule, &session, Some(16))
        .await
        .map_err(|_| tr("shell-notification-actions-are-unavailable"))?;
    let properties = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(UDISKS)
        .and_then(|rule| rule.interface("org.freedesktop.DBus.Properties"))
        .and_then(|rule| rule.member("PropertiesChanged"))
        .and_then(|rule| rule.path_namespace("/org/freedesktop/UDisks2/block_devices"))
        .map(zbus::match_rule::Builder::build)
        .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    let mut changed = zbus::MessageStream::for_match_rule(properties, &system, Some(64))
        .await
        .map_err(|_| tr("shell-udisks2-is-not-available"))?;
    let mut pending: HashMap<u32, Pending> = HashMap::new();
    // Hotplugged block objects not handled yet, and those that were.
    let mut waiting: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut handled: std::collections::HashSet<String> = std::collections::HashSet::new();
    let (unlocked_tx, mut unlocked) = tokio::sync::mpsc::unbounded_channel::<Unlocked>();
    loop {
        tokio::select! {
            Some(done) = unlocked.recv() => {
                let id = notify(&session, &format!("{} unlocked", done.label), tr("shell-ready-to-use"), &["open", "Open", "eject", "Eject"], true).await;
                if let Ok(id) = id {
                    pending.insert(id, Pending::Mounted { path: done.cleartext, mount: done.mount.clone(), label: done.label, backing: Some(done.backing) });
                }
                if output.send(Event::Open(done.mount)).await.is_err() {
                    return Ok(());
                }
            }
            signal = removed.next() => {
                let Some(signal) = signal else { return Err("UDisks2 stopped".into()) };
                let Ok((path, interfaces)) = signal.body().deserialize::<(OwnedObjectPath, Vec<String>)>() else { continue };
                if !interfaces.iter().any(|name| name == BLOCK) {
                    continue;
                }
                waiting.remove(path.as_str());
                handled.remove(path.as_str());
                // Unplugged: its notification has nothing left to act on.
                let gone: Vec<u32> = pending.iter().filter(|(_, item)| item.concerns(path.as_str())).map(|(id, _)| *id).collect();
                for id in gone {
                    pending.remove(&id);
                    close(&session, id).await;
                }
            }
            signal = added.next() => {
                let Some(signal) = signal else { return Err("UDisks2 stopped".into()) };
                let Ok((path, interfaces)) = signal.body().deserialize::<(OwnedObjectPath, Interfaces)>() else { continue };
                // UDisks2 adds Filesystem or Encrypted once it has probed a
                // device, in a signal of its own: read the whole object.
                if ![BLOCK, FILESYSTEM, ENCRYPTED].iter().any(|name| interfaces.contains_key(*name)) || handled.contains(path.as_str()) {
                    continue;
                }
                if waiting.len() < 64 {
                    waiting.insert(path.to_string());
                }
                if arrive(&system, &session, path.as_str(), &mut pending).await {
                    waiting.remove(path.as_str());
                    handled.insert(path.to_string());
                }
            }
            message = changed.next() => {
                let Some(Ok(message)) = message else { return Err("UDisks2 stopped".into()) };
                let Some(path) = message.header().path().map(std::string::ToString::to_string) else { continue };
                if !waiting.contains(&path) {
                    continue;
                }
                if arrive(&system, &session, &path, &mut pending).await {
                    waiting.remove(&path);
                    handled.insert(path);
                }
            }
            message = actions.next() => {
                let Some(Ok(message)) = message else { return Err(tr("shell-the-session-bus-closed").into()) };
                let Ok((id, key)) = message.body().deserialize::<(u32, String)>() else { continue };
                let Some(item) = pending.get(&id).cloned() else { continue };
                match (item, key.as_str()) {
                    (Pending::Mounted { mount, .. }, "open") => {
                        if output.send(Event::Open(mount)).await.is_err() {
                            return Ok(());
                        }
                    }
                    (item @ Pending::Mounted { .. }, "eject") => {
                        pending.remove(&id);
                        close(&session, id).await;
                        let Pending::Mounted { path, label, backing, .. } = &item else { continue };
                        match eject(&system, path, backing.as_deref()).await {
                            Ok(()) => {
                                let _ = notify(&session, &format!("{label} can be removed"), tr("shell-it-is-safe-to-unplug-it-now"), &[], false).await;
                            }
                            Err(error) => {
                                let summary = format!("{label} is still in use");
                                if let Ok(retry) = notify(&session, &summary, &error, &["eject", tr("shell-try-again")], true).await {
                                    pending.insert(retry, item);
                                }
                            }
                        }
                    }
                    (Pending::Offered { path, label }, "mount") => {
                        pending.remove(&id);
                        close(&session, id).await;
                        match mount(&system, &path).await {
                            Ok(mount) => {
                                if let Ok(id) = notify(&session, &format!("{label} connected"), tr("shell-ready-to-use"), &["open", "Open", "eject", "Eject"], true).await {
                                    pending.insert(id, Pending::Mounted { path, mount: mount.clone(), label, backing: None });
                                }
                                if output.send(Event::Open(mount)).await.is_err() {
                                    return Ok(());
                                }
                            }
                            Err(error) => {
                                let _ = notify(&session, &format!("{label} could not be opened"), &error, &[], false).await;
                            }
                        }
                    }
                    (Pending::Locked { path, label }, "unlock") => {
                        pending.remove(&id);
                        close(&session, id).await;
                        if let Err(error) = unlock_dialog(&path, label, unlocked_tx.clone()) {
                            let _ = output.send(Event::Error(error)).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// Acts on a hotplugged volume if it calls for something now; whether it
// was handled (a later change cannot make it call for more).
async fn arrive(
    system: &zbus::Connection,
    session: &zbus::Connection,
    path: &str,
    pending: &mut HashMap<u32, Pending>,
) -> bool {
    let Some(volume) = volume(path, &read_object(system, path).await) else {
        return false;
    };
    if volume.ignore || volume.mounted {
        return true;
    }
    let Some(plan) = plan(&volume) else {
        return false;
    };
    if !session_in_use(system).await {
        return true;
    }
    match plan {
        Plan::Mount => match mount(system, &volume.path).await {
            Ok(mount) => {
                if let Ok(id) = notify(
                    session,
                    &tr!("shell-connected-3", "arg0" => volume.label.clone()),
                    tr("shell-ready-to-use"),
                    &["open", tr("shell-open"), "eject", tr("shell-eject")],
                    true,
                )
                .await
                {
                    pending.insert(
                        id,
                        Pending::Mounted {
                            path: volume.path,
                            mount,
                            label: volume.label,
                            backing: None,
                        },
                    );
                }
            }
            Err(error) => {
                let _ = notify(
                    session,
                    &tr!("shell-could-not-be-opened", "arg0" => volume.label.clone()),
                    &error,
                    &[],
                    false,
                )
                .await;
            }
        },
        Plan::Offer => {
            if let Ok(id) = notify(
                session,
                &tr!("shell-connected-3", "arg0" => volume.label.clone()),
                tr("shell-mount-it-to-open-its-files"),
                &["mount", tr("shell-mount")],
                true,
            )
            .await
            {
                pending.insert(
                    id,
                    Pending::Offered {
                        path: volume.path,
                        label: volume.label,
                    },
                );
            }
        }
        Plan::Unlock => {
            if let Ok(id) = notify(
                session,
                &tr!("shell-is-encrypted", "arg0" => volume.label.clone()),
                tr("shell-unlock-it-with-its-passphrase-to-use-it"),
                &["unlock", tr("shell-unlock")],
                true,
            )
            .await
            {
                pending.insert(
                    id,
                    Pending::Locked {
                        path: volume.path,
                        label: volume.label,
                    },
                );
            }
        }
    }
    true
}

async fn read_object(system: &zbus::Connection, path: &str) -> Interfaces {
    let mut interfaces = Interfaces::new();
    let Ok(properties) = zbus::Proxy::new(
        system,
        UDISKS,
        path.to_owned(),
        "org.freedesktop.DBus.Properties",
    )
    .await
    else {
        return interfaces;
    };
    for name in [BLOCK, FILESYSTEM, ENCRYPTED] {
        if let Ok(values) = properties
            .call::<_, _, HashMap<String, OwnedValue>>("GetAll", &(name,))
            .await
        {
            interfaces.insert(name.to_owned(), values);
        }
    }
    interfaces
}

// Whether this login is the active, unlocked session on its seat.
async fn session_in_use(system: &zbus::Connection) -> bool {
    if crate::session_lock::locked() {
        return false;
    }
    let Ok(id) = std::env::var("XDG_SESSION_ID") else {
        return false;
    };
    let active: Option<bool> = async {
        let manager = zbus::Proxy::new(
            system,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        .ok()?;
        let path: OwnedObjectPath = manager.call("GetSession", &(id.as_str(),)).await.ok()?;
        let session = zbus::Proxy::new(
            system,
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
        )
        .await
        .ok()?;
        session.get_property("Active").await.ok()
    }
    .await;
    active == Some(true)
}

fn options() -> HashMap<&'static str, zbus::zvariant::Value<'static>> {
    HashMap::new()
}

async fn mount(system: &zbus::Connection, path: &str) -> Result<PathBuf, String> {
    let filesystem = zbus::Proxy::new(system, UDISKS, path.to_owned(), FILESYSTEM)
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    let mount: String = filesystem
        .call("Mount", &(options(),))
        .await
        .map_err(|error| udisks_error(&error))?;
    Ok(PathBuf::from(mount))
}

async fn eject(system: &zbus::Connection, path: &str, backing: Option<&str>) -> Result<(), String> {
    let filesystem = zbus::Proxy::new(system, UDISKS, path.to_owned(), FILESYSTEM)
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    match filesystem.call::<_, _, ()>("Unmount", &(options(),)).await {
        Ok(()) => {}
        // Already unmounted elsewhere (Files): still power it down.
        Err(error) if error.to_string().contains("NotMounted") => {}
        Err(error) => return Err(udisks_error(&error)),
    }
    // An unlocked drive is locked again before it is powered down.
    let device = match backing {
        Some(backing) => {
            let encrypted = zbus::Proxy::new(system, UDISKS, backing.to_owned(), ENCRYPTED)
                .await
                .map_err(|_| tr("shell-the-drive-went-away"))?;
            if let Err(error) = encrypted.call::<_, _, ()>("Lock", &(options(),)).await
                && !error.to_string().contains("NotUnlocked")
            {
                return Err(udisks_error(&error));
            }
            backing
        }
        None => path,
    };
    let block = zbus::Proxy::new(system, UDISKS, device.to_owned(), BLOCK)
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    let drive_path: OwnedObjectPath = block
        .get_property("Drive")
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    if drive_path.as_str() == "/" {
        return Ok(());
    }
    let drive = zbus::Proxy::new(system, UDISKS, drive_path, "org.freedesktop.UDisks2.Drive")
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    let ejectable: bool = drive.get_property("Ejectable").await.unwrap_or(false);
    let power_off: bool = drive.get_property("CanPowerOff").await.unwrap_or(false);
    // Unmounted is already safe; ejecting or powering off is a courtesy.
    if ejectable {
        let _ = drive.call::<_, _, ()>("Eject", &(options(),)).await;
    } else if power_off {
        let _ = drive.call::<_, _, ()>("PowerOff", &(options(),)).await;
    }
    Ok(())
}

/// A removable drive with a filesystem, mounted (by anyone: Files, this
/// shell, an administrator's `/etc/fstab` entry under `/mnt`) or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mounted {
    pub path: String,
    pub label: String,
    /// `None`: connected but not mounted.
    pub mount: Option<PathBuf>,
    // The encrypted volume an unlocked drive came from.
    backing: Option<String>,
}

/// The removable drives connected now, by name; `None` without UDisks2.
pub async fn mounted() -> Option<Vec<Mounted>> {
    use std::os::unix::ffi::OsStringExt;
    let system = zbus::Connection::system().await.ok()?;
    let manager = zbus::Proxy::new(
        &system,
        UDISKS,
        "/org/freedesktop/UDisks2",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    .ok()?;
    let objects: HashMap<OwnedObjectPath, Interfaces> =
        manager.call("GetManagedObjects", &()).await.ok()?;
    let mut found: Vec<Mounted> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let volume = volume(path.as_str(), interfaces)?;
            if volume.ignore || volume.system || !volume.filesystem {
                return None;
            }
            let mount = match get::<Vec<Vec<u8>>>(interfaces.get(FILESYSTEM)?, "MountPoints")
                .unwrap_or_default()
                .into_iter()
                .next()
            {
                Some(mut point) => {
                    // Mount points are NUL-terminated.
                    point.retain(|byte| *byte != 0);
                    let mount = PathBuf::from(std::ffi::OsString::from_vec(point));
                    // Mounted somewhere the desktop does not open: not a removable drive's.
                    if !openable(&mount) {
                        return None;
                    }
                    Some(mount)
                }
                None => None,
            };
            let backing = get::<OwnedObjectPath>(interfaces.get(BLOCK)?, "CryptoBackingDevice")
                .filter(|path| path.as_str() != "/")
                .map(|path| path.to_string());
            Some(Mounted {
                path: volume.path,
                label: volume.label,
                mount,
                backing,
            })
        })
        .collect();
    found.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.path.cmp(&b.path)));
    found.truncate(16);
    Some(found)
}

/// Mounts a connected drive where UDisks2 (and polkit) choose.
pub async fn mount_drive(drive: &Mounted) -> Result<PathBuf, String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    mount(&system, &drive.path).await
}

/// Unmounts a drive (locking it again if it was unlocked) and powers it down.
pub async fn eject_mounted(drive: &Mounted) -> Result<(), String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    eject(&system, &drive.path, drive.backing.as_deref()).await
}

fn udisks_error(error: &zbus::Error) -> String {
    let text = error.to_string();
    if text.contains("DeviceBusy") || text.contains("target is busy") {
        tr("shell-close-the-files-and-apps-that-are-using-it-then-try-again").into()
    } else if text.contains("NotAuthorized") {
        tr("shell-you-are-not-allowed-to-use-this-drive").into()
    } else {
        tr("shell-the-drive-did-not-respond").into()
    }
}

// Posts a notification; a `lasting` one keeps its actions in the
// notification center after its banner hides, the rest are transient.
async fn notify(
    session: &zbus::Connection,
    summary: &str,
    body: &str,
    actions: &[&str],
    lasting: bool,
) -> Result<u32, String> {
    let proxy = notifications(session)
        .await
        .ok_or_else(|| tr("shell-no-notification-service"))?;
    let mut hints = HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert("category", zbus::zvariant::Value::from("device.added"));
    if !lasting {
        hints.insert("transient", zbus::zvariant::Value::from(true));
    }
    proxy
        .call(
            "Notify",
            &(
                "Drives",
                0u32,
                "drive-removable-media",
                summary,
                body,
                actions,
                hints,
                -1i32,
            ),
        )
        .await
        .map_err(|_| tr("shell-the-notification-was-not-accepted").into())
}

async fn close(session: &zbus::Connection, id: u32) {
    if let Some(proxy) = notifications(session).await {
        let _ = proxy.call::<_, _, ()>("CloseNotification", &(id,)).await;
    }
}

async fn notifications(session: &zbus::Connection) -> Option<zbus::Proxy<'static>> {
    zbus::Proxy::new(
        session,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .ok()
}

/// Whether `path` names a UDisks2 block device object.
#[must_use]
pub fn block_object(path: &str) -> bool {
    let device = path
        .strip_prefix("/org/freedesktop/UDisks2/block_devices/")
        .unwrap_or_default();
    !device.is_empty()
        && device
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// Starts the passphrase dialog for an encrypted volume, by its UDisks2
// object path. It prints `<cleartext object> <mount point>` once unlocked.
fn unlock_dialog(
    path: &str,
    label: String,
    done: tokio::sync::mpsc::UnboundedSender<Unlocked>,
) -> Result<(), String> {
    use tokio::io::AsyncBufReadExt;
    if !block_object(path) {
        return Err(tr("shell-unexpected-drive").into());
    }
    let program = std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(|dir| PathBuf::from(dir).join("enzui-drive-unlock"))
        .filter(|program| program.is_file())
        .ok_or_else(|| tr("shell-the-unlock-dialog-is-not-installed"))?;
    let mut child = tokio::process::Command::new(program)
        .args([path, label.as_str()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| tr("shell-the-unlock-dialog-could-not-start"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| tr("shell-the-unlock-dialog-could-not-start"))?;
    let backing = path.to_owned();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        if let Ok(Some(line)) = lines.next_line().await
            && let Some((cleartext, mount)) = line.split_once(' ')
            && block_object(cleartext)
            && openable(Path::new(mount))
        {
            let _ = done.send(Unlocked {
                backing,
                cleartext: cleartext.to_owned(),
                mount: PathBuf::from(mount),
                label,
            });
        }
        let _ = child.wait().await;
    });
    Ok(())
}

/// Unlocks through UDisks2 and returns the mounted cleartext object; secrets travel only over
/// D-Bus.
pub async fn unlock(path: &str, passphrase: &str) -> Result<(String, PathBuf), String> {
    if !block_object(path) {
        return Err(tr("shell-unexpected-drive").into());
    }
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let encrypted = zbus::Proxy::new(&system, UDISKS, path.to_owned(), ENCRYPTED)
        .await
        .map_err(|_| tr("shell-the-drive-went-away"))?;
    let cleartext: OwnedObjectPath = encrypted
        .call("Unlock", &(passphrase, options()))
        .await
        .map_err(|error| {
            let text = error.to_string();
            if text.contains("Failed to activate")
                || text.contains("No key available")
                || text.contains("passphrase")
            {
                tr("shell-that-passphrase-is-not-right").to_owned()
            } else {
                udisks_error(&error)
            }
        })?;
    let mount = mount(&system, cleartext.as_str()).await?;
    Ok((cleartext.to_string(), mount))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stick() -> Volume {
        Volume {
            path: "/org/freedesktop/UDisks2/block_devices/sdb1".into(),
            label: "STICK".into(),
            filesystem: true,
            encrypted: false,
            ignore: false,
            auto: true,
            system: false,
            cleartext: false,
            mounted: false,
        }
    }

    #[test]
    fn drives_behaviour() {
        // Only unmounted removable volumes are mounted or offered for unlock.
        {
            assert_eq!(plan(&stick()), Some(Plan::Mount));
            assert_eq!(
                plan(&Volume {
                    system: true,
                    ..stick()
                }),
                None,
                "internal disk"
            );
            assert_eq!(
                plan(&Volume {
                    ignore: true,
                    ..stick()
                }),
                None
            );
            assert_eq!(
                plan(&Volume {
                    auto: false,
                    ..stick()
                }),
                Some(Plan::Offer),
                "noauto: offered, not mounted"
            );
            assert_eq!(
                plan(&Volume {
                    auto: false,
                    ignore: true,
                    ..stick()
                }),
                None
            );
            assert_eq!(
                plan(&Volume {
                    mounted: true,
                    ..stick()
                }),
                None
            );
            assert_eq!(
                plan(&Volume {
                    cleartext: true,
                    ..stick()
                }),
                None,
                "the unlock dialog mounts it"
            );
            assert_eq!(
                plan(&Volume {
                    filesystem: false,
                    encrypted: true,
                    ..stick()
                }),
                Some(Plan::Unlock)
            );
            assert_eq!(
                plan(&Volume {
                    filesystem: false,
                    ..stick()
                }),
                None,
                "swap or unknown"
            );
        }
        // Volumes are read from udisks properties.
        {
            let value = |v: zbus::zvariant::Value<'_>| OwnedValue::try_from(v).unwrap();
            let block = HashMap::from([
                ("IdUsage".to_owned(), value("filesystem".into())),
                ("IdLabel".to_owned(), value("".into())),
                ("Size".to_owned(), value(16_000_000_000u64.into())),
                ("HintAuto".to_owned(), value(true.into())),
                ("HintSystem".to_owned(), value(false.into())),
                (
                    "CryptoBackingDevice".to_owned(),
                    value(zbus::zvariant::ObjectPath::from_static_str_unchecked("/").into()),
                ),
            ]);
            let filesystem = HashMap::from([(
                "MountPoints".to_owned(),
                value(Vec::<Vec<u8>>::new().into()),
            )]);
            let interfaces: Interfaces = HashMap::from([
                (BLOCK.to_owned(), block),
                (FILESYSTEM.to_owned(), filesystem),
            ]);
            let found = volume("/org/freedesktop/UDisks2/block_devices/sdb1", &interfaces).unwrap();
            assert_eq!(
                found.label.replace(['\u{2068}', '\u{2069}'], ""),
                "16 GB drive"
            );
            assert_eq!(plan(&found), Some(Plan::Mount));
            assert!(volume("/x", &HashMap::new()).is_none());
        }
        // Only removable media mounts open.
        {
            assert!(openable(Path::new("/run/media/admin/STICK")));
            assert!(!openable(Path::new("/home/admin")));
            assert!(!openable(Path::new("/run/media/admin/a\nb")));
            assert!(
                openable(Path::new("/mnt/backup")) && !openable(Path::new("/mnt/../home/admin"))
            );
        }
    }
}
