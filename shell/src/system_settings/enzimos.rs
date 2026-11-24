//! JSON-metadata client for org.enzimos.System1 reviewed jobs.
//! Recovery export owns a sealed descriptor; uncertain outcomes never trigger automatic retry.

use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const NAME: &str = "org.enzimos.System1";
const PATH: &str = "/org/enzimos/System1";
const REFUSED: &str = "org.enzimos.System1.Refused";

/// Why a backend call did not give a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The backend refused with one of its public codes.
    Code(String),
    /// The job ended without a known outcome: the system may have changed.
    Uncertain,
    /// The job was cancelled before it started.
    Cancelled,
    /// No backend on this system, or it could not be reached.
    Unavailable,
    /// Observation timed out while the job remained running; its outcome is unknown.
    StillRunning,
    /// Planning failed before mutation; unlike an uncertain job, the system is known unchanged.
    NotPrepared,
    Invalid,
}
impl Refusal {
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Self::Uncertain | Self::StillRunning)
            || matches!(self, Self::Code(c) if c == "backend-result-unknown-do-not-retry-automatically")
    }
    pub fn message(&self) -> String {
        match self {
            Self::Code(code) => code_message(code).into(),
            Self::Uncertain => tr("shell-enzimos-could-not-confirm-whether-the-change-was-made-check-the-current-stat").into(),
            Self::Cancelled => tr("shell-the-change-was-cancelled").into(),
            Self::Unavailable => tr("shell-the-enzimos-system-service-is-not-available").into(),
            Self::StillRunning => tr("shell-the-change-is-still-running-check-again-in-a-while").into(),
            Self::NotPrepared => tr("shell-the-system-could-not-prepare-this-change-nothing-was-changed").into(),
            Self::Invalid => tr("shell-the-enzimos-system-service-sent-an-answer-settings-does-not-understand").into(),
        }
    }
}

/// A friendly sentence for each public refusal code.
pub fn code_message(code: &str) -> &'static str {
    match code {
        "not-authorized" => tr("shell-the-change-was-not-authorized-2"),
        "backup-destination-busy" => tr("shell-the-backup-drive-is-busy-try-again-shortly"),
        "backup-destination-not-configured" => tr("shell-backups-need-a-drive"),
        "backup-attempt-needs-restart" => tr("shell-backup-needs-restart"),
        "backup-attempt-capacity" => tr("shell-backup-attempts-full"),
        "backup-destination-absent" => {
            tr("shell-the-backup-drive-is-not-connected-connect-it-and-try-again")
        }
        "service-busy" | "job-store-busy" => {
            tr("shell-enzimos-is-busy-with-another-change-try-again-in-a-moment")
        }
        "job-plan-changed" => {
            tr("shell-the-system-changed-while-this-was-being-prepared-check-the-current-state-and")
        }
        "job-plan-invalid" | "request-fields" | "request-version" => {
            tr("shell-settings-asked-for-something-the-system-service-does-not-accept")
        }
        "operation-unavailable" => tr("shell-this-change-is-not-available-on-this-system"),
        "job-not-found" | "job-owner-mismatch" | "job-transition-refused" => {
            tr("shell-the-change-could-not-be-followed-check-the-current-state")
        }
        "backend-result-unknown-do-not-retry-automatically" => {
            tr("shell-enzimos-could-not-confirm-whether-the-change-was-made-check-the-current-stat")
        }
        _ => tr("shell-the-enzimos-system-service-refused-the-change"),
    }
}

