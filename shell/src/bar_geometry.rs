use iced::advanced::{
    Shell, Widget,
    layout::{self, Layout},
    renderer,
    widget::{Id, Operation, Tree, operation::Scrollable},
};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub struct IslandRect {
    pub key: String,
    pub bounds: Rectangle,
    pub clip: Rectangle,
}
#[derive(Clone, Debug, PartialEq)]
pub struct BarGeometry {
    pub size: Size,
    pub islands: Vec<IslandRect>,
}
impl BarGeometry {
    pub fn anchor(&self, key: &str) -> Option<Rectangle> {
        self.islands
            .iter()
            .find(|island| island.key == key)
            .and_then(|island| island.bounds.intersection(&island.clip))
    }
    pub fn valid(&self) -> bool {
        fn finite(rect: Rectangle) -> bool {
            [rect.x, rect.y, rect.width, rect.height]
                .into_iter()
                .all(f32::is_finite)
                && rect.width > 0.0
                && rect.height > 0.0
        }
        let mut keys = BTreeSet::new();
        self.size.width.is_finite()
            && self.size.height.is_finite()
            && (1.0..=16384.0).contains(&self.size.width)
            && (1.0..=128.0).contains(&self.size.height)
            && self.islands.len() <= 64
            && self.islands.iter().all(|island| {
                finite(island.bounds)
                    && finite(island.clip)
                    && island.clip.x >= 0.0
                    && island.clip.y >= 0.0
                    && island.clip.x + island.clip.width <= self.size.width + 0.01
                    && island.clip.y + island.clip.height <= self.size.height + 0.01
                    && !island.key.is_empty()
                    && island.key.len() <= 104
                    && !island.key.chars().any(char::is_control)
                    && keys.insert(&island.key)
            })
    }
}
struct Collect {
    ids: Vec<(String, Id)>,
    seen: BTreeSet<String>,
    islands: Vec<IslandRect>,
    ambiguous: bool,
    offset: Vector,
    clip: Rectangle,
    pending: Option<(Rectangle, Vector)>,
}
impl Collect {
    fn new(keys: &[String], bounds: Rectangle) -> Self {
        Self {
            ids: keys
                .iter()
                .map(|key| (key.clone(), Id::from(format!("bar-island-{key}"))))
                .collect(),
            seen: BTreeSet::new(),
            islands: Vec::new(),
            ambiguous: false,
            offset: Vector::ZERO,
            clip: bounds,
            pending: None,
        }
    }
}
impl Operation for Collect {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        if let Some((key, _)) = self.ids.iter().find(|(_, expected)| Some(expected) == id) {
            if !self.seen.insert(key.clone()) {
                self.ambiguous = true;
                return;
            }
            let bounds = bounds - self.offset;
            if let Some(clip) = bounds.intersection(&self.clip) {
                self.islands.push(IslandRect {
                    key: key.clone(),
                    bounds,
                    clip,
                });
            }
        }
    }
    fn scrollable(
        &mut self,
        _: Option<&Id>,
        bounds: Rectangle,
        _: Size,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        self.pending = Some((bounds, translation));
    }
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let previous = (self.offset, self.clip);
        if let Some((bounds, translation)) = self.pending.take() {
            self.clip = self
                .clip
                .intersection(&(bounds - self.offset))
                .unwrap_or_default();
            self.offset += translation;
        }
        operate(self);
        (self.offset, self.clip) = previous;
    }
}

