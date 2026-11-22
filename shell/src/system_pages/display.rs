use super::*;
use enzui_shell::i18n::{self, tr};
use enzui_shell::system_settings::display::{
    self, Change, Displays, Mode, Output, Rotation, SCALES, Side,
};
use enzui_shell::tr;
use enzui_widgets::pressable::{PressableStyle, enz_pressable};
use iced::{
    Alignment, Length,
    widget::{Space, column, container, pin, stack},
};

pub(super) const CONFIRM: Duration = Duration::from_secs(15);
const PREVIEW: (f32, f32) = (560.0, 190.0);

fn preview_geometry(outputs: &[&Output]) -> ((f64, f64), f64, (f64, f64)) {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for output in outputs {
        let (w, h) = output.logical_size();
        min_x = min_x.min(f64::from(output.position.0));
        min_y = min_y.min(f64::from(output.position.1));
        max_x = max_x.max(f64::from(output.position.0) + w);
        max_y = max_y.max(f64::from(output.position.1) + h);
    }
    let span = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let fit = (f64::from(PREVIEW.0) / span.0).min(f64::from(PREVIEW.1) / span.1) * 0.92;
    let offset = (
        (f64::from(PREVIEW.0) - span.0 * fit) / 2.0,
        (f64::from(PREVIEW.1) - span.1 * fit) / 2.0,
    );
    ((min_x, min_y), fit, offset)
}

pub(super) fn tile_at(displays: &Displays, point: iced::Point) -> Option<String> {
    let enabled: Vec<&Output> = displays
        .outputs
        .iter()
        .filter(|o| o.enabled && o.current_mode().is_some())
        .collect();
    let ((min_x, min_y), fit, offset) = preview_geometry(&enabled);
    enabled.iter().rev().find_map(|output| {
        let (w, h) = output.logical_size();
        let x = offset.0 + (f64::from(output.position.0) - min_x) * fit;
        let y = offset.1 + (f64::from(output.position.1) - min_y) * fit;
        let (px, py) = (f64::from(point.x), f64::from(point.y));
        (px >= x && px <= x + w * fit && py >= y && py <= y + h * fit).then(|| output.name.clone())
    })
}

// Snaps a dragged display to the nearest main-display edge; returns `None` for a click.
pub(super) fn dropped_layout(
    displays: &Displays,
    name: &str,
    moved: (f32, f32),
) -> Option<Vec<Change>> {
    if moved.0.hypot(moved.1) < 6.0 {
        return None;
    }
    let enabled: Vec<&Output> = displays
        .outputs
        .iter()
        .filter(|o| o.enabled && o.current_mode().is_some())
        .collect();
    let (_, fit, _) = preview_geometry(&enabled);
    let mut layout = display::layout(displays);
    let change = layout.iter_mut().find(|c| c.output == name && c.enabled)?;
    change.position.0 += (f64::from(moved.0) / fit).round() as i32;
    change.position.1 += (f64::from(moved.1) / fit).round() as i32;
    let sides = display::sides(&layout);
    display::pack(&mut layout, &sides);
    Some(layout)
}

/// Whether `scale` leaves a usable desktop on `mode`. The compositor accepts any listed scale
/// and rounds the logical size, so fractional results such as 1804.8 pixels are fine; the
/// limit matches the automatic scale policy in `config/desktop-scale.lua`.
fn supports_scale(mode: Mode, scale: f64) -> bool {
    let (w, h) = (
        (f64::from(mode.width) / scale).round(),
        (f64::from(mode.height) / scale).round(),
    );
    (scale <= 1.0 && w.max(h) <= 16384.0) || (w.max(h) >= 1024.0 && w.min(h) >= 640.0)
}

fn scale_label(scale: f64) -> String {
    format!("{:.0}%", scale * 100.0)
}

