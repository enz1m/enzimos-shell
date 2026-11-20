//! Bounded wf-recorder child using capture preferences; stops on request or low disk space.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Recording stops when less than this is free where it is written.
pub const MIN_FREE_BYTES: u64 = 1 << 30;

pub struct Recording {
    child: std::process::Child,
    pub path: PathBuf,
    pub started: Instant,
    // Dropped after the encoder, which records from it.
    mix: Option<Mix>,
}

// The PulseAudio name of the sink desktop sound and the microphone are
// mixed into while recording both.
const MIX: &str = "enzui-recording-mix";

// How long one `pactl` call may take: a stuck sound server must not hold
// the recording's start or end.
const PACTL: Duration = Duration::from_secs(2);

// `tool` with `args`, its trimmed output if it succeeds within `PACTL`.
fn pactl(tool: &Path, args: &[&str]) -> Option<String> {
    use std::io::Read;
    let mut child = std::process::Command::new(tool)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PACTL;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut output = String::new();
                child
                    .stdout
                    .take()?
                    .take(65536)
                    .read_to_string(&mut output)
                    .ok()?;
                return status.success().then(|| output.trim().to_owned());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A recording that started on a worker thread, handed to the shell once.
#[derive(Clone)]
pub struct Started(std::sync::Arc<std::sync::Mutex<Option<Recording>>>);

impl Started {
    #[must_use]
    pub fn take(&self) -> Option<Recording> {
        self.0.lock().ok()?.take()
    }
}

impl std::fmt::Debug for Started {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Started")
    }
}

/// Starts recording on a blocking worker: preparing the sound mix runs
/// `pactl`, which a stuck sound server can hold for seconds.
pub async fn start_in_background(
    tools: crate::system_controls::SystemControls,
    target: Target,
    preferences: crate::capture::Preferences,
) -> Result<Started, String> {
    tokio::task::spawn_blocking(move || {
        start(&tools, &target, &preferences)
            .map(|recording| Started(std::sync::Arc::new(std::sync::Mutex::new(Some(recording)))))
    })
    .await
    .unwrap_or_else(|_| Err(tr("shell-recording-could-not-start").into()))
}

// Per-recording PipeWire audio mix, unloaded on drop; stale shell-owned mixes are cleared
// before reuse.
struct Mix {
    tool: PathBuf,
    modules: Vec<String>,
}

impl Mix {
    fn create(tools: &crate::system_controls::SystemControls) -> Result<Self, String> {
        let tool = tools
            .tool("pactl")
            .ok_or_else(|| tr("shell-recording-desktop-sound-and-the-microphone-needs-pactl"))?;
        let run = |args: &[&str]| pactl(&tool, args);
        // Stale modules of this sink, from a recording that never finished.
        for line in run(&["list", "short", "modules"])
            .unwrap_or_default()
            .lines()
        {
            let mut fields = line.split('\t');
            if let (Some(id), Some(_), Some(arguments)) =
                (fields.next(), fields.next(), fields.next())
                && arguments.contains(MIX)
            {
                let _ = run(&["unload-module", id]);
            }
        }
        // Named before the sink exists, so it can never be its own source.
        let sink = run(&["get-default-sink"])
            .filter(|name| !name.is_empty() && name != MIX)
            .ok_or_else(|| tr("shell-there-is-no-sound-output-to-record"))?;
        let source = run(&["get-default-source"])
            .filter(|name| !name.is_empty())
            .ok_or_else(|| tr("shell-there-is-no-microphone-to-record"))?;
        let mut mix = Self {
            tool: tool.clone(),
            modules: Vec::new(),
        };
        for args in [
            vec![
                "load-module".to_owned(),
                "module-null-sink".into(),
                format!("sink_name={MIX}"),
                "sink_properties=device.description='Screen recording'".into(),
            ],
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={sink}.monitor"),
                format!("sink={MIX}"),
                "latency_msec=40".into(),
            ],
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={source}"),
                format!("sink={MIX}"),
                "latency_msec=40".into(),
            ],
        ] {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let id = run(&args)
                .filter(|id| id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty())
                .ok_or_else(|| tr("shell-the-sound-could-not-be-prepared-for-recording"))?;
            mix.modules.push(id);
        }
        Ok(mix)
    }
}

impl Drop for Mix {
    fn drop(&mut self) {
        for id in self.modules.iter().rev() {
            let _ = pactl(&self.tool, &["unload-module", id]);
        }
    }
}

