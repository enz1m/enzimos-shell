use super::super::{
    layout::Catalog,
    manifest::{MAX_PACKAGE_BYTES, Package, Permission},
};
use super::{
    records::{MAX_STATE_BYTES, Record, Registry, Store},
    support::*,
};
use crate::settings;
use enzui_shell::i18n::tr;
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

impl Store {
    pub fn at(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute() || root.file_name().is_none() {
            return Err(tr("shell-plugin-store-needs-an-explicit-absolute-directory").into());
        }
        Ok(Self {
            root,
            settings: None,
        })
    }
    pub fn for_settings(settings: settings::Store) -> Result<Self, String> {
        let root = settings
            .path()
            .parent()
            .ok_or_else(|| tr("shell-invalid-shell-settings-directory"))?
            .join("plugins");
        let mut store = Self::at(root)?;
        store.settings = Some(settings);
        Ok(store)
    }
    pub(super) fn catalog(&self, state: &Registry, proposed: Option<&Package>) -> Catalog {
        let mut catalog = Catalog::default();
        for record in &state.entries {
            if let Some(package) =
                proposed.filter(|p| p.digest() == record.digest && p.manifest().id == record.id)
            {
                catalog.insert(
                    &record.id,
                    Some(package.manifest()),
                    record.grants.contains(&Permission::Bar),
                );
            } else {
                let package = self.package(record).ok();
                catalog.insert(
                    &record.id,
                    package.as_ref().map(Package::manifest),
                    record.grants.contains(&Permission::Bar),
                );
            }
        }
        catalog
    }
    pub fn reconcile_layout(&self, reset: bool) -> Result<settings::Settings, String> {
        let settings = self
            .settings
            .as_ref()
            .ok_or_else(|| tr("shell-plugin-layout-needs-an-attached-settings-store"))?;
        self.locked(|| {
            let catalog = self.catalog(&self.registry()?, None);
            settings.transform_layout(|current| catalog.reconcile(current, reset))
        })
    }
    pub fn transform_layout_for(
        &self,
        id: &str,
        activation: u64,
        transform: impl FnOnce(
            &crate::bar_layout::BarLayout,
        ) -> Result<crate::bar_layout::BarLayout, String>,
    ) -> Result<settings::Settings, String> {
        let settings = self
            .settings
            .as_ref()
            .ok_or_else(|| tr("shell-plugin-layout-needs-an-attached-settings-store"))?;
        self.locked(|| {
            let authorized = self.registry()?.entries.into_iter().any(|record| {
                record.id == id
                    && record.activation == activation
                    && record.enabled
                    && record.grants.contains(&Permission::Layout)
            });
            if !authorized {
                return Err(tr("shell-plugin-layout-authority-changed").into());
            }
            settings.transform_layout(transform)
        })
    }
    pub(super) fn prepare(&self) -> Result<(), String> {
        private_directory(&self.root)?;
        private_directory(&self.root.join("packages"))
    }
    pub fn read_package(path: &Path) -> Result<Package, String> {
        Package::parse(&bytes(open_regular(path, false)?, MAX_PACKAGE_BYTES)?)
    }
    pub fn registry(&self) -> Result<Registry, String> {
        match fs::symlink_metadata(&self.root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
            Err(e) => return Err(error(e)),
            Ok(_) => check_directory(&self.root)?,
        }
        let path = self.root.join("state.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
            Err(e) => return Err(error(e)),
            Ok(_) => {}
        }
        let value: Value =
            serde_json::from_slice(&bytes(open_regular(&path, true)?, MAX_STATE_BYTES)?)
                .map_err(|_| tr("shell-invalid-plugin-state-json"))?;
        parse(&value)
    }
    pub(super) fn package_path(&self, id: &str, hash: [u8; 32]) -> PathBuf {
        self.root
            .join("packages")
            .join(id)
            .join(format!("{}.enzplugin", fingerprint(hash)))
    }
    pub fn package(&self, record: &Record) -> Result<Package, String> {
        check_directory(&self.root)?;
        check_directory(&self.root.join("packages"))?;
        check_directory(&self.root.join("packages").join(&record.id))?;
        let package = Package::parse(&bytes(
            open_regular(&self.package_path(&record.id, record.digest), true)?,
            MAX_PACKAGE_BYTES,
        )?)?;
        if package.manifest().id != record.id
            || package.digest() != record.digest
            || !record.grants.is_subset(&package.manifest().permissions)
        {
            return Err(tr("shell-installed-package-identity-or-grants-changed").into());
        }
        package.manifest().validate_config(&record.config)?;
        Ok(package)
    }
    pub(super) fn persist(&self, path: &Path, data: &[u8]) -> Result<(), String> {
        let directory = path
            .parent()
            .ok_or_else(|| tr("shell-invalid-plugin-data-destination"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(error)?;
        temporary.write_all(data).map_err(error)?;
        temporary.as_file().sync_all().map_err(error)?;
        temporary.persist(path).map_err(error)?;
        File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(error)?;
        Ok(())
    }
    pub(super) fn prune(&self, state: &Registry) -> Result<(), String> {
        for entry in fs::read_dir(self.root.join("packages")).map_err(error)? {
            let entry = entry.map_err(error)?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if valid_id(&id).is_err() || !entry.file_type().map_err(error)?.is_dir() {
                continue;
            }
            check_directory(&entry.path())?;
            let keep = state
                .entries
                .iter()
                .find(|r| r.id == id)
                .map(|r| format!("{}.enzplugin", fingerprint(r.digest)));
            for file in fs::read_dir(entry.path()).map_err(error)? {
                let file = file.map_err(error)?;
                let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if keep.as_deref() == Some(&name) {
                    continue;
                }
                if name.strip_suffix(".enzplugin").is_some_and(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                }) && !file.file_type().map_err(error)?.is_dir()
                {
                    fs::remove_file(file.path()).map_err(error)?;
                }
            }
            if keep.is_none() {
                let _ = fs::remove_dir(entry.path());
            }
        }
        Ok(())
    }
    pub(super) fn locked<T>(&self, apply: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.prepare()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join("lock"))
            .map_err(error)?;
        let meta = lock.metadata().map_err(error)?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != fs::metadata("/proc/self").map_err(error)?.uid()
            || meta.mode() & 0o077 != 0
        {
            return Err(tr("shell-plugin-lock-must-be-private-and-owned").into());
        }
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Err(tr("shell-plugin-storage-is-busy").into()),
            }
        }
        apply()
    }
    pub(super) fn transaction<T>(
        &self,
        apply: impl FnOnce(&mut Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        self.locked(|| {
            let mut registry = self.registry()?;
            self.prune(&registry)?;
            let original = encode(&registry);
            registry.serial = registry
                .serial
                .checked_add(1)
                .ok_or_else(|| tr("shell-plugin-state-revision-is-exhausted"))?;
            let result = apply(&mut registry)?;
            let value = encode(&registry);
            parse(&value)?;
            if value["entries"] == original["entries"] {
                return Ok(result);
            }
            let data = serde_json::to_vec(&value).map_err(error)?;
            if data.len() > MAX_STATE_BYTES {
                return Err(tr("shell-plugin-state-exceeds-its-size-limit").into());
            }
            self.persist(&self.root.join("state.json"), &data)?;
            let _ = self.prune(&registry);
            Ok(result)
        })
    }
}
