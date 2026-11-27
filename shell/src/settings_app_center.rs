//! Settings MCP Center requests through the GUI update path.
//! User changes complete after saving; system changes require confirmed backend readback.
//! Cancellable approval ends before mutation, while committed changes remain committed.

use super::{Message, SettingsApp, system_pages};
use enzui_agent::center::{
    AppFunction, AppFunctionRegistry, AppMetadata, CallRef, CenterCall, CenterEnded, CenterFailure,
    CenterHost, CenterProvision, CenterTurn, CommitReportDisposition, CommitReportError,
    CommitReportOutcome, CommitReported, CompletionDisposition, ConnectorEvent, ConsentScope,
    FunctionRisk, FunctionSpec,
};
use enzui_shell::authorization::{Approval, Authority, Change as ClockChange};
use enzui_shell::i18n::tr;
use enzui_shell::settings_agent::{self, Context, Entry, Scope, Target};
use enzui_shell::system_settings::{Panel, Reading, Request};
use enzui_shell::tr;
use iced::{Subscription, Task};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug)]
pub(super) enum AgentCall {
    Desktop {
        tool: &'static str,
        arguments: Value,
    },
    List {
        offset: usize,
        limit: usize,
        scope: Option<Scope>,
        category: Option<String>,
    },
    Get {
        key: String,
    },
    Set {
        key: String,
        value: Value,
        scope: Scope,
    },
}

