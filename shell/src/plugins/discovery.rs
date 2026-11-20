use super::manifest::{MAX_PACKAGE_BYTES, Package};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct Candidate {
    pub app_id: String,
    pub package: Arc<Package>,
}

fn installed_app_id(name: &str) -> bool {
    name.len() <= 128
        && name.len() >= 3
        && name.contains('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn read_package(path: &Path) -> Option<Arc<Package>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((MAX_PACKAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return None;
    }
    Package::parse(&bytes).ok().map(Arc::new)
}

pub fn installed_at(roots: &[PathBuf]) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        let Ok(root) = root.canonicalize() else {
            continue;
        };
        let app_root = root.join("app");
        let Ok(entries) = fs::read_dir(&app_root) else {
            continue;
        };
        let mut apps: Vec<_> = entries.take(129).filter_map(Result::ok).collect();
        if apps.len() > 128 {
            continue;
        }
        apps.sort_by_key(std::fs::DirEntry::file_name);
        for app in apps {
            let Some(id) = app.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !installed_app_id(&id) || !app.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let base = app_root.join(&id);
            let dir = base.join("current/active/files/share/enzui-shell/plugins");
            let Ok(dir) = dir.canonicalize() else {
                continue;
            };
            if !dir.starts_with(&base) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = entries.take(17).filter_map(Result::ok).collect();
            if entries.len() > 16 {
                continue;
            }
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                if found.len() >= 64 {
                    return found;
                }
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("enzplugin")
                    || !entry.file_type().is_ok_and(|kind| kind.is_file())
                {
                    continue;
                }
                let Some(package) = read_package(&path) else {
                    continue;
                };
                if seen.insert((id.clone(), package.manifest().id.clone())) {
                    found.push(Candidate {
                        app_id: id.clone(),
                        package,
                    });
                }
            }
        }
    }
    found
}

pub fn installed() -> Vec<Candidate> {
    let user_data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let mut roots: Vec<PathBuf> = user_data
        .into_iter()
        .map(|path| path.join("flatpak"))
        .collect();
    roots.push(PathBuf::from("/var/lib/flatpak"));
    installed_at(&roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::store::Store;
    use serde_json::json;

    #[test]
    fn only_installed_flatpak_deployments_and_regular_bounded_packages_are_discovered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("flatpak");
        let base = root.join("app/dev.enzui.CpuMeter");
        let plugin_dir = base.join("x86_64/stable/commit/files/share/enzui-shell/plugins");
        fs::create_dir_all(&plugin_dir).unwrap();
        std::os::unix::fs::symlink("x86_64/stable", base.join("current")).unwrap();
        std::os::unix::fs::symlink("commit", base.join("x86_64/stable/active")).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../examples/plugins/cpu/manifest.json"))
                .unwrap();
        let mut manifest = manifest;
        manifest["program"] = json!(include_str!("../../../examples/plugins/cpu/main.py"));
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let path = plugin_dir.join("cpu.enzplugin");
        fs::write(&path, bytes).unwrap();
        fs::hard_link(&path, plugin_dir.join("copy.enzplugin")).unwrap();
        std::os::unix::fs::symlink(&path, plugin_dir.join("symlink.enzplugin")).unwrap();
        let found = installed_at(std::slice::from_ref(&root));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].app_id, "dev.enzui.CpuMeter");
        assert_eq!(found[0].package.manifest().id, "cpu-meter");
        assert!(Store::read_package(&path).is_err());
        let evil = root.join("app/dev.enzui.Evil");
        fs::create_dir_all(&evil).unwrap();
        std::os::unix::fs::symlink(base.join("x86_64/stable"), evil.join("current")).unwrap();
        assert_eq!(installed_at(&[root]).len(), 1);
    }
}
