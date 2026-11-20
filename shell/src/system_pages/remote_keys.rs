use enzui_shell::remote_access::key_fingerprint;
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

static WORKER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone, Debug)]
pub(in crate::settings_app) struct SelectedKey {
    pub(super) public_key: String,
    directory: Arc<Mutex<Option<tempfile::TempDir>>>,
}

impl SelectedKey {
    pub(super) fn accept(&self) -> Option<PathBuf> {
        self.directory
            .lock()
            .ok()?
            .take()
            .map(tempfile::TempDir::keep)
    }
}

fn read_public(path: &Path) -> Result<String, ()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| ())?;
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.is_file() || metadata.len() > 16384 {
        return Err(());
    }
    let mut text = String::new();
    file.take(16385).read_to_string(&mut text).map_err(|_| ())?;
    let text = text.trim();
    if text.len() > 16384 || key_fingerprint(text).is_none() {
        return Err(());
    }
    Ok(text.to_owned())
}

pub(super) async fn select(path: PathBuf) -> Result<SelectedKey, ()> {
    let permit = WORKER.try_acquire().map_err(|_| ())?;
    // Hold admission inside the blocking operation even if the caller retires.
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Ok(SelectedKey {
            public_key: read_public(&path)?,
            directory: Arc::new(Mutex::new(None)),
        })
    })
    .await
    .map_err(|_| ())?
}

pub(super) async fn generate() -> Result<SelectedKey, ()> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .filter(|path| path.is_absolute())
        .ok_or(())?
        .join("enzui/remote-access/keys");
    generate_at(&base).await
}

pub(super) async fn generate_at(base: &Path) -> Result<SelectedKey, ()> {
    let _permit = WORKER.try_acquire().map_err(|_| ())?;
    tokio::fs::create_dir_all(base).await.map_err(|_| ())?;
    let directory = tempfile::Builder::new()
        .prefix("key-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(base)
        .map_err(|_| ())?;
    let path = directory.path().join("id_ed25519");
    let mut child = tokio::process::Command::new("/usr/bin/ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "enzimOS remote access",
            "-f",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ())?;
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) if status.success() => {}
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(());
        }
    }
    Ok(SelectedKey {
        public_key: read_public(&path.with_extension("pub"))?,
        directory: Arc::new(Mutex::new(Some(directory))),
    })
}
