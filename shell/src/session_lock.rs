//! Starts the lock client and waits for compositor acknowledgement.
//! The lock client outlives the shell so shell exit cannot unlock the session.

use enzui_shell::i18n::tr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

static STARTING: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
// A lock client holds the lock and is supervised.
static HELD: AtomicBool = AtomicBool::new(false);

/// Whether this shell's lock screen holds the session.
pub fn locked() -> bool {
    LOCKED.load(Ordering::Acquire)
}

/// Whether the session is locked or a lock is starting.
pub fn locking() -> bool {
    STARTING.load(Ordering::Acquire) || locked()
}

/// The lock client next to the shell's own binaries, else on `PATH`.
pub fn command() -> Option<PathBuf> {
    let executable = |p: &PathBuf| {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(|d| PathBuf::from(d).join("enzui-lock"))
        .filter(executable)
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .filter(|d| d.is_absolute())
                .map(|d| d.join("enzui-lock"))
                .find(executable)
        })
}

/// Locks the session; returns once it is locked (or already was).
pub async fn lock() -> Result<(), String> {
    lock_with(command().ok_or_else(|| tr("shell-the-enzui-lock-screen-is-not-installed"))?).await
}

async fn lock_with(program: PathBuf) -> Result<(), String> {
    // Do not start a second client while this shell already holds or is acquiring the lock.
    if HELD.load(Ordering::Acquire) || STARTING.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let (result, pending) = start(program).await;
    // A client still drawing its first frame keeps the claim until it
    // reports, so a second one cannot race it.
    if !pending {
        STARTING.store(false, Ordering::Release);
    }
    result
}

type Lines = tokio::io::Lines<BufReader<tokio::process::ChildStdout>>;

// Starts a lock client and reads its first line.
async fn spawn(
    program: &PathBuf,
) -> Result<(tokio::process::Child, Lines, Option<String>), String> {
    let mut child = tokio::process::Command::new(program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        // Its own process group: signals meant for the shell do not unlock.
        .process_group(0)
        .spawn()
        .map_err(|_| "Could not start the lock screen")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not start the lock screen")?;
    let mut lines = BufReader::new(stdout).lines();
    // One CPU and software rendering can take several seconds to first frame.
    let first = match tokio::time::timeout(Duration::from_secs(15), lines.next_line()).await {
        Ok(Ok(line)) => line,
        _ => None,
    };
    Ok((child, lines, first))
}

// The outcome, and whether a slow client is still being waited for.
async fn start(program: PathBuf) -> (Result<(), String>, bool) {
    let (child, lines, first) = match spawn(&program).await {
        Ok(started) => started,
        Err(error) => return (Err(error), false),
    };
    match first.as_deref().map(str::trim) {
        Some("locked") => {
            LOCKED.store(true, Ordering::Release);
            HELD.store(true, Ordering::Release);
            locked_hint(true).await;
            tokio::spawn(supervise(program, child, lines));
            (Ok(()), false)
        }
        Some("already-locked") => {
            tokio::spawn(reap(child, lines));
            (Ok(()), false)
        }
        // It exits before locking when it cannot draw, so another locker may.
        Some(_) => {
            tokio::spawn(reap(child, lines));
            (
                Err(tr("shell-the-lock-screen-could-not-start").into()),
                false,
            )
        }
        None => {
            tokio::spawn(late(program, child, lines));
            (
                Err(tr("shell-the-lock-screen-is-still-starting").into()),
                true,
            )
        }
    }
}

// A late locked message is still the compositor acknowledgement; only then publish LockedHint.
async fn late(program: PathBuf, child: tokio::process::Child, mut lines: Lines) {
    let first = tokio::time::timeout(Duration::from_secs(120), lines.next_line()).await;
    match first {
        Ok(Ok(Some(line))) if line.trim() == "locked" => {
            LOCKED.store(true, Ordering::Release);
            HELD.store(true, Ordering::Release);
            STARTING.store(false, Ordering::Release);
            locked_hint(true).await;
            supervise(program, child, lines).await;
        }
        Ok(Ok(Some(_))) => {
            STARTING.store(false, Ordering::Release);
            reap(child, lines).await;
        }
        // Gone, or still silent after two minutes: stop it so a new lock can start.
        _ => {
            let mut child = child;
            let _ = child.kill().await;
            STARTING.store(false, Ordering::Release);
        }
    }
}

// Publishes LockedHint only after compositor acknowledgement, clearing it only after
// authenticated unlock.
async fn locked_hint(locked: bool) {
    let Ok(id) = std::env::var("XDG_SESSION_ID") else {
        return;
    };
    let set = async {
        let system = zbus::Connection::system().await.ok()?;
        let manager = zbus::Proxy::new(
            &system,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        .ok()?;
        let path: zbus::zvariant::OwnedObjectPath =
            manager.call("GetSession", &(id.as_str(),)).await.ok()?;
        let session = zbus::Proxy::new(
            &system,
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
        )
        .await
        .ok()?;
        session
            .call::<_, _, ()>("SetLockedHint", &(locked,))
            .await
            .ok()
    };
    let _ = tokio::time::timeout(Duration::from_secs(3), set).await;
}

// Keeps reading so the client never blocks on a full pipe, and reaps it.
// Returns whether it reported `unlocked`.
async fn reap(mut child: tokio::process::Child, mut lines: Lines) -> bool {
    let mut unlocked = false;
    while let Ok(Some(line)) = lines.next_line().await {
        unlocked |= line.trim() == "unlocked";
    }
    let _ = child.wait().await;
    unlocked
}

// Restart failed lock clients with rate limiting; compositor enforcement remains active
// throughout.
async fn supervise(program: PathBuf, child: tokio::process::Child, lines: Lines) {
    supervise_held(program, child, lines).await;
    HELD.store(false, Ordering::Release);
}

async fn supervise_held(program: PathBuf, child: tokio::process::Child, lines: Lines) {
    let mut restarts: Vec<std::time::Instant> = Vec::new();
    let (mut child, mut lines) = (child, lines);
    loop {
        if reap(child, lines).await {
            LOCKED.store(false, Ordering::Release);
            locked_hint(false).await;
            return;
        }
        loop {
            restarts.retain(|at| at.elapsed() < Duration::from_secs(60));
            if restarts.len() >= 5 {
                eprintln!("enzui-shell: the lock screen keeps failing; the session stays locked");
                return;
            }
            restarts.push(std::time::Instant::now());
            tokio::time::sleep(Duration::from_millis(500)).await;
            match spawn(&program).await {
                Ok((next, next_lines, Some(line))) if line.trim() == "locked" => {
                    (child, lines) = (next, next_lines);
                    break;
                }
                Ok((next, next_lines, Some(line))) if line.trim() == "already-locked" => {
                    reap(next, next_lines).await;
                    return;
                }
                Ok((next, next_lines, _)) => {
                    reap(next, next_lines).await;
                }
                Err(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_held_lock_starts_no_second_lock_screen() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let starts = directory.path().join("starts");
        let program = directory.path().join("enzui-lock");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho start >> '{}'\necho locked\nexec sleep 1\n",
                starts.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        lock_with(program.clone()).await.unwrap();
        assert!(locked() && HELD.load(Ordering::Acquire));
        lock_with(program).await.unwrap();
        assert_eq!(std::fs::read_to_string(&starts).unwrap().lines().count(), 1);
    }
}
