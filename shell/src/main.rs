//! Desktop main executable.
use enzui_shell::i18n::{self, tr};
mod assistant;
mod control_center;
#[path = "plugin_applications/hover.rs"]
mod dock_hover;
#[path = "plugin_applications/dock_ui.rs"]
mod dock_ui;
mod notification_panel;
mod phone_clipboard;
mod plugin_applications;
mod plugin_clipboard;
mod plugin_notifications;
mod plugin_panel;
mod quick_search;
mod region;
mod search_clipboard;
mod shell_accessibility;
mod shell_activity;
mod shell_capture;
mod shell_chrome;
mod shell_clock;
mod shell_commands;
mod shell_controls;
mod shell_diagnostics;
mod shell_emoji;
mod shell_media;
mod shell_messages;
mod shell_osd;
mod shell_osk;
mod shell_overview;
mod shell_panels;
mod shell_schedule;
mod shell_settings_io;
mod shell_settings_update;
mod shell_sidebar;
mod shell_switcher;
#[cfg(test)]
mod shell_tests;
mod shell_trace;
mod shell_view;
mod shell_window_settings;
mod shell_windows;
mod sidebar_view;
mod window_spread;
mod workspace_view;

use enzui_shell::{
    compositor::{Compositor, ControlGeometry, Snapshot},
    desktop_frame,
    settings::{
        BarEdge, Change as SettingChange, Metrics, Settings, SidebarEdge, Store as SettingsStore,
    },
    system_controls::{Action, ActionQueue, SystemControls, SystemSnapshot},
};
#[cfg(feature = "frame-report")]
use shell_diagnostics::frame_report;
use shell_diagnostics::message_trace;
use shell_messages::{ControlBodyKey, Message, Request, SetupErrors, SidebarConfiguration};
use shell_window_settings::{clock, resize_chrome, window_settings};

use iced::{Task, window};
use std::{collections::VecDeque, path::PathBuf, time::Instant};

