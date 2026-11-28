use crate::{
    Message, Shell,
    dock_ui::{self, Origin},
};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    dock::{Position, Window},
    search::WindowTarget,
};
use iced::{
    Element, Rectangle, Task,
    widget::{column, container, image, mouse_area, row},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const THUMBNAIL_WIDTH: u32 = 160;
const THUMBNAIL_HEIGHT: u32 = 106;

#[derive(Clone, Debug)]
pub(crate) enum Event {
    Enter(String, Origin),
    Leave(String),
    Dwell(u64),
    Bounds(u64, Origin, Option<Rectangle>),
    Point(u64, Result<iced::Point, String>),
    Refresh(u64),
    Picture(u64, WindowTarget, [u32; 2], Option<image::Handle>),
    Hold(bool),
    Dismiss(u64),
    Focus(WindowTarget),
    Cycle(String, iced::mouse::ScrollDelta),
}
pub(crate) struct Preview {
    pub serial: u64,
    next: usize,
    pub bounds: Rectangle,
    pub windows: Vec<Window>,
    id: String,
    body: Rectangle,
    point: iced::Point,
    position: Position,
    pixels: [u32; 2],
}
#[derive(Default)]
pub(crate) struct State {
    pub preview: Option<Preview>,
    pub hovered: Option<(String, Origin)>,
    pub serial: u64,
    pub held: bool,
    pub busy: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    cycle: Option<(String, String, Instant)>,
    gesture: Gesture,
    pictures: std::collections::VecDeque<Picture>,
}

struct Picture {
    target: WindowTarget,
    handle: Option<image::Handle>,
    pixels: [u32; 2],
    time: Instant,
}

#[derive(Default)]
struct Gesture {
    app: String,
    last: Option<Instant>,
    distance: f32,
    stepped: bool,
}

impl Gesture {
    fn step(&mut self, id: &str, delta: iced::mouse::ScrollDelta, now: Instant) -> Option<f32> {
        use iced::mouse::ScrollDelta::{Lines, Pixels};
        let (x, y) = match delta {
            Lines { x, y } | Pixels { x, y } => (x, y),
        };
        let distance = if y.abs() >= x.abs() { -y } else { -x };
        if !distance.is_finite() || distance == 0.0 {
            return None;
        }
        if matches!(delta, Lines { .. }) {
            return Some(distance);
        }
        if self.app != id
            || self
                .last
                .is_none_or(|last| now - last >= Duration::from_millis(230))
        {
            self.app = id.into();
            self.distance = 0.0;
            self.stepped = false;
        }
        self.last = Some(now);
        self.distance += distance;
        if !self.stepped && self.distance.abs() >= 14.0 {
            self.stepped = true;
            return Some(self.distance);
        }
        None
    }
}
impl State {
    fn enter(&mut self, id: String, origin: Origin) -> bool {
        let needs_dwell = self.preview.as_ref().is_none_or(|preview| preview.id != id);
        if needs_dwell {
            self.clear();
        }
        self.hovered = Some((id, origin));
        needs_dwell
    }

    fn picture(&self, target: &WindowTarget, pixels: [u32; 2]) -> Option<&Picture> {
        self.pictures
            .iter()
            .find(|p| p.target == *target && p.pixels == pixels)
    }

    fn cache(
        &mut self,
        target: WindowTarget,
        pixels: [u32; 2],
        picture: Option<image::Handle>,
    ) -> bool {
        let old = self
            .pictures
            .iter()
            .position(|p| p.target == target)
            .and_then(|index| self.pictures.remove(index));
        let unchanged = old.as_ref().is_some_and(|old| {
            old.pixels == pixels
                && match (&old.handle, &picture) {
                    (None, None) => true,
                    (
                        Some(image::Handle::Rgba {
                            width,
                            height,
                            pixels,
                            ..
                        }),
                        Some(image::Handle::Rgba {
                            width: new_width,
                            height: new_height,
                            pixels: new_pixels,
                            ..
                        }),
                    ) => (width, height, pixels) == (new_width, new_height, new_pixels),
                    _ => false,
                }
        });
        let handle = if unchanged {
            old.and_then(|old| old.handle)
        } else {
            picture
        };
        self.pictures.push_front(Picture {
            target,
            handle,
            pixels,
            time: Instant::now(),
        });
        self.pictures.truncate(24);
        !unchanged
    }

    pub(crate) fn update_windows(&mut self, entries: &[dock_ui::Entry], screen: iced::Size) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        let Some(entry) = entries.iter().find(|entry| entry.group.id == preview.id) else {
            self.clear();
            return;
        };
        let windows = entry
            .group
            .windows
            .iter()
            .take(16)
            .cloned()
            .collect::<Vec<_>>();
        if windows.is_empty() {
            self.clear();
        } else {
            if preview.windows.len() != windows.len() {
                preview.bounds = place(
                    screen,
                    preview.body,
                    preview.point,
                    preview.position,
                    windows.len(),
                );
            }
            preview.windows = windows;
        }
    }

    pub(crate) fn clear_cache(&mut self) {
        self.pictures.clear();
    }

    pub(crate) fn clear(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.preview = None;
        self.hovered = None;
        self.held = false;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn message(event: Event) -> Message {
    Message::Dock(dock_ui::Event::Preview(event))
}
fn delay(serial: u64, dwell: bool) -> Task<Message> {
    Task::perform(
        async move {
            tokio::time::sleep(Duration::from_millis(if dwell { 350 } else { 400 })).await;
            serial
        },
        move |serial| {
            message(if dwell {
                Event::Dwell(serial)
            } else {
                Event::Dismiss(serial)
            })
        },
    )
}
pub(crate) fn place(
    screen: iced::Size,
    body: Rectangle,
    point: iced::Point,
    position: Position,
    count: usize,
) -> Rectangle {
    let inward = match position {
        Position::Left => screen.width - body.x - body.width - 18.0,
        Position::Right => body.x - 18.0,
        _ => screen.width - 24.0,
    };
    let width = (count.clamp(1, 6) as f32 * 176.0 + 12.0).min(inward.max(1.0));
    let height = 148.0_f32.min((screen.height - 24.0).max(1.0));
    let (x, y) = match position {
        Position::Left => (body.x + body.width + 6.0, point.y - height / 2.0),
        Position::Right => (body.x - width - 6.0, point.y - height / 2.0),
        Position::Top => (point.x - width / 2.0, body.y + body.height + 6.0),
        _ => (point.x - width / 2.0, body.y - height - 6.0),
    };
    Rectangle {
        x: x.clamp(12.0, (screen.width - width - 12.0).max(12.0)),
        y: y.clamp(12.0, (screen.height - height - 12.0).max(12.0)),
        width,
        height,
    }
}
pub(super) struct WheelTarget {
    pub id: String,
    pub size: iced::Size,
    pub scroll: enzui_widgets::scroll_view::ScrollGeometryCell,
}

impl iced::advanced::Widget<Message, iced::Theme, iced::Renderer> for WheelTarget {
    fn size(&self) -> iced::Size<iced::Length> {
        iced::Size::new(self.size.width.into(), self.size.height.into())
    }

    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        _: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) {
        let size =
            <Self as iced::advanced::Widget<Message, iced::Theme, iced::Renderer>>::size(self);
        iced::advanced::layout::empty(limits, size.width, size.height).apply(tree);
    }

    fn update(
        &mut self,
        _: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout,
        cursor: iced::mouse::Cursor,
        _: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) = event
            && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport)
            && !self
                .scroll
                .get()
                .is_some_and(|geometry| geometry.is_overflowing())
        {
            shell.publish(message(Event::Cycle(self.id.clone(), *delta)));
            shell.capture_event();
        }
    }

    fn frame_interest(&self, _: &iced::advanced::widget::Tree) -> bool {
        false
    }

    fn draw(
        &self,
        _: &iced::advanced::widget::Tree,
        _: &mut iced::Renderer,
        _: &iced::Theme,
        _: &iced::advanced::renderer::Style,
        _: iced::advanced::Layout,
        _: iced::mouse::Cursor,
        _: &Rectangle,
    ) {
    }
}

