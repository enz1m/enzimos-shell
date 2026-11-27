use super::{Message as ParentMessage, SettingsApp, model_downloads};
use enzui_shell::i18n::tr;
use enzui_shell::local_ai::{
    self, Change, ChatGpt, Failure, Gateway, Inference, ModelOffer, Online, PRESETS, ProviderEdit,
    Service,
};
use enzui_shell::{tr, writing_tools};
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
    offers: Vec<ModelOffer>,
    downloads: Vec<model_downloads::Reading>,
    advanced: bool,
    benchmark: Option<(String, iced::task::Handle)>,
    benchmark_serial: u64,
    benchmarks: std::collections::BTreeMap<String, Result<local_ai::Benchmark, String>>,
    enabled: bool,
    writing: writing_tools::Preferences,
    writing_error: Option<String>,
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
    Vec<ModelOffer>,
    Vec<model_downloads::Reading>,
    bool,
    writing_tools::Preferences,
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
    Fetch(String),
    Queued(Result<(), String>),
    RefreshStatus,
    Benchmark(String),
    BenchmarkFinished(u64, String, Result<local_ai::Benchmark, String>),
    CancelBenchmark,
    ToggleAdvanced,
    SetEnabled(bool),
    WritingEnabled(bool),
    WritingModel(Option<String>),
    Enabled(Result<(), String>),
    /// The model tool's answer to a choice or a download.
    ToolDone(Result<String, String>),
}

fn models_tool(args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = std::process::Command::new(local_ai::MODELS_TOOL)
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
                let catalog = local_ai::read_model_catalog();
                let downloads = catalog
                    .iter()
                    .filter_map(|offer| model_downloads::reading(&offer.id))
                    .collect();
                (
                    local_ai::read_inference(),
                    gateway,
                    chatgpt,
                    catalog,
                    downloads,
                    local_ai::enabled(),
                    writing_tools::Preferences::load(),
                )
            })
            .await
            .unwrap_or_else(|_| {
                (
                    Err(Failure::Unavailable),
                    Err(Failure::Unavailable),
                    None,
                    Vec::new(),
                    Vec::new(),
                    false,
                    Default::default(),
                )
            })
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

impl State {
    fn inference(&self) -> Option<&Inference> {
        self.inference.as_ref()?.as_ref().ok()
    }

    pub(super) fn provider_selectable(&self, id: &str) -> bool {
        id == "local"
            || self
                .gateway
                .as_ref()
                .and_then(|gateway| gateway.as_ref().ok())
                .is_some_and(|gateway| {
                    gateway
                        .providers
                        .iter()
                        .any(|provider| provider.id == id && provider.selectable)
                })
    }

    fn model_busy(&self) -> bool {
        self.busy.is_some()
            || self.benchmark.is_some()
            || self.downloads.iter().any(model_downloads::Reading::pending)
    }
}

impl SettingsApp {
    pub(super) fn local_text_model_ids(&self) -> Vec<String> {
        self.local_ai
            .inference()
            .map_or_else(Vec::new, |inference| inference.models.clone())
    }

    pub(super) fn selectable_providers(&self) -> Vec<String> {
        let mut ids = vec!["local".to_owned()];
        if let Some(Ok(gateway)) = &self.local_ai.gateway {
            ids.extend(
                gateway
                    .providers
                    .iter()
                    .filter(|provider| provider.id != "local" && provider.selectable)
                    .map(|provider| provider.id.clone()),
            );
        }
        ids
    }

