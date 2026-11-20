use crate::{
    catalog,
    settings::{BarEdge, SidebarEdge},
};
use enzui_shell::i18n::tr;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Toggle(u32, i32),
    Unpin(u32),
    RestoreIcon {
        id: u32,
        owner: u32,
        x: i32,
        y: i32,
    },
    IconFeedback {
        id: u32,
        owner: u32,
        light: bool,
        x: i32,
        y: i32,
    },
    CancelIconFeedback(u32),
    Hide,
    ClearError,
}
impl Action {
    pub fn lua(self) -> String {
        match self {
            Self::Toggle(id, anchor) => format!("enzui_sidebar_toggle({id},{anchor})"),
            Self::Unpin(id) => format!("enzui_sidebar_unpin({id},false)"),
            Self::RestoreIcon { id, owner, x, y } => {
                format!("enzui_sidebar_restore_icon({id},{owner},{x},{y})")
            }
            Self::IconFeedback {
                id,
                owner,
                light,
                x,
                y,
            } => {
                format!("enzui_sidebar_icon_feedback({id},{owner},{light},{x},{y})")
            }
            Self::CancelIconFeedback(owner) => {
                format!("enzui_sidebar_cancel_icon_feedback({owner})")
            }
            Self::Hide => "enzui_sidebar_hide()".into(),
            Self::ClearError => "enzui_sidebar_clear_error()".into(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u32,
    pub pid: u32,
    pub available: bool,
    pub app: Option<&'static catalog::App>,
    /// The application's icon theme icon, from its window's app id.
    pub image: Option<std::path::PathBuf>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub output: String,
    pub enabled: bool,
    pub side: SidebarEdge,
    pub bar: BarEdge,
    pub origin: [i32; 2],
    pub reveal: bool,
    pub drop_hover: bool,
    pub drop_add: bool,
    pub canvas: Option<Rect>,
    pub icon_drag: bool,
    pub autohidden: bool,
    pub keyboard_reveal: bool,
    pub native: bool,
    pub scene_version: u8,
    pub shown: Option<u32>,
    pub error: String,
    pub rail: Rect,
    pub pane: Rect,
    /// How far the shown app sits inside the pane, on every side.
    pub inset: u16,
    pub entries: Vec<Entry>,
}
impl Snapshot {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 32768 {
            return Err(tr("shell-sidebar-snapshot-exceeds-its-limit").into());
        }
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|_| tr("shell-invalid-sidebar-snapshot"))?;
        let output = v["output"]
            .as_str()
            .filter(|s| crate::desktop_frame::valid_output(s))
            .ok_or_else(|| tr("shell-invalid-sidebar-output"))?
            .to_owned();
        let side = match v["side"].as_str() {
            Some("left") => SidebarEdge::Left,
            Some("right") => SidebarEdge::Right,
            _ => return Err(tr("shell-invalid-sidebar-edge").into()),
        };
        let bar = match v["bar"].as_str() {
            Some("top") => BarEdge::Top,
            Some("bottom") => BarEdge::Bottom,
            _ => return Err(tr("shell-invalid-sidebar-bar-edge").into()),
        };
        let position = |v: &serde_json::Value| {
            v.as_i64()
                .filter(|v| v.unsigned_abs() <= 100000)
                .map(|v| v as i32)
                .ok_or_else(|| tr("shell-invalid-sidebar-position"))
        };
        let rect = |v: &serde_json::Value| -> Result<Rect, String> {
            let dimension = |v: &serde_json::Value| {
                v.as_u64()
                    .filter(|v| (1..=16384).contains(v))
                    .map(|v| v as u32)
                    .ok_or_else(|| tr("shell-invalid-sidebar-dimension"))
            };
            Ok(Rect {
                x: position(&v[0])?,
                y: position(&v[1])?,
                width: dimension(&v[2])?,
                height: dimension(&v[3])?,
            })
        };
        let origin = [position(&v["origin"][0])?, position(&v["origin"][1])?];
        let rail = rect(&v["rail"])?;
        let pane = rect(&v["pane"])?;
        let canvas = v.get("canvas").map(rect).transpose()?;
        if let Some(canvas) = canvas
            && (canvas.x != rail.x
                || canvas.width != rail.width
                || canvas.y > rail.y
                || i64::from(canvas.y) + i64::from(canvas.height)
                    < i64::from(rail.y) + i64::from(rail.height))
        {
            return Err(tr("shell-sidebar-capsule-is-outside-its-canvas").into());
        }
        if !(32..=96).contains(&rail.width) || pane.width > 1536 || pane.height > 1024 {
            return Err(tr("shell-unsupported-sidebar-surface-bounds").into());
        }
        let rows = v["entries"]
            .as_array()
            .filter(|r| r.len() <= 16)
            .ok_or_else(|| tr("shell-invalid-sidebar-entry-count"))?;
        let mut ids = BTreeSet::new();
        let mut entries = Vec::new();
        for row in rows {
            let id = row["id"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| tr("shell-invalid-miniapp-id"))?;
            let pid = row["pid"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 1)
                .ok_or_else(|| tr("shell-invalid-miniapp-pid"))?;
            if !ids.insert(id) {
                return Err(tr("shell-duplicate-miniapp-id").into());
            }
            let available = row["available"]
                .as_bool()
                .ok_or_else(|| tr("shell-missing-miniapp-availability"))?;
            let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
            let app = executable
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .and_then(|name| catalog::APPS.iter().find(|app| app.id == name));
            let image = row["class"]
                .as_str()
                .and_then(app_icon)
                .or_else(|| app.and_then(|app| app.themed_icon(48)));
            entries.push(Entry {
                id,
                pid,
                available,
                app,
                image,
            });
        }
        let shown = v["shown"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| tr("shell-invalid-shown-miniapp"))?;
        if shown != 0 && !ids.contains(&shown) {
            return Err(tr("shell-shown-miniapp-is-missing").into());
        }
        let error = v["error"]
            .as_str()
            .filter(|e| {
                matches!(
                    *e,
                    "" | "unavailable"
                        | "multi-window"
                        | "restore-needed"
                        | "ineligible"
                        | "desktop-return"
                )
            })
            .ok_or_else(|| tr("shell-invalid-sidebar-status"))?
            .to_owned();
        let native = match v.get("native") {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| tr("shell-invalid-sidebar-presentation-mode"))?,
        };
        let scene_version = match v.get("scene_version") {
            None => u8::from(native),
            Some(value) => value
                .as_u64()
                .filter(|n| *n <= 8)
                .ok_or_else(|| tr("shell-invalid-sidebar-scene-version"))?
                as u8,
        };
        if canvas.is_some() && scene_version < 8 {
            return Err(tr("shell-dynamic-sidebar-needs-a-shaped-native-canvas").into());
        }
        if native != (scene_version != 0) {
            return Err(tr("shell-inconsistent-sidebar-scene-version").into());
        }
        Ok(Self {
            output,
            enabled: v.get("enabled").map_or(Ok(true), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-sidebar-enabled-state"))
            })?,
            side,
            bar,
            origin,
            rail,
            pane,
            inset: v.get("inset").map_or(Ok(0), |value| {
                value
                    .as_u64()
                    .and_then(|inset| u16::try_from(inset).ok())
                    .filter(|inset| *inset <= 64)
                    .ok_or_else(|| tr("shell-invalid-sidebar-snapshot"))
            })?,
            canvas,
            icon_drag: v.get("icon_drag").map_or(Ok(false), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-icon-drag-capability"))
            })?,
            entries,
            error,
            autohidden: v.get("autohidden").map_or(Ok(false), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-sidebar-autohide-state"))
            })?,
            keyboard_reveal: v.get("keyboard_reveal").map_or(Ok(false), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-sidebar-keyboard-state"))
            })?,
            shown: (shown != 0).then_some(shown),
            native,
            scene_version,
            reveal: v["reveal"]
                .as_bool()
                .ok_or_else(|| tr("shell-missing-sidebar-visibility"))?,
            drop_hover: v.get("drop_hover").map_or(Ok(false), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-sidebar-drop-hover"))
            })?,
            drop_add: v.get("drop_add").map_or(Ok(false), |value| {
                value
                    .as_bool()
                    .ok_or_else(|| tr("shell-invalid-sidebar-insertion-marker"))
            })?,
        })
    }
    pub fn backing_class(&self) -> &'static str {
        if self.scene_version >= 2 {
            "dev.enzui.SidebarBacking"
        } else {
            "dev.enzui.SidebarPane"
        }
    }
    pub fn visible(&self) -> bool {
        self.enabled
            && !self.autohidden
            && (self.reveal || !self.entries.is_empty() || !self.error.is_empty())
    }
    pub fn native_rail(&self) -> bool {
        self.scene_version >= 7
    }
    pub fn source_rect(&self) -> Rect {
        self.canvas.unwrap_or(self.rail)
    }
    pub fn icon_center(&self, index: usize, slot: u16, scroll: f32) -> i32 {
        let count = (self.entries.len()
            + usize::from(!self.error.is_empty())
            + usize::from(self.drop_add && !self.entries.is_empty()))
        .max(1);
        let slot = f32::from(slot);
        let block = count as f32 * slot + (count - 1) as f32 * 6.0;
        let available = self.rail.height as f32 - 16.0;
        let top = 8.0 + ((available - block) / 2.0).max(0.0);
        let scroll = if scroll.is_finite() {
            scroll.clamp(0.0, (block - available).max(0.0))
        } else {
            0.0
        };
        (self.rail.y as f32 + top + index as f32 * (slot + 6.0) + slot / 2.0 - scroll).round()
            as i32
    }
    /// The pane's corner radius around an app whose window corners are
    /// `rounding`: the same curve, `inset` further out.
    pub fn pane_radius(&self, rounding: u16) -> f32 {
        f32::from(rounding.saturating_add(self.inset))
    }
    pub fn pane_visible(&self) -> bool {
        self.shown
            .is_some_and(|id| self.entries.iter().any(|e| e.id == id && e.available))
    }
    pub fn notice(&self) -> &'static str {
        match self.error.as_str() {
            "unavailable" => tr("shell-this-app-is-controlled-elsewhere"),
            "multi-window" => tr("shell-finish-the-app-s-dialog-before-hiding-it"),
            "desktop-return" => tr("shell-app-returned-to-the-desktop-for-an-additional-window"),
            "restore-needed" => tr("shell-app-restoration-needs-retrying-right-click-its-icon"),
            "ineligible" => {
                tr("shell-this-window-cannot-be-pinned-dialog-fullscreen-group-or-app-limit")
            }
            _ => tr("shell-drag-an-application-window-here-to-pin-it"),
        }
    }
}

