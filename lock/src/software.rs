use enzui_shell::i18n::tr;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use iced_wgpu::graphics::Viewport;
use wayland_client::{
    QueueHandle,
    protocol::{wl_shm, wl_surface},
};

use crate::State;

pub(super) fn present(
    renderer: &mut iced_tiny_skia::Renderer,
    shm: &wl_shm::WlShm,
    surface: &wl_surface::WlSurface,
    qh: &QueueHandle<State>,
    viewport: &Viewport,
    transparent: bool,
    index: usize,
) -> Result<(), String> {
    let size = viewport.physical_size();
    let mut pixels = tiny_skia::Pixmap::new(size.width, size.height)
        .ok_or_else(|| tr("shell-cannot-allocate-software-frame"))?;
    let mut mask = tiny_skia::Mask::new(size.width, size.height)
        .ok_or_else(|| tr("shell-cannot-allocate-software-mask"))?;
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        viewport,
        &[iced::Rectangle::with_size(viewport.logical_size())],
        if transparent {
            iced::Color::TRANSPARENT
        } else {
            iced::Color::BLACK
        },
    );
    native_argb(pixels.data_mut());
    let bytes = pixels.data();
    let length = i32::try_from(bytes.len()).map_err(|_| tr("shell-software-frame-too-large"))?;
    let fd = rustix::fs::memfd_create("enzui-lock-frame", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|_| tr("shell-cannot-allocate-software-buffer"))?;
    rustix::fs::ftruncate(&fd, bytes.len() as u64)
        .map_err(|_| tr("shell-cannot-size-software-buffer"))?;
    let file = std::fs::File::from(fd);
    file.write_all_at(bytes, 0)
        .map_err(|_| tr("shell-cannot-write-software-buffer"))?;
    let pool = shm.create_pool(file.as_fd(), length, qh, ());
    let buffer = pool.create_buffer(
        0,
        size.width as i32,
        size.height as i32,
        size.width as i32 * 4,
        wl_shm::Format::Argb8888,
        qh,
        usize::MAX - index,
    );
    pool.destroy();
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, size.width as i32, size.height as i32);
    surface.commit();
    Ok(())
}

// ICED's TinySkia engine already writes BGRA for softbuffer. Wayland ARGB8888
// uses the same bytes on little-endian machines and native words on big-endian.
fn native_argb(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let word = u32::from_le_bytes(*pixel);
        pixel.copy_from_slice(&word.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn software_colors_match_wayland_argb_without_swapping_red_and_blue() {
        let mut renderer = iced_tiny_skia::Renderer::new(crate::renderer_settings());
        let mut pixels = tiny_skia::Pixmap::new(1, 1).unwrap();
        let mut mask = tiny_skia::Mask::new(1, 1).unwrap();
        let viewport = Viewport::with_physical_size(
            iced::Size::new(1, 1),
            iced_wgpu::core::renderer::Scale {
                window: 1.0,
                application: 1.0,
            },
        );
        renderer.draw(
            &mut pixels.as_mut(),
            &mut mask,
            &viewport,
            &[iced::Rectangle::with_size(iced::Size::new(1.0, 1.0))],
            iced::Color::from_rgb8(31, 73, 149),
        );
        native_argb(pixels.data_mut());
        assert_eq!(
            u32::from_ne_bytes(pixels.data()[..4].try_into().unwrap()),
            0xff1f4995
        );
    }
}
