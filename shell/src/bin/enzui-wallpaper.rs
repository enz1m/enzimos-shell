//! Renders image or colour wallpaper surfaces for the desktop.
use enzui_shell::i18n::{self, tr};
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use raster::{RgbaImage, imageops};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_region, wl_shm, wl_surface},
};

const FADE: Duration = Duration::from_millis(280);

struct Preferences {
    color: [u8; 3],
    image: Option<PathBuf>,
    fit: String,
    animate: bool,
}

impl Preferences {
    fn parse() -> Result<Self, String> {
        let mut args = env::args().skip(1);
        let color = args
            .next()
            .ok_or_else(|| tr("shell-missing-wallpaper-color"))?;
        let fit = args
            .next()
            .ok_or_else(|| tr("shell-missing-wallpaper-image-fit"))?;
        let image = args
            .next()
            .ok_or_else(|| tr("shell-missing-wallpaper-image-path"))?;
        let animate = args
            .next()
            .ok_or_else(|| tr("shell-missing-wallpaper-fade-mode"))?;
        if args.next().is_some()
            || !["fill", "fit", "center", "stretch", "tile"].contains(&fit.as_str())
        {
            return Err(tr("shell-invalid-wallpaper-arguments").into());
        }
        let bytes = color.as_bytes();
        if bytes.len() != 6 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(tr("shell-invalid-wallpaper-color").into());
        }
        let mut parsed = [0; 3];
        for (channel, pair) in parsed.iter_mut().zip(bytes.as_chunks::<2>().0.iter()) {
            let pair =
                std::str::from_utf8(pair).map_err(|_| tr("shell-invalid-wallpaper-color"))?;
            *channel =
                u8::from_str_radix(pair, 16).map_err(|_| tr("shell-invalid-wallpaper-color"))?;
        }
        let color = parsed;
        let image = (image != "-").then(|| PathBuf::from(image));
        if image.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(tr("shell-wallpaper-image-must-be-absolute").into());
        }
        let animate = match animate.as_str() {
            "fade" => true,
            "immediate" => false,
            _ => return Err(tr("shell-invalid-wallpaper-fade-mode").into()),
        };
        Ok(Self {
            color,
            image,
            fit,
            animate,
        })
    }
}

fn pixels(settings: &Preferences, width: u32, height: u32) -> Result<Vec<[u8; 3]>, String> {
    let count = width
        .checked_mul(height)
        .filter(|size| *size <= 16 * 1024 * 1024)
        .ok_or_else(|| tr("shell-wallpaper-output-is-too-large"))? as usize;
    let mut result = vec![settings.color; count];
    let Some(path) = &settings.image else {
        return Ok(result);
    };
    let image = raster::open(path).map_err(|_| tr("shell-wallpaper-image-could-not-be-decoded"))?;
    if image.width() == 0
        || image.height() == 0
        || image.width() > 4096
        || image.height() > 4096
        || image.width() * image.height() > 16 * 1024 * 1024
    {
        return Err(tr("shell-wallpaper-image-dimensions-exceed-their-bounds").into());
    }
    let image = image.to_rgba8();
    let mut canvas = RgbaImage::from_pixel(
        width,
        height,
        raster::Rgba([settings.color[0], settings.color[1], settings.color[2], 255]),
    );
    match settings.fit.as_str() {
        "fill" => imageops::overlay(
            &mut canvas,
            &raster::DynamicImage::ImageRgba8(image)
                .resize_to_fill(width, height, imageops::FilterType::Triangle)
                .to_rgba8(),
            0,
            0,
        ),
        "fit" => {
            let sized = raster::DynamicImage::ImageRgba8(image)
                .resize(width, height, imageops::FilterType::Triangle)
                .to_rgba8();
            let x = (i64::from(width) - i64::from(sized.width())) / 2;
            let y = (i64::from(height) - i64::from(sized.height())) / 2;
            imageops::overlay(&mut canvas, &sized, x, y);
        }
        "center" => imageops::overlay(
            &mut canvas,
            &image,
            (i64::from(width) - i64::from(image.width())) / 2,
            (i64::from(height) - i64::from(image.height())) / 2,
        ),
        "stretch" => imageops::overlay(
            &mut canvas,
            &imageops::resize(&image, width, height, imageops::FilterType::Triangle),
            0,
            0,
        ),
        "tile" => {
            for y in (0..height).step_by(image.height() as usize) {
                for x in (0..width).step_by(image.width() as usize) {
                    imageops::overlay(&mut canvas, &image, i64::from(x), i64::from(y));
                }
            }
        }
        _ => unreachable!(),
    }
    for (dest, pixel) in result.iter_mut().zip(canvas.pixels()) {
        let a = u32::from(pixel[3]);
        for channel in 0..3 {
            dest[channel] = ((u32::from(pixel[channel]) * a
                + u32::from(settings.color[channel]) * (255 - a)
                + 127)
                / 255) as u8;
        }
    }
    Ok(result)
}

