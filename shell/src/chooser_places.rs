//! Read-only, bounded file-chooser places from user folders, pins, mounts and recent files.

use enzui_shell::i18n::tr;
use std::path::{Path, PathBuf};

/// A sidebar entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub icon: &'static str,
    pub path: PathBuf,
}

const HOME_ICON: &str = "house";
const COMPUTER_ICON: &str = "hard-drive";
const PINNED_ICON: &str = "folder";
const DRIVE_ICON: &str = "usb";
/// Every glyph this module names, for the bundled-icon test.
pub const ICONS: [&str; 10] = [
    HOME_ICON,
    COMPUTER_ICON,
    PINNED_ICON,
    DRIVE_ICON,
    "monitor",
    "file-text",
    "download",
    "image",
    "music",
    "video",
];

const USER_FOLDERS: [(&str, &str, &str, &str); 6] = [
    ("XDG_DESKTOP_DIR", "Desktop", "Desktop", "monitor"),
    ("XDG_DOCUMENTS_DIR", "Documents", "Documents", "file-text"),
    ("XDG_DOWNLOAD_DIR", "Downloads", "Downloads", "download"),
    ("XDG_PICTURES_DIR", "Pictures", "Pictures", "image"),
    ("XDG_MUSIC_DIR", "Music", "Music", "music"),
    ("XDG_VIDEOS_DIR", "Videos", "Videos", "video"),
];