impl AgentCall {
    fn needs_clock(&self) -> bool {
        match self {
            Self::Desktop { .. } => false,
            Self::List {
                scope, category, ..
            } => settings_agent::touches_system(*scope, category.as_deref()),
            Self::Get { key } => {
                settings_agent::find(key).is_some_and(|e| e.scope == Scope::System)
            }
            Self::Set { key, scope, .. } => {
                *scope == Scope::System
                    && settings_agent::find(key).is_some_and(|e| e.scope == Scope::System)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyArgs {
    key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    key: String,
    value: Value,
}

pub(super) fn registry() -> AppFunctionRegistry<AgentCall> {
    enzui_shell::i18n::canonical(|| {
        let [
            (list_in, list_out),
            (get_in, get_out),
            (set_in, set_out),
            (system_in, system_out),
        ] = settings_agent::schemas();
        let reads = vec![
            ConsentScope::Once,
            ConsentScope::Session,
            ConsentScope::Always,
        ];
        let list = FunctionSpec::new(
            "settings.list",
            tr("shell-list-settings"),
            "Lists the settings agents may read and change, with their current values, types, ranges and \
         whether they need an administrator. Paginated with offset and limit (at most 50).",
            list_in,
            list_out,
            FunctionRisk::ReadOnly,
            reads.clone(),
        );
        let get = FunctionSpec::new(
            "settings.get",
            tr("shell-read-a-setting"),
            tr("shell-reads-one-setting-by-key-including-all-of-its-choices"),
            get_in,
            get_out,
            FunctionRisk::ReadOnly,
            reads,
        );
        let set = FunctionSpec::new(
            "settings.set",
            tr("shell-change-a-desktop-preference"),
            "Changes one of this account's desktop preferences (appearance, bar, sidebar, windows, clock, \
         notifications) and returns the saved value.",
            set_in,
            set_out,
            FunctionRisk::Mutating,
            vec![ConsentScope::Once],
        );
        let system = FunctionSpec::new(
            "settings.set_system",
            tr("shell-change-a-computer-setting"),
            "Changes a setting of the whole computer (time zone, network time). An administrator must \
         also approve it on this computer; agent consent does not replace that.",
            system_in,
            system_out,
            FunctionRisk::Mutating,
            vec![ConsentScope::Once],
        );
        let mut functions = vec![
            AppFunction::typed(list, |args: ListArgs| AgentCall::List {
                offset: args.offset,
                limit: args.limit.unwrap_or(settings_agent::PAGE_LIMIT),
                scope: args.scope.as_deref().and_then(settings_agent::scope),
                category: args.category,
            })
            .expect("settings.list spec"),
            AppFunction::typed(get, |args: KeyArgs| AgentCall::Get { key: args.key })
                .expect("settings.get spec"),
            AppFunction::typed(set, |args: SetArgs| AgentCall::Set {
                key: args.key,
                value: args.value,
                scope: Scope::User,
            })
            .expect("settings.set spec"),
            AppFunction::typed(system, |args: SetArgs| AgentCall::Set {
                key: args.key,
                value: args.value,
                scope: Scope::System,
            })
            .expect("settings.set_system spec"),
        ];
        functions.extend(desktop_functions());
        AppFunctionRegistry::from_functions(functions).expect("the settings registry is valid")
    })
}

fn desktop_functions() -> Vec<AppFunction<AgentCall>> {
    let string = || json!({"type":"string"});
    let function = |id, tool, title, description, properties: Value, required, risk| {
        let input = json!({
            "type":"object", "additionalProperties":false,
            "properties":properties, "required":required,
        });
        let spec = FunctionSpec::new(
            id,
            title,
            description,
            input,
            json!({"type":"object"}),
            risk,
            vec![ConsentScope::Once],
        )
        .supports_cancellation(true);
        AppFunction::typed(spec, move |arguments: Value| AgentCall::Desktop {
            tool,
            arguments,
        })
        .expect("desktop function spec")
    };
    use FunctionRisk::{Mutating, ReadOnly};
    vec![
        function(
            "settings.search",
            "search_settings",
            "Search Settings",
            "Find Settings controls, values and allowed choices by short query. Search before changing a setting.",
            json!({"query":{"type":"string","maxLength":160},"offset":{"type":"integer","minimum":0}}),
            json!(["query"]),
            ReadOnly,
        ),
        function(
            "settings.read",
            "get_setting",
            "Read a setting",
            "Read a setting and its allowed values by id from search_settings.",
            json!({"id":string()}),
            json!(["id"]),
            ReadOnly,
        ),
        function(
            "settings.change",
            "set_setting",
            "Change a setting",
            "Change a setting by id from search_settings, using one of its allowed values. Requires the user's approval.",
            json!({"id":string(),"value":{"type":["string","boolean","number"]}}),
            json!(["id", "value"]),
            Mutating,
        ),
        function(
            "desktop.open",
            "desktop_open",
            "Open an app or folder",
            "Launch an installed app by name. To open a folder, use app Files and the folder name or absolute path.",
            json!({"app":string(),"folder":string()}),
            json!(["app"]),
            Mutating,
        ),
        function(
            "desktop.timer",
            "desktop_timer",
            "Start a timer",
            "Start a countdown that posts a desktop notification when it expires.",
            json!({"seconds":{"type":"integer","minimum":1,"maximum":86400}}),
            json!(["seconds"]),
            Mutating,
        ),
        function(
            "desktop.status",
            "device_status",
            "Read device status",
            "Read live battery, CPU (including busy processes), memory, or network information.",
            json!({"metric":{"type":"string","enum":["battery","cpu","memory","network"]}}),
            json!(["metric"]),
            ReadOnly,
        ),
        function(
            "desktop.downloads",
            "find_downloads",
            "Find downloads",
            "Find Downloads filenames modified today, yesterday, or on a YYYY-MM-DD date. Optional extension, e.g. pdf.",
            json!({"date":string(),"extension":string()}),
            json!(["date"]),
            ReadOnly,
        ),
    ]
}

pub(super) fn metadata() -> AppMetadata {
    enzui_shell::i18n::canonical(|| AppMetadata {
        id: settings_agent::APP_ID.into(),
        name: tr("shell-settings").into(),
        description: tr("shell-desktop-and-computer-settings").into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

#[derive(Default)]
pub(super) struct Agent {
    host: Option<CenterHost<AgentCall>>,
    desktop: Vec<(CenterCall, iced::task::Handle)>,
    // Calls that arrived before the settings file was first read.
    loading: Vec<(CenterCall, AgentCall)>,
    loaded: bool,
    // Calls that wait for a fresh date and time reading.
    clock_waiting: Vec<(CenterCall, AgentCall)>,
    clock_reading: bool,
    // User changes, answered once every queued write is saved.
    saving: Vec<(CenterCall, &'static Entry)>,
    // The system change being applied.
    applying: Option<(CenterCall, &'static Entry, Request)>,
    // Applied system changes, answered from the next reading.
    reporting: Vec<(CenterCall, &'static Entry)>,
    // The clock change waiting for an administrator, or being made.
    approving: Option<Approving>,
    approvals: u64,
    // Committed changes whose report the broker has not stored yet.
    commits: Vec<CommitReport>,
}

struct CommitReport {
    call: CallRef,
    attempts: u8,
    // Sent and waiting for the broker's answer; otherwise due for a retry.
    sent: bool,
}

const COMMIT_REPORT_ATTEMPTS: u8 = 3;

struct Approving {
    id: u64,
    call: CenterCall,
    entry: &'static Entry,
    change: ClockChange,
    // Dropping or sending withdraws the prompt.
    withdraw: Option<tokio::sync::oneshot::Sender<()>>,
    committing: bool,
}

fn clock_change(request: &Request) -> Option<ClockChange> {
    match request {
        Request::Timezone(zone) => Some(ClockChange::Timezone(zone.clone())),
        Request::Ntp(on) => Some(ClockChange::Ntp(*on)),
        _ => None,
    }
}

impl Agent {
    // Connects when this installation enrolled Settings with the MCP Center.
    pub(super) fn start() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let host = CenterProvision::discover(settings_agent::APP_ID).and_then(|provision| {
            CenterHost::spawn(provision, metadata(), registry())
                .map_err(|error| {
                    eprintln!("enzui-settings: MCP Center connection disabled: {error}");
                })
                .ok()
        });
        Self {
            host,
            ..Self::default()
        }
    }

    pub(super) fn host(&self) -> Option<&CenterHost<AgentCall>> {
        self.host.as_ref()
    }

    pub(super) fn host_mut(&mut self) -> Option<&mut CenterHost<AgentCall>> {
        self.host.as_mut()
    }

    // Agent work of Settings' own that must finish before a windowless
    // worker may leave: a first read, a save, a system change or a report.
    pub(super) fn working(&self) -> bool {
        !self.desktop.is_empty()
            || !self.loading.is_empty()
            || self.clock_reading
            || !self.clock_waiting.is_empty()
            || !self.saving.is_empty()
            || self.applying.is_some()
            || !self.reporting.is_empty()
            || self.approving.is_some()
            || self.commits.iter().any(|report| !report.sent)
    }

    #[cfg(test)]
    pub(super) fn offline() -> Self {
        Self {
            host: Some(CenterHost::offline(registry())),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn sent(&mut self) -> Vec<enzui_agent::center::InvocationEvent> {
        self.host
            .as_mut()
            .map(CenterHost::take_sent)
            .unwrap_or_default()
    }

    pub(super) fn subscription(&self) -> Subscription<Message> {
        let events = self.host.as_ref().map_or_else(Subscription::none, |host| {
            host.subscription(Message::Center)
        });
        // Deadlines pass without a connector event; so do report retries.
        let retrying = self.commits.iter().any(|report| !report.sent);
        let deadline = if retrying
            || self
                .approving
                .as_ref()
                .is_some_and(|approving| !approving.committing)
        {
            iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::AgentDeadline)
        } else {
            Subscription::none()
        };
        Subscription::batch([events, deadline])
    }

    // Final shutdown: refuse new calls, cancel live ones, join the connector.
    pub(super) fn shutdown(&mut self) {
        for (_, task) in self.desktop.drain(..) {
            task.abort();
        }
        // A prompt still open is withdrawn.
        if let Some(withdraw) = self
            .approving
            .as_mut()
            .and_then(|approving| approving.withdraw.take())
        {
            let _ = withdraw.send(());
        }
        if let Some(host) = self.host.take() {
            host.shutdown();
        }
    }

    fn answer(&mut self, call: CenterCall, outcome: Result<Value, CenterFailure>) {
        if let Some(host) = self.host.as_mut() {
            host.answer(call, outcome);
        }
    }

    // A change that was made: answered if the call is still live, and
    // reported to the broker as committed either way.
    fn committed(&mut self, call: CenterCall, key: &str, value: Value) {
        let Some(host) = self.host.as_mut() else {
            return;
        };
        let receipt = host.complete_with_receipt(call, json!({"key": key, "value": value}));
        if receipt.disposition != CompletionDisposition::Queued {
            eprintln!(
                "enzui-settings: {key} was changed for invocation {}, which could not be answered ({:?})",
                receipt.call.invocation_id(),
                receipt.disposition
            );
        }
        self.commits.push(CommitReport {
            call: receipt.call,
            attempts: 0,
            sent: false,
        });
        self.send_reports();
    }

    fn send_reports(&mut self) {
        let Some(host) = self.host.as_mut() else {
            return;
        };
        self.commits.retain_mut(|report| {
            if report.sent {
                return true;
            }
            report.attempts += 1;
            match host.report_commit(&report.call) {
                Ok(()) => {
                    report.sent = true;
                    true
                }
                Err(CommitReportError::RateLimited | CommitReportError::QueueFull)
                    if report.attempts < COMMIT_REPORT_ATTEMPTS =>
                {
                    true
                }
                // A version-2 broker or no connection: nothing can take it.
                Err(CommitReportError::Unavailable) => false,
                Err(error) => {
                    eprintln!(
                        "enzui-settings: the commit of invocation {} was not reported ({error:?})",
                        report.call.invocation_id()
                    );
                    false
                }
            }
        });
    }

    fn reported(&mut self, reported: Vec<CommitReported>) {
        for answer in reported {
            let Some(index) = self
                .commits
                .iter()
                .position(|report| report.sent && report.call == answer.call)
            else {
                continue;
            };
            let retry = match answer.outcome {
                CommitReportOutcome::Broker(disposition) if disposition.is_durable() => false,
                CommitReportOutcome::Broker(CommitReportDisposition::StorageUnavailable)
                    if self.commits[index].attempts < COMMIT_REPORT_ATTEMPTS =>
                {
                    true
                }
                outcome => {
                    eprintln!(
                        "enzui-settings: the broker did not store the commit of invocation {} ({outcome:?})",
                        answer.call.invocation_id()
                    );
                    false
                }
            };
            if retry {
                self.commits[index].sent = false;
            } else {
                self.commits.remove(index);
            }
        }
    }

    fn live(&self, call: &CenterCall) -> bool {
        self.host.as_ref().is_some_and(|host| host.is_live(call))
    }

    // Withdraws the prompt of an approval whose call ended.
    pub(super) fn ended(&mut self, ended: &[CenterEnded]) {
        self.desktop.retain(|(call, task)| {
            if ended.iter().any(|end| end.call == call.reference()) {
                task.abort();
                false
            } else {
                true
            }
        });
        if let Some(approving) = self.approving.as_mut()
            && ended
                .iter()
                .any(|end| end.call == approving.call.reference())
            && let Some(withdraw) = approving.withdraw.take()
        {
            let _ = withdraw.send(());
        }
    }
}

fn refusal(refusal: settings_agent::Refusal) -> CenterFailure {
    CenterFailure::new(refusal.code, refusal.message)
}

impl SettingsApp {
    fn agent_context<'a>(
        &self,
        clock: Option<&'a enzui_shell::system_settings::Clock>,
    ) -> Context<'a> {
        Context {
            clock,
            islands_supported: self.islands_supported,
            system_writes: self.system_writes_allowed(),
        }
    }

    fn agent_clock(&self) -> Option<enzui_shell::system_settings::Clock> {
        match self.reading(Panel::DateTime) {
            Some(Reading::DateTime(clock)) => Some(clock),
            _ => None,
        }
    }

    pub(super) fn center_event(&mut self, event: ConnectorEvent) -> Task<Message> {
        let Some(host) = self.agent.host.as_mut() else {
            return Task::none();
        };
        let update = host.receive_with_ends(event);
        self.agent.ended(&update.ended);
        self.agent.reported(update.reported);
        let CenterTurn::Call(call, request) = update.turn else {
            return Task::none();
        };
        if self.closing {
            self.agent.answer(
                call,
                Err(CenterFailure::new(
                    "unavailable",
                    tr("shell-settings-is-closing"),
                )),
            );
            return Task::none();
        }
        // Answers built from defaults would be wrong; a cold start waits.
        if !self.agent.loaded {
            self.agent.loading.push((call, request));
            return Task::none();
        }
        self.dispatch(call, request)
    }

    fn dispatch(&mut self, call: CenterCall, request: AgentCall) -> Task<Message> {
        if request.needs_clock() {
            self.agent.clock_waiting.push((call, request));
            return self.read_agent_clock();
        }
        self.serve(call, request)
    }

    // The settings file was read: calls that waited for it are served, or
    // refused when the first read failed.
    pub(super) fn agent_settings_read(&mut self, ok: bool) -> Task<Message> {
        if self.agent.loaded {
            return Task::none();
        }
        let waiting = std::mem::take(&mut self.agent.loading);
        if !ok {
            for (call, _) in waiting {
                self.agent.answer(
                    call,
                    Err(CenterFailure::new(
                        "unavailable",
                        tr("shell-settings-could-not-read-its-settings-file"),
                    )),
                );
            }
            return Task::none();
        }
        self.agent.loaded = true;
        Task::batch(
            waiting
                .into_iter()
                .map(|(call, request)| self.dispatch(call, request)),
        )
    }

    fn read_agent_clock(&mut self) -> Task<Message> {
        if std::mem::replace(&mut self.agent.clock_reading, true) {
            return Task::none();
        }
        let backend = self.system_backend();
        Task::perform(
            async move { backend.read(Panel::DateTime).await },
            |reading| Message::AgentClock(Box::new(reading)),
        )
    }

    pub(super) fn agent_clock_read(&mut self, reading: Reading) -> Task<Message> {
        self.agent.clock_reading = false;
        self.store_system_reading(reading);
        let clock = self.agent_clock();
        for (call, entry) in std::mem::take(&mut self.agent.reporting) {
            let value = entry.value(&self.settings, &self.agent_context(clock.as_ref()));
            self.agent.committed(call, entry.id, json!(value));
        }
        let waiting = std::mem::take(&mut self.agent.clock_waiting);
        Task::batch(
            waiting
                .into_iter()
                .map(|(call, request)| self.serve(call, request)),
        )
    }

    pub(super) fn desktop_tool_result(
        &mut self,
        reference: CallRef,
        result: Result<Value, &'static str>,
    ) {
        if let Some(index) = self
            .agent
            .desktop
            .iter()
            .position(|(call, _)| call.reference() == reference)
        {
            let (call, _) = self.agent.desktop.swap_remove(index);
            self.agent.answer(
                call,
                result.map_err(|code| CenterFailure::new(code, "Desktop tool failed")),
            );
        }
    }

    fn serve(&mut self, call: CenterCall, request: AgentCall) -> Task<Message> {
        if !self.agent.live(&call) {
            return Task::none();
        }
        // A windowless worker watches no files: it answers what is on disk now,
        // not what it read when it started (the shell may have changed dark
        // mode since). A change of its own still being written is kept.
        if self.window.is_none()
            && matches!(request, AgentCall::List { .. } | AgentCall::Get { .. })
            && self.agent.saving.is_empty()
            && let Ok(fresh) = self.store.load()
        {
            self.settings = fresh;
        }
        let clock = self.agent_clock();
        let context = self.agent_context(clock.as_ref());
        match request {
            AgentCall::Desktop { tool, arguments } => {
                let reference = call.reference();
                let (task, handle) = Task::future(async move {
                    let result = enzui_shell::desktop_control::settings_tool(tool, arguments).await;
                    Message::DesktopToolResult(reference, result)
                })
                .abortable();
                self.agent.desktop.push((call, handle));
                task
            }
            AgentCall::List {
                offset,
                limit,
                scope,
                category,
            } => {
                let listing = settings_agent::list(
                    &self.settings,
                    &context,
                    offset,
                    limit,
                    scope,
                    category.as_deref(),
                );
                self.agent.answer(call, Ok(listing));
                Task::none()
            }
            AgentCall::Get { key } => {
                let outcome = settings_agent::find(&key)
                    .map(|entry| entry.describe(&self.settings, &context, true))
                    .ok_or_else(|| {
                        CenterFailure::new("unknown_setting", tr("shell-no-setting-has-that-key"))
                    });
                self.agent.answer(call, outcome);
                Task::none()
            }
            AgentCall::Set { key, value, scope } => {
                let Some(entry) = settings_agent::find(&key) else {
                    self.agent.answer(
                        call,
                        Err(CenterFailure::new(
                            "unknown_setting",
                            tr("shell-no-setting-has-that-key"),
                        )),
                    );
                    return Task::none();
                };
                if entry.scope != scope {
                    let function = if entry.scope == Scope::System {
                        "settings.set_system"
                    } else {
                        "settings.set"
                    };
                    self.agent.answer(call, Err(CenterFailure::new("wrong_function", tr!("shell-change-with", "arg0" => key.clone(), "arg1" => function.to_owned()))));
                    return Task::none();
                }
                match entry.target(&value, &context) {
                    Err(error) => {
                        self.agent.answer(call, Err(refusal(error)));
                        Task::none()
                    }
                    Ok(Target::User(change)) => self.agent_change(call, entry, change),
                    Ok(Target::System(request)) => self.agent_request(call, entry, request),
                }
            }
        }
    }

    fn agent_change(
        &mut self,
        call: CenterCall,
        entry: &'static Entry,
        change: enzui_shell::settings::Change,
    ) -> Task<Message> {
        // The same validation the controls get, before anything is queued.
        if let Err(error) = self.settings.clone().preview(change.clone()) {
            self.agent
                .answer(call, Err(CenterFailure::new("invalid_value", error)));
            return Task::none();
        }
        let task = self.update(Message::Change(change));
        if self.busy {
            self.agent.saving.push((call, entry));
        } else {
            self.agent.answer(
                call,
                Err(CenterFailure::new(
                    "unavailable",
                    tr("shell-settings-could-not-save-this-now"),
                )),
            );
        }
        task
    }

    fn agent_request(
        &mut self,
        call: CenterCall,
        entry: &'static Entry,
        request: Request,
    ) -> Task<Message> {
        if self.agent.applying.is_some() || self.agent.approving.is_some() || !self.system_idle() {
            self.agent.answer(
                call,
                Err(CenterFailure::new(
                    "busy",
                    tr("shell-another-computer-setting-is-being-changed-try-again"),
                )),
            );
            return Task::none();
        }
        if let Some(change) = clock_change(&request).filter(|_| !self.system_backend().demo) {
            return self.approve(call, entry, change);
        }
        let task = self.system_message(system_pages::Message::Request(request.clone()));
        if self.current_system_request() == Some(&request) {
            self.agent.applying = Some((call, entry, request));
        } else {
            self.agent.answer(
                call,
                Err(CenterFailure::new(
                    "unavailable",
                    tr("shell-settings-could-not-apply-this-now"),
                )),
            );
        }
        task
    }

    fn approve(
        &mut self,
        call: CenterCall,
        entry: &'static Entry,
        change: ClockChange,
    ) -> Task<Message> {
        self.agent.approvals += 1;
        let id = self.agent.approvals;
        let (withdraw, withdrawn) = tokio::sync::oneshot::channel::<()>();
        let action = change.action();
        let cancellation = format!("enzui-settings-{}-{id}", std::process::id());
        self.agent.approving = Some(Approving {
            id,
            call,
            entry,
            change,
            withdraw: Some(withdraw),
            committing: false,
        });
        Task::perform(
            async move {
                let authority = match Authority::connect().await {
                    Ok(authority) => authority,
                    Err(error) => return (Err(error), None),
                };
                // A dropped sender (Settings closing) withdraws as well.
                let approval = authority
                    .approve(action, &cancellation, async move {
                        let _ = withdrawn.await;
                    })
                    .await;
                (approval, Some(authority))
            },
            move |(result, authority)| Message::AgentApproved(id, result, authority),
        )
    }

    pub(super) fn agent_approved(
        &mut self,
        id: u64,
        result: Result<Approval, String>,
        authority: Option<Authority>,
    ) -> Task<Message> {
        let Some(approving) = self.agent.approving.take_if(|approving| approving.id == id) else {
            return Task::none();
        };
        let live = self.agent.live(&approving.call);
        let failure = match (result, authority) {
            // Asked once more right before the change: an ended call changes nothing.
            (Ok(Approval::Approved), Some(authority)) if live && !self.closing => {
                let change = approving.change.clone();
                self.agent.approving = Some(Approving {
                    committing: true,
                    withdraw: None,
                    ..approving
                });
                return Task::perform(
                    async move { authority.commit(&change).await },
                    move |result| Message::AgentCommitted(id, result),
                );
            }
            _ if !live => return Task::none(),
            (Ok(Approval::Approved), _) => CenterFailure::new(
                "unavailable",
                tr("shell-settings-could-not-make-this-change-now"),
            ),
            (Ok(Approval::Refused), _) => CenterFailure::new(
                "not_authorized",
                tr("shell-an-administrator-did-not-approve-this-change"),
            ),
            (Ok(Approval::Withdrawn), _) => CenterFailure::new(
                "cancelled",
                tr("shell-the-approval-was-withdrawn-nothing-was-changed"),
            ),
            (Err(error), _) => CenterFailure::new("failed", error),
        };
        self.agent.answer(approving.call, Err(failure));
        Task::none()
    }

    pub(super) fn agent_committed(
        &mut self,
        id: u64,
        result: Result<(), (&'static str, String)>,
    ) -> Task<Message> {
        let Some(approving) = self.agent.approving.take_if(|approving| approving.id == id) else {
            return Task::none();
        };
        match result {
            Ok(()) => {
                self.agent.reporting.push((approving.call, approving.entry));
                self.read_agent_clock()
            }
            Err((code, message)) => {
                self.agent
                    .answer(approving.call, Err(CenterFailure::new(code, message)));
                Task::none()
            }
        }
    }

    pub(super) fn agent_deadline(&mut self) -> Task<Message> {
        if let Some(host) = self.agent.host.as_mut() {
            let ended = host.poll_ended();
            self.agent.ended(&ended);
        }
        self.agent.send_reports();
        Task::none()
    }

    // Before the system pages take a finished request: answers the agent that
    // asked for it.
    pub(super) fn agent_applied(&mut self, result: &Result<(), String>) -> Task<Message> {
        let Some((call, entry, request)) = self.agent.applying.take() else {
            return Task::none();
        };
        if self.current_system_request() != Some(&request) {
            self.agent.applying = Some((call, entry, request));
            return Task::none();
        }
        match result {
            Ok(()) => {
                self.agent.reporting.push((call, entry));
                self.read_agent_clock()
            }
            Err(error) => {
                let lowered = error.to_lowercase();
                let code = if ["authoriz", "authentic", "permission", "denied"]
                    .iter()
                    .any(|word| lowered.contains(word))
                {
                    "not_authorized"
                } else {
                    "failed"
                };
                self.agent
                    .answer(call, Err(CenterFailure::new(code, error.clone())));
                Task::none()
            }
        }
    }

    // After a save: once nothing is left to write, answers every waiting
    // change with its saved value, or with the save's failure.
    pub(super) fn agent_saved(&mut self, failure: Option<String>) {
        if failure.is_none() && self.busy {
            return;
        }
        let clock = self.agent_clock();
        for (call, entry) in std::mem::take(&mut self.agent.saving) {
            match &failure {
                Some(error) => self
                    .agent
                    .answer(call, Err(CenterFailure::new("save_failed", error.clone()))),
                None => {
                    let value = entry.value(&self.settings, &self.agent_context(clock.as_ref()));
                    self.agent.committed(call, entry.id, json!(value));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "settings_app_center_tests.rs"]
mod tests;
