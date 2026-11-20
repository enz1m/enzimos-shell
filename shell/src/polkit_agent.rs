//! Session polkit agent relaying PAM prompts through polkit's helper.
//! The helper authenticates with polkitd; this client only relays questions and answers.

use enzui_shell::i18n::tr;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use zbus::zvariant::{OwnedValue, Value};

const AGENT_PATH: &str = "/dev/enzui/PolicyKit1/AuthenticationAgent";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPERS: [&str; 2] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
];

/// Someone who may authenticate the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub uid: u32,
    pub name: String,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthRequest {
    pub id: u64,
    pub action: String,
    pub message: String,
    pub identities: Vec<Identity>,
    /// The identity asked first: the session's user when allowed.
    pub preferred: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Begin(AuthRequest),
    /// PAM asks a question; `secret` hides the typing.
    Prompt {
        id: u64,
        text: String,
        secret: bool,
    },
    /// A message from PAM or from the agent (`error` for failures).
    Notice {
        id: u64,
        text: String,
        error: bool,
    },
    /// The request ended: authenticated, cancelled or given up.
    Done {
        id: u64,
    },
    /// The agent could not register; nothing will be asked.
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Answer {
        id: u64,
        text: String,
    },
    /// Authenticate as someone else from the request's identities.
    Identity {
        id: u64,
        uid: u32,
    },
    Cancel {
        id: u64,
    },
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
    Failed(String),
}

type Pending = Arc<Mutex<BTreeMap<u64, (String, mpsc::UnboundedSender<Command>)>>>;

struct Agent {
    events: mpsc::UnboundedSender<Event>,
    pending: Pending,
    next: Mutex<u64>,
}

fn passwd() -> Vec<(String, u32, u32, String)> {
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            let [name, _, uid, gid, gecos, ..] = f.as_slice() else {
                return None;
            };
            Some((
                (*name).to_owned(),
                uid.parse().ok()?,
                gid.parse().ok()?,
                gecos.split(',').next().unwrap_or("").trim().to_owned(),
            ))
        })
        .collect()
}

/// The users behind polkit's identities; groups (like `wheel`) expand to
/// their members, the way other agents present them.
pub fn identities(
    raw: &[(String, HashMap<String, OwnedValue>)],
    passwd: &[(String, u32, u32, String)],
    group: &str,
) -> Vec<Identity> {
    let person = |uid: u32| {
        passwd
            .iter()
            .find(|p| p.1 == uid)
            .map(|(name, uid, _, real)| Identity {
                uid: *uid,
                name: name.clone(),
                display: if real.is_empty() {
                    name.clone()
                } else {
                    real.clone()
                },
            })
    };
    let mut out: Vec<Identity> = Vec::new();
    for (kind, fields) in raw {
        let number = |key: &str| fields.get(key).and_then(|v| u32::try_from(v).ok());
        let found: Vec<Identity> = match kind.as_str() {
            "unix-user" => number("uid").and_then(person).into_iter().collect(),
            "unix-group" => {
                let Some(gid) = number("gid") else { continue };
                let members: Vec<&str> = group
                    .lines()
                    .find(|l| l.split(':').nth(2).and_then(|g| g.parse::<u32>().ok()) == Some(gid))
                    .and_then(|l| l.split(':').nth(3))
                    .map(|m| m.split(',').filter(|m| !m.is_empty()).collect())
                    .unwrap_or_default();
                passwd
                    .iter()
                    .filter(|p| p.2 == gid || members.contains(&p.0.as_str()))
                    .filter_map(|p| person(p.1))
                    .collect()
            }
            _ => Vec::new(),
        };
        for identity in found {
            if !out.iter().any(|o| o.uid == identity.uid) && out.len() < 16 {
                out.push(identity);
            }
        }
    }
    out
}

enum Outcome {
    Success,
    Failed,
    Cancelled,
    Switch(u32),
    Broken(String),
}

async fn helper(
    user: &str,
    cookie: &str,
) -> Result<
    (
        Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box<dyn AsyncWrite + Unpin + Send>,
        Option<tokio::process::Child>,
    ),
    String,
