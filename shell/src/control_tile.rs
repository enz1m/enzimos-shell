use crate::{
    control_layout::Shape,
    settings::{QuickAction, Settings},
    system_controls::SystemSnapshot,
};
use enzui_core::semantics::{Role, Semantics, Toggle};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_theme::EnzTheme;
use enzui_widgets::{
    focus_button::{FocusStatus, focus_button_maybe},
    icon::enz_icon,
    slider::enz_slider,
    text::enz_text,
};
use iced::{
    Color, Element, Length,
    widget::{column, container, row},
};

#[derive(Clone, Debug)]
pub struct Info {
    pub action: QuickAction,
    pub icon: &'static str,
    pub title: &'static str,
    pub detail: String,
    pub active: bool,
    pub toggle: Option<bool>,
    pub enabled: bool,
    pub primary_enabled: bool,
    pub disclosure: bool,
    /// Detail for the 3-wide size; empty uses `detail`.
    pub medium: String,
    /// Detail for the full-width size; empty uses `medium`.
    pub extra: String,
}
// A connection's state for its tile: connected, or connected but held at
// a sign-in page or without the internet.
fn connected(system: &SystemSnapshot) -> &'static str {
    match system.network.connectivity {
        crate::network_connectivity::State::Portal => tr("shell-sign-in"),
        crate::network_connectivity::State::Limited => tr("shell-limited"),
        _ => tr("shell-connected"),
    }
}

