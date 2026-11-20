use super::*;
use iced::widget::{column, row, stack};

impl Ui {
    pub(super) fn on_screen_keyboard<'a>(
        &'a self,
        alpha: f32,
        size: OskSize,
    ) -> Element<'a, Message> {
        let key = |label: String, message: Option<Message>, width: f32, active: bool| {
            key(
                label,
                if self.keyboard { message } else { None },
                width * size.width,
                size.height,
                alpha,
                active,
            )
        };
        const LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
        const SYMBOLS: [&str; 4] = ["!@#$%^&*()", "-_=+[]{}\\|", ";:'\",.<>/?", "`~€£¥§°"];
        let rows = if self.symbols { SYMBOLS } else { LETTERS };
        let gap = 6.0 * size.width;
        let mut keys = column![].spacing(gap).align_x(Alignment::Center);
        for (index, line) in rows.iter().enumerate() {
            let mut row = row![].spacing(gap);
            if index == 3 {
                row = row.push(key(
                    if self.symbols {
                        "abc".into()
                    } else {
                        "⇧".into()
                    },
                    Some(Message::Osk(if self.symbols {
                        Osk::Symbols
                    } else {
                        Osk::Shift
                    })),
                    63.0,
                    self.shift,
                ));
            }
            for c in line.chars() {
                let label: String = if self.shift {
                    c.to_uppercase().collect()
                } else {
                    c.to_string()
                };
                row = row.push(key(
                    label.clone(),
                    Some(Message::Osk(Osk::Text(label))),
                    40.0,
                    false,
                ));
            }
            if index == 3 {
                row = row.push(key(
                    "⌫".into(),
                    Some(Message::Osk(Osk::Backspace)),
                    63.0,
                    false,
                ));
            }
            keys = keys.push(row);
        }
        keys = keys.push(
            row![
                key(
                    if self.symbols {
                        "ABC".into()
                    } else {
                        "123".into()
                    },
                    Some(Message::Osk(Osk::Symbols)),
                    64.0,
                    true
                ),
                key(
                    enzui_shell::i18n::tr("shell-space").into(),
                    Some(Message::Osk(Osk::Text(" ".into()))),
                    314.0,
                    false
                ),
                key(
                    enzui_shell::i18n::tr("shell-enter").into(),
                    Some(Message::Submit),
                    64.0,
                    true
                ),
            ]
            .spacing(gap),
        );
        frosted(
            self.clock.material(),
            container(keys).padding(10.0 * size.width),
            14.0,
            alpha,
        )
    }
}

#[derive(Clone, Copy)]
pub(super) struct OskSize {
    pub(super) width: f32,
    pub(super) height: f32,
}

impl OskSize {
    pub(super) fn fitting(output: iced::Size) -> Self {
        let width = ((output.width - 48.0) / 474.0)
            .min((output.height - 200.0) / 244.0)
            .clamp(0.4, 1.2);
        Self {
            width,
            height: 40.0 * width,
        }
    }
}

pub(super) fn key<'a>(
    label: String,
    message: Option<Message>,
    width: f32,
    height: f32,
    alpha: f32,
    active: bool,
) -> Element<'a, Message> {
    button(
        container(
            text(label)
                .size((height * 0.39).clamp(10.0, 20.0))
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.95 * alpha)),
        )
        .center_x(width)
        .center_y(height),
    )
    .padding(0)
    .on_press_maybe(message)
    .style(move |_, status| button::Style {
        background: Some(
            Color::from_rgba(
                1.0,
                1.0,
                1.0,
                match (status, active) {
                    (button::Status::Pressed, _) => 0.36,
                    (_, true) | (button::Status::Hovered, _) => 0.26,
                    _ => 0.14,
                } * alpha,
            )
            .into(),
        ),
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

pub(super) fn placeholder(question: &str) -> String {
    let question = question.trim().trim_end_matches(':').trim();
    if question.is_empty() || question.eq_ignore_ascii_case("password") {
        tr("shell-enter-password").into()
    } else {
        question.to_owned()
    }
}

pub(super) fn ring(focused: bool, radius: f32, alpha: f32) -> Border {
    if focused {
        Border {
            radius: radius.into(),
            width: 2.0,
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.95 * alpha),
        }
    } else {
        Border {
            radius: radius.into(),
            ..Default::default()
        }
    }
}

pub(super) fn frosted<'a>(
    material: LockMaterial,
    content: impl Into<Element<'a, Message>>,
    radius: f32,
    alpha: f32,
) -> Element<'a, Message> {
    use enzui_theme::{GlassInk, MaterialStyle};
    if material == LockMaterial::Simple {
        return container(content)
            .style(move |theme| {
                let accent = enzui_widgets::theme_lookup::palette(theme).accent;
                container::Style {
                    background: Some(
                        Color::from_rgba(
                            0.075 + accent.r * 0.12,
                            0.085 + accent.g * 0.12,
                            0.11 + accent.b * 0.12,
                            alpha,
                        )
                        .into(),
                    ),
                    border: Border {
                        radius: radius.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into();
    }
    let mut tokens = MaterialStyle::Frosted.tokens();
    tokens.blur_sigma = if material == LockMaterial::Glass {
        0.0
    } else {
        9.0 * alpha
    };
    tokens.refraction = 0.05 * alpha;
    tokens.bevel_width = 4.0 * alpha;
    tokens.highlight = 0.22 * alpha;
    tokens.brightness = 0.0;
    tokens.contrast_protection = 0.55 * alpha;
    tokens.tint_opacity = if material == LockMaterial::Glass {
        0.07 * alpha
    } else {
        0.12 * alpha
    };
    enzui_widgets::glass::enz_glass(content)
        .style(MaterialStyle::Frosted)
        .ink(GlassInk::Light)
        .material_tokens(tokens)
        .radius(radius)
        .elevation(0)
        .into()
}

pub(super) fn chevron<'a>(
    material: LockMaterial,
    glyph: &'a str,
    message: Message,
    alpha: f32,
    focused: bool,
) -> Element<'a, Message> {
    frosted(
        material,
        button(
            container(icon(glyph, 18.0, alpha))
                .center_x(32)
                .center_y(32),
        )
        .padding(0)
        .on_press(message)
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.16 * alpha).into()),
            border: ring(focused, 16.0, alpha),
            ..Default::default()
        }),
        16.0,
        alpha,
    )
}

