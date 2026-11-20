//! Shared desktop window settings, sizing and clock text.
use super::{Message, i18n};
use iced::{Task, window};

pub(super) fn resize_chrome(id: window::Id, size: iced::Size) -> Task<Message> {
    window::set_max_size(id, None)
        .chain(window::set_min_size(id, Some(size)))
        .chain(window::set_max_size(id, Some(size)))
        .chain(window::resize(id, size))
}

pub(super) fn clock(session: &enzui_shell::session_preferences::SessionPreferences) -> String {
    {
        let now = chrono::Local::now();
        format!(
            "{}   {}",
            i18n::clock_date(now.date_naive(), true),
            i18n::time(&now, session.clock_24h)
        )
    }
}

pub(super) fn window_settings(app_id: &str, width: f32, height: f32) -> window::Settings {
    window::Settings {
        size: iced::Size::new(width, height),
        resizable: false,
        decorations: false,
        transparent: matches!(
            app_id,
            "dev.enzui.Dock"
                | "dev.enzui.Desktop"
                | "dev.enzui.Launcher"
                | "dev.enzui.ControlCenter"
                | "dev.enzui.QuickSearch"
                | "dev.enzui.NotificationCenter"
                | "dev.enzui.NotificationToasts"
                | "dev.enzui.Osd"
                | "dev.enzui.Switcher"
                | "dev.enzui.Capture"
                | "dev.enzui.WindowSpread"
                | "dev.enzui.EmojiPicker"
                | "dev.enzui.Keyboard"
                | "dev.enzui.PluginPanel"
                | "dev.enzui.AssistantPanel"
                | "dev.enzui.AssistantTrigger"
                | "dev.enzui.WorkspaceOverview"
                | "dev.enzui.Sidebar"
                | "dev.enzui.SidebarPane"
                | "dev.enzui.SidebarBacking"
        ),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: app_id.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
