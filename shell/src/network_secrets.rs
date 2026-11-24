//! NetworkManager secret-agent requests bound to its verified root-owned bus identity.
//!
//! Answers return over D-Bus without persistence or logging. Cancellation, expiry or
//! service-owner replacement withdraws pending questions.

use enzui_security::secret::SecretText;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const IDENTIFIER: &str = "dev.enzui.network-secrets";
// NetworkManager gives up on an agent after two minutes.
const DEADLINE: Duration = Duration::from_secs(110);

const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;
// Bounds on what one request may carry, checked before anything is queued.
const MAX_PENDING: usize = 4;
const MAX_HINTS: usize = 32;
const MAX_HINT_BYTES: usize = 1024;
const MAX_FIELDS: usize = 8;

/// One thing to type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub id: u64,
    /// The connection's name, as the person saved it.
    pub connection: String,
    pub kind: Kind,
    /// Extra words the VPN service sent with its question, if any.
    pub message: Option<String>,
    /// The previous answer was refused.
    pub retry: bool,
    pub fields: Vec<Field>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wifi,
    Enterprise,
    Vpn,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Ask(Ask),
    Done { id: u64 },
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Answer {
        id: u64,
        values: Vec<(String, SecretText)>,
    },
    Cancel {
        id: u64,
    },
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    PermissionDenied(String),
    UserCanceled(String),
    NoSecrets(String),
    AgentCanceled(String),
}

type Settings = HashMap<String, HashMap<String, OwnedValue>>;
type Pending = Arc<Mutex<BTreeMap<u64, (String, String, mpsc::UnboundedSender<Command>)>>>;
// Verified root-owned NetworkManager bus identity.
type Owner = Arc<Mutex<Option<String>>>;

