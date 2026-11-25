use super::{Message, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::theme_palette::{self, PRESETS, Scheme, Seed, Style};
use enzui_shell::theme_presets::{Choices, Corners, Density, Scrollbars, Shadows};
use enzui_shell::theme_store::Load;
use enzui_shell::tr;
use enzui_theme::EnzThemeMode;
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    color_picker::{Hsva, enz_color_picker},
    color_swatch::enz_color_swatch,
    enz_text,
    pressable::{PressableStyle, enz_pressable},
    text::TextTone,
};
use iced::{
    Alignment, Element, Length,
    widget::{column, container, row, svg},
};

/// What the theme editor asks the application to do.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Action {
    Choose { before: Choices, after: Choices },
    ReducedMotion(bool),
    WindowButtons(enzui_theme::WindowButtons),
    FocusRing(f32),
    WindowRadius(u16),
    WindowFrame(enzui_theme::WindowFrame),
    ButtonShape(enzui_theme::RadiusStep),
}

/// The custom colour dialog and the wallpaper colour.
#[derive(Clone, Debug)]
pub(super) enum Edit {
    Custom(bool),
    /// Opens the same picker for the focused window border's colour.
    BorderCustom,
    /// Opens the same picker for the shell's own colour.
    ChromeCustom,
    Pick(Hsva),
    Hex(String),
    UseCustom,
    Host(enzui_widgets::modal_host::ModalHostEvent),
    FromWallpaper,
    WallpaperRead(u64, Option<iced::Color>),
}

/// What a custom colour is chosen for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Theme,
    Border,
    Chrome,
}

/// The custom colour being edited.
#[derive(Clone, Debug)]
pub(super) struct Custom {
    pub value: Hsva,
    pub hex: String,
    /// What the colour is for: only the theme previews a palette.
    pub target: Target,
}

pub(super) fn edit(value: Edit) -> Message {
    Message::ThemeEdit(value)
}

impl SettingsApp {
    pub(super) fn theme_mode(&self) -> EnzThemeMode {
        if let Some(themes) = &self.themes {
            return themes.theme().mode();
        }
        if self.settings.light {
            EnzThemeMode::Light
        } else {
            EnzThemeMode::Dark
        }
    }

    pub(super) fn theme_choices(&self) -> Choices {
        let mut choices = self.theme.choices();
        if let Some(scheme) = self
            .settings
            .theme_scheme
            .as_deref()
            .and_then(Scheme::from_id_str)
        {
            choices.scheme = scheme;
        }
        choices
    }

