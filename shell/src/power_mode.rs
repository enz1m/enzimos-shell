//! Power-profile client supporting either power-profiles-daemon bus name.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use zbus::zvariant::OwnedValue;

// Newer and older names of the same service.
const SERVICES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    PowerSaver,
    Balanced,
    Performance,
}

impl Profile {
    pub const ALL: [Self; 3] = [Self::PowerSaver, Self::Balanced, Self::Performance];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::PowerSaver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::PowerSaver => tr("shell-power-saver"),
            Self::Balanced => tr("shell-balanced"),
            Self::Performance => tr("shell-performance"),
        }
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::PowerSaver => tr("shell-lower-performance-longer-battery-life"),
            Self::Balanced => tr("shell-standard-performance-and-power-use"),
            Self::Performance => tr("shell-highest-performance-more-power-and-heat"),
        }
    }

    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::PowerSaver => "leaf",
            Self::Balanced => "gauge",
            Self::Performance => "zap",
        }
    }

    fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|profile| profile.key() == key)
    }
}

/// The active mode and the modes this computer offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerMode {
    pub active: Profile,
    pub available: Vec<Profile>,
}

async fn service(connection: &zbus::Connection) -> Option<zbus::Proxy<'static>> {
    for (name, path, interface) in SERVICES {
        let proxy = zbus::Proxy::new(connection, name, path, interface)
            .await
            .ok()?;
        if proxy.get_property::<String>("ActiveProfile").await.is_ok() {
            return Some(proxy);
        }
    }
    None
}

/// The power mode, or `None` without power-profiles-daemon or when it does
/// not answer within [`crate::bus::READ`].
pub async fn read() -> Option<PowerMode> {
    read_on(&crate::bus::bounded(crate::bus::system()).await??).await
}

async fn read_on(connection: &zbus::Connection) -> Option<PowerMode> {
    crate::bus::bounded(read_now(connection)).await?
}

async fn read_now(connection: &zbus::Connection) -> Option<PowerMode> {
    let proxy = service(connection).await?;
    let active = Profile::parse(&proxy.get_property::<String>("ActiveProfile").await.ok()?)?;
    let profiles: Vec<std::collections::HashMap<String, OwnedValue>> =
        proxy.get_property("Profiles").await.unwrap_or_default();
    let mut available: Vec<Profile> = profiles
        .iter()
        .filter_map(|entry| String::try_from(entry.get("Profile")?.clone()).ok())
        .filter_map(|key| Profile::parse(&key))
        .collect();
    if available.is_empty() {
        available = vec![active];
    }
    available.sort_by_key(|profile| Profile::ALL.iter().position(|p| p == profile));
    available.dedup();
    Some(PowerMode { active, available })
}

/// Makes `profile` the active mode.
pub async fn set(profile: Profile) -> Result<(), String> {
    let connection = crate::bus::bounded(crate::bus::system())
        .await
        .flatten()
        .ok_or_else(|| tr("shell-no-system-bus"))?;
    let failed = || tr!("shell-could-not-be-turned-on", "arg0" => profile.label().to_owned());
    crate::bus::bounded(async {
        let proxy = service(&connection)
            .await
            .ok_or_else(|| tr("shell-power-modes-are-not-available").to_owned())?;
        proxy
            .set_property("ActiveProfile", profile.key())
            .await
            .map_err(|_| failed())
    })
    .await
    .unwrap_or_else(|| Err(failed()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profiles(bool);
    #[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
    impl Profiles {
        #[zbus(property)]
        async fn active_profile(&self) -> String {
            if self.0 {
                std::future::pending::<()>().await;
            }
            "balanced".into()
        }
    }

    /// A power-profiles service that never answers leaves the power mode
    /// unknown within the read bound instead of holding the control
    /// center's refresh; one that answers is read.
    ///
    /// Sabotage: call `read_now` directly in `read_on`.
    #[tokio::test]
    async fn a_stuck_power_profiles_service_does_not_hold_the_reader() {
        let bus = crate::test_bus::Bus::start();
        let _service = bus
            .serve(SERVICES[0].0, SERVICES[0].1, Profiles(false))
            .await;
        let mode = read_on(&bus.client().await).await.unwrap();
        assert_eq!(mode.active, Profile::Balanced);
        let stuck = crate::test_bus::Bus::start();
        let _service = stuck
            .serve(SERVICES[0].0, SERVICES[0].1, Profiles(true))
            .await;
        let client = stuck.client().await;
        let read = tokio::time::timeout(crate::bus::READ * 2, read_on(&client)).await;
        assert_eq!(read, Ok(None));
    }
}
