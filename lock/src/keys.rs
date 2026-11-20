//! Keyboard input from the compositor's keymap, through libxkbcommon.

#![allow(
    unsafe_code,
    reason = "libxkbcommon uses owned opaque pointers behind its C ABI."
)]

use std::ffi::c_char;
use std::os::fd::OwnedFd;
use xkbcommon_dl::{
    XkbCommon, keysyms, xkb_context, xkb_context_flags, xkb_keymap, xkb_keymap_compile_flags,
    xkb_keymap_format, xkb_state, xkb_state_component,
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
        true
    }
    pub(crate) fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
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
    // Interprets a Wayland key code (evdev code; xkb adds 8).
    pub(crate) fn key(&self, code: u32) -> Key {
        if self.state.is_null() {
            return Key::Other;
        }
        let Some(code) = code.checked_add(8) else {
            return Key::Other;
        };
        // SAFETY: state is live; XKB accepts any keycode and returns NoSymbol for unknown keys.
        let sym = unsafe { (self.xkb.xkb_state_key_get_one_sym)(self.state, code) };
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
            (self.xkb.xkb_state_key_get_utf8)(self.state, code, buffer.as_mut_ptr(), buffer.len())
        };
        if len <= 0 || len as usize >= buffer.len() {
            return Key::Other;
        }
        let bytes: Vec<u8> = buffer[..len as usize].iter().map(|&b| b as u8).collect();
        match String::from_utf8(bytes) {
            Ok(text) if text.chars().all(|c| !c.is_control()) => Key::Text(text),
            _ => Key::Other,
        }
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
