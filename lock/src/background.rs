//! The lock/login backdrop: an embedded default scene, with personal image,
//! solid-colour and live wallpaper choices respected.

use enzui_shell::desktop_preferences::{Wallpaper, WallpaperKind};
use raster::{RgbaImage, imageops};
use std::path::Path;

/// Premultiplication is not needed: every backdrop is opaque.
pub(crate) struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Picture {
    fn from(image: RgbaImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        }
    }
    pub(crate) fn solid(width: u32, height: u32, color: [u8; 3]) -> Self {
        let pixel = [color[0], color[1], color[2], 255];
        Self {
            width,
            height,
            rgba: pixel.repeat((width * height) as usize),
        }
    }
}

pub(crate) struct Backdrop {
    pub sharp: Picture,
    /// A small, blurred copy; drawn scaled up behind the password prompt.
    pub soft: Picture,
    /// Live scenes draw their own clock, so the lock screen does not add one.
    pub has_clock: bool,
    pub light: bool,
}

fn hex(color: &str) -> [u8; 3] {
    let value = u32::from_str_radix(color.trim_start_matches('#'), 16).unwrap_or(0x172b40);
    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
}

fn cover(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if image.width() == width && image.height() == height {
        return image.clone();
    }
    let aspect = f64::from(width.max(1)) / f64::from(height.max(1));
    let source_aspect = f64::from(image.width()) / f64::from(image.height().max(1));
    let (crop_w, crop_h) = if source_aspect > aspect {
        (
            (f64::from(image.height()) * aspect).round().max(1.0) as u32,
            image.height(),
        )
    } else {
        (
            image.width(),
            (f64::from(image.width()) / aspect).round().max(1.0) as u32,
        )
    };
    let cropped = imageops::crop_imm(
        image,
        (image.width() - crop_w) / 2,
        (image.height() - crop_h) / 2,
        crop_w,
        crop_h,
    );
    imageops::resize(
        &cropped.to_image(),
        width,
        height,
        imageops::FilterType::Triangle,
    )
}

pub(crate) fn soften(image: &RgbaImage) -> Picture {
    let small = imageops::resize(
        image,
        (image.width() / 16).max(8),
        (image.height() / 16).max(8),
        imageops::FilterType::Triangle,
    );
    Picture::from(imageops::fast_blur(&small, 2.5))
}

fn flatten(image: &mut RgbaImage, color: [u8; 3]) {
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for (channel, background) in pixel.0[..3].iter_mut().zip(color) {
            *channel = ((u32::from(*channel) * alpha + u32::from(background) * (255 - alpha) + 127)
                / 255) as u8;
        }
        pixel[3] = 255;
    }
}

fn image_file(path: &Path, width: u32, height: u32) -> Option<RgbaImage> {
    let image = enzui_shell::profile_image::decode(path).ok()?;
    Some(cover(&image, width, height))
}

// Builds the backdrop for one output size, falling back to the wallpaper
// colour whenever a file cannot be read.
pub(crate) fn load(wallpaper: &Wallpaper, width: u32, height: u32) -> Backdrop {
    let default = || {
        std::env::var_os("ENZUI_DESKTOP_ROOT")
            .and_then(|root| {
                image_file(
                    &Path::new(&root).join("assets/wallpaper.png"),
                    width,
                    height,
                )
            })
            .or_else(|| {
                raster::load_from_memory(include_bytes!("../../assets/wallpaper.png"))
                    .ok()
                    .map(|image| cover(&image.to_rgba8(), width, height))
            })
    };
    let (image, has_clock) = match wallpaper.kind {
        WallpaperKind::Color => (None, false),
        WallpaperKind::Image => (
            wallpaper
                .image
                .as_deref()
                .and_then(|p| image_file(Path::new(p), width, height)),
            false,
        ),
        WallpaperKind::Default => (default(), false),
        WallpaperKind::Live => (default(), false),
    };
    match image {
        Some(mut image) => {
            flatten(&mut image, hex(&wallpaper.color));
            Backdrop {
                light: luma(&image) > 0.65,
                soft: soften(&image),
                sharp: Picture::from(image),
                has_clock,
            }
        }
        None => {
            let color = hex(&wallpaper.color);
            Backdrop {
                sharp: Picture::solid(width, height, color),
                soft: Picture::solid(8, 8, color),
                has_clock: false,
                light: (0.299 * f32::from(color[0])
                    + 0.587 * f32::from(color[1])
                    + 0.114 * f32::from(color[2]))
                    / 255.0
                    > 0.65,
            }
        }
    }
}

pub(crate) fn selected(
    wallpaper: &Wallpaper,
    clock: &enzui_shell::session_preferences::LockClock,
) -> Wallpaper {
    let mut selected = wallpaper.clone();
    if let Some(path) = &clock.background {
        selected.kind = WallpaperKind::Image;
        selected.image = Some(path.clone());
    }
    selected
}

pub(crate) fn luma(image: &RgbaImage) -> f32 {
    let mut sum = 0.0;
    for y in 0..3 {
        for x in 0..4 {
            let p = image.get_pixel(
                ((2 * x + 1) * image.width() / 8).min(image.width() - 1),
                ((2 * y + 1) * image.height() / 6).min(image.height() - 1),
            );
            sum += (0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
                / 255.0;
        }
    }
    sum / 12.0
}
