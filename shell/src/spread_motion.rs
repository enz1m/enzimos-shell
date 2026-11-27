use super::{
    CAPTION_GAP, Message, Phase, RING, Rect, Spread, SpreadMessage, framed_picture, picture_button,
    tile_caption, tile_label,
};
use enzui_shell::presentation::{Projected, inverse_event};
use iced::advanced::Renderer as _;
use iced::advanced::{
    Layout, Shell, Widget, layout, renderer,
    widget::{Operation, Tree},
};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Transformation, mouse, window};
use std::time::Instant;

pub(super) fn backdrop(spread: &Spread) -> Element<'_, Message> {
    Element::new(Backdrop(spread))
}

struct Backdrop<'a>(&'a Spread);

impl Widget<Message, iced::Theme, iced::Renderer> for Backdrop<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, _: &iced::Renderer, limits: &layout::Limits) {
        tree.size = limits.max();
    }

    fn frame_interest(&self, _: &Tree) -> bool {
        false
    }

    fn draw(
        &self,
        _: &Tree,
        renderer: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        layout: Layout,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let p = self.0.progress(self.0.sampled_at(Instant::now()));
        if p > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    ..Default::default()
                },
                Color {
                    a: 0.32 * p,
                    ..Color::BLACK
                },
            );
        }
    }
}

pub(super) fn tiles<'a>(
    spread: &'a Spread,
    window: window::Id,
    palette: &enzui_theme::Palette,
    strip: Option<(Rect, Element<'a, Message>)>,
    strip_offset: f32,
) -> Element<'a, Message> {
    let reflow = spread.is_reflowing();
    let chosen = spread.chosen.filter(|_| spread.phase == Phase::Closing);
    let order = (0..spread.entries.len())
        .filter(|i| Some(*i) != chosen)
        .chain(chosen);
    let overview = spread.strip.is_some();
    let close_tile = if reflow { spread.current_tile() } else { None };
    let mut parts = Vec::new();
    let mut children = Vec::new();
    if let Some((rect, child)) = strip {
        parts.push(Part {
            index: 0,
            kind: PartKind::Strip {
                rect,
                offset: strip_offset,
            },
            interactive: reflow,
        });
        children.push(child);
    }
    for index in order {
        let entry = &spread.entries[index];
        let gone = spread.entry_gone(entry);
        if gone && !reflow {
            continue;
        }
        let id = entry.window.stable_id;
        let base = if reflow {
            spread
                .tile_motion(index, 0.0)
                .map_or(entry.to, |(r, _, _)| r)
        } else {
            entry.from
        };
        let selected = reflow && (spread.selected == Some(id) || spread.hovered == Some(id));
        let picture = framed_picture(entry, base, spread.rounding, selected, palette);
        let label = if reflow && !gone {
            tile_label(entry)
        } else {
            String::new()
        };
        parts.push(Part {
            index,
            kind: PartKind::Picture { width: base.width },
            interactive: !gone && spread.swipe.get().is_none() && spread.settle.is_none(),
        });
        children.push(if gone {
            picture
        } else {
            picture_button(picture, entry, &label, reflow && overview)
        });
        if reflow && !gone {
            let width = if overview {
                base.width + 2.0 * RING
            } else {
                base.width.max(120.0)
            };
            parts.push(Part {
                index,
                kind: PartKind::Caption,
                interactive: true,
            });
            children.push(
                iced::widget::container(tile_caption(
                    entry,
                    label,
                    selected,
                    overview,
                    selected || close_tile == Some(id),
                    palette,
                ))
                .center_x(width)
                .into(),
            );
        }
    }
    Element::new(Tiles {
        spread,
        window,
        parts,
        children,
    })
}

#[derive(Clone, Copy)]
enum PartKind {
    Picture { width: f32 },
    Caption,
    Strip { rect: Rect, offset: f32 },
}

struct Part {
    index: usize,
    kind: PartKind,
    interactive: bool,
}

struct Tiles<'a> {
    spread: &'a Spread,
    window: window::Id,
    parts: Vec<Part>,
    children: Vec<Element<'a, Message>>,
}

#[derive(Clone, Copy)]
struct MotionSample {
    travel: f32,
    reflow: Option<f32>,
}

