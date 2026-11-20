use super::{Message as ParentMessage, SettingsApp};
use enzui_shell::agent_harness::{self, Edit, Server, Summary};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::modal_host::ModalHostEvent;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    text::enz_text,
    text_field::enz_text_field,
};
use iced::{
    Element, Length, Task,
    widget::{column, row, text_editor},
};

#[derive(Default)]
pub(super) struct State {
    loading: bool,
    // `Ok(None)`: no configuration yet.
    config: Option<Result<Option<serde_json::Value>, String>>,
    running: bool,
    models: Option<Result<Vec<String>, String>>,
    root: String,
    skills: String,
    model: String,
    error: Option<String>,
    busy: bool,
    // What the running harness lists; `None` until read.
    found: Option<Result<Vec<agent_harness::Skill>, String>>,
    server: Option<ServerDraft>,
    skill: Option<SkillDraft>,
}

#[derive(Default)]
struct ServerDraft {
    name: String,
    web: bool,
    program: String,
    arguments: text_editor::Content,
    url: String,
    authorization: String,
    reveal: bool,
    error: Option<String>,
}

#[derive(Default)]
struct SkillDraft {
    folder: String,
    name: String,
    description: String,
    instructions: text_editor::Content,
    error: Option<String>,
    busy: bool,
}

// Secret drafts must remain redacted in Debug output.
#[derive(Clone)]
pub(super) struct Secret(String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret")
    }
}

#[derive(Clone, Debug)]
pub(super) enum Field {
    Name(String),
    Program(String),
    Arguments(text_editor::Action),
    Url(String),
    Authorization(Secret),
    SkillFolder(String),
    SkillName(String),
    Description(String),
    Instructions(text_editor::Action),
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Host(ModalHostEvent),
    Field(Field),
    OpenServer,
    ServerWeb(bool),
    Reveal,
    SaveServer,
    OpenSkill,
    CreateSkill,
    Created(Result<(), String>),
    Listed(Result<Vec<agent_harness::Skill>, String>),
    Close,
    Loaded(Result<Option<serde_json::Value>, String>, bool),
    Create,
    Edit(Edit),
    Root(String),
    Skills(String),
    Model(String),
    ListModels,
    Models(Result<Vec<String>, String>),
    Run(bool),
    Ran(Result<(), String>),
}

fn list() -> Task<ParentMessage> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                agent_harness::request("skills.list", serde_json::json!({}))
                    .map(|listed| agent_harness::skills(&listed))
            })
            .await
            .unwrap_or_else(|_| Err(tr("shell-the-skills-could-not-be-listed").into()))
        },
        |skills| ParentMessage::Assistant(Message::Listed(skills)),
    )
}

fn load() -> Task<ParentMessage> {
    Task::perform(
        async {
            (
                tokio::task::spawn_blocking(agent_harness::load)
                    .await
                    .unwrap_or_else(|_| Err(tr("shell-the-configuration-cannot-be-read").into())),
                agent_harness::running().await,
            )
        },
        |(config, running)| ParentMessage::Assistant(Message::Loaded(config, running)),
    )
}

