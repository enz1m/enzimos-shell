use super::{
    capture::CaptureNotice,
    model::{Action, Archived, Closed, Key, Model, Notification, Policy, Posted},
    origin::Origin,
    payload::{Arguments, Draft, Urgency},
    session::Target,
};
use crate::compositor::Compositor;
use enzui_shell::i18n::tr;
use iced::futures::StreamExt;
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use zbus::{
    Connection, MatchRule, Message, MessageStream,
    fdo::RequestNameFlags,
    message::{Flags, Type},
};

pub const SERVICE: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
const XML: &str = r#"<node><interface name="org.freedesktop.Notifications">
<method name="Notify"><arg name="app_name" type="s" direction="in"/><arg name="replaces_id" type="u" direction="in"/><arg name="app_icon" type="s" direction="in"/><arg name="summary" type="s" direction="in"/><arg name="body" type="s" direction="in"/><arg name="actions" type="as" direction="in"/><arg name="hints" type="a{sv}" direction="in"/><arg name="expire_timeout" type="i" direction="in"/><arg name="id" type="u" direction="out"/></method>
<method name="CloseNotification"><arg name="id" type="u" direction="in"/></method>
<method name="GetCapabilities"><arg name="capabilities" type="as" direction="out"/></method>
<method name="GetServerInformation"><arg name="name" type="s" direction="out"/><arg name="vendor" type="s" direction="out"/><arg name="version" type="s" direction="out"/><arg name="spec_version" type="s" direction="out"/></method>
<signal name="NotificationClosed"><arg name="id" type="u"/><arg name="reason" type="u"/></signal>
<signal name="ActionInvoked"><arg name="id" type="u"/><arg name="action_key" type="s"/></signal>
<signal name="ActivationToken"><arg name="id" type="u"/><arg name="activation_token" type="s"/></signal>
<signal name="NotificationReplied"><arg name="id" type="u"/><arg name="text" type="s"/></signal>
</interface><interface name="org.freedesktop.DBus.Introspectable"><method name="Introspect"><arg name="xml_data" type="s" direction="out"/></method></interface></node>"#;
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
#[derive(Clone)]
pub struct Snapshot {
    pub epoch: u64,
    pub policy: Policy,
    pub active: Vec<Arc<Notification>>,
    pub history: Vec<Archived>,
    pub dropped_frames: u64,
}
impl Snapshot {
    pub fn empty(epoch: u64, policy: Policy) -> Self {
        Self {
            epoch,
            policy,
            active: vec![],
            history: vec![],
            dropped_frames: 0,
        }
    }
}
#[derive(Clone)]
pub enum Command {
    PostCapture(CaptureNotice),
    /// `token` is an XDG activation token from the click, handed to the app
    /// first (`ActivationToken`) so it may raise its window.
    Action {
        key: Key,
        action: Action,
        token: Option<String>,
    },
    Dismiss(Key),
    Read(Vec<Key>),
    HoldToasts(Vec<Key>),
    Policy(Policy),
    ClearHistory,
}
#[derive(Clone)]
pub enum Event {
    Ready(u64),
    PostedCapture(Result<(), String>),
    Action {
        key: Key,
        result: Result<(), String>,
    },
    Policy(Result<(), String>),
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotificationServerEvent")
    }
}
#[derive(Default)]
struct Gate(BTreeMap<String, (Instant, u16)>);
impl Gate {
    fn accepts(&mut self, key: &str, now: Instant) -> bool {
        self.0
            .retain(|_, (at, _)| now.duration_since(*at) < Duration::from_secs(1));
        if key.len() > 128 || (!self.0.contains_key(key) && self.0.len() >= 128) {
            return false;
        }
        let (_, count) = self.0.entry(key.into()).or_insert((now, 0));
        if *count >= 32 {
            return false;
        }
        *count += 1;
        true
    }
}
pub fn epoch() -> Result<u64, String> {
    NEXT_EPOCH
        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map_err(|_| tr("shell-notification-session-identities-exhausted").into())
}
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
async fn write(operation: impl Future<Output = zbus::Result<()>>) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(1), operation)
        .await
        .map_err(|_| tr("shell-notification-bus-write-timed-out"))?
        .map_err(|_| tr("shell-notification-bus-write-failed").into())
}
fn expects_reply(message: &Message) -> bool {
    !message
        .primary_header()
        .flags()
        .contains(Flags::NoReplyExpected)
}
async fn failure(
    connection: &Connection,
    message: &Message,
    kind: &str,
    reason: &str,
) -> Result<(), String> {
    if expects_reply(message) {
        write(connection.reply_error(
            &message.header(),
            format!("org.freedesktop.DBus.Error.{kind}"),
            &reason,
        ))
        .await?;
    }
    Ok(())
}
async fn close_signals(connection: &Connection, closed: Vec<Closed>) -> Result<(), String> {
    for closed in closed {
        if closed.origin.is_internal() {
            continue;
        }
        write(connection.emit_signal(
            Some(closed.origin.sender.as_str()),
            PATH,
            SERVICE,
            "NotificationClosed",
            &(closed.id, closed.reason as u32),
        ))
        .await?;
    }
    Ok(())
}
struct Service {
    epoch: u64,
    target: Target,
    root: PathBuf,
    connection: Connection,
    identity: Connection,
    model: Model,
    compositor: Option<Compositor>,
    sender_gate: Gate,
    app_gate: Gate,
}
impl Service {
    async fn post(&mut self, origin: Origin, draft: Draft) -> Result<Posted, String> {
        let policy = self.model.policy();
        let fullscreen = if policy.hold_fullscreen
            && draft.urgency != Urgency::Critical
            && policy.toast(&origin.principal, draft.urgency)
            && let Some(compositor) = &self.compositor
        {
            tokio::time::timeout(Duration::from_millis(150), compositor.desktop_fullscreen())
                .await
                .unwrap_or(false)
        } else {
            false
        };
        self.model
            .post(origin, draft, Instant::now(), wall_ms(), fullscreen)
    }

