//! Configuration and private-socket client for the enzimOS agent harness.
//! The harness owns sessions, tool execution and approval decisions.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_FRAME: usize = 1 << 20;
const MAX_CONFIG: u64 = 65536;
pub const UNIT: &str = "enzimos-agent-harness.service";

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[must_use]
pub fn config_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| Some(home()?.join(".config")))?;
    Some(config.join("enzimos/agent-harness.json"))
}

#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())?;
    Some(runtime.join("enzimos-agent-harness/service.sock"))
}

/// Whether the harness is installed on this system.
#[must_use]
pub fn installed() -> bool {
    Path::new("/usr/bin/enzimos-agent-harness").is_file()
}

/// A new configuration: the local provider only, no workspace, no action
/// without asking, no MCP server.
#[must_use]
pub fn starter() -> Value {
    let uid = rustix::process::getuid().as_raw();
    let skills = home()
        .map(|home| {
            home.join(".config/enzimos/skills")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default();
    json!({
        "gateway_socket": format!("/run/user/{uid}/enzimos-ai-gateway/service.sock"),
        "providers": ["local"],
        "default_provider": "local",
        "profiles": {"default": {"workspace_roots": []}},
        "skill_directories": [skills],
        "writable_skill_directories": [skills],
        "mcp_servers": {}
    })
}

/// The stored configuration, `None` when there is none yet.
pub fn load() -> Result<Option<Value>, String> {
    let path = config_path().ok_or_else(|| tr("shell-no-home-folder"))?;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(tr("shell-the-assistant-configuration-cannot-be-read").into());
        }
    };
    let metadata = file
        .metadata()
        .map_err(|_| tr("shell-the-assistant-configuration-cannot-be-read"))?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_CONFIG
    {
        return Err(
            tr("shell-the-assistant-configuration-must-be-a-private-file-owned-by-you").into(),
        );
    }
    let mut text = String::new();
    file.take(MAX_CONFIG + 1)
        .read_to_string(&mut text)
        .map_err(|_| tr("shell-the-assistant-configuration-cannot-be-read"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| tr("shell-the-assistant-configuration-is-not-valid-json"))?;
    value
        .is_object()
        .then_some(Some(value))
        .ok_or_else(|| tr("shell-the-assistant-configuration-is-not-an-object").into())
}

/// Writes the configuration atomically, private to this user.
pub fn save(value: &Value) -> Result<(), String> {
    let path = config_path().ok_or_else(|| tr("shell-no-home-folder"))?;
    let home = home().ok_or_else(|| tr("shell-no-home-folder"))?;
    save_at(&path, &home, value)
}

fn save_at(path: &Path, home: &Path, value: &Value) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| tr("shell-no-configuration-folder"))?;
    std::fs::create_dir_all(directory)
        .map_err(|_| tr("shell-the-configuration-folder-cannot-be-created"))?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| tr("shell-the-configuration-folder-cannot-be-made-private"))?;
    let text = serde_json::to_vec_pretty(value)
        .map_err(|_| tr("shell-the-configuration-cannot-be-written"))?;
    if text.len() as u64 > MAX_CONFIG {
        return Err(tr("shell-the-configuration-is-too-large").into());
    }
    // The service refuses skill folders that do not exist; one in the home
    // folder is created, private.
    for folder in value["skill_directories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let folder = Path::new(folder);
        if !folder.is_absolute() {
            return Err(tr("shell-a-skill-folder-cannot-be-created").into());
        }
        if folder.starts_with(home) && !folder.exists() {
            let ancestor = folder
                .ancestors()
                .find(|path| path.exists())
                .and_then(|path| path.canonicalize().ok());
            let home = home
                .canonicalize()
                .map_err(|_| tr("shell-a-skill-folder-cannot-be-created"))?;
            if folder
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
                || !ancestor.is_some_and(|path| path.starts_with(&home))
            {
                return Err(tr("shell-a-skill-folder-cannot-be-created").into());
            }
            std::fs::create_dir_all(folder)
                .map_err(|_| tr("shell-a-skill-folder-cannot-be-created"))?;
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| tr("shell-a-skill-folder-cannot-be-made-private"))?;
        }
        if !std::fs::metadata(folder).is_ok_and(|m| m.is_dir()) {
            return Err(tr("shell-a-skill-folder-cannot-be-created").into());
        }
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| tr("shell-the-configuration-cannot-be-written"))?;
    temporary
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|_| tr("shell-the-configuration-cannot-be-written"))?;
    temporary
        .write_all(&text)
        .map_err(|_| tr("shell-the-configuration-cannot-be-written"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| tr("shell-the-configuration-cannot-be-written"))?;
    temporary
        .persist(path)
        .map(drop)
        .map_err(|_| tr("shell-the-configuration-cannot-be-written").to_owned())?;
    std::fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| tr("shell-configuration-published-durability-unknown").to_owned())?;
    Ok(())
}

