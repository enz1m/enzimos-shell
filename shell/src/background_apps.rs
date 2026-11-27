use enzui_shell::i18n::tr;
use enzui_shell::tr;
mod client;

pub use client::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: usize = 1024 * 1024;
const INVALID: &str = "The background app service sent an unsupported answer.";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    schema: u64,
    #[serde(default)]
    pub registered_native_units: bool,
    #[serde(default)]
    pub graceful_lifecycle_adapter: bool,
    #[serde(default)]
    pub flatpak_background_adapter: bool,
    #[serde(default)]
    pub agent_policy_adapter: bool,
    /// `ActivateForSearch` may start an explicitly permitted provider.
    #[serde(default)]
    pub search_policy_adapter: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    Unknown,
    Windowless,
    HasWindows,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct App {
    schema: u64,
    pub app_id: String,
    pub title: String,
    pub purpose: String,
    pub running: bool,
    pub unit_observation_known: bool,
    pub visibility: Visibility,
    pub window_count: Option<u32>,
    pub minimized_window_count: Option<u32>,
    pub work_observation_known: bool,
    pub work: Vec<Value>,
    pub activation_policy: BTreeMap<String, bool>,
    pub autostart_supported: bool,
    pub autostart_state: Option<String>,
    pub graceful_stop_supported: bool,
    pub force_stop_supported: bool,
    pub identity_verified: bool,
}

impl App {
    pub fn work_status(&self) -> String {
        if !self.work_observation_known {
            tr("shell-current-work-unknown").into()
        } else if self.work.is_empty() {
            tr("shell-no-work-reported").into()
        } else {
            tr!("shell-work-items-reported", "arg0" => format!("{:}", self.work.len()))
        }
    }

    pub fn status(&self) -> String {
        if !self.unit_observation_known {
            return tr("shell-running-state-unknown").into();
        }
        if !self.running {
            return tr("shell-not-running").into();
        }
        match self.visibility {
            Visibility::Unknown => tr("shell-running-window-state-unknown").into(),
            Visibility::Windowless => tr("shell-running-without-windows").into(),
            Visibility::HasWindows => {
                let windows = self.window_count.unwrap_or(0);
                let minimized = self.minimized_window_count.unwrap_or(0);
                tr!("background-running", "windows" => windows, "minimized" => minimized)
            }
        }
    }

    pub fn can_stop(&self, mode: StopMode) -> bool {
        self.running
            && self.unit_observation_known
            && self.identity_verified
            && if mode == StopMode::Force {
                self.force_stop_supported
            } else {
                self.graceful_stop_supported
            }
    }

