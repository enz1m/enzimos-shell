//! Input arming for dialogs that grant access or submit on the user's behalf.
//!
//! A dialog that appears under the pointer or takes keyboard focus while the user
//! is typing elsewhere must not act on that input. [`ArmGate`] stays disarmed until
//! [`ARM_DELAY`] has passed since the window was shown and focused; [`ArmGate::wake`]
//! schedules the redraw that ends the disarmed styling.

use std::time::{Duration, Instant};

/// How long a newly shown dialog ignores input.
pub const ARM_DELAY: Duration = Duration::from_millis(500);

/// Whether a dialog window has been visible long enough to act on input.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArmGate {
    shown_at: Option<Instant>,
}

impl ArmGate {
    /// Records when the window was shown; later calls keep the first time.
    pub fn shown(&mut self, now: Instant) {
        self.shown_at.get_or_insert(now);
    }

    /// Restarts the delay when keyboard focus arrives before the gate is armed, since a
    /// window usually opens before the compositor maps and focuses it. Returns whether the
    /// delay restarted.
    pub fn focused(&mut self, now: Instant) -> bool {
        let restart = !self.armed_at(now);
        if restart {
            self.shown_at = Some(now);
        }
        restart
    }

    /// Whether input arriving at `now` may act. A window never shown is disarmed.
    pub fn armed_at(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|shown| now.saturating_duration_since(shown) >= ARM_DELAY)
    }

    pub fn armed(&self) -> bool {
        self.armed_at(Instant::now())
    }

    /// A task that delivers `message` once a window shown or focused now becomes armed.
    pub fn wake<Message: Send + 'static>(message: Message) -> iced::Task<Message> {
        iced::Task::perform(tokio::time::sleep(ARM_DELAY), move |()| message)
    }
}
