use super::{Message, Shell, window_settings};
use enzui_shell::tr;
#[cfg(test)]
mod tests;
use enzui_shell::commands::Command;
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::search::compositor::{SwitchEntry, SwitcherSnapshot};
use enzui_widgets::{icon::enz_icon, text::enz_text};
use iced::widget::{column, container, row};
use iced::{Element, Length, Task, window};

const TILE: f32 = 112.0;
const HEIGHT: f32 = 170.0;
const SHADOW_ROOM: f32 = 24.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct Switcher {
    entries: Vec<SwitchEntry>,
    selected: usize,
    revision: u64,
    epoch: u64,
    active: bool,
    reading: bool,
    reread: bool,
    window: Option<window::Id>,
    placing: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum SwitcherMessage {
    Loaded(u64, Result<SwitcherSnapshot, String>),
    Placed(u64, Result<ControlGeometry, String>),
    Pick(u64, usize),
    Focused(Result<(), String>),
}

fn visible_entries(count: usize, selected: usize) -> std::ops::Range<usize> {
    let length = count.min(10);
    let start = selected
        .saturating_sub(length / 2)
        .min(count.saturating_sub(length));
    start..start + length
}

impl Shell {
    pub(crate) fn switcher_command(&mut self, command: Command) -> Task<Message> {
        if command == Command::SwitchState {
            return self.refresh_switcher();
        }
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.switcher_action(command).await },
            |result| Message::Switcher(SwitcherMessage::Focused(result)),
        )
    }
    fn refresh_switcher(&mut self) -> Task<Message> {
        if self.switcher.reading {
            self.switcher.reread = true;
            return Task::none();
        }
        self.switcher.reading = true;
        self.switcher.epoch = self.switcher.epoch.wrapping_add(1);
        let epoch = self.switcher.epoch;
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.switcher_snapshot().await },
            move |result| Message::Switcher(SwitcherMessage::Loaded(epoch, result)),
        )
    }
    fn close_switcher(&mut self) -> Task<Message> {
        self.switcher.active = false;
        self.switcher.entries.clear();
        self.switcher
            .window
            .take()
            .map_or_else(Task::none, window::close)
    }
    pub(crate) fn switcher_message(&mut self, message: SwitcherMessage) -> Task<Message> {
        match message {
            SwitcherMessage::Loaded(epoch, result) if epoch == self.switcher.epoch => {
                self.switcher.reading = false;
                if std::mem::take(&mut self.switcher.reread) {
                    return self.refresh_switcher();
                }
                let Ok(state) = result else {
                    return self.close_switcher();
                };
                if state.revision < self.switcher.revision {
                    return Task::none();
                }
                self.switcher.revision = state.revision;
                if !state.active {
                    return self.close_switcher();
                }
                self.switcher.active = true;
                self.switcher.entries = state.entries;
                self.switcher.selected = state.selected;
                if self.switcher.entries.len() < 2
                    || self.switcher.window.is_some()
                    || self.switcher.placing
                {
                    return Task::none();
                }
                self.switcher.placing = true;
                let compositor = self.compositor.clone();
                let width = self.switcher_width();
                let (metrics, bar) = (self.metrics, self.bar);
                Task::perform(
                    async move {
                        compositor
                            .prepare_overlay(
                                "dev.enzui.Switcher",
                                width + 2.0 * SHADOW_ROOM,
                                HEIGHT + 2.0 * SHADOW_ROOM,
                                true,
                                metrics,
                                bar,
                            )
                            .await
                    },
                    move |result| Message::Switcher(SwitcherMessage::Placed(epoch, result)),
                )
            }
            SwitcherMessage::Loaded(..) => Task::none(),
            SwitcherMessage::Placed(epoch, result) => {
                self.switcher.placing = false;
                if result.is_err()
                    || epoch != self.switcher.epoch
                    || !self.switcher.active
                    || self.switcher.window.is_some()
                {
                    return if self.switcher.active && self.switcher.window.is_none() {
                        self.refresh_switcher()
                    } else {
                        Task::none()
                    };
                }
                let (id, open) = window::open(window_settings(
                    "dev.enzui.Switcher",
                    self.switcher_width() + 2.0 * SHADOW_ROOM,
                    HEIGHT + 2.0 * SHADOW_ROOM,
                ));
                self.switcher.window = Some(id);
                open.map(Message::Opened)
            }
            SwitcherMessage::Pick(revision, index)
                if self.switcher.active
                    && revision == self.switcher.revision
                    && index < self.switcher.entries.len() =>
            {
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.switcher_pick(revision, index).await },
                    |result| Message::Switcher(SwitcherMessage::Focused(result)),
                )
            }
            SwitcherMessage::Pick(..) => Task::none(),
            SwitcherMessage::Focused(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                self.refresh_switcher()
            }
        }
    }
    pub(crate) fn owns_switcher(&self, id: window::Id) -> bool {
        self.switcher.window == Some(id)
    }
    fn switcher_width(&self) -> f32 {
        (self.switcher.entries.len().clamp(2, 10) as f32 * (TILE + 8.0) + 32.0).min(1400.0)
    }
    pub(crate) fn switcher_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if self.switcher.window != Some(id) {
            return None;
        }
        let palette = self.window_tokens(Some(id)).palette;
        let accent = palette.accent;
        let visible = visible_entries(self.switcher.entries.len(), self.switcher.selected);
        let tiles: Vec<Element<'_, Message>> = self.switcher.entries.iter().enumerate().skip(visible.start).take(visible.len()).map(|(index, entry)| {
            let selected = index == self.switcher.selected;
            let icon: Element<'_, Message> = match &entry.image {
                Some(path) if path.extension().is_some_and(|e| e == "svg") => iced::widget::svg(iced::widget::svg::Handle::from_path(path)).width(48.0).height(48.0).into(),
                Some(path) => iced::widget::image(path.clone()).width(48.0).height(48.0).into(),
                None => enz_icon("app-window").size(40.0).into_element(),
            };
            // Tiles name the app; the selected window's own title is the caption below the row.
            let label = enzui_shell::search::catalogue::app_identity(&entry.class)
                .0
                .unwrap_or_else(|| if entry.title.is_empty() { entry.class.clone() } else { entry.title.clone() });
            let tile = container(column![container(icon).center_x(Length::Fill), enz_text(label.clone()).size_px(enzui_theme::text_scale::effective().apply_px(12.0)).single_line_elided().into_element()].spacing(8).align_x(iced::Alignment::Center))
                .padding(10).width(TILE).style(move |_| container::Style {
                    background: selected.then(|| iced::Color { a: 0.25, ..accent }.into()),
                    border: iced::Border { radius: 14.0.into(), width: if selected { 2.0 } else { 0.0 }, color: accent },
                    ..Default::default()
                });
            enzui_widgets::clickable::enz_clickable(tile, Message::Switcher(SwitcherMessage::Pick(self.switcher.revision, index)))
                .aria_label(tr!("shell-of-3", "arg0" => label, "arg1" => format!("{:}", index + 1), "arg2" => format!("{:}", self.switcher.entries.len()))).id(format!("switcher-{index}").into()).into_element()
        }).collect();
        // The selected window's own title, when it says more than the app's name.
        let caption = self
            .switcher
            .entries
            .get(self.switcher.selected)
            .and_then(|entry| {
                let name = enzui_shell::search::catalogue::app_identity(&entry.class).0;
                // The document part only: "README.md — Text Editor" named the
                // app a second time, under a tile that already names it.
                let title = entry
                    .title
                    .split_once(" — ")
                    .map_or(entry.title.as_str(), |(document, _)| document);
                (!title.is_empty() && name.as_deref() != Some(title)).then(|| title.to_owned())
            });
        let mut content = column![row(tiles).spacing(8).align_y(iced::Alignment::Center)]
            .spacing(6)
            .align_x(iced::Alignment::Center);
        if let Some(caption) = caption {
            // Under the selected tile rather than the middle of the row, so
            // the title reads as that window's.
            const CAPTION: f32 = 240.0;
            let count = visible.len() as f32;
            let row_width = count * TILE + (count - 1.0).max(0.0) * 8.0;
            let centre = (self.switcher.selected.saturating_sub(visible.start)) as f32
                * (TILE + 8.0)
                + TILE / 2.0;
            let width = CAPTION.min(row_width);
            let left = (centre - width / 2.0).clamp(0.0, (row_width - width).max(0.0));
            content = content.push(
                container(
                    container(
                        enz_text(caption)
                            .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                            .muted_on(palette.surface)
                            .single_line_elided()
                            .into_element(),
                    )
                    .center_x(Length::Fixed(width)),
                )
                .padding(iced::Padding::default().left(left))
                .width(Length::Fixed(row_width)),
            );
        }
        let body = container(content)
            .center(Length::Fill)
            .style(move |_| container::Style {
                text_color: Some(palette.text),
                ..Default::default()
            });
        Some(self.transient_glass_lifted(id, body, 22.0, SHADOW_ROOM))
    }
}
