use super::{Message as ParentMessage, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::local_ai::{
    self, Change, ChatGpt, Failure, Gateway, Inference, Online, PRESETS, ProviderEdit, Service,
    SpeechOffer,
};
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, enz_button};
use enzui_widgets::combo_box::enz_combo_box;
use enzui_widgets::text_field::enz_text_field_owned;
use iced::{Element, Task, widget::column};

#[derive(Default)]
pub(super) struct State {
    inference: Option<Result<Inference, Failure>>,
    native_drop: Option<enzui_widgets::file_drop::FileDrop>,
    gateway: Option<Result<Gateway, Failure>>,
    reading: bool,
    // The service a change is being sent to.
    busy: Option<Service>,
    notice: Option<(Service, String)>,
    chatgpt: Option<ChatGpt>,
    /// Speech models the model tool can download; empty without the runtime.
    offers: Vec<SpeechOffer>,
    /// The provider being added: preset (past the presets: another), name, address, key.
    preset: usize,
    name: String,
    url: String,
    key: String,
}

/// What one read of the services found.
type Readings = (
    Result<Inference, Failure>,
    Result<Gateway, Failure>,
    Option<ChatGpt>,
    Vec<SpeechOffer>,
);

#[derive(Clone, Debug)]
pub(super) enum Message {
    Read(Box<Readings>),
    Preset(usize),
    Name(String),
    Url(String),
    Key(String),
    /// Add the provider in the form, or remove one by name.
    Edit(ProviderEdit),
    Edited(Result<(), String>),
    SignIn,
    SignOut,
    /// A sign-in finished, failed or was given up.
    SignedIn(Result<(), String>),
    Set(Change),
    Applied(Service, Result<String, Failure>),
    /// `enzimos-inference-models add`'s answer for a chosen file.
    ModelAdded(Result<String, String>),
    Dropped(enzui_widgets::file_drop::FileDrop),
    /// Dictation uses this speech model.
    SpeechDefault(String),
    /// Download and add a speech model from the catalog.
    Fetch(String),
    /// The model tool's answer to a choice or a download.
    ToolDone(Result<String, String>),
}

/// Where the model registry tool is installed.
const MODELS_TOOL: &str = "/usr/bin/enzimos-inference-models";
/// The runtime the downloadable speech models need.
const SPEECH_ONNX: &str = "/usr/lib/enzimos-inference/speech-onnx/sherpa-onnx-offline";

/// Runs the model registry tool; its message without the tool's name.
fn models_tool(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = std::process::Command::new(MODELS_TOOL)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|_| tr("shell-the-model-tool-is-not-installed").to_owned())?;
    let text = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        text.lines()
            .last()
            .unwrap_or_default()
            .trim()
            .trim_start_matches("enzimos-inference-models: ")
            .to_owned()
    };
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        Err(text(&output.stderr))
    }
}

fn tool_task(
    args: Vec<std::ffi::OsString>,
    done: fn(Result<String, String>) -> Message,
) -> Task<ParentMessage> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                models_tool(
                    &args
                        .iter()
                        .map(std::ffi::OsString::as_os_str)
                        .collect::<Vec<_>>(),
                )
            })
            .await
            .unwrap_or_else(|_| Err(tr("shell-the-model-tool-is-not-installed").to_owned()))
        },
        move |result| send(done(result)),
    )
}

fn send(message: Message) -> ParentMessage {
    ParentMessage::LocalAi(message)
}

fn read() -> Task<ParentMessage> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                let gateway = local_ai::read_gateway();
                let chatgpt = gateway
                    .as_ref()
                    .is_ok_and(|g| g.providers.iter().any(|p| p.kind == "chatgpt-login"))
                    .then(|| local_ai::chatgpt().ok())
                    .flatten();
                let offers = if std::path::Path::new(SPEECH_ONNX).is_file() {
                    std::process::Command::new(MODELS_TOOL)
                        .arg("catalog")
                        .stdin(std::process::Stdio::null())
                        .output()
                        .ok()
                        .filter(|output| output.status.success())
                        .map(|output| {
                            local_ai::parse_catalog(&String::from_utf8_lossy(&output.stdout))
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                (local_ai::read_inference(), gateway, chatgpt, offers)
            })
            .await
            .unwrap_or((
                Err(Failure::Unavailable),
                Err(Failure::Unavailable),
                None,
                Vec::new(),
            ))
        },
        |readings| send(Message::Read(Box::new(readings))),
    )
}

