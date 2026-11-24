use super::{Message as AppMessage, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::plugins::{
    discovery::{self, Candidate},
    manifest::{Package, Permission, SettingKind},
    store::{Record, Registry, Store},
};
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    combo_box::enz_combo_box,
    keyed_row::keyed_column,
    stepper::enz_stepper,
    text::enz_text,
    text_field::enz_text_field,
    toggle::enz_toggle,
};
use iced::{
    Element, Length, Task,
    widget::{column, container, row},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct Data {
    registry: Registry,
    packages: BTreeMap<String, Arc<Package>>,
    discovered: Vec<Candidate>,
}
impl Data {
    pub(super) fn fingerprint(&self) -> String {
        let records: Vec<_> = self
            .registry
            .entries
            .iter()
            .map(|r| {
                (
                    r.id(),
                    r.digest(),
                    r.enabled(),
                    r.activation(),
                    r.config_revision(),
                    r.fault(),
                )
            })
            .collect();
        let packages: Vec<_> = self
            .packages
            .iter()
            .map(|(id, package)| (id, package.digest()))
            .collect();
        let discovered: Vec<_> = self
            .discovered
            .iter()
            .map(|c| (&c.app_id, c.package.digest()))
            .collect();
        format!(
            "{}{records:?}{packages:?}{discovered:?}",
            self.registry.serial
        )
    }
}
#[derive(Clone)]
struct Review {
    package: Arc<Package>,
    previous: Option<[u8; 32]>,
    grants: BTreeSet<Permission>,
    origin: Option<String>,
    // Downloaded from the configured signed catalogue and verified.
    from_catalog: bool,
}
#[derive(Clone)]
enum Operation {
    Install {
        package: Arc<Package>,
        previous: Option<[u8; 32]>,
        grants: BTreeSet<Permission>,
        enabled: bool,
    },
    Enable {
        id: String,
        digest: [u8; 32],
        enabled: bool,
    },
    Config {
        id: String,
        digest: [u8; 32],
        key: String,
        value: Value,
    },
    Remove {
        id: String,
        digest: [u8; 32],
        activation: u64,
    },
}
impl Operation {
    fn same_key(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Enable {
                    id: a, digest: x, ..
                },
                Self::Enable {
                    id: b, digest: y, ..
                },
            ) => a == b && x == y,
            (
                Self::Config {
                    id: a,
                    digest: x,
                    key: k,
                    ..
                },
                Self::Config {
                    id: b,
                    digest: y,
                    key: l,
                    ..
                },
            ) => a == b && x == y && k == l,
            _ => false,
        }
    }
    fn apply(self, store: &Store) -> Result<(), String> {
        match self {
            Self::Install {
                package,
                previous,
                grants,
                enabled,
            } => {
                store.install(&package, previous, grants, enabled)?;
            }
            Self::Enable {
                id,
                digest,
                enabled,
            } => {
                store.enabled(&id, digest, enabled)?;
            }
            Self::Config {
                id,
                digest,
                key,
                value,
            } => {
                store.configure(&id, digest, &key, value)?;
            }
            Self::Remove {
                id,
                digest,
                activation,
            } => store.remove(&id, digest, activation)?,
        }
        store
            .reconcile_layout(false)
            .map_err(|e| tr!("shell-plugin-saved-but-bar-layout-is-pending", "arg0" => e))?;
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct State {
    store: Option<Store>,
    data: Option<Data>,
    selected: Option<String>,
    path: String,
    review: Option<Review>,
    review_generation: u64,
    review_busy: bool,
    reading: bool,
    inflight: Option<Operation>,
    pending: VecDeque<Operation>,
    error: Option<String>,
    read_error: Option<String>,
    remove: Option<(String, [u8; 32], u64)>,
    // The configured signed catalogue's last listing, and a check running.
    catalog: Option<Result<Vec<enzui_shell::plugins::catalog::Entry>, String>>,
    catalog_busy: bool,
}
impl State {
    pub(super) fn new(settings: &enzui_shell::settings::Store) -> Self {
        Self {
            store: Store::for_settings(settings.clone()).ok(),
            selected: std::env::var("ENZUI_SETTINGS_PLUGIN").ok().filter(|id| {
                enzui_shell::bar_layout::Item::new(format!("plugin:{id}/entry"), true).is_ok()
            }),
            ..Default::default()
        }
    }
    pub(super) fn select_from_launch(&mut self, id: Option<String>) {
        self.selected = id;
    }
    /// Installed plugins' names by id.
    pub(super) fn names(&self) -> std::collections::BTreeMap<String, String> {
        self.data.as_ref().map_or_else(Default::default, |data| {
            data.packages
                .iter()
                .map(|(id, package)| (id.clone(), package.manifest().name.clone()))
                .collect()
        })
    }
    pub(super) fn busy(&self) -> bool {
        self.inflight.is_some() || !self.pending.is_empty()
    }
    fn accept(&mut self, data: Data) {
        if self
            .data
            .as_ref()
            .is_none_or(|old| data.registry.serial >= old.registry.serial)
        {
            self.data = Some(data);
        }
    }
    fn operations(&self) -> impl Iterator<Item = &Operation> {
        self.inflight.iter().chain(self.pending.iter())
    }
    fn enabled(&self, record: &Record) -> bool {
        self.operations()
            .fold(record.enabled(), |old, op| match op {
                Operation::Enable {
                    id,
                    digest,
                    enabled,
                } if id == record.id() && *digest == record.digest() => *enabled,
                _ => old,
            })
    }
    fn config(&self, record: &Record, key: &str) -> Value {
        self.operations()
            .fold(record.config()[key].clone(), |old, op| match op {
                Operation::Config {
                    id,
                    digest,
                    key: k,
                    value,
                } if id == record.id() && *digest == record.digest() && k == key => value.clone(),
                _ => old,
            })
    }
}
#[derive(Clone)]
pub(super) enum Message {
    Read(Result<Data, String>),
    Saved(Result<Data, String>),
    Path(String),
    Review,
    ReviewInstalled(String, String),
    Reviewed(u64, Result<Arc<Package>, String>),
    Grant(Permission, bool),
    CancelReview,
    Install(bool),
    Enable(String, bool),
    Select(String),
    /// Closes the open plugin's details dialog.
    CloseDetails,
    Host(enzui_widgets::modal_host::ModalHostEvent),
    Config(String, String, Value),
    Permissions(String),
    AskRemove(String),
    CancelRemove,
    Remove,
    CatalogCheck,
    CatalogChecked(Result<Vec<enzui_shell::plugins::catalog::Entry>, String>),
    /// Downloads a catalogue plugin through the verified client and reviews it.
    CatalogReview(enzui_shell::plugins::catalog::Entry),
    CatalogReviewed(u64, Result<Arc<Package>, String>),
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginSettingsMessage")
    }
}
fn message(value: Message) -> AppMessage {
    AppMessage::Plugin(value)
}
fn read(store: &Store) -> Result<Data, String> {
    let registry = store.registry()?;
    let mut packages = BTreeMap::new();
    for record in &registry.entries {
        if let Ok(package) = store.package(record) {
            packages.insert(record.id().to_owned(), Arc::new(package));
        }
    }
    Ok(Data {
        registry,
        packages,
        discovered: discovery::installed(),
    })
}
fn runtime_available() -> bool {
    std::env::var("ENZUI_PLUGIN_ISOLATION").as_deref() == Ok("delegated")
        && enzui_shell::plugins::worker::helper().is_some()
}
impl SettingsApp {
    pub(super) fn read_plugins(&mut self) -> Task<AppMessage> {
        if !self.layout_preview || self.plugins.reading || self.plugins.busy() {
            return Task::none();
        }
        let Some(store) = self.plugins.store.clone() else {
            return Task::none();
        };
        self.plugins.reading = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || read(&store))
                    .await
                    .map_err(|_| tr("shell-plugin-reader-ended").to_owned())?
            },
            |result| message(Message::Read(result)),
        )
    }
    fn write_plugin(&mut self, operation: Operation) -> Task<AppMessage> {
        let Some(store) = self.plugins.store.clone() else {
            return Task::none();
        };
        self.plugins.inflight = Some(operation.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    operation.apply(&store)?;
                    read(&store)
                })
                .await
                .map_err(|_| tr("shell-plugin-settings-writer-ended").to_owned())?
            },
            |result| message(Message::Saved(result)),
        )
    }
    fn queue_plugin(&mut self, operation: Operation) -> Task<AppMessage> {
        if self.closing {
            return Task::none();
        }
        self.plugins.error = None;
        if self.plugins.inflight.is_some() {
            self.plugins.pending.retain(|old| !old.same_key(&operation));
            self.plugins.pending.push_back(operation);
            Task::none()
        } else {
            self.write_plugin(operation)
        }
    }
    pub(super) fn plugin_settings_message(&mut self, event: Message) -> Task<AppMessage> {
        if !self.layout_preview {
            return Task::none();
        }
        match event {
            Message::Read(result) => {
                self.plugins.reading = false;
                if !self.plugins.busy() {
                    match result {
                        Ok(data) => {
                            self.plugins.read_error = None;
                            self.plugins.accept(data);
                        }
                        Err(error) => self.plugins.read_error = Some(error),
                    }
                }
                Task::none()
            }
            Message::Saved(result) => {
                self.plugins.inflight = None;
                match result {
                    Ok(data) => {
                        self.plugins.read_error = None;
                        self.plugins.accept(data);
                        if let Some(next) = self.plugins.pending.pop_front() {
                            self.write_plugin(next)
                        } else if self.closing
                            && !self.busy
                            && !self.wallpaper.busy
                            && !self.users_pending()
                        {
                            iced::exit()
                        } else {
                            Task::none()
                        }
                    }
                    Err(error) => {
                        self.closing = false;
                        self.plugins.pending.clear();
                        self.plugins.error = Some(error);
                        self.read_plugins()
                    }
                }
            }
            Message::Path(path) => {
                if path.len() <= 4096 {
                    self.plugins.path = path;
                    self.plugins.review = None;
                    self.plugins.review_generation = self.plugins.review_generation.wrapping_add(1);
                }
                Task::none()
            }
            Message::Review => {
                if self.plugins.review_busy {
                    return Task::none();
                }
                let path = PathBuf::from(&self.plugins.path);
                if !path.is_absolute() {
                    self.plugins.error =
                        Some(tr("shell-choose-an-absolute-local-enzplugin-file-path").into());
                    return Task::none();
                }
                self.plugins.review = None;
                self.plugins.review_generation = self.plugins.review_generation.wrapping_add(1);
                let generation = self.plugins.review_generation;
                self.plugins.review_busy = true;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            Store::read_package(&path).map(Arc::new)
                        })
                        .await
                        .map_err(|_| tr("shell-package-reader-ended").to_owned())?
                    },
                    move |result| message(Message::Reviewed(generation, result)),
                )
            }
            Message::ReviewInstalled(app_id, plugin_id) => {
                let Some(candidate) = self.plugins.data.as_ref().and_then(|data| {
                    data.discovered
                        .iter()
                        .find(|c| c.app_id == app_id && c.package.manifest().id == plugin_id)
                }) else {
                    return Task::none();
                };
                let package = std::sync::Arc::clone(&candidate.package);
                let previous = self.plugins.data.as_ref().and_then(|data| {
                    data.registry
                        .entries
                        .iter()
                        .find(|r| r.id() == plugin_id)
                        .map(Record::digest)
                });
                self.plugins.review_generation = self.plugins.review_generation.wrapping_add(1);
                self.plugins.review = Some(Review {
                    grants: package.manifest().permissions.clone(),
                    package,
                    previous,
                    origin: Some(app_id),
                    from_catalog: false,
                });
                self.plugins.error = None;
                Task::none()
            }
            Message::CatalogCheck => {
                if std::mem::replace(&mut self.plugins.catalog_busy, true) {
                    return Task::none();
                }
                Task::perform(enzui_shell::plugins::catalog::check(), |result| {
                    message(Message::CatalogChecked(result))
                })
            }
            Message::CatalogChecked(result) => {
                self.plugins.catalog_busy = false;
                self.plugins.catalog = Some(result);
                Task::none()
            }
            Message::CatalogReview(entry) => {
                if self.plugins.review_busy {
                    return Task::none();
                }
                self.plugins.review = None;
                self.plugins.review_generation = self.plugins.review_generation.wrapping_add(1);
                let generation = self.plugins.review_generation;
                self.plugins.review_busy = true;
                Task::perform(
                    async move {
                        enzui_shell::plugins::catalog::download(&entry)
                            .await
                            .map(Arc::new)
                    },
                    move |result| message(Message::CatalogReviewed(generation, result)),
                )
            }
            Message::CatalogReviewed(generation, result) => {
                self.plugins.review_busy = false;
                if generation != self.plugins.review_generation {
                    return Task::none();
                }
                match result {
                    Ok(package) => {
                        let installed = self.plugins.data.as_ref().and_then(|d| {
                            d.registry
                                .entries
                                .iter()
                                .find(|r| r.id() == package.manifest().id)
                        });
                        // An update keeps what was granted and starts any new
                        // permission unchecked; it never widens access by itself.
                        let grants = match installed {
                            Some(record) => package
                                .manifest()
                                .permissions
                                .iter()
                                .filter(|p| record.grants().contains(p))
                                .cloned()
                                .collect(),
                            None => package.manifest().permissions.clone(),
                        };
                        self.plugins.review = Some(Review {
                            grants,
                            previous: installed.map(Record::digest),
                            package,
                            origin: None,
                            from_catalog: true,
                        });
                        self.plugins.error = None;
                    }
                    Err(error) => self.plugins.error = Some(error),
                }
                Task::none()
            }
            Message::Reviewed(generation, result) => {
                self.plugins.review_busy = false;
                if generation != self.plugins.review_generation {
                    return Task::none();
                }
                match result {
                    Ok(package) => {
                        let previous = self
                            .plugins
                            .data
                            .as_ref()
                            .and_then(|d| {
                                d.registry
                                    .entries
                                    .iter()
                                    .find(|r| r.id() == package.manifest().id)
                            })
                            .map(Record::digest);
                        self.plugins.review = Some(Review {
                            grants: package.manifest().permissions.clone(),
                            package,
                            previous,
                            origin: None,
                            from_catalog: false,
                        });
                        self.plugins.error = None;
                    }
                    Err(error) => self.plugins.error = Some(error),
                }
                Task::none()
            }
            Message::Grant(permission, granted) => {
                if let Some(review) = self.plugins.review.as_mut()
                    && review.package.manifest().permissions.contains(&permission)
                {
                    if granted {
                        review.grants.insert(permission);
                    } else {
                        review.grants.remove(&permission);
                    }
                }
                Task::none()
            }
            Message::CancelReview => {
                self.plugins.review = None;
                self.plugins.review_generation = self.plugins.review_generation.wrapping_add(1);
                Task::none()
            }
            Message::Install(enabled) => {
                if enabled && !runtime_available() {
                    return Task::none();
                }
                let Some(review) = self.plugins.review.take() else {
                    return Task::none();
                };
                self.queue_plugin(Operation::Install {
                    package: review.package,
                    previous: review.previous,
                    grants: review.grants,
                    enabled,
                })
            }
            Message::Enable(id, enabled) => {
                if enabled && !runtime_available() {
                    return Task::none();
                }
                let Some(record) = self
                    .plugins
                    .data
                    .as_ref()
                    .and_then(|d| d.registry.entries.iter().find(|r| r.id() == id))
                else {
                    return Task::none();
                };
                let digest = record.digest();
                self.queue_plugin(Operation::Enable {
                    id,
                    digest,
                    enabled,
                })
            }
            Message::Select(id) => {
                self.plugins.remove = None;
                self.plugins.selected = Some(id);
                Task::none()
            }
            Message::CloseDetails
            | Message::Host(enzui_widgets::modal_host::ModalHostEvent::Dismissed(_)) => {
                self.plugins.remove = None;
                self.plugins.selected = None;
                Task::none()
            }
            Message::Host(event) => enzui_widgets::modal_host::focus_wrap_operation(event)
                .map_or_else(Task::none, iced::advanced::widget::operate),
            Message::Config(id, key, value) => {
                let Some(record) = self
                    .plugins
                    .data
                    .as_ref()
                    .and_then(|d| d.registry.entries.iter().find(|r| r.id() == id))
                else {
                    return Task::none();
                };
                let digest = record.digest();
                self.queue_plugin(Operation::Config {
                    id,
                    digest,
                    key,
                    value,
                })
            }
            Message::Permissions(id) => {
                if let Some(data) = &self.plugins.data
                    && let Some(record) = data.registry.entries.iter().find(|r| r.id() == id)
                    && let Some(package) = data.packages.get(&id)
                {
                    self.plugins.review = Some(Review {
                        package: std::sync::Arc::clone(package),
                        previous: Some(record.digest()),
                        grants: record.grants().clone(),
                        origin: None,
                        from_catalog: false,
                    });
                }
                Task::none()
            }
            Message::AskRemove(id) => {
                self.plugins.selected = Some(id.clone());
                self.plugins.remove = self
                    .plugins
                    .data
                    .as_ref()
                    .and_then(|d| d.registry.entries.iter().find(|r| r.id() == id))
                    .map(|r| (id, r.digest(), r.activation()));
                Task::none()
            }
            Message::CancelRemove => {
                self.plugins.remove = None;
                Task::none()
            }
            Message::Remove => {
                let Some((id, digest, activation)) = self.plugins.remove.take() else {
                    return Task::none();
                };
                self.queue_plugin(Operation::Remove {
                    id,
                    digest,
                    activation,
                })
            }
        }
    }
    // The configured signed catalogue: what it offers, and whether each
    // plugin is installed, has an update, or can be reviewed for install.
    fn catalog_view(&self, enabled: bool) -> Element<'_, AppMessage> {
        let palette = self.tokens().palette;
        let (client, configured) = enzui_shell::plugins::catalog::available();
        let mut list = column![].spacing(8);
        if !client {
            return self.mark_setting("plugins/catalog", iced::widget::Space::new());
        }
        if !configured {
            list = list.push(
                enz_text(enzui_shell::i18n::tr("shell-no-plugin-catalogue-is-set-up-to-use-one-enrol-its-reviewed-root-in-config-e"))
                    .muted_on(palette.surface)
                    .into_element(),
            );
            return self.mark_setting("plugins/catalog", list);
        }
        list = list.push(
            row![
                enz_text(tr("shell-signed-catalogue"))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
                iced::widget::Space::new().width(Length::Fill),
                enz_button(
                    if self.plugins.catalog_busy {
                        enzui_shell::i18n::tr("shell-checking")
                    } else {
                        tr("shell-check-catalogue")
                    },
                    message(Message::CatalogCheck)
                )
                .id("plugins-check-catalog".into())
                .icon("refresh-cw")
                .enabled(enabled && !self.plugins.catalog_busy)
                .into_element(),
            ]
            .align_y(iced::Alignment::Center),
        );
        match &self.plugins.catalog {
            None => list = list.push(enz_text(tr("shell-check-the-catalogue-to-see-the-plugins-it-offers-each-is-verified-before-you")).muted_on(palette.surface).into_element()),
            Some(Err(error)) => list = list.push(enz_text(error.clone()).into_element()),
            Some(Ok(entries)) if entries.is_empty() => list = list.push(enz_text(tr("shell-the-catalogue-offers-no-plugins")).muted_on(palette.surface).into_element()),
            Some(Ok(entries)) => {
                for entry in entries {
                    let installed = self.plugins.data.as_ref().and_then(|d| d.packages.get(&entry.id)).map(|p| p.manifest().version.clone());
                    let action: Element<'_, AppMessage> = match installed {
                        Some(version) if version == entry.version => enz_text(tr("shell-installed")).muted_on(palette.surface).into_element(),
                        Some(_) => enz_button(tr("shell-review-update"), message(Message::CatalogReview(entry.clone())))
                            .id(format!("plugins-catalog-review-{}", entry.id).into())
                            .icon("shield-check")
                            .enabled(enabled && !self.plugins.review_busy)
                            .into_element(),
                        None => enz_button(tr("shell-review"), message(Message::CatalogReview(entry.clone())))
                            .id(format!("plugins-catalog-review-{}", entry.id).into())
                            .icon("shield-check")
                            .enabled(enabled && !self.plugins.review_busy)
                            .into_element(),
                    };
                    list = list.push(
                        row![
                            column![
                                enz_text(format!("{} {}", entry.name, entry.version)).into_element(),
                                enz_text(entry.description.clone()).muted_on(palette.surface).into_element(),
                            ]
                            .spacing(2)
                            .width(Length::Fill),
                            action,
                        ]
                        .spacing(12)
                        .align_y(iced::Alignment::Center),
                    );
                }
            }
        }
        self.mark_setting("plugins/catalog", list)
    }
    pub(super) fn plugins_view(&self) -> Element<'_, AppMessage> {
        let palette = self.tokens().palette;
        let enabled = !self.closing;
        let mut body = column![
            enz_text(tr(
                "shell-install-local-packages-only-after-reviewing-their-requested-permissions-plug"
            ))
            .muted_on(palette.surface)
            .into_element()
        ]
        .spacing(16)
        .width(Length::Fill);
        if !runtime_available() {
            body=body.push(enz_text(tr("shell-activation-requires-a-fresh-managed-desktop-with-delegated-plugin-isolation")).into_element());
        }
        body = body
            .push(
                self.mark_setting(
                    "plugins/install",
                    enz_text_field(
                        enzui_shell::i18n::current().text("shell-local-package"),
                        enzui_shell::i18n::current()
                            .text("shell-absolute-path-to-a-enzplugin-file"),
                        &self.plugins.path,
                        |v| message(Message::Path(v)),
                    )
                    .id_prefix("plugin-package-path")
                    .enabled(enabled)
                    .id("plugin-local-package".into())
                    .into_element(),
                ),
            )
            .push(
                enz_button(tr("shell-review-package"), message(Message::Review))
                    .icon("shield-check")
                    .id("plugin-review-package".into())
                    .enabled(enabled && !self.plugins.review_busy)
                    .into_element(),
            );
        if let Some(data) = &self.plugins.data
            && !data.discovered.is_empty()
        {
            body = body.push(enz_text(tr("shell-plugins-from-installed-flatpak-apps-review-before-installing-off-by-default")).into_element());
            for candidate in &data.discovered {
                let package = candidate.package.manifest();
                let app = candidate.app_id.clone();
                let id = package.id.clone();
                body = body.push(
                    row![
                        enz_text(format!("{} · {}", package.name, candidate.app_id)).into_element(),
                        enz_button(
                            enzui_shell::i18n::tr("shell-review"),
                            message(Message::ReviewInstalled(app, id))
                        )
                        .id(format!(
                            "plugins-review-installed-{}-{}",
                            candidate.app_id, package.id
                        )
                        .into())
                        .icon("shield-check")
                        .enabled(enabled && !self.plugins.review_busy)
                        .into_element()
                    ]
                    .spacing(12),
                );
            }
        }
        body = body.push(self.catalog_view(enabled));
        if let Some(error) = self
            .plugins
            .error
            .as_ref()
            .or(self.plugins.read_error.as_ref())
        {
            body = body.push(enz_text(error).into_element());
        }
        if let Some(review) = &self.plugins.review {
            let manifest = review.package.manifest();
            let mut details = column![
                enz_text(format!("{} • {}", manifest.name, manifest.version))
                    .size_px(enzui_theme::text_scale::effective().apply_px(20.0))
                    .into_element(),
                enz_text(format!("{} • {}", manifest.id, manifest.description))
                    .muted_on(palette.surface)
                    .into_element()
            ]
            .spacing(10);
            if let Some(app) = &review.origin {
                details = details.push(
                    enz_text(tr!("shell-provided-by-installed-flatpak-app", "arg0" => app.clone()))
                        .into_element(),
                );
            }
            if review.from_catalog {
                details = details.push(
                    enz_text(if review.previous.is_some() {
                        tr("shell-from-the-signed-catalogue-verified-before-this-review-permissions-the-instal")
                    } else {
                        tr("shell-from-the-signed-catalogue-verified-before-this-review")
                    })
                    .into_element(),
                );
            }
            for permission in &manifest.permissions {
                let p = *permission;
                details = details.push(
                    enz_toggle(
                        review.grants.contains(permission),
                        permission.label(),
                        move |value| message(Message::Grant(p, value)),
                    )
                    .enabled(enabled)
                    .id_prefix(format!("grant-{}", permission.key()))
                    .into_element(),
                );
            }
            details = details.push(
                column![
                    enz_button(
                        tr("shell-install-apply-and-enable"),
                        message(Message::Install(true))
                    )
                    .id("plugins-install-enable".into())
                    .icon("download")
                    .enabled(enabled && runtime_available())
                    .into_element(),
                    enz_button(
                        tr("shell-install-apply-disabled"),
                        message(Message::Install(false))
                    )
                    .id("plugins-install-disabled".into())
                    .kind(ButtonKind::Secondary)
                    .icon("download")
                    .enabled(enabled)
                    .into_element(),
                    enz_button(
                        enzui_shell::i18n::tr("shell-cancel"),
                        message(Message::CancelReview)
                    )
                    .id("plugins-review-cancel".into())
                    .kind(ButtonKind::Quiet)
                    .enabled(enabled)
                    .into_element()
                ]
                .spacing(8),
            );
            body = body.push(
                container(details)
                    .padding(14)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(palette.surface_alt.into()),
                        border: iced::Border {
                            radius: 10.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        body = body.push(
            // An anchor only: the highlight boxed the label like a field.
            self.anchor_setting(
                "plugins/installed",
                // A label inside its card, at the size of the rows under it.
                enz_text(tr("shell-installed-plugins"))
                    .size_px(self.text_px(13.0))
                    .weight_rung(enzui_theme::WeightRung::Semibold)
                    .into_element(),
            ),
        );
        if let Some(data) = &self.plugins.data {
            let rows = data
                .registry
                .entries
                .iter()
                .map(|record| {
                    let id = record.id().to_owned();
                    let name = data
                        .packages
                        .get(&id)
                        .map_or_else(|| record.id(), |p| p.manifest().name.as_str());
                    let on = self.plugins.enabled(record);
                    let toggle = id.clone();
                    let known = data.packages.contains_key(&id);
                    let status = if self.plugins.busy() {
                        tr("shell-applying-changes").to_owned()
                    } else if let Some(fault) = record.fault() {
                        tr!("shell-stopped-2", "arg0" => fault.to_owned())
                    } else if on {
                        tr("shell-enabled").into()
                    } else {
                        tr("shell-disabled").into()
                    };
                    // The row every other setting uses: name, state, controls.
                    let controls = row![
                        enz_button(
                            enzui_shell::i18n::tr("shell-details"),
                            message(Message::Select(id.clone()))
                        )
                        .kind(ButtonKind::Quiet)
                        .size(enzui_widgets::button::ButtonSize::Sm)
                        .enabled(enabled)
                        .id(format!("plugin-details-{id}").into())
                        .into_element(),
                        enz_toggle(on, "", move |v| message(Message::Enable(toggle.clone(), v)))
                            .aria_label(enzui_shell::tr!("shell-a11y-enable", "arg0" => enzui_shell::i18n::argument(&(name))))
                            .id_prefix(format!("plugin-enabled-{id}"))
                            .enabled(enabled && (on || (known && runtime_available())))
                            .into_element(),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center);
                    let item = crate::settings_app::settings_row(name.to_owned())
                        .icon("puzzle")
                        .subtitle(status)
                        .control(controls)
                        .into_element();
                    (id, item)
                })
                .collect();
            body = body.push(keyed_column(rows).spacing(2.0).width(Length::Fill));
            if data.registry.entries.is_empty() {
                body = body.push(enz_text(tr("shell-no-plugins-installed")).into_element());
            }
        }
        body.into()
    }

    pub(super) fn plugin_details_dialog(&self) -> Option<Element<'_, AppMessage>> {
        let data = self.plugins.data.as_ref()?;
        let record = self
            .plugins
            .selected
            .as_ref()
            .and_then(|id| data.registry.entries.iter().find(|r| r.id() == id))?;
        let enabled = !self.closing;
        let palette = self.tokens().palette;

        let id = record.id().to_owned();
        let mut details = column![].spacing(4);
        if let Some(package) = data.packages.get(&id) {
            details = details.push(
                enz_button(
                    tr("shell-review-permissions"),
                    message(Message::Permissions(id.clone())),
                )
                .id("plugins-review-permissions".into())
                .kind(ButtonKind::Quiet)
                .icon("shield-check")
                .enabled(enabled)
                .into_element(),
            );
            for spec in &package.manifest().settings {
                let current = self.plugins.config(record, &spec.key);
                let plugin = id.clone();
                let key = spec.key.clone();
                let control: Element<'_, AppMessage> = match &spec.kind {
                    SettingKind::Applications { .. } => {
                        let ids = current
                            .as_array()
                            .map(|v| {
                                v.iter()
                                    .filter_map(Value::as_str)
                                    .map(|v| v.trim_start_matches("app:"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        column![enz_text(&spec.label).into_element(),enz_text(if ids.is_empty() {tr("shell-no-favorites-yet").into()} else {ids}).muted_on(palette.surface).into_element(),enz_text(tr("shell-right-click-applications-in-search-or-the-dock-to-change-favorites")).size(enzui_widgets::text::TextSize::Caption).into_element()].spacing(4).padding([6, 10]).into()
                    }
                    // The same rows as every other preference: the name on
                    // the left, its control on the right. Labels above
                    // full-width fields read as a different app.
                    SettingKind::Toggle { .. } => crate::settings_app::settings_row(&spec.label)
                        .id_prefix(format!("plugin-{id}-{}", spec.key))
                        .switch(current.as_bool().unwrap_or(false), move |v| {
                            message(Message::Config(plugin.clone(), key.clone(), json!(v)))
                        })
                        .enabled(enabled)
                        .into_element(),
                    SettingKind::Integer { min, max, .. } => {
                        crate::settings_app::settings_row(&spec.label)
                            .control(
                                enz_stepper(current.as_i64().unwrap_or(*min) as i32, move |v| {
                                    message(Message::Config(plugin.clone(), key.clone(), json!(v)))
                                })
                                .aria_label(&spec.label)
                                .min(*min as i32)
                                .max(*max as i32)
                                .id(format!("plugin-{id}-{}", spec.key))
                                .enabled(enabled)
                                .into_element(),
                            )
                            .enabled(enabled)
                            .into_element()
                    }
                    SettingKind::Choice {
                        choices, labels, ..
                    } => {
                        let mapping = choices
                            .iter()
                            .map(|v| (labels.get(v).unwrap_or(v).clone(), v.clone()))
                            .collect::<BTreeMap<_, _>>();
                        let field = enz_combo_box(
                            "",
                            choices
                                .iter()
                                .map(|v| labels.get(v).unwrap_or(v).clone())
                                .collect(),
                            current
                                .as_str()
                                .map(|v| labels.get(v).map_or(v, String::as_str).to_owned()),
                            move |v| {
                                message(Message::Config(
                                    plugin.clone(),
                                    key.clone(),
                                    json!(mapping.get(&v).unwrap_or(&v)),
                                ))
                            },
                        )
                        .aria_label(&spec.label)
                        .id_prefix(format!("plugin-{id}-{}", spec.key))
                        .size(enzui_widgets::text_field::TextFieldSize::Sm)
                        .enabled(enabled)
                        .into_element();
                        crate::settings_app::settings_row(&spec.label)
                            .control(field)
                            .enabled(enabled)
                            .into_element()
                    }
                };
                details = details.push(control);
            }
        } else {
            details=details.push(enz_text(tr("shell-package-is-missing-or-invalid-disable-or-remove-it-no-unverified-program-wil")).into_element());
        }
        if self
            .plugins
            .remove
            .as_ref()
            .is_some_and(|(selected, _, _)| *selected == id)
        {
            details=details.push(column![enz_text(enzui_shell::tr!("shell-remove-this-stops-the-plugin-and-removes-its-package-and-configuration", "arg0" => enzui_shell::i18n::argument(&(id)))).into_element(),row![enz_button(tr("shell-remove-package"),message(Message::Remove)).id("plugins-remove-confirm".into()).kind(ButtonKind::Destructive).icon("trash-2").enabled(enabled).into_element(),enz_button(enzui_shell::i18n::tr("shell-cancel"),message(Message::CancelRemove)).id("plugins-remove-cancel".into()).kind(ButtonKind::Quiet).enabled(enabled).into_element()].spacing(8)].spacing(8));
        } else {
            details = details.push(
                enz_button(
                    tr("shell-remove-2"),
                    message(Message::AskRemove(id.clone())),
                )
                .id("plugins-remove".into())
                .kind(ButtonKind::QuietDestructive)
                .icon("trash-2")
                .enabled(enabled)
                .into_element(),
            );
        }
        let name = data
            .packages
            .get(&id)
            .map_or_else(|| id.clone(), |p| p.manifest().name.clone());
        let header: Element<'_, AppMessage> = column![
            enz_text(name.clone())
                .size_px(self.text_px(18.0))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            enz_text(id.clone())
                .size_px(self.text_px(12.0))
                .muted_on(palette.surface)
                .into_element(),
        ]
        .spacing(2)
        .into();
        let close = message(Message::CloseDetails);
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(Element::from(
            container(details).padding(iced::Padding {
                right: 12.0,
                ..Default::default()
            }),
        ))
        .id("plugin-details-body".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .into_element();
        let footer: Element<'_, AppMessage> = row![
            iced::widget::space::horizontal(),
            enz_button(tr("shell-done"), close.clone())
                .kind(ButtonKind::Primary)
                .id("plugin-details-done".into())
                .into_element(),
        ]
        .into();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![header],
            scroll,
            vec![footer],
            16.0,
            640.0,
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(520.0)
                .padding(20.0)
                .on_dismiss(move || close.clone())
                .close_label(tr!("shell-close-details-2", "arg0" => name))
                .into_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, SettingsApp, Arc<Package>) {
        let root = tempfile::tempdir().unwrap();
        let store = enzui_shell::settings::Store::at(root.path().join("settings.json")).unwrap();
        let (mut app, _) = SettingsApp::new(store);
        app.layout_preview = true;
        app.plugins.reading = false;
        let package=Arc::new(Package::parse(&serde_json::to_vec(&json!({"format":1,"id":"fixture","name":"Fixture","version":"1","description":"Disposable configuration","permissions":["shell.behavior"],"elements":[],"settings":[{"key":"step","label":"Step","kind":"integer","min":1,"max":10,"default":1}],"program":"pass\n"})).unwrap()).unwrap());
        let store = app.plugins.store.as_ref().unwrap();
        store
            .install(&package, None, BTreeSet::new(), false)
            .unwrap();
        app.plugins.data = Some(read(store).unwrap());
        (root, app, package)
    }
    #[test]
    fn approval_keeps_the_reviewed_bytes_and_explicitly_selected_grants() {
        let (_root, mut app, package) = fixture();
        let digest = package.digest();
        let _ = app.plugin_settings_message(Message::Reviewed(0, Ok(package)));
        let _ = app.plugin_settings_message(Message::Grant(Permission::Behavior, false));
        let _ = app.plugin_settings_message(Message::Install(false));
        match app.plugins.inflight.as_ref().unwrap() {
            Operation::Install {
                package,
                grants,
                previous,
                enabled,
            } => {
                assert_eq!(package.digest(), digest);
                assert!(grants.is_empty());
                assert_eq!(*previous, Some(digest));
                assert!(!enabled);
            }
            _ => panic!("Expected reviewed install"),
        }
        let _ = app.update(AppMessage::Close);
        assert!(app.closing);
        let _ =
            app.plugin_settings_message(Message::Saved(Err("Concurrent package update".into())));
        assert!(!app.closing);
        assert!(!app.plugins.busy());
        assert_eq!(
            app.plugins.error.as_deref(),
            Some("Concurrent package update")
        );
    }
}
