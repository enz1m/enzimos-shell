//! Review-bound developer setup on the system bus. Mutations retain a dedicated
//! connection to the reviewing owner; disconnect cancels cache acquisition or
//! prevents a future commit, never rollback. The OS owns caller checks and polkit.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};
use std::time::Duration;

const SERVICE: &str = "org.enzimos.Developer1";
const PATH: &str = "/org/enzimos/Developer1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageStatus {
    Unknown,
    Missing,
    Installed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub package: String,
    pub status: PackageStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// The fixed profile name (`build`, `runtimes`, `containers`, `virtualization`).
    pub id: String,
    pub label: String,
    pub packages: Vec<String>,
    pub tools: Vec<Tool>,
    /// Every package of the profile is on the system, whoever installed it.
    pub installed: bool,
    /// This account has set the profile up; only that can be removed.
    pub enrolled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub profiles: Vec<Profile>,
    /// The developer extension is installed and matches this system.
    pub ready: bool,
    pub unavailable: Option<String>,
    pub removal: bool,
    pub download: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    Install,
    Remove,
    Download,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Review {
    pub operation: Operation,
    pub profile: String,
    pub plan_sha256: String,
    pub owner: Option<String>,
    /// Explicit packages installed, removed or acquired for the cache.
    pub packages: Vec<String>,
    /// Supporting cache/install packages, or preserved packages for removal.
    pub others: Vec<String>,
    pub download_bytes: Option<u64>,
    /// Space the installation takes, from the plan (covered by its digest).
    pub installed: Space,
    /// Advisory free space beside the plan: for `/usr`, and for the download cache.
    pub free_bytes: Option<u64>,
    pub cache_free_bytes: Option<u64>,
}

/// An installation's exact size. The service reports it only when every
/// package archive is already verified locally, and never estimates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    NotReported,
    Unknown,
    Known(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    State(String),
    Transfer { bytes: u64, total_bytes: u64 },
    Finished(Result<(), String>),
}

fn explain(code: &str) -> String {
    match code {
        "not-authorized" => tr("shell-an-administrator-did-not-approve-this-change").into(),
        "review-stale"
        | "enrollment-changed"
        | "subordinate-ids-changed"
        | "bundle-file-changed" => {
            tr("shell-the-system-changed-since-the-review-review-again").into()
        }
        "request-expired-before-commit" => {
            tr("shell-the-approval-took-too-long-nothing-was-changed").into()
        }
        "sender-disconnected" | "sender-context-changed" => {
            tr("shell-the-change-was-stopped-because-settings-or-the-session-changed").into()
        }
        "package-manager-busy" | "transaction-busy" => {
            tr("shell-another-system-change-is-running-try-again-when-it-finishes").into()
        }
        "ordinary-account-required" => {
            tr("shell-developer-tools-are-set-up-for-ordinary-accounts-only").into()
        }
        "extension-not-installed" => {
            tr("shell-the-developer-extension-is-not-installed-on-this-system").into()
        }
        "extension-download-not-supported" => {
            tr("shell-this-developer-extension-does-not-support-download-only-acquisition").into()
        }
        "download-free-space-floor" => {
            tr("shell-there-is-not-enough-free-space-for-the-download-cache").into()
        }
        "download-integrity-refused" => {
            tr("shell-the-downloaded-files-could-not-be-verified-installation-was-not-started")
                .into()
        }
        "download-cancelled" | "download-deadline" => {
            tr("shell-the-download-stopped-verified-cache-files-are-kept-installation-was-not-star")
                .into()
        }
        "job-from-previous-boot" | "worker-ended-without-result" | "worker-start-refused" => {
            tr("shell-the-change-did-not-run-to-the-end-check-the-installed-tools-before-trying-ag")
                .into()
        }
        _ => {
            tr!("shell-the-developer-tools-service-refused-the-change", "arg0" => code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(48).collect::<String>())
        }
    }
}

async fn call(
    connection: &zbus::Connection,
    method: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    call_on(connection, SERVICE, method, body).await
}

