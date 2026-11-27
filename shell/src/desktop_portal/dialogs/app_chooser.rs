use super::{DialogRequest, Dialogs, Options, PortalDialog, PortalDialogAnswer, PortalDialogKind};
use crate::desktop_portal::{Event, owned, text};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use zbus::zvariant::{ObjectPath, Value};

/// An application offered by the portal frontend, with desktop entry metadata.
#[derive(Clone, Debug)]
pub struct ApplicationChoice {
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
}

pub(super) struct AppChooser(Dialogs);

impl AppChooser {
    pub(super) fn new(dialogs: Dialogs) -> Self {
        Self(dialogs)
    }
}

#[derive(Debug)]
pub(super) struct Choices {
    mime: String,
    state: Mutex<ChoiceState>,
    update: tokio::sync::Mutex<()>,
}

#[derive(Debug, Default)]
struct ChoiceState {
    shown: bool,
    latest: Option<Vec<ApplicationChoice>>,
}

impl Choices {
    pub(super) fn open(
        &self,
        mut dialog: PortalDialog,
        reply: oneshot::Sender<PortalDialogAnswer>,
        events: &mpsc::UnboundedSender<Event>,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let PortalDialogKind::OpenWith { choices, .. } = &mut dialog.kind {
            if let Some(latest) = &state.latest {
                choices.clone_from(latest);
            } else {
                state.latest = Some(choices.clone());
            }
        }
        state.shown = events.send(Event::Dialog(dialog, reply)).is_ok();
        state.shown
    }

    fn contains(&self, id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .latest
            .as_ref()
            .is_some_and(|choices| choices.iter().any(|choice| choice.id == id))
    }
}

async fn resolve(choices: Vec<String>, mime: String) -> Vec<ApplicationChoice> {
    tokio::task::spawn_blocking(move || {
        let wanted = choices
            .iter()
            .map(|id| id.trim_end_matches(".desktop"))
            .collect();
        let index = crate::default_apps::portal_app_index(&mime, &wanted);
        let mut seen = HashSet::new();
        let mut apps: Vec<_> = choices
            .into_iter()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 255
                    && !id.starts_with('.')
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            })
            .filter_map(|id| {
                let desktop_id = id.trim_end_matches(".desktop");
                if !seen.insert(desktop_id.to_owned()) {
                    return None;
                }
                let (name, icon, recommended) = index
                    .get(desktop_id)
                    .cloned()
                    .unwrap_or_else(|| (desktop_id.to_owned(), None, false));
                Some((!recommended, ApplicationChoice { id, name, icon }))
            })
            .collect();
        apps.sort_by_key(|(other, _)| *other);
        apps.into_iter().map(|(_, app)| app).collect()
    })
    .await
    .unwrap_or_default()
}

#[zbus::interface(name = "org.freedesktop.impl.portal.AppChooser")]
impl AppChooser {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The portal backend method signature is fixed by D-Bus."
    )]
    async fn choose_application(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        choices: Vec<String>,
        options: Options,
    ) -> (u32, Options) {
        let mime = text(&options, "content_type").unwrap_or_default();
        let current = Arc::new(Choices {
            state: Mutex::new(ChoiceState::default()),
            update: tokio::sync::Mutex::new(()),
            mime: mime.clone(),
        });
        let prepare = async {
            let item = text(&options, "filename")
                .or_else(|| text(&options, "uri"))
                .unwrap_or_else(|| mime.clone())
                .chars()
                .take(300)
                .collect();
            Ok(PortalDialog::new(
                app_id,
                parent_window,
                &options,
                PortalDialogKind::OpenWith {
                    item,
                    choices: resolve(choices, mime).await,
                    last_choice: text(&options, "last_choice"),
                },
            ))
        };
        match self
            .0
            .ask(server, handle, header, Some(Arc::clone(&current)), prepare)
            .await
        {
            Ok(PortalDialogAnswer::Application {
                id,
                activation_token,
            }) if current.contains(&id) => {
                let mut results = Options::from([("choice".into(), owned(Value::from(id)))]);
                if let Some(token) = activation_token {
                    results.insert("activation_token".into(), owned(Value::from(token)));
                }
                (0, results)
            }
            Err(response) => (response, Options::new()),
            _ => (1, Options::new()),
        }
    }

    async fn update_choices(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        choices: Vec<String>,
    ) -> zbus::fdo::Result<()> {
        let Ok(interface) = server.interface::<_, DialogRequest>(&handle).await else {
            return Ok(());
        };
        let request = interface.get().await;
        if header.sender() != Some(&*request.owner) {
            return Err(zbus::fdo::Error::AccessDenied(
                "Request belongs to another caller".into(),
            ));
        }
        if let Some(current) = &request.choices {
            let _update = current.update.lock().await;
            let choices = resolve(choices, current.mime.clone()).await;
            let mut state = current
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.shown {
                let _ = request
                    .events
                    .send(Event::AppChoicesChanged(request.id, choices.clone()));
            }
            state.latest = Some(choices);
        }
        Ok(())
    }
}
