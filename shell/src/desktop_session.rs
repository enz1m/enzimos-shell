//! Desktop launch mode and host-isolation boundaries.
//!
//! Installed and preview modes use managed identities; only preview is nested.
//! ENZUI_SHELL_PLATFORM_PREVIEW independently controls test credentials.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Standalone,
    Preview,
    Installed,
}

impl Mode {
    pub fn from_values(session: Option<&str>, preview: Option<&str>) -> Self {
        match (session, preview) {
            (Some("installed"), _) => Mode::Installed,
            (Some("preview"), _) | (_, Some("1")) => Mode::Preview,
            _ => Mode::Standalone,
        }
    }
    pub fn managed(self) -> bool {
        self != Mode::Standalone
    }
    pub fn nested(self) -> bool {
        self == Mode::Preview
    }
}

/// A separate remote login's desktop: an installed login without a seat, so
/// the machine's drives, lid and sleep are not its to manage.
#[must_use]
pub fn remote() -> bool {
    std::env::var("ENZUI_REMOTE_SESSION").as_deref() == Ok("1")
}

pub fn mode() -> Mode {
    Mode::from_values(
        std::env::var("ENZUI_DESKTOP_SESSION").ok().as_deref(),
        std::env::var("ENZUI_SHELL_PLATFORM_PREVIEW")
            .ok()
            .as_deref(),
    )
}