/// The tool approval mode requested by the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalMode {
    #[default]
    Ask,
    WorkspaceEdits,
    FullSend,
}

impl ApprovalMode {
    /// The modes offered in the approval selector.
    pub const ALL: [Self; 3] = [Self::Ask, Self::WorkspaceEdits, Self::FullSend];

    /// Decode a service value, using Ask for missing or unrecognized values.
    #[must_use]
    pub fn read(value: &Value) -> Self {
        match value.as_str() {
            Some("workspace_edits") => Self::WorkspaceEdits,
            Some("full_send") => Self::FullSend,
            _ => Self::Ask,
        }
    }

    /// Stable key shared by the preference file and harness protocol.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::WorkspaceEdits => "workspace_edits",
            Self::FullSend => "full_send",
        }
    }

    /// Localized label for Settings and chat controls.
    #[must_use]
    pub fn label(self) -> &'static str {
        tr(match self {
            Self::Ask => "shell-agent-approval-ask",
            Self::WorkspaceEdits => "shell-agent-approval-edits",
            Self::FullSend => "shell-agent-approval-full",
        })
    }
}

/// Preference for a new conversation.
#[must_use]
pub fn approval_default() -> ApprovalMode {
    load().ok().flatten().map_or(ApprovalMode::Ask, |config| {
        summary(&config).approval_default
    })
}

/// Live screenshot sharing preference for a conversation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screenshots {
    /// The model has no screenshot tool.
    #[default]
    Off,
    /// Send a text summary.
    Text,
    /// Include the image in the model request.
    Vision,
}

impl Screenshots {
    /// The available screenshot choices.
    pub const ALL: [Self; 3] = [Self::Off, Self::Text, Self::Vision];

    /// Read the service setting, defaulting to Off.
    #[must_use]
    pub fn read(value: &Value) -> Self {
        match value.as_str() {
            Some("text") => Self::Text,
            Some("vision") => Self::Vision,
            _ => Self::Off,
        }
    }

    /// Stable key in the harness protocol.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Text => "text",
            Self::Vision => "vision",
        }
    }

    /// Localized label for the screenshot selector.
    #[must_use]
    pub fn label(self) -> &'static str {
        tr(match self {
            Self::Off => "shell-agent-screenshots-off",
            Self::Text => "shell-agent-screenshots-text",
            Self::Vision => "shell-agent-screenshots-vision",
        })
    }
}

/// One change to the configuration; unknown keys are kept as they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Provider(String),
    Model(String),
    AddRoot(String),
    RemoveRoot(String),
    ApprovalDefault(ApprovalMode),
    AddSkills(String),
    RemoveSkills(String),
    ServerEnabled(String, bool),
    RemoveServer(String),
    AddServer(String, Server),
}

