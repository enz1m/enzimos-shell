use iced::{
    Rectangle, Size, Task, Vector,
    advanced::widget::{
        Id, Operation,
        operation::{Focusable, Outcome, Scrollable},
    },
    window,
};

pub fn bounds(window: window::Id, id: impl Into<Id>) -> Task<Option<Rectangle>> {
    iced_runtime::task::widget_in(window, Bounds::new(id.into()))
}
pub fn natural_size(window: window::Id, id: impl Into<Id>) -> Task<Option<Size>> {
    iced_runtime::task::widget_in(
        window,
        NaturalSize {
            id: id.into(),
            found: None,
            ambiguous: false,
        },
    )
}
struct NaturalSize {
    id: Id,
    found: Option<Size>,
    ambiguous: bool,
}
impl Operation<Option<Size>> for NaturalSize {
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        if id == Some(&self.id) {
            self.ambiguous |= self.found.is_some();
            self.found = Some(bounds.size());
        }
    }
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<Size>>)) {
        operate(self);
    }
    fn finish(&self) -> Outcome<Option<Size>> {
        Outcome::Some(if self.ambiguous { None } else { self.found })
    }
}
struct Bounds {
    id: Id,
    found: Option<Rectangle>,
    seen: bool,
    ambiguous: bool,
    offset: Vector,
    clip: Option<Rectangle>,
    pending: Option<(Rectangle, Vector)>,
}
impl Bounds {
    fn new(id: Id) -> Self {
        Self {
            id,
            found: None,
            seen: false,
            ambiguous: false,
            offset: Vector::ZERO,
            clip: None,
            pending: None,
        }
    }
}
impl Operation<Option<Rectangle>> for Bounds {
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
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<Rectangle>>)) {
        let old = (self.offset, self.clip);
        if let Some((bounds, translation)) = self.pending.take() {
            let bounds = bounds - self.offset;
            self.clip = Some(match self.clip {
                Some(clip) => clip.intersection(&bounds).unwrap_or_default(),
                None => bounds,
            });
            self.offset += translation;
        }
        operate(self);
        self.offset = old.0;
        self.clip = old.1;
    }
    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _: &mut dyn Focusable) {
        if id == Some(&self.id) {
            self.ambiguous |= self.seen;
            self.seen = true;
            let bounds = bounds - self.offset;
            self.found = match self.clip {
                Some(clip) => clip.intersection(&bounds),
                None => Some(bounds),
            };
        }
    }
    fn finish(&self) -> Outcome<Option<Rectangle>> {
        Outcome::Some(if self.ambiguous { None } else { self.found })
    }
}