// A window app id's icon, cached until the icon theme changes: snapshots
// arrive on every rail change and the lookup reads the filesystem.
fn app_icon(class: &str) -> Option<std::path::PathBuf> {
    use std::{collections::HashMap, sync::Mutex};
    type Icons = HashMap<String, Option<std::path::PathBuf>>;
    static CACHE: Mutex<(u64, Option<Icons>)> = Mutex::new((0, None));
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let generation = crate::icon_theme::generation();
    if cache.0 != generation || cache.1.is_none() {
        *cache = (generation, Some(HashMap::new()));
    }
    let map = cache.1.as_mut()?;
    if map.len() > 256 {
        map.clear();
    }
    map.entry(class.to_owned())
        .or_insert_with(|| crate::search::catalogue::icon_for_app_id(class))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_behaviour() {
        // Resized panes stay bounded without rejecting a real sidebar drag.
        {
            let mut value = serde_json::json!({"output":"TEST","side":"left","bar":"top","origin":[0,0],"rail":[12,234,40,52],"pane":[60,95,539,708],"entries":[],"shown":0,"reveal":false,"error":"","native":true,"scene_version":7});
            assert_eq!(Snapshot::parse(&value.to_string()).unwrap().pane.width, 539);
            value["pane"] = serde_json::json!([60, 95, 1536, 1024]);
            assert_eq!(
                Snapshot::parse(&value.to_string()).unwrap().pane.height,
                1024
            );
            for pane in [
                serde_json::json!([60, 95, 1537, 708]),
                serde_json::json!([60, 95, 539, 1025]),
            ] {
                value["pane"] = pane;
                assert!(Snapshot::parse(&value.to_string()).is_err());
            }
        }
        // The pane is concentric with the app it holds.
        //
        // Sabotage: return `rounding` alone from `pane_radius`.
        {
            let mut value = serde_json::json!({"output":"TEST","side":"left","bar":"top","origin":[0,0],"rail":[12,234,40,52],"pane":[60,95,372,652],"inset":6,"entries":[],"shown":0,"reveal":false,"error":"","native":true,"scene_version":7});
            assert_eq!(
                Snapshot::parse(&value.to_string()).unwrap().pane_radius(22),
                28.0
            );
            value["inset"] = serde_json::json!(65);
            assert!(Snapshot::parse(&value.to_string()).is_err());
        }
        // Snapshots are bounded and foreign or inconsistent targets are rejected.
        {
            let value = serde_json::json!({"output":"TEST","side":"left","bar":"bottom","origin":[0,0],"rail":[0,0,36,864],"pane":[36,106,372,652],"entries":[{"id":1,"pid":1234,"available":true}],"shown":1,"reveal":false,"error":""});
            let state = Snapshot::parse(&value.to_string()).unwrap();
            assert!(state.visible() && state.pane_visible());
            for (key, bad) in [
                ("native", serde_json::json!("true")),
                ("scene_version", serde_json::json!(9)),
                ("scene_version", serde_json::json!(2)),
                ("output", serde_json::json!("ENZREMOTE-APP")),
                ("shown", serde_json::json!(2)),
                ("rail", serde_json::json!([0, 0, 1440, 900])),
                (
                    "entries",
                    serde_json::json!([{"id":1,"pid":0,"available":true}]),
                ),
            ] {
                let mut v = value.clone();
                v[key] = bad;
                assert!(Snapshot::parse(&v.to_string()).is_err());
            }
            assert!(Snapshot::parse(&"x".repeat(32769)).is_err());
        }
    }
}
