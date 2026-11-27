use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::sync::Arc;

use super::{BarEdge, Metric, Metrics, QuickAction, SidebarEdge};

pub(super) const DEFAULT_ASSISTANT_WIDTH: u16 = 460;
pub const MIN_ASSISTANT_WIDTH: u16 = 360;
pub const MAX_ASSISTANT_WIDTH: u16 = 720;

pub(super) fn clamp_assistant_width(width: u64) -> u16 {
    width.clamp(MIN_ASSISTANT_WIDTH.into(), MAX_ASSISTANT_WIDTH.into()) as u16
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub light: bool,
    pub fonts: crate::font_settings::Fonts,
    pub language: crate::i18n::Language,
    pub dock: crate::dock::Preferences,
    pub shell_style: crate::shell_style::ShellStyle,
    pub islands: crate::shell_style::Islands,
    pub sidebar_behavior: crate::sidebar_preferences::Preferences,
    pub sidebar_enabled: bool,
    /// Background work status (copies, downloads) in the bar.
    pub activity_status: bool,
    /// The AI assistant's corner button and panel (on by default).
    pub assistant_enabled: bool,
    /// A short notification when a game controller connects.
    pub controller_notifications: bool,
    pub assistant_corner: super::AssistantCorner,
    pub assistant_width: u16,
    /// The shadow of the bar, islands and frame, 0 (off, the default) to 4.
    pub bar_shadow: u8,
    /// The shadow of panels opened from the bar, 0 (off) to 4.
    pub panel_shadow: u8,
    /// What floating panels are made of: 0 solid, else a liquid glass style
    /// from `reveal::PANEL_MATERIALS`, painted by the compositor.
    pub panel_glass: u8,
    /// The chrome's own colour, opacity and glass adjustments.
    pub chrome: crate::chrome_style::ChromeStyle,
    pub sidebar_pin_scope: crate::sidebar_preferences::PinScope,
    pub bar: BarEdge,
    pub sidebar: SidebarEdge,
    pub tiling: Option<bool>,
    /// Desktops the Control Center switched to tiling (`true`) or floating
    /// on their own, by desktop id; a new default for `tiling` keeps them.
    pub desktop_tiling: std::collections::BTreeMap<u32, bool>,
    /// Whether the input method runs; `None` decides automatically (see
    /// `input_method::wanted`).
    pub input_method: Option<bool>,
    /// A bar button that shows the on-screen keyboard.
    pub osk_button: bool,
    /// The accessibility menu's button in the bar.
    pub accessibility_menu: bool,
    /// Reopen the apps that were open when this account last left.
    pub restore_apps: bool,
    /// Every other display shows the main display.
    pub display_mirror: bool,
    /// Keep a local, private history of unlocked time and focused apps.
    pub screen_time: bool,
    /// IANA time zones shown as world clocks in the clock popover.
    pub world_clocks: Vec<String>,
    pub metrics: Metrics,
    pub quick_actions: u32,
    pub controls: Arc<crate::control_layout::Layout>,
    pub layout: Arc<crate::bar_layout::BarLayout>,
    pub notifications: Arc<crate::notifications::preferences::Preferences>,
    pub quick_search: Arc<crate::search::preferences::Preferences>,
    pub compositor: crate::desktop_preferences::CompositorPreferences,
    pub wallpaper: crate::desktop_preferences::Wallpaper,
    pub session: crate::session_preferences::SessionPreferences,
    pub night_light: crate::night_light::Preferences,
    pub capture: crate::capture::Preferences,
    pub displays: Vec<crate::system_settings::display::Saved>,
    /// The icon theme folder name; `None` follows the desktop.
    pub icon_theme: Option<String>,
    /// Persisted scheme ID; `None` delegates style and colour to the theme file.
    pub theme_scheme: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            light: false,
            fonts: Default::default(),
            language: Default::default(),
            dock: Default::default(),
            shell_style: Default::default(),
            islands: Default::default(),
            sidebar_behavior: Default::default(),
            sidebar_enabled: true,
            activity_status: true,
            assistant_enabled: true,
            controller_notifications: true,
            assistant_corner: super::AssistantCorner::BottomRight,
            assistant_width: DEFAULT_ASSISTANT_WIDTH,
            bar_shadow: 0,
            panel_shadow: 2,
            panel_glass: 0,
            chrome: crate::chrome_style::ChromeStyle::default(),
            sidebar_pin_scope: Default::default(),
            bar: BarEdge::Top,
            sidebar: SidebarEdge::Left,
            tiling: None,
            desktop_tiling: Default::default(),
            input_method: None,
            osk_button: false,
            accessibility_menu: false,
            restore_apps: false,
            display_mirror: false,
            screen_time: false,
            world_clocks: Vec::new(),
            metrics: Metrics::default(),
            quick_actions: QuickAction::ALL
                .into_iter()
                .filter(|action| !matches!(action, QuickAction::Lock | QuickAction::Power))
                .fold(0, |mask, action| mask | action.bit()),
            controls: Arc::default(),
            layout: Arc::new(crate::bar_layout::BarLayout::default().with_notifications()),
            notifications: Arc::default(),
            quick_search: Arc::default(),
            compositor: Default::default(),
            wallpaper: Default::default(),
            session: Default::default(),
            night_light: Default::default(),
            capture: Default::default(),
            displays: Vec::new(),
            icon_theme: None,
            theme_scheme: None,
        }
    }
}
impl Settings {
    pub fn effective_metrics(&self) -> Metrics {
        if self.shell_style == crate::shell_style::ShellStyle::Frame {
            self.metrics
        } else {
            Metrics {
                frame: 0,
                bar: self.islands.height,
                radius: self.islands.radius,
                rail: self.islands.rail,
                islands: Some(self.islands),
                ..self.metrics
            }
        }
    }
    pub fn shows(&self, action: QuickAction) -> bool {
        self.quick_actions & action.bit() != 0
    }
    pub fn preview(&mut self, change: Change) -> Result<(), String> {
        match change {
            Change::Dock(change) => self.dock.apply(&change)?,
            Change::Search(change) => Arc::make_mut(&mut self.quick_search).preview(&change)?,
            Change::Notification(change) => {
                Arc::make_mut(&mut self.notifications).preview(&change)?;
            }
            Change::Light(value) => self.light = value,
            Change::Language(language) => self.language = language,
            Change::ShellStyle(value) => self.shell_style = value,
            Change::Island(key, value) => self.islands.set(key, value)?,
            Change::IslandsJoinedBar(value) => self.islands.joined_bar = value,
            Change::SidebarVisibility(value) => self.sidebar_behavior.visibility = value,
            Change::SidebarEnabled(value) => self.sidebar_enabled = value,
            Change::ActivityStatus(value) => self.activity_status = value,
            Change::AssistantEnabled(value) => self.assistant_enabled = value,
            Change::ControllerNotifications(value) => self.controller_notifications = value,
            Change::AssistantCorner(value) => self.assistant_corner = value,
            Change::AssistantWidth(value) => {
                self.assistant_width = clamp_assistant_width(value.into());
            }
            Change::BarShadow(value) => self.bar_shadow = value.min(4),
            Change::PanelGlass(value) => {
                // A new material shows its own look; fine-tuning starts over.
                self.chrome.glass_tint = None;
                self.chrome.glass_blur = None;
                self.chrome.glass_bending = None;
                self.chrome.glass_highlight = None;
                self.panel_glass = value.min(crate::reveal::PANEL_MATERIALS.len() as u8 - 1);
            }
            Change::PanelShadow(value) => self.panel_shadow = value.min(4),
            Change::Chrome(change) => self.chrome.apply(change)?,
            Change::Font(ref change) => self.fonts.apply(change.clone())?,
            Change::IconTheme(ref value) => self.icon_theme = value.clone(),
            Change::ThemeScheme(ref value) => self.theme_scheme = Some(value.clone()),
            Change::SidebarPinScope(value) => self.sidebar_pin_scope = value,
            Change::FullscreenPadding(value) => self.sidebar_behavior.set_padding(value)?,
            Change::ResetIslandSizing => {
                self.islands = Default::default();
                self.sidebar_behavior = Default::default();
            }
            Change::ResetFrameSizing => {
                for key in [Metric::Frame, Metric::Radius, Metric::Bar, Metric::Rail] {
                    self.metrics.set(key, Metrics::default().get(key))?;
                }
            }
            Change::Compositor(key, value) => self.compositor.set(key, value)?,
            Change::KeyboardLayouts(ref layouts) => self.compositor.set_layouts(layouts)?,
            Change::Display(ref change) => {
                let mut next = self.compositor.clone();
                match change {
                    crate::desktop_preferences::DisplayChange::X11Sharp(on) => next.x11_sharp = *on,
                    crate::desktop_preferences::DisplayChange::ColorMode {
                        output,
                        mode: Some(mode),
                    } => {
                        next.color_modes.insert(output.clone(), mode.clone());
                    }
                    crate::desktop_preferences::DisplayChange::ColorMode { output, mode: None } => {
                        next.color_modes.remove(output);
                    }
                }
                next.validate()?;
                self.compositor = next;
            }
            Change::Shortcut(ref id, ref chord) => {
                self.compositor.set_shortcut(id, chord.as_deref())?;
            }
            Change::CustomShortcut(ref id, ref shortcut) => {
                self.compositor.set_custom_shortcut(id, shortcut.as_ref())?;
            }
            Change::Terminal(ref program) => {
                if !crate::desktop_preferences::valid_terminal(program) {
                    return Err(tr("shell-invalid-terminal").into());
                }
                self.compositor.terminal = program.clone();
            }
            Change::Wallpaper(change) => self.wallpaper.apply(change)?,
            Change::Session(ref change) => self.session.apply(change)?,
            Change::NightLight(change) => self.night_light.apply(change)?,
            Change::Capture(ref change) => self.capture.apply(change.clone())?,
            Change::Displays(ref layout) => {
                crate::system_settings::display::remember(&mut self.displays, layout)?;
            }
            Change::Bar(value) => self.bar = value,
            Change::Sidebar(value) => self.sidebar = value,
            Change::Tiling(value) => self.tiling = Some(value),
            Change::DesktopTiling(desktop, mode) => {
                set_desktop_tiling(&mut self.desktop_tiling, desktop, mode)?;
            }
            Change::DesktopTilingReset => self.desktop_tiling.clear(),
            Change::InputMethod(value) => self.input_method = Some(value),
            Change::OskButton(value) => self.osk_button = value,
            Change::AccessibilityMenu(value) => self.accessibility_menu = value,
            Change::RestoreApps(value) => self.restore_apps = value,
            Change::DisplayMirror(value) => self.display_mirror = value,
            Change::ScreenTime(value) => self.screen_time = value,
            Change::WorldClockAdd(ref zone) => add_world_clock(&mut self.world_clocks, zone)?,
            Change::WorldClockRemove(ref zone) => self.world_clocks.retain(|known| known != zone),
            Change::Metric(metric, value) => self.metrics.set(metric, value)?,
            Change::QuickAction(action, shown) => {
                if shown {
                    self.quick_actions |= action.bit();
                } else {
                    self.quick_actions &= !action.bit();
                }
            }
            Change::ResetSizing => self.metrics = Metrics::default(),
            Change::ControlsLayout { expected, value } => {
                if self.controls != expected && self.controls != value {
                    return Err(
                        tr("shell-control-center-changed-elsewhere-reload-and-retry").into(),
                    );
                }
                self.controls = value;
            }
            Change::Layout { expected, value } => {
                if self.layout != expected && self.layout != value {
                    return Err(tr("shell-bar-layout-changed-elsewhere-reload-and-retry").into());
                }
                self.layout = value;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Change {
    Font(crate::font_settings::FontChange),
    Dock(crate::dock::Change),
    Search(crate::search::preferences::Change),
    Notification(crate::notifications::preferences::Change),
    Light(bool),
    Language(crate::i18n::Language),
    ShellStyle(crate::shell_style::ShellStyle),
    Island(crate::shell_style::IslandOption, u16),
    IslandsJoinedBar(bool),
    SidebarVisibility(crate::sidebar_preferences::Visibility),
    SidebarEnabled(bool),
    ActivityStatus(bool),
    AssistantEnabled(bool),
    ControllerNotifications(bool),
    AssistantCorner(super::AssistantCorner),
    AssistantWidth(u16),
    BarShadow(u8),
    PanelGlass(u8),
    PanelShadow(u8),
    Chrome(crate::chrome_style::Change),
    InputMethod(bool),
    OskButton(bool),
    AccessibilityMenu(bool),
    RestoreApps(bool),
    DisplayMirror(bool),
    ScreenTime(bool),
    WorldClockAdd(String),
    WorldClockRemove(String),
    NightLight(crate::night_light::Change),
    Capture(crate::capture::Change),
    IconTheme(Option<String>),
    ThemeScheme(String),
    SidebarPinScope(crate::sidebar_preferences::PinScope),
    FullscreenPadding(u16),
    ResetIslandSizing,
    ResetFrameSizing,
    Compositor(crate::desktop_preferences::OptionKey, i32),
    KeyboardLayouts(Vec<String>),
    /// The program Super+Return starts; empty restores the installed one.
    Terminal(String),
    /// A chord for a changeable action (`crate::shortcut_map`); `None`
    /// restores its default, and an empty id restores every default.
    Shortcut(String, Option<String>),
    /// Adds, updates or removes a user-defined shortcut.
    CustomShortcut(String, Option<crate::shortcut_map::CustomShortcut>),
    /// Colour modes and X11 scaling, which the compositor applies.
    Display(crate::desktop_preferences::DisplayChange),
    Wallpaper(crate::desktop_preferences::WallpaperChange),
    Session(crate::session_preferences::SessionChange),
    /// Remembers the layout of the displays connected now.
    Displays(Vec<crate::system_settings::display::Saved>),
    Bar(BarEdge),
    Sidebar(SidebarEdge),
    Tiling(bool),
    /// One desktop tiles (`Some(true)`), floats (`Some(false)`) or follows
    /// the default again (`None`).
    DesktopTiling(u32, Option<bool>),
    /// Every desktop follows the default again.
    DesktopTilingReset,
    Metric(Metric, u16),
    QuickAction(QuickAction, bool),
    ResetSizing,
    ControlsLayout {
        expected: Arc<crate::control_layout::Layout>,
        value: Arc<crate::control_layout::Layout>,
    },
    Layout {
        expected: Arc<crate::bar_layout::BarLayout>,
        value: Arc<crate::bar_layout::BarLayout>,
    },
}
impl Change {
    #[must_use]
    pub fn coalesce(mut self, earlier: &Self) -> Self {
        if let (
            Self::Layout { expected, .. },
            Self::Layout {
                expected: base,
                value,
            },
        ) = (&mut self, earlier)
            && *expected == *value
        {
            *expected = std::sync::Arc::clone(base);
        }
        if let (
            Self::ControlsLayout { expected, .. },
            Self::ControlsLayout {
                expected: base,
                value,
            },
        ) = (&mut self, earlier)
            && *expected == *value
        {
            *expected = std::sync::Arc::clone(base);
        }
        self
    }
    pub fn same_key(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Font(a), Self::Font(b)) => match (a, b) {
                (
                    crate::font_settings::FontChange::Family(a, _),
                    crate::font_settings::FontChange::Family(b, _),
                ) => a == b,
                (
                    crate::font_settings::FontChange::Weight(_),
                    crate::font_settings::FontChange::Weight(_),
                ) => true,
                _ => false,
            },
            (Self::Dock(a), Self::Dock(b)) => match (a, b) {
                (crate::dock::Change::Pin(a, _), crate::dock::Change::Pin(b, _))
                | (crate::dock::Change::EnableAndPin(a), crate::dock::Change::EnableAndPin(b)) => {
                    a == b
                }
                (
                    crate::dock::Change::Move(..)
                    | crate::dock::Change::MoveFolder(..)
                    | crate::dock::Change::ReorderPins(_),
                    _,
                )
                | (
                    _,
                    crate::dock::Change::Move(..)
                    | crate::dock::Change::MoveFolder(..)
                    | crate::dock::Change::ReorderPins(_),
                ) => false,
                _ => std::mem::discriminant(a) == std::mem::discriminant(b),
            },
            (Self::Search(a), Self::Search(b)) => a.same_key(b),
            (Self::Notification(a), Self::Notification(b)) => a.same_key(b),
            (Self::Metric(a, _), Self::Metric(b, _)) => a == b,
            (Self::Island(a, _), Self::Island(b, _)) => a == b,
            (Self::Shortcut(..) | Self::CustomShortcut(..), _) => false,
            (Self::Compositor(a, _), Self::Compositor(b, _)) => a == b,
            (Self::Wallpaper(a), Self::Wallpaper(b)) => a.same_slot(b),
            (Self::Session(a), Self::Session(b)) => {
                std::mem::discriminant(a) == std::mem::discriminant(b)
            }
            (Self::QuickAction(a, _), Self::QuickAction(b, _)) => a == b,
            (Self::DesktopTiling(a, _), Self::DesktopTiling(b, _)) => a == b,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}

pub struct NotificationState {
    pub settings: Settings,
    pub history: Result<Vec<crate::notifications::model::Archived>, String>,
}

pub(super) fn parse(value: &serde_json::Value) -> Result<Settings, String> {
    let mut metrics = Metrics::default();
    for metric in Metric::ALL {
        if let Some(value) = value.get(metric.key()) {
            metrics.set(
                metric,
                value
                    .as_u64()
                    .and_then(|v| u16::try_from(v).ok())
                    .ok_or_else(|| tr("shell-invalid-shell-size-preference"))?,
            )?;
        }
    }
    let mut quick_actions = Settings::default().quick_actions;
    if let Some(actions) = value.get("quick_actions") {
        let actions = actions
            .as_array()
            .filter(|a| a.len() <= QuickAction::ALL.len())
            .ok_or_else(|| tr("shell-invalid-quick-actions"))?;
        quick_actions = 0;
        for key in actions {
            let action = QuickAction::ALL
                .into_iter()
                .find(|a| Some(a.key()) == key.as_str())
                .ok_or_else(|| tr("shell-unknown-quick-action"))?;
            if quick_actions & action.bit() != 0 {
                return Err(tr("shell-duplicate-quick-action").into());
            }
            quick_actions |= action.bit();
        }
    }
    let boolean = |key| match value.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| tr("shell-invalid-shell-boolean-preference")),
    };
    let dock = crate::dock::Preferences::from_json(value.get("dock"))?;
    Ok(Settings {
        fonts: crate::font_settings::Fonts::from_json(value.get("fonts"))?,
        light: boolean("light")?.unwrap_or(false),
        language: match value.get("language") {
            None => Default::default(),
            Some(value) => value
                .as_str()
                .and_then(crate::i18n::Language::from_key)
                .ok_or_else(|| tr("shell-invalid-shell-language"))?,
        },
        dock,
        shell_style: crate::shell_style::ShellStyle::from_json(value.get("shell_style"))?,
        islands: crate::shell_style::Islands::from_json(value.get("islands"))?,
        sidebar_behavior: crate::sidebar_preferences::Preferences::from_json(
            value.get("sidebar_behavior"),
        )?,
        sidebar_enabled: boolean("sidebar_enabled")?.unwrap_or(true),
        activity_status: boolean("activity_status")?.unwrap_or(true),
        assistant_enabled: boolean("assistant_enabled")?.unwrap_or(true),
        controller_notifications: boolean("controller_notifications")?.unwrap_or(true),
        assistant_corner: match value.get("assistant_corner") {
            None => super::AssistantCorner::BottomRight,
            Some(corner) => corner
                .as_str()
                .and_then(super::AssistantCorner::from_key)
                .ok_or_else(|| tr("shell-invalid-assistant-corner"))?,
        },
        assistant_width: value["assistant_width"]
            .as_u64()
            .map_or(DEFAULT_ASSISTANT_WIDTH, clamp_assistant_width),
        // An earlier on/off choice reads as Small.
        bar_shadow: match value.get("bar_shadow") {
            None | Some(serde_json::Value::Bool(false)) => 0,
            Some(serde_json::Value::Bool(true)) => 2,
            Some(level) => level
                .as_u64()
                .filter(|level| *level <= 4)
                .ok_or_else(|| tr("shell-invalid-bar-shadow"))? as u8,
        },
        panel_shadow: match value.get("panel_shadow") {
            None => 2,
            Some(level) => level
                .as_u64()
                .filter(|level| *level <= 4)
                .ok_or_else(|| tr("shell-invalid-panel-shadow"))? as u8,
        },
        panel_glass: match value.get("panel_glass") {
            None => 0,
            Some(style) => style
                .as_u64()
                .filter(|style| (*style as usize) < crate::reveal::PANEL_MATERIALS.len())
                .ok_or_else(|| tr("shell-invalid-panel-material"))?
                as u8,
        },
        chrome: crate::chrome_style::ChromeStyle::from_json(value.get("chrome"))?,
        sidebar_pin_scope: crate::sidebar_preferences::PinScope::from_json(
            value.get("sidebar_pin_scope"),
        )?,
        bar: match value.get("bar").and_then(|v| v.as_str()) {
            None if value.get("bar").is_none() => BarEdge::Top,
            Some("top") => BarEdge::Top,
            Some("bottom") => BarEdge::Bottom,
            _ => return Err(tr("shell-invalid-bar-position").into()),
        },
        sidebar: match value.get("sidebar").and_then(|v| v.as_str()) {
            None if value.get("sidebar").is_none() => SidebarEdge::Left,
            Some("left") => SidebarEdge::Left,
            Some("right") => SidebarEdge::Right,
            _ => return Err(tr("shell-invalid-sidebar-position").into()),
        },
        tiling: boolean("tiling")?,
        desktop_tiling: desktop_tiling_from_json(value.get("desktop_tiling"))?,
        input_method: boolean("input_method")?,
        osk_button: boolean("osk_button")?.unwrap_or(false),
        accessibility_menu: boolean("accessibility_menu")?.unwrap_or(false),
        restore_apps: boolean("restore_apps")?.unwrap_or(false),
        display_mirror: boolean("display_mirror")?.unwrap_or(false),
        screen_time: boolean("screen_time")?.unwrap_or(false),
        world_clocks: world_clocks_from_json(value.get("world_clocks")),
        compositor: crate::desktop_preferences::CompositorPreferences::shortcuts_from_account(
            value,
        )?,
        wallpaper: crate::desktop_preferences::Wallpaper::from_json(value.get("wallpaper"))?,
        session: crate::session_preferences::SessionPreferences::from_json(value.get("session"))?,
        night_light: crate::night_light::Preferences::from_json(value.get("night_light"))?,
        capture: crate::capture::Preferences::from_json(value.get("capture"))?,
        displays: crate::system_settings::display::saved_from_json(value.get("displays"))?,
        icon_theme: match value.get("icon_theme") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(id)) if icon_theme_id(id) => Some(id.clone()),
            _ => return Err(tr("shell-invalid-icon-theme").into()),
        },
        theme_scheme: value
            .get("theme_scheme")
            .and_then(serde_json::Value::as_str)
            .filter(|id| crate::theme_palette::Scheme::from_id_str(id).is_some())
            .map(str::to_owned),
        quick_search: Arc::new(crate::search::preferences::Preferences::from_json(
            value.get("quick_search"),
        )?),
        notifications: Arc::new(crate::notifications::preferences::Preferences::from_json(
            value.get("notifications"),
        )?),
        metrics,
        quick_actions,
        controls: Arc::new(crate::control_layout::Layout::from_json(
            value.get("control_center_layout"),
        )?),
        layout: Arc::new(
            value
                .get("bar_layout")
                .map(crate::bar_layout::BarLayout::from_json)
                .transpose()?
                .unwrap_or_default()
                .with_notifications()
                .without_legacy_dock(),
        ),
    })
}

