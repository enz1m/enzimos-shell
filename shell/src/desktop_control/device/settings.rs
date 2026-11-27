use super::{INVALID, Result, UNAVAILABLE};
use crate::{
    default_apps::{self, Kind},
    desktop_preferences::{WallpaperChange, WallpaperKind},
    night_light,
    notifications::preferences,
    settings::{Change, Store},
    shell_style::ShellStyle,
    system_settings::{Request, SystemSettings, display},
};
use serde_json::{Value, json};

fn store() -> Result<Store> {
    Store::from_environment().ok_or(UNAVAILABLE)
}

fn default_kind(setting: &str) -> Option<Kind> {
    match setting {
        "default_browser" => Some(Kind::Web),
        "default_mail" => Some(Kind::Mail),
        "default_files" => Some(Kind::Files),
        _ => None,
    }
}

pub(super) async fn read(setting: &str) -> Result<Value> {
    if let Some(kind) = default_kind(setting) {
        let current = tokio::task::spawn_blocking(move || {
            let choices = default_apps::read("");
            let id = choices.current.get(&kind)?;
            let name = choices.apps.get(&kind)?.iter().find(|app| app.id == *id);
            Some(json!({"id": id, "name": name.map(|app| &app.name)}))
        })
        .await
        .map_err(|_| UNAVAILABLE)?;
        return Ok(json!({"setting": setting, "value": current}));
    }
    let value = match setting {
        "power_mode" => {
            let profiles = SystemSettings::from_environment()
                .power_profiles()
                .await
                .ok_or(UNAVAILABLE)?;
            json!({"current": profiles.active, "available": profiles.available})
        }
        "display_scale" => {
            let displays = tokio::task::spawn_blocking(display::read)
                .await
                .map_err(|_| UNAVAILABLE)?;
            if !displays.supported {
                return Err(UNAVAILABLE);
            }
            Value::Array(
                displays
                    .outputs
                    .iter()
                    .filter(|output| output.enabled)
                    .take(8)
                    .map(|output| json!({"display": output.name, "scale": output.scale}))
                    .collect(),
            )
        }
        _ => {
            let state = store()?.load().map_err(|_| UNAVAILABLE)?;
            match setting {
                "night_light" => json!(state.night_light),
                "wallpaper" => json!({"kind":state.wallpaper.kind,"color":state.wallpaper.color}),
                "shell_style" => json!(state.shell_style.key()),
                "dark_mode" => json!(!state.light),
                "dnd" => json!(state.notifications.policy.dnd),
                "notification_previews" => json!(state.notifications.images),
                "keyboard_layout" => {
                    let configured = state.compositor.layouts();
                    if configured.is_empty() {
                        let (_, system) = crate::system_settings::keyboard_layouts(
                            &SystemSettings::from_environment(),
                        )
                        .await;
                        json!([system.as_deref().unwrap_or("us")])
                    } else {
                        json!(configured)
                    }
                }
                _ => return Err(INVALID),
            }
        }
    };
    Ok(json!({"setting":setting,"value":value}))
}

pub(super) async fn set(setting: &str, value: &str) -> Result<Value> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(INVALID);
    }
    if let Some(kind) = default_kind(setting) {
        let value = value.to_owned();
        let id = tokio::task::spawn_blocking(move || {
            let choices = default_apps::read("");
            let mut matches = choices
                .apps
                .get(&kind)
                .ok_or(INVALID)?
                .iter()
                .filter(|app| {
                    app.id.eq_ignore_ascii_case(&value) || app.name.eq_ignore_ascii_case(&value)
                });
            let app = matches
                .next()
                .filter(|_| matches.next().is_none())
                .ok_or(INVALID)?;
            default_apps::set(kind, &app.id).map_err(|_| UNAVAILABLE)?;
            Ok::<_, &'static str>(app.id.clone())
        })
        .await
        .map_err(|_| UNAVAILABLE)??;
        return Ok(json!({"setting":setting,"value":id}));
    }
    let toggle = || match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(INVALID),
    };
    let change = match setting {
        "night_light" => Change::NightLight(night_light::Change::Enabled(toggle()?)),
        "wallpaper" => Change::Wallpaper(match value {
            "default" => WallpaperChange::Kind(WallpaperKind::Default),
            color
                if color.len() == 7
                    && color.starts_with('#')
                    && color.as_bytes().iter().skip(1).all(u8::is_ascii_hexdigit) =>
            {
                WallpaperChange::Solid(color.to_owned())
            }
            _ => return Err(INVALID),
        }),
        "shell_style" => {
            Change::ShellStyle(ShellStyle::from_json(Some(&json!(value))).map_err(|_| INVALID)?)
        }
        "notification_previews" => Change::Notification(preferences::Change::Images(toggle()?)),
        "keyboard_layout" => {
            let settings = SystemSettings::from_environment();
            let (available, _) = crate::system_settings::keyboard_layouts(&settings).await;
            let layouts: Vec<_> = value.split(',').map(str::trim).map(str::to_owned).collect();
            if !layouts
                .iter()
                .all(|id| available.iter().any(|(code, _)| code == id))
            {
                return Err(INVALID);
            }
            Change::KeyboardLayouts(layouts)
        }
        "power_mode" => {
            SystemSettings::from_environment()
                .apply(&Request::PowerProfile(value.to_owned()))
                .await
                .map_err(|_| UNAVAILABLE)?;
            return Ok(json!({"setting":setting,"value":value}));
        }
        "display_scale" => {
            let (output, scale) = value.split_once('=').ok_or(INVALID)?;
            let scale = scale
                .parse::<f64>()
                .ok()
                .filter(|scale| display::SCALES.contains(scale))
                .ok_or(INVALID)?;
            let output = output.to_owned();
            tokio::task::spawn_blocking(move || set_scale(&output, scale))
                .await
                .map_err(|_| UNAVAILABLE)??;
            return Ok(json!({"setting":setting,"value":value}));
        }
        _ => return Err(INVALID),
    };
    store()?.update(change).map_err(|_| UNAVAILABLE)?;
    Ok(json!({"setting":setting,"value":value}))
}

fn set_scale(output: &str, scale: f64) -> Result<()> {
    let store = store()?;
    let displays = display::read();
    if !displays.supported {
        return Err(UNAVAILABLE);
    }
    let previous = display::layout(&displays);
    let mut layout = previous.clone();
    let changed = layout
        .iter_mut()
        .find(|item| item.enabled && item.output == output)
        .filter(|item| display::supports_scale(item.mode, scale))
        .ok_or(INVALID)?;
    if (changed.scale - scale).abs() < 1e-6 {
        return Ok(());
    }
    changed.scale = scale;
    display::pack(&mut layout, &display::sides(&previous));
    display::apply(&layout).map_err(|_| UNAVAILABLE)?;
    if store
        .update(Change::Displays(display::saved(&displays, &layout)))
        .is_err()
    {
        let _ = display::apply(&previous);
        return Err(UNAVAILABLE);
    }
    Ok(())
}