struct Shell {
    dock: dock_ui::State,
    compositor: Compositor,
    plugin: plugin_panel::State,
    assistant: assistant::State,
    controllers: enzui_shell::controllers::Notifications,
    controllers_generation: u64,
    notification: notification_panel::State,
    activity: shell_activity::Activity,
    osd: shell_osd::Osd,
    glass_windows: std::collections::HashMap<window::Id, enzui_shell::surface_input::SurfaceInput>,
    glass_binding: std::collections::HashSet<window::Id>,
    glass_inks: std::collections::HashMap<window::Id, enzui_shell::glass_ink::AdaptiveInk>,
    glass_ink_ticking: bool,
    media: shell_media::Media,
    switcher: shell_switcher::Switcher,
    spread: window_spread::Spread,
    emoji: shell_emoji::EmojiPicker,
    capture: shell_capture::Capture,
    osk: shell_osk::Osk,
    clock_panel: shell_clock::ClockPanel,
    battery_warnings: enzui_shell::battery_warnings::Warnings,
    input_method: Option<bool>,
    privacy: enzui_shell::privacy::Usage,
    remote_sharing: Option<enzui_shell::remote_access::SharingGrant>,
    recording: Option<enzui_shell::recording::Recording>,
    // A recording starting on a worker; `true` once stopping was asked.
    recording_start: Option<bool>,
    night_light: Option<enzui_shell::night_light::NightLight>,
    night_light_target: u16,
    search: quick_search::State,
    root: PathBuf,
    panel: window::Id,
    launcher: Option<window::Id>,
    launcher_geometry: Option<ControlGeometry>,
    launcher_generation: u64,
    launcher_opening: bool,
    launcher_placing: bool,
    launcher_visible: bool,
    launcher_started: bool,
    launcher_focus_revision: u64,
    query: String,
    launching: bool,
    launch_inflight: Option<(u64, Option<window::Id>)>,
    snapshot: Option<Snapshot>,
    busy: bool,
    requests: VecDeque<Request>,
    light: bool,
    theme_document: enzui_core::theme_file::ThemeDocument,
    renderer_font: Option<iced::Font>,
    clock: String,
    quiet: bool,
    renderer_logged: bool,
    error: Option<String>,
    controls: SystemControls,
    system: SystemSnapshot,
    audio_devices: Option<Result<enzui_shell::audio_devices::Devices, String>>,
    audio_request: u64,
    system_busy: bool,
    system_action_busy: bool,
    system_queue: ActionQueue,
    control_center: Option<window::Id>,
    control_output: Option<String>,
    control_opening: bool,
    control_requested: bool,
    control_warming: bool,
    // A zero-progress frame precedes the first newly painted opening frame.
    control_frames: u8,
    control_placing: bool,
    control_reflow_pending: bool,
    control_body: Option<(ControlBodyKey, iced::Size)>,
    control_body_pending: Option<(window::Id, ControlBodyKey)>,
    control_body_retry: Option<Instant>,
    control_anchor: Option<iced::Rectangle>,
    control_generation: u64,
    control_visible: bool,
    control_started: bool,
    pending_surface_focus: std::collections::HashSet<window::Id>,
    pending_surface_requests: std::collections::HashSet<window::Id>,
    volume_draft: Option<u8>,
    brightness_draft: Option<u8>,
    confirm: Option<Action>,
    // Inhibitors block automatic power confirmation at the deadline.
    confirm_deadline: Option<(Action, Instant)>,
    confirm_inhibitors: Option<Vec<enzui_shell::inhibitors::Inhibitor>>,
    power_menu: bool,
    accessibility_menu: bool,
    // Orca state is unknown when it is not installed.
    screen_reader: Option<bool>,
    switch_user: bool,
    crashes_seen: Option<std::collections::HashSet<String>>,
    schedule: shell_schedule::Schedule,
    /// When an idle tick next fires; see [`shell_schedule`].
    next_tick_at: Instant,
    /// Follows the remote-sharing grant, so its indicator need not be polled.
    sharing_watch: Option<enzui_core::file_tree_watch::DirectoryWatchService>,
    session_restored: bool,
    session_forgotten: bool,
    // Reassert mirroring periodically so newly connected displays join it.
    mirror_applied: Option<bool>,
    focus_panels: std::collections::HashSet<window::Id>,
    screen_time_forgotten: bool,
    mirror_busy: bool,
    control_details: Option<enzui_shell::settings::QuickAction>,
    control_menu: Option<control_center::TileMenu>,
    desktop_modes_sent: Option<(bool, std::collections::BTreeMap<u32, bool>)>,
    control_details_open: bool,
    control_detail_epoch: u64,
    control_deferred_reflow: bool,
    // Wait for detail animation settlement before resizing the panel.
    control_detail_since: Option<Instant>,
    control_detail_capacity: f32,
    notice: Option<String>,
    setup_errors: SetupErrors,
    settings_shortcut_busy: bool,
    settings_shortcut_retry_at: Option<Instant>,
    drag_maximize_busy: bool,
    drag_maximize_ready: bool,
    drag_maximize_retry_at: Option<Instant>,
    drag_maximize_theme: Option<([u8; 3], u16)>,
    panel_action: Option<(window::Id, u64, Box<Message>)>,
    panel_action_serial: u64,
    /// A compositor event arrived while the snapshot was being read.
    snapshot_dirty: bool,
    /// The only window the message just handled changed, when it was one.
    redraw_only: Option<window::Id>,
    /// NetworkManager's signals deliver the network's devices.
    network_watched: bool,
    /// PipeWire's graph, followed for privacy, delivers the audio levels.
    audio_watched: bool,
    idle_inhibited: bool,
    idle_inhibition_ready: bool,
    settings_store: Option<SettingsStore>,
    settings_watch: Option<enzui_core::file_tree_watch::DirectoryWatchService>,
    settings_read_at: Option<Instant>,
    desired_settings: Settings,
    display_layouts: (
        std::sync::mpsc::Sender<Vec<enzui_shell::system_settings::display::Saved>>,
        Option<Vec<enzui_shell::system_settings::display::Saved>>,
    ),
    desktop_applied: Option<enzui_shell::desktop_preferences::CompositorPreferences>,
    preferences_busy: bool,
    desktop_retry_at: Option<Instant>,
    settings_read_busy: bool,
    settings_save_busy: bool,
    settings_generation: u64,
    /// The settings changed on disk while a read or save was running: read
    /// again once it ends (the watch reports a change once).
    settings_reread: bool,
    settings_queue: VecDeque<SettingChange>,
    bar: BarEdge,
    bar_ready: bool,
    bar_size: iced::Size,
    bar_busy: bool,
    // Keep the bar hidden throughout frame/island reconfiguration.
    bar_style_fade: bool,
    /// The frame fading out (`true`) or in since an instant, while the
    /// shell changes style.
    frame_fade: Option<(std::time::Instant, bool)>,
    /// When the sidebar's state was last read.
    sidebar_read_at: Option<std::time::Instant>,
    /// The rail's compositor fade: start, from and to; and its settled value.
    rail_fade: Option<(std::time::Instant, f32, f32)>,
    rail_alpha: f32,
    /// The rail's surface for its compositor fade, in either style (on the
    /// frame it has no input surface), and whether one is being acquired.
    rail_alpha_surface: Option<(window::Id, enzui_shell::surface_input::SurfaceInput)>,
    rail_alpha_pending: bool,
    bar_epoch: u64,
    bar_applied_epoch: u64,
    bar_geometry: Option<(u64, enzui_shell::bar_geometry::BarGeometry)>,
    // The bar epoch `bar_geometry` was measured in: serials restart when
    // the islands are rebuilt (after Frame), so they order only one epoch.
    bar_geometry_epoch: u64,
    bar_regions_sent: Option<(u64, u64)>,
    bar_regions_busy: bool,
    bar_regions_retry: Option<Instant>,
    surface_inputs: std::collections::HashMap<window::Id, enzui_shell::surface_input::SurfaceInput>,
    surface_globals: Option<enzui_shell::surface_input::Globals>,
    // When compositor glass is accepted, islands omit their own body fill.
    bar_glass: Option<enzui_shell::surface_input::SurfaceInput>,
    bar_glass_pending: bool,
    bar_glass_on: bool,
    frame_appearance: tokio::sync::watch::Sender<desktop_frame::Appearance>,
    sidebar: Option<enzui_shell::sidebar::Snapshot>,
    sidebar_rail: Option<window::Id>,
    sidebar_input_failed: bool,
    sidebar_focused: bool,
    sidebar_pane: Option<window::Id>,
    sidebar_busy: bool,
    sidebar_dirty: bool,
    sidebar_configured: Option<SidebarConfiguration>,
    metrics: Metrics,
    sidebar_queue: VecDeque<enzui_shell::sidebar::Action>,
    sidebar_scroll: f32,
    control_size: iced::Size,
    control_attachment: enzui_shell::reveal::Attachment,
    overview_size: iced::Size,
    overview_attachment: enzui_shell::reveal::Attachment,
    overview_prepared: Option<ControlGeometry>,
    overview_previews: Vec<enzui_shell::previews::Preview>,
    overview_desktops: Option<enzui_shell::workspaces::Desktops>,
    preview_dirty: bool,
    preview_failed: bool,
    preview_revision: u64,
    preview_cached_revision: Option<u64>,
    desktops: Option<enzui_shell::workspaces::Desktops>,
    desktop_busy: bool,
    desktop_generation: u64,
    desktop_dirty: bool,
    desktop_steps: i32,
    desktop_wheel: enzui_shell::workspaces::WheelNavigation,
    frame_lost: bool,
    outputs_checked: Option<Instant>,
    overview: Option<window::Id>,
    overview_opening: bool,
    overview_generation: u64,
    overview_visible: bool,
    overview_started: bool,
    previews: Vec<enzui_shell::previews::Preview>,
    preview_generation: u64,
    preview_busy: bool,
    /// Counts waits for windows to settle before previews are taken.
    preview_wait: u64,
    preview_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Shell {
    fn new(compositor: Compositor, root: PathBuf) -> (Self, Task<Message>) {
        let bar_width = std::env::var("ENZUI_SHELL_BAR_WIDTH")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|width| (240..=16384).contains(width))
            .map_or(1440.0, f32::from);
        let (panel, open) = window::open(window_settings("dev.enzui.Desktop", bar_width, 36.0));
        let settings_store = SettingsStore::from_environment();
        let light = false;
        let (frame_appearance, appearance) =
            tokio::sync::watch::channel(desktop_frame::Appearance::default());
        let frame_compositor = compositor.clone();
        let frame = Task::perform(
            async move {
                let output = frame_compositor.prepare_desktop_frame().await?;
                let executable = std::env::current_exe()
                    .map_err(|_| tr("shell-cannot-locate-desktop-frame"))?
                    .with_file_name("enzui-desktop-frame");
                desktop_frame::run(&executable, &output, appearance).await
            },
            Message::FrameStopped,
        );
        let mut shell = Self {
            dock: Default::default(),
            compositor,
            plugin: Default::default(),
            assistant: Default::default(),
            controllers: Default::default(),
            controllers_generation: 0,
            notification: Default::default(),
            activity: Default::default(),
            osd: Default::default(),
            glass_windows: std::collections::HashMap::new(),
            glass_binding: std::collections::HashSet::new(),
            glass_inks: std::collections::HashMap::new(),
            glass_ink_ticking: false,
            media: Default::default(),
            switcher: Default::default(),
            spread: Default::default(),
            emoji: Default::default(),
            capture: Default::default(),
            osk: Default::default(),
            clock_panel: Default::default(),
            battery_warnings: Default::default(),
            input_method: None,
            privacy: Default::default(),
            remote_sharing: Self::read_sharing_grant(),
            recording: None,
            recording_start: None,
            night_light: None,
            night_light_target: enzui_shell::night_light::NEUTRAL,
            search: Default::default(),
            root,
            panel,
            launcher: None,
            launcher_geometry: None,
            launcher_generation: 0,
            launcher_opening: false,
            launcher_placing: false,
            launcher_visible: false,
            launcher_started: false,
            launcher_focus_revision: 0,
            query: String::new(),
            launching: false,
            launch_inflight: None,
            snapshot: None,
            busy: false,
            requests: VecDeque::new(),
            light,
            renderer_font: None,
            theme_document: enzui_shell::theme_store::Store::read(
                &enzui_core::theme_file::ThemePaths::from_env(),
            )
            .into_document(),
            clock: clock(&Default::default()),
            quiet: false,
            renderer_logged: false,
            error: None,
            controls: SystemControls::from_environment(),
            system: SystemSnapshot::default(),
            audio_devices: None,
            audio_request: 0,
            system_busy: false,
            system_action_busy: false,
            system_queue: ActionQueue::default(),
            control_center: None,
            control_output: None,
            control_opening: false,
            control_requested: false,
            control_warming: false,
            control_frames: 0,
            control_placing: false,
            control_reflow_pending: false,
            control_body: None,
            control_body_pending: None,
            control_body_retry: None,
            control_anchor: None,
            control_generation: 0,
            control_visible: false,
            control_started: false,
            pending_surface_focus: std::collections::HashSet::new(),
            pending_surface_requests: std::collections::HashSet::new(),
            volume_draft: None,
            brightness_draft: None,
            confirm: None,
            confirm_deadline: None,
            confirm_inhibitors: None,
            power_menu: false,
            accessibility_menu: false,
            screen_reader: None,
            switch_user: false,
            crashes_seen: None,
            schedule: shell_schedule::Schedule::new(Instant::now()),
            next_tick_at: Instant::now() + shell_schedule::SLOW_TICK,
            sharing_watch: Self::watch_sharing_grant(),
            session_restored: false,
            session_forgotten: false,
            mirror_applied: None,
            focus_panels: std::collections::HashSet::new(),
            screen_time_forgotten: false,
            mirror_busy: false,
            control_details: None,
            control_menu: None,
            desktop_modes_sent: None,
            control_details_open: false,
            control_detail_epoch: 0,
            control_deferred_reflow: false,
            control_detail_since: None,
            control_detail_capacity: 0.0,
            notice: None,
            setup_errors: SetupErrors::default(),
            settings_shortcut_busy: false,
            settings_shortcut_retry_at: None,
            drag_maximize_busy: false,
            drag_maximize_ready: false,
            drag_maximize_retry_at: None,
            drag_maximize_theme: None,
            panel_action: None,
            panel_action_serial: 0,
            snapshot_dirty: false,
            redraw_only: None,
            network_watched: false,
            audio_watched: false,
            idle_inhibited: false,
            idle_inhibition_ready: enzui_shell::desktop_session::mode()
                != enzui_shell::desktop_session::Mode::Installed
                || enzui_shell::desktop_session::remote(),
            settings_watch: shell_settings_io::watch(settings_store.as_ref()),
            settings_read_at: None,
            settings_store,
            desired_settings: Settings::default(),
            display_layouts: {
                let (layouts, receiver) = std::sync::mpsc::channel();
                // Preserve the active layout while Settings awaits confirmation.
                let restore_initial =
                    std::env::var_os("ENZUI_KEEP_ACTIVE_DISPLAY_LAYOUT").is_none();
                std::thread::spawn(move || {
                    enzui_shell::system_settings::display::restore_on_connect(
                        receiver,
                        restore_initial,
                    );
                });
                (layouts, None)
            },
            desktop_applied: None,
            preferences_busy: false,
            desktop_retry_at: None,
            settings_read_busy: false,
            settings_save_busy: false,
            settings_generation: 0,
            settings_reread: false,
            settings_queue: VecDeque::new(),
            bar: BarEdge::Top,
            bar_ready: false,
            bar_size: iced::Size::new(bar_width, 36.0),
            bar_busy: false,
            bar_style_fade: false,
            frame_fade: None,
            sidebar_read_at: None,
            rail_fade: None,
            rail_alpha: 1.0,
            rail_alpha_surface: None,
            rail_alpha_pending: false,
            bar_epoch: 0,
            bar_applied_epoch: 0,
            bar_geometry: None,
            bar_geometry_epoch: 0,
            bar_regions_sent: None,
            bar_regions_busy: false,
            bar_regions_retry: None,
            surface_inputs: std::collections::HashMap::new(),
            surface_globals: None,
            bar_glass: None,
            bar_glass_pending: false,
            bar_glass_on: false,
            frame_appearance,
            sidebar: None,
            sidebar_rail: None,
            sidebar_input_failed: false,
            sidebar_focused: false,
            sidebar_pane: None,
            sidebar_busy: false,
            sidebar_dirty: false,
            sidebar_configured: None,
            metrics: Metrics::default(),
            sidebar_queue: VecDeque::new(),
            sidebar_scroll: 0.0,
            control_size: iced::Size::new(418.0, 538.0),
            control_attachment: Default::default(),
            overview_size: iced::Size::new(1440.0, 207.0),
            overview_attachment: Default::default(),
            overview_prepared: None,
            overview_previews: Vec::new(),
            overview_desktops: None,
            preview_dirty: true,
            preview_failed: false,
            preview_revision: 0,
            preview_cached_revision: None,
            desktops: None,
            desktop_busy: false,
            desktop_generation: 0,
            desktop_dirty: false,
            desktop_steps: 0,
            desktop_wheel: Default::default(),
            frame_lost: false,
            outputs_checked: None,
            overview: None,
            overview_opening: false,
            overview_generation: 0,
            overview_visible: false,
            overview_started: false,
            previews: Vec::new(),
            preview_generation: 0,
            preview_busy: false,
            preview_wait: 0,
            preview_cancel: None,
        };
        let task = Task::batch([
            frame,
            shell.refresh(None),
            shell.refresh_system(None),
            shell.refresh_desktops(None),
            shell.read_settings(),
            shell.settings_shortcut(),
            shell.drag_maximize(),
            open.map(Message::Opened),
        ]);
        (shell, task)
    }

