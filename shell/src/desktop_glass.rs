//! Compositor glass for transparent application panes.
//!
//! Bind the window surface, wrap its view with window_glass, then mark glass_pane regions.
//! Unsupported surfaces use solid panels. Compositor sampling excludes the app's own content.

use crate::surface_input::SurfaceInput;
use enzui_shell::i18n::tr;
use enzui_theme::MaterialStyle;
use iced::advanced::{
    Renderer as _, Shell, Widget,
    layout::{self, Layout},
    overlay, renderer,
    widget::{Operation, Tree},
};
use iced::{Color, Element, Event, Length, Rectangle, Size, Task, Vector, mouse, window};
use std::cell::{Cell, RefCell};

/// Binds a reachable Wayland surface; `shared` reuses globals from the same connection.
pub fn bind(
    id: window::Id,
    shared: Option<crate::surface_input::Globals>,
) -> Task<Result<SurfaceInput, String>> {
    iced_runtime::window::wayland_parent_export(id).then(move |payload| {
        let shared = shared.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    crate::surface_input::acquire_for_glass(payload, shared)
                })
                .await
                .unwrap_or_else(|_| Err(tr("shell-glass-worker-stopped").into()))
            },
            |result| result,
        )
    })
}

/// Wakes when compositor backdrop measurements change, allowing adaptive ink to advance.
pub fn luminance_changes() -> iced::Subscription<()> {
    iced::Subscription::run(|| {
        iced::futures::stream::unfold(
            crate::surface_input::luminance_changes(),
            |mut changes| async move {
                changes.changed().await.ok()?;
                Some(((), changes))
            },
        )
    })
}

/// How a window's glass looks: one material for all its panes.
#[derive(Clone, Debug, Default)]
pub struct WindowGlass {
    /// The bound surface; `None` until [`bind`] completes.
    pub input: Option<SurfaceInput>,
    /// `None` paints every pane solid.
    pub style: Option<MaterialStyle>,
    /// The glass colour where the style keeps the theme's ink; under a chosen
    /// ink the glass takes that ink's colour.
    pub tint: Color,
    /// Adaptive ink mix: 1 is light, 0 is dark; `None` uses the theme.
    pub light_ink: Option<f32>,
    pub reduced_motion: bool,
    /// (blur, downward offset, opacity) around every pane.
    pub shadow: Option<(f32, f32, f32)>,
    /// The surface is being bound: draw nothing yet, so the window does not
    /// show solid for a frame before its glass arrives.
    pub pending: bool,
}

thread_local! {
    // While a glass window draws: its material and the panes placed so far.
    static PLACED: RefCell<Option<Placed>> = const { RefCell::new(None) };
}

// The window's glass while it draws: the theme its panes' content takes,
// their glass colour, and the panes placed so far.
struct Placed {
    theme: iced::Theme,
    tint: Color,
    panes: Vec<crate::surface_input::GlassRegion>,
}

/// `content` as a window whose [`glass_pane`]s show the desktop through glass.
pub fn window_glass<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    glass: &WindowGlass,
) -> Element<'a, Message> {
    Element::new(Root {
        content: content.into(),
        glass: glass.clone(),
        fixed_regions: None,
        extra: None,
    })
}

/// Updates fixed surface regions during drawing, before the content buffer commits.
pub fn fixed_glass<'a, Message: 'a>(
    content: Element<'a, Message>,
    glass: WindowGlass,
    regions: Vec<(Rectangle, f32)>,
) -> Element<'a, Message> {
    fixed_glass_with_extra(content, glass, regions, None)
}

/// Draws fixed regions with an optional independently styled glass region.
pub fn fixed_glass_with_extra<'a, Message: 'a>(
    content: Element<'a, Message>,
    glass: WindowGlass,
    regions: Vec<(Rectangle, f32)>,
    extra: Option<(Rectangle, f32, MaterialStyle)>,
) -> Element<'a, Message> {
    Element::new(Root {
        content,
        glass,
        fixed_regions: Some(regions),
        extra,
    })
}

/// A glass pane of its content's size, inside a [`window_glass`].
pub fn glass_pane<'a, Message>(content: impl Into<Element<'a, Message>>) -> GlassPane<'a, Message> {
    GlassPane {
        content: content.into(),
        radius: 12.0,
        bleed: Vector::ZERO,
        fallback: Color::TRANSPARENT,
        outline: Color::TRANSPARENT,
        shadow: iced::Shadow::default(),
    }
}

