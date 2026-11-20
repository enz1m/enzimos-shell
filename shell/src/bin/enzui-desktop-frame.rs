//! Draws the desktop edge bands and rounded corners on compositor layer surfaces.
use enzui_shell::desktop_frame::{Appearance, HORIZONTAL_BAND, Piece, SIDE_BAND, valid_output};
use enzui_shell::i18n::{self, tr};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, Dispatch, QueueHandle,
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
use std::os::fd::AsFd;
use wayland_protocols::wp::alpha_modifier::v1::client::{
    wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, wp_alpha_modifier_v1::WpAlphaModifierV1,
};

fn read_unbuffered(input: impl AsFd, bytes: &mut [u8]) -> std::io::Result<usize> {
    rustix::io::read(input, bytes).map_err(Into::into)
}

struct Strip {
    layer: LayerSurface,
    piece: Piece,
    width: u32,
    height: u32,
    scale: i32,
    /// A picture committed and not yet shown, since when. The compositor
    /// sends no frame for a surface it does not draw (one faded out, an idle
    /// desktop), so a wait is given up after a while.
    pending: Option<std::time::Instant>,
    dirty: bool,
    /// The compositor's alpha factor for this strip, when it offers one.
    alpha: Option<WpAlphaModifierSurfaceV1>,
}
impl Strip {
    fn waiting(&self) -> bool {
        self.pending
            .is_some_and(|since| since.elapsed() < std::time::Duration::from_millis(100))
    }
}
struct Frame {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layers: LayerShell,
    shm: Shm,
    pool: SlotPool,
    target: String,
    output: Option<wl_output::WlOutput>,
    strips: Vec<Strip>,
    appearance: Appearance,
    alpha_manager: Option<WpAlphaModifierV1>,
    // While the shell changes style the frame fades as a whole: the strips
    // are painted once and the compositor scales their alpha, every strip by
    // the same factor in the same frame.
    shown: f32,
    sent: Option<f32>,
    fade: Option<Fade>,
    command: Vec<u8>,
    exit: bool,
}
#[derive(Clone, Copy)]
struct Fade {
    start: std::time::Instant,
    from: f32,
    to: f32,
    length: std::time::Duration,
    curve: u8,
}
impl Fade {
    fn value(self) -> f32 {
        let elapsed = std::time::Instant::now().saturating_duration_since(self.start);
        let t = (elapsed.as_secs_f32() / self.length.as_secs_f32().max(0.001)).clamp(0.0, 1.0);
        // On the bar's own curve, so the two fade as one.
        self.from + (self.to - self.from) * enzui_shell::desktop_frame::fade_eased(self.curve, t)
    }
    fn done(self) -> bool {
        std::time::Instant::now().saturating_duration_since(self.start) >= self.length
    }
}
impl Frame {
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
        for piece in [Piece::Top, Piece::Bottom, Piece::Left, Piece::Right] {
            let surface = self.compositor.create_surface(qh);
            let region = Region::new(&self.compositor).expect("input region");
            surface.set_input_region(Some(region.wl_region()));
            let layer = self.layers.create_layer_surface(
                qh,
                surface,
                Layer::Top,
                Some("enzui-display-frame"),
                Some(&output),
            );
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.set_exclusive_zone(-1);
            place(&layer, piece);
            let alpha = self
                .alpha_manager
                .as_ref()
                .map(|manager| manager.get_surface(layer.wl_surface(), qh, ()));
            self.strips.push(Strip {
                alpha,
                layer,
                piece,
                width: 0,
                height: 0,
                scale: info.scale_factor,
                pending: None,
                dirty: false,
            });
        }
    }
    fn paint(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(fade) = self.fade {
            self.shown = fade.value();
            if fade.done() {
                self.fade = None;
            }
        }
        // Without the compositor's alpha factor the frame shows or hides at
        // once.
        let blank = self.alpha_manager.is_none() && self.shown < 0.5;
        for strip in &mut self.strips {
            if !strip.dirty || strip.waiting() || strip.width == 0 || strip.height == 0 {
                continue;
            }
            let mut pixels = strip.piece.pixels_for_bar(
                strip.width,
                strip.height,
                strip.scale,
                self.appearance,
            )?;
            if blank {
                pixels.fill(0);
            }
            let w = strip.width as i32 * strip.scale;
            let h = strip.height as i32 * strip.scale;
            let (buffer, _) = self
                .pool
                .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)?;
            buffer
                .canvas(&mut self.pool)
                .ok_or_else(|| tr("shell-frame-buffer-is-unavailable"))?
                .copy_from_slice(&pixels);
            let surface = strip.layer.wl_surface();
            surface.set_buffer_scale(strip.scale);
            surface.damage_buffer(0, 0, w, h);
            surface.frame(qh, surface.clone());
            buffer.attach_to(surface)?;
            if let Some(alpha) = &strip.alpha {
                alpha.set_multiplier(factor(self.shown));
            }
            strip.layer.commit();
            strip.dirty = false;
            strip.pending = Some(std::time::Instant::now());
        }
        // One step of the fade: every strip takes the same factor in one go.
        // Paced by the loop's timer, not by frames: the compositor sends none
        // for a surface it does not draw, and the frame stood still at full
        // strength while the bar faded on an otherwise idle desktop.
        if self.alpha_manager.is_some() && self.sent != Some(self.shown) {
            for strip in &mut self.strips {
                let Some(alpha) = &strip.alpha else { continue };
                if strip.width == 0 || strip.height == 0 {
                    continue;
                }
                alpha.set_multiplier(factor(self.shown));
                strip.layer.commit();
            }
            self.sent = Some(self.shown);
        }
        Ok(())
    }
    fn commands(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        for &byte in bytes {
            if byte == b'\n' {
                let parsed = Appearance::parse(&self.command)?;
                self.command.clear();
                let target = if parsed.hidden >= 100 { 0.0 } else { 1.0 };
                let current = self.fade.map_or(self.shown, |fade| fade.to);
                if target != current {
                    let length = std::time::Duration::from_millis(u64::from(parsed.fade_ms));
                    if length.is_zero() {
                        self.fade = None;
                        self.shown = target;
                    } else {
                        self.fade = Some(Fade {
                            // The bar's first faded frame comes a redraw after
                            // the shell decides: measured, the frame led it by
                            // about this much.
                            start: std::time::Instant::now()
                                + enzui_shell::desktop_frame::fade_lag(target),
                            from: self.fade.map_or(self.shown, Fade::value),
                            to: target,
                            length,
                            curve: parsed.fade_curve,
                        });
                    }
                }
                // The strips' pictures do not depend on the fade.
                let appearance = Appearance {
                    hidden: 0,
                    fade_ms: 0,
                    fade_curve: 0,
                    ..parsed
                };
                let blank_changed =
                    self.alpha_manager.is_none() && (target < 0.5) != (self.shown < 0.5);
                if appearance != self.appearance || blank_changed {
                    self.appearance = appearance;
                    for strip in &mut self.strips {
                        strip.dirty = true;
                    }
                }
            } else {
                if self.command.len() >= 512 {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                self.command.push(byte);
            }
        }
        Ok(())
    }
}
impl CompositorHandler for Frame {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        for strip in &mut self.strips {
            if strip.layer.wl_surface() == surface {
                strip.scale = factor;
                strip.dirty = true;
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
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        for strip in &mut self.strips {
            if strip.layer.wl_surface() == surface {
                strip.pending = None;
            }
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
impl OutputHandler for Frame {
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
impl LayerShellHandler for Frame {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        for strip in &mut self.strips {
            if &strip.layer == layer {
                strip.width = configure.new_size.0;
                strip.height = configure.new_size.1;
                strip.dirty = true;
            }
        }
    }
}
impl ShmHandler for Frame {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_compositor!(Frame);
delegate_output!(Frame);
delegate_shm!(Frame);
delegate_layer!(Frame);
delegate_registry!(Frame);
impl ProvidesRegistryState for Frame {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}
// The compositor's alpha factor for `shown`, 0 to 1.
fn factor(shown: f32) -> u32 {
    (f64::from(shown.clamp(0.0, 1.0)) * f64::from(u32::MAX)).round() as u32
}
impl Dispatch<WpAlphaModifierV1, ()> for Frame {
    fn event(
        _: &mut Self,
        _: &WpAlphaModifierV1,
        _: <WpAlphaModifierV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WpAlphaModifierSurfaceV1, ()> for Frame {
    fn event(
        _: &mut Self,
        _: &WpAlphaModifierSurfaceV1,
        _: <WpAlphaModifierSurfaceV1 as smithay_client_toolkit::reexports::client::Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
fn place(layer: &LayerSurface, piece: Piece) {
    let (anchor, width, height) = match piece {
        Piece::Top => (
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            0,
            HORIZONTAL_BAND,
        ),
        Piece::Bottom => (
            Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            0,
            HORIZONTAL_BAND,
        ),
        Piece::Left => (Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT, SIDE_BAND, 0),
        Piece::Right => (Anchor::TOP | Anchor::BOTTOM | Anchor::RIGHT, SIDE_BAND, 0),
    };
    layer.set_anchor(anchor);
    layer.set_size(width, height);
    layer.set_margin(0, 0, 0, 0);
    layer.commit();
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2
        || !valid_output(&args[0])
        || Appearance::parse(args[1].as_bytes()).is_err()
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        return Err(tr(
            "shell-usage-enzui-desktop-frame-output-dark-light-top-bottom-explicit-wayland-disp",
        )
        .into());
    }
    let conn = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<Frame>(&conn)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh)?;
    let layers = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let alpha_manager = globals.bind::<WpAlphaModifierV1, _, _>(&qh, 1..=1, ()).ok();
    let initial = Appearance::parse(args[1].as_bytes())?;
    let mut frame = Frame {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layers,
        pool: SlotPool::new(4096, &shm)?,
        shm,
        target: args[0].clone(),
        output: None,
        strips: Vec::new(),
        appearance: Appearance {
            hidden: 0,
            fade_ms: 0,
            fade_curve: 0,
            ..initial
        },
        alpha_manager,
        shown: if initial.hidden >= 100 { 0.0 } else { 1.0 },
        sent: None,
        fade: None,
        command: Vec::new(),
        exit: false,
    };
    let mut events: EventLoop<Frame> = EventLoop::try_new()?;
    WaylandSource::new(conn, queue).insert(events.handle())?;
    events.handle().insert_source(
        Generic::new(std::io::stdin(), Interest::READ, Mode::Level),
        |_, input, frame| {
            let mut bytes = [0; 64];
            let count = read_unbuffered(input.as_fd(), &mut bytes)?;
            if count == 0 {
                frame.exit = true;
                return Ok(PostAction::Remove);
            }
            frame.commands(&bytes[..count])?;
            Ok(PostAction::Continue)
        },
    )?;
    let start = std::time::Instant::now();
    while !frame.exit {
        // Once the output is found, the helper sleeps until the shell or the
        // compositor has something for it.
        let timeout = if frame.output.is_none() {
            Some(std::time::Duration::from_secs(1))
        } else if frame.fade.is_some() {
            // A fade steps at about the display's pace.
            Some(std::time::Duration::from_millis(8))
        } else if frame
            .strips
            .iter()
            .any(|strip| strip.dirty && strip.pending.is_some())
        {
            // A picture waiting on a frame that may not come.
            Some(std::time::Duration::from_millis(30))
        } else {
            None
        };
        events.dispatch(timeout, &mut frame)?;
        if frame.output.is_none() && start.elapsed() > std::time::Duration::from_secs(3) {
            return Err(tr("shell-requested-desktop-output-is-unavailable").into());
        }
        frame.paint(&qh)?;
    }
    Ok(())
}
fn main() {
    i18n::initialize();
    if let Err(error) = run() {
        eprintln!("enzui-desktop-frame: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_shell::{settings::Settings, shell_style::ShellStyle};
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    #[test]
    fn long_islands_command_remains_fd_readable_until_its_newline() {
        let settings = Settings {
            shell_style: ShellStyle::Islands,
            ..Default::default()
        };
        let appearance = Appearance {
            metrics: settings.effective_metrics(),
            ..Default::default()
        };
        let command = appearance.command();
        assert!(command.len() > 64 && command.len() < 128);
        let (input, output) = rustix::pipe::pipe().unwrap();
        assert_eq!(
            rustix::io::write(&output, command.as_bytes()).unwrap(),
            command.len()
        );
        let mut received = Vec::new();
        for _ in 0..2 {
            let mut descriptors = [PollFd::new(&input, PollFlags::IN)];
            assert_eq!(
                poll(&mut descriptors, Some(&Timespec::default())).unwrap(),
                1
            );
            let mut bytes = [0; 64];
            let count = read_unbuffered(&input, &mut bytes).unwrap();
            received.extend_from_slice(&bytes[..count]);
        }
        assert_eq!(received, command.as_bytes());
        assert_eq!(Appearance::parse(&received).unwrap(), appearance);
        let mut descriptors = [PollFd::new(&input, PollFlags::IN)];
        assert_eq!(
            poll(&mut descriptors, Some(&Timespec::default())).unwrap(),
            0
        );
    }
}
