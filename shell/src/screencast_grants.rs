//! Screen-sharing grants the user allowed to outlive a restart.
//!
//! The screen-cast portal backend owns the file and its tokens; this module
//! lists the grants without their tokens and revokes them. Both sides hold an
//! exclusive `flock` on the sibling `.lock` file while they rewrite the file,
//! and the backend reads it on every restore, so a revocation applies to the
//! next request.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

const HEADER: &str = "enzim-screencast-grants 1";
const FIELDS: usize = 14;
const MAX_BYTES: u64 = 1 << 20;

/// What a grant shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Screen,
    /// A rectangle of the screen, in its pixels.
    Region {
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
}

/// One remembered grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub app_id: String,
    pub source: Source,
    /// The connector, such as `DP-1`.
    pub output: String,
    /// The monitor's make, model and serial as the compositor reports them.
    pub monitor: String,
    /// Remote-desktop input allowed without asking: 1 keyboard, 2 pointer.
    pub devices: u32,
    /// Seconds since the Unix epoch.
    pub issued: u64,
}

impl Grant {
    #[must_use]
    pub fn keyboard(&self) -> bool {
        self.devices & 1 != 0
    }
    #[must_use]
    pub fn pointer(&self) -> bool {
        self.devices & 2 != 0
    }
}

/// The grants file and the result of reading it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub grants: Vec<Grant>,
    pub error: Option<String>,
}

/// `$XDG_STATE_HOME/xdg-desktop-portal-hyprland/screencast-grants`, as the
/// backend computes it.
#[must_use]
pub fn path() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let base =
        absolute("XDG_STATE_HOME").or_else(|| absolute("HOME").map(|h| h.join(".local/state")))?;
    Some(base.join("xdg-desktop-portal-hyprland/screencast-grants"))
}

/// Lists the grants; a missing file lists none.
#[must_use]
pub fn read(path: &Path) -> Listing {
    let result = locked(path, false).and_then(|_lock| match load(path)? {
        Some(text) => Ok(parse(&text)),
        None => Ok(Vec::new()),
    });
    match result {
        Ok(grants) => Listing {
            grants,
            error: None,
        },
        Err(error) => Listing {
            grants: Vec::new(),
            error: Some(error),
        },
    }
}

/// Removes the grant with this id. Revoking a grant that is gone succeeds.
///
/// # Errors
///
/// When the file cannot be locked, is not private to this user, or cannot be
/// rewritten.
pub fn revoke(path: &Path, id: &str) -> Result<(), String> {
    let _lock = locked(path, true)?;
    let Some(text) = load(path)? else {
        return Ok(());
    };
    let mut kept = String::new();
    let mut removed = false;
    for line in text.lines() {
        if line_id(line).as_deref() == Some(id) {
            removed = true;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if !removed {
        return Ok(());
    }
    let dir = path.parent().ok_or("The grants file has no folder")?;
    let tmp = dir.join(format!(
        ".screencast-grants.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        file.write_all(kept.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("The grant could not be removed: {e}")
    })
}

// Holds the backend's lock file for the caller's read or rewrite.
fn locked(path: &Path, exclusive: bool) -> Result<Option<File>, String> {
    let Some(dir) = path.parent() else {
        return Err("The grants file has no folder".into());
    };
    if !dir.exists() {
        return Ok(None);
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    let uid = rustix::process::getuid().as_raw();
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o022 != 0 {
        return Err("The grants folder is not private to this account".into());
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(PathBuf::from(name))
        .map_err(|e| format!("The grants file cannot be locked: {e}"))?;
    if exclusive {
        lock.lock()
    } else {
        lock.lock_shared()
    }
    .map_err(|e| format!("The grants file cannot be locked: {e}"))?;
    Ok(Some(lock))
}

// The file's text, or None when there is no file.
fn load(path: &Path) -> Result<Option<String>, String> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("The grants file cannot be read: {e}")),
    };
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let uid = rustix::process::getuid().as_raw();
    // The backend ignores a file anyone else could have written.
    if !meta.is_file() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err("The grants file is not private to this account".into());
    }
    if meta.len() > MAX_BYTES {
        return Err("The grants file is too large".into());
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("The grants file cannot be read: {e}"))?;
    if !text.is_empty() && text.lines().next() != Some(HEADER) {
        return Err("The grants file has an unknown format".into());
    }
    Ok(Some(text))
}

