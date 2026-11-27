use crate::{
    Message, Shell, control_center::TileMenu, dock_motion, resize_chrome, window_settings,
};
use enzui_core::{
    context_menu::InvocationKind,
    menu_model::{CheckState, EntryId, MenuEntryModel, MenuModel},
};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    dock::{
        self, Action, App, Builtin, Change, Geometry, Group, Indicator, Material, Position,
        Snapshot, Style,
    },
    search::{AppTarget, Target, WindowTarget},
    settings::QuickAction,
};
use enzui_widgets::context_menu_host::{
    ContextMenuEvent, ContextMenuState, apply_event_restoring, apply_event_restoring_with,
    enz_context_menu_host, enz_optional_point_context_menu_host,
};
use enzui_widgets::{
    context_menu_host::FocusRestoreTarget,
    file_drop::{FileDrop, enz_file_drop_target_with_receipt},
};
use iced::{
    Element, Length, Subscription, Task,
    widget::{column, container, mouse_area, row, space},
    window,
};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

const FIND_APPS_WIDTH: f32 = 132.0;
const MENU_WINDOWS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Origin {
    Dock,
    Bar,
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Changed,
    Badges(u64, enzui_shell::launcher_badges::Snapshot),
    Preview(crate::dock_hover::Event),
    Apps(u64, Vec<App>),
    Loaded(u64, Result<Snapshot, String>),
    Prepared(u64, Result<(), String>),
    Hover(bool, bool),
    TouchContact(iced::touch::Finger, bool),
    RestorePointerHover(iced::Point),
    Hide(u64),
    Frame(window::Id, Instant),
    KeyboardReady(window::Id, Result<(), String>),
    Click(String),
    Launch(String),
    DropFolders(FileDrop),
    DropOnApp(String, FileDrop),
    Reorder(enzui_core::drag::DropRequest),
    Folders(enzui_shell::plugins::service::Owner, FileDrop, Vec<String>),
    Invoke(String, Origin, ContextMenuEvent),
    Menu(ContextMenuEvent),
    Anchor(u64, Result<iced::Point, String>),
    IconBounds(u64, String, Origin, Option<iced::Rectangle>),
    Done(u64, Result<(), String>),
    PolicyPublished(Result<(), String>),
    Search(Box<enzui_shell::search::Item>, ContextMenuEvent),
    SearchSaved(Result<(), String>),
}
fn message(event: Event) -> Message {
    Message::Dock(event)
}
pub(crate) struct Entry {
    pub group: Group,
    pub model: MenuModel,
}
pub(crate) struct Menu {
    pub id: String,
    pub origin: Origin,
    pub model: MenuModel,
    pub state: ContextMenuState,
}
struct KeyboardNavigation {
    window: window::Id,
    id: iced::widget::Id,
    previous: Option<WindowTarget>,
}

pub(crate) struct State {
    pub badges: enzui_shell::launcher_badges::Snapshot,
    pub badges_generation: u64,
    pub claim: Option<crate::plugin_applications::Claim>,
    pub surface_element: Option<(enzui_shell::plugins::service::Owner, String)>,
    pub synced_owner: Option<(enzui_shell::plugins::service::Owner, String)>,
    pub request_serial: u64,
    pub writes: std::collections::VecDeque<Change>,
    pub writing: Option<(enzui_shell::plugins::service::Owner, u64)>,
    pub write_serial: u64,
    pub pending_pin_order: Option<Vec<String>>,
    pub preview: crate::dock_hover::State,
    pub window: Option<window::Id>,
    pub geometry: Option<Geometry>,
    pub snapshot: Option<Snapshot>,
    pub apps: Arc<[App]>,
    pub entries: Vec<Entry>,
    pinned: usize,
    strip: dock_motion::Strip,
    keyboard: Option<KeyboardNavigation>,
    pub bar_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
    dock_scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
    reveal: dock_motion::Track,
    launches: std::collections::BTreeMap<String, u64>,
    pub menu: Option<Menu>,
    pub closed: ContextMenuState,
    pub search_model: MenuModel,
    pub preferences: dock::Preferences,
    pub generation: u64,
    pub menu_serial: u64,
    pub action_serial: u64,
    pub busy: bool,
    pub dirty: bool,
    pub apps_busy: bool,
    pub opening: bool,
    pub hovered: bool,
    pub hover_body: bool,
    pub hover_sensor: bool,
    touches: HashSet<iced::touch::Finger>,
    touch_cursor: bool,
    pub hide_serial: u64,
    hide_pending: bool,
    pub hidden: bool,
    pub fullscreen: bool,
    pub error: Option<String>,
    pending: Option<(String, Origin, InvocationKind)>,
    publishing_policy: bool,
    sent_policy: Option<(
        Geometry,
        dock::Preferences,
        bool,
        bool,
        bool,
        Option<iced::Rectangle>,
    )>,
    layout: Option<(
        enzui_shell::settings::BarEdge,
        enzui_shell::settings::Metrics,
        (u32, u32),
        bool,
    )>,
    search_saving: bool,
    pub search_offer: Option<Box<enzui_shell::plugins::store::Record>>,
    pub search_item: Option<enzui_shell::search::Item>,
    pub search_state: ContextMenuState,
}
impl Default for State {
    fn default() -> Self {
        Self {
            badges: Default::default(),
            badges_generation: 0,
            claim: None,
            surface_element: None,
            synced_owner: None,
            request_serial: 0,
            writes: Default::default(),
            writing: None,
            write_serial: 0,
            pending_pin_order: None,
            preview: Default::default(),
            window: None,
            geometry: None,
            snapshot: None,
            apps: Arc::default(),
            entries: Vec::new(),
            pinned: 0,
            strip: Default::default(),
            keyboard: None,
            bar_scroll: Default::default(),
            dock_scroll: Default::default(),
            reveal: Default::default(),
            launches: Default::default(),
            menu: None,
            closed: ContextMenuState::closed(),
            search_model: MenuModel::new(vec![
                MenuEntryModel::command("open", tr("shell-open")),
                MenuEntryModel::command("pin", tr("shell-keep-application-in-dock")),
                MenuEntryModel::command("settings", tr("shell-plugin-settings")),
            ]),
            preferences: Default::default(),
            generation: 0,
            menu_serial: 0,
            action_serial: 0,
            busy: false,
            dirty: false,
            apps_busy: false,
            opening: false,
            hovered: false,
            hover_body: false,
            hover_sensor: false,
            touches: HashSet::new(),
            touch_cursor: false,
            hide_serial: 0,
            hide_pending: false,
            hidden: false,
            fullscreen: false,
            error: None,
            pending: None,
            publishing_policy: false,
            sent_policy: None,
            layout: None,
            search_saving: false,
            search_offer: None,
            search_item: None,
            search_state: ContextMenuState::closed(),
        }
    }
}

impl State {
    fn animating(&self) -> bool {
        self.reveal.active() || self.strip.active()
    }
}

fn window_indicator<'a>(
    indicator: Indicator,
    count: usize,
    color: iced::Color,
    edge: Position,
    target: f32,
    spacing: enzui_theme::SpacingScale,
) -> Element<'a, Message> {
    use iced::{Border, Shadow};
    let vertical = matches!(edge, Position::Left | Position::Right);
    let (thickness, span) = match indicator {
        Indicator::Dots | Indicator::Count => (spacing.space3 / 2.0, spacing.space3 / 2.0),
        Indicator::Glow => (
            spacing.space1,
            if count > 1 {
                spacing.lg()
            } else {
                spacing.md()
            },
        ),
        _ => (
            spacing.space1,
            if count > 1 {
                spacing.md()
            } else {
                spacing.space3
            },
        ),
    };
    let mark = || {
        container(space())
            .width(if vertical { thickness } else { span })
            .height(if vertical { span } else { thickness })
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: Border {
                    radius: (thickness / 2.0).into(),
                    ..Default::default()
                },
                shadow: if indicator == Indicator::Glow {
                    Shadow {
                        color,
                        blur_radius: spacing.space3,
                        offset: iced::Vector::ZERO,
                    }
                } else {
                    Shadow::default()
                },
                ..Default::default()
            })
            .into()
    };
    let ink: Element<'_, Message> = if indicator == Indicator::Dots && count > 1 {
        let dots = vec![mark(), mark()];
        if vertical {
            column(dots).spacing(spacing.space1).into()
        } else {
            row(dots).spacing(spacing.space1).into()
        }
    } else {
        mark()
    };
    let mark = container(ink).center(target).padding(spacing.space1);
    match edge {
        Position::Left => mark.align_left(target),
        Position::Right => mark.align_right(target),
        Position::Top => mark.align_top(target),
        _ => mark.align_bottom(target),
    }
    .into()
}

fn progress_mark(
    percent: u8,
    edge: Position,
    target: f32,
    width: f32,
    palette: &enzui_theme::Palette,
) -> Element<'static, Message> {
    let line = |height, color| {
        container(space())
            .width(2.0)
            .height(height)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(color)),
                border: iced::Border::default().rounded(1.0),
                ..Default::default()
            })
    };
    let mut layers = vec![line(14.0, palette.border_strong).into()];
    if percent > 0 {
        layers.push(
            container(line(14.0 * f32::from(percent) / 100.0, palette.accent))
                .align_bottom(14.0)
                .into(),
        );
    }
    let mark = container(iced::widget::stack(layers)).center_y(target);
    match edge {
        Position::Left => mark
            .align_bottom(target)
            .padding(iced::Padding::ZERO.right(3.0).bottom(5.0))
            .align_right(width),
        Position::Right => mark
            .align_bottom(target)
            .padding(iced::Padding::ZERO.left(3.0).bottom(5.0))
            .align_left(width),
        _ => mark
            .padding(iced::Padding::ZERO.left(3.0))
            .align_left(width),
    }
    .into()
}

fn count_label(count: usize) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

fn window_count_badge<'a>(
    label: &str,
    active: bool,
    size: enzui_widgets::badge::BadgeSizeTokens,
    palette: &enzui_theme::Palette,
) -> Element<'a, Message> {
    use enzui_widgets::badge::{BadgeTokens, BadgeTone, enz_badge};
    let fill = palette.filled_color(if active { palette.accent } else { palette.text });
    let tokens = BadgeTokens {
        default_bg: fill.color,
        default_fg: fill.text,
        ..BadgeTokens::from_palette(palette)
    };
    enz_badge(label)
        .tone(BadgeTone::Default)
        .size_tokens(size)
        .tokens(tokens)
        .into_element()
}

