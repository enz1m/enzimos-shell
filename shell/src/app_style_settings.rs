//! Appearance › App styles: one application's own mode, accent and corners,
//! written to its theme file beside the shared one.

use super::{Message, SettingsApp};
use enzui_core::theme_file::ModePreference;
use enzui_shell::app_theme::{Change, File, Target, targets};
use enzui_shell::i18n::tr;
use enzui_shell::theme_palette::{self, PRESETS};
use enzui_shell::theme_presets::Corners;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    color_swatch::enz_color_swatch,
};
use iced::{
    Alignment, Element,
    widget::{column, container, row},
};

/// What the App styles card asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Action {
    Select(&'static str),
    Change(Change),
    Reset,
}

/// The application being styled and its file.
#[derive(Debug, Clone)]
pub(super) struct State {
    app: Target,
    file: File,
}

impl State {
    /// Read the application's file again, after a theme file changed.
    pub(super) fn refresh(&mut self) {
        self.file = File::read(&super::theme_paths(), self.app.id);
    }
}

impl Default for State {
    fn default() -> Self {
        let app = targets()[0];
        Self {
            app,
            file: File::read(&super::theme_paths(), app.id),
        }
    }
}

fn mode_label(mode: Option<ModePreference>) -> &'static str {
    match mode {
        None | Some(ModePreference::System) => tr("shell-same-as-the-desktop"),
        Some(ModePreference::Light) => tr("shell-light"),
        Some(ModePreference::Dark) => tr("shell-dark"),
    }
}

fn corners_label(corners: Option<Corners>) -> &'static str {
    corners.map_or_else(|| tr("shell-same-as-the-desktop"), Corners::label)
}

fn style(action: Change) -> Message {
    Message::AppStyle(Action::Change(action))
}

impl SettingsApp {
    pub(super) fn app_style_settings(&self) -> Element<'_, Message> {
        let state = &self.app_style;
        let current = state.file.style();
        let live = self.controls_live() && state.file.unusable().is_none();
        let app = self.choice_setting(
            "appearance/app_style",
            enzui_shell::i18n::label(state.app.name),
            targets()
                .into_iter()
                .map(|target| {
                    (
                        enzui_shell::i18n::label(target.name).to_owned(),
                        Message::AppStyle(Action::Select(target.id)),
                    )
                })
                .collect(),
        );
        let mode = self.choice_setting(
            "appearance/app_style_mode",
            mode_label(current.mode),
            [
                None,
                Some(ModePreference::Light),
                Some(ModePreference::Dark),
            ]
            .into_iter()
            .map(|mode| (mode_label(mode).to_owned(), style(Change::Mode(mode))))
            .collect(),
        );
        let corners = self.choice_setting(
            "appearance/app_style_corners",
            corners_label(current.corners),
            std::iter::once(None)
                .chain(Corners::ALL.iter().copied().map(Some))
                .map(|corners| {
                    (
                        corners_label(corners).to_owned(),
                        style(Change::Corners(corners)),
                    )
                })
                .collect(),
        );
        let mut swatches = row![].spacing(10).align_y(Alignment::Center);
        for (name, hex) in PRESETS {
            let color = theme_palette::rgb(hex);
            swatches = swatches.push(
                enz_color_swatch(color, style(Change::Accent(Some(color))))
                    .size(30.0)
                    .radius(15.0)
                    .selected(current.accent == Some(color))
                    .aria_label(name)
                    .id_prefix(format!("app-style-color-{}", name.to_lowercase()))
                    .enabled(live)
                    .into_element(),
            );
        }
        let swatches = swatches.push(
            enz_button(tr("shell-same-as-the-desktop"), style(Change::Accent(None)))
                .kind(ButtonKind::Secondary)
                .id("app-style-color-desktop".into())
                .enabled(live && current.accent.is_some())
                .into_element(),
        );
        // The colour row keeps the card's icon and text columns; the swatches
        // start under its title.
        let tokens = crate::settings_app::settings_row_tokens();
        let accent = column![
            crate::settings_app::settings_row(tr("shell-color"))
                .icon("palette")
                .into_element(),
            container(swatches.wrap().vertical_spacing(10)).padding(
                iced::Padding::ZERO
                    .left(tokens.padding_x + tokens.icon_slot + tokens.gap)
                    .right(12.0)
                    .bottom(8.0)
            ),
        ];
        let status: String = match state.file.unusable() {
            Some(reason) => tr!("shell-app-style-untouched", "arg0" => reason.to_owned()),
            None if current == Default::default() => {
                tr("shell-app-style-follows-the-desktop").into()
            }
            None => tr!("shell-app-style-has-its-own", "arg0" => format!("{}.toml", state.app.id)),
        };
        let reset = crate::settings_app::settings_row(status)
            .icon("rotate-ccw")
            .control(
                enz_button(
                    tr("shell-use-the-desktop-style"),
                    Message::AppStyle(Action::Reset),
                )
                .kind(ButtonKind::Secondary)
                .id("app-style-reset".into())
                .enabled(self.controls_live() && current != Default::default())
                .into_element(),
            )
            .into_element();
        self.section_card_for(
            "appearance/app_style",
            column![app, mode, accent, corners, reset].spacing(4),
        )
        .title(tr("shell-app-styles"))
        .description(tr("shell-app-styles-description"))
        .into_element()
    }

    pub(super) fn app_style_update(&mut self, action: Action) -> iced::Task<Message> {
        let paths = super::theme_paths();
        match action {
            Action::Select(id) => {
                if let Some(app) = targets().into_iter().find(|target| target.id == id) {
                    self.app_style = State {
                        app,
                        file: File::read(&paths, app.id),
                    };
                }
            }
            Action::Change(change) => {
                // The file may have been edited since it was read.
                self.app_style.file = File::read(&paths, self.app_style.app.id);
                if self.app_style.file.apply(change) {
                    if let Err(error) = self.app_style.file.save() {
                        self.error = Some(error);
                    }
                } else {
                    self.error = Some(tr("shell-theme-controls-refused").into());
                }
            }
            Action::Reset => {
                if let Err(error) = self.app_style.file.reset() {
                    self.error = Some(error);
                }
            }
        }
        iced::Task::none()
    }
}
