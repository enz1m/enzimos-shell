use enzui_shell::i18n::tr;
#[path = "app_style_settings.rs"]
mod app_style_settings;
#[path = "assistant_settings.rs"]
mod assistant_settings;
#[path = "settings_background.rs"]
mod background;
#[path = "background_settings.rs"]
mod background_settings;
#[path = "capture_settings.rs"]
mod capture_settings;
#[path = "settings_components.rs"]
mod components;
#[path = "control_editor.rs"]
mod control_editor;
#[path = "settings_dev_tools.rs"]
mod dev_tools;
#[path = "settings_fonts.rs"]
mod fonts;
#[path = "local_ai_settings.rs"]
mod local_ai_settings;
#[path = "settings_lock.rs"]
mod lock_settings;
#[path = "settings_navigation.rs"]
mod navigation;
#[path = "notifications_settings.rs"]
mod notifications_settings;
#[path = "settings_numeric.rs"]
mod numeric;
#[path = "settings_pages.rs"]
mod pages;
#[path = "plugins_settings.rs"]
mod plugins_settings;
#[path = "settings_previews.rs"]
mod previews;
#[path = "quick_search_settings.rs"]
mod quick_search_settings;
#[path = "sandbox_settings.rs"]
mod sandbox_settings;
#[path = "settings_sections.rs"]
mod sections;
mod settings_app_center;
#[cfg(test)]
mod settings_app_tests;
mod settings_app_update;
mod settings_app_view;
#[path = "settings_file_chooser.rs"]
mod settings_file_chooser;
#[path = "settings_startup.rs"]
mod settings_startup;
#[path = "shortcuts_settings.rs"]
mod shortcuts_settings;
mod system_pages;
#[path = "theme_preview.rs"]
mod theme_preview;
#[path = "theme_settings.rs"]
mod theme_settings;
#[path = "settings_users.rs"]
mod users;
#[path = "wallpaper_picker.rs"]
mod wallpaper_picker;
mod wallpaper_settings;

thread_local! {
    static ROW_ICON_SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

pub(crate) fn settings_row<'a, M: Clone + 'a>(
    title: impl Into<std::borrow::Cow<'a, str>>,
) -> enzui_widgets::preferences_row::EnzPreferencesRow<'a, M> {
    enzui_widgets::preferences_row::enz_preferences_row(title).tokens(settings_row_tokens())
}

pub(crate) fn settings_row_tokens() -> enzui_widgets::preferences_row::PreferencesRowTokens {
    let mut tokens = enzui_widgets::preferences_row::PreferencesRowTokens::default();
    tokens.icon_size =
        (tokens.icon_size * ROW_ICON_SCALE.with(std::cell::Cell::get)).clamp(12.0, 72.0);
    tokens.icon_slot = tokens.icon_slot.max(tokens.icon_size + 6.0);
    tokens
}
use enzui_shell::settings::{Change, Settings, Store};
use enzui_shell::settings_catalog::Category as Page;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    app_shell::{ShellLayout, enz_app_shell, enz_window_settings},
    nav_split::NavSplitState,
    sidebar_content::enz_sidebar_content,
    sidebar_row::enz_sidebar_row,
    title_bar::enz_title_bar,
    title_bar_gesture::WindowCommand,
};
use iced::{Element, Subscription, Task, window};
use std::{collections::VecDeque, time::Duration};

#[derive(Clone, Copy, Debug, Default)]
enum ImagePurpose {
    #[default]
    Desktop,
    Lock,
    Avatar(u32),
}

const SEARCH_FIELD: &str = "settings-find";