    // Every update: a problem recorded in `notice` is shown as a notification.
    fn update_and_report(&mut self, message: Message) -> Task<Message> {
        let task = self.update(message);
        match self.notice.take() {
            Some(text) if !text.trim().is_empty() => Task::batch([
                task,
                Task::perform(
                    enzui_shell::backup_notices::post_shell_notice(text),
                    Message::NoticePosted,
                ),
            ]),
            _ => task,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.quiet = false;
        self.redraw_only = None;
        let traced = message_trace().map(|_| message.trace_kind());
        if traced.is_some()
            && let Message::DesktopsRead(generation, result) = &message
        {
            eprintln!(
                "enzui-shell desktop-read: generation={generation} accepted={} known_before={} success={} count={} dirty={} steps={}",
                *generation == self.desktop_generation,
                self.desktops.is_some(),
                result.is_ok(),
                result.as_ref().map_or(0, |desktops| desktops.entries.len()),
                self.desktop_dirty,
                self.desktop_steps
            );
        }
        let task = self.update_message(message);
        let task = Task::batch([task, self.dock_sync(), self.dock_publish()]);
        self.update_frame();
        let assistant = self.assistant_sync();
        let quiet = self.quiet && assistant.units() == 0;
        if let (Some(trace), Some(kind)) = (message_trace(), traced) {
            trace.record(kind, quiet);
        }
        if quiet {
            return Task::batch([task, window::invalidate(window::Invalidation::none())]);
        }
        if let Some(id) = self.redraw_only.take() {
            return Task::batch([
                task,
                assistant,
                window::invalidate(window::Invalidation::only(id)),
            ]);
        }
        Task::batch([task, assistant])
    }
    fn update_message(&mut self, message: Message) -> Task<Message> {
        let message = match message {
            Message::PanelActionReady(serial, next) => {
                if serial != self.panel_action_serial {
                    return Task::none();
                }
                self.cancel_panel_action();
                *next
            }
            message => message,
        };
        if matches!(
            &message,
            Message::ToggleControls | Message::ToggleOverview | Message::ShowLauncher
                | Message::QuickSearch(quick_search::Message::Toggle | quick_search::Message::Show(_))
                | Message::Launch(_) | Message::SettingsPage(_) | Message::SettingsPlugin(_)
                | Message::Command(enzui_shell::commands::Command::SwitchNext
                    | enzui_shell::commands::Command::SwitchPrevious
                    | enzui_shell::commands::Command::SwitchAppNext)
        ) || self.panel_action.as_ref().is_some_and(|(owner, _, _)| {
            matches!(&message, Message::Escape(id) | Message::Window(id, window::Event::CloseRequested | window::Event::Closed) if id == owner)
        }) {
            self.cancel_panel_action();
        }
        if matches!(&message, Message::ShowLauncher) {
            self.launcher_focus_revision = self.launcher_focus_revision.wrapping_add(1);
        }
        let measured = matches!(&message, Message::ControlBodyMeasured(..));
        let plugin_close = if matches!(
            &message,
            Message::ToggleControls | Message::ToggleOverview | Message::ShowLauncher
        ) {
            self.force_close_plugin()
        } else {
            Task::none()
        };
        let notification_close = if matches!(
            &message,
            Message::ToggleControls | Message::ToggleOverview | Message::ShowLauncher
        ) {
            self.force_close_notifications()
        } else {
            Task::none()
        };
        let search_close = if matches!(&message, Message::ToggleControls | Message::ToggleOverview)
        {
            self.close_search()
        } else {
            Task::none()
        };
        let task = match message {
            Message::Dock(event) => self.dock_event(event),
            Message::ApplicationSurface(event) => self.application_surface_event(event),
            Message::PanelActionReady(_, _) => Task::none(),
            Message::ControlBodyRetry => Task::none(),
            Message::SettingsChanged => {
                shell_settings_io::watch_plugins(
                    self.settings_watch.as_ref(),
                    self.settings_store.as_ref(),
                );
                self.quiet = true;
                if let Some(channel) = &self.plugin.channel {
                    let _ = channel.try_send(enzui_shell::plugins::service::Command::Refresh);
                }
                if self.settings_read_busy || self.settings_save_busy {
                    // An atomic replace reports more than once; the read
                    // already running may have seen the old file.
                    self.settings_reread = true;
                    Task::none()
                } else {
                    self.read_settings()
                }
            }
            Message::QuickSearch(message) => self.search_message(message),
            Message::Notification(message) => self.notification_message(message),
            Message::Assistant(message) => self.assistant_message(message),
            Message::Controllers(generation, list) => self.controllers_changed(generation, list),
            Message::ControllerNotified(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: controller notification: {error}");
                }
                Task::none()
            }
            Message::Connectivity(connectivity) => {
                if let Some(devices) = connectivity.devices {
                    self.system.network = devices.into();
                    self.network_watched = true;
                }
                self.system.network.connectivity = connectivity.state;
                Task::none()
            }
            Message::Activity(message) => self.activity_message(message),
            Message::Command(command) => self.run_command(command),
            Message::Osd(message) => self.osd_message(message),
            Message::GlassBound(id, result) => self.glass_bound(id, result),
            Message::GlassBackdrop(tick) => self.glass_backdrop(tick),
            Message::Media(message) => self.media_message(message),
            Message::Switcher(message) => self.switcher_message(message),
            Message::Spread(message) => self.spread_message(message),
            Message::Emoji(message) => self.emoji_message(message),
            Message::Capture(message) => self.capture_message(message),
            Message::Osk(message) => self.osk_message(message),
            Message::Clock(message) => self.clock_message(message),
            Message::ToggleOsk => self.toggle_osk(),
            Message::EmojiKey(id, key) if self.emoji.owns(id) => self.emoji_key(&key),
            Message::EmojiKey(id, key) if self.capture.owns(id) => self.capture_key(&key),
            Message::EmojiKey(..) => Task::none(),
            Message::BackupCheck => {
                Task::perform(enzui_shell::backup_notices::check(), Message::BackupChecked)
            }
            Message::NoticePosted(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: notice: {error}");
                }
                Task::none()
            }
            Message::BackupChecked(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: backup status: {error}");
                }
                Task::none()
            }
            Message::BatteryWarned(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: battery warning: {error}");
                }
                Task::none()
            }
            Message::Drive(enzui_shell::drives::Event::Open(folder)) => {
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.open_folder(&folder).await },
                    Message::DriveOpened,
                )
            }
            Message::Drive(enzui_shell::drives::Event::Error(error))
            | Message::DriveOpened(Err(error)) => {
                eprintln!("enzui-shell: drives: {error}");
                Task::none()
            }
            Message::DriveOpened(Ok(())) => Task::none(),
            Message::Inhibitors(action, locks) => {
                if self.confirm.as_ref() == Some(&action) {
                    self.confirm_inhibitors = Some(enzui_shell::inhibitors::relevant(
                        &locks,
                        action == Action::LogOut,
                    ));
                }
                Task::none()
            }
            Message::Crashes(Some(reports)) => {
                let fresh: Vec<_> = match &mut self.crashes_seen {
                    Some(seen) => reports
                        .into_iter()
                        .filter(|report| seen.insert(report.key()))
                        .collect(),
                    None => {
                        self.crashes_seen = Some(
                            reports
                                .iter()
                                .map(enzui_shell::crash_reports::Report::key)
                                .collect(),
                        );
                        Vec::new()
                    }
                };
                // Coalesce crash-loop notifications.
                Task::batch(fresh.into_iter().rev().take(3).map(|report| {
                    Task::perform(
                        async move { enzui_shell::crash_reports::notify(&report).await },
                        Message::CrashNotified,
                    )
                }))
            }
            Message::Crashes(None) | Message::CrashNotified(Ok(())) => Task::none(),
            Message::SessionApps(Ok(())) => Task::none(),
            Message::FocusReturned(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: focus: {error}");
                }
                Task::none()
            }
            Message::ScreenTimeRecorded(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: screen time: {error}");
                }
                Task::none()
            }
            Message::ParentalFilter(filter) => {
                let changed = enzui_shell::parental::current() != filter;
                enzui_shell::parental::set_current(filter);
                if changed {
                    self.search_message(quick_search::Message::Refresh)
                } else {
                    Task::none()
                }
            }
            Message::Mirrored(on, result) => {
                self.mirror_busy = false;
                match result {
                    Ok(()) => self.mirror_applied = Some(on),
                    Err(error) => eprintln!("enzui-shell: mirroring: {error}"),
                }
                Task::none()
            }
            Message::SessionApps(Err(error)) => {
                eprintln!("enzui-shell: reopening apps: {error}");
                Task::none()
            }
            Message::CrashNotified(Err(error)) => {
                eprintln!("enzui-shell: crash report: {error}");
                Task::none()
            }
            Message::SwitchUserAvailable(available) => {
                self.switch_user = available;
                Task::none()
            }
            Message::SwitchUser => {
                self.power_menu = false;
                let close = self.force_close_controls();
                Task::batch([
                    close,
                    Task::perform(enzui_shell::user_switch::switch(), Message::SwitchedUser),
                ])
            }
            Message::SwitchedUser(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::RecordingTick => self.recording_tick(),
            Message::RecordOutput(output) => match output {
                Ok(output) => self.start_recording(enzui_shell::recording::Target::Output(output)),
                Err(error) => {
                    self.notice = Some(error);
                    Task::none()
                }
            },
            Message::RecordingStarted(started) => self.recording_started(started),
            Message::StopRecording => {
                let stop = self.stop_recording();
                Task::batch([stop, self.return_app_focus()])
            }
            Message::RecordingSaved(ending) => {
                self.notice = Some(ending.notice());
                Task::none()
            }
            Message::Privacy(usage) => {
                if usage.output.is_some() {
                    self.system.audio = usage.output;
                    self.system.microphone = usage.input;
                    self.audio_watched = true;
                }
                self.privacy = usage;
                Task::none()
            }
            Message::InputMethodApplied(result) => {
                if let Err(error) = result {
                    eprintln!("enzui-shell: {error}");
                }
                Task::none()
            }
            Message::Plugin(message) => self.plugin_message(message),
            Message::SidebarAction(action) => self.refresh_sidebar(Some(action)),
            Message::SidebarRead(placement, result) => self.sidebar_read(placement, result),
            Message::Tiling(enabled) => match self.snapshot.as_ref() {
                Some(snapshot) => self.save_setting(SettingChange::DesktopTiling(
                    snapshot.workspace,
                    Some(enabled),
                )),
                None => Task::none(),
            },
            Message::TilingDefault(enabled) => self.save_setting(SettingChange::Tiling(enabled)),
            Message::TilingFollowDefault => match self.snapshot.as_ref() {
                Some(snapshot) => {
                    self.save_setting(SettingChange::DesktopTiling(snapshot.workspace, None))
                }
                None => Task::none(),
            },
            Message::ControlMenuOpen(action, at) => {
                self.control_menu = control_center::TileMenu::open(self, action, at);
                Task::none()
            }
            Message::ControlMenu(event) => self.control_menu_event(event),
            Message::QuickSetting(change) => self.save_setting(change),
            Message::SwitchLayout(index) => {
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.switch_layout(index).await },
                    Message::LayoutSwitched,
                )
            }
            Message::LayoutSwitched(result) => match result {
                Ok(()) => self.refresh(None),
                Err(error) => {
                    self.notice = Some(error);
                    Task::none()
                }
            },
            Message::ScreenReader => self.toggle_screen_reader(),
            Message::ScreenReaderState(result, state) => {
                self.screen_reader = state;
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
            Message::ReduceMotion(reduced) => self.set_reduced_motion(reduced),
            Message::HideLauncher => self.close_launcher(),
            Message::RegionInterior => Task::none(),
            msg @ (Message::Window(..) | Message::Opened(..) | Message::Escape(..)) => {
                if let Message::Window(id, window::Event::Closed) = &msg {
                    self.glass_windows.remove(id);
                    self.glass_binding.remove(id);
                    self.glass_inks.remove(id);
                }
                // Closed callbacks may arrive after panel state has been retired.
                // Cached controls restore focus when parked, including forced teardown.
                let is_panel = |shell: &Self, id: window::Id| {
                    Some(id) == shell.overview
                        || Some(id) == shell.launcher
                        || shell.notification.owns_center(id)
                        || shell.assistant.owns_panel(id)
                };
                let popup_closed = match &msg {
                    Message::Window(id, window::Event::Closed) => self.focus_panels.remove(id),
                    _ => false,
                };
                let task = self.update_windows(msg.clone());
                if let Message::Opened(id) = &msg
                    && is_panel(self, *id)
                {
                    self.focus_panels.insert(*id);
                }
                if popup_closed {
                    task.chain(self.return_app_focus())
                } else {
                    task
                }
            }
            msg @ (Message::SettingsPage(..)
            | Message::SettingsPlugin(..)
            | Message::SettingsLaunched(..)
            | Message::SettingsLoaded(..)
            | Message::SettingsSaved(..)
            | Message::SettingsShortcutReady(..)
            | Message::SidebarSurfaceReady(..)
            | Message::BarGlassReady(..)
            | Message::SurfaceInputReady(..)
            | Message::BarGeometry(..)
            | Message::BarRegionsApplied(..)
            | Message::BarApplied(..)
            | Message::BarStyleHidden
            | Message::FrameFadeTick
            | Message::RailAlphaTick
            | Message::RailAlphaSurface(..)
            | Message::DesktopPreferencesApplied(..)) => self.update_settings(msg),
            msg @ (Message::ControlsFrame(..)
            | Message::ControlsWarmMeasured(..)
            | Message::ControlsWarmed(..)
            | Message::ControlsWarmHidden(..)
            | Message::WarmControls
            | Message::ToggleControls
            | Message::ToggleAccessibility
            | Message::ToggleOverview
            | Message::ShowLauncher
            | Message::SettingsTick
            | Message::FrameStopped(..)
            | Message::OutputsChecked(..)
            | Message::DesktopChanged
            | Message::SidebarScroll(..)
            | Message::DesktopsRead(..)
            | Message::DesktopWheel(..)
            | Message::DismissRegions
            | Message::OverviewHidden(..)
            | Message::ControlsHidden(..)
            | Message::OverviewAnchor(..)
            | Message::PreviewsReady(..)
            | Message::PreviewsDue(..)
            | Message::OverviewPrepared(..)
            | Message::SelectDesktop(..)
            | Message::Tick
            | Message::SharingChanged
            | Message::LauncherAnchor(..)
            | Message::LauncherPrepared(..)
            | Message::LauncherHidden(..)
            | Message::LauncherFocusChecked(..)
            | Message::LauncherFocused(..)
            | Message::Search(..)
            | Message::Launch(..)
            | Message::Launched(..)
            | Message::ToggleTheme
            | Message::DragMaximizeReady(..)
            | Message::ControlsAnchor(..)
            | Message::ControlsPrepared(..)
            | Message::ControlsReflowed(..)
            | Message::ControlBodyMeasured(..)
            | Message::Volume(..)
            | Message::Brightness(..)
            | Message::RefreshAudioDevices
            | Message::AudioDevices(..)
            | Message::SystemAction(..)
            | Message::Idle(..)
            | Message::IdleInhibited(..)
            | Message::PowerPolicy(..)
            | Message::ControlSettings
            | Message::ControlDetailsSettled(..)
            | Message::ControlDetails(..)
            | Message::Confirm(..)
            | Message::CancelConfirm
            | Message::ShowPower
            | Message::SystemRefreshed(..)
            | Message::Screenshot
            | Message::CaptureScreenshot
            | Message::Dictate
            | Message::ToggleDictation
            | Message::Refreshed(..)
            | Message::FocusSidebar(..)
            | Message::Renderer(..)) => self.update_chrome(msg),
        };
        self.update_frame();
        let search_layout = self.search_sync_layout();
        let desktop_settings = self.sync_desktop_preferences();
        // Measurement results must not trigger another measurement loop.
        let control_measurement = if measured || self.quiet {
            Task::none()
        } else {
            self.measure_control_body()
        };
        let tokens = self.base_tokens();
        let (scale, _) = enzui_theme::text_scale::TextScale::admit(self.metrics.text_scale());
        let scaled = enzui_theme::text_scale::publish(scale);
        let published = enzui_shell::font_settings::publish_text_defaults(&tokens) | scaled;
        let font = enzui_shell::font_settings::renderer_default(&tokens);
        let renderer_font = if self.renderer_font == Some(font) && !published {
            Task::none()
        } else {
            self.renderer_font = Some(font);
            iced::font::set_defaults(
                font,
                enzui_shell::font_settings::renderer_text_size(&tokens),
            )
        };
        if [
            &renderer_font,
            &control_measurement,
            &desktop_settings,
            &search_layout,
            &plugin_close,
            &notification_close,
            &search_close,
        ]
        .iter()
        .any(|task| task.units() != 0)
        {
            self.quiet = false;
        }
        Task::batch([
            renderer_font,
            control_measurement,
            desktop_settings,
            search_layout,
            plugin_close,
            notification_close,
            search_close,
            task,
        ])
    }
}

