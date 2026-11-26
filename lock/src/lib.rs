//! Compositor-enforced lock screen and greetd greeter.
//!
//! The lock survives client failure. Stdout reports `locked` after compositor acknowledgement,
//! `already-locked` for an existing lock, and `unlocked` before successful exit.
//! The greeter authenticates through greetd before starting the selected session.

use enzui_shell::i18n::tr;
use enzui_shell::tr;
mod access;
mod accounts;
mod appearance;
mod auth;
mod avatars;
mod background;
mod clock_widgets;
mod exec;
mod gpu;
mod greetd;
mod greeter_session;
mod keys;
mod live_background;
#[cfg(feature = "greeter-preview")]
mod preview;
mod software;
mod switch;
mod wayland_events;
#[cfg(feature = "greeter-preview")]
pub use preview::run as run_preview;
mod ui;

use std::io::{Read as _, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use iced_runtime::user_interface::{self, UserInterface};
use iced_wgpu::{
    Engine, Renderer as WgpuRenderer,
    graphics::{Shell as GraphicsShell, Viewport},
    wgpu,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

static SWITCH_FAILED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

type IcedRenderer = iced::Renderer;

struct Output {
    output: wl_output::WlOutput,
    global: u32,
    removed: bool,
    name: String,
    scale: i32,
    mode: (i32, i32),
    capture: Option<Capture>,
}

// Screencopy allocates the compositor-requested buffer in a memfd.
struct Capture {
    frame: Option<ZwlrScreencopyFrameV1>,
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    memfd: Option<OwnedFd>,
    buffer: Option<wl_buffer::WlBuffer>,
    y_invert: bool,
    done: Option<Option<background::Picture>>,
}

struct Surface {
    output: usize,
    surface: wl_surface::WlSurface,
    lock_surface: Option<ExtSessionLockSurfaceV1>,
    size: (u32, u32),
    configured: bool,
    gpu: Option<SurfaceGpu>,
    images: Option<ui::Images>,
    cursor: iced::mouse::Cursor,
    events: Vec<iced::Event>,
    cache: user_interface::Cache,
    /// The local minute the surface last showed; see [`clock_minute`].
    drawn: Option<i64>,
    buffer_scale: i32,
    fractional: Option<Fractional>,
    /// The physical size the surface's backdrop was last requested at.
    backdrop: Option<(u32, u32)>,
    software_pending: usize,
    /// When the frame callback asked for with the last present was
    /// requested, until the compositor answers it.
    frame_pending: Option<Instant>,
    /// A change arrived while the surface could not draw; see [`frame_pass`].
    stale: bool,
    /// A frame that hides the desktop completely was presented.
    opaque: bool,
}

// A buffer at the preferred scale, rounded as winit rounds it, shown at the
// logical size through a viewport.
struct Fractional {
    scale: WpFractionalScaleV1,
    viewport: WpViewport,
    preferred: Option<u32>,
    destination: Option<(u32, u32)>,
}

/// The wp_fractional_scale_v1 denominator.
const SCALE_DENOMINATOR: u32 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scale {
    /// A `wl_surface.set_buffer_scale` factor.
    Integer(u32),
    /// A preferred scale in 120ths, presented through a viewport.
    Fractional(u32),
}

impl Scale {
    fn factor(self) -> f32 {
        match self {
            Self::Integer(scale) => scale as f32,
            Self::Fractional(scale) => scale as f32 / SCALE_DENOMINATOR as f32,
        }
    }

    fn physical(self, (width, height): (u32, u32)) -> (u32, u32) {
        let apply = |length: u32| match self {
            Self::Integer(scale) => length.saturating_mul(scale),
            Self::Fractional(scale) => {
                let scaled = (u64::from(length) * u64::from(scale)
                    + u64::from(SCALE_DENOMINATOR / 2))
                    / u64::from(SCALE_DENOMINATOR);
                u32::try_from(scaled).unwrap_or(u32::MAX)
            }
        };
        (apply(width).max(1), apply(height).max(1))
    }
}

impl Surface {
    fn scale(&self, outputs: &[Output]) -> Scale {
        match self.fractional.as_ref().and_then(|f| f.preferred) {
            Some(preferred) => Scale::Fractional(preferred),
            None => Scale::Integer(outputs[self.output].scale.max(1) as u32),
        }
    }

    fn physical(&self, outputs: &[Output]) -> (u32, u32) {
        self.scale(outputs).physical(self.size)
    }

    // Buffer scale and viewport are double-buffered: a change needs a new
    // frame to take effect, even when the buffer size stays the same.
    fn apply_scale(&mut self, scale: Scale) -> bool {
        let mut changed = false;
        let buffer_scale = match scale {
            Scale::Integer(scale) => scale as i32,
            Scale::Fractional(_) => 1,
        };
        if self.buffer_scale != buffer_scale {
            self.buffer_scale = buffer_scale;
            self.surface.set_buffer_scale(buffer_scale);
            changed = true;
        }
        if let Some(fractional) = &mut self.fractional {
            let destination = matches!(scale, Scale::Fractional(_)).then_some(self.size);
            if fractional.destination != destination {
                fractional.destination = destination;
                let (width, height) = destination.map_or((-1, -1), |(w, h)| (w as i32, h as i32));
                fractional.viewport.set_destination(width, height);
                changed = true;
            }
        }
        changed
    }
}

struct SurfaceGpu {
    surface: Option<wgpu::Surface<'static>>,
    renderer: IcedRenderer,
    format: wgpu::TextureFormat,
    physical: (u32, u32),
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    seat: Option<wl_seat::WlSeat>,
    lock_manager: Option<ExtSessionLockManagerV1>,
    screencopy: Option<ZwlrScreencopyManagerV1>,
    cursor_shape: Option<WpCursorShapeManagerV1>,
    fractional_scale: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    outputs: Vec<Output>,
    lock: Option<ExtSessionLockV1>,
    /// When the compositor confirmed the lock.
    locked: Option<Instant>,
    finished: bool,
    surfaces: Vec<Surface>,
    keyboard: Option<keys::Keyboard>,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_shape: Option<WpCursorShapeDeviceV1>,
    pointer_surface: Option<usize>,
    pointer_serial: Option<u32>,
    pointer_activity: Option<Instant>,
    // Where the pointer entered; only real movement away from it wakes the
    // prompt, not the pointer merely resting over the lock surface.
    pointer_origin: Option<(f64, f64)>,
    repeat: (i32, i32),
    held: Option<(u32, Instant)>,
    keys: Vec<keys::Key>,
    input: bool,
}

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn wait(
    connection: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    limit: Duration,
    ready: impl Fn(&State) -> bool,
) -> Result<bool, String> {
    let deadline = Instant::now() + limit;
    while !ready(state) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        pump(
            connection,
            queue,
            state,
            left.min(Duration::from_millis(50)),
            None,
        )?;
    }
    Ok(true)
}

/// How long a lock with nothing moving may sleep: until the clock shows a new
/// minute or the prompt's idle timeout fades it, whichever comes first.
/// Input, frames and the compositor wake it sooner.
fn rest(ui: &ui::Ui, now: Instant) -> Duration {
    use chrono::Timelike as _;
    let clock = chrono::Local::now();
    let into_minute = Duration::from_secs(u64::from(clock.second()))
        + Duration::from_nanos(u64::from(clock.nanosecond().min(999_999_999)));
    let minute = Duration::from_secs(60).saturating_sub(into_minute) + Duration::from_millis(20);
    let prompt = ui
        .last_input
        .map(|at| (at + ui::PROMPT_IDLE).saturating_duration_since(now))
        .filter(|left| !left.is_zero())
        .map_or(minute, |left| left + Duration::from_millis(20));
    minute.min(prompt).max(Duration::from_millis(5))
}

const GPU_WAIT: Duration = Duration::from_millis(600);
/// How long a see-through lock waits for its backdrops before fading in over black.
const REVEAL_WAIT: Duration = Duration::from_secs(1);
/// The same wait for a lock that starts over its opaque desktop capture.
const CAPTURED_REVEAL_WAIT: Duration = Duration::from_secs(5);
/// How long a confirmed lock waits for an opaque frame on every output before reporting.
const OPAQUE_WAIT: Duration = Duration::from_secs(3);

struct SurfaceReveal {
    removed: bool,
    configured: bool,
    backdrop: bool,
}

// A removed output never configures its surface, and a surface configured
// late gets its backdrop late, so neither holds back the fade-in.
fn reveal_due(
    surfaces: impl IntoIterator<Item = SurfaceReveal>,
    waited: Duration,
    limit: Duration,
) -> bool {
    waited >= limit
        || surfaces
            .into_iter()
            .all(|s| s.removed || !s.configured || s.backdrop)
}

/// How long a surface waits for its frame callback before drawing anyway.
const FRAME_FALLBACK: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    /// The surface waits for the compositor (its frame callback, or a
    /// software buffer it still holds).
    Wait,
    /// Nothing changed and the clock still shows the current minute.
    Skip,
    Draw,
}

