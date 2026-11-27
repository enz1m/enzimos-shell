use enzui_shell::i18n::tr;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Apps,
    Windows,
    Actions,
    Calculator,
    Files,
    /// Results from inside apps, through their search providers.
    InApps,
    Clipboard,
    Web,
}
impl Category {
    pub const ALL: [Self; 8] = [
        Self::Apps,
        Self::Windows,
        Self::Actions,
        Self::Calculator,
        Self::Files,
        Self::InApps,
        Self::Clipboard,
        Self::Web,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Apps => "apps",
            Self::Windows => "windows",
            Self::Actions => "actions",
            Self::Calculator => "calculator",
            Self::Files => "files",
            Self::InApps => "in_apps",
            Self::Clipboard => "clipboard",
            Self::Web => "web",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => tr("shell-apps"),
            Self::Windows => tr("shell-windows"),
            Self::Actions => tr("shell-actions"),
            Self::Calculator => tr("shell-calculator"),
            Self::Files => tr("shell-files"),
            Self::InApps => tr("shell-in-apps"),
            Self::Clipboard => tr("shell-clipboard"),
            Self::Web => tr("shell-web"),
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.key() == value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub animations: bool,
    pub background_fade: bool,
    pub compact: bool,
    pub rows: u8,
    pub categories: Vec<Category>,
    pub roots: Vec<String>,
    pub exclusions: Vec<String>,
    pub recent: bool,
    pub usage: BTreeMap<String, u32>,
    pub favorites: Vec<String>,
    pub aliases: BTreeMap<String, String>,
    pub clipboard_minutes: u16,
    pub clipboard_generation: u64,
    pub web_provider: WebProvider,
    /// Search providers (by desktop entry ID) the person switched on or off;
    /// the rest follow [`Preferences::provider_on`]'s default.
    pub provider_choices: BTreeMap<String, bool>,
    /// Tracks offered built-in sources so newly introduced sources default on only once.
    #[serde(default)]
    pub sources: u8,
}
const SOURCES: u8 = 1;
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            animations: true,
            background_fade: true,
            compact: false,
            rows: 8,
            categories: vec![
                Category::Apps,
                Category::Windows,
                Category::Actions,
                Category::Calculator,
                Category::InApps,
                Category::Web,
            ],
            roots: Vec::new(),
            exclusions: Vec::new(),
            recent: true,
            usage: BTreeMap::new(),
            favorites: Vec::new(),
            aliases: BTreeMap::new(),
            clipboard_minutes: 30,
            clipboard_generation: 0,
            web_provider: WebProvider::default(),
            provider_choices: BTreeMap::new(),
            sources: SOURCES,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebProvider {
    #[default]
    DuckDuckGo,
    Google,
    Wikipedia,
}
impl WebProvider {
    pub const ALL: [Self; 3] = [Self::DuckDuckGo, Self::Google, Self::Wikipedia];
    pub fn label(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Google => tr("shell-google"),
            Self::Wikipedia => tr("shell-wikipedia"),
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "https://duckduckgo.com/?q=",
            Self::Google => "https://www.google.com/search?q=",
            Self::Wikipedia => "https://en.wikipedia.org/w/index.php?search=",
        }
    }
}
#[derive(Clone, Debug)]
pub enum Change {
    Enabled(bool),
    Animations(bool),
    BackgroundFade(bool),
    Compact(bool),
    Rows(u8),
    Category(Category, bool),
    Roots(Vec<String>),
    Exclusions(Vec<String>),
    Root(String, bool),
    ExcludedRoot(String, bool),
    Recent(bool),
    ClearUsage,
    RecordUse(String),
    Favorite(String, bool),
    Alias(String, Option<String>),
    ClipboardMinutes(u16),
    ClearClipboard,
    WebProvider(WebProvider),
    /// A search provider (desktop entry ID) on or off.
    Provider(String, bool),
    Reset,
}
impl Change {
    pub fn same_key(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Category(a, _), Self::Category(b, _)) => a == b,
            (Self::Favorite(a, _), Self::Favorite(b, _))
            | (Self::Alias(a, _), Self::Alias(b, _))
            | (Self::Root(a, _), Self::Root(b, _))
            | (Self::ExcludedRoot(a, _), Self::ExcludedRoot(b, _))
            | (Self::Provider(a, _), Self::Provider(b, _)) => a == b,
            (Self::RecordUse(_), Self::RecordUse(_)) => false,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}
