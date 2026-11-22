//! Lays its content out at a height of its own, whatever room it is given,
//! and fills that room. Inside a clipping card that is growing or shrinking,
//! the content keeps its final layout and the card reveals it; laid out in
//! the card's passing height, rows past the room were squeezed flat.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget;
use iced::advanced::{Layout, Shell, Widget};
use iced::{Element, Event, Length, Rectangle, Size, Vector, touch};

pub struct NaturalHeight<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    height: f32,
}

/// `content` laid out `height` tall, filling the room it is given.
pub fn natural_height<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    height: f32,
) -> NaturalHeight<'a, Message, Theme, Renderer> {
    NaturalHeight {
        content: content.into(),
        height: height.max(0.0),
    }
}

fn visible_cursor(cursor: mouse::Cursor, viewport: Rectangle) -> mouse::Cursor {
    if cursor.is_over(viewport) {
        cursor
    } else {
        cursor.obstruct()
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for NaturalHeight<'_, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) {
        let limits = limits.width(Length::Fill).height(Length::Fill);
        let room = Size::new(limits.max().width, self.height);
        self.content.as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::new(0.0, room.height), room),
        );
        tree.children[0].translation = Vector::ZERO;
        tree.size = limits.resolve(Length::Fill, Length::Fill, tree.children[0].size);
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the content is the only child");
        self.content
            .as_widget_mut()
            .operate(tree, layout, viewport, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let viewport = layout.bounds().intersection(viewport).unwrap_or_default();
        // A clipped row must not take input from the footer covering its old bounds.
        let lost;
        let event = match event {
            Event::Touch(touch::Event::FingerPressed { position, .. })
                if !viewport.contains(*position) =>
            {
                return;
            }
            Event::Touch(touch::Event::FingerLifted { id, position })
                if !viewport.contains(*position) =>
            {
                lost = Event::Touch(touch::Event::FingerLost {
                    id: *id,
                    position: *position,
                });
                &lost
            }
            _ => event,
        };
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the content is the only child");
        self.content.as_widget_mut().update(
            tree,
            event,
            layout,
            visible_cursor(cursor, viewport),
            renderer,
            shell,
            &viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let Some(viewport) = layout.bounds().intersection(viewport) else {
            return mouse::Interaction::default();
        };
        let (layout, tree) = layout
            .iter(&tree.children)
            .next()
            .expect("the content is the only child");
        self.content.as_widget().mouse_interaction(
            tree,
            layout,
            visible_cursor(cursor, viewport),
            &viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let cursor = visible_cursor(cursor, *viewport);
        let (layout, tree) = layout
            .iter(&tree.children)
            .next()
            .expect("the content is the only child");
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the content is the only child");
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}

impl<'a, Message, Theme, Renderer> From<NaturalHeight<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(widget: NaturalHeight<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
