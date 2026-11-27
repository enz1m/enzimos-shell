use super::{Message as ParentMessage, SettingsApp, local_ai_settings, settings_row};
use enzui_shell::{agent_harness, i18n::tr};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    combo_box::enz_combo_box,
    text_field::enz_text_field_owned,
};
use iced::{
    Element, Length, Task,
    widget::{column, row, space},
};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct State {
    open: bool,
    busy: bool,
    transport: usize,
    name: String,
    endpoint: String,
    credentials: String,
    folder: String,
    rows: Value,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Toggle,
    Refresh,
    Transport(usize),
    Name(String),
    Endpoint(String),
    Credentials(String),
    Folder(String),
    AddServer,
    AddFolder,
    Change(Value),
    Loaded(Result<Value, String>),
}

fn send(message: Message) -> ParentMessage {
    ParentMessage::LocalAi(local_ai_settings::Message::Extensions(message))
}

fn request(method: &'static str, fields: Value) -> Task<ParentMessage> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || agent_harness::request(method, fields))
                .await
                .unwrap_or_else(|_| Err(tr("shell-the-service-did-not-answer").into()))
        },
        |result| send(Message::Loaded(result)),
    )
}

impl State {
    pub(super) fn update(&mut self, message: Message, live: bool) -> Task<ParentMessage> {
        match message {
            Message::Toggle => {
                self.open = !self.open;
                if self.open && !self.busy {
                    return self.update(Message::Refresh, live);
                }
            }
            Message::Refresh if !self.busy => {
                self.busy = true;
                return request("extensions.list", json!({}));
            }
            Message::Transport(index) => {
                self.transport = index;
                self.endpoint.clear();
                self.credentials.clear();
            }
            Message::Name(value) => self.name = value,
            Message::Endpoint(value) => self.endpoint = value,
            Message::Credentials(value) => self.credentials = value,
            Message::Folder(value) => self.folder = value,
            Message::AddServer if live && !self.busy => {
                let credentials = if self.credentials.trim().is_empty() {
                    Value::Null
                } else if let Ok(Value::Object(values)) = serde_json::from_str(&self.credentials) {
                    Value::Object(values)
                } else {
                    self.error = Some(tr("shell-assistant-extension-invalid-credentials").into());
                    return Task::none();
                };
                let config = if self.transport == 0 {
                    json!({"transport":"http","enabled":true,"url":self.endpoint.trim(),"headers":credentials})
                } else if let Ok(Value::Array(argv)) = serde_json::from_str(&self.endpoint) {
                    json!({"transport":"stdio","enabled":true,"argv":argv,"environment":credentials})
                } else {
                    self.error = Some(tr("shell-assistant-extension-invalid-command").into());
                    return Task::none();
                };
                self.credentials.clear();
                return self.update(Message::Change(json!({"action":"add","kind":"server","name":self.name.trim(),"config":config})), live);
            }
            Message::AddFolder if live && !self.busy => {
                return self.update(
                    Message::Change(
                        json!({"action":"add","kind":"skill","path":self.folder.trim()}),
                    ),
                    live,
                );
            }
            Message::Change(fields) if live && !self.busy => {
                self.busy = true;
                self.error = None;
                return request("extensions.change", fields);
            }
            Message::Loaded(result) => {
                self.busy = false;
                match result {
                    Ok(rows) => {
                        self.rows = rows;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn view<'a>(&self, app: &SettingsApp) -> Element<'a, ParentMessage> {
        let toggle = enz_button(
            tr(if self.open {
                "shell-hide"
            } else {
                "shell-show"
            }),
            send(Message::Toggle),
        )
        .kind(ButtonKind::Secondary)
        .id("assistant-extensions-toggle".into())
        .into_element();
        let mut rows = column![
            settings_row(tr("shell-assistant-extensions"))
                .subtitle(tr("shell-assistant-extensions-description"))
                .subtitle_wrap(true)
                .control(toggle)
                .into_element()
        ]
        .spacing(6);
        if !self.open {
            return rows.into();
        }
        let editable = app.controls_live() && !self.busy;
        for (kind, group, field) in [("server", "servers", "name"), ("skill", "folders", "path")] {
            for item in self.rows[group].as_array().into_iter().flatten() {
                let Some(label) = item[field].as_str() else {
                    continue;
                };
                let enabled = item["enabled"] == true;
                let change = |action: &str| {
                    send(Message::Change(json!({
                        "action":action, "kind":kind, (field):label, "enabled": !enabled
                    })))
                };
                rows = rows.push(
                    settings_row(label.to_owned())
                        .subtitle(tr(if enabled { "shell-on" } else { "shell-off" }))
                        .control(
                            row![
                                enz_button(
                                    tr(if enabled {
                                        "shell-assistant-extension-disable"
                                    } else {
                                        "shell-assistant-extension-enable"
                                    }),
                                    change("enable")
                                )
                                .kind(ButtonKind::Secondary)
                                .enabled(editable)
                                .into_element(),
                                enz_button(tr("shell-remove"), change("remove"))
                                    .kind(ButtonKind::Secondary)
                                    .enabled(editable)
                                    .into_element(),
                            ]
                            .spacing(6),
                        )
                        .into_element(),
                );
            }
        }
        let transport = enz_combo_box(
            "",
            vec![
                tr("shell-assistant-extension-http").to_owned(),
                tr("shell-assistant-extension-stdio").to_owned(),
            ],
            None,
            |_| send(Message::Transport(0)),
        )
        .id("assistant-extension-transport".into())
        .aria_label(tr("shell-assistant-extension-transport"))
        .on_select_index(|index| send(Message::Transport(index)))
        .selected_index(Some(self.transport))
        .enabled(editable)
        .width(180)
        .into_element();
        let http = self.transport == 0;
        let field = |label, sample, value: &String, change: fn(String) -> Message, id, secret| {
            enz_text_field_owned(tr(label), sample, value.clone(), move |text| {
                send(change(text))
            })
            .id(iced::widget::Id::new(id))
            .secure(secret)
            .into_element()
        };
        rows = rows
            .push(
                settings_row(tr("shell-assistant-extension-add-server"))
                    .subtitle(tr(if http {
                        "shell-assistant-extension-http-detail"
                    } else {
                        "shell-assistant-extension-stdio-detail"
                    }))
                    .subtitle_wrap(true)
                    .control(transport)
                    .into_element(),
            )
            .push(
                column![
                    field(
                        "shell-name",
                        "my_server",
                        &self.name,
                        Message::Name,
                        "assistant-extension-name",
                        false
                    ),
                    field(
                        if http {
                            "shell-address"
                        } else {
                            "shell-assistant-extension-command"
                        },
                        if http {
                            "https://example.com/mcp"
                        } else {
                            r#"["/usr/bin/my-server","--stdio"]"#
                        },
                        &self.endpoint,
                        Message::Endpoint,
                        "assistant-extension-address",
                        false
                    ),
                    field(
                        if http {
                            "shell-assistant-extension-headers"
                        } else {
                            "shell-assistant-extension-environment"
                        },
                        if http {
                            r#"{"Authorization":"Bearer …"}"#
                        } else {
                            r#"{"API_TOKEN":"…"}"#
                        },
                        &self.credentials,
                        Message::Credentials,
                        "assistant-extension-credentials",
                        true
                    ),
                    row![
                        space().width(Length::Fill),
                        enz_button(tr("shell-add"), send(Message::AddServer))
                            .id("assistant-add-server".into())
                            .kind(ButtonKind::Secondary)
                            .enabled(editable && !self.name.is_empty() && !self.endpoint.is_empty())
                            .into_element()
                    ],
                    field(
                        "shell-assistant-extension-skill-folder",
                        "/home/you/skills",
                        &self.folder,
                        Message::Folder,
                        "assistant-skill-folder",
                        false
                    ),
                    row![
                        space().width(Length::Fill),
                        enz_button(
                            tr("shell-assistant-extension-add-folder"),
                            send(Message::AddFolder)
                        )
                        .id("assistant-add-skill-folder".into())
                        .kind(ButtonKind::Secondary)
                        .enabled(editable && !self.folder.is_empty())
                        .into_element(),
                        enz_button(
                            tr("shell-assistant-extension-refresh"),
                            send(Message::Refresh)
                        )
                        .id("assistant-extensions-refresh".into())
                        .kind(ButtonKind::Quiet)
                        .enabled(!self.busy)
                        .into_element()
                    ]
                    .spacing(6)
                ]
                .spacing(8)
                .padding([4, 16]),
            );
        if let Some(error) = &self.error {
            rows = rows.push(
                enzui_widgets::text::enz_text(error.clone())
                    .wrap(true)
                    .into_element(),
            );
        }
        rows.into()
    }
}
