//! One-connection-per-request harness client; a streamed turn retains its connection until
//! completion.

use enzui_shell::agent_harness::{self, ApprovalMode};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use iced::futures::SinkExt;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

use super::Screenshots;

const MAX_FRAME: usize = 1 << 20;

// ENZUI_AGENT_MOCK=1 runs the scripted development agent instead of the harness.
fn mock() -> bool {
    std::env::var_os("ENZUI_AGENT_MOCK").is_some()
}

fn socket() -> Option<PathBuf> {
    std::env::var_os("ENZUI_AGENT_SOCKET")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            if mock() {
                enzui_shell::agent_mock::socket_path()
            } else {
                agent_harness::socket_path()
            }
        })
}

fn owns_mock() -> bool {
    mock() && std::env::var_os("ENZUI_AGENT_SOCKET").is_none()
}

// The shell runs the harness user service, unless another socket was named.
fn owns_harness() -> bool {
    !mock() && std::env::var_os("ENZUI_AGENT_SOCKET").is_none()
}

/// Open a session in the first folder configured in Settings › Agents.
pub(super) async fn open(mode: ApprovalMode, screenshots: Screenshots) -> Result<String, String> {
    let mut fields = if socket() == agent_harness::socket_path() {
        tokio::task::spawn_blocking(|| {
            let config = agent_harness::load()?.unwrap_or_else(agent_harness::starter);
            let setup = agent_harness::summary(&config);
            let unset =
                || tr("shell-choose-a-workspace-folder-in-settings-agents-first").to_owned();
            let workspace = setup.roots.first().cloned().ok_or_else(unset)?;
            let model = if setup.model.is_empty() && setup.provider != "local" {
                agent_harness::models(&setup.gateway)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| tr("shell-the-gateway-lists-no-models").to_owned())?
            } else {
                setup.model
            };
            Ok::<_, String>(
                json!({"workspace": workspace, "model": model, "provider": setup.provider}),
            )
        })
        .await
        .map_err(|_| tr("shell-a-new-chat-could-not-be-started").to_owned())??
    } else {
        json!({})
    };
    fields["approval_mode"] = json!(if mode == ApprovalMode::FullSend {
        ApprovalMode::Ask.wire()
    } else {
        mode.wire()
    });
    fields["screenshots"] = json!(screenshots.wire());
    let created = request("session.create", fields).await?;
    let id = created["session_id"]
        .as_str()
        .or_else(|| created["id"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| tr("shell-a-new-chat-could-not-be-started").to_owned())?;
    if mode == ApprovalMode::FullSend
        && let Err(error) = set_approval_mode(&id, mode).await
    {
        let _ = request("session.close", json!({"session_id": id})).await;
        return Err(error);
    }
    Ok(id)
}

pub(super) async fn set_approval_mode(id: &str, mode: ApprovalMode) -> Result<(), String> {
    let result = request(
        "session.set_approval_mode",
        json!({
            "session_id": id,
            "approval_mode": mode.wire(),
            "confirm_full_send": mode == ApprovalMode::FullSend,
        }),
    )
    .await?;
    if result["approval_mode"] == mode.wire() {
        Ok(())
    } else {
        Err(tr("shell-agent-approval-mode-unavailable").into())
    }
}

/// The model a session uses, as the composer names it.
fn model_label() -> String {
    if mock() {
        return tr("shell-mock-agent").into();
    }
    if socket() != agent_harness::socket_path() {
        return String::new();
    }
    agent_harness::load()
        .ok()
        .flatten()
        .map(|config| agent_harness::summary(&config).model)
        .filter(|model| !model.is_empty())
        .unwrap_or_default()
}

/// A first configuration for the harness: read-only, in the Documents folder.
fn first_configuration() -> Result<(), String> {
    if agent_harness::load()?.is_some() {
        return Ok(());
    }
    let mut config = agent_harness::starter();
    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
    {
        let documents = home.join("Documents");
        if std::fs::create_dir_all(&documents).is_ok() {
            agent_harness::apply(
                &mut config,
                &agent_harness::Edit::AddRoot(documents.to_string_lossy().into_owned()),
            )?;
        }
    }
    agent_harness::save(&config)
}

