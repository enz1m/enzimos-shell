use super::*;
use enzui_shell::i18n::{self, tr};
use enzui_shell::system_settings::{
    About, Backed, Clock, Power, critical_confirmation, critical_label, critical_risk,
    memory_detail, memory_label, profile_label,
};
use enzui_shell::tr;
use enzui_widgets::{
    icon::enz_icon,
    pressable::{PressableStyle, enz_pressable},
    progress_bar::{ProgressTone, enz_progress_bar},
};
use iced::{
    Alignment, Length,
    widget::{Space, column, container, row},
};

fn profile_details(profile: &str) -> (&'static str, &'static str) {
    match profile {
        "power-saver" => ("leaf", tr("shell-longer-battery-life")),
        "performance" => ("zap", tr("shell-fastest-uses-more-power")),
        _ => ("gauge", tr("shell-performance-and-battery")),
    }
}

fn memory_status(
    policy: &enzui_shell::system_settings::MemoryPolicy,
    busy: bool,
) -> (&'static str, String, String) {
    let running = policy.active_state == "active";
    if busy {
        (
            "loader",
            tr("shell-applying").into(),
            tr("shell-your-change-takes-effect-once-the-system-confirms-it").into(),
        )
    } else if policy.mode == "off" && matches!(policy.active_state.as_str(), "inactive" | "failed")
    {
        (
            "shield-off",
            tr("shell-off").into(),
            tr("shell-only-the-kernel-steps-in-when-memory-runs-out").into(),
        )
    } else if policy.mode == "off" {
        (
            "shield-alert",
            tr("shell-off-is-chosen-stopping-is-not-confirmed").into(),
            tr!("shell-the-guard-reports-refreshing-its-actual-state-is-required-before-calli", "arg0" => policy.active_state.clone()),
        )
    } else if running && policy.running_mode.as_deref() == Some(policy.mode.as_str()) {
        let using = policy
            .memory_bytes
            .map(|b| tr!("shell-uses-mb-of-memory", "arg0" => enzui_shell::i18n::number(format!("{:.1}", b as f64 / 1_000_000.0))))
            .unwrap_or_default();
        ("shield-check", tr("shell-guard-running").into(), using)
    } else if running && policy.running_mode.is_none() {
        (
            "shield-alert",
            tr("shell-running-preset-is-unconfirmed").into(),
            tr!("shell-is-chosen-the-system-has-not-confirmed-the-guard-s-running-preset", "arg0" => memory_label(&policy.mode).to_owned()),
        )
    } else if running {
        (
            "shield-alert",
            tr("shell-configured-and-running-presets-differ").into(),
            tr!("shell-is-chosen-still-running-the-new-preset-is-not-confirmed", "arg0" => memory_label(&policy.mode).to_owned(), "arg1" => policy.running_mode.as_deref().map_or_else(|| tr("shell-an-unknown-preset"), memory_label).to_owned()),
        )
    } else if matches!(policy.active_state.as_str(), "inactive" | "failed") {
        (
            "shield-alert",
            tr("shell-not-running").into(),
            tr!("shell-is-chosen-but-the-guard-is", "arg0" => memory_label(&policy.mode).to_owned(), "arg1" => policy.active_state.clone()),
        )
    } else {
        (
            "shield-alert",
            tr("shell-guard-status-is-unconfirmed").into(),
            tr!("shell-is-chosen-the-system-has-not-confirmed-whether-the-guard-is-running", "arg0" => memory_label(&policy.mode).to_owned()),
        )
    }
}

fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    if size >= 100.0 || unit == 0 {
        format!("{size:.0} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

impl SettingsApp {
    fn battery_card<'a>(&'a self, power: Option<&Power>) -> Element<'a, Parent> {
        let route = "power/battery";
        let content: Element<'a, Parent> = match power {
            None => self.loading_row(),
            Some(Power { battery: None, .. }) => self.info_row(
                "plug",
                tr("shell-no-battery"),
                tr("shell-this-computer-runs-on-mains-power"),
            ),
            Some(Power {
                battery: Some(battery),
                ..
            }) => {
                let icon =
                    enzui_shell::system_controls::battery_icon(battery.percent, battery.charging);
                let tone = match (battery.charging, battery.percent) {
                    (true, _) => ProgressTone::Success,
                    (false, 0..=15) => ProgressTone::Danger,
                    (false, 16..=25) => ProgressTone::Warning,
                    _ => ProgressTone::Accent,
                };
                let mut status = battery.state.clone();
                if let Some(remaining) = &battery.remaining {
                    status = format!(
                        "{status} · {remaining} {}",
                        if battery.charging {
                            tr("shell-until-full")
                        } else {
                            tr("shell-remaining")
                        }
                    );
                }
                let palette = self.tokens().palette;
                container(
                    column![
                        row![
                            enz_icon(icon).size(28.0).into_element(),
                            column![
                                enz_text(format!("{}%", battery.percent))
                                    .size(TextSize::Title2)
                                    .into_element(),
                                enz_text(status)
                                    .size(TextSize::Footnote)
                                    .muted_on(palette.surface)
                                    .into_element(),
                            ]
                            .spacing(2),
                        ]
                        .spacing(14)
                        .align_y(Alignment::Center),
                        enz_progress_bar(f32::from(battery.percent) / 100.0)
                            .tone(tone)
                            .bar_height(6.0)
                            .into_element(),
                    ]
                    .spacing(12),
                )
                .padding([12, 12])
                .into()
            }
        };
        let (critical, critical_shown) = self.critical_rows(power);
        self.section_card_for(
            route,
            column![self.mark_setting(route, content), critical].spacing(2),
        )
        .title(tr("shell-battery"))
        .description(if critical_shown {
            tr("shell-charge-level-the-time-left-and-what-the-computer-does-when-the-battery-is-al")
        } else {
            tr("shell-charge-level-and-the-time-left")
        })
        .into_element()
    }
    fn power_mode_card<'a>(&self, power: Option<&Power>) -> Element<'a, Parent> {
        let route = "power/mode";
        let content: Element<'a, Parent> = match power.and_then(|p| p.profiles.as_ref()) {
            None if power.is_none() => self.loading_row(),
            None => self.info_row(
                "gauge",
                tr("shell-power-modes-are-unavailable"),
                tr("shell-install-power-profiles-daemon-to-switch-between-power-saver-and-performance"),
            ),
            Some(profiles) => {
                let palette = self.tokens().palette;
                let tiles = profiles.available.iter().map(|profile| {
                    let selected = *profile == profiles.active;
                    let (icon, detail) = profile_details(profile);
                    enz_pressable(
                        column![
                            enz_icon(icon).size(22.0).into_element(),
                            enz_text(profile_label(profile).to_owned()).weight_rung(enzui_theme::WeightRung::Semibold).into_element(),
                            enz_text(detail).size(TextSize::Footnote).muted_on(palette.surface).into_element(),
                        ]
                        .spacing(6)
                        .align_x(Alignment::Center)
                        .into(),
                        Self::request(Request::PowerProfile(profile.clone())),
                    )
                    .style(PressableStyle::Card)
                    .selected(selected)
                    .selectable(true)
                    .aria_label(tr!("shell-power-mode-2", "arg0" => profile_label(profile).to_owned()))
                    .id(format!("power-mode-{profile}-{selected}").into())
                    .padding(12)
                    .radius(enzui_theme::text_defaults::shape_radius(|shape| shape.card))
                    .border_width(1.0)
                    .width(Length::Fill)
                    .enabled(self.system_writes())
                    .into_element()
                });
                container(row(tiles).spacing(8)).padding([6, 10]).into()
            }
        };
        self.section_card_for(route, self.mark_setting(route, content))
            .title(tr("shell-power-mode"))
            .description(tr(
                "shell-trade-speed-for-battery-life-plugged-in-computers-can-stay-on-balanced",
            ))
            .into_element()
    }
    pub(super) fn power_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let power = match self.reading(Panel::Power) {
            Some(Reading::Power(p)) => Some(p),
            _ => None,
        };
        use enzui_shell::session_preferences::{self as session, SessionChange};
        use enzui_shell::settings::Change;
        let prefs = &self.settings.session;
        let change = |c: SessionChange| Parent::Change(Change::Session(c));
        let has_battery = power.as_ref().is_some_and(|p| p.battery.is_some());
        let minutes = |choices: &[u16], wrap: fn(u16) -> SessionChange| -> Vec<(String, Parent)> {
            choices
                .iter()
                .map(|&m| (session::minutes_label(m), change(wrap(m))))
                .collect()
        };
        let mut sleep = column![
            self.choice_setting(
                "power/blank",
                session::minutes_label(prefs.blank_minutes),
                minutes(&session::BLANK_CHOICES, SessionChange::Blank)
            ),
            self.choice_setting(
                "power/lock",
                session::lock_label(prefs.lock_delay_minutes),
                session::LOCK_CHOICES
                    .iter()
                    .map(|&m| (session::lock_label(m), change(SessionChange::LockDelay(m))))
                    .collect(),
            ),
        ]
        .spacing(2);
        if has_battery {
            sleep = sleep.push(self.choice_setting(
                "power/suspend",
                session::minutes_label(prefs.suspend_battery_minutes),
                minutes(&session::SUSPEND_CHOICES, SessionChange::SuspendBattery),
            ));
        }
        sleep = sleep.push(self.choice_setting(
            if has_battery {
                "power/suspend_ac"
            } else {
                "power/suspend"
            },
            session::minutes_label(prefs.suspend_ac_minutes),
            minutes(&session::SUSPEND_CHOICES, SessionChange::SuspendAc),
        ));
        if power.as_ref().is_some_and(|p| p.lid) {
            sleep = sleep.push(
                self.choice_setting(
                    "power/lid",
                    prefs.lid.label(),
                    session::LidAction::ALL
                        .into_iter()
                        .map(|a| (a.label().to_owned(), change(SessionChange::Lid(a))))
                        .collect(),
                ),
            );
        } else {
            // The route stays reachable from search on computers without a lid.
            sleep = sleep.push(self.mark_control_anchor("power/lid", iced::widget::Space::new()));
        }
        if !has_battery {
            sleep = sleep
                .push(self.mark_control_anchor("power/suspend_ac", iced::widget::Space::new()));
        }
        use enzui_shell::platform::Feature;
        let mut body = body.push(self.battery_card(power.as_ref()));
        if self.offers(Feature::PowerModes) {
            body = body.push(self.power_mode_card(power.as_ref()));
        }
        if self.offers(Feature::GameMode) {
            body = body.push(self.game_mode_card(power.as_ref()));
        }
        body.push(
            self.section_card_for("power/blank", sleep)
                .title(tr("shell-screen-sleep"))
                .description(tr("shell-what-happens-when-the-computer-is-left-alone"))
                .into_element(),
        )
        .push(self.memory_card(power.as_ref()))
        .push(self.tmp_storage_card(power.as_ref()))
    }
    fn game_mode_card<'a>(&'a self, power: Option<&Power>) -> Element<'a, Parent> {
        use enzui_shell::game_mode::preferences::OptionKey;
        let route = "power/game_mode";
        let Some(power) = power else {
            return self
                .section_card_for(
                    route,
                    column![
                        self.mark_setting(route, self.loading_row()),
                        self.mark_control_anchor("power/game_priority", Space::new())
                    ],
                )
                .title(tr("shell-game-mode"))
                .into_element();
        };
        let status = match &power.game_mode {
            Some(mode) if mode.active() => {
                enzui_shell::i18n::count("shell-game-mode-on-for-games", mode.clients)
            }
            Some(_) => tr("shell-game-mode-is-off").into(),
            None => tr("shell-game-mode-is-unavailable").into(),
        };
        let prefs = &power.game_preferences;
        let writes = self.system_writes();
        let mut rows = column![self.mark_setting(route, self.info_row("gamepad-2", status, tr("shell-games-turn-it-on-themselves-add-game-mode-to-control-center-to-turn-it-on")))].spacing(2);
        if let Some(error) = &prefs.error {
            rows = rows.push(self.info_row(
                "triangle-alert",
                tr("shell-configuration-could-not-be-fully-read"),
                error.clone(),
            ));
        }
        if prefs.performance_profile {
            rows = rows.push(self.info_row("zap", tr("shell-performance-power-mode-while-active"), tr("shell-the-computer-switches-to-performance-and-returns-to-your-power-mode-when-the")));
        }
        rows = rows.push(
            crate::settings_app::settings_row(tr("shell-keep-the-screen-awake"))
                .icon("monitor-check")
                .subtitle(tr("shell-the-screen-stays-on-while-a-game-runs"))
                .id_prefix("game-mode-awake")
                .switch(prefs.keep_awake, |on| {
                    Self::request(Request::GameModeOption(OptionKey::KeepAwake, u8::from(on)))
                })
                .enabled(writes)
                .into_element(),
        );
        rows = rows.push(crate::settings_app::settings_row(tr("shell-help-older-windows-games-run-smoothly"))
            .icon("cpu")
            .subtitle(tr("shell-turns-off-a-safeguard-that-can-slow-some-windows-games-a-misbehaving-program"))
            .id_prefix("game-mode-splitlock").switch(prefs.disable_splitlock, |on| Self::request(Request::GameModeOption(OptionKey::SplitLock, u8::from(on)))).enabled(writes).into_element());
        let choices = [
            (tr("shell-off"), 0),
            (tr("shell-higher"), 5),
            (tr("shell-highest"), 10),
        ]
        .into_iter()
        .map(|(label, value)| {
            (
                label.to_owned(),
                Self::request(Request::GameModeOption(OptionKey::Priority, value)),
            )
        })
        .collect();
        let label = match prefs.priority {
            0 => tr("shell-off").into(),
            5 => tr("shell-higher").into(),
            10 => tr("shell-highest").into(),
            other => tr!("shell-raise-by", "arg0" => format!("{:}", other)),
        };
        rows = rows.push(self.choice_setting("power/game_priority", label, choices));
        rows = rows.push(
            crate::settings_app::settings_row(tr(
                "shell-silence-notifications-while-gamemode-is-active",
            ))
            .icon("bell-off")
            .subtitle(tr(
                "shell-turns-on-do-not-disturb-while-a-game-runs-urgent-alerts-still-appear",
            ))
            .id_prefix("game-mode-notifications")
            .switch(self.settings.notifications.silence_game_mode, |on| {
                Parent::Change(enzui_shell::settings::Change::Notification(
                    enzui_shell::notifications::preferences::Change::SilenceGameMode(on),
                ))
            })
            .enabled(self.controls_live())
            .into_element(),
        );
        self.section_card_for(route, rows)
            .title(tr("shell-game-mode"))
            .description(tr(
                "shell-helps-games-run-smoothly-changes-apply-the-next-time-a-game-starts",
            ))
            .into_element()
    }
    fn memory_card<'a>(&'a self, power: Option<&Power>) -> Element<'a, Parent> {
        let route = "power/memory";
        let policy = match power.map(|p| &p.memory) {
            Some(Backed::Ready(policy)) => policy.clone(),
            Some(Backed::Refused(error)) => {
                let row = self.info_row(
                    "memory-stick",
                    tr("shell-memory-protection-cannot-be-read"),
                    error.clone(),
                );
                return self
                    .section_card_for(route, column![self.mark_setting(route, row)])
                    .title(tr("shell-memory-protection"))
                    .into_element();
            }
            // The route stays reachable from search where enzimOS is absent.
            _ => return self.mark_control_anchor(route, Space::new()),
        };
        let busy = self.busy_with(|r| matches!(r, Request::MemoryPolicy { .. }));
        let mut rows = column![].spacing(2);
        if self.system.memory_risk {
            rows = rows.push(
                self.mark_setting(
                    route,
                    crate::settings_app::settings_row(tr("shell-turn-memory-protection-off"))
                        .subtitle(memory_detail("off"))
                        .icon("triangle-alert")
                        .into_element(),
                ),
            );
            rows = rows.push(
                row![
                    Space::new().width(Length::Fill),
                    enzui_widgets::button::enz_button(
                        tr("shell-cancel"),
                        Parent::System(Message::MemoryRisk(false))
                    )
                    .kind(enzui_widgets::button::ButtonKind::Secondary)
                    .id("memory-off-cancel".into())
                    .into_element(),
                    enzui_widgets::button::enz_button(
                        tr("shell-i-understand-turn-off"),
                        Parent::System(Message::AcceptMemoryRisk)
                    )
                    .kind(enzui_widgets::button::ButtonKind::Destructive)
                    .id("memory-off-accept".into())
                    .enabled(self.system_writes())
                    .into_element(),
                ]
                .spacing(8)
                .padding([4, 12]),
            );
        } else {
            let choices = policy
                .available
                .iter()
                .map(|mode| {
                    let message = if mode == "off" {
                        Parent::System(Message::MemoryRisk(true))
                    } else {
                        Self::request(Request::MemoryPolicy {
                            mode: mode.clone(),
                            risk_acknowledged: false,
                        })
                    };
                    (memory_label(mode).to_owned(), message)
                })
                .collect();
            rows = rows.push(
                self.mark_setting(
                    route,
                    crate::settings_app::settings_row(tr("shell-protection"))
                        .icon("memory-stick")
                        .id_prefix(format!("control-{route}"))
                        .subtitle(memory_detail(&policy.mode))
                        .value_menu(
                            memory_label(&policy.mode).to_owned(),
                            self.menu.as_deref() == Some(route),
                            Parent::Menu(route.into(), true),
                            Parent::Menu(route.into(), false),
                            choices,
                        )
                        .enabled(self.system_writes() && !busy)
                        .into_element(),
                ),
            );
        }
        let (icon, status, detail) = memory_status(&policy, busy);
        rows = rows.push(
            crate::settings_app::settings_row(status)
                .icon(icon)
                .subtitle(detail)
                .id_prefix("memory-status")
                .into_element(),
        );
        // Which programs are never closed, said once in plain words; the
        // process names are an implementation detail.
        let desktop_kept = policy.protected_names.as_ref().is_some_and(|names| {
            names
                .iter()
                .any(|name| matches!(name.as_str(), "enz-hyprland" | "Hyprland" | "enzui-shell"))
        });
        rows = rows.push(match &policy.protected_names {
            Some(_) if desktop_kept => self.info_row(
                "monitor-check",
                tr("shell-memory-guard-protects-desktop"),
                tr("shell-memory-guard-protects-system-processes"),
            ),
            Some(_) => self.info_row(
                "shield-alert",
                tr("shell-the-desktop-is-not-protected"),
                tr("shell-parts-of-the-desktop-could-be-closed-when-memory-runs-out"),
            ),
            None => self.info_row(
                "shield-alert",
                tr("shell-protected-programs-could-not-be-checked"),
                tr("shell-parts-of-the-desktop-could-be-closed-when-memory-runs-out"),
            ),
        });
        self.section_card_for(route, rows)
            .title(tr("shell-memory-protection"))
            .description(tr("shell-when-memory-runs-out-an-app-may-be-closed-so-the-desktop-stays-usable-unsave"))
            .into_element()
    }

    fn tmp_storage_card<'a>(&'a self, power: Option<&Power>) -> Element<'a, Parent> {
        let route = "power/temporary_storage";
        let rows = match power.map(|power| &power.tmp_storage) {
            Some(Backed::Ready(storage)) => {
                let label = |mode: &str| {
                    if mode == "ram" {
                        tr("shell-tmp-storage-memory")
                    } else {
                        tr("shell-tmp-storage-disk")
                    }
                };
                let busy =
                    self.busy_with(|request| matches!(request, Request::TemporaryStorage(_)));
                let row = if let Some(configured) = &storage.configured {
                    let detail = if storage.active != *configured {
                        tr!("shell-tmp-storage-pending", "backend" => label(&storage.active))
                    } else {
                        tr("shell-tmp-storage-description").to_owned()
                    };
                    crate::settings_app::settings_row(tr("shell-tmp-storage-title"))
                        .icon("hard-drive")
                        .id_prefix("tmp-storage")
                        .subtitle(detail)
                        .value_menu(
                            label(configured).to_owned(),
                            self.menu.as_deref() == Some(route),
                            Parent::Menu(route.into(), true),
                            Parent::Menu(route.into(), false),
                            ["ram", "disk"]
                                .into_iter()
                                .map(|mode| {
                                    (
                                        label(mode).to_owned(),
                                        Self::request(Request::TemporaryStorage(mode.into())),
                                    )
                                })
                                .collect(),
                        )
                        .enabled(self.system_writes() && !busy)
                        .into_element()
                } else {
                    self.info_row(
                        "hard-drive",
                        label(&storage.active),
                        tr("shell-tmp-storage-custom"),
                    )
                };
                column![self.mark_setting(route, row)]
            }
            Some(Backed::Refused(error)) => column![self.mark_setting(
                route,
                self.info_row(
                    "hard-drive",
                    tr("shell-tmp-storage-unavailable"),
                    error.clone()
                ),
            )],
            _ => return self.mark_control_anchor(route, Space::new()),
        };
        self.section_card_for(route, rows)
            .title(tr("shell-tmp-storage-heading"))
            .into_element()
    }

    fn critical_rows<'a>(&'a self, power: Option<&Power>) -> (Element<'a, Parent>, bool) {
        let route = "power/critical";
        let critical = match power.map(|p| &p.critical) {
            Some(Backed::Ready(critical)) => critical.clone(),
            Some(Backed::Refused(error)) => {
                let row = self.info_row(
                    "battery-warning",
                    tr("shell-the-critical-battery-action-cannot-be-read"),
                    error.clone(),
                );
                return (self.mark_setting(route, row), true);
            }
            // The route stays reachable from search where enzimOS is absent.
            _ => return (self.mark_control_anchor(route, Space::new()), false),
        };
        let content: Element<'a, Parent> = match self.system.battery_risk.clone() {
            Some(action) => {
                let palette = self.tokens().palette;
                let (question, accept) = critical_confirmation(&action);
                column![
                    self.mark_setting(
                        route,
                        crate::settings_app::settings_row(question)
                            .subtitle(critical_risk(&action))
                            .subtitle_wrap(true)
                            .icon("triangle-alert")
                            .into_element()
                    ),
                    row![
                        Space::new().width(Length::Fill),
                        enzui_widgets::button::enz_button(
                            tr("shell-cancel"),
                            Parent::System(Message::BatteryRisk(None))
                        )
                        .kind(enzui_widgets::button::ButtonKind::Secondary)
                        .id("critical-battery-cancel".into())
                        .into_element(),
                        enzui_widgets::button::enz_button(
                            accept,
                            Parent::System(Message::AcceptBatteryRisk)
                        )
                        .kind(enzui_widgets::button::ButtonKind::Destructive)
                        .id("critical-battery-accept".into())
                        .enabled(self.system_writes())
                        .into_element(),
                    ]
                    .spacing(8)
                    .padding([4, 12]),
                    enz_text(tr("shell-your-computer-applies-this-from-its-next-start"))
                        .size(TextSize::Footnote)
                        .muted_on(palette.surface)
                        .into_element(),
                ]
                .spacing(6)
                .into()
            }
            None => {
                let choices = critical
                    .supported
                    .iter()
                    .map(|action| {
                        let message = if critical.risky.contains(action) {
                            Parent::System(Message::BatteryRisk(Some(action.clone())))
                        } else {
                            Self::request(Request::CriticalBattery {
                                action: action.clone(),
                                risk_acknowledged: false,
                            })
                        };
                        (critical_label(action).to_owned(), message)
                    })
                    .collect();
                let configured = critical_label(&critical.configured);
                let selected = if critical.supported.contains(&critical.configured) {
                    configured.to_owned()
                } else {
                    tr!("shell-configured-power-action-is-unavailable", "arg0" => configured.to_owned())
                };
                self.choice_setting(route, selected, choices)
            }
        };
        (content, true)
    }
    pub(super) fn datetime_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let body = body.push(
            self.section_card_for(
                "datetime/language",
                self.choice_setting(
                    "datetime/language",
                    self.settings.language.label(),
                    i18n::Language::ALL
                        .into_iter()
                        .map(|language| {
                            (
                                language.label().into(),
                                Parent::Change(enzui_shell::settings::Change::Language(language)),
                            )
                        })
                        .collect(),
                ),
            )
            .title(tr("language-title"))
            .description(tr("language-description"))
            .into_element(),
        );
        let clock = match self.reading(Panel::DateTime) {
            Some(Reading::DateTime(c)) => Some(c),
            _ => None,
        };
        let rows = match &clock {
            None => column![
                self.mark_setting("datetime/timezone", self.loading_row()),
                self.mark_setting("datetime/automatic", self.loading_row())
            ],
            Some(clock) => {
                let Clock {
                    timezone,
                    ntp,
                    can_ntp,
                    synchronized,
                    ..
                } = clock;
                let now = {
                    let now = chrono::Local::now();
                    format!(
                        "{}, {}",
                        i18n::date(now.date_naive(), true),
                        i18n::time(&now, self.settings.session.clock_24h)
                    )
                };
                let zone = timezone
                    .clone()
                    .unwrap_or_else(|| tr("shell-unknown").into())
                    .replace('_', " ");
                let status = match (ntp, synchronized) {
                    (Some(true), Some(true)) => tr("shell-synchronized-with-network-time"),
                    (Some(true), _) => tr("shell-waiting-for-network-time"),
                    _ => tr("shell-set-manually"),
                };
                let zone_row = crate::settings_app::settings_row(tr("shell-time-zone"))
                    .subtitle(format!("{} · {now}", zone.replace('/', " · ")))
                    .icon("globe")
                    .control(
                        enzui_widgets::button::enz_button(
                            tr("shell-change"),
                            Parent::System(Message::Form(super::forms::Form::ZoneQuery(Some(
                                String::new(),
                            )))),
                        )
                        .kind(enzui_widgets::button::ButtonKind::Secondary)
                        .size(enzui_widgets::button::ButtonSize::Sm)
                        .id("datetime-zone-change".into())
                        .enabled(self.system_writes() && !clock.zones.is_empty())
                        .into_element(),
                    )
                    .into_element();
                let mut rows = column![self.mark_setting("datetime/timezone", zone_row)].spacing(2);
                if let Some(query) = &self.system.forms.zone_query {
                    rows = rows.push(self.zone_picker(&clock.zones, query, false));
                }
                column![
                    rows,
                    self.mark_setting(
                        "datetime/automatic",
                        crate::settings_app::settings_row(tr("shell-set-time-automatically"))
                            .subtitle(status)
                            .icon("refresh-cw")
                            .id_prefix("control-datetime/automatic")
                            .switch(*ntp == Some(true), |on| Self::request(Request::Ntp(on)))
                            .enabled(self.system_writes() && *can_ntp)
                            .into_element(),
                    ),
                ]
            }
        }
        .spacing(2)
        .push(self.switch_setting(
            "datetime/clock_24h",
            self.settings.session.clock_24h,
            |on| {
                Parent::Change(enzui_shell::settings::Change::Session(
                    enzui_shell::session_preferences::SessionChange::Clock24h(on),
                ))
            },
        ));
        body.push(
            self.section_card_for("datetime/timezone", rows)
                .title(tr("shell-time"))
                .description(tr(
                    "shell-the-clock-follows-the-time-zone-and-when-on-network-time",
                ))
                .into_element(),
        )
        .push(
            self.section_card_for(
                "datetime/world_clocks",
                self.world_clock_rows(
                    clock
                        .as_ref()
                        .map(|c| c.zones.as_slice())
                        .unwrap_or_default(),
                ),
            )
            .title(tr("shell-world-clocks"))
            .description(tr(
                "shell-shown-under-the-calendar-when-you-open-the-clock-in-the-bar",
            ))
            .into_element(),
        )
    }
    fn world_clock_rows<'a>(&'a self, zones: &[String]) -> Element<'a, Parent> {
        use super::forms::Form;
        use enzui_shell::settings::Change;
        let mut rows = column![].spacing(2);
        for zone in &self.settings.world_clocks {
            let (region, _) = zone.rsplit_once('/').unwrap_or(("", zone));
            rows = rows.push(
                crate::settings_app::settings_row(enzui_shell::world_clocks::city(zone))
                    .subtitle(region.replace('_', " ").replace('/', " · "))
                    .icon("clock")
                    .action(
                        tr("shell-remove"),
                        Parent::Change(Change::WorldClockRemove(zone.clone())),
                    )
                    .id(format!("datetime-clock-{zone}").into())
                    .enabled(self.controls_live())
                    .into_element(),
            );
        }
        let full = self.settings.world_clocks.len() >= enzui_shell::world_clocks::MAX;
        let add = crate::settings_app::settings_row(if self.settings.world_clocks.is_empty() {
            tr("shell-no-world-clocks")
        } else {
            tr("shell-add-another")
        })
        .icon("plus")
        .control(
            enzui_widgets::button::enz_button(
                tr("shell-add-2"),
                Parent::System(Message::Form(Form::ClockQuery(Some(String::new())))),
            )
            .kind(enzui_widgets::button::ButtonKind::Secondary)
            .size(enzui_widgets::button::ButtonSize::Sm)
            .id("datetime-clock-add".into())
            .enabled(self.controls_live() && !full && !zones.is_empty())
            .into_element(),
        )
        .into_element();
        rows = rows.push(self.mark_setting("datetime/world_clocks", add));
        if let Some(query) = &self.system.forms.clock_query {
            rows = rows.push(self.zone_picker(zones, query, true));
        }
        rows.into()
    }
    fn zone_picker<'a>(
        &'a self,
        zones: &[String],
        query: &'a str,
        clock: bool,
    ) -> Element<'a, Parent> {
        use super::forms::Form;
        let set_query = move |q: Option<String>| {
            if clock {
                Parent::System(Message::Form(Form::ClockQuery(q)))
            } else {
                Parent::System(Message::Form(Form::ZoneQuery(q)))
            }
        };
        let needle = query.trim().to_lowercase().replace(' ', "_");
        let matches: Vec<&String> = zones
            .iter()
            .filter(|z| needle.is_empty() || z.to_lowercase().contains(&needle))
            .take(8)
            .collect();
        let mut picker = column![
            iced::widget::row![
                enzui_widgets::search_field::enz_search_field(query, move |q| set_query(Some(q)))
                    .placeholder(tr("shell-search-by-city-or-region"))
                    .id(if clock {
                        "datetime-clock-search"
                    } else {
                        "datetime-zone-search"
                    })
                    .into_element(),
                enzui_widgets::button::enz_button(tr("shell-cancel"), set_query(None))
                    .id("datetime-search-cancel".into())
                    .kind(enzui_widgets::button::ButtonKind::Plain)
                    .into_element(),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(4);
        if matches.is_empty() {
            picker = picker.push(self.caption(tr("shell-no-time-zones-match")));
        }
        for zone in matches {
            let (region, city) = zone.rsplit_once('/').unwrap_or(("", zone));
            picker = picker.push(
                crate::settings_app::settings_row(city.replace('_', " "))
                    .subtitle(region.replace('_', " ").replace('/', " · "))
                    .action(
                        if clock {
                            tr("shell-add-2")
                        } else {
                            tr("shell-choose")
                        },
                        if clock {
                            Parent::System(Message::ChooseWorldClock(zone.clone()))
                        } else {
                            Self::request(Request::Timezone(zone.clone()))
                        },
                    )
                    .id(format!("datetime-{}-{zone}", if clock { "pick" } else { "zone" }).into())
                    .enabled(if clock {
                        self.controls_live()
                    } else {
                        self.system_writes()
                    })
                    .into_element(),
            );
        }
        container(picker).padding([8, 12]).into()
    }
    pub(super) fn about_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let about = match self.reading(Panel::About) {
            Some(Reading::About(a)) => *a,
            _ => About::default(),
        };
        let unknown = || "—".to_owned();
        let palette = self.tokens().palette;
        // enzimOS identifies itself with its mark; other systems keep the device glyph.
        let badge: Element<'a, Parent> = if about
            .os
            .as_deref()
            .is_some_and(enzui_shell::brand::is_enzimos)
        {
            enzui_shell::brand::mark(56.0).into_element()
        } else {
            container(enz_icon("monitor").size(30.0).into_element())
                .padding(14)
                .style(move |_| container::Style {
                    background: Some(palette.accent_soft.into()),
                    border: iced::Border {
                        radius: 16.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
        };
        let hero = container(
            row![
                badge,
                column![
                    enz_text(about.hostname.clone().unwrap_or_else(unknown))
                        .size(TextSize::Title2)
                        .into_element(),
                    enz_text(enzui_shell::tr!("shell-enzui-desktop-2", "arg0" => enzui_shell::i18n::argument(&(about.os.clone().unwrap_or_else(|| "Linux".into())))))
                    .muted_on(palette.surface)
                    .into_element(),
                ]
                .spacing(3),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding([12, 12]);
        let rename: Element<'a, Parent> = match &self.system.forms.hostname {
            None => crate::settings_app::settings_row(tr("shell-device-name"))
                .subtitle(tr(
                    "shell-how-this-computer-appears-to-other-devices-on-the-network",
                ))
                .control(
                    enzui_widgets::button::enz_button(
                        tr("shell-rename"),
                        Parent::System(Message::Form(super::forms::Form::Hostname(
                            about.hostname.clone(),
                        ))),
                    )
                    .kind(enzui_widgets::button::ButtonKind::Secondary)
                    .size(enzui_widgets::button::ButtonSize::Sm)
                    .id("about-rename".into())
                    .enabled(self.system_writes())
                    .into_element(),
                )
                .into_element(),
            Some(name) => {
                use super::forms::Form;
                container(
                    column![
                        enzui_widgets::text_field::enz_text_field(
                            enzui_shell::i18n::current().text("shell-device-name"),
                            "my-laptop",
                            name,
                            |t| Parent::System(Message::Form(Form::Hostname(Some(t))))
                        )
                        .description(
                            enzui_shell::i18n::current().text("shell-letters-numbers-and-hyphens")
                        )
                        .on_submit(Parent::System(Message::Form(Form::HostnameSave)))
                        .id("about-hostname".into())
                        .into_element(),
                        iced::widget::row![
                            Space::new().width(Length::Fill),
                            enzui_widgets::button::enz_button(
                                tr("shell-cancel"),
                                Parent::System(Message::Form(Form::Hostname(None)))
                            )
                            .id("hostname-cancel".into())
                            .kind(enzui_widgets::button::ButtonKind::Plain)
                            .into_element(),
                            enzui_widgets::button::enz_button(
                                enzui_shell::i18n::tr("shell-save"),
                                Parent::System(Message::Form(Form::HostnameSave))
                            )
                            .id("about-hostname-save".into())
                            .enabled(self.system_writes() && !name.trim().is_empty())
                            .into_element(),
                        ]
                        .spacing(8),
                    ]
                    .spacing(8),
                )
                .padding([8, 12])
                .into()
            }
        };
        let device = column![self.mark_setting("about/device", hero), rename].spacing(2);
        let system = column![
            self.mark_setting(
                "about/system",
                self.property_row(
                    tr("shell-operating-system"),
                    about.os.clone().unwrap_or_else(unknown)
                )
            ),
            self.property_row(enzui_shell::i18n::tr("shell-kernel"), about.kernel.clone().unwrap_or_else(unknown)),
            self.property_row(
                enzui_shell::i18n::tr("shell-desktop"),
                about
                    .compositor
                    .as_ref()
                    .map_or_else(|| "enzUI".to_owned(), |c| enzui_shell::tr!("shell-enzui-on", "arg0" => enzui_shell::i18n::argument(&(c))))
            ),
        ]
        .spacing(2);
        let mut system = system;
        if let Some(network) = self.network_elsewhere() {
            system = system.push(self.property_row(tr("shell-network"), network));
        }
        if let Some(enzimos) = &about.enzimos {
            if !enzimos.firmware.is_empty() {
                let firmware = match enzimos.firmware.as_str() {
                    "uefi" => "UEFI".to_owned(),
                    "bios" => "BIOS (legacy)".to_owned(),
                    other => other.to_owned(),
                };
                system = system.push(self.property_row(tr("shell-firmware"), firmware));
            }
            // Package versions are for support and developers: folded away.
            if !enzimos.packages.is_empty() {
                let open = self.system.about_details;
                let mut details = column![].spacing(2);
                for (name, version) in &enzimos.packages {
                    details = details.push(self.detail_row_owned(name.clone(), version.clone()));
                }
                system = system.push(
                    crate::settings_app::settings_row(tr("shell-technical-details"))
                        .action(
                            if open {
                                tr("shell-hide")
                            } else {
                                tr("shell-show")
                            },
                            Parent::System(Message::AboutDetails(!open)),
                        )
                        .id("about-technical-details".into())
                        .into_element(),
                );
                if open {
                    system = system.push(details);
                }
            }
        }
        let processor = match (&about.cpu, about.threads) {
            (Some(cpu), Some(threads)) => format!("{cpu} × {threads}"),
            (Some(cpu), None) => cpu.clone(),
            _ => unknown(),
        };
        let memory = about.memory_kib.map_or_else(
            unknown,
            |kib| tr!("shell-gib-2", "arg0" => enzui_shell::i18n::number(format!("{:.1}", kib as f64 / 1024.0 / 1024.0))),
        );
        let graphics = if about.gpus.is_empty() {
            unknown()
        } else {
            about.gpus.join(", ")
        };
        let disk = about.disk.map_or_else(
            unknown,
            |(total, free)| tr!("shell-free-of", "arg0" => bytes(free), "arg1" => bytes(total)),
        );
        let hardware = column![
            self.mark_setting(
                "about/hardware",
                self.property_row(enzui_shell::i18n::tr("shell-processor"), processor)
            ),
            self.property_row(enzui_shell::i18n::tr("shell-memory"), memory),
            self.property_row(enzui_shell::i18n::tr("shell-graphics"), graphics),
            self.property_row(enzui_shell::i18n::tr("shell-disk"), disk),
        ]
        .spacing(2);
        let mut problems = column![].spacing(2);
        match &about.crashes {
            None => {
                problems = problems.push(self.mark_setting(
                    "about/problems",
                    self.property_row(
                        tr("shell-reports"),
                        tr("shell-not-available-on-this-system"),
                    ),
                ));
            }
            Some(reports) if reports.is_empty() => {
                problems = problems.push(self.mark_setting(
                    "about/problems",
                    self.property_row(tr("shell-last-day"), tr("shell-no-app-has-crashed")),
                ));
            }
            Some(reports) => {
                let mut rows = column![].spacing(2);
                for report in reports.iter().rev().take(10) {
                    let when = chrono::DateTime::parse_from_rfc3339(&report.timestamp).map_or_else(
                        |_| report.timestamp.clone(),
                        |t| i18n::local_timestamp(t, self.settings.session.clock_24h),
                    );
                    let what = if report.signal.is_empty() {
                        when
                    } else {
                        format!("{when} · {}", report.signal)
                    };
                    rows = rows.push(self.property_row(report.app(), what));
                }
                problems = problems.push(self.mark_setting("about/problems", rows));
            }
        }
        body.push(self.section_card_for("about/device", device).title(tr("shell-device")).into_element())
            .push(self.section_card_for("about/system", system).title(tr("shell-system")).into_element())
            .push(self.section_card_for("about/hardware", hardware).title(tr("shell-hardware")).into_element())
            .push(
                self.section_card_for("about/problems", problems)
                    .title(tr("shell-problem-reports"))
                    .description(tr("shell-apps-that-quit-unexpectedly-in-the-last-day-reports-stay-on-this-computer-no"))
                    .into_element(),
            )
    }
}