    fn theme_mockup(&self, choices: Choices) -> Element<'_, Message> {
        let mode = self.theme_mode();
        let other = match mode {
            EnzThemeMode::Dark => EnzThemeMode::Light,
            _ => EnzThemeMode::Dark,
        };
        let art = super::theme_preview::card(choices, mode, other);
        container(
            svg(svg::Handle::from_memory(art.into_bytes()))
                .width(Length::Fill)
                .height(260.0),
        )
        .width(Length::Fill)
        .into()
    }

    fn style_setting(&self, choices: Choices) -> Element<'_, Message> {
        use super::theme_preview::{TILE_HEIGHT, tile};
        let mode = self.theme_mode();
        let tiles = Style::ALL.into_iter().map(|style| {
            let scheme = Scheme {
                style,
                ..choices.scheme
            };
            let art = tile(scheme, mode, choices.corners.rungs()[3]);
            let selected = choices.scheme.style == style;
            enz_pressable(
                column![
                    svg(svg::Handle::from_memory(art.into_bytes()))
                        .width(Length::Fill)
                        .height(TILE_HEIGHT * 0.9),
                    enz_text(style.label())
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(6)
                .align_x(Alignment::Center)
                .into(),
                Message::Theme(Action::Choose {
                    before: choices,
                    after: Choices { scheme, ..choices },
                }),
            )
            .style(PressableStyle::Card)
            .selected(selected)
            .selectable(true)
            .aria_label(tr!("shell-style-2", "arg0" => style.label().to_owned()))
            .id(format!("appearance-style-{}", style.id_str()).into())
            .padding(8)
            .radius(enzui_theme::text_defaults::shape_radius(|shape| shape.card))
            .border_width(1.0)
            .width(Length::Fill)
            .enabled(self.controls_live())
            .into_element()
        });
        let body = column![
            row(tiles).spacing(8),
            enz_text(match choices.scheme.style {
                Style::Neutral =>
                    tr("shell-quiet-grey-surfaces-the-colour-marks-buttons-selection-and-focus"),
                Style::Tinted => tr("shell-surfaces-and-lines-take-a-soft-wash-of-the-colour"),
                Style::Vibrant => tr("shell-rich-colourful-surfaces-and-a-saturated-accent"),
                Style::Contrast =>
                    tr("shell-deeper-surfaces-brighter-text-and-stronger-lines-in-the-colour"),
            })
            .tone(TextTone::Muted)
            .wrap(true)
            .into_element(),
        ]
        .spacing(10);
        self.mark_setting(
            "appearance/style",
            column![
                crate::settings_app::settings_row(enzui_shell::i18n::tr("shell-style"))
                    .icon("swatch-book")
                    .into_element(),
                container(body).padding(under_row_title()),
            ],
        )
    }

    fn color_setting(&self, choices: Choices) -> Element<'_, Message> {
        let scheme = choices.scheme;
        let live = self.controls_live();
        let current = scheme.seed;
        let mut swatches = row![].spacing(10).align_y(Alignment::Center);
        for (name, hex) in PRESETS {
            let color = theme_palette::rgb(hex);
            let seed = if hex == 0x0a84ff {
                Seed::Default
            } else {
                Seed::Color(color)
            };
            let selected = current == seed || (hex == 0x0a84ff && current == Seed::Color(color));
            swatches = swatches.push(
                enz_color_swatch(
                    color,
                    Message::Theme(Action::Choose {
                        before: choices,
                        after: Choices {
                            scheme: Scheme { seed, ..scheme },
                            ..choices
                        },
                    }),
                )
                .size(30.0)
                .radius(15.0)
                .selected(selected)
                .aria_label(name)
                .id_prefix(format!("appearance-color-{}", name.to_lowercase()))
                .enabled(live)
                .into_element(),
            );
        }
        let custom = matches!(current, Seed::Color(color) if !PRESETS.iter().any(|(_, hex)| theme_palette::rgb(*hex) == color));
        if let (true, Seed::Color(color)) = (custom, current) {
            swatches = swatches.push(
                enz_color_swatch(color, edit(Edit::Custom(true)))
                    .size(30.0)
                    .radius(15.0)
                    .selected(true)
                    .aria_label(tr("shell-custom-colour"))
                    .id_prefix("appearance-color-custom")
                    .enabled(live)
                    .into_element(),
            );
        }
        if let Seed::Wallpaper(color) = current {
            swatches = swatches.push(
                enz_color_swatch(color, edit(Edit::FromWallpaper))
                    .size(30.0)
                    .radius(15.0)
                    .selected(true)
                    .aria_label(tr("shell-the-wallpaper-s-colour"))
                    .id_prefix("appearance-color-wallpaper")
                    .enabled(live)
                    .into_element(),
            );
        }
        // The two ways to pick any colour sit beside the swatches, and wrap
        // under them only when the row runs out of room.
        let swatches = swatches.push(iced::widget::Space::new().width(6)).push(
            row![
                enz_button(tr("shell-from-wallpaper"), edit(Edit::FromWallpaper))
                    .kind(ButtonKind::Secondary)
                    .icon("image")
                    .id("appearance-color-from-wallpaper".into())
                    .enabled(live && self.wallpaper.supported)
                    .into_element(),
                enz_button(
                    enzui_shell::i18n::tr("shell-custom-2"),
                    edit(Edit::Custom(true))
                )
                .kind(ButtonKind::Secondary)
                .icon("pipette")
                .id("appearance-color-custom-open".into())
                .enabled(live)
                .into_element(),
            ]
            .spacing(8),
        );
        let body = column![
            crate::settings_app::settings_row(enzui_shell::i18n::tr("shell-color"))
                .icon("palette")
                .into_element(),
            container(swatches.wrap().vertical_spacing(10)).padding(under_row_title()),
        ];
        self.mark_setting("appearance/accent", body)
    }

    pub(super) fn theme_custom_dialog(&self) -> Option<Element<'_, Message>> {
        let custom = self.theme_custom.as_ref()?;
        let palette = self.tokens().palette;
        let color = custom.value.to_color();
        let preview = Scheme {
            seed: Seed::Color(color),
            ..self.theme_choices().scheme
        };
        let generated = theme_palette::shown(preview, self.theme_mode());
        let chips = [
            "background",
            "surface",
            "surface_selected",
            "accent",
            "accent_soft",
            "text",
        ]
        .map(|role| {
            let fill = generated.get(role);
            container(iced::widget::Space::new().width(28).height(28))
                .style(move |_| container::Style {
                    background: Some(fill.into()),
                    border: iced::Border {
                        radius: 8.0.into(),
                        width: 1.0,
                        color: palette.border,
                    },
                    ..Default::default()
                })
                .into()
        });
        let parsed = theme_palette::parse_hex(custom.hex.trim());
        let (title, label) = match custom.target {
            Target::Border => (
                tr("shell-window-border-custom"),
                tr("shell-window-border-custom"),
            ),
            Target::Chrome => (tr("shell-chrome-custom"), tr("shell-chrome-custom")),
            Target::Theme => (tr("shell-custom-colour"), tr("shell-theme-colour")),
        };
        let preview = column![
            enz_text(tr("shell-the-palette-it-makes"))
                .tone(TextTone::Muted)
                .into_element(),
            row(chips).spacing(6),
        ]
        .spacing(6);
        let content = column![
            enz_text(title)
                .size_px(self.text_px(18.0))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            enz_color_picker(custom.value, |value| edit(Edit::Pick(value)))
                .label(label)
                .hex_text(custom.hex.clone())
                .hex_error(if parsed.is_some() || custom.hex.is_empty() {
                    String::new()
                } else {
                    enzui_shell::i18n::tr("shell-use-rrggbb").into()
                })
                .on_hex_input(|text| edit(Edit::Hex(text)))
                .id_prefix("appearance-custom-color")
                .into_element(),
        ]
        .spacing(16)
        .push((custom.target == Target::Theme).then_some(preview))
        .push(
            row![
                iced::widget::space::horizontal(),
                enz_button(
                    enzui_shell::i18n::tr("shell-cancel"),
                    edit(Edit::Custom(false))
                )
                .kind(ButtonKind::Plain)
                .id("appearance-custom-cancel".into())
                .into_element(),
                enz_button(tr("shell-use-colour"), edit(Edit::UseCustom))
                    .kind(ButtonKind::Primary)
                    .id("appearance-custom-use".into())
                    .into_element(),
            ]
            .spacing(8),
        );
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content.into())
                .width(420.0)
                .padding(20.0)
                .on_dismiss(|| edit(Edit::Custom(false)))
                .close_label(tr("shell-close-custom-colour"))
                .into_element(),
        )
    }

    fn menu_setting<T: Copy + PartialEq + 'static>(
        &self,
        route: &'static str,
        current: T,
        all: &'static [T],
        label: fn(T) -> &'static str,
        with: impl Fn(T) -> Choices,
    ) -> Element<'_, Message> {
        self.choice_setting(
            route,
            label(current),
            all.iter()
                .map(|&value| {
                    (
                        label(value).to_owned(),
                        Message::Theme(Action::Choose {
                            before: self.theme_choices(),
                            after: with(value),
                        }),
                    )
                })
                .collect(),
        )
    }

    fn window_buttons_setting(&self) -> Element<'_, Message> {
        let current = self.tokens().window_buttons;
        let choices = enzui_theme::WindowButtons::NAMES
            .iter()
            .filter_map(|name| enzui_theme::WindowButtons::from_name(name))
            .map(|buttons| {
                (
                    window_buttons_label(buttons).to_owned(),
                    Message::Theme(Action::WindowButtons(buttons)),
                )
            })
            .collect();
        self.choice_setting(
            "appearance/window_buttons",
            window_buttons_label(current),
            choices,
        )
    }

    fn window_frame_setting(&self) -> Element<'_, Message> {
        let current = self.tokens().window_frame;
        // Clear refracting glass (no blur) leaves window content unreadable
        // over other windows: Settings offers the frosted materials only.
        let choices = enzui_theme::WindowFrame::NAMES
            .iter()
            .filter(|name| !matches!(**name, "glass" | "clear"))
            .filter_map(|name| enzui_theme::WindowFrame::from_name(name))
            .map(|frame| {
                (
                    window_frame_label(frame).to_owned(),
                    Message::Theme(Action::WindowFrame(frame)),
                )
            })
            .collect();
        self.choice_setting(
            "appearance/window_frame",
            window_frame_label(current),
            choices,
        )
    }

    fn button_shape_setting(&self) -> Element<'_, Message> {
        use enzui_theme::RadiusStep;
        let current = self.tokens().shape.button;
        let choices = [RadiusStep::Full, RadiusStep::Md, RadiusStep::None]
            .into_iter()
            .map(|step| {
                (
                    button_shape_label(step).to_owned(),
                    Message::Theme(Action::ButtonShape(step)),
                )
            })
            .collect();
        self.choice_setting(
            "appearance/button_shape",
            button_shape_label(current),
            choices,
        )
    }

    fn focus_ring_setting(&self) -> Element<'_, Message> {
        let current = self.tokens().stroke.focus_ring;
        let label = |width: f32| format!("{} px", enzui_shell::i18n::number(format!("{width:.0}")));
        self.choice_setting(
            "appearance/focus_ring",
            label(current),
            [1.0, 2.0, 3.0, 4.0]
                .into_iter()
                .map(|width| (label(width), Message::Theme(Action::FocusRing(width))))
                .collect(),
        )
    }

    fn theme_file_setting(&self) -> Element<'_, Message> {
        let store = &self.theme;
        let status: String = match store.load() {
            Load::Absent => {
                tr("shell-no-theme-file-yet-choosing-above-creates-one-that-every-enzui-app-reads")
                    .into()
            }
            Load::Unusable(reason) => {
                tr!("shell-this-theme-file-was-left-untouched-because", "arg0" => reason.clone())
            }
            Load::Loaded { .. } => {
                let studio = store.studio_tokens();
                if studio == 0 {
                    tr("shell-shared-with-every-enzui-app-edited-here-and-in-theme-studio").into()
                } else {
                    enzui_shell::i18n::count("theme-studio-kept", studio)
                }
            }
        };

        let row = crate::settings_app::settings_row(tr("shell-theme-file"))
            .icon(enzui_shell::settings_catalog::icon("appearance/theme_file").unwrap_or(""))
            .id_prefix("control-appearance/theme_file")
            .subtitle(status)
            .enabled(self.controls_live())
            .into_element();
        self.mark_setting("appearance/theme_file", row)
    }

    pub(super) fn theme_settings(&self) -> Element<'_, Message> {
        let choices = self.theme_choices();
        let mockup = self.preview_frame(self.theme_mockup(choices));

        let style_card = self
            .section_card_for(
                "appearance/theme",
                column![
                    mockup,
                    self.appearance_mode_setting(),
                    self.style_setting(choices),
                    self.color_setting(choices),
                ]
                .spacing(12),
            )
            .title(enzui_shell::i18n::tr("shell-style-color"))
            .description(tr("shell-one-colour-makes-the-whole-palette-in-light-and-dark-for-the-shell-and-every"))
            .into_element();

        let shape_card = self
            .section_card_for(
                "appearance/corners",
                column![
                    self.shape_preview(),
                    self.window_frame_setting(),
                    self.window_buttons_setting(),
                    self.button_shape_setting(),
                    self.focus_ring_setting(),
                    self.menu_setting(
                        "appearance/corners",
                        choices.corners,
                        Corners::ALL,
                        Corners::label,
                        |corners| Choices { corners, ..choices }
                    ),
                    self.menu_setting(
                        "appearance/density",
                        choices.density,
                        Density::ALL,
                        Density::label,
                        |density| Choices { density, ..choices }
                    ),
                    self.menu_setting(
                        "appearance/shadows",
                        choices.shadows,
                        Shadows::ALL,
                        Shadows::label,
                        |shadows| Choices { shadows, ..choices }
                    ),
                    self.menu_setting(
                        "appearance/scrollbars",
                        choices.scrollbars,
                        Scrollbars::ALL,
                        Scrollbars::label,
                        |scrollbars| Choices {
                            scrollbars,
                            ..choices
                        }
                    ),
                    self.theme_file_setting(),
                ]
                .spacing(4),
            )
            .title(tr("shell-shape-and-spacing"))
            .description(tr(
                "shell-these-apply-to-every-enzui-application-not-just-the-shell",
            ))
            .into_element();

        column![style_card, shape_card, self.app_style_settings()]
            .spacing(32)
            .into()
    }
}

