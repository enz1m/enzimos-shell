use super::model::{DetailPhase, Drag, State};
use crate::{
    control_layout::{Layout as Model, Placement},
    settings::QuickAction,
};
use enzui_theme::MotionEasing;
use iced::{Point, Rectangle, Vector};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn state() -> State {
    State {
        phase: DetailPhase::Stable,
        phase_armed: false,
        keys: vec![QuickAction::Appearance],
        detail: None,
        detail_open: false,
        section_from: Rectangle::default(),
        section_target: Rectangle::default(),
        pending_section: None,
        authority: Arc::default(),
        from: Vec::new(),
        target: Vec::new(),
        at: Instant::now(),
        started: None,
        duration: Duration::from_millis(100),
        easing: MotionEasing::Linear,
        drag: None,
        pending_from: None,
        settling: None,
        swallow: false,
        needs_frame: false,
    }
}
fn target(x: f32) -> Vec<Placement> {
    vec![Placement {
        action: QuickAction::Appearance,
        bounds: Rectangle {
            x,
            y: 0.0,
            width: 64.0,
            height: 70.0,
        },
    }]
}

#[test]
fn canceled_expanded_drag_retains_its_whole_floating_card_then_returns_without_a_commit() {
    let mut state = state();
    let duration = state.duration;
    state.retarget(target(0.0), duration, MotionEasing::Linear);
    state.detail = Some(QuickAction::Appearance);
    state.section_target.height = 172.0;
    state.drag = Some(Drag {
        action: QuickAction::Appearance,
        start: Point::new(10.0, 10.0),
        point: Point::new(210.0, 110.0),
        offset: Vector::new(10.0, 10.0),
        origin: Point::ORIGIN,
        width: 360.0,
        initial: state.target[0].bounds,
        resize: false,
        started: true,
        preview: Model::default(),
        slots: Vec::new(),
        extent: 70.0,
    });
    assert_eq!(
        state.pose(QuickAction::Appearance).position(),
        Point::new(200.0, 100.0)
    );
    assert_eq!(state.visual_section().position(), Point::new(200.0, 170.0));
    assert_eq!(state.section_pose().position(), Point::new(0.0, 70.0));
    state.cancel();
    state.retarget(target(0.0), duration, MotionEasing::Linear);
    assert!(state.swallow && state.drag.is_none());
    assert_eq!(state.visual_section().position(), Point::new(200.0, 170.0));
    assert_eq!(state.front(), Some(QuickAction::Appearance));
    assert_eq!(
        state.pose(QuickAction::Appearance).position(),
        Point::new(200.0, 100.0)
    );
    state.at += duration;
    assert_eq!(state.front(), None);
    assert_eq!(
        state.pose(QuickAction::Appearance).position(),
        Point::ORIGIN
    );
}
