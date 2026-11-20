//! Discharge-level battery warnings; critical power actions remain UPower's responsibility.

use crate::system_controls::Battery;
use enzui_shell::i18n::tr;
use enzui_shell::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    Low(u8),
    VeryLow(u8),
    Critical(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Warnings {
    // The most severe level already announced this discharge (1..=3).
    given: u8,
}

impl Warnings {
    /// The warning a new reading calls for, if any.
    pub fn step(&mut self, battery: Option<&Battery>) -> Option<Warning> {
        let battery = battery?;
        if battery.charging || battery.percent > 15 {
            self.given = 0;
            return None;
        }
        let (level, warning) = match battery.percent {
            0..=3 => (3, Warning::Critical(battery.percent)),
            4..=5 => (2, Warning::VeryLow(battery.percent)),
            6..=10 => (1, Warning::Low(battery.percent)),
            _ => return None,
        };
        (level > self.given).then(|| {
            self.given = level;
            warning
        })
    }
}

/// What UPower does at the critical level, in words.
pub async fn critical_action() -> &'static str {
    let action: Option<String> = crate::bus::bounded(async {
        let system = crate::bus::system().await?;
        let upower = zbus::Proxy::new(
            &system,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
        )
        .await
        .ok()?;
        upower.call("GetCriticalAction", &()).await.ok()
    })
    .await
    .flatten();
    match action.as_deref() {
        Some("HybridSleep" | "Hibernate") => "hibernate",
        Some("Suspend") => "suspend",
        Some("PowerOff") => tr("shell-power-off"),
        _ => tr("shell-shut-down"),
    }
}

/// The notification for `warning`: summary, body and whether it is urgent.
#[must_use]
pub fn message(warning: Warning, action: &str) -> (String, String, bool) {
    match warning {
        Warning::Low(percent) => (
            tr!("shell-battery-low-2", "arg0" => format!("{:}", percent)),
            tr("shell-plug-in-the-charger-soon").into(),
            false,
        ),
        Warning::VeryLow(percent) => (
            tr!("shell-battery-very-low", "arg0" => format!("{:}", percent)),
            tr("shell-plug-in-the-charger-now-to-keep-working").into(),
            true,
        ),
        Warning::Critical(percent) => (
            tr!("shell-battery-critical-2", "arg0" => format!("{:}", percent)),
            tr!("shell-the-computer-will-very-soon-unless-it-is-plugged-in-save-your-work", "arg0" => action.to_owned()),
            true,
        ),
    }
}

/// Posts a notification through the desktop's own notification service.
pub async fn notify(summary: String, body: String, urgent: bool) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    .map_err(|_| tr("shell-no-notification-service"))?;
    let mut hints = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    hints.insert(
        "urgency",
        zbus::zvariant::Value::U8(if urgent { 2 } else { 1 }),
    );
    hints.insert("category", zbus::zvariant::Value::from("device"));
    let actions: Vec<&str> = Vec::new();
    let _: u32 = proxy
        .call(
            "Notify",
            &(
                "Power",
                0u32,
                "battery-low",
                summary.as_str(),
                body.as_str(),
                actions,
                hints,
                if urgent { 0i32 } else { 10_000i32 },
            ),
        )
        .await
        .map_err(|_| tr("shell-the-notification-was-not-accepted"))?;
    Ok(())
}
