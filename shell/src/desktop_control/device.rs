use crate::{
    chooser_places,
    compositor::Compositor,
    network_connectivity, notifications,
    search::{AppTarget, Target, catalogue},
    settings::{Change, Store},
    system_controls::{Action, SystemControls},
};
use chrono::{Days, Local, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;

mod settings;

type Result<T> = std::result::Result<T, &'static str>;

const UNAVAILABLE: &str = "unavailable";
const INVALID: &str = "invalid";

#[derive(Deserialize)]
#[serde(tag = "tool", content = "arguments", deny_unknown_fields)]
enum Request {
    #[serde(rename = "desktop_setting")]
    Setting(Setting),
    #[serde(rename = "desktop_read_setting")]
    ReadSetting(ReadSetting),
    #[serde(rename = "desktop_open")]
    Open(Open),
    #[serde(rename = "desktop_timer")]
    Timer(Timer),
    #[serde(rename = "device_status")]
    Status(Status),
    #[serde(rename = "find_downloads")]
    Downloads(Downloads),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setting {
    setting: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadSetting {
    setting: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    app: String,
    folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Timer {
    seconds: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Status {
    metric: Metric,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Metric {
    Battery,
    Cpu,
    Memory,
    Network,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Downloads {
    date: String,
    extension: Option<String>,
}

pub(super) async fn execute(
    compositor: &Compositor,
    root: &Path,
    timers: Arc<Semaphore>,
    mut frame: Value,
) -> Result<Value> {
    frame.as_object_mut().ok_or(INVALID)?.remove("method");
    let request: Request = serde_json::from_value(frame).map_err(|_| INVALID)?;
    if crate::session_lock::locking()
        || !compositor
            .request(&["locked"])
            .await
            .is_ok_and(|value| value.trim() == "false")
    {
        return Err("locked");
    }

    match request {
        Request::Setting(arguments) => set(arguments).await,
        Request::ReadSetting(ReadSetting { setting }) => settings::read(&setting).await,
        Request::Open(arguments) => open(compositor, root, arguments).await,
        Request::Timer(Timer { seconds }) => start_timer(timers, seconds),
        Request::Status(Status { metric }) => status(metric).await,
        Request::Downloads(arguments) => downloads(arguments).await,
    }
}

async fn set(Setting { setting, value }: Setting) -> Result<Value> {
    let controls = SystemControls::from_environment();
    match setting.as_str() {
        "dark_mode" | "dnd" => {
            let enabled = match value.as_str() {
                "on" => true,
                "off" => false,
                _ => return Err(INVALID),
            };
            let change = if setting == "dark_mode" {
                Change::Light(!enabled)
            } else {
                Change::Notification(notifications::preferences::Change::Dnd(enabled))
            };
            Store::from_environment()
                .ok_or(UNAVAILABLE)?
                .update(change)
                .map_err(|_| UNAVAILABLE)?;
            Ok(json!({"setting":setting, "enabled":enabled}))
        }
        "brightness" | "volume" => {
            let brightness = setting == "brightness";
            if brightness && controls.brightness().await.is_none() {
                return Err("no_backlight");
            }
            if crate::desktop_session::mode().nested() {
                return Err(UNAVAILABLE);
            }
            if !brightness && matches!(value.as_str(), "mute" | "unmute") {
                let muted = value == "mute";
                controls
                    .apply(&Action::Mute(muted))
                    .await
                    .map_err(|_| UNAVAILABLE)?;
                return Ok(json!({"setting":setting,"muted":muted}));
            }
            let floor = if brightness { 5 } else { 0 };
            let percent = match value.as_str() {
                "up" | "down" => {
                    let current = if brightness {
                        controls.brightness().await.map(|reading| reading.percent)
                    } else {
                        controls.output_audio().await.map(|reading| reading.percent)
                    }
                    .ok_or(UNAVAILABLE)?;
                    if value == "up" {
                        current.saturating_add(10).min(100)
                    } else {
                        current.saturating_sub(10).max(floor)
                    }
                }
                _ => value
                    .parse::<u8>()
                    .ok()
                    .filter(|level| (floor..=100).contains(level))
                    .ok_or(INVALID)?,
            };
            let action = if brightness {
                Action::Brightness(percent)
            } else {
                Action::Volume(percent)
            };
            controls.apply(&action).await.map_err(|_| UNAVAILABLE)?;
            Ok(json!({"setting":setting, "percent":percent}))
        }
        _ => settings::set(&setting, &value).await,
    }
}

fn user_folder(name: &str) -> Option<PathBuf> {
    chooser_places::user_folders()
        .into_iter()
        .find(|place| {
            place.label.eq_ignore_ascii_case(name)
                || place
                    .path
                    .file_name()
                    .is_some_and(|base| base.eq_ignore_ascii_case(name))
                || name.eq_ignore_ascii_case("Downloads") && place.icon == "download"
        })
        .map(|place| place.path)
}

async fn open(compositor: &Compositor, root: &Path, Open { app, folder }: Open) -> Result<Value> {
    if app.trim().is_empty() || app.len() > 120 || app.chars().any(char::is_control) {
        return Err(INVALID);
    }
    if let Some(folder) = folder {
        if !app.eq_ignore_ascii_case("Files")
            || folder.is_empty()
            || folder.len() > 1024
            || folder.chars().any(char::is_control)
        {
            return Err(INVALID);
        }
        let path = PathBuf::from(&folder);
        let path = if path.is_absolute() {
            path
        } else {
            user_folder(&folder).ok_or("folder")?
        };
        compositor
            .open_dock_folder(root, &path)
            .await
            .map_err(|_| "folder")?;
        return Ok(json!({"opened":"Files", "folder":path}));
    }

    let root_owned = root.to_path_buf();
    let (name, target) = tokio::task::spawn_blocking(move || {
        let items = catalogue::scan(&root_owned, &catalogue::directories());
        let mut candidates = items.into_iter().filter_map(|item| {
            let Target::App(target) = item.target? else {
                return None;
            };
            let id_matches = match &target {
                AppTarget::Prepared(id) => id.eq_ignore_ascii_case(&app),
                AppTarget::Desktop { path, .. } => path
                    .file_stem()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&app)),
            };
            if id_matches || item.title.eq_ignore_ascii_case(&app) {
                Some((item.title, target))
            } else {
                None
            }
        });
        let match_ = candidates.next()?;
        candidates.next().is_none().then_some(match_)
    })
    .await
    .map_err(|_| UNAVAILABLE)?
    .ok_or("app")?;
    compositor
        .execute_search_target(root, Target::App(target))
        .await
        .map_err(|_| UNAVAILABLE)?;
    Ok(json!({"opened":name}))
}

fn start_timer(timers: Arc<Semaphore>, seconds: u32) -> Result<Value> {
    if !(1..=86_400).contains(&seconds) {
        return Err(INVALID);
    }
    let permit = timers.try_acquire_owned().map_err(|_| "limit")?;
    tokio::spawn(async move {
        let _permit = permit;
        tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
        let summary = crate::i18n::tr("shell-assistant-timer-finished").to_owned();
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            crate::battery_warnings::post_notification(
                "Assistant",
                "alarm-clock",
                "alarm",
                summary,
                String::new(),
                true,
            ),
        )
        .await;
    });
    Ok(json!({"seconds": seconds, "started": true}))
}

async fn status(metric: Metric) -> Result<Value> {
    match metric {
        Metric::Battery => {
            let controls = SystemControls::from_environment();
            let battery = if let Some((battery, _)) = crate::battery_warnings::reading().await {
                Some(battery)
            } else {
                controls.battery().await
            };
            Ok(battery.map_or_else(
                || json!({"available":false,"status":"No battery reading is available"}),
                |battery| json!({"available":true,"percent":battery.percent,"charging":battery.charging,"external_power":battery.external_power}),
            ))
        }
        Metric::Cpu => tokio::task::spawn_blocking(cpu)
            .await
            .map_err(|_| UNAVAILABLE)?,
        Metric::Memory => {
            let mut system = sysinfo::System::new();
            system.refresh_memory();
            let total = system.total_memory();
            if total == 0 {
                return Err(UNAVAILABLE);
            }
            let used = system.used_memory();
            Ok(
                json!({"used_mib":used / (1 << 20), "total_mib":total / (1 << 20),
                "percent":used.saturating_mul(100) / total}),
            )
        }
        Metric::Network => {
            let system = crate::bus::system().await.ok_or(UNAVAILABLE)?;
            let reading = network_connectivity::current(&system)
                .await
                .ok_or(UNAVAILABLE)?;
            Ok(json!({
                "internet":reading.state.label(),
                "ethernet":reading.devices.as_ref().map(|devices| devices.ethernet),
                "wifi":reading.devices.map(|devices| devices.wifi_connected),
            }))
        }
    }
}

fn cpu() -> Result<Value> {
    let mut system = sysinfo::System::new();
    let refresh = sysinfo::ProcessRefreshKind::nothing().with_cpu();
    let sample = |system: &mut sysinfo::System| {
        system.refresh_cpu_usage();
        system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::All, true, refresh);
    };
    sample(&mut system);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sample(&mut system);
    if system.cpus().is_empty() {
        return Err(UNAVAILABLE);
    }
    let mut processes: Vec<_> = system
        .processes()
        .values()
        .filter(|proc| proc.cpu_usage() > 0.0)
        .collect();
    processes.sort_unstable_by(|a, b| b.cpu_usage().total_cmp(&a.cpu_usage()));
    let busiest = processes
        .into_iter()
        .take(3)
        .map(|process| {
            json!({
                "process":catalogue::display_text(&process.name().to_string_lossy(), 48),
                "cpu_percent_of_one_core":process.cpu_usage().round(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"cpu_percent":system.global_cpu_usage().round(), "busiest":busiest}))
}

async fn downloads(Downloads { date, extension }: Downloads) -> Result<Value> {
    let today = Local::now().date_naive();
    let date = match date.as_str() {
        "today" => Some(today),
        "yesterday" => today.checked_sub_days(Days::new(1)),
        value if value.len() == 10 => NaiveDate::parse_from_str(value, "%Y-%m-%d").ok(),
        _ => None,
    }
    .ok_or(INVALID)?;
    if extension.as_ref().is_some_and(|value| {
        !(1..=16).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        return Err(INVALID);
    }
    let directory = user_folder("Downloads").ok_or("folder")?;
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .map_err(|_| UNAVAILABLE)?;
    let mut matches = Vec::new();
    let (mut scanned, mut matching) = (0, 0);
    while scanned < 4096 {
        let Some(entry) = entries.next_entry().await.map_err(|_| UNAVAILABLE)? else {
            break;
        };
        scanned += 1;
        let name = entry.file_name();
        let Some(name) = name
            .to_str()
            .filter(|name| !name.chars().any(char::is_control))
        else {
            continue;
        };
        if extension.as_ref().is_some_and(|extension| {
            !Path::new(name)
                .extension()
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(extension))
        }) {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if chrono::DateTime::<Local>::from(modified).date_naive() == date {
            matching += 1;
            if matches.len() < 6 {
                matches.push(name.to_owned());
            }
        }
    }
    matches.sort_unstable();
    Ok(
        json!({"date":date.to_string(),"files":matches,"truncated":scanned == 4096 || matching > matches.len()}),
    )
}
