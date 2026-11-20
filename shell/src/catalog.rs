#[derive(Clone, Copy, Debug)]
pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    /// The app's icon in the icon theme (its application ID), when it has one;
    /// `icon` and `color` draw the stand-in when the theme lacks it.
    pub theme_icon: Option<&'static str>,
    pub color: [u8; 3],
}

impl App {
    pub fn display_name(self) -> &'static str {
        crate::i18n::label(self.name)
    }
    pub fn display_description(self) -> &'static str {
        crate::i18n::label(self.description)
    }
    /// The app's own icon from the icon theme, if the theme has it.
    pub fn themed_icon(self, size: u32) -> Option<std::path::PathBuf> {
        crate::icon_theme::lookup(self.theme_icon?, size)
    }
}

/// The desktop entry id an installed catalog app is known by.
#[must_use]
pub fn desktop_id(id: &str) -> Option<&'static str> {
    Some(match id {
        "files-app" => "io.github.enzui.Files",
        "text-app" => "io.github.enzui.Text",
        "terminal-emulator" => "io.github.enzui.Terminal",
        "task-manager-app" => "io.github.enzui.TaskManager",
        _ => return None,
    })
}

pub const APPS: &[App] = &[
    App {
        id: "enzui-settings",
        name: "Settings",
        description: "Desktop and appearance",
        icon: "settings",
        theme_icon: Some("dev.enzui.Settings"),
        color: [106, 114, 130],
    },
    App {
        id: "files-app",
        name: "Files",
        description: "Folders and documents",
        icon: "folder",
        theme_icon: Some("io.github.enzui.Files"),
        color: [55, 130, 220],
    },
    App {
        id: "text-app",
        name: "Text",
        description: "Write and edit",
        icon: "file-text",
        theme_icon: Some("io.github.enzui.Text"),
        color: [61, 151, 126],
    },
    App {
        id: "terminal-emulator",
        name: "Terminal",
        description: "Command line",
        icon: "terminal",
        theme_icon: Some("io.github.enzui.Terminal"),
        color: [105, 104, 133],
    },
    App {
        id: "enzui-assistant",
        name: "Assistant",
        description: "Agent sessions with your approval",
        icon: "bot",
        theme_icon: Some("dev.enzui.Assistant"),
        color: [74, 120, 196],
    },
    App {
        id: "task-manager-app",
        name: "System Monitor",
        description: "Processes and resources",
        icon: "activity",
        theme_icon: Some("io.github.enzui.TaskManager"),
        color: [147, 103, 199],
    },
];

fn shell_binary(id: &str) -> bool {
    matches!(id, "enzui-settings" | "enzui-assistant")
}

/// Whether `app` belongs on this system: the Assistant only with an agent harness.
#[must_use]
pub fn offered(app: &App) -> bool {
    app.id != "enzui-assistant" || crate::agent_harness::installed()
}

pub fn executable(root: &std::path::Path, id: &str) -> std::path::PathBuf {
    if let Some(installed) = crate::installation::layout(root) {
        return if shell_binary(id) {
            installed.bin.join(id)
        } else {
            installed.apps.join(id)
        };
    }
    if shell_binary(id) {
        std::env::var_os("ENZUI_SHELL_BIN_DIR")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute() && path.starts_with(root))
            .unwrap_or_else(|| root.join("target/debug"))
            .join(id)
    } else {
        root.join(".build/apps/bin").join(id)
    }
}

pub fn matches(app: &App, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    format!("{} {}", app.name, app.description)
        .to_lowercase()
        .contains(&query)
}
