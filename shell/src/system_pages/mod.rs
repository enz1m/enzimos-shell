use enzui_shell::i18n::tr;
mod accessibility;
mod agents;
mod apps;
mod backup_space;
mod backups;
mod controllers;
mod developer;
mod devices;
mod disks;
mod display;
mod enzimos;
mod fingerprint;
mod firmware;
mod forms;
mod input;
mod network;
pub(super) mod phone_link;
mod power;
mod remote;
mod sound;
mod thunderbolt;

use super::{Message as Parent, SettingsApp};
use enzui_shell::settings_catalog::Category;
use enzui_shell::system_settings::{LongJob, Panel, Reading, Request, SystemSettings};
use enzui_widgets::{
    badge::{BadgeTone, enz_badge},
    banner::{BannerTone, enz_banner},
    text::{TextSize, enz_text},
};
use iced::{Element, Task};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

type Layouts = (Vec<(String, String)>, Option<String>);

pub(super) struct State {
    backend: SystemSettings,
    readings: HashMap<Panel, Reading>,
    read_at: HashMap<Panel, Instant>,
    in_flight: Option<Panel>,
    queue: VecDeque<Request>,
    busy: bool,
    // Requests whose outcome is shown before a reading confirms it, so a
    // dragged slider does not snap back while the tool runs.
    drafts: Vec<Request>,
    layouts: Option<Layouts>,
    layouts_loading: bool,
    all_vpns: bool,
    pub(super) layout_query: Option<String>,
    display_selected: Option<String>,
    // The layout to restore, and when, until the user keeps a change.
    display_confirm: Option<(Vec<enzui_shell::system_settings::display::Change>, Instant)>,
    forms: forms::Forms,
    // The request the backend is running now, for progress labels.
    current: Option<Request>,
    failure: Option<(Request, String)>,
    // A change finished while a reading was in flight; that reading may
    // predate it, so read once more.
    reread: bool,
    // What this computer offers (which pages show); `None` until known.
    pub(super) platform: Option<enzui_shell::platform::Platform>,
    // When the platform was last detected, and whether a detection runs.
    platform_at: Option<Instant>,
    platform_busy: bool,
    jobs: HashMap<LongJob, JobState>,
    deferred_job: Option<LongJob>,
    // Night light times being typed: (turns on, turns off).
    night_drafts: (Option<String>, Option<String>),
    night_text: (String, String),
    night_error: Option<String>,
    // A risky critical battery action waiting for the user's acknowledgement.
    battery_risk: Option<String>,
    memory_risk: bool,
    // A display tile being dragged in the arrangement preview: which, and
    // the pointer where the drag began and is now (preview pixels).
    display_drag: Option<(String, Option<iced::Point>, iced::Point)>,
    // Where the pointer is over the preview.
    display_pointer: Option<iced::Point>,
    // The supervised account being edited, if any.
    parental: Option<ParentalEdit>,
    // Game controllers present, while their page is open.
    controllers: Option<Vec<enzui_shell::controllers::Controller>>,
    // The controller being tested, and what the test has seen.
    controller_test: Option<controllers::Test>,
    controller_seat: enzui_shell::session_activity::State,
    controller_notice: Option<&'static str>,
    developer: developer::Developer,
    firmware: firmware::Firmware,
    thunderbolt: thunderbolt::Thunderbolt,
    disks: disks::Disks,
    fingerprint: fingerprint::Fingerprint,
    backup_space: backup_space::BackupSpace,
    location: enzui_shell::location::Mode,
    location_error: Option<String>,
    backups: backups::Backups,
    remote: remote::Remote,
    phone: phone_link::Phone,
    screen_reader: accessibility::ScreenReader,
    // About's package list is open.
    about_details: bool,
}

