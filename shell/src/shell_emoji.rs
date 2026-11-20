use super::{Message, Shell, window_settings};
use enzui_shell::compositor::ControlGeometry;
use enzui_shell::emoji::Emoji;
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::text::{TextSize, enz_text};
use iced::widget::{column, container, row, scrollable};
use iced::{Element, Length, Task, window};
use std::sync::Arc;

const WIDTH: f32 = 452.0;
const HEIGHT: f32 = 460.0;
const COLUMNS: usize = 8;
const SHOWN: usize = 160;

// Emoji picker types through the virtual keyboard or falls back to clipboard copy.
#[derive(Clone, Debug, Default)]
pub(crate) struct EmojiPicker {
    window: Option<window::Id>,
    placing: bool,
    all: Arc<Vec<Emoji>>,
    recent: Vec<String>,
    query: String,
    selected: usize,
    can_type: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum EmojiMessage {
    Placed(Result<ControlGeometry, String>),
    Query(String),
    Choose(String),
    Typed(String, Result<(), String>),
    Copied(String, bool),
}

impl EmojiPicker {
    pub(crate) fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }
}

impl Shell {
    pub(crate) fn open_emoji_picker(&mut self) -> Task<Message> {
        if self.emoji.window.is_some() || self.emoji.placing {
            return self.close_emoji_picker();
        }
        if self.emoji.all.is_empty() {
            self.emoji.all = Arc::new(enzui_shell::emoji::load());
        }
        if self.emoji.all.is_empty() {
            self.notice = Some(tr("shell-emoji-are-not-installed-unicode-emoji").into());
            return Task::none();
        }
        self.emoji.recent = enzui_shell::emoji::recent();
        self.emoji.query.clear();
        self.emoji.selected = 0;
        self.emoji.can_type = enzui_shell::virtual_keyboard::available();
        self.emoji.placing = true;
        let compositor = self.compositor.clone();
        let (metrics, bar) = (self.metrics, self.bar);
        Task::perform(
            async move {
                compositor
                    .prepare_overlay("dev.enzui.EmojiPicker", WIDTH, HEIGHT, true, metrics, bar)
                    .await
            },
            |result| Message::Emoji(EmojiMessage::Placed(result)),
        )
    }

    pub(crate) fn close_emoji_picker(&mut self) -> Task<Message> {
        self.emoji.placing = false;
        self.emoji
            .window
            .take()
            .map_or_else(Task::none, window::close)
    }

    pub(crate) fn emoji_window(&mut self, event: &window::Event) -> Task<Message> {
        match event {
            window::Event::Unfocused | window::Event::CloseRequested => self.close_emoji_picker(),
            window::Event::Closed => {
                self.emoji.window = None;
                Task::none()
            }
            _ => Task::none(),
        }
    }

    pub(crate) fn emoji_opened(&mut self, id: window::Id) -> Task<Message> {
        Task::batch([
            iced::widget::operation::focus("emoji-search"),
            self.bind_transient_glass(id),
        ])
    }

    fn emoji_matches(&self) -> Vec<String> {
        if self.emoji.query.trim().is_empty() {
            let mut list = self.emoji.recent.clone();
            list.extend(
                self.emoji
                    .all
                    .iter()
                    .map(|e| e.text.clone())
                    .filter(|text| !self.emoji.recent.contains(text))
                    .take(SHOWN),
            );
            list.truncate(SHOWN);
            return list;
        }
        enzui_shell::emoji::search(&self.emoji.all, &self.emoji.query)
            .into_iter()
            .take(SHOWN)
            .map(|e| e.text.clone())
            .collect()
    }

    pub(crate) fn emoji_key(&mut self, key: &iced::keyboard::Key) -> Task<Message> {
        use iced::keyboard::{Key, key::Named};
        let count = self.emoji_matches().len();
        let step: i64 = match key {
            Key::Named(Named::ArrowRight) => 1,
            Key::Named(Named::ArrowLeft) => -1,
            Key::Named(Named::ArrowDown) => COLUMNS as i64,
            Key::Named(Named::ArrowUp) => -(COLUMNS as i64),
            Key::Named(Named::Enter) => {
                return match self.emoji_matches().get(self.emoji.selected).cloned() {
                    Some(text) => self.emoji_message(EmojiMessage::Choose(text)),
                    None => Task::none(),
                };
            }
            _ => return Task::none(),
        };
        if count > 0 {
            self.emoji.selected =
                (self.emoji.selected as i64 + step).clamp(0, count as i64 - 1) as usize;
        }
        Task::none()
    }

