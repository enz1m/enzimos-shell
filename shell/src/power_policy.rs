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
        if std::env::var("ENZUI_MANAGED_LOCK").as_deref() == Ok("1") {
            iced::Subscription::run(cover_recovery_stream)
        } else {
            iced::Subscription::none()
        },
    ])
}

fn cover_recovery_stream() -> impl iced::futures::Stream<Item = String> {
    iced::stream::channel(1, async |output| {
        let _output = output;
        let Ok(compositor) = crate::compositor::Compositor::from_environment() else {
            return;
        };
        loop {
            if request_within(
                &compositor,
                &["enzuisleepcover", "recovery"],
                Duration::from_secs(1),
            )
            .await
            .is_ok_and(|status| status.trim() == "needed")
            {
                set_displays(Some(&compositor), true).await;
                attach_resume_locker().await;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}

fn sleep_stream() -> impl iced::futures::Stream<Item = String> {
    use iced::futures::SinkExt;
    iced::stream::channel(1, async |mut output| {
        loop {
            match run_sleep().await {
                Ok(()) => tokio::time::sleep(Duration::from_millis(200)).await,
                Err(error) => {
                    if output.send(error).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cover {
    Ready,
    Unsupported,
}

async fn present_sleep_cover(compositor: &crate::compositor::Compositor) -> Result<Cover, String> {
    let mut status = compositor.request(&["enzuisleepcover", "arm"]).await?;
    if matches!(status.trim(), "unknown request" | "unavailable") {
        return Ok(Cover::Unsupported);
    }
    while status.trim() == "pending" {
        tokio::time::sleep(Duration::from_millis(50)).await;
        status = compositor.request(&["enzuisleepcover", "status"]).await?;
    }
    if status.trim() != "ready" {
        return Err(cover_error());
    }
    Ok(Cover::Ready)
}

fn cover_error() -> String {
    tr("shell-the-lock-cover-was-not-presented").into()
}

async fn request_within(
    compositor: &crate::compositor::Compositor,
    request: &[&str],
    limit: Duration,
) -> Result<String, String> {
    tokio::time::timeout(limit, compositor.request(request))
        .await
        .map_err(|_| cover_error())?
}

async fn cover_within(
    compositor: Option<&crate::compositor::Compositor>,
    limit: Duration,
) -> Result<Cover, String> {
    let compositor = compositor.ok_or_else(cover_error)?;
    tokio::time::timeout(limit, present_sleep_cover(compositor))
        .await
        .map_err(|_| cover_error())?
}

async fn confirm_sleep(
    cover: Result<Cover, String>,
    lock: impl AsyncFnOnce() -> Result<(), String>,
    attest: impl AsyncFnOnce(bool) -> Result<(), String>,
) -> Result<(), String> {
    match cover {
        Ok(Cover::Ready) => attest(false).await,
        Ok(Cover::Unsupported) => {
            lock().await?;
            attest(true).await
        }
        Err(_) => {
            lock().await?;
            attest(false).await
        }
    }
}

pub(crate) async fn prepare_request() -> Result<(), String> {
    let compositor =
        crate::compositor::Compositor::from_environment().map_err(|_| cover_error())?;
    confirm_sleep(
        cover_within(Some(&compositor), Duration::from_secs(2)).await,
        async || crate::session_lock::lock_for_sleep(crate::session_lock::SLEEP_LOCK_LIMIT).await,
        async |legacy| attest_sleep(&compositor, legacy, true).await,
    )
    .await
}

async fn attest_sleep(
    compositor: &crate::compositor::Compositor,
    legacy: bool,
    reserve: bool,
) -> Result<(), String> {
    let (command, expected): (&[&str], _) = if legacy {
        (&["locked"], "true")
    } else if reserve {
        (&["enzuisleepcover", "sleep-ready"], "ready")
    } else {
        (&["enzuisleepcover", "status"], "ready")
    };
    if request_within(compositor, command, Duration::from_secs(1))
        .await?
        .trim()
        == expected
    {
        Ok(())
    } else {
        Err(cover_error())
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
    let mut owners = manager
        .receive_owner_changed()
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
    let mut needs_locker = false;
    let mut covered = false;
    loop {
        tokio::select! {
            _ = owners.next() => return Ok(()),
            signal = sleeping.next() => {
                let Some(signal) = signal else { return Ok(()) };
                let Ok(starting) = signal.body().deserialize::<bool>() else { continue };
                if starting {
                    let deadline = tokio::time::Instant::now() + delay_limit;
                    let cover = cover_within(compositor.as_ref(), delay_limit.min(Duration::from_secs(2))).await;
                    covered = cover == Ok(Cover::Ready);
                    let acknowledged = if covered {
                        needs_locker = true;
                        let _ = tokio::time::timeout_at(deadline, set_displays(compositor.as_ref(), false)).await;
                        true
                    } else {
                        needs_locker = match lock_before_deadline(crate::session_lock::lock_for_sleep, deadline).await {
                            Ok(()) => {
                                let _ = tokio::time::timeout_at(deadline, set_displays(compositor.as_ref(), false)).await;
                                false
                            }
                            Err(error) => {
                                eprintln!("enzui-shell: lock before sleep: {error}");
                                true
                            }
                        };
                        !needs_locker
                    };
                    if acknowledged
                        && let Some(compositor) = compositor.as_ref()
                        && tokio::time::timeout_at(deadline, attest_sleep(compositor, cover == Ok(Cover::Unsupported), false))
                            .await.is_ok_and(|status| status.is_ok())
                    {
                        delay = None;
                    }
                } else {
                    if delay.is_none() {
                        delay = inhibit(&manager, "sleep", tr("shell-locking-the-screen-before-sleep"), "delay").await;
                    }
                    let needs_locker = std::mem::take(&mut needs_locker);
                    let covered = std::mem::take(&mut covered);
                    if needs_locker && !covered {
                        attach_resume_locker().await;
                    }
                    set_displays(compositor.as_ref(), true).await;
                    if needs_locker && covered {
                        tokio::spawn(attach_resume_locker());
                    }
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

async fn attach_resume_locker() {
    if let Err(error) =
        crate::session_lock::lock_for_sleep(crate::session_lock::SLEEP_LOCK_LIMIT).await
    {
        eprintln!("enzui-shell: lock on resume: {error}");
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
    async fn sleep_requires_current_compositor_confirmation() {
        let legacy = confirm_sleep(
            Ok(Cover::Unsupported),
            async || Ok(()),
            async |legacy| {
                assert!(legacy);
                Ok(())
            },
        );
        assert!(legacy.await.is_ok());

        let rejected = confirm_sleep(
            Err("IPC timeout".into()),
            async || Err("rejected".into()),
            async |_| panic!("locker rejection must abort suspend"),
        );
        assert_eq!(rejected.await.unwrap_err(), "rejected");

        let unconfirmed = confirm_sleep(
            Err("IPC timeout".into()),
            async || Ok(()),
            async |legacy| {
                assert!(!legacy);
                Err("pending".into())
            },
        );
        assert_eq!(unconfirmed.await.unwrap_err(), "pending");

        let concurrent = confirm_sleep(
            Ok(Cover::Ready),
            async || panic!("an independently confirmed cover needs no second locker"),
            async |legacy| {
                assert!(!legacy);
                Ok(())
            },
        );
        assert!(concurrent.await.is_ok());
    }

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
