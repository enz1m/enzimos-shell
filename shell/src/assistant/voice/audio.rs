use base64::Engine as _;
use serde_json::{Value, json};
use std::{collections::VecDeque, io, path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, oneshot},
};

use enzui_shell::local_ai::{Service, exchange_local_async};

const FRAME: usize = 640;
const MAX_FRAMES: usize = 600;
const TRAILING_SILENCE: usize = 32;

pub(super) type Warm = Arc<Mutex<Option<Synth>>>;

pub(super) struct Synth {
    _child: Child,
    input: ChildStdin,
    output: ChildStdout,
}

fn pipewire(path: &str, rate: &str, role: &str) -> Command {
    let mut command = Command::new(path);
    command
        .args([
            "--raw",
            "--rate",
            rate,
            "--channels",
            "1",
            "--format",
            "s16",
            "--latency",
            "60ms",
            "--media-role",
            role,
            "-",
        ])
        .kill_on_drop(true)
        .stderr(Stdio::null());
    command
}

fn bad_stream() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid voice audio stream")
}

async fn load() -> io::Result<Synth> {
    let name = "enzui-voice-runtime";
    let sibling = std::env::current_exe()?.with_file_name(name);
    let executable = if sibling.is_file() {
        sibling
    } else {
        Path::new("/usr/bin").join(name)
    };
    let package = Path::new("/usr/share/enzimos/voice/phonon-31m-en");
    let staged = executable.with_file_name("phonon-31m-en");
    let model = if staged.is_dir() { &staged } else { package };
    let mut child = Command::new("/usr/bin/nice")
        .args(["-n", "5"])
        .arg(executable)
        .arg(model)
        .env("RAYON_NUM_THREADS", "1")
        .env("TOKENIZERS_PARALLELISM", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let input = child.stdin.take().ok_or_else(bad_stream)?;
    let mut output = child.stdout.take().ok_or_else(bad_stream)?;
    let mut tag = [0; 4];
    output.read_exact(&mut tag).await?;
    if &tag != b"EV01" {
        return Err(bad_stream());
    }
    Ok(Synth {
        _child: child,
        input,
        output,
    })
}

pub(super) async fn warm(cache: Warm) -> io::Result<()> {
    let mut cached = cache.lock().await;
    if cached.is_none() {
        *cached = Some(tokio::time::timeout(Duration::from_secs(10), load()).await??);
    }
    Ok(())
}

pub(super) async fn play(cache: Warm, text: String) -> io::Result<()> {
    let mut cached = cache.lock().await;
    let mut synth = match cached.take() {
        Some(synth) => synth,
        None => tokio::time::timeout(Duration::from_secs(10), load()).await??,
    };
    tokio::time::timeout(Duration::from_secs(24), play_stream(&mut synth, &text)).await??;
    *cached = Some(synth);
    Ok(())
}

async fn play_stream(synth: &mut Synth, text: &str) -> io::Result<()> {
    synth.input.write_all(text.as_bytes()).await?;
    synth.input.write_all(b"\n").await?;
    let mut player = None;
    let mut audio = None;
    let mut pcm = vec![0; 32 * 1024];
    loop {
        let count = synth.output.read_u32_le().await? as usize;
        if count == 0 {
            break;
        }
        if count > pcm.len() || !count.is_multiple_of(2) {
            return Err(bad_stream());
        }
        let pcm = &mut pcm[..count];
        synth.output.read_exact(pcm).await?;
        if audio.is_none() {
            let mut child = pipewire("/usr/bin/pw-play", "24000", "Communication")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()?;
            audio = child.stdin.take();
            player = Some(child);
        }
        audio
            .as_mut()
            .ok_or_else(bad_stream)?
            .write_all(pcm)
            .await?;
    }
    drop(audio);
    if let Some(mut player) = player
        && player.wait().await?.success()
    {
        return Ok(());
    }
    Err(bad_stream())
}

#[derive(Default)]
struct Utterance {
    pcm: Vec<u8>,
    lead: VecDeque<[u8; FRAME]>,
    voiced: usize,
    trailing: usize,
}

impl Utterance {
    fn frame(&mut self, frame: [u8; FRAME], automatic: bool) -> bool {
        let (samples, _) = frame.as_chunks::<2>();
        let square_sum = samples.iter().fold(0i64, |sum, &sample| {
            let sample = i64::from(i16::from_le_bytes(sample));
            sum + sample * sample
        });
        let speech = square_sum > 550 * 550 * (FRAME / 2) as i64;
        if speech {
            self.voiced += 1;
            self.trailing = 0;
        } else {
            self.trailing += 1;
        }
        if automatic && self.voiced == 0 {
            if self.lead.len() == 10 {
                self.lead.pop_front();
            }
            self.lead.push_back(frame);
            return false;
        }
        for leading in self.lead.drain(..) {
            self.pcm.extend_from_slice(&leading);
        }
        self.pcm.extend_from_slice(&frame);
        self.pcm.len() >= MAX_FRAMES * FRAME || (automatic && self.trailing >= TRAILING_SILENCE)
    }

    fn wav(self) -> Option<Vec<u8>> {
        if self.voiced < 10 {
            return None;
        }
        let size = u32::try_from(self.pcm.len()).ok()?;
        let mut wav = b"RIFF".to_vec();
        wav.extend((size + 36).to_le_bytes());
        wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data");
        wav.extend(size.to_le_bytes());
        wav.extend(self.pcm);
        Some(wav)
    }
}

pub(super) async fn capture(
    automatic: bool,
    mut stop: oneshot::Receiver<bool>,
) -> io::Result<Option<Vec<u8>>> {
    let mut recorder = pipewire("/usr/bin/pw-record", "16000", "Communication")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = recorder.stdout.take().ok_or_else(bad_stream)?;
    let mut utterance = Utterance::default();
    loop {
        let mut frame = [0; FRAME];
        tokio::select! {
            biased;
            finish = &mut stop => {
                if finish != Ok(true) {
                    return Ok(None);
                }
                break;
            }
            read = tokio::time::timeout(Duration::from_secs(3), input.read_exact(&mut frame)) => {
                read??;
                if utterance.frame(frame, automatic) {
                    break;
                }
            }
        }
    }
    recorder.kill().await?;
    Ok(utterance.wav())
}

pub(super) async fn transcribe(wav: Vec<u8>, directed: bool) -> Option<String> {
    let request = || {
        exchange_local_async(
            Service::Inference,
            "GET",
            "/v1/models",
            None,
            Duration::from_secs(4),
        )
    };
    let (200, listing) = request().await? else {
        return None;
    };
    let model = recognition_model(&listing, directed)?;
    let body = json!({
        "model": model,
        "directed": directed,
        "wav_base64": base64::engine::general_purpose::STANDARD.encode(wav),
    });
    let (200, response): (u16, Value) = exchange_local_async(
        Service::Inference,
        "POST",
        "/v1/audio/voice-transcriptions",
        Some(&body),
        Duration::from_secs(45),
    )
    .await?
    else {
        return None;
    };
    let text = response["text"].as_str()?;
    Some(
        text.split_whitespace()
            .take(100)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn recognition_model(listing: &Value, directed: bool) -> Option<&str> {
    let models = listing["data"].as_array()?;
    models
        .iter()
        .filter(|entry| entry["kind"] == "speech" && (directed || entry["voice_parallel"] == true))
        .min_by_key(|entry| {
            (
                entry["voice_parallel"] != true,
                entry["default"] != true,
                entry["id"] != "whisper-base",
            )
        })?["id"]
        .as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_prefers_parallel_whisper_without_changing_dictation_default() {
        let mut listing = json!({"data":[
            {"id":"parakeet","kind":"speech","default":true},
            {"id":"whisper-base","kind":"speech","voice_parallel":true},
            {"id":"custom-whisper","kind":"speech","voice_parallel":true},
        ]});
        assert_eq!(recognition_model(&listing, false), Some("whisper-base"));
        assert_eq!(recognition_model(&listing, true), Some("whisper-base"));
        listing["data"].as_array_mut().unwrap().truncate(1);
        assert_eq!(recognition_model(&listing, false), None);
        assert_eq!(recognition_model(&listing, true), Some("parakeet"));
    }

    #[test]
    fn automatic_capture_keeps_preroll_and_rejects_short_noise() {
        let mut capture = Utterance::default();
        for _ in 0..100 {
            assert!(!capture.frame([0; FRAME], true));
        }
        assert!(!capture.frame([0x12; FRAME], true));
        assert!(capture.wav().is_none());

        let mut capture = Utterance::default();
        for _ in 0..30 {
            capture.frame([0; FRAME], true);
        }
        for _ in 0..10 {
            capture.frame([0x12; FRAME], true);
        }
        for _ in 1..TRAILING_SILENCE {
            assert!(!capture.frame([0; FRAME], true));
        }
        assert!(capture.frame([0; FRAME], true));
        assert_eq!(
            capture.wav().expect("a speech-length sample").len(),
            44 + (20 + TRAILING_SILENCE) * FRAME
        );
    }
}

pub(super) async fn streaming_socket() -> Option<tokio::net::UnixStream> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    tokio::net::UnixStream::connect(
        Path::new(&runtime).join("enzimos-streaming-speech/service.sock"),
    )
    .await
    .ok()
}

pub(super) async fn streaming<F, Update>(
    socket: tokio::net::UnixStream,
    automatic: bool,
    mut stop: oneshot::Receiver<bool>,
    mut partial: F,
) -> io::Result<(String, Vec<(String, f64)>)>
where
    F: FnMut(String) -> Update,
    Update: std::future::Future<Output = ()>,
{
    use tokio::io::AsyncBufReadExt as _;
    let (reader, mut writer) = socket.into_split();
    writer
        .write_all(&[if automatic { b'a' } else { b'm' }, 12])
        .await?;
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let mut finishing = false;
    let mut alternatives = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_secs(if automatic { 78 } else { 18 }));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            finish = &mut stop, if !finishing => {
                if finish != Ok(true) {
                    return Ok((String::new(), Vec::new()));
                }
                finishing = true;
                writer.write_all(b"f").await?;
            }
            line = lines.next_line() => {
                let line = line?.ok_or_else(bad_stream)?;
                let (kind, text) = line.split_once('\t').ok_or_else(bad_stream)?;
                match kind {
                    "ready" if text == "alternatives-v1" => writer.write_all(b"n").await?,
                    "ready" | "level" | "transcribing" => {}
                    "partial" if !finishing => partial(text.trim().to_owned()).await,
                    "partial" => {}
                    "alternatives" => alternatives = serde_json::from_str(text).unwrap_or_default(),
                    "final" => return Ok((text.trim().to_owned(), alternatives)),
                    _ => return Err(bad_stream()),
                }
            }
            () = &mut deadline => return Err(bad_stream()),
        }
    }
}
