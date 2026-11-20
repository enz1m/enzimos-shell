//! Inspects processes, ports and containers through the desktop backend.
use enzui_core::theme_file::integration::{ThemeEvent, ThemeIntegration};
use enzui_shell::dev_monitor::{
    Client, Container, ContainerId, Detail, Launch, Operation, Options, ProcessId, RefreshGate,
    RefreshTicket, ShortcutKind, Snapshot, Target, UrlLaunch,
};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_widgets::{
    app_shell::{ShellLayout, enz_app_shell, enz_window_settings},
    button::{ButtonKind, ButtonSize, enz_button},
    checkbox::enz_checkbox,
    confirm_dialog::{ConfirmDialogEvent, enz_confirm_dialog},
    modal_host::{ModalFocusLifecycle, ModalHostEvent, enz_modal_host},
    scroll_view::enz_scroll_view,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
    title_bar::enz_title_bar,
    title_bar_gesture::WindowCommand,
    view_switcher::{SwitcherPage, enz_view_switcher},
};
use iced::widget::{container, row};
use iced::{Element, Length, Subscription, Task, window};
use std::time::Duration;
#[path = "dev_tools/browse.rs"]
mod browse;
#[path = "dev_tools/observed.rs"]
mod observed;
#[path = "dev_tools/view.rs"]
mod view;
use browse::{Section, page_range};

#[derive(Clone, Debug)]
enum Message {
    Opened(window::Id),
    Close,
    Window(WindowCommand),
    Tick,
    Snapshot(RefreshTicket, Result<(Client, Snapshot), String>),
    Containers(usize),
    System(bool),
    Udp(bool),
    Review(Target, Operation, String, String, Client),
    ForceReview,
    Confirm(ConfirmDialogEvent),
    Modal(ModalHostEvent),
    Done(Result<String, String>),
    PodmanApi,
    PodmanApiDone(Result<(), String>),
    Logs(Target, Client),
    Read(u64, Result<String, String>),
    Inspect(ProcessId, Client),
    Inspected(u64, ProcessId, Result<Detail, String>),
    PortQuery(String),
    Lookup,
    LookedUp(u64, u16, Result<(Client, Snapshot), String>),
    ClearDetail,
    Project(Option<String>),
    Page(Section, usize),
    ContainerDetails(ContainerId, Box<Container>),
    CloseContainerDetails,
    Shortcut(Target, ShortcutKind, Client),
    ShortcutToken(
        u64,
        window::Id,
        Target,
        ShortcutKind,
        Client,
        Option<String>,
    ),
    ShortcutReady(u64, window::Id, Option<String>, Result<Launch, String>),
    Launched(u64, Result<(), String>),
    OpenUrl(UrlLaunch),
    UrlToken(u64, window::Id, UrlLaunch, Option<String>),
    Copy(String),
    Copied(Result<(), String>),
    Theme(Box<ThemeEvent>),
}

struct Review {
    target: Target,
    operation: Operation,
    label: String,
    client: Client,
    focus: ModalFocusLifecycle,
    state: enzui_core::confirm::ConfirmState,
}

