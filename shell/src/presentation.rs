use enzui_widgets::motion::{PanelMotion, SlideDirection};
use iced::advanced::{
    Renderer as _, Shell, Widget,
    layout::{self, Layout},
    renderer,
    widget::{self, Operation, Tree},
};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};
use std::collections::BTreeMap;

pub fn slide<'a, Message: Clone + 'a>(
    child: Element<'a, Message>,
    visible: bool,
    direction: SlideDirection,
    theme: &iced::Theme,
    hidden: Option<Message>,
) -> Element<'a, Message> {
    Element::new(Slide {
        child,
        visible,
        interactive: visible,
        direction,
        theme: panel_theme(theme, None),
        hidden,
        reveal: None,
        fit: None,
        input: None,
        glass: None,
        expressive: None,
    })
}
pub fn attached<'a, Message: Clone + 'a>(
    child: Element<'a, Message>,
    visible: bool,
    direction: SlideDirection,
    theme: &iced::Theme,
    hidden: Option<Message>,
    reveal: crate::reveal::Reveal,
) -> Element<'a, Message> {
    Element::new(Slide {
        child,
        visible,
        interactive: visible,
        direction,
        theme: panel_theme(theme, None),
        hidden,
        reveal: Some(reveal),
        fit: None,
        input: None,
        glass: None,
        expressive: None,
    })
}
pub fn attached_source<'a, Message: Clone + 'a>(
    child: Element<'a, Message>,
    interactive: bool,
    direction: SlideDirection,
    theme: &iced::Theme,
    reveal: crate::reveal::Reveal,
) -> Element<'a, Message> {
    Element::new(Slide {
        child,
        visible: true,
        interactive,
        direction,
        theme: panel_theme(theme, None),
        hidden: None,
        reveal: Some(reveal),
        fit: None,
        input: None,
        glass: None,
        expressive: None,
    })
}
/// Measures a panel from its laid-out children.
pub struct FitHeight {
    /// Widget IDs and the space each measurement needs beyond its bounds.
    pub targets: Vec<(widget::Id, f32)>,
    /// The smallest combined panel and shadow height in logical pixels.
    pub minimum: f32,
}

#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
pub fn attached_fitted<'a, Message: Clone + 'a>(
    child: Element<'a, Message>,
    visible: bool,
    direction: SlideDirection,
    theme: &iced::Theme,
    hidden: Option<Message>,
    reveal: crate::reveal::Reveal,
    fit: Option<FitHeight>,
    input: Option<crate::surface_input::SurfaceInput>,
    glass: Option<enzui_theme::MaterialStyle>,
    expressive: Option<enzui_theme::ExpressiveMotion>,
) -> Element<'a, Message> {
    Element::new(Slide {
        child,
        visible,
        interactive: visible,
        direction,
        theme: panel_theme(theme, expressive),
        hidden,
        reveal: Some(reveal),
        fit,
        input,
        glass,
        expressive,
    })
}
fn panel_theme(
    theme: &iced::Theme,
    expressive: Option<enzui_theme::ExpressiveMotion>,
) -> iced::Theme {
    enzui_theme::EnzTheme::from_iced(theme).map_or_else(
        || theme.clone(),
        |mut tokens| {
            // Opening a panel takes about a fifth of a second in the standard
            // style and about a third in the others: long enough to follow,
            // never long enough to wait for.
            let factor = match expressive {
                Some(enzui_theme::ExpressiveMotion::SlowSpatial) => 350.0 / 650.0,
                Some(enzui_theme::ExpressiveMotion::DefaultSpatial) => 320.0 / 500.0,
                Some(_) => 1.0,
                None => 0.85,
            };
            tokens.motion.normal_ms *= factor;
            tokens.iced_theme()
        },
    )
}
fn effects_token(
    expressive: Option<enzui_theme::ExpressiveMotion>,
) -> Option<enzui_theme::ExpressiveMotion> {
    use enzui_theme::ExpressiveMotion;
    expressive.map(|token| match token {
        ExpressiveMotion::FastSpatial => ExpressiveMotion::FastEffects,
        ExpressiveMotion::DefaultSpatial => ExpressiveMotion::DefaultEffects,
        ExpressiveMotion::SlowSpatial => ExpressiveMotion::SlowEffects,
        effect => effect,
    })
}

