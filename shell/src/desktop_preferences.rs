use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionKey {
    InnerGap,
    OuterGap,
    Border,
    BorderColor,
    BorderCustom,
    BorderOpacity,
    InactiveBorderOpacity,
    Rounding,
    Animations,
    AnimationRate,
    AnimationStyle,
    FocusPointer,
    NaturalScroll,
    PointerSpeed,
    PointerAcceleration,
    LeftHanded,
    ScrollSpeed,
    TapToClick,
    DisableWhileTyping,
    RepeatDelay,
    RepeatRate,
    CapsLock,
    LayoutSwitch,
    Gestures,
    Zoom,
    CursorSize,
}
impl OptionKey {
    pub const ALL: [Self; 26] = [
        Self::InnerGap,
        Self::OuterGap,
        Self::Border,
        Self::BorderColor,
        Self::BorderCustom,
        Self::BorderOpacity,
        Self::InactiveBorderOpacity,
        Self::Rounding,
        Self::Animations,
        Self::AnimationRate,
        Self::AnimationStyle,
        Self::FocusPointer,
        Self::NaturalScroll,
        Self::PointerSpeed,
        Self::PointerAcceleration,
        Self::LeftHanded,
        Self::ScrollSpeed,
        Self::TapToClick,
        Self::DisableWhileTyping,
        Self::RepeatDelay,
        Self::RepeatRate,
        Self::CapsLock,
        Self::LayoutSwitch,
        Self::Gestures,
        Self::Zoom,
        Self::CursorSize,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::InnerGap => "gaps_inner",
            Self::OuterGap => "gaps_outer",
            Self::Border => "border_size",
            Self::BorderColor => "border_color",
            Self::BorderCustom => "border_custom",
            Self::BorderOpacity => "border_opacity",
            Self::InactiveBorderOpacity => "border_inactive_opacity",
            Self::Rounding => "rounding",
            Self::Animations => "animations",
            Self::AnimationRate => "animation_rate",
            Self::AnimationStyle => "animation_style",
            Self::FocusPointer => "focus_pointer",
            Self::NaturalScroll => "natural_scroll",
            Self::PointerSpeed => "pointer_speed",
            Self::PointerAcceleration => "pointer_acceleration",
            Self::LeftHanded => "left_handed",
            Self::ScrollSpeed => "scroll_speed",
            Self::TapToClick => "tap_to_click",
            Self::DisableWhileTyping => "disable_while_typing",
            Self::RepeatDelay => "repeat_delay",
            Self::RepeatRate => "repeat_rate",
            Self::CapsLock => "caps_lock",
            Self::LayoutSwitch => "layout_switch",
            Self::Gestures => "gestures",
            Self::Zoom => "zoom",
            Self::CursorSize => "cursor_size",
        }
    }
    pub fn bounds(self) -> (i32, i32) {
        match self {
            Self::InnerGap => (0, 48),
            Self::OuterGap => (0, 96),
            Self::Border => (0, 6),
            Self::BorderColor => (0, BorderColor::ALL.len() as i32 - 1),
            Self::BorderCustom => (0, 0xff_ffff),
            Self::BorderOpacity | Self::InactiveBorderOpacity => (0, 100),
            Self::Rounding => (0, 32),
            Self::AnimationRate => (25, 200),
            Self::AnimationStyle => (0, 3),
            Self::PointerSpeed => (-100, 100),
            Self::ScrollSpeed => (25, 300),
            Self::RepeatDelay => (150, 1000),
            Self::RepeatRate => (10, 80),
            Self::CapsLock => (0, CapsLock::ALL.len() as i32 - 1),
            Self::LayoutSwitch => (0, LayoutSwitch::ALL.len() as i32 - 1),
            Self::Zoom => (ZOOM_MIN, ZOOM_MAX),
            Self::CursorSize => (16, 96),
            _ => (0, 1),
        }
    }
    pub fn toggle(self) -> bool {
        matches!(
            self,
            Self::Animations
                | Self::FocusPointer
                | Self::NaturalScroll
                | Self::PointerAcceleration
                | Self::LeftHanded
                | Self::TapToClick
                | Self::DisableWhileTyping
                | Self::Gestures
        )
    }
}
/// What the focused window's border is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderColor {
    /// Quiet grey on floating windows; the accent on the focused tile.
    Automatic,
    Accent,
    Neutral,
    White,
    /// The colour chosen in `CompositorPreferences::border_custom`.
    Custom,
}
impl BorderColor {
    pub const ALL: [Self; 5] = [
        Self::Automatic,
        Self::Accent,
        Self::Neutral,
        Self::White,
        Self::Custom,
    ];
    pub fn from_value(value: i32) -> Self {
        usize::try_from(value)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
            .unwrap_or(Self::Automatic)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => tr("shell-border-color-automatic"),
            Self::Accent => tr("shell-border-color-accent"),
            Self::Neutral => tr("shell-border-color-neutral"),
            Self::White => tr("shell-border-color-white"),
            Self::Custom => tr("shell-border-color-custom"),
        }
    }
}
pub const MAX_LAYOUTS: usize = 4;
/// Screen zoom around the pointer, in percent.
pub const ZOOM_MIN: i32 = 100;
pub const ZOOM_MAX: i32 = 800;
pub const ZOOM_STEP: i32 = 50;

