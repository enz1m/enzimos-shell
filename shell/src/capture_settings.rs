use super::{Message, SettingsApp};
use enzui_shell::capture::{Audio, Change as Capture, Codec, Container, ImageFormat, Quality};
use enzui_shell::i18n::tr;
use enzui_shell::settings::Change;
use iced::widget::{Column, column};

fn change(change: Capture) -> Message {
    Message::Change(Change::Capture(change))
}

impl SettingsApp {
    pub(super) fn capture_page<'a>(&'a self, body: Column<'a, Message>) -> Column<'a, Message> {
        let capture = &self.settings.capture;
        let format = capture.image_format;
        let mut screenshots = column![
            self.choice_setting(
                "capture/format",
                format.label(),
                ImageFormat::ALL
                    .iter()
                    .map(|format| (
                        format.label().to_owned(),
                        change(Capture::ImageFormat(*format))
                    ))
                    .collect()
            )
        ]
        .spacing(2);
        // Only the chosen format's own setting (lossless WebP has none), or
        // another's while a search reveals it, inert.
        let shown =
            |route: &str, own: ImageFormat| format == own || self.highlighted == Some(route);
        if shown("capture/jpeg_quality", ImageFormat::Jpeg) {
            screenshots = screenshots.push(self.dependent(format == ImageFormat::Jpeg, || {
                self.number_setting(
                    "capture/jpeg_quality",
                    capture.jpeg_quality.into(),
                    1,
                    100,
                    "",
                    |value| change(Capture::JpegQuality(value as u8)),
                )
            }));
        }
        if shown("capture/png_compression", ImageFormat::Png) {
            // Named trade-offs instead of a bare 0-9 level; a level stored
            // by an older version still shows as its own choice.
            let mut levels = vec![1u8, 6, 9];
            if !levels.contains(&capture.png_compression) {
                levels.push(capture.png_compression);
                levels.sort_unstable();
            }
            screenshots = screenshots.push(self.dependent(format == ImageFormat::Png, || {
                self.choice_setting(
                    "capture/png_compression",
                    png_level_label(capture.png_compression),
                    levels
                        .into_iter()
                        .map(|level| {
                            (
                                png_level_label(level),
                                change(Capture::PngCompression(level)),
                            )
                        })
                        .collect(),
                )
            }));
        }
        let screenshots = screenshots
            // A screenshot must go somewhere, so the last way cannot be turned off.
            .push(self.dependent(capture.save_screenshots, || {
                self.switch_setting("capture/copy", capture.copy_to_clipboard, |on| {
                    change(Capture::CopyToClipboard(on))
                })
            }))
            .push(self.dependent(capture.copy_to_clipboard, || {
                self.switch_setting("capture/save", capture.save_screenshots, |on| {
                    change(Capture::SaveScreenshots(on))
                })
            }))
            .push(
                self.switch_setting("capture/pointer", capture.show_pointer, |on| {
                    change(Capture::ShowPointer(on))
                }),
            );
        let codecs = capture.container.codecs();
        let recording = column![
            self.choice_setting(
                "capture/container",
                capture.container.label(),
                Container::ALL
                    .iter()
                    .map(|container| (
                        container.label().to_owned(),
                        change(Capture::Container(*container))
                    ))
                    .collect()
            ),
            self.choice_setting(
                "capture/codec",
                capture.codec.label(),
                Codec::ALL
                    .iter()
                    .filter(|codec| codecs.contains(codec))
                    .map(|codec| (codec.label().to_owned(), change(Capture::Codec(*codec))))
                    .collect()
            ),
            self.choice_setting(
                "capture/framerate",
                enzui_shell::tr!("shell-frames-a-second", "arg0" => enzui_shell::i18n::argument(&(capture.framerate))),
                [24u8, 30, 60]
                    .into_iter()
                    .map(|rate| (
                        enzui_shell::tr!("shell-frames-a-second", "arg0" => enzui_shell::i18n::argument(&(rate))),
                        change(Capture::Framerate(rate))
                    ))
                    .collect()
            ),
            self.choice_setting(
                "capture/quality",
                capture.quality.label(),
                Quality::ALL
                    .iter()
                    .map(|quality| (
                        quality.label().to_owned(),
                        change(Capture::Quality(*quality))
                    ))
                    .collect()
            ),
            self.choice_setting(
                "capture/audio",
                capture.audio.label(),
                Audio::ALL
                    .iter()
                    .map(|audio| (audio.label().to_owned(), change(Capture::Audio(*audio))))
                    .collect()
            ),
        ]
        .spacing(2);
        body.push(
            self.section_card_for("capture/format", screenshots)
                .title(tr("shell-images"))
                .description(tr("shell-press-print-to-choose-an-area-a-window-or-the-whole-screen-saved-in-pictures"))
                .into_element(),
        )
        .push(
            self.section_card_for("capture/container", recording)
                .title(tr("shell-screen-recording"))
                .description(tr("shell-start-from-the-print-menu-or-press-ctrl-shift-alt-r-to-record-the-whole-scre"))
                .into_element(),
        )
    }
}

fn png_level_label(level: u8) -> String {
    match level {
        0..=3 => tr("shell-faster-saving").to_owned(),
        4..=7 => tr("shell-balanced").to_owned(),
        _ => tr("shell-smaller-files").to_owned(),
    }
}