impl Info {
    pub fn read(
        action: QuickAction,
        settings: &Settings,
        system: &SystemSnapshot,
        tiled: bool,
        notifications_ready: bool,
    ) -> Self {
        use QuickAction::*;
        let (icon, detail, toggle, enabled, disclosure) = match action {
            Network if system.network.wifi_present => (
                if system.network.wifi_enabled == Some(false) {
                    "wifi-off"
                } else {
                    "wifi"
                },
                if system.network.wifi_connected {
                    connected(system)
                } else if system.network.wifi_enabled == Some(false) {
                    tr("shell-off")
                } else {
                    tr("shell-not-connected")
                }
                .into(),
                system.network.wifi_enabled,
                system.network.wifi_enabled.is_some(),
                true,
            ),
            Network => (
                "ethernet-port",
                if !system.network.ethernet_present {
                    tr("shell-no-network")
                } else if system.network.ethernet {
                    connected(system)
                } else {
                    tr("shell-not-connected")
                }
                .into(),
                None,
                system.network.ethernet_present,
                true,
            ),
            Bluetooth => (
                "bluetooth",
                match system.bluetooth {
                    Some(true) => tr("shell-on"),
                    Some(false) => tr("shell-off"),
                    None => tr("shell-no-adapter"),
                }
                .into(),
                system.bluetooth,
                system.bluetooth.is_some(),
                true,
            ),
            Appearance => (
                // The switch's own symbol; its state is the fill and "On"/"Off"
                // (a sun when off read as "light mode is on").
                "moon",
                // A switch: its detail says whether it is on, like the others.
                if settings.light {
                    tr("shell-off")
                } else {
                    tr("shell-on")
                }
                .into(),
                Some(!settings.light),
                true,
                false,
            ),
            Tiling => (
                "panels-top-left",
                if tiled {
                    tr("shell-on")
                } else {
                    tr("shell-off")
                }
                .into(),
                Some(tiled),
                true,
                false,
            ),
            Volume => {
                let muted = system.audio.as_ref().is_none_or(|audio| audio.muted);
                (
                    if muted { "volume-x" } else { "volume-2" },
                    system.audio.as_ref().map_or_else(
                        || tr("shell-no-audio-output").into(),
                        |audio| {
                            if audio.muted {
                                tr("shell-muted").into()
                            } else {
                                format!("{}%", audio.percent)
                            }
                        },
                    ),
                    system.audio.as_ref().map(|audio| !audio.muted),
                    system.audio.is_some(),
                    true,
                )
            }
            Brightness => (
                "sun",
                system.brightness.as_ref().map_or_else(
                    || tr("shell-no-backlight").into(),
                    |value| format!("{}%", value.percent),
                ),
                None,
                system.brightness.is_some(),
                true,
            ),
            Dnd => (
                "bell-off",
                if settings.notifications.policy.dnd {
                    tr("shell-on")
                } else if settings.notifications.silence_game_mode
                    && system
                        .game_mode
                        .as_ref()
                        .is_some_and(crate::game_mode::Status::active)
                {
                    tr("shell-game-mode-manual-choice-is-off")
                } else {
                    tr("shell-off")
                }
                .into(),
                Some(
                    settings
                        .notifications
                        .effective_policy(
                            system
                                .game_mode
                                .as_ref()
                                .is_some_and(crate::game_mode::Status::active),
                        )
                        .dnd,
                ),
                notifications_ready,
                false,
            ),
            Screenshot => (
                "camera",
                if system.screenshot {
                    tr("shell-this-desktop")
                } else {
                    tr("shell-grim-is-not-installed")
                }
                .into(),
                None,
                system.screenshot,
                false,
            ),
            Phone => (
                "smartphone",
                system.phone.as_ref().map_or_else(
                    || {
                        tr(if crate::phone_link::available() {
                            "shell-off"
                        } else {
                            "shell-phone-not-installed"
                        })
                        .into()
                    },
                    |presence| {
                        if presence.connected > 0 {
                            match presence.battery {
                                Some(level) => {
                                    format!("{} connected · {level}%", presence.connected)
                                }
                                None => format!("{} connected", presence.connected),
                            }
                        } else {
                            tr("shell-phone-offline").to_owned()
                        }
                    },
                ),
                Some(system.phone.is_some()),
                true,
                false,
            ),
            Dictation => (
                "mic",
                if system.dictation {
                    tr("shell-speak-to-type")
                } else {
                    tr("shell-not-installed")
                }
                .into(),
                None,
                system.dictation,
                false,
            ),
            Lock => (
                "lock-keyhole",
                if system.lock {
                    tr("shell-lock-desktop")
                } else {
                    tr("shell-the-enzui-lock-screen-is-not-installed")
                }
                .into(),
                None,
                system.lock,
                false,
            ),
            Power => (
                "power",
                if system.power {
                    tr("shell-power-options")
                } else {
                    tr("shell-not-available")
                }
                .into(),
                None,
                system.power,
                true,
            ),
            NightLight => {
                let night = &settings.night_light;
                (
                    "sunset",
                    if night.enabled {
                        tr("shell-on")
                    } else {
                        tr("shell-off")
                    }
                    .into(),
                    Some(night.enabled),
                    true,
                    true,
                )
            }
            GameMode => {
                let mode = system.game_mode.as_ref();
                (
                    "gamepad-2",
                    mode.map_or_else(
                        || tr("shell-gamemode-service-unavailable").into(),
                        |mode| {
                            if mode.requested {
                                tr("shell-desktop-request-click-to-release").into()
                            } else if mode.active() {
                                enzui_shell::i18n::count("amount-game-clients", mode.clients)
                            } else {
                                tr("shell-off-click-to-request").into()
                            }
                        },
                    ),
                    Some(mode.is_some_and(super::game_mode::Status::active)),
                    system.system_writes,
                    false,
                )
            }
            PowerMode => match &system.power_mode {
                Some(mode) => (
                    mode.active.icon(),
                    mode.active.label().into(),
                    None,
                    system.system_writes,
                    true,
                ),
                None => (
                    "gauge",
                    tr("shell-power-modes-are-not-available").into(),
                    None,
                    false,
                    true,
                ),
            },
            Keyboard => {
                let layouts = settings.compositor.layouts();
                (
                    "keyboard",
                    layouts.first().map_or_else(
                        || tr("shell-default-layout").into(),
                        |code| code.to_uppercase(),
                    ),
                    None,
                    true,
                    true,
                )
            }
            Drives => match system.drives.as_deref() {
                None => (
                    "hard-drive",
                    tr("shell-disks-service-unavailable").into(),
                    None,
                    false,
                    true,
                ),
                Some([]) => ("hard-drive", tr("shell-no-drives").into(), None, true, true),
                Some([drive]) => ("usb", drive.label.clone(), None, true, true),
                Some(drives) => (
                    "usb",
                    tr!("shell-drives-2", "arg0" => format!("{:}", drives.len())),
                    None,
                    true,
                    true,
                ),
            },
        };
        let medium = match action {
            NightLight if settings.night_light.enabled => {
                let night = &settings.night_light;
                if !night.scheduled || night.active_at(crate::night_light::local_minutes()) {
                    tr!("shell-on-k-2", "arg0" => format!("{:}", night.temperature))
                } else {
                    tr("shell-waiting-2").into()
                }
            }
            _ => String::new(),
        };
        let extra = match action {
            NightLight => night_light_extra(settings.night_light),
            PowerMode => system
                .power_mode
                .as_ref()
                .map(|mode| mode.active.description().to_owned())
                .unwrap_or_default(),
            Drives => match system.drives.as_deref() {
                Some(drives) if drives.len() > 1 => drives
                    .iter()
                    .map(|drive| drive.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                Some([drive]) => match &drive.mount {
                    Some(mount) => format!("{} · {}", drive.label, mount.display()),
                    None => {
                        tr!("shell-not-mounted-2", "arg0" => drive.label.clone())
                    }
                },
                _ => String::new(),
            },
            _ => String::new(),
        };
        Self {
            action,
            icon,
            title: action.label(),
            detail,
            // A wired connection is no switch, but it is on: its tile fills
            // like a connected Wi-Fi one.
            active: toggle == Some(true)
                || (action == Network && !system.network.wifi_present && system.network.ethernet),
            toggle,
            enabled,
            primary_enabled: true,
            disclosure,
            medium,
            extra,
        }
    }

    /// The detail a tile of `shape` shows.
    #[must_use]
    pub fn detail_for(&self, shape: Shape) -> &str {
        let medium = if self.medium.is_empty() {
            &self.detail
        } else {
            &self.medium
        };
        match shape {
            Shape::Wide if !self.extra.is_empty() => &self.extra,
            Shape::Wide | Shape::Triple => medium,
            Shape::Compact | Shape::Pill => &self.detail,
        }
    }
}

fn clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

// Night light's state, temperature and the next change of its schedule.
fn night_light_extra(night: crate::night_light::Preferences) -> String {
    if !night.enabled {
        return if night.scheduled {
            tr!("shell-off-scheduled", "arg0" => clock(night.from), "arg1" => clock(night.to))
        } else {
            tr("shell-off").into()
        };
    }
    if !night.scheduled {
        return tr!("shell-on-k-2", "arg0" => format!("{:}", night.temperature));
    }
    if night.active_at(crate::night_light::local_minutes()) {
        tr!("shell-on-k-until", "arg0" => format!("{:}", night.temperature), "arg1" => clock(night.to))
    } else {
        tr!("shell-waiting-from", "arg0" => clock(night.from))
    }
}

pub fn inline_value(shape: Shape) -> bool {
    matches!(shape, Shape::Triple | Shape::Wide)
}

pub fn detail_height(action: QuickAction) -> f32 {
    match action {
        QuickAction::Bluetooth | QuickAction::Network => 228.0,
        QuickAction::Volume => 200.0,
        QuickAction::NightLight => 150.0,
        QuickAction::PowerMode => 186.0,
        QuickAction::Keyboard => 196.0,
        QuickAction::Drives => 196.0,
        _ => 136.0,
    }
}

pub fn detail_preview<'a, Message: Clone + 'a>(
    action: QuickAction,
    theme: EnzTheme,
) -> Element<'a, Message> {
    let p = theme.palette;
    let sample = |name: &'static str, icon: &'static str| -> Element<'a, Message> {
        row![
            enz_icon(icon).size(18.0).into_element(),
            enz_text(name)
                .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                .single_line_elided()
                .width(Length::Fill)
                .into_element(),
            enz_text(enzui_shell::i18n::tr("shell-example"))
                .size_px(enzui_theme::text_scale::effective().apply_px(11.0))
                .muted_on(p.surface_alt)
                .into_element()
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .height(34)
        .into()
    };
    let details: Element<'a, Message> = match action {
        QuickAction::Bluetooth => column![
            sample(tr("shell-example-headphones"), "headphones"),
            sample(tr("shell-example-keyboard"), "keyboard"),
            sample(tr("shell-example-speaker"), "speaker")
        ]
        .spacing(6)
        .into(),
        QuickAction::Network => column![sample(tr("shell-example-network"), "wifi")].into(),
        _ => enz_text(match action {
            QuickAction::Volume => tr("shell-expanded-volume-slider-and-mute-control"),
            QuickAction::Brightness => tr("shell-expanded-brightness-slider"),
            QuickAction::Power => tr("shell-restart-and-power-off-require-confirmation"),
            QuickAction::NightLight => tr("shell-colour-temperature-and-schedule"),
            QuickAction::PowerMode => tr("shell-power-saver-balanced-and-performance"),
            QuickAction::Keyboard => tr("shell-keyboard-layouts-and-the-input-method"),
            QuickAction::Drives => tr("shell-removable-drives-with-open-and-eject"),
            _ => tr("shell-expanded-control-preview"),
        })
        .into_element(),
    };
    container(
        column![
            enz_text(
                if matches!(action, QuickAction::Network | QuickAction::Bluetooth) {
                    tr("shell-design-examples-only-device-connections-are-not-available-in-this-preview")
                } else {
                    tr("shell-layout-preview-only-use-control-center-to-adjust-the-live-control")
                }
            )
            .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
            .muted_on(p.surface_alt)
            .into_element(),
            details,
        ]
        .spacing(10),
    )
    .padding(14)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

