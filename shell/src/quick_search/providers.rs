//! App-provider selection, query dispatch and result projection for Search.

use super::{model::Message, support::perform};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::search::{
    Category, Item, Target,
    providers::{self, Declaration, Found},
};
use enzui_shell::tr;
use iced::Task;
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

#[derive(Clone, Debug)]
pub(crate) struct Provider {
    pub(crate) declaration: Declaration,
    pub(crate) label: Arc<str>,
    pub(crate) icon: String,
    pub(crate) image: Option<std::path::PathBuf>,
}

// Typing pause before providers are asked.
const PAUSE: Duration = Duration::from_millis(120);
// Failed answers in a row before a provider is left alone for the session.
const STRIKES: u8 = 3;

// Settings has a native provider ID without a catalogue desktop entry.
const SETTINGS_PROVIDER: &str = "dev.enzui.Settings";

fn native_desktop_id(id: &str) -> Option<&'static str> {
    if id == "enzui-settings" {
        Some(SETTINGS_PROVIDER)
    } else {
        enzui_shell::catalog::desktop_id(id)
    }
}

pub(crate) fn matched(declarations: Vec<Declaration>, apps: &[Item]) -> Vec<Provider> {
    declarations
        .into_iter()
        .filter_map(|declaration| {
            let stem = declaration.desktop_id.strip_suffix(".desktop")?.to_owned();
            let app = apps.iter().find(|app| {
                app.id == format!("app:desktop/{}", declaration.desktop_id)
                    || app
                        .id
                        .strip_prefix("app:")
                        .and_then(native_desktop_id)
                        .is_some_and(|id| id == stem)
            })?;
            Some(Provider {
                label: Arc::from(app.title.as_str()),
                icon: app.icon.clone(),
                image: app.image.clone(),
                declaration,
            })
        })
        .collect()
}

