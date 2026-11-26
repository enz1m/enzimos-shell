use super::{Result, broker::Cancel, unavailable};
use crate::i18n::tr;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, channel},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use tokio::sync::oneshot;
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

const NAMESPACE: &str = "enzui-assistant-control";
const WIDTH: i32 = 265;
const HEIGHT: i32 = 34;
const FONT: &[u8] = include_bytes!("../../assets/lock-fonts/Nunito-800.ttf");

struct Surface {
    layer: LayerSurface,
    presented: bool,
}

struct Bar {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    pointer: Option<wl_pointer::WlPointer>,
    shm: Shm,
    pool: SlotPool,
    label: String,
    surfaces: Vec<Surface>,
    ready: Option<oneshot::Sender<()>>,
    sealed: bool,
    stopped: bool,
    cancel: Cancel,
}

impl Bar {
    fn stop(&mut self) {
        self.cancel.revoke();
        self.stopped = true;
    }
}

pub(super) struct Indicator(channel::Sender<()>);

impl Indicator {
    pub(super) fn start(cancel: Cancel) -> (Self, oneshot::Receiver<()>) {
        let (stop, stopping) = channel::channel();
        let (placed, ready) = oneshot::channel();
        let label = tr("shell-assistant-control-indicator").to_owned();
        std::thread::spawn(move || {
            let result = run(cancel.clone(), stopping, placed, label);
            cancel.revoke();
            if let Err(error) = result {
                eprintln!("enzui-shell: Assistant indicator: {error}");
            }
        });
        (Self(stop), ready)
    }
}

impl Drop for Indicator {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn run(
    cancel: Cancel,
    stop: channel::Channel<()>,
    ready: oneshot::Sender<()>,
    label: String,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Bar>(&connection)?;
    let qh = queue.handle();
    let layers = LayerShell::bind(&globals, &qh)?;
    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm)?;
    let mut bar = Bar {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        pointer: None,
        shm,
        pool,
        label,
        surfaces: Vec::new(),
        ready: Some(ready),
        sealed: false,
        stopped: false,
        cancel,
    };
    queue.roundtrip(&mut bar)?;
    let outputs: Vec<_> = bar.outputs.outputs().take(17).collect();
    if outputs.is_empty() || outputs.len() > 16 || bar.pointer.is_none() {
        return Err(unavailable().into());
    }
    bar.sealed = true;
    for output in outputs {
        let layer = layers.create_layer_surface(
            &qh,
            compositor.create_surface(&qh),
            Layer::Overlay,
            Some(NAMESPACE),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP);
        layer.set_margin(8, 0, 0, 0);
        layer.set_exclusive_zone(-1);
        layer.set_size(WIDTH as u32, HEIGHT as u32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        bar.surfaces.push(Surface {
            layer,
            presented: false,
        });
    }
    let mut loop_: EventLoop<Bar> = EventLoop::try_new()?;
    loop_
        .handle()
        .insert_source(stop, |_, (), bar| bar.stopped = true)
        .map_err(|error| error.error)?;
    WaylandSource::new(connection, queue).insert(loop_.handle())?;
    while !bar.stopped {
        loop_.dispatch(None, &mut bar)?;
    }
    Ok(())
}

fn draw(canvas: &mut [u8], label: &str, scale: i32) -> Result<()> {
    let font = FontRef::try_from_slice(FONT).map_err(|_| unavailable())?;
    let mut font = font.as_scaled(PxScale::from(15.5 * scale as f32));
    let width = |font: &ab_glyph::PxScaleFont<&FontRef<'_>>| {
        label
            .chars()
            .map(|ch| font.h_advance(font.glyph_id(ch)))
            .sum::<f32>()
    };
    let panel_width = (WIDTH * scale) as f32;
    let text_width = width(&font);
    if text_width > panel_width - 20.0 {
        font.scale = PxScale::from(font.scale.x * (panel_width - 20.0) / text_width);
    }
    let mut x = (panel_width - width(&font)) / 2.0;
    let y = ((HEIGHT * scale) as f32 + font.ascent() + font.descent()) / 2.0;
    let background = 0xff852823_u32.to_ne_bytes();
    for pixel in canvas.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&background);
    }
    for ch in label.chars() {
        let id = font.glyph_id(ch);
        if id.0 == 0 {
            return Err(unavailable());
        }
        let glyph = id.with_scale_and_position(font.scale, point(x, y));
        if let Some(outline) = font.font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|dx, dy, coverage| {
                let px = bounds.min.x as i32 + dx as i32;
                let py = bounds.min.y as i32 + dy as i32;
                if (0..WIDTH * scale).contains(&px) && (0..HEIGHT * scale).contains(&py) {
                    let start = ((py * WIDTH * scale + px) * 4) as usize;
                    let light =
                        |value| (f32::from(value) + (255.0 - f32::from(value)) * coverage) as u32;
                    let argb = 0xff000000
                        | (light(0x85_u8) << 16)
                        | (light(0x28_u8) << 8)
                        | light(0x23_u8);
                    canvas[start..start + 4].copy_from_slice(&argb.to_ne_bytes());
                }
            });
        }
        x += font.h_advance(id);
    }
    Ok(())
}

impl LayerShellHandler for Bar {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.stop();
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if ![0, WIDTH as u32].contains(&configure.new_size.0)
            || ![0, HEIGHT as u32].contains(&configure.new_size.1)
        {
            self.stop();
            return;
        }
        let result = (|| {
            let scale = 2;
            let (buffer, canvas) = self
                .pool
                .create_buffer(
                    WIDTH * scale,
                    HEIGHT * scale,
                    WIDTH * scale * 4,
                    wl_shm::Format::Argb8888,
                )
                .map_err(|_| unavailable())?;
            draw(canvas, &self.label, scale)?;
            let surface = layer.wl_surface();
            surface.set_buffer_scale(scale);
            surface.damage_buffer(0, 0, WIDTH * scale, HEIGHT * scale);
            surface.frame(qh, surface.clone());
            buffer.attach_to(surface).map_err(|_| unavailable())?;
            layer.commit();
            Ok::<(), String>(())
        })();
        if result.is_err() {
            self.stop();
        }
    }
}

impl CompositorHandler for Bar {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
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
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(item) = self
            .surfaces
            .iter_mut()
            .find(|item| item.layer.wl_surface() == surface)
        {
            item.presented = true;
        }
        if self.surfaces.iter().all(|item| item.presented)
            && let Some(ready) = self.ready.take()
        {
            let _ = ready.send(());
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

impl OutputHandler for Bar {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.sealed {
            self.stop();
        }
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.stop();
    }
}

impl SeatHandler for Bar {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seats.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.stop();
        }
    }
}

impl PointerHandler for Bar {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        if events.iter().any(|event| {
            matches!(event.kind, PointerEventKind::Press { button: 0x110, .. })
                && self
                    .surfaces
                    .iter()
                    .any(|item| item.layer.wl_surface() == &event.surface)
        }) {
            self.stop();
        }
    }
}

impl ShmHandler for Bar {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Bar {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Bar);
delegate_output!(Bar);
delegate_layer!(Bar);
delegate_shm!(Bar);
delegate_seat!(Bar);
delegate_pointer!(Bar);
delegate_registry!(Bar);