impl SettingsApp {
    pub(super) fn assistant_read(&mut self) -> Task<ParentMessage> {
        if self.assistant.config.is_some() || self.assistant.loading || !agent_harness::installed()
        {
            return Task::none();
        }
        self.assistant.loading = true;
        load()
    }
    pub(super) fn assistant_message(&mut self, message: Message) -> Task<ParentMessage> {
        let state = &mut self.assistant;
        match message {
            Message::Loaded(config, running) => {
                state.loading = false;
                state.busy = false;
                state.running = running;
                let configured = matches!(config, Ok(Some(_)));
                state.config = Some(config);
                state.found = None;
                // Only a running harness can list or create skills.
                if running && configured {
                    list()
                } else {
                    Task::none()
                }
            }
            Message::Listed(skills) => {
                state.found = Some(skills);
                Task::none()
            }
            Message::Host(ModalHostEvent::Dismissed(_)) | Message::Close => {
                state.server = None;
                state.skill = state.skill.take().filter(|draft| draft.busy);
                Task::none()
            }
            Message::Host(event) => enzui_widgets::modal_host::focus_wrap_operation(event)
                .map_or_else(Task::none, iced::advanced::widget::operate),
            Message::OpenServer => {
                state.server = Some(ServerDraft::default());
                Task::none()
            }
            Message::ServerWeb(web) => {
                if let Some(draft) = &mut state.server {
                    draft.web = web;
                    draft.error = None;
                }
                Task::none()
            }
            Message::Reveal => {
                if let Some(draft) = &mut state.server {
                    draft.reveal = !draft.reveal;
                }
                Task::none()
            }
            Message::Field(field) => {
                match (field, &mut state.server, &mut state.skill) {
                    (Field::Name(text), Some(draft), _) => {
                        draft.name = text.chars().take(24).collect();
                    }
                    (Field::Program(text), Some(draft), _) => {
                        draft.program = text.chars().take(4096).collect();
                    }
                    (Field::Arguments(action), Some(draft), _) => {
                        draft.arguments.perform(action);
                    }
                    (Field::Url(text), Some(draft), _) => {
                        draft.url = text.chars().take(4096).collect();
                    }
                    (Field::Authorization(Secret(text)), Some(draft), _) => {
                        draft.authorization = text.chars().take(4096).collect();
                    }
                    (Field::SkillFolder(folder), _, Some(draft)) if !draft.busy => {
                        draft.folder = folder;
                    }
                    (Field::SkillName(text), _, Some(draft)) if !draft.busy => {
                        draft.name = text.chars().take(64).collect();
                    }
                    (Field::Description(text), _, Some(draft)) if !draft.busy => {
                        draft.description = text.chars().take(1024).collect();
                    }
                    (Field::Instructions(action), _, Some(draft))
                        if !draft.busy || !action.is_edit() =>
                    {
                        draft.instructions.perform(action);
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::SaveServer => {
                let (Some(draft), Some(Ok(Some(config)))) = (&mut state.server, &state.config)
                else {
                    return Task::none();
                };
                let server = if draft.web {
                    Server::Web {
                        url: draft.url.trim().to_owned(),
                        authorization: Some(draft.authorization.trim().to_owned())
                            .filter(|token| !token.is_empty()),
                    }
                } else {
                    let program = draft.program.trim();
                    let runnable = std::fs::metadata(program).is_ok_and(|meta| {
                        use std::os::unix::fs::PermissionsExt;
                        meta.is_file() && meta.permissions().mode() & 0o111 != 0
                    });
                    if !runnable {
                        draft.error =
                            Some(tr("shell-there-is-no-runnable-program-at-that-path").into());
                        return Task::none();
                    }
                    let arguments = draft.arguments.text();
                    let argv = std::iter::once(program.to_owned())
                        .chain(
                            arguments
                                .lines()
                                .map(str::trim)
                                .filter(|arg| !arg.is_empty())
                                .map(str::to_owned),
                        )
                        .collect();
                    Server::Program { argv }
                };
                let mut config = config.clone();
                match agent_harness::apply(
                    &mut config,
                    &Edit::AddServer(draft.name.trim().to_owned(), server),
                ) {
                    Ok(()) => {
                        state.server = None;
                        self.assistant_save(config)
                    }
                    Err(error) => {
                        draft.error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::OpenSkill => {
                let Some(Ok(Some(config))) = &state.config else {
                    return Task::none();
                };
                let folder = agent_harness::summary(config)
                    .writable_skills
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                state.skill = Some(SkillDraft {
                    folder,
                    ..SkillDraft::default()
                });
                Task::none()
            }
            Message::CreateSkill => {
                let Some(draft) = &mut state.skill else {
                    return Task::none();
                };
                if draft.busy {
                    return Task::none();
                }
                let (folder, name, description, instructions) = (
                    draft.folder.clone(),
                    draft.name.trim().to_owned(),
                    draft.description.trim().to_owned(),
                    draft.instructions.text(),
                );
                if let Err(error) = agent_harness::new_skill(&name, &description, &instructions) {
                    draft.error = Some(error);
                    return Task::none();
                }
                draft.busy = true;
                draft.error = None;
                let fields = serde_json::json!({"directory": folder, "name": name, "description": description, "instructions": instructions});
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            agent_harness::request("skills.create", fields).map(drop)
                        })
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-the-skill-could-not-be-created").into()))
                    },
                    |result| ParentMessage::Assistant(Message::Created(result)),
                )
            }
            Message::Created(result) => {
                match (result, &mut state.skill) {
                    (Ok(()), _) => {
                        state.skill = None;
                        return list();
                    }
                    (Err(error), Some(draft)) => {
                        draft.busy = false;
                        draft.error = Some(error);
                    }
                    (Err(error), None) => state.error = Some(error),
                }
                Task::none()
            }
            Message::Create => self.assistant_save(agent_harness::starter()),
            Message::Edit(edit) => {
                let Some(Ok(Some(mut config))) = state.config.clone() else {
                    return Task::none();
                };
                match agent_harness::apply(&mut config, &edit) {
                    Ok(()) => {
                        match edit {
                            Edit::AddRoot(_) => state.root.clear(),
                            Edit::AddSkills(_) => state.skills.clear(),
                            Edit::Model(_) => state.model.clear(),
                            _ => {}
                        }
                        self.assistant_save(config)
                    }
                    Err(error) => {
                        state.error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::Root(text) => {
                state.root = text;
                Task::none()
            }
            Message::Skills(text) => {
                state.skills = text;
                Task::none()
            }
            Message::Model(text) => {
                state.model = text;
                Task::none()
            }
            Message::ListModels => {
                let Some(Ok(Some(config))) = &state.config else {
                    return Task::none();
                };
                let gateway = agent_harness::summary(config).gateway;
                state.models = None;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || agent_harness::models(&gateway))
                            .await
                            .unwrap_or_else(|_| {
                                Err(tr("shell-the-ai-gateway-did-not-answer").into())
                            })
                    },
                    |models| ParentMessage::Assistant(Message::Models(models)),
                )
            }
            Message::Models(models) => {
                state.models = Some(models);
                Task::none()
            }
            Message::Run(on) => {
                state.busy = true;
                state.error = None;
                Task::perform(agent_harness::set_running(on), |result| {
                    ParentMessage::Assistant(Message::Ran(result))
                })
            }
            Message::Ran(result) => {
                state.error = result.err();
                load()
            }
        }
    }
    fn assistant_save(&mut self, config: serde_json::Value) -> Task<ParentMessage> {
        let state = &mut self.assistant;
        state.busy = true;
        state.error = None;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || agent_harness::save(&config))
                    .await
                    .unwrap_or_else(|_| Err(tr("shell-the-configuration-cannot-be-written").into()))
            },
            |result| {
                ParentMessage::Assistant(match result {
                    Ok(()) => Message::Ran(Ok(())),
                    Err(error) => Message::Ran(Err(error)),
                })
            },
        )
    }
    pub(super) fn assistant_section(&self) -> Element<'_, ParentMessage> {
        let state = &self.assistant;
        let live = self.controls_live() && !state.busy;
        let palette = self.tokens().palette;
        let note = |text: String| {
            enz_text(text)
                .size_px(self.text_px(12.0))
                .muted_on(palette.surface)
                .wrap(true)
                .into_element()
        };
        let edit = |edit: Edit| ParentMessage::Assistant(Message::Edit(edit));
        let mut rows = column![].spacing(2);
        if !agent_harness::installed() {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-the-assistant-is-not-installed"))
                    .subtitle(tr("shell-assistant-not-installed-detail"))
                    .icon("bot")
                    .into_element(),
            );
        } else {
            match &state.config {
                None => rows = rows.push(note(tr("shell-reading").into())),
                Some(Err(error)) => {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr(
                            "shell-the-assistant-cannot-be-set-up",
                        ))
                        .subtitle(error.clone())
                        .subtitle_wrap(true)
                        .icon("triangle-alert")
                        .into_element(),
                    );
                }
                Some(Ok(None)) => {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr("shell-set-up-the-assistant"))
                            .subtitle(tr("shell-uses-the-local-provider-only-with-no-folders-and-no-action-without-asking-no"))
                            .subtitle_wrap(true)
                            .icon("bot")
                            .control(enz_button(tr("shell-set-up"), ParentMessage::Assistant(Message::Create)).kind(ButtonKind::Secondary).enabled(live).into_element())
                            .into_element(),
                    );
                }
                Some(Ok(Some(config))) => {
                    rows = rows.push(self.assistant_rows(
                        &agent_harness::summary(config),
                        live,
                        &edit,
                    ));
                }
            }
        }
        if let Some(error) = &state.error {
            rows = rows.push(note(error.clone()));
        }
        self.section_card_for(
            "agents/assistant",
            self.mark_setting("agents/assistant", rows),
        )
        .title(tr("shell-assistant"))
        .description(tr("shell-assistant-description"))
        .into_element()
    }
    fn assistant_rows<'a>(
        &'a self,
        summary: &Summary,
        live: bool,
        edit: &dyn Fn(Edit) -> ParentMessage,
    ) -> Element<'a, ParentMessage> {
        let state = &self.assistant;
        let mut rows = column![].spacing(2);
        rows = rows.push(
            crate::settings_app::settings_row(if state.running {
                tr("shell-running")
            } else {
                tr("shell-stopped")
            })
            .subtitle(tr(
                "shell-for-this-login-it-does-not-start-at-sign-in-by-itself",
            ))
            .icon("bot")
            .control(
                enz_button(
                    if state.running {
                        tr("shell-stop")
                    } else {
                        tr("shell-start")
                    },
                    ParentMessage::Assistant(Message::Run(!state.running)),
                )
                .kind(ButtonKind::Secondary)
                .enabled(live && (state.running || !summary.roots.is_empty()))
                .into_element(),
            )
            .into_element(),
        );
        let choices: Vec<(String, ParentMessage)> = summary
            .providers
            .iter()
            .map(|p| (p.clone(), edit(Edit::Provider(p.clone()))))
            .collect();
        rows = rows.push(
            crate::settings_app::settings_row(tr("shell-provider"))
                .subtitle(tr("shell-only-providers-approved-in-this-configuration-there-is-no-automatic-fallback"))
                .subtitle_wrap(true)
                .value_menu(summary.provider.clone(), self.menu.as_deref() == Some("assistant-provider"), ParentMessage::Menu("assistant-provider".into(), true), ParentMessage::Menu("assistant-provider".into(), false), choices)
                .enabled(live)
                .into_element(),
        );
        let model_field = enz_text_field(
            "",
            enzui_shell::i18n::current().text("shell-model-name"),
            &state.model,
            |text| ParentMessage::Assistant(Message::Model(text)),
        )
        .on_submit(edit(Edit::Model(state.model.clone())))
        .id("assistant-model".into())
        .into_element();
        let model_field = iced::widget::container(model_field).width(Length::Fixed(180.0));
        rows = rows.push(
            crate::settings_app::settings_row(tr("shell-model"))
                .subtitle(if summary.model.is_empty() {
                    tr("shell-not-chosen").to_owned()
                } else {
                    summary.model.clone()
                })
                .control(
                    row![
                        model_field,
                        enz_button(
                            enzui_shell::i18n::tr("shell-list"),
                            ParentMessage::Assistant(Message::ListModels)
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(live)
                        .into_element()
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .width(Length::Shrink),
                )
                .into_element(),
        );
        match &state.models {
            Some(Ok(models)) if !models.is_empty() => {
                for model in models.iter().take(8) {
                    rows = rows.push(
                        crate::settings_app::settings_row(model.clone())
                            .action(tr("shell-use"), edit(Edit::Model(model.clone())))
                            .enabled(live)
                            .into_element(),
                    );
                }
            }
            Some(Ok(_)) => {
                rows = rows.push(
                    crate::settings_app::settings_row(tr("shell-the-gateway-lists-no-models"))
                        .into_element(),
                );
            }
            Some(Err(error)) => {
                rows = rows.push(crate::settings_app::settings_row(error.clone()).into_element());
            }
            None => {}
        }
        rows = rows.push(
            iced::widget::container(
                enz_text(tr("shell-workspace-folders"))
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding([6, 10]),
        );
        if summary.roots.is_empty() {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-no-folders-yet"))
                    .subtitle(tr(
                        "shell-a-session-can-only-work-inside-a-folder-listed-here",
                    ))
                    .into_element(),
            );
        }
        for root in &summary.roots {
            rows = rows.push(
                crate::settings_app::settings_row(root.clone())
                    .icon("folder")
                    .action(tr("shell-remove"), edit(Edit::RemoveRoot(root.clone())))
                    .enabled(live)
                    .into_element(),
            );
        }
        rows = rows.push(
            row![
                enz_text_field("", "/home/you/Projects", &state.root, |text| {
                    ParentMessage::Assistant(Message::Root(text))
                })
                .on_submit(edit(Edit::AddRoot(state.root.clone())))
                .into_element(),
                enz_button(
                    enzui_shell::i18n::tr("shell-add-2"),
                    edit(Edit::AddRoot(state.root.clone()))
                )
                .kind(ButtonKind::Secondary)
                .enabled(live && !state.root.trim().is_empty())
                .into_element(),
            ]
            .spacing(8)
            .padding([4, 10]),
        );
        rows = rows.push(
            iced::widget::container(
                enz_text(tr("shell-without-asking"))
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding([6, 10]),
        );
        for (key, title, on) in [
            (
                "allow_writes",
                tr("shell-change-files-in-these-folders"),
                summary.writes,
            ),
            (
                "allow_commands",
                tr("shell-run-commands-in-a-sandbox"),
                summary.commands,
            ),
            (
                "allow_network",
                tr("shell-use-the-network-from-commands"),
                summary.network,
            ),
        ] {
            rows = rows.push(
                crate::settings_app::settings_row(title)
                    .switch(on, move |on| {
                        ParentMessage::Assistant(Message::Edit(Edit::Flag(key, on)))
                    })
                    .enabled(live)
                    .into_element(),
            );
        }
        rows = rows.push(
            iced::widget::container(
                enz_text(tr("shell-skill-folders"))
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding([6, 10]),
        );
        for folder in &summary.skills {
            rows = rows.push(
                crate::settings_app::settings_row(folder.clone())
                    .icon("book-open")
                    .action(tr("shell-remove"), edit(Edit::RemoveSkills(folder.clone())))
                    .enabled(live)
                    .into_element(),
            );
        }
        rows = rows.push(
            row![
                enz_text_field(
                    "",
                    "/home/you/.config/enzimos/skills",
                    &state.skills,
                    |text| ParentMessage::Assistant(Message::Skills(text))
                )
                .on_submit(edit(Edit::AddSkills(state.skills.clone())))
                .into_element(),
                enz_button(
                    enzui_shell::i18n::tr("shell-add-2"),
                    edit(Edit::AddSkills(state.skills.clone()))
                )
                .kind(ButtonKind::Secondary)
                .enabled(live && !state.skills.trim().is_empty())
                .into_element(),
            ]
            .spacing(8)
            .padding([4, 10]),
        );
        rows = rows.push(match &state.found {
            _ if !state.running => crate::settings_app::settings_row(tr("shell-skills")).subtitle(tr("shell-start-the-assistant-to-see-and-create-its-skills")).subtitle_wrap(true).icon("sparkles").into_element(),
            None => crate::settings_app::settings_row(tr("shell-skills")).subtitle(tr("shell-reading")).icon("sparkles").into_element(),
            Some(Err(error)) => crate::settings_app::settings_row(tr("shell-skills-are-unavailable")).subtitle(error.clone()).subtitle_wrap(true).icon("triangle-alert").into_element(),
            Some(Ok(found)) => crate::settings_app::settings_row(match found.len() {
                0 => tr("shell-no-skills-yet").to_owned(),
                1 => "1 skill".to_owned(),
                count => tr!("shell-skills-2", "arg0" => format!("{:}", count)),
            })
            .subtitle(tr("shell-instructions-you-can-add-to-a-message-in-the-assistant-they-never-grant-file"))
            .subtitle_wrap(true)
            .icon("sparkles")
            .action(tr("shell-new-skill"), ParentMessage::Assistant(Message::OpenSkill))
            .enabled(live && !summary.writable_skills.is_empty())
            .into_element(),
        });
        for skill in state
            .found
            .as_ref()
            .and_then(|found| found.as_ref().ok())
            .into_iter()
            .flatten()
            .filter(|_| state.running)
            .take(32)
        {
            rows = rows.push(
                crate::settings_app::settings_row(skill.name.clone())
                    .subtitle(skill.description.clone())
                    .subtitle_wrap(true)
                    .into_element(),
            );
        }
        rows = rows.push(
            iced::widget::container(
                enz_text(tr("shell-tool-servers-mcp"))
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            )
            .padding([6, 10]),
        );
        rows = rows.push(
            crate::settings_app::settings_row(if summary.servers.is_empty() { tr("shell-no-servers") } else { tr("shell-add-a-tool-server") })
                .subtitle(tr("shell-a-program-it-runs-as-you-or-a-web-address-every-tool-call-still-asks-you-fir"))
                .subtitle_wrap(true)
                .action(tr("shell-add"), ParentMessage::Assistant(Message::OpenServer))
                .enabled(live && summary.servers.len() < 8)
                .into_element(),
        );
        for (name, enabled, target) in &summary.servers {
            let name_owned = name.clone();
            let control = row![
                enzui_widgets::toggle::enz_toggle(*enabled, "", move |on| {
                    ParentMessage::Assistant(Message::Edit(Edit::ServerEnabled(
                        name_owned.clone(),
                        on,
                    )))
                })
                .aria_label(enzui_shell::tr!("shell-a11y-use", "arg0" => enzui_shell::i18n::argument(&(name))))
                .enabled(live)
                .into_element(),
                enzui_widgets::toolbar_icon::enz_toolbar_icon(
                    "x",
                    edit(Edit::RemoveServer(name.clone()))
                )
                .aria_label(enzui_shell::tr!("shell-remove-3", "arg0" => enzui_shell::i18n::argument(&(name))))
                .tooltip(enzui_shell::i18n::tr("shell-remove"))
                .enabled(live)
                .into_element(),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            rows = rows.push(
                crate::settings_app::settings_row(name.clone())
                    .subtitle(target.clone())
                    .icon("plug")
                    .control(control)
                    .into_element(),
            );
        }
        if state.running {
            rows = rows.push(
                iced::widget::container(
                    enz_text(tr("shell-changes-reach-the-service-when-it-starts-again"))
                        .size_px(self.text_px(12.0))
                        .into_element(),
                )
                .padding([6, 10]),
            );
        }
        rows.into()
    }
}

impl SettingsApp {
    pub(super) fn assistant_dialog(&self) -> Option<Element<'_, ParentMessage>> {
        let state = &self.assistant;
        let palette = self.tokens().palette;
        let field = |message: fn(String) -> Field| {
            move |text: String| ParentMessage::Assistant(Message::Field(message(text)))
        };
        let heading = |title: &'static str, text: &'static str| -> Element<'_, ParentMessage> {
            column![
                enz_text(title)
                    .size_px(self.text_px(18.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
                enz_text(text)
                    .size_px(self.text_px(12.0))
                    .muted_on(palette.surface)
                    .wrap(true)
                    .into_element(),
            ]
            .spacing(4)
            .into()
        };
        let close = ParentMessage::Assistant(Message::Close);
        let (header, body, footer, close_label): (
            Element<'_, ParentMessage>,
            Element<'_, ParentMessage>,
            Element<'_, ParentMessage>,
            &str,
        ) = if let Some(draft) = &state.server {
            let mut body = column![
                enz_text_field(
                    enzui_shell::i18n::current().text("shell-name"),
                    "docs",
                    &draft.name,
                    field(Field::Name)
                )
                .id("assistant-server-name".into())
                .into_element(),
                enzui_widgets::view_switcher::enz_view_switcher(
                    vec![
                        enzui_widgets::view_switcher::SwitcherPage::new(
                            "program",
                            enzui_shell::i18n::tr("shell-program")
                        )
                        .with_icon("terminal"),
                        enzui_widgets::view_switcher::SwitcherPage::new(
                            "web",
                            tr("shell-web-address")
                        )
                        .with_icon("globe"),
                    ],
                    usize::from(draft.web),
                    |index| ParentMessage::Assistant(Message::ServerWeb(index == 1)),
                )
                .aria_label(tr("shell-server-kind"))
                .into_element(),
            ]
            .spacing(12);
            if draft.web {
                body = body
                    .push(enz_text_field(enzui_shell::i18n::current().text("shell-address"), "https://example.org/mcp", &draft.url, field(Field::Url)).id("assistant-server-url".into()).into_element())
                    .push(
                        enzui_widgets::secure_field::enz_secure_field(&draft.authorization, |text| ParentMessage::Assistant(Message::Field(Field::Authorization(Secret(text)))), draft.reveal, ParentMessage::Assistant(Message::Reveal))
                            .label(enzui_shell::i18n::current().text("shell-authorization-optional"))
                            .id("assistant-server-authorization".into()).into_element(),
                    )
                    .push(enz_text(tr("shell-sent-as-the-authorization-header-and-stored-only-in-your-private-configurati")).size_px(self.text_px(12.0)).muted_on(palette.surface).wrap(true).into_element());
            } else {
                body = body
                    .push(enz_text_field(enzui_shell::i18n::current().text("shell-program"), "/usr/bin/enzui-mcp", &draft.program, field(Field::Program)).id("assistant-server-program".into()).into_element())
                    .push(enzui_widgets::text_area::enz_text_area(&draft.arguments, |action| ParentMessage::Assistant(Message::Field(Field::Arguments(action)))).label(enzui_shell::i18n::current().text("shell-arguments-one-per-line")).rows(3).id("assistant-server-arguments").into_element())
                    .push(enz_text(tr("shell-the-program-runs-with-your-own-permissions-outside-the-command-sandbox-whene")).size_px(self.text_px(12.0)).muted_on(palette.surface).wrap(true).into_element());
            }
            if let Some(error) = &draft.error {
                body = body.push(
                    enz_text(error.clone())
                        .size_px(self.text_px(12.0))
                        .wrap(true)
                        .into_element(),
                );
            }
            let footer = row![
                iced::widget::space::horizontal(),
                enz_button(enzui_shell::i18n::tr("shell-cancel"), close.clone())
                    .kind(ButtonKind::Secondary)
                    .into_element(),
                enz_button(
                    enzui_shell::i18n::tr("shell-add-2"),
                    ParentMessage::Assistant(Message::SaveServer)
                )
                .kind(ButtonKind::Primary)
                .enabled(
                    !draft.name.trim().is_empty()
                        && if draft.web {
                            !draft.url.trim().is_empty()
                        } else {
                            !draft.program.trim().is_empty()
                        }
                )
                .into_element(),
            ]
            .spacing(8)
            .into();
            (
                heading(
                    tr("shell-add-a-tool-server"),
                    tr(
                        "shell-the-assistant-can-use-its-tools-in-new-sessions-each-tool-call-still-asks-yo",
                    ),
                ),
                body.into(),
                footer,
                tr("shell-close-add-a-tool-server"),
            )
        } else if let Some(draft) = &state.skill {
            let folders = self
                .assistant
                .config
                .as_ref()
                .and_then(|config| config.as_ref().ok())
                .and_then(Option::as_ref)
                .map(|config| agent_harness::summary(config).writable_skills)
                .unwrap_or_default();
            let mut body = column![
                enz_text_field(
                    enzui_shell::i18n::current().text("shell-name"),
                    "release-notes",
                    &draft.name,
                    field(Field::SkillName)
                )
                .id("assistant-skill-name".into())
                .into_element(),
                enz_text_field(
                    enzui_shell::i18n::current().text("shell-description"),
                    enzui_shell::i18n::current().text("shell-when-to-use-it-in-one-line"),
                    &draft.description,
                    field(Field::Description)
                )
                .id("assistant-skill-description".into())
                .into_element(),
                enzui_widgets::text_area::enz_text_area(&draft.instructions, |action| {
                    ParentMessage::Assistant(Message::Field(Field::Instructions(action)))
                })
                .label(enzui_shell::i18n::current().text("shell-instructions"))
                .rows(8)
                .id("assistant-skill-instructions")
                .into_element(),
            ]
            .spacing(12);
            if folders.len() > 1 {
                let choices = folders
                    .iter()
                    .map(|folder| {
                        (
                            folder.clone(),
                            ParentMessage::Assistant(Message::Field(Field::SkillFolder(
                                folder.clone(),
                            ))),
                        )
                    })
                    .collect();
                body = body.push(
                    crate::settings_app::settings_row(tr("shell-save-in"))
                        .value_menu(
                            draft.folder.clone(),
                            self.menu.as_deref() == Some("assistant-skill-folder"),
                            ParentMessage::Menu("assistant-skill-folder".into(), true),
                            ParentMessage::Menu("assistant-skill-folder".into(), false),
                            choices,
                        )
                        .enabled(!draft.busy)
                        .into_element(),
                );
            } else {
                body = body.push(
                    enz_text(tr!("shell-saved-in", "arg0" => draft.folder.clone()))
                        .size_px(self.text_px(12.0))
                        .muted_on(palette.surface)
                        .wrap(true)
                        .into_element(),
                );
            }
            if let Some(error) = &draft.error {
                body = body.push(
                    enz_text(error.clone())
                        .size_px(self.text_px(12.0))
                        .wrap(true)
                        .into_element(),
                );
            }
            let footer = row![
                iced::widget::space::horizontal(),
                enz_button(enzui_shell::i18n::tr("shell-cancel"), close.clone())
                    .kind(ButtonKind::Secondary)
                    .enabled(!draft.busy)
                    .into_element(),
                enz_button(
                    if draft.busy {
                        enzui_shell::i18n::tr("shell-creating")
                    } else {
                        enzui_shell::i18n::tr("shell-create")
                    },
                    ParentMessage::Assistant(Message::CreateSkill)
                )
                .kind(ButtonKind::Primary)
                .enabled(!draft.busy && !draft.name.trim().is_empty())
                .into_element(),
            ]
            .spacing(8)
            .into();
            (
                heading(
                    tr("shell-new-skill-2"),
                    tr(
                        "shell-a-skill-adds-these-instructions-to-a-message-when-you-choose-it-it-never-run",
                    ),
                ),
                body.into(),
                footer,
                tr("shell-close-new-skill"),
            )
        } else {
            return None;
        };
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(Element::from(
            iced::widget::container(body).padding(iced::Padding {
                right: 12.0,
                ..Default::default()
            }),
        ))
        .id("assistant-dialog-body".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .into_element();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![header],
            scroll,
            vec![footer],
            16.0,
            600.0,
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(480.0)
                .padding(20.0)
                .on_dismiss(move || close.clone())
                .close_label(close_label)
                .into_element(),
        )
    }
}