struct Slide<'a, Message> {
    child: Element<'a, Message>,
    visible: bool,
    interactive: bool,
    direction: SlideDirection,
    theme: iced::Theme,
    hidden: Option<Message>,
    reveal: Option<crate::reveal::Reveal>,
    fit: Option<FitHeight>,
    input: Option<crate::surface_input::SurfaceInput>,
    // The compositor's liquid glass for an island panel's body.
    glass: Option<enzui_theme::MaterialStyle>,
    expressive: Option<enzui_theme::ExpressiveMotion>,
}
struct SlideState {
    motion: PanelMotion,
    opacity: PanelMotion,
    extent: Size,
}
struct MeasureHeight {
    id: widget::Id,
    height: Option<f32>,
    matches: usize,
}
impl Operation for MeasureHeight {
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle, _: &Rectangle) {
        if id == Some(&self.id) {
            self.matches += 1;
            self.height = Some(bounds.height);
        }
    }
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
}
impl<Message> Slide<'_, Message> {
    fn opacity(&self, state: &SlideState) -> f32 {
        // A panel joined to the bar or frame is that surface growing out of
        // it: fading in, it showed darker than the frame beside it until it
        // settled. Only a floating panel fades.
        let joined = self.reveal.is_some_and(|reveal| !reveal.is_island());
        if joined {
            1.0
        } else if self.expressive.is_some() {
            state.opacity.sampled_progress()
        } else if self.reveal.is_some() {
            // The standard style fades in ahead of its settling.
            (state.motion.sampled_progress() * 1.6).min(1.0)
        } else {
            1.0
        }
    }
    fn composite(
        &self,
        state: &SlideState,
        size: Size,
    ) -> Option<(crate::reveal::Frame, iced::Transformation)> {
        let reveal = self.reveal.filter(|reveal| reveal.is_island())?;
        if self.expressive == Some(enzui_theme::ExpressiveMotion::DefaultSpatial) {
            return None;
        }
        let current = self.frame(state, size)?;
        let mut rest = reveal.frame(
            Size::new(size.width, state.extent.height.min(size.height)),
            1.0,
        );
        let anchor = if matches!(
            reveal.shape,
            crate::reveal::Shape::Controls { bottom: true, .. }
        ) {
            Vector::new(0.0, size.height - state.extent.height.min(size.height))
        } else {
            Vector::ZERO
        };
        rest.clip += anchor;
        rest.translation += anchor;
        for (rect, _) in &mut rest.fills {
            *rect += anchor;
        }
        if rest.clip.width <= 0.0 || rest.clip.height <= 0.0 {
            return None;
        }
        let scale = current.clip.width / rest.clip.width;
        let transform = iced::Transformation::translate(current.clip.x, current.clip.y)
            * iced::Transformation::scale(scale)
            * iced::Transformation::translate(-rest.clip.x, -rest.clip.y);
        Some((rest, settle_transform(transform, size)))
    }
    fn frame(&self, state: &SlideState, size: Size) -> Option<crate::reveal::Frame> {
        self.reveal.map(|reveal| {
            let extent = Size::new(size.width, state.extent.height.min(size.height));
            let anchor = if matches!(
                reveal.shape,
                crate::reveal::Shape::Controls { bottom: true, .. }
            ) {
                Vector::new(0.0, size.height - extent.height)
            } else {
                Vector::ZERO
            };
            let mut frame = match self.expressive {
                Some(motion) => crate::panel_geometry::frame(
                    reveal,
                    extent,
                    state.motion.sampled_progress(),
                    state.motion.sampled_spatial_progress(),
                    motion,
                ),
                None if reveal.is_island() => {
                    crate::panel_geometry::settle(reveal, extent, state.motion.sampled_progress())
                }
                None => reveal.frame(extent, state.motion.sampled_progress()),
            };
            if self.expressive.is_some()
                && !matches!(reveal.shape, crate::reveal::Shape::Controls { .. })
            {
                let distance =
                    if matches!(self.direction, SlideDirection::Up | SlideDirection::Down) {
                        extent.height
                    } else {
                        extent.width
                    };
                let correction = (state.motion.sampled_spatial_progress()
                    - state.motion.sampled_progress())
                    * distance;
                frame.translation += match self.direction {
                    SlideDirection::Up => Vector::new(0.0, correction),
                    SlideDirection::Down => Vector::new(0.0, -correction),
                    SlideDirection::Left => Vector::new(correction, 0.0),
                    SlideDirection::Right => Vector::new(-correction, 0.0),
                };
            }
            frame.translation += anchor;
            frame.clip += anchor;
            for (rect, _) in &mut frame.fills {
                *rect += anchor;
            }
            for (rect, _, _) in &mut frame.corners {
                *rect += anchor;
            }
            frame
        })
    }
}
fn settle_transform(transform: iced::Transformation, size: Size) -> iced::Transformation {
    let scale = transform.scale_factor();
    let delta = transform.translation();
    let displacement =
        delta.x.abs().max(delta.y.abs()) + (scale - 1.0).abs() * size.width.max(size.height);
    if displacement < 0.25 {
        iced::Transformation::IDENTITY
    } else {
        transform
    }
}
fn offset(direction: SlideDirection, size: Size, progress: f32) -> Vector {
    let remaining = 1.0 - progress.clamp(0.0, 1.0);
    match direction {
        SlideDirection::Up => Vector::new(0.0, -size.height * remaining),
        SlideDirection::Down => Vector::new(0.0, size.height * remaining),
        SlideDirection::Left => Vector::new(-size.width * remaining, 0.0),
        SlideDirection::Right => Vector::new(size.width * remaining, 0.0),
    }
}
fn fitted_height(canvas: f32, minimum: f32, candidates: impl IntoIterator<Item = f32>) -> f32 {
    candidates
        .into_iter()
        .filter(|height| height.is_finite())
        .reduce(f32::max)
        .map_or(canvas, |height| height.max(minimum).min(canvas))
}

impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for Slide<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<SlideState>()
    }
    fn state(&self) -> widget::tree::State {
        let mut state = PanelMotion::at_target(self.visible);
        state.set_expressive(self.expressive);
        state.observe_theme(&self.theme);
        let opacity =
            PanelMotion::at_target(self.visible).with_expressive(effects_token(self.expressive));
        opacity.observe_theme(&self.theme);
        widget::tree::State::new(SlideState {
            motion: state,
            opacity,
            extent: Size::ZERO,
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<SlideState>();
        state.motion.observe_theme(&self.theme);
        state.motion.set_expressive(self.expressive);
        state.motion.set_target_at_frame(self.visible);
        state.opacity.observe_theme(&self.theme);
        state.opacity.set_expressive(effects_token(self.expressive));
        state.opacity.set_target_at_frame(self.visible);
        tree.diff_children(std::slice::from_mut(&mut self.child));
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let size = tree.children[0].size;
        let mut extent = size;
        if let Some(fit) = &self.fit {
            let mut candidates = Vec::with_capacity(fit.targets.len());
            for (id, extra) in &fit.targets {
                let mut measurement = MeasureHeight {
                    id: id.clone(),
                    height: None,
                    matches: 0,
                };
                self.child.as_widget_mut().operate(
                    &mut tree.children[0],
                    Layout::new(size),
                    &Rectangle::INFINITE,
                    renderer,
                    &mut measurement,
                );
                if measurement.matches == 1
                    && let Some(height) = measurement.height
                {
                    // A fitted panel keeps its shadow room around it.
                    let margins = self
                        .reveal
                        .map_or(iced::Padding::ZERO, crate::reveal::Reveal::margins);
                    candidates.push(height + extra + margins.top + margins.bottom);
                }
            }
            extent.height = fitted_height(size.height, fit.minimum, candidates);
        }
        let state = tree.state.downcast_mut::<SlideState>();
        state.extent = extent;
        let delta = self.frame(state, size).map_or_else(
            || offset(self.direction, size, state.motion.sampled_progress()),
            |frame| frame.translation,
        );
        state.motion.note_layout_at_frame();
        state.opacity.note_layout_at_frame();
        tree.size = size;
        tree.children[0].translation = delta;
    }
    fn frame_interest(&self, tree: &Tree) -> bool {
        let state = tree.state.downcast_ref::<SlideState>();
        (state.motion.sampled_needs_layout() || state.opacity.sampled_needs_layout())
            || (self.interactive && tree.children.iter().any(Tree::has_frame_interest))
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
        let state = tree.state.downcast_mut::<SlideState>();
        if let Event::Window(iced::window::Event::RedrawRequested(at)) = event {
            state.motion.note_frame(*at);
            state.opacity.note_frame(*at);
            if state.motion.sampled_needs_layout() || state.opacity.sampled_needs_layout() {
                shell.invalidate_layout();
                shell.request_redraw();
            }
            if state.motion.take_hidden_completion_at_frame()
                && let Some(message) = &self.hidden
            {
                shell.publish(message.clone());
            }
        }
        if !self.interactive {
            return;
        }
        let bounds = layout.bounds();
        let composite = self.composite(state, bounds.size());
        let transform = composite.as_ref().map(|(_, transform)| {
            iced::Transformation::translate(bounds.x, bounds.y)
                * *transform
                * iced::Transformation::translate(-bounds.x, -bounds.y)
        });
        let clip = self
            .frame(state, bounds.size())
            .map_or(bounds, |frame| frame.clip + Vector::new(bounds.x, bounds.y));
        if let Some(clip) = clip.intersection(viewport) {
            let cursor = if cursor.is_over(clip) {
                cursor
            } else {
                cursor.levitate()
            };
            let projected_event =
                transform.map(|transform| crate::panel_geometry::inverse_event(event, transform));
            let event = projected_event.as_ref().unwrap_or(event);
            let (cursor, clip) = transform.map_or((cursor, clip), |transform| {
                (cursor * transform.inverse(), clip * transform.inverse())
            });
            let layout = layout.child(&tree.children, 0);
            self.child.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                cursor,
                renderer,
                shell,
                &clip,
            );
        }
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
        let cursor = if self.interactive {
            cursor
        } else {
            cursor.levitate()
        };
        let mut glass = false;
        if let Some(input) = &self.input {
            let _commit = input.commit_together();
            let island = self.reveal.is_some_and(crate::reveal::Reveal::is_island);
            let frame = self.frame(
                tree.state.downcast_ref::<SlideState>(),
                layout.bounds().size(),
            );
            if island || self.fit.is_some() {
                match &frame {
                    Some(frame) if self.interactive && frame.extended => {
                        input.shape(frame, layout.bounds().size());
                    }
                    _ => {
                        input.empty();
                    }
                }
            } else if self.interactive {
                input.reset();
            } else {
                input.empty();
            }
            // The glass follows the panel while it is shown at all, so a
            // closing panel keeps its body until it has slid away.
            glass = match (
                island,
                self.glass,
                self.reveal,
                frame.as_ref().filter(|frame| frame.extended),
            ) {
                (true, Some(style), Some(reveal), Some(frame)) => {
                    let regions: Vec<(Rectangle, f32)> = frame
                        .fills
                        .iter()
                        .map(|(bounds, radius)| {
                            (
                                *bounds,
                                radius
                                    .top_left
                                    .max(radius.top_right)
                                    .max(radius.bottom_left)
                                    .max(radius.bottom_right),
                            )
                        })
                        .collect();
                    let reduced = enzui_theme::EnzTheme::from_iced(&self.theme)
                        .is_some_and(|theme| theme.motion.reduced_motion);
                    input.glass(
                        &regions,
                        &crate::reveal::glass_tokens(style, reduced),
                        reveal.color,
                        reveal.glass_shadow(),
                    )
                }
                _ => {
                    input.clear_glass();
                    false
                }
            };
        }
        if tree
            .state
            .downcast_ref::<SlideState>()
            .motion
            .sampled_progress()
            <= f32::EPSILON
        {
            return;
        }
        if let Some(outer) = layout.bounds().intersection(viewport) {
            let bounds = layout.bounds();
            let state = tree.state.downcast_ref::<SlideState>();
            let composite = self.composite(state, bounds.size());
            let current = self.frame(state, bounds.size());
            if current.as_ref().is_some_and(|frame| !frame.extended) {
                return;
            }
            // On glass the content takes the glass palette, as inside an app's
            // own glass: its fills turn translucent and it reads as one piece.
            let on_glass = self
                .glass
                .filter(|_| glass)
                .map(|style| enzui_widgets::theme_lookup::on_material(theme, style));
            let theme = on_glass.as_ref().unwrap_or(theme);
            let opacity = self.opacity(state);
            let frame = composite.as_ref().map(|(rest, _)| rest.clone()).or(current);
            let transform =
                composite
                    .as_ref()
                    .map_or(iced::Transformation::IDENTITY, |(_, transform)| {
                        iced::Transformation::translate(bounds.x, bounds.y)
                            * *transform
                            * iced::Transformation::translate(-bounds.x, -bounds.y)
                    });
            let delta = transform.translation();
            let scale = transform.scale_factor();
            let origin = if (1.0 - scale).abs() > f32::EPSILON {
                iced::Point::new(delta.x / (1.0 - scale), delta.y / (1.0 - scale))
            } else {
                iced::Point::ORIGIN
            };
            let cursor = cursor * transform.inverse();
            let paint = |renderer: &mut iced::Renderer| {
                // A glass body is the compositor's; only the content draws here.
                if let (Some(reveal), Some(frame), false) = (self.reveal, frame.as_ref(), glass) {
                    reveal.draw(frame, bounds.position(), renderer, outer);
                }
                let clip = frame
                    .as_ref()
                    .map_or_else(|| Rectangle::with_size(bounds.size()), |frame| frame.clip)
                    + Vector::new(bounds.x, bounds.y);
                if let Some(viewport) = clip.intersection(&outer) {
                    let draw = |renderer: &mut iced::Renderer| {
                        self.child.as_widget().draw(
                            &tree.children[0],
                            renderer,
                            theme,
                            style,
                            layout.child(&tree.children, 0),
                            cursor,
                            &viewport,
                        );
                    };
                    if let Some(radius) = frame
                        .as_ref()
                        .filter(|_| self.reveal.is_some_and(crate::reveal::Reveal::is_island))
                        .and_then(|frame| {
                            frame
                                .fills
                                .iter()
                                .find(|(bounds, _)| *bounds == frame.clip)
                                .map(|(_, radius)| *radius)
                        })
                    {
                        renderer.with_layer_rounded(clip, radius, draw);
                    } else {
                        renderer.with_layer(clip, draw);
                    }
                }
                // A floating glass popover keeps the hairline rim its bar pill
                // has, so its edge holds on a wallpaper as dark as the glass.
                if let (Some(reveal), Some(frame), true) = (self.reveal, frame.as_ref(), glass)
                    && reveal.is_island()
                    && let [(body, radius)] = frame.fills.as_slice()
                {
                    renderer.fill_quad(
                        iced::advanced::renderer::Quad {
                            bounds: *body + Vector::new(bounds.x, bounds.y),
                            border: iced::Border {
                                radius: *radius,
                                width: 1.0,
                                color: crate::reveal::glass_rim(reveal.color),
                            },
                            ..Default::default()
                        },
                        iced::Color::TRANSPARENT,
                    );
                }
            };
            if (scale - 1.0).abs() > f32::EPSILON {
                renderer.with_layer_composite(outer, opacity, scale, 0.0, origin, paint);
            } else if opacity < 1.0 {
                renderer.with_layer_alpha(outer, opacity, paint);
            } else {
                renderer.with_layer(outer, paint);
            }
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        if self.interactive {
            let bounds = layout.bounds();
            let transform = self
                .composite(tree.state.downcast_ref::<SlideState>(), bounds.size())
                .map(|(_, transform)| {
                    iced::Transformation::translate(bounds.x, bounds.y)
                        * transform
                        * iced::Transformation::translate(-bounds.x, -bounds.y)
                });
            let layout = layout.child(&tree.children, 0);
            if let Some(transform) = transform {
                let mut projected = crate::panel_geometry::Projected {
                    operation,
                    transform,
                };
                self.child.as_widget_mut().operate(
                    &mut tree.children[0],
                    layout,
                    viewport,
                    renderer,
                    &mut projected,
                );
            } else {
                self.child.as_widget_mut().operate(
                    &mut tree.children[0],
                    layout,
                    viewport,
                    renderer,
                    operation,
                );
            }
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        let clip = self
            .frame(tree.state.downcast_ref::<SlideState>(), bounds.size())
            .map_or(bounds, |frame| frame.clip + Vector::new(bounds.x, bounds.y));
        let transform = self
            .composite(tree.state.downcast_ref::<SlideState>(), bounds.size())
            .map(|(_, transform)| {
                iced::Transformation::translate(bounds.x, bounds.y)
                    * transform
                    * iced::Transformation::translate(-bounds.x, -bounds.y)
            });
        let cursor = transform.map_or(cursor, |transform| cursor * transform.inverse());
        let clip = transform.map_or(clip, |transform| clip * transform.inverse());
        if self.interactive && cursor.is_over(clip) {
            self.child.as_widget().mouse_interaction(
                &tree.children[0],
                layout.child(&tree.children, 0),
                cursor,
                viewport,
                renderer,
            )
        } else {
            mouse::Interaction::None
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
        if !self.interactive
            || tree
                .state
                .downcast_ref::<SlideState>()
                .motion
                .sampled_animating()
        {
            return Vec::new();
        }
        let layout = layout.child(&tree.children, 0);
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

/// The bar's desktop marks: `(id, active)` per desktop; `spare` lists the
/// empty desktop kept ready at the end, drawn hollow so it does not read as
/// one more desktop with windows.
pub fn desktop_marks<'a, Message: 'a>(
    entries: Vec<(u32, bool)>,
    spare: Vec<u32>,
    theme: &iced::Theme,
    color: iced::Color,
) -> Element<'a, Message> {
    Element::new(Marks {
        entries,
        spare,
        theme: theme.clone(),
        color,
    })
}
struct Marks {
    entries: Vec<(u32, bool)>,
    spare: Vec<u32>,
    theme: iced::Theme,
    color: iced::Color,
}
struct MarkState(BTreeMap<u32, PanelMotion>);
impl Marks {
    fn reconcile(&self, state: &mut MarkState) {
        state
            .0
            .retain(|id, _| self.entries.iter().any(|(next, _)| id == next));
        for &(id, active) in &self.entries {
            let motion = state
                .0
                .entry(id)
                .or_insert_with(|| PanelMotion::at_target(active));
            motion.observe_theme(&self.theme);
            motion.set_target_at_frame(active);
        }
    }
}
impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Marks {
    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed((self.entries.len() * 12 + 4) as f32),
            Length::Fixed(4.0),
        )
    }
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<MarkState>()
    }
    fn state(&self) -> widget::tree::State {
        let mut state = MarkState(BTreeMap::new());
        self.reconcile(&mut state);
        widget::tree::State::new(state)
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.reconcile(tree.state.downcast_mut::<MarkState>());
    }
    fn layout(&mut self, tree: &mut Tree, _: &iced::Renderer, limits: &layout::Limits) {
        let size = <Self as Widget<Message, iced::Theme, iced::Renderer>>::size(self);
        tree.size = limits.resolve(size.width, size.height, Size::ZERO);
    }
    fn frame_interest(&self, tree: &Tree) -> bool {
        tree.state
            .downcast_ref::<MarkState>()
            .0
            .values()
            .any(PanelMotion::sampled_needs_layout)
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _: Layout,
        _: mouse::Cursor,
        _: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        if let Event::Window(iced::window::Event::RedrawRequested(at)) = event {
            for motion in tree.state.downcast_mut::<MarkState>().0.values_mut() {
                motion.note_frame(*at);
                if motion.sampled_needs_layout() {
                    shell.request_redraw();
                }
                motion.note_layout_at_frame();
            }
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        layout: Layout,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<MarkState>();
        let mut x = layout.bounds().x;
        for (id, active) in &self.entries {
            let width = 4.0 + 12.0 * state.0[id].sampled_progress();
            let hollow = !*active && self.spare.contains(id);
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x,
                        y: layout.bounds().center_y() - 2.0,
                        width,
                        height: 4.0,
                    },
                    border: iced::Border {
                        radius: 2.0.into(),
                        width: if hollow { 1.0 } else { 0.0 },
                        color: self.color,
                    },
                    ..Default::default()
                },
                if hollow {
                    iced::Color::TRANSPARENT
                } else {
                    self.color
                },
            );
            x += width + 8.0;
        }
    }
}
