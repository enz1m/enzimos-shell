//! Scripted agent-harness protocol v1 server for assistant previews.
//!
//! Streams reasoning, messages and tool output until the final turn status.
//! Prompt keywords select fixtures; this service executes no model-generated tools.

use serde_json::{Value, json};
use std::fmt::Write as _;
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot, watch},
};

const MAX_FRAME: usize = 1 << 20;

/// Where the mock listens: `$XDG_RUNTIME_DIR/enzui-agent-mock/service.sock`.
#[must_use]
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())?;
    Some(runtime.join("enzui-agent-mock/service.sock"))
}

// One step of a scripted reply.
#[derive(Clone, Debug)]
enum Step {
    Think(&'static str),
    Say(String),
    Tool {
        tool: &'static str,
        title: String,
        arguments: Value,
        output: String,
        millis: u64,
        failed: bool,
    },
    Ask {
        tool: &'static str,
        title: String,
        arguments: Value,
        output: String,
    },
    Fail(&'static str),
}

/// A transcript record as `session.export` returns it in `items`.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    User(String),
    Assistant(String),
    Reasoning {
        text: String,
        millis: u64,
    },
    Tool {
        tool: String,
        title: String,
        arguments: Value,
        output: String,
        status: String,
        millis: u64,
    },
    Note(String),
}

impl Item {
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::User(text) => json!({"kind": "user", "text": text}),
            Self::Assistant(text) => json!({"kind": "assistant", "text": text}),
            Self::Reasoning { text, millis } => {
                json!({"kind": "reasoning", "text": text, "duration_ms": millis})
            }
            Self::Tool {
                tool,
                title,
                arguments,
                output,
                status,
                millis,
            } => {
                json!({"kind": "tool", "tool": tool, "title": title, "arguments": arguments, "output": output, "status": status, "duration_ms": millis})
            }
            Self::Note(text) => json!({"kind": "note", "text": text}),
        }
    }
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value[key].as_str().unwrap_or("").to_owned();
        let millis = value["duration_ms"].as_u64().unwrap_or(0);
        Some(match value["kind"].as_str()? {
            "user" => Self::User(text("text")),
            "assistant" => Self::Assistant(text("text")),
            "reasoning" => Self::Reasoning {
                text: text("text"),
                millis,
            },
            "tool" => Self::Tool {
                tool: text("tool"),
                title: text("title"),
                arguments: value["arguments"].clone(),
                output: text("output"),
                status: text("status"),
                millis,
            },
            "note" => Self::Note(text("text")),
            _ => return None,
        })
    }
}

struct Session {
    id: String,
    title: String,
    updated: i64,
    items: Vec<Item>,
}

struct Live {
    cancel: watch::Sender<bool>,
    steer: mpsc::UnboundedSender<String>,
    approvals: HashMap<String, oneshot::Sender<bool>>,
}

#[derive(Default)]
struct State {
    sessions: Vec<Session>,
    live: HashMap<String, Live>,
    next: u64,
    images: PathBuf,
}

type Shared = Arc<Mutex<State>>;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Binds the private socket and serves until the process ends.
pub async fn serve(path: &Path) -> Result<(), String> {
    let directory = path.parent().ok_or("Invalid socket path")?;
    std::fs::create_dir_all(directory).map_err(|_| "The socket folder cannot be created")?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "The socket folder cannot be made private")?;
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).map_err(|_| "The socket cannot be bound")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| "The socket cannot be made private")?;
    let images = directory.join("images");
    std::fs::create_dir_all(&images).map_err(|_| "The image folder cannot be created")?;
    write_chart(&images.join("throughput.png"))
        .map_err(|()| "The example chart cannot be written")?;
    let state: Shared = Arc::new(Mutex::new(State {
        images,
        ..State::default()
    }));
    seed(&state);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let state = std::sync::Arc::clone(&state);
        tokio::spawn(async move {
            let _ = connection(stream, state).await;
        });
    }
}

