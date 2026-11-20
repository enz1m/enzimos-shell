//! Calendar events through MCP Center's read-only list_events call and per-agent consent.
//! The adapter session persists for the shell's lifetime.

use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const AGENT_ID: &str = "dev.enzui.Desktop";
const CALENDAR_ID: &str = "io.github.enzui.Calendar";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub title: String,
    pub calendar: String,
    pub all_day: bool,
    pub start_minute: u16,
    pub end_minute: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Unavailable,
    /// Waiting for the person to answer the Center's consent prompt.
    Permission,
    /// The person refused ("Never allow"): asking again only adds prompts.
    Denied,
    Closed,
    Other(String),
}

pub struct Client {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    next: u64,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

fn credential() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(
        config
            .join("enzui/agents")
            .join(format!("{AGENT_ID}.credential.json")),
    )
    .filter(|path| path.is_file())
}

fn adapter() -> Option<PathBuf> {
    ["/usr/bin/enzui-mcp", "/usr/local/bin/enzui-mcp"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

impl Client {
    pub async fn connect() -> Result<Self, Failure> {
        let (Some(adapter), Some(credential)) = (adapter(), credential()) else {
            return Err(Failure::Unavailable);
        };
        let mut child = tokio::process::Command::new(adapter)
            .arg("--credential")
            .arg(credential)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Failure::Unavailable)?;
        let input = child.stdin.take().ok_or(Failure::Unavailable)?;
        let output = BufReader::new(child.stdout.take().ok_or(Failure::Unavailable)?).lines();
        let mut client = Self {
            child,
            input,
            output,
            next: 1,
        };
        client
            .request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "enzui-shell", "version": "1"}}))
            .await?;
        client
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        Ok(client)
    }

    async fn send(&mut self, message: &Value) -> Result<(), Failure> {
        let mut line = message.to_string();
        line.push('\n');
        self.input
            .write_all(line.as_bytes())
            .await
            .map_err(|_| Failure::Unavailable)?;
        self.input.flush().await.map_err(|_| Failure::Unavailable)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        let reply = tokio::time::timeout(Duration::from_secs(10), async {
            while let Ok(Some(line)) = self.output.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message.get("id").and_then(Value::as_u64) == Some(id) {
                    return Some(message);
                }
            }
            None
        })
        .await
        .map_err(|_| Failure::Other(tr("shell-the-mcp-center-did-not-answer").into()))?
        .ok_or(Failure::Unavailable)?;
        if let Some(error) = reply.get("error") {
            return Err(Failure::Other(
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| tr("shell-mcp-error"))
                    .to_owned(),
            ));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    /// The events of `date` (YYYY-MM-DD).
    pub async fn events(&mut self, date: &str) -> Result<Vec<Event>, Failure> {
        let result = self
            .request(
                "tools/call",
                json!({"name": "enzui_call", "arguments": {"app_id": CALENDAR_ID, "function_id": "list_events", "arguments": {"from": date, "to": date}}}),
            )
            .await?;
        parse(&result)
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Reads an `enzui_call` tool result.
pub fn parse(result: &Value) -> Result<Vec<Event>, Failure> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<String>();
        return Err(if text.contains("permission_denied") {
            Failure::Denied
        } else if text.contains("permission_required") {
            Failure::Permission
        } else if text.contains("app_unavailable")
            || text.contains("not running")
            || text.contains("unknown_app")
        {
            Failure::Closed
        } else {
            Failure::Other(tr("shell-calendar-could-not-be-read").into())
        });
    }
    let events = result
        .pointer("/structuredContent/result/events")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::Other(tr("shell-unexpected-calendar-reply").into()))?;
    Ok(events
        .iter()
        .take(50)
        .filter_map(|event| {
            let minute = |key: &str| {
                event
                    .get(key)
                    .and_then(Value::as_u64)
                    .map(|m| m.min(24 * 60) as u16)
            };
            let text = |key: &str| {
                event.get(key).and_then(Value::as_str).map(|t| {
                    t.chars()
                        .filter(|c| !c.is_control())
                        .take(80)
                        .collect::<String>()
                })
            };
            Some(Event {
                title: text("title")?,
                calendar: text("calendar").unwrap_or_default(),
                all_day: event
                    .get("all_day")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                start_minute: minute("start_minute")?,
                end_minute: minute("end_minute")?,
            })
        })
        .collect())
}
