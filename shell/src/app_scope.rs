//! Launches installed, non-D-Bus apps in transient per-app user scopes.
//! Previews retain applications within their own managed scope.

use std::path::Path;

const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";

/// The scope for one launch of `app_id`; none when the id cannot name a unit.
#[must_use]
pub fn unit_name(app_id: &str, nonce: u32) -> Option<String> {
    let valid = !app_id.is_empty()
        && app_id.len() <= 200
        && app_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    valid.then(|| format!("app-enzui-{app_id}-{nonce:08x}.scope"))
}

fn nonce() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hasher.finish() as u32
}

/// The arguments that start a command in a new scope for `app_id`, or none
/// outside an installed login.
#[must_use]
pub fn prefix(app_id: &str) -> Vec<String> {
    if crate::desktop_session::mode() != crate::desktop_session::Mode::Installed
        || !Path::new(SYSTEMD_RUN).is_file()
    {
        return Vec::new();
    }
    let Some(unit) = unit_name(app_id, nonce()) else {
        return Vec::new();
    };
    [
        SYSTEMD_RUN,
        "--user",
        "--scope",
        "--collect",
        "--quiet",
        "--expand-environment=no",
        &format!("--unit={unit}"),
        "--",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// [`prefix`] as shell words for a compositor `exec_cmd` line (each word is
/// single-quoted), ending in a space when not empty.
#[must_use]
pub fn shell_prefix(app_id: &str) -> String {
    prefix(app_id)
        .iter()
        .map(|word| format!("'{}' ", word.replace('\'', "'\\''")))
        .collect()
}