const PINNED_MAX: usize = 32;
const RECENT_MAX: usize = 100;
const READ_MAX: u64 = 4 << 20;

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn config_home(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

fn data_home(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
}

fn read_bounded(path: &Path) -> String {
    let small = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= READ_MAX);
    if small {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Home, the user's folders from `user-dirs.dirs` (standard names otherwise).
pub fn standard(home: &Path, user_dirs: &str) -> Vec<Place> {
    let lookup = |key: &str, fallback: &str| {
        user_dirs
            .lines()
            .find_map(|line| {
                let value = line
                    .strip_prefix(key)?
                    .strip_prefix('=')?
                    .trim()
                    .trim_matches('"');
                let path = value.strip_prefix("$HOME").map_or_else(
                    || PathBuf::from(value),
                    |rest| home.join(rest.trim_start_matches('/')),
                );
                path.is_absolute().then_some(path)
            })
            .unwrap_or_else(|| home.join(fallback))
    };
    let mut places = vec![Place {
        label: tr("place-home").into(),
        icon: HOME_ICON,
        path: home.to_path_buf(),
    }];
    for (key, fallback, label, icon) in USER_FOLDERS {
        let path = lookup(key, fallback);
        if path != home && path.is_dir() {
            places.push(Place {
                label: crate::i18n::label(label).into(),
                icon,
                path,
            });
        }
    }
    places
}

pub fn user_folders() -> Vec<Place> {
    let home = home();
    standard(
        &home,
        &read_bounded(&config_home(&home).join("user-dirs.dirs")),
    )
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn file_uri(uri: &str) -> Option<PathBuf> {
    let path = PathBuf::from(percent_decode(uri.strip_prefix("file://")?)?);
    (path.is_absolute() && !path.as_os_str().is_empty()).then_some(path)
}

/// Folders pinned in Files (`label<TAB>path` lines) and in GTK apps
/// (`file:///path label` lines), in that order, without repeats.
pub fn pinned_from(files_app: &str, gtk: &str) -> Vec<Place> {
    let mut pinned: Vec<Place> = Vec::new();
    let from_files = files_app
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (label, path) = line.split_once('\t')?;
            let path = PathBuf::from(path.trim_end());
            path.is_absolute().then(|| (label.trim().to_owned(), path))
        });
    let from_gtk = gtk.lines().filter_map(|line| {
        let (uri, label) = line
            .split_once(' ')
            .map_or((line, ""), |(uri, label)| (uri, label));
        let path = file_uri(uri.trim())?;
        Some((label.trim().to_owned(), path))
    });
    for (label, path) in from_files.chain(from_gtk) {
        if pinned.len() >= PINNED_MAX || pinned.iter().any(|p| p.path == path) {
            continue;
        }
        let label = if label.is_empty() {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        } else {
            label
        };
        pinned.push(Place {
            label: label.chars().take(80).collect(),
            icon: PINNED_ICON,
            path,
        });
    }
    pinned
}

/// Pinned folders that exist and are not already a standard place.
pub fn pinned(standard: &[Place]) -> Vec<Place> {
    let home = home();
    let config = config_home(&home);
    pinned_from(
        &read_bounded(&config.join("files-app/bookmarks")),
        &read_bounded(&config.join("gtk-3.0/bookmarks")),
    )
    .into_iter()
    .filter(|place| place.path.is_dir() && !standard.iter().any(|s| s.path == place.path))
    .collect()
}

/// Mounted removable drives (`/run/media/$USER/*`, `/media/$USER/*`), then
/// the whole computer.
pub fn devices() -> Vec<Place> {
    let user = home()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut drives: Vec<Place> = Vec::new();
    if !user.is_empty() {
        for root in [
            PathBuf::from("/run/media").join(&user),
            PathBuf::from("/media").join(&user),
        ] {
            let Ok(listing) = std::fs::read_dir(&root) else {
                continue;
            };
            for item in listing.flatten().take(32) {
                let path = item.path();
                if path.is_dir() {
                    drives.push(Place {
                        label: item.file_name().to_string_lossy().into_owned(),
                        icon: DRIVE_ICON,
                        path,
                    });
                }
            }
        }
    }
    drives.sort_by(|a, b| a.label.cmp(&b.label));
    drives.push(Place {
        label: tr("shell-computer").into(),
        icon: COMPUTER_ICON,
        path: PathBuf::from("/"),
    });
    drives
}

/// Files from `recently-used.xbel`, most recently used first: their paths
/// and when they were used (RFC 3339, as written).
pub fn recent_from(xbel: &str) -> Vec<(PathBuf, String)> {
    let mut recent: Vec<(PathBuf, String)> = Vec::new();
    for tag in xbel.split("<bookmark ").skip(1) {
        let Some(head) = tag.split('>').next() else {
            continue;
        };
        let attribute = |name: &str| {
            let start = head.find(&format!("{name}=\""))? + name.len() + 2;
            Some(head.get(start..)?.split('"').next()?.replace("&amp;", "&"))
        };
        let Some(path) = attribute("href").as_deref().and_then(file_uri) else {
            continue;
        };
        let when = attribute("visited")
            .or_else(|| attribute("modified"))
            .unwrap_or_default();
        recent.retain(|(seen, _)| *seen != path);
        recent.push((path, when));
    }
    recent.sort_by(|a, b| b.1.cmp(&a.1));
    recent.truncate(RECENT_MAX);
    recent
}

/// Recently used files that still exist.
pub fn recent() -> Vec<PathBuf> {
    recent_from(&read_bounded(
        &data_home(&home()).join("recently-used.xbel"),
    ))
    .into_iter()
    .map(|(path, _)| path)
    .filter(|path| path.is_file())
    .collect()
}

/// Whether `folder` holds configuration or other hidden data, such as `~/.config/autostart`,
/// where a written file can change how the session behaves.
pub fn is_sensitive(folder: &Path, home: &Path) -> bool {
    let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let config = std::fs::canonicalize(config_home(&home)).unwrap_or_else(|_| config_home(&home));
    folder.starts_with(&config)
        || folder.strip_prefix(&home).is_ok_and(|inside| {
            inside
                .components()
                .any(|part| part.as_os_str().as_encoded_bytes().starts_with(b"."))
        })
}

/// Resolves typed paths lexically; relative paths start in `folder`, and `//` or `/~/` restarts
/// the path.
pub fn resolve(typed: &str, folder: &Path, home: &Path) -> Option<PathBuf> {
    let mut typed = typed.trim();
    // Typing `/` or `~/` after the prefilled folder starts over, as in GTK.
    if !typed.starts_with("file://") {
        if let Some(at) = typed.rfind("/~/") {
            typed = typed.get(at + 1..)?;
        }
        if let Some(at) = typed.rfind("//") {
            typed = typed.get(at + 1..)?;
        }
    }
    if typed.is_empty() {
        return None;
    }
    let joined = if typed == "~" {
        home.to_path_buf()
    } else if let Some(rest) = typed.strip_prefix("~/") {
        home.join(rest)
    } else if let Some(uri) = typed.strip_prefix("file://") {
        PathBuf::from(percent_decode(uri)?)
    } else {
        folder.join(typed)
    };
    let mut clean = PathBuf::from("/");
    for part in joined.components() {
        match part {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::Normal(name) => clean.push(name),
            _ => {}
        }
    }
    Some(clean)
}

/// Why a new folder cannot have this name, if it cannot.
pub fn folder_name_problem(name: &str, folder: &Path) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        Some(tr("shell-type-a-name-for-the-new-folder"))
    } else if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Some(tr("shell-folder-names-can-t-contain-slashes"))
    } else if name.len() > 255 {
        Some(tr("shell-this-name-is-too-long"))
    } else if folder.join(name).symlink_metadata().is_ok() {
        Some(tr("shell-something-with-this-name-is-already-here"))
    } else {
        None
    }
}

