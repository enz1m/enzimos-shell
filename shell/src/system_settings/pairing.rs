//! BlueZ pairing agent forwarding passkey and PIN questions to Settings.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const AGENT_PATH: &str = "/dev/enzui/settings/pairing";

/// A question BlueZ asks while pairing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// Both screens show this number; the user confirms they match.
    Confirm(u32),
    /// Type this number on the device's keyboard.
    ShowPasskey(u32),
    /// Type this PIN on the device.
    ShowPin(String),
    /// The device asks for a PIN chosen here (older devices).
    EnterPin,
    Authorize,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Reject,
    Pin(String),
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Accept => "Accept",
            Self::Reject => "Reject",
            Self::Pin(_) => "Pin(***)",
        })
    }
}

/// A pending question and the way to answer it; cloning shares the reply.
#[derive(Clone)]
pub struct Prompt {
    pub question: Question,
    reply: Arc<Mutex<Option<oneshot::Sender<Answer>>>>,
}
impl Prompt {
    pub fn answer(&self, answer: Answer) {
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(reply) = reply {
            let _ = reply.send(answer);
        }
    }
    /// Informational questions need no reply.
    pub fn needs_answer(&self) -> bool {
        !matches!(
            self.question,
            Question::ShowPasskey(_) | Question::ShowPin(_)
        )
    }
}
impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prompt")
            .field("question", &self.question)
            .finish()
    }
}
impl PartialEq for Prompt {
    fn eq(&self, other: &Self) -> bool {
        self.question == other.question && Arc::ptr_eq(&self.reply, &other.reply)
    }
}