pub struct ValueControl<'a, Message> {
    pub value: u8,
    pub minimum: u8,
    pub change: Box<dyn Fn(f32) -> Message + 'a>,
}

fn blend(a: Color, b: Color, amount: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * amount,
        g: a.g + (b.g - a.g) * amount,
        b: a.b + (b.b - a.b) * amount,
        a: a.a + (b.a - a.a) * amount,
    }
}

/// A labelled action tile in the Control Center's tile language: a rounded
/// surface with the icon above its label. Used by the power menu.
pub fn action_tile<'a, Message: Clone + 'a>(
    icon: &'static str,
    label: &'static str,
    message: Message,
    id: &str,
    theme: &EnzTheme,
    enabled: bool,
    font: f32,
) -> Element<'a, Message> {
    let p = &theme.palette;
    // Unavailable actions read clearly quieter than the ones that work.
    let ink = if enabled {
        p.text
    } else {
        Color { a: 0.45, ..p.text }
    };
    let content = column![
        enz_icon(icon).size(22.0).tint(ink).into_element(),
        enz_text(label)
            .announce(false)
            .size_px((font - 1.0).max(10.0))
            .weight_rung(enzui_theme::WeightRung::Medium)
            .color(ink)
            .no_wrap()
            .into_element(),
    ]
    .spacing(8)
    .align_x(iced::Alignment::Center);
    button(
        content.into(),
        message,
        Semantics::new(Role::Button).label(label),
        id.to_owned(),
        theme,
        false,
        enabled,
        16.0.into(),
        iced::Padding::from([8.0, 8.0]),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
fn button<'a, Message: Clone + 'a>(
    content: Element<'a, Message>,
    message: Message,
    semantics: Semantics,
    id: String,
    theme: &EnzTheme,
    active: bool,
    enabled: bool,
    radius: iced::border::Radius,
    padding: iced::Padding,
) -> Element<'a, Message> {
    let p = theme.palette.clone();
    let bg = tile_background(theme, active);
    let ink = if enabled { p.text } else { p.text_muted_on(bg) };
    focus_button_maybe(content, enabled.then_some(message))
        .id(id.into())
        .semantics(semantics)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(padding)
        .clip(true)
        .content_align_x(iced::Alignment::Center)
        .content_align_y(iced::Alignment::Center)
        .style_focus(move |_, status| iced::widget::button::Style {
            background: Some(
                match status {
                    FocusStatus::Hovered => blend(bg, ink, 0.07),
                    FocusStatus::Pressed => blend(bg, ink, 0.12),
                    _ => bg,
                }
                .into(),
            ),
            text_color: ink,
            border: iced::Border {
                color: if active { ink } else { p.accent },
                width: if status == FocusStatus::Focused {
                    2.0
                } else {
                    0.0
                },
                radius,
            },
            ..Default::default()
        })
        .into()
}

