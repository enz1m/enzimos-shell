use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, StreamExt};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use zbus::{Connection, MatchRule, MessageStream};

const NAME: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";
const LEGACY_PATH: &str = "/ScreenSaver";
const MAX_COOKIES: usize = 256;
const MAX_PER_OWNER: usize = 32;

#[derive(Default)]
struct Cookies {
    next: u32,
    owners: BTreeMap<u32, String>,
}

impl Cookies {
    fn add(&mut self, owner: &str) -> zbus::fdo::Result<u32> {
        if self.owners.len() >= MAX_COOKIES
            || self
                .owners
                .values()
                .filter(|value| value.as_str() == owner)
                .count()
                >= MAX_PER_OWNER
        {
            return Err(zbus::fdo::Error::LimitsExceeded(
                tr("shell-too-many-idle-inhibitors").into(),
            ));
        }
        loop {
            self.next = self.next.wrapping_add(1).max(1);
            if let std::collections::btree_map::Entry::Vacant(entry) = self.owners.entry(self.next)
            {
                entry.insert(owner.to_owned());
                return Ok(self.next);
            }
        }
    }

    fn remove(&mut self, owner: &str, cookie: u32) -> zbus::fdo::Result<()> {
        if self.owners.get(&cookie).is_some_and(|known| known != owner) {
            return Err(zbus::fdo::Error::AccessDenied(
                tr("shell-idle-inhibitor-belongs-to-another-caller").into(),
            ));
        }
        self.owners.remove(&cookie);
        Ok(())
    }
}

struct Shared {
    cookies: Mutex<Cookies>,
    changes: watch::Sender<bool>,
    #[cfg(test)]
    reads: std::sync::atomic::AtomicUsize,
}

impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cookies: Mutex::new(Cookies::default()),
            changes: watch::channel(false).0,
            #[cfg(test)]
            reads: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn change<T>(
        &self,
        update: impl FnOnce(&mut Cookies) -> zbus::fdo::Result<T>,
    ) -> zbus::fdo::Result<T> {
        let mut cookies = self
            .cookies
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed(tr("shell-idle-inhibition-stopped").into()))?;
        let result = update(&mut cookies)?;
        self.changes.send_if_modified(|active| {
            let now = !cookies.owners.is_empty();
            if *active == now {
                false
            } else {
                *active = now;
                true
            }
        });
        Ok(result)
    }
}

#[derive(Clone)]
struct Api(Arc<Shared>);

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl Api {
    fn inhibit(
        &self,
        application_name: &str,
        reason_for_inhibit: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<u32> {
        if application_name.len() > 1024 || reason_for_inhibit.len() > 4096 {
            return Err(zbus::fdo::Error::InvalidArgs(
                tr("shell-idle-inhibitor-description-is-too-long").into(),
            ));
        }
        let owner = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied(tr("shell-missing-caller").into()))?;
        self.0.change(|cookies| cookies.add(owner.as_str()))
    }

    fn un_inhibit(
        &self,
        cookie: u32,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let owner = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied(tr("shell-missing-caller").into()))?;
        self.0
            .change(|cookies| cookies.remove(owner.as_str(), cookie))
    }

    #[zbus(property)]
    fn inhibited(&self) -> bool {
        #[cfg(test)]
        self.0
            .reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.0
            .cookies
            .lock()
            .is_ok_and(|cookies| !cookies.owners.is_empty())
    }
}

// The desktop owns this service independently of the replaceable shell process.
// Cookies follow unique bus callers, rather than app names or guessed PIDs.
pub async fn serve(connection: Connection) -> Result<(), String> {
    run(connection, Shared::new()).await
}

