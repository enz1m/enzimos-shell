use super::{Message as ParentMessage, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::sandbox_policy::{self, Category, Edit, Entry, Grant, Portals, Snapshot, Source};
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    text::enz_text,
};
use iced::{Element, Task, widget::column};

#[cfg(test)]
#[path = "sandbox_settings_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct State {
    open: Option<String>,
    generation: u64,
    snapshot: Option<Result<Snapshot, String>>,
    portals: Option<Result<Portals, String>>,
    busy: bool,
    pending: Option<u64>,
    operation_epoch: u64,
    error: Option<String>,
    advanced: bool,
    filter: String,
    work: Option<iced::task::Handle>,
    focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            work.abort();
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Toggle(String),
    Loaded(u64, String, Result<Snapshot, String>),
    Edit(u64, String, Edit),
    Reset(u64, String),
    Applied(u64, u64, String, Result<Snapshot, String>),
    Advanced(bool),
    Portals(u64, String, Result<Portals, String>),
    Answer(u64, String, Grant, Option<bool>),
    Answered(u64, u64, String, Result<Portals, String>),
    Refresh(u64),
    Filter(String),
    Host(u64, enzui_widgets::modal_host::ModalHostEvent),
}

fn label(entry: &Entry) -> String {
    let (name, mode) = entry
        .value
        .split_once(':')
        .map_or((entry.value.as_str(), ""), |(name, mode)| (name, mode));
    let known = match (entry.category, name) {
        (Category::Shared, "network") => tr("shell-network"),
        (Category::Shared, "ipc") => tr("shell-shared-memory-with-other-programs"),
        (Category::Sockets, "x11") => "X11 display",
        (Category::Sockets, "fallback-x11") => tr("shell-x11-display-when-wayland-is-missing"),
        (Category::Sockets, "wayland") => tr("shell-wayland-display"),
        (Category::Sockets, "pulseaudio") => tr("shell-sound-and-microphones-pulseaudio"),
        (Category::Sockets, "session-bus") => tr("shell-every-session-service"),
        (Category::Sockets, "system-bus") => tr("shell-every-system-service"),
        (Category::Sockets, "ssh-auth") => tr("shell-ssh-keys"),
        (Category::Sockets, "pcsc") => tr("shell-smart-cards"),
        (Category::Sockets, "cups") => tr("shell-printing"),
        (Category::Sockets, "gpg-agent") => tr("shell-gpg-keys"),
        (Category::Devices, "all") => tr("shell-all-devices"),
        (Category::Devices, "dri") => tr("shell-graphics-acceleration"),
        (Category::Devices, "kvm") => tr("shell-virtual-machines"),
        (Category::Devices, "shm") => tr("shell-shared-memory-devices"),
        (Category::Devices, "input") => tr("shell-input-devices"),
        (Category::Devices, "usb") => tr("shell-usb-devices"),
        (Category::Features, "devel") => tr("shell-debugging-other-programs"),
        (Category::Features, "multiarch") => tr("shell-32-bit-programs"),
        (Category::Features, "bluetooth") => tr("shell-bluetooth"),
        (Category::Features, "canbus") => tr("shell-can-bus"),
        (Category::Features, "per-app-dev-shm") => tr("shell-private-shared-memory"),
        (Category::Filesystems, "host") => tr("shell-all-files"),
        (Category::Filesystems, "host-os") => tr("shell-system-files"),
        (Category::Filesystems, "host-etc") => tr("shell-system-settings-files"),
        (Category::Filesystems, "home") => tr("shell-home-folder"),
        (Category::Filesystems, "xdg-download") => tr("shell-downloads"),
        (Category::Filesystems, "xdg-documents") => tr("shell-documents"),
        (Category::Filesystems, "xdg-music") => tr("shell-music"),
        (Category::Filesystems, "xdg-pictures") => tr("shell-pictures"),
        (Category::Filesystems, "xdg-videos") => tr("shell-videos"),
        (Category::Filesystems, "xdg-desktop") => tr("shell-desktop-folder"),
        _ => "",
    };
    let name = if known.is_empty() {
        name.to_owned()
    } else {
        known.to_owned()
    };
    match mode {
        "ro" => tr!("shell-read-only", "arg0" => name),
        "create" => tr!("shell-can-create", "arg0" => name),
        "rw" => tr!("shell-read-and-write", "arg0" => name),
        "" => name,
        other => {
            tr!("shell-mode", "arg0" => name, "arg1" => other.to_owned())
        }
    }
}

