use super::super::{
    budget::MAX_WORKERS,
    manifest::{Package, Permission},
};
use super::{
    records::{MAX_INSTALLED, Record, Registry, Store},
    support::*,
};
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self},
};

impl Store {
    /// Seeds the collecting clipboard for new stores and the disabled application dock.
    /// Independent markers preserve removal and existing configuration.
    pub fn initialize_defaults(&self) -> Result<bool, String> {
        self.locked(|| {
            let clipboard = self.initialize_clipboard_locked()?;
            let dock = self.initialize_applications_locked()?;
            Ok(clipboard || dock)
        })
    }
    fn initialize_clipboard_locked(&self) -> Result<bool, String> {
        {
            let marker = self.root.join("defaults-v1.json");
            match fs::symlink_metadata(&marker) {
                Ok(_) => {
                    let value: Value =
                        serde_json::from_slice(&bytes(open_regular(&marker, true)?, 128)?)
                            .map_err(|_| tr("shell-invalid-default-plugin-marker"))?;
                    if value != json!({"version": 1}) {
                        return Err(tr("shell-invalid-default-plugin-marker").into());
                    }
                    return Ok(false);
                }
                Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
                Err(value) => return Err(error(value)),
            }
            let installed = match fs::symlink_metadata(self.root.join("state.json")) {
                Ok(_) => {
                    self.registry()?;
                    false
                }
                Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                    let package = bundled_clipboard()?;
                    let grants = BTreeSet::from([
                        Permission::Bar,
                        Permission::Panels,
                        Permission::ClipboardRead,
                        Permission::ClipboardWrite,
                    ]);
                    if package.manifest().id != "clipboard"
                        || package.manifest().permissions != grants
                        || package.manifest().defaults()["collect"] != true
                    {
                        return Err(tr("shell-bundled-clipboard-defaults-changed").into());
                    }
                    let state = Registry {
                        serial: 1,
                        entries: vec![Record {
                            id: "clipboard".into(),
                            digest: package.digest(),
                            enabled: true,
                            activation: 1,
                            config_revision: 1,
                            grants,
                            config: package.manifest().defaults(),
                            fault: None,
                        }],
                    };
                    let encoded = encode(&state);
                    parse(&encoded)?;
                    if let Some(settings) = &self.settings {
                        self.catalog(&state, Some(&package))
                            .reconcile(&settings.load()?.layout, false)?;
                    }
                    let path = self.package_path("clipboard", package.digest());
                    private_directory(
                        path.parent()
                            .ok_or_else(|| tr("shell-package-destination-has-no-parent"))?,
                    )?;
                    self.persist(&path, package.bytes())?;
                    self.persist(
                        &self.root.join("state.json"),
                        &serde_json::to_vec(&encoded).map_err(error)?,
                    )?;
                    true
                }
                Err(value) => return Err(error(value)),
            };
            self.persist(&marker, br#"{"version":1}"#)?;
            Ok(installed)
        }
    }

    /// Moves an older copy of the bundled clipboard to the one this shell
    /// carries, keeping its grants, settings and on/off state. A package
    /// asking for other permissions, or one not older, is left alone.
    pub fn refresh_bundled_clipboard(&self) -> Result<bool, String> {
        self.refresh_bundled(&bundled_clipboard()?)
    }
    pub(super) fn refresh_bundled(&self, bundled: &Package) -> Result<bool, String> {
        let registry = self.registry()?;
        let Some(record) = registry
            .entries
            .iter()
            .find(|record| record.id == bundled.manifest().id)
        else {
            return Ok(false);
        };
        let installed = self.package(record)?;
        if installed.manifest().permissions != bundled.manifest().permissions
            || !older(&installed.manifest().version, &bundled.manifest().version)
        {
            return Ok(false);
        }
        self.install(
            bundled,
            Some(record.digest),
            record.grants.clone(),
            record.enabled,
        )?;
        Ok(true)
    }

