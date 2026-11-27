use enzui_shell::i18n::tr;
use iced::{Color, Rectangle, Size, border};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay_client_toolkit::reexports::client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    backend::{Backend, ObjectData, ObjectId, protocol::Message},
    protocol::{
        wl_callback, wl_compositor, wl_display, wl_region, wl_registry, wl_surface::WlSurface,
    },
};
use std::{
    any::Any,
    os::fd::OwnedFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all,
    clippy::pedantic,
    reason = "Wayland scanner generates protocol bindings with upstream naming and signatures."
)]
mod material_protocol {
    use wayland_client;
    use wayland_client::protocol::*;
    pub(super) mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../protocols/enzui-material-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("../protocols/enzui-material-v1.xml");
}
use material_protocol::{enzui_material_manager_v1, enzui_surface_material_v1};
use wayland_protocols::wp::alpha_modifier::v1::client::{
    wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, wp_alpha_modifier_v1,
    wp_alpha_modifier_v1::WpAlphaModifierV1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mask {
    Infinite,
    Spans(Vec<Span>),
}
#[derive(Clone)]
pub struct SurfaceInput(Arc<Inner>);
#[derive(Clone)]
pub struct Globals(Arc<GlobalState>);
struct GlobalState {
    connection: Connection,
    compositor: wl_compositor::WlCompositor,
    material: Option<enzui_material_manager_v1::EnzuiMaterialManagerV1>,
    /// The compositor's per-surface alpha factor, when it offers one.
    alpha: Option<WpAlphaModifierV1>,
    /// One alpha object per surface, whichever input holds it (a surface
    /// may have more than one, and a second object is a protocol error),
    /// with the factor it last sent.
    alphas: Mutex<Vec<(WlSurface, WpAlphaModifierSurfaceV1, u32)>>,
    // Keep the globals' queue alive, but let winit alone read its connection.
    // These globals have no events after discovery. Backdrop events use direct
    // ObjectData callbacks, which run when winit reads the shared backend.
    // A blocking reader here can drain UI events without waking winit's poll.
    _queue: Mutex<EventQueue<Discovery>>,
}
struct Inner {
    globals: Globals,
    surface: WlSurface,
    generation: u64,
    active: AtomicBool,
    last: Mutex<Option<Mask>>,
    glass: Mutex<Glass>,
    backdrop: Arc<Backdrop>,
    // `Some` while changes share one commit; true once one is owed.
    batch: Mutex<Option<bool>>,
}
#[derive(Default)]
struct Glass {
    object: Option<enzui_surface_material_v1::EnzuiSurfaceMaterialV1>,
    sent: Option<Vec<u8>>,
    shadow: (f32, f32, f32),
    measure_only: bool,
}
impl Glass {
    fn destroy(&mut self) -> bool {
        let Some(object) = self.object.take().filter(Proxy::is_alive) else {
            return false;
        };
        object.destroy();
        true
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(glass) = self.glass.get_mut()
            && glass.destroy()
        {
            let _ = self.globals.0.connection.flush();
        }
    }
}
#[derive(Default)]
struct Backdrop {
    luminance: Mutex<Vec<Option<f32>>>,
}
impl ObjectData for Backdrop {
    fn event(
        self: Arc<Self>,
        _: &Backend,
        message: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        use smithay_client_toolkit::reexports::client::backend::protocol::Argument;
        if let [Argument::Uint(region), Argument::Fixed(value)] = message.args.as_slice()
            && let Ok(mut luminance) = self.luminance.lock()
        {
            let region = *region as usize;
            if region < 32 {
                if luminance.len() <= region {
                    luminance.resize(region + 1, None);
                }
                luminance[region] = Some((*value as f32 / 256.0).clamp(0.0, 1.0));
                drop(luminance);
                wake().send_modify(|serial| *serial = serial.wrapping_add(1));
            }
        }
        None
    }
    fn destroyed(&self, _: ObjectId) {}
}
fn wake() -> &'static tokio::sync::watch::Sender<u64> {
    static WAKE: std::sync::OnceLock<tokio::sync::watch::Sender<u64>> = std::sync::OnceLock::new();
    WAKE.get_or_init(|| tokio::sync::watch::channel(0).0)
}
/// Wakes on compositor backdrop measurements so idle windows can update adaptive ink.
pub fn luminance_changes() -> tokio::sync::watch::Receiver<u64> {
    wake().subscribe()
}
/// One glass region: its rounded rectangle (surface-local, logical) and the
/// colour of its glass, which tells the compositor which ink it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassRegion {
    pub bounds: Rectangle,
    pub radius: border::Radius,
    pub tint: Color,
}
/// Defers [`SurfaceInput`] commits until dropped; see [`SurfaceInput::commit_together`].
pub struct CommitTogether<'a> {
    input: &'a SurfaceInput,
    owner: bool,
    with_buffer: bool,
}
impl CommitTogether<'_> {
    pub(crate) fn with_buffer(mut self) -> Self {
        self.with_buffer = true;
        self
    }
}
impl Drop for CommitTogether<'_> {
    fn drop(&mut self) {
        if !self.owner {
            return;
        }
        let inner = &self.input.0;
        let owed = inner
            .batch
            .lock()
            .ok()
            .and_then(|mut batch| batch.take())
            .unwrap_or(false);
        if owed && inner.surface.is_alive() {
            if !self.with_buffer {
                inner.surface.commit();
            }
            let _ = inner.globals.0.connection.flush();
        }
    }
}
impl std::fmt::Debug for SurfaceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceInput")
            .field("generation", &self.0.generation)
            .finish_non_exhaustive()
    }
}
impl SurfaceInput {
    pub fn globals(&self) -> Globals {
        self.0.globals.clone()
    }
    fn from_parts(
        surface: WlSurface,
        generation: u64,
        globals: Globals,
        claim_input: bool,
    ) -> Result<Self, String> {
        let result = Self(Arc::new(Inner {
            globals,
            surface,
            generation,
            active: AtomicBool::new(true),
            last: Mutex::new(None),
            glass: Mutex::new(Glass::default()),
            backdrop: Arc::new(Backdrop::default()),
            batch: Mutex::new(None),
        }));
        if claim_input && !result.empty() {
            return Err(tr("shell-could-not-initialize-input-region").into());
        }
        Ok(result)
    }
    /// Scales the whole surface's alpha in the compositor (0 to 1), without a
    /// repaint: a fade that keeps time with the frame helper's, which fades
    /// the same way. Returns false where the compositor has no alpha factor.
    pub fn set_alpha(&self, alpha: f32) -> bool {
        let globals = &self.0.globals.0;
        let Some(manager) = globals.alpha.as_ref().filter(|m| m.is_alive()) else {
            return false;
        };
        if !self.0.active.load(Ordering::Acquire) || !self.0.surface.is_alive() {
            return false;
        }
        let Ok(mut alphas) = globals.alphas.lock() else {
            return false;
        };
        // Objects of surfaces that are gone go with them.
        alphas.retain(|(surface, object, _)| {
            let alive = surface.is_alive();
            if !alive {
                object.destroy();
            }
            alive
        });
        let factor = (f64::from(alpha.clamp(0.0, 1.0)) * f64::from(u32::MAX)).round() as u32;
        let index = match alphas
            .iter()
            .position(|(surface, _, _)| *surface == self.0.surface)
        {
            Some(index) => index,
            None => {
                let Ok(object) = manager.send_constructor::<WpAlphaModifierSurfaceV1>(
                    wp_alpha_modifier_v1::Request::GetSurface {
                        surface: self.0.surface.clone(),
                    },
                    Arc::new(NoEvents),
                ) else {
                    return false;
                };
                alphas.push((self.0.surface.clone(), object, u32::MAX));
                alphas.len() - 1
            }
        };
        let (_, object, sent) = &mut alphas[index];
        if *sent == factor {
            return true;
        }
        object.set_multiplier(factor);
        *sent = factor;
        // Double-buffered like the rest of the surface's state.
        self.0.surface.commit();
        globals.connection.flush().is_ok()
    }
    /// Commits input and glass updates together when the guard drops.
    pub fn commit_together(&self) -> CommitTogether<'_> {
        let owner = self.0.batch.lock().is_ok_and(|mut batch| {
            let owner = batch.is_none();
            batch.get_or_insert(false);
            owner
        });
        CommitTogether {
            input: self,
            owner,
            with_buffer: false,
        }
    }
    fn commit(&self) -> bool {
        if let Ok(mut batch) = self.0.batch.lock()
            && let Some(owed) = batch.as_mut()
        {
            *owed = true;
            return true;
        }
        self.0.surface.commit();
        self.0.globals.0.connection.flush().is_ok()
    }
    pub fn invalidate(&self) {
        self.0.active.store(false, Ordering::Release);
        // The sidebar surface survives Frame/Islands changes. Release its
        // material before a replacement input owner can bind that surface.
        if let Ok(mut glass) = self.0.glass.lock()
            && glass.destroy()
        {
            let _ = self.0.globals.0.connection.flush();
        }
    }
    pub fn reset(&self) -> bool {
        self.apply(Mask::Infinite)
    }
    pub fn empty(&self) -> bool {
        self.apply(Mask::Spans(Vec::new()))
    }
    pub fn rectangle(&self, width: i32, height: i32) -> bool {
        if width <= 0 || height <= 0 {
            return false;
        }
        self.apply(Mask::Spans(vec![Span {
            x: 0,
            y: 0,
            width,
            height,
        }]))
    }
    /// Input only inside these rectangles (surface-local, logical).
    pub fn rectangles(&self, rectangles: &[Rectangle]) -> bool {
        let spans: Vec<Span> = rectangles
            .iter()
            .filter(|rect| rect.width >= 1.0 && rect.height >= 1.0)
            .map(|rect| Span {
                x: rect.x.round() as i32,
                y: rect.y.round() as i32,
                width: rect.width.round() as i32,
                height: rect.height.round() as i32,
            })
            .collect();
        self.apply(Mask::Spans(spans))
    }
    pub fn rounded_rectangles(&self, rectangles: &[Rectangle], radius: f32) -> bool {
        self.rounded_regions(&rectangles.iter().map(|r| (*r, radius)).collect::<Vec<_>>())
    }
    pub fn rounded_regions(&self, regions: &[(Rectangle, f32)]) -> bool {
        let canvas = regions.iter().fold(Size::ZERO, |size, (r, _)| {
            Size::new(
                size.width.max(r.x + r.width),
                size.height.max(r.y + r.height),
            )
        });
        let fills: Vec<_> = regions
            .iter()
            .map(|&(bounds, radius)| (bounds, radius.into()))
            .collect();
        self.shaped_regions(&fills, canvas)
    }
    pub(crate) fn shaped_regions(
        &self,
        regions: &[(Rectangle, border::Radius)],
        canvas: Size,
    ) -> bool {
        match raster(regions, &[], canvas) {
            Some(spans) => self.apply(Mask::Spans(spans)),
            None => {
                self.empty();
                false
            }
        }
    }
    pub fn shape(&self, frame: &crate::reveal::Frame, canvas: Size) -> bool {
        match raster(&frame.fills, &frame.corners, canvas) {
            Some(spans) => self.apply(Mask::Spans(spans)),
            None => {
                self.empty();
                false
            }
        }
    }
    /// Sets surface-local compositor glass regions; returns false when callers must paint a
    /// fallback.
    /// Shadow components are blur, downward offset and opacity.
    pub fn glass(
        &self,
        regions: &[(Rectangle, f32)],
        tokens: &enzui_theme::MaterialTokens,
        tint: Color,
        shadow: Option<(f32, f32, f32)>,
    ) -> bool {
        let regions: Vec<GlassRegion> = regions
            .iter()
            .map(|&(bounds, radius)| GlassRegion {
                bounds,
                radius: radius.into(),
                tint,
            })
            .collect();
        self.glass_regions(&regions, tokens, shadow)
    }
    /// [`Self::glass`] with each region's own tint. The `i`th region's
    /// backdrop brightness is [`Self::luminance`]`(i)`.
    pub fn glass_regions(
        &self,
        regions: &[GlassRegion],
        tokens: &enzui_theme::MaterialTokens,
        shadow: Option<(f32, f32, f32)>,
    ) -> bool {
        self.mixed_glass_regions(regions.iter().map(|region| (*region, *tokens)), shadow)
    }

    pub(crate) fn mixed_glass_regions(
        &self,
        regions: impl IntoIterator<Item = (GlassRegion, enzui_theme::MaterialTokens)>,
        shadow: Option<(f32, f32, f32)>,
    ) -> bool {
        let independent_corners = self.glass_supports_corners();
        let bytes: Vec<u8> = regions
            .into_iter()
            .take(32)
            .flat_map(|(region, tokens)| {
                let tuned = crate::reveal::glass_tweaks().apply(shell_frost(&tokens, region.tint));
                let t = &tuned;
                let rect = region.bounds;
                let tint = light_frost_tint(&tokens, region.tint)
                    .into_rgba8()
                    .map(|channel| f32::from(channel) / 255.0);
                let radius = region.radius;
                [
                    rect.x,
                    rect.y,
                    rect.width.max(0.0),
                    rect.height.max(0.0),
                    radius.top_left.max(0.0),
                    t.blur_sigma,
                    t.refraction,
                    t.bevel_width,
                    t.index_of_refraction,
                    t.dispersion,
                    t.color_diffusion,
                    t.color_reach,
                    t.optical_depth,
                    t.tint_opacity,
                    t.saturation,
                    t.highlight,
                    t.brightness,
                    t.light_angle,
                    t.profile_curvature,
                    t.contrast_protection,
                    t.protected_inset,
                    tint[0],
                    tint[1],
                    tint[2],
                    tint[3],
                ]
                .into_iter()
                .chain(
                    [radius.top_right, radius.bottom_left, radius.bottom_right]
                        .into_iter()
                        .map(|r| r.max(0.0))
                        .take(if independent_corners { 3 } else { 0 }),
                )
            })
            .map(|value| if value.is_finite() { value } else { 0.0 })
            .flat_map(f32::to_le_bytes)
            .collect();
        self.send_glass(bytes, shadow.unwrap_or_default())
    }
    /// Returns the last perceptual luminance (0–1), or `None` before measurement or without
    /// support.
    pub fn luminance(&self, index: usize) -> Option<f32> {
        self.0
            .backdrop
            .luminance
            .lock()
            .ok()?
            .get(index)
            .copied()
            .flatten()
    }
    /// The average of what the compositor last measured behind the current
    /// glass's regions, or `None` before any was measured.
    pub fn average_luminance(&self) -> Option<f32> {
        let luminance = self.0.backdrop.luminance.lock().ok()?;
        let measured: Vec<f32> = luminance.iter().flatten().copied().collect();
        (!measured.is_empty()).then(|| measured.iter().sum::<f32>() / measured.len() as f32)
    }
    /// Whether this surface currently has glass the compositor accepted.
    pub fn has_glass(&self) -> bool {
        self.0.glass.lock().is_ok_and(|glass| {
            glass.object.is_some() && glass.sent.as_ref().is_some_and(|sent| !sent.is_empty())
        })
    }
    /// Whether the compositor reports backdrop brightness at all.
    pub fn measures(&self) -> bool {
        self.0
            .globals
            .0
            .material
            .as_ref()
            .is_some_and(|manager| manager.is_alive() && manager.version() >= 2)
    }
    /// Commits measure-only mode so adaptive glass can wait for its initial backdrop reading.
    pub fn set_measure_only(&self, measure_only: bool) {
        let Ok(mut glass) = self.0.glass.lock() else {
            return;
        };
        if glass.measure_only == measure_only || !self.measures() {
            return;
        }
        if let Some(object) = glass.object.as_ref().filter(|object| object.is_alive()) {
            object.set_measure_only(u32::from(measure_only));
            glass.measure_only = measure_only;
        }
    }
    /// Whether the compositor paints glass for this surface at all.
    pub fn glass_supported(&self) -> bool {
        self.0
            .globals
            .0
            .material
            .as_ref()
            .is_some_and(wayland_client::Proxy::is_alive)
    }
    pub fn glass_supports_corners(&self) -> bool {
        self.0
            .globals
            .0
            .material
            .as_ref()
            .is_some_and(|manager| manager.is_alive() && manager.version() >= 4)
    }
    pub fn clear_glass(&self) {
        let _ = self.send_glass(Vec::new(), Default::default());
    }
    /// Finish hiding a retained surface after its content animation reaches zero.
    /// The last empty pixel buffer can be reused, so clearing its material needs
    /// an explicit commit even when neither pixels nor the input mask changes.
    pub fn finish_hidden(&self) {
        self.clear_glass();
        if self.empty() {
            self.0.surface.commit();
            let _ = self.0.globals.0.connection.flush();
        }
    }
    fn send_glass(&self, bytes: Vec<u8>, shadow: (f32, f32, f32)) -> bool {
        let Some(manager) = self
            .0
            .globals
            .0
            .material
            .as_ref()
            .filter(|manager| manager.is_alive())
        else {
            return false;
        };
        if !self.0.active.load(Ordering::Acquire) || !self.0.surface.is_alive() {
            return false;
        }
        let Ok(mut glass) = self.0.glass.lock() else {
            return false;
        };
        // Invalidation may have acquired the lock after the check above.
        if !self.0.active.load(Ordering::Acquire) {
            return false;
        }
        if glass.sent.as_ref() == Some(&bytes) && glass.shadow == shadow {
            return !bytes.is_empty();
        }
        if glass.object.is_none() {
            if bytes.is_empty() {
                return false;
            }
            glass.object = manager
                .send_constructor(
                    enzui_material_manager_v1::Request::GetSurfaceMaterial {
                        surface: self.0.surface.clone(),
                    },
                    std::sync::Arc::<Backdrop>::clone(&self.0.backdrop),
                )
                .ok();
        }
        let Some(object) = glass.object.clone() else {
            return false;
        };
        let empty = bytes.is_empty();
        if glass.shadow != shadow {
            object.set_shadow(
                f64::from(shadow.0),
                f64::from(shadow.1),
                f64::from(shadow.2),
            );
            glass.shadow = shadow;
        }
        let independent_corners = object.version() >= 4;
        let record_bytes = if independent_corners { 28 * 4 } else { 25 * 4 };
        let count = bytes.len() / record_bytes;
        if glass
            .sent
            .as_ref()
            .is_none_or(|sent| sent.len() / record_bytes != count)
            && let Ok(mut luminance) = self.0.backdrop.luminance.lock()
        {
            luminance.clear();
        }
        if independent_corners {
            object.set_regions_with_corners(bytes.clone());
        } else {
            object.set_regions(bytes.clone());
        }
        glass.sent = Some(bytes);
        // Double-buffered: an unchanged pixel buffer may be reused (a new
        // shell colour waited for the content's next repaint), so the
        // regions need their own surface commit, like the input mask.
        self.commit() && !empty
    }
    fn apply(&self, mask: Mask) -> bool {
        if !self.0.active.load(Ordering::Acquire)
            || !self.0.surface.is_alive()
            || !self.0.globals.0.compositor.is_alive()
        {
            return false;
        }
        let Ok(mut previous) = self.0.last.lock() else {
            return false;
        };
        if previous.as_ref() == Some(&mask) {
            return true;
        }
        match &mask {
            Mask::Infinite => self.0.surface.set_input_region(None),
            Mask::Spans(spans) => {
                let region: wl_region::WlRegion =
                    match self.0.globals.0.compositor.send_constructor(
                        wl_compositor::Request::CreateRegion {},
                        Arc::new(NoEvents),
                    ) {
                        Ok(region) => region,
                        Err(_) => return false,
                    };
                for span in spans {
                    region.add(span.x, span.y, span.width, span.height);
                }
                self.0.surface.set_input_region(Some(&region));
                region.destroy();
            }
        }
        *previous = Some(mask);
        // An unchanged pixel buffer may be reused; input still needs its own surface commit.
        self.commit()
    }
}
/// Waits for prior requests on the owned window's display without a second reader.
/// This lets focus restoration follow a destroyed surface without a fixed delay.
pub async fn synchronize(payload: Option<Box<dyn Any + Send>>) -> bool {
    let Some(payload) = payload else {
        return false;
    };
    let Ok(parts) = payload.downcast::<(Connection, WlSurface, u64)>() else {
        return false;
    };
    let (connection, _, _) = *parts;
    let (send, done) = tokio::sync::oneshot::channel();
    let callback: Result<wl_callback::WlCallback, _> = connection.display().send_constructor(
        wl_display::Request::Sync {},
        Arc::new(SyncDone(Mutex::new(Some(send)))),
    );
    let Ok(_callback) = callback else {
        return false;
    };
    if connection.flush().is_err() {
        return false;
    }
    tokio::time::timeout(Duration::from_millis(250), done)
        .await
        .is_ok_and(|result| result.is_ok())
}
struct SyncDone(Mutex<Option<tokio::sync::oneshot::Sender<()>>>);
impl ObjectData for SyncDone {
    fn event(
        self: Arc<Self>,
        _: &Backend,
        _: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        if let Some(send) = self.0.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = send.send(());
        }
        None
    }
    fn destroyed(&self, _: ObjectId) {}
}
struct NoEvents;
impl ObjectData for NoEvents {
    fn event(
        self: Arc<Self>,
        _: &Backend,
        _: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _: ObjectId) {}
}
#[derive(Default)]
struct Discovery {
    compositor: Option<wl_compositor::WlCompositor>,
    material: Option<enzui_material_manager_v1::EnzuiMaterialManagerV1>,
    alpha: Option<WpAlphaModifierV1>,
    done: bool,
}
impl Dispatch<wl_registry::WlRegistry, ()> for Discovery {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_compositor" && state.compositor.is_none() {
                state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            } else if interface == "enzui_material_manager_v1" && state.material.is_none() {
                state.material = Some(registry.bind(name, version.min(4), qh, ()));
            } else if interface == "wp_alpha_modifier_v1" && state.alpha.is_none() {
                state.alpha = Some(registry.bind(name, 1, qh, ()));
            }
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for Discovery {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.done = true;
        }
    }
}
impl Dispatch<enzui_material_manager_v1::EnzuiMaterialManagerV1, ()> for Discovery {
    fn event(
        _: &mut Self,
        _: &enzui_material_manager_v1::EnzuiMaterialManagerV1,
        _: enzui_material_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WpAlphaModifierV1, ()> for Discovery {
    fn event(
        _: &mut Self,
        _: &WpAlphaModifierV1,
        _: wp_alpha_modifier_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_compositor::WlCompositor, ()> for Discovery {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
pub fn acquire(
    payload: Option<Box<dyn Any + Send>>,
    shared: Option<Globals>,
) -> Result<SurfaceInput, String> {
    acquire_surface(payload, shared, true)
}
/// The surface for glass only: its input region stays the compositor's.
pub fn acquire_for_glass(
    payload: Option<Box<dyn Any + Send>>,
    shared: Option<Globals>,
) -> Result<SurfaceInput, String> {
    acquire_surface(payload, shared, false)
}
fn acquire_surface(
    payload: Option<Box<dyn Any + Send>>,
    shared: Option<Globals>,
    claim_input: bool,
) -> Result<SurfaceInput, String> {
    let (connection, surface, generation) = *payload
        .ok_or_else(|| tr("shell-no-owned-wayland-surface"))?
        .downcast::<(Connection, WlSurface, u64)>()
        .map_err(|_| tr("shell-unsupported-owned-surface-payload"))?;
    if let Some(shared) = shared
        && shared.0.connection.backend() == connection.backend()
        && shared.0.compositor.is_alive()
    {
        return SurfaceInput::from_parts(surface, generation, shared, claim_input);
    }
    let mut queue = connection.new_event_queue::<Discovery>();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, ());
    let mut state = Discovery::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    let discovery = (|| {
        loop {
            queue
                .dispatch_pending(&mut state)
                .map_err(|_| tr("shell-input-region-registry-disconnected"))?;
            if state.done {
                break;
            }
            if !surface.is_alive() {
                return Err(tr("shell-input-region-surface-closed"));
            }
            if Instant::now() >= deadline {
                return Err(tr("shell-input-region-registry-timed-out"));
            }
            connection
                .flush()
                .map_err(|_| tr("shell-input-region-connection-closed"))?;
            let Some(guard) = connection.prepare_read() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            let mut fds = [PollFd::from_borrowed_fd(
                guard.connection_fd(),
                PollFlags::IN,
            )];
            match poll(
                &mut fds,
                Some(&Timespec {
                    tv_sec: 0,
                    tv_nsec: 25_000_000,
                }),
            ) {
                Ok(0) | Err(rustix::io::Errno::INTR) => drop(guard),
                Ok(_) if fds[0].revents().contains(PollFlags::IN) => {
                    guard
                        .read()
                        .map_err(|_| tr("shell-input-region-connection-closed"))?;
                }
                _ => {
                    return Err(tr("shell-input-region-connection-failed"));
                }
            }
        }
        Ok(())
    })();
    // wl_registry has no destructor request; release its local dispatch route after synchronization.
    let _ = connection.backend().destroy_object(&registry.id());
    discovery?;
    let compositor = state
        .compositor
        .ok_or_else(|| tr("shell-input-regions-unsupported"))?;
    SurfaceInput::from_parts(
        surface,
        generation,
        Globals(Arc::new(GlobalState {
            connection,
            compositor,
            material: state.material,
            alpha: state.alpha,
            alphas: Mutex::new(Vec::new()),
            _queue: Mutex::new(queue),
        })),
        claim_input,
    )
}
pub fn content_regions(frame: &crate::reveal::Frame, canvas: Size) -> Vec<Rectangle> {
    let Some(body) = frame.fills.iter().find(|(bounds, _)| *bounds == frame.clip) else {
        return Vec::new();
    };
    raster(std::slice::from_ref(body), &[], canvas)
        .unwrap_or_default()
        .into_iter()
        .map(|span| Rectangle {
            x: span.x as f32,
            y: span.y as f32,
            width: span.width as f32,
            height: span.height as f32,
        })
        .collect()
}

fn raster(
    fills: &[(Rectangle, border::Radius)],
    corners: &[(Rectangle, bool, bool)],
    canvas: Size,
) -> Option<Vec<Span>> {
    if !canvas.width.is_finite()
        || !canvas.height.is_finite()
        || !(1.0..=16384.0).contains(&canvas.width)
        || !(1.0..=16384.0).contains(&canvas.height)
        || fills.len() > 16
        || corners.len() > 16
    {
        return None;
    }
    let mut spans: Vec<Span> = Vec::new();
    for &(bounds, radius) in fills {
        if ![
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            radius.top_left,
            radius.top_right,
            radius.bottom_left,
            radius.bottom_right,
        ]
        .into_iter()
        .all(f32::is_finite)
            || bounds.x.abs() > 16384.0
            || bounds.y.abs() > 16384.0
            || !(0.0..=16384.0).contains(&bounds.width)
            || !(0.0..=16384.0).contains(&bounds.height)
        {
            return None;
        }
        let limit = f64::from(bounds.width.min(bounds.height)) / 2.0;
        let r = |value: f32| f64::from(value).clamp(0.0, limit);
        let left = f64::from(bounds.x);
        let right = left + f64::from(bounds.width);
        let top = f64::from(bounds.y);
        let bottom = top + f64::from(bounds.height);
        let inset = |y: f64, upper: f32, lower: f32| {
            let upper = r(upper);
            let lower = r(lower);
            let (radius, distance) = if y < top + upper {
                (upper, top + upper - y)
            } else if y > bottom - lower {
                (lower, y - (bottom - lower))
            } else {
                (0.0, 0.0)
            };
            radius - (radius * radius - distance * distance).max(0.0).sqrt()
        };
        let start = (top - 0.5).ceil().max(0.0) as i32;
        let end = (bottom - 0.5).ceil().min(f64::from(canvas.height).ceil()) as i32;
        for y in start..end {
            let row = f64::from(y) + 0.5;
            let x = (left + inset(row, radius.top_left, radius.bottom_left) - 0.5)
                .ceil()
                .max(0.0) as i32;
            let stop = (right - inset(row, radius.top_right, radius.bottom_right) - 0.5)
                .ceil()
                .min(f64::from(canvas.width).ceil()) as i32;
            if stop <= x {
                continue;
            }
            if let Some(last) = spans.last_mut()
                && last.x == x
                && last.width == stop - x
                && last.y + last.height == y
            {
                last.height += 1;
            } else {
                spans.push(Span {
                    x,
                    y,
                    width: stop - x,
                    height: 1,
                });
            }
        }
    }
    for &(bounds, right, bottom) in corners {
        if ![bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f32::is_finite)
            || bounds.x.abs() > 16384.0
            || bounds.y.abs() > 16384.0
            || !(0.0..=16384.0).contains(&bounds.width)
            || !(0.0..=16384.0).contains(&bounds.height)
        {
            return None;
        }
        if bounds.width == 0.0 || bounds.height == 0.0 {
            continue;
        }
        let left = f64::from(bounds.x);
        let top = f64::from(bounds.y);
        let width = f64::from(bounds.width);
        let height = f64::from(bounds.height);
        let start = (top - 0.5).ceil().max(0.0) as i32;
        let end = (top + height - 0.5)
            .ceil()
            .min(f64::from(canvas.height).ceil()) as i32;
        for y in start..end {
            let v = (f64::from(y) + 0.5 - top) / height;
            let distance = if bottom { v } else { 1.0 - v };
            let cut = (1.0 - distance * distance).max(0.0).sqrt() * width;
            let x = (left + if right { cut } else { 0.0 } - 0.5).ceil().max(0.0) as i32;
            let stop = (left + width - if right { 0.0 } else { cut } - 0.5)
                .ceil()
                .min(f64::from(canvas.width).ceil()) as i32;
            if stop <= x {
                continue;
            }
            if let Some(last) = spans.last_mut()
                && last.x == x
                && last.width == stop - x
                && last.y + last.height == y
            {
                last.height += 1;
            } else {
                spans.push(Span {
                    x,
                    y,
                    width: stop - x,
                    height: 1,
                });
            }
        }
    }
    Some(spans)
}

fn light_frost_tint(tokens: &enzui_theme::MaterialTokens, tint: Color) -> Color {
    let light = 0.2126 * tint.r + 0.7152 * tint.g + 0.0722 * tint.b > 0.5;
    if !light || *tokens != enzui_theme::MaterialStyle::FlatFrosted.builtin_tokens() {
        return tint;
    }
    let f = 0.45;
    Color {
        r: tint.r + (1.0 - tint.r) * f,
        g: tint.g + (1.0 - tint.g) * f,
        b: tint.b + (1.0 - tint.b) * f,
        a: tint.a,
    }
}

/// A material as the shell paints it, over a light or dark tint: what
/// Settings shows as each glass setting's starting value.
pub fn shell_material(
    style: enzui_theme::MaterialStyle,
    light: bool,
) -> enzui_theme::MaterialTokens {
    shell_frost(
        &style.tokens(),
        if light { Color::WHITE } else { Color::BLACK },
    )
}

// Default shell frost keeps theme ink legible over high-contrast desktops;
// explicit optical presets retain their own treatment.
pub(crate) fn shell_frost(
    tokens: &enzui_theme::MaterialTokens,
    tint: Color,
) -> enzui_theme::MaterialTokens {
    if *tokens != enzui_theme::MaterialStyle::FlatFrosted.builtin_tokens() {
        return *tokens;
    }
    let light = 0.2126 * tint.r + 0.7152 * tint.g + 0.0722 * tint.b > 0.5;
    enzui_theme::MaterialTokens {
        tint_opacity: if light { 0.82 } else { 0.94 },
        blur_sigma: 20.0,
        saturation: 1.0,
        ..*tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_input_behaviour() {
        // Malformed geometry is rejected and empty shapes have no input.
        {
            assert_eq!(raster(&[], &[], Size::new(320.0, 640.0)), Some(vec![]));
            assert!(raster(&[], &[], Size::new(f32::NAN, 640.0)).is_none());
            let shape = Rectangle {
                x: 0.0,
                y: 0.0,
                width: f32::INFINITY,
                height: 10.0,
            };
            assert!(raster(&[(shape, 0.0.into())], &[], Size::new(320.0, 640.0)).is_none());
        }
        // Partially clipped shapes keep the original curve origin.
        {
            let shape = Rectangle {
                x: -10.0,
                y: -10.0,
                width: 40.0,
                height: 40.0,
            };
            let spans = raster(&[(shape, 20.0.into())], &[], Size::new(20.0, 20.0)).unwrap();
            assert!(
                spans
                    .iter()
                    .all(|s| s.x >= 0 && s.y >= 0 && s.x + s.width <= 20 && s.y + s.height <= 20)
            );
            assert!(spans.iter().any(|s| s.x == 0 && s.y == 0));
        }
        {
            let (input, mut server) = wire_fixture();
            let surface = input.0.surface.id().protocol_id();
            let tokens = enzui_theme::MaterialStyle::FlatFrosted.builtin_tokens();
            let panel = Rectangle::new(iced::Point::ORIGIN, Size::new(120.0, 80.0));
            let mut frame = |size: f32, shadow| {
                let outer = input.commit_together();
                let inner = input.commit_together();
                let bounds = Rectangle::new(iced::Point::ORIGIN, Size::new(size, size));
                input.rounded_regions(&[(bounds, 12.0)]);
                input.glass(&[(bounds, 12.0)], &tokens, Color::BLACK, shadow);
                drop(inner);
                assert!(requests(&mut server).is_empty());
                drop(outer);
                requests(&mut server)
            };
            let open = frame(40.0, Some((8.0, 2.0, 0.3)));
            let commits = |sent: &[(u32, u16)]| {
                sent.iter()
                    .filter(|&&request| request == (surface, SURFACE_COMMIT))
                    .count()
            };
            assert_eq!(commits(&open), 1);
            assert_eq!(open.last(), Some(&(surface, SURFACE_COMMIT)));
            assert!(open.contains(&(surface, SURFACE_SET_INPUT_REGION)));
            assert!(
                open.iter()
                    .any(|&(id, opcode)| id != surface && opcode == MATERIAL_SET_REGIONS)
            );
            assert!(frame(40.0, Some((8.0, 2.0, 0.3))).is_empty());
            assert_eq!(commits(&frame(64.0, Some((8.0, 2.0, 0.3)))), 1);

            assert!(input.rectangles(&[panel]));
            assert_eq!(
                requests(&mut server).last(),
                Some(&(surface, SURFACE_COMMIT))
            );
            let hide = input.commit_together().with_buffer();
            input.clear_glass();
            input.empty();
            drop(hide);
            let pending = requests(&mut server);
            assert!(pending.contains(&(surface, SURFACE_SET_INPUT_REGION)));
            assert!(
                pending
                    .iter()
                    .any(|&(id, opcode)| id != surface && opcode == MATERIAL_SET_REGIONS)
            );
            assert_eq!(commits(&pending), 0);

            input.0.surface.commit();
            input.0.globals.0.connection.flush().unwrap();
            assert_eq!(commits(&requests(&mut server)), 1);
        }
    }

    const SURFACE_SET_INPUT_REGION: u16 = 5;
    const SURFACE_COMMIT: u16 = 6;
    const MATERIAL_SET_REGIONS: u16 = 1;

    // Client objects are created locally, so the peer only has to read requests.
    fn wire_fixture() -> (SurfaceInput, std::os::unix::net::UnixStream) {
        let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        server.set_nonblocking(true).unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let queue = connection.new_event_queue::<Discovery>();
        let qh = queue.handle();
        let registry = connection.display().get_registry(&qh, ());
        let compositor: wl_compositor::WlCompositor = registry.bind(1, 4, &qh, ());
        let material: enzui_material_manager_v1::EnzuiMaterialManagerV1 =
            registry.bind(2, 2, &qh, ());
        let surface: WlSurface = compositor
            .send_constructor(wl_compositor::Request::CreateSurface {}, Arc::new(NoEvents))
            .unwrap();
        let globals = Globals(Arc::new(GlobalState {
            connection,
            compositor,
            material: Some(material),
            alpha: None,
            alphas: Mutex::new(Vec::new()),
            _queue: Mutex::new(queue),
        }));
        let input = SurfaceInput::from_parts(surface, 0, globals, false).unwrap();
        let mut server = server;
        requests(&mut server);
        (input, server)
    }

    fn requests(server: &mut std::os::unix::net::UnixStream) -> Vec<(u32, u16)> {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            match server.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        let mut sent = Vec::new();
        let mut rest = bytes.as_slice();
        while rest.len() >= 8 {
            let word = |at: usize| u32::from_le_bytes(rest[at..at + 4].try_into().unwrap());
            let (object, header) = (word(0), word(4));
            sent.push((object, (header & 0xffff) as u16));
            rest = &rest[(header >> 16) as usize..];
        }
        sent
    }
}
