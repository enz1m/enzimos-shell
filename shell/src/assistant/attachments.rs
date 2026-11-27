use std::{
    fs::OpenOptions,
    io::{Cursor, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use enzui_shell::i18n::tr;
use enzui_widgets::file_chooser::{
    self, CancelReason, ChooserOptions, DbusBackend, FileRequest, FileResult, PendingRequest,
    PortalOperation, PortalOutcome, PortalParent,
};
use enzui_widgets::file_drop::FileDrop;
use iced::{Subscription, Task, widget::image};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Message as Assistant, message};
use crate::{Message as ShellMessage, Shell};

pub(super) const MAX_FILES: usize = 3;
const MAX_TEXT: usize = 4096;
const MAX_FILE: usize = 5 * 1024 * 1024;
const MAX_IMAGE: usize = 128 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Attachment {
    pub(super) name: Arc<str>,
    payload: Arc<Payload>,
}

#[derive(Debug)]
enum Payload {
    Text(String),
    Image {
        jpeg: String,
        preview: image::Handle,
    },
}

impl Attachment {
    pub(super) fn wire(&self) -> Value {
        match &*self.payload {
            Payload::Text(text) => json!({"name": &*self.name, "text": text}),
            Payload::Image { jpeg, .. } => json!({"name": &*self.name, "jpeg": jpeg}),
        }
    }

    pub(super) fn preview(&self) -> Option<image::Handle> {
        match &*self.payload {
            Payload::Text(_) => None,
            Payload::Image { preview, .. } => Some(preview.clone()),
        }
    }

    fn text_bytes(&self) -> usize {
        match &*self.payload {
            Payload::Text(text) => text.len(),
            Payload::Image { .. } => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Pick,
    Wake,
    Picked(u64, PortalOutcome<FileResult>),
    Drop(FileDrop),
    Loaded(u64, Vec<Result<Attachment, String>>, Option<FileDrop>),
    Remove(usize),
}

#[derive(Default)]
pub(super) struct State {
    backend: Option<DbusBackend>,
    pending: Option<PendingRequest>,
    generation: u64,
    loading: bool,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.loading || self.choosing()
    }

    pub(super) fn choosing(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn cancel(&mut self) {
        self.generation += 1;
        self.loading = false;
        if let Some(mut pending) = self.pending.take() {
            pending.cancel_handle().cancel(CancelReason::Application);
            if let Some(backend) = self.backend.as_mut() {
                pending.settle(backend);
            }
        }
    }

    pub(super) fn subscription(&self) -> Subscription<ShellMessage> {
        file_chooser::completion_subscription(
            self.generation,
            self.pending.as_ref(),
            self.backend
                .as_ref()
                .map(|backend| backend as &dyn file_chooser::PortalBackend),
        )
        .map(|_| message(Assistant::Attachments(Message::Wake)))
    }
}

impl Shell {
    pub(super) fn assistant_attachments(&mut self, event: Message) -> Task<ShellMessage> {
        let state = &mut self.assistant;
        let chooser = &mut state.attachment_loader;
        if state.running.is_some()
            || state.approval_busy
            || state.revision.is_some()
            || state.panel.as_ref().is_none_or(|pane| !pane.visible)
        {
            return Task::none();
        }
        match event {
            Message::Pick if !chooser.busy() && state.files.len() < MAX_FILES => {
                if chooser.backend.is_none() {
                    chooser.backend = DbusBackend::new().ok();
                }
                if let Some(backend) = chooser.backend.as_mut() {
                    let options = ChooserOptions::new().title(tr("shell-assistant-attach-files"));
                    let request =
                        FileRequest::new(PortalOperation::OpenMultiple, PortalParent::None)
                            .with_options(options);
                    if let Ok(pending) = request.start(backend, |_| true) {
                        chooser.pending = Some(pending);
                        state.error = None;
                        return Task::none();
                    }
                }
                state.error = Some(tr("shell-file-chooser-unavailable").into());
            }
            Message::Wake => {
                if let (Some(pending), Some(backend)) =
                    (chooser.pending.as_mut(), chooser.backend.as_mut())
                {
                    let generation = chooser.generation;
                    let task = file_chooser::pump(pending, backend, move |outcome| {
                        message(Assistant::Attachments(Message::Picked(generation, outcome)))
                    });
                    if !pending.is_unsettled() {
                        chooser.pending = None;
                    }
                    return task;
                }
            }
            Message::Picked(generation, outcome) if generation == chooser.generation => {
                if let Some(error) = file_chooser::failure_text(&outcome) {
                    state.error = Some(error);
                } else if let PortalOutcome::Completed(result) = outcome {
                    let files = result.local_paths();
                    if files.is_empty() {
                        state.error = Some(tr("shell-file-chooser-local-file").into());
                    } else {
                        return self.assistant_load_files(files, None);
                    }
                }
            }
            Message::Drop(drop) if !chooser.busy() => {
                return self.assistant_load_files(drop.paths().to_vec(), Some(drop));
            }
            Message::Loaded(generation, loaded, drop) if generation == chooser.generation => {
                chooser.loading = false;
                let mut text_bytes = state
                    .files
                    .iter()
                    .map(Attachment::text_bytes)
                    .sum::<usize>();
                let mut errors = Vec::new();
                for result in loaded {
                    match result {
                        Ok(file) if text_bytes + file.text_bytes() <= MAX_TEXT => {
                            text_bytes += file.text_bytes();
                            state.files.push(file);
                        }
                        Ok(_) => errors.push(tr("shell-assistant-attachment-text-limit").into()),
                        Err(error) => errors.push(error),
                    }
                }
                if errors.is_empty()
                    && let Some(drop) = drop
                {
                    drop.finish();
                }
                state.error = (!errors.is_empty()).then(|| errors.join("\n"));
            }
            Message::Remove(index) if index < state.files.len() && !chooser.busy() => {
                state.files.remove(index);
                state.error = None;
            }
            _ => {}
        }
        Task::none()
    }

    fn assistant_load_files(
        &mut self,
        files: Vec<PathBuf>,
        drop: Option<FileDrop>,
    ) -> Task<ShellMessage> {
        let state = &mut self.assistant;
        if files.len() + state.files.len() > MAX_FILES {
            state.error = Some(tr("shell-assistant-attachment-count-limit").into());
            return Task::none();
        }
        state.error = None;
        state.attachment_loader.loading = true;
        let generation = state.attachment_loader.generation;
        super::perform(async move {
            let mut loaded = Vec::with_capacity(files.len());
            for file in files {
                loaded.push(load(file).await);
            }
            Assistant::Attachments(Message::Loaded(generation, loaded, drop))
        })
    }
}

fn read_file(path: &Path) -> Result<Vec<u8>, &'static str> {
    let unreadable = "shell-assistant-attachment-unreadable";
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| unreadable)?;
    let metadata = file.metadata().map_err(|_| unreadable)?;
    if !metadata.is_file() {
        return Err(unreadable);
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable)?;
    if bytes.len() > MAX_FILE {
        return Err("shell-assistant-attachment-size-limit");
    }
    Ok(bytes)
}

async fn load(path: PathBuf) -> Result<Attachment, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.len() <= 255)
        .ok_or_else(|| tr("shell-assistant-attachment-unreadable").to_owned())?;
    let name: Arc<str> = Arc::from(name);
    let error = |key| format!("{} ({})", tr(key), name);
    let bytes = tokio::task::spawn_blocking(move || read_file(&path))
        .await
        .map_err(|_| error("shell-assistant-attachment-unreadable"))?
        .map_err(error)?;
    let payload = if bytes.starts_with(b"%PDF-") {
        let extracted = pdf_text(bytes).await.map_err(error)?;
        Payload::Text(text(extracted).map_err(error)?)
    } else if matches!(
        raster::guess_format(&bytes),
        Ok(raster::ImageFormat::Png | raster::ImageFormat::Jpeg | raster::ImageFormat::WebP)
    ) {
        tokio::task::spawn_blocking(move || image_payload(&bytes))
            .await
            .map_err(|_| error("shell-assistant-attachment-unreadable"))?
            .map_err(error)?
    } else {
        Payload::Text(text(bytes).map_err(error)?)
    };
    Ok(Attachment {
        name,
        payload: Arc::new(payload),
    })
}

