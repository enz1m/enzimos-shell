use crate::Message;
use enzui_core::drag::{
    DestinationProposal, DragOperation, DragSession, SessionOutcome, SourceCapabilities,
};
use enzui_shell::sidebar::Action;
use iced::advanced::{
    Shell, Widget,
    layout::{self, Layout},
    renderer,
    widget::{self, Operation, Tree},
};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

pub(super) struct Icon<'a> {
    pub child: Element<'a, Message>,
    pub id: u32,
    pub owner: u32,
    pub origin: Point,
    pub canvas: Size,
    pub scroll: f32,
    pub enabled: bool,
    pub light: bool,
}
struct State {
    identity: (u32, u32, Point, f32),
    drag: Option<DragSession>,
    press_bounds: Rectangle,
    position: Option<Point>,
    suppress_release: bool,
    cancel_child: bool,
    cancel_feedback: bool,
}
impl Icon<'_> {
    fn identity(&self) -> (u32, u32, Point, f32) {
        (self.id, self.owner, self.origin, self.scroll)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "The view composes independent state and actions supplied by its caller."
    )]
    fn child_update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let mut messages = iced::advanced::shell::Bus::new();
        let mut local = shell.local(&mut messages);
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            &mut local,
            viewport,
        );
        let anchor = (self.origin.y + layout.bounds().center_y() - self.scroll).round() as i32;
        shell.merge(local, |message| match message {
            Message::SidebarAction(Action::Toggle(id, _)) if id == self.id => {
                Message::SidebarAction(Action::Toggle(id, anchor))
            }
            other => other,
        });
    }
}
impl Widget<Message, iced::Theme, iced::Renderer> for Icon<'_> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State {
            identity: self.identity(),
            drag: None,
            press_bounds: Rectangle::default(),
            position: None,
            suppress_release: false,
            cancel_child: false,
            cancel_feedback: false,
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.identity != self.identity() || !self.enabled {
            state.cancel_feedback |= state.drag.is_some();
            state.suppress_release |= state.drag.take().is_some();
            state.cancel_child = true;
            state.identity = self.identity();
        }
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::pass_through(tree, 0);
    }
    fn frame_interest(&self, tree: &Tree) -> bool {
        tree.children.iter().any(Tree::has_frame_interest)
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
        let state = tree.state.downcast_mut::<State>();
        let chord = state.drag.is_some()
            && matches!(event, Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left);
        let cancel = chord
            || matches!(
                event,
                Event::Window(iced::window::Event::Unfocused | iced::window::Event::Closed)
            )
            || matches!(
                event,
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                })
            );
        if cancel
            || (state.drag.is_some()
                && !state
                    .drag
                    .as_ref()
                    .expect("the drag was checked before testing its phase")
                    .is_active()
                && state.press_bounds != layout.bounds())
        {
            state.cancel_feedback |= state.drag.is_some();
            state.suppress_release |= state.drag.take().is_some();
            state.cancel_child = true;
        }
        if std::mem::take(&mut state.cancel_feedback) {
            shell.publish(Message::SidebarAction(Action::CancelIconFeedback(
                self.owner,
            )));
        }
        let cancel_child = std::mem::take(&mut state.cancel_child);
        if cancel_child {
            self.child_update(
                tree,
                &Event::Window(iced::window::Event::Unfocused),
                layout,
                cursor.levitate(),
                renderer,
                shell,
                viewport,
            );
        }
        if chord {
            shell.capture_event();
            return;
        }
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.enabled && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) =>
            {
                if let Some(point) = cursor.position() {
                    let point = Point::new(point.x, point.y - self.scroll);
                    state.drag = Some(DragSession::begin(
                        enzui_core::drag::CollectionId::new(format!("sidebar-{}", self.owner)),
                        vec![enzui_core::drag::ItemId::new(self.id.to_string())],
                        SourceCapabilities::move_only(),
                        (point.x, point.y),
                        8.0,
                    ));
                    state.press_bounds = layout.bounds();
                    state.position = Some(point);
                    state.suppress_release = false;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if let Some(drag) = state.drag.as_mut() {
                    let was_active = drag.is_active();
                    state.position = Some(*position);
                    if drag.pointer_moved((position.x, position.y)) {
                        if !was_active {
                            shell.publish(Message::SidebarAction(Action::IconFeedback {
                                id: self.id,
                                owner: self.owner,
                                light: self.light,
                                x: (self.origin.x + layout.bounds().center_x()).round() as i32,
                                y: (self.origin.y + layout.bounds().center_y() - self.scroll)
                                    .round() as i32,
                            }));
                            self.child_update(
                                tree,
                                &Event::Window(iced::window::Event::Unfocused),
                                layout,
                                cursor.levitate(),
                                renderer,
                                shell,
                                viewport,
                            );
                            shell.request_redraw();
                        }
                        shell.capture_event();
                        return;
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let drag = state.drag.take();
                if std::mem::take(&mut state.suppress_release) {
                    shell.capture_event();
                    return;
                }
                if let Some(mut drag) = drag
                    && drag.is_active()
                {
                    if let Some(point) = state
                        .position
                        .filter(|p| p.x.is_finite() && p.y.is_finite())
                    {
                        if point.x < -24.0
                            || point.x >= self.canvas.width + 24.0
                            || point.y < 0.0
                            || point.y >= self.canvas.height
                        {
                            drag.hover(Some(DestinationProposal::onto_collection(
                                "desktop",
                                vec![DragOperation::Move],
                            )));
                        }
                        if matches!(drag.release(), Some(SessionOutcome::Dropped(_))) {
                            shell.publish(Message::SidebarAction(Action::RestoreIcon {
                                id: self.id,
                                owner: self.owner,
                                x: (self.origin.x + point.x).round() as i32,
                                y: (self.origin.y + point.y).round() as i32,
                            }));
                        }
                    }
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            _ => {}
        }
        self.child_update(tree, event, layout, cursor, renderer, shell, viewport);
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
        if tree
            .state
            .downcast_ref::<State>()
            .drag
            .as_ref()
            .is_some_and(DragSession::is_active)
        {
            mouse::Interaction::Grabbing
        } else {
            self.child.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'a, Message, iced::Theme, iced::Renderer>> {
        if tree.state.downcast_ref::<State>().drag.is_some() {
            return Vec::new();
        }
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
