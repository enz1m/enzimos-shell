use super::{
    AppTarget, Category, Item, Target, WindowTarget,
    catalogue::{FileIdentity, display_text},
};
use crate::{clipboard::session, compositor::Compositor};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::{os::unix::fs::MetadataExt, path::Path};

#[derive(Clone, Copy, Debug)]
pub struct IslandPlacement {
    pub local_x: f32,
    pub bar_end: f32,
    pub side_gap: u16,
    pub far_gap: u16,
    pub bottom: bool,
    pub attachment: crate::reveal::Attachment,
    /// Joined to the frame's bar rather than floating by the islands.
    pub frame: bool,
    /// The card dims the rest of the screen.
    pub dim: bool,
    pub overview: bool,
}
impl IslandPlacement {
    fn lua(self) -> String {
        // The window keeps shadow room around the card (`reveal::margins`).
        let m = crate::reveal::margins(self.bottom, self.attachment);
        format!(
            "{{x={},bar_end={},side_gap={},far_gap={},bottom={},ml={},mt={},mr={},mb={},dim={},frame={},overview={}}}",
            self.local_x,
            self.bar_end,
            self.side_gap,
            self.far_gap,
            self.bottom,
            m.left,
            m.top,
            m.right,
            m.bottom,
            self.dim,
            self.frame,
            self.overview
        )
    }
}
#[derive(Clone, Debug)]
pub struct SearchGeometry {
    pub body_max: iced::Size,
    pub output: String,
    pub island: Option<IslandPlacement>,
}

#[derive(Clone, Debug, Default)]
pub struct Windows {
    pub items: Vec<Item>,
    pub focused: Option<WindowTarget>,
}
fn local_client(value: &serde_json::Value) -> bool {
    value["monitor"]
        .as_i64()
        .is_some_and(|monitor| monitor >= 0)
        && !value["tags"].as_array().is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_str()
                    .is_some_and(|tag| tag.starts_with("enzremote-"))
            })
        })
}
/// One window in the switcher.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchEntry {
    pub target: WindowTarget,
    pub title: String,
    pub class: String,
    pub image: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SwitcherSnapshot {
    pub revision: u64,
    pub active: bool,
    pub selected: usize,
    pub entries: Vec<SwitchEntry>,
}

pub(crate) fn target(value: &serde_json::Value) -> Option<WindowTarget> {
    let address = value["address"].as_str()?;
    if address.len() < 3
        || address.len() > 20
        || !address.starts_with("0x")
        || !address
            .strip_prefix("0x")
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    let stable_id = value["stableId"].as_str()?;
    if stable_id.is_empty()
        || stable_id.len() > 32
        || !stable_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let pid = u32::try_from(value["pid"].as_u64()?)
        .ok()
        .filter(|pid| *pid > 1)?;
    let start = session::start(pid).ok()?;
    let sidebar = value["tags"].as_array().and_then(|tags| {
        tags.iter().find_map(|tag| {
            tag.as_str()?
                .strip_prefix("enzui-sidebar-")?
                .parse::<u32>()
                .ok()
        })
    });
    Some(WindowTarget {
        address: address.into(),
        stable_id: stable_id.into(),
        pid,
        start,
        sidebar,
    })
}
fn retained_popup_focus(value: &serde_json::Value, pid: u32, popup: &str) -> bool {
    value["pid"].as_u64() == Some(u64::from(pid))
        && value["class"]
            .as_str()
            .is_some_and(|class| class == "dev.enzui.Desktop" || class == popup)
}
fn retained_search_focus(value: &serde_json::Value, pid: u32) -> bool {
    retained_popup_focus(value, pid, "dev.enzui.QuickSearch")
}
/// A window of the window spread, in global logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SpreadWindow {
    pub address: String,
    /// Monotonic identity assigned when the compositor creates the window.
    pub stable_id: u64,
    pub pid: u32,
    pub title: String,
    pub class: String,
    pub image: Option<std::path::PathBuf>,
    pub at: [f32; 2],
    pub size: [f32; 2],
}

/// The shell output's active desktop for the window spread.
#[derive(Clone, Debug, PartialEq)]
pub struct SpreadDesktop {
    /// The compositor monitor ID for the shell output.
    pub output_id: i64,
    /// The output's position and size, logical.
    pub output: [f32; 4],
    /// Physical pixels per logical pixel on the output.
    pub scale: f32,
    pub desktop: i64,
    /// Bottom-most first, as they are stacked.
    pub windows: Vec<SpreadWindow>,
}

