//! Account-editable compositor chords shared with config/shortcut-map.lua.

mod key_labels;

pub use key_labels::{KeyLabels, modifier_labels, subscription as key_labels_subscription};

use enzui_core::{Modifier, NamedKey, Shortcut, ShortcutKey, ShortcutValidity};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One changeable action: a stable id, its name and Settings group, and the
/// chord the desktop's configuration binds it to.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub id: &'static str,
    pub route: &'static str,
    pub name: &'static str,
    pub section: &'static str,
    pub default: &'static str,
}

macro_rules! actions {
    ($($id:literal, $name:literal, $section:literal, $default:literal;)*) => {
        &[$(Action { id: $id, route: concat!("keyboard/shortcut-", $id), name: $name, section: $section, default: $default }),*]
    };
}

pub const ACTIONS: &[Action] = actions![
    "search", "Open Quick Search", "Apps", "SUPER + SPACE";
    "launcher", "Open Applications", "Apps", "SUPER + A";
    "terminal", "Open Terminal", "Apps", "SUPER + Return";
    "terminal-alias", "Open Terminal (second shortcut)", "Apps", "SUPER + T";
    "shortcuts", "Show keyboard shortcuts", "System", "SUPER + code:61";
    "controls", "Open Control Center", "System", "SUPER + C";
    "dock", "Turn the dock on or off", "System", "SUPER + SHIFT + D";
    "lock", "Lock the screen", "System", "SUPER + L";
    "notifications", "Show notifications", "System", "SUPER + N";
    "settings", "Open Settings", "System", "SUPER + code:59";
    "emoji", "Emoji picker", "System", "SUPER + code:60";
    "writing-tools", "Writing Tools", "System", "SUPER + SHIFT + code:25";
    "sidebar", "Show the sidebar", "System", "SUPER + SHIFT + B";
    "brightness-up", "Increase brightness", "System", "XF86MonBrightnessUp";
    "brightness-down", "Decrease brightness", "System", "XF86MonBrightnessDown";
    "window-close", "Close window", "Windows", "SUPER + Q";
    "window-close-alternative", "Close window (second shortcut)", "Windows", "ALT + F4";
    "window-float", "Float or tile window", "Windows", "SUPER + V";
    "window-fullscreen", "Fullscreen", "Windows", "SUPER + F";
    "window-maximize", "Maximize", "Windows", "SUPER + M";
    "window-spread", "Show all windows", "Windows", "SUPER + W";
    "switch-next", "Switch windows", "Switching", "ALT + Tab";
    "switch-previous", "Switch windows backwards", "Switching", "ALT + SHIFT + Tab";
    "switch-app", "Switch windows of the app", "Switching", "ALT + code:49";
    "switch-cancel", "Cancel window switching", "Switching", "ALT + Escape";
    "overview", "Show desktops", "Desktops", "SUPER + D";
    "desktop-layout", "Switch desktop window layout", "Desktops", "SUPER + SHIFT + V";
    "desktop-before", "Go to previous desktop", "Desktops", "SUPER + Prior";
    "desktop-after", "Go to next desktop", "Desktops", "SUPER + Next";
    "move-before", "Move window to previous desktop", "Desktops", "SUPER + SHIFT + Prior";
    "move-after", "Move window to next desktop", "Desktops", "SUPER + SHIFT + Next";
    "desktop-1", "Go to desktop 1", "Desktops", "SUPER + code:10";
    "desktop-2", "Go to desktop 2", "Desktops", "SUPER + code:11";
    "desktop-3", "Go to desktop 3", "Desktops", "SUPER + code:12";
    "desktop-4", "Go to desktop 4", "Desktops", "SUPER + code:13";
    "desktop-5", "Go to desktop 5", "Desktops", "SUPER + code:14";
    "desktop-6", "Go to desktop 6", "Desktops", "SUPER + code:15";
    "desktop-7", "Go to desktop 7", "Desktops", "SUPER + code:16";
    "desktop-8", "Go to desktop 8", "Desktops", "SUPER + code:17";
    "desktop-9", "Go to desktop 9", "Desktops", "SUPER + code:18";
    "move-1", "Move window to desktop 1", "Desktops", "SUPER + SHIFT + code:10";
    "move-2", "Move window to desktop 2", "Desktops", "SUPER + SHIFT + code:11";
    "move-3", "Move window to desktop 3", "Desktops", "SUPER + SHIFT + code:12";
    "move-4", "Move window to desktop 4", "Desktops", "SUPER + SHIFT + code:13";
    "move-5", "Move window to desktop 5", "Desktops", "SUPER + SHIFT + code:14";
    "move-6", "Move window to desktop 6", "Desktops", "SUPER + SHIFT + code:15";
    "move-7", "Move window to desktop 7", "Desktops", "SUPER + SHIFT + code:16";
    "move-8", "Move window to desktop 8", "Desktops", "SUPER + SHIFT + code:17";
    "move-9", "Move window to desktop 9", "Desktops", "SUPER + SHIFT + code:18";
    "capture", "Screenshot or record", "Screenshots", "Print";
    "screenshot-screen", "Screenshot the screen", "Screenshots", "SHIFT + Print";
    "screenshot-window", "Screenshot a window", "Screenshots", "ALT + Print";
    "record", "Record the screen", "Screenshots", "CTRL + ALT + SHIFT + R";
    "dictation", "Dictate (hold, or tap for hands-free)", "Accessibility", "SUPER + H";
    "screen-reader", "Screen reader", "Accessibility", "SUPER + ALT + S";
    "zoom-in", "Zoom in", "Accessibility", "SUPER + ALT + code:21";
    "zoom-out", "Zoom out", "Accessibility", "SUPER + ALT + code:20";
    "zoom-reset", "Reset zoom", "Accessibility", "SUPER + ALT + code:17";
    "keyboard", "Show the on-screen keyboard", "Accessibility", "SUPER + ALT + K";
    "volume-up", "Raise volume", "Sound and Media", "XF86AudioRaiseVolume";
    "volume-down", "Lower volume", "Sound and Media", "XF86AudioLowerVolume";
    "volume-mute", "Mute sound", "Sound and Media", "XF86AudioMute";
    "microphone-mute", "Mute microphone", "Sound and Media", "XF86AudioMicMute";
    "media-play", "Play or pause media", "Sound and Media", "XF86AudioPlay";
    "media-pause", "Play or pause media (second shortcut)", "Sound and Media", "XF86AudioPause";
    "media-next", "Next track", "Sound and Media", "XF86AudioNext";
    "media-previous", "Previous track", "Sound and Media", "XF86AudioPrev";
    "media-stop", "Stop media", "Sound and Media", "XF86AudioStop";
];

