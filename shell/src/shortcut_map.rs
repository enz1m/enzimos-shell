//! Account-editable compositor chords shared with config/shortcut-map.lua.

use enzui_core::{Modifier, NamedKey, Shortcut, ShortcutKey, ShortcutValidity};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
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
    "window-close", "Close window", "Windows", "SUPER + Q";
    "window-close-alternative", "Close window (second shortcut)", "Windows", "ALT + F4";
    "window-float", "Float or tile window", "Windows", "SUPER + V";
    "window-fullscreen", "Fullscreen", "Windows", "SUPER + F";
    "window-maximize", "Maximize", "Windows", "SUPER + M";
    "window-spread", "Show all windows", "Windows", "SUPER + W";
    "switch-next", "Switch windows", "Switching", "ALT + Tab";
    "switch-previous", "Switch windows backwards", "Switching", "ALT + SHIFT + Tab";
    "switch-app", "Switch windows of the app", "Switching", "ALT + grave";
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
    "lock", "Lock the screen", "System", "SUPER + L";
    "terminal", "Open Terminal", "System", "SUPER + Return";
    "notifications", "Show notifications", "System", "SUPER + N";
    "settings", "Open Settings", "System", "SUPER + comma";
    "emoji", "Emoji picker", "System", "SUPER + period";
    "dictation", "Dictate (hold, or tap for hands-free)", "System", "SUPER + H";
    "sidebar", "Show the sidebar", "System", "SUPER + SHIFT + B";
    "capture", "Screenshot or record", "Screenshots", "Print";
    "screenshot-screen", "Screenshot the screen", "Screenshots", "SHIFT + Print";
    "screenshot-window", "Screenshot a window", "Screenshots", "ALT + Print";
    "record", "Record the screen", "Screenshots", "CTRL + SHIFT + ALT + R";
    "screen-reader", "Screen reader", "Accessibility", "SUPER + ALT + S";
    "zoom-in", "Zoom in", "Accessibility", "SUPER + ALT + equal";
    "zoom-out", "Zoom out", "Accessibility", "SUPER + ALT + minus";
    "zoom-reset", "Reset zoom", "Accessibility", "SUPER + ALT + 8";
];

// Chords the desktop keeps for itself.
const FIXED: &[&str] = &[
    "ALT + Escape",
    "SUPER + SPACE",
    "CTRL + SPACE",
    "ALT + SPACE",
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
        _ => return function_key(key).map(|n| format!("F{n}")),
    };
    Some(name.to_owned())
}

fn function_key(key: NamedKey) -> Option<u8> {
    use NamedKey::*;
    [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12]
        .iter()
        .position(|f| *f == key)
        .map(|i| i as u8 + 1)
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
    } else if let Some((symbol, _)) = SYMBOLS.iter().find(|(_, name)| *name == key) {
        (*symbol).into()
    } else {
        let named = match key {
            "Return" => NamedKey::Enter,
            "Tab" => NamedKey::Tab,
            "SPACE" | "space" => NamedKey::Space,
            "Escape" => NamedKey::Escape,
            "BackSpace" => NamedKey::Backspace,
            "Delete" => NamedKey::Delete,
            "Insert" => NamedKey::Insert,
            "Home" => NamedKey::Home,
            "End" => NamedKey::End,
            "Prior" => NamedKey::PageUp,
            "Next" => NamedKey::PageDown,
            "Up" => NamedKey::ArrowUp,
            "Down" => NamedKey::ArrowDown,
            "Left" => NamedKey::ArrowLeft,
            "Right" => NamedKey::ArrowRight,
            "Print" => NamedKey::PrintScreen,
            _ => {
                use NamedKey::*;
                let number: usize = key.strip_prefix('F')?.parse().ok()?;
                *[F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12].get(number.checked_sub(1)?)?
            }
        };
        named.into()
    };
    Some(Shortcut::new(modifiers, key))
}