async fn run(connection: Connection, shared: Arc<Shared>) -> Result<(), String> {
    let mut latest = shared.changes.subscribe();
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|e| e.to_string())?
        .interface("org.freedesktop.DBus")
        .map_err(|e| e.to_string())?
        .member("NameOwnerChanged")
        .map_err(|e| e.to_string())?
        .build();
    let mut owners = MessageStream::for_match_rule(rule, &connection, Some(256))
        .await
        .map_err(|e| e.to_string())?;
    for path in [PATH, LEGACY_PATH] {
        connection
            .object_server()
            .at(path, Api(std::sync::Arc::clone(&shared)))
            .await
            .map_err(|e| e.to_string())?;
    }
    let reply = connection
        .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(
        reply,
        zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner
    ) {
        return Err(tr("shell-another-idle-inhibition-service-already-owns-this-session").into());
    }
    loop {
        tokio::select! {
            changed = latest.changed() => {
                changed.map_err(|_| tr("shell-idle-inhibitor-state-stopped").to_owned())?;
                latest.borrow_and_update();
                for path in [PATH, LEGACY_PATH] {
                    let interface = connection.object_server().interface::<_, Api>(path).await.map_err(|e| e.to_string())?;
                    interface.get().await.inhibited_changed(interface.signal_emitter()).await.map_err(|e| e.to_string())?;
                }
            }
            message = owners.next() => {
                let message = message.ok_or_else(|| tr("shell-session-bus-closed"))?.map_err(|e| e.to_string())?;
                let (name, old, new) = message.body().deserialize::<(String, String, String)>().map_err(|e| e.to_string())?;
                if name == NAME && new != connection.unique_name().map(|n| n.as_str()).unwrap_or_default() {
                    return Err(tr("shell-idle-inhibition-service-ownership-was-lost").into());
                }
                if name.starts_with(':') && old == name && new.is_empty() {
                    shared.change(|cookies| { cookies.owners.retain(|_, owner| owner != &name); Ok(()) }).map_err(|e| e.to_string())?;
                }
            }
        }
    }
}

