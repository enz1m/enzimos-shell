use enzui_shell::i18n::tr;
use std::path::PathBuf;

use super::{
    policy::control_geometry,
    types::{Compositor, ControlGeometry},
};

impl Compositor {
    /// Shell commands raised by key bindings (`crate::commands`).
    pub fn commands(&self) -> iced::Subscription<crate::commands::Command> {
        let socket = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("hypr").join(&self.instance).join(".socket2.sock"));
        crate::commands::subscription(socket)
    }

    pub fn focus_changes(&self) -> iced::Subscription<()> {
        let socket = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("hypr").join(&self.instance).join(".socket2.sock"));
        crate::workspaces::focus_changes(socket)
    }

    /// The active window's owner and tag, and every mapped window's, for
    /// deciding whose work status to hide.
    pub async fn activity_windows(&self) -> Result<crate::activity::Windows, String> {
        let owner = |window: &serde_json::Value| {
            let pid = window["pid"]
                .as_i64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid > 0)?;
            let tag = window["xdgTag"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned);
            Some((pid, tag))
        };
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?)
                .map_err(|_| tr("shell-invalid-window-list"))?;
        if !active.is_object() {
            return Err(tr("shell-invalid-window-list").into());
        }
        let clients: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "clients"]).await?)
                .map_err(|_| tr("shell-invalid-window-list"))?;
        let tagged = clients
            .as_array()
            .ok_or_else(|| tr("shell-invalid-window-list"))?
            .iter()
            .filter(|window| window["mapped"] != false)
            .filter_map(owner)
            .filter_map(|(pid, tag)| Some((pid, tag?)))
            .take(512)
            .collect();
        Ok(crate::activity::Windows {
            active: owner(&active),
            tagged,
        })
    }

    pub fn desktop_changes(&self) -> iced::Subscription<()> {
        crate::workspaces::changes(self.event_socket())
    }

    /// The compositor's event socket (`.socket2.sock`).
    pub fn event_socket(&self) -> Option<PathBuf> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|path| path.join("hypr").join(&self.instance).join(".socket2.sock"))
    }

    /// Serves `dev.enzui.Shell.Windows1` while held; see [`crate::window_feed`].
    pub fn windows_feed(&self) -> iced::Subscription<()> {
        iced::Subscription::run_with(
            (self.executable.clone(), self.instance.clone()),
            |(executable, instance): &(PathBuf, String)| {
                crate::window_feed::stream(Self {
                    executable: executable.clone(),
                    instance: instance.clone(),
                })
            },
        )
    }

    /// How windows' corners are rounded on screen: the radius in logical
    /// pixels and `rounding_power`; none when it cannot be read.
    pub async fn window_rounding(&self) -> (f32, f32) {
        let option = |text: Result<String, String>, key: &str| -> Option<f64> {
            serde_json::from_str::<serde_json::Value>(&text.ok()?)
                .ok()?
                .get(key)?
                .as_f64()
        };
        let radius = option(
            self.request(&["-j", "getoption", "decoration:rounding"])
                .await,
            "int",
        )
        .unwrap_or(0.0);
        let power = option(
            self.request(&["-j", "getoption", "decoration:rounding_power"])
                .await,
            "float",
        )
        .unwrap_or(2.0);
        let finite = |value: f64, low: f64, high: f64| {
            if value.is_finite() {
                value.clamp(low, high) as f32
            } else {
                low as f32
            }
        };
        (finite(radius, 0.0, 200.0), finite(power, 1.0, 10.0))
    }

    pub async fn windows(&self) -> Result<Vec<serde_json::Value>, String> {
        let text = self.request(&["-j", "clients"]).await?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(tr("shell-desktop-window-list-exceeds-its-bound").into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| tr("shell-invalid-desktop-window-list"))?;
        value
            .as_array()
            .filter(|rows| rows.len() <= 512)
            .cloned()
            .ok_or_else(|| tr("shell-invalid-desktop-window-count").into())
    }

    pub async fn desktop_snapshot(&self) -> Result<crate::workspaces::Desktops, String> {
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-desktop-output"))?;
        let query = format!(
            "repl return enzui_workspace_snapshot({})",
            serde_json::to_string(name).expect("serializing a string cannot fail")
        );
        crate::workspaces::Desktops::parse(&self.request(&["--batch", &query]).await?)
    }

    pub async fn desktop_step(
        &self,
        direction: i32,
    ) -> Result<crate::workspaces::Desktops, String> {
        if direction != -1 && direction != 1 {
            return Err(tr("shell-invalid-desktop-direction").into());
        }
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-desktop-output"))?;
        let query = format!(
            "return enzui_workspace_step({}, {direction})",
            serde_json::to_string(name).expect("serializing a string cannot fail")
        );
        if self.request(&["eval", &query]).await?.trim() != "ok" {
            return Err(tr("shell-desktop-navigation-was-rejected").into());
        }
        self.desktop_snapshot().await
    }

    pub async fn desktop_focus(&self, id: u32) -> Result<crate::workspaces::Desktops, String> {
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-desktop-output"))?;
        let query = format!(
            "return enzui_workspace_focus({}, {id})",
            serde_json::to_string(name).expect("serializing a string cannot fail")
        );
        if self.request(&["eval", &query]).await?.trim() != "ok" {
            return Err(tr("shell-desktop-selection-was-rejected").into());
        }
        self.desktop_snapshot().await
    }

    pub async fn prepare_desktop_frame(&self) -> Result<String, String> {
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .filter(|name| crate::desktop_frame::valid_output(name))
            .ok_or_else(|| tr("shell-invalid-desktop-frame-output"))?;
        if let Ok(mut primary) = super::core::PRIMARY.lock() {
            *primary = Some(super::core::OutputKey::of(&output));
        }
        let policy = format!(
            "{}\n{}\n{}\n{}\n{}\nenzui_install_workspace_keys()\nenzui_install_output_pinning()",
            include_str!("../../../config/desktop-policy.lua"),
            include_str!("../../../config/display-frame.lua"),
            include_str!("../../../config/panel-policy.lua"),
            include_str!("../../../config/workspace-policy.lua"),
            include_str!("../../../config/output-rules.lua")
        );
        let response = self.request(&["eval", &policy]).await?;
        if response.trim() != "ok" {
            return Err(tr("shell-desktop-shell-policy-was-rejected").into());
        }
        Ok(name.into())
    }

    pub async fn islands_supported(&self) -> bool {
        self.request(&["--batch", "repl return (function() local a=hl.plugin and hl.plugin.enzui_sidebar_scene return (a and a.version()==8 and a.supports_islands and a.supports_islands()) or false end)()"]).await.is_ok_and(|value| value.trim() == "true")
    }

    pub async fn set_bar_regions(
        &self,
        epoch: u64,
        serial: u64,
        geometry: &crate::bar_geometry::BarGeometry,
    ) -> Result<(), String> {
        if !geometry.valid()
            || epoch == 0
            || serial == 0
            || epoch > (1 << 53) - 1
            || serial > (1 << 53) - 1
        {
            return Err(tr("shell-invalid-island-input-geometry").into());
        }
        let rows = geometry
            .islands
            .iter()
            .map(|island| {
                let a = island.bounds;
                let c = island.clip;
                format!(
                    "{{{},{},{},{},{},{},{},{}}}",
                    a.x, a.y, a.width, a.height, c.x, c.y, c.width, c.height
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let command = format!(
            "local a=hl.plugin and hl.plugin.enzui_sidebar_scene;assert(a and a.version()==8 and a.bar_regions({}, {epoch}, {serial}, {{{rows}}}), 'Island input geometry rejected');return {{ok=true}}",
            std::process::id()
        );
        if self.request(&["eval", &command]).await?.trim() != "ok" {
            return Err(tr("shell-island-input-geometry-was-rejected").into());
        }
        Ok(())
    }

    pub async fn apply_bar_position(
        &self,
        edge: crate::settings::BarEdge,
        metrics: crate::settings::Metrics,
        epoch: u64,
    ) -> Result<ControlGeometry, String> {
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-desktop-output"))?;
        let edge = match edge {
            crate::settings::BarEdge::Top => "top",
            crate::settings::BarEdge::Bottom => "bottom",
        };
        if epoch == 0 || epoch > (1 << 53) - 1 {
            return Err(tr("shell-invalid-bar-generation").into());
        }
        let islands = metrics.islands.is_some();
        let width = output["width"]
            .as_f64()
            .expect("control_geometry validated finite output dimensions and scale")
            / output["scale"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale");
        let owner = std::process::id();
        let quoted_output = serde_json::to_string(name).expect("serializing a string cannot fail");
        let native = format!(
            "local a=hl.plugin and hl.plugin.enzui_sidebar_scene; if {islands} then assert(a and a.version()==8 and a.supports_islands(), 'Islands needs a current prepared compositor') end; if a and a.version()==8 then assert(a.configure({owner}, {quoted_output}) and a.bar_mode({owner}, {epoch}, {islands}, {width}, {}), 'Bar input mode rejected') end",
            metrics.bar
        );
        // The native plugin accepts the bar only on its output; failed moves retry on the next
        // poll.
        let lua = format!(
            "{}\n{}\n{}\n{}\nif enzui_panels_onto({quoted_output}, {owner}) then return 'moving' end\n{native}\nreturn enzui_apply_shell_layout({}, {}, {})",
            include_str!("../../../config/shell-metrics.lua"),
            include_str!("../../../config/sidebar-visibility.lua"),
            include_str!("../../../config/shell-reservations.lua"),
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/shell-layout.lua")
            ),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::to_string(edge).expect("serializing a string cannot fail"),
            metrics.lua()
        );
        match self.request(&["eval", &lua]).await?.trim() {
            "ok" => {}
            "moving" => {
                return Err(tr("shell-the-bar-is-moving-onto-the-primary-display").into());
            }
            _ => return Err(tr("shell-bar-placement-was-rejected").into()),
        }
        Ok(ControlGeometry {
            width: (output["width"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
                / output["scale"]
                    .as_f64()
                    .expect("control_geometry validated finite output dimensions and scale"))
                as f32,
            height: f32::from(metrics.bar),
            output: name.into(),
            attachment: Default::default(),
        })
    }
}
