use super::{
    COMPOSER, Message, Screenshots, TRANSCRIPT, attachments, message, resize,
    transcript::{Body, Decision, Row, ToolStatus},
};
use crate::{Message as ShellMessage, Shell, region};
use enzui_shell::tr;
use enzui_shell::{agent_harness::ApprovalMode, i18n::tr};
use enzui_theme::Palette;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    code_block::enz_code_block,
    composer::enz_composer,
    file_drop::enz_file_drop_target_with_receipt,
    frame_clock::{enz_frame_clock, frame_clock_phase},
    icon::enz_icon,
    markdown::{self, enz_markdown_blocks},
    motion::{MotionEffect, SlideDirection, motion},
    pressable::{PressableStyle, enz_pressable},
    spinner::enz_spinner,
    tail_follow::enz_tail_follow,
    text::enz_text,
    text_field::enz_text_field,
    toolbar_icon::enz_toolbar_icon,
    virtual_list::enz_virtual_list_with_geometry,
};
use iced::{
    Alignment, Color, Element, Length, Padding,
    font::Weight,
    widget::{column, container, mouse_area, row, space, stack},
};

const BODY: f32 = 13.0;
const SMALL: f32 = 12.0;
const TINY: f32 = 11.0;

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn tool_icon(tool: &str) -> &'static str {
    match tool {
        "run_command" => "terminal",
        "file_read" | "read_file" => "eye",
        "file_write" | "write_file" => "file-pen",
        "files_list" | "search_files" => "folder-search",
        "screenshot" => "camera",
        "render_chart" => "chart-column",
        "web_search" => "globe",
        _ => "wrench",
    }
}

fn seconds(millis: u64) -> String {
    if millis < 1000 {
        tr!("shell-ms", "arg0" => format!("{:}", millis))
    } else {
        format!("{:.1} s", millis as f32 / 1000.0)
    }
}

