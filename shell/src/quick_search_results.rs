use super::{Message, State, message};
use crate::{Message as ShellMessage, Shell};
use enzui_shell::i18n::tr;
use enzui_shell::search::{Category, Target};
use enzui_widgets::{
    icon::enz_icon,
    listbox_shape::{listbox, listbox_notice, listbox_option},
    pressable::{PressableStyle, enz_pressable},
    semantics_container::SemanticsContainer,
    text::enz_text,
    tooltip::enz_tooltip,
};

// Height of a category heading row, counted by the surface-size calculation
// so a grouped list is not clipped by its own headings.
pub(super) const GROUP_HEADING_HEIGHT: f32 = 20.0;

/// The chosen result's (and chosen top app's) fill: an opaque accent tint over
/// the mode's ground, so it shows on glass too (the translucent `accent_soft`
/// all but vanished over light glass, below 3:1 against the panel).
pub(super) fn selection_fill(palette: &enzui_theme::Palette) -> iced::Color {
    let ground = iced::Color {
        a: 1.0,
        ..palette.background
    };
    let lit = 0.2126 * ground.r + 0.7152 * ground.g + 0.0722 * ground.b > 0.5;
    let f = if lit { 0.20 } else { 0.32 };
    iced::Color {
        r: ground.r + (palette.accent.r - ground.r) * f,
        g: ground.g + (palette.accent.g - ground.g) * f,
        b: ground.b + (palette.accent.b - ground.b) * f,
        a: 1.0,
    }
}
use iced::{
    Element, Length,
    widget::{column, container, row},
};

impl State {
    pub(crate) fn app_tiles_visible(&self) -> bool {
        self.model.query.trim().is_empty()
            && self.model.category.is_none()
            && !self.browse_all_apps
            && self.preferences.shows(Category::Apps)
            && self.alias.is_none()
            && self.preedit.is_none()
    }
    pub(super) fn file_wait_notice(&self) -> Option<&'static str> {
        if !self.preferences.shows(Category::Files)
            || self.preferences.roots.is_empty()
            || self.model.query.chars().take(2).count() < 2
            || self
                .model
                .category
                .is_some_and(|category| category != Category::Files)
        {
            return None;
        }
        if self.file_busy {
            Some(tr("shell-indexing-chosen-folders"))
        } else if self.file_query_busy {
            Some(tr("shell-searching-file-names"))
        } else {
            None
        }
    }
}