/// What Caps Lock does, as an XKB option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapsLock {
    Default,
    Control,
    Escape,
    Off,
}
impl CapsLock {
    pub const ALL: [Self; 4] = [Self::Default, Self::Control, Self::Escape, Self::Off];
    pub fn from_value(value: i32) -> Self {
        Self::ALL
            .get(value as usize)
            .copied()
            .unwrap_or(Self::Default)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => tr("shell-caps-lock-default"),
            Self::Control => tr("shell-control"),
            Self::Escape => tr("shell-escape"),
            Self::Off => tr("shell-disabled"),
        }
    }
    pub fn xkb_option(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Control => "caps:ctrl_modifier",
            Self::Escape => "caps:escape",
            Self::Off => "caps:none",
        }
    }
}

/// The key combination that moves to the next keyboard layout. Super +
/// Space is not offered: the shell uses it for Quick Search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutSwitch {
    Off,
    AltShift,
    CtrlSpace,
}
impl LayoutSwitch {
    pub const ALL: [Self; 3] = [Self::Off, Self::AltShift, Self::CtrlSpace];
    pub fn from_value(value: i32) -> Self {
        Self::ALL
            .get(value as usize)
            .copied()
            .unwrap_or(Self::AltShift)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => tr("shell-no-shortcut"),
            Self::AltShift => "Alt + Shift",
            Self::CtrlSpace => "Ctrl + Space",
        }
    }
    pub fn xkb_option(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::AltShift => "grp:alt_shift_toggle",
            Self::CtrlSpace => "grp:ctrl_space_toggle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompositorPreferences {
    pub gaps_inner: u16,
    pub gaps_outer: u16,
    pub border_size: u16,
    /// What the focused window's border is drawn in (`BorderColor`).
    pub border_color: u8,
    /// The custom border colour as 0xRRGGBB, used when `border_color` is
    /// `BorderColor::Custom`.
    pub border_custom: u32,
    /// Opacity of the focused window's border, in percent.
    pub border_opacity: u8,
    /// Opacity of every other window's border, in percent.
    pub border_inactive_opacity: u8,
    /// Window corner radius; unset follows the theme's window radius, so the
    /// compositor rounds windows as enzUI apps draw their own elsewhere.
    pub rounding: Option<u16>,
    pub animations: bool,
    pub animation_rate: u16,
    pub animation_style: u8,
    pub focus_pointer: bool,
    pub natural_scroll: bool,
    pub pointer_speed: i16,
    pub pointer_acceleration: bool,
    pub left_handed: bool,
    pub scroll_speed: u16,
    pub tap_to_click: bool,
    pub disable_while_typing: bool,
    pub repeat_delay: u16,
    pub repeat_rate: u16,
    pub caps_lock: u8,
    pub layout_switch: u8,
    /// Three-finger touchpad swipes switch desktops and open the overview.
    pub gestures: bool,
    /// Percent; 100 is no zoom.
    pub zoom: u16,
    /// Pointer size in pixels.
    pub cursor_size: u16,
    /// Comma-separated XKB layout codes; empty keeps the compositor's own.
    pub keyboard_layouts: String,
    /// The program Super+Return starts; empty keeps the installed desktop's.
    pub terminal: String,
    /// X11 apps draw at the display's full resolution (sharp, sized by the
    /// app) instead of being scaled up by the compositor.
    pub x11_sharp: bool,
    /// Colour mode per output name (`wide`, `hdr`); absent is standard sRGB.
    pub color_modes: std::collections::BTreeMap<String, String>,
    /// Chosen chords per changeable action (`crate::shortcut_map`); an
    /// action not listed keeps its default.
    pub shortcuts: std::collections::BTreeMap<String, String>,
}
impl Default for CompositorPreferences {
    fn default() -> Self {
        Self {
            gaps_inner: 6,
            gaps_outer: 12,
            border_size: 2,
            border_color: 0,
            border_custom: 0x0a84ff,
            border_opacity: 80,
            border_inactive_opacity: 20,
            rounding: None,
            animations: true,
            animation_rate: 150,
            animation_style: 0,
            focus_pointer: false,
            natural_scroll: false,
            pointer_speed: 0,
            pointer_acceleration: true,
            left_handed: false,
            scroll_speed: 100,
            tap_to_click: true,
            disable_while_typing: true,
            repeat_delay: 600,
            repeat_rate: 25,
            caps_lock: 0,
            layout_switch: 1,
            gestures: true,
            zoom: 100,
            cursor_size: 24,
            keyboard_layouts: String::new(),
            terminal: String::new(),
            x11_sharp: false,
            color_modes: std::collections::BTreeMap::new(),
            shortcuts: std::collections::BTreeMap::new(),
        }
    }
}
impl CompositorPreferences {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-compositor-preferences"))?,
            None => Self::default(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn motion(&self) -> crate::motion::Policy {
        crate::motion::Policy {
            rate: self.animation_rate,
            windows: self.animations,
        }
    }
    /// The window corner radius in effect: the chosen one, else the theme's.
    pub fn rounding(&self) -> u16 {
        self.rounding.unwrap_or_else(|| {
            let (min, max) = OptionKey::Rounding.bounds();
            enzui_theme::text_defaults::window_radius()
                .round()
                .clamp(min as f32, max as f32) as u16
        })
    }
    pub fn value(&self, key: OptionKey) -> i32 {
        match key {
            OptionKey::InnerGap => self.gaps_inner.into(),
            OptionKey::OuterGap => self.gaps_outer.into(),
            OptionKey::Border => self.border_size.into(),
            OptionKey::BorderColor => self.border_color.into(),
            OptionKey::BorderCustom => self.border_custom as i32,
            OptionKey::BorderOpacity => self.border_opacity.into(),
            OptionKey::InactiveBorderOpacity => self.border_inactive_opacity.into(),
            OptionKey::Rounding => self.rounding().into(),
            OptionKey::Animations => self.animations.into(),
            OptionKey::AnimationRate => self.animation_rate.into(),
            OptionKey::AnimationStyle => self.animation_style.into(),
            OptionKey::FocusPointer => self.focus_pointer.into(),
            OptionKey::NaturalScroll => self.natural_scroll.into(),
            OptionKey::PointerSpeed => self.pointer_speed.into(),
            OptionKey::PointerAcceleration => self.pointer_acceleration.into(),
            OptionKey::LeftHanded => self.left_handed.into(),
            OptionKey::ScrollSpeed => self.scroll_speed.into(),
            OptionKey::TapToClick => self.tap_to_click.into(),
            OptionKey::DisableWhileTyping => self.disable_while_typing.into(),
            OptionKey::RepeatDelay => self.repeat_delay.into(),
            OptionKey::RepeatRate => self.repeat_rate.into(),
            OptionKey::CapsLock => self.caps_lock.into(),
            OptionKey::LayoutSwitch => self.layout_switch.into(),
            OptionKey::Gestures => self.gestures.into(),
            OptionKey::Zoom => self.zoom.into(),
            OptionKey::CursorSize => self.cursor_size.into(),
        }
    }
    pub fn set(&mut self, key: OptionKey, value: i32) -> Result<(), String> {
        let (min, max) = key.bounds();
        if !(min..=max).contains(&value) {
            return Err(
                tr!("shell-must-be-between-and", "arg0" => key.key().to_owned(), "arg1" => format!("{:}", min), "arg2" => format!("{:}", max)),
            );
        }
        match key {
            OptionKey::InnerGap => self.gaps_inner = value as u16,
            OptionKey::OuterGap => self.gaps_outer = value as u16,
            OptionKey::Border => self.border_size = value as u16,
            OptionKey::BorderColor => self.border_color = value as u8,
            OptionKey::BorderCustom => self.border_custom = value as u32,
            OptionKey::BorderOpacity => self.border_opacity = value as u8,
            OptionKey::InactiveBorderOpacity => self.border_inactive_opacity = value as u8,
            OptionKey::Rounding => self.rounding = Some(value as u16),
            OptionKey::Animations => self.animations = value != 0,
            OptionKey::AnimationRate => self.animation_rate = value as u16,
            OptionKey::AnimationStyle => self.animation_style = value as u8,
            OptionKey::FocusPointer => self.focus_pointer = value != 0,
            OptionKey::NaturalScroll => self.natural_scroll = value != 0,
            OptionKey::PointerSpeed => self.pointer_speed = value as i16,
            OptionKey::PointerAcceleration => self.pointer_acceleration = value != 0,
            OptionKey::LeftHanded => self.left_handed = value != 0,
            OptionKey::ScrollSpeed => self.scroll_speed = value as u16,
            OptionKey::TapToClick => self.tap_to_click = value != 0,
            OptionKey::DisableWhileTyping => self.disable_while_typing = value != 0,
            OptionKey::RepeatDelay => self.repeat_delay = value as u16,
            OptionKey::RepeatRate => self.repeat_rate = value as u16,
            OptionKey::CapsLock => self.caps_lock = value as u8,
            OptionKey::LayoutSwitch => self.layout_switch = value as u8,
            OptionKey::Gestures => self.gestures = value != 0,
            OptionKey::Zoom => self.zoom = value as u16,
            OptionKey::CursorSize => self.cursor_size = value as u16,
        }
        Ok(())
    }
    pub fn layouts(&self) -> Vec<&str> {
        self.keyboard_layouts
            .split(',')
            .filter(|code| !code.is_empty())
            .collect()
    }
    pub fn set_layouts(&mut self, layouts: &[String]) -> Result<(), String> {
        if layouts.len() > MAX_LAYOUTS
            || !layouts
                .iter()
                .all(|code| crate::system_settings::valid_layout(code))
        {
            return Err(tr("shell-keyboard-layouts-must-be-up-to-four-xkb-layout-codes").into());
        }
        let mut unique: Vec<&str> = Vec::new();
        for code in layouts {
            if !unique.contains(&code.as_str()) {
                unique.push(code);
            }
        }
        self.keyboard_layouts = unique.join(",");
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        for key in OptionKey::ALL {
            let (min, max) = key.bounds();
            if !(min..=max).contains(&self.value(key)) {
                return Err(tr("shell-compositor-preferences-exceed-their-bounds").into());
            }
        }
        let layouts: Vec<String> = self.layouts().into_iter().map(str::to_owned).collect();
        if layouts.len() > MAX_LAYOUTS
            || !layouts
                .iter()
                .all(|code| crate::system_settings::valid_layout(code))
        {
            return Err(tr("shell-invalid-keyboard-layouts").into());
        }
        if !valid_terminal(&self.terminal) {
            return Err(tr("shell-invalid-terminal").into());
        }
        if self.color_modes.len() > 8
            || !self.color_modes.iter().all(|(output, mode)| {
                valid_output_name(output) && COLOR_MODES.contains(&mode.as_str())
            })
        {
            return Err(tr("shell-invalid-colour-modes").into());
        }
        if !crate::shortcut_map::valid(&self.shortcuts) {
            return Err(tr("shell-invalid-keyboard-shortcuts").into());
        }
        Ok(())
    }
    /// Chooses `chord` for the action `id`, or its default for `None`; every
    /// default with `id` empty.
    pub fn set_shortcut(&mut self, id: &str, chord: Option<&str>) -> Result<(), String> {
        match (id, chord) {
            ("", _) => self.shortcuts.clear(),
            (id, None) => {
                self.shortcuts.remove(id);
            }
            (id, Some(chord)) => {
                let action = crate::shortcut_map::ACTIONS
                    .iter()
                    .find(|action| action.id == id)
                    .ok_or_else(|| tr("shell-unknown-shortcut"))?;
                if chord == action.default {
                    self.shortcuts.remove(id);
                } else {
                    self.shortcuts.insert(id.to_owned(), chord.to_owned());
                }
            }
        }
        if !crate::shortcut_map::valid(&self.shortcuts) {
            return Err(tr("shell-that-shortcut-is-already-in-use-or-cannot-be-used").into());
        }
        Ok(())
    }
    pub fn update_shortcut_json(
        value: &mut serde_json::Value,
        id: &str,
        chord: Option<&str>,
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("compositor"))?;
        next.set_shortcut(id, chord)?;
        if value.get("compositor").is_none() {
            value["compositor"] = serde_json::json!({});
        }
        value["compositor"]["shortcuts"] = serde_json::to_value(&next.shortcuts)
            .map_err(|_| tr("shell-cannot-encode-shortcuts"))?;
        Ok(())
    }
    pub fn update_display_json(
        value: &mut serde_json::Value,
        change: &DisplayChange,
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("compositor"))?;
        match change {
            DisplayChange::X11Sharp(on) => next.x11_sharp = *on,
            DisplayChange::ColorMode {
                output,
                mode: Some(mode),
            } => {
                next.color_modes.insert(output.clone(), mode.clone());
            }
            DisplayChange::ColorMode { output, mode: None } => {
                next.color_modes.remove(output);
            }
        }
        next.validate()?;
        if value.get("compositor").is_none() {
            value["compositor"] = serde_json::json!({});
        }
        value["compositor"]["x11_sharp"] = next.x11_sharp.into();
        value["compositor"]["color_modes"] = serde_json::to_value(&next.color_modes)
            .map_err(|_| tr("shell-cannot-encode-colour-modes"))?;
        Ok(())
    }
    pub fn update_terminal_json(
        value: &mut serde_json::Value,
        terminal: &str,
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("compositor"))?;
        terminal.clone_into(&mut next.terminal);
        next.validate()?;
        if value.get("compositor").is_none() {
            value["compositor"] = serde_json::json!({});
        }
        value["compositor"]["terminal"] = next.terminal.into();
        Ok(())
    }
    pub fn update_layouts_json(
        value: &mut serde_json::Value,
        layouts: &[String],
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("compositor"))?;
        next.set_layouts(layouts)?;
        if value.get("compositor").is_none() {
            value["compositor"] = serde_json::json!({});
        }
        value["compositor"]["keyboard_layouts"] = next.keyboard_layouts.into();
        Ok(())
    }
    pub fn update_json(
        value: &mut serde_json::Value,
        key: OptionKey,
        setting: i32,
    ) -> Result<(), String> {
        let mut next = Self::from_json(value.get("compositor"))?;
        next.set(key, setting)?;
        let serialized = serde_json::to_value(next)
            .map_err(|_| tr("shell-cannot-encode-compositor-preferences"))?;
        if value.get("compositor").is_none() {
            value["compositor"] = serde_json::json!({});
        }
        value["compositor"][key.key()] = serialized[key.key()].clone();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperKind {
    #[default]
    Default,
    Color,
    Image,
    Live,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperFit {
    #[default]
    Fill,
    Fit,
    Center,
    Stretch,
    Tile,
}
impl WallpaperFit {
    pub const ALL: [Self; 5] = [
        Self::Fill,
        Self::Fit,
        Self::Center,
        Self::Stretch,
        Self::Tile,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fill => tr("shell-fill"),
            Self::Fit => tr("shell-fit"),
            Self::Center => tr("shell-center"),
            Self::Stretch => tr("shell-stretch"),
            Self::Tile => tr("shell-tile"),
        }
    }
}
/// Frame rate policy for live wallpapers, passed to enzwall as `--smooth`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperMotion {
    /// Smooth on mains power, Balanced on battery.
    #[default]
    Auto,
    Smooth,
    Balanced,
}
impl WallpaperMotion {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Smooth, Self::Balanced];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => tr("shell-automatic"),
            Self::Smooth => tr("shell-smooth"),
            Self::Balanced => tr("shell-balanced"),
        }
    }
}
#[derive(Clone, Debug)]
pub enum WallpaperChange {
    Kind(WallpaperKind),
    Color(String),
    Solid(String),
    Image(String),
    Live(String),
    Motion(WallpaperMotion),
    PauseIdle(bool),
    HideClock(bool),
    HideDate(bool),
    Fit(WallpaperFit),
    Retry,
}
impl WallpaperChange {
    pub fn same_slot(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Color(_), Self::Color(_))
                | (Self::Fit(_), Self::Fit(_))
                | (Self::Retry, Self::Retry)
                | (Self::PauseIdle(_), Self::PauseIdle(_))
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    pub kind: WallpaperKind,
    pub color: String,
    pub image: Option<String>,
    pub fit: WallpaperFit,
    pub revision: u64,
    /// Bundled enzwall scene id, used when `kind` is `Live`.
    pub scene: Option<String>,
    pub motion: WallpaperMotion,
    /// Live scenes sleep (dim and stop moving) after a while without input.
    pub pause_idle: bool,
    pub hide_clock: bool,
    pub hide_date: bool,
}
impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            kind: WallpaperKind::Default,
            color: "#172b40".into(),
            image: None,
            fit: WallpaperFit::Fill,
            revision: 0,
            scene: None,
            motion: WallpaperMotion::Auto,
            pause_idle: false,
            hide_clock: false,
            hide_date: false,
        }
    }
}
pub fn valid_scene_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}
impl Wallpaper {
    pub fn apply(&mut self, change: WallpaperChange) -> Result<(), String> {
        let mut next = self.clone();
        match change {
            WallpaperChange::Kind(kind) => next.kind = kind,
            WallpaperChange::Color(color) => next.color = color,
            WallpaperChange::Solid(color) => {
                next.color = color;
                next.kind = WallpaperKind::Color;
            }
            WallpaperChange::Image(image) => {
                next.image = Some(image);
                next.kind = WallpaperKind::Image;
            }
            WallpaperChange::Live(scene) => {
                next.scene = Some(scene);
                next.kind = WallpaperKind::Live;
            }
            WallpaperChange::Motion(motion) => next.motion = motion,
            WallpaperChange::PauseIdle(pause) => next.pause_idle = pause,
            WallpaperChange::HideClock(hide) => next.hide_clock = hide,
            WallpaperChange::HideDate(hide) => next.hide_date = hide,
            WallpaperChange::Fit(fit) => next.fit = fit,
            WallpaperChange::Retry => {}
        }
        next.validate()?;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| tr("shell-wallpaper-revisions-exhausted"))?;
        *self = next;
        Ok(())
    }
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-wallpaper-preferences"))?,
            None => Self::default(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.color.len() != 7
            || !self.color.starts_with('#')
            || !self
                .color
                .strip_prefix('#')
                .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(tr("shell-use-a-six-digit-color-such-as-172b40").into());
        }
        if self.image.as_ref().is_some_and(|path| {
            path.len() > 4096
                || path.chars().any(char::is_control)
                || !std::path::Path::new(path).is_absolute()
        }) {
            return Err(tr("shell-wallpaper-must-name-an-absolute-local-image-path").into());
        }
        if self.kind == WallpaperKind::Image && self.image.is_none() {
            return Err(tr("shell-choose-a-wallpaper-image-first").into());
        }
        if self
            .scene
            .as_deref()
            .is_some_and(|scene| !valid_scene_id(scene))
        {
            return Err(
                tr("shell-live-wallpaper-names-use-lowercase-letters-digits-and-dashes").into(),
            );
        }
        if self.kind == WallpaperKind::Live && self.scene.is_none() {
            return Err(tr("shell-choose-a-live-wallpaper-first").into());
        }
        Ok(())
    }
}

