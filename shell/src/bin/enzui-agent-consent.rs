//! MCP Center consent dialog; unanswered requests remain subject to the broker's deny timeout.

use enzui_shell::agent_center::{self, Decision, Pending};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    badge::{BadgeTone, enz_badge},
    button::{ButtonKind, enz_button},
    icon::enz_icon,
    text::{TextSize, enz_text},
};
use iced::widget::{Space, column, container, row};
use iced::{Alignment, Element, Length, Subscription, Task, window};
use std::time::Duration;

#[derive(Clone, Debug)]
enum Message {
    Pending(Vec<Pending>),
    Decide(Decision),
    Decided(String, Result<(), String>),
    Closed(window::Id),
    Key(iced::keyboard::Key),
}

struct Dialog {
    request: Pending,
    window: window::Id,
    deciding: bool,
    notice: Option<String>,
}

struct App {
    current: Option<Dialog>,
    queue: Vec<Pending>,
    // Requests already answered. The broker may keep listing a request
    // whose agent session ended; it must not be asked again.
    answered: std::collections::HashSet<String>,
    light: bool,
}

const ICON: &str = "shield-question-mark";

fn debug(message: impl FnOnce() -> String) {
    if std::env::var_os("ENZUI_AGENT_CONSENT_DEBUG").is_some() {
        eprintln!("enzui-agent-consent: {}", message());
    }
}