    pub fn install(
        &self,
        package: &Package,
        expected_previous: Option<[u8; 32]>,
        grants: BTreeSet<Permission>,
        enabled: bool,
    ) -> Result<Record, String> {
        if !grants.is_subset(&package.manifest().permissions) {
            return Err(tr("shell-unrequested-plugin-permission-cannot-be-granted").into());
        }
        self.transaction(|state| {
            let id = &package.manifest().id;
            let index = state.entries.iter().position(|r| r.id == *id);
            let old = index.map(|i| &state.entries[i]);
            if old.map(|r| r.digest) != expected_previous {
                return Err(tr("shell-installed-plugin-changed-review-the-package-again").into());
            }
            if index.is_none() && state.entries.len() >= MAX_INSTALLED {
                return Err(tr("shell-installed-plugin-limit-reached").into());
            }
            if enabled
                && state
                    .entries
                    .iter()
                    .filter(|r| r.enabled && r.id != *id)
                    .count()
                    >= MAX_WORKERS
            {
                return Err(tr("shell-active-plugin-limit-reached").into());
            }
            let mut config = package.manifest().defaults();
            if let Some(old) = old {
                for spec in &package.manifest().settings {
                    if let Some(value) = old.config.get(&spec.key).filter(|v| spec.accepts(v)) {
                        config[&spec.key] = value.clone();
                    }
                }
            }
            let record = Record {
                id: id.clone(),
                digest: package.digest(),
                enabled,
                activation: state.serial,
                config_revision: state.serial,
                grants,
                config,
                fault: None,
            };
            if let Some(i) = index {
                state.entries[i] = record.clone();
            } else {
                state.entries.push(record.clone());
            }
            if let Some(settings) = &self.settings {
                let catalog = self.catalog(state, Some(package));
                let current = settings.load()?.layout;
                catalog.reconcile(&current, false)?;
                catalog.reconcile(&current, true)?;
            }
            let path = self.package_path(id, package.digest());
            private_directory(
                path.parent()
                    .ok_or_else(|| tr("shell-package-destination-has-no-parent"))?,
            )?;
            self.persist(&path, package.bytes())?;
            Ok(record)
        })
    }
    pub fn enabled(&self, id: &str, hash: [u8; 32], enabled: bool) -> Result<Record, String> {
        self.transaction(|state| {
            if enabled
                && state
                    .entries
                    .iter()
                    .filter(|r| r.id != id && r.enabled)
                    .count()
                    >= MAX_WORKERS
            {
                return Err(tr("shell-active-plugin-limit-reached").into());
            }
            let serial = state.serial;
            let record = state
                .entries
                .iter_mut()
                .find(|r| r.id == id && r.digest == hash)
                .ok_or_else(|| tr("shell-plugin-changed-reload-settings"))?;
            if enabled {
                self.package(record)?;
            }
            record.enabled = enabled;
            record.activation = serial;
            record.fault = None;
            Ok(record.clone())
        })
    }
    pub fn configure(
        &self,
        id: &str,
        hash: [u8; 32],
        key: &str,
        value: Value,
    ) -> Result<Record, String> {
        self.transaction(|state| {
            let serial = state.serial;
            let record = state
                .entries
                .iter_mut()
                .find(|r| r.id == id && r.digest == hash)
                .ok_or_else(|| tr("shell-plugin-changed-reload-settings"))?;
            let package = self.package(record)?;
            let spec = package
                .manifest()
                .settings
                .iter()
                .find(|s| s.key == key)
                .ok_or_else(|| tr("shell-unknown-plugin-setting"))?;
            if !spec.accepts(&value) {
                return Err(tr("shell-plugin-setting-is-outside-its-schema").into());
            }
            record.config[key] = value;
            record.config_revision = serial;
            Ok(record.clone())
        })
    }
    pub fn quarantine(
        &self,
        id: &str,
        hash: [u8; 32],
        activation: u64,
        fault: &str,
    ) -> Result<bool, String> {
        if !matches!(
            fault,
            "unavailable" | "exited" | "unresponsive" | "invalid-data" | "resource-limit"
        ) {
            return Err(tr("shell-invalid-plugin-fault-classification").into());
        }
        self.transaction(|state| {
            let serial = state.serial;
            if let Some(record) = state
                .entries
                .iter_mut()
                .find(|r| r.id == id && r.digest == hash && r.activation == activation && r.enabled)
            {
                record.enabled = false;
                record.activation = serial;
                record.fault = Some(fault.to_owned());
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
    pub fn remove(&self, id: &str, hash: [u8; 32], activation: u64) -> Result<(), String> {
        self.transaction(|state| {
            let index = state
                .entries
                .iter()
                .position(|r| r.id == id && r.digest == hash && r.activation == activation)
                .ok_or_else(|| tr("shell-plugin-changed-reload-settings"))?;
            state.entries.remove(index);
            Ok(())
        })
    }
}

fn bundled_clipboard() -> Result<Package, String> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../examples/plugins/clipboard/manifest.json"
    ))
    .map_err(|_| tr("shell-invalid-bundled-clipboard-manifest"))?;
    value["program"] = include_str!("../../../../examples/plugins/clipboard/main.py").into();
    Package::parse(&serde_json::to_vec(&value).map_err(error)?)
}

/// Whether dotted numeric version `installed` comes before `bundled`; other
/// version shapes are never replaced.
fn older(installed: &str, bundled: &str) -> bool {
    let parts = |version: &str| {
        version
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    matches!((parts(installed), parts(bundled)), (Some(a), Some(b)) if a < b)
}
