use super::{
    budget::{BudgetPool, BudgetSlot},
    manifest::{MAX_PROGRAM_BYTES, Package, Permission},
};
use enzui_shell::i18n::tr;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

pub const MAX_FRAME_BYTES: usize = 65536;
const FRAME_DEADLINE: Duration = Duration::from_secs(3);
const START_DEADLINE: Duration = Duration::from_secs(5);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Unavailable,
    Exited,
    Deadline,
    InputBlocked,
    FrameTooLarge,
    InvalidJson,
    Flood,
    Protocol,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => tr("shell-plugin-isolation-is-unavailable"),
            Self::Exited => tr("shell-plugin-process-exited"),
            Self::Deadline => tr("shell-plugin-response-deadline-exceeded"),
            Self::InputBlocked => tr("shell-plugin-did-not-accept-input"),
            Self::FrameTooLarge => tr("shell-plugin-message-exceeded-its-limit"),
            Self::InvalidJson => tr("shell-plugin-returned-invalid-json"),
            Self::Flood => tr("shell-plugin-output-rate-exceeded-its-limit"),
            Self::Protocol => tr("shell-plugin-protocol-was-rejected"),
        })
    }
}
struct Lines {
    buffer: Vec<u8>,
}
impl Lines {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(MAX_FRAME_BYTES + 1),
        }
    }
    async fn next<R: AsyncRead + Unpin>(&mut self, reader: &mut R) -> Result<Vec<u8>, Fault> {
        let mut scanned = 0;
        loop {
            if let Some(end) = self.buffer[scanned..]
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| i + scanned)
            {
                if end > MAX_FRAME_BYTES {
                    return Err(Fault::FrameTooLarge);
                }
                let frame = self.buffer[..end].to_vec();
                self.buffer.drain(..=end);
                return Ok(frame);
            }
            if self.buffer.len() > MAX_FRAME_BYTES {
                return Err(Fault::FrameTooLarge);
            }
            scanned = self.buffer.len();
            let mut bytes = [0u8; 4096];
            let limit = bytes.len().min(MAX_FRAME_BYTES + 1 - self.buffer.len());
            let count = reader
                .read(&mut bytes[..limit])
                .await
                .map_err(|_| Fault::Exited)?;
            if count == 0 {
                return Err(Fault::Exited);
            }
            self.buffer.extend_from_slice(&bytes[..count]);
        }
    }
}
struct Rate {
    since: Instant,
    messages: usize,
    bytes: usize,
}
impl Rate {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            messages: 0,
            bytes: 0,
        }
    }
    fn accept(&mut self, size: usize) -> Result<(), Fault> {
        if self.since.elapsed() >= Duration::from_secs(1) {
            *self = Self::new();
        }
        self.messages += 1;
        self.bytes = self.bytes.saturating_add(size);
        if self.messages > 32 || self.bytes > 4 * MAX_FRAME_BYTES {
            Err(Fault::Flood)
        } else {
            Ok(())
        }
    }
}
pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    lines: Lines,
    rate: Rate,
    slot: BudgetSlot,
    generation: String,
    stopped: bool,
    read_deadline: Option<Instant>,
    writing: bool,
}
/// Uses the exported worker in installed layouts and the build-local worker in checkouts.
pub fn helper() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("ENZUI_DESKTOP_ROOT")
        .map(std::path::PathBuf::from)
        .filter(|root| root.is_absolute());
    let helper = match root.as_deref().and_then(crate::installation::layout) {
        Some(layout) => layout.bin.join("enzui-plugin-worker"),
        None => Some(env!("ENZUI_PLUGIN_WORKER"))
            .filter(|path| !path.is_empty())?
            .into(),
    };
    helper.is_file().then_some(helper)
}
impl Worker {
    pub async fn spawn(
        pool: &BudgetPool,
        package: &Package,
        config: &Value,
        grants: &BTreeSet<Permission>,
    ) -> Result<Self, Fault> {
        let helper = helper().ok_or(Fault::Unavailable)?;
        let manifest = package.manifest();
        if !grants.is_subset(&manifest.permissions) || manifest.validate_config(config).is_err() {
            return Err(Fault::Protocol);
        }
        let slot = pool.slot().map_err(|_| Fault::Unavailable)?;
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|_| Fault::Unavailable)?;
        let generation = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let mut child = Command::new(helper)
            .arg(slot.path())
            .arg("--pipe-source-v2")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| Fault::Unavailable)?;
        let input = child.stdin.take().ok_or(Fault::Unavailable)?;
        let output = child.stdout.take().ok_or(Fault::Unavailable)?;
        let mut worker = Self {
            child,
            input,
            output,
            lines: Lines::new(),
            rate: Rate::new(),
            slot,
            generation,
            stopped: false,
            read_deadline: Some(Instant::now() + START_DEADLINE),
            writing: false,
        };
        send_program(&mut worker.input, package.program().as_bytes()).await?;
        let hello = worker.receive_with_deadline(START_DEADLINE).await?;
        if hello.as_object().map(serde_json::Map::len) != Some(3)
            || hello["type"].as_str() != Some("hello")
            || hello["protocol"].as_u64() != Some(1)
            || hello["plugin"].as_str() != Some(&manifest.id)
        {
            worker.stop();
            return Err(Fault::Protocol);
        }
        worker.send(&json!({"type":"start","protocol":1,"generation":worker.generation,"config":config,"grants":grants.iter().map(|p|p.key()).collect::<Vec<_>>()})).await?;
        Ok(worker)
    }
    pub fn generation(&self) -> &str {
        &self.generation
    }
    pub fn resources(&self) -> Result<super::budget::ResourceStats, Fault> {
        self.slot.stats().map_err(|_| Fault::Unavailable)
    }
    pub async fn send(&mut self, value: &Value) -> Result<(), Fault> {
        if self.stopped {
            return Err(Fault::Exited);
        }
        if self.writing {
            self.stop();
            return Err(Fault::InputBlocked);
        }
        self.writing = true;
        let result = async {
            let mut bytes = serde_json::to_vec(value).map_err(|_| Fault::Protocol)?;
            if bytes.len() > MAX_FRAME_BYTES {
                return Err(Fault::FrameTooLarge);
            }
            bytes.push(b'\n');
            timeout(Duration::from_millis(500), self.input.write_all(&bytes))
                .await
                .map_err(|_| Fault::InputBlocked)?
                .map_err(|_| Fault::Exited)
        }
        .await;
        self.writing = false;
        if result.is_err() {
            self.stop();
        }
        result
    }
    pub async fn receive(&mut self) -> Result<Value, Fault> {
        self.receive_with_deadline(FRAME_DEADLINE).await
    }
    async fn receive_with_deadline(&mut self, deadline: Duration) -> Result<Value, Fault> {
        if self.stopped {
            return Err(Fault::Exited);
        }
        let when = *self
            .read_deadline
            .get_or_insert_with(|| Instant::now() + deadline);
        let result = async {
            let remaining = when
                .checked_duration_since(Instant::now())
                .ok_or(Fault::Deadline)?;
            let bytes = timeout(remaining, self.lines.next(&mut self.output))
                .await
                .map_err(|_| Fault::Deadline)??;
            self.read_deadline = None;
            self.rate.accept(bytes.len())?;
            serde_json::from_slice(&bytes).map_err(|_| Fault::InvalidJson)
        }
        .await;
        if result.is_err() {
            self.stop();
        }
        result
    }
    pub fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            let _ = self.slot.terminate();
            let _ = self.child.start_kill();
        }
    }
    pub async fn shutdown(mut self) -> Result<(), Fault> {
        self.stop();
        timeout(Duration::from_secs(2), self.child.wait())
            .await
            .map_err(|_| Fault::Deadline)?
            .map_err(|_| Fault::Exited)?;
        for _ in 0..40 {
            if self.slot.empty().map_err(|_| Fault::Unavailable)? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Err(Fault::Deadline)
    }
}
async fn send_program<W: AsyncWrite + Unpin>(input: &mut W, program: &[u8]) -> Result<(), Fault> {
    if program.is_empty() || program.len() > MAX_PROGRAM_BYTES {
        return Err(Fault::Protocol);
    }
    let mut header = [0; 12];
    header[..8].copy_from_slice(b"ENZPY002");
    header[8..].copy_from_slice(&(program.len() as u32).to_be_bytes());
    timeout(Duration::from_millis(500), async {
        input.write_all(&header).await?;
        input.write_all(program).await
    })
    .await
    .map_err(|_| Fault::InputBlocked)?
    .map_err(|_| Fault::Exited)
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_async_behaviour() {
        // Framing retains following messages and caps incomplete frames.
        {
            let mut input = &b"{\"one\":1}\n{\"two\":2}\n"[..];
            let mut lines = Lines::new();
            assert_eq!(lines.next(&mut input).await.unwrap(), b"{\"one\":1}");
            assert_eq!(lines.next(&mut input).await.unwrap(), b"{\"two\":2}");
            assert_eq!(lines.next(&mut input).await.unwrap_err(), Fault::Exited);
            let bytes = vec![b'x'; MAX_FRAME_BYTES + 2];
            let mut input = bytes.as_slice();
            let mut lines = Lines::new();
            assert_eq!(
                lines.next(&mut input).await.unwrap_err(),
                Fault::FrameTooLarge
            );
            assert_eq!(lines.buffer.len(), MAX_FRAME_BYTES + 1);
        }
        // Source bootstrap write is bounded when the runner does not read.
        {
            let (mut output, _input) = tokio::io::duplex(1);
            assert_eq!(
                send_program(&mut output, b"pass\n").await,
                Err(Fault::InputBlocked)
            );
        }
    }
}
