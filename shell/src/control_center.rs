use super::{Action, Message, Shell};
pub(crate) mod connections;
use enzui_core::{
    context_menu::{InvocationKind, Point as MenuPoint},
    menu_model::{CheckState, EntryId, MenuEntryModel, MenuModel},
};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    control_grid::{Detail, Grid},
    control_layout::Shape,
    control_tile::{self, Info, ValueControl},
    settings::QuickAction,
};
use enzui_widgets::context_menu_host::{
    ContextMenuEvent, ContextMenuState, apply_event, enz_optional_point_context_menu_host,
};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    clickable::enz_clickable,
    icon::enz_icon,
    scroll_view::enz_scroll_view,
    text::enz_text,
    toolbar_icon::enz_toolbar_icon,
};
use iced::{
    Element, Length,
    widget::{column, container, responsive, row},
};

pub(crate) fn status_button(shell: &Shell) -> Element<'_, Message> {
    let s = &shell.system;
    let size = f32::from(shell.metrics.icon);
    let mut icons = row![].spacing(12).align_y(iced::Alignment::Center);
    // Capture in progress: a coloured dot each, first in the bar's corner
    // (the Control Center lists who is capturing).
    let palette = shell.window_tokens(shell.control_center).palette;
    let mut dots = row![].spacing(4).align_y(iced::Alignment::Center);
    for (active, color) in [
        (!shell.privacy.microphone.is_empty(), palette.warning),
        (!shell.privacy.camera.is_empty(), palette.success),
        (!shell.privacy.screen.is_empty(), palette.accent),
    ] {
        if active {
            dots = dots.push(container(iced::widget::space()).width(8).height(8).style(
                move |_| container::Style {
                    background: Some(color.into()),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ));
        }
    }
    if shell.privacy.any() {
        icons = icons.push(dots);
    }
    if let Some(layout) = shell
        .snapshot
        .as_ref()
        .and_then(|s| s.keyboard_layout.as_deref())
    {
        icons = icons.push(
            enz_text(layout)
                .size_px(f32::from(shell.metrics.text) - 1.0)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
        );
    }
    if let Some(audio) = &s.audio {
        icons = icons.push(
            enz_icon(if audio.muted || audio.percent == 0 {
                "volume-x"
            } else {
                "volume-2"
            })
            .size(size)
            .into_element(),
        );
    }
    if s.available(QuickAction::Network) {
        icons = icons.push(
            enz_icon(if s.network.ethernet {
                "ethernet-port"
            } else if s.network.wifi_connected {
                "wifi"
            } else {
                "wifi-off"
            })
            .size(size)
            .into_element(),
        );
    }
    if s.bluetooth == Some(true) {
        icons = icons.push(enz_icon("bluetooth").size(size).into_element());
    }
    if let Some(battery) = &s.battery {
        icons = icons.push(enz_icon(battery.icon()).size(size + 3.0).into_element());
    }
    if s.game_mode
        .as_ref()
        .is_some_and(enzui_shell::game_mode::Status::active)
    {
        icons = icons.push(enz_icon("gamepad-2").size(size).into_element());
    }
    icons = icons.push(enz_icon("sliders-horizontal").size(size).into_element());
    enzui_shell::bar_item::trigger(enz_clickable(icons, Message::ToggleControls), shell.metrics)
        .aria_label(tr("shell-control-center-2"))
        .description(if shell.privacy.any() {
            shell.privacy.describe()
        } else {
            network_label(shell).to_owned()
        })
        .id("control-center".into())
        .into_element()
}
fn network_label(shell: &Shell) -> &'static str {
    if shell.system.network.ethernet {
        tr("shell-ethernet-connected")
    } else if shell.system.network.wifi_connected {
        tr("shell-wi-fi-connected")
    } else {
        tr("shell-network-disconnected")
    }
}
fn primary(shell: &Shell, action: QuickAction) -> Message {
    match action {
        QuickAction::Network if shell.system.network.wifi_present => {
            if shell.system.network.wifi_enabled == Some(true) {
                Message::Confirm(Action::Wifi(false))
            } else {
                Message::SystemAction(Action::Wifi(true))
            }
        }
        QuickAction::Network | QuickAction::Brightness => Message::ControlDetails(Some(action)),
        QuickAction::Bluetooth => {
            Message::SystemAction(Action::Bluetooth(shell.system.bluetooth != Some(true)))
        }
        QuickAction::Appearance => Message::ToggleTheme,
        QuickAction::Tiling => Message::Tiling(!shell.snapshot.as_ref().is_some_and(|v| v.tiling)),
        QuickAction::Volume => Message::SystemAction(Action::Mute(
            !shell.system.audio.as_ref().is_some_and(|a| a.muted),
        )),
        QuickAction::Dnd => Message::Notification(crate::notification_panel::Message::Dnd(
            !shell.desired_settings.notifications.policy.dnd,
        )),
        QuickAction::Screenshot => Message::Screenshot,
        QuickAction::Dictation => Message::Dictate,
        QuickAction::Phone => Message::SettingsPage("phone"),
        QuickAction::Lock => Message::SystemAction(Action::Lock),
        QuickAction::Power => Message::ShowPower,
        QuickAction::NightLight => Message::QuickSetting(
            enzui_shell::settings::Change::NightLight(enzui_shell::night_light::Change::Enabled(
                !shell.desired_settings.night_light.enabled,
            )),
        ),
        // Several layouts: the next one.
        QuickAction::Keyboard if keyboard(shell).is_some_and(|k| k.layouts.len() > 1) => {
            Message::SwitchLayout(None)
        }
        // The larger sizes have details to open; the smaller act at once.
        QuickAction::PowerMode | QuickAction::Keyboard | QuickAction::Drives
            if control_tile::has_disclosure(shell.desired_settings.controls.shape(action)) =>
        {
            Message::ControlDetails(Some(action))
        }
        QuickAction::GameMode => Message::SystemAction(Action::GameMode(
            !shell
                .system
                .game_mode
                .as_ref()
                .is_some_and(|mode| mode.requested),
        )),
        QuickAction::PowerMode => match &shell.system.power_mode {
            Some(mode) => {
                let at = mode
                    .available
                    .iter()
                    .position(|profile| *profile == mode.active)
                    .unwrap_or(0);
                mode.available
                    .get((at + 1) % mode.available.len().max(1))
                    .map_or(Message::RegionInterior, |profile| {
                        Message::SystemAction(Action::PowerMode(*profile))
                    })
            }
            None => Message::RegionInterior,
        },
        QuickAction::Keyboard => Message::SettingsPage("keyboard"),
        // The first mounted drive opens; an unmounted one is mounted.
        QuickAction::Drives => match shell.system.drives.as_deref().and_then(|drives| {
            drives
                .iter()
                .find(|d| d.mount.is_some())
                .or_else(|| drives.first())
        }) {
            Some(drive) => match &drive.mount {
                Some(mount) => Message::Drive(enzui_shell::drives::Event::Open(mount.clone())),
                None => Message::SystemAction(Action::Mount(drive.clone())),
            },
            None => Message::RegionInterior,
        },
    }
}
fn keyboard(shell: &Shell) -> Option<&enzui_shell::compositor::Keyboard> {
    shell
        .snapshot
        .as_ref()
        .and_then(|s| s.keyboard.as_ref())
        .filter(|k| !k.layouts.is_empty())
}
fn value(shell: &Shell, action: QuickAction) -> Option<ValueControl<'static, Message>> {
    match action {
        QuickAction::Volume => shell.system.audio.as_ref().map(|audio| ValueControl {
            value: shell.volume_draft.unwrap_or(audio.percent),
            minimum: 0,
            change: Box::new(Message::Volume),
        }),
        QuickAction::Brightness => {
            shell
                .system
                .brightness
                .as_ref()
                .map(|brightness| ValueControl {
                    value: shell.brightness_draft.unwrap_or(brightness.percent),
                    minimum: 5,
                    change: Box::new(Message::Brightness),
                })
        }
        _ => None,
    }
}
pub(super) fn info(shell: &Shell, action: QuickAction) -> Info {
    let mut info = Info::read(
        action,
        &shell.desired_settings,
        &shell.system,
        shell.snapshot.as_ref().is_some_and(|v| v.tiling),
        shell.notification.ready(),
    );
    if matches!(
        action,
        QuickAction::Network
            | QuickAction::Bluetooth
            | QuickAction::Lock
            | QuickAction::Power
            | QuickAction::GameMode
    ) {
        info.enabled &= !shell.system_action_busy;
    }
    if action == QuickAction::Bluetooth
        || (action == QuickAction::Network && shell.system.network.wifi_present)
    {
        info.primary_enabled = connections::radio_toggle_allowed(shell);
    }
    if action == QuickAction::Tiling {
        info.enabled = shell.snapshot.is_some();
    }
    if action == QuickAction::Drives
        && !control_tile::has_disclosure(shell.desired_settings.controls.shape(action))
    {
        info.enabled &= shell
            .system
            .drives
            .as_ref()
            .is_some_and(|drives| !drives.is_empty());
    }
    if action == QuickAction::Keyboard
        && let Some(keyboard) = keyboard(shell)
    {
        info.detail = keyboard.layouts[keyboard.active].to_uppercase();
        info.medium.clone_from(&keyboard.keymap);
        info.extra = match keyboard.layouts.len() {
            1 => keyboard.keymap.clone(),
            count => {
                tr!("shell-of-2", "arg0" => keyboard.keymap.clone(), "arg1" => format!("{:}", keyboard.active + 1), "arg2" => format!("{:}", count))
            }
        };
    }
    info
}
fn can_disclose(shell: &Shell, action: QuickAction, disclosure: bool) -> bool {
    let shape = shell.desired_settings.controls.shape(action);
    disclosure
        && control_tile::has_disclosure(shape)
        && (action == QuickAction::Volume
            || !control_tile::inline_value(shape)
            || value(shell, action).is_none())
}

