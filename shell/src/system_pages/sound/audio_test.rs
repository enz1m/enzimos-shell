use enzui_shell::i18n::tr;
use iced::futures::{SinkExt, channel::mpsc};
use std::time::Duration;
use std::{io, path::Path, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::watch,
    time::{self, Instant},
};

const RATE: usize = 48_000;
const CHANNELS: usize = 2;
const CHUNK_MS: usize = 50;
const CHUNK_BYTES: usize = RATE * CHANNELS * size_of::<f32>() * CHUNK_MS / 1_000;
pub(super) const TEST_SECONDS: usize = 3;
pub(super) const TEST_CHUNKS: usize = TEST_SECONDS * 1_000 / CHUNK_MS;
const FRAME_PERIOD: Duration = Duration::from_millis(CHUNK_MS as u64);
pub(super) const FLOOR_DB: f32 = -60.0;
const QUIET_DB: f32 = -35.0;
// Signed 16-bit PCM's positive rail converts to a float just below one.
const FULL_SCALE: f32 = 32_767.0 / 32_768.0;

#[derive(Clone)]
pub(in crate::settings_app) struct Clip {
    pcm: Arc<Vec<u8>>,
    peak: f32,
}

impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clip")
            .field("bytes", &self.pcm.len())
            .field("peak", &self.peak)
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::settings_app) struct Meter {
    pub(super) bar: f32,
    pub(super) peak_db: f32,
    pub(super) clipping: bool,
    pub(super) quiet: bool,
}

struct Levels {
    bar_db: f32,
    peak_db: f32,
    held: u8,
    clip_hold: u8,
    quiet_chunks: u8,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            bar_db: FLOOR_DB,
            peak_db: FLOOR_DB,
            held: 0,
            clip_hold: 0,
            quiet_chunks: 0,
        }
    }
}

fn dbfs(peak: f32) -> f32 {
    (20.0 * peak.log10()).clamp(FLOOR_DB, 0.0)
}

impl Levels {
    fn sample(&mut self, peak: f32) -> Meter {
        let db = dbfs(peak);
        let response = if db > self.bar_db { 0.65 } else { 0.18 };
        self.bar_db += (db - self.bar_db) * response;
        if db >= self.peak_db {
            self.peak_db = db;
            self.held = 16;
        } else if self.held == 0 {
            self.peak_db = (self.peak_db - 1.5).max(db);
        } else {
            self.held -= 1;
        }
        self.clip_hold = if peak >= FULL_SCALE {
            20
        } else {
            self.clip_hold.saturating_sub(1)
        };
        self.quiet_chunks = if db >= QUIET_DB {
            0
        } else if db > FLOOR_DB {
            self.quiet_chunks.saturating_add(1)
        } else {
            self.quiet_chunks.saturating_sub(1)
        };
        Meter {
            bar: (self.bar_db - FLOOR_DB) / -FLOOR_DB,
            peak_db: self.peak_db,
            clipping: self.clip_hold > 0,
            quiet: self.quiet_chunks >= 16,
        }
    }
}