/// System-localed X11 keyboard configuration, shared with the greeter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemKeyboard {
    pub layout: String,
    pub variant: String,
    pub options: Vec<String>,
}

pub fn system_keyboard() -> SystemKeyboard {
    parse_system_keyboard(
        &std::fs::read_to_string("/etc/X11/xorg.conf.d/00-keyboard.conf").unwrap_or_default(),
    )
}

fn xkb_option<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let quoted = format!("\"{name}\"");
    let line = text.lines().find(|l| {
        l.split_whitespace().next() == Some("Option")
            && l.split_whitespace().nth(1) == Some(quoted.as_str())
    })?;
    line.split('"').nth(3)
}

fn parse_system_keyboard(text: &str) -> SystemKeyboard {
    let layout = xkb_option(text, "XkbLayout").filter(|v| {
        !v.is_empty()
            && v.split(',').count() <= MAX_LAYOUTS
            && v.split(',').all(crate::system_settings::valid_layout)
    });
    let Some(layout) = layout else {
        return SystemKeyboard {
            layout: "us".into(),
            ..SystemKeyboard::default()
        };
    };
    let variant = xkb_option(text, "XkbVariant")
        .filter(|v| {
            v.len() <= 256
                && v.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_,-".contains(&b))
        })
        .unwrap_or_default();
    let options = xkb_option(text, "XkbOptions")
        .unwrap_or_default()
        .split(',')
        .filter(|o| {
            !o.is_empty()
                && o.len() <= 64
                && o.contains(':')
                && o.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:".contains(&b))
        })
        .take(16)
        .map(str::to_owned)
        .collect();
    SystemKeyboard {
        layout: layout.into(),
        variant: variant.into(),
        options,
    }
}