pub fn observe<'a, Message: Clone + 'a>(
    child: Element<'a, Message>,
    keys: Vec<String>,
    epoch: u64,
    on_change: impl Fn(u64, u64, BarGeometry) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(Observer {
        child,
        keys,
        epoch,
        on_change: Box::new(on_change),
    })
}
struct Observer<'a, Message> {
    child: Element<'a, Message>,
    keys: Vec<String>,
    epoch: u64,
    on_change: Box<dyn Fn(u64, u64, BarGeometry) -> Message + 'a>,
}
#[derive(Default)]
struct State {
    last: Option<(u64, BarGeometry)>,
    serial: u64,
}
impl<Message: Clone> Widget<Message, iced::Theme, iced::Renderer> for Observer<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
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
        if matches!(
            event,
            Event::Window(iced::window::Event::RedrawRequested(_))
        ) && self.keys.len() <= 64
        {
            // Island bounds are surface-local, and the bar may sit inset in its
            // surface: clip to where it actually is, and report its extent from
            // the surface origin. Clipping to its size at the origin cut the
            // last island (glass and input) short by the inset.
            let bounds = layout.bounds();
            let mut collect = Collect::new(&self.keys, bounds);
            self.child.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                viewport,
                renderer,
                &mut collect,
            );
            let geometry = BarGeometry {
                size: Size::new(bounds.x + bounds.width, bounds.y + bounds.height),
                islands: collect.islands,
            };
            let state = tree.state.downcast_mut::<State>();
            if !collect.ambiguous
                && geometry.valid()
                && state.last.as_ref() != Some(&(self.epoch, geometry.clone()))
            {
                state.serial = state.serial.wrapping_add(1).max(1);
                state.last = Some((self.epoch, geometry.clone()));
                shell.publish((self.on_change)(self.epoch, state.serial, geometry));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    struct Passive;
    impl Scrollable for Passive {
        fn snap_to(
            &mut self,
            _: iced::advanced::widget::operation::scrollable::RelativeOffset<Option<f32>>,
            _: iced::advanced::widget::operation::Animation,
            _: Rectangle,
            _: Size,
        ) {
            panic!("observer cannot scroll");
        }
        fn scroll_to(
            &mut self,
            _: iced::advanced::widget::operation::scrollable::AbsoluteOffset<Option<f32>>,
            _: iced::advanced::widget::operation::Animation,
            _: Rectangle,
            _: Size,
        ) {
            panic!("observer cannot scroll");
        }
        fn scroll_by(
            &mut self,
            _: iced::advanced::widget::operation::scrollable::AbsoluteOffset,
            _: iced::advanced::widget::operation::Animation,
            _: Rectangle,
            _: Size,
        ) {
            panic!("observer cannot scroll");
        }
    }
    #[test]
    fn bar_geometry_behaviour() {
        // Observer keeps frame events before reconciliation and with static children.
        {
            let child = iced::widget::Space::new().width(400).height(32).into();
            let mut element = observe(child, vec![], 1, |_, _, _| ());
            let mut tree = Tree::new(&element);
            assert!(tree.has_frame_interest());
            tree.diff(&mut element);
            assert_eq!(tree.children.len(), 1);
            assert!(!tree.children[0].has_frame_interest());
            assert!(tree.has_frame_interest());
        }
        // Island measurement preserves unclipped shape and separates visible clip.
        {
            let key = "builtin.controls".to_owned();
            let id = Id::from(format!("bar-island-{key}"));
            let mut op = Collect::new(
                std::slice::from_ref(&key),
                Rectangle::with_size(Size::new(400.0, 32.0)),
            );
            let viewport = Rectangle {
                x: 280.0,
                y: 0.0,
                width: 100.0,
                height: 32.0,
            };
            op.scrollable(
                None,
                viewport,
                viewport.size(),
                Vector::new(50.0, 0.0),
                &mut Passive,
            );
            op.traverse(&mut |op| {
                op.container(
                    Some(&id),
                    Rectangle {
                        x: 300.0,
                        y: 0.0,
                        width: 150.0,
                        height: 32.0,
                    },
                    &Rectangle::INFINITE,
                );
            });
            assert_eq!(op.islands[0].bounds.x, 250.0);
            assert_eq!(op.islands[0].clip.x, 280.0);
            assert_eq!(op.islands[0].clip.width, 100.0);
            let geometry = BarGeometry {
                size: Size::new(400.0, 32.0),
                islands: op.islands.clone(),
            };
            assert!(geometry.valid());
            assert_eq!(geometry.anchor(&key), Some(viewport));
            op.container(Some(&id), viewport, &Rectangle::INFINITE);
            assert!(op.ambiguous);
        }
        // An inset bar clips its last island where the bar ends.
        {
            // A bar 12 px into its surface: the last island ends at the bar's
            // right edge, 12 px past its width from the surface origin.
            let key = "builtin.controls".to_owned();
            let id = Id::from(format!("bar-island-{key}"));
            let bar = Rectangle {
                x: 12.0,
                y: 0.0,
                width: 1576.0,
                height: 32.0,
            };
            let mut op = Collect::new(std::slice::from_ref(&key), bar);
            let island = Rectangle {
                x: 1484.0,
                y: 0.0,
                width: 104.0,
                height: 32.0,
            };
            op.container(Some(&id), island, &Rectangle::INFINITE);
            assert_eq!(op.islands[0].clip, island);
            let geometry = BarGeometry {
                size: Size::new(bar.x + bar.width, bar.height),
                islands: op.islands,
            };
            assert!(geometry.valid());
        }
        // Offscreen or nonfinite masks cannot claim a bar.
        {
            let mut geometry = BarGeometry {
                size: Size::new(400.0, 32.0),
                islands: vec![],
            };
            assert!(geometry.valid());
            geometry.size.width = f32::NAN;
            assert!(!geometry.valid());
            geometry.size.width = 400.0;
            geometry.islands.push(IslandRect {
                key: "x".into(),
                bounds: Rectangle::with_size(Size::new(10.0, 32.0)),
                clip: Rectangle {
                    x: -1.0,
                    y: 0.0,
                    width: 10.0,
                    height: 32.0,
                },
            });
            assert!(!geometry.valid());
        }
    }
}
