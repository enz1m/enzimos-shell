//! Exercises clipboard capture, restoration and revocation with owned fixture clients.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]
use enzui_shell::clipboard::{
    content::{Content, MAX_TEXT, Mime},
    session::Target,
    transport::{self, Command, Event},
};
use enzui_shell::i18n;
use std::{
    io::Cursor,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command as Process},
    sync::mpsc,
};

async fn source(bytes: &[u8], mime: &str) -> Result<Child, String> {
    let mut child = Process::new("/usr/bin/wl-copy")
        .args(["--foreground", "--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Clipboard fixture could not start")?;
    let mut input = child.stdin.take().ok_or("Fixture has no input")?;
    tokio::time::timeout(Duration::from_secs(3), input.write_all(bytes))
        .await
        .map_err(|_| "Fixture input timed out")?
        .map_err(|_| "Fixture input closed")?;
    drop(input);
    Ok(child)
}
async fn copied(events: &mut mpsc::Receiver<Event>) -> Result<Arc<Content>, String> {
    match tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| "No clipboard event")?
    {
        Some(Event::Copied { content, .. }) => Ok(content),
        _ => Err("Unexpected clipboard event".into()),
    }
}
async fn restore(
    tx: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
    request: u64,
    content: Arc<Content>,
    authority: Arc<AtomicBool>,
) -> Result<bool, String> {
    tx.send(Command::Restore {
        request,
        content,
        authority,
    })
    .await
    .map_err(|_| "Clipboard command channel closed")?;
    match tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .map_err(|_| "No restore acknowledgement")?
    {
        Some(Event::Restored {
            request: got,
            result,
        }) if got == request => Ok(result.is_ok()),
        _ => Err("Unexpected restore acknowledgement".into()),
    }
}
async fn paste(mime: &str) -> Result<std::process::Output, String> {
    tokio::time::timeout(
        Duration::from_secs(3),
        Process::new("/usr/bin/wl-paste")
            .args(["--no-newline", "--type", mime])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Fixture paste timed out")?
    .map_err(|_| "Fixture paste could not start".into())
}
async fn campaign(
    tx: &mpsc::Sender<Command>,
    events: &mut mpsc::Receiver<Event>,
) -> Result<(), String> {
    if !matches!(
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .map_err(|_| "Clipboard did not become ready")?,
        Some(Event::Ready)
    ) {
        return Err("Clipboard did not become ready".into());
    }
    tx.send(Command::Collect(true))
        .await
        .map_err(|_| "Clipboard command channel closed")?;
    let authority = Arc::new(AtomicBool::new(true));
    let text = "Disposable clipboard fixture\nExact UTF-8: café ✓\tend".as_bytes();
    let mut text_source = source(text, "text/plain;charset=utf-8").await?;
    let content = copied(events).await?;
    if content.mime != Mime::Text || &*content.bytes != text {
        return Err("Text capture differs".into());
    }
    if !restore(tx, events, 1, content, std::sync::Arc::clone(&authority)).await? {
        return Err("Text restore rejected".into());
    }
    let pasted = paste("text/plain;charset=utf-8").await?;
    if !pasted.status.success() || pasted.stdout != text {
        return Err("Text paste differs".into());
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), text_source.wait()).await;
    if tokio::time::timeout(Duration::from_millis(300), events.recv())
        .await
        .is_ok()
    {
        return Err("Self-restore recaptured content".into());
    }
    let pixels = raster::RgbaImage::from_pixel(64, 48, raster::Rgba([30, 90, 160, 255]));
    let mut bytes = Cursor::new(Vec::new());
    pixels
        .write_to(&mut bytes, raster::ImageFormat::Png)
        .map_err(|_| "Could not encode fixture PNG")?;
    let bytes = bytes.into_inner();
    let mut image_source = source(&bytes, "image/png").await?;
    let image = copied(events).await?;
    if image.mime != Mime::Png || image.bytes.as_ref() != bytes.as_slice() || image.image.is_none()
    {
        return Err("Image capture differs".into());
    }
    if !restore(
        tx,
        events,
        2,
        std::sync::Arc::clone(&image),
        std::sync::Arc::clone(&authority),
    )
    .await?
    {
        return Err("Image restore rejected".into());
    }
    let pasted = paste("image/png").await?;
    if !pasted.status.success() || pasted.stdout != bytes {
        return Err("Image paste differs".into());
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), image_source.wait()).await;
    if restore(
        tx,
        events,
        3,
        std::sync::Arc::clone(&image),
        Arc::new(AtomicBool::new(false)),
    )
    .await?
    {
        return Err("Revoked restore accepted".into());
    }
    let pasted = paste("image/png").await?;
    if !pasted.status.success() || pasted.stdout != bytes {
        return Err("Rejected restore changed selection".into());
    }
    let mut oversized = source(&vec![b'x'; MAX_TEXT + 1], "text/plain").await?;
    if tokio::time::timeout(Duration::from_millis(600), events.recv())
        .await
        .is_ok()
    {
        return Err("Oversize content escaped its bound".into());
    }
    let mut recovered = source(b"Recovery fixture", "text/plain").await?;
    let recovery = copied(events).await?;
    if &*recovery.bytes != b"Recovery fixture" {
        return Err("Transfer did not recover".into());
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), oversized.wait()).await;
    if !restore(tx, events, 4, recovery, std::sync::Arc::clone(&authority)).await? {
        return Err("Recovery restore rejected".into());
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), recovered.wait()).await;
    authority.store(false, Ordering::Release);
    tokio::time::sleep(Duration::from_millis(200)).await;
    if paste("text/plain").await?.status.success() {
        return Err("Revoked owner retained its selection".into());
    }
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    i18n::initialize();
    let result = async {
        if std::env::args()
            .collect::<Vec<_>>()
            .get(1)
            .map(String::as_str)
            != Some("disposable")
            || !std::env::var("ENZUI_HYPR_RUNROOT").ok().is_some_and(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("clipboard-fixture-"))
            })
        {
            return Err(
                "Only an explicit disposable clipboard-fixture managed session is allowed".into(),
            );
        }
        let target = Target::from_environment()?;
        let alive = Arc::new(AtomicBool::new(true));
        let worker_alive = std::sync::Arc::clone(&alive);
        let (commands, rx) = mpsc::channel(8);
        let (events, mut receiver) = mpsc::channel(4);
        let worker =
            tokio::task::spawn_blocking(move || transport::run(target, rx, events, worker_alive));
        let result = campaign(&commands, &mut receiver).await;
        alive.store(false, Ordering::Release);
        drop(commands);
        let ended = tokio::time::timeout(Duration::from_secs(4), worker)
            .await
            .map_err(|_| "Clipboard worker did not stop")?
            .map_err(|_| "Clipboard worker panicked")?;
        result?;
        ended?;
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => println!(
            "{{\"text_capture_restore\":true,\"png_capture_restore\":true,\"self_copy_suppression\":true,\"revoked_restore_denied\":true,\"oversize_recovery\":true,\"revoked_selection_released\":true,\"worker_joined\":true}}"
        ),
        Err(error) => {
            eprintln!("Clipboard fixture failed: {error}");
            std::process::exit(1);
        }
    }
}
