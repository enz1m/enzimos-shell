use super::{
    grid::Grid,
    model::{DetailPhase, Drag, Edit, State, edit_bounds, resize_handle, resize_hit},
};
use crate::{
    control_layout::{Placement, Shape, closest_slot},
    settings::QuickAction,
};
use iced::advanced::{
    Layout, Shell, Widget, layout, overlay, renderer,
    widget::{Operation, Tree, tree},
};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, keyboard, mouse, window};
use std::{sync::Arc, time::Instant};

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Grid<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            phase: DetailPhase::Stable,
            phase_armed: false,
            keys: self.keys.clone(),
            detail: None,
            detail_open: false,
            section_from: Rectangle::default(),
            section_target: Rectangle::default(),
            pending_section: None,
            authority: std::sync::Arc::clone(&self.model),
            from: Vec::new(),
            target: Vec::new(),
            at: Instant::now(),
            started: None,
            duration: self.duration,
            easing: self.easing,
            drag: None,
            pending_from: None,
            settling: None,
            swallow: false,
            needs_frame: true,
        })
    }
    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.authority != self.model
            || state.keys != self.keys
            || (!self.editable && state.drag.is_some())
        {
            state.cancel();
            state.needs_frame = true;
        }
        let mut old = std::mem::take(&mut tree.children);
        let old_detail = if state.detail.is_some() {
            old.pop()
        } else {
            None
        };
        let mut old: Vec<_> = state.keys.iter().copied().zip(old).collect();
        tree.children = self
            .keys
            .iter()
            .zip(&mut self.children)
            .map(|(key, child)| {
                let mut tree = if let Some(index) = old.iter().position(|(id, _)| id == key) {
                    old.swap_remove(index).1
                } else {
                    Tree::new(child.as_widget())
                };
                tree.diff(child.as_widget_mut());
                tree
            })
            .collect();
        if let Some(detail) = &mut self.detail {
            let mut child = if state.detail == Some(detail.action) {
                old_detail.unwrap_or_else(|| Tree::new(detail.content.as_widget()))
            } else {
                Tree::new(detail.content.as_widget())
            };
            child.diff(detail.content.as_widget_mut());
            tree.children.push(child);
        }
        let next_detail = self.detail.as_ref().map(|detail| detail.action);
        let next_open = self.detail.as_ref().is_some_and(|detail| detail.open);
        if state.detail != next_detail || state.detail_open != next_open {
            state.cancel();
            state.change_detail(next_detail, next_open);
            state.needs_frame = true;
        }
        state.detail = next_detail;
        state.detail_open = next_open;
        state.keys.clone_from(&self.keys);
        state.authority = std::sync::Arc::clone(&self.model);
        tree.refresh_frame_children();
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn frame_interest(&self, tree: &Tree) -> bool {
        tree.state.downcast_ref::<State>().transitioning()
            || tree.state.downcast_ref::<State>().needs_frame
            || tree.children.iter().any(Tree::has_frame_interest)
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        let width = limits.max().width;
        let state = tree.state.downcast_mut::<State>();
        state.advance_phase(self.duration);
        let model = state
            .drag
            .as_ref()
            .map_or_else(|| self.model.as_ref(), |drag| &drag.preview);
        let resizing = state
            .drag
            .as_ref()
            .filter(|drag| drag.started && drag.resize)
            .map(|drag| {
                (
                    drag.action,
                    drag.initial.width + drag.point.x - drag.start.x,
                )
            });
        let targets = self.placements_resizing(model, width, resizing, state.phase);
        let detail_height = self
            .phase_detail(state.phase)
            .map_or(0.0, |(_, height)| height);
        let section = self
            .detail
            .as_ref()
            .and_then(|detail| {
                targets
                    .iter()
                    .find(|p| p.action == detail.action)
                    .map(|p| Rectangle {
                        x: 0.0,
                        y: p.bounds.y + self.height,
                        width,
                        height: detail_height,
                    })
            })
            .unwrap_or_default();
        if section != state.section_target {
            state.pending_section = Some(section);
        }
        let duration = if state.phase == DetailPhase::Stable {
            self.duration
        } else {
            self.duration / 2
        };
        state.retarget(targets, duration, self.easing);
        state.phase_armed = true;
        let section = state.section_pose();
        let height = state
            .target
            .iter()
            .map(|p| {
                let rect = state.slot_pose(p.action);
                rect.y + rect.height
            })
            .fold(0.0, f32::max)
            .max(if section.height > 0.0 {
                section.y + section.height
            } else {
                0.0
            });
        let section = state.visual_section();
        for ((child, tree), key) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(&self.keys)
        {
            let rect = state.pose(*key);
            child.as_widget_mut().layout(
                tree,
                renderer,
                &layout::Limits::new(Size::ZERO, rect.size())
                    .width(rect.width)
                    .height(rect.height),
            );
            tree.translation = Vector::new(rect.x, rect.y);
        }
        if let Some(detail) = &mut self.detail {
            let tree = tree.children.last_mut().expect("Detail tree");
            detail.content.as_widget_mut().layout(
                tree,
                renderer,
                &layout::Limits::new(
                    Size::ZERO,
                    Size::new(section.width, detail.height - crate::control_layout::GAP),
                )
                .width(section.width)
                .height(detail.height - crate::control_layout::GAP),
            );
            tree.translation = Vector::new(section.x, section.y + crate::control_layout::GAP);
        }
        let extent = state
            .drag
            .as_ref()
            .filter(|drag| drag.started)
            .map_or(0.0, |drag| drag.extent);
        tree.size = Size::new(width, height.max(extent));
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds(), viewport);
        if !self.interactive {
            return;
        }
        operation.traverse(&mut |operation| {
            let layouts: Vec<_> = layout.children(&tree.children).collect();
            let mut order: Vec<_> = (0..self.children.len())
                .filter(|&i| {
                    self.editable
                        || !self
                            .detail
                            .as_ref()
                            .is_some_and(|d| d.open && d.action != self.keys[i])
                })
                .collect();
            if self.detail.as_ref().is_some_and(|detail| detail.open)
                && !tree.state.downcast_ref::<State>().transitioning()
            {
                order.push(self.children.len());
            }
            order.sort_by(|&a, &b| {
                layouts[a]
                    .bounds()
                    .y
                    .total_cmp(&layouts[b].bounds().y)
                    .then(layouts[a].bounds().x.total_cmp(&layouts[b].bounds().x))
            });
            for index in order {
                let child = if index < self.children.len() {
                    &mut self.children[index]
                } else {
                    &mut self
                        .detail
                        .as_mut()
                        .expect("detail layouts and child trees are constructed together")
                        .content
                };
                child.as_widget_mut().operate(
                    &mut tree.children[index],
                    layouts[index],
                    viewport,
                    renderer,
                    operation,
                );
            }
        });
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
        state.at = match event {
            Event::Window(window::Event::RedrawRequested(at)) => *at,
            _ => Instant::now(),
        };
        let bounds = layout.bounds();
        // Ancestor scroll and motion transforms already apply to the cursor/layout pair.
        let point = cursor
            .position()
            .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        if state.drag.as_ref().is_some_and(|drag| {
            drag.origin != bounds.position()
                || drag.width != bounds.width
                || drag.initial.height != self.height
        }) {
            state.cancel();
            shell.invalidate_layout();
        }
        let cancel = matches!(
            event,
            Event::Window(window::Event::Unfocused | window::Event::Resized(_))
        ) || matches!(
            event,
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            })
        );
        if cancel && state.drag.is_some() {
            state.cancel();
            shell.capture_event();
            shell.invalidate_layout();
            shell.request_redraw();
            return;
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && state.transitioning()
            && !shell.is_event_captured()
            && cursor.position().is_some_and(|point| {
                edit_bounds(bounds).contains(point) && viewport.contains(point)
            })
        {
            state.swallow = true;
            shell.capture_event();
            return;
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && self.editable
            && !shell.is_event_captured()
            && let Some(point) = point
            && cursor.position().is_some_and(|p| viewport.contains(p))
            && let Some((index, rect)) = self
                .keys
                .iter()
                .enumerate()
                .map(|(i, key)| (i, state.pose(*key)))
                .find(|(_, rect)| resize_hit(*rect, point))
                .or_else(|| {
                    self.keys
                        .iter()
                        .enumerate()
                        .map(|(i, key)| (i, state.pose(*key)))
                        .find(|(_, rect)| rect.contains(point))
                })
        {
            let resize = self.keys[index] != QuickAction::Volume && resize_hit(rect, point);
            let disclosure = self.disclosures[index] && rect.width > rect.height + 24.0;
            if resize || !disclosure || point.x < rect.x + rect.width - 28.0 {
                state.swallow = false;
                state.settling = None;
                let action = self.keys[index];
                let slots = self.model.drop_slots_with_detail(
                    action,
                    bounds.width,
                    self.height,
                    &self.keys,
                    self.detail
                        .as_ref()
                        .filter(|d| d.open)
                        .map(|d| (d.action, d.height)),
                );
                state.drag = Some(Drag {
                    action: self.keys[index],
                    start: point,
                    point,
                    offset: point - rect.position(),
                    origin: bounds.position(),
                    width: bounds.width,
                    initial: rect,
                    resize,
                    started: false,
                    preview: (*self.model).clone(),
                    extent: slots
                        .iter()
                        .map(|slot| slot.bounds.y + slot.bounds.height)
                        .fold(bounds.height, f32::max),
                    slots,
                });
                shell.capture_event();
                return;
            }
        }
        if matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }))
            && let Some(drag) = &mut state.drag
        {
            if let Some(point) = point {
                drag.point = point;
                drag.started |= point.distance(drag.start) >= 6.0;
                if drag.started {
                    if drag.resize {
                        let width = drag.initial.width + point.x - drag.start.x;
                        drag.preview.set_shape(
                            drag.action,
                            Shape::nearest(width, bounds.width, self.height),
                        );
                    } else if let Some(slot) = closest_slot(&drag.slots, point - drag.offset) {
                        drag.preview = (*self.model).clone();
                        drag.preview.move_before(drag.action, slot.before);
                    }
                    shell.invalidate_layout();
                    shell.request_redraw();
                }
            }
            shell.capture_event();
            return;
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) {
            let released: Vec<_> = self
                .keys
                .iter()
                .map(|key| Placement {
                    action: *key,
                    bounds: state.pose(*key),
                })
                .collect();
            if let Some(drag) = state.drag.take() {
                if (drag.resize
                    || point.is_some_and(|p| Rectangle::with_size(bounds.size()).contains(p)))
                    && cursor.position().is_some_and(|p| viewport.contains(p))
                    && self.editable
                    && state.authority == self.model
                {
                    if drag.started {
                        let mut from = released;
                        if !drag.resize
                            && let Some(active) = from.iter_mut().find(|p| p.action == drag.action)
                        {
                            active.bounds.x = drag.point.x - drag.offset.x;
                            active.bounds.y = drag.point.y - drag.offset.y;
                        }
                        state.from = from;
                        state.settling = Some(drag.action);
                        state.target = self.placements(&drag.preview, bounds.width);
                        state.started = Some(state.at);
                        shell.publish((self.edit)(Edit::Commit {
                            expected: std::sync::Arc::clone(&self.model),
                            value: Arc::new(drag.preview),
                        }));
                    } else {
                        shell.publish((self.edit)(Edit::Select(drag.action)));
                    }
                } else {
                    state.drag = Some(drag);
                    state.cancel();
                }
                state.swallow = false;
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
                return;
            }
            if state.swallow {
                state.swallow = false;
                shell.capture_event();
                return;
            }
        }
        if state.drag.is_some() && matches!(event, Event::Mouse(mouse::Event::WheelScrolled { .. }))
        {
            state.cancel();
            shell.invalidate_layout();
            shell.request_redraw();
        }
        if matches!(event, Event::Window(window::Event::RedrawRequested(_)))
            && (state.transitioning() || state.needs_frame)
        {
            state.needs_frame = false;
            shell.invalidate_layout();
            shell.request_redraw();
        }
        let layouts: Vec<_> = layout.children(&tree.children).collect();
        for (index, ((child, state), layout)) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layouts.iter().copied())
            .enumerate()
        {
            let blocked = !self.interactive
                || (!self.editable
                    && self
                        .detail
                        .as_ref()
                        .is_some_and(|d| d.open && d.action != self.keys[index]));
            let event =
                if blocked && !matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
                    &Event::Window(window::Event::Unfocused)
                } else {
                    event
                };
            child.as_widget_mut().update(
                state,
                event,
                layout,
                if blocked {
                    mouse::Cursor::Unavailable
                } else {
                    cursor
                },
                renderer,
                shell,
                viewport,
            );
            state.refresh_frame_interest(child.as_widget());
        }
        if let Some(detail) = &mut self.detail {
            let state = tree.state.downcast_ref::<State>();
            let section = state.section_pose() + Vector::new(bounds.x, bounds.y);
            if self.interactive
                && detail.open
                && !state.transitioning()
                && state.drag.is_none()
                && let Some(clip) = section.intersection(viewport)
            {
                let index = self.children.len();
                let layout = layouts[index];
                let child = &mut tree.children[index];
                detail
                    .content
                    .as_widget_mut()
                    .update(child, event, layout, cursor, renderer, shell, &clip);
                child.refresh_frame_interest(detail.content.as_widget());
            }
        }
        tree.refresh_frame_children();
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
        use iced::advanced::Renderer;
        let state = tree.state.downcast_ref::<State>();
        let paint_bounds = if self.editable {
            edit_bounds(layout.bounds())
        } else {
            layout.bounds()
        };
        let Some(clip) = paint_bounds.intersection(viewport) else {
            return;
        };
        let front = state.front();
        renderer.with_layer(clip, |renderer| {
            let children: Vec<_> = layout.children(&tree.children).collect();
            let section =
                state.visual_section() + Vector::new(layout.bounds().x, layout.bounds().y);
            let draw_detail = |renderer: &mut iced::Renderer, index: usize| {
                if let Some(detail) = &self.detail
                    && detail.action == self.keys[index]
                    && section.height > 0.0
                    && let Some(section_clip) = section.intersection(&clip)
                {
                    let gap = crate::control_layout::GAP.min(section.height);
                    let card = Rectangle {
                        y: section.y + gap,
                        height: (section.height - gap).max(0.0),
                        ..section
                    };
                    let radius = 16.0_f32.min(card.height / 2.0);
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: card,
                            border: iced::Border {
                                radius: radius.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        self.surface,
                    );
                    renderer.with_layer_rounded(card, radius.into(), |renderer| {
                        renderer.with_layer(section_clip, |renderer| {
                            detail.content.as_widget().draw(
                                &tree.children[self.children.len()],
                                renderer,
                                theme,
                                style,
                                children[self.children.len()],
                                if state.transitioning() || state.drag.is_some() {
                                    mouse::Cursor::Unavailable
                                } else {
                                    cursor
                                },
                                &section_clip,
                            );
                        });
                    });
                }
            };
            let order = self
                .keys
                .iter()
                .enumerate()
                .filter(|(_, key)| Some(**key) != front)
                .map(|(i, _)| i)
                .chain(self.keys.iter().position(|key| Some(*key) == front));
            for index in order {
                // Separate layers prevent deferred text/SVG batches from painting above the
                // next card.
                renderer.with_layer_alpha(clip, 1.0, |renderer| {
                    draw_detail(renderer, index);
                    self.children[index].as_widget().draw(
                        &tree.children[index],
                        renderer,
                        theme,
                        style,
                        children[index],
                        if front.is_some() || state.transitioning() {
                            mouse::Cursor::Unavailable
                        } else {
                            cursor
                        },
                        &clip,
                    );
                    if self.editable {
                        renderer.with_layer(clip, |renderer| {
                            let rect = children[index].bounds();
                            if self.selected == Some(self.keys[index])
                                || front == Some(self.keys[index])
                            {
                                renderer.fill_quad(
                                    renderer::Quad {
                                        bounds: Rectangle {
                                            x: rect.x + 1.0,
                                            y: rect.y + 1.0,
                                            width: rect.width - 2.0,
                                            height: rect.height - 2.0,
                                        },
                                        border: iced::Border {
                                            color: self.accent,
                                            width: 2.0,
                                            radius: (rect.height / 2.0).into(),
                                        },
                                        ..Default::default()
                                    },
                                    iced::Color::TRANSPARENT,
                                );
                            }
                            if self.keys[index] != QuickAction::Volume
                                && front.map_or_else(
                                    || {
                                        self.selected == Some(self.keys[index])
                                            || cursor.position().is_some_and(|point| {
                                                rect.contains(point) || resize_hit(rect, point)
                                            })
                                    },
                                    |active| active == self.keys[index],
                                )
                            {
                                renderer.fill_quad(
                                    renderer::Quad {
                                        bounds: resize_handle(rect),
                                        border: iced::Border {
                                            radius: 7.0.into(),
                                            width: 1.5,
                                            color: self.accent,
                                        },
                                        ..Default::default()
                                    },
                                    self.handle,
                                );
                            }
                        });
                    }
                });
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
        if !self.interactive {
            return mouse::Interaction::default();
        }
        if let Some(drag) = &tree.state.downcast_ref::<State>().drag {
            return if drag.resize {
                mouse::Interaction::ResizingHorizontally
            } else {
                mouse::Interaction::Grabbing
            };
        }
        if self.editable
            && let Some(point) = cursor.position()
            && viewport.contains(point)
            && self
                .keys
                .iter()
                .zip(layout.children(&tree.children))
                .any(|(key, child)| {
                    *key != QuickAction::Volume && resize_hit(child.bounds(), point)
                })
        {
            return mouse::Interaction::ResizingHorizontally;
        }
        let details = self
            .detail
            .as_ref()
            .filter(|detail| detail.open)
            .map(|detail| {
                detail.content.as_widget().mouse_interaction(
                    tree.children
                        .last()
                        .expect("detail layouts and child trees are constructed together"),
                    layout
                        .children(&tree.children)
                        .next_back()
                        .expect("detail layouts and child trees are constructed together"),
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .unwrap_or_default();
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children(&tree.children))
            .map(|((child, state), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
            .max(details)
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
        let state = tree.state.downcast_ref::<State>();
        if state.drag.is_some() || state.transitioning() {
            return Vec::new();
        }
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
impl<'a, Message: 'a> From<Grid<'a, Message>> for Element<'a, Message> {
    fn from(grid: Grid<'a, Message>) -> Self {
        Element::new(grid)
    }
}