async fn call_on(
    connection: &zbus::Connection,
    owner: &str,
    method: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let proxy = zbus::Proxy::new(connection, owner, PATH, SERVICE)
        .await
        .map_err(|_| tr("shell-developer-tools-are-not-installed"))?;
    let reply = match body {
        Some(body) => {
            tokio::time::timeout(
                Duration::from_secs(170),
                proxy.call_with_flags::<_, _, String>(
                    method,
                    zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                    &(body.to_string(),),
                ),
            )
            .await
        }
        None => {
            tokio::time::timeout(
                Duration::from_secs(20),
                proxy.call_with_flags::<_, _, String>(
                    method,
                    zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
                    &(),
                ),
            )
            .await
        }
    }
    .map_err(|_| {
        tr("shell-the-developer-tools-service-did-not-answer-the-outcome-may-be-unknown")
    })?;
    match reply {
        Ok(Some(text)) if text.len() <= 1024 * 1024 => serde_json::from_str(&text)
            .map_err(|_| tr("shell-invalid-reply-from-the-developer-tools-service").into()),
        Ok(_) => Err(tr("shell-invalid-reply-from-the-developer-tools-service").into()),
        Err(zbus::Error::MethodError(name, message, _))
            if name.as_str() == "org.enzimos.Developer1.Refused" =>
        {
            Err(explain(message.as_deref().unwrap_or_default()))
        }
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().contains("ServiceUnknown") => {
            Err(tr("shell-developer-tools-are-not-installed").into())
        }
        Err(_) => {
            Err(tr("shell-the-developer-tools-service-failed-the-outcome-may-be-unknown").into())
        }
    }
}

/// Detects a running or activatable service without starting it.
pub async fn available() -> bool {
    let Ok(system) = zbus::Connection::system().await else {
        return false;
    };
    let Ok(bus) = zbus::fdo::DBusProxy::new(&system).await else {
        return false;
    };
    bus.list_activatable_names()
        .await
        .is_ok_and(|names| names.iter().any(|name| name.as_str() == SERVICE))
        || bus
            .list_names()
            .await
            .is_ok_and(|names| names.iter().any(|name| name.as_str() == SERVICE))
}

/// The profiles and this account's state; `None` without the service.
pub async fn read() -> Result<Option<Overview>, String> {
    if !available().await {
        return Ok(None);
    }
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let owner = activate_owner(&system).await?;
    let capabilities = call_on(&system, &owner, "GetCapabilities", None).await?;
    let query = call_on(&system, &owner, "Query", None).await?;
    if capabilities["schema"] != 1 || query["schema"] != 1 {
        return Err(tr("shell-unsupported-developer-tools-service-schema").into());
    }
    Ok(Some(overview(&capabilities, &query)))
}

fn valid_profile(profile: &str) -> bool {
    matches!(
        profile,
        "build" | "runtimes" | "containers" | "virtualization"
    )
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(256)
        .map(str::to_owned)
        .collect()
}

/// Projects capabilities and enrollment in the fixed profile order.
#[must_use]
pub fn overview(capabilities: &Value, query: &Value) -> Overview {
    let order = ["build", "runtimes", "containers", "virtualization"];
    let mut profiles: Vec<Profile> = capabilities["profiles"]
        .as_object()
        .into_iter()
        .flatten()
        .take(16)
        .filter(|(id, _)| valid_profile(id))
        .map(|(id, profile)| Profile {
            id: id.clone(),
            label: profile["label"]
                .as_str()
                .unwrap_or(id)
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect(),
            packages: strings(&profile["packages"]),
            tools: strings(&profile["packages"])
                .into_iter()
                .map(|package| {
                    let status = match query["profiles"][id]["packages"].get(&package) {
                        Some(Value::Null) => PackageStatus::Missing,
                        Some(Value::String(version))
                            if !version.is_empty()
                                && version.len() <= 256
                                && version
                                    .chars()
                                    .all(|c| !c.is_control() && !c.is_whitespace()) =>
                        {
                            PackageStatus::Installed(version.clone())
                        }
                        _ => PackageStatus::Unknown,
                    };
                    Tool { package, status }
                })
                .collect(),
            installed: query["profiles"][id]["installed"].as_bool() == Some(true),
            enrolled: query["profiles"][id]["enrolled"].as_bool() == Some(true),
        })
        .collect();
    profiles.sort_by_key(|p| order.iter().position(|o| *o == p.id).unwrap_or(order.len()));
    let operations = strings(&capabilities["operations"]);
    Overview {
        profiles,
        ready: query["extension_available"].as_bool() == Some(true)
            && query["extension_compatible"].as_bool() == Some(true)
            && operations.iter().any(|o| o == "review")
            && operations.iter().any(|o| o == "install"),
        unavailable: query["unavailable_reason"].as_str().map(str::to_owned),
        removal: capabilities["package_removal"].as_bool() == Some(true)
            && operations.iter().any(|o| o == "remove")
            && operations.iter().any(|o| o == "review-removal"),
        download: query["extension_available"] == true
            && query["extension_compatible"] == true
            && query["extension_download_supported"] == true
            && operations.iter().any(|o| o == "review-download")
            && operations.iter().any(|o| o == "download"),
    }
}

