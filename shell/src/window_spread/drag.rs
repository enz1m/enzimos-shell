use super::{Message, SpreadMessage};
use iced::advanced::{
    Layout, Renderer as _, Shell, Widget, layout, renderer,
    widget::{Id, Operation, Tree, operation::Focusable, tree},
};
use iced::{Border, Color, Element, Event, Length, Point, Rectangle, Size, mouse, touch, window};

pub(super) fn window_widget(id: u64) -> Id {
    format!("overview-window-{id:x}").into()
}

pub(super) fn desktop_widget(id: u32) -> Id {
    format!("overview-desktop-{id}").into()
}

pub(super) fn overview_drag<'a>(
    child: Element<'a, Message>,
    windows: impl Iterator<Item = u64>,
    desktops: impl Iterator<Item = u32>,
    accent: Color,
) -> Element<'a, Message> {
    Element::new(OverviewDrag {
        child,
        windows: windows.map(|id| (window_widget(id), id)).collect(),
        desktops: desktops.map(|id| (desktop_widget(id), id)).collect(),
        accent,
    })
}

struct OverviewDrag<'a> {
    child: Element<'a, Message>,
    windows: Vec<(Id, u64)>,
    desktops: Vec<(Id, u32)>,
    accent: Color,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Pointer,
    Touch(touch::Finger),
}

struct Press {
    owner: Owner,
    window: u64,
    source: Rectangle,
    start: Point,
    dragging: bool,
    target: Option<(u32, Rectangle)>,
}

#[derive(Default)]
struct State {
    press: Option<Press>,
}

struct Hit<'a, T> {
    position: Point,
    ids: &'a [(Id, T)],
    found: Option<(T, Rectangle)>,
}

impl<T: Copy + Send + Sync> Operation for Hit<'_, T> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        if self.found.is_none() {
            operate(self);
        }
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _: &mut dyn Focusable) {
        if self.found.is_none()
            && bounds.contains(self.position)
            && let Some((_, value)) = self.ids.iter().find(|(candidate, _)| Some(candidate) == id)
        {
            self.found = Some((*value, bounds));
        }
    }
}

fn hit<T: Copy + Send + Sync>(
    child: &mut dyn Widget<Message, iced::Theme, iced::Renderer>,
    tree: &mut Tree,
    layout: Layout,
    renderer: &iced::Renderer,
    viewport: &Rectangle,
    ids: &[(Id, T)],
    position: Point,
) -> Option<(T, Rectangle)> {
    if !viewport.contains(position) {
        return None;
    }
    let mut hit = Hit {
        position,
        ids,
        found: None,
    };
    child.operate(tree, layout, viewport, renderer, &mut hit);
    hit.found
}

impl Widget<Message, iced::Theme, iced::Renderer> for OverviewDrag<'_> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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
        let child = self.child.as_widget_mut();
        let child_tree = &mut tree.children[0];
        let down = match *event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                cursor.position().map(|p| (Owner::Pointer, p))
            }
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                Some((Owner::Touch(id), position))
            }
            _ => None,
        };
        if let Some((owner, start)) = down {
            if state.press.is_none()
                && let Some((window, source)) = hit(
                    child,
                    child_tree,
                    layout,
                    renderer,
                    viewport,
                    &self.windows,
                    start,
                )
            {
                state.press = Some(Press {
                    owner,
                    window,
                    source,
                    start,
                    dragging: false,
                    target: None,
                });
            }
            if state.press.is_some() {
                shell.capture_event();
                return;
            }
        }

        if let Some(press) = &mut state.press {
            let cancelled = match *event {
                Event::Mouse(mouse::Event::CursorLeft) => press.owner == Owner::Pointer,
                Event::Touch(touch::Event::FingerLost { id, .. }) => {
                    press.owner == Owner::Touch(id)
                }
                Event::Window(window::Event::Unfocused) => true,
                _ => false,
            };
            if cancelled {
                state.press = None;
                shell.request_redraw();
                return;
            }

            let (position, release) = match *event {
                Event::Mouse(mouse::Event::CursorMoved { position })
                    if press.owner == Owner::Pointer =>
                {
                    (Some(position), false)
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if press.owner == Owner::Pointer =>
                {
                    (cursor.position(), true)
                }
                Event::Touch(touch::Event::FingerMoved { id, position })
                    if press.owner == Owner::Touch(id) =>
                {
                    (Some(position), false)
                }
                Event::Touch(touch::Event::FingerLifted { id, position })
                    if press.owner == Owner::Touch(id) =>
                {
                    (Some(position), true)
                }
                _ => (None, false),
            };
            if let Some(position) = position {
                press.dragging |= press.start.distance(position)
                    >= enzui_core::desktop_settings::input()
                        .drag_threshold
                        .max(4.0);
                if press.dragging {
                    press.target = hit(
                        child,
                        child_tree,
                        layout,
                        renderer,
                        viewport,
                        &self.desktops,
                        position,
                    );
                }
            }
            if release {
                if let Some(message) = if press.dragging {
                    press
                        .target
                        .filter(|_| position.is_some())
                        .map(|(desktop, _)| SpreadMessage::Move(press.window, desktop))
                } else {
                    position
                        .filter(|p| press.source.contains(*p))
                        .map(|_| SpreadMessage::Pick(press.window))
                } {
                    shell.publish(Message::Spread(message));
                }
                state.press = None;
            }
            if release || position.is_some() {
                shell.request_redraw();
                shell.capture_event();
                return;
            }
            if matches!(event, Event::Mouse(_) | Event::Touch(_)) {
                shell.capture_event();
                return;
            }
        }

        child.update(child_tree, event, layout, cursor, renderer, shell, viewport);
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
        let state = tree.state.downcast_ref::<State>();
        let Some(press) = state.press.as_ref().filter(|press| press.dragging) else {
            return;
        };
        renderer.with_layer(layout.bounds(), |renderer| {
            for (bounds, width) in std::iter::once((press.source, 0.0))
                .chain(press.target.map(|(_, bounds)| (bounds, 3.0)))
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: Border {
                            radius: 11.0.into(),
                            color: self.accent,
                            width,
                        },
                        ..Default::default()
                    },
                    Color {
                        a: 0.24,
                        ..self.accent
                    },
                );
            }
        });
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
            .press
            .as_ref()
            .is_some_and(|press| press.owner == Owner::Pointer && press.dragging)
        {
            return mouse::Interaction::Grabbing;
        }
        self.child.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}
