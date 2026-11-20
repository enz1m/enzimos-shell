use enzui_shell::i18n::tr;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    net::{
        AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with,
        sockopt::{socket_error, socket_peercred},
    },
};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
};

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Target {
    runtime: PathBuf,
    xdg: PathBuf,
    display: String,
}
fn failure(_: impl std::fmt::Display) -> String {
    tr("shell-clipboard-requires-the-explicitly-selected-managed-desktop").into()
}
pub(crate) fn private_bytes(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(failure)?;
    let meta = file.metadata().map_err(failure)?;
    let uid = fs::metadata("/proc/self").map_err(failure)?.uid();
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(failure("metadata"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(failure)?;
    if bytes.len() > limit {
        return Err(failure("size"));
    }
    Ok(bytes)
}
pub(crate) fn private_text(path: &Path, limit: usize) -> Result<String, String> {
    String::from_utf8(private_bytes(path, limit)?)
        .map(|s| s.trim().to_owned())
        .map_err(failure)
}
pub(crate) fn start(pid: u32) -> Result<u64, String> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(failure)?
        .rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| failure("identity"))
}
pub(crate) fn membership(pid: u32, unit: &str) -> Result<(), String> {
    let membership = fs::read_to_string(format!("/proc/{pid}/cgroup")).map_err(failure)?;
    let paths: Vec<_> = membership
        .lines()
        .filter_map(|line| line.strip_prefix("0::"))
        .collect();
    if paths.len() != 1 || !paths[0].ends_with(&format!("/{unit}/supervisor")) {
        return Err(failure("scope"));
    }
    Ok(())
}
fn managed_layout(runtime: &Path, xdg: &Path) -> bool {
    if runtime.parent() == Some(xdg.join("enzui-shell-platform").as_path()) {
        return true;
    }
    let Some(platform) = runtime.parent() else {
        return false;
    };
    let Some(root) = platform.parent() else {
        return false;
    };
    let Some(name) = runtime.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some((profile, nonce)) = name.rsplit_once('-') else {
        return false;
    };
    platform
        .file_name()
        .is_some_and(|name| name == "enzui-shell-platform")
        && !profile.is_empty()
        && profile.len() <= 32
        && profile
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && nonce.len() == 16
        && nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && xdg == root.join(format!("e{}", nonce.get(..8).unwrap_or_default()))
}
impl Target {
    pub(crate) fn runtime(&self) -> &Path {
        &self.runtime
    }
    pub fn from_environment() -> Result<Self, String> {
        if std::env::var("ENZUI_PLUGIN_ISOLATION").as_deref() != Ok("delegated") {
            return Err(failure("isolation"));
        }
        Self::managed(
            std::env::var_os("ENZUI_HYPR_RUNROOT")
                .map(PathBuf::from)
                .ok_or_else(|| failure("runtime"))?,
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| failure("runtime"))?,
            std::env::var("WAYLAND_DISPLAY").map_err(failure)?,
        )
    }
    pub fn managed(runtime: PathBuf, xdg: PathBuf, display: String) -> Result<Self, String> {
        let target = Self {
            runtime,
            xdg,
            display,
        };
        target.validate_paths()?;
        Ok(target)
    }
    fn validate_paths(&self) -> Result<(), String> {
        if !self.xdg.is_absolute()
            || !managed_layout(&self.runtime, &self.xdg)
            || !self.display.strip_prefix("wayland-").is_some_and(|s| {
                !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_digit())
            })
        {
            return Err(failure("path"));
        }
        for path in [&self.xdg, &self.runtime] {
            let meta = fs::symlink_metadata(path).map_err(failure)?;
            if !meta.is_dir()
                || meta.uid() != fs::metadata("/proc/self").map_err(failure)?.uid()
                || meta.mode() & 0o077 != 0
            {
                return Err(failure("directory"));
            }
        }
        Ok(())
    }
    fn matches_config(&self, config: &Value, display: &str) -> bool {
        config["runtime"].as_str() == self.runtime.to_str()
            && config["xdg_runtime"].as_str() == self.xdg.to_str()
            && display == self.display
    }
    pub fn connect(&self) -> Result<UnixStream, String> {
        self.connect_checked(true)
    }
    // Reading a live desktop receipt is also allowed from a separately activated
    // Settings unit. Clipboard operations still require the caller's membership.
    pub(crate) fn connect_observer(&self) -> Result<UnixStream, String> {
        self.connect_checked(false)
    }
    fn connect_checked(&self, require_caller: bool) -> Result<UnixStream, String> {
        self.validate_paths()?;
        let config: Value =
            serde_json::from_slice(&private_bytes(&self.runtime.join("launch.json"), 32768)?)
                .map_err(failure)?;
        let unit = config["unit"]
            .as_str()
            .filter(|s| {
                s.starts_with("enzui-platform-")
                    && s.ends_with(".service")
                    && s.len() <= 100
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            })
            .ok_or_else(|| failure("unit"))?;
        if !self.matches_config(
            &config,
            &private_text(&self.runtime.join("wayland-display"), 100)?,
        ) {
            return Err(failure("selection"));
        }
        let pid: u32 = private_text(&self.runtime.join("hyprland.pid"), 32)?
            .parse()
            .map_err(failure)?;
        let ticks: u64 = private_text(&self.runtime.join("hyprland.pid.start"), 32)?
            .parse()
            .map_err(failure)?;
        if require_caller {
            membership(std::process::id(), unit)?;
        }
        membership(pid, unit)?;
        if start(pid)? != ticks {
            return Err(failure("start"));
        }
        let build = Path::new(config["build"].as_str().ok_or_else(|| failure("build"))?);
        if !build.is_absolute() {
            return Err(failure("build"));
        }
        // enz-hyprland; build roots from before the rename hold Hyprland.
        let expected = fs::metadata(build.join("hyprland-current-build/enz-hyprland"))
            .or_else(|_| fs::metadata(build.join("hyprland-current-build/Hyprland")))
            .map_err(failure)?;
        let actual = fs::metadata(format!("/proc/{pid}/exe")).map_err(failure)?;
        if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino()) {
            return Err(failure("executable"));
        }
        let socket = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .map_err(failure)?;
        let address = SocketAddrUnix::new(self.xdg.join(&self.display)).map_err(failure)?;
        match connect(&socket, &address) {
            Ok(()) => (),
            Err(rustix::io::Errno::INPROGRESS) => {
                let mut fds = [PollFd::new(&socket, PollFlags::OUT)];
                if poll(
                    &mut fds,
                    Some(&Timespec {
                        tv_sec: 0,
                        tv_nsec: 500_000_000,
                    }),
                )
                .map_err(failure)?
                    == 0
                {
                    return Err(failure(tr("shell-connection-timeout")));
                }
                socket_error(&socket).map_err(failure)?.map_err(failure)?;
            }
            Err(e) => return Err(failure(e)),
        }
        let stream = UnixStream::from(socket);
        let peer = socket_peercred(&stream).map_err(failure)?;
        if peer.pid.as_raw_pid() as u32 != pid
            || peer.uid.as_raw() != fs::metadata("/proc/self").map_err(failure)?.uid()
            || start(pid)? != ticks
        {
            return Err(failure("peer"));
        }
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn session_behaviour() {
        // Private runtime namespace is bound to the selected session nonce.
        {
            let root = tempfile::tempdir().unwrap();
            let runtime = root
                .path()
                .join("enzui-shell-platform/fixture-0123456789abcdef");
            let xdg = root.path().join("e01234567");
            fs::create_dir_all(&runtime).unwrap();
            fs::create_dir(&xdg).unwrap();
            for path in [&runtime, &xdg] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let target = Target {
                runtime: runtime.clone(),
                xdg: xdg.clone(),
                display: "wayland-0".into(),
            };
            assert!(target.validate_paths().is_ok());
            assert!(target.connect().is_err());
            let config = serde_json::json!({"runtime":runtime,"xdg_runtime":xdg});
            assert!(target.matches_config(&config, "wayland-0"));
            let host_target = Target {
                xdg: root.path().into(),
                ..target.clone()
            };
            assert!(!host_target.matches_config(&config, "wayland-0"));
            assert!(!target.matches_config(&config, "wayland-1"));
            assert!(!managed_layout(&runtime, &root.path().join("e99999999")));
            assert!(!managed_layout(
                &root.path().join("elsewhere/fixture-0123456789abcdef"),
                &xdg
            ));
            assert!(!managed_layout(
                &root.path().join("enzui-shell-platform/fixture-short"),
                &xdg
            ));
        }
        // Endpoints cannot fall back to the host or escape the managed root.
        {
            let root = tempfile::tempdir().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let runtime = root.path().join("enzui-shell-platform/fixture");
            fs::create_dir_all(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            let target = Target {
                runtime,
                xdg: root.path().into(),
                display: "wayland-1".into(),
            };
            assert!(target.validate_paths().is_ok());
            assert!(target.connect().is_err());
            for display in [
                "/run/user/1000/wayland-0",
                "../wayland-0",
                "wayland-",
                "wayland-1/../../wayland-0",
            ] {
                assert!(
                    Target {
                        display: display.into(),
                        ..target.clone()
                    }
                    .validate_paths()
                    .is_err()
                );
            }
            assert!(
                Target {
                    runtime: root.path().join("legacy"),
                    ..target
                }
                .validate_paths()
                .is_err()
            );
        }
        // Private metadata rejects links public files and oversize.
        {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("metadata");
            fs::write(&path, b"fixture").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(private_bytes(&path, 7).unwrap(), b"fixture");
            assert!(private_bytes(&path, 6).is_err());
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(private_bytes(&link, 7).is_err());
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(private_bytes(&path, 7).is_err());
        }
    }
}
