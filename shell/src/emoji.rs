//! Fully qualified Unicode emoji without skin-tone variants; recents persist in the account
//! state directory.

use std::path::{Path, PathBuf};

/// Where Unicode's emoji list is installed, under each data directory.
const DATA: &str = "unicode/emoji/emoji-test.txt";
const RECENT: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emoji {
    pub text: String,
    pub name: String,
    pub group: String,
}

/// Parses `emoji-test.txt`.
#[must_use]
pub fn parse(source: &str) -> Vec<Emoji> {
    let mut group = String::new();
    let mut found = Vec::new();
    for line in source.lines() {
        if let Some(name) = line.strip_prefix("# group: ") {
            name.trim().clone_into(&mut group);
            continue;
        }
        let Some((fields, comment)) = line.split_once('#') else {
            continue;
        };
        if fields.split(';').nth(1).map(str::trim) != Some("fully-qualified") {
            continue;
        }
        // "😀 E1.0 grinning face"
        let mut parts = comment.trim().splitn(3, ' ');
        let (Some(text), Some(_version), Some(name)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if name.contains("skin tone") || group == "Component" {
            continue;
        }
        found.push(Emoji {
            text: text.to_owned(),
            name: name.to_owned(),
            group: group.clone(),
        });
    }
    found
}

/// The OS's emoji, or none when its data is not installed.
#[must_use]
pub fn load() -> Vec<Emoji> {
    data_path(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("XDG_DATA_DIRS"),
    )
    .and_then(|path| std::fs::read_to_string(path).ok())
    .map(|source| parse(&source))
    .unwrap_or_default()
}

/// The first emoji list in the XDG data directories: the account's own,
/// then the system's (`/usr/local/share:/usr/share` when unset). Not only
/// `/usr/share`: some systems install it elsewhere and name that directory
/// in `XDG_DATA_DIRS`.
fn data_path(
    data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    data_dirs: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let home = data_home
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/share")));
    let system = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&system))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(DATA))
        .find(|path| path.is_file())
}

/// The emoji whose name has a word starting with each word of `query`.
#[must_use]
pub fn search<'a>(all: &'a [Emoji], query: &str) -> Vec<&'a Emoji> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    all.iter()
        .filter(|emoji| {
            let name = emoji.name.to_lowercase();
            let parts: Vec<&str> = name
                .split(|c: char| !c.is_alphanumeric())
                .filter(|part| !part.is_empty())
                .collect();
            words
                .iter()
                .all(|word| parts.iter().any(|part| part.starts_with(word.as_str())))
        })
        .collect()
}

fn recent_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(state.join("enzui/emoji-recent"))
}

#[must_use]
pub fn recent() -> Vec<String> {
    recent_path()
        .map(|path| read_recent(&path))
        .unwrap_or_default()
}

fn read_recent(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty() && line.chars().count() <= 16)
                .take(RECENT)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Puts `text` first in the recent list.
pub fn remember(text: &str) {
    if let Some(path) = recent_path() {
        let _ = write_recent(&path, text);
    }
}

fn write_recent(path: &Path, text: &str) -> std::io::Result<()> {
    let mut list = read_recent(path);
    list.retain(|item| item != text);
    list.insert(0, text.to_owned());
    list.truncate(RECENT);
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut temporary, (list.join("\n") + "\n").as_bytes())?;
    temporary
        .persist(path)
        .map(drop)
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_emoji_list_is_found_in_any_data_directory() {
        let root = tempfile::tempdir().unwrap();
        let (home, other) = (root.path().join("home"), root.path().join("other"));
        std::fs::create_dir_all(other.join("unicode/emoji")).unwrap();
        std::fs::write(other.join(DATA), "").unwrap();
        let dirs = std::env::join_paths([PathBuf::from("/nonexistent"), other.clone()]).unwrap();
        assert_eq!(
            data_path(Some(home.clone().into()), None, Some(dirs.clone())),
            Some(other.join(DATA))
        );
        // The account's own copy comes first.
        std::fs::create_dir_all(home.join("unicode/emoji")).unwrap();
        std::fs::write(home.join(DATA), "").unwrap();
        assert_eq!(
            data_path(Some(home.clone().into()), None, Some(dirs)),
            Some(home.join(DATA))
        );
    }
}
