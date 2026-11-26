use super::{Message, Shell};
use crate::shell_osd::Level;
use enzui_shell::commands::Command;
use enzui_shell::i18n::tr;
use enzui_shell::settings::Change as SettingChange;
use enzui_shell::system_controls::Action;
use iced::Task;

const STEP: u8 = 5;

impl Shell {
    pub(crate) fn command_subscription(&self) -> iced::Subscription<Message> {
        self.compositor.commands().map(Message::Command)
    }

    pub(crate) fn run_command(&mut self, command: Command) -> Task<Message> {
        match command {
            Command::VolumeUp | Command::VolumeDown => {
                let Some(audio) = self.system.audio else {
                    return Task::none();
                };
                let current = self.volume_draft.unwrap_or(audio.percent);
                let next = if command == Command::VolumeUp {
                    current.saturating_add(STEP).min(100)
                } else {
                    current.saturating_sub(STEP)
                };
                self.volume_draft = Some(next);
                // Raising the volume unmutes, as on other desktops.
                let unmute = if audio.muted && command == Command::VolumeUp {
                    if let Some(audio) = self.system.audio.as_mut() {
                        audio.muted = false;
                    }
                    self.enqueue_system(Action::Mute(false))
                } else {
                    Task::none()
                };
                let muted = self.system.audio.as_ref().is_some_and(|a| a.muted);
                Task::batch([
                    unmute,
                    self.enqueue_system(Action::Volume(next)),
                    self.show_osd(Level::Volume, next, muted),
                ])
            }
            Command::VolumeMute => {
                let Some(audio) = self.system.audio.as_mut() else {
                    return Task::none();
                };
                audio.muted = !audio.muted;
                let (muted, percent) = (audio.muted, self.volume_draft.unwrap_or(audio.percent));
                Task::batch([
                    self.enqueue_system(Action::Mute(muted)),
                    self.show_osd(Level::Volume, percent, muted),
                ])
            }
            Command::MicrophoneMute => {
                let Some(microphone) = self.system.microphone.as_mut() else {
                    return Task::none();
                };
                microphone.muted = !microphone.muted;
                let (muted, percent) = (microphone.muted, microphone.percent);
                Task::batch([
                    self.enqueue_system(Action::MicrophoneMute(muted)),
                    self.show_osd(Level::Microphone, percent, muted),
                ])
            }
            Command::BrightnessUp | Command::BrightnessDown => {
                let Some(brightness) = self.system.brightness.clone() else {
                    return Task::none();
                };
                let current = self.brightness_draft.unwrap_or(brightness.percent);
                let next = if command == Command::BrightnessUp {
                    current.saturating_add(STEP).min(100)
                } else {
                    current.saturating_sub(STEP).max(5)
                };
                self.brightness_draft = Some(next);
                Task::batch([
                    self.enqueue_system(Action::Brightness(next)),
                    self.show_osd(Level::Brightness, next, false),
                ])
            }
            _ => self.run_later_command(command),
        }
    }