fn risk(risk: &str) -> (&'static str, BadgeTone) {
    match risk {
        "read_only" => (tr("shell-reads-information"), BadgeTone::Default),
        "sensitive_read" => (tr("shell-reads-private-information"), BadgeTone::Warning),
        "mutating" => (tr("shell-makes-changes"), BadgeTone::Warning),
        "destructive" => (tr("shell-can-delete-or-overwrite-data"), BadgeTone::Error),
        "external_side_effect" => (tr("shell-acts-outside-this-computer"), BadgeTone::Error),
        _ => (tr("shell-unknown-effect"), BadgeTone::Error),
    }
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let light = enzui_shell::settings::Store::from_environment()
            .and_then(|s| s.load().ok())
            .is_some_and(|s| s.light);
        (
            Self {
                current: None,
                queue: Vec::new(),
                answered: Default::default(),
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
    fn open_next(&mut self) -> Task<Message> {
        let Some(request) = self.queue.first().cloned() else {
            return Task::none();
        };
        let (window, open) = window::open(window::Settings {
            // Taller when the call has arguments to show.
            size: iced::Size::new(
                480.0,
                if request.arguments.is_some() {
                    500.0
                } else {
                    360.0
                },
            ),
            resizable: false,
            decorations: false,
            exit_on_close_request: false,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "dev.enzui.AgentConsent".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        self.current = Some(Dialog {
            request,
            window,
            deciding: false,
            notice: None,
        });
        open.discard()
    }
    fn close_current(&mut self) -> Task<Message> {
        self.current
            .take()
            .map_or_else(Task::none, |dialog| window::close(dialog.window))
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pending(mut list) => {
                debug(|| {
                    format!(
                        "pending {:?}",
                        list.iter().map(|p| &p.request_id).collect::<Vec<_>>()
                    )
                });
                // Only requests the broker still lists need remembering.
                self.answered
                    .retain(|id| list.iter().any(|p| p.request_id == *id));
                list.retain(|p| !self.answered.contains(&p.request_id));
                list.sort_by_key(|p| p.created_unix_ms);
                self.queue = list;
                // Answered elsewhere, cancelled by the agent or timed out.
                let gone = self.current.as_ref().is_some_and(|d| {
                    !d.deciding
                        && !self
                            .queue
                            .iter()
                            .any(|p| p.request_id == d.request.request_id)
                });
                let close = if gone {
                    self.close_current()
                } else {
                    Task::none()
                };
                if self.current.is_none() {
                    return close.chain(self.open_next());
                }
                close
            }
            Message::Decide(decision) => {
                let Some(dialog) = self.current.as_mut().filter(|d| !d.deciding) else {
                    return Task::none();
                };
                if decision != Decision::Deny
                    && decision != Decision::Never
                    && !dialog
                        .request
                        .allowed
                        .iter()
                        .any(|scope| scope == scope_key(decision))
                {
                    return Task::none();
                }
                dialog.deciding = true;
                dialog.notice = None;
                let id = dialog.request.request_id.clone();
                debug(|| format!("deciding {id}: {decision:?}"));
                Task::perform(
                    async move {
                        let result = tokio::task::spawn_blocking({
                            let id = id.clone();
                            move || agent_center::decide(&id, decision)
                        })
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-the-decision-could-not-be-sent").into()));
                        (id, result)
                    },
                    |(id, result)| Message::Decided(id, result),
                )
            }
            Message::Decided(id, result) => {
                debug(|| format!("decided {id}: {result:?}"));
                let Some(dialog) = self.current.as_mut().filter(|d| d.request.request_id == id)
                else {
                    return Task::none();
                };
                // A request the broker no longer knows cannot be answered;
                // it is dropped like an answered one.
                let gone = result.as_ref().is_err_and(|e| e == agent_center::GONE);
                match result {
                    _ if gone => {
                        self.answered.insert(id.clone());
                        self.queue.retain(|p| p.request_id != id);
                        let close = self.close_current();
                        close.chain(self.open_next())
                    }
                    Ok(()) => {
                        self.answered.insert(id.clone());
                        self.queue.retain(|p| p.request_id != id);
                        let close = self.close_current();
                        close.chain(self.open_next())
                    }
                    Err(error) => {
                        dialog.deciding = false;
                        dialog.notice = Some(error);
                        Task::none()
                    }
                }
            }
            // Dismissing the dialog is a refusal, never a silent approval.
            Message::Closed(window)
                if self.current.as_ref().is_some_and(|d| d.window == window) =>
            {
                self.update(Message::Decide(Decision::Deny))
            }
            Message::Closed(_) => Task::none(),
            Message::Key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)) => {
                self.update(Message::Decide(Decision::Deny))
            }
            Message::Key(_) => Task::none(),
        }
    }
    fn view(&self, window: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| {
        let Some(dialog) = self.current.as_ref().filter(|d| d.window == window) else { return Space::new().into() };
        let palette = self.tokens().palette;
        let request = &dialog.request;
        let named = |name: &str, id: &str| if name.is_empty() { id.to_owned() } else { name.to_owned() };
        let (effect, tone) = risk(&request.risk);
        let allowed = |decision: Decision| request.allowed.iter().any(|scope| scope == scope_key(decision));
        let mut header = row![
            enz_icon(ICON).size(22.0).into_element(),
            enz_text(tr!("shell-consent-allow-agent-to-use-app", "arg0" => named(&request.agent_name, &request.agent_id), "arg1" => named(&request.app_name, &request.app_id)))
                .size(TextSize::Title3)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if self.queue.len() > 1 {
            let at = self.queue.iter().position(|p| p.request_id == request.request_id).map_or(1, |i| i + 1);
            header = header.push(Space::new().width(Length::Fill)).push(enz_text(tr!("shell-of", "arg0" => format!("{:}", at), "arg1" => format!("{:}", self.queue.len()))).size(TextSize::Footnote).muted_on(palette.surface).into_element());
        }
        let mut function = column![
            row![
                enz_text(named(&request.function_title, &request.function_id)).weight_rung(enzui_theme::WeightRung::Semibold).into_element(),
                enz_badge(effect).tone(tone).into_element(),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(4);
        if !request.function_description.is_empty() {
            function = function.push(enz_text(request.function_description.clone()).muted_on(palette.surface).into_element());
        }
        let identity = column![
            enz_text(tr!("shell-consent-agent-id", "arg0" => request.agent_id.clone())).size(TextSize::Footnote).muted_on(palette.surface).into_element(),
            enz_text(tr!("shell-consent-app-function", "arg0" => request.app_id.clone(), "arg1" => request.function_id.clone())).size(TextSize::Footnote).muted_on(palette.surface).into_element(),
        ]
        .spacing(2);
        let busy = dialog.deciding;
        let mut allow = row![Space::new().width(Length::Fill)].spacing(8);
        if allowed(Decision::Always) {
            allow = allow.push(enz_button(tr("shell-always-allow"), Message::Decide(Decision::Always)).kind(ButtonKind::Secondary).enabled(!busy).into_element());
        }
        if allowed(Decision::Session) {
            allow = allow.push(enz_button(tr("shell-allow-this-session"), Message::Decide(Decision::Session)).kind(ButtonKind::Secondary).enabled(!busy).into_element());
        }
        if allowed(Decision::Once) {
            allow = allow.push(enz_button(tr("shell-allow-once"), Message::Decide(Decision::Once)).enabled(!busy).into_element());
        }
        let refuse = row![
            enz_button(enzui_shell::i18n::tr("shell-deny"), Message::Decide(Decision::Deny)).kind(ButtonKind::Secondary).id("agent-consent-deny".into()).enabled(!busy).into_element(),
            enz_button(tr("shell-never-allow"), Message::Decide(Decision::Never)).kind(ButtonKind::Plain).enabled(!busy).into_element(),
        ]
        .spacing(8);
        let mut content = column![header, function].spacing(14);
        // What exactly is being allowed: the call's own arguments.
        if let Some(arguments) = &request.arguments {
            let fill = palette.surface_alt;
            content = content.push(
                column![
                    enz_text(tr("shell-consent-details")).size(TextSize::Footnote).weight_rung(enzui_theme::WeightRung::Semibold).into_element(),
                    container(
                        iced::widget::scrollable(
                            enz_text(arguments.clone())
                                .size(TextSize::Footnote)
                                .font_role(enzui_theme::FontRole::Monospace)
                                .into_element(),
                        )
                        .height(Length::Fill),
                    )
                    .padding(10)
                    .height(Length::Fixed(132.0))
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(fill.into()),
                        border: iced::Border { radius: 8.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                ]
                .spacing(6),
            );
        }
        content = content.push(identity);
        if let Some(notice) = &dialog.notice {
            content = content.push(enz_text(notice.clone()).size(TextSize::Footnote).into_element());
        }
        content = content.push(Space::new().height(Length::Fill)).push(refuse).push(allow);
        container(content)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(palette.surface.into()), ..Default::default() })
            .into()

        })
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(poll).map(Message::Pending),
            window::close_requests().map(Message::Closed),
            iced::event::listen_with(|event, status, _| match (event, status) {
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }),
                    iced::event::Status::Ignored,
                ) => Some(Message::Key(key)),
                _ => None,
            }),
        ])
    }
}

