//! Length-prefixed greetd JSON IPC on a worker thread, keeping PAM off the UI thread.

use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Success,
    Prompt { text: String, secret: bool },
    Info(String),
    AuthFailed(String),
    Error(String),
}

fn parse(reply: &Value) -> Reply {
    let text = |key: &str| {
        reply
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(300)
            .collect::<String>()
    };
    match reply.get("type").and_then(Value::as_str) {
        Some("success") => Reply::Success,
        Some("auth_message") => match reply.get("auth_message_type").and_then(Value::as_str) {
            Some("secret") => Reply::Prompt {
                text: text("auth_message"),
                secret: true,
            },
            Some("visible") => Reply::Prompt {
                text: text("auth_message"),
                secret: false,
            },
            _ => Reply::Info(text("auth_message")),
        },
        Some("error") if reply.get("error_type").and_then(Value::as_str) == Some("auth_error") => {
            Reply::AuthFailed(text("description"))
        }
        _ => Reply::Error(text("description")),
    }
}

#[derive(Default)]
struct Control {
    epoch: u64,
    active: Option<UnixStream>,
}
type SharedControl = Arc<Mutex<Control>>;

struct Client {
    stream: UnixStream,
    path: Option<std::path::PathBuf>,
    control: Option<SharedControl>,
    epoch: u64,
    broken: bool,
}
impl Client {
    fn connect() -> Result<Self, String> {
        let path =
            std::env::var_os("GREETD_SOCK").ok_or_else(|| tr("shell-not-started-by-greetd"))?;
        let stream = UnixStream::connect(&path).map_err(|_| tr("shell-cannot-reach-greetd"))?;
        Ok(Self {
            stream,
            path: Some(path.into()),
            control: None,
            epoch: 0,
            broken: false,
        })
    }
    fn call(&mut self, request: Value) -> Result<Reply, String> {
        if self.broken {
            let path = self.path.as_ref().ok_or_else(|| tr("shell-lost-greetd"))?;
            self.stream = UnixStream::connect(path).map_err(|_| tr("shell-cannot-reach-greetd"))?;
            self.broken = false;
            if !matches!(self.call(json!({"type":"cancel_session"}))?, Reply::Success) {
                self.broken = true;
                return Err(tr("shell-lost-greetd").into());
            }
        }
        if let Some(control) = &self.control {
            let mut control = control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if control.epoch != self.epoch {
                self.broken = true;
                return Err(tr("shell-lost-greetd").into());
            }
            control.active = Some(
                self.stream
                    .try_clone()
                    .map_err(|_| tr("shell-lost-greetd"))?,
            );
        }
        let result = self.exchange(request);
        if let Some(control) = &self.control {
            control
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .active = None;
        }
        self.broken = result.is_err();
        result
    }
    fn exchange(&mut self, request: Value) -> Result<Reply, String> {
        // PAM/MFA can legitimately wait. User cancellation independently shuts
        // down this socket; the five-minute bound is for a broken transport.
        let deadline = Instant::now() + Duration::from_secs(300);
        let body = request.to_string();
        let length = u32::try_from(body.len()).map_err(|_| tr("shell-request-too-large"))?;
        let write_deadline = Instant::now() + Duration::from_secs(10);
        for bytes in [&length.to_ne_bytes()[..], body.as_bytes()] {
            let mut bytes = bytes;
            while !bytes.is_empty() {
                let left = write_deadline
                    .checked_duration_since(Instant::now())
                    .ok_or_else(|| tr("shell-lost-greetd"))?;
                self.stream
                    .set_write_timeout(Some(left))
                    .map_err(|_| tr("shell-lost-greetd"))?;
                let n = self
                    .stream
                    .write(bytes)
                    .map_err(|_| tr("shell-lost-greetd"))?;
                if n == 0 {
                    return Err(tr("shell-lost-greetd").into());
                }
                bytes = &bytes[n..];
            }
        }
        let mut header = [0u8; 4];
        read_before(&mut self.stream, &mut header, deadline)
            .map_err(|_| tr("shell-lost-greetd"))?;
        let length = u32::from_ne_bytes(header) as usize;
        if length > 1 << 20 {
            return Err(tr("shell-greetd-sent-an-oversized-reply").into());
        }
        let mut body = vec![0u8; length];
        read_before(&mut self.stream, &mut body, deadline).map_err(|_| tr("shell-lost-greetd"))?;
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| tr("shell-greetd-sent-an-invalid-reply"))?;
        Ok(parse(&value))
    }
}