> {
    if let Ok(stream) = tokio::net::UnixStream::connect(HELPER_SOCKET).await {
        let (read, mut write) = stream.into_split();
        write
            .write_all(format!("{user}\n{cookie}\n").as_bytes())
            .await
            .map_err(|_| tr("shell-the-polkit-helper-stopped-answering"))?;
        return Ok((Box::new(read), Box::new(write), None));
    }
    let path = HELPERS
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .ok_or_else(|| tr("shell-polkit-s-authentication-helper-is-not-installed"))?;
    let mut child = tokio::process::Command::new(path)
        .arg(user)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| tr("shell-polkit-s-authentication-helper-could-not-start"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| tr("shell-polkit-s-authentication-helper-could-not-start"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| tr("shell-polkit-s-authentication-helper-could-not-start"))?;
    stdin
        .write_all(format!("{cookie}\n").as_bytes())
        .await
        .map_err(|_| tr("shell-the-polkit-helper-stopped-answering"))?;
    Ok((Box::new(stdout), Box::new(stdin), Some(child)))
}

// One PAM conversation through polkit's helper.
async fn converse(
    user: &str,
    cookie: &str,
    id: u64,
    events: &mpsc::UnboundedSender<Event>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Outcome {
    let (read, mut write, _child) = match helper(user, cookie).await {
        Ok(parts) => parts,
        Err(error) => return Outcome::Broken(error),
    };
    let mut lines = BufReader::new(read).lines();
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line,
            command = commands.recv() => match command {
                Some(Command::Cancel { .. }) | None => return Outcome::Cancelled,
                Some(Command::Identity { uid, .. }) => return Outcome::Switch(uid),
                Some(Command::Answer { .. }) => continue,
            },
        };
        let Ok(Some(line)) = line else {
            return Outcome::Failed;
        };
        let line = line.trim_end();
        let (secret, question) = if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_OFF ") {
            (true, Some(text))
        } else if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_ON ") {
            (false, Some(text))
        } else {
            (false, None)
        };
        if let Some(text) = question {
            let _ = events.send(Event::Prompt {
                id,
                text: text.trim().chars().take(200).collect(),
                secret,
            });
            let answer = match commands.recv().await {
                Some(Command::Answer { text, .. }) => text,
                Some(Command::Identity { uid, .. }) => return Outcome::Switch(uid),
                Some(Command::Cancel { .. }) | None => return Outcome::Cancelled,
            };
            // The helper reads one line per answer.
            let answer: String = answer
                .chars()
                .filter(|c| *c != '\n' && *c != '\r')
                .collect();
            if write
                .write_all(format!("{answer}\n").as_bytes())
                .await
                .is_err()
            {
                return Outcome::Failed;
            }
            drop(answer);
            continue;
        }
        if let Some(text) = line.strip_prefix("PAM_ERROR_MSG ") {
            let _ = events.send(Event::Notice {
                id,
                text: text.trim().chars().take(200).collect(),
                error: true,
            });
        } else if let Some(text) = line.strip_prefix("PAM_TEXT_INFO ") {
            let _ = events.send(Event::Notice {
                id,
                text: text.trim().chars().take(200).collect(),
                error: false,
            });
        } else if line == "SUCCESS" {
            return Outcome::Success;
        } else if line == "FAILURE" {
            return Outcome::Failed;
        }
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        _icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), AgentError> {
        let people = identities_now(&identities);
        if people.is_empty() {
            return Err(AgentError::Failed(
                tr("shell-no-one-can-authenticate-this-request").into(),
            ));
        }
        let id = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *next += 1;
            *next
        };
        let own = rustix::process::getuid().as_raw();
        let mut uid = people
            .iter()
            .find(|p| p.uid == own)
            .unwrap_or(&people[0])
            .uid;
        let (tx, mut commands) = mpsc::unbounded_channel();
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, (cookie.clone(), tx));
        let _ = self.events.send(Event::Begin(AuthRequest {
            id,
            action: action_id.chars().take(128).collect(),
            message: message.chars().take(400).collect(),
            identities: people.clone(),
            preferred: uid,
        }));
        let result = loop {
            let Some(user) = people.iter().find(|p| p.uid == uid).map(|p| p.name.clone()) else {
                break Err(AgentError::Failed(tr("shell-unknown-identity").into()));
            };
            match converse(&user, &cookie, id, &self.events, &mut commands).await {
                Outcome::Success => break Ok(()),
                Outcome::Failed => {
                    let _ = self.events.send(Event::Notice {
                        id,
                        text: tr("shell-that-password-is-not-correct-try-again").into(),
                        error: true,
                    });
                }
                Outcome::Switch(next) if people.iter().any(|p| p.uid == next) => uid = next,
                Outcome::Switch(_) => {}
                Outcome::Cancelled => {
                    break Err(AgentError::Cancelled(
                        tr("shell-authentication-was-cancelled").into(),
                    ));
                }
                Outcome::Broken(error) => break Err(AgentError::Failed(error)),
            }
        };
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        let _ = self.events.send(Event::Done { id });
        result
    }
    #[allow(
        clippy::unused_async,
        reason = "This method implements an asynchronous D-Bus interface."
    )]
    async fn cancel_authentication(&self, cookie: String) {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (id, (known, tx)) in pending.iter() {
            if *known == cookie {
                let _ = tx.send(Command::Cancel { id: *id });
            }
        }
    }
}

