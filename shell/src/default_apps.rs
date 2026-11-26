//! XDG MIME defaults from installed desktop entries; terminal choice uses compositor
//! preferences.

use enzui_shell::i18n::tr;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Web,
    Mail,
    Calendar,
    Music,
    Video,
    Photos,
    Files,
    Text,
    Terminal,
}

impl Kind {
    pub const ALL: [Self; 9] = [
        Self::Web,
        Self::Mail,
        Self::Calendar,
        Self::Music,
        Self::Video,
        Self::Photos,
        Self::Files,
        Self::Text,
        Self::Terminal,
    ];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Web => tr("shell-web"),
            Self::Mail => tr("shell-mail"),
            Self::Calendar => tr("shell-calendar"),
            Self::Music => tr("shell-music"),
            Self::Video => tr("shell-video"),
            Self::Photos => tr("shell-photos"),
            Self::Files => tr("shell-files"),
            Self::Text => tr("shell-text"),
            Self::Terminal => tr("shell-terminal"),
        }
    }
    /// The MIME types a choice is written for; the first decides the current one.
    #[must_use]
    pub fn types(self) -> &'static [&'static str] {
        match self {
            Self::Web => &[
                "x-scheme-handler/http",
                "x-scheme-handler/https",
                "text/html",
                "application/xhtml+xml",
            ],
            Self::Mail => &["x-scheme-handler/mailto"],
            Self::Calendar => &["text/calendar"],
            Self::Music => &[
                "audio/mpeg",
                "audio/flac",
                "audio/ogg",
                "audio/x-wav",
                "audio/mp4",
            ],
            Self::Video => &["video/mp4", "video/webm", "video/x-matroska", "video/ogg"],
            Self::Photos => &["image/jpeg", "image/png", "image/gif", "image/webp"],
            Self::Files => &["inode/directory"],
            Self::Text => &["text/plain", "text/markdown"],
            Self::Terminal => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// The desktop entry ID (`org.example.App.desktop`).
    pub id: String,
    pub name: String,
    /// The program alone, when the entry runs it without arguments (what a
    /// terminal choice needs).
    pub program: Option<String>,
    /// The entry's `Icon`, resolved through the icon theme when read.
    pub icon: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    pub current: BTreeMap<Kind, String>,
    pub apps: BTreeMap<Kind, Vec<App>>,
}

struct Entry {
    app: App,
    types: Vec<String>,
    terminal: bool,
    wm_class: Option<String>,
    path: PathBuf,
    // `X-Flatpak`: the Flatpak app ID this entry starts.
    flatpak: Option<String>,
    // The first word of `Exec`, the program it starts.
    exec: Option<String>,
    // `Exec` takes files or URLs (`%f`, `%F`, `%u`, `%U`).
    files: bool,
    development: bool,
    // `Icon`: a theme icon name or an absolute file.
    icon: Option<String>,
}

// Parses the `[Desktop Entry]` group; `None` for hidden or non-app entries.
fn entry(id: &str, text: &str) -> Option<Entry> {
    let mut group = false;
    let (mut name, mut exec, mut types, mut categories, mut wm_class, mut flatpak) =
        (None, None, Vec::new(), String::new(), None, None);
    let mut icon = None;
    let (mut application, mut hidden) = (false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| group) else {
            continue;
        };
        match key.trim() {
            "Type" => application = value.trim() == "Application",
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "Icon" => icon = Some(value.trim().to_owned()).filter(|icon| !icon.is_empty()),
            "MimeType" => {
                types = value
                    .split(';')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "Categories" => value.clone_into(&mut categories),
            "StartupWMClass" => {
                wm_class = Some(value.trim().to_owned()).filter(|class| !class.is_empty());
            }
            "X-Flatpak" => {
                flatpak =
                    Some(value.trim().to_owned()).filter(|id| !id.is_empty() && id.len() <= 255);
            }
            "NoDisplay" | "Hidden" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    if !application || hidden {
        return None;
    }
    let first = exec
        .as_deref()
        .and_then(|exec| exec.split_whitespace().next())
        .map(|word| word.trim_matches('"').to_owned());
    let program = exec.as_deref().and_then(|exec| {
        let words: Vec<&str> = exec
            .split_whitespace()
            .filter(|w| !w.starts_with('%'))
            .collect();
        (words.len() == 1 && !words[0].contains(['"', '\'', '$', '`'])).then(|| words[0].to_owned())
    });
    Some(Entry {
        app: App {
            id: id.to_owned(),
            name: name?.chars().filter(|c| !c.is_control()).take(64).collect(),
            program,
            icon: None,
        },
        icon,
        types,
        terminal: categories.split(';').any(|c| c == "TerminalEmulator"),
        wm_class,
        path: PathBuf::new(),
        flatpak,
        files: exec.as_deref().is_some_and(|exec| {
            exec.split_whitespace()
                .any(|w| matches!(w, "%f" | "%F" | "%u" | "%U"))
        }),
        development: categories
            .split(';')
            .any(|c| matches!(c, "Development" | "IDE")),
        exec: first,
    })
}

fn directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .into_iter()
        .chain(
            dirs.split(':')
                .map(PathBuf::from)
                .filter(|p| p.is_absolute()),
        )
        .map(|d| d.join("applications"))
        .collect()
}

fn entries() -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    for directory in directories() {
        let Ok(listing) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut files: Vec<_> = listing
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        files.sort();
        for path in files.into_iter().take(2000) {
            let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            // The first directory holding an ID wins, as in the spec.
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(mut entry) = std::fs::read_to_string(&path)
                .ok()
                .filter(|t| t.len() < 256 * 1024)
                .and_then(|t| entry(&id, &t))
            {
                entry.path = path;
                found.push(entry);
            }
        }
    }
    found
}

/// Maps lowercased StartupWMClass, or desktop ID fallback, to installed-entry metadata.
#[must_use]
pub fn by_window_class() -> std::collections::HashMap<String, (String, PathBuf, String)> {
    let mut index = std::collections::HashMap::new();
    for entry in entries() {
        let class = entry
            .wm_class
            .clone()
            .unwrap_or_else(|| entry.app.id.trim_end_matches(".desktop").to_owned());
        index
            .entry(class.to_lowercase())
            .or_insert((entry.app.id, entry.path, class));
    }
    index
}

/// A shown app as a parental app filter names it: its Flatpak ID, or the
/// absolute program its `Exec` starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launchable {
    pub id: String,
    pub name: String,
    pub flatpak: Option<String>,
    pub program: Option<PathBuf>,
}