impl SettingsApp {
    pub(super) fn theme_edit(&mut self, value: Edit) -> iced::Task<Message> {
        use enzui_widgets::modal_host::{ModalHostEvent, focus_wrap_operation};
        use iced::Task;
        match value {
            Edit::Custom(true) => {
                let color = self.theme_choices().scheme.seed.color();
                self.theme_custom = Some(Custom {
                    value: Hsva::from_color(color),
                    hex: theme_palette::hex(color),
                    target: Target::Theme,
                });
            }
            Edit::BorderCustom => {
                let color = theme_palette::rgb(self.settings.compositor.border_custom);
                self.theme_custom = Some(Custom {
                    value: Hsva::from_color(color),
                    hex: theme_palette::hex(color),
                    target: Target::Border,
                });
            }
            Edit::ChromeCustom => {
                let color = theme_palette::rgb(self.settings.chrome.custom_color());
                self.theme_custom = Some(Custom {
                    value: Hsva::from_color(color),
                    hex: theme_palette::hex(color),
                    target: Target::Chrome,
                });
            }
            Edit::Custom(false) | Edit::Host(ModalHostEvent::Dismissed(_)) => {
                self.theme_custom = None;
            }
            Edit::Host(event) => {
                return focus_wrap_operation(event)
                    .map_or_else(Task::none, iced::advanced::widget::operate);
            }
            Edit::Pick(value) => {
                if let Some(custom) = &mut self.theme_custom {
                    custom.value = value;
                    custom.hex = theme_palette::hex(value.to_color());
                }
            }
            Edit::Hex(text) => {
                if let Some(custom) = &mut self.theme_custom {
                    custom.hex = text.chars().take(9).collect();
                    if let Some(color) = theme_palette::parse_hex(custom.hex.trim()) {
                        custom.value = Hsva::from_color(color);
                    }
                }
            }
            Edit::UseCustom => {
                if let Some(custom) = self.theme_custom.take() {
                    if custom.target == Target::Chrome {
                        use enzui_shell::chrome_style::{Change as ChromeChange, ChromeColor};
                        let [r, g, b, _] = custom.value.to_color().into_rgba8();
                        let rgb = u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
                        return self.update(Message::Change(
                            enzui_shell::settings::Change::Chrome(ChromeChange::Color(
                                ChromeColor::Custom(rgb),
                            )),
                        ));
                    }
                    if custom.target == Target::Border {
                        use enzui_shell::desktop_preferences::{BorderColor, OptionKey};
                        use enzui_shell::settings::Change;
                        let [r, g, b, _] = custom.value.to_color().into_rgba8();
                        let rgb = i32::from(r) << 16 | i32::from(g) << 8 | i32::from(b);
                        let chosen = self.update(Message::Change(Change::Compositor(
                            OptionKey::BorderCustom,
                            rgb,
                        )));
                        let shown = self.update(Message::Change(Change::Compositor(
                            OptionKey::BorderColor,
                            BorderColor::ALL
                                .iter()
                                .position(|c| *c == BorderColor::Custom)
                                .unwrap_or_default() as i32,
                        )));
                        return Task::batch([chosen, shown]);
                    }
                    return self.choose_seed(Seed::Color(custom.value.to_color()));
                }
            }
            Edit::FromWallpaper => return self.read_wallpaper_colour(),
            Edit::WallpaperRead(read, color) => {
                if let (true, Some(color)) = (read == self.theme_wallpaper.1, color) {
                    return self.choose_seed(Seed::Wallpaper(color));
                }
            }
        }
        Task::none()
    }

