use super::types::Compositor;
use enzui_shell::i18n::tr;

impl Compositor {
    #[allow(
        clippy::too_many_arguments,
        reason = "The view composes independent state and actions supplied by its caller."
    )]
    pub async fn sidebar_configure(
        &self,
        side: crate::settings::SidebarEdge,
        bar: crate::settings::BarEdge,
        metrics: crate::settings::Metrics,
        enabled: bool,
        motion: crate::motion::Policy,
        behavior: Option<crate::sidebar_preferences::Preferences>,
        pin_scope: crate::sidebar_preferences::PinScope,
        dock_shares_edge: bool,
    ) -> Result<crate::sidebar::Snapshot, String> {
        let native = self.request(&["--batch","repl return (hl.plugin and hl.plugin.enzui_pointer_events and hl.plugin.enzui_pointer_events.version and hl.plugin.enzui_pointer_events.version()) or 0"]).await?;
        if native.trim() != "1" {
            return Err(tr("shell-miniapp-dragging-needs-a-newly-prepared-desktop-the-current-session-will-not").into());
        }
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-sidebar-output"))?;
        let side = if side == crate::settings::SidebarEdge::Left {
            "left"
        } else {
            "right"
        };
        let bar = if bar == crate::settings::BarEdge::Top {
            "top"
        } else {
            "bottom"
        };
        let lua = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\nenzui_install_workspace_keys();assert(enzui_sidebar_configure({},\"{side}\",\"{bar}\",{},{},{},{},\"{}\",{enabled},{dock_shares_edge}).ok);enzui_install_sidebar()",
            include_str!("../../../config/shell-metrics.lua"),
            include_str!("../../../config/sidebar-visibility.lua"),
            include_str!("../../../config/shell-reservations.lua"),
            include_str!("../../../config/desktop-policy.lua"),
            include_str!("../../../config/panel-policy.lua"),
            include_str!("../../../config/workspace-policy.lua"),
            concat!(
                "\n",
                include_str!("../../../config/output-size.lua"),
                "\n",
                include_str!("../../../config/sidebar-policy.lua")
            ),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            std::process::id(),
            metrics.lua(),
            motion.lua()?,
            behavior
                .map(super::super::sidebar_preferences::Preferences::lua)
                .transpose()?
                .unwrap_or_else(|| "nil".into()),
            pin_scope.key()
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-sidebar-configuration-rejected").into());
        }
        let state = self.sidebar_snapshot().await?;
        for (class, rect, passive) in [
            ("dev.enzui.Sidebar", state.source_rect(), !state.visible()),
            (state.backing_class(), state.pane, true),
        ] {
            let x = rect.x - state.origin[0];
            let y = rect.y - state.origin[1];
            let lua = format!(
                "hl.window_rule({{name={class_name},match={{class={pattern}}},float=true,pin=true,monitor={output},move=\"{x} {y}\",size=\"{w} {h}\",min_size=\"{w} {h}\",max_size=\"{w} {h}\",border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,no_initial_focus=true,no_focus={passive},no_xdg_drags=true}});for _,win in ipairs(hl.get_windows()) do if win.mapped and win.pid=={owner} and win.class=={class_name} then hl.dispatch(hl.dsp.window.move({{window=win,x={absolute_x},y={absolute_y}}}));hl.dispatch(hl.dsp.window.resize({{window=win,x={w},y={h}}})) end end",
                class_name =
                    serde_json::to_string(class).expect("serializing a string cannot fail"),
                pattern = serde_json::to_string(&format!("^{}$", class.replace('.', "[.]")))
                    .expect("serializing a string cannot fail"),
                output =
                    serde_json::to_string(&state.output).expect("serializing a string cannot fail"),
                w = rect.width,
                h = rect.height,
                absolute_x = rect.x,
                absolute_y = rect.y,
                owner = std::process::id(),
            );
            if self.request(&["eval", &lua]).await?.trim() != "ok" {
                return Err(tr("shell-sidebar-placement-rejected").into());
            }
        }
        Ok(state)
    }

    /// Keeps the rail's frames coming for `ms` while its look changes: the
    /// scene draws it from its last picture and sends it frames only when it
    /// draws.
    pub async fn wake_sidebar_rail(&self, ms: u64) -> Result<(), String> {
        let owner = std::process::id();
        let ms = ms.min(2000);
        self.request(&[
            "--batch",
            // No semicolons: the batch splits on them.
            &format!(
                "repl local a=hl.plugin and hl.plugin.enzui_sidebar_scene if a and type(a.rail_frames)==\"function\" then a.rail_frames({owner},{ms}) end return true"
            ),
        ])
        .await
        .map(|_| ())
    }

    pub async fn sidebar_snapshot(&self) -> Result<crate::sidebar::Snapshot, String> {
        crate::sidebar::Snapshot::parse(
            &self
                .request(&["--batch", "repl return enzui_sidebar_snapshot()"])
                .await?,
        )
    }

    pub async fn sidebar_action(
        &self,
        action: crate::sidebar::Action,
    ) -> Result<crate::sidebar::Snapshot, String> {
        if self
            .request(&["eval", &format!("return {}", action.lua())])
            .await?
            .trim()
            != "ok"
        {
            return Err(tr("shell-miniapp-action-rejected").into());
        }
        self.sidebar_snapshot().await
    }

    pub async fn open_terminal(&self) -> Result<(), String> {
        let response = self
            .request(&[
                "eval",
                r#"return hl.dispatch(hl.dsp.exec_cmd('exec "${ENZUI_TERMINAL:-foot}"'))"#,
            ])
            .await?;
        if response.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-terminal-launch-was-rejected").into())
        }
    }
}
