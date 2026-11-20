use crate::{compositor::Compositor, workspaces::Desktops};
use enzui_shell::i18n::tr;
use iced::widget::image::Handle;
use raster::{
    RgbaImage,
    imageops::{FilterType, overlay, resize},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub struct Preview {
    pub desktop: u32,
    pub image: Handle,
    pub unavailable: usize,
}
#[derive(Clone, Debug, PartialEq)]
struct Candidate {
    address: String,
    pid: u32,
    start: String,
    desktop: u32,
    at: [f64; 2],
    size: [f64; 2],
}
fn candidate(value: &serde_json::Value, monitor: i64) -> Option<Candidate> {
    let address = value["address"].as_str()?;
    let hex = address.strip_prefix("0x")?;
    if hex.is_empty()
        || hex.len() > 16
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
        || value["monitor"].as_i64() != Some(monitor)
    {
        return None;
    }
    if matches!(
        value["class"].as_str(),
        Some(
            "dev.enzui.Desktop"
                | "dev.enzui.ControlCenter"
                | "dev.enzui.Launcher"
                | "dev.enzui.QuickSearch"
                | "dev.enzui.WorkspaceOverview"
                | "dev.enzui.WindowSpread"
                | "dev.enzui.Sidebar"
                | "dev.enzui.SidebarPane"
                | "dev.enzui.SidebarBacking"
        )
    ) {
        return None;
    }
    if value["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|t| {
            t.as_str().is_some_and(|t| {
                t.starts_with("enzremote-guard-") || t.starts_with("enzui-sidebar-")
            })
        })
    }) {
        return None;
    }
    let desktop = u32::try_from(value["workspace"]["id"].as_u64()?).ok()?;
    if desktop == 0 {
        return None;
    }
    let pid = u32::try_from(value["pid"].as_u64()?).ok()?;
    if pid <= 1 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let start = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .to_owned();
    let at = [value["at"][0].as_f64()?, value["at"][1].as_f64()?];
    let size = [value["size"][0].as_f64()?, value["size"][1].as_f64()?];
    if !at.iter().all(|v| v.is_finite() && v.abs() <= 100000.0)
        || !size
            .iter()
            .all(|v| v.is_finite() && (1.0..=16384.0).contains(v))
    {
        return None;
    }
    Some(Candidate {
        address: address.into(),
        pid,
        start,
        desktop,
        at,
        size,
    })
}
fn cache_root() -> Result<PathBuf, String> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .filter(|p| p.is_absolute())
        .map(|p| p.join("enzui-shell/previews"))
        .ok_or_else(|| tr("shell-no-preview-cache-directory-is-configured").into())
}
fn read_image(path: &Path) -> Result<RgbaImage, String> {
    let metadata = path
        .metadata()
        .map_err(|_| tr("shell-missing-preview-image"))?;
    if metadata.len() > 32 * 1024 * 1024 {
        return Err(tr("shell-preview-image-exceeds-its-byte-bound").into());
    }
    let reader =
        raster::ImageReader::open(path).map_err(|_| tr("shell-cannot-open-preview-image"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| tr("shell-cannot-read-preview-dimensions"))?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 16 * 1024 * 1024
    {
        return Err(tr("shell-preview-image-exceeds-its-pixel-bound").into());
    }
    raster::open(path)
        .map(raster::DynamicImage::into_rgba8)
        .map_err(|_| tr("shell-cannot-decode-preview-image").into())
}

// Installed next to the shell; a development tree builds it under .build.
/// Asks the snapshot helper for a picture fitting `WxH` pixels: it averages
/// the window down as it copies it, so a full-size picture is neither
/// encoded nor decoded. A helper without it writes the window's size, which
/// is then scaled here.
const SNAPSHOT_FIT: &str = "ENZUI_SNAPSHOT_FIT";

fn snapshot_helper(root: &Path) -> PathBuf {
    std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(|dir| PathBuf::from(dir).join("window-snapshot"))
        .filter(|path| path.is_absolute() && path.is_file())
        .unwrap_or_else(|| root.join(".build/window-snapshot/window-snapshot"))
}

/// One window's picture, scaled to fit `width` × `height` pixels.
pub async fn capture_window(
    root: PathBuf,
    address: String,
    width: u32,
    height: u32,
) -> Result<Handle, String> {
    let valid = address.len() > 2
        && address.len() <= 20
        && address.starts_with("0x")
        && address
            .strip_prefix("0x")
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()));
    if !valid || !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(tr("shell-invalid-window-picture-request").into());
    }
    let cache = cache_root()?;
    std::fs::create_dir_all(&cache).map_err(|_| tr("shell-cannot-create-preview-cache"))?;
    let temporary = tempfile::Builder::new()
        .prefix("window-")
        .tempdir_in(cache)
        .map_err(|_| tr("shell-cannot-create-private-preview-directory"))?;
    let path = temporary.path().join("window.png");
    let helper = snapshot_helper(&root);
    let announced = helper.clone();
    tokio::task::spawn_blocking(move || crate::compositor::announce_capture(&announced))
        .await
        .map_err(|_| tr("shell-capture-stopped"))??;
    let status = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new(helper)
            .args([
                address.as_str(),
                path.to_str()
                    .ok_or_else(|| tr("shell-invalid-preview-path"))?,
            ])
            .env(SNAPSHOT_FIT, format!("{width}x{height}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status(),
    )
    .await;
    if !matches!(status, Ok(Ok(status)) if status.success()) {
        return Err(tr("shell-the-window-could-not-be-captured").into());
    }
    let image = tokio::task::spawn_blocking(move || {
        let image = read_image(&path)?;
        // A window that is not being drawn (tucked into the sidebar) comes
        // back as an empty picture; callers show their placeholder instead
        // of a blank card.
        if image.pixels().all(|pixel| pixel.0[3] == 0) {
            return Err(tr("shell-the-window-could-not-be-captured").to_owned());
        }
        // Never enlarged: a picture bigger than its slot is scaled down.
        let fit = (width as f32 / image.width() as f32)
            .min(height as f32 / image.height() as f32)
            .min(1.0);
        let (w, h) = (
            ((image.width() as f32 * fit).round() as u32).max(1),
            ((image.height() as f32 * fit).round() as u32).max(1),
        );
        Ok::<_, String>(if (w, h) == (image.width(), image.height()) {
            image
        } else {
            resize(&image, w, h, FilterType::Triangle)
        })
    })
    .await
    .map_err(|_| tr("shell-preview-decoder-stopped"))??;
    drop(temporary);
    Ok(Handle::from_rgba(
        image.width(),
        image.height(),
        image.into_raw(),
    ))
}

