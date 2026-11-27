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
                    .map(|output| {
                        let allowed = output
                            .current
                            .and_then(|index| output.modes.get(index))
                            .map(|mode| {
                                display::SCALES
                                    .iter()
                                    .copied()
                                    .filter(|scale| display::supports_scale(*mode, *scale))
                                    .collect::<Vec<_>>()
                            });
                        json!({"display": output.name, "scale": output.scale, "allowed": allowed})
                    })
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

fn preference(id: &str) -> Option<&'static crate::settings_agent::Entry> {
    crate::settings_agent::find(match id {
        "datetime/language" => "desktop.language",
        "appearance/theme" => "appearance.dark_mode",
        "appearance/style" => "shell.style",
        "sizing/corner_radius" => "appearance.corner_radius",
        "sizing/text_size" => "appearance.text_size",
        "shell/bar" => "shell.bar_position",
        "sizing/bar_height" => "shell.bar_height",
        "shell/sidebar_enabled" => "shell.sidebar_enabled",
        "shell/activity_status" => "shell.activity_status",
        "shell/sidebar" => "shell.sidebar_side",
        "windows/tiling" => "windows.tiling",
        "keyboard/input_method" => "keyboard.input_method",
        "display/night_light" | "controls/night_light" => "display.night_light",
        "display/night_light_temperature" => "display.night_light_temperature",
        "datetime/clock_24h" => "clock.24_hour",
        "datetime/timezone" => "system.time_zone",
        "datetime/automatic" => "system.automatic_time",
        "notifications/dnd" | "controls/dnd" => "notifications.do_not_disturb",
        _ => id,
    })
}

async fn describe(
    entry: crate::settings_catalog::Entry,
    state: &crate::settings::Settings,
) -> Value {
    let id = entry.route;
    let backed = preference(id);
    let mut row = json!({"id":id,"label":entry.title(),"section":entry.section(),"value":null,"editable":false});
    if let Some(pref) = backed {
        let clock = if pref.scope == crate::settings_agent::Scope::System
            && let crate::system_settings::Reading::DateTime(clock) =
                SystemSettings::from_environment()
                    .read(crate::system_settings::Panel::DateTime)
                    .await
        {
            Some(clock)
        } else {
            None
        };
        let context = crate::settings_agent::Context {
            clock: clock.as_ref(),
            ..Default::default()
        };
        let details = pref.describe(state, &context, false);
        row["value"] = details["value"].clone();
        row["editable"] = details["available"].clone();
        let allowed = match pref.kind {
            crate::settings_agent::Kind::Boolean => json!([true, false]),
            crate::settings_agent::Kind::Integer { min, max } => json!({"min":min,"max":max}),
            _ => details["choices"].clone(),
        };
        row["allowed"] = allowed;
        if id == "appearance/theme" {
            row["value"] = if state.light {
                json!("light")
            } else {
                json!("dark")
            };
            row["allowed"] = json!(["light", "dark"]);
        }
    } else if let Some(setting) = control(id) {
        row["allowed"] = match setting {
            "brightness" => json!({"min":5,"max":100,"relative":["up","down"]}),
            "volume" => json!({"min":0,"max":100,"relative":["up","down","mute","unmute"]}),
            "display_scale" => json!("output=scale; see each display for allowed scales"),
            "notification_previews" => json!([true, false]),
            "wallpaper" => json!(["default", "#RRGGBB"]),
            _ => Value::Null,
        };
        row["value"] = match setting {
            "brightness" => crate::system_controls::SystemControls::from_environment()
                .brightness()
                .await
                .map_or(Value::Null, |reading| json!(reading.percent)),
            "volume" => crate::system_controls::SystemControls::from_environment()
                .output_audio()
                .await
                .map_or(Value::Null, |reading| json!(reading.percent)),
            _ => read(setting)
                .await
                .map_or(Value::Null, |mut reading| reading["value"].take()),
        };
        row["editable"] = json!(!row["value"].is_null());
        if setting == "brightness" && row["value"].is_null() {
            row["unavailable_reason"] = json!("No controllable screen backlight was found.");
        }
    } else if id == "display/night_light_schedule" {
        row["value"] = json!(state.night_light.scheduled);
        row["allowed"] = json!([true, false]);
        row["editable"] = json!(true);
    }
    row
}