impl std::fmt::Debug for Recording {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Where recordings go: the preview's capture folder, else Videos/Screencasts.
#[must_use]
pub fn directory() -> Option<PathBuf> {
    std::env::var_os("ENZUI_SCREENSHOT_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Videos/Screencasts"))
        })
}

#[must_use]
pub fn available(tools: &crate::system_controls::SystemControls) -> bool {
    tools.tool("wf-recorder").is_some()
}

/// What is recorded: a whole output, or an area of the desktop in global
/// logical coordinates (`x,y WxH`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Output(String),
    Area(String),
}

/// Starts recording `target` as `preferences` ask; blocks while the sound
/// mix is prepared (see [`start_in_background`]).
pub fn start(
    tools: &crate::system_controls::SystemControls,
    target: &Target,
    preferences: &crate::capture::Preferences,
) -> Result<Recording, String> {
    let text = match target {
        Target::Output(output) => output,
        Target::Area(geometry) => geometry,
    };
    if text.is_empty()
        || text.len() > 180
        || text.starts_with("ENZREMOTE-")
        || text.chars().any(char::is_control)
    {
        return Err(tr("shell-invalid-recording-output").into());
    }
    if let Target::Area(geometry) = target
        && !geometry
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ',' | ' ' | 'x' | '-'))
    {
        return Err(tr("shell-invalid-recording-area").into());
    }
    let tool = tools
        .tool("wf-recorder")
        .ok_or_else(|| tr("shell-screen-recording-needs-wf-recorder"))?;
    let folder = preferences
        .recording_dir()
        .ok_or_else(|| tr("shell-no-folder-for-recordings"))?;
    std::fs::create_dir_all(&folder)
        .map_err(|_| tr("shell-cannot-create-the-recordings-folder"))?;
    if free_bytes(&folder).is_some_and(|free| free < MIN_FREE_BYTES) {
        return Err(tr("shell-not-enough-free-disk-space-to-record").into());
    }
    let mix = if preferences.audio == crate::capture::Audio::DesktopAndMicrophone {
        Some(Mix::create(tools)?)
    } else {
        None
    };
    // Created here, so an existing file is never overwritten.
    let (path, _) =
        crate::capture::new_file(&folder, "Screencast", preferences.container.extension())?;
    if let Err(error) = crate::compositor::announce_capture(&tool) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    let child = std::process::Command::new(tool)
        .args(arguments(target, preferences))
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(child) => Ok(Recording {
            child,
            path,
            started: Instant::now(),
            mix,
        }),
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            Err(tr("shell-recording-could-not-start").into())
        }
    }
}

// `wf-recorder`'s arguments up to the file name (`-f` comes last).
fn arguments(target: &Target, preferences: &crate::capture::Preferences) -> Vec<String> {
    use crate::capture::{Audio, Codec, Container};
    let mut args: Vec<String> = vec!["-y".into()];
    match target {
        Target::Output(output) => args.extend(["-o".into(), output.clone()]),
        Target::Area(geometry) => args.extend(["-g".into(), geometry.clone()]),
    }
    let codec = preferences.codec;
    args.extend([
        "-c".into(),
        codec.encoder().into(),
        "-p".into(),
        format!("crf={}", codec.crf(preferences.quality)),
    ]);
    // Fast settings: the encoder runs alongside the desktop.
    let speed: &[&str] = match codec {
        Codec::H264 | Codec::Hevc => &["preset=veryfast"],
        Codec::Vp9 => &["b=0", "deadline=realtime", "cpu-used=8", "row-mt=1"],
        Codec::Av1 => &["preset=10"],
    };
    for param in speed {
        args.extend(["-p".into(), (*param).into()]);
    }
    args.extend(["-r".into(), preferences.framerate.to_string()]);
    let device = match preferences.audio {
        Audio::None => None,
        Audio::Desktop => Some("--audio=@DEFAULT_MONITOR@".to_owned()),
        Audio::Microphone => Some("--audio=@DEFAULT_SOURCE@".to_owned()),
        Audio::DesktopAndMicrophone => Some(format!("--audio={MIX}.monitor")),
    };
    if let Some(device) = device {
        let audio_codec = if preferences.container == Container::Webm {
            "libopus"
        } else {
            "aac"
        };
        args.extend([device, "-C".into(), audio_codec.into()]);
    }
    args.push("-f".into());
    args
}

