use super::{Message, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    stepper::enz_stepper,
};
use iced::{Element, Length, widget::container};

impl SettingsApp {
    pub(super) fn section_card<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> enzui_widgets::card::EnzCard<'a, Message> {
        super::sections::section_with_scale(self.header_scale(), content)
    }
    pub(super) fn section_card_for<'a>(
        &self,
        route: &str,
        content: impl Into<Element<'a, Message>>,
    ) -> super::sections::SectionCard<'a, Message> {
        super::sections::section_for_scaled(self.header_scale(), route, content)
    }
    pub(super) fn preview_frame<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let theme = self.tokens();
        let palette = theme.palette;
        let label = iced::widget::row![
            enzui_widgets::enz_icon("eye")
                .size(enzui_theme::text_scale::effective().apply_px(14.0))
                .into_element(),
            enzui_widgets::enz_text(enzui_shell::i18n::tr("shell-preview"))
                .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .muted_on(palette.background)
                .into_element(),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        container(iced::widget::column![label, content.into()].spacing(10))
            .padding(12)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(palette.background.into()),
                border: iced::Border {
                    radius: enzui_theme::text_defaults::shape_radius(|shape| shape.card).into(),
                    width: 1.0,
                    color: palette.border,
                },
                ..Default::default()
            })
            .into()
    }
    fn header_scale(&self) -> f32 {
        self.tokens().typography.scale_factor().factor()
    }
    pub(super) fn controls_live(&self) -> bool {
        !self.closing && !self.suspended.get()
    }
    // Dependent rows stay mounted and readable while inert, so a reader can see
    // what the governing preference controls before turning it on.
    pub(super) fn dependent<T>(&self, active: bool, rows: impl FnOnce() -> T) -> T {
        let restore = self.suspended.replace(!active || self.suspended.get());
        let built = rows();
        self.suspended.set(restore);
        built
    }
    pub(super) fn mark_control_anchor<'a>(
        &self,
        key: &str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        container(content)
            .id(format!("setting-{key}"))
            .width(Length::Fill)
            .into()
    }
    pub(super) fn mark_setting<'a>(
        &self,
        key: &str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        self.setting_target(key, content, true)
    }
    pub(super) fn anchor_setting<'a>(
        &self,
        key: &str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        self.setting_target(key, content, false)
    }
    fn setting_target<'a>(
        &self,
        key: &str,
        content: impl Into<Element<'a, Message>>,
        decorate: bool,
    ) -> Element<'a, Message> {
        let highlight = decorate && self.highlighted == Some(key);
        let theme = self.tokens();
        let palette = theme.palette;
        container(content)
            .id(format!("setting-{key}"))
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: highlight.then_some(palette.accent_soft.into()),
                border: iced::Border {
                    color: if highlight {
                        palette.accent
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: enzui_theme::text_defaults::shape_radius(|shape| shape.row).into(),
                },
                ..Default::default()
            })
            .into()
    }
    pub(super) fn switch_setting<'a>(
        &self,
        key: &'static str,
        value: bool,
        on_change: impl Fn(bool) -> Message + 'a,
    ) -> Element<'a, Message> {
        self.switch_setting_noted(key, value, row_note(key).unwrap_or(""), on_change)
    }
    pub(super) fn switch_setting_noted<'a>(
        &self,
        key: &'static str,
        value: bool,
        note: &'static str,
        on_change: impl Fn(bool) -> Message + 'a,
    ) -> Element<'a, Message> {
        let entry = enzui_shell::settings_catalog::find(key).expect("Registered switch setting");
        self.mark_setting(
            key,
            crate::settings_app::settings_row(entry.title())
                .icon(enzui_shell::settings_catalog::icon(key).unwrap_or(""))
                .subtitle(note)
                .id_prefix(format!("control-{key}"))
                .switch(value, on_change)
                .enabled(self.controls_live())
                .into_element(),
        )
    }
    pub(super) fn number_setting<'a>(
        &'a self,
        key: &'static str,
        value: i32,
        min: i32,
        max: i32,
        unit: &str,
        on_change: impl Fn(i32) -> Message + 'a,
    ) -> Element<'a, Message> {
        let entry = enzui_shell::settings_catalog::find(key).expect("Registered numeric setting");
        if matches!(key, "quick-search/rows" | "notifications/count") {
            return self.mark_setting(
                key,
                crate::settings_app::settings_row(entry.title())
                    .icon(enzui_shell::settings_catalog::icon(key).unwrap_or(""))
                    .control(
                        enz_stepper(value, on_change)
                            .min(min)
                            .max(max)
                            .value_text(format!("{value}{unit}"))
                            .id(format!("control-{key}"))
                            .enabled(self.controls_live())
                            .into_element(),
                    )
                    .enabled(self.controls_live())
                    .into_element(),
            );
        }
        let Message::Change(template) = on_change(value) else {
            unreachable!("Numeric settings use typed preference changes");
        };
        let spec = super::numeric::Spec {
            key: key.into(),
            epoch: self.numeric.epoch,
            value,
            min,
            max,
            unit: unit.into(),
            template,
        };
        self.mark_setting(key, self.numeric_view(spec, entry.title()))
    }
    pub(super) fn action_setting<'a>(
        &self,
        key: &'static str,
        verb: &'a str,
        icon: &'static str,
        kind: ButtonKind,
        on_click: Message,
    ) -> Element<'a, Message> {
        let entry = enzui_shell::settings_catalog::find(key).expect("Registered action setting");
        self.mark_setting(
            key,
            crate::settings_app::settings_row(entry.title())
                .icon(enzui_shell::settings_catalog::icon(key).unwrap_or(""))
                .id_prefix(format!("control-{key}"))
                .control(
                    enz_button(verb, on_click)
                        .kind(kind)
                        .icon(icon)
                        .id(format!("control-{key}-button").into())
                        .enabled(self.controls_live())
                        .into_element(),
                )
                .enabled(self.controls_live())
                .into_element(),
        )
    }
    pub(super) fn choice_setting<'a>(
        &self,
        key: &'static str,
        current: impl Into<String>,
        choices: Vec<(String, Message)>,
    ) -> Element<'a, Message> {
        let entry = enzui_shell::settings_catalog::find(key).expect("Registered choice setting");
        let current = current.into();
        let width = value_menu_width(
            std::iter::once(current.as_str())
                .chain(choices.iter().map(|(label, _)| label.as_str())),
        );
        self.mark_setting(
            key,
            crate::settings_app::settings_row(entry.title())
                .icon(enzui_shell::settings_catalog::icon(key).unwrap_or(""))
                .subtitle(row_note(key).unwrap_or(""))
                .id_prefix(format!("control-{key}"))
                .accessory_width(width)
                .value_menu(
                    current,
                    self.menu.as_deref() == Some(key),
                    Message::Menu(key.into(), true),
                    Message::Menu(key.into(), false),
                    choices,
                )
                .enabled(self.controls_live())
                .into_element(),
        )
    }
}

