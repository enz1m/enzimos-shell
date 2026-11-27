use super::super::{
    applications,
    manifest::{Manifest, Permission},
};
use super::{
    records::{MAX_INSTALLED, Record, Store},
    support::*,
};
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs};

impl Record {
    pub(crate) fn dock_can_enable_from_search(&self) -> bool {
        if self.id != applications::DOCK_PLUGIN
            || (self.enabled && self.config["visible"] != false)
            || !self.grants.contains(&Permission::Applications)
            || !self.grants.contains(&Permission::Bar)
        {
            return false;
        }
        static DIGEST: std::sync::OnceLock<Option<[u8; 32]>> = std::sync::OnceLock::new();
        *DIGEST.get_or_init(|| applications::package().ok().map(|package| package.digest()))
            == Some(self.digest)
    }
}

impl Store {
    pub fn enable_dock_and_pin(&self, offer: &Record, id: String) -> Result<Record, String> {
        self.transaction(|state| {
            let active = state.entries.iter().filter(|record| record.enabled).count();
            let record = state
                .entries
                .iter_mut()
                .find(|record| {
                    record.dock_can_enable_from_search()
                        && record.digest == offer.digest
                        && record.activation == offer.activation
                })
                .ok_or_else(|| tr("shell-plugin-changed-reload-settings"))?;
            if !record.enabled && active >= super::super::budget::MAX_WORKERS {
                return Err(tr("shell-active-plugin-limit-reached").into());
            }
            let package = self.package(record)?;
            record.config = applications::edit(
                package.manifest(),
                &record.config,
                &crate::dock::Change::EnableAndPin(id),
            )?;
            record.config_revision = state.serial;
            if !record.enabled {
                record.enabled = true;
                record.activation = state.serial;
            }
            record.fault = None;
            Ok(record.clone())
        })
    }