struct Agent {
    events: mpsc::UnboundedSender<Event>,
    pending: Pending,
    next: Mutex<u64>,
    owner: Owner,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Agent {
    // Only the verified NetworkManager may ask, cancel, save or delete.
    fn check(&self, header: &zbus::message::Header<'_>) -> Result<(), AgentError> {
        let sender = header.sender().map(zbus::names::UniqueName::as_str);
        match lock(&self.owner).as_deref() {
            Some(owner) if sender == Some(owner) => Ok(()),
            _ => Err(AgentError::PermissionDenied(
                tr("shell-only-networkmanager-may-ask-this-agent").into(),
            )),
        }
    }
}

fn withdraw(owner: &Owner, pending: &Pending) {
    *lock(owner) = None;
    for (id, (_, _, tx)) in lock(pending).iter() {
        let _ = tx.send(Command::Cancel { id: *id });
    }
}

fn bounded(hints: &[String]) -> bool {
    hints.len() <= MAX_HINTS && hints.iter().all(|hint| hint.len() <= MAX_HINT_BYTES)
}

fn text(settings: &Settings, setting: &str, key: &str) -> Option<String> {
    let value = settings.get(setting)?.get(key)?;
    String::try_from(value.try_clone().ok()?).ok()
}

// A VPN's `data` (its plugin's settings, `a{ss}`).
fn vpn_data(settings: &Settings) -> HashMap<String, String> {
    settings
        .get("vpn")
        .and_then(|vpn| vpn.get("data"))
        .and_then(|value| HashMap::<String, String>::try_from(value.try_clone().ok()?).ok())
        .unwrap_or_default()
}

fn field(key: &str, label: &str, secret: bool) -> Field {
    Field {
        key: key.into(),
        label: label.into(),
        secret,
    }
}

/// Builds questions from NetworkManager hints or connection settings; unsupported sign-in
/// returns an error.
pub fn fields(
    settings: &Settings,
    setting: &str,
    hints: &[String],
) -> Result<(Kind, Option<String>, Vec<Field>), String> {
    let mut message = None;
    let mut named = Vec::new();
    for hint in hints {
        if let Some(text) = hint.strip_prefix("x-vpn-message:") {
            message = Some(text.chars().take(400).collect());
        } else if !hint.starts_with("x-") && !hint.is_empty() && hint.len() <= 64 {
            named.push(hint.clone());
        }
    }
    let label = |key: &str| match key {
        "password" => tr("shell-password"),
        "psk" => tr("shell-wi-fi-password"),
        "cert-pass" => tr("shell-certificate-password"),
        "challenge-response" => tr("shell-one-time-code"),
        "pin" => "PIN",
        "private-key" => tr("shell-private-key"),
        "Xauth password" => tr("shell-password"),
        "IPSec secret" => tr("shell-shared-secret"),
        _ => tr("shell-secret"),
    };
    match setting {
        "vpn" => {
            let service = text(settings, "vpn", "service-type").unwrap_or_default();
            if service.ends_with(".openconnect") {
                return Err(tr(
                    "shell-this-vpn-signs-in-through-a-web-page-which-is-not-supported-yet",
                )
                .into());
            }
            let data = vpn_data(settings);
            let mut wanted: Vec<String> = named.clone();
            if wanted.is_empty() && service.ends_with(".openvpn") {
                if let Some("password" | "password-tls") =
                    data.get("connection-type").map(String::as_str)
                {
                    wanted.push("password".into());
                }
                if data
                    .get("cert-pass-flags")
                    .is_some_and(|flags| flags != "4")
                    && data.contains_key("key")
                {
                    wanted.push("cert-pass".into());
                }
                if data.contains_key("static-challenge") {
                    wanted.push("challenge-response".into());
                }
            }
            if wanted.is_empty() {
                wanted.push("password".into());
            }
            let mut fields: Vec<Field> = wanted
                .iter()
                .map(|key| field(key, label(key), true))
                .collect();
            if let Some(user) = data.get("username").filter(|u| !u.is_empty()) {
                // Shown, so the person knows which account the password is for.
                message.get_or_insert_with(|| tr!("shell-signing-in-as", "arg0" => user.chars().take(64).collect::<String>()));
            } else if service.ends_with(".openvpn")
                && matches!(
                    data.get("connection-type").map(String::as_str),
                    Some("password" | "password-tls")
                )
            {
                // NetworkManager takes the name from the profile only, so no
                // password can succeed until it is set.
                message = Some(tr("shell-this-vpn-has-no-sign-in-name-yet-add-it-in-settings-network-vpn-then-connect").into());
            }
            fields.dedup_by(|a, b| a.key == b.key);
            Ok((Kind::Vpn, message, fields))
        }
        "802-11-wireless-security" => {
            let management = text(settings, setting, "key-mgmt").unwrap_or_default();
            let key = if management == "none" {
                "wep-key0"
            } else {
                "psk"
            };
            Ok((
                Kind::Wifi,
                message,
                vec![field(key, tr("shell-wi-fi-password"), true)],
            ))
        }
        "802-1x" => Ok((
            Kind::Enterprise,
            message,
            vec![field("password", "Password", true)],
        )),
        "wireguard" => Ok((
            Kind::Vpn,
            message,
            vec![field("private-key", tr("shell-private-key"), true)],
        )),
        _ if !named.is_empty() => Ok((
            Kind::Other,
            message,
            named
                .iter()
                .map(|key| field(key, label(key), true))
                .collect(),
        )),
        _ => Err(
            tr("shell-this-connection-asks-for-a-kind-of-secret-this-desktop-cannot-ask-for")
                .into(),
        ),
    }
}

/// The sign-in program of VPN plugins that need their own (OpenConnect's
/// servers sign in through web forms or SAML pages), from the plugin's
/// registration file.
fn auth_dialog(service: &str) -> Option<std::path::PathBuf> {
    if !service.ends_with(".openconnect") {
        return None;
    }
    let dir = std::fs::read_dir("/usr/lib/NetworkManager/VPN").ok()?;
    for entry in dir.flatten().take(64) {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if !text
            .lines()
            .any(|l| l.trim() == format!("service={service}"))
        {
            continue;
        }
        let path = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("auth-dialog="))
            .map(|p| std::path::PathBuf::from(p.trim()))
            .filter(|p| p.is_absolute() && p.is_file())?;
        return Some(path);
    }
    None
}

/// The stdin of NetworkManager's VPN sign-in program protocol: the
/// connection's data and existing secrets, then `DONE`.
fn auth_dialog_input(data: &HashMap<String, String>) -> String {
    use std::fmt::Write as _;
    let mut input = String::new();
    let mut pairs: Vec<(&String, &String)> = data.iter().collect();
    pairs.sort();
    for (key, value) in pairs {
        if key.contains('\n') || value.contains('\n') {
            continue;
        }
        let _ = write!(input, "DATA_KEY={key}\nDATA_VAL={value}\n\n");
    }
    input.push_str("DONE\n\n");
    input
}

