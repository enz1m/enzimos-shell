use super::{CaptureMessage, Kind, Message, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{
    clickable::enz_clickable_with_status,
    text::{TextSize, enz_text},
    toolbar_icon::enz_toolbar_icon,
    view_switcher::{SwitcherPage, enz_view_switcher},
};
use iced::widget::{Space, column, container, mouse_area, row, stack};
use iced::{Color, Element, Length, window};

impl Shell {
    pub(crate) fn capture_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if self.capture.window != Some(id) {
            return None;
        }
        let frame = self.capture.frame.as_ref()?;
        let palette = self.tokens().palette;
        let accent = palette.accent;
        let picture = if self.capture.pointer {
            frame
                .with_pointer
                .clone()
                .unwrap_or_else(|| frame.plain.clone())
        } else {
            frame.plain.clone()
        };
        // The plain image: inside the framework image's clipped placeholder
        // frame the frozen desktop did not draw (the selector showed a flat
        // fill), while this draws it on the first frame after the upload.
        let background: Element<'_, Message> =
            enzui_widgets::semantics_container::SemanticsContainer::leaf(
                iced::widget::image(picture)
                    .content_fit(iced::ContentFit::Fill)
                    .width(Length::Fill)
                    .height(Length::Fill),
                enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::Image)
                    .label(tr("shell-the-frozen-screen")),
            )
            .into();
        let shade = || {
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| container::Style {
                    background: Some(
                        Color {
                            a: 0.45,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..Default::default()
                })
        };
        let fixed = |width: f32, height: f32| {
            container(Space::new())
                .width(Length::Fixed(width))
                .height(Length::Fixed(height))
                .style(|_| container::Style {
                    background: Some(
                        Color {
                            a: 0.45,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..Default::default()
                })
        };
        let mask: Element<'_, Message> = match self.capture.selection() {
            Some(area) if self.capture.kind != Kind::Screen => {
                let area = area.clamped(frame.size.0, frame.size.1);
                let hole = container(Space::new())
                    .width(Length::Fixed(area.width))
                    .height(Length::Fixed(area.height))
                    .style(move |_| container::Style {
                        border: iced::Border {
                            width: 2.0,
                            color: accent,
                            radius: 0.0.into(),
                        },
                        ..Default::default()
                    });
                column![
                    fixed(frame.size.0, area.y),
                    row![
                        fixed(area.x, area.height),
                        hole,
                        shade().height(Length::Fixed(area.height))
                    ],
                    shade()
                ]
                .into()
            }
            // The whole screen: nothing to mark.
            Some(_) => Space::new().width(Length::Fill).height(Length::Fill).into(),
            None => shade().into(),
        };
        // Corner handles and a size readout for an area: the selection is
        // adjusted by dragging, and its size is what a capture is judged by.
        let marks: Element<'_, Message> = match self.capture.selection() {
            Some(area) if self.capture.kind == Kind::Area => {
                let area = area.clamped(frame.size.0, frame.size.1);
                const HANDLE: f32 = 10.0;
                let handle = |x: f32, y: f32| -> Element<'_, Message> {
                    iced::widget::pin(
                        container(Space::new())
                            .width(Length::Fixed(HANDLE))
                            .height(Length::Fixed(HANDLE))
                            .style(move |_| container::Style {
                                background: Some(Color::WHITE.into()),
                                border: iced::Border {
                                    width: 2.0,
                                    color: accent,
                                    radius: 2.0.into(),
                                },
                                ..Default::default()
                            }),
                    )
                    .x(x - HANDLE / 2.0)
                    .y(y - HANDLE / 2.0)
                    .into()
                };
                let (left, top) = (area.x, area.y);
                let (right, bottom) = (area.x + area.width, area.y + area.height);
                let label = format!("{} × {}", area.width.round(), area.height.round());
                // Below the selection, or above it when it reaches the bottom.
                let label_y = if bottom + 34.0 < frame.size.1 {
                    bottom + 8.0
                } else {
                    (top - 30.0).max(8.0)
                };
                let size = iced::widget::pin(
                    container(
                        enz_text(label)
                            .size(TextSize::Footnote)
                            .weight_rung(enzui_theme::WeightRung::Medium)
                            .color(Color::WHITE)
                            .into_element(),
                    )
                    .padding([3, 8])
                    // The selection's own accent: a dark pill vanished on the
                    // dimmed dark desktop.
                    .style(move |_| container::Style {
                        background: Some(accent.into()),
                        border: iced::Border {
                            radius: 999.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                )
                .x((right - 96.0).max(left))
                .y(label_y);
                stack![
                    handle(left, top),
                    handle(right, top),
                    handle(left, bottom),
                    handle(right, bottom),
                    size,
                ]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            }
            _ => Space::new().width(Length::Fill).height(Length::Fill).into(),
        };
        let surface = mouse_area(stack![background, mask, marks])
            .on_move(|point| Message::Capture(CaptureMessage::Moved(point)))
            .on_press(Message::Capture(CaptureMessage::Pressed))
            .on_release(Message::Capture(CaptureMessage::Released))
            .interaction(self.capture.interaction());
        let hint = match (self.capture.kind, self.capture.record) {
            (Kind::Area, _) => {
                tr("shell-drag-to-choose-an-area-or-move-it-with-the-arrow-keys").to_owned()
            }
            (Kind::Window, _) => match self
                .capture
                .selected
                .and_then(|index| self.capture.windows.get(index))
            {
                Some(target) => {
                    tr!("shell-click-another-window-or-use-the-arrow-keys", "arg0" => target.title.clone())
                }
                None => tr("shell-there-are-no-windows-on-this-desktop").to_owned(),
            },
            (Kind::Screen, false) => tr("shell-the-whole-screen").to_owned(),
            (Kind::Screen, true) => tr("shell-the-whole-screen-is-recorded").to_owned(),
        };
        let pill = |content: Element<'static, Message>| {
            container(content)
                .padding([6, 14])
                .style(move |_| container::Style {
                    background: Some(palette.surface.into()),
                    border: iced::Border {
                        radius: 999.0.into(),
                        ..Default::default()
                    },
                    text_color: Some(palette.text),
                    ..Default::default()
                })
        };
        let recorded = if self.capture.record {
            tr("shell-the-pointer-is-recorded")
        } else {
            ""
        };
        let hint = pill(enz_text(tr!("shell-enter-captures-esc-closes", "arg0" => hint, "arg1" => recorded.to_owned())).size(TextSize::Footnote).into_element());
        let modes = enz_view_switcher(
            vec![
                SwitcherPage::new("screenshot", enzui_shell::i18n::tr("shell-screenshot"))
                    .with_icon("camera"),
                SwitcherPage::new("record", enzui_shell::i18n::tr("shell-record"))
                    .with_icon("video"),
            ],
            usize::from(self.capture.record),
            |index| Message::Capture(CaptureMessage::Record(index == 1)),
        )
        .aria_label(tr("shell-screenshot-or-recording"))
        // A soft tinted selection: the shutter stays the only solid accent.
        .selection_indicator(enzui_widgets::view_switcher::TabSelectionStyle::TranslucentAccent)
        .into_element();
        let kinds = enz_view_switcher(
            vec![
                SwitcherPage::new("area", enzui_shell::i18n::tr("shell-area"))
                    .with_icon("square-dashed"),
                SwitcherPage::new("screen", enzui_shell::i18n::tr("shell-screen"))
                    .with_icon("monitor"),
                SwitcherPage::new("window", enzui_shell::i18n::tr("shell-window"))
                    .with_icon("app-window"),
            ],
            match self.capture.kind {
                Kind::Area => 0,
                Kind::Screen => 1,
                Kind::Window => 2,
            },
            |index| {
                Message::Capture(CaptureMessage::Kind(
                    [Kind::Area, Kind::Screen, Kind::Window][index.min(2)],
                ))
            },
        )
        .aria_label(tr("shell-what-to-capture"))
        // A soft tinted selection: the shutter stays the only solid accent.
        .selection_indicator(enzui_widgets::view_switcher::TabSelectionStyle::TranslucentAccent)
        .into_element();
        // wf-recorder always records the pointer, so there is no choice to offer.
        let pointer: Element<'_, Message> = if self.capture.record {
            Space::new().into()
        } else {
            enz_toolbar_icon(
                "mouse-pointer-2",
                Message::Capture(CaptureMessage::Pointer(!self.capture.pointer)),
            )
            .active(self.capture.pointer)
            .enabled(frame.with_pointer.is_some())
            .aria_label(tr("shell-show-the-pointer"))
            .tooltip(tr("shell-show-the-pointer"))
            .into_element()
        };
        let record = self.capture.record;
        // Hovering and pressing tint the button itself rather than a square
        // behind it; the clickable reports its own state to the disc.
        // A white disc in its ink-coloured ring in both modes: the ink-coloured
        // disc was a dark "stop" button in light mode.
        let base = if record { palette.danger } else { Color::WHITE };
        let tokens = enzui_widgets::clickable::ClickableTokens {
            hover_overlay: Color::TRANSPARENT,
            pressed_overlay: Color::TRANSPARENT,
            ..enzui_widgets::clickable::ClickableTokens::from_palette(&palette)
        };
        let shutter = enz_clickable_with_status(
            move |status| {
                container(
                    container(Space::new())
                        .width(Length::Fixed(if record { 26.0 } else { 44.0 }))
                        .height(Length::Fixed(if record { 26.0 } else { 44.0 }))
                        .style(move |_| container::Style {
                            background: Some(
                                match status.get() {
                                    enzui_widgets::focus_button::FocusStatus::Pressed => {
                                        Color { a: 0.64, ..base }
                                    }
                                    enzui_widgets::focus_button::FocusStatus::Hovered => {
                                        Color { a: 0.80, ..base }
                                    }
                                    _ => base,
                                }
                                .into(),
                            ),
                            border: iced::Border {
                                radius: if record { 13.0 } else { 22.0 }.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                )
                .center_x(Length::Fixed(56.0))
                .center_y(Length::Fixed(56.0))
                .style(move |_| container::Style {
                    border: iced::Border {
                        width: 3.0,
                        color: palette.text,
                        radius: 28.0.into(),
                    },
                    ..Default::default()
                })
                .into()
            },
            Message::Capture(CaptureMessage::Take),
        )
        .tokens(tokens)
        // The keyboard focus ring follows the round button.
        .radius(28.0)
        .aria_label(if record {
            tr("shell-start-recording")
        } else {
            tr("shell-take-screenshot")
        })
        .id("capture-take".into())
        .into_element();
        let settings = enz_toolbar_icon("settings", Message::Capture(CaptureMessage::Settings))
            .aria_label(tr("shell-screenshot-and-recording-settings"))
            .tooltip(tr("shell-settings"))
            .into_element();
        let close = enz_toolbar_icon("x", Message::Capture(CaptureMessage::Close))
            .aria_label(tr("shell-close"))
            .tooltip(tr("shell-close"))
            .into_element();
        let toolbar = container(
            row![modes, kinds, pointer, shutter, settings, close]
                .spacing(14)
                .align_y(iced::Alignment::Center),
        )
        .padding([10, 18])
        .style(move |_| container::Style {
            background: Some(palette.surface.into()),
            border: iced::Border {
                radius: 24.0.into(),
                width: 1.0,
                color: palette.border,
            },
            text_color: Some(palette.text),
            ..Default::default()
        });
        // Presses on the toolbar's own padding are not area drags.
        let controls = mouse_area(
            column![hint, toolbar]
                .spacing(10)
                .align_x(iced::Alignment::Center),
        )
        .on_press(Message::Capture(CaptureMessage::Ignore));
        // The toolbar clears what sits on the bottom edge, the bar or a dock,
        // rather than lying half across it.
        let mut bottom = 32.0_f32;
        if self.bar == enzui_shell::settings::BarEdge::Bottom {
            bottom = bottom.max(f32::from(self.metrics.bar_band()) + 12.0);
        }
        let dock = &self.dock.preferences;
        if self.dock_active() && dock.position == enzui_shell::dock::Position::Bottom {
            bottom = bottom.max(f32::from(dock.margin) + f32::from(dock.icon_size) + 34.0);
        }
        let chrome = container(controls)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding(iced::Padding::new(32.0).bottom(bottom));
        Some(stack![surface, chrome].into())
    }
}