const NEW_DEFAULTS: &[&str] = &[
    "launcher",
    "terminal-alias",
    "shortcuts",
    "controls",
    "dock",
    "overview",
    "desktop-layout",
    "desktop-before",
    "desktop-after",
    "move-before",
    "move-after",
    "keyboard",
];

const MEDIA: &[(NamedKey, &str)] = &[
    (NamedKey::AudioVolumeUp, "XF86AudioRaiseVolume"),
    (NamedKey::AudioVolumeDown, "XF86AudioLowerVolume"),
    (NamedKey::AudioVolumeMute, "XF86AudioMute"),
    (NamedKey::MicrophoneVolumeMute, "XF86AudioMicMute"),
    (NamedKey::BrightnessUp, "XF86MonBrightnessUp"),
    (NamedKey::BrightnessDown, "XF86MonBrightnessDown"),
    (NamedKey::MediaPlay, "XF86AudioPlay"),
    (NamedKey::MediaPause, "XF86AudioPause"),
    (NamedKey::MediaTrackNext, "XF86AudioNext"),
    (NamedKey::MediaTrackPrevious, "XF86AudioPrev"),
    (NamedKey::MediaStop, "XF86AudioStop"),
];

const SYMBOLS: &[(&str, &str)] = &[
    ("`", "grave"),
    (".", "period"),
    (",", "comma"),
    ("=", "equal"),
    ("-", "minus"),
    ("/", "slash"),
    (";", "semicolon"),
    ("'", "apostrophe"),
    ("[", "bracketleft"),
    ("]", "bracketright"),
    ("\\", "backslash"),
];

