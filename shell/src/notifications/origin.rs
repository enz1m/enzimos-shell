use crate::{catalog, clipboard::session};
use enzui_shell::i18n::tr;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
pub struct Origin {
    pub sender: String,
    pub principal: String,
    pub label: String,
    pub icon: String,
    /// The sending app's own icon file, from its desktop entry.
    pub app_icon: Option<PathBuf>,
    /// The sending app's desktop entry, which a click on its message opens.
    pub desktop: Option<PathBuf>,
    pub(crate) pid: u32,
    pub(crate) start: u64,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    // Forwarded by a portal backend for a sandboxed app.
    pub(crate) portal: bool,
}
impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotificationOrigin")
    }
}
fn error(_: impl std::fmt::Display) -> String {
    tr("shell-notification-sender-identity-is-unavailable").into()
}
fn executable_principal(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().as_bytes());
    format!(
        "exe:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn policy_path(path: PathBuf, links: u64) -> PathBuf {
    if links == 0
        && let Some(bytes) = path.as_os_str().as_bytes().strip_suffix(b" (deleted)")
    {
        PathBuf::from(OsString::from_vec(bytes.to_vec()))
    } else {
        path
    }
}
fn safe_label(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c as u32,0x200b..=0x200f|0x202a..=0x202e|0x2060..=0x206f|0xfeff)
        })
        .take(80)
        .collect()
}
// Only root-owned system portal backends in installed sessions may identify sandboxed senders.
fn portal_backend(pid: u32, executable: &Path, meta: &fs::Metadata) -> bool {
    let system = matches!(
        executable.parent().and_then(Path::to_str),
        Some("/usr/lib" | "/usr/libexec")
    );
    let named = executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("xdg-desktop-portal"));
    let uid = meta.uid();
    let sealed = uid == 0 && meta.mode() & 0o022 == 0;
    let Ok(own) = fs::metadata("/proc/self").map(|meta| meta.uid()) else {
        return false;
    };
    let service = fs::read_to_string(format!("/proc/{pid}/cgroup")).is_ok_and(|membership| {
        let paths: Vec<_> = membership
            .lines()
            .filter_map(|line| line.strip_prefix("0::"))
            .collect();
        paths.len() == 1
            && paths[0].contains(&format!("/user@{own}.service/"))
            && paths[0].ends_with(".service")
    });
    system && named && sealed && service && !crate::desktop_session::mode().nested()
}
// User services and activated apps must identify the same managed desktop.
fn app_desktop(cgroup: &str, uid: u32, environment: &[u8], own: &[u8]) -> bool {
    let mut unified = cgroup.lines().filter_map(|line| line.strip_prefix("0::"));
    let Some(path) = unified.next() else {
        return false;
    };
    if unified.next().is_some() {
        return false;
    }
    let prefix = format!("/user.slice/user-{uid}.slice/user@{uid}.service/app.slice/");
    let Some(unit) = path.strip_prefix(&prefix) else {
        return false;
    };
    if unit.len() > 255
        || !(unit.ends_with(".service") || unit.ends_with(".scope"))
        || !unit
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-@\\".contains(&byte))
    {
        return false;
    }
    let value = |bytes: &[u8], key: &str| -> Option<Vec<u8>> {
        let prefix = format!("{key}=");
        let mut values = bytes
            .split(|b| *b == 0)
            .filter_map(|entry| entry.strip_prefix(prefix.as_bytes()));
        let first = values.next()?;
        (!first.is_empty() && values.next().is_none()).then(|| first.to_vec())
    };
    [
        "ENZUI_HYPR_RUNROOT",
        "WAYLAND_DISPLAY",
        "HYPRLAND_INSTANCE_SIGNATURE",
    ]
    .into_iter()
    .all(|key| {
        value(own, key).is_some_and(|expected| value(environment, key).as_ref() == Some(&expected))
    })
}
fn membership(pid: u32, unit: &str) -> Result<(), String> {
    if session::membership(pid, unit).is_ok() {
        return Ok(());
    }
    if !crate::desktop_session::mode().managed() {
        return Err(error("scope"));
    }
    let uid = fs::metadata("/proc/self").map_err(error)?.uid();
    let read = |pid| -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        fs::File::open(format!("/proc/{pid}/environ"))
            .map_err(error)?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(error)?;
        if bytes.len() > 65536 {
            return Err(error("environment bound"));
        }
        Ok(bytes)
    };
    let cgroup = fs::read_to_string(format!("/proc/{pid}/cgroup")).map_err(error)?;
    if app_desktop(&cgroup, uid, &read(pid)?, &read(std::process::id())?) {
        Ok(())
    } else {
        Err(error("scope"))
    }
}
async fn credentials(connection: &zbus::Connection, sender: &str) -> Result<(u32, u32), String> {
    let name = zbus::names::UniqueName::try_from(sender).map_err(error)?;
    tokio::time::timeout(Duration::from_secs(1), async {
        let proxy = zbus::fdo::DBusProxy::new(connection).await.map_err(error)?;
        let pid = proxy
            .get_connection_unix_process_id(name.clone().into())
            .await
            .map_err(error)?;
        let uid = proxy
            .get_connection_unix_user(name.into())
            .await
            .map_err(error)?;
        Ok::<_, String>((pid, uid))
    })
    .await
    .map_err(error)?
}
impl Origin {
    pub(super) fn internal(icon: &str) -> Result<Self, String> {
        let pid = std::process::id();
        let meta = fs::metadata("/proc/self/exe").map_err(error)?;
        let executable = policy_path(
            fs::read_link("/proc/self/exe").map_err(error)?,
            meta.nlink(),
        );
        Ok(Self {
            // No D-Bus caller may replace or close an internal notification.
            sender: String::new(),
            principal: executable_principal(&executable),
            label: tr("shell-system").into(),
            icon: icon.into(),
            app_icon: None,
            desktop: None,
            pid,
            start: session::start(pid).map_err(error)?,
            device: meta.dev(),
            inode: meta.ino(),
            portal: false,
        })
    }

