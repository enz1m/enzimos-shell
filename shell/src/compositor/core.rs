use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

use super::{
    policy::{parse_keyboard, parse_keyboard_layout, parse_workspace},
    types::{Compositor, Snapshot},
};

impl Compositor {
    pub fn dock_changes(&self) -> iced::Subscription<()> {
        let socket = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("hypr").join(&self.instance).join(".socket2.sock"));
        iced::Subscription::run_with(socket, |socket| {
            crate::workspaces::dock_events(socket.clone())
        })
    }

    /// Requires an explicit nested instance and control binary; never targets the host by default.
    pub fn from_environment() -> Result<Self, String> {
        Self::new(
            std::env::var_os("ENZUI_HYPR_CONTROL_BIN")
                .map(PathBuf::from)
                .ok_or_else(|| tr("shell-launch-the-shell-with-enzui-hypr-shell"))?,
            std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
                .map_err(|_| tr("shell-missing-nested-compositor-instance"))?,
        )
    }

    pub fn new(executable: PathBuf, instance: String) -> Result<Self, String> {
        if !executable.is_absolute()
            || !executable.is_file()
            || instance.is_empty()
            || instance.len() > 180
            || !instance
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(tr("shell-invalid-nested-compositor-control-endpoint").into());
        }
        Ok(Self {
            executable,
            instance,
        })
    }

    /// Pauses the desktop's shortcuts while Settings records a new one, so
    /// the keys reach Settings; they come back by themselves after a while.
    pub async fn shortcut_recording(&self, on: bool) -> Result<(), String> {
        let lua = format!(
            "if enzui_shortcut_recording then return enzui_shortcut_recording({on}) end return \"ok\""
        );
        match self.request(&["eval", &lua]).await?.trim() {
            "ok" => Ok(()),
            _ => Err(tr("shell-shortcuts-could-not-be-paused").into()),
        }
    }
    /// Captures the current appearance before the caller writes a new theme.
    ///
    /// Call with `ready` after writing to start the dissolve. Capture can time out.
    pub async fn appearance_transition(&self, ready: bool) {
        const CAPTURE_BOUND: Duration = Duration::from_millis(250);
        let phase = if ready { "ready" } else { "begin" };
        let transition = |phase: &str| {
            format!(
                "repl local p = hl.plugin.enzui_appearance if p then return tostring(p.transition('{phase}')) end return 'false'"
            )
        };
        let mut reply = self.request(&["--batch", &transition(phase)]).await;
        if ready {
            return;
        }
        let captured = transition("captured");
        let deadline = tokio::time::Instant::now() + CAPTURE_BOUND;
        while matches!(&reply, Ok(reply) if reply.trim() == "pending")
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(4)).await;
            reply = self.request(&["--batch", &captured]).await;
        }
    }

    pub(crate) async fn desktop_fullscreen(&self) -> bool {
        let query = "repl return enzui_desktop_fullscreen and enzui_desktop_fullscreen()";
        self.request(&["--batch", query])
            .await
            .is_ok_and(|response| response.trim() == "true")
    }

    pub(crate) async fn request(&self, arguments: &[&str]) -> Result<String, String> {
        // A live compositor answers on its socket directly: no hyprctl
        // process per request (the shell's idle polls made several a second).
        if let Some(socket) = self.request_socket() {
            return tokio::time::timeout(
                Duration::from_millis(1500),
                request_socket(&socket, arguments),
            )
            .await
            .map_err(|_| tr("shell-compositor-did-not-respond").to_owned())?;
        }
        let mut command = Command::new(&self.executable);
        command
            .args(["-i", self.instance.as_str()])
            .args(arguments)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_millis(1500), command.output())
            .await
            .map_err(|_| tr("shell-compositor-did-not-respond").to_owned())?
            .map_err(|error| tr!("shell-compositor-control-4", "arg0" => format!("{:}", error)))?;
        if !output.status.success() {
            // hyprctl prints the compositor's reason (a failed Lua assertion,
            // an unknown dispatcher) on stdout; keep its first line.
            let reason: String = String::from_utf8_lossy(&output.stdout)
                .lines()
                .chain(String::from_utf8_lossy(&output.stderr).lines())
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            return Err(if reason.is_empty() {
                tr("shell-compositor-rejected-the-request").into()
            } else {
                tr!("shell-compositor-rejected-the-request-2", "arg0" => reason)
            });
        }
        String::from_utf8(output.stdout).map_err(|_| tr("shell-invalid-compositor-response").into())
    }

    pub async fn writing_request(&self, command: &str) -> Result<String, String> {
        let socket = self
            .request_socket()
            .ok_or_else(|| tr("writing-selection-unavailable"))?;
        tokio::time::timeout(
            Duration::from_millis(600),
            request_socket(&socket, &["enzuiwriting", command]),
        )
        .await
        .map_err(|_| tr("writing-selection-unavailable").to_owned())?
    }

    fn request_socket(&self) -> Option<std::path::PathBuf> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let socket = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?)
            .join("hypr")
            .join(&self.instance)
            .join(".socket.sock");
        let metadata = std::fs::symlink_metadata(&socket).ok()?;
        (socket.is_absolute()
            && metadata.file_type().is_socket()
            && metadata.uid() == rustix::process::getuid().as_raw())
        .then_some(socket)
    }

    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        let response = self.request(&["-j", "activeworkspace"]).await?;
        let workspace = parse_workspace(&response)?;
        let mode = self
            .request(&["--batch", &desktop_mode_query(workspace)])
            .await?;
        let tiling = match mode.trim() {
            "true" => true,
            "false" => false,
            _ => {
                return Err(tr("shell-desktop-window-policy-is-unavailable").into());
            }
        };
        let devices = self.request(&["-j", "devices"]).await.ok();
        let keyboard_layout = devices.as_deref().and_then(parse_keyboard_layout);
        let keyboard = devices.as_deref().and_then(parse_keyboard);
        Ok(Snapshot {
            workspace,
            tiling,
            keyboard_layout,
            keyboard,
        })
    }

    /// Starts or stops the Orca screen reader, as Super+Alt+S does.
    pub async fn toggle_screen_reader(&self) -> Result<(), String> {
        let reply = self.request(&["eval", r#"return hl.dispatch(hl.dsp.exec_cmd('pkill -u "$(id -u)" -x orca || exec orca --replace'))"#]).await?;
        if reply.trim() == "ok" || reply.trim().is_empty() {
            Ok(())
        } else {
            Err(tr("shell-the-screen-reader-could-not-be-started").into())
        }
    }

    /// Makes layout `index` of every keyboard active, or the next one.
    pub async fn switch_layout(&self, index: Option<usize>) -> Result<(), String> {
        let target = index.map_or_else(|| "next".to_owned(), |index| index.to_string());
        let reply = self.request(&["switchxkblayout", "all", &target]).await?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-the-keyboard-layout-could-not-be-switched").into())
        }
    }

    pub async fn switch_workspace(&self, workspace: u32) -> Result<Snapshot, String> {
        if !(1..=9).contains(&workspace) {
            return Err(tr("shell-workspace-must-be-1-9").into());
        }
        let response = self
            .request(&[
                "eval",
                &format!("return hl.dispatch(hl.dsp.focus({{workspace={workspace}}}))"),
            ])
            .await?;
        if response.trim() != "ok" {
            return Err(tr("shell-workspace-change-was-rejected").into());
        }
        self.snapshot().await
    }

    /// Sets the default window mode and the desktops that keep their own,
    /// converting the windows of every desktop whose mode changes.
    pub async fn set_desktop_modes(
        &self,
        default: bool,
        modes: &std::collections::BTreeMap<u32, bool>,
    ) -> Result<Snapshot, String> {
        let response = self
            .request(&["eval", &desktop_modes_call(default, modes)])
            .await?;
        if response.trim() != "ok" {
            return Err(tr("shell-window-layout-change-was-rejected").into());
        }
        self.snapshot().await
    }

    pub async fn launch_app(&self, root: &std::path::Path, id: &str) -> Result<(), String> {
        if !crate::catalog::APPS.iter().any(|app| app.id == id) {
            return Err(tr("shell-unknown-application").into());
        }
        if !root.is_absolute() || !crate::catalog::executable(root, id).is_file() {
            return Err(tr("shell-application-is-not-installed").into());
        }
        if id == "enzui-settings" {
            return self.launch_settings_page(root, "appearance").await;
        }
        let quote =
            |path: &std::path::Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        // The Assistant is the user's own client of their agent harness, not
        // a sandboxed app: it needs the private harness socket and config.
        let command = if id == "enzui-assistant" {
            let store = crate::settings::Store::from_environment()
                .ok_or_else(|| tr("shell-settings-store-is-unavailable"))?;
            if crate::desktop_session::mode() != crate::desktop_session::Mode::Installed {
                format!(
                    "env ENZUI_SHELL_SETTINGS={} {}",
                    quote(store.path()),
                    quote(&crate::catalog::executable(root, id))
                )
            } else {
                let scope = crate::app_scope::shell_prefix("dev.enzui.Assistant");
                format!(
                    "{scope}env ENZUI_SHELL_SETTINGS={} {} {id}",
                    quote(store.path()),
                    quote(&root.join("scripts/launch-desktop-app")),
                )
            }
        } else {
            let scope =
                crate::app_scope::shell_prefix(crate::catalog::desktop_id(id).unwrap_or(id));
            format!(
                "{scope}{} {id}",
                quote(&root.join("scripts/launch-desktop-app"))
            )
        };
        let lua = format!(
            "return hl.dispatch(hl.dsp.exec_cmd({}))",
            serde_json::to_string(&command).map_err(|error| error.to_string())?
        );
        let response = self.request(&["eval", &lua]).await?;
        if response.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-application-launch-was-rejected").into())
        }
    }

    /// Mirrors the desktop output; mirrored outputs are discovered through `monitors all`.
    pub async fn set_mirroring(&self, on: bool) -> Result<(), String> {
        let primary = self.desktop_output().await?;
        let (Some(name), Some(id)) = (primary["name"].as_str(), primary["id"].as_i64()) else {
            return Err(tr("shell-no-desktop-output").into());
        };
        let monitors: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors", "all"]).await?)
                .map_err(|_| tr("shell-invalid-output-list"))?;
        let plain = |text: &str| {
            !text.is_empty()
                && text.len() <= 64
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if !plain(name) {
            return Err(tr("shell-unexpected-output-name").into());
        }
        let mut lua = String::new();
        for monitor in monitors.as_array().into_iter().flatten() {
            let Some(other) = monitor["name"]
                .as_str()
                .filter(|other| *other != name && plain(other) && !other.starts_with("ENZREMOTE-"))
            else {
                continue;
            };
            let mirroring = monitor["mirrorOf"].as_str().unwrap_or("none");
            if on && mirroring != id.to_string() {
                let _ = writeln!(
                    lua,
                    "hl.monitor({{output = \"{other}\", mirror = \"{name}\"}})"
                );
            } else if !on && mirroring != "none" {
                let _ = writeln!(lua, "hl.monitor({{output = \"{other}\", mirror = \"\"}})");
            }
        }
        if lua.is_empty() {
            return Ok(());
        }
        lua.push_str("return \"ok\"");
        if self.request(&["eval", &lua]).await?.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-mirroring-was-rejected").into())
        }
    }

    /// The focused window's class, if a window has the focus.
    pub async fn active_class(&self) -> Option<String> {
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await.ok()?).ok()?;
        active["class"]
            .as_str()
            .filter(|class| !class.is_empty())
            .map(|class| class.chars().take(128).collect())
    }

    /// Remembers the apps that have windows now, for reopening after login.
    pub async fn remember_session_apps(&self) -> Result<(), String> {
        let clients: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "clients"]).await?)
                .map_err(|_| tr("shell-invalid-window-list"))?;
        let monitors: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors"]).await?)
                .map_err(|_| tr("shell-invalid-display-list"))?;
        let path = crate::session_restore::path().ok_or_else(|| tr("shell-no-state-folder"))?;
        tokio::task::spawn_blocking(move || {
            let chosen = crate::session_restore::choose(
                &clients,
                &monitors,
                &crate::default_apps::by_window_class(),
            );
            crate::session_restore::save(&path, &chosen)
                .map_err(|_| tr("shell-open-apps-could-not-be-remembered").to_owned())
        })
        .await
        .map_err(|_| tr("shell-open-apps-could-not-be-remembered").to_owned())?
    }

    /// Restores remembered apps once per login, returning the number started.
    pub async fn restore_session_apps(&self) -> Result<usize, String> {
        let path = crate::session_restore::path().ok_or_else(|| tr("shell-no-state-folder"))?;
        if !crate::session_restore::first_restore_of_login() {
            return Ok(0);
        }
        let monitors: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors"]).await?)
                .map_err(|_| tr("shell-invalid-display-list"))?;
        let connected: Vec<String> = monitors
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|monitor| monitor.get("name")?.as_str().map(str::to_owned))
            .collect();
        let launches = tokio::task::spawn_blocking(move || {
            // The window class each entry's windows carry, and its file.
            let entries: std::collections::HashMap<String, (String, std::path::PathBuf)> =
                crate::default_apps::by_window_class()
                    .into_values()
                    .map(|(id, path, class)| (id, (class, path)))
                    .collect();
            crate::session_restore::load(&path)
                .into_iter()
                .enumerate()
                .filter_map(|(index, saved)| {
                    let (class, entry) = entries.get(&saved.id)?;
                    let activatable = crate::search::catalogue::dbus_activatable(entry);
                    Some((
                        crate::session_restore::launch_lua(
                            entry,
                            class,
                            &saved,
                            &connected,
                            index,
                            activatable.is_none(),
                        )?,
                        activatable,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| tr("shell-remembered-apps-could-not-be-read").to_owned())?;
        let mut started = 0;
        for (lua, activatable) in launches {
            if self.request(&["eval", &lua]).await?.trim() != "ok" {
                continue;
            }
            // `gio launch` would not wait for a service that is still starting.
            match activatable {
                Some(name) => {
                    started += usize::from(
                        crate::search::compositor::activate_application(&name)
                            .await
                            .is_ok(),
                    );
                }
                None => started += 1,
            }
        }
        if started > 0 {
            tokio::time::sleep(crate::session_restore::PLACEMENT).await;
        }
        self.request(&["eval", crate::session_restore::END_LUA])
            .await?;
        Ok(started)
    }

    /// Opens a mounted removable drive in the default file manager.
    pub async fn open_folder(&self, folder: &std::path::Path) -> Result<(), String> {
        if !crate::drives::openable(folder) {
            return Err(tr("shell-only-removable-drives-are-opened").into());
        }
        let command = format!(
            "xdg-open '{}'",
            folder.to_string_lossy().replace('\'', "'\\''")
        );
        let lua = format!(
            "return hl.dispatch(hl.dsp.exec_cmd({}))",
            serde_json::to_string(&command).map_err(|error| error.to_string())?
        );
        if self.request(&["eval", &lua]).await?.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-opening-the-drive-was-rejected").into())
        }
    }

    pub async fn launch_notification_settings(&self, root: &std::path::Path) -> Result<(), String> {
        self.launch_settings_page(root, "notifications").await
    }
    pub async fn launch_settings_page(
        &self,
        root: &std::path::Path,
        page: &str,
    ) -> Result<(), String> {
        self.launch_settings_target(root, page, None).await
    }
    pub async fn launch_settings_plugin(
        &self,
        root: &std::path::Path,
        id: &str,
    ) -> Result<(), String> {
        crate::bar_layout::Item::new(format!("plugin:{id}/entry"), true)?;
        self.launch_settings_target(root, "plugins/installed", Some(id))
            .await
    }
    async fn launch_settings_target(
        &self,
        root: &std::path::Path,
        page: &str,
        plugin: Option<&str>,
    ) -> Result<(), String> {
        if !crate::settings_catalog::valid_route(page) {
            return Err(tr("shell-unknown-settings-page").into());
        }
        let executable = crate::catalog::executable(root, "enzui-settings");
        if !root.is_absolute() || !executable.is_file() {
            return Err(tr("shell-settings-executable-is-unavailable").into());
        }
        let store = crate::settings::Store::from_environment()
            .ok_or_else(|| tr("shell-settings-store-is-unavailable"))?;
        let quote =
            |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        let command = format!(
            "env ENZUI_SETTINGS_PLUGIN={} ENZUI_SETTINGS_PAGE={} ENZUI_HYPR_CONTROL_BIN={} ENZUI_DESKTOP_ROOT={} ENZUI_SHELL_SETTINGS={} {}",
            quote(std::path::Path::new(plugin.unwrap_or(""))),
            page,
            quote(&self.executable),
            quote(root),
            quote(store.path()),
            quote(&executable)
        );
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-settings-output"))?;
        let lua = format!(
            "{}\nreturn hl.dispatch(hl.dsp.exec_cmd(enzui_settings_command({}, {})))",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/settings-launch.lua")
            ),
            serde_json::to_string(&command)
                .map_err(|_| tr("shell-cannot-encode-settings-command"))?,
            serde_json::to_string(name).map_err(|_| tr("shell-cannot-encode-settings-output"))?
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-settings-launch-was-rejected").into());
        }
        Ok(())
    }
    pub async fn install_drag_maximize(&self, accent: [u8; 3], radius: u16) -> Result<(), String> {
        let executable = std::env::current_exe()
            .map_err(|_| tr("shell-cannot-locate-the-maximize-preview-helper"))?
            .with_file_name("enzui-maximize-preview");
        if !executable.is_file() {
            return Err(tr("shell-maximize-preview-helper-is-unavailable").into());
        }
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .filter(|name| crate::desktop_frame::valid_output(name))
            .ok_or_else(|| tr("shell-missing-maximize-preview-output"))?;
        let lua = self.drag_maximize_payload(name, &executable, accent, radius);
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-drag-to-maximize-was-rejected").into());
        }
        Ok(())
    }
    pub(super) fn drag_maximize_payload(
        &self,
        output: &str,
        executable: &std::path::Path,
        accent: [u8; 3],
        radius: u16,
    ) -> String {
        // io.popen runs this through a shell, so both arguments are quoted.
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let command = format!(
            "exec {} {}",
            quote(&executable.to_string_lossy()),
            quote(output)
        );
        let encode = |value: &str| {
            // Lua long-bracket quoting is avoided: these are plain shell and hex
            // strings, so escaping quotes and backslashes is sufficient.
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        };
        // The leading newline matters: the file opens with a `--` comment, and
        // hyprctl would otherwise read the payload as a command-line flag.
        format!(
            "\n{}\nassert(enzui_drag_maximize_install({}), \"install failed\") assert(enzui_drag_maximize_theme({}, {}), \"theme failed\")",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/drag-maximize.lua")
            ),
            encode(&command),
            encode(&format!(
                "{:02x}{:02x}{:02x}",
                accent[0], accent[1], accent[2]
            )),
            radius.min(512)
        )
    }
    pub async fn apply_titlebar_theme(
        &self,
        theme: &crate::titlebar_theme::TitleBarTheme,
    ) -> Result<(), String> {
        if self.request(&["eval", &theme.lua()]).await?.trim() != "ok" {
            return Err(tr("shell-title-bar-theme-was-rejected").into());
        }
        Ok(())
    }
    pub async fn update_drag_maximize_theme(
        &self,
        accent: [u8; 3],
        radius: u16,
    ) -> Result<(), String> {
        let lua = format!(
            "assert(enzui_drag_maximize_theme, \"drag-maximize is not installed\") assert(enzui_drag_maximize_theme({}, {}), \"theme failed\")",
            serde_json::to_string(&format!(
                "{:02x}{:02x}{:02x}",
                accent[0], accent[1], accent[2]
            ))
            .map_err(|_| tr("shell-cannot-encode-the-preview-accent"))?,
            radius.min(512)
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-drag-to-maximize-accent-was-rejected").into());
        }
        Ok(())
    }
    pub async fn focus_notification_center(&self) -> Result<(), String> {
        self.focus_owned_popup(
            "dev.enzui.NotificationCenter",
            tr("shell-notification-center"),
        )
        .await
    }
    pub async fn focus_launcher(&self) -> Result<(), String> {
        self.focus_owned_popup("dev.enzui.Launcher", "launcher")
            .await
    }
    pub async fn focus_dock(&self) -> Result<(), String> {
        self.focus_owned_popup("dev.enzui.Dock", "dock").await
    }
    pub async fn focus_control_center(&self) -> Result<(), String> {
        self.cache_control_center("true").await
    }
    pub async fn park_control_center(&self) -> Result<(), String> {
        self.cache_control_center("false").await
    }
    pub async fn prepare_control_center_frame(&self) -> Result<(), String> {
        self.cache_control_center(r#""prepare""#).await
    }
    /// The first preparation of a Control Center that was just opened. Its
    /// surface can be ready on the shell's side a moment before the
    /// compositor has mapped the window, and the policy refuses a window it
    /// cannot find yet: asked once, that start-up race was logged as a
    /// rejected request.
    pub async fn prepare_new_control_center_frame(&self) -> Result<(), String> {
        let mut result = self.prepare_control_center_frame().await;
        for _ in 0..20 {
            if result.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            result = self.prepare_control_center_frame().await;
        }
        result
    }
    async fn cache_control_center(&self, visible: &str) -> Result<(), String> {
        let lua = format!(
            "assert(enzui_control_center_cache({}, {visible}));return true",
            std::process::id()
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-control-center-placement-was-rejected").into());
        }
        Ok(())
    }
    pub async fn focus_overview(&self) -> Result<(), String> {
        self.focus_owned_popup("dev.enzui.WorkspaceOverview", "overview")
            .await
    }
    pub async fn focus_plugin_panel(&self) -> Result<(), String> {
        self.focus_owned_popup("dev.enzui.PluginPanel", tr("shell-plugin-panel"))
            .await
    }
    pub async fn focus_desktop_bar(&self) -> Result<(), String> {
        self.focus_owned_popup("dev.enzui.Desktop", "bar").await
    }
    pub async fn restore_bar_from_dock(&self) -> Result<(), String> {
        self.focus_owned_popup_if("dev.enzui.Desktop", "bar", Some("dev.enzui.Dock"))
            .await
    }
    async fn focus_owned_popup(&self, class: &str, name: &str) -> Result<(), String> {
        self.focus_owned_popup_if(class, name, None).await
    }
    async fn focus_owned_popup_if(
        &self,
        class: &str,
        name: &str,
        from: Option<&str>,
    ) -> Result<(), String> {
        let guard = from.map(|from| format!("local current=hl.get_active_window();if not current or current.pid~={} or current.class~={} then return true end;",std::process::id(),serde_json::to_string(from).expect("serializing a string cannot fail"))).unwrap_or_default();
        let class =
            serde_json::to_string(class).map_err(|_| tr("shell-cannot-encode-popup-class"))?;
        let ambiguous = serde_json::to_string(&tr!("shell-ambiguous", "arg0" => name.to_owned()))
            .map_err(|_| tr("shell-cannot-encode-popup-error"))?;
        let unavailable =
            serde_json::to_string(&tr!("shell-is-unavailable", "arg0" => name.to_owned()))
                .map_err(|_| tr("shell-cannot-encode-popup-error"))?;
        let lua = format!(
            "{guard}local target=nil;for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class=={class} then if target then error({ambiguous}) end;target=w end end;if not target then error({unavailable}) end;local previous=hl.get_config(\"cursor.no_warps\");assert(type(previous)==\"boolean\",\"Missing cursor warp policy\");if not previous then hl.config({{cursor={{no_warps=true}}}}) end;local ok,result=pcall(function() hl.dispatch(hl.dsp.focus({{window=target}}));return hl.dispatch(hl.dsp.window.alter_zorder({{mode=\"top\",window=target}})) end);if not previous then hl.config({{cursor={{no_warps=false}}}}) end;if not ok then error(result) end;return result",
            std::process::id()
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr!("shell-focus-was-rejected", "arg0" => name.to_owned()));
        }
        Ok(())
    }
    pub async fn install_notification_shortcut(
        &self,
        receiver: crate::shortcuts::Receiver,
    ) -> Result<(), String> {
        let executable = std::env::current_exe()
            .map_err(|_| tr("shell-cannot-locate-shortcut-helper"))?
            .with_file_name("enzui-shell-command");
        if !executable.is_file() {
            return Err(tr("shell-notification-shortcut-helper-is-unavailable").into());
        }
        let command = format!(
            "'{}' notifications {}",
            executable.to_string_lossy().replace('\'', "'\\''"),
            receiver.arguments()
        );
        let lua = format!(
            "hl.unbind(\"SUPER + N\"); enzui_notification_binding=hl.bind(\"SUPER + N\",hl.dsp.exec_cmd({}));if not enzui_notification_binding then error(\"Notification binding was rejected\") end",
            serde_json::to_string(&command)
                .map_err(|_| tr("shell-cannot-encode-notification-shortcut"))?
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-notification-shortcut-was-rejected").into());
        }
        Ok(())
    }
    pub async fn install_settings_shortcut(&self, root: &std::path::Path) -> Result<(), String> {
        let executable = crate::catalog::executable(root, "enzui-settings");
        if !root.is_absolute() || !executable.is_file() {
            return Err(tr("shell-settings-recovery-executable-is-unavailable").into());
        }
        let store = crate::settings::Store::from_environment()
            .ok_or_else(|| tr("shell-settings-recovery-store-is-unavailable"))?;
        let quote =
            |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        let command = format!(
            "env ENZUI_SETTINGS_PAGE=appearance ENZUI_HYPR_CONTROL_BIN={} ENZUI_DESKTOP_ROOT={} ENZUI_SHELL_SETTINGS={} {}",
            quote(&self.executable),
            quote(root),
            quote(store.path()),
            quote(&executable)
        );
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-settings-output"))?;
        let lua = format!(
            "{}\nif enzui_settings_binding then enzui_settings_binding:remove() end; enzui_settings_binding=hl.bind(\"SUPER + code:59\",function() for _,w in ipairs(hl.get_windows()) do if w.pid=={} and (w.class==\"dev.enzui.PluginPanel\" or w.class==\"dev.enzui.ControlCenter\" or w.class==\"dev.enzui.WorkspaceOverview\" or w.class==\"dev.enzui.Launcher\" or w.class==\"dev.enzui.QuickSearch\" or w.class==\"dev.enzui.NotificationCenter\") then hl.dispatch(hl.dsp.window.close({{window=w}})) end end; return hl.dispatch(hl.dsp.exec_cmd(enzui_settings_command({}, {}))) end)",
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/settings-launch.lua")
            ),
            std::process::id(),
            serde_json::to_string(&command)
                .map_err(|_| tr("shell-cannot-encode-settings-shortcut"))?,
            serde_json::to_string(name).map_err(|_| tr("shell-cannot-encode-settings-output"))?
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-settings-recovery-shortcut-was-rejected").into());
        }
        Ok(())
    }

    pub async fn desktop_output(&self) -> Result<serde_json::Value, String> {
        let value: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors"]).await?)
                .map_err(|_| tr("shell-invalid-desktop-output-list"))?;
        let outputs = value
            .as_array()
            .ok_or_else(|| tr("shell-missing-desktop-outputs"))?;
        let eligible = |v: &&serde_json::Value| {
            v["name"]
                .as_str()
                .is_some_and(|name| !name.starts_with("ENZREMOTE-"))
        };
        // The top-left physical output of the layout hosts the shell; the
        // installed launcher (scripts/start-shell-platform) uses the same rule.
        let position = |v: &serde_json::Value, key: &str| v[key].as_i64().unwrap_or(0);
        let mut output = outputs
            .iter()
            .filter(eligible)
            .min_by_key(|v| (position(v, "y"), position(v, "x"), position(v, "id")))
            .cloned()
            .ok_or_else(|| tr("shell-no-desktop-output-is-available"))?;
        // Hyprland reports the mode's size; callers lay out in the output's
        // visible orientation, which a 90 or 270 degree rotation turns.
        if output["transform"].as_i64().is_some_and(|t| t % 2 == 1) {
            let (width, height) = (output["width"].take(), output["height"].take());
            output["width"] = height;
            output["height"] = width;
        }
        Ok(output)
    }

    /// How the primary output differs from the one the shell's frame was made
    /// for: another output, size or rotation needs a new shell; a new scale
    /// on the same output needs only the bar placed again.
    pub async fn primary_change(&self, frame_lost: bool) -> OutputChange {
        let Ok(output) = self.desktop_output().await else {
            return OutputChange::Same;
        };
        let current = OutputKey::of(&output);
        PRIMARY
            .lock()
            .ok()
            .and_then(|recorded| recorded.clone())
            .map_or(OutputChange::Same, |recorded| {
                recorded.change_to(&current, frame_lost)
            })
    }

    /// Tells the shell that a display's scale is about to change and waits
    /// until it has hidden its chrome, so the bar is not seen at the old
    /// output's size, or moved by the compositor while the change settles.
    /// Returns whether the shell answered: without a shell, or with one that
    /// does not know the request, the change goes ahead after a short wait.
    pub async fn hide_chrome_for_rescale(&self) -> bool {
        use tokio::io::AsyncBufReadExt;
        let Some(socket) = self.event_socket() else {
            return false;
        };
        let answer = async {
            // Listening before asking, so the answer cannot be missed.
            let stream = tokio::net::UnixStream::connect(&socket).await.ok()?;
            if stream.peer_cred().ok().map(|peer| peer.uid())
                != Some(rustix::process::getuid().as_raw())
            {
                return None;
            }
            self.request(&["dispatch", &event_call("enzui-command:output-rescaling")])
                .await
                .ok()?;
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line == format!("custom>>{RESCALE_HIDDEN}") {
                    return Some(());
                }
            }
            None
        };
        matches!(
            tokio::time::timeout(RESCALE_ANSWER, answer).await,
            Ok(Some(()))
        )
    }

    /// Tells the shell the scale change requested after
    /// [`Self::hide_chrome_for_rescale`] was applied or failed.
    pub async fn rescale_finished(&self) {
        let _ = self
            .request(&["dispatch", &event_call("enzui-command:output-rescaled")])
            .await;
    }

    /// Answers [`Self::hide_chrome_for_rescale`]: the chrome is hidden.
    pub async fn chrome_hidden_for_rescale(&self) {
        let _ = self
            .request(&["dispatch", &event_call(RESCALE_HIDDEN)])
            .await;
    }

    /// Whether the compositor has finished with its outputs: it pins an
    /// output-management change as monitor rules a moment later, and that
    /// moves windows on the output again. A compositor without pinning is
    /// settled, and so is one that cannot be asked.
    pub async fn outputs_settled(&self) -> bool {
        // `eval` answers only "ok"; a value comes back from `repl`, and a
        // batch splits at semicolons, so the query has none.
        let query = "repl return tostring(enzui_outputs_settled == nil or enzui_outputs_settled())";
        !matches!(
            self.request(&["--batch", query])
                .await
                .as_deref()
                .map(str::trim),
            Ok("false")
        )
    }
}

