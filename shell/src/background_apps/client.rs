use super::*;
use enzui_shell::i18n::tr;
use iced::futures::StreamExt;
use std::{fs, os::unix::fs::MetadataExt, time::Duration};
use zbus::{Connection, MatchRule, MessageStream};

const NAME: &str = "org.enzimos.Background1";
const PATH: &str = "/org/enzimos/Background1";
const EXECUTABLE: &str = "/usr/lib/enzimos-background/enzimos-background-service";
const UNAVAILABLE: &str =
    "The installed background app service is unavailable. Refresh to check again.";
const REPLACED: &str = "The background app service changed. Refresh and review again.";

#[derive(Clone, Debug)]
pub struct Client {
    connection: Connection,
    owner: String,
}

fn error(error: zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(code), _) if code.len() <= 96
            && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') => match code.as_str() {
                "active-trusted-local-client-required" => tr("shell-background-controls-require-an-active-unlocked-local-desktop-session").into(),
                "background-stop-review-stale" | "background-policy-review-stale" | "application-lifecycle-review-stale" => tr("shell-the-app-changed-since-this-review-refresh-and-review-again").into(),
                "application-graceful-stop-unavailable" => tr("shell-the-app-cannot-accept-a-normal-shutdown-request-force-quit-remains-a-separat").into(),
                "portal-app-stop-failed" => tr("shell-the-service-could-not-confirm-that-the-sandboxed-app-stopped-refresh-before").into(),
                "portal-unavailable" => tr("shell-the-desktop-portal-s-background-list-is-unavailable-refresh-before-trying-ag").into(),
                "portal-app-not-running-in-background" | "portal-app-identity-changed" => tr("shell-that-background-app-instance-is-no-longer-available-refresh-the-list").into(),
                "background-policy-adapter-unavailable" => tr("shell-this-activation-policy-is-not-supported-by-the-installed-service").into(),
                _ => tr("shell-the-background-app-service-refused-the-operation-refresh-before-trying-again").into(),
            },
        _ => UNAVAILABLE.into(),
    }
}

async fn current_owner(connection: &Connection) -> Result<String, String> {
    zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(error)?
        .get_name_owner(NAME.try_into().map_err(|_| INVALID)?)
        .await
        .map(|owner| owner.to_string())
        .map_err(|_| REPLACED.into())
}

impl Client {
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub async fn connect() -> Result<(Self, Capabilities, Snapshot), String> {
        tokio::time::timeout(Duration::from_secs(20), async {
            let connection = Connection::session().await.map_err(error)?;
            let daemon = zbus::fdo::DBusProxy::new(&connection)
                .await
                .map_err(error)?;
            daemon
                .start_service_by_name(NAME.try_into().map_err(|_| INVALID)?, 0)
                .await
                .map_err(|_| UNAVAILABLE)?;
            let owner = current_owner(&connection).await?;
            let name = zbus::names::BusName::try_from(owner.as_str()).map_err(|_| INVALID)?;
            let pid = daemon
                .get_connection_unix_process_id(name.clone())
                .await
                .map_err(|e| error(e.into()))?;
            let uid = daemon
                .get_connection_unix_user(name)
                .await
                .map_err(|e| error(e.into()))?;
            tokio::task::spawn_blocking(move || {
                let installed = fs::metadata(EXECUTABLE).map_err(|_| UNAVAILABLE)?;
                let running = fs::metadata(format!("/proc/{pid}/exe")).map_err(|_| UNAVAILABLE)?;
                let own_uid = fs::metadata("/proc/self").map_err(|_| UNAVAILABLE)?.uid();
                if uid != own_uid
                    || installed.uid() != 0
                    || installed.mode() & 0o022 != 0
                    || !installed.is_file()
                    || (installed.dev(), installed.ino()) != (running.dev(), running.ino())
                {
                    return Err(UNAVAILABLE);
                }
                Ok(())
            })
            .await
            .map_err(|_| UNAVAILABLE)?
            .map_err(str::to_owned)?;
            let client = Self { connection, owner };
            let (caps, snapshot) = client.refresh().await?;
            Ok((client, caps, snapshot))
        })
        .await
        .map_err(|_| tr("shell-reading-background-apps-took-too-long").to_owned())?
    }

