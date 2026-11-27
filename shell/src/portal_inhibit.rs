use super::{BUS_NAME, FRONTEND, PATH, require_frontend, text};
use iced::futures::{StreamExt, future::join_all};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use zbus::{
    Connection, fdo,
    message::Header,
    object_server::{ResponseDispatchNotifier, SignalEmitter},
    zvariant::{OwnedFd, OwnedObjectPath, OwnedValue},
};

const INTERFACE: &str = "org.freedesktop.impl.portal.Inhibit";
const LOGIN: &str = "org.freedesktop.login1";
const SAVER: &str = "org.freedesktop.ScreenSaver";
const LOGOUT: u32 = 1;
const SWITCH_USER: u32 = 2;
const SUSPEND: u32 = 4;
const IDLE: u32 = 8;
const RUNNING: u32 = 1;
const QUERY_END: u32 = 2;
const ENDING: u32 = 3;
const ACK_TIMEOUT: Duration = Duration::from_secs(1);
const BUS_TIMEOUT: Duration = Duration::from_secs(3);
type Options = HashMap<String, OwnedValue>;

#[derive(Clone, Default)]
pub(super) struct Inhibit(Arc<State>);

struct State {
    requests: Mutex<HashMap<OwnedObjectPath, Arc<Lease>>>,
    monitors: Mutex<HashMap<OwnedObjectPath, Arc<Monitor>>>,
    locked: AtomicBool,
    session: AtomicU32,
    transition: tokio::sync::Mutex<()>,
    monitor_changes: Notify,
}

impl Default for State {
    fn default() -> Self {
        Self {
            requests: Mutex::default(),
            monitors: Mutex::default(),
            locked: AtomicBool::new(false),
            session: AtomicU32::new(RUNNING),
            transition: tokio::sync::Mutex::default(),
            monitor_changes: Notify::new(),
        }
    }
}

struct Lease {
    close: Close,
    app_id: String,
    reason: String,
    flags: u32,
    acquired: AtomicBool,
}

struct Close {
    owner: String,
    cancelled: Notify,
    closed: AtomicBool,
}