/// The program's answer: `key\nvalue\n` pairs ending with an empty line.
fn auth_dialog_output(output: &str) -> Vec<(String, SecretText)> {
    let mut values = Vec::new();
    let mut lines = output.lines();
    while let Some(key) = lines.next() {
        if key.is_empty() {
            break;
        }
        let Some(value) = lines.next() else { break };
        if key.len() <= 128 && value.len() <= 16 * 1024 && values.len() < 32 {
            values.push((key.to_owned(), SecretText::new(value.to_owned())));
        }
    }
    values
}

/// Run the plugin's sign-in program and return the secrets it produced.
async fn run_auth_dialog(
    dialog: &std::path::Path,
    connection: &Settings,
    retry: bool,
) -> Result<Settings, AgentError> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let uuid = text(connection, "connection", "uuid").unwrap_or_default();
    let name = text(connection, "connection", "id").unwrap_or_default();
    let service = text(connection, "vpn", "service-type").unwrap_or_default();
    let mut command = tokio::process::Command::new(dialog);
    command.args(["-u", &uuid, "-n", &name, "-s", &service, "-i"]);
    if retry {
        command.arg("-r");
    }
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| {
            AgentError::NoSecrets(tr("shell-the-vpn-sign-in-program-is-not-installed").into())
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AgentError::NoSecrets(tr("shell-the-vpn-sign-in-did-not-finish").into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AgentError::NoSecrets(tr("shell-the-vpn-sign-in-did-not-finish").into()))?;
    let _ = stdin
        .write_all(auth_dialog_input(&vpn_data(connection)).as_bytes())
        .await;
    let read = async {
        let mut reader = tokio::io::BufReader::new(stdout).lines();
        let mut text = String::new();
        let mut previous_empty = false;
        while let Ok(Some(line)) = reader.next_line().await {
            if line.is_empty() && (previous_empty || text.is_empty()) {
                break;
            }
            previous_empty = line.is_empty();
            text.push_str(&line);
            text.push('\n');
            if text.len() > 64 * 1024 {
                break;
            }
        }
        SecretText::new(text)
    };
    let output = tokio::time::timeout(DEADLINE, read).await;
    // The program waits for QUIT before it exits.
    let _ = stdin.write_all(b"QUIT\n\n").await;
    drop(stdin);
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let values = match output {
        Ok(output) => auth_dialog_output(output.expose_secret()),
        Err(_) => {
            return Err(AgentError::AgentCanceled(
                tr("shell-nobody-answered-in-time").into(),
            ));
        }
    };
    if values.is_empty() {
        return Err(AgentError::UserCanceled(
            tr("shell-the-sign-in-was-cancelled").into(),
        ));
    }
    Ok(reply("vpn", &values))
}