// The windows a spread shows: the app windows (floating or tiled) on `desktop` of
// `monitor`, not fullscreen, pinned, held by the sidebar or remote.
fn spread_window(
    value: &serde_json::Value,
    monitor: i64,
    desktop: i64,
    own_pid: u32,
) -> Option<(i64, SpreadWindow)> {
    let shown = local_client(value)
        && value["pid"].as_u64() != Some(u64::from(own_pid))
        && value["mapped"] != false
        && value["hidden"] != true
        && value["pinned"] != true
        && value["fullscreen"].as_i64() == Some(0)
        && value["monitor"].as_i64() == Some(monitor)
        && value["workspace"]["id"].as_i64() == Some(desktop);
    if !shown {
        return None;
    }
    let target = target(value).filter(|t| t.sidebar.is_none())?;
    let number = |v: &serde_json::Value| {
        v.as_f64()
            .filter(|n| n.is_finite() && n.abs() < 1e6)
            .map(|n| n as f32)
    };
    let at = [number(&value["at"][0])?, number(&value["at"][1])?];
    let size = [number(&value["size"][0])?, number(&value["size"][1])?];
    if size[0] < 1.0 || size[1] < 1.0 {
        return None;
    }
    let class = value["class"].as_str().unwrap_or("");
    Some((
        value["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
        SpreadWindow {
            address: target.address,
            stable_id: u64::from_str_radix(&target.stable_id, 16).ok()?,
            pid: target.pid,
            title: display_text(value["title"].as_str().unwrap_or(""), 256),
            class: display_text(class, 128),
            image: crate::search::catalogue::icon_for_app_id(class),
            at,
            size,
        },
    ))
}

impl Compositor {
    /// The windows of the shell output's active desktop, bottom-most first.
    /// Activities keeps the oldest windows when the spread reaches its size limit.
    pub async fn spread_windows(&self, overview: bool) -> Result<SpreadDesktop, String> {
        let output = self.desktop_output().await?;
        let scale = output["scale"]
            .as_f64()
            .filter(|s| (0.5..=4.0).contains(s))
            .ok_or_else(|| tr("shell-invalid-output-scale"))?;
        let field = |key: &str| {
            output[key]
                .as_f64()
                .ok_or_else(|| tr("shell-missing-output-geometry"))
        };
        let geometry = [
            field("x")? as f32,
            field("y")? as f32,
            (field("width")? / scale).round() as f32,
            (field("height")? / scale).round() as f32,
        ];
        let monitor = output["id"]
            .as_i64()
            .ok_or_else(|| tr("shell-missing-output"))?;
        let desktop = output["activeWorkspace"]["id"]
            .as_i64()
            .ok_or_else(|| tr("shell-missing-active-desktop"))?;
        let text = self.request(&["-j", "clients"]).await?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-window-list-is-too-large").into());
        }
        let values: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-window-list"))?;
        const SPREAD_LIMIT: usize = 32;
        const CLIENT_LIMIT: usize = 512;
        let mut windows: Vec<(i64, SpreadWindow)> = values
            .as_array()
            .ok_or_else(|| tr("shell-invalid-window-list"))?
            .iter()
            .take(CLIENT_LIMIT)
            .filter_map(|value| spread_window(value, monitor, desktop, std::process::id()))
            .take(if overview { CLIENT_LIMIT } else { SPREAD_LIMIT })
            .collect();
        if overview && windows.len() > SPREAD_LIMIT {
            windows.sort_unstable_by_key(|(_, window)| window.stable_id);
            windows.truncate(SPREAD_LIMIT);
        }
        windows.sort_by_key(|(history, _)| std::cmp::Reverse(*history));
        Ok(SpreadDesktop {
            output_id: monitor,
            output: geometry,
            scale: scale as f32,
            desktop,
            windows: windows.into_iter().map(|(_, window)| window).collect(),
        })
    }

    /// Whether the keyboard is still with Search. In the overview, a press on
    /// its window spread may take it; the spread handles that press itself.
    pub async fn search_focus_retained(
        &self,
        overview: bool,
        close_prompt_pids: &[u32],
    ) -> Result<bool, String> {
        let value: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?)
                .map_err(|_| tr("shell-invalid-focused-window"))?;
        let pid = std::process::id();
        Ok(retained_search_focus(&value, pid)
            || overview
                && (retained_popup_focus(&value, pid, "dev.enzui.WindowSpread")
                    || value["pid"]
                        .as_u64()
                        .and_then(|pid| u32::try_from(pid).ok())
                        .is_some_and(|pid| close_prompt_pids.contains(&pid))))
    }

    pub async fn launcher_focus_retained(&self) -> Result<bool, String> {
        let value: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?)
                .map_err(|_| tr("shell-invalid-focused-window"))?;
        Ok(retained_popup_focus(
            &value,
            std::process::id(),
            "dev.enzui.Launcher",
        ))
    }

    /// Returns current-desktop windows in focus order, excluding shell and sidebar-held
    /// windows.
    pub async fn switcher_windows(&self) -> Result<Vec<SwitchEntry>, String> {
        let text = self.request(&["-j", "clients"]).await?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-window-list-is-too-large").into());
        }
        let values: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-window-list"))?;
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activeworkspace"]).await?)
                .map_err(|_| tr("shell-invalid-active-desktop"))?;
        let desktop = active["id"].as_i64();
        let mut entries: Vec<(i64, SwitchEntry)> = values
            .as_array()
            .ok_or_else(|| tr("shell-invalid-window-list"))?
            .iter()
            .take(512)
            .filter(|value| {
                local_client(value) && value["pid"].as_u64() != Some(u64::from(std::process::id()))
            })
            .filter(|value| {
                value["mapped"] != false
                    && value["hidden"] != true
                    && value["workspace"]["id"].as_i64() == desktop
            })
            .filter_map(|value| {
                let target = target(value).filter(|t| t.sidebar.is_none())?;
                let class = value["class"].as_str().unwrap_or("");
                Some((
                    value["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                    SwitchEntry {
                        title: display_text(value["title"].as_str().unwrap_or(""), 256),
                        class: display_text(class, 128),
                        image: crate::search::catalogue::icon_for_app_id(class),
                        target,
                    },
                ))
            })
            .collect();
        entries.sort_by_key(|(history, _)| *history);
        Ok(entries
            .into_iter()
            .map(|(_, entry)| entry)
            .take(64)
            .collect())
    }

    pub async fn switcher_snapshot(&self) -> Result<SwitcherSnapshot, String> {
        let clients = self.request(&["-j", "clients"]).await?;
        if clients.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-window-list-is-too-large").into());
        }
        let clients: serde_json::Value =
            serde_json::from_str(&clients).map_err(|_| tr("shell-invalid-window-list"))?;
        let clients = clients
            .as_array()
            .ok_or_else(|| tr("shell-invalid-window-list"))?;
        let text = self
            .request(&["--batch", "repl return enzui_switcher_snapshot()"])
            .await?;
        if text.len() > 8192 {
            return Err(tr("shell-invalid-switcher-snapshot").into());
        }
        let state: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-switcher-snapshot"))?;
        let revision = state["revision"]
            .as_u64()
            .ok_or_else(|| tr("shell-invalid-switcher-revision"))?;
        let active = state["active"]
            .as_bool()
            .ok_or_else(|| tr("shell-invalid-switcher-activity"))?;
        let selected = state["selected"]
            .as_u64()
            .filter(|i| (1..=64).contains(i))
            .ok_or("Invalid switcher selection")? as usize
            - 1;
        let ids = state["entries"]
            .as_array()
            .filter(|ids| ids.len() <= 64)
            .ok_or_else(|| tr("shell-invalid-switcher-entries"))?;
        let mut entries = Vec::new();
        for id in ids {
            let id = id
                .as_str()
                .ok_or_else(|| tr("shell-invalid-switcher-identity"))?;
            let value = clients
                .iter()
                .find(|value| value["stableId"].as_str() == Some(id))
                .ok_or_else(|| tr("shell-switcher-window-changed"))?;
            let target = target(value).ok_or_else(|| tr("shell-switcher-window-changed"))?;
            let class = value["class"].as_str().unwrap_or("");
            entries.push(SwitchEntry {
                target,
                title: display_text(value["title"].as_str().unwrap_or(""), 256),
                class: display_text(class, 128),
                image: crate::search::catalogue::icon_for_app_id(class),
            });
        }
        if active && selected >= entries.len() || !active && !entries.is_empty() {
            return Err(tr("shell-invalid-switcher-selection").into());
        }
        Ok(SwitcherSnapshot {
            revision,
            active,
            selected,
            entries,
        })
    }
    pub async fn switcher_action(&self, command: crate::commands::Command) -> Result<(), String> {
        use crate::commands::Command;
        let action = match command {
            Command::SwitchNext => "enzui_switcher_step(1,false)",
            Command::SwitchPrevious => "enzui_switcher_step(-1,false)",
            Command::SwitchAppNext => "enzui_switcher_step(1,true)",
            Command::SwitchCommit => "enzui_switcher_commit()",
            Command::SwitchCancel => "enzui_switcher_cancel()",
            _ => return Err(tr("shell-invalid-switcher-action").into()),
        };
        let reply = self
            .request(&["eval", &format!("{action};return 'ok'")])
            .await?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-switcher-action-refused").into())
        }
    }
    pub async fn switcher_pick(&self, revision: u64, index: usize) -> Result<(), String> {
        if index >= 64 {
            return Err(tr("shell-invalid-switcher-selection").into());
        }
        let reply = self
            .request(&[
                "--batch",
                &format!("repl return enzui_switcher_pick({revision},{})", index + 1),
            ])
            .await?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-switcher-selection-changed").into())
        }
    }

    pub async fn search_windows(&self) -> Result<Windows, String> {
        let text = self.request(&["-j", "clients"]).await?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-window-catalogue-is-too-large").into());
        }
        let values: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-window-catalogue"))?;
        let values = values
            .as_array()
            .ok_or_else(|| tr("shell-invalid-window-catalogue"))?;
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?)
                .map_err(|_| tr("shell-invalid-active-window"))?;
        let monitors: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors"]).await?)
                .map_err(|_| tr("shell-invalid-search-outputs"))?;
        let allowed: Vec<_> = monitors
            .as_array()
            .ok_or_else(|| tr("shell-invalid-search-outputs"))?
            .iter()
            .filter(|m| {
                m["name"]
                    .as_str()
                    .is_some_and(|name| !name.starts_with("ENZREMOTE-"))
            })
            .filter_map(|m| m["id"].as_i64())
            .collect();
        let mut result = Windows::default();
        for value in values.iter().take(512) {
            if !allowed.contains(&value["monitor"].as_i64().unwrap_or(-1))
                || !local_client(value)
                || value["pid"].as_u64() == Some(u64::from(std::process::id()))
            {
                continue;
            }
            let Some(window) = target(value) else {
                continue;
            };
            if value["workspace"]["id"].as_i64().unwrap_or(0) < 1 && window.sidebar.is_none() {
                continue;
            }
            let title = display_text(
                value["title"]
                    .as_str()
                    .unwrap_or_else(|| tr("shell-window")),
                256,
            );
            let class = display_text(
                value["class"]
                    .as_str()
                    .unwrap_or_else(|| tr("shell-application")),
                128,
            );
            let image =
                crate::search::catalogue::icon_for_app_id(value["class"].as_str().unwrap_or(""));
            if active["address"] == value["address"] && window.sidebar.is_none() {
                result.focused = Some(window.clone());
            }
            let subtitle = if window.sidebar.is_some() {
                tr!("shell-pinned-app", "arg0" => class.clone())
            } else {
                tr!("shell-desktop-2", "arg0" => class.clone(), "arg1" => value["workspace"]["name"].as_str().unwrap_or("").to_owned())
            };
            result.items.push(
                Item::new(
                    format!(
                        "window:{}:{}:{}",
                        window.pid, window.start, window.stable_id
                    ),
                    Category::Windows,
                    if title.is_empty() { class } else { title },
                    subtitle,
                    if window.sidebar.is_some() {
                        tr("shell-resume")
                    } else {
                        tr("shell-switch")
                    },
                    "app-window",
                    Some(Target::Window(window)),
                )
                .with_image(image),
            );
        }
        Ok(result)
    }
    pub async fn search_geometry(
        &self,
        metrics: crate::settings::Metrics,
        edge: crate::settings::BarEdge,
        anchor: Option<iced::Rectangle>,
    ) -> Result<SearchGeometry, String> {
        Self::search_layout(&self.desktop_output().await?, metrics, edge, anchor)
    }
    pub async fn present_search(
        &self,
        output: &str,
        size: iced::Size,
        epoch: u64,
        live: std::sync::Arc<std::sync::atomic::AtomicBool>,
        placement: Option<IslandPlacement>,
        fade: bool,
    ) -> Result<(), String> {
        let lua = format!(
            "{}\nreturn enzui_search_present({}, {}, {}, {}, {}, {}, {})",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/search-policy.lua")
            ),
            std::process::id(),
            serde_json::to_string(output).map_err(|_| tr("shell-invalid-search-output"))?,
            size.width.round(),
            size.height.round(),
            epoch,
            placement.map_or_else(|| "nil".into(), IslandPlacement::lua),
            fade,
        );
        let mut failure = String::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        for _ in 0..60 {
            if std::time::Instant::now() >= deadline {
                break;
            }
            if !live.load(std::sync::atomic::Ordering::Acquire) {
                return Err(tr("shell-search-transition-canceled").into());
            }
            match self.request(&["eval", &lua]).await {
                Ok(response) if response.trim() == "ok" => return Ok(()),
                Ok(response) => failure = response.chars().take(256).collect(),
                Err(error) => failure = error,
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        Err(tr!("shell-quick-search-surface-did-not-become-available", "arg0" => failure.clone()))
    }
    pub async fn resize_search(
        &self,
        output: &str,
        size: iced::Size,
        epoch: u64,
        placement: Option<IslandPlacement>,
    ) -> Result<(), String> {
        let lua = format!(
            "{}\nreturn enzui_search_resize({}, {}, {}, {}, {}, {})",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/search-policy.lua")
            ),
            std::process::id(),
            serde_json::to_string(output).map_err(|_| tr("shell-invalid-search-output"))?,
            size.width.round(),
            size.height.round(),
            epoch,
            placement.map_or_else(|| "nil".into(), IslandPlacement::lua),
        );
        if self.request(&["eval", &lua]).await?.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-quick-search-could-not-resize").into())
        }
    }
    pub async fn hide_search_dim(&self, epoch: u64, fade: bool) -> Result<(), String> {
        let lua = format!(
            "{}\nreturn enzui_search_hide_dim({}, {}, {})",
            include_str!("../../../config/search-policy.lua"),
            std::process::id(),
            epoch,
            fade
        );
        self.request(&["eval", &lua]).await.map(drop)
    }
    pub async fn park_search(&self, epoch: u64) -> Result<(), String> {
        let lua = format!(
            "{}\nreturn enzui_search_park({}, {epoch})",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/search-policy.lua")
            ),
            std::process::id()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        for _ in 0..60 {
            if std::time::Instant::now() >= deadline {
                break;
            }
            if self
                .request(&["eval", &lua])
                .await
                .is_ok_and(|response| response.trim() == "ok")
            {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        Err(tr("shell-quick-search-could-not-leave-the-desktop").into())
    }
    fn search_layout(
        output: &serde_json::Value,
        metrics: crate::settings::Metrics,
        bar: crate::settings::BarEdge,
        anchor: Option<iced::Rectangle>,
    ) -> Result<SearchGeometry, String> {
        let width = output["width"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-width"))?;
        let height = output["height"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-height"))?;
        let scale = output["scale"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| tr("shell-invalid-output-scale"))?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output-name"))?;
        if let Some(style) = metrics.islands {
            let screen = (width / scale).round() as f32;
            let anchor = anchor.unwrap_or_else(|| {
                iced::Rectangle::new(
                    iced::Point::new((screen - 1.0) / 2.0, 0.0),
                    iced::Size::new(1.0, f32::from(metrics.bar)),
                )
            });
            let (x, width, attachment) =
                crate::compositor::popup_layout(screen, metrics, 620.0, 0.0, 0.0, anchor)?;
            let bar_end = f32::from(metrics.bar_band());
            let available_height = (height / scale).round() as f32
                - bar_end
                - f32::from(style.edge_gap)
                - attachment.join(f32::from(metrics.radius));
            if width < 240.0 || !available_height.is_finite() || available_height < 160.0 {
                return Err(tr("shell-output-is-too-small-for-island-search").into());
            }
            return Ok(SearchGeometry {
                body_max: iced::Size::new(width.floor(), available_height.min(760.0).floor()),
                output: name.into(),
                island: Some(IslandPlacement {
                    local_x: x,
                    bar_end,
                    side_gap: style.side_gap,
                    far_gap: style.edge_gap,
                    bottom: bar == crate::settings::BarEdge::Bottom,
                    attachment,
                    frame: false,
                    overview: false,
                    // A panel joined to the bar stays one surface with it, undimmed.
                    dim: matches!(attachment, crate::reveal::Attachment::Island { .. }),
                }),
            });
        }
        let r = output["reserved"]
            .as_array()
            .filter(|r| r.len() == 4)
            .ok_or_else(|| tr("shell-missing-output-work-area"))?;
        let edge = |i: usize| {
            r[i].as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| tr("shell-invalid-output-reservation"))
        };
        // On the frame Search opens like the bar's other panels: joined to the
        // bar under the button that opened it, or under the bar's middle.
        let screen = (width / scale).round() as f32;
        let (left, right) = (edge(0)? as f32, edge(2)? as f32);
        let anchor = anchor.unwrap_or_else(|| {
            iced::Rectangle::new(
                iced::Point::new((screen - 1.0) / 2.0, 0.0),
                iced::Size::new(1.0, f32::from(metrics.bar)),
            )
        });
        let (x, panel_width, attachment) =
            crate::compositor::popup_layout(screen, metrics, 620.0, left, right, anchor)?;
        let radius = f32::from(metrics.radius);
        let available_height = (height / scale).round() as f32
            - f32::from(metrics.bar)
            - f32::from(metrics.frame)
            - attachment.join(radius)
            - 24.0;
        if panel_width < 240.0 || !available_height.is_finite() || available_height < 160.0 {
            return Err(tr("shell-output-is-too-small-for-quick-search").into());
        }
        Ok(SearchGeometry {
            body_max: iced::Size::new(panel_width.floor(), available_height.min(760.0).floor()),
            output: name.into(),
            island: Some(IslandPlacement {
                local_x: x,
                bar_end: f32::from(metrics.bar),
                side_gap: left.min(right) as u16,
                far_gap: metrics.frame,
                bottom: bar == crate::settings::BarEdge::Bottom,
                attachment,
                frame: true,
                overview: false,
                dim: matches!(attachment, crate::reveal::Attachment::Island { .. }),
            }),
        })
    }
    pub async fn focus_search_result(&self, window: WindowTarget) -> Result<(), String> {
        if session::start(window.pid).ok() != Some(window.start) {
            return Err(tr("shell-window-process-changed").into());
        }
        let lua = format!(
            "{}\nreturn enzui_search_focus_window({}, {}, {}, {})",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/search-policy.lua")
            ),
            serde_json::to_string(&window.address)
                .map_err(|_| tr("shell-invalid-window-address"))?,
            window.pid,
            serde_json::to_string(&window.stable_id)
                .map_err(|_| tr("shell-invalid-window-identity"))?,
            window
                .sidebar
                .map_or_else(|| "nil".into(), |id| id.to_string())
        );
        if self.request(&["eval", &lua]).await?.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-window-changed-or-is-controlled-elsewhere").into())
        }
    }
    /// Raise and focus a window of process `pid` (started at `start`), on
    /// whichever desktop it is; `false` when the process has no window.
    pub async fn focus_process_window(&self, pid: u32, start: u64) -> Result<bool, String> {
        if session::start(pid).ok() != Some(start) {
            return Ok(false);
        }
        // Search's helpers raise and focus without moving the pointer.
        let lua = format!(
            "{}\nlocal found\nfor _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={pid} and w.workspace and type(w.workspace.id)==\"number\" and w.workspace.id>0 then found=w break end end\nif not found then return {{ok=false}} end\npreserve_pointer(function() dispatch(hl.dsp.window.alter_zorder({{window=found,mode=\"top\"}})) dispatch(hl.dsp.focus({{window=found}})) end)\nreturn {{ok=true}}",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/search-policy.lua")
            ),
        );
        Ok(self.request(&["eval", &lua]).await?.trim() == "ok")
    }
    async fn focus_existing_app(
        &self,
        apps: &[crate::dock::App],
        app_target: &AppTarget,
    ) -> Result<bool, String> {
        let wanted = Target::App(app_target.clone());
        let mut windows = self.windows().await?;
        windows.sort_by_key(|w| w["focusHistoryID"].as_i64().unwrap_or(i64::MAX));
        for window in windows {
            if !local_client(&window) || window["mapped"] != true || window["hidden"] == true {
                continue;
            }
            if let Some(target) = target(&window)
                && crate::dock::match_app(apps, &window, &target)
                    .is_some_and(|app| app.item.target.as_ref() == Some(&wanted))
            {
                self.focus_search_result(target).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn focus_catalogue_app(&self, root: &Path, app: &AppTarget) -> Result<bool, String> {
        let root = root.to_owned();
        let apps = tokio::task::spawn_blocking(move || {
            super::catalogue::scan(&root, &super::catalogue::directories())
                .into_iter()
                .filter_map(crate::dock::App::from_item)
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| tr("shell-application-could-not-be-started"))?;
        self.focus_existing_app(&apps, app).await
    }

    pub async fn execute_search_target(&self, root: &Path, target: Target) -> Result<(), String> {
        drop(session::Target::from_environment()?.connect()?);
        let mut command = tokio::process::Command::new("/usr/bin/gio");
        match target {
            Target::App(AppTarget::Prepared(id)) => {
                if !crate::catalog::APPS.iter().any(|app| app.id == id)
                    || !root.is_absolute()
                    || !crate::catalog::executable(root, &id).is_file()
                {
                    return Err(tr("shell-application-is-no-longer-installed").into());
                }
                if !crate::parental::program_allowed(&crate::catalog::executable(root, &id)) {
                    return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
                }
                if self
                    .focus_catalogue_app(root, &AppTarget::Prepared(id.clone()))
                    .await?
                {
                    return Ok(());
                }
                if id == "enzui-settings" {
                    return self.launch_settings_page(root, "appearance").await;
                }
                if id == "enzui-assistant" {
                    return self.launch_app(root, &id).await;
                }
                let launcher = root.join("scripts/launch-desktop-app");
                command = scoped(
                    crate::catalog::desktop_id(&id).unwrap_or(&id),
                    launcher.as_os_str(),
                );
                command.arg(id);
            }
            Target::App(AppTarget::Desktop { path, identity }) => {
                identity.verify(&path)?;
                if !crate::parental::entry_allowed(&path) {
                    return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
                }
                if self
                    .focus_catalogue_app(
                        root,
                        &AppTarget::Desktop {
                            path: path.clone(),
                            identity: identity.clone(),
                        },
                    )
                    .await?
                {
                    return Ok(());
                }
                identity.verify(&path)?;
                if let Some(name) = super::catalogue::dbus_activatable(&path) {
                    return activate_application(&name).await;
                }
                let app = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default()
                    .to_owned();
                command = scoped(&app, std::ffi::OsStr::new("/usr/bin/gio"));
                command.args(["launch", "--"]).arg(path);
            }
            Target::Window(window) => return self.focus_search_result(window).await,
            Target::Settings(page) => return self.launch_settings_page(root, page).await,
            Target::SettingsSection(route) => {
                let id = crate::settings_catalog::SearchDestination::Section(route).id();
                return self.launch_settings_page(root, &id).await;
            }
            Target::File {
                path,
                identity,
                directory,
            } => {
                let file = rustix::fs::openat2(
                    rustix::fs::CWD,
                    &path,
                    rustix::fs::OFlags::PATH | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                    rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
                )
                .map(std::fs::File::from)
                .map_err(|_| tr("shell-file-changed-or-is-no-longer-available"))?;
                let metadata = file
                    .metadata()
                    .map_err(|_| tr("shell-file-is-no-longer-available"))?;
                let actual = FileIdentity {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    size: metadata.size(),
                    modified: (metadata.mtime(), metadata.mtime_nsec()),
                };
                if actual != identity
                    || directory != metadata.is_dir()
                    || (!directory && metadata.mode() & 0o111 != 0)
                {
                    return Err(tr(
                        "shell-file-changed-refresh-search-sources-and-choose-it-again",
                    )
                    .into());
                }
                command.args(["open", "--"]).arg(path);
            }
            Target::Web(url) => {
                if super::web_url(&url).is_none() {
                    return Err(tr("shell-only-http-s-addresses-can-be-opened").into());
                }
                command.args(["open", "--"]).arg(url);
            }
            _ => {
                return Err(tr("shell-this-action-belongs-to-the-desktop-frontend").into());
            }
        }
        spawn_desktop_application(command).await
    }

    pub async fn open_dock_folder(&self, root: &Path, path: &Path) -> Result<(), String> {
        drop(session::Target::from_environment()?.connect()?);
        if !path.is_absolute()
            || !tokio::fs::metadata(path)
                .await
                .is_ok_and(|meta| meta.is_dir())
        {
            return Err(tr("shell-dock-folder-unavailable").into());
        }
        let desktop_id = crate::catalog::desktop_id("files-app").expect("registered Files app");
        let executable = crate::catalog::executable(root, "files-app");
        let mut command = if executable.is_file() {
            if !crate::parental::program_allowed(&executable) {
                return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
            }
            let mut command = scoped(
                desktop_id,
                root.join("scripts/launch-desktop-app").as_os_str(),
            );
            command.arg("files-app");
            command
        } else {
            let entry = super::catalogue::installed_entry(desktop_id)
                .ok_or_else(|| tr("shell-application-is-no-longer-installed"))?;
            if !crate::parental::entry_allowed(&entry) {
                return Err(tr("shell-this-app-is-not-allowed-on-this-account").into());
            }
            let mut command = scoped(desktop_id, std::ffi::OsStr::new("/usr/bin/gio"));
            command.args(["launch", "--"]).arg(entry);
            command
        };
        command.arg(crate::desktop_portal::uri(path));
        spawn_desktop_application(command).await
    }
}

async fn spawn_desktop_application(mut command: tokio::process::Command) -> Result<(), String> {
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|_| tr("shell-application-could-not-be-started"))?;
    let mut exited = tokio::spawn(async move { child.wait().await });
    match tokio::time::timeout(std::time::Duration::from_millis(300), &mut exited).await {
        Ok(Ok(Ok(status))) if status.success() => Ok(()),
        Ok(_) => Err(tr(
            "shell-application-exited-before-opening-check-its-installation-or-default-app",
        )
        .into()),
        Err(_) => Ok(()),
    }
}

// `program` as the start of a launch of `app`, in the app's own scope on
// an installed login (see [`crate::app_scope`]).
fn scoped(app: &str, program: &std::ffi::OsStr) -> tokio::process::Command {
    let prefix = crate::app_scope::prefix(app);
    match prefix.split_first() {
        Some((run, rest)) => {
            let mut command = tokio::process::Command::new(run);
            command.args(rest).arg(program);
            command
        }
        None => tokio::process::Command::new(program),
    }
}

// Wait for D-Bus activation acknowledgement; gio launch may exit before a starting service
// accepts it.
pub(crate) async fn activate_application(name: &str) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-this-desktop-s-session-bus-is-unavailable"))?;
    activate_application_on(&connection, name, None).await
}

pub(crate) async fn activate_application_on(
    connection: &zbus::Connection,
    name: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
    let mut platform = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
    if let Some(token) = token {
        platform.insert("activation-token", zbus::zvariant::Value::from(token));
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connection.call_method(
            Some(name),
            path.as_str(),
            Some("org.freedesktop.Application"),
            "Activate",
            &(platform,),
        ),
    )
    .await
    .map_err(|_| tr("shell-application-did-not-respond-it-may-still-be-starting"))?
    .map(drop)
    .map_err(|_| tr("shell-application-could-not-be-started").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compositor_behaviour() {
        // Island search geometry reserves a clear gap without a connection.
        {
            use crate::settings::{BarEdge, Metrics};
            let metrics = Metrics {
                bar: 32,
                radius: 16,
                frame: 0,
                islands: Some(Default::default()),
                ..Default::default()
            };
            let anchor =
                iced::Rectangle::new(iced::Point::new(12.0, 0.0), iced::Size::new(116.0, 32.0));
            let output =
                json!({"width":400,"height":640,"scale":1,"name":"fixture","reserved":[0,50,0,0]});
            for edge in [BarEdge::Top, BarEdge::Bottom] {
                let geometry =
                    Compositor::search_layout(&output, metrics, edge, Some(anchor)).unwrap();
                assert_eq!(geometry.body_max, iced::Size::new(376.0, 580.0));
                let placement = geometry.island.unwrap();
                assert_eq!(placement.local_x, 12.0);
                assert_eq!(placement.bar_end, 50.0);
                assert_eq!(placement.bottom, edge == BarEdge::Bottom);
                assert_eq!(placement.attachment.join(16.0), 0.0);
            }
            let hidden = Compositor::search_layout(&output, metrics, BarEdge::Top, None).unwrap();
            assert_eq!(hidden.island.unwrap().attachment.join(16.0), 0.0);
            // On the frame it is joined to the bar, here across both frame sides.
            let frame =
                Compositor::search_layout(&output, Metrics::default(), BarEdge::Top, None).unwrap();
            let placement = frame.island.unwrap();
            assert!(placement.frame);
            assert_eq!(placement.attachment, crate::reveal::Attachment::BothFrames);
            assert_eq!(frame.body_max.width, 400.0);
            assert!(
                Compositor::search_layout(
                    &json!({"width":200,"height":240,"scale":1,"name":"fixture"}),
                    metrics,
                    BarEdge::Top,
                    None
                )
                .is_err()
            );
        }
        // Classic launcher focus exception is limited to its own shell surfaces.
        {
            for class in ["dev.enzui.Desktop", "dev.enzui.Launcher"] {
                assert!(retained_popup_focus(
                    &json!({"pid":42,"class":class}),
                    42,
                    "dev.enzui.Launcher"
                ));
                assert!(!retained_popup_focus(
                    &json!({"pid":43,"class":class}),
                    42,
                    "dev.enzui.Launcher"
                ));
            }
            for class in [
                "dev.enzui.QuickSearch",
                "dev.enzui.PluginPanel",
                "io.github.enzui.Terminal",
            ] {
                assert!(!retained_popup_focus(
                    &json!({"pid":42,"class":class}),
                    42,
                    "dev.enzui.Launcher"
                ));
            }
            assert!(!retained_popup_focus(&json!({}), 42, "dev.enzui.Launcher"));
        }
        // Bar focus exception requires the same shell process.
        {
            assert!(retained_search_focus(
                &json!({"pid":42,"class":"dev.enzui.Desktop"}),
                42
            ));
            assert!(retained_search_focus(
                &json!({"pid":42,"class":"dev.enzui.QuickSearch"}),
                42
            ));
            assert!(!retained_search_focus(
                &json!({"pid":43,"class":"dev.enzui.Desktop"}),
                42
            ));
            assert!(!retained_search_focus(
                &json!({"pid":42,"class":"dev.enzui.PluginPanel"}),
                42
            ));
            assert!(!retained_search_focus(&json!({}), 42));
        }
    }
}
