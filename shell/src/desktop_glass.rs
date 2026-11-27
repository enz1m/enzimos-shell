//! Compositor glass for transparent application panes.
//!
//! Bind the window surface, wrap its view with window_glass, then mark glass_pane regions.
//! Unsupported surfaces use solid panels. Compositor sampling excludes the app's own content.

use crate::surface_input::{GlassRegion, SurfaceInput};
use enzui_shell::i18n::tr;
use enzui_theme::{EnzTheme, MaterialStyle, text_defaults::TextDefaultsScope};
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
        input_mask: None,
        overlay_defaults: None,
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

/// Input shape published with the material regions of a fixed glass surface.
pub enum InputMask {
    /// Popups capture across the entire surface.
    Full,
    /// Input follows each rounded rectangle.
    Rounded {
        regions: Vec<(Rectangle, iced::border::Radius)>,
        canvas: Size,
    },
}

/// Publishes the draw's input and glass regions in the same surface commit.
/// The overlay theme supplies the rounded menu chrome on glass and on fallback.
pub fn fixed_glass_with_input<'a, Message: 'a>(
    content: Element<'a, Message>,
    glass: WindowGlass,
    regions: Vec<(Rectangle, iced::border::Radius)>,
    input_mask: InputMask,
    overlay_defaults: Option<EnzTheme>,
) -> Element<'a, Message> {
    Element::new(Root {
        content,
        glass,
        fixed_regions: Some(regions),
        extra: None,
        input_mask: Some(input_mask),
        overlay_defaults: overlay_defaults.map(Box::new),
    })
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
        fixed_regions: Some(
            regions
                .into_iter()
                .map(|(bounds, radius)| (bounds, radius.into()))
                .collect(),
        ),
        extra,
        input_mask: None,
        overlay_defaults: None,
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
    fixed_regions: Option<Vec<(Rectangle, iced::border::Radius)>>,
    extra: Option<(Rectangle, f32, MaterialStyle)>,
    input_mask: Option<InputMask>,
    overlay_defaults: Option<Box<EnzTheme>>,
}

impl<Message> Root<'_, Message> {
    fn overlay_defaults(&self) -> Option<TextDefaultsScope> {
        self.overlay_defaults
            .as_deref()
            .map(TextDefaultsScope::enter)
    }

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

    fn with_menu_glass<'a>(
        glass: &'a WindowGlass,
        fixed: &'a [(Rectangle, iced::border::Radius)],
        defaults: &'a EnzTheme,
        viewport: Rectangle,
        overlays: &mut Vec<overlay::Element<'a, Message, iced::Theme, iced::Renderer>>,
    ) where
        Message: 'a,
    {
        if overlays
            .last()
            .is_some_and(|overlay| overlay.as_overlay().grabs())
            && let Some(menu) = overlays.pop()
        {
            overlays.push(overlay::Element::new(Box::new(MenuGlass {
                menu,
                glass,
                fixed,
                defaults,
                viewport,
            })));
        }
    }
}

struct MenuGlass<'a, Message> {
    menu: overlay::Element<'a, Message, iced::Theme, iced::Renderer>,
    glass: &'a WindowGlass,
    fixed: &'a [(Rectangle, iced::border::Radius)],
    defaults: &'a EnzTheme,
    viewport: Rectangle,
}

impl<Message> overlay::Overlay<Message, iced::Theme, iced::Renderer> for MenuGlass<'_, Message> {
    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        let menu = self.menu.as_overlay();
        let mut defaults = self.defaults.clone();
        if let Some((input, material)) = self.glass.input.as_ref().zip(self.glass.style) {
            // Context menu layout exposes one child per level, including open submenus.
            let nodes = menu
                .node()
                .filter(|_| {
                    menu.area()
                        .is_some_and(|area| area.is_within(&self.viewport))
                })
                .map_or(&[][..], layout::Node::children);
            let tint = self
                .glass
                .effective_tint(crate::glass_ink::look(theme, material, self.glass.light_ink).1);
            let regions = self
                .fixed
                .iter()
                .map(|&(bounds, radius)| GlassRegion {
                    bounds,
                    radius,
                    tint,
                })
                .chain(nodes.iter().map(|node| {
                    let bounds = node.bounds();
                    let radius = defaults
                        .window_radius
                        .min(bounds.width.min(bounds.height) / 2.0);
                    GlassRegion {
                        bounds,
                        radius: radius.into(),
                        tint,
                    }
                }))
                .collect::<Vec<_>>();
            let _commit = input.commit_together().with_buffer();
            if regions.is_empty() {
                input.clear_glass();
            } else if input.glass_regions(
                &regions,
                &crate::reveal::glass_tokens(material, self.glass.reduced_motion),
                self.glass.shadow,
            ) && !nodes.is_empty()
            {
                defaults.palette.surface_elevated = Color::TRANSPARENT;
                defaults.palette.border = crate::reveal::glass_rim(tint);
                // The material protocol takes settled rectangles, so the menu draws at that size.
                defaults.motion.reduced_motion = true;
            }
        }
        menu.draw(renderer, &defaults.iced_theme(), style, cursor);
    }

    fn operate(&mut self, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.menu.as_overlay_mut().operate(renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        self.menu
            .as_overlay_mut()
            .update(event, cursor, renderer, shell);
    }

    fn mouse_interaction(
        &self,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.menu.as_overlay().mouse_interaction(cursor, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        renderer: &iced::Renderer,
    ) -> Vec<overlay::Element<'a, Message, iced::Theme, iced::Renderer>> {
        self.menu.as_overlay_mut().overlay(renderer)
    }

    fn index(&self) -> f32 {
        self.menu.as_overlay().index()
    }

    fn node(&self) -> Option<&layout::Node> {
        self.menu.as_overlay().node()
    }

    fn area(&self) -> Option<Rectangle> {
        self.menu.as_overlay().area()
    }

    fn anchor(&self) -> Option<Rectangle> {
        self.menu.as_overlay().anchor()
    }

    fn grabs(&self) -> bool {
        self.menu.as_overlay().grabs()
    }
}

