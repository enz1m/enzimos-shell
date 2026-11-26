use super::*;

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let (name, interface, version) = match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => (name, interface, version),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some((index, output)) = state
                    .outputs
                    .iter_mut()
                    .enumerate()
                    .find(|(_, output)| output.global == name)
                {
                    output.removed = true;
                    if output.output.version() >= 3 {
                        output.output.release();
                    }
                    if let Some(capture) = output.capture.take() {
                        if let Some(frame) = capture.frame {
                            frame.destroy();
                        }
                        if let Some(buffer) = capture.buffer {
                            buffer.destroy();
                        }
                    }
                    if let Some(surface) = state.surfaces.get_mut(index) {
                        surface.gpu = None;
                        surface.images = None;
                        surface.cache = user_interface::Cache::new();
                        surface.events.clear();
                        surface.backdrop = None;
                        surface.configured = false;
                        if let Some(lock_surface) = surface.lock_surface.take() {
                            lock_surface.destroy();
                        }
                        if let Some(fractional) = surface.fractional.take() {
                            fractional.scale.destroy();
                            fractional.viewport.destroy();
                        }
                        surface.surface.destroy();
                    }
                    if state.pointer_surface == Some(index) {
                        state.pointer_surface = None;
                        state.pointer_origin = None;
                    }
                }
                return;
            }
            _ => return,
        };
        match interface.as_str() {
            "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(4), qh, ())),
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(7), qh, ()));
            }
            "ext_session_lock_manager_v1" => {
                state.lock_manager = Some(registry.bind(name, 1, qh, ()));
            }
            "zwlr_screencopy_manager_v1" => {
                state.screencopy = Some(registry.bind(name, version.min(3), qh, ()));
            }
            "wp_cursor_shape_manager_v1" => {
                state.cursor_shape = Some(registry.bind(name, 1, qh, ()));
            }
            "wp_fractional_scale_manager_v1" => {
                state.fractional_scale = Some(registry.bind(name, 1, qh, ()));
            }
            "wp_viewporter" => state.viewporter = Some(registry.bind(name, 1, qh, ())),
            "wl_output" => {
                let index = state.outputs.len();
                let output = registry.bind(name, version.min(4), qh, index);
                state.outputs.push(Output {
                    output,
                    global: name,
                    removed: false,
                    name: String::new(),
                    scale: 1,
                    mode: (0, 0),
                    capture: None,
                });
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                output.mode = (width, height);
            }
            wl_output::Event::Scale { factor } => output.scale = factor.clamp(1, 4),
            wl_output::Event::Name { name } => output.name = name,
            _ => {}
        }
    }
}
impl Dispatch<ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let shm = state.shm.clone();
        let Some(capture) = state
            .outputs
            .get_mut(*index)
            .and_then(|o| o.capture.as_mut())
        else {
            return;
        };
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                if matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888)
                    && capture.format.is_none()
                {
                    capture.format = Some((format, width, height, stride));
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => {
                capture.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                start_copy(capture, frame, shm.as_ref(), qh, *index);
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                capture.done = Some(read_capture(capture));
                if let Some(frame) = capture.frame.take() {
                    frame.destroy();
                }
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                capture.done = Some(None);
                if let Some(frame) = capture.frame.take() {
                    frame.destroy();
                }
                if let Some(buffer) = capture.buffer.take() {
                    buffer.destroy();
                }
                capture.memfd = None;
            }
            _ => {}
        }
        // Version 1 and 2 frames have no buffer_done; they copy right away.
        if frame.version() < 3
            && capture.format.is_some()
            && capture.buffer.is_none()
            && capture.done.is_none()
        {
            start_copy(capture, frame, shm.as_ref(), qh, *index);
        }
    }
}

fn start_copy(
    capture: &mut Capture,
    frame: &ZwlrScreencopyFrameV1,
    shm: Option<&wl_shm::WlShm>,
    qh: &QueueHandle<State>,
    index: usize,
) {
    let (Some((format, width, height, stride)), Some(shm)) = (capture.format, shm) else {
        capture.done = Some(None);
        return;
    };
    let size = stride as usize * height as usize;
    let Ok(fd) = rustix::fs::memfd_create("enzui-lock-capture", rustix::fs::MemfdFlags::CLOEXEC)
    else {
        capture.done = Some(None);
        return;
    };
    if rustix::fs::ftruncate(&fd, size as u64).is_err() {
        capture.done = Some(None);
        return;
    }
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        qh,
        index,
    );
    pool.destroy();
    frame.copy(&buffer);
    capture.memfd = Some(fd);
    capture.buffer = Some(buffer);
}

