use enzui_theme::{ExpressiveMotion, MotionChannel};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(super) struct Timing {
    pub push: Duration,
    pub slide: Duration,
    pub leave: Duration,
    pub delay: Duration,
    pub fade_in: Duration,
    pub fade_out: Duration,
    pub step: Duration,
    pub mapping: Duration,
    pub update: Duration,
    pub reduced: bool,
    pub curve: Option<ExpressiveMotion>,
}
impl Timing {
    pub(super) fn new(rate: u16, reduced: bool) -> Self {
        let scale = if reduced {
            0.0
        } else {
            enzui_shell::motion::duration_factor(rate)
        };
        let ms = |value: u64| Duration::from_secs_f64(value as f64 * f64::from(scale) / 1000.0);
        Self {
            push: ms(180),
            slide: ms(320),
            leave: ms(260),
            delay: ms(500),
            fade_in: ms(220),
            fade_out: ms(90),
            step: ms(150),
            mapping: ms(140),
            update: ms(40),
            reduced,
            curve: None,
        }
    }
    // Bloom keeps the panels' shortened duration; the row push and exit keep
    // their bounded curves.
    pub(super) fn with_style(mut self, style: u8, rate: u16) -> Self {
        self.curve = enzui_shell::motion::expressive(style).filter(|_| !self.reduced);
        if let Some(curve) = self.curve {
            let base = if curve == ExpressiveMotion::SlowSpatial {
                350.0
            } else {
                curve.base_ms()
            };
            self.slide = Duration::from_micros(
                (base * enzui_shell::motion::duration_factor(rate) * 1000.0).round() as u64,
            );
        }
        self
    }
    pub(super) fn overshoot(self) -> f32 {
        self.curve
            .map_or(0.0, |curve| (curve.curve().peak() - 1.0).max(0.0))
    }
    pub(super) fn progress(elapsed: Duration, span: Duration) -> f32 {
        if span.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f32() / span.as_secs_f32()).clamp(0.0, 1.0)
        }
    }
    pub(super) fn entrance(self, elapsed: Duration, push: bool) -> (f32, f32) {
        let ease = |elapsed, span| 1.0 - (1.0 - Self::progress(elapsed, span)).powi(3);
        (
            if push { ease(elapsed, self.push) } else { 1.0 },
            self.curve.map_or_else(
                || {
                    ease(
                        elapsed.saturating_sub(if push { self.push } else { Duration::ZERO }),
                        self.slide,
                    )
                },
                |curve| {
                    curve.sample(
                        Self::progress(
                            elapsed.saturating_sub(if push { self.push } else { Duration::ZERO }),
                            self.slide,
                        ),
                        MotionChannel::Spatial,
                    )
                },
            ),
        )
    }
}
impl crate::Shell {
    pub(super) fn toast_timing(&self) -> Timing {
        let rate = self.desired_settings.compositor.animation_rate;
        Timing::new(rate, self.tokens().motion.reduced_motion)
            .with_style(self.desired_settings.compositor.animation_style, rate)
    }
    pub(super) fn toast_overshoot(&self) -> f32 {
        let travel = f32::from(self.desired_settings.notifications.width)
            + 2.0 * enzui_shell::notifications::card::SHADOW_MARGIN;
        (travel * self.toast_timing().overshoot())
            .ceil()
            .min(enzui_shell::compositor::MAX_TOAST_OVERSHOOT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_behaviour() {
        // Arrival and exit follow the selected speed.
        {
            let ordinary = Timing::new(100, false);
            let fast = Timing::new(200, false);
            let slow = Timing::new(50, false);
            assert_eq!(
                ordinary.entrance(Duration::from_millis(250), true),
                fast.entrance(Duration::from_millis(125), true)
            );
            assert_eq!(
                ordinary.entrance(Duration::from_millis(250), true),
                slow.entrance(Duration::from_millis(500), true)
            );
            assert_eq!(
                Timing::progress(Duration::from_millis(130), ordinary.leave),
                Timing::progress(Duration::from_millis(65), fast.leave)
            );
            assert_eq!(fast.step, Duration::from_millis(75));
            assert_eq!(slow.slide, Duration::from_millis(640));
        }
        // Styles slide on their spatial curve and settle exactly.
        {
            let standard = Timing::new(100, false).with_style(0, 100);
            assert!(standard.curve.is_none() && standard.overshoot() == 0.0);
            assert_eq!(standard.slide, Duration::from_millis(320));
            let spring = Timing::new(100, false).with_style(1, 100);
            assert_eq!(spring.slide, Duration::from_millis(350));
            assert!(spring.overshoot() > 0.05 && spring.overshoot() < 0.2);
            let peak = (1..100)
                .map(|ms| spring.entrance(Duration::from_millis(ms * 4), false).1)
                .fold(0.0, f32::max);
            assert!(peak > 1.0 && peak <= 1.0 + spring.overshoot() + 1e-3);
            assert_eq!(spring.entrance(spring.slide, false), (1.0, 1.0));
            assert_eq!(
                spring.entrance(spring.push + spring.slide, true),
                (1.0, 1.0)
            );
            assert_eq!(
                Timing::new(100, false).with_style(3, 100).slide,
                Duration::from_millis(350)
            );
            assert_eq!(
                Timing::new(200, false).with_style(2, 200).slide,
                Duration::from_millis(250)
            );
            let reduced = Timing::new(100, true).with_style(1, 100);
            assert!(reduced.curve.is_none() && reduced.overshoot() == 0.0);
            assert_eq!(reduced.entrance(Duration::ZERO, true), (1.0, 1.0));
        }
        // Reduced motion settles every phase without zero division.
        {
            let reduced = Timing::new(25, true);
            assert_eq!(reduced.entrance(Duration::ZERO, true), (1.0, 1.0));
            assert_eq!(reduced.entrance(Duration::ZERO, false), (1.0, 1.0));
            assert_eq!(Timing::progress(Duration::ZERO, reduced.leave), 1.0);
            assert!(
                reduced.mapping.is_zero() && reduced.update.is_zero() && reduced.step.is_zero()
            );
        }
    }
}
