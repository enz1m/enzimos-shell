use super::{logical_label, physical_chord, split_chord};
use crate::compositor::Compositor;
use enzui_core::Modifier;
use iced::keyboard::key::{Code, Physical};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use xkbcommon::xkb;

const POSITIONS: &[(Code, u32)] = {
    use Code::*;
    &[
        (Backquote, 49),
        (Digit1, 10),
        (Digit2, 11),
        (Digit3, 12),
        (Digit4, 13),
        (Digit5, 14),
        (Digit6, 15),
        (Digit7, 16),
        (Digit8, 17),
        (Digit9, 18),
        (Digit0, 19),
        (Minus, 20),
        (Equal, 21),
        (KeyQ, 24),
        (KeyW, 25),
        (KeyE, 26),
        (KeyR, 27),
        (KeyT, 28),
        (KeyY, 29),
        (KeyU, 30),
        (KeyI, 31),
        (KeyO, 32),
        (KeyP, 33),
        (BracketLeft, 34),
        (BracketRight, 35),
        (KeyA, 38),
        (KeyS, 39),
        (KeyD, 40),
        (KeyF, 41),
        (KeyG, 42),
        (KeyH, 43),
        (KeyJ, 44),
        (KeyK, 45),
        (KeyL, 46),
        (Semicolon, 47),
        (Quote, 48),
        (Backslash, 51),
        (KeyZ, 52),
        (KeyX, 53),
        (KeyC, 54),
        (KeyV, 55),
        (KeyB, 56),
        (KeyN, 57),
        (KeyM, 58),
        (Comma, 59),
        (Period, 60),
        (Slash, 61),
        (IntlBackslash, 94),
        (IntlRo, 97),
        (IntlYen, 132),
    ]
};

pub(super) fn known_code(code: u32) -> bool {
    POSITIONS.iter().any(|(_, keycode)| *keycode == code)
}

// Hyprland expects the XKB keycode (evdev code plus eight).
pub(super) fn position(key: Physical) -> Option<u32> {
    POSITIONS
        .iter()
        .find_map(|&(position, code)| (key == position).then_some(code))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    symbol: String,
    label: String,
}

/// Display labels for physical shortcuts in one keyboard layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyLabels(Arc<BTreeMap<u32, Key>>);

