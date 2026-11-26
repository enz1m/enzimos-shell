//! Account-editable compositor chords shared with config/shortcut-map.lua.

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
    pub name: &'static str,
    pub section: &'static str,
    pub default: &'static str,
}

macro_rules! actions {
    ($($id:literal, $name:literal, $section:literal, $default:literal;)*) => {
        &[$(Action { id: $id, name: $name, section: $section, default: $default }),*]
    };
}

pub const ACTIONS: &[Action] = actions![
    "search", "Open Quick Search", "Apps", "SUPER + SPACE";
    "launcher", "Open Applications", "Apps", "SUPER + A";
    "terminal", "Open Terminal", "Apps", "SUPER + Return";
    "terminal-alias", "Open Terminal (second shortcut)", "Apps", "SUPER + T";
    "shortcuts", "Show keyboard shortcuts", "System", "SUPER + slash";
    "controls", "Open Control Center", "System", "SUPER + C";
    "dock", "Turn the dock on or off", "System", "SUPER + SHIFT + D";
    "lock", "Lock the screen", "System", "SUPER + L";
    "notifications", "Show notifications", "System", "SUPER + N";
    "settings", "Open Settings", "System", "SUPER + comma";
    "emoji", "Emoji picker", "System", "SUPER + period";
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
    "switch-app", "Switch windows of the app", "Switching", "ALT + grave";
    "switch-cancel", "Cancel window switching", "Switching", "ALT + Escape";
    "overview", "Show desktops", "Desktops", "SUPER + D";
    "desktop-before", "Go to previous desktop", "Desktops", "SUPER + Prior";
    "desktop-after", "Go to next desktop", "Desktops", "SUPER + Next";
    "move-before", "Move window to previous desktop", "Desktops", "SUPER + SHIFT + Prior";
    "move-after", "Move window to next desktop", "Desktops", "SUPER + SHIFT + Next";
    "desktop-1", "Go to desktop 1", "Desktops", "SUPER + 1";
    "desktop-2", "Go to desktop 2", "Desktops", "SUPER + 2";
    "desktop-3", "Go to desktop 3", "Desktops", "SUPER + 3";
    "desktop-4", "Go to desktop 4", "Desktops", "SUPER + 4";
    "desktop-5", "Go to desktop 5", "Desktops", "SUPER + 5";
    "desktop-6", "Go to desktop 6", "Desktops", "SUPER + 6";
    "desktop-7", "Go to desktop 7", "Desktops", "SUPER + 7";
    "desktop-8", "Go to desktop 8", "Desktops", "SUPER + 8";
    "desktop-9", "Go to desktop 9", "Desktops", "SUPER + 9";
    "move-1", "Move window to desktop 1", "Desktops", "SUPER + SHIFT + 1";
    "move-2", "Move window to desktop 2", "Desktops", "SUPER + SHIFT + 2";
    "move-3", "Move window to desktop 3", "Desktops", "SUPER + SHIFT + 3";
    "move-4", "Move window to desktop 4", "Desktops", "SUPER + SHIFT + 4";
    "move-5", "Move window to desktop 5", "Desktops", "SUPER + SHIFT + 5";
    "move-6", "Move window to desktop 6", "Desktops", "SUPER + SHIFT + 6";
    "move-7", "Move window to desktop 7", "Desktops", "SUPER + SHIFT + 7";
    "move-8", "Move window to desktop 8", "Desktops", "SUPER + SHIFT + 8";
    "move-9", "Move window to desktop 9", "Desktops", "SUPER + SHIFT + 9";
    "capture", "Screenshot or record", "Screenshots", "Print";
    "screenshot-screen", "Screenshot the screen", "Screenshots", "SHIFT + Print";
    "screenshot-window", "Screenshot a window", "Screenshots", "ALT + Print";
    "record", "Record the screen", "Screenshots", "CTRL + SHIFT + ALT + R";
    "dictation", "Dictate (hold, or tap for hands-free)", "Accessibility", "SUPER + H";
    "screen-reader", "Screen reader", "Accessibility", "SUPER + ALT + S";
    "zoom-in", "Zoom in", "Accessibility", "SUPER + ALT + equal";
    "zoom-out", "Zoom out", "Accessibility", "SUPER + ALT + minus";
    "zoom-reset", "Reset zoom", "Accessibility", "SUPER + ALT + 8";
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
    let modifiers = shortcut.modifiers();
    let mut parts: Vec<String> = [
        (Modifier::SUPER, "SUPER"),
        (Modifier::CTRL, "CTRL"),
        (Modifier::SHIFT, "SHIFT"),
        (Modifier::ALT, "ALT"),
    ]
    .into_iter()
    .filter(|(modifier, _)| modifiers.contains(*modifier))
    .map(|(_, name)| name.to_owned())
    .collect();
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
    parts.push(key);
    Some(parts.join(" + "))
}

