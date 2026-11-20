use super::{AppTarget, Category, Item, Target};
use enzui_shell::i18n::tr;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified: (i64, i64),
}
impl FileIdentity {
    pub fn capture(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|_| tr("shell-item-is-no-longer-available"))?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(tr("shell-unsupported-item-type").into());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
        })
    }
    pub fn verify(&self, path: &Path) -> Result<(), String> {
        if &Self::capture(path)? == self {
            Ok(())
        } else {
            Err(tr("shell-item-changed-refresh-and-choose-it-again").into())
        }
    }
}
pub fn display_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(limit)
        .collect()
}
// Apply OnlyShowIn/NotShowIn against the session's colon-separated XDG_CURRENT_DESKTOP.
pub(crate) fn shown_in(only: Option<&str>, not: Option<&str>, current: &str) -> bool {
    let current: Vec<&str> = current.split(':').filter(|name| !name.is_empty()).collect();
    let listed = |list: &str| {
        list.split(';').any(|name| {
            current
                .iter()
                .any(|desktop| desktop.eq_ignore_ascii_case(name.trim()))
        })
    };
    only.is_none_or(listed) && !not.is_some_and(listed)
}
// The entry's item, its program, and whether it is a Flatpak export.
enum EntryPurpose {
    Launcher,
    Identity,
}

