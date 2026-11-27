//! Print requests served by the Shell's desktop portal.

#[path = "portal_print/cups.rs"]
mod cups;
#[path = "portal_print/options.rs"]
mod options;

pub use options::{Duplex, Orientation, Pages, Paper, Printer, Selection};

use super::{Chooser, Event, PortalRequest, owned, text};
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, oneshot};
use zbus::names::OwnedUniqueName;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

type Dict = HashMap<String, OwnedValue>;
type Response = (u32, Dict);

const TOKEN_TTL: Duration = Duration::from_secs(600);
const MAX_TOKENS: usize = 256;

/// A print dialog and its starting settings.
#[derive(Debug)]
pub struct DialogRequest {
    /// Identifier shared with Request.Close.
    pub id: u64,
    /// XDG portal parent window identifier.
    pub parent_window: String,
    /// Whether the dialog blocks interaction with its parent.
    pub modal: bool,
    /// App-supplied title.
    pub title: String,
    /// Label for the primary action.
    pub accept_label: Option<String>,
    /// Available queues, or a discovery failure.
    pub printers: Result<Vec<Printer>, String>,
    /// Selection initialized from the app's hints and the printer's defaults.
    pub selection: Option<Selection>,
    /// Whether the requesting app can render its current page.
    pub has_current_page: bool,
    /// Whether the requesting app can render selected pages.
    pub has_selected_pages: bool,
}

/// Outcome of a Shell print dialog.
#[derive(Debug)]
pub enum DialogAnswer {
    /// The user chose a printer and confirmed the settings.
    Chosen(Box<Selection>),
    /// No usable destination was available.
    Unavailable,
}

#[derive(PartialEq, Eq)]
struct Caller {
    sender: Option<OwnedUniqueName>,
    app: String,
    request_scope: String,
}

impl Caller {
    fn new(header: &zbus::message::Header<'_>, handle: &ObjectPath<'_>, app: &str) -> Self {
        Self {
            sender: header.sender().map(|name| name.to_owned().into()),
            app: app.to_owned(),
            // The frontend multiplexes all apps over one connection, including host apps with no ID.
            request_scope: handle
                .as_str()
                .rsplit_once('/')
                .map_or("", |(scope, _)| scope)
                .into(),
        }
    }
}

struct Prepared {
    caller: Caller,
    expires: Instant,
    selection: Selection,
}

#[derive(Default)]
struct Tokens(HashMap<u32, Prepared>);

impl Tokens {
    fn issue(
        &mut self,
        caller: Caller,
        selection: Selection,
        now: Instant,
    ) -> std::io::Result<u32> {
        self.0.retain(|_, prepared| prepared.expires > now);
        if self.0.len() == MAX_TOKENS
            && let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, prepared)| prepared.expires)
                .map(|(id, _)| *id)
        {
            self.0.remove(&oldest);
        }
        let mut random = std::fs::File::open("/dev/urandom")?;
        loop {
            let mut bytes = [0; 4];
            std::io::Read::read_exact(&mut random, &mut bytes)?;
            let token = u32::from_ne_bytes(bytes);
            if token != 0 && !self.0.contains_key(&token) {
                self.0.insert(
                    token,
                    Prepared {
                        caller,
                        expires: now + TOKEN_TTL,
                        selection,
                    },
                );
                return Ok(token);
            }
        }
    }

    fn take(&mut self, token: u32, caller: &Caller, now: Instant) -> Option<Selection> {
        self.0.retain(|_, prepared| prepared.expires > now);
        if self.0.get(&token)?.caller != *caller {
            return None;
        }
        self.0.remove(&token).map(|prepared| prepared.selection)
    }
}

pub(super) struct Print {
    chooser: Arc<Chooser>,
    tokens: Mutex<Tokens>,
}

impl Print {
    pub(super) fn new(chooser: Arc<Chooser>) -> Self {
        Self {
            chooser,
            tokens: Mutex::default(),
        }
    }

    async fn request<F, Fut>(
        &self,
        server: &zbus::ObjectServer,
        handle: ObjectPath<'_>,
        run: F,
    ) -> Response
    where
        F: FnOnce(u64, Arc<Notify>) -> Fut,
        Fut: Future<Output = Response>,
    {
        let id = self.chooser.next_id();
        let cancel = Arc::new(Notify::new());
        let handle: OwnedObjectPath = handle.into();
        match server
            .at(
                &handle,
                PortalRequest {
                    id,
                    events: self.chooser.events.clone(),
                    cancel: Some(Arc::clone(&cancel)),
                },
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => return failure("The print request is already open."),
            Err(error) => return failure(error),
        }
        let response = run(id, cancel).await;
        let _ = server.remove::<PortalRequest, _>(&handle).await;
        response
    }

