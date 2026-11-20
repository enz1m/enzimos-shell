use crate::{Message, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_widgets::{clickable::enz_clickable, text::enz_text};
use iced::{
    Element, Length,
    widget::{column, container, mouse_area, row, scrollable},
};

pub(crate) fn indicator(shell: &Shell) -> Element<'_, Message> {
    let p = shell.window_tokens(shell.overview).palette;
    let marks: Element<'_, Message> = if let Some(desktops) = &shell.desktops {
        enzui_shell::presentation::desktop_marks(
            desktops
                .entries
                .iter()
                .map(|d| (d.id, d.id == desktops.active))
                .collect(),
            desktops
                .entries
                .iter()
                .filter(|d| d.placeholder)
                .map(|d| d.id)
                .collect(),
            &shell.window_tokens(shell.overview).iced_theme(),
            p.text,
        )
    } else {
        enz_text("…")
            .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
            .into_element()
    };
    let label = shell
        .desktops
        .as_ref().map_or_else(|| tr("shell-desktops-loading").into(), |d| {
            // The empty desktop kept ready at the end is not counted.
            let count = d.entries.iter().filter(|e| !e.placeholder || e.id == d.active).count();
            tr!("shell-desktops-of-active", "arg0" => format!("{:}", d.active_index() + 1), "arg1" => format!("{:}", count.max(1)))
        });
    // The marks are compact; hovering names them ("Desktop 1 of 2").
    let trigger = enzui_shell::bar_item::trigger(
        enz_clickable(marks, Message::ToggleOverview),
        shell.metrics,
    )
    .aria_label(label.clone())
    .id("desktop-overview".into())
    .enabled(shell.desktops.is_some())
    .into_element();
    mouse_area(enzui_widgets::tooltip::enz_tooltip(
        trigger,
        label,
        if shell.bar == enzui_shell::settings::BarEdge::Bottom {
            iced::widget::tooltip::Position::Top
        } else {
            iced::widget::tooltip::Position::Bottom
        },
    ))
    .on_scroll(Message::DesktopWheel)
    // A floating desktop's windows side by side.
    .on_right_press(Message::Spread(crate::window_spread::SpreadMessage::Toggle))
    .into()
}

fn preview_size(image: Option<&iced::widget::image::Handle>) -> iced::Size {
    let aspect = match image {
        Some(iced::widget::image::Handle::Rgba { width, height, .. })
            if *width > 0 && *height > 0 =>
        {
            *width as f32 / *height as f32
        }
        _ => 1.6,
    };
    let width = 236.0_f32.min(160.0 * aspect);
    iced::Size::new(width, width / aspect)
}

fn body_padding(island: bool, edges: (u32, u32)) -> iced::Padding {
    let (left, right) = if island { (0, 0) } else { edges };
    iced::Padding {
        top: 12.0,
        bottom: 12.0,
        left: left as f32 + 12.0,
        right: right as f32 + 12.0,
    }
}

pub(crate) fn overview(shell: &Shell) -> Element<'_, Message> {
    let p = shell.window_tokens(shell.overview).palette;
    let mut cards = row![].spacing(12).align_y(iced::Alignment::Center);
    if let Some(desktops) = &shell.overview_desktops {
        for (index, desktop) in desktops.entries.iter().enumerate() {
            let active = desktop.id == desktops.active;
            let preview = shell
                .overview_previews
                .iter()
                .find(|preview| preview.desktop == desktop.id);
            let mut size = preview_size(preview.map(|preview| &preview.image));
            // The trailing empty desktop is where a new one starts.
            let spare = desktop.windows == 0 && index + 1 == desktops.entries.len() && index > 0;
            // Adding a desktop is a small trailing action, not a desktop.
            if spare {
                size.width = (size.width * 0.45).max(64.0);
            }
            let picture: Element<'_, Message> = if spare {
                container(
                    enzui_widgets::enz_icon("plus")
                        .size(24.0)
                        .tint(p.text_muted)
                        .into_element(),
                )
                .center_x(size.width)
                .center_y(size.height)
                // A faint wash of the ink with an ink hairline: the surface
                // colour at low alpha became a bright slab on dark glass and
                // vanished on light glass.
                .style(move |_| container::Style {
                    background: Some(iced::Color { a: 0.05, ..p.text }.into()),
                    border: iced::Border {
                        radius: 10.0.into(),
                        width: 1.0,
                        color: iced::Color { a: 0.18, ..p.text },
                    },
                    ..Default::default()
                })
                .into()
            } else if let Some(preview) = preview {
                enzui_widgets::image::enz_image_handle(preview.image.clone())
                    .fit(enzui_widgets::image::EnzImageFit::Contain)
                    .width(Length::Fixed(size.width))
                    .height(Length::Fixed(size.height))
                    .radius(10.0)
                    .into_element()
            } else {
                container(enzui_widgets::enz_icon("monitor").size(28.0).into_element())
                    .center_x(size.width)
                    .center_y(size.height)
                    .style(move |_| container::Style {
                        background: Some(p.surface_alt.into()),
                        border: iced::Border {
                            radius: 10.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .into()
            };
            let content = picture;
            let mut tokens = enzui_widgets::clickable::ClickableTokens::from_palette(&p);
            tokens.background = iced::Color::TRANSPARENT;
            tokens.border = if active {
                p.accent
            } else {
                iced::Color::TRANSPARENT
            };
            let card = enz_clickable(content, Message::SelectDesktop(desktop.id))
                    .aria_label(tr!("workspace-caption", "number" => index + 1, "windows" => desktop.windows, "current" => if active { "yes" } else { "no" }))
                    .description(if preview.is_none() {
                        tr("shell-preview-unavailable")
                    } else if preview.is_some_and(|p| p.unavailable > 0) {
                        tr("shell-preview-incomplete")
                    } else {
                        tr("shell-select-desktop")
                    })
                    .id(format!("desktop-card-{}", desktop.id).into())
                    .tokens(tokens)
                    .width(size.width + 4.0)
                    .height(size.height + 4.0)
                    .padding(2)
                    .radius(12.0)
                    .border_width(2.0)
                    .into_element();
            // The picture and its accent ring say which desktop it is; a name
            // under it only repeated its position.
            cards = cards.push(card);
        }
    }
    let island = matches!(
        shell.overview_attachment,
        enzui_shell::reveal::Attachment::Island { .. }
    );
    let join = shell
        .overview_attachment
        .join(f32::from(shell.metrics.radius));
    let body = mouse_area(
        container(
            scrollable(container(cards).center_x(Length::Fill))
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(4).scroller_width(4),
                ))
                .width(Length::Fill)
                .height(if island { Length::Shrink } else { Length::Fill }),
        )
        .padding(body_padding(island, shell.desktop_edges()))
        .align_y(iced::alignment::Vertical::Center)
        .width(Length::Fill)
        .height((shell.overview_size.height - join).max(1.0)),
    )
    .on_press(Message::RegionInterior);
    let bottom = shell.bar == enzui_shell::settings::BarEdge::Bottom;
    if join == 0.0 {
        return body.into();
    }
    let corners = iced::widget::space().width(Length::Fill).height(join);
    if bottom != island {
        column![corners, body].into()
    } else {
        column![body, corners].into()
    }
}