/// A tool server the user adds. The harness runs a program with the user's
/// own authority, so only an explicit absolute executable is accepted.
#[derive(Clone, PartialEq, Eq)]
pub enum Server {
    Program {
        argv: Vec<String>,
    },
    Web {
        url: String,
        authorization: Option<String>,
    },
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Program { argv } => f
                .debug_struct("Program")
                .field("argv", &argv.len())
                .finish(),
            Self::Web { authorization, .. } => f
                .debug_struct("Web")
                .field("authorization", &authorization.is_some())
                .finish(),
        }
    }
}

/// The harness's server-name rule: 1 to 24 of `a-z`, `0-9` and `_`, starting with a letter.
#[must_use]
pub fn server_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=24).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn checked_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let refuse = || tr("shell-use-an-https-address-or-http-on-127-0-0-1-or-1").to_owned();
    if url.len() > 4096 || url.chars().any(char::is_control) || url.contains(char::is_whitespace) {
        return Err(refuse());
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(refuse)?;
    if rest.contains('#') {
        return Err(tr("shell-the-address-must-not-contain-a-fragment").into());
    }
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(tr("shell-put-credentials-in-the-authorization-field-not-the-address").into());
    }
    let host = match authority.strip_prefix('[') {
        Some(inner) => inner
            .split_once(']')
            .map(|(host, _)| host)
            .ok_or_else(refuse)?,
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(url.to_owned()),
        "http" if host == "127.0.0.1" || host == "::1" => Ok(url.to_owned()),
        _ => Err(refuse()),
    }
}

fn server_value(server: &Server) -> Result<Value, String> {
    match server {
        Server::Program { argv } => {
            let program = argv
                .first()
                .ok_or_else(|| tr("shell-choose-the-program-to-run"))?;
            if !program.starts_with('/') {
                return Err(tr("shell-use-the-program-s-absolute-path").into());
            }
            if argv.len() > 64
                || argv
                    .iter()
                    .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            {
                return Err(tr("shell-up-to-64-arguments-of-4096-characters").into());
            }
            Ok(json!({"enabled": true, "transport": "stdio", "argv": argv}))
        }
        Server::Web { url, authorization } => {
            let mut value = json!({"enabled": true, "transport": "http", "url": checked_url(url)?});
            if let Some(token) = authorization
                .as_deref()
                .map(str::trim)
                .filter(|token| !token.is_empty())
            {
                if token.len() > 4096 || token.chars().any(|c| (c as u32) < 32 || c == '\u{7f}') {
                    return Err(tr(
                        "shell-the-authorization-must-be-one-line-of-up-to-4096-characters",
                    )
                    .into());
                }
                value["authorization"] = json!(token);
            }
            Ok(value)
        }
    }
}

fn absolute_directory(path: &str) -> Result<String, String> {
    let path = path.trim();
    if !Path::new(path).is_absolute() || path.len() > 4096 || path.contains('\0') {
        return Err(tr("shell-use-an-absolute-folder-path").into());
    }
    let trimmed = path.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.to_owned()
    })
}

fn strings<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>, String> {
    if !value[key].is_array() {
        value[key] = json!([]);
    }
    value[key]
        .as_array_mut()
        .ok_or_else(|| tr("shell-invalid-configuration").to_owned())
}

