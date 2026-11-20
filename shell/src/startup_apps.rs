use enzui_shell::i18n::tr;
use enzui_shell::tr;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const MAX_ENTRY: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub command: String,
    pub enabled: bool,
    pub reason: Option<String>,
    pub user_override: bool,
    pub can_enable: bool,
    pub review: Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub review: Review,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    source: PathBuf,
    digest: [u8; 32],
    destination_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub apps: Vec<App>,
    pub candidates: Vec<Candidate>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Store {
    user: PathBuf,
    system: Vec<PathBuf>,
    applications: Vec<PathBuf>,
    desktops: Vec<String>,
}

impl Store {
    pub fn from_environment() -> Result<Self, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|p| p.join(".config")))
            .ok_or_else(|| tr("shell-no-absolute-configuration-directory"))?;
        let roots = |key: &str, fallback: &str| {
            std::env::var(key)
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| fallback.into())
                .split(':')
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .collect::<Vec<_>>()
        };
        Ok(Self {
            user: config.join("autostart"),
            system: roots("XDG_CONFIG_DIRS", "/etc/xdg")
                .into_iter()
                .map(|p| p.join("autostart"))
                .collect(),
            applications: enzui_shell::search::catalogue::directories(),
            desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "enzUI:Hyprland".into())
                .split(':')
                .map(str::to_owned)
                .collect(),
        })
    }

    pub fn read(&self) -> Inventory {
        let mut inventory = Inventory::default();
        let mut seen = HashSet::new();
        for directory in std::iter::once(&self.user).chain(self.system.iter()) {
            for path in files(directory, &mut inventory.error) {
                let Some(id) = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| valid_id(s))
                    .map(str::to_owned)
                else {
                    continue;
                };
                if !seen.insert(id.clone()) {
                    continue;
                }
                match load(&path).and_then(|text| {
                    let original = Entry::parse(&text)?;
                    let source = if original.hidden && original.command.is_empty() {
                        self.system
                            .iter()
                            .chain(self.applications.iter())
                            .map(|directory| directory.join(&id))
                            .find_map(|path| {
                                load(&path).ok().and_then(|text| {
                                    Entry::parse(&text)
                                        .ok()
                                        .filter(|entry| !entry.command.is_empty())
                                        .map(|entry| (path, text, entry))
                                })
                            })
                    } else {
                        None
                    };
                    let (source_path, source_text, entry) =
                        source.unwrap_or_else(|| (path.clone(), text.clone(), original));
                    if !entry.visible_app() {
                        return Ok(None);
                    }
                    let reason = if entry.command.is_empty() {
                        Some(tr("shell-original-launch-entry-is-unavailable").into())
                    } else {
                        entry.reason(&self.desktops)
                    };
                    let review = self.review(&id, &source_path, &source_text)?;
                    Ok(Some(App {
                        id,
                        name: entry.name,
                        can_enable: !entry.command.is_empty(),
                        command: entry.command,
                        enabled: {
                            let current = Entry::parse(&text)?;
                            !current.hidden && !current.off_here()
                        },
                        reason,
                        user_override: directory == &self.user,
                        review,
                    }))
                }) {
                    Ok(Some(app)) => inventory.apps.push(app),
                    Ok(None) => {}
                    Err(error) => {
                        inventory.error = Some(
                            tr!("shell-some-startup-entries-could-not-be-read", "arg0" => error.clone()),
                        );
                    }
                }
            }
        }
        let mut candidates = HashSet::new();
        for directory in &self.applications {
            for path in files(directory, &mut inventory.error) {
                let Some(id) = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| valid_id(s))
                    .map(str::to_owned)
                else {
                    continue;
                };
                if !candidates.insert(id.clone()) || seen.contains(&id) {
                    continue;
                }
                let Ok(text) = load(&path) else {
                    continue;
                };
                let Ok(entry) = Entry::parse(&text) else {
                    continue;
                };
                if entry.hidden || !entry.visible_app() || entry.reason(&self.desktops).is_some() {
                    continue;
                }
                if let Ok(review) = self.review(&id, &path, &text) {
                    inventory.candidates.push(Candidate {
                        id,
                        name: entry.name,
                        review,
                    });
                }
            }
        }
        inventory
            .apps
            .sort_by_key(|app| (app.name.to_lowercase(), app.id.clone()));
        inventory
            .candidates
            .sort_by_key(|app| (app.name.to_lowercase(), app.id.clone()));
        inventory
    }

    fn review(&self, id: &str, source: &Path, text: &str) -> Result<Review, String> {
        let destination = self.user.join(id);
        let destination_digest = match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_file() => Some(digest(&load(&destination)?)),
            Ok(_) => {
                return Err(tr("shell-user-startup-entry-is-not-a-regular-file").into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        Ok(Review {
            source: source.to_owned(),
            digest: digest(text),
            destination_digest,
        })
    }

    pub fn set(
        &self,
        id: &str,
        review: &Review,
        enabled: bool,
        adding: bool,
    ) -> Result<(), String> {
        if !valid_id(id) {
            return Err(tr("shell-invalid-startup-application-id").into());
        }
        let source_allowed = std::iter::once(&self.user)
            .chain(self.system.iter())
            .chain(self.applications.iter())
            .any(|directory| review.source == directory.join(id));
        if !source_allowed {
            return Err(tr("shell-startup-source-is-outside-configured-directories").into());
        }
        let text = load(&review.source)?;
        if digest(&text) != review.digest || self.review(id, &review.source, &text)? != *review {
            return Err(tr(
                "shell-startup-entry-changed-review-its-current-settings-before-trying-again",
            )
            .into());
        }
        let entry = Entry::parse(&text)?;
        if enabled && entry.command.is_empty() {
            return Err(tr("shell-this-disabled-override-has-no-launch-command-restore-its-original-entry-befo").into());
        }
        if adding
            && (!enabled
                || review.destination_digest.is_some()
                || entry.hidden
                || entry.no_display
                || entry.reason(&self.desktops).is_some())
        {
            return Err(tr("shell-that-application-is-not-an-available-new-startup-entry").into());
        }
        // Turning an app off here lists enzUI in its NotShowIn rather than
        // hiding it, so the desktops the user also logs in to still start it.
        let mut fields: Vec<(&str, String)> = Vec::new();
        if adding {
            fields.push(("Hidden", "false".into()));
            fields.push(("OnlyShowIn", format!("{DESKTOP};")));
        } else {
            let mut others: String = entry
                .fields
                .get("NotShowIn")
                .map(|list| {
                    list.split(';')
                        .filter(|d| !d.is_empty() && *d != DESKTOP)
                        .map(|d| format!("{d};"))
                        .collect()
                })
                .unwrap_or_default();
            if !enabled {
                others.push_str(DESKTOP);
                others.push(';');
            }
            fields.push(("NotShowIn", others));
            if enabled && entry.hidden {
                fields.push(("Hidden", "false".into()));
            }
        }
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let output = rewrite(&text, &fields, adding);
        std::fs::create_dir_all(&self.user).map_err(|e| e.to_string())?;
        let mut staged = tempfile::NamedTempFile::new_in(&self.user).map_err(|e| e.to_string())?;
        use std::io::Write;
        staged
            .write_all(output.as_bytes())
            .map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        let current_source = load(&review.source)?;
        if digest(&current_source) != review.digest
            || self.review(id, &review.source, &current_source)? != *review
        {
            return Err(tr("shell-startup-entry-changed-while-saving").into());
        }
        if review.destination_digest.is_none() {
            staged
                .persist_noclobber(self.user.join(id))
                .map_err(|e| e.to_string())?;
        } else {
            staged
                .persist(self.user.join(id))
                .map_err(|e| e.to_string())?;
        }
        std::fs::File::open(&self.user)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// The desktop name this session sets in XDG_CURRENT_DESKTOP.
const DESKTOP: &str = "enzUI";

struct Entry {
    name: String,
    command: String,
    hidden: bool,
    no_display: bool,
    fields: BTreeMap<String, String>,
}

impl Entry {
    fn parse(text: &str) -> Result<Self, String> {
        let mut group = false;
        let mut found = false;
        let mut fields = BTreeMap::new();
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                group = line == "[Desktop Entry]";
                if group && std::mem::replace(&mut found, true) {
                    return Err(tr("shell-duplicate-desktop-entry-group").into());
                }
                continue;
            }
            if line.starts_with('#') || !group {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if fields
                .insert(key.trim().to_owned(), value.trim().to_owned())
                .is_some()
            {
                return Err(tr("shell-duplicate-desktop-entry-key").into());
            }
        }
        let hidden = fields.get("Hidden").is_some_and(|v| v == "true");
        if !hidden && fields.get("Type").map(String::as_str) != Some("Application") {
            return Err(tr("shell-not-an-application-desktop-entry").into());
        }
        let name = fields
            .get("Name")
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| hidden.then(|| tr("shell-disabled-startup-override").into()))
            .ok_or_else(|| tr("shell-startup-application-has-no-name"))?;
        let command = fields
            .get("Exec")
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| hidden.then(String::new))
            .ok_or_else(|| tr("shell-startup-application-has-no-command"))?;
        Ok(Self {
            name,
            command,
            hidden: fields.get("Hidden").is_some_and(|v| v == "true"),
            no_display: fields.get("NoDisplay").is_some_and(|v| v == "true"),
            fields,
        })
    }

    fn visible_app(&self) -> bool {
        !self.no_display
            && self
                .fields
                .get("X-GNOME-Autostart-Phase")
                .is_none_or(|phase| phase == "Applications")
    }

    /// Turned off in enzUI only: its NotShowIn lists enzUI.
    fn off_here(&self) -> bool {
        self.fields
            .get("NotShowIn")
            .is_some_and(|list| list.split(';').any(|d| d == DESKTOP))
    }

    fn reason(&self, desktops: &[String]) -> Option<String> {
        let matches = |value: &str| {
            value
                .split(';')
                .filter(|s| !s.is_empty())
                .any(|value| desktops.iter().any(|desktop| desktop == value))
        };
        // enzUI in NotShowIn is this page's own off switch, not a reason.
        let others_exclude = |value: &str| {
            value
                .split(';')
                .filter(|s| !s.is_empty() && *s != DESKTOP)
                .any(|value| desktops.iter().any(|desktop| desktop == value))
        };
        if self.fields.get("OnlyShowIn").is_some_and(|s| !matches(s))
            || self
                .fields
                .get("NotShowIn")
                .is_some_and(|s| others_exclude(s))
        {
            return Some(tr("shell-not-configured-for-this-desktop").into());
        }
        if self
            .fields
            .get("X-systemd-skip")
            .is_some_and(|s| s == "true")
        {
            return Some(tr("shell-skipped-by-the-login-startup-generator").into());
        }
        if self.fields.contains_key("X-GNOME-Autostart-Phase")
            && desktops.iter().any(|desktop| desktop == "GNOME")
        {
            return Some(
                tr("shell-gnome-startup-phase-is-handled-outside-this-login-generator").into(),
            );
        }
        if self
            .fields
            .get("X-GNOME-Autostart-enabled")
            .is_some_and(|s| s == "false")
        {
            return Some(tr("shell-disabled-by-its-desktop-specific-setting").into());
        }
        if self
            .fields
            .get("AutostartCondition")
            .is_some_and(|s| !s.is_empty())
            || self
                .fields
                .get("X-KDE-autostart-condition")
                .is_some_and(|s| !s.is_empty())
        {
            return Some(tr("shell-has-an-additional-startup-condition").into());
        }
        if let Some(program) = self.fields.get("TryExec").filter(|s| !s.is_empty())
            && !executable(program)
        {
            return Some(tr("shell-required-executable-is-unavailable").into());
        }
        None
    }
}

