use crate::{Message as ShellMessage, Shell, plugin_panel};
use enzui_shell::i18n::tr;
use enzui_shell::{
    clipboard::{
        history::History,
        session::Target,
        transport::{self, Command, Event},
    },
    plugins::service::{Command as PluginCommand, Owner},
};
use iced::{
    Subscription, Task,
    futures::{SinkExt, Stream},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::mpsc;

struct Watcher {
    owner: Owner,
    authority: Arc<AtomicBool>,
    collecting: bool,
    since: Instant,
}
#[derive(Default)]
pub(super) struct State {
    watchers: BTreeMap<String, Watcher>,
    history: History,
    target: Option<Target>,
    epoch: u64,
    channel: Option<mpsc::Sender<Command>>,
    collection_sent: Option<bool>,
    error: Option<String>,
    pending: BTreeMap<u64, (Owner, u64)>,
    next_request: u64,
    phone: Option<crate::phone_clipboard::Handle>,
    phone_collecting: bool,
    phone_pending: Option<Arc<enzui_shell::clipboard::content::Content>>,
}
#[derive(Clone)]
pub(super) enum Message {
    Phone(crate::phone_clipboard::Notice),
    Connected(u64, mpsc::Sender<Command>),
    Event(u64, Event),
    Unavailable(u64, String),
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
        let (events, mut rx) = mpsc::channel(4);
        if output.send(Message::Connected(epoch, tx)).await.is_err() {
            return;
        }
        let mut worker =
            tokio::task::spawn_blocking(move || transport::run(target, commands, events, alive));
        let reason = loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Some(event) => if output.send(Message::Event(epoch, event)).await.is_err() { return; },
                    None => break match (&mut worker).await {
                        Ok(Err(reason)) => reason,
                        _ => tr("shell-clipboard-transport-stopped").into(),
                    },
                },
                result = &mut worker => break match result {
                    Ok(Err(reason)) => reason,
                    _ => tr("shell-clipboard-transport-stopped").into(),
                },
            }
        };
        let _ = output.send(Message::Unavailable(epoch, reason)).await;
        std::future::pending::<()>().await;
    })
}
fn message(value: Message) -> ShellMessage {
    ShellMessage::Plugin(plugin_panel::Message::Clipboard(value))
}
impl Shell {
    pub(super) fn clipboard_subscription(&self) -> Subscription<ShellMessage> {
        self.plugin
            .clipboard
            .target
            .clone()
            .map_or_else(Subscription::none, |target| {
                Subscription::run_with(
                    Config {
                        target,
                        epoch: self.plugin.clipboard.epoch,
                    },
                    stream,
                )
                .map(message)
            })
    }
    fn clipboard_provider(&mut self, owner: Owner, data: serde_json::Value) -> bool {
        if self.plugin.channel.as_ref().is_some_and(|channel| {
            channel
                .try_send(PluginCommand::Provider { owner, data })
                .is_ok()
        }) {
            true
        } else {
            self.plugin.error = Some(tr("shell-plugin-service-is-unavailable-or-busy").into());
            false
        }
    }
    fn clipboard_collect(&mut self) {
        let state = &mut self.plugin.clipboard;
        let collecting = (state.phone_collecting
            && state
                .phone
                .as_ref()
                .is_some_and(|phone| phone.authority.load(Ordering::Acquire)))
            || state
                .watchers
                .values()
                .any(|w| w.collecting && w.authority.load(Ordering::Acquire));
        if state.collection_sent == Some(collecting) {
            return;
        }
        if let Some(channel) = &state.channel {
            if channel.try_send(Command::Collect(collecting)).is_ok() {
                state.collection_sent = Some(collecting);
            } else {
                let epoch = state.epoch;
                let _ = self.clipboard_message(Message::Unavailable(
                    epoch,
                    tr("shell-clipboard-control-channel-is-busy").into(),
                ));
            }
        }
    }
    pub(super) fn clipboard_subscribe(
        &mut self,
        owner: Owner,
        id: u64,
        enabled: bool,
        authority: Arc<AtomicBool>,
    ) -> Task<ShellMessage> {
        let result = (|| {
            let state = &mut self.plugin.clipboard;
            if let Some(error) = &state.error {
                return Err(error.clone());
            }
            if state.target.is_none() {
                if !enabled {
                    return Ok(());
                }
                state.target = Some(Target::from_environment()?);
                state.epoch = state
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| tr("shell-clipboard-session-identities-exhausted"))?;
            }
            let watcher = state
                .watchers
                .entry(owner.id.clone())
                .or_insert_with(|| Watcher {
                    owner: owner.clone(),
                    authority,
                    collecting: false,
                    since: Instant::now(),
                });
            if watcher.owner != owner {
                return Err(tr("shell-clipboard-subscription-owner-changed").into());
            }
            if !watcher.collecting && enabled {
                watcher.since = Instant::now();
            }
            watcher.collecting = enabled;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.clipboard_collect();
                if let Some(error) = self.plugin.clipboard.error.clone() {
                    self.plugin_reply(owner, id, false, &error);
                    return Task::none();
                }
                self.plugin_reply(owner.clone(), id, true, "");
                self.clipboard_provider(owner, serde_json::json!({"provider":"clipboard","event":"status","state":if enabled {"collecting"} else {"paused"}}));
            }
            Err(error) => self.plugin_reply(owner, id, false, &error),
        }
        Task::none()
    }
    pub(super) fn clipboard_restore(
        &mut self,
        owner: Owner,
        id: u64,
        blob: &str,
        gesture: &str,
        authority: Arc<AtomicBool>,
    ) -> Task<ShellMessage> {
        let result = (|| {
            if !self.consume_plugin_gesture(&owner, gesture) {
                return Err(tr("shell-clipboard-restore-needs-a-current-user-action").into());
            }
            let state = &mut self.plugin.clipboard;
            if state.pending.len() >= 8 {
                return Err(tr("shell-clipboard-restore-is-busy").into());
            }
            let content = state
                .history
                .get(&owner, blob, Instant::now())
                .ok_or_else(|| tr("shell-clipboard-entry-is-no-longer-available"))?;
            let channel = state
                .channel
                .as_ref()
                .ok_or_else(|| tr("shell-clipboard-transport-is-unavailable"))?;
            state.next_request = state
                .next_request
                .checked_add(1)
                .ok_or_else(|| tr("shell-clipboard-request-identities-exhausted"))?;
            let request = state.next_request;
            channel
                .try_send(Command::Restore {
                    request,
                    content,
                    authority,
                })
                .map_err(|_| tr("shell-clipboard-restore-is-busy"))?;
            state.pending.insert(request, (owner.clone(), id));
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.plugin_reply(owner, id, false, &error);
        }
        Task::none()
    }
    pub(super) fn clipboard_release(
        &mut self,
        owner: Owner,
        id: u64,
        handles: &[String],
    ) -> Task<ShellMessage> {
        self.plugin.clipboard.history.release(&owner, handles);
        self.plugin_reply(owner, id, true, "");
        Task::none()
    }
    pub(super) fn clipboard_image(
        &self,
        owner: &Owner,
        handle: &str,
    ) -> Option<iced::widget::image::Handle> {
        self.plugin
            .clipboard
            .history
            .get(owner, handle, Instant::now())?
            .image
            .clone()
    }
    pub(super) fn clipboard_revoke(&mut self, owner: &Owner) {
        let state = &mut self.plugin.clipboard;
        if state
            .watchers
            .get(&owner.id)
            .is_some_and(|w| w.owner == *owner)
        {
            state.watchers.remove(&owner.id);
        }
        state.history.revoke(owner);
        state.pending.retain(|_, (expected, _)| expected != owner);
        if state.watchers.is_empty() && state.phone.is_none() {
            state.target = None;
            state.channel = None;
            state.collection_sent = None;
            state.error = None;
        }
        self.clipboard_collect();
    }
    pub(super) fn clipboard_clear(&mut self) {
        let epoch = self.plugin.clipboard.epoch;
        self.plugin.clipboard = State {
            epoch,
            ..Default::default()
        };
    }
    // Drops expired history entries; true when any were removed.
    pub(super) fn clipboard_expire(&mut self) -> bool {
        let mut batches: Vec<(Owner, Vec<String>)> = Vec::new();
        let expired = self.plugin.clipboard.history.expire(Instant::now());
        let any = !expired.is_empty();
        for (owner, handle) in expired {
            if let Some((_, handles)) = batches.iter_mut().find(|(expected, _)| *expected == owner)
            {
                handles.push(handle);
            } else {
                batches.push((owner, vec![handle]));
            }
        }
        for (owner, handles) in batches {
            if self
                .plugin
                .live
                .get(&owner.id)
                .is_some_and(|live| live.owner == owner && live.authority.load(Ordering::Acquire))
            {
                for chunk in handles.chunks(128) {
                    self.clipboard_provider(
                        owner.clone(),
                        serde_json::json!({"provider":"clipboard","event":"removed","blobs":chunk}),
                    );
                }
            }
        }
        any
    }
    fn phone_clipboard_restore(&mut self) {
        let state = &mut self.plugin.clipboard;
        let Some(phone) = state
            .phone
            .as_ref()
            .filter(|phone| phone.authority.load(Ordering::Acquire))
        else {
            state.phone_pending = None;
            return;
        };
        if let (Some(content), Some(channel)) = (&state.phone_pending, &state.channel) {
            let Some(request) = state.next_request.checked_add(1) else {
                return;
            };
            if channel
                .try_send(Command::Restore {
                    request,
                    content: Arc::clone(content),
                    authority: Arc::clone(&phone.authority),
                })
                .is_ok()
            {
                state.next_request = request;
                state.phone_pending = None;
            }
        }
    }
    fn phone_clipboard_notice(
        &mut self,
        notice: crate::phone_clipboard::Notice,
    ) -> Task<ShellMessage> {
        if notice
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.authority.load(Ordering::Acquire))
        {
            return Task::none();
        }
        let state = &mut self.plugin.clipboard;
        state.phone = notice.handle;
        state.phone_collecting = notice.collecting;
        if state.phone.is_none() {
            state.phone_pending = None;
            if state.watchers.is_empty() {
                state.target = None;
                state.channel = None;
                state.collection_sent = None;
            }
        } else {
            if state.target.is_none()
                && let Ok(target) = Target::from_environment()
                && let Some(epoch) = state.epoch.checked_add(1)
            {
                state.epoch = epoch;
                state.target = Some(target);
                state.error = None;
            }
            if let Some(text) = notice.text {
                state.phone_pending = enzui_shell::clipboard::content::Content::decode(
                    enzui_shell::clipboard::content::Mime::Text,
                    text.into_bytes(),
                )
                .ok()
                .map(Arc::new);
            }
        }
        self.clipboard_collect();
        self.phone_clipboard_restore();
        Task::none()
    }
    pub(super) fn clipboard_message(&mut self, event: Message) -> Task<ShellMessage> {
        let event = match event {
            Message::Phone(notice) => return self.phone_clipboard_notice(notice),
            event => event,
        };
        let epoch = match &event {
            Message::Phone(_) => return Task::none(),
            Message::Connected(epoch, _)
            | Message::Event(epoch, _)
            | Message::Unavailable(epoch, _) => *epoch,
        };
        if epoch != self.plugin.clipboard.epoch || self.plugin.clipboard.target.is_none() {
            return Task::none();
        }
        match event {
            Message::Phone(_) => return Task::none(),
            Message::Connected(_, channel) => {
                self.plugin.clipboard.channel = Some(channel);
                self.plugin.clipboard.collection_sent = None;
                self.clipboard_collect();
                self.phone_clipboard_restore();
            }
            Message::Unavailable(_, error) => {
                let state = &mut self.plugin.clipboard;
                state.error = Some(error.clone());
                state.channel = None;
                state.target = None;
                state.collection_sent = None;
                state.history.clear();
                let pending = std::mem::take(&mut state.pending);
                let owners: Vec<_> = state.watchers.values().map(|w| w.owner.clone()).collect();
                for (_, (owner, id)) in pending {
                    self.plugin_reply(owner, id, false, &error);
                }
                for owner in owners {
                    self.clipboard_provider(owner, serde_json::json!({"provider":"clipboard","event":"unavailable","message":error}));
                }
            }
            Message::Event(_, Event::Ready) => self.clipboard_collect(),
            Message::Event(_, Event::Restored { request, result }) => {
                if let Some((owner, id)) = self.plugin.clipboard.pending.remove(&request) {
                    match result {
                        Ok(()) => self.plugin_reply(owner, id, true, ""),
                        Err(error) => self.plugin_reply(owner, id, false, &error),
                    }
                }
            }
            Message::Event(_, Event::Rejected { error, started }) => {
                let owners: Vec<_> = self
                    .plugin
                    .clipboard
                    .watchers
                    .values()
                    .filter(|w| {
                        w.collecting && w.since <= started && w.authority.load(Ordering::Acquire)
                    })
                    .map(|w| w.owner.clone())
                    .collect();
                for owner in owners {
                    self.clipboard_provider(
                        owner,
                        serde_json::json!({"provider":"clipboard","event":"error","message":error}),
                    );
                }
            }
            Message::Event(_, Event::Copied { content, started }) => {
                if self.plugin.clipboard.phone_collecting
                    && matches!(content.mime, enzui_shell::clipboard::content::Mime::Text)
                    && content.bytes.len() <= 48 * 1024
                    && let Some(phone) = self
                        .plugin
                        .clipboard
                        .phone
                        .as_ref()
                        .filter(|phone| phone.since <= started)
                    && let Ok(text) = std::str::from_utf8(&content.bytes)
                {
                    phone.publish(text.to_owned());
                }
                self.clipboard_expire();
                let owners: Vec<_> = self
                    .plugin
                    .clipboard
                    .watchers
                    .values()
                    .filter(|w| {
                        w.collecting && w.since <= started && w.authority.load(Ordering::Acquire)
                    })
                    .map(|w| (w.owner.clone(), std::sync::Arc::clone(&w.authority)))
                    .collect();
                for (owner, authority) in owners {
                    match self.plugin.clipboard.history.insert(
                        owner.clone(),
                        authority,
                        std::sync::Arc::clone(&content),
                        Instant::now(),
                    ) {
                        Ok(published) => {
                            if !published.removed.is_empty() {
                                self.clipboard_provider(owner.clone(), serde_json::json!({"provider":"clipboard","event":"removed","blobs":published.removed}));
                            }
                            let handle = published.handle.clone();
                            if !self.clipboard_provider(owner.clone(), serde_json::json!({"provider":"clipboard","event":"entry","blob":published.handle,"preview":content.preview,"mime":content.mime.names()[0],"bytes":content.size(),"image":content.image.is_some(),"fresh":published.fresh})) && published.fresh {
                                self.plugin.clipboard.history.release(&owner, &[handle]);
                            }
                        }
                        Err(error) => {
                            self.clipboard_provider(owner, serde_json::json!({"provider":"clipboard","event":"error","message":error}));
                        }
                    }
                }
            }
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_shell::clipboard::content::{Content, Mime};
    use std::os::unix::fs::PermissionsExt;
    fn shell(root: &std::path::Path) -> (Shell, mpsc::Receiver<PluginCommand>) {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = root.join("enzui-shell-platform/fixture");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let compositor =
            enzui_shell::compositor::Compositor::new("/usr/bin/true".into(), "fixture".into())
                .unwrap();
        let (mut shell, _) = Shell::new(compositor, "/".into());
        let (tx, rx) = mpsc::channel(32);
        shell.plugin.channel = Some(tx);
        shell.plugin.clipboard.target =
            Some(Target::managed(runtime, root.into(), "wayland-1".into()).unwrap());
        shell.plugin.clipboard.epoch = 1;
        (shell, rx)
    }
    fn owner(run: u64) -> Owner {
        Owner {
            id: "fixture".into(),
            activation: 1,
            run,
        }
    }
    fn content() -> Arc<Content> {
        Arc::new(Content::decode(Mime::Text, b"fixture".to_vec()).unwrap())
    }
    #[test]
    fn plugin_clipboard_behaviour() {
        // Stale session events and old owner removal cannot destroy new resources.
        {
            let root = tempfile::tempdir().unwrap();
            let (mut shell, _) = shell(root.path());
            let now = Instant::now();
            let authority = Arc::new(AtomicBool::new(true));
            shell.plugin.clipboard.watchers.insert(
                "fixture".into(),
                Watcher {
                    owner: owner(2),
                    authority: std::sync::Arc::clone(&authority),
                    collecting: true,
                    since: now,
                },
            );
            let stored = shell
                .plugin
                .clipboard
                .history
                .insert(owner(2), authority, content(), now)
                .unwrap();
            let _ = shell.clipboard_message(Message::Unavailable(0, "stale".into()));
            shell.clipboard_revoke(&owner(1));
            assert!(
                shell
                    .plugin
                    .clipboard
                    .history
                    .get(&owner(2), &stored.handle, now)
                    .is_some()
            );
            assert!(shell.plugin.clipboard.target.is_some());
            shell.clipboard_revoke(&owner(2));
            assert!(
                shell
                    .plugin
                    .clipboard
                    .history
                    .get(&owner(2), &stored.handle, now)
                    .is_none()
            );
            assert!(shell.plugin.clipboard.target.is_none());
        }
        // Collection backpressure fails closed and restore requires a gesture.
        {
            let root = tempfile::tempdir().unwrap();
            let (mut shell, mut replies) = shell(root.path());
            let authority = Arc::new(AtomicBool::new(true));
            let now = Instant::now();
            let stored = shell
                .plugin
                .clipboard
                .history
                .insert(owner(1), std::sync::Arc::clone(&authority), content(), now)
                .unwrap();
            let (tx, mut commands) = mpsc::channel(1);
            shell.plugin.clipboard.channel = Some(tx.clone());
            let _ = shell.clipboard_restore(owner(1), 1, &stored.handle, "invalid", authority);
            assert!(commands.try_recv().is_err());
            let PluginCommand::Reply { result, .. } = replies.try_recv().unwrap() else {
                panic!("Expected failed restore")
            };
            assert_eq!(result["ok"], false);
            assert!(tx.try_send(Command::Collect(true)).is_ok());
            shell.clipboard_collect();
            assert!(shell.plugin.clipboard.target.is_none());
            assert!(shell.plugin.clipboard.channel.is_none());
            assert!(shell.plugin.clipboard.error.is_some());
            assert!(
                shell
                    .plugin
                    .clipboard
                    .history
                    .get(&owner(1), &stored.handle, now)
                    .is_none()
            );
        }
    }
}
