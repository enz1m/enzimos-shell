use enzui_shell::i18n::tr;

use enzui_shell::tr;
#[path = "settings_preview_scene.rs"]
mod scene;

use super::{Message, SettingsApp};
use enzui_shell::{
    notifications::preferences::{Change as NotificationChange, Style},
    settings::Change,
    shell_style::ShellStyle,
};
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    enz_icon, enz_text,
    pressable::{PressableStyle, enz_pressable},
};
use iced::{
    Element, Length,
    widget::{column, container, responsive, row, svg},
};

fn css(color: iced::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8
    )
}

fn choice_caption(label: &'static str, selected: bool) -> Element<'static, Message> {
    let indicator: Element<'static, Message> = if selected {
        enz_icon("check").size(14.0).into_element()
    } else {
        iced::widget::space().width(14).height(14).into()
    };
    row![enz_text(label).into_element(), indicator]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .into()
}

impl SettingsApp {
    fn desktop_picture(
        &self,
        style: ShellStyle,
        tiled: bool,
        notifications: bool,
        height: f32,
    ) -> Element<'static, Message> {
        let image = scene::desktop(
            &self.settings,
            &self.tokens().palette,
            style,
            tiled,
            notifications,
        );
        svg(svg::Handle::from_memory(image.into_bytes()))
            .width(Length::Fill)
            .height(height)
            .into()
    }
    fn layout_preview(&self, tiled: bool) -> Element<'static, Message> {
        self.desktop_picture(self.settings.shell_style, tiled, false, 150.0)
    }
    pub(super) fn shell_style_choices(&self) -> Element<'_, Message> {
        let choices = responsive(move |size| -> Element<'_, Message> {
            let choices = [ShellStyle::Frame, ShellStyle::Islands]
                .into_iter()
                .map(|style| {
                    let description = match style {
                        ShellStyle::Frame => tr("shell-a-continuous-edge-with-joined-panels"),
                        ShellStyle::Islands => tr("shell-floating-pills-separate-or-connected"),
                    };
                    enz_pressable(
                        column![
                            self.desktop_picture(
                                style,
                                self.settings.tiling.unwrap_or(false),
                                false,
                                148.0
                            ),
                            choice_caption(style.label(), style == self.settings.shell_style),
                            enz_text(description)
                                .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                                .wrap(true)
                                .muted_on(self.tokens().palette.surface)
                                .into_element(),
                        ]
                        .spacing(8)
                        .align_x(iced::Alignment::Center)
                        .into(),
                        Message::Change(Change::ShellStyle(style)),
                    )
                    .style(PressableStyle::Card)
                    .selected(style == self.settings.shell_style)
                    .selectable(true)
                    .aria_label(tr!("shell-shell-style-2", "arg0" => style.label().to_owned()))
                    .id(format!("shell-style-{}", style.key()).into())
                    .padding(10)
                    .radius(enzui_theme::text_defaults::shape_radius(|shape| shape.card))
                    .border_width(1.0)
                    .width(Length::Fill)
                    .enabled(
                        !self.closing && (style == ShellStyle::Frame || self.islands_supported),
                    )
                    .into_element()
                })
                .collect::<Vec<_>>();
            if size.width >= 440.0 {
                row(choices).spacing(12).into()
            } else {
                column(choices).spacing(12).into()
            }
        });
        // The cards mark their own selection; a row highlight behind them shows
        // as a sliver around each card.
        self.anchor_setting("appearance/shell_style", choices)
    }
    pub(super) fn placement_preview(&self) -> Element<'static, Message> {
        self.preview_frame(self.desktop_picture(
            self.settings.shell_style,
            self.settings.tiling.unwrap_or(false),
            false,
            230.0,
        ))
    }
    pub(super) fn typography_preview(&self) -> Element<'static, Message> {
        let metrics = self.settings.effective_metrics();
        let ramp = self.tokens().typography;
        // The sample resolves each rung from the scaled ramp, so it shows the
        // heading/body/caption relationship the choice actually produces.
        let content = column![
            row![
                enz_icon("layout-grid")
                    .size(f32::from(metrics.icon))
                    .into_element(),
                enz_text(tr("shell-applications"))
                    .size_px(ramp.title_size)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
                enz_icon("bell")
                    .size(f32::from(metrics.icon))
                    .into_element(),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            enz_text(tr("shell-body-text-in-menus-panels-and-dialogs"))
                .size_px(ramp.body_size)
                .into_element(),
            enz_text(tr("shell-captions-and-secondary-labels"))
                .size_px(ramp.caption_size)
                .muted_on(self.tokens().palette.background)
                .into_element(),
        ]
        .spacing(8);
        self.preview_frame(content)
    }
    pub(super) fn window_appearance_preview(&self) -> Element<'static, Message> {
        self.preview_frame(self.desktop_picture(
            self.settings.shell_style,
            self.settings.tiling.unwrap_or(false),
            false,
            210.0,
        ))
    }
    pub(super) fn toast_placement_preview(&self) -> Element<'static, Message> {
        self.preview_frame(self.desktop_picture(
            self.settings.shell_style,
            self.settings.tiling.unwrap_or(false),
            true,
            230.0,
        ))
    }

    pub(super) fn shape_preview(&self) -> Element<'_, Message> {
        use enzui_widgets::card::{CardTone, enz_card};
        let tokens = self.tokens();
        let palette = tokens.palette;
        let row = |icon: &'static str, title: &'static str, value: &'static str| {
            crate::settings_app::settings_row(title)
                .icon(icon)
                .control(enz_text(value).into_element())
                .into_element()
        };
        let rows = enz_card(
            column![
                row("wifi", "Wi-Fi", enzui_shell::i18n::tr("place-home")),
                row("bluetooth", "Bluetooth", enzui_shell::i18n::tr("shell-on"))
            ]
            .spacing(2),
        )
        .tone(CardTone::Default)
        .padding(6)
        .into_element();
        let raised = enz_card(
            column![
                enz_text(tr("shell-raised-card"))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
                enz_text(tr("shell-menus-popovers-and-dialogs-cast-this-shadow"))
                    .wrap(true)
                    .muted_on(palette.surface_elevated)
                    .into_element(),
            ]
            .spacing(4),
        )
        .tone(CardTone::Elevated)
        .padding(14)
        .into_element();
        let width = self.theme_choices().scrollbars.widths().0;
        let line = |w: f32| {
            container(
                iced::widget::Space::new()
                    .width(Length::FillPortion((w * 10.0) as u16))
                    .height(8),
            )
            .style(move |_| container::Style {
                background: Some(palette.surface_selected.into()),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        let lines = column((0..7).map(|i| {
            row![
                line(if i % 3 == 0 { 0.9 } else { 0.6 }),
                iced::widget::Space::new().width(Length::FillPortion(3))
            ]
            .into()
        }))
        .spacing(10)
        .width(Length::Fill);
        let thumb =
            container(iced::widget::Space::new().width(width).height(46)).style(move |_| {
                container::Style {
                    background: Some(
                        iced::Color {
                            a: 0.55,
                            ..palette.text_muted
                        }
                        .into(),
                    ),
                    border: iced::Border {
                        radius: (width / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            });
        let list =
            enz_card(row![lines, column![iced::widget::Space::new().height(18), thumb]].spacing(8))
                .tone(CardTone::Default)
                .padding(12)
                .into_element();
        self.preview_frame(
            row![
                // The raised card's shadow needs room inside the frame.
                column![rows, raised]
                    .spacing(14)
                    .padding(iced::Padding::ZERO.bottom(14.0).right(6.0))
                    .width(Length::FillPortion(3)),
                container(list).width(Length::FillPortion(2)),
            ]
            .spacing(16),
        )
    }

    pub(super) fn layout_choices(&self) -> Element<'_, Message> {
        let current = self.settings.tiling.unwrap_or(false);
        let choices = [false, true].into_iter().map(|tiled| {
            let label = if tiled {
                tr("shell-tiled")
            } else {
                tr("shell-floating")
            };
            enz_pressable(
                column![
                    self.layout_preview(tiled),
                    choice_caption(label, tiled == current)
                ]
                .spacing(6)
                .align_x(iced::Alignment::Center)
                .into(),
                Message::Change(Change::Tiling(tiled)),
            )
            .style(PressableStyle::Card)
            .selected(tiled == current)
            .selectable(true)
            .aria_label(tr!("shell-window-layout-2", "arg0" => label.to_owned()))
            .id(format!("layout-choice-{tiled}-{current}").into())
            .padding(8)
            .radius(enzui_theme::text_defaults::shape_radius(|shape| shape.card))
            .border_width(1.0)
            .width(Length::Fill)
            .enabled(!self.closing)
            .into_element()
        });
        let own = self.settings.desktop_tiling.len();
        let mut column = column![
            row(choices).spacing(8),
            enz_text(tr(
                "shell-the-layout-of-every-desktop-a-desktop-switched-in-the-control-center-keeps-i"
            ))
            .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
            .wrap(true)
            .muted_on(self.tokens().palette.surface)
            .into_element(),
        ]
        .spacing(8);
        if own > 0 {
            column = column.push(
                row![
                    enz_text(if own == 1 {
                        tr("shell-1-desktop-has-its-own-layout").to_owned()
                    } else {
                        enzui_shell::tr!("shell-desktops-have-their-own-layout", "arg0" => enzui_shell::i18n::argument(&(own)))
                    })
                    .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                    .into_element(),
                    iced::widget::space().width(Length::Fill),
                    enz_button(
                        tr("shell-use-the-default-everywhere"),
                        Message::Change(Change::DesktopTilingReset)
                    )
                    .kind(ButtonKind::Quiet)
                    .enabled(!self.closing)
                    .into_element(),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        // As with the shell style: the cards mark the selection themselves.
        self.anchor_setting("windows/tiling", container(column).padding([6, 10]))
    }

    // One live example in the chosen style, drawn by the same card the
    // desktop uses.
    pub(super) fn notification_style_preview(&self) -> Element<'_, Message> {
        let p = &self.settings.notifications;
        let settings = &self.settings;
        let light = settings.light;
        let mut palette = self.tokens().palette;
        let chrome = settings.chrome.color.tint(light, palette.accent);
        let tint = chrome.unwrap_or_else(|| {
            let [r, g, b] = enzui_shell::panel_style::themed_surface(
                settings.theme_scheme.as_deref(),
                self.theme.document(),
                light,
            )
            .unwrap_or_else(|| enzui_shell::desktop_frame::color(light));
            iced::Color::from_rgb8(r, g, b)
        });
        if let Some(chrome) = chrome
            && enzui_shell::chrome_style::is_light(chrome) != light
        {
            palette = if light {
                enzui_theme::Palette::dark()
            } else {
                enzui_theme::Palette::light()
            };
        }
        let tint = iced::Color {
            a: f32::from(settings.chrome.opacity) / 100.0,
            ..tint
        };
        let metrics = settings.effective_metrics();
        let shadow = settings.panel_shadow.min(if metrics.islands.is_some() {
            enzui_shell::reveal::ISLAND_PANEL_SHADOW_MAX
        } else {
            4
        });
        let action = |label: &'static str, kind: ButtonKind| {
            enz_button(label, Message::PreviewNotification)
                .kind(kind)
                .size(ButtonSize::Sm)
                .enabled(!self.closing && !self.preview_notification_busy)
                .into_element()
        };
        let card = enzui_shell::notifications::card::view(
            enzui_shell::notifications::card::Card {
                style: p.style,
                surface: enzui_shell::notifications::card::Surface::Panel {
                    tint,
                    shadow: enzui_shell::reveal::glass_shadow(
                        shadow,
                        tint,
                        p.position.is_bottom(settings.bar),
                    ),
                    glass: false,
                },
                text: f32::from(metrics.text),
                radius: f32::from(metrics.radius),
                icon: "calendar",
                app_icon: None,
                source: tr("shell-calendar"),
                time: Some(tr("shell-now").into()),
                summary: tr("shell-design-review-in-10-minutes"),
                body: tr("shell-room-4-with-maya-jon-and-three-others-notes-are-in-the-shared-doc"),
                body_lines: 2,
                critical: false,
                unread: false,
                image: None,
                dismiss: Some(enz_icon("x").size(14.0).into_element()),
                actions: vec![vec![
                    action(tr("shell-snooze"), ButtonKind::Secondary),
                    action(tr("shell-open"), ButtonKind::Secondary),
                ]],
                height: None,
                progress: None,
            },
            &palette,
        );
        column![
            self.preview_frame(
                container(container(card).width(Length::Fixed(f32::from(p.width))))
                    .padding(enzui_shell::notifications::card::SHADOW_MARGIN)
                    .center_x(Length::Fill),
            ),
            self.choice_setting(
                "notifications/style",
                p.style.label(),
                Style::ALL
                    .into_iter()
                    .map(|style| {
                        (
                            style.label().to_owned(),
                            Message::Change(Change::Notification(NotificationChange::Style(style))),
                        )
                    })
                    .collect(),
            ),
        ]
        .spacing(8)
        .into()
    }
}