fn named(key: NamedKey) -> Option<String> {
    let name = match key {
        NamedKey::Enter => "Return",
        NamedKey::Tab => "Tab",
        NamedKey::Space => "SPACE",
        NamedKey::Escape => "Escape",
        NamedKey::Backspace => "BackSpace",
        NamedKey::Delete => "Delete",
        NamedKey::Insert => "Insert",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "Prior",
        NamedKey::PageDown => "Next",
        NamedKey::ArrowUp => "Up",
        NamedKey::ArrowDown => "Down",
        NamedKey::ArrowLeft => "Left",
        NamedKey::ArrowRight => "Right",
        NamedKey::PrintScreen => "Print",
        _ => {
            return MEDIA
                .iter()
                .find(|(named, _)| *named == key)
                .map(|(_, chord)| (*chord).to_owned())
                .or_else(|| function_key(key).map(|n| format!("F{n}")));
        }
    };
    Some(name.to_owned())
}

const FUNCTION_KEYS: [NamedKey; 24] = {
    use NamedKey::*;
    [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
        F21, F22, F23, F24,
    ]
};

fn function_key(key: NamedKey) -> Option<usize> {
    FUNCTION_KEYS.iter().position(|f| *f == key).map(|i| i + 1)
}

/// The compositor chord for `shortcut`, or `None` for a key it cannot name.
#[must_use]
pub fn to_chord(shortcut: &Shortcut) -> Option<String> {
    let key = match shortcut.key() {
        ShortcutKey::Named(key) => named(*key)?,
        ShortcutKey::Character(text) => {
            let mut chars = text.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase().to_string()
            } else {
                SYMBOLS
                    .iter()
                    .find(|(symbol, _)| symbol.chars().eq([c]))?
                    .1
                    .to_owned()
            }
        }
    };
    Some(format_chord(shortcut.modifiers(), &key))
}

fn format_chord(modifiers: Modifier, key: &str) -> String {
    [
        (Modifier::SUPER, "SUPER"),
        (Modifier::CTRL, "CTRL"),
        (Modifier::ALT, "ALT"),
        (Modifier::SHIFT, "SHIFT"),
    ]
    .into_iter()
    .filter_map(|(modifier, name)| modifiers.contains(modifier).then_some(name))
    .chain([key])
    .collect::<Vec<_>>()
    .join(" + ")
}

fn split_chord(chord: &str) -> Option<(Modifier, &str)> {
    let mut parts = chord.rsplit('+').map(str::trim);
    let key = parts.next().filter(|key| !key.is_empty())?;
    let mut modifiers = Modifier::NONE;
    for part in parts {
        modifiers |= match part.to_ascii_uppercase().as_str() {
            "SUPER" => Modifier::SUPER,
            "CTRL" | "CONTROL" => Modifier::CTRL,
            "SHIFT" => Modifier::SHIFT,
            "ALT" => Modifier::ALT,
            _ => return None,
        };
    }
    Some((modifiers, key))
}

fn physical_chord(chord: &str) -> Option<(Modifier, u32)> {
    let (modifiers, key) = split_chord(chord)?;
    let code = key.strip_prefix("code:")?.parse().ok()?;
    key_labels::known_code(code).then_some((modifiers, code))
}

