use crate::clipboard::session::{self, Target, private_bytes};
use enzui_shell::i18n::tr;
use enzui_theme::EnzTheme;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    /// Playback percentage: 100 preserves duration; higher values are faster.
    pub rate: u16,
    pub windows: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            rate: 150,
            windows: true,
        }
    }
}
impl Policy {
    pub fn validate(self) -> Result<(), String> {
        if !(25..=200).contains(&self.rate) {
            return Err(tr("shell-animation-speed-must-be-between-25-and-200-percent").into());
        }
        Ok(())
    }
    pub fn lua(self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("{{rate={},windows={}}}", self.rate, self.windows))
    }
}

/// Apply once to fresh theme tokens; easing and reduced-motion state are retained.
pub fn scale(theme: &mut EnzTheme, rate: u16) {
    let factor = duration_factor(rate);
    theme.motion.fast_ms *= factor;
    theme.motion.normal_ms *= factor;
    theme.motion.slow_ms *= factor;
}
/// Replaces Search's motion tiers; call after general theme scaling.
pub fn search_timing(theme: &mut EnzTheme, rate: u16, enabled: bool) {
    let factor = if enabled { duration_factor(rate) } else { 0.0 };
    theme.motion.fast_ms = 100.0 * factor;
    theme.motion.normal_ms = 140.0 * factor;
    theme.motion.slow_ms = 160.0 * factor;
}
pub fn style_label(style: u8) -> &'static str {
    match style {
        1 => tr("shell-spring"),
        2 => tr("shell-glide"),
        3 => tr("shell-bloom"),
        _ => tr("shell-standard"),
    }
}
pub fn expressive(style: u8) -> Option<enzui_theme::ExpressiveMotion> {
    use enzui_theme::ExpressiveMotion;
    match style {
        1 => Some(ExpressiveMotion::FastSpatial),
        2 => Some(ExpressiveMotion::DefaultSpatial),
        3 => Some(ExpressiveMotion::SlowSpatial),
        _ => None,
    }
}
pub fn duration_factor(rate: u16) -> f32 {
    100.0 / f32::from(rate.clamp(25, 200))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Owner {
    runtime: PathBuf,
    pid: u32,
    start: u64,
    device: u64,
    inode: u64,
}
impl Owner {
    /// Captures the current process only in an explicitly selected managed runtime.
    pub fn current() -> Option<Self> {
        let target = Target::from_environment().ok()?;
        drop(target.connect().ok()?);
        let pid = std::process::id();
        let meta = fs::metadata(format!("/proc/{pid}/exe")).ok()?;
        Some(Self {
            runtime: target.runtime().into(),
            pid,
            start: session::start(pid).ok()?,
            device: meta.dev(),
            inode: meta.ino(),
        })
    }
    fn matches_ready(&self, runtime: &Path, ready: &Value) -> bool {
        self.runtime == runtime
            && ready["shell_pid"].as_u64() == Some(self.pid.into())
            && ready["shell_start"].as_u64() == Some(self.start)
    }
    fn live(&self, unit: &str) -> Option<()> {
        session::membership(self.pid, unit).ok()?;
        if session::start(self.pid).ok()? != self.start
            || fs::metadata(format!("/proc/{}", self.pid)).ok()?.uid()
                != fs::metadata("/proc/self").ok()?.uid()
        {
            return None;
        }
        let meta = fs::metadata(format!("/proc/{}/exe", self.pid)).ok()?;
        ((meta.dev(), meta.ino()) == (self.device, self.inode)).then_some(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub value: Option<Value>,
    pub available: bool,
}
fn supported(value: &Value, settings: &Path) -> Option<()> {
    if value["motion_version"] != 1 {
        return None;
    }
    let owner: Owner = serde_json::from_value(value["owner"].clone()).ok()?;
    let target = Target::from_environment().ok()?;
    drop(target.connect_observer().ok()?);
    let config: Value =
        serde_json::from_slice(&private_bytes(&target.runtime().join("launch.json"), 32768).ok()?)
            .ok()?;
    if config["settings"].as_str().map(Path::new) != Some(settings) {
        return None;
    }
    let ready: Value =
        serde_json::from_slice(&private_bytes(&target.runtime().join("ready.json"), 32768).ok()?)
            .ok()?;
    if !owner.matches_ready(target.runtime(), &ready) {
        return None;
    }
    owner.live(config["unit"].as_str()?)
}

/// A saved receipt is insufficient: availability also requires the live selected shell.
pub fn read_status(settings: &Path) -> Status {
    let value = private_bytes(&settings.with_file_name("compositor-status.json"), 8192)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let available = value
        .as_ref()
        .is_some_and(|value| supported(value, settings).is_some());
    Status { value, available }
}
