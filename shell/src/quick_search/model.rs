use crate::search_clipboard;
use enzui_shell::i18n::tr;
use enzui_shell::{
    search::{
        Category, Inventory, Item, Model, Ticket,
        compositor::{IslandPlacement, SearchGeometry, Windows},
        files,
        preferences::{Change, Preferences},
    },
    settings::Change as SettingChange,
};
use iced::window;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) struct HeightMotion {
    pub(super) from: f32,
    pub(super) to: f32,
    pub(super) started: Instant,
    pub(super) duration: Duration,
}
impl HeightMotion {
    pub(super) fn height(self, now: Instant) -> f32 {
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / self.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        (self.from + (self.to - self.from) * progress * progress * (3.0 - 2.0 * progress)).round()
    }
    pub(super) fn finished(self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= self.duration
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileKey {
    pub(super) revision: u64,
    pub(super) roots: Vec<String>,
    pub(super) exclusions: Vec<String>,
}
#[derive(Default)]
pub(crate) struct State {
    pub window: Option<window::Id>,
    pub showing: bool,
    /// Shown as the overview's search: centred under the bar, over the
    /// overview's own backdrop rather than dimming the screen.
    pub overview: bool,
    pub visible: bool,
    pub started: bool,
    pub configured: bool,
    pub model: Model,
    pub preferences: Preferences,
    pub(super) browse_all_apps: bool,
    /// The recommended apps, kept above the full list while it is open.
    pub(super) top_apps: Vec<enzui_shell::search::Item>,
    pub(super) app_scroll_carry: f32,
    pub(super) epoch: u64,
    pub(super) focus_revision: u64,
    pub(super) source_revision: u64,
    pub(super) preparing: bool,
    pub(super) presentation: Option<Arc<AtomicBool>>,
    pub(super) geometry: Option<(iced::Size, String)>,
    pub(super) island: Option<IslandPlacement>,
    pub(super) presented: bool,
    pub(super) surface_size: Option<iced::Size>,
    pub(super) height_motion: Option<HeightMotion>,
    pub(super) height_hold: Option<f32>,
    pub(super) height_due: Option<Instant>,
    pub(super) height_epoch: u64,
    pub(super) pending_browse_collapse: bool,
    pub(super) layout_busy: bool,
    pub(super) layout_revision: u64,
    pub(super) preedit: Option<String>,
    pub feedback: Option<String>,
    pub(super) pending_preference: Option<Change>,
    pub(super) pending_desktop: Option<SettingChange>,
    pub(super) apps: Inventory,
    pub(super) apps_ready: bool,
    pub(super) apps_busy: bool,
    pub(super) apps_generation: u64,
    pub(super) apps_cancel: Option<Arc<AtomicBool>>,
    pub(super) windows: Vec<Item>,
    pub(super) windows_busy: bool,
    pub(super) file_wanted: Option<FileKey>,
    pub(super) file_revision: u64,
    pub(super) file_built: Option<FileKey>,
    pub(super) file_busy: bool,
    pub(super) file_cancel: Option<Arc<AtomicBool>>,
    pub(super) file_index: Option<files::Index>,
    pub(super) file_query_busy: bool,
    pub(super) file_query: Option<(String, Option<Category>)>,
    pub(super) file_results: Vec<Item>,
    pub(super) file_error: Option<String>,
    pub clipboard: search_clipboard::State,
    pub(super) action_busy: bool,
    pub(super) copy_epoch: Option<u64>,
    pub(super) alias: Option<(String, String)>,
    pub(super) alias_draft: String,
    pub(super) last_query_us: u128,
    pub(super) context: Option<(bool, bool, bool)>,
    pub(super) providers: Vec<super::providers::Provider>,
    pub(super) provider_results: Vec<Item>,
    // Raised for every query; answers carry the value they were asked with.
    pub(super) provider_generation: Arc<std::sync::atomic::AtomicU64>,
    pub(super) provider_strikes: std::collections::HashMap<String, u8>,
    // Track attempted provider activation for the current Search opening.
    pub(super) activation: Option<Arc<enzui_shell::background_apps::Client>>,
    pub(super) activation_checked: bool,
    pub(super) activation_tried: std::collections::HashSet<String>,
    pub(super) last_press: Option<Instant>,
    /// The last press in the bar, which holds Search's trigger.
    pub(super) bar_press: Option<Instant>,
    /// The bar press the outside check closed Search for; the click it
    /// belongs to ends there instead of opening Search again.
    pub(super) closed_by: Option<Instant>,
}
impl State {
    /// Whether the blank card offers apps. The overview's card keeps to the
    /// field, leaving the windows their room.
    pub(crate) fn blank_apps(&self) -> bool {
        self.preferences.shows(enzui_shell::search::Category::Apps) && !self.overview
    }
    /// The transition the shown card was presented under.
    pub(crate) fn shown_epoch(&self) -> Option<u64> {
        self.showing.then_some(self.epoch)
    }
}
#[derive(Clone)]
pub(crate) enum Message {
    Show(Option<Category>),
    Toggle,
    Close,
    Dismiss,
    FocusInput,
    FocusChecked(u64, u64, bool),
    Hidden(window::Id, u64),
    DimHidden(u64, Result<(), String>),
    Parked(u64, Result<(), String>),
    Anchor(u64, Option<iced::Rectangle>),
    Prepared(u64, Result<SearchGeometry, String>),
    Presented(u64, Result<(), String>),
    Resized(u64, u64, Result<(), String>),
    Query(String),
    Preedit {
        window: window::Id,
        epoch: u64,
        alias: Option<String>,
        text: Option<String>,
    },
    Category(Option<Category>),
    BrowseApps(bool),
    ScrollApps(f32),
    Navigate(isize),
    Activate(Ticket),
    Key(
        window::Id,
        u64,
        iced::keyboard::Key,
        iced::keyboard::Modifiers,
        bool,
    ),
    Apps(u64, Result<Inventory, String>),
    Windows(u64, Result<Windows, String>),
    Indexed(FileKey, Result<files::Index, String>),
    FileQuery(FileKey, String, Option<Category>, Result<Vec<Item>, String>),
    Provided(
        u64,
        String,
        String,
        Option<
            Result<
                Vec<enzui_shell::search::providers::Found>,
                enzui_shell::search::providers::Silence,
            >,
        >,
    ),
    /// Whether the OS can start providers for search.
    Activation(u64, Option<Arc<enzui_shell::background_apps::Client>>),
    /// A provider the OS was asked to start for this query, and whether it did.
    Activated(u64, String, String, bool),
    ProviderToken(
        u64,
        window::Id,
        Ticket,
        enzui_shell::search::Target,
        Option<String>,
    ),
    /// A press inside Search's own window.
    Pressed,
    /// A press in the bar's window.
    BarPressed,
    /// Whether the press a compositor command reported landed in Search.
    OutsideCheck(u64, Instant),
    ClipboardTarget(u64, Result<enzui_shell::clipboard::session::Target, String>),
    Clipboard(search_clipboard::Message),
    Done(u64, Result<(), String>),
    Copied(u64, bool),
    Refresh,
    Favorite(Ticket),
    Alias(Ticket),
    AliasDraft(String),
    AliasSave,
    AliasCancel,
    Settings,
    Tick,
    HeightFrame,
    HeightDue(u64),
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(tr("shell-quick-search-event"))
    }
}