pub fn apply(config: &mut Value, edit: &Edit) -> Result<(), String> {
    match edit {
        Edit::Provider(name) => {
            if !config["providers"]
                .as_array()
                .is_some_and(|known| known.iter().any(|p| p.as_str() == Some(name)))
            {
                return Err(tr("shell-that-provider-is-not-approved-in-this-configuration").into());
            }
            config["default_provider"] = json!(name);
        }
        Edit::Model(model) => {
            let model = model.trim();
            if model.len() > 128 || model.chars().any(char::is_control) {
                return Err(tr("shell-model-names-up-to-128-characters").into());
            }
            config["enzui"]["model"] = json!(model);
        }
        Edit::AddRoot(path) | Edit::RemoveRoot(path) => {
            let path = absolute_directory(path)?;
            if !config["profiles"]["default"].is_object() {
                config["profiles"]["default"] = json!({});
            }
            let profile = &mut config["profiles"]["default"];
            let roots = strings(profile, "workspace_roots")?;
            roots.retain(|root| root.as_str() != Some(path.as_str()));
            if matches!(edit, Edit::AddRoot(_)) {
                if roots.len() >= 16 {
                    return Err(tr("shell-up-to-16-workspace-folders").into());
                }
                roots.push(json!(path));
            }
        }
        Edit::ApprovalDefault(mode) => {
            config["enzui"]["approval_default"] = json!(mode.wire());
        }
        Edit::AddSkills(path) | Edit::RemoveSkills(path) => {
            let path = absolute_directory(path)?;
            for key in ["skill_directories", "writable_skill_directories"] {
                let list = strings(config, key)?;
                list.retain(|known| known.as_str() != Some(path.as_str()));
                if matches!(edit, Edit::AddSkills(_)) {
                    list.push(json!(path));
                }
            }
        }
        Edit::ServerEnabled(name, on) => {
            let server = config["mcp_servers"]
                .get_mut(name)
                .filter(|server| server.is_object())
                .ok_or_else(|| tr("shell-that-server-is-no-longer-configured"))?;
            server["enabled"] = json!(on);
        }
        Edit::RemoveServer(name) => {
            config["mcp_servers"]
                .as_object_mut()
                .and_then(|servers| servers.remove(name))
                .ok_or_else(|| tr("shell-that-server-is-no-longer-configured"))?;
        }
        Edit::AddServer(name, server) => {
            if !server_name(name) {
                return Err(tr(
                    "shell-names-are-1-to-24-lowercase-letters-digits-or-starting-with-a-letter",
                )
                .into());
            }
            let value = server_value(server)?;
            if !config["mcp_servers"].is_object() {
                config["mcp_servers"] = json!({});
            }
            let servers = config["mcp_servers"]
                .as_object_mut()
                .ok_or_else(|| tr("shell-invalid-configuration"))?;
            if servers.contains_key(name) {
                return Err(tr("shell-a-server-with-that-name-exists").into());
            }
            if servers.len() >= 8 {
                return Err(tr("shell-up-to-8-tool-servers").into());
            }
            servers.insert(name.clone(), value);
        }
    }
    Ok(())
}

/// What Settings shows of a configuration. Secrets are never read out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub providers: Vec<String>,
    pub provider: String,
    pub model: String,
    pub roots: Vec<String>,
    pub approval_default: ApprovalMode,
    pub skills: Vec<String>,
    /// Skill folders where Settings may create new skills.
    pub writable_skills: Vec<String>,
    /// Name, enabled, and what it runs or reaches (an executable or URL).
    pub servers: Vec<(String, bool, String)>,
    pub gateway: String,
}

#[must_use]
pub fn summary(config: &Value) -> Summary {
    let texts = |value: &Value| {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let profile = &config["profiles"]["default"];
    let servers = config["mcp_servers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, server)| {
            let target = match server["transport"].as_str() {
                Some("http") => server["url"]
                    .as_str()
                    .unwrap_or("")
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("")
                    .to_owned(),
                _ => server["argv"][0].as_str().unwrap_or("").to_owned(),
            };
            (
                name.clone(),
                server["enabled"].as_bool().unwrap_or(false),
                target,
            )
        })
        .collect();
    Summary {
        providers: texts(&config["providers"]),
        provider: config["default_provider"]
            .as_str()
            .unwrap_or("local")
            .to_owned(),
        model: config["enzui"]["model"].as_str().unwrap_or("").to_owned(),
        roots: texts(&profile["workspace_roots"]),
        approval_default: ApprovalMode::read(&config["enzui"]["approval_default"]),
        skills: texts(&config["skill_directories"]),
        writable_skills: texts(&config["writable_skill_directories"]),
        servers,
        gateway: config["gateway_socket"].as_str().unwrap_or("").to_owned(),
    }
}

