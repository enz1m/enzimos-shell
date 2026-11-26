//! When to raise the energy-performance preference, and how to put it back.
//!
//! A boost writes `performance` to every policy that is not already there and
//! records the replaced values in a marker file first. Ending a boost, or
//! recovering one after a crash, restores a policy only while it still holds
//! the boost's value and power-profiles-daemon's profile is unchanged, so a
//! value another manager wrote meanwhile stays.

use crate::cpu::{self, PERFORMANCE, POWER, Policy};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long one request keeps the preference raised.
pub const DURATION: Duration = Duration::from_millis(1500);

const MARKER_HEADER: &str = "enzui-power-boost 1";
const PERFORMANCE_PROFILE: &str = "performance";
const POWER_SAVER_PROFILE: &str = "power-saver";

/// The answer to one boost request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The preference is raised.
    Boosted,
    /// A running boost lasts [`DURATION`] from now.
    Extended,
    /// The system already runs at its highest preference or governor.
    Performance,
    /// A power-saving choice is in effect and is left alone.
    PowerSaving,
    /// No cpufreq policy offers a `performance` preference.
    Unsupported,
    /// A daemon that rewrites the preference continuously is running.
    OtherManager,
    /// Another writer replaced a recent boost's value; boosts pause.
    Contended,
    /// The helper is stopping or rate-limiting this caller.
    Busy,
    /// The caller is not in the active local session.
    Denied,
    /// Reading or writing the preference failed; nothing stays changed.
    Failed,
}

impl Outcome {
    /// The wire form of the outcome.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Boosted => "boosted",
            Self::Extended => "extended",
            Self::Performance => "performance",
            Self::PowerSaving => "power-saving",
            Self::Unsupported => "unsupported",
            Self::OtherManager => "other-manager",
            Self::Contended => "contended",
            Self::Busy => "busy",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// Host facts read before a new boost starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Host {
    /// power-profiles-daemon's (or tuned-ppd's) active profile, when one runs.
    pub profile: Option<String>,
    /// A daemon that rewrites the preference in a loop is running.
    pub other_manager: bool,
}

/// How a boost ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Every boosted policy has its previous preference again.
    Restored,
    /// The power profile changed during the boost; its values stay.
    ProfileChanged,
    /// Another writer replaced a boosted value, which stays.
    Replaced,
}

struct Active {
    until: Instant,
    profile: Option<String>,
    previous: Vec<(Policy, String)>,
}

/// Boost state for one cpufreq root and its crash marker.
pub struct Booster {
    root: PathBuf,
    marker: PathBuf,
    active: Option<Active>,
}

impl Booster {
    /// Boosts below `root` (normally [`cpu::CPUFREQ`]), recording them in `marker`.
    #[must_use]
    pub fn new(root: PathBuf, marker: PathBuf) -> Self {
        Self {
            root,
            marker,
            active: None,
        }
    }

    /// When the running boost ends, if one runs.
    #[must_use]
    pub fn until(&self) -> Option<Instant> {
        self.active.as_ref().map(|active| active.until)
    }

    /// Moves the end of a running boost to [`DURATION`] after `now`.
    pub fn extend(&mut self, now: Instant) -> bool {
        self.active
            .as_mut()
            .map(|active| active.until = now + DURATION)
            .is_some()
    }

    /// Starts a boost unless `host` or the current policies rule it out.
    pub fn start(&mut self, now: Instant, host: &Host) -> Outcome {
        if self.extend(now) {
            return Outcome::Extended;
        }
        let policies = cpu::policies(&self.root);
        if policies.is_empty() || !policies.iter().all(Policy::offers_performance) {
            return Outcome::Unsupported;
        }
        if host.other_manager {
            return Outcome::OtherManager;
        }
        match host.profile.as_deref() {
            Some(PERFORMANCE_PROFILE) => return Outcome::Performance,
            Some(POWER_SAVER_PROFILE) => return Outcome::PowerSaving,
            _ => {}
        }
        let mut previous = Vec::new();
        for policy in policies {
            if policy.governor().as_deref() == Some(PERFORMANCE) {
                return Outcome::Performance;
            }
            let Ok(value) = policy.preference() else {
                return Outcome::Failed;
            };
            match value.as_str() {
                PERFORMANCE => {}
                POWER => return Outcome::PowerSaving,
                _ if token(&value) => previous.push((policy, value)),
                _ => return Outcome::Failed,
            }
        }
        if previous.is_empty() {
            return Outcome::Performance;
        }
        if write_marker(&self.marker, host.profile.as_deref(), &previous).is_err() {
            return Outcome::Failed;
        }
        for (written, (policy, _)) in previous.iter().enumerate() {
            if policy.set_preference(PERFORMANCE).is_err() {
                let profile = host.profile.as_deref();
                restore(&previous[..written], profile, profile);
                let _ = std::fs::remove_file(&self.marker);
                return Outcome::Failed;
            }
        }
        self.active = Some(Active {
            until: now + DURATION,
            profile: host.profile.clone(),
            previous,
        });
        Outcome::Boosted
    }