    fn choose_seed(&mut self, seed: Seed) -> iced::Task<Message> {
        let choices = self.theme_choices();
        if choices.scheme.seed == seed {
            return iced::Task::none();
        }
        self.update(Message::Theme(Action::Choose {
            before: choices,
            after: Choices {
                scheme: Scheme {
                    seed,
                    ..choices.scheme
                },
                ..choices
            },
        }))
    }

    fn wallpaper_source(&self) -> (String, Option<iced::Color>) {
        use super::wallpaper_settings::preview::PreviewVisual;
        match self.wallpaper.preview_visual(&self.settings.wallpaper) {
            PreviewVisual::Image(path) => (path, None),
            PreviewVisual::Color(color) => (theme_palette::hex(color), Some(color)),
        }
    }

    fn read_wallpaper_colour(&mut self) -> iced::Task<Message> {
        let (key, color) = self.wallpaper_source();
        self.theme_wallpaper.0 = Some(key.clone());
        self.theme_wallpaper.1 += 1;
        let read = self.theme_wallpaper.1;
        if let Some(color) = color {
            return iced::Task::done(edit(Edit::WallpaperRead(read, Some(color))));
        }
        iced::Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    theme_palette::file_seed(std::path::Path::new(&key))
                })
                .await
                .ok()
                .flatten()
            },
            move |color| edit(Edit::WallpaperRead(read, color)),
        )
    }

    pub(super) fn follow_wallpaper_colour(&mut self) -> iced::Task<Message> {
        if !matches!(self.theme_choices().scheme.seed, Seed::Wallpaper(_))
            || !self.wallpaper.supported
            || self.closing
        {
            return iced::Task::none();
        }
        let (key, _) = self.wallpaper_source();
        if self.theme_wallpaper.0.as_ref() == Some(&key) {
            return iced::Task::none();
        }
        self.read_wallpaper_colour()
    }
}