impl Shell {
    pub(super) fn search_results(
        &self,
        start: usize,
        len: usize,
        compact: bool,
    ) -> Element<'_, ShellMessage> {
        let state = &self.search;
        let palette = self.tokens().palette;
        let mut rows = Vec::new();
        if len == 0 {
            let notice = if let Some(notice) = state.file_wait_notice() {
                notice
            } else if state.apps_busy {
                tr("shell-loading-applications")
            } else if state.model.category == Some(Category::Files)
                && state.preferences.roots.is_empty()
            {
                tr("shell-choose-search-folders-in-quick-search-settings")
            } else if state.model.category == Some(Category::Files)
                && state.model.query.chars().count() < 2
            {
                tr("shell-type-at-least-two-characters-to-search-file-names")
            } else if state.model.category == Some(Category::Clipboard) && !state.clipboard.enabled
            {
                tr("shell-clipboard-history-is-off-enable-it-in-settings")
            } else if state.model.category == Some(Category::Clipboard)
                && state.clipboard.entries.is_empty()
            {
                tr("shell-clipboard-history-is-empty-copy-text-or-an-image-in-an-app")
            } else {
                tr("shell-no-matches")
            };
            rows.push((
                "notice:".into(),
                SemanticsContainer::leaf(
                    container(
                        enz_text(notice)
                            .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                            .muted_on(palette.surface_elevated)
                            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                            .full_width()
                            .into_element(),
                    )
                    .padding(8)
                    .height(super::support::row_pitch(compact) - 4.0)
                    .width(Length::Fill),
                    listbox_notice(notice),
                )
                .into(),
            ));
        }
        let selected_index = state.model.selected_index();
        let fill = selection_fill(&palette);
        let window = start..start + len;
        for (index, item) in state.model.results.iter().enumerate().skip(start).take(len) {
            if state.model.group_heading(index, &window).is_some() {
                // Decorative: each row already announces its own category, so a
                // heading node would make assistive technology say it twice.
                rows.push((
                    result_key(true, &item.id),
                    container(
                        enz_text(state.model.heading_text(index))
                            .size_px(if compact { 10.0 } else { 11.0 })
                            .weight_rung(enzui_theme::WeightRung::Semibold)
                            .muted_on(palette.surface_elevated)
                            .no_wrap()
                            .full_width()
                            .announce(false)
                            .into_element(),
                    )
                    .padding(iced::Padding::ZERO.left(14.0).right(10.0).top(4.0))
                    .width(Length::Fill)
                    .height(GROUP_HEADING_HEIGHT)
                    .into(),
                ));
            }
            let selected = selected_index == Some(index);
            // The chosen row, and while typing the top hit Enter opens, share
            // one soft accent highlight (no outline).
            let emphasized = selected
                && (state.model.explicit_selection || !state.model.query.trim().is_empty());
            // An app needs no description to be recognised and opened.
            let is_app = matches!(item.target.as_ref(), Some(Target::App(_)));
            let title = enz_text(&item.title)
                .size_px(if compact { 13.0 } else { 15.0 })
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .no_wrap()
                .full_width()
                .announce(false)
                .into_element();
            let subtitle = enz_text(&item.subtitle)
                .size_px(if compact { 11.0 } else { 13.0 })
                .muted_on(if emphasized {
                    fill
                } else {
                    palette.surface_elevated
                })
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                .lines(1)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .full_width()
                .announce(false)
                .into_element();
            let height = super::support::row_pitch(compact) - 4.0;
            let cell = container(
                row![
                    // App images and symbolic icons share one slot, so every
                    // row's text starts at the same edge.
                    container(match &item.image {
                        Some(path) => enzui_shell::icon_theme::source(path)
                            .into_element(if compact { 28.0 } else { 32.0 }),
                        None => enz_icon(&item.icon).size(20.0).into_element(),
                    })
                    .center_x(32)
                    .center_y(32),
                    if is_app || item.subtitle.is_empty() {
                        column![title].width(Length::Fill)
                    } else {
                        column![title, subtitle].spacing(2).width(Length::Fill)
                    }
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            )
            .padding(iced::Padding::ZERO.left(10.0).right(10.0))
            .width(Length::Fill)
            .height(height)
            .center_y(height)
            .clip(true)
            .style(move |_| container::Style {
                background: emphasized.then_some(fill.into()),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            let action = enz_pressable(
                cell.into(),
                message(Message::Activate(state.model.ticket(item))),
            )
            .style(PressableStyle::Row)
            .enabled(state.alias.is_none() && state.preedit.is_none() && !state.action_busy)
            .radius(6.0)
            .embedded(true)
            .id(enzui_core::identity::derive_id(
                "quick-search-result",
                &item.id,
            ))
            .semantics(listbox_option(
                format!(
                    "{}, {}, {}, {}",
                    item.title,
                    item.verb,
                    item.category.label(),
                    item.subtitle
                ),
                selected,
                index + 1,
                state.model.results.len(),
            ))
            .into_element();
            let action = match item.target.as_ref() {
                Some(Target::File { path, .. })
                    if state.alias.is_none() && state.preedit.is_none() && !state.action_busy =>
                {
                    enzui_widgets::file_drag::enz_file_drag_source(action, [path.clone()])
                        .allows_captured_press(true)
                        .into()
                }
                _ => action,
            };
            let action = self.dock_search_host(item, action);
            let result = if state.browse_all_apps || state.pending_browse_collapse {
                action
            } else {
                enz_tooltip(
                    action,
                    item.action_hint(),
                    iced::widget::tooltip::Position::Bottom,
                )
            };
            let expressive = state
                .preferences
                .animations
                .then(|| {
                    enzui_shell::motion::expressive(
                        self.desired_settings.compositor.animation_style,
                    )
                })
                .flatten();
            let result = if expressive.is_some() {
                let entrance = enzui_widgets::motion::motion(
                    result,
                    true,
                    enzui_widgets::motion::MotionEffect::Fade,
                )
                .expressive(Some(enzui_theme::ExpressiveMotion::FastEffects));
                if state.browse_all_apps
                    || state.pending_browse_collapse
                    || state.model.explicit_selection
                {
                    entrance.into()
                } else {
                    entrance.enter_on_mount().into()
                }
            } else {
                result
            };
            rows.push((result_key(false, &item.id), result));
        }
        SemanticsContainer::new(
            enzui_widgets::keyed_column::keyed_column(rows)
                .spacing(4.0)
                .width(Length::Fill),
            listbox(tr("shell-quick-search-results")),
        )
        .into()
    }
}

fn result_key(heading: bool, id: &str) -> String {
    format!("{}:{id}", if heading { "heading" } else { "result" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_state() -> State {
        let mut state = State::default();
        state.preferences.categories.push(Category::Files);
        state.preferences.roots.push("/owned-fixture".into());
        state.model.query = "report".into();
        state.model.category = Some(Category::Files);
        state
    }

    #[test]
    fn pending_file_work_is_not_a_completed_empty_result() {
        let mut state = file_state();
        assert_eq!(state.file_wait_notice(), None);
        state.file_query_busy = true;
        assert_eq!(state.file_wait_notice(), Some("Searching file names…"));
        state.file_busy = true;
        assert_eq!(state.file_wait_notice(), Some("Indexing chosen folders…"));
    }
}