/// The logical shortcut a compositor chord names.
#[must_use]
pub fn from_chord(chord: &str) -> Option<Shortcut> {
    let (modifiers, key) = split_chord(chord)?;
    let key: ShortcutKey = if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()) {
        key.to_ascii_lowercase().as_str().into()
    } else if let Some((symbol, _)) = SYMBOLS
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(key))
    {
        (*symbol).into()
    } else {
        let named = if let Some((named, _)) = MEDIA
            .iter()
            .find(|(_, chord)| chord.eq_ignore_ascii_case(key))
        {
            *named
        } else {
            match key.to_ascii_lowercase().as_str() {
                "return" => NamedKey::Enter,
                "tab" => NamedKey::Tab,
                "space" => NamedKey::Space,
                "escape" => NamedKey::Escape,
                "backspace" => NamedKey::Backspace,
                "delete" => NamedKey::Delete,
                "insert" => NamedKey::Insert,
                "home" => NamedKey::Home,
                "end" => NamedKey::End,
                "prior" => NamedKey::PageUp,
                "next" => NamedKey::PageDown,
                "up" => NamedKey::ArrowUp,
                "down" => NamedKey::ArrowDown,
                "left" => NamedKey::ArrowLeft,
                "right" => NamedKey::ArrowRight,
                "print" => NamedKey::PrintScreen,
                _ => {
                    let number: usize = key
                        .get(1..)
                        .filter(|_| key.starts_with(['f', 'F']))?
                        .parse()
                        .ok()?;
                    *FUNCTION_KEYS.get(number.checked_sub(1)?)?
                }
            }
        };
        named.into()
    };
    Some(Shortcut::new(modifiers, key))
}

/// Checks whether a key combination can be registered as a global shortcut.
#[must_use]
pub fn validity(shortcut: &Shortcut) -> ShortcutValidity {
    let Some(chord) = to_chord(shortcut) else {
        return ShortcutValidity::Invalid(
            tr("shell-this-key-cannot-be-used-for-a-shortcut").into(),
        );
    };
    if chord == "SUPER + CTRL + ALT + SHIFT + F24" {
        return ShortcutValidity::Reserved(tr!("shell-is-kept-by-the-desktop", "arg0" => chord));
    }
    let standalone = matches!(shortcut.key(), ShortcutKey::Named(key)
        if *key == NamedKey::PrintScreen || function_key(*key).is_some()
            || MEDIA.iter().any(|(named, _)| named == key));
    let modifiers = shortcut.modifiers();
    if !standalone && !has_shortcut_modifier(modifiers) {
        return ShortcutValidity::Invalid(tr("shell-hold-super-ctrl-or-alt-with-the-key").into());
    }
    ShortcutValidity::Valid
}

fn has_shortcut_modifier(modifiers: Modifier) -> bool {
    modifiers.contains(Modifier::SUPER)
        || modifiers.contains(Modifier::CTRL)
        || modifiers.contains(Modifier::ALT)
}

/// Encodes a key press by position for printable keys and by name for other keys.
pub fn capture_chord(
    key: &iced::keyboard::Key,
    physical: iced::keyboard::key::Physical,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Result<String, String>> {
    use enzui_widgets::key_capture_chip::{CaptureAction, capture_action};
    if let Some(code) = key_labels::position(physical) {
        let modifiers = Modifier::from_iced(modifiers);
        return Some(if has_shortcut_modifier(modifiers) {
            Ok(format_chord(modifiers, &format!("code:{code}")))
        } else {
            Err(tr("shell-hold-super-ctrl-or-alt-with-the-key").into())
        });
    }
    let CaptureAction::Candidate(shortcut) = capture_action(key, modifiers, false) else {
        return None;
    };
    Some(match validity(&shortcut) {
        ShortcutValidity::Valid => Ok(to_chord(&shortcut)?),
        ShortcutValidity::Invalid(reason) | ShortcutValidity::Reserved(reason) => Err(reason),
    })
}

/// A user-defined keyboard shortcut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomShortcut {
    pub name: String,
    pub chord: String,
    pub target: CustomTarget,
}

/// The operation run when a custom shortcut is pressed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomTarget {
    Command(String),
    Action(String),
}

/// Shell actions that can also be assigned to custom shortcuts.
pub const CUSTOM_ACTIONS: &[(&str, crate::commands::Command)] = &[
    ("search", crate::commands::Command::Search),
    ("launcher", crate::commands::Command::Launcher),
    ("shortcuts", crate::commands::Command::Shortcuts),
    ("controls", crate::commands::Command::Controls),
    ("overview", crate::commands::Command::Overview),
    ("notifications", crate::commands::Command::Notifications),
    ("capture", crate::commands::Command::Capture),
    ("lock", crate::commands::Command::Lock),
];