#[derive(Clone, Debug)]
enum Message {
    FileChooser(settings_file_chooser::Message),
    Startup(settings_startup::Message),
    Controls(control_editor::Message),
    Numeric(numeric::Message),
    SearchSettings(quick_search_settings::Message),
    Wallpaper(wallpaper_settings::Message),
    System(system_pages::Message),
    WallpaperFrame,
    WallpaperPicker(wallpaper_picker::Message),
    PreviewNotification,
    PreviewNotificationSent(Result<(), String>),
    DesktopStatus(enzui_shell::motion::Status),
    Plugin(plugins_settings::Message),
    Shortcuts(enzui_widgets::editable_shortcuts::EditableShortcutsEvent),
    Sandbox(sandbox_settings::Message),
    Background(background_settings::Message),
    Assistant(assistant_settings::Message),
    LocalAi(local_ai_settings::Message),
    Noop,
    Page(Page),
    Category(enzui_shell::settings_catalog::Category, bool),
    Collapse,
    Section(&'static str),
    SectionLocated(u64, Option<navigation::Position>),
    PageScrolled,
    SectionObserved(u64, Option<(&'static str, f32)>),
    InterruptScroll(navigation::Interruption),
    FindSetting,
    Filter(String),
    Reveal(&'static str),
    RevealFrame,
    Opened(window::Id),
    Viewport(window::Id, iced::Size),
    Menu(String, bool),
    Sidebar(NavSplitState),
    Change(Change),
    Layout(enzui_shell::settings_layout::Action),
    Saved(Result<Settings, String>),
    Loaded(u64, Result<Settings, String>),
    DismissError(String),
    Tick,
    /// A file in the settings folder changed.
    FilesChanged,
    Connectivity,
    Close,
    OpenDevTools,
    DevToolsToken(u64, window::Id, Option<String>),
    Window(WindowCommand),
    WindowState(enzui_widgets::app_shell::ShellWindowEvent),
    Theme(theme_settings::Action),
    ThemePrepared(u64, theme_settings::Action),
    AppStyle(app_style_settings::Action),
    SharedTheme(Box<enzui_core::theme_file::integration::ThemeEvent>),
    ThemeEditorRead(u64, Box<enzui_shell::theme_store::Store>),
    ThemeEdit(theme_settings::Edit),
    ReduceMotion(bool),
    Center(enzui_agent::center::ConnectorEvent),
    ApplicationWake,
    BackgroundTaskRefused(String),
    AgentClock(Box<enzui_shell::system_settings::Reading>),
    // Retain the approving bus connection for the subsequent mutation.
    AgentApproved(
        u64,
        Result<enzui_shell::authorization::Approval, String>,
        Option<enzui_shell::authorization::Authority>,
    ),
    AgentCommitted(u64, Result<(), (&'static str, String)>),
    AgentDeadline,
    LockFontOpen(bool),
    LockFontMove(i32),
    LockFontSelect(String),
    LockBackgroundChoose,
    LockBackgroundDropped(enzui_widgets::file_drop::FileDrop),
    Users(users::Message),
    FontQuery(Option<(enzui_shell::font_settings::Slot, String)>),
}

struct SettingsApp {
    chrome: enzui_widgets::app_shell::ShellWindowState,
    #[cfg(test)]
    built_routes: std::cell::RefCell<std::collections::BTreeSet<String>>,
    // The sections the last build gave an anchor a sidebar jump can land on.
    #[cfg(test)]
    built_sections: std::cell::RefCell<std::collections::BTreeSet<String>>,
    // Whether each recorded row was built live or inert, last time.
    #[cfg(test)]
    route_liveness: std::cell::RefCell<std::collections::BTreeMap<String, bool>>,
    store: Store,
    plugins: plugins_settings::State,
    search: quick_search_settings::State,
    shortcuts: shortcuts_settings::State,
    sandbox: sandbox_settings::State,
    assistant: assistant_settings::State,
    local_ai: local_ai_settings::State,
    numeric: numeric::State,
    controls: control_editor::State,
    wallpaper: wallpaper_settings::State,
    system: system_pages::State,
    agent: settings_app_center::Agent,
    process: enzui_agent_app::AgentProcess<Message>,
    picker: Option<wallpaper_picker::State>,
    file_chooser: settings_file_chooser::State,
    startup: settings_startup::State,
    image_purpose: ImagePurpose,
    users: users::Users,
    lock_font_active: usize,
    font_query: Option<(enzui_shell::font_settings::Slot, String)>,
    theme_custom: Option<theme_settings::Custom>,
    // Bind wallpaper colour reads to their source so newer selections win.
    theme_wallpaper: (Option<String>, u64),
    renderer_font: Option<iced::Font>,
    picker_generation: u64,
    picker_read: Option<u64>,
    settings: Settings,
    theme: enzui_shell::theme_store::Store,
    app_style: app_style_settings::State,
    themes: Option<enzui_core::theme_file::integration::ThemeIntegration>,
    theme_editor_generation: u64,
    desktop_status: Option<String>,
    desktop_status_busy: bool,
    // Cache periodic read outcomes to avoid idle redraws.
    polls: std::collections::HashMap<&'static str, u64>,
    /// The settings folder, watched so its files are read when they change
    /// rather than every second.
    files_watch: Option<enzui_core::file_tree_watch::DirectoryWatchService>,
    /// When the settings files were last read.
    files_read_at: std::time::Instant,
    motion_supported: bool,
    display_colors: Vec<enzui_shell::desktop_preferences::ColorSupport>,
    islands_supported: bool,
    page: Page,
    navigation: navigation::Navigation,
    filter: String,
    menu: Option<String>,
    background: background_settings::State,
    highlighted: Option<&'static str>,
    reveal_pending: bool,
    window: Option<window::Id>,
    viewport: iced::Size,
    nav: NavSplitState,
    busy: bool,
    resetting_layout: bool,
    reading: bool,
    error: Option<String>,
    generation: u64,
    // The generation carried by the newest settings read.
    read_generation: u64,
    closing: bool,
    stop_requested: bool,
    pending: VecDeque<Change>,
    layout_preview: bool,
    // Retain the drag host's measured rows across bar-editor views.
    layout_drag: enzui_widgets::drag_drop_host::DragHostChannel,
    preview_notification_busy: bool,
    dev_tools_launch: dev_tools::DevToolsLaunch,
    // Disable governed rows while building an inactive settings group.
    suspended: std::cell::Cell<bool>,
}
impl SettingsApp {
    pub(super) fn new(store: Store) -> (Self, Task<Message>) {
        let mut app = Self::model(store);
        let task = app.window_boot();
        (app, task)
    }
    pub(super) fn model(store: Store) -> Self {
        let plugins = plugins_settings::State::new(&store);
        let layout_preview = enzui_shell::desktop_session::mode().managed();
        let page = Page::requested(
            std::env::var("ENZUI_SETTINGS_PAGE").ok().as_deref(),
            layout_preview,
        );
        let target = std::env::var("ENZUI_SETTINGS_PAGE")
            .ok()
            .and_then(|route| enzui_shell::settings_catalog::find(&route));
        let mut app = Self {
            #[cfg(test)]
            built_routes: Default::default(),
            #[cfg(test)]
            built_sections: Default::default(),
            #[cfg(test)]
            route_liveness: Default::default(),
            store,
            plugins,
            search: Default::default(),
            shortcuts: Default::default(),
            sandbox: Default::default(),
            background: Default::default(),
            assistant: Default::default(),
            local_ai: Default::default(),
            numeric: Default::default(),
            controls: Default::default(),
            theme: enzui_shell::theme_store::Store::read(&theme_paths()),
            app_style: app_style_settings::State::default(),
            themes: None,
            theme_editor_generation: 0,
            wallpaper: wallpaper_settings::State::for_environment(),
            system: system_pages::State::for_environment(),
            agent: settings_app_center::Agent::start(),
            process: enzui_agent_app::AgentProcess::desktop(None),
            picker: None,
            file_chooser: settings_file_chooser::State::default(),
            startup: settings_startup::State::default(),
            image_purpose: ImagePurpose::Desktop,
            users: Default::default(),
            lock_font_active: 0,
            font_query: None,
            theme_custom: None,
            theme_wallpaper: (None, 0),
            renderer_font: None,
            picker_generation: 0,
            picker_read: None,
            settings: Settings::default(),
            desktop_status: None,
            desktop_status_busy: false,
            polls: std::collections::HashMap::new(),
            files_watch: None,
            files_read_at: std::time::Instant::now(),
            motion_supported: false,
            display_colors: Vec::new(),
            islands_supported: false,
            page,
            navigation: Default::default(),
            filter: String::new(),
            menu: None,
            highlighted: target.map(|entry| entry.route),
            reveal_pending: target.is_some(),
            window: None,
            chrome: enzui_widgets::app_shell::ShellWindowState::deferred(),
            viewport: iced::Size::new(1028.0, 768.0),
            nav: NavSplitState::new(),
            busy: false,
            resetting_layout: false,
            reading: true,
            error: None,
            generation: 0,
            read_generation: 0,
            closing: false,
            stop_requested: false,
            suspended: std::cell::Cell::new(false),
            pending: VecDeque::new(),
            layout_preview,
            layout_drag: Default::default(),
            preview_notification_busy: false,
            dev_tools_launch: Default::default(),
        };
        app.sync_night_labels();
        app.navigation.expanded = Some(page);
        app.navigation.selected = app.sections(page).first().map(|section| section.route);
        app
    }
    pub(super) fn window_boot(&mut self) -> Task<Message> {
        if self.files_watch.is_none() {
            self.files_watch = self.store.path().parent().and_then(|folder| {
                use enzui_core::file_tree_watch::{
                    DirectoryWatchService, linux::LinuxInotifyBackend,
                };
                let service = DirectoryWatchService::new(LinuxInotifyBackend::new().ok()?);
                service.watch(folder).ok()?;
                Some(service)
            });
        }
        let read = self.refresh_settings_read();
        let theme_boot = if self.themes.is_none() {
            // Settings has a style of its own too (Appearance › App styles).
            let (themes, boot) = match enzui_core::app_data::AppId::new("dev.enzui.Settings") {
                Ok(app) => enzui_core::theme_file::integration::ThemeIntegration::boot_for(app),
                Err(_) => enzui_core::theme_file::integration::ThemeIntegration::boot(),
            };
            self.themes = Some(themes);
            boot.map(|event| Message::SharedTheme(Box::new(event)))
        } else {
            Task::none()
        };
        Task::batch([
            read,
            theme_boot,
            self.read_plugins(),
            self.read_desktop_status(),
            self.read_controls(),
            self.read_wallpaper_gallery(),
            self.read_system(true),
            self.read_users(),
            self.detect_platform(),
        ])
    }
    pub(super) fn process_subscription(&self) -> Subscription<Message> {
        self.process
            .subscription(|_| Message::ApplicationWake, Message::BackgroundTaskRefused)
    }
    pub(super) fn read(&mut self) -> Task<Message> {
        let store = self.store.clone();
        let generation = self.generation;
        self.read_generation = generation;
        let paths = theme_paths();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = enzui_shell::font_settings::migrate_legacy(&store, &paths) {
                        eprintln!("desktop font migration: {error}");
                    }
                    store.load()
                })
                .await
                .map_err(|_| tr("shell-settings-reader-stopped").to_owned())?
            },
            move |result| Message::Loaded(generation, result),
        )
    }
    /// Reads the settings, plugins, wallpaper and desktop status files.
    pub(super) fn read_files(&mut self) -> Task<Message> {
        self.reading = true;
        self.files_read_at = std::time::Instant::now();
        Task::batch([
            self.read(),
            self.read_plugins(),
            self.wallpaper_status(),
            self.read_desktop_status(),
            self.read_controls(),
        ])
    }
    pub(super) fn refresh_settings_read(&mut self) -> Task<Message> {
        // Reopening preferences picks up edits made while only the worker was running.
        self.generation = self.generation.wrapping_add(1);
        self.reading = true;
        self.read()
    }
    pub(super) fn read_desktop_status(&mut self) -> Task<Message> {
        if self.desktop_status_busy {
            return Task::none();
        }
        self.desktop_status_busy = true;
        let path = self.store.path().to_owned();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || enzui_shell::motion::read_status(&path))
                    .await
                    .unwrap_or_default()
            },
            Message::DesktopStatus,
        )
    }
    pub(super) fn write(&mut self, change: Change) -> Task<Message> {
        self.busy = true;
        let store = self.store.clone();
        let compositor = (matches!(&change, Change::Light(_) | Change::ThemeScheme(_))
            && !self.tokens().motion.reduced_motion)
            .then(enzui_shell::compositor::Compositor::from_environment)
            .and_then(Result::ok);
        Task::perform(
            async move {
                if let Some(compositor) = &compositor {
                    compositor.appearance_transition(false).await;
                }
                let result = tokio::task::spawn_blocking(move || store.update(change))
                    .await
                    .map_err(|_| tr("shell-settings-writer-stopped").to_owned())?;
                if let Some(compositor) = &compositor {
                    compositor.appearance_transition(true).await;
                }
                result
            },
            Message::Saved,
        )
    }
}