    async fn method(&mut self, message: Message) -> Result<(), String> {
        let header = message.header();
        let Some(sender) = header.sender().map(zbus::names::UniqueName::as_str) else {
            return Ok(());
        };
        if !self.sender_gate.accepts(sender, Instant::now()) {
            return failure(
                &self.connection,
                &message,
                "LimitsExceeded",
                tr("shell-notification-method-rate-exceeded"),
            )
            .await;
        }
        if header.path().map(zbus::zvariant::ObjectPath::as_str) != Some(PATH) {
            return failure(
                &self.connection,
                &message,
                "UnknownObject",
                tr("shell-unknown-notification-object"),
            )
            .await;
        }
        let interface = header.interface().map(zbus::names::InterfaceName::as_str);
        let member = header.member().map_or("", zbus::names::MemberName::as_str);
        if interface == Some("org.freedesktop.DBus.Introspectable") && member == "Introspect" {
            if message.body().deserialize::<()>().is_err() {
                return failure(
                    &self.connection,
                    &message,
                    "InvalidArgs",
                    tr("shell-expected-no-arguments"),
                )
                .await;
            }
            if expects_reply(&message) {
                write(self.connection.reply(&header, &XML)).await?;
            }
            return Ok(());
        }
        if interface != Some(SERVICE) {
            return failure(
                &self.connection,
                &message,
                "UnknownInterface",
                tr("shell-unknown-notification-interface"),
            )
            .await;
        }
        match member {
            "GetCapabilities" => {
                if message.body().deserialize::<()>().is_err() {
                    return failure(
                        &self.connection,
                        &message,
                        "InvalidArgs",
                        tr("shell-expected-no-arguments"),
                    )
                    .await;
                }
                if expects_reply(&message) {
                    write(self.connection.reply(
                        &header,
                        &vec![
                            "body",
                            "body-images",
                            "icon-static",
                            "actions",
                            "inline-reply",
                        ],
                    ))
                    .await?;
                }
            }
            "GetServerInformation" => {
                if message.body().deserialize::<()>().is_err() {
                    return failure(
                        &self.connection,
                        &message,
                        "InvalidArgs",
                        tr("shell-expected-no-arguments"),
                    )
                    .await;
                }
                if expects_reply(&message) {
                    write(self.connection.reply(
                        &header,
                        &("enzUI", "enzimos", env!("CARGO_PKG_VERSION"), "1.3"),
                    ))
                    .await?;
                }
            }
            "Notify" => {
                let draft = match message
                    .body()
                    .deserialize::<Arguments>()
                    .map_err(|_| tr("shell-invalid-notification-arguments").to_owned())
                    .and_then(Draft::parse)
                {
                    Ok(draft) => draft,
                    Err(error) => {
                        return failure(&self.connection, &message, "InvalidArgs", &error).await;
                    }
                };
                let origin = match Origin::capture(
                    &self.identity,
                    sender,
                    self.target.unit(),
                    &self.root,
                    draft.desktop_entry.as_deref().unwrap_or(&draft.app_name),
                )
                .await
                {
                    Ok(origin) => origin,
                    Err(error) => {
                        return failure(&self.connection, &message, "AccessDenied", &error).await;
                    }
                };
                if !self.app_gate.accepts(&origin.principal, Instant::now()) {
                    return failure(
                        &self.connection,
                        &message,
                        "LimitsExceeded",
                        tr("shell-application-notification-rate-exceeded"),
                    )
                    .await;
                }
                let posted = match self.post(origin, draft).await {
                    Ok(posted) => posted,
                    Err(error) => {
                        return failure(&self.connection, &message, "AccessDenied", &error).await;
                    }
                };
                if expects_reply(&message) {
                    write(self.connection.reply(&header, &posted.id)).await?;
                }
                close_signals(&self.connection, posted.closed).await?;
            }
            "CloseNotification" => {
                let Ok(id) = message.body().deserialize::<u32>() else {
                    return failure(
                        &self.connection,
                        &message,
                        "InvalidArgs",
                        tr("shell-invalid-notification-identity"),
                    )
                    .await;
                };
                let origin = match Origin::capture(
                    &self.identity,
                    sender,
                    self.target.unit(),
                    &self.root,
                    "",
                )
                .await
                {
                    Ok(origin) => origin,
                    Err(error) => {
                        return failure(&self.connection, &message, "AccessDenied", &error).await;
                    }
                };
                let closed = match self.model.close_from_app(&origin, id, wall_ms()) {
                    Ok(closed) => closed,
                    Err(error) => {
                        return failure(&self.connection, &message, "AccessDenied", &error).await;
                    }
                };
                if expects_reply(&message) {
                    write(self.connection.reply(&header, &())).await?;
                }
                close_signals(&self.connection, closed.into_iter().collect()).await?;
            }
            _ => {
                return failure(
                    &self.connection,
                    &message,
                    "UnknownMethod",
                    tr("shell-unknown-notification-method"),
                )
                .await;
            }
        }
        Ok(())
    }
    async fn command(
        &mut self,
        command: Command,
        events: &mpsc::Sender<Event>,
    ) -> Result<(), String> {
        match command {
            Command::PostCapture(capture) => {
                let posted = match Origin::internal_capture(capture.icon()) {
                    Ok(origin) => self.post(origin, capture.into_draft()).await,
                    Err(error) => Err(error),
                };
                let result = match posted {
                    Ok(posted) => close_signals(&self.connection, posted.closed).await,
                    Err(error) => Err(error),
                };
                events
                    .try_send(Event::PostedCapture(result))
                    .map_err(|_| "Notification consumer is unavailable or busy")?;
            }
            Command::Action { key, action, token } => {
                let result = match self.model.authorize(key, action) {
                    Err(error) => Err(error),
                    Ok(dispatch) if dispatch.origin.is_internal() => {
                        let result = match self
                            .model
                            .get(key)
                            .and_then(|entry| entry.draft.saved_file.as_ref())
                        {
                            Some(file) => file.open(token.as_deref()).await,
                            None => {
                                Err(tr("shell-notification-action-is-unavailable-or-invalid")
                                    .into())
                            }
                        };
                        if result.is_ok() {
                            let closed = self.model.action_delivered(key, wall_ms());
                            close_signals(&self.connection, closed.into_iter().collect()).await?;
                        }
                        result
                    }
                    Ok(dispatch) => match dispatch
                        .origin
                        .verify(&self.identity, self.target.unit())
                        .await
                    {
                        Err(error) => Err(error),
                        Ok(()) => match self.model.authorize(key, dispatch.action.clone()) {
                            Err(error) => Err(error),
                            Ok(dispatch) => {
                                match &dispatch.action {
                                    Action::Invoke(action) => {
                                        // The specification sends the token just before the action.
                                        if let Some(token) = token
                                            .as_deref()
                                            .filter(|t| !t.is_empty() && t.len() <= 4096)
                                        {
                                            write(self.connection.emit_signal(
                                                Some(dispatch.origin.sender.as_str()),
                                                PATH,
                                                SERVICE,
                                                "ActivationToken",
                                                &(dispatch.id, token),
                                            ))
                                            .await?;
                                        }
                                        write(self.connection.emit_signal(
                                            Some(dispatch.origin.sender.as_str()),
                                            PATH,
                                            SERVICE,
                                            "ActionInvoked",
                                            &(dispatch.id, action),
                                        ))
                                        .await?;
                                    }
                                    Action::Reply(text) => {
                                        write(self.connection.emit_signal(
                                            Some(dispatch.origin.sender.as_str()),
                                            PATH,
                                            SERVICE,
                                            "NotificationReplied",
                                            &(dispatch.id, text),
                                        ))
                                        .await?;
                                    }
                                }
                                let closed = self.model.action_delivered(key, wall_ms());
                                close_signals(&self.connection, closed.into_iter().collect())
                                    .await?;
                                Ok(())
                            }
                        },
                    },
                };
                events
                    .try_send(Event::Action { key, result })
                    .map_err(|_| "Notification consumer is unavailable or busy")?;
            }
            Command::Dismiss(key) => {
                let closed = self.model.dismiss(key, wall_ms());
                close_signals(&self.connection, closed.into_iter().collect()).await?;
            }
            Command::Read(keys) => {
                for key in keys.into_iter().take(64) {
                    self.model.mark_read(key);
                }
            }
            Command::HoldToasts(keys) => self.model.hold_toasts(&keys, Instant::now()),
            Command::Policy(policy) => {
                let result = match self.model.set_policy(policy, wall_ms()) {
                    Ok(closed) => {
                        close_signals(&self.connection, closed).await?;
                        Ok(())
                    }
                    Err(error) => Err(error),
                };
                events
                    .try_send(Event::Policy(result))
                    .map_err(|_| "Notification consumer is unavailable or busy")?;
            }
            Command::ClearHistory => self.model.clear_history(),
        }
        Ok(())
    }
    fn publish(&mut self, snapshots: &watch::Sender<Arc<Snapshot>>, dropped: u64) {
        if self.model.take_dirty() || snapshots.borrow().dropped_frames != dropped {
            let mut active: Vec<_> = self.model.active().cloned().collect();
            active.sort_by_key(|notification| std::cmp::Reverse(notification.key.revision));
            snapshots.send_replace(Arc::new(Snapshot {
                epoch: self.epoch,
                policy: self.model.policy().clone(),
                active,
                history: self.model.history().iter().cloned().collect(),
                dropped_frames: dropped,
            }));
        }
    }
}
pub struct Initial {
    pub root: PathBuf,
    pub policy: Policy,
    pub history: Vec<Archived>,
}
pub async fn run(
    root: PathBuf,
    policy: Policy,
    commands: mpsc::Receiver<Command>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    events: mpsc::Sender<Event>,
) -> Result<(), String> {
    run_initialized(
        Initial {
            root,
            policy,
            history: Vec::new(),
        },
        commands,
        snapshots,
        events,
    )
    .await
}
pub async fn run_initialized(
    initial: Initial,
    mut commands: mpsc::Receiver<Command>,
    snapshots: watch::Sender<Arc<Snapshot>>,
    events: mpsc::Sender<Event>,
) -> Result<(), String> {
    let Initial {
        root,
        policy,
        history,
    } = initial;
    if !root.is_absolute() {
        return Err(tr("shell-notification-source-root-must-be-explicit").into());
    }
    let epoch = snapshots.borrow().epoch;
    let mut model = Model::new(epoch, policy)?;
    model.restore_history(history, wall_ms())?;
    let target = tokio::task::spawn_blocking(Target::from_environment)
        .await
        .map_err(|_| tr("shell-notification-scope-reader-stopped"))??;
    let dropped = Arc::new(AtomicU64::new(0));
    let connection = target.connect(std::sync::Arc::clone(&dropped)).await?;
    let identity = target.connect(std::sync::Arc::clone(&dropped)).await?;
    let rule = MatchRule::builder().msg_type(Type::MethodCall).build();
    let mut incoming = MessageStream::for_match_rule(rule, &connection, Some(8))
        .await
        .map_err(|_| tr("shell-cannot-register-notification-methods"))?;
    let ownership = tokio::time::timeout(
        Duration::from_secs(2),
        connection.request_name_with_flags(SERVICE, RequestNameFlags::DoNotQueue.into()),
    )
    .await
    .map_err(|_| tr("shell-notification-name-request-timed-out"))?
    .map_err(|_| tr("shell-another-notification-service-already-owns-this-bus"))?;
    if ownership != zbus::fdo::RequestNameReply::PrimaryOwner {
        return Err(match notification_owner(&connection).await {
            Some(program) => held_by(&program),
            None => tr("shell-notification-service-did-not-acquire-exclusive-ownership").into(),
        });
    }
    let mut service = Service {
        epoch,
        target,
        root,
        connection,
        identity,
        model,
        compositor: Compositor::from_environment().ok(),
        sender_gate: Gate::default(),
        app_gate: Gate::default(),
    };
    events
        .try_send(Event::Ready(epoch))
        .map_err(|_| tr("shell-notification-consumer-is-unavailable"))?;
    service.publish(&snapshots, 0);
    // Expiry and toast motion tick every 250 ms while a notification shows;
    // with none, only history pruning remains, which a slow tick covers.
    let period = |service: &Service| {
        Duration::from_millis(if service.model.has_active() {
            250
        } else {
            5000
        })
    };
    let mut next = tokio::time::Instant::now() + period(&service);
    let mut ticked = false;
    let result = loop {
        let result = tokio::select! {
            biased;
            () = snapshots.closed() => break Ok(()),
            () = service.connection.closed() => break Err(tr("shell-notification-bus-connection-closed").into()),
            () = service.identity.closed() => break Err(tr("shell-notification-identity-connection-closed").into()),
            () = tokio::time::sleep_until(next) => { ticked = true; let closed = service.model.expire(Instant::now(), wall_ms()); close_signals(&service.connection, closed).await },
            command = commands.recv() => match command { Some(command) => service.command(command, &events).await, None => break Ok(()) },
            message = incoming.next() => match message { Some(Ok(message)) => service.method(message).await, _ => break Err(tr("shell-notification-method-stream-ended").into()) },
        };
        if let Err(error) = result {
            break Err(error);
        }
        service.publish(&snapshots, dropped.load(Ordering::Relaxed));
        let soon = tokio::time::Instant::now() + period(&service);
        next = if std::mem::take(&mut ticked) {
            soon
        } else {
            next.min(soon)
        };
    };
    drop(incoming);
    let _ = tokio::time::timeout(Duration::from_secs(1), service.connection.close()).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), service.identity.close()).await;
    result
}