    pub(super) fn initialize_applications_locked(&self) -> Result<bool, String> {
        let marker = self.root.join("defaults-app-dock-v1.json");
        match fs::symlink_metadata(&marker) {
            Ok(_) => {
                let value: Value =
                    serde_json::from_slice(&bytes(open_regular(&marker, true)?, 128)?)
                        .map_err(|_| tr("shell-invalid-app-dock-default-marker"))?;
                if value != json!({"version":1}) {
                    return Err(tr("shell-invalid-app-dock-default-marker").into());
                }
                return Ok(false);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(failure) => return Err(error(failure)),
        }
        let mut registry = self.registry()?;
        let exists = registry
            .entries
            .iter()
            .any(|record| record.id == applications::DOCK_PLUGIN);
        if !exists {
            if registry.entries.len() >= MAX_INSTALLED {
                return Ok(false);
            }
            let package = applications::package()?;
            let mut config = package.manifest().defaults();
            if let Some(settings) = self.settings.as_ref().and_then(|s| s.load().ok()) {
                let mut legacy = settings.dock;
                if legacy.position == crate::dock::Position::Bar {
                    legacy.position = if settings.bar == crate::settings::BarEdge::Top {
                        crate::dock::Position::Top
                    } else {
                        crate::dock::Position::Bottom
                    };
                }
                let value = serde_json::to_value(legacy)
                    .map_err(|_| tr("shell-invalid-old-dock-configuration"))?;
                for (key, value) in value
                    .as_object()
                    .ok_or_else(|| tr("shell-invalid-serialized-application-preferences"))?
                {
                    if key != "enabled" {
                        config[key] = value.clone();
                    }
                }
            }
            package.manifest().validate_config(&config)?;
            registry.serial = registry
                .serial
                .checked_add(1)
                .ok_or_else(|| tr("shell-plugin-state-revision-exhausted"))?;
            let record = Record {
                id: applications::DOCK_PLUGIN.into(),
                digest: package.digest(),
                enabled: false,
                activation: registry.serial,
                config_revision: registry.serial,
                grants: BTreeSet::from([Permission::Bar, Permission::Applications]),
                config,
                fault: None,
            };
            registry.entries.push(record);
            let encoded = encode(&registry);
            parse(&encoded)?;
            let data = serde_json::to_vec(&encoded).map_err(error)?;
            if data.len() > super::records::MAX_STATE_BYTES {
                return Ok(false);
            }
            let path = self.package_path(applications::DOCK_PLUGIN, package.digest());
            private_directory(
                path.parent()
                    .ok_or_else(|| tr("shell-package-destination-has-no-parent"))?,
            )?;
            self.persist(&path, package.bytes())?;
            self.persist(&self.root.join("state.json"), &data)?;
        }
        self.persist(&marker, br#"{"version":1}"#)?;
        Ok(!exists)
    }
    pub(super) fn upgrade_applications_locked(&self) -> Result<bool, String> {
        const BUNDLED_V1: &str = "2e25a35e0f3048e5a6c8f10ee841f6b66b197f07df6b93d0d704b86346bae4d7";

        const BUNDLED_V1_1: &str =
            "eea15915617416f8a4f72e7895fe1cdffcf2d7b718cf93412a6630b912b96a09";

        let mut registry = self.registry()?;
        let Some(record) = registry.entries.iter_mut().find(|record| {
            record.id == applications::DOCK_PLUGIN
                && matches!(
                    fingerprint(record.digest).as_str(),
                    BUNDLED_V1 | BUNDLED_V1_1
                )
        }) else {
            return Ok(false);
        };
        let package = applications::package()?;
        let mut config = package.manifest().defaults();
        for spec in &package.manifest().settings {
            if let Some(previous) = record.config.get(&spec.key).filter(|v| spec.accepts(v)) {
                config[&spec.key] = previous.clone();
            }
        }
        let legacy = fingerprint(record.digest) == BUNDLED_V1;
        if legacy
            && config["style"] == "island"
            && config["spacing"] == 4
            && config["radius"] == 12
            && config["margin"] == 8
        {
            config["style"] = "desktop".into();
        }
        if legacy {
            config["hover_previews"] = true.into();
        }
        package.manifest().validate_config(&config)?;
        registry.serial = registry
            .serial
            .checked_add(1)
            .ok_or_else(|| tr("shell-plugin-state-revision-exhausted"))?;
        record.digest = package.digest();
        record.activation = registry.serial;
        record.config_revision = registry.serial;
        record.config = config;
        record.fault = None;
        let encoded = encode(&registry);
        parse(&encoded)?;
        let data = serde_json::to_vec(&encoded).map_err(error)?;
        if data.len() > super::records::MAX_STATE_BYTES {
            return Err(tr("shell-plugin-state-exceeds-its-size-limit").into());
        }
        let path = self.package_path(applications::DOCK_PLUGIN, package.digest());
        private_directory(
            path.parent()
                .ok_or_else(|| tr("shell-package-destination-has-no-parent"))?,
        )?;
        self.persist(&path, package.bytes())?;
        self.persist(&self.root.join("state.json"), &data)?;
        Ok(true)
    }

    // Broker edits bind to the active installation, so a revoked or replaced
    // worker cannot write late configuration over the next activation.
    pub fn configure_active(
        &self,
        id: &str,
        digest: [u8; 32],
        activation: u64,
        permission: Permission,
        edit: impl FnOnce(&Manifest, &Value) -> Result<Value, String>,
    ) -> Result<Record, String> {
        self.transaction(|registry| {
            let serial = registry.serial;
            let record = registry
                .entries
                .iter_mut()
                .find(|record| {
                    record.id == id
                        && record.digest == digest
                        && record.activation == activation
                        && record.enabled
                        && record.grants.contains(&permission)
                })
                .ok_or_else(|| tr("shell-plugin-authority-changed"))?;
            let package = self.package(record)?;
            let config = edit(package.manifest(), &record.config)?;
            package.manifest().validate_config(&config)?;
            record.config = config;
            record.config_revision = serial;
            Ok(record.clone())
        })
    }
}
