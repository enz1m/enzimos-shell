//! Revision-bound preferences and status for OS local-AI services.
//! Conflicts and lost replies require a fresh read before another mutation.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

const MAX_REPLY: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    Inference,
    Gateway,
}

impl Service {
    fn directory(self) -> &'static str {
        match self {
            Self::Inference => "enzimos-inference",
            Self::Gateway => "enzimos-ai-gateway",
        }
    }
    pub fn socket(self) -> Option<PathBuf> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())?;
        Some(runtime.join(self.directory()).join("service.sock"))
    }
    fn name(self) -> &'static str {
        match self {
            Self::Inference => tr("shell-the-on-device-ai-service"),
            Self::Gateway => tr("shell-the-ai-provider-service"),
        }
    }
}

pub(crate) fn exchange(
    service: Service,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), Failure> {
    exchange_at(
        &service.socket().ok_or(Failure::Unavailable)?,
        method,
        path,
        body,
    )
}

pub(crate) fn exchange_with_timeout(
    service: Service,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<(u16, Value), Failure> {
    exchange_at_with_timeout(
        &service.socket().ok_or(Failure::Unavailable)?,
        method,
        path,
        body,
        timeout,
    )
}

fn exchange_at(
    socket: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), Failure> {
    exchange_at_with_timeout(socket, method, path, body, Duration::from_secs(10))
}

fn exchange_at_with_timeout(
    socket: &std::path::Path,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<(u16, Value), Failure> {
    let mut stream =
        crate::agent_harness::private_socket(socket).map_err(|_| Failure::Unavailable)?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let request = request_bytes(method, path, body);
    // Once a change is sent, a failure to hear back says nothing about it.
    let lost = if method == "POST" {
        Failure::ReplyLost
    } else {
        Failure::Unavailable
    };
    stream.write_all(&request).map_err(|_| lost.clone())?;
    let mut response = Vec::new();
    stream
        .take(MAX_REPLY)
        .read_to_end(&mut response)
        .map_err(|_| lost.clone())?;
    parse_response(&response).ok_or(lost)
}

fn request_bytes(method: &str, path: &str, body: Option<&Value>) -> Vec<u8> {
    let payload = body
        .map(|b| serde_json::to_vec(b).unwrap_or_default())
        .unwrap_or_default();
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n")
            .into_bytes();
    if body.is_some() {
        request.extend_from_slice(
            format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                payload.len()
            )
            .as_bytes(),
        );
    }
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(&payload);
    request
}

#[derive(Debug, Clone)]
/// Speed and sampled memory for a fixed local model workload.
pub enum Benchmark {
    Text {
        first_ms: f64,
        tokens_per_second: f64,
        memory_mib: f64,
    },
    Speech {
        audio_seconds: f64,
        elapsed_seconds: f64,
        memory_mib: f64,
    },
}

/// Measures a local model until the connection is dropped.
///
/// # Errors
/// Returns the service refusal or an unavailable or invalid response.
pub async fn benchmark(model: String) -> Result<Benchmark, String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let unavailable = || Failure::Unavailable.message(Service::Inference);
    let path = Service::Inference.socket().ok_or_else(unavailable)?;
    let socket = crate::agent_harness::private_socket(&path)?;
    socket.set_nonblocking(true).map_err(|_| unavailable())?;
    let mut socket = tokio::net::UnixStream::from_std(socket).map_err(|_| unavailable())?;
    let request = request_bytes(
        "POST",
        "/v1/models/benchmark",
        Some(&json!({"model": model})),
    );
    let (status, response) = tokio::time::timeout(Duration::from_secs(150), async {
        socket.write_all(&request).await.ok()?;
        let mut reply = Vec::new();
        socket.take(MAX_REPLY).read_to_end(&mut reply).await.ok()?;
        parse_response(&reply)
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(unavailable)?;
    let invalid = || Failure::Invalid.message(Service::Inference);
    if status != 200 {
        return Err(text(&response["error"], 240).unwrap_or_else(invalid));
    }
    let memory_mib = response["memory_mib"].as_f64().ok_or_else(invalid)?;
    if let Some(first_ms) = response["first_token_ms"].as_f64() {
        let tokens_per_second = response["tokens_per_second"].as_f64().ok_or_else(invalid)?;
        Ok(Benchmark::Text {
            first_ms,
            tokens_per_second,
            memory_mib,
        })
    } else {
        let audio_seconds = response["audio_seconds"].as_f64().ok_or_else(invalid)?;
        let elapsed_seconds = response["elapsed_seconds"].as_f64().ok_or_else(invalid)?;
        Ok(Benchmark::Speech {
            audio_seconds,
            elapsed_seconds,
            memory_mib,
        })
    }
}

fn parse_response(response: &[u8]) -> Option<(u16, Value)> {
    let text = std::str::from_utf8(response).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    if head.to_ascii_lowercase().contains("transfer-encoding:") {
        return None;
    }
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let value = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body.trim()).ok()?
    };
    Some((status, value))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Not installed, not running, or not private to this account.
    Unavailable,
    /// The service answered something Settings does not understand.
    Invalid,
    /// Running work or a newer revision: read again, review, try again.
    Conflict,
    /// The service refused the change; its message.
    Refused(String),
    /// The change was sent but no reply came: it may or may not have applied.
    ReplyLost,
    /// The service runs without saved preferences, so nothing can change.
    ReadOnly,
}

