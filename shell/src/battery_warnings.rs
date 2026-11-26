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
    pub fn step(
        &mut self,
        battery: Option<&Battery>,
        upower_level: Option<u32>,
    ) -> Option<Warning> {
        let battery = battery?;
        if battery.external_power || battery.charging {
            self.given = 0;
            return None;
        }
        let percent = battery.percent;
        let (level, warning) = match (upower_level, percent) {
            (Some(5), _) => (3, Warning::Critical(percent)),
            (Some(4), _) | (_, 0..=5) => (2, Warning::VeryLow(percent)),
            (Some(3), _) | (_, 6..=10) => (1, Warning::Low(percent)),
            (_, 16..=100) => {
                self.given = 0;
                return None;
            }
            _ => return None,
        };
        (level > self.given).then(|| {
            self.given = level;
            warning
        })
    }
}

/// The aggregate power source and warning level used by UPower.
pub(crate) async fn reading() -> Option<(Battery, u32)> {
    crate::bus::bounded(async {
        let system = crate::bus::system().await?;
        let upower = manager(&system).await?;
        let device = zbus::fdo::PropertiesProxy::builder(&system)
            .destination("org.freedesktop.UPower")
            .ok()?
            .path("/org/freedesktop/UPower/devices/DisplayDevice")
            .ok()?
            .build()
            .await
            .ok()?;
        let (properties, on_battery) = tokio::join!(
            device.get_all("org.freedesktop.UPower.Device".try_into().ok()?),
            upower.get_property::<bool>("OnBattery"),
        );
        let properties = properties.ok()?;
        if !bool::try_from(properties.get("IsPresent")?).ok()? {
            return None;
        }
        let level = u32::try_from(properties.get("WarningLevel")?).ok()?;
        let percent = f64::try_from(properties.get("Percentage")?).ok()?;
        let state = u32::try_from(properties.get("State")?).ok()?;
        if !(0.0..=100.0).contains(&percent) {
            return None;
        }
        Some((
            Battery {
                percent: percent.round() as u8,
                charging: state == 1,
                external_power: !on_battery.ok()?,
            },
            level,
        ))
    })
    .await
    .flatten()
}

async fn manager(system: &zbus::Connection) -> Option<zbus::Proxy<'static>> {
    zbus::Proxy::new_owned(
        system.clone(),
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .ok()
}

/// The effective action reported by UPower, if it will take one.
pub async fn critical_action() -> Option<&'static str> {
    let action: Option<String> = crate::bus::bounded(async {
        let system = crate::bus::system().await?;
        let upower = manager(&system).await?;
        upower.call("GetCriticalAction", &()).await.ok()
    })
    .await
    .flatten();
    action_label(action.as_deref())
}

fn action_label(action: Option<&str>) -> Option<&'static str> {
    Some(match action? {
        "HybridSleep" => tr("shell-enter-hybrid-sleep"),
        "Hibernate" => tr("shell-hibernate-action"),
        "Suspend" => tr("shell-suspend-action"),
        "PowerOff" => tr("shell-power-off"),
        _ => return None,
    })
}

/// The notification for `warning`: summary, body and whether it is urgent.
#[must_use]
pub fn message(warning: Warning, action: Option<&str>) -> (String, String, bool) {
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
            action.map_or_else(
                || tr("shell-plug-in-the-charger-and-save-your-work").into(),
                |action| tr!("shell-the-computer-will-very-soon-unless-it-is-plugged-in-save-your-work", "arg0" => action.to_owned()),
            ),
            true,
        ),
    }
}

/// Posts a notification through the desktop's own notification service.
pub async fn notify(summary: String, body: String, urgent: bool) -> Result<(), String> {
    notify_power("battery-low", summary, body, urgent).await
}

pub(crate) async fn notify_power(
    icon: &str,
    summary: String,
    body: String,
    urgent: bool,
) -> Result<(), String> {
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
                icon,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_follows_upower_and_resets_when_external_power_connects() {
        let mut warnings = Warnings::default();
        let mut battery = Battery {
            percent: 28,
            charging: false,
            external_power: false,
        };
        assert_eq!(
            warnings.step(Some(&battery), Some(3)),
            Some(Warning::Low(28))
        );
        assert_eq!(
            warnings.step(Some(&battery), Some(4)),
            Some(Warning::VeryLow(28))
        );
        assert_eq!(
            warnings.step(Some(&battery), Some(5)),
            Some(Warning::Critical(28))
        );
        assert_eq!(warnings.step(Some(&battery), Some(5)), None);
        battery.external_power = true;
        battery.percent = 2;
        assert_eq!(warnings.step(Some(&battery), None), None);
        battery.external_power = false;
        assert_eq!(
            warnings.step(Some(&battery), None),
            Some(Warning::VeryLow(2))
        );
        assert_eq!(
            warnings.step(Some(&battery), Some(5)),
            Some(Warning::Critical(2))
        );
    }

    #[test]
    fn missing_or_ignored_action_does_not_promise_shutdown() {
        assert_eq!(action_label(None), None);
        assert_eq!(action_label(Some("Ignore")), None);
        assert!(action_label(Some("PowerOff")).is_some());
    }
}
