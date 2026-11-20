use super::{Message, Shell, window_settings};
use enzui_shell::capture::{Rect, Source};
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::i18n::tr;
use iced::widget::image::Handle;
use iced::{Point, Task, window};

mod view;
use std::time::Duration;

// A press this close to an edge (or corner) of the selected area resizes it.
const GRIP: f32 = 12.0;

// Capture selection over a frozen desktop; direct screen/window shortcuts bypass selection.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    window: Option<window::Id>,
    placing: bool,
    frame: Option<Frame>,
    record: bool,
    kind: Kind,
    pointer: bool,
    // Kept between openings, as the last area is usually wanted again.
    area: Option<Rect>,
    drag: Option<Drag>,
    cursor: Point,
    windows: Vec<Target>,
    selected: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    Area,
    Screen,
    Window,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    New(Point, Option<Rect>),
    Resize(Rect, Edges),
    Move(f32, f32),
}

// The edges of the area under the pointer: left, right, top, bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Edges(bool, bool, bool, bool);

impl Edges {
    fn any(self) -> bool {
        self.0 || self.1 || self.2 || self.3
    }
}

// Which edges of `area` a press at `point` takes hold of; near a corner two.
fn edges(area: Rect, point: Point) -> Edges {
    let near = |a: f32, b: f32| (a - b).abs() <= GRIP;
    let across = point.y >= area.y - GRIP && point.y <= area.y + area.height + GRIP;
    let along = point.x >= area.x - GRIP && point.x <= area.x + area.width + GRIP;
    let left = across && near(point.x, area.x);
    let right = across && !left && near(point.x, area.x + area.width);
    let top = along && near(point.y, area.y);
    let bottom = along && !top && near(point.y, area.y + area.height);
    Edges(left, right, top, bottom)
}

#[derive(Clone)]
pub(crate) struct Frame {
    output: String,
    origin: (i32, i32),
    size: (f32, f32),
    plain: Handle,
    with_pointer: Option<Handle>,
    allocations: Vec<iced::widget::image::Allocation>,
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("output", &self.output)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

// A window that can be chosen, in the output's logical pixels.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    id: String,
    title: String,
    rect: Rect,
    // Fullscreen windows are drawn without rounded corners.
    rounded: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum CaptureMessage {
    Screen,
    ActiveWindow,
    Frozen(Result<(Frame, Vec<Target>), String>),
    Allocated(
        Handle,
        Vec<Result<iced::widget::image::Allocation, iced::widget::image::Error>>,
    ),
    Placed(Result<ControlGeometry, String>),
    Record(bool),
    Kind(Kind),
    Pointer(bool),
    Moved(Point),
    Pressed,
    Released,
    Take,
    Close,
    Settings,
    StartRecording(enzui_shell::recording::Target),
    Saved(Result<enzui_shell::capture::Saved, String>),
    Ignore,
}

impl Capture {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }

    fn selection(&self) -> Option<Rect> {
        let frame = self.frame.as_ref()?;
        match self.kind {
            Kind::Area => self.area,
            Kind::Screen => Some(Rect {
                x: 0.0,
                y: 0.0,
                width: frame.size.0,
                height: frame.size.1,
            }),
            Kind::Window => self
                .selected
                .and_then(|index| self.windows.get(index))
                .map(|target| target.rect),
        }
    }

    fn window_at(&self, point: Point) -> Option<usize> {
        self.windows
            .iter()
            .position(|target| target.rect.contains((point.x, point.y)))
    }

