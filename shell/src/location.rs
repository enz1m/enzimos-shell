//! Location services: the user's choice and the GeoClue agent that enforces it.
//!
//! GeoClue (configured by enzimOS to accept only this agent) refuses every
//! app while no agent is registered for the user. The agent registers only
//! once GeoClue is running because something asked, so it never starts
//! GeoClue itself, and with Location off it stays unregistered: nothing is
//! looked up and nothing is sent anywhere.
//!
//! - **Approximate** caps every app at city level, which GeoClue answers from
//!   the computer's internet address alone.
//! - **Precise** allows the nearby Wi-Fi networks to be used as well.
//!
//! Flatpak apps go through the location portal, which asks the user per app;
//! sandboxed apps that call GeoClue directly are refused.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;

/// The agent's desktop identity, which GeoClue's configuration allows.
pub const AGENT_ID: &str = "enzui-location-agent";
/// Session-bus name and object of the agent's control interface.
pub const CONTROL_NAME: &str = "dev.enzui.Location";
pub const CONTROL_PATH: &str = "/dev/enzui/Location";
const GEOCLUE: &str = "org.freedesktop.GeoClue2";
const AGENT_PATH: &str = "/org/freedesktop/GeoClue2/Agent";

/// What the user allows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    Approximate,
    Precise,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Approximate, Self::Precise];

    /// GeoClue accuracy level (0 none, 4 city, 8 exact).
    #[must_use]
    pub fn accuracy(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Approximate => 4,
            Self::Precise => 8,
        }
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Approximate => "approximate",
            Self::Precise => "precise",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }
}

/// Where the choice is kept (`$XDG_CONFIG_HOME/enzui-shell/location.json`).
#[must_use]
pub fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|base| base.join("enzui-shell/location.json"))
}

/// The saved choice; anything unreadable means Off.
#[must_use]
pub fn load() -> Mode {
    path()
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| bytes.len() < 4096)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("mode")
                .and_then(|m| m.as_str())
                .and_then(Mode::from_key)
        })
        .unwrap_or_default()
}

/// Save the choice atomically.
///
/// # Errors
/// I/O failures.
pub fn save(mode: Mode) -> std::io::Result<()> {
    use std::io::Write;
    let path = path().ok_or_else(|| std::io::Error::other("no configuration directory"))?;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no configuration directory"))?;
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(".location.json.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(
        serde_json::json!({ "mode": mode.key() })
            .to_string()
            .as_bytes(),
    )?;
    file.sync_all()?;
    std::fs::rename(temporary, path)
}

/// Settings: change the choice, through the running agent when there is one
/// (so it applies at once), otherwise by saving it for the next session.
///
/// # Errors
/// A sentence when the choice could not be saved.
pub async fn set(mode: Mode) -> Result<(), String> {
    if let Ok(session) = zbus::Connection::session().await
        && let Ok(proxy) =
            zbus::Proxy::new(&session, CONTROL_NAME, CONTROL_PATH, CONTROL_NAME).await
        && tokio::time::timeout(
            std::time::Duration::from_secs(5),
            proxy.call::<_, _, ()>("SetMode", &(mode.key(),)),
        )
        .await
        .is_ok_and(|r| r.is_ok())
    {
        return Ok(());
    }
    save(mode).map_err(|_| enzui_shell::i18n::tr("shell-location-could-not-be-saved").to_owned())
}

// ----- the agent -----

struct Agent {
    mode: Arc<Mutex<Mode>>,
}

#[zbus::interface(name = "org.freedesktop.GeoClue2.Agent")]
impl Agent {
    /// Sandboxed apps reaching GeoClue directly are refused: they are meant
    /// to use the location portal, which asks the user per app.
    #[allow(clippy::unused_self, reason = "D-Bus method")]
    fn authorize_app(&self, desktop_id: &str, requested: u32) -> (bool, u32) {
        let _ = (desktop_id, requested);
        (false, 0)
    }

    #[zbus(property)]
    fn max_accuracy_level(&self) -> u32 {
        self.mode.lock().map_or(0, |mode| mode.accuracy())
    }
}

struct Control {
    mode: Arc<Mutex<Mode>>,
    changed: tokio::sync::watch::Sender<Mode>,
}

#[zbus::interface(name = "dev.enzui.Location")]
impl Control {
    #[zbus(property)]
    fn mode(&self) -> String {
        self.mode.lock().map_or(Mode::Off, |m| *m).key().to_owned()
    }

    async fn set_mode(
        &self,
        key: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let mode = Mode::from_key(key)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(key.chars().take(32).collect()))?;
        save(mode).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        if let Ok(mut current) = self.mode.lock() {
            *current = mode;
        }
        let _ = self.mode_changed(&emitter).await;
        let _ = self.changed.send(mode);
        Ok(())
    }
}

async fn register(system: &zbus::Connection) -> bool {
    let Ok(manager) = zbus::Proxy::new(
        system,
        GEOCLUE,
        "/org/freedesktop/GeoClue2/Manager",
        "org.freedesktop.GeoClue2.Manager",
    )
    .await
    else {
        return false;
    };
    // NoAutoStart: registering must never start GeoClue.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        manager.call_with_flags::<_, _, ()>(
            "AddAgent",
            zbus::proxy::MethodFlags::NoAutoStart.into(),
            &(AGENT_ID,),
        ),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Run the agent for this session until the system bus goes away.
///
/// # Errors
/// D-Bus connection failures.
pub async fn run_agent() -> zbus::Result<()> {
    use iced::futures::StreamExt;
    let mode = Arc::new(Mutex::new(load()));
    let (changed, mut watch) = tokio::sync::watch::channel(load());
    let system = zbus::connection::Builder::system()?
        .serve_at(
            AGENT_PATH,
            Agent {
                mode: Arc::clone(&mode),
            },
        )?
        .build()
        .await?;
    let _session = zbus::connection::Builder::session()?
        .serve_at(
            CONTROL_PATH,
            Control {
                mode: Arc::clone(&mode),
                changed,
            },
        )?
        .name(CONTROL_NAME)?
        .build()
        .await?;
    let dbus = zbus::fdo::DBusProxy::new(&system).await?;
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, GEOCLUE)])
        .await?;
    let geoclue = zbus::names::BusName::try_from(GEOCLUE)?;
    let mut registered = false;
    if load() != Mode::Off && dbus.name_has_owner(geoclue.clone()).await.unwrap_or(false) {
        registered = register(&system).await;
    }
    loop {
        tokio::select! {
            owner = owners.next() => {
                let Some(signal) = owner else { return Ok(()) };
                let started = signal.args().is_ok_and(|a| a.new_owner().is_some());
                // A new GeoClue has no agents: register again, unless off.
                registered = started && *watch.borrow() != Mode::Off && register(&system).await;
            }
            update = watch.changed() => {
                if update.is_err() {
                    return Ok(());
                }
                let mode = *watch.borrow_and_update();
                if let Ok(iface) = system.object_server().interface::<_, Agent>(AGENT_PATH).await {
                    // GeoClue stops or restarts its clients when this changes.
                    let _ = iface.get().await.max_accuracy_level_changed(iface.signal_emitter()).await;
                }
                if mode != Mode::Off && !registered && dbus.name_has_owner(geoclue.clone()).await.unwrap_or(false) {
                    registered = register(&system).await;
                }
            }
        }
    }
}