// Preserve system variants and XKB options unless Settings explicitly overrides them.
fn keyboard(
    preferences: &CompositorPreferences,
    system: &SystemKeyboard,
) -> (String, String, String) {
    let caps = CapsLock::from_value(preferences.caps_lock.into()).xkb_option();
    let switch = LayoutSwitch::from_value(preferences.layout_switch.into()).xkb_option();
    let own_layouts = !preferences.keyboard_layouts.is_empty();
    let kept = system.options.iter().filter(|o| {
        !(o.starts_with("caps:") && !caps.is_empty())
            && !(o.starts_with("grp:") && (!switch.is_empty() || own_layouts))
    });
    let options: Vec<&str> = kept
        .map(String::as_str)
        .chain([caps, switch].into_iter().filter(|o| !o.is_empty()))
        .collect();
    if own_layouts {
        let mut system_variants = system.variant.split(',');
        let variants: std::collections::HashMap<&str, &str> = system
            .layout
            .split(',')
            .map(|layout| (layout, system_variants.next().unwrap_or("")))
            .collect();
        let chosen: Vec<&str> = preferences
            .keyboard_layouts
            .split(',')
            .map(|layout| variants.get(layout).copied().unwrap_or(""))
            .collect();
        let variant = if chosen.iter().all(|v| v.is_empty()) {
            String::new()
        } else {
            chosen.join(",")
        };
        (
            preferences.keyboard_layouts.clone(),
            variant,
            options.join(","),
        )
    } else {
        (
            system.layout.clone(),
            system.variant.clone(),
            options.join(","),
        )
    }
}

