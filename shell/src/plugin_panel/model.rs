use enzui_shell::{
    bar_layout::Section,
    compositor::ControlGeometry,
    plugins::{
        manifest::Manifest,
        protocol::{Panel, Snapshot},
        service::{Command, Event, Owner},
        theme::Colors,
        view::Interaction,
    },
};
use iced::window;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

pub(crate) struct Live {
    pub owner: Owner,
    pub manifest: Arc<Manifest>,
    pub snapshot: Arc<Snapshot>,
    pub(crate) authority: Arc<AtomicBool>,
}
pub(crate) struct Pane {
    pub window: window::Id,
    pub(super) owner: Owner,
    pub(super) element: String,
    pub(super) manifest: Arc<Manifest>,
    pub(super) view: Arc<Panel>,
    pub(super) revision: u64,
    pub(super) geometry: ControlGeometry,
    pub(super) visible: bool,
    pub(super) started: bool,
    pub(super) anchor: iced::Rectangle,
    pub(super) applied_body: iced::Size,
    pub(super) actual_size: iced::Size,
    pub(super) resizing: bool,
    pub(super) layout_epoch: u64,
    pub(super) scroll: enzui_widgets::scroll_view::ScrollTranslation,
    pub(super) geometry_error: Option<String>,
}
#[derive(Clone)]
pub(super) struct Target {
    pub(super) owner: Owner,
    pub(super) element: String,
    pub(super) placement: Option<(iced::Rectangle, iced::Size)>,
}
#[derive(Clone)]
pub(super) struct LayoutRequest {
    pub(super) owner: Owner,
    pub(super) id: u64,
    pub(super) element: String,
    pub(super) section: Section,
    pub(super) before: Option<String>,
    pub(super) enabled: Option<bool>,
    pub(super) authority: Arc<AtomicBool>,
}
pub(super) struct ThemeClaim {
    pub(super) owner: Owner,
    pub(super) colors: Colors,
    pub(super) authority: Arc<AtomicBool>,
}
#[derive(Default)]
pub(crate) struct State {
    pub live: BTreeMap<String, Live>,
    pub channel: Option<tokio::sync::mpsc::Sender<Command>>,
    pub error: Option<String>,
    pub pane: Option<Pane>,
    pub clipboard: crate::plugin_clipboard::State,
    pub notifications: crate::plugin_notifications::State,
    pub(super) pending: Option<Target>,
    pub(super) preparing: Option<(u64, Target)>,
    pub(super) retiring: Option<window::Id>,
    pub(super) reflow: Option<window::Id>,
    pub(super) placement_busy: Option<u64>,
    pub(super) serial: u64,
    pub(super) gestures: BTreeMap<String, (Owner, Instant)>,
    pub(super) layout_busy: bool,
    pub(super) layout_dirty: bool,
    pub(super) catalog_serial: Option<u64>,
    pub(super) layout_request_busy: bool,
    pub(super) layout_requests: VecDeque<LayoutRequest>,
    pub(super) theme: Option<ThemeClaim>,
}
#[derive(Clone)]
pub(crate) enum Message {
    Event(Event),
    Clipboard(crate::plugin_clipboard::Message),
    NotificationTick,
    Toggle(String),
    Anchor(u64, Option<iced::Rectangle>),
    Prepared(u64, Result<ControlGeometry, String>),
    Reflow(
        window::Id,
        Owner,
        iced::Size,
        Result<ControlGeometry, String>,
    ),
    Inspect(window::Id),
    Close,
    Hidden(window::Id),
    Input {
        owner: Owner,
        element: String,
        revision: u64,
        interaction: Interaction,
    },
    LayoutSaved(Result<(), String>),
    LayoutRequestSaved {
        owner: Owner,
        id: u64,
        result: Result<(), String>,
    },
}
impl State {}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginPanelMessage")
    }
}
