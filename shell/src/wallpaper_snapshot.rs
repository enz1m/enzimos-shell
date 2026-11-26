//! Still snapshots of live desktop wallpapers.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Locates the wallpaper renderer shipped alongside the shell.
pub fn binary() -> Option<PathBuf> {
    std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(|dir| PathBuf::from(dir).join("enzwall"))
        .filter(|path| path.is_absolute() && path.is_file())
}

/// Renders a scene with the current clock and date into a caller-owned PNG path.
/// Blocks until the renderer exits or the two-second timeout elapses.
pub fn render(
    scene: &Path,
    out: &Path,
    size: (u32, u32),
    hide_clock: bool,
    hide_date: bool,
) -> bool {
    let Some(binary) = binary() else {
        return false;
    };
    let (width, height) = size;
    let mut command = Command::new(binary);
    command
        .arg("snapshot")
        .arg(scene)
        .arg(out)
        .arg("--size")
        .arg(format!("{width}x{height}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if hide_clock {
        command.arg("--hide-clock");
    }
    if hide_date {
        command.arg("--hide-date");
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
