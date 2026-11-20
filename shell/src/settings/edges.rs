use enzui_shell::i18n::tr;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]

pub enum BarEdge {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarEdge {
    #[default]
    Left,
    Right,
}

impl BarEdge {
    pub const ALL: [Self; 2] = [Self::Top, Self::Bottom];
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => tr("shell-top"),
            Self::Bottom => tr("shell-bottom"),
        }
    }
}

/// The screen corner the assistant's button and panel use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssistantCorner {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

impl AssistantCorner {
    pub const ALL: [Self; 4] = [
        Self::BottomRight,
        Self::BottomLeft,
        Self::TopRight,
        Self::TopLeft,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::BottomRight => tr("shell-bottom-right"),
            Self::BottomLeft => tr("shell-bottom-left"),
            Self::TopRight => tr("shell-top-right"),
            Self::TopLeft => tr("shell-top-left"),
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::BottomRight => "bottom_right",
            Self::BottomLeft => "bottom_left",
            Self::TopRight => "top_right",
            Self::TopLeft => "top_left",
        }
    }
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|corner| corner.key() == key)
    }
    pub fn bottom(self) -> bool {
        matches!(self, Self::BottomRight | Self::BottomLeft)
    }
    pub fn right(self) -> bool {
        matches!(self, Self::BottomRight | Self::TopRight)
    }
}

impl SidebarEdge {
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => tr("shell-left"),
            Self::Right => tr("shell-right"),
        }
    }
}
