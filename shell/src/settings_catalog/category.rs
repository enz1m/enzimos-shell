use super::{Entry, category, find};
use enzui_shell::i18n::tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Appearance,
    Shell,
    Windows,
    Search,
    Notifications,
    Capture,
    Network,
    Bluetooth,
    Phone,
    Remote,
    Display,
    Sound,
    Power,
    Input,
    Keyboard,
    Controllers,
    Accessibility,
    Printers,
    Thunderbolt,
    Disks,
    DateTime,
    Updates,
    Backups,
    Developer,
    Agents,
    Apps,
    Users,
    About,
}
impl Category {
    /// Sidebar order; [`Group`] splits it into captioned runs.
    pub const ALL: [Self; 28] = [
        Self::Appearance,
        Self::Shell,
        Self::Windows,
        Self::Search,
        Self::Notifications,
        Self::Capture,
        Self::Network,
        Self::Bluetooth,
        Self::Phone,
        Self::Remote,
        Self::Display,
        Self::Sound,
        Self::Power,
        Self::Input,
        Self::Keyboard,
        Self::Controllers,
        Self::Accessibility,
        Self::Printers,
        Self::Thunderbolt,
        Self::Disks,
        Self::Apps,
        Self::Agents,
        Self::Updates,
        Self::Backups,
        Self::Users,
        Self::About,
        Self::DateTime,
        Self::Developer,
    ];
    /// The sidebar folder holding this page, if it is not top-level.
    /// Folder members are contiguous in [`Self::ALL`].
    pub fn folder(self) -> Option<Folder> {
        match self {
            Self::Input | Self::Keyboard | Self::Controllers => Some(Folder::InputDevices),
            Self::About | Self::DateTime | Self::Developer | Self::Users => Some(Folder::System),
            _ => None,
        }
    }
    pub fn group(self) -> Group {
        match self {
            Self::Appearance
            | Self::Shell
            | Self::Windows
            | Self::Search
            | Self::Notifications
            | Self::Capture => Group::Desktop,
            Self::Network | Self::Bluetooth | Self::Phone | Self::Remote => Group::Connections,
            Self::Display
            | Self::Sound
            | Self::Power
            | Self::Input
            | Self::Keyboard
            | Self::Controllers
            | Self::Accessibility
            | Self::Printers
            | Self::Thunderbolt
            | Self::Disks => Group::Devices,
            Self::Users
            | Self::Updates
            | Self::Backups
            | Self::DateTime
            | Self::Developer
            | Self::Apps
            | Self::Agents
            | Self::About => Group::System,
        }
    }
    pub fn title(self) -> &'static str {
        tr(self.title_key())
    }
    pub(super) fn title_key(self) -> &'static str {
        match self {
            Self::Appearance => "shell-appearance",
            Self::Shell => "shell-shell",
            Self::Windows => "shell-windows",
            Self::Search => "shell-search",
            Self::Notifications => "shell-notifications",
            Self::Capture => "shell-screenshots",
            Self::Network => "shell-network",
            Self::Bluetooth => "shell-bluetooth",
            Self::Phone => "shell-phone-link",
            Self::Remote => "shell-remote-access",
            Self::Display => "shell-display",
            Self::Sound => "shell-sound",
            Self::Power => "shell-power",
            Self::Input => "shell-mouse-touchpad",
            Self::Keyboard => "shell-keyboard",
            Self::Controllers => "shell-controllers",
            Self::Accessibility => "shell-accessibility",
            Self::Printers => "shell-printers",
            Self::Thunderbolt => "shell-thunderbolt",
            Self::Disks => "shell-disks",
            Self::DateTime => "shell-date-time",
            Self::Developer => "shell-developer",
            Self::Updates => "shell-updates",
            Self::Backups => "shell-history-backups",
            Self::Agents => "shell-agents",
            Self::Users => "shell-users",
            Self::Apps => "shell-apps",
            Self::About => "shell-about",
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            Self::Appearance => tr("shell-colors-background-and-the-size-of-shell-text-and-icons"),
            Self::Shell => tr("shell-the-bar-sidebar-control-center-and-plugins"),
            Self::Windows => tr("shell-how-application-windows-are-arranged-drawn-and-focused"),
            Self::Search => tr("shell-quick-search-behavior-sources-and-what-it-remembers"),
            Self::Notifications => tr("shell-delivery-toast-appearance-and-notification-history"),
            Self::Capture => tr("shell-screenshot-formats-the-clipboard-and-screen-recording"),
            Self::Network => tr("shell-wi-fi-wired-connections-and-vpns"),
            Self::Bluetooth => tr("shell-the-bluetooth-adapter-and-paired-devices"),
            Self::Phone => tr("shell-phone-link-description"),
            Self::Remote => {
                tr("shell-sign-in-from-another-computer-with-ssh-or-share-this-desktop")
            }
            Self::Display => tr("shell-resolution-refresh-rate-scale-and-arrangement"),
            Self::Sound => tr("shell-output-and-input-devices-and-their-volume"),
            Self::Power => tr("shell-battery-power-mode-and-when-the-screen-sleeps"),
            Self::Input => tr("shell-pointer-speed-buttons-scrolling-and-touchpad-gestures"),
            Self::Keyboard => tr("shell-layouts-key-repeat-and-special-keys"),
            Self::Controllers => tr("shell-game-controllers-their-batteries-and-a-button-test"),
            Self::Accessibility => tr("shell-zoom-pointer-size-motion-and-the-screen-reader"),
            Self::Printers => tr("shell-printers-and-the-default-destination"),
            Self::Thunderbolt => tr("shell-thunderbolt-summary"),
            Self::Disks => tr("shell-disks-summary"),
            Self::DateTime => tr("shell-time-zone-and-network-time"),
            Self::Developer => tr("shell-optional-developer-tools-and-how-project-folders-open"),
            Self::Updates => tr("shell-install-new-versions-of-enzimos-and-see-the-installed-ones"),
            Self::Backups => tr("shell-snapshots-of-your-home-folder-and-encrypted-backups"),
            Self::Agents => tr("shell-what-ai-agents-may-do-in-your-apps-and-what-they-did"),
            Self::Users => tr("shell-profile-pictures-local-accounts-and-administrator-access"),
            Self::Apps => tr("shell-which-apps-open-your-links-and-files"),
            Self::About => tr("shell-this-computer-s-name-system-and-hardware"),
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Appearance => "palette",
            Self::Shell => "panels-top-left",
            Self::Windows => "app-window",
            Self::Search => "search",
            Self::Notifications => "bell",
            Self::Capture => "camera",
            Self::Network => "network",
            Self::Bluetooth => "bluetooth",
            Self::Phone => "smartphone",
            Self::Remote => "screen-share",
            Self::Display => "monitor",
            Self::Sound => "volume-2",
            Self::Power => "battery-medium",
            Self::Input => "mouse-pointer",
            Self::Keyboard => "keyboard",
            Self::Controllers => "gamepad-2",
            Self::Accessibility => "accessibility",
            Self::Printers => "printer",
            Self::Thunderbolt => "zap",
            Self::Disks => "hard-drive",
            Self::DateTime => "clock",
            Self::Developer => "code",
            Self::Updates => "download",
            Self::Backups => "history",
            Self::Agents => "bot",
            Self::Users => "users",
            Self::Apps => "layout-grid",
            Self::About => "info",
        }
    }
    pub fn page(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Shell => "shell",
            Self::Windows => "windows",
            Self::Search => "quick-search",
            Self::Notifications => "notifications",
            Self::Capture => "capture",
            Self::Network => "network",
            Self::Bluetooth => "bluetooth",
            Self::Phone => "phone",
            Self::Remote => "remote",
            Self::Display => "display",
            Self::Sound => "sound",
            Self::Power => "power",
            Self::Input => "input",
            Self::Keyboard => "keyboard",
            Self::Controllers => "controllers",
            Self::Accessibility => "accessibility",
            Self::Printers => "printers",
            Self::Thunderbolt => "thunderbolt",
            Self::Disks => "disks",
            Self::DateTime => "datetime",
            Self::Developer => "developer",
            Self::Updates => "updates",
            Self::Backups => "backups",
            Self::Agents => "agents",
            Self::Users => "users",
            Self::Apps => "apps",
            Self::About => "about",
        }
    }
    /// Resolves category or setting routes; gated categories fall back to the initial setup
    /// page.
    pub fn requested(value: Option<&str>, preview: bool) -> Self {
        let resolved = value.and_then(|value| {
            find(value)
                .map(Entry::category)
                .or_else(|| category(value.split('/').next().unwrap_or(value)))
        });
        match resolved {
            Some(Self::Notifications) if !preview => Self::Appearance,
            Some(category) => category,
            None => Self::Appearance,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Desktop,
    Connections,
    Devices,
    System,
}
impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::Desktop => tr("shell-desktop"),
            Self::Connections => tr("shell-connections"),
            Self::Devices => tr("shell-devices"),
            Self::System => tr("shell-apps-system"),
        }
    }
}

/// A sidebar entry that groups related pages under one expander.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Folder {
    InputDevices,
    System,
}
impl Folder {
    pub fn title(self) -> &'static str {
        match self {
            Self::InputDevices => tr("shell-input-devices"),
            Self::System => tr("shell-system"),
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::InputDevices => "keyboard",
            Self::System => "info",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::InputDevices => "input-devices",
            Self::System => "system",
        }
    }
    /// Its pages, in sidebar order.
    pub fn pages(self) -> impl Iterator<Item = Category> {
        Category::ALL
            .into_iter()
            .filter(move |category| category.folder() == Some(self))
    }
}