struct Agent {
    ask: Box<dyn Fn(Prompt) + Send + Sync>,
    // Local adapter addresses identify reciprocal pairing requests from this computer.
    local: Vec<String>,
}
fn address_of(device: &ObjectPath<'_>) -> Option<String> {
    let name = device.as_str().rsplit('/').next()?.strip_prefix("dev_")?;
    Some(name.replace('_', ":"))
}
impl Agent {
    fn loopback(&self, device: &ObjectPath<'_>) -> bool {
        address_of(device).is_some_and(|a| self.local.iter().any(|l| l.eq_ignore_ascii_case(&a)))
    }
    async fn ask(&self, question: Question) -> Answer {
        let (tx, rx) = oneshot::channel();
        let informational = matches!(question, Question::ShowPasskey(_) | Question::ShowPin(_));
        (self.ask)(Prompt {
            question,
            reply: Arc::new(Mutex::new(Some(tx))),
        });
        if informational {
            return Answer::Accept;
        }
        tokio::time::timeout(Duration::from_secs(60), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(Answer::Reject)
    }
}

fn rejected() -> zbus::fdo::Error {
    zbus::fdo::Error::Failed("org.bluez.Error.Rejected".into())
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}
    async fn request_pin_code(&self, _device: ObjectPath<'_>) -> zbus::fdo::Result<String> {
        match self.ask(Question::EnterPin).await {
            Answer::Pin(pin) if (1..=16).contains(&pin.len()) => Ok(pin),
            _ => Err(rejected()),
        }
    }
    async fn display_pin_code(&self, _device: ObjectPath<'_>, pincode: String) {
        self.ask(Question::ShowPin(pincode)).await;
    }
    async fn request_passkey(&self, _device: ObjectPath<'_>) -> zbus::fdo::Result<u32> {
        match self.ask(Question::EnterPin).await {
            Answer::Pin(pin) => pin.parse().map_err(|_| rejected()),
            _ => Err(rejected()),
        }
    }
    async fn display_passkey(&self, _device: ObjectPath<'_>, passkey: u32, _entered: u16) {
        self.ask(Question::ShowPasskey(passkey)).await;
    }
    async fn request_confirmation(
        &self,
        device: ObjectPath<'_>,
        passkey: u32,
    ) -> zbus::fdo::Result<()> {
        if self.loopback(&device) {
            return Ok(());
        }
        match self.ask(Question::Confirm(passkey)).await {
            Answer::Accept => Ok(()),
            _ => Err(rejected()),
        }
    }
    async fn request_authorization(&self, device: ObjectPath<'_>) -> zbus::fdo::Result<()> {
        if self.loopback(&device) {
            return Ok(());
        }
        match self.ask(Question::Authorize).await {
            Answer::Accept => Ok(()),
            _ => Err(rejected()),
        }
    }
    // Pairing already authorizes services of the selected device.
    fn authorize_service(&self, _device: ObjectPath<'_>, _uuid: String) {}
    fn cancel(&self) {}
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

async fn objects(connection: &zbus::Connection) -> Result<Objects, String> {
    let manager = zbus::Proxy::new(
        connection,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await
    .map_err(|_| tr("shell-bluetooth-is-not-running"))?;
    manager
        .call("GetManagedObjects", &())
        .await
        .map_err(|_| tr("shell-bluetooth-is-not-running").into())
}

fn addresses(objects: &Objects, interface: &str) -> Vec<(OwnedObjectPath, String)> {
    objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let address = interfaces.get(interface)?.get("Address")?;
            Some((path.clone(), String::try_from(address.clone()).ok()?))
        })
        .collect()
}

async fn device_path(
    connection: &zbus::Connection,
    address: &str,
) -> Result<OwnedObjectPath, String> {
    addresses(&objects(connection).await?, "org.bluez.Device1")
        .into_iter()
        .find(|(_, a)| a.eq_ignore_ascii_case(address))
        .map(|(path, _)| path)
        .ok_or_else(|| tr("shell-that-device-is-no-longer-nearby-search-again").into())
}

fn explain(error: zbus::Error) -> String {
    let text = error.to_string();
    if text.contains("AuthenticationFailed")
        || text.contains("AuthenticationRejected")
        || text.contains("Rejected")
    {
        tr("shell-pairing-was-declined-check-the-code-and-try-again").into()
    } else if text.contains("AuthenticationTimeout") || text.contains("Timeout") {
        tr("shell-the-device-did-not-answer-put-it-in-pairing-mode-and-try-again").into()
    } else if text.contains("AlreadyExists") {
        tr("shell-this-device-is-already-paired").into()
    } else {
        tr("shell-pairing-failed-put-the-device-in-pairing-mode-and-try-again").into()
    }
}

/// Pairs, trusts and connects a device, asking `ask` whenever the device
/// needs a code confirmed, shown or entered.
pub async fn pair(
    address: &str,
    ask: impl Fn(Prompt) + Send + Sync + 'static,
) -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-cannot-reach-the-system-bus"))?;
    connection
        .object_server()
        .at(
            AGENT_PATH,
            Agent {
                ask: Box::new(ask),
                local: addresses(&objects(&connection).await?, "org.bluez.Adapter1")
                    .into_iter()
                    .map(|(_, a)| a)
                    .collect(),
            },
        )
        .await
        .map_err(|_| tr("shell-cannot-register-the-pairing-agent"))?;
    let agents = zbus::Proxy::new(
        &connection,
        "org.bluez",
        "/org/bluez",
        "org.bluez.AgentManager1",
    )
    .await
    .map_err(|_| tr("shell-bluetooth-is-not-running"))?;
    let path = ObjectPath::try_from(AGENT_PATH).expect("Valid agent path");
    agents
        .call::<_, _, ()>("RegisterAgent", &(&path, "KeyboardDisplay"))
        .await
        .map_err(|_| tr("shell-cannot-register-the-pairing-agent"))?;
    let _ = agents
        .call::<_, _, ()>("RequestDefaultAgent", &(&path,))
        .await;
    let result = async {
        let device_path = device_path(&connection, address).await?;
        let device = zbus::Proxy::new(
            &connection,
            "org.bluez",
            device_path.as_str(),
            "org.bluez.Device1",
        )
        .await
        .map_err(|_| tr("shell-bluetooth-is-not-running"))?;
        match tokio::time::timeout(
            Duration::from_secs(70),
            device.call::<_, _, ()>("Pair", &()),
        )
        .await
        {
            Err(_) => {
                return Err(tr(
                    "shell-the-device-did-not-answer-put-it-in-pairing-mode-and-try-again",
                )
                .to_owned());
            }
            Ok(Err(error)) if !error.to_string().contains("AlreadyExists") => {
                return Err(explain(error));
            }
            Ok(_) => {}
        }
        let _ = device.set_property("Trusted", true).await;
        // Connecting is best effort: some devices connect on their own.
        let _ = tokio::time::timeout(
            Duration::from_secs(15),
            device.call::<_, _, ()>("Connect", &()),
        )
        .await;
        Ok(())
    }
    .await;
    let _ = agents.call::<_, _, ()>("UnregisterAgent", &(&path,)).await;
    let _ = connection
        .object_server()
        .remove::<Agent, _>(AGENT_PATH)
        .await;
    result
}

