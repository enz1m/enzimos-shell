//! The Assistant's conversation-scoped desktop control entry point.
mod broker;
mod eis;
mod indicator;
mod portal;

use crate::{compositor::Compositor, i18n::tr};
use serde_json::{Value, json};
use std::{
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

const FRAME: usize = 4096;
const INSTALLED_BIN: &str = "/usr/lib/enzui/bin";
const INSTALLED_LIB: &str = "/usr/lib/enzui/compositor/prefix/lib";
type Failure = (&'static str, &'static str);

fn report((stage, reason): Failure) {
    eprintln!("enzui desktop control: {stage}: {reason}");
}

fn executable(name: &str) -> std::io::Result<PathBuf> {
    if crate::desktop_session::mode().nested() {
        Ok(std::env::current_exe()?.with_file_name(name))
    } else {
        Ok(Path::new(INSTALLED_BIN).join(name))
    }
}
type Result<T> = std::result::Result<T, String>;

fn unavailable() -> String {
    tr("shell-assistant-control-unavailable").into()
}

fn socket_path() -> Option<PathBuf> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    runtime
        .is_absolute()
        .then(|| runtime.join("enzui-assistant-control.sock"))
}

pub enum Request {
    Allow,
    Revoke,
}

pub async fn request(session: &str, action: Request) -> Result<()> {
    let (method, timeout) = match action {
        Request::Allow => ("allow", Duration::from_secs(75)),
        Request::Revoke => ("revoke", Duration::from_secs(2)),
    };
    let exchange = async {
        let path = socket_path().ok_or(("broker connection", "missing runtime directory"))?;
        let shell = executable("enzui-shell")
            .map_err(|_| ("broker peer check", "cannot locate the expected Shell"))?;
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|_| ("broker connection", "cannot connect to the Shell socket"))?;
        let _peer =
            Peer::new(&stream, &[&shell]).map_err(|reason| ("broker peer check", reason))?;
        let mut stream = BufReader::new(stream);
        write_frame(
            stream.get_mut(),
            &json!({
                "method": method,
                "session_id": session,
            }),
        )
        .await
        .map_err(|_| ("broker exchange", "cannot send request"))?;
        let response = read_frame(&mut stream)
            .await
            .map_err(|_| ("broker exchange", "no valid response"))?;
        Ok(response["ok"] == true)
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(true)) => return Ok(()),
        Ok(Ok(false)) => {}
        Ok(Err(failure)) => report(failure),
        Err(_) => report(("broker exchange", "request timed out")),
    }
    Err(unavailable())
}

pub fn start(compositor: Compositor) {
    if crate::desktop_session::mode().managed() {
        std::thread::spawn(move || {
            if let Err(error) = broker::run(compositor) {
                eprintln!("enzui desktop control: broker startup: {error}");
            }
        });
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn generation() -> Result<String> {
    use std::{fmt::Write, io::Read};
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| unavailable())?;
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    let mut line = Vec::new();
    reader
        .take(FRAME as u64 + 1)
        .read_until(b'\n', &mut line)
        .await
        .map_err(|_| unavailable())?;
    if line.len() > FRAME || !line.ends_with(b"\n") {
        return Err(unavailable());
    }
    serde_json::from_slice(&line).map_err(|_| unavailable())
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
) -> Result<()> {
    let mut line = value.to_string();
    if line.len() >= FRAME {
        return Err(unavailable());
    }
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|_| unavailable())
}

struct Peer(OwnedFd);

