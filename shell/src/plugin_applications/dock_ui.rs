use crate::{Message, Shell, control_center::TileMenu, window_settings};
use enzui_core::{
    context_menu::InvocationKind,
    menu_model::{CheckState, EntryId, MenuEntryModel, MenuModel},
};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    dock::{self, Action, App, Builtin, Change, Geometry, Group, Position, Snapshot, Style},
    search::{AppTarget, Target, WindowTarget},
    settings::QuickAction,
};
use enzui_widgets::{
    context_menu_host::FocusRestoreTarget,
    file_drop::{FileDrop, enz_file_drop_target_with_receipt},
};
use enzui_widgets::{
    context_menu_host::{
        ContextMenuEvent, ContextMenuState, apply_event_restoring, apply_event_restoring_with,
        enz_context_menu_host, enz_optional_point_context_menu_host,
    },
    toolbar_icon::{ToolbarIconTokens, enz_toolbar_icon},
};
use iced::{
    Element, Length, Subscription, Task,
    widget::{column, container, mouse_area, row, space},
    window,
};
use std::{sync::Arc, time::Duration};

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
    Hide(u64),
    Click(String),
    DropFolders(FileDrop),
    Folders(enzui_shell::plugins::service::Owner, FileDrop, Vec<String>),
    Invoke(String, Origin, ContextMenuEvent),
    Menu(ContextMenuEvent),
    Anchor(u64, Result<iced::Point, String>),
    IconBounds(u64, String, Origin, Option<iced::Rectangle>),
    Done(u64, Result<(), String>),
    Search(Box<enzui_shell::search::Item>, ContextMenuEvent),
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
    pub preview: crate::dock_hover::State,
    pub window: Option<window::Id>,
    pub geometry: Option<Geometry>,
    pub snapshot: Option<Snapshot>,
    pub apps: Arc<[App]>,
    pub entries: Vec<Entry>,
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
    pub hide_serial: u64,
    pub hidden: bool,
    pub fullscreen: bool,
    pub error: Option<String>,
    pending: Option<(String, Origin, InvocationKind)>,
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
    )>,
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
            preview: Default::default(),
            window: None,
            geometry: None,
            snapshot: None,
            apps: Arc::default(),
            entries: Vec::new(),
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
            hide_serial: 0,
            hidden: false,
            fullscreen: false,
            error: None,
            pending: None,
            sent_policy: None,
            layout: None,
            search_item: None,
            search_state: ContextMenuState::closed(),
        }
    }
}
fn model(group: &Group, p: &dock::Preferences) -> MenuModel {
    if let Some(builtin) = &group.builtin {
        let mut rows = vec![MenuEntryModel::command("open", &group.title)];
        if matches!(builtin, Builtin::Folder(_)) {
            rows.push(MenuEntryModel::command("pin", tr("shell-remove-from-dock")));
        }
        rows.push(MenuEntryModel::command(
            "settings",
            tr("shell-plugin-settings"),
        ));
        return MenuModel::new(rows);
    }
    let mut rows = vec![
        MenuEntryModel::command(
            "open",
            if group.windows.is_empty() {
                tr("shell-open")
            } else {
                tr("shell-show-windows")
            },
        )
        .enabled(group.app.is_some() || !group.windows.is_empty()),
        MenuEntryModel::command("new", if matches!(group.app.as_ref(), Some(AppTarget::Prepared(id)) if id != "enzui-settings" && id != "enzui-assistant") { tr("shell-new-window") } else { tr("shell-launch-application") }).enabled(group.app.is_some()),
        MenuEntryModel::separator("windows"),
    ];
    for w in group.windows.iter().take(16) {
        rows.push(
            MenuEntryModel::command(format!("window:{}", w.target.stable_id), &w.title)
                .description(if w.minimized {
                    tr("shell-minimized")
                } else {
                    tr("shell-show-window")
                })
                .check(if w.active {
                    CheckState::Checked
                } else {
                    CheckState::Unchecked
                }),
        );
    }
    rows.extend([
        MenuEntryModel::command("minimize", tr("shell-minimize-windows")).enabled(
            group
                .windows
                .iter()
                .any(|w| !w.minimized && w.target.sidebar.is_none()),
        ),
        MenuEntryModel::command("close", tr("shell-close-all-windows"))
            .enabled(!group.windows.is_empty()),
        MenuEntryModel::separator("pinning"),
        MenuEntryModel::command(
            "pin",
            if group.pinned {
                tr("shell-remove-from-dock")
            } else {
                tr("shell-keep-in-dock")
            },
        )
        .enabled(group.pinned || (group.app.is_some() && p.pins.len() < 32))
        .check(if group.pinned {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        }),
        MenuEntryModel::command("earlier", tr("shell-move-earlier")).enabled(
            p.pins
                .iter()
                .position(|id| id == &group.id)
                .is_some_and(|i| i > 0),
        ),
        MenuEntryModel::command("later", tr("shell-move-later")).enabled(
            p.pins
                .iter()
                .position(|id| id == &group.id)
                .is_some_and(|i| i + 1 < p.pins.len()),
        ),
        MenuEntryModel::separator("preferences"),
        MenuEntryModel::command("settings", tr("shell-plugin-settings")),
    ]);
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
        let layout = (self.bar, self.metrics, self.desktop_edges());
        let layout_changed = self.dock.layout.replace(layout) != Some(layout);
        if !owner_changed && self.dock.preferences == desired {
            if layout_changed && self.dock.preferences.enabled {
                self.dock.preview.clear();
                self.dock_rebuild();
            }
            return Task::none();
        }
        let was = self.dock.preferences.enabled;
        let pins_changed = self.dock.preferences.pins != desired.pins;
        self.dock.preferences = desired;
        self.dock.generation = self.dock.generation.wrapping_add(1);
        if owner_changed || was != self.dock.preferences.enabled {
            self.dock.badges_generation = self.dock.badges_generation.wrapping_add(1);
            self.dock.badges.clear();
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
            self.dock.entries.clear();
            self.dock.apps = Arc::default();
            self.dock.snapshot = None;
            self.dock.hidden = true;
            if let Some(id) = self.dock.window.take() {
                if let Some(input) = self.surface_inputs.remove(&id) {
                    input.invalidate();
                }
                return window::close(id).chain(self.dock_publish());
            }
            return Task::none();
        }
        if !was || owner_changed || self.dock.apps.is_empty() || pins_changed {
            self.dock.apps_busy = false;
            self.dock_load_apps()
        } else {
            self.dock_refresh()
        }
    }
    pub(crate) fn dock_publish(&mut self) -> Task<Message> {
        let Some(geometry) = self.dock.geometry.as_ref() else {
            return Task::none();
        };
        if self.dock.preferences.enabled
            && self
                .dock
                .window
                .is_none_or(|id| !self.surface_inputs.contains_key(&id))
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
            |result| message(Event::Done(0, result)),
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
    pub(crate) fn dock_rebuild(&mut self) {
        let Some(snapshot) = self.dock.snapshot.as_ref() else {
            return;
        };
        let p = &self.dock.preferences;
        let groups = dock::groups(p, &self.dock.apps, &snapshot.windows, snapshot.desktop);
        self.dock.entries = groups
            .into_iter()
            .map(|group| Entry {
                model: model(&group, p),
                group,
            })
            .collect();
        self.dock
            .preview
            .update_windows(&self.dock.entries, snapshot.screen);
        let mut geometry = dock::geometry(
            p,
            self.dock.entries.len(),
            snapshot.screen,
            self.bar,
            self.metrics,
            self.desktop_edges(),
        );
        geometry.output.clone_from(&snapshot.output);
        geometry.output_id = snapshot.output_id;
        geometry.desktop = snapshot.desktop;
        self.dock.fullscreen = dock::fullscreen(&geometry, &snapshot.windows);
        if self.dock.fullscreen {
            self.dock.preview.clear();
            self.dock.hovered = false;
            self.dock.hover_body = false;
            self.dock.hover_sensor = false;
            self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
        }
        self.dock.hidden = (self.dock.fullscreen
            || dock::obscured(p, &geometry, &snapshot.windows) && !self.dock.hovered)
            && self.dock.menu.is_none()
            && self.dock.preview.preview.is_none();
        self.dock.geometry = Some(geometry);
    }
    pub(crate) fn dock_subscription(&self) -> Subscription<Message> {
        if !self.dock.preferences.enabled {
            return Subscription::none();
        }
        let generation = self.dock.badges_generation;
        Subscription::batch([
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
    pub(crate) fn dock_event(&mut self, event: Event) -> Task<Message> {
        if !self.dock_active()
            && matches!(
                &event,
                Event::Click(_)
                    | Event::Invoke(..)
                    | Event::Search(..)
                    | Event::DropFolders(_)
                    | Event::Folders(..)
            )
        {
            return Task::none();
        }
        match event {
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
                    Ok(snapshot) => self.dock.snapshot = Some(snapshot),
                    Err(error) => {
                        self.dock.error = Some(error);
                        return Task::none();
                    }
                }
                let old = self.dock.geometry.clone();
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
                    if old.as_ref().is_none_or(|old| {
                        old.screen != geometry.screen || old.output != geometry.output
                    }) {
                        tasks.push(window::resize(id, geometry.screen));
                    }
                    let compositor = self.compositor.clone();
                    tasks.push(Task::perform(
                        async move { compositor.place_dock(&geometry).await },
                        move |r| message(Event::Done(0, r)),
                    ));
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
                if self.dock.fullscreen && self.dock.menu.is_none() {
                    return Task::none();
                }
                if body {
                    self.dock.hover_body = hovered;
                } else {
                    self.dock.hover_sensor = hovered;
                }
                let hovered = self.dock.hover_body || self.dock.hover_sensor;
                self.dock.hovered = hovered;
                self.dock.hide_serial = self.dock.hide_serial.wrapping_add(1);
                let serial = self.dock.hide_serial;
                if hovered {
                    self.dock.hidden = false;
                    Task::none()
                } else {
                    Task::perform(
                        async { tokio::time::sleep(Duration::from_millis(400)).await },
                        move |()| message(Event::Hide(serial)),
                    )
                }
            }
            Event::Hide(serial) => {
                if serial == self.dock.hide_serial
                    && !self.dock.hovered
                    && self.dock.menu.is_none()
                    && self.dock.preview.preview.is_none()
                {
                    self.dock_rebuild();
                }
                Task::none()
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
                self.dock_launch(group.app)
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

    fn dock_launch(&mut self, app: Option<AppTarget>) -> Task<Message> {
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
        let root = self.root.clone();
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                if !authority.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(tr("shell-plugin-authority-changed").into());
                }
                compositor
                    .execute_search_target(&root, Target::App(app))
                    .await
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
                for window in windows.into_iter().take(64) {
                    if !authority.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(tr("shell-plugin-authority-changed").into());
                    }
                    compositor.dock_window_action(window, action).await?;
                }
                Ok(())
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
            "new" => self.dock_launch(group.app),
            "open" => {
                if let Some(w) = group.windows.first() {
                    self.dock_windows(vec![w.target.clone()], "focus")
                } else {
                    self.dock_launch(group.app)
                }
            }
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
    pub(crate) fn dock_icons(&self, origin: Origin, vertical: bool) -> Element<'_, Message> {
        let palette = self
            .window_tokens(match origin {
                Origin::Bar => Some(self.panel),
                Origin::Dock => self.dock.window,
            })
            .palette;
        let p = &self.dock.preferences;
        let size = if matches!(origin, Origin::Bar) {
            f32::from(p.icon_size)
                .min(f32::from(self.metrics.bar) - 16.0)
                .max(12.0)
        } else {
            f32::from(p.icon_size)
        };
        use iced::widget::tooltip::Position as Tooltip;
        let tooltip = match p.position {
            Position::Left => Tooltip::Right,
            Position::Right => Tooltip::Left,
            Position::Top => Tooltip::Bottom,
            Position::Bar if self.bar == enzui_shell::settings::BarEdge::Top => Tooltip::Bottom,
            _ => Tooltip::Top,
        };
        let mut buttons = Vec::new();
        for entry in &self.dock.entries {
            let group = &entry.group;
            let id = group.id.clone();
            let active = group.windows.iter().any(|w| w.active);
            let running = !group.windows.is_empty();
            let badge = enzui_shell::launcher_badges::group_id(group)
                .and_then(|id| self.dock.badges.get(&id))
                .copied()
                .unwrap_or_default();
            let urgent = badge.urgent || group.windows.iter().any(|window| window.urgent);
            let icon = match &group.image {
                Some(image) => enzui_shell::icon_theme::source(image).into_element(size),
                None => {
                    let color = group
                        .app
                        .as_ref()
                        .and_then(|target| match target {
                            AppTarget::Prepared(id) => {
                                enzui_shell::catalog::APPS.iter().find(|app| app.id == id)
                            }
                            _ => None,
                        })
                        .map_or(palette.text, |app| {
                            iced::Color::from_rgb8(app.color[0], app.color[1], app.color[2])
                        });
                    enzui_widgets::icon::enz_icon(&group.icon)
                        .size(size)
                        .tint(color)
                        .into_element()
                }
            };
            let icon = container(icon)
                .center_x(size + 6.0)
                .center_y(size + 6.0)
                .style(move |_| container::Style {
                    border: iced::Border {
                        width: if urgent { 1.5 } else { 0.0 },
                        color: palette.warning,
                        radius: f32::from(p.highlight_radius).into(),
                    },
                    ..Default::default()
                });
            let mut layers: Vec<Element<'_, Message>> = vec![icon.into()];
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
                layers.push(
                    container(indicator)
                        .align_right(size + 6.0)
                        .height(size + 6.0)
                        .align_y(iced::alignment::Vertical::Top)
                        .into(),
                );
            }
            let label = if group.builtin.is_some() {
                group.title.clone()
            } else {
                tr!("shell-windows-2", "arg0" => group.title.clone(), "arg1" => format!("{:}", group.windows.len()), "arg2" => (if group.pinned { tr("shell-kept-in-dock") } else { "" }).to_owned(), "arg3" => (if badge.count_visible && badge.count > 0 {format!(" · {} unread",badge.count)} else {String::new()}).clone(), "arg4" => (if urgent {tr("shell-needs-attention")} else {""}).to_owned())
            };
            let button = enzui_widgets::pressable::enz_pressable(
                iced::widget::stack(layers)
                    .width(size + 6.0)
                    .height(size + 6.0)
                    .into(),
                message(Event::Click(id.clone())),
            )
            .style(enzui_widgets::pressable::PressableStyle::Plain)
            .padding(0)
            .radius(f32::from(p.highlight_radius))
            .selected(active)
            .aria_label(label)
            .id(format!("dock-app-{id}").into())
            .enabled(group.builtin.is_some() || group.app.is_some() || running)
            .into_element();
            let dots = (0..if running {
                group.windows.len().min(3)
            } else {
                1
            })
                .map(|_| {
                    container(space())
                        .width(3)
                        .height(3)
                        .style(move |_| container::Style {
                            background: running.then_some(
                                if active {
                                    palette.accent
                                } else {
                                    palette.text_muted
                                }
                                .into(),
                            ),
                            border: iced::Border {
                                radius: 1.5.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .into()
                })
                .collect::<Vec<Element<'_, Message>>>();
            let dots: Element<'_, Message> = if vertical {
                column(dots).spacing(2).into()
            } else {
                row(dots).spacing(2).into()
            };
            let marker: Element<'_, Message> = container(dots)
                .center_x(if vertical { 3.0 } else { size + 6.0 })
                .center_y(if vertical { size + 6.0 } else { 3.0 })
                .into();
            let button: Element<'_, Message> = match p.position {
                Position::Left => row![button, marker]
                    .spacing(1)
                    .width(size + 10.0)
                    .height(size + 6.0)
                    .into(),
                Position::Right => row![marker, button]
                    .spacing(1)
                    .width(size + 10.0)
                    .height(size + 6.0)
                    .into(),
                Position::Bottom => column![marker, button]
                    .spacing(1)
                    .width(size + 6.0)
                    .height(size + 10.0)
                    .into(),
                Position::Bar if self.bar == enzui_shell::settings::BarEdge::Bottom => {
                    column![marker, button]
                        .spacing(1)
                        .width(size + 6.0)
                        .height(size + 10.0)
                        .into()
                }
                _ => column![button, marker]
                    .spacing(1)
                    .width(size + 6.0)
                    .height(size + 10.0)
                    .into(),
            };
            let hover_id = id.clone();
            let leave_id = id.clone();
            let scroll_id = id.clone();
            let button = mouse_area(button)
                .on_enter(message(Event::Preview(crate::dock_hover::Event::Enter(
                    hover_id, origin,
                ))))
                .on_exit(message(Event::Preview(crate::dock_hover::Event::Leave(
                    leave_id,
                ))));
            let button = if group.windows.len() > 1 {
                button.on_scroll(move |delta| {
                    message(Event::Preview(crate::dock_hover::Event::Cycle(
                        scroll_id.clone(),
                        delta,
                    )))
                })
            } else {
                button
            };
            let button: Element<'_, Message> = if running && p.hover_previews {
                button.into()
            } else {
                enzui_widgets::tooltip::enz_tooltip(button.into(), group.title.clone(), tooltip)
            };
            buttons.push(
                enz_context_menu_host(button, &entry.model, &self.dock.closed, move |event| {
                    message(Event::Invoke(id.clone(), origin, event))
                })
                .into(),
            );
        }
        if buttons.is_empty() {
            buttons.push(
                enz_toolbar_icon("plus", Message::ShowLauncher)
                    .tokens(ToolbarIconTokens::from_palette(&palette))
                    .aria_label(tr("shell-search-for-an-application-to-keep-in-the-dock"))
                    .into_element(),
            );
        }
        let spacing = f32::from(p.icon_spacing(self.metrics));
        let icons: Element<'_, Message> = if vertical {
            column(buttons).spacing(spacing).into()
        } else {
            row(buttons).spacing(spacing).into()
        };
        if matches!(origin, Origin::Bar) {
            self.dock_folder_target(icons)
        } else {
            icons
        }
    }
    pub(crate) fn dock_view(&self) -> Element<'_, Message> {
        let Some(g) = self.dock.geometry.as_ref() else {
            return space().into();
        };
        let p = &self.dock.preferences;
        let palette = self.window_tokens(self.dock.window).palette;
        let input = self.dock.window.and_then(|id| self.surface_inputs.get(&id));
        let glass = self.chrome_glass().filter(|_| {
            p.style == Style::Desktop
                && self.metrics.islands.is_some()
                && input.is_some_and(enzui_shell::surface_input::SurfaceInput::glass_supported)
        });
        let menu_open = self.dock.menu.is_some();
        if let Some(input) = input {
            if menu_open {
                input.reset();
            } else if p.position == Position::Bar {
                input.rounded_rectangles(
                    &self
                        .dock
                        .preview
                        .preview
                        .as_ref()
                        .map(|p| vec![p.bounds])
                        .unwrap_or_default(),
                    10.0,
                );
            } else {
                let regions = dock::input_regions(
                    g,
                    p,
                    self.dock.hidden,
                    self.dock.fullscreen,
                    self.dock.preview.preview.as_ref().map(|p| p.bounds),
                );
                input.rounded_regions(&regions);
            }
        }
        let mut layers: Vec<Element<'_, Message>> =
            vec![space().width(Length::Fill).height(Length::Fill).into()];
        if p.position != Position::Bar {
            if !self.dock.hidden {
                let icons = self.dock_icons(Origin::Dock, g.vertical);
                let width = (g.body.width - 2.0 * g.padding[1]).max(1.0);
                let height = (g.body.height - 2.0 * g.padding[0]).max(1.0);
                let mut scroll = enzui_widgets::scroll_view::enz_scroll_view(icons)
                    .width(Length::Shrink.max(width))
                    .height(Length::Shrink.max(height));
                if !g.vertical {
                    scroll = scroll.axis(enzui_widgets::scroll_view::Axis::Horizontal);
                }
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
                let rim = if self.light {
                    iced::Color::from_rgba(0.0, 0.0, 0.0, 0.08)
                } else {
                    iced::Color::from_rgba(1.0, 1.0, 1.0, 0.06)
                };
                let body = self.dock_folder_target(
                    container(scroll.into_element())
                        .padding(g.padding)
                        .center_x(g.body.width)
                        .center_y(g.body.height)
                        .style(move |_| container::Style {
                            background: (p.style != Style::Minimal && glass.is_none())
                                .then_some(palette.surface.into()),
                            border: iced::Border {
                                radius,
                                width: f32::from(glass.is_some()),
                                color: rim,
                            },
                            ..Default::default()
                        })
                        .into(),
                );
                layers.push(
                    container(
                        mouse_area(body)
                            .on_enter(message(Event::Hover(true, true)))
                            .on_exit(message(Event::Hover(false, true))),
                    )
                    .padding(iced::Padding::ZERO.top(g.body.y).left(g.body.x))
                    .into(),
                );
            }
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
        let regions = if glass.is_some() && p.position != Position::Bar && !self.dock.hidden {
            vec![(g.body, g.radius)]
        } else {
            Vec::new()
        };
        enzui_shell::desktop_glass::fixed_glass(
            view,
            enzui_shell::desktop_glass::WindowGlass {
                input: input.cloned(),
                style: glass,
                tint: self.glass_tint(id, self.tokens().palette.surface),
                light_ink: self.window_ink(id),
                reduced_motion: self.tokens().motion.reduced_motion,
                shadow: enzui_shell::desktop_frame::bar_glass_shadow(
                    self.desired_settings.bar_shadow,
                    self.light,
                    p.position == Position::Bottom,
                ),
                pending: false,
            },
            regions,
        )
    }
    pub(crate) fn dock_search_host<'a>(
        &'a self,
        item: &enzui_shell::search::Item,
        action: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if !self.dock_active() || !matches!(item.target, Some(Target::App(_))) {
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
    fn dock_search_menu(
        &mut self,
        item: enzui_shell::search::Item,
        event: ContextMenuEvent,
    ) -> Task<Message> {
        if let ContextMenuEvent::Invoked(invocation) = event {
            if !matches!(item.target, Some(Target::App(_))) {
                return Task::none();
            }
            let pinned = self.dock.preferences.pins.contains(&item.id);
            self.dock.search_model = MenuModel::new(vec![
                MenuEntryModel::command("open", tr("shell-open")),
                MenuEntryModel::command(
                    "pin",
                    if pinned {
                        tr("shell-remove-from-dock")
                    } else if self.dock.preferences.enabled {
                        tr("shell-keep-in-dock")
                    } else {
                        tr("shell-keep-application-in-dock")
                    },
                )
                .enabled(pinned || self.dock.preferences.pins.len() < 32),
                MenuEntryModel::command("settings", tr("shell-plugin-settings")),
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
            Some("pin") => {
                let pinned = self.dock.preferences.pins.contains(&item.id);
                let change = if !pinned && !self.dock.preferences.enabled {
                    Change::EnableAndPin(item.id)
                } else {
                    Change::Pin(item.id, !pinned)
                };
                self.dock_save(change)
            }
            Some("open") => {
                let ticket = self.search.model.ticket(&item);
                self.search_message(crate::quick_search::Message::Activate(ticket))
            }
            Some("settings") => self.dock_plugin_settings(),
            _ => Task::none(),
        };
        if !self.dock.search_state.is_open() {
            self.dock.search_item = None;
        }
        update.finish(action)
    }
}