pub const MAX_CUSTOM: usize = 24;

pub fn action(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.id == id)
}

pub fn chord<'a>(action: &'a Action, choices: &'a BTreeMap<String, String>) -> &'a str {
    choices
        .get(action.id)
        .map_or(action.default, String::as_str)
}

/// A shortcut entry for the dialog or onboarding.
#[derive(Clone, Debug)]
pub struct Binding {
    pub id: String,
    pub name: String,
    pub section: String,
    pub chord: String,
}

/// Returns the account's shortcut catalog in display order. An empty chord is disabled.
pub fn bindings(preferences: &crate::desktop_preferences::CompositorPreferences) -> Vec<Binding> {
    ACTIONS
        .iter()
        .map(|action| Binding {
            id: action.id.into(),
            name: crate::i18n::label(action.name).into(),
            section: crate::i18n::label(action.section).into(),
            chord: chord(action, &preferences.shortcuts).into(),
        })
        .chain(
            preferences
                .custom_shortcuts
                .iter()
                .map(|(id, shortcut)| Binding {
                    id: id.clone(),
                    name: shortcut.name.clone(),
                    section: tr("shell-shortcuts-custom").into(),
                    chord: shortcut.chord.clone(),
                }),
        )
        .collect()
}

/// Returns frequently used desktop bindings, carrying the account's selected chords.
pub fn important(preferences: &crate::desktop_preferences::CompositorPreferences) -> Vec<Binding> {
    let bindings = bindings(preferences);
    [
        "search",
        "launcher",
        "switch-next",
        "overview",
        "desktop-layout",
        "capture",
        "lock",
        "shortcuts",
    ]
    .iter()
    .filter_map(|id| {
        bindings
            .iter()
            .find(|entry| &entry.id == id && !entry.chord.is_empty())
            .cloned()
    })
    .collect()
}

pub(crate) fn preserve_existing_assignments(
    choices: &mut BTreeMap<String, String>,
    custom: &BTreeMap<String, CustomShortcut>,
    keys: &KeyLabels,
) {
    let claimed: Vec<_> = choices
        .values()
        .map(String::as_str)
        .chain(custom.values().map(|shortcut| shortcut.chord.as_str()))
        .filter_map(|chord| checked_chord(chord).ok().flatten())
        .collect();
    for action in ACTIONS {
        if (NEW_DEFAULTS.contains(&action.id) || physical_chord(action.default).is_some())
            && claimed
                .iter()
                .any(|chord| overlaps(chord, action.default, keys))
        {
            choices.entry(action.id.into()).or_default();
        }
    }
}

fn overlaps(a: &str, b: &str, keys: &KeyLabels) -> bool {
    if a == b {
        return true;
    }
    let Some(((mods_a, key_a), (mods_b, key_b))) = a.rsplit_once(" + ").zip(b.rsplit_once(" + "))
    else {
        return false;
    };
    if mods_a != mods_b {
        return false;
    }
    match (key_a.strip_prefix("code:"), key_b.strip_prefix("code:")) {
        (Some(code), None) => code
            .parse()
            .is_ok_and(|code| keys.matches_symbol(code, key_b)),
        (None, Some(code)) => code
            .parse()
            .is_ok_and(|code| keys.matches_symbol(code, key_a)),
        _ => false,
    }
}

fn checked_chord(chord: &str) -> Result<Option<String>, String> {
    if chord.is_empty() {
        return Ok(None);
    }
    if let Some((modifiers, code)) = physical_chord(chord) {
        return if has_shortcut_modifier(modifiers) {
            Ok(Some(format_chord(modifiers, &format!("code:{code}"))))
        } else {
            Err(tr("shell-hold-super-ctrl-or-alt-with-the-key").into())
        };
    }
    let shortcut =
        from_chord(chord).ok_or_else(|| tr("shell-this-key-cannot-be-used-for-a-shortcut"))?;
    match validity(&shortcut) {
        ShortcutValidity::Valid => Ok(to_chord(&shortcut)),
        ShortcutValidity::Invalid(reason) | ShortcutValidity::Reserved(reason) => Err(reason),
    }
}

