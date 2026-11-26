use super::{Failure, generation};
use iced::futures::StreamExt;
use std::{collections::HashMap, time::Duration};
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedFd, OwnedObjectPath, OwnedValue, Value},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
pub(super) const BACKEND: &str = "org.freedesktop.impl.portal.desktop.enzuiremote";
const DEVICES: u32 = 3;

type Result<T> = std::result::Result<T, Failure>;

type Options<'a> = HashMap<&'a str, Value<'a>>;
type Fields = HashMap<String, OwnedValue>;

pub(super) struct Portal {
    pub(super) connection: Connection,
    session: OwnedObjectPath,
}

impl Portal {
    pub(super) async fn new(generation: &str) -> Result<Self> {
        let connection = Connection::session()
            .await
            .map_err(|error| dbus_failure("portal session bus", error))?;
        let sender = sender(&connection)?;
        let session = OwnedObjectPath::try_from(format!(
            "{PATH}/session/{sender}/enzui_assistant_{generation}"
        ))
        .map_err(|_| ("portal session bus", "invalid session object path"))?;
        Ok(Self {
            connection,
            session,
        })
    }

    pub(super) fn handle(&self) -> &str {
        self.session.as_str()
    }

    pub(super) async fn start(&self) -> Result<()> {
        let portal = Proxy::new(&self.connection, PORTAL, PATH, REMOTE)
            .await
            .map_err(|error| dbus_failure("RemoteDesktop proxy", error))?;
        let token = self
            .session
            .as_str()
            .rsplit('/')
            .next()
            .ok_or(("CreateSession", "missing session token"))?;
        let created = self
            .request("CreateSession", |options| async {
                let mut options = options;
                options.insert("session_handle_token", Value::from(token));
                portal.call("CreateSession", &options).await
            })
            .await?;
        let handle = created.get("session_handle").and_then(|value| {
            <&str>::try_from(value).ok().or_else(|| {
                <&zbus::zvariant::ObjectPath<'_>>::try_from(value)
                    .ok()
                    .map(zbus::zvariant::ObjectPath::as_str)
            })
        });
        if handle != Some(self.session.as_str()) {
            return Err(("CreateSession", "portal returned a different session path"));
        }
        self.request("SelectDevices", |mut options| async {
            options.insert("types", Value::from(DEVICES));
            options.insert("persist_mode", Value::from(0_u32));
            portal
                .call("SelectDevices", &(&self.session, options))
                .await
        })
        .await?;
        let started = self
            .request("Start", |options| async {
                portal.call("Start", &(&self.session, "", options)).await
            })
            .await?;
        if started.get("devices").and_then(|v| u32::try_from(v).ok()) != Some(DEVICES) {
            return Err(("consent response", "portal did not grant both devices"));
        }
        Ok(())
    }

    async fn request<F, Fut>(&self, stage: &'static str, call: F) -> Result<Fields>
    where
        F: FnOnce(Options<'static>) -> Fut,
        Fut: std::future::Future<Output = zbus::Result<OwnedObjectPath>>,
    {
        let timeout = if stage == "Start" { 60 } else { 2 };
        tokio::time::timeout(Duration::from_secs(timeout), async {
            let token = format!(
                "r{}",
                generation().map_err(|_| (stage, "could not allocate a request token"))?
            );
            let sender = sender(&self.connection)?;
            let path = format!("{PATH}/request/{sender}/{token}");
            let proxy = Proxy::new(
                &self.connection,
                PORTAL,
                path.as_str(),
                "org.freedesktop.portal.Request",
            )
            .await
            .map_err(|error| dbus_failure(stage, error))?;
            let mut response = proxy
                .receive_signal("Response")
                .await
                .map_err(|error| dbus_failure(stage, error))?;
            let returned = call(HashMap::from([("handle_token", Value::from(token))]))
                .await
                .map_err(|error| dbus_failure(stage, error))?;
            if returned.as_str() != path {
                return Err((stage, "portal returned a different request path"));
            }
            let message = response
                .next()
                .await
                .ok_or((stage, "response stream closed"))?;
            let (code, values): (u32, Fields) = message
                .body()
                .deserialize()
                .map_err(|_| (stage, "invalid response payload"))?;
            match code {
                0 => Ok(values),
                1 if stage == "Start" => Err((
                    "consent response",
                    "Start returned 1 (declined or cancelled)",
                )),
                1 => Err((stage, "portal returned 1 (cancelled)")),
                2 => Err((stage, "portal returned 2 (failed)")),
                _ => Err((stage, "portal returned an unknown response code")),
            }
        })
        .await
        .map_err(|_| (stage, "portal request timed out"))?
    }

    pub(super) async fn session(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            PORTAL,
            &self.session,
            "org.freedesktop.portal.Session",
        )
        .await
        .map_err(|error| dbus_failure("portal session", error))
    }

    pub(super) async fn connect_eis(&self) -> Result<OwnedFd> {
        let stage = "ConnectToEIS";
        tokio::time::timeout(Duration::from_secs(2), async {
            let portal = Proxy::new(&self.connection, PORTAL, PATH, REMOTE)
                .await
                .map_err(|error| dbus_failure(stage, error))?;
            portal
                .call("ConnectToEIS", &(&self.session, Options::new()))
                .await
                .map_err(|error| dbus_failure(stage, error))
        })
        .await
        .map_err(|_| (stage, "portal call timed out"))?
    }

    pub(super) async fn close(self) {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            if let Ok(proxy) = self.session().await {
                let _: zbus::Result<()> = proxy.call("Close", &()).await;
            }
        })
        .await;
        let _ = self.connection.close().await;
    }
}

fn sender(connection: &Connection) -> Result<String> {
    Ok(connection
        .unique_name()
        .ok_or(("portal session bus", "connection has no unique name"))?
        .trim_start_matches(':')
        .replace('.', "_"))
}

// Error descriptions may include paths or arguments supplied by a portal.
fn dbus_failure(stage: &'static str, error: zbus::Error) -> Failure {
    let reason = match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown" => "D-Bus service is unavailable",
            "org.freedesktop.DBus.Error.NameHasNoOwner" => "D-Bus service has no owner",
            "org.freedesktop.DBus.Error.UnknownMethod" => "D-Bus method is unavailable",
            "org.freedesktop.DBus.Error.UnknownObject" => "D-Bus object is unavailable",
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.portal.Error.NotAllowed" => "D-Bus access denied",
            "org.freedesktop.DBus.Error.InvalidArgs"
            | "org.freedesktop.portal.Error.InvalidArgument" => {
                "portal rejected the method arguments"
            }
            "org.freedesktop.DBus.Error.NoReply" => "D-Bus service did not reply",
            _ => "D-Bus method returned an error",
        },
        zbus::Error::Connection(..) => "cannot connect to D-Bus",
        zbus::Error::Address(_) => "invalid D-Bus address",
        zbus::Error::Handshake(_) => "D-Bus authentication failed",
        zbus::Error::InputOutput(_) => "D-Bus transport failed",
        zbus::Error::Variant(_) | zbus::Error::InvalidReply => "invalid D-Bus reply",
        _ => "D-Bus operation failed",
    };
    (stage, reason)
}