fn rewrite(text: &str, changes: &[(&str, &str)], adding: bool) -> String {
    let mut result = String::new();
    let mut group = false;
    let mut emitted = false;
    // An empty value removes the key.
    let append = |result: &mut String| {
        for (key, value) in changes.iter().filter(|(_, value)| !value.is_empty()) {
            let _ = writeln!(result, "{key}={value}");
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if group && !emitted {
                append(&mut result);
                emitted = true;
            }
            group = trimmed == "[Desktop Entry]";
        }
        let key = trimmed.split_once('=').map(|(key, _)| key.trim());
        if group
            && key.is_some_and(|key| {
                changes.iter().any(|(changed, _)| *changed == key) || adding && key == "NotShowIn"
            })
        {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    if group && !emitted {
        append(&mut result);
    }
    result
}

fn files(directory: &Path, error: &mut Option<String>) -> Vec<PathBuf> {
    let listing = match std::fs::read_dir(directory) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            *error =
                Some(tr!("shell-startup-directory-could-not-be-read", "arg0" => format!("{:}", e)));
            return Vec::new();
        }
    };
    let mut paths: Vec<_> = listing
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|s| s == "desktop"))
        .take(2001)
        .collect();
    if paths.len() > 2000 {
        *error =
            Some(tr("shell-startup-inventory-is-limited-to-2000-entries-per-directory").into());
        paths.truncate(2000);
    }
    paths.sort();
    paths
}