#[must_use]
pub fn launchable() -> Vec<Launchable> {
    let mut found: Vec<Launchable> = Vec::new();
    for entry in entries() {
        let program = entry
            .exec
            .as_deref()
            .and_then(crate::parental::program_path);
        let item = Launchable {
            id: entry.app.id,
            name: entry.app.name,
            flatpak: entry.flatpak,
            program,
        };
        if (item.flatpak.is_some() || item.program.is_some())
            && !found
                .iter()
                .any(|known| known.flatpak == item.flatpak && known.program == item.program)
        {
            found.push(item);
        }
    }
    found.sort_by_key(|app| app.name.to_lowercase());
    found
}

/// The Flatpak app ID and program path of the desktop entry at `path`.
#[must_use]
pub fn identity(path: &std::path::Path) -> (Option<String>, Option<PathBuf>) {
    let id = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match std::fs::read_to_string(path)
        .ok()
        .filter(|t| t.len() < 256 * 1024)
        .and_then(|text| entry(id, &text))
    {
        Some(entry) => (
            entry.flatpak,
            entry
                .exec
                .as_deref()
                .and_then(crate::parental::program_path),
        ),
        None => (None, None),
    }
}

/// The `Name` of an installed desktop entry, by ID with or without `.desktop`.
#[must_use]
pub fn name(id: &str) -> Option<String> {
    let id = if id.ends_with(".desktop") {
        id.to_owned()
    } else {
        format!("{id}.desktop")
    };
    if id.contains('/') || id.starts_with('.') {
        return None;
    }
    directories().into_iter().find_map(|directory| {
        let text = std::fs::read_to_string(directory.join(&id))
            .ok()
            .filter(|t| t.len() < 256 * 1024)?;
        entry(&id, &text).map(|entry| entry.app.name)
    })
}

