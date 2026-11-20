//! Authenticated control client for MCP Center consent and permissions.
//!
//! The broker owns authorization; this client relays decisions using its private control
//! credential.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

const PROTOCOL_VERSION: u32 = 2;
const MAX_FRAME: usize = 1_048_576;
const DOMAIN: &[u8] = b"enzui-center-v2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub request_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub app_id: String,
    pub app_name: String,
    pub function_id: String,
    pub function_title: String,
    pub function_description: String,
    pub risk: String,
    /// The scopes the broker allows for this request (`once`, `session`, `always`).
    pub allowed: Vec<String>,
    pub created_unix_ms: u64,
    /// The call's arguments as pretty JSON (bounded by the broker), shown so
    /// the person sees what they allow; `None` when the call has none.
    pub arguments: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Once,
    Session,
    Always,
    Deny,
    Never,
}

impl Decision {
    fn key(self) -> &'static str {
        match self {
            Decision::Once => "once",
            Decision::Session => "session",
            Decision::Always => "always",
            Decision::Deny => "deny",
            Decision::Never => "never",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub revoked: bool,
    pub created_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permission {
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub app_id: String,
    pub app_name: Option<String>,
    pub function_id: String,
    pub allowed: bool,
    pub updated_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub id: u64,
    /// The broker's UUID for one call attempt; its events share it.
    pub request_id: Option<String>,
    pub created_unix_ms: u64,
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub app_id: String,
    pub app_name: Option<String>,
    pub function_id: String,
    pub outcome: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlCredential {
    version: u32,
    kind: String,
    id: String,
    #[serde(rename = "display_name")]
    _display_name: String,
    secret: String,
}

fn base(env: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(env)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|dir| dir.join("enzui/center.sock"))
}

fn credential_path() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state").map(|dir| dir.join("enzui/control.credential.json"))
}

/// The error of a decision about a request the broker no longer holds (it
/// was answered elsewhere, timed out, or its agent session ended).
pub const GONE: &str = "The request is no longer waiting";

/// Whether this account has a broker to talk to at all.
pub fn available() -> bool {
    socket_path().is_some_and(|p| p.exists()) && credential_path().is_some_and(|p| p.is_file())
}

fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner: Vec<u8> = block.iter().map(|b| b ^ 0x36).collect();
    let outer: Vec<u8> = block.iter().map(|b| b ^ 0x5c).collect();
    let inner = Sha256::new()
        .chain_update(&inner)
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(&outer)
        .chain_update(inner)
        .finalize()
        .into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn lp(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

// Proof transcript follows enzui-mcp-center docs/IPC_PROTOCOL.md.
fn proof(
    secret: &[u8],
    direction: &str,
    nonce: &str,
    client_nonce: &str,
    role: &str,
    id: &str,
    uid: u32,
) -> String {
    let mut message = Vec::new();
    message.extend_from_slice(DOMAIN);
    message.push(0);
    message.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    lp(&mut message, direction);
    lp(&mut message, nonce);
    lp(&mut message, client_nonce);
    lp(&mut message, role);
    lp(&mut message, id);
    message.extend_from_slice(&uid.to_be_bytes());
    hex(&hmac(secret, &message))
}

fn failure(_: impl std::fmt::Display) -> String {
    tr("shell-the-mcp-center-is-not-reachable").into()
}

fn read_frame(stream: &mut UnixStream) -> Result<Value, String> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).map_err(failure)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(failure("frame"));
    }
    let mut data = vec![0u8; length];
    stream.read_exact(&mut data).map_err(failure)?;
    serde_json::from_slice(&data).map_err(failure)
}

fn write_frame(stream: &mut UnixStream, value: &Value) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(failure)?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err(failure("frame"));
    }
    stream
        .write_all(&(data.len() as u32).to_be_bytes())
        .map_err(failure)?;
    stream.write_all(&data).map_err(failure)
}