impl Failure {
    pub fn message(&self, service: Service) -> String {
        match self {
            Self::Unavailable => tr!("shell-is-not-running", "arg0" => service.name().to_owned()),
            Self::Invalid => tr!("shell-answered-in-a-form-settings-does-not-understand", "arg0" => service.name().to_owned()),
            Self::Conflict => tr("shell-something-changed-meanwhile-work-in-progress-or-another-change-the-current-v").into(),
            Self::Refused(why) => tr!("shell-refused-the-change", "arg0" => service.name().to_owned(), "arg1" => why.clone()),
            Self::ReplyLost => tr("shell-no-answer-came-after-the-change-was-sent-so-it-may-or-may-not-have-applied-t").into(),
            Self::ReadOnly => tr!("shell-was-started-without-saved-preferences-so-they-cannot-be-changed-here", "arg0" => service.name().to_owned()),
        }
    }
}

/// The inference service's preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferencePreferences {
    /// `None`: as many as policy allows (at most 2).
    pub cpu_threads: Option<u8>,
    pub memory_mib: u32,
    pub idle_seconds: u32,
    pub context: u32,
    pub backend: String,
    pub default_model: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inference {
    pub revision: String,
    pub persistent: bool,
    pub preferences: InferencePreferences,
    /// Registered text models.
    pub models: Vec<String>,
    /// Registered speech-to-text models, which dictation uses.
    pub speech_models: Vec<String>,
    pub model_titles: std::collections::BTreeMap<String, String>,
    /// The speech model the user chose for dictation, when they chose one.
    pub speech_default: Option<String>,
    pub loaded_model: Option<String>,
    pub backend: Option<String>,
    pub jobs: u32,
    pub failure: Option<String>,
    pub peak_rss_mib: Option<u64>,
    /// Engines installed on this system, as the service names them; empty
    /// from a service that does not report them.
    pub installed_runtimes: Vec<String>,
}

impl Inference {
    /// The speech model dictation uses: the one the user chose, else Whisper
    /// base, else the first installed (as `enzimos-dictation` decides).
    #[must_use]
    pub fn voice_model(&self) -> Option<&str> {
        self.speech_default
            .as_deref()
            .filter(|id| self.speech_models.iter().any(|m| m == id))
            .or_else(|| {
                self.speech_models
                    .iter()
                    .find(|m| *m == "whisper-base")
                    .map(String::as_str)
            })
            .or_else(|| self.speech_models.first().map(String::as_str))
    }
}

/// A model available from the installed model registry tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOffer {
    pub id: String,
    pub title: String,
    /// As the tool prints it, such as `487 MB`.
    pub size: String,
    pub description: String,
    pub speech: bool,
    pub needs_runtime: bool,
    pub added: bool,
}

/// Decodes the model registry catalog, including speech-only catalogs.
#[must_use]
pub fn parse_catalog(text: &str) -> Vec<ModelOffer> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let (id, title, size, description, state, speech) = match fields.as_slice() {
                [id, title, size, description, state]
                | [id, title, size, description, state, "speech"] => {
                    (*id, *title, *size, *description, *state, true)
                }
                [id, title, size, description, state, "text"] => {
                    (*id, *title, *size, *description, *state, false)
                }
                _ => return None,
            };
            let clean = |s: &str| {
                (!s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
                    .then(|| s.to_owned())
            };
            Some(ModelOffer {
                id: clean(id)?,
                title: clean(title)?,
                size: clean(size)?,
                description: clean(description)?,
                speech,
                needs_runtime: state == "requires-runtime",
                added: match state {
                    "added" => true,
                    "available" | "requires-runtime" => false,
                    _ => return None,
                },
            })
        })
        .take(16)
        .collect()
}