// Choices around a current value: the presets, plus the current one when it
// is not among them.
fn presets(current: u32, values: &[u32]) -> Vec<u32> {
    let mut all = values.to_vec();
    if !all.contains(&current) {
        all.push(current);
        all.sort_unstable();
    }
    all
}

/// Signs in to ChatGPT: adds the provider when missing, opens the sign-in page
/// and waits (up to ten minutes) for the gateway to finish.
fn sign_in(add: bool) -> Result<(), String> {
    let failed = |failure: Failure| failure.message(Service::Gateway);
    if add {
        local_ai::change_providers(&ProviderEdit::AddChatGpt)?;
    }
    // A restarted gateway takes a moment to listen again.
    let mut url = Err(Failure::Unavailable);
    for _ in 0..50 {
        url = local_ai::chatgpt_sign_in();
        if url != Err(Failure::Unavailable) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let url = url.map_err(failed)?;
    std::process::Command::new("/usr/bin/xdg-open")
        .arg(&url)
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|_| tr("shell-the-browser-could-not-be-opened").to_owned())?;
    for _ in 0..600 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let status = local_ai::chatgpt().map_err(failed)?;
        if !status.waiting {
            return match status.error {
                Some(error) if !status.signed_in => Err(error),
                _ => Ok(()),
            };
        }
    }
    Err(tr("shell-the-sign-in-was-not-finished-in-time").into())
}

fn blocking(
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
    done: fn(Result<(), String>) -> Message,
) -> Task<ParentMessage> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(work)
                .await
                .unwrap_or_else(|_| Err(tr("shell-it-gave-no-reason").into()))
        },
        move |result| send(done(result)),
    )
}

fn seconds(n: u32) -> String {
    match n {
        n if n < 60 => tr!("shell-after-seconds", "arg0" => format!("{:}", n)),
        60 => enzui_shell::i18n::tr("shell-after-1-minute").into(),
        n if n < 3600 => tr!("shell-after-minutes", "arg0" => format!("{:}", n / 60)),
        _ => enzui_shell::i18n::tr("shell-after-1-hour").into(),
    }
}

impl SettingsApp {
    /// Adds the model file the user chose to this account's registry.
    pub(super) fn local_ai_add_model(&mut self, path: std::path::PathBuf) -> Task<ParentMessage> {
        if self.local_ai.busy.is_some() {
            return Task::none();
        }
        self.local_ai.busy = Some(Service::Inference);
        self.local_ai.notice = Some((Service::Inference, tr("shell-adding-the-model").to_owned()));
        tool_task(
            vec!["add".into(), path.into_os_string()],
            Message::ModelAdded,
        )
    }

    // Adding a model needs the on-device AI service's model tool: offered
    // while the service answers, or where its tool is installed.
    pub(super) fn model_import_offered(&self) -> bool {
        match &self.local_ai.inference {
            Some(Ok(_)) => true,
            Some(Err(_)) => self.offers(enzui_shell::platform::Feature::LocalAiModels),
            None => false,
        }
    }

    pub(super) fn local_ai_read(&mut self) -> Task<ParentMessage> {
        if self.local_ai.reading || self.local_ai.inference.is_some() {
            return Task::none();
        }
        self.local_ai.reading = true;
        read()
    }