fn read_credential(path: &Path) -> Result<(String, Vec<u8>), String> {
    let meta = std::fs::symlink_metadata(path).map_err(failure)?;
    let uid = std::fs::metadata("/proc/self").map_err(failure)?.uid();
    if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o077 != 0 || meta.len() > 4096 {
        return Err(tr("shell-the-mcp-center-control-credential-is-not-private").into());
    }
    let credential: ControlCredential =
        serde_json::from_slice(&std::fs::read(path).map_err(failure)?).map_err(failure)?;
    let secret = unhex(&credential.secret).filter(|s| s.len() == 32);
    match (credential.version, credential.kind.as_str(), secret) {
        (1, "control", Some(secret)) => Ok((credential.id, secret)),
        _ => Err(tr("shell-the-mcp-center-control-credential-is-invalid").into()),
    }
}

fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(failure)?;
    Ok(hex(&bytes))
}

fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 64 && unhex(nonce).is_some()
}

fn request(body: Value) -> Result<Value, String> {
    request_with(body, Duration::from_secs(5))
}

// The answer timeout begins after sending the request.
fn request_with(body: Value, answer: Duration) -> Result<Value, String> {
    let socket = socket_path().ok_or_else(|| failure("runtime"))?;
    let (id, secret) = read_credential(&credential_path().ok_or_else(|| failure("state"))?)?;
    let uid = std::fs::metadata("/proc/self").map_err(failure)?.uid();
    let meta = std::fs::symlink_metadata(&socket).map_err(failure)?;
    if meta.uid() != uid {
        return Err(failure(tr("shell-socket-owner")));
    }
    let mut stream = UnixStream::connect(&socket).map_err(failure)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(failure)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(failure)?;
    let peer = rustix::net::sockopt::socket_peercred(&stream).map_err(failure)?;
    if peer.uid.as_raw() != uid {
        return Err(failure("peer"));
    }
    let challenge = read_frame(&mut stream)?;
    let nonce = challenge["nonce"]
        .as_str()
        .filter(|n| valid_nonce(n))
        .ok_or_else(|| failure("challenge"))?
        .to_owned();
    if challenge["version"].as_u64() != Some(u64::from(PROTOCOL_VERSION)) {
        return Err(tr("shell-the-mcp-center-speaks-another-protocol-version").into());
    }
    let client_nonce = random_hex()?;
    write_frame(
        &mut stream,
        &json!({
            "version": PROTOCOL_VERSION,
            "role": "control",
            "credential_id": id,
            "client_nonce": client_nonce,
            "proof": proof(&secret, "client", &nonce, &client_nonce, "control", &id, uid),
        }),
    )?;
    let reply = read_frame(&mut stream)?;
    // The broker proves it holds the same secret before anything is sent.
    if reply["status"].as_str() != Some("accepted")
        || reply["proof"].as_str()
            != Some(
                proof(
                    &secret,
                    "broker",
                    &nonce,
                    &client_nonce,
                    "control",
                    &id,
                    uid,
                )
                .as_str(),
            )
    {
        return Err(tr("shell-the-mcp-center-refused-this-desktop-s-control-credential").into());
    }
    write_frame(&mut stream, &body)?;
    stream.set_read_timeout(Some(answer)).map_err(failure)?;
    let response = read_frame(&mut stream)?;
    if let Some(ok) = response["result"].get("Ok") {
        return Ok(ok.clone());
    }
    if response["result"]["Err"]["code"] == "request_not_found" {
        return Err(GONE.into());
    }
    let message = response["result"]["Err"]["message"]
        .as_str()
        .unwrap_or_else(|| tr("shell-request-refused"));
    Err(tr!("shell-the-mcp-center-refused-the-request", "arg0" => message.to_owned()))
}

fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect()
}

fn optional(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(|_| text(value, key))
}

pub fn pending() -> Result<Vec<Pending>, String> {
    Ok(pending_from(&request(
        json!({"operation": "list_pending"}),
    )?))
}