struct App {
    themes: ThemeIntegration,
    window: Option<window::Id>,
    gate: RefreshGate,
    options: Options,
    refresh: Option<iced::task::Handle>,
    client: Option<Client>,
    snapshot: Option<Snapshot>,
    error: Option<String>,
    status: Option<String>,
    logs: Option<String>,
    detail: Option<Detail>,
    port_query: String,
    project: Option<String>,
    pages: [usize; 6],
    container_detail: Option<(ContainerId, Container)>,
    lookup: Option<(u16, Client, Snapshot)>,
    explicit: Option<iced::task::Handle>,
    explicit_pending: bool,
    read_epoch: u64,
    review: Option<Review>,
    restore_after_action: Option<ModalFocusLifecycle>,
    force: Option<(Target, Client, String)>,
    busy: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (themes, boot) = ThemeIntegration::boot();
        (
            Self {
                themes,
                window: None,
                gate: RefreshGate::default(),
                options: Options::default(),
                refresh: None,
                client: None,
                snapshot: None,
                error: None,
                status: None,
                logs: None,
                detail: None,
                port_query: String::new(),
                project: None,
                pages: [0; 6],
                container_detail: None,
                lookup: None,
                explicit: None,
                explicit_pending: false,
                read_epoch: 0,
                review: None,
                restore_after_action: None,
                force: None,
                busy: false,
            },
            boot.map(|event| Message::Theme(Box::new(event))),
        )
    }

    fn refresh(&mut self) -> Task<Message> {
        let Some(ticket) = self.gate.begin(self.options) else {
            return Task::none();
        };
        let client = self.client.clone();
        let (task, handle) = Task::perform(
            async move {
                let client = match client {
                    Some(client) => client,
                    None => Client::session().await?,
                };
                let snapshot = client.snapshot(ticket.options).await?;
                Ok((client, snapshot))
            },
            move |result| Message::Snapshot(ticket, result),
        )
        .abortable();
        self.refresh = Some(handle);
        task
    }

    fn reset_refresh(&mut self) -> Task<Message> {
        self.gate.close();
        if let Some(task) = self.refresh.take() {
            task.abort();
        }
        self.snapshot = None;
        self.pages = [0; 6];
        self.container_detail = None;
        self.logs = None;
        self.detail = None;
        self.lookup = None;
        self.explicit_pending = false;
        if let Some(task) = self.explicit.take() {
            task.abort();
        }
        self.read_epoch = self.read_epoch.wrapping_add(1);
        self.error = None;
        if self.window.is_some() {
            self.gate.open();
        }
        self.refresh()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Opened(id) => {
                self.window = Some(id);
                self.gate.open();
                self.refresh()
            }
            Message::Close | Message::Window(WindowCommand::Close) => {
                self.gate.close();
                self.window = None;
                if let Some(task) = self.refresh.take() {
                    task.abort();
                }
                self.review = None;
                self.container_detail = None;
                self.restore_after_action = None;
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                self.read_epoch = self.read_epoch.wrapping_add(1);
                iced::exit()
            }
            Message::Window(command) => self
                .window
                .map_or_else(Task::none, |id| command.into_task(id)),
            Message::Tick => Task::batch([
                self.refresh(),
                window::invalidate(window::Invalidation::none()),
            ]),
            Message::Snapshot(ticket, result) => {
                if self.gate.finish(ticket, self.options) {
                    self.refresh = None;
                    match result {
                        Ok((client, snapshot)) => {
                            self.client = Some(client);
                            // Force stop is offered only for an identity still on screen.
                            if !self.busy && self.force.as_ref().is_some_and(|(target, _, _)| matches!(target, Target::Process(id) if !snapshot.shows_process(*id))) {
                                self.force = None;
                            }
                            self.snapshot = Some(snapshot);
                            self.error = None;
                        }
                        Err(error) => {
                            self.error = Some(error);
                            self.snapshot = None;
                            self.client = None;
                        }
                    }
                    Task::none()
                } else {
                    Task::none()
                }
            }
            Message::Containers(index) => {
                self.options.containers = index == 1;
                self.reset_refresh()
            }
            Message::System(value) => {
                self.options.system = value;
                self.reset_refresh()
            }
            Message::Udp(value) => {
                self.options.udp = value;
                self.reset_refresh()
            }
            Message::Review(target, operation, label, invoker, client)
                if !self.busy && self.review.is_none() && self.window.is_some() =>
            {
                let (focus, operation_task) = ModalFocusLifecycle::open_at(
                    "dev-tools-confirm",
                    Some(invoker.into()),
                    "dev-tools-cancel".into(),
                );
                self.review = Some(Review {
                    target,
                    operation,
                    label,
                    client,
                    focus,
                    state: enzui_core::confirm::ConfirmState::new(None),
                });
                iced::advanced::widget::operate(operation_task)
            }
            Message::Review(..) => Task::none(),
            Message::ForceReview if !self.busy && self.review.is_none() => {
                if let Some((target, client, label)) = &self.force {
                    let (focus, op) = ModalFocusLifecycle::open_at(
                        "dev-tools-confirm",
                        Some("dev-tools-force".into()),
                        "dev-tools-cancel".into(),
                    );
                    self.review = Some(Review {
                        target: target.clone(),
                        operation: Operation::ForceStop,
                        label: label.clone(),
                        client: client.clone(),
                        focus,
                        state: enzui_core::confirm::ConfirmState::new(None),
                    });
                    iced::advanced::widget::operate(op)
                } else {
                    Task::none()
                }
            }
            Message::ForceReview => Task::none(),
            Message::Confirm(ConfirmDialogEvent::Resolved(
                enzui_core::modal::DismissCause::DefaultAction,
            )) if !self.busy => {
                if let Some(review) = self.review.take() {
                    self.restore_after_action = Some(review.focus.clone());
                    self.busy = true;
                    if review.operation == Operation::Stop
                        && matches!(review.target, Target::Process(_))
                    {
                        self.force = Some((
                            review.target.clone(),
                            review.client.clone(),
                            review.label.clone(),
                        ));
                    }
                    self.status = Some(
                        tr!("shell-request-in-progress", "arg0" => review.operation.label().to_owned()),
                    );
                    Task::perform(
                        async move { review.client.action(review.target, review.operation).await },
                        Message::Done,
                    )
                } else {
                    Task::none()
                }
            }
            Message::Confirm(event) => match event {
                ConfirmDialogEvent::Resolved(cause) => {
                    self.update(Message::Modal(ModalHostEvent::Dismissed(cause)))
                }
                ConfirmDialogEvent::FocusNext => {
                    self.update(Message::Modal(ModalHostEvent::FocusNext))
                }
                ConfirmDialogEvent::FocusPrevious => {
                    self.update(Message::Modal(ModalHostEvent::FocusPrevious))
                }
                ConfirmDialogEvent::CheckboxToggled(_) => Task::none(),
            },
            Message::Modal(event) => {
                if let Some(review) = self.review.as_ref() {
                    let outcome = review.focus.reduce(event);
                    if outcome.dismissed().is_some() {
                        self.review = None;
                    }
                    outcome.into_task()
                } else {
                    Task::none()
                }
            }
            Message::PodmanApi if !self.busy => {
                self.busy = true;
                self.status = Some(tr("shell-turning-on-the-podman-api").into());
                Task::perform(
                    enzui_shell::dev_monitor::podman::enable_api(),
                    Message::PodmanApiDone,
                )
            }
            Message::PodmanApi => Task::none(),
            Message::PodmanApiDone(result) => {
                self.busy = false;
                self.status = Some(match result {
                    Ok(()) => tr("shell-podman-api-turned-on-for-your-account-containers-appear-on-the-next-refresh").into(),
                    Err(error) => tr!("shell-could-not-turn-on-the-podman-api", "arg0" => error),
                });
                self.refresh()
            }
            Message::Done(result) => {
                self.busy = false;
                self.status =
                    Some(result.unwrap_or_else(|e| tr!("shell-request-failed", "arg0" => e)));
                let focus = self
                    .restore_after_action
                    .take()
                    .map_or_else(Task::none, |focus| {
                        focus
                            .reduce(ModalHostEvent::Dismissed(
                                enzui_core::modal::DismissCause::DefaultAction,
                            ))
                            .into_task()
                    });
                Task::batch([focus, self.refresh()])
            }
            Message::Shortcut(target, kind, client)
                if !self.busy && !self.explicit_pending && self.window.is_some() =>
            {
                if enzui_shell::desktop_session::mode().nested()
                    || kind == ShortcutKind::Terminal
                        && enzui_shell::desktop_session::mode()
                            != enzui_shell::desktop_session::Mode::Installed
                {
                    self.status =
                        Some(tr("shell-developer-launches-are-not-enabled-in-this-preview").into());
                    return Task::none();
                }
                let window = self.window.expect("the event is scoped to an open window");
                self.explicit_pending = true;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                let epoch = self.read_epoch;
                window::request_activation_token(window).map(move |result| {
                    Message::ShortcutToken(
                        epoch,
                        window,
                        target.clone(),
                        kind,
                        client.clone(),
                        result.ok().map(iced::window::ActivationToken::into_raw),
                    )
                })
            }
            Message::Shortcut(..) => Task::none(),
            Message::ShortcutToken(epoch, window, target, kind, client, token)
                if epoch == self.read_epoch
                    && self.window == Some(window)
                    && self.explicit_pending =>
            {
                let (task, handle) = Task::perform(
                    async move {
                        client
                            .shortcuts(target)
                            .await
                            .map(|shortcuts| shortcuts.launch(kind))
                    },
                    move |result| Message::ShortcutReady(epoch, window, token, result),
                )
                .abortable();
                self.explicit = Some(handle);
                task
            }
            Message::ShortcutToken(..) => Task::none(),
            Message::ShortcutReady(epoch, window, token, result)
                if epoch == self.read_epoch
                    && self.window == Some(window)
                    && self.explicit_pending =>
            {
                self.explicit = None;
                match result {
                    Ok(launch) => Task::done(Message::Launched(epoch, launch.start(token))),
                    Err(error) => {
                        self.explicit_pending = false;
                        self.status =
                            Some(tr!("shell-developer-shortcut-unavailable", "arg0" => error));
                        Task::none()
                    }
                }
            }
            Message::ShortcutReady(..) => Task::none(),
            Message::Launched(epoch, result)
                if epoch == self.read_epoch && self.window.is_some() =>
            {
                self.explicit_pending = false;
                self.status = Some(result.map_or_else(|error| tr!("shell-launch-failed", "arg0" => error), |()| tr("shell-application-launch-requested-no-success-beyond-process-start-is-implied").into()));
                Task::none()
            }
            Message::Launched(..) => Task::none(),
            Message::Inspect(id, client)
                if !self.busy && !self.explicit_pending && self.window.is_some() =>
            {
                self.detail = None;
                self.explicit_pending = true;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                let epoch = self.read_epoch;
                let (task, handle) =
                    Task::perform(async move { client.inspect(id).await }, move |result| {
                        Message::Inspected(epoch, id, result)
                    })
                    .abortable();
                self.explicit = Some(handle);
                task
            }
            Message::Inspect(..) => Task::none(),
            Message::Inspected(epoch, id, result)
                if epoch == self.read_epoch && self.window.is_some() && self.explicit_pending =>
            {
                self.explicit = None;
                self.explicit_pending = false;
                match result {
                    Ok(detail) if detail.identity == id => self.detail = Some(detail),
                    Ok(_) => self.status = Some(tr("shell-process-detail-identity-changed").into()),
                    Err(error) => {
                        self.status = Some(tr!("shell-process-inspection-failed", "arg0" => error));
                    }
                }
                Task::none()
            }
            Message::Inspected(..) => Task::none(),
            Message::ContainerDetails(id, detail) => {
                self.read_epoch = self.read_epoch.wrapping_add(1);
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                self.pages[Section::ContainerPorts.index()] = 0;
                self.pages[Section::ContainerMounts.index()] = 0;
                self.container_detail = Some((id, *detail));
                iced::widget::operation::scroll_to(
                    "dev-tools-scroll",
                    iced::widget::scrollable::AbsoluteOffset::<f32>::default(),
                    iced::widget::operation::Animation::Instant,
                )
            }
            Message::CloseContainerDetails => {
                self.container_detail = None;
                Task::none()
            }
            Message::Page(section, page) => {
                self.pages[section.index()] = page;
                self.detail = None;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                iced::widget::operation::scroll_to(
                    "dev-tools-scroll",
                    iced::widget::scrollable::AbsoluteOffset::<f32>::default(),
                    iced::widget::operation::Animation::Instant,
                )
            }
            Message::Project(key) => {
                self.container_detail = None;
                self.pages[Section::Ports.index()] = 0;
                self.pages[Section::Containers.index()] = 0;
                self.project = key.filter(|key| {
                    self.snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.project(key).is_some())
                });
                self.detail = None;
                self.lookup = None;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                Task::none()
            }
            Message::PortQuery(query) => {
                self.pages[Section::Lookup.index()] = 0;
                self.port_query = query.chars().filter(char::is_ascii_digit).take(5).collect();
                self.lookup = None;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                Task::none()
            }
            Message::Lookup if !self.busy && !self.explicit_pending && self.window.is_some() => {
                let Some(port) = self.port_query.parse::<u16>().ok().filter(|port| *port > 0)
                else {
                    return Task::none();
                };
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                self.lookup = None;
                self.explicit_pending = true;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                let epoch = self.read_epoch;
                let (task, handle) = Task::perform(
                    async move {
                        client
                            .lookup(port)
                            .await
                            .map(|lookup| (client, lookup.snapshot()))
                    },
                    move |result| Message::LookedUp(epoch, port, result),
                )
                .abortable();
                self.explicit = Some(handle);
                task
            }
            Message::Lookup => Task::none(),
            Message::LookedUp(epoch, port, result)
                if epoch == self.read_epoch
                    && self.window.is_some()
                    && self.explicit_pending
                    && self.port_query.parse::<u16>().ok() == Some(port) =>
            {
                self.explicit = None;
                self.explicit_pending = false;
                match result {
                    Ok((client, snapshot)) => self.lookup = Some((port, client, snapshot)),
                    Err(error) => {
                        self.status = Some(tr!("shell-port-lookup-failed", "arg0" => error));
                    }
                }
                Task::none()
            }
            Message::LookedUp(..) => Task::none(),
            Message::ClearDetail => {
                self.pages[Section::Lookup.index()] = 0;
                self.detail = None;
                self.lookup = None;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                self.explicit_pending = false;
                if let Some(task) = self.explicit.take() {
                    task.abort();
                }
                Task::none()
            }
            Message::Logs(target, client)
                if !self.busy && !self.explicit_pending && self.window.is_some() =>
            {
                self.explicit_pending = true;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                let epoch = self.read_epoch;
                let (task, handle) =
                    Task::perform(async move { client.logs(target).await }, move |result| {
                        Message::Read(epoch, result)
                    })
                    .abortable();
                self.explicit = Some(handle);
                task
            }
            Message::Logs(..) => Task::none(),
            Message::Read(epoch, result) if epoch == self.read_epoch && self.window.is_some() => {
                self.explicit = None;
                self.explicit_pending = false;
                self.logs =
                    Some(result.unwrap_or_else(|e| tr!("shell-logs-unavailable", "arg0" => e)));
                Task::none()
            }
            Message::Read(..) => Task::none(),
            Message::OpenUrl(launch)
                if !self.busy
                    && !self.explicit_pending
                    && self.window.is_some()
                    && !enzui_shell::desktop_session::mode().nested() =>
            {
                self.explicit_pending = true;
                self.read_epoch = self.read_epoch.wrapping_add(1);
                let epoch = self.read_epoch;
                let window = self.window.expect("the event is scoped to an open window");
                window::request_activation_token(window).map(move |result| {
                    Message::UrlToken(
                        epoch,
                        window,
                        launch.clone(),
                        result.ok().map(iced::window::ActivationToken::into_raw),
                    )
                })
            }
            Message::OpenUrl(..) => Task::none(),
            Message::UrlToken(epoch, window, launch, token)
                if epoch == self.read_epoch
                    && self.window == Some(window)
                    && self.explicit_pending =>
            {
                self.explicit_pending = false;
                self.status = Some(launch.start(token).map_or_else(|error| tr!("shell-url-launch-failed", "arg0" => error), |()| tr("shell-url-handler-launch-requested-http-service-and-reachability-were-not-tested").into()));
                Task::none()
            }
            Message::UrlToken(..) => Task::none(),
            Message::Copy(text) => iced::clipboard::write(text)
                .map(|result| Message::Copied(result.map_err(|e| format!("{e:?}")))),
            Message::Copied(result) => {
                self.status = Some(result.map_or_else(
                    |e| tr!("shell-copy-failed", "arg0" => e),
                    |()| tr("shell-copied-to-clipboard").into(),
                ));
                Task::none()
            }
            Message::Theme(event) => self
                .themes
                .update(*event)
                .map(|event| Message::Theme(Box::new(event))),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.themes
                .subscription()
                .map(|event| Message::Theme(Box::new(event))),
            window::open_events().map(Message::Opened),
            window::close_requests().map(|_| Message::Close),
            if self.window.is_some() {
                iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick)
            } else {
                Subscription::none()
            },
        ])
    }
}

fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => tr("shell-configured"),
        Some(false) => tr("shell-not-configured"),
        None => tr("shell-unknown"),
    }
}

fn cpu(value: Option<f64>) -> String {
    value
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map_or_else(|| tr("shell-unknown").into(), |n| format!("{n:.1}%"))
}

fn memory(value: Option<u64>) -> String {
    value.map_or_else(
        || tr("shell-unknown").into(),
        |n| tr!("shell-mib", "arg0" => enzui_shell::i18n::number(format!("{:.1}", n as f64 / 1048576.0))),
    )
}

#[cfg(test)]
#[path = "dev_tools/tests.rs"]
mod tests;

fn main() -> iced::Result {
    i18n::initialize();
    let mut settings = enz_window_settings(ShellLayout::TitlebarContent);
    settings.size = iced::Size::new(860.0, 640.0);
    settings.platform_specific.application_id = "dev.enzui.DevTools".into();
    enzui_widgets::app_setup::run_with_services(
        iced::application(App::new, App::update, App::view)
            .title(tr("shell-dev-tools"))
            .window(settings)
            .theme(|app: &App| app.themes.theme().iced_theme())
            .subscription(App::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    )
}