    pub fn is_internal(&self) -> bool {
        self.sender.is_empty()
    }

    /// The sending process and its start time, which tell it apart from a
    /// later process with the same id.
    pub fn process(&self) -> (u32, u64) {
        (self.pid, self.start)
    }
    pub async fn capture(
        connection: &zbus::Connection,
        sender: &str,
        unit: &str,
        root: &Path,
        claimed: &str,
    ) -> Result<Self, String> {
        let (pid, uid) = credentials(connection, sender).await?;
        let sender = sender.to_owned();
        let claimed = claimed.to_owned();
        let unit = unit.to_owned();
        let root = root.to_owned();
        tokio::task::spawn_blocking(move || {
            if uid != fs::metadata("/proc/self").map_err(error)?.uid() {
                return Err(error("uid"));
            }
            let member = membership(pid, &unit);
            let start = session::start(pid).map_err(error)?;
            let meta = fs::metadata(format!("/proc/{pid}/exe")).map_err(error)?;
            let executable = policy_path(
                fs::read_link(format!("/proc/{pid}/exe")).map_err(error)?,
                meta.nlink(),
            );
            let portal = member.is_err() && portal_backend(pid, &executable, &meta);
            if member.is_err() && !portal {
                return Err(error("scope"));
            }
            let known = catalog::APPS.iter().find(|app| {
                fs::metadata(catalog::executable(&root, app.id))
                    .ok()
                    .is_some_and(|candidate| {
                        (candidate.dev(), candidate.ino()) == (meta.dev(), meta.ino())
                    })
            });
            let principal = if portal {
                // The portal names the app it identified; each app is its own source.
                executable_principal(Path::new(&format!("portal:{}", safe_label(&claimed))))
            } else {
                executable_principal(&executable)
            };
            // The desktop's own messages (battery, drives) come from this
            // process, which the pid identifies, not a claimed name.
            let own = pid == std::process::id();
            // Any other installed app is named by the desktop entry whose
            // program is the verified executable, never by what it claims.
            let installed = (known.is_none() && !own && !portal)
                .then(|| crate::search::catalogue::app_for_executable(&executable))
                .flatten();
            let (app_icon, desktop) = installed
                .as_ref()
                .map_or((None, None), |(_, icon, desktop)| {
                    (icon.clone(), Some(desktop.clone()))
                });
            let (label, icon) = match known {
                _ if own => (tr("shell-system").to_owned(), "info".to_owned()),
                _ if portal => {
                    let name =
                        crate::default_apps::name(&claimed).unwrap_or_else(|| safe_label(&claimed));
                    (
                        Some(name)
                            .filter(|name| !name.trim().is_empty())
                            .unwrap_or_else(|| tr("shell-flatpak-app").into()),
                        "package".into(),
                    )
                }
                Some(app) => (app.name.to_owned(), app.icon.to_owned()),
                None => match &installed {
                    Some((name, _, _)) => (safe_label(name), "app-window".into()),
                    None => (
                        safe_label(&executable.file_name().unwrap_or_default().to_string_lossy()),
                        "terminal".into(),
                    ),
                },
            };
            let current = fs::metadata(format!("/proc/{pid}/exe")).map_err(error)?;
            if session::start(pid).map_err(error)? != start
                || (current.dev(), current.ino()) != (meta.dev(), meta.ino())
            {
                return Err(error("identity"));
            }
            Ok(Self {
                sender,
                principal,
                label,
                icon,
                app_icon,
                desktop,
                pid,
                start,
                device: meta.dev(),
                inode: meta.ino(),
                portal,
            })
        })
        .await
        .map_err(error)?
    }
    pub async fn verify(&self, connection: &zbus::Connection, unit: &str) -> Result<(), String> {
        let (pid, uid) = credentials(connection, &self.sender).await?;
        if pid != self.pid {
            return Err(error("pid"));
        }
        let expected = self.clone();
        let unit = unit.to_owned();
        tokio::task::spawn_blocking(move || {
            if uid != fs::metadata("/proc/self").map_err(error)?.uid() {
                return Err(error("uid"));
            }
            let meta = fs::metadata(format!("/proc/{pid}/exe")).map_err(error)?;
            if expected.portal {
                let executable = fs::read_link(format!("/proc/{pid}/exe")).map_err(error)?;
                if !portal_backend(pid, &executable, &meta) {
                    return Err(error("scope"));
                }
            } else {
                membership(pid, &unit)?;
            }
            if session::start(pid).map_err(error)? != expected.start
                || (meta.dev(), meta.ino()) != (expected.device, expected.inode)
            {
                return Err(error("identity"));
            }
            Ok(())
        })
        .await
        .map_err(error)?
    }
    /// A fixed origin `number` named `label`, for the desktop's own tests.
    #[doc(hidden)]
    pub fn fixture(label: &str, number: u32) -> Self {
        Self {
            sender: format!(":1.{number}"),
            principal: format!("exe:{number:064x}"),
            label: label.into(),
            icon: "bell".into(),
            app_icon: None,
            desktop: None,
            pid: number,
            start: 1,
            device: 1,
            inode: 1,
            portal: false,
        }
    }
    pub fn same_sender(&self, other: &Self) -> bool {
        self.sender == other.sender
            && self.pid == other.pid
            && self.start == other.start
            && self.device == other.device
            && self.inode == other.inode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_behaviour() {
        // Executable identity does not use the claimed application name.
        {
            let first = executable_principal(&PathBuf::from("/fixture/one"));
            assert_eq!(first, executable_principal(&PathBuf::from("/fixture/one")));
            assert_ne!(first, executable_principal(&PathBuf::from("/fixture/two")));
            assert_eq!(first.len(), 68);
            assert_eq!(safe_label("fixture\n\u{202e}app"), "fixtureapp");
            let group = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/enzui-agent-dev.enzui.Settings.service";
            let desktop = b"ENZUI_HYPR_RUNROOT=/run/user/1000/desktop-a\0WAYLAND_DISPLAY=wayland-1\0HYPRLAND_INSTANCE_SIGNATURE=instance-a\0";
            assert!(app_desktop(group, 1000, desktop, desktop));
            let background = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/enzimos-phone-link.service";
            assert!(app_desktop(background, 1000, desktop, desktop));
            assert!(app_desktop(
                &format!("1:net_cls:/\n{background}\n"),
                1000,
                desktop,
                desktop
            ));
            assert!(app_desktop(
                &format!("{background}\n1:net_cls:/\n"),
                1000,
                desktop,
                desktop
            ));
            assert!(!app_desktop(
                &format!("{background}\n{background}"),
                1000,
                desktop,
                desktop
            ));
            assert!(!app_desktop(background, 1001, desktop, desktop));
            assert!(!app_desktop(
                background,
                1000,
                b"WAYLAND_DISPLAY=another\0",
                desktop
            ));
            assert!(!app_desktop(group, 1001, desktop, desktop));
            assert!(!app_desktop(
                group,
                1000,
                b"WAYLAND_DISPLAY=wayland-2\0",
                desktop
            ));
            assert!(!app_desktop(group, 1000, desktop, b""));
            assert!(!app_desktop(
                &format!("{group}/child"),
                1000,
                desktop,
                desktop
            ));
        }
        // Updating a running binary does not escape its path based policy.
        {
            assert_eq!(
                policy_path("/fixture/app (deleted)".into(), 0),
                PathBuf::from("/fixture/app")
            );
            assert_eq!(
                policy_path("/fixture/app (deleted)".into(), 1),
                PathBuf::from("/fixture/app (deleted)")
            );
        }
        // A reused pid or changed executable is not the same sender.
        {
            let first = Origin {
                sender: ":1.1".into(),
                principal: "fixture".into(),
                label: "Fixture".into(),
                icon: "bell".into(),
                app_icon: None,
                desktop: None,
                pid: 1,
                start: 2,
                device: 3,
                inode: 4,
                portal: false,
            };
            assert!(first.same_sender(&first.clone()));
            assert!(!first.same_sender(&Origin {
                start: 3,
                ..first.clone()
            }));
            assert!(!first.same_sender(&Origin {
                inode: 5,
                ..first.clone()
            }));
            assert!(!first.same_sender(&Origin {
                sender: ":1.2".into(),
                ..first.clone()
            }));
        }
    }
}
