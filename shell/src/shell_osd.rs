use super::{Message, Shell, window_settings};
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::i18n::tr;
use enzui_widgets::{icon::enz_icon, progress_bar::enz_progress_bar, text::enz_text};
use iced::widget::{container, row};
use iced::{Element, Task, window};
use std::time::Duration;

const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 56.0;
/// Room around the pill for its shadow, so it stays distinct over a window of
/// its own colour.
const SHADOW_ROOM: f32 = 16.0;
const SHOWN: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Volume,
    Microphone,
    Brightness,
}

// The transient level display for hardware keys: never focused, closed
// again shortly after the last change.
#[derive(Clone, Debug, Default)]
pub(crate) struct Osd {
    window: Option<window::Id>,
    placing: bool,
    shown: Option<(Level, u8, bool)>,
    serial: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum OsdMessage {
    Placed(Result<ControlGeometry, String>),
    Expire(u64),
}

impl Shell {
    // Shows `level` at `percent` (muted draws the muted icon) and restarts
    // the hide timer.
    pub(crate) fn show_osd(&mut self, level: Level, percent: u8, muted: bool) -> Task<Message> {
        self.osd.shown = Some((level, percent, muted));
        self.osd.serial = self.osd.serial.wrapping_add(1);
        let serial = self.osd.serial;
        let expire = Task::perform(tokio::time::sleep(SHOWN), move |()| {
            Message::Osd(OsdMessage::Expire(serial))
        });
        if self.osd.window.is_some() || std::mem::replace(&mut self.osd.placing, true) {
            return expire;
        }
        let compositor = self.compositor.clone();
        let (metrics, bar) = (self.metrics, self.bar);
        Task::batch([
            Task::perform(
                async move {
                    compositor
                        .prepare_overlay(
                            "dev.enzui.Osd",
                            WIDTH + 2.0 * SHADOW_ROOM,
                            HEIGHT + 2.0 * SHADOW_ROOM,
                            false,
                            metrics,
                            bar,
                        )
                        .await
                },
                |result| Message::Osd(OsdMessage::Placed(result)),
            ),
            expire,
        ])
    }

    pub(crate) fn osd_message(&mut self, message: OsdMessage) -> Task<Message> {
        match message {
            OsdMessage::Placed(result) => {
                self.osd.placing = false;
                if result.is_err() || self.osd.shown.is_none() || self.osd.window.is_some() {
                    return Task::none();
                }
                let (id, open) = window::open(window_settings(
                    "dev.enzui.Osd",
                    WIDTH + 2.0 * SHADOW_ROOM,
                    HEIGHT + 2.0 * SHADOW_ROOM,
                ));
                self.osd.window = Some(id);
                open.map(Message::Opened)
            }
            OsdMessage::Expire(serial) if serial == self.osd.serial => {
                self.osd.shown = None;
                self.osd
                    .window
                    .take()
                    .map_or_else(Task::none, window::close)
            }
            OsdMessage::Expire(_) => Task::none(),
        }
    }

    pub(crate) fn owns_osd(&self, id: window::Id) -> bool {
        self.osd.window == Some(id)
    }

    pub(crate) fn osd_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if self.osd.window != Some(id) {
            return None;
        }
        let palette = self.window_tokens(Some(id)).palette;
        let Some((level, percent, muted)) = self.osd.shown else {
            return Some(iced::widget::space().into());
        };
        let icon = match (level, muted) {
            (Level::Volume, true) => "volume-x",
            (Level::Volume, false) if percent < 34 => "volume-1",
            (Level::Volume, false) => "volume-2",
            (Level::Microphone, true) => "mic-off",
            (Level::Microphone, false) => "mic",
            (Level::Brightness, _) => "sun",
        };
        let what = match level {
            Level::Volume => tr("shell-volume"),
            Level::Microphone => tr("shell-microphone"),
            Level::Brightness => tr("shell-brightness"),
        };
        let value = if muted {
            tr("shell-muted").to_owned()
        } else {
            format!("{percent}%")
        };
        let content = row![
            enz_icon(icon).size(22.0).into_element(),
            container(
                enz_progress_bar(if muted {
                    0.0
                } else {
                    f32::from(percent) / 100.0
                })
                .bar_height(8.0)
                .into_element()
            )
            .width(iced::Length::Fill),
            // The value hugs the bar's end; no dead space on the right.
            container(
                enz_text(value.clone())
                    .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                    .color(palette.text_muted)
                    .into_element()
            )
            .width(44.0)
            .align_x(iced::Alignment::End),
        ]
        .spacing(14)
        .align_y(iced::Alignment::Center);
        let pill = container(content)
            .padding([0, 18])
            .center_y(iced::Length::Fill)
            .width(iced::Length::Fill)
            .style(move |_| container::Style {
                text_color: Some(palette.text),
                ..Default::default()
            });
        Some(
            container(self.transient_glass_lifted(id, pill, HEIGHT / 2.0, SHADOW_ROOM))
                .id(iced::advanced::widget::Id::from(format!(
                    "osd {what} {value}"
                )))
                .into(),
        )
    }
}