pub const MODELS_TOOL: &str = "/usr/bin/enzimos-inference-models";

pub fn read_model_catalog() -> Vec<ModelOffer> {
    std::process::Command::new(MODELS_TOOL)
        .arg("catalog")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(Vec::new, |output| {
            parse_catalog(&String::from_utf8_lossy(&output.stdout))
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub id: String,
    pub kind: String,
    pub enabled: bool,
    /// Can be the default: enabled and able to chat.
    pub selectable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gateway {
    pub revision: String,
    pub persistent: bool,
    pub default_provider: String,
    pub providers: Vec<Provider>,
}

fn text(value: &Value, max: usize) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
}

fn snapshot(value: &Value) -> Option<(String, bool, &serde_json::Map<String, Value>)> {
    if value["schema_version"] != json!(1) {
        return None;
    }
    let revision = value["revision"]
        .as_str()
        .filter(|r| r.len() == 64 && r.bytes().all(|b| b.is_ascii_hexdigit()))?
        .to_owned();
    Some((
        revision,
        value["persistent"].as_bool()?,
        value["preferences"].as_object()?,
    ))
}

pub fn parse_inference(preferences: &Value, status: &Value, models: &Value) -> Option<Inference> {
    let (revision, persistent, p) = snapshot(preferences)?;
    let number = |key: &str| p.get(key)?.as_u64().and_then(|n| u32::try_from(n).ok());
    let cpu_threads = match p.get("cpu_threads")? {
        Value::Null => None,
        value => Some(value.as_u64().filter(|n| (1..=2).contains(n))? as u8),
    };
    Some(Inference {
        revision,
        persistent,
        preferences: InferencePreferences {
            cpu_threads,
            memory_mib: number("memory_mib")?,
            idle_seconds: number("idle_seconds")?,
            context: number("context")?,
            backend: text(p.get("backend")?, 64)?,
            default_model: match p.get("default_model")? {
                Value::Null => String::new(),
                value => text(value, 128)?,
            },
        },
        models: models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["kind"].as_str().unwrap_or("llm") == "llm")
            .filter_map(|m| text(&m["id"], 128))
            .take(64)
            .collect(),
        speech_models: models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["kind"] == "speech")
            .filter_map(|m| text(&m["id"], 128))
            .take(16)
            .collect(),
        model_titles: models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .take(80)
            .filter_map(|m| Some((text(&m["id"], 128)?, text(&m["title"], 128)?)))
            .collect(),
        speech_default: models["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["kind"] == "speech" && m["default"] == json!(true))
            .and_then(|m| text(&m["id"], 128)),
        loaded_model: text(&status["loaded_model"], 128),
        backend: text(&status["backend"], 64),
        jobs: status["jobs"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0),
        failure: status["failure"]
            .as_str()
            .map(|f| f.chars().take(300).collect()),
        peak_rss_mib: status["peak_worker_rss_mib"].as_u64().filter(|n| *n > 0),
        installed_runtimes: status["installed_runtimes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| RUNTIMES.contains(id))
            .take(4)
            .map(str::to_owned)
            .collect(),
    })
}

const RUNTIMES: [&str; 4] = ["cpu", "vulkan", "sycl", "cuda"];

pub fn parse_gateway(preferences: &Value, providers: &Value) -> Option<Gateway> {
    let (revision, persistent, p) = snapshot(preferences)?;
    Some(Gateway {
        revision,
        persistent,
        default_provider: text(p.get("default_provider")?, 64)?,
        providers: providers["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let kind = text(&row["kind"], 32)?;
                let enabled = row["enabled"].as_bool().unwrap_or(kind == "local");
                let chat = kind == "local"
                    || row["capabilities"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|c| c == "chat"));
                Some(Provider {
                    id: text(&row["id"], 64)?,
                    selectable: enabled && (chat || kind == "chatgpt-login"),
                    kind,
                    enabled,
                })
            })
            .take(32)
            .collect(),
    })
}