/// Room for the longest choice, so a value such as "Same as the desktop" is
/// never cut short and the trigger does not change width when it changes.
pub(super) fn value_menu_width<'a>(labels: impl IntoIterator<Item = &'a str>) -> f32 {
    let size = enzui_theme::text_scale::effective().apply_px(13.0);
    let longest = labels
        .into_iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0);
    (longest as f32 * size * 0.56 + 36.0).clamp(160.0, 300.0)
}

// What a row does, for rows whose section is shared with others and so no
// longer explains them in its description.
fn row_note(key: &str) -> Option<&'static str> {
    Some(match key {
        "shell/material" => tr("shell-material-description"),
        "shell/chrome_color" => tr("shell-chrome-color-note"),
        "windows/focus_pointer" => tr("shell-moving-the-pointer-over-a-window-focuses-it"),
        "windows/super_key" => tr("shell-super-key-note"),
        "windows/restore_apps" => {
            tr("shell-reopens-the-apps-that-were-open-when-you-last-logged-out-or-shut-down-each-o")
        }
        "wallpaper/pause_idle" => {
            tr("shell-a-live-wallpaper-dims-and-stops-moving-after-five-minutes-without-input-to-s")
        }
        "accessibility/reduce_motion" => {
            tr("shell-turns-off-window-animations-and-asks-every-enzui-app-to-keep-still")
        }
        _ => return None,
    })
}
