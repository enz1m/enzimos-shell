//! Desktop messages and their shared payloads.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SidebarConfiguration {
    pub(super) side: SidebarEdge,
    pub(super) bar: BarEdge,
    pub(super) metrics: Metrics,
    pub(super) motion: enzui_shell::motion::Policy,
    // Part of what is configured, so changing one of them alone reaches
    // the compositor: without them "Show miniapp sidebar", its visibility
    // and the pin scope took effect only at the next start.
    pub(super) enabled: bool,
    pub(super) behavior: enzui_shell::sidebar_preferences::Preferences,
    pub(super) pin_scope: enzui_shell::sidebar_preferences::PinScope,
}

#[derive(Debug, Clone)]
pub(super) enum Message {
    Dock(dock_ui::Event),
    ApplicationSurface(plugin_applications::Event),
    QuickSearch(quick_search::Message),
    Connectivity(enzui_shell::network_connectivity::Connectivity),
    Controllers(u64, Vec<enzui_shell::controllers::Controller>),
    ControllerNotified(Result<(), String>),
    Assistant(assistant::Message),
    Notification(notification_panel::Message),
    Activity(shell_activity::ActivityMessage),
    Command(enzui_shell::commands::Command),
    Osd(shell_osd::OsdMessage),
    GlassBound(
        window::Id,
        Result<enzui_shell::surface_input::SurfaceInput, String>,
    ),
    // Backdrop measurement or adaptive-ink animation wake.
    GlassBackdrop(bool),
    Media(shell_media::MediaMessage),
    Switcher(shell_switcher::SwitcherMessage),
    Spread(window_spread::SpreadMessage),
    Emoji(shell_emoji::EmojiMessage),
    Capture(shell_capture::CaptureMessage),
    Osk(shell_osk::OskMessage),
    Clock(shell_clock::ClockMessage),
    ToggleOsk,
    EmojiKey(window::Id, iced::keyboard::Key),
    BatteryWarned(Result<(), String>),
    BackupCheck,
    BackupChecked(Result<(), String>),
    NoticePosted(Result<(), String>),
    InputMethodApplied(Result<(), String>),
    Drive(enzui_shell::drives::Event),
    Privacy(enzui_shell::privacy::Usage),
    RecordingTick,
    RecordOutput(Result<String, String>),
    StopRecording,
    RecordingSaved(enzui_shell::recording::Ending),
    RecordingStarted(Result<enzui_shell::recording::Started, String>),
    DriveOpened(Result<(), String>),
    Plugin(plugin_panel::Message),
    Tick,
    /// The remote-sharing grant folder changed.
    SharingChanged,
    SettingsTick,
    SidebarAction(enzui_shell::sidebar::Action),
    SidebarScroll(f32),
    SidebarRead(
        Option<SidebarConfiguration>,
        Result<enzui_shell::sidebar::Snapshot, String>,
    ),
    DesktopsRead(u64, Result<enzui_shell::workspaces::Desktops, String>),
    DesktopWheel(iced::mouse::ScrollDelta),
    DesktopChanged,
    ToggleOverview,
    OverviewHidden(window::Id),
    OverviewAnchor(u64, Option<iced::Rectangle>),
    LauncherAnchor(u64, Option<iced::Rectangle>),
    LauncherPrepared(u64, Result<ControlGeometry, String>),
    LauncherHidden(window::Id),
    LauncherFocusChecked(window::Id, u64, bool),
    LauncherFocused(window::Id, u64, Result<(), String>),
    ControlsHidden(window::Id),
    DismissRegions,
    RegionInterior,
    OverviewPrepared(u64, Result<ControlGeometry, String>),
    PreviewsReady(
        u64,
        u64,
        Result<Vec<enzui_shell::previews::Preview>, String>,
    ),
    /// The windows settled since previews were last wanted (see
    /// `start_previews`); carries the wait it ends.
    PreviewsDue(u64),
    OverviewFrame(window::Id),
    OverviewRevealCheck(u64),
    SelectDesktop(u32),
    FrameStopped(Result<(), String>),
    OutputsChecked(enzui_shell::compositor::OutputChange),
    /// The chrome has faded out for an announced scale change.
    RescaleHidden(u64),
    /// Keeps the compositor drawing until an announced change is applied.
    RescaleFrames(u64),
    /// Checks whether the output has settled after a scale change.
    RescaleSettle(u64),
    /// The output after a scale change, and whether the compositor has
    /// settled it.
    RescaleChecked(u64, enzui_shell::compositor::OutputChange, bool),
    /// An announced scale change has taken too long: the chrome comes back.
    RescaleLimit(u64),
    Tiling(bool),
    TilingDefault(bool),
    TilingFollowDefault,
    ControlMenuOpen(enzui_shell::settings::QuickAction, iced::Point),
    ControlMenu(enzui_widgets::context_menu_host::ContextMenuEvent),
    QuickSetting(SettingChange),
    SwitchLayout(Option<usize>),
    LayoutSwitched(Result<(), String>),
    ToggleAccessibility,
    ScreenReader,
    ScreenReaderState(Result<(), String>, Option<bool>),
    ReduceMotion(bool),
    ShowLauncher,
    HideLauncher,
    Search(String),
    Launch(&'static str),
    SettingsPage(&'static str),
    SettingsPlugin(String),
    PanelActionReady(u64, Box<Message>),
    Launched(u64, Option<window::Id>, Result<(), String>),
    ToggleTheme,
    ToggleControls,
    WarmControls,
    ControlsWarmed(window::Id),
    ControlsFrame(window::Id),
    ControlsWarmMeasured(window::Id, Option<iced::Size>),
    ControlsWarmHidden(window::Id),
    ControlsPrepared(u64, Result<ControlGeometry, String>),
    ControlsReflowed(u64, Result<ControlGeometry, String>),
    ControlsAnchor(u64, Option<iced::Rectangle>),
    SettingsShortcutReady(Result<(), String>),
    SettingsLaunched(u64, Result<(), String>),
    DragMaximizeReady(Result<(), String>),
    ControlConnections(super::control_center::connections::Event),
    RefreshAudioDevices,
    AudioDevices(u64, Result<enzui_shell::audio_devices::Devices, String>),
    SystemAction(Action),
    Idle(enzui_shell::idle::Config, enzui_shell::idle::Notice),
    IdleInhibited(Result<bool, String>),
    PowerPolicy(String),
    SystemRefreshed(Option<Action>, Result<(), String>, SystemSnapshot),
    Volume(f32),
    Brightness(f32),
    Confirm(Action),
    Inhibitors(Action, Vec<enzui_shell::inhibitors::Inhibitor>),
    ControlDetails(Option<enzui_shell::settings::QuickAction>),
    ControlDetailsSettled(u64, u64),
    ControlSettings,
    CancelConfirm,
    ShowPower,
    SwitchUserAvailable(bool),
    Crashes(Option<Vec<enzui_shell::crash_reports::Report>>),
    SessionApps(Result<(), String>),
    Mirrored(bool, Result<(), String>),
    ParentalFilter(Option<enzui_shell::parental::AppFilter>),
    ScreenTimeRecorded(Result<(), String>),
    FocusReturned(Result<(), String>),
    CrashNotified(Result<(), String>),
    SwitchUser,
    SwitchedUser(Result<(), String>),
    Screenshot,
    CaptureScreenshot,
    /// Start or stop a hands-free dictation take, once the panel has closed
    /// and the window being typed in has focus again.
    Dictate,
    ToggleDictation,
    SettingsLoaded(u64, Result<Settings, String>),
    SettingsSaved(Result<Settings, String>),
    DesktopPreferencesApplied(
        enzui_shell::desktop_preferences::CompositorPreferences,
        Result<(), String>,
    ),
    BarApplied(BarEdge, Metrics, Result<ControlGeometry, String>),
    BarStyleHidden,
    /// The frame fades while the shell changes style.
    FrameFadeTick,
    /// The rail's compositor fade takes its next step.
    RailAlphaTick,
    /// The rail's surface for its compositor fade.
    RailAlphaSurface(
        iced::window::Id,
        Result<enzui_shell::surface_input::SurfaceInput, String>,
    ),
    Refreshed(Result<Snapshot, String>),
    SidebarSurfaceReady(
        window::Id,
        Result<enzui_shell::surface_input::SurfaceInput, String>,
    ),
    BarGlassReady(Result<enzui_shell::surface_input::SurfaceInput, String>),
    SurfaceInputReady(
        window::Id,
        Result<enzui_shell::surface_input::SurfaceInput, String>,
    ),
    FocusSidebar(window::Id),
    BarGeometry(u64, u64, enzui_shell::bar_geometry::BarGeometry),
    BarRegionsApplied(u64, u64, Result<(), String>),
    ControlBodyMeasured(window::Id, ControlBodyKey, Option<iced::Size>),
    ControlBodyRetry,
    SettingsChanged,
    Opened(window::Id),
    Window(window::Id, window::Event),
    Escape(window::Id),
    Renderer(String, String),
}

#[derive(Debug, Clone)]
pub(super) enum Request {
    DesktopModes(bool, std::collections::BTreeMap<u32, bool>),
}

#[derive(Default)]
pub(super) struct SetupErrors {
    pub(super) bar: Option<String>,
    pub(super) preferences: Option<String>,
    pub(super) shortcut: Option<String>,
    pub(super) sidebar: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ControlBodyKey {
    pub(super) generation: u64,
    pub(super) width: f32,
}
impl SetupErrors {
    pub(super) fn message(&self) -> Option<&str> {
        self.bar
            .as_deref()
            .or(self.preferences.as_deref())
            .or(self.shortcut.as_deref())
    }
}