/// The shell's answer to a rescale announcement, on the event socket.
const RESCALE_HIDDEN: &str = "enzui-output-rescale-hidden";

/// How long Settings waits for the shell to hide its chrome before changing
/// a display's scale anyway: the shell answers once its fade, at most the
/// theme's normal motion, has reached the screen.
const RESCALE_ANSWER: Duration = Duration::from_millis(600);

fn event_call(name: &str) -> String {
    format!("hl.dsp.event(\"{name}\")")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputChange {
    Same,
    /// Same output, size and rotation at another scale. The frame helper
    /// follows the compositor's new logical size and buffer scale itself.
    Rescaled,
    Moved,
}

#[derive(Clone, PartialEq)]
pub(crate) struct OutputKey {
    name: String,
    width: u64,
    height: u64,
    scale: String,
    transform: i64,
}

impl OutputKey {
    pub(crate) fn of(output: &serde_json::Value) -> Self {
        Self {
            name: output["name"].as_str().unwrap_or_default().into(),
            width: output["width"].as_u64().unwrap_or(0),
            height: output["height"].as_u64().unwrap_or(0),
            scale: output["scale"].to_string(),
            transform: output["transform"].as_i64().unwrap_or(0),
        }
    }

    pub(crate) fn change_to(&self, current: &Self, frame_lost: bool) -> OutputChange {
        let rescaled = Self {
            scale: current.scale.clone(),
            ..self.clone()
        };
        if frame_lost || rescaled != *current {
            OutputChange::Moved
        } else if self.scale != current.scale {
            OutputChange::Rescaled
        } else {
            OutputChange::Same
        }
    }
}

/// Records the scale a bar was placed at, once the bar has been laid out for
/// it. Only a rescale is recorded: anything else still needs a new shell.
pub(crate) fn record_rescale(output: &serde_json::Value) {
    let current = OutputKey::of(output);
    if let Ok(mut recorded) = PRIMARY.lock()
        && let Some(primary) = recorded.as_mut()
        && primary.change_to(&current, false) == OutputChange::Rescaled
    {
        *primary = current;
    }
}

// The output the desktop frame was prepared for.
pub(crate) static PRIMARY: std::sync::Mutex<Option<OutputKey>> = std::sync::Mutex::new(None);

/// Leaves the planned-restart marker for the installed supervisor and ends
/// the shell.
pub fn restart_for_new_output() -> ! {
    if let Some(runtime) = std::env::var_os("ENZUI_HYPR_RUNROOT")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(runtime.join("outputs-changed"));
    }
    eprintln!("enzui-shell: the primary output changed; restarting on the new one");
    crate::idle::restore_outputs();
    std::process::exit(0)
}