async fn activate_owner(connection: &zbus::Connection) -> Result<String, String> {
    call(connection, "GetCapabilities", None).await?;
    let bus = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    bus.get_name_owner(
        SERVICE
            .try_into()
            .map_err(|_| tr("shell-invalid-developer-service-identity"))?,
    )
    .await
    .map(|owner| owner.to_string())
    .map_err(|_| tr("shell-developer-tools-service-is-unavailable").into())
}

/// Returns a plan bound to the current service owner; replacement requires a new review.
pub async fn review(operation: Operation, profile: String) -> Result<Review, String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    review_on(&system, operation, profile).await
}

async fn review_on(
    system: &zbus::Connection,
    operation: Operation,
    profile: String,
) -> Result<Review, String> {
    if !valid_profile(&profile) {
        return Err(tr("shell-unknown-developer-profile").into());
    }
    let owner = activate_owner(system).await?;
    let method = match operation {
        Operation::Install => "Review",
        Operation::Remove => "ReviewRemoval",
        Operation::Download => "ReviewDownload",
    };
    let reply = call_on(
        system,
        &owner,
        method,
        Some(json!({"schema": 1, "profiles": [profile]})),
    )
    .await?;
    let mut review = parse_review(operation, &profile, &reply)
        .ok_or_else(|| tr("shell-invalid-review-from-the-developer-tools-service"))?;
    review.owner = Some(owner);
    Ok(review)
}

#[must_use]
pub fn parse_review(operation: Operation, profile: &str, reply: &Value) -> Option<Review> {
    if !valid_profile(profile) {
        return None;
    }
    let sha = reply["plan_sha256"].as_str().filter(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })?;
    let plan = &reply["plan"];
    let expected = match operation {
        Operation::Install => "install",
        Operation::Remove => "remove",
        Operation::Download => "download",
    };
    if plan["schema"] != 1
        || plan["operation"] != expected
        || plan["profiles"] != json!([profile])
        || plan["removes_user_data"] != false
    {
        return None;
    }
    let safe = match operation {
        Operation::Install | Operation::Download => [
            "starts_network_services",
            "enables_linger",
            "adds_privileged_groups",
        ],
        Operation::Remove => [
            "removes_subordinate_ids",
            "removes_package_dependencies",
            "stops_running_work",
        ],
    };
    if safe.iter().any(|key| plan[key] != false) {
        return None;
    }
    if operation == Operation::Download
        && (plan["installed_packages_changed"] != false || plan["automatically_installs"] != false)
    {
        return None;
    }
    let packages = |key: &str| -> Option<Vec<String>> {
        let values = plan[key].as_array()?;
        if values.len() > 256 {
            return None;
        }
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|s| {
                        !s.is_empty()
                            && s.len() <= 128
                            && s.as_bytes()[0].is_ascii_alphanumeric()
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"@+_.:-".contains(&b))
                    })
                    .map(str::to_owned)
            })
            .collect()
    };
    let (packages, others) = match operation {
        Operation::Install | Operation::Download => (
            packages("new_packages")?,
            packages("new_dependency_packages")?,
        ),
        Operation::Remove => (
            packages("transaction_packages")?,
            packages("preserved_owned_packages")?,
        ),
    };
    let download_bytes = match operation {
        Operation::Install | Operation::Download => Some(plan["download_bytes"].as_u64()?),
        Operation::Remove => None,
    };
    if operation == Operation::Download && download_bytes? > 1024 * 1024 * 1024 {
        return None;
    }
    let installed = match (
        operation,
        &plan["installed_bytes_known"],
        plan["installed_bytes"].as_u64(),
    ) {
        (Operation::Install, Value::Bool(true), Some(bytes)) => Space::Known(bytes),
        (Operation::Install, Value::Bool(true), None) => return None,
        (Operation::Install, Value::Bool(false), _) => Space::Unknown,
        _ => Space::NotReported,
    };
    let free = |key: &str| reply["disk"][key].as_u64();
    Some(Review {
        operation,
        profile: profile.to_owned(),
        plan_sha256: sha.to_owned(),
        owner: None,
        packages,
        others,
        download_bytes,
        installed,
        free_bytes: free("free_bytes"),
        cache_free_bytes: free("cache_free_bytes"),
    })
}

