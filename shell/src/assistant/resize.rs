use enzui_shell::{
    compositor::ControlGeometry,
    settings::{Change, MAX_ASSISTANT_WIDTH, MIN_ASSISTANT_WIDTH},
};
use iced::{Subscription, Task, mouse, window};

use super::{Message as Assistant, message};
use crate::{Message as ShellMessage, Shell};

const WIDTH_STEP: i16 = 40;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Start(f32),
    Moved(window::Id, f32),
    Released(window::Id),
    Step(bool),
    Placed(window::Id, u16, Result<ControlGeometry, String>),
}

#[derive(Default)]
pub(super) struct State {
    pub(super) drag: Option<Drag>,
    pending: Option<window::Id>,
    preferred: Option<u16>,
}

pub(super) struct Drag {
    width: u16,
    start: f32,
    current: f32,
}

impl Drag {
    fn width(&self, right: bool) -> u16 {
        let delta = self.current - self.start;
        let delta = if right { -delta } else { delta };
        (f32::from(self.width) + delta).round().clamp(
            f32::from(MIN_ASSISTANT_WIDTH),
            f32::from(MAX_ASSISTANT_WIDTH),
        ) as u16
    }
}

impl State {
    pub(super) fn subscription(&self) -> Subscription<ShellMessage> {
        if self.drag.is_none() {
            return Subscription::none();
        }
        iced::event::listen_with(|event, _, id| {
            let event = match event {
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Message::Moved(id, position.x)
                }
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Message::Released(id)
                }
                _ => return None,
            };
            Some(message(Assistant::Resize(event)))
        })
    }

    pub(super) fn preview(&self, right: bool) -> Option<u16> {
        self.drag.as_ref().map(|drag| drag.width(right))
    }
}

impl Shell {
    pub(super) fn assistant_resize(&mut self, event: Message) -> Task<ShellMessage> {
        let corner = self.desired_settings.assistant_corner;
        let state = &mut self.assistant.resize;
        let Some(pane) = self.assistant.panel.as_mut().filter(|pane| pane.visible) else {
            state.pending = None;
            state.drag = None;
            return Task::none();
        };
        let id = pane.window;
        let width = state
            .preferred
            .unwrap_or(self.desired_settings.assistant_width);
        let width = match event {
            Message::Start(offset) if state.pending.is_none() => {
                let margins =
                    enzui_shell::reveal::margins(corner.bottom(), pane.geometry.attachment);
                let x = margins.left
                    + if corner.right() {
                        0.0
                    } else {
                        pane.geometry.width - 10.0
                    }
                    + offset;
                let body = pane.geometry.width
                    - if self.metrics.islands.is_none() {
                        2.0 * f32::from(self.metrics.radius)
                    } else {
                        0.0
                    };
                state.drag = Some(Drag {
                    width: body.round() as u16,
                    start: x,
                    current: x,
                });
                return Task::none();
            }
            Message::Moved(window, x) if window == id => {
                if let Some(drag) = state.drag.as_mut() {
                    drag.current = x;
                }
                return Task::none();
            }
            Message::Released(window) if window == id => {
                let Some(drag) = state.drag.take() else {
                    return Task::none();
                };
                if (drag.current - drag.start).abs() < 2.0 {
                    return Task::none();
                }
                drag.width(corner.right())
            }
            Message::Step(wide) => width
                .saturating_add_signed(if wide { WIDTH_STEP } else { -WIDTH_STEP })
                .clamp(MIN_ASSISTANT_WIDTH, MAX_ASSISTANT_WIDTH),
            Message::Placed(window, width, result) if state.pending == Some(window) => {
                state.pending = None;
                if id != window {
                    return Task::none();
                }
                return match result {
                    Ok(geometry) => {
                        pane.geometry = geometry;
                        state.preferred = Some(width);
                        self.save_setting(Change::AssistantWidth(width))
                    }
                    Err(error) => {
                        self.assistant.error = Some(error);
                        Task::none()
                    }
                };
            }
            _ => return Task::none(),
        };
        if state.pending.is_some() {
            return Task::none();
        }
        state.pending = Some(id);
        let (compositor, metrics, bar) = (self.compositor.clone(), self.metrics, self.bar);
        super::perform(async move {
            Message::Placed(
                id,
                width,
                compositor
                    .prepare_assistant_panel(metrics, bar, corner, width)
                    .await,
            )
            .into()
        })
    }
}

impl From<Message> for Assistant {
    fn from(message: Message) -> Self {
        Self::Resize(message)
    }
}
