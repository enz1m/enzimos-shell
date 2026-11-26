use super::{
    origin::Origin,
    payload::{Draft, Image, Urgency},
    toast::Toast,
};
use enzui_shell::i18n::tr;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_ACTIVE: usize = 64;
pub const MAX_PER_APP: usize = 16;
pub const MAX_HISTORY: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Allow,
    Block,
    Priority,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub dnd: bool,
    pub allow_critical: bool,
    pub hold_fullscreen: bool,
    pub toast_ms: u32,
    pub rules: BTreeMap<String, Rule>,
    pub history: bool,
    pub history_generation: u64,
    pub history_minutes: u32,
    pub history_mib: u32,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            dnd: false,
            allow_critical: false,
            hold_fullscreen: true,
            toast_ms: 5000,
            rules: BTreeMap::new(),
            history: false,
            history_generation: 0,
            history_minutes: 7 * 24 * 60,
            history_mib: 8,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if !(1000..=30000).contains(&self.toast_ms)
            || !(1..=30 * 24 * 60).contains(&self.history_minutes)
            || !(1..=16).contains(&self.history_mib)
            || self.rules.len() > 128
            || self.rules.keys().any(|key| {
                !key.strip_prefix("exe:").is_some_and(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                })
            })
        {
            return Err(tr("shell-invalid-notification-policy").into());
        }
        Ok(())
    }
    pub fn rule(&self, principal: &str) -> Rule {
        self.rules.get(principal).copied().unwrap_or(Rule::Allow)
    }
    pub fn toast(&self, principal: &str, urgency: Urgency) -> bool {
        let rule = self.rule(principal);
        rule != Rule::Block
            && (!self.dnd
                || rule == Rule::Priority
                || self.allow_critical && urgency == Urgency::Critical)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub epoch: u64,
    pub id: u32,
    pub revision: u64,
}
#[derive(Clone)]
pub struct Notification {
    pub key: Key,
    pub origin: Origin,
    pub draft: Draft,
    pub received_ms: u64,
    /// The arrival time on the local monotonic clock.
    pub received_at: Instant,
    pub updated_ms: u64,
    pub unread: bool,
    pub expires: Option<Instant>,
    pub toast: Toast,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Application = 3,
    Unspecified = 4,
}
#[derive(Clone)]
pub struct Closed {
    pub origin: Origin,
    pub id: u32,
    pub reason: CloseReason,
}
#[derive(Clone)]
pub struct Archived {
    pub id: u64,
    pub principal: String,
    pub source: String,
    pub summary: String,
    pub body: String,
    pub image: Option<Image>,
    pub received_ms: u64,
    pub closed_ms: u64,
    pub reason: CloseReason,
}
impl Archived {
    pub fn bytes(&self) -> usize {
        self.principal.len()
            + self.source.len()
            + self.summary.len()
            + self.body.len()
            + self
                .image
                .as_ref()
                .map_or(0, |image| image.pixels().0.len())
    }
}
pub struct Posted {
    pub id: u32,
    pub notification: Option<Arc<Notification>>,
    pub toast: bool,
    pub closed: Vec<Closed>,
}
#[derive(Clone)]
pub enum Action {
    Invoke(String),
    Reply(String),
}
pub struct Dispatch {
    pub origin: Origin,
    pub id: u32,
    pub action: Action,
}
pub struct Model {
    epoch: u64,
    next_id: u32,
    revision: u64,
    next_history: u64,
    history_initialized: bool,
    policy: Policy,
    active: BTreeMap<u32, Arc<Notification>>,
    history: VecDeque<Archived>,
    dirty: bool,
}
impl Model {
    pub fn new(epoch: u64, policy: Policy) -> Result<Self, String> {
        policy.validate()?;
        Ok(Self {
            epoch,
            next_id: 0,
            revision: 0,
            next_history: 0,
            history_initialized: false,
            policy,
            active: BTreeMap::new(),
            history: VecDeque::new(),
            dirty: true,
        })
    }
    pub(crate) fn restore_history(
        &mut self,
        records: Vec<Archived>,
        now_ms: u64,
    ) -> Result<(), String> {
        if self.history_initialized || !self.history.is_empty() || self.next_history != 0 {
            return Err(tr("shell-notification-history-was-already-initialized").into());
        }
        self.history_initialized = true;
        for mut record in super::archive::bounded(records, &self.policy, now_ms) {
            self.next_history = self
                .next_history
                .checked_add(1)
                .ok_or_else(|| tr("shell-notification-history-identities-exhausted"))?;
            record.id = self.next_history;
            self.history.push_back(record);
        }
        self.dirty = true;
        Ok(())
    }
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
    pub fn active(&self) -> impl Iterator<Item = &Arc<Notification>> {
        self.active.values()
    }
    pub fn history(&self) -> &VecDeque<Archived> {
        &self.history
    }
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
    pub fn get(&self, key: Key) -> Option<&Arc<Notification>> {
        self.active
            .get(&key.id)
            .filter(|notification| notification.key == key)
    }
    fn archive(&mut self, notification: &Notification, reason: CloseReason, wall_ms: u64) {
        if !self.policy.history || notification.draft.transient {
            return;
        }
        let Some(id) = self.next_history.checked_add(1) else {
            return;
        };
        self.next_history = id;
        self.history.push_front(Archived {
            id,
            principal: notification.origin.principal.clone(),
            source: notification.origin.label.clone(),
            summary: notification.draft.summary.clone(),
            body: notification.draft.body.clone(),
            image: notification.draft.image.clone(),
            received_ms: notification.received_ms,
            closed_ms: wall_ms,
            reason,
        });
        self.prune_history(wall_ms);
    }
    fn remove(
        &mut self,
        id: u32,
        reason: CloseReason,
        wall_ms: u64,
        archive: bool,
    ) -> Option<Closed> {
        let notification = self.active.remove(&id)?;
        self.dirty = true;
        if archive {
            self.archive(&notification, reason, wall_ms);
        }
        Some(Closed {
            origin: notification.origin.clone(),
            id,
            reason,
        })
    }
    pub fn post(
        &mut self,
        origin: Origin,
        draft: Draft,
        now: Instant,
        wall_ms: u64,
        fullscreen: bool,
    ) -> Result<Posted, String> {
        let previous = self.active.get(&draft.replaces);
        if previous.is_some_and(|old| !old.origin.same_sender(&origin)) {
            return Err(tr("shell-notification-belongs-to-another-sender").into());
        }
        let id = match previous {
            Some(old) => old.key.id,
            None => self
                .next_id
                .checked_add(1)
                .ok_or_else(|| tr("shell-notification-identities-exhausted"))?,
        };
        let received_ms = previous.map_or(wall_ms, |old| old.received_ms);
        let received_at = previous.map_or(now, |old| old.received_at);
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| tr("shell-notification-revisions-exhausted"))?;
        let mut closed = Vec::new();
        if previous.is_none() {
            self.next_id = id;
        }
        if self.policy.rule(&origin.principal) == Rule::Block {
            self.dirty |= self.active.remove(&id).is_some();
            closed.push(Closed {
                origin,
                id,
                reason: CloseReason::Dismissed,
            });
            return Ok(Posted {
                id,
                notification: None,
                toast: false,
                closed,
            });
        }
        if !self.active.contains_key(&id) {
            if self
                .active
                .values()
                .filter(|n| n.origin.principal == origin.principal)
                .count()
                >= MAX_PER_APP
            {
                let oldest = self
                    .active
                    .values()
                    .filter(|n| n.origin.principal == origin.principal)
                    .min_by_key(|n| n.key.revision)
                    .expect("the active set exceeds its nonzero capacity")
                    .key
                    .id;
                closed.extend(self.remove(oldest, CloseReason::Unspecified, wall_ms, true));
            }
            if self.active.len() >= MAX_ACTIVE {
                let oldest = self
                    .active
                    .values()
                    .min_by_key(|n| n.key.revision)
                    .expect("the active set exceeds its nonzero capacity")
                    .key
                    .id;
                closed.extend(self.remove(oldest, CloseReason::Unspecified, wall_ms, true));
            }
        }
        let timeout_ms = match draft.timeout_ms {
            0 => None,
            -1 if draft.transient => Some(self.policy.toast_ms),
            -1 => Some(24 * 60 * 60 * 1000),
            value => Some((value as u32).min(24 * 60 * 60 * 1000)),
        };
        let held = self.policy.hold_fullscreen && fullscreen && draft.urgency != Urgency::Critical;
        let toast = !held && self.policy.toast(&origin.principal, draft.urgency);
        self.revision = revision;
        self.dirty = true;
        let expires = timeout_ms.map(|ms| now + Duration::from_millis(ms.into()));
        let toast_deadline = now + Duration::from_millis(self.policy.toast_ms.into());
        let toast_until =
            toast.then(|| expires.map_or(toast_deadline, |at| at.min(toast_deadline)));
        let notification = Arc::new(Notification {
            key: Key {
                epoch: self.epoch,
                id,
                revision,
            },
            origin,
            draft,
            received_ms,
            received_at,
            updated_ms: wall_ms,
            unread: true,
            expires,
            toast: Toast::new(toast_until),
        });
        self.active.insert(id, std::sync::Arc::clone(&notification));
        Ok(Posted {
            id,
            notification: Some(notification),
            toast,
            closed,
        })
    }
    pub fn authorize(&self, key: Key, action: Action) -> Result<Dispatch, String> {
        self.authorize_at(key, action, Instant::now())
    }
    fn authorize_at(&self, key: Key, action: Action, now: Instant) -> Result<Dispatch, String> {
        let notification = self
            .get(key)
            .filter(|notification| notification.expires.is_none_or(|at| at > now))
            .ok_or_else(|| tr("shell-notification-changed-expired-or-closed"))?;
        if self.policy.rule(&notification.origin.principal) == Rule::Block {
            return Err(tr("shell-notification-app-is-blocked").into());
        }
        match &action {
            Action::Invoke(action) if notification.draft.permits_action(action) => (),
            Action::Reply(text)
                if notification.draft.reply.is_some()
                    && !text.trim().is_empty()
                    && text.len() <= 16384
                    && text.chars().count() <= 4096
                    && !text.contains('\0') => {}
            _ => {
                return Err(tr("shell-notification-action-is-unavailable-or-invalid").into());
            }
        }
        Ok(Dispatch {
            origin: notification.origin.clone(),
            id: key.id,
            action,
        })
    }
    pub fn action_delivered(&mut self, key: Key, wall_ms: u64) -> Option<Closed> {
        let notification = self.get(key)?;
        if notification.draft.resident {
            self.mark_read(key);
            None
        } else {
            self.remove(key.id, CloseReason::Dismissed, wall_ms, true)
        }
    }
    pub fn mark_read(&mut self, key: Key) {
        if self
            .get(key)
            .is_some_and(|notification| notification.unread)
            && let Some(notification) = self.active.get_mut(&key.id)
        {
            self.dirty = true;
            let notification = Arc::make_mut(notification);
            notification.unread = false;
            notification.toast.clear();
        }
    }
    pub fn hold_toasts(&mut self, keys: &[Key], now: Instant) {
        let keys: BTreeSet<_> = keys.iter().take(MAX_ACTIVE).copied().collect();
        for notification in self.active.values_mut() {
            let enabled = keys.contains(&notification.key)
                && notification.unread
                && notification.expires.is_none_or(|until| until > now);
            let mut toast = notification.toast;
            self.dirty |= toast.hold(enabled, now);
            if toast != notification.toast {
                Arc::make_mut(notification).toast = toast;
            }
        }
    }
    pub fn dismiss(&mut self, key: Key, wall_ms: u64) -> Option<Closed> {
        self.get(key)?;
        self.remove(key.id, CloseReason::Dismissed, wall_ms, true)
    }
    pub fn close_from_app(
        &mut self,
        origin: &Origin,
        id: u32,
        wall_ms: u64,
    ) -> Result<Option<Closed>, String> {
        if self
            .active
            .get(&id)
            .is_some_and(|notification| !notification.origin.same_sender(origin))
        {
            return Err(tr("shell-notification-belongs-to-another-sender").into());
        }
        Ok(self.remove(id, CloseReason::Application, wall_ms, true))
    }
    /// Whether a notification is showing, so expiry and toast motion need ticks.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
    pub fn expire(&mut self, now: Instant, wall_ms: u64) -> Vec<Closed> {
        let expired: Vec<_> = self
            .active
            .values()
            .filter(|notification| notification.expires.is_some_and(|at| at <= now))
            .map(|notification| notification.key.id)
            .collect();
        let closed = expired
            .into_iter()
            .filter_map(|id| self.remove(id, CloseReason::Expired, wall_ms, true))
            .collect();
        for notification in self.active.values_mut() {
            let mut toast = notification.toast;
            if toast.advance(now) {
                Arc::make_mut(notification).toast = toast;
                self.dirty = true;
            }
        }
        self.prune_history(wall_ms);
        closed
    }
    pub fn set_policy(&mut self, policy: Policy, wall_ms: u64) -> Result<Vec<Closed>, String> {
        policy.validate()?;
        if policy == self.policy {
            return Ok(Vec::new());
        }
        self.dirty = true;
        if policy.history_generation != self.policy.history_generation || !policy.history {
            self.history.clear();
        }
        self.policy = policy;
        let blocked: Vec<_> = self
            .active
            .values()
            .filter(|notification| self.policy.rule(&notification.origin.principal) == Rule::Block)
            .map(|notification| notification.key.id)
            .collect();
        let closed = blocked
            .into_iter()
            .filter_map(|id| self.remove(id, CloseReason::Dismissed, wall_ms, false))
            .collect();
        for notification in self.active.values_mut() {
            if !self
                .policy
                .toast(&notification.origin.principal, notification.draft.urgency)
            {
                Arc::make_mut(notification).toast.clear();
            }
        }
        self.prune_history(wall_ms);
        Ok(closed)
    }
    pub fn clear_history(&mut self) {
        self.dirty |= !self.history.is_empty();
        self.history.clear();
    }
    fn prune_history(&mut self, wall_ms: u64) {
        let before = self.history.len();
        self.history.retain(|entry| {
            super::archive::within_retention(entry.closed_ms, wall_ms, self.policy.history_minutes)
        });
        let mut bytes: usize = self.history.iter().map(Archived::bytes).sum();
        while self.history.len() > MAX_HISTORY
            || bytes > self.policy.history_mib as usize * 1024 * 1024
        {
            if let Some(entry) = self.history.pop_back() {
                bytes -= entry.bytes();
            } else {
                break;
            }
        }
        self.dirty |= self.history.len() != before;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn origin(number: u32) -> Origin {
        Origin {
            sender: format!(":1.{number}"),
            principal: format!("exe:{number:064x}"),
            label: "Fixture".into(),
            icon: "bell".into(),
            app_icon: None,
            desktop: None,
            pid: number,
            start: 1,
            device: 1,
            inode: 1,
            portal: false,
        }
    }
    fn draft() -> Draft {
        Draft::parse((
            "Claimed fixture".into(),
            0,
            String::new(),
            "Fixture".into(),
            "Body".into(),
            vec![
                "open".into(),
                "Open".into(),
                "inline-reply".into(),
                "Reply".into(),
            ],
            Default::default(),
            -1,
        ))
        .unwrap()
    }
    #[test]
    fn model_behaviour() {
        // Replacements and close requests are bound to sender and revision.
        {
            let now = Instant::now();
            let mut model = Model::new(1, Policy::default()).unwrap();
            let first = model
                .post(origin(1), draft(), now, 1, false)
                .unwrap()
                .notification
                .unwrap();
            let mut replacement = draft();
            replacement.replaces = first.key.id;
            assert!(
                model
                    .post(origin(2), replacement.clone(), now, 2, false)
                    .is_err()
            );
            assert!(model.close_from_app(&origin(2), first.key.id, 2).is_err());
            let second = model
                .post(origin(1), replacement, now, 2, false)
                .unwrap()
                .notification
                .unwrap();
            assert_eq!(first.key.id, second.key.id);
            assert_ne!(first.key.revision, second.key.revision);
            assert!(
                model
                    .authorize(first.key, Action::Invoke("open".into()))
                    .is_err()
            );
            assert!(
                model
                    .authorize(second.key, Action::Invoke("open".into()))
                    .is_ok()
            );
            assert!(
                model
                    .authorize(
                        Key {
                            epoch: 2,
                            ..second.key
                        },
                        Action::Invoke("open".into())
                    )
                    .is_err()
            );
        }
        // Dnd priority and block rules are not based on claimed names.
        {
            let now = Instant::now();
            let policy = Policy {
                dnd: true,
                ..Policy::default()
            };
            let mut model = Model::new(1, policy.clone()).unwrap();
            let normal = model.post(origin(1), draft(), now, 1, false).unwrap();
            assert!(!normal.toast);
            assert!(normal.notification.is_some());
            let mut changed = policy;
            changed.rules.insert(origin(1).principal, Rule::Priority);
            model.set_policy(changed.clone(), 2).unwrap();
            assert!(model.post(origin(1), draft(), now, 2, false).unwrap().toast);
            assert!(!model.post(origin(2), draft(), now, 2, false).unwrap().toast);
            changed.rules.insert(origin(1).principal, Rule::Block);
            changed.allow_critical = true;
            model.set_policy(changed, 3).unwrap();
            let mut critical = draft();
            critical.urgency = Urgency::Critical;
            assert!(
                model
                    .post(origin(1), critical.clone(), now, 3, false)
                    .unwrap()
                    .notification
                    .is_none()
            );
            assert!(
                model
                    .post(origin(2), critical.clone(), now, 3, true)
                    .unwrap()
                    .toast
            );

            let mut changed = Policy::default();
            changed.rules.insert(origin(2).principal, Rule::Priority);
            model.set_policy(changed.clone(), 4).unwrap();
            let held = model.post(origin(2), draft(), now, 4, true).unwrap();
            assert!(!held.toast);
            assert!(held.notification.unwrap().unread);
            assert!(model.post(origin(2), critical, now, 5, true).unwrap().toast);
            changed.hold_fullscreen = false;
            model.set_policy(changed, 6).unwrap();
            assert!(model.post(origin(2), draft(), now, 6, true).unwrap().toast);
        }
        // History is opt in transient aware bounded and generation cleared.
        {
            let now = Instant::now();
            let mut model = Model::new(1, Policy::default()).unwrap();
            let key = model
                .post(origin(1), draft(), now, 1, false)
                .unwrap()
                .notification
                .unwrap()
                .key;
            model.dismiss(key, 2);
            assert!(model.history.is_empty());
            let policy = Policy {
                history: true,
                history_minutes: 1,
                ..Policy::default()
            };
            model.set_policy(policy.clone(), 2).unwrap();
            let mut transient = draft();
            transient.transient = true;
            let key = model
                .post(origin(1), transient, now, 2, false)
                .unwrap()
                .notification
                .unwrap()
                .key;
            model.dismiss(key, 3);
            assert!(model.history.is_empty());
            for _ in 0..300 {
                let key = model
                    .post(origin(1), draft(), now, 3, false)
                    .unwrap()
                    .notification
                    .unwrap()
                    .key;
                model.dismiss(key, 4);
            }
            assert_eq!(model.history.len(), MAX_HISTORY);
            model.expire(now, 60004);
            assert!(model.history.is_empty());
            let key = model
                .post(origin(1), draft(), now, 60005, false)
                .unwrap()
                .notification
                .unwrap()
                .key;
            model.dismiss(key, 60006);
            model
                .set_policy(
                    Policy {
                        history_generation: 1,
                        ..policy
                    },
                    60006,
                )
                .unwrap();
            assert!(model.history.is_empty());
        }
    }
}
