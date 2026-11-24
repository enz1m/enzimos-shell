use super::{Message, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    desktop_preferences::OptionKey,
    settings::{BarEdge, Change, Metric, SidebarEdge},
    settings_catalog::{self, Category},
    shell_style::{IslandOption, ShellStyle},
};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    icon::enz_icon,
    pressable::{PressableStyle, enz_pressable},
    scroll_view::enz_scroll_view,
    text::{TextSize, enz_text},
};
use iced::{
    Element, Length,
    widget::{Column, Space, column, container, row},
};

impl SettingsApp {
    fn metric_fields<'a>(&'a self, metrics: &[Metric]) -> Element<'a, Message> {
        let mut fields = column![].spacing(2);
        for &metric in metrics {
            let key = settings_catalog::find(&format!("sizing/{}", metric.key()))
                .expect("the settings catalog contains each built-in setting")
                .route;
            let (min, max) = metric.bounds();
            fields = fields.push(self.number_setting(
                key,
                self.settings.metrics.get(metric).into(),
                min.into(),
                max.into(),
                " px",
                move |value| Message::Change(Change::Metric(metric, value as u16)),
            ));
        }
        fields.into()
    }
    pub(super) fn appearance_mode_setting(&self) -> Element<'_, Message> {
        let entry = settings_catalog::find("appearance/theme").expect("Registered color scheme");
        self.mark_setting(
            entry.route,
            // A switch has to name what being on means, so the row is "Dark
            // mode" rather than the section's own "Color scheme" caption.
            crate::settings_app::settings_row(tr("shell-dark-mode"))
                .icon(settings_catalog::icon(entry.route).unwrap_or(""))
                .id_prefix("control-appearance/theme")
                // The switch shows the mode the desktop is drawn in, the
                // same source the preview uses; a fresh profile's shell file
                // can disagree with the theme store until the first change.
                .switch(
                    self.theme_mode() == enzui_theme::EnzThemeMode::Dark,
                    |dark| Message::Change(Change::Light(!dark)),
                )
                .enabled(!self.closing)
                .into_element(),
        )
    }
    fn text_size_setting(&self) -> Element<'_, Message> {
        use enzui_shell::settings::TextStep;
        let selected = TextStep::nearest(self.settings.metrics.text);
        self.choice_setting(
            "sizing/text_size",
            selected.description(),
            TextStep::ALL
                .into_iter()
                .map(|step| {
                    (
                        step.description().into(),
                        Message::Change(Change::Metric(Metric::Text, step.px())),
                    )
                })
                .collect(),
        )
    }
    fn inspected_shell_style(&self) -> ShellStyle {
        match self.highlighted {
            Some(key) if key.starts_with("appearance/islands_") => ShellStyle::Islands,
            Some(
                "sizing/frame_width"
                | "sizing/corner_radius"
                | "sizing/bar_height"
                | "sizing/sidebar_width",
            ) => ShellStyle::Frame,
            _ => self.settings.shell_style,
        }
    }
    fn appearance_settings(&self) -> Element<'_, Message> {
        let mut body = column![self.theme_settings()].spacing(32);
        if self.wallpaper.supported {
            body = body.push(
                self.section_card_for("wallpaper/source", self.wallpaper_settings())
                    .title(tr("shell-wallpaper"))
                    .description(tr("shell-choose-the-background-for-this-desktop"))
                    .into_element(),
            );
        }
        body.push(
            self.section_card_for(
                "sizing/text_size",
                column![
                    self.text_size_setting(),
                    self.icon_theme_setting(),
                    self.metric_fields(&[Metric::Icon]),
                    self.typography_preview(),
                ]
                .spacing(2),
            )
            .title(tr("shell-text-icons"))
            .description(tr(
                "shell-scales-every-text-size-the-desktop-draws-from-captions-to-headings",
            ))
            .into_element(),
        )
        .push(
            self.section_card_for("fonts/ui", self.font_settings())
                .title(tr("shell-fonts"))
                .description(tr(
                    "shell-typefaces-and-weight-shared-by-the-desktop-and-enzui-applications",
                ))
                .into_element(),
        )
        .push(self.lock_clock_settings())
        .into()
    }
    fn shell_settings(&self) -> Element<'_, Message> {
        let inspected = self.inspected_shell_style();
        let details = match inspected {
            ShellStyle::Frame => {
                self.metric_fields(&[Metric::Frame, Metric::Radius, Metric::Bar, Metric::Rail])
            }
            ShellStyle::Islands => {
                let mut fields = column![self.switch_setting(
                    "appearance/islands_joined_bar",
                    self.settings.islands.joined_bar,
                    |joined| Message::Change(Change::IslandsJoinedBar(joined)),
                )]
                .spacing(2);
                for key in IslandOption::ALL {
                    let route =
                        settings_catalog::find(&format!("appearance/islands_{}", key.key()))
                            .expect("the settings catalog contains each built-in setting")
                            .route;
                    let (min, max) = key.bounds();
                    fields = fields.push(self.number_setting(
                        route,
                        self.settings.islands.get(key).into(),
                        min.into(),
                        max.into(),
                        " px",
                        move |value| Message::Change(Change::Island(key, value as u16)),
                    ));
                }
                fields = fields.push(self.number_setting(
                    "appearance/islands_fullscreen_padding",
                    self.settings.sidebar_behavior.fullscreen_padding.into(),
                    0,
                    enzui_shell::sidebar_preferences::Preferences::MAX_PADDING.into(),
                    " px",
                    |value| Message::Change(Change::FullscreenPadding(value as u16)),
                ));
                fields.into()
            }
        };
        let explanation = if inspected != self.settings.shell_style {
            tr("shell-inactive-style-settings-select-this-style-above-to-apply-them")
        } else if inspected == ShellStyle::Frame {
            tr("shell-a-continuous-desktop-frame-with-joined-panels-sizes-are-logical-pixels")
        } else if self.settings.islands.joined_bar {
            tr("shell-one-connected-floating-bar-with-clear-space-around-it-sizes-are-logical-pixe")
        } else {
            tr("shell-separate-floating-pills-with-clear-space-around-them-sizes-are-logical-pixel")
        };
        let mut style = column![self.shell_style_choices()].spacing(8);
        if !self.islands_supported {
            style = style.push(
                container(
                    enz_text(tr("shell-islands-requires-a-newly-prepared-native-shell-existing-desktops-are-not-res"))
                        .size(TextSize::Caption)
                        .muted_on(self.tokens().palette.surface),
                )
                .padding([0, 12]),
            );
        }
        let body = column![
            self.section_card_for("appearance/shell_style", style)
                .title(tr("shell-shell-style"))
                .description(tr(
                    "shell-choose-the-desktop-s-shape-illustrations-use-your-current-sizing"
                ))
                .into_element(),
        ]
        .spacing(32);
        let reset = row![
            Space::new().width(Length::Fill),
            enz_button(
                enzui_shell::tr!("shell-reset-2", "arg0" => enzui_shell::i18n::argument(&(inspected.label()))),
                Message::Change(if inspected == ShellStyle::Frame {
                    Change::ResetFrameSizing
                } else {
                    Change::ResetIslandSizing
                }),
            )
            .id("shell-style-reset".into())
            .kind(ButtonKind::Quiet)
            .icon("rotate-ccw")
            .enabled(!self.closing)
            .into_element(),
        ];
        body.push(
            self.section_card_for(
                if inspected == ShellStyle::Frame {
                    "sizing/frame_width"
                } else {
                    "appearance/islands_height"
                },
                column![details, reset].spacing(20),
            )
            .title(if inspected == ShellStyle::Frame {
                tr("shell-frame-size")
            } else {
                tr("shell-islands-size")
            })
            .description(explanation)
            .into_element(),
        )
        .into()
    }
    // The shell's colour, opacity, material and glass. Every row is built
    // (search finds each), and shown only where it applies: the material
    // only for islands, glass only on glass, opacity only on solid chrome.
    fn chrome_settings(&self, islands: bool) -> Element<'_, Message> {
        use enzui_shell::chrome_style::{
            Change as ChromeChange, ChromeColor, GLASS_BENDING, GLASS_BENDING_DEFAULT, GLASS_BLUR,
            GLASS_HIGHLIGHT, GLASS_HIGHLIGHT_DEFAULT, GLASS_TINT, OPACITY,
        };
        let style = self.settings.chrome;
        let change = |value: ChromeChange| Message::Change(Change::Chrome(value));
        let color = self.choice_setting(
            "shell/chrome_color",
            style.color.label(),
            [
                ChromeColor::Theme,
                ChromeColor::Accent,
                ChromeColor::Custom(style.custom_color()),
            ]
            .into_iter()
            .map(|color| (color.label().into(), change(ChromeChange::Color(color))))
            .collect(),
        );
        let custom = {
            let route = "shell/chrome_custom";
            let entry = settings_catalog::find(route)
                .expect("the settings catalog contains each built-in setting");
            let color = enzui_shell::theme_palette::rgb(style.custom_color());
            let edit = super::theme_settings::edit(super::theme_settings::Edit::ChromeCustom);
            self.mark_setting(
                route,
                crate::settings_app::settings_row(entry.title())
                    .icon(settings_catalog::icon(route).unwrap_or(""))
                    .id_prefix(format!("control-{route}"))
                    .control(
                        row![
                            enz_text(enzui_shell::theme_palette::hex(color))
                                .tone(enzui_widgets::text::TextTone::Muted)
                                .into_element(),
                            enzui_widgets::color_swatch::enz_color_swatch(color, edit)
                                .size(26.0)
                                .radius(13.0)
                                .aria_label(entry.title())
                                .id_prefix("shell-chrome-custom")
                                .enabled(self.controls_live())
                                .into_element(),
                        ]
                        .spacing(10)
                        .align_y(iced::Alignment::Center),
                    )
                    .into_element(),
            )
        };
        let material = self.choice_setting(
            "shell/material",
            if self.settings.panel_glass == 0 {
                tr("shell-material-solid")
            } else {
                tr("shell-frosted-glass-2")
            },
            vec![
                (
                    tr("shell-material-solid").into(),
                    Message::Change(Change::PanelGlass(0)),
                ),
                (
                    tr("shell-frosted-glass-2").into(),
                    Message::Change(Change::PanelGlass(1)),
                ),
            ],
        );
        let glass = islands && self.settings.panel_glass != 0;
        // The material's own values stand until the person moves a slider.
        let own = enzui_shell::reveal::chrome_material(self.settings.panel_glass).map(|material| {
            enzui_shell::surface_input::shell_material(material, self.settings.light)
        });
        let percent = |value: f32, (min, max): (u8, u8)| {
            ((value * 100.0).round() as i32).clamp(i32::from(min), i32::from(max))
        };
        let opacity = self.number_setting(
            "shell/chrome_opacity",
            style.opacity.into(),
            OPACITY.0.into(),
            OPACITY.1.into(),
            "%",
            move |value| change(ChromeChange::Opacity(value as u8)),
        );
        let tint = self.number_setting(
            "shell/glass_tint",
            style.glass_tint.map_or_else(
                || percent(own.map_or(0.45, |own| own.tint_opacity), GLASS_TINT),
                i32::from,
            ),
            GLASS_TINT.0.into(),
            GLASS_TINT.1.into(),
            "%",
            move |value| change(ChromeChange::GlassTint(Some(value as u8))),
        );
        let blur = self.number_setting(
            "shell/glass_blur",
            style.glass_blur.map_or_else(
                || {
                    (own.map_or(0.0, |own| own.blur_sigma).round() as i32)
                        .clamp(GLASS_BLUR.0.into(), GLASS_BLUR.1.into())
                },
                i32::from,
            ),
            GLASS_BLUR.0.into(),
            GLASS_BLUR.1.into(),
            "px",
            move |value| change(ChromeChange::GlassBlur(Some(value as u8))),
        );
        let bending = self.number_setting(
            "shell/glass_bending",
            style.glass_bending.unwrap_or(GLASS_BENDING_DEFAULT).into(),
            GLASS_BENDING.0.into(),
            GLASS_BENDING.1.into(),
            "%",
            move |value| change(ChromeChange::GlassBending(Some(value as u8))),
        );
        let highlight = self.number_setting(
            "shell/glass_highlight",
            style
                .glass_highlight
                .unwrap_or(GLASS_HIGHLIGHT_DEFAULT)
                .into(),
            GLASS_HIGHLIGHT.0.into(),
            GLASS_HIGHLIGHT.1.into(),
            "%",
            move |value| change(ChromeChange::GlassHighlight(Some(value as u8))),
        );
        column![color]
            .push(matches!(style.color, ChromeColor::Custom(_)).then_some(custom))
            .push(islands.then_some(material))
            .push((!glass).then_some(opacity))
            .push(glass.then_some(tint))
            .push(glass.then_some(blur))
            .push(glass.then_some(bending))
            .push(glass.then_some(highlight))
            .spacing(2)
            .into()
    }
    fn shell_page<'a>(&'a self, body: Column<'a, Message>, enabled: bool) -> Column<'a, Message> {
        let islands = self.settings.shell_style == ShellStyle::Islands;
        // Islands have no large shadows: round a small island it read as a
        // dark smudge, and round a panel it was cut off square.
        let (bar_shadow_max, panel_shadow_max) = if islands {
            (
                enzui_shell::desktop_frame::ISLAND_SHADOW_MAX,
                enzui_shell::reveal::ISLAND_PANEL_SHADOW_MAX,
            )
        } else {
            (4, 4)
        };
        let placement = column![
            self.placement_preview(),
            self.choice_setting(
                "shell/bar",
                self.settings.bar.label(),
                BarEdge::ALL
                    .into_iter()
                    .map(|edge| (edge.label().into(), Message::Change(Change::Bar(edge))))
                    .collect(),
            ),
            self.choice_setting(
                "shell/bar_shadow",
                enzui_shell::reveal::shadow_level(self.settings.bar_shadow.min(bar_shadow_max)),
                (0..=bar_shadow_max)
                    .map(|level| (
                        enzui_shell::reveal::shadow_level(level).into(),
                        Message::Change(Change::BarShadow(level))
                    ))
                    .collect(),
            ),
            self.choice_setting(
                "shell/panel_shadow",
                enzui_shell::reveal::shadow_level(self.settings.panel_shadow.min(panel_shadow_max)),
                (0..=panel_shadow_max)
                    .map(|level| (
                        enzui_shell::reveal::shadow_level(level).into(),
                        Message::Change(Change::PanelShadow(level))
                    ))
                    .collect(),
            ),
            self.chrome_settings(islands),
            self.switch_setting(
                "shell/activity_status",
                self.settings.activity_status,
                |enabled| { Message::Change(Change::ActivityStatus(enabled)) }
            ),
            self.switch_setting(
                "shell/sidebar_enabled",
                self.settings.sidebar_enabled,
                |enabled| { Message::Change(Change::SidebarEnabled(enabled)) }
            ),
            self.dependent(self.settings.sidebar_enabled, || self.choice_setting(
                "shell/sidebar",
                self.settings.sidebar.label(),
                SidebarEdge::ALL
                    .into_iter()
                    .map(|edge| (edge.label().into(), Message::Change(Change::Sidebar(edge))))
                    .collect(),
            )),
            self.dependent(self.settings.sidebar_enabled, || self.choice_setting(
                "shell/sidebar_pin_scope",
                self.settings.sidebar_pin_scope.label(),
                enzui_shell::sidebar_preferences::PinScope::ALL
                    .into_iter()
                    .map(|scope| (
                        scope.label().into(),
                        Message::Change(Change::SidebarPinScope(scope))
                    ))
                    .collect(),
            )),
        ]
        .spacing(2);
        let placement = if self.inspected_shell_style() == ShellStyle::Islands {
            placement.push(self.dependent(self.settings.sidebar_enabled, || {
                self.choice_setting(
                    "appearance/islands_sidebar_behavior",
                    self.settings.sidebar_behavior.visibility.label(),
                    enzui_shell::sidebar_preferences::Visibility::ALL
                        .into_iter()
                        .map(|mode| {
                            (
                                mode.label().into(),
                                Message::Change(Change::SidebarVisibility(mode)),
                            )
                        })
                        .collect(),
                )
            }))
        } else {
            placement
        };
        let assistant = column![
            self.switch_setting(
                "shell/assistant_enabled",
                self.settings.assistant_enabled,
                |enabled| { Message::Change(Change::AssistantEnabled(enabled)) }
            ),
            self.dependent(self.settings.assistant_enabled, || self.choice_setting(
                "shell/assistant_corner",
                self.settings.assistant_corner.label(),
                enzui_shell::settings::AssistantCorner::ALL
                    .into_iter()
                    .map(|corner| (
                        corner.label().into(),
                        Message::Change(Change::AssistantCorner(corner))
                    ))
                    .collect(),
            )),
        ]
        .spacing(2);
        let mut body = body.push(self.shell_settings()).push(
            self.section_card_for("shell/bar", placement)
                .title(tr("shell-placement"))
                .description(tr("shell-which-screen-edge-the-bar-and-the-miniapp-sidebar-use"))
                .into_element(),
        ).push(
            self.section_card_for("shell/assistant_enabled", assistant)
                .title(tr("shell-ai-assistant"))
                .description(tr("shell-a-button-that-appears-when-you-point-at-a-screen-corner-and-opens-the-ai-ass"))
                .into_element(),
        );
        if self.layout_preview {
            body = body.push(
                self.section_card_for(
                    "bar-layout/items",
                    // No highlight: it shows through the editor only in patches.
                    self.anchor_setting(
                        "bar-layout/items",
                        enzui_shell::settings_layout::view(
                            &self.settings.layout,
                            &self.plugins.names(),
                            self.tokens(),
                            enabled && !self.resetting_layout,
                            !self.busy && !self.plugins.busy(),
                            &self.layout_drag,
                            Message::Layout,
                        ),
                    ),
                )
                .title(tr("shell-bar-contents"))
                .description(tr(
                    "shell-drag-items-to-arrange-the-bar-or-into-the-tray-below-it-to-hide-them",
                ))
                .into_element(),
            );
        }
        let body = body.push(
            self.section_card_for("controls/network", self.control_editor())
                .title(tr("shell-quick-controls"))
                .description(tr("shell-arrange-the-controls-shown-in-control-center"))
                .into_element(),
        );
        if self.layout_preview {
            body.push(
                self.section_card_for("plugins/installed", self.plugins_view())
                    .title(tr("shell-plugins"))
                    .description(tr("shell-extensions-that-add-bar-items-and-search-results"))
                    .into_element(),
            )
        } else {
            body
        }
    }
    // Keyed by catalog route so Windows and Input split one compositor
    // preference set without either page hardcoding the other's rows.
    pub(super) fn compositor_rows(
        &self,
    ) -> std::collections::HashMap<&'static str, Element<'_, Message>> {
        OptionKey::ALL
            .into_iter()
            .map(|key| (Self::compositor_route(key), self.compositor_row(key)))
            .collect()
    }
    fn compositor_route(key: OptionKey) -> &'static str {
        settings_catalog::find(&format!("windows/{}", key.key()))
            .expect("the settings catalog contains each built-in setting")
            .route
    }
    pub(super) fn record_liveness(&self, route: &str) {
        #[cfg(test)]
        self.route_liveness
            .borrow_mut()
            .insert(route.to_owned(), self.controls_live());
        let _ = route;
    }
    // One row on its own, so a page can build it inside `dependent`.
    pub(super) fn compositor_row(&self, key: OptionKey) -> Element<'_, Message> {
        let entry = settings_catalog::find(Self::compositor_route(key))
            .expect("the settings catalog contains each built-in setting");
        self.record_liveness(entry.route);
        let value = self.settings.compositor.value(key);
        if key == OptionKey::AnimationRate && !self.motion_supported {
            self.mark_setting(entry.route, crate::settings_app::settings_row(tr("shell-animation-speed"))
                .icon(settings_catalog::icon(entry.route).unwrap_or(""))
                .subtitle(tr("shell-requires-a-current-managed-desktop-this-session-will-not-be-restarted"))
                .subtitle_wrap(true)
                .control(enz_text(format!("{value}%")).into_element())
                .enabled(false).into_element())
        } else if key == OptionKey::AnimationStyle {
            self.choice_setting(
                entry.route,
                enzui_shell::motion::style_label(value as u8),
                (0..=3)
                    .map(|style| {
                        (
                            enzui_shell::motion::style_label(style).into(),
                            Message::Change(Change::Compositor(key, i32::from(style))),
                        )
                    })
                    .collect(),
            )
        } else if key == OptionKey::BorderColor {
            use enzui_shell::desktop_preferences::BorderColor;
            self.choice_setting(
                entry.route,
                BorderColor::from_value(value).label(),
                BorderColor::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, color)| {
                        (
                            color.label().into(),
                            Message::Change(Change::Compositor(key, index as i32)),
                        )
                    })
                    .collect(),
            )
        } else if key == OptionKey::BorderCustom {
            let color = enzui_shell::theme_palette::rgb(value as u32);
            let edit = super::theme_settings::edit(super::theme_settings::Edit::BorderCustom);
            self.mark_setting(
                entry.route,
                crate::settings_app::settings_row(entry.title())
                    .icon(settings_catalog::icon(entry.route).unwrap_or(""))
                    .id_prefix(format!("control-{}", entry.route))
                    .control(
                        row![
                            enz_text(enzui_shell::theme_palette::hex(color))
                                .tone(enzui_widgets::text::TextTone::Muted)
                                .into_element(),
                            enzui_widgets::color_swatch::enz_color_swatch(color, edit)
                                .size(26.0)
                                .radius(13.0)
                                .aria_label(entry.title())
                                .id_prefix("windows-border-custom")
                                .enabled(self.controls_live())
                                .into_element(),
                        ]
                        .spacing(10)
                        .align_y(iced::Alignment::Center),
                    )
                    .into_element(),
            )
        } else if key == OptionKey::CapsLock {
            use enzui_shell::desktop_preferences::CapsLock;
            self.choice_setting(
                entry.route,
                CapsLock::from_value(value).label(),
                CapsLock::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        (
                            mode.label().into(),
                            Message::Change(Change::Compositor(key, index as i32)),
                        )
                    })
                    .collect(),
            )
        } else if key == OptionKey::LayoutSwitch {
            use enzui_shell::desktop_preferences::LayoutSwitch;
            self.choice_setting(
                entry.route,
                LayoutSwitch::from_value(value).label(),
                LayoutSwitch::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, mode)| {
                        (
                            mode.label().into(),
                            Message::Change(Change::Compositor(key, index as i32)),
                        )
                    })
                    .collect(),
            )
        } else if key == OptionKey::LeftHanded {
            self.choice_setting(
                entry.route,
                if value != 0 {
                    tr("shell-right")
                } else {
                    tr("shell-left")
                },
                vec![
                    (
                        tr("shell-left").into(),
                        Message::Change(Change::Compositor(key, 0)),
                    ),
                    (
                        tr("shell-right").into(),
                        Message::Change(Change::Compositor(key, 1)),
                    ),
                ],
            )
        } else if key.toggle() {
            self.switch_setting(entry.route, value != 0, move |value| {
                Message::Change(Change::Compositor(key, i32::from(value)))
            })
        } else {
            let (min, max) = key.bounds();
            self.number_setting(
                entry.route,
                value,
                min,
                max,
                match key {
                    OptionKey::PointerSpeed
                    | OptionKey::AnimationRate
                    | OptionKey::ScrollSpeed
                    | OptionKey::BorderOpacity
                    | OptionKey::InactiveBorderOpacity
                    | OptionKey::Zoom => "%",
                    OptionKey::RepeatDelay => " ms",
                    OptionKey::RepeatRate => " /s",
                    _ => " px",
                },
                move |value| Message::Change(Change::Compositor(key, value)),
            )
        }
    }
    pub(super) fn desktop_status_note(&self) -> Option<Element<'_, Message>> {
        self.desktop_status.as_ref().map(|error| {
            enz_text(tr!("shell-saved-but-could-not-apply-retrying", "arg0" => error.clone()))
                .into_element()
        })
    }
    fn windows_page<'a>(&'a self, body: Column<'a, Message>) -> Column<'a, Message> {
        let mut rows = self.compositor_rows();
        let mut take = |route: &'static str| {
            rows.remove(route)
                .expect("the settings catalog contains each built-in setting")
        };
        let layout = column![
            self.layout_choices(),
            take("windows/gaps_inner"),
            take("windows/gaps_outer"),
        ]
        .spacing(2);
        let appearance = column![
            self.window_appearance_preview(),
            take("windows/border_size"),
            take("windows/border_color"),
        ]
        .push({
            // The picker's row only while the custom colour is chosen.
            let custom = take("windows/border_custom");
            (enzui_shell::desktop_preferences::BorderColor::from_value(
                self.settings.compositor.border_color.into(),
            ) == enzui_shell::desktop_preferences::BorderColor::Custom)
                .then_some(custom)
        })
        .push(
            column![
                take("windows/border_opacity"),
                take("windows/border_inactive_opacity"),
                take("windows/rounding"),
                take("windows/animations"),
                take("windows/animation_rate"),
                take("windows/animation_style"),
            ]
            .spacing(2),
        )
        .spacing(2);
        let behavior = column![
            take("windows/focus_pointer"),
            self.switch_setting("windows/restore_apps", self.settings.restore_apps, |on| {
                Message::Change(Change::RestoreApps(on))
            }),
        ]
        .spacing(2);
        let body = body
            .push(
                self.section_card_for("windows/tiling", layout)
                    .title(tr("shell-tiling-gaps"))
                    .description(tr("shell-gaps-apply-per-window-edge-in-logical-pixels"))
                    .into_element(),
            )
            .push(
                self.section_card_for("windows/border_size", appearance)
                    .title(tr("shell-appearance-motion"))
                    .description(tr("shell-borders-and-corners-drawn-around-application-windows-and-how-they-animate-sp"))
                    .into_element(),
            )
            .push(
                self.section_card_for("windows/focus_pointer", behavior)
                    .title(tr("shell-behavior"))
                    .into_element(),
            );
        match self.desktop_status_note() {
            Some(note) => body.push(note),
            None => body,
        }
    }
    pub(super) fn settings_page(&self) -> Element<'_, Message> {
        let enabled = !self.closing;
        let mut body = column![].spacing(32);
        if !self.filter.trim().is_empty() {
            body = column![
                enz_text(tr("shell-search-results"))
                    .size(TextSize::Title2)
                    .into_element()
            ]
            .spacing(8);
            let entries: Vec<_> = settings_catalog::search(&self.filter)
                .into_iter()
                .filter(|entry| {
                    self.layout_preview
                        || !matches!(entry.page(), "plugins" | "notifications" | "bar-layout")
                })
                .filter(|entry| entry.page() != "wallpaper" || self.wallpaper.supported)
                .filter(|entry| self.route_shown(*entry))
                .collect();
            if entries.is_empty() {
                body = body.push(
                    enz_text(tr("shell-no-matching-settings"))
                        .muted_on(self.tokens().palette.surface)
                        .into_element(),
                );
            }
            for entry in entries {
                body = body.push(
                    enz_pressable(
                        container(
                            row![
                                enz_icon(entry.category().icon()).size(20.0).into_element(),
                                column![
                                    enz_text(entry.title()).into_element(),
                                    enz_text(format!(
                                        "{} · {}",
                                        entry.category().title(),
                                        entry.section()
                                    ))
                                    .size(TextSize::Caption)
                                    .muted_on(self.tokens().palette.surface)
                                    .into_element()
                                ]
                                .spacing(3),
                            ]
                            .spacing(12)
                            .align_y(iced::Alignment::Center),
                        )
                        .padding(8)
                        .width(Length::Fill)
                        .into(),
                        Message::Reveal(entry.route),
                    )
                    .style(PressableStyle::Row)
                    .id(format!("find-setting-{}", entry.route).into())
                    .into_element(),
                );
            }
        } else {
            body = body.push(
                enz_text(self.page.title())
                    .size(TextSize::Title1)
                    .heading(enzui_core::semantics::HeadingLevel::Page)
                    .into_element(),
            );
            body = match self.page {
                Category::Appearance => body.push(self.appearance_settings()),
                Category::Users => {
                    let body = body.push(self.users_page());
                    match self.fingerprint_card() {
                        Some(card) => body.push(card),
                        None => body,
                    }
                }
                Category::Shell => self.shell_page(body, enabled),
                Category::Windows => self.windows_page(body),
                Category::Search => body.push(self.quick_search_settings()),
                Category::Notifications => body.push(self.notification_settings()),
                Category::Capture => self.capture_page(body),
                _ => self.system_page(body),
            };
        }
        let scroll = enz_scroll_view(
            container(enzui_widgets::keyed_column::keyed_column(vec![(
                format!("{:?}:{}", self.page, self.filter),
                body.into(),
            )]))
            .padding(24)
            .width(Length::Fill),
        )
        .id("settings-scroll".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .on_vertical_scroll(|_| Message::PageScrolled)
        .initial_offset(iced::widget::scrollable::AbsoluteOffset {
            x: 0.0,
            y: self.navigation.scroll.get().y,
        })
        .publish_translation(self.navigation.scroll.clone())
        .backdrop(self.tokens().palette.background)
        .into_element();
        enzui_widgets::keyed_column::keyed_column(vec![(
            self.navigation.scroll_generation.to_string(),
            scroll,
        )])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