/// The shortcut a compositor chord names.
#[must_use]
pub fn from_chord(chord: &str) -> Option<Shortcut> {
    let mut parts: Vec<&str> = chord.split('+').map(str::trim).collect();
    let key = parts.pop().filter(|key| !key.is_empty())?;
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
    if chord == "SUPER + CTRL + SHIFT + ALT + F24" {
        return ShortcutValidity::Reserved(tr!("shell-is-kept-by-the-desktop", "arg0" => chord));
    }
    let standalone = matches!(shortcut.key(), ShortcutKey::Named(key)
        if *key == NamedKey::PrintScreen || function_key(*key).is_some()
            || MEDIA.iter().any(|(named, _)| named == key));
    let modifiers = shortcut.modifiers();
    if !standalone
        && !(modifiers.contains(Modifier::SUPER)
            || modifiers.contains(Modifier::CTRL)
            || modifiers.contains(Modifier::ALT))
    {
        return ShortcutValidity::Invalid(tr("shell-hold-super-ctrl-or-alt-with-the-key").into());
    }
    ShortcutValidity::Valid
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

pub(crate) fn preserve_existing_assignments(choices: &mut BTreeMap<String, String>) {
    let claimed: Vec<_> = choices
        .values()
        .filter_map(|chord| from_chord(chord))
        .collect();
    for id in NEW_DEFAULTS {
        if let Some(action) = action(id)
            && from_chord(action.default).is_some_and(|key| claimed.contains(&key))
        {
            choices.entry((*id).into()).or_default();
        }
    }
}

fn checked_chord(chord: &str) -> Result<Option<String>, String> {
    if chord.is_empty() {
        return Ok(None);
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
) -> Result<(), String> {
    if choices.keys().any(|id| action(id).is_none()) || custom.len() > MAX_CUSTOM {
        return Err(tr("shell-invalid-keyboard-shortcuts").into());
    }
    let mut taken = BTreeMap::new();
    for action in ACTIONS {
        let name = crate::i18n::label(action.name);
        if let Some(key) = checked_chord(chord(action, choices))?
            && let Some(other) = taken.insert(key.clone(), name)
        {
            return Err(
                tr!("shell-shortcuts-conflict", "shortcut" => badge(&key), "action" => other.to_owned()),
            );
        }
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
        if let Some(key) = checked_chord(&custom.chord)?
            && let Some(other) = taken.insert(key.clone(), custom.name.as_str())
        {
            return Err(
                tr!("shell-shortcuts-conflict", "shortcut" => badge(&key), "action" => other.to_owned()),
            );
        }
    }
    Ok(())
}

/// Formats a compositor chord using the current language and readable media labels.
pub fn badge(chord: &str) -> String {
    if chord.is_empty() {
        return tr("shell-shortcuts-unassigned").into();
    }
    let key = match chord {
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
            return from_chord(chord).map_or_else(|| chord.to_owned(), |key| key.localized_label());
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
) -> Result<Vec<Binding>, String> {
    let snapshot = compositor
        .request(&["--batch", &live_snapshot_lua()])
        .await?;
    let live: BTreeMap<_, _> = snapshot
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    Ok(bindings(preferences)
        .into_iter()
        .filter_map(|mut entry| {
            let key = action(&entry.id).map_or(entry.id.as_str(), |action| action.default);
            entry.chord = (*live.get(key)?).into();
            Some(entry)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_map_behaviour() {
        let mut choices = BTreeMap::new();
        let mut custom = BTreeMap::new();
        assert!(validate(&choices, &custom).is_ok());
        for action in ACTIONS {
            let key = from_chord(action.default).unwrap();
            assert_eq!(from_chord(&to_chord(&key).unwrap()), Some(key));
        }
        choices.insert("lock".into(), "super+J".into());
        custom.insert(
            "custom-a".into(),
            CustomShortcut {
                name: "Read notes".into(),
                chord: "SUPER + J".into(),
                target: CustomTarget::Command("printf '%s' 'lá'".into()),
            },
        );
        assert!(validate(&choices, &custom).is_err());
        choices.insert("lock".into(), String::new());
        assert!(validate(&choices, &custom).is_ok());
        assert!(lua(&choices, &custom).contains(&lua_string("printf '%s' 'lá'")));
        choices.insert("window-float".into(), "SUPER + slash".into());
        preserve_existing_assignments(&mut choices);
        assert_eq!(choices["shortcuts"], "");
        assert!(validate(&choices, &custom).is_ok());
    }
}