fn requested_window_size(width: Option<&str>, height: Option<&str>) -> Option<iced::Size> {
    let width = width?
        .parse::<u16>()
        .ok()
        .filter(|value| (240..=1024).contains(value))?;
    let height = height?
        .parse::<u16>()
        .ok()
        .filter(|value| (240..=768).contains(value))?;
    Some(iced::Size::new(f32::from(width), f32::from(height)))
}
pub(crate) fn main() -> std::process::ExitCode {
    use enzui_agent_app::LaunchMode;
    match LaunchMode::from_args(std::env::args_os().skip(1)) {
        Ok(LaunchMode::ExportManifest) => background::export_manifest(),
        Ok(LaunchMode::Background) => match background::run_background() {
            Ok(Some(app)) => run_gui(Some(app)),
            Ok(None) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("enzui-settings: background mode: {error}");
                std::process::ExitCode::FAILURE
            }
        },
        Ok(LaunchMode::Gui) => run_gui(None),
        Err(error) => {
            eprintln!("enzui-settings: {error}");
            std::process::ExitCode::from(2)
        }
    }
}

fn run_gui(presented: Option<SettingsApp>) -> std::process::ExitCode {
    use enzui_agent_app::{AgentProcess, DesktopClaim};
    let Some(store) = Store::from_environment() else {
        eprintln!("enzui-settings: no shell settings directory is configured");
        return std::process::ExitCode::from(2);
    };
    let mut service = None;
    if presented.is_none() {
        let intent = background::launch_intent(
            std::env::var("ENZUI_SETTINGS_PAGE").ok().as_deref(),
            std::env::var("ENZUI_SETTINGS_PLUGIN").ok().as_deref(),
        );
        match enzui_agent_app::claim_desktop(enzui_shell::settings_agent::APP_ID, intent) {
            Ok(DesktopClaim::Owner(owned)) => service = Some(owned),
            Ok(DesktopClaim::Forwarded) => return std::process::ExitCode::SUCCESS,
            Ok(DesktopClaim::Independent) => {}
            Err(error) => {
                eprintln!("enzui-settings: could not reach the running Settings: {error}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    let mut settings = enz_window_settings(ShellLayout::TitlebarSidebarContent);
    if let Some(size) = requested_window_size(
        std::env::var("ENZUI_SETTINGS_WIDTH").ok().as_deref(),
        std::env::var("ENZUI_SETTINGS_HEIGHT").ok().as_deref(),
    ) {
        settings.size = size;
    }
    settings.platform_specific.application_id = "dev.enzui.Settings".into();
    settings.exit_on_close_request = false;
    let pending = std::cell::RefCell::new(presented);
    let service = std::cell::RefCell::new(service);
    let boot = move || {
        if let Some(mut app) = pending.borrow_mut().take() {
            let task = app.present();
            return (app, task);
        }
        let (mut app, task) = SettingsApp::new(store.clone());
        app.process = AgentProcess::desktop(service.borrow_mut().take());
        app.serve_desktop();
        (app, task)
    };
    let result = enzui_widgets::app_setup::run_with_services(
        iced::application(boot, SettingsApp::update, SettingsApp::view)
            .title(SettingsApp::window_title)
            .window(settings)
            .theme(SettingsApp::theme)
            .subscription(SettingsApp::subscription),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(enzui_shell::i18n::localizer().clone()),
    );
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzui-settings: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

// Sample settings must not overwrite the user's theme file.
#[cfg(not(test))]
fn theme_paths() -> enzui_core::theme_file::ThemePaths {
    enzui_core::theme_file::ThemePaths::from_env()
}
#[cfg(test)]
fn theme_paths() -> enzui_core::theme_file::ThemePaths {
    let dir = std::env::temp_dir().join(format!("enzui-settings-theme-{}", std::process::id()));
    enzui_core::theme_file::ThemePaths::new(
        Some(dir),
        Vec::<std::path::PathBuf>::new(),
        None::<std::path::PathBuf>,
    )
}
