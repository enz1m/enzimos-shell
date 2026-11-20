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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_is_not_published_and_release_resumes_remaining_time() {
        let now = Instant::now();
        let at = |seconds| now + Duration::from_secs(seconds);
        let mut toast = Toast::new(Some(at(5)));
        assert!(toast.hold(true, at(2)));
        let snapshot = toast;
        assert!(!toast.hold(true, at(4)));
        assert!(!toast.advance(at(6)));
        assert!(snapshot.visible(at(6)));
        assert!(toast.hold(false, at(6)));
        assert!(toast.visible(at(8)));
        assert!(toast.advance(at(9)));
        assert!(!toast.visible(at(9)));
    }
}
