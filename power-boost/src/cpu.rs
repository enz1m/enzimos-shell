//! cpufreq policies that expose an energy-performance preference (EPP).

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// The kernel's cpufreq policy directory.
pub const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";

/// The preference a boost writes.
pub const PERFORMANCE: &str = "performance";
/// The strongest power-saving preference; a manager that chose it is respected.
pub const POWER: &str = "power";

const PREFERENCE: &str = "energy_performance_preference";
const AVAILABLE: &str = "energy_performance_available_preferences";
const GOVERNOR: &str = "scaling_governor";

/// One `policyN` directory with a writable energy-performance preference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    dir: PathBuf,
}

impl Policy {
    /// The policy named `name` (such as `policy0`) below `root`, if the name is one.
    #[must_use]
    pub fn named(root: &Path, name: &str) -> Option<Self> {
        let number = name.strip_prefix("policy")?;
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then(|| Self {
            dir: root.join(name),
        })
    }

    /// The directory name, such as `policy0`.
    #[must_use]
    pub fn name(&self) -> &str {
        self.dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }

    /// The current preference, as the kernel reports it.
    ///
    /// # Errors
    ///
    /// Fails when the attribute cannot be read.
    pub fn preference(&self) -> io::Result<String> {
        read(&self.dir.join(PREFERENCE))
    }

    /// Writes a preference the kernel accepts, such as `performance` or a value read earlier.
    ///
    /// # Errors
    ///
    /// Fails when the attribute is missing or the driver refuses the value
    /// (for example while the governor is `performance`).
    pub fn set_preference(&self, value: &str) -> io::Result<()> {
        // A missing attribute is an error, never a new file.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.dir.join(PREFERENCE))?;
        file.write_all(value.as_bytes())
    }

    /// The scaling governor, if the driver reports one.
    #[must_use]
    pub fn governor(&self) -> Option<String> {
        read(&self.dir.join(GOVERNOR)).ok()
    }

    /// Whether `performance` is among the preferences the driver offers.
    #[must_use]
    pub fn offers_performance(&self) -> bool {
        read(&self.dir.join(AVAILABLE))
            .is_ok_and(|list| list.split_whitespace().any(|p| p == PERFORMANCE))
    }
}

/// The policies below `root` that have a preference attribute, in name order.
///
/// Drivers without EPP (acpi-cpufreq, intel_pstate without HWP, amd-pstate in
/// passive or guided mode) yield none.
#[must_use]
pub fn policies(root: &Path) -> Vec<Policy> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(u32, Policy)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let policy = Policy::named(root, name.to_str()?)?;
            let number = policy.name().strip_prefix("policy")?.parse().ok()?;
            // A policy whose CPUs are all offline keeps its attribute, but
            // reading it fails; skipping it keeps the online ones boostable.
            let online =
                read(&policy.dir.join("affected_cpus")).map_or(true, |cpus| !cpus.is_empty());
            (online && policy.dir.join(PREFERENCE).is_file()).then_some((number, policy))
        })
        .collect();
    found.sort_by_key(|(number, _)| *number);
    found.into_iter().map(|(_, policy)| policy).collect()
}

fn read(path: &Path) -> io::Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_owned())
}

#[cfg(test)]
pub(crate) mod fake {
    use std::path::Path;

    /// Adds `policyN` with the given preference, governor and offered preferences.
    pub(crate) fn policy(root: &Path, number: u32, preference: &str, governor: &str) {
        let dir = root.join(format!("policy{number}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(super::PREFERENCE), format!("{preference}\n")).unwrap();
        std::fs::write(dir.join(super::GOVERNOR), format!("{governor}\n")).unwrap();
        std::fs::write(
            dir.join(super::AVAILABLE),
            "default performance balance_performance balance_power power\n",
        )
        .unwrap();
    }

    pub(crate) fn preference(root: &Path, number: u32) -> String {
        std::fs::read_to_string(root.join(format!("policy{number}/{}", super::PREFERENCE)))
            .unwrap()
            .trim()
            .to_owned()
    }

    pub(crate) fn set(root: &Path, number: u32, attribute: &str, value: &str) {
        std::fs::write(root.join(format!("policy{number}/{attribute}")), value).unwrap();
    }
}