/// The breadcrumb trail for `folder`: labels and the folder each opens,
/// starting at Home for folders inside it.
pub fn trail(folder: &Path, home: &Path) -> (Vec<String>, Vec<PathBuf>) {
    let (mut labels, mut paths, rest) = match folder.strip_prefix(home) {
        Ok(rest) if home != Path::new("/") => (
            vec![tr("place-home").to_owned()],
            vec![home.to_path_buf()],
            rest.to_path_buf(),
        ),
        _ => (
            vec!["Computer".to_owned()],
            vec![PathBuf::from("/")],
            folder
                .strip_prefix("/")
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        ),
    };
    let mut at = paths[0].clone();
    for part in rest.components() {
        at.push(part);
        labels.push(part.as_os_str().to_string_lossy().into_owned());
        paths.push(at.clone());
    }
    (labels, paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_places_behaviour() {
        // Recent files are decoded deduplicated and newest first.
        {
            let xbel = r#"<xbel>
            <bookmark href="file:///home/a/old.txt" added="x" modified="2026-09-01T10:00:00Z" visited="2026-09-01T10:00:00Z"><info/></bookmark>
            <bookmark href="file:///home/a/R%26D%20notes.md" modified="2026-09-03T10:00:00Z"></bookmark>
            <bookmark href="https://example.org/page" visited="2026-09-09T10:00:00Z"></bookmark>
            <bookmark href="file:///home/a/old.txt" visited="2026-09-05T10:00:00Z"></bookmark>
        </xbel>"#;
            let recent = recent_from(xbel);
            assert_eq!(
                recent
                    .iter()
                    .map(|(p, _)| p.to_str().unwrap())
                    .collect::<Vec<_>>(),
                ["/home/a/old.txt", "/home/a/R&D notes.md"]
            );
            assert!(recent_from("<bookmark href=\"file:///bad%zz\">").is_empty());
        }
        // New folder names are checked before creating.
        {
            let folder = tempfile::tempdir().unwrap();
            std::fs::create_dir(folder.path().join("taken")).unwrap();
            for bad in ["", "  ", ".", "..", "a/b", "taken"] {
                assert!(folder_name_problem(bad, folder.path()).is_some(), "{bad:?}");
            }
            assert_eq!(folder_name_problem("New project", folder.path()), None);
        }
    }
}