impl crate::compositor::Compositor {
    /// What each output can show, and its colour preset now.
    pub async fn color_support(&self) -> Result<Vec<ColorSupport>, String> {
        let monitors: serde_json::Value =
            serde_json::from_str(&self.request(&["-j", "monitors", "all"]).await?)
                .map_err(|_| tr("shell-invalid-output-list"))?;
        Ok(color_support(&monitors))
    }
    pub async fn apply_desktop_preferences(
        &self,
        preferences: &CompositorPreferences,
    ) -> Result<(), String> {
        preferences.validate()?;
        self.desktop_output().await?;
        let p = preferences;
        let keyboard = keyboard(p, &system_keyboard());
        let command = format!(
            "{}\nreturn enzui_apply_desktop_preferences({},{},{},{},{},{},{},{},{},{{accel={},left_handed={},scroll={},tap={},dwt={},repeat_delay={},repeat_rate={},gestures={},zoom={},cursor={},border_color={},border_custom={},border_active={},border_inactive={},terminal=\"{}\",x11_sharp={},kb_options=\"{}\",kb_layout=\"{}\",kb_variant=\"{}\"}})",
            include_str!("../../config/desktop-preferences.lua"),
            p.gaps_inner,
            p.gaps_outer,
            p.border_size,
            p.rounding(),
            p.animations,
            p.focus_pointer,
            p.natural_scroll,
            p.pointer_speed,
            p.animation_rate,
            p.pointer_acceleration,
            p.left_handed,
            p.scroll_speed,
            p.tap_to_click,
            p.disable_while_typing,
            p.repeat_delay,
            p.repeat_rate,
            p.gestures,
            p.zoom,
            p.cursor_size,
            p.border_color,
            p.border_custom,
            p.border_opacity,
            p.border_inactive_opacity,
            terminal(p),
            p.x11_sharp,
            keyboard.2,
            keyboard.0,
            keyboard.1,
        );
        if self.request(&["eval", &command]).await?.trim() != "ok" {
            return Err(tr("shell-compositor-settings-were-rejected").into());
        }
        match self
            .request(&["eval", &crate::shortcut_map::lua(&p.shortcuts)])
            .await?
            .trim()
        {
            "ok" => {}
            "missing" => {
                return Err(
                    tr("shell-changed-shortcuts-need-the-current-desktop-configuration").into(),
                );
            }
            _ => {
                return Err(tr("shell-keyboard-shortcuts-were-rejected").into());
            }
        }
        let colors = color_lua(p, &self.color_support().await?);
        if !colors.is_empty()
            && self
                .request(&["eval", &format!("{colors}return \"ok\"")])
                .await?
                .trim()
                != "ok"
        {
            return Err(tr("shell-colour-modes-were-rejected").into());
        }
        // The pointer the compositor draws (and the cursor-shape pointers of
        // apps) changes size at once; new apps also get it in XCURSOR_SIZE.
        let size = p.cursor_size.to_string();
        if self.request(&["setcursor", &size]).await?.trim() != "ok" {
            return Err(tr("shell-pointer-size-was-not-applied").into());
        }
        let response = self.request(&["-j", "animations"]).await?;
        let value =
            serde_json::from_str(&response).map_err(|_| tr("shell-invalid-animation-readback"))?;
        if animation_readback(&value, p.animation_rate) {
            Ok(())
        } else {
            Err(tr("shell-animation-speed-was-not-applied").into())
        }
    }
}