fn disclosure_button<'a, Message: Clone + 'a>(
    info: &Info,
    theme: &EnzTheme,
    message: Message,
    radius: f32,
    expanded: bool,
) -> Element<'a, Message> {
    let p = theme.palette.clone();
    let bg = tile_background(theme, info.active);
    let ink = if info.active {
        tile_ink(&p, bg)
    } else {
        p.text
    };
    let muted = if info.active {
        blend(ink, bg, 0.15)
    } else {
        p.text_muted_on(bg)
    };
    let focus = if info.active { ink } else { p.accent };
    let status = std::rc::Rc::new(std::cell::Cell::new(FocusStatus::Active));
    let glyph = enz_icon(if expanded {
        "chevron-up"
    } else {
        "chevron-down"
    })
    .size(16.0)
    .tint_with_status(std::rc::Rc::clone(&status), move |_, status| {
        if status == FocusStatus::Active {
            muted
        } else {
            ink
        }
    })
    .into_element();
    focus_button_maybe(glyph, info.enabled.then_some(message))
        .id(format!("control-{}-details", info.action.key()).into())
        .semantics(
            Semantics::new(Role::Button)
                .label(tr!("shell-details-2", "arg0" => (if expanded { tr("shell-hide") } else { tr("shell-show") }).to_owned(), "arg1" => info.title.to_owned()))
                .expanded(expanded),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(0)
        .content_align_x(iced::Alignment::Center)
        .content_align_y(iced::Alignment::Center)
        .style_focus(move |_, current| {
            status.set(current);
            iced::widget::button::Style {
                background: Some(bg.into()),
                text_color: ink,
                border: iced::Border {
                    color: focus,
                    width: if current == FocusStatus::Focused {
                        2.0
                    } else {
                        0.0
                    },
                    radius: iced::border::Radius {
                        top_left: 0.0,
                        bottom_left: 0.0,
                        top_right: radius,
                        bottom_right: radius,
                    },
                },
                ..Default::default()
            }
        })
        .into()
}