fn identities_now(raw: &[(String, HashMap<String, OwnedValue>)]) -> Vec<Identity> {
    identities(
        raw,
        &passwd(),
        &std::fs::read_to_string("/etc/group").unwrap_or_default(),
    )
}

// The logind session this process belongs to, which the agent serves.
async fn session_id(system: &zbus::Connection) -> Result<String, String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID")
        && !id.is_empty()
        && id.len() <= 64
    {
        return Ok(id);
    }
    let manager = zbus::Proxy::new(
        system,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    let path: zbus::zvariant::OwnedObjectPath = manager
        .call("GetSessionByPID", &(std::process::id()))
        .await
        .map_err(|_| tr("shell-this-process-is-not-in-a-login-session"))?;
    let session = zbus::Proxy::new(
        system,
        "org.freedesktop.login1",
        path,
        "org.freedesktop.login1.Session",
    )
    .await
    .map_err(|_| tr("shell-logind-is-not-available"))?;
    session
        .get_property::<String>("Id")
        .await
        .map_err(|_| tr("shell-logind-is-not-available").into())
}

/// Registers the agent for this login session and serves it until the
/// command channel closes.
pub async fn run(
    events: mpsc::UnboundedSender<Event>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<(), String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus"))?;
    let pending: Pending = Arc::default();
    let agent = Agent {
        events: events.clone(),
        pending: std::sync::Arc::clone(&pending),
        next: Mutex::new(0),
    };
    system
        .object_server()
        .at(AGENT_PATH, agent)
        .await
        .map_err(|_| tr("shell-could-not-publish-the-agent"))?;
    let session = session_id(&system).await?;
    let authority = zbus::Proxy::new(
        &system,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await
    .map_err(|_| tr("shell-polkit-is-not-available"))?;
    let subject: (&str, HashMap<&str, Value>) = (
        "unix-session",
        HashMap::from([("session-id", Value::from(session.as_str()))]),
    );
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into());
    authority
        .call::<_, _, ()>(
            "RegisterAuthenticationAgent",
            &(subject, locale.as_str(), AGENT_PATH),
        )
        .await
        .map_err(|error| match error {
            zbus::Error::MethodError(_, Some(message), _) => message,
            _ => tr("shell-polkit-refused-the-agent").into(),
        })?;
    // Commands from the dialog go to the conversation they belong to.
    while let Some(command) = commands.recv().await {
        let id = match &command {
            Command::Answer { id, .. } | Command::Identity { id, .. } | Command::Cancel { id } => {
                *id
            }
        };
        if let Some((_, tx)) = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
        {
            let _ = tx.send(command);
        }
    }
    Ok(())
}

/// Exits with the parent shell to prevent duplicate session agents.
pub fn exit_with_parent() {
    let parent = std::os::unix::process::parent_id();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if std::os::unix::process::parent_id() != parent {
                std::process::exit(0);
            }
        }
    });
}

pub fn supervise(binary: std::path::PathBuf) {
    std::thread::spawn(move || {
        let mut failures = 0u32;
        loop {
            let started = std::time::Instant::now();
            let status = std::process::Command::new(&binary)
                .stdin(std::process::Stdio::null())
                .status();
            if status.is_err() {
                return;
            }
            failures = if started.elapsed() > std::time::Duration::from_secs(60) {
                0
            } else {
                failures + 1
            };
            if failures >= 5 {
                eprintln!("enzui-shell: {} keeps exiting; giving up", binary.display());
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(2 * u64::from(failures)));
        }
    });
}
