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

impl Store {
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
