use crate::{Message as ShellMessage, Shell, plugin_panel};
use enzui_shell::i18n::tr;
use enzui_shell::{
    notifications::server::Snapshot,
    plugins::{
        budget::MAX_WORKERS,
        notifications::Feed,
        service::{Command, Owner},
    },
};
use iced::{Subscription, Task};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const BUSY: &str = "Notification feed is busy; latest state is pending";

struct Watcher {
    owner: Owner,
    authority: Arc<AtomicBool>,
    feed: Feed,
    sent: Option<Value>,
}
#[derive(Default)]
pub(super) struct State {
    watchers: BTreeMap<String, Watcher>,
}
impl State {
    pub(super) fn clear(&mut self) {
        self.watchers.clear();
    }
    pub(super) fn revoke(&mut self, owner: &Owner) {
        if self
            .watchers
            .get(&owner.id)
            .is_some_and(|watcher| watcher.owner == *owner)
        {
            self.watchers.remove(&owner.id);
        }
    }
    fn prune(&mut self) {
        self.watchers
            .retain(|_, watcher| watcher.authority.load(Ordering::Acquire));
    }
    fn subscribe(
        &mut self,
        owner: Owner,
        authority: Arc<AtomicBool>,
        enabled: bool,
    ) -> Result<(), String> {
        self.prune();
        if !authority.load(Ordering::Acquire) {
            return Err(tr("shell-notification-feed-authority-changed").into());
        }
        if self
            .watchers
            .get(&owner.id)
            .is_some_and(|watcher| watcher.owner != owner)
        {
            return Err(tr("shell-notification-feed-owner-changed").into());
        }
        if !enabled {
            self.revoke(&owner);
            return Ok(());
        }
        if let Some(watcher) = self.watchers.get_mut(&owner.id) {
            watcher.sent = None;
            return Ok(());
        }
        if self.watchers.len() >= MAX_WORKERS {
            return Err(tr("shell-notification-feed-is-busy").into());
        }
        self.watchers.insert(
            owner.id.clone(),
            Watcher {
                owner,
                authority,
                feed: Feed::new()?,
                sent: None,
            },
        );
        Ok(())
    }
    fn publish(
        &mut self,
        snapshot: Option<&Snapshot>,
        channel: &tokio::sync::mpsc::Sender<Command>,
    ) -> Result<(), String> {
        self.prune();
        let mut failed = Vec::new();
        let mut error = None;
        for watcher in self.watchers.values_mut() {
            let data = match watcher.feed.frame(snapshot) {
                Ok(data) => data,
                Err(reason) => {
                    failed.push(watcher.owner.clone());
                    error = Some(reason);
                    continue;
                }
            };
            if watcher.sent.as_ref() == Some(&data) || !watcher.authority.load(Ordering::Acquire) {
                continue;
            }
            if channel
                .try_send(Command::Provider {
                    owner: watcher.owner.clone(),
                    data: data.clone(),
                })
                .is_ok()
            {
                watcher.sent = Some(data);
            } else {
                error = Some(BUSY.into());
            }
        }
        for owner in failed {
            self.revoke(&owner);
        }
        error.map_or(Ok(()), Err)
    }
}
impl Shell {
    pub(super) fn plugin_notification_subscription(&self) -> Subscription<ShellMessage> {
        if self.plugin.notifications.watchers.is_empty() {
            return Subscription::none();
        }
        iced::time::every(Duration::from_millis(500))
            .map(|_| ShellMessage::Plugin(plugin_panel::Message::NotificationTick))
    }
    pub(super) fn plugin_notifications_publish(&mut self) -> Task<ShellMessage> {
        self.plugin.notifications.prune();
        if let Some(channel) = &self.plugin.channel {
            match self
                .plugin
                .notifications
                .publish(self.notification.snapshot(), channel)
            {
                Err(error) => self.plugin.error = Some(error),
                Ok(()) if self.plugin.error.as_deref() == Some(BUSY) => self.plugin.error = None,
                Ok(()) => (),
            }
        }
        Task::none()
    }
    pub(super) fn plugin_notification_subscribe(
        &mut self,
        owner: Owner,
        id: u64,
        enabled: bool,
        authority: Arc<AtomicBool>,
    ) -> Task<ShellMessage> {
        match self
            .plugin
            .notifications
            .subscribe(owner.clone(), authority, enabled)
        {
            Ok(()) => {
                self.plugin_reply(owner.clone(), id, true, "");
                if enabled {
                    self.plugin_notifications_publish()
                } else {
                    self.plugin_command(Command::Provider {
                        owner,
                        data: json!({"provider":"notifications","event":"paused","items":[]}),
                    });
                    Task::none()
                }
            }
            Err(error) => {
                self.plugin_reply(owner, id, false, &error);
                Task::none()
            }
        }
    }
}