    pub(super) fn local_ai_message(&mut self, message: Message) -> Task<ParentMessage> {
        let live = self.controls_live();
        let import_offered = self.model_import_offered() && self.page == super::Page::Agents;
        let state = &mut self.local_ai;
        match message {
            Message::Read(readings) => {
                let (inference, gateway, chatgpt, offers) = *readings;
                state.reading = false;
                state.offers = offers;
                state.inference = Some(inference);
                state.gateway = Some(gateway);
                state.chatgpt = chatgpt;
            }
            Message::Preset(index) => {
                state.preset = index;
                let (name, url) = PRESETS.get(index).copied().unwrap_or(("", "https://"));
                name.clone_into(&mut state.name);
                url.clone_into(&mut state.url);
            }
            Message::Name(text) => state.name = text,
            Message::Url(text) => state.url = text,
            Message::Key(text) => state.key = text,
            Message::Edit(edit) => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Gateway);
                state.notice = None;
                return blocking(move || local_ai::change_providers(&edit), Message::Edited);
            }
            Message::Edited(result) => {
                state.busy = None;
                match result {
                    Ok(()) => {
                        state.name.clear();
                        state.url.clear();
                        state.key.clear();
                    }
                    Err(error) => state.notice = Some((Service::Gateway, error)),
                }
                state.reading = true;
                // The restarted gateway answers again shortly.
                return Task::perform(
                    async { tokio::time::sleep(std::time::Duration::from_millis(600)).await },
                    |()| ParentMessage::Noop,
                )
                .chain(read());
            }
            Message::SignIn => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                let add = !state
                    .gateway
                    .as_ref()
                    .and_then(|g| g.as_ref().ok())
                    .is_some_and(|g| g.providers.iter().any(|p| p.kind == "chatgpt-login"));
                state.busy = Some(Service::Gateway);
                state.notice = Some((
                    Service::Gateway,
                    tr("shell-finish-signing-in-in-your-browser").into(),
                ));
                return blocking(move || sign_in(add), Message::SignedIn);
            }
            Message::SignOut => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Gateway);
                state.notice = None;
                return blocking(
                    || local_ai::chatgpt_sign_out().map_err(|f| f.message(Service::Gateway)),
                    Message::SignedIn,
                );
            }
            Message::SignedIn(result) => {
                state.busy = None;
                state.notice = result.err().map(|error| (Service::Gateway, error));
                state.reading = true;
                return read();
            }
            Message::Set(change) => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                let service = change.service();
                let known = match service {
                    Service::Inference => state
                        .inference
                        .as_ref()
                        .and_then(|r| r.as_ref().ok())
                        .map(|i| (i.revision.clone(), i.persistent)),
                    Service::Gateway => state
                        .gateway
                        .as_ref()
                        .and_then(|r| r.as_ref().ok())
                        .map(|g| (g.revision.clone(), g.persistent)),
                };
                let Some((revision, persistent)) = known else {
                    return Task::none();
                };
                state.busy = Some(service);
                state.notice = None;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            local_ai::apply(&change, &revision, persistent)
                        })
                        .await
                        .unwrap_or(Err(Failure::ReplyLost))
                    },
                    move |result| send(Message::Applied(service, result)),
                );
            }
            Message::Dropped(drop) => {
                if !live
                    || !import_offered
                    || state.busy.is_some()
                    || state.native_drop.is_some()
                    || drop.paths().len() != 1
                    || drop
                        .action()
                        .is_some_and(|action| action != enzui_widgets::file_drop::DragAction::Copy)
                {
                    return Task::none();
                }
                let path = drop.paths()[0].clone();
                state.native_drop = Some(drop);
                return self.local_ai_add_model(path);
            }
            Message::ModelAdded(result) => {
                if let Some(drop) = state.native_drop.take()
                    && result.is_ok()
                {
                    drop.finish();
                }
                state.busy = None;
                let text = match result {
                    Ok(added) => {
                        tr!("shell-model-added", "arg0" => added.trim_start_matches("added ").to_owned())
                    }
                    Err(error) => tr!("shell-model-not-added", "arg0" => error),
                };
                state.notice = Some((Service::Inference, text));
                state.reading = true;
                return read();
            }
            Message::SpeechDefault(id) => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Inference);
                return tool_task(vec!["default".into(), id.into()], Message::ToolDone);
            }
            Message::Fetch(id) => {
                if state.busy.is_some() || !live {
                    return Task::none();
                }
                let title = state
                    .offers
                    .iter()
                    .find(|o| o.id == id)
                    .map_or_else(|| id.clone(), |o| o.title.clone());
                state.busy = Some(Service::Inference);
                state.notice = Some((
                    Service::Inference,
                    tr!("shell-downloading-speech-model", "arg0" => title),
                ));
                return tool_task(vec!["fetch".into(), id.into()], Message::ToolDone);
            }
            Message::ToolDone(result) => {
                state.busy = None;
                state.notice = match result {
                    Ok(message) if message.starts_with("added ") => Some((
                        Service::Inference,
                        tr!("shell-model-added", "arg0" => message.trim_start_matches("added ").to_owned()),
                    )),
                    Ok(_) => None,
                    Err(error) => Some((
                        Service::Inference,
                        tr!("shell-model-not-added", "arg0" => error),
                    )),
                };
                state.reading = true;
                return read();
            }
            Message::Applied(service, result) => {
                state.busy = None;
                if let Err(failure) = &result {
                    state.notice = Some((service, failure.message(service)));
                }
                // Whatever happened, what is shown next is what the service says now.
                state.reading = true;
                return read();
            }
        }
        Task::none()
    }

    fn local_ai_notice<'a>(&self, service: Service) -> Option<Element<'a, ParentMessage>> {
        let (from, text) = self.local_ai.notice.as_ref()?;
        (*from == service).then(|| self.caption_row_text(text.clone()))
    }

    fn caption_row_text<'a>(&self, text: String) -> Element<'a, ParentMessage> {
        iced::widget::container(
            enzui_widgets::text::enz_text(text)
                .size(enzui_widgets::text::TextSize::Footnote)
                .muted_on(self.tokens().palette.surface)
                .wrap(true)
                .into_element(),
        )
        .padding([6, 12])
        .into()
    }

    fn inference_rows<'a>(&self, inference: &Inference) -> iced::widget::Column<'a, ParentMessage> {
        let p = &inference.preferences;
        let live = inference.persistent && self.local_ai.busy.is_none() && self.controls_live();
        let (title, mut details) = match &inference.loaded_model {
            Some(model) => (tr!("shell-using", "arg0" => model.clone()), Vec::new()),
            None => (
                tr("shell-ready").to_owned(),
                vec![tr("shell-no-model-loaded-one-loads-when-something-asks").to_owned()],
            ),
        };
        if let Some(backend) = &inference.backend {
            details.push(tr!("shell-on-2", "arg0" => backend.clone()));
        }
        if inference.jobs > 0 {
            details.push(enzui_shell::i18n::count(
                "amount-requests-running",
                inference.jobs,
            ));
        }
        if let Some(peak) = inference.peak_rss_mib {
            details.push(tr!("shell-used-up-to-mb", "arg0" => format!("{:}", peak)));
        }
        let mut rows = column![
            self.mark_setting(
                "agents/local-ai",
                crate::settings_app::settings_row(title)
                    .subtitle(details.join(" · "))
                    .subtitle_wrap(true)
                    .icon("cpu")
                    .into_element()
            )
        ]
        .spacing(2);
        if let Some(failure) = &inference.failure {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-the-last-request-failed"))
                    .subtitle(failure.clone())
                    .subtitle_wrap(true)
                    .icon("triangle-alert")
                    .into_element(),
            );
        }
        let choice = |title: &'static str,
                      subtitle: &'static str,
                      id: &'static str,
                      names: Vec<String>,
                      selected: Option<usize>,
                      pick: Box<dyn Fn(usize) -> Option<Change>>|
         -> Element<'a, ParentMessage> {
            let control = enz_combo_box("", names, None, |_| ParentMessage::Noop)
                .aria_label(title)
                .on_select_index(move |index| {
                    pick(index).map_or(ParentMessage::Noop, |change| send(Message::Set(change)))
                })
                .selected_index(selected)
                .enabled(live)
                .width(220)
                .id(id.into())
                .into_element();
            crate::settings_app::settings_row(title)
                .subtitle(subtitle)
                .control(control)
                .into_element()
        };
        let models = inference.models.clone();
        let current_model = models.iter().position(|m| *m == p.default_model);
        rows = rows.push(choice(
            enzui_shell::i18n::tr("shell-model"),
            tr("shell-used-when-an-app-or-the-assistant-does-not-ask-for-one"),
            "local-ai-model",
            models.clone(),
            current_model,
            Box::new(move |index| models.get(index).cloned().map(Change::DefaultModel)),
        ));
        let threads: [Option<u8>; 3] = [None, Some(1), Some(2)];
        rows = rows.push(choice(
            tr("shell-processor-threads"),
            tr("shell-how-much-of-the-processor-a-request-may-use"),
            "local-ai-threads",
            vec![
                enzui_shell::i18n::tr("shell-automatic").into(),
                enzui_shell::i18n::count("processor-thread-count", 1),
                enzui_shell::i18n::count("processor-thread-count", 2),
            ],
            threads.iter().position(|t| *t == p.cpu_threads),
            Box::new(move |index| threads.get(index).copied().map(Change::CpuThreads)),
        ));
        let memory = presets(p.memory_mib, &[512, 1024, 1536, 1900, 2048]);
        rows = rows.push(choice(
            tr("shell-memory-limit"),
            tr("shell-larger-models-need-more"),
            "local-ai-memory",
            memory
                .iter()
                .map(|m| tr!("shell-mb-5", "arg0" => format!("{:}", m)))
                .collect(),
            memory.iter().position(|m| *m == p.memory_mib),
            Box::new(move |index| memory.get(index).copied().map(Change::MemoryMib)),
        ));
        let idle = presets(p.idle_seconds, &[30, 60, 300, 900, 3600]);
        rows = rows.push(choice(
            tr("shell-free-memory-when-idle"),
            tr("shell-the-model-unloads-after-this-long-without-a-request"),
            "local-ai-idle",
            idle.iter().map(|s| seconds(*s)).collect(),
            idle.iter().position(|s| *s == p.idle_seconds),
            Box::new(move |index| idle.get(index).copied().map(Change::IdleSeconds)),
        ));
        let context = presets(p.context, &[4096, 6144, 8192]);
        rows = rows.push(choice(
            tr("shell-context-length"),
            tr("shell-how-much-text-a-request-can-carry-longer-uses-more-memory"),
            "local-ai-context",
            context
                .iter()
                .map(|c| tr!("shell-tokens", "arg0" => format!("{:}", c)))
                .collect(),
            context.iter().position(|c| *c == p.context),
            Box::new(move |index| context.get(index).copied().map(Change::Context)),
        ));
        // Automatic, every installed engine, and the current choice even when
        // an older service does not list what is installed.
        let mut engines = vec!["auto".to_owned()];
        engines.extend(inference.installed_runtimes.iter().cloned());
        if !engines.contains(&p.backend) {
            engines.push(p.backend.clone());
        }
        let names = engines
            .iter()
            .map(|e| match e.as_str() {
                "auto" => tr("shell-automatic").to_owned(),
                "cpu" => enzui_shell::i18n::tr("shell-processor-cpu").to_owned(),
                "vulkan" => "Vulkan".to_owned(),
                "sycl" => "SYCL".to_owned(),
                "cuda" => "CUDA".to_owned(),
                other => other.to_owned(),
            })
            .collect();
        let selected = engines.iter().position(|e| *e == p.backend);
        rows = rows.push(choice(
            enzui_shell::i18n::tr("shell-engine"),
            tr("shell-automatic-picks-the-qualified-one-for-this-computer"),
            "local-ai-engine",
            names,
            selected,
            Box::new(move |index| engines.get(index).cloned().map(Change::Backend)),
        ));
        if !inference.persistent {
            rows = rows.push(self.caption_row_text(Failure::ReadOnly.message(Service::Inference)));
        }
        rows
    }

    fn gateway_rows<'a>(&self, gateway: &Gateway) -> iced::widget::Column<'a, ParentMessage> {
        let live = gateway.persistent && self.local_ai.busy.is_none() && self.controls_live();
        let selectable: Vec<String> = gateway
            .providers
            .iter()
            .filter(|p| p.selectable)
            .map(|p| p.id.clone())
            .collect();
        let selected = selectable
            .iter()
            .position(|id| *id == gateway.default_provider);
        let choices = selectable.clone();
        let control = enz_combo_box("", selectable, None, |_| ParentMessage::Noop)
            .aria_label(tr("shell-default-provider"))
            .on_select_index(move |index| {
                choices
                    .get(index)
                    .cloned()
                    .map_or(ParentMessage::Noop, |id| {
                        send(Message::Set(Change::DefaultProvider(id)))
                    })
            })
            .selected_index(selected)
            .enabled(live)
            .width(220)
            .id("local-ai-provider".into())
            .into_element();
        let mut rows = column![
            self.mark_setting(
                "agents/ai-providers",
                crate::settings_app::settings_row(tr("shell-default-provider"))
                    .subtitle(tr(
                        "shell-where-requests-go-when-they-do-not-name-a-provider"
                    ))
                    .control(control)
                    .into_element(),
            )
        ]
        .spacing(2);
        let editing = self.local_ai.busy.is_none() && self.controls_live();
        let chatgpt = self.local_ai.chatgpt.clone().unwrap_or_default();
        let account_button = |signed_in: bool| {
            if signed_in {
                enz_button(tr("shell-sign-out"), send(Message::SignOut))
            } else {
                enz_button(tr("shell-sign-in"), send(Message::SignIn))
            }
            .kind(ButtonKind::Secondary)
            .enabled(editing)
            .into_element()
        };
        for provider in &gateway.providers {
            let state = match (provider.enabled, provider.kind.as_str()) {
                (_, "chatgpt-login") => match (&chatgpt.email, chatgpt.signed_in) {
                    (Some(email), true) => tr!("shell-signed-in-as", "arg0" => email.clone()),
                    (None, true) => tr("shell-signed-in").into(),
                    _ => tr("shell-not-signed-in").into(),
                },
                (false, _) => tr("shell-off").into(),
                (true, "local") => tr("shell-on-this-computer").into(),
                (true, _) => tr("shell-sends-requests-over-the-internet").into(),
            };
            let mut row = crate::settings_app::settings_row(provider.id.clone())
                .subtitle(format!("{} · {state}", provider.kind))
                .icon(if provider.kind == "local" {
                    "cpu"
                } else {
                    "cloud"
                });
            row = match provider.kind.as_str() {
                "local" => row,
                "chatgpt-login" => row.control(account_button(chatgpt.signed_in)),
                _ => row.control(
                    enz_button(
                        tr("shell-remove"),
                        send(Message::Edit(ProviderEdit::Remove(provider.id.clone()))),
                    )
                    .kind(ButtonKind::Secondary)
                    .enabled(editing)
                    .into_element(),
                ),
            };
            rows = rows.push(row.into_element());
        }
        if !gateway.providers.iter().any(|p| p.kind == "chatgpt-login") {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-chatgpt"))
                    .subtitle(tr("shell-use-your-chatgpt-plan-requests-go-to-openai"))
                    .subtitle_wrap(true)
                    .icon("cloud")
                    .control(account_button(false))
                    .into_element(),
            );
        }
        rows = rows.push(self.provider_form(editing));
        if !gateway.persistent {
            rows = rows.push(self.caption_row_text(Failure::ReadOnly.message(Service::Gateway)));
        }
        rows
    }

    /// Adding an OpenAI-compatible provider: a preset or another address, and its key.
    fn provider_form<'a>(&self, editing: bool) -> Element<'a, ParentMessage> {
        let state = &self.local_ai;
        let mut names: Vec<String> = PRESETS.iter().map(|(name, _)| (*name).to_owned()).collect();
        names.push(tr("shell-another-provider").into());
        let preset = enz_combo_box("", names, None, |_| ParentMessage::Noop)
            .aria_label(tr("shell-provider"))
            .on_select_index(|index| send(Message::Preset(index)))
            .selected_index((!state.url.is_empty()).then_some(state.preset))
            .enabled(editing)
            .width(220)
            .into_element();
        let name =
            enz_text_field_owned(tr("shell-name"), "openrouter", state.name.clone(), |text| {
                send(Message::Name(text))
            })
            .id("ai-provider-name".into())
            .into_element();
        let url =
            enz_text_field_owned(tr("shell-address"), "https://", state.url.clone(), |text| {
                send(Message::Url(text))
            })
            .id("ai-provider-url".into())
            .into_element();
        let key = enz_text_field_owned(tr("shell-api-key"), "", state.key.clone(), |text| {
            send(Message::Key(text))
        })
        .secure(true)
        .id("ai-provider-key".into())
        .into_element();
        let ready = local_ai::provider_name(&state.name) && local_ai::endpoint(&state.url);
        let edit = ProviderEdit::AddOnline(Online {
            name: state.name.clone(),
            base_url: state.url.clone(),
            key: Some(state.key.clone()).filter(|k| !k.trim().is_empty()),
        });
        let add = enz_button(tr("shell-add"), send(Message::Edit(edit)))
            .kind(ButtonKind::Secondary)
            .enabled(editing && ready)
            .into_element();
        let mut form = column![
            crate::settings_app::settings_row(tr("shell-add-an-online-provider"))
                .subtitle(tr("shell-any-openai-compatible-service-such-as-openrouter-the-key-is-kept-private-to-"))
                .subtitle_wrap(true)
                .icon("plus")
                .control(preset)
                .into_element()
        ]
        .spacing(6);
        if !state.url.is_empty() || state.preset >= PRESETS.len() {
            form = form.push(
                column![
                    name,
                    url,
                    key,
                    iced::widget::row![iced::widget::space().width(iced::Length::Fill), add]
                ]
                .spacing(6)
                .padding([4, 16]),
            );
        }
        form.into()
    }

    pub(super) fn local_ai_sections<'a>(
        &'a self,
        body: iced::widget::Column<'a, ParentMessage>,
    ) -> iced::widget::Column<'a, ParentMessage> {
        let state = &self.local_ai;
        let mut on_device = column![].spacing(2);
        match &state.inference {
            None => {
                on_device = on_device.push(
                    self.mark_setting(
                        "agents/local-ai",
                        crate::settings_app::settings_row(tr("shell-checking"))
                            .enabled(false)
                            .into_element(),
                    ),
                );
            }
            Some(Err(failure)) => {
                let (title, subtitle) = match failure {
                    Failure::Unavailable => (tr("shell-not-running"), tr("shell-the-on-device-ai-service-runs-for-this-login-once-enzimos-provides-a-model").to_owned()),
                    other => (tr("shell-not-available"), other.message(Service::Inference)),
                };
                on_device = on_device.push(
                    self.mark_setting(
                        "agents/local-ai",
                        crate::settings_app::settings_row(title)
                            .subtitle(subtitle)
                            .subtitle_wrap(true)
                            .icon("cpu")
                            .into_element(),
                    ),
                );
            }
            Some(Ok(inference)) => on_device = self.inference_rows(inference),
        }
        if self.model_import_offered() {
            let add = enzui_widgets::button::enz_button(
                tr("shell-add-a-model"),
                ParentMessage::FileChooser(super::settings_file_chooser::Message::OpenModel),
            )
            .kind(enzui_widgets::button::ButtonKind::Secondary)
            .enabled(state.busy.is_none() && self.controls_live())
            .into_element();
            let target: Element<'_, ParentMessage> = crate::settings_app::settings_row(tr(
                "shell-models-on-this-computer",
            ))
            .subtitle(tr(
                "shell-add-a-gguf-model-such-as-minicpm-or-a-whisper-speech-model-from-a-file",
            ))
            .subtitle_wrap(true)
            .icon("package")
            .control(add)
            .into_element();
            on_device = on_device.push(
                enzui_widgets::file_drop::enz_file_drop_target_with_receipt(target, |drop| {
                    send(Message::Dropped(drop))
                })
                .enabled(
                    state.busy.is_none() && state.native_drop.is_none() && self.controls_live(),
                )
                .copy_only(true)
                .key("local-ai-model-import"),
            );
        }
        // Dictation (Super+H) is the speech model's user.
        if let Some(Ok(inference)) = &state.inference {
            let voice = inference.voice_model();
            let subtitle = match voice {
                Some(model) => tr!("shell-voice-typing-uses", "arg0" => model.to_owned()),
                None => tr("shell-voice-typing-needs-a-speech-model").to_owned(),
            };
            let mut row = crate::settings_app::settings_row(tr("shell-voice-typing"))
                .subtitle(subtitle)
                .subtitle_wrap(true)
                .icon("mic");
            if inference.speech_models.len() > 1 {
                let models = inference.speech_models.clone();
                let selected = models.iter().position(|m| Some(m.as_str()) == voice);
                row = row.control(
                    enz_combo_box("", models.clone(), None, |_| ParentMessage::Noop)
                        .aria_label(tr("shell-speech-model"))
                        .on_select_index(move |index| {
                            models.get(index).map_or(ParentMessage::Noop, |id| {
                                send(Message::SpeechDefault(id.clone()))
                            })
                        })
                        .selected_index(selected)
                        .enabled(state.busy.is_none() && self.controls_live())
                        .width(220)
                        .into_element(),
                );
            }
            on_device = on_device.push(row.into_element());
            for offer in state.offers.iter().filter(|offer| !offer.added) {
                let download =
                    enz_button(tr("shell-download"), send(Message::Fetch(offer.id.clone())))
                        .kind(ButtonKind::Secondary)
                        .enabled(state.busy.is_none() && self.controls_live())
                        .into_element();
                on_device = on_device.push(
                    crate::settings_app::settings_row(offer.title.clone())
                        .subtitle(tr!("shell-speech-model-offer", "arg0" => offer.size.clone(), "arg1" => offer.languages.clone()))
                        .subtitle_wrap(true)
                        .icon("download")
                        .control(download)
                        .into_element(),
                );
            }
        }
        if let Some(notice) = self.local_ai_notice(Service::Inference) {
            on_device = on_device.push(notice);
        }
        let mut providers = column![].spacing(2);
        match &state.gateway {
            None => {
                providers = providers.push(
                    self.mark_setting(
                        "agents/ai-providers",
                        crate::settings_app::settings_row(tr("shell-checking"))
                            .enabled(false)
                            .into_element(),
                    ),
                );
            }
            Some(Err(failure)) => {
                let subtitle = match failure {
                    Failure::Unavailable => {
                        tr("shell-the-ai-provider-service-is-not-running-for-this-login").to_owned()
                    }
                    other => other.message(Service::Gateway),
                };
                providers = providers.push(
                    self.mark_setting(
                        "agents/ai-providers",
                        crate::settings_app::settings_row(tr("shell-not-running"))
                            .subtitle(subtitle)
                            .subtitle_wrap(true)
                            .icon("cloud")
                            .into_element(),
                    ),
                );
            }
            Some(Ok(gateway)) => providers = self.gateway_rows(gateway),
        }
        if let Some(notice) = self.local_ai_notice(Service::Gateway) {
            providers = providers.push(notice);
        }
        body.push(
            self.section_card_for("agents/local-ai", on_device)
                .title(tr("shell-on-device-ai"))
                .description(tr("shell-runs-models-on-this-computer-changes-apply-to-the-next-request-work-in-progr"))
                .into_element(),
        )
        .push(
            self.section_card_for("agents/ai-providers", providers)
                .title(tr("shell-ai-providers"))
                .description(tr("shell-a-request-that-fails-on-this-computer-is-never-sent-to-another-provider-on-i"))
                .into_element(),
        )
    }
}
