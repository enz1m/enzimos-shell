use enzui_shell::dock::{Group, Slots};
use enzui_theme::{MotionScale, MotionToken};
use std::time::Instant;

struct Transition {
    from: f32,
    to: f32,
    start: Instant,
    duration: f32,
    easing: enzui_theme::MotionEasing,
}

#[derive(Default)]
pub(super) struct Track {
    value: f32,
    transition: Option<Transition>,
}

impl Track {
    pub(super) fn value(&self) -> f32 {
        self.value
    }

    pub(super) fn active(&self) -> bool {
        self.transition.is_some()
    }

    pub(super) fn retarget(
        &mut self,
        to: f32,
        at: Instant,
        motion: MotionScale,
        token: MotionToken,
    ) {
        if self
            .transition
            .as_ref()
            .is_some_and(|target| target.to == to)
            && !motion.reduced_motion
        {
            return;
        }
        self.advance(at);
        let duration = motion.secs(token);
        if duration <= 0.0 || self.value == to {
            self.value = to;
            self.transition = None;
        } else {
            self.transition = Some(Transition {
                from: self.value,
                to,
                start: at,
                duration,
                easing: motion.easing,
            });
        }
    }

    pub(super) fn advance(&mut self, at: Instant) {
        let Some(transition) = &self.transition else {
            return;
        };
        let time =
            at.saturating_duration_since(transition.start).as_secs_f32() / transition.duration;
        if time >= 1.0 {
            self.value = transition.to;
            self.transition = None;
        } else {
            let progress = transition.easing.apply(time);
            self.value = transition.from + (transition.to - transition.from) * progress;
        }
    }
}

pub(super) struct Slot {
    pub(super) group: Group,
    pub(super) departing: bool,
    extent: Track,
}

impl Slot {
    pub(super) fn extent(&self) -> f32 {
        self.extent.value()
    }
}

#[derive(Default)]
pub(super) struct Strip {
    pub(super) trailing: Vec<Slot>,
    pub(super) divider: Track,
}

impl Strip {
    pub(super) fn sync<'a>(
        &mut self,
        groups: impl Iterator<Item = &'a Group>,
        pinned: &[String],
        divider: bool,
        at: Instant,
        motion: MotionScale,
    ) {
        let mut previous: Vec<_> = self.trailing.drain(..).map(Some).collect();
        let mut current: Vec<_> = groups
            .map(|group| {
                let mut slot = previous
                    .iter_mut()
                    .find(|old| old.as_ref().is_some_and(|old| old.group.id == group.id))
                    .and_then(Option::take)
                    .unwrap_or_else(|| Slot {
                        group: group.clone(),
                        departing: false,
                        extent: Track::default(),
                    });
                slot.group.clone_from(group);
                slot.departing = false;
                slot.extent.retarget(1.0, at, motion, MotionToken::Fast);
                slot
            })
            .collect();
        for (index, old) in previous.into_iter().enumerate() {
            let Some(mut old) = old.filter(|old| !pinned.contains(&old.group.id)) else {
                continue;
            };
            old.departing = true;
            old.extent.retarget(0.0, at, motion, MotionToken::Fast);
            if old.extent.active() {
                current.insert(index.min(current.len()), old);
            }
        }
        self.trailing = current;
        self.divider
            .retarget(f32::from(divider), at, motion, MotionToken::Fast);
    }

    pub(super) fn advance(&mut self, at: Instant) {
        self.divider.advance(at);
        for slot in &mut self.trailing {
            slot.extent.advance(at);
        }
        self.trailing
            .retain(|slot| !slot.departing || slot.extent.active());
    }

    pub(super) fn active(&self) -> bool {
        self.divider.active() || self.trailing.iter().any(|slot| slot.extent.active())
    }

    pub(super) fn span(&self, slots: Slots, pinned: usize) -> f32 {
        let trailing = self.trailing.iter().enumerate().map(|(index, slot)| {
            let gap = if index == 0 && pinned == 0 {
                0.0
            } else {
                slots.spacing
            };
            slot.extent() * (slots.target + gap)
        });
        slots.span(pinned) + trailing.sum::<f32>() + self.divider.value() * slots.separator_span()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reopening_an_app_reverses_its_exit_without_replacing_the_slot() {
        let running = |id: &str| Group {
            id: id.into(),
            title: id.into(),
            icon: String::new(),
            image: None,
            app: None,
            pinned: false,
            builtin: None,
            windows: Vec::new(),
        };
        let apps = [running("a"), running("b"), running("c")];
        let start = Instant::now();
        let motion = MotionScale {
            fast_ms: 120.0,
            ..MotionScale::default()
        };
        let mut strip = Strip::default();
        strip.sync(
            apps.iter(),
            &[],
            false,
            start,
            motion.with_reduced_motion(true),
        );
        strip.sync([&apps[0], &apps[2]].into_iter(), &[], false, start, motion);

        let midway = start + Duration::from_millis(60);
        strip.advance(midway);
        let exiting = &strip.trailing[1];
        assert_eq!(exiting.group.id, "b");
        assert!(exiting.departing && (0.0..1.0).contains(&exiting.extent()));
        let extent = exiting.extent();

        strip.sync(apps.iter(), &[], false, midway, motion);
        assert_eq!(strip.trailing.len(), 3);
        assert!(!strip.trailing[1].departing);
        assert_eq!(strip.trailing[1].extent(), extent);
        strip.advance(midway + Duration::from_millis(120));
        assert_eq!(strip.trailing[1].extent(), 1.0);
        assert!(!strip.active());
    }
}