fn unescape(field: &str) -> Option<String> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = field.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn line_id(line: &str) -> Option<String> {
    let mut fields = line.split('\t');
    (fields.next() == Some("grant")).then_some(())?;
    unescape(fields.next()?)
}

fn parse(text: &str) -> Vec<Grant> {
    text.lines().skip(1).filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Grant> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != FIELDS || fields[0] != "grant" {
        return None;
    }
    let number = |i: usize| fields[i].parse::<u32>().ok();
    let source = match fields[4] {
        "output" => Source::Screen,
        "region" => Source::Region {
            x: number(7)?,
            y: number(8)?,
            w: number(9)?,
            h: number(10)?,
        },
        _ => return None,
    };
    Some(Grant {
        id: unescape(fields[1])?,
        app_id: unescape(fields[3])?,
        source,
        output: unescape(fields[5])?,
        monitor: unescape(fields[6])?,
        devices: number(12)?,
        issued: fields[13].parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A line in the backend's format; the token column must never surface.
    const SCREEN: &str = "grant\tab12\tSECRET-TOKEN\torg.obs.Studio\toutput\tDP-1\tDell Inc. U2720Q 7ZX1 (DP-1)\t0\t0\t0\t0\t1\t0\t1764000000";
    const REGION: &str = "grant\tcd34\tOTHER-TOKEN\tcom.rustdesk.RustDesk\tregion\tHDMI-A-1\tOdd%09name\t10\t20\t640\t480\t0\t3\t1764000100";

    fn fixture(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = dir.path().join("xdg-desktop-portal-hyprland");
        std::fs::create_dir(&folder).expect("folder");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let path = folder.join("screencast-grants");
        let mut text = format!("{HEADER}\n");
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(&path, text).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
        (dir, path)
    }

    #[test]
    fn lists_grants_without_tokens() {
        let (_dir, path) = fixture(&[SCREEN, REGION, "grant\tbroken"]);
        let listing = read(&path);
        assert_eq!(listing.error, None);
        assert_eq!(listing.grants.len(), 2);
        assert_eq!(listing.grants[0].source, Source::Screen);
        assert_eq!(listing.grants[0].monitor, "Dell Inc. U2720Q 7ZX1 (DP-1)");
        assert_eq!(
            listing.grants[1].source,
            Source::Region {
                x: 10,
                y: 20,
                w: 640,
                h: 480
            }
        );
        assert_eq!(listing.grants[1].monitor, "Odd\tname");
        assert!(listing.grants[1].keyboard() && listing.grants[1].pointer());
        assert!(!format!("{listing:?}").contains("TOKEN"));
    }

    #[test]
    fn revoke_removes_only_that_grant_and_keeps_the_file_private() {
        let (_dir, path) = fixture(&[SCREEN, REGION]);
        revoke(&path, "ab12").expect("revoke");
        let grants = read(&path).grants;
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].id, "cd34");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(
            text.starts_with(HEADER)
                && !text.contains("SECRET-TOKEN")
                && text.contains("OTHER-TOKEN")
        );
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        revoke(&path, "ab12").expect("revoking again succeeds");
    }

    #[test]
    fn missing_file_lists_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("none/screencast-grants");
        assert_eq!(read(&path), Listing::default());
        revoke(&path, "ab12").expect("nothing to revoke");
    }

    #[test]
    fn refuses_a_file_others_can_read() {
        let (_dir, path) = fixture(&[SCREEN]);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(read(&path).error.is_some());
        assert!(revoke(&path, "ab12").is_err());
    }

    #[test]
    fn refuses_an_unknown_format() {
        let (_dir, path) = fixture(&[]);
        std::fs::write(&path, "enzim-screencast-grants 2\n").expect("write");
        assert!(read(&path).error.is_some());
        assert!(revoke(&path, "ab12").is_err());
    }
}
