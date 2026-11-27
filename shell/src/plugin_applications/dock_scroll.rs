use crate::Message;
use enzui_widgets::scroll_view::{ScrollGeometryCell, publish_geometry_of};
use iced::advanced::{
    Layout, Shell, Widget, layout, renderer,
    widget::{Operation, Tree},
};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, keyboard, mouse, touch};
use std::collections::HashMap;

pub(crate) fn dock_scroll<'a>(
    viewport: Element<'a, Message>,
    geometry: ScrollGeometryCell,
    horizontal: bool,
) -> Element<'a, Message> {
    Element::new(DockInput {
        child: publish_geometry_of(viewport, geometry.clone()),
        geometry,
        horizontal,
    })
}

struct DockInput<'a> {
    child: Element<'a, Message>,
    geometry: ScrollGeometryCell,
    horizontal: bool,
}

#[derive(Default)]
struct InputState {
    modifiers: keyboard::Modifiers,
    contacts: HashMap<touch::Finger, Contact>,
}

const TAP_SLOP: f32 = 8.0;

struct Contact {
    start: Point,
    swiped: bool,
}

impl Contact {
    fn record_motion(&mut self, position: Point) {
        self.swiped |= self.start.distance(position) >= TAP_SLOP;
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for DockInput<'_> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<InputState>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(InputState::default())
    }
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::pass_through(tree, 0);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let input = tree.state.downcast_mut::<InputState>();
        let remapped = match *event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                input.modifiers = modifiers;
                None
            }
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y },
            }) if self.horizontal
                && !input.modifiers.shift()
                && self
                    .geometry
                    .get()
                    .is_some_and(|geometry| geometry.is_overflowing_x()) =>
            {
                Some(Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: y, y: 0.0 },
                }))
            }
            Event::Touch(touch::Event::FingerPressed { id, position })
                if layout.bounds().contains(position) && viewport.contains(position) =>
            {
                input.contacts.insert(
                    id,
                    Contact {
                        start: position,
                        swiped: false,
                    },
                );
                None
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                if let Some(contact) = input.contacts.get_mut(&id) {
                    contact.record_motion(position);
                }
                None
            }
            Event::Touch(touch::Event::FingerLifted { id, position }) => {
                // Content moves under the finger, so release can still land on the pressed icon.
                input
                    .contacts
                    .remove(&id)
                    .filter(|contact| {
                        contact.swiped || contact.start.distance(position) >= TAP_SLOP
                    })
                    .map(|_| Event::Touch(touch::Event::FingerLost { id, position }))
            }
            Event::Touch(touch::Event::FingerLost { id, .. }) => {
                input.contacts.remove(&id);
                None
            }
            _ => None,
        };
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            remapped.as_ref().unwrap_or(event),
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn frame_interest(&self, tree: &Tree) -> bool {
        tree.children.first().is_none_or(Tree::has_frame_interest)
    }

    fn update_frame(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let child = &mut tree.children[0];
        self.child
            .as_widget_mut()
            .update_frame(child, event, layout, cursor, renderer, shell, viewport);
        child.refresh_frame_interest(self.child.as_widget());
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.child.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            viewport,
            renderer,
            operation,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.child.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.child.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