fn read_before(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !bytes.is_empty() {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or(std::io::ErrorKind::TimedOut)?;
        stream.set_read_timeout(Some(left))?;
        let n = stream.read(bytes)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes = &mut bytes[n..];
    }
    Ok(())
}

pub(crate) enum Request {
    /// Selects the account; PAM starts only when Answer is submitted.
    Begin(String),
    /// Ends the sign-in in progress without starting another.
    Cancel,
    Answer(String),
    Launch {
        command: Vec<String>,
        env: Vec<String>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Prompt { text: String, secret: bool },
    Notice(String),
    Failed(String),
    Authenticated,
    Launched,
    Error(String),
}

// Where the conversation goes: greetd, or a stand-in for the nested preview.
enum Backend {
    Greetd(Client),
    #[cfg(any(test, feature = "greeter-preview"))]
    Preview {
        expected: String,
        asked: bool,
    },
}
impl Backend {
    fn invalidate(&mut self) {
        match self {
            Self::Greetd(client) => {
                client.broken = true;
                let _ = client.stream.shutdown(std::net::Shutdown::Both);
            }
            #[cfg(any(test, feature = "greeter-preview"))]
            Self::Preview { asked, .. } => *asked = false,
        }
    }
    fn call(&mut self, request: Value) -> Result<Reply, String> {
        match self {
            Self::Greetd(client) => client.call(request),
            #[cfg(any(test, feature = "greeter-preview"))]
            Self::Preview { expected, asked } => Ok(match request["type"].as_str() {
                Some("create_session") => {
                    *asked = true;
                    Reply::Prompt {
                        text: "Password:".into(),
                        secret: true,
                    }
                }
                Some("post_auth_message_response") if *asked => {
                    *asked = false;
                    if request["response"].as_str() == Some(expected.as_str()) {
                        Reply::Success
                    } else {
                        Reply::AuthFailed(tr("shell-authentication-failed").into())
                    }
                }
                Some("start_session" | "cancel_session") => Reply::Success,
                _ => Reply::Error(tr("shell-unexpected-request").into()),
            }),
        }
    }
}

// Runs the conversation until `requests` closes.
pub(crate) fn run(requests: mpsc::Receiver<Request>, events: mpsc::Sender<Event>) {
    let backend = match Client::connect() {
        Ok(client) => Backend::Greetd(client),
        #[cfg(feature = "greeter-preview")]
        Err(_)
            if std::env::var("ENZUI_SHELL_PLATFORM_PREVIEW").as_deref() == Ok("1")
                && std::env::var("ENZUI_LOCK_TEST_PASSWORD").is_ok() =>
        {
            Backend::Preview {
                expected: std::env::var("ENZUI_LOCK_TEST_PASSWORD").expect("checked fixture"),
                asked: false,
            }
        }
        Err(error) => {
            let _ = events.send(Event::Error(error));
            return;
        }
    };
    conversation(backend, requests, events);
}

fn conversation(
    mut backend: Backend,
    requests: mpsc::Receiver<Request>,
    events: mpsc::Sender<Event>,
) {
    let control = Arc::new(Mutex::new(Control::default()));
    match &mut backend {
        Backend::Greetd(client) => client.control = Some(Arc::clone(&control)),
        #[cfg(any(test, feature = "greeter-preview"))]
        Backend::Preview { .. } => {}
    }
    let (tx, rx) = mpsc::channel();
    let worker_control = Arc::clone(&control);
    let worker =
        std::thread::spawn(move || conversation_worker(backend, rx, events, worker_control));
    while let Ok(request) = requests.recv() {
        let mut current = control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(request, Request::Cancel | Request::Begin(_)) {
            current.epoch += 1;
            if let Some(stream) = current.active.take() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
        if tx.send((current.epoch, request)).is_err() {
            break;
        }
    }
    let mut current = control
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    current.epoch += 1;
    if let Some(stream) = current.active.take() {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
    drop(current);
    drop(tx);
    let _ = worker.join();
}

fn conversation_worker(
    mut backend: Backend,
    requests: mpsc::Receiver<(u64, Request)>,
    events: mpsc::Sender<Event>,
    control: SharedControl,
) {
    let mut user: Option<String> = None;
    let mut active = false;
    let epoch_now = || {
        control
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .epoch
    };
    while let Ok((epoch, request)) = requests.recv() {
        if epoch != epoch_now() {
            continue;
        }
        match &mut backend {
            Backend::Greetd(client) => client.epoch = epoch,
            #[cfg(any(test, feature = "greeter-preview"))]
            Backend::Preview { .. } => {}
        }
        let mut first_answer = None;
        let reply = match request {
            Request::Begin(name) => {
                if active {
                    let _ = backend.call(json!({"type": "cancel_session"}));
                }
                active = false;
                user = Some(name);
                continue;
            }
            Request::Cancel => {
                if std::mem::take(&mut active) {
                    let _ = backend.call(json!({"type": "cancel_session"}));
                }
                user = None;
                continue;
            }
            Request::Answer(secret) => {
                if active {
                    backend.call(json!({"type": "post_auth_message_response", "response": secret}))
                } else if let Some(name) = user.as_ref() {
                    active = true;
                    // A blank initial submit starts PAM; it must not answer a
                    // new-password challenge before the user has seen it.
                    first_answer = (!secret.is_empty()).then_some(secret);
                    backend.call(json!({"type": "create_session", "username": name}))
                } else {
                    continue;
                }
            }
            Request::Launch { command, env } => {
                let reply =
                    backend.call(json!({"type": "start_session", "cmd": command, "env": env}));
                if epoch != epoch_now() {
                    backend.invalidate();
                    active = false;
                    user = None;
                    continue;
                }
                let _ = events.send(match reply {
                    Ok(Reply::Success) => Event::Launched,
                    Ok(Reply::Error(e) | Reply::AuthFailed(e)) => Event::Error(if e.is_empty() {
                        tr("shell-the-session-could-not-start").into()
                    } else {
                        e
                    }),
                    Ok(_) => Event::Error(tr("shell-the-session-could-not-start").into()),
                    Err(e) => Event::Error(e),
                });
                continue;
            }
        };
        let mut reply = reply;
        // Follows the conversation to the next thing the user must answer.
        loop {
            if epoch != epoch_now() {
                backend.invalidate();
                active = false;
                user = None;
                break;
            }
            match reply {
                Ok(Reply::Prompt { text, secret }) => {
                    // Only the first secret prompt may consume the submitted password.
                    // Visible challenges and subsequent factors need their own answer.
                    if secret && let Some(answer) = first_answer.take() {
                        reply = backend.call(
                            json!({"type": "post_auth_message_response", "response": answer}),
                        );
                        continue;
                    }
                    let _ = events.send(Event::Prompt { text, secret });
                    break;
                }
                Ok(Reply::Info(text)) => {
                    let _ = events.send(Event::Notice(text));
                    reply = backend
                        .call(json!({"type": "post_auth_message_response", "response": null}));
                }
                Ok(Reply::Success) => {
                    let _ = events.send(Event::Authenticated);
                    break;
                }
                Ok(Reply::AuthFailed(text)) => {
                    std::thread::sleep(Duration::from_millis(800));
                    if epoch != epoch_now() {
                        backend.invalidate();
                        active = false;
                        user = None;
                        break;
                    }
                    let _ = events.send(Event::Failed(text));
                    let _ = backend.call(json!({"type": "cancel_session"}));
                    active = false;
                    break;
                }
                Ok(Reply::Error(text)) | Err(text) => {
                    let _ = backend.call(json!({"type": "cancel_session"}));
                    active = false;
                    let _ = events.send(Event::Error(if text.is_empty() {
                        tr("shell-signing-in-failed").into()
                    } else {
                        text
                    }));
                    break;
                }
            }
        }
    }
    // Controller closure has already interrupted any active transport.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greetd_behaviour() {
        // Selection and failed attempt do not start pam.
        {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("greetd.sock");
            let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            let client = UnixStream::connect(&path).unwrap();
            let (mut server, _) = listener.accept().unwrap();
            server
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let (tx, rx) = mpsc::channel();
            let (events, replies) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                conversation(
                    Backend::Greetd(Client {
                        stream: client,
                        path: Some(path),
                        control: None,
                        epoch: 0,
                        broken: false,
                    }),
                    rx,
                    events,
                );
            });
            let request = |server: &mut UnixStream| {
                let mut header = [0; 4];
                server.read_exact(&mut header).unwrap();
                let mut body = vec![0; u32::from_ne_bytes(header) as usize];
                server.read_exact(&mut body).unwrap();
                serde_json::from_slice::<Value>(&body).unwrap()
            };
            let reply = |server: &mut UnixStream, value: Value| {
                let bytes = value.to_string().into_bytes();
                server
                    .write_all(&(bytes.len() as u32).to_ne_bytes())
                    .unwrap();
                server.write_all(&bytes).unwrap();
            };
            tx.send(Request::Begin("first".into())).unwrap();
            tx.send(Request::Begin("second".into())).unwrap();
            assert!(server.read(&mut [0]).is_err());
            tx.send(Request::Answer("wrong".into())).unwrap();
            assert_eq!(
                request(&mut server),
                json!({"type":"create_session", "username":"second"})
            );
            reply(
                &mut server,
                json!({"type":"auth_message", "auth_message_type":"secret", "auth_message":"Password:"}),
            );
            assert_eq!(
                request(&mut server),
                json!({"type":"post_auth_message_response", "response":"wrong"})
            );
            reply(
                &mut server,
                json!({"type":"error", "error_type":"auth_error", "description":"bad"}),
            );
            assert!(matches!(
                replies.recv_timeout(Duration::from_secs(2)).unwrap(),
                Event::Failed(_)
            ));
            assert_eq!(request(&mut server)["type"], "cancel_session");
            reply(&mut server, json!({"type":"success"}));
            assert!(server.read(&mut [0]).is_err());
            tx.send(Request::Answer("right".into())).unwrap();
            assert_eq!(request(&mut server)["type"], "create_session");
            reply(
                &mut server,
                json!({"type":"auth_message", "auth_message_type":"secret", "auth_message":"Password:"}),
            );
            assert_eq!(request(&mut server)["response"], "right");
            reply(
                &mut server,
                json!({"type":"auth_message", "auth_message_type":"visible", "auth_message":"Token:"}),
            );
            assert!(matches!(
                replies.recv_timeout(Duration::from_secs(1)).unwrap(),
                Event::Prompt { secret: false, .. }
            ));
            assert!(server.read(&mut [0]).is_err());
            tx.send(Request::Cancel).unwrap();
            assert_eq!(request(&mut server)["type"], "cancel_session");
            reply(&mut server, json!({"type":"success"}));
            assert!(server.read(&mut [0]).is_err());
            server
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            // A partial reply must not hold the cancel request behind the blocked read.
            tx.send(Request::Begin("stalled".into())).unwrap();
            tx.send(Request::Answer("secret".into())).unwrap();
            assert_eq!(request(&mut server)["type"], "create_session");
            server.write_all(&[10, 0]).unwrap();
            tx.send(Request::Cancel).unwrap();
            assert_eq!(server.read(&mut [0]).unwrap(), 0);
            tx.send(Request::Begin("retry".into())).unwrap();
            tx.send(Request::Answer(String::new())).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut retry = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("owned fake greetd retry failed: {error}"),
                }
            };
            retry
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            assert_eq!(request(&mut retry)["type"], "cancel_session");
            reply(&mut retry, json!({"type":"success"}));
            let next = request(&mut retry);
            assert_eq!(next["username"], "retry");
            reply(
                &mut retry,
                json!({"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}),
            );
            assert!(matches!(
                replies.recv_timeout(Duration::from_secs(2)).unwrap(),
                Event::Prompt { secret: true, .. }
            ));
            tx.send(Request::Cancel).unwrap();
            assert_eq!(request(&mut retry)["type"], "cancel_session");
            reply(&mut retry, json!({"type":"success"}));
            drop(tx);
            worker.join().unwrap();
        }
        // Blank begin waits for the first login password prompt.
        {
            let (client, mut server) = UnixStream::pair().unwrap();
            server
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let (tx, rx) = mpsc::channel();
            let (events, replies) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                conversation(
                    Backend::Greetd(Client {
                        stream: client,
                        path: None,
                        control: None,
                        epoch: 0,
                        broken: false,
                    }),
                    rx,
                    events,
                );
            });
            let request = |server: &mut UnixStream| {
                let mut header = [0; 4];
                server.read_exact(&mut header).unwrap();
                let mut body = vec![0; u32::from_ne_bytes(header) as usize];
                server.read_exact(&mut body).unwrap();
                serde_json::from_slice::<Value>(&body).unwrap()
            };
            let reply = |server: &mut UnixStream, value: Value| {
                let bytes = value.to_string().into_bytes();
                server
                    .write_all(&(bytes.len() as u32).to_ne_bytes())
                    .unwrap();
                server.write_all(&bytes).unwrap();
            };
            tx.send(Request::Begin("new-user".into())).unwrap();
            tx.send(Request::Answer(String::new())).unwrap();
            assert_eq!(request(&mut server)["type"], "create_session");
            for prompt in ["New password:", "Retype new password:"] {
                reply(
                    &mut server,
                    json!({"type":"auth_message","auth_message_type":"secret","auth_message":prompt}),
                );
                assert!(matches!(
                    replies.recv_timeout(Duration::from_secs(1)).unwrap(),
                    Event::Prompt { secret: true, .. }
                ));
                assert!(server.read(&mut [0]).is_err());
                tx.send(Request::Answer("test-only-new-password".into()))
                    .unwrap();
                assert_eq!(request(&mut server)["response"], "test-only-new-password");
            }
            reply(&mut server, json!({"type":"success"}));
            assert!(matches!(
                replies.recv_timeout(Duration::from_secs(1)).unwrap(),
                Event::Authenticated
            ));
            drop(tx);
            assert_eq!(server.read(&mut [0]).unwrap(), 0);
            worker.join().unwrap();
        }
        // Replies and the preview conversation.
        {
            assert_eq!(
                parse(
                    &json!({"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password: "})
                ),
                Reply::Prompt {
                    text: "Password: ".into(),
                    secret: true
                }
            );
            assert_eq!(
                parse(
                    &json!({"type": "auth_message", "auth_message_type": "info", "auth_message": "Hi"})
                ),
                Reply::Info("Hi".into())
            );
            assert_eq!(
                parse(&json!({"type": "error", "error_type": "auth_error", "description": "no"})),
                Reply::AuthFailed("no".into())
            );
            assert_eq!(
                parse(&json!({"type": "error", "error_type": "error", "description": "x"})),
                Reply::Error("x".into())
            );
            assert_eq!(parse(&json!({"type": "success"})), Reply::Success);

            let mut preview = Backend::Preview {
                expected: "right".into(),
                asked: false,
            };
            assert!(matches!(
                preview.call(json!({"type": "create_session", "username": "u"})),
                Ok(Reply::Prompt { secret: true, .. })
            ));
            assert!(matches!(
                preview.call(json!({"type": "post_auth_message_response", "response": "wrong"})),
                Ok(Reply::AuthFailed(_))
            ));
            preview
                .call(json!({"type": "create_session", "username": "u"}))
                .unwrap();
            assert_eq!(
                preview.call(json!({"type": "post_auth_message_response", "response": "right"})),
                Ok(Reply::Success)
            );
        }
    }
}
