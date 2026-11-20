use super::{
    server::{self, Command, Snapshot},
    session::Target,
};
use crate::settings::Store;
use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, Stream};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

#[derive(Clone)]
pub enum Update {
    Ready {
        epoch: u64,
        commands: mpsc::Sender<Command>,
        authority: Arc<AtomicBool>,
    },
    Snapshot(Arc<Snapshot>),
    Operation(server::Event),
    Storage(Result<(), String>),
    Stopped(String),
}
#[derive(Clone)]
pub struct Event {
    pub run: u64,
    pub update: Update,
}
impl std::fmt::Debug for Event {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NotificationRuntimeEvent")
    }
}
#[derive(Clone, Hash)]
struct Config {
    root: PathBuf,
    settings: PathBuf,
    run: u64,
}
struct Authority(Arc<AtomicBool>);
impl Drop for Authority {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
type Sink = iced::futures::channel::mpsc::Sender<Event>;
pub fn subscription(root: PathBuf, settings: PathBuf, run: u64) -> iced::Subscription<Event> {
    iced::Subscription::run_with(
        Config {
            root,
            settings,
            run,
        },
        stream,
    )
}
fn stream(config: &Config) -> impl Stream<Item = Event> + use<> {
    let config = config.clone();
    iced::stream::channel(16, async move |mut output| {
        let run = config.run;
        if let Err(error) = run_service(config, output.clone()).await {
            let _ = output
                .send(Event {
                    run,
                    update: Update::Stopped(error),
                })
                .await;
        }
        std::future::pending::<()>().await;
    })
}
fn retention_due(
    previous_ms: u64,
    now_ms: u64,
    minutes: u32,
    timestamps: impl Iterator<Item = u64>,
) -> bool {
    if now_ms < previous_ms {
        return true;
    }
    let age = u64::from(minutes) * 60000;
    timestamps
        .map(|at| at.saturating_add(age))
        .any(|deadline| deadline > previous_ms && deadline <= now_ms)
}
#[derive(Default)]
struct Retry {
    at: Option<Instant>,
    step: usize,
}
impl Retry {
    fn ready(&self, now: Instant) -> bool {
        self.at.is_none_or(|at| now >= at)
    }
    fn complete(&mut self, now: Instant, success: bool) {
        if success {
            *self = Self::default();
        } else {
            let delays = [1, 2, 4, 8, 16, 30];
            self.at = Some(now + Duration::from_secs(delays[self.step]));
            self.step = (self.step + 1).min(delays.len() - 1);
        }
    }
}
#[derive(Default)]
struct StorageReport {
    current: Option<Result<(), String>>,
    sent: Option<Result<(), String>>,
}
impl StorageReport {
    fn pending(&self) -> bool {
        self.current != self.sent
    }
    fn deliver(&mut self, channel: &mpsc::Sender<Result<(), String>>) -> bool {
        if channel.is_closed() {
            return false;
        }
        if self.pending()
            && let Some(result) = &self.current
        {
            match channel.try_send(result.clone()) {
                Ok(()) => self.sent = self.current.clone(),
                Err(mpsc::error::TrySendError::Full(_)) => (),
                Err(mpsc::error::TrySendError::Closed(_)) => return false,
            }
        }
        true
    }
}
fn observe_snapshot(
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
    latest: &mut Arc<Snapshot>,
    retry: &mut Retry,
) {
    let next = snapshots.borrow_and_update().clone();
    if next.policy.history_generation != latest.policy.history_generation
        || next.policy.history != latest.policy.history
    {
        *retry = Retry::default();
    }
    *latest = next;
}
async fn persist(
    store: Store,
    mut snapshots: watch::Receiver<Arc<Snapshot>>,
    authority: Arc<AtomicBool>,
    statuses: mpsc::Sender<Result<(), String>>,
) {
    let mut dirty = false;
    let mut latest = snapshots.borrow().clone();
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // History expires by the minute: with nothing to save, a kept history is
    // looked at once a minute rather than every second.
    let mut retention = tokio::time::interval(Duration::from_secs(60));
    retention.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut retry = Retry::default();
    let mut report = StorageReport::default();
    let mut attempted_at = None;
    loop {
        let pending = dirty || retry.at.is_some() || report.pending();
        let tick = tokio::select! {
            biased;
            _ = timer.tick(), if pending => true,
            _ = retention.tick(), if !pending && latest.policy.history => true,
            changed=snapshots.changed() => {
                if changed.is_err() { break; }
                observe_snapshot(&mut snapshots, &mut latest, &mut retry); dirty=true;
                false
            }
        };
        if tick {
            if !authority.load(Ordering::Acquire) {
                break;
            }
            match snapshots.has_changed() {
                Ok(true) => {
                    observe_snapshot(&mut snapshots, &mut latest, &mut retry);
                    dirty = true;
                }
                Ok(false) => (),
                Err(_) => break,
            }
            if !report.deliver(&statuses) {
                break;
            }
            if !retry.ready(Instant::now()) {
                continue;
            }
            let now = wall_ms();
            let due = attempted_at.is_some_and(|previous| {
                retention_due(
                    previous,
                    now,
                    latest.policy.history_minutes,
                    latest
                        .active
                        .iter()
                        .filter(|entry| !entry.draft.transient)
                        .map(|entry| entry.updated_ms)
                        .chain(latest.history.iter().map(|entry| entry.closed_ms)),
                )
            });
            if !dirty && !due && retry.at.is_none() {
                continue;
            }
            dirty = false;
            let snapshot = std::sync::Arc::clone(&latest);
            let store = store.clone();
            let authority = std::sync::Arc::clone(&authority);
            let result = tokio::task::spawn_blocking(move || {
                if !authority.load(Ordering::Acquire) {
                    return Ok(());
                }
                store.observe_notification_applications(&snapshot, &authority)?;
                store.save_notification_history_for(&snapshot, now, &authority)?;
                Ok::<_, String>(())
            })
            .await
            .unwrap_or_else(|_| Err(tr("shell-notification-storage-worker-stopped").into()));
            attempted_at = Some(now);
            retry.complete(Instant::now(), result.is_ok());
            report.current = Some(result);
            if !report.deliver(&statuses) {
                break;
            }
        }
        if !authority.load(Ordering::Acquire) {
            break;
        }
    }
}
async fn run_service(config: Config, mut output: Sink) -> Result<(), String> {
    tokio::task::spawn_blocking(Target::from_environment)
        .await
        .map_err(|_| tr("shell-notification-scope-reader-stopped"))??;
    let store = Store::at(config.settings)?;
    let initial_store = store.clone();
    let initial =
        tokio::task::spawn_blocking(move || initial_store.read_notification_state(wall_ms()))
            .await
            .map_err(|_| tr("shell-notification-preferences-reader-stopped"))??;
    let (history, history_error) = match initial.history {
        Ok(history) => (history, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    if let Some(error) = history_error {
        output
            .send(Event {
                run: config.run,
                update: Update::Storage(Err(error)),
            })
            .await
            .map_err(|_| tr("shell-notification-frontend-closed"))?;
    }
    let epoch = server::epoch()?;
    let (commands, input) = mpsc::channel(16);
    let (events, mut receiver) = mpsc::channel(16);
    let (snapshots, mut observer) = watch::channel(Arc::new(Snapshot::empty(
        epoch,
        initial.settings.notifications.policy.clone(),
    )));
    let (statuses, mut status) = mpsc::channel(4);
    let mut workers = JoinSet::new();
    let authority = Authority(Arc::new(AtomicBool::new(true)));
    workers.spawn(server::run_initialized(
        server::Initial {
            root: config.root,
            policy: initial.settings.notifications.policy.clone(),
            history,
        },
        input,
        snapshots,
        events,
    ));
    let save_authority = std::sync::Arc::clone(&authority.0);
    let save_observer = observer.clone();
    workers.spawn(async move {
        persist(store, save_observer, save_authority, statuses).await;
        Ok(())
    });
    loop {
        let update = tokio::select! {
            biased;
            ended=workers.join_next() => {
                authority.0.store(false,Ordering::Release);
                return match ended {Some(Ok(Err(error)))=>Err(error),Some(Err(_))=>Err(tr("shell-notification-runtime-worker-stopped").into()),_=>Err(tr("shell-notification-service-stopped").into())};
            }
            event=receiver.recv() => match event {
                Some(server::Event::Ready(ready)) if ready==epoch => Update::Ready {epoch,commands:commands.clone(),authority:std::sync::Arc::clone(&authority.0)},
                Some(event)=>Update::Operation(event),
                None=>return Err(tr("shell-notification-service-channel-closed").into()),
            },
            changed=observer.changed() => { changed.map_err(|_| tr("shell-notification-state-channel-closed"))?; Update::Snapshot(observer.borrow_and_update().clone()) },
            result=status.recv() => Update::Storage(result.ok_or_else(|| tr("shell-notification-storage-channel-closed"))?),
        };
        output
            .send(Event {
                run: config.run,
                update,
            })
            .await
            .map_err(|_| tr("shell-notification-frontend-closed"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_recovery_status_retries_after_a_full_channel_without_replaying_old_errors() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender.try_send(Err("occupied".into())).unwrap();
        let mut report = StorageReport {
            current: Some(Err("failed".into())),
            sent: None,
        };
        assert!(report.deliver(&sender));
        assert!(report.pending());
        report.current = Some(Ok(()));
        receiver.try_recv().unwrap().unwrap_err();
        assert!(report.deliver(&sender));
        assert_eq!(receiver.try_recv().unwrap(), Ok(()));
        assert!(!report.pending());
        assert!(report.deliver(&sender));
        assert!(receiver.try_recv().is_err());
        drop(receiver);
        assert!(!report.deliver(&sender));
    }
}
