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
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) enum Event {
    Enter(String, Origin),
    Leave(String),
    Dwell(u64),
    Bounds(u64, Origin, Option<Rectangle>),
    Point(u64, Result<iced::Point, String>),
    Pictures(u64, Vec<(WindowTarget, Option<image::Handle>)>),
    Hold(bool),
    Dismiss(u64),
    Focus(WindowTarget),
    Cycle(String, f32),
}
pub(crate) struct Preview {
    pub serial: u64,
    pub loaded: bool,
    pub bounds: Rectangle,
    pub windows: Vec<Window>,
    pub pictures: Vec<(WindowTarget, Option<image::Handle>)>,
}
#[derive(Default)]
pub(crate) struct State {
    pub preview: Option<Preview>,
    pub hovered: Option<(String, Origin)>,
    pub serial: u64,
    pub held: bool,
    pub busy: bool,
    pub cancel: Option<Arc<AtomicBool>>,
    pub cycle: Option<(String, String, std::time::Instant)>,
}
impl State {
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
    let width = (count.clamp(1, 6) as f32 * 176.0 + 12.0).min((screen.width - 24.0).max(1.0));
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
                if self.dock.menu.is_some()
                    || self
                        .dock
                        .entries
                        .iter()
                        .find(|e| e.group.id == id)
                        .is_none_or(|e| e.group.windows.is_empty())
                {
                    return Task::none();
                }
                self.dock.preview.clear();
                self.dock.preview.hovered = Some((id, origin));
                delay(self.dock.preview.serial, true)
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
                    loaded: false,
                    bounds: place(g.screen, body, point, position, windows.len()),
                    windows,
                    pictures: Vec::new(),
                });
                self.dock.hidden = false;
                self.dock_capture_previews(serial)
            }
            Event::Pictures(serial, pictures) => {
                self.dock.preview.busy = false;
                if self
                    .dock
                    .preview
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.serial == serial)
                {
                    if let Some(preview) = self.dock.preview.preview.as_mut() {
                        preview.pictures = pictures;
                        preview.loaded = true;
                    }
                    Task::none()
                } else if self.dock.preview.hovered.is_some() && self.dock.preview.preview.is_some()
                {
                    self.dock_capture_previews(self.dock.preview.serial)
                } else {
                    Task::none()
                }
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
                self.dock_windows(vec![target], "focus")
            }
            Event::Cycle(id, delta) => {
                let Some(entry) = self.dock.entries.iter().find(|e| e.group.id == id) else {
                    return Task::none();
                };
                if self
                    .dock
                    .preview
                    .cycle
                    .as_ref()
                    .is_some_and(|(app, _, at)| {
                        *app == id && at.elapsed() < Duration::from_millis(80)
                    })
                {
                    return Task::none();
                }
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
                self.dock.preview.cycle = Some((
                    id.clone(),
                    target.stable_id.clone(),
                    std::time::Instant::now(),
                ));
                self.dock.preview.clear();
                let origin = if self.dock.preferences.position == Position::Bar {
                    Origin::Bar
                } else {
                    Origin::Dock
                };
                self.dock.preview.hovered = Some((id, origin));
                Task::batch([
                    self.dock_windows(vec![target], "focus"),
                    delay(self.dock.preview.serial, true),
                ])
            }
            _ => Task::none(),
        }
    }
    fn dock_capture_previews(&mut self, serial: u64) -> Task<Message> {
        if self.dock.preview.busy {
            return Task::none();
        }
        let Some(preview) = self.dock.preview.preview.as_ref() else {
            return Task::none();
        };
        let windows = preview.windows.clone();
        let root = self.root.clone();
        let compositor = self.compositor.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.dock.preview.cancel = Some(std::sync::Arc::clone(&cancel));
        self.dock.preview.busy = true;
        Task::perform(
            async move {
                let mut pictures = Vec::new();
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let before = compositor.dock_snapshot(&[]).await.ok();
                for window in windows.into_iter().take(16) {
                    if cancel.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                        break;
                    }
                    if before.as_ref().is_none_or(|snapshot| {
                        !snapshot.windows.iter().any(|w| w.target == window.target)
                    }) {
                        continue;
                    }
                    let picture = tokio::time::timeout(
                        Duration::from_millis(650),
                        enzui_shell::previews::capture_window(
                            root.clone(),
                            window.target.address.clone(),
                            160,
                            106,
                        ),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok);
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    pictures.push((window.target, picture));
                }
                let after = compositor.dock_snapshot(&[]).await.ok();
                pictures.retain(|(target, _)| {
                    let initial = before
                        .as_ref()
                        .and_then(|s| s.windows.iter().find(|w| w.target == *target));
                    let final_window = after
                        .as_ref()
                        .and_then(|s| s.windows.iter().find(|w| w.target == *target));
                    initial.is_some() && initial == final_window
                });
                pictures
            },
            move |pictures| message(Event::Pictures(serial, pictures)),
        )
    }
    pub(crate) fn dock_preview_view(&self) -> Option<Element<'_, Message>> {
        let preview = self.dock.preview.preview.as_ref()?;
        let palette = self.tokens().palette;
        let cards = preview
            .windows
            .iter()
            .map(|w| {
                let picture = preview
                    .pictures
                    .iter()
                    .find(|(target, _)| *target == w.target)
                    .and_then(|(_, picture)| picture.as_ref());
                let content: Element<'_, Message> = if let Some(picture) = picture {
                    container(
                        image(picture.clone())
                            .width(iced::Length::Shrink)
                            .height(iced::Length::Shrink),
                    )
                    .center_x(164)
                    .center_y(106)
                    .into()
                } else {
                    container(
                        enzui_widgets::text::enz_text(if w.minimized {
                            tr("shell-minimized")
                        } else if !preview.loaded {
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
                enzui_widgets::pressable::enz_pressable(
                    column![
                        content,
                        enzui_widgets::text::enz_text(&w.title)
                            .no_wrap()
                            .size(enzui_widgets::text::TextSize::Caption)
                            .into_element()
                    ]
                    .width(164)
                    .spacing(4)
                    .into(),
                    message(Event::Focus(w.target.clone())),
                )
                .style(enzui_widgets::pressable::PressableStyle::Plain)
                .padding(4)
                .radius(f32::from(self.dock.preferences.highlight_radius))
                .aria_label(tr!("shell-show-2", "arg0" => w.title.clone()))
                .into_element()
            })
            .collect::<Vec<_>>();
        let bounds = preview.bounds;
        let content = enzui_widgets::scroll_view::enz_scroll_view(row(cards).spacing(4))
            .axis(enzui_widgets::scroll_view::Axis::Horizontal)
            .width(bounds.width - 12.0)
            .height(bounds.height - 12.0)
            .into_element();
        let body = container(content)
            .padding(6)
            .width(bounds.width)
            .height(bounds.height)
            .style(move |_| container::Style {
                background: Some(palette.surface.into()),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..Default::default()
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