pub(crate) fn rows(
    provider: &Provider,
    found: Vec<Found>,
    query: &str,
    lookup: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> Vec<Item> {
    if found.is_empty() {
        return Vec::new();
    }
    let desktop_id = &provider.declaration.desktop_id;
    // Built-in Settings destinations are Actions; the provider adds its search entry.
    let built_in = desktop_id.strip_suffix(".desktop") == Some(SETTINGS_PROVIDER);
    let mut rows: Vec<Item> = found
        .into_iter()
        .filter(|_| !built_in)
        .take(providers::SHOWN)
        .map(|found| {
            // A themed icon the result names, else the app's own.
            let image = found
                .icon
                .as_deref()
                .and_then(&(lookup))
                .or_else(|| provider.image.clone());
            Item::new(
                format!("provider:{desktop_id}:{}", found.id),
                Category::InApps,
                found.title,
                found.subtitle,
                tr("shell-open"),
                provider.icon.clone(),
                Some(Target::Provider {
                    provider: Box::new(provider.declaration.clone()),
                    id: found.id,
                    query: query.to_owned(),
                }),
            )
            .with_source(std::sync::Arc::clone(&provider.label))
            .with_image(image)
        })
        .collect();
    rows.push(
        Item::new(
            format!("provider-search:{desktop_id}"),
            Category::InApps,
            tr!("shell-search-in-2", "arg0" => format!("{:}", provider.label)),
            tr!("shell-show-every-result-for", "arg0" => query.to_owned()),
            tr("shell-search"),
            "search",
            Some(Target::ProviderSearch {
                provider: Box::new(provider.declaration.clone()),
                query: query.to_owned(),
            }),
        )
        .with_source(std::sync::Arc::clone(&provider.label)),
    );
    rows
}

impl Shell {
    pub(super) fn search_find_providers(&mut self) {
        self.search.providers = matched(
            providers::discover(&providers::data_dirs()),
            &self.search.apps,
        );
    }

    // Discover OS search-activation support without starting the service.
    pub(super) fn search_check_activation(&mut self) -> Task<ShellMessage> {
        let wanted = self
            .search
            .providers
            .iter()
            .any(|p| p.declaration.protocol == providers::Protocol::Enzui);
        if !self.search.showing || self.search.activation_checked || !wanted {
            return Task::none();
        }
        self.search.activation_checked = true;
        let epoch = self.search.epoch;
        perform(
            async {
                let (client, capabilities, _) =
                    enzui_shell::background_apps::Client::connect().await.ok()?;
                capabilities.search_policy_adapter.then(|| Arc::new(client))
            },
            move |client| Message::Activation(epoch, client),
        )
    }

    // OS-approved activation retries a missing native provider once; query calls retain
    // NoAutoStart.
    fn search_activate(
        &mut self,
        generation: u64,
        desktop_id: String,
        query: String,
    ) -> Task<ShellMessage> {
        let Some(client) = self.search.activation.clone() else {
            return Task::none();
        };
        let enzui = self.search.providers.iter().any(|p| {
            p.declaration.desktop_id == desktop_id
                && p.declaration.protocol == providers::Protocol::Enzui
        });
        if !enzui || !self.search.activation_tried.insert(desktop_id.clone()) {
            return Task::none();
        }
        let Some(app_id) = desktop_id.strip_suffix(".desktop").map(str::to_owned) else {
            return Task::none();
        };
        perform(
            async move { client.activate_for_search(&app_id).await.unwrap_or(false) },
            move |started| {
                Message::Activated(generation, desktop_id.clone(), query.clone(), started)
            },
        )
    }

    pub(super) fn search_activated(
        &mut self,
        generation: u64,
        desktop_id: String,
        query: String,
        started: bool,
    ) -> Task<ShellMessage> {
        if !self.search.showing
            || !started
            || generation != self.search.provider_generation.load(Ordering::Acquire)
            || self.search.model.query.trim() != query
        {
            return Task::none();
        }
        let Some(provider) = self
            .search
            .providers
            .iter()
            .find(|p| p.declaration.desktop_id == desktop_id)
            .cloned()
        else {
            return Task::none();
        };
        let asked = query.clone();
        perform(
            async move {
                let bus = providers::session().await?;
                Some(providers::search(&bus, &provider.declaration, &asked).await)
            },
            move |answer| Message::Provided(generation, desktop_id.clone(), query.clone(), answer),
        )
    }

    // Query generations retire late provider results.
    pub(super) fn search_ask_providers(&mut self) -> Task<ShellMessage> {
        let generation = self
            .search
            .provider_generation
            .fetch_add(1, Ordering::AcqRel)
            + 1;
        self.search.provider_results.clear();
        let query = self.search.model.query.trim().to_owned();
        let category = self.search.model.category;
        if !self.search.showing
            || !self.search.preferences.shows(Category::InApps)
            || category.is_some_and(|c| c != Category::InApps)
        {
            return Task::none();
        }
        let length = query.chars().count();
        let asked: Vec<Provider> = self
            .search
            .providers
            .iter()
            .filter(|p| length >= p.declaration.min_query)
            .filter(|p| {
                self.search.preferences.provider_on(
                    &p.declaration.desktop_id,
                    p.declaration.protocol == providers::Protocol::Enzui,
                )
            })
            .filter(|p| {
                self.search
                    .provider_strikes
                    .get(&p.declaration.desktop_id)
                    .copied()
                    .unwrap_or(0)
                    < STRIKES
            })
            .cloned()
            .collect();
        Task::batch(asked.into_iter().map(|provider| {
            let current = std::sync::Arc::clone(&self.search.provider_generation);
            let query = query.clone();
            let desktop_id = provider.declaration.desktop_id.clone();
            let asked_query = query.clone();
            perform(
                async move {
                    tokio::time::sleep(PAUSE).await;
                    if current.load(Ordering::Acquire) != generation {
                        return None;
                    }
                    let bus = providers::session().await?;
                    Some(providers::search(&bus, &provider.declaration, &query).await)
                },
                move |answer| {
                    Message::Provided(generation, desktop_id.clone(), asked_query.clone(), answer)
                },
            )
        }))
    }

    pub(super) fn search_provided(
        &mut self,
        generation: u64,
        desktop_id: String,
        query: String,
        answer: Option<Result<Vec<Found>, providers::Silence>>,
    ) -> Task<ShellMessage> {
        if !self.search.showing
            || generation != self.search.provider_generation.load(Ordering::Acquire)
            || self.search.model.query.trim() != query
        {
            return Task::none();
        }
        let Some(answer) = answer else {
            return Task::none();
        };
        let Some(index) = self
            .search
            .providers
            .iter()
            .position(|p| p.declaration.desktop_id == desktop_id)
        else {
            return Task::none();
        };
        match answer {
            // An app that is not running is not asked, unless the OS may
            // start it for search; only failures count.
            Err(providers::Silence::NotRunning) => {
                return self.search_activate(generation, desktop_id, query);
            }
            Err(providers::Silence::Failed(_)) => {
                let strikes = self.search.provider_strikes.entry(desktop_id).or_default();
                *strikes = strikes.saturating_add(1);
            }
            Ok(found) => {
                self.search.provider_strikes.remove(&desktop_id);
                let provider = self.search.providers[index].clone();
                let source = std::sync::Arc::clone(&provider.label);
                self.search
                    .provider_results
                    .retain(|item| item.source.as_ref() != Some(&source));
                self.search
                    .provider_results
                    .extend(rows(&provider, found, &query, |name| {
                        enzui_shell::icon_theme::lookup(name, 48)
                    }));
                // Groups keep the providers' order, whichever answered first.
                let order = |item: &Item| {
                    self.search
                        .providers
                        .iter()
                        .position(|p| Some(&p.label) == item.source.as_ref())
                        .unwrap_or(usize::MAX)
                };
                let mut results = std::mem::take(&mut self.search.provider_results);
                results.sort_by_key(|item| order(item));
                self.search.provider_results = results;
                self.search_recompute();
            }
        }
        Task::none()
    }

    pub(super) fn search_open_in_app(
        &mut self,
        ticket: enzui_shell::search::Ticket,
        target: Target,
    ) -> Task<ShellMessage> {
        let Some(window) = self.search.window else {
            return Task::none();
        };
        let epoch = self.search.epoch;
        self.search.action_busy = true;
        self.search.feedback = Some(tr("shell-opening").into());
        iced::window::request_activation_token(window).map(move |result| {
            super::support::message(Message::ProviderToken(
                epoch,
                window,
                ticket.clone(),
                target.clone(),
                result.ok().map(iced::window::ActivationToken::into_raw),
            ))
        })
    }

    pub(super) fn search_open_in_app_with_token(
        &mut self,
        epoch: u64,
        target: Target,
        token: Option<String>,
    ) -> Task<ShellMessage> {
        perform(
            async move {
                let bus = providers::session().await.ok_or_else(|| {
                    tr("shell-this-desktop-s-session-bus-is-unavailable").to_owned()
                })?;
                match target {
                    Target::Provider {
                        provider,
                        id,
                        query,
                    } => {
                        providers::activate_with_token(
                            &bus,
                            &provider,
                            &id,
                            &query,
                            token.as_deref(),
                        )
                        .await
                    }
                    Target::ProviderSearch { provider, query } => {
                        providers::launch_search_with_token(
                            &bus,
                            &provider,
                            &query,
                            token.as_deref(),
                        )
                        .await
                    }
                    _ => Ok(()),
                }
                .map_err(|_| tr("shell-the-app-could-not-open-this-result").to_owned())
            },
            move |result| Message::Done(epoch, result),
        )
    }
}
