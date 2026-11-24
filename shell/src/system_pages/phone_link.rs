use super::*;
use enzui_shell::phone_link::{self, Request, Snapshot};
use enzui_shell::system_settings::phone_network::Access;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    qr::{EnzQrData, enz_qr},
    text_field::enz_text_field,
};
use iced::widget::{Space, column, row};
use serde_json::{Map, Value, json};

#[derive(Default)]
pub(super) struct Phone {
    snapshot: Option<Snapshot>,
    network: Option<Result<Access, String>>,
    network_reading: bool,
    network_epoch: u64,
    network_last: Option<Instant>,
    reading: bool,
    busy: bool,
    epoch: u64,
    last: Option<Instant>,
    notice: Option<String>,
    qr: Option<(EnzQrData, Instant)>,
    drafts: HashMap<String, String>,
    dirty_drafts: std::collections::HashSet<String>,
    applied_draft: Option<String>,
    confirm: Option<Confirm>,
    pub(super) send_device: Option<String>,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum Confirm {
    Stop,
    Restart,
    Forget(String),
    Close(String),
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum PhoneMessage {
    Read(u64, Result<Snapshot, String>),
    NetworkRead(u64, Access),
    NetworkRefresh,
    NetworkAllow,
    NetworkDone(u64, Result<Access, String>),
    Command(Request),
    Completed(u64, Result<enzimos_phone_client::Reply, String>),
    Service(bool),
    ServiceDone(u64, Result<(), String>),
    Ask(Confirm),
    Confirm,
    Cancel,
    HideQr,
    Draft(String, String),
    ApplyField(String),
    ChooseFiles(String),
    Files(Vec<std::path::PathBuf>),
}

fn message(message: PhoneMessage) -> Parent {
    Parent::System(Message::Phone(message))
}

impl SettingsApp {
    pub(in crate::settings_app) fn phone_leave(&mut self) {
        self.system.phone.qr = None;
        self.system.phone.confirm = None;
        self.system.phone.send_device = None;
    }

    pub(super) fn phone_read(&mut self, force: bool) -> Task<Parent> {
        if self.system.backend.demo {
            self.system.phone.snapshot = Some(Snapshot::default());
            return Task::none();
        }
        let state = &mut self.system.phone;
        if state
            .qr
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            state.qr = None;
        }
        if state.reading
            || state.busy
            || (!force
                && state
                    .last
                    .is_some_and(|last| last.elapsed() < Duration::from_secs(3)))
        {
            return Task::none();
        }
        state.reading = true;
        state.epoch = state.epoch.saturating_add(1);
        let epoch = state.epoch;
        let host = Task::perform(phone_link::snapshot(), move |result| {
            message(PhoneMessage::Read(epoch, result))
        });
        Task::batch([host, self.phone_network_read(false)])
    }

    fn phone_network_read(&mut self, force: bool) -> Task<Parent> {
        let state = &mut self.system.phone;
        if state.network_reading
            || state.busy
            || (!force
                && state
                    .network_last
                    .is_some_and(|last| last.elapsed() < Duration::from_secs(60)))
        {
            return Task::none();
        }
        state.network_reading = true;
        state.network_epoch = state.network_epoch.saturating_add(1);
        let epoch = state.network_epoch;
        let backend = self.system.backend.clone();
        Task::perform(
            async move { backend.phone_network().await },
            move |result| message(PhoneMessage::NetworkRead(epoch, result)),
        )
    }

    pub(in crate::settings_app) fn phone_message(&mut self, event: PhoneMessage) -> Task<Parent> {
        let blocked = self.system.backend.demo || !self.system.backend.writes_allowed;
        if blocked
            && matches!(
                &event,
                PhoneMessage::Command(_)
                    | PhoneMessage::NetworkAllow
                    | PhoneMessage::Service(_)
                    | PhoneMessage::Confirm
                    | PhoneMessage::Ask(_)
                    | PhoneMessage::ApplyField(_)
                    | PhoneMessage::ChooseFiles(_)
                    | PhoneMessage::Files(_)
            )
        {
            self.system.phone.notice = Some(tr("shell-phone-preview-read-only").into());
            return Task::none();
        }
        if matches!(event, PhoneMessage::Command(_)) {
            self.menu = None;
        }
        let state = &mut self.system.phone;
        match event {
            PhoneMessage::NetworkRefresh => return self.phone_network_read(true),
            PhoneMessage::NetworkRead(epoch, result) if epoch == state.network_epoch => {
                state.network_reading = false;
                state.network_last = Some(Instant::now());
                state.network = Some(Ok(result));
            }
            PhoneMessage::NetworkAllow => {
                if state.busy {
                    return Task::none();
                }
                state.busy = true;
                state.network_reading = false;
                state.network_epoch = state.network_epoch.saturating_add(1);
                let epoch = state.network_epoch;
                let backend = self.system.backend.clone();
                return Task::perform(
                    async move { backend.allow_phone_network().await },
                    move |result| message(PhoneMessage::NetworkDone(epoch, result)),
                );
            }
            PhoneMessage::NetworkDone(epoch, result) if epoch == state.network_epoch => {
                state.busy = false;
                state.network_last = Some(Instant::now());
                state.network = Some(result);
            }
            PhoneMessage::Read(epoch, result) if epoch == state.epoch => {
                state.reading = false;
                state.last = Some(Instant::now());
                match result {
                    Ok(snapshot) => {
                        for (key, value) in &snapshot.settings {
                            if !value.is_boolean() && !state.dirty_drafts.contains(key) {
                                state.drafts.insert(
                                    key.clone(),
                                    value
                                        .as_str()
                                        .map_or_else(|| value.to_string(), str::to_owned),
                                );
                            }
                        }
                        state.snapshot = Some(snapshot);
                    }
                    Err(error) => state.notice = Some(error),
                }
            }
            PhoneMessage::Command(request) => {
                if state.busy {
                    return Task::none();
                }
                state.busy = true;
                state.epoch = state.epoch.saturating_add(1);
                state.reading = false;
                state.notice = None;
                let epoch = state.epoch;
                return Task::perform(phone_link::command(request), move |result| {
                    message(PhoneMessage::Completed(epoch, result))
                });
            }
            PhoneMessage::Completed(epoch, result) if epoch == state.epoch => {
                state.busy = false;
                match result {
                    Ok(reply) => {
                        if let Some(key) = state.applied_draft.take() {
                            state.dirty_drafts.remove(&key);
                        }
                        if let Some(payload) = reply
                            .0
                            .get("payload")
                            .filter(|_| self.page == Category::Phone)
                        {
                            match EnzQrData::try_new(payload.to_string()) {
                                Ok(data) => {
                                    state.qr = Some((
                                        data,
                                        Instant::now()
                                            + Duration::from_secs(
                                                reply.0["expires_in"]
                                                    .as_u64()
                                                    .unwrap_or(300)
                                                    .min(300),
                                            ),
                                    ));
                                }
                                Err(_) => state.notice = Some(tr("shell-phone-qr-failed").into()),
                            }
                        }
                        if reply.0["restart_required"] == true {
                            state.notice = Some(tr("shell-phone-restart-needed").into());
                        }
                    }
                    Err(error) => {
                        state.applied_draft = None;
                        state.notice = Some(error);
                    }
                }
                return self.phone_read(true);
            }
            PhoneMessage::Service(enabled) => {
                if state.busy {
                    return Task::none();
                }
                state.busy = true;
                state.reading = false;
                state.epoch = state.epoch.saturating_add(1);
                let epoch = state.epoch;
                return Task::perform(phone_link::set_running(enabled), move |result| {
                    message(PhoneMessage::ServiceDone(epoch, result))
                });
            }
            PhoneMessage::ServiceDone(epoch, result) if epoch == state.epoch => {
                state.busy = false;
                state.notice = result.err();
                return self.phone_read(true);
            }
            PhoneMessage::Ask(confirm) => state.confirm = Some(confirm),
            PhoneMessage::Cancel => state.confirm = None,
            PhoneMessage::Confirm => {
                let Some(action) = state.confirm.take() else {
                    return Task::none();
                };
                return match action {
                    Confirm::Stop => self.phone_message(PhoneMessage::Service(false)),
                    Confirm::Forget(device_id) => {
                        self.phone_message(PhoneMessage::Command(Request::Unpair { device_id }))
                    }
                    Confirm::Close(session_id) => self
                        .phone_message(PhoneMessage::Command(Request::SessionStop { session_id })),
                    Confirm::Restart => {
                        state.busy = true;
                        state.reading = false;
                        state.epoch = state.epoch.saturating_add(1);
                        let epoch = state.epoch;
                        Task::perform(
                            async {
                                phone_link::set_running(false).await?;
                                phone_link::set_running(true).await
                            },
                            move |result| message(PhoneMessage::ServiceDone(epoch, result)),
                        )
                    }
                };
            }
            PhoneMessage::HideQr => state.qr = None,
            PhoneMessage::Draft(key, value) => {
                state.dirty_drafts.insert(key.clone());
                state.drafts.insert(key, value);
            }
            PhoneMessage::ApplyField(key) => {
                let text = state.drafts.get(&key).cloned().unwrap_or_default();
                let value = match state
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.settings.get(&key))
                {
                    Some(Value::Number(_)) => match text.parse::<u64>() {
                        Ok(value) => json!(value),
                        Err(_) => {
                            state.notice = Some(tr("shell-phone-number-needed").into());
                            return Task::none();
                        }
                    },
                    _ => json!(text),
                };
                state.applied_draft = Some(key.clone());
                return self.phone_message(PhoneMessage::Command(Request::SettingsSet {
                    values: Map::from_iter([(key, value)]),
                }));
            }
            PhoneMessage::ChooseFiles(device_id) => {
                state.send_device = Some(device_id);
                return Task::done(Parent::FileChooser(
                    crate::settings_app::settings_file_chooser::Message::OpenPhoneFiles,
                ));
            }
            PhoneMessage::Files(paths) => {
                if let Some(device_id) = state.send_device.take().filter(|_| !paths.is_empty()) {
                    let Ok(paths) = paths
                        .into_iter()
                        .map(|path| path.into_os_string().into_string())
                        .collect::<Result<Vec<_>, _>>()
                    else {
                        state.notice = Some(tr("shell-phone-file-name-error").into());
                        return Task::none();
                    };
                    return self.phone_message(PhoneMessage::Command(Request::SendFiles {
                        device_id: Some(device_id),
                        paths,
                    }));
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn phone_page<'a>(
        &'a self,
        mut body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        if let Some(notice) = self.demo_notice().or_else(|| self.read_only_notice()) {
            body = body.push(notice);
        }
        let state = &self.system.phone;
        let busy = state.busy || self.system.backend.demo || !self.system.backend.writes_allowed;
        let button = |label: &'static str, event: PhoneMessage| {
            enz_button(tr(label), message(event))
                .id(format!("phone-{label}").into())
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .enabled(!busy)
                .into_element()
        };
        let notice = state
            .notice
            .as_ref()
            .map(|notice| self.info_row("info", tr("shell-phone-link"), notice.clone()));
        // Without a running service there is nothing to pair or share yet:
        // one row says what is missing, instead of one per section.
        let missing = match &state.snapshot {
            None if state.reading => Some(self.loading_row()),
            None => Some(self.info_row(
                "smartphone",
                tr("shell-phone-unavailable"),
                tr("shell-phone-retry"),
            )),
            Some(snapshot) if !snapshot.installed && !snapshot.running => Some(self.info_row(
                "smartphone",
                tr("shell-phone-not-installed"),
                tr("shell-phone-not-installed-detail"),
            )),
            Some(_) => None,
        };
        if let Some(missing) = missing {
            let mut rows = column![self.mark_setting("phone/overview", missing)].spacing(2);
            if let Some(notice) = notice {
                rows = rows.push(notice);
            }
            return body.push(
                self.section_card_for(
                    "phone/overview",
                    column![
                        rows,
                        self.mark_control_anchor("phone/pair", Space::new()),
                        self.mark_control_anchor("phone/sharing", Space::new()),
                    ],
                )
                .into_element(),
            );
        }
        let snapshot = state
            .snapshot
            .as_ref()
            .expect("a missing service returned above");
        let mut overview = column![].spacing(2);
        if let Some(notice) = notice {
            overview = overview.push(notice);
        }
        overview = overview.push(
            self.mark_setting(
                "phone/overview",
                crate::settings_app::settings_row(tr("shell-phone-enable"))
                    .icon("smartphone")
                    .subtitle(tr("shell-phone-enable-description"))
                    .id_prefix("control-phone/overview")
                    .switch(snapshot.running, |enabled| {
                        message(if enabled {
                            PhoneMessage::Service(true)
                        } else {
                            PhoneMessage::Ask(Confirm::Stop)
                        })
                    })
                    .enabled(!busy && phone_link::can_control_service())
                    .into_element(),
            ),
        );
        {
            let description = match &state.network {
                None => tr("shell-phone-network-reading").to_owned(),
                Some(Err(error)) => error.clone(),
                Some(Ok(Access::Unfiltered)) => tr("shell-phone-network-unfiltered").to_owned(),
                Some(Ok(Access::Other(name))) => {
                    enzui_shell::tr!("shell-phone-network-other", "arg0" => *name)
                }
                Some(Ok(Access::Allowed)) => tr("shell-phone-network-allowed").to_owned(),
                Some(Ok(Access::Unverified)) => tr("shell-phone-network-unverified").to_owned(),
            };
            let mut access = crate::settings_app::settings_row(tr("shell-phone-network-access"))
                .icon("shield")
                .subtitle(description);
            if matches!(&state.network, Some(Ok(Access::Unverified))) {
                access = access.control(button(
                    "shell-phone-network-allow",
                    PhoneMessage::NetworkAllow,
                ));
            } else if matches!(&state.network, Some(Err(_))) {
                access = access.control(
                    enz_button(tr("shell-retry"), message(PhoneMessage::NetworkRefresh))
                        .id("phone-network-retry".into())
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(!state.network_reading && !state.busy)
                        .into_element(),
                );
            }
            overview = overview.push(access.into_element());
        }
        if phone_link::can_control_service() && snapshot.running {
            overview = overview.push(
                crate::settings_app::settings_row(tr("shell-phone-restart"))
                    .icon("rotate-ccw")
                    .subtitle(tr("shell-phone-restart-detail"))
                    .control(button("shell-restart", PhoneMessage::Ask(Confirm::Restart)))
                    .into_element(),
            );
        }
        if let Some(confirm) = &state.confirm {
            overview = overview.push(self.phone_confirm(confirm, busy));
        }
        body = body.push(
            self.section_card_for("phone/overview", overview)
                .into_element(),
        );

        let mut phones = column![].spacing(2);
        let mut pair = crate::settings_app::settings_row(tr("shell-pair-a-phone"))
            .icon("qr-code")
            .subtitle(if snapshot.running {
                tr("shell-phone-pair-detail")
            } else {
                tr("shell-phone-turn-on-to-pair")
            });
        if snapshot.running {
            pair = pair.control(button(
                "shell-phone-show-code",
                PhoneMessage::Command(Request::PairCode),
            ));
        }
        phones = phones
            .push(self.mark_setting("phone/pair", pair.enabled(snapshot.running).into_element()));
        if let Some((data, _)) = state.qr.as_ref().filter(|_| snapshot.running) {
            phones = phones.push(
                iced::widget::container(
                    column![
                        enz_qr(data).total_size(240.0).into_element(),
                        self.caption(tr("shell-phone-scan-qr")),
                        button("shell-close", PhoneMessage::HideQr),
                    ]
                    .spacing(10)
                    .align_x(iced::Alignment::Center),
                )
                .center_x(iced::Length::Fill)
                .padding([8, 12]),
            );
        }
        for pending in snapshot.pending.iter().filter(|_| snapshot.running) {
            phones = phones.push(
                crate::settings_app::settings_row(pending.name.clone())
                    .accessory_width(WIDE_CONTROL)
                    .icon("smartphone")
                    .subtitle(enzui_shell::tr!("shell-phone-wants-to-pair", "arg0" => pending.code.clone()))
                    .control(
                        row![
                            button(
                                "shell-phone-decline",
                                PhoneMessage::Command(Request::Decline {
                                    target: pending.code.clone()
                                })
                            ),
                            enz_button(
                                tr("shell-phone-approve"),
                                message(PhoneMessage::Command(Request::Approve {
                                    target: pending.code.clone()
                                }))
                            )
                            .id(format!("phone-approve-{}", pending.code).into())
                            .kind(ButtonKind::Accent)
                            .size(ButtonSize::Sm)
                            .enabled(!busy)
                            .into_element(),
                        ]
                        .spacing(8),
                    )
                    .into_element(),
            );
        }
        body = body.push(
            self.section_card_for("phone/pair", phones)
                .title(tr("shell-phone-devices"))
                .into_element(),
        );
        // Each paired phone in its own card: its status, actions and what
        // it may do.
        for device in &snapshot.devices {
            body = body.push(
                self.section_card(self.phone_device_rows(device, busy))
                    .into_element(),
            );
        }

        let mut sharing = column![].spacing(2);
        if snapshot.schema.is_empty() {
            sharing = sharing.push(self.info_row(
                "smartphone",
                tr("shell-phone-sharing"),
                tr("shell-phone-turn-on-to-pair"),
            ));
        }
        for field in &snapshot.schema {
            let (Some(key), Some(title)) = (field["key"].as_str(), field["title"].as_str()) else {
                continue;
            };
            let enabled = !busy && field["locked"] != true;
            let description = field["description"].as_str().unwrap_or("").to_owned();
            let row = crate::settings_app::settings_row(title.to_owned())
                .subtitle(description)
                .id_prefix(format!("phone-setting-{key}"));
            if field["kind"] == "bool" {
                let key = key.to_owned();
                let on = snapshot
                    .settings
                    .get(&key)
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                sharing = sharing.push(
                    row.switch(on, move |value| {
                        message(PhoneMessage::Command(Request::SettingsSet {
                            values: Map::from_iter([(key.clone(), json!(value))]),
                        }))
                    })
                    .enabled(enabled)
                    .into_element(),
                );
            } else if field["kind"] == "choice" {
                let selected = snapshot.settings.get(key).map(|value| {
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                });
                let mut current = String::new();
                let choices: Vec<(String, Parent)> = field["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|choice| {
                        let value = choice["value"].as_str()?;
                        let label = choice["label"].as_str()?.to_owned();
                        if selected.as_deref() == Some(value) {
                            current.clone_from(&label);
                        }
                        Some((
                            label,
                            message(PhoneMessage::Command(Request::SettingsSet {
                                values: Map::from_iter([(key.to_owned(), json!(value))]),
                            })),
                        ))
                    })
                    .collect();
                let menu = format!("phone-setting-{key}");
                sharing = sharing.push(
                    row.value_menu(
                        current,
                        self.menu.as_deref() == Some(menu.as_str()),
                        Parent::Menu(menu.clone(), true),
                        Parent::Menu(menu, false),
                        choices,
                    )
                    .enabled(enabled)
                    .into_element(),
                );
            } else {
                let key = key.to_owned();
                let draft_key = key.clone();
                let (field_id, apply_id) =
                    (format!("phone-field-{key}"), format!("phone-apply-{key}"));
                let changed = state.dirty_drafts.contains(&key);
                let value = state.drafts.get(&key).map_or("", String::as_str);
                sharing = sharing.push(
                    row.accessory_width(WIDE_CONTROL)
                        .control(
                            row![
                                iced::widget::container(
                                    enz_text_field("", "", value, move |value| message(
                                        PhoneMessage::Draft(draft_key.clone(), value)
                                    ))
                                    .id(field_id.into())
                                    .enabled(enabled)
                                    .into_element()
                                )
                                .width(iced::Length::Fixed(WIDE_CONTROL - 80.0)),
                                enz_button(
                                    tr("shell-apply"),
                                    message(PhoneMessage::ApplyField(key))
                                )
                                .id(apply_id.into())
                                .kind(ButtonKind::Secondary)
                                .enabled(enabled && changed)
                                .size(ButtonSize::Sm)
                                .into_element()
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                        )
                        .enabled(enabled)
                        .into_element(),
                );
            }
        }
        body = body.push(
            self.section_card_for("phone/sharing", self.mark_setting("phone/sharing", sharing))
                .title(tr("shell-phone-sharing"))
                .into_element(),
        );
        if !snapshot.sessions.is_empty() {
            let mut sessions = column![].spacing(2);
            for session in &snapshot.sessions {
                sessions = sessions.push(
                    crate::settings_app::settings_row(session.app_name.clone())
                        .icon("cast")
                        .subtitle(session.device_name.clone())
                        .control(
                            enz_button(
                                tr("shell-phone-end-session"),
                                message(PhoneMessage::Ask(Confirm::Close(
                                    session.session_id.clone(),
                                ))),
                            )
                            .id(format!("phone-end-session-{}", session.session_id).into())
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .enabled(!busy)
                            .into_element(),
                        )
                        .into_element(),
                );
            }
            body = body.push(
                self.section_card(sessions)
                    .title(tr("shell-phone-sessions"))
                    .into_element(),
            );
        }
        body
    }

    fn phone_confirm<'a>(&self, confirm: &Confirm, busy: bool) -> Element<'a, Parent> {
        let detail = match confirm {
            Confirm::Forget(_) => tr("shell-phone-forget-warning"),
            Confirm::Close(_) => tr("shell-phone-close-warning"),
            _ => tr("shell-phone-stop-warning"),
        };
        column![
            crate::settings_app::settings_row(detail)
                .icon("triangle-alert")
                .subtitle_wrap(true)
                .into_element(),
            row![
                Space::new().width(iced::Length::Fill),
                enz_button(tr("shell-cancel"), message(PhoneMessage::Cancel))
                    .id("phone-confirm-cancel".into())
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .into_element(),
                enz_button(tr("shell-continue"), message(PhoneMessage::Confirm))
                    .id("phone-confirm-continue".into())
                    .kind(ButtonKind::Destructive)
                    .size(ButtonSize::Sm)
                    .enabled(!busy)
                    .into_element(),
            ]
            .spacing(8)
            .padding([4, 12]),
        ]
        .spacing(2)
        .into()
    }

    fn phone_device_rows<'a>(
        &'a self,
        device: &'a enzui_shell::phone_link::Device,
        busy: bool,
    ) -> Element<'a, Parent> {
        let small = |label: &'static str, event: PhoneMessage| {
            enz_button(tr(label), message(event))
                .id(format!("phone-{}-{label}", device.device_id).into())
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .enabled(!busy)
                .into_element()
        };
        let mut status = if device.connected {
            tr("shell-connected").to_owned()
        } else {
            tr("shell-phone-offline").to_owned()
        };
        if let Some(battery) = &device.battery {
            status = format!("{status} · {}%", battery.level.min(100));
            if battery.charging {
                status = format!("{status} {}", tr("shell-phone-charging"));
            }
        }
        let forget = enzui_widgets::toolbar_icon::enz_toolbar_icon(
            "x",
            message(PhoneMessage::Ask(Confirm::Forget(device.device_id.clone()))),
        )
        .id(format!("phone-forget-{}", device.device_id).into())
        .aria_label(enzui_shell::tr!("shell-forget-2", "arg0" => device.name.clone()))
        .tooltip(tr("shell-phone-forget"))
        .enabled(!busy)
        .into_element();
        let mut rows = column![
            crate::settings_app::settings_row(device.name.clone())
                .icon("smartphone")
                .subtitle(status)
                .control(forget)
                .into_element()
        ]
        .spacing(2);
        if device.connected {
            let ring = |stop: bool| {
                PhoneMessage::Command(Request::Ring {
                    device_id: Some(device.device_id.clone()),
                    stop,
                })
            };
            rows = rows
                .push(
                    crate::settings_app::settings_row(tr("shell-phone-ring"))
                        .accessory_width(WIDE_CONTROL)
                        .icon("bell-ring")
                        .subtitle(tr("shell-phone-ring-detail"))
                        .control(
                            row![
                                small("shell-phone-stop-ring", ring(true)),
                                small("shell-phone-ring-now", ring(false)),
                            ]
                            .spacing(8),
                        )
                        .into_element(),
                )
                .push(
                    crate::settings_app::settings_row(tr("shell-phone-send-files"))
                        .icon("send")
                        .control(small(
                            "shell-phone-choose-files",
                            PhoneMessage::ChooseFiles(device.device_id.clone()),
                        ))
                        .into_element(),
                );
        }
        for (key, label, icon) in [
            ("files", "shell-phone-permission-files", "folder"),
            ("clipboard", "shell-phone-permission-clipboard", "clipboard"),
            (
                "notifications",
                "shell-phone-permission-notifications",
                "bell",
            ),
            (
                "remote_control",
                "shell-phone-permission-control",
                "mouse-pointer-2",
            ),
            ("app_streaming", "shell-phone-permission-streaming", "cast"),
            ("phone_video", "shell-phone-permission-video", "camera"),
            ("sms", "shell-phone-permission-sms", "message-square"),
        ] {
            if !device.permissions.contains_key(key) {
                continue;
            }
            let id = device.device_id.clone();
            let key = key.to_owned();
            let on = device
                .permissions
                .get(&key)
                .and_then(Value::as_bool)
                .unwrap_or(false);
            rows = rows.push(
                crate::settings_app::settings_row(tr(label))
                    .icon(icon)
                    .id_prefix(format!("phone-{}-{key}", device.device_id))
                    .switch(on, move |enabled| {
                        message(PhoneMessage::Command(Request::DeviceUpdate {
                            device_id: id.clone(),
                            name: None,
                            permissions: Some(json!({key.clone(): enabled})),
                        }))
                    })
                    .enabled(!busy)
                    .into_element(),
            );
        }
        rows.into()
    }
}

// Room for two buttons, or a field and its button, beside a row's label.
const WIDE_CONTROL: f32 = 280.0;
