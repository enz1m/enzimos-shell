use enzui_shell::i18n::tr;
use enzui_shell::tr;
mod icon;
mod island;

use crate::{Message, Shell};
use enzui_shell::{
    settings::SidebarEdge,
    sidebar::{Action, Snapshot},
};
use enzui_widgets::{motion::SlideDirection, toolbar_icon::enz_toolbar_icon};
use iced::{
    Element, Length,
    widget::{column, container, mouse_area, scrollable, space},
};

pub(crate) fn rail<'a>(shell: &'a Shell, state: &'a Snapshot) -> Element<'a, Message> {
    if !shell.rail_paintable() {
        return iced::widget::space().into();
    }
    let p = shell.window_tokens(shell.sidebar_rail).palette;
    let shared_edge = shell.dock_shares_sidebar();
    let dropping = shell.metrics.islands.is_some() && state.reveal;
    let hovered = dropping && state.drop_hover;
    let mut icon_tokens = shell.window_icon_tokens(shell.sidebar_rail);
    icon_tokens.slot_height = f32::from(shell.metrics.rail_slot());
    icon_tokens.button_size = icon_tokens.slot_height - 4.0;
    if hovered && (state.canvas.is_none() || state.entries.is_empty()) {
        icon_tokens.icon_color = p.on_accent();
        icon_tokens.hover_color = p.on_accent();
        icon_tokens.pressed_color = p.on_accent();
        icon_tokens.active_color = p.on_accent();
    }
    let mut icons = column![]
        .spacing(6)
        .width(Length::Fill)
        .align_x(iced::Alignment::Center);
    for (index, entry) in state.entries.iter().enumerate() {
        let icon = entry.app.map_or("app-window", |app| app.icon);
        let name = entry
            .app
            .map_or_else(|| tr("shell-application"), |app| app.display_name());
        let shown = state.shown == Some(entry.id);
        let label = tr!("shell-miniapp", "arg0" => (if shown { enzui_shell::i18n::tr("shell-hide") } else { enzui_shell::i18n::tr("shell-show") }).to_owned(), "arg1" => name.to_owned());
        let press = Message::SidebarAction(Action::Toggle(
            entry.id,
            state.icon_center(
                index,
                shell.metrics.rail_slot(),
                shell.sidebar_scroll,
                shell.metrics.islands.is_none() && shared_edge,
            ),
        ));
        let glyph = match &entry.image {
            Some(path) => {
                // The icon follows the rail's width. On the frame the rail is a
                // strip of the screen edge and an app's icon fills it like a
                // slim dock's; on a floating island it keeps more room around.
                let room = if shell.metrics.islands.is_some() {
                    10.0
                } else {
                    6.0
                };
                let glyph = (icon_tokens.slot_height - room).max(icon_tokens.icon_size + 6.0);
                app_icon_button(
                    path,
                    press,
                    label,
                    entry.id,
                    entry.available,
                    &icon_tokens,
                    glyph,
                )
            }
            None => enz_toolbar_icon(icon, press)
                .tokens(icon_tokens)
                .aria_label(label)
                .id(format!("miniapp-{}", entry.id).into())
                .enabled(entry.available)
                .into_element(),
        };
        let button: Element<'a, Message> = mouse_area(glyph)
            .on_right_press(Message::SidebarAction(Action::Unpin(entry.id)))
            .into();
        icons = icons.push(if state.icon_drag {
            let source = state.source_rect();
            Element::new(icon::Icon {
                child: button,
                id: entry.id,
                owner: std::process::id(),
                origin: iced::Point::new(source.x as f32, source.y as f32),
                canvas: iced::Size::new(source.width as f32, source.height as f32),
                scroll: shell.sidebar_scroll,
                enabled: entry.available,
                light: shell.desired_settings.light,
            })
        } else {
            button
        });
    }
    if !state.error.is_empty() {
        icons = icons.push(
            enz_toolbar_icon("triangle-alert", Message::SidebarAction(Action::ClearError))
                .tokens(icon_tokens)
                .aria_label(state.notice())
                .id("miniapp-notice".into())
                .into_element(),
        );
    } else if state.entries.is_empty() && state.canvas.is_none() {
        icons = icons.push(
            enz_toolbar_icon(
                if hovered { "check" } else { "pin" },
                Message::RegionInterior,
            )
            .tokens(icon_tokens)
            .aria_label(if hovered {
                tr("shell-release-to-pin-application")
            } else {
                tr("shell-drag-application-to-this-screen-edge-to-pin")
            })
            .into_element(),
        );
    }
    if state.canvas.is_some() {
        let mut marker_tokens = shell.window_icon_tokens(shell.sidebar_rail);
        marker_tokens.icon_color = if hovered { p.on_accent() } else { p.accent };
        let marker = enz_toolbar_icon(
            if state.entries.is_empty() {
                if hovered { "check" } else { "pin" }
            } else {
                "plus"
            },
            Message::RegionInterior,
        )
        .tokens(marker_tokens)
        .aria_label(tr("shell-release-to-pin-another-application"))
        .into_element();
        let marker = container(marker)
            .style(move |_| container::Style {
                background: if state.entries.is_empty() {
                    None
                } else {
                    Some(if hovered { p.accent } else { p.accent_soft }.into())
                },
                border: iced::Border {
                    radius: (f32::from(shell.metrics.rail) / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into();
        // While the shell changes style the rail fades in the compositor
        // (`Shell::rail_alpha_tick`); its glass, which that fade cannot
        // reach, leaves with it instead of staying behind.
        let shown = shell.chrome_shown();
        let rail = Element::new(island::Rail {
            children: [
                scrollable(icons)
                    .on_scroll(|scroll| Message::SidebarScroll(scroll.viewport.absolute_offset().y))
                    .width(Length::Fill)
                    .height(Length::Shrink)
                    .into(),
                marker,
            ],
            count: state.entries.len() + usize::from(!state.error.is_empty()),
            slot: f32::from(shell.metrics.rail_slot()),
            scroll: shell.sidebar_scroll,
            target_height: state.rail.height as f32,
            top_aligned: shared_edge,
            radius: shell
                .metrics
                .islands
                .filter(|style| {
                    shared_edge && f32::from(style.radius) < f32::from(style.height) / 2.0
                })
                .map(|style| f32::from(style.radius)),
            outline: shared_edge.then_some(p.border),
            show_marker: state.drop_add,
            interactive: state.visible()
                && !shell.sidebar_input_failed
                && shell
                    .sidebar_rail
                    .is_some_and(|id| shell.surface_inputs.contains_key(&id)),
            color: if state.entries.is_empty() && hovered {
                p.accent
            } else if state.entries.is_empty() && dropping {
                p.accent_soft
            } else {
                // The theme's surface, like the bar's islands: the adaptive
                // ink turned the rail light over a dark wallpaper in dark mode.
                shell.tokens().palette.surface
            },
            policy: shell.tokens().motion,
            input: shell
                .sidebar_rail
                .and_then(|id| shell.surface_inputs.get(&id))
                .cloned(),
            // The compositor's glass cannot fade with the content: it leaves
            // with the rail instead of staying behind it.
            glass: shell.chrome_glass().filter(|_| shown).map(|style| {
                enzui_shell::reveal::glass_tokens(style, shell.tokens().motion.reduced_motion)
            }),
        });
        return rail;
    }
    let content = container(
        scrollable(icons)
            .on_scroll(|scroll| Message::SidebarScroll(scroll.viewport.absolute_offset().y))
            .width(Length::Fill)
            .height(Length::Shrink),
    )
    .padding([8, 2])
    .width(u32::from(shell.metrics.rail))
    .height(Length::Fill)
    .align_y(if shared_edge {
        iced::alignment::Vertical::Top
    } else {
        iced::alignment::Vertical::Center
    });
    let content = container(content)
        .padding(iced::Padding {
            left: if state.side == SidebarEdge::Right && shell.metrics.islands.is_none() {
                f32::from(shell.metrics.radius)
            } else {
                0.0
            },
            right: if state.side == SidebarEdge::Left && shell.metrics.islands.is_none() {
                f32::from(shell.metrics.radius)
            } else {
                0.0
            },
            ..Default::default()
        })
        .width(Length::Fill)
        .height(Length::Fill);
    let direction = if state.side == SidebarEdge::Left {
        SlideDirection::Left
    } else {
        SlideDirection::Right
    };
    let reveal = enzui_shell::reveal::Reveal {
        shape_input: false,
        shape: if shell.metrics.islands.is_some() {
            enzui_shell::reveal::Shape::FloatingRail {
                right: state.side == SidebarEdge::Right,
            }
        } else {
            enzui_shell::reveal::Shape::Rail {
                right: state.side == SidebarEdge::Right,
                frame: f32::from(shell.metrics.frame),
            }
        },
        radius: if shell.metrics.islands.is_some() {
            f32::from(shell.metrics.rail) / 2.0
        } else {
            f32::from(shell.metrics.radius)
        },
        color: if hovered {
            p.accent
        } else if dropping {
            p.accent_soft
        } else {
            p.surface
        },
    };
    let rail = if state.native_rail() {
        enzui_shell::presentation::attached_source(
            content.into(),
            state.visible(),
            direction,
            &shell.tokens().iced_theme(),
            reveal,
        )
    } else {
        enzui_shell::presentation::attached(
            content.into(),
            state.visible(),
            direction,
            &shell.tokens().iced_theme(),
            None,
            reveal,
        )
    };
    // While the shell changes style the rail fades in the compositor, with
    // the frame (`Shell::rail_alpha_tick`).
    rail
}

pub(crate) fn pane<'a>(shell: &'a Shell, state: &'a Snapshot) -> Element<'a, Message> {
    let p = shell.tokens().palette;
    let left = state.side == SidebarEdge::Left;
    let radius = state.pane_radius(shell.desired_settings.compositor.rounding());
    let content = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(p.surface.into()),
            border: iced::Border {
                radius: iced::border::Radius {
                    top_left: if left && shell.metrics.islands.is_none() {
                        0.0
                    } else {
                        radius
                    },
                    bottom_left: if left && shell.metrics.islands.is_none() {
                        0.0
                    } else {
                        radius
                    },
                    top_right: if left || shell.metrics.islands.is_some() {
                        radius
                    } else {
                        0.0
                    },
                    bottom_right: if left || shell.metrics.islands.is_some() {
                        radius
                    } else {
                        0.0
                    },
                },
                ..Default::default()
            },
            ..Default::default()
        });
    if state.native {
        return content.into();
    }
    container(enzui_shell::presentation::slide(
        content.into(),
        state.pane_visible(),
        if left {
            SlideDirection::Left
        } else {
            SlideDirection::Right
        },
        &shell.tokens().iced_theme(),
        None,
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .clip(true)
    .into()
}

// A rail button showing the application's own icon, in the toolbar icon's
// geometry and states (the toolbar icon takes only bundled names).
fn app_icon_button<'a>(
    path: &std::path::Path,
    press: Message,
    label: String,
    id: u32,
    enabled: bool,
    tokens: &enzui_widgets::toolbar_icon::ToolbarIconTokens,
    glyph: f32,
) -> Element<'a, Message> {
    use enzui_widgets::focus_button::{FocusStatus, focus_button_maybe};
    let size = tokens.button_size.max(glyph + 4.0);
    let hover = tokens.hover_background;
    let pressed = tokens.active_background;
    let ring = tokens.active_color;
    let icon: Element<'a, Message> =
        container(enzui_shell::icon_theme::source(path).into_element(glyph))
            .center_x(size)
            .center_y(size)
            .into();
    let button: Element<'a, Message> = focus_button_maybe(icon, enabled.then_some(press))
        .id(format!("miniapp-{id}").into())
        .semantics(
            enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::Button).label(label),
        )
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .padding(0)
        .style_focus(move |_, status| iced::widget::button::Style {
            background: match status {
                FocusStatus::Hovered => Some(hover.into()),
                FocusStatus::Pressed => Some(pressed.into()),
                _ => None,
            },
            border: iced::Border {
                color: ring,
                width: if status == FocusStatus::Focused {
                    2.0
                } else {
                    0.0
                },
                radius: (size / 2.0).into(),
            },
            ..Default::default()
        })
        .into();
    container(button)
        .center_x(Length::Fill)
        .height(tokens.slot_height)
        .center_y(tokens.slot_height)
        .into()
}
