use super::{
    model::{Archived, Key, MAX_ACTIVE},
    payload::Urgency,
};
use enzui_core::live_region::Announcement;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct Arrivals {
    seen: BTreeSet<Key>,
    speaking: Option<Key>,
}
impl Arrivals {
    pub fn sync(&mut self, active: impl IntoIterator<Item = Key>, visible: Option<Key>) {
        let active: BTreeSet<_> = active.into_iter().take(MAX_ACTIVE).collect();
        self.speaking = visible.filter(|key| {
            active.contains(key) && (!self.seen.contains(key) || self.speaking == Some(*key))
        });
        self.seen = active;
    }
    pub fn announcement(&self, key: Key, urgency: Urgency) -> Announcement {
        if self.speaking != Some(key) {
            return Announcement::Silent;
        }
        if urgency == Urgency::Critical {
            Announcement::Alert
        } else {
            Announcement::Status
        }
    }
}

pub fn announcement_label(source: &str, summary: &str, body: &str, expanded: bool) -> String {
    let body = if expanded {
        body.to_owned()
    } else {
        body.chars().take(400).collect()
    };
    [source, summary, &body]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(". ")
}

pub fn grouped_order<'a>(principals: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut groups = Vec::<Vec<usize>>::new();
    let mut positions = BTreeMap::new();
    for (index, principal) in principals.into_iter().enumerate() {
        let position = *positions.entry(principal).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[position].push(index);
    }
    groups.into_iter().flatten().collect()
}
pub fn history_matches(entry: &Archived, query: &str) -> bool {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return true;
    }
    let source = entry.source.to_lowercase();
    let summary = entry.summary.to_lowercase();
    let body = entry.body.to_lowercase();
    terms
        .iter()
        .all(|term| source.contains(term) || summary.contains(term) || body.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(id: u32, revision: u64) -> Key {
        Key {
            epoch: 1,
            id,
            revision,
        }
    }
    #[test]
    fn one_fresh_arrival_speaks_without_reannouncing_promoted_or_read_cards() {
        let mut arrivals = Arrivals::default();
        let (a, b) = (key(1, 1), key(2, 2));
        arrivals.sync([b, a], Some(b));
        assert_eq!(
            arrivals.announcement(b, Urgency::Normal),
            Announcement::Status
        );
        assert_eq!(
            arrivals.announcement(a, Urgency::Critical),
            Announcement::Silent
        );
        arrivals.sync([b, a], Some(b));
        assert_eq!(
            arrivals.announcement(b, Urgency::Normal),
            Announcement::Status
        );
        arrivals.sync([a], Some(a));
        assert_eq!(
            arrivals.announcement(a, Urgency::Normal),
            Announcement::Silent
        );
        arrivals.sync([a], None);
        arrivals.sync([a], Some(a));
        assert_eq!(
            arrivals.announcement(a, Urgency::Normal),
            Announcement::Silent
        );
        let replacement = key(1, 3);
        arrivals.sync([replacement], Some(replacement));
        assert_eq!(
            arrivals.announcement(replacement, Urgency::Critical),
            Announcement::Alert
        );
        assert_eq!(arrivals.seen.len(), 1);
    }
}