pub(super) fn window_context(
    window: &Window,
    snapshot: &enzui_shell::dock::Snapshot,
) -> Option<String> {
    if window.minimized {
        return Some(tr("shell-minimized").to_owned());
    }
    (window.output != snapshot.output_id || window.workspace != snapshot.desktop)
        .then(|| tr!("shell-desktop-number", "arg0" => window.workspace))
}

pub(crate) fn cycle_target(
    windows: &[Window],
    previous: Option<&str>,
    delta: f32,
) -> Option<WindowTarget> {
    if windows.is_empty() || !delta.is_finite() || delta == 0.0 {
        return None;
    }
    let current = previous
        .and_then(|id| windows.iter().position(|w| w.target.stable_id == id))
        .or_else(|| windows.iter().position(|w| w.active));
    let index = current.map_or(0, |i| {
        if delta > 0.0 {
            (i + 1) % windows.len()
        } else {
            (i + windows.len() - 1) % windows.len()
        }
    });
    Some(windows[index].target.clone())
}
impl Shell {
    pub(crate) fn dock_preview_event(&mut self, event: Event) -> Task<Message> {
        if !self.dock_active()
            && matches!(
                &event,
                Event::Enter(..) | Event::Focus(..) | Event::Cycle(..)
            )
        {
            return Task::none();
        }
        if self.dock.fullscreen {
            self.dock.preview.clear();
            return Task::none();
        }
        match event {
            Event::Enter(id, origin) => {
                if !self.dock.preferences.hover_previews
                    || self.dock.menu.is_some()
                    || self
                        .dock
                        .entries
                        .iter()
                        .find(|e| e.group.id == id)
                        .is_none_or(|e| e.group.windows.is_empty())
                {
                    return Task::none();
                }
                if self.dock.preview.enter(id, origin) {
                    delay(self.dock.preview.serial, true)
                } else {
                    Task::none()
                }
            }
            Event::Leave(id) => {
                if self
                    .dock
                    .preview
                    .hovered
                    .as_ref()
                    .is_some_and(|(current, _)| *current == id)
                {
                    self.dock.preview.hovered = None;
                    self.dock.preview.serial = self.dock.preview.serial.wrapping_add(1);
                    delay(self.dock.preview.serial, false)
                } else {
                    Task::none()
                }
            }
            Event::Dwell(serial) if serial == self.dock.preview.serial => {
                let Some((id, origin)) = self.dock.preview.hovered.clone() else {
                    return Task::none();
                };
                let window = if matches!(origin, Origin::Bar) {
                    Some(self.panel)
                } else {
                    self.dock.window
                };
                window.map_or_else(Task::none, |window| {
                    enzui_shell::widget_bounds::bounds(window, format!("dock-app-{id}"))
                        .map(move |bounds| message(Event::Bounds(serial, origin, bounds)))
                })
            }
            Event::Bounds(serial, origin, Some(bounds)) if serial == self.dock.preview.serial => {
                let compositor = self.compositor.clone();
                let class = if matches!(origin, Origin::Bar) {
                    "dev.enzui.Desktop"
                } else {
                    "dev.enzui.Dock"
                };
                let invocation = enzui_core::context_menu::InvocationKind::Keyboard(
                    enzui_core::context_menu::Rect::new(
                        bounds.center_x(),
                        bounds.center_y(),
                        0.0,
                        0.0,
                    ),
                );
                Task::perform(
                    async move { compositor.dock_menu_point(class, invocation).await },
                    move |point| message(Event::Point(serial, point)),
                )
            }
            Event::Point(serial, Ok(point)) if serial == self.dock.preview.serial => {
                let Some((id, _)) = self.dock.preview.hovered.clone() else {
                    return Task::none();
                };
                let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
                    return Task::none();
                };
                let Some(g) = self.dock.geometry.as_ref() else {
                    return Task::none();
                };
                let windows = entry
                    .group
                    .windows
                    .iter()
                    .take(16)
                    .cloned()
                    .collect::<Vec<_>>();
                let mut body = g.body;
                let position = if self.dock.preferences.position == Position::Bar {
                    if self.bar == enzui_shell::settings::BarEdge::Top {
                        body.y = point.y;
                        body.height = f32::from(self.metrics.bar) / 2.0;
                        Position::Top
                    } else {
                        body.y = point.y - f32::from(self.metrics.bar) / 2.0;
                        Position::Bottom
                    }
                } else {
                    self.dock.preferences.position
                };
                self.dock.preview.preview = Some(Preview {
                    serial,
                    next: 0,
                    bounds: place(g.screen, body, point, position, windows.len()),
                    windows,
                    id,
                    body,
                    point,
                    position,
                    pixels: [THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT].map(|size| {
                        (size as f32 * self.dock.snapshot.as_ref().map_or(1.0, |s| s.scale))
                            .ceil()
                            .clamp(1.0, 640.0) as u32
                    }),
                });
                self.dock.hidden = false;
                self.dock_capture_previews(serial)
            }
            Event::Refresh(serial) => {
                self.quiet = true;
                self.dock_capture_previews(serial)
            }
            Event::Picture(serial, target, pixels, picture) => {
                self.dock.preview.busy = false;
                let current = self.dock.preview.preview.as_ref().map(|p| p.serial);
                if current != Some(serial) {
                    self.quiet = true;
                    return current
                        .map_or_else(Task::none, |serial| self.dock_capture_previews(serial));
                }
                if self.dock.preview.cache(target, pixels, picture) {
                    self.redraw_only = self.dock.window;
                } else {
                    self.quiet = true;
                }
                let initializing =
                    self.dock.preview.preview.as_ref().is_some_and(|p| {
                        p.next < p.windows.iter().filter(|w| !w.minimized).count()
                    });
                let interval = Duration::from_millis(if initializing { 80 } else { 320 });
                Task::perform(
                    async move { tokio::time::sleep(interval).await },
                    move |()| message(Event::Refresh(serial)),
                )
            }
            Event::Hold(held) => {
                self.dock.preview.held = held;
                if held {
                    Task::none()
                } else {
                    delay(self.dock.preview.serial, false)
                }
            }
            Event::Dismiss(serial)
                if serial == self.dock.preview.serial
                    && self.dock.preview.hovered.is_none()
                    && !self.dock.preview.held =>
            {
                self.dock.preview.clear();
                self.dock_rebuild();
                Task::none()
            }
            Event::Focus(target) => {
                self.dock.preview.clear();
                self.dock_rebuild();
                self.dock_windows(vec![target], "focus")
            }
            Event::Cycle(id, delta) => {
                let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
                    return Task::none();
                };
                let now = Instant::now();
                if entry.group.windows.len() < 2 {
                    return Task::none();
                }
                let Some(delta) = self.dock.preview.gesture.step(&id, delta, now) else {
                    self.quiet = true;
                    return Task::none();
                };
                let previous = self
                    .dock
                    .preview
                    .cycle
                    .as_ref()
                    .filter(|(app, _, at)| *app == id && at.elapsed() < Duration::from_secs(2))
                    .map(|(_, id, _)| id.as_str());
                let Some(target) = cycle_target(&entry.group.windows, previous, delta) else {
                    return Task::none();
                };
                self.dock.preview.cycle = Some((id, target.stable_id.clone(), now));
                self.dock.preview.clear();
                self.dock_rebuild();
                self.dock_windows(vec![target], "focus")
            }
            _ => Task::none(),
        }
    }
    fn dock_capture_previews(&mut self, serial: u64) -> Task<Message> {
        if self.dock.preview.busy {
            return Task::none();
        }
        let Some(preview) = self
            .dock
            .preview
            .preview
            .as_mut()
            .filter(|p| p.serial == serial)
        else {
            return Task::none();
        };
        let visible = preview
            .windows
            .iter()
            .filter(|w| !w.minimized)
            .collect::<Vec<_>>();
        if visible.is_empty() {
            return Task::none();
        }
        let target = visible[preview.next % visible.len()].target.clone();
        let pixels = preview.pixels;
        let first = preview.next < visible.len();
        preview.next = preview.next.wrapping_add(1);
        if first
            && self
                .dock
                .preview
                .picture(&target, pixels)
                .is_some_and(|p| p.time.elapsed() < Duration::from_millis(320))
        {
            return Task::done(message(Event::Refresh(serial)));
        }
        let root = self.root.clone();
        let compositor = self.compositor.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.dock.preview.cancel = Some(Arc::clone(&cancel));
        self.dock.preview.busy = true;
        Task::perform(
            async move {
                let mut picture = None;
                if compositor.dock_capture_target(&target).await && !cancel.load(Ordering::Relaxed)
                {
                    picture = tokio::time::timeout(
                        Duration::from_millis(650),
                        enzui_shell::previews::capture_window(
                            root,
                            target.address.clone(),
                            pixels[0],
                            pixels[1],
                        ),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok);
                }
                if picture.is_some()
                    && (cancel.load(Ordering::Relaxed)
                        || !compositor.dock_capture_target(&target).await)
                {
                    picture = None;
                }
                (target, picture)
            },
            move |(target, picture)| message(Event::Picture(serial, target, pixels, picture)),
        )
    }
    pub(crate) fn dock_preview_view(
        &self,
        radius: f32,
        glass: bool,
    ) -> Option<Element<'_, Message>> {
        const FRAME_INSET: f32 = 6.0;
        let preview = self.dock.preview.preview.as_ref()?;
        let palette = self.window_tokens(self.dock.window).palette;
        let snapshot = self.dock.snapshot.as_ref()?;
        let cards = preview
            .windows
            .iter()
            .map(|w| {
                let cached = self.dock.preview.picture(&w.target, preview.pixels);
                let picture = cached
                    .and_then(|p| p.handle.as_ref())
                    .filter(|_| !w.minimized);
                let content: Element<'_, Message> = if let Some(picture) = picture {
                    container(
                        image(picture.clone())
                            .width(THUMBNAIL_WIDTH as f32)
                            .height(THUMBNAIL_HEIGHT as f32)
                            .content_fit(iced::ContentFit::Contain),
                    )
                    .center_x(164)
                    .center_y(106)
                    .into()
                } else {
                    container(
                        enzui_widgets::text::enz_text(if w.minimized {
                            tr("shell-minimized")
                        } else if cached.is_none() {
                            tr("shell-loading-preview")
                        } else {
                            tr("shell-preview-unavailable")
                        })
                        .size(enzui_widgets::text::TextSize::Caption)
                        .into_element(),
                    )
                    .center_x(164)
                    .center_y(106)
                    .into()
                };
                let active = w.active;
                let context = window_context(w, snapshot);
                let mut caption = row![
                    enzui_widgets::text::enz_text(&w.title)
                        .width(iced::Length::Fill)
                        .no_wrap()
                        .size(enzui_widgets::text::TextSize::Caption)
                        .into_element()
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center);
                if let Some(context) = &context {
                    caption = caption.push(
                        enzui_widgets::text::enz_text(context.clone())
                            .no_wrap()
                            .size(enzui_widgets::text::TextSize::Caption)
                            .color(palette.text_muted)
                            .into_element(),
                    );
                }
                let title = context.map_or_else(
                    || w.title.clone(),
                    |context| format!("{} · {context}", w.title),
                );
                enzui_widgets::pressable::enz_pressable(
                    column![content, caption].width(164).spacing(4).into(),
                    message(Event::Focus(w.target.clone())),
                )
                .style(enzui_widgets::pressable::PressableStyle::Plain)
                .padding(4)
                .radius((radius - FRAME_INSET).max(0.0))
                .surface_colors_resolved(move |_| enzui_widgets::pressable::SurfaceColors {
                    background: active.then_some(palette.surface_selected),
                    hover_background: Some(palette.surface_hover),
                    border: None,
                })
                .aria_label(if w.minimized {
                    tr!("shell-show-minimized-window", "title" => w.title.clone())
                } else {
                    tr!("shell-show-2", "arg0" => title)
                })
                .into_element()
            })
            .collect::<Vec<_>>();
        let bounds = preview.bounds;
        let content = enzui_widgets::scroll_view::enz_scroll_view(row(cards).spacing(4))
            .axis(enzui_widgets::scroll_view::Axis::Horizontal)
            .width(bounds.width - 2.0 * FRAME_INSET)
            .height(bounds.height - 2.0 * FRAME_INSET)
            .into_element();
        let body = container(content)
            .padding(FRAME_INSET)
            .width(bounds.width)
            .height(bounds.height)
            .style(move |_| container::Style {
                background: (!glass).then_some(palette.surface.into()),
                border: iced::Border {
                    radius: radius.into(),
                    width: 1.0,
                    color: if glass {
                        enzui_shell::reveal::glass_rim(palette.surface)
                    } else {
                        palette.border
                    },
                },
                ..Default::default()
            });
        Some(
            container(
                mouse_area(body)
                    .on_enter(message(Event::Hold(true)))
                    .on_exit(message(Event::Hold(false))),
            )
            .padding(iced::Padding::ZERO.left(bounds.x).top(bounds.y))
            .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::mouse::ScrollDelta;

    #[test]
    fn returning_to_the_previewed_icon_keeps_the_card_open() {
        let mut state = State {
            serial: 2,
            preview: Some(Preview {
                serial: 1,
                next: 0,
                bounds: Rectangle::default(),
                windows: Vec::new(),
                id: "terminal".into(),
                body: Rectangle::default(),
                point: iced::Point::ORIGIN,
                position: Position::Bottom,
                pixels: [160, 106],
            }),
            ..State::default()
        };

        assert!(!state.enter("terminal".into(), Origin::Dock));
        assert!(state.preview.is_some());
        assert!(state.hovered.is_some());
        assert!(state.enter("editor".into(), Origin::Dock));
        assert!(state.preview.is_none());
    }

    #[test]
    fn one_smooth_swipe_advances_once_including_momentum() {
        let mut gesture = Gesture::default();
        let start = Instant::now();
        let pixels = |y| ScrollDelta::Pixels { x: 0.0, y };
        assert!(gesture.step("app", pixels(-5.0), start).is_none());
        assert!(
            gesture
                .step("app", pixels(-10.0), start + Duration::from_millis(20))
                .unwrap()
                > 0.0
        );
        for after in [70, 130, 200, 270] {
            assert!(
                gesture
                    .step("app", pixels(-30.0), start + Duration::from_millis(after))
                    .is_none()
            );
        }
        assert!(
            gesture
                .step("app", pixels(30.0), start + Duration::from_millis(530))
                .unwrap()
                < 0.0
        );
        let wheel = ScrollDelta::Lines { x: 0.0, y: -1.0 };
        assert!(gesture.step("app", wheel, start).unwrap() > 0.0);
        assert!(gesture.step("app", wheel, start).unwrap() > 0.0);
    }
}
