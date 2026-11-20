//! The encrypted backup copy on an SMB network share (NAS): status, setup,
//! test and removal through org.enzimos.System1 (D-124).

use super::{SystemSettings, backup_setup, enzimos};
use enzui_core::form::SecretText;
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{os::fd::OwnedFd, time::Duration};

const NAME: &str = "org.enzimos.System1";
const PATH: &str = "/org/enzimos/System1";
const REFUSED: &str = "org.enzimos.System1.Refused";
/// The share password is one line of at most this many bytes.
pub const SHARE_PASSWORD_BYTES: usize = 512;
/// Setup checks the share and may create a repository on it.
const SETUP_PATIENCE: Duration = Duration::from_secs(6 * 60);
const TEST_PATIENCE: Duration = Duration::from_secs(3 * 60);
const REMOVE_PATIENCE: Duration = Duration::from_secs(2 * 60);

/// What `Query backups` reports under `remote`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteShare {
    /// `not-configured`, `configured`, `reconfigure-as-smb` or `invalid`.
    pub state: String,
    /// `local-backup` (the local backup password is reused) or `input`.
    pub password_source: String,
    pub server: String,
    pub share: String,
    pub folder: String,
    pub user: String,
    pub domain: String,
    pub location: String,
    pub share_password_ready: bool,
    pub repository_password_ready: bool,
    pub repository_ready: bool,
}

fn field(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(255)
        .collect()
}

impl RemoteShare {
    pub fn from_json(value: &Value) -> Self {
        let flag = |key: &str| value[key].as_bool().unwrap_or(false);
        Self {
            state: match value["state"].as_str() {
                Some(state @ ("configured" | "reconfigure-as-smb" | "invalid")) => state.into(),
                _ => "not-configured".into(),
            },
            password_source: field(value, "repository_password_source"),
            server: field(value, "server"),
            share: field(value, "share"),
            folder: field(value, "folder"),
            user: field(value, "user"),
            domain: field(value, "domain"),
            location: field(value, "location"),
            share_password_ready: flag("share_password_ready"),
            repository_password_ready: flag("repository_password_ready"),
            repository_ready: flag("repository_ready"),
        }
    }
    pub fn configured(&self) -> bool {
        self.state == "configured"
    }
    /// The NAS needs its own repository password: there is no local backup
    /// whose password it could share.
    pub fn needs_repository_password(&self) -> bool {
        self.password_source == "input"
    }
    /// `smb://server/share/folder`, as the backend reports it or built from its parts.
    pub fn address(&self) -> String {
        if self.location.is_empty() {
            address(&self.server, &self.share, &self.folder)
        } else {
            self.location.clone()
        }
    }
    /// The user as the share sees it: `DOMAIN\user` with a domain.
    pub fn account(&self) -> String {
        if self.domain.is_empty() {
            self.user.clone()
        } else {
            format!("{}\\{}", self.domain, self.user)
        }
    }
}

pub fn address(server: &str, share: &str, folder: &str) -> String {
    let folder = folder.trim_matches('/');
    if folder.is_empty() {
        format!("smb://{server}/{share}")
    } else {
        format!("smb://{server}/{share}/{folder}")
    }
}

/// The share to set up, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareInput {
    pub server: String,
    pub share: String,
    pub folder: String,
    pub user: String,
    pub domain: String,
}

impl ShareInput {
    /// Fills the form from the current setup, or suggests a folder named
    /// after this computer.
    pub fn suggested(current: &RemoteShare, hostname: &str) -> Self {
        if current.server.is_empty() {
            let host: String = hostname
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .take(63)
                .collect();
            Self {
                folder: format!(
                    "enzimos/{}",
                    if host.is_empty() { "computer" } else { &host }
                ),
                ..Self::default()
            }
        } else {
            Self {
                server: current.server.clone(),
                share: current.share.clone(),
                folder: current.folder.clone(),
                user: current.user.clone(),
                domain: current.domain.clone(),
            }
        }
    }
    /// Everything the backend requires is filled in, with no path tricks.
    pub fn complete(&self) -> bool {
        let plain = |text: &str, max: usize| {
            let text = text.trim();
            !text.is_empty()
                && text.len() <= max
                && !text
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        };
        let folder = self.folder.trim().trim_matches('/');
        plain(&self.server, 253)
            && plain(&self.share, 80)
            && plain(&self.user, 256)
            && (self.domain.trim().is_empty() || plain(&self.domain, 255))
            && folder.len() <= 1024
            && !folder.chars().any(char::is_control)
            && !folder.split('/').any(|part| matches!(part, "." | ".."))
    }
    /// The `BackupRemoteShareSetup` body; the domain only when given.
    pub fn body(&self) -> String {
        let mut share = json!({
            "server": self.server.trim(),
            "share": self.share.trim(),
            "folder": self.folder.trim().trim_matches('/'),
            "user": self.user.trim(),
        });
        if !self.domain.trim().is_empty() {
            share["domain"] = json!(self.domain.trim());
        }
        json!({"schema": 1, "allow_interaction": true, "share": share}).to_string()
    }
}