pub(super) fn update(
    store: &mut enzui_shell::theme_store::Store,
    action: Action,
) -> Result<(), String> {
    match action {
        Action::Choose { before, after } => {
            store.stage_from(before, after);
            store.save()
        }
        Action::WindowButtons(buttons) => {
            if !store.stage_window_buttons(buttons) {
                return Err(tr("shell-theme-controls-refused").into());
            }
            store.save()
        }
        Action::FocusRing(width) => {
            if !store.stage_focus_ring(width) {
                return Err(tr("shell-theme-controls-refused").into());
            }
            store.save()
        }
        Action::WindowRadius(radius) => {
            if !store.stage_window_radius(radius) {
                return Err(tr("shell-theme-controls-refused").into());
            }
            store.save()
        }
        Action::WindowFrame(frame) => {
            if !store.stage_window_frame(frame) {
                return Err(tr("shell-theme-controls-refused").into());
            }
            store.save()
        }
        Action::ButtonShape(step) => {
            if !store.stage_button_shape(step) {
                return Err(tr("shell-theme-controls-refused").into());
            }
            store.save()
        }
        Action::ReducedMotion(reduced) => {
            if !store.stage_reduced_motion(reduced) {
                return Err(tr("shell-reduced-motion-could-not-be-set-2").into());
            }
            store.save()
        }
    }
}

