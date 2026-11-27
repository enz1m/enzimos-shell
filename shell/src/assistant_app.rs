//! Assistant window for one harness session; closing cancels its turn and pending approvals.

use enzui_shell::agent_harness::{self, ApprovalMode, Screenshots};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    app_shell::{ShellLayout, enz_window_settings},
    button::{ButtonKind, ButtonSize, enz_button, enz_filter_chip},
    card::enz_card,
    preferences_row::enz_preferences_row,
    scroll_view::enz_scroll_view,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
};
use iced::{
    Element, Length, Task,
    futures::SinkExt,
    widget::{column, container, row},
};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Decision {
    Pending,
    Sending(bool),
    Allowed,
    Denied,
    Failed,
    Closed,
}

// Oversized approval arguments cannot be fully reviewed and may only be denied.
const SHOWN_ARGUMENTS: usize = 64 * 1024;

#[derive(Clone, Debug)]
enum Item {
    // The message and the skills chosen for its turn.
    User(String, Vec<String>),
    Reply(String),
    Tool {
        call: String,
        tool: String,
        status: Option<String>,
    },
    /// `complete` is false when the arguments are larger than can be shown.
    Approval {
        request: String,
        binding: String,
        tool: String,
        arguments: String,
        complete: bool,
        once_only: bool,
        reason: String,
        decision: Decision,
    },
    Note(String),
}

#[derive(Clone, Debug)]
enum Message {
    Ready(Result<Setup, String>),
    Prompt(String),
    Send,
    Opened(Result<String, String>),
    Event(Event),
    Approve(String, bool, bool),
    ShowPermissions(bool),
    ChooseApproval(ApprovalMode),
    EnableFullSend,
    ChooseScreenshots(Screenshots),
    AllowDesktopControl,
    DesktopControlFinished(Result<(), String>),
    Close(iced::window::Id),
    PermissionsChanged(Result<(ApprovalMode, Screenshots), String>),
    Resolved(String, bool, Result<(), String>),
    Cancel,
    New,
    Resume(String),
    Resumed(Result<(String, Vec<Item>, ApprovalMode, Screenshots), String>),
    Skill(String),
    Noop,
}

#[derive(Clone, Debug)]
struct SavedConversation {
    id: String,
    title: Option<String>,
    workspace: String,
}

#[derive(Debug, Clone)]
struct Setup {
    roots: Vec<String>,
    model: String,
    provider: String,
    approval_default: ApprovalMode,
    saved: Vec<SavedConversation>,
    // `Err` when the harness could not list them; turns still work without.
    skills: Result<Vec<agent_harness::Skill>, String>,
}

#[derive(Clone, Debug)]
enum Event {
    Frame(Value),
    Finished(Result<String, String>),
}

struct Assistant {
    tokens: EnzTheme,
    setup: Option<Result<Setup, String>>,
    session: Option<String>,
    items: Vec<Item>,
    prompt: String,
    pending_prompt: Option<String>,
    running: bool,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    approval_mode: ApprovalMode,
    screenshots: Screenshots,
    show_permissions: bool,
    confirm_full_send: bool,
    changing_permissions: bool,
    desktop_control_pending: bool,
    error: Option<String>,
    // Skills added to each turn, as listed: (name, sha256).
    chosen: Vec<(String, String)>,
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|_| Err(tr("shell-the-assistant-stopped-unexpectedly").into()))
}

fn ready() -> Task<Message> {
    Task::perform(
        blocking(|| {
            let config = agent_harness::load()?
                .ok_or_else(|| tr("shell-set-up-the-assistant-in-settings-agents-first"))?;
            let summary = agent_harness::summary(&config);
            let saved = agent_harness::request("session.list", json!({}))?
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    Some(SavedConversation {
                        id: row["id"].as_str()?.to_owned(),
                        title: row["title"].as_str().filter(|title| !title.trim().is_empty()).map(str::to_owned),
                        workspace: row["workspace"].as_str().unwrap_or("").to_owned(),
                    })
                })
                .collect();
            let skills = agent_harness::request("skills.list", json!({}))
                .map(|listed| agent_harness::skills(&listed));
            Ok(Setup {
                roots: summary.roots,
                model: summary.model,
                provider: summary.provider,
                approval_default: summary.approval_default,
                saved,
                skills,
            })
        }),
        Message::Ready,
    )
}

