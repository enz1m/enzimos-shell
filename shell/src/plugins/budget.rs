use enzui_shell::i18n::tr;
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const MAX_WORKERS: usize = 8;
pub const WORKER_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone)]
pub struct BudgetPool(Arc<Pool>);
struct Pool {
    path: PathBuf,
    active: AtomicUsize,
    retired: Mutex<Vec<PathBuf>>,
}
pub struct BudgetSlot {
    path: PathBuf,
    _permit: Permit,
}
#[derive(Clone, Copy, Debug)]
pub struct ResourceStats {
    pub memory_peak_bytes: u64,
    pub oom_kills: u64,
    pub process_limit_hits: u64,
}
struct Permit {
    pool: Arc<Pool>,
    retired: Option<PathBuf>,
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(path) = self.retired.take() {
            self.pool
                .retired
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(path);
        } else {
            self.pool.active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
fn unavailable(_: impl std::fmt::Display) -> String {
    tr("shell-delegated-plugin-resource-isolation-is-unavailable").into()
}
fn write(path: &Path, name: &str, value: &str) -> Result<(), String> {
    fs::write(path.join(name), value).map_err(unavailable)
}
fn token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(unavailable)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn start_ticks(stat: &str) -> Result<u64, String> {
    stat.rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| tr("shell-cannot-identify-plugin-supervisor-process").into())
}
fn unified_path(contents: &str) -> Result<&Path, String> {
    let mut paths = contents.lines().filter_map(|line| line.strip_prefix("0::"));
    let path = paths
        .next()
        .ok_or_else(|| tr("shell-a-unified-resource-hierarchy-is-required"))?;
    if paths.next().is_some()
        || !path.starts_with('/')
        || path.split('/').any(|c| c == ".." || c == ".")
    {
        return Err(tr("shell-invalid-plugin-resource-hierarchy").into());
    }
    Ok(Path::new(path))
}
impl BudgetPool {
    pub fn from_environment() -> Result<Self, String> {
        if std::env::var("ENZUI_PLUGIN_ISOLATION").as_deref() != Ok("delegated") {
            return Err(tr(
                "shell-plugin-execution-requires-an-explicitly-delegated-desktop-session",
            )
            .into());
        }
        let membership = fs::read_to_string("/proc/self/cgroup").map_err(unavailable)?;
        let current = Path::new("/sys/fs/cgroup").join(
            unified_path(&membership)?
                .strip_prefix("/")
                .map_err(unavailable)?,
        );
        if current.file_name().and_then(|v| v.to_str()) != Some("supervisor") {
            return Err(tr("shell-plugin-supervisor-is-not-in-its-delegated-leaf").into());
        }
        let root = current
            .parent()
            .ok_or_else(|| tr("shell-missing-delegated-plugin-root"))?;
        let uid = fs::metadata("/proc/self").map_err(unavailable)?.uid();
        if fs::metadata(root).map_err(unavailable)?.uid() != uid
            || !root.join("cgroup.kill").is_file()
            || !fs::read_to_string(root.join("cgroup.procs"))
                .map_err(unavailable)?
                .trim()
                .is_empty()
        {
            return Err(tr("shell-plugin-root-is-not-privately-delegated").into());
        }
        let controllers =
            fs::read_to_string(root.join("cgroup.controllers")).map_err(unavailable)?;
        if ["cpu", "memory", "pids"]
            .iter()
            .any(|wanted| !controllers.split_whitespace().any(|c| c == *wanted))
        {
            return Err(tr("shell-plugin-cpu-memory-and-process-limits-are-required").into());
        }
        write(root, "cgroup.subtree_control", "+cpu +memory +pids")?;
        let ticks = start_ticks(&fs::read_to_string("/proc/self/stat").map_err(unavailable)?)?;
        let path = root.join(format!(
            "plugins-{}-{ticks}-{}",
            std::process::id(),
            token()?
        ));
        fs::create_dir(&path).map_err(unavailable)?;
        let pool = Self(Arc::new(Pool {
            path,
            active: AtomicUsize::new(0),
            retired: Mutex::new(Vec::new()),
        }));
        write(
            &pool.0.path,
            "memory.max",
            &((MAX_WORKERS as u64 + 1) * WORKER_MEMORY_BYTES).to_string(),
        )?;
        write(&pool.0.path, "memory.swap.max", "0")?;
        write(&pool.0.path, "pids.max", "128")?;
        write(&pool.0.path, "cpu.max", "50000 100000")?;
        write(&pool.0.path, "cgroup.subtree_control", "+cpu +memory +pids")?;
        Ok(pool)
    }
    pub fn reap(&self) -> usize {
        let mut retired = self
            .0
            .retired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = retired.len();
        retired.retain(|path| match fs::remove_dir(path) {
            Ok(()) => false,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        });
        let removed = before - retired.len();
        self.0.active.fetch_sub(removed, Ordering::AcqRel);
        removed
    }
    pub fn slot(&self) -> Result<BudgetSlot, String> {
        self.reap();
        self.0
            .active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_WORKERS).then_some(n + 1)
            })
            .map_err(|_| tr("shell-too-many-active-plugins"))?;
        let permit = Permit {
            pool: std::sync::Arc::clone(&self.0),
            retired: None,
        };
        let path = self.0.path.join(format!("w-{}", token()?));
        fs::create_dir(&path).map_err(unavailable)?;
        let slot = BudgetSlot {
            path,
            _permit: permit,
        };
        write(&slot.path, "memory.max", &WORKER_MEMORY_BYTES.to_string())?;
        write(&slot.path, "memory.swap.max", "0")?;
        write(&slot.path, "memory.oom.group", "1")?;
        write(&slot.path, "pids.max", "16")?;
        write(&slot.path, "cpu.max", "15000 100000")?;
        Ok(slot)
    }
}
impl BudgetSlot {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn terminate(&self) -> Result<(), String> {
        write(&self.path, "cgroup.kill", "1")
    }
    pub fn stats(&self) -> Result<ResourceStats, String> {
        let read = |name| fs::read_to_string(self.path.join(name)).map_err(unavailable);
        let event = |text: String, key: &str| {
            text.lines()
                .find_map(|line| {
                    line.split_once(' ')
                        .filter(|(k, _)| *k == key)
                        .and_then(|(_, v)| v.parse().ok())
                })
                .ok_or_else(|| tr("shell-plugin-resource-counters-are-unavailable").to_owned())
        };
        Ok(ResourceStats {
            memory_peak_bytes: read("memory.peak")?.trim().parse().map_err(unavailable)?,
            oom_kills: event(read("memory.events")?, "oom_kill")?,
            process_limit_hits: event(read("pids.events")?, "max")?,
        })
    }
    pub fn empty(&self) -> Result<bool, String> {
        Ok(fs::read_to_string(self.path.join("cgroup.events"))
            .map_err(unavailable)?
            .lines()
            .any(|l| l == "populated 0"))
    }
}
impl Drop for BudgetSlot {
    fn drop(&mut self) {
        let _ = self.terminate();
        if let Err(error) = fs::remove_dir(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            self._permit.retired = Some(self.path.clone());
        }
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        let _ = write(&self.path, "cgroup.kill", "1");
        if let Ok(children) = fs::read_dir(&self.path) {
            for child in children.flatten() {
                if child
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("w-"))
                {
                    let _ = fs::remove_dir(child.path());
                }
            }
        }
        let _ = fs::remove_dir(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_behaviour() {
        // Membership parsing selects the unified entry without ancestor escapes.
        {
            assert_eq!(
                unified_path("1:name=systemd:/\n0::/user.slice/owned.service/supervisor\n")
                    .unwrap(),
                Path::new("/user.slice/owned.service/supervisor")
            );
            for invalid in [
                "1:name=systemd:/",
                "0::relative",
                "0::/../other",
                "0::/owned/./supervisor",
                "0::/one\n0::/two",
            ] {
                assert!(unified_path(invalid).is_err());
            }
        }
        // Retired groups hold admission until the directory can be removed.
        {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("w-owned");
            fs::create_dir(&path).unwrap();
            fs::write(path.join("busy"), b"fixture").unwrap();
            let pool = BudgetPool(Arc::new(Pool {
                path: temp.path().to_owned(),
                active: AtomicUsize::new(1),
                retired: Mutex::new(Vec::new()),
            }));
            drop(Permit {
                pool: std::sync::Arc::clone(&pool.0),
                retired: Some(path.clone()),
            });
            assert_eq!(pool.reap(), 0);
            assert_eq!(pool.0.active.load(Ordering::Acquire), 1);
            fs::remove_file(path.join("busy")).unwrap();
            assert_eq!(pool.reap(), 1);
            assert_eq!(pool.0.active.load(Ordering::Acquire), 0);
            assert_eq!(pool.reap(), 0);
            assert!(pool.0.retired.lock().unwrap().is_empty());
        }
        // Process identity ignores parentheses in the comm field.
        {
            let tail = (0..25).map(|i| i.to_string()).collect::<Vec<_>>().join(" ");
            assert_eq!(
                start_ticks(&format!("1 (name ) with parens) {tail}")).unwrap(),
                19
            );
            assert!(start_ticks("invalid").is_err());
        }
    }
}