    // The pointer's shape: what pressing here would do.
    fn interaction(&self) -> iced::mouse::Interaction {
        use iced::mouse::Interaction;
        match self.kind {
            Kind::Screen => return Interaction::Idle,
            Kind::Window => return Interaction::Pointer,
            Kind::Area => {}
        }
        let held = match self.drag {
            Some(Drag::Move(..)) => return Interaction::Grabbing,
            Some(Drag::New(..)) => return Interaction::Crosshair,
            Some(Drag::Resize(_, held)) => held,
            None => self
                .area
                .map(|area| edges(area, self.cursor))
                .unwrap_or_default(),
        };
        match held {
            Edges(true, _, true, _) | Edges(_, true, _, true) => {
                Interaction::ResizingDiagonallyDown
            }
            Edges(true, _, _, true) | Edges(_, true, true, _) => Interaction::ResizingDiagonallyUp,
            Edges(true, ..) | Edges(_, true, ..) => Interaction::ResizingHorizontally,
            Edges(_, _, true, _) | Edges(_, _, _, true) => Interaction::ResizingVertically,
            _ if self
                .area
                .is_some_and(|area| area.contains((self.cursor.x, self.cursor.y))) =>
            {
                Interaction::Grab
            }
            _ => Interaction::Crosshair,
        }
    }
}

// A window's own picture, its corners transparent as they are rounded on
// screen.
async fn window_picture(
    controls: &enzui_shell::system_controls::SystemControls,
    compositor: &enzui_shell::compositor::Compositor,
    target: Target,
    pointer: bool,
) -> Result<raster::DynamicImage, String> {
    let picture = enzui_shell::capture::grab(controls, &Source::Window(target.id), pointer).await?;
    if !target.rounded {
        return Ok(picture);
    }
    let (radius, power) = compositor.window_rounding().await;
    let scale = picture.width() as f32 / target.rect.width.max(1.0);
    let mut rgba = picture.into_rgba8();
    enzui_shell::capture::round_corners(&mut rgba, radius * scale, power);
    Ok(raster::DynamicImage::ImageRgba8(rgba))
}

fn pixels(handle: &Handle) -> Option<raster::DynamicImage> {
    let Handle::Rgba {
        width,
        height,
        pixels,
        ..
    } = handle
    else {
        return None;
    };
    raster::RgbaImage::from_raw(*width, *height, pixels.to_vec())
        .map(raster::DynamicImage::ImageRgba8)
}

fn handle(image: raster::DynamicImage) -> Handle {
    let rgba = image.into_rgba8();
    Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw())
}

// The windows on `output`'s visible desktop, the most recently used first.
fn targets(output: &serde_json::Value, clients: &[serde_json::Value]) -> Vec<Target> {
    let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0) as f32;
    let workspace = output["activeWorkspace"]["id"].as_i64();
    let (origin_x, origin_y) = (number(&output["x"]), number(&output["y"]));
    let mut found: Vec<(i64, Target)> = clients
        .iter()
        .filter(|client| client["mapped"] != false && client["hidden"] != true)
        .filter(|client| client["monitor"] == output["id"])
        .filter(|client| {
            client["pinned"] == true || client["workspace"]["id"].as_i64() == workspace
        })
        // The shell's own surfaces are not windows to capture.
        .filter(|client| client["pid"].as_u64() != Some(u64::from(std::process::id())))
        .filter_map(|client| {
            let id = match &client["stableId"] {
                serde_json::Value::String(id) => id.clone(),
                serde_json::Value::Number(id) => id.to_string(),
                _ => return None,
            };
            let rect = Rect {
                x: number(&client["at"][0]) - origin_x,
                y: number(&client["at"][1]) - origin_y,
                width: number(&client["size"][0]).max(1.0),
                height: number(&client["size"][1]).max(1.0),
            };
            let title = client["title"]
                .as_str()
                .or_else(|| client["class"].as_str())
                .unwrap_or_else(|| tr("shell-window"))
                .chars()
                .take(120)
                .collect();
            let rounded =
                client["fullscreen"].as_i64().unwrap_or(0) == 0 && client["fullscreen"] != true;
            Some((
                client["focusHistoryID"].as_i64().unwrap_or(i64::MAX),
                Target {
                    id,
                    title,
                    rect,
                    rounded,
                },
            ))
        })
        .collect();
    found.sort_by_key(|(order, _)| *order);
    found
        .into_iter()
        .map(|(_, target)| target)
        .take(64)
        .collect()
}

impl Shell {
    fn wanted_pointer(&self) -> bool {
        self.desired_settings.capture.show_pointer
    }

