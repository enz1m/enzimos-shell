use iced::advanced::widget::{
    Id, Operation,
    operation::{DockedClock, Focusable, Scrollable, SelectableTextLayout, TextInput},
};
use iced::{Rectangle, Size, Transformation, Vector};
use std::any::Any;

/// Reports a child widget’s geometry in the parent’s coordinate space.
pub struct Projected<'a> {
    /// The operation receiving the projected geometry.
    pub operation: &'a mut dyn Operation,
    /// The transform from child layout coordinates to the operation’s space.
    pub transform: Transformation,
}
impl Operation for Projected<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let transform = self.transform;
        self.operation.traverse(&mut |operation| {
            operate(&mut Projected {
                operation,
                transform,
            });
        });
    }
    fn is_native_reconciliation(&self) -> bool {
        self.operation.is_native_reconciliation()
    }
    fn is_docked_clock_rebase(&self) -> bool {
        self.operation.is_docked_clock_rebase()
    }
    fn docked_clock(&mut self, clock: &mut dyn DockedClock) {
        self.operation.docked_clock(clock);
    }
    fn projection_applied(&mut self, root: iced::advanced::renderer::NodeId) {
        self.operation.projection_applied(root);
    }
    fn needs_editor_reconciliation(&self, node: iced::advanced::renderer::NodeId) -> bool {
        self.operation.needs_editor_reconciliation(node)
    }
    fn editor_reconciled(&mut self, node: iced::advanced::renderer::NodeId, success: bool) {
        self.operation.editor_reconciled(node, success);
    }
    fn needs_scroll_reconciliation(&self, node: iced::advanced::renderer::NodeId) -> bool {
        self.operation.needs_scroll_reconciliation(node)
    }
    fn scroll_reconciled(&mut self, node: iced::advanced::renderer::NodeId, success: bool) {
        self.operation.scroll_reconciled(node, success);
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, viewport: &Rectangle) {
        self.operation
            .container(id, bounds * self.transform, &(*viewport * self.transform));
    }
    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.operation.focusable(id, bounds * self.transform, state);
    }
    fn text_input(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn TextInput) {
        self.operation
            .text_input(id, bounds * self.transform, state);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Size,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        self.operation.scrollable(
            id,
            bounds * self.transform,
            content * self.transform,
            translation * self.transform,
            state,
        );
    }
    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.operation.text(id, bounds * self.transform, text);
    }
    fn selectable_text(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        text: &str,
        layout: &dyn SelectableTextLayout,
    ) {
        self.operation
            .selectable_text(id, bounds * self.transform, text, layout);
    }
    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        self.operation.custom(id, bounds * self.transform, state);
    }
}