/// Reads one service's status and preferences.
pub fn read_inference() -> Result<Inference, Failure> {
    let get = |path: &str| match exchange(Service::Inference, "GET", path, None)? {
        (200, value) => Ok(value),
        _ => Err(Failure::Invalid),
    };
    parse_inference(
        &get("/v1/preferences")?,
        &get("/status")?,
        &get("/v1/models")?,
    )
    .ok_or(Failure::Invalid)
}

pub fn read_gateway() -> Result<Gateway, Failure> {
    let get = |path: &str| match exchange(Service::Gateway, "GET", path, None)? {
        (200, value) => Ok(value),
        _ => Err(Failure::Invalid),
    };
    parse_gateway(&get("/v1/preferences")?, &get("/v1/providers")?).ok_or(Failure::Invalid)
}

/// The changes the services accept, checked here as they check them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    CpuThreads(Option<u8>),
    MemoryMib(u32),
    IdleSeconds(u32),
    Context(u32),
    Backend(String),
    DefaultModel(String),
    DefaultProvider(String),
}

impl Change {
    pub fn service(&self) -> Service {
        if matches!(self, Self::DefaultProvider(_)) {
            Service::Gateway
        } else {
            Service::Inference
        }
    }
    /// The `preferences` patch, or why the value is out of policy.
    pub fn patch(&self) -> Result<Value, &'static str> {
        Ok(match self {
            Self::CpuThreads(None) => json!({"cpu_threads": null}),
            Self::CpuThreads(Some(n @ 1..=2)) => json!({"cpu_threads": n}),
            Self::CpuThreads(_) => {
                return Err(tr("shell-processor-threads-are-1-2-or-automatic"));
            }
            Self::MemoryMib(n @ 256..=2048) => json!({"memory_mib": n}),
            Self::MemoryMib(_) => {
                return Err(tr("shell-memory-is-between-256-and-2048-mb"));
            }
            Self::IdleSeconds(n @ 1..=3600) => json!({"idle_seconds": n}),
            Self::IdleSeconds(_) => {
                return Err(tr("shell-unloading-waits-between-1-second-and-1-hour"));
            }
            Self::Context(n @ 4096..=8192) => json!({"context": n}),
            Self::Context(_) => {
                return Err(tr("shell-context-is-between-4096-and-8192-tokens"));
            }
            Self::Backend(name) if !name.is_empty() && name.len() <= 64 => json!({"backend": name}),
            Self::DefaultModel(name) if !name.is_empty() && name.len() <= 128 => {
                json!({"default_model": name})
            }
            Self::DefaultProvider(name) if !name.is_empty() && name.len() <= 64 => {
                json!({"default_provider": name})
            }
            _ => {
                return Err(tr("shell-choose-one-of-the-offered-values"));
            }
        })
    }
}

/// Applies one change against `revision`; `Ok` carries the new revision.
pub fn apply(change: &Change, revision: &str, persistent: bool) -> Result<String, Failure> {
    let socket = change.service().socket().ok_or(Failure::Unavailable)?;
    apply_at(&socket, change, revision, persistent)
}

fn apply_at(
    socket: &std::path::Path,
    change: &Change,
    revision: &str,
    persistent: bool,
) -> Result<String, Failure> {
    if !persistent {
        return Err(Failure::ReadOnly);
    }
    let patch = change.patch().map_err(|why| Failure::Refused(why.into()))?;
    let body = json!({"schema_version": 1, "revision": revision, "preferences": patch});
    match exchange_at(socket, "POST", "/v1/preferences", Some(&body))? {
        (200, value) => snapshot(&value)
            .map(|(revision, ..)| revision)
            .ok_or(Failure::ReplyLost),
        (409, _) => Err(Failure::Conflict),
        (_, value) => Err(Failure::Refused(value["error"].as_str().map_or_else(
            || tr("shell-it-gave-no-reason").to_owned(),
            |e| e.chars().take(200).collect(),
        ))),
    }
}

fn disabled_path() -> Option<PathBuf> {
    crate::welcome::record_path()?
        .parent()
        .map(|config| config.join("local-ai-disabled"))
}

