use super::content::Content;
use crate::plugins::service::Owner;
use enzui_shell::i18n::tr;
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_OWNER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OWNER_ITEMS: usize = 128;
const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;
const MAX_TOTAL_ITEMS: usize = 8 * MAX_OWNER_ITEMS;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
struct Blob {
    owner: Owner,
    authority: Arc<AtomicBool>,
    content: Arc<Content>,
    inserted: Instant,
}
#[derive(Default)]
pub struct History {
    blobs: BTreeMap<String, Blob>,
    bytes: usize,
}
pub struct Published {
    pub handle: String,
    pub removed: Vec<String>,
    pub fresh: bool,
}
impl History {
    pub fn insert(
        &mut self,
        owner: Owner,
        authority: Arc<AtomicBool>,
        content: Arc<Content>,
        now: Instant,
    ) -> Result<Published, String> {
        if !authority.load(Ordering::Acquire) {
            return Err(tr("shell-clipboard-history-authority-changed").into());
        }
        if content.size() > MAX_OWNER_BYTES {
            return Err(tr("shell-clipboard-entry-exceeds-history-budget").into());
        }
        if let Some((handle, _)) = self.blobs.iter().find(|(_, blob)| {
            blob.owner == owner
                && blob.content.digest == content.digest
                && now.duration_since(blob.inserted) < MAX_AGE
        }) {
            return Ok(Published {
                handle: handle.clone(),
                removed: Vec::new(),
                fresh: false,
            });
        }
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|_| tr("shell-clipboard-handles-are-unavailable"))?;
        let handle: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        if self.blobs.contains_key(&handle) {
            return Err(tr("shell-clipboard-handle-collision").into());
        }
        let mut removed = Vec::new();
        loop {
            let owned: Vec<_> = self
                .blobs
                .iter()
                .filter(|(_, blob)| blob.owner == owner)
                .collect();
            let owned_bytes: usize = owned.iter().map(|(_, blob)| blob.content.size()).sum();
            if owned.len() < MAX_OWNER_ITEMS && owned_bytes + content.size() <= MAX_OWNER_BYTES {
                break;
            }
            let oldest = owned
                .into_iter()
                .min_by_key(|(_, blob)| blob.inserted)
                .map(|(id, _)| id.clone())
                .ok_or_else(|| tr("shell-clipboard-history-budget-is-unavailable"))?;
            self.remove(&oldest);
            removed.push(oldest);
        }
        if self.blobs.len() >= MAX_TOTAL_ITEMS || self.bytes + content.size() > MAX_TOTAL_BYTES {
            return Err(tr("shell-clipboard-history-is-full").into());
        }
        self.bytes += content.size();
        self.blobs.insert(
            handle.clone(),
            Blob {
                owner,
                authority,
                content,
                inserted: now,
            },
        );
        Ok(Published {
            handle,
            removed,
            fresh: true,
        })
    }
    fn remove(&mut self, handle: &str) {
        if let Some(blob) = self.blobs.remove(handle) {
            self.bytes -= blob.content.size();
        }
    }
    pub fn get(&self, owner: &Owner, handle: &str, now: Instant) -> Option<Arc<Content>> {
        self.blobs
            .get(handle)
            .filter(|blob| {
                blob.owner == *owner
                    && blob.authority.load(Ordering::Acquire)
                    && now.duration_since(blob.inserted) < MAX_AGE
            })
            .map(|blob| std::sync::Arc::clone(&blob.content))
    }
    pub fn release(&mut self, owner: &Owner, handles: &[String]) -> Vec<String> {
        let removed: Vec<_> = handles
            .iter()
            .filter(|handle| {
                self.blobs
                    .get(*handle)
                    .is_some_and(|blob| blob.owner == *owner)
            })
            .cloned()
            .collect();
        for handle in &removed {
            self.remove(handle);
        }
        removed
    }
    pub fn revoke(&mut self, owner: &Owner) {
        let handles: Vec<_> = self
            .blobs
            .iter()
            .filter(|(_, blob)| blob.owner == *owner)
            .map(|(id, _)| id.clone())
            .collect();
        for handle in handles {
            self.remove(&handle);
        }
    }
    pub fn expire(&mut self, now: Instant) -> Vec<(Owner, String)> {
        let removed: Vec<_> = self
            .blobs
            .iter()
            .filter(|(_, blob)| {
                !blob.authority.load(Ordering::Acquire)
                    || now.duration_since(blob.inserted) >= MAX_AGE
            })
            .map(|(handle, blob)| (blob.owner.clone(), handle.clone()))
            .collect();
        for (_, handle) in &removed {
            self.remove(handle);
        }
        removed
    }
    pub fn clear(&mut self) {
        self.blobs.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::content::Mime;
    use super::*;
    fn owner(run: u64) -> Owner {
        Owner {
            id: "fixture".into(),
            activation: 1,
            run,
        }
    }
    fn content(value: &[u8]) -> Arc<Content> {
        Arc::new(Content::decode(Mime::Text, value.to_vec()).unwrap())
    }
    #[test]
    fn history_behaviour() {
        // Resources are owner bound revocable and deduplicated without renewing retention.
        {
            let mut history = History::default();
            let authority = Arc::new(AtomicBool::new(true));
            let now = Instant::now();
            let first = history
                .insert(
                    owner(1),
                    std::sync::Arc::clone(&authority),
                    content(b"fixture"),
                    now,
                )
                .unwrap();
            let duplicate = history
                .insert(
                    owner(1),
                    std::sync::Arc::clone(&authority),
                    content(b"fixture"),
                    now + Duration::from_secs(10),
                )
                .unwrap();
            assert_eq!(first.handle, duplicate.handle);
            assert!(!duplicate.fresh);
            assert!(history.get(&owner(2), &first.handle, now).is_none());
            assert!(
                history
                    .release(&owner(2), std::slice::from_ref(&first.handle))
                    .is_empty()
            );
            assert_eq!(history.bytes, 7);
            assert!(
                history
                    .get(&owner(1), &first.handle, now + MAX_AGE)
                    .is_none()
            );
            assert_eq!(history.expire(now + MAX_AGE).len(), 1);
            assert_eq!(history.bytes, 0);
            let next = history
                .insert(
                    owner(1),
                    std::sync::Arc::clone(&authority),
                    content(b"fixture"),
                    now,
                )
                .unwrap();
            assert_ne!(next.handle, first.handle);
            authority.store(false, Ordering::Release);
            assert!(history.get(&owner(1), &next.handle, now).is_none());
            assert!(
                history
                    .insert(owner(1), authority, content(b"fixture"), now)
                    .is_err()
            );
            assert_eq!(history.expire(now).len(), 1);
        }
        // Item eviction removes only the requesting owners oldest entry.
        {
            let mut history = History::default();
            let now = Instant::now();
            let authority = Arc::new(AtomicBool::new(true));
            let peer = history
                .insert(
                    owner(2),
                    std::sync::Arc::clone(&authority),
                    content(b"peer"),
                    now,
                )
                .unwrap();
            let mut first = None;
            for i in 0..=MAX_OWNER_ITEMS {
                let value = history
                    .insert(
                        owner(1),
                        std::sync::Arc::clone(&authority),
                        content(format!("fixture {i}").as_bytes()),
                        now + Duration::from_secs(i as u64),
                    )
                    .unwrap();
                if i == 0 {
                    first = Some(value.handle.clone());
                }
                if i == MAX_OWNER_ITEMS {
                    assert_eq!(value.removed, vec![first.clone().unwrap()]);
                }
            }
            assert!(history.get(&owner(1), &first.unwrap(), now).is_none());
            assert!(history.get(&owner(2), &peer.handle, now).is_some());
            history.revoke(&owner(1));
            assert_eq!(history.bytes, 4);
            history.clear();
            assert_eq!(history.bytes, 0);
        }
        // Byte eviction counts content and keeps usage bounded.
        {
            let mut history = History::default();
            let authority = Arc::new(AtomicBool::new(true));
            let now = Instant::now();
            for i in 0..18 {
                let mut value = vec![b'a'; 1024 * 1024];
                value[0] = b'a' + i;
                history
                    .insert(
                        owner(1),
                        std::sync::Arc::clone(&authority),
                        content(&value),
                        now + Duration::from_secs(u64::from(i)),
                    )
                    .unwrap();
                assert!(history.bytes <= MAX_OWNER_BYTES);
            }
            assert_eq!(history.blobs.len(), 16);
        }
    }
}