impl Close {
    fn new(owner: String) -> Self {
        Self {
            owner,
            cancelled: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn request(&self, header: &Header<'_>) -> fdo::Result<()> {
        if header
            .sender()
            .is_none_or(|sender| sender.as_str() != self.owner)
        {
            return Err(fdo::Error::AccessDenied(
                "Portal handle belongs to another caller".into(),
            ));
        }
        self.cancel();
        Ok(())
    }

    fn cancel(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancelled.notify_one();
    }
}

struct Monitor {
    close: Close,
    ready: AtomicBool,
    query: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

struct Request(Arc<Lease>);
struct Session(Arc<Monitor>);

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        self.0.close.request(&header)?;
        self.0.acquired.store(false, Ordering::Release);
        Ok(())
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        self.0.close.request(&header)?;
        self.0.ready.store(false, Ordering::Release);
        acknowledge(&self.0);
        Ok(())
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

fn acknowledge(monitor: &Monitor) {
    let pending = monitor
        .query
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(pending) = pending {
        let _ = pending.send(());
    }
}

fn insert<T>(
    items: &Mutex<HashMap<OwnedObjectPath, Arc<T>>>,
    path: &OwnedObjectPath,
    value: Arc<T>,
) -> fdo::Result<()> {
    let mut items = items
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if items.len() >= 256 || items.contains_key(path) {
        return Err(fdo::Error::LimitsExceeded(
            "Portal handle is in use or capacity was reached".into(),
        ));
    }
    items.insert(path.clone(), value);
    Ok(())
}

fn remove<T>(items: &Mutex<HashMap<OwnedObjectPath, Arc<T>>>, path: &OwnedObjectPath) {
    items
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(path);
}

fn check_handle(path: &OwnedObjectPath, kind: &str) -> fdo::Result<()> {
    if path.as_str().starts_with(&format!("{PATH}/{kind}/")) && path.as_str().len() <= 512 {
        Ok(())
    } else {
        Err(fdo::Error::InvalidArgs("Invalid portal handle".into()))
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    #[allow(
        clippy::too_many_arguments,
        reason = "The portal wire contract fixes the method arguments."
    )]
    async fn inhibit(
        &self,
        handle: OwnedObjectPath,
        app_id: String,
        _window: &str,
        flags: u32,
        options: Options,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let owner = require_frontend(connection, &header).await?;
        check_handle(&handle, "request")?;
        let flags = flags & (LOGOUT | SWITCH_USER | SUSPEND | IDLE);
        let reason = text(&options, "reason").unwrap_or_default();
        if flags == 0 || app_id.len() > 255 || reason.len() > 1024 {
            return Err(fdo::Error::InvalidArgs(
                "Invalid inhibitor flags or description".into(),
            ));
        }
        let lease = Arc::new(Lease {
            close: Close::new(owner),
            app_id,
            reason,
            flags,
            acquired: AtomicBool::new(false),
        });
        insert(&self.0.requests, &handle, Arc::clone(&lease))?;
        let server = connection.object_server();
        let result = server.at(&handle, Request(Arc::clone(&lease))).await;
        if !matches!(result, Ok(true)) {
            remove(&self.0.requests, &handle);
            return Err(fdo::Error::Failed(
                "Could not export inhibit request".into(),
            ));
        }
        let state = Arc::clone(&self.0);
        let connection = connection.clone();
        let (ready, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let acquired = tokio::select! {
                () = lease.close.cancelled.notified() => None,
                result = tokio::time::timeout(BUS_TIMEOUT, acquire(&lease)) => Some(result),
            };
            match acquired {
                Some(Ok(Ok(hold))) => {
                    lease.acquired.store(true, Ordering::Release);
                    let _ = ready.send(Ok(()));
                    lease.close.cancelled.notified().await;
                    lease.acquired.store(false, Ordering::Release);
                    drop(hold);
                }
                Some(Ok(Err(error))) => {
                    let _ = ready.send(Err(error));
                }
                Some(Err(_)) => {
                    let _ = ready.send(Err(fdo::Error::Timeout("Inhibit timed out".into())));
                }
                None => {
                    let _ = ready.send(Ok(()));
                }
            }
            let _ = connection
                .object_server()
                .remove::<Request, _>(&handle)
                .await;
            remove(&state.requests, &handle);
        });
        result
            .await
            .map_err(|_| fdo::Error::Failed("Inhibit request stopped".into()))?
    }

    async fn create_monitor(
        &self,
        _handle: OwnedObjectPath,
        session_handle: OwnedObjectPath,
        _app_id: &str,
        _window: &str,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<ResponseDispatchNotifier<u32>> {
        let owner = require_frontend(connection, &header).await?;
        check_handle(&session_handle, "session")?;
        let monitor = Arc::new(Monitor {
            close: Close::new(owner),
            ready: AtomicBool::new(false),
            query: Mutex::default(),
        });
        insert(&self.0.monitors, &session_handle, Arc::clone(&monitor))?;
        let result = connection
            .object_server()
            .at(&session_handle, Session(Arc::clone(&monitor)))
            .await;
        if !matches!(result, Ok(true)) {
            remove(&self.0.monitors, &session_handle);
            return Err(fdo::Error::Failed(
                "Could not export monitor session".into(),
            ));
        }
        let (response, dispatched) = ResponseDispatchNotifier::new(0);
        let state = Arc::clone(&self.0);
        let connection = connection.clone();
        tokio::spawn(async move {
            dispatched.await;
            if !monitor.close.closed.load(Ordering::Acquire) {
                monitor.ready.store(true, Ordering::Release);
                state.monitor_changes.notify_one();
                let _ = state
                    .emit(&connection, &session_handle, &monitor.close.owner)
                    .await;
            }
            monitor.close.cancelled.notified().await;
            let _ = connection
                .object_server()
                .remove::<Session, _>(&session_handle)
                .await;
            remove(&state.monitors, &session_handle);
            state.monitor_changes.notify_one();
        });
        Ok(response)
    }

    async fn query_end_response(
        &self,
        session_handle: OwnedObjectPath,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let caller = require_frontend(connection, &header).await?;
        let monitors = self
            .0
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let monitor = monitors
            .get(&session_handle)
            .filter(|m| m.close.owner == caller)
            .ok_or_else(|| fdo::Error::UnknownObject("Unknown portal monitor".into()))?;
        acknowledge(monitor);
        Ok(())
    }

    async fn list_session_inhibitors(
        &self,
        flags: u32,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<Vec<(String, String)>> {
        require_shell(connection, &header).await?;
        let requests = self
            .0
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(requests
            .values()
            .filter(|lease| {
                lease.flags & flags != 0
                    && lease.acquired.load(Ordering::Acquire)
                    && !lease.close.closed.load(Ordering::Acquire)
            })
            .map(|lease| (lease.app_id.clone(), lease.reason.clone()))
            .collect())
    }

    async fn prepare_logout(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        require_shell(connection, &header).await?;
        self.0.end(connection).await;
        Ok(())
    }

    async fn cancel_logout(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        require_shell(connection, &header).await?;
        self.0.set_running(connection).await;
        Ok(())
    }

    #[zbus(signal)]
    async fn state_changed(
        emitter: &SignalEmitter<'_>,
        session_handle: &OwnedObjectPath,
        state: Options,
    ) -> zbus::Result<()>;
}

async fn require_shell(connection: &Connection, header: &Header<'_>) -> fdo::Result<()> {
    let bus = fdo::DBusProxy::new(connection).await?;
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("Missing caller".into()))?;
    let pid = bus
        .get_connection_unix_process_id(sender.as_ref().into())
        .await?;
    let error = |_| fdo::Error::AccessDenied("Only the desktop Shell may call this method".into());
    let current = std::fs::read_link("/proc/self/exe").map_err(error)?;
    let caller = std::fs::read_link(format!("/proc/{pid}/exe")).map_err(error)?;
    if current.with_file_name("enzui-shell") != caller {
        return Err(fdo::Error::AccessDenied(
            "Only the desktop Shell may call this method".into(),
        ));
    }
    Ok(())
}

struct Held {
    _sleep: Option<OwnedFd>,
    _idle: Option<Connection>,
}

async fn acquire(lease: &Lease) -> fdo::Result<Held> {
    let mut hold = Held {
        _sleep: None,
        _idle: None,
    };
    let app = if lease.app_id.is_empty() {
        "Application"
    } else {
        &lease.app_id
    };
    if lease.flags & SUSPEND != 0 {
        let connection = Connection::system().await?;
        let logind = logind(&connection).await?;
        hold._sleep = Some(
            logind
                .call("Inhibit", &("sleep", app, &lease.reason, "block"))
                .await?,
        );
    }
    if lease.flags & IDLE != 0 {
        // A separate D-Bus owner makes the idle cookie follow the inhibitor's lifetime.
        let idle = Connection::session().await?;
        let proxy = zbus::Proxy::new(&idle, SAVER, "/org/freedesktop/ScreenSaver", SAVER).await?;
        proxy
            .call::<_, _, u32>("Inhibit", &(app, &lease.reason))
            .await?;
        drop(proxy);
        hold._idle = Some(idle);
    }
    Ok(hold)
}

async fn logind(connection: &Connection) -> zbus::Result<zbus::Proxy<'_>> {
    zbus::Proxy::new(
        connection,
        LOGIN,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
}

impl State {
    fn monitors(&self) -> Vec<(OwnedObjectPath, Arc<Monitor>)> {
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, monitor)| {
                monitor.ready.load(Ordering::Acquire)
                    && !monitor.close.closed.load(Ordering::Acquire)
            })
            .map(|(path, monitor)| (path.clone(), Arc::clone(monitor)))
            .collect()
    }

