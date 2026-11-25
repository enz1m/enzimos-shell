//! The enzUI polkit authentication agent: registers for this login session
//! and shows a password dialog whenever an action needs an administrator.

use enzui_security::secret::SecretText;
use enzui_shell::arm_delay::ArmGate;
use enzui_shell::i18n::{self, tr};
use enzui_shell::polkit_agent::{AuthRequest, Command, Event};
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
    Password(SecretText),
    Submit,
    Cancel,
    NextIdentity,
    Closed(window::Id),
    Focused(window::Id),
    Shown(window::Id),
    Armed,
}

struct Dialog {
    request: AuthRequest,
    window: window::Id,
    uid: u32,
    prompt: Option<(String, bool)>,
    password: SecretText,
    notice: Option<(String, bool)>,
    checking: bool,
    gate: ArmGate,
}

struct App {
    current: Option<Dialog>,
    waiting: VecDeque<AuthRequest>,
    light: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                current: None,
                waiting: VecDeque::new(),
                light: Self::appearance(),
            },
            Task::none(),
        )
    }
    fn appearance() -> bool {
        enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light)
    }
    fn tokens(&self) -> EnzTheme {
        if self.light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        }
    }
    fn open(&mut self, request: AuthRequest) -> Task<Message> {
        i18n::initialize();
        self.light = Self::appearance();
        let (window, open) = window::open(window::Settings {
            size: iced::Size::new(440.0, 330.0),
            resizable: false,
            decorations: false,
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.Authentication".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        self.current = Some(Dialog {
            uid: request.preferred,
            request,
            window,
            prompt: None,
            password: SecretText::new(String::new()),
            notice: None,
            checking: false,
            gate: ArmGate::default(),
        });
        open.map(Message::Shown)
            .chain(iced::widget::operation::focus(PASSWORD))
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        // Typing aimed at another window must not reach a dialog that just took focus.
        if matches!(
            message,
            Message::Password(_) | Message::Submit | Message::NextIdentity
        ) && !self.current.as_ref().is_some_and(|d| d.gate.armed())
        {
            return Task::none();
        }
        match message {
            Message::Shown(window) => {
                if let Some(dialog) = self.current.as_mut().filter(|d| d.window == window) {
                    dialog.gate.shown(std::time::Instant::now());
                    return ArmGate::wake(Message::Armed);
                }
            }
            Message::Armed => {}
            Message::Agent(Event::Begin(request)) => {
                if self.current.is_some() {
                    self.waiting.push_back(request);
                    return Task::none();
                }
                return self.open(request);
            }
            Message::Agent(Event::Prompt { id, text, secret }) => {
                if let Some(dialog) = self.current.as_mut().filter(|d| d.request.id == id) {
                    dialog.prompt = Some((text, secret));
                    dialog.checking = false;
                    return iced::widget::operation::focus(PASSWORD);
                }
            }
            Message::Agent(Event::Notice { id, text, error }) => {
                if let Some(dialog) = self.current.as_mut().filter(|d| d.request.id == id) {
                    dialog.notice = Some((text, error));
                    dialog.checking = false;
                }
            }
            Message::Agent(Event::Done { id }) => {
                if self.current.as_ref().is_some_and(|d| d.request.id == id) {
                    let window = self.current.take().map(|d| d.window);
                    let close = window.map_or_else(Task::none, window::close);
                    let next = match self.waiting.pop_front() {
                        Some(request) => self.open(request),
                        None => Task::none(),
                    };
                    return close.chain(next);
                }
                self.waiting.retain(|r| r.id != id);
            }
            Message::Agent(Event::Unavailable(error)) => eprintln!("enzui-polkit-agent: {error}"),
            Message::Password(text) => {
                if let Some(dialog) = self.current.as_mut() {
                    dialog.password =
                        SecretText::new(text.expose_secret().chars().take(512).collect());
                }
            }
            Message::Submit => {
                if let Some(dialog) = self.current.as_mut()
                    && dialog.prompt.is_some()
                    && !dialog.checking
                    && !dialog.password.is_empty()
                {
                    dialog.checking = true;
                    dialog.notice = None;
                    send(Command::Answer {
                        id: dialog.request.id,
                        text: std::mem::replace(
                            &mut dialog.password,
                            SecretText::new(String::new()),
                        ),
                    });
                }
            }
            Message::Cancel => {
                if let Some(dialog) = &self.current {
                    send(Command::Cancel {
                        id: dialog.request.id,
                    });
                }
            }
            Message::NextIdentity => {
                if let Some(dialog) = self.current.as_mut()
                    && dialog.request.identities.len() > 1
                    && !dialog.checking
                {
                    let people = &dialog.request.identities;
                    let at = people.iter().position(|p| p.uid == dialog.uid).unwrap_or(0);
                    dialog.uid = people[(at + 1) % people.len()].uid;
                    dialog.prompt = None;
                    dialog.password = SecretText::new(String::new());
                    dialog.notice = None;
                    send(Command::Identity {
                        id: dialog.request.id,
                        uid: dialog.uid,
                    });
                }
            }
            Message::Closed(window) => {
                if self.current.as_ref().is_some_and(|d| d.window == window) {
                    return self.update(Message::Cancel);
                }
            }
            // A click elsewhere unfocuses the field; the compositor gives the
            // dialog its focus back, and the typing should go on landing here.
            Message::Focused(window) => {
                if let Some(dialog) = self.current.as_mut().filter(|d| d.window == window) {
                    let focus = iced::widget::operation::focus(PASSWORD);
                    if dialog.gate.focused(std::time::Instant::now()) {
                        return Task::batch([focus, ArmGate::wake(Message::Armed)]);
                    }
                    return focus;
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
            let person = dialog
                .request
                .identities
                .iter()
                .find(|p| p.uid == dialog.uid);
            let name = person.map_or_else(
                || tr("shell-administrator").to_owned(),
                |p| p.display.clone(),
            );
            let initial: String = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_default();
            let avatar = container(
                enz_text(initial)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .center_x(36)
            .center_y(36)
            .style(move |_| container::Style {
                background: Some(palette.accent_soft.into()),
                border: iced::Border {
                    radius: 18.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            let mut who = row![
                avatar,
                column![
                    enz_text(name)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                    enz_text(person.map(|p| p.name.clone()).unwrap_or_default())
                        .size(TextSize::Footnote)
                        .muted_on(palette.surface)
                        .into_element()
                ]
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            if dialog.request.identities.len() > 1 {
                who = who.push(Space::new().width(Length::Fill)).push(
                    enz_button(tr("shell-switch"), Message::NextIdentity)
                        .kind(ButtonKind::Plain)
                        .enabled(!dialog.checking)
                        .into_element(),
                );
            }
            let (question, secret): (String, bool) = match &dialog.prompt {
                Some((text, secret)) => {
                    let text = text.trim_end_matches(':').trim();
                    (
                        if text.is_empty() || text.eq_ignore_ascii_case("password") {
                            enzui_shell::i18n::current().text("shell-password")
                        } else {
                            text.to_owned()
                        },
                        *secret,
                    )
                }
                None => (enzui_shell::i18n::current().text("shell-password"), true),
            };
            let mut field = enz_text_field(question, "", dialog.password.expose_secret(), |text| {
                Message::Password(SecretText::new(text))
            })
            .secure(secret)
            .on_submit(Message::Submit)
            .id(PASSWORD.into())
            .enabled(dialog.prompt.is_some() && !dialog.checking);
            if let Some((notice, true)) = &dialog.notice {
                field = field.error(notice.clone());
            } else if let Some((notice, false)) = &dialog.notice {
                field = field.description(notice.clone());
            }
            let ready = dialog.prompt.is_some() && !dialog.checking && !dialog.password.is_empty();
            let content = column![
                row![
                    enz_icon("lock-keyhole").size(22.0).into_element(),
                    enz_text(tr("shell-authentication-required"))
                        .size(TextSize::Title3)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                enz_text(if dialog.request.message.is_empty() {
                    tr("shell-an-application-wants-to-make-a-change-that-needs-an-administrator")
                        .to_owned()
                } else {
                    dialog.request.message.clone()
                })
                .muted_on(palette.surface)
                .into_element(),
                who,
                field.into_element(),
                row![
                    Space::new().width(Length::Fill),
                    enz_button(enzui_shell::i18n::tr("shell-cancel"), Message::Cancel)
                        .kind(ButtonKind::Secondary)
                        .id("authentication-cancel".into())
                        .into_element(),
                    enz_button(
                        if dialog.checking {
                            enzui_shell::i18n::tr("shell-checking")
                        } else {
                            enzui_shell::i18n::tr("shell-authenticate")
                        },
                        Message::Submit
                    )
                    .id("authentication-submit".into())
                    .enabled(ready)
                    .into_element(),
                ]
                .spacing(8),
            ]
            .spacing(14);
            // The compositor fixes the window's size; spare height goes evenly
            // above and below rather than between the field and its buttons.
            container(content)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_y(Length::Fill)
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
            iced::event::listen_with_interest(
                iced::event::Interest::KEYBOARD,
                |event, status, _| unhandled_key(&event, status),
            ),
        ])
    }
}

const PASSWORD: &str = "authentication-password";

// Only the focused password field takes text or submits; a key nothing handled can
// only cancel.
fn unhandled_key(event: &iced::Event, status: iced::event::Status) -> Option<Message> {
    match (event, status) {
        (
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }),
            iced::event::Status::Ignored,
        ) => Some(Message::Cancel),
        _ => None,
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
        let serve = enzui_shell::polkit_agent::run(events.clone(), receiver);
        tokio::select! {
            result = serve => {
                if let Err(error) = result {
                    eprintln!("enzui-polkit-agent: {error}");
                    // Without registration there is nothing to do; exit so the
                    // shell's supervisor can try again later.
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
            .title(|_: &App, _| tr("shell-authentication-required").to_owned())
            .theme(|app: &App, _| app.tokens().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_shell::arm_delay::ARM_DELAY;
    use iced::keyboard::{Key, Location, Modifiers, key};
    use std::time::Instant;

    fn prompting() -> App {
        let mut app = App {
            current: None,
            waiting: VecDeque::new(),
            light: false,
        };
        let _ = app.open(AuthRequest {
            id: 7,
            action: "org.example.change".into(),
            message: String::new(),
            identities: Vec::new(),
            preferred: 0,
        });
        let dialog = app.current.as_mut().unwrap();
        dialog.prompt = Some(("Password:".into(), true));
        app
    }

    fn press(key: Key, text: Option<&str>) -> iced::Event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::empty(),
            text: text.map(Into::into),
            repeat: false,
        })
    }

    #[test]
    fn password_takes_only_focused_input_once_armed() {
        let mut app = prompting();
        let _ = app.update(Message::Password(SecretText::new("typed-elsewhere".into())));
        app.current.as_mut().unwrap().gate.shown(Instant::now());
        let _ = app.update(Message::Password(SecretText::new("typed-elsewhere".into())));
        assert!(app.current.as_ref().unwrap().password.is_empty());

        app.current.as_mut().unwrap().gate = ArmGate::default();
        app.current
            .as_mut()
            .unwrap()
            .gate
            .shown(Instant::now().checked_sub(ARM_DELAY).unwrap());
        let unhandled = |app: &mut App, event: iced::Event| {
            if let Some(message) = unhandled_key(&event, iced::event::Status::Ignored) {
                let _ = app.update(message);
            }
        };
        unhandled(&mut app, press(Key::Character("x".into()), Some("x")));
        assert!(app.current.as_ref().unwrap().password.is_empty());

        let _ = app.update(Message::Password(SecretText::new("secret".into())));
        unhandled(&mut app, press(Key::Named(key::Named::Enter), Some("\r")));
        let dialog = app.current.as_ref().unwrap();
        assert!(dialog.password.expose_secret() == "secret" && !dialog.checking);

        let _ = app.update(Message::Submit);
        let dialog = app.current.as_ref().unwrap();
        assert!(dialog.password.is_empty() && dialog.checking);
    }
}