pub(super) fn controls(shell: &Shell) -> Element<'_, Message> {
    let theme = shell.window_tokens(shell.control_center);
    let font = f32::from(shell.metrics.text);
    let visible = |action| shell.desired_settings.shows(action) && shell.system.available(action);
    let detail = shell.control_details.filter(|&action| visible(action));
    let mut items = Vec::new();
    for action in QuickAction::ALL
        .into_iter()
        .filter(|&action| visible(action))
    {
        let disclosure_only =
            shell.control_details_open && detail.is_some_and(|owner| owner != action);
        let mut info = info(shell, action);
        info.primary_enabled &= !disclosure_only;
        let shape = shell.desired_settings.controls.shape(action);
        let disclosure = can_disclose(shell, action, info.disclosure);
        let theme = theme.clone();
        let child = responsive(move |size| {
            control_tile::view(
                info.clone(),
                shape,
                theme.clone(),
                primary(shell, action),
                disclosure.then_some(Message::ControlDetails(Some(action))),
                value(shell, action),
                false,
                size.width,
                size.height,
                font,
                shell.control_details == Some(action) && shell.control_details_open,
            )
        });
        let child: Element<'_, Message> = if TileMenu::rows(shell, action).is_some() {
            enzui_widgets::enz_context_target(child, move |at| Message::ControlMenuOpen(action, at))
                .enabled(!disclosure_only)
                .into()
        } else {
            child.into()
        };
        items.push((action, disclosure, child));
    }
    if items.is_empty() {
        return enz_text(tr(
            "shell-no-available-controls-selected-add-controls-in-settings",
        ))
        .into_element();
    }
    container(
        Grid::new(
            std::sync::Arc::clone(&shell.desired_settings.controls),
            items,
            enzui_shell::control_layout::row_height(font),
            &theme,
            false,
            |_| Message::RegionInterior,
        )
        .interactive(!shell.control_placing)
        .switchable_details()
        .detail(detail.map(|action| Detail {
            action,
            open: shell.control_details_open,
            height: control_tile::detail_height(action),
            header_color: if info(shell, action).active {
                theme.palette.accent
            } else {
                theme.palette.surface_alt
            },
            content: details(shell, action),
        })),
    )
    .id("control-grid-extent")
    .into()
}
fn details(shell: &Shell, action: QuickAction) -> Element<'_, Message> {
    let font = f32::from(shell.metrics.text);
    let p = shell.window_tokens(shell.control_center).palette;
    if action == QuickAction::Power {
        return detail_panel(
            shell,
            column![
                enz_text(tr("shell-save-your-work-before-restarting-or-powering-off"))
                    .size_px((font - 1.0).max(10.0))
                    .muted_on(p.surface_alt)
                    .wrap(true)
                    .into_element()
            ],
            vec![
                enz_button(
                    enzui_shell::i18n::tr("shell-restart-2"),
                    Message::Confirm(Action::Restart),
                )
                .kind(ButtonKind::Quiet)
                .into_element(),
                enz_button(tr("shell-power-off-2"), Message::Confirm(Action::PowerOff))
                    .into_element(),
            ],
        );
    }
    if action == QuickAction::Volume {
        return audio_devices(shell);
    }
    if let Some(details) = quick_details(shell, action) {
        return details;
    }
    match action {
        QuickAction::Network => return connections::network(shell),
        QuickAction::Bluetooth => return connections::bluetooth(shell),
        QuickAction::Brightness => {}
        _ => return iced::widget::space().into(),
    }
    detail_panel(
        shell,
        column![
            container(responsive(move |size| {
                control_tile::view(
                    info(shell, action),
                    Shape::Wide,
                    shell.window_tokens(shell.control_center),
                    primary(shell, action),
                    None,
                    value(shell, action),
                    false,
                    size.width,
                    size.height,
                    font,
                    true,
                )
            }))
            .height(64)
        ],
        Vec::new(),
    )
}
// Who is capturing, at the bottom of the Control Center.
fn privacy_list(shell: &Shell) -> Option<Element<'_, Message>> {
    let p = shell.window_tokens(shell.control_center).palette;
    let font = f32::from(shell.metrics.text);
    let mut list = column![].spacing(6);
    for (apps, icon, what, color) in [
        (
            &shell.privacy.microphone,
            "mic",
            tr("shell-microphone"),
            p.warning,
        ),
        (
            &shell.privacy.camera,
            "video",
            tr("shell-camera"),
            p.success,
        ),
        (
            &shell.privacy.screen,
            "screen-share",
            tr("shell-screen"),
            p.accent,
        ),
    ] {
        if apps.is_empty() {
            continue;
        }
        let names = apps.iter().cloned().collect::<Vec<_>>().join(", ");
        list = list.push(
            row![
                enz_icon(icon).size(16.0).tint(color).into_element(),
                enz_text(enzui_shell::tr!("shell-in-use-by", "arg0" => enzui_shell::i18n::argument(&(what)), "arg1" => enzui_shell::i18n::argument(&(names))))
                    .size_px((font - 1.0).max(10.0))
                    .wrap(true)
                    .into_element(),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }
    shell.privacy.any().then(|| list.into())
}

// Every tile's details share one layout: their content, scrolling if it
// must, and their actions together at the bottom right.
fn detail_panel<'a>(
    shell: &Shell,
    content: iced::widget::Column<'a, Message>,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let _ = shell;
    let mut body = column![
        iced::widget::scrollable(content.spacing(4).width(Length::Fill)).height(Length::Fill)
    ]
    .spacing(8);
    if !actions.is_empty() {
        body = body.push(
            row![iced::widget::space().width(Length::Fill)]
                .extend(actions)
                .spacing(8)
                .align_y(iced::Alignment::Center),
        );
    }
    container(body)
        .padding([12, 12])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// A choice in a details list: icon, label (and a quieter line), and a check
// on the one in effect. The whole row is the button.
#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
fn option_row<'a>(
    shell: &Shell,
    id: String,
    icon: &'static str,
    label: String,
    subtitle: Option<String>,
    selected: bool,
    enabled: bool,
    message: Message,
) -> Element<'a, Message> {
    let p = shell.window_tokens(shell.control_center).palette;
    let font = f32::from(shell.metrics.text);
    let ink = if enabled || selected {
        p.text
    } else {
        p.text_muted_on(p.surface_alt)
    };
    let mut text = column![
        enz_text(label)
            .size_px(font)
            .color(ink)
            .single_line_elided()
            .into_element()
    ]
    .spacing(1);
    if let Some(subtitle) = subtitle {
        text = text.push(
            enz_text(subtitle)
                .size_px((font - 2.0).max(10.0))
                .muted_on(p.surface_alt)
                .single_line_elided()
                .into_element(),
        );
    }
    let check: Element<'a, Message> = if selected {
        enz_icon("check").size(16.0).tint(p.accent).into_element()
    } else {
        iced::widget::space().width(16).into()
    };
    let content = row![
        enz_icon(icon).size(18.0).tint(ink).into_element(),
        container(text).width(Length::Fill),
        check
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);
    enz_clickable(
        container(content).padding([6, 8]).width(Length::Fill),
        message,
    )
    .width(Length::Fill)
    .radius(10.0)
    .enabled(enabled && !selected && !shell.control_placing)
    .id(id.into())
    .into_element()
}

