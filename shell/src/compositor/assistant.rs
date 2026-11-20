use super::{
    Compositor, ControlGeometry,
    policy::{control_geometry, shadowed},
};
use crate::settings::{AssistantCorner, BarEdge, Metrics};
use enzui_shell::i18n::tr;
use iced::{Rectangle, Size};

/// The assistant panel's preferred body size.
pub(super) const PANEL_BODY: Size = Size::new(460.0, 720.0);
/// The islands button's diameter.
pub const ISLAND_BUTTON: f32 = 44.0;
/// The frame tab's body, before its join into the frame.
pub const FRAME_TAB: Size = Size::new(48.0, 34.0);

/// Where the corner trigger's window sits and what it holds, in the
/// window's own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerLayout {
    /// The window, output-local.
    pub window: Rectangle,
    /// The button (islands) or the tab's reveal box (frame).
    pub button: Rectangle,
    /// Hover strips along the screen edges that reveal the button.
    pub strips: Vec<Rectangle>,
    /// The visible button's input bounds, combined with the reveal strips.
    pub shown: Rectangle,
    pub corner: AssistantCorner,
    /// The tab grows out of the frame (or the bar) instead of floating.
    pub joined: bool,
}

fn bar_on_edge(bar: BarEdge, corner: AssistantCorner) -> bool {
    (bar == BarEdge::Bottom) == corner.bottom()
}

/// How far the frame tab, and the panel that grows out of it, keep from
/// the screen's side: clear of the frame's own corner, since the tab's
/// fillet running into the corner's made one lumpy shape of the two.
fn frame_tab_inset(metrics: Metrics) -> f32 {
    f32::from(metrics.frame) + f32::from(metrics.radius) + 24.0
}

/// The trigger window's layout for an output of `screen` logical pixels.
#[must_use]
pub fn trigger_layout(
    screen: Size,
    metrics: Metrics,
    bar: BarEdge,
    corner: AssistantCorner,
) -> TriggerLayout {
    let on_bar = bar_on_edge(bar, corner);
    let (mut button, width, height, edge, side) = match metrics.islands {
        None => {
            let frame = f32::from(metrics.frame);
            let radius = f32::from(metrics.radius);
            let edge = if on_bar {
                f32::from(metrics.bar)
            } else {
                frame
            };
            let tab = Size::new(FRAME_TAB.width + 2.0 * radius, FRAME_TAB.height + radius);
            let inset = frame_tab_inset(metrics) - frame;
            let width = frame + inset + tab.width + 40.0;
            let height = edge + tab.height + 72.0;
            let x = if corner.right() {
                width - frame - inset - tab.width
            } else {
                frame + inset
            };
            let y = if corner.bottom() {
                height - edge - tab.height
            } else {
                edge
            };
            (
                Rectangle::new(iced::Point::new(x, y), tab),
                width,
                height,
                edge,
                frame,
            )
        }
        Some(style) => {
            let side = f32::from(style.side_gap);
            let edge = if on_bar {
                f32::from(metrics.bar_band())
            } else {
                f32::from(style.edge_gap)
            };
            let width = side + ISLAND_BUTTON + 56.0;
            let height = edge + ISLAND_BUTTON + 72.0;
            let x = if corner.right() {
                width - side - ISLAND_BUTTON
            } else {
                side
            };
            let y = if corner.bottom() {
                height - edge - ISLAND_BUTTON
            } else {
                edge
            };
            (
                Rectangle::new(
                    iced::Point::new(x, y),
                    Size::new(ISLAND_BUTTON, ISLAND_BUTTON),
                ),
                width,
                height,
                edge,
                side,
            )
        }
    };
    let x = if corner.right() {
        screen.width - width
    } else {
        0.0
    };
    let y = if corner.bottom() {
        screen.height - height
    } else {
        0.0
    };
    // Edge triggers must leave the bar band available for bar input.
    let thick = |inset: f32| inset.clamp(3.0, 10.0);
    // On the bar's edge the bar band stays out of the window's input.
    let band = if on_bar {
        if metrics.islands.is_some() {
            f32::from(metrics.bar_band())
        } else {
            f32::from(metrics.bar)
        }
    } else {
        0.0
    };
    let open = if corner.bottom() {
        Rectangle::new(iced::Point::ORIGIN, Size::new(width, height - band))
    } else {
        Rectangle::new(iced::Point::new(0.0, band), Size::new(width, height - band))
    };
    let mut strips = vec![Rectangle::new(
        iced::Point::new(
            if corner.right() {
                width - thick(side)
            } else {
                0.0
            },
            open.y,
        ),
        Size::new(thick(side), open.height),
    )];
    if !on_bar {
        strips.push(Rectangle::new(
            iced::Point::new(
                0.0,
                if corner.bottom() {
                    height - thick(edge)
                } else {
                    0.0
                },
            ),
            Size::new(width, thick(edge)),
        ));
    }
    let offset = if corner.bottom() { 0.0 } else { band };
    button.y -= offset;
    for strip in &mut strips {
        strip.y -= offset;
    }
    TriggerLayout {
        window: Rectangle::new(
            iced::Point::new(x, y + offset),
            Size::new(width, height - band),
        ),
        button,
        strips,
        shown: button,
        corner,
        joined: metrics.islands.is_none(),
    }
}

