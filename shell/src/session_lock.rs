//! Starts the lock client and waits for compositor acknowledgement.
//! The lock client outlives the shell so shell exit cannot unlock the session.

use enzui_shell::i18n::tr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

static STARTING: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
// A lock client holds the lock and is supervised.
static HELD: AtomicBool = AtomicBool::new(false);
// A lock client crashed, possibly in its graphics driver; later ones draw in software.
static CRASHED: AtomicBool = AtomicBool::new(false);

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

const LOCK_DEADLINE: Duration = Duration::from_secs(15);
// A lock client that has not reported by then is stopped so a new lock can start.
const STARTUP_LIMIT: Duration = Duration::from_secs(135);

// The pending or settled outcome of the latest lock client start, shared by every caller.
type Outcome = Option<Result<(), String>>;
static ATTEMPT: Mutex<Option<watch::Receiver<Outcome>>> = Mutex::new(None);

/// Locks the session; returns once the compositor confirms it is locked.
///
/// # Errors
/// Fails when the lock screen cannot start or is not confirmed within 15 seconds.
pub async fn lock() -> Result<(), String> {
    lock_within(LOCK_DEADLINE).await
}

/// Locks the session, waiting at most `limit` for the compositor to confirm it.
/// A lock still starting at the deadline keeps starting in the background.
///
/// # Errors
/// Fails when the lock screen cannot start or is not confirmed within `limit`.
pub async fn lock_within(limit: Duration) -> Result<(), String> {
    lock_with(
        command().ok_or_else(|| tr("shell-the-enzui-lock-screen-is-not-installed"))?,
        limit,
    )
    .await
}

async fn lock_with(program: PathBuf, limit: Duration) -> Result<(), String> {
    let mut outcome = {
        let mut attempt = ATTEMPT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if HELD.load(Ordering::Acquire) && locked() {
            return Ok(());
        }
        // Join a client that is still starting rather than racing a second one.
        match attempt
            .as_ref()
            .filter(|outcome| outcome.borrow().is_none())
        {
            Some(pending) => pending.clone(),
            None => {
                let (child, lines) = launch(&program)?;
                let (sender, receiver) = watch::channel(None);
                STARTING.store(true, Ordering::Release);
                tokio::spawn(acquire(program, child, lines, sender));
                *attempt = Some(receiver.clone());
                receiver
            }
        }
    };
    match tokio::time::timeout(limit, outcome.wait_for(Option::is_some)).await {
        Ok(Ok(settled)) => settled
            .clone()
            .unwrap_or_else(|| Err(tr("shell-the-lock-screen-could-not-start").into())),
        Ok(Err(_)) => Err(tr("shell-the-lock-screen-could-not-start").into()),
        Err(_) => Err(tr("shell-the-lock-screen-is-still-starting").into()),
    }
}

type Lines = tokio::io::Lines<BufReader<tokio::process::ChildStdout>>;

fn launch(program: &PathBuf) -> Result<(tokio::process::Child, Lines), String> {
    let mut command = tokio::process::Command::new(program);
    if CRASHED.load(Ordering::Acquire) {
        command.env("ENZUI_LOCK_SOFTWARE", "1");
    }
    let mut child = command
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
    Ok((child, BufReader::new(stdout).lines()))
}

// Starts a lock client and reads its first line.
async fn spawn(
    program: &PathBuf,
) -> Result<(tokio::process::Child, Lines, Option<String>), String> {
    let (child, mut lines) = launch(program)?;
    // One CPU and software rendering can take several seconds to first frame.
    let first = match tokio::time::timeout(Duration::from_secs(15), lines.next_line()).await {
        Ok(Ok(line)) => line,
        _ => None,
    };
    Ok((child, lines, first))
}

fn note_crash(status: std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    let crashed = status.signal().is_some_and(|signal| {
        [
            libc::SIGSEGV,
            libc::SIGABRT,
            libc::SIGBUS,
            libc::SIGILL,
            libc::SIGFPE,
            libc::SIGTRAP,
        ]
        .contains(&signal)
    });
    if crashed {
        CRASHED.store(true, Ordering::Release);
    }
    crashed
}

