use crate::{
    control_layout::{DropSlot, Layout as Model, Placement},
    settings::QuickAction,
};
use iced::{Element, Point, Rectangle, Vector};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum Edit {
    Select(QuickAction),
    Commit {
        expected: Arc<Model>,
        value: Arc<Model>,
    },
}

#[derive(Clone)]
pub(super) struct Drag {
    pub(super) action: QuickAction,
    pub(super) start: Point,
    pub(super) point: Point,
    pub(super) offset: Vector,
    pub(super) origin: Point,
    pub(super) width: f32,
    pub(super) initial: Rectangle,
    pub(super) resize: bool,
    pub(super) started: bool,
    pub(super) preview: Model,
    pub(super) slots: Vec<DropSlot>,
    pub(super) extent: f32,
}
pub struct Detail<'a, Message> {
    pub action: QuickAction,
    pub open: bool,
    pub height: f32,
    pub header_color: iced::Color,
    pub content: Element<'a, Message>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) enum DetailPhase {
    #[default]
    Stable,
    ReflowOpen,
    Grow,
    Collapse,
    ReflowClosed,
}

pub(super) struct State {
    pub(super) phase: DetailPhase,
    pub(super) phase_armed: bool,
    pub(super) keys: Vec<QuickAction>,
    pub(super) detail: Option<QuickAction>,
    pub(super) detail_open: bool,
    pub(super) section_from: Rectangle,
    pub(super) section_target: Rectangle,
    pub(super) pending_section: Option<Rectangle>,
    pub(super) report_key: Option<u64>,
    pub(super) reported: bool,
    pub(super) settled_layout: bool,
    pub(super) authority: Arc<Model>,
    pub(super) from: Vec<Placement>,
    pub(super) target: Vec<Placement>,
    pub(super) at: Instant,
    pub(super) started: Option<Instant>,
    pub(super) duration: Duration,
    pub(super) easing: enzui_theme::MotionEasing,
    pub(super) drag: Option<Drag>,
    pub(super) pending_from: Option<Vec<Placement>>,
    pub(super) settling: Option<QuickAction>,
    pub(super) swallow: bool,
    pub(super) needs_frame: bool,
}
pub(super) fn resize_handle(rect: Rectangle) -> Rectangle {
    Rectangle {
        x: rect.x + rect.width - 7.0,
        y: rect.center_y() - 7.0,
        width: 14.0,
        height: 14.0,
    }
}
pub(super) fn resize_hit(rect: Rectangle, point: Point) -> bool {
    resize_handle(rect).center().distance(point) <= 12.0
}
pub(super) fn edit_bounds(rect: Rectangle) -> Rectangle {
    Rectangle {
        x: rect.x - 14.0,
        y: rect.y,
        width: rect.width + 28.0,
        ..rect
    }
}
pub(super) fn lerp(a: Rectangle, b: Rectangle, t: f32) -> Rectangle {
    Rectangle {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        width: a.width + (b.width - a.width) * t,
        height: a.height + (b.height - a.height) * t,
    }
}
