use super::{Message, Shell};
use enzui_shell::desktop_preferences::OptionKey;
use enzui_shell::i18n::tr;
use enzui_shell::settings::Change;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    clickable::enz_clickable,
    icon::enz_icon,
    preferences_row::enz_preferences_row,
};
use iced::widget::column;
use iced::{Element, Task};
use std::time::Duration;

// The pointer size the menu's "Larger pointer" switches to.
const LARGE_POINTER: i32 = 48;
const POINTER: i32 = 24;

pub(crate) fn screen_reader() -> Option<bool> {
    enzui_shell::compositor::screen_reader_state()
}

impl Shell {
    // The bar item the Control Center opens from.
    pub(crate) fn control_trigger(&self) -> &'static str {
        if self.accessibility_menu {
            "accessibility-item"
        } else {
            "control-center"
        }
    }

    pub(crate) fn accessibility_item(&self) -> Element<'_, Message> {
        enzui_shell::bar_item::trigger(
            enz_clickable(
                enz_icon("accessibility")
                    .size(f32::from(self.metrics.icon))
                    .into_element(),
                Message::ToggleAccessibility,
            ),
            self.metrics,
        )
        .aria_label(enzui_shell::i18n::tr("shell-accessibility"))
        .id("accessibility-item".into())
        .into_element()
    }

    pub(crate) fn toggle_screen_reader(&mut self) -> Task<Message> {
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                let result = compositor.toggle_screen_reader().await;
                // Orca takes a moment to start or stop.
                tokio::time::sleep(Duration::from_millis(1200)).await;
                (
                    result,
                    tokio::task::spawn_blocking(screen_reader)
                        .await
                        .ok()
                        .flatten(),
                )
            },
            |(result, state)| Message::ScreenReaderState(result, state),
        )
    }

    // Asks every enzUI app to keep still, as Settings › Accessibility does,
    // and turns window animations off or back on.
    pub(crate) fn set_reduced_motion(&mut self, reduced: bool) -> Task<Message> {
        let mut store =
            enzui_shell::theme_store::Store::read(&enzui_core::theme_file::ThemePaths::from_env());
        if !store.stage_reduced_motion(reduced) {
            self.notice = Some(tr("shell-reduced-motion-could-not-be-set").into());
            return Task::none();
        }
        if let Err(error) = store.save() {
            self.notice = Some(tr!("shell-reduced-motion-could-not-be-saved", "arg0" => error));
            return Task::none();
        }
        self.theme_document = std::sync::Arc::new(store.into_document());
        self.save_setting(Change::Compositor(
            OptionKey::Animations,
            i32::from(!reduced),
        ))
    }

    pub(crate) fn accessibility_menu(&self) -> Element<'_, Message> {
        let compositor = &self.desired_settings.compositor;
        let zoom = i32::from(compositor.zoom);
        let zoomed = zoom > enzui_shell::desktop_preferences::ZOOM_MIN;
        let large = i32::from(compositor.cursor_size) >= LARGE_POINTER;
        let reduced =
            enzui_core::theme_file::document_reduced_motion(&self.theme_document).unwrap_or(false);
        let busy = self.control_placing;
        let switch =
            |id: &str, title: &'static str, subtitle: String, on: bool, message: Message| {
                enz_preferences_row(title)
                    .subtitle(subtitle)
                    .id_prefix(format!("accessibility-{id}"))
                    .switch(on, move |_| message.clone())
                    .enabled(!busy)
                    .into_element()
            };
        let mut menu = column![
            switch(
                "zoom",
                "Zoom",
                if zoomed {
                    format!("{zoom}% · Super+Alt+= and Super+Alt+- change it")
                } else {
                    tr("shell-magnifies-around-the-pointer").into()
                },
                zoomed,
                Message::Command(enzui_shell::commands::Command::ZoomReset),
            ),
            switch(
                "pointer",
                tr("shell-larger-pointer"),
                format!("{} px", compositor.cursor_size),
                large,
                Message::QuickSetting(Change::Compositor(
                    OptionKey::CursorSize,
                    if large { POINTER } else { LARGE_POINTER }
                )),
            ),
            switch(
                "motion",
                tr("shell-reduce-motion"),
                tr("shell-no-window-animations-enzui-apps-keep-still").into(),
                reduced,
                Message::ReduceMotion(!reduced)
            ),
            switch(
                "keyboard",
                tr("shell-on-screen-keyboard"),
                tr("shell-types-into-the-window-you-are-using").into(),
                self.osk.shown(),
                Message::ToggleOsk
            ),
        ]
        .spacing(2);
        if let Some(running) = self.screen_reader {
            menu = menu.push(switch(
                "reader",
                tr("shell-screen-reader"),
                "Orca · Super+Alt+S".into(),
                running,
                Message::ScreenReader,
            ));
        }
        menu.push(
            enz_button(
                tr("shell-accessibility-settings"),
                Message::SettingsPage("accessibility"),
            )
            .kind(ButtonKind::Quiet)
            .icon("settings")
            .into_element(),
        )
        .into()
    }
}
