//! Locks with compositor acknowledgement before requesting a greeter on another VT.

use enzui_shell::i18n::tr;
const SERVICE: &str = "org.enzimos.UserSwitch1";

async fn bus() -> Option<zbus::Proxy<'static>> {
    let system = zbus::Connection::system().await.ok()?;
    zbus::Proxy::new_owned(
        system,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .ok()
}

/// Whether the dispatcher is installed (running or activatable).
pub async fn available() -> bool {
    let Some(bus) = bus().await else { return false };
    for method in ["ListActivatableNames", "ListNames"] {
        if let Ok(names) = bus.call::<_, _, Vec<String>>(method, &()).await
            && names.iter().any(|name| name == SERVICE)
        {
            return true;
        }
    }
    false
}

/// Locks, then opens a greeter for another user.
pub async fn switch() -> Result<(), String> {
    if let Err(error) = crate::session_lock::lock().await {
        // A slow lock screen still counts once the compositor acknowledges it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !crate::session_lock::locked() {
            if std::time::Instant::now() >= deadline {
                return Err(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let dispatcher = zbus::Proxy::new(&system, SERVICE, "/org/enzimos/UserSwitch1", SERVICE)
        .await
        .map_err(|_| tr("shell-user-switching-is-not-available"))?;
    dispatcher
        .call::<_, _, ()>("OpenGreeter", &())
        .await
        .map_err(|_| tr("shell-another-user-cannot-sign-in-right-now").to_owned())
}
