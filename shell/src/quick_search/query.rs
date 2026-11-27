use super::{
    model::{FileKey, Message},
    support::{clipboard_connection, perform},
};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::search::{self, Category, DesktopAction, Item, Model, Target, files};
use enzui_shell::tr;
use iced::{Task, window};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

impl Shell {
    pub(crate) fn search_preferences(&mut self) -> Task<ShellMessage> {
        let mut preferences = (*self.desired_settings.quick_search).clone();
        preferences.enabled &=
            std::env::var("ENZUI_PLUGIN_ISOLATION").as_deref() == Ok("delegated");
        let changed = !self.search.configured || preferences != self.search.preferences;
        self.search.configured = true;
        if !preferences.animations || self.tokens().motion.reduced_motion {
            self.search.height_motion = None;
            self.search.height_hold = None;
            self.search.height_due = None;
            if self.search.pending_browse_collapse {
                self.search.pending_browse_collapse = false;
                self.search.browse_all_apps = false;
                self.search.app_scroll_carry = 0.0;
                self.search.model.set_category(None);
                self.search_recompute();
            }
        }
        if let Some(change) = self.search.pending_desktop.clone() {
            let mut test = self.desired_settings.clone();
            if test.preview(change).is_ok() && test == self.desired_settings {
                self.search.pending_desktop = None;
                self.search.feedback = Some(tr("shell-desktop-preference-saved").into());
            }
        }
        if !changed {
            let context = (
                self.desired_settings.light,
                self.snapshot.as_ref().is_some_and(|s| s.tiling),
                self.desired_settings.notifications.policy.dnd,
            );
            if self.search.showing && self.search.context != Some(context) {
                self.search_recompute();
            }
            return Task::none();
        }
        self.search.preferences = preferences.clone();
        self.search
            .provider_generation
            .fetch_add(1, Ordering::AcqRel);
        self.search.provider_results.clear();
        if let Some(change) = &self.search.pending_preference {
            let mut test = preferences.clone();
            if test.preview(change).is_ok() && test == preferences {
                self.search.pending_preference = None;
                self.search.feedback = Some(tr("shell-preference-saved").into());
            }
        }
        let clipboard = self.search.clipboard.configure(
            preferences.shows(Category::Clipboard),
            preferences.clipboard_generation,
            preferences.clipboard_minutes,
        );
        let clipboard = clipboard.map_or_else(Task::none, clipboard_connection);
        let key = preferences.shows(Category::Files).then(|| FileKey {
            revision: self.search.file_revision,
            roots: preferences.roots.clone(),
            exclusions: preferences.exclusions.clone(),
        });
        self.search_file_request(key, false);
        if self
            .search
            .model
            .category
            .is_some_and(|category| !preferences.shows(category))
        {
            self.search.model.set_category(None);
        }
        if !preferences.shows(Category::Apps) {
            self.search.apps = Arc::default();
            self.search.apps_ready = false;
            self.search.apps_generation = self.search.apps_generation.saturating_add(1);
            if let Some(cancel) = &self.search.apps_cancel {
                cancel.store(true, Ordering::Release);
            }
        }
        if !preferences.shows(Category::Windows) {
            self.search.windows.clear();
        }
        let close = if !preferences.enabled {
            self.search.showing = false;
            self.search.visible = false;
            self.search.preparing = false;
            if let Some(live) = &self.search.presentation {
                live.store(false, Ordering::Release);
            }
            self.search.epoch = self.search.epoch.saturating_add(1);
            self.search.apps = Arc::default();
            self.search.apps_ready = false;
            self.search.apps_generation = self.search.apps_generation.saturating_add(1);
            if let Some(cancel) = &self.search.apps_cancel {
                cancel.store(true, Ordering::Release);
            }
            self.search.windows.clear();
            self.search.model = Model::default();
            self.search.alias = None;
            self.search.preedit = None;
            self.search
                .window
                .take()
                .map_or_else(Task::none, window::close)
        } else {
            Task::none()
        };
        self.search_recompute();
        Task::batch([
            close,
            clipboard,
            self.search_scan_apps(),
            self.search_read_windows(),
            self.search_index(),
            self.search_file_query(),
        ])
    }
    pub(super) fn refresh_search_sources(&mut self) -> Task<ShellMessage> {
        Task::batch([
            self.search_check_activation(),
            self.search_icons_changed(),
            self.search_read_windows(),
        ])
    }