/// The program that holds the notifications name (another daemon such as
/// mako, dunst or a desktop's own shell), from its process name.
async fn notification_owner(connection: &zbus::Connection) -> Option<String> {
    let bus = zbus::fdo::DBusProxy::new(connection).await.ok()?;
    let name = zbus::names::BusName::try_from(SERVICE).ok()?;
    let pid = tokio::time::timeout(
        Duration::from_secs(1),
        bus.get_connection_unix_process_id(name),
    )
    .await
    .ok()?
    .ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim().to_owned()).filter(|program| !program.is_empty())
}

/// What the notification center says when `program` holds notifications.
fn held_by(program: &str) -> String {
    enzui_shell::tr!("shell-notifications-are-handled-by-another-program", "arg0" => program.to_owned())
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    /// When another program holds notifications, the notification center names it.
    /// Sabotage: return None from notification_owner.
    #[tokio::test]
    async fn the_program_holding_notifications_is_named() {
        let bus = crate::test_bus::Bus::start();
        let holder = bus.client().await;
        holder.request_name(SERVICE).await.unwrap();
        let shell = bus.client().await;
        let expected = std::fs::read_to_string("/proc/self/comm")
            .unwrap()
            .trim()
            .to_owned();
        assert_eq!(
            notification_owner(&shell).await.as_deref(),
            Some(expected.as_str())
        );
        assert!(held_by(&expected).contains(&expected));
    }
}
