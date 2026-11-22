use enzui_shell::i18n::tr;
pub(super) fn launcher_rule_selection(islands: bool) -> &'static str {
    // Named Lua rules merge effects; independent style rules prevent stale placement.
    if islands {
        "hl.window_rule({name=\"desktop-launcher\",enabled=false})\nhl.window_rule({name=\"desktop-frame-launcher\",enabled=false})\n"
    } else {
        "hl.window_rule({name=\"desktop-launcher\",enabled=false})\nhl.window_rule({name=\"desktop-island-launcher\",enabled=false})\n"
    }
}

pub(crate) fn popup_layout(
    screen: f32,
    metrics: crate::settings::Metrics,
    body: f32,
    left: f32,
    right: f32,
    anchor: iced::Rectangle,
) -> Result<(f32, f32, crate::reveal::Attachment), String> {
    let Some(style) = metrics.islands else {
        return popup_horizontal(
            screen,
            f32::from(metrics.bar),
            body,
            f32::from(metrics.radius),
            left,
            right,
            anchor,
        );
    };
    if ![
        screen,
        body,
        anchor.x,
        anchor.y,
        anchor.width,
        anchor.height,
    ]
    .into_iter()
    .all(f32::is_finite)
        || body <= 0.0
        || anchor.width <= 0.0
        || anchor.height <= 0.0
    {
        return Err(tr("shell-invalid-island-trigger-geometry").into());
    }
    let anchor = anchor
        .intersection(&iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: screen,
            height: f32::from(metrics.bar),
        })
        .ok_or_else(|| tr("shell-island-trigger-is-outside-the-bar"))?;
    let margin = f32::from(style.side_gap);
    let width = body.min(screen - margin * 2.0).max(1.0);
    if width + margin * 2.0 > screen {
        return Err(tr("shell-no-island-panel-work-area").into());
    }
    let (low, high) = (margin, screen - margin - width);
    let x = (anchor.center_x() - width / 2.0).round().clamp(low, high);
    // A panel that would stop just short of an edge docks to it, so it lines
    // up with the edge-docked panels instead of leaving a sliver of desktop.
    const SNAP: f32 = 48.0;
    let x = if high - x < SNAP {
        high
    } else if x - low < SNAP {
        low
    } else {
        x
    };
    let seed_width = if anchor.width <= 1.0 {
        0.0
    } else {
        anchor.width.min(width)
    };
    let center = (anchor.center_x() - x).clamp(seed_width / 2.0, width - seed_width / 2.0);
    Ok((
        x,
        width,
        crate::reveal::Attachment::Island { center, seed_width },
    ))
}

pub(super) fn popup_horizontal(
    screen: f32,
    bar: f32,
    body: f32,
    radius: f32,
    left: f32,
    right: f32,
    anchor: iced::Rectangle,
) -> Result<(f32, f32, crate::reveal::Attachment), String> {
    use crate::reveal::Attachment;
    if ![anchor.x, anchor.y, anchor.width, anchor.height]
        .into_iter()
        .all(f32::is_finite)
        || anchor.width <= 0.0
        || anchor.height <= 0.0
    {
        return Err(tr("shell-invalid-popup-trigger-geometry").into());
    }
    let anchor = anchor
        .intersection(&iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: screen,
            height: bar,
        })
        .ok_or_else(|| tr("shell-popup-trigger-is-outside-the-bar"))?;
    let available = screen - left - right;
    if available <= 0.0 {
        return Err(tr("shell-no-popup-work-area").into());
    }
    if body + 2.0 * radius >= available {
        return Ok((left, available, Attachment::BothFrames));
    }
    let width = body + 2.0 * radius;
    let x = anchor.center_x() - width / 2.0;
    // A panel that would stop just short of the frame joins it instead of
    // leaving a strip of wallpaper with its own rounded frame corner.
    const SNAP: f32 = 48.0;
    if x <= left + radius + SNAP {
        return Ok((left, body + radius, Attachment::LeftFrame));
    }
    if x + width >= screen - right - radius - SNAP {
        return Ok((
            screen - right - body - radius,
            body + radius,
            Attachment::RightFrame,
        ));
    }
    Ok((
        x.round().clamp(left, screen - right - width),
        width,
        Attachment::Bar,
    ))
}

pub(super) fn bottom_panel_anchor(
    edge: crate::settings::BarEdge,
    pid: u32,
    x: f32,
    y: f64,
    width: f32,
    height: f32,
) -> String {
    if edge != crate::settings::BarEdge::Bottom {
        return String::new();
    }
    format!(
        "local scene=hl.plugin and hl.plugin.enzui_sidebar_scene;if scene and type(scene.anchor_panel)==\"function\" and scene.owner_alive({pid}) then assert(scene.anchor_panel(w.address,{pid},{x},{y},{width},{height}),\"Panel anchor rejected\") end;"
    )
}

// The window around a shadowed popover's panel at `x`, `y`, `width` by
// `height`: grown by [`crate::reveal::margins`].
pub(super) fn shadowed(
    x: f32,
    y: f64,
    width: f32,
    height: f32,
    bottom: bool,
    attachment: crate::reveal::Attachment,
) -> (f32, f64, f32, f32) {
    let m = crate::reveal::margins(bottom, attachment);
    (
        x - m.left,
        y - f64::from(m.top),
        width + m.left + m.right,
        height + m.top + m.bottom,
    )
}

