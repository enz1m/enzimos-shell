use super::{Message, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::session_preferences::{ClockSize, LockLayout, LockMaterial, SessionChange};
use enzui_shell::tr;
use enzui_widgets::{
    anchored_overlay::dismissible_anchored_menu,
    button::{ButtonKind, ButtonSize, enz_button},
    focus_button::{FocusStatus, focus_button},
    icon::enz_icon,
    keyboard_capture::keyboard_capture_unscoped,
    menu::enz_menu,
    text::{TextAlign, TextTone, enz_text},
};
use iced::{
    Element, Length, Task,
    widget::{column, container, row, svg},
};

fn change(value: SessionChange) -> Message {
    Message::Change(enzui_shell::settings::Change::Session(value))
}

impl SettingsApp {
    fn lock_background_drop<'a>(&'a self, content: Element<'a, Message>) -> Element<'a, Message> {
        enzui_widgets::file_drop::enz_file_drop_target_with_receipt(
            content,
            Message::LockBackgroundDropped,
        )
        .enabled(
            self.controls_live()
                && !self.wallpaper.pending()
                && self.wallpaper.native_drop.is_none(),
        )
        .copy_only(true)
        .key(("lock-background", self.wallpaper.gallery_generation))
        .into()
    }

    pub(super) fn lock_clock_settings(&self) -> Element<'_, Message> {
        let p = &self.settings.session.lock_clock;
        enzui_shell::lock_fonts::load();
        let rows = column![
            self.lock_mockup(),
            self.choice_setting(
                "datetime/lock_style",
                p.material().label(),
                LockMaterial::ALL
                    .into_iter()
                    .map(|m| (m.label().into(), change(SessionChange::LockMaterial(m))))
                    .collect()
            ),
            self.choice_setting(
                "datetime/lock_layout",
                p.layout.label(),
                LockLayout::ALL
                    .into_iter()
                    .map(|l| (l.label().into(), change(SessionChange::LockLayout(l))))
                    .collect()
            ),
            self.choice_setting(
                "datetime/lock_size",
                p.size.label(),
                ClockSize::ALL
                    .into_iter()
                    .map(|s| (s.label().into(), change(SessionChange::LockSize(s))))
                    .collect()
            ),
            self.mark_setting(
                "datetime/lock_font",
                super::settings_row(enzui_shell::i18n::tr("shell-font"))
                    .icon("type")
                    .control(self.lock_font_select())
                    .into_element()
            ),
        ]
        .spacing(4);
        let mut rows = rows;
        let weights = enzui_shell::lock_fonts::weights(p.font_family());
        rows = rows.push(
            self.choice_setting(
                "datetime/lock_weight",
                weight_label(p.weight),
                weights
                    .into_iter()
                    .map(|w| (weight_label(w), change(SessionChange::LockWeight(w))))
                    .collect(),
            ),
        );
        rows = rows.push(self.switch_setting("datetime/lock_time", p.show_time, |v| {
            change(SessionChange::LockTime(v))
        }));
        rows = rows.push(self.switch_setting("datetime/lock_date", p.show_date, |v| {
            change(SessionChange::LockDate(v))
        }));
        rows = rows.push(
            self.lock_background_drop(
                self.mark_setting(
                    "datetime/lock_background",
                    super::settings_row(tr("shell-background"))
                        .subtitle(if p.background.is_some() {
                            tr("shell-custom-image")
                        } else {
                            tr("shell-follow-desktop-wallpaper")
                        })
                        .icon("image")
                        .control(
                            row![
                                enz_button(
                                    enzui_shell::i18n::tr("shell-desktop"),
                                    change(SessionChange::LockBackground(None))
                                )
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Sm)
                                .enabled(self.controls_live() && p.background.is_some())
                                .into_element(),
                                enz_button(
                                    enzui_shell::i18n::tr("shell-image-2"),
                                    Message::LockBackgroundChoose
                                )
                                .kind(ButtonKind::Secondary)
                                .size(ButtonSize::Sm)
                                .enabled(self.controls_live() && !self.wallpaper.pending())
                                .into_element(),
                            ]
                            .spacing(6),
                        )
                        .into_element(),
                ),
            ),
        );
        rows = rows.push(self.switch_setting(
            "datetime/lock_wallpaper_clock",
            p.hide_wallpaper_clock,
            |v| change(SessionChange::LockWallpaperClock(v)),
        ));
        rows = rows.push(self.switch_setting(
            "datetime/lock_wallpaper_date",
            p.hide_wallpaper_date,
            |v| change(SessionChange::LockWallpaperDate(v)),
        ));
        self.section_card_for("datetime/lock_time", rows)
            .title(tr("shell-login-screen"))
            .description(tr(
                "shell-make-it-yours-material-applies-to-the-clock-buttons-and-password-field-duet",
            ))
            .into_element()
    }

    pub(super) fn lock_font_open(&mut self, open: bool) -> Task<Message> {
        let open = open && self.controls_live();
        self.menu = open.then(|| "datetime/lock_font".into());
        self.lock_font_active = lock_font_options()
            .iter()
            .position(|family| *family == self.settings.session.lock_clock.font_family())
            .unwrap_or(0);
        self.lock_font_reveal()
    }

    pub(super) fn lock_font_move(&mut self, movement: i32) -> Task<Message> {
        if self.menu.as_deref() != Some("datetime/lock_font") {
            return Task::none();
        }
        let count = lock_font_options().len();
        self.lock_font_active = match movement {
            i32::MIN => 0,
            i32::MAX => count.saturating_sub(1),
            delta => (self.lock_font_active as i64 + i64::from(delta))
                .rem_euclid(count.max(1) as i64) as usize,
        };
        self.lock_font_reveal()
    }

    fn lock_font_reveal(&self) -> Task<Message> {
        let last = lock_font_options().len().saturating_sub(1).max(1);
        iced::widget::operation::snap_to(
            enzui_core::identity::derive_id("lock-font-options", "menu-scroll"),
            iced::widget::scrollable::RelativeOffset {
                x: 0.0,
                y: self.lock_font_active as f32 / last as f32,
            },
            iced::widget::operation::Animation::Instant,
        )
    }

    fn lock_font_select(&self) -> Element<'_, Message> {
        use enzui_core::{
            placement::{Alignment as PopupAlignment, Placement, Side},
            semantics::{Role, Semantics},
        };
        use iced::{
            keyboard::{Key, key::Named},
            widget::{Space, text},
        };
        let open = self.menu.as_deref() == Some("datetime/lock_font");
        let palette = self.tokens().palette;
        let family = self.settings.session.lock_clock.font_family();
        let families = lock_font_options();
        let active = families
            .get(self.lock_font_active)
            .copied()
            .unwrap_or(family);
        let decode = move |key: &Key, modifiers: iced::keyboard::Modifiers| {
            if !open || modifiers.command() || modifiers.alt() {
                return None;
            }
            match key {
                Key::Named(Named::ArrowDown) => Some(Message::LockFontMove(1)),
                Key::Named(Named::ArrowUp) => Some(Message::LockFontMove(-1)),
                Key::Named(Named::Home) => Some(Message::LockFontMove(i32::MIN)),
                Key::Named(Named::End) => Some(Message::LockFontMove(i32::MAX)),
                Key::Named(Named::Enter | Named::Space) => {
                    Some(Message::LockFontSelect(active.into()))
                }
                Key::Named(Named::Escape) => Some(Message::LockFontOpen(false)),
                _ => None,
            }
        };
        let mut semantics = Semantics::new(Role::ComboBox)
            .label(tr!("shell-clock-font", "arg0" => family.to_owned()))
            .expanded(open);
        if open {
            semantics = semantics
                .active_descendant(format!("lock-font-option-{}", self.lock_font_active).into());
        }
        let tokens = super::settings_row_tokens();
        let commit = if open {
            Message::LockFontSelect(active.into())
        } else {
            Message::LockFontOpen(true)
        };
        let value_padding = 6.0;
        let caption = enz_text(font_label(family))
            .size_px(tokens.value_size)
            .text_max_width(
                tokens.inline_field_width
                    - tokens.chevron_size
                    - tokens.gap / 2.0
                    - value_padding * 2.0,
            )
            .tone(if self.controls_live() {
                TextTone::Muted
            } else {
                TextTone::Disabled
            })
            .align(TextAlign::Right)
            .single_line_elided();
        let trigger = focus_button(
            container(
                row![
                    container(caption.into_element()).width(Length::Fill),
                    enz_icon("navigation.down")
                        .size(tokens.chevron_size)
                        .tint(palette.text_muted)
                        .into_element(),
                ]
                .spacing(tokens.gap / 2.0)
                .align_y(iced::Alignment::Center),
            )
            .padding([2.0, value_padding]),
            Message::LockFontOpen(!open),
        )
        .on_enter(commit.clone())
        .on_space(commit)
        .id("lock-font-trigger".into())
        .semantics(semantics)
        .on_press_maybe(self.controls_live().then_some(Message::LockFontOpen(!open)))
        .clip(true)
        .style_focus(move |_, status| iced::widget::button::Style {
            background: Some(
                if matches!(status, FocusStatus::Hovered | FocusStatus::Pressed) {
                    palette.surface_hover.into()
                } else {
                    iced::Color::TRANSPARENT.into()
                },
            ),
            text_color: palette.text,
            border: iced::Border {
                color: if status == FocusStatus::Focused {
                    palette.focus_ring
                } else {
                    iced::Color::TRANSPARENT
                },
                width: if status == FocusStatus::Focused {
                    1.5
                } else {
                    0.0
                },
                radius: tokens.row_radius.into(),
            },
            ..Default::default()
        });
        let popup = open.then(|| {
            let options = column(families.iter().enumerate().map(|(index, &name)| {
                let selected = name == family;
                let highlighted = index == self.lock_font_active;
                let marker: Element<'_, Message> = if selected {
                    enz_icon("check").size(14.0).into_element()
                } else {
                    Space::new().width(14).into()
                };
                focus_button(
                    row![
                        container(
                            text(font_label(name))
                                .font(enzui_shell::lock_fonts::font(name, 400))
                                .size(18)
                                .wrapping(iced::widget::text::Wrapping::None)
                        )
                        .width(Length::Fill)
                        .height(36)
                        .center_y(36)
                        .clip(true),
                        marker,
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                    Message::LockFontSelect(name.into()),
                )
                .id(format!("lock-font-option-{index}").into())
                .semantics(
                    Semantics::new(Role::ListItem)
                        .label(name)
                        .selected(selected),
                )
                .embedded(true)
                .width(Length::Fill)
                .height(36)
                .padding([0, 10])
                .clip(true)
                .style_focus(move |_, status| iced::widget::button::Style {
                    background: (highlighted
                        || matches!(status, FocusStatus::Hovered | FocusStatus::Pressed))
                    .then_some(palette.accent_soft.into()),
                    text_color: palette.text,
                    border: iced::Border {
                        radius: 5.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
            }));
            let menu: Element<'_, Message> = container(
                enz_menu()
                    .width(280.0)
                    .label(tr("shell-clock-fonts"))
                    .id_prefix("lock-font-options")
                    .push(options),
            )
            .height(296)
            .into();
            keyboard_capture_unscoped(menu, decode)
        });
        let anchored = dismissible_anchored_menu(
            trigger,
            popup,
            Placement::new(Side::Bottom)
                .alignment(PopupAlignment::End)
                .gap(4.0),
            || Message::LockFontOpen(false),
        );
        keyboard_capture_unscoped(anchored, decode)
    }

    fn lock_mockup(&self) -> Element<'static, Message> {
        let p = &self.settings.session.lock_clock;
        let accent = self.tokens().palette.accent;
        let color = format!(
            "#{:02x}{:02x}{:02x}",
            (accent.r * 255.0) as u8,
            (accent.g * 255.0) as u8,
            (accent.b * 255.0) as u8
        );
        let (cx, cy, w, stacked, ux, uy) = match p.layout {
            LockLayout::Centered => (320.0, 38.0, 300.0, false, 320.0, 145.0),
            LockLayout::Split => (475.0, 24.0, 115.0, true, 175.0, 108.0),
            LockLayout::Stacked => (320.0, 14.0, 90.0, true, 320.0, 172.0),
            LockLayout::Corner => (190.0, 38.0, 265.0, false, 475.0, 133.0),
        };
        let time = if stacked { "09\n41" } else { "09:41" };
        let (paths, fw, _) = enzui_shell::lock_fonts::outlines(p.font_family(), p.weight, time)
            .or_else(|| enzui_shell::lock_fonts::outlines("Fredoka", 600, time))
            .expect("bundled clock font");
        let scale = w / fw;
        let opacity = match p.material() {
            LockMaterial::Simple => "1",
            LockMaterial::Glass => ".48",
            LockMaterial::FrostedGlass => ".78",
        };
        let clock = if p.show_time {
            format!(
                "<g transform='translate({} {cy}) scale({scale})' fill='white' fill-opacity='{opacity}' stroke='#ffffff' stroke-opacity='.3' stroke-width='1'>{paths}</g>",
                cx - w * 0.5
            )
        } else {
            String::new()
        };
        let date = if p.show_date {
            format!(
                "<path d='M{} {} h68' stroke='white' stroke-opacity='.75' stroke-width='3' stroke-linecap='round'/>",
                cx - 34.0,
                cy - 8.0
            )
        } else {
            String::new()
        };
        let mut doc = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 640 250'><defs><linearGradient id='bg' x2='1' y2='1'><stop stop-color='#11192b'/><stop offset='1' stop-color='{color}'/></linearGradient><radialGradient id='glow'><stop stop-color='#c4b6e8' stop-opacity='.36'/><stop offset='1' stop-color='#c4b6e8' stop-opacity='0'/></radialGradient></defs><rect width='640' height='250' rx='18' fill='url(#bg)'/><ellipse cx='175' cy='210' rx='220' ry='180' fill='url(#glow)'/>{clock}{date}<circle cx='{ux}' cy='{uy}' r='15' fill='{color}'/><circle cx='{ux}' cy='{}' r='5' fill='white' fill-opacity='.85'/><path d='M{} {} q10 -10 20 0' fill='white' fill-opacity='.85'/><rect x='{}' y='{}' width='106' height='17' rx='8.5' fill='#12192e' fill-opacity='.65' stroke='white' stroke-opacity='.18'/><path d='M{} {}h42' stroke='white' stroke-opacity='.55' stroke-width='2' stroke-linecap='round'/><circle cx='613' cy='225' r='10' fill='#10192e' fill-opacity='.55'/><path d='M613 218v7m-4-5a6 6 0 1 0 8 0' stroke='white' stroke-width='1.5' fill='none'/></svg>",
            uy - 4.0,
            ux - 10.0,
            uy + 9.0,
            ux - 53.0,
            uy + 25.0,
            ux - 34.0,
            uy + 33.5
        );
        let background = p
            .background
            .as_deref()
            .or_else(|| {
                (self.settings.wallpaper.kind
                    == enzui_shell::desktop_preferences::WallpaperKind::Image)
                    .then_some(self.settings.wallpaper.image.as_deref())
                    .flatten()
            })
            .and_then(|path| enzui_shell::profile_image::handle(std::path::Path::new(path)));
        if background.is_some() {
            doc = doc.replace(
                "<rect width='640' height='250' rx='18' fill='url(#bg)'/>",
                "<rect width='640' height='250' rx='18' fill='#000000' fill-opacity='.2'/>",
            );
        }
        if p.background.is_none()
            && self.settings.wallpaper.kind
                == enzui_shell::desktop_preferences::WallpaperKind::Color
        {
            doc = doc.replace(
                "fill='url(#bg)'",
                &format!("fill='{}'", self.settings.wallpaper.color),
            );
        }
        let graphic: Element<'static, Message> = svg(svg::Handle::from_memory(doc.into_bytes()))
            .width(Length::Fill)
            .height(160)
            .into();
        let mock: Element<'static, Message> = if let Some(handle) = background {
            iced::widget::stack![
                iced::widget::image(handle)
                    .width(Length::Fill)
                    .height(160)
                    .content_fit(iced::ContentFit::Cover)
                    .border_radius(18.0),
                graphic
            ]
            .into()
        } else {
            graphic
        };
        let mock = enzui_widgets::semantics_container::SemanticsContainer::leaf(
            mock,
            enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::Image)
                .label(tr("shell-login-screen-layout-preview")),
        );
        column![
            mock,
            container(
                enz_text(format!(
                    "{} · {} · {}",
                    p.layout.label(),
                    p.material().label(),
                    p.font_family()
                ))
                .into_element()
            )
            .center_x(Length::Fill)
        ]
        .spacing(8)
        .into()
    }
}
fn font_label(family: &str) -> String {
    if family == "Fredoka" {
        enzui_shell::i18n::tr("shell-fredoka-default").into()
    } else {
        family.into()
    }
}

fn lock_font_options() -> Vec<&'static str> {
    let recommended = enzui_shell::lock_fonts::RECOMMENDED;
    recommended
        .into_iter()
        .chain(
            enzui_shell::lock_fonts::families()
                .iter()
                .copied()
                .filter(|family| !recommended.contains(family)),
        )
        .collect()
}

fn weight_label(weight: u16) -> String {
    format!(
        "{} · {weight}",
        match weight {
            0..=199 => tr("shell-thin"),
            200..=299 => tr("shell-extra-light"),
            300..=399 => tr("font-weight-light"),
            400..=499 => tr("shell-regular"),
            500..=599 => tr("font-weight-medium"),
            600..=699 => tr("shell-semibold"),
            700..=799 => tr("shell-bold"),
            800..=899 => tr("shell-extra-bold"),
            _ => tr("shell-black"),
        }
    )
}