fn section_label<'a>(shell: &Shell, title: &'static str) -> Element<'a, Message> {
    let p = shell.window_tokens(shell.control_center).palette;
    container(
        enz_text(title)
            .size_px(enzui_theme::text_scale::effective().apply_px(11.0))
            .weight_rung(enzui_theme::WeightRung::Semibold)
            .muted_on(p.surface_alt)
            .into_element(),
    )
    .padding(iced::Padding {
        top: 6.0,
        bottom: 2.0,
        left: 8.0,
        right: 0.0,
    })
    .into()
}

fn quick_details(shell: &Shell, action: QuickAction) -> Option<Element<'_, Message>> {
    use enzui_shell::settings::Change;
    let p = shell.window_tokens(shell.control_center).palette;
    let font = f32::from(shell.metrics.text);
    let note = |text: String| -> Element<'_, Message> {
        container(
            enz_text(text)
                .size_px((font - 2.0).max(10.0))
                .muted_on(p.surface_alt)
                .wrap(true)
                .into_element(),
        )
        .padding([4, 8])
        .into()
    };
    let writes = shell.system.system_writes && !shell.system_action_busy && !shell.control_placing;
    let settings = |page: &'static str| {
        enz_button(tr("shell-settings"), Message::SettingsPage(page))
            .kind(ButtonKind::Quiet)
            .icon("settings")
            .into_element()
    };
    Some(match action {
        QuickAction::NightLight => {
            use enzui_shell::night_light::{COOLEST, Change as Night, WARMEST};
            let night = shell.desired_settings.night_light;
            let schedule = if night.scheduled {
                tr!("shell-scheduled-from-to", "arg0" => format!("{:02}", night.from / 60), "arg1" => format!("{:02}", night.from % 60), "arg2" => format!("{:02}", night.to / 60), "arg3" => format!("{:02}", night.to % 60))
            } else {
                tr("shell-on-all-the-time-while-turned-on").into()
            };
            detail_panel(
                shell,
                column![
                    container(
                        enzui_widgets::slider::enz_slider(
                            f32::from(WARMEST)..=f32::from(COOLEST),
                            f32::from(night.temperature),
                            |kelvin| {
                                Message::QuickSetting(Change::NightLight(Night::Temperature(
                                    ((kelvin / 100.0).round() * 100.0) as u16,
                                )))
                            }
                        )
                        .step(100.0)
                        .step_marks(false)
                        .label(tr("shell-colour-temperature"))
                        .show_value(true)
                        .value_format(|kelvin| format!("{kelvin:.0} K"))
                        .enabled(!shell.control_placing)
                        .id("night-light-temperature".into())
                        .into_element()
                    )
                    .padding([0, 8]),
                    note(schedule),
                ],
                vec![settings("display/night_light")],
            )
        }
        QuickAction::PowerMode => {
            let Some(mode) = &shell.system.power_mode else {
                return Some(detail_panel(
                    shell,
                    column![note(
                        tr("shell-power-modes-are-not-available-on-this-computer").into()
                    )],
                    Vec::new(),
                ));
            };
            let mut body = column![];
            for profile in &mode.available {
                body = body.push(option_row(
                    shell,
                    format!("power-mode-{}", profile.key()),
                    profile.icon(),
                    profile.label().into(),
                    Some(profile.description().into()),
                    *profile == mode.active,
                    writes,
                    Message::SystemAction(Action::PowerMode(*profile)),
                ));
            }
            if !shell.system.system_writes {
                body = body.push(note(
                    tr("shell-this-preview-shows-your-computer-s-power-mode-change-it-there")
                        .into(),
                ));
            }
            detail_panel(shell, body, vec![settings("power")])
        }
        QuickAction::Keyboard => {
            let mut body = column![];
            match keyboard(shell) {
                Some(keyboard) => {
                    for (index, code) in keyboard.layouts.iter().enumerate() {
                        let active = index == keyboard.active;
                        body = body.push(option_row(
                            shell,
                            format!("keyboard-layout-{index}"),
                            "keyboard",
                            code.to_uppercase(),
                            (active && !keyboard.keymap.is_empty())
                                .then(|| keyboard.keymap.clone()),
                            active,
                            true,
                            Message::SwitchLayout(Some(index)),
                        ));
                    }
                }
                None => {
                    body = body.push(note(
                        tr("shell-the-keyboard-layout-is-not-known-yet").into(),
                    ));
                }
            }
            if enzui_shell::input_method::available() {
                let on = shell.desired_settings.input_method.unwrap_or(false);
                body = body.push(
                    container(
                        enzui_widgets::toggle::enz_toggle(
                            on,
                            enzui_shell::i18n::current().text("shell-input-method"),
                            move |value| Message::QuickSetting(Change::InputMethod(value)),
                        )
                        .id("keyboard-input-method".into())
                        .into_element(),
                    )
                    .padding([6, 8]),
                );
            }
            detail_panel(shell, body, vec![settings("keyboard")])
        }
        QuickAction::Drives => {
            let mut body = column![];
            match shell.system.drives.as_deref() {
                None | Some([]) => {
                    body = body.push(note(tr("shell-no-removable-drives-are-connected").into()));
                }
                Some(drives) => {
                    for (index, drive) in drives.iter().enumerate() {
                        let action: Element<'_, Message> = match &drive.mount {
                            Some(mount) => enz_toolbar_icon(
                                "folder-open",
                                Message::Drive(enzui_shell::drives::Event::Open(mount.clone())),
                            )
                            .aria_label(tr!("shell-open-3", "arg0" => drive.label.clone()))
                            .tooltip(tr("shell-open"))
                            .id(format!("drive-open-{index}").into())
                            .into_element(),
                            None => enz_button(
                                tr("shell-mount"),
                                Message::SystemAction(Action::Mount(drive.clone())),
                            )
                            .kind(ButtonKind::Quiet)
                            .size(enzui_widgets::button::ButtonSize::Sm)
                            .enabled(writes)
                            .id(format!("drive-mount-{index}").into())
                            .into_element(),
                        };
                        body = body.push(
                            container(
                                row![
                                    enz_icon("usb").size(18.0).into_element(),
                                    column![
                                        enz_text(drive.label.clone())
                                            .size_px(font)
                                            .single_line_elided()
                                            .into_element(),
                                        enz_text(drive.mount.as_ref().map_or_else(
                                            || { tr("shell-not-mounted").to_owned() },
                                            |mount| mount.display().to_string()
                                        ))
                                        .size_px((font - 2.0).max(10.0))
                                        .muted_on(p.surface_alt)
                                        .single_line_elided()
                                        .into_element(),
                                    ]
                                    .spacing(1)
                                    .width(Length::Fill),
                                    action,
                                    enz_toolbar_icon(
                                        "arrow-up-from-line",
                                        Message::SystemAction(Action::Eject(drive.clone()))
                                    )
                                    .aria_label(enzui_shell::tr!("shell-a11y-eject", "arg0" => enzui_shell::i18n::argument(&(drive.label))))
                                    .tooltip(enzui_shell::i18n::tr("shell-eject"))
                                    .enabled(writes)
                                    .id(format!("drive-eject-{index}").into())
                                    .into_element(),
                                ]
                                .spacing(10)
                                .align_y(iced::Alignment::Center),
                            )
                            .padding([6, 8]),
                        );
                    }
                }
            }
            if !shell.system.system_writes {
                body = body.push(note(
                    tr("shell-this-preview-lists-your-computer-s-drives-eject-them-there").into(),
                ));
            }
            detail_panel(shell, body, Vec::new())
        }
        _ => return None,
    })
}