pub fn has_disclosure(shape: Shape) -> bool {
    matches!(shape, Shape::Triple | Shape::Wide)
}

#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
pub fn view<'a, Message: Clone + 'a>(
    info: Info,
    shape: Shape,
    theme: EnzTheme,
    primary: Message,
    disclosure: Option<Message>,
    value: Option<ValueControl<'a, Message>>,
    preview: bool,
    width: f32,
    height: f32,
    font: f32,
    expanded: bool,
) -> Element<'a, Message> {
    if info.action == QuickAction::Volume {
        return volume_row(
            info, theme, primary, disclosure, value, preview, expanded, height,
        );
    }
    let compact = width <= height + 24.0;
    // A 2-wide tile. The rows are 44px now, so the old 2.6 ratio no longer
    // counted it, and long names ("Modo oscuro") lost their tighter fit.
    let narrow = !compact && width < height * 3.0;
    let radius = height / 2.0;
    let disclosure = disclosure.filter(|_| has_disclosure(shape) && !compact);
    let p = theme.palette.clone();
    let bg = tile_background(&theme, info.active);
    // Only the accent fill needs choosing: an off tile is the panel's own
    // (possibly translucent glass) surface, where the theme's ink is right.
    let ink = if info.active {
        tile_ink(&p, bg)
    } else {
        p.text
    };
    let mut semantics = Semantics::new(if !preview && info.toggle.is_some() {
        Role::Switch
    } else {
        Role::Button
    })
    .label(if preview {
        tr!("shell-select-layout", "arg0" => info.title.to_owned())
    } else {
        info.title.to_owned()
    })
    .description(info.detail_for(shape).to_owned());
    if !preview && let Some(active) = info.toggle {
        semantics = semantics.toggled(if active { Toggle::True } else { Toggle::False });
    }
    let enabled = preview || (info.enabled && info.primary_enabled);
    if inline_value(shape)
        && width >= height * 1.8
        && disclosure.is_none()
        && let Some(value) = value
    {
        // Styled as the volume row: the icon, the pill and its percentage.
        let icon = enzui_widgets::enz_toolbar_icon(info.icon, primary)
            .aria_label(info.title)
            .id(format!("control-{}-primary", info.action.key()).into())
            .enabled(enabled)
            .into_element();
        let level = pill_level(
            value,
            info.title,
            format!("control-{}-value", info.action.key()),
            enabled,
            &p,
        );
        return value_row(icon, level, None, &p, height);
    }
    let icon = enz_icon(info.icon).size(18.0).tint(ink).into_element();
    let content: Element<'a, Message> = if compact {
        icon
    } else {
        row![
            icon,
            column![
                enz_text(match info.action {
                    QuickAction::Volume => enzui_shell::i18n::tr("shell-volume"),
                    // Its mode is the detail under it.
                    QuickAction::PowerMode if narrow => enzui_shell::i18n::tr("shell-power"),
                    _ => info.title,
                })
                // A long name still fits a 2-wide tile.
                .size_px(if narrow && info.title.chars().count() > 10 {
                    (font - 1.5).max(10.0)
                } else {
                    font - 0.5
                })
                .single_line_elided()
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .color(ink)
                .into_element(),
                enz_text(info.detail_for(shape).to_owned())
                    .size_px((font - 2.5).max(10.0))
                    .single_line_elided()
                    .color(if info.active {
                        ink
                    } else {
                        p.text_muted_on(bg)
                    })
                    .into_element()
            ]
            .spacing(0)
            .width(Length::Fill)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into()
    };
    let main = button(
        content,
        primary,
        semantics,
        format!("control-{}-primary", info.action.key()),
        &theme,
        info.active,
        enabled,
        if disclosure.is_some() {
            iced::border::Radius {
                top_left: radius,
                bottom_left: radius,
                top_right: 0.0,
                bottom_right: 0.0,
            }
        } else {
            radius.into()
        },
        // Two lines of text fill most of a tile's height: little room is left
        // above and below them.
        if compact {
            iced::Padding::from([5.0, 0.0])
        } else if narrow {
            // The full-width tile's space before and after the icon; the
            // title elides on the right instead.
            iced::Padding {
                top: 5.0,
                bottom: 5.0,
                left: 12.0,
                right: 6.0,
            }
        } else {
            iced::Padding::from([5.0, 12.0])
        },
    );
    let main = if compact {
        enzui_widgets::tooltip::enz_tooltip(
            main,
            format!(
                "{} · {}",
                info.title,
                if info.extra.is_empty() {
                    &info.detail
                } else {
                    &info.extra
                }
            ),
            iced::widget::tooltip::Position::Bottom,
        )
    } else {
        main
    };
    if let Some(disclosure) = disclosure {
        let arrow = disclosure_button(&info, &theme, disclosure, radius, expanded);
        enzui_widgets::rounded_clip::rounded_clip(
            container(
                row![
                    container(main).width(Length::Fill),
                    container(arrow).width(28).height(Length::Fill)
                ]
                .spacing(0),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(bg.into()),
                border: iced::Border {
                    radius: radius.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            radius,
        )
        .into()
    } else {
        main
    }
}

/// The volume pill: a soft track, the accent fill and a borderless white knob.
fn pill_slider(p: &enzui_theme::Palette) -> enzui_widgets::slider::SliderTokens {
    let mut tokens = enzui_widgets::slider::SliderTokens::from_palette(p);
    tokens.track = p.surface_hover;
    tokens.track_border_width = Some(0.0);
    tokens.thumb = iced::Color::WHITE;
    tokens.thumb_dragging = iced::Color::WHITE;
    tokens.thumb_border_width = Some(0.0);
    tokens.thumb_shadow = iced::Color {
        a: 0.25,
        ..iced::Color::BLACK
    };
    tokens.thumb_shadow_offset = iced::Vector::new(0.0, 1.0);
    tokens.thumb_shadow_blur = 4.0;
    tokens
}

#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
fn volume_row<'a, Message: Clone + 'a>(
    info: Info,
    theme: EnzTheme,
    primary: Message,
    disclosure: Option<Message>,
    value: Option<ValueControl<'a, Message>>,
    preview: bool,
    expanded: bool,
    height: f32,
) -> Element<'a, Message> {
    let enabled = info.enabled && info.primary_enabled && !preview;
    let p = theme.palette;
    let muted = info.toggle != Some(true);
    let mute = enzui_widgets::enz_toolbar_icon(info.icon, primary)
        .aria_label(if muted {
            tr("shell-unmute-output")
        } else {
            tr("shell-mute-output")
        })
        .id("control-volume-primary".into())
        .enabled(enabled)
        .into_element();
    let level: Element<'a, Message> = if let Some(value) = value {
        pill_level(
            value,
            tr("shell-output-volume"),
            "control-volume-value".to_owned(),
            enabled,
            &p,
        )
    } else {
        enz_text(tr("shell-no-audio-output"))
            .single_line_elided()
            .into_element()
    };
    let arrow = disclosure.map(|message| {
        enzui_widgets::enz_toolbar_icon(
            if expanded {
                "chevron-up"
            } else {
                "chevron-down"
            },
            message,
        )
        .aria_label(if expanded {
            tr("shell-hide-audio-devices")
        } else {
            tr("shell-show-audio-devices")
        })
        .id("control-volume-details".into())
        .enabled(info.enabled || preview)
        .into_element()
    });
    value_row(mute, level, arrow, &p, height)
}