fn window_frame_label(frame: enzui_theme::WindowFrame) -> &'static str {
    match frame.name() {
        "opaque" => tr("shell-window-frame-opaque"),
        "frosted" => tr("shell-window-frame-frosted"),
        "flat_frosted" => tr("shell-window-frame-flat-frosted"),
        "subtle" => tr("shell-window-frame-subtle"),
        "glass" => tr("shell-window-frame-glass"),
        "clear" => tr("shell-window-frame-clear"),
        _ => tr("shell-window-frame-app"),
    }
}

fn button_shape_label(step: enzui_theme::RadiusStep) -> &'static str {
    match step {
        enzui_theme::RadiusStep::Full => tr("shell-button-shape-pill"),
        enzui_theme::RadiusStep::None => tr("shell-square"),
        _ => tr("shell-rounded"),
    }
}

fn window_buttons_label(buttons: enzui_theme::WindowButtons) -> &'static str {
    use enzui_theme::WindowButtons;
    tr(match buttons {
        WindowButtons::Desktop => "shell-window-buttons-desktop",
        WindowButtons::Close => "shell-window-buttons-close",
        WindowButtons::MinimizeClose => "shell-window-buttons-minimize-close",
        WindowButtons::MaximizeClose => "shell-window-buttons-maximize-close",
        WindowButtons::All => "shell-window-buttons-all",
    })
}

// Content that belongs to a row title starts in the card's text column.
fn under_row_title() -> iced::Padding {
    let tokens = crate::settings_app::settings_row_tokens();
    iced::Padding::ZERO
        .left(tokens.padding_x + tokens.icon_slot + tokens.gap)
        .right(12.0)
        .bottom(8.0)
}