pub(super) fn chip<'a>(
    material: LockMaterial,
    label: String,
    message: Option<Message>,
    alpha: f32,
    focused: bool,
) -> Element<'a, Message> {
    frosted(
        material,
        button(
            text(label)
                .size(13)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.92 * alpha)),
        )
        .padding([7, 14])
        .on_press_maybe(message)
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.10 * alpha).into()),
            border: ring(focused, 15.0, alpha),
            text_color: Color::from_rgba(1.0, 1.0, 1.0, 0.92 * alpha),
            ..Default::default()
        }),
        15.0,
        alpha,
    )
}

pub(super) fn icon<'a>(name: &str, size: f32, alpha: f32) -> Element<'a, Message> {
    enzui_widgets::icon::enz_icon(name)
        .size(size)
        .tint(Color::from_rgba(1.0, 1.0, 1.0, alpha))
        .into_element()
}

pub(super) fn icon_button<'a>(
    material: LockMaterial,
    name: &str,
    message: Message,
    alpha: f32,
    focused: bool,
) -> Element<'a, Message> {
    frosted(
        material,
        button(container(icon(name, 20.0, alpha)).center_x(40).center_y(40))
            .padding(0)
            .on_press(message)
            .style(move |_, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.10 * alpha).into()),
                border: ring(focused, 20.0, alpha),
                ..Default::default()
            }),
        20.0,
        alpha,
    )
}

pub(super) fn icon_chip<'a>(
    material: LockMaterial,
    name: &str,
    label: &str,
    message: Message,
    alpha: f32,
    focused: bool,
    trailing: bool,
) -> Element<'a, Message> {
    let label =
        text(label.to_owned())
            .size(13)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.92 * alpha));
    let content = if trailing {
        row![label, icon(name, 14.0, alpha)]
    } else {
        row![icon(name, 16.0, alpha), label]
    };
    frosted(
        material,
        button(content.spacing(8).align_y(Alignment::Center))
            .padding([9, 14])
            .on_press(message)
            .style(move |_, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.10 * alpha).into()),
                border: ring(focused, 18.0, alpha),
                ..Default::default()
            }),
        18.0,
        alpha,
    )
}

impl Ui {
    pub(super) fn power_popup(&self, content: f32, size: iced::Size) -> Element<'_, Message> {
        let progress = ease_out(self.power_reveal);
        let alpha = content * progress;
        let option = |name: &str, title: &str, target: Target, destructive: bool| {
            let ink = if destructive {
                Color::from_rgba(1.0, 0.64, 0.62, alpha)
            } else {
                Color::from_rgba(0.96, 0.96, 0.99, alpha)
            };
            let glyph = enzui_widgets::icon::enz_icon(name)
                .size(18.0)
                .tint(ink)
                .into_element::<Message>();
            button(
                row![
                    container(glyph).center_x(28).center_y(28),
                    text(title.to_owned()).size(14).color(ink),
                    Space::new().width(Length::Fill)
                ]
                .spacing(9)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([8, 10])
            .on_press_maybe(self.power_menu.then(|| target.message()))
            .style(move |_, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| Color::from_rgba(1.0, 1.0, 1.0, 0.09 * alpha).into()),
                border: ring(self.focus_visible && self.target == target, 12.0, alpha),
                ..Default::default()
            })
        };
        let divider = container(Space::new().height(1).width(Length::Fill)).style(move |_| {
            container::Style {
                background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.10 * alpha).into()),
                ..Default::default()
            }
        });
        let card = frosted(
            self.clock.material(),
            container(
                column![
                    option(
                        "moon",
                        enzui_shell::i18n::tr("shell-sleep"),
                        Target::Sleep,
                        false
                    ),
                    option(
                        "rotate-cw",
                        enzui_shell::i18n::tr("shell-restart"),
                        Target::Restart,
                        false
                    ),
                    container(divider).padding([5, 10]),
                    option("power", tr("shell-shut-down-3"), Target::PowerOff, true),
                ]
                .spacing(2),
            )
            .width((size.width - 48.0).min(248.0))
            .padding(10)
            .style(move |_| container::Style {
                background: None,
                border: Border {
                    radius: 22.0.into(),
                    width: 1.0,
                    color: Color::from_rgba(0.9, 0.92, 1.0, 0.20 * alpha),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.38 * alpha),
                    offset: Vector::new(0.0, 10.0),
                    blur_radius: 36.0,
                },
                ..Default::default()
            }),
            22.0,
            alpha,
        );
        let outside: Element<'_, Message> = if self.power_menu {
            iced::widget::mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::TogglePower)
                .into()
        } else {
            Space::new().width(Length::Fill).height(Length::Fill).into()
        };
        stack![
            outside,
            container(iced::widget::mouse_area(card).on_press(Message::Noop))
                .padding(iced::Padding::new(24.0).bottom(76.0 - 10.0 * (1.0 - progress)))
                .align_right(Length::Fill)
                .align_bottom(Length::Fill)
        ]
        .into()
    }
}
