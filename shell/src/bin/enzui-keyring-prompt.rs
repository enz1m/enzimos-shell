//! enzUI password dialogs for the system credential service.
use enzui_security::secret::SecretText;
use enzui_shell::{
    i18n::{self, tr},
    keyring_prompt::{Command, Event},
};
use enzui_theme::EnzTheme;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::{
    Element, Length, Subscription, Task,
    widget::{Space, column, container, row},
    window,
};
use std::{collections::VecDeque, sync::OnceLock};
use tokio::sync::mpsc;

static COMMANDS: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
fn send(command: Command) {
    if let Some(sender) = COMMANDS.get() {
        let _ = sender.try_send(command);
    }
}
#[derive(Debug, Clone)]
enum Message {
    Service(Event),
    Password(SecretText),
    Confirm(SecretText),
    Submit,
    Cancel,
    Closed(window::Id),
    Escape,
}
struct Dialog {
    id: u64,
    label: String,
    create: bool,
    window: window::Id,
    password: SecretText,
    confirm: SecretText,
    retry: bool,
    checking: bool,
}
#[derive(Default)]
struct App {
    current: Option<Dialog>,
    waiting: VecDeque<Event>,
    light: bool,
}
impl App {
    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }
    fn open(&mut self, event: Event) -> Task<Message> {
        let Event::Begin {
            id,
            label,
            create,
            activation,
        } = event
        else {
            return Task::none();
        };
        self.light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        let settings = window::Settings {
            size: iced::Size::new(460.0, if create { 370.0 } else { 300.0 }),
            resizable: false,
            decorations: false,
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.KeyringPrompt".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let (window, open) = match activation {
            Some(token) => window::open_with_activation(
                settings,
                window::ActivationToken::for_platform(
                    window::ActivationPlatform::Wayland,
                    token.expose_secret().to_owned(),
                ),
            ),
            None => window::open(settings),
        };
        self.current = Some(Dialog {
            id,
            label,
            create,
            window,
            password: SecretText::new(String::new()),
            confirm: SecretText::new(String::new()),
            retry: false,
            checking: false,
        });
        open.discard()
            .chain(iced::widget::operation::focus("keyring-password"))
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Service(event @ Event::Begin { .. }) => {
                if self.current.is_some() {
                    self.waiting.push_back(event);
                } else {
                    return self.open(event);
                }
            }
            Message::Service(Event::Retry { id }) => {
                if let Some(d) = self.current.as_mut().filter(|d| d.id == id) {
                    d.password = SecretText::new(String::new());
                    d.checking = false;
                    d.retry = true;
                    return iced::widget::operation::focus("keyring-password");
                }
            }
            Message::Service(Event::Done { id }) => {
                self.waiting
                    .retain(|e| !matches!(e, Event::Begin { id: queued, .. } if *queued == id));
                if self.current.as_ref().is_some_and(|d| d.id == id) {
                    let Some(d) = self.current.take() else {
                        return Task::none();
                    };
                    let close = window::close(d.window);
                    drop(d);
                    let next = self
                        .waiting
                        .pop_front()
                        .map_or_else(Task::none, |e| self.open(e));
                    return close.chain(next);
                }
            }
            Message::Password(password) => {
                if let Some(d) = self.current.as_mut().filter(|d| !d.checking) {
                    d.password = password;
                }
            }
            Message::Confirm(password) => {
                if let Some(d) = self.current.as_mut().filter(|d| !d.checking) {
                    d.confirm = password;
                }
            }
            Message::Submit => {
                if let Some(d) = self.current.as_mut().filter(|d| {
                    !d.checking
                        && !d.password.is_empty()
                        && d.password.expose_secret().len() <= 4096
                        && (!d.create
                            || (d.password.expose_secret().chars().count() >= 8
                                && d.password.expose_secret() == d.confirm.expose_secret()))
                }) {
                    d.checking = true;
                    let password =
                        std::mem::replace(&mut d.password, SecretText::new(String::new()));
                    d.confirm = SecretText::new(String::new());
                    send(Command::Answer { id: d.id, password });
                }
            }
            Message::Cancel | Message::Escape => {
                if let Some(d) = &self.current {
                    send(Command::Cancel { id: d.id });
                }
            }
            Message::Closed(id) => {
                if self.current.as_ref().is_some_and(|d| d.window == id) {
                    return self.update(Message::Cancel);
                }
            }
        }
        Task::none()
    }
    fn view(&self, id: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
            let Some(d) = self.current.as_ref().filter(|d| d.window == id) else {
                return Space::new().into();
            };
            let palette = self.tokens().palette;
            let mut field =
                enz_text_field(tr("shell-password"), "", d.password.expose_secret(), |s| {
                    Message::Password(SecretText::new(s))
                })
                .secure(true)
                .id("keyring-password".into())
                .on_submit(Message::Submit)
                .enabled(!d.checking);
            if d.retry {
                field = field.error(tr("shell-keyring-incorrect-password"));
            }
            let mut body = column![
                enz_text(if d.create {
                    tr("shell-keyring-create")
                } else {
                    tr("shell-keyring-unlock")
                })
                .size(TextSize::Title3)
                .into_element(),
                enz_text(d.label.clone())
                    .single_line_elided()
                    .width(Length::Fill)
                    .into_element(),
                enz_text(if d.create {
                    tr("shell-keyring-create-description")
                } else {
                    tr("shell-keyring-unlock-description")
                })
                .muted_on(palette.surface)
                .into_element(),
                field.into_element()
            ]
            .spacing(14);
            if d.create {
                body = body.push(
                    enz_text_field(
                        tr("shell-keyring-confirm-password"),
                        "",
                        d.confirm.expose_secret(),
                        |s| Message::Confirm(SecretText::new(s)),
                    )
                    .secure(true)
                    .id("keyring-confirm".into())
                    .on_submit(Message::Submit)
                    .enabled(!d.checking)
                    .into_element(),
                );
            }
            let ready = !d.checking
                && !d.password.is_empty()
                && d.password.expose_secret().len() <= 4096
                && (!d.create
                    || (d.password.expose_secret().chars().count() >= 8
                        && d.password.expose_secret() == d.confirm.expose_secret()));
            body = body.push(Space::new().height(Length::Fill)).push(
                row![
                    Space::new().width(Length::Fill),
                    enz_button(tr("shell-cancel"), Message::Cancel)
                        .kind(ButtonKind::Secondary)
                        .into_element(),
                    enz_button(
                        if d.checking {
                            tr("shell-checking")
                        } else if d.create {
                            tr("shell-keyring-create-action")
                        } else {
                            tr("shell-keyring-unlock-action")
                        },
                        Message::Submit
                    )
                    .enabled(ready)
                    .into_element()
                ]
                .spacing(8),
            );
            container(body)
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
            Subscription::run(service).map(Message::Service),
            window::close_requests().map(Message::Closed),
            iced::event::listen_with_interest(iced::event::Interest::KEYBOARD, |event, _, _| {
                matches!(
                    event,
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    })
                )
                .then_some(Message::Escape)
            }),
        ])
    }
}
fn service() -> impl iced::futures::Stream<Item = Event> {
    iced::stream::channel(16, async move |mut output| {
        use iced::futures::SinkExt;
        let (events, mut receiver) = mpsc::channel(16);
        let (commands, input) = mpsc::channel(16);
        let _ = COMMANDS.set(commands);
        let forward = async {
            while let Some(event) = receiver.recv().await {
                if output.send(event).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            result = enzui_shell::keyring_prompt::run(events, input) => { if let Err(error) = result { eprintln!("enzui-keyring-prompt: {error}"); std::process::exit(1); } }
            () = forward => {}
        }
    })
}
fn main() -> iced::Result {
    i18n::initialize();
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(|| (App::default(), Task::none()), App::update, App::view)
            .title(|_: &App, _| tr("shell-keyring-title").to_owned())
            .theme(|app: &App, _| app.tokens().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default().localizer(i18n::localizer().clone()),
    )
}
