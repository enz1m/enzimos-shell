use super::model::{Detail, DetailPhase, Edit};
use crate::{
    control_layout::{Layout as Model, Placement},
    settings::QuickAction,
};
use iced::Element;
use std::{sync::Arc, time::Duration};

pub struct Grid<'a, Message> {
    pub(super) model: Arc<Model>,
    pub(super) keys: Vec<QuickAction>,
    pub(super) disclosures: Vec<bool>,
    pub(super) children: Vec<Element<'a, Message>>,
    pub(super) height: f32,
    pub(super) duration: Duration,
    pub(super) easing: enzui_theme::MotionEasing,
    pub(super) editable: bool,
    pub(super) interactive: bool,
    pub(super) accent: iced::Color,
    pub(super) handle: iced::Color,
    pub(super) surface: iced::Color,
    pub(super) selected: Option<QuickAction>,
    pub(super) detail: Option<Detail<'a, Message>>,
    pub(super) on_settled: Option<(u64, Box<dyn Fn() -> Message + 'a>)>,
    pub(super) edit: Box<dyn Fn(Edit) -> Message + 'a>,
}
impl<'a, Message> Grid<'a, Message> {
    pub fn new(
        model: Arc<Model>,
        mut items: Vec<(QuickAction, bool, Element<'a, Message>)>,
        height: f32,
        theme: &enzui_theme::EnzTheme,
        editable: bool,
        edit: impl Fn(Edit) -> Message + 'a,
    ) -> Self {
        items.sort_by_key(|(action, _, _)| model.order().iter().position(|key| key == action));
        let mut keys = Vec::new();
        let mut disclosures = Vec::new();
        let mut children = Vec::new();
        for (key, disclosure, child) in items {
            keys.push(key);
            disclosures.push(disclosure);
            children.push(child);
        }
        Self {
            model,
            keys,
            disclosures,
            children,
            height,
            duration: Duration::from_secs_f32(
                (theme.motion.ms(enzui_theme::MotionToken::Normal) / 1000.0).min(2.0),
            ),
            easing: theme.motion.easing,
            editable,
            interactive: true,
            accent: theme.palette.accent,
            handle: theme.palette.text,
            surface: theme.palette.surface_alt,
            selected: None,
            detail: None,
            on_settled: None,
            edit: Box::new(edit),
        }
    }
    #[must_use]
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }
    #[must_use]
    pub fn selected(mut self, action: Option<QuickAction>) -> Self {
        self.selected = action;
        self
    }
    #[must_use]
    pub fn detail(mut self, detail: Option<Detail<'a, Message>>) -> Self {
        self.detail = detail.filter(|detail| self.keys.contains(&detail.action));
        self
    }
    #[must_use]
    pub fn on_settled(mut self, key: u64, callback: impl Fn() -> Message + 'a) -> Self {
        self.on_settled = Some((key, Box::new(callback)));
        self
    }
    pub(super) fn phase_detail(&self, phase: DetailPhase) -> Option<(QuickAction, f32)> {
        self.detail.as_ref().and_then(|d| match phase {
            DetailPhase::ReflowOpen | DetailPhase::Collapse => Some((d.action, 0.0)),
            DetailPhase::Grow => Some((d.action, d.height)),
            DetailPhase::Stable if d.open => Some((d.action, d.height)),
            _ => None,
        })
    }
    pub(super) fn placements(&self, model: &Model, width: f32) -> Vec<Placement> {
        self.placements_resizing(model, width, None, DetailPhase::Stable)
    }
    pub(super) fn placements_resizing(
        &self,
        model: &Model,
        width: f32,
        resizing: Option<(QuickAction, f32)>,
        phase: DetailPhase,
    ) -> Vec<Placement> {
        model.placements_with_detail(
            width,
            self.height,
            |action| self.keys.contains(&action),
            self.phase_detail(phase),
            resizing,
        )
    }
}