async fn connect() -> Result<tokio::net::UnixStream, String> {
    let path = socket().ok_or_else(|| tr("shell-no-runtime-folder"))?;
    let stream = tokio::net::UnixStream::connect(&path)
        .await
        .map_err(|_| tr("shell-the-assistant-service-is-not-running").to_owned())?;
    let peer = stream
        .peer_cred()
        .map_err(|_| tr("shell-the-assistant-service-could-not-be-identified"))?;
    if peer.uid() != rustix::process::getuid().as_raw() {
        return Err(tr("shell-the-assistant-service-belongs-to-another-user").into());
    }
    Ok(stream)
}

fn frame(method: &str, fields: Value) -> Vec<u8> {
    let mut body = json!({"schema_version": 1, "method": method});
    for (key, value) in fields.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    let mut raw = serde_json::to_vec(&body).unwrap_or_default();
    raw.push(b'\n');
    raw
}

async fn line(reader: &mut (impl tokio::io::AsyncBufRead + Unpin)) -> Result<Value, String> {
    let mut line = Vec::new();
    reader
        .take(MAX_FRAME as u64 + 1)
        .read_until(b'\n', &mut line)
        .await
        .map_err(|_| tr("shell-the-assistant-connection-was-lost"))?;
    if line.is_empty() {
        return Err(tr("shell-the-assistant-service-closed-the-connection").into());
    }
    if line.len() > MAX_FRAME || !line.ends_with(b"\n") {
        return Err(tr("shell-the-assistant-service-sent-too-much-at-once").into());
    }
    serde_json::from_slice(&line)
        .map_err(|_| tr("shell-the-assistant-service-sent-an-unknown-frame").into())
}

/// One request and its reply.
pub(super) async fn request(method: &'static str, fields: Value) -> Result<Value, String> {
    let mut stream = connect().await?;
    stream
        .write_all(&frame(method, fields))
        .await
        .map_err(|_| tr("shell-the-assistant-service-did-not-accept-the-request"))?;
    let mut reader = tokio::io::BufReader::new(stream);
    let reply = tokio::time::timeout(std::time::Duration::from_secs(15), line(&mut reader))
        .await
        .map_err(|_| tr("shell-the-assistant-service-did-not-answer"))??;
    match reply["ok"].as_bool() {
        Some(true) => Ok(reply["result"].clone()),
        _ => Err(reply["error"].as_str().map_or_else(|| tr("shell-the-assistant-refused-the-request").to_owned(), |error| tr!("shell-the-assistant-refused", "arg0" => error.chars().take(160).collect::<String>()))),
    }
}

/// What a running turn reports.
#[derive(Clone, Debug)]
pub(crate) enum TurnEvent {
    Event(Value),
    Finished(Result<String, String>),
}

// The turn's event stream; its connection lives as long as the stream.
pub(super) fn turn(
    session: String,
    prompt: String,
    mode: ApprovalMode,
    screenshots: Screenshots,
    cancelled: tokio::sync::oneshot::Receiver<()>,
) -> impl iced::futures::Stream<Item = TurnEvent> {
    iced::stream::channel(
        64,
        async move |mut output: iced::futures::channel::mpsc::Sender<TurnEvent>| {
            let exchange = async {
                let mut stream = connect().await?;
                stream
                    .write_all(&frame(
                        "turn.start",
                        json!({"session_id": session, "prompt": prompt, "approval_mode": mode.wire(), "screenshots": screenshots.wire()}),
                    ))
                    .await
                    .map_err(|_| tr("shell-the-assistant-did-not-accept-the-message"))?;
                let mut reader = tokio::io::BufReader::new(stream);
                let mut frame = tokio::time::timeout(Duration::from_secs(30), line(&mut reader))
                    .await
                    .map_err(|_| tr("shell-the-assistant-service-did-not-answer"))??;
                let mut pending: Option<(Value, Instant)> = None;
                loop {
                    if let Some(event) = frame.get_mut("event").map(Value::take) {
                        if event["type"] == "message.delta" {
                            match pending.as_mut() {
                                Some((batch, _)) => {
                                    if let Some(Value::String(text)) = batch.get_mut("text") {
                                        text.push_str(event["text"].as_str().unwrap_or_default());
                                    }
                                }
                                None => {
                                    pending =
                                        Some((event, Instant::now() + Duration::from_millis(16)));
                                }
                            }
                        } else {
                            if let Some((batch, _)) = pending.take() {
                                let _ = output.send(TurnEvent::Event(batch)).await;
                            }
                            let _ = output.send(TurnEvent::Event(event)).await;
                        }
                    } else {
                        if let Some((batch, _)) = pending.take() {
                            let _ = output.send(TurnEvent::Event(batch)).await;
                        }
                        if frame["ok"] == Value::Bool(true) {
                            return Ok(frame["status"].as_str().unwrap_or("completed").to_owned());
                        }
                        let code = frame["error"].as_str().filter(|error| !error.is_empty());
                        return Err(code.map_or_else(
                            || tr("shell-the-turn-failed").to_owned(),
                            |code| agent_harness::turn_error(code).unwrap_or(code).to_owned(),
                        ));
                    }
                    let read = line(&mut reader);
                    tokio::pin!(read);
                    let next = if let Some((_, deadline)) = &pending {
                        tokio::select! {
                            biased;
                            () = tokio::time::sleep_until(*deadline) => {
                                if let Some((batch, _)) = pending.take() {
                                    let _ = output.send(TurnEvent::Event(batch)).await;
                                }
                                read.await
                            }
                            result = &mut read => result,
                        }
                    } else {
                        read.await
                    };
                    if next.is_err()
                        && let Some((batch, _)) = pending.take()
                    {
                        let _ = output.send(TurnEvent::Event(batch)).await;
                    }
                    frame = next?;
                }
            };
            let result = tokio::select! {
                biased;
                _ = cancelled => Ok("cancelled".to_owned()),
                result = tokio::time::timeout(Duration::from_secs(610), exchange) => {
                    result.unwrap_or_else(|_| Err(tr("shell-assistant-provider-timeout").into()))
                },
            };
            let _ = output.send(TurnEvent::Finished(result)).await;
        },
    )
}

