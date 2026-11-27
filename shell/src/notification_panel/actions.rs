use enzui_shell::notifications::{activation::Target, model::Key};
use iced::window;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(super) enum Intent {
    Invoke(String),
    Activate(Option<Target>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    serial: u64,
    run: u64,
    window: window::Id,
    pub(super) key: Key,
}
struct Waiting {
    ticket: Ticket,
    intent: Intent,
    live: Arc<AtomicBool>,
    dispatched: bool,
}
#[derive(Default)]
pub(super) struct Actions {
    serial: u64,
    waiting: BTreeMap<Key, Waiting>,
}
impl Actions {
    pub(super) fn waiting(&self, key: &Key) -> bool {
        self.waiting.contains_key(key)
    }
    pub(super) fn begin(
        &mut self,
        run: u64,
        window: window::Id,
        key: Key,
        action: String,
    ) -> Option<Ticket> {
        self.begin_intent(run, window, key, Intent::Invoke(action))
    }
    pub(super) fn begin_intent(
        &mut self,
        run: u64,
        window: window::Id,
        key: Key,
        intent: Intent,
    ) -> Option<Ticket> {
        if self.waiting(&key) || self.waiting.len() >= enzui_shell::notifications::model::MAX_ACTIVE
        {
            return None;
        }
        self.serial = self.serial.checked_add(1)?;
        let ticket = Ticket {
            serial: self.serial,
            run,
            window,
            key,
        };
        self.waiting.insert(
            key,
            Waiting {
                ticket,
                intent,
                live: Arc::new(AtomicBool::new(true)),
                dispatched: false,
            },
        );
        Some(ticket)
    }
    pub(super) fn finish(
        &mut self,
        ticket: Ticket,
        run: u64,
        source: Option<window::Id>,
    ) -> Option<(Key, String)> {
        if self
            .waiting
            .get(&ticket.key)
            .is_none_or(|current| current.ticket != ticket)
        {
            return None;
        }
        let waiting = self.waiting.remove(&ticket.key)?;
        waiting.live.store(false, Ordering::Release);
        let Intent::Invoke(action) = waiting.intent else {
            return None;
        };
        (run == ticket.run && source == Some(ticket.window)).then_some((ticket.key, action))
    }
    pub(super) fn intent(&self, ticket: Ticket) -> Option<&Intent> {
        self.waiting
            .get(&ticket.key)
            .filter(|waiting| waiting.ticket == ticket && !waiting.dispatched)
            .map(|waiting| &waiting.intent)
    }
    pub(super) fn dispatch_activation(
        &mut self,
        ticket: Ticket,
        run: u64,
        source: Option<window::Id>,
    ) -> Option<(Option<Target>, Arc<AtomicBool>)> {
        if self
            .waiting
            .get(&ticket.key)
            .is_none_or(|waiting| waiting.ticket != ticket || waiting.dispatched)
        {
            return None;
        }
        if run != ticket.run || source != Some(ticket.window) {
            self.retire(ticket);
            return None;
        }
        let waiting = self.waiting.get_mut(&ticket.key)?;
        let Intent::Activate(target) = &waiting.intent else {
            return None;
        };
        waiting.dispatched = true;
        Some((target.clone(), std::sync::Arc::clone(&waiting.live)))
    }
    pub(super) fn completed(&mut self, ticket: Ticket) -> bool {
        if self
            .waiting
            .get(&ticket.key)
            .is_none_or(|waiting| waiting.ticket != ticket || !waiting.dispatched)
        {
            return false;
        }
        self.retire(ticket);
        true
    }
    pub(super) fn retire(&mut self, ticket: Ticket) {
        if self
            .waiting
            .get(&ticket.key)
            .is_some_and(|waiting| waiting.ticket == ticket)
            && let Some(waiting) = self.waiting.remove(&ticket.key)
        {
            waiting.live.store(false, Ordering::Release);
        }
    }
    pub(super) fn retain_keys(&mut self, keys: &[Key]) {
        self.waiting.retain(|key, waiting| {
            let keep = keys.contains(key);
            if !keep {
                waiting.live.store(false, Ordering::Release);
            }
            keep
        });
    }
    pub(super) fn cancel(&mut self) {
        for waiting in self.waiting.values() {
            waiting.live.store(false, Ordering::Release);
        }
        self.waiting.clear();
    }
}
impl Drop for Actions {
    fn drop(&mut self) {
        self.cancel();
    }
}
