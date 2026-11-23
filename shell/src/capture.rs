//! Screenshot encoding and capture preferences.
//! Window capture uses ext-foreign-toplevel; recording is delegated to wf-recorder.

use enzui_shell::i18n::tr;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}
impl ImageFormat {
    pub const ALL: [Self; 3] = [Self::Png, Self::Jpeg, Self::Webp];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => enzui_shell::i18n::tr("shell-webp-lossless"),
        }
    }
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    #[default]
    Mp4,
    Mkv,
    Webm,
}
impl Container {
    pub const ALL: [Self; 3] = [Self::Mp4, Self::Mkv, Self::Webm];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "Matroska (MKV)",
            Self::Webm => "WebM",
        }
    }
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
    /// The video codecs this container can hold.
    #[must_use]
    pub fn codecs(self) -> &'static [Codec] {
        match self {
            Self::Mp4 => &[Codec::H264, Codec::Hevc, Codec::Av1],
            Self::Mkv => &[Codec::H264, Codec::Hevc, Codec::Vp9, Codec::Av1],
            Self::Webm => &[Codec::Vp9, Codec::Av1],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    #[default]
    H264,
    Hevc,
    Vp9,
    Av1,
}
impl Codec {
    pub const ALL: [Self; 4] = [Self::H264, Self::Hevc, Self::Vp9, Self::Av1];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "H.265 (HEVC)",
            Self::Vp9 => "VP9",
            Self::Av1 => "AV1",
        }
    }
    /// The FFmpeg encoder `wf-recorder` is given.
    #[must_use]
    pub fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::Hevc => "libx265",
            Self::Vp9 => "libvpx-vp9",
            Self::Av1 => "libsvtav1",
        }
    }
    /// The constant-quality value for a quality level (lower is better).
    #[must_use]
    pub fn crf(self, quality: Quality) -> u8 {
        let (low, standard, high) = match self {
            Self::H264 => (28, 23, 18),
            Self::Hevc => (30, 26, 21),
            Self::Vp9 => (40, 33, 26),
            Self::Av1 => (42, 35, 28),
        };
        match quality {
            Quality::Low => low,
            Quality::Standard => standard,
            Quality::High => high,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Low,
    #[default]
    Standard,
    High,
}
impl Quality {
    pub const ALL: [Self; 3] = [Self::Low, Self::Standard, Self::High];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => tr("shell-smaller-files"),
            Self::Standard => tr("shell-standard"),
            Self::High => tr("shell-high-quality"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audio {
    #[default]
    None,
    Desktop,
    Microphone,
    DesktopAndMicrophone,
}
impl Audio {
    pub const ALL: [Self; 4] = [
        Self::None,
        Self::Desktop,
        Self::Microphone,
        Self::DesktopAndMicrophone,
    ];
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => tr("shell-no-sound"),
            Self::Desktop => tr("shell-desktop-sound"),
            Self::Microphone => tr("shell-microphone"),
            Self::DesktopAndMicrophone => tr("shell-desktop-sound-and-microphone"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub image_format: ImageFormat,
    /// 1–100.
    pub jpeg_quality: u8,
    /// 0 (fastest) – 9 (smallest).
    pub png_compression: u8,
    pub copy_to_clipboard: bool,
    pub save_screenshots: bool,
    pub show_pointer: bool,
    /// Absolute; empty is Pictures/Screenshots.
    pub screenshot_folder: String,
    pub container: Container,
    pub codec: Codec,
    /// 24, 30 or 60.
    pub framerate: u8,
    pub quality: Quality,
    pub audio: Audio,
    /// Absolute; empty is Videos/Screencasts.
    pub recording_folder: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            image_format: ImageFormat::Png,
            jpeg_quality: 90,
            png_compression: 6,
            copy_to_clipboard: true,
            save_screenshots: true,
            show_pointer: false,
            screenshot_folder: String::new(),
            container: Container::Mp4,
            codec: Codec::H264,
            framerate: 30,
            quality: Quality::Standard,
            audio: Audio::None,
            recording_folder: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    ImageFormat(ImageFormat),
    JpegQuality(u8),
    PngCompression(u8),
    CopyToClipboard(bool),
    SaveScreenshots(bool),
    ShowPointer(bool),
    ScreenshotFolder(String),
    Container(Container),
    Codec(Codec),
    Framerate(u8),
    Quality(Quality),
    Audio(Audio),
    RecordingFolder(String),
}

fn valid_folder(folder: &str) -> bool {
    folder.is_empty()
        || (folder.len() <= 4096
            && Path::new(folder).is_absolute()
            && !folder.chars().any(char::is_control))
}

impl Preferences {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-capture-preferences"))?,
            None => Self::default(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        let ok = (1..=100).contains(&self.jpeg_quality)
            && self.png_compression <= 9
            && matches!(self.framerate, 24 | 30 | 60)
            && self.container.codecs().contains(&self.codec)
            && valid_folder(&self.screenshot_folder)
            && valid_folder(&self.recording_folder)
            && (self.save_screenshots || self.copy_to_clipboard);
        if ok {
            Ok(())
        } else {
            Err(tr("shell-capture-preferences-exceed-their-bounds").into())
        }
    }
    pub fn apply(&mut self, change: Change) -> Result<(), String> {
        let mut next = self.clone();
        match change {
            Change::ImageFormat(format) => next.image_format = format,
            Change::JpegQuality(quality) => next.jpeg_quality = quality,
            Change::PngCompression(level) => next.png_compression = level,
            Change::CopyToClipboard(on) => next.copy_to_clipboard = on,
            Change::SaveScreenshots(on) => next.save_screenshots = on,
            Change::ShowPointer(on) => next.show_pointer = on,
            Change::ScreenshotFolder(folder) => next.screenshot_folder = folder,
            Change::Container(container) => {
                next.container = container;
                // A codec the new container cannot hold becomes its first one.
                if !container.codecs().contains(&next.codec) {
                    next.codec = container.codecs()[0];
                }
            }
            Change::Codec(codec) => next.codec = codec,
            Change::Framerate(rate) => next.framerate = rate,
            Change::Quality(quality) => next.quality = quality,
            Change::Audio(audio) => next.audio = audio,
            Change::RecordingFolder(folder) => next.recording_folder = folder,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn update_json(value: &mut serde_json::Value, change: Change) -> Result<(), String> {
        let mut next = Self::from_json(value.get("capture"))?;
        next.apply(change)?;
        value["capture"] = serde_json::to_value(next)
            .map_err(|_| tr("shell-cannot-encode-capture-preferences"))?;
        Ok(())
    }
    /// Where screenshots go: the chosen folder, else the desktop's.
    #[must_use]
    pub fn screenshot_dir(&self) -> Option<PathBuf> {
        if self.screenshot_folder.is_empty() {
            crate::system_controls::screenshot_directory()
        } else {
            Some(PathBuf::from(&self.screenshot_folder))
        }
    }
    #[must_use]
    pub fn recording_dir(&self) -> Option<PathBuf> {
        if self.recording_folder.is_empty() {
            crate::recording::directory()
        } else {
            Some(PathBuf::from(&self.recording_folder))
        }
    }
}

/// A rectangle in an output's logical pixels, from its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    /// The rectangle spanned by two corners, at least 1×1.
    #[must_use]
    pub fn spanning(a: (f32, f32), b: (f32, f32)) -> Self {
        let (x, y) = (a.0.min(b.0), a.1.min(b.1));
        Self {
            x,
            y,
            width: (a.0 - b.0).abs().max(1.0),
            height: (a.1 - b.1).abs().max(1.0),
        }
    }
    #[must_use]
    pub fn contains(&self, point: (f32, f32)) -> bool {
        point.0 >= self.x
            && point.1 >= self.y
            && point.0 < self.x + self.width
            && point.1 < self.y + self.height
    }
    /// Kept inside `width`×`height`.
    #[must_use]
    pub fn clamped(self, width: f32, height: f32) -> Self {
        let x = self.x.clamp(0.0, (width - 1.0).max(0.0));
        let y = self.y.clamp(0.0, (height - 1.0).max(0.0));
        Self {
            x,
            y,
            width: self.width.min(width - x).max(1.0),
            height: self.height.min(height - y).max(1.0),
        }
    }
    /// The same area in an image `scale` times the logical size.
    #[must_use]
    pub fn pixels(&self, scale: f64) -> (u32, u32, u32, u32) {
        let at = |v: f32| (f64::from(v) * scale).round().max(0.0) as u32;
        (
            at(self.x),
            at(self.y),
            at(self.width).max(1),
            at(self.height).max(1),
        )
    }
    /// `wf-recorder`'s `x,y WxH` in global logical coordinates.
    #[must_use]
    pub fn geometry(&self, origin: (i32, i32)) -> String {
        format!(
            "{},{} {}x{}",
            origin.0 + self.x.round() as i32,
            origin.1 + self.y.round() as i32,
            self.width.round().max(1.0) as i32,
            self.height.round().max(1.0) as i32
        )
    }
}

/// Encodes `image` as `preferences` asks.
pub fn encode(image: &raster::DynamicImage, preferences: &Preferences) -> Result<Vec<u8>, String> {
    use raster::ImageEncoder;
    let mut bytes = Vec::new();
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    match preferences.image_format {
        ImageFormat::Png => {
            use raster::codecs::png::{CompressionType, FilterType, PngEncoder};
            let compression = match preferences.png_compression {
                0..=2 => CompressionType::Fast,
                3..=6 => CompressionType::Default,
                _ => CompressionType::Best,
            };
            PngEncoder::new_with_quality(&mut bytes, compression, FilterType::Adaptive)
                .write_image(&rgba, width, height, raster::ExtendedColorType::Rgba8)
                .map_err(|_| tr("shell-the-screenshot-could-not-be-encoded"))?;
        }
        ImageFormat::Jpeg => {
            // JPEG has no alpha channel: transparent corners become white.
            let rgb = raster::RgbImage::from_fn(width, height, |x, y| {
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                let over = |c: u8| {
                    ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8
                };
                raster::Rgb([over(r), over(g), over(b)])
            });
            raster::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut bytes,
                preferences.jpeg_quality,
            )
            .write_image(&rgb, width, height, raster::ExtendedColorType::Rgb8)
            .map_err(|_| tr("shell-the-screenshot-could-not-be-encoded"))?;
        }
        ImageFormat::Webp => {
            raster::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
                .write_image(&rgba, width, height, raster::ExtendedColorType::Rgba8)
                .map_err(|_| tr("shell-the-screenshot-could-not-be-encoded"))?;
        }
    }
    Ok(bytes)
}

/// Makes pixels outside compositor-rounded corners transparent; `power` 2 gives circular
/// corners.
pub fn round_corners(image: &mut raster::RgbaImage, radius: f32, power: f32) {
    let (width, height) = image.dimensions();
    let radius = radius.min(width as f32 / 2.0).min(height as f32 / 2.0);
    if radius.is_nan() || radius < 1.0 || !power.is_finite() || power < 1.0 {
        return;
    }
    let span = radius.ceil() as u32;
    for y in (0..span).chain(height.saturating_sub(span).max(span)..height) {
        for x in (0..span).chain(width.saturating_sub(span).max(span)..width) {
            // Distance into the corner, from the centre of its rounding.
            let dx = (radius - (x.min(width - 1 - x) as f32 + 0.5)).max(0.0);
            let dy = (radius - (y.min(height - 1 - y) as f32 + 0.5)).max(0.0);
            let distance = (dx.powf(power) + dy.powf(power)).powf(1.0 / power);
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let pixel = image.get_pixel_mut(x, y);
            pixel.0[3] = (f32::from(pixel.0[3]) * coverage).round() as u8;
        }
    }
}

/// Encodes PNG at the default level, for the clipboard.
pub fn png(image: &raster::DynamicImage) -> Result<Vec<u8>, String> {
    encode(
        image,
        &Preferences {
            image_format: ImageFormat::Png,
            png_compression: 3,
            ..Preferences::default()
        },
    )
}

/// A new file in `directory` named after the time, created only by us.
pub fn new_file(
    directory: &Path,
    stem: &str,
    extension: &str,
) -> Result<(PathBuf, std::fs::File), String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(directory).map_err(|_| tr("shell-the-folder-could-not-be-created"))?;

    let base = format!(
        "{stem} from {}",
        chrono::Local::now().format("%Y-%m-%d %H-%M-%S")
    );
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("{base}.{extension}")
        } else {
            format!("{base} ({attempt}).{extension}")
        };
        let path = directory.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(tr("shell-the-file-could-not-be-created").into());
            }
        }
    }
    Err(tr("shell-too-many-files-with-this-name").into())
}