    pub(crate) fn open_capture(&mut self) -> Task<Message> {
        if self.capture.window.is_some() || self.capture.placing {
            return self.close_capture();
        }
        // While recording, the same key stops it.
        if self.recording.is_some() {
            return self.stop_recording();
        }
        self.capture.placing = true;
        // An open toast leaves first, and the screen is frozen once it has gone.
        let (had_toast, hide) = self.pause_toasts(true);
        let settle = std::time::Duration::from_millis(if had_toast { 180 } else { 0 });
        let (compositor, controls) = (self.compositor.clone(), self.controls.clone());
        let grab = Task::perform(
            async move {
                tokio::time::sleep(settle).await;
                let output = compositor.desktop_output().await?;
                let name = output["name"]
                    .as_str()
                    .ok_or_else(|| tr("shell-missing-output"))?
                    .to_owned();
                let scale = output["scale"]
                    .as_f64()
                    .filter(|scale| *scale > 0.0)
                    .ok_or_else(|| tr("shell-missing-output-scale"))?;
                let size = (
                    (output["width"].as_f64().unwrap_or(0.0) / scale) as f32,
                    (output["height"].as_f64().unwrap_or(0.0) / scale) as f32,
                );
                let origin = (
                    output["x"].as_i64().unwrap_or(0) as i32,
                    output["y"].as_i64().unwrap_or(0) as i32,
                );
                let source = Source::Output(name.clone());
                let (plain, with_pointer, clients) = tokio::join!(
                    enzui_shell::capture::grab(&controls, &source, false),
                    enzui_shell::capture::grab(&controls, &source, true),
                    compositor.windows()
                );
                let frame = Frame {
                    output: name,
                    origin,
                    size,
                    plain: handle(plain?),
                    with_pointer: with_pointer.ok().map(handle),
                    allocations: Vec::new(),
                };
                Ok((frame, targets(&output, &clients.unwrap_or_default())))
            },
            |result| Message::Capture(CaptureMessage::Frozen(result)),
        );
        Task::batch([hide, grab])
    }

    pub(crate) fn close_capture(&mut self) -> Task<Message> {
        self.capture.placing = false;
        self.capture.drag = None;
        self.capture.frame = None;
        let (_, toasts) = self.pause_toasts(false);
        let close = self
            .capture
            .window
            .take()
            .map_or_else(Task::none, window::close);
        Task::batch([close, toasts])
    }

    pub(crate) fn capture_window(&mut self, event: &window::Event) -> Task<Message> {
        match event {
            window::Event::Unfocused | window::Event::CloseRequested => self.close_capture(),
            window::Event::Closed => {
                self.capture.window = None;
                if self.capture.placing {
                    Task::none()
                } else {
                    self.pause_toasts(false).1
                }
            }
            _ => Task::none(),
        }
    }

    // Enter takes the capture; the arrows move the area or choose the
    // next window, so it all works from the keyboard.
    pub(crate) fn capture_key(&mut self, key: &iced::keyboard::Key) -> Task<Message> {
        use iced::keyboard::{Key, key::Named};
        let (dx, dy) = match key {
            Key::Named(Named::Enter) => return self.capture_message(CaptureMessage::Take),
            Key::Named(Named::ArrowLeft) => (-10.0, 0.0),
            Key::Named(Named::ArrowRight) => (10.0, 0.0),
            Key::Named(Named::ArrowUp) => (0.0, -10.0),
            Key::Named(Named::ArrowDown) => (0.0, 10.0),
            _ => return Task::none(),
        };
        let Some(size) = self.capture.frame.as_ref().map(|frame| frame.size) else {
            return Task::none();
        };
        match self.capture.kind {
            Kind::Area => {
                if let Some(area) = self.capture.area.as_mut() {
                    let moved = Rect {
                        x: area.x + dx,
                        y: area.y + dy,
                        ..*area
                    };
                    *area = Rect {
                        x: moved.x.clamp(0.0, (size.0 - moved.width).max(0.0)),
                        y: moved.y.clamp(0.0, (size.1 - moved.height).max(0.0)),
                        ..moved
                    };
                }
            }
            Kind::Window if !self.capture.windows.is_empty() => {
                let count = self.capture.windows.len();
                let current = self.capture.selected.unwrap_or(0);
                self.capture.selected = Some(if dx + dy > 0.0 {
                    (current + 1) % count
                } else {
                    (current + count - 1) % count
                });
            }
            _ => {}
        }
        Task::none()
    }