pub fn validate_share_password(password: &SecretText) -> Result<(), String> {
    let text = password.expose_secret();
    if text.is_empty()
        || text.len() > SHARE_PASSWORD_BYTES
        || text.bytes().any(|b| matches!(b, 0 | 10 | 13))
    {
        return Err(error_message("remote-share-secret-refused"));
    }
    Ok(())
}

/// The setup descriptors, in order: the share password, then the repository
/// password only when the NAS needs its own.
pub fn setup_inputs(
    share_password: &SecretText,
    repository_password: Option<&SecretText>,
    needs_repository_password: bool,
) -> Result<Vec<OwnedFd>, String> {
    validate_share_password(share_password)?;
    let mut fds = vec![backup_setup::sealed_input(share_password)?];
    match (needs_repository_password, repository_password) {
        (true, Some(password)) => fds.push(backup_setup::sealed_input(password)?),
        (true, None) => return Err(error_message("remote-repository-password-required")),
        (false, _) => {}
    }
    Ok(fds)
}

/// The body of a `BackupRemoteShare` call: `test` or `remove`.
pub fn action_body(action: &str) -> String {
    json!({"schema": 1, "allow_interaction": true, "action": action}).to_string()
}

/// What a test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tested {
    pub reachable: bool,
    pub signed_in: bool,
    /// `ready`, `not-pinned`, `absent`, `password-refused`,
    /// `different-repository` or `unknown`.
    pub repository: String,
    pub reason: Option<String>,
}

impl Tested {
    pub fn ok(&self) -> bool {
        self.reachable && self.signed_in && self.repository == "ready"
    }
    /// One sentence for the row: what works, or the first thing that does not.
    pub fn summary(&self) -> String {
        if let Some(reason) = self.reason.as_deref().filter(|_| !self.ok()) {
            return error_message(reason);
        }
        if !self.reachable {
            return error_message("remote-share-unreachable");
        }
        if !self.signed_in {
            return error_message("remote-share-sign-in-refused");
        }
        match self.repository.as_str() {
            "ready" => tr("shell-share-test-ready").into(),
            "absent" => tr("shell-share-test-absent").into(),
            "password-refused" => error_message("remote-repository-password-refused"),
            "different-repository" => tr("shell-share-test-different").into(),
            "not-pinned" => tr("shell-share-test-not-pinned").into(),
            _ => tr("shell-share-test-unknown").into(),
        }
    }
}

pub fn parse_tested(value: &Value) -> Result<Tested, String> {
    let reachable = value["reachable"].as_bool();
    let signed_in = value["signed_in"].as_bool();
    match (value["status"].as_str(), reachable, signed_in) {
        (Some("remote-share-tested"), Some(reachable), Some(signed_in)) => Ok(Tested {
            reachable,
            signed_in,
            repository: match value["repository"].as_str() {
                Some(
                    state @ ("ready"
                    | "not-pinned"
                    | "absent"
                    | "password-refused"
                    | "different-repository"),
                ) => state.into(),
                _ => "unknown".into(),
            },
            reason: value["reason"]
                .as_str()
                .filter(|code| code.len() <= 80)
                .map(str::to_owned),
        }),
        _ => Err(tr("shell-invalid-backup-setup-response").into()),
    }
}

/// `created` or `adopted`, from a setup reply.
pub fn parse_configured(value: &Value) -> Result<String, String> {
    match (value["status"].as_str(), value["repository"].as_str()) {
        (Some("remote-share-configured"), Some(repository @ ("created" | "adopted"))) => {
            Ok(repository.into())
        }
        _ => Err(tr("shell-invalid-backup-setup-response").into()),
    }
}

pub fn parse_removed(value: &Value) -> Result<(), String> {
    if value["status"] == "remote-share-removed" && value["nas_data_deleted"] == false {
        Ok(())
    } else {
        Err(tr("shell-invalid-backup-setup-response").into())
    }
}

