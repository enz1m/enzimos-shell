use crate::plugin_clipboard::Message;
use enzimos_phone_client::{ControlRequest, Reply};
use enzui_shell::session_activity::{self, State};
use iced::futures::{SinkExt, Stream, StreamExt};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Handle {
    pub authority: Arc<AtomicBool>,
    pub since: Instant,
    pending: Arc<Mutex<Option<String>>>,
}
impl Handle {
    pub(super) fn publish(&self, text: String) {
        if self.authority.load(Ordering::Acquire) {
            *self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(text);
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.pending) == 1 {
            self.authority.store(false, Ordering::Release);
        }
    }
}
#[derive(Clone)]
pub(super) struct Notice {
    pub handle: Option<Handle>,
    pub collecting: bool,
    pub text: Option<String>,
}
fn revoke(handle: &mut Option<Handle>) {
    if let Some(handle) = handle.take() {
        handle.authority.store(false, Ordering::Release);
        handle
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}
struct Stop(Option<Handle>);
impl Drop for Stop {
    fn drop(&mut self) {
        revoke(&mut self.0);
    }
}

pub(super) fn stream(path: &Path) -> impl Stream<Item = Message> + use<> {
    let path = path.to_path_buf();
    iced::stream::channel(4, move |output| {
        follow(path, session_activity::stream(), output)
    })
}

async fn follow(
    path: PathBuf,
    activity: impl Stream<Item = State>,
    mut output: iced::futures::channel::mpsc::Sender<Message>,
) {
    iced::futures::pin_mut!(activity);
    let mut active = false;
    let mut next_poll = tokio::time::Instant::now();
    let mut since = None;
    let mut current = Stop(None);
    let mut quiet = Quiet::default();
    loop {
        enum Update {
            Activity(Option<State>),
            Reply(Result<Reply, std::io::Error>),
        }
        let update = tokio::select! {
            biased;
            state = activity.next() => Update::Activity(state),
            () = tokio::time::sleep_until(next_poll), if active => {
                let text = current.0.as_ref().and_then(|handle| {
                    handle
                        .pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                });
                let exchange = async {
                    if let Some(text) = text {
                        enzimos_phone_client::request(
                            &path,
                            &ControlRequest::ClipboardPublish { text },
                        )
                        .await?;
                    }
                    enzimos_phone_client::request(&path, &ControlRequest::ClipboardPoll { since }).await
                };
                tokio::select! {
                    biased;
                    state = activity.next() => Update::Activity(state),
                    reply = exchange => Update::Reply(reply),
                }
            }
        };
        let notice = match update {
            Update::Activity(state) => {
                quiet = Quiet::default();
                revoke(&mut current.0);
                since = None;
                active = state == Some(State::Active);
                next_poll = tokio::time::Instant::now();
                if state.is_none() {
                    return;
                }
                Notice {
                    handle: None,
                    collecting: false,
                    text: None,
                }
            }
            Update::Reply(Ok(reply)) if reply.0["ok"] == true && reply.0["available"] == true => {
                next_poll = tokio::time::Instant::now() + ACTIVE_POLL_PERIOD;
                since = reply.0["revision"].as_u64();
                let handle = current.0.get_or_insert_with(|| Handle {
                    authority: Arc::new(AtomicBool::new(true)),
                    since: Instant::now(),
                    pending: Arc::new(Mutex::new(None)),
                });
                Notice {
                    handle: Some(handle.clone()),
                    collecting: reply.0["enabled"] == true,
                    text: reply.0["text"]
                        .as_str()
                        .filter(|text| !text.is_empty() && text.len() <= 48 * 1024)
                        .map(str::to_owned),
                }
            }
            Update::Reply(_) => {
                next_poll = tokio::time::Instant::now() + UNAVAILABLE_POLL_PERIOD;
                revoke(&mut current.0);
                since = None;
                Notice {
                    handle: None,
                    collecting: false,
                    text: None,
                }
            }
        };
        if quiet.should_send(&notice, Instant::now())
            && output.send(Message::Phone(notice)).await.is_err()
        {
            return;
        }
    }
}

// Each notice redraws the whole shell, so unchanged ones are dropped.
#[derive(Default)]
struct Quiet {
    sent: Option<(Option<usize>, bool, Instant)>,
}
impl Quiet {
    // Unchanged notices still go out every RESEND so a busy channel is retried.
    fn should_send(&mut self, notice: &Notice, now: Instant) -> bool {
        let handle = notice
            .handle
            .as_ref()
            .map(|handle| Arc::as_ptr(&handle.authority) as usize);
        if notice.text.is_none()
            && self.sent.is_some_and(|(sent, collecting, at)| {
                sent == handle && collecting == notice.collecting && now - at < RESEND
            })
        {
            return false;
        }
        self.sent = Some((handle, notice.collecting, now));
        true
    }
}
const ACTIVE_POLL_PERIOD: Duration = Duration::from_millis(300);
const UNAVAILABLE_POLL_PERIOD: Duration = Duration::from_secs(3);
const RESEND: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn request(
        listener: &tokio::net::UnixListener,
    ) -> (tokio::net::UnixStream, serde_json::Value) {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut socket)
            .read_line(&mut line)
            .await
            .unwrap();
        (socket, serde_json::from_str(&line).unwrap())
    }
    async fn notice(output: &mut iced::futures::channel::mpsc::Receiver<Message>) -> Notice {
        let event = tokio::time::timeout(Duration::from_secs(3), output.next())
            .await
            .unwrap()
            .unwrap();
        match event {
            Message::Phone(notice) => notice,
            _ => panic!("unexpected transport event"),
        }
    }

    #[tokio::test]
    async fn lock_cancels_inflight_clipboard_and_reconnect_does_not_restore_old_text() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().join("control.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (mut states, activity) = iced::futures::channel::mpsc::channel(4);
        let (tx, mut output) = iced::futures::channel::mpsc::channel(4);
        let worker = tokio::spawn(follow(path, activity, tx));
        let (seen, mut requests) = tokio::sync::mpsc::channel(4);
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut socket, first) = request(&listener).await;
            seen.send(first).await.unwrap();
            socket.write_all(b"{\"ok\":true,\"available\":true,\"enabled\":true,\"revision\":1,\"text\":null}\n").await.unwrap();
            let (mut socket, publish) = request(&listener).await;
            seen.send(publish).await.unwrap();
            socket.write_all(b"{\"ok\":true}\n").await.unwrap();
            let (mut socket, pending) = request(&listener).await;
            seen.send(pending).await.unwrap();
            held.await.unwrap();
            let _ = socket.write_all(b"{\"ok\":true,\"available\":true,\"enabled\":true,\"revision\":2,\"text\":\"stale fixture\"}\n").await;
            let (mut socket, reconnect) = request(&listener).await;
            seen.send(reconnect).await.unwrap();
            socket.write_all(b"{\"ok\":true,\"available\":true,\"enabled\":true,\"revision\":3,\"text\":null}\n").await.unwrap();
        });
        states.send(State::Active).await.unwrap();
        assert!(notice(&mut output).await.handle.is_none());
        let first = notice(&mut output).await;
        let old = first.handle.unwrap();
        assert!(first.collecting);
        let first_request = requests.recv().await.unwrap();
        assert_eq!(first_request["cmd"], "clipboard_poll");
        assert!(first_request["since"].is_null());
        old.publish("desktop fixture".into());
        let published = tokio::time::timeout(Duration::from_secs(3), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published["cmd"], "clipboard_publish");
        assert_eq!(published["text"], "desktop fixture");
        assert_eq!(requests.recv().await.unwrap()["since"], 1);
        states.send(State::Locked).await.unwrap();
        let locked = notice(&mut output).await;
        assert!(locked.handle.is_none());
        assert!(!old.authority.load(Ordering::Acquire));
        release.send(()).unwrap();
        old.publish("stale queued fixture".into());
        states.send(State::Active).await.unwrap();
        assert!(notice(&mut output).await.handle.is_none());
        let resumed = notice(&mut output).await;
        assert!(
            resumed.text.is_none(),
            "a cancelled response must never restore after unlock"
        );
        let fresh = resumed.handle.unwrap();
        assert!(fresh.authority.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&old.authority, &fresh.authority));
        let reconnect = requests.recv().await.unwrap();
        assert_eq!(
            reconnect["cmd"], "clipboard_poll",
            "old queued text leaked after unlock"
        );
        assert!(reconnect["since"].is_null());
        worker.abort();
        let _ = worker.await;
        assert!(!fresh.authority.load(Ordering::Acquire));
        server.await.unwrap();
    }
}