fn desktop_entry(
    path: &Path,
    id: &str,
    purpose: EntryPurpose,
) -> Option<(Item, Option<PathBuf>, bool)> {
    let identity = FileIdentity::capture(path).ok()?;
    if identity.size > 65536 {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(65537)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 65536 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut fields = std::collections::BTreeMap::new();
    let mut active = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if !active || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.entry(key.trim()).or_insert_with(|| value.trim());
        }
    }
    if fields.get("Type") != Some(&"Application")
        || fields.get("Hidden") == Some(&"true")
        || (matches!(purpose, EntryPurpose::Launcher) && fields.get("NoDisplay") == Some(&"true"))
        || (!fields.contains_key("Exec") && fields.get("DBusActivatable") != Some(&"true"))
        || !shown_in(
            fields.get("OnlyShowIn").copied(),
            fields.get("NotShowIn").copied(),
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "enzUI".into()),
        )
    {
        return None;
    }
    if let Some(value) = fields.get("TryExec") {
        let available = if Path::new(value).is_absolute() {
            Path::new(value).is_file()
        } else {
            ["/usr/local/bin", "/usr/bin", "/bin"]
                .iter()
                .any(|directory| Path::new(directory).join(value).is_file())
        };
        if !available {
            return None;
        }
    }
    // Parental controls: an app this account may not start is not offered.
    if !crate::parental::entry_allowed(path) {
        return None;
    }
    let title = display_text(crate::i18n::desktop_text(&fields, "Name")?, 128);
    if title.is_empty() {
        return None;
    }
    let comment = display_text(
        crate::i18n::desktop_text(&fields, "Comment")
            .unwrap_or_else(|| tr("shell-installed-application")),
        256,
    );
    let image = fields
        .get("Icon")
        .and_then(|icon| crate::icon_theme::lookup(icon, 48));
    let program = fields
        .get("Exec")
        .and_then(|exec| exec.split_whitespace().next())
        .map(PathBuf::from)
        .filter(|program| program.is_absolute());
    Some((
        Item::new(
            format!("app:desktop/{id}"),
            Category::Apps,
            title,
            comment,
            tr("shell-open"),
            "app-window",
            Some(Target::App(AppTarget::Desktop {
                path: path.to_owned(),
                identity,
            })),
        )
        .with_image(image),
        program,
        fields.contains_key("X-Flatpak"),
    ))
}
pub fn scan(root: &Path, directories: &[PathBuf]) -> Vec<Item> {
    scan_cancellable(
        root,
        directories,
        &std::sync::atomic::AtomicBool::new(false),
    )
}
pub fn scan_cancellable(
    root: &Path,
    directories: &[PathBuf],
    cancel: &std::sync::atomic::AtomicBool,
) -> Vec<Item> {
    crate::icon_theme::refresh();
    let mut result = Vec::new();
    let mut native = Vec::new();
    // Desktop ids of the catalog apps installed natively.
    let mut twins = BTreeSet::new();
    // Catalog apps whose prepared binary is missing: (result index, desktop id).
    let mut absent = Vec::new();
    for app in crate::catalog::APPS
        .iter()
        .filter(|app| crate::catalog::offered(app))
    {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return result;
        }
        let executable = crate::catalog::executable(root, app.id);
        if !crate::parental::program_allowed(&executable) {
            continue;
        }
        let installed = executable.is_file();
        if let Some(metadata) = fs::metadata(&executable).ok().filter(|_| installed) {
            native.push((result.len(), (metadata.dev(), metadata.ino()), app.icon));
            twins.extend(crate::catalog::desktop_id(app.id).map(|id| format!("{id}.desktop")));
        } else if let Some(id) = crate::catalog::desktop_id(app.id) {
            absent.push((result.len(), format!("{id}.desktop")));
        }
        result.push(
            Item::new(
                format!("app:{}", app.id),
                Category::Apps,
                app.display_name(),
                if installed {
                    app.display_description()
                } else {
                    tr("shell-not-installed-in-this-desktop")
                },
                tr("shell-open"),
                app.icon,
                installed.then(|| Target::App(AppTarget::Prepared(app.id.into()))),
            )
            .with_image(app.themed_icon(48)),
        );
    }
    let mut seen = BTreeSet::new();
    let mut flatpaks = Vec::new();
    let mut inspected = 0;
    for directory in directories.iter().take(8) {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .take(4096)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return result;
            }
            inspected += 1;
            if inspected > 8192 || result.len() >= 2048 {
                return result;
            }
            if path
                .extension()
                .is_none_or(|extension| extension != "desktop")
            {
                continue;
            }
            let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|id| id.len() <= 220)
            else {
                continue;
            };
            let twin = twins.contains(id);
            if !twin && !seen.insert(id.to_owned()) {
                continue;
            }
            let Some((mut item, program, flatpak)) =
                desktop_entry(&path, id, EntryPurpose::Launcher)
            else {
                continue;
            };
            // A catalog app installed natively and as a Flatpak, in either
            // folder order: the native one stays the catalog app and the
            // Flatpak is listed as one.
            if twin {
                if !seen.insert(if flatpak {
                    format!("flatpak:{id}")
                } else {
                    id.to_owned()
                }) {
                    continue;
                }
                if flatpak {
                    tr("shell-flatpak-version").clone_into(&mut item.subtitle);
                }
            }
            // Match native apps by executable identity so installed metadata preserves their
            // native launch route.
            let file = program
                .and_then(|program| fs::metadata(program).ok())
                .map(|m| (m.dev(), m.ino()));
            if let Some(position) = native
                .iter()
                .position(|(_, identity, _)| Some(*identity) == file)
            {
                let (index, _, icon) = native.remove(position);
                let builtin = &result[index];
                result[index] = Item::new(
                    builtin.id.clone(),
                    Category::Apps,
                    item.title,
                    item.subtitle,
                    tr("shell-open"),
                    icon,
                    builtin.target.clone(),
                )
                .with_image(item.image);
                continue;
            }
            if flatpak {
                flatpaks.push(result.len());
            }
            result.push(item);
        }
    }
    // App rows show only a name, so a Flatpak installed beside the same app
    // from the system says so in its name: two rows reading "Firefox" cannot
    // be told apart.
    for index in flatpaks {
        if result.iter().enumerate().any(|(other, item)| {
            other != index && item.target.is_some() && item.title == result[index].title
        }) {
            let title = format!("{} ({})", result[index].title, tr("shell-flatpak-version"));
            result[index] = result[index].clone().retitled(title);
        }
    }
    without_absent_twins(result, &absent, &seen)
}
// A catalog app that is installed as an ordinary desktop app is listed once,
// as that app, rather than also as "Not installed in this desktop".
fn without_absent_twins(
    mut result: Vec<Item>,
    absent: &[(usize, String)],
    seen: &BTreeSet<String>,
) -> Vec<Item> {
    let mut drop: Vec<usize> = absent
        .iter()
        .filter(|(_, id)| seen.contains(id))
        .map(|(index, _)| *index)
        .collect();
    drop.sort_unstable();
    for index in drop.into_iter().rev() {
        result.remove(index);
    }
    result
}
/// Resolves desktop metadata for an executable, caching per file for the process lifetime.
pub fn app_for_executable(executable: &Path) -> Option<(String, Option<PathBuf>, PathBuf)> {
    use std::sync::{Mutex, OnceLock};
    type Found = Option<(String, Option<PathBuf>, PathBuf)>;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(u64, u64), Found>>> = OnceLock::new();
    let meta = fs::metadata(executable).ok()?;
    let key = (meta.dev(), meta.ino());
    let cache = CACHE.get_or_init(Default::default);
    if let Some(found) = cache.lock().ok()?.get(&key) {
        return found.clone();
    }
    let found = find_app_for_executable(executable, &directories());
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, found.clone());
    }
    found
}