// What one pass of the loop does with a surface. A change seen while the
// surface waits is kept until it can draw: the pass that saw it has already
// consumed the loop's redraw request, and the frame callback that ends the
// wait carries no change of its own, so the new state would otherwise show
// only on the next input.
fn frame_pass(stale: &mut bool, waiting: bool, changed: bool, current: bool) -> Pass {
    if waiting {
        *stale |= changed;
        Pass::Wait
    } else if changed || *stale || !current {
        Pass::Draw
    } else {
        Pass::Skip
    }
}

/// The local wall-clock minute: a new one changes what the clock and date
/// show, including across a time-zone change.
fn clock_minute() -> i64 {
    chrono::Local::now()
        .naive_local()
        .and_utc()
        .timestamp()
        .div_euclid(60)
}

/// Empties a nonblocking wake pipe; false once its writer has closed it.
fn drain_open(wake: &OwnedFd) -> bool {
    let mut drained = [0_u8; 64];
    loop {
        match rustix::io::read(wake, &mut drained) {
            Ok(0) => return false,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
}

/// Dispatches compositor events, waiting up to `timeout` for them or for
/// any of `also` to become readable.
fn pump<'a>(
    connection: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
    also: impl IntoIterator<Item = std::os::fd::BorrowedFd<'a>>,
) -> Result<(), String> {
    queue
        .dispatch_pending(state)
        .map_err(|_| tr("shell-lost-the-compositor"))?;
    connection
        .flush()
        .map_err(|_| tr("shell-lost-the-compositor"))?;
    if let Some(guard) = connection.prepare_read() {
        let fd = guard.connection_fd();
        let mut fds = vec![rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN,
        )];
        let also: Vec<_> = also.into_iter().collect();
        for also in &also {
            fds.push(rustix::event::PollFd::new(
                also,
                rustix::event::PollFlags::IN,
            ));
        }
        let timeout = rustix::event::Timespec {
            tv_sec: timeout.as_secs() as i64,
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        if rustix::event::poll(&mut fds, Some(&timeout)).is_ok_and(|n| n > 0)
            && fds[0].revents().contains(rustix::event::PollFlags::IN)
        {
            guard.read().map_err(|_| tr("shell-lost-the-compositor"))?;
        }
    }
    queue
        .dispatch_pending(state)
        .map_err(|_| tr("shell-lost-the-compositor"))?;
    Ok(())
}

use gpu::{configure, raw_surface};

fn handle(picture: background::Picture) -> iced::widget::image::Handle {
    iced::widget::image::Handle::from_rgba(picture.width, picture.height, picture.rgba)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Chooses an existing-session lock or a greetd-backed login.
pub enum Mode {
    /// Locks the current Wayland session until PAM authenticates its owner.
    Lock,
    /// Authenticates through greetd and launches a selected desktop session.
    Greeter,
}

fn add_lock_surface(
    state: &mut State,
    compositor: &wl_compositor::WlCompositor,
    lock: &ExtSessionLockV1,
    qh: &QueueHandle<State>,
) {
    let index = state.surfaces.len();
    let surface = compositor.create_surface(qh, ());
    let output = &state.outputs[index];
    let scale = output.scale;
    // Requested before the lock surface, so a compositor that knows the
    // output's scale can send it with the first configure.
    let fractional = match (&state.fractional_scale, &state.viewporter, output.removed) {
        (Some(manager), Some(viewporter), false) => Some(Fractional {
            scale: manager.get_fractional_scale(&surface, qh, index),
            viewport: viewporter.get_viewport(&surface, qh, ()),
            preferred: None,
            destination: None,
        }),
        _ => None,
    };
    let lock_surface = if output.removed {
        surface.destroy();
        None
    } else {
        surface.set_buffer_scale(scale);
        Some(lock.get_lock_surface(&surface, &output.output, qh, index))
    };
    state.surfaces.push(Surface {
        output: index,
        surface,
        lock_surface,
        size: (0, 0),
        configured: false,
        gpu: None,
        images: None,
        cursor: iced::mouse::Cursor::Unavailable,
        events: Vec::new(),
        cache: user_interface::Cache::new(),
        drawn: None,
        buffer_scale: scale,
        fractional,
        backdrop: None,
        software_pending: 0,
        frame_pending: None,
        stale: false,
        opaque: false,
    });
}

/// The renderer's defaults: the desktop's own UI family, so the lock screen's
/// text (name, password field, hints) matches the shell instead of falling
/// back to the generic sans-serif alias.
pub(crate) fn renderer_settings() -> iced::advanced::renderer::Settings {
    iced::advanced::renderer::Settings {
        font: iced::Font {
            family: iced::font::Family::Name(enzui_shell::font_settings::default_sans()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Runs until unlocking, login completion or a display/protocol failure.
///
/// # Errors
/// Returns a diagnostic if display setup, authentication transport or rendering fails.
pub fn run(mode: Mode) -> Result<(), String> {
    let connection =
        Connection::connect_to_env().map_err(|_| tr("shell-no-display-server-connection"))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State {
        compositor: None,
        shm: None,
        seat: None,
        lock_manager: None,
        screencopy: None,
        cursor_shape: None,
        fractional_scale: None,
        viewporter: None,
        outputs: Vec::new(),
        lock: None,
        locked: None,
        finished: false,
        surfaces: Vec::new(),
        keyboard: None,
        wl_keyboard: None,
        pointer: None,
        pointer_shape: None,
        pointer_surface: None,
        pointer_serial: None,
        pointer_activity: None,
        pointer_origin: None,
        repeat: (25, 600),
        held: None,
        keys: Vec::new(),
        input: false,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-lost-the-compositor"))?;
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-lost-the-compositor"))?;
    let (Some(compositor), Some(manager)) = (state.compositor.clone(), state.lock_manager.clone())
    else {
        return Err(tr("shell-the-compositor-does-not-support-secure-session-locking").into());
    };

    // The supervisor asks for software drawing after a lock client crashed,
    // so a graphics driver that kills the process cannot keep the lock blank.
    let software = std::env::var("ENZUI_LOCK_SOFTWARE").as_deref() == Ok("1");
    let (gpu_sender, gpu_started) = mpsc::channel();
    if !software {
        std::thread::spawn(move || {
            let _ = gpu_sender.send(
                std::panic::catch_unwind(Gpu::new)
                    .unwrap_or_else(|_| Err(tr("shell-graphics-initialization-panicked").into())),
            );
        });
    }

    // Session-lock xray fades over the live desktop instead of a captured frame.
    let xray = mode == Mode::Lock
        && (std::env::var("ENZUI_LOCK_XRAY").as_deref() == Ok("1") || hyprland_xray());

    // Remote-session outputs are locked too, but not shown the capture.
    // The greeter has no desktop to capture; it starts from black.
    if mode == Mode::Lock
        && !xray
        && let Some(screencopy) = state.screencopy.clone()
    {
        for (index, output) in state.outputs.iter_mut().enumerate() {
            if output.removed {
                continue;
            }
            output.capture = Some(Capture {
                frame: Some(screencopy.capture_output(0, &output.output, &qh, index)),
                format: None,
                memfd: None,
                buffer: None,
                y_invert: false,
                done: None,
            });
        }
        wait(
            &connection,
            &mut queue,
            &mut state,
            Duration::from_millis(900),
            |s| {
                s.outputs
                    .iter()
                    .all(|o| o.capture.as_ref().is_none_or(|c| c.done.is_some()))
            },
        )?;
    }

    // The compositor confirms the lock only once every output shows a lock
    // surface, so surfaces are created and drawn before waiting for it.
    let lock = manager.lock(&qh, ());
    let locking = Instant::now();
    enzui_shell::power_boost::request();
    state.lock = Some(lock.clone());
    while state.surfaces.len() < state.outputs.len() {
        add_lock_surface(&mut state, &compositor, &lock, &qh);
    }
    wait(
        &connection,
        &mut queue,
        &mut state,
        Duration::from_secs(2),
        |s| {
            s.finished
                || s.surfaces
                    .iter()
                    .all(|x| x.configured || s.outputs[x.output].removed)
        },
    )?;
    if state.finished {
        // Not a failure: the session is locked, just not by us.
        say("already-locked");
        return Ok(());
    }

    // The compositor shows its lock-missing screen if no lock surface is drawn
    // within a second, so slow graphics give way to software drawing.
    let gpu_ready = std::cell::OnceCell::new();
    if !software {
        wait(
            &connection,
            &mut queue,
            &mut state,
            GPU_WAIT.saturating_sub(locking.elapsed()),
            |_| {
                if let Ok(gpu) = gpu_started.try_recv() {
                    let _ = gpu_ready.set(gpu);
                }
                gpu_ready.get().is_some()
            },
        )?;
    }
    // A late device is released by its thread rather than kept unused.
    drop(gpu_started);
    let gpu = match gpu_ready.into_inner() {
        Some(Ok(gpu)) => Some(gpu),
        Some(Err(error)) => {
            eprintln!("enzui-lock: {error}; using tiny-skia software rendering");
            None
        }
        None if software => {
            eprintln!("enzui-lock: using tiny-skia software rendering after a crash");
            None
        }
        None => {
            eprintln!(
                "enzui-lock: graphics are still starting; using tiny-skia software rendering"
            );
            None
        }
    };

    // Settings: appearance of the prompt and which wallpaper to show.
    let settings = enzui_shell::settings::Store::from_environment()
        .and_then(|s| s.load().ok())
        .unwrap_or_default();
    let mut ui = ui::Ui::new(auth::display_name(), settings.session.clock_24h);
    ui.clock = settings.session.lock_clock.clone();
    let (theme, reduced) = appearance::theme();
    ui.reduced_motion = reduced || !settings.compositor.animations;
    let mut access = access::Access::new();
    ui.xray = xray;
    let greeter = (mode == Mode::Greeter).then(|| start_greeter(&mut ui));
    // The session is locked by now, so offering another sign-in is safe.
    ui.can_switch = greeter.is_none() && switch::available();
    let mut launching = false;
    // Until then the loop keeps polling for answers that arrive without a
    // wake of their own (a user switch's failure).
    let mut busy_until = Instant::now();

    // Backdrops are prepared off the main thread while the captured
    // desktop is already on screen.
    let (backdrops_tx, backdrops) = mpsc::channel();
    let mut sizes = Vec::new();
    for (index, surface) in state.surfaces.iter_mut().enumerate() {
        if surface.configured && !state.outputs[surface.output].removed {
            let physical = surface.physical(&state.outputs);
            surface.backdrop = Some(physical);
            sizes.push((index, physical));
        }
    }
    let wallpaper = background::selected(&settings.wallpaper, &settings.session.lock_clock);
    let late_backdrops = (backdrops_tx.clone(), wallpaper.clone());
    std::thread::spawn(move || {
        for (index, (width, height)) in sizes {
            let backdrop = background::load(&wallpaper, width, height);
            if backdrops_tx
                .send((index, (width, height), backdrop))
                .is_err()
            {
                break;
            }
        }
    });

    for surface in state.surfaces.iter_mut().filter(|s| s.configured) {
        let (width, height) = surface.physical(&state.outputs);
        let desktop = state.outputs[surface.output]
            .capture
            .as_mut()
            .and_then(|c| c.done.take())
            .flatten();
        let placeholder =
            handle(desktop.unwrap_or_else(|| background::Picture::solid(width, height, [0, 0, 0])));
        surface.images = Some(ui::Images {
            desktop: (mode == Mode::Lock && !xray).then(|| placeholder.clone()),
            wallpaper: placeholder.clone(),
            soft: placeholder,
            has_clock: true,
            has_date: true,
            light: false,
        });
    }
    // Until the backdrop is ready only the captured desktop is shown.
    let reveal_from = Instant::now();
    ui.started = reveal_from + Duration::from_secs(3600);
    let (auth_tx, auth_rx) = mpsc::channel::<bool>();
    // Fingerprint unlock beside the password, on the lock screen only:
    // after a restart the password is needed to unlock the keyring.
    let (finger_tx, finger_rx) = mpsc::channel::<Option<String>>();
    let mut finger_wake = (mode == Mode::Lock && auth::fingerprint_offered())
        .then(|| {
            rustix::pipe::pipe_with(
                rustix::pipe::PipeFlags::NONBLOCK | rustix::pipe::PipeFlags::CLOEXEC,
            )
            .ok()
        })
        .flatten()
        .map(|(read, write)| {
            let unlock = auth_tx.clone();
            std::thread::spawn(move || {
                let wake = || {
                    let _ = rustix::io::write(&write, &[1]);
                };
                let show = |text: String| {
                    let _ = finger_tx.send(Some(text));
                    wake();
                };
                match auth::fingerprint(&show) {
                    auth::Fingerprint::Matched => {
                        let _ = unlock.send(true);
                    }
                    auth::Fingerprint::Refused => {
                        let _ =
                            finger_tx.send(Some(tr("shell-fingerprint-not-recognized").to_owned()));
                    }
                    auth::Fingerprint::Unavailable => {
                        let _ = finger_tx.send(None);
                    }
                }
                wake();
            });
            read
        });
    let mut revealed = false;
    let mut reported = false;
    let mut fallback_images = std::collections::BTreeMap::<usize, ui::Images>::new();
    let mut last = Instant::now();
    // Surfaces configured after the first wait are prepared like new outputs.
    let mut initial = 0;
    let mut live_size = state.surfaces.first().map_or((960, 540), |s| s.size);
    let mut live =
        live_background::Live::start(&settings.wallpaper, &ui.clock, live_size, ui.reduced_motion);
    // Controls handled after a frame was drawn show on the next one.
    let mut redraw = false;
    loop {
        // Outputs that arrive while locked get their own lock surface; until
        // it is drawn the compositor shows nothing of the desktop there.
        while state.surfaces.len() < state.outputs.len() {
            add_lock_surface(&mut state, &compositor, &lock, &qh);
        }
        fallback_images.retain(|&index, _| !state.outputs[index].removed);
        for (index, surface) in state.surfaces.iter_mut().enumerate().skip(initial) {
            if surface.images.is_some() || !surface.configured {
                continue;
            }
            let (width, height) = surface.physical(&state.outputs);
            surface.backdrop = Some((width, height));
            let black = handle(background::Picture::solid(width, height, [0, 0, 0]));
            surface.images = Some(ui::Images {
                desktop: None,
                wallpaper: black.clone(),
                soft: black,
                has_clock: true,
                has_date: true,
                light: false,
            });
            let (sender, wallpaper) = (late_backdrops.0.clone(), late_backdrops.1.clone());
            std::thread::spawn(move || {
                let _ = sender.send((
                    index,
                    (width, height),
                    background::load(&wallpaper, width, height),
                ));
            });
        }
        // A backdrop requested before a scale or size change is loaded again
        // at the surface's new size.
        for (index, surface) in state.surfaces.iter_mut().enumerate() {
            let physical = surface.physical(&state.outputs);
            if surface
                .backdrop
                .is_none_or(|requested| requested == physical)
                || state.outputs[surface.output].removed
            {
                continue;
            }
            surface.backdrop = Some(physical);
            let (sender, wallpaper) = (late_backdrops.0.clone(), late_backdrops.1.clone());
            std::thread::spawn(move || {
                let (width, height) = physical;
                let _ = sender.send((index, physical, background::load(&wallpaper, width, height)));
            });
        }
        if state.surfaces[initial..]
            .iter()
            .all(|s| s.images.is_some() || state.outputs[s.output].removed)
        {
            initial = state.surfaces.len();
        }
        // The prompt is on the first output still connected.
        let primary = state
            .surfaces
            .iter()
            .position(|s| !state.outputs[s.output].removed)
            .unwrap_or(0);
        while let Ok((index, size, backdrop)) = backdrops.try_recv() {
            if let Some(surface) = state
                .surfaces
                .get_mut(index)
                .filter(|s| s.backdrop == Some(size))
            {
                let desktop = surface.images.as_ref().and_then(|i| i.desktop.clone());
                let images = ui::Images {
                    desktop,
                    has_clock: backdrop.has_clock,
                    has_date: false,
                    light: backdrop.light,
                    wallpaper: handle(backdrop.sharp),
                    soft: handle(backdrop.soft),
                };
                fallback_images.insert(index, images.clone());
                // A live frame that came first stays: an unchanged scene
                // does not send it again.
                surface.images = Some(match live.as_ref().and_then(live_background::Live::shown) {
                    Some(frame) => ui::Images {
                        desktop: images.desktop,
                        ..frame.clone()
                    },
                    None => images,
                });
            }
        }
        if let Some(surface) = state.surfaces.get(primary)
            && surface.size != live_size
        {
            live_size = surface.size;
            drop(live.take());
            live = live_background::Live::start(
                &settings.wallpaper,
                &ui.clock,
                live_size,
                ui.reduced_motion,
            );
        }
        if let Some(feed) = &mut live {
            if let Some(frame) = feed.frame() {
                for surface in &mut state.surfaces {
                    if let Some(images) = &mut surface.images {
                        let desktop = images.desktop.clone();
                        *images = frame.clone();
                        images.desktop = desktop;
                    }
                }
                redraw = true;
            }
            if !feed.healthy() {
                drop(live.take());
                for (index, images) in &fallback_images {
                    if let Some(surface) = state.surfaces.get_mut(*index) {
                        surface.images = Some(images.clone());
                    }
                }
                redraw = true;
            }
        }
        if !revealed
            && reveal_due(
                state
                    .surfaces
                    .iter()
                    .enumerate()
                    .map(|(index, s)| SurfaceReveal {
                        removed: state.outputs[s.output].removed,
                        configured: s.configured,
                        backdrop: fallback_images.contains_key(&index),
                    }),
                reveal_from.elapsed(),
                if xray {
                    REVEAL_WAIT
                } else {
                    CAPTURED_REVEAL_WAIT
                },
            )
        {
            revealed = true;
            ui.started = Instant::now();
        }
        let mut actions: Vec<ui::Message> = Vec::new();
        for request in access.requests(&ui) {
            match request {
                access::Request::Focus(target) => ui.target = target,
                access::Request::Press(target) => {
                    ui.target = target;
                    actions.push(target.message());
                }
            }
            ui.focus_visible = true;
            redraw = true;
        }
        let mut changed = false;
        if let Some((requests, events, wake)) = &greeter {
            if let Some(wake) = wake {
                let mut drained = [0_u8; 64];
                while (&*wake).read(&mut drained).is_ok_and(|n| n > 0) {}
            }
            while let Ok(event) = events.try_recv() {
                changed = true;
                if greeter_event(&mut ui, event, requests, &mut launching) {
                    lock.unlock_and_destroy();
                    let _ = queue.roundtrip(&mut state);
                    return Ok(());
                }
            }
        }
        // A finished fingerprint thread leaves its pipe readable for good.
        if finger_wake.as_ref().is_some_and(|wake| !drain_open(wake)) {
            finger_wake = None;
        }
        while let Ok(text) = finger_rx.try_recv() {
            ui.fingerprint = text;
            changed = true;
        }
        let answered = auth_rx.try_recv();
        if let Ok(ok) = answered {
            if ok {
                enzui_shell::power_boost::request();
                ui.phase = ui::Phase::Leaving(Instant::now());
            } else {
                ui.phase = ui::Phase::Idle;
                ui.password.clear();
                ui.error = Some((
                    tr("shell-incorrect-password-try-again").into(),
                    Instant::now(),
                ));
            }
        }
        let now = Instant::now();
        if let Some((code, due)) = state.held
            && now >= due
            && let Some(keyboard) = state.keyboard.as_ref()
        {
            state.keys.push(keyboard.key(code));
            let rate = state.repeat.0.max(1) as u64;
            state.held = Some((code, now + Duration::from_millis(1000 / rate)));
        }
        if std::mem::take(&mut state.input) && matches!(ui.phase, ui::Phase::Idle) {
            let resting = ui
                .last_input
                .is_none_or(|at| now.saturating_duration_since(at) >= ui::PROMPT_IDLE);
            if resting || !state.keys.is_empty() {
                enzui_shell::power_boost::request();
            }
            ui.last_input = Some(now);
        }
        ui.caps_lock = state
            .keyboard
            .as_ref()
            .is_some_and(keys::Keyboard::caps_lock);
        let mut submit = false;
        let mut dirty = !state.keys.is_empty() || std::mem::take(&mut redraw);
        for key in std::mem::take(&mut state.keys) {
            if ui.phase != ui::Phase::Idle {
                continue;
            }
            // Away from the password field, Enter and Space press the focused
            // control; typing goes back to the field.
            let on_field = ui.target == ui::Target::Field;
            match key {
                keys::Key::Tab => ui.move_focus(true),
                keys::Key::BackTab => ui.move_focus(false),
                keys::Key::Enter if !on_field => actions.push(ui.target.message()),
                keys::Key::Text(text) if !on_field && text == " " => {
                    actions.push(ui.target.message());
                }
                keys::Key::Text(_) | keys::Key::Backspace | keys::Key::DeleteWord
                    if ui.power_menu => {}
                keys::Key::Text(text) if ui.password.chars().count() < 256 => {
                    ui.target = ui::Target::Field;
                    ui.password.push_str(&text);
                    ui.error = None;
                }
                keys::Key::Backspace => {
                    ui.password.pop();
                }
                keys::Key::DeleteWord => ui.password.clear(),
                keys::Key::Escape if ui.power_menu => {
                    ui.power_menu = false;
                    ui.target = ui::Target::PowerMenu;
                }
                keys::Key::Escape => {
                    ui.password.clear();
                    ui.power_menu = false;
                    ui.error = None;
                    ui.last_input = None;
                    ui.target = ui::Target::Field;
                    ui.keyboard = false;
                }
                keys::Key::Enter => submit = true,
                keys::Key::Up | keys::Key::Left if ui.power_menu => ui.move_focus(false),
                keys::Key::Down | keys::Key::Right if ui.power_menu => ui.move_focus(true),
                keys::Key::Up if ui.password.is_empty() => actions.push(ui::Message::PreviousUser),
                keys::Key::Down if ui.password.is_empty() => actions.push(ui::Message::NextUser),
                _ => {}
            }
        }

        let dt = now.saturating_duration_since(last).as_secs_f32().min(0.1);
        last = now;
        dirty |= answered.is_ok() || changed;
        let animating = ui.tick(now, dt) || !revealed || dirty;
        let covering = ui.covers(now);
        for index in 0..state.surfaces.len() {
            let surface = &mut state.surfaces[index];
            if state.outputs[surface.output].removed {
                continue;
            }
            // Frames follow the compositor's refresh: the next one waits for
            // the callback asked for with the last, so an animation presents
            // once per refresh instead of on a timer that beats against it.
            // A compositor holds the callbacks of a surface it does not show
            // (an output that is off), which then draws at a slow fallback.
            let waiting = surface.software_pending >= 2
                || surface
                    .frame_pending
                    .is_some_and(|t| now.saturating_duration_since(t) < FRAME_FALLBACK);
            // Idle frames only refresh the clock, which shows whole minutes.
            let minute = clock_minute();
            let current = surface.events.is_empty() && surface.drawn == Some(minute);
            let pass = frame_pass(&mut surface.stale, waiting, animating, current);
            if pass == Pass::Wait {
                continue;
            }
            if surface.images.is_none() {
                continue;
            }
            let scale = surface.scale(&state.outputs);
            let rescaled = surface.apply_scale(scale);
            let physical = scale.physical(surface.size);
            if surface.gpu.is_none() {
                let hardware = gpu.as_ref().and_then(|gpu| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let target =
                            raw_surface(&gpu.instance, &connection, &surface.surface).ok()?;
                        let caps = target.get_capabilities(&gpu.adapter);
                        if caps.formats.is_empty() || caps.alpha_modes.is_empty() {
                            return None;
                        }
                        let format = configure(gpu, &target, physical, xray);
                        let engine = Engine::new(
                            &gpu.adapter,
                            gpu.device.clone(),
                            gpu.queue.clone(),
                            format,
                            None,
                            GraphicsShell::headless(),
                        );
                        Some(SurfaceGpu {
                            surface: Some(target),
                            renderer: iced::Renderer::Primary({
                                let mut renderer = WgpuRenderer::new(engine, renderer_settings());
                                // Frames are kept in a canvas: glass copies
                                // its backdrop from it rather than from a
                                // full-screen scene drawn and blitted back.
                                renderer.track_frame_damage(true);
                                renderer
                            }),
                            format,
                            physical,
                        })
                    }))
                    .ok()
                    .flatten()
                });
                let target = hardware.unwrap_or_else(|| {
                    eprintln!("enzui-lock: output {index} uses tiny-skia");
                    SurfaceGpu {
                        surface: None,
                        renderer: iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
                            renderer_settings(),
                        )),
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        physical,
                    }
                });
                surface.gpu = Some(target);
            }
            let target = surface
                .gpu
                .as_mut()
                .ok_or_else(|| tr("shell-lock-surface-renderer-was-not-prepared"))?;
            let resized = target.physical != physical;
            if resized {
                if let (Some(gpu), Some(surface)) = (&gpu, &target.surface) {
                    target.format = configure(gpu, surface, physical, xray);
                }
                target.physical = physical;
            }
            if pass == Pass::Skip && !resized && !rescaled {
                continue;
            }
            surface.drawn = Some(minute);
            surface.stale = false;
            let viewport = Viewport::with_physical_size(
                iced::Size::new(physical.0, physical.1),
                iced_wgpu::core::renderer::Scale {
                    window: scale.factor(),
                    application: 1.0,
                },
            );
            let logical = viewport.logical_size();
            let events = std::mem::take(&mut surface.events);
            let mut messages = iced_wgpu::core::shell::Bus::new();
            let Some(images) = surface.images.as_ref() else {
                continue;
            };
            let view = ui.view(images, index == primary, logical, now);
            let mut interface = UserInterface::build(
                view,
                logical,
                std::mem::take(&mut surface.cache),
                &mut target.renderer,
            );
            let _ = interface.update(
                &iced::window::Headless,
                &iced_wgpu::core::shell::Waker::noop(),
                &events,
                surface.cursor,
                &mut target.renderer,
                &mut messages,
            );
            interface.draw(
                &mut target.renderer,
                &theme,
                &Default::default(),
                surface.cursor,
            );
            surface.cache = interface.into_cache();
            for message in messages.drain_messages() {
                match message {
                    ui::Message::Submit => submit = true,
                    other => actions.push(other),
                }
            }
            surface.surface.frame(&qh, index);
            surface.frame_pending = Some(now);
            if let (Some(gpu), Some(output)) = (&gpu, &target.surface) {
                match output.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => {
                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        let clear = if xray {
                            iced::Color::TRANSPARENT
                        } else {
                            iced::Color::BLACK
                        };
                        if let iced::Renderer::Primary(renderer) = &mut target.renderer {
                            if frame
                                .texture
                                .usage()
                                .contains(wgpu::TextureUsages::COPY_DST)
                            {
                                let _ = renderer.present_damaged(
                                    clear,
                                    &frame.texture,
                                    &viewport,
                                    false,
                                );
                            } else {
                                renderer.present(Some(clear), target.format, &view, &viewport);
                            }
                        }
                        gpu.queue.present(frame);
                        surface.opaque |= covering;
                    }
                    _ => target.format = configure(gpu, output, physical, xray),
                }
            } else if let iced::Renderer::Secondary(renderer) = &mut target.renderer {
                software::present(
                    renderer,
                    state
                        .shm
                        .as_ref()
                        .ok_or_else(|| tr("shell-no-shared-memory-rendering-support"))?,
                    &surface.surface,
                    &qh,
                    &viewport,
                    xray,
                    index,
                )?;
                surface.software_pending += 1;
                surface.opaque |= covering;
            }
        }
        // `locked` releases the shell's sleep inhibitor, so it waits until the
        // desktop is hidden on every output, not just covered by a transparent frame.
        if !reported
            && state.locked.is_some_and(|at| {
                at.elapsed() >= OPAQUE_WAIT
                    || state
                        .surfaces
                        .iter()
                        .all(|s| s.opaque || !s.configured || state.outputs[s.output].removed)
            })
        {
            reported = true;
            say("locked");
        }
        let mut forwarded = Vec::new();
        if !actions.is_empty() || submit {
            busy_until = Instant::now() + Duration::from_secs(10);
        }
        for action in actions {
            redraw = true;
            match ui.local(action) {
                Some(ui::Message::Submit) => submit = true,
                Some(other) => forwarded.push(other),
                None => {}
            }
        }
        if let Some((requests, ..)) = &greeter {
            for action in forwarded {
                greeter_action(&mut ui, action, requests);
            }
        } else if forwarded.contains(&ui::Message::SwitchUser) {
            std::thread::spawn(|| {
                if let Err(error) = switch::open_greeter() {
                    eprintln!("enzui-lock: {error}");
                    *SWITCH_FAILED
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
                }
            });
        }
        let switch_error = SWITCH_FAILED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(error) = switch_error {
            ui.error = Some((error, Instant::now()));
            redraw = true;
        }
        let returning = ui.greeter.as_ref().and_then(|g| g.existing.clone());
        if let (Some(id), true) = (returning, submit && ui.phase == ui::Phase::Idle) {
            std::thread::spawn(move || {
                if let Err(error) = switch::activate_existing(&id) {
                    eprintln!("enzui-greeter: {error}");
                }
            });
        } else if let (Some((requests, ..)), true) = (
            &greeter,
            submit && ui.phase == ui::Phase::Idle && ui.answer_ready(),
        ) {
            ui.phase = ui::Phase::Checking;
            let _ = requests.send(greetd::Request::Answer(std::mem::take(&mut ui.password)));
        } else if submit && ui.phase == ui::Phase::Idle && !ui.password.is_empty() {
            ui.phase = ui::Phase::Checking;
            let password = std::mem::take(&mut ui.password);
            let tx = auth_tx.clone();
            std::thread::spawn(move || {
                // A failed attempt answers no sooner than 0.8 s, like login.
                let started = Instant::now();
                let ok = auth::check(&password);
                drop(password);
                if !ok {
                    std::thread::sleep(
                        Duration::from_millis(800).saturating_sub(started.elapsed()),
                    );
                }
                let _ = tx.send(ok);
            });
        }
        if ui.outro_done(Instant::now())
            && let Some((requests, ..)) = &greeter
        {
            // The prompt has faded; greetd now hands control to the authenticated session.
            if !launching {
                launching = true;
                launch(&mut ui, requests);
            }
        } else if ui.outro_done(Instant::now()) {
            lock.unlock_and_destroy();
            let _ = queue.roundtrip(&mut state);
            // The live wallpaper is left as it was: replaying its entrance
            // would jump away from the frame the lock just faded into.
            say("unlocked");
            return Ok(());
        }
        if state.finished {
            return Err(if state.locked.is_some() {
                tr("shell-the-compositor-ended-the-lock")
            } else {
                tr("shell-another-lock-screen-is-already-running")
            }
            .into());
        }
        access.update(&ui);
        // A live wallpaper's frames wake the loop themselves.
        let frame = if redraw || state.held.is_some() {
            Duration::from_millis(8)
        } else if animating {
            // A frame callback wakes the loop for the next frame.
            if state.surfaces.iter().any(|s| s.frame_pending.is_some()) {
                FRAME_FALLBACK
            } else {
                Duration::from_millis(8)
            }
        } else if Instant::now() < busy_until
            || greeter.as_ref().is_some_and(|(_, _, wake)| wake.is_none())
        {
            Duration::from_millis(250)
        } else {
            rest(&ui, Instant::now())
        };
        let wakes = live
            .as_ref()
            .map(live_background::Live::wakes)
            .into_iter()
            .chain(
                greeter
                    .as_ref()
                    .and_then(|(_, _, wake)| wake.as_ref().map(AsFd::as_fd)),
            )
            .chain(finger_wake.as_ref().map(AsFd::as_fd));
        pump(&connection, &mut queue, &mut state, frame, wakes)?;
    }
}

