//! Fingerprint unlock beside the password on the lock screen.
//!
//! A finger never unlocks saved credentials: only the password reaches the
//! Secret Service. While the credential keyring is locked the lock asks for
//! the password instead, unless a password unlock in this session already
//! left it locked. An attempt that a system sleep or a busy reader cut short
//! is started again.

use crate::auth;
use enzui_shell::i18n::tr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const QUERY_LIMIT: Duration = Duration::from_secs(2);
const FIRST_RETRY: Duration = Duration::from_secs(2);
const LAST_RETRY: Duration = Duration::from_secs(60);
// A hot reader prompts and fails at once, so only a longer attempt resets the backoff.
const STEADY_ATTEMPT: Duration = Duration::from_secs(30);
// Suspend time counts in the boot clock but not in the monotonic clock.
const SLEEP_GAP: Duration = Duration::from_secs(1);

/// What the fingerprint thread reports to the lock.
pub(crate) enum Report {
    /// Text below the password field; `None` clears it.
    Text(Option<String>),
    /// The enrolled finger matched.
    Matched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keyring {
    Locked,
    /// Unlocked, absent, not running, or not answering.
    Open,
}

#[derive(Debug, PartialEq, Eq)]
enum Next {
    Retry(Duration),
    Stop,
}

/// Whether the lock waits for the password rather than a finger.
fn password_first(keyring: Keyring, password_tried: bool) -> bool {
    keyring == Keyring::Locked && !password_tried
}

/// What follows an attempt that ended without a match or a refusal.
///
/// `prompted` is whether any attempt in this lock reached the reader;
/// `slept` whether the system slept or was preparing to sleep.
fn after_unavailable(prompted: bool, slept: bool, failures: u32) -> Next {
    if slept {
        Next::Retry(Duration::ZERO)
    } else if prompted {
        Next::Retry(
            FIRST_RETRY
                .saturating_mul(1 << failures.min(8))
                .min(LAST_RETRY),
        )
    } else {
        Next::Stop
    }
}

// Written when the password unlocked the lock while it was asked for the keyring:
// a keyring still locked afterwards does not open with the login password.
fn marker() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    runtime
        .is_absolute()
        .then(|| runtime.join("enzui-lock-keyring-password-tried"))
}

/// Records that a password unlock happened while the lock was waiting for it.
pub(crate) fn password_used_for_keyring() {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(path) = marker() {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path);
    }
}

fn sleep_gap() -> Duration {
    use rustix::time::{ClockId, clock_gettime};
    let span = |id| {
        let t = clock_gettime(id);
        Duration::new(
            t.tv_sec.max(0) as u64,
            t.tv_nsec.clamp(0, 999_999_999) as u32,
        )
    };
    span(ClockId::Boottime).saturating_sub(span(ClockId::Monotonic))
}

async fn keyring() -> Keyring {
    let query = async {
        let bus = zbus::Connection::session().await.ok()?;
        let dbus = zbus::fdo::DBusProxy::new(&bus).await.ok()?;
        let name = zbus::names::BusName::try_from("org.freedesktop.secrets").ok()?;
        // Asking a stopped keyring would start it locked; the password could not reach it.
        if !dbus.name_has_owner(name).await.ok()? {
            return None;
        }
        let service = zbus::Proxy::new(
            &bus,
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
        )
        .await
        .ok()?;
        let path: zbus::zvariant::OwnedObjectPath =
            service.call("ReadAlias", &("default",)).await.ok()?;
        if path.as_str() == "/" {
            return None;
        }
        let collection = zbus::Proxy::new(
            &bus,
            "org.freedesktop.secrets",
            path,
            "org.freedesktop.Secret.Collection",
        )
        .await
        .ok()?;
        collection.get_property::<bool>("Locked").await.ok()
    };
    match tokio::time::timeout(QUERY_LIMIT, query).await {
        Ok(Some(true)) => Keyring::Locked,
        _ => Keyring::Open,
    }
}

async fn preparing_for_sleep() -> bool {
    let query = async {
        let bus = zbus::Connection::system().await.ok()?;
        let manager = zbus::Proxy::new(
            &bus,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        .ok()?;
        manager.get_property::<bool>("PreparingForSleep").await.ok()
    };
    matches!(
        tokio::time::timeout(QUERY_LIMIT, query).await,
        Ok(Some(true))
    )
}

/// Offers fingerprint unlock until a finger matches, the reader refuses
/// further attempts, or no reader or enrolled finger exists. Blocking.
///
/// Sets `waiting_for_password` when the lock asks for the password to unlock
/// saved credentials instead.
pub(crate) fn run(report: &dyn Fn(Report), waiting_for_password: &AtomicBool) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let password_tried = marker().is_some_and(|path| path.exists());
    let keyring = runtime.block_on(keyring());
    if keyring == Keyring::Open
        && let Some(path) = marker()
    {
        let _ = std::fs::remove_file(path);
    }
    if password_first(keyring, password_tried) {
        waiting_for_password.store(true, Ordering::Release);
        report(Report::Text(Some(
            tr("shell-fingerprint-credentials-locked").to_owned(),
        )));
        return;
    }
    let mut prompted = false;
    let mut failures = 0;
    loop {
        while runtime.block_on(preparing_for_sleep()) {
            std::thread::sleep(Duration::from_millis(500));
        }
        let gap = sleep_gap();
        let started = Instant::now();
        let reached = AtomicBool::new(false);
        let show = |text: String| {
            reached.store(true, Ordering::Relaxed);
            report(Report::Text(Some(text)));
        };
        match auth::fingerprint(&show) {
            auth::Fingerprint::Matched => return report(Report::Matched),
            auth::Fingerprint::Refused => {
                return report(Report::Text(Some(
                    tr("shell-fingerprint-not-recognized").to_owned(),
                )));
            }
            auth::Fingerprint::Unavailable => {}
        }
        prompted |= reached.load(Ordering::Relaxed);
        if started.elapsed() >= STEADY_ATTEMPT {
            failures = 0;
        }
        let slept = sleep_gap() > gap + SLEEP_GAP || runtime.block_on(preparing_for_sleep());
        match after_unavailable(prompted, slept, failures) {
            Next::Stop => return report(Report::Text(None)),
            Next::Retry(delay) => {
                if !delay.is_zero() {
                    report(Report::Text(None));
                    failures += 1;
                    std::thread::sleep(delay);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_keyring_waits_for_the_password_once() {
        assert!(password_first(Keyring::Locked, false));
        assert!(!password_first(Keyring::Locked, true));
        assert!(!password_first(Keyring::Open, false));
    }

    #[test]
    fn interrupted_attempts_restart_but_a_missing_reader_does_not() {
        assert_eq!(after_unavailable(false, false, 0), Next::Stop);
        assert_eq!(
            after_unavailable(false, true, 0),
            Next::Retry(Duration::ZERO)
        );
        assert_eq!(after_unavailable(true, false, 0), Next::Retry(FIRST_RETRY));
        assert_eq!(after_unavailable(true, false, 40), Next::Retry(LAST_RETRY));
    }
}