// Changes in glass availability or ink require another frame until the drawn state agrees.
#[derive(Default)]
struct Drawn {
    appearance: Cell<Option<(bool, Option<u8>)>>,
    fixed_regions: RefCell<Vec<(Rectangle, iced::border::Radius)>>,
    menu_glass: Cell<bool>,
}

impl WindowGlass {
    fn effective_tint(&self, ink_tint: Color) -> Color {
        if self.light_ink.is_some() {
            ink_tint
        } else {
            Color {
                a: 1.0,
                ..self.tint
            }
        }
    }

    fn drawn(&self, material: Option<MaterialStyle>) -> (bool, Option<u8>) {
        (
            material.is_some(),
            self.light_ink
                .map(|ink| (ink.clamp(0.0, 1.0) * 16.0).round() as u8),
        )
    }
}

macro_rules! delegate {
    ($overlays:ident) => {
        fn size(&self) -> Size<Length> {
            self.content.as_widget().size()
        }
        fn diff(&mut self, tree: &mut Tree) {
            let _defaults = self.overlay_defaults();
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
            let _defaults = self.overlay_defaults();
            let child = layout.child(&tree.children, 0);
            let overlays = self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                child,
                renderer,
                viewport,
                translation,
                window,
            );
            $overlays!(self, overlays, window)
        }
    };
}

trait Stale {
    fn stale(&self, tree: &Tree) -> bool;
}
impl<Message> Stale for Root<'_, Message> {
    fn stale(&self, tree: &Tree) -> bool {
        tree.state.downcast_ref::<Drawn>().appearance.get()
            != Some(self.glass.drawn(self.material()))
    }
}
impl<Message> Stale for GlassPane<'_, Message> {
    fn stale(&self, _: &Tree) -> bool {
        false
    }
}

macro_rules! menu_overlay {
    ($root:ident, $overlays:ident, $window:ident) => {{
        let mut overlays = $overlays;
        if let Some((fixed, defaults)) = $root
            .fixed_regions
            .as_deref()
            .zip($root.overlay_defaults.as_deref())
        {
            Self::with_menu_glass(
                &$root.glass,
                fixed,
                defaults,
                Rectangle::with_size($window),
                &mut overlays,
            );
        }
        overlays
    }};
}

macro_rules! unchanged_overlays {
    ($pane:ident, $overlays:ident, $window:ident) => {
        $overlays
    };
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Root<'_, Message> {
    delegate!(menu_overlay);
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
        let drawn = tree.state.downcast_ref::<Drawn>();
        let menu_paints_glass = material.is_some() && self.overlay_defaults.is_some();
        let menu_changed = drawn.menu_glass.replace(menu_paints_glass) != menu_paints_glass;
        let _commit = self.input_mask.as_ref().and(input).map(|input| {
            if menu_paints_glass {
                return input.commit_together().with_buffer();
            }
            let batch = input.commit_together();
            let visible = if material.is_some() {
                self.fixed_regions.as_deref().unwrap_or_default()
            } else {
                &[]
            };
            let mut previous = drawn.fixed_regions.borrow_mut();
            if previous.as_slice() == visible && !menu_changed {
                batch
            } else {
                previous.clear();
                previous.extend_from_slice(visible);
                // Publish moving or disappearing glass with the matching buffer.
                batch.with_buffer()
            }
        });
        if let Some((input, mask)) = input.zip(self.input_mask.as_ref()) {
            match mask {
                InputMask::Full => input.reset(),
                InputMask::Rounded { regions, canvas } => input.shaped_regions(regions, *canvas),
            };
        }
        drawn.appearance.set(Some(self.glass.drawn(material)));
        if self.glass.pending && input.is_none() && self.glass.style.is_some() {
            return;
        }
        let placed = material.map(|style| {
            let (theme, ink_tint) = crate::glass_ink::look(theme, style, self.glass.light_ink);
            let tint = self.glass.effective_tint(ink_tint);
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
        if let Some(input) = input.filter(|_| !menu_paints_glass) {
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
                    radius: radius.into(),
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

impl<Message> GlassPane<'_, Message> {
    fn overlay_defaults(&self) -> Option<TextDefaultsScope> {
        None
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for GlassPane<'_, Message> {
    delegate!(unchanged_overlays);
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
                    radius: self.radius.into(),
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