// Retain the global tiling fallback for compositors without per-desktop policy.
fn desktop_mode_query(workspace: u32) -> String {
    format!(
        "repl return tostring(enzui_desktop_tiling_at and enzui_desktop_tiling_at({workspace}) or (enzui_desktop_tiling_at == nil and enzui_desktop.tiling == true))"
    )
}

fn desktop_modes_call(default: bool, modes: &std::collections::BTreeMap<u32, bool>) -> String {
    let modes: Vec<String> = modes
        .iter()
        .map(|(desktop, mode)| format!("[{desktop}]={mode}"))
        .collect();
    format!(
        "return enzui_set_desktop_modes({default}, {{{}}})",
        modes.join(",")
    )
}

fn request_line(arguments: &[&str]) -> String {
    let mut flags = String::new();
    let mut batch = false;
    let mut words = Vec::new();
    for argument in arguments {
        match *argument {
            "-j" if !flags.contains('j') => flags.push('j'),
            "--batch" => batch = true,
            word => words.push(word),
        }
    }
    let body = words.join(" ");
    if batch {
        format!("[[BATCH]]{body}")
    } else {
        format!("{flags}/{body}")
    }
}

async fn request_socket(socket: &std::path::Path, arguments: &[&str]) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|error| tr!("shell-compositor-control-4", "arg0" => format!("{:}", error)))?;
    if stream.peer_cred().ok().map(|peer| peer.uid()) != Some(rustix::process::getuid().as_raw()) {
        return Err(tr("shell-compositor-control-socket-belongs-to-another-user").into());
    }
    stream
        .write_all(request_line(arguments).as_bytes())
        .await
        .map_err(|error| tr!("shell-compositor-control-4", "arg0" => format!("{:}", error)))?;
    let mut reply = Vec::new();
    stream
        .take(16 << 20)
        .read_to_end(&mut reply)
        .await
        .map_err(|error| tr!("shell-compositor-control-4", "arg0" => format!("{:}", error)))?;
    let reply = String::from_utf8(reply).map_err(|_| tr("shell-invalid-compositor-response"))?;
    if let Some(reason) = reply.strip_prefix("error:") {
        let reason: String = reason
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        return Err(if reason.is_empty() {
            tr("shell-compositor-rejected-the-request").into()
        } else {
            tr!("shell-compositor-rejected-the-request-error", "arg0" => reason)
        });
    }
    Ok(reply)
}

