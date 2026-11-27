//! Single-instance Settings runtime with a windowless agent mode.
//! Desktop launches wake the existing model instead of creating another broker connection.

use super::{Message, SettingsApp, settings_app_center};
use enzui_agent::center::background::Disposition;
use enzui_agent::center::installed::{ActivationSource, BackgroundDeclaration, UnitSandbox};
use enzui_agent::center::task_host::{TaskHost, TaskHostError};
use enzui_agent_app::{
    AgentProcess, ApplicationIntent, BackgroundApp, BackgroundExit, DEFAULT_IDLE_EXIT,
    LifecycleCallers, SearchProvider, SearchRequest, SearchResult,
};
use enzui_core::activation::{ActivationContext, dbus::APPLICATION_CALL_CAPACITY};
use enzui_shell::i18n::tr;
use enzui_shell::settings::Store;
use enzui_shell::settings_agent::APP_ID;
use enzui_shell::settings_catalog::{self, Category as Page, SearchDestination};
use enzui_shell::tr;
use iced::{Subscription, Task};
use std::sync::Arc;
use std::time::Instant;

// The exported executable the generated unit and D-Bus service run.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) const INSTALLED_EXECUTABLE: &str = "/usr/lib/enzui/bin/enzui-settings";
const URI_SCHEME: &str = "enzui-settings:";

// Settings' window needs the person's ordinary desktop rights (wallpapers,
// autostart entries, plugins, recovery files), so the unit is a desktop unit.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) fn unit_sandbox() -> UnitSandbox {
    UnitSandbox {
        network: true,
        writable_home: true,
        own_directories: None,
        writable: Vec::new(),
        desktop: true,
    }
}

// Append the OS search-activation source absent from the framework enum.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) fn background_json() -> Result<Vec<u8>, String> {
    let bytes = enzui_agent::center::installed::background_declaration(
        APP_ID,
        INSTALLED_EXECUTABLE,
        &background_declaration(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let sources = value["activation_sources"]
        .as_array_mut()
        .ok_or_else(|| tr("shell-no-activation-sources"))?;
    sources.push(serde_json::Value::from("search"));
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) fn desktop_entry() -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Settings\nName[es]=Ajustes\nComment=Desktop and appearance\nComment[es]=Escritorio y apariencia\nIcon=dev.enzui.Settings\n\
         Exec={INSTALLED_EXECUTABLE}\nDBusActivatable=true\nCategories=Settings;System;\nOnlyShowIn=enzUI;\n"
    )
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) fn unit() -> Result<String, String> {
    enzui_agent::center::installed::unit(APP_ID, INSTALLED_EXECUTABLE, &unit_sandbox())
        .map_err(|error| error.to_string())
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Windowless lifecycle declarations are exercised by the app-scoped tests."
    )
)]
pub(super) fn background_declaration() -> BackgroundDeclaration {
    enzui_shell::i18n::canonical(|| BackgroundDeclaration {
        title: tr("shell-settings").into(),
        purpose: tr(
            "shell-answers-approved-agent-requests-about-desktop-and-computer-settings-without",
        )
        .into(),
        activation_sources: vec![ActivationSource::User, ActivationSource::Agent],
        autostart_supported: false,
        graceful_stop: true,
        work: false,
    })
}

// What a launch asks for, forwarded to a running instance as an `Open` URI.
pub(super) fn launch_intent(page: Option<&str>, plugin: Option<&str>) -> ApplicationIntent {
    match page.filter(|route| settings_catalog::valid_route(route)) {
        Some(route) => {
            let mut uri = match plugin.filter(|id| plugin_id(id)) {
                Some(id) => format!("{URI_SCHEME}{route}?plugin={id}"),
                None => format!("{URI_SCHEME}{route}"),
            };
            if route.starts_with("backups/")
                && let Some(id) = launch_share()
            {
                let separator = if uri.contains('?') { '&' } else { '?' };
                uri = format!("{uri}{separator}share={id}");
            }
            ApplicationIntent::Open(vec![uri])
        }
        None => ApplicationIntent::Activate,
    }
}

/// `ENZUI_SETTINGS_SHARE`: a Files network location to set up for backups.
pub(super) fn launch_share() -> Option<String> {
    std::env::var("ENZUI_SETTINGS_SHARE")
        .ok()
        .filter(|id| share_id(id))
}

