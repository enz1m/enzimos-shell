use super::{Result, generation, unavailable};
use iced::futures::StreamExt;
use std::collections::HashMap;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedFd, OwnedObjectPath, OwnedValue, Value},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE: &str = "org.freedesktop.portal.RemoteDesktop";
pub(super) const BACKEND: &str = "org.freedesktop.impl.portal.desktop.enzuiremote";
const DEVICES: u32 = 3;

type Options<'a> = HashMap<&'a str, Value<'a>>;
type Fields = HashMap<String, OwnedValue>;

pub(super) struct Portal {
    pub(super) connection: Connection,
    session: OwnedObjectPath,
}

impl Portal {
    pub(super) async fn new(generation: &str) -> Result<Self> {
        let connection = Connection::session().await.map_err(|_| unavailable())?;
        let sender = sender(&connection)?;
        let session = OwnedObjectPath::try_from(format!(
            "{PATH}/session/{sender}/enzui_assistant_{generation}"
        ))
        .map_err(|_| unavailable())?;
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
            .map_err(|_| unavailable())?;
        let token = self
            .session
            .as_str()
            .rsplit('/')
            .next()
            .ok_or_else(unavailable)?;
        let created = self
            .request(|options| async {
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
            return Err(unavailable());
        }
        self.request(|mut options| async {
            options.insert("types", Value::from(DEVICES));
            options.insert("persist_mode", Value::from(0_u32));
            portal
                .call("SelectDevices", &(&self.session, options))
                .await
        })
        .await?;
        let started = self
            .request(|options| async { portal.call("Start", &(&self.session, "", options)).await })
            .await?;
        if started.get("devices").and_then(|v| u32::try_from(v).ok()) != Some(DEVICES) {
            return Err(unavailable());
        }
        Ok(())
    }

    async fn request<F, Fut>(&self, call: F) -> Result<Fields>
    where
        F: FnOnce(Options<'static>) -> Fut,
        Fut: std::future::Future<Output = zbus::Result<OwnedObjectPath>>,
    {
        let token = format!("r{}", generation()?);
        let sender = sender(&self.connection)?;
        let path = format!("{PATH}/request/{sender}/{token}");
        let proxy = Proxy::new(
            &self.connection,
            PORTAL,
            path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .await
        .map_err(|_| unavailable())?;
        let mut response = proxy
            .receive_signal("Response")
            .await
            .map_err(|_| unavailable())?;
        let returned = call(HashMap::from([("handle_token", Value::from(token))]))
            .await
            .map_err(|_| unavailable())?;
        if returned.as_str() != path {
            return Err(unavailable());
        }
        let message = response.next().await.ok_or_else(unavailable)?;
        let (code, values): (u32, Fields) =
            message.body().deserialize().map_err(|_| unavailable())?;
        if code != 0 {
            return Err(unavailable());
        }
        Ok(values)
    }

    pub(super) async fn session(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            PORTAL,
            &self.session,
            "org.freedesktop.portal.Session",
        )
        .await
        .map_err(|_| unavailable())
    }

    pub(super) async fn connect_eis(&self) -> Result<OwnedFd> {
        let portal = Proxy::new(&self.connection, PORTAL, PATH, REMOTE)
            .await
            .map_err(|_| unavailable())?;
        portal
            .call("ConnectToEIS", &(&self.session, Options::new()))
            .await
            .map_err(|_| unavailable())
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
        .ok_or_else(unavailable)?
        .trim_start_matches(':')
        .replace('.', "_"))
}
