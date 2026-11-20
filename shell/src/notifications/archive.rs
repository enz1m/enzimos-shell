use super::{
    model::{Archived, CloseReason, MAX_HISTORY, Policy},
    payload::{self, Image},
    server::Snapshot,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use enzui_shell::i18n::tr;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error, SeqAccess, Visitor},
};
use std::{
    fmt, fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const MAX_FILE_BYTES: usize = 34 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Picture {
    width: u32,
    height: u32,
    rgba: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    principal: String,
    source: String,
    summary: String,
    body: String,
    image: Option<Picture>,
    received_ms: u64,
    closed_ms: u64,
    reason: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    generation: u64,
    #[serde(deserialize_with = "records")]
    records: Vec<Record>,
}
fn records<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Record>, D::Error> {
    struct Bounded;
    impl<'de> Visitor<'de> for Bounded {
        type Value = Vec<Record>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(tr("shell-bounded-notification-history"))
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut records = Vec::new();
            while let Some(record) = sequence.next_element()? {
                if records.len() == MAX_HISTORY {
                    return Err(A::Error::custom(tr(
                        "shell-too-many-historical-notifications",
                    )));
                }
                records.push(record);
            }
            Ok(records)
        }
    }
    deserializer.deserialize_seq(Bounded)
}
impl Record {
    fn from_entry(entry: &Archived) -> Self {
        Self {
            principal: entry.principal.clone(),
            source: entry.source.clone(),
            summary: entry.summary.clone(),
            body: entry.body.clone(),
            image: entry.image.as_ref().map(|image| {
                let (pixels, width, height) = image.pixels();
                Picture {
                    width,
                    height,
                    rgba: STANDARD.encode(pixels),
                }
            }),
            received_ms: entry.received_ms,
            closed_ms: entry.closed_ms,
            reason: entry.reason as u32,
        }
    }
    fn into_entry(self, id: u64) -> Result<Archived, String> {
        let mut policy = Policy::default();
        policy
            .rules
            .insert(self.principal.clone(), super::model::Rule::Allow);
        policy.validate()?;
        let image = self
            .image
            .map(|image| {
                if image.width == 0
                    || image.height == 0
                    || image.width > 256
                    || image.height > 256
                    || image.rgba.len() > 349528
                {
                    return Err(tr("shell-invalid-historical-image-dimensions").to_owned());
                }
                let pixels = STANDARD
                    .decode(image.rgba)
                    .map_err(|_| tr("shell-invalid-historical-image-encoding"))?;
                Image::from_rgba(image.width, image.height, pixels)
            })
            .transpose()?;
        let reason = match self.reason {
            1 => CloseReason::Expired,
            2 => CloseReason::Dismissed,
            3 => CloseReason::Application,
            4 => CloseReason::Unspecified,
            _ => return Err(tr("shell-invalid-historical-close-reason").into()),
        };
        Ok(Archived {
            id,
            principal: self.principal,
            source: payload::text(&self.source, 96, false)?,
            summary: payload::text(&self.summary, 256, false)?,
            body: payload::text(&self.body, 4096, true)?,
            image,
            received_ms: self.received_ms,
            closed_ms: self.closed_ms,
            reason,
        })
    }
}
pub(crate) fn bounded(mut entries: Vec<Archived>, policy: &Policy, now_ms: u64) -> Vec<Archived> {
    if !policy.history {
        return Vec::new();
    }
    entries.retain(|entry| {
        entry.closed_ms <= now_ms
            && now_ms - entry.closed_ms < u64::from(policy.history_minutes) * 60000
    });
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.closed_ms));
    let limit = policy.history_mib as usize * 1024 * 1024;
    let mut bytes = 0usize;
    entries
        .into_iter()
        .filter(|entry| {
            let size = entry.bytes();
            if bytes + size > limit {
                false
            } else {
                bytes += size;
                true
            }
        })
        .take(MAX_HISTORY)
        .collect()
}
fn projection(snapshot: &Snapshot, policy: &Policy, now_ms: u64) -> Vec<Archived> {
    let mut entries = snapshot.history.clone();
    for notification in &snapshot.active {
        if notification.draft.transient {
            continue;
        }
        entries.push(Archived {
            id: 0,
            principal: notification.origin.principal.clone(),
            source: notification.origin.label.clone(),
            summary: notification.draft.summary.clone(),
            body: notification.draft.body.clone(),
            image: notification.draft.image.clone(),
            received_ms: notification.received_ms,
            closed_ms: notification.updated_ms,
            reason: CloseReason::Unspecified,
        });
    }
    bounded(entries, policy, now_ms)
}
fn decode(bytes: &[u8], generation: u64) -> Result<Vec<Archived>, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(tr("shell-notification-history-exceeds-its-file-limit").into());
    }
    let document: Document = serde_json::from_slice(bytes).map_err(|_| {
        tr("shell-notification-history-is-invalid-clear-it-explicitly-to-replace-it")
    })?;
    if document.version != 1 || document.generation != generation {
        return Err(tr("shell-notification-history-version-or-generation-changed").into());
    }
    document
        .records
        .into_iter()
        .enumerate()
        .map(|(index, record)| record.into_entry(index as u64 + 1))
        .collect()
}
#[derive(Clone, Debug)]
pub(crate) struct Storage {
    path: PathBuf,
}
impl Storage {
    pub(crate) fn for_settings(settings: &Path) -> Result<Self, String> {
        let mut filename = settings
            .file_name()
            .ok_or_else(|| tr("shell-invalid-notification-history-location"))?
            .to_owned();
        filename.push(".notifications");
        Ok(Self {
            path: settings.with_file_name(filename),
        })
    }
    fn directory(&self) -> Result<&Path, String> {
        let path = self
            .path
            .parent()
            .ok_or_else(|| tr("shell-invalid-notification-history-location"))?;
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| tr("shell-cannot-inspect-notification-history-directory"))?;
        if !metadata.is_dir()
            || metadata.uid()
                != fs::metadata("/proc/self")
                    .map_err(|_| "Cannot identify notification history owner")?
                    .uid()
            || metadata.mode() & 0o077 != 0
        {
            return Err(tr("shell-notification-history-requires-a-private-owned-directory").into());
        }
        Ok(path)
    }
    fn exists(&self) -> Result<bool, String> {
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(tr("shell-cannot-inspect-notification-history").into()),
            Ok(metadata)
                if metadata.is_file()
                    && metadata.nlink() == 1
                    && metadata.uid()
                        == fs::metadata("/proc/self")
                            .map_err(|_| "Cannot identify notification history owner")?
                            .uid()
                    && metadata.mode() & 0o077 == 0 =>
            {
                Ok(true)
            }
            _ => Err(tr("shell-notification-history-must-be-a-private-owned-regular-file").into()),
        }
    }
    pub(crate) fn erase(&self) -> Result<(), String> {
        if self.exists()? {
            self.directory()?;
            fs::remove_file(&self.path)
                .map_err(|_| tr("shell-cannot-remove-notification-history"))?;
        }
        Ok(())
    }
    pub(crate) fn load(&self, policy: &Policy, now_ms: u64) -> Result<Vec<Archived>, String> {
        policy.validate()?;
        if !policy.history {
            self.erase()?;
            return Ok(Vec::new());
        }
        self.directory()?;
        if !self.exists()? {
            return Ok(Vec::new());
        }
        let bytes = crate::clipboard::session::private_bytes(&self.path, MAX_FILE_BYTES)
            .map_err(|_| tr("shell-cannot-read-private-notification-history"))?;
        Ok(bounded(
            decode(&bytes, policy.history_generation)?,
            policy,
            now_ms,
        ))
    }
    pub(crate) fn save(
        &self,
        policy: &Policy,
        snapshot: &Snapshot,
        now_ms: u64,
    ) -> Result<(), String> {
        policy.validate()?;
        if !policy.history {
            return self.erase();
        }
        let directory = self.directory()?;
        let old = if self.exists()? {
            let bytes = crate::clipboard::session::private_bytes(&self.path, MAX_FILE_BYTES)
                .map_err(|_| tr("shell-cannot-read-private-notification-history"))?;
            decode(&bytes, policy.history_generation)?;
            Some(bytes)
        } else {
            None
        };
        let document = Document {
            version: 1,
            generation: policy.history_generation,
            records: projection(snapshot, policy, now_ms)
                .iter()
                .map(Record::from_entry)
                .collect(),
        };
        let bytes = serde_json::to_vec(&document)
            .map_err(|_| tr("shell-cannot-encode-notification-history"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(tr("shell-notification-history-exceeds-its-file-limit").into());
        }
        if old.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(directory)
            .map_err(|_| tr("shell-cannot-prepare-private-notification-history"))?;
        temporary
            .write_all(&bytes)
            .map_err(|_| tr("shell-cannot-write-notification-history"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| tr("shell-cannot-sync-notification-history"))?;
        temporary
            .persist(&self.path)
            .map_err(|_| tr("shell-cannot-replace-notification-history"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }
    fn entry() -> Archived {
        Archived {
            id: 1,
            principal: format!("exe:{:064x}", 1),
            source: "Fixture".into(),
            summary: "Fixture summary".into(),
            body: "Exact body\n👩\u{200d}💻".into(),
            image: Some(Image::from_rgba(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap()),
            received_ms: 1,
            closed_ms: 2,
            reason: CloseReason::Dismissed,
        }
    }
    fn policy() -> Policy {
        Policy {
            history: true,
            history_generation: 1,
            ..Policy::default()
        }
    }
    #[test]
    fn archive_behaviour() {
        // Archive roundtrip has no sender or action authority.
        {
            let temp = directory();
            let storage = Storage::for_settings(&temp.path().join("settings.json")).unwrap();
            let mut snapshot = Snapshot::empty(3, policy());
            snapshot.history.push(entry());
            storage.save(&policy(), &snapshot, 3).unwrap();
            let loaded = storage.load(&policy(), 3).unwrap();
            assert_eq!(loaded[0].body, entry().body);
            assert_eq!(
                loaded[0].image.as_ref().unwrap().pixels().0,
                entry().image.unwrap().pixels().0
            );
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(&storage.path).unwrap()).unwrap();
            assert!(value["records"][0].get("sender").is_none());
            assert!(value["records"][0].get("actions").is_none());
            assert_eq!(fs::metadata(&storage.path).unwrap().mode() & 0o077, 0);
        }
        // Symlinks are not followed or deleted.
        {
            let temp = directory();
            let storage = Storage::for_settings(&temp.path().join("settings.json")).unwrap();
            let fixture = temp.path().join("fixture");
            fs::write(&fixture, b"preserve").unwrap();
            std::os::unix::fs::symlink(&fixture, &storage.path).unwrap();
            assert!(storage.load(&policy(), 3).is_err());
            assert!(storage.erase().is_err());
            assert!(
                storage
                    .save(&policy(), &Snapshot::empty(1, policy()), 3)
                    .is_err()
            );
            assert_eq!(fs::read(fixture).unwrap(), b"preserve");
        }
    }
}
