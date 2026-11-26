use enzui_shell::i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum QuickAction {
    Network,
    Bluetooth,
    Appearance,
    Tiling,
    Volume,
    Brightness,
    Dnd,
    Screenshot,
    Lock,
    Power,
    NightLight,
    PowerMode,
    Keyboard,
    Drives,
    GameMode,
    Dictation,
    Phone,
}
impl QuickAction {
    pub const ALL: [Self; 17] = [
        Self::Network,
        Self::Bluetooth,
        Self::Phone,
        Self::Appearance,
        Self::Tiling,
        Self::Volume,
        Self::Brightness,
        Self::Dnd,
        Self::Screenshot,
        Self::Lock,
        Self::Power,
        Self::NightLight,
        Self::PowerMode,
        Self::Keyboard,
        Self::Drives,
        Self::GameMode,
        Self::Dictation,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Bluetooth => "bluetooth",
            Self::Appearance => "appearance",
            Self::Tiling => "tiling",
            Self::Volume => "volume",
            Self::Brightness => "brightness",
            Self::Dnd => "dnd",
            Self::Screenshot => "screenshot",
            Self::Lock => "lock",
            Self::Power => "power",
            Self::NightLight => "night_light",
            Self::PowerMode => "power_mode",
            Self::Keyboard => "keyboard",
            Self::Drives => "drives",
            Self::GameMode => "game_mode",
            Self::Dictation => "dictation",
            Self::Phone => "phone",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Network => tr("shell-network"),
            Self::Bluetooth => tr("shell-bluetooth"),
            Self::Appearance => tr("shell-dark-mode"),
            Self::Tiling => tr("shell-tiling"),
            Self::Volume => tr("shell-volume-and-mute"),
            Self::Brightness => tr("shell-brightness"),
            Self::Dnd => tr("shell-do-not-disturb"),
            Self::Screenshot => tr("shell-screenshot"),
            Self::Lock => tr("shell-lock"),
            Self::Power => tr("shell-power-menu"),
            Self::NightLight => tr("shell-night-light"),
            Self::PowerMode => tr("shell-power-mode"),
            Self::Keyboard => tr("shell-keyboard"),
            Self::Drives => tr("shell-drives"),
            Self::GameMode => tr("shell-game-mode"),
            Self::Dictation => tr("shell-dictation"),
            Self::Phone => tr("shell-phone-link"),
        }
    }
    pub(super) fn bit(self) -> u32 {
        1 << (self as u32)
    }
}
