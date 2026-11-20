use super::*;

impl Gpu {
    // Create graphics before locking so initialization failure leaves the session usable.
    pub(super) fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env()
                .unwrap_or(wgpu::Backends::VULKAN | wgpu::Backends::GL),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // The graphics adapter first, then a software renderer (llvmpipe,
        // lavapipe): signing in must work where the GPU driver does not.
        let (adapter, device, queue) = [false, true]
            .into_iter()
            .find_map(|software| {
                let adapter =
                    pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::LowPower,
                        compatible_surface: None,
                        force_fallback_adapter: software,
                        apply_limit_buckets: false,
                    }))
                    .ok()?;
                let (device, queue) = request_device(&adapter)?;
                Some((adapter, device, queue))
            })
            .ok_or_else(|| tr("shell-no-graphics-adapter-can-draw-the-lock-screen"))?;
        // Log uncaptured GPU errors so the lock prompt can continue accepting input.
        device.on_uncaptured_error(std::sync::Arc::new(|error: wgpu::Error| {
            eprintln!("enzui-lock: graphics error: {error}");
        }));
        let info = adapter.get_info();
        eprintln!(
            "enzui-lock: {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

#[allow(
    unsafe_code,
    reason = "wgpu needs native Wayland handles for ext-session-lock surfaces."
)]
pub(super) fn raw_surface(
    instance: &wgpu::Instance,
    connection: &Connection,
    surface: &wl_surface::WlSurface,
) -> Result<wgpu::Surface<'static>, String> {
    let display = NonNull::new(connection.backend().display_ptr() as *mut std::ffi::c_void)
        .ok_or_else(|| tr("shell-no-display-handle"))?;
    let window = NonNull::new(surface.id().as_ptr() as *mut std::ffi::c_void)
        .ok_or_else(|| tr("shell-no-surface-handle"))?;
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Wayland(
            wgpu::rwh::WaylandDisplayHandle::new(display),
        )),
        raw_window_handle: wgpu::rwh::RawWindowHandle::Wayland(
            wgpu::rwh::WaylandWindowHandle::new(window),
        ),
    };
    // SAFETY: State retains the Wayland connection and destroys each GPU surface before its lock surface.
    unsafe { instance.create_surface_unsafe(target) }
        .map_err(|_| tr("shell-cannot-draw-on-the-lock-surface").into())
}

fn request_device(adapter: &wgpu::Adapter) -> Option<(wgpu::Device, wgpu::Queue)> {
    // Try adapter limits before minimal iced limits for older or software drivers.
    let minimal =
        [wgpu::Limits::default(), wgpu::Limits::downlevel_defaults()].map(|limits| wgpu::Limits {
            max_bind_groups: 2,
            max_non_sampler_bindings: 2048,
            ..limits.using_resolution(adapter.limits())
        });
    [(adapter.features(), adapter.limits())]
        .into_iter()
        .chain(
            minimal
                .into_iter()
                .map(|limits| (adapter.features() & wgpu::Features::SHADER_F16, limits)),
        )
        .find_map(|(required_features, required_limits)| {
            pollster_block(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("enzui-lock"),
                required_features,
                required_limits,
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))
            .ok()
        })
}

fn pollster_block<F: std::future::Future>(future: F) -> F::Output {
    iced::futures::executor::block_on(future)
}

pub(super) fn configure(
    gpu: &Gpu,
    surface: &wgpu::Surface<'static>,
    physical: (u32, u32),
    transparent: bool,
) -> wgpu::TextureFormat {
    let caps = surface.get_capabilities(&gpu.adapter);
    let alpha_mode = if transparent
        && caps
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
    {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        wgpu::CompositeAlphaMode::Opaque
    };
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .unwrap_or(caps.formats[0]);
    surface.configure(
        &gpu.device,
        &wgpu::SurfaceConfiguration {
            // Copies let a frame be drawn into the renderer's kept canvas and
            // copied over (see `present_damaged`).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | (caps.usages & wgpu::TextureUsages::COPY_DST),
            format,
            width: physical.0,
            height: physical.1,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        },
    );
    format
}