    pub(crate) fn emoji_message(&mut self, message: EmojiMessage) -> Task<Message> {
        match message {
            EmojiMessage::Placed(result) => {
                if !std::mem::replace(&mut self.emoji.placing, false) || self.emoji.window.is_some()
                {
                    return Task::none();
                }
                if let Err(error) = result {
                    self.notice = Some(error);
                    return Task::none();
                }
                let (id, open) =
                    window::open(window_settings("dev.enzui.EmojiPicker", WIDTH, HEIGHT));
                self.emoji.window = Some(id);
                open.map(Message::Opened)
            }
            EmojiMessage::Query(query) => {
                self.emoji.query = query.chars().take(64).collect();
                self.emoji.selected = 0;
                Task::none()
            }
            EmojiMessage::Choose(text) => {
                enzui_shell::emoji::remember(&text);
                if !self.emoji.can_type {
                    // Written while the picker still has focus, so the
                    // compositor accepts it as this desktop's selection.
                    let copy = iced::clipboard::write(text.clone()).map(move |result| {
                        Message::Emoji(EmojiMessage::Copied(text.clone(), result.is_ok()))
                    });
                    return copy.chain(self.close_emoji_picker());
                }
                let close = self.close_emoji_picker();
                let typed = text.clone();
                // Focus returns to the previous window once the picker is gone.
                let type_it = Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        tokio::task::spawn_blocking(move || {
                            enzui_shell::virtual_keyboard::type_text(&typed)
                        })
                        .await
                        .unwrap_or_else(|_| Err(tr("shell-typing-stopped").into()))
                    },
                    move |result| Message::Emoji(EmojiMessage::Typed(text, result)),
                );
                close.chain(type_it)
            }
            EmojiMessage::Typed(_, Ok(())) => Task::none(),
            EmojiMessage::Typed(text, Err(error)) => {
                eprintln!("enzui-shell: emoji: {error}");
                self.notice = Some(tr!("shell-could-not-be-typed-here", "arg0" => text));
                Task::none()
            }
            EmojiMessage::Copied(text, copied) => {
                self.notice = Some(if copied {
                    tr!("shell-copied-paste-it-with-ctrl-v", "arg0" => text)
                } else {
                    tr("shell-the-emoji-could-not-be-copied").into()
                });
                Task::none()
            }
        }
    }

    pub(crate) fn emoji_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        if self.emoji.window != Some(id) {
            return None;
        }
        let palette = self.window_tokens(Some(id)).palette;
        let accent = palette.accent;
        let matches = self.emoji_matches();
        let names: std::collections::HashMap<&str, &str> = self
            .emoji
            .all
            .iter()
            .map(|e| (e.text.as_str(), e.name.as_str()))
            .collect();
        let mut grid = column![].spacing(4);
        for (line, chunk) in matches.chunks(COLUMNS).enumerate() {
            let mut cells = row![].spacing(4);
            for (offset, text) in chunk.iter().enumerate() {
                let index = line * COLUMNS + offset;
                let selected = index == self.emoji.selected;
                // Named, so the colour glyphs win over monochrome symbol fonts.
                let glyph = iced::widget::text(text.clone())
                    .size(26.0)
                    .font(iced::Font::new(tr("shell-noto-color-emoji")));
                let cell =
                    container(glyph)
                        .center_x(44)
                        .center_y(44)
                        .style(move |_| container::Style {
                            background: selected.then(|| iced::Color { a: 0.25, ..accent }.into()),
                            border: iced::Border {
                                radius: 10.0.into(),
                                width: if selected { 2.0 } else { 0.0 },
                                color: accent,
                            },
                            ..Default::default()
                        });
                let name = names.get(text.as_str()).copied().unwrap_or("emoji");
                cells = cells.push(
                    enzui_widgets::clickable::enz_clickable(
                        cell,
                        Message::Emoji(EmojiMessage::Choose(text.clone())),
                    )
                    .aria_label(name.to_owned())
                    .id(format!("emoji-{index}").into())
                    .into_element(),
                );
            }
            grid = grid.push(cells);
        }
        // A caption only when it says something: recents lead the list, or
        // nothing matched.
        let heading = if self.emoji.query.trim().is_empty() && !self.emoji.recent.is_empty() {
            Some(tr("shell-recent-and-all-emoji"))
        } else if matches.is_empty() {
            Some(tr("shell-no-emoji-found"))
        } else {
            None
        };
        let footer = if self.emoji.can_type {
            tr("shell-enter-types-the-emoji-into-your-window")
        } else {
            tr("shell-choosing-an-emoji-copies-it-to-the-clipboard")
        };
        // Laid out as the other popups: a title, then a pill search field
        // like the notification history's.
        let mut content = column![
            enz_text(tr("shell-emoji"))
                .size_px(enzui_shell::panel_style::title_px(f32::from(
                    self.metrics.text
                )))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            enzui_widgets::search_field::enz_search_field(&self.emoji.query, |query| {
                Message::Emoji(EmojiMessage::Query(query))
            })
            .placeholder(tr("shell-type-a-name-like-heart-or-cat"))
            .id("emoji-search")
            .radius(18.0)
            .width(Length::Fill)
            .into_element(),
        ]
        .spacing(enzui_shell::panel_style::GAP);
        if let Some(heading) = heading {
            content = content.push(
                enz_text(heading)
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .into_element(),
            );
        }
        let content = content.push(
            column![
                // Room for the scrollbar beside the last column.
                scrollable(container(grid).padding(iced::Padding {
                    right: 14.0,
                    ..Default::default()
                }))
                .height(Length::Fill),
                enz_text(footer)
                    .size(TextSize::Footnote)
                    .muted_on(palette.surface)
                    .into_element(),
            ]
            .spacing(10),
        );
        let body = container(content)
            .padding(enzui_shell::panel_style::INSET)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                text_color: Some(palette.text),
                ..Default::default()
            });
        Some(self.transient_glass(id, body, 22.0))
    }
}