fn find_app_for_executable(
    executable: &Path,
    directories: &[PathBuf],
) -> Option<(String, Option<PathBuf>, PathBuf)> {
    let meta = fs::metadata(executable).ok()?;
    let key = (meta.dev(), meta.ino());
    let name = executable.file_name()?.to_owned();
    let resolve = |program: &str| -> Option<PathBuf> {
        let path = Path::new(program);
        if path.is_absolute() {
            return Some(path.to_owned());
        }
        ["/usr/local/bin", "/usr/bin", "/bin"]
            .iter()
            .map(|dir| Path::new(dir).join(program))
            .find(|p| p.is_file())
    };
    let mut by_name = None;
    let mut found = None;
    'scan: for directory in directories.iter().take(8) {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for path in entries.filter_map(Result::ok).take(4096).map(|e| e.path()) {
            if path.extension().is_none_or(|ext| ext != "desktop") {
                continue;
            }
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|id| id.len() <= 220)
            else {
                continue;
            };
            let Some((item, ..)) = desktop_entry(&path, id, EntryPurpose::Identity) else {
                continue;
            };
            let Some(program) = exec_program(&path).and_then(|p| resolve(&p)) else {
                continue;
            };
            let same = fs::metadata(&program).is_ok_and(|m| (m.dev(), m.ino()) == key);
            if same {
                found = Some((item.title, item.image, path));
                break 'scan;
            }
            if by_name.is_none() && program.file_name() == Some(name.as_os_str()) {
                by_name = Some((item.title, item.image, path));
            }
        }
    }
    found.or(by_name)
}

// The program an entry runs: the first word of `Exec`, past an `env` prefix.
fn exec_program(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let exec = text
        .lines()
        .skip_while(|line| line.trim() != "[Desktop Entry]")
        .find_map(|line| line.trim().strip_prefix("Exec="))?;
    let mut words = exec.split_whitespace();
    let mut program = words.next()?;
    if program == "env" || program.ends_with("/env") {
        program = words.find(|word| !word.contains('='))?;
    }
    Some(program.trim_matches('"').to_owned())
}

/// Resolves a running app ID through its desktop entry, then the icon theme.
pub fn icon_for_app_id(id: &str) -> Option<PathBuf> {
    let id = id.trim();
    if id.is_empty() || id.len() > 200 || id.contains(['/', '\0']) || id.starts_with('.') {
        return None;
    }
    let lower = id.to_lowercase();
    let candidates = [id, lower.as_str()];
    directories()
        .iter()
        .flat_map(|dir| {
            candidates
                .iter()
                .map(move |name| dir.join(format!("{name}.desktop")))
        })
        .filter(|path| path.is_file())
        .find_map(|path| {
            let file = path.file_name()?.to_str()?.to_owned();
            desktop_entry(&path, &file, EntryPurpose::Identity)?.0.image
        })
        .or_else(|| {
            candidates
                .iter()
                .find_map(|name| crate::icon_theme::lookup(name, 48))
        })
}
/// A running app ID's display name from its desktop entry, when one exists.
pub fn name_for_app_id(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() || id.len() > 200 || id.contains(['/', '\0']) || id.starts_with('.') {
        return None;
    }
    let lower = id.to_lowercase();
    let candidates = [id, lower.as_str()];
    directories()
        .iter()
        .flat_map(|dir| {
            candidates
                .iter()
                .map(move |name| dir.join(format!("{name}.desktop")))
        })
        .filter(|path| path.is_file())
        .find_map(|path| {
            let file = path.file_name()?.to_str()?.to_owned();
            Some(desktop_entry(&path, &file, EntryPurpose::Identity)?.0.title)
        })
        .filter(|name| !name.trim().is_empty())
}
/// A running app's name and icon for shell views that redraw every frame:
/// resolved from its desktop entry once per app ID, then cached. Refreshed
/// with [`crate::icon_theme::generation`] so a theme change re-resolves.
pub fn app_identity(id: &str) -> (Option<String>, Option<PathBuf>) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Cache = (u64, HashMap<String, (Option<String>, Option<PathBuf>)>);
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let generation = crate::icon_theme::generation();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new((generation, HashMap::new())))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.0 != generation {
        *cache = (generation, HashMap::new());
    }
    if cache.1.len() > 256 {
        cache.1.clear();
    }
    cache
        .1
        .entry(id.to_owned())
        .or_insert_with(|| (name_for_app_id(id), icon_for_app_id(id)))
        .clone()
}
/// Returns the validated desktop-entry bus name for DBusActivatable entries.
pub fn dbus_activatable(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.strip_suffix(".desktop")?;
    let valid = name.len() <= 255
        && name.split('.').count() >= 2
        && name.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with(|c: char| c.is_ascii_digit())
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        });
    if !valid || FileIdentity::capture(path).ok()?.size > 65536 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let mut active = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
        } else if active
            && line
                .split_once('=')
                .is_some_and(|(k, v)| k.trim() == "DBusActivatable" && v.trim() == "true")
        {
            return Some(name.to_owned());
        }
    }
    None
}
/// Application folders in XDG precedence: the data home, then each
/// `XDG_DATA_DIRS` entry (Flatpak's exports among them).
pub fn directories() -> Vec<PathBuf> {
    directories_from(
        std::env::var_os("HOME"),
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("XDG_DATA_DIRS"),
    )
}