async fn connection(stream: UnixStream, state: Shared) -> Result<(), ()> {
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read);
    let mut line = Vec::new();
    (&mut reader)
        .take(MAX_FRAME as u64 + 1)
        .read_until(b'\n', &mut line)
        .await
        .map_err(drop)?;
    if line.len() > MAX_FRAME || !line.ends_with(b"\n") {
        return send(&mut write, &json!({"ok": false, "error": "invalid frame"})).await;
    }
    let Ok(request) = serde_json::from_slice::<Value>(&line) else {
        return send(&mut write, &json!({"ok": false, "error": "invalid JSON"})).await;
    };
    if request["schema_version"] != json!(1) {
        return send(
            &mut write,
            &json!({"ok": false, "error": "unsupported schema"}),
        )
        .await;
    }
    let session = request["session_id"].as_str().unwrap_or("").to_owned();
    let reply = match request["method"].as_str().unwrap_or("") {
        "turn.start" => {
            return turn(
                &mut write,
                &state,
                session,
                request["prompt"].as_str().unwrap_or("").to_owned(),
            )
            .await;
        }
        "session.list" => Ok(list(&state)),
        "session.create" => Ok(create(&state, request["title"].as_str())),
        "session.resume" => exists(&state, &session).map(|()| json!({})),
        "session.export" => export(&state, &session),
        "session.close" => Ok(json!({})),
        "session.delete" => {
            let mut state = state.lock().map_err(drop)?;
            state.sessions.retain(|known| known.id != session);
            Ok(json!({}))
        }
        "session.rename" => {
            let mut state = state.lock().map_err(drop)?;
            let title = request["title"]
                .as_str()
                .unwrap_or("")
                .trim()
                .chars()
                .take(80)
                .collect::<String>();
            match state.sessions.iter_mut().find(|known| known.id == session) {
                Some(known) if !title.is_empty() => {
                    known.title = title;
                    Ok(json!({}))
                }
                _ => Err("unknown session or empty title".to_owned()),
            }
        }
        "turn.cancel" => {
            let state = state.lock().map_err(drop)?;
            match state.live.get(&session) {
                Some(live) => {
                    let _ = live.cancel.send(true);
                    Ok(json!({}))
                }
                None => Err("no running turn".into()),
            }
        }
        "turn.steer" => {
            let state = state.lock().map_err(drop)?;
            let text = request["text"].as_str().unwrap_or("").trim().to_owned();
            match state.live.get(&session) {
                Some(live) if !text.is_empty() => {
                    let _ = live.steer.send(text);
                    Ok(json!({}))
                }
                Some(_) => Err("empty steering message".into()),
                None => Err("no running turn".into()),
            }
        }
        "approval.resolve" => {
            let mut state = state.lock().map_err(drop)?;
            let request_id = request["request_id"].as_str().unwrap_or("");
            match state
                .live
                .get_mut(&session)
                .and_then(|live| live.approvals.remove(request_id))
            {
                Some(answer) => {
                    let _ = answer.send(request["allow"].as_bool().unwrap_or(false));
                    Ok(json!({}))
                }
                None => Err("stale approval".into()),
            }
        }
        _ => Err("unknown method".into()),
    };
    match reply {
        Ok(result) => send(&mut write, &json!({"ok": true, "result": result})).await,
        Err(error) => send(&mut write, &json!({"ok": false, "error": error})).await,
    }
}

async fn send(write: &mut tokio::net::unix::OwnedWriteHalf, value: &Value) -> Result<(), ()> {
    let mut raw = serde_json::to_vec(value).map_err(drop)?;
    raw.push(b'\n');
    write.write_all(&raw).await.map_err(drop)
}

