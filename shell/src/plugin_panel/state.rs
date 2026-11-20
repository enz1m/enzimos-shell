use super::model::{LayoutRequest, Message, State};
use crate::Message as ShellMessage;
use enzui_shell::i18n::tr;
use iced::window;
use std::sync::atomic::Ordering;

impl State {
    pub(crate) fn theme_tokens(
        &self,
        base: &enzui_theme::EnzTheme,
    ) -> Option<enzui_theme::EnzTheme> {
        let claim = self.theme.as_ref()?;
        claim
            .authority
            .load(Ordering::Acquire)
            .then(|| claim.colors.apply(base).ok())
            .flatten()
    }
    pub(crate) fn theme_surface(&self, base: &enzui_theme::EnzTheme) -> Option<[u8; 3]> {
        let color = self.theme_tokens(base)?.palette.surface;
        Some([color.r, color.g, color.b].map(|v| (v * 255.0).round() as u8))
    }
    pub(crate) fn owns_window(&self, id: window::Id) -> bool {
        self.pane.as_ref().is_some_and(|p| p.window == id) || self.retiring == Some(id)
    }
}
pub(super) fn message(message: Message) -> ShellMessage {
    ShellMessage::Plugin(message)
}
pub(super) fn widget_id(qualified: &str) -> String {
    format!("plugin-element:{qualified}")
}
pub(super) fn requested_layout(
    current: &enzui_shell::bar_layout::BarLayout,
    request: &LayoutRequest,
) -> Result<enzui_shell::bar_layout::BarLayout, String> {
    if !request.authority.load(Ordering::Acquire) {
        return Err(tr("shell-plugin-layout-authority-changed").into());
    }
    let mut layout = current.clone();
    layout.move_before(
        &[request.element.as_str()],
        request.section,
        request.before.as_deref(),
    )?;
    if let Some(enabled) = request.enabled {
        layout.set_enabled(&request.element, enabled)?;
    }
    Ok(layout)
}