pub(super) fn recorded_hint(clip: &Clip) -> Option<&'static str> {
    if clip.peak >= FULL_SCALE {
        Some("shell-mic-clipping")
    } else if dbfs(clip.peak) <= FLOOR_DB {
        Some("shell-mic-no-signal")
    } else if dbfs(clip.peak) < QUIET_DB {
        Some("shell-mic-too-quiet")
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum CaptureEvent {
    Meter(Meter, Option<(u64, usize)>),
    Recorded(u64, Clip),
    Failed,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::settings_app) enum PlaybackEvent {
    Progress(f32),
    Finished(bool),
}

fn command(tool: &Path, mode: &str, target: u64) -> Command {
    let properties = serde_json::json!({
        "application.name": tr("shell-settings"),
        "application.id": "dev.enzui.Settings",
        "media.name": tr("shell-mic-test"),
        "media.role": "Test",
        "node.dont-fallback": true,
        "node.dont-reconnect": true,
        "node.dont-move": true,
    });
    let mut command = Command::new(tool);
    command
        .args([
            mode,
            "--raw",
            "--format",
            "f32",
            "--rate",
            &RATE.to_string(),
            "--channels",
            &CHANNELS.to_string(),
            "--latency",
            "50ms",
            "--target",
            &target.to_string(),
            "--properties",
            &properties.to_string(),
            "-",
        ])
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(super) fn capture(
    tool: std::path::PathBuf,
    source: u64,
    request: watch::Receiver<Option<u64>>,
) -> impl iced::futures::Stream<Item = CaptureEvent> {
    iced::stream::channel(2, async move |mut events| {
        if read_samples(&tool, source, request, &mut events)
            .await
            .is_err()
        {
            let _ = events.send(CaptureEvent::Failed).await;
        }
    })
}

async fn read_samples(
    tool: &Path,
    source: u64,
    mut request: watch::Receiver<Option<u64>>,
    events: &mut mpsc::Sender<CaptureEvent>,
) -> io::Result<()> {
    let mut child = command(tool, "--record", source)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut pcm = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("capture pipe missing"))?;
    let mut block = vec![0; CHUNK_BYTES];
    let mut meter = Levels::default();
    let mut recorded = None;
    loop {
        if request.has_changed().unwrap_or(false) {
            recorded = request
                .borrow_and_update()
                .map(|id| (id, Vec::with_capacity(CHUNK_BYTES * TEST_CHUNKS), 0.0_f32));
        }
        time::timeout(Duration::from_secs(3), pcm.read_exact(&mut block)).await??;
        let peak = block
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(f32::from_ne_bytes)
            .filter(|sample| sample.is_finite())
            .map(f32::abs)
            .fold(0.0, f32::max);
        let progress = recorded.as_mut().map(|(id, bytes, maximum)| {
            bytes.extend_from_slice(&block);
            *maximum = maximum.max(peak);
            (*id, bytes.len() / CHUNK_BYTES)
        });
        if events
            .send(CaptureEvent::Meter(meter.sample(peak), progress))
            .await
            .is_err()
        {
            return Ok(());
        }
        if progress.is_some_and(|(_, chunks)| chunks == TEST_CHUNKS)
            && let Some((id, bytes, peak)) = recorded.take()
            && events
                .send(CaptureEvent::Recorded(
                    id,
                    Clip {
                        pcm: Arc::new(bytes),
                        peak,
                    },
                ))
                .await
                .is_err()
        {
            return Ok(());
        }
    }
}

pub(super) fn playback(
    tool: std::path::PathBuf,
    sink: u64,
    clip: Clip,
) -> impl iced::futures::Stream<Item = PlaybackEvent> {
    iced::stream::channel(2, async move |mut events| {
        let ok = play_samples(&tool, sink, clip, &mut events).await.is_ok();
        let _ = events.send(PlaybackEvent::Finished(ok)).await;
    })
}

async fn play_samples(
    tool: &Path,
    sink: u64,
    clip: Clip,
    events: &mut mpsc::Sender<PlaybackEvent>,
) -> io::Result<()> {
    let mut child = command(tool, "--playback", sink)
        .stdin(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("playback pipe missing"))?;
    let play = async move {
        stdin.write_all(&clip.pcm).await?;
        drop(stdin);
        if child.wait().await?.success() {
            Ok(())
        } else {
            Err(io::Error::other("audio playback failed"))
        }
    };
    let play = time::timeout(Duration::from_secs(6), play);
    tokio::pin!(play);
    let start = Instant::now();
    let mut tick = time::interval(FRAME_PERIOD);
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            result = &mut play => return result?,
            _ = tick.tick() => {
                let progress = (start.elapsed().as_secs_f32() / TEST_SECONDS as f32).min(0.99);
                if events.send(PlaybackEvent::Progress(progress)).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_holds_peaks_and_warns_only_for_clipping_or_sustained_weak_signal() {
        let mut meter = Levels::default();
        assert!(!meter.sample(0.0).quiet);
        for _ in 0..15 {
            assert!(!meter.sample(0.01).quiet);
        }
        assert!(meter.sample(0.01).quiet);
        let normal = meter.sample(0.25);
        assert!(!normal.quiet);
        assert!((normal.peak_db + 12.04).abs() < 0.01);
        assert!(!normal.clipping);
        let clipped = meter.sample(FULL_SCALE);
        assert!(clipped.clipping);
        assert!(clipped.peak_db.abs() < 0.001);
        for _ in 0..16 {
            assert_eq!(meter.sample(0.0).peak_db, clipped.peak_db);
        }
        assert!(meter.sample(0.0).peak_db < clipped.peak_db);
        for _ in 0..3 {
            meter.sample(0.0);
        }
        assert!(!meter.sample(0.0).clipping);
    }
}