pub(crate) fn validate(
    choices: &BTreeMap<String, String>,
    custom: &BTreeMap<String, CustomShortcut>,
    keys: &KeyLabels,
    edited: Option<&str>,
) -> Result<(), String> {
    if choices.keys().any(|id| action(id).is_none()) || custom.len() > MAX_CUSTOM {
        return Err(tr("shell-invalid-keyboard-shortcuts").into());
    }
    let mut taken = Vec::<(String, &str, &str)>::new();
    let mut claim = |id, chord, name| {
        if let Some(key) = checked_chord(chord)? {
            if let Some((prior_key, prior_id, prior_name)) = taken
                .iter()
                .find(|(other, _, _)| overlaps(other, &key, keys))
            {
                let (requested, assigned) = if Some(*prior_id) == edited {
                    (prior_key, name)
                } else {
                    (&key, *prior_name)
                };
                return Err(tr!("shell-shortcuts-conflict",
                    "shortcut" => keys.badge(requested), "action" => assigned.to_owned()));
            }
            taken.push((key, id, name));
        }
        Ok(())
    };
    for action in ACTIONS {
        claim(
            action.id,
            chord(action, choices),
            crate::i18n::label(action.name),
        )?;
    }
    for (id, custom) in custom {
        let valid_text = |value: &str, max: usize| {
            !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
        };
        if !id.strip_prefix("custom-").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.len() <= 32
                && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        }) || !valid_text(&custom.name, 96)
            || !match &custom.target {
                CustomTarget::Command(command) => valid_text(command, 512),
                CustomTarget::Action(id) => CUSTOM_ACTIONS.iter().any(|(action, _)| action == id),
            }
        {
            return Err(tr("shell-shortcuts-custom-invalid").into());
        }
        claim(id.as_str(), &custom.chord, custom.name.as_str())?;
    }
    Ok(())
}

fn logical_label(chord: &str, key: &str) -> String {
    let key = match key {
        "XF86MonBrightnessUp" => "shell-shortcut-key-brightness-up",
        "XF86MonBrightnessDown" => "shell-shortcut-key-brightness-down",
        "XF86AudioRaiseVolume" => "shell-shortcut-key-volume-up",
        "XF86AudioLowerVolume" => "shell-shortcut-key-volume-down",
        "XF86AudioMute" => "shell-shortcut-key-mute",
        "XF86AudioMicMute" => "shell-shortcut-key-mic-mute",
        "XF86AudioPlay" => "shell-shortcut-key-play",
        "XF86AudioPause" => "shell-shortcut-key-pause",
        "XF86AudioNext" => "shell-next-track",
        "XF86AudioPrev" => "shell-previous-track",
        "XF86AudioStop" => "shell-shortcut-key-stop",
        _ => {
            return from_chord(chord).map_or_else(
                || key.to_owned(),
                |shortcut| Shortcut::new(Modifier::NONE, shortcut.key().clone()).localized_label(),
            );
        }
    };
    tr(key).into()
}

fn lua_string(text: &str) -> String {
    serde_json::to_string(text).expect("serializing a string cannot fail")
}