/// Whether the service is running now.
pub async fn running() -> bool {
    tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", UNIT])
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Starts or stops the service for this login only.
pub async fn set_running(on: bool) -> Result<(), String> {
    let status = tokio::process::Command::new("systemctl")
        .args(["--user", if on { "start" } else { "stop" }, UNIT])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| tr("shell-systemctl-is-unavailable"))?;
    if status.success() {
        Ok(())
    } else {
        Err(if on {
            tr("shell-the-assistant-service-did-not-start")
        } else {
            tr("shell-the-assistant-service-did-not-stop")
        }
        .into())
    }
}

pub(crate) fn private_socket(path: &Path) -> Result<UnixStream, String> {
    let uid = rustix::process::getuid().as_raw();
    let parent = std::fs::metadata(path.parent().ok_or_else(|| tr("shell-invalid-socket"))?)
        .map_err(|_| tr("shell-the-service-is-not-running"))?;
    let socket =
        std::fs::symlink_metadata(path).map_err(|_| tr("shell-the-service-is-not-running"))?;
    use std::os::unix::fs::FileTypeExt;
    if parent.uid() != uid
        || parent.mode() & 0o077 != 0
        || !socket.file_type().is_socket()
        || socket.uid() != uid
        || socket.mode() & 0o077 != 0
    {
        return Err(tr("shell-the-service-socket-is-not-private-to-you").into());
    }
    let stream = UnixStream::connect(path).map_err(|_| tr("shell-the-service-is-not-running"))?;
    let peer = rustix::net::sockopt::socket_peercred(&stream)
        .map_err(|_| tr("shell-the-service-could-not-be-identified"))?;
    if peer.uid.as_raw() != uid {
        return Err(tr("shell-the-service-belongs-to-another-user").into());
    }
    Ok(stream)
}

/// A checked connection to the service, for a turn's event stream.
pub fn connect() -> Result<UnixStream, String> {
    private_socket(&socket_path().ok_or_else(|| tr("shell-no-runtime-folder"))?)
}

/// Localized recovery guidance for actionable turn failures.
pub fn turn_error(code: &str) -> Option<&'static str> {
    let key = match code {
        "gateway-unavailable" => "shell-assistant-gateway-unavailable",
        "local-models-unavailable" => "shell-assistant-local-models-unavailable",
        "provider-timeout" => "shell-assistant-provider-timeout",
        "request-unavailable" => "shell-assistant-request-unavailable",
        "invalid-attachments" => "shell-assistant-invalid-attachments",
        "attachment-vision-required" => "shell-assistant-attachment-vision-required",
        "local-context-limit" => "shell-assistant-local-context-limit",
        "stale-turn-revision" => "shell-assistant-stale-revision",
        _ => return None,
    };
    Some(tr(key))
}

/// The `turn.start` request line, with the skills chosen for this turn as
/// `(name, sha256)`; the harness refuses one that changed since it was listed.
#[must_use]
pub fn turn_request(
    session_id: &str,
    prompt: &str,
    skills: &[(String, String)],
    mode: ApprovalMode,
    screenshots: Screenshots,
) -> Vec<u8> {
    let mut body = json!({"schema_version": 1, "method": "turn.start", "session_id": session_id, "prompt": prompt,
        "approval_mode": mode.wire(), "screenshots": screenshots.wire()});
    if !skills.is_empty() {
        body["skills"] = skills
            .iter()
            .take(MAX_TURN_SKILLS)
            .map(|(name, sha256)| json!({"name": name, "sha256": sha256}))
            .collect();
    }
    let mut raw = serde_json::to_vec(&body).unwrap_or_default();
    raw.push(b'\n');
    raw
}

pub const MAX_TURN_SKILLS: usize = 8;

/// A skill the harness found (`skills.list`): instructions it can add to a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub sha256: String,
}