fn control(id: &str) -> Option<&'static str> {
    Some(match id {
        "controls/brightness" => "brightness",
        "controls/volume" | "sound/output_volume" => "volume",
        "display/scale" => "display_scale",
        "wallpaper/source" => "wallpaper",
        "power/mode" => "power_mode",
        "keyboard/layouts" => "keyboard_layout",
        "notifications/images" => "notification_previews",
        _ => return None,
    })
}

pub(super) async fn search(query: &str, offset: usize) -> Result<Value> {
    if query.trim().is_empty() || query.len() > 640 || offset > 1000 {
        return Err(INVALID);
    }
    let state = store()?.load().map_err(|_| UNAVAILABLE)?;
    let index = |query: &str| crate::settings_catalog::search_groups(query, |_, _| true);
    let mut groups = index(query);
    if groups.is_empty() {
        let words = query.split_whitespace().take(8).collect::<Vec<_>>();
        for length in (1..words.len()).rev() {
            groups = words
                .windows(length)
                .map(|words| index(&words.join(" ")))
                .find(|matches| !matches.is_empty())
                .unwrap_or_default();
            if !groups.is_empty() {
                break;
            }
        }
    }
    let limit = 4;
    let mut seen = std::collections::HashSet::new();
    let mut entries = groups
        .into_iter()
        .flat_map(|group| group.controls)
        .filter(|entry| seen.insert(entry.route))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        preference(entry.route).is_none()
            && control(entry.route).is_none()
            && entry.route != "display/night_light_schedule"
    });
    let mut rows = Vec::new();
    for entry in entries.iter().skip(offset).take(limit) {
        rows.push(describe(*entry, &state).await);
    }
    Ok(
        json!({"settings":rows,"next_offset":(entries.len() > offset + limit).then_some(offset + limit)}),
    )
}

pub(super) async fn get(id: &str) -> Result<Value> {
    let entry = crate::settings_catalog::find(id).ok_or(INVALID)?;
    let state = store()?.load().map_err(|_| UNAVAILABLE)?;
    Ok(describe(entry, &state).await)
}

pub(super) async fn change(
    compositor: &crate::compositor::Compositor,
    id: &str,
    value: &Value,
) -> Result<Value> {
    let entry = crate::settings_catalog::find(id).ok_or(INVALID)?;
    let id = entry.route;
    if let Some(pref) = preference(id) {
        let context = crate::settings_agent::Context {
            islands_supported: id == "appearance/style" && compositor.islands_supported().await,
            ..Default::default()
        };
        let coerced = match (id, value.as_str()) {
            ("appearance/theme", Some("dark")) => Some(json!(true)),
            ("appearance/theme", Some("light")) => Some(json!(false)),
            (_, Some("true" | "on")) if pref.kind == crate::settings_agent::Kind::Boolean => {
                Some(json!(true))
            }
            (_, Some("false" | "off")) if pref.kind == crate::settings_agent::Kind::Boolean => {
                Some(json!(false))
            }
            _ => None,
        };
        let target = pref
            .target(coerced.as_ref().unwrap_or(value), &context)
            .map_err(|error| error.code)?;
        if let crate::settings_agent::Target::User(change) = target {
            store()?.update(change).map_err(|_| UNAVAILABLE)?;
        } else {
            return Err(UNAVAILABLE);
        }
    } else if id == "display/night_light_schedule" {
        let scheduled = value.as_bool().ok_or(INVALID)?;
        store()?
            .update(Change::NightLight(night_light::Change::Scheduled(
                scheduled,
            )))
            .map_err(|_| UNAVAILABLE)?;
    } else if let Some(setting) = control(id) {
        let typed;
        let value = match value {
            Value::String(text) => text,
            Value::Bool(enabled) if setting == "notification_previews" => {
                if *enabled {
                    "on"
                } else {
                    "off"
                }
            }
            Value::Number(number) if matches!(setting, "brightness" | "volume") => {
                typed = number.to_string();
                &typed
            }
            _ => return Err(INVALID),
        };
        if matches!(setting, "brightness" | "volume") {
            super::set(super::Setting {
                setting: setting.to_owned(),
                value: value.to_owned(),
            })
            .await?;
        } else {
            set(setting, value).await?;
        }
    } else {
        return Err(UNAVAILABLE);
    }
    get(id).await
}
