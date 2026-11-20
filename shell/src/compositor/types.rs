use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Compositor {
    pub(super) executable: PathBuf,
    pub(super) instance: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub workspace: u32,
    pub tiling: bool,
    /// The active keyboard layout while several are configured.
    pub keyboard_layout: Option<String>,
    pub keyboard: Option<Keyboard>,
}

/// The main keyboard's layouts as the compositor has them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyboard {
    /// Layout codes (`us`, `hu`), in switching order.
    pub layouts: Vec<String>,
    pub active: usize,
    /// The active keymap's name (`English (US)`).
    pub keymap: String,
}

#[derive(Clone, Debug, Default)]
pub struct ControlGeometry {
    pub width: f32,
    pub height: f32,
    pub output: String,
    pub attachment: crate::reveal::Attachment,
}

#[derive(Clone, Debug)]
pub struct ControlContent {
    pub grid: crate::control_layout::VisibleLayout,
    pub measured: Option<iced::Size>,
}
impl ControlContent {
    pub fn preferred_height(&self, width: f32, row: f32, padding: f32, join: f32) -> f32 {
        let minimum = self.grid.height(width, row) + crate::control_layout::ACTIONS_ROW;
        let body = self
            .measured
            .filter(|size| {
                size.width.is_finite()
                    && size.height.is_finite()
                    && (size.width - width).abs() <= 0.75
                    && (0.0..=65536.0).contains(&size.height)
            })
            .map_or(minimum, |size| size.height.max(minimum));
        (body + padding * 2.0 + join).max(180.0)
    }
}
impl From<crate::control_layout::VisibleLayout> for ControlContent {
    fn from(grid: crate::control_layout::VisibleLayout) -> Self {
        Self {
            grid,
            measured: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PopupPlacement {
    pub bar: crate::settings::BarEdge,
    pub left: u32,
    pub right: u32,
    pub metrics: crate::settings::Metrics,
    pub anchor: Option<iced::Rectangle>,
    pub body: iced::Size,
}

#[derive(Clone, Copy, Debug)]
pub struct ToastPlacement {
    pub bar: crate::settings::BarEdge,
    pub metrics: crate::settings::Metrics,
    pub left: u32,
    pub right: u32,
    /// Width of one card; the window adds the shadow margin around it.
    pub width: f32,
    /// Card heights in stacking order; only the first `count` are used.
    pub heights: [f32; 5],
    pub count: usize,
    pub position: crate::notifications::preferences::Position,
    /// Extra room on the cards' inner side for an overshooting slide, up to
    /// [`MAX_TOAST_OVERSHOOT`]; it takes no input.
    pub overshoot: f32,
}

pub const MAX_TOAST_OVERSHOOT: f32 = 64.0;