/// The files and folders `xdg-mime query default` looks through: the
/// association lists in the configuration and data folders and the
/// applications folders whose entries it checks.
fn association_sources() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let split = |name: &str, default: &str| -> Vec<PathBuf> {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default.to_owned())
            .split(':')
            .map(PathBuf::from)
            .collect()
    };
    let lists: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|desktop| !desktop.is_empty())
        .map(|desktop| format!("{}-mimeapps.list", desktop.to_lowercase()))
        .chain(["mimeapps.list".to_owned()])
        .collect();
    let mut paths = Vec::new();
    for dir in config_home
        .into_iter()
        .chain(split("XDG_CONFIG_DIRS", "/etc/xdg"))
    {
        paths.extend(lists.iter().map(|list| dir.join(list)));
        paths.push(dir);
    }
    for dir in data_home
        .into_iter()
        .chain(split("XDG_DATA_DIRS", "/usr/local/share:/usr/share"))
    {
        let applications = dir.join("applications");
        for name in lists
            .iter()
            .map(String::as_str)
            .chain(["defaults.list", "mimeinfo.cache"])
        {
            paths.push(applications.join(name));
        }
        paths.push(applications);
    }
    paths
}

/// Modification times of [`association_sources`]: a folder's time changes as
/// lists and entries are added to or removed from it, a list's as it is
/// edited.
fn association_stamp() -> Vec<Option<(i64, i64)>> {
    stamp_of(&association_sources())
}

fn stamp_of(paths: &[std::path::PathBuf]) -> Vec<Option<(i64, i64)>> {
    use std::os::unix::fs::MetadataExt;
    paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .ok()
                .map(|m| (m.mtime(), m.mtime_nsec()))
        })
        .collect()
}

type Associations = (
    Vec<Option<(i64, i64)>>,
    std::collections::HashMap<String, Option<String>>,
);

/// `xdg-mime query default` answers while no association source changed:
/// each query runs a shell script of some twenty processes, and the Apps
/// page asks for every kind each time it reads.
fn query(mime: &str) -> Option<String> {
    static CACHE: std::sync::Mutex<Option<Associations>> = std::sync::Mutex::new(None);
    let stamp = association_stamp();
    if let Ok(mut cache) = CACHE.lock() {
        let (cached, answers) =
            cache.get_or_insert_with(|| (stamp.clone(), std::collections::HashMap::new()));
        if *cached != stamp {
            *cached = stamp;
            answers.clear();
        }
        if let Some(answer) = answers.get(mime) {
            return answer.clone();
        }
        let answer = query_now(mime);
        answers.insert(mime.to_owned(), answer.clone());
        return answer;
    }
    query_now(mime)
}

fn query_now(mime: &str) -> Option<String> {
    let out = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let id = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!id.is_empty()).then_some(id)
}

const INSTALLED_TERMINAL: &str = "/usr/lib/enzui/apps/terminal-emulator";

/// The installed candidates and current choices (blocking); `terminal` is
/// the chosen program, empty for the installed one.
#[must_use]
pub fn read(terminal: &str) -> Choices {
    let all = entries();
    let mut choices = Choices::default();
    for kind in Kind::ALL {
        let mut apps: Vec<App> = all
            .iter()
            .filter(|e| {
                if kind == Kind::Terminal {
                    e.terminal && e.app.program.is_some()
                } else {
                    e.types.iter().any(|t| t == kind.types()[0])
                }
            })
            .map(|e| App {
                icon: e
                    .icon
                    .as_deref()
                    .and_then(|icon| crate::icon_theme::lookup(icon, 32)),
                ..e.app.clone()
            })
            .collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        let current = if kind == Kind::Terminal {
            // The running desktop's ENZUI_TERMINAL, which the choice sets.
            // Without either, the desktop starts the installed terminal, as
            // start-shell-platform does; D-Bus-activated Settings lacks the variable.
            let program = if terminal.is_empty() {
                std::env::var("ENZUI_TERMINAL")
                    .ok()
                    .filter(|program| !program.is_empty())
                    .unwrap_or_else(|| INSTALLED_TERMINAL.to_owned())
            } else {
                terminal.to_owned()
            };
            let base = program.rsplit('/').next().unwrap_or_default().to_owned();
            apps.iter()
                .find(|a| {
                    a.program.as_deref().is_some_and(|p| {
                        p == program || p.rsplit('/').next() == Some(base.as_str())
                    })
                })
                .map(|a| a.id.clone())
        } else {
            query(kind.types()[0])
        };
        if let Some(current) = current {
            choices.current.insert(kind, current);
        }
        choices.apps.insert(kind, apps);
    }
    choices
}

