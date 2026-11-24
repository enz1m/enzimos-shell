//! NetworkManager secret-agent dialog; see enzui_shell::network_secrets.

use enzui_security::secret::SecretText;
use enzui_shell::i18n::{self, tr};
use enzui_shell::network_secrets::{Ask, Command, Event, Kind};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    icon::enz_icon,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Subscription, Task, window};
use std::collections::VecDeque;
use std::sync::OnceLock;
use tokio::sync::mpsc;

static COMMANDS: OnceLock<mpsc::UnboundedSender<Command>> = OnceLock::new();

fn send(command: Command) {
    if let Some(commands) = COMMANDS.get() {
        let _ = commands.send(command);
    }
}

#[derive(Clone, Debug)]
enum Message {
    Agent(Event),
    Value(usize, SecretText),
    Reveal(usize),
    Submit,
    Cancel,
    Closed(window::Id),
    Focused(window::Id),
    Escape,
}

struct Dialog {
    ask: Ask,
    window: window::Id,
    values: Vec<SecretText>,
    revealed: Vec<bool>,
    sent: bool,
    // The first field is focused once; later window focus keeps the user's field.
    focused: bool,
}

struct App {
    current: Option<Dialog>,
    waiting: VecDeque<Ask>,
    light: bool,
}

fn field_id(index: usize) -> String {
    format!("network-secret-{index}")
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        (
            Self {
                current: None,
                waiting: VecDeque::new(),
                light,
            },
            Task::none(),
        )
    }
    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }
    fn open(&mut self, ask: Ask) -> Task<Message> {
        let height =
            230.0 + 76.0 * ask.fields.len() as f32 + if ask.message.is_some() { 24.0 } else { 0.0 };
        let (window, open) = window::open(window::Settings {
            size: iced::Size::new(440.0, height.min(560.0)),
            resizable: false,
            decorations: false,
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.NetworkSecrets".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let count = ask.fields.len();
        self.current = Some(Dialog {
            ask,
            window,
            values: (0..count).map(|_| SecretText::new(String::new())).collect(),
            revealed: vec![false; count],
            sent: false,
            focused: false,
        });
        open.discard()
            .chain(iced::widget::operation::focus(field_id(0)))
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Agent(Event::Ask(ask)) => {
                if self.current.is_some() {
                    self.waiting.push_back(ask);
                    return Task::none();
                }
                return self.open(ask);
            }
            Message::Agent(Event::Done { id }) => {
                if self.current.as_ref().is_some_and(|d| d.ask.id == id) {
                    // Typed secrets leave memory with the dialog.
                    let window = self.current.take().map(|d| d.window);
                    let close = window.map_or_else(Task::none, window::close);
                    let next = match self.waiting.pop_front() {
                        Some(ask) => self.open(ask),
                        None => Task::none(),
                    };
                    return close.chain(next);
                }
                self.waiting.retain(|ask| ask.id != id);
            }
            Message::Agent(Event::Unavailable(error)) => {
                eprintln!("enzui-network-secrets: {error}");
            }
            Message::Value(index, text) => {
                if let Some(value) = self.current.as_mut().and_then(|d| d.values.get_mut(index)) {
                    *value = SecretText::new(
                        text.expose_secret()
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(1024)
                            .collect(),
                    );
                }
            }
            Message::Reveal(index) => {
                if let Some(revealed) = self
                    .current
                    .as_mut()
                    .and_then(|d| d.revealed.get_mut(index))
                {
                    *revealed = !*revealed;
                }
            }
            Message::Submit => {
                if let Some(dialog) = self.current.as_mut()
                    && !dialog.sent
                    && dialog.values.iter().all(|v| !v.is_empty())
                {
                    dialog.sent = true;
                    let values =
                        dialog
                            .ask
                            .fields
                            .iter()
                            .map(|f| f.key.clone())
                            .zip(dialog.values.iter_mut().map(|value| {
                                std::mem::replace(value, SecretText::new(String::new()))
                            }))
                            .collect();
                    send(Command::Answer {
                        id: dialog.ask.id,
                        values,
                    });
                }
            }
            Message::Cancel | Message::Escape => {
                if let Some(dialog) = &self.current {
                    send(Command::Cancel { id: dialog.ask.id });
                }
            }
            Message::Closed(window) => {
                if self.current.as_ref().is_some_and(|d| d.window == window) {
                    return self.update(Message::Cancel);
                }
            }
            Message::Focused(window) => {
                if let Some(dialog) = self
                    .current
                    .as_mut()
                    .filter(|d| d.window == window && !d.focused)
                {
                    dialog.focused = true;
                    return iced::widget::operation::focus(field_id(0));
                }
            }
        }
        Task::none()
    }
    fn view(&self, window: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            let Some(dialog) = self.current.as_ref().filter(|d| d.window == window) else {
                return Space::new().into();
            };
            let palette = self.tokens().palette;
            let ask = &dialog.ask;
            let (icon, title) = match ask.kind {
                Kind::Vpn => (
                    "shield",
                    tr!("shell-sign-in-to-2", "arg0" => ask.connection.clone()),
                ),
                Kind::Wifi => (
                    "wifi",
                    tr!("shell-password-for", "arg0" => ask.connection.clone()),
                ),
                Kind::Enterprise => (
                    "wifi",
                    tr!("shell-sign-in-to-2", "arg0" => ask.connection.clone()),
                ),
                Kind::Other => (
                    "lock-keyhole",
                    tr!("shell-needs-a-secret", "arg0" => ask.connection.clone()),
                ),
            };
            let explanation = match (ask.kind, ask.retry) {
                (_, true) => tr("shell-that-was-not-accepted-check-it-and-try-again").to_owned(),
                (Kind::Vpn, false) => {
                    tr("shell-the-vpn-asks-for-this-each-time-it-connects-it-is-not-saved")
                        .to_owned()
                }
                _ => tr("shell-the-network-needs-this-to-connect").to_owned(),
            };
            let mut content = column![
                row![
                    enz_icon(icon).size(22.0).into_element(),
                    enz_text(title)
                        .size(TextSize::Title3)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element()
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                enz_text(explanation)
                    .muted_on(palette.surface)
                    .into_element(),
            ]
            .spacing(14);
            if let Some(message) = &ask.message {
                content = content.push(
                    enz_text(message.clone())
                        .size(TextSize::Footnote)
                        .into_element(),
                );
            }
            let last = ask.fields.len().saturating_sub(1);
            for (index, field) in ask.fields.iter().enumerate() {
                let revealed = dialog.revealed.get(index).copied().unwrap_or(false);
                let input = enz_text_field(
                    &field.label,
                    "",
                    dialog.values[index].expose_secret(),
                    move |text| Message::Value(index, SecretText::new(text)),
                )
                .secure(field.secret && !revealed)
                .id(field_id(index).into())
                .enabled(!dialog.sent);
                let input = if index == last {
                    input.on_submit(Message::Submit)
                } else {
                    input
                };
                let mut line = row![container(input.into_element()).width(Length::Fill)]
                    .spacing(8)
                    .align_y(Alignment::End);
                if field.secret {
                    line = line.push(
                        enz_button(
                            if revealed {
                                tr("shell-hide")
                            } else {
                                tr("shell-show")
                            },
                            Message::Reveal(index),
                        )
                        .kind(ButtonKind::Plain)
                        .into_element(),
                    );
                }
                content = content.push(line);
            }
            let ready = !dialog.sent && dialog.values.iter().all(|v| !v.is_empty());
            content = content.push(Space::new().height(Length::Fill)).push(
                row![
                    Space::new().width(Length::Fill),
                    enz_button(enzui_shell::i18n::tr("shell-cancel"), Message::Cancel)
                        .kind(ButtonKind::Secondary)
                        .into_element(),
                    enz_button(
                        if dialog.sent {
                            enzui_shell::i18n::tr("shell-connecting")
                        } else {
                            enzui_shell::i18n::tr("shell-connect")
                        },
                        Message::Submit
                    )
                    .kind(ButtonKind::Accent)
                    .enabled(ready)
                    .into_element(),
                ]
                .spacing(8),
            );
            container(content)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(palette.surface.into()),
                    ..Default::default()
                })
                .into()
        })
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(agent).map(Message::Agent),
            window::close_requests().map(Message::Closed),
            window::events().filter_map(|(id, event)| {
                matches!(event, window::Event::Focused).then_some(Message::Focused(id))
            }),
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::Escape),
                _ => None,
            }),
        ])
    }
}

fn agent() -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(16, async move |mut output| {
        use iced::futures::SinkExt;
        let (events, mut incoming) = mpsc::unbounded_channel();
        let (commands, receiver) = mpsc::unbounded_channel();
        let _ = COMMANDS.set(commands);
        let forward = async {
            while let Some(event) = incoming.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
        };
        let serve = enzui_shell::network_secrets::run(events.clone(), receiver);
        tokio::select! {
            result = serve => {
                if let Err(error) = result {
                    eprintln!("enzui-network-secrets: {error}");
                    std::process::exit(1);
                }
            }
            () = forward => {}
        }
    })
}

fn main() -> iced::Result {
    i18n::initialize();
    enzui_shell::polkit_agent::exit_with_parent();
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(App::new, App::update, App::view)
            .title(|_: &App, _| tr("shell-network-sign-in").to_owned())
            .theme(|app: &App, _| app.tokens().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