// The reply NetworkManager expects: VPN secrets inside `vpn.secrets`, the
// rest as their setting's own keys.
fn reply(setting: &str, values: &[(String, SecretText)]) -> Settings {
    let mut inner: HashMap<String, OwnedValue> = HashMap::new();
    if setting == "vpn" {
        let secrets: HashMap<&str, &str> = values
            .iter()
            .map(|(key, value)| (key.as_str(), value.expose_secret()))
            .collect();
        if let Ok(value) = OwnedValue::try_from(Value::from(secrets)) {
            inner.insert("secrets".into(), value);
        }
    } else {
        for (key, value) in values {
            if let Ok(value) = OwnedValue::try_from(Value::from(value.expose_secret())) {
                inner.insert(key.clone(), value);
            }
        }
    }
    HashMap::from([(setting.to_owned(), inner)])
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        hints: Vec<String>,
        flags: u32,
    ) -> Result<Settings, AgentError> {
        self.check(&header)?;
        if !bounded(&hints) || setting_name.len() > 64 {
            return Err(AgentError::NoSecrets(
                tr("shell-the-request-is-larger-than-this-agent-accepts").into(),
            ));
        }
        if flags & ALLOW_INTERACTION == 0 {
            // Nothing is stored here, so without asking there is nothing to give.
            return Err(AgentError::NoSecrets(
                tr("shell-the-enzui-agent-keeps-no-secrets").into(),
            ));
        }
        if setting_name == "vpn"
            && let Some(dialog) =
                auth_dialog(&text(&connection, "vpn", "service-type").unwrap_or_default())
        {
            return run_auth_dialog(&dialog, &connection, flags & REQUEST_NEW != 0).await;
        }
        let (kind, message, fields) =
            fields(&connection, &setting_name, &hints).map_err(AgentError::NoSecrets)?;
        if fields.len() > MAX_FIELDS {
            return Err(AgentError::NoSecrets(
                tr("shell-the-request-asks-for-more-than-this-agent-shows").into(),
            ));
        }
        let name = text(&connection, "connection", "id")
            .unwrap_or_else(|| "network".into())
            .chars()
            .take(80)
            .collect();
        let id = {
            let mut next = lock(&self.next);
            *next += 1;
            *next
        };
        let (tx, mut commands) = mpsc::unbounded_channel();
        {
            let mut pending = lock(&self.pending);
            if pending.len() >= MAX_PENDING {
                return Err(AgentError::NoSecrets(
                    tr("shell-too-many-sign-ins-are-waiting").into(),
                ));
            }
            pending.insert(id, (connection_path.to_string(), setting_name.clone(), tx));
        }
        let _ = self.events.send(Event::Ask(Ask {
            id,
            connection: name,
            kind,
            message,
            retry: flags & REQUEST_NEW != 0,
            fields: fields.clone(),
        }));
        let answer = tokio::time::timeout(DEADLINE, commands.recv()).await;
        lock(&self.pending).remove(&id);
        let _ = self.events.send(Event::Done { id });
        match answer {
            Ok(Some(Command::Answer { values, .. })) => {
                let values: Vec<(String, SecretText)> = values
                    .into_iter()
                    .filter(|(key, value)| {
                        fields.iter().any(|f| &f.key == key)
                            && !value.is_empty()
                            && value.expose_secret().len() <= 4096
                    })
                    .collect();
                if values.is_empty() {
                    return Err(AgentError::UserCanceled(
                        tr("shell-nothing-was-entered").into(),
                    ));
                }
                Ok(reply(&setting_name, &values))
            }
            Ok(Some(Command::Cancel { .. }) | None) => Err(AgentError::UserCanceled(
                tr("shell-the-sign-in-was-cancelled").into(),
            )),
            Err(_) => Err(AgentError::AgentCanceled(
                tr("shell-nobody-answered-in-time").into(),
            )),
        }
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn cancel_get_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        connection_path: OwnedObjectPath,
        setting_name: String,
    ) -> Result<(), AgentError> {
        self.check(&header)?;
        let pending = lock(&self.pending);
        for (id, (path, setting, tx)) in pending.iter() {
            if *path == connection_path.as_str() && *setting == setting_name {
                let _ = tx.send(Command::Cancel { id: *id });
            }
        }
        Ok(())
    }
    // Secrets are never kept by this agent.
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn save_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        _connection: Settings,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), AgentError> {
        self.check(&header)
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn delete_secrets(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        _connection: Settings,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), AgentError> {
        self.check(&header)
    }
}

// Accept NetworkManager's unique name only when root owns it.
async fn network_manager(bus: &zbus::fdo::DBusProxy<'_>) -> Option<String> {
    let name = zbus::names::BusName::try_from("org.freedesktop.NetworkManager").ok()?;
    let owner = bus.get_name_owner(name).await.ok()?;
    let uid = bus
        .get_connection_unix_user(owner.clone().into())
        .await
        .ok()?;
    (uid == 0).then(|| owner.to_string())
}

async fn register(system: &zbus::Connection) -> Result<(), String> {
    let manager = zbus::Proxy::new(
        system,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/AgentManager",
        "org.freedesktop.NetworkManager.AgentManager",
    )
    .await
    .map_err(|_| tr("shell-networkmanager-is-not-available"))?;
    // Capability 1: VPN hints, so a VPN's own questions reach the dialog.
    manager
        .call::<_, _, ()>("RegisterWithCapabilities", &(IDENTIFIER, 1u32))
        .await
        .map_err(|error| match error {
            zbus::Error::MethodError(_, Some(message), _) => message,
            _ => tr("shell-networkmanager-refused-the-agent").into(),
        })
}