fn count_corner<'a>(
    pill: Element<'a, Message>,
    edge: Position,
    target: f32,
    spacing: enzui_theme::SpacingScale,
) -> Element<'a, Message> {
    let corner = container(pill).center(target).padding(spacing.space1);
    let corner = if edge == Position::Left {
        corner.align_left(target)
    } else {
        corner.align_right(target)
    };
    if edge == Position::Top {
        corner.align_top(target)
    } else {
        corner.align_bottom(target)
    }
    .into()
}

fn group_icon<'a>(
    group: &'a Group,
    size: f32,
    palette: &enzui_theme::Palette,
    backing: bool,
) -> Element<'a, Message> {
    let glyph = match &group.image {
        Some(image) => enzui_shell::icon_theme::source(image).into_element(size),
        None => enzui_widgets::icon::enz_icon(&group.icon)
            .size(size)
            .tint(palette.text)
            .into_element(),
    };
    if !backing {
        return glyph;
    }
    let style = container::Style {
        background: Some(palette.surface.scale_alpha(0.94).into()),
        border: iced::Border {
            radius: 8.0.into(),
            width: 1.0,
            color: palette.text.scale_alpha(0.28),
        },
        shadow: iced::Shadow {
            color: iced::Color {
                a: 0.48,
                ..palette.shadow
            },
            blur_radius: 4.0,
            offset: iced::Vector::new(0.0, 1.0),
        },
        ..Default::default()
    };
    container(glyph)
        .center(size + 6.0)
        .style(move |_| style)
        .into()
}

fn window_row(window: &dock::Window, snapshot: &Snapshot) -> MenuEntryModel {
    let item =
        MenuEntryModel::command(format!("window:{}", window.target.stable_id), &window.title)
            .check(if window.active {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            });
    if let Some(context) = crate::dock_hover::window_context(window, snapshot) {
        item.description(context)
    } else {
        item
    }
}