/// Returns output-local panel bounds and its frame or island attachment.
pub fn panel_layout(
    screen: Size,
    metrics: Metrics,
    bar: BarEdge,
    corner: AssistantCorner,
    body: Size,
) -> Result<(Rectangle, crate::reveal::Attachment), String> {
    use crate::reveal::Attachment;
    let on_bar = bar_on_edge(bar, corner);
    let radius = f32::from(metrics.radius);
    let (rect, attachment) = match metrics.islands {
        None => {
            let frame = f32::from(metrics.frame);
            let edge = if on_bar {
                f32::from(metrics.bar)
            } else {
                frame
            };
            let far = if on_bar {
                frame
            } else {
                f32::from(metrics.bar)
            };
            // It grows out of the edge where the tab sat, clear of the
            // frame's corner as the tab is, rather than from the side frame
            // (it opened somewhere else than the button that was hovered).
            let inset = frame_tab_inset(metrics);
            let width = body.width.min(screen.width - 2.0 * inset - 2.0 * radius) + 2.0 * radius;
            let height = body.height.min(screen.height - edge - far - 24.0) + radius;
            let x = if corner.right() {
                screen.width - inset - width
            } else {
                inset
            };
            let y = if corner.bottom() {
                screen.height - edge - height
            } else {
                edge
            };
            (
                Rectangle::new(iced::Point::new(x, y), Size::new(width, height)),
                Attachment::Bar,
            )
        }
        Some(style) => {
            let side = f32::from(style.side_gap);
            let edge = if on_bar {
                f32::from(metrics.bar_band())
            } else {
                f32::from(style.edge_gap)
            };
            let far = if on_bar {
                f32::from(style.edge_gap)
            } else {
                f32::from(metrics.bar_band())
            };
            let width = body.width.min(screen.width - 2.0 * side);
            let height = body.height.min(screen.height - edge - far - 24.0);
            let x = if corner.right() {
                screen.width - side - width
            } else {
                side
            };
            let y = if corner.bottom() {
                screen.height - edge - height
            } else {
                edge
            };
            let button = if corner.right() {
                width - ISLAND_BUTTON / 2.0
            } else {
                ISLAND_BUTTON / 2.0
            };
            (
                Rectangle::new(iced::Point::new(x, y), Size::new(width, height)),
                Attachment::Island {
                    center: button,
                    seed_width: ISLAND_BUTTON,
                },
            )
        }
    };
    if rect.width < 280.0 || rect.height < 240.0 {
        return Err(tr("shell-the-screen-is-too-small-for-the-assistant").into());
    }
    Ok((rect, attachment))
}