    fn has_monitors(&self) -> bool {
        self.monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|monitor| {
                monitor.ready.load(Ordering::Acquire)
                    && !monitor.close.closed.load(Ordering::Acquire)
            })
    }

    async fn emit(
        &self,
        connection: &Connection,
        path: &OwnedObjectPath,
        owner: &str,
    ) -> zbus::Result<()> {
        let options = Options::from([
            (
                "screensaver-active".into(),
                self.locked.load(Ordering::Acquire).into(),
            ),
            (
                "session-state".into(),
                self.session.load(Ordering::Acquire).into(),
            ),
        ]);
        connection
            .emit_signal(
                Some(owner),
                PATH,
                INTERFACE,
                "StateChanged",
                &(path, options),
            )
            .await
    }

    async fn publish(&self, connection: &Connection) {
        for (path, monitor) in self.monitors() {
            let _ = self.emit(connection, &path, &monitor.close.owner).await;
        }
    }

    async fn end(&self, connection: &Connection) {
        let _transition = self.transition.lock().await;
        if self.session.load(Ordering::Acquire) != RUNNING {
            return;
        }
        self.session.store(QUERY_END, Ordering::Release);
        let mut acknowledgements = Vec::new();
        for (path, monitor) in self.monitors() {
            let (tx, rx) = tokio::sync::oneshot::channel();
            *monitor
                .query
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
            let _ = self.emit(connection, &path, &monitor.close.owner).await;
            acknowledgements.push(rx);
        }
        let _ = tokio::time::timeout(ACK_TIMEOUT, join_all(acknowledgements)).await;
        for (_, monitor) in self.monitors() {
            acknowledge(&monitor);
        }
        self.session.store(ENDING, Ordering::Release);
        self.publish(connection).await;
    }

    async fn set_locked(&self, connection: &Connection, locked: bool) {
        if self.locked.swap(locked, Ordering::AcqRel) != locked {
            self.publish(connection).await;
        }
    }

    async fn set_running(&self, connection: &Connection) {
        let _transition = self.transition.lock().await;
        if self.session.swap(RUNNING, Ordering::AcqRel) != RUNNING {
            self.publish(connection).await;
        }
    }

    #[allow(
        clippy::iter_over_hash_type,
        reason = "Closing independent handles has no ordering requirement."
    )]
    fn release_owner(&self, owner: &str) {
        for lease in self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            if lease.close.owner == owner {
                lease.acquired.store(false, Ordering::Release);
                lease.close.cancel();
            }
        }
        for monitor in self
            .monitors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            if monitor.close.owner == owner {
                monitor.ready.store(false, Ordering::Release);
                acknowledge(monitor);
                monitor.close.cancel();
            }
        }
    }
}

