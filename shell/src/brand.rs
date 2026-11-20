//! The enzimOS mark, for the places where the system names itself.
//!
//! Brand rules (enzimos-icons `brand.html`): ink `#1C1C1C` on light surfaces,
//! paper `#F1F0EE` on dark ones, never another colour; the small mark is the
//! drawing for 16–64 px.

use iced::Color;

const MARK_SMALL: &[u8] = include_bytes!("../../assets/brand/enzimos-mark-small.svg");

pub const INK: Color = Color::from_rgb8(0x1C, 0x1C, 0x1C);
pub const PAPER: Color = Color::from_rgb8(0xF1, 0xF0, 0xEE);

/// The small enzimOS mark at `size` px, ink on light themes and paper on dark ones.
pub fn mark<'a>(size: f32) -> enzui_widgets::icon::EnzIcon<'a> {
    enzui_widgets::icon::enz_icon_from_handle(iced::widget::svg::Handle::from_memory(MARK_SMALL))
        .size(size)
        .tint_resolved(|theme| {
            if enzui_widgets::theme_lookup::palette(theme).is_dark() {
                PAPER
            } else {
                INK
            }
        })
}

/// Whether an os-release name is enzimOS (the mark is not shown for other systems).
pub fn is_enzimos(os: &str) -> bool {
    os.trim_start().to_ascii_lowercase().starts_with("enzimos")
}