pub(super) struct ParentalEdit {
    account: enzui_shell::parental::Account,
    // Each shown app and the filter entry that names it.
    apps: Vec<(enzui_shell::default_apps::Launchable, String)>,
    // `None` while reading (which may ask for an administrator).
    controls: Option<Result<enzui_shell::parental::Controls, String>>,
    saving: bool,
    saved: Option<Result<(), String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum JobState {
    Running,
    Done(Result<bool, String>),
}
impl State {
    pub(super) fn for_environment() -> Self {
        Self {
            backend: SystemSettings::from_environment(),
            readings: HashMap::new(),
            read_at: HashMap::new(),
            in_flight: None,
            queue: VecDeque::new(),
            busy: false,
            drafts: Vec::new(),
            layouts: None,
            layouts_loading: false,
            all_vpns: false,
            layout_query: None,
            display_selected: None,
            display_confirm: None,
            forms: forms::Forms::default(),
            current: None,
            failure: None,
            reread: false,
            platform: None,
            platform_at: None,
            platform_busy: false,
            jobs: HashMap::new(),
            deferred_job: None,
            night_drafts: (None, None),
            night_text: ("20:00".into(), "07:00".into()),
            night_error: None,
            battery_risk: None,
            memory_risk: false,
            display_drag: None,
            display_pointer: None,
            parental: None,
            controllers: None,
            controller_test: None,
            controller_seat: Default::default(),
            controller_notice: None,
            developer: developer::Developer::default(),
            firmware: firmware::Firmware::default(),
            thunderbolt: thunderbolt::Thunderbolt::default(),
            disks: disks::Disks::default(),
            fingerprint: fingerprint::Fingerprint::default(),
            backup_space: backup_space::BackupSpace::default(),
            location: enzui_shell::location::load(),
            location_error: None,
            backups: backups::Backups::default(),
            remote: remote::Remote::default(),
            phone: phone_link::Phone::default(),
            screen_reader: accessibility::ScreenReader::default(),
            about_details: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Message {
    Loaded(Box<Reading>),
    // A page's reading took too long; the last one stays.
    ReadTimedOut(Panel),
    Request(Request),
    Applied(Result<(), String>),
    Layouts(Vec<(String, String)>, Option<String>),
    ShowAllVpns(bool),
    Controllers(Vec<enzui_shell::controllers::Controller>),
    TestController(Option<String>),
    ControllerInput(u64, enzui_shell::controllers::Input),
    ControllerSeat(enzui_shell::session_activity::State),
    VibrateController(u64),
    ControllerVibration(u64, u64, enzui_shell::controller_vibration::Event),
    Developer(developer::DeveloperMessage),
    Firmware(firmware::FirmwareMessage),
    Thunderbolt(thunderbolt::ThunderboltMessage),
    Disks(disks::DisksMessage),
    Fingerprint(fingerprint::FingerprintMessage),
    BackupSpace(backup_space::SpaceMessage),
    Location(enzui_shell::location::Mode),
    LocationSaved(enzui_shell::location::Mode, Result<(), String>),
    Remote(remote::RemoteMessage),
    Phone(phone_link::PhoneMessage),
    ScreenReader(accessibility::ScreenReaderMessage),
    Backups(backups::BackupsMessage),
    LayoutQuery(Option<String>),
    AboutDetails(bool),
    SetLayouts(Vec<String>),
    SelectDisplay(String),
    SetDisplay(Vec<enzui_shell::system_settings::display::Change>),
    KeepDisplay,
    RevertDisplay,
    Form(forms::Form),
    VpnProfileSelected(String),
    Pairing(Pairing),
    Platform(Box<enzui_shell::platform::Platform>),
    StartJob(LongJob),
    JobDone(LongJob, Result<bool, String>),
    Reboot,
    NightTime(bool, enzui_widgets::time_field::TimeFieldEvent),
    ChooseWorldClock(String),
    BatteryRisk(Option<String>),
    AcceptBatteryRisk,
    MemoryRisk(bool),
    AcceptMemoryRisk,
    DisplayDragStart,
    DisplayDragMove(iced::Point),
    DisplayDragEnd(bool),
    ParentalOpen(enzui_shell::parental::Account),
    ParentalLoaded(
        u32,
        Result<
            (
                enzui_shell::parental::Controls,
                Vec<(enzui_shell::default_apps::Launchable, String)>,
            ),
            String,
        >,
    ),
    ParentalApp(String, bool),
    ParentalInstalling(bool),
    ParentalSave,
    // The account whose controls were saved, and the outcome.
    ParentalSaved(u32, Result<(), String>),
    ParentalClose,
}

#[derive(Clone, Debug)]
pub(super) enum Pairing {
    Ask(enzui_shell::system_settings::pairing::Prompt),
    Answer(enzui_shell::system_settings::pairing::Answer),
    Done(Result<(), String>),
}

/// The network share dialog's modal host events.
pub(super) fn backup_share_host(event: enzui_widgets::modal_host::ModalHostEvent) -> Parent {
    backups::share_host(event)
}

pub(super) fn panel(category: Category) -> Option<Panel> {
    Some(match category {
        Category::Display => Panel::Display,
        Category::Network => Panel::Network,
        Category::Bluetooth => Panel::Bluetooth,
        Category::Sound => Panel::Sound,
        Category::Power => Panel::Power,
        Category::Printers => Panel::Printers,
        Category::DateTime => Panel::DateTime,
        Category::About => Panel::About,
        Category::Updates => Panel::Updates,
        Category::Backups => Panel::Safety,
        Category::Agents => Panel::Agents,
        Category::Apps => Panel::Apps,
        _ => return None,
    })
}

// How long a page's reading may take. The enzimOS pages may wait for the
// backup repository's lock; the app permission inventory has 30 s itself.
fn read_limit(panel: Panel) -> Duration {
    Duration::from_secs(match panel {
        Panel::Updates | Panel::Safety => 120,
        Panel::Apps => 45,
        _ => 20,
    })
}

fn refresh(panel: Panel) -> Option<Duration> {
    match panel {
        Panel::Sound => Some(Duration::from_secs(2)),
        // NetworkManager's changes reread the page at once (the connectivity
        // subscription); this only refreshes the Wi-Fi list it caches. Each
        // read starts several `nmcli` processes, each loading every
        // NetworkManager object.
        Panel::Network => Some(Duration::from_secs(15)),
        // Listing recovery points temporarily mounts the repository under its operation lock.
        Panel::Safety => Some(Duration::from_secs(60)),
        Panel::About => None,
        Panel::Updates | Panel::Apps => Some(Duration::from_secs(30)),
        _ => Some(Duration::from_secs(5)),
    }
}

impl SettingsApp {
    pub(super) fn reveal_ready(&self) -> bool {
        self.highlighted.is_none_or(|route| self.route_ready(route))
    }

    pub(super) fn route_ready(&self, route: &str) -> bool {
        if route.starts_with("users/") {
            return self.users_ready();
        }
        let panel =
            enzui_shell::settings_catalog::find(route).and_then(|entry| panel(entry.category()));
        panel.is_none_or(|panel| self.system.readings.contains_key(&panel))
            && (!matches!(
                route,
                "apps/background"
                    | "apps/permissions"
                    | "apps/location"
                    | "apps/parental"
                    | "apps/screen_time"
            ) || self.background_ready())
    }

    /// Detects what this computer offers, at most every 30 s.
    pub(super) fn detect_platform(&mut self) -> Task<Parent> {
        if self.system.platform_busy
            || self
                .system
                .platform_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return Task::none();
        }
        self.system.platform_busy = true;
        let backend = self.system.backend.clone();
        Task::perform(async move { backend.platform().await }, |platform| {
            Parent::System(Message::Platform(Box::new(platform)))
        })
    }
    /// Whether a feature's page or card shows; until the platform is known
    /// it does, as it always has.
    pub(super) fn offers(&self, feature: enzui_shell::platform::Feature) -> bool {
        self.system
            .platform
            .as_ref()
            .is_none_or(|platform| platform.offers(feature))
    }
    pub(super) fn category_available(&self, category: Category) -> bool {
        enzui_shell::platform::shows(self.system.platform.as_ref(), category.page())
    }
    /// What manages the network where NetworkManager's page is hidden.
    pub(super) fn network_elsewhere(&self) -> Option<String> {
        let services = self.system.platform.as_ref()?.network_elsewhere()?;
        Some(if services.is_empty() {
            tr("shell-no-network-service-is-running").to_owned()
        } else {
            enzui_shell::tr!("shell-managed-by", "arg0" => services.join(", "))
        })
    }
    /// Whether a catalog route is listed: its page and the setting itself.
    pub(super) fn route_shown(&self, entry: enzui_shell::settings_catalog::Entry) -> bool {
        self.category_available(entry.category())
            && enzui_shell::platform::shows(self.system.platform.as_ref(), entry.route)
    }
    /// Whether a launch request or search choice for `route` is refused:
    /// only once the platform is known to lack it, so a request that
    /// arrives before detection opens and is moved on when it finishes.
    pub(super) fn route_refused(&self, route: &str) -> bool {
        self.system.platform.is_some()
            && !enzui_shell::platform::shows(self.system.platform.as_ref(), route)
    }
    pub(super) fn display_confirm_pending(&self) -> bool {
        self.system.display_confirm.is_some()
    }
    /// How often the settings tick runs: every second for a countdown, the
    /// sound levels (read every 2 s) or a backup setup that is waiting;
    /// otherwise every 5 s, as often as any other page is read again.
    pub(super) fn tick_period(&self) -> Duration {
        if self.display_confirm_pending()
            || panel(self.page) == Some(Panel::Sound)
            || self.backup_setup_waiting()
        {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(5)
        }
    }
    pub(super) fn system_tick(&mut self) -> Task<Parent> {
        if self
            .system
            .display_confirm
            .as_ref()
            .is_some_and(|(_, deadline)| Instant::now() >= *deadline)
        {
            return self.system_message(Message::RevertDisplay);
        }
        Task::batch([
            self.backup_setup_tick(),
            self.read_system(false),
            self.detect_platform(),
        ])
    }
    pub(super) fn network_changed(&mut self) -> Task<Parent> {
        // A new state brings the sign-in notice back if it was dismissed.
        self.system.forms.portal_dismissed = false;
        self.read_system(true)
    }
    // A read of another page held the reader while this one waited; it
    // starts now instead of at the next tick.
    fn read_page_left_waiting(&mut self, finished: Panel) -> Task<Parent> {
        if panel(self.page).is_some_and(|shown| shown != finished) {
            self.read_system(true)
        } else {
            Task::none()
        }
    }
    pub(super) fn read_system(&mut self, force: bool) -> Task<Parent> {
        if self.page == Category::Users {
            return Task::batch([self.read_users(), self.fingerprint_read(force)]);
        }
        if self.page == Category::Developer {
            return self.developer_read(force);
        }
        if self.page == Category::Phone {
            return self.phone_read(force);
        }
        if self.page == Category::Accessibility {
            return self.screen_reader_read();
        }
        if self.page == Category::Thunderbolt {
            return self.thunderbolt_read(force);
        }
        if self.page == Category::Backups {
            // Like the page's own read: nothing new while a backup job runs.
            let running = self
                .system
                .jobs
                .values()
                .any(|job| *job == JobState::Running);
            let space = if running {
                Task::none()
            } else {
                self.backup_space_read(force)
            };
            if space.units() > 0 {
                return Task::batch([space, self.read_panel(force)]);
            }
        }
        if self.page == Category::Disks {
            return self.disks_read(force);
        }
        // Device firmware shows on Updates beside the system's own updates.
        let firmware = if self.page == Category::Updates {
            self.firmware_read(force)
        } else {
            Task::none()
        };
        if firmware.units() > 0 {
            return Task::batch([firmware, self.read_panel(force)]);
        }
        self.read_panel(force)
    }

    fn read_panel(&mut self, force: bool) -> Task<Parent> {
        if self.page == Category::Remote {
            return self.remote_read(force);
        }
        if self.page == Category::Agents {
            let assistant = self.assistant_read();
            let local_ai = self.local_ai_read();
            if assistant.units() + local_ai.units() > 0 {
                return Task::batch([assistant, local_ai]);
            }
        }
        if self.page == Category::Keyboard
            && self.system.layouts.is_none()
            && !self.system.layouts_loading
        {
            self.system.layouts_loading = true;
            let backend = self.system.backend.clone();
            return Task::perform(
                async move { enzui_shell::system_settings::keyboard_layouts(&backend).await },
                |(layouts, system)| Parent::System(Message::Layouts(layouts, system)),
            );
        }
        let Some(panel) = panel(self.page) else {
            return Task::none();
        };
        if panel == Panel::Safety
            && (self.backup_read_blocked(force)
                || self
                    .system
                    .jobs
                    .values()
                    .any(|job| *job == JobState::Running))
        {
            return Task::none();
        }
        let state = &mut self.system;
        let stale = match (state.read_at.get(&panel), refresh(panel)) {
            (None, _) => true,
            (Some(at), Some(every)) => at.elapsed() >= every,
            (Some(_), None) => false,
        };
        if state.in_flight.is_some() || state.busy || !state.queue.is_empty() || !(force || stale) {
            return Task::none();
        }
        state.in_flight = Some(panel);
        let backend = state.backend.clone();
        // One page's stuck backend must not hold every other page's reads.
        Task::perform(
            async move { tokio::time::timeout(read_limit(panel), backend.read(panel)).await },
            move |reading| {
                Parent::System(match reading {
                    Ok(reading) => Message::Loaded(Box::new(reading)),
                    Err(_) => Message::ReadTimedOut(panel),
                })
            },
        )
    }
    pub(super) fn system_message(&mut self, message: Message) -> Task<Parent> {
        match message {
            Message::BatteryRisk(action) => {
                self.menu = None;
                self.system.battery_risk = action;
                Task::none()
            }
            Message::MemoryRisk(asking) => {
                self.menu = None;
                self.system.memory_risk = asking;
                Task::none()
            }
            Message::AcceptMemoryRisk => {
                self.system.memory_risk = false;
                self.system_message(Message::Request(Request::MemoryPolicy {
                    mode: "off".into(),
                    risk_acknowledged: true,
                }))
            }
            Message::AcceptBatteryRisk => match self.system.battery_risk.take() {
                Some(action) => self.system_message(Message::Request(Request::CriticalBattery {
                    action,
                    risk_acknowledged: true,
                })),
                None => Task::none(),
            },
            Message::Platform(platform) => {
                self.system.platform_busy = false;
                self.system.platform_at = Some(Instant::now());
                self.system.platform = Some(*platform);
                if self
                    .highlighted
                    .is_some_and(|route| self.route_refused(route))
                {
                    self.highlighted = None;
                    self.reveal_pending = false;
                }
                if self.route_refused(self.page.page()) {
                    return self.update(Parent::Page(Category::Appearance));
                }
                Task::none()
            }
            Message::StartJob(job) => {
                if self.system.jobs.get(&job) == Some(&JobState::Running) {
                    return Task::none();
                }
                if self.system.deferred_job.is_some() {
                    self.error = Some(tr("shell-a-system-job-is-already-waiting-for-the-current-reading-wait-for-it-to-finis").into());
                    return Task::none();
                }
                self.system.jobs.insert(job, JobState::Running);
                if self.system.in_flight.is_some() {
                    self.system.deferred_job = Some(job);
                    Task::none()
                } else {
                    self.start_system_job(job)
                }
            }
            Message::JobDone(job, result) => {
                self.system.jobs.insert(job, JobState::Done(result));
                self.read_system(true)
            }
            Message::ChooseWorldClock(zone) => {
                self.system.forms.clock_query = None;
                self.update(Parent::Change(
                    enzui_shell::settings::Change::WorldClockAdd(zone),
                ))
            }
            Message::NightTime(from, event) => {
                use enzui_widgets::time_field::TimeFieldEvent;
                let draft = if from {
                    &mut self.system.night_drafts.0
                } else {
                    &mut self.system.night_drafts.1
                };
                match event {
                    TimeFieldEvent::Drafted(text) => {
                        *draft = Some(text);
                        Task::none()
                    }
                    TimeFieldEvent::Committed(time) => {
                        *draft = None;
                        self.system.night_error = None;
                        let minutes = (time.seconds_since_midnight() / 60) as u16;
                        let change = if from {
                            enzui_shell::night_light::Change::From(minutes)
                        } else {
                            enzui_shell::night_light::Change::To(minutes)
                        };
                        self.update(Parent::Change(enzui_shell::settings::Change::NightLight(
                            change,
                        )))
                    }
                    TimeFieldEvent::Refused(_) => {
                        self.system.night_error = Some(tr("shell-type-a-time-like-20-00").into());
                        Task::none()
                    }
                }
            }
            Message::Reboot => {
                let backend = self.system.backend.clone();
                Task::perform(async move { backend.reboot().await }, |result| {
                    Parent::System(Message::Applied(result))
                })
            }
            Message::ReadTimedOut(panel) => {
                self.system.in_flight = None;
                // Read again at the page's next refresh, not at once.
                self.system.read_at.insert(panel, Instant::now());
                if let Some(job) = self.system.deferred_job.take() {
                    return self.start_system_job(job);
                }
                self.read_page_left_waiting(panel)
            }
            Message::Loaded(reading) => {
                let panel = reading.panel();
                self.system.in_flight = None;
                if !self.system.busy && self.system.queue.is_empty() {
                    self.system.drafts.clear();
                }
                // A busy backend's refusal keeps the last good page and is
                // read again at the next refresh.
                if !self
                    .system
                    .readings
                    .get(&panel)
                    .is_some_and(|previous| reading.refused_over(previous))
                {
                    self.system.read_at.insert(panel, Instant::now());
                    self.system.readings.insert(panel, *reading);
                }
                if let Some(job) = self.system.deferred_job.take() {
                    return self.start_system_job(job);
                }
                if std::mem::take(&mut self.system.reread) {
                    return self.read_system(true);
                }
                self.read_page_left_waiting(panel)
            }
            Message::LayoutQuery(query) => {
                let opened = self.system.layout_query.is_none() && query.is_some();
                self.system.layout_query = query.map(|q| q.chars().take(64).collect());
                if opened {
                    iced::widget::operation::focus("keyboard-layout-search")
                } else {
                    Task::none()
                }
            }
            Message::Form(form) => self.form_message(form),
            Message::VpnProfileSelected(path) => {
                self.form_message(forms::Form::VpnPath(Some(path)))
            }
            Message::Pairing(Pairing::Ask(prompt)) => {
                // Informational codes replace each other; questions wait
                // their turn so none is dropped unanswered.
                self.system
                    .forms
                    .pair_prompts
                    .retain(enzui_shell::system_settings::pairing::Prompt::needs_answer);
                self.system.forms.pair_prompts.push(prompt);
                Task::none()
            }
            Message::Pairing(Pairing::Answer(answer)) => {
                if let Some(index) = self
                    .system
                    .forms
                    .pair_prompts
                    .iter()
                    .position(enzui_shell::system_settings::pairing::Prompt::needs_answer)
                {
                    self.system.forms.pair_prompts.remove(index).answer(answer);
                }
                self.system.forms.pin.clear();
                Task::none()
            }
            Message::Pairing(Pairing::Done(result)) => {
                self.system.forms.pair_prompts.clear();
                self.system_message(Message::Applied(result))
            }
            Message::SelectDisplay(name) => {
                self.system.display_selected = Some(name);
                Task::none()
            }
            Message::ParentalOpen(account) => {
                let uid = account.uid;
                self.system.parental = Some(ParentalEdit {
                    account: account.clone(),
                    apps: Vec::new(),
                    controls: None,
                    saving: false,
                    saved: None,
                });
                Task::perform(
                    async move {
                        let controls = enzui_shell::parental::controls(&account).await?;
                        let apps = tokio::task::spawn_blocking(|| {
                            let refs = enzui_shell::parental::flatpak_refs();
                            enzui_shell::default_apps::launchable()
                                .into_iter()
                                .filter_map(|app| {
                                    Some((enzui_shell::parental::key(&app, &refs)?, app))
                                })
                                .map(|(key, app)| (app, key))
                                .collect()
                        })
                        .await
                        .map_err(|_| tr("shell-apps-could-not-be-listed").to_owned())?;
                        Ok((controls, apps))
                    },
                    move |result| Parent::System(Message::ParentalLoaded(uid, result)),
                )
            }
            Message::ParentalLoaded(uid, result) => {
                if let Some(edit) = self
                    .system
                    .parental
                    .as_mut()
                    .filter(|edit| edit.account.uid == uid)
                {
                    match result {
                        Ok((controls, apps)) => {
                            edit.apps = apps;
                            edit.controls = Some(Ok(controls));
                        }
                        Err(error) => edit.controls = Some(Err(error)),
                    }
                }
                Task::none()
            }
            Message::ParentalApp(key, allowed) => {
                if let Some(Some(Ok(controls))) = self
                    .system
                    .parental
                    .as_mut()
                    .map(|edit| edit.controls.as_mut())
                {
                    let filter = &mut controls.filter;
                    // An allow list names what may start; a block list what may not.
                    let listed = allowed == filter.allowlist;
                    filter.entries.retain(|entry| *entry != key);
                    if listed {
                        filter.entries.push(key);
                    }
                }
                Task::none()
            }
            Message::ParentalInstalling(on) => {
                if let Some(Some(Ok(controls))) = self
                    .system
                    .parental
                    .as_mut()
                    .map(|edit| edit.controls.as_mut())
                {
                    controls.allow_installing = on;
                }
                Task::none()
            }
            Message::ParentalSave => {
                let Some(edit) = self.system.parental.as_mut().filter(|edit| !edit.saving) else {
                    return Task::none();
                };
                let Some(Ok(controls)) = edit.controls.clone() else {
                    return Task::none();
                };
                edit.saving = true;
                edit.saved = None;
                let account = edit.account.clone();
                let uid = account.uid;
                Task::perform(
                    async move { enzui_shell::parental::set_controls(&account, &controls).await },
                    move |result| Parent::System(Message::ParentalSaved(uid, result)),
                )
            }
            Message::ParentalSaved(uid, result) => {
                // Another account's editor opened meanwhile is not marked
                // saved by this one's answer.
                if let Some(edit) = self
                    .system
                    .parental
                    .as_mut()
                    .filter(|edit| edit.account.uid == uid)
                {
                    edit.saving = false;
                    edit.saved = Some(result);
                }
                Task::none()
            }
            Message::ParentalClose => {
                self.system.parental = None;
                Task::none()
            }
            Message::DisplayDragStart => {
                let (Some(point), Some(Reading::Display(displays))) = (
                    self.system.display_pointer,
                    self.system.readings.get(&Panel::Display),
                ) else {
                    return Task::none();
                };
                if let Some(name) = display::tile_at(displays, point) {
                    self.system.display_selected = Some(name.clone());
                    self.system.display_drag = Some((name, Some(point), point));
                }
                Task::none()
            }
            Message::DisplayDragMove(point) => {
                self.system.display_pointer = Some(point);
                if let Some((_, _, to)) = &mut self.system.display_drag {
                    *to = point;
                }
                Task::none()
            }
            Message::DisplayDragEnd(drop) => {
                if !drop {
                    self.system.display_pointer = None;
                }
                let Some((name, Some(from), to)) = self.system.display_drag.take() else {
                    return Task::none();
                };
                let Some(Reading::Display(displays)) =
                    self.system.readings.get(&Panel::Display).cloned()
                else {
                    return Task::none();
                };
                match display::dropped_layout(&displays, &name, (to.x - from.x, to.y - from.y))
                    .filter(|_| drop)
                {
                    Some(layout) => self.request_display(layout),
                    None => Task::none(),
                }
            }
            Message::SetDisplay(change) => self.request_display(change),
            Message::KeepDisplay => {
                self.system.display_confirm = None;
                match self.system.readings.get(&Panel::Display) {
                    Some(Reading::Display(d)) => {
                        let saved = enzui_shell::system_settings::display::saved(
                            d,
                            &enzui_shell::system_settings::display::layout(d),
                        );
                        self.update(Parent::Change(enzui_shell::settings::Change::Displays(
                            saved,
                        )))
                    }
                    _ => Task::none(),
                }
            }
            Message::RevertDisplay => match self.system.display_confirm.take() {
                Some((previous, _)) => {
                    self.system_message(Message::Request(Request::Display(previous)))
                }
                None => Task::none(),
            },
            Message::SetLayouts(layouts) => {
                self.system.layout_query = None;
                self.update(Parent::Change(
                    enzui_shell::settings::Change::KeyboardLayouts(layouts),
                ))
            }
            Message::Controllers(list) => {
                // A tested controller that went away ends its test.
                if let Some(test) = &self.system.controller_test
                    && !list.iter().any(|c| c.id == test.id && c.node == test.node)
                {
                    self.system.controller_test = None;
                }
                self.system.controllers = Some(list);
                Task::none()
            }
            Message::TestController(id) => {
                if id.is_some() && enzui_shell::controller_vibration::has_pending_job() {
                    self.system.controller_notice =
                        Some(enzui_shell::controller_vibration::STOPPING);
                    return Task::none();
                }
                self.system.controller_notice = None;
                self.system.controller_test = self
                    .can_test_controllers()
                    .then_some(id)
                    .flatten()
                    .and_then(|id| {
                        let node = self
                            .system
                            .controllers
                            .as_ref()?
                            .iter()
                            .find(|c| c.id == id)?
                            .node
                            .clone();
                        Some(controllers::Test::new(id, node))
                    });
                Task::none()
            }
            Message::ControllerInput(generation, input) => {
                if let Some(test) = self.system.controller_test.as_mut() {
                    test.apply(generation, input);
                }
                Task::none()
            }
            Message::VibrateController(generation) => self.controller_vibrate(generation),
            Message::ControllerVibration(generation, epoch, event) => {
                self.controller_vibration_event(generation, epoch, event);
                Task::none()
            }
            Message::ControllerSeat(state) => {
                self.system.controller_seat = state;
                if !self.can_test_controllers() {
                    self.system.controller_test = None;
                }
                Task::none()
            }
            Message::Developer(message) => self.developer_message(message),
            Message::Firmware(message) => self.firmware_message(message),
            Message::Thunderbolt(message) => self.thunderbolt_message(message),
            Message::Disks(message) => self.disks_message(message),
            Message::Fingerprint(message) => self.fingerprint_message(message),
            Message::BackupSpace(message) => self.backup_space_message(message),
            Message::AboutDetails(open) => {
                self.system.about_details = open;
                Task::none()
            }
            Message::Location(mode) => {
                self.menu = None;
                if self.system.backend.demo {
                    self.system.location = mode;
                    return Task::none();
                }
                Task::perform(enzui_shell::location::set(mode), move |result| {
                    Parent::System(Message::LocationSaved(mode, result))
                })
            }
            Message::LocationSaved(mode, result) => {
                match result {
                    Ok(()) => {
                        self.system.location = mode;
                        self.system.location_error = None;
                    }
                    Err(error) => self.system.location_error = Some(error),
                }
                Task::none()
            }
            Message::Remote(message) => self.remote_message(message),
            Message::Phone(message) => self.phone_message(message),
            Message::ScreenReader(message) => self.screen_reader_message(message),
            Message::Backups(message) => self.backups_message(message),
            Message::ShowAllVpns(all) => {
                self.system.all_vpns = all;
                Task::none()
            }
            Message::Layouts(layouts, system) => {
                self.system.layouts_loading = false;
                self.system.layouts = Some((layouts, system));
                Task::none()
            }
            Message::Request(request) => {
                if self.closing {
                    return Task::none();
                }
                if !self.system.backend.allows(&request) {
                    self.error = Some(tr("shell-system-changes-are-off-in-this-preview").into());
                    return Task::none();
                }
                self.system.drafts.retain(|d| !d.same_slot(&request));
                self.system.drafts.push(request.clone());
                self.system.queue.retain(|q| !q.same_slot(&request));
                self.system.queue.push_back(request);
                self.apply_next_system()
            }
            Message::Applied(result) => {
                self.system.busy = false;
                let request = self.system.current.take();
                let handled = request
                    .as_ref()
                    .is_some_and(|request| self.finish_form(request, &result));
                if result.is_ok()
                    && let Some((failed, error)) = &self.system.failure
                    && request
                        .as_ref()
                        .is_some_and(|request| request == failed || request.same_slot(failed))
                {
                    if self.error.as_ref() == Some(error) {
                        self.error = None;
                    }
                    self.system.failure = None;
                }
                if let Err(error) = result {
                    // Only a failed display change ends its own confirmation; another
                    // request failing meanwhile must leave the countdown and its revert.
                    let display = matches!(request, Some(Request::Display(_)));
                    if display {
                        self.system.display_confirm = None;
                    }
                    if !handled {
                        self.system.failure = request.map(|request| (request, error.clone()));
                        self.error = Some(error);
                    }
                    self.system
                        .queue
                        .retain(|queued| !display && matches!(queued, Request::Display(_)));
                    self.system.drafts.clear();
                }
                if self.system.queue.is_empty() {
                    self.system.reread |= self.system.in_flight.is_some();
                    self.read_system(true)
                } else {
                    self.apply_next_system()
                }
            }
        }
    }
    fn start_system_job(&self, job: LongJob) -> Task<Parent> {
        let backend = self.system.backend.clone();
        Task::perform(async move { backend.long_job(job).await }, move |result| {
            Parent::System(Message::JobDone(job, result))
        })
    }
    fn apply_next_system(&mut self) -> Task<Parent> {
        if self.system.busy {
            return Task::none();
        }
        let Some(request) = self.system.queue.pop_front() else {
            return Task::none();
        };
        self.system.busy = true;
        self.system.current = Some(request.clone());
        let backend = self.system.backend.clone();
        if let Request::BluetoothPair(address) = request {
            use iced::futures::SinkExt;
            let stream = iced::stream::channel(
                8,
                async move |mut events: iced::futures::channel::mpsc::Sender<Pairing>| {
                    let asks = std::sync::Mutex::new(events.clone());
                    let result = backend
                        .pair_device(&address, move |prompt| {
                            let _ = asks
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .try_send(Pairing::Ask(prompt));
                        })
                        .await;
                    let _ = events.send(Pairing::Done(result)).await;
                },
            );
            return Task::run(stream, |event| Parent::System(Message::Pairing(event)));
        }
        Task::perform(async move { backend.apply(&request).await }, |result| {
            Parent::System(Message::Applied(result))
        })
    }
    #[cfg(test)]
    pub(super) fn live_system_backend(&mut self) {
        self.system.backend.demo = false;
        self.system.backend.writes_allowed = true;
    }
    pub(super) fn system_backend(&self) -> SystemSettings {
        self.system.backend.clone()
    }
    pub(super) fn system_writes_allowed(&self) -> bool {
        self.system.backend.allows(&Request::Ntp(true))
    }
    pub(super) fn system_idle(&self) -> bool {
        !self.system.busy && self.system.queue.is_empty()
    }
    pub(super) fn current_system_request(&self) -> Option<&Request> {
        self.system.current.as_ref()
    }
    pub(super) fn store_system_reading(&mut self, reading: Reading) {
        let panel = reading.panel();
        // Unmarked as read, so the next refresh asks again.
        if self
            .system
            .readings
            .get(&panel)
            .is_some_and(|previous| reading.refused_over(previous))
        {
            return;
        }
        self.system.read_at.insert(panel, Instant::now());
        self.system.readings.insert(panel, reading);
    }
    pub(super) fn reading(&self, panel: Panel) -> Option<Reading> {
        let mut reading = self.system.readings.get(&panel)?.clone();
        for draft in &self.system.drafts {
            if !matches!(draft, Request::PrinterAdd(_)) {
                reading.apply(draft);
            }
        }
        Some(reading)
    }
    fn system_writes(&self) -> bool {
        self.system.backend.writes_allowed && self.controls_live()
    }
    fn request(request: Request) -> Parent {
        Parent::System(Message::Request(request))
    }
    fn read_only_notice(&self) -> Option<Element<'_, Parent>> {
        if self.system.backend.writes_allowed {
            return None;
        }
        Some(
            enz_banner(tr("shell-read-only-in-this-preview"))
                .body(tr("shell-you-are-seeing-this-computer-s-real-settings-changes-to-them-are-turned-off"))
                .tone(BannerTone::Info)
                .icon("eye")
                .id("system-read-only")
                .into_element(),
        )
    }
    fn demo_notice(&self) -> Option<Element<'_, Parent>> {
        self.system.backend.demo.then(|| {
            enz_banner(if self.page == Category::Developer {
                tr("shell-sample-data")
            } else {
                tr("shell-sample-devices")
            })
                .body(if self.page == Category::Developer {
                    tr("shell-this-page-shows-example-data")
                } else {
                    tr("shell-this-page-shows-example-hardware-so-it-can-be-reviewed-without-it-nothing-he")
                })
                .tone(BannerTone::Warning)
                .icon("flask-conical")
                .id("system-demo")
                .into_element()
        })
    }
    pub(super) fn caption<'a>(&self, text: impl Into<String>) -> Element<'a, Parent> {
        enz_text(text.into())
            .size(TextSize::Footnote)
            .muted_on(self.tokens().palette.surface)
            .into_element()
    }
    fn loading_row<'a>(&self) -> Element<'a, Parent> {
        crate::settings_app::settings_row(tr("shell-checking"))
            .enabled(false)
            .into_element()
    }
    pub(super) fn info_row<'a>(
        &self,
        icon: &'a str,
        title: impl Into<String>,
        subtitle: impl Into<String>,
    ) -> Element<'a, Parent> {
        crate::settings_app::settings_row(title.into())
            .subtitle(subtitle.into())
            .icon(icon)
            .into_element()
    }
    fn property_row<'a>(
        &self,
        title: impl Into<String>,
        value: impl Into<String>,
    ) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        iced::widget::container(
            iced::widget::row![
                enz_text(title)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .no_wrap()
                    .into_element(),
                enz_text(value.into())
                    .muted_on(palette.surface)
                    .align(enzui_widgets::text::TextAlign::Right)
                    .width(iced::Length::Fill)
                    .into_element(),
            ]
            .spacing(24)
            .align_y(iced::Alignment::Center),
        )
        .padding([13, 12])
        .width(iced::Length::Fill)
        .into()
    }
    fn detail_row<'a>(&self, title: &'a str, value: impl Into<String>) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        iced::widget::container(
            iced::widget::row![
                enz_text(title)
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .width(iced::Length::Fixed(140.0))
                    .into_element(),
                enz_text(value.into())
                    .size(TextSize::Footnote)
                    .single_line_elided()
                    .into_element(),
            ]
            .spacing(12),
        )
        .padding([4, 12])
        .into()
    }
    fn detail_row_owned<'a>(&self, title: String, value: String) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        iced::widget::container(
            iced::widget::row![
                enz_text(title)
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .width(iced::Length::Fill)
                    .into_element(),
                enz_text(value).size(TextSize::Footnote).into_element(),
            ]
            .spacing(12),
        )
        .padding([4, 10])
        .into()
    }
    fn caption_row<'a>(&self, text: impl Into<String>) -> Element<'a, Parent> {
        iced::widget::container(self.caption(text))
            .padding([6, 12])
            .into()
    }
    fn status_badge<'a>(&self, label: &str, tone: BadgeTone) -> Element<'a, Parent> {
        enz_badge(label).tone(tone).into_element()
    }
    pub(super) fn system_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let mut body = body;
        // These pages read their own sample data in demo mode.
        if matches!(
            self.page,
            Category::Thunderbolt | Category::Disks | Category::Developer
        ) && let Some(notice) = self.demo_notice()
        {
            body = body.push(notice);
        }
        if let Some(panel) = panel(self.page) {
            if let Some(notice) = self.demo_notice() {
                body = body.push(notice);
            } else if !matches!(
                panel,
                Panel::Sound | Panel::About | Panel::Display | Panel::Agents | Panel::Apps
            ) && let Some(notice) = self.read_only_notice()
            {
                body = body.push(notice);
            }
        }
        match self.page {
            Category::Display => self.display_page(body),
            Category::Network => self.network_page(body),
            Category::Bluetooth => self.bluetooth_page(body),
            Category::Sound => self.sound_page(body),
            Category::Power => self.power_page(body),
            Category::Printers => self.printers_page(body),
            Category::Thunderbolt => self.thunderbolt_page(body),
            Category::Disks => self.disks_page(body),
            Category::DateTime => self.datetime_page(body),
            Category::About => self.about_page(body),
            Category::Updates => self.updates_page(body),
            Category::Backups => self.safety_page(body),
            Category::Agents => self.agents_page(body),
            Category::Input => self.pointer_page(body),
            Category::Keyboard => self.keyboard_page(body),
            Category::Controllers => self.controllers_page(body),
            Category::Accessibility => self.accessibility_page(body),
            Category::Apps => self.apps_page(body),
            Category::Developer => self.developer_page(body),
            Category::Remote => self.remote_page(body),
            Category::Phone => self.phone_page(body),
            _ => body,
        }
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;
    use crate::settings_app::Store;

    fn app() -> (SettingsApp, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let app = SettingsApp::model(Store::at(directory.path().join("settings.json")).unwrap());
        (app, directory)
    }

    fn built(app: &SettingsApp) -> std::collections::BTreeSet<String> {
        app.built_routes.borrow_mut().clear();
        let _ = app.settings_page();
        app.built_routes.borrow().clone()
    }

    #[test]
    fn system_pages_behaviour() {
        // Off enzimOS a page whose backend is missing is not listed, found or
        // opened, a page a launch opened before detection finished is left,
        // and the cards of missing services are hidden; on enzimOS every page
        // shows.
        //
        // Sabotage: drop the `route_refused` guard on `Page` or `Reveal`, the
        // route check in `route_shown`, the move to Appearance in the
        // `Platform` message, or the `offers` check before the power mode or
        // background apps card.
        {
            use enzui_shell::platform::Platform;
            use enzui_shell::settings_catalog::find;
            let (mut app, _directory) = app();
            let _ = app.update(Parent::Page(Category::Network));
            assert_eq!(app.page, Category::Network, "known only after detection");
            let arch = Platform {
                network_services: vec!["iwd"],
                pipewire: false,
                ..Platform::default()
            };
            let _ = app.system_message(Message::Platform(Box::new(arch)));
            assert_eq!(app.page, Category::Appearance);
            for page in [
                Category::Network,
                Category::Bluetooth,
                Category::Printers,
                Category::Updates,
                Category::Backups,
                Category::Developer,
                Category::Remote,
            ] {
                assert!(!app.category_available(page), "{page:?}");
                let _ = app.update(Parent::Page(page));
                assert_eq!(app.page, Category::Appearance, "{page:?}");
            }
            for route in [
                "power/mode",
                "network/wifi",
                "power/memory",
                "apps/background",
            ] {
                let _ = app.update(Parent::Reveal(route));
                assert_eq!(app.page, Category::Appearance, "{route}");
                assert!(!app.route_shown(find(route).unwrap()), "{route}");
            }
            assert!(app.route_shown(find("power/blank").unwrap()));
            assert!(
                app.network_elsewhere()
                    .is_some_and(|line| line.contains("iwd"))
            );
            app.page = Category::Power;
            let power = built(&app);
            assert!(power.contains("power/blank") && !power.contains("power/mode"));
            app.page = Category::Apps;
            assert!(!built(&app).contains("apps/background"));
            app.page = Category::Sound;
            let sound = built(&app);
            assert!(sound.contains("sound/output_volume") && !sound.contains("sound/output"));

            let enzimos = Platform {
                enzimos: true,
                ..Platform::default()
            };
            let _ = app.system_message(Message::Platform(Box::new(enzimos)));
            let _ = app.update(Parent::Page(Category::Network));
            assert_eq!(app.page, Category::Network);
            assert!(app.network_elsewhere().is_none());
            app.page = Category::Power;
            assert!(built(&app).contains("power/mode"));
            app.page = Category::Apps;
            assert!(built(&app).contains("apps/background"));
        }
        // A page read that ends, finished or timed out, frees the reader and
        // starts the read of the page now shown, which waited behind it.
        //
        // Sabotage: leave `in_flight` set in the `ReadTimedOut` handler, or drop
        // `read_page_left_waiting`.
        {
            let (mut app, _directory) = app();
            app.page = Category::Sound;
            app.system.in_flight = Some(Panel::Network);
            assert!(
                app.system_message(Message::ReadTimedOut(Panel::Network))
                    .units()
                    > 0
            );
            assert_eq!(app.system.in_flight, Some(Panel::Sound));
            app.page = Category::Network;
            let finished = Reading::Sound(Default::default());
            assert!(
                app.system_message(Message::Loaded(Box::new(finished)))
                    .units()
                    > 0
            );
            assert_eq!(app.system.in_flight, Some(Panel::Network));
        }
        // The Display page shows while every output is off.
        //
        // Sabotage: drop the first-output fallback in `display_selected`.
        {
            use enzui_shell::system_settings::display::{Displays, Output};
            let (mut app, _directory) = app();
            app.system.readings.insert(
                Panel::Display,
                Reading::Display(Displays {
                    supported: true,
                    outputs: vec![Output {
                        name: "DP-1".into(),
                        enabled: false,
                        ..Output::default()
                    }],
                }),
            );
            app.page = Category::Display;
            assert!(built(&app).contains("display/resolution"));
        }
        // Backup job waits for repository read and starts once.
        {
            use enzui_shell::system_settings::{Backed, LongJob, Panel, Reading, enzimos::Safety};
            let directory = tempfile::tempdir().unwrap();
            let (mut app, _) =
                SettingsApp::new(Store::at(directory.path().join("settings.json")).unwrap());
            app.page = Category::Backups;
            app.system.in_flight = Some(Panel::Safety);
            let start = app.system_message(Message::StartJob(LongJob::BackupNow));
            assert_eq!(start.units(), 0);
            assert_eq!(app.system.deferred_job, Some(LongJob::BackupNow));
            assert_eq!(
                app.system.jobs.get(&LongJob::BackupNow),
                Some(&JobState::Running)
            );
            assert_eq!(
                app.system_message(Message::StartJob(LongJob::BackupNow))
                    .units(),
                0
            );
            let loaded = app.system_message(Message::Loaded(Box::new(Reading::Safety(
                Backed::Ready(Box::new(Safety::default())),
            ))));
            assert!(loaded.units() > 0);
            assert!(app.system.deferred_job.is_none());
            assert_eq!(
                app.system_message(Message::StartJob(LongJob::BackupNow))
                    .units(),
                0
            );
            assert_eq!(app.read_system(true).units(), 0);
        }
    }
}
