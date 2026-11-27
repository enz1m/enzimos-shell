use super::*;
mod audio_test;
mod microphone;
use enzui_shell::audio_devices::Kind;
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::{Level, Sound};
use enzui_widgets::{slider::enz_slider, toolbar_icon::enz_toolbar_icon};
use iced::{
    Alignment,
    widget::{column, container, row, sensor::Sensor},
};
pub(super) use microphone::{Event as MicrophoneEvent, Microphone};

impl SettingsApp {
    fn device_choice<'a>(
        &self,
        route: &'static str,
        kind: Kind,
        sound: Option<&Sound>,
    ) -> Element<'a, Parent> {
        let Some(sound) = sound else {
            return self.mark_setting(route, self.loading_row());
        };
        let Some(devices) = sound.devices.as_ref() else {
            return self.mark_setting(
                route,
                self.info_row(
                    "triangle-alert",
                    tr("shell-sound-devices-cannot-be-listed"),
                    "",
                ),
            );
        };
        let nodes: Vec<_> = devices.nodes.iter().filter(|d| d.kind == kind).collect();
        if nodes.is_empty() {
            let (icon, title) = match kind {
                Kind::Output => ("volume-x", tr("shell-no-output-devices")),
                Kind::Input => ("mic-off", tr("shell-no-input-devices")),
            };
            return self.mark_setting(
                route,
                self.info_row(
                    icon,
                    title,
                    tr("shell-connect-speakers-headphones-or-a-microphone"),
                ),
            );
        }
        let current = nodes
            .iter()
            .find(|d| devices.selected(d))
            .map_or_else(|| tr("shell-choose-2").to_owned(), |d| d.label.clone());
        let choices = nodes
            .iter()
            .map(|device| {
                (
                    device.label.clone(),
                    Self::request(Request::AudioDevice((*device).clone())),
                )
            })
            .collect();
        self.choice_setting(route, current, choices)
    }
    fn level_row<'a>(
        &self,
        route: &'static str,
        level: Option<Level>,
        icons: (&'static str, &'static str),
        on_volume: fn(u8) -> Request,
        on_mute: fn(bool) -> Request,
    ) -> Element<'a, Parent> {
        let entry = enzui_shell::settings_catalog::find(route).expect("Registered level setting");
        let Some(level) = level else {
            return self.mark_setting(
                route,
                crate::settings_app::settings_row(entry.title())
                    .icon(icons.0)
                    .subtitle(tr("shell-unavailable"))
                    .enabled(false)
                    .into_element(),
            );
        };
        let live = self.controls_live();
        let palette = self.tokens().palette;
        let value = if level.muted {
            tr("shell-muted").to_owned()
        } else {
            format!("{}%", level.percent)
        };
        let mute = enz_toolbar_icon(
            if level.muted { icons.1 } else { icons.0 },
            Self::request(on_mute(!level.muted)),
        )
        .active(level.muted)
        .aria_label(if level.muted {
            tr("shell-unmute")
        } else {
            tr("shell-mute")
        })
        .tooltip(if level.muted {
            tr("shell-unmute")
        } else {
            tr("shell-mute")
        })
        .id(format!("control-{route}-mute").into())
        .enabled(live)
        .into_element();
        let slider = enz_slider(0.0..=100.0, f32::from(level.percent), move |v| {
            Self::request(on_volume(v.round() as u8))
        })
        .step(1.0)
        .show_value(false)
        // The row names it, like every other setting.
        .aria_label(tr("shell-volume"))
        .id(format!("control-{route}").into())
        .enabled(live)
        .into_element();
        // One line on the shared row: level, its value, and mute at the end.
        self.mark_setting(
            route,
            crate::settings_app::settings_row(entry.title())
                .icon(if level.muted { icons.1 } else { icons.0 })
                .control(
                    row![
                        container(slider).width(240.0),
                        container(
                            enz_text(value)
                                .size_px(self.text_px(12.0))
                                .muted_on(palette.surface)
                                .into_element()
                        )
                        .width(44)
                        .align_x(Alignment::End),
                        mute,
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .into_element(),
        )
    }
    pub(super) fn sound_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let read = match self.reading(Panel::Sound) {
            Some(Reading::Sound(sound)) => Some(sound),
            _ => None,
        };
        let sound = read.clone().unwrap_or_default();
        // Choosing a device needs PipeWire's `wpctl` or PulseAudio's `pactl`.
        let devices = self.offers(enzui_shell::platform::Feature::AudioDevices);
        let choice = |route, kind| devices.then(|| self.device_choice(route, kind, read.as_ref()));
        let mut output = column![].spacing(2);
        if let Some(choice) = choice("sound/output", Kind::Output) {
            output = output.push(choice);
        }
        let mut output = output.push(self.level_row(
            "sound/output_volume",
            sound.output,
            ("volume-2", "volume-x"),
            Request::OutputVolume,
            Request::OutputMute,
        ));
        if let Some(error) = &sound.error {
            output = output.push(self.caption(error.clone()));
        }
        let mut input = column![].spacing(2);
        if let Some(choice) = choice("sound/input", Kind::Input) {
            input = input.push(choice);
        }
        let input = input
            .push(self.level_row(
                "sound/input_volume",
                sound.input,
                ("mic", "mic-off"),
                Request::InputVolume,
                Request::InputMute,
            ))
            .push(self.microphone_test_view(&sound));
        body.push(
            self.section_card_for("sound/output", output)
                .title(tr("shell-output"))
                .description(tr("shell-where-sound-plays-and-how-loud-it-is"))
                .into_element(),
        )
        .push(
            Sensor::new(
                self.section_card_for("sound/input", input)
                    .title(tr("shell-input"))
                    .description(tr("shell-the-microphone-applications-record-from"))
                    .into_element(),
            )
            .on_show(|_| Parent::System(Message::Microphone(MicrophoneEvent::Visible(true))))
            .on_hide(Parent::System(Message::Microphone(
                MicrophoneEvent::Visible(false),
            ))),
        )
    }
}
