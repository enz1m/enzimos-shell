//! Background activity metadata from enzimos-activityd.
//!
//! Only a job's originating window hides its work; window tags are scoped to the publisher PID.
//! Without a tag, any window of that PID counts as the origin. Metadata grants no launch
//! authority.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::Deserialize;
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u32 = 1;
// The service's own bound on jobs.
const MAX_JOBS: usize = 64;
const MAX_LINE: usize = 256 * 1024;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Bytes,
    Items,
    Steps,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub unit: Unit,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// Producer choice for the transient live activity surface.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Suppress work while its originating window is active.
    #[default]
    WhenUnfocused,
    /// Show work even while its originating window is active.
    Always,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Work {
    #[serde(default)]
    pub visibility: Visibility,
    pub job: String,
    pub label: String,
    #[serde(default)]
    pub detail: String,
    pub state: State,
    #[serde(default)]
    pub progress: Option<Progress>,
    #[serde(default)]
    pub eta_ms: Option<u64>,
    #[serde(default)]
    pub window_token: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    pub app_id: String,
    pub peer_pid: u32,
    pub work: Work,
    pub outcome: Option<Outcome>,
    pub age_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Frame {
    op: String,
    version: u32,
    generation: String,
    revision: u64,
    jobs: Vec<Job>,
}

/// The service's latest state, as received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: String,
    pub revision: u64,
    pub jobs: Vec<Job>,
    /// When it arrived; ages and estimates run from here.
    pub received: Instant,
}

fn plain(text: &str, max: usize) -> bool {
    text.len() <= max
        && !text.chars().any(|c| {
            c.is_control() || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

fn token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b))
}

/// One snapshot line, checked against the v1 bounds. Anything else ends the
/// connection, as the protocol asks of a subscriber.
pub fn parse(line: &[u8]) -> Result<Snapshot, &'static str> {
    let frame: Frame = serde_json::from_slice(line).map_err(|_| tr("shell-not-a-snapshot"))?;
    if frame.op != "snapshot" || frame.version != PROTOCOL_VERSION {
        return Err(tr("shell-not-a-version-1-snapshot"));
    }
    if frame.jobs.len() > MAX_JOBS || !plain(&frame.generation, 128) {
        return Err(tr("shell-snapshot-out-of-bounds"));
    }
    for job in &frame.jobs {
        let work = &job.work;
        let valid = plain(&job.id, 128)
            && !job.app_id.is_empty()
            && job.app_id.len() <= 128
            && job.app_id.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
            && !work.label.trim().is_empty()
            && plain(&work.label, 96)
            && plain(&work.detail, 160)
            && work.window_token.as_deref().is_none_or(token)
            && work
                .progress
                .is_none_or(|p| p.total > 0 && p.done <= p.total)
            && work.eta_ms.is_none_or(|eta| eta <= 7 * 24 * 3600 * 1000)
            && (work.state != State::Paused || work.eta_ms.is_none());
        if !valid {
            return Err(tr("shell-job-out-of-bounds"));
        }
    }
    Ok(Snapshot {
        generation: frame.generation,
        revision: frame.revision,
        jobs: frame.jobs,
        received: Instant::now(),
    })
}

/// What the compositor reports about windows, for suppression.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Windows {
    /// The active window's owner and tag.
    pub active: Option<(u32, Option<String>)>,
    /// Every mapped window's owner and tag.
    pub tagged: Vec<(u32, String)>,
}

impl Windows {
    /// Whether `job` belongs to the active window: its verified tag, or with
    /// no verified tag, its process.
    #[must_use]
    pub fn is_origin(&self, job: &Job) -> bool {
        let Some((pid, tag)) = &self.active else {
            return false;
        };
        let verified = job.work.window_token.as_ref().filter(|token| {
            self.tagged
                .iter()
                .any(|(owner, tag)| *owner == job.peer_pid && tag == *token)
        });
        match verified {
            Some(token) => *pid == job.peer_pid && tag.as_deref() == Some(token.as_str()),
            None => *pid == job.peer_pid,
        }
    }
}

/// The work the bar shows, in a stable order: unfinished first, then by app
/// and service id.
#[must_use]
pub fn visible<'a>(snapshot: &'a Snapshot, windows: &Windows) -> Vec<&'a Job> {
    let mut jobs: Vec<&Job> = snapshot
        .jobs
        .iter()
        .filter(|job| job.work.visibility == Visibility::Always || !windows.is_origin(job))
        .collect();
    jobs.sort_by(|a, b| {
        (a.outcome.is_some(), &a.app_id, &a.id).cmp(&(b.outcome.is_some(), &b.app_id, &b.id))
    });
    jobs
}

