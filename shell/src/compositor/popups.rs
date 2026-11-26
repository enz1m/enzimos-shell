use super::{
    policy::{
        attached_y, bottom_panel_anchor, control_geometry, launcher_rule_selection,
        notification_toast_height, popup_layout, shadowed,
    },
    types::{
        Compositor, ControlContent, ControlGeometry, MAX_TOAST_OVERSHOOT, PopupPlacement,
        ToastPlacement,
    },
};
use enzui_shell::i18n::tr;

impl Compositor {
    /// Places the shortcut reference over the Shell's output.
    pub async fn prepare_shortcuts_overlay(&self) -> Result<iced::Size, String> {
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output"))?;
        let scale = output["scale"]
            .as_f64()
            .filter(|scale| (0.5..=4.0).contains(scale))
            .ok_or_else(|| tr("shell-invalid-output-scale"))?;
        let dimension = |axis: &str| {
            output[axis]
                .as_f64()
                .map(|value| (value / scale).round())
                .filter(|value| (1.0..=16384.0).contains(value))
                .ok_or_else(|| tr("shell-shortcuts-overlay-unavailable"))
        };
        let (width, height) = (dimension("width")?, dimension("height")?);
        let monitor = serde_json::json!(name);
        let size = serde_json::json!(format!("{width} {height}"));
        let lua = format!(
            "hl.window_rule({{name='desktop-overlay-shortcuts',match={{class='^dev[.]enzui[.]Shortcuts$'}},float=true,pin=true,monitor={monitor},move='0 0',size={size},min_size={size},max_size={size},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,no_blur=true,no_xdg_drags=true}})"
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-shortcuts-overlay-unavailable").into());
        }
        Ok(iced::Size::new(width as f32, height as f32))
    }

    /// Requests the running Shell's shortcut reference from Settings.
    pub async fn show_shortcuts_overlay(&self) -> Result<(), String> {
        let script = "return hl.dispatch(hl.dsp.event('enzui-command:show-shortcuts'))";
        if self.request(&["eval", script]).await?.trim() != "ok" {
            return Err(tr("shell-shortcuts-overlay-unavailable").into());
        }
        Ok(())
    }