fn packed_pixel(rgb: [u8; 3], opacity: u8) -> [u8; 4] {
    let [r, g, b] =
        rgb.map(|component| ((u16::from(component) * u16::from(opacity) + 127) / 255) as u8);
    [b, g, r, opacity]
}

struct Screen {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    scale: i32,
    pixels: Vec<[u8; 3]>,
    size: (u32, u32),
    started: Option<Instant>,
    opaque_ack_requested: bool,
    completed: bool,
}

struct Wallpaper {
    registry: RegistryState,
    output: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    settings: Preferences,
    screens: Vec<Screen>,
    announced: bool,
    closed: bool,
}

impl Wallpaper {
    fn add(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = self.output.info(&output);
        // Remote-session app outputs are streamed, never part of the desktop.
        if info
            .as_ref()
            .and_then(|i| i.name.as_deref())
            .is_some_and(|name| name.starts_with("ENZREMOTE-"))
            || self.screens.iter().any(|s| s.output == output)
        {
            return;
        }
        let scale = info.map_or(1, |i| i.scale_factor.clamp(1, 4));
        let surface = self.compositor.create_surface(qh);
        let region = self.compositor.wl_compositor().create_region(qh, ());
        surface.set_input_region(Some(&region));
        surface.set_buffer_scale(scale);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("enzui-wallpaper"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        self.screens.push(Screen {
            output,
            layer,
            scale,
            pixels: Vec::new(),
            size: (0, 0),
            started: None,
            opaque_ack_requested: false,
            completed: false,
        });
    }

    fn render(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let screen = &mut self.screens[index];
        let (width, height) = (
            screen.size.0 * screen.scale as u32,
            screen.size.1 * screen.scale as u32,
        );
        screen.pixels =
            pixels(&self.settings, width, height).expect("Wallpaper image could not be rendered");
        screen.started = None;
        screen.opaque_ack_requested = false;
        screen.completed = false;
        self.draw(index, qh);
    }

    fn draw(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let screen = &mut self.screens[index];
        let (width, height) = (
            screen.size.0 * screen.scale as u32,
            screen.size.1 * screen.scale as u32,
        );
        let elapsed = screen.started.get_or_insert_with(Instant::now).elapsed();
        let opacity = if self.settings.animate {
            ((elapsed.as_secs_f32() / FADE.as_secs_f32()).min(1.0) * 255.0)
                .round()
                .max(1.0) as u8
        } else {
            255
        };
        let stride = width as i32 * 4;
        let (buffer, data) = self
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("Wallpaper frame buffer");
        for (pixel, rgb) in data.as_chunks_mut::<4>().0.iter_mut().zip(&screen.pixels) {
            pixel.copy_from_slice(&packed_pixel(*rgb, opacity));
        }
        let surface = screen.layer.wl_surface();
        surface.set_buffer_scale(screen.scale);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if opacity < 255 || !screen.opaque_ack_requested {
            surface.frame(qh, surface.clone());
            if opacity == 255 {
                screen.opaque_ack_requested = true;
            }
        }
        buffer
            .attach_to(surface)
            .expect("Wallpaper frame attachment");
        screen.layer.commit();
    }

    fn announce(&mut self) {
        if !self.announced && !self.screens.is_empty() && self.screens.iter().all(|s| s.completed) {
            std::io::stdout()
                .write_all(b"ready\n")
                .expect("Wallpaper presentation acknowledgement");
            self.announced = true;
        }
    }

    fn screen(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.screens
            .iter()
            .position(|s| s.layer.wl_surface() == surface)
    }
}

impl CompositorHandler for Wallpaper {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if let Some(index) = self.screen(surface) {
            let factor = factor.clamp(1, 4);
            if self.screens[index].scale != factor {
                self.screens[index].scale = factor;
                if self.screens[index].size != (0, 0) {
                    self.render(index, qh);
                }
            }
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(index) = self.screen(surface) else {
            return;
        };
        if self.screens[index].opaque_ack_requested {
            self.screens[index].completed = true;
            self.announce();
        } else {
            self.draw(index, qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Wallpaper {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add(qh, output);
    }
    fn update_output(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // The name arrives with the output's first description.
        self.add(qh, output);
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.screens.retain(|s| s.output != output);
    }
}

impl ShmHandler for Wallpaper {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for Wallpaper {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.screens.retain(|s| &s.layer != layer);
        if self.screens.is_empty() {
            self.closed = true;
        }
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let size = configure.new_size;
        let Some(index) = self.screens.iter().position(|s| &s.layer == layer) else {
            return;
        };
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        if size != self.screens[index].size {
            self.screens[index].size = size;
            self.render(index, qh);
        } else {
            self.draw(index, qh);
        }
    }
}

delegate_compositor!(Wallpaper);
delegate_output!(Wallpaper);
delegate_shm!(Wallpaper);
delegate_layer!(Wallpaper);
delegate_registry!(Wallpaper);

impl ProvidesRegistryState for Wallpaper {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

impl Dispatch<wl_region::WlRegion, ()> for Wallpaper {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    i18n::initialize();
    let settings = Preferences::parse()?;
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init(&connection)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let mut state = Wallpaper {
        registry: RegistryState::new(&globals),
        output: OutputState::new(&globals, &qh),
        pool: SlotPool::new(4, &shm)?,
        compositor,
        layer_shell,
        shm,
        settings,
        screens: Vec::new(),
        announced: false,
        closed: false,
    };
    while !state.closed {
        queue.blocking_dispatch(&mut state)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enzui_wallpaper_behaviour() {
        // Premultiplied alpha keeps transitions translucent.
        {
            assert_eq!(packed_pixel([100, 200, 50], 128), [25, 100, 50, 128]);
            assert_eq!(packed_pixel([100, 200, 50], 255), [50, 200, 100, 255]);
        }
        // Picture modes fill tile and letterbox against the chosen color.
        {
            let directory = tempfile::tempdir().unwrap();
            let image = directory.path().join("wallpaper.png");
            RgbaImage::from_pixel(2, 2, raster::Rgba([200, 80, 40, 255]))
                .save(&image)
                .unwrap();
            let mut preferences = Preferences {
                color: [4, 5, 6],
                image: Some(image),
                fit: "center".into(),
                animate: false,
            };
            let centered = pixels(&preferences, 6, 4).unwrap();
            assert_eq!(centered[0], [4, 5, 6]);
            assert_eq!(centered[6 + 2], [200, 80, 40]);
            preferences.fit = "tile".into();
            assert!(
                pixels(&preferences, 6, 4)
                    .unwrap()
                    .iter()
                    .all(|pixel| *pixel == [200, 80, 40])
            );
            preferences.fit = "stretch".into();
            assert!(
                pixels(&preferences, 6, 4)
                    .unwrap()
                    .iter()
                    .all(|pixel| *pixel == [200, 80, 40])
            );
            preferences.fit = "fit".into();
            assert_eq!(pixels(&preferences, 6, 4).unwrap()[0], [4, 5, 6]);
            preferences.fit = "fill".into();
            assert_eq!(pixels(&preferences, 6, 4).unwrap()[0], [200, 80, 40]);
        }
        // Solid colors are filled across the entire output.
        {
            let preferences = Preferences {
                color: [7, 8, 9],
                image: None,
                fit: "fill".into(),
                animate: true,
            };
            assert_eq!(pixels(&preferences, 3, 2).unwrap(), vec![[7, 8, 9]; 6]);
            assert!(pixels(&preferences, 4096, 4097).is_err());
        }
    }
}