pub(crate) fn lua(
    choices: &BTreeMap<String, String>,
    custom: &BTreeMap<String, CustomShortcut>,
) -> String {
    let map = choices
        .iter()
        .filter_map(|(id, selected)| {
            let default = action(id)?.default;
            Some(format!(
                "[{}]={}",
                lua_string(default),
                lua_string(selected)
            ))
        })
        .collect::<Vec<_>>()
        .join(",");
    let custom = custom
        .iter()
        .filter_map(|(id, shortcut)| {
            if shortcut.chord.is_empty() {
                return None;
            }
            let target = match &shortcut.target {
                CustomTarget::Command(command) => format!("command={}", lua_string(command)),
                CustomTarget::Action(id) => {
                    let (_, command) = CUSTOM_ACTIONS.iter().find(|(action, _)| action == id)?;
                    format!("event={}", lua_string(command.name()))
                }
            };
            Some(format!(
                "{{id={},chord={},{} }}",
                lua_string(id),
                lua_string(&shortcut.chord),
                target
            ))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"local map={{{map}}}
local custom={{{custom}}}
if enzui_set_shortcuts then return enzui_set_shortcuts(map,custom) end
if next(map) or next(custom) then error({}) end"#,
        lua_string(tr(
            "shell-changed-shortcuts-need-the-current-desktop-configuration"
        ))
    )
}

pub(crate) fn live_snapshot_lua() -> String {
    let keys = ACTIONS
        .iter()
        .map(|action| lua_string(action.default))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"repl return enzui_shortcut_snapshot and enzui_shortcut_snapshot({{{keys}}}) or """#)
}

/// Resolves the currently registered bindings from this desktop session.
pub async fn live_bindings(
    compositor: &crate::compositor::Compositor,
    preferences: &crate::desktop_preferences::CompositorPreferences,
) -> Result<(Vec<Binding>, KeyLabels), String> {
    let query = live_snapshot_lua();
    let args = ["--batch", query.as_str()];
    let (snapshot, keys) = tokio::join!(compositor.request(&args), KeyLabels::read(compositor),);
    let snapshot = snapshot?;
    let live: BTreeMap<_, _> = snapshot
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    let bindings = bindings(preferences)
        .into_iter()
        .filter_map(|mut entry| {
            let key = action(&entry.id).map_or(entry.id.as_str(), |action| action.default);
            entry.chord = (*live.get(key)?).into();
            Some(entry)
        })
        .collect();
    Ok((bindings, keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hungarian_shift_slash_keeps_the_number_row_position_and_shift() {
        use iced::keyboard::{Key, Modifiers, key::Code};

        let chord = capture_chord(
            &Key::Character("6".into()),
            iced::keyboard::key::Physical::Code(Code::Digit6),
            Modifiers::LOGO | Modifiers::SHIFT,
        )
        .unwrap()
        .unwrap();
        assert_eq!(chord, action("move-6").unwrap().default);

        let keys = KeyLabels::configured("hu", "", "");
        assert!(overlaps(
            "SUPER + minus",
            action("shortcuts").unwrap().default,
            &keys
        ));
        assert!(!overlaps(
            "SUPER + SHIFT + minus",
            action("shortcuts").unwrap().default,
            &keys
        ));
        assert!(!overlaps(
            "SUPER + slash",
            action("shortcuts").unwrap().default,
            &keys
        ));
    }

    #[test]
    fn shortcut_map_behaviour() {
        let keys = KeyLabels::configured("us", "", "");
        let mut choices = BTreeMap::new();
        let mut custom = BTreeMap::new();
        assert!(validate(&choices, &custom, &keys, None).is_ok());
        for action in ACTIONS {
            assert_eq!(
                checked_chord(action.default).unwrap().as_deref(),
                Some(action.default)
            );
        }
        choices.insert("search".into(), "SUPER + code:61".into());
        let conflict = validate(&choices, &custom, &keys, Some("search")).unwrap_err();
        assert!(conflict.contains(crate::i18n::label(action("shortcuts").unwrap().name)));
        choices.remove("search");
        choices.insert("lock".into(), "super+J".into());
        custom.insert(
            "custom-a".into(),
            CustomShortcut {
                name: "Read notes".into(),
                chord: "SUPER + J".into(),
                target: CustomTarget::Command("printf '%s' 'lá'".into()),
            },
        );
        assert!(validate(&choices, &custom, &keys, None).is_err());
        choices.insert("lock".into(), String::new());
        assert!(validate(&choices, &custom, &keys, None).is_ok());
        assert!(lua(&choices, &custom).contains(&lua_string("printf '%s' 'lá'")));
        choices.insert("window-float".into(), "SUPER + code:61".into());
        custom.get_mut("custom-a").unwrap().chord = "SUPER + SHIFT + V".into();
        preserve_existing_assignments(&mut choices, &custom, &keys);
        assert_eq!(choices["desktop-layout"], "");
        assert_eq!(choices["shortcuts"], "");
        assert!(validate(&choices, &custom, &keys, None).is_ok());
    }
}