/// The colour modes Settings offers beyond standard sRGB.
pub const COLOR_MODES: [&str; 2] = ["wide", "hdr"];

/// A display-wide preference the shell gives the compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayChange {
    X11Sharp(bool),
    /// `None` returns the output to standard sRGB.
    ColorMode {
        output: String,
        mode: Option<String>,
    },
}

#[must_use]
pub fn valid_output_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with("ENZREMOTE-")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// What one output can show, from `hyprctl -j monitors`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorSupport {
    pub name: String,
    /// The compositor's preset now (`srgb`, `wide`, `hdr`, …).
    pub mode: String,
    /// The display's EDID describes HDR, or BT.2020 primaries.
    pub hdr: bool,
    pub wide: bool,
}

#[must_use]
pub fn color_support(monitors: &serde_json::Value) -> Vec<ColorSupport> {
    monitors
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m["name"]
                .as_str()
                .filter(|n| valid_output_name(n))?
                .to_owned();
            let details = &m["hardwareDetails"];
            Some(ColorSupport {
                name,
                mode: m["colorManagementPreset"]
                    .as_str()
                    .unwrap_or("srgb")
                    .chars()
                    .take(16)
                    .collect(),
                hdr: details["hdr"].as_bool().unwrap_or(false),
                wide: details["bt2020"].as_bool().unwrap_or(false),
            })
        })
        .take(8)
        .collect()
}

