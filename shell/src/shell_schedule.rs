//! When the desktop's periodic work runs.
//!
//! The shell tick fires every 2 s only while something on screen needs it (a
//! countdown, an open panel, a sharing grant nobody watches). Otherwise it
//! fires at the next minute boundary, for the clock, and at least every 10 s,
//! for the system's state. The periodic jobs keep their own deadlines, so a
//! slower tick changes when they are noticed, not how often they run.
use std::time::{Duration, Instant};

/// The tick while something needs it.
pub(crate) const FAST_TICK: Duration = Duration::from_secs(2);
/// The longest an idle tick waits.
pub(crate) const SLOW_TICK: Duration = Duration::from_secs(10);

/// A job that runs every `every`, first at `next`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cadence {
    next: Instant,
    every: Duration,
}

impl Cadence {
    pub(crate) fn new(now: Instant, first: Duration, every: Duration) -> Self {
        Self {
            next: now + first,
            every,
        }
    }

    /// Whether the job is due at `now`; a due job is next due `every` later.
    pub(crate) fn due(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + self.every;
        true
    }
}

/// The desktop's periodic jobs.
#[derive(Debug)]
pub(crate) struct Schedule {
    /// The system's state (battery, radios, brightness).
    pub(crate) system: Cadence,
    /// Desktops and the sidebar: a safety net for missed compositor events.
    pub(crate) desktops: Cadence,
    /// Crash reports since the last look.
    pub(crate) crashes: Cadence,
    /// A guardian's parental-control changes.
    pub(crate) parental: Cadence,
    /// Screen time, recorded for the time since the last record.
    pub(crate) screen_time: Cadence,
    pub(crate) screen_time_recorded: Instant,
    /// Remembering open apps for the next login.
    pub(crate) session_apps: Cadence,
    /// Restoring the last session's apps, once, shortly after login.
    pub(crate) session_restore_at: Instant,
}

impl Schedule {
    pub(crate) fn new(now: Instant) -> Self {
        let minute = Duration::from_secs(60);
        Self {
            system: Cadence::new(now, Duration::ZERO, Duration::from_secs(10)),
            desktops: Cadence::new(now, Duration::ZERO, Duration::from_secs(30)),
            crashes: Cadence::new(now, Duration::from_secs(2), Duration::from_secs(30)),
            parental: Cadence::new(now, Duration::from_secs(4), minute),
            screen_time: Cadence::new(now, Duration::from_secs(30), minute),
            screen_time_recorded: now,
            session_apps: Cadence::new(now, minute, minute),
            session_restore_at: now + Duration::from_secs(6),
        }
    }

    /// Whole seconds of screen time since the last record, which this starts.
    pub(crate) fn screen_time_seconds(&mut self, now: Instant) -> u32 {
        let seconds = now
            .saturating_duration_since(self.screen_time_recorded)
            .as_secs()
            .min(120) as u32;
        self.screen_time_recorded = now;
        seconds
    }
}

/// The tick's period: [`FAST_TICK`] while `busy`, otherwise until just after
/// the next minute of `seconds_into_minute` (so the clock turns on time), and
/// never longer than [`SLOW_TICK`].
pub(crate) fn tick_period(busy: bool, seconds_into_minute: f64) -> Duration {
    if busy {
        return FAST_TICK;
    }
    let left = (60.0 - seconds_into_minute.clamp(0.0, 60.0)).max(0.0) + 0.05;
    Duration::from_secs_f64(left).clamp(Duration::from_millis(50), SLOW_TICK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_schedule_behaviour() {
        // A cadence runs once per period however often it is asked.
        {
            let start = Instant::now();
            let mut cadence = Cadence::new(start, Duration::ZERO, Duration::from_secs(30));
            let runs = (0..=60)
                .filter(|second| cadence.due(start + Duration::from_secs(*second)))
                .count();
            assert_eq!(runs, 3, "at 0, 30 and 60 s");
            // Asked only every 10 s, it still runs every 30 s.
            let mut sparse = Cadence::new(start, Duration::ZERO, Duration::from_secs(30));
            let runs = (0..=6)
                .filter(|step| sparse.due(start + Duration::from_secs(10 * step)))
                .count();
            assert_eq!(runs, 3);
        }
        // Sabotage: return [`FAST_TICK`] always. The idle tick then wakes the
        // desktop every 2 s.
        {
            assert_eq!(tick_period(true, 30.0), FAST_TICK);
            assert_eq!(tick_period(false, 0.0), SLOW_TICK);
            let near = tick_period(false, 57.0);
            assert!(
                near > Duration::from_secs(3) && near < Duration::from_millis(3100),
                "{near:?}"
            );
            assert!(tick_period(false, 59.999) >= Duration::from_millis(50));
        }
        // Screen time counts the time since the last record.
        {
            let start = Instant::now();
            let mut schedule = Schedule::new(start);
            assert_eq!(
                schedule.screen_time_seconds(start + Duration::from_secs(67)),
                67
            );
            assert_eq!(
                schedule.screen_time_seconds(start + Duration::from_secs(127)),
                60
            );
            // A long gap (a suspended laptop) is not all counted as use.
            assert_eq!(
                schedule.screen_time_seconds(start + Duration::from_secs(4000)),
                120
            );
        }
    }
}
