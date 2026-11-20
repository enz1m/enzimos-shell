use super::super::{Message as AppMessage, SettingsApp};
use super::preview::{Message, PreviewVisual, message, parse_color, preview_element};
use enzui_shell::desktop_preferences::{WallpaperFit, WallpaperKind};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    clickable::{ClickableTokens, enz_clickable},
    color_picker::enz_color_picker,
    image::{EnzImageFit, enz_image},
    motion::cross_fade,
    responsive_grid::{CellFn, enz_responsive_grid_keyed},
    text::enz_text,
};
use iced::{
    Element, Length,
    widget::{Space, column, container, responsive, row},
};

impl SettingsApp {
    pub(crate) fn wallpaper_settings(&self) -> Element<'_, AppMessage> {
        if !self.wallpaper.supported {
            return enz_text(tr(
                "shell-start-a-new-managed-desktop-to-use-wallpaper-controls-existing-desktops-are",
            ))
            .into_element();
        }
        let p = &self.settings.wallpaper;
        let enabled = !self.closing && !self.wallpaper.busy && self.picker_read.is_none();
        let palette = self.tokens().palette;
        let kind = self.wallpaper.view_kind.unwrap_or(p.kind);
        let incoming = self.wallpaper.displayed(p);
        let outgoing = self
            .wallpaper
            .preview_previous
            .clone()
            .unwrap_or_else(|| incoming.clone());
        let incoming_handle = match &incoming {
            PreviewVisual::Image(path) => self.wallpaper.handle(path),
            _ => None,
        };
        let outgoing_handle = match &outgoing {
            PreviewVisual::Image(path) => self.wallpaper.handle(path),
            _ => None,
        };
        let slot = self.wallpaper.preview_slot;
        let warming = self
            .wallpaper
            .preview_warming
            .as_ref()
            .and_then(|(_, path)| self.wallpaper.handle(path));
        let preview = responsive(move |size| -> Element<'_, AppMessage> {
            let width = size.width.min(520.0);
            let height = width * 9.0 / 16.0;
            // The current picture has no ground of its own: one still
            // reaching the GPU shows the previous one, never a grey frame.
            let current = preview_element(
                incoming.clone(),
                incoming_handle.clone(),
                width,
                height,
                iced::Color::TRANSPARENT,
            );
            let previous = preview_element(
                outgoing.clone(),
                outgoing_handle.clone(),
                width,
                height,
                palette.surface_alt,
            );
            let picture = if slot {
                cross_fade(previous, current, true)
            } else {
                cross_fade(current, previous, false)
            };
            let mut layers = vec![
                container(picture)
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center)
                    .into(),
            ];
            if let Some(handle) = warming.clone() {
                layers.push(
                    iced::widget::image(handle)
                        .width(1)
                        .height(1)
                        .opacity(0.01_f32)
                        .into(),
                );
            }
            iced::widget::stack(layers).into()
        })
        .height(Length::Shrink);
        let preview: Element<'_, AppMessage> =
            enzui_widgets::file_drop::enz_file_drop_target_with_receipt(preview, |drop| {
                message(Message::Dropped(drop))
            })
            .enabled(enabled && self.wallpaper.native_drop.is_none())
            .copy_only(true)
            .key(("wallpaper-picture", self.wallpaper.gallery_generation))
            .into();
        let source = self.choice_setting(
            "wallpaper/source",
            match kind {
                WallpaperKind::Default => tr("shell-default"),
                WallpaperKind::Color => tr("shell-solid-color"),
                WallpaperKind::Image => tr("shell-picture"),
                WallpaperKind::Live => tr("shell-live"),
            },
            vec![
                (
                    tr("shell-default").into(),
                    message(Message::Kind(WallpaperKind::Default)),
                ),
                (
                    tr("shell-picture").into(),
                    message(Message::Kind(WallpaperKind::Image)),
                ),
                (
                    tr("shell-solid-color").into(),
                    message(Message::Kind(WallpaperKind::Color)),
                ),
                (
                    tr("shell-live").into(),
                    message(Message::Kind(WallpaperKind::Live)),
                ),
            ],
        );
        let mut body = column![preview, source].spacing(14);
        if kind == WallpaperKind::Color {
            let mut picker =
                enz_color_picker(self.wallpaper.hsva, |value| message(Message::Picked(value)))
                    .label(tr("shell-background-color"))
                    .id_prefix("wallpaper-color")
                    .hex_text(self.wallpaper.color.clone())
                    .on_hex_input(|text| message(Message::Color(text)))
                    .plane_height(200.0)
                    .enabled(enabled);
            if parse_color(&self.wallpaper.color).is_none() {
                picker = picker.hex_error(tr("shell-use-followed-by-six-hexadecimal-digits"));
            }
            body = body.push(
                self.mark_setting(
                    "wallpaper/color",
                    container(
                        container(
                            column![
                                picker.into_element(),
                                row![
                                    Space::new().width(Length::Fill),
                                    enz_button(
                                        enzui_shell::i18n::tr("shell-cancel"),
                                        message(Message::CancelColor)
                                    )
                                    .kind(ButtonKind::Quiet)
                                    .enabled(enabled)
                                    .into_element(),
                                    enz_button(tr("shell-use-color"), message(Message::ApplyColor))
                                        .icon("check")
                                        .enabled(
                                            enabled && parse_color(&self.wallpaper.color).is_some()
                                        )
                                        .into_element(),
                                ]
                                .spacing(8),
                            ]
                            .spacing(12),
                        )
                        .width(Length::Fill.max(520.0)),
                    )
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center),
                ),
            );
        } else if kind == WallpaperKind::Live {
            body = body.push(
                self.choice_setting(
                    "wallpaper/motion",
                    p.motion.label(),
                    enzui_shell::desktop_preferences::WallpaperMotion::ALL
                        .into_iter()
                        .map(|m| (m.label().into(), message(Message::Motion(m))))
                        .collect(),
                ),
            );
            body = body.push(
                self.switch_setting("wallpaper/pause_idle", p.pause_idle, |pause| {
                    message(Message::PauseIdle(pause))
                }),
            );
            body = body.push(
                self.switch_setting("wallpaper/hide_clock", p.hide_clock, |hide| {
                    message(Message::HideClock(hide))
                }),
            );
            body = body.push(
                self.switch_setting("wallpaper/hide_date", p.hide_date, |hide| {
                    message(Message::HideDate(hide))
                }),
            );
            body = body.push(self.mark_setting("wallpaper/live", self.live_gallery()));
        } else {
            let header = row![
                container(
                    column![
                        enz_text(enzui_shell::i18n::tr("shell-wallpapers"))
                            .size_px(self.text_px(16.0))
                            .into_element(),
                        enz_text(enzui_shell::i18n::tr("shell-pictures-wallpapers"))
                            .size_px(self.text_px(12.0))
                            .muted_on(palette.background)
                            .into_element()
                    ]
                    .spacing(2)
                )
                .width(Length::Fill),
                enz_button(
                    if self.wallpaper.gallery_loading {
                        enzui_shell::i18n::tr("shell-loading")
                    } else {
                        enzui_shell::i18n::tr("shell-refresh")
                    },
                    message(Message::RefreshGallery)
                )
                .kind(ButtonKind::Quiet)
                .icon("refresh-cw")
                .size(ButtonSize::Sm)
                // Browsing remains enabled during import; handlers reject re-entry.
                .enabled(
                    self.wallpaper
                        .can_choose_source(self.closing, self.picker_read.is_some())
                        && !self.wallpaper.gallery_loading,
                )
                .into_element(),
                enz_button(tr("shell-choose-file"), message(Message::Browse))
                    .kind(ButtonKind::Quiet)
                    .icon("folder-open")
                    .size(ButtonSize::Sm)
                    .enabled(
                        self.wallpaper
                            .can_choose_source(self.closing, self.picker_read.is_some()),
                    )
                    .into_element(),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center);
            body = body.push(header);
            let cells: Vec<(String, CellFn<'_, AppMessage>)> = self
                .wallpaper
                .gallery
                .entries
                .iter()
                .map(|entry| {
                    let selected = self.wallpaper.gallery_choice.as_ref() == Some(&entry.id);
                    let generation = self.wallpaper.gallery_generation;
                    let tokens = ClickableTokens {
                        border: if selected {
                            palette.accent
                        } else {
                            iced::Color::TRANSPARENT
                        },
                        ..ClickableTokens::from_palette(&palette)
                    };
                    (
                        entry.id.clone(),
                        Box::new(move || {
                            let image = enz_image(entry.thumbnail.clone())
                                .radius(10.0)
                                .alt_text("")
                                .height(Length::Fixed(100.0))
                                .fit(EnzImageFit::Cover)
                                .placeholder_background(iced::Color::TRANSPARENT)
                                .into_element();
                            enz_clickable(
                                column![
                                    image,
                                    container(
                                        enz_text(&entry.title)
                                            .size_px(self.text_px(12.0))
                                            .single_line_elided()
                                            .into_element()
                                    )
                                    .padding(iced::Padding::ZERO.left(4).right(4)),
                                ]
                                .spacing(5),
                                message(Message::SelectGallery(generation, entry.id.clone())),
                            )
                            .id(format!("wallpaper-gallery-{}", entry.id).into())
                            .aria_label(tr!("shell-use-wallpaper", "arg0" => entry.title.clone()))
                            .tokens(tokens)
                            // Keep border width constant so selection changes cannot reflow the
                            // grid.
                            .padding(4)
                            .radius(14.0)
                            .border_width(2.0)
                            .width(Length::Fill)
                            // Keep tiles in Tab order during import; generation guards reject
                            // stale choices.
                            .enabled(!self.closing && self.picker_read.is_none())
                            .into_element()
                        }) as CellFn<'_, AppMessage>,
                    )
                })
                .collect();
            if cells.is_empty() {
                body = body.push(
                    enz_text(if self.wallpaper.gallery_loading {
                        tr("shell-loading-local-wallpapers")
                    } else {
                        tr("shell-add-png-jpeg-or-webp-images-to-pictures-wallpapers-then-refresh")
                    })
                    .muted_on(palette.background)
                    .into_element(),
                );
            } else {
                body = body.push(
                    enz_responsive_grid_keyed(cells)
                        .columns_compact(2)
                        .columns_narrow(2)
                        .columns_medium(3)
                        .columns_wide(3)
                        .into_element(),
                );
            }
            if self.wallpaper.gallery.truncated || self.wallpaper.gallery.skipped > 0 {
                body = body.push(
                    enz_text(tr!("shell-showing-up-to-48-wallpapers-unreadable-or-unsupported-images-skipped", "arg0" => format!("{:}", self.wallpaper.gallery.skipped)))
                    .size_px(self.text_px(12.0))
                    .muted_on(palette.background)
                    .into_element(),
                );
            }
            if let Some(error) = &self.wallpaper.gallery_error {
                body = body.push(enz_text(error).into_element());
            }
        }
        // Fit describes how an image is mapped onto the output; a solid colour
        // has no such choice, so the row is absent rather than inert.
        if kind != WallpaperKind::Color && kind != WallpaperKind::Live {
            body = body.push(
                self.choice_setting(
                    "wallpaper/fit",
                    p.fit.label(),
                    WallpaperFit::ALL
                        .into_iter()
                        .map(|fit| (fit.label().into(), message(Message::Fit(fit))))
                        .collect(),
                ),
            );
        }
        if let Some(error) = &self.wallpaper.status {
            body = body.push(enz_text(error).into_element()).push(
                enz_button(tr("shell-retry-background"), message(Message::Retry))
                    .kind(ButtonKind::Secondary)
                    .icon("refresh-cw")
                    .enabled(enabled)
                    .into_element(),
            );
        }
        body.into()
    }

    fn live_gallery(&self) -> Element<'_, AppMessage> {
        let palette = self.tokens().palette;
        let chosen = self.settings.wallpaper.scene.as_deref();
        let header = column![
            enz_text(tr("shell-live-wallpapers"))
                .size_px(self.text_px(16.0))
                .into_element(),
            enz_text(tr("shell-moving-scenes-rendered-on-the-gpu"))
                .size_px(self.text_px(12.0))
                .muted_on(palette.background)
                .into_element(),
        ]
        .spacing(2);
        let cells: Vec<(String, CellFn<'_, AppMessage>)> = self
            .wallpaper
            .live
            .iter()
            .map(|scene| {
                let selected = self.settings.wallpaper.kind == WallpaperKind::Live && chosen == Some(scene.id.as_str());
                let tokens = ClickableTokens {
                    border: if selected { palette.accent } else { iced::Color::TRANSPARENT },
                    ..ClickableTokens::from_palette(&palette)
                };
                let traits = match (scene.animated, scene.parallax) {
                    (true, true) => enzui_shell::i18n::tr("shell-animated-parallax"),
                    (true, false) => tr("shell-animated"),
                    (false, true) => enzui_shell::i18n::tr("shell-still-parallax"),
                    (false, false) => tr("shell-still"),
                };
                (
                    scene.id.clone(),
                    Box::new(move || {
                        let picture: Element<'_, AppMessage> = match &scene.preview {
                            Some(path) => enz_image(path.clone())
                                .radius(10.0)
                                .alt_text("")
                                .height(Length::Fixed(100.0))
                                .fit(EnzImageFit::Cover)
                                .placeholder_background(iced::Color::TRANSPARENT)
                                .into_element(),
                            None => container(Space::new()).height(100.0).width(Length::Fill).into(),
                        };
                        enz_clickable(
                            column![
                                picture,
                                container(
                                    column![
                                        enz_text(&scene.name).size_px(self.text_px(12.0)).single_line_elided().into_element(),
                                        enz_text(traits)
                                            .size_px(self.text_px(11.0))
                                            .muted_on(palette.surface)
                                            .single_line_elided()
                                            .into_element(),
                                    ]
                                    .spacing(1),
                                )
                                .padding(iced::Padding::ZERO.left(4).right(4)),
                            ]
                            .spacing(5),
                            message(Message::SelectLive(scene.id.clone())),
                        )
                        .id(format!("wallpaper-live-{}", scene.id).into())
                        .aria_label(tr!("shell-use-live-wallpaper", "arg0" => scene.name.clone(), "arg1" => scene.description.clone()))
                        .tokens(tokens)
                        .padding(4)
                        .radius(14.0)
                        .border_width(2.0)
                        .width(Length::Fill)
                        .enabled(!self.closing && !self.wallpaper.busy)
                        .into_element()
                    }) as CellFn<'_, AppMessage>,
                )
            })
            .collect();
        let mut body = column![header].spacing(10);
        if cells.is_empty() {
            body = body.push(
                enz_text(
                    match (&self.wallpaper.live_error, self.wallpaper.live_loading) {
                        (Some(error), _) => error.as_str(),
                        (None, true) => tr("shell-loading-live-wallpapers"),
                        (None, false) => {
                            tr("shell-no-live-wallpapers-are-installed-with-this-desktop")
                        }
                    },
                )
                .muted_on(palette.background)
                .into_element(),
            );
        } else {
            body = body.push(
                enz_responsive_grid_keyed(cells)
                    .columns_compact(2)
                    .columns_narrow(2)
                    .columns_medium(3)
                    .columns_wide(3)
                    .into_element(),
            );
        }
        body.into()
    }
}
