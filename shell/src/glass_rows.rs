//! Compositor glass regions derived from each row's laid-out bounds.
use crate::surface_input::SurfaceInput;
use iced::advanced::{
    Shell, Widget,
    layout::{self, Layout},
    renderer,
    widget::{Operation, Tree},
};
use iced::{Color, Element, Event, Length, Rectangle, Size, Vector, mouse};

/// The glass every row gets: the framework's tokens, the tint, the corner
/// radius and the shadow (blur, offset, opacity).
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub tokens: enzui_theme::MaterialTokens,
    pub tint: Color,
    pub radius: f32,
    pub shadow: Option<(f32, f32, f32)>,
    /// Room on each row's left that the glass leaves out (a badge's overhang).
    pub inset_left: f32,
}

/// `child` must lay its rows out as its own children (a column), each made by
/// [`shifted`]; a row's glass moves with its shift.
pub fn glass_rows<'a, Message: 'a>(
    child: Element<'a, Message>,
    input: Option<SurfaceInput>,
    look: Option<Look>,
) -> Element<'a, Message> {
    Element::new(GlassRows {
        child,
        input,
        look,
        trailing: None,
    })
}

/// Restricts surface input to row bounds; `trailing` reserves a square control without glass.
pub fn glass_rows_input<'a, Message: 'a>(
    child: Element<'a, Message>,
    input: Option<SurfaceInput>,
    look: Option<Look>,
    trailing: Option<f32>,
) -> Element<'a, Message> {
    Element::new(GlassRows {
        child,
        input,
        look,
        trailing: Some(trailing.unwrap_or(0.0)),
    })
}

/// Like [`glass_rows_input`], but the last row neither takes input nor gets
/// glass (a control that is not shown at the moment).
pub fn glass_rows_input_without_last<'a, Message: 'a>(
    child: Element<'a, Message>,
    input: Option<SurfaceInput>,
    look: Option<Look>,
) -> Element<'a, Message> {
    Element::new(GlassRows {
        child,
        input,
        look,
        trailing: Some(-1.0),
    })
}

/// Moves layout and hit regions by `dx`; `open` scales occupied height without clipping the
/// child.
pub fn shifted<'a, Message: 'a>(
    child: impl Into<Element<'a, Message>>,
    dx: f32,
    open: f32,
) -> Element<'a, Message> {
    Element::new(Shifted {
        child: child.into(),
        dx,
        open: open.clamp(0.0, 1.0),
    })
}

struct GlassRows<'a, Message> {
    child: Element<'a, Message>,
    input: Option<SurfaceInput>,
    look: Option<Look>,
    // `Some` limits input to the rows; a positive size is a trailing control.
    trailing: Option<f32>,
}

struct Shifted<'a, Message> {
    child: Element<'a, Message>,
    dx: f32,
    open: f32,
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Shifted<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        tree.children[0].translation = Vector::new(self.dx, 0.0);
        let size = tree.children[0].size;
        tree.size = Size::new(size.width, size.height * self.open);
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
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the wrapper always has one child layout");
        self.child
            .as_widget_mut()
            .update(tree, event, layout, cursor, renderer, shell, viewport);
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
        let (layout, tree) = layout
            .iter(&tree.children)
            .next()
            .expect("the wrapper always has one child layout");
        self.child
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the wrapper always has one child layout");
        self.child
            .as_widget_mut()
            .operate(tree, layout, viewport, renderer, operation);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let (layout, tree) = layout
            .iter(&tree.children)
            .next()
            .expect("the wrapper always has one child layout");
        self.child
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
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
        let (layout, tree) = layout
            .iter_mut(&mut tree.children)
            .next()
            .expect("the wrapper always has one child layout");
        self.child
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation, window)
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for GlassRows<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
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
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
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
        if let Some(input) = &self.input {
            let mut rows: Vec<Rectangle> = layout
                .children(&tree.children[0].children)
                .zip(&tree.children[0].children)
                .map(|(row, row_tree)| {
                    row.bounds()
                        + row_tree
                            .children
                            .first()
                            .map_or(Vector::ZERO, |inner| inner.translation)
                })
                .filter(|bounds| bounds.width >= 1.0 && bounds.height >= 1.0)
                .collect();
            if self.trailing.is_some_and(|size| size < 0.0) {
                rows.pop();
            }
            let control = match self.trailing {
                Some(size) if size > 0.0 => rows.pop().map(|row| Rectangle {
                    x: row.center_x() - size / 2.0,
                    y: row.y + (row.height - size).max(0.0) / 2.0,
                    width: size,
                    height: size.min(row.height),
                }),
                _ => None,
            };
            match self.look {
                Some(look) => {
                    let regions: Vec<(Rectangle, f32)> = rows
                        .iter()
                        .map(|bounds| Rectangle {
                            x: bounds.x + look.inset_left,
                            width: (bounds.width - look.inset_left).max(0.0),
                            ..*bounds
                        })
                        .map(|bounds| (bounds, look.radius.min(bounds.height / 2.0)))
                        .collect();
                    input.glass(&regions, &look.tokens, look.tint, look.shadow);
                }
                None => input.clear_glass(),
            }
            if self.trailing.is_some() {
                let mut region: Vec<Rectangle> = rows.clone();
                region.extend(control);
                input.rectangles(&region);
            }
        }
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
