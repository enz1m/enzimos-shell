//! Exercises notification ownership and action delivery on a private session bus.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]
use enzui_shell::i18n;
use enzui_shell::notifications::{
    model::{Action, Key, Policy, Rule},
    payload::Arguments,
    server::{self, Command, Event, PATH, SERVICE, Snapshot},
    session::Target,
};
use iced::futures::StreamExt;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use zbus::{
    Connection, MatchRule, Message, MessageStream,
    message::{Flags, Type},
    zvariant::{OwnedValue, Value},
};

async fn client(target: &Target) -> Result<Connection, String> {
    let (stream, guid) = target.stream().await?;
    let connection = tokio::time::timeout(
        Duration::from_secs(5),
        zbus::connection::Builder::unix_stream(stream)
            .max_queued(16)
            .method_timeout(Duration::from_secs(2))
            .build(),
    )
    .await
    .map_err(|_| "Fixture handshake timed out")?
    .map_err(|_| "Fixture handshake failed")?;
    if guid.is_some_and(|guid| {
        !connection
            .server_guid()
            .as_str()
            .eq_ignore_ascii_case(&guid)
    }) {
        return Err("Fixture bus identity changed".into());
    }
    Ok(connection)
}
fn arguments(replaces: u32) -> Arguments {
    (
        "Claimed fixture name".into(),
        replaces,
        "/fixture/icon-must-not-be-read".into(),
        "Disposable notification".into(),
        "Exact body: café ✓".into(),
        vec![
            "open".into(),
            "Open".into(),
            "inline-reply".into(),
            "Reply".into(),
        ],
        HashMap::new(),
        -1,
    )
}
async fn notify(connection: &Connection, arguments: &Arguments) -> Result<u32, String> {
    connection
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify", arguments)
        .await
        .map_err(|_| "Fixture notification call failed")?
        .body()
        .deserialize()
        .map_err(|_| "Fixture notification reply is invalid".into())
}
async fn signals(connection: &Connection) -> Result<MessageStream, String> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(SERVICE)
        .map_err(|_| "Fixture interface is invalid")?
        .path(PATH)
        .map_err(|_| "Fixture path is invalid")?
        .build();
    MessageStream::for_match_rule(rule, connection, Some(16))
        .await
        .map_err(|_| "Fixture signal subscription failed".into())
}
async fn signal(stream: &mut MessageStream, member: &str) -> Result<Message, String> {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(message) = stream.next().await {
            let message = message.map_err(|_| "Fixture signal stream failed")?;
            if message
                .header()
                .member()
                .is_some_and(|name| name.as_str() == member)
            {
                return Ok(message);
            }
        }
        Err("Fixture signal stream ended".to_owned())
    })
    .await
    .map_err(|_| "Fixture signal timed out")?
}
async fn snapshot(
    receiver: &mut watch::Receiver<Arc<Snapshot>>,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Result<Arc<Snapshot>, String> {
    tokio::time::timeout(
        Duration::from_secs(4),
        receiver.wait_for(|value| predicate(value)),
    )
    .await
    .map_err(|_| "Fixture snapshot timed out")?
    .map(|value| value.clone())
    .map_err(|_| "Fixture snapshot channel closed".into())
}
async fn event(receiver: &mut mpsc::Receiver<Event>) -> Result<Event, String> {
    tokio::time::timeout(Duration::from_secs(4), receiver.recv())
        .await
        .map_err(|_| "Fixture event timed out")?
        .ok_or_else(|| "Fixture event channel closed".into())
}
async fn policy(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    policy: Policy,
) -> Result<(), String> {
    commands
        .send(Command::Policy(policy))
        .await
        .map_err(|_| "Fixture command channel closed")?;
    match event(events).await? {
        Event::Policy(result) => result,
        _ => Err("Unexpected fixture policy event".into()),
    }
}
async fn action(
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    key: Key,
    action: Action,
) -> Result<bool, String> {
    commands
        .send(Command::Action {
            key,
            action,
            token: None,
        })
        .await
        .map_err(|_| "Fixture command channel closed")?;
    match event(events).await? {
        Event::Action { key: got, result } if key == got => Ok(result.is_ok()),
        _ => Err("Unexpected fixture action event".into()),
    }
}
async fn campaign(
    target: &Target,
    commands: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
) -> Result<(), String> {
    if !matches!(event(events).await?, Event::Ready(_)) {
        return Err("Notification fixture did not become ready".into());
    }
    let first = client(target).await?;
    let second = client(target).await?;
    let mut first_signals = signals(&first).await?;
    let mut second_signals = signals(&second).await?;
    let capabilities: Vec<String> = first
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &())
        .await
        .map_err(|_| "Capabilities call failed")?
        .body()
        .deserialize()
        .map_err(|_| "Capabilities signature differs")?;
    if !capabilities.iter().any(|value| value == "inline-reply")
        || capabilities
            .iter()
            .any(|value| matches!(value.as_str(), "body-markup" | "persistence" | "sound"))
    {
        return Err("Capability promises differ".into());
    }
    let info: (String, String, String, String) = first
        .call_method(
            Some(SERVICE),
            PATH,
            Some(SERVICE),
            "GetServerInformation",
            &(),
        )
        .await
        .map_err(|_| "Information call failed")?
        .body()
        .deserialize()
        .map_err(|_| "Information signature differs")?;
    if info.0 != "enzUI" || info.3 != "1.3" {
        return Err("Server information differs".into());
    }
    let id = notify(&first, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == id)
    })
    .await?;
    let original = current
        .active
        .iter()
        .find(|notification| notification.key.id == id)
        .ok_or("Fixture notification missing")?;
    let old_key = original.key;
    let principal = original.origin.principal.clone();
    if original.origin.label == original.draft.app_name
        || original.draft.body != "Exact body: café ✓"
    {
        return Err("Notification origin or body differs".into());
    }
    if notify(&second, &arguments(id)).await.is_ok() {
        return Err("Peer replacement was accepted".into());
    }
    if second
        .call_method(Some(SERVICE), PATH, Some(SERVICE), "CloseNotification", &id)
        .await
        .is_ok()
    {
        return Err("Peer close was accepted".into());
    }
    let mut replacement = arguments(id);
    replacement.3 = "Replacement fixture".into();
    if notify(&first, &replacement).await? != id {
        return Err("Replacement changed the notification identity".into());
    }
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == id && notification.key != old_key)
    })
    .await?;
    let key = current
        .active
        .iter()
        .find(|notification| notification.key.id == id)
        .ok_or("Replacement fixture missing")?
        .key;
    if action(commands, events, old_key, Action::Invoke("open".into())).await? {
        return Err("Stale action was accepted".into());
    }
    if !action(commands, events, key, Action::Invoke("open".into())).await? {
        return Err("Current action was rejected".into());
    }
    let invoked: (u32, String) = signal(&mut first_signals, "ActionInvoked")
        .await?
        .body()
        .deserialize()
        .map_err(|_| "Action signal signature differs")?;
    if invoked != (id, "open".into()) {
        return Err("Action signal differs".into());
    }
    let closed: (u32, u32) = signal(&mut first_signals, "NotificationClosed")
        .await?
        .body()
        .deserialize()
        .map_err(|_| "Close signal signature differs")?;
    if closed != (id, 2) {
        return Err("Action close reason differs".into());
    }
    let reply_id = notify(&first, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == reply_id)
    })
    .await?;
    let reply_key = current
        .active
        .iter()
        .find(|notification| notification.key.id == reply_id)
        .ok_or("Reply fixture missing")?
        .key;
    let reply = "Exact reply\n👩\u{200d}💻 café".to_owned();
    if !action(commands, events, reply_key, Action::Reply(reply.clone())).await? {
        return Err("Inline reply was rejected".into());
    }
    let replied: (u32, String) = signal(&mut first_signals, "NotificationReplied")
        .await?
        .body()
        .deserialize()
        .map_err(|_| "Reply signal signature differs")?;
    if replied != (reply_id, reply) {
        return Err("Inline reply text differs".into());
    }
    if tokio::time::timeout(Duration::from_millis(250), second_signals.next())
        .await
        .is_ok()
    {
        return Err("Private action or reply reached another sender".into());
    }
    let mut image = arguments(0);
    image.6.insert(
        "image-data".into(),
        OwnedValue::try_from(Value::from((
            2i32,
            1i32,
            8i32,
            true,
            8i32,
            4i32,
            vec![9u8, 20, 100, 255, 50, 90, 100, 255],
        )))
        .map_err(|_| "Image fixture could not be encoded")?,
    );
    let image_id = notify(&first, &image).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == image_id)
    })
    .await?;
    let image = current
        .active
        .iter()
        .find(|notification| notification.key.id == image_id)
        .and_then(|notification| notification.draft.image.as_ref())
        .ok_or("Image fixture missing")?;
    if image.pixels() != (&[9, 20, 100, 255, 50, 90, 100, 255][..], 2, 1) {
        return Err("Image fixture pixels differ".into());
    }
    let mut preferences = Policy {
        history: true,
        dnd: true,
        ..Policy::default()
    };
    policy(commands, events, preferences.clone()).await?;
    let muted = notify(&first, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == muted)
    })
    .await?;
    if current
        .active
        .iter()
        .any(|notification| notification.toast.visible(Instant::now()))
    {
        return Err("DND retained a toast".into());
    }
    preferences.rules.insert(principal.clone(), Rule::Block);
    policy(commands, events, preferences.clone()).await?;
    let blocked = notify(&first, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value.policy.rule(&principal) == Rule::Block
    })
    .await?;
    if current.active.iter().any(|notification| {
        notification.key.id == blocked || notification.origin.principal == principal
    }) {
        return Err("Blocked app retained a notification".into());
    }
    preferences.rules.insert(principal.clone(), Rule::Priority);
    policy(commands, events, preferences.clone()).await?;
    let allowed = notify(&first, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == allowed)
    })
    .await?;
    if current
        .active
        .iter()
        .find(|notification| notification.key.id == allowed)
        .is_none_or(|notification| !notification.toast.visible(Instant::now()))
    {
        return Err("Priority app did not bypass DND".into());
    }
    let mut transient = arguments(0);
    transient.3 = "Transient fixture".into();
    transient
        .6
        .insert("transient".into(), OwnedValue::from(true));
    transient.7 = 100;
    let transient_id = notify(&first, &transient).await?;
    snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == transient_id)
    })
    .await?;
    let current = snapshot(snapshots, |value| {
        !value
            .active
            .iter()
            .any(|notification| notification.key.id == transient_id)
    })
    .await?;
    if current
        .history
        .iter()
        .any(|entry| entry.summary == "Transient fixture")
    {
        return Err("Transient notification entered history".into());
    }
    let mut large = arguments(0);
    large.4 = "x".repeat(3 * 1024 * 1024);
    let message = Message::method_call(PATH, "Notify")
        .map_err(|_| "Oversize fixture failed")?
        .destination(SERVICE)
        .map_err(|_| "Oversize destination failed")?
        .interface(SERVICE)
        .map_err(|_| "Oversize interface failed")?
        .sender(first.unique_name().ok_or("Fixture sender missing")?)
        .map_err(|_| "Oversize sender failed")?
        .with_flags(Flags::NoReplyExpected)
        .map_err(|_| "Oversize flags failed")?
        .build(&large)
        .map_err(|_| "Oversize encoding failed")?;
    tokio::time::timeout(Duration::from_secs(3), first.send(&message))
        .await
        .map_err(|_| "Oversize send timed out")?
        .map_err(|_| "Oversize send failed")?;
    drop(message);
    drop(large);
    let recovered = notify(&first, &arguments(0)).await?;
    snapshot(snapshots, |value| {
        value.dropped_frames > 0
            && value
                .active
                .iter()
                .any(|notification| notification.key.id == recovered)
    })
    .await?;
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("/usr/bin/gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                SERVICE,
                "--object-path",
                PATH,
                "--method",
                "org.freedesktop.Notifications.Notify",
                "--",
                "GLib fixture",
                "0",
                "",
                "Independent GLib fixture",
                "Fixture body",
                "[]",
                "{}",
                "-1",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "GLib fixture timed out")?
    .map_err(|_| "GLib fixture could not start")?;
    if !output.status.success() {
        return Err("Independent GLib notification failed".into());
    }
    snapshot(snapshots, |value| {
        value.active.iter().any(|notification| {
            notification.draft.summary == "Independent GLib fixture"
                && notification.origin.label != "GLib fixture"
        })
    })
    .await?;
    let gone = client(target).await?;
    let gone_id = notify(&gone, &arguments(0)).await?;
    let current = snapshot(snapshots, |value| {
        value
            .active
            .iter()
            .any(|notification| notification.key.id == gone_id)
    })
    .await?;
    let gone_key = current
        .active
        .iter()
        .find(|notification| notification.key.id == gone_id)
        .ok_or("Gone sender fixture missing")?
        .key;
    gone.close()
        .await
        .map_err(|_| "Gone sender fixture did not close")?;
    if action(
        commands,
        events,
        gone_key,
        Action::Reply("Not delivered".into()),
    )
    .await?
    {
        return Err("Disconnected sender received authority".into());
    }
    commands
        .send(Command::ClearHistory)
        .await
        .map_err(|_| "Clear history command failed")?;
    snapshot(snapshots, |value| value.history.is_empty()).await?;
    Ok(())
}
async fn no_takeover(target: &Target, root: PathBuf) -> Result<(), String> {
    let blocker = client(target).await?;
    blocker
        .request_name_with_flags(
            SERVICE,
            zbus::fdo::RequestNameFlags::AllowReplacement | zbus::fdo::RequestNameFlags::DoNotQueue,
        )
        .await
        .map_err(|_| "Could not reserve fixture name")?;
    let (_commands, input) = mpsc::channel(1);
    let (events, _receiver) = mpsc::channel(1);
    let (snapshots, _observer) = watch::channel(Arc::new(Snapshot::empty(
        server::epoch()?,
        Policy::default(),
    )));
    let result = tokio::time::timeout(
        Duration::from_secs(6),
        server::run(root, Policy::default(), input, snapshots, events),
    )
    .await
    .map_err(|_| "Competing server did not refuse the owned name")?;
    if result.is_ok() {
        return Err("Competing notification server started".into());
    }
    let proxy = zbus::fdo::DBusProxy::new(&blocker)
        .await
        .map_err(|_| "Fixture name query failed")?;
    let owner = proxy
        .get_name_owner(
            zbus::names::BusName::try_from(SERVICE)
                .map_err(|_| "Fixture service name is invalid")?,
        )
        .await
        .map_err(|_| "Fixture name disappeared")?;
    if Some(owner.as_str()) != blocker.unique_name().map(|name| name.as_str()) {
        return Err("Existing notification owner was replaced".into());
    }
    drop(proxy);
    blocker
        .close()
        .await
        .map_err(|_| "Fixture name reservation did not close")?;
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    i18n::initialize();
    let result = async {
        if std::env::args().nth(1).as_deref() != Some("disposable")
            || !std::env::var("ENZUI_HYPR_RUNROOT")
                .ok()
                .is_some_and(|path| {
                    std::path::Path::new(&path).file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with("notification-fixture-")
                    })
                })
        {
            return Err(
                "Only an explicit disposable notification-fixture managed session is allowed"
                    .into(),
            );
        }
        let target = Target::from_environment()?;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("Fixture root unavailable")?
            .to_owned();
        no_takeover(&target, root.clone()).await?;
        let epoch = server::epoch()?;
        let policy = Policy {
            history: true,
            ..Policy::default()
        };
        let (commands, input) = mpsc::channel(16);
        let (events, mut receiver) = mpsc::channel(16);
        let (snapshots, mut observer) =
            watch::channel(Arc::new(Snapshot::empty(epoch, policy.clone())));
        let worker = tokio::spawn(server::run(root, policy, input, snapshots, events));
        let result = campaign(&target, &commands, &mut receiver, &mut observer).await;
        drop(commands);
        let ended = tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .map_err(|_| "Notification fixture did not stop")?
            .map_err(|_| "Notification fixture panicked")?;
        result?;
        ended?;
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => println!(
            "{{\"existing_owner_preserved\":true,\"sender_bound_replacement\":true,\"stale_action_denied\":true,\"targeted_action\":true,\"exact_inline_reply\":true,\"raw_image\":true,\"dnd_and_app_rules\":true,\"oversize_recovery\":true,\"independent_glib_client\":true,\"disconnected_sender_denied\":true,\"history_clear\":true,\"worker_joined\":true}}"
        ),
        Err(error) => {
            eprintln!("Notification fixture failed: {error}");
            std::process::exit(1);
        }
    }
}