fn scope_key(decision: Decision) -> &'static str {
    match decision {
        Decision::Once => "once",
        Decision::Session => "session",
        Decision::Always => "always",
        Decision::Deny => "deny",
        Decision::Never => "never",
    }
}

// Prefer broker long-polling; temporarily poll when wait_pending is unavailable.
fn poll() -> impl iced::futures::Stream<Item = Vec<Pending>> {
    iced::stream::channel(4, async move |mut output| {
        use iced::futures::SinkExt;
        let mut last: Option<Vec<Pending>> = None;
        let mut version: Option<u64> = None;
        let mut wait_after = std::time::Instant::now();
        loop {
            let available = agent_center::available();
            let mut waited = false;
            if available {
                let since = version;
                let wait = if std::time::Instant::now() >= wait_after {
                    tokio::task::spawn_blocking(move || agent_center::wait_pending(since))
                        .await
                        .ok()
                } else {
                    None
                };
                let list = match wait {
                    Some(Ok((list, next))) => {
                        version = Some(next);
                        waited = true;
                        Some(list)
                    }
                    failed => {
                        if failed.is_some() {
                            wait_after = std::time::Instant::now() + Duration::from_secs(60);
                        }
                        version = None;
                        match tokio::task::spawn_blocking(agent_center::pending).await {
                            Ok(Ok(list)) => Some(list),
                            Ok(Err(error)) => {
                                eprintln!("enzui-agent-consent: {error}");
                                None
                            }
                            Err(_) => None,
                        }
                    }
                };
                if let Some(list) = list
                    && last.as_ref() != Some(&list)
                {
                    last = Some(list.clone());
                    if output.send(list).await.is_err() {
                        return;
                    }
                }
            }
            if !waited {
                tokio::time::sleep(Duration::from_millis(if available { 800 } else { 5000 })).await;
            }
        }
    })
}

fn main() -> iced::Result {
    i18n::initialize();
    enzui_shell::polkit_agent::exit_with_parent();
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(App::new, App::update, App::view)
            .title(|_: &App, _| tr("shell-agent-request").to_owned())
            .theme(|app: &App, _| app.tokens().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
