use super::{Message as Parent, Page, SettingsApp};
use enzui_shell::background_apps::{Action, App, Capabilities, Client, Review, Snapshot, StopMode};
use enzui_shell::i18n::tr;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    modal_host::{ModalFocusLifecycle, ModalHostEvent, ModalPresentation, ModalPresentationKey},
    text::enz_text,
};
use iced::{Element, Subscription, Task};
use std::{
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

#[path = "background_settings_view.rs"]
mod view;

#[cfg(test)]
#[path = "background_settings_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct State {
    inspected: Option<String>,
    generation: u64,
    read_epoch: u64,
    operation: u64,
    client: Option<Client>,
    caps: Option<Capabilities>,
    snapshot: Option<Result<Snapshot, String>>,
    reading: bool,
    read_at: Option<Instant>,
    read_work: Option<iced::task::Handle>,
    operation_work: Option<iced::task::Handle>,
    expiry_work: Option<iced::task::Handle>,
    review: ModalPresentation<Pending>,
    committing: bool,
    error: Option<String>,
    notice: Option<String>,
    refresh_again: bool,
    dirty: bool,
    unchanged_reads: u8,
    event_read_after: Option<Instant>,
}

struct Pending {
    operation: u64,
    action: Action,
    result: Option<Result<Review, String>>,
    prepared: Option<Instant>,
    focus: ModalFocusLifecycle,
}

impl State {
    fn retire(&mut self) {
        let (generation, operation) = (self.generation.saturating_add(1), self.operation);
        self.abort();
        let mut review = std::mem::take(&mut self.review);
        review.dismiss();
        *self = Self::default();
        self.generation = generation;
        self.operation = operation;
        self.review = review;
    }

    fn observed(&mut self, caps: &Capabilities, snapshot: &Snapshot) {
        let unchanged = self.caps.as_ref() == Some(caps)
            && self.snapshot.as_ref().is_some_and(|previous| {
                previous
                    .as_ref()
                    .is_ok_and(|previous| previous.same_observation(snapshot))
            });
        self.unchanged_reads = if unchanged {
            self.unchanged_reads.saturating_add(1)
        } else {
            0
        };
        let delay = match self.unchanged_reads {
            0 => Duration::from_millis(250),
            1 => Duration::from_secs(1),
            2 => Duration::from_secs(5),
            _ => Duration::from_secs(30),
        };
        self.event_read_after = Some(Instant::now() + delay);
    }

    fn abort(&mut self) {
        for work in [
            &mut self.read_work,
            &mut self.operation_work,
            &mut self.expiry_work,
        ] {
            if let Some(work) = work.take() {
                work.abort();
            }
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.abort();
    }
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Inspect(Option<String>),
    DetailHost(ModalHostEvent),
    Refresh,
    Loaded(u64, u64, Result<(Client, Capabilities, Snapshot), String>),
    Changed(u64, String, Result<(), String>),
    Stop(String, StopMode),
    Portal(String, String),
    Policy(String, String, bool),
    Reviewed(u64, u64, Result<Review, String>),
    Confirm(u64),
    Cancel(u64),
    Expired(u64, u64),
    Applied(u64, u64, Result<(), String>),
    Host(u64, ModalHostEvent),
    Settled(ModalPresentationKey),
}

#[derive(Clone)]
struct Watch {
    generation: u64,
    client: Client,
}
impl Hash for Watch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.generation.hash(state);
        self.client.owner().hash(state);
    }
}

fn policy_available(app: &App, key: &str, caps: &Capabilities) -> bool {
    match key {
        "autostart" => {
            app.autostart_supported
                && app
                    .autostart_state
                    .as_deref()
                    .is_some_and(|s| matches!(s, "enabled" | "disabled"))
        }
        "agent" => caps.agent_policy_adapter,
        "search" => caps.search_policy_adapter,
        _ => false,
    }
}

fn action_present(snapshot: &Snapshot, action: &Action) -> bool {
    match action {
        Action::Stop { app, .. } | Action::Policy { app, .. } => {
            snapshot.apps.iter().any(|current| current == app)
        }
        Action::Portal(app) => snapshot
            .flatpak_background
            .as_ref()
            .is_some_and(|apps| apps.contains(app)),
    }
}

impl SettingsApp {
    pub(super) fn background_ready(&self) -> bool {
        self.background.snapshot.is_some()
    }