use greeter_session::{greeter_action, greeter_event, launch, start_greeter};

fn hyprland_xray() -> bool {
    use std::io::{Read, Write};
    let (Some(runtime), Some(instance)) = (
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
    ) else {
        return false;
    };
    let instance = instance.to_string_lossy();
    if instance.is_empty() || instance.contains('/') {
        return false;
    }
    let path = std::path::Path::new(&runtime)
        .join("hypr")
        .join(&*instance)
        .join(".socket.sock");
    let Ok(mut socket) = std::os::unix::net::UnixStream::connect(path) else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
    if socket
        .write_all(b"j/getoption misc:session_lock_xray")
        .is_err()
    {
        return false;
    }
    let mut reply = Vec::new();
    let _ = socket.take(4096).read_to_end(&mut reply);
    serde_json::from_slice::<serde_json::Value>(&reply).is_ok_and(|v| {
        v.get("bool") == Some(&serde_json::Value::Bool(true))
            || v.get("int").and_then(serde_json::Value::as_i64) == Some(1)
    })
}

#[cfg(test)]
mod wake_tests {
    use super::drain_open;

    #[test]
    fn a_wake_pipe_is_dropped_once_its_writer_closes() {
        let (read, write) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::NONBLOCK).expect("a pipe");
        assert!(drain_open(&read), "an empty pipe with a writer stays open");
        rustix::io::write(&write, &[1, 1]).expect("a wake");
        assert!(drain_open(&read), "pending wakes are drained");
        rustix::io::write(&write, &[1]).expect("a last wake");
        drop(write);
        assert!(
            !drain_open(&read),
            "a closed pipe is reported after its last wake"
        );
    }
}

