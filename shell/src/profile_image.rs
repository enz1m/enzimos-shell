use enzui_shell::i18n::tr;
use std::{
    io::{BufReader, Read},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

pub fn decode(path: &Path) -> Result<raster::RgbaImage, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| tr("shell-cannot-open-image"))?;
    let meta = file.metadata().map_err(|_| tr("shell-cannot-read-image"))?;
    if !meta.is_file() || meta.len() > 32 * 1024 * 1024 {
        return Err(tr("shell-choose-a-regular-image-smaller-than-32-mib").into());
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| tr("shell-cannot-read-image"))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(tr("shell-image-is-too-large").into());
    }
    let mut reader = raster::ImageReader::new(BufReader::new(std::io::Cursor::new(bytes)))
        .with_guessed_format()
        .map_err(|_| tr("shell-unknown-image-format"))?;
    let mut limits = raster::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|i| i.to_rgba8())
        .map_err(|_| tr("shell-image-exceeds-its-decoding-limits-or-is-damaged").into())
}

pub fn handle(path: &Path) -> Option<iced::widget::image::Handle> {
    cached(path, false)
}

pub fn avatar(path: &Path) -> Option<iced::widget::image::Handle> {
    cached(path, true)
}

fn avatar_thumbnail(image: &raster::RgbaImage) -> raster::RgbaImage {
    let side = image.width().min(image.height());
    let crop = raster::imageops::crop_imm(
        image,
        (image.width() - side) / 2,
        (image.height() - side) / 2,
        side,
        side,
    );
    raster::imageops::resize(
        &crop.to_image(),
        256,
        256,
        raster::imageops::FilterType::Triangle,
    )
}

fn cached(path: &Path, avatar: bool) -> Option<iced::widget::image::Handle> {
    use std::sync::Mutex;
    type Cached = (
        std::path::PathBuf,
        std::time::SystemTime,
        u64,
        bool,
        iced::widget::image::Handle,
    );
    static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().ok()?;
    let mut cache = CACHE.lock().ok()?;
    if let Some(c) = cache
        .iter()
        .find(|c| c.0 == path && c.1 == modified && c.2 == meta.len() && c.3 == avatar)
    {
        return Some(c.4.clone());
    }
    let image = decode(path).ok()?;
    let image = if avatar {
        avatar_thumbnail(&image)
    } else {
        raster::imageops::thumbnail(&image, 256, 256)
    };
    let handle =
        iced::widget::image::Handle::from_rgba(image.width(), image.height(), image.into_raw());
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.retain(|c| c.0 != path || c.3 != avatar);
    cache.push((path.into(), modified, meta.len(), avatar, handle.clone()));
    Some(handle)
}