/// What `grim` captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Output(String),
    /// A window by its `ext-foreign-toplevel` identifier (Hyprland's
    /// `stableId`): only its own content, even where it is covered.
    Window(String),
}

/// The image `grim` captures from `source`, pointer included if asked.
pub async fn grab(
    tools: &crate::system_controls::SystemControls,
    source: &Source,
    pointer: bool,
) -> Result<raster::DynamicImage, String> {
    let (flag, name) = match source {
        Source::Output(name) => ("-o", name),
        Source::Window(id) => ("-T", id),
    };
    if name.is_empty()
        || name.len() > 180
        || name.starts_with("ENZREMOTE-")
        || name.starts_with('-')
        || name.chars().any(char::is_control)
    {
        return Err(tr("shell-invalid-capture-source").into());
    }
    let tool = tools
        .tool("grim")
        .ok_or_else(|| tr("shell-grim-is-not-installed"))?;
    let announced = tool.clone();
    tokio::task::spawn_blocking(move || crate::compositor::announce_capture(&announced))
        .await
        .map_err(|_| tr("shell-the-screen-could-not-be-captured"))??;
    let mut command = tokio::process::Command::new(tool);
    // Uncompressed: it is decoded at once.
    command.args(["-t", "png", "-l", "0"]);
    if pointer {
        command.arg("-c");
    }
    command
        .args([flag, name.as_str(), "-"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .map_err(|_| tr("shell-the-screen-could-not-be-captured-in-time"))?
        .map_err(|_| tr("shell-the-screen-could-not-be-captured"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(match source {
            Source::Output(_) => tr("shell-the-screen-could-not-be-captured"),
            Source::Window(_) => tr("shell-the-window-could-not-be-captured"),
        }
        .into());
    }
    tokio::task::spawn_blocking(move || {
        raster::load_from_memory_with_format(&output.stdout, raster::ImageFormat::Png)
    })
    .await
    .map_err(|_| tr("shell-the-capture-could-not-be-read"))?
    .map_err(|_| tr("shell-the-capture-could-not-be-read").into())
}

/// Where a screenshot went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub path: Option<PathBuf>,
    pub copied: bool,
    pub(crate) identity: Option<crate::search::catalogue::FileIdentity>,
}

/// Saves and copies `image` as `preferences` ask (blocking).
pub fn keep(
    image: &raster::DynamicImage,
    preferences: &Preferences,
    wl_copy: Option<&Path>,
) -> Result<Saved, String> {
    use std::io::Write;
    let (path, identity) = if preferences.save_screenshots {
        let bytes = encode(image, preferences)?;
        let folder = preferences
            .screenshot_dir()
            .ok_or_else(|| tr("shell-no-folder-for-screenshots"))?;
        let (path, mut file) =
            new_file(&folder, "Screenshot", preferences.image_format.extension())?;
        if file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .is_err()
        {
            let _ = std::fs::remove_file(&path);
            return Err(tr("shell-the-screenshot-could-not-be-saved").into());
        }
        let identity = file
            .metadata()
            .ok()
            .map(|metadata| crate::search::catalogue::FileIdentity::from_metadata(&metadata));
        (Some(path), identity)
    } else {
        (None, None)
    };
    let copied = preferences.copy_to_clipboard
        && wl_copy.is_some_and(|tool| png(image).is_ok_and(|bytes| copy_png(tool, &bytes).is_ok()));
    if path.is_none() && !copied {
        return Err(tr("shell-the-screenshot-could-not-be-copied").into());
    }
    Ok(Saved {
        path,
        copied,
        identity,
    })
}

/// The part of `image` inside `area` (logical pixels of an output whose
/// logical width is `logical_width`).
#[must_use]
pub fn crop(image: &raster::DynamicImage, area: Rect, logical_width: f32) -> raster::DynamicImage {
    let scale = f64::from(image.width()) / f64::from(logical_width.max(1.0));
    let (x, y, width, height) = area.pixels(scale);
    let x = x.min(image.width().saturating_sub(1));
    let y = y.min(image.height().saturating_sub(1));
    image.crop_imm(
        x,
        y,
        width.min(image.width() - x).max(1),
        height.min(image.height() - y).max(1),
    )
}

/// The clipboard tool screenshots are copied with.
#[must_use]
pub fn clipboard_tool(tools: &crate::system_controls::SystemControls) -> Option<PathBuf> {
    tools.tool("wl-copy")
}

// wl-copy retains clipboard ownership after this function returns.
fn copy_png(tool: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new(tool)
        .args(["--type", "image/png"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| tr("shell-wl-copy-could-not-start"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| tr("shell-wl-copy-is-not-available"))?
        .write_all(bytes)
        .map_err(|_| tr("shell-the-screenshot-could-not-be-copied"))?;
    // wl-copy forks to keep serving; the parent exits at once.
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        _ => Err(tr("shell-the-screenshot-could-not-be-copied").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_behaviour() {
        // Preferences stay in bounds and codecs follow the container.
        {
            let mut preferences = Preferences::default();
            assert!(preferences.apply(Change::JpegQuality(0)).is_err());
            assert!(preferences.apply(Change::Framerate(45)).is_err());
            assert!(
                preferences
                    .apply(Change::ScreenshotFolder("relative".into()))
                    .is_err()
            );
            preferences.apply(Change::SaveScreenshots(false)).unwrap();
            assert!(
                preferences.apply(Change::CopyToClipboard(false)).is_err(),
                "a screenshot must go somewhere"
            );
            preferences.apply(Change::Codec(Codec::Hevc)).unwrap();
            preferences
                .apply(Change::Container(Container::Webm))
                .unwrap();
            assert_eq!(preferences.codec, Codec::Vp9, "WebM cannot hold HEVC");
            assert!(preferences.apply(Change::Codec(Codec::H264)).is_err());
            let mut json = serde_json::json!({});
            Preferences::update_json(&mut json, Change::ImageFormat(ImageFormat::Jpeg)).unwrap();
            assert_eq!(
                Preferences::from_json(json.get("capture"))
                    .unwrap()
                    .image_format,
                ImageFormat::Jpeg
            );
        }
        // Areas map to pixels and recorder geometry.
        {
            let area = Rect::spanning((300.0, 200.0), (100.0, 50.0));
            assert_eq!(
                (area.x, area.y, area.width, area.height),
                (100.0, 50.0, 200.0, 150.0)
            );
            assert_eq!(area.pixels(1.5), (150, 75, 300, 225));
            assert_eq!(area.geometry((1920, 0)), "2020,50 200x150");
            let clamped = Rect {
                x: 1200.0,
                y: 700.0,
                width: 400.0,
                height: 300.0,
            }
            .clamped(1280.0, 800.0);
            assert_eq!((clamped.width, clamped.height), (80.0, 100.0));
        }
        // Images encode in each format.
        {
            let image = raster::DynamicImage::new_rgba8(8, 6);
            for format in ImageFormat::ALL {
                let bytes = encode(
                    &image,
                    &Preferences {
                        image_format: format,
                        ..Preferences::default()
                    },
                )
                .unwrap();
                let decoded = raster::load_from_memory(&bytes).unwrap();
                assert_eq!((decoded.width(), decoded.height()), (8, 6), "{format:?}");
            }
            let mut window = raster::RgbaImage::from_pixel(40, 30, raster::Rgba([10, 20, 30, 255]));
            round_corners(&mut window, 8.0, 2.0);
            assert_eq!(window.get_pixel(0, 0).0[3], 0, "the corner is transparent");
            assert_eq!(window.get_pixel(39, 29).0[3], 0);
            assert_eq!(
                window.get_pixel(20, 0).0[3],
                255,
                "edges between the corners stay"
            );
            assert_eq!(window.get_pixel(20, 15).0[3], 255);
            let partial = window.get_pixel(2, 2).0[3];
            assert!(
                partial > 0 && partial < 255,
                "the curve is antialiased: {partial}"
            );
            let jpeg = encode(
                &raster::DynamicImage::ImageRgba8(window),
                &Preferences {
                    image_format: ImageFormat::Jpeg,
                    jpeg_quality: 100,
                    ..Preferences::default()
                },
            )
            .unwrap();
            let corner = raster::load_from_memory(&jpeg)
                .unwrap()
                .to_rgb8()
                .get_pixel(0, 0)
                .0;
            assert!(
                corner.iter().all(|c| *c > 240),
                "a JPEG corner is white, not black: {corner:?}"
            );
            let wide = raster::DynamicImage::new_rgba8(200, 100);
            let part = crop(
                &wide,
                Rect {
                    x: 50.0,
                    y: 10.0,
                    width: 80.0,
                    height: 200.0,
                },
                100.0,
            );
            assert_eq!(
                (part.width(), part.height()),
                (100, 80),
                "scaled by two and kept inside"
            );
            let directory = tempfile::tempdir().unwrap();
            let preferences = Preferences {
                screenshot_folder: directory.path().display().to_string(),
                copy_to_clipboard: false,
                ..Preferences::default()
            };
            let saved = keep(&part, &preferences, None).unwrap();
            assert_eq!(raster::open(saved.path.unwrap()).unwrap().width(), 100);
            let (first, _) = new_file(directory.path(), "Screenshot", "png").unwrap();
            let (second, _) = new_file(directory.path(), "Screenshot", "png").unwrap();
            assert_ne!(first, second);
        }
    }
}
