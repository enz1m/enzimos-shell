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
const SHELL_EXE: &str = "/usr/lib/enzui/bin/enzui-shell";
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
        let path = socket_path().ok_or_else(unavailable)?;
        let shell = if crate::desktop_session::mode().nested() {
            std::env::current_exe()
                .map_err(|_| unavailable())?
                .with_file_name("enzui-shell")
        } else {
            PathBuf::from(SHELL_EXE)
        };
        let stream = UnixStream::connect(&path)
            .await
            .map_err(|_| unavailable())?;
        let _peer = Peer::new(&stream, &[&shell]).ok_or_else(unavailable)?;
        let mut stream = BufReader::new(stream);
        write_frame(
            stream.get_mut(),
            &json!({
                "method": method,
                "session_id": session,
            }),
        )
        .await?;
        let response = read_frame(&mut stream).await?;
        if response["ok"] == true {
            Ok(())
        } else {
            Err(unavailable())
        }
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| unavailable())?
}

pub fn start(compositor: Compositor) {
    if crate::desktop_session::mode().managed() {
        std::thread::spawn(move || {
            if let Err(error) = broker::run(compositor) {
                eprintln!("enzui-shell: desktop control: {error}");
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
    fn new(socket: &UnixStream, expected: &[&Path]) -> Option<Self> {
        let credentials = socket.peer_cred().ok()?;
        if credentials.uid() != rustix::process::getuid().as_raw() {
            return None;
        }
        let pid = credentials.pid()?;
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
            return None;
        }
        // SAFETY: getsockopt returned a new owned pidfd.
        let peer = Self(unsafe { OwnedFd::from_raw_fd(fd) });
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let exe = fs::metadata(proc.join("exe")).ok()?;
        let allowed = expected
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .any(|meta| (meta.ino(), meta.dev()) == (exe.ino(), exe.dev()));
        let env = fs::read(proc.join("environ")).ok()?;
        let library_path = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
        let clean = env.split(|byte| *byte == 0).all(|entry| {
            !entry
                .strip_prefix(b"LD_PRELOAD=")
                .is_some_and(|value| !value.is_empty())
                && !entry
                    .strip_prefix(b"LD_AUDIT=")
                    .is_some_and(|value| !value.is_empty())
                && entry
                    .strip_prefix(b"LD_LIBRARY_PATH=")
                    .is_none_or(|value| value.is_empty() || value == library_path.as_bytes())
        });
        (allowed && clean && peer.alive()).then_some(peer)
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
