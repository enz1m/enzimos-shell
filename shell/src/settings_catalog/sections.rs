use super::Category;

#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub category: Category,
    pub label: &'static str,
    pub route: &'static str,
}
pub const SECTIONS: &[Section] = &[
    Section {
        category: Category::Phone,
        label: "shell-phone-devices",
        route: "phone/overview",
    },
    Section {
        category: Category::Phone,
        label: "shell-phone-sharing",
        route: "phone/sharing",
    },
    Section {
        category: Category::DateTime,
        label: "language-title",
        route: "datetime/language",
    },
    Section {
        category: Category::Users,
        label: "shell-accounts",
        route: "users/accounts",
    },
    Section {
        category: Category::Appearance,
        label: "shell-style-color",
        route: "appearance/theme",
    },
    Section {
        category: Category::Appearance,
        label: "shell-shape-and-spacing",
        route: "appearance/corners",
    },
    Section {
        category: Category::Appearance,
        label: "shell-app-styles",
        route: "appearance/app_style",
    },
    Section {
        category: Category::Appearance,
        label: "shell-wallpaper",
        route: "wallpaper/source",
    },
    Section {
        category: Category::Appearance,
        label: "shell-text-icons",
        route: "sizing/text_size",
    },
    Section {
        category: Category::Appearance,
        label: "shell-fonts",
        route: "fonts/ui",
    },
    Section {
        category: Category::Shell,
        label: "shell-shell-style",
        route: "appearance/shell_style",
    },
    Section {
        category: Category::Shell,
        label: "shell-frame-size",
        route: "sizing/frame_width",
    },
    Section {
        category: Category::Shell,
        label: "shell-islands-size",
        route: "appearance/islands_height",
    },
    Section {
        category: Category::Shell,
        label: "shell-placement",
        route: "shell/bar",
    },
    Section {
        category: Category::Shell,
        label: "shell-ai-assistant",
        route: "shell/assistant_enabled",
    },
    Section {
        category: Category::Shell,
        label: "shell-bar-contents",
        route: "bar-layout/items",
    },
    Section {
        category: Category::Shell,
        label: "shell-dock",
        route: "dock/enabled",
    },
    Section {
        category: Category::Shell,
        label: "shell-quick-controls",
        route: "controls/network",
    },
    Section {
        category: Category::Shell,
        label: "shell-plugins",
        route: "plugins/installed",
    },
    Section {
        category: Category::Windows,
        label: "shell-tiling-gaps",
        route: "windows/tiling",
    },
    Section {
        category: Category::Windows,
        label: "shell-appearance-motion",
        route: "windows/border_size",
    },
    Section {
        category: Category::Windows,
        label: "shell-behavior",
        route: "windows/focus_pointer",
    },
    Section {
        category: Category::Search,
        label: "shell-quick-search",
        route: "quick-search/enabled",
    },
    Section {
        category: Category::Search,
        label: "shell-sources",
        route: "quick-search/apps",
    },
    Section {
        category: Category::Search,
        label: "shell-filename-search",
        route: "quick-search/roots",
    },
    Section {
        category: Category::Search,
        label: "shell-ranking-privacy",
        route: "quick-search/recent",
    },
    Section {
        category: Category::Search,
        label: "shell-quick-keys-favorites",
        route: "quick-search/aliases",
    },
    Section {
        category: Category::Notifications,
        label: "shell-delivery",
        route: "notifications/dnd",
    },
    Section {
        category: Category::Notifications,
        label: "shell-toast-appearance",
        route: "notifications/position",
    },
    Section {
        category: Category::Notifications,
        label: "shell-history-privacy",
        route: "notifications/history",
    },
    Section {
        category: Category::Notifications,
        label: "shell-apps",
        route: "notifications/applications",
    },
    Section {
        category: Category::Capture,
        label: "shell-images",
        route: "capture/format",
    },
    Section {
        category: Category::Capture,
        label: "shell-screen-recording",
        route: "capture/container",
    },
    Section {
        category: Category::Network,
        label: "shell-internet",
        route: "network/internet",
    },
    Section {
        category: Category::Network,
        label: "shell-wi-fi",
        route: "network/wifi",
    },
    Section {
        category: Category::Network,
        label: "shell-wired",
        route: "network/wired",
    },
    Section {
        category: Category::Network,
        label: "shell-vpn",
        route: "network/vpn",
    },
    Section {
        category: Category::Network,
        label: "shell-firewall",
        route: "network/firewall",
    },
    Section {
        category: Category::Bluetooth,
        label: "shell-paired-devices",
        route: "bluetooth/devices",
    },
    Section {
        category: Category::Remote,
        label: "shell-remote-sign-in",
        route: "remote/ssh",
    },
    Section {
        category: Category::Remote,
        label: "shell-desktop-sharing",
        route: "remote/sharing",
    },
    Section {
        category: Category::Display,
        label: "shell-arrangement",
        route: "display/arrangement",
    },
    Section {
        category: Category::Display,
        label: "shell-display-settings",
        route: "display/resolution",
    },
    Section {
        category: Category::Display,
        label: "shell-legacy-apps",
        route: "display/x11_sharp",
    },
    Section {
        category: Category::Display,
        label: "shell-night-light",
        route: "display/night_light",
    },
    Section {
        category: Category::Sound,
        label: "shell-output",
        route: "sound/output",
    },
    Section {
        category: Category::Sound,
        label: "shell-input",
        route: "sound/input",
    },
    Section {
        category: Category::Power,
        label: "shell-battery",
        route: "power/battery",
    },
    Section {
        category: Category::Power,
        label: "shell-power-mode",
        route: "power/mode",
    },
    Section {
        category: Category::Power,
        label: "shell-game-mode",
        route: "power/game_mode",
    },
    Section {
        category: Category::Power,
        label: "shell-screen-sleep",
        route: "power/blank",
    },
    Section {
        category: Category::Power,
        label: "shell-memory-protection",
        route: "power/memory",
    },
    Section {
        category: Category::Power,
        label: "shell-tmp-storage-heading",
        route: "power/temporary_storage",
    },
    Section {
        category: Category::Input,
        label: "shell-mouse",
        route: "windows/pointer_speed",
    },
    Section {
        category: Category::Input,
        label: "shell-scrolling",
        route: "windows/natural_scroll",
    },
    Section {
        category: Category::Input,
        label: "shell-touchpad",
        route: "windows/tap_to_click",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-layouts",
        route: "keyboard/layouts",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-typing",
        route: "windows/repeat_delay",
    },
    Section {
        category: Category::Keyboard,
        label: "shell-shortcuts",
        route: "keyboard/shortcuts",
    },
    Section {
        category: Category::Accessibility,
        label: "shell-seeing",
        route: "windows/zoom",
    },
    Section {
        category: Category::Accessibility,
        label: "shell-bar-buttons",
        route: "accessibility/keyboard",
    },
    Section {
        category: Category::Controllers,
        label: "shell-connected-controllers",
        route: "controllers/list",
    },
    Section {
        category: Category::Printers,
        label: "shell-your-printers",
        route: "printers/list",
    },
    Section {
        category: Category::Printers,
        label: "shell-print-jobs",
        route: "printers/jobs",
    },
    Section {
        category: Category::Thunderbolt,
        label: "shell-thunderbolt-devices",
        route: "thunderbolt/devices",
    },
    Section {
        category: Category::Disks,
        label: "shell-disks",
        route: "disks/list",
    },
    Section {
        category: Category::DateTime,
        label: "shell-time",
        route: "datetime/timezone",
    },
    Section {
        category: Category::Appearance,
        label: "shell-login-screen",
        route: "datetime/lock_time",
    },
    Section {
        category: Category::Developer,
        label: "shell-developer-tools",
        route: "developer/tools",
    },
    Section {
        category: Category::Developer,
        label: "shell-projects",
        route: "developer/projects",
    },
    Section {
        category: Category::Updates,
        label: "shell-enzimos-section",
        route: "updates/install",
    },
    Section {
        category: Category::Updates,
        label: "shell-versions",
        route: "updates/generations",
    },
    Section {
        category: Category::Updates,
        label: "shell-device-firmware",
        route: "updates/firmware",
    },
    Section {
        category: Category::Backups,
        label: "shell-overview",
        route: "backups/overview",
    },
    Section {
        category: Category::Backups,
        label: "shell-snapshots",
        route: "backups/history",
    },
    Section {
        category: Category::Backups,
        label: "shell-encrypted-backups",
        route: "backups/schedule",
    },
    Section {
        category: Category::Backups,
        label: "shell-whats-backed-up",
        route: "backups/rules",
    },
    Section {
        category: Category::Backups,
        label: "shell-backup-space",
        route: "backups/space",
    },
    Section {
        category: Category::Backups,
        label: "shell-restore-from-a-backup",
        route: "backups/points",
    },
    Section {
        category: Category::Apps,
        label: "shell-default-apps",
        route: "apps/defaults",
    },
    Section {
        category: Category::Apps,
        label: "shell-startup-apps",
        route: "apps/startup",
    },
    Section {
        category: Category::Apps,
        label: "shell-background-apps",
        route: "apps/background",
    },
    Section {
        category: Category::Apps,
        label: "shell-app-permissions",
        route: "apps/permissions",
    },
    Section {
        category: Category::Apps,
        label: "shell-location",
        route: "apps/location",
    },
    Section {
        category: Category::Apps,
        label: "shell-screen-sharing",
        route: "apps/screen_sharing",
    },
    Section {
        category: Category::Apps,
        label: "shell-parental-controls",
        route: "apps/parental",
    },
    Section {
        category: Category::Apps,
        label: "shell-screen-time",
        route: "apps/screen_time",
    },
    Section {
        category: Category::Agents,
        label: "shell-permissions",
        route: "agents/permissions",
    },
    Section {
        category: Category::Agents,
        label: "shell-assistant",
        route: "agents/assistant",
    },
    Section {
        category: Category::Agents,
        label: "writing-tools",
        route: "agents/writing-tools",
    },
    Section {
        category: Category::Agents,
        label: "shell-on-device-ai",
        route: "agents/local-ai",
    },
    Section {
        category: Category::Agents,
        label: "shell-ai-providers",
        route: "agents/ai-providers",
    },
    Section {
        category: Category::Agents,
        label: "shell-identities",
        route: "agents/identities",
    },
    Section {
        category: Category::Agents,
        label: "shell-activity",
        route: "agents/activity",
    },
    Section {
        category: Category::About,
        label: "shell-device",
        route: "about/device",
    },
    Section {
        category: Category::About,
        label: "shell-system",
        route: "about/system",
    },
    Section {
        category: Category::About,
        label: "shell-hardware",
        route: "about/hardware",
    },
    Section {
        category: Category::About,
        label: "shell-problem-reports",
        route: "about/problems",
    },
];