    pub(crate) fn refresh_overview_search(&mut self) -> Task<ShellMessage> {
        if std::mem::take(&mut self.search.overview_refresh_pending) && self.search.showing {
            self.refresh_search_sources()
        } else {
            Task::none()
        }
    }

    // Re-resolve application icons after the icon theme changed.
    pub(crate) fn search_icons_changed(&mut self) -> Task<ShellMessage> {
        self.search.apps_ready = false;
        self.search.apps_generation = self.search.apps_generation.saturating_add(1);
        if let Some(cancel) = &self.search.apps_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.search.apps_busy = false;
        self.search_scan_apps()
    }
    pub(super) fn search_scan_apps(&mut self) -> Task<ShellMessage> {
        if !self.search.preferences.shows(Category::Apps)
            || self.search.apps_busy
            || self.search.apps_ready
        {
            return Task::none();
        }
        self.search.apps_busy = true;
        let generation = self.search.apps_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.apps_cancel = Some(std::sync::Arc::clone(&cancel));
        let root = self.root.clone();
        perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    Arc::new(search::catalogue::scan_cancellable(
                        &root,
                        &search::catalogue::directories(),
                        &cancel,
                    ))
                })
                .await
                .map_err(|_| tr("shell-application-catalogue-stopped").to_owned())
            },
            move |result| Message::Apps(generation, result),
        )
    }
    pub(super) fn search_read_windows(&mut self) -> Task<ShellMessage> {
        if !self.search.showing
            || !self.search.preferences.shows(Category::Windows)
            || self.search.windows_busy
        {
            return Task::none();
        }
        self.search.windows_busy = true;
        let epoch = self.search.epoch;
        let compositor = self.compositor.clone();
        perform(
            async move { compositor.search_windows().await },
            move |result| Message::Windows(epoch, result),
        )
    }
    pub(super) fn search_file_request(&mut self, mut key: Option<FileKey>, force: bool) {
        if !force && key == self.search.file_wanted {
            return;
        }
        self.search.file_error = None;
        if let Some(revision) = self.search.file_revision.checked_add(1) {
            self.search.file_revision = revision;
            if let Some(key) = &mut key {
                key.revision = revision;
            }
        } else {
            self.search.file_error = Some(tr("shell-file-search-generations-exhausted").into());
            key = None;
        }
        if let Some(cancel) = &self.search.file_cancel {
            cancel.store(true, Ordering::Release);
        }
        self.search.file_wanted = key;
        self.search.file_built = None;
        self.search.file_index = None;
        self.search.file_results.clear();
        self.search.file_query = None;
    }
    pub(super) fn search_index(&mut self) -> Task<ShellMessage> {
        let Some(key) = self.search.file_wanted.clone() else {
            return Task::none();
        };
        if self.search.file_busy || self.search.file_built.as_ref() == Some(&key) {
            return Task::none();
        }
        self.search.file_busy = true;
        self.search.file_error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search.file_cancel = Some(std::sync::Arc::clone(&cancel));
        let input = key.clone();
        perform(
            async move {
                // Use a dedicated thread: background CPU/I/O priority must not leak into pooled
                // blocking work.
                let (sender, receiver) = tokio::sync::oneshot::channel();
                std::thread::Builder::new()
                    .name("file-index".into())
                    .spawn(move || {
                        enzui_core::background_io::lower_current_thread();
                        let _ = sender.send(files::build(&input.roots, &input.exclusions, &cancel));
                    })
                    .map_err(|_| tr("shell-file-indexer-could-not-start").to_owned())?;
                receiver
                    .await
                    .map_err(|_| tr("shell-file-indexer-stopped").to_owned())
            },
            move |result| Message::Indexed(key.clone(), result),
        )
    }
    pub(super) fn search_file_query(&mut self) -> Task<ShellMessage> {
        if self.search.file_query_busy
            || !self.search.showing
            || !self.search.preferences.shows(Category::Files)
        {
            return Task::none();
        }
        let query = self.search.model.query.clone();
        let category = self.search.model.category;
        if query.chars().count() < 2 || category.is_some_and(|category| category != Category::Files)
        {
            self.search.file_results.clear();
            return Task::none();
        }
        if self.search.file_query.as_ref() == Some(&(query.clone(), category)) {
            return Task::none();
        }
        let Some(index) = &self.search.file_index else {
            return Task::none();
        };
        let Some(key) = self.search.file_built.clone() else {
            return Task::none();
        };
        self.search.file_query_busy = true;
        let items = std::sync::Arc::clone(&index.items);
        let input = query.clone();
        let preferences = self.search.preferences.clone();
        perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    search::rank(items.iter(), &input, Some(Category::Files), &preferences)
                })
                .await
                .map_err(|_| tr("shell-file-query-stopped").to_owned())
            },
            move |result| Message::FileQuery(key.clone(), query.clone(), category, result),
        )
    }
    pub(super) fn search_commands(&self) -> Vec<Item> {
        let mut items = Vec::new();
        use enzui_shell::settings_catalog::{self, SearchDestination};

        let platform = self.system.platform.as_ref();
        let shows = |route: &str| enzui_shell::platform::shows(platform, route);
        let query = &self.search.model.query;
        for group in settings_catalog::search_groups(query, |page, entry| {
            shows(page.page()) && entry.is_none_or(|entry| shows(entry.route))
        })
        .into_iter()
        .take(settings_catalog::DESKTOP_GROUP_LIMIT)
        {
            let (subtitle, target) = match group.destination {
                SearchDestination::Page(page) => {
                    (tr("shell-settings").into(), Target::Settings(page.page()))
                }
                SearchDestination::Section(route) => (
                    tr!("shell-settings-2", "arg0" => group.category.title().to_owned()),
                    Target::SettingsSection(route),
                ),
            };
            items.push(
                Item::new(
                    format!("setting:{}", group.destination.id()),
                    Category::Actions,
                    group.title,
                    subtitle,
                    tr("shell-open"),
                    group.category.icon(),
                    Some(target),
                )
                .with_settings_rank(group.rank),
            );
        }
        if let Some(id) =
            search::alias_target(query, self.search.model.category, &self.search.preferences)
            && let Some(route) = id.strip_prefix("setting:")
            && shows(route)
            && !items.iter().any(|item| item.id == *id)
        {
            // Explicit quick keys keep their saved destinations when controls are grouped.
            if let Some(section) = SearchDestination::section(route)
                && shows(section.category.page())
                && shows(section.route)
            {
                items.push(Item::new(
                    id.clone(),
                    Category::Actions,
                    tr(section.label),
                    section.category.title(),
                    tr("shell-open"),
                    section.category.icon(),
                    Some(Target::SettingsSection(section.route)),
                ));
            } else if let Some(entry) = settings_catalog::find(route)
                && shows(entry.category().page())
            {
                items.push(Item::new(
                    id.clone(),
                    Category::Actions,
                    entry.title(),
                    entry.category().title(),
                    tr("shell-open"),
                    entry.category().icon(),
                    Some(Target::Settings(entry.route)),
                ));
            } else if let Some(page) = settings_catalog::category(route)
                && shows(page.page())
            {
                items.push(Item::new(
                    id.clone(),
                    Category::Actions,
                    page.title(),
                    tr("shell-settings"),
                    tr("shell-open"),
                    page.icon(),
                    Some(Target::Settings(page.page())),
                ));
            }
        }
        let light = self.desired_settings.light;
        let tiled = self.snapshot.as_ref().is_some_and(|s| s.tiling);
        let dnd = self.desired_settings.notifications.policy.dnd;
        for (id, title, subtitle, icon, action) in [
            (
                "appearance",
                if light {
                    tr("shell-use-dark-appearance")
                } else {
                    tr("shell-use-light-appearance")
                },
                tr("shell-changes-this-desktop-s-theme"),
                "palette",
                DesktopAction::Light(!light),
            ),
            (
                "tiling",
                if tiled {
                    tr("shell-use-floating-windows")
                } else {
                    tr("shell-tile-desktop-windows")
                },
                tr("shell-changes-the-window-layout-of-this-desktop"),
                "panels-top-left",
                DesktopAction::Tiling(!tiled),
            ),
            (
                "dnd",
                if dnd {
                    tr("shell-turn-off-do-not-disturb")
                } else {
                    tr("shell-turn-on-do-not-disturb")
                },
                tr("shell-changes-notification-delivery-for-this-desktop"),
                "bell-off",
                DesktopAction::Dnd(!dnd),
            ),
            (
                "notifications",
                tr("shell-notification-center-2"),
                tr("shell-read-notifications-and-history"),
                "bell",
                DesktopAction::NotificationCenter,
            ),
            (
                "controls",
                tr("shell-control-center-2"),
                tr("shell-review-network-audio-and-power-controls"),
                "sliders-horizontal",
                DesktopAction::ControlCenter,
            ),
        ] {
            let verb = match action {
                DesktopAction::Light(_) | DesktopAction::Tiling(_) | DesktopAction::Dnd(_) => {
                    tr("shell-change")
                }
                _ => tr("shell-open"),
            };
            items.push(Item::new(
                format!("action:{id}"),
                Category::Actions,
                title,
                subtitle,
                verb,
                icon,
                Some(Target::Desktop(action)),
            ));
        }
        items
    }
    pub(crate) fn search_recompute(&mut self) {
        if !self.search.showing {
            self.search.model.results.clear();
            self.search.model.selected = None;
            return;
        }
        if self.search.alias.is_some() {
            return;
        }
        if self.search.model.query.trim().is_empty()
            && (self.search.model.category.is_none() || self.search.browse_all_apps)
        {
            if !self.search.blank_apps() {
                self.search.top_apps.clear();
                self.search.model.replace(Vec::new());
                return;
            }
            let installed =
                search::browse_installed_apps(self.search.apps.iter(), &self.search.preferences);
            self.search.top_apps = installed.iter().take(search::TOP_APPS).cloned().collect();
            self.search.model.replace(if self.search.browse_all_apps {
                // The full list is a catalogue to scan, so it is alphabetical;
                // usage ranking only chooses the top apps.
                let mut installed = installed;
                installed.sort_by_cached_key(|item| item.title.to_lowercase());
                installed
            } else {
                installed.into_iter().take(search::TOP_APPS).collect()
            });
            let capacity = self.search_capacity();
            if self.search.browse_all_apps {
                self.search.model.window_start = self
                    .search
                    .model
                    .window_start
                    .min(self.search.model.results.len().saturating_sub(capacity));
            } else {
                self.search.model.visible(capacity);
            }
            return;
        }
        self.search.context = Some((
            self.desired_settings.light,
            self.snapshot.as_ref().is_some_and(|s| s.tiling),
            self.desired_settings.notifications.policy.dnd,
        ));
        let begin = Instant::now();
        let commands = self.search_commands();
        let mut base = search::rank(
            self.search
                .apps
                .iter()
                .chain(self.search.windows.iter())
                .chain(commands.iter())
                .chain(
                    self.search
                        .clipboard
                        .entries
                        .iter()
                        .map(|entry| &entry.item),
                ),
            &self.search.model.query,
            self.search.model.category,
            &self.search.preferences,
        );
        if self.search.file_query.as_ref()
            == Some(&(self.search.model.query.clone(), self.search.model.category))
        {
            base.extend(self.search.file_results.clone());
        }
        if let Some(id) = search::alias_target(
            &self.search.model.query,
            self.search.model.category,
            &self.search.preferences,
        ) && !base.iter().any(|item| &item.id == id)
        {
            base.insert(
                0,
                Item::new(
                    "alias-unavailable",
                    Category::Actions,
                    tr("shell-quick-key-unavailable"),
                    tr("shell-the-app-is-missing-or-its-search-source-is-disabled"),
                    tr("shell-unavailable"),
                    "keyboard",
                    None,
                ),
            );
        }
        let utility = search::utility_results(
            &self.search.model.query,
            self.search.model.category,
            &self.search.preferences,
        );
        let mut results: Vec<_> = utility
            .iter()
            .filter(|item| item.category == Category::Calculator)
            .cloned()
            .collect();
        results.extend(base);
        if !self.search.model.query.trim().is_empty() {
            results.sort_by_key(|item| !item.is_installed_app());
        }
        // Apps' own results follow the desktop's, keep their room, and come
        // before the web fallback.
        let from_apps = self
            .search
            .provider_results
            .len()
            .min(search::MAX_RESULTS / 2);
        let fallback: Vec<_> = utility
            .into_iter()
            .filter(|item| item.category != Category::Calculator)
            .collect();
        results.truncate(search::MAX_RESULTS.saturating_sub(from_apps + fallback.len()));
        results.extend(self.search.provider_results.iter().take(from_apps).cloned());
        results.extend(fallback);
        results.truncate(search::MAX_RESULTS);
        self.search
            .model
            .replace(search::group_by_category(results));
        let capacity = self.search_capacity();
        self.search.model.visible(capacity);
        self.search.last_query_us = begin.elapsed().as_micros();
    }
}