/// The sentence for a network-share refusal code; other codes keep the
/// general wording.
pub fn error_message(code: &str) -> String {
    let key = match code {
        "remote-share-invalid" => "shell-share-error-invalid",
        "remote-share-secret-refused" => "shell-share-error-secret",
        "remote-share-unreachable" => "shell-share-error-unreachable",
        "remote-share-sign-in-refused" => "shell-share-error-sign-in",
        "remote-share-not-found" => "shell-share-error-not-found",
        "remote-share-smb3-unsupported" => "shell-share-error-smb3",
        "remote-share-unavailable" => "shell-share-error-unavailable",
        "remote-repository-password-refused" => "shell-share-error-password-refused",
        "remote-repository-foreign" => "shell-share-error-foreign",
        "remote-repository-password-required" => "shell-share-error-password-required",
        "remote-share-not-configured" => "shell-share-error-not-configured",
        "remote-backup-reconfigure-as-smb" => "shell-share-error-reconfigure",
        "remote-rotation-unfinished" => "shell-share-error-rotation",
        other => return enzimos::code_message(other).into(),
    };
    tr(key).into()
}

impl SystemSettings {
    /// Sets the share up (or changes it); returns `created` or `adopted`.
    pub async fn remote_share_setup(
        &self,
        input: ShareInput,
        share_password: SecretText,
        repository_password: Option<SecretText>,
        needs_repository_password: bool,
    ) -> Result<String, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if !input.complete() {
            return Err(error_message("remote-share-invalid"));
        }
        let fds = setup_inputs(
            &share_password,
            repository_password.as_ref(),
            needs_repository_password,
        )?;
        if self.demo {
            drop(fds);
            tokio::time::sleep(Duration::from_millis(1800)).await;
            return super::demo::remote_share_setup(&input, share_password.expose_secret())
                .map_err(|code| error_message(&code));
        }
        drop((share_password, repository_password));
        let reply = self
            .remote_call("BackupRemoteShareSetup", input.body(), fds, SETUP_PATIENCE)
            .await?;
        parse_configured(&reply)
    }
    pub async fn remote_share_test(&self) -> Result<Tested, String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            return super::demo::remote_share_test().map_err(|code| error_message(&code));
        }
        let reply = self
            .remote_call(
                "BackupRemoteShare",
                action_body("test"),
                Vec::new(),
                TEST_PATIENCE,
            )
            .await?;
        parse_tested(&reply)
    }
    /// Forgets the share; the backups already on the NAS stay there.
    pub async fn remote_share_remove(&self) -> Result<(), String> {
        if !self.writes_allowed {
            return Err(tr("shell-system-changes-are-off-in-this-preview").into());
        }
        if self.demo {
            tokio::time::sleep(Duration::from_millis(800)).await;
            return super::demo::remote_share_remove().map_err(|code| error_message(&code));
        }
        let reply = self
            .remote_call(
                "BackupRemoteShare",
                action_body("remove"),
                Vec::new(),
                REMOVE_PATIENCE,
            )
            .await?;
        parse_removed(&reply)
    }
    async fn remote_call(
        &self,
        member: &str,
        body: String,
        fds: Vec<OwnedFd>,
        patience: Duration,
    ) -> Result<Value, String> {
        let connection = zbus::Connection::system()
            .await
            .map_err(|_| tr("shell-the-system-bus-is-unavailable").to_owned())?;
        let call = async {
            let reply = if member == "BackupRemoteShareSetup" {
                let fds: Vec<zbus::zvariant::Fd> =
                    fds.iter().map(zbus::zvariant::Fd::from).collect();
                connection
                    .call_method(Some(NAME), PATH, Some(NAME), member, &(body.as_str(), fds))
                    .await
            } else {
                connection
                    .call_method(Some(NAME), PATH, Some(NAME), member, &(body.as_str(),))
                    .await
            };
            let reply = reply.map_err(|error| match error {
                zbus::Error::MethodError(name, message, _) if name.as_str() == REFUSED => {
                    error_message(message.as_deref().unwrap_or_default())
                }
                _ => enzimos::Refusal::Unavailable.message(),
            })?;
            let text: String = reply
                .body()
                .deserialize()
                .map_err(|_| tr("shell-invalid-backup-setup-response").to_owned())?;
            serde_json::from_str::<Value>(&text)
                .map_err(|_| tr("shell-invalid-backup-setup-response").to_owned())
        };
        // A late answer is not retried: the page reads the status again.
        tokio::time::timeout(patience, call)
            .await
            .map_err(|_| tr("shell-share-no-answer").to_owned())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ShareInput {
        ShareInput {
            server: "nas.local".into(),
            share: "backups".into(),
            folder: "/enzimos/laptop/".into(),
            user: "ann".into(),
            domain: String::new(),
        }
    }

    #[test]
    fn backup_remote_behaviour() {
        // The setup body names only the share and adds a domain when given.
        {
            let body: Value = serde_json::from_str(&input().body()).unwrap();
            assert_eq!(
                body,
                json!({"schema":1,"allow_interaction":true,"share":{"server":"nas.local","share":"backups","folder":"enzimos/laptop","user":"ann"}})
            );
            let body: Value = serde_json::from_str(
                &ShareInput {
                    domain: " HOME ".into(),
                    ..input()
                }
                .body(),
            )
            .unwrap();
            assert_eq!(body["share"]["domain"], "HOME");
            assert_eq!(body["share"].as_object().unwrap().len(), 5);
        }
        // The repository password is sent only without a local backup.
        {
            let share = SecretText::new("share-secret".into());
            let repository = SecretText::new("repository-secret".into());
            assert_eq!(
                setup_inputs(&share, Some(&repository), false)
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(setup_inputs(&share, None, false).unwrap().len(), 1);
            assert_eq!(
                setup_inputs(&share, Some(&repository), true).unwrap().len(),
                2
            );
            assert_eq!(
                setup_inputs(&share, None, true).unwrap_err(),
                error_message("remote-repository-password-required")
            );
            let long = SecretText::new("x".repeat(SHARE_PASSWORD_BYTES + 1));
            assert!(setup_inputs(&long, None, false).is_err());
            let two_lines = SecretText::new("a\nb".into());
            assert_eq!(
                setup_inputs(&two_lines, None, false).unwrap_err(),
                error_message("remote-share-secret-refused")
            );
        }
        // Action bodies follow the published shape.
        {
            let body: Value = serde_json::from_str(&action_body("test")).unwrap();
            assert_eq!(
                body,
                json!({"schema":1,"allow_interaction":true,"action":"test"})
            );
        }
        // Refusal codes read as plain sentences.
        {
            for code in [
                "remote-share-invalid",
                "remote-share-secret-refused",
                "remote-share-unreachable",
                "remote-share-sign-in-refused",
                "remote-share-not-found",
                "remote-share-smb3-unsupported",
                "remote-share-unavailable",
                "remote-repository-password-refused",
                "remote-repository-foreign",
                "remote-repository-password-required",
                "remote-share-not-configured",
                "remote-backup-reconfigure-as-smb",
                "remote-rotation-unfinished",
            ] {
                let message = error_message(code);
                assert!(
                    !message.starts_with("shell-") && !message.is_empty(),
                    "{code}"
                );
                assert_ne!(message, enzimos::code_message(code), "{code}");
            }
            assert_eq!(
                error_message("service-busy"),
                enzimos::code_message("service-busy")
            );
        }
        // Status and replies parse strictly.
        {
            let share = RemoteShare::from_json(&json!({
                "state":"configured","server":"nas.local","share":"backups","folder":"enzimos/laptop",
                "user":"ann","domain":"HOME","repository_password_source":"local-backup","repository_ready":true
            }));
            assert!(share.configured() && !share.needs_repository_password());
            assert_eq!(share.address(), "smb://nas.local/backups/enzimos/laptop");
            assert_eq!(share.account(), "HOME\\ann");
            assert_eq!(
                RemoteShare::from_json(&json!({"state":"sftp"})).state,
                "not-configured"
            );
            let tested = parse_tested(&json!({"status":"remote-share-tested","reachable":true,"signed_in":true,"repository":"ready","reason":null})).unwrap();
            assert!(tested.ok());
            assert!(parse_tested(&json!({"status":"remote-share-removed"})).is_err());
            assert_eq!(
                parse_configured(
                    &json!({"status":"remote-share-configured","repository":"adopted"})
                )
                .unwrap(),
                "adopted"
            );
            assert!(
                parse_removed(&json!({"status":"remote-share-removed","nas_data_deleted":true}))
                    .is_err()
            );
        }
        // Incomplete or unsafe shares are not sent.
        {
            assert!(input().complete());
            for broken in [
                ShareInput {
                    server: String::new(),
                    ..input()
                },
                ShareInput {
                    share: "a/b".into(),
                    ..input()
                },
                ShareInput {
                    folder: "enzimos/../etc".into(),
                    ..input()
                },
                ShareInput {
                    user: "an\nn".into(),
                    ..input()
                },
            ] {
                assert!(!broken.complete(), "{broken:?}");
            }
            let suggested = ShareInput::suggested(&RemoteShare::default(), "enzimos-laptop");
            assert_eq!(suggested.folder, "enzimos/enzimos-laptop");
        }
    }
}
