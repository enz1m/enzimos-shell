use enzui_shell::{
    reveal::Attachment,
    settings::{Change, MAX_ASSISTANT_WIDTH, MIN_ASSISTANT_WIDTH},
};
use iced::{Subscription, Task, mouse, window};

use super::{Message as Assistant, message};
use crate::{Message as ShellMessage, Shell};

const WIDTH_STEP: i16 = 40;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Start,
    Moved(window::Id, f32),
    Released(window::Id),
    Step(bool),
}

pub(super) struct Drag {
    width: u16,
    /// Set from the first pointer motion so the drag uses the window's own coordinates.
    start: Option<f32>,
    current: f32,
}

impl Drag {
    fn width(&self, right: bool) -> u16 {
        let Some(start) = self.start else {
            return self.width;
        };
        let delta = self.current - start;
        let delta = if right { -delta } else { delta };
        (f32::from(self.width) + delta).round().clamp(
            f32::from(MIN_ASSISTANT_WIDTH),
            f32::from(MAX_ASSISTANT_WIDTH),
        ) as u16
    }
}

pub(super) fn subscription() -> Subscription<ShellMessage> {
    iced::event::listen_with_interest(iced::event::Interest::POINTER, |event, _, id| {
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

impl Shell {
    pub(super) fn assistant_body_width(&self) -> f32 {
        let right = self.desired_settings.assistant_corner.right();
        let width = self
            .assistant
            .drag
            .as_ref()
            .map_or(self.desired_settings.assistant_width, |drag| {
                drag.width(right)
            });
        let join = if self.metrics.islands.is_none() {
            2.0 * f32::from(self.metrics.radius)
        } else {
            0.0
        };
        self.assistant.panel.as_ref().map_or_else(
            || f32::from(width),
            |pane| (f32::from(width) + join).min(pane.geometry.width),
        )
    }

    pub(super) fn assistant_resize(&mut self, event: Message) -> Task<ShellMessage> {
        let corner = self.desired_settings.assistant_corner;
        let Some(pane) = self.assistant.panel.as_ref().filter(|pane| pane.visible) else {
            self.assistant.drag = None;
            return Task::none();
        };
        let width = self.assistant_body_width();
        let preferred = self.desired_settings.assistant_width;
        let next = match event {
            Message::Start => {
                let join = if matches!(pane.geometry.attachment, Attachment::Bar) {
                    2.0 * f32::from(self.metrics.radius)
                } else {
                    0.0
                };
                self.assistant.drag = Some(Drag {
                    width: (width - join).round() as u16,
                    start: None,
                    current: 0.0,
                });
                return Task::none();
            }
            Message::Moved(window, x) if window == pane.window => {
                if let Some(drag) = self.assistant.drag.as_mut() {
                    drag.start.get_or_insert(x);
                    drag.current = x;
                }
                return Task::none();
            }
            Message::Released(window) if window == pane.window => {
                let Some(drag) = self.assistant.drag.take() else {
                    return Task::none();
                };
                if drag
                    .start
                    .is_none_or(|start| (drag.current - start).abs() < 2.0)
                {
                    return Task::none();
                }
                drag.width(corner.right())
            }
            Message::Step(wide) if self.assistant.drag.is_none() => preferred
                .saturating_add_signed(if wide { WIDTH_STEP } else { -WIDTH_STEP })
                .clamp(MIN_ASSISTANT_WIDTH, MAX_ASSISTANT_WIDTH),
            _ => return Task::none(),
        };
        if next == preferred {
            return Task::none();
        }
        self.desired_settings.assistant_width = next;
        self.save_setting(Change::AssistantWidth(next))
    }
}