/// A level as the volume shows it: a thick pill whose fill is the level, with
/// a flush round knob, and the percentage beside it.
fn pill_level<'a, Message: Clone + 'a>(
    value: ValueControl<'a, Message>,
    label: &str,
    id: String,
    enabled: bool,
    p: &enzui_theme::Palette,
) -> Element<'a, Message> {
    let percent = value.value;
    row![
        container(
            enz_slider(
                f32::from(value.minimum)..=100.0,
                f32::from(value.value),
                value.change
            )
            .step(1.0)
            .step_marks(false)
            .aria_label(label)
            .show_value(false)
            .track_height(20.0)
            .thumb_width(20.0)
            .thumb_height(20.0)
            .tokens(pill_slider(p))
            .enabled(enabled)
            .id(id.into())
            .into_element()
        )
        .width(Length::Fill),
        container(
            enz_text(format!("{percent}%"))
                .size_px(enzui_theme::text_scale::effective().apply_px(13.0))
                .color(p.text_muted)
                .into_element()
        )
        .width(40)
        .align_x(iced::Alignment::End),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center)
    .into()
}

fn value_row<'a, Message: 'a>(
    icon: Element<'a, Message>,
    level: Element<'a, Message>,
    trailing: Option<Element<'a, Message>>,
    p: &enzui_theme::Palette,
    height: f32,
) -> Element<'a, Message> {
    let radius = (height / 2.0).min(28.0);
    let background = p.surface_alt;
    let text = p.text;
    // Both sliders reserve the same trailing column for the volume disclosure.
    let trailing = trailing.unwrap_or_else(|| iced::widget::space().into());
    let content = row![
        container(icon).width(32),
        container(level).width(Length::Fill),
        container(trailing).width(32),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);
    container(content)
        .padding([0, 8])
        .width(Length::Fill)
        .height(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(background.into()),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            text_color: Some(text),
            ..Default::default()
        })
        .into()
}