/// The listed skills; rows of an unknown form are left out.
#[must_use]
pub fn skills(listed: &Value) -> Vec<Skill> {
    listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let name = row["name"].as_str().filter(|name| skill_name(name))?;
            let sha256 = row["sha256"]
                .as_str()
                .filter(|sha| sha.len() == 64 && sha.bytes().all(|c| c.is_ascii_hexdigit()))?;
            Some(Skill {
                name: name.to_owned(),
                description: row["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(1024)
                    .collect(),
                sha256: sha256.to_owned(),
            })
        })
        .take(128)
        .collect()
}

/// The harness's skill-name rule: lowercase words of `a-z0-9` joined by `-`, up to 64.
#[must_use]
pub fn skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// Checks a new skill as the harness will, before asking it to write one.
pub fn new_skill(name: &str, description: &str, instructions: &str) -> Result<(), String> {
    if !skill_name(name) {
        return Err(tr("shell-names-are-lowercase-words-of-letters-and-digits-joined-by").into());
    }
    if !(1..=1024).contains(&description.trim().chars().count()) {
        return Err(tr("shell-describe-the-skill-in-up-to-1024-characters").into());
    }
    if instructions.trim().is_empty() || instructions.len() > 60 * 1024 {
        return Err(tr("shell-write-the-instructions-up-to-60-kib").into());
    }
    Ok(())
}

/// One request and its reply on a fresh connection (not for `turn.start`).
pub fn request(method: &str, fields: Value) -> Result<Value, String> {
    let path = socket_path().ok_or_else(|| tr("shell-no-runtime-folder"))?;
    let mut stream = private_socket(&path)?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let mut body = json!({"schema_version": 1, "method": method});
    for (key, value) in fields.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    let mut raw = serde_json::to_vec(&body).map_err(|_| tr("shell-invalid-request"))?;
    raw.push(b'\n');
    if raw.len() > MAX_FRAME {
        return Err(tr("shell-the-request-is-too-large").into());
    }
    stream
        .write_all(&raw)
        .map_err(|_| tr("shell-the-service-did-not-accept-the-request"))?;
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_FRAME as u64 + 1))
        .read_until(b'\n', &mut line)
        .map_err(|_| tr("shell-the-service-did-not-answer"))?;
    if line.len() > MAX_FRAME || !line.ends_with(b"\n") {
        return Err(tr("shell-the-service-answered-in-an-unknown-form").into());
    }
    let reply: Value = serde_json::from_slice(&line)
        .map_err(|_| tr("shell-the-service-answered-in-an-unknown-form"))?;
    match reply["ok"].as_bool() {
        Some(true) => Ok(reply["result"].clone()),
        _ => Err(reply["error"].as_str().map_or_else(|| tr("shell-the-service-refused-the-request").to_owned(), |error| tr!("shell-the-service-refused", "arg0" => error.chars().take(160).collect::<String>()))),
    }
}

