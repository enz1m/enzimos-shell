use enzui_shell::{i18n::tr, settings_catalog::SearchRank};
use enzui_shell::tr;
/// How many apps the launcher's blank page shows in its icon row.
pub const TOP_APPS: usize = 6;

pub mod calculator;
pub mod catalogue;
pub mod compositor;
pub mod files;
pub mod preferences;
pub mod providers;

use enzui_core::palette::{self, MatchQuality};
pub use preferences::Category;
use preferences::Preferences;
use std::{path::PathBuf, sync::Arc};

pub const MAX_QUERY_CHARS: usize = 256;
pub const MAX_RESULTS: usize = 40;
const PARTITION_THRESHOLD: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppTarget {
    Prepared(String),
    Desktop {
        path: PathBuf,
        identity: catalogue::FileIdentity,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowTarget {
    pub address: String,
    pub stable_id: String,
    pub pid: u32,
    pub start: u64,
    pub sidebar: Option<u32>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopAction {
    Light(bool),
    Tiling(bool),
    Dnd(bool),
    NotificationCenter,
    ControlCenter,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    App(AppTarget),
    Window(WindowTarget),
    Settings(&'static str),
    SettingsSection(&'static str),
    Desktop(DesktopAction),
    Copy(String),
    File {
        path: PathBuf,
        identity: catalogue::FileIdentity,
        directory: bool,
    },
    Web(String),
    Clipboard(u64),
    /// One of an app's own results, opened by that app.
    Provider {
        provider: Box<providers::Declaration>,
        id: String,
        query: String,
    },
    /// The app's own search for the query.
    ProviderSearch {
        provider: Box<providers::Declaration>,
        query: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub category: Category,
    pub title: String,
    pub subtitle: String,
    pub verb: &'static str,
    pub icon: String,
    /// The application's own icon file from the icon theme, drawn instead of
    /// `icon` when present.
    pub image: Option<std::path::PathBuf>,
    pub target: Option<Target>,
    /// The app a result comes from, heading its group.
    pub source: Option<Arc<str>>,
    normalized: Arc<String>,
    normalized_title: Arc<String>,
    settings_rank: Option<SearchRank>,
}
impl Item {
    pub fn new(
        id: impl Into<String>,
        category: Category,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        verb: &'static str,
        icon: impl Into<String>,
        target: Option<Target>,
    ) -> Self {
        let title = title.into();
        let subtitle = subtitle.into();
        Self {
            id: id.into(),
            category,
            normalized: Arc::new(palette::normalize(&format!("{title} {subtitle}"))),
            normalized_title: Arc::new(palette::normalize(&title)),
            settings_rank: None,
            title,
            subtitle,
            verb,
            icon: icon.into(),
            image: None,
            target,
            source: None,
        }
    }
    /// Whether this result can launch an installed application.
    pub fn is_installed_app(&self) -> bool {
        matches!(self.target, Some(Target::App(_)))
    }
    #[must_use]
    pub fn with_source(mut self, source: Arc<str>) -> Self {
        self.source = Some(source);
        self
    }
    #[must_use]
    pub fn with_image(mut self, image: Option<std::path::PathBuf>) -> Self {
        self.image = image;
        self
    }
    /// The same item under another title; what it is found by (its
    /// keywords and the old title among them) is kept.
    #[must_use]
    pub fn retitled(mut self, title: String) -> Self {
        self.normalized = Arc::new(format!(
            "{} {}",
            palette::normalize(&title),
            self.normalized
        ));
        self.normalized_title = Arc::new(palette::normalize(&title));
        self.title = title;
        self
    }
    #[must_use]
    pub fn search_terms(mut self, text: &str) -> Self {
        let limited: String = text.chars().take(1024 * 1024).collect();
        self.normalized = Arc::new(palette::normalize(&format!(
            "{} {}",
            self.normalized, limited
        )));
        self
    }
    #[must_use]
    pub fn with_settings_rank(mut self, rank: SearchRank) -> Self {
        self.settings_rank = Some(rank);
        self
    }
    pub fn action_hint(&self) -> String {
        if self.target.is_some() {
            format!("{} {}", self.verb, self.title)
        } else {
            self.subtitle.clone()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub id: String,
}
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub query: String,
    pub category: Option<Category>,
    pub results: Vec<Item>,
    pub selected: Option<String>,
    pub explicit_selection: bool,
    pub generation: u64,
    pub window_start: usize,
    pub error: Option<String>,
}
impl Model {
    pub fn set_query(&mut self, query: String) -> bool {
        if query.chars().count() > MAX_QUERY_CHARS || query.len() > 1024 || query.contains('\0') {
            self.error = Some(tr("shell-search-is-limited-to256-characters").into());
            self.results.clear();
            self.selected = None;
            self.invalidate();
            return false;
        }
        if self.query == query {
            return false;
        }
        self.query = query;
        self.error = None;
        self.explicit_selection = false;
        self.selected = None;
        self.window_start = 0;
        self.invalidate();
        true
    }
    pub fn set_category(&mut self, category: Option<Category>) {
        self.category = category;
        self.explicit_selection = false;
        self.selected = None;
        self.window_start = 0;
        self.invalidate();
    }
    pub fn invalidate(&mut self) {
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.results.clear();
            self.selected = None;
            self.error = Some(tr("shell-search-session-identities-exhausted").into());
        }
    }
    pub fn replace(&mut self, results: Vec<Item>) {
        if self.results == results {
            if !self.explicit_selection && self.selected.is_none() {
                self.selected = self.results.first().map(|item| item.id.clone());
            }
            return;
        }
        let previous = self.selected.clone();
        self.results = results;
        self.selected = if self.explicit_selection {
            previous.filter(|id| self.results.iter().any(|item| &item.id == id))
        } else {
            self.results.first().map(|item| item.id.clone())
        };
        self.invalidate();
    }
    pub fn selected_index(&self) -> Option<usize> {
        self.results
            .iter()
            .position(|item| Some(&item.id) == self.selected.as_ref())
    }
    pub fn selected_item(&self) -> Option<&Item> {
        self.selected_index()
            .and_then(|index| self.results.get(index))
    }
    pub fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let index = self.selected_index().map_or(0, |index| {
            index
                .saturating_add_signed(delta)
                .min(self.results.len() - 1)
        });
        self.selected = Some(self.results[index].id.clone());
        self.explicit_selection = true;
        self.invalidate();
    }
    pub fn ticket(&self, item: &Item) -> Ticket {
        Ticket {
            generation: self.generation,
            id: item.id.clone(),
        }
    }
    pub fn activate(&mut self, ticket: &Ticket) -> Result<Target, String> {
        if ticket.generation != self.generation {
            return Err(tr("shell-results-changed-choose-the-action-again").into());
        }
        let item = self
            .results
            .iter()
            .find(|item| item.id == ticket.id)
            .ok_or_else(|| tr("shell-this-result-is-no-longer-available"))?;
        let result = item.target.clone().ok_or_else(|| {
            if item.subtitle.is_empty() {
                tr("shell-this-action-is-unavailable").into()
            } else {
                item.subtitle.clone()
            }
        });
        self.selected = Some(ticket.id.clone());
        self.explicit_selection = true;
        self.invalidate();
        result
    }
    pub fn visible(&mut self, rows: usize) -> std::ops::Range<usize> {
        let window = palette::window_from(
            self.results.len(),
            rows,
            self.selected_index(),
            self.window_start,
        );
        self.window_start = window.start;
        window.start..window.end()
    }
    /// Returns a category heading at the visible start and each group boundary; single-category
    /// lists omit headings.
    pub fn group_heading(&self, index: usize, window: &std::ops::Range<usize>) -> Option<Category> {
        let item = self.results.get(index)?;
        let group = |other: &Item| (other.category, other.source.clone());
        if !self.results.iter().any(|other| group(other) != group(item)) {
            return None;
        }
        let opens = index == window.start
            || self
                .results
                .get(index.checked_sub(1)?)
                .is_none_or(|previous| group(previous) != group(item));
        opens.then_some(item.category)
    }
    /// The words a group heading shows: the app's name for results from
    /// inside an app, the category otherwise.
    pub fn heading_text(&self, index: usize) -> String {
        self.results.get(index).map_or_else(String::new, |item| {
            item.source
                .as_deref()
                .map_or_else(|| item.category.label().to_owned(), str::to_owned)
        })
    }
    /// How many headings [`Self::group_heading`] draws across `window`.
    pub fn heading_count(&self, window: &std::ops::Range<usize>) -> usize {
        window
            .clone()
            .filter(|index| self.group_heading(*index, window).is_some())
            .count()
    }
}
/// The loose match under the exact ones: `token` as the initials of the
/// title's words ("lw" for LibreOffice Writer), or as its letters in order
/// within a short run ("firfox" for Firefox). Letters scattered across the
/// whole title do not count: "fire" is not LibreOffice Impress.
fn subsequence(text: &str, token: &str) -> bool {
    let token: Vec<char> = token.chars().collect();
    if token.len() < 2 {
        return false;
    }
    let text: Vec<char> = text.chars().collect();
    let mut initials = text
        .iter()
        .enumerate()
        .filter(|(index, ch)| {
            ch.is_alphanumeric() && (*index == 0 || !text[index - 1].is_alphanumeric())
        })
        .map(|(_, ch)| *ch);
    if token
        .iter()
        .all(|wanted| initials.by_ref().any(|initial| initial == *wanted))
    {
        return true;
    }
    let run = token.len() + (token.len() / 2).max(2);
    (0..text.len())
        .filter(|start| text[*start] == token[0])
        .any(|start| {
            let mut at = start;
            token[1..].iter().all(|wanted| {
                text[at + 1..]
                    .iter()
                    .position(|ch| ch == wanted)
                    .is_some_and(|step| {
                        at += step + 1;
                        at - start < run
                    })
            })
        })
}
pub fn browse_installed_apps<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    preferences: &Preferences,
) -> Vec<Item> {
    rank_bounded(
        items
            .into_iter()
            .filter(|item| item.category == Category::Apps && item.target.is_some()),
        "",
        Some(Category::Apps),
        preferences,
        2048,
    )
}
pub fn rank<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    query: &str,
    category: Option<Category>,
    preferences: &Preferences,
) -> Vec<Item> {
    rank_bounded(items, query, category, preferences, MAX_RESULTS)
}
fn rank_bounded<'a>(
    items: impl IntoIterator<Item = &'a Item>,
    query: &str,
    category: Option<Category>,
    preferences: &Preferences,
    limit: usize,
) -> Vec<Item> {
    let query = palette::normalize(query.trim());
    let words = palette::tokens(&query);
    let alias = preferences.aliases.get(&query);
    let mut scored = Vec::new();
    for (ordinal, item) in items.into_iter().enumerate() {
        if !preferences.shows(item.category)
            || category.is_some_and(|category| category != item.category)
        {
            continue;
        }
        let shortcut = alias == Some(&item.id);
        let score = if shortcut {
            0
        } else if let Some(rank) = item.settings_rank {
            match rank {
                SearchRank::ExactPage => 1,
                SearchRank::PagePrefix => 2,
                SearchRank::SectionName => 3,
                SearchRank::ControlLabel => 4,
                SearchRank::Context => 5,
            }
        } else if item.normalized_title.as_str() == query {
            1
        } else if palette::matches_tokens(&item.normalized, &words) {
            match palette::quality(&item.normalized_title, &query) {
                Some(MatchQuality::Prefix) => 2,
                Some(MatchQuality::Word) => 3,
                _ => 4,
            }
        } else if words
            .iter()
            .all(|token| subsequence(&item.normalized_title, token))
        {
            5 + usize::from(item.normalized_title.chars().next() != query.chars().next())
        } else {
            continue;
        };
        let favorite = preferences.favorites.contains(&item.id);
        let usage = if preferences.recent {
            preferences
                .usage
                .get(&item.id)
                .copied()
                .unwrap_or(0)
                .min(1000)
        } else {
            0
        };
        scored.push((
            (
                !query.is_empty() && !item.is_installed_app(),
                score,
                item.target.is_none(),
                !favorite,
                std::cmp::Reverse(usage),
                if query.is_empty() {
                    Category::ALL
                        .iter()
                        .position(|category| *category == item.category)
                        .unwrap_or(7)
                } else {
                    0
                },
                match &item.target {
                    Some(Target::App(AppTarget::Prepared(id))) => crate::catalog::APPS
                        .iter()
                        .position(|app| app.id == id)
                        .unwrap_or(6),
                    _ => 7,
                },
                if item.category == Category::Clipboard || item.settings_rank.is_some() {
                    ordinal
                } else {
                    0
                },
                item.normalized_title.as_str(),
                item.id.as_str(),
                ordinal,
            ),
            item,
        ));
    }
    if scored.len() > limit && scored.len() > PARTITION_THRESHOLD {
        scored.select_nth_unstable_by(limit, |a, b| a.0.cmp(&b.0));
        scored.truncate(limit);
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, item)| item.clone())
        .collect()
}

/// Groups categories by their best result, preserving rank within each category.
pub fn group_by_category(results: Vec<Item>) -> Vec<Item> {
    let mut order: Vec<Category> = Vec::new();
    for item in &results {
        if !order.contains(&item.category) {
            order.push(item.category);
        }
    }
    if order.len() < 2 {
        return results;
    }
    let mut grouped = Vec::with_capacity(results.len());
    for category in order {
        grouped.extend(results.iter().filter(|i| i.category == category).cloned());
    }
    grouped
}

pub fn alias_target<'a>(
    query: &str,
    category: Option<Category>,
    preferences: &'a Preferences,
) -> Option<&'a String> {
    let id = preferences.aliases.get(&palette::normalize(query.trim()))?;
    let source = if id.starts_with("app:") {
        Category::Apps
    } else {
        Category::Actions
    };
    category
        .is_none_or(|category| category == source)
        .then_some(id)
}
pub fn utility_results(
    query: &str,
    category: Option<Category>,
    preferences: &Preferences,
) -> Vec<Item> {
    let mut results = Vec::new();
    let query = query.trim();
    if alias_target(query, category, preferences).is_some() {
        return results;
    }
    if preferences.shows(Category::Calculator)
        && category.is_none_or(|value| value == Category::Calculator)
        && !query.is_empty()
        && let Some(answer) = calculator::evaluate(query)
    {
        results.push(match answer {
            Ok(answer) => Item::new(
                "calculation",
                Category::Calculator,
                answer.value.clone(),
                answer.detail,
                tr("shell-copy"),
                "calculator",
                Some(Target::Copy(answer.value)),
            ),
            Err(error) => Item::new(
                "calculation-error",
                Category::Calculator,
                tr("shell-check-expression"),
                error,
                "",
                "calculator",
                None,
            ),
        });
    }
    if results.is_empty()
        && preferences.shows(Category::Web)
        && category.is_none_or(|value| value == Category::Web)
        && !query.is_empty()
    {
        if query.contains("://") && web_url(query).is_none() {
            results.push(Item::new(
                "web-invalid",
                Category::Web,
                tr("shell-address-cannot-be-opened"),
                tr("shell-use-an-http-s-address-without-credentials-or-control-characters"),
                "",
                "globe",
                None,
            ));
            return results;
        }
        let (title, subtitle, url) = if let Some(url) = web_url(query) {
            (
                query.to_owned(),
                tr("shell-open-this-http-s-address-in-the-default-browser").to_owned(),
                url,
            )
        } else {
            let search = query.trim_start_matches('?').trim();
            let escaped: String = search
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            (
                tr!("shell-search-3", "arg0" => preferences.web_provider.label().to_owned()),
                tr!("shell-sent-only-when-opened", "arg0" => search.to_owned()),
                format!("{}{escaped}", preferences.web_provider.prefix()),
            )
        };
        results.push(Item::new(
            "web-search",
            Category::Web,
            title,
            subtitle,
            tr("shell-open"),
            "globe",
            Some(Target::Web(url)),
        ));
    }
    results
}
pub fn web_url(value: &str) -> Option<String> {
    if value.len() > 2048 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let host = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?
        .split(['/', '?', '#'])
        .next()?;
    (!host.is_empty() && !host.contains('@') && !host.starts_with('-') && !host.contains('\\'))
        .then(|| value.to_owned())
}
pub type Inventory = Arc<Vec<Item>>;

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, title: &str) -> Item {
        Item::new(
            id,
            Category::Apps,
            title,
            "Application",
            "Open",
            "app-window",
            Some(Target::App(AppTarget::Prepared(id.into()))),
        )
    }
    #[test]
    fn search_behaviour() {
        // Loose matches need initials or a short run.
        {
            let titles = [
                "Firefox",
                "LibreOffice Impress",
                "LibreOffice Writer",
                "Notification history retention",
            ];
            let items: Vec<_> = titles
                .iter()
                .enumerate()
                .map(|(index, title)| item(&format!("app:{index}"), title))
                .collect();
            let found = |query: &str| -> Vec<String> {
                rank(items.iter(), query, None, &Preferences::default())
                    .into_iter()
                    .map(|item| item.title)
                    .collect()
            };
            assert_eq!(found("fire"), ["Firefox"]);
            assert_eq!(found("firfox"), ["Firefox"]);
            assert_eq!(found("lw"), ["LibreOffice Writer"]);
            assert_eq!(found("nhr"), ["Notification history retention"]);
        }
        {
            let items = [
                Item::new(
                    "setting:display",
                    Category::Actions,
                    "Display",
                    "Settings",
                    "Open",
                    "settings",
                    Some(Target::Settings("display")),
                ),
                item("app:keyword", "Digital Screen Player").search_terms("display"),
                item("app:prefix", "Display Viewer"),
            ];
            let preferences = Preferences::default();
            let found = rank(items.iter(), "display", None, &preferences);
            assert_eq!(
                found
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["app:prefix", "app:keyword", "setting:display"],
            );
            let found = rank_bounded(items.iter(), "display", None, &preferences, 2);
            assert_eq!(
                found
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["app:prefix", "app:keyword"],
            );
        }
        {
            let catalogue = crate::settings_catalog::search_groups("display", |_, _| true);
            let items: Vec<_> = catalogue
                .into_iter()
                .take(3)
                .map(|group| {
                    Item::new(
                        group.destination.id(),
                        Category::Actions,
                        group.title,
                        group.category.title(),
                        "Open",
                        group.category.icon(),
                        None,
                    )
                    .with_settings_rank(group.rank)
                })
                .collect();
            let found = rank(items.iter(), "display", None, &Preferences::default());
            assert_eq!(found[0].id, "display");
            assert_eq!(found[1].id, "section/datetime/language");
        }
        // Bounded projection matches full order across categories and queries.
        {
            let mut preferences = Preferences {
                categories: Category::ALL.to_vec(),
                ..Preferences::default()
            };
            preferences.favorites.push("app:0017".into());
            preferences.usage.insert("app:0027".into(), 1000);
            preferences.aliases.insert("zz".into(), "app:0400".into());
            preferences.aliases.insert("text".into(), "app:0400".into());
            for count in [
                0,
                1,
                39,
                40,
                41,
                128,
                512,
                PARTITION_THRESHOLD,
                PARTITION_THRESHOLD + 1,
                20_000,
            ] {
                let mut items: Vec<_> = (0..count)
                    .map(|index| {
                        let mut value = Item::new(
                            format!("app:{index:04}"),
                            Category::ALL[index % Category::ALL.len()],
                            ["Text", "Téxt", "Context", "Fixture report", "Same"][index % 5],
                            format!("Entry {index}"),
                            "Open",
                            "app-window",
                            Some(Target::Copy(index.to_string())),
                        );
                        if index % 13 == 0 {
                            value.target = None;
                        }
                        value
                    })
                    .collect();
                let mut seed = 17_usize;
                for index in (1..items.len()).rev() {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    items.swap(index, seed % (index + 1));
                }
                for _ in 0..2 {
                    for query in ["", "text", "txt", "TÉXT", "fixture", "zz", "nomatch"] {
                        for category in [
                            None,
                            Some(Category::Apps),
                            Some(Category::Clipboard),
                            Some(Category::Files),
                        ] {
                            let mut expected = rank_bounded(
                                items.iter(),
                                query,
                                category,
                                &preferences,
                                items.len(),
                            );
                            expected.truncate(MAX_RESULTS);
                            assert_eq!(
                                rank(items.iter(), query, category, &preferences),
                                expected,
                                "count={count} query={query} category={category:?}"
                            );
                        }
                    }
                    items.reverse();
                }
            }
        }
        // Activation rejects stale query and replaced action.
        {
            let mut m = Model::default();
            m.replace(vec![item("app:a", "Text")]);
            let ticket = m.ticket(m.selected_item().unwrap());
            m.set_query("Files".into());
            assert!(m.activate(&ticket).is_err());
            m.replace(vec![item("app:b", "Files")]);
            assert!(m.activate(&ticket).is_err());
            assert!(m.activate(&m.ticket(m.selected_item().unwrap())).is_ok());
        }
        // Urls do not accept commands credentials or controls.
        {
            for url in [
                "file:///tmp/a",
                "javascript:alert(1)",
                "https://user:password@example.com",
                "https://example.com\ncommand",
                "https://",
            ] {
                assert!(web_url(url).is_none());
            }
            assert_eq!(
                web_url("https://example.com/path?q=1").as_deref(),
                Some("https://example.com/path?q=1")
            );
        }
    }
}