// An icon theme is named by its folder, which must stay a single name.
pub(crate) fn icon_theme_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.starts_with('.') && !id.contains(['/', '\\', '\0'])
}

// Adds `zone` to the world clocks unless it is there already or the list is full.
pub(super) fn add_world_clock(clocks: &mut Vec<String>, zone: &str) -> Result<(), String> {
    if !crate::world_clocks::valid(zone) {
        return Err(tr("shell-unknown-time-zone").into());
    }
    if clocks.len() >= crate::world_clocks::MAX && !clocks.iter().any(|known| known == zone) {
        return Err(
            tr!("shell-up-to-world-clocks", "arg0" => format!("{:}", crate::world_clocks::MAX)),
        );
    }
    if !clocks.iter().any(|known| known == zone) {
        clocks.push(zone.to_owned());
    }
    Ok(())
}

// The stored world clocks; entries that are not zone names are dropped.
pub(super) fn world_clocks_from_json(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|zone| {
            zone.len() <= 64
                && zone
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
                && !zone.contains("..")
        })
        .take(crate::world_clocks::MAX)
        .map(str::to_owned)
        .collect()
}

const MAX_DESKTOP_MODES: usize = 256;

pub(super) fn set_desktop_tiling(
    modes: &mut std::collections::BTreeMap<u32, bool>,
    desktop: u32,
    mode: Option<bool>,
) -> Result<(), String> {
    if desktop == 0 || desktop >= 2_147_483_000 {
        return Err(tr("shell-invalid-desktop").into());
    }
    match mode {
        Some(mode) => {
            if !modes.contains_key(&desktop) && modes.len() >= MAX_DESKTOP_MODES {
                return Err(tr("shell-too-many-desktops-with-their-own-window-mode").into());
            }
            modes.insert(desktop, mode);
        }
        None => {
            modes.remove(&desktop);
        }
    }
    Ok(())
}

pub(super) fn desktop_tiling_from_json(
    value: Option<&serde_json::Value>,
) -> Result<std::collections::BTreeMap<u32, bool>, String> {
    let mut modes = std::collections::BTreeMap::new();
    let Some(value) = value else { return Ok(modes) };
    let object = value
        .as_object()
        .ok_or_else(|| tr("shell-invalid-desktop-window-modes"))?;
    for (key, mode) in object {
        let desktop = key
            .parse::<u32>()
            .map_err(|_| tr("shell-invalid-desktop-window-modes"))?;
        set_desktop_tiling(
            &mut modes,
            desktop,
            Some(
                mode.as_bool()
                    .ok_or_else(|| tr("shell-invalid-desktop-window-modes"))?,
            ),
        )?;
    }
    Ok(modes)
}