impl KeyLabels {
    fn compile(
        rules: &str,
        model: &str,
        layouts: &str,
        variants: &str,
        options: &str,
        group: u32,
    ) -> Self {
        if [rules, model, layouts, variants, options]
            .iter()
            .any(|s| s.contains('\0'))
        {
            return Self::default();
        }
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let Some(keymap) = xkb::Keymap::new_from_names(
            &context,
            rules,
            model,
            layouts,
            variants,
            Some(options.to_owned()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            return Self::default();
        };
        Self(Arc::new(
            POSITIONS
                .iter()
                .filter_map(|&(_, code)| {
                    let &[symbol] = keymap.key_get_syms_by_level(xkb::Keycode::new(code), group, 0)
                    else {
                        return None;
                    };
                    let name = xkb::keysym_get_name(symbol);
                    let label = match name.as_str() {
                        "dead_circumflex" => "^".into(),
                        "dead_grave" => "`".into(),
                        "dead_tilde" => "~".into(),
                        "dead_acute" => "´".into(),
                        "dead_diaeresis" => "¨".into(),
                        "dead_caron" => "ˇ".into(),
                        "dead_abovering" => "˚".into(),
                        _ => {
                            let text = xkb::keysym_to_utf8(symbol);
                            if text.is_empty() || text.chars().any(char::is_control) {
                                name.clone()
                            } else {
                                text
                            }
                        }
                    };
                    Some((
                        code,
                        Key {
                            symbol: name,
                            label,
                        },
                    ))
                })
                .collect(),
        ))
    }

    pub(crate) fn configured(layouts: &str, variants: &str, options: &str) -> Self {
        thread_local! {
            static CONFIGURED: RefCell<Option<([String; 3], KeyLabels)>> = const { RefCell::new(None) };
        }
        CONFIGURED.with_borrow_mut(|cached| {
            let spec = [layouts, variants, options];
            if let Some((previous, labels)) = cached
                && previous.each_ref().map(String::as_str) == spec
            {
                return labels.clone();
            }
            let labels = Self::compile("", "", layouts, variants, options, 0);
            *cached = Some((spec.map(str::to_owned), labels.clone()));
            labels
        })
    }

    fn from_devices(response: &str) -> Option<Self> {
        let response: serde_json::Value = serde_json::from_str(response).ok()?;
        let keyboards = response["keyboards"].as_array()?;
        let keyboard = keyboards
            .iter()
            .find(|k| k["main"].as_bool() == Some(true))
            .or_else(|| keyboards.first())?;
        let field = |name: &str| keyboard[name].as_str().unwrap_or_default();
        let group = keyboard["active_layout_index"].as_u64().unwrap_or(0);
        Some(Self::compile(
            field("rules"),
            field("model"),
            field("layout"),
            field("variant"),
            field("options"),
            u32::try_from(group).unwrap_or(0),
        ))
    }

    pub(super) async fn read(compositor: &Compositor) -> Self {
        compositor
            .request(&["-j", "devices"])
            .await
            .ok()
            .as_deref()
            .and_then(Self::from_devices)
            .unwrap_or_default()
    }

    /// Formats the keys in display order using the active keyboard layout.
    pub fn caps(&self, chord: &str) -> Vec<String> {
        let Some((modifiers, key)) = split_chord(chord) else {
            return vec![if chord.is_empty() {
                crate::i18n::tr("shell-shortcuts-unassigned").into()
            } else {
                chord.to_owned()
            }];
        };
        let label = if let Some((_, code)) = physical_chord(chord) {
            self.0.get(&code).map_or_else(
                || crate::tr!("shell-shortcut-key-code", "code" => code.to_string()),
                |key| key.label.to_ascii_uppercase(),
            )
        } else {
            logical_label(chord, key)
        };
        modifier_labels(modifiers).chain([label]).collect()
    }

    /// Formats the chord with its individual keys separated by plus signs.
    pub fn badge(&self, chord: &str) -> String {
        self.caps(chord).join(" + ")
    }

    pub(super) fn matches_symbol(&self, code: u32, symbol: &str) -> bool {
        self.0
            .get(&code)
            .is_some_and(|key| key.symbol.eq_ignore_ascii_case(symbol))
    }
}

/// Returns the localized labels for held modifiers in chord order.
pub fn modifier_labels(modifiers: Modifier) -> impl Iterator<Item = String> {
    [
        (Modifier::SUPER, "enzui-key-super"),
        (Modifier::CTRL, "enzui-key-control"),
        (Modifier::ALT, "enzui-key-alt"),
        (Modifier::SHIFT, "enzui-key-shift"),
    ]
    .into_iter()
    .filter(move |(flag, _)| modifiers.contains(*flag))
    .map(|(_, label)| enzui_core::localization::text(label))
}

/// Emits labels when the compositor connects, reloads or switches keyboard layouts.
pub fn subscription() -> iced::Subscription<KeyLabels> {
    use iced::futures::StreamExt;
    let Ok(compositor) = Compositor::from_environment() else {
        return iced::Subscription::none();
    };
    iced::Subscription::run_with(compositor.event_socket(), |socket| {
        crate::workspaces::keyboard_events(socket.clone()).then(|()| async {
            match Compositor::from_environment() {
                Ok(compositor) => KeyLabels::read(&compositor).await,
                Err(_) => KeyLabels::default(),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_layout_and_variant_label_physical_shortcuts() {
        for (group, labels) in [
            (0, ["/", ".", ",", "`"]),
            (1, ["/", ".", ",", "`"]),
            (2, ["-", ".", ",", "0"]),
        ] {
            let devices = serde_json::json!({"keyboards": [
                {"layout": "us", "main": false},
                {"layout": "us,gb,hu", "model": "pc105", "main": true, "active_layout_index": group},
            ]});
            let map = KeyLabels::from_devices(&devices.to_string()).unwrap();
            for (code, label) in [61, 60, 59, 49].into_iter().zip(labels) {
                assert!(map.badge(&format!("SUPER + code:{code}")).ends_with(label));
            }
        }
        let dvorak = KeyLabels::compile("", "", "us", "dvorak", "", 0);
        assert!(dvorak.badge("SUPER + code:61").ends_with("Z"));
        let german = KeyLabels::configured("de", "", "");
        assert!(german.badge("SUPER + ALT + code:20").ends_with("ß"));
        let czech = KeyLabels::configured("cz", "", "");
        crate::i18n::canonical(|| {
            assert_eq!(
                czech.caps("SHIFT + SUPER + ALT + CTRL + code:10"),
                ["Super", "Ctrl", "Alt", "Shift", "+"],
            );
        });
    }
}