    pub(crate) fn capture_message(&mut self, message: CaptureMessage) -> Task<Message> {
        match message {
            CaptureMessage::Screen => {
                let (compositor, controls, pointer) = (
                    self.compositor.clone(),
                    self.controls.clone(),
                    self.wanted_pointer(),
                );
                let preferences = self.desired_settings.capture.clone();
                Task::perform(
                    async move {
                        let output = compositor.desktop_output().await?;
                        let name = output["name"]
                            .as_str()
                            .ok_or_else(|| tr("shell-missing-output"))?
                            .to_owned();
                        let image =
                            enzui_shell::capture::grab(&controls, &Source::Output(name), pointer)
                                .await?;
                        keep(image, preferences, &controls).await
                    },
                    |result| Message::Capture(CaptureMessage::Saved(result)),
                )
            }
            CaptureMessage::ActiveWindow => {
                let (compositor, controls, pointer) = (
                    self.compositor.clone(),
                    self.controls.clone(),
                    self.wanted_pointer(),
                );
                let preferences = self.desired_settings.capture.clone();
                Task::perform(
                    async move {
                        let output = compositor.desktop_output().await?;
                        let clients = compositor.windows().await?;
                        let target = targets(&output, &clients)
                            .into_iter()
                            .next()
                            .ok_or_else(|| tr("shell-there-is-no-window-to-capture"))?;
                        let image = window_picture(&controls, &compositor, target, pointer).await?;
                        keep(image, preferences, &controls).await
                    },
                    |result| Message::Capture(CaptureMessage::Saved(result)),
                )
            }
            CaptureMessage::Frozen(result) => {
                if !self.capture.placing || self.capture.window.is_some() {
                    return Task::none();
                }
                let (frame, windows) = match result {
                    Ok(found) => found,
                    Err(error) => {
                        self.capture.placing = false;
                        self.notice = Some(error);
                        return self.pause_toasts(false).1;
                    }
                };
                let (width, height) = frame.size;
                // A remembered area from another screen size starts over.
                if self
                    .capture
                    .area
                    .is_none_or(|area| area.x + area.width > width || area.y + area.height > height)
                {
                    self.capture.area = Some(Rect {
                        x: (width / 4.0).round(),
                        y: (height / 4.0).round(),
                        width: (width / 2.0).round(),
                        height: (height / 2.0).round(),
                    });
                }
                self.capture.pointer = self.wanted_pointer() && frame.with_pointer.is_some();
                let plain = frame.plain.clone();
                let uploads = std::iter::once(plain.clone())
                    .chain(frame.with_pointer.clone())
                    .map(iced::widget::image::allocate);
                let allocate = Task::batch(uploads).collect();
                self.capture.frame = Some(frame);
                self.capture.windows = windows;
                self.capture.selected = (!self.capture.windows.is_empty()).then_some(0);
                self.capture.drag = None;
                // The renderer loads images asynchronously. Map the full-screen
                // selector only after its frozen desktop can draw on frame one.
                allocate.map(move |result| {
                    Message::Capture(CaptureMessage::Allocated(plain.clone(), result))
                })
            }
            CaptureMessage::Allocated(plain, result) => {
                if !self.capture.placing || self.capture.window.is_some() {
                    return Task::none();
                }
                let Some(frame) = self
                    .capture
                    .frame
                    .as_mut()
                    .filter(|frame| frame.plain == plain)
                else {
                    return Task::none();
                };
                let Ok(allocations) = result.into_iter().collect::<Result<Vec<_>, _>>() else {
                    self.capture.placing = false;
                    self.capture.frame = None;
                    self.notice = Some(tr("shell-the-screen-could-not-be-captured").into());
                    return self.pause_toasts(false).1;
                };
                frame.allocations = allocations;
                let compositor = self.compositor.clone();
                let (metrics, bar) = (self.metrics, self.bar);
                Task::perform(
                    async move {
                        compositor
                            .prepare_overlay("dev.enzui.Capture", 1.0, 1.0, true, metrics, bar)
                            .await
                    },
                    |result| Message::Capture(CaptureMessage::Placed(result)),
                )
            }
            CaptureMessage::Placed(result) => {
                if !std::mem::replace(&mut self.capture.placing, false)
                    || self.capture.window.is_some()
                {
                    return Task::none();
                }
                match result {
                    Ok(geometry) => {
                        let (id, open) = window::open(window_settings(
                            "dev.enzui.Capture",
                            geometry.width,
                            geometry.height,
                        ));
                        self.capture.window = Some(id);
                        open.map(Message::Opened)
                    }
                    Err(error) => {
                        self.capture.frame = None;
                        self.notice = Some(error);
                        self.pause_toasts(false).1
                    }
                }
            }
            CaptureMessage::Record(record) => {
                self.capture.record = record;
                Task::none()
            }
            CaptureMessage::Kind(kind) => {
                self.capture.kind = kind;
                Task::none()
            }
            CaptureMessage::Pointer(on) => {
                self.capture.pointer = on
                    && self
                        .capture
                        .frame
                        .as_ref()
                        .is_some_and(|frame| frame.with_pointer.is_some());
                Task::none()
            }
            CaptureMessage::Moved(point) => {
                self.capture.cursor = point;
                let size = self
                    .capture
                    .frame
                    .as_ref()
                    .map_or((1.0, 1.0), |frame| frame.size);
                let clamp = |point: Point| (point.x.clamp(0.0, size.0), point.y.clamp(0.0, size.1));
                match (self.capture.kind, self.capture.drag) {
                    // The window under the pointer is the one chosen.
                    (Kind::Window, _) => {
                        if let Some(index) = self.capture.window_at(point) {
                            self.capture.selected = Some(index);
                        }
                    }
                    (Kind::Area, Some(Drag::New(from, _))) => {
                        self.capture.area = Some(Rect::spanning(clamp(from), clamp(point)));
                    }
                    (Kind::Area, Some(Drag::Resize(from, Edges(left, right, top, bottom)))) => {
                        let (x0, x1) = (
                            if left { point.x } else { from.x },
                            if right { point.x } else { from.x + from.width },
                        );
                        let (y0, y1) = (
                            if top { point.y } else { from.y },
                            if bottom {
                                point.y
                            } else {
                                from.y + from.height
                            },
                        );
                        self.capture.area = Some(Rect::spanning(
                            clamp(Point::new(x0, y0)),
                            clamp(Point::new(x1, y1)),
                        ));
                    }
                    (Kind::Area, Some(Drag::Move(dx, dy))) => {
                        if let Some(area) = self.capture.area.as_mut() {
                            area.x = (point.x - dx).clamp(0.0, (size.0 - area.width).max(0.0));
                            area.y = (point.y - dy).clamp(0.0, (size.1 - area.height).max(0.0));
                        }
                    }
                    _ => {}
                }
                Task::none()
            }
            CaptureMessage::Pressed => {
                let point = self.capture.cursor;
                match self.capture.kind {
                    Kind::Window => {
                        if let Some(index) = self.capture.window_at(point) {
                            self.capture.selected = Some(index);
                        }
                    }
                    Kind::Area => {
                        self.capture.drag = Some(match self.capture.area {
                            Some(area) => match edges(area, point) {
                                held if held.any() => Drag::Resize(area, held),
                                _ if area.contains((point.x, point.y)) => {
                                    Drag::Move(point.x - area.x, point.y - area.y)
                                }
                                _ => Drag::New(point, Some(area)),
                            },
                            None => Drag::New(point, None),
                        });
                    }
                    Kind::Screen => {}
                }
                Task::none()
            }
            CaptureMessage::Released => {
                // A click without a drag keeps the area there was.
                if let Some(Drag::New(from, before)) = self.capture.drag.take()
                    && (from.x - self.capture.cursor.x).abs() < 4.0
                    && (from.y - self.capture.cursor.y).abs() < 4.0
                    && before.is_some()
                {
                    self.capture.area = before;
                }
                Task::none()
            }
            CaptureMessage::Take => self.take_capture(),
            CaptureMessage::Close => self.close_capture(),
            CaptureMessage::Settings => {
                let close = self.close_capture();
                close.chain(Task::done(Message::SettingsPage("capture")))
            }
            CaptureMessage::StartRecording(target) => self.start_recording(target),
            CaptureMessage::Saved(result) => {
                self.notice = Some(match result {
                    Ok(saved) => saved.notice(),
                    Err(error) => error,
                });
                Task::none()
            }
            CaptureMessage::Ignore => Task::none(),
        }
    }