fn edited(
    displays: &Displays,
    name: &str,
    edit: impl FnOnce(&mut Change),
    side: Option<Side>,
) -> Vec<Change> {
    let mut layout = display::layout(displays);
    let mut sides = display::sides(&layout);
    if let Some(change) = layout.iter_mut().find(|c| c.output == name) {
        let was_enabled = change.enabled;
        edit(change);
        if change.enabled && !was_enabled {
            sides.push((name.to_owned(), Side::Right));
        }
    }
    if let Some(side) = side {
        sides.retain(|(n, _)| n != name);
        sides.push((name.to_owned(), side));
    }
    display::pack(&mut layout, &sides);
    layout
}

impl SettingsApp {
    pub(super) fn display_selected<'a>(&self, displays: &'a Displays) -> Option<&'a Output> {
        let chosen = self.system.display_selected.as_deref();
        displays
            .outputs
            .iter()
            .find(|o| Some(o.name.as_str()) == chosen)
            .or_else(|| displays.outputs.iter().find(|o| o.enabled))
            // Every output can be off for a moment, while one is plugged in.
            .or_else(|| displays.outputs.first())
    }
    pub(super) fn request_display(&mut self, layout: Vec<Change>) -> Task<Parent> {
        let previous = match self.system.readings.get(&Panel::Display) {
            Some(Reading::Display(d)) => display::layout(d),
            _ => Vec::new(),
        };
        if previous == layout {
            return Task::none();
        }
        // A revert keeps the original layout, not the one being undone.
        if self.system.display_confirm.is_none() && !previous.is_empty() {
            self.system.display_confirm = Some((previous, Instant::now() + CONFIRM));
        }
        self.system_message(Message::Request(Request::Display(layout)))
    }
    fn arrangement<'a>(&self, displays: &Displays, selected: &str) -> Element<'a, Parent> {
        let enabled: Vec<&Output> = displays
            .outputs
            .iter()
            .filter(|o| o.enabled && o.current_mode().is_some())
            .collect();
        let ((min_x, min_y), fit, offset) = preview_geometry(&enabled);
        let palette = self.tokens().palette;
        let dragged = self
            .system
            .display_drag
            .as_ref()
            .and_then(|(name, from, to)| {
                Some((
                    name.clone(),
                    (to.x - from.as_ref()?.x, to.y - from.as_ref()?.y),
                ))
            });
        let mut layers = stack![Space::new().width(PREVIEW.0).height(PREVIEW.1)];
        for (index, output) in enabled.iter().enumerate() {
            let (w, h) = output.logical_size();
            let moved = dragged
                .as_ref()
                .filter(|(name, _)| *name == output.name)
                .map_or((0.0, 0.0), |(_, moved)| {
                    (f64::from(moved.0), f64::from(moved.1))
                });
            let x = offset.0 + (f64::from(output.position.0) - min_x) * fit + moved.0;
            let y = offset.1 + (f64::from(output.position.1) - min_y) * fit + moved.1;
            let active = output.name == selected;
            let label = column![
                enz_text(format!("{}", index + 1))
                    .size(TextSize::Title2)
                    .into_element(),
                enz_text(output.name.clone())
                    .size(TextSize::Caption)
                    .muted_on(palette.surface_alt)
                    .into_element(),
            ]
            .align_x(Alignment::Center)
            .spacing(2);
            let tile = enz_pressable(
                container(label)
                    .center_x(Length::Fixed((w * fit - 4.0).max(24.0) as f32))
                    .center_y(Length::Fixed((h * fit - 4.0).max(18.0) as f32))
                    .style(move |_| container::Style {
                        background: Some(if active { palette.accent_soft } else { palette.surface_alt }.into()),
                        border: iced::Border {
                            radius: 8.0.into(),
                            width: if active { 2.0 } else { 1.0 },
                            color: if active { palette.accent } else { palette.border_strong },
                        },
                        ..Default::default()
                    })
                    .into(),
                Parent::System(Message::SelectDisplay(output.name.clone())),
            )
            .style(PressableStyle::Plain)
            .selected(active)
            .selectable(true)
            .aria_label(tr!("shell-display-2", "arg0" => format!("{:}", index + 1), "arg1" => output.title().clone()))
            .id(format!("display-tile-{}-{active}", output.name).into())
            .radius(8.0)
            .border_width(0.0)
            .width(Length::Fixed((w * fit - 4.0).max(24.0) as f32))
            .into_element();
            layers = layers.push(pin(tile).x(x as f32 + 2.0).y(y as f32 + 2.0));
        }
        // The pointer acts on a layer above the tiles: a press picks the tile
        // under it and starts a drag. The tiles below stay the keyboard's.
        layers = layers.push(
            iced::widget::mouse_area(Space::new().width(PREVIEW.0).height(PREVIEW.1))
                .on_move(|point| Parent::System(Message::DisplayDragMove(point)))
                .on_press(Parent::System(Message::DisplayDragStart))
                .on_release(Parent::System(Message::DisplayDragEnd(true)))
                // Leaving the preview drops the display where it left.
                .on_exit(Parent::System(Message::DisplayDragEnd(true))),
        );
        column![
            container(layers).center_x(Length::Fill).padding([12, 12]),
            container(self.caption(if enabled.len() > 1 {
                tr("shell-drag-a-display-to-move-it-next-to-another-it-snaps-edge-to-edge")
            } else {
                tr("shell-select-a-display-to-change-it")
            }))
            .padding([0, 12]),
        ]
        .into()
    }
    fn display_rows<'a>(&self, output: &Output, displays: &Displays) -> Element<'a, Parent> {
        let enabled = displays.outputs.iter().filter(|o| o.enabled).count();
        let mut rows = column![].spacing(2);
        if displays.outputs.len() > 1 {
            let choices = displays
                .outputs
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    (
                        format!("{}. {}", i + 1, o.title()),
                        Parent::System(Message::SelectDisplay(o.name.clone())),
                    )
                })
                .collect();
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-display"))
                    .icon("monitor")
                    .id_prefix("control-display/choose")
                    .value_menu(
                        output.title(),
                        self.menu.as_deref() == Some("display/choose"),
                        Parent::Menu("display/choose".into(), true),
                        Parent::Menu("display/choose".into(), false),
                        choices,
                    )
                    .enabled(self.controls_live())
                    .into_element(),
            );
            let name = output.name.clone();
            let owned = displays.clone();
            // The last display that is on cannot be turned off.
            let can_toggle = !output.enabled || enabled > 1;
            let mut toggle = crate::settings_app::settings_row(tr("shell-use-this-display"))
                .icon("power")
                .id_prefix("control-display/enabled")
                .switch(output.enabled, move |on| {
                    Parent::System(Message::SetDisplay(edited(
                        &owned,
                        &name,
                        |c| c.enabled = on,
                        None,
                    )))
                });
            if !can_toggle {
                toggle = toggle.subtitle(tr("shell-at-least-one-display-stays-on"));
            }
            rows = rows.push(
                self.mark_setting(
                    "display/enabled",
                    toggle
                        .enabled(self.controls_live() && can_toggle)
                        .into_element(),
                ),
            );
        } else {
            rows = rows.push(self.mark_control_anchor("display/enabled", Space::new()));
        }
        let (Some(current), true) = (output.current_mode(), output.enabled) else {
            return rows
                .push(self.info_row(
                    "monitor-off",
                    tr("shell-this-display-is-off"),
                    tr("shell-turn-it-on-to-choose-its-resolution-and-orientation"),
                ))
                .push(self.mark_setting("display/resolution", Space::new()))
                .push(self.mark_setting("display/refresh", Space::new()))
                .push(self.mark_setting("display/scale", Space::new()))
                .push(self.mark_setting("display/rotation", Space::new()))
                .push(self.mark_setting("display/color", Space::new()))
                .into();
        };
        let request = |edit: Box<dyn FnOnce(&mut Change)>| {
            Parent::System(Message::SetDisplay(edited(
                displays,
                &output.name,
                edit,
                None,
            )))
        };
        let mut resolutions: Vec<(i32, i32)> = Vec::new();
        let mut sorted = output.modes.clone();
        sorted.sort_by_key(|m| std::cmp::Reverse((m.width * m.height, m.refresh_mhz)));
        for mode in &sorted {
            if !resolutions.contains(&(mode.width, mode.height)) {
                resolutions.push((mode.width, mode.height));
            }
        }
        let preferred = output
            .modes
            .iter()
            .find(|m| m.preferred)
            .map(|m| (m.width, m.height));
        let resolution_choices = resolutions
            .iter()
            .map(|&(w, h)| {
                let best: Mode = sorted
                    .iter()
                    .find(|m| (m.width, m.height) == (w, h))
                    .copied()
                    .unwrap_or(current);
                let mut label = format!("{w} × {h}");
                if preferred == Some((w, h)) {
                    label = format!("{label} ({})", tr("shell-recommended").to_lowercase());
                }
                (
                    label,
                    request(Box::new(move |c| {
                        c.mode = best;
                        if !supports_scale(best, c.scale) {
                            c.scale = SCALES
                                .into_iter()
                                .filter(|&scale| supports_scale(best, scale))
                                .min_by(|a, b| (a - c.scale).abs().total_cmp(&(b - c.scale).abs()))
                                .unwrap_or(1.0);
                        }
                    })),
                )
            })
            .collect();
        let mut refresh_choices: Vec<(String, Parent)> = Vec::new();
        // Sorted fastest first, so 59.95 and 60 Hz collapse to the faster one.
        for &m in sorted
            .iter()
            .filter(|m| (m.width, m.height) == (current.width, current.height))
        {
            if !refresh_choices
                .iter()
                .any(|(label, _)| *label == m.refresh())
            {
                refresh_choices.push((m.refresh(), request(Box::new(move |c| c.mode = m))));
            }
        }
        let scale_choices = SCALES
            .iter()
            .filter(|&&scale| scale == output.scale || supports_scale(current, scale))
            .map(|&s| (scale_label(s), request(Box::new(move |c| c.scale = s))))
            .collect();
        let rotation_choices = Rotation::ALL
            .iter()
            .map(|&r| {
                (
                    r.label().to_owned(),
                    request(Box::new(move |c| c.rotation = r)),
                )
            })
            .collect();
        let (lw, lh) = output.logical_size();
        let color = self.color_row(&output.name);
        rows.push(self.choice_setting("display/resolution", current.resolution(), resolution_choices))
            .push(self.choice_setting("display/refresh", current.refresh(), refresh_choices))
            .push(self.choice_setting("display/scale", scale_label(output.scale), scale_choices))
            .push(self.choice_setting("display/rotation", output.rotation.label(), rotation_choices))
            .push(color)
            .push(container(self.caption(tr!("shell-apps-see-a-desktop-on-this-display", "arg0" => enzui_shell::i18n::number(format!("{lw:.0}")), "arg1" => enzui_shell::i18n::number(format!("{lh:.0}"))))).padding([4, 12]))
            .into()
    }
    fn color_row<'a>(&self, name: &str) -> Element<'a, Parent> {
        use enzui_shell::desktop_preferences::DisplayChange;
        let Some(support) = self.display_colors.iter().find(|s| s.name == name) else {
            return self.mark_control_anchor("display/color", Space::new());
        };
        let label = |mode: &str| match mode {
            "wide" => tr("shell-wide-colour"),
            "hdr" => "HDR",
            _ => enzui_shell::i18n::tr("shell-standard-srgb"),
        };
        let set = |mode: Option<&str>| {
            Parent::Change(enzui_shell::settings::Change::Display(
                DisplayChange::ColorMode {
                    output: name.to_owned(),
                    mode: mode.map(str::to_owned),
                },
            ))
        };
        let wanted = self
            .settings
            .compositor
            .color_modes
            .get(name)
            .map_or("srgb", String::as_str);
        let mut choices = vec![(label("srgb").to_owned(), set(None))];
        if support.wide || support.hdr {
            choices.push((label("wide").to_owned(), set(Some("wide"))));
        }
        if support.hdr {
            choices.push((label("hdr").to_owned(), set(Some("hdr"))));
        }
        if choices.len() == 1 {
            return self.mark_setting(
                "display/color",
                crate::settings_app::settings_row(tr("shell-colour"))
                    .icon("palette")
                    .subtitle(tr(
                        "shell-standard-srgb-this-display-does-not-report-wide-colour-or-hdr",
                    ))
                    .subtitle_wrap(true)
                    .into_element(),
            );
        }
        self.choice_setting("display/color", label(wanted), choices)
    }
    fn position_row<'a>(&self, output: &Output, displays: &Displays) -> Element<'a, Parent> {
        let layout = display::layout(displays);
        let main = layout.iter().find(|c| c.enabled).map(|c| c.output.clone());
        let side = display::sides(&layout)
            .into_iter()
            .find(|(n, _)| *n == output.name)
            .map(|(_, s)| s);
        let (Some(main), Some(side)) = (main, side) else {
            return self.mark_setting("display/position", Space::new());
        };
        let main_title = displays
            .outputs
            .iter()
            .find(|o| o.name == main)
            .map(Output::title)
            .unwrap_or(main);
        let choices = Side::ALL
            .iter()
            .map(|&s| {
                (
                    format!("{} {main_title}", s.label()),
                    Parent::System(Message::SetDisplay(edited(
                        displays,
                        &output.name,
                        |_| {},
                        Some(s),
                    ))),
                )
            })
            .collect();
        self.choice_setting(
            "display/position",
            format!("{} {main_title}", side.label()),
            choices,
        )
    }
    // Mounted in the window's notification band so it stays in view while
    // the page scrolls.
    pub(crate) fn display_confirm_notice(
        &self,
    ) -> Option<enzui_widgets::notice_stack::EnzNotice<Parent>> {
        let (_, deadline) = self.system.display_confirm.as_ref()?;
        let left = deadline.saturating_duration_since(Instant::now()).as_secs();
        Some(
            enzui_widgets::notice_stack::EnzNotice::new(
                "display-confirm",
                tr("shell-keep-these-display-settings"),
                enzui_widgets::toast::ToastTone::Warning,
            )
            .description(enzui_shell::tr!("shell-reverting-in-s-if-you-do-nothing", "arg0" => enzui_shell::i18n::argument(&(left))))
            .action(tr("shell-keep-changes"), Parent::System(Message::KeepDisplay))
            .secondary_action(tr("shell-revert"), Parent::System(Message::RevertDisplay)),
        )
    }
    pub(super) fn display_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let displays = match self.reading(Panel::Display) {
            Some(Reading::Display(d)) => Some(d),
            _ => None,
        };
        let (arrangement, settings, subtitle) = match &displays {
            None => (
                self.loading_row(),
                column![
                    self.mark_setting("display/resolution", self.loading_row()),
                    self.mark_setting("display/refresh", Space::new()),
                    self.mark_setting("display/scale", Space::new()),
                    self.mark_setting("display/rotation", Space::new()),
                    self.mark_setting("display/color", Space::new()),
                    self.mark_setting("display/enabled", Space::new()),
                    self.mark_setting("display/position", Space::new()),
                    self.mark_setting("display/mirror", Space::new()),
                ]
                .into(),
                String::new(),
            ),
            Some(d) if !d.supported || d.outputs.is_empty() => (
                self.info_row(
                    "monitor",
                    tr("shell-display-settings-are-unavailable"),
                    tr("shell-the-compositor-does-not-offer-the-wlr-output-management-protocol"),
                ),
                column![
                    self.mark_setting(
                        "display/resolution",
                        self.info_row("monitor", tr("shell-no-displays-to-configure"), "")
                    ),
                    self.mark_setting("display/refresh", Space::new()),
                    self.mark_setting("display/scale", Space::new()),
                    self.mark_setting("display/rotation", Space::new()),
                    self.mark_setting("display/color", Space::new()),
                    self.mark_setting("display/enabled", Space::new()),
                    self.mark_setting("display/position", Space::new()),
                    self.mark_setting("display/mirror", Space::new()),
                ]
                .into(),
                String::new(),
            ),
            Some(d) => {
                let output = self.display_selected(d).expect("A display is listed");
                let mut parts = vec![output.title()];
                if let Some(inches) = output.diagonal_inches() {
                    parts.push(format!("{inches:.0}″"));
                }
                if output.title() != output.name {
                    parts.push(output.name.clone());
                }
                let mirror = if d.outputs.len() > 1 || self.settings.display_mirror {
                    self.switch_setting("display/mirror", self.settings.display_mirror, |on| {
                        Parent::Change(enzui_shell::settings::Change::DisplayMirror(on))
                    })
                } else {
                    self.mark_control_anchor("display/mirror", Space::new())
                };
                let arrangement = if self.settings.display_mirror {
                    column![
                        self.info_row(
                            "copy",
                            tr("shell-every-display-shows-the-main-display"),
                            tr("shell-turn-mirroring-off-to-arrange-the-displays-side-by-side")
                        ),
                        mirror
                    ]
                    .spacing(2)
                    .into()
                } else {
                    column![
                        self.arrangement(d, &output.name),
                        self.position_row(output, d),
                        mirror
                    ]
                    .spacing(2)
                    .into()
                };
                (arrangement, self.display_rows(output, d), parts.join(" · "))
            }
        };
        body.push(
            self.section_card_for("display/arrangement", self.anchor_setting("display/arrangement", arrangement))
                .title(tr("shell-arrangement"))
                .description(tr("shell-select-a-display-to-change-it-kept-changes-are-restored-whenever-these-displ"))
                .into_element(),
        )
        .push(
            self.section_card_for("display/resolution", settings)
                .title(tr("shell-display-settings"))
                .description(if subtitle.is_empty() { tr("shell-resolution-refresh-rate-and-scale") } else { &subtitle })
                .into_element(),
        )
        .push({
            use enzui_shell::desktop_preferences::DisplayChange;
            let sharp = self.switch_setting_noted(
                "display/x11_sharp",
                self.settings.compositor.x11_sharp,
                tr("shell-they-may-then-look-small-unless-they-have-their-own-size-setting"),
                |on| Parent::Change(enzui_shell::settings::Change::Display(DisplayChange::X11Sharp(on))),
            );
            self.section_card_for("display/x11_sharp", sharp)
                .title(tr("shell-legacy-apps"))
                .description(tr("shell-some-older-apps-can-t-follow-display-scaling-and-look-slightly-blurry"))
                .into_element()
        })
        .push(
            self.section_card_for("display/night_light", self.night_light_rows())
                .title(tr("shell-night-light"))
                .description(tr("shell-warmer-colours-in-the-evening-are-easier-on-the-eyes"))
                .into_element(),
        )
    }

    pub(in crate::settings_app) fn sync_night_labels(&mut self) {
        use enzui_core::time_of_day::{DayPeriodLabels, HourCycle, TimeFormat, TimeOfDay};
        let localizer = i18n::current();
        let format = TimeFormat::new(if self.settings.session.clock_24h {
            HourCycle::H23
        } else {
            HourCycle::H12
        })
        .with_day_period_labels(DayPeriodLabels::new(
            localizer.text("enzui-time-am"),
            localizer.text("enzui-time-pm"),
        ));
        let text = |minutes: u16| {
            format.format(
                TimeOfDay::hm(u32::from(minutes / 60), u32::from(minutes % 60))
                    .unwrap_or(TimeOfDay::MIDNIGHT),
            )
        };
        self.system.night_text = (
            text(self.settings.night_light.from),
            text(self.settings.night_light.to),
        );
    }

    fn night_light_rows(&self) -> Element<'_, Parent> {
        use enzui_core::time_of_day::{HourCycle, TimeFormat, TimeOfDay};
        use enzui_shell::night_light::{COOLEST, Change as Night, WARMEST};
        let night = self.settings.night_light;
        let change = |c: Night| Parent::Change(enzui_shell::settings::Change::NightLight(c));
        let format = TimeFormat::new(if self.settings.session.clock_24h {
            HourCycle::H23
        } else {
            HourCycle::H12
        })
        .with_day_period_labels(enzui_core::time_of_day::DayPeriodLabels::new(
            i18n::current().text("enzui-time-am"),
            i18n::current().text("enzui-time-pm"),
        ));
        let time = |minutes: u16| {
            TimeOfDay::hm(u32::from(minutes) / 60, u32::from(minutes) % 60)
                .unwrap_or(TimeOfDay::MIDNIGHT)
        };
        let (from, to) = (time(night.from), time(night.to));
        let drafts = &self.system.night_drafts;
        let from_text = drafts.0.as_deref().unwrap_or(&self.system.night_text.0);
        let to_text = drafts.1.as_deref().unwrap_or(&self.system.night_text.1);
        // Built inside `dependent` (below), so they are inert while night
        // light is off or always on; built ahead of it they stayed live.
        let times = || {
            self.record_liveness("display/night_light_times");
            self.mark_setting(
                "display/night_light_times",
                iced::widget::row![
                    iced::widget::container(
                        enzui_widgets::icon::enz_icon("clock")
                            .size(18.0)
                            .tint(if self.controls_live() {
                                self.tokens().palette.text
                            } else {
                                self.tokens().palette.disabled
                            })
                            .into_element()
                    )
                    .center_x(24.0)
                    .padding(iced::Padding::default().top(22.0)),
                    enzui_widgets::time_field::enz_time_field(from_text, from, |event| {
                        Parent::System(Message::NightTime(true, event))
                    })
                    .format(format.clone())
                    .label(tr("shell-turns-on"))
                    .id("night-light-from")
                    .enabled(self.controls_live())
                    .into_element(),
                    enzui_widgets::time_field::enz_time_field(to_text, to, |event| Parent::System(
                        Message::NightTime(false, event)
                    ))
                    .format(format)
                    .label(tr("shell-turns-off"))
                    .id("night-light-to")
                    .enabled(self.controls_live())
                    .into_element(),
                ]
                .spacing(10)
                .padding([8, 10]),
            )
        };
        let error: Element<'_, Parent> = match &self.system.night_error {
            Some(error) => enzui_widgets::text::enz_text(error.clone())
                .size(enzui_widgets::text::TextSize::Footnote)
                .into_element(),
            None => Space::new().into(),
        };
        let schedule = || {
            self.record_liveness("display/night_light_schedule");
            self.choice_setting(
                "display/night_light_schedule",
                if night.scheduled {
                    tr("shell-scheduled")
                } else {
                    tr("shell-always-on")
                },
                vec![
                    (tr("shell-scheduled").into(), change(Night::Scheduled(true))),
                    (
                        tr("shell-always-on").into(),
                        change(Night::Scheduled(false)),
                    ),
                ],
            )
        };
        column![
            self.switch_setting("display/night_light", night.enabled, move |on| change(
                Night::Enabled(on)
            )),
            self.dependent(night.enabled, || self.number_setting(
                "display/night_light_temperature",
                night.temperature.into(),
                WARMEST.into(),
                COOLEST.into(),
                " K",
                move |kelvin| change(Night::Temperature(kelvin as u16))
            )),
            self.dependent(night.enabled, schedule),
            self.dependent(night.enabled && night.scheduled, times),
            error,
        ]
        .spacing(2)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32) -> Mode {
        Mode {
            width,
            height,
            refresh_mhz: 60_000,
            preferred: true,
        }
    }

    #[test]
    fn fractional_scales_need_only_a_usable_desktop() {
        // 2256x1504 has no whole logical size at 125%, 150% or 175%.
        let surface = mode(2256, 1504);
        let offered: Vec<f64> = SCALES
            .into_iter()
            .filter(|&scale| supports_scale(surface, scale))
            .collect();
        assert_eq!(offered, [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]);
        assert!(!supports_scale(mode(1366, 768), 1.25));
        assert!(supports_scale(mode(3840, 2160), 2.0));
    }
}
