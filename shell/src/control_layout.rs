use crate::settings::QuickAction;
use enzui_shell::i18n::tr;
use iced::{Point, Rectangle};
use serde_json::{Value, json};

pub const GAP: f32 = 8.0;
pub const MIN_CELL: f32 = 44.0;
/// The row of system actions under the controls, with the gap above it.
pub const ACTIONS_ROW: f32 = 42.0;
/// How much wider than tall a cell is: a two-cell control then has room for
/// its name beside its icon.
const CELL_EXTRA: f32 = 12.0;
pub const PREVIEW_WIDTH: f32 = 6.0 * (MIN_CELL + CELL_EXTRA) + 5.0 * GAP;

pub fn row_height(font: f32) -> f32 {
    // A control is a touch taller than a button and two lines of its text:
    // at 52 the tiles dwarfed every other control on the desktop.
    (font * 2.0 + 18.0).max(44.0)
}

pub fn grid_width(font: f32) -> f32 {
    grid_width_for(font, 6)
}

pub fn grid_width_for(font: f32, count: u8) -> f32 {
    (f32::from(count.clamp(2, 6)) * (row_height(font) + CELL_EXTRA)
        + f32::from(count.clamp(2, 6) - 1) * GAP)
        .max(232.0)
}

pub fn columns(width: f32, row_height: f32) -> usize {
    let minimum = MIN_CELL.max(row_height) + CELL_EXTRA;
    (((width + GAP) / (minimum + GAP)).floor().clamp(1.0, 6.0) as usize).max(1)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    Compact,
    #[default]
    Pill,
    Triple,
    Wide,
}
impl Shape {
    pub const ALL: [Self; 4] = [Self::Compact, Self::Pill, Self::Triple, Self::Wide];
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => tr("shell-compact"),
            Self::Pill => "2 wide",
            Self::Triple => "3 wide",
            Self::Wide => tr("shell-full-width"),
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Pill => "pill",
            Self::Triple => "triple",
            Self::Wide => "wide",
        }
    }
    fn parse(value: &Value) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|shape| Some(shape.key()) == value.as_str())
            .ok_or_else(|| tr("shell-unknown-control-center-size").into())
    }
    pub fn span(self, columns: usize) -> usize {
        match self {
            Self::Compact => 1,
            Self::Pill => 2.min(columns),
            Self::Triple => 3.min(columns),
            Self::Wide => columns,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    order: Vec<QuickAction>,
    shapes: [Shape; QuickAction::ALL.len()],
    columns: u8,
}
impl Default for Layout {
    fn default() -> Self {
        use QuickAction::*;
        let mut value = Self {
            order: QuickAction::ALL.to_vec(),
            shapes: [Shape::Compact; QuickAction::ALL.len()],
            columns: 6,
        };
        // Toggles whose glyph alone is ambiguous carry their name.
        for action in [Network, Bluetooth, Appearance, Tiling] {
            value.set_shape(action, Shape::Pill);
        }
        for action in [Volume, Brightness] {
            value.set_shape(action, Shape::Wide);
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisibleLayout {
    pub layout: std::sync::Arc<Layout>,
    pub actions: Vec<QuickAction>,
}
impl VisibleLayout {
    pub fn height(&self, width: f32, row_height: f32, detail: Option<(QuickAction, f32)>) -> f32 {
        self.layout
            .placements_with_detail(
                width,
                row_height,
                |action| self.actions.contains(&action),
                detail,
                None,
            )
            .iter()
            .map(|p| {
                p.bounds.y
                    + p.bounds.height
                    + detail
                        .filter(|(action, _)| *action == p.action)
                        .map_or(0.0, |(_, height)| height)
            })
            .fold(0.0, f32::max)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub action: QuickAction,
    pub bounds: Rectangle,
}
#[derive(Clone, Debug)]
pub struct DropSlot {
    pub before: Option<QuickAction>,
    pub bounds: Rectangle,
}

pub fn closest_slot(slots: &[DropSlot], origin: Point) -> Option<&DropSlot> {
    slots.iter().min_by(|a, b| {
        let distance = |slot: &DropSlot| {
            let delta = origin - slot.bounds.position();
            delta.x * delta.x + delta.y * delta.y * 4.0
        };
        distance(a).total_cmp(&distance(b))
    })
}

impl Layout {
    pub fn drop_slots(
        &self,
        action: QuickAction,
        width: f32,
        height: f32,
        visible: &[QuickAction],
    ) -> Vec<DropSlot> {
        self.drop_slots_with_detail(action, width, height, visible, None)
    }
    pub fn drop_slots_with_detail(
        &self,
        action: QuickAction,
        width: f32,
        height: f32,
        visible: &[QuickAction],
        detail: Option<(QuickAction, f32)>,
    ) -> Vec<DropSlot> {
        let mut anchors: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|key| *key != action && visible.contains(key))
            .map(Some)
            .collect();
        anchors.push(None);
        let mut slots: Vec<DropSlot> = self
            .placements_with_detail(width, height, |key| visible.contains(&key), detail, None)
            .into_iter()
            .filter(|p| p.action == action)
            .map(|p| DropSlot {
                before: Some(action),
                bounds: p.bounds,
            })
            .collect();
        for before in anchors {
            let mut candidate = self.clone();
            candidate.move_before(action, before);
            if let Some(placement) = candidate
                .placements_with_detail(width, height, |key| visible.contains(&key), detail, None)
                .into_iter()
                .find(|p| p.action == action)
                && !slots.iter().any(|slot| slot.bounds == placement.bounds)
            {
                slots.push(DropSlot {
                    before,
                    bounds: placement.bounds,
                });
            }
        }
        slots
    }
    pub fn order(&self) -> &[QuickAction] {
        &self.order
    }
    pub fn column_count(&self) -> u8 {
        self.columns
    }
    pub fn set_column_count(&mut self, count: u8) -> Result<(), String> {
        if !(2..=6).contains(&count) {
            return Err(tr("shell-control-center-columns-must-be-between-2-and-6").into());
        }
        self.columns = count;
        Ok(())
    }
    pub fn shape(&self, action: QuickAction) -> Shape {
        if action == QuickAction::Volume {
            Shape::Wide
        } else {
            self.shapes[action as usize]
        }
    }
    pub fn set_shape(&mut self, action: QuickAction, shape: Shape) {
        self.shapes[action as usize] = if action == QuickAction::Volume {
            Shape::Wide
        } else {
            shape
        };
    }
    pub fn move_before(&mut self, action: QuickAction, before: Option<QuickAction>) {
        if before == Some(action) {
            return;
        }
        self.order.retain(|entry| *entry != action);
        let index = before
            .and_then(|anchor| self.order.iter().position(|entry| *entry == anchor))
            .unwrap_or(self.order.len());
        self.order.insert(index, action);
    }
    pub fn from_json(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let object = value
            .as_object()
            .ok_or_else(|| tr("shell-invalid-control-center-layout"))?;
        if object
            .get("version")
            .is_some_and(|version| version.as_u64() != Some(1))
        {
            return Err(tr("shell-unsupported-control-center-layout-version").into());
        }
        let mut layout = Self::default();
        if let Some(count) = object.get("columns") {
            layout.set_column_count(count.as_u64().unwrap_or(0).try_into().unwrap_or(0))?;
        }
        if let Some(order) = object.get("order") {
            // Controls added since the order was saved follow it, in their
            // default order.
            let order = order
                .as_array()
                .filter(|entries| !entries.is_empty() && entries.len() <= QuickAction::ALL.len())
                .ok_or_else(|| tr("shell-control-center-order-must-name-its-controls"))?;
            let mut actions = Vec::new();
            for key in order {
                let action = action_key(key)?;
                if actions.contains(&action) {
                    return Err(tr("shell-duplicate-control-center-control").into());
                }
                actions.push(action);
            }
            for action in QuickAction::ALL {
                if !actions.contains(&action) {
                    actions.push(action);
                }
            }
            layout.order = actions;
        }
        if let Some(shapes) = object.get("sizes") {
            let shapes = shapes
                .as_object()
                .ok_or_else(|| tr("shell-invalid-control-center-sizes"))?;
            if shapes.len() > QuickAction::ALL.len() {
                return Err(tr("shell-too-many-control-center-sizes").into());
            }
            for (key, shape) in shapes {
                layout.set_shape(
                    action_key(&Value::String(key.clone()))?,
                    Shape::parse(shape)?,
                );
            }
        }
        Ok(layout)
    }
    /// Writes owned layout fields without removing peer extension fields.
    pub fn write_json(&self, value: &mut Value) -> Result<(), String> {
        if value.is_null() {
            *value = json!({});
        }
        Self::from_json(Some(value))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-control-center-layout"))?;
        object.insert("version".into(), json!(1));
        object.insert("columns".into(), json!(self.columns));
        object.insert(
            "order".into(),
            json!(
                self.order
                    .iter()
                    .map(|action| action.key())
                    .collect::<Vec<_>>()
            ),
        );
        object.insert(
            "sizes".into(),
            Value::Object(
                QuickAction::ALL
                    .into_iter()
                    .map(|action| {
                        (
                            action.key().into(),
                            Value::String(self.shape(action).key().into()),
                        )
                    })
                    .collect(),
            ),
        );
        Ok(())
    }
    fn grid_metrics(&self, width: f32, height: f32) -> (usize, f32) {
        let columns = columns(width, height).min(self.columns as usize);
        let cell = ((width + GAP) / columns as f32 - GAP).max(height.min(width));
        (columns, cell)
    }

    pub fn shape_after_resize(
        &self,
        action: QuickAction,
        delta: f32,
        width: f32,
        height: f32,
    ) -> Shape {
        let (columns, cell) = self.grid_metrics(width, height);
        let original = self.shape(action);
        // Row stretching changes the handle's starting position without changing its saved span.
        let target = original.span(columns) as f32 * (cell + GAP) + delta;
        Shape::ALL
            .into_iter()
            .min_by(|a, b| {
                let distance =
                    |shape: Shape| (target - shape.span(columns) as f32 * (cell + GAP)).abs();
                distance(*a)
                    .total_cmp(&distance(*b))
                    .then((*a != original).cmp(&(*b != original)))
            })
            .unwrap_or(original)
    }

    /// Packs visible controls in logical pixels; hidden controls retain their saved order and size.
    pub fn placements(
        &self,
        width: f32,
        height: f32,
        visible: impl Fn(QuickAction) -> bool,
    ) -> Vec<Placement> {
        self.placements_resizing(width, height, visible, None)
    }
    pub fn placements_with_detail(
        &self,
        width: f32,
        height: f32,
        visible: impl Fn(QuickAction) -> bool,
        detail: Option<(QuickAction, f32)>,
        resizing: Option<(QuickAction, f32)>,
    ) -> Vec<Placement> {
        let detail = detail
            .filter(|(action, height)| visible(*action) && height.is_finite() && *height >= 0.0);
        let mut places = self.placements_resizing(width, height, visible, resizing);
        if let Some((action, extra)) = detail
            && let Some(y) = places
                .iter()
                .find(|p| p.action == action)
                .map(|p| p.bounds.y)
        {
            for p in &mut places {
                if p.bounds.y > y {
                    p.bounds.y += extra;
                }
            }
        }
        places
    }
    pub fn placements_resizing(
        &self,
        width: f32,
        height: f32,
        visible: impl Fn(QuickAction) -> bool,
        resizing: Option<(QuickAction, f32)>,
    ) -> Vec<Placement> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Vec::new();
        }
        let (columns, cell) = self.grid_metrics(width, height);
        let mut used = 0;
        let mut row = 0;
        let mut placements = Vec::new();
        for &action in self.order.iter().filter(|&&action| visible(action)) {
            let resized = resizing
                .filter(|(key, value)| {
                    *key == action && action != QuickAction::Volume && value.is_finite()
                })
                .map(|(_, value)| value.clamp(height.min(width), width));
            let span = resized.map_or_else(
                || self.shape(action).span(columns),
                |width| (((width + GAP) / (cell + GAP)).ceil() as usize).clamp(1, columns),
            );
            if used + span > columns {
                row += 1;
                used = 0;
            }
            let tile_width =
                resized.unwrap_or_else(|| span as f32 * cell + (span - 1) as f32 * GAP);
            placements.push(Placement {
                action,
                bounds: Rectangle {
                    x: 0.0,
                    y: row as f32 * (height + GAP),
                    width: tile_width,
                    height: height.min(tile_width),
                },
            });
            used += span;
        }
        for row in placements.chunk_by_mut(|a, b| a.bounds.y == b.bounds.y) {
            let used = row.iter().map(|tile| tile.bounds.width).sum::<f32>()
                + (row.len() - 1) as f32 * GAP;
            let spare = (width - used).max(0.0);
            let available = |action| !resizing.is_some_and(|(key, _)| key == action);
            let has_wide_tiles = row
                .iter()
                .any(|tile| available(tile.action) && self.shape(tile.action) != Shape::Compact);
            let stretchable = |action| {
                available(action) && (!has_wide_tiles || self.shape(action) != Shape::Compact)
            };
            let stretch_count = row.iter().filter(|tile| stretchable(tile.action)).count();
            let extra = spare / stretch_count.max(1) as f32;
            let mut x = 0.0;
            for tile in row {
                tile.bounds.x = x;
                if stretchable(tile.action) {
                    tile.bounds.width += extra;
                }
                x += tile.bounds.width + GAP;
            }
        }
        placements
    }
}

fn action_key(value: &Value) -> Result<QuickAction, String> {
    QuickAction::ALL
        .into_iter()
        .find(|action| Some(action.key()) == value.as_str())
        .ok_or_else(|| tr("shell-unknown-control-center-control").into())
}
