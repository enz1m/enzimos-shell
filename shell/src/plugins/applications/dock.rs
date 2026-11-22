use crate::search::{AppTarget, Item, Target, WindowTarget};
use enzui_shell::i18n::tr;
use iced::{Rectangle, Size};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read};

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
choice!(Style, Island, [Island => ("island",tr("shell-island")), Panel => ("panel",tr("shell-edge-panel")), Minimal => ("minimal",tr("shell-icons-only"))]);
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
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Position::Bottom,
            style: Style::Island,
            visibility: Visibility::Never,
            icon_size: 24,
            spacing: 4,
            radius: 12,
            highlight_radius: 6,
            margin: 8,
            show_running: true,
            current_desktop: false,
            pins: Vec::new(),
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
    Move(String, i8),
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
                if id == "enzui-settings" {
                    classes.push("dev.enzui.Settings".into());
                }
                if let Some(class) = crate::catalog::desktop_id(id) {
                    classes.push(class.into());
                }
            }
            AppTarget::Desktop { path, .. } => {
                let mut text = String::new();
                if std::fs::File::open(path)
                    .and_then(|file| file.take(65537).read_to_string(&mut text))
                    .is_err()
                    || text.len() > 65536
                {
                    text.clear();
                }
                let mut active = false;
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with('[') {
                        active = line == "[Desktop Entry]";
                    } else if active
                        && let Some(class) = line.strip_prefix("StartupWMClass=")
                        && !class.is_empty()
                        && class.len() <= 255
                    {
                        classes.push(class.into());
                    }
                }
                if let Some(name) = path.file_stem().and_then(|v| v.to_str()) {
                    classes.push(name.into());
                }
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
    pub minimized: bool,
}
#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub image: Option<std::path::PathBuf>,
    pub app: Option<AppTarget>,
    pub pinned: bool,
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
    let mut ordered: Vec<&Window> = windows.iter().collect();
    ordered.sort_by(|left, right| stable_window_order(left, right));
    let ids = preferences.pins.iter().cloned().chain(
        ordered
            .into_iter()
            .filter(|w| {
                preferences.show_running
                    && (!preferences.current_desktop || w.workspace == desktop || w.minimized)
            })
            .map(|w| {
                w.app_id
                    .clone()
                    .unwrap_or_else(|| format!("window:{}", w.class))
            }),
    );
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for id in ids {
        if result.len() == 64 || !seen.insert(id.clone()) {
            continue;
        }
        let app = apps.iter().find(|app| app.item.id == id);
        let mut own: Vec<Window> = windows
            .iter()
            .filter(|w| {
                w.app_id.as_ref() == Some(&id)
                    || w.app_id.is_none() && id == format!("window:{}", w.class)
            })
            .filter(|w| !preferences.current_desktop || w.workspace == desktop || w.minimized)
            .cloned()
            .collect();
        own.sort_by(stable_window_order);
        let target = app.and_then(|app| match &app.item.target {
            Some(Target::App(target)) => Some(target.clone()),
            _ => None,
        });
        result.push(Group {
            id: id.clone(),
            title: app
                .map(|a| a.item.title.clone())
                .or_else(|| own.first().map(|w| w.class.clone()))
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
            windows: own,
        });
    }
    result
}
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub screen: Size,
    pub body: Rectangle,
    pub sensor: Rectangle,
    pub vertical: bool,
    pub output: String,
    pub output_id: i64,
    pub desktop: i64,
}
pub fn geometry(
    p: &Preferences,
    count: usize,
    screen: Size,
    bar: crate::settings::BarEdge,
    bar_band: f32,
    sidebar: Option<crate::settings::SidebarEdge>,
    sidebar_band: f32,
) -> Geometry {
    let vertical = matches!(p.position, Position::Left | Position::Right);
    let inset = f32::from(p.margin);
    let slot = f32::from(p.icon_size) + 6.0;
    let thickness = f32::from(p.icon_size) + 22.0;
    let left = if sidebar == Some(crate::settings::SidebarEdge::Left) {
        sidebar_band
    } else {
        0.0
    };
    let right = if sidebar == Some(crate::settings::SidebarEdge::Right) {
        sidebar_band
    } else {
        0.0
    };
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
        screen.height - top - bottom - 2.0 * inset
    } else {
        screen.width - left - right - 2.0 * inset
    };
    let span = if p.style == Style::Panel {
        available
    } else {
        (count.max(1) as f32 * (slot + f32::from(p.spacing)) - f32::from(p.spacing) + 12.0)
            .min(available)
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
    Geometry {
        screen,
        body,
        sensor,
        vertical,
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
        if !fullscreen {
            regions.push((geometry.sensor, 0.0));
        }
        if !hidden {
            let radius = if preferences.style == Style::Minimal {
                0.0
            } else {
                f32::from(preferences.radius)
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
            if value["tags"].as_array().is_some_and(|tags| {
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
            let app = apps.iter().find(|app| {
                app.classes.iter().any(|known| {
                    known.eq_ignore_ascii_case(class)
                        || Some(known.as_str()) == value["initialClass"].as_str()
                })
            });
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
                fullscreen: value["fullscreen"].as_i64() == Some(2),
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
            && preferences.visibility == Visibility::Never
        {
            (if geometry.vertical {
                geometry.body.width
            } else {
                geometry.body.height
            }) + f32::from(preferences.margin)
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
    fn app(id: &str) -> App {
        App::from_item(Item::new(
            format!("app:{id}"),
            Category::Apps,
            id,
            "",
            "Open",
            "app-window",
            Some(Target::App(AppTarget::Prepared(id.into()))),
        ))
        .unwrap()
    }
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
            minimized: workspace < 0,
        }
    }

    #[tokio::test]
    async fn dock_behaviour() {
        // Dock behaviour.
        {
            // Pins have stable order and running windows do not make duplicate icons.
            {
                let p = Preferences {
                    pins: vec!["app:b".into(), "app:a".into()],
                    ..Default::default()
                };
                let result = groups(
                    &p,
                    &[app("a"), app("b"), app("c")],
                    &[window("a", 1, 1), window("a", 1, 2), window("c", 1, 3)],
                    1,
                );
                assert_eq!(
                    result.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
                    ["app:b", "app:a", "app:c"]
                );
                assert_eq!(result[1].windows.len(), 2);
                assert!(result[1].pinned && !result[2].pinned);
            }
            // Geometry fits all edges and overflow is bounded.
            {
                for position in Position::ALL
                    .iter()
                    .copied()
                    .filter(|p| *p != Position::Bar)
                {
                    for style in Style::ALL {
                        for icon_size in [24, 40, 64] {
                            let p = Preferences {
                                position,
                                style: *style,
                                icon_size,
                                ..Default::default()
                            };
                            let g = geometry(
                                &p,
                                64,
                                Size::new(640.0, 480.0),
                                crate::settings::BarEdge::Bottom,
                                48.0,
                                Some(crate::settings::SidebarEdge::Left),
                                50.0,
                            );
                            assert!(
                                g.body.x >= 0.0
                                    && g.body.y >= 0.0
                                    && g.body.x + g.body.width <= 640.0
                                    && g.body.y + g.body.height <= 432.0
                            );
                            assert!(
                                g.sensor.x >= 0.0
                                    && g.sensor.y >= 0.0
                                    && g.sensor.x + g.sensor.width <= 640.0
                                    && g.sensor.y + g.sensor.height <= 480.0
                            );
                        }
                    }
                }
            }
        }

        let compositor =
            crate::compositor::Compositor::new("/usr/bin/true".into(), "dock-fixture".into())
                .unwrap();
        let mut target = window("a", 1, 1).target;
        target.pid = std::process::id();
        target.start = crate::window_feed::start_ticks(
            &std::fs::read_to_string(format!("/proc/{}/stat", target.pid)).unwrap(),
        )
        .unwrap();
        assert!(
            compositor
                .dock_window_action(target.clone(), "close")
                .await
                .is_err()
        );
        target.start += 1;
        assert!(
            compositor
                .dock_window_action(target, "focus")
                .await
                .is_err()
        );
    }
}