/// The models the configured gateway offers (`/v1/models`), for choosing one.
pub fn models(gateway: &str) -> Result<Vec<String>, String> {
    let path = Path::new(gateway);
    if !path.is_absolute() {
        return Err(tr("shell-no-gateway-is-configured").into());
    }
    let mut stream =
        private_socket(path).map_err(|_| tr("shell-the-ai-gateway-is-not-running").to_owned())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream
        .write_all(b"GET /v1/models HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|_| tr("shell-the-ai-gateway-did-not-answer"))?;
    let mut response = Vec::new();
    stream
        .take(MAX_FRAME as u64)
        .read_to_end(&mut response)
        .map_err(|_| tr("shell-the-ai-gateway-did-not-answer"))?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| tr("shell-the-ai-gateway-answered-in-an-unknown-form"))?;
    if head.split(' ').nth(1) != Some("200")
        || head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    {
        return Err(tr("shell-the-ai-gateway-did-not-list-its-models").into());
    }
    let value: Value = serde_json::from_str(body.trim())
        .map_err(|_| tr("shell-the-ai-gateway-answered-in-an-unknown-form"))?;
    Ok(value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| model["id"].as_str())
        .filter(|id| (1..=128).contains(&id.len()))
        .take(64)
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_harness_behaviour() {
        // Edits keep unknown keys and never expose secrets.
        {
            let mut config = starter();
            config["future"] = json!({"kept": true});
            config["mcp_servers"]["docs"] = json!({"enabled": true, "transport": "http", "url": "https://example.org/mcp?token=x", "authorization": "Bearer secret"});
            apply(&mut config, &Edit::AddRoot("/home/ann/Projects/".into())).unwrap();
            apply(
                &mut config,
                &Edit::ApprovalDefault(ApprovalMode::WorkspaceEdits),
            )
            .unwrap();
            apply(&mut config, &Edit::Model("qwen3".into())).unwrap();
            apply(&mut config, &Edit::ServerEnabled("docs".into(), false)).unwrap();
            assert!(
                apply(&mut config, &Edit::Provider("cloud".into())).is_err(),
                "only approved providers"
            );
            assert!(apply(&mut config, &Edit::AddRoot("relative".into())).is_err());
            let shown = summary(&config);
            assert_eq!(shown.roots, ["/home/ann/Projects"]);
            assert_eq!(shown.approval_default, ApprovalMode::WorkspaceEdits);
            assert_eq!(shown.model, "qwen3");
            assert_eq!(
                shown.servers,
                [(
                    "docs".to_owned(),
                    false,
                    "https://example.org/mcp".to_owned()
                )]
            );
            assert_eq!(config["future"]["kept"], json!(true));
            assert_eq!(
                config["mcp_servers"]["docs"]["authorization"],
                json!("Bearer secret"),
                "kept, not shown"
            );
            apply(&mut config, &Edit::RemoveServer("docs".into())).unwrap();
            assert!(summary(&config).servers.is_empty());
        }
        // Configuration preparation preserves the original on failure.
        {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("config/harness.json");
            let mut config = json!({"skill_directories":[], "preserved":true});
            save_at(&path, directory.path(), &config).unwrap();
            let original = std::fs::read(&path).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            let blocked = directory.path().join("blocked");
            std::fs::write(&blocked, b"not a directory").unwrap();
            config["skill_directories"] = json!([blocked.join("skills")]);
            assert!(save_at(&path, directory.path(), &config).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            let outside = tempfile::tempdir().unwrap();
            let link = directory.path().join("outside");
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            config["skill_directories"] = json!([link.join("new-skills")]);
            assert!(save_at(&path, directory.path(), &config).is_err());
            assert!(!outside.path().join("new-skills").exists());
            assert_eq!(std::fs::read(&path).unwrap(), original);
            config["skill_directories"] = json!([directory.path().join("skills/nested")]);
            save_at(&path, directory.path(), &config).unwrap();
            assert!(directory.path().join("skills/nested").is_dir());
        }
        // Added servers follow the harness rules and keep secrets out of debug.
        {
            let mut config = starter();
            let web = |url: &str| Server::Web {
                url: url.into(),
                authorization: Some("Bearer secret".into()),
            };
            apply(
                &mut config,
                &Edit::AddServer("docs".into(), web("https://example.org/mcp")),
            )
            .unwrap();
            assert_eq!(
                config["mcp_servers"]["docs"],
                json!({"enabled": true, "transport": "http", "url": "https://example.org/mcp", "authorization": "Bearer secret"})
            );
            apply(
                &mut config,
                &Edit::AddServer("local_1".into(), web("http://127.0.0.1:8080/mcp")),
            )
            .unwrap();
            apply(
                &mut config,
                &Edit::AddServer("v6".into(), web("http://[::1]:9/")),
            )
            .unwrap();
            for url in [
                "http://example.org/mcp",
                "http://localhost/mcp",
                "https://user:pw@example.org/",
                "https://example.org/#x",
                "ftp://example.org",
                "https://exa mple.org",
            ] {
                assert!(
                    apply(&mut config, &Edit::AddServer("other".into(), web(url))).is_err(),
                    "{url}"
                );
            }
            assert!(
                apply(
                    &mut config,
                    &Edit::AddServer("docs".into(), web("https://example.org/b"))
                )
                .is_err(),
                "names stay unique"
            );
            for name in ["", "Docs", "1docs", "docs-x", "abcdefghijklmnopqrstuvwxy"] {
                assert!(
                    apply(
                        &mut config,
                        &Edit::AddServer(name.into(), web("https://example.org/"))
                    )
                    .is_err(),
                    "{name}"
                );
            }
            let program = |argv: &[&str]| Server::Program {
                argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            };
            assert!(
                apply(
                    &mut config,
                    &Edit::AddServer("relative".into(), program(&["enzui-mcp"]))
                )
                .is_err()
            );
            assert!(apply(&mut config, &Edit::AddServer("empty".into(), program(&[]))).is_err());
            apply(
                &mut config,
                &Edit::AddServer(
                    "center".into(),
                    program(&[
                        "/usr/bin/enzui-mcp",
                        "--socket",
                        "/run/user/1000/enzui/center.sock",
                    ]),
                ),
            )
            .unwrap();
            assert_eq!(
                config["mcp_servers"]["center"]["argv"][2],
                json!("/run/user/1000/enzui/center.sock")
            );
            assert!(
                !format!(
                    "{:?}",
                    Edit::AddServer("docs".into(), web("https://example.org/?token=x"))
                )
                .contains("secret")
            );
            assert!(
                !format!(
                    "{:?}",
                    Edit::AddServer("docs".into(), web("https://example.org/?token=x"))
                )
                .contains("token=x")
            );
            for name in ["a", "b", "c", "d"] {
                apply(
                    &mut config,
                    &Edit::AddServer(name.into(), web("https://example.org/")),
                )
                .unwrap();
            }
            assert!(
                apply(
                    &mut config,
                    &Edit::AddServer("ninth".into(), web("https://example.org/"))
                )
                .is_err(),
                "harness limit of 8"
            );
        }
        // Turn skills are bounded and listed rows validated.
        {
            let sha = "a".repeat(64);
            let chosen: Vec<_> = (0..10)
                .map(|i| (format!("skill-{i}"), sha.clone()))
                .collect();
            let line = turn_request(
                "s1",
                "hello",
                &chosen,
                ApprovalMode::FullSend,
                Screenshots::Vision,
            );
            let body: Value = serde_json::from_slice(&line[..line.len() - 1]).unwrap();
            assert_eq!(body["approval_mode"], "full_send");
            assert_eq!(body["screenshots"], "vision");
            assert_eq!(body["skills"].as_array().unwrap().len(), MAX_TURN_SKILLS);
            assert_eq!(body["skills"][0], json!({"name": "skill-0", "sha256": sha}));
            let plain = turn_request("s1", "hello", &[], ApprovalMode::Ask, Screenshots::Off);
            let plain: Value = serde_json::from_slice(&plain).unwrap();
            assert!(plain.get("skills").is_none(), "no field without a choice");
            let listed = json!([
                {"name": "release-notes", "description": "Notes", "sha256": sha},
                {"name": "Bad", "description": "x", "sha256": sha},
                {"name": "short-sha", "description": "x", "sha256": "abc"},
            ]);
            assert_eq!(
                skills(&listed),
                [Skill {
                    name: "release-notes".into(),
                    description: "Notes".into(),
                    sha256: sha
                }]
            );
            assert!(new_skill("release-notes", "Summarize", "Use bullets").is_ok());
            assert!(new_skill("release_notes", "Summarize", "x").is_err());
            assert!(new_skill("notes", "", "x").is_err());
            assert!(new_skill("notes", "d", "  ").is_err());
        }
    }
}