impl Tiles<'_> {
    fn sample(&self) -> MotionSample {
        let at = self.spread.sampled_at(Instant::now());
        MotionSample {
            travel: self.spread.progress(at),
            reflow: self.spread.reflow_progress(at),
        }
    }

    fn pose(
        &self,
        part: &Part,
        size: Size,
        origin: Point,
        sample: MotionSample,
    ) -> Option<(Transformation, f32)> {
        if let PartKind::Strip { rect, offset } = part.kind {
            return Some((
                Transformation::translate(
                    origin.x + rect.x,
                    origin.y + rect.y + offset * (1.0 - sample.travel),
                ),
                sample.travel,
            ));
        }
        let entry = &self.spread.entries[part.index];
        let (r, caption, alpha) = if let Some(t) = sample.reflow {
            self.spread.tile_motion(part.index, t)?
        } else {
            (entry.from.lerp(entry.to, sample.travel), 0.0, 1.0)
        };
        if r.width <= 0.0 || alpha <= 0.0 {
            return None;
        }
        let (x, y, scale) = match part.kind {
            PartKind::Picture { width } => {
                let scale = r.width / width.max(1.0);
                (r.x - RING * scale, r.y - RING * scale, scale)
            }
            PartKind::Strip { .. } => unreachable!(),
            PartKind::Caption => (
                r.x + (r.width - size.width) / 2.0,
                caption + CAPTION_GAP,
                1.0,
            ),
        };
        Some((
            Transformation::translate(origin.x + x, origin.y + y) * Transformation::scale(scale),
            alpha,
        ))
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for Tiles<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.children);
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        tree.size = limits.max();
        // A source window can be larger than the spread's viewport.
        let child_limits = layout::Limits::new(Size::ZERO, Size::INFINITE);
        for (child, state) in self.children.iter_mut().zip(&mut tree.children) {
            child.as_widget_mut().layout(state, renderer, &child_limits);
        }
    }

    fn frame_interest(&self, _: &Tree) -> bool {
        self.spread.animating() || self.spread.tracking_swipe()
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let sample = self.sample();
        operation.container(None, layout.bounds(), viewport);
        operation.traverse(&mut |operation| {
            for index in 0..self.parts.len() {
                let part = &self.parts[index];
                let state = &mut tree.children[index];
                if part.interactive
                    && let Some((transform, _)) =
                        self.pose(part, state.size, layout.position(), sample)
                {
                    let child_layout = Layout::new(state.size);
                    self.children[index].as_widget_mut().operate(
                        state,
                        child_layout,
                        &(*viewport * transform.inverse()),
                        renderer,
                        &mut Projected {
                            operation,
                            transform,
                        },
                    );
                }
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        mut cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(at)) = event {
            if self.spread.tracking_swipe() {
                shell.request_redraw();
            } else if self.spread.animating() {
                if self.spread.sample_frame(*at) {
                    shell.publish(Message::Spread(SpreadMessage::MotionFinished(
                        self.window,
                        *at,
                    )));
                } else {
                    shell.request_redraw();
                }
            }
            return;
        }

        let sample = self.sample();
        for index in (0..self.parts.len()).rev() {
            let part = &self.parts[index];
            let state = &mut tree.children[index];
            if !part.interactive {
                continue;
            }
            let Some((transform, _)) = self.pose(part, state.size, layout.position(), sample)
            else {
                continue;
            };
            let inverse = transform.inverse();
            let child = self.children[index].as_widget_mut();
            let layout = Layout::new(state.size);
            let viewport = *viewport * inverse;
            child.update(
                state,
                &inverse_event(event, transform),
                layout,
                cursor * inverse,
                renderer,
                shell,
                &viewport,
            );
            if shell.is_event_captured() {
                break;
            }
            if child.mouse_interaction(state, layout, cursor * inverse, &viewport, renderer)
                != mouse::Interaction::None
            {
                cursor = cursor.levitate();
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
        let sample = self.sample();
        for index in (0..self.parts.len()).rev() {
            let part = &self.parts[index];
            let state = &tree.children[index];
            if part.interactive
                && let Some((transform, _)) = self.pose(part, state.size, layout.position(), sample)
            {
                let inverse = transform.inverse();
                let interaction = self.children[index].as_widget().mouse_interaction(
                    state,
                    Layout::new(state.size),
                    cursor * inverse,
                    &(*viewport * inverse),
                    renderer,
                );
                if interaction != mouse::Interaction::None {
                    return interaction;
                }
            }
        }
        mouse::Interaction::None
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
        let sample = self.sample();
        for index in 0..self.parts.len() {
            let part = &self.parts[index];
            let state = &tree.children[index];
            let Some((transform, alpha)) = self.pose(part, state.size, layout.position(), sample)
            else {
                continue;
            };
            if alpha <= 0.0 {
                continue;
            }
            let inverse = transform.inverse();
            let draw = |renderer: &mut iced::Renderer| {
                renderer.with_transformation(transform, |renderer| {
                    self.children[index].as_widget().draw(
                        state,
                        renderer,
                        theme,
                        style,
                        Layout::new(state.size),
                        cursor * inverse,
                        &(*viewport * inverse),
                    );
                });
            };
            if alpha < 1.0 {
                let bounds = Layout::new(state.size).bounds() * transform;
                if let Some(clip) = bounds.intersection(viewport) {
                    renderer.with_layer_alpha(clip, alpha, draw);
                }
            } else {
                draw(renderer);
            }
        }
    }
}
