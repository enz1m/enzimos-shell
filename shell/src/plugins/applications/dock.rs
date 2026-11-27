use crate::search::{AppTarget, Item, Target, WindowTarget};
use enzui_shell::i18n::tr;
use iced::{Rectangle, Size};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

macro_rules! choice {
    ($name:ident, $default:ident, [$($variant:ident => ($key:literal, $label:expr)),+]) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all="kebab-case")]
        pub enum $name { $($variant),+ }
        impl Default for $name { fn default() -> Self { Self::$default } }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn label(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
            pub fn key(self) -> &'static str { match self { $(Self::$variant => $key),+ } }
        }
    }
}
choice!(Position, Bottom, [Bar => ("bar",tr("shell-in-the-bar")), Top => ("top",tr("shell-top")), Bottom => ("bottom",tr("shell-bottom")), Left => ("left",tr("shell-left")), Right => ("right",tr("shell-right"))]);
choice!(Style, Desktop, [Desktop => ("desktop",tr("shell-dock-match-desktop")), Island => ("island",tr("shell-island")), Panel => ("panel",tr("shell-edge-panel")), Minimal => ("minimal",tr("shell-icons-only"))]);
choice!(Visibility, Never, [Never => ("never",tr("shell-always-visible")), Always => ("always",tr("shell-autohide")), Overlap => ("overlap",tr("shell-hide-when-windows-overlap")), Fullscreen => ("fullscreen",tr("shell-hide-for-fullscreen-apps"))]);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub position: Position,
    pub style: Style,
    pub visibility: Visibility,
    pub icon_size: u16,
    pub spacing: u16,
    pub radius: u16,
    pub highlight_radius: u16,
    pub margin: u16,
    pub show_running: bool,
    pub current_desktop: bool,
    pub pins: Vec<String>,
    pub folders: Vec<String>,
    pub hover_previews: bool,
    pub overview: bool,
    pub applications: bool,
    pub downloads: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Position::Bottom,
            style: Style::Desktop,
            visibility: Visibility::Never,
            icon_size: 24,
            spacing: 4,
            radius: 12,
            highlight_radius: 6,
            margin: 8,
            show_running: true,
            current_desktop: false,
            pins: Vec::new(),
            folders: Vec::new(),
            hover_previews: false,
            overview: false,
            applications: false,
            downloads: false,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Change {
    Enabled(bool),
    Position(Position),
    Style(Style),
    Visibility(Visibility),
    IconSize(u16),
    Spacing(u16),
    Radius(u16),
    HighlightRadius(u16),
    Margin(u16),
    ShowRunning(bool),
    CurrentDesktop(bool),
    Pin(String, bool),
    EnableAndPin(String),
    PinFolders(Vec<String>),
    UnpinFolder(String),
    Move(String, i8),
    ReorderPins(enzui_core::drag::DropRequest),
}
impl Preferences {
    pub fn in_bar(&self, bar: crate::settings::BarEdge) -> bool {
        self.position == Position::Bar
            || matches!(
                (self.position, bar),
                (Position::Top, crate::settings::BarEdge::Top)
                    | (Position::Bottom, crate::settings::BarEdge::Bottom)
            )
    }
    #[must_use]
    pub fn effective(&self, bar: crate::settings::BarEdge) -> Self {
        let mut effective = self.clone();
        if self.in_bar(bar) {
            effective.position = Position::Bar;
        }
        effective
    }
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            None => Self::default(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-dock-preferences"))?,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(12..=64).contains(&self.icon_size)
            || self.spacing > 24
            || self.radius > 32
            || self.highlight_radius > 32
            || self.margin > 24
        {
            return Err(tr("shell-dock-sizes-are-outside-their-supported-range").into());
        }
        let mut seen = BTreeSet::new();
        if self.pins.len() > 32
            || self
                .pins
                .iter()
                .any(|id| !valid_app_id(id) || !seen.insert(id))
        {
            return Err(
                tr("shell-dock-pins-must-be-unique-application-identities-at-most-32").into(),
            );
        }
        if !valid_folders(self.folders.iter().map(String::as_str)) {
            return Err(tr("shell-dock-invalid-folders").into());
        }
        Ok(())
    }
    pub fn apply(&mut self, change: &Change) -> Result<(), String> {
        let mut next = self.clone();
        match change {
            Change::Enabled(v) => next.enabled = *v,
            Change::EnableAndPin(id) => {
                next.enabled = true;
                if !next.pins.contains(id) {
                    next.pins.push(id.clone());
                }
            }
            Change::Position(v) => next.position = *v,
            Change::Style(v) => next.style = *v,
            Change::Visibility(v) => next.visibility = *v,
            Change::IconSize(v) => next.icon_size = *v,
            Change::Spacing(v) => next.spacing = *v,
            Change::Radius(v) => next.radius = *v,
            Change::HighlightRadius(v) => next.highlight_radius = *v,
            Change::Margin(v) => next.margin = *v,
            Change::ShowRunning(v) => next.show_running = *v,
            Change::CurrentDesktop(v) => next.current_desktop = *v,
            Change::Pin(id, true) => {
                if !next.pins.contains(id) {
                    next.pins.push(id.clone());
                }
            }
            Change::Pin(id, false) => next.pins.retain(|known| known != id),
            Change::PinFolders(uris) => {
                for uri in uris {
                    if next.folders.len() < MAX_FOLDERS && !next.folders.contains(uri) {
                        next.folders.push(uri.clone());
                    }
                }
            }
            Change::UnpinFolder(uri) => next.folders.retain(|known| known != uri),
            Change::ReorderPins(request) => {
                use enzui_core::drag::{ItemId, reorder};
                let order: Vec<_> = next.pins.iter().cloned().map(ItemId::new).collect();
                if let Ok(diff) = reorder(&order, &request.items, request.slot) {
                    next.pins = diff.order.into_iter().map(|id| id.0).collect();
                }
            }
            Change::Move(id, step) => {
                if let Some(index) = next.pins.iter().position(|known| known == id) {
                    let to = (index as isize + isize::from(*step))
                        .clamp(0, next.pins.len() as isize - 1)
                        as usize;
                    next.pins.swap(index, to);
                }
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn update_json(value: &mut serde_json::Value, change: &Change) -> Result<(), String> {
        let mut next = Self::from_json(value.get("dock"))?;
        next.apply(change)?;
        let fields =
            serde_json::to_value(next).map_err(|_| tr("shell-cannot-save-dock-preferences"))?;
        let root = value
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-settings-document"))?;
        let dock = root
            .entry("dock")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-dock-preferences"))?;
        for (key, value) in fields
            .as_object()
            .ok_or_else(|| tr("shell-invalid-serialized-application-preferences"))?
        {
            dock.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}
pub fn valid_app_id(id: &str) -> bool {
    if let Some(name) = id.strip_prefix("app:desktop/") {
        return name.len() <= 220
            && name.ends_with(".desktop")
            && name
                .chars()
                .all(|c| !c.is_control() && c != '/' && c != '\\');
    }
    id.starts_with("app:")
        && (5..=255).contains(&id.len())
        && id
            .strip_prefix("app:")
            .unwrap_or_default()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub const MAX_FOLDERS: usize = 16;

pub fn folder_path(uri: &str) -> Option<std::path::PathBuf> {
    if uri.len() > 1536 || !uri.starts_with("file:///") {
        return None;
    }
    let path = enzui_clipboard::validate_file_uri(uri).ok()?;
    (crate::desktop_portal::uri(&path) == uri).then_some(path)
}

pub(crate) fn valid_folders<'a>(uris: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    uris.into_iter()
        .all(|uri| seen.len() < MAX_FOLDERS && folder_path(uri).is_some() && seen.insert(uri))
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Overview,
    Applications,
    Downloads,
}

#[derive(Clone, Debug)]
pub enum Builtin {
    Action(Action),
    Folder(String),
}

impl Builtin {
    pub fn title(&self) -> String {
        match self {
            Self::Action(Action::Overview) => tr("shell-dock-window-overview").into(),
            Self::Action(Action::Applications) => tr("shell-dock-app-launcher").into(),
            Self::Action(Action::Downloads) => tr("shell-dock-downloads").into(),
            Self::Folder(uri) => folder_path(uri).map_or_else(String::new, |path| {
                crate::search::catalogue::display_text(
                    &path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy(),
                    120,
                )
            }),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Action(Action::Overview) => "panels-top-left",
            Self::Action(Action::Applications) => "grid-2x2",
            Self::Action(Action::Downloads) => "folder-down",
            Self::Folder(_) => "folder",
        }
    }
}

#[derive(Clone, Debug)]
pub struct App {
    pub item: Item,
    pub classes: Vec<String>,
}
impl App {
    pub fn from_item(mut item: Item) -> Option<Self> {
        let Target::App(target) = item.target.as_ref()? else {
            return None;
        };
        let mut classes: Vec<String> = Vec::new();
        match target {
            AppTarget::Prepared(id) => {
                classes.push(
                    crate::catalog::desktop_id(id)
                        .unwrap_or(match id.as_str() {
                            "enzui-settings" => "dev.enzui.Settings",
                            "enzui-assistant" => "dev.enzui.Assistant",
                            _ => id,
                        })
                        .into(),
                );
            }
            AppTarget::Desktop { path, .. } => {
                classes = crate::search::catalogue::application_ids(path);
            }
        }
        if let AppTarget::Prepared(id) = target {
            let names: &[&str] = match id.as_str() {
                "terminal-emulator" => &["io.github.enzui.Terminal", "utilities-terminal"],
                "files-app" => &["io.github.enzui.Files", "system-file-manager", "folder"],
                "text-app" => &[
                    "io.github.enzui.Text",
                    "accessories-text-editor",
                    "text-editor",
                ],
                "enzui-settings" => &["dev.enzui.Settings", "preferences-system"],
                "task-manager-app" => &["io.github.enzui.TaskManager", "utilities-system-monitor"],
                _ => &["application-x-executable"],
            };
            item.image = item.image.clone().or_else(|| {
                classes
                    .iter()
                    .find_map(|class| crate::search::catalogue::icon_for_app_id(class))
                    .or_else(|| {
                        names
                            .iter()
                            .find_map(|name| crate::icon_theme::lookup(name, 24))
                    })
            });
        }
        Some(Self { item, classes })
    }
}
fn same_class(left: &str, right: &str) -> bool {
    let left = left.strip_suffix(".desktop").unwrap_or(left);
    let right = right.strip_suffix(".desktop").unwrap_or(right);
    !left.is_empty() && left.eq_ignore_ascii_case(right)
}

fn generic_class(class: &str) -> bool {
    ["", "electron", "java", "sun-awt-X11-XFramePeer", "wine"]
        .iter()
        .any(|value| class.eq_ignore_ascii_case(value))
}

pub(crate) fn match_app<'a>(
    apps: &'a [App],
    client: &serde_json::Value,
    target: &WindowTarget,
) -> Option<&'a App> {
    if apps.is_empty() {
        return None;
    }
    let current = client["class"].as_str().unwrap_or_default();
    let initial = client["initialClass"].as_str().unwrap_or_default();
    let mut best = None;
    let mut best_score = 0;
    for app in apps {
        let score = app
            .classes
            .iter()
            .enumerate()
            .map(|(i, known)| {
                match (
                    i == 0,
                    same_class(current, known),
                    same_class(initial, known),
                ) {
                    (true, true, _) => 4,
                    (true, _, true) => 2,
                    (false, true, _) if !generic_class(current) => 3,
                    (false, _, true) if !generic_class(initial) => 1,
                    _ => 0,
                }
            })
            .max()
            .unwrap_or_default();
        if score > best_score {
            best = Some(app);
            best_score = score;
        } else if score == best_score {
            best = None;
        }
    }
    best.or_else(|| {
        let cgroup = std::fs::read_to_string(format!("/proc/{}/cgroup", target.pid)).ok()?;
        let (_, id) = crate::window_feed::unit_of(&cgroup)?;
        let mut matches = apps.iter().filter(|app| {
            app.classes
                .first()
                .is_some_and(|known| same_class(known, &id))
        });
        let app = matches.next()?;
        matches.next().is_none().then_some(app)
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub target: WindowTarget,
    pub app_id: Option<String>,
    pub title: String,
    pub class: String,
    pub workspace: i64,
    pub output: i64,
    pub bounds: Rectangle,
    pub fullscreen: bool,
    pub active: bool,
    pub urgent: bool,
    pub minimized: bool,
}
impl Window {
    fn group_id(&self) -> String {
        if let Some(id) = &self.app_id {
            return id.clone();
        }
        if generic_class(&self.class) {
            return format!("process:{}:{}", self.target.pid, self.target.start);
        }
        format!("window:{}", self.class.to_ascii_lowercase())
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub image: Option<std::path::PathBuf>,
    pub app: Option<AppTarget>,
    pub pinned: bool,
    pub builtin: Option<Builtin>,
    pub windows: Vec<Window>,
}
fn stable_window_order(left: &Window, right: &Window) -> std::cmp::Ordering {
    left.target
        .stable_id
        .parse::<u64>()
        .ok()
        .cmp(&right.target.stable_id.parse::<u64>().ok())
        .then_with(|| left.target.stable_id.cmp(&right.target.stable_id))
}
pub fn groups(
    preferences: &Preferences,
    apps: &[App],
    windows: &[Window],
    desktop: i64,
) -> Vec<Group> {
    // Client enumeration follows compositor z-order; focus must not reorder
    // dock slots, hover cards or the next window in a cycling sequence.
    let mut ordered: Vec<_> = windows.iter().map(|w| (w.group_id(), w)).collect();
    ordered.sort_by(|(_, left), (_, right)| stable_window_order(left, right));
    let ids = preferences.pins.iter().cloned().chain(
        ordered
            .iter()
            .filter(|(_, w)| {
                preferences.show_running
                    && (!preferences.current_desktop || w.workspace == desktop || w.minimized)
            })
            .map(|(id, _)| id.clone()),
    );
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for id in ids {
        if result.len() == 64 || !seen.insert(id.clone()) {
            continue;
        }
        let app = apps.iter().find(|app| app.item.id == id);
        let own: Vec<Window> = ordered
            .iter()
            .filter(|(known, w)| {
                known == &id
                    && (!preferences.current_desktop || w.workspace == desktop || w.minimized)
            })
            .map(|(_, window)| (*window).clone())
            .collect();
        let target = app.and_then(|app| match &app.item.target {
            Some(Target::App(target)) => Some(target.clone()),
            _ => None,
        });
        result.push(Group {
            id: id.clone(),
            title: app
                .map(|a| a.item.title.clone())
                .or_else(|| {
                    own.first().map(|w| {
                        if generic_class(&w.class) && !w.title.is_empty() {
                            w.title.clone()
                        } else {
                            w.class.clone()
                        }
                    })
                })
                .unwrap_or_else(|| {
                    id.trim_start_matches("app:")
                        .trim_end_matches(".desktop")
                        .to_owned()
                }),
            icon: app.map_or_else(|| "app-window".into(), |a| a.item.icon.clone()),
            image: app
                .and_then(|a| a.item.image.clone())
                .or_else(|| {
                    own.first()
                        .and_then(|w| crate::search::catalogue::icon_for_app_id(&w.class))
                })
                .or_else(|| {
                    crate::icon_theme::lookup(
                        "application-x-executable",
                        u32::from(preferences.icon_size),
                    )
                }),
            app: target,
            pinned: preferences.pins.contains(&id),
            builtin: None,
            windows: own,
        });
    }
    let builtins = preferences
        .folders
        .iter()
        .cloned()
        .map(Builtin::Folder)
        .chain(
            [
                (preferences.overview, Action::Overview),
                (preferences.applications, Action::Applications),
                (preferences.downloads, Action::Downloads),
            ]
            .into_iter()
            .filter(|(enabled, _)| *enabled)
            .map(|(_, action)| Builtin::Action(action)),
        );
    for builtin in builtins {
        let id = match &builtin {
            Builtin::Folder(uri) => format!("folder:{uri}"),
            Builtin::Action(Action::Overview) => "dock:overview".into(),
            Builtin::Action(Action::Applications) => "dock:apps".into(),
            Builtin::Action(Action::Downloads) => "dock:downloads".into(),
        };
        result.push(Group {
            id,
            title: builtin.title(),
            icon: builtin.icon().into(),
            image: None,
            app: None,
            pinned: matches!(builtin, Builtin::Folder(_)),
            builtin: Some(builtin),
            windows: Vec::new(),
        });
    }
    result
}
#[derive(Clone, Copy, Debug)]
pub struct Slots {
    pub icon: f32,
    pub target: f32,
    pub spacing: f32,
}

impl Slots {
    pub fn new(p: &Preferences, metrics: crate::settings::Metrics) -> Self {
        let requested = f32::from(p.icon_size);
        let (icon, target) = if p.position == Position::Bar {
            let target = f32::from(metrics.bar_item_height());
            (requested.min(target - 12.0), target)
        } else {
            (requested, (requested + 14.0).max(36.0))
        };
        Self {
            icon,
            target,
            spacing: f32::from(p.spacing),
        }
    }

    pub fn span(self, count: usize) -> f32 {
        count as f32 * self.target + count.saturating_sub(1) as f32 * self.spacing
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub screen: Size,
    pub body: Rectangle,
    pub sensor: Rectangle,
    pub vertical: bool,
    pub padding: [f32; 2],
    pub radius: f32,
    pub framed: bool,
    pub reserve: f32,
    pub output: String,
    pub output_id: i64,
    pub desktop: i64,
}
pub fn geometry(
    p: &Preferences,
    count: usize,
    screen: Size,
    bar: crate::settings::BarEdge,
    metrics: crate::settings::Metrics,
    edges: (u32, u32),
) -> Geometry {
    let vertical = matches!(p.position, Position::Left | Position::Right);
    let framed = p.style == Style::Desktop && metrics.islands.is_none();
    let island = metrics.islands.filter(|_| p.style == Style::Desktop);
    let inset = island.map_or_else(
        || if framed { 0.0 } else { f32::from(p.margin) },
        |style| {
            f32::from(if vertical {
                style.side_gap
            } else {
                style.edge_gap
            })
        },
    );
    let slots = Slots::new(p, metrics);
    let chrome_thickness = match (island, vertical) {
        (Some(style), true) => f32::from(style.rail),
        (Some(style), false) => f32::from(style.height),
        (None, true) if framed => f32::from(metrics.rail),
        (None, false) if framed => f32::from(metrics.bar),
        _ => 0.0,
    };
    let spacing = enzui_theme::SpacingScale::default();
    let thickness = (slots.target + spacing.xs()).max(chrome_thickness);
    let cross_padding = (thickness - slots.target) / 2.0;
    let axis_padding = island.map_or(spacing.space5, |style| f32::from(style.padding));
    let span_inset = island.map_or(inset, |style| {
        f32::from(if vertical {
            style.edge_gap
        } else {
            style.side_gap
        })
    });
    let padding = if vertical {
        [axis_padding, cross_padding]
    } else {
        [cross_padding, axis_padding]
    };
    let (edge_left, edge_right) = (edges.0 as f32, edges.1 as f32);
    let frame = f32::from(metrics.frame);
    let usable_edge = |edge: f32| {
        if island.is_some() {
            (edge - inset).max(0.0)
        } else if framed && edge == frame {
            0.0
        } else {
            edge
        }
    };
    let left = if vertical {
        usable_edge(edge_left)
    } else if island.is_some() {
        edge_left.max(span_inset)
    } else {
        edge_left
    };
    let right = if vertical {
        usable_edge(edge_right)
    } else if island.is_some() {
        edge_right.max(span_inset)
    } else {
        edge_right
    };
    let bar_band = f32::from(metrics.bar_band());
    let top = if bar == crate::settings::BarEdge::Top {
        bar_band
    } else {
        0.0
    };
    let bottom = if bar == crate::settings::BarEdge::Bottom {
        bar_band
    } else {
        0.0
    };
    let available = if vertical {
        screen.height - top - bottom - 2.0 * span_inset
    } else {
        if island.is_some() {
            screen.width - left - right
        } else {
            screen.width - left - right - 2.0 * span_inset
        }
    };
    let span = if p.style == Style::Panel {
        available
    } else {
        (slots.span(count.max(1)) + 2.0 * axis_padding).min(available)
    }
    .max(1.0);
    let body = match p.position {
        Position::Left => Rectangle {
            x: left + inset,
            y: top + (screen.height - top - bottom - span) / 2.0,
            width: thickness,
            height: span,
        },
        Position::Right => Rectangle {
            x: screen.width - right - inset - thickness,
            y: top + (screen.height - top - bottom - span) / 2.0,
            width: thickness,
            height: span,
        },
        Position::Top => Rectangle {
            x: left + (screen.width - left - right - span) / 2.0,
            y: top + inset,
            width: span,
            height: thickness,
        },
        _ => Rectangle {
            x: left + (screen.width - left - right - span) / 2.0,
            y: screen.height - bottom - inset - thickness,
            width: span,
            height: thickness,
        },
    };
    let sensor = match p.position {
        Position::Left => Rectangle {
            x: 0.0,
            y: body.y,
            width: 3.0,
            height: body.height,
        },
        Position::Right => Rectangle {
            x: screen.width - 3.0,
            y: body.y,
            width: 3.0,
            height: body.height,
        },
        Position::Top => Rectangle {
            x: body.x,
            y: 0.0,
            width: body.width,
            height: 3.0,
        },
        _ => Rectangle {
            x: body.x,
            y: screen.height - 3.0,
            width: body.width,
            height: 3.0,
        },
    };
    let covered = match p.position {
        Position::Left => edge_left - left,
        Position::Right => edge_right - right,
        Position::Top | Position::Bottom if framed => frame,
        _ => 0.0,
    };
    let radius = if framed {
        f32::from(metrics.radius).min(thickness / 2.0)
    } else if let Some(style) = island {
        if f32::from(style.radius) >= f32::from(style.height) / 2.0 {
            thickness.min(span) / 2.0
        } else {
            f32::from(style.radius)
        }
    } else {
        f32::from(p.radius)
    };
    Geometry {
        screen,
        body,
        sensor,
        vertical,
        padding,
        radius,
        framed,
        reserve: (inset + thickness - covered).max(0.0),
        output: String::new(),
        output_id: 0,
        desktop: 1,
    }
}
pub fn fullscreen(geometry: &Geometry, windows: &[Window]) -> bool {
    windows.iter().any(|w| {
        w.output == geometry.output_id
            && w.workspace == geometry.desktop
            && w.fullscreen
            && !w.minimized
    })
}

pub fn input_regions(
    geometry: &Geometry,
    preferences: &Preferences,
    hidden: bool,
    fullscreen: bool,
    preview: Option<Rectangle>,
) -> Vec<(Rectangle, f32)> {
    let mut regions = Vec::new();
    if preferences.enabled && preferences.position != Position::Bar {
        if !fullscreen
            && matches!(
                preferences.visibility,
                Visibility::Always | Visibility::Overlap
            )
        {
            regions.push((geometry.sensor, 0.0));
        }
        if !hidden {
            let radius = if preferences.style == Style::Minimal || geometry.framed {
                0.0
            } else {
                geometry.radius
            };
            regions.push((geometry.body, radius));
        }
    }
    if let Some(preview) = preview {
        regions.push((preview, 10.0));
    }
    regions
}

pub fn obscured(p: &Preferences, geometry: &Geometry, windows: &[Window]) -> bool {
    match p.visibility {
        Visibility::Never => false,
        Visibility::Always => true,
        Visibility::Fullscreen => fullscreen(geometry, windows),
        Visibility::Overlap => windows.iter().any(|w| {
            w.output == geometry.output_id
                && w.workspace == geometry.desktop
                && !w.minimized
                && w.bounds.intersects(&geometry.body)
        }),
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub windows: Vec<Window>,
    pub screen: Size,
    pub output: String,
    pub output_id: i64,
    pub desktop: i64,
}
impl crate::compositor::Compositor {
    pub async fn dock_snapshot(&self, apps: &[App]) -> Result<Snapshot, String> {
        let output = self.desktop_output().await?;
        let text = self.request(&["-j", "clients"]).await?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-window-inventory-is-too-large").into());
        }
        let clients: Vec<serde_json::Value> =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-dock-window-inventory"))?;
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?)
                .map_err(|_| tr("shell-invalid-focused-window"))?;
        let mut windows = Vec::new();
        for value in clients.iter().take(512) {
            if value["mapped"] != true
                || value["pid"].as_u64() == Some(u64::from(std::process::id()))
            {
                continue;
            }
            let tags = value["tags"].as_array();
            if tags.is_some_and(|tags| {
                tags.iter().any(|t| {
                    t.as_str()
                        .is_some_and(|t| t.starts_with("enzremote-guard-"))
                })
            }) {
                continue;
            }
            let Some(target) = crate::search::compositor::target(value) else {
                continue;
            };
            let class = value["class"].as_str().unwrap_or("");
            let app = match_app(apps, value, &target);
            let workspace = value["workspace"]["id"].as_i64().unwrap_or(0);
            let pair = |key: &str| -> Option<[f32; 2]> {
                Some([
                    value[key][0].as_f64()? as f32,
                    value[key][1].as_f64()? as f32,
                ])
            };
            let Some([x, y]) = pair("at") else { continue };
            let Some([width, height]) = pair("size") else {
                continue;
            };
            if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0
            {
                continue;
            }
            windows.push(Window {
                active: active["stableId"].as_str() == Some(&target.stable_id),
                urgent: value["urgent"] == true,
                target,
                app_id: app.map(|a| a.item.id.clone()),
                title: crate::search::catalogue::display_text(
                    value["title"].as_str().unwrap_or(class),
                    180,
                ),
                class: crate::search::catalogue::display_text(class, 128),
                workspace,
                output: value["monitor"].as_i64().unwrap_or(-1),
                bounds: Rectangle {
                    x,
                    y,
                    width,
                    height,
                },
                fullscreen: value["fullscreen"].as_i64() == Some(2)
                    || tags.is_some_and(|tags| {
                        tags.iter().any(|tag| tag == "enzui-borderless-fullscreen")
                    }),
                minimized: value["hidden"] == true || workspace <= 0,
            });
        }
        let scale = output["scale"]
            .as_f64()
            .filter(|v| *v > 0.0)
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen = Size::new(
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32,
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32,
        );
        let ox = output["x"].as_f64().unwrap_or(0.0) as f32;
        let oy = output["y"].as_f64().unwrap_or(0.0) as f32;
        for window in &mut windows {
            window.bounds.x -= ox;
            window.bounds.y -= oy;
        }
        Ok(Snapshot {
            windows,
            screen,
            output: output["name"]
                .as_str()
                .ok_or_else(|| tr("shell-missing-output-name"))?
                .into(),
            output_id: output["id"].as_i64().unwrap_or(0),
            desktop: output["activeWorkspace"]["id"].as_i64().unwrap_or(1),
        })
    }
    pub async fn prepare_dock(&self, snapshot: &Snapshot) -> Result<(), String> {
        let lua = format!(
            "{}\nassert(hl.plugin.enzui_sidebar_scene and hl.plugin.enzui_sidebar_scene.supports_dock and hl.plugin.enzui_sidebar_scene.supports_dock(), \"This compositor needs dock input-region support\")\nhl.window_rule({{name=\"enzui-dock\",match={{class=\"^dev[.]enzui[.]Dock$\"}},float=true,pin=true,monitor={},move=\"0 0\",size={},border_size=0,rounding=0,decorate=false,no_shadow=true,no_anim=true,no_initial_focus=true,no_xdg_drags=true}})\nreturn \"ok\"",
            include_str!("../../../../config/dock-policy.lua"),
            serde_json::to_string(&snapshot.output).expect("serializing a string cannot fail"),
            serde_json::to_string(&format!(
                "{} {}",
                snapshot.screen.width, snapshot.screen.height
            ))
            .expect("serializing a string cannot fail")
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-dock-rule-rejected").into());
        }
        Ok(())
    }
    pub async fn place_dock(&self, geometry: &Geometry) -> Result<(), String> {
        let lua = format!(
            "{}\nreturn enzui_dock_place({}, {}, {}, {})",
            include_str!("../../../../config/dock-policy.lua"),
            std::process::id(),
            serde_json::to_string(&geometry.output).expect("serializing a string cannot fail"),
            geometry.screen.width,
            geometry.screen.height
        );
        match self.request(&["eval", &lua]).await?.trim() {
            "ok" | "pending" => Ok(()),
            _ => Err(tr("shell-dock-placement-rejected").into()),
        }
    }
    pub async fn dock_capture_target(&self, target: &WindowTarget) -> bool {
        let start = tokio::fs::read_to_string(format!("/proc/{}/stat", target.pid))
            .await
            .ok()
            .as_deref()
            .and_then(crate::window_feed::start_ticks);
        if start != Some(target.start) || target.pid == std::process::id() {
            return false;
        }
        let lua = format!(
            "{}\nlocal w=enzui_dock_target({}, {}, {}); return w~=nil and not w.hidden and w.workspace and w.workspace.id>0",
            include_str!("../../../../config/dock-policy.lua"),
            serde_json::to_string(&target.address).expect("serializing a string cannot fail"),
            target.pid,
            serde_json::to_string(&target.stable_id).expect("serializing a string cannot fail"),
        );
        self.request(&["repl", &lua])
            .await
            .is_ok_and(|value| value.trim() == "true")
    }

    pub async fn dock_window_action(
        &self,
        target: WindowTarget,
        action: &'static str,
    ) -> Result<(), String> {
        let start = std::fs::read_to_string(format!("/proc/{}/stat", target.pid))
            .ok()
            .as_deref()
            .and_then(crate::window_feed::start_ticks);
        if start != Some(target.start) || target.pid == std::process::id() {
            return Err(tr("shell-window-owner-changed").into());
        }
        if target.sidebar.is_some() && action == "focus" {
            return self.focus_search_result(target).await;
        }
        if target.sidebar.is_some() && action == "minimize" {
            return Err(tr("shell-pinned-miniapps-cannot-be-minimized").into());
        }
        let lua = format!(
            "{}\nreturn enzui_dock_action({}, {}, {}, {})",
            include_str!("../../../../config/dock-policy.lua"),
            serde_json::to_string(&target.address).expect("serializing a string cannot fail"),
            target.pid,
            serde_json::to_string(&target.stable_id).expect("serializing a string cannot fail"),
            serde_json::to_string(action).expect("serializing a string cannot fail")
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-dock-window-action-rejected").into());
        }
        Ok(())
    }
}

impl crate::compositor::Compositor {
    pub async fn dock_menu_point(
        &self,
        class: &str,
        invocation: enzui_core::context_menu::InvocationKind,
    ) -> Result<iced::Point, String> {
        let output = self.desktop_output().await?;
        let ox = output["x"].as_f64().unwrap_or(0.0) as f32;
        let oy = output["y"].as_f64().unwrap_or(0.0) as f32;
        if matches!(
            invocation,
            enzui_core::context_menu::InvocationKind::Pointer(_)
        ) {
            let value: serde_json::Value =
                serde_json::from_str(&self.request(&["-j", "cursorpos"]).await?)
                    .map_err(|_| tr("shell-invalid-pointer"))?;
            return Ok(iced::Point::new(
                value["x"].as_f64().ok_or("Missing pointer x")? as f32 - ox,
                value["y"].as_f64().ok_or("Missing pointer y")? as f32 - oy,
            ));
        }
        let values: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "clients"]).await?)
                .map_err(|_| tr("shell-invalid-windows"))?;
        let window = values
            .as_array()
            .and_then(|values| {
                values.iter().find(|v| {
                    v["pid"].as_u64() == Some(u64::from(std::process::id()))
                        && v["class"].as_str() == Some(class)
                })
            })
            .ok_or_else(|| tr("shell-menu-invoker-unavailable"))?;
        let point = invocation.anchor();
        Ok(iced::Point::new(
            window["at"][0].as_f64().unwrap_or(0.0) as f32 + point.x - ox,
            window["at"][1].as_f64().unwrap_or(0.0) as f32 + point.y - oy,
        ))
    }
}

impl crate::compositor::Compositor {
    pub async fn dock_policy(
        &self,
        geometry: &Geometry,
        preferences: &Preferences,
        hidden: bool,
        fullscreen: bool,
        menu: bool,
        preview: Option<Rectangle>,
    ) -> Result<(), String> {
        let values: Vec<[f32; 4]> =
            input_regions(geometry, preferences, hidden, fullscreen, preview)
                .iter()
                .map(|(r, _)| [r.x, r.y, r.width, r.height])
                .collect();
        let side = match preferences.position {
            Position::Top => "top",
            Position::Left => "left",
            Position::Right => "right",
            _ => "bottom",
        };
        let reserve = if preferences.enabled
            && preferences.position != Position::Bar
            && matches!(
                preferences.visibility,
                Visibility::Never | Visibility::Fullscreen
            ) {
            geometry.reserve
        } else {
            0.0
        };
        let lua = format!(
            "{}\n{}\nreturn enzui_dock_update({}, {}, {}, {}, {}, {}, {}, {})",
            include_str!("../../../../config/shell-reservations.lua"),
            include_str!("../../../../config/dock-policy.lua"),
            std::process::id(),
            serde_json::to_string(&geometry.output).expect("serializing a string cannot fail"),
            format_args!(
                "{{{}}}",
                values
                    .iter()
                    .map(|r| format!("{{{},{},{},{}}}", r[0], r[1], r[2], r[3]))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            menu,
            serde_json::to_string(side).expect("serializing a string cannot fail"),
            reserve,
            format_args!(
                "{{{},{},{},{}}}",
                geometry.body.x, geometry.body.y, geometry.body.width, geometry.body.height
            ),
            preferences.enabled
                && preferences.position != Position::Bar
                && preferences.visibility == Visibility::Overlap
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-dock-input-policy-rejected").into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AppTarget, Category};
    fn window(app: &str, workspace: i64, index: u32) -> Window {
        Window {
            target: WindowTarget {
                address: format!("0x{index:x}"),
                stable_id: format!("{index:x}"),
                pid: 1000 + index,
                start: 1,
                sidebar: None,
            },
            app_id: Some(format!("app:{app}")),
            title: format!("Window {index}"),
            class: app.into(),
            workspace,
            output: 0,
            bounds: Rectangle {
                x: 100.0,
                y: 100.0,
                width: 400.0,
                height: 300.0,
            },
            fullscreen: false,
            active: false,
            urgent: false,
            minimized: workspace < 0,
        }
    }

    #[test]
    fn matched_dock_keeps_joined_bar_insets_without_spanning_the_output() {
        use crate::{
            settings::{BarEdge, Metrics},
            shell_style::Islands,
        };
        let islands = Islands {
            joined_bar: true,
            edge_gap: 10,
            side_gap: 20,
            ..Islands::default()
        };
        let metrics = Metrics {
            bar: islands.height,
            islands: Some(islands),
            ..Metrics::default()
        };
        let preferences = Preferences {
            enabled: true,
            ..Preferences::default()
        };
        let screen = Size::new(960.0, 600.0);
        let dock = geometry(&preferences, 6, screen, BarEdge::Top, metrics, (20, 20));

        assert_eq!(
            dock.body.width,
            Slots::new(&preferences, metrics).span(6) + 32.0
        );
        assert_eq!(dock.body.center_x(), screen.width / 2.0);
        assert_eq!(dock.body.y + dock.body.height, screen.height - 10.0);
        assert_eq!(dock.radius, dock.body.height / 2.0);
        assert_eq!(
            input_regions(&dock, &preferences, false, false, None),
            vec![(dock.body, dock.radius)]
        );
    }

    #[test]
    fn desktop_identity_selects_the_pin_independently_of_catalogue_order() {
        let root = tempfile::tempdir().unwrap();
        let desktop = |id: &str, fields: &str| {
            let filename = format!("{id}.desktop");
            let path = root.path().join(&filename);
            std::fs::write(&path, format!("[Desktop Entry]\n{fields}")).unwrap();
            let identity = crate::search::catalogue::FileIdentity::capture(&path).unwrap();
            App::from_item(Item::new(
                format!("app:desktop/{filename}"),
                Category::Apps,
                id,
                "",
                "Open",
                "app-window",
                Some(Target::App(AppTarget::Desktop { path, identity })),
            ))
            .unwrap()
        };
        let browser = desktop("chromium", "StartupWMClass=org.example.App\n");
        let pwa = desktop("chromium-app-Default", "StartupWMClass=crx_demo\n");
        let flatpak = desktop("org.example.App", "X-Flatpak=org.example.Variant\n");
        let duplicate = desktop("other-app", "StartupWMClass=crx_demo\n");
        let apps = [browser, pwa, flatpak, duplicate];
        let mut running = window("unidentified", 1, 1);
        let identify = |client, catalogue| {
            match_app(catalogue, &client, &window("unidentified", 1, 1).target)
                .map(|app| app.item.id.clone())
        };
        let app_id = identify(
            serde_json::json!({"class": "org.example.APP.desktop", "initialClass": "chromium"}),
            &apps,
        );
        assert_eq!(app_id.as_deref(), Some(apps[2].item.id.as_str()));
        running.app_id = app_id;
        let prefs = Preferences {
            pins: vec![apps[2].item.id.clone()],
            ..Default::default()
        };
        let groups = groups(&prefs, &apps, &[running], 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].windows.len(), 1);
        assert_eq!(
            identify(
                serde_json::json!({"class": "Electron", "initialClass": "CRX_demo"}),
                &apps[..3]
            ),
            Some(apps[1].item.id.clone())
        );
        assert_eq!(
            identify(
                serde_json::json!({"class": "crx_demo", "initialClass": "chromium"}),
                &apps[..3]
            ),
            Some(apps[1].item.id.clone())
        );
        assert_eq!(
            identify(serde_json::json!({"class": "crx_demo"}), &apps),
            None
        );
        assert_eq!(
            identify(serde_json::json!({"class": "org.example.Variant"}), &apps),
            Some(apps[2].item.id.clone())
        );
    }
}
