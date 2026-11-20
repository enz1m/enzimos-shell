use super::wire::BoundedSocket;
use crate::clipboard::session::{self, private_bytes, private_text};
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
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::MetadataExt, net::UnixStream},
    path::PathBuf,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

#[derive(Clone)]
pub struct Target {
    desktop: session::Target,
    unit: String,
    user_bus: bool,
    address: String,
    pid: u32,
    start: u64,
    device: u64,
    inode: u64,
}
fn error(_: impl std::fmt::Display) -> String {
    tr("shell-notifications-require-the-selected-managed-session-bus").into()
}
fn decode(value: &str) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = if bytes[at] == b'%' {
            let pair = value.get(at + 1..at + 3).ok_or_else(|| error("escape"))?;
            at += 3;
            u8::from_str_radix(pair, 16).map_err(error)?
        } else {
            let byte = bytes[at];
            at += 1;
            byte
        };
        if byte == 0 || byte.is_ascii_control() {
            return Err(error("address"));
        }
        decoded.push(byte);
    }
    if decoded.is_empty() || decoded.len() > 107 {
        return Err(error(tr("shell-address-length")));
    }
    Ok(decoded)
}
fn endpoint(address: &str) -> Result<(SocketAddrUnix, Option<String>), String> {
    if address.len() > 1024 || address.contains(';') {
        return Err(error("address"));
    }
    let address = address
        .strip_prefix("unix:")
        .ok_or_else(|| error("transport"))?;
    let mut fields = BTreeMap::new();
    for field in address.split(',') {
        let (key, value) = field.split_once('=').ok_or_else(|| error("field"))?;
        if !matches!(key, "path" | "abstract" | "guid") || fields.insert(key, value).is_some() {
            return Err(error("field"));
        }
    }
    let guid = fields
        .get("guid")
        .map(|value| {
            if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                Err(error("guid"))
            } else {
                Ok((*value).to_owned())
            }
        })
        .transpose()?;
    let endpoint = match (fields.get("path"), fields.get("abstract")) {
        (Some(path), None) => {
            let path = PathBuf::from(OsString::from_vec(decode(path)?));
            if !path.is_absolute() {
                return Err(error("path"));
            }
            SocketAddrUnix::new(path).map_err(error)?
        }
        (None, Some(name)) => SocketAddrUnix::new_abstract_name(&decode(name)?).map_err(error)?,
        _ => return Err(error(tr("shell-ambiguous-endpoint"))),
    };
    Ok((endpoint, guid))
}
fn socket(address: &SocketAddrUnix) -> Result<UnixStream, String> {
    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .map_err(error)?;
    match connect(&socket, address) {
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
            .map_err(error)?
                == 0
            {
                return Err(error("timeout"));
            }
            socket_error(&socket).map_err(error)?.map_err(error)?;
        }
        Err(e) => return Err(error(e)),
    }
    Ok(UnixStream::from(socket))
}
impl Target {
    pub fn from_environment() -> Result<Self, String> {
        let desktop = session::Target::from_environment().map_err(error)?;
        let address = private_text(&desktop.runtime().join("session-bus"), 1024).map_err(error)?;
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").as_deref() != Ok(address.as_str()) {
            return Err(error("selection"));
        }
        endpoint(&address)?;
        let config: Value = serde_json::from_slice(
            &private_bytes(&desktop.runtime().join("launch.json"), 32768).map_err(error)?,
        )
        .map_err(error)?;
        let unit = config["unit"]
            .as_str()
            .ok_or_else(|| error("unit"))?
            .to_owned();
        let peer: Value = serde_json::from_slice(
            &private_bytes(&desktop.runtime().join("session-bus-peer.json"), 1024)
                .map_err(error)?,
        )
        .map_err(error)?;
        // The installed session bus belongs to the user manager, at the fixed per-user runtime
        // path.
        let user_bus = config["session_bus"].as_str() == Some("user");
        if user_bus {
            let xdg = config["xdg_runtime"]
                .as_str()
                .ok_or_else(|| error("runtime"))?;
            if address != format!("unix:path={xdg}/bus") {
                return Err(error(tr("shell-user-bus")));
            }
        }
        let field = |key| peer[key].as_u64().ok_or_else(|| error("peer"));
        let pid = u32::try_from(field("pid")?).map_err(error)?;
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(error("pid"));
        }
        Ok(Self {
            desktop,
            unit,
            user_bus,
            address,
            pid,
            start: field("start")?,
            device: field("device")?,
            inode: field("inode")?,
        })
    }
    pub fn unit(&self) -> &str {
        &self.unit
    }
    fn verify_process(&self) -> Result<(), String> {
        session::membership(std::process::id(), &self.unit).map_err(error)?;
        if !self.user_bus {
            session::membership(self.pid, &self.unit).map_err(error)?;
        }
        if session::start(self.pid).map_err(error)? != self.start {
            return Err(error("identity"));
        }
        if self.user_bus {
            // The user manager's executable is not readable; the fixed bus
            // path, its owner and process start identify it instead.
            return Ok(());
        }
        let meta = fs::metadata(format!("/proc/{}/exe", self.pid)).map_err(error)?;
        if (meta.dev(), meta.ino()) != (self.device, self.inode) {
            return Err(error("executable"));
        }
        Ok(())
    }
    fn connect_socket(&self) -> Result<(UnixStream, Option<String>), String> {
        drop(self.desktop.connect().map_err(error)?);
        if private_text(&self.desktop.runtime().join("session-bus"), 1024).map_err(error)?
            != self.address
        {
            return Err(error(tr("shell-address-changed")));
        }
        self.verify_process()?;
        let (address, guid) = endpoint(&self.address)?;
        let stream = socket(&address)?;
        let peer = socket_peercred(&stream).map_err(error)?;
        if peer.pid.as_raw_pid() as u32 != self.pid
            || peer.uid.as_raw() != fs::metadata("/proc/self").map_err(error)?.uid()
        {
            return Err(error("peer"));
        }
        self.verify_process()?;
        Ok((stream, guid))
    }
    pub async fn stream(&self) -> Result<(tokio::net::UnixStream, Option<String>), String> {
        let target = self.clone();
        let (stream, guid) = tokio::task::spawn_blocking(move || target.connect_socket())
            .await
            .map_err(error)??;
        Ok((
            tokio::net::UnixStream::from_std(stream).map_err(error)?,
            guid,
        ))
    }
    pub async fn connect(&self, dropped: Arc<AtomicU64>) -> Result<zbus::Connection, String> {
        let (stream, guid) = self.stream().await?;
        let connection = tokio::time::timeout(
            Duration::from_secs(5),
            zbus::connection::Builder::socket(BoundedSocket::new(stream, dropped))
                .max_queued(1)
                .build(),
        )
        .await
        .map_err(error)?
        .map_err(error)?;
        if guid.is_some_and(|guid| {
            !connection
                .server_guid()
                .as_str()
                .eq_ignore_ascii_case(&guid)
        }) {
            return Err(error(tr("shell-guid-changed")));
        }
        Ok(connection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_behaviour() {
        // Only one explicit unix endpoint is accepted.
        {
            assert!(endpoint("unix:path=/tmp/fixture").is_ok());
            assert!(
                endpoint("unix:abstract=fixture,guid=0123456789abcdef0123456789abcdef").is_ok()
            );
            for invalid in [
                "",
                "autolaunch:",
                "tcp:host=localhost,port=1",
                "unix:path=relative",
                "unix:dir=/tmp",
                "unix:path=/tmp/a,path=/tmp/b",
                "unix:path=/tmp/a,abstract=b",
                "unix:path=/tmp/a;unix:path=/tmp/b",
                "unix:path=/tmp/a,unknown=b",
                "unix:path=/tmp/a,guid=bad",
            ] {
                assert!(endpoint(invalid).is_err(), "{invalid}");
            }
        }
        // Escapes are bounded and do not hide nulls or controls.
        {
            assert_eq!(decode("/tmp/fixture%20name").unwrap(), b"/tmp/fixture name");
            for invalid in ["", "%", "%0", "%gg", "a%00b", "a%0ab"] {
                assert!(decode(invalid).is_err());
            }
            assert!(decode(&"x".repeat(108)).is_err());
        }
    }
}