fn directories_from(
    home: Option<std::ffi::OsString>,
    data_home: Option<std::ffi::OsString>,
    data_dirs: Option<std::ffi::OsString>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    match data_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(path) => roots.push(path),
        None => roots.extend(home.map(|home| PathBuf::from(home).join(".local/share"))),
    }
    let user_data = roots.first().cloned();
    let dirs: Vec<PathBuf> = data_dirs
        .map(|dirs| {
            std::env::split_paths(&dirs)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .filter(|dirs: &Vec<PathBuf>| !dirs.is_empty())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    roots.extend(dirs.into_iter().take(32));
    // A first user install can create these after the login environment was built.
    if let Some(data) = user_data {
        roots.push(data.join("flatpak/exports/share"));
    }
    roots.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    let mut paths = Vec::new();
    for root in roots {
        let path = root.join("applications");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// The installed desktop entry for `id` (a desktop id without `.desktop`),
/// found in the same order as the catalogue reads entries.
pub fn installed_entry(id: &str) -> Option<PathBuf> {
    directories()
        .into_iter()
        .map(|directory| directory.join(format!("{id}.desktop")))
        .find(|path| path.is_file())
}

// Desktop IDs, Flatpak IDs and explicit WM classes identify apps; titles do not.
pub(crate) fn application_ids(path: &Path) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
        result.push(stem.to_owned());
    }
    let Ok(file) = fs::File::open(path) else {
        return result;
    };
    let mut text = String::new();
    if file.take(65537).read_to_string(&mut text).is_err() || text.len() > 65536 {
        return result;
    }
    let mut active = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if active
            && let Some((key, value)) = line.split_once('=')
            && matches!(key, "StartupWMClass" | "X-Flatpak")
            && !value.is_empty()
            && value.len() <= 256
        {
            result.push(value.to_owned());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_behaviour() {
        // Hidden user entries mask system entries and actions are not commands.
        {
            let root = tempfile::tempdir().unwrap();
            let user = root.path().join("user");
            let system = root.path().join("system");
            fs::create_dir_all(&user).unwrap();
            fs::create_dir_all(&system).unwrap();
            fs::write(
                user.join("hidden.desktop"),
                "[Desktop Entry]\nType=Application\nHidden=true\nName=Private\nExec=false\n",
            )
            .unwrap();
            fs::write(
                system.join("hidden.desktop"),
                "[Desktop Entry]\nType=Application\nName=System\nExec=false\n",
            )
            .unwrap();
            fs::write(system.join("visible.desktop"), "[Desktop Entry]\nType=Application\nName=Visible\nComment=Description\nExec=sh -c 'not executed'\n[Desktop Action Other]\nName=Other\n").unwrap();
            let background = root.path().join("background-host");
            fs::write(&background, "fixture executable identity").unwrap();
            let background_entry = user.join("background.desktop");
            fs::write(&background_entry, format!("[Desktop Entry]\nType=Application\nNoDisplay=true\nName=Background service\nExec={} settings\n", background.display())).unwrap();
            let items = scan(root.path(), &[user.clone(), system.clone()]);
            let external: Vec<_> = items
                .iter()
                .filter(|i| i.id.starts_with("app:desktop/"))
                .collect();
            assert_eq!(external.len(), 1);
            assert_eq!(external[0].title, "Visible");
            let identity = find_app_for_executable(&background, &[user, system.clone()]).unwrap();
            assert_eq!(identity.0, "Background service");
            assert_eq!(identity.2, background_entry);
            let home = root.path().join("new-account");
            let directories = directories_from(
                Some(home.clone().into_os_string()),
                None,
                Some(system.into_os_string()),
            );
            let exported = home.join(".local/share/flatpak/exports/share/applications");
            assert!(
                !scan(root.path(), &directories)
                    .iter()
                    .any(|item| item.title == "New Flatpak")
            );
            fs::create_dir_all(&exported).unwrap();
            fs::write(exported.join("org.example.New.desktop"), "[Desktop Entry]\nType=Application\nName=New Flatpak\nExec=flatpak run org.example.New\n").unwrap();
            assert!(
                scan(root.path(), &directories)
                    .iter()
                    .any(|item| item.title == "New Flatpak")
            );

            assert!(matches!(
                external[0].target,
                Some(Target::App(AppTarget::Desktop { .. }))
            ));
        }
        // A catalog app installed natively and as a Flatpak shows as the
        // catalog app plus one Flatpak row labelled as such, whichever folder
        // comes first; never two identical rows.
        //
        // Sabotage: drop the `twin` key change (the second entry is then
        // skipped in one order), or keep the Flatpak entry's own comment.
        {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join(".build/apps/bin");
            fs::create_dir_all(&bin).unwrap();
            fs::write(bin.join("files-app"), "").unwrap();
            let flatpak = root.path().join("flatpak");
            let native = root.path().join("native");
            fs::create_dir_all(&flatpak).unwrap();
            fs::create_dir_all(&native).unwrap();
            fs::write(
                flatpak.join("io.github.enzui.Files.desktop"),
                "[Desktop Entry]\nType=Application\nName=Files\nComment=Folders and documents\nExec=flatpak run io.github.enzui.Files\nX-Flatpak=io.github.enzui.Files\n",
            )
            .unwrap();
            fs::write(
                native.join("io.github.enzui.Files.desktop"),
                format!(
                    "[Desktop Entry]\nType=Application\nName=Files\nComment=Folders and documents\nExec={}\n",
                    bin.join("files-app").display()
                ),
            )
            .unwrap();
            for order in [[&flatpak, &native], [&native, &flatpak]] {
                let items = scan(root.path(), &[order[0].clone(), order[1].clone()]);
                let files: Vec<_> = items
                    .iter()
                    .filter(|item| item.title.starts_with("Files"))
                    .collect();
                assert_eq!(files.len(), 2, "{order:?}");
                assert!(
                    files
                        .iter()
                        .any(|item| item.id == "app:files-app" && item.title == "Files")
                );
                // App rows show no second line, so the name carries the label.
                assert!(
                    files.iter().any(|item| item.id.starts_with("app:desktop/")
                        && item.title == "Files (Flatpak)"
                        && item.subtitle == tr("shell-flatpak-version")),
                    "{order:?}"
                );
            }
        }
        // A catalog app installed only as a desktop entry is listed once.
        {
            let root = tempfile::tempdir().unwrap();
            let apps = root.path().join("apps");
            fs::create_dir_all(&apps).unwrap();
            fs::write(
                apps.join("io.github.enzui.Text.desktop"),
                "[Desktop Entry]\nType=Application\nName=Text\nComment=Edit plain text\nExec=/usr/bin/true\n",
            )
            .unwrap();
            let items = scan(root.path(), &[apps]);
            let text: Vec<_> = items.iter().filter(|item| item.title == "Text").collect();
            assert_eq!(text.len(), 1);
            assert_ne!(text[0].subtitle, tr("shell-not-installed-in-this-desktop"));
            // Catalog apps with neither a binary nor an entry still say so.
            assert!(items.iter().any(|item| item.id == "app:files-app"
                && item.subtitle == tr("shell-not-installed-in-this-desktop")));
        }
        // File identity rejects replacement and spoofing controls are not displayed.
        {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("file");
            fs::write(&path, "one").unwrap();
            let identity = FileIdentity::capture(&path).unwrap();
            fs::write(&path, "changed content").unwrap();
            assert!(identity.verify(&path).is_err());
            assert_eq!(display_text("safe\u{202e}\nname", 100), "safename");
        }
    }
}