/// Returns an outcome, pause state or remaining estimate; elapsed estimates never imply
/// completion.
#[must_use]
pub fn status(job: &Job, received: Instant, now: Instant) -> Option<String> {
    if let Some(outcome) = job.outcome {
        return Some(
            match outcome {
                Outcome::Succeeded => tr("shell-done"),
                Outcome::Failed => tr("shell-failed"),
                Outcome::Cancelled => tr("shell-cancelled"),
            }
            .into(),
        );
    }
    if job.work.state == State::Paused {
        return Some(tr("shell-paused").into());
    }
    let eta = Duration::from_millis(job.work.eta_ms?);
    let elapsed = Duration::from_millis(job.age_ms) + now.saturating_duration_since(received);
    let left = eta.checked_sub(elapsed).filter(|left| !left.is_zero())?;
    let seconds = left.as_secs().max(1);
    Some(if seconds < 60 {
        tr!("shell-about-s-left", "arg0" => format!("{:}", seconds))
    } else if seconds < 3600 {
        tr!("shell-about-min-left", "arg0" => format!("{:}", seconds.div_ceil(60)))
    } else {
        tr!("shell-about-h-left", "arg0" => format!("{:}", seconds.div_ceil(3600)))
    })
}

/// Completed fraction, or `None` for work of unknown size.
#[must_use]
pub fn fraction(job: &Job) -> Option<f32> {
    job.work
        .progress
        .map(|p| (p.done as f64 / p.total as f64) as f32)
}

/// Whether the bar needs a clock to count an estimate down.
#[must_use]
pub fn counting(jobs: &[&Job]) -> bool {
    jobs.iter().any(|job| {
        job.outcome.is_none() && job.work.state == State::Running && job.work.eta_ms.is_some()
    })
}

#[derive(Clone, Debug)]
pub enum Event {
    Snapshot(Box<Snapshot>),
    /// Disconnected, or the service is absent: nothing is known.
    Cleared,
}

/// The service socket for this login, chosen as the framework's publisher
/// chooses it, so apps and the bar meet at one socket.
#[must_use]
pub fn socket() -> Option<std::path::PathBuf> {
    choose_socket(
        std::env::var_os("ENZUI_ACTIVITY_SOCKET"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::path::Path::exists,
    )
}

// Legacy activity sockets remain a fallback only when the neutral service socket is absent.
fn choose_socket(
    explicit: Option<std::ffi::OsString>,
    runtime: Option<std::ffi::OsString>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::path::PathBuf> {
    if let Some(socket) = explicit
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Some(socket);
    }
    let runtime = runtime
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())?;
    let neutral = runtime.join("enzui-activity/service.sock");
    let legacy = runtime.join("enzimos-activity/service.sock");
    Some(if !exists(&neutral) && exists(&legacy) {
        legacy
    } else {
        neutral
    })
}

/// Latest snapshots from the service, reconnecting with capped backoff.
pub fn subscription(socket: std::path::PathBuf) -> iced::Subscription<Event> {
    iced::Subscription::run_with(socket, stream)
}

// Subscription::run_with borrows its hashed data; the stream must not retain that borrow.
#[allow(
    clippy::ptr_arg,
    reason = "Subscription::run_with requires the concrete hashed data type."
)]
fn stream(socket: &std::path::PathBuf) -> impl iced::futures::Stream<Item = Event> + use<> {
    use iced::futures::SinkExt;
    let socket = socket.clone();
    iced::stream::channel(4, async move |mut output| {
        let mut backoff = Duration::from_millis(500);
        loop {
            let mut received = false;
            if let Ok(stream) = connect(&socket).await {
                let result = follow(stream, &mut output, &mut received).await;
                if result.is_err() && output.is_closed() {
                    return;
                }
            }
            if output.send(Event::Cleared).await.is_err() {
                return;
            }
            backoff = if received {
                Duration::from_millis(500)
            } else {
                (backoff * 2).min(MAX_BACKOFF)
            };
            tokio::time::sleep(backoff).await;
        }
    })
}

async fn connect(socket: &std::path::Path) -> std::io::Result<tokio::net::UnixStream> {
    use std::os::unix::fs::MetadataExt;
    use tokio::io::AsyncWriteExt;
    let mut stream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::UnixStream::connect(socket),
    )
    .await??;
    let uid = std::fs::metadata("/proc/self")?.uid();
    if stream.peer_cred()?.uid() != uid {
        return Err(std::io::Error::other(tr(
            "shell-the-activity-service-belongs-to-another-user",
        )));
    }
    stream
        .write_all(format!("{{\"op\":\"subscribe\",\"version\":{PROTOCOL_VERSION}}}\n").as_bytes())
        .await?;
    Ok(stream)
}