fn load(path: &Path) -> Result<String, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_ENTRY as u64 {
        return Err(tr("shell-desktop-entry-is-not-a-bounded-regular-file").into());
    }
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() > MAX_ENTRY as u64 {
        return Err(tr("shell-desktop-entry-is-not-a-bounded-regular-file").into());
    }
    let mut text = String::new();
    file.take(MAX_ENTRY as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_ENTRY {
        return Err(tr("shell-desktop-entry-exceeds-the-size-limit").into());
    }
    Ok(text)
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn valid_id(id: &str) -> bool {
    id.ends_with(".desktop")
        && id.len() > 8
        && id.len() <= 255
        && !id.starts_with('.')
        && !id.contains(['/', '\\'])
        && !id.chars().any(char::is_control)
}

fn executable(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let works = |p: PathBuf| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    let path = PathBuf::from(program);
    if path.is_absolute() {
        works(path)
    } else if program.contains('/') {
        false
    } else {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .any(|p| works(p.join(program)))
        })
    }
}

pub fn read() -> Inventory {
    Store::from_environment().map_or_else(
        |error| Inventory {
            error: Some(error),
            ..Default::default()
        },
        |store| store.read(),
    )
}

/// Example startup apps for the Settings preview; nothing is read or written.
#[must_use]
pub fn demo() -> Inventory {
    let review = |id: &str| Review {
        source: PathBuf::from(format!("/usr/share/applications/{id}")),
        digest: [0; 32],
        destination_digest: None,
    };
    let app = |id: &str, name: &str, enabled| App {
        id: id.into(),
        name: name.into(),
        command: String::new(),
        enabled,
        reason: None,
        user_override: false,
        can_enable: true,
        review: review(id),
    };
    Inventory {
        apps: vec![
            app("org.keepassxc.KeePassXC.desktop", "KeePassXC", true),
            app(
                "com.nextcloud.desktopclient.nextcloud.desktop",
                "Nextcloud",
                false,
            ),
        ],
        candidates: vec![Candidate {
            id: "org.mozilla.Thunderbird.desktop".into(),
            name: "Thunderbird".into(),
            review: review("org.mozilla.Thunderbird.desktop"),
        }],
        error: None,
    }
}

#[cfg(test)]
mod tests;