/// Example installed apps for the Settings preview; icons resolve through the
/// icon theme when present.
#[must_use]
pub fn demo() -> Choices {
    let app = |id: &str, name: &str, icon: &str, program: Option<&str>| App {
        id: format!("{id}.desktop"),
        name: name.into(),
        program: program.map(str::to_owned),
        icon: crate::icon_theme::lookup(icon, 32),
    };
    // The enzimOS apps are the defaults, beside a common alternative or two.
    let browser = app("org.enzim.Browser", "Browser", "org.enzim.Browser", None);
    let firefox = app("firefox", "Firefox", "firefox", None);
    let mail = app("io.github.enzui.Mail", "Mail", "io.github.enzui.Mail", None);
    let thunderbird = app(
        "org.mozilla.Thunderbird",
        "Thunderbird",
        "thunderbird",
        None,
    );
    let calendar = app(
        "io.github.enzui.Calendar",
        "Calendar",
        "io.github.enzui.Calendar",
        None,
    );
    let music = app(
        "io.github.enzui.Music",
        "Music",
        "io.github.enzui.Music",
        None,
    );
    let video = app(
        "io.github.enzui.Video",
        "Video",
        "io.github.enzui.Video",
        None,
    );
    let mpv = app("mpv", "mpv Media Player", "mpv", None);
    let vlc = app("vlc", "VLC media player", "vlc", None);
    let photos = app("org.enzimos.Photos", "Photos", "org.enzimos.Photos", None);
    let files = app(
        "io.github.enzui.Files",
        "Files",
        "io.github.enzui.Files",
        None,
    );
    let text = app("io.github.enzui.Text", "Text", "io.github.enzui.Text", None);
    let zed = app("dev.zed.Zed", "Zed", "zed", None);
    let terminal = app(
        "io.github.enzui.Terminal",
        "Terminal",
        "io.github.enzui.Terminal",
        Some("enzui-terminal"),
    );
    let foot = app("foot", "Foot", "foot", Some("foot"));
    let all = [
        (
            Kind::Web,
            vec![browser.clone(), firefox.clone()],
            Some(&browser),
        ),
        (
            Kind::Mail,
            vec![mail.clone(), thunderbird.clone()],
            Some(&mail),
        ),
        (
            Kind::Calendar,
            vec![calendar.clone(), thunderbird],
            Some(&calendar),
        ),
        (Kind::Music, vec![music.clone(), vlc.clone()], Some(&music)),
        (Kind::Video, vec![video.clone(), mpv, vlc], Some(&video)),
        (Kind::Photos, vec![photos.clone(), firefox], Some(&photos)),
        (Kind::Files, vec![files.clone()], Some(&files)),
        (Kind::Text, vec![text.clone(), zed], Some(&text)),
        (
            Kind::Terminal,
            vec![terminal.clone(), foot],
            Some(&terminal),
        ),
    ];
    let mut choices = Choices::default();
    for (kind, apps, current) in all {
        if let Some(current) = current {
            choices.current.insert(kind, current.id.clone());
        }
        choices.apps.insert(kind, apps);
    }
    choices
}

/// The type `enzimos-open-project` opens a project folder with; separate
/// from `inode/directory`, so ordinary folder opening never changes.
pub const PROJECT_TYPE: &str = "application/x-enzimos-project";

/// Installed development apps that open folders, and the current project
/// handler (blocking).
#[must_use]
pub fn project_handlers() -> (Option<String>, Vec<App>) {
    let mut apps: Vec<App> = entries()
        .into_iter()
        .filter(|e| e.development && e.files)
        .map(|e| e.app)
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    (query(PROJECT_TYPE), apps)
}

/// Makes `id` this account's project handler (blocking).
pub fn set_project_handler(id: &str) -> Result<(), String> {
    if !id.ends_with(".desktop")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(tr("shell-unknown-application").into());
    }
    write_defaults(&[PROJECT_TYPE], id)
        .map_err(|_| tr("shell-the-project-handler-could-not-be-changed").into())
}

/// enzUI's own association list. Its session names itself enzUI in
/// `XDG_CURRENT_DESKTOP`, so this list comes first there and nowhere else: a
/// default chosen here leaves other desktops' defaults alone.
fn own_list() -> Option<std::path::PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(config_home.join("enzui-mimeapps.list"))
}