    pub(super) fn background_sync(&mut self) -> Task<Parent> {
        if self.page != Page::Apps || self.closing {
            if self.background.snapshot.is_some()
                || self.background.reading
                || self.background.review.is_open()
            {
                let close = self
                    .background
                    .review
                    .value()
                    .map_or_else(Task::none, |review| {
                        review.focus.close_for_host::<Parent>().into_task()
                    });
                self.background.retire();
                return close;
            }
            return Task::none();
        }
        if self.background.snapshot.is_none() {
            return self.background_read(false);
        }
        if self.background.refresh_again
            && !self.background.review.is_open()
            && !self.background.committing
        {
            self.background.refresh_again = false;
            return self.background_read(true);
        }
        if self.background.dirty
            && self.background.client.is_some()
            && !self.background.reading
            && !self.background.review.is_open()
            && !self.background.committing
            && self
                .background
                .event_read_after
                .is_none_or(|at| Instant::now() >= at)
        {
            return self.background_read(true);
        }
        if self.background.client.is_some()
            && self
                .background
                .read_at
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(30))
        {
            return self.background_read(false);
        }
        Task::none()
    }

    pub(super) fn background_read(&mut self, force: bool) -> Task<Parent> {
        if self.page != Page::Apps || self.closing {
            return Task::none();
        }
        let state = &mut self.background;
        if state.reading || state.committing || state.review.is_open() {
            state.refresh_again |= force;
            return Task::none();
        }
        if !force
            && state
                .read_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return Task::none();
        }
        state.read_epoch = state.read_epoch.saturating_add(1);
        let (generation, epoch) = (state.generation, state.read_epoch);
        state.reading = true;
        state.refresh_again = false;
        state.dirty = false;
        let client = state.client.clone();
        let (task, handle) = Task::perform(
            async move {
                match client {
                    Some(client) => client
                        .refresh()
                        .await
                        .map(|(caps, snapshot)| (client, caps, snapshot)),
                    None => Client::connect().await,
                }
            },
            move |result| Parent::Background(Message::Loaded(generation, epoch, result)),
        )
        .abortable();
        state.read_work = Some(handle);
        task
    }

    pub(super) fn background_subscription(&self) -> Subscription<Parent> {
        if self.page != Page::Apps || self.closing {
            return Subscription::none();
        }
        let Some(client) = self.background.client.as_ref() else {
            return Subscription::none();
        };
        Subscription::run_with(
            Watch {
                generation: self.background.generation,
                client: client.clone(),
            },
            |watch| {
                use iced::futures::StreamExt;
                let (generation, owner) = (watch.generation, watch.client.owner().to_owned());
                watch.client.clone().changes().map(move |result| {
                    Parent::Background(Message::Changed(generation, owner.clone(), result))
                })
            },
        )
    }

    fn background_dismiss(&mut self, restore: bool) -> Task<Parent> {
        let state = &mut self.background;
        if let Some(work) = state.operation_work.take() {
            work.abort();
        }
        if let Some(work) = state.expiry_work.take() {
            work.abort();
        }
        if !state.review.is_open() {
            return Task::none();
        }
        let task = state.review.value().map_or_else(Task::none, |review| {
            if restore {
                review
                    .focus
                    .reduce::<Parent>(ModalHostEvent::Dismissed(
                        enzui_core::modal::DismissCause::CancelAction,
                    ))
                    .into_task()
            } else {
                review.focus.close_for_host::<Parent>().into_task()
            }
        });
        state.review.dismiss();
        task
    }

    fn background_review(&mut self, action: Action) -> Task<Parent> {
        if !self.controls_live()
            || self.page != Page::Apps
            || self.background.review.is_open()
            || self.background.committing
        {
            return Task::none();
        }
        let state = &mut self.background;
        let (Some(client), Some(Ok(snapshot))) = (state.client.clone(), &state.snapshot) else {
            return Task::none();
        };
        if !action_present(snapshot, &action) {
            state.error = Some(tr("shell-the-app-changed-refresh-and-review-again").into());
            return Task::none();
        }
        if let Some(work) = state.read_work.take() {
            work.abort();
        }
        state.reading = false;
        state.read_epoch = state.read_epoch.saturating_add(1);
        let Some(operation) = state.operation.checked_add(1) else {
            return Task::none();
        };
        state.operation = operation;
        state.unchanged_reads = 0;
        state.event_read_after = None;
        let (generation, operation) = (state.generation, state.operation);
        let invoker = match &action {
            Action::Stop { app, mode } => format!("background-stop-{}-{}", app.app_id, mode.key()),
            Action::Portal(app) => format!("background-portal-{}", app.instance),
            Action::Policy { app, requested } => format!(
                "background-policy-{}-{}",
                app.app_id,
                requested
                    .iter()
                    .find(|(key, value)| app.activation_policy.get(*key) != Some(*value))
                    .map_or("autostart", |(key, _)| key.as_str())
            ),
        };
        let (focus, open) = ModalFocusLifecycle::open_at(
            &format!("background-review-{generation}-{operation}"),
            Some(invoker.into()),
            "background-review-cancel".into(),
        );
        state.review.present(Pending {
            operation,
            action: action.clone(),
            result: None,
            prepared: None,
            focus,
        });
        state.error = None;
        let (task, handle) =
            Task::perform(async move { client.review(action).await }, move |result| {
                Parent::Background(Message::Reviewed(generation, operation, result))
            })
            .abortable();
        state.operation_work = Some(handle);
        Task::batch([task, iced::advanced::widget::operate(open)])
    }

    pub(super) fn background_message(&mut self, message: Message) -> Task<Parent> {
        match message {
            Message::Inspect(id) if !self.background.committing => {
                self.background.inspected = id;
                Task::none()
            }
            Message::DetailHost(ModalHostEvent::Dismissed(_)) if !self.background.committing => {
                self.background.inspected = None;
                Task::none()
            }
            Message::DetailHost(event) => enzui_widgets::modal_host::focus_wrap_operation(event)
                .map_or_else(Task::none, iced::advanced::widget::operate),
            Message::Inspect(_) => Task::none(),
            Message::Refresh => {
                self.background.unchanged_reads = 0;
                self.background.event_read_after = None;
                self.background_read(true)
            }
            Message::Loaded(generation, epoch, result)
                if generation == self.background.generation
                    && epoch == self.background.read_epoch
                    && self.page == Page::Apps
                    && !self.closing =>
            {
                let state = &mut self.background;
                state.reading = false;
                state.read_work = None;
                state.read_at = Some(Instant::now());
                match result {
                    Ok((client, caps, snapshot)) => {
                        state.observed(&caps, &snapshot);
                        state.client = Some(client);
                        state.caps = Some(caps);
                        state.snapshot = Some(Ok(snapshot));
                    }
                    Err(error) => {
                        state.client = None;
                        state.caps = None;
                        state.snapshot = Some(Err(error));
                    }
                }
                if std::mem::take(&mut state.refresh_again) {
                    self.background_read(true)
                } else {
                    Task::none()
                }
            }
            Message::Changed(generation, owner, result)
                if generation == self.background.generation
                    && self
                        .background
                        .client
                        .as_ref()
                        .is_some_and(|client| client.owner() == owner) =>
            {
                if let Err(error) = result {
                    let committing = self.background.committing;
                    let close = self.background_dismiss(true);
                    self.background.retire();
                    self.background.snapshot = Some(Err(error));
                    if committing {
                        self.background.error = Some(tr("shell-the-service-changed-before-the-outcome-was-confirmed-refresh-before-trying-a").into());
                    }
                    return close;
                }
                self.background.dirty = true;
                Task::none()
            }
            Message::Stop(id, mode) => {
                let Some(Ok(snapshot)) = &self.background.snapshot else {
                    return Task::none();
                };
                let Some(app) = snapshot
                    .apps
                    .iter()
                    .find(|app| app.app_id == id && app.can_stop(mode))
                    .cloned()
                else {
                    return Task::none();
                };
                self.background_review(Action::Stop { app, mode })
            }
            Message::Portal(id, instance) => {
                let Some(Ok(snapshot)) = &self.background.snapshot else {
                    return Task::none();
                };
                let Some(app) = snapshot
                    .flatpak_background
                    .as_ref()
                    .and_then(|apps| {
                        apps.iter()
                            .find(|app| app.app_id == id && app.instance == instance)
                    })
                    .cloned()
                else {
                    return Task::none();
                };
                self.background_review(Action::Portal(app))
            }
            Message::Policy(id, key, enabled) => {
                let (Some(Ok(snapshot)), Some(caps)) =
                    (&self.background.snapshot, &self.background.caps)
                else {
                    return Task::none();
                };
                let Some(app) = snapshot
                    .apps
                    .iter()
                    .find(|app| app.app_id == id && policy_available(app, &key, caps))
                    .cloned()
                else {
                    return Task::none();
                };
                if snapshot
                    .policy_operation
                    .as_ref()
                    .is_some_and(enzui_shell::background_apps::PolicyOperation::pending)
                {
                    return Task::none();
                }
                let mut requested = app.activation_policy.clone();
                if requested.get(&key).is_none_or(|value| *value == enabled) {
                    return Task::none();
                }
                requested.insert(key, enabled);
                self.background_review(Action::Policy { app, requested })
            }
            Message::Reviewed(generation, operation, result)
                if generation == self.background.generation
                    && self.background.review.is_open()
                    && self
                        .background
                        .review
                        .value()
                        .is_some_and(|review| review.operation == operation) =>
            {
                let state = &mut self.background;
                state.operation_work = None;
                let review = state
                    .review
                    .value_mut()
                    .expect("the review was checked before handling this action");
                review.prepared = Some(Instant::now());
                review.result = Some(result);
                let (task, handle) = Task::perform(
                    async { tokio::time::sleep(Duration::from_secs(30)).await },
                    move |()| Parent::Background(Message::Expired(generation, operation)),
                )
                .abortable();
                state.expiry_work = Some(handle);
                task
            }
            Message::Confirm(operation) if self.controls_live() && !self.background.committing => {
                let state = &mut self.background;
                let (Some(client), Some(pending)) = (
                    state.client.clone(),
                    state
                        .review
                        .value()
                        .filter(|review| state.review.is_open() && review.operation == operation),
                ) else {
                    return Task::none();
                };
                let Some(Ok(review)) = &pending.result else {
                    return Task::none();
                };
                if pending
                    .prepared
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
                {
                    state.error =
                        Some(tr("shell-this-review-expired-review-the-action-again").into());
                    return self.background_dismiss(true);
                }
                let review = review.clone();
                let generation = state.generation;
                state.committing = true;
                if let Some(work) = state.expiry_work.take() {
                    work.abort();
                }
                let (task, handle) =
                    Task::perform(async move { client.commit(&review).await }, move |result| {
                        Parent::Background(Message::Applied(generation, operation, result))
                    })
                    .abortable();
                state.operation_work = Some(handle);
                task
            }
            Message::Cancel(operation)
                if !self.background.committing
                    && self.background.review.is_open()
                    && self
                        .background
                        .review
                        .value()
                        .is_some_and(|review| review.operation == operation) =>
            {
                self.background_dismiss(true)
            }
            Message::Expired(generation, operation)
                if generation == self.background.generation
                    && !self.background.committing
                    && self.background.review.is_open()
                    && self
                        .background
                        .review
                        .value()
                        .is_some_and(|review| review.operation == operation) =>
            {
                self.background.error =
                    Some(tr("shell-this-review-expired-review-the-action-again").into());
                self.background_dismiss(true)
            }
            Message::Applied(generation, operation, result)
                if generation == self.background.generation
                    && self.background.committing
                    && self.background.review.is_open()
                    && self
                        .background
                        .review
                        .value()
                        .is_some_and(|review| review.operation == operation) =>
            {
                self.background.committing = false;
                self.background.operation_work = None;
                match result {
                    Ok(()) => {
                        self.background.error = None;
                        self.background.notice = Some(
                            tr("shell-request-sent-the-list-below-shows-the-latest-observed-state")
                                .into(),
                        );
                    }
                    Err(error) => {
                        self.background.error = Some(error);
                        self.background.notice = None;
                    }
                }
                let close = self.background_dismiss(true);
                Task::batch([close, self.background_read(true)])
            }
            Message::Host(operation, event) if !self.background.committing => {
                let Some(pending) = self.background.review.value().filter(|review| {
                    self.background.review.is_open() && review.operation == operation
                }) else {
                    return Task::none();
                };
                let outcome = pending.focus.reduce::<Parent>(event);
                if outcome.dismissed().is_some() {
                    self.background.review.dismiss();
                    if let Some(work) = self.background.operation_work.take() {
                        work.abort();
                    }
                    if let Some(work) = self.background.expiry_work.take() {
                        work.abort();
                    }
                }
                outcome.into_task()
            }
            Message::Settled(key) => {
                self.background.review.finish_exit(key);
                Task::none()
            }
            _ => Task::none(),
        }
    }
}