// Only the compositor acknowledgement, reported as `locked`, publishes LockedHint.
async fn acquire(
    program: PathBuf,
    mut child: tokio::process::Child,
    mut lines: Lines,
    outcome: watch::Sender<Outcome>,
) {
    let deadline = tokio::time::Instant::now() + STARTUP_LIMIT;
    let mut retried = false;
    let first = loop {
        let first = tokio::time::timeout_at(deadline, lines.next_line()).await;
        // A client that crashed after taking the lock leaves it without a surface.
        if !retried
            && matches!(first, Ok(Ok(None) | Err(_)))
            && tokio::time::timeout(Duration::from_secs(1), child.wait())
                .await
                .is_ok_and(|status| status.is_ok_and(note_crash))
            && let Ok((next, next_lines)) = launch(&program)
        {
            retried = true;
            (child, lines) = (next, next_lines);
            continue;
        }
        break first;
    };
    let settle = |result: Result<(), String>| {
        STARTING.store(false, Ordering::Release);
        outcome.send_replace(Some(result));
    };
    match first {
        Ok(Ok(Some(line))) if line.trim() == "locked" => {
            LOCKED.store(true, Ordering::Release);
            HELD.store(true, Ordering::Release);
            settle(Ok(()));
            locked_hint(true).await;
            supervise(program, child, lines).await;
        }
        Ok(Ok(Some(line))) if line.trim() == "already-locked" => {
            settle(Ok(()));
            reap(child, lines).await;
        }
        // It exits before locking when it cannot draw, so another locker may.
        Ok(Ok(Some(_))) => {
            settle(Err(tr("shell-the-lock-screen-could-not-start").into()));
            reap(child, lines).await;
        }
        _ => {
            let _ = child.kill().await;
            settle(Err(tr("shell-the-lock-screen-could-not-start").into()));
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
    if let Ok(status) = child.wait().await {
        note_crash(status);
    }
    unlocked
}

// Restart failed lock clients with rate limiting; compositor enforcement remains active
// throughout. HELD is cleared before any await so a new lock never trusts an ending client.
async fn supervise(program: PathBuf, child: tokio::process::Child, lines: Lines) {
    let mut restarts: Vec<std::time::Instant> = Vec::new();
    let (mut child, mut lines) = (child, lines);
    loop {
        if reap(child, lines).await {
            HELD.store(false, Ordering::Release);
            LOCKED.store(false, Ordering::Release);
            locked_hint(false).await;
            return;
        }
        loop {
            restarts.retain(|at| at.elapsed() < Duration::from_secs(60));
            if restarts.len() >= 5 {
                HELD.store(false, Ordering::Release);
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
                    HELD.store(false, Ordering::Release);
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
    async fn lock_succeeds_only_after_one_client_confirms() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        // A client that crashes while starting is replaced by one drawing in software.
        let crashing = directory.path().join("enzui-lock-crashing");
        std::fs::write(
            &crashing,
            "#!/bin/sh\n[ \"$ENZUI_LOCK_SOFTWARE\" = 1 ] && echo already-locked || kill -SEGV $$\n",
        )
        .unwrap();
        std::fs::set_permissions(&crashing, std::fs::Permissions::from_mode(0o700)).unwrap();
        lock_with(crashing, LOCK_DEADLINE).await.unwrap();
        assert!(!locked());

        let starts = directory.path().join("starts");
        let program = directory.path().join("enzui-lock");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho start >> '{}'\nsleep 0.5\necho locked\nexec sleep 5\n",
                starts.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let expired = lock_with(program.clone(), Duration::from_millis(50)).await;
        assert!(expired.is_err() && !locked());
        let first = tokio::spawn(lock_with(program.clone(), LOCK_DEADLINE));
        lock_with(program.clone(), LOCK_DEADLINE).await.unwrap();
        assert!(locked() && HELD.load(Ordering::Acquire));
        first.await.unwrap().unwrap();
        lock_with(program, LOCK_DEADLINE).await.unwrap();
        assert_eq!(std::fs::read_to_string(&starts).unwrap().lines().count(), 1);
    }
}