/// The label colour for a tile of colour `bg`: the theme's ink unless the
/// other (light or dark) reads better on it. A light accent (amber, mint, a
/// glass-lightened blue) needs dark text; the theme's white ink failed there.
fn tile_ink(p: &enzui_theme::Palette, bg: Color) -> Color {
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    let contrast = |a: Color, b: Color| {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    };
    let dark = Color::from_rgb8(0x1D, 0x1D, 0x1F);
    let other = if luminance(p.text) > 0.5 {
        dark
    } else {
        Color::WHITE
    };
    if contrast(p.text, bg) >= 4.5 || contrast(p.text, bg) >= contrast(other, bg) {
        p.text
    } else {
        other
    }
}

fn tile_background(theme: &EnzTheme, active: bool) -> Color {
    let p = &theme.palette;
    if active {
        // Toward what the tile's surface shows: on glass `surface_alt` is a
        // faint white wash, which turned the active tile pale sky-blue in
        // islands but deep accent everywhere else.
        // In light themes the softened accent was a second, paler blue next
        // to the slider's fill: there the tile takes the filled accent itself.
        blend(
            p.filled_color(p.accent).color,
            p.solid_backdrop(p.surface_alt),
            if p.is_dark() { 0.35 } else { 0.0 },
        )
    } else if p.surface_alt.a < 1.0 && !p.is_dark() {
        // On light glass the alternate surface is a faint white wash that
        // nearly matched the panel (about 1.05:1), and a whiter wash still
        // read as "nearly white on pale": a light ink wash separates.
        Color { a: 0.07, ..p.text }
    } else {
        p.surface_alt
    }
}
