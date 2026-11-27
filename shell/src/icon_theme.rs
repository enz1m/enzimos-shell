//! Installed freedesktop icon-theme lookup with parent-theme fallbacks.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
};

const EXTENSIONS: [&str; 2] = ["svg", "png"];
const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 256 * 1024;

/// An installed icon theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The folder name, which is what settings store.
    pub id: String,
    /// The theme's display name.
    pub name: String,
}

#[derive(Clone, Debug)]
enum Kind {
    Fixed,
    Scalable { min: u32, max: u32 },
    Threshold(u32),
}

#[derive(Clone, Debug)]
struct Directory {
    path: String,
    size: u32,
    scale: u32,
    kind: Kind,
}

impl Directory {
    fn matches(&self, size: u32) -> bool {
        match self.kind {
            Kind::Fixed => self.size == size,
            Kind::Scalable { min, max } => (min..=max).contains(&size),
            Kind::Threshold(t) => self.size.abs_diff(size) <= t,
        }
    }
    fn distance(&self, pixels: u32) -> u32 {
        let scaled = |size: u32| size.saturating_mul(self.scale);
        match self.kind {
            Kind::Fixed => scaled(self.size).abs_diff(pixels),
            Kind::Scalable { min, max } => scaled(min)
                .saturating_sub(pixels)
                .max(pixels.saturating_sub(scaled(max))),
            Kind::Threshold(t) => scaled(self.size).abs_diff(pixels).saturating_sub(scaled(t)),
        }
    }
}

#[derive(Clone, Debug)]
struct Theme {
    roots: Vec<PathBuf>,
    directories: Vec<Directory>,
    inherits: Vec<String>,
}

/// Where icon themes live, in lookup order.
pub fn base_directories() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    if let Some(data) = data_home {
        bases.push(data.join("icons"));
    }
    if let Some(home) = &home {
        bases.push(home.join(".icons"));
    }
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in dirs
        .split(':')
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        let path = dir.join("icons");
        if !bases.contains(&path) {
            bases.push(path);
        }
    }
    // Flatpak exports may be added after this process started.
    for data in [
        home.map(|p| p.join(".local/share/flatpak/exports/share")),
        Some(PathBuf::from("/var/lib/flatpak/exports/share")),
    ]
    .into_iter()
    .flatten()
    {
        let icons = data.join("icons");
        if !bases.contains(&icons) {
            bases.push(icons);
        }
    }
    bases
}

fn read_small(path: &Path, limit: u64) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

// `[section] key=value` pairs of an index.theme, keys unlocalised.
fn parse_index(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_owned(), HashMap::new()));
        } else if let (Some((key, value)), Some((_, fields))) =
            (line.split_once('='), sections.last_mut())
            && !line.starts_with('#')
            && !key.contains('[')
        {
            fields
                .entry(key.trim().to_owned())
                .or_insert_with(|| value.trim().to_owned());
        }
    }
    sections
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.starts_with('.') && !id.contains(['/', '\\', '\0'])
}

fn load_theme(id: &str, bases: &[PathBuf]) -> Option<Theme> {
    if !valid_id(id) {
        return None;
    }
    let roots: Vec<PathBuf> = bases
        .iter()
        .map(|base| base.join(id))
        .filter(|dir| dir.is_dir())
        .collect();
    let index = roots
        .iter()
        .find_map(|root| read_small(&root.join("index.theme"), MAX_INDEX_BYTES))?;
    let sections = parse_index(&index);
    let header = &sections.iter().find(|(name, _)| name == "Icon Theme")?.1;
    let list = |key: &str| -> Vec<String> {
        header
            .get(key)
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut names = list("Directories");
    names.extend(list("ScaledDirectories"));
    let directories = names
        .into_iter()
        .filter_map(|path| {
            let fields = &sections.iter().find(|(name, _)| *name == path)?.1;
            let number = |key: &str| fields.get(key).and_then(|v| v.parse::<u32>().ok());
            let size = number("Size")?;
            let kind = match fields.get("Type").map(String::as_str) {
                Some("Fixed") => Kind::Fixed,
                Some("Scalable") => Kind::Scalable {
                    min: number("MinSize").unwrap_or(size),
                    max: number("MaxSize").unwrap_or(size),
                },
                _ => Kind::Threshold(number("Threshold").unwrap_or(2)),
            };
            Some(Directory {
                path,
                size,
                scale: number("Scale").unwrap_or(1).max(1),
                kind,
            })
        })
        .collect();
    Some(Theme {
        roots,
        directories,
        inherits: list("Inherits"),
    })
}

/// Icon themes installed in the shared folders, by display name.
pub fn installed() -> Vec<Installed> {
    let mut themes: Vec<Installed> = Vec::new();
    for base in base_directories() {
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok).take(1024) {
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_id(&id) || themes.iter().any(|theme| theme.id == id) {
                continue;
            }
            let Some(text) = read_small(&entry.path().join("index.theme"), MAX_INDEX_BYTES) else {
                continue;
            };
            let sections = parse_index(&text);
            let Some((_, header)) = sections.iter().find(|(name, _)| name == "Icon Theme") else {
                continue;
            };
            // Adwaita is Hidden but selectable; cursor-only themes and fallback IDs are not
            // icon choices.
            if matches!(id.as_str(), "hicolor" | "default")
                || header
                    .get("Directories")
                    .is_none_or(|v| v.trim().is_empty())
            {
                continue;
            }
            let name = header.get("Name").cloned().unwrap_or_else(|| id.clone());
            themes.push(Installed { id, name });
        }
    }
    themes.sort_by_key(|theme| theme.name.to_lowercase());
    themes
}