impl Recording {
    /// Whether the encoder is still running.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    /// Whether the disk it writes to is nearly full.
    #[must_use]
    pub fn disk_low(&self) -> bool {
        self.path
            .parent()
            .and_then(free_bytes)
            .is_some_and(|free| free < MIN_FREE_BYTES)
    }
    /// Asks the encoder to finish the file and waits for it (blocking, a
    /// few seconds at most).
    #[must_use]
    pub fn stop(self) -> Ending {
        let Self {
            child, path, mix, ..
        } = self;
        let ending = finish(child, path, Duration::from_secs(5));
        drop(mix);
        ending
    }
}

/// How a recording ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    Saved(PathBuf),
    /// The encoder failed or had to be killed; the file is kept, but its end
    /// may be missing.
    Incomplete(PathBuf),
    Failed(String),
}

impl Ending {
    /// The notice the desktop shows.
    #[must_use]
    pub fn notice(&self) -> String {
        match self {
            Self::Saved(path) => {
                tr!("shell-saved-2", "arg0" => format!("{:}", path.display()))
            }
            Self::Incomplete(path) => {
                tr!("shell-the-recording-may-be-incomplete-what-was-recorded-is-in", "arg0" => format!("{:}", path.display()))
            }
            Self::Failed(error) => error.clone(),
        }
    }
}

fn finish(mut child: std::process::Child, path: PathBuf, patience: Duration) -> Ending {
    if let Ok(pid) = i32::try_from(child.id())
        && let Some(pid) = rustix::process::Pid::from_raw(pid)
        && matches!(child.try_wait(), Ok(None))
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::INT);
    }
    let deadline = Instant::now() + patience;
    // `None`: the encoder did not finish in time and was killed.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let written = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0);
    match (written, status.is_some_and(|status| status.success())) {
        (true, true) => Ending::Saved(path),
        (true, false) => Ending::Incomplete(path),
        (false, _) => {
            let _ = std::fs::remove_file(&path);
            Ending::Failed(tr("shell-the-recording-could-not-be-saved").into())
        }
    }
}

fn free_bytes(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// "1:05" for the bar.
#[must_use]
pub fn elapsed_text(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(script: &str, path: &Path) -> std::process::Child {
        let child = std::process::Command::new("sh")
            .args(["-c", script, "fake-encoder"])
            .arg(path)
            .spawn()
            .unwrap();
        // Let the shell install its traps before it is signalled.
        std::thread::sleep(Duration::from_millis(200));
        child
    }

    #[test]
    fn recording_behaviour() {
        // A sound server that never answers fails the mix within the `pactl`
        // bound instead of holding the recording's start, and an answering
        // one is read.
        //
        // Sabotage: wait for `pactl` without the deadline.
        {
            use std::os::unix::fs::PermissionsExt;
            let tools = tempfile::tempdir().unwrap();
            let tool = tools.path().join("pactl");
            let script = |body: &str| {
                std::fs::write(&tool, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
            };
            script("echo alsa_output.speaker");
            assert_eq!(
                pactl(&tool, &["get-default-sink"]).as_deref(),
                Some("alsa_output.speaker")
            );
            script("exec sleep 30");
            let controls = crate::system_controls::SystemControls::fixture(tools.path());
            let started = Instant::now();
            assert!(Mix::create(&controls).is_err());
            assert!(started.elapsed() < PACTL * 3);
        }
        // Only an encoder that finishes cleanly saves the recording.
        {
            let directory = tempfile::tempdir().unwrap();
            let path = |name: &str| directory.path().join(name);
            let clean = fake(
                "trap 'printf done >> \"$1\"; exit 0' INT; printf data > \"$1\"; while :; do sleep 0.05; done",
                &path("clean.mp4"),
            );
            assert_eq!(
                finish(clean, path("clean.mp4"), Duration::from_secs(3)),
                Ending::Saved(path("clean.mp4"))
            );
            let failed = fake("printf partial > \"$1\"; exit 3", &path("failed.mp4"));
            assert_eq!(
                finish(failed, path("failed.mp4"), Duration::from_secs(3)),
                Ending::Incomplete(path("failed.mp4"))
            );
            let stuck = fake(
                "trap '' INT; printf partial > \"$1\"; while :; do sleep 0.05; done",
                &path("stuck.mp4"),
            );
            let started = Instant::now();
            assert_eq!(
                finish(stuck, path("stuck.mp4"), Duration::from_millis(400)),
                Ending::Incomplete(path("stuck.mp4"))
            );
            assert!(started.elapsed() < Duration::from_secs(3));
            let empty = fake("exit 0", &path("empty.mp4"));
            assert!(matches!(
                finish(empty, path("empty.mp4"), Duration::from_secs(3)),
                Ending::Failed(_)
            ));
            assert!(!path("empty.mp4").exists());
        }
    }
}
