//! One look for the panels opened from the bar: the same inset around their
//! content, the same gap between their parts and the same title.

/// Space between a panel's edge and its content.
pub const INSET: f32 = 16.0;
/// Space between a panel's parts: its title, tabs and list.
pub const GAP: f32 = 12.0;

/// A panel title's size at the desktop's text size.
pub fn title_px(text: f32) -> f32 {
    text + 3.0
}