fn turn(
    session: String,
    prompt: String,
    skills: Vec<(String, String)>,
    mode: ApprovalMode,
    screenshots: Screenshots,
    stop: tokio::sync::oneshot::Receiver<()>,
) -> Task<Message> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    const MAX_FRAME: usize = 1 << 20;
    let stream = iced::stream::channel(
        64,
        async move |mut output: iced::futures::channel::mpsc::Sender<Event>| {
            let exchange = async {
                let stream = agent_harness::connect()?;
                stream
                    .set_nonblocking(true)
                    .map_err(|_| tr("shell-the-service-connection-failed"))?;
                let mut stream = tokio::net::UnixStream::from_std(stream)
                    .map_err(|_| tr("shell-the-service-connection-failed"))?;
                stream
                    .write_all(&agent_harness::turn_request(
                        &session,
                        &prompt,
                        &skills,
                        mode,
                        screenshots,
                    ))
                    .await
                    .map_err(|_| tr("shell-the-service-did-not-accept-the-turn"))?;
                let mut reader = tokio::io::BufReader::new(&mut stream);
                loop {
                    // Each frame is bounded while it is read, not after.
                    let mut line = Vec::new();
                    (&mut reader)
                        .take(MAX_FRAME as u64 + 1)
                        .read_until(b'\n', &mut line)
                        .await
                        .map_err(|_| tr("shell-the-service-connection-was-lost"))?;
                    if line.is_empty() {
                        return Err(
                            tr("shell-the-service-closed-before-the-turn-finished").to_owned()
                        );
                    }
                    if line.len() > MAX_FRAME || !line.ends_with(b"\n") {
                        return Err(tr("shell-the-service-sent-too-much-at-once").to_owned());
                    }
                    let frame: Value = serde_json::from_slice(&line)
                        .map_err(|_| tr("shell-the-service-sent-an-unknown-frame"))?;
                    if frame.get("event").is_some() {
                        let _ = output.send(Event::Frame(frame["event"].clone())).await;
                    } else if frame["ok"] == Value::Bool(true) {
                        return Ok(frame["status"].as_str().unwrap_or("completed").to_owned());
                    } else {
                        return Err(frame["error"].as_str().map_or_else(
                            || tr("shell-the-turn-failed").to_owned(),
                            |error| agent_harness::turn_error(error).map_or_else(
                                || tr!("shell-the-turn-failed-2", "arg0" => error.to_owned()),
                                str::to_owned,
                            ),
                        ));
                    }
                }
            };
            let result = tokio::select! {
                biased;
                _ = stop => Ok("cancelled".to_owned()),
                result = exchange => result,
            };
            let _ = output.send(Event::Finished(result)).await;
        },
    );
    Task::run(stream, Message::Event)
}

fn tokens() -> EnzTheme {
    let light = enzui_shell::settings::Store::from_environment()
        .and_then(|store| store.load().ok())
        .is_some_and(|settings| settings.light);
    let mut theme = if light {
        EnzTheme::light()
    } else {
        EnzTheme::dark()
    };
    enzui_shell::theme_store::Store::read(&enzui_core::theme_file::ThemePaths::from_env())
        .into_document()
        .apply(
            &mut theme,
            if light {
                enzui_theme::EnzThemeMode::Light
            } else {
                enzui_theme::EnzThemeMode::Dark
            },
        );
    theme
}

fn pretty(arguments: &Value) -> (String, bool) {
    let text = serde_json::to_string_pretty(arguments).unwrap_or_default();
    if text.len() <= SHOWN_ARGUMENTS {
        (text, true)
    } else {
        let mut end = SHOWN_ARGUMENTS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        (text.get(..end).unwrap_or_default().to_owned(), false)
    }
}

