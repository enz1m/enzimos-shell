use super::{SystemSettings, enzimos};
use enzui_core::form::SecretText;
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{
    io::{Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    time::Duration,
};

const NAME: &str = "org.enzimos.System1";
const PATH: &str = "/org/enzimos/System1";
const LIMIT: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub model: String,
    pub label: String,
    pub bytes: Option<u64>,
    pub token: Option<String>,
    pub reason: String,
}

fn display(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn candidates(value: &Value) -> Result<Vec<Candidate>, String> {
    if value["schema"] != 1 {
        return Err(tr("shell-the-backup-drive-list-is-not-understood").into());
    }
    let rows = value["candidates"]
        .as_array()
        .filter(|rows| rows.len() <= 128)
        .ok_or_else(|| tr("shell-the-backup-drive-list-is-not-understood"))?;
    Ok(rows
        .iter()
        .map(|row| Candidate {
            model: display(row, "model"),
            label: display(row, "label"),
            bytes: row["bytes"].as_u64(),
            token: row["selection_id"]
                .as_str()
                .filter(|token| {
                    hex(token, 64)
                        && row["state"] == "needs-filesystem-verification"
                        && row["filesystem_verified"] == false
                })
                .map(str::to_owned),
            reason: display(row, "reason"),
        })
        .collect())
}
pub fn available(caps: &Value) -> bool {
    let setup = &caps["backup_setup"];
    setup["schema"] == 1
        && setup["control"] == "BackupSetupControl(s)->s"
        && setup["input"] == "BackupSetupInput(sh)->s"
        && setup["runtime_qualified"] == true
        && setup["sealed_input_only"] == true
        && caps["queries"]
            .as_array()
            .is_some_and(|queries| queries.iter().any(|query| query == "backup-candidates"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Initialize,
    Adopt,
}
impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Adopt => "adopt",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: String,
    pub attempts: u8,
    pub digest: String,
    pub retryable: bool,
    pub cancellable: bool,
}
impl Status {
    pub fn waiting(&self) -> bool {
        matches!(
            self.state.as_str(),
            "review-awaiting-approval" | "reviewing" | "awaiting-approval" | "running"
        )
    }
    /// A rolled-back request is retryable only through a new review: input on
    /// the same request is refused.
    pub fn accepts_input(&self) -> bool {
        self.state == "planned" || (self.retryable && self.state != "rolled-back")
    }
    /// A final answer: nothing is pending that cancelling or rechecking could change.
    pub fn settled(&self) -> bool {
        self.state == "rolled-back"
            || (!self.retryable
                && matches!(self.state.as_str(), "refused" | "cancelled" | "completed"))
    }
    pub fn failed_review(&self) -> bool {
        self.state == "refused" && self.attempts == 0 && !self.retryable
    }
    /// Ended with nothing changed, so a new review (a new request) may start.
    pub fn restartable(&self) -> bool {
        self.failed_review() || self.state == "cancelled" || self.state == "rolled-back"
    }
    pub fn label(&self) -> &'static str {
        match self.state.as_str() {
            "review-awaiting-approval" => {
                tr("shell-waiting-for-administrator-approval-to-review-the-drive")
            }
            "reviewing" => tr("shell-checking-the-drive-read-only-do-not-disconnect-it"),
            "planned" => tr(
                "shell-drive-review-complete-enter-the-repository-password-and-approve-enrollment-s",
            ),
            "awaiting-approval" => {
                tr("shell-waiting-for-separate-administrator-approval-to-enroll-this-repository")
            }
            "running" => tr(
                "shell-enrolling-the-repository-do-not-disconnect-the-drive-cancellation-is-no-long",
            ),
            "completed" => tr(
                "shell-repository-enrolled-save-or-print-a-separate-recovery-copy-next-automatic-ba",
            ),
            "password-retry" => tr(
                "shell-the-existing-repository-did-not-accept-that-password-enter-a-new-password-an",
            ),
            "refused" => {
                tr("shell-approval-was-refused-before-entry-no-automatic-retry-will-be-made")
            }
            "cancelled" => tr(
                "shell-cancelled-before-entry-an-already-open-authentication-prompt-may-still-need",
            ),
            "rolled-back" => tr(
                "shell-setup-failed-and-was-fully-undone-nothing-was-created-or-kept-start-a-new-dr",
            ),
            _ => tr(
                "shell-the-outcome-is-uncertain-inspect-the-current-system-state-do-not-repeat-enro",
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Client {
    connection: zbus::Connection,
    owner: String,
}
#[derive(Clone, Debug)]
pub struct Job {
    pub client: Client,
    pub id: String,
    pub mode: Mode,
    pub reviewed: Option<String>,
}
impl Job {
    pub fn new(client: Client, mode: Mode) -> Self {
        Self {
            client,
            id: enzimos::request_id(),
            mode,
            reviewed: None,
        }
    }
    fn body(&self, action: &str, selection: Option<&str>) -> String {
        json!({"schema":1,"action":action,"request_id":self.id,"selection_id":selection,
            "mode":if action == "plan" { Some(self.mode.key()) } else { None },
            "plan_sha256":if action == "cancel" { self.reviewed.as_deref() } else { None },
            "allow_interaction":action == "plan"})
        .to_string()
    }
    fn parse(&self, value: Value) -> Result<Status, String> {
        let state = value["state"]
            .as_str()
            .ok_or_else(|| tr("shell-invalid-backup-setup-status"))?;
        let states = [
            "review-awaiting-approval",
            "reviewing",
            "planned",
            "awaiting-approval",
            "running",
            "completed",
            "password-retry",
            "refused",
            "cancelled",
            "rolled-back",
            "uncertain",
        ];
        let digest = value["plan_sha256"]
            .as_str()
            .filter(|s| hex(s, 64))
            .ok_or_else(|| tr("shell-invalid-backup-setup-status"))?;
        let attempts = value["attempts"]
            .as_u64()
            .filter(|n| *n <= 8)
            .ok_or_else(|| tr("shell-invalid-backup-setup-status"))? as u8;
        let retryable = value["retryable"]
            .as_bool()
            .ok_or_else(|| tr("shell-invalid-backup-setup-status"))?;
        let cancellable = value["cancellation_available"]
            .as_bool()
            .ok_or_else(|| tr("shell-invalid-backup-setup-status"))?;
        if value["schema"] != 1
            || value["request_id"] != self.id
            || value["mode"] != self.mode.key()
            || value["automatic_backup_enabled"] != false
            || !states.contains(&state)
            || self
                .reviewed
                .as_ref()
                .is_some_and(|reviewed| reviewed != digest)
            || (retryable
                && (!matches!(state, "password-retry" | "refused" | "rolled-back")
                    || attempts >= 8))
            || (state == "rolled-back" && (!retryable || cancellable))
            || (cancellable
                && !matches!(
                    state,
                    "review-awaiting-approval"
                        | "planned"
                        | "awaiting-approval"
                        | "password-retry"
                        | "refused"
                        | "cancelled"
                ))
        {
            return Err(
                tr("shell-invalid-backup-setup-status-check-the-system-before-continuing").into(),
            );
        }
        Ok(Status {
            state: state.into(),
            attempts,
            digest: digest.into(),
            retryable,
            cancellable,
        })
    }
    pub fn observe(&mut self, status: &Status) {
        if status.accepts_input() {
            self.reviewed = Some(status.digest.clone());
        }
    }
    pub async fn plan(&self, token: &str) -> Result<Status, String> {
        if !hex(token, 64) {
            return Err(tr("shell-select-a-current-eligible-backup-drive").into());
        }
        self.parse(
            self.client
                .call("BackupSetupControl", &self.body("plan", Some(token)), None)
                .await?,
        )
    }
    pub async fn status(&self) -> Result<Status, String> {
        self.parse(
            self.client
                .call("BackupSetupControl", &self.body("status", None), None)
                .await?,
        )
    }
    pub async fn cancel(&self, digest: &str) -> Result<Status, String> {
        if !hex(digest, 64) {
            return Err(tr("shell-the-current-review-is-not-available").into());
        }
        let mut job = self.clone();
        job.reviewed = Some(digest.into());
        self.parse(
            self.client
                .call("BackupSetupControl", &job.body("cancel", None), None)
                .await?,
        )
    }
    pub async fn input(&self, password: SecretText) -> Result<Status, String> {
        let digest = self
            .reviewed
            .as_ref()
            .ok_or_else(|| tr("shell-review-the-drive-before-entering-a-password"))?;
        let fd = sealed_input(&password)?;
        drop(password);
        let body =
            json!({"schema":1,"request_id":self.id,"plan_sha256":digest,"allow_interaction":true})
                .to_string();
        self.parse(
            self.client
                .call("BackupSetupInput", &body, Some(&fd))
                .await?,
        )
    }
}
impl Client {
    pub(super) async fn connect(connection: zbus::Connection) -> Result<Self, String> {
        let bus = zbus::fdo::DBusProxy::new(&connection)
            .await
            .map_err(|_| tr("shell-the-system-bus-is-unavailable"))?;
        let owner = bus
            .get_name_owner(
                NAME.try_into()
                    .map_err(|_| tr("shell-invalid-system-service-identity"))?,
            )
            .await
            .map_err(|_| tr("shell-the-system-service-is-unavailable"))?
            .to_string();
        let client = Self {
            connection: connection.clone(),
            owner,
        };
        if !available(&client.call("GetCapabilities", "", None).await?) {
            return Err(tr("shell-backup-enrollment-is-not-available-on-this-system").into());
        }
        if bus
            .get_name_owner(
                NAME.try_into()
                    .map_err(|_| enzui_shell::i18n::tr("shell-invalid-system-service-identity"))?,
            )
            .await
            .map_err(|_| enzui_shell::i18n::tr("shell-the-system-service-stopped"))?
            .as_str()
            != client.owner
        {
            return Err(tr("shell-the-system-service-changed-reopen-the-drive-review").into());
        }
        Ok(client)
    }
    async fn call(&self, method: &str, body: &str, fd: Option<&OwnedFd>) -> Result<Value, String> {
        let call = async {
            let bus = zbus::fdo::DBusProxy::new(&self.connection)
                .await
                .map_err(|_| tr("shell-the-system-bus-is-unavailable"))?;
            let owner = bus
                .get_name_owner(
                    NAME.try_into()
                        .map_err(|_| tr("shell-invalid-system-service-identity"))?,
                )
                .await
                .map_err(|_| tr("shell-the-system-service-stopped-do-not-repeat-enrollment"))?;
            if owner.as_str() != self.owner {
                return Err(tr("shell-the-system-service-changed-do-not-repeat-enrollment-inspect-the-existing-req").into());
            }
            let reply = if let Some(fd) = fd {
                self.connection.call_method(Some(self.owner.as_str()), PATH, Some(NAME), method, &(body, zbus::zvariant::Fd::from(fd))).await
            } else if method == "GetCapabilities" {
                self.connection.call_method(Some(self.owner.as_str()), PATH, Some(NAME), method, &()).await
            } else {
                self.connection.call_method(Some(self.owner.as_str()), PATH, Some(NAME), method, &(body,)).await
            }.map_err(|error| match error {
                zbus::Error::MethodError(name, message, _) if name.as_str() == "org.enzimos.System1.Refused" => enzimos::code_message(message.as_deref().unwrap_or_default()).to_owned(),
                _ => tr("shell-backup-setup-status-is-unavailable-check-this-request-s-status-before-repeat").into(),
            })?;
            let text: String = reply
                .body()
                .deserialize()
                .map_err(|_| tr("shell-invalid-backup-setup-response"))?;
            if text.len() > LIMIT {
                return Err(tr("shell-the-backup-setup-response-is-too-large").into());
            }
            serde_json::from_str(&text)
                .map_err(|_| tr("shell-invalid-backup-setup-response").into())
        };
        tokio::time::timeout(Duration::from_secs(20), call).await.map_err(|_| tr("shell-no-reply-from-backup-setup-the-outcome-may-be-unknown-check-status-do-not-re").to_owned())?
    }
}
pub fn validate_password(password: &SecretText) -> Result<(), String> {
    let text = password.expose_secret();
    if text.is_empty() || text.len() > 4096 || text.bytes().any(|b| matches!(b, 0 | 10 | 13)) {
        return Err(tr("shell-enter-a-single-line-password-of-1-4096-utf-8-bytes").into());
    }
    Ok(())
}
pub fn sealed_input(password: &SecretText) -> Result<OwnedFd, String> {
    validate_password(password)?;
    let fd = rustix::fs::memfd_create(
        "enzimos-backup-input",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|_| tr("shell-could-not-prepare-protected-password-input"))?;
    rustix::fs::fchmod(&fd, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR)
        .map_err(|_| tr("shell-could-not-protect-password-input"))?;
    let mut file = std::fs::File::from(fd);
    file.write_all(password.expose_secret().as_bytes())
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| tr("shell-could-not-prepare-protected-password-input"))?;
    rustix::fs::fcntl_add_seals(
        &file,
        rustix::fs::SealFlags::SEAL
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::WRITE,
    )
    .map_err(|_| tr("shell-could-not-seal-password-input"))?;
    Ok(file.into())
}
impl SystemSettings {
    pub async fn backup_setup_client(&self) -> Result<Client, String> {
        if !self.writes_allowed || self.demo {
            return Err(tr("shell-backup-setup-is-unavailable-in-this-preview").into());
        }
        self.enzimos()
            .await
            .ok_or_else(|| tr("shell-the-system-service-is-unavailable"))?
            .backup_setup_client()
            .await
    }
}

#[cfg(test)]
mod tests;
