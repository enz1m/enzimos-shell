use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::time::Duration;
pub mod preferences;

const NAME: &str = "com.feralinteractive.GameMode";
const PATH: &str = "/com/feralinteractive/GameMode";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub clients: u32,
    pub requested: bool,
}

impl Status {
    pub fn active(&self) -> bool {
        self.clients > 0
    }
}

pub async fn read() -> Option<Status> {
    let connection = tokio::time::timeout(TIMEOUT, crate::bus::session())
        .await
        .ok()??;
    let Some(owner) = tokio::time::timeout(TIMEOUT, owner(&connection))
        .await
        .ok()?
        .ok()
    else {
        // gamemoded starts on a game's first request; until then it is off.
        return tokio::time::timeout(TIMEOUT, activatable(&connection))
            .await
            .ok()?
            .then_some(Status {
                clients: 0,
                requested: false,
            });
    };
    tokio::time::timeout(
        TIMEOUT,
        status(&connection, &owner, std::process::id() as i32),
    )
    .await
    .ok()?
    .ok()
}

async fn activatable(connection: &zbus::Connection) -> bool {
    let Ok(bus) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    bus.list_activatable_names()
        .await
        .is_ok_and(|names| names.iter().any(|name| name.as_str() == NAME))
}

async fn owner(connection: &zbus::Connection) -> Result<String, String> {
    let bus = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| e.to_string())?;
    bus.get_name_owner(
        NAME.try_into()
            .map_err(|e: zbus::names::Error| e.to_string())?,
    )
    .await
    .map(|owner| owner.to_string())
    .map_err(|e| e.to_string())
}

async fn status(connection: &zbus::Connection, owner: &str, pid: i32) -> Result<Status, String> {
    let proxy = zbus::Proxy::new(connection, owner, PATH, NAME)
        .await
        .map_err(|e| e.to_string())?;
    let clients: i32 = proxy
        .get_property("ClientCount")
        .await
        .map_err(|e| e.to_string())?;
    let state: i32 = proxy
        .call("QueryStatus", &(pid,))
        .await
        .map_err(|e| e.to_string())?;
    if clients < 0 || !matches!(state, 0..=2) {
        return Err(tr("shell-gamemode-returned-an-unknown-status").into());
    }
    Ok(Status {
        clients: clients as u32,
        requested: state == 2,
    })
}

pub async fn set(enabled: bool) -> Result<(), String> {
    let connection = tokio::time::timeout(TIMEOUT, zbus::Connection::session())
        .await
        .map_err(|_| tr("shell-gamemode-connection-timed-out").to_owned())?
        .map_err(|e| e.to_string())?;
    set_on(&connection, std::process::id() as i32, enabled).await
}

async fn set_on(connection: &zbus::Connection, pid: i32, enabled: bool) -> Result<(), String> {
    tokio::time::timeout(TIMEOUT, async {
        let name = if enabled {
            let proxy = zbus::Proxy::new(connection, NAME, PATH, NAME)
                .await
                .map_err(|e| e.to_string())?;
            let _: i32 = proxy
                .get_property("ClientCount")
                .await
                .map_err(|e| e.to_string())?;
            owner(connection).await?
        } else {
            match owner(connection).await {
                Ok(owner) => owner,
                Err(error) if error.contains("NameHasNoOwner") => return Ok(()),
                Err(error) => return Err(error),
            }
        };
        let before = status(connection, &name, pid).await?;
        if before.requested == enabled {
            return Ok(());
        }
        let proxy = zbus::Proxy::new(connection, name, PATH, NAME)
            .await
            .map_err(|e| e.to_string())?;
        let result: i32 = proxy
            .call(
                if enabled {
                    "RegisterGame"
                } else {
                    "UnregisterGame"
                },
                &(pid,),
            )
            .await
            .map_err(|e| e.to_string())?;
        if result != 0 {
            return Err(
                tr!("shell-gamemode-refused-the-request", "arg0" => format!("{:}", result)),
            );
        }
        let after = status(connection, proxy.destination().as_str(), pid).await?;
        if after.requested != enabled {
            return Err(tr("shell-gamemode-did-not-confirm-this-desktop-s-request").into());
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        tr("shell-gamemode-request-timed-out-it-will-not-be-retried-automatically").to_owned()
    })?
}
