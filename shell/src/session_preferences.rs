use enzui_shell::i18n::tr;
use serde::{Deserialize, Serialize};

/// What closing the lid of an undocked laptop does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LidAction {
    /// D-105: lock, then suspend.
    #[default]
    LockSuspend,
    Lock,
    Nothing,
}
impl LidAction {
    pub const ALL: [Self; 3] = [Self::LockSuspend, Self::Lock, Self::Nothing];
    pub fn label(self) -> &'static str {
        match self {
            Self::LockSuspend => tr("shell-lock-and-suspend"),
            Self::Lock => tr("shell-lock-only"),
            Self::Nothing => tr("shell-do-nothing"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockStyle {
    #[default]
    #[serde(alias = "orbit", alias = "modern", alias = "compact")]
    Prism,
    #[serde(alias = "editorial")]
    Muse,
    Glass,
    FrostedGlass,
}
impl ClockStyle {
    pub const ALL: [Self; 4] = [Self::Prism, Self::Muse, Self::Glass, Self::FrostedGlass];
    pub fn label(self) -> &'static str {
        match self {
            Self::Prism => tr("shell-prism"),
            Self::Muse => tr("shell-muse"),
            Self::Glass => tr("shell-glass"),
            Self::FrostedGlass => tr("shell-frosted-glass"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockSize {
    Small,
    #[default]
    Medium,
    Large,
}
impl ClockSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => tr("shell-small"),
            Self::Medium => tr("shell-medium"),
            Self::Large => tr("shell-large"),
        }
    }
    pub fn factor(self) -> f32 {
        match self {
            Self::Small => 0.12,
            Self::Medium => 0.18,
            Self::Large => 0.23,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockMaterial {
    #[default]
    Glass,
    FrostedGlass,
    Simple,
}
impl LockMaterial {
    pub const ALL: [Self; 3] = [Self::Glass, Self::FrostedGlass, Self::Simple];
    pub fn label(self) -> &'static str {
        match self {
            Self::Glass => tr("shell-glass"),
            Self::FrostedGlass => tr("shell-frosted-glass-2"),
            Self::Simple => tr("shell-simple"),
        }
    }
    pub fn style(self) -> ClockStyle {
        match self {
            Self::Glass => ClockStyle::Glass,
            Self::FrostedGlass => ClockStyle::FrostedGlass,
            Self::Simple => ClockStyle::Prism,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockLayout {
    #[default]
    Centered,
    Split,
    Stacked,
    Corner,
}
impl LockLayout {
    pub const ALL: [Self; 4] = [Self::Centered, Self::Split, Self::Stacked, Self::Corner];
    pub fn label(self) -> &'static str {
        match self {
            Self::Centered => tr("shell-centered"),
            Self::Split => tr("shell-duet"),
            Self::Stacked => tr("shell-portrait"),
            Self::Corner => tr("shell-horizon"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockClock {
    pub show_time: bool,
    pub show_date: bool,
    // Kept for reading preferences written before material and layout split.
    pub style: ClockStyle,
    pub size: ClockSize,
    pub material: Option<LockMaterial>,
    pub layout: LockLayout,
    pub font: Option<String>,
    pub weight: u16,
    pub background: Option<String>,
    pub hide_wallpaper_clock: bool,
    pub hide_wallpaper_date: bool,
}
impl LockClock {
    pub fn material(&self) -> LockMaterial {
        self.material.unwrap_or(match self.style {
            ClockStyle::Glass => LockMaterial::Glass,
            ClockStyle::FrostedGlass => LockMaterial::FrostedGlass,
            _ => LockMaterial::Simple,
        })
    }
    pub fn font_family(&self) -> &str {
        self.font.as_deref().unwrap_or("Fredoka")
    }
    pub fn stacked(&self, width: f32) -> bool {
        self.layout == LockLayout::Stacked || self.layout == LockLayout::Split && width >= 680.0
    }
}
impl Default for LockClock {
    fn default() -> Self {
        Self {
            show_time: true,
            show_date: true,
            style: ClockStyle::Glass,
            size: Default::default(),
            material: None,
            layout: Default::default(),
            font: None,
            weight: 600,
            background: None,
            hide_wallpaper_clock: true,
            hide_wallpaper_date: true,
        }
    }
}

/// Idle, lock and clock behavior of the session, stored under `session`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionPreferences {
    pub clock_24h: bool,
    pub lock_clock: LockClock,
    /// Minutes without input before the screen turns off; 0 never.
    pub blank_minutes: u16,
    /// Minutes after blanking before the session locks; `None` never locks
    /// automatically. D-104 keeps automatic locking on by default.
    pub lock_delay_minutes: Option<u16>,
    /// Minutes idle before suspending; 0 never.
    pub suspend_battery_minutes: u16,
    pub suspend_ac_minutes: u16,
    pub lid: LidAction,
}
impl Default for SessionPreferences {
    fn default() -> Self {
        Self {
            clock_24h: true,
            lock_clock: Default::default(),
            blank_minutes: 5,
            lock_delay_minutes: Some(0),
            suspend_battery_minutes: 15,
            suspend_ac_minutes: 0,
            lid: LidAction::LockSuspend,
        }
    }
}

pub const BLANK_CHOICES: [u16; 7] = [1, 2, 5, 10, 15, 30, 0];
pub const LOCK_CHOICES: [Option<u16>; 4] = [Some(0), Some(1), Some(5), None];
pub const SUSPEND_CHOICES: [u16; 5] = [15, 30, 60, 120, 0];

pub fn minutes_label(minutes: u16) -> String {
    match minutes {
        0 => tr("shell-never").into(),
        m if m % 60 == 0 => crate::i18n::count("duration-hours", m / 60),
        m => crate::i18n::count("duration-minutes", m),
    }
}
pub fn lock_label(delay: Option<u16>) -> String {
    match delay {
        None => tr("shell-never").into(),
        Some(0) => tr("shell-immediately").into(),
        Some(m) => minutes_label(m),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionChange {
    Clock24h(bool),
    LockTime(bool),
    LockDate(bool),
    LockStyle(ClockStyle),
    LockSize(ClockSize),
    LockMaterial(LockMaterial),
    LockLayout(LockLayout),
    LockFont(Option<String>),
    LockWeight(u16),
    LockBackground(Option<String>),
    LockWallpaperClock(bool),
    LockWallpaperDate(bool),
    LockFontReset,
    Blank(u16),
    LockDelay(Option<u16>),
    SuspendBattery(u16),
    SuspendAc(u16),
    Lid(LidAction),
}
impl SessionPreferences {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-session-preferences"))?,
            None => Self::default(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        let ok = self.blank_minutes <= 240
            && self.lock_delay_minutes.is_none_or(|m| m <= 60)
            && self.suspend_battery_minutes <= 720
            && self.suspend_ac_minutes <= 720
            && (100..=900).contains(&self.lock_clock.weight)
            && self.lock_clock.font.as_ref().is_none_or(|s| {
                !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(char::is_control)
            })
            && self.lock_clock.background.as_ref().is_none_or(|s| {
                s.starts_with('/') && s.len() <= 4096 && !s.chars().any(char::is_control)
            });
        if ok {
            Ok(())
        } else {
            Err(tr("shell-session-preferences-exceed-their-bounds").into())
        }
    }
    pub fn apply(&mut self, change: &SessionChange) -> Result<(), String> {
        let mut next = self.clone();
        match *change {
            SessionChange::Clock24h(on) => next.clock_24h = on,
            SessionChange::LockTime(on) => next.lock_clock.show_time = on,
            SessionChange::LockDate(on) => next.lock_clock.show_date = on,
            SessionChange::LockStyle(style) => {
                next.lock_clock.style = style;
                next.lock_clock.material = None;
            }
            SessionChange::LockMaterial(material) => next.lock_clock.material = Some(material),
            SessionChange::LockLayout(layout) => next.lock_clock.layout = layout,
            SessionChange::LockFont(ref family) => {
                next.lock_clock.font.clone_from(family);
                next.lock_clock.weight = crate::lock_fonts::weights(next.lock_clock.font_family())
                    .into_iter()
                    .min_by_key(|w| w.abs_diff(600))
                    .unwrap_or(600);
            }
            SessionChange::LockWeight(weight) => next.lock_clock.weight = weight,
            SessionChange::LockBackground(ref path) => next.lock_clock.background.clone_from(path),
            SessionChange::LockWallpaperClock(hide) => next.lock_clock.hide_wallpaper_clock = hide,
            SessionChange::LockWallpaperDate(hide) => next.lock_clock.hide_wallpaper_date = hide,
            SessionChange::LockFontReset => {
                next.lock_clock.font = None;
                next.lock_clock.weight = 600;
            }
            SessionChange::LockSize(size) => next.lock_clock.size = size,
            SessionChange::Blank(m) => next.blank_minutes = m,
            SessionChange::LockDelay(m) => next.lock_delay_minutes = m,
            SessionChange::SuspendBattery(m) => next.suspend_battery_minutes = m,
            SessionChange::SuspendAc(m) => next.suspend_ac_minutes = m,
            SessionChange::Lid(action) => next.lid = action,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    /// Minutes of idleness before locking, if the session locks itself.
    pub fn lock_after_minutes(&self) -> Option<u16> {
        let delay = self.lock_delay_minutes?;
        // A screen that never blanks still locks after five idle minutes.
        let base = if self.blank_minutes == 0 {
            5
        } else {
            self.blank_minutes
        };
        Some(base + delay)
    }
    pub fn clock_format(&self) -> &'static str {
        if self.clock_24h { "%H:%M" } else { "%-I:%M %p" }
    }
    pub fn update_json(
        value: &mut serde_json::Value,
        change: &SessionChange,
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("session"))?;
        next.apply(change)?;
        value["session"] = serde_json::to_value(next)
            .map_err(|_| tr("shell-cannot-encode-session-preferences"))?;
        Ok(())
    }
}