fn screen(output: &serde_json::Value) -> Result<(f32, f32, f64, f64), String> {
    let scale = output["scale"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-scale"))?;
    let width = (output["width"].as_f64().ok_or("Missing output width")? / scale) as f32;
    let height = (output["height"].as_f64().ok_or("Missing output height")? / scale) as f32;
    let x = output["x"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-origin"))?;
    let y = output["y"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-origin"))?;
    Ok((width, height, x, y))
}

impl Compositor {
    /// Places the corner trigger: pinned, never focused on opening.
    pub async fn prepare_assistant_trigger(
        &self,
        metrics: Metrics,
        bar: BarEdge,
        corner: AssistantCorner,
    ) -> Result<(String, TriggerLayout), String> {
        self.place_assistant_trigger(metrics, bar, corner, false)
            .await
    }

    /// Moves the mapped tab to its corner; an error while it is not mapped
    /// yet (a placement before mapping only sets up its rule).
    pub async fn move_assistant_trigger(
        &self,
        metrics: Metrics,
        bar: BarEdge,
        corner: AssistantCorner,
    ) -> Result<(), String> {
        self.place_assistant_trigger(metrics, bar, corner, true)
            .await
            .map(drop)
    }

    async fn place_assistant_trigger(
        &self,
        metrics: Metrics,
        bar: BarEdge,
        corner: AssistantCorner,
        mapped: bool,
    ) -> Result<(String, TriggerLayout), String> {
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output"))?
            .to_owned();
        let (width, height, origin_x, origin_y) = screen(&output)?;
        let layout = trigger_layout(Size::new(width, height), metrics, bar, corner);
        self.place(
            "dev.enzui.AssistantTrigger",
            "desktop-assistant-trigger",
            &name,
            layout.window,
            origin_x,
            origin_y,
            "no_initial_focus=true,",
            mapped,
        )
        .await?;
        Ok((name, layout))
    }

    /// Places the assistant panel with room for its shadow.
    pub async fn prepare_assistant_panel(
        &self,
        metrics: Metrics,
        bar: BarEdge,
        corner: AssistantCorner,
    ) -> Result<ControlGeometry, String> {
        let output = self.desktop_output().await?;
        control_geometry(&output)?;
        let name = output["name"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-output"))?
            .to_owned();
        let (width, height, origin_x, origin_y) = screen(&output)?;
        let (rect, attachment) =
            panel_layout(Size::new(width, height), metrics, bar, corner, PANEL_BODY)?;
        let (x, y, w, h) = shadowed(
            rect.x,
            f64::from(rect.y),
            rect.width,
            rect.height,
            corner.bottom(),
            attachment,
        );
        let window = Rectangle::new(iced::Point::new(x, y as f32), Size::new(w, h));
        self.place(
            "dev.enzui.AssistantPanel",
            "desktop-assistant-panel",
            &name,
            window,
            origin_x,
            origin_y,
            "",
            false,
        )
        .await?;
        Ok(ControlGeometry {
            width: rect.width,
            height: rect.height,
            output: name,
            attachment,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The view composes independent state and actions supplied by its caller."
    )]
    async fn place(
        &self,
        class: &str,
        rule: &str,
        output: &str,
        window: Rectangle,
        origin_x: f64,
        origin_y: f64,
        focus: &str,
        mapped: bool,
    ) -> Result<(), String> {
        let (x, y, width, height) = (
            window.x.round(),
            window.y.round(),
            window.width.round(),
            window.height.round(),
        );
        let lua = format!(
            // Register the initial rule only before mapping; reapplying it moves an already
            // placed window.
            // `sized` finds the window opened at this size: a replaced
            // window can still be mapped while its successor maps.
            "local found,sized=false,false;for _,w in ipairs(hl.get_windows()) do if w.mapped and w.pid=={} and w.class=={} then found=true;if math.abs(w.size.x-{width})<1.5 and math.abs(w.size.y-{height})<1.5 then sized=true end;hl.dispatch(hl.dsp.window.resize({{window=w,x={width},y={height}}})) hl.dispatch(hl.dsp.window.move({{window=w,x={},y={}}})) end end;if {mapped} and not sized then error('not mapped') end;if not found then hl.window_rule({{name={},match={{class={}}},float=true,pin=true,monitor={},border_size=0,rounding=0,decorate=false,no_anim=true,no_shadow=true,{focus}no_xdg_drags=true}}) end",
            std::process::id(),
            serde_json::to_string(class).expect("serializing a string cannot fail"),
            f64::from(x) + origin_x,
            f64::from(y) + origin_y,
            serde_json::to_string(rule).expect("serializing a string cannot fail"),
            serde_json::to_string(&format!("^{}$", class.replace('.', "[.]")))
                .expect("serializing a string cannot fail"),
            serde_json::to_string(output).expect("serializing a string cannot fail"),
        );
        if self.request(&["eval", &lua]).await?.trim() != "ok" {
            return Err(tr("shell-assistant-placement-was-rejected").into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_and_panel_keep_to_their_corner_and_clear_the_bar() {
        let screen = Size::new(1440.0, 900.0);
        let frame = Metrics::default();
        let layout = trigger_layout(screen, frame, BarEdge::Top, AssistantCorner::BottomRight);
        assert_eq!(layout.window.x + layout.window.width, 1440.0);
        assert_eq!(layout.window.y + layout.window.height, 900.0);
        assert!(layout.joined && layout.strips.len() == 2);
        assert_eq!(
            layout.button.y + layout.button.height,
            layout.window.height - f32::from(frame.frame)
        );
        let top = trigger_layout(screen, frame, BarEdge::Top, AssistantCorner::TopLeft);
        assert_eq!(
            (top.window.x, top.window.y, top.strips.len()),
            (0.0, f32::from(frame.bar), 1)
        );
        assert_eq!(top.shown.y, 0.0);
        assert_eq!(top.strips[0].y, 0.0);
        let (rect, attachment) = panel_layout(
            screen,
            frame,
            BarEdge::Top,
            AssistantCorner::BottomRight,
            PANEL_BODY,
        )
        .unwrap();
        assert_eq!(attachment, crate::reveal::Attachment::Bar);
        // Its edge lines up with the tab's, so it opens where the tab was.
        assert_eq!(
            rect.x + rect.width,
            layout.window.x + layout.button.x + layout.button.width
        );
        assert_eq!(rect.y + rect.height, 900.0 - f32::from(frame.frame));
        assert!(rect.y >= f32::from(frame.bar));
        let islands = Metrics {
            islands: Some(crate::shell_style::Islands::default()),
            ..frame
        };
        let (rect, attachment) = panel_layout(
            screen,
            islands,
            BarEdge::Bottom,
            AssistantCorner::BottomLeft,
            PANEL_BODY,
        )
        .unwrap();
        assert!(matches!(
            attachment,
            crate::reveal::Attachment::Island { .. }
        ));
        assert!(rect.y + rect.height <= 900.0 - f32::from(islands.bar_band()));
        assert!(
            panel_layout(
                Size::new(320.0, 240.0),
                frame,
                BarEdge::Top,
                AssistantCorner::BottomRight,
                PANEL_BODY
            )
            .is_err()
        );
    }
}
