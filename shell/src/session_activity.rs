use iced::futures::{SinkExt, StreamExt, channel::mpsc};
use std::time::Duration;
use zbus::{
    Connection, MatchRule, MessageStream,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const SERVICE: &str = "org.freedesktop.login1";
const INTERFACE: &str = "org.freedesktop.login1.Session";

/// Change a backlight through logind's active local-session authorization.
pub async fn set_brightness(device: &str, value: u32) -> Result<(), String> {
    let work = async {
        let connection = Connection::system().await.ok()?;
        let proxy = session(
            &connection,
            hint().as_deref(),
            rustix::process::getuid().as_raw(),
        )
        .await?;
        proxy
            .call::<_, _, ()>("SetBrightness", &("backlight", device, value))
            .await
            .ok()
    };
    tokio::time::timeout(Duration::from_secs(3), work)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| enzui_shell::i18n::tr("shell-no-controllable-backlight-is-available").into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    Active,
    Inactive,
    Locked,
    #[default]
    Unknown,
}

fn hint() -> Option<String> {
    std::env::var("XDG_SESSION_ID").ok().filter(|id| {
        !id.is_empty()
            && id.len() <= 32
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}

async fn session(
    connection: &Connection,
    hint: Option<&str>,
    uid: u32,
) -> Option<zbus::Proxy<'static>> {
    let manager = zbus::Proxy::new(
        connection,
        SERVICE,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .ok()?;
    // User services resolve to the manager session by PID. The imported hint
    // identifies the graphical login and is validated against its account below.
    let path = if let Some(hint) = hint {
        manager
            .call::<_, _, OwnedObjectPath>("GetSession", &(hint,))
            .await
            .ok()?
    } else {
        manager
            .call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id(),))
            .await
            .ok()?
    };
    let proxy: zbus::Proxy<'static> = zbus::proxy::Builder::new(connection)
        .destination(SERVICE)
        .ok()?
        .path(path)
        .ok()?
        .interface(INTERFACE)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    let user: (u32, OwnedObjectPath) = proxy.get_property("User").await.ok()?;
    let seat: (String, OwnedObjectPath) = proxy.get_property("Seat").await.ok()?;
    let kind: String = proxy.get_property("Type").await.ok()?;
    let remote: bool = proxy.get_property("Remote").await.ok()?;
    (user.0 == uid && !seat.0.is_empty() && matches!(kind.as_str(), "wayland" | "x11") && !remote)
        .then_some(proxy)
}

async fn read(proxy: &zbus::Proxy<'_>) -> State {
    let state = async {
        if !proxy.get_property::<bool>("Active").await.ok()? {
            return Some(State::Inactive);
        }
        let locked = proxy.get_property::<bool>("LockedHint").await.ok()?;
        Some(if locked { State::Locked } else { State::Active })
    };
    tokio::time::timeout(Duration::from_secs(3), state)
        .await
        .ok()
        .flatten()
        .unwrap_or(State::Unknown)
}

async fn follow(
    connection: &Connection,
    hint: Option<&str>,
    uid: u32,
    output: &mut mpsc::Sender<State>,
) -> Result<(), zbus::Error> {
    let Some(proxy) = tokio::time::timeout(Duration::from_secs(3), session(connection, hint, uid))
        .await
        .ok()
        .flatten()
    else {
        let _ = output.send(State::Unknown).await;
        return Ok(());
    };
    let signal = || MatchRule::builder().msg_type(zbus::message::Type::Signal);
    let properties = signal()
        .sender(SERVICE)?
        .path(proxy.path().clone())?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();
    let owners = signal()
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(SERVICE)?
        .build();
    let removed = signal()
        .sender(SERVICE)?
        .interface("org.freedesktop.login1.Manager")?
        .member("SessionRemoved")?
        .build();
    let properties = MessageStream::for_match_rule(properties, connection, Some(8)).await?;
    let owners = MessageStream::for_match_rule(owners, connection, Some(8)).await?;
    let removed = MessageStream::for_match_rule(removed, connection, Some(8)).await?;
    let mut events =
        iced::futures::stream::select(iced::futures::stream::select(properties, owners), removed);
    let mut last = read(&proxy).await;
    if output.send(last).await.is_err() {
        return Ok(());
    }
    while let Some(event) = events.next().await {
        let event = event?;
        if event
            .header()
            .member()
            .is_some_and(|member| member.as_str() == "NameOwnerChanged")
        {
            let _ = output.send(State::Unknown).await;
            return Ok(());
        }
        if event
            .header()
            .member()
            .is_some_and(|member| member.as_str() == "SessionRemoved")
        {
            if event
                .body()
                .deserialize::<(String, OwnedObjectPath)>()
                .is_ok_and(|(_, path)| path.as_str() == proxy.path().as_str())
            {
                let _ = output.send(State::Unknown).await;
                return Ok(());
            }
            continue;
        }
        let Ok((interface, changes, invalidated)) = event.body().deserialize::<(
            String,
            std::collections::HashMap<String, OwnedValue>,
            Vec<String>,
        )>() else {
            continue;
        };
        if interface == INTERFACE
            && ["User", "Seat", "Type", "Remote"]
                .iter()
                .any(|key| changes.contains_key(*key) || invalidated.iter().any(|name| name == key))
        {
            let _ = output.send(State::Unknown).await;
            return Ok(());
        }
        if interface != INTERFACE
            || (!["Active", "LockedHint"].iter().any(|key| {
                changes.contains_key(*key) || invalidated.iter().any(|property| property == key)
            }))
        {
            continue;
        }
        let now = read(&proxy).await;
        if now != last {
            last = now;
            if output.send(now).await.is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

pub fn subscription() -> iced::Subscription<State> {
    iced::Subscription::run(stream)
}

/// Observe the current local login; loss of logind publishes `Unknown`.
pub fn stream() -> impl iced::futures::Stream<Item = State> {
    let hint = hint();
    iced::stream::channel(4, async move |mut output: mpsc::Sender<State>| {
        loop {
            if let Ok(connection) = Connection::system().await {
                let _ = follow(
                    &connection,
                    hint.as_deref(),
                    rustix::process::geteuid().as_raw(),
                    &mut output,
                )
                .await;
            }
            if output.send(State::Unknown).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    const PATH: &str = "/org/freedesktop/login1/session/c5";

    struct Manager;
    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Manager {
        #[zbus(name = "GetSessionByPID")]
        fn get_session_by_pid(&self, _pid: u32) -> OwnedObjectPath {
            "/org/freedesktop/login1/session/_1".try_into().unwrap()
        }
        fn get_session(&self, id: &str) -> zbus::fdo::Result<OwnedObjectPath> {
            if id == "c5" {
                Ok(OwnedObjectPath::try_from(PATH)
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?)
            } else {
                Err(zbus::fdo::Error::Failed("No session".into()))
            }
        }
    }

    struct Session {
        uid: u32,
        active: Arc<AtomicBool>,
        locked: Arc<AtomicBool>,
        reads: Arc<AtomicUsize>,
    }
    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl Session {
        #[zbus(property)]
        fn user(&self) -> (u32, OwnedObjectPath) {
            (
                self.uid,
                OwnedObjectPath::try_from("/org/freedesktop/login1/user/_1000").unwrap(),
            )
        }
        #[zbus(property)]
        fn seat(&self) -> (String, OwnedObjectPath) {
            (
                "seat0".into(),
                OwnedObjectPath::try_from("/org/freedesktop/login1/seat/seat0").unwrap(),
            )
        }
        #[zbus(property, name = "Type")]
        fn kind(&self) -> &str {
            "wayland"
        }
        #[zbus(property)]
        fn remote(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn locked_hint(&self) -> bool {
            self.locked.load(Ordering::SeqCst)
        }
        #[zbus(property)]
        fn active(&self) -> bool {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.active.load(Ordering::SeqCst)
        }
    }

    #[tokio::test]
    async fn session_activity_async_behaviour() {
        // Session hint requires matching uid and does not guess when missing.
        {
            let bus = crate::test_bus::Bus::start();
            let producer = bus.serve(SERVICE, "/org/freedesktop/login1", Manager).await;
            producer
                .object_server()
                .at(
                    PATH,
                    Session {
                        uid: 1000,
                        active: Arc::new(AtomicBool::new(true)),
                        locked: Arc::new(AtomicBool::new(false)),
                        reads: Arc::new(AtomicUsize::new(0)),
                    },
                )
                .await
                .unwrap();
            let listener = bus.client().await;
            assert!(session(&listener, None, 1000).await.is_none());
            assert!(session(&listener, Some("c5"), 1001).await.is_none());
            assert!(session(&listener, Some("c5"), 1000).await.is_some());
        }
        // Activity edges and owner loss are event driven and release the connection.
        {
            let bus = crate::test_bus::Bus::start();
            let active = Arc::new(AtomicBool::new(true));
            let locked = Arc::new(AtomicBool::new(false));
            let reads = Arc::new(AtomicUsize::new(0));
            let producer = bus.serve(SERVICE, "/org/freedesktop/login1", Manager).await;
            producer
                .object_server()
                .at(
                    PATH,
                    Session {
                        uid: 1000,
                        active: std::sync::Arc::clone(&active),
                        locked: std::sync::Arc::clone(&locked),
                        reads: std::sync::Arc::clone(&reads),
                    },
                )
                .await
                .unwrap();
            let listener = bus.client().await;
            let name = listener.unique_name().unwrap().to_owned();
            let (mut output, mut input) = mpsc::channel(4);
            let task =
                tokio::spawn(async move { follow(&listener, Some("c5"), 1000, &mut output).await });
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Active)
            );
            let initial = reads.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(reads.load(Ordering::SeqCst), initial);
            active.store(false, Ordering::SeqCst);
            producer
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        INTERFACE,
                        std::collections::HashMap::from([("Active", OwnedValue::from(false))]),
                        Vec::<String>::new(),
                    ),
                )
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Inactive)
            );
            active.store(true, Ordering::SeqCst);
            producer
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        INTERFACE,
                        std::collections::HashMap::<String, OwnedValue>::new(),
                        vec!["Active"],
                    ),
                )
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Active)
            );
            locked.store(true, Ordering::SeqCst);
            producer
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        INTERFACE,
                        std::collections::HashMap::from([("LockedHint", OwnedValue::from(true))]),
                        Vec::<String>::new(),
                    ),
                )
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Locked)
            );
            locked.store(false, Ordering::SeqCst);
            producer
                .emit_signal(
                    None::<&str>,
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(
                        INTERFACE,
                        std::collections::HashMap::<String, OwnedValue>::new(),
                        vec!["LockedHint"],
                    ),
                )
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Active)
            );
            producer.release_name(SERVICE).await.unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(State::Unknown)
            );
            task.await.unwrap().unwrap();
            let observer = bus.client().await;
            let proxy = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while proxy.name_has_owner(name.clone().into()).await.unwrap() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    }
}
