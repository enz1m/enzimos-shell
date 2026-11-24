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

/// Holds the policy for `lid`; changing it takes the inhibitors afresh.
pub fn subscription(lid: LidAction) -> iced::Subscription<String> {
    iced::Subscription::run_with(lid, stream)
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "Subscription::run_with borrows the concrete hashed input."
)]
fn stream(lid: &LidAction) -> impl iced::futures::Stream<Item = String> + use<> {
    use iced::futures::SinkExt;
    let lid = *lid;
    iced::stream::channel(1, async move |mut output| {
        if let Err(error) = run(lid).await {
            let _ = output.send(error).await;
        }
    })
}

async fn run(lid: LidAction) -> Result<(), String> {
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
    let upower = zbus::Proxy::new(
        &connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .ok();
    let mut lid_changes = match &upower {
        Some(p) => Some(p.receive_property_changed::<bool>("LidIsClosed").await),
        None => None,
    };

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
    // Keeps logind from suspending on the lid when the preference says not to.
    let _lid_block = if lid == LidAction::LockSuspend {
        None
    } else {
        inhibit(
            &manager,
            "handle-lid-switch",
            tr("shell-the-lid-setting-is-handled-by-enzui"),
            "block",
        )
        .await
    };
    loop {
        tokio::select! {
            signal = sleeping.next() => {
                let Some(signal) = signal else { return Ok(()) };
                let starting: bool = signal.body().deserialize().unwrap_or(false);
                if starting {
                    let _ = crate::session_lock::lock_within(delay_limit).await;
                    delay = None;
                } else if delay.is_none() {
                    delay = inhibit(&manager, "sleep", tr("shell-locking-the-screen-before-sleep"), "delay").await;
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
                let _ = crate::session_lock::lock().await;
            }
            closed = async {
                match lid_changes.as_mut() {
                    Some(changes) => changes.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(change) = closed else { lid_changes = None; continue };
                if change.get().await.unwrap_or(false) && lid == LidAction::Lock {
                    let _ = crate::session_lock::lock().await;
                }
            }
        }
    }
}
