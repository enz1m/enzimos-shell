//! A quiet-period check for desktop components replaced during this login.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::Subscription;
use iced::futures::SinkExt;

use crate::Message;

const POLL: Duration = Duration::from_secs(3);
const QUIET: Duration = Duration::from_secs(6);

type Identity = (u64, u64, i128);
type Files = Vec<(PathBuf, Option<Identity>)>;

fn identity(path: &Path) -> Option<Identity> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mtime_ns = i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec());
    Some((metadata.dev(), metadata.ino(), mtime_ns))
}

struct Monitor {
    running: Files,
    package_lock: PathBuf,
    candidate: Option<(Vec<Option<Identity>>, Instant)>,
}

impl Monitor {
    fn from_session() -> Option<Self> {
        let mode = enzui_shell::desktop_session::mode();
        if !mode.managed() {
            return None;
        }
        let running: Files =
            serde_json::from_str(&std::env::var("ENZUI_DESKTOP_UPDATE_FILES").ok()?).ok()?;
        let package_lock = if mode.nested() {
            PathBuf::from(std::env::var_os("ENZUI_DESKTOP_UPDATE_LOCK")?)
        } else {
            PathBuf::from("/var/lib/pacman/db.lck")
        };
        Some(Self {
            running,
            package_lock,
            candidate: None,
        })
    }

    fn ready(&mut self, now: Instant) -> bool {
        if self.package_lock.exists() {
            self.candidate = None;
            return false;
        }
        let installed: Vec<_> = self
            .running
            .iter()
            .map(|(path, _)| identity(path))
            .collect();
        if installed
            .iter()
            .zip(&self.running)
            .all(|(on_disk, (_, at_login))| on_disk == at_login)
        {
            self.candidate = None;
            return false;
        }
        if let Some((previous, since)) = &self.candidate
            && *previous == installed
        {
            return now.duration_since(*since) >= QUIET;
        }
        self.candidate = Some((installed, now));
        false
    }
}

pub(super) fn subscription() -> Subscription<Message> {
    Subscription::run(updates)
}

fn updates() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(1, async |mut output| {
        let Some(mut monitor) = Monitor::from_session() else {
            return;
        };
        loop {
            tokio::time::sleep(POLL).await;
            if monitor.ready(Instant::now()) {
                let _ = output.send(Message::DesktopUpdated).await;
                break;
            }
        }
        std::future::pending::<()>().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_replacement_settles_after_the_lock_and_both_pair_members() {
        let root = tempfile::tempdir().unwrap();
        let components =
            ["shell", "compositor", "scene", "harness"].map(|name| root.path().join(name));
        let package_lock = root.path().join("db.lck");
        for path in &components {
            fs::write(path, "running").unwrap();
        }
        let running = components
            .iter()
            .map(|path| (path.clone(), identity(path)))
            .collect();
        let mut monitor = Monitor {
            running,
            package_lock: package_lock.clone(),
            candidate: None,
        };
        let start = Instant::now();
        let replacement = root.path().join("replacement");
        assert!(!monitor.ready(start));
        fs::write(&package_lock, "").unwrap();
        fs::write(&replacement, "new compositor").unwrap();
        fs::rename(&replacement, &components[1]).unwrap();
        assert!(!monitor.ready(start + QUIET));
        fs::remove_file(&package_lock).unwrap();
        assert!(!monitor.ready(start + QUIET * 2));
        fs::write(&replacement, "new scene").unwrap();
        fs::rename(&replacement, &components[2]).unwrap();
        assert!(!monitor.ready(start + QUIET * 3));
        assert!(monitor.ready(start + QUIET * 4));
    }
}