/// The wallpaper scaled to a thumbnail, decoded once while the file and the
/// size stay the same: every preview capture starts from it.
fn wallpaper_thumbnail(path: &Path, width: u32, height: u32) -> Result<RgbaImage, String> {
    type Key = (PathBuf, Option<std::time::SystemTime>, u64, u32, u32);
    static LAST: std::sync::Mutex<Option<(Key, RgbaImage)>> = std::sync::Mutex::new(None);
    let metadata = std::fs::metadata(path).ok();
    let key: Key = (
        path.to_owned(),
        metadata.as_ref().and_then(|m| m.modified().ok()),
        metadata.as_ref().map_or(0, std::fs::Metadata::len),
        width,
        height,
    );
    let mut last = LAST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((cached, image)) = last.as_ref()
        && *cached == key
    {
        return Ok(image.clone());
    }
    let image = resize(&read_image(path)?, width, height, FilterType::Triangle);
    *last = Some((key, image.clone()));
    Ok(image)
}

/// What a desktop's preview starts from: the wallpaper that desktop shows. A
/// chosen picture, colour or live scene that cannot be read falls back to the
/// built-in wallpaper rather than failing the preview.
fn backdrop(
    root: &Path,
    wallpaper: &crate::desktop_preferences::Wallpaper,
    width: u32,
    height: u32,
) -> Result<RgbaImage, String> {
    use crate::desktop_preferences::WallpaperKind;
    let chosen = match wallpaper.kind {
        WallpaperKind::Color => {
            let hex = wallpaper.color.trim_start_matches('#');
            let channel = |at: usize| {
                hex.get(at..at + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            };
            if let (6, Some(r), Some(g), Some(b)) = (hex.len(), channel(0), channel(2), channel(4))
            {
                return Ok(RgbaImage::from_pixel(
                    width,
                    height,
                    raster::Rgba([r, g, b, 255]),
                ));
            }
            None
        }
        WallpaperKind::Image => wallpaper.image.as_ref().map(PathBuf::from),
        WallpaperKind::Live => wallpaper
            .scene
            .as_ref()
            .filter(|scene| {
                !scene.is_empty()
                    && scene.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
            .map(|scene| root.join("enzwall/scenes").join(scene).join("preview.png")),
        WallpaperKind::Default => None,
    };
    chosen
        .and_then(|path| wallpaper_thumbnail(&path, width, height).ok())
        .map_or_else(
            || wallpaper_thumbnail(&root.join("assets/wallpaper.png"), width, height),
            Ok,
        )
}

pub async fn capture(
    root: PathBuf,
    compositor: Compositor,
    desktops: Desktops,
    wallpaper: crate::desktop_preferences::Wallpaper,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<Preview>, String> {
    static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let _permit = GATE
        .acquire()
        .await
        .map_err(|_| tr("shell-preview-worker-closed"))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(tr("shell-preview-cancelled").into());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let output = compositor.desktop_output().await?;
    let monitor = output["id"]
        .as_i64()
        .ok_or_else(|| tr("shell-missing-preview-monitor"))?;
    let scale = output["scale"]
        .as_f64()
        .filter(|s| (0.5..=4.0).contains(s))
        .ok_or_else(|| tr("shell-invalid-preview-scale"))?;
    let width = output["width"]
        .as_f64()
        .filter(|w| (1.0..=16384.0).contains(w))
        .ok_or("Invalid preview width")?
        / scale;
    let height = output["height"]
        .as_f64()
        .filter(|h| (1.0..=16384.0).contains(h))
        .ok_or("Invalid preview height")?
        / scale;
    let origin = [
        output["x"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-monitor-position"))?,
        output["y"]
            .as_f64()
            .ok_or_else(|| tr("shell-missing-monitor-position"))?,
    ];
    let thumb_width = 320_u32;
    let thumb_height = (320.0 * height / width).round().clamp(64.0, 640.0) as u32;
    let cache = cache_root()?;
    std::fs::create_dir_all(&cache).map_err(|_| tr("shell-cannot-create-preview-cache"))?;
    let temporary = tempfile::Builder::new()
        .prefix("capture-")
        .tempdir_in(cache)
        .map_err(|_| tr("shell-cannot-create-private-preview-directory"))?;
    let backdrop_root = root.clone();
    let background = tokio::task::spawn_blocking(move || {
        backdrop(&backdrop_root, &wallpaper, thumb_width, thumb_height)
    })
    .await
    .map_err(|_| tr("shell-preview-renderer-stopped"))??;
    let mut canvases: BTreeMap<u32, (RgbaImage, usize)> = desktops
        .entries
        .iter()
        .map(|d| (d.id, (background.clone(), 0)))
        .collect();
    let mut unavailable: BTreeMap<u32, usize> = desktops
        .entries
        .iter()
        .map(|d| (d.id, d.windows as usize))
        .collect();
    let source = compositor.windows().await?;
    let mut attempts = 0;
    for (index, value) in source.iter().enumerate().take(128) {
        if cancelled.load(Ordering::Relaxed) {
            return Err(tr("shell-preview-cancelled").into());
        }
        let Some(window) = candidate(value, monitor).filter(|w| canvases.contains_key(&w.desktop))
        else {
            continue;
        };
        if attempts >= 24 || std::time::Instant::now() >= deadline {
            break;
        }
        attempts += 1;
        let path = temporary.path().join(format!("window-{index}.png"));
        // The window's size in the thumbnail.
        let w = (window.size[0] / width * f64::from(thumb_width))
            .round()
            .clamp(1.0, 2048.0) as u32;
        let h = (window.size[1] / height * f64::from(thumb_height))
            .round()
            .clamp(1.0, 2048.0) as u32;
        let remaining = deadline
            .saturating_duration_since(std::time::Instant::now())
            .min(Duration::from_secs(4));
        let helper = snapshot_helper(&root);
        let announced = helper.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || crate::compositor::announce_capture(&announced))
                .await,
            Ok(Ok(()))
        ) {
            continue;
        }
        let status = tokio::time::timeout(
            remaining,
            Command::new(helper)
                .args([
                    window.address.as_str(),
                    path.to_str()
                        .ok_or_else(|| tr("shell-invalid-preview-path"))?,
                ])
                .env(SNAPSHOT_FIT, format!("{w}x{h}"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
        if !matches!(status,Ok(Ok(status))if status.success()) {
            continue;
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(tr("shell-preview-cancelled").into());
        }
        let after = compositor.windows().await?;
        if after
            .iter()
            .find(|v| v["address"].as_str() == Some(window.address.as_str()))
            .and_then(|v| candidate(v, monitor))
            .as_ref()
            != Some(&window)
        {
            continue;
        }
        let decoded = tokio::task::spawn_blocking(move || {
            read_image(&path).map(|image| {
                if (image.width(), image.height()) == (w, h) {
                    image
                } else {
                    resize(&image, w, h, FilterType::Triangle)
                }
            })
        })
        .await
        .map_err(|_| tr("shell-preview-decoder-stopped"))?;
        let Ok(image) = decoded else { continue };
        let x = ((window.at[0] - origin[0]) / width * f64::from(thumb_width)).round() as i64;
        let y = ((window.at[1] - origin[1]) / height * f64::from(thumb_height)).round() as i64;
        if let Some((canvas, count)) = canvases.get_mut(&window.desktop) {
            overlay(canvas, &image, x, y);
            *count += 1;
        }
        if let Some(count) = unavailable.get_mut(&window.desktop) {
            *count = count.saturating_sub(1);
        }
    }
    Ok(canvases
        .into_iter()
        .map(|(desktop, (image, _))| Preview {
            desktop,
            image: Handle::from_rgba(image.width(), image.height(), image.into_raw()),
            unavailable: unavailable[&desktop],
        })
        .collect())
}