    /// Ends the running boost given the profile active now.
    pub fn finish(&mut self, profile: Option<&str>) -> Option<Ending> {
        let active = self.active.take()?;
        let ending = restore(&active.previous, active.profile.as_deref(), profile);
        let _ = std::fs::remove_file(&self.marker);
        Some(ending)
    }

    /// Ends a boost a previous helper process left behind in the marker.
    pub fn recover(&self, profile: Option<&str>) -> Option<Ending> {
        let text = std::fs::read_to_string(&self.marker).ok()?;
        let _ = std::fs::remove_file(&self.marker);
        let (started, previous) = parse_marker(&self.root, &text)?;
        Some(restore(&previous, started.as_deref(), profile))
    }
}

fn restore(previous: &[(Policy, String)], started: Option<&str>, now: Option<&str>) -> Ending {
    // An unreadable or stopped profile service leaves the values as the last
    // test: a stopped daemon no longer rewrites them, and keeping `performance`
    // would last until something else wrote the preference.
    let changed = match (started, now) {
        (Some(started), Some(now)) => started != now,
        (None, Some(_)) => true,
        (_, None) => false,
    };
    if changed {
        return Ending::ProfileChanged;
    }
    let mut ending = Ending::Restored;
    for (policy, value) in previous {
        // A performance governor pins the preference; a manager chose it during the boost.
        let still_ours = policy.governor().as_deref() != Some(PERFORMANCE)
            && policy
                .preference()
                .is_ok_and(|current| current == PERFORMANCE);
        if !still_ours || policy.set_preference(value).is_err() {
            ending = Ending::Replaced;
        }
    }
    ending
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

fn write_marker(
    marker: &Path,
    profile: Option<&str>,
    previous: &[(Policy, String)],
) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let mut text = format!("{MARKER_HEADER}\nprofile {}\n", profile.unwrap_or(""));
    for (policy, value) in previous {
        let _ = writeln!(text, "{} {value}", policy.name());
    }
    let partial = marker.with_extension("partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, marker)
}

type Marker = (Option<String>, Vec<(Policy, String)>);

fn parse_marker(root: &Path, text: &str) -> Option<Marker> {
    let mut lines = text.lines();
    (lines.next()? == MARKER_HEADER).then_some(())?;
    let profile = match lines.next()?.strip_prefix("profile ")? {
        "" => None,
        name if token(name) => Some(name.to_owned()),
        _ => return None,
    };
    let previous = lines
        .map(|line| {
            let (name, value) = line.split_once(' ')?;
            token(value).then_some(())?;
            Some((Policy::named(root, name)?, value.to_owned()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((profile, previous))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::fake;

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(policies: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("cpufreq")).unwrap();
            let fixture = Self { dir };
            for (number, (preference, governor)) in policies.iter().enumerate() {
                fake::policy(&fixture.root(), number as u32, preference, governor);
            }
            fixture
        }
        fn root(&self) -> PathBuf {
            self.dir.path().join("cpufreq")
        }
        fn marker(&self) -> PathBuf {
            self.dir.path().join("boost")
        }
        fn booster(&self) -> Booster {
            Booster::new(self.root(), self.marker())
        }
        fn preferences(&self, count: u32) -> Vec<String> {
            (0..count)
                .map(|n| fake::preference(&self.root(), n))
                .collect()
        }
    }

    fn balanced() -> Host {
        Host {
            profile: Some("balanced".into()),
            other_manager: false,
        }
    }

    #[test]
    fn a_boost_raises_every_policy_and_restores_the_values_it_replaced() {
        let fixture = Fixture::new(&[
            ("balance_power", "powersave"),
            ("balance_performance", "powersave"),
        ]);
        let mut booster = fixture.booster();
        let now = Instant::now();
        assert_eq!(booster.start(now, &balanced()), Outcome::Boosted);
        assert_eq!(fixture.preferences(2), ["performance", "performance"]);
        assert!(fixture.marker().is_file());
        assert_eq!(booster.until(), Some(now + DURATION));
        assert_eq!(booster.finish(Some("balanced")), Some(Ending::Restored));
        assert_eq!(
            fixture.preferences(2),
            ["balance_power", "balance_performance"]
        );
        assert!(!fixture.marker().exists());
        assert_eq!(booster.finish(Some("balanced")), None);
    }

    #[test]
    fn repeated_requests_extend_a_running_boost() {
        let fixture = Fixture::new(&[("balance_power", "powersave")]);
        let mut booster = fixture.booster();
        let now = Instant::now();
        assert!(!booster.extend(now));
        assert_eq!(booster.start(now, &balanced()), Outcome::Boosted);
        let later = now + Duration::from_millis(900);
        assert_eq!(booster.start(later, &balanced()), Outcome::Extended);
        assert_eq!(booster.until(), Some(later + DURATION));
    }

    #[test]
    fn performance_and_power_saving_choices_are_left_alone() {
        type Case<'a> = (&'a [(&'a str, &'a str)], Option<&'a str>, Outcome);
        let cases: [Case<'_>; 6] = [
            (
                &[("balance_power", "powersave")],
                Some("performance"),
                Outcome::Performance,
            ),
            (
                &[("balance_power", "powersave")],
                Some("power-saver"),
                Outcome::PowerSaving,
            ),
            (
                &[
                    ("balance_power", "powersave"),
                    ("balance_power", "performance"),
                ],
                None,
                Outcome::Performance,
            ),
            (
                &[("performance", "powersave"), ("performance", "powersave")],
                None,
                Outcome::Performance,
            ),
            (&[("power", "powersave")], None, Outcome::PowerSaving),
            (&[], None, Outcome::Unsupported),
        ];
        for (policies, profile, expected) in cases {
            let fixture = Fixture::new(policies);
            let host = Host {
                profile: profile.map(str::to_owned),
                other_manager: false,
            };
            assert_eq!(fixture.booster().start(Instant::now(), &host), expected);
            let before: Vec<&str> = policies.iter().map(|(p, _)| *p).collect();
            assert_eq!(fixture.preferences(policies.len() as u32), before);
            assert!(!fixture.marker().exists());
        }
    }

    #[test]
    fn a_continuous_manager_or_missing_performance_preference_prevents_a_boost() {
        let fixture = Fixture::new(&[("balance_power", "powersave")]);
        let host = Host {
            other_manager: true,
            ..balanced()
        };
        assert_eq!(
            fixture.booster().start(Instant::now(), &host),
            Outcome::OtherManager
        );
        fake::set(
            &fixture.root(),
            0,
            "energy_performance_available_preferences",
            "default balance_power power\n",
        );
        assert_eq!(
            fixture.booster().start(Instant::now(), &balanced()),
            Outcome::Unsupported
        );
        assert_eq!(fixture.preferences(1), ["balance_power"]);
    }

    #[test]
    fn only_policies_below_performance_are_boosted_and_restored() {
        let fixture = Fixture::new(&[("performance", "powersave"), ("balance_power", "powersave")]);
        let mut booster = fixture.booster();
        assert_eq!(booster.start(Instant::now(), &balanced()), Outcome::Boosted);
        assert_eq!(
            std::fs::read_to_string(fixture.marker()).unwrap(),
            "enzui-power-boost 1\nprofile balanced\npolicy1 balance_power\n"
        );
        booster.finish(Some("balanced"));
        assert_eq!(fixture.preferences(2), ["performance", "balance_power"]);
    }

    #[test]
    fn a_value_written_by_another_manager_during_a_boost_stays() {
        let fixture = Fixture::new(&[
            ("balance_power", "powersave"),
            ("balance_power", "powersave"),
        ]);
        let mut booster = fixture.booster();
        booster.start(Instant::now(), &balanced());
        fake::set(
            &fixture.root(),
            1,
            "energy_performance_preference",
            "balance_performance",
        );
        assert_eq!(booster.finish(Some("balanced")), Some(Ending::Replaced));
        assert_eq!(
            fixture.preferences(2),
            ["balance_power", "balance_performance"]
        );
    }

    #[test]
    fn a_profile_change_during_a_boost_keeps_the_new_profiles_values() {
        // The new profile's preference equals the boost's, so only the profile tells them apart.
        let fixture = Fixture::new(&[("balance_power", "powersave")]);
        let mut booster = fixture.booster();
        booster.start(Instant::now(), &balanced());
        assert_eq!(
            booster.finish(Some("performance")),
            Some(Ending::ProfileChanged)
        );
        assert_eq!(fixture.preferences(1), ["performance"]);
    }

    #[test]
    fn a_profile_service_that_stops_during_a_boost_still_gets_its_values_back() {
        let fixture = Fixture::new(&[("balance_power", "powersave")]);
        let mut booster = fixture.booster();
        booster.start(Instant::now(), &balanced());
        assert_eq!(booster.finish(None), Some(Ending::Restored));
        assert_eq!(fixture.preferences(1), ["balance_power"]);
    }

    #[test]
    fn a_performance_governor_chosen_during_a_boost_is_kept() {
        let fixture = Fixture::new(&[("balance_power", "powersave")]);
        let mut booster = fixture.booster();
        booster.start(Instant::now(), &Host::default());
        fake::set(&fixture.root(), 0, "scaling_governor", "performance");
        assert_eq!(booster.finish(None), Some(Ending::Replaced));
        assert_eq!(fixture.preferences(1), ["performance"]);
    }

    #[test]
    fn a_boost_left_by_a_crashed_helper_is_restored_only_if_unchanged() {
        let fixture = Fixture::new(&[
            ("balance_power", "powersave"),
            ("balance_power", "powersave"),
        ]);
        fixture.booster().start(Instant::now(), &balanced());
        assert_eq!(
            fixture.booster().recover(Some("balanced")),
            Some(Ending::Restored)
        );
        assert_eq!(fixture.preferences(2), ["balance_power", "balance_power"]);
        assert!(!fixture.marker().exists());
        assert_eq!(fixture.booster().recover(Some("balanced")), None);

        fixture.booster().start(Instant::now(), &balanced());
        assert_eq!(
            fixture.booster().recover(Some("performance")),
            Some(Ending::ProfileChanged)
        );
        assert_eq!(fixture.preferences(2), ["performance", "performance"]);
        assert!(!fixture.marker().exists());

        fake::set(
            &fixture.root(),
            0,
            "energy_performance_preference",
            "balance_power",
        );
        fake::set(
            &fixture.root(),
            1,
            "energy_performance_preference",
            "balance_power",
        );
        fixture.booster().start(Instant::now(), &balanced());
        fake::set(&fixture.root(), 0, "energy_performance_preference", "power");
        assert_eq!(
            fixture.booster().recover(Some("balanced")),
            Some(Ending::Replaced)
        );
        assert_eq!(fixture.preferences(2), ["power", "balance_power"]);
    }

    #[test]
    fn a_damaged_marker_is_discarded_without_writing() {
        let fixture = Fixture::new(&[("performance", "powersave")]);
        for text in [
            "enzui-power-boost 1\nprofile balanced\n../policy0 balance_power\n",
            "enzui-power-boost 1\nprofile balanced\npolicy0 balance power\n",
            "enzui-power-boost 2\nprofile balanced\npolicy0 balance_power\n",
            "enzui-power-boost 1\npolicy0 balance_power\n",
        ] {
            std::fs::write(fixture.marker(), text).unwrap();
            assert_eq!(fixture.booster().recover(Some("balanced")), None);
            assert!(!fixture.marker().exists());
            assert_eq!(fixture.preferences(1), ["performance"]);
        }
        std::fs::write(
            fixture.marker(),
            "enzui-power-boost 1\nprofile \npolicy0 balance_power\n",
        )
        .unwrap();
        assert_eq!(fixture.booster().recover(None), Some(Ending::Restored));
        assert_eq!(fixture.preferences(1), ["balance_power"]);
    }

    #[test]
    fn a_failed_write_undoes_the_policies_already_boosted() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(&[
            ("balance_power", "powersave"),
            ("balance_power", "powersave"),
        ]);
        let locked = fixture.root().join("policy1/energy_performance_preference");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
        // Root ignores file modes, so the sabotage cannot fail a write there.
        if std::fs::OpenOptions::new()
            .write(true)
            .open(&locked)
            .is_ok()
        {
            return;
        }
        let mut booster = fixture.booster();
        assert_eq!(booster.start(Instant::now(), &balanced()), Outcome::Failed);
        assert_eq!(fixture.preferences(2), ["balance_power", "balance_power"]);
        assert!(!fixture.marker().exists());
        assert_eq!(booster.until(), None);
    }
}