fn exists(state: &Shared, id: &str) -> Result<(), String> {
    let state = state.lock().map_err(|_| "state lost")?;
    state
        .sessions
        .iter()
        .any(|session| session.id == id)
        .then_some(())
        .ok_or_else(|| "unknown session".into())
}

fn list(state: &Shared) -> Value {
    let Ok(state) = state.lock() else {
        return json!([]);
    };
    let mut rows: Vec<&Session> = state.sessions.iter().collect();
    rows.sort_by_key(|session| std::cmp::Reverse(session.updated));
    Value::Array(
        rows.into_iter()
            .map(|session| {
                let preview = session
                    .items
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        Item::Assistant(text) | Item::User(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .unwrap_or("")
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("")
                    .trim_start_matches(['#', ' ', '>', '-', '*'])
                    .chars()
                    .take(90)
                    .collect::<String>();
                json!({"id": session.id, "title": session.title, "updated": session.updated, "preview": preview, "workspace": ""})
            })
            .collect(),
    )
}

fn create(state: &Shared, title: Option<&str>) -> Value {
    let Ok(mut state) = state.lock() else {
        return json!({});
    };
    state.next += 1;
    let id = format!("mock-{}", state.next);
    let title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("New chat")
        .chars()
        .take(80)
        .collect();
    state.sessions.push(Session {
        id: id.clone(),
        title,
        updated: now(),
        items: Vec::new(),
    });
    json!({"id": id})
}

fn export(state: &Shared, id: &str) -> Result<Value, String> {
    let state = state.lock().map_err(|_| "state lost")?;
    let session = state
        .sessions
        .iter()
        .find(|session| session.id == id)
        .ok_or("unknown session")?;
    let messages: Vec<Value> = session
        .items
        .iter()
        .filter_map(|item| match item {
            Item::User(text) => Some(json!({"role": "user", "content": text})),
            Item::Assistant(text) => Some(json!({"role": "assistant", "content": text})),
            _ => None,
        })
        .collect();
    Ok(
        json!({"id": session.id, "title": session.title, "messages": messages, "items": session.items.iter().map(Item::to_json).collect::<Vec<_>>()}),
    )
}

struct Pace(u64);
impl Pace {
    fn next(&mut self, low: u64, high: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        low + (self.0 >> 33) % (high - low + 1)
    }
}

fn pieces(text: &str, pace: &mut Pace) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut target = pace.next(2, 7) as usize;
    for character in text.chars() {
        current.push(character);
        if current.chars().count() >= target || character == '\n' {
            out.push(std::mem::take(&mut current));
            target = pace.next(2, 7) as usize;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

enum Interrupt {
    Cancelled,
    Steered(String),
}

async fn turn(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    state: &Shared,
    session: String,
    prompt: String,
) -> Result<(), ()> {
    let prompt = prompt.trim().to_owned();
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (steer_tx, mut steer) = mpsc::unbounded_channel();
    let (images, rejected) = {
        let mut guard = state.lock().map_err(drop)?;
        let images = guard.images.clone();
        let error = if prompt.is_empty() {
            Some("empty prompt")
        } else if !guard.sessions.iter().any(|known| known.id == session) {
            Some("unknown session")
        } else if guard.live.contains_key(&session) {
            Some("a turn is already running")
        } else {
            None
        };
        if error.is_none() {
            guard.live.insert(
                session.clone(),
                Live {
                    cancel: cancel_tx,
                    steer: steer_tx,
                    approvals: HashMap::new(),
                },
            );
            if let Some(known) = guard.sessions.iter_mut().find(|known| known.id == session) {
                if known.items.is_empty() && known.title == "New chat" {
                    known.title = title_for(&prompt);
                }
                known.items.push(Item::User(prompt.clone()));
                known.updated = now();
            }
        }
        (images, error)
    };
    if let Some(error) = rejected {
        return send(write, &json!({"ok": false, "error": error})).await;
    }
    let started = std::time::Instant::now();
    let mut pace = Pace(prompt.len() as u64 ^ 0x9e37_79b9);
    let mut steps = script(&prompt, &images);
    let mut produced: Vec<Item> = Vec::new();
    let mut output_tokens = 0usize;
    let mut status = "completed";
    let mut calls = 0;
    macro_rules! event {
        ($value:expr) => {
            if send(write, &json!({"event": $value})).await.is_err() {
                status = "cancelled";
                break;
            }
        };
    }
    if send(write, &json!({"event": {"type": "turn.started"}}))
        .await
        .is_err()
    {
        status = "cancelled";
    }
    let mut index = if status == "cancelled" {
        steps.len()
    } else {
        0
    };
    'steps: while index < steps.len() {
        let step = steps[index].clone();
        index += 1;
        // Waits, giving up early for a cancel or a steering message.
        macro_rules! pause {
            ($millis:expr) => {{
                let wait = tokio::time::sleep(Duration::from_millis($millis));
                tokio::select! {
                    () = wait => None,
                    _ = cancel.changed() => Some(Interrupt::Cancelled),
                    Some(text) = steer.recv() => Some(Interrupt::Steered(text)),
                }
            }};
        }
        macro_rules! interrupted {
            ($interrupt:expr) => {
                match $interrupt {
                    Some(Interrupt::Cancelled) => {
                        status = "cancelled";
                        break 'steps;
                    }
                    Some(Interrupt::Steered(text)) => {
                        event!(json!({"type": "steer.accepted", "text": text}));
                        produced.push(Item::User(text.clone()));
                        steps.truncate(index);
                        steps.extend(steered(&text));
                    }
                    None => {}
                }
            };
        }
        match step {
            Step::Think(text) => {
                let begun = std::time::Instant::now();
                for piece in pieces(text, &mut pace) {
                    event!(json!({"type": "reasoning.delta", "text": piece}));
                    let interrupt = pause!(pace.next(10, 22));
                    interrupted!(interrupt);
                }
                let millis = begun.elapsed().as_millis() as u64 + 400;
                event!(json!({"type": "reasoning.completed", "duration_ms": millis}));
                produced.push(Item::Reasoning {
                    text: text.to_owned(),
                    millis,
                });
            }
            Step::Say(text) => {
                let mut said = String::new();
                for piece in pieces(&text, &mut pace) {
                    said.push_str(&piece);
                    output_tokens += 1;
                    event!(json!({"type": "message.delta", "text": piece}));
                    let interrupt = pause!(pace.next(16, 34));
                    if interrupt.is_some() {
                        event!(json!({"type": "message.completed", "content": said}));
                        produced.push(Item::Assistant(said.clone()));
                        said.clear();
                    }
                    let stop = matches!(interrupt, Some(Interrupt::Cancelled));
                    let steered = matches!(interrupt, Some(Interrupt::Steered(_)));
                    interrupted!(interrupt);
                    if stop || steered {
                        continue 'steps;
                    }
                }
                event!(json!({"type": "message.completed", "content": said}));
                produced.push(Item::Assistant(said));
            }
            Step::Tool {
                tool,
                title,
                arguments,
                output,
                millis,
                failed,
            } => {
                calls += 1;
                let call = format!("call-{calls}");
                event!(
                    json!({"type": "tool.started", "call_id": call, "tool": tool, "title": title, "arguments": arguments})
                );
                let begun = std::time::Instant::now();
                let lines: Vec<&str> = output.lines().collect();
                let each = (millis / (lines.len() as u64 + 1)).max(20);
                let mut cancelled = false;
                for line in &lines {
                    let interrupt = pause!(each);
                    if matches!(interrupt, Some(Interrupt::Cancelled)) {
                        cancelled = true;
                    }
                    interrupted!(interrupt);
                    event!(
                        json!({"type": "tool.output.delta", "call_id": call, "text": format!("{line}\n")})
                    );
                }
                let status_text = if cancelled {
                    "cancelled"
                } else if failed {
                    "failed"
                } else {
                    "completed"
                };
                let millis = begun.elapsed().as_millis() as u64;
                event!(
                    json!({"type": "tool.completed", "call_id": call, "status": status_text, "output": output, "duration_ms": millis})
                );
                produced.push(Item::Tool {
                    tool: tool.into(),
                    title,
                    arguments,
                    output,
                    status: status_text.into(),
                    millis,
                });
            }
            Step::Ask {
                tool,
                title,
                arguments,
                output,
            } => {
                calls += 1;
                let request = format!("approval-{calls}");
                let (answer_tx, answer) = oneshot::channel();
                if let Ok(mut guard) = state.lock()
                    && let Some(live) = guard.live.get_mut(&session)
                {
                    live.approvals.insert(request.clone(), answer_tx);
                }
                event!(
                    json!({"type": "approval.requested", "request_id": request, "binding": format!("{session}:{request}"), "tool": tool, "title": title, "arguments": arguments})
                );
                let allow = tokio::select! {
                    answer = answer => answer.unwrap_or(false),
                    _ = cancel.changed() => {
                        status = "cancelled";
                        event!(json!({"type": "approval.resolved", "request_id": request, "allow": false}));
                        break 'steps;
                    }
                };
                event!(json!({"type": "approval.resolved", "request_id": request, "allow": allow}));
                if allow {
                    steps.insert(index, Step::Say("Done — the change is written. I kept a backup next to it as `notes.md.bak`, so you can undo it from Files if needed.".into()));
                    steps.insert(
                        index,
                        Step::Tool {
                            tool,
                            title,
                            arguments,
                            output,
                            millis: 900,
                            failed: false,
                        },
                    );
                } else {
                    steps.insert(index, Step::Say("Okay, I left the file untouched. If you'd rather review the text first, I can paste the proposed contents here instead.".into()));
                }
            }
            Step::Fail(error) => {
                event!(json!({"type": "error", "message": error}));
                produced.push(Item::Note(error.to_owned()));
                status = "failed";
                break;
            }
        }
    }
    let duration = started.elapsed().as_millis() as u64;
    if let Ok(mut guard) = state.lock() {
        guard.live.remove(&session);
        if let Some(known) = guard.sessions.iter_mut().find(|known| known.id == session) {
            if status == "cancelled" {
                produced.push(Item::Note("Stopped".into()));
            }
            known.items.extend(produced);
            known.updated = now();
        }
    }
    let usage =
        json!({"input_tokens": prompt.len() / 4 + 1200, "output_tokens": output_tokens * 2});
    let _ = send(write, &json!({"event": {"type": "turn.completed", "status": status, "duration_ms": duration, "usage": usage}})).await;
    send(write, &json!({"ok": status != "failed", "status": status, "error": if status == "failed" { "the model provider is unavailable" } else { "" }})).await
}

fn title_for(prompt: &str) -> String {
    let words: Vec<&str> = prompt.split_whitespace().take(6).collect();
    let mut title = words.join(" ");
    if title.chars().count() > 42 {
        title = title.chars().take(41).collect::<String>() + "…";
    }
    let mut chars = title.chars();
    chars.next().map_or_else(
        || "New chat".into(),
        |first| first.to_uppercase().collect::<String>() + chars.as_str(),
    )
}

fn steered(text: &str) -> Vec<Step> {
    vec![Step::Say(format!(
        "Got it — adjusting for that: *{}*.\n\nI'll keep the rest of the answer short and focus on that point.",
        text.chars().take(120).collect::<String>()
    ))]
}

fn has(prompt: &str, words: &[&str]) -> bool {
    let prompt = prompt.to_lowercase();
    words.iter().any(|word| prompt.contains(word))
}

fn script(prompt: &str, images: &Path) -> Vec<Step> {
    let chart = images.join("throughput.png");
    if has(prompt, &["error", "fail", "crash"]) {
        return vec![
            Step::Think("The user wants to see a failure. I'll start answering and then report the provider error the way a real outage would."),
            Step::Say("Let me check that for you. Looking at the recent logs, the first thing that stands out".into()),
            Step::Fail("The model provider stopped responding (HTTP 503). Your message was kept — try again in a moment."),
        ];
    }
    if has(prompt, &["approval", "approve", "write", "delete", "save"]) {
        return vec![
            Step::Think("Writing a file needs the user's approval. I'll read the current file, then ask before changing it."),
            Step::Tool { tool: "read_file", title: "Read notes.md".into(), arguments: json!({"path": "~/Documents/notes.md"}), output: "# Notes\n- Groceries: oat milk, coffee\n- Call the landlord about the heating\n".into(), millis: 500, failed: false },
            Step::Say("Your notes have two items. I'll add today's meeting summary at the end.".into()),
            Step::Ask { tool: "write_file", title: "Write notes.md".into(), arguments: json!({"path": "~/Documents/notes.md", "append": "\n## Meeting, Friday\n- Ship the AI panel preview\n- Review tiling feedback\n"}), output: "Wrote 3 lines (96 bytes) to ~/Documents/notes.md".into() },
        ];
    }
    if has(
        prompt,
        &["tool", "file", "search", "find", "disk", "wifi", "network"],
    ) {
        return vec![
            Step::Think("I should look at the actual system state before answering: disk usage first, then the largest folders."),
            Step::Tool { tool: "run_command", title: "df -h /home".into(), arguments: json!({"command": "df -h /home"}), output: "Filesystem      Size  Used Avail Use% Mounted on\n/dev/example    476G  401G   75G  85% /home".into(), millis: 700, failed: false },
            Step::Tool { tool: "search_files", title: "Largest folders in ~".into(), arguments: json!({"root": "~", "sort": "size", "limit": 4}), output: "38.2 GB  ~/.cache\n21.7 GB  ~/Videos/Screencasts\n12.9 GB  ~/Projects/example-app/target\n6.4 GB   ~/Downloads".into(), millis: 1400, failed: false },
            Step::Tool { tool: "read_file", title: "Read ~/.cache/README".into(), arguments: json!({"path": "~/.cache/README"}), output: "No such file".into(), millis: 200, failed: true },
            Step::Say("Your home partition is **85% full** (75 GB free). Most of it is reclaimable:\n\n1. `~/.cache` — **38 GB**, safe to clear; apps rebuild what they need.\n2. `~/Projects/example-app/target` — **13 GB** of build output; `cargo clean` frees it.\n3. `~/Videos/Screencasts` — 22 GB you may want to keep or move.\n\nWant me to clear the cache? I'll ask before deleting anything.".into()),
        ];
    }
    if has(
        prompt,
        &["image", "picture", "chart", "graph", "screenshot"],
    ) {
        return vec![
            Step::Think(
                "A chart answers this better than numbers. I'll render the last week's throughput and explain it.",
            ),
            Step::Tool {
                tool: "render_chart",
                title: "Render throughput chart".into(),
                arguments: json!({"series": "requests/s", "range": "7d"}),
                output: format!("Saved {}", chart.display()),
                millis: 1100,
                failed: false,
            },
            Step::Say(format!(
                "Here is the last seven days of throughput:\n\n![Requests per second, last 7 days]({})\n\nThursday's peak is the release; the dip on Saturday is the planned maintenance window. Everything else stays within the usual band.",
                chart.display()
            )),
        ];
    }
    if has(prompt, &["link", "url", "docs", "reference"]) {
        return vec![Step::Say("A few good references:\n\n- [iced — a cross-platform GUI library for Rust](https://iced.rs)\n- [Hyprland wiki](https://wiki.hypr.land) — window rules, dispatchers and Lua config\n- [The Rust Book](https://doc.rust-lang.org/book/), chapter 10 for lifetimes\n- [CommonMark spec](https://spec.commonmark.org/0.31.2/)\n\nInline links work too: see <https://example.org> or [this page](https://example.org/guide#setup).".into())];
    }
    if has(prompt, &["code", "rust", "function", "snippet"]) {
        return vec![
            Step::Think("A short, compilable example is the clearest answer. Keep the explanation brief."),
            Step::Say("Use a `HashMap` entry to count words in one pass:\n\n```rust\nuse std::collections::HashMap;\n\nfn word_counts(text: &str) -> HashMap<&str, usize> {\n    let mut counts = HashMap::new();\n    for word in text.split_whitespace() {\n        *counts.entry(word).or_insert(0) += 1;\n    }\n    counts\n}\n```\n\n`entry` looks the key up once, and `or_insert(0)` creates it on first sight — no second lookup, no `contains_key`.".into()),
        ];
    }
    if has(prompt, &["table", "compare"]) {
        return vec![Step::Say("| Mode | Bar | Panels | Best for |\n|:-----|:---:|:-------|:---------|\n| Frame | joined to the frame | grow out of the frame | focused work |\n| Islands | floating pills | float below the bar | light, airy desktops |\n| Tiling | either | either | many windows at once |\n\nBoth styles share the same panels; only how they attach changes.".into())];
    }
    if has(prompt, &["long", "essay", "explain everything"]) {
        let mut text = String::from("## A longer answer\n\n");
        for part in 1..=6 {
            let _ = write!(
                text,
                "### Part {part}\n\nThis section is deliberately long so the transcript has something to scroll. Each paragraph wraps across several lines in the panel, which exercises measured row heights in the virtual list, the tail follow while text streams in, and the jump-to-latest button once you scroll away from the bottom.\n\n- point {part}.1 — keep reading position stable\n- point {part}.2 — follow the newest text only when at the bottom\n\n"
            );
        }
        return vec![
            Step::Think(
                "The user wants a long answer; structure it with headings so it stays scannable.",
            ),
            Step::Say(text),
        ];
    }
    if has(
        prompt,
        &["markdown", "format", "demo", "everything", "show"],
    ) {
        return vec![
            Step::Think(
                "Show every formatting element once, compactly, so the rendering can be checked at a glance.",
            ),
            Step::Say(format!(
                "## Formatting check\n\nPlain text with **bold**, *italic*, `inline code` and a [link](https://iced.rs).\n\n- A bullet list\n- with a second item\n\n1. Ordered\n2. List\n\n> A quote, for when the assistant cites something.\n\n```bash\ncargo build --release -j1\n```\n\n| Column | Value |\n|---|---:|\n| Rows | 42 |\n| Time | 1.3 s |\n\n![Throughput chart]({})\n\nThat's everything the panel renders.",
                chart.display()
            )),
        ];
    }
    vec![Step::Say(match prompt.to_lowercase().trim_end_matches(['!', '.', '?']) {
        "hi" | "hello" | "hey" => "Hi! I can answer questions, look things up on this computer, and change settings or files when you approve it. What would you like to do?".into(),
        _ => "Sure. Here's the short version: the change you're describing is safe to make now, and it only affects this desktop. If you want, I can walk you through it step by step, or just do it and show you what changed.".into(),
    })]
}

fn seed(state: &Shared) {
    let Ok(mut state) = state.lock() else { return };
    let chart = state.images.join("throughput.png");
    let hour = 3600;
    let base = now();
    let mut long = Vec::new();
    for turn in 1..=40 {
        long.push(Item::User(format!(
            "Step {turn}: what's next in the migration?"
        )));
        long.push(Item::Assistant(format!(
            "**Step {turn}.** Move the next service behind the new gateway, then watch its error rate for ten minutes before continuing. {}",
            if turn % 3 == 0 { "\n\n- check the dashboards\n- keep the old route for rollback" } else { "" }
        )));
    }
    let sessions = vec![
        Session {
            id: "mock-seed-4".into(),
            title: "What can you do on this desktop?".into(),
            updated: base - 600,
            items: vec![
                Item::User("What can you do on this desktop?".into()),
                Item::Reasoning { text: "Give a compact overview with one concrete example of each ability.".into(), millis: 1800 },
                Item::Assistant("I can help in three ways:\n\n- **Answer** questions, with formatting, code and links.\n- **Look things up** on this computer — files, settings, disk space.\n- **Change things** you approve first, like writing a file or toggling a setting.\n\nTry *“show me a chart”*, *“check my disk space”* or *“save a note”*.".into()),
                Item::User("Check my disk space".into()),
                Item::Tool { tool: "run_command".into(), title: "df -h /home".into(), arguments: json!({"command": "df -h /home"}), output: "Filesystem      Size  Used Avail Use% Mounted on\n/dev/example    476G  401G   75G  85% /home".into(), status: "completed".into(), millis: 640 },
                Item::Assistant(format!("Your home partition is **85% full** — 75 GB free. Here's how it filled up this week:\n\n![Disk use over the week]({})", chart.display())),
            ],
        },
        Session {
            id: "mock-seed-3".into(),
            title: "Rust: borrowed vs owned strings".into(),
            updated: base - 5 * hour,
            items: vec![
                Item::User("When should a function take &str instead of String?".into()),
                Item::Assistant("Take `&str` when you only **read** the text, `String` when you **keep** it:\n\n```rust\nfn greet(name: &str) -> String { format!(\"Hello, {name}\") }\nstruct User { name: String }\n```\n\n`&str` accepts both `&String` and literals, so it's the friendlier parameter.".into()),
            ],
        },
        Session { id: "mock-seed-2".into(), title: "Gateway migration plan".into(), updated: base - 30 * hour, items: long },
        Session {
            id: "mock-seed-1".into(),
            title: "Wi-Fi keeps dropping".into(),
            updated: base - 74 * hour,
            items: vec![
                Item::User("My Wi-Fi keeps dropping every few minutes".into()),
                Item::Tool { tool: "run_command".into(), title: "journalctl -u NetworkManager".into(), arguments: json!({"command": "journalctl -u NetworkManager --since -1h"}), output: "wlan0: disconnected (reason 4: inactivity)\nwlan0: power save enabled".into(), status: "completed".into(), millis: 900 },
                Item::Assistant("The adapter drops when **power saving** kicks in. Turn it off in Settings › Network › Wi-Fi › Power saving, or run `nmcli connection modify <name> wifi.powersave 2`.".into()),
            ],
        },
    ];
    state.sessions = sessions;
}

fn write_chart(path: &Path) -> Result<(), ()> {
    let (width, height) = (640u32, 300u32);
    let values = [0.42f32, 0.55, 0.48, 0.92, 0.61, 0.18, 0.5];
    let mut image = raster::RgbaImage::new(width, height);
    // Transparent, with mid-grey guides and flat bars: it sits on the panel in
    // light and dark themes alike (a baked dark plate stayed dark in light mode).
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let mut color = [0, 0, 0, 0];
        if (y + 1) % (height / 4) == 0 {
            color = [128, 128, 140, 70];
        }
        let slot = width / values.len() as u32;
        let index = (x / slot) as usize;
        let within = x % slot;
        if index < values.len() && within > slot / 5 && within < slot - slot / 5 {
            let top = height as f32 * (1.0 - values[index] * 0.86) - 12.0;
            if (y as f32) > top && y < height - 12 {
                color = if index == 3 {
                    [245, 146, 52, 255]
                } else {
                    [64, 132, 240, 255]
                };
            }
        }
        *pixel = raster::Rgba(color);
    }
    image.save(path).map_err(drop)
}
