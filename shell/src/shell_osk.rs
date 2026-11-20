use super::{Message, Shell, window_settings};
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::i18n::tr;
use enzui_shell::virtual_keyboard::Named;
use enzui_widgets::text::enz_text;
use iced::widget::{column, container, row};
use iced::{Element, Length, Task, window};

// The keys and 12px around them: at 640 by 250 the rows touched the
// panel's top and bottom edges.
const WIDTH: f32 = 644.0;
const HEIGHT: f32 = 268.0;
const LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
const SYMBOLS: [&str; 4] = ["!@#$%^&*()", "-_=+[]{}\\|", ";:'\",.<>/?", "`~€£¥§°"];

// Non-focusing keyboard overlay targeting the previously focused application.
#[derive(Clone, Debug, Default)]
pub(crate) struct Osk {
    window: Option<window::Id>,
    placing: bool,
    shift: bool,
    symbols: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum OskMessage {
    Placed(Result<ControlGeometry, String>),
    Text(String),
    Key(Named),
    Shift,
    Symbols,
    Close,
    Sent(Result<(), String>),
}

impl Osk {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }
    pub(crate) fn shown(&self) -> bool {
        self.window.is_some() || self.placing
    }
}

impl Shell {
    pub(crate) fn toggle_osk(&mut self) -> Task<Message> {
        if self.osk.shown() {
            return self.osk_message(OskMessage::Close);
        }
        self.osk.placing = true;
        let compositor = self.compositor.clone();
        let (metrics, bar) = (self.metrics, self.bar);
        Task::perform(
            async move {
                compositor
                    .prepare_overlay("dev.enzui.Keyboard", WIDTH, HEIGHT, false, metrics, bar)
                    .await
            },
            |result| Message::Osk(OskMessage::Placed(result)),
        )
    }

    pub(crate) fn osk_message(&mut self, message: OskMessage) -> Task<Message> {
        let compositor = self.compositor.clone();
        // A click on a key (or on the bar) can take focus from the app being
        // typed into, so each key first hands focus back to that app.
        let send = move |work: Box<dyn FnOnce() -> Result<(), String> + Send>| {
            Task::perform(
                async move {
                    compositor.focus_previous().await?;
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    tokio::task::spawn_blocking(work)
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-typing-stopped").into()))
                },
                |result| Message::Osk(OskMessage::Sent(result)),
            )
        };
        match message {
            OskMessage::Placed(result) => {
                if !std::mem::replace(&mut self.osk.placing, false) || self.osk.window.is_some() {
                    return Task::none();
                }
                if let Err(error) = result {
                    self.notice = Some(error);
                    return Task::none();
                }
                let (id, open) = window::open(window_settings("dev.enzui.Keyboard", WIDTH, HEIGHT));
                self.osk.window = Some(id);
                open.map(Message::Opened)
            }
            OskMessage::Text(text) => {
                // Shift applies to one key, as on a phone.
                self.osk.shift = false;
                send(Box::new(move || {
                    enzui_shell::virtual_keyboard::type_text(&text)
                }))
            }
            OskMessage::Key(key) => {
                send(Box::new(move || enzui_shell::virtual_keyboard::press(key)))
            }
            OskMessage::Shift => {
                self.osk.shift = !self.osk.shift;
                Task::none()
            }
            OskMessage::Symbols => {
                self.osk.symbols = !self.osk.symbols;
                self.osk.shift = false;
                Task::none()
            }
            OskMessage::Close => {
                self.osk.placing = false;
                self.osk.shift = false;
                self.osk
                    .window
                    .take()
                    .map_or_else(Task::none, window::close)
            }
            OskMessage::Sent(Ok(())) => Task::none(),
            OskMessage::Sent(Err(error)) => {
                self.notice = Some(error);
                Task::none()
            }
        }
    }

    pub(crate) fn osk_window(&mut self, event: &window::Event) -> Task<Message> {
        match event {
            window::Event::Closed => self.osk_message(OskMessage::Close),
            _ => Task::none(),
        }
    }

    pub(crate) fn osk_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if !self.osk.owns(id) {
            return None;
        }
        let palette = self.window_tokens(Some(id)).palette;
        let (accent, text) = (palette.accent, palette.text);
        let key = |label: String,
                   message: OskMessage,
                   width: f32,
                   active: bool,
                   name: &str|
         -> Element<'_, Message> {
            let face = container(
                enz_text(label)
                    .size_px(enzui_theme::text_scale::effective().apply_px(17.0))
                    .into_element(),
            )
            .center_x(width)
            .center_y(44)
            .style(move |_| container::Style {
                background: Some(
                    if active {
                        iced::Color { a: 0.35, ..accent }
                    } else {
                        iced::Color { a: 0.08, ..text }
                    }
                    .into(),
                ),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            enzui_widgets::clickable::enz_clickable(face, Message::Osk(message))
                .aria_label(name.to_owned())
                .id(format!("osk-{name}").into())
                .into_element()
        };
        let rows = if self.osk.symbols { SYMBOLS } else { LETTERS };
        let mut keys = column![].spacing(6).align_x(iced::Alignment::Center);
        for (index, line) in rows.iter().enumerate() {
            let mut keyrow = row![].spacing(6);
            if index == 3 && !self.osk.symbols {
                keyrow = keyrow.push(key(
                    "⇧".into(),
                    OskMessage::Shift,
                    72.0,
                    self.osk.shift,
                    enzui_shell::i18n::tr("shell-key-shift"),
                ));
            }
            for c in line.chars() {
                let label: String = if self.osk.shift {
                    c.to_uppercase().collect()
                } else {
                    c.to_string()
                };
                keyrow = keyrow.push(key(
                    label.clone(),
                    OskMessage::Text(label.clone()),
                    52.0,
                    false,
                    &label,
                ));
            }
            if index == 3 {
                keyrow = keyrow.push(key(
                    "⌫".into(),
                    OskMessage::Key(Named::Backspace),
                    72.0,
                    false,
                    enzui_shell::i18n::tr("shell-key-backspace"),
                ));
            }
            keys = keys.push(keyrow);
        }
        keys = keys.push(
            row![
                key(
                    if self.osk.symbols {
                        "abc".into()
                    } else {
                        "#+=".into()
                    },
                    OskMessage::Symbols,
                    72.0,
                    false,
                    enzui_shell::i18n::tr("shell-key-symbols")
                ),
                key(
                    "⇥".into(),
                    OskMessage::Key(Named::Tab),
                    52.0,
                    false,
                    enzui_shell::i18n::tr("shell-tab")
                ),
                key(
                    " ".into(),
                    OskMessage::Text(" ".into()),
                    232.0,
                    false,
                    enzui_shell::i18n::tr("shell-space")
                ),
                key(
                    "←".into(),
                    OskMessage::Key(Named::Left),
                    52.0,
                    false,
                    enzui_shell::i18n::tr("shell-left")
                ),
                key(
                    "→".into(),
                    OskMessage::Key(Named::Right),
                    52.0,
                    false,
                    enzui_shell::i18n::tr("shell-right")
                ),
                key(
                    "↵".into(),
                    OskMessage::Key(Named::Enter),
                    72.0,
                    false,
                    enzui_shell::i18n::tr("shell-enter")
                ),
                key(
                    "✕".into(),
                    OskMessage::Close,
                    52.0,
                    false,
                    tr("shell-hide-keyboard")
                ),
            ]
            .spacing(6),
        );
        let body = container(keys)
            .center(Length::Fill)
            .style(move |_| container::Style {
                text_color: Some(text),
                ..Default::default()
            });
        Some(self.transient_glass(id, body, 18.0))
    }
}