pub fn enabled() -> bool {
    disabled_path().is_none_or(|path| !path.exists())
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path =
        disabled_path().ok_or_else(|| tr("shell-model-configuration-unavailable").to_owned())?;
    let write_marker = |enable: bool| -> std::io::Result<()> {
        if enable {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            if let Some(config) = path.parent() {
                std::fs::create_dir_all(config)?;
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&path)
                .map(|_| ())
        }
    };
    let was_enabled = !path.exists();
    write_marker(enabled).map_err(|error| error.to_string())?;
    let units = ["enzimos-inference.service", "enzimos-dictation.service"]
        .into_iter()
        .filter(|unit| {
            std::path::Path::new("/usr/lib/systemd/user")
                .join(unit)
                .is_file()
        })
        .collect::<Vec<_>>();
    if units.is_empty() {
        return Ok(());
    }
    let result = std::process::Command::new("/usr/bin/systemctl")
        .args([
            "--user",
            "--no-block",
            if enabled { "start" } else { "stop" },
        ])
        .args(units)
        .stdin(std::process::Stdio::null())
        .output();
    if result.as_ref().is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    let _ = write_marker(was_enabled);
    Err(tr("shell-model-service-change-failed").into())
}

/// The gateway's provider configuration: `~/.config/enzimos/ai-providers.json`.
#[must_use]
pub fn providers_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())?;
    Some(home.join(".config/enzimos/ai-providers.json"))
}

/// The gateway's configuration as written: the on-device runtime only.
#[must_use]
pub fn starter_providers() -> Value {
    let socket = Service::Inference
        .socket()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    json!({"default_provider": "local", "providers": {"local": {"kind": "local", "socket": socket}}})
}

/// An online provider the user adds; the key stays in a private file the
/// gateway reads, never in the configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct Online {
    pub name: String,
    pub base_url: String,
    pub key: Option<String>,
}

impl std::fmt::Debug for Online {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Online")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("key", &self.key.is_some())
            .finish()
    }
}

/// Well-known OpenAI-compatible endpoints: (name, base URL).
pub const PRESETS: [(&str, &str); 3] = [
    ("openrouter", "https://openrouter.ai/api/v1"),
    ("openai", "https://api.openai.com/v1"),
    ("groq", "https://api.groq.com/openai/v1"),
];

/// Provider names: 1 to 32 of `a-z`, `0-9` and `-`, starting with a letter.
#[must_use]
pub fn provider_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// An endpoint the gateway accepts: HTTPS, or HTTP on this computer.
#[must_use]
pub fn endpoint(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| {
        url.strip_prefix("http://").filter(|r| {
            ["127.0.0.1", "localhost", "[::1]"]
                .iter()
                .any(|h| r.starts_with(h))
        })
    });
    rest.is_some_and(|r| {
        !r.is_empty() && !r.contains(['?', '#', '@', ' ']) && url.is_ascii() && url.len() <= 512
    })
}

/// A change to the gateway's providers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderEdit {
    AddOnline(Online),
    /// The ChatGPT sign-in provider, named `chatgpt`.
    AddChatGpt,
    Remove(String),
}

/// Applies one edit to a provider configuration; returns the credential
/// file to write (path, key) or remove (path, None).
pub fn edit_providers(
    config: &mut Value,
    edit: &ProviderEdit,
    credentials: &std::path::Path,
) -> Result<Option<(PathBuf, Option<String>)>, String> {
    if !config["providers"].is_object() {
        config["providers"] = json!({});
    }
    match edit {
        ProviderEdit::AddOnline(online) => {
            if !provider_name(&online.name) || online.name == "local" {
                return Err(
                    tr("shell-provider-names-are-lowercase-letters-digits-and-hyphens").into(),
                );
            }
            if !endpoint(&online.base_url) {
                return Err(tr("shell-the-address-must-start-with-https").into());
            }
            let key = online
                .key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty());
            if key.is_some_and(|k| k.len() > 4096 || !k.bytes().all(|c| (33..=126).contains(&c))) {
                return Err(tr("shell-the-key-is-not-valid").into());
            }
            let file = credentials.join(format!("{}.key", online.name));
            let mut provider = json!({"kind": "openai-compatible", "enabled": true,
                "base_url": online.base_url.trim_end_matches('/'), "capabilities": ["chat"]});
            if key.is_some() {
                provider["credential_file"] = json!(file.to_string_lossy());
            }
            config["providers"][&online.name] = provider;
            Ok(key.map(|key| (file, Some(key.to_owned()))))
        }
        ProviderEdit::AddChatGpt => {
            config["providers"]["chatgpt"] = json!({"kind": "chatgpt-login", "enabled": true});
            Ok(None)
        }
        ProviderEdit::Remove(name) => {
            if name == "local" {
                return Err(tr("shell-the-on-device-provider-stays").into());
            }
            let removed = config["providers"]
                .as_object_mut()
                .and_then(|providers| providers.remove(name))
                .ok_or_else(|| {
                    tr("shell-that-provider-is-not-approved-in-this-configuration").to_owned()
                })?;
            if config["default_provider"] == json!(name) {
                config["default_provider"] = json!("local");
            }
            Ok(removed["credential_file"]
                .as_str()
                .map(|file| (PathBuf::from(file), None)))
        }
    }
}

