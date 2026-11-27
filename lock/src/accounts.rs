//! Who can sign in and into which session, for the greeter.

use enzui_shell::i18n;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct User {
    pub name: String,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    /// The `.desktop` file's stem, stable across renames of `Name`.
    pub id: String,
    pub name: String,
    pub exec: String,
    pub desktop_names: String,
    pub icon: Option<String>,
    /// The entry file, for the `%k` field code.
    pub path: PathBuf,
}

fn uid_range(login_defs: &str) -> (u32, u32) {
    let value = |key: &str| {
        login_defs.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(key))
                .then(|| words.next()?.parse().ok())
                .flatten()
        })
    };
    (
        value("UID_MIN").unwrap_or(1000),
        value("UID_MAX").unwrap_or(60000),
    )
}

// Local accounts that can log in, in uid order.
pub(crate) fn parse_users(passwd: &str, login_defs: &str) -> Vec<User> {
    let (min, max) = uid_range(login_defs);
    let mut users: Vec<(u32, User)> = passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, uid, _, gecos, _, shell] = fields.as_slice() else {
                return None;
            };
            let uid: u32 = uid.parse().ok()?;
            let shell = shell.trim();
            let usable = (min..=max).contains(&uid)
                && !name.is_empty()
                && !shell.is_empty()
                && !["nologin", "false"].iter().any(|s| shell.ends_with(s));
            let real = gecos.split(',').next().unwrap_or("").trim();
            usable.then(|| {
                (
                    uid,
                    User {
                        name: (*name).to_owned(),
                        display: if real.is_empty() {
                            (*name).to_owned()
                        } else {
                            real.to_owned()
                        },
                    },
                )
            })
        })
        .collect();
    users.sort_by_key(|(uid, _)| *uid);
    users.into_iter().map(|(_, user)| user).take(64).collect()
}

pub(crate) fn users() -> Vec<User> {
    parse_users(
        &std::fs::read_to_string("/etc/passwd").unwrap_or_default(),
        &std::fs::read_to_string("/etc/login.defs").unwrap_or_default(),
    )
}

// Reads one `[Desktop Entry]`; hidden or broken entries are skipped.
pub(crate) fn parse_session(id: &str, text: &str) -> Option<Session> {
    let mut in_entry = false;
    let (mut name, mut exec, mut names, mut hidden, mut icon) =
        (None, None, String::new(), false, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "Icon" => icon = Some(value.trim().to_owned()).filter(|i| !i.is_empty()),
            "DesktopNames" => names = value.trim().trim_end_matches(';').replace(';', ":"),
            "Hidden" | "NoDisplay" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    let locale = i18n::current().locale().to_string().replace('-', "_");
    let language = locale.split('_').next().unwrap_or("en");
    let mut localized = std::collections::BTreeMap::new();
    let mut active = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if active && let Some((key, value)) = line.split_once('=') {
            localized.entry(key.trim()).or_insert_with(|| value.trim());
        }
    }
    if let Some(value) = localized
        .get(format!("Name[{locale}]").as_str())
        .or_else(|| localized.get(format!("Name[{language}]").as_str()))
    {
        name = Some((*value).to_owned());
    }
    let exec = exec.filter(|e| !e.is_empty())?;
    (!hidden).then(|| Session {
        id: id.to_owned(),
        name: name.unwrap_or_else(|| id.to_owned()),
        exec,
        desktop_names: names,
        icon,
        path: PathBuf::new(),
    })
}

impl Session {
    // Expands Desktop Entry `Exec` quoting and field codes; returns an error for unsupported
    // entries.
    pub(crate) fn command(&self) -> Result<Vec<String>, String> {
        let path = self.path.to_string_lossy();
        let entry = crate::exec::Entry {
            name: &self.name,
            icon: self.icon.as_deref(),
            path: &path,
        };
        crate::exec::argv(&self.exec, &entry).map(|args| crate::exec::greetd_command(&args))
    }
}

fn session_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.split(':')
        .filter(|d| d.starts_with('/'))
        .map(|d| Path::new(d).join("wayland-sessions"))
        .collect()
}

// Wayland sessions, enzUI first; an earlier data directory wins.
pub(crate) fn sessions() -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();
    for dir in session_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if sessions.iter().any(|s| s.id == id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(mut session) = parse_session(id, &text) {
                session.path.clone_from(&path);
                sessions.push(session);
            }
        }
    }
    sessions.sort_by_key(|s| !s.id.starts_with("enzui"));
    sessions.truncate(32);
    sessions
}

// greetd may use `/` as HOME, so its cache falls back to the tmpfiles-managed directory.
fn state_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute() && h != Path::new("/"))
        .map(|h| h.join(".cache"));
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|c| c.is_absolute())
        .or(home)
        .unwrap_or_else(|| PathBuf::from("/var/cache"));
    Some(cache.join("enzui-greeter/last"))
}

// The user and session chosen at the last successful sign-in.
pub(crate) fn last() -> (Option<String>, Option<String>) {
    let text = state_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut lines = text
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty());
    (lines.next(), lines.next())
}

pub(crate) fn remember(user: &str, session: &str) {
    let Some(path) = state_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{user}\n{session}\n"));
}
