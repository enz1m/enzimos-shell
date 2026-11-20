use super::model::{DetailPhase, State, lerp};
use crate::{control_layout::Placement, settings::QuickAction};
use iced::Rectangle;
use std::time::Duration;

impl State {
    pub(super) fn progress(&self) -> f32 {
        self.started.map_or(1.0, |start| {
            if self.duration.is_zero() {
                1.0
            } else {
                (self.at.saturating_duration_since(start).as_secs_f32()
                    / self.duration.as_secs_f32())
                .clamp(0.0, 1.0)
            }
        })
    }
    pub(super) fn moving(&self) -> bool {
        self.progress() < 1.0
    }
    pub(super) fn transitioning(&self) -> bool {
        self.moving() || self.phase != DetailPhase::Stable
    }
    pub(super) fn change_detail(&mut self, owner: Option<QuickAction>, open: bool) {
        let from = self.pending_from.take().unwrap_or_else(|| {
            self.target
                .iter()
                .map(|p| Placement {
                    action: p.action,
                    bounds: self.pose(p.action),
                })
                .collect()
        });
        let section = self.section_pose();
        self.from.clone_from(&from);
        self.target.clone_from(&from);
        self.pending_from = Some(from);
        self.section_from = section;
        self.section_target = section;
        self.pending_section = None;
        self.started = None;
        self.phase_armed = false;
        if self.detail != owner {
            self.section_from.height = 0.0;
            self.section_target.height = 0.0;
            self.pending_section = None;
            self.phase = if open {
                DetailPhase::ReflowOpen
            } else {
                DetailPhase::Stable
            };
        } else if open {
            self.phase = if self.section_pose().height > 0.0 {
                DetailPhase::Grow
            } else {
                DetailPhase::ReflowOpen
            };
        } else {
            self.phase = if self.section_pose().height > 0.0 {
                DetailPhase::Collapse
            } else {
                DetailPhase::ReflowClosed
            };
        }
        if self.target.is_empty() {
            self.phase = DetailPhase::Stable;
        }
        self.detail = owner;
        self.detail_open = open;
    }
    pub(super) fn advance_phase(&mut self, duration: Duration) {
        if duration.is_zero() {
            self.phase = DetailPhase::Stable;
        } else if self.phase_armed && !self.moving() {
            self.phase = match self.phase {
                DetailPhase::ReflowOpen => DetailPhase::Grow,
                DetailPhase::Collapse => DetailPhase::ReflowClosed,
                DetailPhase::Grow | DetailPhase::ReflowClosed | DetailPhase::Stable => {
                    DetailPhase::Stable
                }
            };
        }
    }
    pub(super) fn front(&self) -> Option<QuickAction> {
        self.drag
            .as_ref()
            .filter(|drag| drag.started)
            .map(|drag| drag.action)
            .or_else(|| self.moving().then_some(self.settling).flatten())
    }
    pub(super) fn slot_pose(&self, action: QuickAction) -> Rectangle {
        let target = self
            .target
            .iter()
            .find(|p| p.action == action)
            .map(|p| p.bounds)
            .unwrap_or_default();
        let from = self
            .from
            .iter()
            .find(|p| p.action == action)
            .map_or(target, |p| p.bounds);
        let t = self.easing.apply(self.progress());
        if matches!(
            self.phase,
            DetailPhase::ReflowOpen | DetailPhase::ReflowClosed
        ) {
            let fast = (t * 2.0).min(1.0);
            let late = (t * 2.0 - 1.0).max(0.0);
            let horizontal = if target.y < from.y {
                fast
            } else if target.y > from.y || self.phase == DetailPhase::ReflowOpen {
                late
            } else {
                fast
            };
            let vertical = if target.y > from.y {
                fast
            } else if target.y < from.y {
                late
            } else {
                t
            };
            let mut rect = lerp(from, target, horizontal);
            rect.y = from.y + (target.y - from.y) * vertical;
            rect
        } else {
            lerp(from, target, t)
        }
    }
    pub(super) fn pose(&self, action: QuickAction) -> Rectangle {
        let bounds = self.slot_pose(action);
        if let Some(drag) = &self.drag
            && drag.started
            && drag.action == action
        {
            if drag.resize {
                let width = (drag.initial.width + drag.point.x - drag.start.x)
                    .clamp(drag.initial.height.min(drag.width), drag.width);
                Rectangle {
                    x: drag.initial.x.min(drag.width - width).max(0.0),
                    width,
                    ..bounds
                }
            } else {
                Rectangle {
                    x: drag.point.x - drag.offset.x,
                    y: drag.point.y - drag.offset.y,
                    ..bounds
                }
            }
        } else {
            bounds
        }
    }
    pub(super) fn section_pose(&self) -> Rectangle {
        let height = lerp(
            self.section_from,
            self.section_target,
            self.easing.apply(self.progress()),
        )
        .height;
        let owner = self
            .detail
            .filter(|action| self.target.iter().any(|p| p.action == *action))
            .map(|action| self.slot_pose(action));
        Rectangle {
            height,
            x: 0.0,
            y: owner.map_or(self.section_target.y, |r| r.y + r.height),
            width: self.section_target.width,
        }
    }
    pub(super) fn visual_section(&self) -> Rectangle {
        let mut section = self.section_pose();
        if let Some(action) = self.detail {
            let owner = self.pose(action);
            section.x = if self.front() == Some(action) {
                owner.x
            } else {
                0.0
            };
            section.y = owner.y + owner.height;
        }
        section
    }
    pub(super) fn retarget(
        &mut self,
        target: Vec<Placement>,
        duration: Duration,
        easing: enzui_theme::MotionEasing,
    ) {
        if self.target == target
            && self.pending_from.is_none()
            && self.pending_section.is_none()
            && (!self.moving() || (self.duration == duration && self.easing == easing))
        {
            if !self.moving() {
                self.started = None;
            }
            self.duration = duration;
            self.easing = easing;
            return;
        }
        if !self.moving() && self.pending_from.is_none() && self.drag.is_none() {
            self.settling = None;
        }
        let from = self.pending_from.take().unwrap_or_else(|| {
            target
                .iter()
                .map(|p| Placement {
                    action: p.action,
                    bounds: self.pose(p.action),
                })
                .collect()
        });
        self.section_from = self.section_pose();
        self.section_target = self.pending_section.take().unwrap_or(self.section_target);
        self.started = (!self.target.is_empty()).then_some(self.at);
        self.from = from;
        self.target = target;
        self.duration = duration;
        self.easing = easing;
    }
    pub(super) fn cancel(&mut self) {
        if self.drag.is_some() {
            self.pending_from = Some(
                self.target
                    .iter()
                    .map(|p| Placement {
                        action: p.action,
                        bounds: self.pose(p.action),
                    })
                    .collect(),
            );
            self.settling = self.drag.as_ref().map(|drag| drag.action);
            self.drag = None;
            self.swallow = true;
        }
    }
}