/// Authorizes this process's next capture child; unsupported compositors may reject the
/// announcement.
/// Blocks for at most one second.
pub fn announce_capture(executable: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Write};
    let path = executable
        .to_str()
        .filter(|p| p.starts_with('/') && !p.contains(char::is_whitespace))
        .ok_or_else(|| tr("shell-unsupported-capture-helper-path"))?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| tr("shell-no-runtime-directory"))?;
    let instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|i| !i.is_empty() && !i.contains('/'))
        .ok_or_else(|| tr("shell-no-compositor-instance"))?;
    let mut stream = std::os::unix::net::UnixStream::connect(
        runtime.join("hypr").join(instance).join(".socket.sock"),
    )
    .map_err(|_| tr("shell-compositor-control-is-unavailable"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    stream
        .write_all(format!("/enzuiadmit intent capture {path} 3000").as_bytes())
        .map_err(|_| tr("shell-compositor-control-is-unavailable"))?;
    let mut reply = String::new();
    let _ = stream.take(4096).read_to_string(&mut reply);
    match reply.trim() {
        "ok" | "unknown request" => Ok(()),
        // The compositor's own reason ("not the registered shell", "peer
        // exited", ...) makes a refused capture diagnosable.
        other => Err(format!(
            "{} ({})",
            tr("shell-the-compositor-did-not-allow-this-capture"),
            other
                .trim_start_matches("error: ")
                .chars()
                .take(80)
                .collect::<String>()
        )),
    }
}

/// Whether Orca is installed (`None` when not), and whether it is running
/// for this account: the state [`Compositor::toggle_screen_reader`] flips.
pub fn screen_reader_state() -> Option<bool> {
    let installed = std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|dir| dir.is_absolute())
        .any(|dir| dir.join("orca").is_file());
    if !installed {
        return None;
    }
    let uid = rustix::process::getuid().as_raw();
    // Only for showing the switch's state: Super+Alt+S stops it by name too.
    let running = std::fs::read_dir("/proc").ok()?.flatten().any(|entry| {
        use std::os::unix::fs::MetadataExt;
        entry.metadata().is_ok_and(|meta| meta.uid() == uid)
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| comm.trim() == "orca")
    });
    Some(running)
}