    /// Requests OS-approved search activation once, with a five-second deadline; `false` means
    /// refusal.
    pub async fn activate_for_search(&self, app_id: &str) -> Result<bool, String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            if current_owner(&self.connection).await? != self.owner {
                return Err(REPLACED.into());
            }
            let proxy = zbus::Proxy::new(&self.connection, self.owner.as_str(), PATH, NAME)
                .await
                .map_err(error)?;
            let (started, _reason): (bool, String) = proxy
                .call("ActivateForSearch", &(app_id,))
                .await
                .map_err(error)?;
            Ok(started)
        })
        .await
        .map_err(|_| tr("shell-starting-the-app-for-search-took-too-long").to_owned())?
    }
    async fn call(&self, method: &str, body: Option<Value>) -> Result<String, String> {
        tokio::time::timeout(Duration::from_secs(15), async {
            if current_owner(&self.connection).await? != self.owner {
                return Err(REPLACED.into());
            }
            let proxy = zbus::Proxy::new(&self.connection, self.owner.as_str(), PATH, NAME)
                .await
                .map_err(error)?;
            let reply: String = match body {
                Some(mut value) => {
                    value["schema"] = 1.into();
                    proxy.call(method, &(value.to_string(),)).await
                }
                None => proxy.call(method, &()).await,
            }
            .map_err(error)?;
            if reply.len() > LIMIT {
                return Err(INVALID.into());
            }
            if current_owner(&self.connection).await? != self.owner {
                return Err(REPLACED.into());
            }
            Ok(reply)
        })
        .await
        .map_err(|_| {
            tr("shell-the-background-operation-timed-out-its-outcome-is-unknown-refresh-before-try")
                .to_owned()
        })?
    }

    pub async fn query(&self) -> Result<Snapshot, String> {
        Snapshot::decode(&self.call("Query", None).await?)
    }

    pub async fn refresh(&self) -> Result<(Capabilities, Snapshot), String> {
        let caps: Capabilities = decode(&self.call("GetCapabilities", None).await?)?;
        if caps.schema != 1 {
            return Err(INVALID.into());
        }
        Ok((caps, self.query().await?))
    }

    pub async fn review(&self, action: Action) -> Result<Review, String> {
        let body = match &action {
            Action::Stop { app, mode } if app.can_stop(*mode) => {
                serde_json::json!({"app_id":app.app_id,"mode":mode.key()})
            }
            Action::Policy { app, requested }
                if policy(requested) && requested.keys().eq(app.activation_policy.keys()) =>
            {
                serde_json::json!({"app_id":app.app_id,"policy":requested})
            }
            Action::Portal(app) if app.validate() => {
                return Ok(Review {
                    action,
                    digest: None,
                    plan: Value::Null,
                });
            }
            _ => {
                return Err(
                    tr("shell-that-background-action-is-unavailable-refresh-the-list").into(),
                );
            }
        };
        let method = if matches!(action, Action::Policy { .. }) {
            "ReviewPolicy"
        } else {
            "ReviewStop"
        };
        Review::decode(&self.call(method, Some(body)).await?, action)
    }

    pub async fn commit(&self, review: &Review) -> Result<(), String> {
        let (method, body) = match &review.action {
            Action::Portal(app) => (
                "StopPortalApp",
                serde_json::json!({"app_id":app.app_id,"instance":app.instance}),
            ),
            Action::Stop { app, mode } => (
                "Stop",
                serde_json::json!({"app_id":app.app_id,"mode":mode.key(),"plan_sha256":review.digest,"valid_for_ms":10000}),
            ),
            Action::Policy { app, requested } => (
                "SetPolicy",
                serde_json::json!({"app_id":app.app_id,"policy":requested,"plan_sha256":review.digest,"valid_for_ms":10000}),
            ),
        };
        let reply: Value = decode(&self.call(method, Some(body)).await?)?;
        let valid = reply["schema"] == 1
            && match &review.action {
                Action::Portal(app) => {
                    reply["app_id"] == app.app_id
                        && reply["instance"] == app.instance
                        && reply["state"] == "stop-requested"
                }
                Action::Stop { mode, .. } => {
                    reply["future_activation_policy_changed"] == false
                        && reply["personal_files_removed"] == false
                        && reply["force_fallback_used"] == false
                        && reply["result"]["mode"] == mode.key()
                }
                Action::Policy { requested, .. } => {
                    reply["activation_policy"]
                        == serde_json::to_value(requested).map_err(|_| INVALID)?
                        && reply["ongoing_work_stopped"] == false
                        && reply["agent_tool_consent_granted"] == false
                }
            };
        if valid {
            Ok(())
        } else {
            Err(tr("shell-the-background-operation-returned-an-unknown-outcome-refresh-before-trying-a").into())
        }
    }

    pub fn changes(self) -> impl iced::futures::Stream<Item = Result<(), String>> + Send + 'static {
        let client = self;
        iced::stream::channel(1, async move |mut output| {
            use iced::futures::SinkExt;
            let result = async {
                let rule = MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender(client.owner.as_str())
                    .map_err(error)?
                    .path(PATH)
                    .map_err(error)?
                    .interface(NAME)
                    .map_err(error)?
                    .member("Changed")
                    .map_err(error)?
                    .build();
                let mut events = MessageStream::for_match_rule(rule, &client.connection, Some(8))
                    .await
                    .map_err(error)?;
                let rule = MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender("org.freedesktop.DBus")
                    .map_err(error)?
                    .interface("org.freedesktop.DBus")
                    .map_err(error)?
                    .member("NameOwnerChanged")
                    .map_err(error)?
                    .add_arg(NAME)
                    .map_err(error)?
                    .build();
                let mut owners = MessageStream::for_match_rule(rule, &client.connection, Some(8))
                    .await
                    .map_err(error)?;
                if current_owner(&client.connection).await? != client.owner {
                    return Err(REPLACED.to_owned());
                }
                if output.send(Ok(())).await.is_err() {
                    return Ok(());
                }
                loop {
                    tokio::select! {
                        event = events.next() => {
                            event.ok_or(UNAVAILABLE)?.map_err(error)?;
                            if output.send(Ok(())).await.is_err() { return Ok(()); }
                        }
                        _ = owners.next() => return Err(REPLACED.to_owned()),
                    }
                }
            }
            .await;
            if let Err(error) = result {
                let _ = output.send(Err(error)).await;
            }
        })
    }
}