fn model(group: &Group, p: &dock::Preferences, snapshot: &Snapshot) -> MenuModel {
    if let Some(builtin) = &group.builtin {
        let mut rows = vec![MenuEntryModel::command("open", &group.title)];
        if let Builtin::Folder(uri) = builtin {
            rows.push(MenuEntryModel::command("pin", tr("shell-remove-from-dock")).icon("pin-off"));
            if let Some(index) = p.folders.iter().position(|folder| folder == uri) {
                if index > 0 {
                    rows.push(MenuEntryModel::command("earlier", tr("shell-move-earlier")));
                }
                if index + 1 < p.folders.len() {
                    rows.push(MenuEntryModel::command("later", tr("shell-move-later")));
                }
            }
        }
        rows.push(MenuEntryModel::command("settings", tr("shell-dock-settings")).icon("settings"));
        return MenuModel::new(rows);
    }
    let mut rows = Vec::new();
    rows.extend(
        group
            .windows
            .iter()
            .take(MENU_WINDOWS)
            .map(|window| window_row(window, snapshot)),
    );
    if group.windows.len() > MENU_WINDOWS {
        rows.push(MenuEntryModel::submenu(
            "more-windows",
            tr("shell-dock-more-windows"),
        ));
    }
    if !group.windows.is_empty() {
        rows.push(MenuEntryModel::separator("windows"));
    }
    if let Some(app) = group.app.as_ref() {
        let label = if group.windows.is_empty() {
            tr("shell-open")
        } else if matches!(app, AppTarget::Prepared(id) if id != "enzui-settings" && id != "enzui-assistant")
        {
            tr("shell-new-window")
        } else {
            tr("shell-launch-application")
        };
        rows.push(MenuEntryModel::command("new", label).icon("plus"));
    }
    if group
        .windows
        .iter()
        .any(|w| !w.minimized && w.target.sidebar.is_none())
    {
        rows.push(MenuEntryModel::command("minimize", tr("shell-minimize-windows")).icon("minus"));
    }
    if !group.windows.is_empty() {
        rows.push(MenuEntryModel::command("close", tr("shell-close-all-windows")).icon("x"));
    }
    if group.pinned || (group.app.is_some() && p.pins.len() < 32) {
        if !rows.is_empty() {
            rows.push(MenuEntryModel::separator("pinning"));
        }
        rows.push(
            MenuEntryModel::command(
                "pin",
                if group.pinned {
                    tr("shell-remove-from-dock")
                } else {
                    tr("shell-keep-in-dock")
                },
            )
            .icon(if group.pinned { "pin-off" } else { "pin" }),
        );
    }
    if let Some(index) = p.pins.iter().position(|id| id == &group.id) {
        if index > 0 {
            rows.push(MenuEntryModel::command("earlier", tr("shell-move-earlier")));
        }
        if index + 1 < p.pins.len() {
            rows.push(MenuEntryModel::command("later", tr("shell-move-later")));
        }
    }
    if !rows.is_empty() {
        rows.push(MenuEntryModel::separator("preferences"));
    }
    rows.push(MenuEntryModel::command("settings", tr("shell-dock-settings")).icon("settings"));
    MenuModel::new(rows)
}
impl Shell {
    fn dock_folder_target<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        if self.dock_owner().is_none_or(|claim| {
            claim.owner.id != enzui_shell::plugins::applications::DOCK_PLUGIN
                || self.dock.preferences.folders.len() == dock::MAX_FOLDERS
        }) {
            return content;
        }
        enz_file_drop_target_with_receipt(content, |drop| message(Event::DropFolders(drop)))
            .copy_only(true)
            .into()
    }

    pub(crate) fn dock_sync(&mut self) -> Task<Message> {
        if self.dock.search_item.as_ref().is_some_and(|item| {
            !self.search.showing
                || !self
                    .search
                    .model
                    .results
                    .iter()
                    .any(|result| result.id == item.id)
        }) {
            self.dock.search_state.close();
            self.dock.search_item = None;
        }
        let defaults = dock::Preferences::default();
        let desired = self
            .dock_owner()
            .map_or(&defaults, |claim| &claim.preferences)
            .effective(self.bar);
        let owner_changed = match (self.dock.synced_owner.as_ref(), self.dock_owner()) {
            (Some((owner, element)), Some(claim)) => {
                owner != &claim.owner || element != &claim.element
            }
            (None, None) => false,
            _ => true,
        };
        if owner_changed {
            self.dock.synced_owner = self
                .dock_owner()
                .map(|claim| (claim.owner.clone(), claim.element.clone()));
        }
        let shares_sidebar = self.desired_settings.sidebar_enabled
            && desired.shares_sidebar(self.desired_settings.sidebar);
        let layout = (self.bar, self.metrics, self.desktop_edges(), shares_sidebar);
        let sidebar_changed = self
            .dock
            .layout
            .is_some_and(|previous| previous.3 != shares_sidebar);
        let layout_changed = self.dock.layout.replace(layout) != Some(layout);
        if !owner_changed && self.dock.preferences == desired {
            if layout_changed && self.dock.preferences.enabled {
                self.dock_reset_placement();
                self.dock_rebuild();
            }
            return if sidebar_changed {
                self.refresh_sidebar(None)
            } else {
                Task::none()
            };
        }
        let previous_pins = &self.dock.preferences.pins;
        let pins_changed = previous_pins != &desired.pins;
        let pin_membership_changed = previous_pins.len() != desired.pins.len()
            || previous_pins.iter().any(|id| !desired.pins.contains(id));
        if !owner_changed && pins_changed && !pin_membership_changed {
            let mut reordered = self.dock.preferences.clone();
            reordered.pins.clone_from(&desired.pins);
            if reordered == desired {
                self.dock.preferences = desired;
                self.dock_rebuild();
                return if sidebar_changed {
                    self.refresh_sidebar(None)
                } else {
                    Task::none()
                };
            }
        }
        let bar_material = self.bar_dock_material();
        let was = self.dock.preferences.enabled;
        self.dock.preferences = desired;
        self.dock.keyboard = None;
        let bar = if bar_material != self.bar_dock_material() {
            self.refresh_bar_glass()
        } else {
            Task::none()
        };
        let sidebar = if sidebar_changed {
            self.refresh_sidebar(None)
        } else {
            Task::none()
        };
        self.dock.generation = self.dock.generation.wrapping_add(1);
        if owner_changed || was != self.dock.preferences.enabled {
            self.dock.badges_generation = self.dock.badges_generation.wrapping_add(1);
            self.dock.badges.clear();
            self.dock.launches.clear();
        }
        self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
        self.dock.action_serial = self.dock.action_serial.wrapping_add(1);
        self.dock.search_state.close();
        self.dock.search_item = None;
        self.dock.busy = false;
        self.dock.opening = false;
        self.dock.menu = None;
        self.dock.pending = None;
        self.dock.preview.clear();
        if owner_changed || !self.dock.preferences.hover_previews || !self.dock.preferences.enabled
        {
            self.dock.preview.clear_cache();
        }
        self.dock.hovered = false;
        self.dock.hover_body = false;
        self.dock.hover_sensor = false;
        self.dock.touches.clear();
        self.dock.hide_pending = false;
        self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
        self.dock.sent_policy = None;
        self.dock.search_model = MenuModel::new(vec![
            MenuEntryModel::command("open", tr("shell-open")),
            MenuEntryModel::command(
                "pin",
                if self.dock.preferences.enabled {
                    tr("shell-keep-in-dock")
                } else {
                    tr("shell-keep-application-in-dock")
                },
            ),
            MenuEntryModel::command("settings", tr("shell-plugin-settings")),
        ]);
        if !self.dock.preferences.enabled {
            self.dock.strip = Default::default();
            self.dock.touch_cursor = false;
            self.dock.entries.clear();
            self.dock.pinned = 0;
            self.dock.apps = Arc::default();
            self.dock.snapshot = None;
            self.dock.hidden = true;
            if let Some(id) = self.dock.window.take() {
                if let Some(input) = self.surface_inputs.remove(&id) {
                    input.invalidate();
                }
                return Task::batch([bar, sidebar, window::close(id).chain(self.dock_publish())]);
            }
            return Task::batch([bar, sidebar]);
        }
        let reload = if !was || owner_changed || self.dock.apps.is_empty() || pin_membership_changed
        {
            self.dock.apps_busy = false;
            self.dock_load_apps()
        } else {
            self.dock_refresh()
        };
        Task::batch([bar, sidebar, reload])
    }
    pub(crate) fn dock_publish(&mut self) -> Task<Message> {
        let Some(geometry) = self.dock.geometry.as_ref() else {
            return Task::none();
        };
        if self.dock.publishing_policy
            || (self.dock.preferences.enabled
                && self
                    .dock
                    .window
                    .is_none_or(|id| !self.surface_inputs.contains_key(&id)))
        {
            return Task::none();
        }
        let hidden = self.dock.hidden;
        let fullscreen = self.dock.fullscreen;
        let menu = self.dock.menu.is_some();
        let preview = self
            .dock
            .preview
            .preview
            .as_ref()
            .map(|preview| preview.bounds);
        if self.dock.sent_policy.as_ref().is_some_and(
            |(old_geometry, old_prefs, old_hidden, old_fullscreen, old_menu, old_preview)| {
                old_geometry == geometry
                    && old_prefs == &self.dock.preferences
                    && *old_hidden == hidden
                    && *old_fullscreen == fullscreen
                    && *old_menu == menu
                    && *old_preview == preview
            },
        ) {
            return Task::none();
        }
        let geometry = geometry.clone();
        self.dock.publishing_policy = true;
        self.dock.sent_policy = Some((
            geometry.clone(),
            self.dock.preferences.clone(),
            hidden,
            fullscreen,
            menu,
            preview,
        ));
        let compositor = self.compositor.clone();
        let prefs = self.dock.preferences.clone();
        Task::perform(
            async move {
                compositor
                    .dock_policy(&geometry, &prefs, hidden, fullscreen, menu, preview)
                    .await
            },
            |result| message(Event::PolicyPublished(result)),
        )
    }
    pub(crate) fn dock_load_apps(&mut self) -> Task<Message> {
        if !self.dock.preferences.enabled || self.dock.apps_busy {
            return Task::none();
        }
        self.dock.apps_busy = true;
        let generation = self.dock.generation;
        let root = self.root.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    enzui_shell::search::catalogue::scan(
                        &root,
                        &enzui_shell::search::catalogue::directories(),
                    )
                    .into_iter()
                    .filter_map(App::from_item)
                    .collect()
                })
                .await
                .unwrap_or_default()
            },
            move |apps| message(Event::Apps(generation, apps)),
        )
    }
    pub(crate) fn dock_refresh(&mut self) -> Task<Message> {
        if !self.dock.preferences.enabled {
            return Task::none();
        }
        if self.dock.busy {
            self.dock.dirty = true;
            return Task::none();
        }
        self.dock.busy = true;
        let generation = self.dock.generation;
        let compositor = self.compositor.clone();
        let apps = std::sync::Arc::clone(&self.dock.apps);
        Task::perform(
            async move { compositor.dock_snapshot(&apps).await },
            move |result| message(Event::Loaded(generation, result)),
        )
    }
    fn dock_reset_placement(&mut self) {
        self.dock.preview.clear();
        self.dock.menu = None;
        self.dock.pending = None;
        self.dock.keyboard = None;
        self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
        self.dock.hovered = false;
        self.dock.hover_body = false;
        self.dock.hover_sensor = false;
        self.dock.touches.clear();
        self.dock.hide_pending = false;
        self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
    }

    pub(crate) fn dock_rebuild(&mut self) {
        let Some(snapshot) = self.dock.snapshot.as_ref() else {
            return;
        };
        let p = &self.dock.preferences;
        let shares_sidebar = self.dock_shares_sidebar();
        let sidebar_windows;
        let windows = if shares_sidebar {
            sidebar_windows = snapshot
                .windows
                .iter()
                .filter(|w| w.target.sidebar.is_none())
                .cloned()
                .collect::<Vec<_>>();
            &sidebar_windows
        } else {
            &snapshot.windows
        };
        let slots = dock::Slots::new(p, self.metrics, shares_sidebar);
        let groups = dock::groups(
            p,
            &self.dock.apps,
            windows,
            snapshot.desktop,
            slots.icon,
            snapshot.scale,
        );
        self.dock.entries = groups
            .into_iter()
            .map(|group| Entry {
                model: model(&group, p, snapshot),
                group,
            })
            .collect();
        self.dock
            .launches
            .retain(|id, _| self.dock.entries.iter().any(|entry| entry.group.id == *id));
        self.dock
            .preview
            .update_windows(&self.dock.entries, snapshot.screen);
        let pinned = p.pins.len().min(self.dock.entries.len());
        self.dock.pinned = pinned;
        let divider = pinned > 0
            && self
                .dock
                .entries
                .get(pinned)
                .is_some_and(|entry| !entry.group.windows.is_empty());
        let motion = self.tokens().motion;
        let motion = motion.with_reduced_motion(
            motion.reduced_motion
                || p.position == Position::Bar
                || self.dock.geometry.is_none()
                || self.dock.entries.is_empty()
                || self.dock.hidden,
        );
        self.dock.strip.sync(
            self.dock.entries[pinned..].iter().map(|entry| &entry.group),
            &p.pins,
            divider,
            Instant::now(),
            motion,
        );
        let span = if self.dock.entries.is_empty()
            && !matches!(p.position, Position::Left | Position::Right)
        {
            FIND_APPS_WIDTH
        } else {
            slots.span(self.dock.entries.len()) + f32::from(divider) * slots.separator_span()
        };
        let mut geometry = dock::geometry(
            p,
            span,
            snapshot.screen,
            self.bar,
            self.metrics,
            self.desktop_edges(),
            shares_sidebar,
        );
        geometry.output.clone_from(&snapshot.output);
        geometry.output_id = snapshot.output_id;
        geometry.desktop = snapshot.desktop;
        self.dock.fullscreen = dock::fullscreen(&geometry, &snapshot.windows);
        if self.dock.fullscreen {
            self.dock.keyboard = None;
            self.dock.preview.clear();
            self.dock.menu = None;
            self.dock.pending = None;
            self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
            self.dock.hovered = false;
            self.dock.hover_body = false;
            self.dock.hover_sensor = false;
            self.dock.touches.clear();
            self.dock.hide_pending = false;
            self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
        }
        self.dock.hidden = self.dock.fullscreen
            || (dock::obscured(p, &geometry, &snapshot.windows)
                && !self.dock.hovered
                && !self.dock.hide_pending
                && self.dock.menu.is_none()
                && self.dock.keyboard.is_none()
                && self.dock.preview.preview.is_none());
        self.dock.geometry = Some(geometry);
    }
    pub(crate) fn dock_sync_reveal(&mut self) {
        let detached = self.dock.preferences.position != Position::Bar;
        let visible = self.dock.preferences.enabled && detached && !self.dock.hidden;
        let motion = self.tokens().motion;
        if motion.reduced_motion || self.dock.fullscreen || (detached && self.dock.hidden) {
            self.dock.launches.clear();
        }
        self.dock.reveal.retarget(
            f32::from(visible),
            Instant::now(),
            motion.with_reduced_motion(
                motion.reduced_motion
                    || self.dock.fullscreen
                    || self.dock.keyboard.is_some()
                    || !detached
                    || self.dock.window.is_none(),
            ),
            enzui_theme::MotionToken::Normal,
        );
    }

    fn first_dock_focus(&self) -> Option<iced::widget::Id> {
        if self.dock.entries.is_empty() {
            return Some("dock-add-app".into());
        }
        self.dock
            .entries
            .iter()
            .find(|entry| {
                entry.group.builtin.is_some()
                    || entry.group.app.is_some()
                    || !entry.group.windows.is_empty()
            })
            .map(|entry| format!("dock-app-{}", entry.group.id).into())
    }

    pub(crate) fn dock_focus_keyboard(&mut self) -> Task<Message> {
        if !self.dock_active()
            || self.dock.fullscreen
            || self.dock.menu.is_some()
            || self.dock.keyboard.is_some()
        {
            return Task::none();
        }
        let in_bar = self.dock.preferences.position == Position::Bar;
        let (Some(window), Some(id)) = (
            if in_bar {
                Some(self.panel)
            } else {
                self.dock.window
            },
            self.first_dock_focus(),
        ) else {
            return Task::none();
        };
        let previous = self.dock.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .windows
                .iter()
                .find(|window| window.active)
                .map(|w| w.target.clone())
        });
        self.dock.keyboard = Some(KeyboardNavigation {
            window,
            id,
            previous,
        });
        self.dock.preview.clear();
        self.dock.hidden = false;
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                if in_bar {
                    compositor.focus_desktop_bar().await
                } else {
                    compositor.focus_dock().await
                }
            },
            move |result| message(Event::KeyboardReady(window, result)),
        )
    }

    pub(crate) fn dock_end_keyboard(
        &mut self,
        window: window::Id,
        restore: bool,
    ) -> Option<Task<Message>> {
        if !self
            .dock
            .keyboard
            .as_ref()
            .is_some_and(|nav| nav.window == window)
        {
            return None;
        }
        let nav = self.dock.keyboard.take()?;
        self.dock_rebuild();
        Some(match nav.previous {
            Some(target) if restore => self.dock_windows(vec![target], "focus"),
            _ => Task::none(),
        })
    }

    pub(crate) fn dock_subscription(&self) -> Subscription<Message> {
        if !self.dock.preferences.enabled {
            return Subscription::none();
        }
        use iced_runtime::futures::subscription;
        let generation = self.dock.badges_generation;
        let mut animation = Subscription::none();
        if let Some(target) = self.dock.window
            && self.dock.preferences.position != Position::Bar
            && self.dock.animating()
        {
            animation = subscription::filter_map_with_interest(
                ("dock-motion", target),
                iced::event::Interest::REDRAWS,
                move |event| match event {
                    subscription::Event::Interaction {
                        window,
                        event: iced::Event::Window(window::Event::RedrawRequested(at)),
                        ..
                    } if window == target => Some(message(Event::Frame(window, at))),
                    _ => None,
                },
            );
        }
        let restore_pointer = self.dock.touch_cursor && self.dock.touches.is_empty();
        let touch = if let Some(target) = self.dock.window
            && self.dock.preferences.position != Position::Bar
            && matches!(
                self.dock.preferences.visibility,
                dock::Visibility::Always | dock::Visibility::Overlap
            ) {
            subscription::filter_map_with_interest(
                ("dock-touch", target, restore_pointer),
                iced::event::Interest::POINTER,
                move |event| match event {
                    subscription::Event::Interaction {
                        window,
                        event: iced::Event::Touch(event),
                        ..
                    } if window == target => match event {
                        iced::touch::Event::FingerPressed { id, .. } => {
                            Some(message(Event::TouchContact(id, true)))
                        }
                        iced::touch::Event::FingerLifted { id, .. }
                        | iced::touch::Event::FingerLost { id, .. } => {
                            Some(message(Event::TouchContact(id, false)))
                        }
                        _ => None,
                    },
                    subscription::Event::Interaction {
                        window,
                        event: iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }),
                        ..
                    } if window == target && restore_pointer => {
                        Some(message(Event::RestorePointerHover(position)))
                    }
                    _ => None,
                },
            )
        } else {
            Subscription::none()
        };
        Subscription::batch([
            animation,
            touch,
            enzui_shell::launcher_badges::subscription(generation)
                .map(|(generation, badges)| message(Event::Badges(generation, badges))),
            self.compositor
                .focus_changes()
                .map(|()| message(Event::Changed)),
            self.compositor
                .dock_changes()
                .map(|()| message(Event::Changed)),
        ])
    }
    fn dock_hide_after(&mut self, delay: Duration) -> Task<Message> {
        self.dock.hide_pending = true;
        self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
        let serial = self.dock.hide_serial;
        Task::perform(async move { tokio::time::sleep(delay).await }, move |()| {
            message(Event::Hide(serial))
        })
    }
    fn dock_pointer_hover(&mut self) -> Task<Message> {
        self.dock.hovered = self.dock.hover_body || self.dock.hover_sensor;
        if self.dock.hovered {
            self.dock.hide_pending = false;
            self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
            self.dock.hidden = false;
            Task::none()
        } else {
            self.dock_hide_after(Duration::from_millis(400))
        }
    }
    pub(crate) fn dock_event(&mut self, event: Event) -> Task<Message> {
        if !self.dock_active()
            && matches!(
                &event,
                Event::Click(_)
                    | Event::Launch(_)
                    | Event::Invoke(..)
                    | Event::DropFolders(_)
                    | Event::DropOnApp(..)
                    | Event::Reorder(_)
                    | Event::Folders(..)
            )
        {
            return Task::none();
        }
        match event {
            Event::Frame(id, at) => {
                if self.dock.window == Some(id)
                    && self.dock.preferences.position != Position::Bar
                    && self.dock.animating()
                {
                    self.dock.strip.advance(at);
                    self.dock.reveal.advance(at);
                    self.redraw_only = Some(id);
                } else {
                    self.quiet = true;
                }
                Task::none()
            }
            Event::KeyboardReady(window, result) => {
                let Some(nav) = self
                    .dock
                    .keyboard
                    .as_ref()
                    .filter(|nav| nav.window == window)
                else {
                    return Task::none();
                };
                match result {
                    Ok(()) => iced_runtime::task::widget_in(
                        window,
                        enzui_core::focus::focus_and_reveal_id(nav.id.clone()),
                    ),
                    Err(error) => {
                        self.dock.error = Some(error);
                        self.dock_end_keyboard(window, false)
                            .unwrap_or_else(Task::none)
                    }
                }
            }
            Event::Preview(event) => self.dock_preview_event(event),
            Event::Changed => self.dock_refresh(),
            Event::Badges(generation, badges) => {
                if self.dock_active() && generation == self.dock.badges_generation {
                    self.dock.badges = badges;
                }
                Task::none()
            }
            Event::Apps(generation, apps) => {
                if generation != self.dock.generation {
                    return Task::none();
                }
                self.dock.apps_busy = false;
                self.dock.apps = apps.into();
                self.dock_refresh()
            }
            Event::Loaded(generation, result) => {
                if generation != self.dock.generation {
                    return Task::none();
                }
                self.dock.busy = false;
                match result {
                    Ok(snapshot) => {
                        if self.dock.snapshot.as_ref().is_some_and(|old| {
                            old.output_id != snapshot.output_id
                                || old.output != snapshot.output
                                || old.screen != snapshot.screen
                                || old.scale != snapshot.scale
                        }) {
                            self.dock_reset_placement();
                            self.dock.preview.clear_cache();
                        }
                        self.dock.snapshot = Some(snapshot);
                    }
                    Err(error) => {
                        self.dock.error = Some(error);
                        return Task::none();
                    }
                }
                let old_screen = self.dock.geometry.as_ref().map(|g| g.screen);
                self.dock_rebuild();
                let mut tasks = Vec::new();
                if self.dock.window.is_none() && !self.dock.opening {
                    let Some(snapshot) = self.dock.snapshot.clone() else {
                        return Task::none();
                    };
                    self.dock.opening = true;
                    let compositor = self.compositor.clone();
                    tasks.push(Task::perform(
                        async move { compositor.prepare_dock(&snapshot).await },
                        move |result| message(Event::Prepared(generation, result)),
                    ));
                } else if let (Some(id), Some(geometry)) =
                    (self.dock.window, self.dock.geometry.clone())
                {
                    let resize = if old_screen == Some(geometry.screen) {
                        Task::none()
                    } else {
                        resize_chrome(id, geometry.screen)
                    };
                    let compositor = self.compositor.clone();
                    tasks.push(resize.chain(Task::perform(
                        async move { compositor.place_dock(&geometry).await },
                        |r| message(Event::Done(0, r)),
                    )));
                }
                if std::mem::take(&mut self.dock.dirty) {
                    tasks.push(self.dock_refresh());
                }
                Task::batch(tasks)
            }
            Event::Prepared(generation, result) => {
                if generation != self.dock.generation || !self.dock.preferences.enabled {
                    return Task::none();
                }
                self.dock.opening = false;
                if let Err(error) = result {
                    if self.dock.error.as_ref() != Some(&error) {
                        eprintln!("enzui-shell dock: {error}");
                    }
                    self.notice = Some(error.clone());
                    self.dock.error = Some(error);
                    return Task::none();
                }
                if self.dock.window.is_some() {
                    return Task::none();
                }
                let size = self
                    .dock
                    .geometry
                    .as_ref()
                    .map_or(iced::Size::new(1440.0, 900.0), |g| g.screen);
                let (id, open) =
                    window::open(window_settings("dev.enzui.Dock", size.width, size.height));
                self.dock.window = Some(id);
                open.map(Message::Opened)
            }
            Event::Hover(hovered, body) => {
                if self.dock.touch_cursor || (self.dock.fullscreen && self.dock.menu.is_none()) {
                    return Task::none();
                }
                if body {
                    self.dock.hover_body = hovered;
                } else {
                    self.dock.hover_sensor = hovered;
                }
                self.dock_pointer_hover()
            }
            Event::RestorePointerHover(point) => {
                let Some(geometry) = self.dock.geometry.as_ref() else {
                    return Task::none();
                };
                if !self.dock.touch_cursor || !self.dock.touches.is_empty() || self.dock.fullscreen
                {
                    return Task::none();
                }
                self.dock.touch_cursor = false;
                self.dock.hover_body = !self.dock.hidden && geometry.body.contains(point);
                self.dock.hover_sensor = geometry.sensor.contains(point);
                self.dock_pointer_hover()
            }
            Event::TouchContact(id, pressed) => {
                if self.dock.fullscreen {
                    return Task::none();
                }
                if pressed {
                    // Iced retains the touch location as its cursor until real pointer motion.
                    self.dock.touch_cursor = true;
                    self.dock.touches.insert(id);
                    self.dock.hovered = true;
                    self.dock.hidden = false;
                    self.dock.hide_pending = false;
                    self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
                    return Task::none();
                }
                if !self.dock.touches.remove(&id) || !self.dock.touches.is_empty() {
                    return Task::none();
                }
                self.dock.hover_body = false;
                self.dock.hover_sensor = false;
                self.dock.hovered = false;
                self.dock_hide_after(Duration::from_millis(1600))
            }
            Event::Hide(serial) => {
                if serial == self.dock.hide_serial {
                    self.dock.hide_pending = false;
                    self.dock_rebuild();
                }
                Task::none()
            }
            Event::Reorder(request) => {
                self.dock.preview.clear();
                let change = Change::ReorderPins(request);
                let mut preview = self.dock.preferences.clone();
                if let Some(order) = &self.dock.pending_pin_order {
                    preview.pins.clone_from(order);
                }
                if preview.apply(&change).is_ok() {
                    // Keep the drop position while the delegated store writes the order.
                    self.dock.pending_pin_order = Some(preview.pins);
                }
                self.dock_save(change)
            }
            Event::DropOnApp(id, drop) => {
                if drop.is_temporary() {
                    return Task::none();
                }
                let Some(claim) = self.dock_owner() else {
                    return Task::none();
                };
                let Some(app) = self
                    .dock
                    .entries
                    .iter()
                    .find(|entry| entry.group.id == id)
                    .and_then(|entry| entry.group.file_target.clone())
                else {
                    return Task::none();
                };
                let authority = Arc::clone(&claim.authority);
                self.dock.preview.clear();
                self.dock.action_serial = self.dock.action_serial.wrapping_add(1);
                let serial = self.dock.action_serial;
                let compositor = self.compositor.clone();
                Task::perform(
                    async move {
                        if !authority.load(std::sync::atomic::Ordering::Acquire) {
                            return Err(tr("shell-plugin-authority-changed").into());
                        }
                        compositor.open_dock_files(app, drop.paths()).await?;
                        drop.finish();
                        Ok(())
                    },
                    move |result| message(Event::Done(serial, result)),
                )
            }
            Event::DropFolders(drop) => {
                if drop.is_temporary() {
                    return Task::none();
                }
                let Some(claim) = self.dock_owner().filter(|claim| {
                    claim.owner.id == enzui_shell::plugins::applications::DOCK_PLUGIN
                }) else {
                    return Task::none();
                };
                let owner = claim.owner.clone();
                Task::perform(
                    async move {
                        let paths = drop.paths().to_vec();
                        let folders = tokio::task::spawn_blocking(move || {
                            paths
                                .into_iter()
                                .take(dock::MAX_FOLDERS * 4)
                                .filter_map(|path| path.canonicalize().ok())
                                .filter(|path| path.is_dir())
                                .map(|path| enzui_shell::desktop_portal::uri(&path))
                                .filter(|uri| dock::folder_path(uri).is_some())
                                .collect()
                        })
                        .await
                        .unwrap_or_default();
                        (drop, folders)
                    },
                    move |(drop, folders)| message(Event::Folders(owner, drop, folders)),
                )
            }
            Event::Folders(owner, drop, folders) => {
                if !folders.is_empty()
                    && self.dock_owner().is_some_and(|claim| claim.owner == owner)
                {
                    drop.finish();
                    return self.dock_save(Change::PinFolders(folders));
                }
                Task::none()
            }
            Event::Click(id) => {
                let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
                    return Task::none();
                };
                let group = entry.group.clone();
                if let Some(builtin) = group.builtin {
                    return self.dock_builtin(builtin);
                }
                if group.windows.len() > 1 {
                    let origin = if self.dock.preferences.position == Position::Bar {
                        Origin::Bar
                    } else {
                        Origin::Dock
                    };
                    let window = match origin {
                        Origin::Bar => Some(self.panel),
                        Origin::Dock => self.dock.window,
                    };
                    let Some(window) = window else {
                        return Task::none();
                    };
                    self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
                    let serial = self.dock.menu_serial;
                    return enzui_shell::widget_bounds::bounds(window, format!("dock-app-{id}"))
                        .map(move |bounds| {
                            message(Event::IconBounds(serial, id.clone(), origin, bounds))
                        });
                }
                if let Some(window) = group.windows.first() {
                    return self.dock_windows(vec![window.target.clone()], "focus");
                }
                self.dock_launch(group.id, group.app, false)
            }
            Event::Launch(id) => {
                let Some(app) = self
                    .dock
                    .entries
                    .iter()
                    .find(|e| e.group.id == id)
                    .and_then(|entry| entry.group.app.clone())
                else {
                    return Task::none();
                };
                self.dock.preview.clear();
                self.dock_launch(id, Some(app), true)
            }
            Event::IconBounds(serial, id, origin, Some(bounds))
                if serial == self.dock.menu_serial =>
            {
                self.dock_open_menu(
                    id,
                    origin,
                    InvocationKind::Keyboard(enzui_core::context_menu::Rect::new(
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                    )),
                )
            }
            Event::IconBounds(..) => Task::none(),
            Event::Invoke(id, origin, ContextMenuEvent::Invoked(invocation)) => {
                self.dock_open_menu(id, origin, invocation)
            }
            Event::Invoke(..) => Task::none(),
            Event::Anchor(serial, result) => {
                if serial != self.dock.menu_serial {
                    return Task::none();
                }
                let Some((id, origin, _)) = self.dock.pending.take() else {
                    return Task::none();
                };
                let Ok(point) = result else {
                    self.dock.error = result.err();
                    return Task::none();
                };
                let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
                    return Task::none();
                };
                let mut state = ContextMenuState::closed();
                state.open_with_restore(
                    InvocationKind::Pointer(enzui_core::context_menu::Point::new(point.x, point.y)),
                    FocusRestoreTarget::Widget(iced::advanced::widget::Id::from(format!(
                        "dock-app-{id}"
                    ))),
                );
                let mut model = entry.model.clone();
                if let Some(layout) = TileMenu::rows(self, QuickAction::Tiling) {
                    let mut rows = model.entries().to_vec();
                    rows.insert(
                        rows.len() - 1,
                        MenuEntryModel::submenu("layout", tr("shell-desktop-window-layout")),
                    );
                    model = MenuModel::new(rows).with_submenu("layout", layout);
                }
                if let Some(snapshot) = self.dock.snapshot.as_ref()
                    && entry.group.windows.len() > MENU_WINDOWS
                {
                    let windows = entry.group.windows[MENU_WINDOWS..]
                        .iter()
                        .map(|window| window_row(window, snapshot))
                        .collect();
                    model = model.with_submenu("more-windows", MenuModel::new(windows));
                }
                self.dock.menu = Some(Menu {
                    id,
                    origin,
                    model,
                    state,
                });
                self.dock.hidden = false;
                let compositor = self.compositor.clone();
                Task::perform(async move { compositor.focus_dock().await }, move |r| {
                    message(Event::Done(0, r))
                })
            }
            Event::Menu(event) => {
                let Some(menu) = self.dock.menu.as_mut() else {
                    return Task::none();
                };
                let id = menu.id.clone();
                let origin = menu.origin;
                let submenus = menu.model.clone();
                let update = apply_event_restoring_with::<Message>(
                    &mut menu.model,
                    &mut menu.state,
                    &event,
                    &|_, parent: &EntryId| submenus.submenu(parent).cloned(),
                );
                let activated = update.activated().cloned();
                let closed = !menu.state.is_open();
                if closed {
                    self.dock.menu = None;
                    self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
                    self.dock_rebuild();
                }
                let action = activated.map_or_else(Task::none, |entry| {
                    if let Some(message) = TileMenu::message(self, QuickAction::Tiling, &entry) {
                        self.update(message)
                    } else {
                        self.dock_menu_action(&id, entry.as_str())
                    }
                });
                let action = if closed && matches!(origin, Origin::Bar) {
                    let compositor = self.compositor.clone();
                    action.chain(Task::perform(
                        async move { compositor.restore_bar_from_dock().await },
                        |r| message(Event::Done(0, r)),
                    ))
                } else {
                    action
                };
                update.finish(action)
            }
            Event::PolicyPublished(result) => {
                self.dock.publishing_policy = false;
                self.quiet = true;
                if let Err(error) = result {
                    eprintln!("enzui-shell dock: {error}");
                }
                Task::none()
            }
            Event::Done(serial, result) => {
                if serial != 0 && serial != self.dock.action_serial {
                    return Task::none();
                }
                let error = result.err();
                self.quiet = serial == 0 && self.dock.error == error;
                self.dock.error = error;
                if let Some(error) = &self.dock.error {
                    eprintln!("enzui-shell dock: {error}");
                    if serial != 0 {
                        self.notice = Some(error.clone());
                    }
                }
                if serial == 0 {
                    Task::none()
                } else {
                    self.dock_refresh()
                }
            }
            Event::Search(item, event) => self.dock_search_menu(*item, event),
            Event::SearchSaved(result) => {
                self.dock.search_saving = false;
                match result {
                    Ok(()) => {
                        self.plugin_command(enzui_shell::plugins::service::Command::Refresh);
                    }
                    Err(error) => self.notice = Some(error),
                }
                Task::none()
            }
        }
    }
    fn dock_open_menu(
        &mut self,
        id: String,
        origin: Origin,
        invocation: InvocationKind,
    ) -> Task<Message> {
        self.dock.preview.clear();
        self.dock.menu_serial = self.dock.menu_serial.wrapping_add(1);
        let serial = self.dock.menu_serial;
        self.dock.pending = Some((id, origin, invocation));
        let compositor = self.compositor.clone();
        let class = match origin {
            Origin::Dock => "dev.enzui.Dock",
            Origin::Bar => "dev.enzui.Desktop",
        };
        Task::perform(
            async move { compositor.dock_menu_point(class, invocation).await },
            move |point| message(Event::Anchor(serial, point)),
        )
    }
    fn dock_builtin(&mut self, builtin: Builtin) -> Task<Message> {
        let Some(authority) = self.dock_owner().map(|claim| Arc::clone(&claim.authority)) else {
            return Task::none();
        };
        self.dock.preview.clear();
        match builtin {
            Builtin::Action(Action::Overview) => {
                return self.spread_message(crate::window_spread::SpreadMessage::Toggle);
            }
            Builtin::Action(Action::Applications) => return Task::done(Message::ShowLauncher),
            _ => {}
        }
        let compositor = self.compositor.clone();
        let root = self.root.clone();
        self.dock.action_serial = self.dock.action_serial.wrapping_add(1);
        let serial = self.dock.action_serial;
        Task::perform(
            async move {
                if !authority.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(tr("shell-plugin-authority-changed").into());
                }
                let path = match builtin {
                    Builtin::Folder(uri) => dock::folder_path(&uri),
                    Builtin::Action(Action::Downloads) => tokio::task::spawn_blocking(|| {
                        enzui_shell::chooser_places::user_folders()
                            .into_iter()
                            .find(|place| place.icon == "download")
                            .map(|place| place.path)
                    })
                    .await
                    .ok()
                    .flatten(),
                    _ => None,
                }
                .ok_or_else(|| tr("shell-dock-folder-unavailable"))?;
                compositor.open_dock_folder(&root, &path).await
            },
            move |result| message(Event::Done(serial, result)),
        )
    }

    fn dock_launch(&mut self, id: String, app: Option<AppTarget>, new: bool) -> Task<Message> {
        let Some(authority) = self
            .dock_owner()
            .filter(|_| self.dock_active())
            .map(|c| std::sync::Arc::clone(&c.authority))
        else {
            return Task::none();
        };
        let Some(app) = app else { return Task::none() };
        self.dock.action_serial = self.dock.action_serial.wrapping_add(1);
        let serial = self.dock.action_serial;
        if !self.tokens().motion.reduced_motion {
            self.dock.launches.insert(id, serial);
        }
        let root = self.root.clone();
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                if !authority.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(tr("shell-plugin-authority-changed").into());
                }
                if new {
                    compositor.launch_dock_app(&root, app).await
                } else {
                    compositor
                        .execute_search_target(&root, Target::App(app))
                        .await
                }
            },
            move |result| message(Event::Done(serial, result)),
        )
    }
    pub(crate) fn dock_windows(
        &mut self,
        windows: Vec<WindowTarget>,
        action: &'static str,
    ) -> Task<Message> {
        let Some(authority) = self
            .dock_owner()
            .filter(|_| self.dock_active())
            .map(|c| std::sync::Arc::clone(&c.authority))
        else {
            return Task::none();
        };
        self.dock.action_serial = self.dock.action_serial.wrapping_add(1);
        let serial = self.dock.action_serial;
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                let mut result = Ok(());
                for window in windows {
                    if !authority.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(tr("shell-plugin-authority-changed").into());
                    }
                    if let Err(error) = compositor.dock_window_action(window, action).await {
                        result = Err(error);
                    }
                }
                result
            },
            move |result| message(Event::Done(serial, result)),
        )
    }
    fn dock_menu_action(&mut self, id: &str, action: &str) -> Task<Message> {
        let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
            return Task::none();
        };
        let group = entry.group.clone();
        if let Some(builtin) = group.builtin {
            return match (action, builtin) {
                ("open", item) => self.dock_builtin(item),
                ("pin", Builtin::Folder(uri)) => self.dock_save(Change::UnpinFolder(uri)),
                ("earlier" | "later", Builtin::Folder(uri)) => self.dock_save(Change::MoveFolder(
                    uri,
                    if action == "earlier" { -1 } else { 1 },
                )),
                ("settings", _) => self.dock_plugin_settings(),
                _ => Task::none(),
            };
        }
        match action {
            "pin" => self.dock_save(Change::Pin(group.id, !group.pinned)),
            "earlier" | "later" => self.dock_save(Change::Move(
                group.id,
                if action == "earlier" { -1 } else { 1 },
            )),
            "settings" => self.dock_plugin_settings(),
            "new" => self.dock_launch(group.id, group.app, !group.windows.is_empty()),
            "close" => self.dock_windows(
                group.windows.iter().map(|w| w.target.clone()).collect(),
                "close",
            ),
            "minimize" => self.dock_windows(
                group
                    .windows
                    .iter()
                    .filter(|w| !w.minimized && w.target.sidebar.is_none())
                    .map(|w| w.target.clone())
                    .collect(),
                "minimize",
            ),
            _ => {
                if let Some(w) = group
                    .windows
                    .iter()
                    .find(|w| action == format!("window:{}", w.target.stable_id))
                {
                    self.dock_windows(vec![w.target.clone()], "focus")
                } else {
                    Task::none()
                }
            }
        }
    }
    pub(crate) fn bar_dock_material(&self) -> Option<Material> {
        let p = &self.dock.preferences;
        (p.enabled && p.position == Position::Bar && p.material != Material::Panels)
            .then_some(p.material)
    }

    pub(crate) fn dock_material(&self) -> Option<enzui_theme::MaterialStyle> {
        let prefs = &self.dock.preferences;
        if prefs.style == Style::Minimal || prefs.position == Position::Bar {
            return None;
        }
        let panels = self.metrics.islands.and_then(|_| self.chrome_glass());
        prefs.material.glass(panels)
    }

    pub(crate) fn dock_icons(&self, origin: Origin, vertical: bool) -> Element<'_, Message> {
        let palette = match origin {
            Origin::Bar => match self.bar_dock_material() {
                Some(material) => match material.glass(None) {
                    Some(style) => self.tokens().on_material(style),
                    None => self.tokens(),
                },
                None => self.window_tokens(Some(self.panel)),
            },
            Origin::Dock => self.window_tokens(self.dock.window),
        }
        .palette;
        let p = &self.dock.preferences;
        let slots = dock::Slots::new(p, self.metrics, self.dock_shares_sidebar());
        let size = slots.icon;
        let target = slots.target;
        let default_highlight = p.highlight_radius == dock::Preferences::default().highlight_radius;
        let radius = if default_highlight && matches!(origin, Origin::Bar) {
            self.metrics.bar_item_radius(7.0).min(target / 2.0)
        } else {
            f32::from(p.highlight_radius).min(target / 2.0)
        };
        let edge = match (p.position, self.bar) {
            (Position::Bar, enzui_shell::settings::BarEdge::Top) => Position::Top,
            (Position::Bar, enzui_shell::settings::BarEdge::Bottom) => Position::Bottom,
            (position, _) => position,
        };
        let spacing = self.tokens().spacing;
        use enzui_widgets::pressable::{
            PressableStyle, PressableTokens, SurfaceColors, enz_pressable,
        };
        let pressable = PressableTokens::from_palette(&palette);
        let badge_size = enzui_widgets::badge::BadgeSizeTokens {
            height: spacing.lg(),
            text_size: enzui_theme::TypographyScale::ambient().label_size,
            radius: spacing.sm(),
            padding_x: spacing.space1,
            min_width: spacing.lg(),
            icon_size: 0.0,
            icon_gap: 0.0,
        };
        let badge_height = badge_size
            .height
            .max(enzui_widgets::badge::BadgeSize::Sm.line_height_px());
        let icons_only = matches!(origin, Origin::Dock) && p.style == Style::Minimal;
        let in_bar = matches!(origin, Origin::Bar);
        let icon_column = target - 2.0 * spacing.space1;
        let count_width = |group: &Group| {
            (in_bar && p.indicator == Indicator::Count && group.windows.len() > 1).then(|| {
                let label = count_label(group.windows.len());
                (badge_size.text_size * 0.85 * label.len() as f32 + 2.0 * badge_size.padding_x)
                    .max(badge_size.min_width)
            })
        };
        let button_width = |group: &Group| {
            count_width(group).map_or(target, |pill| icon_column + pill + spacing.space1)
        };
        use iced::widget::tooltip::Position as Tooltip;
        let tooltip = match p.position {
            Position::Left => Tooltip::Right,
            Position::Right => Tooltip::Left,
            Position::Top => Tooltip::Bottom,
            Position::Bar if self.bar == enzui_shell::settings::BarEdge::Top => Tooltip::Bottom,
            _ => Tooltip::Top,
        };
        let pinned = self.dock.pinned;
        let desktop_ids = self
            .dock
            .entries
            .iter()
            .filter_map(|entry| enzui_shell::launcher_badges::group_id(&entry.group))
            .fold(
                std::collections::HashMap::<_, usize>::new(),
                |mut counts, id| {
                    *counts.entry(id).or_default() += 1;
                    counts
                },
            );
        let mut buttons = Vec::new();
        let mut favorites: Vec<_> = self.dock.entries[..pinned].iter().collect();
        if let Some(order) = &self.dock.pending_pin_order {
            favorites.sort_by_key(|entry| {
                order
                    .iter()
                    .position(|id| id == &entry.group.id)
                    .unwrap_or(order.len())
            });
        }
        let rows = favorites
            .into_iter()
            .map(|entry| (&entry.group, Some(&entry.model)))
            .chain(self.dock.strip.trailing.iter().map(|slot| {
                let model = self.dock.entries[pinned..]
                    .iter()
                    .find(|entry| entry.group.id == slot.group.id)
                    .map(|entry| &entry.model);
                (&slot.group, model)
            }));
        for (group, model) in rows {
            let id = group.id.clone();
            let width = button_width(group);
            let Some(model) = model else {
                let icon = container(group_icon(group, size, &palette, icons_only))
                    .center_x(width)
                    .center_y(target);
                buttons.push((id, icon.into()));
                continue;
            };
            let active = group.windows.iter().any(|w| w.active);
            let running = !group.windows.is_empty();
            let desktop_id = enzui_shell::launcher_badges::group_id(group);
            let badge = desktop_id
                .as_ref()
                .and_then(|id| self.dock.badges.get(id))
                .copied()
                .unwrap_or_default();
            let urgent = badge.urgent || group.windows.iter().any(|window| window.urgent);
            let count = group.windows.len();
            let numbered = p.indicator == Indicator::Count && count > 1;
            let compact_count = count_width(group);
            let inset = if numbered && compact_count.is_none() {
                badge_height + spacing.space1
            } else {
                0.0
            };
            let glyph_pad = if edge == Position::Top {
                iced::Padding::ZERO.top(inset)
            } else {
                iced::Padding::ZERO.bottom(inset)
            };
            let icon = container(group_icon(
                group,
                size.min((target - inset).max(spacing.space1)),
                &palette,
                icons_only,
            ))
            .center_x(if compact_count.is_some() {
                icon_column
            } else {
                width
            })
            .center_y(target)
            .padding(glyph_pad);
            let mut layers: Vec<Element<'_, Message>> = vec![
                container(icon)
                    .align_left(width)
                    .center_y(target)
                    .style(move |_| container::Style {
                        border: iced::Border {
                            width: if urgent { 2.0 } else { 0.0 },
                            color: palette.warning,
                            radius: radius.into(),
                        },
                        ..Default::default()
                    })
                    .into(),
            ];
            if let Some(serial) = self.dock.launches.get(&id) {
                let flash = container(space())
                    .width(width)
                    .height(target)
                    .style(move |_| container::Style {
                        background: Some(palette.accent.scale_alpha(0.24).into()),
                        border: iced::Border {
                            width: 1.0,
                            color: palette.accent,
                            radius: radius.into(),
                        },
                        ..Default::default()
                    });
                let fade = enzui_widgets::motion::animated(flash)
                    .opacity(0.0)
                    .enters_from(iced::Vector::ZERO, 1.0);
                layers.push(
                    enzui_widgets::keyed_row::keyed_row(vec![(serial.to_string(), fade.into())])
                        .into(),
                );
            } else {
                layers.push(space().into());
            }
            if group.builtin.is_none() && p.indicator != Indicator::None {
                let color = if active {
                    palette.accent
                } else {
                    palette.text_muted
                };
                let marker = if numbered {
                    let pill =
                        window_count_badge(&count_label(count), active, badge_size, &palette);
                    if let Some(pill_width) = compact_count {
                        container(container(pill).center_x(pill_width).center_y(target))
                            .padding(iced::Padding::ZERO.right(spacing.space1))
                            .align_right(width)
                            .into()
                    } else {
                        count_corner(pill, edge, target, spacing)
                    }
                } else {
                    window_indicator(p.indicator, count, color, edge, target, spacing)
                };
                layers.push(
                    enzui_widgets::motion::motion(
                        marker,
                        running,
                        enzui_widgets::motion::MotionEffect::Fade,
                    )
                    .enter_on_mount()
                    .into(),
                );
            }
            if let Some(label) = badge.label() {
                use enzui_widgets::badge::{BadgeSizeTokens, BadgeTone, enz_badge};
                let compact = size < 20.0;
                let indicator = enz_badge(&label)
                    .tone(BadgeTone::AccentStrong)
                    .size_tokens(BadgeSizeTokens {
                        height: if compact { 11.0 } else { 14.0 },
                        text_size: if compact { 8.0 } else { 9.0 },
                        radius: 7.0,
                        padding_x: 2.0,
                        min_width: 11.0,
                        icon_size: 0.0,
                        icon_gap: 0.0,
                    })
                    .into_element();
                let notification = container(indicator)
                    .padding(spacing.space1)
                    .align_top(target);
                let notification: Element<'_, Message> = if compact_count.is_some() {
                    container(notification.align_right(icon_column))
                        .align_left(width)
                        .into()
                } else if edge == Position::Top && numbered {
                    notification.align_left(width).into()
                } else {
                    notification.align_right(width).into()
                };
                layers.push(notification);
            }
            let progress = desktop_id
                .as_ref()
                .filter(|id| desktop_ids.get(*id) == Some(&1))
                .and_then(|id| self.dock_progress(id, badge.progress));
            if let Some(percent) = progress {
                layers.push(progress_mark(percent, edge, target, width, &palette));
            }
            let label = if group.builtin.is_some() {
                group.title.clone()
            } else {
                let mut label = tr!("shell-dock-window-count", "title" => group.title.clone(), "count" => count);
                if group.pinned {
                    label.push_str(tr("shell-kept-in-dock"));
                }
                if badge.count_visible && badge.count > 0 {
                    label.push_str(&tr!("shell-dock-unread-count", "count" => badge.count));
                }
                if urgent {
                    label.push_str(tr("shell-needs-attention"));
                }
                if let Some(percent) = progress {
                    label.push_str(&tr!("shell-dock-progress", "percent" => percent));
                }
                label
            };
            let button = enz_pressable(
                iced::widget::stack(layers)
                    .width(width)
                    .height(target)
                    .into(),
                message(Event::Click(id.clone())),
            )
            .style(PressableStyle::Plain)
            .tokens(pressable)
            .width(width)
            .padding(0)
            .radius(radius)
            .surface_colors_resolved({
                let rest = active.then_some(palette.surface_selected);
                let hover = palette.surface_hover;
                move |_| SurfaceColors {
                    background: rest,
                    hover_background: Some(hover),
                    border: None,
                }
            })
            .aria_label(label)
            .id(format!("dock-app-{id}").into())
            .enabled(group.builtin.is_some() || group.app.is_some() || running)
            .into_element();
            let hover_id = id.clone();
            let leave_id = id.clone();
            let button = mouse_area(button)
                .on_enter(message(Event::Preview(crate::dock_hover::Event::Enter(
                    hover_id, origin,
                ))))
                .on_exit(message(Event::Preview(crate::dock_hover::Event::Leave(
                    leave_id,
                ))));
            let button = if group.app.is_some() {
                button.on_middle_press(message(Event::Launch(id.clone())))
            } else {
                button
            };
            let button: Element<'_, Message> = if group.windows.len() > 1 {
                let scroll = if in_bar {
                    &self.dock.bar_scroll
                } else {
                    &self.dock.dock_scroll
                };
                iced::widget::stack([
                    button.into(),
                    Element::new(crate::dock_hover::WheelTarget {
                        id: id.clone(),
                        size: iced::Size::new(width, target),
                        scroll: scroll.clone(),
                    }),
                ])
                .into()
            } else {
                button.into()
            };
            let button: Element<'_, Message> = if running && p.hover_previews {
                button
            } else {
                let tip = enzui_widgets::tooltip::TooltipTokens {
                    gap: spacing.sm(),
                    padding_x: spacing.space5,
                    ..enzui_widgets::tooltip::TooltipTokens::from_palette(&palette)
                };
                enzui_widgets::tooltip::enz_tooltip_with_tokens(
                    button,
                    group.title.clone(),
                    tooltip,
                    tip,
                )
            };
            let menu_id = id.clone();
            let button = enz_context_menu_host(button, model, &self.dock.closed, move |event| {
                message(Event::Invoke(menu_id.clone(), origin, event))
            })
            .into();
            let button = if group.file_target.is_some() {
                enz_file_drop_target_with_receipt(button, move |drop| {
                    message(Event::DropOnApp(id.clone(), drop))
                })
                .copy_only(true)
                .key(&group.id)
                .into()
            } else {
                button
            };
            buttons.push((group.id.clone(), button));
        }
        if buttons.is_empty() {
            let mut content = row![
                enzui_widgets::icon::enz_icon("plus")
                    .size(16.0)
                    .tint(palette.text)
                    .into_element()
            ]
            .spacing(spacing.sm())
            .align_y(iced::Alignment::Center);
            if !vertical {
                content = content.push(
                    enzui_widgets::text::enz_text(tr("shell-dock-find-apps"))
                        .no_wrap()
                        .size(enzui_widgets::text::TextSize::Caption)
                        .announce(false)
                        .into_element(),
                );
            }
            let empty_radius = if default_highlight
                && matches!(origin, Origin::Dock)
                && p.style == Style::Desktop
                && self.metrics.islands.is_some()
            {
                self.dock
                    .geometry
                    .as_ref()
                    .map_or(radius, |g| g.radius.min(target / 2.0))
            } else {
                radius
            };
            let add = enz_pressable(
                container(content)
                    .center_x(if vertical { target } else { FIND_APPS_WIDTH })
                    .center_y(target)
                    .into(),
                Message::ShowLauncher,
            )
            .style(PressableStyle::Plain)
            .tokens(pressable)
            .padding(0)
            .radius(empty_radius)
            .aria_label(tr("shell-search-for-an-application-to-keep-in-the-dock"))
            .id("dock-add-app".into())
            .into_element();
            let add =
                enzui_widgets::tooltip::enz_tooltip(add, tr("shell-dock-find-apps-hint"), tooltip);
            return if matches!(origin, Origin::Bar) {
                self.dock_folder_target(add)
            } else {
                add
            };
        }
        let trailing = buttons.split_off(pinned);
        let mut items = Vec::new();
        if pinned > 0 {
            let mut list = enzui_widgets::reorder_list::enz_reorder_list(buttons, |request| {
                message(Event::Reorder(request))
            })
            .spacing(slots.spacing)
            .row_radius(radius)
            .row_inset(spacing.space1)
            .drag_through_children(true)
            .animate_changes();
            let span = slots.span(pinned);
            list = if vertical {
                list.width(Length::Fixed(target))
                    .height(Length::Fixed(span))
            } else {
                list.horizontal()
                    .width(if in_bar && p.indicator == Indicator::Count {
                        Length::Shrink
                    } else {
                        Length::Fixed(span)
                    })
                    .height(Length::Fixed(target))
            };
            items.push(("dock:pinned-strip".into(), list.into_element()));
        }
        let divider = self.dock.strip.divider.value() * slots.separator_span();
        if divider > 0.0 {
            let thickness = divider.min(enzui_theme::text_defaults::focus_ring_width() / 2.0);
            let mark = container(space())
                .width(if vertical { spacing.lg() } else { thickness })
                .height(if vertical { thickness } else { spacing.lg() })
                .style(move |_| container::Style {
                    background: Some(palette.border_strong.into()),
                    ..Default::default()
                });
            let rule = if vertical {
                container(mark).center_x(target).center_y(divider)
            } else {
                container(mark).center_x(divider).center_y(target)
            };
            items.push(("dock:divider".into(), rule.into()));
        }
        for (index, ((id, button), slot)) in trailing
            .into_iter()
            .zip(&self.dock.strip.trailing)
            .enumerate()
        {
            use enzui_widgets::motion::{MotionEffect, motion};
            let gap = if index == 0 && pinned == 0 {
                0.0
            } else {
                slots.spacing
            };
            let extent = slot.extent() * (button_width(&slot.group) + gap);
            let button =
                container(motion(button, !slot.departing, MotionEffect::Fade).enter_on_mount())
                    .clip(true);
            let button = if vertical {
                button.center_x(target).align_bottom(extent)
            } else {
                button.align_right(extent).center_y(target)
            };
            items.push((id, button.into()));
        }
        let icons: Element<'_, Message> = if vertical {
            enzui_widgets::keyed_row::keyed_column(items).into()
        } else {
            enzui_widgets::keyed_row::keyed_row(items).into()
        };
        if matches!(origin, Origin::Bar) {
            self.dock_folder_target(icons)
        } else {
            icons
        }
    }
    pub(crate) fn dock_view(&self) -> Element<'_, Message> {
        let Some(rest) = self.dock.geometry.as_ref() else {
            return space().into();
        };
        let p = &self.dock.preferences;
        let shares_sidebar = self.dock_shares_sidebar();
        let live = self.dock.strip.active().then(|| {
            let slots = dock::Slots::new(p, self.metrics, shares_sidebar);
            dock::geometry(
                p,
                self.dock.strip.span(slots, self.dock.pinned),
                rest.screen,
                self.bar,
                self.metrics,
                self.desktop_edges(),
                shares_sidebar,
            )
        });
        let g = live.as_ref().unwrap_or(rest);
        let progress = self.dock.reveal.value();
        let slide = 1.0 - progress;
        let offset = match p.position {
            Position::Left => iced::Vector::new(-(g.body.x + g.body.width) * slide, 0.0),
            Position::Right => iced::Vector::new((g.screen.width - g.body.x) * slide, 0.0),
            Position::Top => iced::Vector::new(0.0, -(g.body.y + g.body.height) * slide),
            _ => iced::Vector::new(0.0, (g.screen.height - g.body.y) * slide),
        };
        let palette = self.window_tokens(self.dock.window).palette;
        let input = self.dock.window.and_then(|id| self.surface_inputs.get(&id));
        let glass = self.dock_material().filter(|_| {
            input.is_some_and(|input| {
                input.glass_supported() && (!g.framed || input.glass_supports_corners())
            })
        });
        let preview = self.dock.preview.preview.as_ref().map(|p| p.bounds);
        let visible = g.body + offset;
        let radius = iced::border::Radius::new(g.radius);
        let radius = if g.framed {
            match p.position {
                Position::Left => radius.left(0.0),
                Position::Right => radius.right(0.0),
                Position::Top => radius.top(0.0),
                _ => radius.bottom(0.0),
            }
        } else {
            radius
        };
        let mask = if self.dock.menu.is_some() {
            enzui_shell::desktop_glass::InputMask::Full
        } else {
            let regions = if p.position == Position::Bar {
                preview.into_iter().map(|bounds| (bounds, 10.0)).collect()
            } else {
                dock::input_regions(g, p, true, self.dock.fullscreen, preview)
            };
            let mut regions: Vec<_> = regions
                .into_iter()
                .map(|(bounds, radius)| (bounds, radius.into()))
                .collect();
            if p.position != Position::Bar && !self.dock.hidden && progress > 0.0 {
                let input_radius = if p.style == Style::Minimal {
                    0.0.into()
                } else {
                    radius
                };
                regions.push((visible, input_radius));
            }
            enzui_shell::desktop_glass::InputMask::Rounded {
                regions,
                canvas: g.screen,
            }
        };
        let mut layers: Vec<Element<'_, Message>> =
            vec![space().width(Length::Fill).height(Length::Fill).into()];
        if p.position != Position::Bar {
            if progress > 0.0 {
                let icons = self.dock_icons(Origin::Dock, g.vertical);
                let width = (g.body.width - 2.0 * g.padding[1]).max(1.0);
                let height = (g.body.height - 2.0 * g.padding[0]).max(1.0);
                let mut scroll = enzui_widgets::scroll_view::enz_scroll_view(icons)
                    .width(Length::Shrink.max(width))
                    .height(Length::Shrink.max(height));
                if !g.vertical {
                    scroll = scroll.axis(enzui_widgets::scroll_view::Axis::Horizontal);
                }
                let shadow = if !g.framed
                    && p.style != Style::Minimal
                    && p.style != Style::Panel
                    && glass.is_none()
                    && self.desired_settings.bar_shadow > 0
                {
                    let (opacity, reach) = enzui_shell::desktop_frame::island_shadow_look(
                        self.desired_settings.bar_shadow,
                        self.chrome_light(),
                    );
                    iced::Shadow {
                        color: iced::Color {
                            a: opacity,
                            ..palette.shadow
                        },
                        blur_radius: reach,
                        offset: iced::Vector::ZERO,
                    }
                } else {
                    iced::Shadow::default()
                };
                let framed = g.framed;
                let contour = if self.chrome_light() {
                    palette.border_strong
                } else {
                    palette.border
                };
                let body = self.dock_folder_target(
                    container(crate::plugin_applications::dock_scroll::dock_scroll(
                        scroll.into_element(),
                        self.dock.dock_scroll.clone(),
                        !g.vertical,
                    ))
                    .padding(g.padding)
                    .center_x(g.body.width)
                    .center_y(g.body.height)
                    .style(move |_| container::Style {
                        background: (p.style != Style::Minimal && glass.is_none())
                            .then_some(palette.surface.into()),
                        border: iced::Border {
                            radius,
                            width: f32::from(
                                p.style != Style::Minimal && (!framed || glass.is_some()),
                            ),
                            color: contour,
                        },
                        shadow,
                        ..Default::default()
                    })
                    .into(),
                );
                let body = mouse_area(body)
                    .on_enter(message(Event::Hover(true, true)))
                    .on_exit(message(Event::Hover(false, true)));
                layers.push(
                    container(iced::widget::float(body).translate(move |_, _| offset))
                        .padding(iced::Padding::ZERO.top(g.body.y).left(g.body.x))
                        .into(),
                );
            }
            if !self.dock.fullscreen
                && matches!(
                    p.visibility,
                    dock::Visibility::Always | dock::Visibility::Overlap
                )
            {
                let sensor = container(space())
                    .width(g.sensor.width)
                    .height(g.sensor.height);
                layers.push(
                    container(
                        mouse_area(sensor)
                            .on_enter(message(Event::Hover(true, false)))
                            .on_exit(message(Event::Hover(false, false))),
                    )
                    .padding(iced::Padding::ZERO.top(g.sensor.y).left(g.sensor.x))
                    .into(),
                );
            }
        }
        if let Some(preview) = self.dock_preview_view() {
            layers.push(preview);
        }
        let canvas = iced::widget::stack(layers)
            .width(Length::Fill)
            .height(Length::Fill);
        let view = enz_optional_point_context_menu_host(
            canvas,
            self.dock.menu.as_ref().map(|m| (&m.model, &m.state)),
            |event| message(Event::Menu(event)),
        )
        .into();
        let Some(id) = self.dock.window else {
            return view;
        };
        let regions = if glass.is_some() && p.position != Position::Bar && progress > 0.0 {
            vec![(visible, radius)]
        } else {
            Vec::new()
        };
        enzui_shell::desktop_glass::fixed_glass_with_input(
            view,
            enzui_shell::desktop_glass::WindowGlass {
                input: input.cloned(),
                style: glass,
                tint: self.glass_tint(id, self.tokens().palette.surface),
                light_ink: self.window_ink(id),
                reduced_motion: self.tokens().motion.reduced_motion,
                shadow: (!g.framed && p.style != Style::Panel)
                    .then(|| {
                        enzui_shell::desktop_frame::bar_glass_shadow(
                            self.desired_settings.bar_shadow,
                            self.light,
                            p.position == Position::Bottom,
                        )
                    })
                    .flatten(),
                pending: false,
            },
            regions,
            mask,
        )
    }
    pub(crate) fn dock_search_host<'a>(
        &'a self,
        item: &enzui_shell::search::Item,
        action: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if !matches!(item.target, Some(Target::App(_)))
            || (!self.dock_active() && !self.dock_can_offer_opt_in())
        {
            return action;
        }
        let state = if self
            .dock
            .search_item
            .as_ref()
            .is_some_and(|known| known.id == item.id)
        {
            &self.dock.search_state
        } else {
            &self.dock.closed
        };
        let item = item.clone();
        enz_context_menu_host(action, &self.dock.search_model, state, move |event| {
            message(Event::Search(Box::new(item.clone()), event))
        })
        .into()
    }
    fn dock_can_offer_opt_in(&self) -> bool {
        !self.dock.search_saving
            && self.dock.search_offer.is_some()
            && self.dock_owner().is_none_or(|claim| {
                claim.owner.id == enzui_shell::plugins::applications::DOCK_PLUGIN
            })
    }

    fn dock_enable_from_search(&mut self, item: enzui_shell::search::Item) -> Task<Message> {
        if !self.dock_can_offer_opt_in() {
            return Task::none();
        }
        let Some((offer, settings)) = self
            .dock
            .search_offer
            .clone()
            .zip(self.settings_store.clone())
        else {
            return Task::none();
        };
        let id = item
            .dock_pin_id(|id| {
                offer.config()["pins"]
                    .as_array()
                    .is_some_and(|pins| pins.iter().any(|pin| pin == id))
            })
            .to_owned();
        self.dock.search_saving = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let store = enzui_shell::plugins::store::Store::for_settings(settings)?;
                    store.enable_dock_and_pin(&offer, id).map(|_| ())
                })
                .await
                .map_err(|_| tr("shell-application-surface-writer-stopped").to_owned())?
            },
            |result| message(Event::SearchSaved(result)),
        )
    }

    fn dock_search_menu(
        &mut self,
        item: enzui_shell::search::Item,
        event: ContextMenuEvent,
    ) -> Task<Message> {
        if let ContextMenuEvent::Invoked(invocation) = event {
            if !matches!(item.target, Some(Target::App(_))) {
                return Task::none();
            }
            let active = self.dock_active();
            if !active && !self.dock_can_offer_opt_in() {
                return Task::none();
            }
            let pin_id =
                item.dock_pin_id(|id| self.dock.preferences.pins.iter().any(|pin| pin == id));
            let pinned = active && self.dock.preferences.pins.iter().any(|id| id == pin_id);
            let can_pin = if active {
                pinned || self.dock.preferences.pins.len() < 32
            } else {
                self.dock
                    .search_offer
                    .as_ref()
                    .and_then(|record| record.config()["pins"].as_array())
                    .is_some_and(|pins| {
                        let contains = |id: &str| pins.iter().any(|pin| pin == id);
                        pins.len() < 32 || contains(item.dock_pin_id(contains))
                    })
            };
            self.dock.search_model = MenuModel::new(vec![
                MenuEntryModel::command("open", tr("shell-open")),
                MenuEntryModel::command(
                    if active { "pin" } else { "enable-and-pin" },
                    if pinned {
                        tr("shell-remove-from-dock")
                    } else if active {
                        tr("shell-keep-in-dock")
                    } else {
                        tr("shell-enable-dock-and-pin")
                    },
                )
                .icon(if pinned { "pin-off" } else { "pin" })
                .enabled(can_pin),
                MenuEntryModel::command("settings", tr("shell-dock-settings")),
            ]);
            let restore = if self.search.app_tiles_visible() {
                iced::advanced::widget::Id::from(format!("quick-app-{}", item.id))
            } else {
                enzui_core::identity::derive_id("quick-search-result", &item.id)
            };
            self.dock
                .search_state
                .open_with_restore(invocation, FocusRestoreTarget::Widget(restore));
            self.dock.search_item = Some(item);
            return Task::none();
        }
        if self
            .dock
            .search_item
            .as_ref()
            .is_none_or(|current| current.id != item.id)
        {
            return Task::none();
        }
        let update = apply_event_restoring::<Message>(
            &mut self.dock.search_model,
            &mut self.dock.search_state,
            &event,
        );
        let activated = update.activated().cloned();
        let action = match activated
            .as_ref()
            .map(enzui_core::menu_model::EntryId::as_str)
        {
            Some("pin") if self.dock_active() => {
                let id =
                    item.dock_pin_id(|id| self.dock.preferences.pins.iter().any(|pin| pin == id));
                let pinned = self.dock.preferences.pins.iter().any(|pin| pin == id);
                self.dock_save(Change::Pin(id.to_owned(), !pinned))
            }
            Some("enable-and-pin") => self.dock_enable_from_search(item),
            Some("open") => {
                let ticket = self.search.model.ticket(&item);
                self.search_message(crate::quick_search::Message::Activate(ticket))
            }
            Some("settings") if self.dock_active() => self.dock_plugin_settings(),
            Some("settings") => Task::done(Message::SettingsPage("section/dock/enabled")),
            _ => Task::none(),
        };
        if !self.dock.search_state.is_open() {
            self.dock.search_item = None;
        }
        update.finish(action)
    }
}