/// Builder returned by [`glass_pane`].
pub struct GlassPane<'a, Message> {
    content: Element<'a, Message>,
    radius: f32,
    bleed: Vector,
    fallback: Color,
    outline: Color,
    shadow: iced::Shadow,
}

impl<'a, Message> GlassPane<'a, Message> {
    /// Corner radius, logical pixels; 12 by default.
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
    /// Extends glass under opaque neighbours to hide the rim; negative values extend left or
    /// upward.
    #[must_use]
    pub fn bleed(mut self, x: f32, y: f32) -> Self {
        self.bleed = Vector::new(x, y);
        self
    }
    /// The fill without compositor glass.
    #[must_use]
    pub fn fallback(mut self, color: Color) -> Self {
        self.fallback = color;
        self
    }
    /// A hairline around the fill without compositor glass, so a solid pane
    /// stays distinct over a window of the same colour.
    #[must_use]
    pub fn outline(mut self, color: Color) -> Self {
        self.outline = color;
        self
    }
    /// A shadow under the fill without compositor glass; the window must
    /// leave room around the pane for it.
    #[must_use]
    pub fn shadow(mut self, shadow: iced::Shadow) -> Self {
        self.shadow = shadow;
        self
    }
    fn shape(&self, bounds: Rectangle) -> Rectangle {
        let x = bounds.x + self.bleed.x.min(0.0);
        let y = bounds.y + self.bleed.y.min(0.0);
        Rectangle {
            x,
            y,
            width: bounds.width + self.bleed.x.abs(),
            height: bounds.height + self.bleed.y.abs(),
        }
    }
}

impl<'a, Message: 'a> From<GlassPane<'a, Message>> for Element<'a, Message> {
    fn from(pane: GlassPane<'a, Message>) -> Self {
        Element::new(pane)
    }
}

struct Root<'a, Message> {
    content: Element<'a, Message>,
    glass: WindowGlass,
    fixed_regions: Option<Vec<(Rectangle, f32)>>,
    extra: Option<(Rectangle, f32, MaterialStyle)>,
}

impl<Message> Root<'_, Message> {
    fn material(&self) -> Option<MaterialStyle> {
        self.glass.style.filter(|style| {
            !style.tokens().is_opaque()
                && self
                    .glass
                    .input
                    .as_ref()
                    .is_some_and(SurfaceInput::glass_supported)
        })
    }
}

// Changes in glass availability or ink require another frame until the drawn state agrees.
#[derive(Default)]
struct Drawn(Cell<Option<(bool, Option<u8>)>>);

impl WindowGlass {
    fn drawn(&self, material: Option<MaterialStyle>) -> (bool, Option<u8>) {
        (
            material.is_some(),
            self.light_ink
                .map(|ink| (ink.clamp(0.0, 1.0) * 16.0).round() as u8),
        )
    }
}

macro_rules! delegate {
    () => {
        fn size(&self) -> Size<Length> {
            self.content.as_widget().size()
        }
        fn diff(&mut self, tree: &mut Tree) {
            tree.diff_children(std::slice::from_mut(&mut self.content));
        }
        fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits);
            tree.size = tree.children[0].size;
            tree.children[0].translation = Vector::ZERO;
        }
        fn frame_interest(&self, tree: &Tree) -> bool {
            tree.children.iter().any(Tree::has_frame_interest) || self.stale(tree)
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
            let child = layout.child(&tree.children, 0);
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                shell,
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
            let child = layout.child(&tree.children, 0);
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                child,
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
            let child = layout.child(&tree.children, 0);
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                child,
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
        ) -> Vec<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
            let child = layout.child(&tree.children, 0);
            self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                child,
                renderer,
                viewport,
                translation,
                window,
            )
        }
    };
}