/// Registers with NetworkManager and serves until the command channel closes.
pub async fn run(
    events: mpsc::UnboundedSender<Event>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<(), String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let pending: Pending = Arc::default();
    let owner: Owner = Arc::default();
    let agent = Agent {
        events: events.clone(),
        pending: std::sync::Arc::clone(&pending),
        next: Mutex::new(0),
        owner: std::sync::Arc::clone(&owner),
    };
    system
        .object_server()
        .at(AGENT_PATH, agent)
        .await
        .map_err(|_| tr("shell-could-not-publish-the-agent"))?;
    let bus = zbus::fdo::DBusProxy::new(&system)
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    // Watch first, so an owner change during registration is not missed.
    let mut owners = bus
        .receive_name_owner_changed_with_args(&[(0, "org.freedesktop.NetworkManager")])
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    *lock(&owner) = Some(
        network_manager(&bus)
            .await
            .ok_or_else(|| tr("shell-networkmanager-is-not-running-as-root"))?,
    );
    register(&system).await?;
    use iced::futures::StreamExt;
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()) };
                let id = match &command {
                    Command::Answer { id, .. } | Command::Cancel { id } => *id,
                };
                if let Some((_, _, tx)) = lock(&pending).get(&id) {
                    let _ = tx.send(command);
                }
            }
            change = owners.next() => {
                let Some(change) = change else { return Err(tr("shell-the-system-bus-closed").into()) };
                // Whatever the change, the NetworkManager that asked the open
                // questions no longer owns the name.
                withdraw(&owner, &pending);
                if change.args().is_ok_and(|args| args.new_owner().is_some()) {
                    // NetworkManager needs a moment to publish its agent manager.
                    for _ in 0..20 {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        let Some(name) = network_manager(&bus).await else { continue };
                        *lock(&owner) = Some(name);
                        if register(&system).await.is_ok() {
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(entries: &[(&str, &str, Value<'_>)]) -> Settings {
        let mut out: Settings = HashMap::new();
        for (setting, key, value) in entries {
            out.entry((*setting).into())
                .or_default()
                .insert((*key).into(), OwnedValue::try_from(value.clone()).unwrap());
        }
        out
    }

    use crate::test_bus::Bus;

    const AGENT_NAME: &str = "dev.enzui.TestSecretAgent";
    type Answer = (Settings,);

    fn wifi_request() -> (Settings, OwnedObjectPath, String, Vec<String>, u32) {
        let connection = settings(&[
            ("connection", "id", Value::from("Home")),
            (
                "802-11-wireless-security",
                "key-mgmt",
                Value::from("wpa-psk"),
            ),
        ]);
        (
            connection,
            OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/1").unwrap(),
            "802-11-wireless-security".into(),
            Vec::new(),
            ALLOW_INTERACTION,
        )
    }

    async fn ask(
        caller: &zbus::Connection,
        request: (Settings, OwnedObjectPath, String, Vec<String>, u32),
    ) -> Result<Answer, String> {
        caller
            .call_method(
                Some(AGENT_NAME),
                AGENT_PATH,
                Some("org.freedesktop.NetworkManager.SecretAgent"),
                "GetSecrets",
                &request,
            )
            .await
            .map_err(|error| match error {
                zbus::Error::MethodError(name, _, _) => name.to_string(),
                other => other.to_string(),
            })
            .and_then(|reply| {
                reply
                    .body()
                    .deserialize::<Settings>()
                    .map(|s| (s,))
                    .map_err(|e| e.to_string())
            })
    }

    async fn call(
        caller: &zbus::Connection,
        member: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> Result<(), String> {
        caller
            .call_method(
                Some(AGENT_NAME),
                AGENT_PATH,
                Some("org.freedesktop.NetworkManager.SecretAgent"),
                member,
                body,
            )
            .await
            .map(drop)
            .map_err(|error| match error {
                zbus::Error::MethodError(name, _, _) => name.to_string(),
                other => other.to_string(),
            })
    }

    async fn agent(
        bus: &Bus,
        owner: Option<String>,
    ) -> (
        zbus::Connection,
        mpsc::UnboundedReceiver<Event>,
        Pending,
        Owner,
    ) {
        let (events, received) = mpsc::unbounded_channel();
        let pending: Pending = Arc::default();
        let trusted: Owner = Arc::new(Mutex::new(owner));
        let served = bus
            .serve(
                AGENT_NAME,
                AGENT_PATH,
                Agent {
                    events,
                    pending: std::sync::Arc::clone(&pending),
                    next: Mutex::new(0),
                    owner: std::sync::Arc::clone(&trusted),
                },
            )
            .await;
        (served, received, pending, trusted)
    }

    #[tokio::test]
    async fn network_secrets_behaviour() {
        // Network secrets behaviour.
        {
            // Vpn sign in programs get the data and answer in pairs.
            {
                let data = HashMap::from([("gateway".to_owned(), "vpn.example.com".to_owned())]);
                assert_eq!(
                    auth_dialog_input(&data),
                    "DATA_KEY=gateway\nDATA_VAL=vpn.example.com\n\nDONE\n\n"
                );
                let answer = "cookie\nabc=1\ngwcert\nsha256:ff\n\n";
                assert_eq!(
                    auth_dialog_output(answer),
                    vec![
                        ("cookie".to_owned(), SecretText::new("abc=1".to_owned())),
                        ("gwcert".to_owned(), SecretText::new("sha256:ff".to_owned()))
                    ]
                );
                assert!(
                    auth_dialog_output("").is_empty(),
                    "a cancelled sign-in answers nothing"
                );
            }
            // Questions follow hints then the connection and web sign in is refused.
            {
                let data = HashMap::from([
                    ("connection-type".to_owned(), "password-tls".to_owned()),
                    ("username".to_owned(), "ann".to_owned()),
                    ("static-challenge".to_owned(), "Code".to_owned()),
                ]);
                let openvpn = settings(&[
                    ("connection", "id", Value::from("Office")),
                    (
                        "vpn",
                        "service-type",
                        Value::from("org.freedesktop.NetworkManager.openvpn"),
                    ),
                    ("vpn", "data", Value::from(data)),
                ]);
                let (kind, message, asked) = fields(&openvpn, "vpn", &[]).unwrap();
                assert_eq!(kind, Kind::Vpn);
                assert_eq!(
                    message.unwrap().replace(['\u{2068}', '\u{2069}'], ""),
                    "Signing in as ann"
                );
                assert_eq!(
                    asked.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
                    ["password", "challenge-response"]
                );
                let nameless = settings(&[
                    ("connection", "id", Value::from("Hotel")),
                    (
                        "vpn",
                        "service-type",
                        Value::from("org.freedesktop.NetworkManager.openvpn"),
                    ),
                    (
                        "vpn",
                        "data",
                        Value::from(HashMap::from([(
                            "connection-type".to_owned(),
                            "password".to_owned(),
                        )])),
                    ),
                ]);
                let (_, message, _) = fields(
                    &nameless,
                    "vpn",
                    &[
                        "x-vpn-message:A password is required.".into(),
                        "password".into(),
                    ],
                )
                .unwrap();
                assert!(
                    message.unwrap().contains("no sign-in name"),
                    "the missing name is the reason, not the password"
                );
                let (_, message, asked) = fields(
                    &openvpn,
                    "vpn",
                    &[
                        "x-vpn-message:Enter your token".into(),
                        "challenge-response".into(),
                    ],
                )
                .unwrap();
                assert_eq!(message.as_deref(), Some("Enter your token"));
                assert_eq!(asked.len(), 1);
                let openconnect = settings(&[(
                    "vpn",
                    "service-type",
                    Value::from("org.freedesktop.NetworkManager.openconnect"),
                )]);
                assert!(fields(&openconnect, "vpn", &[]).is_err());
                let wifi = settings(&[(
                    "802-11-wireless-security",
                    "key-mgmt",
                    Value::from("wpa-psk"),
                )]);
                assert_eq!(
                    fields(&wifi, "802-11-wireless-security", &[]).unwrap().2[0].key,
                    "psk"
                );
                let answer = reply(
                    "vpn",
                    &[("password".into(), SecretText::new("s3cret".into()))],
                );
                let secrets = HashMap::<String, String>::try_from(
                    answer["vpn"]["secrets"].try_clone().unwrap(),
                )
                .unwrap();
                assert_eq!(secrets["password"], "s3cret");
            }
        }

        // Only the verified network manager may ask cancel save or delete.
        {
            let bus = Bus::start();
            let network_manager = bus.client().await;
            let intruder = bus.client().await;
            let (_agent, mut events, pending, _owner) = agent(
                &bus,
                network_manager
                    .unique_name()
                    .map(std::string::ToString::to_string),
            )
            .await;
            let denied = "org.freedesktop.NetworkManager.SecretAgent.PermissionDenied";
            assert_eq!(ask(&intruder, wifi_request()).await.unwrap_err(), denied);
            assert!(
                events.try_recv().is_err(),
                "a foreign caller opens no dialog"
            );
            let (connection, path, ..) = wifi_request();
            assert_eq!(
                call(
                    &intruder,
                    "SaveSecrets",
                    &(connection.clone(), path.clone())
                )
                .await
                .unwrap_err(),
                denied
            );
            assert_eq!(
                call(
                    &intruder,
                    "DeleteSecrets",
                    &(connection.clone(), path.clone())
                )
                .await
                .unwrap_err(),
                denied
            );
            assert!(
                call(&network_manager, "SaveSecrets", &(connection, path.clone()))
                    .await
                    .is_ok()
            );

            let asking = tokio::spawn({
                let network_manager = network_manager.clone();
                async move { ask(&network_manager, wifi_request()).await }
            });
            let Some(Event::Ask(question)) = events.recv().await else {
                panic!("no dialog")
            };
            assert_eq!(
                (
                    question.connection.as_str(),
                    question.fields[0].key.as_str()
                ),
                ("Home", "psk")
            );
            assert_eq!(
                call(
                    &intruder,
                    "CancelGetSecrets",
                    &(path.clone(), "802-11-wireless-security")
                )
                .await
                .unwrap_err(),
                denied
            );
            assert_eq!(lock(&pending).len(), 1, "still waiting");
            let tx = lock(&pending).get(&question.id).unwrap().2.clone();
            tx.send(Command::Answer {
                id: question.id,
                values: vec![("psk".into(), SecretText::new("hunter22".into()))],
            })
            .unwrap();
            let (answer,) = asking.await.unwrap().unwrap();
            assert_eq!(
                String::try_from(
                    answer["802-11-wireless-security"]["psk"]
                        .try_clone()
                        .unwrap()
                )
                .unwrap(),
                "hunter22"
            );
        }
        // Losing network manager withdraws every open question.
        {
            let bus = Bus::start();
            let network_manager = bus.client().await;
            let (_agent, mut events, pending, owner) = agent(
                &bus,
                network_manager
                    .unique_name()
                    .map(std::string::ToString::to_string),
            )
            .await;
            let asking = tokio::spawn({
                let network_manager = network_manager.clone();
                async move { ask(&network_manager, wifi_request()).await }
            });
            assert!(matches!(events.recv().await, Some(Event::Ask(_))));
            withdraw(&owner, &pending);
            assert_eq!(
                asking.await.unwrap().unwrap_err(),
                "org.freedesktop.NetworkManager.SecretAgent.UserCanceled"
            );
            assert!(matches!(events.recv().await, Some(Event::Done { .. })));
            assert_eq!(
                ask(&network_manager, wifi_request()).await.unwrap_err(),
                "org.freedesktop.NetworkManager.SecretAgent.PermissionDenied"
            );
        }
        // Requests are bounded before they queue.
        {
            let bus = Bus::start();
            let network_manager = bus.client().await;
            let (_agent, mut events, _pending, _owner) = agent(
                &bus,
                network_manager
                    .unique_name()
                    .map(std::string::ToString::to_string),
            )
            .await;
            let no_secrets = "org.freedesktop.NetworkManager.SecretAgent.NoSecrets";
            let mut request = wifi_request();
            request.3 = vec!["x".repeat(MAX_HINT_BYTES + 1)];
            assert_eq!(
                ask(&network_manager, request).await.unwrap_err(),
                no_secrets
            );
            let mut request = wifi_request();
            request.3 = (0..=MAX_HINTS).map(|i| format!("x-{i}")).collect();
            assert_eq!(
                ask(&network_manager, request).await.unwrap_err(),
                no_secrets
            );
            let mut request = wifi_request();
            request.2 = "vpn".into();
            request.3 = (0..=MAX_FIELDS).map(|i| format!("secret{i}")).collect();
            assert_eq!(
                ask(&network_manager, request).await.unwrap_err(),
                no_secrets
            );
            assert!(
                events.try_recv().is_err(),
                "nothing oversized reached the dialog"
            );
            let mut waiting = Vec::new();
            for _ in 0..MAX_PENDING {
                let network_manager = network_manager.clone();
                waiting.push(tokio::spawn(async move {
                    ask(&network_manager, wifi_request()).await
                }));
                assert!(matches!(events.recv().await, Some(Event::Ask(_))));
            }
            assert_eq!(
                ask(&network_manager, wifi_request()).await.unwrap_err(),
                no_secrets
            );
            for task in waiting {
                task.abort();
            }
        }
    }
}