pub fn prompt_view<'a, Message: Clone + 'a>(
    prompt: &Prompt,
    pin: &'a str,
    palette: enzui_theme::Palette,
    change_pin: impl Fn(String) -> Message + 'a,
    answer: impl Fn(Answer) -> Message,
    caption: impl FnOnce(&str) -> iced::Element<'a, Message>,
) -> iced::Element<'a, Message> {
    use enzui_widgets::{
        button::{ButtonKind, enz_button},
        text::{TextSize, enz_text},
    };
    use iced::{Element, widget::column};
    let code = |n: u32| {
        enz_text(format!("{:03} {:03}", n / 1000, n % 1000))
            .size(TextSize::Title1)
            .into_element()
    };
    let (title, body, big, actions): (
        &str,
        &str,
        Option<Element<'a, Message>>,
        Vec<Element<'a, Message>>,
    ) = match &prompt.question {
        Question::Confirm(n) => (
            tr("shell-does-this-code-match-the-one-on-your-device"),
            tr("shell-only-continue-if-both-screens-show-the-same-number"),
            Some(code(*n)),
            vec![
                enz_button(tr("shell-cancel"), answer(Answer::Reject))
                    .kind(ButtonKind::Secondary)
                    .id("pair-reject".into())
                    .into_element(),
                enz_button(tr("shell-codes-match"), answer(Answer::Accept))
                    .id("pair-confirm".into())
                    .into_element(),
            ],
        ),
        Question::ShowPasskey(n) => (
            tr("shell-type-this-code-on-your-device"),
            tr("shell-then-press-enter-on-the-device"),
            Some(code(*n)),
            vec![],
        ),
        Question::ShowPin(pin) => (
            tr("shell-type-this-pin-on-your-device"),
            tr("shell-then-press-enter-on-the-device"),
            Some(enz_text(pin.clone()).size(TextSize::Title1).into_element()),
            vec![],
        ),
        Question::EnterPin => (
            tr("shell-enter-the-device-s-pin"),
            tr("shell-often-0000-or-1234-check-the-device-s-manual"),
            Some(
                enzui_widgets::text_field::enz_text_field("PIN", "0000", pin, change_pin)
                    .on_submit(answer(Answer::Pin(pin.to_owned())))
                    .id("pair-pin".into())
                    .into_element(),
            ),
            vec![
                enz_button(tr("shell-cancel"), answer(Answer::Reject))
                    .kind(ButtonKind::Secondary)
                    .into_element(),
                enz_button(tr("shell-pair"), answer(Answer::Pin(pin.to_owned())))
                    .id("pair-pin-submit".into())
                    .into_element(),
            ],
        ),
        Question::Authorize => (
            tr("shell-allow-this-device-to-pair"),
            tr("shell-it-asked-to-pair-without-a-code"),
            None,
            vec![
                enz_button(tr("shell-deny"), answer(Answer::Reject))
                    .kind(ButtonKind::Secondary)
                    .into_element(),
                enz_button(tr("shell-allow"), answer(Answer::Accept))
                    .id("pair-allow".into())
                    .into_element(),
            ],
        ),
    };
    let mut content = column![
        enz_text(title)
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .into_element(),
        caption(body),
    ]
    .spacing(4);
    if let Some(big) = big {
        content = content.push(iced::widget::container(big).padding([8, 0]));
    }
    if !actions.is_empty() {
        content = content.push(
            iced::widget::row(
                std::iter::once(iced::widget::Space::new().width(iced::Length::Fill).into())
                    .chain(actions),
            )
            .spacing(8),
        );
    }
    iced::widget::container(content)
        .padding([14, 16])
        .style(move |_| iced::widget::container::Style {
            background: Some(palette.accent_soft.into()),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: palette.accent,
            },
            ..Default::default()
        })
        .into()
}