    pub(super) fn provider_title<'a>(&self, id: &'a str) -> &'a str {
        match id {
            "local" => tr("shell-on-this-computer"),
            "chatgpt" | "chatgpt-login" => tr("shell-chatgpt"),
            _ => id,
        }
    }

    /// Adds the model file the user chose to this account's registry.
    pub(super) fn local_ai_add_model(&mut self, path: std::path::PathBuf) -> Task<ParentMessage> {
        if self.local_ai.model_busy() || !self.controls_live() {
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
            Some(Err(_)) => std::path::Path::new(local_ai::MODELS_TOOL).is_file(),
            None => false,
        }
    }

    pub(super) fn local_ai_route_shown(&self, route: &str) -> bool {
        let inference = self
            .local_ai
            .inference
            .as_ref()
            .and_then(|state| state.as_ref().ok());
        match route {
            "agents/local-ai-enable" | "agents/local-ai-import" | "agents/speech-import" => {
                self.model_import_offered()
            }
            "agents/default-text-model" => {
                self.local_ai.enabled && inference.is_some_and(|state| !state.models.is_empty())
            }
            "agents/add-ai-provider" => self.local_ai.gateway.as_ref().is_some_and(Result::is_ok),
            _ if route.starts_with("agents/runtime-") => {
                self.local_ai.enabled && inference.is_some()
            }
            _ => true,
        }
    }

    pub(super) fn reveal_local_ai_route(&mut self, route: &str) {
        if route.starts_with("agents/runtime-") && route != "agents/runtime-options" {
            self.local_ai.advanced = true;
        }
    }

    pub(super) fn read_local_ai_for_search(&mut self) -> Task<ParentMessage> {
        if self.local_ai.inference.is_some() || self.local_ai.reading {
            return Task::none();
        }
        if enzui_shell::settings_catalog::search(&self.filter)
            .iter()
            .any(|entry| !self.local_ai_route_shown(entry.route))
        {
            self.local_ai_read()
        } else {
            Task::none()
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
                let (inference, gateway, chatgpt, offers, downloads, enabled, writing) = *readings;
                state.reading = false;
                if state.inference.as_ref() == Some(&inference)
                    && state.gateway.as_ref() == Some(&gateway)
                    && state.chatgpt == chatgpt
                    && state.offers == offers
                    && state.downloads == downloads
                    && state.enabled == enabled
                    && state.writing == writing
                {
                    return iced::window::invalidate(iced::window::Invalidation::none());
                }
                state.writing = writing;
                state.downloads = downloads;
                state.enabled = enabled;
                state.offers = offers;
                state.inference = Some(inference);
                state.gateway = Some(gateway);
                state.chatgpt = chatgpt;
            }
            Message::WritingEnabled(_) | Message::WritingModel(_) => {
                if !live {
                    return Task::none();
                }
                let mut preferences = writing_tools::Preferences::load();
                match message {
                    Message::WritingEnabled(enabled) => preferences.enabled = enabled,
                    Message::WritingModel(model) => preferences.model = model,
                    _ => return Task::none(),
                }
                state.writing_error = preferences.save().err();
                if state.writing_error.is_none() {
                    state.writing = preferences;
                }
            }
            Message::RefreshStatus => {
                let task = if live && !state.reading && state.busy.is_none() {
                    state.reading = true;
                    read()
                } else {
                    Task::none()
                };
                return Task::batch([
                    task,
                    iced::window::invalidate(iced::window::Invalidation::none()),
                ]);
            }
            Message::Benchmark(model) => {
                if !live
                    || !state.enabled
                    || state.model_busy()
                    || !state.inference().is_some_and(|inference| {
                        inference.models.contains(&model)
                            || inference.speech_models.contains(&model)
                    })
                {
                    return Task::none();
                }
                state.benchmarks.remove(&model);
                let id = model.clone();
                state.benchmark_serial = state.benchmark_serial.wrapping_add(1);
                let serial = state.benchmark_serial;
                let (task, handle) =
                    Task::perform(local_ai::benchmark(model.clone()), move |result| {
                        send(Message::BenchmarkFinished(serial, model, result))
                    })
                    .abortable();
                state.benchmark = Some((id, handle.abort_on_drop()));
                return task;
            }
            Message::BenchmarkFinished(serial, id, result) => {
                if state.benchmark_serial == serial && state.benchmark.is_some() {
                    state.benchmark = None;
                    state.benchmarks.insert(id, result);
                }
            }
            Message::CancelBenchmark => {
                state.benchmark = None;
            }
            Message::ToggleAdvanced => state.advanced = !state.advanced,
            Message::SetEnabled(enabled) => {
                if state.model_busy() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Inference);
                state.notice = None;
                return blocking(move || local_ai::set_enabled(enabled), Message::Enabled);
            }
            Message::Enabled(result) => {
                state.busy = None;
                state.notice = result.err().map(|error| (Service::Inference, error));
                state.reading = true;
                return read();
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
                if service == Service::Inference && state.model_busy() {
                    return Task::none();
                }
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
                    || state.model_busy()
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
                if state.model_busy() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Inference);
                return tool_task(vec!["default".into(), id.into()], Message::ToolDone);
            }
            Message::Fetch(id) => {
                if state.model_busy() || !live {
                    return Task::none();
                }
                state.busy = Some(Service::Inference);
                state.notice = None;
                return blocking(
                    move || model_downloads::queue(&[id], false),
                    Message::Queued,
                );
            }
            Message::Queued(result) => {
                state.busy = None;
                state.notice = result.err().map(|error| (Service::Inference, error));
                state.reading = true;
                return read();
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

    fn inference_rows<'a>(
        &self,
        mut rows: iced::widget::Column<'a, ParentMessage>,
        inference: &Inference,
    ) -> iced::widget::Column<'a, ParentMessage> {
        let p = &inference.preferences;
        let live = inference.persistent && !self.local_ai.model_busy() && self.controls_live();
        let (title, mut details) = match &inference.loaded_model {
            Some(model) => (
                tr!("shell-model-loaded", "arg0" => self.model_title(model).to_owned()),
                Vec::new(),
            ),
            None => (
                tr("shell-model-standby").to_owned(),
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
        rows = rows.push(
            self.mark_setting(
                "agents/local-ai",
                crate::settings_app::settings_row(title)
                    .subtitle(details.join(" · "))
                    .subtitle_wrap(true)
                    .icon("cpu")
                    .into_element(),
            ),
        );
        if let Some(failure) = &inference.failure {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-the-last-request-failed"))
                    .subtitle(failure.clone())
                    .subtitle_wrap(true)
                    .icon("triangle-alert")
                    .into_element(),
            );
        }
        let choice = |route: &'static str,
                      title: &'static str,
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
            self.mark_setting(
                route,
                crate::settings_app::settings_row(title)
                    .subtitle(subtitle)
                    .control(control)
                    .into_element(),
            )
        };
        if !inference.models.is_empty() {
            let models = inference.models.clone();
            let current_model = models.iter().position(|m| *m == p.default_model);
            rows = rows.push(choice(
                "agents/default-text-model",
                tr("shell-model-default-text"),
                tr("shell-used-when-an-app-or-the-assistant-does-not-ask-for-one"),
                "local-ai-model",
                models
                    .iter()
                    .map(|model| self.model_title(model).to_owned())
                    .collect(),
                current_model,
                Box::new(move |index| models.get(index).cloned().map(Change::DefaultModel)),
            ));
        }
        rows = rows.push(
            self.mark_setting(
                "agents/runtime-options",
                crate::settings_app::settings_row(tr("shell-model-runtime-options"))
                    .icon("sliders-horizontal")
                    .control(
                        enz_button(
                            if self.local_ai.advanced {
                                tr("shell-hide")
                            } else {
                                tr("shell-show")
                            },
                            send(Message::ToggleAdvanced),
                        )
                        .id("local-ai-advanced".into())
                        .kind(ButtonKind::Secondary)
                        .into_element(),
                    )
                    .into_element(),
            ),
        );
        if !self.local_ai.advanced {
            return rows;
        }
        let threads: [Option<u8>; 3] = [None, Some(1), Some(2)];
        rows = rows.push(choice(
            "agents/runtime-threads",
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
            "agents/runtime-memory",
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
            "agents/runtime-idle",
            tr("shell-free-memory-when-idle"),
            tr("shell-the-model-unloads-after-this-long-without-a-request"),
            "local-ai-idle",
            idle.iter().map(|s| seconds(*s)).collect(),
            idle.iter().position(|s| *s == p.idle_seconds),
            Box::new(move |index| idle.get(index).copied().map(Change::IdleSeconds)),
        ));
        let context = presets(p.context, &[4096, 6144, 8192]);
        rows = rows.push(choice(
            "agents/runtime-context",
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
            "agents/runtime-engine",
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
        let control = enz_combo_box(
            "",
            selectable
                .iter()
                .map(|id| self.provider_title(id).to_owned())
                .collect::<Vec<_>>(),
            None,
            |_| ParentMessage::Noop,
        )
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
                "agents/default-ai-provider",
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
                    .id("local-ai-chatgpt-sign-out".into())
            } else {
                enz_button(tr("shell-sign-in"), send(Message::SignIn))
                    .id("local-ai-chatgpt-sign-in".into())
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
            if provider.kind == "local" {
                continue;
            }
            let mut row =
                crate::settings_app::settings_row(self.provider_title(&provider.id).to_owned())
                    .subtitle(state)
                    .icon(if provider.kind == "local" {
                        "cpu"
                    } else {
                        "cloud"
                    });
            row = match provider.kind.as_str() {
                "chatgpt-login" => row.control(account_button(chatgpt.signed_in)),
                _ => row.control(
                    enz_button(
                        tr("shell-remove"),
                        send(Message::Edit(ProviderEdit::Remove(provider.id.clone()))),
                    )
                    .id(format!("local-ai-provider-remove-{}", provider.id).into())
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
            .id("local-ai-provider-preset".into())
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
            .id("local-ai-provider-add".into())
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
        self.mark_setting("agents/add-ai-provider", form)
    }

    pub(super) fn model_title<'a>(&'a self, id: &'a str) -> &'a str {
        self.local_ai
            .offers
            .iter()
            .find(|offer| offer.id == id)
            .map(|offer| offer.title.as_str())
            .or_else(|| {
                self.local_ai
                    .inference()?
                    .model_titles
                    .get(id)
                    .map(String::as_str)
            })
            .unwrap_or(id)
    }

    fn benchmark_rows<'a>(
        &self,
        mut rows: iced::widget::Column<'a, ParentMessage>,
        models: &[String],
        speech: bool,
    ) -> iced::widget::Column<'a, ParentMessage> {
        let state = &self.local_ai;
        for model in models {
            let running = state.benchmark.as_ref().is_some_and(|(id, _)| id == model);
            let detail = if running {
                tr("shell-ai-benchmark-running").to_owned()
            } else {
                match state.benchmarks.get(model) {
                    Some(Ok(local_ai::Benchmark::Text {
                        first_ms,
                        tokens_per_second,
                        memory_mib,
                    })) => {
                        tr!("shell-ai-benchmark-text",
                            "first" => format!("{:.2}", first_ms / 1000.0),
                            "rate" => format!("{tokens_per_second:.1}"),
                            "memory" => format!("{memory_mib:.0}"))
                    }
                    Some(Ok(local_ai::Benchmark::Speech {
                        audio_seconds,
                        elapsed_seconds,
                        memory_mib,
                    })) => {
                        tr!("shell-ai-benchmark-speech",
                            "audio" => format!("{audio_seconds:.1}"),
                            "elapsed" => format!("{elapsed_seconds:.2}"),
                            "memory" => format!("{memory_mib:.0}"))
                    }
                    Some(Err(error)) => error.clone(),
                    None => tr("shell-model-installed").to_owned(),
                }
            };
            let (label, action) = if running {
                (tr("shell-cancel"), Message::CancelBenchmark)
            } else {
                (tr("shell-ai-benchmark"), Message::Benchmark(model.clone()))
            };
            rows = rows.push(
                crate::settings_app::settings_row(self.model_title(model).to_owned())
                    .subtitle(detail)
                    .subtitle_wrap(true)
                    .icon(if speech { "mic" } else { "cpu" })
                    .control(
                        enz_button(label, send(action))
                            .id(format!("local-ai-benchmark-{model}").into())
                            .kind(ButtonKind::Secondary)
                            .enabled(
                                self.controls_live()
                                    && (running || (state.enabled && !state.model_busy())),
                            )
                            .into_element(),
                    )
                    .into_element(),
            );
        }
        rows
    }

    fn available_model_rows<'a>(
        &self,
        mut rows: iced::widget::Column<'a, ParentMessage>,
        speech: bool,
    ) -> iced::widget::Column<'a, ParentMessage> {
        let state = &self.local_ai;
        let live = state.enabled && !state.model_busy() && self.controls_live();
        for offer in state
            .offers
            .iter()
            .filter(|offer| offer.speech == speech && (!offer.added || state.inference().is_none()))
        {
            let current = state
                .downloads
                .iter()
                .find(|download| download.model == offer.id);
            let status = current.map(|download| match &download.phase {
                model_downloads::Phase::Downloading { done, total } if *total > 0 =>
                    tr!("shell-model-downloading-percent", "arg0" => (done.saturating_mul(100) / total).min(100).to_string()),
                model_downloads::Phase::Downloading { .. } => tr("shell-model-downloading").to_owned(),
                model_downloads::Phase::Queued => tr("shell-model-download-queued").to_owned(),
                model_downloads::Phase::Verifying => tr("shell-model-verifying").to_owned(),
                model_downloads::Phase::Installed => tr("shell-model-installed").to_owned(),
                model_downloads::Phase::Failed(error) => error.clone(),
            });
            let mut row = crate::settings_app::settings_row(offer.title.clone())
                .subtitle(status.unwrap_or_else(|| {
                    let availability = if offer.added {
                        tr("shell-model-installed")
                    } else if offer.needs_runtime {
                        tr("shell-model-runtime-required")
                    } else {
                        offer.size.as_str()
                    };
                    format!("{} · {availability}", offer.description)
                }))
                .subtitle_wrap(true)
                .icon(if speech { "mic" } else { "cpu" });
            if !offer.added {
                let action = if matches!(
                    current.map(|download| &download.phase),
                    Some(model_downloads::Phase::Failed(_))
                ) {
                    tr("shell-retry")
                } else {
                    tr("shell-download")
                };
                row = row.control(
                    enz_button(action, send(Message::Fetch(offer.id.clone())))
                        .id(format!("local-ai-model-fetch-{}", offer.id).into())
                        .kind(ButtonKind::Secondary)
                        .enabled(live && !offer.needs_runtime)
                        .into_element(),
                );
            }
            rows = rows.push(row.into_element());
        }
        rows
    }

    fn writing_model_row(&self) -> Element<'_, ParentMessage> {
        use crate::settings_app::settings_row;
        let state = &self.local_ai;
        let mut models = vec![None];
        if let Some(Ok(inference)) = &state.inference {
            models.extend(inference.models.iter().cloned().map(Some));
        }
        if let Some(preferred) = &state.writing.model
            && !models.iter().any(|model| model.as_ref() == Some(preferred))
        {
            models.push(Some(preferred.clone()));
        }
        let selected = models
            .iter()
            .position(|model| *model == state.writing.model);
        let names = models
            .iter()
            .map(|model| {
                model
                    .as_deref()
                    .map_or_else(|| tr("writing-automatic"), |id| self.model_title(id))
                    .to_owned()
            })
            .collect();
        let model = enz_combo_box("", names, None, |_| ParentMessage::Noop)
            .aria_label(tr("writing-model"))
            .on_select_index(move |index| {
                models.get(index).map_or(ParentMessage::Noop, |id| {
                    send(Message::WritingModel(id.clone()))
                })
            })
            .selected_index(selected)
            .enabled(self.controls_live() && state.writing.enabled)
            .width(240)
            .id("writing-model".into())
            .into_element();
        self.mark_setting(
            "agents/writing-model",
            settings_row(tr("writing-model"))
                .control(model)
                .into_element(),
        )
    }

    pub(super) fn writing_tools_section(&self) -> Element<'_, ParentMessage> {
        use crate::settings_app::settings_row;
        let state = &self.local_ai;
        let mut rows = column![
            self.mark_setting(
                "agents/writing-tools",
                settings_row(tr("writing-enabled"))
                    .switch(state.writing.enabled, |enabled| send(
                        Message::WritingEnabled(enabled)
                    ))
                    .enabled(self.controls_live())
                    .into_element()
            ),
            self.mark_setting(
                "agents/writing-shortcut",
                settings_row(tr("writing-shortcut"))
                    .control(
                        enz_button(
                            tr("writing-shortcut-edit"),
                            ParentMessage::Section("keyboard/shortcut-writing-tools")
                        )
                        .kind(ButtonKind::Secondary)
                        .enabled(self.controls_live())
                        .id("writing-shortcut-edit".into())
                        .into_element()
                    )
                    .into_element(),
            ),
        ]
        .spacing(2);
        if let Some(error) = &state.writing_error {
            rows = rows.push(self.caption_row_text(error.clone()));
        }
        self.section_card_for("agents/writing-tools", rows)
            .title(tr("writing-tools"))
            .description(tr("writing-settings-description"))
            .into_element()
    }

    pub(super) fn voice_typing_section(&self) -> Element<'_, ParentMessage> {
        let row = crate::settings_app::settings_row(tr("shell-ai-dictation-shortcut"))
            .subtitle(tr("shell-model-voice-description"))
            .subtitle_wrap(true)
            .control(
                enz_button(
                    tr("shell-shortcuts"),
                    ParentMessage::Section("keyboard/shortcut-reference"),
                )
                .id("dictation-shortcuts".into())
                .kind(ButtonKind::Secondary)
                .enabled(self.controls_live())
                .into_element(),
            )
            .into_element();
        self.section_card_for(
            "agents/voice-typing",
            column![self.mark_setting("agents/voice-typing", row)],
        )
        .title(tr("shell-voice-typing"))
        .into_element()
    }

    pub(super) fn local_ai_sections<'a>(
        &'a self,
        body: iced::widget::Column<'a, ParentMessage>,
    ) -> iced::widget::Column<'a, ParentMessage> {
        let state = &self.local_ai;
        let mut on_device = column![].spacing(2);
        if self.model_import_offered() {
            on_device = on_device.push(
                self.mark_setting(
                    "agents/local-ai-enable",
                    crate::settings_app::settings_row(tr("shell-model-enable-local"))
                        .icon("cpu")
                        .subtitle(tr("shell-model-enable-local-description"))
                        .switch(state.enabled, |enabled| send(Message::SetEnabled(enabled)))
                        .enabled(!state.model_busy() && self.controls_live())
                        .into_element(),
                ),
            );
        }
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
            Some(Err(_)) if !state.enabled => {}
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
            Some(Ok(inference)) if state.enabled => {
                on_device = self.inference_rows(on_device, inference);
            }
            Some(Ok(_)) => {}
        }
        if self.model_import_offered() {
            let add = enzui_widgets::button::enz_button(
                tr("shell-model-browse"),
                ParentMessage::FileChooser(super::settings_file_chooser::Message::OpenModel),
            )
            .id("local-ai-model-import".into())
            .kind(enzui_widgets::button::ButtonKind::Secondary)
            .enabled(!state.model_busy() && self.controls_live())
            .into_element();
            let target: Element<'_, ParentMessage> =
                crate::settings_app::settings_row(tr("shell-model-import-file"))
                    .subtitle(tr("shell-model-import-file-description"))
                    .subtitle_wrap(true)
                    .icon("package")
                    .control(add)
                    .into_element();
            on_device = on_device.push(
                self.mark_setting(
                    "agents/local-ai-import",
                    enzui_widgets::file_drop::enz_file_drop_target_with_receipt(target, |drop| {
                        send(Message::Dropped(drop))
                    })
                    .enabled(
                        !state.model_busy() && state.native_drop.is_none() && self.controls_live(),
                    )
                    .copy_only(true)
                    .key("local-ai-model-import"),
                ),
            );
        }
        if let Some(inference) = state.inference() {
            on_device = self.benchmark_rows(on_device, &inference.models, false);
        }
        on_device = self.available_model_rows(on_device, false);

        let inference = state
            .inference
            .as_ref()
            .and_then(|result| result.as_ref().ok());
        let voice = inference.and_then(Inference::voice_model);
        let subtitle = match &state.inference {
            _ if !state.enabled => tr("shell-off").to_owned(),
            None => tr("shell-checking").to_owned(),
            Some(Err(_)) => tr("shell-not-available").to_owned(),
            Some(Ok(_)) => match voice {
                Some(model) => {
                    tr!("shell-voice-typing-uses", "arg0" => self.model_title(model).to_owned())
                }
                None => tr("shell-voice-typing-needs-a-speech-model").to_owned(),
            },
        };
        let mut row = crate::settings_app::settings_row(tr("shell-model-default-speech"))
            .subtitle(subtitle)
            .subtitle_wrap(true)
            .icon("mic");
        if let Some(inference) = inference
            && !inference.speech_models.is_empty()
        {
            let models = inference.speech_models.clone();
            let selected = models.iter().position(|m| Some(m.as_str()) == voice);
            row = row.control(
                enz_combo_box(
                    "",
                    models
                        .iter()
                        .map(|model| self.model_title(model).to_owned())
                        .collect::<Vec<_>>(),
                    None,
                    |_| ParentMessage::Noop,
                )
                .id("local-ai-speech-model".into())
                .aria_label(tr("shell-speech-model"))
                .on_select_index(move |index| {
                    models.get(index).map_or(ParentMessage::Noop, |id| {
                        send(Message::SpeechDefault(id.clone()))
                    })
                })
                .selected_index(selected)
                .enabled(state.enabled && !state.model_busy() && self.controls_live())
                .width(220)
                .into_element(),
            );
        }
        let mut speech =
            column![self.mark_setting("agents/default-speech-model", row.into_element())]
                .spacing(2);
        if let Some(inference) = inference {
            speech = self.benchmark_rows(speech, &inference.speech_models, true);
        }
        speech = self.available_model_rows(speech, true);
        if self.model_import_offered() {
            speech = speech.push(
                self.mark_setting(
                    "agents/speech-import",
                    crate::settings_app::settings_row(tr("shell-model-import-speech-folder"))
                        .subtitle(tr("shell-model-import-speech-folder-description"))
                        .subtitle_wrap(true)
                        .icon("folder-open")
                        .control(
                            enz_button(
                                tr("shell-model-browse"),
                                ParentMessage::FileChooser(
                                    super::settings_file_chooser::Message::OpenModelFolder,
                                ),
                            )
                            .id("local-ai-speech-import".into())
                            .kind(ButtonKind::Secondary)
                            .enabled(!state.model_busy() && self.controls_live())
                            .into_element(),
                        )
                        .into_element(),
                ),
            );
        }
        if let Some(notice) = self.local_ai_notice(Service::Inference) {
            on_device = on_device.push(notice);
        }
        let mut providers = column![].spacing(2);
        match &state.gateway {
            None => {
                providers = providers.push(
                    self.mark_setting(
                        "agents/default-ai-provider",
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
                        "agents/default-ai-provider",
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
        let mut defaults = self
            .assistant_provider_rows()
            .push(self.writing_model_row());
        if let Some(error) = &state.writing_error {
            defaults = defaults.push(self.caption_row_text(error.clone()));
        }
        body.push(
            self.section_card_for("agents/ai-providers", defaults)
                .title(tr("shell-ai-providers"))
                .description(tr("shell-ai-providers-description"))
                .into_element(),
        )
        .push(
            self.section_card_for("agents/local-ai", on_device)
                .title(tr("shell-model-local-text"))
                .description(tr("shell-model-local-text-description"))
                .into_element(),
        )
        .push(
            self.section_card_for("agents/speech-models", speech)
                .title(tr("shell-ai-speech-models"))
                .description(tr("shell-ai-speech-models-description"))
                .into_element(),
        )
        .push(
            self.section_card_for("agents/online-providers", providers)
                .title(tr("shell-ai-online-providers"))
                .description(tr("shell-a-request-that-fails-on-this-computer-is-never-sent-to-another-provider-on-i"))
                .into_element(),
        )
    }
}