#[cfg(test)]
mod frame_tests {
    use super::{Pass, REVEAL_WAIT, SurfaceReveal, frame_pass, reveal_due};
    use std::time::Duration;

    /// The lock fades in once every shown output has its backdrop, and after
    /// a bounded wait otherwise, so it never stays transparent over the desktop.
    #[test]
    fn reveal_waits_only_for_shown_outputs_and_only_briefly() {
        let surface = |removed, configured, backdrop| SurfaceReveal {
            removed,
            configured,
            backdrop,
        };
        let early = Duration::from_millis(10);
        let ready = surface(false, true, true);
        let gone = surface(true, false, false);
        let pending = surface(false, true, false);
        assert!(reveal_due([ready, gone], early, REVEAL_WAIT));
        assert!(!reveal_due(
            [surface(false, true, true), pending],
            early,
            REVEAL_WAIT
        ));
        assert!(reveal_due(
            [surface(false, true, true), surface(false, true, false)],
            REVEAL_WAIT,
            REVEAL_WAIT
        ));
    }

    /// A change seen while the surface waits for its frame callback is drawn
    /// once the callback arrives, with nothing else changing. A GL surface
    /// returns from its present before the callback comes, so the pass after
    /// a click waits; the clicked state then showed only on the next input.
    ///
    /// Sabotage: drop `*stale |= changed`, or `*stale` from the draw check.
    #[test]
    fn a_change_seen_while_waiting_is_drawn_after_the_wait() {
        let mut stale = false;
        assert_eq!(frame_pass(&mut stale, true, true, true), Pass::Wait);
        assert_eq!(frame_pass(&mut stale, false, false, true), Pass::Draw);
        // Once drawn (the loop clears `stale`), an unchanged surface rests.
        stale = false;
        assert_eq!(frame_pass(&mut stale, false, false, true), Pass::Skip);
        assert_eq!(frame_pass(&mut stale, true, false, true), Pass::Wait);
        assert_eq!(frame_pass(&mut stale, false, false, true), Pass::Skip);
    }
}

#[cfg(test)]
mod scale_tests {
    use super::Scale;

    #[test]
    fn a_fractional_scale_sizes_the_buffer_like_winit() {
        assert_eq!(Scale::Fractional(180).physical((1504, 1003)), (2256, 1505));
        assert_eq!(Scale::Fractional(150).physical((1707, 1067)), (2134, 1334));
        assert_eq!(Scale::Fractional(120).physical((1280, 800)), (1280, 800));
        assert_eq!(Scale::Fractional(180).factor(), 1.5);
    }

    #[test]
    fn an_integer_scale_multiplies_the_surface_size() {
        assert_eq!(Scale::Integer(2).physical((1504, 1003)), (3008, 2006));
        assert_eq!(Scale::Integer(1).physical((1504, 1003)), (1504, 1003));
    }
}