fn share_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The Files location a forwarded `Open` URI of a backups page asks to set
/// up for backups.
pub(super) fn opened_share(uri: &str) -> Option<String> {
    let backups = match opened(uri)?.0 {
        Target::Entry(route) | Target::Section(route) => {
            settings_catalog::find(route)?.category() == Page::Backups
        }
        Target::Page(page) => page == Page::Backups,
    };
    let query = uri.strip_prefix(URI_SCHEME)?.split_once('?')?.1;
    backups
        .then_some(query)?
        .split('&')
        .find_map(|pair| pair.strip_prefix("share="))
        .filter(|id| share_id(id))
        .map(str::to_owned)
}

fn plugin_id(id: &str) -> bool {
    !id.is_empty() && enzui_shell::bar_layout::Item::new(format!("plugin:{id}/entry"), true).is_ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Target {
    Entry(&'static str),
    Section(&'static str),
    Page(Page),
}

// A forwarded `Open` URI: a catalog entry or a whole page, and a plugin to select.
pub(super) fn opened(uri: &str) -> Option<(Target, Option<String>)> {
    let rest = uri.strip_prefix(URI_SCHEME)?;
    let (route, query) = rest.split_once('?').unwrap_or((rest, ""));
    let plugin = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("plugin="));
    let target = match settings_catalog::find(route) {
        Some(entry) => Target::Entry(entry.route),
        None => match SearchDestination::from_id(route)? {
            SearchDestination::Page(page) => Target::Page(page),
            SearchDestination::Section(route) => Target::Section(route),
        },
    };
    Some((target, plugin.filter(|id| plugin_id(id)).map(str::to_owned)))
}

// Settings' pages for the desktop's Search (dev.enzui.SearchProvider1): the
// same catalogue the Settings search field uses, never bringing up UI.
fn search_provider() -> SearchProvider {
    SearchProvider::blocking(|query| {
        let platform = enzui_shell::platform::latest();
        settings_catalog::search_groups(&query.text, |page, entry| {
            enzui_shell::platform::shows(platform.as_ref(), page.page())
                && entry.is_none_or(|entry| {
                    enzui_shell::platform::shows(platform.as_ref(), entry.route)
                })
        })
        .into_iter()
        .take(query.limit.min(settings_catalog::DESKTOP_GROUP_LIMIT))
        .filter_map(|group| {
            let result = SearchResult::new(group.destination.id(), group.title)
                .ok()?
                .icon(group.category.icon());
            Some(match group.destination {
                SearchDestination::Page(_) => result,
                SearchDestination::Section(_) => result.subtitle(group.category.title()),
            })
        })
        .collect()
    })
}

impl SettingsApp {
    pub(super) fn serve_desktop(&mut self) {
        if let Err(error) = self.process.serve_search(search_provider()) {
            eprintln!("enzui-settings: search provider not served: {error}");
        }
        if let Err(error) = self.process.serve_lifecycle(LifecycleCallers::installed()) {
            eprintln!("enzui-settings: graceful stop not served: {error}");
        }
        // Stopping interrupts nothing a person started: writes are short and
        // an agent call in flight keeps the process until it ends.
        self.process.publish_stop_consequences(Vec::new());
    }

    pub(super) fn application_wake(&mut self) -> Task<Message> {
        if self.process.take_stop().is_some() {
            self.stop_requested = true;
            if self.window.is_some() {
                return self.update(Message::Close);
            }
        }
        let mut tasks = Vec::new();
        for activation in self.process.take_searches() {
            tasks.push(match activation.request {
                SearchRequest::Open { id, .. } => match SearchDestination::from_id(&id) {
                    Some(SearchDestination::Page(page)) => self.update(Message::Page(page)),
                    Some(SearchDestination::Section(route)) => {
                        self.update(Message::RevealSection(route))
                    }
                    None => settings_catalog::find(&id).map_or_else(Task::none, |entry| {
                        self.update(Message::Reveal(entry.route))
                    }),
                },
                SearchRequest::Search { query } => self.update(Message::Filter(query)),
            });
            tasks.push(self.activate_launch(activation.context));
        }
        // The queued call wakes the windowless runtime without consuming its activation context.
        if !self.process.is_windowless() {
            for call in self.process.take_application_calls() {
                tasks.push(self.open_intent(call.intent));
                tasks.push(self.activate_launch(call.context));
            }
        }
        Task::batch(tasks)
    }

    fn activate_launch(&mut self, context: Option<ActivationContext>) -> Task<Message> {
        let Some(context) = context.filter(|_| !self.closing) else {
            return Task::none();
        };
        if let Some(window) = self.window {
            return context.activate(window, Instant::now()).discard();
        }
        let now = Instant::now();
        self.pending_activation
            .retain(|token| !token.is_expired(now));
        if self.pending_activation.len() < APPLICATION_CALL_CAPACITY {
            self.pending_activation.push(context);
        }
        Task::none()
    }

    pub(super) fn activate_pending_launches(&mut self, window: iced::window::Id) -> Task<Message> {
        let pending = std::mem::take(&mut self.pending_activation);
        if self.closing {
            return Task::none();
        }
        let now = Instant::now();
        Task::batch(
            pending
                .into_iter()
                .map(|token| token.activate(window, now).discard()),
        )
    }

    fn open_intent(&mut self, intent: ApplicationIntent) -> Task<Message> {
        let ApplicationIntent::Open(uris) = intent else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        for uri in &uris {
            let Some((target, plugin)) = opened(uri) else {
                continue;
            };
            if plugin.is_some() {
                self.plugins.select_from_launch(plugin);
            }
            tasks.push(match target {
                Target::Entry(route) => self.update(Message::Reveal(route)),
                Target::Section(route) => self.update(Message::RevealSection(route)),
                Target::Page(page) => self.update(Message::Page(page)),
            });
            if let Some(id) = opened_share(uri) {
                tasks.push(self.update(super::system_pages::backups::share_for(id)));
            }
        }
        Task::batch(tasks)
    }

    // The window opens over state a windowless worker built: run the work it
    // deferred, read what only the window shows, and open the requested page.
    pub(super) fn present(&mut self) -> Task<Message> {
        let (intent, deferred) = self.process.begin_presenting();
        let open = intent.map_or_else(Task::none, |intent| self.open_intent(intent));
        Task::batch([deferred, self.window_boot(), open, self.application_wake()])
    }
}

// SettingsApp as the windowless runtime drives it.
struct Background(SettingsApp);

impl std::fmt::Debug for Background {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Background")
    }
}

impl BackgroundApp for Background {
    type Message = Message;

    fn update(&mut self, message: Message) -> Task<Message> {
        self.0.update(message)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([self.0.agent.subscription(), self.0.process_subscription()])
    }

    fn disposition(&self) -> Disposition {
        let app = &self.0;
        let busy = app.busy || app.agent.working();
        if app.stop_requested && !busy {
            return Disposition::Exit;
        }
        if app.agent.host().is_none() && busy {
            return Disposition::Working;
        }
        enzui_agent_app::disposition(app.agent.host(), &app.process, busy)
    }

    fn wake_at(&self) -> Option<Instant> {
        self.0
            .agent
            .host()
            .and_then(enzui_agent_app::deadline_instant)
    }

    fn wake(&mut self) -> Task<Message> {
        if let Some(host) = self.0.agent.host_mut() {
            let ended = host.poll_ended();
            self.0.agent.ended(&ended);
        }
        Task::none()
    }

    fn refused(&mut self, error: TaskHostError) -> Task<Message> {
        self.0
            .update(Message::BackgroundTaskRefused(error.to_string()))
    }
}

pub(super) fn run_background() -> Result<Option<SettingsApp>, String> {
    let Some(service) = enzui_agent_app::claim_background(APP_ID)? else {
        return Ok(None);
    };
    let store = Store::from_environment()
        .ok_or_else(|| tr("shell-no-shell-settings-directory-is-configured"))?;
    let tasks =
        Arc::new(TaskHost::spawn().map_err(
            |error| tr!("shell-task-host-did-not-start", "arg0" => format!("{:}", error)),
        )?);
    let mut app = SettingsApp::model(store);
    // Search answers list only the pages this computer can show.
    let boot = Task::batch([app.read(), app.detect_platform()]);
    app.process = AgentProcess::windowless(Some(service), Arc::clone(&tasks));
    app.serve_desktop();
    match enzui_agent_app::run_background(Background(app), boot, &tasks, DEFAULT_IDLE_EXIT)
        .map_err(|error| error.to_string())?
    {
        BackgroundExit::Present(Background(app)) => Ok(Some(app)),
        BackgroundExit::Idle(Background(mut app)) | BackgroundExit::Exit(Background(mut app)) => {
            app.agent.shutdown();
            Ok(None)
        }
    }
}

// Prints the package catalog for `--export-agent-manifest`.
pub(super) fn export_manifest() -> std::process::ExitCode {
    enzui_agent_app::print_manifest(
        &settings_app_center::registry(),
        settings_app_center::metadata(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_background_behaviour() {
        // Launches round trip as open intents.
        {
            assert_eq!(launch_intent(None, None), ApplicationIntent::Activate);
            assert_eq!(
                launch_intent(Some("no-such-page"), None),
                ApplicationIntent::Activate
            );
            let ApplicationIntent::Open(uris) = launch_intent(Some("agents/assistant"), None)
            else {
                panic!("an entry opens")
            };
            assert_eq!(
                opened(&uris[0]),
                Some((Target::Entry("agents/assistant"), None))
            );
            let ApplicationIntent::Open(uris) = launch_intent(Some("appearance"), None) else {
                panic!("a page opens")
            };
            assert_eq!(
                opened(&uris[0]).map(|(target, _)| target),
                Some(Target::Page(Page::Appearance))
            );
            let ApplicationIntent::Open(uris) = launch_intent(Some("section/dock/enabled"), None)
            else {
                panic!("a section opens")
            };
            assert_eq!(
                opened(&uris[0]),
                Some((Target::Section("dock/enabled"), None))
            );
            assert_eq!(opened("enzui-settings:section/dock/style"), None);
            let ApplicationIntent::Open(uris) =
                launch_intent(Some("plugins/installed"), Some("clipboard"))
            else {
                panic!()
            };
            assert_eq!(
                opened(&uris[0]),
                Some((Target::Entry("plugins/installed"), Some("clipboard".into())))
            );
            assert_eq!(
                opened_share("enzui-settings:backups/destination?share=net-0906ea0a378ba969"),
                Some("net-0906ea0a378ba969".into())
            );
            assert_eq!(
                opened_share("enzui-settings:backups/destination?plugin=clipboard&share=../x"),
                None,
                "a share id is a plain location id"
            );
            assert_eq!(
                opened_share("enzui-settings:network/wifi?share=net-1"),
                None,
                "only a backups page sets up a share"
            );
            assert_eq!(
                opened_share("enzui-settings:backups?share=net-1"),
                Some("net-1".into())
            );
            assert_eq!(
                opened("enzui-settings:backups/destination?share=net-1").map(|(target, _)| target),
                Some(Target::Entry("backups/destination"))
            );
            assert_eq!(
                opened("enzui-settings:plugins/installed?plugin=../x"),
                Some((Target::Entry("plugins/installed"), None))
            );
            assert_eq!(opened("file:///etc/passwd"), None);
            assert_eq!(opened("enzui-settings:nowhere"), None);
        }
        // Regenerate with ENZUI_REGENERATE_AGENT_PACKAGING=1 after a registry change.
        {
            use enzui_agent::center::installed;
            let directory =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/agent");
            let files = [
                (
                    "dev.enzui.Settings.json",
                    installed::manifest(
                        &settings_app_center::registry(),
                        settings_app_center::metadata(),
                    )
                    .unwrap(),
                ),
                (
                    "enzui-agent-dev.enzui.Settings.service",
                    unit().unwrap().into_bytes(),
                ),
                (
                    "dev.enzui.Settings.service",
                    installed::dbus_service(APP_ID, INSTALLED_EXECUTABLE)
                        .unwrap()
                        .into_bytes(),
                ),
                (
                    "dev.enzui.Settings.background.json",
                    background_json().unwrap(),
                ),
                (
                    "dev.enzui.Settings.search-provider.ini",
                    enzui_agent_app::search_provider_ini(APP_ID, Some(2)).into_bytes(),
                ),
                ("dev.enzui.Settings.desktop", desktop_entry().into_bytes()),
            ];
            let spanish = enzui_core::localization::Localizer::new(
                enzui_core::localization::CatalogBuilder::new().build(),
                vec!["es".parse().unwrap()],
                Default::default(),
            );
            spanish.scope(|| {
                assert_eq!(
                    installed::manifest(
                        &settings_app_center::registry(),
                        settings_app_center::metadata()
                    )
                    .unwrap(),
                    files[0].1,
                );
                assert_eq!(background_json().unwrap(), files[3].1);
            });
            let regenerate =
                std::env::var("ENZUI_REGENERATE_AGENT_PACKAGING").as_deref() == Ok("1");
            for (name, bytes) in files {
                let path = directory.join(name);
                if regenerate {
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(&path, &bytes).unwrap();
                }
                assert_eq!(
                    std::fs::read(&path).unwrap_or_default(),
                    bytes,
                    "{name} drifted from the live registry"
                );
            }
        }
    }
}
