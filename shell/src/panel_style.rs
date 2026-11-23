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

/// The selected theme's desktop tint, when it has one.
pub fn themed_surface(
    scheme: Option<&str>,
    document: &enzui_core::theme_file::ThemeDocument,
    light: bool,
) -> Option<[u8; 3]> {
    let scheme = crate::theme_palette::Scheme::from_id_str(scheme?)?;
    if scheme.style == crate::theme_palette::Style::Neutral {
        return None;
    }
    // Light themes carry their tint on the page, where `surface` stays white.
    let (token, mode) = if light {
        ("palette.background", enzui_theme::EnzThemeMode::Light)
    } else {
        ("palette.surface", enzui_theme::EnzThemeMode::Dark)
    };
    match document.get(token, mode)? {
        enzui_core::theme_file::registry::TokenValue::Color(color) => {
            Some([color.r, color.g, color.b].map(|v| (v * 255.0).round() as u8))
        }
        _ => None,
    }
}
