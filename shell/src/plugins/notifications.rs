use crate::notifications::{payload::Urgency, server::Snapshot};
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, time::Instant};

pub const MAX_ITEMS: usize = 8;
pub const MAX_SOURCE_CHARS: usize = 96;
pub const MAX_TITLE_CHARS: usize = 128;
pub const MAX_PREVIEW_CHARS: usize = 256;

pub struct Feed {
    namespace: String,
    next: u64,
    identities: BTreeMap<(u64, u32), String>,
}
impl Feed {
    pub fn new() -> Result<Self, String> {
        let mut random = [0; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|_| tr("shell-notification-feed-identity-is-unavailable"))?;
        Ok(Self {
            namespace: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            next: 0,
            identities: BTreeMap::new(),
        })
    }
    pub fn frame(&mut self, snapshot: Option<&Snapshot>) -> Result<Value, String> {
        let Some(snapshot) = snapshot else {
            self.identities.clear();
            return Ok(
                json!({"provider":"notifications","event":"snapshot","available":false,"total":0,"items":[],"truncated":false}),
            );
        };
        let now = Instant::now();
        let active = snapshot
            .active
            .iter()
            .filter(|entry| entry.expires.is_none_or(|at| at > now));
        let total = active.clone().count();
        let entries: Vec<_> = active.take(MAX_ITEMS).collect();
        self.identities.retain(|key, _| {
            entries
                .iter()
                .any(|entry| (entry.key.epoch, entry.key.id) == *key)
        });
        let mut items = Vec::new();
        for entry in entries {
            let key = (entry.key.epoch, entry.key.id);
            if !self.identities.contains_key(&key) {
                self.next = self
                    .next
                    .checked_add(1)
                    .ok_or_else(|| tr("shell-notification-feed-identities-exhausted"))?;
                self.identities
                    .insert(key, format!("notice:{}:{}", self.namespace, self.next));
            }
            let urgency = match entry.draft.urgency {
                Urgency::Low => "low",
                Urgency::Normal => "normal",
                Urgency::Critical => "critical",
            };
            items.push(json!({
                "id":self.identities[&key],
                "version":entry.key.revision,
                "source":entry.origin.label.chars().take(MAX_SOURCE_CHARS).collect::<String>(),
                "title":entry.draft.summary.chars().take(MAX_TITLE_CHARS).collect::<String>(),
                "preview":entry.draft.body.chars().take(MAX_PREVIEW_CHARS).collect::<String>(),
                "urgency":urgency,
                "unread":entry.unread,
                "received_ms":entry.received_ms,
                "updated_ms":entry.updated_ms,
                "has_image":entry.draft.image.is_some(),
            }));
        }
        Ok(
            json!({"provider":"notifications","event":"snapshot","available":true,"total":total,"items":items,"truncated":total>MAX_ITEMS}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{
        model::{Archived, CloseReason, Key, Notification, Policy},
        origin::Origin,
        payload::{Draft, Image},
    };
    use std::sync::Arc;

    fn snapshot() -> Snapshot {
        let mut snapshot = Snapshot::empty(1, Policy::default());
        snapshot.active = (1..=64)
            .map(|id| {
                Arc::new(Notification {
                    key: Key {
                        epoch: 1,
                        id,
                        revision: u64::from(id),
                    },
                    origin: Origin {
                        sender: format!(":1.{id}"),
                        principal: format!("exe:{id:064x}"),
                        label: "S".repeat(200),
                        icon: "bell".into(),
                        app_icon: None,
                        desktop: None,
                        pid: id,
                        start: 1,
                        device: 1,
                        inode: 1,
                        portal: false,
                    },
                    draft: Draft {
                        app_name: "untrusted caller name".into(),
                        desktop_entry: None,
                        replaces: 0,
                        summary: "🦀".repeat(256),
                        body: "🦀".repeat(4096),
                        actions: vec![],
                        reply: None,
                        image: None,
                        urgency: Urgency::Normal,
                        value: None,
                        resident: false,
                        transient: false,
                        timeout_ms: 0,
                        saved_file: None,
                    },
                    received_ms: 1,
                    received_at: std::time::Instant::now(),
                    updated_ms: 2,
                    unread: true,
                    expires: None,
                    toast: crate::notifications::toast::Toast::default(),
                })
            })
            .collect();
        snapshot
    }
    #[test]
    fn previews_are_bounded_and_exclude_transport_identity_and_action_authority() {
        let mut feed = Feed::new().unwrap();
        let mut snapshot = snapshot();
        Arc::make_mut(&mut snapshot.active[0]).draft.image =
            Some(Image::from_rgba(1, 1, vec![255, 0, 0, 255]).unwrap());
        snapshot.history.push(Archived {
            id: 1,
            principal: "archived-principal".into(),
            source: "Archived source".into(),
            summary: "Archived title".into(),
            body: "archive-only-marker".into(),
            image: None,
            received_ms: 1,
            closed_ms: 2,
            reason: CloseReason::Application,
        });
        let frame = feed.frame(Some(&snapshot)).unwrap();
        assert!(!frame.to_string().contains("archive-only-marker"));
        let items = frame["items"].as_array().unwrap();
        assert_eq!(items[0]["has_image"], true);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(frame["total"], 64);
        assert_eq!(frame["truncated"], true);
        assert_eq!(
            items[0]["source"].as_str().unwrap().chars().count(),
            MAX_SOURCE_CHARS
        );
        assert_eq!(
            items[0]["title"].as_str().unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
        assert_eq!(
            items[0]["preview"].as_str().unwrap().chars().count(),
            MAX_PREVIEW_CHARS
        );
        for field in [
            "sender",
            "pid",
            "principal",
            "actions",
            "reply",
            "body",
            "image",
        ] {
            assert!(items[0].get(field).is_none(), "{field}");
        }
        assert!(
            serde_json::to_vec(&json!({"type":"provider","data":frame}))
                .unwrap()
                .len()
                < super::super::worker::MAX_FRAME_BYTES / 2
        );
        assert_eq!(feed.identities.len(), MAX_ITEMS);
    }
}
