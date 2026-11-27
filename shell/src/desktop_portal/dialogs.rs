use super::{Chooser, Event, PATH};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Notify, oneshot};
use zbus::names::OwnedUniqueName;
use zbus::zvariant::{ObjectPath, OwnedValue};

mod account;
mod app_chooser;
mod dynamic_launcher;
mod email;

pub use app_chooser::ApplicationChoice;

pub(super) type Options = HashMap<String, OwnedValue>;

/// A pending consent or application selection window.
#[derive(Debug)]
pub struct PortalDialog {
    pub id: u64,
    pub app: String,
    pub parent: String,
    pub modal: bool,
    pub activation_token: Option<String>,
    pub kind: PortalDialogKind,
}

/// Data needed to display each portal prompt.
#[derive(Debug)]
pub enum PortalDialogKind {
    OpenWith {
        item: String,
        choices: Vec<ApplicationChoice>,
        last_choice: Option<String>,
    },
    Install {
        name: String,
        editable: bool,
        target: Option<String>,
        preview: iced::widget::image::Handle,
    },
    ShareAccount {
        user: String,
        name: String,
        reason: Option<String>,
        avatar: Option<PathBuf>,
    },
}

/// A choice explicitly confirmed in a portal dialog.
#[derive(Debug)]
pub enum PortalDialogAnswer {
    Application {
        id: String,
        activation_token: Option<String>,
    },
    Install(String),
    ShareAccount,
}

#[derive(Debug)]
struct DialogRequest {
    id: u64,
    owner: OwnedUniqueName,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    cancel: Arc<Notify>,
    choices: Option<Arc<app_chooser::Choices>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl DialogRequest {
    fn close(&self, #[zbus(header)] header: zbus::message::Header<'_>) -> zbus::fdo::Result<()> {
        if header.sender() != Some(&*self.owner) {
            return Err(zbus::fdo::Error::AccessDenied(
                "Request belongs to another caller".into(),
            ));
        }
        self.cancel.notify_one();
        Ok(())
    }
}

#[derive(Clone)]
struct Dialogs(Arc<Chooser>);

impl Dialogs {
    async fn ask(
        &self,
        server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        header: zbus::message::Header<'_>,
        choices: Option<Arc<app_chooser::Choices>>,
        prepare: impl Future<Output = Result<PortalDialog, u32>>,
    ) -> Result<PortalDialogAnswer, u32> {
        let owner = header.sender().ok_or(2u32)?.to_owned().into();
        let id = self.0.next_id();
        let cancel = Arc::new(Notify::new());
        let events = &self.0.events;
        if !server
            .at(
                &handle,
                DialogRequest {
                    id,
                    owner,
                    events: events.clone(),
                    cancel: Arc::clone(&cancel),
                    choices: choices.clone(),
                },
            )
            .await
            .map_err(|_| 2u32)?
        {
            return Err(2);
        }

        let mut shown = false;
        let answer = tokio::select! {
            biased;
            () = cancel.notified() => {
                if shown {
                    let _ = events.send(Event::Close(id));
                }
                Err(1)
            }
            answer = async {
                let mut dialog = prepare.await?;
                dialog.id = id;
                let (reply, answer) = oneshot::channel();
                shown = match choices {
                    Some(choices) => choices.open(dialog, reply, events),
                    None => events.send(Event::Dialog(dialog, reply)).is_ok(),
                };
                answer.await.map_err(|_| 1)
            } => answer
        };
        let _ = server.remove::<DialogRequest, _>(&handle).await;
        answer
    }
}

impl PortalDialog {
    fn new(app: &str, parent: &str, options: &Options, kind: PortalDialogKind) -> Self {
        Self {
            id: 0,
            app: crate::default_apps::name(app).unwrap_or_else(|| app.chars().take(255).collect()),
            parent: parent.chars().take(512).collect(),
            modal: options
                .get("modal")
                .and_then(|v| bool::try_from(v).ok())
                .unwrap_or(true),
            activation_token: super::text(options, "activation_token").filter(|s| s.len() <= 4096),
            kind,
        }
    }
}

pub(super) fn register<'a>(
    builder: zbus::connection::Builder<'a>,
    chooser: Arc<Chooser>,
) -> Result<zbus::connection::Builder<'a>, String> {
    let dialogs = Dialogs(chooser);
    builder
        .serve_at(PATH, app_chooser::AppChooser::new(dialogs.clone()))
        .and_then(|builder| {
            builder.serve_at(
                PATH,
                dynamic_launcher::DynamicLauncher::new(dialogs.clone()),
            )
        })
        .and_then(|builder| builder.serve_at(PATH, account::Account::new(dialogs)))
        .and_then(|builder| builder.serve_at(PATH, email::Email))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