fn ago(updated: i64) -> String {
    let minutes = (chrono::Utc::now().timestamp() - updated).max(0) / 60;
    match minutes {
        0 => enzui_shell::i18n::tr("shell-now").into(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

impl Shell {
    pub(crate) fn assistant_view(&self, id: iced::window::Id) -> Option<Element<'_, ShellMessage>> {
        if self.assistant.owns_trigger(id) {
            return Some(self.assistant_trigger(id));
        }
        let pane = self
            .assistant
            .panel
            .as_ref()
            .filter(|pane| pane.window == id)?;
        let corner = self.desired_settings.assistant_corner;
        let mut popup = self.popup(
            id,
            pane.visible && !self.assistant.attachment_loader.choosing(),
            pane.geometry.attachment,
            message(Message::Hidden(id)),
        );
        popup.bottom = corner.bottom();
        let panel = enz_file_drop_target_with_receipt(self.assistant_panel(), |drop| {
            message(Message::Attachments(attachments::Message::Drop(drop)))
        })
        .enabled(pane.visible && self.assistant.attachments_available())
        .copy_only(true)
        .key((self.assistant.session.as_deref(), self.assistant.history));
        Some(region::popup(panel.into(), popup))
    }

    fn assistant_trigger(&self, id: iced::window::Id) -> Element<'_, ShellMessage> {
        let Some(trigger) = &self.assistant.trigger else {
            return space().into();
        };
        // A new tab window draws only once it is in its corner: it showed at
        // full strength by the screen's edge for a few frames (a style change
        // rebuilds it), and while the shell changes style it stays hidden.
        if !trigger.placed || !self.chrome_shown() {
            return space().into();
        }
        let layout = &trigger.layout;
        let shown = self.assistant.shown && self.assistant.panel.is_none();
        if let Some(input) = self.surface_inputs.get(&id) {
            if shown {
                let mut regions = layout.strips.clone();
                regions.push(layout.shown);
                input.rectangles(&regions);
            } else if self.assistant.panel.is_some() {
                input.rectangles(&[]);
            } else {
                input.rectangles(&layout.strips);
            }
        }
        let tokens = self.tokens();
        let palette = tokens.palette.clone();
        // On glass and on the frame's tab the mark carries the accent; the
        // solid floating button is the accent itself, with the mark in the
        // ink that reads on it.
        let on_glass = self.chrome_glass().is_some();
        let filled = !layout.joined && !on_glass;
        let accent = palette.accent;
        let luminance = 0.2126 * accent.r + 0.7152 * accent.g + 0.0722 * accent.b;
        let icon = enz_icon("ai.assistant")
            .size(if layout.joined { 18.0 } else { 20.0 })
            .tint(if !filled {
                accent
            } else if luminance > 0.6 {
                Color::from_rgb(0.07, 0.08, 0.1)
            } else {
                Color::WHITE
            })
            .into_element();
        // The tab and the round button are their own shape; a hover tint
        // would draw a rectangle over them.
        let quiet = enzui_widgets::pressable::PressableTokens {
            hover_overlay: Color::TRANSPARENT,
            pressed_overlay: Color::TRANSPARENT,
            ..enzui_widgets::pressable::PressableTokens::from_palette(&palette)
        };
        let button: Element<'_, ShellMessage> = if layout.joined {
            // Frame: a tab growing out of the frame, one surface with it.
            let tab = enz_pressable(
                container(icon)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .into(),
                message(Message::Toggle),
            )
            .style(PressableStyle::Plain)
            .tokens(quiet)
            .id("assistant-open".into())
            .aria_label(tr("shell-open-the-assistant"))
            .width(Length::Fixed(enzui_shell::compositor::FRAME_TAB.width))
            .into_element();
            let reveal = enzui_shell::reveal::Reveal::popup(
                layout.corner.bottom(),
                enzui_shell::reveal::Attachment::Bar,
                f32::from(self.metrics.radius),
                palette.surface,
                enzui_shell::reveal::PanelShadow::NONE,
            );
            // The tab's body sits between its fillets; the reveal keeps a
            // radius of room on the side away from the join.
            let radius = f32::from(self.metrics.radius);
            let placed = container(
                container(tab).height(Length::Fixed(enzui_shell::compositor::FRAME_TAB.height)),
            )
            .padding(Padding {
                left: radius,
                top: if layout.corner.bottom() { radius } else { 0.0 },
                ..Padding::ZERO
            })
            .width(Length::Fixed(layout.button.width))
            .height(Length::Fixed(layout.button.height));
            enzui_shell::presentation::attached_fitted(
                container(placed)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                shown,
                if layout.corner.bottom() {
                    SlideDirection::Down
                } else {
                    SlideDirection::Up
                },
                &tokens.iced_theme(),
                None,
                reveal,
                None,
                None,
                None,
                None,
            )
        } else {
            // Islands: a floating round button that slides in from the edge.
            let shadow = self.panel_shadow();
            let lit = self.light;
            let size = layout.button.width;
            let circle = enz_pressable(
                container(icon)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .into(),
                message(Message::Toggle),
            )
            .style(PressableStyle::Plain)
            .tokens(quiet)
            .id("assistant-open".into())
            .aria_label(tr("shell-open-the-assistant"))
            .width(Length::Fixed(size))
            .min_height(size)
            .min_height_center()
            .into_element();
            let surface = self.glass_tint(id, palette.surface);
            // Shown, the button is glass when Settings asks for glass panels
            // and the compositor paints it; its own body then steps aside.
            let glass = match (self.surface_inputs.get(&id), self.chrome_glass()) {
                (Some(input), Some(style)) if shown => {
                    let bounds = iced::Rectangle {
                        x: layout.button.x,
                        y: layout.button.y,
                        width: size,
                        height: size,
                    };
                    input.glass(
                        &[(bounds, size / 2.0)],
                        &enzui_shell::reveal::glass_tokens(style, tokens.motion.reduced_motion),
                        surface,
                        enzui_shell::reveal::glass_shadow(shadow.level, surface, false),
                    )
                }
                (Some(input), _) => {
                    input.clear_glass();
                    false
                }
                _ => false,
            };
            let body = container(circle)
                .width(Length::Fixed(size))
                .height(Length::Fixed(size))
                .style(move |_| container::Style {
                    // Solid: one accent disc, the desktop's single coloured
                    // control, with no ring around it. Glass keeps the bar
                    // islands' hairline rim over the compositor's body.
                    background: if glass {
                        None
                    } else if on_glass {
                        Some(surface.into())
                    } else {
                        Some(accent.into())
                    },
                    border: iced::Border {
                        radius: (size / 2.0).into(),
                        width: if on_glass { 1.0 } else { 0.0 },
                        color: if lit {
                            Color::from_rgba(0.0, 0.0, 0.0, 0.08)
                        } else {
                            Color::from_rgba(1.0, 1.0, 1.0, 0.06)
                        },
                    },
                    shadow: if glass {
                        iced::Shadow::default()
                    } else {
                        iced::Shadow {
                            color: Color::from_rgba(
                                0.0,
                                0.0,
                                0.0,
                                if shadow.level == 0 {
                                    0.0
                                } else if lit {
                                    0.18
                                } else {
                                    0.34
                                },
                            ),
                            offset: iced::Vector::new(0.0, 3.0),
                            blur_radius: 12.0,
                        }
                    },
                    ..Default::default()
                });
            // It grows into place where it floats: a slide would start at its
            // window's edge, inside the screen, and show a cut.
            let shown_body = motion(
                motion(body, shown, MotionEffect::Scale),
                shown,
                MotionEffect::Fade,
            );
            let placed = container(shown_body).padding(Padding {
                left: layout.button.x,
                top: layout.button.y,
                ..Padding::ZERO
            });
            return mouse_area(placed.width(Length::Fill).height(Length::Fill))
                .on_enter(message(Message::Hover(true)))
                .on_exit(message(Message::Hover(false)))
                .into();
        };
        let placed = container(
            container(button)
                .width(Length::Fixed(layout.button.width))
                .height(Length::Fixed(layout.button.height)),
        )
        .padding(Padding {
            left: layout.button.x,
            top: layout.button.y,
            ..Padding::ZERO
        })
        .width(Length::Fill)
        .height(Length::Fill);
        mouse_area(placed)
            .on_enter(message(Message::Hover(true)))
            .on_exit(message(Message::Hover(false)))
            .into()
    }

    fn assistant_panel(&self) -> Element<'_, ShellMessage> {
        let palette = self.tokens().palette;
        let state = &self.assistant;
        let header = self.assistant_header();
        let body: Element<'_, ShellMessage> = if state.shortcuts {
            self.assistant_shortcuts()
        } else if state.history {
            self.assistant_history(&palette)
        } else if state.transcript.is_empty() && state.running.is_none() {
            self.assistant_empty(&palette)
        } else {
            self.assistant_transcript(&palette)
        };
        let body = motion(body, true, MotionEffect::Fade);
        let mut content = column![header, container(body).height(Length::Fill)].spacing(4);
        if !state.history && !state.shortcuts {
            if let Some(error) = &state.error {
                content = content.push(
                    container(
                        row![
                            enz_icon("circle-alert")
                                .size(14.0)
                                .tint(palette.danger)
                                .into_element(),
                            enz_text(error.clone())
                                .size_px(SMALL)
                                .wrap(true)
                                .into_element(),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .padding(Padding {
                        left: 4.0,
                        right: 4.0,
                        ..Padding::ZERO
                    }),
                );
            }
            if state.export_failed {
                if let Some(id) = &state.session {
                    content = content.push(
                        enz_button(tr("shell-try-again"), message(Message::Open(id.clone())))
                            .id("assistant-reload-chat".into())
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .enabled(!state.approval_busy)
                            .into_element(),
                    );
                }
            } else {
                if state.choosing_approval {
                    content = content.push(self.assistant_approvals(&palette));
                }
                content = content.push(self.assistant_composer(&palette));
            }
        }
        // The body leaves room for the fillets beside a join and a radius
        // on the side away from it.
        let radius = f32::from(self.metrics.radius);
        let attachment = state
            .panel
            .as_ref()
            .map(|pane| pane.geometry.attachment)
            .unwrap_or_default();
        let (left, right) = attachment.insets(radius);
        let far = attachment.join(radius);
        let bottom = self.desired_settings.assistant_corner.bottom();
        let content = container(content)
            .padding(Padding {
                top: 10.0 + if bottom { far } else { 0.0 },
                right: 12.0 + right,
                bottom: 12.0 + if bottom { 0.0 } else { far },
                left: 12.0 + left,
            })
            .width(Length::Fill)
            .height(Length::Fill);
        let right_side = self.desired_settings.assistant_corner.right();
        let grip = mouse_area(
            container(
                container(space().width(2).height(26)).style(move |_| container::Style {
                    background: Some(mix(palette.surface, palette.text, 0.23).into()),
                    border: iced::Border {
                        radius: 1.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            )
            .center_x(10)
            .center_y(Length::Fill),
        )
        .on_press_at(|position| message(Message::Resize(resize::Message::Start(position.x))))
        .interaction(iced::mouse::Interaction::ResizingHorizontally);
        let grip = container(grip).width(Length::Fill).height(Length::Fill);
        let grip = if right_side {
            grip.align_left(Length::Fill)
        } else {
            grip.align_right(Length::Fill)
        };
        let mut layers = stack![content, grip];
        if let Some(width) = state.resize.preview(right_side) {
            layers = layers.push(
                container(
                    enz_text(format!("{width} px"))
                        .size_px(SMALL)
                        .into_element(),
                )
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            );
        }
        layers.into()
    }

    fn assistant_header(&self) -> Element<'_, ShellMessage> {
        let state = &self.assistant;
        let title = if state.shortcuts {
            tr("shell-assistant-shortcuts").to_owned()
        } else if state.history {
            tr("shell-chats").to_owned()
        } else if state.title.is_empty() {
            tr("shell-new-chat").to_owned()
        } else {
            state.title.clone()
        };
        let leading = if state.shortcuts {
            enz_toolbar_icon("arrow-left", message(Message::Shortcuts(false)))
                .tokens(self.icon_tokens())
                .aria_label(tr("shell-back-to-the-chat"))
                .into_element()
        } else if state.history {
            enz_toolbar_icon("arrow-left", message(Message::History(false)))
                .tokens(self.icon_tokens())
                .id("assistant-back".into())
                .aria_label(tr("shell-back-to-the-chat"))
                .into_element()
        } else {
            enz_toolbar_icon("history", message(Message::History(true)))
                .tokens(self.icon_tokens())
                .id("assistant-history".into())
                .aria_label(tr("shell-chat-history"))
                .tooltip(tr("shell-chat-history"))
                .into_element()
        };
        row![
            leading,
            enz_text(title)
                .size_px(BODY + 1.0)
                .weight(Weight::Semibold)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .into_element(),
            space().width(Length::Fill),
            if !state.history && state.approval_mode == ApprovalMode::FullSend {
                enz_text(ApprovalMode::FullSend.label())
                    .size_px(SMALL)
                    .weight(Weight::Semibold)
                    .color(self.tokens().palette.danger)
                    .into_element()
            } else {
                space().width(Length::Shrink).into()
            },
            enz_toolbar_icon("keyboard", message(Message::Shortcuts(!state.shortcuts)))
                .tokens(self.icon_tokens())
                .id("assistant-shortcuts".into())
                .aria_label(tr("shell-assistant-shortcuts"))
                .tooltip(tr("shell-assistant-shortcuts"))
                .into_element(),
            enz_toolbar_icon("square-pen", message(Message::NewChat))
                .tokens(self.icon_tokens())
                .id("assistant-new".into())
                .aria_label(tr("shell-new-chat"))
                .tooltip(tr("shell-new-chat"))
                .enabled(state.running.is_none() && !state.approval_busy)
                .into_element(),
            enz_toolbar_icon("x", message(Message::Close))
                .tokens(self.icon_tokens())
                .id("assistant-close".into())
                .aria_label(tr("shell-close-the-assistant"))
                .into_element(),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .height(Length::Fixed(34.0))
        .into()
    }

    fn assistant_shortcuts(&self) -> Element<'_, ShellMessage> {
        let mut content = column![].spacing(10);
        for key in [
            "shell-assistant-shortcut-send",
            "shell-assistant-shortcut-newline",
            "shell-assistant-shortcut-close",
            "shell-assistant-shortcut-tab",
            "shell-assistant-resize",
        ] {
            content = content.push(enz_text(tr(key)).size_px(BODY).into_element());
        }
        content
            .push(
                row![
                    enz_button(
                        tr("shell-assistant-narrow"),
                        message(Message::Resize(resize::Message::Step(false)))
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .into_element(),
                    enz_button(
                        tr("shell-assistant-widen"),
                        message(Message::Resize(resize::Message::Step(true)))
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .into_element(),
                ]
                .spacing(6),
            )
            .padding(12)
            .into()
    }

    fn assistant_empty(&self, palette: &Palette) -> Element<'_, ShellMessage> {
        let chip = |label: &'static str, prompt: &'static str, icon: &'static str| {
            enz_pressable(
                row![
                    enz_icon(icon).size(14.0).into_element(),
                    enz_text(label).size_px(SMALL).into_element()
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
                message(Message::Suggest(prompt)),
            )
            .id(format!("assistant-suggest-{icon}").into())
            .style(PressableStyle::Chip)
            .into_element()
        };
        let muted = palette.text_muted;
        let service = match &self.assistant.service {
            None => Some(tr("shell-starting-the-assistant").to_owned()),
            Some(Err(error)) => Some(error.clone()),
            Some(Ok(())) => None,
        };
        let mut body = column![
            enz_icon("ai.assistant")
                .size(28.0)
                .tint(palette.accent)
                .into_element(),
            enz_text(tr("shell-how-can-i-help"))
                .size_px(enzui_theme::text_scale::effective().apply_px(16.0))
                .weight(Weight::Semibold)
                .into_element(),
            enz_text(tr(
                "shell-ask-a-question-or-let-me-look-something-up-on-this-computer"
            ))
            .size_px(SMALL)
            .color(muted)
            .wrap(true)
            .into_element(),
        ]
        .spacing(8)
        .align_x(Alignment::Center);
        if let Some(service) = service {
            body = body.push(enz_text(service).size_px(SMALL).color(muted).into_element());
        }
        let chips = column![
            row![
                chip(
                    tr("shell-suggest-files"),
                    tr("shell-suggest-files-prompt"),
                    "folder-search"
                ),
                chip(
                    tr("shell-suggest-summary"),
                    tr("shell-suggest-summary-prompt"),
                    "file-text"
                )
            ]
            .spacing(6),
            row![
                chip(
                    tr("shell-suggest-email"),
                    tr("shell-suggest-email-prompt"),
                    "mail"
                ),
                chip(
                    tr("shell-suggest-explain"),
                    tr("shell-suggest-explain-prompt"),
                    "lightbulb"
                )
            ]
            .spacing(6),
            row![
                chip(
                    tr("shell-suggest-translate"),
                    tr("shell-suggest-translate-prompt"),
                    "languages"
                ),
                chip(
                    tr("shell-suggest-ideas"),
                    tr("shell-suggest-ideas-prompt"),
                    "ai.assistant"
                )
            ]
            .spacing(6),
        ]
        .spacing(6)
        .align_x(Alignment::Center);
        container(column![body, space().height(14), chips].align_x(Alignment::Center))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    fn assistant_history(&self, palette: &Palette) -> Element<'_, ShellMessage> {
        let state = &self.assistant;
        let muted = palette.text_muted;
        let mut list = column![].spacing(2);
        let query = state.history_search.trim().to_lowercase();
        let mut matches = 0;
        for session in &state.sessions {
            if !query.is_empty()
                && !session.title.to_lowercase().contains(&query)
                && !session.preview.to_lowercase().contains(&query)
            {
                continue;
            }
            matches += 1;
            let active = state.session.as_deref() == Some(session.id.as_str());
            let content = column![
                row![
                    enz_text(session.title.clone())
                        .size_px(BODY)
                        .weight(Weight::Semibold)
                        .ellipsis(iced::widget::text::Ellipsis::End)
                        .into_element(),
                    space().width(Length::Fill),
                    enz_text(ago(session.updated))
                        .size_px(TINY)
                        .color(muted)
                        .into_element(),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                enz_text(session.preview.replace(['*', '`'], ""))
                    .size_px(SMALL)
                    .color(muted)
                    .wrap(false)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .into_element(),
            ]
            .spacing(2);
            list = list.push(
                enz_pressable(
                    container(content)
                        .padding(Padding {
                            top: 7.0,
                            right: 10.0,
                            bottom: 7.0,
                            left: 10.0,
                        })
                        .into(),
                    message(Message::Open(session.id.clone())),
                )
                .id(format!("assistant-session-{}", session.id).into())
                .aria_label(session.title.clone())
                .style(PressableStyle::Row)
                .selected(active)
                .width(Length::Fill)
                .into_element(),
            );
        }
        if matches == 0 {
            list = list.push(
                enz_text(tr(if query.is_empty() {
                    "shell-no-chats-yet"
                } else {
                    "shell-assistant-no-chat-matches"
                }))
                .size_px(SMALL)
                .color(muted)
                .into_element(),
            );
        }
        column![
            enz_text_field(
                "",
                tr("shell-assistant-search-chats"),
                &state.history_search,
                |value| message(Message::HistorySearch(value))
            )
            .id("assistant-search".into())
            .aria_label(tr("shell-assistant-search-chats"))
            .into_element(),
            enzui_widgets::scroll_view::enz_scroll_view(list)
                .height(Length::Fill)
                .scrollbar_tokens(self.tokens().scrollbar)
                .into_element(),
        ]
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn assistant_transcript(&self, palette: &Palette) -> Element<'_, ShellMessage> {
        let state = &self.assistant;
        let palette = palette.clone();
        let elevated = palette.surface_elevated;
        let running = state.running.is_some();
        let can_revise = state.can_revise();
        let last = state.transcript.rows.len() as u64;
        let geometry = state.transcript.geometry();
        let viewport = state.viewport.max(200.0);
        let offset = if state.away {
            state.scroll
        } else {
            (geometry.total_height() - viewport).max(0.0)
        };
        let list =
            enz_virtual_list_with_geometry(&state.transcript, geometry, move |index, row: &Row| {
                assistant_row(row, &palette, running, index + 1 == last, can_revise)
            })
            .scroll_offset(offset)
            .viewport_height(viewport)
            .on_scroll(|viewport| message(Message::Scrolled(viewport)))
            .on_measure(move |measurement| {
                message(Message::Measured(
                    state.transcript.rows[measurement.index].key.clone(),
                    measurement,
                ))
            })
            .id(TRANSCRIPT.into())
            .overscan(4)
            .into_element();
        let list = enz_tail_follow(list);
        let mut layers = stack![list];
        if state.away {
            let pill = enz_pressable(
                row![
                    enz_icon("arrow-down").size(13.0).into_element(),
                    enz_text(enzui_shell::i18n::tr("shell-latest"))
                        .size_px(SMALL)
                        .into_element()
                ]
                .spacing(5)
                .align_y(Alignment::Center)
                .into(),
                message(Message::Latest),
            )
            .style(PressableStyle::Chip)
            .id("assistant-latest".into())
            .aria_label(tr("shell-jump-to-the-latest-message"))
            .width(Length::Shrink)
            .into_element();
            let pill = container(pill).style(move |_| container::Style {
                background: Some(elevated.into()),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                    offset: iced::Vector::new(0.0, 2.0),
                    blur_radius: 8.0,
                },
                ..Default::default()
            });
            layers = layers.push(
                container(pill)
                    .center_x(Length::Fill)
                    .align_bottom(Length::Fill)
                    .padding(Padding {
                        bottom: 8.0,
                        ..Padding::ZERO
                    }),
            );
        }
        layers.width(Length::Fill).height(Length::Fill).into()
    }

    fn assistant_approvals(&self, palette: &Palette) -> Element<'_, ShellMessage> {
        let state = &self.assistant;
        if state.confirming_full_send {
            return column![
                enz_text(tr("shell-agent-full-send-warning"))
                    .size_px(SMALL)
                    .wrap(true)
                    .color(palette.danger)
                    .into_element(),
                row![
                    enz_button(tr("shell-cancel"), message(Message::ShowApprovals(false)))
                        .id("assistant-approval-cancel".into())
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .into_element(),
                    enz_button(
                        tr("shell-agent-confirm-full-send"),
                        message(Message::ConfirmFullSend)
                    )
                    .id("assistant-confirm-full-send".into())
                    .kind(ButtonKind::Accent)
                    .size(ButtonSize::Sm)
                    .into_element(),
                ]
                .spacing(6),
            ]
            .spacing(6)
            .into();
        }
        let mut options = column![].spacing(4);
        for mode in ApprovalMode::ALL {
            options = options.push(
                enz_button(mode.label(), message(Message::ChooseApproval(mode)))
                    .id(format!("assistant-approval-{}", mode.wire()).into())
                    .kind(if mode == state.approval_mode {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Quiet
                    })
                    .size(ButtonSize::Sm)
                    .enabled(!state.approval_busy)
                    .into_element(),
            );
        }
        let mut screenshots = column![
            enz_text(tr("shell-agent-screenshots-caption"))
                .size_px(SMALL)
                .color(palette.text_muted)
                .wrap(true)
                .into_element()
        ]
        .spacing(4);
        for access in Screenshots::ALL {
            screenshots = screenshots.push(
                enz_button(access.label(), message(Message::ChooseScreenshots(access)))
                    .id(format!("assistant-screenshot-policy-{}", access.wire()).into())
                    .kind(if state.screenshots == access {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Quiet
                    })
                    .size(ButtonSize::Sm)
                    .enabled(!state.approval_busy)
                    .into_element(),
            );
        }
        let mut permissions = column![options, screenshots].spacing(8);
        if state.session.is_some() && enzui_shell::desktop_session::mode().managed() {
            permissions = permissions.push(
                enz_button(
                    tr("shell-assistant-control-allow"),
                    message(Message::AllowDesktopControl),
                )
                .id("assistant-desktop-control".into())
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .enabled(!state.desktop_control_pending)
                .into_element(),
            );
        }
        permissions.into()
    }

    fn assistant_composer(&self, palette: &Palette) -> Element<'_, ShellMessage> {
        let state = &self.assistant;
        let running = state.running.is_some();
        let empty = state.composer.text().trim().is_empty();
        let muted = palette.text_muted;
        let model_name = if state.model == "local" || state.model.is_empty() {
            tr("shell-assistant-model-auto")
        } else {
            state.model.as_str()
        };
        let model = row![
            enz_icon("cpu").size(13.0).tint(muted).into_element(),
            enz_text(model_name)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .size_px(TINY)
                .color(muted)
                .into_element()
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let mut trailing = Vec::new();
        if running && !empty {
            // Typing during a turn steers it; Stop stays at hand.
            trailing.push(
                enz_toolbar_icon("square", message(Message::Stop))
                    .tokens(self.icon_tokens())
                    .id("assistant-stop".into())
                    .aria_label(tr("shell-stop"))
                    .tooltip(tr("shell-stop"))
                    .into_element(),
            );
        }
        let mut leading = vec![
            enz_button(
                state.approval_mode.label(),
                message(Message::ShowApprovals(!state.choosing_approval)),
            )
            .id("assistant-approval-policy".into())
            .kind(if state.approval_mode == ApprovalMode::FullSend {
                ButtonKind::Secondary
            } else {
                ButtonKind::Quiet
            })
            .size(ButtonSize::Sm)
            .enabled(!running && !state.approval_busy)
            .into_element(),
        ];
        leading.push(
            enz_toolbar_icon(
                "paperclip",
                message(Message::Attachments(attachments::Message::Pick)),
            )
            .tokens(self.icon_tokens())
            .id("assistant-attach".into())
            .aria_label(tr("shell-assistant-attach-files"))
            .tooltip(tr("shell-assistant-attachment-limits"))
            .enabled(state.attachments_available())
            .into_element(),
        );
        if state.screenshots != Screenshots::Off {
            leading.push(enz_icon("camera").size(13.0).tint(muted).into_element());
        } else if state.approval_mode != ApprovalMode::WorkspaceEdits {
            leading.push(model.into());
        }
        let attachment_only =
            empty && !state.files.is_empty() && state.revision.is_none() && !running;
        if attachment_only {
            trailing.push(
                enz_toolbar_icon("send", message(Message::Send))
                    .tokens(self.icon_tokens())
                    .aria_label(tr("shell-assistant-describe-attachments"))
                    .tooltip(tr("shell-assistant-describe-attachments"))
                    .enabled(!state.attachment_loader.busy() && !state.approval_busy)
                    .into_element(),
            );
        }
        let composer = enz_composer(&state.composer, |action| message(Message::Edit(action)))
            .placeholder(if running {
                enzui_shell::i18n::current().text("shell-steer-the-reply")
            } else {
                enzui_shell::i18n::current().text("shell-ask-anything")
            })
            .max_rows(6)
            .busy(running && empty)
            .rail_leading(leading)
            .rail_trailing(trailing)
            .id(COMPOSER)
            .aria_label(enzui_shell::i18n::tr("shell-message-the-assistant"));
        let composer = if attachment_only {
            composer
        } else {
            composer
                .on_send(message(Message::Send))
                .on_stop(message(Message::Stop))
        };
        let composer = if let Some(running) = &state.running {
            let elapsed = state
                .now
                .unwrap_or(running.since)
                .saturating_duration_since(running.since)
                .as_secs();
            let phase = frame_clock_phase();
            let read = std::rc::Rc::clone(&phase);
            let base = palette.text;
            let label = enz_text(format!("{}…", state.transcript.activity()))
                .size_px(SMALL)
                .color_resolved(move |_| {
                    let pulse = (read.get() * std::f32::consts::TAU / 1.6).sin() * 0.22 + 0.72;
                    Color {
                        a: base.a * pulse,
                        ..base
                    }
                });
            composer.status(
                row![
                    enz_spinner().size(12.0).into_element(),
                    enz_frame_clock(label.into_element(), phase).into_element(),
                    space().width(Length::Fill),
                    enz_text(format!("{}:{:02}", elapsed / 60, elapsed % 60))
                        .size_px(TINY)
                        .color(muted)
                        .into_element(),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
            )
        } else {
            composer
        };
        let mut content = column![].spacing(6);
        if state.revision.is_some() {
            content = content.push(
                row![
                    enz_text(tr("shell-assistant-editing-last"))
                        .size_px(SMALL)
                        .wrap(true)
                        .into_element(),
                    enz_button(tr("shell-cancel"), message(Message::CancelEdit))
                        .kind(ButtonKind::Quiet)
                        .size(ButtonSize::Sm)
                        .enabled(!running)
                        .into_element(),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        } else {
            for (index, file) in state.files.iter().enumerate() {
                let preview: Element<'_, ShellMessage> = match file.preview() {
                    Some(handle) => iced::widget::image(handle).width(30).height(26).into(),
                    None => enz_icon("file-text").size(15.0).tint(muted).into_element(),
                };
                content = content.push(
                    row![
                        preview,
                        enz_text(file.name.as_ref())
                            .size_px(SMALL)
                            .ellipsis(iced::widget::text::Ellipsis::End)
                            .into_element(),
                        space().width(Length::Fill),
                        enz_toolbar_icon(
                            "x",
                            message(Message::Attachments(attachments::Message::Remove(index)))
                        )
                        .tokens(self.icon_tokens())
                        .size(22.0, 13.0)
                        .aria_label(format!(
                            "{} {}",
                            tr("shell-assistant-remove-attachment"),
                            file.name
                        ))
                        .tooltip(tr("shell-assistant-remove-attachment"))
                        .enabled(!state.attachment_loader.busy() && !state.approval_busy)
                        .into_element(),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }
        }
        if state.attachment_loader.busy() {
            content = content.push(
                enz_text(tr("shell-assistant-loading-files"))
                    .size_px(SMALL)
                    .into_element(),
            );
        }
        content.push(composer.into_element()).into()
    }
}

fn disclosure(open: bool, muted: Color) -> Element<'static, ShellMessage> {
    enz_icon(if open {
        "chevron-down"
    } else {
        "chevron-right"
    })
    .size(12.0)
    .tint(muted)
    .into_element()
}

fn assistant_row<'a>(
    row: &Row,
    palette: &Palette,
    running: bool,
    last: bool,
    can_revise: bool,
) -> Element<'a, ShellMessage> {
    let muted = palette.text_muted;
    let inset = mix(palette.surface, palette.text, 0.06);
    let key = row.key.clone();
    let element: Element<'a, ShellMessage> = match &row.body {
        Body::User {
            text,
            steering,
            attachments,
        } => {
            let bubble = enzui_widgets::max_width::max_width(
                container(
                    column![
                        enz_text(text.clone())
                            .size_px(BODY)
                            .wrap(true)
                            .into_element(),
                        if attachments.is_empty() {
                            space().height(0).into()
                        } else {
                            enz_text(attachments.join(" · "))
                                .size_px(TINY)
                                .color(muted)
                                .wrap(true)
                                .into_element()
                        },
                    ]
                    .spacing(3),
                )
                .padding(Padding {
                    top: 7.0,
                    right: 11.0,
                    bottom: 7.0,
                    left: 11.0,
                })
                .style(move |_| container::Style {
                    background: Some(inset.into()),
                    border: iced::Border {
                        radius: 14.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                300.0,
            );
            let mut column = column![].spacing(3).align_x(Alignment::End);
            if *steering {
                column = column.push(
                    row![
                        enz_icon("corner-down-right")
                            .size(11.0)
                            .tint(muted)
                            .into_element(),
                        enz_text(enzui_shell::i18n::tr("shell-steering"))
                            .size_px(TINY)
                            .color(muted)
                            .into_element()
                    ]
                    .spacing(3)
                    .align_y(Alignment::Center),
                );
            }
            container(column.push(bubble))
                .align_right(Length::Fill)
                .padding(Padding {
                    top: 6.0,
                    bottom: 8.0,
                    ..Padding::ZERO
                })
                .into()
        }
        Body::Markdown { text, blocks, .. } => container(
            enz_markdown_blocks(blocks.get_or_init(|| markdown::parse(text)).clone())
                .body_size(BODY)
                .heading_scale(1.0)
                .line_height_multiple(1.45)
                .block_spacing(7.0)
                .on_link_clicked(|url| message(Message::Link(url)))
                .into_element(),
        )
        .padding(Padding {
            top: 2.0,
            bottom: 6.0,
            right: 2.0,
            ..Padding::ZERO
        })
        .width(Length::Fill)
        .into(),
        Body::Image { alt, image, path } => {
            let picture: Element<'a, ShellMessage> = match image {
                Some((handle, width, height)) => {
                    let shown = (super::transcript::CONTENT_WIDTH).min(*width as f32);
                    let tall = shown * *height as f32 / *width as f32;
                    container(
                        iced::widget::image(handle.clone())
                            .width(Length::Fixed(shown))
                            .height(Length::Fixed(tall))
                            .border_radius(10.0),
                    )
                    .into()
                }
                None => enz_text(tr!("shell-image-unavailable-2", "arg0" => path.clone()))
                    .size_px(SMALL)
                    .color(muted)
                    .into_element(),
            };
            let mut column = column![picture].spacing(4);
            if !alt.is_empty() {
                column = column.push(
                    enz_text(alt.clone())
                        .size_px(TINY)
                        .color(muted)
                        .into_element(),
                );
            }
            container(column)
                .padding(Padding {
                    top: 2.0,
                    bottom: 8.0,
                    ..Padding::ZERO
                })
                .into()
        }
        Body::Thought { text, millis, live } => {
            let label = if *live {
                tr("shell-thinking").to_owned()
            } else {
                tr!("shell-thought-for", "arg0" => seconds(*millis))
            };
            let heading = enz_pressable(
                row![
                    enz_icon("brain").size(14.0).tint(muted).into_element(),
                    enz_text(label).size_px(SMALL).color(muted).into_element(),
                    disclosure(row.open, muted),
                    space().width(Length::Fill)
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
                message(Message::Toggled(key.clone())),
            )
            .id(format!("assistant-expand-{key}").into())
            .style(PressableStyle::Plain)
            .into_element();
            let mut column = column![container(heading).align_left(Length::Fill)].spacing(4);
            if row.open {
                column = column.push(
                    container(
                        enz_text(text.clone())
                            .size_px(SMALL)
                            .color(muted)
                            .wrap(true)
                            .into_element(),
                    )
                    .padding(Padding {
                        top: 6.0,
                        right: 10.0,
                        bottom: 6.0,
                        left: 10.0,
                    })
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(inset.into()),
                        border: iced::Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                );
            }
            container(column)
                .padding(Padding {
                    top: 1.0,
                    bottom: 3.0,
                    ..Padding::ZERO
                })
                .into()
        }
        Body::Tool {
            tool,
            title,
            arguments,
            output,
            status,
            millis,
            ..
        } => {
            let status_icon: Element<'a, ShellMessage> = match status {
                ToolStatus::Running if running => enz_spinner().size(12.0).into_element(),
                ToolStatus::Running | ToolStatus::Cancelled => {
                    enz_icon("square").size(10.0).tint(muted).into_element()
                }
                ToolStatus::Done => enz_icon("check").size(12.0).tint(muted).into_element(),
                ToolStatus::Failed => enz_icon("x").size(12.0).tint(palette.danger).into_element(),
            };
            let title = if title.is_empty() {
                if tool == "screenshot" {
                    tr("shell-agent-tool-screenshot")
                } else {
                    tool.as_str()
                }
            } else {
                title.as_str()
            };
            let label = row![
                enz_icon(tool_icon(tool))
                    .size(14.0)
                    .tint(if *status == ToolStatus::Failed {
                        palette.danger
                    } else {
                        muted
                    })
                    .into_element(),
                enz_text(title)
                    .size_px(SMALL)
                    .color(muted)
                    .ellipsis(iced::widget::text::Ellipsis::End)
                    .into_element(),
                space().width(Length::Fill),
                if *millis > 0 {
                    enz_text(seconds(*millis))
                        .size_px(TINY)
                        .color(muted)
                        .into_element()
                } else {
                    space().into()
                },
                status_icon,
                disclosure(row.open, muted),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            let heading = enz_pressable(
                container(label)
                    .padding(Padding {
                        top: 3.0,
                        right: 4.0,
                        bottom: 3.0,
                        left: 2.0,
                    })
                    .into(),
                message(Message::Toggled(key.clone())),
            )
            .id(format!("assistant-expand-{key}").into())
            .style(PressableStyle::Plain)
            .aria_label(format!(
                "{title}, {}",
                match status {
                    ToolStatus::Running => enzui_shell::i18n::tr("shell-a11y-running"),
                    ToolStatus::Done => enzui_shell::i18n::tr("shell-a11y-done"),
                    ToolStatus::Failed => enzui_shell::i18n::tr("shell-a11y-failed"),
                    ToolStatus::Cancelled => enzui_shell::i18n::tr("shell-a11y-stopped"),
                }
            ))
            .width(Length::Fill)
            .into_element();
            let mut column = column![heading].spacing(4);
            if row.open {
                let mut detail = column![].spacing(6);
                if !arguments.is_empty() {
                    detail = detail.push(
                        enz_text(tool.clone())
                            .size_px(TINY)
                            .color(muted)
                            .weight(Weight::Semibold)
                            .into_element(),
                    );
                    detail = detail.push(
                        enz_code_block(arguments.clone())
                            .show_header(false)
                            .show_copy(false)
                            .body_size(TINY)
                            .code_padding_x(0.0)
                            .code_padding_y(0.0)
                            .code_max_height(90.0)
                            .into_element(),
                    );
                }
                if !output.is_empty() {
                    detail = detail.push(
                        enz_text(tr("shell-output"))
                            .size_px(TINY)
                            .color(muted)
                            .weight(Weight::Semibold)
                            .into_element(),
                    );
                    detail = detail.push(
                        enz_code_block(output.trim_end().to_owned())
                            .show_header(false)
                            .show_copy(false)
                            .body_size(TINY)
                            .code_padding_x(0.0)
                            .code_padding_y(0.0)
                            .code_max_height(180.0)
                            .into_element(),
                    );
                }
                column = column.push(
                    container(detail)
                        .padding(Padding {
                            top: 8.0,
                            right: 10.0,
                            bottom: 8.0,
                            left: 10.0,
                        })
                        .width(Length::Fill)
                        .style(move |_| container::Style {
                            background: Some(inset.into()),
                            border: iced::Border {
                                radius: 8.0.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                );
            }
            container(column)
                .padding(Padding {
                    bottom: 2.0,
                    ..Padding::ZERO
                })
                .into()
        }
        Body::Approval {
            tool,
            title,
            arguments,
            decision,
            ..
        } => {
            let mut card = column![
                row![
                    enz_icon("shield-alert")
                        .size(16.0)
                        .tint(palette.warning)
                        .into_element(),
                    enz_text(enzui_shell::tr!("shell-allow-2", "arg0" => enzui_shell::i18n::argument(if title.trim().is_empty() { tool } else { title })))
                        .size_px(BODY)
                        .weight(Weight::Semibold)
                        .into_element(),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                enz_text(enzui_shell::tr!("shell-the-assistant-wants-to-use", "arg0" => enzui_shell::i18n::argument(&(tool))))
                    .size_px(SMALL)
                    .color(muted)
                    .into_element(),
            ]
            .spacing(6);
            if !arguments.is_empty() {
                card = card.push(
                    enz_code_block(arguments.clone())
                        .show_header(false)
                        .show_copy(false)
                        .body_size(TINY)
                        .code_padding_x(0.0)
                        .code_padding_y(0.0)
                        .code_max_height(90.0)
                        .into_element(),
                );
            }
            let footer: Element<'a, ShellMessage> = match decision {
                Decision::Pending | Decision::Sending(_) => {
                    let idle = *decision == Decision::Pending;
                    row![
                        space().width(Length::Fill),
                        enz_button(
                            tr("shell-deny"),
                            message(Message::Decide(key.clone(), false, false))
                        )
                        .id(format!("assistant-deny-{key}").into())
                        .kind(ButtonKind::Quiet)
                        .size(ButtonSize::Sm)
                        .enabled(idle)
                        .into_element(),
                        if tool == "screenshot" {
                            space().width(Length::Shrink).into()
                        } else {
                            enz_button(
                                tr("shell-agent-allow-for-session"),
                                message(Message::Decide(key.clone(), true, true)),
                            )
                            .id(format!("assistant-allow-session-{key}").into())
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .enabled(idle)
                            .into_element()
                        },
                        enz_button(
                            tr(if tool == "screenshot" {
                                "shell-agent-allow-screenshots"
                            } else {
                                "shell-allow-once"
                            }),
                            message(Message::Decide(key.clone(), true, false))
                        )
                        .id(format!("assistant-allow-once-{key}").into())
                        .kind(ButtonKind::Accent)
                        .size(ButtonSize::Sm)
                        .enabled(idle)
                        .into_element(),
                    ]
                    .spacing(6)
                    .into()
                }
                Decision::Allowed => row![
                    enz_icon("check")
                        .size(13.0)
                        .tint(palette.success)
                        .into_element(),
                    enz_text(enzui_shell::i18n::tr("shell-allowed"))
                        .size_px(SMALL)
                        .color(muted)
                        .into_element()
                ]
                .spacing(5)
                .align_y(Alignment::Center)
                .into(),
                Decision::Denied => row![
                    enz_icon("x").size(13.0).tint(muted).into_element(),
                    enz_text(enzui_shell::i18n::tr("shell-denied"))
                        .size_px(SMALL)
                        .color(muted)
                        .into_element()
                ]
                .spacing(5)
                .align_y(Alignment::Center)
                .into(),
                Decision::Closed => enz_text(tr("shell-no-longer-needed"))
                    .size_px(SMALL)
                    .color(muted)
                    .into_element(),
            };
            card = card.push(footer);
            container(
                container(card)
                    .padding(12)
                    .width(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(inset.into()),
                        border: iced::Border {
                            radius: 12.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .padding(Padding {
                top: 4.0,
                bottom: 8.0,
                ..Padding::ZERO
            })
            .into()
        }
        Body::Note { text, error } => {
            let line = row![
                enz_icon(if *error { "circle-alert" } else { "square" })
                    .size(14.0)
                    .tint(if *error { palette.danger } else { muted })
                    .into_element(),
                enz_text(text.clone())
                    .size_px(SMALL)
                    .color(if *error { palette.text } else { muted })
                    .wrap(true)
                    .into_element(),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            let mut column = column![line].spacing(6);
            if *error && !running && last {
                column = column.push(
                    container(
                        enz_button(tr("shell-try-again"), message(Message::Retry))
                            .id("assistant-retry".into())
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .icon("rotate-ccw")
                            .into_element(),
                    )
                    .padding(Padding {
                        left: 20.0,
                        ..Padding::ZERO
                    }),
                );
            }
            container(column)
                .padding(Padding {
                    top: 4.0,
                    bottom: 8.0,
                    ..Padding::ZERO
                })
                .into()
        }
        Body::Meta { text, reply } => {
            let mut line = row![].spacing(4).align_y(Alignment::Center);
            if !text.is_empty() {
                line = line.push(
                    enz_text(text.clone())
                        .size_px(TINY)
                        .color(muted)
                        .into_element(),
                );
            }
            if !reply.is_empty() {
                line = line.push(
                    enz_toolbar_icon("copy", message(Message::Copy(reply.clone())))
                        .size(22.0, 13.0)
                        .aria_label(tr("shell-copy-the-reply"))
                        .tooltip(tr("shell-copy"))
                        .into_element(),
                );
            }
            if last && can_revise {
                for (icon, label, event) in [
                    (
                        "rotate-ccw",
                        tr("shell-assistant-regenerate"),
                        Message::Regenerate,
                    ),
                    ("pencil", tr("shell-assistant-edit-last"), Message::EditLast),
                ] {
                    line = line.push(
                        enz_toolbar_icon(icon, message(event))
                            .size(22.0, 13.0)
                            .aria_label(label)
                            .tooltip(label)
                            .into_element(),
                    );
                }
            }
            container(line)
                .padding(Padding {
                    bottom: 10.0,
                    ..Padding::ZERO
                })
                .into()
        }
    };
    container(element).width(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use enzui_core::collect::SemanticsCollector;
    use iced_runtime::user_interface::{Cache, UserInterface};

    #[test]
    fn same_title_sessions_expose_distinct_click_targets() {
        let compositor = enzui_shell::compositor::Compositor::new(
            "/usr/bin/true".into(),
            "assistant-fixture".into(),
        )
        .unwrap();
        let mut shell = Shell::new(compositor, "/".into()).0;
        shell.assistant.sessions = ["session-a", "session-b"]
            .map(|id| super::super::SessionRow {
                id: id.into(),
                title: "Same title".into(),
                preview: String::new(),
                updated: 0,
            })
            .into();
        let mut renderer = iced::futures::executor::block_on(
            <iced::Renderer as iced::advanced::renderer::Headless>::new(
                iced::advanced::renderer::Settings::default(),
                Some("tiny-skia"),
            ),
        )
        .expect("a headless renderer");
        let mut ui = UserInterface::build(
            shell.assistant_history(&shell.tokens().palette),
            iced::Size::new(420.0, 480.0),
            Cache::default(),
            &mut renderer,
        );
        let mut collector = SemanticsCollector::new();
        ui.operate(&renderer, &mut collector);
        let targets: Vec<_> = collector
            .collected()
            .iter()
            .filter(|node| node.semantics.role == enzui_core::semantics::Role::Button)
            .filter(|node| node.semantics.label.as_deref() == Some("Same title"))
            .map(|node| node.id.as_ref().expect("a routable widget id"))
            .collect();
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0], targets[1]);
    }
}
