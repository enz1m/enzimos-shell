use super::{Message, message, source_chord};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::search::Ticket;
use enzui_shell::tr;
use enzui_widgets::{
    icon::enz_icon,
    listbox_shape::{listbox, listbox_option},
    motion::{MotionEffect, motion},
    offset_scrollbar::enz_offset_scrollbar,
    pressable::{PressableStyle, enz_pressable},
    search_field::enz_search_field,
    semantics_container::SemanticsContainer,
    suggestion_list::SuggestionSize,
    text::{TextSize, enz_text},
    text_field::enz_text_field,
    toolbar_icon::enz_toolbar_icon,
};
use iced::{
    Element, Length,
    widget::{column, container, row},
};

impl Shell {
    pub(crate) fn quick_search_view(&self) -> Element<'_, ShellMessage> {
        let state = &self.search;
        let composing = state.preedit.is_some();
        let expanded = self.search_expanded();
        let palette = self.window_tokens(state.window).palette;
        let mut input_palette = palette.clone();
        // The launcher is one search surface; its always-focused input needs no inner ring.
        input_palette.focus_ring = iced::Color::TRANSPARENT;
        input_palette.field_border = iced::Color::TRANSPARENT;
        input_palette.field_border_strong = iced::Color::TRANSPARENT;
        // The field rests in the tint the results take under the pointer, so
        // it reads as the panel's search bar instead of loose text.
        input_palette.field = over(palette.surface, palette.overlay_hover());
        let theme = self.tokens();
        let button_radius = theme.radius.step(theme.shape.button);
        // A pill inside the panel's rounding, like the other popups' fields.
        let input_radius = (f32::from(self.metrics.radius) - 6.0).max(8.0);
        if composing && state.alias.is_none() {
            // SearchField shares a role between its preedit-unaware placeholder and icon;
            // restore only the decorative icon without replacing or renaming the input.
            input_palette.text_muted.a = 0.0;
        }
        let size = state
            .surface_size
            .unwrap_or_else(|| self.search_window_extent());
        let paint_height = self.search_paint_height();
        let maximum = state.geometry.as_ref().map_or(size, |g| g.0);
        let compact = state.preferences.compact;
        let capacity = if state.alias.is_some() {
            usize::from(maximum.height >= 320.0)
        } else {
            self.search_capacity()
        };
        let visible = enzui_core::palette::window_from(
            state.model.results.len(),
            capacity.max(1),
            if state.browse_all_apps {
                None
            } else {
                state.model.selected_index()
            },
            state.model.window_start,
        );
        let blank_apps = state.model.query.trim().is_empty()
            && (state.model.category.is_none() || state.browse_all_apps)
            && state.blank_apps()
            && state.alias.is_none()
            && !composing;
        let active = state
            .model
            .selected_index()
            .filter(|index| !composing && visible.contains(*index))
            .and_then(|index| state.model.results.get(index))
            .map(|item| {
                if blank_apps && !state.browse_all_apps {
                    format!("quick-app-{}", item.id).into()
                } else {
                    enzui_core::identity::derive_id("quick-search-result", &item.id)
                }
            });
        let input = enz_search_field(&state.model.query, |query| message(Message::Query(query)))
            .placeholder(tr("shell-search-2"))
            .palette(input_palette)
            .id("quick-search-input")
            .radius(input_radius)
            .size(SuggestionSize::Lg)
            .active_descendant(active)
            .width(Length::Fill)
            .enabled(state.alias.is_none() && !state.action_busy)
            .into_element();
        let input_size = SuggestionSize::Lg.tokens();
        let composing_icon = container(
            enz_icon("action.search")
                .size(18.0)
                .tint(palette.text_muted)
                .opacity(if composing && state.alias.is_none() {
                    1.0
                } else {
                    0.0
                })
                .into_element(),
        )
        .padding(iced::Padding::default().left(input_size.padding_x))
        .align_y(iced::Alignment::Center)
        .height(input_size.height)
        .width(Length::Fill);
        let heading = iced::widget::stack![input, composing_icon].width(Length::Fill);
        let failure = self.search_failure();
        let feedback = failure.or(state.feedback.as_deref());
        let presentation_key = format!(
            "{}:{}:{}:{}:{}:{:?}:{feedback:?}",
            state.model.generation,
            if state.browse_all_apps {
                0
            } else {
                visible.start
            },
            visible.len,
            size.width.to_bits(),
            size.height.to_bits(),
            state.alias.as_ref().map(|a| &a.0)
        );
        let feedback_view = feedback.map(|text| {
            enz_text(text)
                .size(TextSize::Caption)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .lines(2)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .full_width()
                .color(if failure.is_some() {
                    palette.tone_error_fg
                } else {
                    palette.text_muted
                })
                .into_element()
        });
        let mut body = column![]
            .spacing(6)
            .width(Length::Fill)
            .height(Length::Fill);
        // Settings and All apps follow the bottom edge of the card.
        let mut footer: Option<Element<'_, ShellMessage>> = None;
        if blank_apps {
            let top = &state.top_apps;
            // Equal-width app slots fill the row; names remain visible for pointer and keys.
            let mut cards = row![].spacing(4).width(Length::Fill);
            for (index, item) in top.iter().enumerate() {
                // While the full list is open, keys move through the list.
                let chosen = !state.browse_all_apps && state.model.selected_index() == Some(index);
                let selected = chosen && state.model.explicit_selection;
                let glyph = match &item.image {
                    Some(path) => enzui_shell::icon_theme::source(path).into_element(46.0),
                    None => enz_icon(&item.icon)
                        .size(28.0)
                        .tint(palette.text_muted)
                        .into_element(),
                };
                let highlight = super::results::selection_fill(&palette);
                let tile: Element<'_, ShellMessage> = container(
                    column![
                        container(glyph).center_x(Length::Fill).center_y(60),
                        enz_text(&item.title)
                            .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                            .single_line_elided()
                            .full_width()
                            .align(enzui_widgets::text::TextAlign::Center)
                            .announce(false)
                            .into_element(),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                )
                .padding(iced::Padding::ZERO.left(4.0).right(4.0).bottom(4.0))
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: selected.then_some(highlight.into()),
                    border: iced::Border {
                        radius: 14.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into();
                let action =
                    enz_pressable(tile, message(Message::Activate(state.model.ticket(item))))
                        .style(PressableStyle::Plain)
                        .semantics(listbox_option(
                            item.title.clone(),
                            chosen,
                            index + 1,
                            top.len(),
                        ))
                        .id(format!("quick-app-{}", item.id).into())
                        .radius(14.0)
                        .padding(0)
                        .width(Length::Fill)
                        .into_element();
                let action = enzui_widgets::tooltip::enz_tooltip(
                    action,
                    item.title.clone(),
                    iced::widget::tooltip::Position::Bottom,
                );
                cards =
                    cards.push(container(self.dock_search_host(item, action)).width(Length::Fill));
            }
            if top.is_empty() {
                body = body.push(
                    enz_text(if state.apps_busy {
                        tr("shell-finding-installed-applications")
                    } else {
                        tr("shell-no-installed-applications-yet")
                    })
                    .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                    .muted_on(palette.surface_elevated)
                    .into_element(),
                );
            } else {
                body = body.push(SemanticsContainer::new(
                    cards,
                    listbox(tr("shell-recommended")),
                ));
            }
            let browsing = state.browse_all_apps;
            footer = Some(
                row![
                    // Not "Settings": the Settings app is often among the apps.
                    enz_toolbar_icon("settings", message(Message::Settings))
                        .aria_label(tr("shell-quick-search-settings"))
                        .tooltip(tr("shell-quick-search-settings"))
                        .enabled(state.alias.is_none() && !composing)
                        .id("quick-search-settings".into())
                        .into_element(),
                    iced::widget::space().width(Length::Fill),
                    enz_pressable(
                        row![
                            enz_text(tr("shell-all-apps"))
                                .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                                .color(palette.text_muted)
                                .into_element(),
                            enz_icon(if browsing {
                                "chevron-up"
                            } else {
                                "chevron-down"
                            })
                            .size(14.0)
                            .tint(palette.text_muted)
                            .into_element(),
                        ]
                        .spacing(5)
                        .align_y(iced::Alignment::Center)
                        .into(),
                        message(Message::BrowseApps(!browsing)),
                    )
                    .style(PressableStyle::Plain)
                    .aria_label(if browsing {
                        tr("shell-recommended")
                    } else {
                        tr("shell-show-all-installed-apps")
                    })
                    .radius(button_radius)
                    .id(if browsing {
                        "quick-search-top-apps"
                    } else {
                        "quick-search-all-apps"
                    }
                    .into())
                    .enabled(state.apps_ready && !state.action_busy)
                    .padding(4)
                    .width(Length::Shrink)
                    .into_element(),
                ]
                .align_y(iced::Alignment::Center)
                .width(Length::Fill)
                .into(),
            );
        }
        // The full app list opens under the recommended apps.
        if (!blank_apps || state.browse_all_apps) && capacity > 0 {
            let list = self.search_results(visible.start, visible.len, compact);
            let list: Element<'_, ShellMessage> = if state.browse_all_apps {
                // The list scrolls by rows; the thumb shows there are more apps below.
                let row = super::support::row_pitch(compact);
                let start = visible.start;
                enz_offset_scrollbar(list)
                    .content_len(row * state.model.results.len() as f32)
                    .viewport_len(row * visible.len as f32)
                    .offset(row * start as f32)
                    .backdrop(palette.solid_backdrop(palette.surface))
                    .on_scroll_to(move |px| {
                        message(Message::ScrollApps((px / row).round() - start as f32))
                    })
                    .on_wheel(|delta| {
                        let amount = match delta {
                            iced::mouse::ScrollDelta::Lines { y, .. } => y,
                            iced::mouse::ScrollDelta::Pixels { y, .. } => y / 32.0,
                        };
                        message(Message::ScrollApps(-amount))
                    })
                    .into()
            } else {
                iced::widget::mouse_area(list)
                    .on_scroll(|delta| {
                        let amount = match delta {
                            iced::mouse::ScrollDelta::Lines { y, .. } => y,
                            iced::mouse::ScrollDelta::Pixels { y, .. } => y / 32.0,
                        };
                        message(Message::Navigate(if amount > 0.0 {
                            -3
                        } else if amount < 0.0 {
                            3
                        } else {
                            0
                        }))
                    })
                    .into()
            };
            body = body.push(list);
        }
        let mut alias_footer: Option<Element<'_, ShellMessage>> = None;
        if let Some((id, title)) = &state.alias {
            let identity = format!("{}:{id}", state.epoch);
            let mut footer = Vec::new();
            if let Some(feedback) = feedback_view {
                footer.push((format!("{identity}:feedback"), feedback));
            }
            footer.push((
                format!("{identity}:label"),
                enz_text(if composing {
                    tr("shell-finish-composing-the-quick-key").to_owned()
                } else {
                    tr!("shell-quick-key-for", "arg0" => title.clone())
                })
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .lines(2)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .full_width()
                .into_element(),
            ));
            footer.push((
                format!("{identity}:editor"),
                row![
                    container(
                        enz_text_field(
                            "",
                            enzui_shell::i18n::current().text("shell-2-16-letters-or-digits"),
                            &state.alias_draft,
                            |value| message(Message::AliasDraft(value))
                        )
                        .id("quick-search-alias".into())
                        .into_element()
                    )
                    .width(Length::Fill),
                    enzui_widgets::keyed_column::keyed_column(vec![(
                        format!("{presentation_key}:{composing}"),
                        row![
                            enzui_widgets::button::enz_button(
                                enzui_shell::i18n::tr("shell-save"),
                                message(Message::AliasSave)
                            )
                            .enabled(!composing)
                            .into_element(),
                            enzui_widgets::button::enz_button(
                                tr("shell-cancel"),
                                message(Message::AliasCancel)
                            )
                            .into_element(),
                        ]
                        .spacing(8)
                        .into()
                    )]),
                ]
                .spacing(8)
                .into(),
            ));
            alias_footer = Some(
                enzui_widgets::keyed_column::keyed_column(footer)
                    .spacing(8.0)
                    .width(Length::Fill)
                    .into(),
            );
        } else {
            if let Some(feedback) = feedback_view {
                body = body.push(feedback);
            }
            let selected = state.model.selected_item();
            let ticket = selected.map_or_else(
                || Ticket {
                    generation: state.model.generation,
                    id: String::new(),
                },
                |item| state.model.ticket(item),
            );
            let persistent = selected.is_some_and(|item| {
                item.target.is_some()
                    && ["app:", "setting:", "action:"]
                        .iter()
                        .any(|prefix| item.id.starts_with(prefix))
            });
            let favorite =
                selected.is_some_and(|item| state.preferences.favorites.contains(&item.id));
            // Keep the active filter visible even when disabled; offer other categories only if
            // they can return results.
            let offered = state.preferences.offered_categories(state.model.category);
            let options = std::iter::once(tr("shell-all").to_owned())
                .chain(offered.iter().map(|category| {
                    if state.preferences.shows(*category) {
                        category.label().to_owned()
                    } else {
                        tr!("shell-off-2", "arg0" => category.label().to_owned())
                    }
                }))
                .collect();
            let selected_source = Some(
                state
                    .model
                    .category
                    .and_then(|category| {
                        offered
                            .iter()
                            .position(|value| *value == category)
                            .map(|index| index + 1)
                    })
                    .unwrap_or(0),
            );
            let routes = offered;
            let source = enzui_widgets::combo_box::enz_combo_box("", options, None, |_| {
                message(Message::Tick)
            })
            .variant(enzui_widgets::tokens::FieldVariant::Inline)
            .size(enzui_widgets::text_field::TextFieldSize::Sm)
            .radius(button_radius)
            .palette(theme.palette)
            .selected_index(selected_source)
            .aria_label(enzui_shell::i18n::tr("shell-search-in"))
            .on_select_index(move |index| {
                message(Message::Category(
                    index
                        .checked_sub(1)
                        .and_then(|index| routes.get(index))
                        .copied(),
                ))
            })
            .id("quick-search-source".into())
            .enabled(!state.action_busy && !composing)
            .into_element();
            let source = enzui_widgets::keyed_column::keyed_column(vec![(
                format!(
                    "{}:{}:{:?}",
                    state.epoch, state.source_revision, state.model.category
                ),
                source,
            )]);
            if !blank_apps && !state.browse_all_apps {
                body = body.push(
                    row![
                        container(source).width(Length::Fixed(if size.width < 400.0 {
                            100.0
                        } else {
                            130.0
                        })),
                        iced::widget::space().width(Length::Fill),
                        enz_toolbar_icon(
                            if favorite { "star-off" } else { "star" },
                            message(Message::Favorite(ticket.clone()))
                        )
                        .aria_label(if favorite {
                            tr("shell-remove-favorite")
                        } else {
                            tr("shell-add-favorite")
                        })
                        .tooltip(if favorite {
                            tr("shell-remove-favorite")
                        } else {
                            tr("shell-add-favorite")
                        })
                        .enabled(persistent && !state.action_busy)
                        .id("quick-search-favorite".into())
                        .into_element(),
                        enz_toolbar_icon("keyboard", message(Message::Alias(ticket)))
                            .aria_label(tr("shell-assign-quick-key"))
                            .tooltip(tr("shell-assign-quick-key"))
                            .enabled(persistent && !state.action_busy)
                            .id("quick-search-quick-key".into())
                            .into_element(),
                        enz_toolbar_icon("refresh-cw", message(Message::Refresh))
                            .aria_label(tr("shell-refresh-search-sources"))
                            .tooltip(tr("shell-refresh-search-sources"))
                            .id("quick-search-refresh".into())
                            .into_element(),
                        enz_toolbar_icon("settings", message(Message::Settings))
                            .aria_label(tr("shell-quick-search-settings"))
                            .tooltip(tr("shell-quick-search-settings"))
                            .enabled(!composing)
                            .id("quick-search-settings".into())
                            .into_element(),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        if composing && state.alias.is_none() {
            body = column![
                enz_text(enzui_shell::i18n::tr("shell-composing"))
                    .size(TextSize::Caption)
                    .muted_on(palette.surface_elevated)
                    .into_element(),
            ];
        }
        let show_results = expanded || blank_apps;
        let result_body: Element<'_, ShellMessage> = if show_results {
            body.into()
        } else {
            iced::widget::space().height(0).into()
        };
        let results =
            enzui_widgets::keyed_column::keyed_column(vec![(presentation_key, result_body)])
                .width(Length::Fill)
                .height(if show_results {
                    Length::Fill
                } else {
                    Length::Fixed(0.0)
                });
        let mut content = column![heading, results].spacing(if show_results { 6 } else { 0 });
        if let Some(footer) = alias_footer {
            content = content.push(footer);
        }
        let island = state.island;
        let join = island.map_or(0.0, |island| {
            island.attachment.join(f32::from(self.metrics.radius))
        });
        // The centred card's window keeps 8 px around it: room for a minimal
        // shadow, which it has unless panel shadows are off.
        let lifted = self.desired_settings.panel_shadow > 0;
        let lit = 0.2126 * palette.surface_elevated.r
            + 0.7152 * palette.surface_elevated.g
            + 0.0722 * palette.surface_elevated.b
            > 0.5;
        let card_radius = f32::from(self.metrics.radius);
        // Keep the app rows at their final layout and reveal them through the
        // space above the footer. The footer itself uses the current height.
        let settled = self.search_surface_size().height - join - 16.0 - CARD_PADDING.y();
        let content: Element<'_, ShellMessage> = if let Some(footer) = footer {
            use super::support::{FOOTER_GAP, FOOTER_HEIGHT};
            column![
                container(enzui_shell::natural_height::natural_height(
                    content,
                    settled - FOOTER_HEIGHT - FOOTER_GAP,
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true),
                container(footer)
                    .height(FOOTER_HEIGHT)
                    .align_y(iced::Alignment::Center),
            ]
            .spacing(FOOTER_GAP)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            enzui_shell::natural_height::natural_height(content, settled).into()
        };
        let card = container(content)
            .id("quick-search-visible-card")
            .padding(CARD_PADDING)
            .width(Length::Fill)
            .height(Length::Fixed((paint_height - join - 16.0).max(0.0)))
            .clip(true)
            .style(move |_| {
                if island.is_some() {
                    container::Style::default()
                } else {
                    container::Style {
                        background: Some(palette.surface_elevated.into()),
                        border: iced::Border {
                            radius: card_radius.into(),
                            ..Default::default()
                        },
                        shadow: if lifted {
                            iced::Shadow {
                                color: iced::Color::from_rgba(
                                    0.0,
                                    0.0,
                                    0.0,
                                    if lit { 0.16 } else { 0.42 },
                                ),
                                offset: iced::Vector::new(0.0, 1.0),
                                blur_radius: 6.0,
                            }
                        } else {
                            iced::Shadow::default()
                        },
                        ..Default::default()
                    }
                }
            });
        let id = state.window.unwrap_or(self.panel);
        let epoch = state.epoch;
        let card = iced::widget::mouse_area(card).on_press(message(Message::FocusInput));
        let content: Element<'_, ShellMessage> = if let Some(island) = island {
            let body = container(
                container(card)
                    .padding(8)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .padding({
                // Joined to the frame, the panel's fillets take room at its
                // sides.
                let (left, right) = island.attachment.insets(f32::from(self.metrics.radius));
                // The panel's rounded far corners take their room away from
                // the bar, as the other joined panels' do (it sat over the
                // search field, which then floated under a band of empty panel).
                iced::Padding {
                    top: if island.bottom { join } else { 0.0 },
                    bottom: if island.bottom { 0.0 } else { join },
                    left,
                    right,
                }
            })
            .width(Length::Fill)
            .height(Length::Fill);
            crate::region::popup_fitted(
                body.into(),
                crate::region::Popup {
                    visible: state.visible,
                    bottom: island.bottom,
                    hidden: message(Message::Hidden(id, epoch)),
                    theme: self.theme(id),
                    attachment: island.attachment,
                    radius: f32::from(self.metrics.radius),
                    color: self.glass_tint(id, self.tokens().palette.surface),
                    expressive: self
                        .search
                        .preferences
                        .animations
                        .then(|| {
                            enzui_shell::motion::expressive(
                                self.desired_settings.compositor.animation_style,
                            )
                        })
                        .flatten(),
                    input: self.surface_inputs.get(&id).cloned(),
                    shadow: self.panel_shadow(),
                    glass: self.panel_glass(),
                },
                Some(("quick-search-visible-card".into(), 16.0 + join)),
            )
        } else {
            if let Some(input) = self.surface_inputs.get(&id) {
                input.rectangle(size.width.round() as i32, paint_height.round() as i32);
            }
            // The fade's layer is clipped to its bounds, so it spans the
            // padding the card's shadow falls into.
            let animated: Element<'_, ShellMessage> = motion(
                container(card)
                    .padding(8)
                    .width(Length::Fill)
                    .height(Length::Fill),
                state.visible,
                MotionEffect::Fade,
            )
            .on_hidden_with(move || message(Message::Hidden(id, epoch)))
            .into();
            iced::widget::mouse_area(animated)
                .on_press(message(Message::Dismiss))
                .into()
        };
        enzui_widgets::keyboard_capture::keyboard_precapture(content, |key, modifiers| {
            source_chord(key, modifiers).is_some()
        })
    }
}

// `top` composited over `base`, opaque.
/// The card's padding: less at its foot, under the last row.
const CARD_PADDING: iced::Padding = iced::Padding {
    top: 8.0,
    right: 8.0,
    bottom: 4.0,
    left: 8.0,
};

fn over(base: iced::Color, top: iced::Color) -> iced::Color {
    let mix = |b: f32, t: f32| b + (t - b) * top.a;
    iced::Color::from_rgb(mix(base.r, top.r), mix(base.g, top.g), mix(base.b, top.b))
}