async fn owner(connection: &Connection) -> Result<Option<String>, zbus::Error> {
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    match bus.get_name_owner(NAME.try_into()?).await {
        Ok(owner) => Ok(Some(owner.to_string())),
        Err(zbus::fdo::Error::NameHasNoOwner(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn follow(
    connection: &Connection,
    output: &mut iced::futures::channel::mpsc::Sender<Result<bool, String>>,
) -> Result<(), zbus::Error> {
    let owners = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .add_arg(NAME)?
        .build();
    let mut owners = MessageStream::for_match_rule(owners, connection, Some(16)).await?;
    let mut last = None;
    loop {
        let Some(owner) = owner(connection).await? else {
            if last != Some(false) {
                if output.send(Ok(false)).await.is_err() {
                    return Ok(());
                }
                last = Some(false);
            }
            owners
                .next()
                .await
                .ok_or_else(|| zbus::Error::Failure(tr("shell-session-bus-closed").into()))??;
            continue;
        };
        let properties = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(owner.as_str())?
            .path(PATH)?
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .add_arg(NAME)?
            .build();
        let mut properties =
            MessageStream::for_match_rule(properties, connection, Some(16)).await?;
        let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(connection)
            .destination(owner.as_str())?
            .path(PATH)?
            .interface(NAME)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        loop {
            let active = match tokio::time::timeout(
                Duration::from_secs(3),
                proxy.get_property::<bool>("Inhibited"),
            )
            .await
            {
                Ok(Ok(active)) => active,
                result => {
                    let error = match result {
                        Ok(Err(error)) => error.to_string(),
                        _ => tr("shell-idle-inhibitor-read-timed-out").into(),
                    };
                    if output.send(Err(error)).await.is_err() {
                        return Ok(());
                    }
                    last = Some(false);
                    tokio::select! {
                        message = owners.next() => {
                            message.ok_or_else(|| zbus::Error::Failure(tr("shell-session-bus-closed").into()))??;
                            break;
                        }
                        message = properties.next() => {
                            message.ok_or_else(|| zbus::Error::Failure(tr("shell-idle-inhibitor-signals-stopped").into()))??;
                            continue;
                        }
                    }
                }
            };
            if last != Some(active) {
                if output.send(Ok(active)).await.is_err() {
                    return Ok(());
                }
                last = Some(active);
            }
            tokio::select! {
                message = owners.next() => {
                    message.ok_or_else(|| zbus::Error::Failure(tr("shell-session-bus-closed").into()))??;
                    break;
                }
                message = properties.next() => {
                    message.ok_or_else(|| zbus::Error::Failure(tr("shell-idle-inhibitor-signals-stopped").into()))??;
                }
            }
        }
    }
}

pub fn subscription() -> iced::Subscription<Result<bool, String>> {
    iced::Subscription::run(|| {
        iced::stream::channel(4, async |mut output| {
            let result = async {
                let connection =
                    tokio::time::timeout(Duration::from_secs(3), Connection::session())
                        .await
                        .map_err(|_| tr("shell-session-bus-connection-timed-out").to_owned())?
                        .map_err(|e| e.to_string())?;
                follow(&connection, &mut output)
                    .await
                    .map_err(|e| e.to_string())
            }
            .await;
            if let Err(error) = result {
                let _ = output.send(Err(error)).await;
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    async fn wait(mut ready: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn service(
        bus: &crate::test_bus::Bus,
    ) -> (tokio::task::JoinHandle<Result<(), String>>, Arc<Shared>) {
        let connection = bus.client().await;
        let shared = Shared::new();
        let state = std::sync::Arc::clone(&shared);
        let task = tokio::spawn(async move { run(connection, state).await });
        let observer = bus.client().await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while owner(&observer).await.unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        (task, shared)
    }

    async fn proxy<'a>(connection: &'a Connection, path: &'static str) -> zbus::Proxy<'a> {
        zbus::proxy::Builder::new(connection)
            .destination(NAME)
            .unwrap()
            .path(path)
            .unwrap()
            .interface(NAME)
            .unwrap()
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap()
    }

    async fn inhibit(proxy: &zbus::Proxy<'_>) -> u32 {
        proxy
            .call("Inhibit", &("org.enzui.Test", "Public idle fixture"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn idle_inhibit_async_behaviour() {
        // Caller owned cookies aggregate and release across both paths.
        {
            let bus = crate::test_bus::Bus::start();
            let (task, shared) = service(&bus).await;
            let first = bus.client().await;
            let second = bus.client().await;
            let a = proxy(&first, PATH).await;
            let b = proxy(&second, LEGACY_PATH).await;
            let one = inhibit(&a).await;
            let two = inhibit(&b).await;
            assert_ne!(one, 0);
            assert_ne!(one, two);
            assert!(b.call::<_, _, ()>("UnInhibit", &(one,)).await.is_err());
            assert_eq!(shared.cookies.lock().unwrap().owners.len(), 2);
            a.call::<_, _, ()>("UnInhibit", &(one,)).await.unwrap();
            assert!(a.get_property::<bool>("Inhibited").await.unwrap());
            b.call::<_, _, ()>("UnInhibit", &(two,)).await.unwrap();
            assert!(!a.get_property::<bool>("Inhibited").await.unwrap());
            b.call::<_, _, ()>("UnInhibit", &(two,)).await.unwrap();
            task.abort();
            let _ = task.await;
        }
        // Observer is passive event driven and releases its connection.
        {
            let bus = crate::test_bus::Bus::start();
            let connection = bus.client().await;
            let name = connection.unique_name().unwrap().to_owned();
            let (mut output, mut input) = iced::futures::channel::mpsc::channel(8);
            let observer = tokio::spawn(async move { follow(&connection, &mut output).await });
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(Ok(false))
            );
            let peer = bus.client().await;
            assert!(owner(&peer).await.unwrap().is_none());
            let (task, shared) = service(&bus).await;
            wait(|| shared.reads.load(Ordering::SeqCst) > 0).await;
            let api = proxy(&peer, PATH).await;
            let _cookie = inhibit(&api).await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(Ok(true))
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            let reads = shared.reads.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(shared.reads.load(Ordering::SeqCst), reads);
            task.abort();
            let _ = task.await;
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), input.next())
                    .await
                    .unwrap(),
                Some(Ok(false))
            );
            observer.abort();
            let _ = observer.await;
            let dbus = zbus::fdo::DBusProxy::new(&peer).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while dbus.name_has_owner(name.clone().into()).await.unwrap() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        // Existing provider is never replaced or queued.
        {
            let bus = crate::test_bus::Bus::start();
            let existing = bus.client().await;
            existing
                .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
                .await
                .unwrap();
            let identity = existing.unique_name().unwrap().to_string();
            let candidate = bus.client().await;
            assert!(
                tokio::time::timeout(Duration::from_secs(3), serve(candidate))
                    .await
                    .unwrap()
                    .is_err()
            );
            assert_eq!(owner(&existing).await.unwrap(), Some(identity));
            existing.release_name(NAME).await.unwrap();
            assert!(owner(&existing).await.unwrap().is_none());
        }
    }
}
