use crate::Message;
use enzui_theme::{MotionScale, MotionToken};
use iced::advanced::{
    Renderer as _, Shell, Widget,
    layout::{self, Layout},
    renderer,
    widget::{self, Operation, Tree},
};
use iced::animation::Animation;
use iced::time::{Duration, Instant};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

pub(super) struct Rail<'a> {
    pub children: [Element<'a, Message>; 2],
    pub count: usize,
    pub slot: f32,
    pub scroll: f32,
    pub target_height: f32,
    pub show_marker: bool,
    pub interactive: bool,
    pub color: iced::Color,
    pub policy: MotionScale,
    pub input: Option<enzui_shell::surface_input::SurfaceInput>,
    pub glass: Option<enzui_theme::MaterialTokens>,
}
struct State {
    height: Animation<f32>,
    marker: Animation<f32>,
    frame: Option<Instant>,
    policy: MotionScale,
    count: usize,
    pending: bool,
    reset_marker: bool,
    center_single: bool,
}
impl State {
    fn sample(animation: &Animation<f32>, frame: Option<Instant>) -> f32 {
        frame.map_or_else(|| animation.value(), |at| animation.interpolate(at))
    }
    fn active(&self) -> bool {
        self.pending
            || self
                .frame
                .is_some_and(|at| self.height.is_animating(at) || self.marker.is_animating(at))
    }
    fn note_frame(&mut self, at: Instant, rail: &Rail<'_>) {
        if self.pending {
            let duration =
                Duration::from_secs_f32(rail.policy.secs(MotionToken::Normal).clamp(0.0, 2.0));
            let easing = enzui_widgets::motion::iced_easing(rail.policy.easing);
            let height = Self::sample(&self.height, self.frame);
            let marker = if self.reset_marker {
                f32::from(rail.show_marker)
            } else {
                Self::sample(&self.marker, self.frame)
            };
            if duration.is_zero() {
                self.height = Animation::new(rail.target_height);
                self.marker = Animation::new(f32::from(rail.show_marker));
            } else {
                self.height = Animation::new(height)
                    .duration(duration)
                    .easing(easing)
                    .go(rail.target_height, at);
                self.marker = Animation::new(marker)
                    .duration(duration)
                    .easing(easing)
                    .go(f32::from(rail.show_marker), at);
            }
            self.pending = false;
            self.reset_marker = false;
            self.policy = rail.policy;
            if self.count != rail.count {
                self.center_single = self.count == 0 && rail.count == 1;
            }
            self.count = rail.count;
        }
        self.frame = Some(at);
    }
}
impl Rail<'_> {
    fn body(&self, state: &State, size: Size) -> Rectangle {
        let height = State::sample(&state.height, state.frame)
            .round()
            .clamp(1.0, size.height.max(1.0));
        Rectangle {
            x: 0.0,
            y: ((size.height - height) / 2.0).round(),
            width: size.width,
            height,
        }
    }
    fn frame(&self, state: &State, size: Size) -> enzui_shell::reveal::Frame {
        let clip = self.body(state, size);
        enzui_shell::reveal::Frame {
            extended: true,
            translation: Vector::ZERO,
            fills: vec![(clip, (clip.width.min(clip.height) / 2.0).into())],
            corners: Vec::new(),
            clip,
        }
    }
}
impl Widget<Message, iced::Theme, iced::Renderer> for Rail<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State {
            height: Animation::new(self.target_height),
            marker: Animation::new(f32::from(self.show_marker)),
            frame: None,
            policy: self.policy,
            count: self.count,
            pending: false,
            reset_marker: false,
            center_single: self.count == 1,
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.height.value() != self.target_height
            || state.marker.value() != f32::from(self.show_marker)
            || state.policy != self.policy
            || state.count != self.count
        {
            state.pending = true;
            state.reset_marker |= state.count != self.count;
        }
        tree.diff_children(&mut self.children);
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        let size = limits.max();
        let state = tree.state.downcast_ref::<State>();
        let body = self.body(state, size);
        let marker = State::sample(&state.marker, state.frame).clamp(0.0, 1.0);
        let natural = self.count as f32 * (self.slot + 6.0) - 6.0;
        let overflow = natural + self.slot + 22.0 > size.height;
        let footer = if overflow && self.count > 0 {
            (self.slot + 6.0) * marker
        } else {
            0.0
        };
        let content_height = (body.height - 16.0 - footer).max(0.0);
        let top = if self.count == 1 && state.center_single && !self.show_marker && marker == 0.0 {
            (size.height - self.slot) / 2.0
        } else {
            body.y + 8.0
        };
        self.children[0].as_widget_mut().layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(
                Size::ZERO,
                Size::new((size.width - 4.0).max(0.0), content_height),
            ),
        );
        self.children[1].as_widget_mut().layout(
            &mut tree.children[1],
            renderer,
            &layout::Limits::new(Size::ZERO, Size::new(size.width, self.slot)),
        );
        let glyph = tree.children[1].size;
        let marker_y = if self.count == 0 {
            (size.height - glyph.height) / 2.0
        } else if overflow {
            body.y + body.height - 8.0 - self.slot * marker
        } else {
            top + self.count as f32 * (self.slot + 6.0) - self.scroll.max(0.0)
        };
        let marker_x = (size.width - glyph.width) / 2.0;
        tree.size = size;
        tree.children[0].translation = Vector::new(2.0, top);
        tree.children[1].translation = Vector::new(marker_x, marker_y);
    }
    fn frame_interest(&self, tree: &Tree) -> bool {
        tree.state.downcast_ref::<State>().active()
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
        let state = tree.state.downcast_mut::<State>();
        if let Event::Window(iced::window::Event::RedrawRequested(at)) = event {
            let moving = state.active();
            state.note_frame(*at, self);
            if moving {
                shell.invalidate_layout();
            }
            if state.active() {
                shell.request_redraw();
            }
        }
        if !self.interactive {
            return;
        }
        let bounds = layout.bounds();
        let body = self.body(state, bounds.size()) + Vector::new(bounds.x, bounds.y);
        if let Some(clip) = body.intersection(viewport) {
            let layout = layout.child(&tree.children, 0);
            self.children[0].as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                if cursor.is_over(clip) {
                    cursor
                } else {
                    cursor.levitate()
                },
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
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let frame = self.frame(state, bounds.size());
        let mut glazed = false;
        if let Some(input) = &self.input {
            let _commit = input.commit_together();
            if self.interactive {
                input.shape(&frame, bounds.size());
            } else {
                input.empty();
            }
            // No shadow: the scene clips the rail to its own box.
            glazed = match &self.glass {
                Some(tokens) => input.glass(
                    &[(frame.clip, frame.clip.width.min(frame.clip.height) / 2.0)],
                    tokens,
                    self.color,
                    None,
                ),
                None => {
                    input.clear_glass();
                    false
                }
            };
        }
        let Some(outer) = bounds.intersection(viewport) else {
            return;
        };
        let offset = Vector::new(bounds.x, bounds.y);
        if !glazed {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: frame.clip + offset,
                    border: iced::Border {
                        radius: (frame.clip.width / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                self.color,
            );
        }
        let marker = State::sample(&state.marker, state.frame).clamp(0.0, 1.0);
        let layouts: Vec<_> = layout.children(&tree.children).collect();
        for region in enzui_shell::surface_input::content_regions(&frame, bounds.size()) {
            if let Some(clip) = (region + offset).intersection(&outer) {
                renderer.with_layer(clip, |renderer| {
                    self.children[0].as_widget().draw(
                        &tree.children[0],
                        renderer,
                        theme,
                        style,
                        layouts[0],
                        if self.interactive {
                            cursor
                        } else {
                            cursor.levitate()
                        },
                        &clip,
                    );
                    if marker > 0.0 || self.count == 0 {
                        renderer.with_layer_alpha(
                            layouts[1].bounds(),
                            if self.count == 0 { 1.0 } else { marker },
                            |renderer| {
                                self.children[1].as_widget().draw(
                                    &tree.children[1],
                                    renderer,
                                    theme,
                                    style,
                                    layouts[1],
                                    cursor.levitate(),
                                    &clip,
                                );
                            },
                        );
                    }
                });
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
            let layout = layout.child(&tree.children, 0);
            self.children[0].as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                viewport,
                renderer,
                operation,
            );
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
        let body = self.body(tree.state.downcast_ref::<State>(), bounds.size())
            + Vector::new(bounds.x, bounds.y);
        if self.interactive && cursor.is_over(body) {
            self.children[0].as_widget().mouse_interaction(
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
        if !self.interactive || tree.state.downcast_ref::<State>().active() {
            return Vec::new();
        }
        let layout = layout.child(&tree.children, 0);
        self.children[0].as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