// The theme the desktop itself names (GTK, then KDE), if any.
fn desktop_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    for (file, section, key) in [
        ("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        ("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        ("kdeglobals", "Icons", "Theme"),
    ] {
        let Some(text) = read_small(&config.join(file), MAX_INDEX_BYTES) else {
            continue;
        };
        if let Some(value) = parse_index(&text)
            .into_iter()
            .find(|(name, _)| name == section)
            .and_then(|(_, fields)| fields.get(key).cloned())
            .map(|value| value.trim_matches('"').to_owned())
            .filter(|value| valid_id(value))
        {
            return Some(value);
        }
    }
    None
}

/// The theme used when the user has not picked one: the desktop's, else the
/// first common default that is installed, else hicolor.
pub fn default_theme() -> String {
    pick_default(desktop_theme(), &installed())
}

// enzimOS ships its own "enzim" theme; the others are common distribution defaults.
const FALLBACK_THEMES: [&str; 4] = ["enzim", "Adwaita", "breeze", "Papirus"];

fn pick_default(desktop: Option<String>, installed: &[Installed]) -> String {
    let present = |id: &str| installed.iter().any(|theme| theme.id == id);
    desktop
        .filter(|id| present(id))
        .or_else(|| {
            FALLBACK_THEMES
                .into_iter()
                .find(|id| present(id))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "hicolor".into())
}

struct Resolver {
    theme: String,
    bases: Vec<PathBuf>,
    themes: HashMap<String, Option<Theme>>,
    found: HashMap<(String, u32, u32), Option<PathBuf>>,
}

fn selected() -> &'static RwLock<Option<String>> {
    static SELECTED: OnceLock<RwLock<Option<String>>> = OnceLock::new();
    SELECTED.get_or_init(|| RwLock::new(None))
}

fn resolver() -> &'static Mutex<Option<Resolver>> {
    static RESOLVER: OnceLock<Mutex<Option<Resolver>>> = OnceLock::new();
    RESOLVER.get_or_init(|| Mutex::new(None))
}

/// Choose the theme for later lookups (`None`: follow the desktop). Returns
/// whether the choice changed, so callers know to refresh resolved icons.
pub fn select(theme: Option<&str>) -> bool {
    let theme = theme.filter(|id| valid_id(id)).map(str::to_owned);
    let mut current = selected()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *current == theme {
        return false;
    }
    *current = theme;
    *resolver()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    true
}

/// Refresh directory metadata and misses after applications are installed or removed.
pub fn refresh() {
    *resolver()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Changes whenever the selected theme does, for callers keeping their own
/// caches of resolved icons.
pub fn generation() -> u64 {
    GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

fn usable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_ICON_BYTES)
}

impl Resolver {
    fn theme(&mut self, id: &str) -> Option<Theme> {
        if !self.themes.contains_key(id) {
            let theme = load_theme(id, &self.bases);
            self.themes.insert(id.to_owned(), theme);
        }
        self.themes.get(id).cloned().flatten()
    }

    fn chain(&mut self) -> Vec<Theme> {
        let mut order = vec![self.theme.clone()];
        let mut chain = Vec::new();
        let mut index = 0;
        while index < order.len() && index < 16 {
            if let Some(theme) = self.theme(&order[index].clone()) {
                for parent in &theme.inherits {
                    if !order.contains(parent) {
                        order.push(parent.clone());
                    }
                }
                chain.push(theme);
            }
            index += 1;
        }
        if !order.iter().any(|id| id == "hicolor")
            && let Some(hicolor) = self.theme("hicolor")
        {
            chain.push(hicolor);
        }
        chain
    }

