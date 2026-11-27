use enzui_shell::clipboard::{
    content::{Content, Mime},
    session::Target,
    transport::{self, Command, Event},
};
use enzui_shell::i18n::tr;
use enzui_shell::search::{Category, Item, Target as SearchTarget};
use iced::{
    Subscription,
    futures::{SinkExt, Stream},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const BUDGET: usize = 4 * 1024 * 1024;
pub(super) struct Entry {
    pub id: u64,
    pub content: Arc<Content>,
    pub item: Item,
    at: Instant,
}
pub(super) struct State {
    pub epoch: u64,
    pub enabled: bool,
    pub target: Option<Target>,
    pub channel: Option<mpsc::Sender<Command>>,
    pub entries: VecDeque<Entry>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub clear_generation: u64,
    pub minutes: u16,
    pub since: Instant,
    next_id: u64,
    next_request: u64,
    pending_restore: Option<u64>,
    authority: Arc<AtomicBool>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            epoch: 0,
            enabled: false,
            target: None,
            channel: None,
            entries: VecDeque::new(),
            error: None,
            notice: None,
            clear_generation: 0,
            minutes: 30,
            since: Instant::now(),
            next_id: 0,
            next_request: 0,
            pending_restore: None,
            authority: Arc::new(AtomicBool::new(false)),
        }
    }
}
#[derive(Clone)]
pub(super) enum Message {
    Connected(u64, mpsc::Sender<Command>),
    Event(u64, Event),
    Failed(u64, String),
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(tr("shell-clipboard-transport-event"))
    }
}
#[derive(Clone, Hash)]
struct Config {
    target: Target,
    epoch: u64,
}
struct Stop(Arc<AtomicBool>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn stream(config: &Config) -> impl Stream<Item = Message> + use<> {
    let Config { target, epoch } = config.clone();
    iced::stream::channel(8, async move |mut output| {
        let alive = Arc::new(AtomicBool::new(true));
        let _stop = Stop(std::sync::Arc::clone(&alive));
        let (tx, commands) = mpsc::channel(8);
        let (events, mut incoming) = mpsc::channel(4);
        if output.send(Message::Connected(epoch, tx)).await.is_err() {
            return;
        }
        let mut worker =
            tokio::task::spawn_blocking(move || transport::run(target, commands, events, alive));
        let reason = loop {
            tokio::select! {
                event = incoming.recv() => match event {
                    Some(event) => if output.send(Message::Event(epoch,event)).await.is_err() { return; },
                    None => break tr("shell-clipboard-transport-stopped").to_owned(),
                },
                result = &mut worker => break match result { Ok(Err(error)) => error, _ => tr("shell-clipboard-transport-stopped").into() },
            }
        };
        let _ = output.send(Message::Failed(epoch, reason)).await;
        std::future::pending::<()>().await;
    })
}
impl State {
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        self.target
            .clone()
            .filter(|_| self.enabled)
            .map_or_else(Subscription::none, |target| {
                Subscription::run_with(
                    Config {
                        target,
                        epoch: self.epoch,
                    },
                    stream,
                )
            })
    }
    pub(crate) fn configure(
        &mut self,
        enabled: bool,
        generation: u64,
        minutes: u16,
    ) -> Option<u64> {
        self.minutes = minutes;
        if self.clear_generation != generation {
            self.clear_generation = generation;
            self.entries.clear();
            self.notice = None;
            self.since = Instant::now();
        }
        self.expire();
        if enabled == self.enabled {
            return None;
        }
        self.authority.store(false, Ordering::Release);
        self.authority = Arc::new(AtomicBool::new(enabled));
        self.enabled = enabled;
        self.entries.clear();
        self.target = None;
        self.channel = None;
        self.pending_restore = None;
        self.error = None;
        self.notice = None;
        self.since = Instant::now();
        self.epoch = self.epoch.checked_add(1)?;
        enabled.then_some(self.epoch)
    }
    pub(crate) fn retry(&mut self) -> Option<u64> {
        if !self.enabled || self.error.is_none() {
            return None;
        }
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.error = Some(tr("shell-clipboard-generations-exhausted").into());
            return None;
        };
        self.authority.store(false, Ordering::Release);
        self.authority = Arc::new(AtomicBool::new(true));
        self.epoch = epoch;
        self.channel = None;
        self.target = None;
        self.error = None;
        self.pending_restore = None;
        Some(epoch)
    }
    pub(crate) fn expire(&mut self) -> bool {
        let count = self.entries.len();
        let retention = Duration::from_secs(u64::from(self.minutes) * 60);
        self.entries.retain(|entry| entry.at.elapsed() < retention);
        count != self.entries.len()
    }
    pub(crate) fn message(&mut self, message: Message) -> Option<Result<(), String>> {
        match message {
            Message::Connected(epoch, channel) if epoch == self.epoch && self.enabled => {
                self.channel = Some(channel);
            }
            Message::Event(epoch, event) if epoch == self.epoch && self.enabled => match event {
                Event::Ready => {
                    if self
                        .channel
                        .as_ref()
                        .is_none_or(|channel| channel.try_send(Command::Collect(true)).is_err())
                    {
                        self.error = Some(tr("shell-clipboard-collection-is-unavailable").into());
                    }
                }
                Event::Copied { content, started } if started >= self.since => {
                    self.expire();
                    if content.size() > BUDGET {
                        self.notice = Some(
                            tr("shell-clipboard-item-exceeds-the-4-mib-history-budget-and-was-not-recorded")
                                .into(),
                        );
                        return None;
                    }
                    self.notice = None;
                    if let Some(index) = self
                        .entries
                        .iter()
                        .position(|entry| entry.content.digest == content.digest)
                    {
                        if let Some(entry) = self.entries.remove(index) {
                            self.entries.push_front(entry);
                        }
                        return None;
                    }
                    while self.entries.len() >= 64
                        || self
                            .entries
                            .iter()
                            .map(|entry| entry.content.size())
                            .sum::<usize>()
                            + content.size()
                            > BUDGET
                    {
                        self.entries.pop_back();
                    }
                    let Some(id) = self.next_id.checked_add(1) else {
                        self.error = Some(tr("shell-clipboard-identities-exhausted").into());
                        return None;
                    };
                    self.next_id = id;
                    let image = content.mime == Mime::Png;
                    let mut item = Item::new(
                        format!("clipboard:{id}"),
                        Category::Clipboard,
                        content.preview.clone(),
                        if image {
                            tr("shell-image-copy-to-clipboard-never-automatic-paste")
                        } else {
                            tr("shell-text-copy-to-clipboard-never-automatic-paste")
                        },
                        tr("shell-copy"),
                        if image { "image" } else { "clipboard" },
                        Some(SearchTarget::Clipboard(id)),
                    );
                    if !image && let Ok(text) = std::str::from_utf8(&content.bytes) {
                        item = item.search_terms(text);
                    }
                    self.entries.push_front(Entry {
                        id,
                        content,
                        item,
                        at: started,
                    });
                }
                Event::Restored { request, result } if self.pending_restore == Some(request) => {
                    self.pending_restore = None;
                    return Some(result);
                }
                _ => {}
            },
            Message::Failed(epoch, error) if epoch == self.epoch => {
                self.error = Some(error.clone());
                self.channel = None;
                if self.pending_restore.take().is_some() {
                    return Some(Err(error));
                }
            }
            _ => {}
        }
        None
    }
    pub(crate) fn copy(&mut self, id: u64) -> Result<(), String> {
        self.expire();
        if !self.enabled {
            return Err(tr("shell-clipboard-history-is-disabled").into());
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| tr("shell-clipboard-item-expired-or-was-cleared"))?;
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| tr("shell-clipboard-request-identities-exhausted"))?;
        self.channel
            .as_ref()
            .ok_or_else(|| tr("shell-clipboard-service-is-unavailable"))?
            .try_send(Command::Restore {
                request: self.next_request,
                content: std::sync::Arc::clone(&entry.content),
                authority: std::sync::Arc::clone(&self.authority),
            })
            .map_err(|_| tr("shell-clipboard-service-is-busy").to_owned())?;
        self.pending_restore = Some(self.next_request);
        Ok(())
    }
}
