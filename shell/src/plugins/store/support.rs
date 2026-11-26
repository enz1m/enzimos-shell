use super::super::{
    budget::MAX_WORKERS,
    manifest::{MAX_SETTINGS, Permission},
};
use super::records::{MAX_INSTALLED, Record, Registry};
use crate::bar_layout::Item;
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(super) fn error(_: impl std::fmt::Display) -> String {
    tr("shell-cannot-access-private-plugin-storage").into()
}
pub(super) fn valid_id(id: &str) -> Result<(), String> {
    Item::new(format!("plugin:{id}/entry"), true).map(|_| ())
}
pub(super) fn fingerprint(digest: [u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn digest(value: &Value) -> Result<[u8; 32], String> {
    let text = value
        .as_str()
        .ok_or_else(|| tr("shell-missing-plugin-content-identity"))?;
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(tr("shell-invalid-plugin-content-identity").into());
    }
    let mut digest = [0; 32];
    for (i, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let part = std::str::from_utf8(pair).map_err(error)?;
        digest[i] = u8::from_str_radix(part, 16).map_err(error)?;
    }
    Ok(digest)
}
pub(super) fn private_directory(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(error)?;
    check_directory(path)
}
pub(super) fn check_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    let uid = fs::metadata("/proc/self").map_err(error)?.uid();
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(tr("shell-plugin-storage-directory-must-be-private-and-owned").into());
    }
    Ok(())
}
pub(super) fn open_regular(path: &Path, private: bool) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(error)?;
    let meta = file.metadata().map_err(error)?;
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(tr("shell-plugin-data-must-be-a-regular-singly-linked-file").into());
    }
    if private
        && (meta.uid() != fs::metadata("/proc/self").map_err(error)?.uid()
            || meta.mode() & 0o077 != 0)
    {
        return Err(tr("shell-plugin-data-must-be-private-and-owned").into());
    }
    Ok(file)
}
pub(super) fn bytes(file: File, max: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > max {
        return Err(tr("shell-plugin-data-exceeds-its-size-limit").into());
    }
    Ok(bytes)
}
pub(super) fn fields(value: &Value, keys: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| tr("shell-invalid-plugin-state-object"))?;
    if object.len() != keys.len() || object.keys().any(|k| !keys.contains(&k.as_str())) {
        return Err(tr("shell-unknown-or-missing-plugin-state-field").into());
    }
    Ok(())
}
pub(super) fn counter(value: &Value) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| tr("shell-invalid-plugin-state-revision").into())
}
pub(super) fn parse(value: &Value) -> Result<Registry, String> {
    fields(value, &["format", "serial", "entries"])?;
    if value["format"].as_u64() != Some(1) {
        return Err(tr("shell-unsupported-plugin-state-format").into());
    }
    let serial = counter(&value["serial"])?;
    let rows = value["entries"]
        .as_array()
        .filter(|a| a.len() <= MAX_INSTALLED)
        .ok_or_else(|| tr("shell-too-many-installed-plugins"))?;
    let mut entries = Vec::new();
    let mut ids = BTreeSet::new();
    for row in rows {
        fields(
            row,
            &[
                "id",
                "digest",
                "enabled",
                "activation",
                "config_revision",
                "grants",
                "config",
                "fault",
            ],
        )?;
        let id = row["id"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-installed-plugin-identity"))?;
        valid_id(id)?;
        if !ids.insert(id) {
            return Err(tr("shell-duplicate-installed-plugin-identity").into());
        }
        let activation = counter(&row["activation"])?;
        let config_revision = counter(&row["config_revision"])?;
        if activation == 0
            || activation > serial
            || config_revision == 0
            || config_revision > serial
        {
            return Err(tr("shell-invalid-installed-plugin-revision").into());
        }
        let mut grants = BTreeSet::new();
        for grant in row["grants"]
            .as_array()
            .filter(|a| a.len() <= Permission::ALL.len())
            .ok_or_else(|| tr("shell-invalid-plugin-grants"))?
        {
            if !grants.insert(Permission::parse(grant)?) {
                return Err(tr("shell-duplicate-plugin-grant").into());
            }
        }
        let config = row["config"]
            .as_object()
            .filter(|o| o.len() <= MAX_SETTINGS)
            .ok_or_else(|| tr("shell-invalid-stored-plugin-settings"))?;
        if config.iter().any(|(k, v)| {
            if k == "folders" && id == super::super::applications::DOCK_PLUGIN {
                return !super::super::applications::valid_folders(v);
            }
            k.len() > 32
                || !(v.is_boolean()
                    || v.as_i64()
                        .is_some_and(|n| (-1_000_000..=1_000_000).contains(&n))
                    || v.as_str().is_some_and(|s| s.chars().count() <= 64)
                    || super::super::applications::valid_pins(v))
        }) {
            return Err(tr("shell-invalid-stored-plugin-setting-value").into());
        }
        let fault = if row["fault"].is_null() {
            None
        } else {
            Some(
                row["fault"]
                    .as_str()
                    .filter(|s| {
                        matches!(
                            *s,
                            "unavailable"
                                | "exited"
                                | "unresponsive"
                                | "invalid-data"
                                | "resource-limit"
                        )
                    })
                    .ok_or_else(|| tr("shell-invalid-plugin-fault-classification"))?
                    .to_owned(),
            )
        };
        entries.push(Record {
            id: id.to_owned(),
            digest: digest(&row["digest"])?,
            enabled: row["enabled"]
                .as_bool()
                .ok_or_else(|| tr("shell-invalid-plugin-activation-state"))?,
            activation,
            config_revision,
            grants,
            config: row["config"].clone(),
            fault,
        });
    }
    if entries.iter().filter(|r| r.enabled).count() > MAX_WORKERS {
        return Err(tr("shell-too-many-enabled-plugins").into());
    }
    Ok(Registry { serial, entries })
}
pub(super) fn encode(state: &Registry) -> Value {
    json!({"format":1,"serial":state.serial,"entries":state.entries.iter().map(|r|json!({"id":r.id,"digest":fingerprint(r.digest),"enabled":r.enabled,"activation":r.activation,"config_revision":r.config_revision,"grants":r.grants.iter().map(|p|p.key()).collect::<Vec<_>>(),"config":r.config,"fault":r.fault})).collect::<Vec<_>>()})
}
