//! Installed component paths from installed.json; checkouts use local target and build
//! directories.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The prepared compositor: `hyprland-current-build/`, `prefix/lib`, plugins.
    pub compositor: PathBuf,
    /// Shell, Settings, lock, greeter and helper executables.
    pub bin: PathBuf,
    /// Prepared enzUI applications, one executable per app id.
    pub apps: PathBuf,
}

/// The installed layout under `root`, or `None` for a development checkout.
pub fn layout(root: &Path) -> Option<Layout> {
    let text = std::fs::read_to_string(root.join("installed.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let path = |key: &str| {
        value[key]
            .as_str()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    Some(Layout {
        compositor: path("compositor")?,
        bin: path("bin")?,
        apps: path("apps")?,
    })
}
