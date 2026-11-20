//! Assistant corner button and chat panel backed by the harness client.

use enzui_shell::i18n::tr;
mod client;
mod transcript;
mod view;

use crate::{Message as ShellMessage, Shell};
use enzui_shell::compositor::{ControlGeometry, TriggerLayout};
use enzui_shell::settings::{AssistantCorner, BarEdge, Metrics};
use iced::{Task, widget::text_editor, window};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
pub(crate) use transcript::Transcript;

pub(crate) const TRANSCRIPT: &str = "assistant-transcript";
pub(crate) const COMPOSER: &str = "assistant-composer";

#[derive(Clone, Debug)]
pub(crate) struct SessionRow {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub updated: i64,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Service(client::Service),
    TriggerPlaced(u64, Result<(String, TriggerLayout), String>),
    Hover(bool),
    HideDue(u64),
    Toggle,
    PanelPrepared(u64, Result<ControlGeometry, String>),
    Hidden(window::Id),
    Close,
    Sessions(Result<Vec<SessionRow>, String>),
    Loaded(String, Result<Value, String>),
    Created(String, Result<String, String>),
    Turn(u64, client::TurnEvent),
    Answered(String, Result<Value, String>),
    Requested(Result<Value, String>),
    Edit(text_editor::Action),
    Send,
    Stop,
    Suggest(&'static str),
    NewChat,
    History(bool),
    Open(String),
    Toggled(String),
    Decide(String, bool),
    Link(String),
    Copy(String),
    Retry,
    Measured(enzui_widgets::row_measure::RowMeasurement),
    Scrolled(iced::widget::scrollable::Viewport),
    Latest,
    Tick,
    Placed(Result<(), String>),
    /// The tab's window reached its corner (or failed to).
    TriggerMoved(window::Id, Result<(), String>),
}

pub(crate) fn message(message: Message) -> ShellMessage {
    ShellMessage::Assistant(message)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    enabled: bool,
    corner: AssistantCorner,
    metrics: Metrics,
    bar: BarEdge,
}

pub(crate) struct Trigger {
    pub window: window::Id,
    pub layout: TriggerLayout,
    /// Moved to its corner since it mapped: until then it draws nothing (a
    /// new tab showed by the screen's edge for a few frames).
    pub placed: bool,
}

pub(crate) struct Pane {
    pub window: window::Id,
    pub geometry: ControlGeometry,
    pub visible: bool,
    pub started: bool,
}

pub(crate) struct Running {
    pub serial: u64,
    pub since: Instant,
}

#[derive(Default)]
pub(crate) struct State {
    placement: Option<Placement>,
    placement_serial: u64,
    retry: Option<Instant>,
    pub trigger: Option<Trigger>,
    /// The button shows (pointer at the corner or over it).
    pub shown: bool,
    hover: bool,
    hide_serial: u64,
    pub panel: Option<Pane>,
    panel_serial: u64,
    panel_opening: bool,
    pub service: Option<Result<(), String>>,
    /// The model the service answers with, for the composer.
    pub model: String,
    pub sessions: Vec<SessionRow>,
    pub session: Option<String>,
    pub title: String,
    pub transcript: Transcript,
    pub composer: text_editor::Content,
    pub running: Option<Running>,
    turn_serial: u64,
    pub history: bool,
    loaded: bool,
    pub error: Option<String>,
    pub scroll: f32,
    pub viewport: f32,
    pub away: bool,
    pub now: Option<Instant>,
}

impl State {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.trigger
            .as_ref()
            .is_some_and(|trigger| trigger.window == id)
            || self.owns_panel(id)
    }
    pub(crate) fn owns_panel(&self, id: window::Id) -> bool {
        self.panel.as_ref().is_some_and(|pane| pane.window == id)
    }
    pub(crate) fn owns_trigger(&self, id: window::Id) -> bool {
        self.trigger
            .as_ref()
            .is_some_and(|trigger| trigger.window == id)
    }
}

fn perform(
    future: impl std::future::Future<Output = Message> + Send + 'static,
) -> Task<ShellMessage> {
    Task::perform(future, message)
}