fn write_defaults(types: &[&str], id: &str) -> std::io::Result<()> {
    let path = own_list().ok_or_else(|| std::io::Error::other("no configuration folder"))?;
    update_defaults(&path, types, id)
}

fn update_defaults(path: &std::path::Path, types: &[&str], id: &str) -> std::io::Result<()> {
    use enzui_core::atomic_state::{StateLock, save_atomic_raw};
    use std::io::ErrorKind;

    let _lock = StateLock::try_acquire(path)?.ok_or(ErrorKind::WouldBlock)?;
    let current = match std::fs::read_to_string(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        result => result?,
    };
    let updated = with_defaults(&current, types, id);
    save_atomic_raw(path, updated.as_bytes())
}

/// `text` with each of `types` defaulting to `id` in [Default Applications];
/// every other line stays as it was.
fn with_defaults(text: &str, types: &[&str], id: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = lines
        .iter()
        .position(|line| line.trim() == "[Default Applications]");
    let start = match header {
        Some(index) => index + 1,
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[Default Applications]".to_owned());
            lines.len()
        }
    };
    let end = lines[start..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + offset);
    lines.splice(
        start..end,
        lines[start..end]
            .iter()
            .filter(|line| {
                let key = line.split('=').next().unwrap_or_default().trim();
                !types.contains(&key)
            })
            .cloned()
            .chain(types.iter().map(|mime| format!("{mime}={id}")))
            .collect::<Vec<_>>(),
    );
    lines.join("\n") + "\n"
}

/// Makes `id` the default for `kind`'s types (blocking).
pub fn set(kind: Kind, id: &str) -> Result<(), String> {
    if kind == Kind::Terminal
        || !id.ends_with(".desktop")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(tr("shell-unknown-application").into());
    }
    write_defaults(kind.types(), id)
        .map_err(|_| tr("shell-the-default-could-not-be-changed").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_preserves_unreadable_defaults_and_serializes_writers() {
        use enzui_core::atomic_state::StateLock;

        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("enzui-mimeapps.list");
        let types = &["text/plain"];
        let app = "io.github.enzui.Text.desktop";

        std::fs::write(&path, [0xff]).unwrap();
        let error = update_defaults(&path, types, app).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), [0xff]);

        std::fs::remove_file(&path).unwrap();
        let lock = StateLock::acquire(&path).unwrap();
        let error = update_defaults(&path, types, app).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(!path.exists());
        drop(lock);

        update_defaults(&path, types, app).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "[Default Applications]\ntext/plain=io.github.enzui.Text.desktop\n"
        );
    }

    #[test]
    fn default_apps_behaviour() {
        // A default chosen in enzUI goes to its own list and keeps the rest of it.
        // Sabotage: keep the old lines for `types` in `with_defaults`.
        {
            let text = "[Added Associations]\ntext/plain=b.desktop;\n\n[Default Applications]\ntext/plain=old.desktop\nimage/png=viewer.desktop\n";
            let updated = with_defaults(
                text,
                &["text/plain", "text/markdown"],
                "io.github.enzui.Text.desktop",
            );
            assert_eq!(
                updated,
                "[Added Associations]\ntext/plain=b.desktop;\n\n[Default Applications]\nimage/png=viewer.desktop\ntext/plain=io.github.enzui.Text.desktop\ntext/markdown=io.github.enzui.Text.desktop\n"
            );
            assert_eq!(
                with_defaults("", &["inode/directory"], "io.github.enzui.Files.desktop"),
                "[Default Applications]\ninode/directory=io.github.enzui.Files.desktop\n"
            );
        }
        // Sabotage: stamp only the folders. Editing a list in place then keeps
        // the stamp and serves the old default.
        {
            let folder = tempfile::tempdir().unwrap();
            let list = folder.path().join("mimeapps.list");
            std::fs::write(&list, "[Default Applications]\ntext/plain=a.desktop\n").unwrap();
            let paths = vec![list.clone(), folder.path().to_path_buf()];
            let before = stamp_of(&paths);
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(&list, "[Default Applications]\ntext/plain=b.desktop\n").unwrap();
            assert_ne!(stamp_of(&paths), before);
        }
    }
}
