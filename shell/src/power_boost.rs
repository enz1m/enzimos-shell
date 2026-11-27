//! Asks `dev.enzui.PowerBoost1` to raise CPU performance while desktop motion starts.
//!
//! [`request`] never blocks the caller. A worker thread sends at most one
//! request per [`RESEND`] and stops asking for a while when the helper
//! declines, so hosts without the helper or with a performance profile cost
//! one bus call.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const NAME: &str = "dev.enzui.PowerBoost1";
const PATH: &str = "/dev/enzui/PowerBoost1";
// The helper holds a boost for 1.5 s, so continuous motion keeps it raised.
const RESEND: Duration = Duration::from_millis(500);
const DECLINED_PAUSE: Duration = Duration::from_secs(30);
const CALL_BOUND: Duration = Duration::from_secs(1);
const NEVER: u64 = u64::MAX;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Asks for a boost because an animation or interaction is starting.
pub fn request() {
    static WAKE: OnceLock<Option<tokio::sync::mpsc::Sender<()>>> = OnceLock::new();
    let now = now();
    let next = NEXT.load(Ordering::Relaxed);
    if now < next
        || NEXT
            .compare_exchange(
                next,
                now.saturating_add(millis(RESEND)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
    {
        return;
    }
    if let Some(wake) = WAKE.get_or_init(start) {
        let _ = wake.try_send(());
    }
}

fn start() -> Option<tokio::sync::mpsc::Sender<()>> {
    let (wake, requests) = tokio::sync::mpsc::channel(1);
    std::thread::Builder::new()
        .name("power-boost".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(serve(requests));
        })
        .ok()?;
    Some(wake)
}

async fn serve(mut requests: tokio::sync::mpsc::Receiver<()>) {
    let Some(connection) = crate::bus::bounded(zbus::Connection::system())
        .await
        .and_then(Result::ok)
    else {
        NEXT.store(NEVER, Ordering::Relaxed);
        return;
    };
    while requests.recv().await.is_some() {
        let reply = tokio::time::timeout(
            CALL_BOUND,
            connection.call_method(Some(NAME), PATH, Some(NAME), "Boost", &()),
        )
        .await;
        let answer = match &reply {
            Ok(Ok(message)) => match message.body().deserialize::<String>() {
                Ok(outcome) => pause(Answer::Outcome(&outcome)),
                Err(_) => pause(Answer::Failed),
            },
            Ok(Err(zbus::Error::MethodError(name, _, _)))
                if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" =>
            {
                pause(Answer::Missing)
            }
            _ => pause(Answer::Failed),
        };
        match answer {
            Some(Duration::MAX) => NEXT.store(NEVER, Ordering::Relaxed),
            Some(pause) => NEXT.store(now().saturating_add(millis(pause)), Ordering::Relaxed),
            None => {}
        }
    }
}

enum Answer<'a> {
    Outcome(&'a str),
    Missing,
    Failed,
}

// `Duration::MAX` stops asking for the rest of the process.
fn pause(answer: Answer<'_>) -> Option<Duration> {
    match answer {
        Answer::Outcome("boosted" | "extended" | "busy") => None,
        Answer::Outcome("unsupported") | Answer::Missing => Some(Duration::MAX),
        Answer::Outcome(_) | Answer::Failed => Some(DECLINED_PAUSE),
    }
}

fn now() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    millis(EPOCH.get_or_init(Instant::now).elapsed())
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(NEVER)
}