fn audio_devices(shell: &Shell) -> Element<'_, Message> {
    use enzui_shell::audio_devices::Kind;
    let p = shell.window_tokens(shell.control_center).palette;
    let note = |text: String| -> Element<'_, Message> {
        container(
            enz_text(text)
                .size_px(enzui_theme::text_scale::effective().apply_px(12.0))
                .muted_on(p.surface_alt)
                .wrap(true)
                .into_element(),
        )
        .padding([4, 8])
        .into()
    };
    let mut body = column![];
    match &shell.audio_devices {
        None => body = body.push(note(tr("shell-checking-audio-devices").into())),
        Some(Err(error)) => body = body.push(note(error.clone())),
        Some(Ok(devices)) => {
            for (kind, title, icon) in [
                (Kind::Output, tr("shell-output"), "speaker"),
                (Kind::Input, tr("shell-input"), "mic"),
            ] {
                body = body.push(section_label(shell, title));
                let nodes: Vec<_> = devices.nodes.iter().filter(|d| d.kind == kind).collect();
                if nodes.is_empty() {
                    body = body.push(note(tr("shell-no-devices-available").into()));
                }
                for device in nodes {
                    body = body.push(option_row(
                        shell,
                        format!("audio-device-{}", device.serial),
                        icon,
                        device.label.clone(),
                        None,
                        devices.selected(device),
                        !shell.system_action_busy,
                        Message::SystemAction(Action::AudioDevice(device.clone())),
                    ));
                }
            }
        }
    }
    detail_panel(
        shell,
        body,
        vec![
            enz_button(
                enzui_shell::i18n::tr("shell-refresh"),
                Message::RefreshAudioDevices,
            )
            .kind(ButtonKind::Quiet)
            .icon("refresh-cw")
            .enabled(shell.audio_devices.is_some())
            .into_element(),
        ],
    )
}
pub(crate) fn view(shell: &Shell) -> Element<'_, Message> {
    let p = shell.window_tokens(shell.control_center).palette;
    let font = f32::from(shell.metrics.text);
    let radius = f32::from(shell.metrics.radius);
    let (left_inset, right_inset) = shell.control_attachment.insets(radius);
    let lock_ready = shell.system.available(QuickAction::Lock) && !shell.system_action_busy;
    // The accessibility menu has no lock or power buttons of its own.
    let header_actions = !shell.accessibility_menu;
    // Settings, lock and power are small tonal buttons in a row under the
    // controls: as large round buttons on a row of their own above them they
    // outweighed the controls the panel is opened for.
    let header_glyph = f32::from(shell.metrics.icon);
    let header_control = header_glyph + 14.0;
    let header_fill = shell
        .window_tokens(shell.control_center)
        .palette
        .surface_alt;
    fn round<'a>(
        button: Element<'a, Message>,
        fill: iced::Color,
        size: f32,
    ) -> Element<'a, Message> {
        container(button)
            .style(move |_| container::Style {
                background: Some(fill.into()),
                border: iced::Border {
                    radius: (size / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }
    let header_button = |button| round(button, header_fill, header_control);
    let leading = if shell.accessibility_menu {
        row![
            enz_text(enzui_shell::i18n::tr("shell-accessibility"))
                .size_px(font + 2.0)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
        ]
    } else if shell.power_menu {
        row![
            header_button(
                enz_toolbar_icon("arrow-left", Message::CancelConfirm)
                    .tokens(shell.window_icon_tokens(shell.control_center))
                    .size(header_control, header_glyph)
                    .aria_label(tr("shell-back-to-controls"))
                    .id("control-back".into())
                    .into_element()
            ),
            enz_text(enzui_shell::i18n::tr("shell-power"))
                .size_px(font + 2.0)
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
    } else {
        row![]
    };
    let settings_button = || {
        header_button(
            enz_toolbar_icon("settings", Message::ControlSettings)
                .tokens(shell.window_icon_tokens(shell.control_center))
                .size(header_control, header_glyph)
                .aria_label(tr("shell-open-settings"))
                .id("control-settings".into())
                .into_element(),
        )
    };
    // Settings, lock and power are one group of system actions at the end.
    let main_view = !shell.accessibility_menu && !shell.power_menu;
    let leading = if main_view && !header_actions {
        row![settings_button()]
    } else {
        leading
    };
    let mut header = row![leading, iced::widget::space().width(Length::Fill)]
        .spacing(8)
        .align_y(iced::Alignment::Center);
    // The power view is those actions itself: Lock is its first tile, and a
    // lit power button beside the title only repeated the back arrow.
    if header_actions && !shell.power_menu {
        if main_view {
            header = header.push(settings_button());
        }
        header = header
            .push(header_button(
                enz_toolbar_icon("lock-keyhole", Message::SystemAction(Action::Lock))
                    .tokens(shell.window_icon_tokens(shell.control_center))
                    .size(header_control, header_glyph)
                    .aria_label(tr("shell-lock-screen"))
                    .id("control-lock".into())
                    .enabled(lock_ready)
                    .into_element(),
            ))
            .push(header_button(
                enz_toolbar_icon("power", Message::ShowPower)
                    .tokens(shell.window_icon_tokens(shell.control_center))
                    .size(header_control, header_glyph)
                    .aria_label(tr("shell-power-menu"))
                    .id("control-power".into())
                    .active(shell.power_menu)
                    .into_element(),
            ));
    }
    // The controls come first; their own view keeps the system actions for
    // its last row. The power and accessibility views lead with a title.
    let mut body = column![].spacing(12);
    let mut actions_row = None;
    if main_view && shell.confirm.is_none() {
        actions_row = Some(header);
    } else {
        body = body.push(header);
    }
    if let Some(notice) = shell.setup_errors.message() {
        body = body.push(enz_text(notice).size_px(font).into_element());
    }
    if let Some(action) = &shell.confirm {
        let (title, detail) = match action {
            Action::Wifi(false) => (
                tr("shell-turn-wi-fi-off"),
                tr("shell-wireless-connections-including-remote-access-may-disconnect"),
            ),
            Action::PowerOff => (
                tr("shell-power-off-this-computer"),
                tr("shell-all-applications-will-close-save-your-work-before-continuing"),
            ),
            Action::Restart => (
                tr("shell-restart-this-computer"),
                tr("shell-all-applications-will-close-save-your-work-before-continuing"),
            ),
            Action::LogOut => (
                tr("shell-log-out"),
                tr("shell-all-applications-will-close-and-you-return-to-the-sign-in-screen"),
            ),
            _ => (
                tr("shell-apply-this-change"),
                tr("shell-this-changes-a-system-setting"),
            ),
        };
        body = body
            .push(enz_text(title).size_px(font + 5.0).into_element())
            .push(enz_text(detail).size_px(font).into_element());
        // Apps that hold the action off, by name and their own reason.
        let locks = shell.confirm_inhibitors.as_deref().unwrap_or_default();
        for lock in locks {
            let who = if lock.who.is_empty() {
                tr("shell-an-app")
            } else {
                lock.who.as_str()
            };
            let line = if lock.why.is_empty() {
                who.to_owned()
            } else {
                format!("{who}: {}", lock.why)
            };
            body = body.push(
                row![
                    enz_icon("triangle-alert")
                        .size(f32::from(shell.metrics.icon))
                        .tint_theme_accent()
                        .into_element(),
                    enz_text(line).size_px(font).into_element()
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        let blocked = !locks.is_empty() && matches!(action, Action::PowerOff | Action::Restart);
        if let Some((pending, at)) = &shell.confirm_deadline
            && pending == action
            && !blocked
            && shell.confirm_inhibitors.is_some()
        {
            let seconds = at
                .saturating_duration_since(std::time::Instant::now())
                .as_secs();
            let verb = match action {
                Action::PowerOff => tr("shell-powering-off"),
                Action::Restart => tr("shell-restarting"),
                _ => tr("shell-logging-out"),
            };
            body = body.push(
                enz_text(
                    tr!("shell-in-s", "arg0" => verb.to_owned(), "arg1" => format!("{:}", seconds)),
                )
                .size_px(font)
                .into_element(),
            );
        }
        let (label, go) = match action {
            Action::PowerOff if blocked => (tr("shell-power-off-anyway"), Action::PowerOffAnyway),
            Action::Restart if blocked => (tr("shell-restart-anyway"), Action::RestartAnyway),
            Action::LogOut if !locks.is_empty() => (tr("shell-log-out-anyway"), Action::LogOut),
            other => (tr("shell-confirm"), other.clone()),
        };
        body = body.push(
            row![
                enz_button(label, Message::SystemAction(go))
                    .enabled(!shell.system_action_busy)
                    .into_element(),
                enz_button(tr("shell-cancel"), Message::CancelConfirm)
                    .kind(ButtonKind::Quiet)
                    .into_element()
            ]
            .spacing(10),
        );
    } else if shell.power_menu {
        let enabled = shell.system.power && !shell.system_action_busy;
        let theme = shell.window_tokens(shell.control_center);
        let tile = |icon: &'static str,
                    label: &'static str,
                    id: &'static str,
                    press: Message,
                    enabled: bool,
                    height: f32| {
            container(enzui_shell::control_tile::action_tile(
                icon, label, press, id, &theme, enabled, font,
            ))
            .width(Length::Fill)
            .height(height)
            .into()
        };
        let gap = enzui_shell::control_layout::GAP;
        let small = 84.0;
        let mut entries = column![
            iced::widget::Row::from_vec(vec![
                tile(
                    "lock-keyhole",
                    enzui_shell::i18n::tr("shell-lock"),
                    "power-lock",
                    Message::SystemAction(Action::Lock),
                    lock_ready,
                    small
                ),
                tile(
                    "moon",
                    enzui_shell::i18n::tr("shell-suspend"),
                    "power-suspend",
                    Message::SystemAction(Action::Suspend),
                    enabled,
                    small
                ),
                tile(
                    "log-out",
                    tr("shell-log-out-2"),
                    "power-log-out",
                    Message::Confirm(Action::LogOut),
                    enabled,
                    small
                ),
            ])
            .spacing(gap),
            iced::widget::Row::from_vec({
                let mut row = vec![
                    tile(
                        "rotate-cw",
                        enzui_shell::i18n::tr("shell-restart"),
                        "power-restart",
                        Message::Confirm(Action::Restart),
                        enabled,
                        small,
                    ),
                    tile(
                        "power",
                        tr("shell-power-off-3"),
                        "power-off",
                        Message::Confirm(Action::PowerOff),
                        enabled,
                        small,
                    ),
                ];
                // Only where the OS can open a greeter beside this session.
                if shell.switch_user {
                    row.insert(
                        0,
                        tile(
                            "users",
                            tr("shell-switch-user"),
                            "power-switch-user",
                            Message::SwitchUser,
                            enabled,
                            small,
                        ),
                    );
                }
                row
            })
            .spacing(gap),
        ]
        .spacing(gap);
        if !shell.system.power {
            entries = entries.push(
                enz_text(tr(
                    "shell-power-actions-are-disabled-in-this-nested-preview",
                ))
                .size_px((font - 1.0).max(10.0))
                .muted_on(p.surface)
                .wrap(true)
                .into_element(),
            );
        }
        body = body.push(entries);
    } else if shell.accessibility_menu {
        body = body.push(shell.accessibility_menu());
    } else {
        body = body.push(controls(shell));
        if let Some(card) = shell.media_card() {
            body = body.push(card);
        }
        if let Some(list) = privacy_list(shell) {
            body = body.push(list);
        }
        // The battery reads at the start of the actions row.
        let battery: Element<'_, Message> = match &shell.system.battery {
            Some(battery) => row![
                enz_icon(battery.icon())
                    .size(f32::from(shell.metrics.icon))
                    .tint(p.text_muted)
                    .into_element(),
                enz_text(format!("{}%", battery.percent))
                    .size_px((font - 1.0).max(10.0))
                    .muted_on(p.surface)
                    .into_element(),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into(),
            None => iced::widget::space().into(),
        };
        if let Some(actions) = actions_row.take() {
            body = body.push(
                row![battery, actions]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            );
        }
    }
    if let Some(actions) = actions_row {
        body = body.push(actions);
    }
    let content = iced::widget::mouse_area(
        container(
            enz_scroll_view(container(body.width(Length::Fill)).id("control-body-extent"))
                .inset(enzui_shell::panel_style::INSET)
                .scrollbar_tokens(shell.tokens().scrollbar)
                .backdrop(p.surface)
                .into_element(),
        )
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .on_press(Message::RegionInterior);
    let bottom = shell.bar == enzui_shell::settings::BarEdge::Bottom;
    let island = matches!(
        shell.control_attachment,
        enzui_shell::reveal::Attachment::Island { .. }
    );
    let join = shell.control_attachment.join(radius);
    let panel = container(content)
        .padding(iced::Padding {
            left: left_inset,
            right: right_inset,
            top: if island {
                if bottom { 0.0 } else { join }
            } else if bottom {
                radius
            } else {
                0.0
            },
            bottom: if island {
                if bottom { join } else { 0.0 }
            } else if bottom {
                0.0
            } else {
                radius
            },
        })
        .width(Length::Fill)
        .height(Length::Fill);
    // Mounted on every frame, open or not, so opening a menu keeps the
    // panel's scroll position.
    enz_optional_point_context_menu_host(
        panel,
        shell
            .control_menu
            .as_ref()
            .map(|menu| (&menu.model, &menu.state)),
        Message::ControlMenu,
    )
    .label(tr("shell-control-options"))
    .into_element()
}

pub(super) struct TileMenu {
    action: QuickAction,
    model: MenuModel,
    state: ContextMenuState,
}

impl TileMenu {
    pub(super) fn rows(shell: &Shell, action: QuickAction) -> Option<MenuModel> {
        let check = |on: bool| {
            if on {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            }
        };
        match action {
            QuickAction::Tiling => {
                let snapshot = shell.snapshot.as_ref()?;
                let default = shell.desired_settings.tiling.unwrap_or(false);
                let own = shell
                    .desired_settings
                    .desktop_tiling
                    .get(&snapshot.workspace)
                    .copied();
                Some(MenuModel::new(vec![
                    MenuEntryModel::command("desktop-tile", tr("shell-tile-this-desktop"))
                        .icon("panels-top-left")
                        .check(check(snapshot.tiling)),
                    MenuEntryModel::command("desktop-float", tr("shell-float-this-desktop"))
                        .icon("app-window")
                        .check(check(!snapshot.tiling)),
                    MenuEntryModel::command("desktop-default", tr("shell-follow-the-default-here"))
                        .description(if default {
                            tr("shell-the-default-tiles")
                        } else {
                            tr("shell-the-default-floats")
                        })
                        .enabled(own.is_some()),
                    MenuEntryModel::separator("default-rule"),
                    MenuEntryModel::command("default-tile", tr("shell-tile-all-desktops"))
                        .description(tr("shell-by-default-desktops-set-here-keep-their-own"))
                        .check(check(default)),
                    MenuEntryModel::command("default-float", tr("shell-float-all-desktops"))
                        .description(tr("shell-by-default-desktops-set-here-keep-their-own"))
                        .check(check(!default)),
                ]))
            }
            _ => None,
        }
    }

    pub(super) fn message(shell: &Shell, action: QuickAction, id: &EntryId) -> Option<Message> {
        let rows = Self::rows(shell, action)?;
        if !rows
            .entry(id)
            .is_some_and(enzui_core::menu_model::MenuEntryModel::is_enabled)
        {
            return None;
        }
        match (action, id.as_str()) {
            (QuickAction::Tiling, "desktop-tile") => Some(Message::Tiling(true)),
            (QuickAction::Tiling, "desktop-float") => Some(Message::Tiling(false)),
            (QuickAction::Tiling, "desktop-default") => Some(Message::TilingFollowDefault),
            (QuickAction::Tiling, "default-tile") => Some(Message::TilingDefault(true)),
            (QuickAction::Tiling, "default-float") => Some(Message::TilingDefault(false)),
            _ => None,
        }
    }

    pub(super) fn open(shell: &Shell, action: QuickAction, at: iced::Point) -> Option<Self> {
        let model = Self::rows(shell, action).filter(MenuModel::has_focusable)?;
        let mut state = ContextMenuState::closed();
        state.open(InvocationKind::Pointer(MenuPoint::new(at.x, at.y)));
        Some(Self {
            action,
            model,
            state,
        })
    }
}

impl Shell {
    pub(crate) fn control_menu_event(&mut self, event: ContextMenuEvent) -> iced::Task<Message> {
        let Some(menu) = self.control_menu.as_mut() else {
            return iced::Task::none();
        };
        let action = menu.action;
        let activated = apply_event(&mut menu.model, &mut menu.state, &event);
        if !menu.state.is_open() {
            self.control_menu = None;
        }
        match activated.and_then(|id| TileMenu::message(self, action, &id)) {
            Some(message) => self.update(message),
            None => iced::Task::none(),
        }
    }
}