fn stable_key(value: &str) -> bool {
    value.len() <= 256
        && ["app:", "setting:", "action:"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
        && !value.chars().any(char::is_control)
}
fn alias_key(value: &str) -> bool {
    (2..=16).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}
fn provider_key(value: &str) -> bool {
    value.len() <= 255
        && value.strip_suffix(".desktop").is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
}
fn paths(values: &[String]) -> bool {
    values.len() <= 8
        && values.iter().all(|value| {
            value.len() <= 2048
                && Path::new(value).is_absolute()
                && !value.chars().any(char::is_control)
                && value != "/"
        })
        && values
            .iter()
            .enumerate()
            .all(|(index, value)| !values[..index].contains(value))
}
impl Preferences {
    /// Uses the saved provider choice or its protocol-specific default.
    pub fn provider_on(&self, desktop_id: &str, default: bool) -> bool {
        self.provider_choices
            .get(desktop_id)
            .copied()
            .unwrap_or(default)
    }
    pub fn shows(&self, category: Category) -> bool {
        self.enabled && self.categories.contains(&category)
    }
    /// Offers enabled categories plus the active filter, even if that filter was disabled.
    pub fn offered_categories(&self, current: Option<Category>) -> Vec<Category> {
        Category::ALL
            .iter()
            .copied()
            .filter(|category| self.shows(*category) || current == Some(*category))
            .collect()
    }
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let mut result: Self = serde_json::from_value(value.clone())
            .map_err(|_| tr("shell-invalid-quick-search-preferences"))?;
        // Results from inside apps arrived with sources 1.
        if result.sources < 1
            && !result.categories.contains(&Category::InApps)
            && result.categories.len() < Category::ALL.len()
        {
            result.categories.push(Category::InApps);
        }
        result.sources = result.sources.max(SOURCES);
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(4..=12).contains(&self.rows)
            || !(1..=1440).contains(&self.clipboard_minutes)
            || self.categories.len() > Category::ALL.len()
            || self.provider_choices.len() > 64
            || self.provider_choices.keys().any(|key| !provider_key(key))
            || self
                .categories
                .iter()
                .enumerate()
                .any(|(i, c)| self.categories[..i].contains(c))
            || !paths(&self.roots)
            || !paths(&self.exclusions)
            || self.usage.len() > 128
            || self.favorites.len() > 32
            || self.aliases.len() > 32
            || self.usage.keys().any(|key| !stable_key(key))
            || self.favorites.iter().any(|key| !stable_key(key))
            || self
                .favorites
                .iter()
                .enumerate()
                .any(|(i, key)| self.favorites[..i].contains(key))
            || self
                .aliases
                .iter()
                .any(|(key, value)| !alias_key(key) || !stable_key(value))
        {
            return Err(tr("shell-quick-search-preferences-exceed-their-limits").into());
        }
        Ok(())
    }
    pub fn preview(&mut self, change: &Change) -> Result<(), String> {
        let mut next = self.clone();
        match change {
            Change::Enabled(value) => next.enabled = *value,
            Change::Animations(value) => next.animations = *value,
            Change::BackgroundFade(value) => next.background_fade = *value,
            Change::Compact(value) => next.compact = *value,
            Change::Rows(value) => next.rows = *value,
            Change::Category(category, value) => {
                next.categories.retain(|entry| entry != category);
                if *value {
                    next.categories.push(*category);
                }
            }
            Change::Roots(value) => next.roots.clone_from(value),
            Change::Exclusions(value) => next.exclusions.clone_from(value),
            Change::Root(path, add) | Change::ExcludedRoot(path, add) => {
                let roots = if matches!(change, Change::Root(..)) {
                    &mut next.roots
                } else {
                    &mut next.exclusions
                };
                roots.retain(|root| root != path);
                if *add {
                    roots.push(path.clone());
                }
            }
            Change::Recent(value) => {
                next.recent = *value;
                if !value {
                    next.usage.clear();
                }
            }
            Change::ClearUsage => next.usage.clear(),
            Change::RecordUse(key) if next.recent && stable_key(key) => {
                if next.usage.contains_key(key) || next.usage.len() < 128 {
                    let count = next.usage.entry(key.clone()).or_default();
                    *count = count.saturating_add(1);
                }
            }
            Change::RecordUse(_) => {}
            Change::Favorite(key, value) => {
                next.favorites.retain(|entry| entry != key);
                if *value {
                    next.favorites.push(key.clone());
                }
            }
            Change::Alias(key, value) => {
                let key = key.trim().to_ascii_lowercase();
                if let Some(value) = value {
                    if !alias_key(&key) {
                        return Err(
                            tr("shell-use-2-16-letters-a-z-or-digits-for-a-quick-key").into()
                        );
                    }
                    if next
                        .aliases
                        .get(&key)
                        .is_some_and(|current| current != value)
                    {
                        return Err(tr("shell-quick-key-is-already-assigned-remove-its-old-assignment-in-settings-first").into());
                    }
                    next.aliases.insert(key, value.clone());
                } else {
                    next.aliases.remove(&key);
                }
            }
            Change::ClipboardMinutes(value) => next.clipboard_minutes = *value,
            Change::ClearClipboard => {
                next.clipboard_generation = next
                    .clipboard_generation
                    .checked_add(1)
                    .ok_or_else(|| tr("shell-clipboard-generations-exhausted"))?;
            }
            Change::WebProvider(value) => next.web_provider = *value,
            Change::Provider(key, on) => {
                if !provider_key(key) {
                    return Err(tr("shell-unknown-search-provider").into());
                }
                next.provider_choices.insert(key.clone(), *on);
            }
            Change::Reset => next = Self::default(),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn update_json(document: &mut serde_json::Value, change: &Change) -> Result<(), String> {
        if !document.is_object() {
            return Err(tr("shell-settings-must-be-an-object").into());
        }
        let mut value = Self::from_json(document.get("quick_search"))?;
        value.preview(change)?;
        let encoded = serde_json::to_value(value)
            .map_err(|_| tr("shell-cannot-encode-quick-search-preferences"))?;
        if !document["quick_search"].is_object() {
            document["quick_search"] = serde_json::json!({});
        }
        let object = document["quick_search"]
            .as_object_mut()
            .ok_or_else(|| tr("shell-invalid-quick-search-preferences"))?;
        for (key, value) in encoded
            .as_object()
            .ok_or_else(|| tr("shell-invalid-quick-search-preferences"))?
        {
            object.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}
