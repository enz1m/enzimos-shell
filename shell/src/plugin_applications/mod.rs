use crate::{Message, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    dock::{Change, Preferences},
    plugins::{applications, manifest::Permission, service::Owner, store::Store},
};
use iced::Task;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct Claim {
    pub owner: Owner,
    pub element: String,
    pub digest: [u8; 32],
    pub revision: u64,
    pub preferences: Preferences,
    pub authority: Arc<AtomicBool>,
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Loaded {
        serial: u64,
        owner: Owner,
        id: u64,
        element: String,
        result: Result<([u8; 32], u64, Preferences), String>,
    },
    Saved {
        serial: u64,
        owner: Owner,
        result: Result<(u64, Preferences), String>,
    },
}
fn message(event: Event) -> Message {
    Message::ApplicationSurface(event)
}
impl Shell {
    pub(crate) fn dock_owner(&self) -> Option<&Claim> {
        self.dock.claim.as_ref().filter(|claim| {
            claim.authority.load(Ordering::Acquire)
                && self.plugin.live.get(&claim.owner.id).is_some_and(|live| {
                    live.owner == claim.owner && live.authority.load(Ordering::Acquire)
                })
        })
    }
    pub(crate) fn dock_active(&self) -> bool {
        self.dock_owner()
            .is_some_and(|claim| claim.preferences.enabled)
    }
    pub(crate) fn dock_plugin_settings(&self) -> Task<Message> {
        self.dock_owner().map_or_else(Task::none, |claim| {
            Task::done(Message::SettingsPlugin(claim.owner.id.clone()))
        })
    }
    pub(crate) fn application_surface_request(
        &mut self,
        owner: Owner,
        id: u64,
        element: Option<String>,
        authority: Arc<AtomicBool>,
    ) -> Task<Message> {
        if element.is_none() {
            let owns = self.dock.claim.as_ref().is_some_and(|c| c.owner == owner)
                || self
                    .dock
                    .surface_element
                    .as_ref()
                    .is_some_and(|(o, _)| *o == owner);
            self.application_surface_revoke(&owner);
            if owns {
                self.dock.request_serial = self.dock.request_serial.wrapping_add(1);
            }
            self.plugin_reply(owner, id, true, "");
            return Task::none();
        }
        if self.dock_owner().is_some_and(|c| c.owner != owner) {
            self.plugin_reply(
                owner,
                id,
                false,
                tr("shell-another-plugin-owns-the-application-surface"),
            );
            return Task::none();
        }
        let Some(live) = self
            .plugin
            .live
            .get(&owner.id)
            .filter(|live| live.owner == owner)
        else {
            return Task::none();
        };
        let Some(element) = live.manifest.element_id(
            element
                .as_deref()
                .expect("this branch handles a selected element"),
        ) else {
            self.plugin_reply(
                owner,
                id,
                false,
                tr("shell-application-surface-must-use-a-declared-bar-element"),
            );
            return Task::none();
        };
        let manifest = std::sync::Arc::clone(&live.manifest);
        let store = match self
            .settings_store
            .clone()
            .ok_or_else(|| tr("shell-settings-store-is-unavailable").to_owned())
            .and_then(Store::for_settings)
        {
            Ok(store) => store,
            Err(e) => {
                self.plugin_reply(owner, id, false, &e);
                return Task::none();
            }
        };
        self.dock.request_serial = self.dock.request_serial.wrapping_add(1);
        let serial = self.dock.request_serial;
        self.dock.surface_element = Some((owner.clone(), element.clone()));
        let expected = owner.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if !authority.load(Ordering::Acquire) {
                        return Err(tr("shell-plugin-authority-changed").into());
                    }
                    let state = store.registry()?;
                    let record = state
                        .entries
                        .iter()
                        .find(|record| {
                            record.id() == expected.id
                                && record.activation() == expected.activation
                                && record.enabled()
                                && record.grants().contains(&Permission::Applications)
                                && record.grants().contains(&Permission::Bar)
                        })
                        .ok_or_else(|| tr("shell-plugin-authority-changed"))?;
                    let package = store.package(record)?;
                    if package.manifest() != manifest.as_ref() {
                        return Err(tr("shell-plugin-package-changed").into());
                    }
                    Ok((
                        record.digest(),
                        record.config_revision(),
                        applications::preferences(package.manifest(), record.config())?,
                    ))
                })
                .await
                .map_err(|_| tr("shell-application-surface-reader-stopped").to_owned())?
            },
            move |result| {
                message(Event::Loaded {
                    serial,
                    owner: owner.clone(),
                    id,
                    element,
                    result,
                })
            },
        )
    }
    pub(crate) fn application_surface_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Loaded {
                serial,
                owner,
                id,
                element,
                result,
            } => {
                let Some(authority) = self
                    .plugin
                    .live
                    .get(&owner.id)
                    .filter(|live| live.owner == owner && live.authority.load(Ordering::Acquire))
                    .map(|live| std::sync::Arc::clone(&live.authority))
                else {
                    return Task::none();
                };
                if serial != self.dock.request_serial {
                    self.plugin_reply(
                        owner,
                        id,
                        false,
                        tr("shell-application-surface-request-superseded"),
                    );
                    return Task::none();
                }
                match result {
                    Ok((digest, revision, preferences)) => {
                        if self.dock_owner().is_some_and(|c| c.owner != owner) {
                            self.plugin_reply(
                                owner,
                                id,
                                false,
                                tr("shell-another-plugin-owns-the-application-surface"),
                            );
                            return Task::none();
                        }
                        if self
                            .dock
                            .claim
                            .as_ref()
                            .is_none_or(|c| c.owner != owner || revision >= c.revision)
                        {
                            self.dock.claim = Some(Claim {
                                owner: owner.clone(),
                                element,
                                digest,
                                revision,
                                preferences,
                                authority,
                            });
                        }
                        self.plugin_reply(owner, id, true, "");
                    }
                    Err(error) => {
                        if let Some(claim) = self.dock.claim.as_mut().filter(|c| c.owner == owner) {
                            claim.preferences.enabled = false;
                        }
                        self.plugin.error =
                            Some(tr!("shell-application-surface", "arg0" => error.clone()));
                        self.plugin_reply(owner, id, false, &error);
                    }
                }
                Task::none()
            }
            Event::Saved {
                serial,
                owner,
                result,
            } => {
                if self.dock.writing.as_ref() != Some(&(owner.clone(), serial)) {
                    return Task::none();
                }
                self.dock.writing = None;
                if self.dock_owner().is_none_or(|c| c.owner != owner) {
                    self.dock.writes.clear();
                    self.dock.pending_pin_order = None;
                    return Task::none();
                }
                match result {
                    Ok((revision, preferences)) => {
                        if let Some(claim) = self.dock.claim.as_mut()
                            && revision >= claim.revision
                        {
                            claim.revision = revision;
                            claim.preferences = preferences;
                        }
                        self.plugin_command(enzui_shell::plugins::service::Command::Refresh);
                    }
                    Err(error) => {
                        self.notice = Some(error);
                        self.dock.writes.clear();
                    }
                }
                if !self
                    .dock
                    .writes
                    .iter()
                    .any(|change| matches!(change, Change::ReorderPins(_)))
                {
                    self.dock.pending_pin_order = None;
                }
                self.dock_write_next()
            }
        }
    }
    pub(crate) fn dock_save(&mut self, change: Change) -> Task<Message> {
        if self.dock_owner().is_none() {
            self.dock.pending_pin_order = None;
            return Task::none();
        }
        self.dock.writes.push_back(change);
        self.dock_write_next()
    }
    fn dock_write_next(&mut self) -> Task<Message> {
        if self.dock.writing.is_some() {
            return Task::none();
        }
        let Some(claim) = self.dock_owner() else {
            self.dock.writes.clear();
            self.dock.pending_pin_order = None;
            return Task::none();
        };
        let owner = claim.owner.clone();
        let digest = claim.digest;
        let authority = std::sync::Arc::clone(&claim.authority);
        let Some(change) = self.dock.writes.pop_front() else {
            return Task::none();
        };
        let store = match self
            .settings_store
            .clone()
            .ok_or_else(|| tr("shell-settings-store-is-unavailable").to_owned())
            .and_then(Store::for_settings)
        {
            Ok(store) => store,
            Err(error) => {
                self.notice = Some(error);
                self.dock.pending_pin_order = None;
                return Task::none();
            }
        };
        self.dock.write_serial = self.dock.write_serial.wrapping_add(1);
        let serial = self.dock.write_serial;
        self.dock.writing = Some((owner.clone(), serial));
        let expected = owner.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let record = store.configure_active(
                        &expected.id,
                        digest,
                        expected.activation,
                        Permission::Applications,
                        |manifest, config| {
                            if !authority.load(Ordering::Acquire) {
                                return Err(tr("shell-plugin-authority-changed").into());
                            }
                            applications::edit(manifest, config, &change)
                        },
                    )?;
                    let package = store.package(&record)?;
                    Ok((
                        record.config_revision(),
                        applications::preferences(package.manifest(), record.config())?,
                    ))
                })
                .await
                .map_err(|_| tr("shell-application-surface-writer-stopped").to_owned())?
            },
            move |result| {
                message(Event::Saved {
                    serial,
                    owner,
                    result,
                })
            },
        )
    }
    pub(crate) fn application_surface_revoke(&mut self, owner: &Owner) {
        if self.dock.claim.as_ref().is_some_and(|c| &c.owner == owner) {
            self.dock.claim = None;
            self.dock.writes.clear();
            self.dock.writing = None;
            self.dock.pending_pin_order = None;
        }
        if self
            .dock
            .surface_element
            .as_ref()
            .is_some_and(|(o, _)| o == owner)
        {
            self.dock.surface_element = None;
        }
    }
}
