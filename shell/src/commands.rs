//! Compositor binding commands received as enzui-command events.
//! Any process with this user's hyprctl access can emit them.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MicrophoneMute,
    BrightnessUp,
    BrightnessDown,
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    MediaStop,
    SwitchNext,
    SwitchPrevious,
    SwitchAppNext,
    SwitchCommit,
    SwitchCancel,
    SwitchState,
    EmojiPicker,
    WritingTools,
    Overview,
    WindowSpread,
    Shortcuts,
    ShowShortcuts,
    Search,
    Launcher,
    Controls,
    DesktopLayout,
    Dock,
    Notifications,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Record,
    Keyboard,
    Capture,
    ScreenshotScreen,
    ScreenshotWindow,
    Lock,
    /// A left press anywhere (a non-consuming compositor bind); the click
    /// itself still reaches whatever is under the pointer.
    PointerPress,
    /// Settings is about to change a display's scale: the shell hides its
    /// chrome first and answers once it is hidden.
    OutputRescaling,
    /// Settings has applied that change, or failed to.
    OutputRescaled,
    /// Super was pressed and released alone on a floating desktop.
    Activities,
    /// Super was pressed and released alone on a tiling desktop, where the
    /// overview has no window spread.
    ActivitiesSearch,
}

impl Command {
    const ALL: [(Self, &'static str); 42] = [
        (Self::VolumeUp, "volume-up"),
        (Self::VolumeDown, "volume-down"),
        (Self::VolumeMute, "volume-mute"),
        (Self::MicrophoneMute, "microphone-mute"),
        (Self::BrightnessUp, "brightness-up"),
        (Self::BrightnessDown, "brightness-down"),
        (Self::MediaPlayPause, "media-play-pause"),
        (Self::MediaNext, "media-next"),
        (Self::MediaPrevious, "media-previous"),
        (Self::MediaStop, "media-stop"),
        (Self::SwitchNext, "switch-next"),
        (Self::SwitchPrevious, "switch-previous"),
        (Self::SwitchAppNext, "switch-app-next"),
        (Self::SwitchCommit, "switch-commit"),
        (Self::SwitchCancel, "switch-cancel"),
        (Self::SwitchState, "switch-state"),
        (Self::EmojiPicker, "emoji-picker"),
        (Self::WritingTools, "writing-tools"),
        (Self::Overview, "overview"),
        (Self::WindowSpread, "window-spread"),
        (Self::Shortcuts, "shortcuts"),
        (Self::ShowShortcuts, "show-shortcuts"),
        (Self::Search, "search"),
        (Self::Launcher, "launcher"),
        (Self::Controls, "controls"),
        (Self::DesktopLayout, "desktop-layout"),
        (Self::Dock, "dock"),
        (Self::Notifications, "notifications"),
        (Self::ZoomIn, "zoom-in"),
        (Self::ZoomOut, "zoom-out"),
        (Self::ZoomReset, "zoom-reset"),
        (Self::Record, "record"),
        (Self::Keyboard, "keyboard"),
        (Self::Capture, "capture"),
        (Self::ScreenshotScreen, "screenshot-screen"),
        (Self::ScreenshotWindow, "screenshot-window"),
        (Self::Lock, "lock"),
        (Self::PointerPress, "pointer-press"),
        (Self::OutputRescaling, "output-rescaling"),
        (Self::OutputRescaled, "output-rescaled"),
        (Self::Activities, "activities"),
        (Self::ActivitiesSearch, "activities-search"),
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(command, _)| *command == self)
            .map_or("", |(_, name)| name)
    }

    /// The command an event line names: `custom>>enzui-command:NAME`.
    #[must_use]
    pub fn parse(line: &[u8]) -> Option<Self> {
        let line = std::str::from_utf8(line).ok()?.trim_end();
        let name = line.strip_prefix("custom>>enzui-command:")?;
        Self::ALL
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(command, _)| *command)
    }
}

/// Commands from the compositor's event socket, reconnecting after a loss.
pub fn subscription(socket: Option<std::path::PathBuf>) -> iced::Subscription<Command> {
    iced::Subscription::run_with(socket, stream)
}

#[allow(
    clippy::ref_option,
    reason = "Subscription::run_with borrows its hashed Option<PathBuf> data as a whole."
)]
fn stream(
    socket: &Option<std::path::PathBuf>,
) -> impl iced::futures::Stream<Item = Command> + use<> {
    use iced::futures::SinkExt;
    use std::os::unix::fs::MetadataExt;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let socket = socket.clone();
    iced::stream::channel(8, async move |mut output| {
        let Some(socket) = socket else {
            iced::futures::future::pending::<()>().await;
            return;
        };
        let uid = std::fs::metadata("/proc/self").ok().map(|m| m.uid());
        loop {
            if let Ok(Ok(stream)) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tokio::net::UnixStream::connect(&socket),
            )
            .await
                && uid.is_some()
                && stream.peer_cred().ok().map(|c| c.uid()) == uid
            {
                let mut reader = tokio::io::BufReader::new(stream);
                loop {
                    let mut line = Vec::new();
                    match (&mut reader).take(65537).read_until(b'\n', &mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.len() > 65536 => break,
                        Ok(_) => {
                            if let Some(command) = Command::parse(&line)
                                && output.send(command).await.is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    })
}
