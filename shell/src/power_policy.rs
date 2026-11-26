//! Lock-before-sleep and lid policy for installed sessions.
//! logind Unlock never bypasses lock-screen authentication; previews hold no host inhibitors.

use crate::session_preferences::LidAction;
use enzui_shell::i18n::tr;
use iced::futures::StreamExt;
use std::time::Duration;
use zbus::zvariant::OwnedFd;

async fn inhibit(manager: &zbus::Proxy<'_>, what: &str, why: &str, mode: &str) -> Option<OwnedFd> {
    manager
        .call("Inhibit", &(what, "enzUI", why, mode))
        .await
        .ok()
}

async fn set_displays(compositor: Option<&crate::compositor::Compositor>, on: bool) {
    let Some(compositor) = compositor else {
        return;
    };
    let action = if on { "on" } else { "off" };
    let _ = compositor
        .request(&["dispatch", &format!("hl.dsp.dpms({{action='{action}'}})")])
        .await;
}

/// Watches sleep signals independently of changes to the lid preference.
pub fn subscription(lid: LidAction) -> iced::Subscription<String> {
    iced::Subscription::batch([
        iced::Subscription::run(sleep_stream),
        iced::Subscription::run_with(lid, lid_stream),
    ])
}

fn sleep_stream() -> impl iced::futures::Stream<Item = String> {
    use iced::futures::SinkExt;
    iced::stream::channel(1, async |mut output| {
        loop {
            if let Err(error) = run_sleep().await
                && output.send(error).await.is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "Subscription::run_with borrows the concrete hashed input."
)]
fn lid_stream(lid: &LidAction) -> impl iced::futures::Stream<Item = String> + use<> {
    use iced::futures::SinkExt;
    let lid = *lid;
    iced::stream::channel(1, async move |mut output| {
        if let Err(error) = run_lid(lid).await {
            let _ = output.send(error).await;
        }
    })
}

const LOCK_RETRY: Duration = Duration::from_millis(200);

async fn lock_before_deadline(
    mut lock: impl AsyncFnMut(Duration) -> Result<(), String>,
    deadline: tokio::time::Instant,
) -> Result<(), String> {
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match lock(remaining).await {
            Ok(()) => return Ok(()),
            Err(error) if tokio::time::Instant::now() + LOCK_RETRY >= deadline => {
                tokio::time::sleep_until(deadline).await;
                return Err(error);
            }
            Err(_) => tokio::time::sleep(LOCK_RETRY).await,
        }
    }
}

async fn run_sleep() -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    let mut sleeping = manager
        .receive_signal("PrepareForSleep")
        .await
        .map_err(|_| tr("shell-logind-is-not-available"))?;
    // The login's own session object; the shell runs in the user manager,
    // so it is found by XDG_SESSION_ID rather than by process.
    let session: Option<zbus::zvariant::OwnedObjectPath> = match std::env::var("XDG_SESSION_ID") {
        Ok(id) if !id.is_empty() => manager.call("GetSession", &(id.as_str(),)).await.ok(),
        _ => None,
    };
    let mut lock_requests = match &session {
        Some(path) => match zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path.as_ref(),
            "org.freedesktop.login1.Session",
        )
        .await
        {
            Ok(proxy) => proxy.receive_signal("Lock").await.ok(),
            Err(_) => None,
        },
        None => None,
    };
    let compositor = crate::compositor::Compositor::from_environment().ok();
    // logind sleeps once delay inhibitors close or this limit passes.
    let delay_limit = manager
        .get_property::<u64>("InhibitDelayMaxUSec")
        .await
        .map_or(Duration::from_secs(5), Duration::from_micros);
    // Held while awake; logind waits for it to close before sleeping.
    let mut delay = inhibit(
        &manager,
        "sleep",
        tr("shell-locking-the-screen-before-sleep"),
        "delay",
    )
    .await;
    let mut lock_failed = false;
    loop {
        tokio::select! {
            signal = sleeping.next() => {
                let Some(signal) = signal else { return Ok(()) };
                let Ok(starting) = signal.body().deserialize::<bool>() else { continue };
                if starting {
                    let deadline = tokio::time::Instant::now() + delay_limit;
                    lock_failed = match lock_before_deadline(crate::session_lock::lock_for_sleep, deadline).await {
                        Ok(()) => {
                            let _ = tokio::time::timeout_at(deadline, set_displays(compositor.as_ref(), false)).await;
                            false
                        }
                        Err(error) => {
                            eprintln!("enzui-shell: lock before sleep: {error}");
                            true
                        }
                    };
                    delay = None;
                } else {
                    if delay.is_none() {
                        delay = inhibit(&manager, "sleep", tr("shell-locking-the-screen-before-sleep"), "delay").await;
                    }
                    if lock_failed
                        && let Err(error) = crate::session_lock::lock_for_sleep(crate::session_lock::SLEEP_LOCK_LIMIT).await
                    {
                        eprintln!("enzui-shell: lock on resume: {error}");
                    }
                    set_displays(compositor.as_ref(), true).await;
                }
            }
            request = async {
                match lock_requests.as_mut() {
                    Some(requests) => requests.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if request.is_none() {
                    lock_requests = None;
                    continue;
                }
                tokio::spawn(crate::session_lock::lock());
            }
        }
    }
}

async fn run_lid(lid: LidAction) -> Result<(), String> {
    if lid == LidAction::LockSuspend {
        return Ok(());
    }
    let connection = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    let upower = zbus::Proxy::new(
        &connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    let mut changes = upower.receive_property_changed::<bool>("LidIsClosed").await;
    let _lid_block = inhibit(
        &manager,
        "handle-lid-switch",
        tr("shell-the-lid-setting-is-handled-by-enzui"),
        "block",
    )
    .await
    .ok_or_else(|| tr("shell-logind-is-not-available"))?;
    while let Some(change) = changes.next().await {
        if lid == LidAction::Lock && change.get().await.unwrap_or(false) {
            let _ = crate::session_lock::lock().await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retries_failed_lock_within_the_delay() {
        let mut calls = 0;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        lock_before_deadline(
            async |remaining| {
                assert!(!remaining.is_zero());
                calls += 1;
                if calls == 1 {
                    Err("startup failed".into())
                } else {
                    Ok(())
                }
            },
            deadline,
        )
        .await
        .unwrap();
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn keeps_the_delay_until_its_deadline_when_lock_start_fails() {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let result = lock_before_deadline(async |_| Err("startup failed".into()), deadline).await;
        assert_eq!(result.unwrap_err(), "startup failed");
        assert!(tokio::time::Instant::now() >= deadline);
    }
}