impl SettingsApp {
    pub(super) fn sandbox_leave(&mut self) {
        if let Some(app) = self.sandbox.open.clone() {
            let _ = self.sandbox_message(Message::Toggle(app));
        }
    }
    pub(super) fn sandbox_message(&mut self, message: Message) -> Task<ParentMessage> {
        if let Message::Toggle(app) = &message
            && self.sandbox.open.as_deref() != Some(app)
            && (!self.controls_live()
                || !self
                    .reading(enzui_shell::system_settings::Panel::Apps)
                    .is_some_and(|reading| match reading {
                        enzui_shell::system_settings::Reading::Apps(apps) => apps
                            .permissions
                            .apps
                            .as_ref()
                            .is_some_and(|apps| apps.iter().any(|known| known.id == *app)),
                        _ => false,
                    }))
        {
            return Task::none();
        }
        let state = &mut self.sandbox;
        match message {
            Message::Toggle(app) => {
                let Some(generation) = state.generation.checked_add(1) else {
                    return Task::none();
                };
                let filter = std::mem::take(&mut state.filter);
                if let Some(work) = state.work.take() {
                    work.abort();
                }
                if state.open.as_deref() == Some(app.as_str()) {
                    let mut next = State::default();
                    next.generation = generation;
                    next.filter = filter;
                    *state = next;
                    return Task::none();
                }
                let worker = sandbox_policy::worker().is_some();
                if self.closing {
                    return Task::none();
                }
                let mut next = State::default();
                next.open = Some(app.clone());
                next.generation = generation;
                next.busy = worker;
                next.filter = filter;
                *state = next;
                let (focus, operation) = enzui_widgets::modal_host::ModalFocusLifecycle::open_at(
                    &format!("app-permissions-{generation}"),
                    Some(format!("app-permissions-{app}").into()),
                    "app-permissions-done".into(),
                );
                state.focus = Some(focus);
                let focus_task = iced::advanced::widget::operate(operation);
                if !worker {
                    return focus_task;
                }
                let portal_app = app.clone();
                let (task, handle) = Task::batch([
                    Task::perform(sandbox_policy::inspect(app.clone()), move |result| {
                        ParentMessage::Sandbox(Message::Loaded(generation, app, result))
                    }),
                    Task::perform(
                        sandbox_policy::portal_inspect(portal_app.clone()),
                        move |result| {
                            ParentMessage::Sandbox(Message::Portals(generation, portal_app, result))
                        },
                    ),
                ])
                .abortable();
                state.work = Some(handle);
                Task::batch([task, focus_task])
            }
            Message::Filter(filter) => {
                state.filter = filter.chars().take(128).collect();
                Task::none()
            }
            Message::Host(generation, event)
                if generation == state.generation && state.open.is_some() =>
            {
                let Some(focus) = state.focus.as_ref() else {
                    return Task::none();
                };
                let outcome = focus.reduce(event);
                let dismissed = outcome.dismissed().is_some();
                let task = outcome
                    .into_operation()
                    .map_or_else(Task::none, iced::advanced::widget::operate);
                if dismissed {
                    self.sandbox_leave();
                }
                task
            }
            Message::Host(..) => Task::none(),
            Message::Loaded(generation, app, result)
                if !self.closing
                    && generation == state.generation
                    && state.open.as_deref() == Some(app.as_str())
                    && state.pending.is_none() =>
            {
                state.busy = false;
                state.snapshot = Some(result);
                Task::none()
            }
            Message::Applied(generation, operation, app, result)
                if !self.closing
                    && generation == state.generation
                    && state.open.as_deref() == Some(app.as_str())
                    && state.pending == Some(operation) =>
            {
                state.busy = false;
                state.pending = None;
                match result {
                    Ok(snapshot) => {
                        state.snapshot = Some(Ok(snapshot));
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
                Task::none()
            }
            Message::Advanced(on) => {
                state.advanced = on;
                Task::none()
            }
            Message::Portals(generation, app, result)
                if !self.closing
                    && generation == state.generation
                    && state.open.as_deref() == Some(app.as_str())
                    && state.pending.is_none() =>
            {
                state.portals = Some(result);
                Task::none()
            }
            Message::Portals(..) => Task::none(),
            Message::Answer(generation, token, grant, allow) => {
                if self.closing
                    || generation != state.generation
                    || state.busy
                    || state.pending.is_some()
                    || state.error.is_some()
                {
                    return Task::none();
                }
                let Some(Ok(portals)) = state.portals.clone() else {
                    return Task::none();
                };
                if state.portals.is_none()
                    || token != portals.review_token
                    || state.open.as_deref() != Some(&portals.app_id)
                    || !portals.grants.contains(&grant)
                    || (allow.is_some() && !grant.switchable())
                {
                    return Task::none();
                }
                let Some(operation) = state.operation_epoch.checked_add(1) else {
                    return Task::none();
                };
                state.operation_epoch = operation;
                state.pending = Some(operation);
                let (task, handle) = Task::perform(
                    sandbox_policy::portal_change(portals.clone(), grant, allow),
                    move |result| {
                        ParentMessage::Sandbox(Message::Answered(
                            generation,
                            operation,
                            portals.app_id,
                            result,
                        ))
                    },
                )
                .abortable();
                if let Some(work) = state.work.replace(handle) {
                    work.abort();
                }
                task
            }
            Message::Answered(generation, operation, app, result)
                if !self.closing
                    && generation == state.generation
                    && state.open.as_deref() == Some(app.as_str())
                    && state.pending == Some(operation) =>
            {
                state.pending = None;
                match result {
                    Ok(portals) => {
                        state.portals = Some(Ok(portals));
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
                Task::none()
            }
            Message::Refresh(generation) => {
                if self.closing
                    || generation != state.generation
                    || state.pending.is_some()
                    || state.busy
                {
                    return Task::none();
                }
                let Some(app) = state.open.clone() else {
                    return Task::none();
                };
                let _ = self.sandbox_message(Message::Toggle(app.clone()));
                self.sandbox_message(Message::Toggle(app))
            }
            Message::Loaded(..) | Message::Applied(..) | Message::Answered(..) => Task::none(),
            Message::Edit(generation, token, edit) => {
                self.sandbox_apply(generation, token, vec![edit], false)
            }
            Message::Reset(generation, token) => {
                self.sandbox_apply(generation, token, Vec::new(), true)
            }
        }
    }
    fn sandbox_apply(
        &mut self,
        generation: u64,
        token: String,
        edits: Vec<Edit>,
        reset: bool,
    ) -> Task<ParentMessage> {
        let state = &mut self.sandbox;
        if self.closing
            || generation != state.generation
            || state.pending.is_some()
            || state.error.is_some()
        {
            return Task::none();
        }
        let Some(Ok(snapshot)) = state.snapshot.clone().filter(|_| !state.busy) else {
            return Task::none();
        };
        if token != snapshot.review_token
            || !snapshot.writable
            || state.portals.is_none()
            || state.open.as_deref() != Some(&snapshot.app_id)
        {
            return Task::none();
        }
        let Some(operation) = state.operation_epoch.checked_add(1) else {
            return Task::none();
        };
        state.operation_epoch = operation;
        state.pending = Some(operation);
        state.busy = true;
        let app = snapshot.app_id.clone();
        let (task, handle) = Task::perform(
            sandbox_policy::apply(snapshot, edits, reset),
            move |result| {
                ParentMessage::Sandbox(Message::Applied(generation, operation, app, result))
            },
        )
        .abortable();
        if let Some(work) = state.work.replace(handle) {
            work.abort();
        }
        task
    }
    pub(super) fn sandbox_open(&self) -> Option<&str> {
        self.sandbox.open.as_deref()
    }
    pub(super) fn sandbox_generation(&self) -> u64 {
        self.sandbox.generation
    }
    pub(super) fn sandbox_focus(&self) -> Option<&enzui_widgets::modal_host::ModalFocusLifecycle> {
        self.sandbox.focus.as_ref()
    }
    pub(super) fn sandbox_filter(&self) -> &str {
        &self.sandbox.filter
    }
    pub(super) fn sandbox_details<'a>(&'a self) -> Option<Element<'a, ParentMessage>> {
        self.sandbox.open.as_ref()?;
        let palette = self.tokens().palette;
        let note = |text: String| {
            enz_text(text)
                .size_px(self.text_px(12.0))
                .muted_on(palette.surface)
                .wrap(true)
                .into_element()
        };
        let snapshot = match &self.sandbox.snapshot {
            None => {
                return Some(note(tr("shell-reading-this-app-s-sandbox").into()));
            }
            Some(Err(error)) => {
                return Some(
                    column![
                        note(error.clone()),
                        enz_button(
                            tr("shell-refresh-review"),
                            ParentMessage::Sandbox(Message::Refresh(self.sandbox.generation))
                        )
                        .size(ButtonSize::Sm)
                        .into_element()
                    ]
                    .into(),
                );
            }
            Some(Ok(snapshot)) => snapshot,
        };
        let generation = self.sandbox.generation;
        let live = snapshot.writable
            && self.sandbox.portals.is_some()
            && !self.sandbox.busy
            && self.sandbox.pending.is_none()
            && self.sandbox.error.is_none()
            && self.controls_live();
        let mut body = column![].spacing(4);
        if let Some(error) = &self.sandbox.error {
            body = body.push(note(tr!("shell-showing-the-last-confirmed-review-refresh-before-another-change", "arg0" => error.clone())))
                .push(enz_button(tr("shell-refresh-review"), ParentMessage::Sandbox(Message::Refresh(generation))).size(ButtonSize::Sm).enabled(self.sandbox.pending.is_none()).into_element());
        }
        if self.sandbox.pending.is_some() {
            body = body.push(note(
                tr("shell-saving-displayed-values-are-the-last-readback").into(),
            ));
        }
        let heading = |text: &'static str| -> Element<'a, ParentMessage> {
            iced::widget::container(
                enz_text(text)
                    .size_px(self.text_px(15.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding(iced::Padding {
                top: 8.0,
                bottom: 2.0,
                ..Default::default()
            })
            .into()
        };
        let group = |text: &'static str| -> Element<'a, ParentMessage> {
            iced::widget::container(
                enz_text(text)
                    .size_px(self.text_px(12.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .muted_on(palette.surface)
                    .into_element(),
            )
            .padding(iced::Padding {
                top: 8.0,
                left: 4.0,
                ..Default::default()
            })
            .into()
        };
        body = body.push(heading(tr("shell-saved-portal-choices")));
        body = body.push(note(tr("shell-without-a-saved-choice-each-portal-applies-its-own-default-forget-removes-th").into()));
        match &self.sandbox.portals {
            None => body = body.push(note(tr("shell-reading").into())),
            Some(Err(error)) => {
                body = body.push(note(error.clone())).push(
                    enz_button(
                        tr("shell-refresh-review"),
                        ParentMessage::Sandbox(Message::Refresh(generation)),
                    )
                    .size(ButtonSize::Sm)
                    .enabled(!self.sandbox.busy && self.sandbox.pending.is_none())
                    .into_element(),
                );
            }
            Some(Ok(portals)) => {
                for grant in &portals.grants {
                    let row = crate::settings_app::settings_row(grant.label());
                    let row = if grant.switchable() {
                        let (on, toggled, forgotten) =
                            (grant.values == ["yes"], grant.clone(), grant.clone());
                        let (token, forget_token) =
                            (portals.review_token.clone(), portals.review_token.clone());
                        row.control(
                            iced::widget::row![
                                enzui_widgets::toggle::enz_toggle(on, "", move |allow| {
                                    ParentMessage::Sandbox(Message::Answer(
                                        generation,
                                        token.clone(),
                                        toggled.clone(),
                                        Some(allow),
                                    ))
                                })
                                .aria_label(enzui_shell::tr!("shell-a11y-allow", "arg0" => enzui_shell::i18n::argument(&(grant.label()))))
                                .enabled(
                                    self.controls_live()
                                        && self.sandbox.pending.is_none()
                                        && self.sandbox.error.is_none()
                                )
                                .into_element(),
                                enz_button(
                                    enzui_shell::i18n::tr("shell-forget"),
                                    ParentMessage::Sandbox(Message::Answer(
                                        generation,
                                        forget_token,
                                        forgotten,
                                        None
                                    ))
                                )
                                .kind(ButtonKind::Quiet)
                                .size(ButtonSize::Sm)
                                .enabled(
                                    self.controls_live()
                                        && self.sandbox.pending.is_none()
                                        && self.sandbox.error.is_none()
                                )
                                .into_element(),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                        )
                    } else {
                        row.subtitle(tr!("shell-stored-as-shown-as-it-is", "arg0" => grant.values.join(", ").clone()))
                    };
                    body = body.push(row.into_element());
                }
                for (table, resource, label) in [
                    ("devices", "camera", tr("shell-camera")),
                    ("devices", "microphone", tr("shell-microphone")),
                    ("notifications", "notification", tr("shell-notifications")),
                    (
                        "background",
                        "background",
                        tr("shell-run-in-the-background"),
                    ),
                    ("screenshot", "screenshot", tr("shell-take-screenshots")),
                    ("location", "location", tr("shell-location")),
                ] {
                    if !portals
                        .grants
                        .iter()
                        .any(|g| g.table == table && g.resource == resource)
                    {
                        body = body.push(
                            crate::settings_app::settings_row(label)
                                .subtitle(tr("shell-no-saved-choice"))
                                .into_element(),
                        );
                    }
                }
                if snapshot.entries.iter().any(|e| {
                    e.allowed && e.category == Category::Sockets && e.value == "pulseaudio"
                }) {
                    body = body.push(note(tr("shell-its-direct-sound-access-below-also-reaches-microphones-without-asking").into()));
                }
            }
        }
        body = body.push(heading(tr("shell-installed-access")));
        body = body.push(note(
            tr("shell-access-the-app-was-installed-with-as-flatpak-computes-it-changes-are-yours-a").into()
        ));
        for category in [
            Category::Shared,
            Category::Sockets,
            Category::Devices,
            Category::Features,
            Category::Filesystems,
            Category::Persistent,
            Category::SessionBus,
            Category::SystemBus,
            Category::Environment,
        ] {
            let entries: Vec<&Entry> = snapshot
                .entries
                .iter()
                .filter(|e| e.category == category)
                .collect();
            if entries.is_empty() {
                continue;
            }
            body = body.push(group(category.title()));
            for entry in entries {
                let mut subtitle = entry.source.label().to_owned();
                if let Some(detail) = entry
                    .detail
                    .as_deref()
                    .filter(|_| matches!(category, Category::SessionBus | Category::SystemBus))
                {
                    subtitle = format!(
                        "{} · {subtitle}",
                        if detail == "own" {
                            tr("shell-can-own-the-name")
                        } else if detail == "none" {
                            enzui_shell::i18n::tr("shell-blocked")
                        } else {
                            tr("shell-can-talk-to-it")
                        }
                    );
                }
                if let Some(caveat) = entry.caveat() {
                    subtitle = format!("{subtitle} · {caveat}");
                }
                let title = match category {
                    Category::Environment => format!(
                        "{} = {}",
                        entry.value,
                        entry.detail.as_deref().unwrap_or("")
                    ),
                    Category::SessionBus | Category::SystemBus | Category::Persistent => {
                        entry.value.clone()
                    }
                    _ => label(entry),
                };
                let row = crate::settings_app::settings_row(title)
                    .subtitle(subtitle)
                    .subtitle_wrap(true);
                let row = match category {
                    _ if !entry.editable => row,
                    Category::Filesystems | Category::SessionBus | Category::SystemBus => {
                        let value = match (category, entry.allowed) {
                            (Category::Filesystems, false) => entry
                                .restore
                                .clone()
                                .unwrap_or_else(|| format!("{}:ro", entry.value)),
                            (Category::Filesystems, true) => entry
                                .value
                                .split(':')
                                .next()
                                .unwrap_or(&entry.value)
                                .to_owned(),
                            _ => entry.restore.clone().unwrap_or_else(|| {
                                format!(
                                    "{}={}",
                                    entry.value,
                                    if entry.detail.as_deref() == Some("own") {
                                        "own"
                                    } else {
                                        "talk"
                                    }
                                )
                            }),
                        };
                        let edit = Edit {
                            category,
                            value,
                            allow: !entry.allowed,
                        };
                        row.control(
                            enz_button(
                                if entry.allowed {
                                    tr("shell-remove")
                                } else {
                                    tr("shell-allow")
                                },
                                ParentMessage::Sandbox(Message::Edit(
                                    generation,
                                    snapshot.review_token.clone(),
                                    edit,
                                )),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .enabled(live)
                            .into_element(),
                        )
                    }
                    _ => {
                        let value = entry.value.clone();
                        let token = snapshot.review_token.clone();
                        row.switch(entry.allowed, move |allow| {
                            ParentMessage::Sandbox(Message::Edit(
                                generation,
                                token.clone(),
                                Edit {
                                    category,
                                    value: value.clone(),
                                    allow,
                                },
                            ))
                        })
                    }
                };
                body = body.push(row.enabled(live || !entry.editable).into_element());
            }
        }
        if !snapshot.unknown.is_empty() {
            body = body.push(group(tr("shell-other-settings")));
            for (group, key, value) in &snapshot.unknown {
                body = body.push(
                    crate::settings_app::settings_row(format!("{key} = {value}"))
                        .subtitle(tr!("shell-shown-as-flatpak-reports-it", "arg0" => group.clone()))
                        .subtitle_wrap(true)
                        .into_element(),
                );
            }
        }
        for limitation in &snapshot.limitations {
            body = body.push(note(limitation.clone()));
        }
        body = body.push(
            enz_button(
                if self.sandbox.advanced {
                    tr("shell-hide-all-policy-layers")
                } else {
                    tr("shell-show-all-policy-layers")
                },
                ParentMessage::Sandbox(Message::Advanced(!self.sandbox.advanced)),
            )
            .kind(ButtonKind::Quiet)
            .size(ButtonSize::Sm)
            .icon(if self.sandbox.advanced {
                "chevron-up"
            } else {
                "chevron-down"
            })
            .into_element(),
        );
        if self.sandbox.advanced {
            for (label, rows) in &snapshot.layers {
                body = body.push(
                    enz_text(*label)
                        .size_px(self.text_px(13.0))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                );
                match rows {
                    None => {
                        body =
                            body.push(note(tr("shell-does-not-apply-to-this-installation").into()));
                    }
                    Some(rows) if rows.is_empty() => {
                        body = body.push(note(tr("shell-nothing-set").into()));
                    }
                    Some(rows) => {
                        for (group, key, value) in rows {
                            body = body.push(note(format!("[{group}] {key} = {value}")));
                        }
                    }
                }
            }
        }
        let changed = snapshot.layers.iter().any(|(label, rows)| {
            *label == Source::UserApp.label() && rows.as_ref().is_some_and(|rows| !rows.is_empty())
        });
        body = body.push(
            crate::settings_app::settings_row(tr("shell-your-changes-to-this-app"))
                .control(
                    enz_button(
                        tr("shell-reset"),
                        ParentMessage::Sandbox(Message::Reset(
                            generation,
                            snapshot.review_token.clone(),
                        )),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .icon("rotate-ccw")
                    .enabled(live && changed)
                    .into_element(),
                )
                .into_element(),
        );
        Some(body.into())
    }
}