    // Commands whose features live in their own modules.
    fn run_later_command(&mut self, command: Command) -> Task<Message> {
        match command {
            Command::MediaPlayPause
            | Command::MediaNext
            | Command::MediaPrevious
            | Command::MediaStop => self.media_command(command),
            Command::SwitchNext
            | Command::SwitchPrevious
            | Command::SwitchAppNext
            | Command::SwitchCommit
            | Command::SwitchCancel
            | Command::SwitchState => self.switcher_command(command),
            Command::WritingTools => self.open_writing_tools(),
            Command::EmojiPicker => self.open_emoji_picker(),
            Command::Record => self.toggle_recording(),
            Command::Capture => self.open_capture(),
            Command::ScreenshotScreen => {
                self.capture_message(crate::shell_capture::CaptureMessage::Screen)
            }
            Command::ScreenshotWindow => {
                self.capture_message(crate::shell_capture::CaptureMessage::ActiveWindow)
            }
            Command::Shortcuts => self.shortcuts_toggle(),
            Command::ShowShortcuts => self.shortcuts_show(),
            Command::Launcher
                if self
                    .search
                    .preferences
                    .shows(enzui_shell::search::Category::Apps) =>
            {
                let message = if self.search.showing {
                    crate::quick_search::Message::Toggle
                } else {
                    crate::quick_search::Message::Show(Some(enzui_shell::search::Category::Apps))
                };
                self.update(Message::QuickSearch(message))
            }
            Command::Launcher => self.update(Message::ShowLauncher),
            Command::Search if self.search.preferences.enabled => {
                self.update(Message::QuickSearch(crate::quick_search::Message::Toggle))
            }
            Command::Search => self.update(Message::ShowLauncher),
            Command::Controls => self.update(Message::ToggleControls),
            Command::DesktopLayout => self.update(Message::Tiling(
                !self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.tiling),
            )),
            Command::Dock => self.dock_save(enzui_shell::dock::Change::Enabled(
                !self
                    .dock_owner()
                    .is_some_and(|owner| owner.preferences.enabled),
            )),
            Command::Notifications => {
                self.notification_message(crate::notification_panel::Message::Shortcut(
                    enzui_shell::shortcuts::Event::OpenNotifications,
                ))
            }
            Command::Overview => self.update(Message::ToggleOverview),
            Command::WindowSpread => {
                self.spread_message(crate::window_spread::SpreadMessage::Toggle)
            }
            Command::Activities => self.activities_command(true),
            Command::ActivitiesSearch => self.activities_command(false),
            Command::Keyboard => self.toggle_osk(),
            // The supervised lock, as the Control Center's: it holds logind's
            // lock hint and refuses a second lock client.
            Command::Lock => self.update(Message::SystemAction(Action::Lock)),
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => self.zoom_command(command),
            Command::PointerPress => self.search_pointer_press(),
            Command::OutputRescaling => self.begin_rescale_hold(),
            Command::OutputRescaled => self.rescale_applied(),
            _ => Task::none(),
        }
    }

    fn toggle_recording(&mut self) -> Task<Message> {
        if self.recording.is_some() {
            return self.stop_recording();
        }
        let compositor = self.compositor.clone();
        Task::perform(
            async move {
                let output = compositor.desktop_output().await?;
                output["name"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| tr("shell-there-is-no-display-to-record").to_owned())
            },
            Message::RecordOutput,
        )
    }

    // Starts recording on a worker, so a stuck sound server never holds
    // the shell.
    pub(crate) fn start_recording(
        &mut self,
        target: enzui_shell::recording::Target,
    ) -> Task<Message> {
        if self.recording.is_some() || self.recording_start.is_some() {
            return Task::none();
        }
        self.recording_start = Some(false);
        Task::perform(
            enzui_shell::recording::start_in_background(
                self.controls.clone(),
                target,
                self.desired_settings.capture.clone(),
            ),
            Message::RecordingStarted,
        )
    }

    pub(crate) fn recording_started(
        &mut self,
        started: Result<enzui_shell::recording::Started, String>,
    ) -> Task<Message> {
        let stop = self.recording_start.take().unwrap_or(true);
        match started.map(|started| started.take()) {
            Ok(Some(recording)) => {
                self.recording = Some(recording);
                if stop {
                    return self.stop_recording();
                }
            }
            Ok(None) => {}
            Err(error) => self.notice = Some(error),
        }
        Task::none()
    }

    pub(crate) fn stop_recording(&mut self) -> Task<Message> {
        if let Some(stopping) = self.recording_start.as_mut() {
            *stopping = true;
        }
        let Some(recording) = self.recording.take() else {
            return Task::none();
        };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || recording.stop())
                    .await
                    .unwrap_or_else(|_| {
                        enzui_shell::recording::Ending::Failed(
                            tr("shell-the-recording-could-not-be-saved").into(),
                        )
                    })
            },
            Message::RecordingSaved,
        )
    }

    // Hands the keyboard back to the app used last once a closed panel's
    // focus has settled (see `Compositor::return_focus_to_app`).
    pub(crate) fn return_app_focus(&self) -> Task<Message> {
        let compositor = self.compositor.clone();
        iced_runtime::window::wayland_parent_export(self.panel).then(move |payload| {
            let compositor = compositor.clone();
            Task::perform(
                async move {
                    if !enzui_shell::surface_input::synchronize(payload).await {
                        return Ok(());
                    }
                    compositor.return_focus_to_app().await
                },
                Message::FocusReturned,
            )
        })
    }

    pub(crate) fn recording_tick(&mut self) -> Task<Message> {
        let Some(recording) = self.recording.as_mut() else {
            return Task::none();
        };
        // The compositor ends screen capture when the session locks; finish
        // the file rather than leave the encoder waiting for frames.
        if enzui_shell::session_lock::locking() {
            self.notice = Some(tr("shell-recording-stopped-the-screen-locked").into());
            return self.stop_recording();
        }
        if !recording.running() {
            self.notice = Some(tr("shell-recording-stopped-unexpectedly").into());
            return self.stop_recording();
        }
        if recording.disk_low() {
            self.notice = Some(tr("shell-recording-stopped-the-disk-is-nearly-full").into());
            return self.stop_recording();
        }
        Task::none()
    }

    fn zoom_command(&mut self, command: Command) -> Task<Message> {
        use enzui_shell::desktop_preferences::{OptionKey, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};
        let current = i32::from(self.desired_settings.compositor.zoom);
        let next = match command {
            Command::ZoomIn => (current + ZOOM_STEP).min(ZOOM_MAX),
            Command::ZoomOut => (current - ZOOM_STEP).max(ZOOM_MIN),
            // Off when zoomed, else back to twice the size.
            _ if current > ZOOM_MIN => ZOOM_MIN,
            _ => 2 * ZOOM_MIN,
        };
        if next == current {
            return Task::none();
        }
        self.save_setting(SettingChange::Compositor(OptionKey::Zoom, next))
    }
}