fn main() -> iced::Result {
    i18n::initialize();
    #[cfg(feature = "frame-report")]
    frame_report();
    let compositor = Compositor::from_environment().unwrap_or_else(|error| {
        eprintln!("enzui-shell: {error}");
        std::process::exit(2);
    });
    let root = std::env::var_os("ENZUI_DESKTOP_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            eprintln!("enzui-shell: missing desktop installation root");
            std::process::exit(2);
        });
    // Nested previews leave the host session's polkit agent in control.
    if !enzui_shell::desktop_session::mode().nested()
        && let Some(agent) = std::env::var_os("ENZUI_SHELL_BIN_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
            .map(|dir| dir.join("enzui-polkit-agent"))
            .filter(|path| path.is_file())
    {
        enzui_shell::polkit_agent::supervise(agent);
    }
    // Nested previews leave the host NetworkManager secret agent in control.
    if !enzui_shell::desktop_session::mode().nested()
        && let Some(secrets) = std::env::var_os("ENZUI_SHELL_BIN_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
            .map(|dir| dir.join("enzui-network-secrets"))
            .filter(|path| path.is_file())
    {
        enzui_shell::polkit_agent::supervise(secrets);
    }
    // Location stays refused for every app until the agent allows it.
    if !enzui_shell::desktop_session::mode().nested()
        && let Some(location) = std::env::var_os("ENZUI_SHELL_BIN_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
            .map(|dir| dir.join("enzui-location-agent"))
            .filter(|path| path.is_file())
    {
        enzui_shell::polkit_agent::supervise(location);
    }
    // MCP Center denies unanswered requests even when the consent dialog is unavailable.
    if enzui_shell::desktop_session::mode().managed()
        && let Some(consent) = std::env::var_os("ENZUI_SHELL_BIN_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok()?.parent().map(PathBuf::from))
            .map(|dir| dir.join("enzui-agent-consent"))
            .filter(|path| path.is_file())
    {
        enzui_shell::polkit_agent::supervise(consent);
    }
    enzui_widgets::app_setup::run_with_services(
        iced::daemon(
            move || Shell::new(compositor.clone(), root.clone()),
            Shell::update_and_report,
            Shell::view,
        )
        .title(Shell::title)
        .theme(Shell::theme)
        .style(Shell::style)
        .subscription(Shell::subscription),
        enzui_widgets::app_setup::AppServices::default().localizer(i18n::localizer().clone()),
    )
}