/// A fresh job identifier: 32 lowercase hex digits.
pub fn request_id() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !filled {
        // Unique enough to tell jobs apart; the backend binds them to us anyway.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes = (now ^ (u128::from(std::process::id()) << 64)).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// Selection-based operations carry their selection on every step; unselected operations omit
// it.
fn operation_body(
    schema: u32,
    operation: &str,
    action: &str,
    request_id: &str,
    plan: Option<&str>,
    selection: Option<&Value>,
) -> String {
    let mut body = json!({
        "schema": schema,
        "operation": operation,
        "action": action,
        "request_id": request_id,
        "plan_sha256": plan,
        "allow_interaction": true,
    });
    if let Some(selection) = selection {
        body["selection"] = if action == "plan" {
            selection.clone()
        } else {
            Value::Null
        };
    }
    body.to_string()
}

#[derive(Clone, Debug)]
pub struct Backend {
    connection: zbus::Connection,
}

impl Backend {
    /// Connects if this system has the backend; activation starts it.
    pub async fn connect() -> Option<Self> {
        let connection = zbus::Connection::system().await.ok()?;
        let backend = Self { connection };
        backend.capabilities().await.ok()?;
        Some(backend)
    }
    async fn call(&self, member: &str, body: Option<String>) -> Result<Value, Refusal> {
        let reply = match body {
            Some(body) => {
                self.connection
                    .call_method(Some(NAME), PATH, Some(NAME), member, &(body,))
                    .await
            }
            None => {
                self.connection
                    .call_method(Some(NAME), PATH, Some(NAME), member, &())
                    .await
            }
        };
        let reply = reply.map_err(|error| match error {
            zbus::Error::MethodError(name, message, _) if name.as_str() == REFUSED => {
                Refusal::Code(message.unwrap_or_default())
            }
            _ => Refusal::Unavailable,
        })?;
        let text: String = reply.body().deserialize().map_err(|_| Refusal::Invalid)?;
        serde_json::from_str(&text).map_err(|_| Refusal::Invalid)
    }
    pub async fn backup_setup_client(&self) -> Result<super::backup_setup::Client, String> {
        super::backup_setup::Client::connect(self.connection.clone()).await
    }
    pub async fn recovery_export(&self) -> Result<(Value, std::os::fd::OwnedFd), Refusal> {
        let body = json!({"schema":1,"allow_interaction":true}).to_string();
        let reply = self
            .connection
            .call_method(
                Some(NAME),
                PATH,
                Some(NAME),
                "BackupRecoveryExport",
                &(body,),
            )
            .await
            .map_err(|error| match error {
                zbus::Error::MethodError(name, message, _) if name.as_str() == REFUSED => {
                    Refusal::Code(message.unwrap_or_default())
                }
                _ => Refusal::Unavailable,
            })?;
        let (text, fd): (String, zbus::zvariant::OwnedFd) =
            reply.body().deserialize().map_err(|_| Refusal::Invalid)?;
        let metadata: Value = serde_json::from_str(&text).map_err(|_| Refusal::Invalid)?;
        if metadata["status"] != "backup-recovery-exported" || metadata["sealed_descriptor"] != true
        {
            return Err(Refusal::Invalid);
        }
        Ok((metadata, fd.into()))
    }
    pub async fn recovery_confirm(&self, ticket: String) -> Result<(), Refusal> {
        let result = self.call("BackupRecoveryConfirm", Some(json!({"schema":1,"ticket":ticket,"assertion":"saved-outside-this-computer","allow_interaction":true}).to_string())).await?;
        if result["status"] != "backup-recovery-copy-attested"
            || result["automatic_backup_enabled"] != false
        {
            return Err(Refusal::Invalid);
        }
        Ok(())
    }
    pub async fn capabilities(&self) -> Result<Value, Refusal> {
        self.call("GetCapabilities", None).await
    }
    /// A read-only status query; returns its `result`.
    /// The queries this backend release answers.
    pub async fn queries(&self) -> Vec<String> {
        self.capabilities()
            .await
            .ok()
            .and_then(|caps| caps.get("queries").and_then(Value::as_array).cloned())
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub async fn query(&self, name: &str) -> Result<Value, Refusal> {
        let reply = self
            .call(
                "Query",
                Some(json!({"schema": 1, "query": name}).to_string()),
            )
            .await?;
        reply.get("result").cloned().ok_or(Refusal::Invalid)
    }
    /// The operations this backend release runs.
    pub async fn operations(&self) -> Vec<String> {
        self.capabilities()
            .await
            .ok()
            .and_then(|caps| caps.get("operations").and_then(Value::as_array).cloned())
            .map(|names| {
                names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Runs one change to its end and returns the job's `result`.
    pub async fn run(
        &self,
        schema: u32,
        operation: &str,
        selection: Option<Value>,
        patience: Duration,
    ) -> Result<Value, Refusal> {
        let planned = self.plan(schema, operation, selection).await?;
        self.finish(&planned, patience).await
    }
    /// Plans one change without asking for approval; the reply carries any
    /// review the backend offers.
    pub async fn plan(
        &self,
        schema: u32,
        operation: &str,
        selection: Option<Value>,
    ) -> Result<Planned, Refusal> {
        let id = request_id();
        let reply = match self
            .call(
                "Operation",
                Some(operation_body(
                    schema,
                    operation,
                    "plan",
                    &id,
                    None,
                    selection.as_ref(),
                )),
            )
            .await
        {
            Err(Refusal::Code(code))
                if code == "backend-result-unknown-do-not-retry-automatically" =>
            {
                return Err(Refusal::NotPrepared);
            }
            other => other?,
        };
        let digest = reply
            .get("plan_sha256")
            .and_then(Value::as_str)
            .filter(|d| d.len() == 64)
            .ok_or(Refusal::Invalid)?
            .to_owned();
        Ok(Planned {
            schema,
            operation: operation.to_owned(),
            id,
            digest,
            selection,
            reply,
        })
    }
    /// Submits a plan (the one step that asks an administrator) and follows
    /// it to its end.
    pub async fn finish(&self, planned: &Planned, patience: Duration) -> Result<Value, Refusal> {
        let step = |action: &str, plan: Option<&str>| {
            operation_body(
                planned.schema,
                &planned.operation,
                action,
                &planned.id,
                plan,
                planned.selection.as_ref(),
            )
        };
        let mut state = self
            .call("Operation", Some(step("submit", Some(&planned.digest))))
            .await?;
        let deadline = Instant::now() + patience;
        let mut wait = Duration::from_secs(1);
        loop {
            match state.get("state").and_then(Value::as_str) {
                Some("completed") => {
                    return Ok(state.get("result").cloned().unwrap_or(Value::Null));
                }
                Some("cancelled") => return Err(Refusal::Cancelled),
                Some("uncertain") => return Err(Refusal::Uncertain),
                Some(
                    "queued" | "running" | "planned" | "capturing" | "transferring" | "finishing"
                    | "validating",
                ) => {}
                _ => return Err(Refusal::Invalid),
            }
            if Instant::now() >= deadline {
                return Err(Refusal::StillRunning);
            }
            tokio::time::sleep(wait).await;
            // A status call that fails leaves the outcome unknown.
            state = match self.call("Operation", Some(step("status", None))).await {
                Ok(state) => {
                    wait = Duration::from_secs(1);
                    state
                }
                // A busy or locked backend may withhold status while the job continues; keep
                // observing.
                Err(Refusal::Code(code)) if status_withheld(&code) => {
                    wait = Duration::from_secs(10);
                    continue;
                }
                Err(Refusal::Code(code)) if code == "job-not-found" => {
                    return Err(Refusal::Uncertain);
                }
                Err(Refusal::Code(code)) => return Err(Refusal::Code(code)),
                Err(_) => return Err(Refusal::Uncertain),
            };
        }
    }
    /// Takes a home snapshot now (`Job` schema 2, one administrator
    /// approval) and returns its number.
    pub async fn snapshot_home(&self) -> Result<u64, Refusal> {
        let body = json!({"schema": 2, "operation": "home-history-create", "action": "submit", "request_id": request_id(), "allow_interaction": true});
        let reply = match self.call("Job", Some(body.to_string())).await {
            Err(Refusal::Unavailable) => return Err(Refusal::Uncertain),
            other => other?,
        };
        match reply.get("state").and_then(Value::as_str) {
            Some("completed") => reply
                .get("snapshot")
                .and_then(Value::as_u64)
                .ok_or(Refusal::Invalid),
            Some("cancelled") => Err(Refusal::Cancelled),
            _ => Err(Refusal::Uncertain),
        }
    }
}

fn status_withheld(code: &str) -> bool {
    matches!(code, "not-authorized" | "service-busy" | "job-store-busy")
}

/// A planned change awaiting its submit.
#[derive(Clone, Debug)]
pub struct Planned {
    schema: u32,
    operation: String,
    id: String,
    digest: String,
    selection: Option<Value>,
    /// The backend's whole plan reply.
    pub reply: Value,
}

/// `Query power-policy`: only the fixed action names are kept.
pub fn critical_battery(value: &Value) -> Option<super::CriticalBattery> {
    let known = |action: &str| super::CRITICAL_ACTIONS.contains(&action);
    let list = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|a| known(a))
            .map(str::to_owned)
            .collect()
    };
    let configured = value
        .get("configured_action")
        .and_then(Value::as_str)
        .filter(|a| known(a))?
        .to_owned();
    let mut supported = list("supported_actions");
    if !supported.contains(&configured) {
        supported.insert(0, configured.clone());
    }
    Some(super::CriticalBattery {
        configured,
        supported,
        risky: list("risk_acknowledgement_required"),
    })
}

pub(super) fn tmp_storage(value: &Value) -> Option<super::TemporaryStorage> {
    let mode = |value: &Value| {
        value
            .as_str()
            .filter(|mode| matches!(*mode, "ram" | "disk"))
            .map(str::to_owned)
    };
    let configured = match value.get("configured")? {
        Value::Null => None,
        value => Some(mode(value)?),
    };
    Some(super::TemporaryStorage {
        active: mode(&value["active"])?,
        configured,
    })
}

/// `Query memory-policy`: only the fixed preset names are kept.
pub fn memory_policy(value: &Value) -> Option<super::MemoryPolicy> {
    let known = |mode: &str| super::MEMORY_MODES.contains(&mode);
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .filter(|m| known(m))?
        .to_owned();
    let mut available: Vec<String> = value
        .get("available_modes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|m| known(m))
        .map(str::to_owned)
        .collect();
    if available.is_empty() {
        available = super::MEMORY_MODES
            .iter()
            .map(|m| (*m).to_owned())
            .collect();
    }
    if !available.contains(&mode) {
        available.insert(0, mode.clone());
    }
    let service = value.get("service");
    let field = |key: &str| service.and_then(|s| s.get(key));
    Some(super::MemoryPolicy {
        mode,
        available,
        active_state: field("ActiveState")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        running_mode: field("EffectiveMode")
            .and_then(Value::as_str)
            .filter(|m| known(m))
            .map(str::to_owned),
        memory_bytes: field("MemoryCurrent")
            .and_then(Value::as_u64)
            .filter(|b| *b < u64::MAX / 2),
        protected_names: value["protected_process_names"]
            .as_array()
            .filter(|names| names.len() <= 64)
            .and_then(|names| {
                let parsed: Option<Vec<String>> = names
                    .iter()
                    .map(|name| {
                        name.as_str()
                            .filter(|name| {
                                !name.is_empty()
                                    && name.len() <= 15
                                    && name
                                        .bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                            })
                            .map(str::to_owned)
                    })
                    .collect();
                parsed.map(|mut names| {
                    names.sort();
                    names.dedup();
                    names
                })
            }),
    })
}

/// `Query system`, as the About and Updates pages use it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    pub os: String,
    pub kernel: String,
    pub firmware: String,
    /// Package name and version, in the backend's order.
    pub packages: Vec<(String, String)>,
    pub generations: Vec<Generation>,
    /// `None`: still running the initial installation.
    pub current_generation: Option<String>,
    pub recovery_images_ready: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Generation {
    pub name: String,
    pub state: String,
    pub ordinal: i64,
    pub kernels: Vec<String>,
    pub confirmed: bool,
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.chars().take(160).collect(),
        Value::Null => String::new(),
        other => other.to_string().chars().take(160).collect(),
    }
}

impl SystemState {
    pub fn from_json(value: &Value) -> Self {
        let os = match value.get("os") {
            Some(Value::Object(os)) => os
                .get("PRETTY_NAME")
                .or_else(|| os.get("pretty_name"))
                .or_else(|| os.get("NAME"))
                .or_else(|| os.get("name"))
                .map(text)
                .unwrap_or_default(),
            Some(other) => text(other),
            None => String::new(),
        };
        let packages = match value.get("packages") {
            Some(Value::Object(map)) => map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (text(&Value::String(k.clone())), text(v)))
                .take(16)
                .collect(),
            _ => Vec::new(),
        };
        let generations = value
            .get("generations")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .take(32)
                    .map(|g| Generation {
                        name: g.get("name").map(text).unwrap_or_default(),
                        state: g.get("state").map(text).unwrap_or_default(),
                        ordinal: g.get("ordinal").and_then(Value::as_i64).unwrap_or(0),
                        kernels: match g.get("kernels") {
                            Some(Value::Array(k)) => k.iter().map(text).take(4).collect(),
                            Some(Value::Object(k)) => k.keys().take(4).cloned().collect(),
                            Some(other) if !other.is_null() => vec![text(other)],
                            _ => Vec::new(),
                        },
                        confirmed: g.get("confirmed").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            os,
            kernel: value.get("kernel").map(text).unwrap_or_default(),
            firmware: value.get("firmware").map(text).unwrap_or_default(),
            packages,
            generations,
            current_generation: value
                .get("current_generation")
                .filter(|v| !v.is_null())
                .map(text),
            // A flag or a count of ready images, depending on the backend release.
            recovery_images_ready: match value.get("recovery_images_ready") {
                Some(Value::Bool(ready)) => *ready,
                Some(Value::Number(count)) => count.as_u64().is_some_and(|n| n > 0),
                _ => false,
            },
        }
    }
}

/// `Query history` and `Query backups`, for the History & Backups page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Safety {
    pub home_history: bool,
    pub home_history_state: String,
    pub root_snapshots: Option<u64>,
    pub home_snapshots: Option<u64>,
    pub latest_home: Option<String>,
    pub local_configured: bool,
    pub remote_configured: bool,
    /// The network share copy (`remote`), set up or not.
    pub remote: super::backup_remote::RemoteShare,
    pub schedule_enabled: bool,
    pub recovery_copy_state: String,
    pub recovery_available: bool,
    pub setup_available: bool,
    pub candidates: Option<Vec<super::backup_setup::Candidate>>,
    pub keep_last: Option<u64>,
    pub backup_retention: Option<super::backups::BackupRetention>,
    pub local_cleanup: Option<super::backups::Cleanup>,
    pub remote_cleanup: Option<super::backups::Cleanup>,
    pub last_backup: Option<super::backups::LastBackup>,
    pub next_run: Option<String>,
    pub destination_label: Option<String>,
    pub destination_free: Option<u64>,
    pub repository_bytes: Option<u64>,
    /// Snapshot lists (system, home), when the backend offers `snapshots`.
    pub lists: Option<(Vec<super::backups::Snapshot>, Vec<super::backups::Snapshot>)>,
    /// Recovery points, when the backend offers `backup-points`.
    pub points: Option<Vec<super::backups::Point>>,
    pub points_error: Option<String>,
    /// How many home snapshots future cleanup keeps, when reported.
    pub home_retention: Option<super::backups::Retention>,
    /// The operations this backend runs (`GetCapabilities`).
    pub operations: Vec<String>,
    /// How the last backup run ended: `ok`, `waiting-for-drive`, `failed`.
    pub run_state: String,
    /// When that state began, the last success, and the failure text.
    pub run_since: Option<i64>,
    pub last_success: Option<i64>,
    pub run_code: Option<String>,
    /// A backup running now, whoever started it.
    pub running: Option<crate::backup_activity::Running>,
    /// Where backups are written: `drive`, `share`, or `None` when neither is
    /// ready. `primary_reported` is false for backends without the field.
    pub primary: Option<String>,
    pub primary_reported: bool,
    pub recovery_exclude_uuid: Option<String>,
    /// Space the backups used on the drive at the last cleanup.
    pub repository_measured: Option<u64>,
    /// The limit is smaller than one backup.
    pub budget_over: bool,
    /// Files waiting to be removed from the backup drive.
    pub pending_removals: u64,
    /// The drive is connected and verified now.
    pub destination_available: bool,
}
impl Safety {
    pub fn from_json(history: &Value, backups: &Value) -> Self {
        let count = |key: &str| {
            history
                .get(key)
                .and_then(|v| v.get("snapshots"))
                .and_then(Value::as_u64)
        };
        Self {
            home_history: history
                .pointer("/home_history/enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            home_history_state: history
                .pointer("/home_history/state")
                .map(text)
                .unwrap_or_default(),
            root_snapshots: count("root"),
            home_snapshots: count("home"),
            latest_home: history
                .pointer("/home/latest")
                .filter(|v| !v.is_null())
                .map(text),
            local_configured: backups
                .get("local_configured")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            remote_configured: backups
                .get("remote_configured")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            remote: super::backup_remote::RemoteShare::from_json(&backups["remote"]),
            schedule_enabled: backups
                .get("schedule_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            recovery_copy_state: backups
                .pointer("/recovery_copy/state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            recovery_available: false,
            setup_available: false,
            candidates: None,
            keep_last: backups.get("keep_last").and_then(Value::as_u64),
            backup_retention: backups
                .pointer("/retention/policy")
                .and_then(super::backups::BackupRetention::from_json),
            local_cleanup: backups
                .pointer("/retention/last_cleanup/local")
                .and_then(super::backups::Cleanup::from_json),
            remote_cleanup: backups
                .pointer("/retention/last_cleanup/remote")
                .and_then(super::backups::Cleanup::from_json),
            last_backup: super::backups::last_backup(backups),
            next_run: backups.get("next_run").filter(|v| !v.is_null()).map(text),
            destination_label: backups
                .get("destination_label")
                .filter(|v| !v.is_null())
                .map(text),
            destination_free: backups
                .get("destination_free_bytes")
                .and_then(Value::as_u64),
            repository_bytes: backups.get("repository_bytes").and_then(Value::as_u64),
            lists: None,
            points: None,
            points_error: None,
            home_retention: history
                .get("home_retention")
                .and_then(super::backups::Retention::from_json),
            operations: Vec::new(),
            run_state: backups
                .pointer("/run/state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            run_since: backups.pointer("/run/since").and_then(Value::as_i64),
            last_success: backups.pointer("/run/last_success").and_then(Value::as_i64),
            running: crate::backup_activity::Running::from_status(backups),
            primary: backups
                .get("primary")
                .and_then(Value::as_str)
                .filter(|primary| matches!(*primary, "drive" | "share"))
                .map(str::to_owned),
            primary_reported: backups.get("primary").is_some(),
            recovery_exclude_uuid: backups
                .get("recovery_save_exclude_uuid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            run_code: backups
                .pointer("/run/code")
                .and_then(Value::as_str)
                .map(|s| s.chars().filter(|c| !c.is_control()).take(160).collect()),
            repository_measured: backups
                .get("repository_bytes_measured")
                .and_then(Value::as_u64),
            budget_over: backups
                .pointer("/budget/over")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            pending_removals: backups
                .get("pending_removals")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            destination_available: backups
                .get("destination_available")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
    /// Waiting for the backup drive to be connected.
    /// The newest run failed: no backup succeeded since that failure began.
    pub fn failed_since_success(&self) -> bool {
        self.run_state == "failed"
            && match (self.run_since, self.last_success) {
                (Some(failed), Some(succeeded)) => failed > succeeded,
                _ => true,
            }
    }
    /// Backups go only to the network share.
    pub fn share_only(&self) -> bool {
        !self.local_configured && self.remote_configured
    }
    /// The network share receives the backups themselves.
    pub fn share_primary(&self) -> bool {
        self.primary.as_deref() == Some("share")
    }
    /// "Back up now" and the schedule have somewhere to write.
    pub fn can_back_up(&self) -> bool {
        if self.primary_reported {
            self.primary.is_some()
        } else {
            self.local_configured
        }
    }
    pub fn waiting_for_drive(&self) -> bool {
        self.run_state == "waiting-for-drive" && !self.destination_available
    }
    pub fn destination(&self) -> bool {
        self.local_configured || self.remote_configured
    }
    pub fn offers(&self, operation: &str) -> bool {
        self.operations.iter().any(|known| known == operation)
    }
}

/// One NetworkManager connection as the backend sees its trust.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustRow {
    pub connection: String,
    pub name: String,
    pub trusted: bool,
}
pub fn trust_rows(value: &Value) -> Vec<TrustRow> {
    value
        .get("connections")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .take(256)
                .filter_map(|row| {
                    Some(TrustRow {
                        connection: row.get("connection")?.as_str()?.to_owned(),
                        name: row.get("name").map(text).unwrap_or_default(),
                        trusted: row.get("trusted").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn enzimos_behaviour() {
        // Memory policy keeps fixed presets and the real service state.
        {
            let value = json!({
                "mode": "responsive",
                "available_modes": ["balanced", "responsive", "off", "turbo"],
                "default_mode": "balanced",
                "service": {"ActiveState": "active", "EffectiveMode": "balanced", "MemoryCurrent": 1_530_000},
            });
            let policy = memory_policy(&value).expect("a policy");
            assert_eq!(policy.available, ["balanced", "responsive", "off"]);
            assert_eq!(
                policy.running_mode.as_deref(),
                Some("balanced"),
                "the guard still runs the old preset"
            );
            assert_eq!(policy.memory_bytes, Some(1_530_000));
            assert!(memory_policy(&json!({"mode": "everything"})).is_none());
            let bare = memory_policy(&json!({"mode": "off"})).expect("a policy");
            assert_eq!(bare.available.len(), 3);
            assert_eq!(bare.active_state, "unknown");
            assert!(bare.protected_names.is_none());
            let mut value = value;
            value["protected_process_names"] = json!(["enzui-shell", "Hyprland", "enzui-shell"]);
            assert_eq!(
                memory_policy(&value).unwrap().protected_names,
                Some(vec!["Hyprland".into(), "enzui-shell".into()])
            );
            for bad in [
                json!(["bad.*"]),
                json!(["sixteencharacters"]),
                json!([12]),
                json!(vec!["Hyprland"; 65]),
            ] {
                value["protected_process_names"] = bad;
                assert!(memory_policy(&value).unwrap().protected_names.is_none());
            }
        }
        // Power policy keeps only known actions.
        {
            let value = json!({"configured_action": "auto", "risky_action_allowed": false,
                "supported_actions": ["auto", "hibernate", "power-off", "ignore", "reboot-now"],
                "risk_acknowledgement_required": ["suspend", "ignore"], "effective_after": "next-boot", "owner": "UPower"});
            let policy = critical_battery(&value).unwrap();
            assert_eq!(
                policy.supported,
                ["auto", "hibernate", "power-off", "ignore"]
            );
            assert_eq!(policy.risky, ["suspend", "ignore"]);
            assert!(critical_battery(&json!({"configured_action": "explode"})).is_none());
            let body: Value = serde_json::from_str(&operation_body(
                28,
                "battery-action-set",
                "plan",
                &request_id(),
                None,
                Some(&json!({"action": "ignore", "risk_acknowledged": true})),
            ))
            .unwrap();
            assert_eq!(body["selection"]["risk_acknowledged"], true);
        }
        // Requests answers and refusals follow the published shapes.
        {
            let id = request_id();
            assert_eq!(id.len(), 32);
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
            assert_ne!(id, request_id());
            let plan: Value = serde_json::from_str(&operation_body(
                25,
                "network-trust-set",
                "plan",
                &id,
                None,
                Some(&json!({"connection": "u", "trusted": true})),
            ))
            .unwrap();
            assert_eq!(plan["selection"]["trusted"], true);
            assert_eq!(plan["plan_sha256"], Value::Null);
            assert_eq!(plan["allow_interaction"], true);
            let submit: Value = serde_json::from_str(&operation_body(
                25,
                "network-trust-set",
                "submit",
                &id,
                Some("ab"),
                Some(&json!({})),
            ))
            .unwrap();
            assert_eq!(
                (submit["selection"].clone(), submit["plan_sha256"].clone()),
                (Value::Null, json!("ab"))
            );
            let update: Value =
                serde_json::from_str(&operation_body(5, "update-apply", "plan", &id, None, None))
                    .unwrap();
            assert!(update.get("selection").is_none());

            assert!(Refusal::Uncertain.is_uncertain());
            assert!(
                Refusal::Code("backend-result-unknown-do-not-retry-automatically".into())
                    .is_uncertain()
            );
            assert!(!Refusal::Code("not-authorized".into()).is_uncertain());
            assert!(!Refusal::NotPrepared.is_uncertain());
            assert_eq!(
                code_message("service-busy"),
                "enzimOS is busy with another change. Try again in a moment."
            );
            assert!(
                status_withheld("not-authorized") && status_withheld("service-busy"),
                "a locked session's status refusal is not the job's end"
            );
            assert!(!status_withheld("job-not-found") && !status_withheld("job-owner-mismatch"));

            let system = SystemState::from_json(&json!({
                "os": {"PRETTY_NAME": "enzimOS", "ID": "enzimos"}, "kernel": "6.18.2-arch1-1", "firmware": "uefi",
                "packages": {"enzimos-native-policy": "0.1.0-101", "linux-lts": null},
                "generations": [{"name": "g-2", "state": "selected", "ordinal": 2, "kernels": ["linux"], "confirmed": false}],
                "current_generation": null, "recovery_images_ready": 1
            }));
            assert_eq!(
                (
                    system.os.as_str(),
                    system.firmware.as_str(),
                    system.packages.len()
                ),
                ("enzimOS", "uefi", 1)
            );
            assert_eq!(
                (
                    system.generations[0].name.as_str(),
                    system.current_generation.clone()
                ),
                ("g-2", None)
            );
            let safety = Safety::from_json(
                &json!({"home_history": {"enabled": true, "state": "active"}, "root": {"snapshots": 3, "latest": null}, "home": {"snapshots": 12, "latest": "2026-09-28 10:00"}}),
                &json!({"local_configured": false, "remote_configured": false, "schedule_enabled": false, "keep_last": 7}),
            );
            assert!(safety.home_history && !safety.destination());
            assert_eq!(
                (
                    safety.root_snapshots,
                    safety.home_snapshots,
                    safety.keep_last
                ),
                (Some(3), Some(12), Some(7))
            );
            let rows = trust_rows(
                &json!({"trusted_zone": "enzimos-trusted", "connections": [{"connection": "u1", "name": "Home", "trusted": true}, {"name": "broken"}]}),
            );
            assert_eq!(
                rows,
                vec![TrustRow {
                    connection: "u1".into(),
                    name: "Home".into(),
                    trusted: true
                }]
            );
        }
    }

    use super::*;
}