fn write_private(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let parent = path
        .parent()
        .ok_or_else(|| tr("shell-no-configuration-folder").to_owned())?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|_| tr("shell-the-configuration-folder-cannot-be-created").to_owned())?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| tr("shell-the-configuration-folder-cannot-be-made-private").to_owned())?;
    let temporary = path.with_extension("new");
    let _ = std::fs::remove_file(&temporary);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    let _ = std::fs::remove_file(&temporary);
    written.map_err(|_| tr("shell-the-configuration-cannot-be-written").to_owned())
}

/// Applies `edit` to the gateway's configuration file (written first when
/// missing), keeps the agent harness's provider list in step, and restarts
/// the gateway so it reads the change.
pub fn change_providers(edit: &ProviderEdit) -> Result<(), String> {
    let path = providers_path().ok_or_else(|| tr("shell-no-home-folder").to_owned())?;
    let mut config = match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice::<Value>(&raw)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| tr("shell-the-assistant-configuration-is-not-valid-json").to_owned())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => starter_providers(),
        Err(_) => return Err(tr("shell-the-configuration-cannot-be-written").into()),
    };
    let credentials = path.with_file_name("credentials");
    let credential = edit_providers(&mut config, edit, &credentials)?;
    match &credential {
        Some((file, Some(key))) => write_private(file, key.as_bytes())?,
        Some((file, None)) => {
            let _ = std::fs::remove_file(file);
        }
        None => {}
    }
    let text = serde_json::to_vec_pretty(&config)
        .map_err(|_| tr("shell-the-configuration-cannot-be-written").to_owned())?;
    write_private(&path, &text)?;
    if let Ok(Some(mut harness)) = crate::agent_harness::load() {
        let names = config["providers"]
            .as_object()
            .map(|p| p.keys().map(|k| json!(k)).collect::<Vec<_>>())
            .unwrap_or_default();
        if harness["providers"] != json!(names) {
            harness["providers"] = json!(names);
            if !names.contains(&harness["default_provider"]) {
                harness["default_provider"] = json!("local");
            }
            crate::agent_harness::save(&harness)?;
            restart("enzimos-agent-harness.service");
        }
    }
    restart("enzimos-ai-gateway.service");
    Ok(())
}

/// Restarts a user service if it runs; a stopped one reads the change when it starts.
fn restart(unit: &str) {
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "try-restart", unit])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// The ChatGPT account as the gateway reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatGpt {
    pub signed_in: bool,
    pub email: Option<String>,
    pub waiting: bool,
    pub error: Option<String>,
}

pub fn chatgpt() -> Result<ChatGpt, Failure> {
    match exchange(Service::Gateway, "GET", "/v1/accounts/chatgpt", None)? {
        (200, value) => Ok(ChatGpt {
            signed_in: value["signed_in"] == json!(true),
            email: text(&value["email"], 254),
            waiting: value["waiting"] == json!(true),
            error: text(&value["error"], 300),
        }),
        _ => Err(Failure::Invalid),
    }
}

/// Starts a ChatGPT sign-in; returns the page to open in the browser.
pub fn chatgpt_sign_in() -> Result<String, Failure> {
    match exchange(
        Service::Gateway,
        "POST",
        "/v1/accounts/chatgpt/login",
        Some(&json!({"provider": "chatgpt"})),
    )? {
        (200, value) => value["authorize_url"]
            .as_str()
            .filter(|url| url.starts_with("https://") || url.starts_with("http://127.0.0.1"))
            .map(str::to_owned)
            .ok_or(Failure::Invalid),
        (_, value) => Err(Failure::Refused(
            value["error"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
        )),
    }
}

pub fn chatgpt_sign_out() -> Result<(), Failure> {
    match exchange(
        Service::Gateway,
        "POST",
        "/v1/accounts/chatgpt/logout",
        Some(&json!({"provider": "chatgpt"})),
    )? {
        (200, _) => Ok(()),
        (_, value) => Err(Failure::Refused(
            value["error"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
        )),
    }
}