pub(super) fn attached_y(
    output: &serde_json::Value,
    height: f32,
    edge: crate::settings::BarEdge,
    metrics: crate::settings::Metrics,
) -> Result<f64, String> {
    control_geometry(output)?;
    Ok(match edge {
        crate::settings::BarEdge::Top => f64::from(metrics.bar_band()),
        crate::settings::BarEdge::Bottom => {
            output["height"]
                .as_f64()
                .expect("control_geometry validated finite output dimensions and scale")
                / output["scale"]
                    .as_f64()
                    .expect("control_geometry validated finite output dimensions and scale")
                - f64::from(metrics.bar_band())
                - f64::from(height)
        }
    })
}

// The toast window height for the leading cards that fit in `available`,
// with the shadow margin around the stack.
pub(super) fn notification_toast_height(heights: &[f32], available: f32) -> Result<f32, String> {
    use crate::notifications::card::{SHADOW_MARGIN, STACK_GAP};
    if !(1..=5).contains(&heights.len())
        || heights
            .iter()
            .any(|h| !h.is_finite() || !(40.0..=512.0).contains(h))
        || !available.is_finite()
    {
        return Err(tr("shell-output-cannot-fit-a-complete-notification-toast").into());
    }
    let mut height = 2.0 * SHADOW_MARGIN - STACK_GAP;
    let mut fitted = 0;
    for card in heights {
        if height + STACK_GAP + card > available {
            break;
        }
        height += STACK_GAP + card;
        fitted += 1;
    }
    if fitted == 0 {
        return Err(tr("shell-output-cannot-fit-a-complete-notification-toast").into());
    }
    Ok(height)
}

/// How many leading cards a toast window of `height` holds.
pub fn fitted_toasts(heights: &[f32], height: f32) -> usize {
    use crate::notifications::card::{SHADOW_MARGIN, STACK_GAP};
    let mut used = 2.0 * SHADOW_MARGIN - STACK_GAP;
    heights
        .iter()
        .take_while(|card| {
            used += STACK_GAP + **card;
            used <= height + 0.5
        })
        .count()
}
pub(super) fn control_geometry(output: &serde_json::Value) -> Result<(f32, f32), String> {
    let width = output["width"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-width"))?;
    let height = output["height"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-height"))?;
    let scale = output["scale"]
        .as_f64()
        .ok_or_else(|| tr("shell-missing-output-scale"))?;
    if !width.is_finite()
        || !height.is_finite()
        || !scale.is_finite()
        || !(0.5..=4.0).contains(&scale)
        || !(320.0..=16384.0).contains(&width)
        || !(240.0..=16384.0).contains(&height)
    {
        return Err(tr("shell-unsupported-control-center-output-geometry").into());
    }
    let width = ((width / scale).round() - 24.0).min(340.0);
    let height = ((height / scale).round() - 60.0).min(520.0);
    if width < 240.0 || height < 160.0 {
        return Err(tr("shell-desktop-output-is-too-small-for-the-control-center").into());
    }
    Ok((width as f32, height as f32))
}

pub(super) fn parse_workspace(response: &str) -> Result<u32, String> {
    let value: serde_json::Value =
        serde_json::from_str(response).map_err(|_| tr("shell-invalid-workspace-response"))?;
    value["id"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| tr("shell-no-regular-workspace-is-active").into())
}

// The main keyboard's active layout code (`US`, `HU`), when more than one
// layout is configured.
pub(super) fn parse_keyboard_layout(response: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(response).ok()?;
    let keyboards = value["keyboards"].as_array()?;
    let keyboard = keyboards
        .iter()
        .find(|k| k["main"].as_bool() == Some(true))
        .or_else(|| keyboards.first())?;
    let layouts: Vec<&str> = keyboard["layout"]
        .as_str()?
        .split(',')
        .filter(|l| !l.is_empty())
        .collect();
    if layouts.len() < 2 {
        return None;
    }
    let index = usize::try_from(keyboard["active_layout_index"].as_u64()?).ok()?;
    let code = layouts.get(index)?;
    code.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then(|| code.chars().take(3).collect::<String>().to_uppercase())
}

// The main keyboard's layouts, the active one and its keymap name.
pub(super) fn parse_keyboard(response: &str) -> Option<super::types::Keyboard> {
    let value: serde_json::Value = serde_json::from_str(response).ok()?;
    let keyboards = value["keyboards"].as_array()?;
    let keyboard = keyboards
        .iter()
        .find(|k| k["main"].as_bool() == Some(true))
        .or_else(|| keyboards.first())?;
    let layouts: Vec<String> = keyboard["layout"]
        .as_str()?
        .split(',')
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 16
                && code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .take(16)
        .map(str::to_owned)
        .collect();
    let active = usize::try_from(keyboard["active_layout_index"].as_u64().unwrap_or(0))
        .ok()?
        .min(layouts.len().saturating_sub(1));
    let keymap = keyboard["active_keymap"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    Some(super::types::Keyboard {
        layouts,
        active,
        keymap,
    })
}