    fn in_theme(theme: &Theme, name: &str, size: u32, pixels: u32) -> Option<PathBuf> {
        let file = |dir: &Directory| {
            theme.roots.iter().find_map(|root| {
                EXTENSIONS
                    .iter()
                    .map(|ext| root.join(&dir.path).join(format!("{name}.{ext}")))
                    .find(|path| usable(path))
            })
        };
        if let Some(path) = theme
            .directories
            .iter()
            .filter(|dir| {
                dir.size > 0 && size.saturating_mul(dir.scale) == pixels && dir.matches(size)
            })
            .find_map(file)
        {
            return Some(path);
        }
        let mut candidates: Vec<_> = theme.directories.iter().collect();
        candidates.sort_by_key(|dir| {
            (
                dir.distance(pixels),
                std::cmp::Reverse(dir.size.saturating_mul(dir.scale)),
            )
        });
        candidates.into_iter().find_map(file)
    }

    fn lookup(&mut self, name: &str, size: u32, pixels: u32) -> Option<PathBuf> {
        let key = (name.to_owned(), size, pixels);
        if let Some(found) = self.found.get(&key) {
            return found.clone();
        }
        let path = self
            .chain()
            .iter()
            .find_map(|theme| Self::in_theme(theme, name, size, pixels))
            .or_else(|| {
                let pixmaps = Path::new("/usr/share/pixmaps");
                EXTENSIONS
                    .iter()
                    .map(|ext| pixmaps.join(format!("{name}.{ext}")))
                    .find(|p| usable(p))
            });
        self.found.insert(key, path.clone());
        path
    }
}

/// Resolves SVG/PNG icons through the selected theme, parents, hicolor and pixmaps; accepts
/// absolute paths.
pub fn lookup(icon: &str, size: u32) -> Option<PathBuf> {
    lookup_for_output(icon, size, size)
}

/// Resolves a theme icon for a logical size and the output's physical raster size.
pub fn lookup_for_output(icon: &str, size: u32, pixels: u32) -> Option<PathBuf> {
    let icon = icon.trim();
    if icon.is_empty() || icon.len() > 512 || icon.contains('\0') {
        return None;
    }
    let path = Path::new(icon);
    if path.is_absolute() {
        let supported = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
        return (supported && usable(path)).then(|| path.to_owned());
    }
    if icon.contains('/') {
        return None;
    }
    // Some entries name a file ("firefox.png") instead of an icon.
    let name = EXTENSIONS
        .iter()
        .chain(["xpm"].iter())
        .find_map(|ext| icon.strip_suffix(&format!(".{ext}")))
        .unwrap_or(icon);
    let selected = selected()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut guard = resolver()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let resolver = guard.get_or_insert_with(|| Resolver {
        theme: selected.clone().unwrap_or_else(default_theme),
        bases: base_directories(),
        themes: HashMap::new(),
        found: HashMap::new(),
    });
    resolver.lookup(name, size, pixels)
}

/// The drawable source for a resolved icon file.
pub fn source(path: &Path) -> enzui_widgets::icon::IconSource {
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
    {
        iced::widget::svg::Handle::from_path(path).into()
    } else {
        iced::widget::image::Handle::from_path(path).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_lookup_keeps_distinct_results_for_mixed_output_scales() {
        let base = tempfile::tempdir().expect("icon theme fixture");
        let root = base.path().join("density");
        let index = "[Icon Theme]\nName=Density\nDirectories=24/apps\nScaledDirectories=18@2/apps,24@2/apps\n\n[24/apps]\nSize=24\nType=Fixed\n\n[18@2/apps]\nSize=18\nScale=2\nType=Fixed\n\n[24@2/apps]\nSize=24\nScale=2\nType=Fixed\n";
        for directory in ["24/apps", "18@2/apps", "24@2/apps"] {
            let path = root.join(directory);
            std::fs::create_dir_all(&path).expect("theme directory");
            std::fs::write(path.join("terminal.png"), b"raster").expect("raster icon");
        }
        std::fs::write(root.join("index.theme"), index).expect("theme index");
        let mut resolver = Resolver {
            theme: "density".to_owned(),
            bases: vec![base.path().to_owned()],
            themes: HashMap::new(),
            found: HashMap::new(),
        };
        for (pixels, directory) in [
            (24, "24/apps"),
            (36, "18@2/apps"),
            (48, "24@2/apps"),
            (24, "24/apps"),
        ] {
            assert_eq!(
                resolver.lookup("terminal", 24, pixels),
                Some(root.join(directory).join("terminal.png")),
            );
        }
    }
}