/// Lua that gives each output its wanted preset, for outputs whose preset
/// differs (a change reconfigures the output).
#[must_use]
pub fn color_lua(preferences: &CompositorPreferences, support: &[ColorSupport]) -> String {
    let mut lua = String::new();
    for output in support {
        let wanted = preferences
            .color_modes
            .get(&output.name)
            .map_or("srgb", String::as_str);
        // Only what the display reports it can show; anything else is sRGB.
        let wanted = match wanted {
            "hdr" if output.hdr => "hdr",
            "wide" if output.wide || output.hdr => "wide",
            _ => "srgb",
        };
        let current = if output.mode == "auto" {
            "srgb"
        } else {
            output.mode.as_str()
        };
        if current != wanted {
            let depth = if wanted == "srgb" { 8 } else { 10 };
            let _ = writeln!(
                lua,
                "hl.monitor({{output = \"{}\", cm = \"{wanted}\", bitdepth = {depth}}})",
                output.name
            );
        }
    }
    lua
}

/// A program name or absolute path the compositor's shell can run unquoted.
#[must_use]
pub fn valid_terminal(program: &str) -> bool {
    program.is_empty()
        || (program.len() <= 255
            && !program.starts_with('-')
            && program.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'/' | b'_' | b'+' | b'-')
            }))
}

