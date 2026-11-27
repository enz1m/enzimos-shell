use std::time::{Duration, Instant};

pub const RENEW_INTERVAL: Duration = Duration::from_secs(1);
const HOLD_LEASE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Hold {
    until: Instant,
    remaining: Duration,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Toast {
    until: Option<Instant>,
    hold: Option<Hold>,
}

impl Toast {
    pub fn new(until: Option<Instant>) -> Self {
        Self { until, hold: None }
    }

    pub fn visible(self, now: Instant) -> bool {
        // Snapshots omit renewals; held visibility changes when the model publishes release.
        self.hold.is_some() || self.until.is_some_and(|until| until > now)
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn advance(&mut self, now: Instant) -> bool {
        let before = *self;
        if let Some(hold) = self.hold.filter(|hold| hold.until <= now) {
            self.until = Some(hold.until + hold.remaining);
            self.hold = None;
        }
        if self.hold.is_none() && self.until.is_some_and(|until| until <= now) {
            self.until = None;
        }
        *self != before
    }

    pub(crate) fn hold(&mut self, enabled: bool, now: Instant) -> bool {
        let mut changed = self.advance(now);
        if enabled {
            if let Some(hold) = self.hold.as_mut() {
                hold.until = now + HOLD_LEASE;
            } else if let Some(until) = self.until {
                self.hold = Some(Hold {
                    until: now + HOLD_LEASE,
                    remaining: until.duration_since(now),
                });
                changed = true;
            }
        } else if let Some(hold) = self.hold.take() {
            self.until = Some(now + hold.remaining);
            changed = true;
        }
        changed
    }
}