fn rows(value: &Value) -> Vec<SessionRow> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some(SessionRow {
                id: row["id"].as_str()?.to_owned(),
                title: row["title"]
                    .as_str()
                    .unwrap_or_else(|| tr("shell-chat"))
                    .to_owned(),
                preview: row["preview"].as_str().unwrap_or("").to_owned(),
                updated: row["updated"].as_i64().unwrap_or(0),
            })
        })
        .collect()
}

fn list() -> Task<ShellMessage> {
    perform(async {
        Message::Sessions(
            client::request("session.list", json!({}))
                .await
                .map(|value| rows(&value)),
        )
    })
}

fn export(id: String) -> Task<ShellMessage> {
    perform(async move {
        // A stored chat is reopened before it is shown, so it can continue; one
        // still open in the service refuses this and stays as it is.
        let _ = client::request("session.resume", json!({"session_id": id})).await;
        let result = client::request("session.export", json!({"session_id": id})).await;
        Message::Loaded(id, result)
    })
}

impl Shell {
    pub(crate) fn assistant_subscription(&self) -> iced::Subscription<ShellMessage> {
        if !self.desired_settings.assistant_enabled {
            return iced::Subscription::none();
        }
        iced::Subscription::batch([
            client::service().map(|state| message(Message::Service(state))),
            if self.assistant.running.is_some() {
                iced::time::every(Duration::from_millis(500)).map(|_| message(Message::Tick))
            } else {
                iced::Subscription::none()
            },
        ])
    }

    pub(crate) fn assistant_sync(&mut self) -> Task<ShellMessage> {
        let placement = Placement {
            enabled: self.desired_settings.assistant_enabled,
            corner: self.desired_settings.assistant_corner,
            metrics: self.metrics,
            bar: self.bar,
        };
        if self.assistant.placement == Some(placement)
            || self.assistant.retry.is_some_and(|due| Instant::now() < due)
        {
            return Task::none();
        }
        self.assistant.retry = None;
        self.assistant.placement = Some(placement);
        self.assistant.placement_serial += 1;
        if !placement.enabled {
            let trigger = self
                .assistant
                .trigger
                .take()
                .map(|trigger| self.finish_panel_close(trigger.window));
            return Task::batch([
                trigger.unwrap_or_else(Task::none),
                self.force_close_assistant(),
            ]);
        }
        let serial = self.assistant.placement_serial;
        let compositor = self.compositor.clone();
        let close = if self.assistant.panel.is_some() {
            self.force_close_assistant()
        } else {
            Task::none()
        };
        Task::batch([
            close,
            perform(async move {
                Message::TriggerPlaced(
                    serial,
                    compositor
                        .prepare_assistant_trigger(
                            placement.metrics,
                            placement.bar,
                            placement.corner,
                        )
                        .await,
                )
            }),
        ])
    }