/// The pending requests once they differ from `since` (a version an earlier
/// call returned), or the unchanged list after about 25 s.
pub fn wait_pending(since: Option<u64>) -> Result<(Vec<Pending>, u64), String> {
    let payload = request_with(
        json!({"operation": "wait_pending", "since": since, "timeout_ms": 25_000}),
        Duration::from_secs(35),
    )?;
    let version = payload["version"]
        .as_u64()
        .ok_or_else(|| failure("version"))?;
    Ok((pending_from(&payload), version))
}

fn pending_from(payload: &Value) -> Vec<Pending> {
    payload["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .take(128)
        .map(|r| Pending {
            request_id: text(r, "request_id"),
            agent_id: text(r, "agent_id"),
            agent_name: text(r, "agent_name"),
            app_id: text(r, "app_id"),
            app_name: text(r, "app_name"),
            function_id: text(r, "function_id"),
            function_title: text(r, "function_title"),
            function_description: text(r, "function_description"),
            risk: text(r, "risk"),
            allowed: r["allowed_consent"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
            created_unix_ms: r["created_unix_ms"].as_u64().unwrap_or(0),
            arguments: r["arguments_preview"].as_str().map(shown_arguments),
        })
        .collect()
}

/// Agent-supplied text for display: line breaks kept, other control and
/// bidirectional-override characters dropped (they could disguise a value),
/// bounded.
fn shown_arguments(raw: &str) -> String {
    raw.chars()
        .filter(|c| {
            *c == '\n'
                || !(c.is_control()
                    || ('\u{202A}'..='\u{202E}').contains(c)
                    || ('\u{2066}'..='\u{2069}').contains(c)
                    || *c == '\u{200E}'
                    || *c == '\u{200F}')
        })
        .take(4096)
        .collect()
}

pub fn decide(request_id: &str, decision: Decision) -> Result<(), String> {
    request(json!({"operation": "decide", "request_id": request_id, "decision": decision.key()}))
        .map(drop)
}

pub fn credentials() -> Result<Vec<Credential>, String> {
    let payload = request(json!({"operation": "list_credentials"}))?;
    Ok(payload["credentials"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Credential {
            id: text(c, "id"),
            display_name: text(c, "display_name"),
            kind: text(c, "kind"),
            revoked: c["revoked"].as_bool().unwrap_or(false),
            created_unix_ms: c["created_unix_ms"].as_u64().unwrap_or(0),
        })
        .collect())
}

pub fn revoke_credential(id: &str) -> Result<(), String> {
    request(json!({"operation": "revoke_credential", "credential_id": id})).map(drop)
}

pub fn permissions() -> Result<Vec<Permission>, String> {
    let payload = request(json!({"operation": "list_permissions"}))?;
    Ok(payload["permissions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Permission {
            agent_id: text(p, "agent_id"),
            agent_name: optional(p, "agent_name"),
            app_id: text(p, "app_id"),
            app_name: optional(p, "app_name"),
            function_id: text(p, "function_id"),
            allowed: p["allowed"].as_bool().unwrap_or(false),
            updated_unix_ms: p["updated_unix_ms"].as_u64().unwrap_or(0),
        })
        .collect())
}

pub fn revoke_permission(agent_id: &str, app_id: &str, function_id: &str) -> Result<(), String> {
    request(json!({"operation": "revoke_permission", "agent_id": agent_id, "app_id": app_id, "function_id": function_id})).map(drop)
}

pub fn activity(limit: u16) -> Result<Vec<Activity>, String> {
    let payload = request(
        json!({"operation": "list_audit", "cursor": null, "limit": limit.clamp(1, 100),
        "agent_id": null, "app_id": null, "function_id": null, "outcome": null}),
    )?;
    Ok(payload["records"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| Activity {
            id: a["id"].as_u64().unwrap_or(0),
            request_id: optional(a, "request_id").filter(|id| id.len() <= 64),
            created_unix_ms: a["created_unix_ms"].as_u64().unwrap_or(0),
            agent_id: text(a, "agent_id"),
            agent_name: optional(a, "agent_name"),
            app_id: text(a, "app_id"),
            app_name: optional(a, "app_name"),
            function_id: text(a, "function_id"),
            outcome: text(a, "outcome"),
        })
        .collect())
}

/// One row per call: the newest event of each request (the list is newest
/// first). Events without a request ID are older records and stay as they are.
#[must_use]
pub fn latest_per_request(events: &[Activity]) -> Vec<&Activity> {
    let mut seen = std::collections::HashSet::new();
    events
        .iter()
        .filter(|event| {
            event
                .request_id
                .as_ref()
                .is_none_or(|id| seen.insert(id.clone()))
        })
        .collect()
}

pub fn clear_activity() -> Result<(), String> {
    request(json!({"operation": "clear_audit", "before_unix_ms": null})).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_center_behaviour() {
        // A call shows its newest event and distinct calls stay apart.
        {
            let event = |id: u64, request: Option<&str>, outcome: &str| Activity {
                id,
                request_id: request.map(str::to_owned),
                created_unix_ms: id,
                agent_id: "a".into(),
                agent_name: None,
                app_id: "app".into(),
                app_name: None,
                function_id: "get".into(),
                outcome: outcome.into(),
            };
            let events = [
                event(5, Some("r2"), "completed"),
                event(4, Some("r1"), "completed"),
                event(3, Some("r2"), "dispatched"),
                event(2, Some("r1"), "dispatched"),
                event(1, None, "dispatched"),
            ];
            let shown: Vec<(u64, &str)> = latest_per_request(&events)
                .iter()
                .map(|e| (e.id, e.outcome.as_str()))
                .collect();
            assert_eq!(
                shown,
                [(5, "completed"), (4, "completed"), (1, "dispatched")]
            );
        }
        // Hmac matches rfc 4231.
        {
            // RFC 4231 test case 2.
            assert_eq!(
                hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
            );
        }
        // Proofs match the protocols golden vector.
        {
            // docs/IPC_PROTOCOL.md in enzui-mcp-center.
            let secret = [7u8; 32];
            let (nonce, client) = ("00".repeat(32), "11".repeat(32));
            assert_eq!(
                proof(
                    &secret,
                    "client",
                    &nonce,
                    &client,
                    "app",
                    "org.enzui.Example",
                    1000
                ),
                "c651a6fe86746ef8a6c5cfc6e00be3e8f73956bb6cb05142884f1126eef0733f"
            );
            assert_eq!(
                proof(
                    &secret,
                    "broker",
                    &nonce,
                    &client,
                    "app",
                    "org.enzui.Example",
                    1000
                ),
                "4f348f83c29d613455301ca64541e96c8783a1d2d55b792ff8f2890473f07511"
            );
        }
        // The prompt shows the broker's argument preview with its line breaks,
        // but without characters that could disguise a value.
        //
        // Sabotage: map `arguments_preview` through `text` (which drops newlines).
        {
            let payload = json!({"requests": [{
                "request_id": "r", "agent_id": "a", "agent_name": "Agent", "app_id": "app",
                "app_name": "App", "function_id": "set", "function_title": "Set",
                "function_description": "", "risk": "mutating", "allowed_consent": ["once"],
                "created_unix_ms": 1,
                "arguments_preview": "{\n  \"key\": \"dark\u{202E}evil\",\n  \"value\": true\n}"
            }, {
                "request_id": "s", "agent_id": "a", "agent_name": "Agent", "app_id": "app",
                "app_name": "App", "function_id": "list", "function_title": "List",
                "function_description": "", "risk": "read_only", "allowed_consent": ["once"],
                "created_unix_ms": 2
            }]});
            let pending = pending_from(&payload);
            assert_eq!(
                pending[0].arguments.as_deref(),
                Some("{\n  \"key\": \"darkevil\",\n  \"value\": true\n}")
            );
            assert_eq!(pending[1].arguments, None);
        }
    }
}