trait Stale {
    fn stale(&self, tree: &Tree) -> bool;
}
impl<Message> Stale for Root<'_, Message> {
    fn stale(&self, tree: &Tree) -> bool {
        tree.state.downcast_ref::<Drawn>().0.get() != Some(self.glass.drawn(self.material()))
    }
}
impl<Message> Stale for GlassPane<'_, Message> {
    fn stale(&self, _: &Tree) -> bool {
        false
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Root<'_, Message> {
    delegate!();
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Drawn>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Drawn::default())
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
        let input = self.glass.input.as_ref();
        let material = self.material();
        tree.state
            .downcast_ref::<Drawn>()
            .0
            .set(Some(self.glass.drawn(material)));
        if self.glass.pending && input.is_none() && self.glass.style.is_some() {
            return;
        }
        let placed = material.map(|style| {
            let (theme, tint) = crate::glass_ink::look(theme, style, self.glass.light_ink);
            let tint = if self.glass.light_ink.is_some() {
                tint
            } else {
                Color {
                    a: 1.0,
                    ..self.glass.tint
                }
            };
            Placed {
                theme,
                tint,
                panes: self
                    .fixed_regions
                    .as_ref()
                    .map_or_else(Vec::new, |regions| {
                        regions
                            .iter()
                            .map(|&(bounds, radius)| crate::surface_input::GlassRegion {
                                bounds,
                                radius,
                                tint,
                            })
                            .collect()
                    }),
            }
        });
        let outer = PLACED.with(|cell| cell.replace(placed));
        let child = layout.child(&tree.children, 0);
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child,
            cursor,
            viewport,
        );
        let placed = PLACED.with(|cell| cell.replace(outer));
        if let Some(input) = input {
            if let Some((bounds, radius, extra)) = self.extra {
                let reduced = self.glass.reduced_motion;
                let panes = material
                    .into_iter()
                    .zip(placed)
                    .flat_map(|(style, placed)| {
                        let tokens = crate::reveal::glass_tokens(style, reduced);
                        placed.panes.into_iter().map(move |pane| (pane, tokens))
                    });
                let region = crate::surface_input::GlassRegion {
                    bounds,
                    radius,
                    tint: self.glass.tint,
                };
                let tokens = crate::reveal::glass_tokens(extra, reduced);
                if !input.mixed_glass_regions(
                    panes.take(31).chain([(region, tokens)]),
                    self.glass.shadow,
                ) {
                    input.clear_glass();
                }
                return;
            }
            match (material, placed) {
                (Some(style), Some(placed)) if !placed.panes.is_empty() => {
                    let tokens = crate::reveal::glass_tokens(style, self.glass.reduced_motion);
                    if !input.glass_regions(&placed.panes, &tokens, self.glass.shadow) {
                        // Too late for this frame's panes; the next one is solid.
                        input.clear_glass();
                    }
                }
                _ => input.clear_glass(),
            }
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for GlassPane<'_, Message> {
    delegate!();
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
        let shape = self.shape(layout.bounds());
        let glass_theme = PLACED.with(|cell| {
            cell.borrow_mut().as_mut().map(|placed| {
                placed.panes.push(crate::surface_input::GlassRegion {
                    bounds: shape,
                    radius: self.radius,
                    tint: placed.tint,
                });
                placed.theme.clone()
            })
        });
        let child = layout.child(&tree.children, 0);
        match glass_theme {
            Some(theme) => {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    &theme,
                    style,
                    child,
                    cursor,
                    viewport,
                );
                // On glass an outlined pane keeps the hairline rim the island
                // pills and popovers have, so its edge holds over anything.
                if self.outline.a > 0.0 {
                    let tint = self.fallback;
                    let lit = 0.2126 * tint.r + 0.7152 * tint.g + 0.0722 * tint.b > 0.5;
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: shape,
                            border: iced::Border {
                                radius: self.radius.into(),
                                width: 1.0,
                                color: if lit {
                                    Color::from_rgba(0.0, 0.0, 0.0, 0.08)
                                } else {
                                    Color::from_rgba(1.0, 1.0, 1.0, 0.06)
                                },
                            },
                            ..Default::default()
                        },
                        Color::TRANSPARENT,
                    );
                }
            }
            None => {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: shape,
                        border: iced::Border {
                            radius: self.radius.into(),
                            width: if self.outline.a > 0.0 { 1.0 } else { 0.0 },
                            color: self.outline,
                        },
                        shadow: self.shadow,
                        ..Default::default()
                    },
                    self.fallback,
                );
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    child,
                    cursor,
                    viewport,
                );
            }
        }
    }
}