/// The service's state, as the subscription that runs it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Service {
    /// Answering; the composer names the model.
    Ready(String),
    Failed(String),
}

// Starts the harness user service (or, for development, the mock) while the assistant is
// enabled; an ENZUI_AGENT_SOCKET service is only observed.
pub(super) fn service() -> iced::Subscription<Service> {
    iced::Subscription::run(|| {
        iced::stream::channel(
            4,
            async move |mut output: iced::futures::channel::mpsc::Sender<Service>| {
                loop {
                    let mut child = None;
                    if owns_harness() {
                        let started = async {
                            if !agent_harness::installed() {
                                return Err(
                                    tr("shell-the-assistant-service-is-not-running").to_owned()
                                );
                            }
                            tokio::task::spawn_blocking(first_configuration)
                                .await
                                .map_err(|_| {
                                    tr("shell-the-assistant-service-did-not-start").to_owned()
                                })??;
                            if !agent_harness::running().await {
                                agent_harness::set_running(true).await?;
                            }
                            Ok(())
                        }
                        .await;
                        if let Err(error) = started {
                            let _ = output.send(Service::Failed(error)).await;
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                            continue;
                        }
                    }
                    if owns_mock() {
                        let binary = std::env::current_exe()
                            .ok()
                            .and_then(|exe| exe.parent().map(|dir| dir.join("enzui-agent-mock")));
                        match binary.filter(|path| path.is_file()) {
                            Some(binary) => match tokio::process::Command::new(binary)
                                .env("ENZUI_AGENT_MOCK_PARENT", "stdin")
                                .stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::null())
                                .kill_on_drop(true)
                                .spawn()
                            {
                                Ok(spawned) => child = Some(spawned),
                                Err(_) => {
                                    let _ = output
                                        .send(Service::Failed(
                                            "The assistant service could not start".into(),
                                        ))
                                        .await;
                                }
                            },
                            None => {
                                let _ = output
                                    .send(Service::Failed(
                                        "The assistant service is not installed".into(),
                                    ))
                                    .await;
                            }
                        }
                    }
                    let mut ready = false;
                    for _ in 0..100 {
                        if connect().await.is_ok() {
                            ready = true;
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    if ready {
                        let label = tokio::task::spawn_blocking(model_label)
                            .await
                            .unwrap_or_default();
                        let _ = output.send(Service::Ready(label)).await;
                    } else {
                        let _ = output
                            .send(Service::Failed(
                                tr("shell-the-assistant-service-is-not-running").into(),
                            ))
                            .await;
                    }
                    match child.as_mut() {
                        Some(child) => {
                            // `wait` would close stdin first, which ends the mock.
                            let _stdin = child.stdin.take();
                            let _ = child.wait().await;
                            let _ = output
                                .send(Service::Failed("The assistant service stopped".into()))
                                .await;
                        }
                        None => {
                            // An external service: look again later.
                            tokio::time::sleep(std::time::Duration::from_secs(if ready {
                                30
                            } else {
                                3
                            }))
                            .await;
                            continue;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            },
        )
    })
}