    pub async fn prepare_overview(
        &self,
        edge: crate::settings::BarEdge,
        metrics: crate::settings::Metrics,
        anchor: Option<iced::Rectangle>,
        width: f32,
    ) -> Result<ControlGeometry, String> {
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-desktop-output"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32;
        let screen_height =
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32;
        let (x, width, attachment) = if metrics.islands.is_some() {
            popup_layout(
                screen,
                metrics,
                width.clamp(264.0, 776.0),
                0.0,
                0.0,
                anchor.unwrap_or_else(|| {
                    iced::Rectangle::new(
                        iced::Point::new((screen - 1.0) / 2.0, 0.0),
                        iced::Size::new(1.0, f32::from(metrics.bar)),
                    )
                }),
            )?
        } else {
            (0.0, screen, Default::default())
        };
        // Room for the tallest desktop picture and its ring; an island's
        // popover is no taller than that.
        let cards = 188.0;
        let mut height = if metrics.islands.is_some() {
            cards
        } else {
            (screen_height * 0.25 - f32::from(metrics.bar)).clamp(cards, 360.0)
        } + attachment.join(f32::from(metrics.radius));
        if let Some(style) = metrics.islands {
            height = height
                .min(screen_height - f32::from(metrics.bar_band()) - f32::from(style.edge_gap));
            if height < 80.0 {
                return Err(tr("shell-no-island-overview-work-area").into());
            }
        }
        let y = attached_y(&output, height, edge, metrics)?;
        let panel = (width, height);
        let (x, y, width, height) = if metrics.islands.is_some() {
            shadowed(
                x,
                y,
                width,
                height,
                edge == crate::settings::BarEdge::Bottom,
                attachment,
            )
        } else {
            (x, y, width, height)
        };
        let policy = format!(
            "{}\nhl.window_rule({{name=\"desktop-overview\",match={{class=\"^dev[.]enzui[.]WorkspaceOverview$\"}},float=true,pin=true,monitor={},move={},size={},border_size=0,rounding=0,decorate=false,no_shadow=true,no_anim=true,no_xdg_drags=true}})",
            include_str!("../../../config/control-center.lua"),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{x} {y}")),
            serde_json::json!(format!("{width} {height}"))
        );
        if self.request(&["eval", &policy]).await?.trim() != "ok" {
            return Err(tr("shell-desktop-overview-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width: panel.0,
            height: panel.1,
            output: name.into(),
            attachment,
        })
    }

    pub async fn prepare_control_center(
        &self,
        edge: crate::settings::BarEdge,
        left_edge: u32,
        right_edge: u32,
        metrics: crate::settings::Metrics,
        anchor: iced::Rectangle,
        controls: ControlContent,
    ) -> Result<ControlGeometry, String> {
        if left_edge > 64 || right_edge > 64 {
            return Err(tr("shell-invalid-desktop-frame-inset").into());
        }
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let maximum_width = (output["width"]
            .as_f64()
            .expect("control_geometry validated finite output dimensions and scale")
            / output["scale"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
            - 24.0) as f32;
        let radius = f32::from(metrics.radius);
        let padding = crate::panel_style::INSET;
        let width = (crate::control_layout::grid_width_for(
            f32::from(metrics.text),
            controls.grid.layout.column_count(),
        ) + padding * 2.0)
            .min(maximum_width);
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output-name"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen_width =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round();
        let (x, width, attachment) = popup_layout(
            screen_width as f32,
            metrics,
            width,
            left_edge as f32,
            right_edge as f32,
            anchor,
        )?;
        let (left, right) = attachment.insets(radius);
        let preferred = controls.reserved_height(
            width - left - right - padding * 2.0,
            crate::control_layout::row_height(f32::from(metrics.text)),
            padding,
            attachment.join(radius),
        );
        let height = preferred.min(
            ((output["height"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
                / scale)
                .round()
                - f64::from(metrics.bar_band())
                - f64::from(
                    metrics
                        .islands
                        .map_or(metrics.frame, |style| style.edge_gap),
                )) as f32,
        );
        let y = attached_y(&output, height, edge, metrics)?;
        let panel = (width, height);
        let (x, y, width, height) = shadowed(
            x,
            y,
            width,
            height,
            edge == crate::settings::BarEdge::Bottom,
            attachment,
        );
        let lua = format!(
            "hl.window_rule({{name=\"desktop-control-center\",match={{class=\"^dev[.]enzui[.]ControlCenter$\"}},float=true,pin=true,monitor={},move={},size={},min_size=\"{width} {height}\",max_size=\"{width} {height}\",border_size=0,rounding=0,decorate=false,no_anim=true,no_initial_focus=true,opacity=\"0 override\",render_unfocused=true,no_shadow=true,no_xdg_drags=true}})",
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{x} {y}")),
            serde_json::json!(format!("{width} {height}"))
        );
        let pid = std::process::id();
        let absolute_x = x + output["x"].as_f64().ok_or("Missing output origin")? as f32;
        let absolute_y = y + output["y"].as_f64().ok_or("Missing output origin")?;
        let anchor = bottom_panel_anchor(edge, pid, absolute_x, absolute_y, width, height);
        let lua = format!(
            "{}\n{lua}\nfor _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={pid} and w.class==\"dev.enzui.ControlCenter\" then {anchor}hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}})) hl.dispatch(hl.dsp.window.move({{window=w,x={absolute_x},y={absolute_y}}})) end end",
            include_str!("../../../config/control-center.lua")
        );
        let response = self.request(&["eval", &lua]).await?;
        if response.trim() != "ok" {
            return Err(tr("shell-control-center-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width: panel.0,
            height: panel.1,
            output: name.into(),
            attachment,
        })
    }

    pub async fn prepare_launcher(
        &self,
        placement: PopupPlacement,
    ) -> Result<ControlGeometry, String> {
        if placement.metrics.islands.is_some() {
            return self
                .prepare_popup(
                    placement,
                    "dev.enzui.Launcher",
                    "desktop-island-launcher",
                    false,
                    false,
                    false,
                )
                .await;
        }
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-launcher-output"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let width = 720.0_f32.min(
            (output["width"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
                / scale)
                .round() as f32
                - f32::from(placement.metrics.frame) * 2.0,
        );
        let height = 500.0_f32.min(
            (output["height"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
                / scale)
                .round() as f32
                - f32::from(placement.metrics.bar + placement.metrics.frame),
        );
        if width < 240.0 || height < 160.0 {
            return Err(tr("shell-output-is-too-small-for-applications").into());
        }
        let lua = format!(
            "{}hl.window_rule({{name=\"desktop-frame-launcher\",match={{class=\"^dev[.]enzui[.]Launcher$\"}},float=true,pin=false,center=true,monitor={},size={},border_size=0,rounding=16,no_anim=true}})",
            launcher_rule_selection(false),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{width} {height}"))
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-applications-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width,
            height,
            output: name.into(),
            attachment: Default::default(),
        })
    }

    pub async fn prepare_plugin_panel(
        &self,
        placement: PopupPlacement,
    ) -> Result<ControlGeometry, String> {
        self.prepare_popup(
            placement,
            "dev.enzui.PluginPanel",
            "desktop-plugin-panel",
            true,
            false,
            false,
        )
        .await
    }
    /// `open` says the center is on screen and is being placed again for a
    /// new height. It resizes itself; the scene holds its edge by the bar
    /// meanwhile, or the compositor would keep the window's centre and
    /// slide it over the bar.
    pub async fn prepare_notification_center(
        &self,
        placement: PopupPlacement,
        open: bool,
        passive: bool,
    ) -> Result<ControlGeometry, String> {
        self.prepare_popup(
            placement,
            "dev.enzui.NotificationCenter",
            "desktop-notification-center",
            false,
            open,
            passive,
        )
        .await
    }
    pub async fn activity_pointer_inside(&self, anchor: iced::Rectangle) -> bool {
        let owner = std::process::id();
        let right = anchor.x + anchor.width;
        let bottom = anchor.y + anchor.height;
        let query = format!(
            "repl local p=hl.get_cursor_pos() for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={owner} then local x=p.x-w.at.x local y=p.y-w.at.y if w.class==\"dev.enzui.Desktop\" and x>={left} and x<{right} and y>={top} and y<{bottom} then return true end if w.class==\"dev.enzui.NotificationCenter\" and x>=0 and y>=0 and x<w.size.x and y<w.size.y then return true end end end return false",
            left = anchor.x,
            top = anchor.y,
        );
        self.request(&["--batch", &query])
            .await
            .is_ok_and(|value| value.trim() == "true")
    }

    async fn prepare_popup(
        &self,
        placement: PopupPlacement,
        class: &str,
        rule: &str,
        resize_existing: bool,
        hold_existing: bool,
        passive: bool,
    ) -> Result<ControlGeometry, String> {
        let PopupPlacement {
            bar,
            left,
            right,
            metrics,
            anchor,
            body,
        } = placement;
        if left > 64
            || right > 64
            || !body.width.is_finite()
            || !body.height.is_finite()
            || !(240.0..=640.0).contains(&body.width)
            || !(120.0..=800.0).contains(&body.height)
        {
            return Err(tr("shell-invalid-plugin-panel-geometry").into());
        }
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-plugin-panel-output"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen_width =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32;
        let anchor = anchor.unwrap_or_else(|| {
            iced::Rectangle::new(
                iced::Point::new((screen_width - 1.0) / 2.0, 0.0),
                iced::Size::new(1.0, f32::from(metrics.bar)),
            )
        });
        let (x, width, attachment) = popup_layout(
            screen_width,
            metrics,
            body.width,
            left as f32,
            right as f32,
            anchor,
        )?;
        let height = (body.height + attachment.join(f32::from(metrics.radius))).min(
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32
                - f32::from(metrics.bar_band())
                - f32::from(
                    metrics
                        .islands
                        .map_or(metrics.frame, |style| style.edge_gap),
                ),
        );
        let y = attached_y(&output, height, bar, metrics)?;
        let panel = (width, height);
        let (x, y, width, height) = shadowed(
            x,
            y,
            width,
            height,
            bar == crate::settings::BarEdge::Bottom,
            attachment,
        );
        let size_lock = if resize_existing {
            format!(",min_size=\"{width} {height}\",max_size=\"{width} {height}\"")
        } else {
            String::new()
        };
        let resize = if resize_existing {
            let anchor = bottom_panel_anchor(
                bar,
                std::process::id(),
                x + output["x"].as_f64().ok_or("Missing output origin")? as f32,
                y + output["y"].as_f64().ok_or("Missing output origin")?,
                width,
                height,
            );
            format!("{anchor}hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}}));")
        } else {
            String::new()
        };
        let hold = if hold_existing {
            let pid = std::process::id();
            let (gx, gy) = (
                x + output["x"].as_f64().ok_or("Missing output origin")? as f32,
                y + output["y"].as_f64().ok_or("Missing output origin")?,
            );
            // Asked, not required: a scene that does not anchor this
            // popover leaves it to resize as before rather than failing the
            // placement. Under a top bar the top edge stays; over a bottom
            // bar, the default, the bottom one.
            let edge = if bar == crate::settings::BarEdge::Bottom {
                ""
            } else {
                ",false"
            };
            format!(
                "local scene=hl.plugin and hl.plugin.enzui_sidebar_scene;if scene and type(scene.anchor_panel)==\"function\" and scene.owner_alive({pid}) then scene.anchor_panel(w.address,{pid},{gx},{gy},{width},{height}{edge}) end;"
            )
        } else {
            String::new()
        };
        let switch = if class == "dev.enzui.Launcher" {
            launcher_rule_selection(true)
        } else {
            ""
        };
        let focus_rule = if passive {
            r#"hl.window_rule({name="desktop-activity-preview",match={class="^dev[.]enzui[.]NotificationCenter$"},enabled=true,no_initial_focus=true,no_follow_mouse=true});"#
        } else if class == "dev.enzui.NotificationCenter" {
            r#"hl.window_rule({name="desktop-activity-preview",enabled=false});"#
        } else {
            ""
        };
        let lua = format!(
            "{}\n{switch}{focus_rule}hl.window_rule({{name={},match={{class={}}},float=true,pin=true,monitor={},move={},size={},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,no_xdg_drags=true{size_lock}}});for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class=={} then {hold}{resize}hl.dispatch(hl.dsp.window.move({{window=w,x={},y={}}})) end end",
            include_str!("../../../config/control-center.lua"),
            serde_json::to_string(rule).expect("serializing a string cannot fail"),
            serde_json::to_string(&format!("^{}$", class.replace('.', "[.]")))
                .expect("serializing a string cannot fail"),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{x} {y}")),
            serde_json::json!(format!("{width} {height}")),
            std::process::id(),
            serde_json::to_string(class).expect("serializing a string cannot fail"),
            x + output["x"]
                .as_f64()
                .ok_or_else(|| tr("shell-missing-output-origin"))? as f32,
            y + output["y"]
                .as_f64()
                .ok_or_else(|| tr("shell-missing-output-origin"))?
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-plugin-panel-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width: panel.0,
            height: panel.1,
            output: name.into(),
            attachment,
        })
    }

    pub async fn prepare_notification_toasts(
        &self,
        placement: ToastPlacement,
    ) -> Result<ControlGeometry, String> {
        use crate::{notifications::preferences::Position, settings::BarEdge};
        let ToastPlacement {
            bar,
            metrics,
            left,
            right,
            width,
            heights,
            count,
            position,
            overshoot,
        } = placement;
        use crate::notifications::card::SHADOW_MARGIN;
        if left > 64
            || right > 64
            || !width.is_finite()
            || !(240.0..=640.0).contains(&width)
            || !(0.0..=MAX_TOAST_OVERSHOOT).contains(&overshoot)
        {
            return Err(tr("shell-invalid-notification-toast-geometry").into());
        }
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-toast-output"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen_width =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32;
        let screen_height =
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32;
        // Reserve window shadow margins beyond the panel's visible edge gap.
        let edge_gap = if metrics.islands.is_some() { 0.0 } else { 10.0 };
        let inset = |edge: u32| (edge as f32 + edge_gap).max(SHADOW_MARGIN);
        // Beside the bar the cards clear its band (islands float below their
        // edge gap); elsewhere they keep the frame's inner gap.
        let band =
            f32::from(metrics.bar_band()) + if metrics.islands.is_some() { 0.0 } else { 8.0 };
        let far =
            f32::from(metrics.screen_inset()) + if metrics.islands.is_some() { 0.0 } else { 8.0 };
        let top = if bar == BarEdge::Top { band } else { far } - SHADOW_MARGIN;
        let bottom = if bar == crate::settings::BarEdge::Bottom {
            band
        } else {
            far
        } - SHADOW_MARGIN;
        let card_width = width.min((screen_width - inset(left) - inset(right)).max(1.0));
        let width = card_width + 2.0 * SHADOW_MARGIN + overshoot;
        let height =
            notification_toast_height(&heights[..count.min(5)], screen_height - top - bottom)?;
        let lower = position.is_bottom(bar);
        let start = matches!(position, Position::TopLeft | Position::BottomLeft);
        let x = if start {
            inset(left) - SHADOW_MARGIN
        } else {
            screen_width - inset(right) - card_width - SHADOW_MARGIN - overshoot
        };
        let y = if lower {
            screen_height - bottom - height
        } else {
            top
        };
        let absolute_x = x + output["x"].as_f64().ok_or("Missing output origin")? as f32;
        let absolute_y = y + output["y"].as_f64().ok_or("Missing output origin")? as f32;
        // Pinned, so a later client resize cannot re-centre the stack.
        let size = serde_json::json!(format!("{width} {height}"));
        let lua = format!(
            "hl.window_rule({{name=\"desktop-notification-toasts\",match={{class=\"^dev[.]enzui[.]NotificationToasts$\"}},float=true,pin=true,monitor={},move={},size={},min_size={size},max_size={size},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,no_initial_focus=true,no_xdg_drags=true}});for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class==\"dev.enzui.NotificationToasts\" then hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}})) hl.dispatch(hl.dsp.window.move({{window=w,x={absolute_x},y={absolute_y}}})) end end",
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{x} {y}")),
            serde_json::json!(format!("{width} {height}")),
            std::process::id()
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-notification-toast-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width,
            height,
            output: name.into(),
            attachment: Default::default(),
        })
    }

    /// Returns focus from the desktop or a retired popup, preserving a new popup.
    pub async fn return_focus_to_app(&self) -> Result<(), String> {
        if crate::session_lock::locked() {
            return Ok(());
        }
        let active: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "activewindow"]).await?).unwrap_or_default();
        let class = active["class"].as_str().unwrap_or_default();
        let desktop = matches!(class, "" | "dev.enzui.Desktop" | "dev.enzui.Sidebar");
        let popup = matches!(
            class,
            "dev.enzui.QuickSearch"
                | "dev.enzui.ControlCenter"
                | "dev.enzui.NotificationCenter"
                | "dev.enzui.WorkspaceOverview"
        );
        let retired = active["mapped"].as_bool() == Some(false)
            || active["hidden"].as_bool() == Some(true)
            || active["visible"].as_bool() == Some(false);
        if desktop || (popup && retired) {
            self.focus_previous().await
        } else {
            Ok(())
        }
    }

    /// Focuses the app window used most recently on the active desktop,
    /// passing over the shell's own surfaces (the bar, overlays).
    pub async fn focus_previous(&self) -> Result<(), String> {
        let windows = self.switcher_windows().await?;
        let Some(entry) = windows.into_iter().next() else {
            return Ok(());
        };
        if !entry
            .target
            .address
            .chars()
            .all(|c| c.is_ascii_alphanumeric())
        {
            return Err(tr("shell-invalid-window-address").into());
        }
        // Without warping: the pointer stays on the key that was pressed.
        let lua = format!(
            "local old=hl.get_config(\"cursor.no_warps\");hl.config({{cursor={{no_warps=true}}}});local found=false;for _,w in ipairs(hl.get_windows()) do if w.mapped and w.address=={} and w.pid=={} then hl.dispatch(hl.dsp.focus({{window=w}}));found=true end end;hl.config({{cursor={{no_warps=old}}}});return found and \"ok\" or \"missing\"",
            serde_json::to_string(&entry.target.address)
                .map_err(|_| tr("shell-invalid-window-address"))?,
            entry.target.pid,
        );
        if self.request(&["eval", &lua]).await?.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-the-window-being-typed-into-went-away").into())
        }
    }

    /// Places a shell-owned overlay; Capture always covers the complete output.
    pub async fn prepare_overlay(
        &self,
        class: &str,
        width: f32,
        height: f32,
        centred: bool,
        metrics: crate::settings::Metrics,
        bar: crate::settings::BarEdge,
    ) -> Result<ControlGeometry, String> {
        if !matches!(
            class,
            "dev.enzui.Osd"
                | "dev.enzui.Switcher"
                | "dev.enzui.EmojiPicker"
                | "dev.enzui.WritingTools"
                | "dev.enzui.Keyboard"
                | "dev.enzui.Capture"
        ) {
            return Err(tr("shell-unknown-overlay").into());
        }
        // Hyprland denies pointer input to no-focus windows; the keyboard instead skips focus
        // on mapping.
        let focus = match class {
            "dev.enzui.EmojiPicker" | "dev.enzui.Capture" | "dev.enzui.WritingTools" => "",
            "dev.enzui.Keyboard" => "no_initial_focus=true,",
            _ => "no_initial_focus=true,no_focus=true,",
        };
        if !(width.is_finite()
            && height.is_finite()
            && (1.0..=1600.0).contains(&width)
            && (1.0..=900.0).contains(&height))
        {
            return Err(tr("shell-invalid-overlay-geometry").into());
        }
        let pointer = if class == "dev.enzui.WritingTools" {
            let reply = self.request(&["-j", "cursorpos"]).await?;
            let cursor: serde_json::Value =
                serde_json::from_str(&reply).map_err(|_| tr("shell-invalid-overlay-geometry"))?;
            cursor["x"].as_f64().zip(cursor["y"].as_f64())
        } else {
            None
        };
        let mut output = self.desktop_output().await?;
        if let Some((x, y)) = pointer {
            let monitors = self.request(&["-j", "monitors"]).await?;
            if let Ok(monitors) = serde_json::from_str::<Vec<serde_json::Value>>(&monitors)
                && let Some(monitor) = monitors.into_iter().find_map(|mut output| {
                    if output["transform"]
                        .as_i64()
                        .is_some_and(|transform| transform % 2 == 1)
                    {
                        let (width, height) = (output["width"].take(), output["height"].take());
                        output["width"] = height;
                        output["height"] = width;
                    }
                    let (ox, oy) = (output["x"].as_f64()?, output["y"].as_f64()?);
                    let scale = output["scale"].as_f64()?;
                    let (w, h) = (
                        output["width"].as_f64()? / scale,
                        output["height"].as_f64()? / scale,
                    );
                    (x >= ox && x < ox + w && y >= oy && y < oy + h).then_some(output)
                })
            {
                output = monitor;
            }
        }
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output"))?;
        let scale = output["scale"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let screen_width =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32;
        let screen_height =
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32;
        let (width, height) = if class == "dev.enzui.Capture" {
            (screen_width.round(), screen_height.round())
        } else {
            (width, height)
        };
        let bottom = if bar == crate::settings::BarEdge::Bottom {
            f32::from(metrics.bar_band())
        } else {
            f32::from(metrics.screen_inset())
        } + 48.0;
        let x = ((screen_width - width) / 2.0).max(0.0).round();
        let y = if centred {
            ((screen_height - height) / 2.0).max(0.0).round()
        } else {
            (screen_height - bottom - height).max(0.0).round()
        };
        let ox = output["x"].as_f64().ok_or("Missing output origin")? as f32;
        let oy = output["y"].as_f64().ok_or("Missing output origin")? as f32;
        let (x, y) = if let Some((px, py)) = pointer {
            let inset = f32::from(metrics.bar_band());
            let px = px as f32 - ox;
            let py = py as f32 - oy;
            let below = py + 14.0;
            let y = if below + height <= screen_height - inset {
                below
            } else {
                py - height - 14.0
            };
            (
                (px + 12.0).clamp(0.0, (screen_width - width).max(0.0)),
                y.clamp(inset, (screen_height - height - inset).max(inset)),
            )
        } else {
            (x, y)
        };
        let absolute_x = x + ox;
        let absolute_y = y + oy;
        let size = serde_json::json!(format!("{width} {height}"));
        let rule = class
            .replace("dev.enzui.", "desktop-overlay-")
            .to_lowercase();
        let pattern = format!("^{}$", class.replace('.', "[.]"));
        let lua = format!(
            "hl.window_rule({{name={},match={{class={}}},float=true,pin=true,monitor={},move={},size={size},min_size={size},max_size={size},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,{focus}no_xdg_drags=true}});for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class=={} then hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}})) hl.dispatch(hl.dsp.window.move({{window=w,x={absolute_x},y={absolute_y}}})) end end",
            serde_json::to_string(&rule).expect("serializing a string cannot fail"),
            serde_json::to_string(&pattern).expect("serializing a string cannot fail"),
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!(format!("{x} {y}")),
            std::process::id(),
            serde_json::to_string(class).expect("serializing a string cannot fail"),
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-overlay-placement-was-rejected").into());
        }
        Ok(ControlGeometry {
            width,
            height,
            output: name.into(),
            attachment: Default::default(),
        })
    }

    /// Places a keyboard-focused spread over the shell output, leaving the bar above it.
    pub async fn prepare_spread(
        &self,
        metrics: crate::settings::Metrics,
        bar: crate::settings::BarEdge,
        take_focus: bool,
    ) -> Result<([f32; 4], f32, f32), String> {
        let output = self.desktop_output().await?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output"))?;
        let scale = output["scale"]
            .as_f64()
            .filter(|s| (0.5..=4.0).contains(s))
            .ok_or_else(|| tr("shell-missing-output-scale"))?;
        let width =
            (output["width"].as_f64().ok_or("Missing output width")? / scale).round() as f32;
        let height =
            (output["height"].as_f64().ok_or("Missing output height")? / scale).round() as f32;
        let band = f32::from(metrics.bar_band());
        let (top, bottom) = if bar == crate::settings::BarEdge::Top {
            (band, 0.0)
        } else {
            (0.0, band)
        };
        if !(1.0..=16384.0).contains(&width) || !(band + 1.0..=16384.0).contains(&height) {
            return Err(tr("shell-invalid-spread-geometry").into());
        }
        let origin = [
            output["x"]
                .as_f64()
                .ok_or_else(|| tr("shell-missing-output-origin"))? as f32,
            output["y"]
                .as_f64()
                .ok_or_else(|| tr("shell-missing-output-origin"))? as f32,
        ];
        let size = serde_json::json!(format!("{width} {height}"));
        let lua = format!(
            "hl.window_rule({{name='desktop-overlay-windowspread',match={{class='^dev[.]enzui[.]WindowSpread$'}},float=true,pin=true,monitor={},move={},size={size},min_size={size},max_size={size},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,no_blur=true,no_xdg_drags=true,no_initial_focus={}}});for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class=='dev.enzui.WindowSpread' then hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}})) hl.dispatch(hl.dsp.window.move({{window=w,x={},y={}}})) end end",
            serde_json::to_string(name).expect("serializing a string cannot fail"),
            serde_json::json!("0 0"),
            !take_focus,
            std::process::id(),
            origin[0],
            origin[1],
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-window-spread-placement-was-rejected").into());
        }
        Ok(([origin[0], origin[1], width, height], top, bottom))
    }

    /// Hides the spread's windows while its pictures stand in for them.
    pub async fn spread_hide(&self, addresses: &[String]) -> Result<(), String> {
        let list = addresses
            .iter()
            .map(|address| spread_address(address))
            .collect::<Result<Vec<_>, _>>()?;
        let reply = self
            .request(&[
                "eval",
                &format!("return enzui_spread_hide({{{}}})", list.join(",")),
            ])
            .await?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-the-windows-could-not-be-hidden").into())
        }
    }

    /// Shows the spread's windows again and focuses `focus`, if given.
    pub async fn spread_restore(&self, focus: Option<&str>) -> Result<(), String> {
        let focus = focus
            .map(spread_address)
            .transpose()?
            .unwrap_or_else(|| "nil".into());
        let reply = self
            .request(&["eval", &format!("return enzui_spread_restore({focus})")])
            .await?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(tr("shell-the-windows-could-not-be-shown-again").into())
        }
    }
}

// A window address as a Lua string literal.
fn spread_address(address: &str) -> Result<String, String> {
    let valid = address.len() > 2
        && address.len() <= 20
        && address.starts_with("0x")
        && address
            .strip_prefix("0x")
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()));
    if valid {
        Ok(format!("'{address}'"))
    } else {
        Err(tr("shell-invalid-window").into())
    }
}