fn request_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2)
        .map(|_| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// Starts once on the reviewing owner and retains a dedicated connection.
/// Dropping cancels cache acquisition or prevents a future commit, never rollback.
pub fn run(review: Review) -> impl iced::futures::Stream<Item = Progress> {
    run_with_connection(review, async {
        zbus::connection::Builder::system()
            .map_err(|_| tr("shell-no-system-bus").to_owned())?
            .build()
            .await
            .map_err(|_| tr("shell-no-system-bus").to_owned())
    })
}

fn run_with_connection(
    review: Review,
    connection: impl std::future::Future<Output = Result<zbus::Connection, String>>,
) -> impl iced::futures::Stream<Item = Progress> {
    use iced::futures::SinkExt;
    iced::stream::channel(
        8,
        async move |mut output: iced::futures::channel::mpsc::Sender<Progress>| {
            let result = async {
                let connection = connection.await?;
                let owner = review.owner.as_deref().ok_or_else(|| {
                    tr("shell-review-the-developer-profile-again-before-starting")
                })?;
                let bus = zbus::fdo::DBusProxy::new(&connection)
                    .await
                    .map_err(|_| tr("shell-no-system-bus"))?;
                let current = bus
                    .get_name_owner(
                        SERVICE
                            .try_into()
                            .map_err(|_| tr("shell-invalid-developer-service-identity"))?,
                    )
                    .await
                    .map_err(|_| tr("shell-developer-tools-service-is-unavailable-review-again"))?;
                if current.as_str() != owner {
                    return Err(tr("shell-developer-tools-service-changed-review-again").into());
                }
                let method = match review.operation {
                    Operation::Install => "Start",
                    Operation::Remove => "Remove",
                    Operation::Download => "Download",
                };
                let request = json!({
                    "schema": 1, "profiles": [review.profile], "plan_sha256": review.plan_sha256,
                    "request_id": request_id(), "allow_interaction": true, "valid_for_ms": 150000,
                });
                let job = call_on(&connection, owner, method, Some(request)).await?;
                let id = job["id"]
                    .as_str()
                    .filter(|id| {
                        id.len() == 32
                            && id
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    .ok_or_else(|| tr("shell-invalid-job-from-the-developer-tools-service"))?
                    .to_owned();
                let mut last = None;
                loop {
                    let status = call_on(
                        &connection,
                        owner,
                        "Status",
                        Some(json!({"schema": 1, "job": id})),
                    )
                    .await?;
                    let progress = parse_progress(&review, &status)?;
                    if let Progress::Finished(result) = progress {
                        return result;
                    }
                    if last.as_ref() != Some(&progress) {
                        last = Some(progress.clone());
                        if output.send(progress).await.is_err() {
                            return Err(tr("shell-settings-stopped-following-the-change").into());
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            .await;
            let _ = output.send(Progress::Finished(result)).await;
            std::future::pending::<()>().await;
        },
    )
}

fn parse_progress(review: &Review, status: &Value) -> Result<Progress, String> {
    let download = review.operation == Operation::Download;
    match status["state"].as_str().unwrap_or_default() {
        "completed" if download => {
            let result = &status["result"];
            if result["installed_packages_changed"] != false
                || result["automatically_installs"] != false
                || result["requires_fresh_install_review"] != true
                || result["profiles"] != json!([review.profile])
                || result["verified_bytes"].as_u64() != review.download_bytes
            {
                return Err(tr(
                    "shell-invalid-download-result-check-the-cache-before-trying-again",
                )
                .into());
            }
            Ok(Progress::Finished(Ok(())))
        }
        "completed" => Ok(Progress::Finished(Ok(()))),
        "cancelled" => Ok(Progress::Finished(Err(if download {
            tr("shell-the-download-was-cancelled-verified-cache-files-are-kept-installation-was-no")
                .into()
        } else {
            tr("shell-the-change-was-cancelled-before-commit").into()
        }))),
        "failed" => Ok(Progress::Finished(Err(status["failure"]
            .as_str()
            .map_or_else(
                || {
                    if download {
                        tr("shell-the-download-failed-installation-was-not-started").into()
                    } else {
                        tr("shell-the-change-failed-before-it-was-committed").into()
                    }
                },
                explain,
            )))),
        "uncertain" if !download => Ok(Progress::Finished(Err(tr(
            "shell-the-change-was-interrupted-while-committing-check-the-installed-tools-before",
        )
        .into()))),
        "queued" | "validating" => Ok(Progress::State(
            status["state"]
                .as_str()
                .expect("state matched a string literal above")
                .into(),
        )),
        "committing" | "configuring" if !download => Ok(Progress::State(
            status["state"]
                .as_str()
                .expect("state matched a string literal above")
                .into(),
        )),
        "downloading" if download => {
            if status["progress"].is_null() {
                return Ok(Progress::State("downloading".into()));
            }
            let bytes = status["progress"]["bytes"]
                .as_u64()
                .ok_or_else(|| tr("shell-invalid-download-byte-progress"))?;
            let total_bytes = status["progress"]["total_bytes"]
                .as_u64()
                .ok_or_else(|| tr("shell-invalid-download-byte-progress"))?;
            if total_bytes > 1024 * 1024 * 1024
                || Some(total_bytes) != review.download_bytes
                || bytes > total_bytes
            {
                return Err(
                    tr("shell-invalid-download-byte-progress-the-outcome-may-be-unknown").into(),
                );
            }
            Ok(Progress::Transfer { bytes, total_bytes })
        }
        _ => Err(tr("shell-invalid-job-state-the-outcome-may-be-unknown").into()),
    }
}

#[must_use]
pub fn state_label(state: &str) -> &'static str {
    match state {
        "queued" | "validating" => tr("shell-waiting-for-approval"),
        "downloading" => tr("shell-downloading"),
        "committing" => tr("shell-installing-packages"),
        "configuring" => tr("shell-setting-up"),
        _ => tr("shell-working-2"),
    }
}

/// A job state that can still be cancelled by closing its connection.
#[must_use]
pub fn cancellable(state: &str) -> bool {
    matches!(state, "" | "queued" | "validating" | "downloading")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_reply(profile: &str) -> Value {
        json!({"plan": {"schema":1, "operation":"install", "profiles":[profile], "new_packages":["podman"], "new_dependency_packages":["passt"], "download_bytes":1024, "starts_network_services":false, "enables_linger":false, "adds_privileged_groups":false, "removes_user_data":false}, "plan_sha256":"a".repeat(64)})
    }

    fn download_reply(profile: &str) -> Value {
        let mut reply = install_reply(profile);
        reply["plan"]["operation"] = json!("download");
        reply["plan"]["installed_packages_changed"] = json!(false);
        reply["plan"]["automatically_installs"] = json!(false);
        reply
    }

    struct Provider {
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, String, Value)>>>,
        state: String,
    }
    #[zbus::interface(name = "org.enzimos.Developer1")]
    impl Provider {
        fn get_capabilities(&self) -> String {
            json!({"schema":1}).to_string()
        }
        fn review(&self, body: String) -> String {
            install_reply(
                body.parse::<Value>().unwrap()["profiles"][0]
                    .as_str()
                    .unwrap(),
            )
            .to_string()
        }
        fn review_download(&self, body: String) -> String {
            download_reply(
                serde_json::from_str::<Value>(&body).unwrap()["profiles"][0]
                    .as_str()
                    .unwrap(),
            )
            .to_string()
        }
        fn download(
            &self,
            body: String,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> String {
            self.calls.lock().unwrap().push((
                "Download".into(),
                header.sender().unwrap().to_string(),
                serde_json::from_str(&body).unwrap(),
            ));
            json!({"id":"b".repeat(32)}).to_string()
        }
        fn start(&self, body: String, #[zbus(header)] header: zbus::message::Header<'_>) -> String {
            self.calls.lock().unwrap().push((
                "Start".into(),
                header.sender().unwrap().to_string(),
                serde_json::from_str(&body).unwrap(),
            ));
            json!({"id":"b".repeat(32)}).to_string()
        }
        fn status(
            &self,
            body: String,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> String {
            self.calls.lock().unwrap().push((
                "Status".into(),
                header.sender().unwrap().to_string(),
                serde_json::from_str(&body).unwrap(),
            ));
            let calls = self.calls.lock().unwrap();
            if calls.iter().any(|(method, _, _)| method == "Download") {
                json!({"state":self.state,"progress":{"bytes":512,"total_bytes":1024},"result":{"profiles":["containers"],"verified_bytes":1024,"installed_packages_changed":false,"automatically_installs":false,"requires_fresh_install_review":true}}).to_string()
            } else {
                json!({"state":self.state}).to_string()
            }
        }
    }

    #[tokio::test]
    async fn developer_tools_async_behaviour() {
        // Private bus transaction uses review owner one sender and exact intent.
        {
            use iced::futures::StreamExt;
            let bus = crate::test_bus::Bus::start();
            let calls = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
            let service = bus
                .serve(
                    SERVICE,
                    PATH,
                    Provider {
                        calls: std::sync::Arc::clone(&calls),
                        state: "completed".into(),
                    },
                )
                .await;
            let client = bus.client().await;
            let review = review_on(&client, Operation::Install, "containers".into())
                .await
                .unwrap();
            assert_eq!(
                review.owner.as_deref(),
                service.unique_name().map(|n| n.as_str())
            );
            let transaction = bus.client().await;
            let sender = transaction.unique_name().unwrap().to_string();
            let stream = run_with_connection(review, async { Ok(transaction) });
            let mut stream = std::pin::pin!(stream);
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), stream.next())
                    .await
                    .unwrap(),
                Some(Progress::Finished(Ok(())))
            );
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|(_, who, _)| who == &sender));
            assert_eq!(calls[0].0, "Start");
            let body = &calls[0].2;
            assert_eq!(body["profiles"], json!(["containers"]));
            assert_eq!(body["plan_sha256"], "a".repeat(64));
            assert_eq!(body["valid_for_ms"], 150000);
            assert_eq!(body["allow_interaction"], true);
            assert_eq!(body["request_id"].as_str().unwrap().len(), 32);
            assert_eq!(body.as_object().unwrap().len(), 6);
            assert_eq!(calls[1].2, json!({"schema":1,"job":"b".repeat(32)}));
        }
        // Dropping precommit stream disconnects transaction without replaying.
        {
            use iced::futures::StreamExt;
            let bus = crate::test_bus::Bus::start();
            let calls = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
            let _service = bus
                .serve(
                    SERVICE,
                    PATH,
                    Provider {
                        calls: std::sync::Arc::clone(&calls),
                        state: "validating".into(),
                    },
                )
                .await;
            let client = bus.client().await;
            let review = review_on(&client, Operation::Install, "runtimes".into())
                .await
                .unwrap();
            let transaction = bus.client().await;
            let sender = transaction.unique_name().unwrap().to_string();
            {
                let stream = run_with_connection(review, async { Ok(transaction) });
                let mut stream = std::pin::pin!(stream);
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(3), stream.next())
                        .await
                        .unwrap(),
                    Some(Progress::State("validating".into()))
                );
            }
            let proxy = zbus::fdo::DBusProxy::new(&client).await.unwrap();
            for _ in 0..100 {
                if !proxy
                    .name_has_owner(sender.as_str().try_into().unwrap())
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                !proxy
                    .name_has_owner(sender.as_str().try_into().unwrap())
                    .await
                    .unwrap()
            );
            assert_eq!(
                calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(method, _, _)| method == "Start")
                    .count(),
                1
            );
        }
    }
}