    pub(crate) fn assistant_message(&mut self, msg: Message) -> Task<ShellMessage> {
        match msg {
            Message::Service(state) => {
                let ready = matches!(state, client::Service::Ready(_));
                self.assistant.service = Some(match state {
                    client::Service::Ready(model) => {
                        self.assistant.model = model;
                        Ok(())
                    }
                    client::Service::Failed(error) => Err(error),
                });
                if ready && self.assistant.panel.is_some() && !self.assistant.loaded {
                    self.assistant_load()
                } else {
                    Task::none()
                }
            }
            Message::TriggerPlaced(serial, result) => {
                if serial != self.assistant.placement_serial {
                    return Task::none();
                }
                match result {
                    Err(_) => {
                        // The desktop may not be ready yet: try again shortly.
                        self.assistant.placement = None;
                        self.assistant.retry = Some(Instant::now() + Duration::from_secs(2));
                        Task::none()
                    }
                    Ok((_, layout)) => {
                        let size = layout.window.size();
                        let mut replace = Task::none();
                        if let Some(trigger) = self.assistant.trigger.as_mut() {
                            if trigger.layout.window.size() == size {
                                trigger.layout = layout;
                                return Task::none();
                            }
                            // A fixed-size window cannot take a new size: it is
                            // replaced by one that has it.
                            let old = trigger.window;
                            self.assistant.trigger = None;
                            replace = self.finish_panel_close(old);
                        }
                        let (id, open) = window::open(crate::window_settings(
                            "dev.enzui.AssistantTrigger",
                            size.width,
                            size.height,
                        ));
                        self.assistant.trigger = Some(Trigger {
                            window: id,
                            layout,
                            placed: false,
                        });
                        Task::batch([replace, open.map(ShellMessage::Opened)])
                    }
                }
            }
            Message::Hover(on) => {
                self.assistant.hover = on;
                if on {
                    self.assistant.shown = self.assistant.panel.is_none();
                    Task::none()
                } else {
                    self.assistant.hide_serial += 1;
                    let serial = self.assistant.hide_serial;
                    perform(async move {
                        tokio::time::sleep(Duration::from_millis(650)).await;
                        Message::HideDue(serial)
                    })
                }
            }
            Message::HideDue(serial) => {
                if serial == self.assistant.hide_serial && !self.assistant.hover {
                    self.assistant.shown = false;
                }
                Task::none()
            }
            Message::Toggle => {
                if self
                    .assistant
                    .panel
                    .as_ref()
                    .is_some_and(|pane| pane.visible)
                {
                    self.close_assistant()
                } else {
                    self.open_assistant()
                }
            }
            Message::PanelPrepared(serial, result) => {
                if serial != self.assistant.panel_serial || !self.assistant.panel_opening {
                    return Task::none();
                }
                self.assistant.panel_opening = false;
                match result {
                    Err(error) => {
                        self.notice = Some(error);
                        Task::none()
                    }
                    Ok(geometry) => {
                        let size = self.assistant_window(&geometry);
                        let (id, open) = window::open(crate::window_settings(
                            "dev.enzui.AssistantPanel",
                            size.width,
                            size.height,
                        ));
                        self.assistant.panel = Some(Pane {
                            window: id,
                            geometry,
                            visible: false,
                            started: false,
                        });
                        let load = if self.assistant.loaded {
                            Task::none()
                        } else {
                            self.assistant_load()
                        };
                        Task::batch([open.map(ShellMessage::Opened), load])
                    }
                }
            }
            Message::Hidden(id) => {
                if self
                    .assistant
                    .panel
                    .as_ref()
                    .is_some_and(|pane| pane.window == id && !pane.visible)
                {
                    self.force_close_assistant()
                } else {
                    Task::none()
                }
            }
            Message::Close => self.close_assistant(),
            Message::Sessions(Ok(sessions)) => {
                self.assistant.sessions = sessions;
                if self.assistant.session.is_none() && !self.assistant.loaded {
                    self.assistant.loaded = true;
                    // The panel opens on the most recent chat.
                    if let Some(latest) = self.assistant.sessions.first() {
                        self.assistant.session = Some(latest.id.clone());
                        self.assistant.title = latest.title.clone();
                        return export(latest.id.clone());
                    }
                }
                self.assistant_title();
                Task::none()
            }
            Message::Sessions(Err(error)) => {
                self.assistant.error = Some(error);
                Task::none()
            }
            Message::Loaded(id, result) => {
                if self.assistant.session.as_deref() != Some(&id)
                    || self.assistant.running.is_some()
                {
                    return Task::none();
                }
                match result {
                    Ok(value) => {
                        self.assistant
                            .transcript
                            .load(value["items"].as_array().map_or(&[][..], Vec::as_slice));
                        value["title"]
                            .as_str()
                            .unwrap_or_else(|| tr("shell-chat"))
                            .clone_into(&mut self.assistant.title);
                        self.assistant.error = None;
                        self.assistant.away = false;
                        iced::widget::operation::snap_to(
                            TRANSCRIPT,
                            iced::widget::scrollable::RelativeOffset::END,
                            iced::widget::operation::Animation::Instant,
                        )
                    }
                    Err(error) => {
                        self.assistant.error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::Created(prompt, result) => match result {
                Ok(id) => {
                    self.assistant.session = Some(id.clone());
                    self.assistant_start(id, prompt)
                }
                Err(error) => {
                    self.assistant.error = Some(error);
                    Task::none()
                }
            },
            Message::Turn(serial, event) => {
                if self
                    .assistant
                    .running
                    .as_ref()
                    .is_none_or(|running| running.serial != serial)
                {
                    return Task::none();
                }
                match event {
                    client::TurnEvent::Event(event) => {
                        self.assistant.transcript.apply(&event);
                        Task::none()
                    }
                    client::TurnEvent::Finished(result) => {
                        match result {
                            Ok(_) => self.assistant.transcript.finish(None),
                            Err(error) => self.assistant.transcript.fail(&error),
                        }
                        self.assistant.running = None;
                        Task::batch([list(), iced::widget::operation::focus(COMPOSER)])
                    }
                }
            }
            Message::Answered(key, result) => {
                if result.is_err() {
                    self.assistant.transcript.undecided(&key);
                    self.assistant.error = result.err();
                }
                Task::none()
            }
            Message::Requested(result) => {
                if let Err(error) = result {
                    self.assistant.error = Some(error);
                }
                Task::none()
            }
            Message::Edit(action) => {
                self.assistant.composer.perform(action);
                Task::none()
            }
            Message::Send => {
                let text = self.assistant.composer.text().trim().to_owned();
                if text.is_empty() {
                    return Task::none();
                }
                self.assistant.composer = text_editor::Content::new();
                self.assistant_send(text)
            }
            Message::Suggest(prompt) => self.assistant_send(prompt.to_owned()),
            Message::Retry => match self.assistant.transcript.last_prompt() {
                Some(prompt) if self.assistant.running.is_none() => self.assistant_send(prompt),
                _ => Task::none(),
            },
            Message::Stop => match self.assistant.session.clone() {
                Some(session) if self.assistant.running.is_some() => perform(async move {
                    Message::Requested(
                        client::request("turn.cancel", json!({"session_id": session})).await,
                    )
                }),
                _ => Task::none(),
            },
            Message::NewChat => {
                if self.assistant.running.is_some() {
                    return Task::none();
                }
                self.assistant.session = None;
                self.assistant.title = String::new();
                self.assistant.transcript.clear();
                self.assistant.history = false;
                self.assistant.error = None;
                iced::widget::operation::focus(COMPOSER)
            }
            Message::History(open) => {
                self.assistant.history = open;
                if open {
                    list()
                } else {
                    iced::widget::operation::focus(COMPOSER)
                }
            }
            Message::Open(id) => {
                self.assistant.history = false;
                if self.assistant.running.is_some()
                    || self.assistant.session.as_deref() == Some(&id)
                {
                    return Task::none();
                }
                self.assistant.session = Some(id.clone());
                self.assistant.title = self
                    .assistant
                    .sessions
                    .iter()
                    .find(|row| row.id == id)
                    .map(|row| row.title.clone())
                    .unwrap_or_default();
                self.assistant.transcript.clear();
                export(id)
            }
            Message::Toggled(key) => {
                self.assistant.transcript.toggle(&key);
                Task::none()
            }
            Message::Decide(key, allow) => match (
                self.assistant.session.clone(),
                self.assistant.transcript.decide(&key, allow),
            ) {
                (Some(session), Some((request, binding))) => perform(async move {
                    let result = client::request("approval.resolve", json!({"session_id": session, "request_id": request, "binding": binding, "allow": allow})).await;
                    Message::Answered(key, result)
                }),
                _ => Task::none(),
            },
            Message::Link(url) => {
                if url.starts_with("https://") || url.starts_with("http://") {
                    Task::perform(
                        async move {
                            let _ = tokio::process::Command::new("xdg-open")
                                .arg(url)
                                .stdin(std::process::Stdio::null())
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .await;
                        },
                        |()| ShellMessage::RegionInterior,
                    )
                } else {
                    Task::none()
                }
            }
            Message::Copy(text) => {
                iced::clipboard::write(text).map(|_| ShellMessage::RegionInterior)
            }
            Message::Measured(measurement) => {
                let transcript = &mut self.assistant.transcript;
                if measurement.index < transcript.heights.len() {
                    let mut model = enzui_core::virtual_sequence::VariableHeightModel::new(
                        transcript.heights.iter().copied(),
                    );
                    self.assistant.scroll = enzui_widgets::virtual_list::reconcile_measurements(
                        &mut model,
                        self.assistant.scroll,
                        [measurement.into()],
                    );
                    transcript.heights[measurement.index] = measurement.height;
                }
                Task::none()
            }
            Message::Scrolled(viewport) => {
                self.assistant.scroll = viewport.absolute_offset().y;
                self.assistant.viewport = viewport.bounds().height;
                let distance = enzui_widgets::tail_follow::distance_from_end(
                    self.assistant.scroll,
                    viewport.bounds().height,
                    viewport.content_bounds().height,
                );
                self.assistant.away = distance > 120.0;
                Task::none()
            }
            Message::Latest => {
                self.assistant.away = false;
                iced::widget::operation::snap_to(
                    TRANSCRIPT,
                    iced::widget::scrollable::RelativeOffset::END,
                    iced::widget::operation::Animation::Instant,
                )
            }
            Message::Placed(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell assistant: {error}");
                }
                Task::none()
            }
            Message::TriggerMoved(id, result) => {
                match result {
                    Ok(()) => {
                        if let Some(trigger) =
                            self.assistant.trigger.as_mut().filter(|t| t.window == id)
                        {
                            trigger.placed = true;
                        }
                    }
                    Err(error) => eprintln!("enzui-shell assistant: {error}"),
                }
                Task::none()
            }
            Message::Tick => {
                self.assistant.now = Some(Instant::now());
                Task::none()
            }
        }
    }

    fn assistant_title(&mut self) {
        if let Some(session) = &self.assistant.session
            && let Some(row) = self
                .assistant
                .sessions
                .iter()
                .find(|row| &row.id == session)
        {
            self.assistant.title = row.title.clone();
        }
    }

    fn assistant_load(&mut self) -> Task<ShellMessage> {
        if self.assistant.service != Some(Ok(())) {
            return Task::none();
        }
        list()
    }

    fn assistant_send(&mut self, text: String) -> Task<ShellMessage> {
        self.assistant.history = false;
        self.assistant.away = false;
        if self.assistant.running.is_some() {
            // Mid-turn: the message steers the running turn.
            let Some(session) = self.assistant.session.clone() else {
                return Task::none();
            };
            return Task::batch([
                perform(async move {
                    Message::Requested(
                        client::request("turn.steer", json!({"session_id": session, "text": text}))
                            .await,
                    )
                }),
                iced::widget::operation::focus(COMPOSER),
            ]);
        }
        match self.assistant.session.clone() {
            Some(session) => self.assistant_start(session, text),
            None => {
                self.assistant.transcript.clear();
                perform(async move { Message::Created(text, client::open().await) })
            }
        }
    }

    fn assistant_start(&mut self, session: String, prompt: String) -> Task<ShellMessage> {
        self.assistant.turn_serial += 1;
        let serial = self.assistant.turn_serial;
        self.assistant.transcript.begin(&prompt);
        self.assistant.running = Some(Running {
            serial,
            since: Instant::now(),
        });
        self.assistant.now = Some(Instant::now());
        self.assistant.error = None;
        if self.assistant.title.is_empty() {
            self.assistant.title = prompt
                .split_whitespace()
                .take(6)
                .collect::<Vec<_>>()
                .join(" ");
        }
        Task::batch([
            Task::run(client::turn(session, prompt), move |event| {
                message(Message::Turn(serial, event))
            }),
            iced::widget::operation::focus(COMPOSER),
            iced::widget::operation::snap_to(
                TRANSCRIPT,
                iced::widget::scrollable::RelativeOffset::END,
                iced::widget::operation::Animation::Instant,
            ),
        ])
    }

    fn assistant_window(&self, geometry: &ControlGeometry) -> iced::Size {
        let m = enzui_shell::reveal::margins(
            self.desired_settings.assistant_corner.bottom(),
            geometry.attachment,
        );
        iced::Size::new(
            geometry.width + m.left + m.right,
            geometry.height + m.top + m.bottom,
        )
    }

    pub(crate) fn open_assistant(&mut self) -> Task<ShellMessage> {
        self.assistant.shown = false;
        if let Some(pane) = self.assistant.panel.as_mut() {
            pane.visible = true;
            return Task::batch([
                window::gain_focus(pane.window),
                iced::widget::operation::focus(COMPOSER),
            ]);
        }
        if self.assistant.panel_opening {
            return Task::none();
        }
        self.assistant.panel_opening = true;
        self.assistant.panel_serial += 1;
        let serial = self.assistant.panel_serial;
        let compositor = self.compositor.clone();
        let (metrics, bar, corner) = (
            self.metrics,
            self.bar,
            self.desired_settings.assistant_corner,
        );
        perform(async move {
            Message::PanelPrepared(
                serial,
                compositor
                    .prepare_assistant_panel(metrics, bar, corner)
                    .await,
            )
        })
    }

    pub(crate) fn close_assistant(&mut self) -> Task<ShellMessage> {
        self.assistant.panel_opening = false;
        if self.assistant.history {
            self.assistant.history = false;
        }
        match self.assistant.panel.as_mut() {
            Some(pane) if pane.started => {
                pane.visible = false;
                Task::none()
            }
            Some(_) => self.force_close_assistant(),
            None => Task::none(),
        }
    }

    pub(crate) fn force_close_assistant(&mut self) -> Task<ShellMessage> {
        self.assistant.panel_opening = false;
        let Some(pane) = self.assistant.panel.take() else {
            return Task::none();
        };
        // Pressing the button can nudge its window; it returns to its corner.
        let trigger = self
            .assistant
            .trigger
            .as_ref()
            .map(|trigger| trigger.window);
        let place = trigger.map_or_else(Task::none, |id| self.assistant_mapped(id));
        Task::batch([self.finish_panel_close(pane.window), place])
    }

    pub(crate) fn assistant_window_event(
        &mut self,
        id: window::Id,
        event: &window::Event,
    ) -> Task<ShellMessage> {
        if self.assistant.owns_trigger(id) {
            if matches!(event, window::Event::Closed) {
                self.assistant.trigger = None;
                // Placed again on the next update.
                self.assistant.placement = None;
            }
            return Task::none();
        }
        match event {
            window::Event::Focused => {
                if let Some(pane) = self
                    .assistant
                    .panel
                    .as_mut()
                    .filter(|pane| pane.window == id)
                {
                    let first = !pane.started;
                    pane.visible = true;
                    pane.started = true;
                    if first {
                        return iced::widget::operation::focus(COMPOSER);
                    }
                }
                Task::none()
            }
            window::Event::CloseRequested => self.close_assistant(),
            window::Event::Closed => {
                if self.assistant.owns_panel(id) {
                    self.assistant.panel = None;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    // Place mapped windows explicitly because window rules use the reserved work-area origin.
    pub(crate) fn assistant_mapped(&mut self, id: window::Id) -> Task<ShellMessage> {
        let compositor = self.compositor.clone();
        let (metrics, bar, corner) = (
            self.metrics,
            self.bar,
            self.desired_settings.assistant_corner,
        );
        let trigger = self.assistant.owns_trigger(id);
        // Once now and once more after the first frames have settled.
        Task::batch([Duration::ZERO, Duration::from_millis(400)].map(|delay| {
            let compositor = compositor.clone();
            perform(async move {
                tokio::time::sleep(delay).await;
                if trigger {
                    // The window may report mapped a little before the
                    // compositor lists it: try again for a few frames.
                    let mut result = Err(String::new());
                    for _ in 0..20 {
                        result = compositor
                            .move_assistant_trigger(metrics, bar, corner)
                            .await;
                        if result.is_ok() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(8)).await;
                    }
                    Message::TriggerMoved(id, result)
                } else {
                    Message::Placed(
                        compositor
                            .prepare_assistant_panel(metrics, bar, corner)
                            .await
                            .map(drop),
                    )
                }
            })
        }))
    }

    pub(crate) fn assistant_escape(&mut self, id: window::Id) -> Task<ShellMessage> {
        if self.assistant.owns_panel(id) {
            if self.assistant.history {
                self.assistant.history = false;
                return Task::none();
            }
            return self.close_assistant();
        }
        Task::none()
    }
}