fn read_capture(capture: &mut Capture) -> Option<background::Picture> {
    let (_, width, height, stride) = capture.format?;
    let fd = capture.memfd.take()?;
    let mut raw = vec![0u8; stride as usize * height as usize];
    let file = std::fs::File::from(fd);
    use std::os::unix::fs::FileExt;
    file.read_exact_at(&mut raw, 0).ok()?;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let row = if capture.y_invert {
            height as usize - 1 - y
        } else {
            y
        };
        let line = &raw[row * stride as usize..][..width as usize * 4];
        // wl_shm 8888 formats are little-endian: bytes are B, G, R, (A|X).
        for px in line.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    if let Some(buffer) = capture.buffer.take() {
        buffer.destroy();
    }
    Some(background::Picture {
        width,
        height,
        rgba,
    })
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => {
                state.locked.get_or_insert_with(Instant::now);
            }
            ext_session_lock_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
}
impl Dispatch<ExtSessionLockSurfaceV1, usize> for State {
    fn event(
        state: &mut Self,
        lock_surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
            && let Some(surface) = state.surfaces.get_mut(*index)
            && surface.lock_surface.as_ref() == Some(lock_surface)
        {
            lock_surface.ack_configure(serial);
            surface.size = (width.max(1), height.max(1));
            surface.configured = true;
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            // Devices come and go (docks, remote sessions); the objects must
            // follow the capabilities or input stops after a replug.
            if caps.contains(wl_seat::Capability::Keyboard) {
                if state.wl_keyboard.is_none() {
                    state.wl_keyboard = Some(seat.get_keyboard(qh, ()));
                }
            } else if let Some(keyboard) = state.wl_keyboard.take() {
                keyboard.release();
                state.held = None;
            }
            if caps.contains(wl_seat::Capability::Pointer) {
                if state.pointer.is_none() {
                    let pointer = seat.get_pointer(qh, ());
                    state.pointer_shape = state
                        .cursor_shape
                        .as_ref()
                        .map(|m| m.get_pointer(&pointer, qh, ()));
                    state.pointer = Some(pointer);
                }
            } else if let Some(pointer) = state.pointer.take() {
                if let Some(shape) = state.pointer_shape.take() {
                    shape.destroy();
                }
                pointer.release();
                state.pointer_serial = None;
                state.pointer_activity = None;
                state.pointer_surface = None;
                state.pointer_origin = None;
            }
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                if state.keyboard.is_none() {
                    state.keyboard = keys::Keyboard::new();
                }
                if let Some(keyboard) = state.keyboard.as_mut() {
                    keyboard.load(fd, size);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keyboard) = state.keyboard.as_mut() {
                    keyboard.modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                let Some(keyboard) = state.keyboard.as_ref() else {
                    return;
                };
                if key_state == wl_keyboard::KeyState::Pressed {
                    state.keys.push(keyboard.key(key));
                    state.input = true;
                    state.held = keyboard.repeats(key).then(|| {
                        (
                            key,
                            Instant::now() + Duration::from_millis(state.repeat.1.max(1) as u64),
                        )
                    });
                } else if state.held.is_some_and(|(held, _)| held == key) {
                    state.held = None;
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => state.repeat = (rate, delay),
            wl_keyboard::Event::Leave { .. } => state.held = None,
            _ => {}
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                state.pointer_surface = state.surfaces.iter().position(|s| s.surface == surface);
                if let Some(shape) = &state.pointer_shape {
                    shape.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                }
                state.pointer_serial = Some(serial);
                state.pointer_activity = Some(Instant::now());
                state.pointer_origin = Some((surface_x, surface_y));
                state.move_pointer(surface_x, surface_y);
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let now = Instant::now();
                if state
                    .pointer_activity
                    .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(2))
                    && let (Some(serial), Some(shape)) =
                        (state.pointer_serial, &state.pointer_shape)
                {
                    // DPMS/idle can discard the compositor's cursor image without a new enter.
                    shape.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                }
                state.pointer_activity = Some(now);
                state.move_pointer(surface_x, surface_y);
                if state
                    .pointer_origin
                    .is_none_or(|(x, y)| (surface_x - x).hypot(surface_y - y) > 12.0)
                {
                    state.pointer_origin = None;
                    state.input = true;
                }
            }
            wl_pointer::Event::Button {
                button: 0x110,
                state: WEnum::Value(button_state),
                ..
            } => {
                state.input = true;
                if let Some(surface) = state
                    .pointer_surface
                    .and_then(|i| state.surfaces.get_mut(i))
                {
                    let left = iced::mouse::Button::Left;
                    surface.events.push(iced::Event::Mouse(
                        if button_state == wl_pointer::ButtonState::Pressed {
                            iced::mouse::Event::ButtonPressed(left)
                        } else {
                            iced::mouse::Event::ButtonReleased(left)
                        },
                    ));
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_serial = None;
                state.pointer_activity = None;
                if let Some(surface) = state
                    .pointer_surface
                    .take()
                    .and_then(|i| state.surfaces.get_mut(i))
                {
                    surface.cursor = iced::mouse::Cursor::Unavailable;
                    surface
                        .events
                        .push(iced::Event::Mouse(iced::mouse::Event::CursorLeft));
                }
            }
            _ => {}
        }
    }
}
impl State {
    fn move_pointer(&mut self, x: f64, y: f64) {
        if let Some(surface) = self.pointer_surface.and_then(|i| self.surfaces.get_mut(i)) {
            let position = iced::Point::new(x as f32, y as f32);
            surface.cursor = iced::mouse::Cursor::Available(position);
            surface
                .events
                .push(iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                    position,
                }));
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(State: ExtSessionLockManagerV1);
wayland_client::delegate_noop!(State: ZwlrScreencopyManagerV1);
wayland_client::delegate_noop!(State: WpCursorShapeManagerV1);
wayland_client::delegate_noop!(State: WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(State: WpViewporter);
wayland_client::delegate_noop!(State: WpViewport);
impl Dispatch<WpFractionalScaleV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event
            && let Some(fractional) = state
                .surfaces
                .get_mut(*index)
                .and_then(|s| s.fractional.as_mut())
        {
            fractional.preferred = Some(scale.clamp(1, 4 * SCALE_DENOMINATOR));
        }
    }
}
wayland_client::delegate_noop!(State: WpCursorShapeDeviceV1);
impl Dispatch<wl_callback::WlCallback, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(surface) = state.surfaces.get_mut(*index)
        {
            surface.frame_pending = None;
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, usize> for State {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if *index > usize::MAX / 2 && matches!(event, wl_buffer::Event::Release) {
            if let Some(surface) = state.surfaces.get_mut(usize::MAX - *index) {
                surface.software_pending = surface.software_pending.saturating_sub(1);
            }
            buffer.destroy();
        }
    }
}