fn text(bytes: Vec<u8>) -> Result<String, &'static str> {
    if bytes.len() > MAX_TEXT {
        return Err("shell-assistant-attachment-text-limit");
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|text| {
            !text.trim().is_empty()
                && !text
                    .chars()
                    .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t' | '\u{000c}'))
        })
        .ok_or("shell-assistant-attachment-format")
}

fn image_payload(bytes: &[u8]) -> Result<Payload, &'static str> {
    let invalid = "shell-assistant-attachment-image-invalid";
    let mut reader = raster::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid)?;
    let mut limits = raster::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| invalid)?;
    for edge in [768, 512, 384] {
        let rgb = decoded.thumbnail(edge, edge).to_rgb8();
        let mut encoded = Vec::new();
        raster::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 75)
            .encode_image(&rgb)
            .map_err(|_| invalid)?;
        let jpeg = STANDARD.encode(&encoded);
        if jpeg.len() <= MAX_IMAGE {
            return Ok(Payload::Image {
                jpeg,
                preview: image::Handle::from_bytes(encoded),
            });
        }
    }
    Err(invalid)
}

async fn pdf_text(bytes: Vec<u8>) -> Result<Vec<u8>, &'static str> {
    let unavailable = "shell-assistant-attachment-pdf-unavailable";
    let invalid = "shell-assistant-attachment-pdf-invalid";
    let mut child = tokio::process::Command::new("pdftotext")
        .args(["-enc", "UTF-8", "-nopgbrk", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable)?;
    let mut input = child.stdin.take().ok_or(invalid)?;
    let mut output = child
        .stdout
        .take()
        .ok_or(invalid)?
        .take((MAX_TEXT + 1) as u64);
    tokio::time::timeout(Duration::from_secs(5), async move {
        let mut text = Vec::new();
        let (sent, read) = tokio::join!(
            async move { input.write_all(&bytes).await },
            output.read_to_end(&mut text)
        );
        if text.len() > MAX_TEXT {
            return Err("shell-assistant-attachment-text-limit");
        }
        sent.map_err(|_| invalid)?;
        read.map_err(|_| invalid)?;
        if child.wait().await.is_ok_and(|status| status.success())
            && text.iter().any(|byte| !byte.is_ascii_whitespace())
        {
            Ok(text)
        } else {
            Err(invalid)
        }
    })
    .await
    .map_err(|_| invalid)?
}