/// Validates bindable chords with modifiers, except Print and function keys; fixed desktop
/// chords are reserved.
#[must_use]
pub fn validity(shortcut: &Shortcut) -> ShortcutValidity {
    let Some(chord) = to_chord(shortcut) else {
        return ShortcutValidity::Invalid(
            tr("shell-this-key-cannot-be-used-for-a-shortcut").into(),
        );
    };
    let standalone = matches!(shortcut.key(), ShortcutKey::Named(key) if *key == NamedKey::PrintScreen || function_key(*key).is_some());
    let modifiers = shortcut.modifiers();
    if !standalone
        && !(modifiers.contains(Modifier::SUPER)
            || modifiers.contains(Modifier::CTRL)
            || modifiers.contains(Modifier::ALT))
    {
        return ShortcutValidity::Invalid(tr("shell-hold-super-ctrl-or-alt-with-the-key").into());
    }
    if FIXED
        .iter()
        .any(|fixed| from_chord(fixed).as_ref() == Some(shortcut))
    {
        return ShortcutValidity::Reserved(tr!("shell-is-kept-by-the-desktop", "arg0" => chord));
    }
    ShortcutValidity::Valid
}

/// Whether stored choices are well formed: known actions, valid chords, no
/// two actions sharing one.
#[must_use]
pub fn valid(choices: &BTreeMap<String, String>) -> bool {
    let mut taken = Vec::new();
    for action in ACTIONS {
        let chord = choices
            .get(action.id)
            .map_or(action.default, String::as_str);
        let Some(shortcut) = from_chord(chord) else {
            return false;
        };
        if taken.contains(&shortcut) {
            return false;
        }
        taken.push(shortcut);
    }
    choices.len() <= ACTIONS.len()
        && choices.iter().all(|(id, chord)| {
            ACTIONS.iter().any(|action| action.id == id)
                && from_chord(chord)
                    .is_some_and(|shortcut| validity(&shortcut) == ShortcutValidity::Valid)
        })
}

/// The compositor command that moves the bindings to `choices`.
#[must_use]
pub fn lua(choices: &BTreeMap<String, String>) -> String {
    let entries: Vec<String> = ACTIONS
        .iter()
        .filter_map(|action| {
            let chord = choices.get(action.id)?;
            Some(format!(
                "[{}]={}",
                serde_json::to_string(action.default).ok()?,
                serde_json::to_string(chord).ok()?
            ))
        })
        .collect();
    format!(
        "local map={{{}}}\nif enzui_set_shortcuts then return enzui_set_shortcuts(map) end\nreturn next(map) and \"missing\" or \"ok\"",
        entries.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_map_behaviour() {
        // Every default round trips and is distinct.
        {
            let mut seen = Vec::new();
            for action in ACTIONS {
                let shortcut =
                    from_chord(action.default).unwrap_or_else(|| panic!("{}", action.default));
                let chord = to_chord(&shortcut).unwrap();
                assert_eq!(
                    from_chord(&chord),
                    Some(shortcut.clone()),
                    "{}",
                    action.default
                );
                assert!(
                    !seen.contains(&shortcut),
                    "{} is bound twice",
                    action.default
                );
                seen.push(shortcut);
            }
            assert!(valid(&BTreeMap::new()));
        }
        // Choices need a modifier and stay unique.
        {
            let chord = |text: &str| from_chord(text).unwrap();
            assert_eq!(validity(&chord("SUPER + J")), ShortcutValidity::Valid);
            assert_eq!(validity(&chord("F5")), ShortcutValidity::Valid);
            assert!(matches!(
                validity(&chord("W")),
                ShortcutValidity::Invalid(_)
            ));
            assert!(matches!(
                validity(&chord("SHIFT + W")),
                ShortcutValidity::Invalid(_)
            ));
            assert!(matches!(
                validity(&chord("CTRL + SPACE")),
                ShortcutValidity::Reserved(_)
            ));
            let mut choices = BTreeMap::from([("window-close".to_owned(), "SUPER + J".to_owned())]);
            assert!(valid(&choices));
            assert!(lua(&choices).contains(r#"["SUPER + Q"]="SUPER + J""#));
            choices.insert("lock".into(), "SUPER + J".into());
            assert!(!valid(&choices), "two actions cannot share a chord");
            choices.remove("lock");
            choices.insert("window-float".into(), "SUPER + Q".into());
            assert!(
                valid(&choices),
                "a default freed by another choice may be reused"
            );
            assert!(!valid(&BTreeMap::from([(
                "unknown".to_owned(),
                "SUPER + J".to_owned()
            )])));
        }
    }
}
