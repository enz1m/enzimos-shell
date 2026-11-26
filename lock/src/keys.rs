//! Keyboard input from the compositor's keymap, through libxkbcommon.

#![allow(
    unsafe_code,
    reason = "libxkbcommon uses owned opaque pointers behind its C ABI."
)]

use std::ffi::{CStr, CString, c_char};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt;
use std::ptr::NonNull;
use xkbcommon_dl::{
    XkbCommon, XkbCommonCompose, keysyms, xkb_compose_compile_flags, xkb_compose_state,
    xkb_compose_state_flags, xkb_compose_status, xkb_context, xkb_context_flags, xkb_keymap,
    xkb_keymap_compile_flags, xkb_keymap_format, xkb_keysym_t, xkb_state, xkb_state_component,
};

/// What a key press means to the lock screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Text(String),
    Backspace,
    DeleteWord,
    Enter,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Other,
}

pub(crate) struct Keyboard {
    xkb: &'static XkbCommon,
    context: *mut xkb_context,
    keymap: *mut xkb_keymap,
    state: *mut xkb_state,
    compose: Option<Compose>,
    group: u32,
}

impl Keyboard {
    pub(crate) fn new() -> Option<Self> {
        let xkb = xkbcommon_dl::xkbcommon_option()?;
        // SAFETY: the loaded XKB function table is live and context creation has no pointer arguments.
        let context = unsafe { (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS) };
        if context.is_null() {
            return None;
        }
        Some(Self {
            xkb,
            context,
            keymap: std::ptr::null_mut(),
            state: std::ptr::null_mut(),
            compose: Compose::new(context, &compose_locale()),
            group: 0,
        })
    }
    // Loads the keymap the compositor sent as a file descriptor.
    pub(crate) fn load(&mut self, fd: OwnedFd, size: u32) -> bool {
        // The keymap fd may be shared, so read from its start regardless of
        // its file offset.
        use std::os::unix::fs::FileExt;
        let mut text = vec![0u8; (size as usize).min(4 << 20)];
        if std::fs::File::from(fd).read_exact_at(&mut text, 0).is_err() {
            return false;
        }
        if let Some(end) = text.iter().position(|&b| b == 0) {
            text.truncate(end);
        }
        if text.last() != Some(&0) {
            text.push(0);
        }
        // SAFETY: context is live and text is NUL-terminated; XKB copies it during this call.
        let keymap = unsafe {
            (self.xkb.xkb_keymap_new_from_string)(
                self.context,
                text.as_ptr() as *const c_char,
                xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        if keymap.is_null() {
            return false;
        }
        // SAFETY: keymap is a successfully created, live XKB keymap.
        let state = unsafe { (self.xkb.xkb_state_new)(keymap) };
        if state.is_null() {
            // SAFETY: state creation failed; this releases the sole newly acquired keymap reference.
            unsafe { (self.xkb.xkb_keymap_unref)(keymap) };
            return false;
        }
        self.release();
        self.keymap = keymap;
        self.state = state;
        self.reset_compose();
        true
    }
    pub(crate) fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if self.group != group {
            self.group = group;
            self.reset_compose();
        }
        if !self.state.is_null() {
            // SAFETY: state is non-null and exclusively owned by this Keyboard.
            unsafe {
                (self.xkb.xkb_state_update_mask)(
                    self.state, depressed, latched, locked, 0, 0, group,
                )
            };
        }
    }
    fn active(&self, name: &[u8]) -> bool {
        !self.state.is_null()
            // SAFETY: state is live and callers supply static NUL-terminated XKB modifier names.
            && unsafe {
                (self.xkb.xkb_state_mod_name_is_active)(self.state, name.as_ptr() as *const c_char, xkb_state_component::XKB_STATE_MODS_EFFECTIVE)
            } > 0
    }
    pub(crate) fn caps_lock(&self) -> bool {
        self.active(xkbcommon_dl::XKB_MOD_NAME_CAPS)
    }
    pub(crate) fn reset_compose(&mut self) {
        if let Some(compose) = &mut self.compose {
            compose.reset();
        }
    }

    pub(crate) fn press(&mut self, code: u32) -> (Key, bool) {
        if let Some(sym) = self.symbol(code) {
            if self.active(xkbcommon_dl::XKB_MOD_NAME_CTRL)
                || self.active(xkbcommon_dl::XKB_MOD_NAME_ALT)
                || self.active(xkbcommon_dl::XKB_MOD_NAME_LOGO)
            {
                self.reset_compose();
            } else if let Some(compose) = &mut self.compose
                && let Some(key) = compose.feed(sym)
            {
                // Repeating the physical key would emit the uncomposed character.
                return (key, false);
            }
        }
        (self.key(code), self.repeats(code))
    }

    fn symbol(&self, code: u32) -> Option<xkb_keysym_t> {
        let code = code.checked_add(8)?;
        if self.state.is_null() {
            return None;
        }
        // SAFETY: state is live; unknown keycodes resolve to NoSymbol.
        Some(unsafe { (self.xkb.xkb_state_key_get_one_sym)(self.state, code) })
    }

    // Interprets a Wayland key code (evdev code; xkb adds 8).
    pub(crate) fn key(&self, code: u32) -> Key {
        let Some(sym) = self.symbol(code) else {
            return Key::Other;
        };
        let ctrl = self.active(xkbcommon_dl::XKB_MOD_NAME_CTRL);
        match sym {
            keysyms::Return | keysyms::KP_Enter | keysyms::ISO_Enter => return Key::Enter,
            keysyms::BackSpace if ctrl => return Key::DeleteWord,
            keysyms::BackSpace => return Key::Backspace,
            keysyms::Escape => return Key::Escape,
            keysyms::u | keysyms::U if ctrl => return Key::Escape,
            keysyms::Up | keysyms::KP_Up => return Key::Up,
            keysyms::Down | keysyms::KP_Down => return Key::Down,
            keysyms::Left | keysyms::KP_Left => return Key::Left,
            keysyms::Right | keysyms::KP_Right => return Key::Right,
            keysyms::ISO_Left_Tab => return Key::BackTab,
            keysyms::Tab if self.active(xkbcommon_dl::XKB_MOD_NAME_SHIFT) => return Key::BackTab,
            keysyms::Tab => return Key::Tab,
            _ => {}
        }
        if ctrl
            || self.active(xkbcommon_dl::XKB_MOD_NAME_ALT)
            || self.active(xkbcommon_dl::XKB_MOD_NAME_LOGO)
        {
            return Key::Other;
        }
        let mut buffer = [0 as c_char; 64];
        // SAFETY: state is live and buffer provides exactly the declared writable capacity.
        let len = unsafe {
            (self.xkb.xkb_state_key_get_utf8)(
                self.state,
                code + 8,
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        text_key(&buffer, len)
    }
    pub(crate) fn repeats(&self, code: u32) -> bool {
        // SAFETY: the short-circuit check guarantees a live keymap, exclusively owned here.
        !self.keymap.is_null()
            && unsafe { (self.xkb.xkb_keymap_key_repeats)(self.keymap, code.saturating_add(8)) }
                != 0
    }
    fn release(&mut self) {
        if !self.state.is_null() {
            // SAFETY: this releases the owned state once, before clearing its pointer.
            unsafe { (self.xkb.xkb_state_unref)(self.state) };
        }
        if !self.keymap.is_null() {
            // SAFETY: this releases the owned keymap once, after releasing the dependent state.
            unsafe { (self.xkb.xkb_keymap_unref)(self.keymap) };
        }
        self.state = std::ptr::null_mut();
        self.keymap = std::ptr::null_mut();
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        self.release();
        // SAFETY: dependent state and keymap were released and context still owns its initial reference.
        unsafe { (self.xkb.xkb_context_unref)(self.context) };
    }
}

fn compose_locale() -> CString {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .and_then(|value| CString::new(value.into_vec()).ok())
        .unwrap_or_else(|| c"C".to_owned())
}

fn text_key(buffer: &[c_char], len: i32) -> Key {
    let Ok(len) = usize::try_from(len) else {
        return Key::Other;
    };
    if len == 0 || len >= buffer.len() {
        return Key::Other;
    }
    let bytes: Vec<u8> = buffer[..len].iter().map(|&b| b as u8).collect();
    match String::from_utf8(bytes) {
        Ok(text) if text.chars().all(|c| !c.is_control()) => Key::Text(text),
        _ => Key::Other,
    }
}

struct Compose {
    xkb: &'static XkbCommonCompose,
    state: NonNull<xkb_compose_state>,
}

impl Compose {
    fn new(context: *mut xkb_context, locale: &CStr) -> Option<Self> {
        let xkb = xkbcommon_dl::xkbcommon_compose_option()?;
        // SAFETY: the caller owns a live context and locale is NUL-terminated.
        let table = NonNull::new(unsafe {
            (xkb.xkb_compose_table_new_from_locale)(
                context,
                locale.as_ptr(),
                xkb_compose_compile_flags::XKB_COMPOSE_COMPILE_NO_FLAGS,
            )
        })?;
        // SAFETY: the new state retains a reference to the live table, so the initial reference can be released.
        let state = unsafe {
            let state = (xkb.xkb_compose_state_new)(
                table.as_ptr(),
                xkb_compose_state_flags::XKB_COMPOSE_STATE_NO_FLAGS,
            );
            (xkb.xkb_compose_table_unref)(table.as_ptr());
            NonNull::new(state)?
        };
        Some(Self { xkb, state })
    }

    fn feed(&mut self, sym: xkb_keysym_t) -> Option<Key> {
        use xkb_compose_status::*;
        let state = self.state.as_ptr();
        // SAFETY: the compose state is live and exclusively owned by self.
        let status = unsafe {
            (self.xkb.xkb_compose_state_feed)(state, sym);
            (self.xkb.xkb_compose_state_get_status)(state)
        };
        match status {
            XKB_COMPOSE_NOTHING => None,
            XKB_COMPOSE_COMPOSING => Some(Key::Other),
            XKB_COMPOSE_CANCELLED => {
                self.reset();
                Some(Key::Other)
            }
            XKB_COMPOSE_COMPOSED => {
                let mut buffer = [0 as c_char; 64];
                // SAFETY: the buffer is writable to its declared capacity and state is live.
                let len = unsafe {
                    (self.xkb.xkb_compose_state_get_utf8)(state, buffer.as_mut_ptr(), buffer.len())
                };
                self.reset();
                Some(text_key(&buffer, len))
            }
        }
    }

    fn reset(&mut self) {
        // SAFETY: the state is live and exclusively owned by self.
        unsafe { (self.xkb.xkb_compose_state_reset)(self.state.as_ptr()) };
    }
}

impl Drop for Compose {
    fn drop(&mut self) {
        // SAFETY: the state has exactly one owned reference.
        unsafe { (self.xkb.xkb_compose_state_unref)(self.state.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_password_characters_locally() {
        let mut keyboard = Keyboard::new().unwrap();
        let mut compose = Compose::new(keyboard.context, c"C.UTF-8").unwrap();
        for (dead, base, expected) in [
            (keysyms::dead_acute, keysyms::e, "é"),
            (keysyms::dead_doubleacute, keysyms::o, "ő"),
            (keysyms::dead_doubleacute, keysyms::u, "ű"),
        ] {
            assert_eq!(compose.feed(dead), Some(Key::Other));
            assert_eq!(compose.feed(base), Some(Key::Text(expected.into())));
            assert_eq!(compose.feed(base), None);
        }
        assert_eq!(compose.feed(keysyms::Multi_key), Some(Key::Other));
        assert_eq!(compose.feed(keysyms::apostrophe), Some(Key::Other));
        assert_eq!(compose.feed(keysyms::e), Some(Key::Text("é".into())));
        assert_eq!(compose.feed(keysyms::dead_doubleacute), Some(Key::Other));
        keyboard.compose = Some(compose);
        keyboard.modifiers(0, 0, 0, 1);
        assert_eq!(keyboard.compose.as_mut().unwrap().feed(keysyms::o), None);
    }
}
