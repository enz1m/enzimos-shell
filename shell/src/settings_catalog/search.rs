use super::{Category, Entry, SECTIONS, Section, all_entries, category, find};
use crate::i18n::{english, tr};
use enzui_core::palette::normalize;

pub const DESKTOP_GROUP_LIMIT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDestination {
    Page(Category),
    Section(&'static str),
}

impl SearchDestination {
    pub fn section(id: &str) -> Option<Section> {
        let route = id.strip_prefix("section/")?;
        sections().find(|section| section.route == route)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::section(id)
            .map(|section| Self::Section(section.route))
            .or_else(|| category(id).map(Self::Page))
    }

    pub fn id(self) -> String {
        match self {
            Self::Page(page) => page.page().to_owned(),
            Self::Section(route) => format!("section/{route}"),
        }
    }

    pub fn route(self) -> &'static str {
        match self {
            Self::Page(page) => page.page(),
            Self::Section(route) => route,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SearchRank {
    ExactPage,
    PagePrefix,
    SectionName,
    ControlLabel,
    Context,
}

pub struct SearchGroup {
    pub destination: SearchDestination,
    pub category: Category,
    pub title: &'static str,
    pub controls: Vec<Entry>,
    pub rank: SearchRank,
}

fn sections() -> impl Iterator<Item = Section> {
    SECTIONS.iter().copied().chain([
        Section {
            category: Category::Search,
            label: "shell-apps-in-search",
            route: "quick-search/providers",
        },
        Section {
            category: Category::DateTime,
            label: "shell-world-clocks",
            route: "datetime/world_clocks",
        },
        Section {
            category: Category::Users,
            label: "shell-fingerprint",
            route: "users/fingerprint",
        },
        Section {
            category: Category::Users,
            label: "shell-profile",
            route: "users/profile",
        },
        Section {
            category: Category::Users,
            label: "shell-add-user",
            route: "users/new",
        },
        Section {
            category: Category::Agents,
            label: "shell-voice-typing",
            route: "agents/voice-typing",
        },
    ])
}

fn section_for(entry: Entry) -> Option<Section> {
    let key = match entry.section_key {
        "shell-keyboard-shortcuts" => "shell-shortcuts",
        _ => entry.section_key,
    };
    sections().find(|section| {
        section.category == entry.category()
            && (section.label == key || section.route == entry.route)
    })
}

fn compact(text: &str) -> String {
    normalize(text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn name_score(key: &'static str, compact_query: &str, words: &[&str]) -> u8 {
    [tr(key), english(key)]
        .into_iter()
        .map(|name| {
            let compact_name = compact(name);
            if compact_name == compact_query {
                0
            } else if compact_name.starts_with(compact_query) {
                1
            } else {
                let name = normalize(name);
                if words.iter().all(|word| name.contains(word)) {
                    2
                } else {
                    3
                }
            }
        })
        .min()
        .unwrap_or(3)
}

pub fn search_groups(
    query: &str,
    visible: impl Fn(Category, Option<Entry>) -> bool,
) -> Vec<SearchGroup> {
    let phrase = normalize(query.trim());
    let compact_query = compact(&phrase);
    if compact_query.is_empty() {
        return Vec::new();
    }

    let words: Vec<_> = phrase.split_whitespace().collect();
    let control_score = |entry: Entry| {
        let quality = [entry.title(), entry.english_title()]
            .into_iter()
            .map(|title| {
                let title = normalize(title);
                if title == phrase {
                    0
                } else if title.starts_with(&phrase) {
                    1
                } else if words.iter().all(|word| title.contains(word)) {
                    2
                } else {
                    3
                }
            })
            .min()
            .unwrap_or(3);
        (quality, entry.shortcut.is_some())
    };
    let mut groups = Vec::new();
    for page in Category::ALL
        .into_iter()
        .filter(|page| visible(*page, None))
    {
        let page_label = page.title_key();
        let page_score = name_score(page_label, &compact_query, &words);
        let page_rank = match page_score {
            0 => SearchRank::ExactPage,
            1 => SearchRank::PagePrefix,
            _ => SearchRank::Context,
        };
        let page_named = page_rank != SearchRank::Context;
        let mut page_group = SearchGroup {
            destination: SearchDestination::Page(page),
            category: page,
            title: page.title(),
            controls: Vec::new(),
            rank: page_rank,
        };
        let mut section_groups = Vec::<(u8, SearchGroup)>::new();
        for entry in
            all_entries().filter(|entry| entry.category() == page && visible(page, Some(*entry)))
        {
            if page_named {
                page_group.controls.push(entry);
                continue;
            }
            let section = section_for(entry).filter(|section| {
                find(section.route).is_some_and(|anchor| visible(page, Some(anchor)))
            });
            let section_score = section.map_or(3, |section| {
                name_score(section.label, &compact_query, &words)
            });
            if section_score == 3 && !entry.matches(&words) {
                continue;
            }
            if let Some(section) = section {
                let destination = SearchDestination::Section(section.route);
                if let Some((_, group)) = section_groups
                    .iter_mut()
                    .find(|(_, group)| group.destination == destination)
                {
                    group.controls.push(entry);
                } else {
                    section_groups.push((
                        section_score,
                        SearchGroup {
                            destination,
                            category: page,
                            title: tr(section.label),
                            controls: vec![entry],
                            rank: if section_score < 3 {
                                SearchRank::SectionName
                            } else {
                                SearchRank::Context
                            },
                        },
                    ));
                }
            } else {
                page_group.controls.push(entry);
            }
        }
        if page_named || !page_group.controls.is_empty() {
            groups.push((page_score, page_group));
        }
        groups.extend(section_groups);
    }

    for (_, group) in &mut groups {
        group
            .controls
            .sort_by_cached_key(|entry| control_score(*entry));
        if group.rank == SearchRank::Context
            && group
                .controls
                .first()
                .is_some_and(|entry| control_score(*entry).0 < 3)
        {
            group.rank = SearchRank::ControlLabel;
        }
    }
    groups.sort_by_cached_key(|(name_score, group)| {
        (
            group.rank,
            *name_score,
            group
                .controls
                .first()
                .copied()
                .map_or((4, false), control_score),
            normalize(group.title),
        )
    });
    groups.into_iter().map(|(_, group)| group).collect()
}
