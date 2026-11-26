//! Thermal pressure from power-profiles-daemon and kernel thermal zones.

use crate::i18n::tr;
use iced::Subscription;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(5);
const COOLDOWN: Duration = Duration::from_secs(300);
const CRITICAL_MARGIN: i64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    Throttled,
    Critical,
}

impl Level {
    pub fn title(self) -> &'static str {
        match self {
            Self::Normal => tr("shell-temperature-normal"),
            Self::Throttled => tr("shell-performance-limited-by-heat"),
            Self::Critical => tr("shell-temperature-near-critical"),
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Normal => tr("shell-no-thermal-warning"),
            Self::Throttled => tr("shell-the-system-is-reducing-performance-to-cool-down"),
            Self::Critical => tr("shell-save-your-work-and-let-the-computer-cool-down"),
        }
    }
}

fn temperature(path: &Path) -> Option<i64> {
    let value = fs::read_to_string(path).ok()?.trim().parse().ok()?;
    (-50_000..=250_000).contains(&value).then_some(value)
}

fn zone_level(zone: &Path) -> Option<Level> {
    if fs::read_to_string(zone.join("mode")).is_ok_and(|mode| mode.trim() == "disabled") {
        return None;
    }
    let current = temperature(&zone.join("temp"))?;
    fs::read_dir(zone)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let stem = name.to_str()?.strip_suffix("_type")?;
            let trip = stem.strip_prefix("trip_point_")?;
            trip.parse::<u32>().ok()?;
            if fs::read_to_string(entry.path()).ok()?.trim() != "critical" {
                return None;
            }
            let threshold = temperature(&zone.join(format!("{stem}_temp")))?;
            (threshold > 0).then_some(if current >= threshold - CRITICAL_MARGIN {
                Level::Critical
            } else {
                Level::Normal
            })
        })
        .max()
}

fn zones(root: &Path) -> Option<Level> {
    fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|zone| {
            zone.file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("thermal_zone"))
                .is_some_and(|index| index.parse::<u32>().is_ok())
        })
        .filter_map(|zone| zone_level(&zone.path()))
        .max()
}

pub(crate) async fn read() -> Option<Level> {
    let (zones, degraded) = tokio::join!(
        tokio::task::spawn_blocking(|| zones(Path::new("/sys/class/thermal"))),
        crate::power_mode::thermal_degradation()
    );
    let profile = degraded.map(|degraded| {
        if degraded {
            Level::Throttled
        } else {
            Level::Normal
        }
    });
    zones.ok().flatten().max(profile)
}

#[derive(Default)]
struct Warnings {
    announced: Option<Level>,
    next_allowed: [Option<Instant>; 2],
}

impl Warnings {
    fn next(&mut self, level: Option<Level>, now: Instant) -> Option<Level> {
        let level = level?;
        let index = match level {
            Level::Normal => {
                self.announced = None;
                return None;
            }
            Level::Throttled => 0,
            Level::Critical => 1,
        };
        if self.announced >= Some(level)
            || self.next_allowed[index].is_some_and(|after| now < after)
        {
            return None;
        }
        self.next_allowed[index] = Some(now + COOLDOWN);
        Some(level)
    }

    fn delivered(&mut self, level: Level) {
        self.announced = Some(level);
    }

    fn retry(&mut self, level: Level) {
        self.next_allowed[usize::from(level == Level::Critical)] =
            Some(Instant::now() + Duration::from_secs(30));
    }
}

/// Posts a normal notice for heat-limited performance and a critical notice near a trip point.
pub fn subscription() -> Subscription<String> {
    Subscription::run(notices)
}

fn notices() -> impl iced::futures::Stream<Item = String> {
    use iced::futures::SinkExt;
    iced::stream::channel(1, async |mut errors| {
        let mut warnings = Warnings::default();
        loop {
            if let Some(level) = warnings.next(read().await, Instant::now()) {
                let result = crate::bus::bounded(crate::battery_warnings::notify_power(
                    "thermometer",
                    level.title().into(),
                    level.detail().into(),
                    level == Level::Critical,
                ))
                .await;
                match result {
                    Some(Ok(())) => warnings.delivered(level),
                    _ => {
                        warnings.retry(level);
                        let error = result.and_then(Result::err).unwrap_or_else(|| {
                            tr("shell-the-notification-was-not-accepted").into()
                        });
                        if errors.send(error).await.is_err() {
                            return;
                        }
                    }
                }
            }
            tokio::time::sleep(POLL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_near_a_zones_own_critical_trip_point() {
        let sys = tempfile::tempdir().unwrap();
        let zone = sys.path().join("thermal_zone0");
        fs::create_dir(&zone).unwrap();
        fs::write(zone.join("trip_point_3_type"), "critical\n").unwrap();
        fs::write(zone.join("trip_point_3_temp"), "95000\n").unwrap();
        fs::write(zone.join("trip_point_4_type"), "passive\n").unwrap();
        fs::write(zone.join("trip_point_4_temp"), "85000\n").unwrap();
        for (temp, expected) in [
            ("not available", None),
            ("89000", Some(Level::Normal)),
            ("90000", Some(Level::Critical)),
        ] {
            fs::write(zone.join("temp"), temp).unwrap();
            assert_eq!(zones(sys.path()), expected);
        }
        fs::write(zone.join("mode"), "disabled").unwrap();
        assert_eq!(zones(sys.path()), None);
    }

    #[test]
    fn escalation_bypasses_throttling_cooldown_and_recovery_rearms_notices() {
        let mut warnings = Warnings::default();
        let start = Instant::now();
        assert_eq!(
            warnings.next(Some(Level::Throttled), start),
            Some(Level::Throttled)
        );
        assert_eq!(
            warnings.next(Some(Level::Critical), start),
            Some(Level::Critical)
        );
        warnings.delivered(Level::Critical);
        assert_eq!(warnings.next(None, start), None);
        assert_eq!(warnings.next(Some(Level::Critical), start + COOLDOWN), None);
        assert_eq!(warnings.next(Some(Level::Normal), start + COOLDOWN), None);
        assert_eq!(
            warnings.next(Some(Level::Throttled), start + COOLDOWN),
            Some(Level::Throttled)
        );
        assert_eq!(warnings.next(Some(Level::Normal), start + COOLDOWN), None);
        assert_eq!(
            warnings.next(Some(Level::Throttled), start + COOLDOWN),
            None
        );
    }
}
