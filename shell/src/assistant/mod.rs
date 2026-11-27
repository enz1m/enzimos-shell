//! Assistant corner button and chat panel backed by the harness client.

use enzui_shell::{
    agent_harness::{self, ApprovalMode, Screenshots},
    i18n::tr,
};
mod attachments;
mod client;
mod resize;
mod transcript;
mod view;

use crate::{Message as ShellMessage, Shell};
use enzui_shell::compositor::{ControlGeometry, TriggerLayout};
use enzui_shell::settings::{AssistantCorner, BarEdge, Metrics};
use iced::{Task, widget::text_editor, window};
use serde_json::{Value, json};
use std::collections::HashMap;
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
    Attachments(attachments::Message),
    Resize(resize::Message),
    Regenerate,
    EditLast,
    CancelEdit,
    HistorySearch(String),
    Shortcuts(bool),
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
    Steered(String, Result<Value, String>),
    Edit(text_editor::Action),
    Send,
    Stop,
    Suggest(&'static str),
    NewChat,
    History(bool),
    Open(String),
    Toggled(String),
    Decide(String, bool, bool),
    ShowApprovals(bool),
    ChooseApproval(ApprovalMode),
    ConfirmFullSend,
    ApprovalChanged(Result<(), String>, ApprovalMode),
    ChooseScreenshots(Screenshots),
    AllowDesktopControl,
    DesktopControlFinished(String, Result<(), String>),
    ScreenshotsChanged(Result<Value, String>, Screenshots),
    Link(String),
    Copy(String),
    Retry,
    Measured(String, enzui_widgets::row_measure::RowMeasurement),
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

struct EditLast {
    draft: text_editor::Content,
}

struct RevisionRun {
    original: Transcript,
    prompt: String,
    started: bool,
}

pub(crate) struct Running {
    pub serial: u64,
    pub since: Instant,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    revision: Option<RevisionRun>,
    completed: Option<u64>,
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
    resize: resize::State,
    pub service: Option<Result<(), String>>,
    /// The model the service answers with, for the composer.
    pub model: String,
    pub sessions: Vec<SessionRow>,
    pub session: Option<String>,
    pub approval_mode: ApprovalMode,
    pub screenshots: Screenshots,
    pub choosing_approval: bool,
    pub confirming_full_send: bool,
    pub approval_busy: bool,
    pub desktop_control_pending: bool,
    pub title: String,
    pub transcript: Transcript,
    pub composer: text_editor::Content,
    drafts: HashMap<Option<String>, (text_editor::Content, Vec<attachments::Attachment>)>,
    files: Vec<attachments::Attachment>,
    last_files: Vec<attachments::Attachment>,
    attachment_loader: attachments::State,
    revision: Option<EditLast>,
    completed: Option<u64>,
    history_search: String,
    shortcuts: bool,
    steering: bool,
    pub export_failed: bool,
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
    fn new_approval(&mut self, preferred: ApprovalMode) {
        self.screenshots = Screenshots::Off;
        self.confirming_full_send = preferred == ApprovalMode::FullSend;
        self.choosing_approval = self.confirming_full_send;
        self.approval_mode = if self.confirming_full_send {
            ApprovalMode::Ask
        } else {
            preferred
        };
    }

    fn switch_draft(&mut self, next: Option<String>) {
        self.cancel_edit();
        self.attachment_loader.cancel();
        let (composer, files) = self.drafts.remove(&next).unwrap_or_default();
        let previous = std::mem::replace(&mut self.composer, composer);
        let previous_files = std::mem::replace(&mut self.files, files);
        if !previous.text().trim().is_empty() || !previous_files.is_empty() {
            self.drafts
                .insert(self.session.clone(), (previous, previous_files));
        }
        self.completed = None;
        self.last_files.clear();
        self.session = next;
    }

    fn cancel_edit(&mut self) {
        if let Some(edit) = self.revision.take() {
            self.composer = edit.draft;
        }
    }

    fn clear_sent_draft(&mut self, prompt: &str) {
        if self.composer.text().trim() == prompt {
            self.composer = text_editor::Content::new();
        }
    }

    fn attachments_available(&self) -> bool {
        self.files.len() < attachments::MAX_FILES
            && self.running.is_none()
            && !self.approval_busy
            && !self.attachment_loader.busy()
            && self.revision.is_none()
            && !self.history
            && !self.shortcuts
    }

    fn revision_allowed(&self) -> bool {
        self.completed.is_some()
            && self.session.is_some()
            && self.running.is_none()
            && !self.approval_busy
            && !self.steering
            && !self.confirming_full_send
            && !self.export_failed
            && !self.attachment_loader.busy()
    }

    fn can_revise(&self) -> bool {
        self.revision.is_none() && self.revision_allowed()
    }

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

fn revoke_desktop_control(session: Option<String>) -> Task<ShellMessage> {
    let Some(session) = session else {
        return Task::none();
    };
    Task::future(async move {
        use enzui_shell::desktop_control::{Request, request};
        let _ = request(&session, Request::Revoke).await;
    })
    .discard()
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
                    .filter(|title| !title.trim().is_empty())
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
        let result = async {
            let resumed = client::request("session.resume", json!({"session_id": id})).await?;
            let mut exported = client::request(
                "session.export",
                json!({"session_id": id, "items_only": true}),
            )
            .await?;
            exported["approval_mode"] = resumed["approval_mode"].clone();
            exported["screenshots"] = resumed["screenshots"].clone();
            Ok(exported)
        }
        .await;
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
            self.assistant.attachment_loader.subscription(),
            self.assistant.resize.subscription(),
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
            Message::Attachments(event) => self.assistant_attachments(event),
            Message::Resize(event) => self.assistant_resize(event),
            Message::Shortcuts(show) => {
                self.assistant.shortcuts = show;
                Task::none()
            }
            Message::HistorySearch(query) => {
                self.assistant.history_search = query;
                Task::none()
            }
            Message::Regenerate => self.assistant_revise(None),
            Message::EditLast => {
                if self.assistant.can_revise()
                    && let Some((prompt, _)) = self.assistant.transcript.last_user()
                {
                    let draft = std::mem::replace(
                        &mut self.assistant.composer,
                        text_editor::Content::with_text(prompt),
                    );
                    self.assistant.revision = Some(EditLast { draft });
                    self.assistant.error = None;
                    return iced::widget::operation::focus(COMPOSER);
                }
                Task::none()
            }
            Message::CancelEdit if self.assistant.running.is_none() => {
                self.assistant.cancel_edit();
                iced::widget::operation::focus(COMPOSER)
            }
            Message::CancelEdit => Task::none(),
            Message::Service(state) => {
                let ready = matches!(state, client::Service::Ready(_));
                self.assistant.service = Some(match state {
                    client::Service::Ready(model) => {
                        if !model.is_empty() {
                            self.assistant.model = model;
                        }
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
                        let mut settings = crate::window_settings(
                            "dev.enzui.AssistantPanel",
                            size.width,
                            size.height,
                        );
                        settings.resizable = true;
                        let (id, open) = window::open(settings);
                        self.assistant.viewport = (geometry.height - 125.0).max(200.0);
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
                if self.assistant.session.is_none()
                    && !self.assistant.loaded
                    && !self.assistant.approval_busy
                {
                    self.assistant.loaded = true;
                    // The panel opens on the most recent chat.
                    if let Some(latest) = self.assistant.sessions.first() {
                        self.assistant.approval_busy = true;
                        let (id, title) = (latest.id.clone(), latest.title.clone());
                        self.assistant.switch_draft(Some(id.clone()));
                        self.assistant.title = title;
                        return export(id);
                    }
                    self.assistant_new_approval();
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
                self.assistant.approval_busy = false;
                self.assistant.export_failed = result.is_err();
                match result {
                    Ok(value) => {
                        self.assistant.approval_mode = ApprovalMode::read(&value["approval_mode"]);
                        self.assistant.screenshots = Screenshots::read(&value["screenshots"]);
                        self.assistant.completed = value["last_turn_revision"].as_u64();
                        self.assistant
                            .transcript
                            .load(value["items"].as_array().map_or(&[][..], Vec::as_slice));
                        value["title"]
                            .as_str()
                            .filter(|title| !title.trim().is_empty())
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
            Message::Created(prompt, result) => {
                self.assistant.approval_busy = false;
                match result {
                    Ok(id) => {
                        self.assistant.session = Some(id.clone());
                        self.assistant.clear_sent_draft(&prompt);
                        let files = std::mem::take(&mut self.assistant.files);
                        self.assistant_start(id, prompt, files, None)
                    }
                    Err(error) => {
                        self.assistant.error = Some(error);
                        iced::widget::operation::focus(COMPOSER)
                    }
                }
            }
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
                        if event["type"] == "turn.started"
                            && let Some(model) = event["model"].as_str()
                        {
                            model.clone_into(&mut self.assistant.model);
                        }
                        if event["type"] == "turn.started"
                            && event["replacing"] == true
                            && let Some(revision) = self
                                .assistant
                                .running
                                .as_mut()
                                .and_then(|run| run.revision.as_mut())
                        {
                            self.assistant.transcript.replace_last(&revision.prompt);
                            revision.started = true;
                        }
                        if event["type"] == "turn.completed"
                            && event["status"] == "completed"
                            && let Some(run) = self.assistant.running.as_mut()
                        {
                            run.completed = event["sequence"].as_u64();
                        }
                        self.assistant.transcript.apply(&event);
                        Task::none()
                    }
                    client::TurnEvent::Finished(result) => {
                        let run = self.assistant.running.take().expect("turn serial matched");
                        let completed = matches!(&result, Ok(status) if status == "completed");
                        if let Some(revision) = run.revision
                            && (!completed || !revision.started)
                        {
                            self.assistant.transcript = revision.original;
                            self.assistant.error = result.err();
                            return iced::widget::operation::focus(COMPOSER);
                        }
                        self.assistant.completed = run.completed.filter(|_| completed);
                        if completed {
                            self.assistant.cancel_edit();
                        }
                        match result {
                            Ok(status) if status == "cancelled" => {
                                self.assistant
                                    .transcript
                                    .finish(Some(tr("shell-cancelled").into()));
                            }
                            Ok(status) if status == "failed" => self
                                .assistant
                                .transcript
                                .fail(tr("shell-assistant-request-unavailable")),
                            Ok(_) => self.assistant.transcript.finish(None),
                            Err(error) => self.assistant.transcript.fail(&error),
                        }
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
            Message::Steered(prompt, result) => {
                self.assistant.steering = false;
                match result {
                    Ok(_) => self.assistant.clear_sent_draft(&prompt),
                    Err(error) => self.assistant.error = Some(error),
                }
                Task::none()
            }
            Message::Edit(action) => {
                if self.assistant.revision.is_none() || self.assistant.running.is_none() {
                    self.assistant.composer.perform(action);
                }
                Task::none()
            }
            Message::Send => {
                let text = self.assistant.composer.text().trim().to_owned();
                if self.assistant.revision.is_some() {
                    return if text.is_empty() {
                        Task::none()
                    } else {
                        self.assistant_revise(Some(text))
                    };
                }
                if text.is_empty() && !self.assistant.files.is_empty() {
                    return self
                        .assistant_send(tr("shell-assistant-describe-attachments").to_owned());
                }
                if text.is_empty() {
                    return Task::none();
                }
                self.assistant_send(text)
            }
            Message::Suggest(prompt) => {
                self.assistant.composer = text_editor::Content::with_text(prompt);
                iced::widget::operation::focus(COMPOSER)
            }
            Message::Retry => match (
                self.assistant.session.clone(),
                self.assistant.transcript.last_prompt(),
            ) {
                (Some(session), Some(prompt))
                    if self.assistant.running.is_none()
                        && !self.assistant.approval_busy
                        && !self.assistant.confirming_full_send
                        && !self.assistant.steering
                        && !self.assistant.export_failed =>
                {
                    self.assistant_start(session, prompt, self.assistant.last_files.clone(), None)
                }
                _ => Task::none(),
            },
            Message::Stop => {
                if let Some(stop) = self
                    .assistant
                    .running
                    .as_mut()
                    .and_then(|run| run.stop.take())
                {
                    let _ = stop.send(());
                }
                Task::none()
            }
            Message::NewChat => {
                if self.assistant.running.is_some()
                    || self.assistant.approval_busy
                    || self.assistant.steering
                {
                    return Task::none();
                }
                let revoke = revoke_desktop_control(self.assistant.session.clone());
                if self.assistant.session.is_some() {
                    self.assistant.switch_draft(None);
                }
                self.assistant.completed = None;
                self.assistant.loaded = true;
                self.assistant.export_failed = false;
                self.assistant_new_approval();
                self.assistant.title = String::new();
                self.assistant.transcript.clear();
                self.assistant.shortcuts = false;
                self.assistant.history = false;
                self.assistant.error = None;
                Task::batch([revoke, iced::widget::operation::focus(COMPOSER)])
            }
            Message::History(open) => {
                self.assistant.history = open;
                self.assistant.shortcuts = false;
                if open {
                    self.assistant.history_search.clear();
                    Task::batch([list(), iced::widget::operation::focus("assistant-search")])
                } else {
                    iced::widget::operation::focus(COMPOSER)
                }
            }
            Message::Open(id) => {
                self.assistant.history = false;
                if self.assistant.running.is_some()
                    || self.assistant.approval_busy
                    || self.assistant.steering
                {
                    return Task::none();
                }
                if self.assistant.session.as_deref() == Some(&id) {
                    if self.assistant.export_failed {
                        self.assistant.approval_busy = true;
                        return export(id);
                    }
                    return iced::widget::operation::focus(COMPOSER);
                }
                let revoke = revoke_desktop_control(self.assistant.session.clone());
                self.assistant.switch_draft(Some(id.clone()));
                self.assistant.approval_busy = true;
                self.assistant.approval_mode = ApprovalMode::Ask;
                self.assistant.screenshots = Screenshots::Off;
                self.assistant.choosing_approval = false;
                self.assistant.confirming_full_send = false;
                self.assistant.title = self
                    .assistant
                    .sessions
                    .iter()
                    .find(|row| row.id == id)
                    .map(|row| row.title.clone())
                    .unwrap_or_default();
                self.assistant.transcript.clear();
                Task::batch([revoke, export(id)])
            }
            Message::Toggled(key) => {
                self.assistant.transcript.toggle(&key);
                Task::none()
            }
            Message::Decide(key, allow, remember) => match (
                self.assistant.session.clone(),
                self.assistant.transcript.decide(&key, allow),
            ) {
                (Some(session), Some((request, binding))) => perform(async move {
                    let result = client::request("approval.resolve", json!({"session_id": session, "request_id": request, "binding": binding, "allow": allow, "remember": remember})).await;
                    Message::Answered(key, result)
                }),
                _ => Task::none(),
            },
            Message::ShowApprovals(show) => {
                self.assistant.choosing_approval = show;
                if !show {
                    self.assistant.confirming_full_send = false;
                }
                Task::none()
            }
            Message::ChooseApproval(ApprovalMode::FullSend)
                if self.assistant.approval_mode != ApprovalMode::FullSend
                    && self.assistant.running.is_none()
                    && !self.assistant.approval_busy =>
            {
                self.assistant.confirming_full_send = true;
                Task::none()
            }
            Message::ChooseApproval(mode) => self.assistant_apply_approval(mode),
            Message::ConfirmFullSend if self.assistant.confirming_full_send => {
                self.assistant_apply_approval(ApprovalMode::FullSend)
            }
            Message::ConfirmFullSend => Task::none(),
            Message::ApprovalChanged(result, mode) => {
                self.assistant.approval_busy = false;
                match result {
                    Ok(()) => self.assistant.approval_mode = mode,
                    Err(error) => self.assistant.error = Some(error),
                }
                Task::none()
            }
            Message::AllowDesktopControl => {
                if self.assistant.desktop_control_pending || self.assistant.running.is_some() {
                    return Task::none();
                }
                let Some(id) = self.assistant.session.clone() else {
                    return Task::none();
                };
                self.assistant.desktop_control_pending = true;
                self.assistant.choosing_approval = false;
                self.assistant.error = None;
                perform(async move {
                    use enzui_shell::desktop_control::{Request, request};
                    let result = request(&id, Request::Allow).await;
                    Message::DesktopControlFinished(id, result)
                })
            }
            Message::DesktopControlFinished(id, result) => {
                self.assistant.desktop_control_pending = false;
                if self.assistant.session.as_deref() == Some(id.as_str()) {
                    self.assistant.error = result.err();
                }
                Task::none()
            }
            Message::ChooseScreenshots(access) => {
                if self.assistant.running.is_some() || self.assistant.approval_busy {
                    return Task::none();
                }
                if let Some(id) = self.assistant.session.clone() {
                    self.assistant.approval_busy = true;
                    perform(async move {
                        let result = client::request(
                            "session.set_screenshots",
                            json!({"session_id": id, "screenshots": access.wire()}),
                        )
                        .await;
                        Message::ScreenshotsChanged(result, access)
                    })
                } else {
                    self.assistant.screenshots = access;
                    Task::none()
                }
            }
            Message::ScreenshotsChanged(result, access) => {
                self.assistant.approval_busy = false;
                match result {
                    Ok(value) if value["screenshots"] == access.wire() => {
                        self.assistant.screenshots = access;
                        self.assistant.error = None;
                    }
                    Ok(_) => {
                        self.assistant.error =
                            Some(tr("shell-agent-screenshots-unavailable").into());
                    }
                    Err(error) => self.assistant.error = Some(error),
                }
                Task::none()
            }
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
            Message::Measured(key, measurement) => {
                let transcript = &mut self.assistant.transcript;
                if transcript
                    .rows
                    .get(measurement.index)
                    .is_some_and(|row| row.key == key)
                {
                    self.assistant.scroll = transcript.measure(measurement, self.assistant.scroll);
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
                self.assistant.away = !enzui_widgets::tail_follow::is_at_end(distance);
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

    fn assistant_new_approval(&mut self) {
        self.assistant
            .new_approval(agent_harness::approval_default());
    }

    fn assistant_apply_approval(&mut self, mode: ApprovalMode) -> Task<ShellMessage> {
        if self.assistant.running.is_some() || self.assistant.approval_busy {
            return Task::none();
        }
        self.assistant.choosing_approval = false;
        self.assistant.confirming_full_send = false;
        self.assistant.error = None;
        if let Some(id) = self.assistant.session.clone() {
            self.assistant.approval_busy = true;
            perform(async move {
                Message::ApprovalChanged(client::set_approval_mode(&id, mode).await, mode)
            })
        } else {
            self.assistant.approval_mode = mode;
            Task::none()
        }
    }

    fn assistant_send(&mut self, text: String) -> Task<ShellMessage> {
        if self.assistant.approval_busy
            || self.assistant.confirming_full_send
            || self.assistant.steering
            || self.assistant.export_failed
            || self.assistant.attachment_loader.busy()
        {
            return Task::none();
        }
        self.assistant.loaded = true;
        self.assistant.history = false;
        self.assistant.choosing_approval = false;
        if self.assistant.running.is_some() {
            // Mid-turn: the message steers the running turn.
            let Some(session) = self.assistant.session.clone() else {
                return Task::none();
            };
            self.assistant.steering = true;
            return Task::batch([
                perform(async move {
                    let result =
                        client::request("turn.steer", json!({"session_id": session, "text": text}))
                            .await;
                    Message::Steered(text, result)
                }),
                iced::widget::operation::focus(COMPOSER),
            ]);
        }
        match self.assistant.session.clone() {
            Some(session) => {
                self.assistant.clear_sent_draft(&text);
                let files = std::mem::take(&mut self.assistant.files);
                self.assistant_start(session, text, files, None)
            }
            None => {
                self.assistant.transcript.clear();
                self.assistant.approval_busy = true;
                let mode = self.assistant.approval_mode;
                let screenshots = self.assistant.screenshots;
                perform(
                    async move { Message::Created(text, client::open(mode, screenshots).await) },
                )
            }
        }
    }

    fn assistant_revise(&mut self, edited: Option<String>) -> Task<ShellMessage> {
        let state = &self.assistant;
        if !state.can_revise()
            && !(edited.is_some() && state.revision.is_some() && state.revision_allowed())
        {
            return Task::none();
        }
        let (Some(session), Some((last, _)), Some(expected)) = (
            state.session.clone(),
            state.transcript.last_user(),
            state.completed,
        ) else {
            return Task::none();
        };
        let prompt = edited.unwrap_or_else(|| last.to_owned());
        self.assistant_start(session, prompt, Vec::new(), Some(expected))
    }

    fn assistant_start(
        &mut self,
        session: String,
        prompt: String,
        files: Vec<attachments::Attachment>,
        expected: Option<u64>,
    ) -> Task<ShellMessage> {
        self.assistant.away = false;
        self.assistant.turn_serial += 1;
        let serial = self.assistant.turn_serial;
        let revision = expected.as_ref().map(|_| RevisionRun {
            original: self.assistant.transcript.clone(),
            prompt: prompt.clone(),
            started: false,
        });
        if revision.is_none() {
            self.assistant.completed = None;
            self.assistant.transcript.begin(
                &prompt,
                files.iter().map(|file| file.name.to_string()).collect(),
            );
            self.assistant.last_files = files.clone();
        }
        let (stop, cancelled) = tokio::sync::oneshot::channel();
        self.assistant.running = Some(Running {
            serial,
            since: Instant::now(),
            stop: Some(stop),
            revision,
            completed: None,
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
        let mode = self.assistant.approval_mode;
        let screenshots = self.assistant.screenshots;
        let (method, payload) = if let Some(expected) = expected {
            (
                "turn.revise",
                json!({"session_id": session, "prompt": prompt, "expected_revision": expected,
                "approval_mode": mode.wire(), "screenshots": screenshots.wire()}),
            )
        } else {
            (
                "turn.start",
                json!({"session_id": session, "prompt": prompt, "attachments": files.iter().map(attachments::Attachment::wire).collect::<Vec<_>>(),
                "approval_mode": mode.wire(), "screenshots": screenshots.wire()}),
            )
        };
        Task::batch([
            Task::run(client::turn(method, payload, cancelled), move |event| {
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
        let width = self.desired_settings.assistant_width;
        perform(async move {
            Message::PanelPrepared(
                serial,
                compositor
                    .prepare_assistant_panel(metrics, bar, corner, width)
                    .await,
            )
        })
    }

    pub(crate) fn close_assistant(&mut self) -> Task<ShellMessage> {
        self.assistant.attachment_loader.cancel();
        self.assistant.resize.drag = None;
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
        self.assistant.attachment_loader.cancel();
        self.assistant.resize.drag = None;
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
        let width = self.desired_settings.assistant_width;
        // Once now and once more after the first frames have settled.
        let delays = if trigger {
            vec![Duration::ZERO, Duration::from_millis(400)]
        } else {
            vec![Duration::ZERO]
        };
        Task::batch(delays.into_iter().map(|delay| {
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
                            .prepare_assistant_panel(metrics, bar, corner, width)
                            .await
                            .map(drop),
                    )
                }
            })
        }))
    }

    pub(crate) fn assistant_escape(&mut self, id: window::Id) -> Task<ShellMessage> {
        if self.assistant.owns_panel(id) {
            if self.assistant.history || self.assistant.shortcuts {
                self.assistant.history = false;
                self.assistant.shortcuts = false;
                return iced::widget::operation::focus(COMPOSER);
            }
            if self.assistant.revision.is_some() && self.assistant.running.is_none() {
                return self.assistant_message(Message::CancelEdit);
            }
            return self.close_assistant();
        }
        Task::none()
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;

    #[test]
    fn a_full_send_default_starts_a_new_chat_in_ask_pending_confirmation() {
        let mut chat = State {
            approval_mode: ApprovalMode::FullSend,
            screenshots: Screenshots::Vision,
            ..State::default()
        };
        chat.new_approval(ApprovalMode::FullSend);
        assert_eq!(chat.approval_mode, ApprovalMode::Ask);
        assert!(chat.confirming_full_send && chat.choosing_approval);
        assert_eq!(chat.screenshots, Screenshots::Off);

        chat.new_approval(ApprovalMode::WorkspaceEdits);
        assert_eq!(chat.approval_mode, ApprovalMode::WorkspaceEdits);
        assert!(!chat.confirming_full_send && !chat.choosing_approval);
    }
}
