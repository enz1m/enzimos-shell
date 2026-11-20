use super::{Message, SettingsApp};
use enzui_shell::font_settings::{self, FontChange, Slot, Weight};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_theme::FontRole;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    search_field::enz_search_field,
    text::enz_text,
};
use iced::{
    Alignment, Element, Length,
    widget::{column, container, row},
};

fn font(change: FontChange) -> Message {
    Message::Change(enzui_shell::settings::Change::Font(change))
}

// Installed icon themes, read once per Settings window.
fn icon_themes() -> &'static [enzui_shell::icon_theme::Installed] {
    static THEMES: std::sync::OnceLock<Vec<enzui_shell::icon_theme::Installed>> =
        std::sync::OnceLock::new();
    THEMES.get_or_init(enzui_shell::icon_theme::installed)
}

impl SettingsApp {
    pub(super) fn icon_theme_setting(&self) -> Element<'_, Message> {
        let themes = icon_themes();
        let name = |id: &str| {
            themes
                .iter()
                .find(|t| t.id == id)
                .map_or_else(|| id.to_owned(), |t| t.name.clone())
        };
        static DEFAULT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let follow = tr!("shell-follow-desktop", "arg0" => name(DEFAULT.get_or_init(enzui_shell::icon_theme::default_theme)));
        let current = match &self.settings.icon_theme {
            Some(id) => name(id),
            None => follow.clone(),
        };
        let mut choices = vec![(
            follow,
            Message::Change(enzui_shell::settings::Change::IconTheme(None)),
        )];
        choices.extend(themes.iter().map(|theme| {
            (
                theme.name.clone(),
                Message::Change(enzui_shell::settings::Change::IconTheme(Some(
                    theme.id.clone(),
                ))),
            )
        }));
        self.choice_setting("appearance/icon_theme", current, choices)
    }

    pub(super) fn font_settings(&self) -> Element<'_, Message> {
        let fonts = &self.settings.fonts;
        let writable = !self.closing;
        let mut rows = column![].spacing(2);
        for slot in Slot::ALL {
            let current = fonts.family(slot);
            let mut controls = row![].spacing(6).align_y(Alignment::Center);
            if current.is_some() {
                controls = controls.push(
                    enz_button(tr("shell-default"), font(FontChange::Family(slot, None)))
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Sm)
                        .id(format!("font-{}-default", slot.key()).into())
                        .enabled(writable)
                        .into_element(),
                );
            }
            controls = controls.push(
                enz_button(
                    tr("shell-change"),
                    Message::FontQuery(Some((slot, String::new()))),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(format!("font-{}-change", slot.key()).into())
                .enabled(writable)
                .into_element(),
            );
            let row = crate::settings_app::settings_row(slot.label())
                .subtitle(format!(
                    "{} · {}",
                    current.unwrap_or_else(|| tr("shell-default")),
                    slot.description()
                ))
                .icon(match slot {
                    Slot::Interface => "type",
                    Slot::Document => "book-open",
                    Slot::Monospace => "code",
                })
                .id_prefix(format!("control-fonts/{}", slot.key()))
                .control(Element::from(controls))
                .into_element();
            rows = rows.push(self.mark_setting(&format!("fonts/{}", slot.key()), row));
            if let Some((open, query)) = &self.font_query
                && *open == slot
            {
                rows = rows.push(self.font_picker(slot, query, current));
            }
        }
        // Only the weights this interface font really has: the rest would
        // snap to a face already offered.
        let family = self
            .tokens()
            .font_catalog
            .family(enzui_theme::FontRole::Ui)
            .clone();
        let faces = font_settings::family_weights(&family);
        let mut offered = Weight::available(&faces);
        if !offered.contains(&fonts.weight) {
            offered.push(fonts.weight);
        }
        if offered.len() > 1 {
            rows = rows.push(
                self.choice_setting(
                    "fonts/weight",
                    fonts.weight.label(),
                    offered
                        .into_iter()
                        .map(|weight| (weight.label().to_owned(), font(FontChange::Weight(weight))))
                        .collect(),
                ),
            );
        } else {
            rows = rows.push(self.mark_setting(
                "fonts/weight",
                crate::settings_app::settings_row(tr("shell-font-weight"))
                    .icon(enzui_shell::settings_catalog::icon("fonts/weight").unwrap_or(""))
                    .subtitle(tr("shell-this-interface-font-comes-in-one-weight-choose-another-font-for-lighter-or-h"))
                    .control(enz_text(tr("shell-regular")).into_element())
                    .enabled(false)
                    .into_element(),
            ));
        }
        rows.push(self.font_sample()).into()
    }

    fn font_picker<'a>(
        &'a self,
        slot: Slot,
        query: &'a str,
        current: Option<&str>,
    ) -> Element<'a, Message> {
        let matches = font_settings::matches(query, slot, 8);
        let mut picker = column![
            row![
                enz_search_field(query, move |text| Message::FontQuery(Some((slot, text))))
                    .placeholder(tr("shell-search-installed-fonts"))
                    .id(format!("font-{}-search", slot.key()))
                    .focus_on_mount(true)
                    .into_element(),
                enz_button(tr("shell-cancel"), Message::FontQuery(None))
                    .kind(ButtonKind::Plain)
                    .into_element(),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(2);
        if matches.is_empty() {
            picker = picker.push(
                enz_text(if slot == Slot::Monospace {
                    tr("shell-no-installed-monospace-fonts-match")
                } else {
                    tr("shell-no-installed-fonts-match")
                })
                .muted_on(self.tokens().palette.surface)
                .into_element(),
            );
        }
        let size = f32::from(self.settings.metrics.text) + 2.0;
        for family in matches {
            let chosen = current == Some(family.name);
            picker = picker.push(
                container(
                    row![
                        iced::widget::text(family.name)
                            .font(iced::Font::new(family.name))
                            .size(size)
                            .width(Length::Fill),
                        enz_button(
                            if chosen {
                                tr("shell-in-use")
                            } else {
                                enzui_shell::i18n::tr("shell-use")
                            },
                            font(FontChange::Family(slot, Some(family.name.to_owned())))
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id(format!("font-{}-use-{}", slot.key(), family.name).into())
                        .enabled(!chosen && !self.closing)
                        .into_element(),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([6, 12]),
            );
        }
        container(picker).padding([8, 12]).into()
    }

    fn font_sample(&self) -> Element<'_, Message> {
        let theme = self.tokens();
        let muted = theme.palette.background;
        let line = |role: FontRole, label: &'static str, sample: &'static str| {
            column![
                enz_text(label)
                    .size_px(enzui_theme::text_scale::effective().apply_px(11.0))
                    .muted_on(muted)
                    .into_element(),
                enz_text(sample)
                    .font_role(role)
                    .font_from_theme(&theme)
                    .wrap(true)
                    .into_element(),
            ]
            .spacing(2)
        };
        self.preview_frame(
            column![
                line(
                    FontRole::Ui,
                    "Interface",
                    tr("shell-settings-notifications-and-the-control-center")
                ),
                line(
                    FontRole::Document,
                    enzui_shell::i18n::tr("shell-document"),
                    tr("shell-the-quick-brown-fox-jumps-over-the-lazy-dog")
                ),
                line(
                    FontRole::Monospace,
                    enzui_shell::i18n::tr("shell-monospace"),
                    "fn main() { println!(\"0O 1lI\"); }"
                ),
            ]
            .spacing(10),
        )
    }
}