impl Peer {
    #[allow(
        unsafe_code,
        reason = "Linux pins a connected peer's identity with SO_PEERPIDFD."
    )]
    fn new(socket: &UnixStream, expected: &[&Path]) -> std::result::Result<Self, &'static str> {
        let credentials = socket
            .peer_cred()
            .map_err(|_| "cannot read socket credentials")?;
        if credentials.uid() != rustix::process::getuid().as_raw() {
            return Err("peer UID differs");
        }
        let pid = credentials.pid().ok_or("peer PID is unavailable")?;
        let mut fd = -1;
        let mut size = std::mem::size_of_val(&fd) as libc::socklen_t;
        // SAFETY: the output buffer and size describe one descriptor.
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERPIDFD,
                (&raw mut fd).cast(),
                &mut size,
            )
        } != 0
        {
            return Err(match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOPROTOOPT) => "kernel does not support SO_PEERPIDFD",
                _ => "cannot pin socket peer with SO_PEERPIDFD",
            });
        }
        // SAFETY: getsockopt returned a new owned pidfd.
        let peer = Self(unsafe { OwnedFd::from_raw_fd(fd) });
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let exe = fs::metadata(proc.join("exe")).map_err(|_| "cannot inspect peer executable")?;
        let allowed = expected
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .any(|meta| (meta.ino(), meta.dev()) == (exe.ino(), exe.dev()));
        if !allowed {
            return Err("running executable differs from the expected file");
        }
        let env = fs::read(proc.join("environ")).map_err(|_| "cannot inspect peer environment")?;
        let inherited = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
        if !clean_loader_environment(&env, crate::desktop_session::mode(), &inherited) {
            return Err("peer loader environment is unexpected");
        }
        if !peer.alive() {
            return Err("peer exited during verification");
        }
        Ok(peer)
    }

    fn alive(&self) -> bool {
        let mut wait = [rustix::event::PollFd::new(
            &self.0,
            rustix::event::PollFlags::IN,
        )];
        let zero = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        rustix::event::poll(&mut wait, Some(&zero)) == Ok(0)
    }
}

fn clean_loader_environment(
    env: &[u8],
    mode: crate::desktop_session::Mode,
    inherited: &std::ffi::OsStr,
) -> bool {
    let library_path = if mode.nested() {
        inherited
    } else {
        std::ffi::OsStr::new(INSTALLED_LIB)
    };
    env.split(|byte| *byte == 0).all(|entry| {
        !entry
            .strip_prefix(b"LD_PRELOAD=")
            .is_some_and(|value| !value.is_empty())
            && !entry
                .strip_prefix(b"LD_AUDIT=")
                .is_some_and(|value| !value.is_empty())
            && entry
                .strip_prefix(b"LD_LIBRARY_PATH=")
                .is_none_or(|value| value.is_empty() || value == library_path.as_bytes())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_peer_loader_path_is_independent_of_the_callers_environment() {
        use crate::desktop_session::Mode;
        use std::ffi::OsStr;

        let installed = b"LD_LIBRARY_PATH=/usr/lib/enzui/compositor/prefix/lib\0";
        let foreign = b"LD_LIBRARY_PATH=/tmp/foreign-library\0";
        assert!(clean_loader_environment(
            installed,
            Mode::Installed,
            OsStr::new("")
        ));
        assert!(!clean_loader_environment(
            foreign,
            Mode::Installed,
            OsStr::new("/tmp/foreign-library")
        ));
        assert!(clean_loader_environment(
            foreign,
            Mode::Preview,
            OsStr::new("/tmp/foreign-library")
        ));
    }

    #[tokio::test]
    async fn replacing_the_expected_executable_refuses_the_still_running_peer() {
        let (peer, _other) = UnixStream::pair().expect("socket pair");
        let fixture = tempfile::tempdir().expect("fixture directory");
        let expected = fixture.path().join("installed-app");
        let executable = std::env::current_exe().expect("running executable");
        std::os::unix::fs::symlink(executable, &expected).expect("installed executable");
        // Cargo adds its own loader path to test executables.
        assert!(matches!(
            Peer::new(&peer, &[&expected]),
            Ok(_) | Err("peer loader environment is unexpected")
        ));

        let replacement = fixture.path().join("update");
        fs::write(&replacement, b"updated executable").expect("replacement binary");
        fs::rename(replacement, &expected).expect("atomic install");
        assert!(matches!(
            Peer::new(&peer, &[&expected]),
            Err("running executable differs from the expected file")
        ));
    }
}