impl Assistant {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                tokens: tokens(),
                setup: None,
                session: None,
                items: Vec::new(),
                prompt: String::new(),
                pending_prompt: None,
                running: false,
                stop: None,
                approval_mode: ApprovalMode::Ask,
                screenshots: Screenshots::Off,
                show_permissions: false,
                confirm_full_send: false,
                changing_permissions: false,
                desktop_control_pending: false,
                error: None,
                chosen: Vec::new(),
            },
            ready(),
        )
    }
    fn start_turn(&mut self, session: String, prompt: String) -> Task<Message> {
        let (stop, cancelled) = tokio::sync::oneshot::channel();
        self.stop = Some(stop);
        turn(
            session,
            prompt,
            self.chosen.clone(),
            self.approval_mode,
            self.screenshots,
            cancelled,
        )
    }

    fn configure(&mut self, mode: ApprovalMode, screenshots: Screenshots) -> Task<Message> {
        if self.running || self.changing_permissions {
            return Task::none();
        }
        self.confirm_full_send = false;
        self.error = None;
        let Some(session_id) = self.session.clone() else {
            self.approval_mode = mode;
            self.screenshots = screenshots;
            return Task::none();
        };
        let mode_changed = mode != self.approval_mode;
        if !mode_changed && screenshots == self.screenshots {
            return Task::none();
        }
        self.changing_permissions = true;
        Task::perform(
            blocking(move || {
                if mode_changed {
                    agent_harness::request(
                        "session.set_approval_mode",
                        json!({"session_id": session_id,
                        "approval_mode": mode.wire(), "confirm_full_send": mode == ApprovalMode::FullSend}),
                    )?;
                } else {
                    agent_harness::request(
                        "session.set_screenshots",
                        json!({"session_id": session_id,
                        "screenshots": screenshots.wire()}),
                    )?;
                }
                Ok((mode, screenshots))
            }),
            Message::PermissionsChanged,
        )
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let task = match message {
            Message::Ready(setup) => {
                // A choice survives only while the same skill is still listed.
                let listed = |name: &str, sha: &str| matches!(&setup, Ok(Setup { skills: Ok(skills), .. }) if skills.iter().any(|skill| skill.name == name && skill.sha256 == sha));
                let before = self.chosen.len();
                self.chosen.retain(|(name, sha)| listed(name, sha));
                if self.chosen.len() < before {
                    self.items.push(Item::Note(
                        tr("shell-a-chosen-skill-changed-or-was-removed-choose-it-again").into(),
                    ));
                }
                if self.session.is_none()
                    && self.items.is_empty()
                    && let Ok(ready) = &setup
                {
                    self.confirm_full_send = ready.approval_default == ApprovalMode::FullSend;
                    self.show_permissions = self.confirm_full_send;
                    self.approval_mode = if self.confirm_full_send {
                        ApprovalMode::Ask
                    } else {
                        ready.approval_default
                    };
                }
                self.setup = Some(setup);
                Task::none()
            }
            Message::Skill(name) => {
                if let Some(index) = self.chosen.iter().position(|(chosen, _)| *chosen == name) {
                    self.chosen.remove(index);
                } else if self.chosen.len() < agent_harness::MAX_TURN_SKILLS
                    && let Some(Ok(Setup {
                        skills: Ok(skills), ..
                    })) = &self.setup
                    && let Some(skill) = skills.iter().find(|skill| skill.name == name)
                {
                    self.chosen.push((skill.name.clone(), skill.sha256.clone()));
                }
                Task::none()
            }
            Message::Prompt(text) => {
                self.prompt = text;
                Task::none()
            }
            Message::Send
                if !self.running
                    && !self.changing_permissions
                    && !self.confirm_full_send
                    && matches!(self.setup, Some(Ok(_)))
                    && !self.prompt.trim().is_empty() =>
            {
                let prompt = std::mem::take(&mut self.prompt);
                self.items.push(Item::User(
                    prompt.clone(),
                    self.chosen.iter().map(|(name, _)| name.clone()).collect(),
                ));
                self.running = true;
                self.show_permissions = false;
                self.error = None;
                match self.session.clone() {
                    Some(session) => self.start_turn(session, prompt),
                    None => {
                        let Some(Ok(setup)) = &self.setup else {
                            return Task::none();
                        };
                        let Some(workspace) = setup.roots.first().cloned() else {
                            self.running = false;
                            self.error = Some(tr("shell-choose-a-workspace-folder-in-settings-agents-first").into());
                            return Task::none();
                        };
                        if setup.model.is_empty() && setup.provider != "local" {
                            self.running = false;
                            self.error = Some(tr("shell-choose-a-workspace-folder-and-a-model-in-settings-agents-first").into());
                            return Task::none();
                        }
                        let (model, provider) = (setup.model.clone(), setup.provider.clone());
                        let mode = self.approval_mode;
                        let screenshots = self.screenshots;
                        self.pending_prompt = Some(prompt);
                        Task::perform(
                            blocking(move || {
                                let initial = if mode == ApprovalMode::FullSend {
                                    ApprovalMode::Ask
                                } else {
                                    mode
                                };
                                let reply = agent_harness::request(
                                    "session.create",
                                    json!({
                                        "workspace": workspace, "model": model, "provider": provider,
                                        "approval_mode": initial.wire(), "screenshots": screenshots.wire(),
                                    }),
                                )?;
                                let id = reply["session_id"].as_str().ok_or_else(|| {
                                    tr("shell-the-service-did-not-open-a-session").to_owned()
                                })?;
                                if mode == ApprovalMode::FullSend
                                    && let Err(error) = agent_harness::request(
                                        "session.set_approval_mode",
                                        json!({
                                            "session_id": id, "approval_mode": mode.wire(), "confirm_full_send": true,
                                        }),
                                    )
                                {
                                    let _ = agent_harness::request(
                                        "session.close",
                                        json!({"session_id": id}),
                                    );
                                    return Err(error);
                                }
                                Ok(id.to_owned())
                            }),
                            Message::Opened,
                        )
                    }
                }
            }
            Message::Send => Task::none(),
            Message::Opened(Ok(session)) => {
                self.session = Some(session.clone());
                match self.pending_prompt.take() {
                    Some(prompt) => self.start_turn(session, prompt),
                    None => {
                        self.running = false;
                        self.items.push(Item::Note(tr("shell-cancelled").into()));
                        Task::none()
                    }
                }
            }
            Message::Opened(Err(error)) => {
                self.running = false;
                self.pending_prompt = None;
                self.error = Some(error);
                Task::none()
            }
            Message::Event(Event::Frame(event)) => {
                self.event(&event);
                Task::none()
            }
            Message::Event(Event::Finished(result)) => {
                self.running = false;
                self.stop = None;
                for item in &mut self.items {
                    if let Item::Approval { decision, .. } = item
                        && matches!(decision, Decision::Pending | Decision::Sending(_))
                    {
                        *decision = Decision::Closed;
                    }
                }
                match result {
                    Ok(status) if status == "cancelled" => {
                        self.items.push(Item::Note(tr("shell-cancelled").into()));
                    }
                    Ok(_) => {}
                    // A chosen skill changed or went away since it was listed:
                    // list again, which drops that choice and says so.
                    Err(error) if error.ends_with("skill-changed") => {
                        self.error = None;
                        return ready();
                    }
                    Err(error) => self.error = Some(error),
                }
                Task::none()
            }
            Message::Approve(request, allow, remember) => {
                let Some(session) = self.session.clone() else {
                    return Task::none();
                };
                let Some(binding) = self.items.iter_mut().find_map(|item| match item {
                    // Arguments that cannot all be shown can only be denied.
                    Item::Approval {
                        request: id,
                        binding,
                        decision,
                        complete,
                        ..
                    } if *id == request
                        && *decision == Decision::Pending
                        && (*complete || !allow) =>
                    {
                        *decision = Decision::Sending(allow);
                        Some(binding.clone())
                    }
                    _ => None,
                }) else {
                    return Task::none();
                };
                let id = request.clone();
                Task::perform(
                    blocking(move || {
                        agent_harness::request("approval.resolve", json!({"session_id": session, "request_id": request, "binding": binding, "allow": allow, "remember": remember})).map(drop)
                    }),
                    move |result| Message::Resolved(id.clone(), allow, result),
                )
            }
            Message::Resolved(request, allow, result) => {
                for item in &mut self.items {
                    if let Item::Approval {
                        request: id,
                        decision,
                        ..
                    } = item
                        && *id == request
                        && *decision == Decision::Sending(allow)
                    {
                        *decision = match &result {
                            Ok(()) if allow => Decision::Allowed,
                            Ok(()) => Decision::Denied,
                            Err(_) => Decision::Failed,
                        };
                    }
                }
                if let Err(error) = result {
                    self.error = Some(error);
                }
                Task::none()
            }
            Message::ShowPermissions(show) => {
                self.show_permissions = show;
                if !show {
                    self.confirm_full_send = false;
                }
                Task::none()
            }
            Message::ChooseApproval(ApprovalMode::FullSend)
                if self.approval_mode != ApprovalMode::FullSend =>
            {
                self.confirm_full_send = true;
                Task::none()
            }
            Message::ChooseApproval(mode) => self.configure(mode, self.screenshots),
            Message::EnableFullSend if self.confirm_full_send => {
                self.configure(ApprovalMode::FullSend, self.screenshots)
            }
            Message::EnableFullSend => Task::none(),
            Message::AllowDesktopControl => {
                let Some(session) = self.session.clone().filter(|_| !self.running && !self.desktop_control_pending) else {
                    return Task::none();
                };
                self.desktop_control_pending = true;
                self.show_permissions = false;
                self.error = None;
                Task::perform(
                    async move {
                        use enzui_shell::desktop_control::{Request, request};
                        request(&session, Request::Allow).await
                    },
                    Message::DesktopControlFinished,
                )
            }
            Message::DesktopControlFinished(result) => {
                self.desktop_control_pending = false;
                self.error = result.err();
                Task::none()
            }
            Message::ChooseScreenshots(screenshots) => {
                self.configure(self.approval_mode, screenshots)
            }
            Message::PermissionsChanged(result) => {
                self.changing_permissions = false;
                match result {
                    Ok((mode, screenshots)) => {
                        self.approval_mode = mode;
                        self.screenshots = screenshots;
                    }
                    Err(error) => self.error = Some(error),
                }
                Task::none()
            }
            Message::Close(window) => {
                if let Some(stop) = self.stop.take() {
                    let _ = stop.send(());
                }
                let close = iced::window::close(window);
                if let Some(session) = self.session.take() {
                    Task::future(async move {
                        use enzui_shell::desktop_control::{Request, request};
                        let _ = request(&session, Request::Revoke).await;
                    }).discard().chain(close)
                } else {
                    close
                }
            }
            Message::Cancel => {
                if let Some(stop) = self.stop.take() {
                    let _ = stop.send(());
                }
                self.pending_prompt = None;
                Task::none()
            }
            Message::New if !self.running && !self.changing_permissions && !self.desktop_control_pending => {
                self.screenshots = Screenshots::Off;
                self.show_permissions = false;
                self.confirm_full_send = false;
                let closing = self.session.take();
                self.items.clear();
                self.error = None;
                let close = match closing {
                    Some(session) => Task::perform(
                        blocking(move || {
                            agent_harness::request("session.close", json!({"session_id": session}))
                                .map(drop)
                        }),
                        |_| Message::Noop,
                    ),
                    None => Task::none(),
                };
                close.chain(ready())
            }
            Message::New => Task::none(),
            Message::Resume(session)
                if !self.running && !self.changing_permissions && self.session.is_none() =>
            {
                self.changing_permissions = true;
                Task::perform(
                    blocking(move || {
                        let resumed = agent_harness::request(
                            "session.resume",
                            json!({"session_id": session}),
                        )?;
                        let saved = agent_harness::request(
                            "session.export",
                            json!({"session_id": session}),
                        )?;
                        let items = saved["messages"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|message| {
                                let content = message["content"]
                                    .as_str()
                                    .filter(|text| !text.trim().is_empty())?
                                    .to_owned();
                                match message["role"].as_str()? {
                                    "user" => Some(Item::User(content, Vec::new())),
                                    "assistant" => Some(Item::Reply(content)),
                                    _ => None,
                                }
                            })
                            .collect();
                        Ok((
                            session,
                            items,
                            ApprovalMode::read(&resumed["approval_mode"]),
                            Screenshots::read(&resumed["screenshots"]),
                        ))
                    }),
                    Message::Resumed,
                )
            }
            Message::Resume(_) => Task::none(),
            Message::Resumed(Ok((session, items, mode, screenshots))) => {
                self.changing_permissions = false;
                self.show_permissions = false;
                self.confirm_full_send = false;
                self.approval_mode = mode;
                self.screenshots = screenshots;
                self.session = Some(session);
                self.items = items;
                Task::none()
            }
            Message::Resumed(Err(error)) => {
                self.changing_permissions = false;
                self.error = Some(error);
                Task::none()
            }
            Message::Noop => Task::none(),
        };
        // New items appear at the bottom, so the transcript follows them.
        let follow = iced::widget::operation::snap_to(
            "assistant-transcript",
            iced::widget::scrollable::RelativeOffset::END,
            iced::widget::operation::Animation::Instant,
        );
        Task::batch([task, follow])
    }
    fn event(&mut self, event: &Value) {
        let text = |key: &str| event[key].as_str().unwrap_or("").to_owned();
        match event["type"].as_str() {
            Some("message.completed") => {
                if let Some(content) = event["content"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                {
                    self.items.push(Item::Reply(content.to_owned()));
                }
            }
            Some("tool.started") => self.items.push(Item::Tool {
                call: text("call_id"),
                tool: text("tool"),
                status: None,
            }),
            Some("tool.completed") => {
                let call = text("call_id");
                if let Some(Item::Tool { status, .. }) = self
                    .items
                    .iter_mut()
                    .rev()
                    .find(|item| matches!(item, Item::Tool { call: c, .. } if *c == call))
                {
                    *status = Some(text("status"));
                }
            }
            Some("approval.requested") => {
                let (arguments, complete) = pretty(&event["arguments"]);
                self.items.push(Item::Approval {
                    request: text("request_id"),
                    binding: text("binding"),
                    tool: text("tool"),
                    arguments,
                    complete,
                    once_only: event["once_only"] == true,
                    reason: text("reason"),
                    decision: Decision::Pending,
                });
            }
            Some("approval.resolved") => {
                let request = text("request_id");
                for item in &mut self.items {
                    if let Item::Approval {
                        request: id,
                        decision: decision @ Decision::Pending,
                        ..
                    } = item
                        && *id == request
                    {
                        *decision = Decision::Closed;
                    }
                }
            }
            Some("turn.vision_fallback") => self
                .items
                .push(Item::Note(tr("shell-assistant-vision-fallback").into())),
            Some("turn.local_limit") => self
                .items
                .push(Item::Note(tr("shell-assistant-local-limit").into())),
            Some("turn.completed") if event["status"] == "failed" => self
                .items
                .push(Item::Note(tr("shell-the-turn-failed").into())),
            _ => {}
        }
    }
    fn view(&self) -> Element<'_, Message> {
        i18n::localizer().view(|| {
        let tokens = &self.tokens;
        let palette = tokens.palette.clone();
        let muted = |text: String| enz_text(text).muted_on(palette.background).wrap(true).into_element();
        let mut body = column![].spacing(10).padding(16);
        match &self.setup {
            None => body = body.push(muted(tr("shell-connecting-to-the-assistant").into())),
            Some(Err(error)) => body = body.push(muted(error.clone())),
            Some(Ok(setup)) if self.session.is_none() && self.items.is_empty() => {
                body = body.push(muted(match setup.roots.first() {
                    Some(root) => {
                        let model = if !setup.model.is_empty() {
                            setup.model.as_str()
                        } else if setup.provider == "local" {
                            tr("shell-assistant-automatic-model")
                        } else {
                            tr("shell-no-model-chosen-yet")
                        };
                        tr!("shell-new-sessions-work-in-with", "arg0" => root.clone(), "arg1" => model.to_owned())
                    },
                    None => tr("shell-add-a-workspace-folder-in-settings-agents-to-start").into(),
                }));
                for (index, saved) in setup.saved.iter().enumerate().take(12) {
                    body = body.push(
                        enz_preferences_row(saved.title.clone().unwrap_or_else(|| tr!("shell-session", "arg0" => (index + 1).to_string())))
                            .subtitle(saved.workspace.clone())
                            .action(tr("shell-resume"), Message::Resume(saved.id.clone()))
                            .into_element(),
                    );
                }
            }
            Some(Ok(_)) => {}
        }
        for item in &self.items {
            body = body.push(match item {
                Item::User(text, skills) if skills.is_empty() => container(enz_text(text.clone()).wrap(true).into_element()).padding([8, 12]).width(Length::Fill).into(),
                Item::User(text, skills) => container(column![enz_text(text.clone()).wrap(true).into_element(), muted(format!("With {}", skills.join(", ")))].spacing(4)).padding([8, 12]).width(Length::Fill).into(),
                Item::Reply(text) => enz_card(enz_text(text.clone()).wrap(true).into_element()).padding(12).width_length(Length::Fill).into_element(),
                Item::Tool { tool, status, .. } => muted(format!(
                    "{tool}: {}",
                    match status.as_deref() {
                        None => "running",
                        Some("completed") => "done",
                        Some(other) => other,
                    }
                )),
                Item::Note(text) => muted(text.clone()),
                Item::Approval { request, tool, arguments, complete, decision, once_only, reason, .. } => {
                    let mut card = column![
                        enz_text(format!("Allow {tool}?")).size(TextSize::Body).weight_rung(enzui_theme::WeightRung::Semibold).into_element(),
                        container(enz_text(arguments.clone()).font_role(enzui_theme::FontRole::Monospace).font_from_theme(tokens).wrap(true).into_element()).padding(8).width(Length::Fill),
                    ]
                    .spacing(8);
                    if !reason.is_empty() {
                        card = card.push(muted(reason.clone()));
                    }
                    if !complete {
                        card = card.push(muted(tr("shell-these-arguments-are-too-large-to-show-in-full-so-they-can-only-be-denied").into()));
                    }
                    card = card.push(match decision {
                        Decision::Pending => row![
                            iced::widget::space().width(Length::Fill),
                            enz_button(enzui_shell::i18n::tr("shell-deny"), Message::Approve(request.clone(), false, false)).kind(ButtonKind::Secondary).into_element(),
                            if *once_only || tool == "screenshot" {
                                iced::widget::space().width(Length::Shrink).into()
                            } else {
                                enz_button(tr("shell-agent-allow-for-session"), Message::Approve(request.clone(), true, true))
                                    .kind(ButtonKind::Secondary).enabled(*complete).into_element()
                            },
                            enz_button(
                                tr(if tool == "screenshot" { "shell-agent-allow-screenshots" } else { "shell-allow-once" }),
                                Message::Approve(request.clone(), true, false)
                            ).kind(ButtonKind::Primary).enabled(*complete).into_element(),
                        ]
                        .spacing(8)
                        .into(),
                        Decision::Sending(true) => muted(tr("shell-allowing").into()),
                        Decision::Sending(false) => muted(tr("shell-denying").into()),
                        Decision::Allowed => muted(tr("shell-allowed").into()),
                        Decision::Denied => muted(tr("shell-denied").into()),
                        Decision::Failed => muted(tr("shell-the-answer-was-not-accepted-the-action-did-not-run-with-your-approval").into()),
                        Decision::Closed => muted(tr("shell-no-longer-waiting").into()),
                    });
                    enz_card(card).padding(12).width_length(Length::Fill).into_element()
                }
            });
        }
        if let Some(error) = &self.error {
            body = body.push(muted(error.clone()));
        }
        let transcript = enz_scroll_view(body).id("assistant-transcript".into()).height(Length::Fill).into_element();
        let field = enz_text_field("", enzui_shell::i18n::current().text("shell-ask-the-assistant"), &self.prompt, Message::Prompt).on_submit(Message::Send).id("assistant-prompt".into()).into_element();
        let action = if self.running {
            enz_button(tr("shell-cancel"), Message::Cancel).kind(ButtonKind::Secondary).into_element()
        } else {
            enz_button(tr("shell-send"), Message::Send).kind(ButtonKind::Primary).enabled(matches!(self.setup, Some(Ok(_))) && !self.confirm_full_send && !self.changing_permissions && !self.prompt.trim().is_empty()).into_element()
        };
        let header = row![
            enz_text(enzui_shell::i18n::tr("shell-assistant")).size(TextSize::Title3).into_element(),
            iced::widget::space().width(Length::Fill),
            if self.approval_mode == ApprovalMode::FullSend {
                enz_text(ApprovalMode::FullSend.label()).color(palette.danger).into_element()
            } else {
                iced::widget::space().width(Length::Shrink).into()
            },
            enz_button(tr("shell-new-session"), Message::New).kind(ButtonKind::Quiet).size(ButtonSize::Sm).icon("plus").enabled(!self.running && !self.changing_permissions).into_element(),
        ]
        .align_y(iced::Alignment::Center)
        .padding([10, 16]);
        let composer = row![container(field).width(Length::Fill), action].spacing(8).align_y(iced::Alignment::Center).padding([10, 16]);
        let permissions: Element<'_, Message> = if self.show_permissions {
            if self.confirm_full_send {
                column![
                    muted(tr("shell-agent-full-send-warning").into()),
                    row![
                        enz_button(tr("shell-cancel"), Message::ShowPermissions(false)).kind(ButtonKind::Secondary).into_element(),
                        enz_button(tr("shell-agent-confirm-full-send"), Message::EnableFullSend).kind(ButtonKind::Accent).into_element(),
                    ].spacing(8),
                ].spacing(6).padding([6, 16]).into()
            } else {
                let modes = ApprovalMode::ALL.into_iter().map(|mode| {
                    enz_filter_chip(mode.label(), self.approval_mode == mode, Message::ChooseApproval(mode))
                        .enabled(!self.changing_permissions).into_element()
                });
                let access = Screenshots::ALL.into_iter().map(|access| {
                    enz_filter_chip(access.label(), self.screenshots == access, Message::ChooseScreenshots(access))
                        .enabled(!self.changing_permissions).into_element()
                });
                column![
                    iced::widget::row(modes).spacing(6).wrap(),
                    muted(tr("shell-agent-screenshots-caption").into()),
                    iced::widget::row(access).spacing(6).wrap(),
                    if self.session.is_some() && enzui_shell::desktop_session::mode().managed() {
                        enz_button(tr("shell-assistant-control-allow"), Message::AllowDesktopControl)
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .enabled(!self.desktop_control_pending)
                            .into_element()
                    } else {
                        iced::widget::space().height(0).into()
                    },
                ].spacing(6).padding([6, 16]).into()
            }
        } else {
            iced::widget::space().height(0).into()
        };
        let mode = container(row![
            enz_button(self.approval_mode.label(), Message::ShowPermissions(!self.show_permissions))
                .kind(if self.approval_mode == ApprovalMode::FullSend { ButtonKind::Secondary } else { ButtonKind::Quiet })
                .enabled(!self.running && !self.changing_permissions)
                .size(ButtonSize::Sm).into_element(),
            if self.screenshots != Screenshots::Off {
                muted(self.screenshots.label().to_owned())
            } else {
                iced::widget::space().width(Length::Shrink).into()
            }
        ].spacing(8).align_y(iced::Alignment::Center)).padding([0, 16]);
        let skills: Element<'_, Message> = match &self.setup {
            Some(Ok(Setup { skills: Ok(skills), .. })) if !skills.is_empty() => {
                let full = self.chosen.len() >= agent_harness::MAX_TURN_SKILLS;
                let chips = skills.iter().map(|skill| {
                    let on = self.chosen.iter().any(|(name, _)| *name == skill.name);
                    enz_filter_chip(skill.name.clone(), on, Message::Skill(skill.name.clone()))
                        .aria_description(skill.description.clone())
                        .enabled(!self.running && (on || !full))
                        .into_element()
                });
                column![
                    muted(format!("Skills for each message ({} of up to {})", self.chosen.len(), agent_harness::MAX_TURN_SKILLS)),
                    iced::widget::row(chips).spacing(6).wrap(),
                ]
                .spacing(6)
                .padding(iced::Padding { top: 6.0, bottom: 0.0, left: 16.0, right: 16.0 })
                .into()
            }
            Some(Ok(Setup { skills: Err(error), .. })) => container(muted(tr!("shell-skills-are-unavailable-2", "arg0" => error.clone()))).padding([6, 16]).into(),
            _ => iced::widget::space().height(0).into(),
        };
        container(column![header, transcript, skills, permissions, mode, composer])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style { background: Some(palette.background.into()), ..Default::default() })
            .into()

        })
    }
    fn theme(&self) -> iced::Theme {
        self.tokens.iced_theme()
    }
}

pub(crate) fn run() -> iced::Result {
    let mut settings = enz_window_settings(ShellLayout::TitlebarContent);
    settings.size = iced::Size::new(560.0, 720.0);
    settings.platform_specific.application_id = "dev.enzui.Assistant".into();
    settings.exit_on_close_request = false;
    enzui_widgets::app_setup::run_with_services(
        iced::application(Assistant::new, Assistant::update, Assistant::view)
            .title(|_: &Assistant| tr("shell-assistant").to_owned())
            .window(settings)
            .subscription(|_: &Assistant| iced::window::close_requests().map(Message::Close))
            .theme(Assistant::theme),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