    fn take_capture(&mut self) -> Task<Message> {
        let Some(frame) = self.capture.frame.clone() else {
            return Task::none();
        };
        let Some(selection) = self.capture.selection() else {
            self.notice = Some(tr("shell-choose-a-window-first").into());
            return Task::none();
        };
        let window = (self.capture.kind == Kind::Window)
            .then(|| {
                self.capture
                    .selected
                    .and_then(|index| self.capture.windows.get(index).cloned())
            })
            .flatten();
        let close = self.close_capture();
        if self.capture.record {
            let target = match self.capture.kind {
                Kind::Screen => enzui_shell::recording::Target::Output(frame.output),
                _ => enzui_shell::recording::Target::Area(
                    selection
                        .clamped(frame.size.0, frame.size.1)
                        .geometry(frame.origin),
                ),
            };
            // Once the interface is off the screen.
            let start = Task::perform(
                async { tokio::time::sleep(Duration::from_millis(300)).await },
                move |()| Message::Capture(CaptureMessage::StartRecording(target)),
            );
            return close.chain(start);
        }
        let (controls, compositor, preferences, pointer) = (
            self.controls.clone(),
            self.compositor.clone(),
            self.desired_settings.capture.clone(),
            self.capture.pointer,
        );
        let image = if pointer {
            frame
                .with_pointer
                .clone()
                .unwrap_or_else(|| frame.plain.clone())
        } else {
            frame.plain.clone()
        };
        let kind = self.capture.kind;
        let shot = Task::perform(
            async move {
                let picture = match (kind, window) {
                    // The window's own content, even where it was covered.
                    (Kind::Window, Some(target)) => {
                        window_picture(&controls, &compositor, target, pointer).await?
                    }
                    (Kind::Screen, _) => tokio::task::spawn_blocking(move || pixels(&image))
                        .await
                        .ok()
                        .flatten()
                        .ok_or_else(|| tr("shell-the-capture-could-not-be-read"))?,
                    _ => tokio::task::spawn_blocking(move || {
                        pixels(&image).map(|picture| {
                            enzui_shell::capture::crop(&picture, selection, frame.size.0)
                        })
                    })
                    .await
                    .ok()
                    .flatten()
                    .ok_or_else(|| tr("shell-the-capture-could-not-be-read"))?,
                };
                keep(picture, preferences, &controls).await
            },
            |result| Message::Capture(CaptureMessage::Saved(result)),
        );
        close.chain(shot)
    }
}

async fn keep(
    image: raster::DynamicImage,
    preferences: enzui_shell::capture::Preferences,
    controls: &enzui_shell::system_controls::SystemControls,
) -> Result<enzui_shell::capture::Saved, String> {
    let tool = enzui_shell::capture::clipboard_tool(controls);
    tokio::task::spawn_blocking(move || {
        enzui_shell::capture::keep(&image, &preferences, tool.as_deref())
    })
    .await
    .map_err(|_| tr("shell-the-screenshot-could-not-be-saved"))?
}
