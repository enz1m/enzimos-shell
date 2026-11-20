use super::{
    Category, Item, Target,
    catalogue::{FileIdentity, display_text},
};
use enzui_shell::i18n::tr;
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat, openat2};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_FILES: usize = 20_000;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub items: Arc<Vec<Item>>,
    pub scanned: usize,
    pub truncated: bool,
    pub errors: usize,
}
pub fn build(roots: &[String], exclusions: &[String], cancel: &AtomicBool) -> Index {
    let mut result = Index::default();
    let mut items = Vec::new();
    let mut pending = VecDeque::new();
    let exclusions: Vec<PathBuf> = exclusions.iter().take(8).map(PathBuf::from).collect();
    for root in roots.iter().take(8) {
        let path = PathBuf::from(root);
        if !path.is_absolute()
            || path == std::path::Path::new("/")
            || exclusions.iter().any(|excluded| path.starts_with(excluded))
        {
            continue;
        }
        let opened = openat2(
            CWD,
            &path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map(fs::File::from);
        match opened.and_then(|file| {
            file.metadata()
                .map(|metadata| (file, metadata))
                .map_err(|_| rustix::io::Errno::IO)
        }) {
            Ok((file, metadata)) if metadata.is_dir() => {
                pending.push_back((path, file, 0_u8, metadata.dev()));
            }
            _ => result.errors += 1,
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bytes = 0;
    while let Some((directory, parent, depth, device)) = pending.pop_front() {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        if Instant::now() >= deadline || result.scanned >= MAX_FILES || bytes >= MAX_BYTES {
            result.truncated = true;
            break;
        }
        let Ok(entries) = fs::read_dir(format!("/proc/self/fd/{}", parent.as_raw_fd())) else {
            result.errors += 1;
            continue;
        };
        for entry in entries {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            if Instant::now() >= deadline || result.scanned >= MAX_FILES || bytes >= MAX_BYTES {
                result.truncated = true;
                break;
            }
            result.scanned += 1;
            let Ok(entry) = entry else {
                result.errors += 1;
                continue;
            };
            let name = entry.file_name();
            let path = directory.join(&name);
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.')
                || exclusions.iter().any(|excluded| path.starts_with(excluded))
                || path.as_os_str().len() > 2048
            {
                continue;
            }
            let opened = match openat(
                &parent,
                name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => fs::File::from(fd),
                Err(_) => {
                    result.errors += 1;
                    continue;
                }
            };
            let Ok(metadata) = opened.metadata() else {
                result.errors += 1;
                continue;
            };
            if metadata.file_type().is_symlink()
                || metadata.dev() != device
                || (!metadata.is_file() && !metadata.is_dir())
            {
                continue;
            }
            if metadata.is_dir() {
                if depth < 8 && pending.len() < 128 {
                    pending.push_back((path.clone(), opened, depth + 1, device));
                } else {
                    result.truncated = true;
                }
            }
            if metadata.is_file()
                && (metadata.mode() & 0o111 != 0
                    || path.extension().is_some_and(|ext| ext == "desktop"))
            {
                continue;
            }
            let identity = FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                size: metadata.size(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
            };
            let id = format!("file:{:x}", Sha256::digest(path.as_os_str().as_bytes()));
            let title = display_text(name, 256);
            let location = display_text(&directory.to_string_lossy(), 512);
            let cost = path.as_os_str().len() * 2 + (title.len() + location.len()) * 4 + 512;
            if cost > MAX_BYTES.saturating_sub(bytes) {
                result.truncated = true;
                break;
            }
            bytes += cost;
            items.push(Item::new(
                id,
                Category::Files,
                title,
                location,
                tr("shell-open"),
                if metadata.is_dir() { "folder" } else { "file" },
                Some(Target::File {
                    path,
                    identity,
                    directory: metadata.is_dir(),
                }),
            ));
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    items.dedup_by(|a, b| a.id == b.id);
    result.items = Arc::new(items);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexing_is_opt_in_bounded_and_does_not_follow_hidden_symlink_or_excluded_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("visible.txt"), "disposable").unwrap();
        fs::write(root.path().join(".secret"), "not indexed").unwrap();
        fs::create_dir(root.path().join("excluded")).unwrap();
        fs::write(root.path().join("excluded/private.txt"), "not indexed").unwrap();
        std::os::unix::fs::symlink("/", root.path().join("link")).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(build(&[], &[], &cancel).items.is_empty());
        let indexed = build(
            &[root.path().to_string_lossy().into()],
            &[root.path().join("excluded").to_string_lossy().into()],
            &cancel,
        );
        assert_eq!(indexed.items.len(), 1);
        assert_eq!(indexed.items[0].title, "visible.txt");
        cancel.store(true, Ordering::Release);
        assert!(
            build(&[root.path().to_string_lossy().into()], &[], &cancel)
                .items
                .is_empty()
        );
    }
}
