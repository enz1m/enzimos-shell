//! Account-selected IANA world clocks resolved through system time-zone data.

use enzui_shell::i18n::tr;
use std::path::Path;

/// At most this many clocks are kept.
pub const MAX: usize = 6;

/// Whether `zone` names an installed IANA time zone.
#[must_use]
pub fn valid(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && zone.contains('/')
        && !zone.contains("..")
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
        && Path::new("/usr/share/zoneinfo").join(zone).is_file()
}

/// "New York" from "America/New_York".
#[must_use]
pub fn city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTime {
    pub city: String,
    pub time: String,
    /// "Tomorrow" or "Yesterday" when the date differs from here.
    pub day: Option<&'static str>,
}

/// The time in each of `zones` now. Blocking (one short `date` per zone).
#[must_use]
pub fn now(zones: &[String], h24: bool) -> Vec<WorldTime> {
    let here = chrono::Local::now().date_naive();
    let format = "+%Y-%m-%d %H:%M";
    zones
        .iter()
        .filter(|zone| valid(zone))
        .filter_map(|zone| {
            let output = std::process::Command::new("date")
                .arg(format)
                .env("TZ", zone)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            let text = String::from_utf8(output.stdout).ok()?;
            let (date, time) = text.trim().split_once(' ')?;
            let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
            let day = match date.signed_duration_since(here).num_days() {
                1 => Some(tr("shell-tomorrow")),
                -1 => Some(tr("shell-yesterday")),
                _ => None,
            };
            let parsed = chrono::NaiveTime::parse_from_str(time, "%H:%M").ok()?;
            let date_time = date.and_time(parsed).and_utc();
            Some(WorldTime {
                city: city(zone),
                time: crate::i18n::time(&date_time, h24),
                day,
            })
        })
        .collect()
}