    async fn ask(
        &self,
        id: u64,
        parent_window: &str,
        title: &str,
        // The caller's previous settings and page setup.
        (settings, page_setup): (&Dict, &Dict),
        options: &Dict,
        cancel: &Notify,
    ) -> Result<Selection, Response> {
        let (printers, selection);
        tokio::select! {
            biased;
            () = cancel.notified() => return Err(cancelled()),
            result = tokio::task::spawn_blocking(cups::printers) => {
                printers = result.unwrap_or_else(|error| Err(error.to_string()));
            }
        }
        let has_current_page = super::flag(options, "has_current_page");
        let has_selected_pages = super::flag(options, "has_selected_pages");
        selection = printers.as_ref().ok().and_then(|printers| {
            Selection::from_hints(
                printers,
                settings,
                page_setup,
                has_current_page,
                has_selected_pages,
            )
        });
        let request = DialogRequest {
            id,
            parent_window: parent_window.chars().take(512).collect(),
            modal: options
                .get("modal")
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(true),
            title: title.chars().take(200).collect(),
            accept_label: text(options, "accept_label").map(|label| {
                label
                    .replace("__", "\0")
                    .replace('_', "")
                    .replace('\0', "_")
                    .chars()
                    .take(48)
                    .collect()
            }),
            printers,
            selection,
            has_current_page,
            has_selected_pages,
        };
        let (reply, answer) = oneshot::channel();
        if self
            .chooser
            .events
            .send(Event::Print(request, reply))
            .is_err()
        {
            return Err(failure("The print dialog could not be opened."));
        }
        tokio::select! {
            biased;
            () = cancel.notified() => Err(cancelled()),
            answer = answer => match answer {
                Ok(DialogAnswer::Chosen(selection)) => Ok(*selection),
                Ok(DialogAnswer::Unavailable) => Err(failure("No printer is available.")),
                Err(_) => Err(cancelled()),
            },
        }
    }
}

fn cancelled() -> Response {
    (1, Dict::new())
}

fn failure(error: impl std::fmt::Display) -> Response {
    let error = error.to_string();
    eprintln!("enzui-portal: Print: {error}");
    (2, Dict::from([("error".into(), owned(Value::from(error)))]))
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Print")]
impl Print {
    #[allow(
        clippy::too_many_arguments,
        reason = "The arguments are defined by the portal D-Bus interface."
    )]
    #[zbus(out_args("response", "results"))]
    async fn prepare_print(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        settings: Dict,
        page_setup: Dict,
        options: Dict,
    ) -> Response {
        let caller = Caller::new(&header, &handle, app_id);
        self.request(server, handle, |id, cancel| async move {
            let selection = match self
                .ask(
                    id,
                    parent_window,
                    title,
                    (&settings, &page_setup),
                    &options,
                    &cancel,
                )
                .await
            {
                Ok(selection) => selection,
                Err(response) => return response,
            };
            let (settings, page_setup) = selection.vardicts();
            let tokens = self
                .tokens
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .issue(caller, selection, Instant::now());
            match tokens {
                Ok(token) => (
                    0,
                    Dict::from([
                        ("settings".into(), owned(Value::from(settings))),
                        ("page-setup".into(), owned(Value::from(page_setup))),
                        ("token".into(), token.into()),
                    ]),
                ),
                Err(error) => failure(error),
            }
        })
        .await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The arguments are defined by the portal D-Bus interface."
    )]
    #[zbus(out_args("response", "results"))]
    async fn print(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        fd: zbus::zvariant::OwnedFd,
        options: Dict,
    ) -> Response {
        let caller = Caller::new(&header, &handle, app_id);
        self.request(server, handle, |id, cancel| async move {
            let prepared = options
                .get("token")
                .and_then(|token| u32::try_from(token).ok())
                .and_then(|token| {
                    self.tokens
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take(token, &caller, Instant::now())
                });
            let (selection, prepared) = match prepared {
                Some(selection) => (selection, true),
                None => match self
                    .ask(
                        id,
                        parent_window,
                        title,
                        (&Dict::new(), &Dict::new()),
                        &options,
                        &cancel,
                    )
                    .await
                {
                    Ok(selection) => (selection, false),
                    Err(response) => return response,
                },
            };
            let title: String = title
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            let document: OwnedFd = fd.into();
            // After handing the job to CUPS, report the scheduler's actual outcome.
            tokio::select! {
                biased;
                () = cancel.notified() => return cancelled(),
                () = std::future::ready(()) => {}
            }
            match tokio::task::spawn_blocking(move || {
                cups::submit(&selection, &title, document, prepared)
            })
            .await
            {
                Ok(Ok(job)) => (0, Dict::from([("job-id".into(), job.into())])),
                Ok(Err(error)) => failure(error),
                Err(error) => failure(error),
            }
        })
        .await
    }
}

#[cfg(test)]
#[path = "portal_print/tests.rs"]
mod tests;
