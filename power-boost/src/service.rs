//! The `dev.enzui.PowerBoost1` system-bus service.

use crate::boost::{Booster, Ending, Host, Outcome};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// The well-known bus name.
pub const NAME: &str = "dev.enzui.PowerBoost1";
/// The object path serving [`NAME`]'s interface.
pub const PATH: &str = "/dev/enzui/PowerBoost1";
/// The polkit action a caller needs; granted to active local sessions.
pub const ACTION: &str = "dev.enzui.power-boost";

const SENDER_INTERVAL: Duration = Duration::from_millis(100);
const AUTHORIZATION_LIFETIME: Duration = Duration::from_secs(5);
const CONTENTION_PAUSE: Duration = Duration::from_secs(60);
const CALL_BOUND: Duration = Duration::from_secs(2);
// Daemons that rewrite the preference every few seconds; one-shot tools such as TLP are not listed.
const LOOP_MANAGERS: [&str; 2] = ["auto-cpufreq.service", "auto-epp.service"];
// power-profiles-daemon's current and older names; tuned-ppd serves the same interface.
const PROFILE_SERVICES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

/// Where the service boosts and how long it stays without work.
#[derive(Clone, Debug)]
pub struct Config {
    /// The cpufreq policy root, normally [`crate::cpu::CPUFREQ`].
    pub cpufreq: PathBuf,
    /// The crash marker; its directory must survive the helper process.
    pub marker: PathBuf,
    /// The helper exits after this long without requests or a running boost.
    pub idle: Duration,
}

struct Caller {
    last: Instant,
    authorization: Option<(bool, Instant)>,
}

#[derive(Default)]
struct Callers(HashMap<String, Caller>);

impl Callers {
    fn admit(&mut self, sender: &str, now: Instant) -> bool {
        self.0.retain(|_, caller| {
            now.saturating_duration_since(caller.last) < AUTHORIZATION_LIFETIME
        });
        match self.0.get_mut(sender) {
            Some(caller) if now.saturating_duration_since(caller.last) < SENDER_INTERVAL => false,
            Some(caller) => {
                caller.last = now;
                true
            }
            None => {
                self.0.insert(
                    sender.to_owned(),
                    Caller {
                        last: now,
                        authorization: None,
                    },
                );
                true
            }
        }
    }

    fn authorization(&self, sender: &str, now: Instant) -> Option<bool> {
        let (allowed, at) = self.0.get(sender)?.authorization?;
        (now.saturating_duration_since(at) < AUTHORIZATION_LIFETIME).then_some(allowed)
    }

    fn remember(&mut self, sender: &str, allowed: bool, at: Instant) {
        if let Some(caller) = self.0.get_mut(sender) {
            caller.authorization = Some((allowed, at));
        }
    }
}

struct State {
    booster: Booster,
    callers: Callers,
    last_request: Instant,
    paused_until: Option<Instant>,
    stopping: bool,
}

struct Service {
    state: Arc<Mutex<State>>,
    wake: Arc<Notify>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[zbus::interface(name = "dev.enzui.PowerBoost1")]
impl Service {
    /// Raises the CPU energy-performance preference for a moment and returns
    /// the outcome: `boosted`, `extended`, `performance`, `power-saving`,
    /// `unsupported`, `other-manager`, `contended`, `busy`, `denied` or `failed`.
    async fn boost(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        let outcome = match header.sender() {
            Some(sender) => self.request(connection, sender.as_str()).await,
            None => Outcome::Denied,
        };
        outcome.as_str().to_owned()
    }
}

impl Service {
    async fn request(&self, connection: &zbus::Connection, sender: &str) -> Outcome {
        let now = Instant::now();
        let cached = {
            let mut state = lock(&self.state);
            if state.stopping || !state.callers.admit(sender, now) {
                return Outcome::Busy;
            }
            state.callers.authorization(sender, now)
        };
        let allowed = match cached {
            Some(allowed) => allowed,
            None => {
                let allowed = authorized(connection, sender).await;
                lock(&self.state).callers.remember(sender, allowed, now);
                allowed
            }
        };
        if !allowed {
            return Outcome::Denied;
        }
        {
            let mut state = lock(&self.state);
            // Only authorized requests keep the helper from exiting when idle.
            state.last_request = now;
            if state.booster.extend(Instant::now()) {
                return Outcome::Extended;
            }
            if state.paused_until.is_some_and(|until| now < until) {
                return Outcome::Contended;
            }
        }
        let host = Host {
            profile: profile(connection).await,
            other_manager: other_manager(connection).await,
        };
        let mut state = lock(&self.state);
        if state.stopping {
            return Outcome::Busy;
        }
        let outcome = state.booster.start(Instant::now(), &host);
        if outcome == Outcome::Boosted {
            self.wake.notify_one();
        }
        outcome
    }
}

/// Serves [`NAME`] on `connection` until idle or terminated, ending any boost first.
///
/// A boost recorded by an earlier process is ended before the name is taken.
///
/// # Errors
///
/// Fails when the object cannot be served or the name cannot be owned.
pub async fn run(connection: zbus::Connection, config: Config) -> zbus::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let booster = Booster::new(config.cpufreq, config.marker);
    if let Some(ending) = booster.recover(profile(&connection).await.as_deref()) {
        eprintln!("enzui-power-boost: ended a boost left by an earlier process: {ending:?}");
    }
    let state = Arc::new(Mutex::new(State {
        booster,
        callers: Callers::default(),
        last_request: Instant::now(),
        paused_until: None,
        stopping: false,
    }));
    let wake = Arc::new(Notify::new());
    let service = Service {
        state: Arc::clone(&state),
        wake: Arc::clone(&wake),
    };
    connection.object_server().at(PATH, service).await?;
    connection.request_name(NAME).await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    loop {
        let deadline = {
            let state = lock(&state);
            state
                .booster
                .until()
                .unwrap_or(state.last_request + config.idle)
        };
        tokio::select! {
            () = tokio::time::sleep_until(deadline.into()) => {}
            () = wake.notified() => continue,
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
        }
        let due = |state: &State| {
            state
                .booster
                .until()
                .is_some_and(|until| until <= Instant::now())
        };
        if due(&lock(&state)) {
            let current = profile(&connection).await;
            let mut state = lock(&state);
            // A request may have extended the boost while the profile was read.
            if due(&state) && state.booster.finish(current.as_deref()) == Some(Ending::Replaced) {
                state.paused_until = Some(Instant::now() + CONTENTION_PAUSE);
            }
        } else {
            let state = lock(&state);
            if state.booster.until().is_none() && state.last_request.elapsed() >= config.idle {
                break;
            }
        }
    }
    let current = profile(&connection).await;
    {
        let mut state = lock(&state);
        state.stopping = true;
        state.booster.finish(current.as_deref());
    }
    let _ = connection.release_name(NAME).await;
    Ok(())
}