// What Super+Return starts: the chosen program, else the one the desktop
// was started with.
fn terminal(preferences: &CompositorPreferences) -> String {
    if !preferences.terminal.is_empty() {
        return preferences.terminal.clone();
    }
    static INSTALLED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTALLED
        .get_or_init(|| {
            std::env::var("ENZUI_TERMINAL")
                .ok()
                .filter(|t| !t.is_empty() && valid_terminal(t))
                .unwrap_or_else(|| "foot".into())
        })
        .clone()
}

fn animation_readback(value: &serde_json::Value, rate: u16) -> bool {
    if !(25..=200).contains(&rate) {
        return false;
    }
    let Some(leaves) = value.get(0).and_then(serde_json::Value::as_array) else {
        return false;
    };
    ["global", "workspaces"].into_iter().all(|name| {
        let matches: Vec<_> = leaves.iter().filter(|leaf| leaf["name"] == name).collect();
        matches.len() == 1
            && matches[0]["speed"]
                .as_f64()
                .is_some_and(|speed| (speed - 300.0 / f64::from(rate)).abs() <= 0.011)
            && matches[0]["enabled"] == true
            && matches[0]["bezier"] == "desktop-ease"
            && (name != "workspaces" || matches[0]["style"] == "slide")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_preferences_behaviour() {
        // Preference writes merge with the latest document.
        {
            use crate::settings::{Change, Store};
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("settings.json");
            std::fs::write(
                &path,
                r##"{"wallpaper":{"future":42},"compositor":{"future":17}}"##,
            )
            .unwrap();
            let first = Store::at(path.clone()).unwrap();
            let peer = Store::at(path.clone()).unwrap();
            let _old = first.load().unwrap();
            peer.update(Change::Wallpaper(WallpaperChange::Fit(WallpaperFit::Fit)))
                .unwrap();
            let next = first
                .update(Change::Wallpaper(WallpaperChange::Color("#223344".into())))
                .unwrap();
            assert_eq!(next.wallpaper.fit, WallpaperFit::Fit);
            assert_eq!(next.wallpaper.revision, 2);
            peer.update(Change::Compositor(OptionKey::OuterGap, 40))
                .unwrap();
            let next = first
                .update(Change::Compositor(OptionKey::InnerGap, 8))
                .unwrap();
            assert_eq!(next.compositor.gaps_outer, 40);
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(json["wallpaper"]["future"], 42);
            assert_eq!(json["compositor"]["future"], 17);
        }
        // Compositor values are bounded and failed changes are atomic.
        {
            let mut p = CompositorPreferences::default();
            for key in OptionKey::ALL {
                let before = p.clone();
                let (min, max) = key.bounds();
                assert!(p.set(key, min - 1).is_err());
                assert_eq!(p, before);
                assert!(p.set(key, max + 1).is_err());
                assert_eq!(p, before);
                p.set(key, max).unwrap();
                assert_eq!(p.value(key), max);
            }
            assert!(
                CompositorPreferences::from_json(Some(&serde_json::json!({"animations":3})))
                    .is_err()
            );
        }
        // Wallpaper rejects invalid colors and nonlocal paths.
        {
            let mut p = Wallpaper {
                color: "red".into(),
                ..Wallpaper::default()
            };
            assert!(p.validate().is_err());
            p.color = "#aabbcc".into();
            p.kind = WallpaperKind::Image;
            assert!(p.validate().is_err());
            p.image = Some("https://example.invalid/a.png".into());
            assert!(p.validate().is_err());
            p.image = Some("/owned/picture.png".into());
            assert!(p.validate().is_ok());
        }
        // Sabotage: return a fixed 12 from `CompositorPreferences::rounding`.
        {
            let mut p = CompositorPreferences::default();
            let theme = enzui_theme::text_defaults::window_radius().round() as i32;
            assert_eq!(p.value(OptionKey::Rounding), theme);
            let saved: CompositorPreferences = serde_json::from_str("{}").unwrap();
            assert_eq!(saved.value(OptionKey::Rounding), theme);
            p.set(OptionKey::Rounding, 7).unwrap();
            assert_eq!(p.value(OptionKey::Rounding), 7);
        }
    }
}
