use super::{Message, Shell};
use enzui_shell::commands::Command;
use enzui_shell::i18n::tr;
use enzui_shell::media::{self, Control, Player, Status};
use enzui_widgets::{icon::enz_icon, text::enz_text, toolbar_icon::enz_toolbar_icon};
use iced::widget::{column, container, row};
use iced::{Element, Length, Task};

#[derive(Clone, Debug, Default)]
pub(crate) struct Media {
    players: Vec<Player>,
    // The player last controlled, preferred while several are idle.
    last: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum MediaMessage {
    Players(Vec<Player>),
    Control(Control),
    Done(Result<(), String>),
}

impl Shell {
    pub(crate) fn media_subscription(&self) -> iced::Subscription<Message> {
        media::subscription().map(|players| Message::Media(MediaMessage::Players(players)))
    }

    fn active_player(&self) -> Option<&Player> {
        media::active(&self.media.players, self.media.last.as_deref())
    }

    pub(crate) fn media_message(&mut self, message: MediaMessage) -> Task<Message> {
        match message {
            MediaMessage::Players(players) => {
                self.media.players = players;
                Task::none()
            }
            MediaMessage::Control(control) => {
                let Some(player) = self.active_player() else {
                    return Task::none();
                };
                let bus = player.bus.clone();
                self.media.last = Some(bus.clone());
                Task::perform(media::control(bus, control), |result| {
                    Message::Media(MediaMessage::Done(result))
                })
            }
            MediaMessage::Done(result) => {
                if let Err(error) = result {
                    self.notice = Some(error);
                }
                Task::none()
            }
        }
    }

    pub(crate) fn media_command(&mut self, command: Command) -> Task<Message> {
        let control = match command {
            Command::MediaPlayPause => Control::PlayPause,
            Command::MediaNext => Control::Next,
            Command::MediaPrevious => Control::Previous,
            Command::MediaStop => Control::Stop,
            _ => return Task::none(),
        };
        self.media_message(MediaMessage::Control(control))
    }

    // The control center's now-playing card, while a player exists.
    pub(crate) fn media_card(&self) -> Option<Element<'_, Message>> {
        let player = self.active_player()?;
        let palette = self.tokens().palette;
        let font = f32::from(self.metrics.text);
        let art: Element<'_, Message> = match &player.art {
            Some(path) => iced::widget::image(path.clone())
                .width(44.0)
                .height(44.0)
                .into(),
            None => container(enz_icon("music").size(22.0).into_element())
                .center(44.0)
                .into(),
        };
        let title = if player.title.is_empty() {
            player.identity.clone()
        } else {
            player.title.clone()
        };
        let subtitle = if player.artist.is_empty() {
            player.identity.clone()
        } else {
            player.artist.clone()
        };
        let playing = player.status == Status::Playing;
        let button = |icon: &'static str,
                      label: &'static str,
                      id: &'static str,
                      control: Control,
                      enabled: bool| {
            enz_toolbar_icon(icon, Message::Media(MediaMessage::Control(control)))
                .tokens(self.icon_tokens())
                .aria_label(label)
                .id(id.into())
                .enabled(enabled)
                .into_element()
        };
        Some(
            container(
                row![
                    art,
                    column![
                        enz_text(title)
                            .size_px(font)
                            .weight_rung(enzui_theme::WeightRung::Semibold)
                            .single_line_elided()
                            .into_element(),
                        enz_text(subtitle)
                            .size_px((font - 1.0).max(10.0))
                            .muted_on(palette.surface)
                            .single_line_elided()
                            .into_element(),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    button(
                        "skip-back",
                        tr("shell-previous-track"),
                        "media-previous",
                        Control::Previous,
                        player.can_previous
                    ),
                    button(
                        if playing { "pause" } else { "play" },
                        if playing {
                            enzui_shell::i18n::tr("shell-a11y-pause")
                        } else {
                            enzui_shell::i18n::tr("shell-key-play")
                        },
                        "media-play-pause",
                        Control::PlayPause,
                        player.can_play_pause
                    ),
                    button(
                        "skip-forward",
                        tr("shell-next-track"),
                        "media-next",
                        Control::Next,
                        player.can_next
                    ),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            )
            .padding(10)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(palette.surface_alt.into()),
                border: iced::Border {
                    radius: 14.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
        )
    }
}