impl Inhibit {
    pub(super) async fn watch(&self, connection: &Connection) -> zbus::Result<()> {
        let bus = fdo::DBusProxy::new(connection).await?;
        let mut owners = bus
            .receive_name_owner_changed_with_args(&[(0, FRONTEND)])
            .await?;
        let state = Arc::clone(&self.0);
        let owner_watch = async {
            while let Some(signal) = owners.next().await {
                if let Ok(args) = signal.args()
                    && let Some(old) = args.old_owner().as_ref()
                {
                    state.release_owner(old.as_str());
                }
            }
        };
        let session_watch = async {
            loop {
                while !state.has_monitors() {
                    state.monitor_changes.notified().await;
                }
                let idle = async {
                    while state.has_monitors() {
                        state.monitor_changes.notified().await;
                    }
                };
                tokio::select! {
                    () = idle => {}
                    _ = watch_login(&state, connection) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        };
        tokio::select! {
            () = owner_watch => {}
            () = session_watch => {}
        }
        Ok(())
    }
}

async fn watch_login(state: &State, session_bus: &Connection) -> zbus::Result<()> {
    let system = Connection::system().await?;
    let manager = logind(&system).await?;
    let mut shutdown = manager.receive_signal("PrepareForShutdown").await?;
    let mut delay = shutdown_delay(&manager).await;
    let lock_watch = async {
        let path = if let Ok(id) = std::env::var("XDG_SESSION_ID") {
            manager
                .call::<_, _, OwnedObjectPath>("GetSession", &(id,))
                .await
        } else {
            manager
                .call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id(),))
                .await
        };
        if let Ok(path) = path
            && let Ok(session) =
                zbus::Proxy::new(&system, LOGIN, path, "org.freedesktop.login1.Session").await
        {
            let mut locked = session.receive_property_changed::<bool>("LockedHint").await;
            if let Ok(value) = session.get_property::<bool>("LockedHint").await {
                state.set_locked(session_bus, value).await;
            }
            while let Some(change) = locked.next().await {
                if let Ok(value) = change.get().await {
                    state.set_locked(session_bus, value).await;
                }
            }
        }
        std::future::pending::<()>().await;
    };
    let shutdown_watch = async {
        while let Some(message) = shutdown.next().await {
            if let Ok(ending) = message.body().deserialize::<bool>() {
                if ending {
                    state.end(session_bus).await;
                    drop(delay.take());
                } else {
                    state.set_running(session_bus).await;
                    delay = shutdown_delay(&manager).await;
                }
            }
        }
    };
    tokio::select! {
        () = lock_watch => {}
        () = shutdown_watch => {}
    }
    Ok(())
}

async fn shutdown_delay(manager: &zbus::Proxy<'_>) -> Option<OwnedFd> {
    tokio::time::timeout(
        BUS_TIMEOUT,
        manager.call(
            "Inhibit",
            &("shutdown", "enzUI", "Portal session monitors", "delay"),
        ),
    )
    .await
    .ok()?
    .ok()
}

pub(crate) async fn session_inhibitors(switch: bool) -> Vec<(String, String)> {
    let flags = if switch { SWITCH_USER } else { LOGOUT };
    crate::bus::bounded(async {
        portal()
            .await?
            .call("ListSessionInhibitors", &(flags,))
            .await
            .ok()
    })
    .await
    .flatten()
    .unwrap_or_default()
}

pub(crate) async fn logout(ending: bool) {
    let method = if ending {
        "PrepareLogout"
    } else {
        "CancelLogout"
    };
    let _ = crate::bus::bounded(async { portal().await?.call::<_, _, ()>(method, &()).await.ok() })
        .await;
}

async fn portal() -> Option<zbus::Proxy<'static>> {
    zbus::Proxy::new_owned(crate::bus::session().await?, BUS_NAME, PATH, INTERFACE)
        .await
        .ok()
}