async fn follow(
    stream: tokio::net::UnixStream,
    output: &mut iced::futures::channel::mpsc::Sender<Event>,
    received: &mut bool,
) -> Result<(), ()> {
    use iced::futures::SinkExt;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut reader = tokio::io::BufReader::new(stream);
    let mut generation: Option<String> = None;
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| ())?;
        if read == 0 || line.len() > MAX_LINE || line.last() != Some(&b'\n') {
            return Ok(());
        }
        let snapshot = parse(&line[..line.len() - 1]).map_err(|_| ())?;
        // A restarted service is a new generation: its ids start over.
        if generation
            .as_ref()
            .is_some_and(|g| *g != snapshot.generation)
            && output.send(Event::Cleared).await.is_err()
        {
            return Err(());
        }
        generation = Some(snapshot.generation.clone());
        *received = true;
        output
            .send(Event::Snapshot(Box::new(snapshot)))
            .await
            .map_err(|_| ())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(jobs: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"op": "snapshot", "version": 1, "generation": "g1", "revision": 3, "jobs": jobs})).unwrap()
    }

    fn job(id: &str, pid: u32, token: Option<&str>) -> serde_json::Value {
        json!({"id": id, "app_id": "io.github.enzui.Files", "peer_pid": pid, "outcome": null, "age_ms": 0,
               "work": {"job": "copy", "label": "Copying files", "detail": "42 items", "state": "running",
                        "progress": {"done": 1, "total": 4, "unit": "items"}, "eta_ms": 15000, "window_token": token}})
    }

    #[test]
    fn activity_behaviour() {
        // Snapshots are checked against the protocol.
        {
            let snapshot = parse(&line(json!([job("1:copy", 10, None)]))).unwrap();
            assert_eq!(
                (
                    snapshot.generation.as_str(),
                    snapshot.revision,
                    snapshot.jobs.len()
                ),
                ("g1", 3, 1)
            );
            let mut extra = job("1:copy", 10, None);
            extra["work"]["icon"] = json!("/usr/bin/evil");
            assert!(
                parse(&line(json!([extra]))).is_err(),
                "unknown fields are refused"
            );
            let mut bidi = job("1:copy", 10, None);
            bidi["work"]["label"] = json!("Copying \u{202e}txt.exe");
            assert!(parse(&line(json!([bidi]))).is_err());
            let mut progress = job("1:copy", 10, None);
            progress["work"]["progress"] = json!({"done": 5, "total": 4, "unit": "items"});
            assert!(parse(&line(json!([progress]))).is_err());
            let mut unit = job("1:copy", 10, None);
            unit["work"]["progress"]["unit"] = json!("parsecs");
            assert!(parse(&line(json!([unit]))).is_err());
            let mut version =
                serde_json::from_slice::<serde_json::Value>(&line(json!([]))).unwrap();
            version["version"] = json!(2);
            assert!(parse(&serde_json::to_vec(&version).unwrap()).is_err());
            let many: Vec<_> = (0..65)
                .map(|i| job(&format!("{i}:copy"), 10, None))
                .collect();
            assert!(parse(&line(json!(many))).is_err());
        }
        // Only the originating window hides its work.
        {
            let mut snapshot = parse(&line(json!([
                job("1:a", 10, Some("files-w1")),
                job("2:b", 10, Some("files-w2")),
                job("3:c", 20, None)
            ])))
            .unwrap();
            let mut timer = job("4:timer", 10, Some("files-w1"));
            timer["work"]["visibility"] = json!("always");
            snapshot
                .jobs
                .extend(parse(&line(json!([timer]))).unwrap().jobs);
            let ids = |windows: &Windows| {
                visible(&snapshot, windows)
                    .iter()
                    .map(|j| j.id.clone())
                    .collect::<Vec<_>>()
            };
            let tagged = vec![(10, "files-w1".to_owned()), (10, "files-w2".to_owned())];
            assert_eq!(
                ids(&Windows::default()),
                ["1:a", "2:b", "3:c", "4:timer"],
                "no active window hides nothing"
            );
            let first = Windows {
                active: Some((10, Some("files-w1".into()))),
                tagged: tagged.clone(),
            };
            assert_eq!(
                ids(&first),
                ["2:b", "3:c", "4:timer"],
                "the other Files window's copy stays"
            );
            let other = Windows {
                active: Some((30, None)),
                tagged: tagged.clone(),
            };
            assert_eq!(ids(&other), ["1:a", "2:b", "3:c", "4:timer"]);
            let untagged = Windows {
                active: Some((20, None)),
                tagged,
            };
            assert_eq!(
                ids(&untagged),
                ["1:a", "2:b", "4:timer"],
                "without a token the process is the origin"
            );
            let unverified = Windows {
                active: Some((10, None)),
                tagged: vec![(99, "files-w1".into())],
            };
            assert_eq!(ids(&unverified), ["3:c", "4:timer"]);
            let spoof = Windows {
                active: Some((30, Some("files-w1".into()))),
                tagged: vec![(10, "files-w1".into()), (30, "files-w1".into())],
            };
            assert_eq!(ids(&spoof), ["1:a", "2:b", "3:c", "4:timer"]);
        }
    }
}
