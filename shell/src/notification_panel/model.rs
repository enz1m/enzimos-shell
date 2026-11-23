use enzui_shell::{
    compositor::ControlGeometry,
    notifications::{
        model::Key,
        preferences::Preferences,
        presentation::Arrivals,
        runtime,
        server::{Command, Snapshot},
    },
};
use iced::window;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) struct Reply {
    pub(super) key: Key,
    pub(super) text: String,
}
pub(super) struct Pane {
    pub(super) window: window::Id,
    pub(super) geometry: ControlGeometry,
    pub(super) visible: bool,
    pub(super) started: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Content {
    #[default]
    Notifications,
    Calendar,
    Activity,
}
#[derive(Default)]
pub(crate) struct State {
    pub(super) run: u64,
    pub(super) epoch: Option<u64>,
    pub(super) channel: Option<tokio::sync::mpsc::Sender<Command>>,
    pub(super) authority: Option<Arc<AtomicBool>>,
    pub(super) snapshot: Option<Arc<Snapshot>>,
    pub(super) arrivals: Arrivals,
    pub(super) center: Option<Pane>,
    pub(super) toast: Option<Pane>,
    pub(super) center_serial: u64,
    pub(super) toast_serial: u64,
    pub(super) center_opening: bool,
    // The body height the open center was last placed at.
    pub(super) center_requested: f32,
    pub(super) toast_opening: bool,
    // While a screen capture is being framed toasts stay away, so the frozen
    // screen (and the picture taken from it) does not carry one.
    pub(crate) toasts_paused: bool,
    pub(super) toast_dirty: bool,
    pub(super) toast_count: usize,
    // Retain outgoing toast rows until their slide and collapse finish before shrinking the
    // window.
    pub(super) toast_shown: Vec<Arc<enzui_shell::notifications::model::Notification>>,
    pub(super) toast_leaving: BTreeMap<(u64, u32), std::time::Instant>,
    pub(super) toast_requested: usize,
    pub(super) toast_overshoot: f32,
    // Card heights the open toast window was sized for.
    pub(super) toast_heights: Vec<f32>,
    // Track reveal timing by notification epoch and ID so replacement does not replay arrival.
    pub(super) revealed: std::collections::BTreeMap<(u64, u32), (std::time::Instant, bool)>,
    pub(super) reveal_scheduled: std::collections::BTreeSet<(u64, u32)>,
    pub(super) reveal_last: Option<std::time::Instant>,
    pub(super) toast_hovered: bool,
    pub(super) toast_focused: bool,
    pub(super) held_keys: Vec<Key>,
    pub(super) error: Option<String>,
    pub(super) storage_error: Option<String>,
    pub(super) shortcut_error: Option<String>,
    pub(super) reply: Option<Reply>,
    pub(super) pending: BTreeMap<Key, bool>,
    pub(super) actions: super::actions::Actions,
    pub(super) history: bool,
    pub(super) history_query: String,
    pub(super) page: usize,
    pub(super) expanded: Option<Key>,
    pub(super) expanded_history: Option<u64>,
    pub(super) focus_reply: bool,
    pub(super) preferences: Option<Arc<Preferences>>,
    pub(super) effective_policy: Option<enzui_shell::notifications::model::Policy>,
    pub(super) content: Content,
    pub(super) activity: ActivityPreview,
}
#[derive(Default)]
pub(super) struct ActivityPreview {
    pub(super) trigger_hovered: bool,
    pub(super) panel_hovered: bool,
    pub(super) trigger_bounds: Option<iced::Rectangle>,
    pub(super) passive: bool,
    pub(super) revision: u64,
}

impl ActivityPreview {
    pub(super) fn reset_popup(&mut self) {
        self.passive = false;
        self.panel_hovered = false;
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(authority) = &self.authority {
            authority.store(false, Ordering::Release);
        }
    }
}
#[derive(Clone)]
pub(crate) enum Message {
    Runtime(runtime::Event),
    Shortcut(enzui_shell::shortcuts::Event),
    ShortcutBound(Result<(), String>),
    CenterFocused(u64, u8, Result<(), String>),
    RetryActivityFocus(u64, u8),
    Toggle,
    /// The clock's calendar, events and world clocks.
    ToggleCalendar,
    /// Current background work, without saved notification history.
    ToggleActivity,
    ActivityTriggerHover(bool),
    ActivityHoverElapsed(u64),
    ActivityPointerChecked(u64, bool),
    Hidden(window::Id),
    Anchor(u64, Option<iced::Rectangle>),
    CenterPrepared(u64, Result<ControlGeometry, String>),
    ToastPrepared(u64, usize, f32, Result<ControlGeometry, String>),
    PopupHover(window::Id, bool),
    Reveal((u64, u32)),
    /// Leaving toasts have finished; the window can take its new size.
    LeftToasts,
    SlideFrame,
    RenewToasts,
    Dnd(bool),
    Retry,
    History(bool),
    HistoryQuery(String),
    HistoricalDetails(u64),
    Page(usize),
    Expand(Key),
    Reply(Key),
    EditReply(Key, String),
    SendReply(Key),
    DiscardReply,
    Invoke(Key, String),
    Dismiss(Key),
    /// Dismiss every current notification (each still goes to history).
    DismissAll,
    /// Hide every shown toast; they stay in the notification centre.
    HideToasts,
    InvokeWith(super::actions::Ticket, Token),
    /// A click on a message: its default action, or the app that sent it.
    Activate(Key),
    Activated(super::actions::Ticket, Result<(), String>),
    ClearHistory,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotificationPanelMessage")
    }
}

// Activation tokens must never enter logs.
#[derive(Clone)]
pub(crate) struct Token(pub(crate) Option<String>);
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token")
    }
}
