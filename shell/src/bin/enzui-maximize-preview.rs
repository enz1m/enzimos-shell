//! Draws the animated window-placement preview on compositor layer surfaces.
use enzui_shell::desktop_frame::valid_output;
use enzui_shell::i18n::{self, tr};
use enzui_shell::maximize_preview::{Animation, BODY_WASH, Command, Rect, frame_pixels};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
        },
    },
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
use std::{os::fd::AsFd, time::Instant};

fn read_unbuffered(input: impl AsFd, bytes: &mut [u8]) -> std::io::Result<usize> {
    rustix::io::read(input, bytes).map_err(Into::into)
}

struct Preview {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layers: LayerShell,
    shm: Shm,
    pool: SlotPool,
    target: String,
    output: Option<wl_output::WlOutput>,
    layer: Option<LayerSurface>,
    scale: i32,
    origin: (i32, i32),
    configured: Option<(u32, u32)>,
    animation: Option<Animation>,
    clear_pending: bool,
    accent: [u8; 3],
    radius: u16,
    last: Option<Instant>,
    command: Vec<u8>,
    exit: bool,
}
impl Preview {
    fn attach(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.output.is_some() {
            return;
        }
        let Some(info) = self.outputs.info(&output) else {
            return;
        };
        if info.name.as_deref() != Some(self.target.as_str()) {
            return;
        }
        self.output = Some(output.clone());
        self.scale = info.scale_factor;
        let surface = self.compositor.create_surface(qh);
        // The preview is decoration: it must never take pointer input away from
        // the drag that is producing it.
        let region = Region::new(&self.compositor).expect("input region");
        surface.set_input_region(Some(region.wl_region()));
        let layer = self.layers.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("enzui-maximize-preview"),
            Some(&output),
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        // One surface covers the output for the helper's lifetime. Reusing it
        // avoids configure round trips and allows repeated preview gestures.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
        layer.set_margin(0, 0, 0, 0);
        layer.commit();
        self.layer = Some(layer);
    }
    fn paint(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn std::error::Error>> {
        let Some(layer) = self.layer.clone() else {
            return Ok(());
        };
        let Some((width, height)) = self.configured.filter(|(w, h)| *w > 0 && *h > 0) else {
            return Ok(());
        };
        let (rect, opacity) = match self.animation {
            Some(animation) => (animation.rect(), animation.opacity()),
            None if self.clear_pending => (Rect::default(), 0.0),
            None => return Ok(()),
        };
        let data = frame_pixels(
            width,
            height,
            self.scale,
            rect,
            self.origin,
            f32::from(self.radius),
            self.accent,
            opacity,
            BODY_WASH,
        )?;
        let w = width as i32 * self.scale;
        let h = height as i32 * self.scale;
        let (buffer, _) = self
            .pool
            .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)?;
        buffer
            .canvas(&mut self.pool)
            .ok_or_else(|| tr("shell-preview-buffer-is-unavailable"))?
            .copy_from_slice(&data);
        let surface = layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, w, h);
        surface.frame(qh, surface.clone());
        buffer.attach_to(surface)?;
        layer.commit();
        self.clear_pending = false;
        Ok(())
    }
    fn hide(&mut self) {
        self.animation = None;
        self.last = None;
        // Unmapping a layer surface invalidates its configure; keep the mapped
        // surface and present a transparent buffer before the next gesture.
        self.clear_pending = true;
    }
    fn apply(&mut self, command: Command) {
        match command {
            Command::Hide => match &mut self.animation {
                Some(animation) if !animation.fading() => animation.release(),
                Some(_) => {}
                None => self.hide(),
            },
            Command::Show {
                from,
                to,
                accent,
                radius,
            } => {
                if !from.valid() || !to.valid() {
                    return;
                }
                self.accent = accent;
                self.radius = radius;
                self.animation = Some(Animation::new(from, to));
                self.last = Some(Instant::now());
            }
        }
    }
    fn tick(&mut self) {
        let Some(animation) = &mut self.animation else {
            return;
        };
        let now = Instant::now();
        let delta = self.last.map_or_else(Default::default, |last| now - last);
        self.last = Some(now);
        animation.advance(delta);
        if animation.finished() {
            self.hide();
        }
    }
    fn commands(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        for &byte in bytes {
            if byte == b'\n' {
                let parsed = Command::parse(&self.command);
                self.command.clear();
                match parsed {
                    Ok(command) => self.apply(command),
                    Err(_) => return Err(std::io::ErrorKind::InvalidData.into()),
                }
            } else {
                if self.command.len() >= 128 {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                self.command.push(byte);
            }
        }
        Ok(())
    }
}
impl CompositorHandler for Preview {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        self.scale = factor;
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
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
impl OutputHandler for Preview {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.attach(qh, output);
    }
    fn update_output(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.attach(qh, output);
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.output.as_ref() == Some(&output) {
            self.exit = true;
        }
    }
}
impl LayerShellHandler for Preview {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.configured = Some(configure.new_size);
    }
}
impl ShmHandler for Preview {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_compositor!(Preview);
delegate_output!(Preview);
delegate_shm!(Preview);
delegate_layer!(Preview);
delegate_registry!(Preview);
impl ProvidesRegistryState for Preview {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

// Inherited compositor pipes may never close; an output-scoped lock retires the previous
// preview.
fn take_over(output: &str) -> Result<std::fs::File, Box<dyn std::error::Error>> {
    use rustix::fs::{FlockOperation, flock};
    use std::io::{Read, Seek, Write};
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is required")?;
    let path = std::path::Path::new(&runtime).join(format!("enzui-maximize-preview-{output}.lock"));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let exe = std::fs::read_link("/proc/self/exe")?;
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    let mut signalled = false;
    while flock(&file, FlockOperation::NonBlockingLockExclusive).is_err() {
        if !signalled || Instant::now() >= deadline {
            let mut text = String::new();
            file.seek(std::io::SeekFrom::Start(0))?;
            file.read_to_string(&mut text)?;
            let holder = text
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|pid| *pid > 1 && *pid != std::process::id() as i32);
            // Only another preview, by its executable, is ever signalled.
            if let Some(pid) = holder.filter(|pid| {
                std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|other| other == exe)
            }) && let Some(pid) = rustix::process::Pid::from_raw(pid)
                && let Ok(fd) =
                    rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
            {
                let signal = if signalled {
                    rustix::process::Signal::KILL
                } else {
                    rustix::process::Signal::TERM
                };
                let _ = rustix::process::pidfd_send_signal(&fd, signal);
            }
            if signalled && Instant::now() >= deadline + std::time::Duration::from_secs(1) {
                return Err(tr("shell-an-older-preview-holds-this-output").into());
            }
            signalled = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    writeln!(file, "{}", std::process::id())?;
    Ok(file)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 1 || !valid_output(&args[0]) || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(tr(
            "shell-usage-enzui-maximize-preview-output-explicit-wayland-display-required",
        )
        .into());
    }
    let _lock = take_over(&args[0])?;
    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<Preview>(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layers = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let mut preview = Preview {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layers,
        pool: SlotPool::new(4096, &shm)?,
        shm,
        target: args[0].clone(),
        output: None,
        layer: None,
        scale: 1,
        origin: (0, 0),
        configured: None,
        animation: None,
        clear_pending: false,
        accent: [0x4a, 0x90, 0xd9],
        radius: 12,
        last: None,
        command: Vec::new(),
        exit: false,
    };
    let mut events: EventLoop<Preview> = EventLoop::try_new()?;
    WaylandSource::new(conn, queue).insert(events.handle())?;
    events.handle().insert_source(
        Generic::new(std::io::stdin(), Interest::READ, Mode::Level),
        |_, input, preview| {
            let mut bytes = [0; 256];
            let count = read_unbuffered(input.as_fd(), &mut bytes)?;
            if count == 0 {
                preview.exit = true;
                return Ok(PostAction::Remove);
            }
            preview.commands(&bytes[..count])?;
            Ok(PostAction::Continue)
        },
    )?;
    let start = Instant::now();
    while !preview.exit {
        // Keep idle dispatch waits short enough to display the first gesture frame promptly.
        let wait = if preview.animation.is_some() {
            std::time::Duration::from_millis(8)
        } else {
            std::time::Duration::from_millis(16)
        };
        events.dispatch(wait, &mut preview)?;
        if preview.output.is_none() && start.elapsed() > std::time::Duration::from_secs(3) {
            return Err(tr("shell-requested-preview-output-is-unavailable").into());
        }
        // Paint before advancing so a newly opened preview starts at its initial pose.
        preview.paint(&qh)?;
        preview.tick();
    }
    Ok(())
}
fn main() {
    i18n::initialize();
    if let Err(error) = run() {
        eprintln!("enzui-maximize-preview: {error}");
        std::process::exit(1);
    }
}