async fn call<B>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Option<zbus::Message>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    tokio::time::timeout(
        CALL_BOUND,
        connection.call_method(Some(destination), path, Some(interface), method, body),
    )
    .await
    .ok()?
    .ok()
}

async fn property(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Option<String> {
    let reply = call(
        connection,
        destination,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(interface, name),
    )
    .await?;
    String::try_from(reply.body().deserialize::<OwnedValue>().ok()?).ok()
}

async fn running(connection: &zbus::Connection, name: &str) -> bool {
    let reply = call(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        &(name,),
    )
    .await;
    reply.is_some_and(|reply| reply.body().deserialize::<bool>().unwrap_or(false))
}

// Asking only a running owner keeps a disabled power-profiles-daemon from being bus-activated
// next to TLP or another manager.
async fn profile(connection: &zbus::Connection) -> Option<String> {
    for (name, path, interface) in PROFILE_SERVICES {
        if running(connection, name).await {
            // One retry: an unread profile is treated as unchanged.
            for _ in 0..2 {
                if let Some(profile) =
                    property(connection, name, path, interface, "ActiveProfile").await
                {
                    return Some(profile);
                }
            }
            return None;
        }
    }
    None
}

async fn other_manager(connection: &zbus::Connection) -> bool {
    const SYSTEMD: &str = "org.freedesktop.systemd1";
    for unit in LOOP_MANAGERS {
        let Some(reply) = call(
            connection,
            SYSTEMD,
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "GetUnit",
            &(unit,),
        )
        .await
        else {
            continue;
        };
        let Ok(path) = reply.body().deserialize::<OwnedObjectPath>() else {
            continue;
        };
        let state = property(
            connection,
            SYSTEMD,
            path.as_str(),
            "org.freedesktop.systemd1.Unit",
            "ActiveState",
        )
        .await;
        if matches!(
            state.as_deref(),
            Some("active" | "activating" | "reloading")
        ) {
            return true;
        }
    }
    false
}

async fn authorized(connection: &zbus::Connection, sender: &str) -> bool {
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(sender))]),
    );
    let details: HashMap<&str, &str> = HashMap::new();
    let Some(reply) = call(
        connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
        "CheckAuthorization",
        &(subject, ACTION, details, 0_u32, ""),
    )
    .await
    else {
        return false;
    };
    matches!(
        reply
            .body()
            .deserialize::<(bool, bool, HashMap<String, String>)>(),
        Ok((true, _, _))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sender_is_heard_at_most_every_interval_and_its_authorization_expires() {
        let mut callers = Callers::default();
        let start = Instant::now();
        assert!(callers.admit(":1.5", start));
        assert!(!callers.admit(":1.5", start + SENDER_INTERVAL / 2));
        assert!(callers.admit(":1.6", start + SENDER_INTERVAL / 2));
        assert!(callers.admit(":1.5", start + SENDER_INTERVAL));
        callers.remember(":1.5", true, start + SENDER_INTERVAL);
        assert_eq!(
            callers.authorization(":1.5", start + SENDER_INTERVAL),
            Some(true)
        );
        assert_eq!(callers.authorization(":1.6", start + SENDER_INTERVAL), None);
        let expired = start + SENDER_INTERVAL + AUTHORIZATION_LIFETIME;
        assert_eq!(callers.authorization(":1.5", expired), None);
        assert!(callers.admit(":1.7", expired));
        assert_eq!(callers.0.len(), 1);
    }
}