    fn validate(&self) -> bool {
        let windows = match self.visibility {
            Visibility::Unknown => {
                self.window_count.is_none() && self.minimized_window_count.is_none()
            }
            Visibility::Windowless => {
                self.window_count == Some(0) && self.minimized_window_count == Some(0)
            }
            Visibility::HasWindows => {
                self.window_count.is_some_and(|n| n > 0 && n <= 256)
                    && self
                        .minimized_window_count
                        .zip(self.window_count)
                        .is_some_and(|(m, n)| m <= n)
            }
        };
        self.schema == 1
            && app_id(&self.app_id)
            && ![
                "dev.enzui.Desktop",
                "dev.enzui.Compositor",
                "org.enzimos.Background",
                "org.enzimos.Activity",
                "org.enzui.McpCenter",
                "org.enzui.Desktop",
                "org.enzui.Shell",
                "org.enzui.Compositor",
                "org.enzimos.Background1",
                "org.enzimos.Activity1",
            ]
            .contains(&self.app_id.as_str())
            && text(&self.title, 160)
            && text(&self.purpose, 160)
            && windows
            && policy(&self.activation_policy)
            && self.work.len() <= 128
            && (!self.graceful_stop_supported && !self.force_stop_supported
                || self.running && self.unit_observation_known && self.identity_verified)
            && self.autostart_state.as_ref().is_none_or(|s| text(s, 32))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PortalApp {
    pub app_id: String,
    pub instance: String,
    pub message: String,
    // Whether the instance process itself still runs; `None` when the
    // service cannot observe it (or predates the field).
    #[serde(default)]
    pub instance_running: Option<bool>,
}

impl PortalApp {
    fn validate(&self) -> bool {
        portal_id(&self.app_id)
            && (1..=20).contains(&self.instance.len())
            && self.instance.bytes().all(|b| b.is_ascii_digit())
            && self.message.chars().count() <= 96
            && self.message.chars().all(printable)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum PolicyPhase {
    Pending,
    Complete,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PolicyOperation {
    phase: PolicyPhase,
    app_id: String,
    automatic_replay: bool,
    fresh_review_required: bool,
}

impl PolicyOperation {
    fn valid(&self) -> bool {
        app_id(&self.app_id)
            && !self.automatic_replay
            && self.fresh_review_required == (self.phase != PolicyPhase::Complete)
    }
    pub fn pending(&self) -> bool {
        self.phase == PolicyPhase::Pending
    }
    pub fn uncertain(&self) -> bool {
        self.phase == PolicyPhase::Uncertain
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    schema: u64,
    pub generation: u64,
    pub apps: Vec<App>,
    pub catalogue_observation_known: bool,
    pub flatpak_background: Option<Vec<PortalApp>>,
    #[serde(default)]
    pub unavailable_declarations: Vec<Value>,
    #[serde(default)]
    pub policy_operation: Option<PolicyOperation>,
}

impl Snapshot {
    pub fn same_observation(&self, other: &Self) -> bool {
        self.apps == other.apps
            && self.catalogue_observation_known == other.catalogue_observation_known
            && self.flatpak_background == other.flatpak_background
            && self.unavailable_declarations == other.unavailable_declarations
            && self.policy_operation == other.policy_operation
    }

    pub fn decode(bytes: &str) -> Result<Self, String> {
        let result: Self = decode(bytes)?;
        let mut ids = BTreeSet::new();
        let mut instances = BTreeSet::new();
        if result
            .policy_operation
            .as_ref()
            .is_some_and(|operation| !operation.valid())
            || result.schema != 1
            || result.apps.len() > 128
            || result.unavailable_declarations.len() > 128
            || !result
                .apps
                .iter()
                .all(|a| a.validate() && ids.insert(&a.app_id))
            || !result.flatpak_background.as_ref().is_none_or(|apps| {
                apps.len() <= 256
                    && apps
                        .iter()
                        .all(|a| a.validate() && instances.insert(&a.instance))
            })
        {
            return Err(INVALID.into());
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    Finish,
    Cancel,
    Force,
}

impl StopMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Finish => "finish-then-exit",
            Self::Cancel => "cancel-and-exit",
            Self::Force => "force",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Finish => tr("shell-finish-work-and-quit"),
            Self::Cancel => tr("shell-cancel-work-and-quit"),
            Self::Force => tr("shell-force-quit"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Stop {
        app: App,
        mode: StopMode,
    },
    Portal(PortalApp),
    Policy {
        app: App,
        requested: BTreeMap<String, bool>,
    },
}

impl Action {
    pub fn app_id(&self) -> &str {
        match self {
            Self::Stop { app, .. } | Self::Policy { app, .. } => &app.app_id,
            Self::Portal(app) => &app.app_id,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Self::Stop { app, .. } | Self::Policy { app, .. } => &app.title,
            Self::Portal(app) => &app.app_id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Review {
    pub action: Action,
    pub digest: Option<String>,
    pub plan: Value,
}

impl Review {
    fn decode(bytes: &str, action: Action) -> Result<Self, String> {
        let value: Value = decode(bytes)?;
        let plan = &value["plan"];
        let digest = value["plan_sha256"]
            .as_str()
            .filter(|s| hex(s))
            .ok_or(INVALID)?
            .to_owned();
        if plan["schema"] != 1 || plan["app_id"].as_str() != Some(action.app_id()) {
            return Err(INVALID.into());
        }
        match &action {
            Action::Stop { mode, .. } => {
                let app_id = action.app_id();
                let unit = plan["unit"].as_str().unwrap_or_default();
                if unit != format!("enzui-background-{app_id}.service")
                    && unit != format!("enzui-agent-{app_id}.service")
                {
                    return Err(INVALID.into());
                }
                if plan["mode"].as_str() != Some(mode.key())
                    || plan["main_pid"]
                        .as_u64()
                        .is_none_or(|pid| pid == 0 || pid > u32::MAX.into())
                    || plan["start_ticks"].as_u64().is_none_or(|start| start == 0)
                    || plan["future_activation_policy_changed"] != false
                    || plan["personal_files_removed"] != false
                    || !plan["may_interrupt_work"].is_boolean()
                    || !plan["may_close_windows"].is_boolean()
                    || !plan["manifest_sha256"].as_str().is_some_and(hex)
                    || !plan["observation_sha256"].as_str().is_some_and(hex)
                {
                    return Err(INVALID.into());
                }
                if *mode != StopMode::Force {
                    let lifecycle = &plan["lifecycle"];
                    let valid = plan["choice"].as_str() == Some(mode.key())
                        && lifecycle["main_pid"] == plan["main_pid"]
                        && lifecycle["start_ticks"] == plan["start_ticks"]
                        && lifecycle["owner"]
                            .as_str()
                            .is_some_and(|name| zbus::names::UniqueName::try_from(name).is_ok())
                        && lifecycle["revision"].as_str().is_some_and(|s| {
                            !s.is_empty()
                                && s.len() <= 80
                                && s.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                        })
                        && lifecycle["consequences"].as_array().is_some_and(|rows| {
                            rows.len() <= 32
                                && rows.iter().all(|row| {
                                    row["kind"].as_str().is_some_and(|s| text(s, 160))
                                        && row["title"].as_str().is_some_and(|s| text(s, 160))
                                        && row["detail"]
                                            .as_str()
                                            .is_some_and(|s| s.is_empty() || text(s, 160))
                                })
                        });
                    if !valid {
                        return Err(INVALID.into());
                    }
                }
            }
            Action::Policy { app, requested } => {
                if !policy(requested)
                    || plan["requested"] != serde_json::to_value(requested).map_err(|_| INVALID)?
                    || plan["previous"]
                        != serde_json::to_value(&app.activation_policy).map_err(|_| INVALID)?
                    || plan["stops_ongoing_work"] != false
                    || plan["grants_agent_tool_consent"] != false
                    || !plan["manifest_sha256"].as_str().is_some_and(hex)
                {
                    return Err(INVALID.into());
                }
            }
            Action::Portal(_) => return Err(INVALID.into()),
        }
        Ok(Self {
            action,
            digest: Some(digest),
            plan: plan.clone(),
        })
    }
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &str) -> Result<T, String> {
    if bytes.len() > LIMIT {
        return Err(INVALID.into());
    }
    serde_json::from_str(bytes).map_err(|_| INVALID.into())
}

fn printable(c: char) -> bool {
    !c.is_control()
        && !matches!(c as u32, 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfeff)
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && value.chars().all(printable)
}
fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn app_id(value: &str) -> bool {
    value.len() <= 127
        && (2..=8).contains(&value.split('.').count())
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphabetic()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}
fn portal_id(value: &str) -> bool {
    value.len() <= 255
        && value.split('.').count() >= 3
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}
fn policy(value: &BTreeMap<String, bool>) -> bool {
    (1..=6).contains(&value.len())
        && value.get("user") != Some(&false)
        && value.keys().all(|k| {
            [
                "user",
                "autostart",
                "agent",
                "scheduled",
                "ongoing-work",
                "search",
            ]
            .contains(&k.as_str())
        })
}
