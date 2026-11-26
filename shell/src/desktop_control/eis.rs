#![allow(
    unsafe_code,
    reason = "libei is the sender for the portal-owned EIS connection."
)]

use super::{Result, unavailable};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::{c_char, c_int, c_void},
    os::fd::{BorrowedFd, IntoRawFd, OwnedFd},
    os::unix::fs::FileExt,
    ptr::{NonNull, null_mut},
};
use tokio::io::unix::AsyncFd;
use xkbcommon::xkb;

#[repr(C)]
struct Context {
    _private: [u8; 0],
}
#[repr(C)]
struct Seat {
    _private: [u8; 0],
}
#[repr(C)]
struct Event {
    _private: [u8; 0],
}
#[repr(C)]
struct Device {
    _private: [u8; 0],
}
#[repr(C)]
struct Region {
    _private: [u8; 0],
}
#[repr(C)]
struct Keymap {
    _private: [u8; 0],
}

const ABSOLUTE: c_int = 1 << 1;
const KEYBOARD: c_int = 1 << 2;
const SCROLL: c_int = 1 << 4;
const BUTTON: c_int = 1 << 5;
const DISCONNECT: c_int = 2;
const SEAT_ADDED: c_int = 3;
const DEVICE_REMOVED: c_int = 6;
const DEVICE_PAUSED: c_int = 7;
const DEVICE_RESUMED: c_int = 8;
const LEFT_BUTTON: u32 = 0x110;
const SHIFT: u32 = 42;

#[link(name = "ei")]
unsafe extern "C" {
    fn ei_new_sender(user_data: *mut c_void) -> *mut Context;
    fn ei_unref(ctx: *mut Context) -> *mut Context;
    fn ei_configure_name(ctx: *mut Context, name: *const c_char);
    fn ei_setup_backend_fd(ctx: *mut Context, fd: c_int) -> c_int;
    fn ei_get_fd(ctx: *mut Context) -> c_int;
    fn ei_now(ctx: *mut Context) -> u64;
    fn ei_dispatch(ctx: *mut Context);
    fn ei_get_event(ctx: *mut Context) -> *mut Event;
    fn ei_event_get_type(event: *mut Event) -> c_int;
    fn ei_event_unref(event: *mut Event) -> *mut Event;
    fn ei_event_get_seat(event: *mut Event) -> *mut Seat;
    fn ei_event_get_device(event: *mut Event) -> *mut Device;
    fn ei_seat_bind_capabilities(seat: *mut Seat, first: c_int, ...);
    fn ei_device_ref(device: *mut Device) -> *mut Device;
    fn ei_device_unref(device: *mut Device) -> *mut Device;
    fn ei_device_has_capability(device: *mut Device, capability: c_int) -> bool;
    fn ei_device_keyboard_get_keymap(device: *mut Device) -> *mut Keymap;
    fn ei_keymap_get_fd(keymap: *mut Keymap) -> c_int;
    fn ei_keymap_get_size(keymap: *mut Keymap) -> usize;
    fn ei_device_get_region(device: *mut Device, index: usize) -> *mut Region;
    fn ei_region_get_width(region: *mut Region) -> u32;
    fn ei_region_get_height(region: *mut Region) -> u32;
    fn ei_region_get_x(region: *mut Region) -> u32;
    fn ei_region_get_y(region: *mut Region) -> u32;
    fn ei_device_start_emulating(device: *mut Device, sequence: u32);
    fn ei_device_stop_emulating(device: *mut Device);
    fn ei_device_frame(device: *mut Device, time: u64);
    fn ei_device_pointer_motion_absolute(device: *mut Device, x: f64, y: f64);
    fn ei_device_button_button(device: *mut Device, button: u32, pressed: bool);
    fn ei_device_scroll_discrete(device: *mut Device, x: i32, y: i32);
    fn ei_device_keyboard_key(device: *mut Device, key: u32, pressed: bool);
}

struct Sender(NonNull<Context>);

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: Sender owns the reference returned by ei_new_sender.
        unsafe { ei_unref(self.0.as_ptr()) };
    }
}

struct Emulation(NonNull<Device>);

impl Emulation {
    fn new(device: NonNull<Device>, sequence: u32) -> Self {
        // SAFETY: the resumed device belongs to a live event, and the new reference outlives it.
        unsafe {
            ei_device_ref(device.as_ptr());
            ei_device_start_emulating(device.as_ptr(), sequence);
        }
        Self(device)
    }
}

impl Drop for Emulation {
    fn drop(&mut self) {
        // SAFETY: Emulation retains a libei device reference until after stop.
        unsafe {
            ei_device_stop_emulating(self.0.as_ptr());
            ei_device_unref(self.0.as_ptr());
        }
    }
}

pub(super) struct Eis {
    context: Sender,
    fd: AsyncFd<OwnedFd>,
    pointer: Option<Emulation>,
    keyboard: Option<Emulation>,
    characters: HashMap<char, Tap>,
    sequence: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tap {
    key: u32,
    modifiers: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scroll {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}

enum Action {
    Move(Position),
    Click(Position),
    Scroll(Scroll),
    Keys(Vec<Tap>),
}

impl Action {
    fn validate(name: &str, arguments: Value, characters: &HashMap<char, Tap>) -> Option<Self> {
        match name {
            "computer_move" | "computer_click" => {
                let point: Position = serde_json::from_value(arguments).ok()?;
                if !(0.0..1.0).contains(&point.x) || !(0.0..1.0).contains(&point.y) {
                    return None;
                }
                Some(if name == "computer_move" {
                    Self::Move(point)
                } else {
                    Self::Click(point)
                })
            }
            "computer_scroll" => {
                let scroll: Scroll = serde_json::from_value(arguments).ok()?;
                ((-12..=12).contains(&scroll.x)
                    && (-12..=12).contains(&scroll.y)
                    && (scroll.x != 0 || scroll.y != 0))
                    .then_some(Self::Scroll(scroll))
            }
            "computer_key" => {
                let Key { key } = serde_json::from_value(arguments).ok()?;
                parse_key(&key, characters).map(|key| Self::Keys(vec![key]))
            }
            "computer_type" => {
                let Text { text } = serde_json::from_value(arguments).ok()?;
                if text.is_empty() || text.len() > 1024 || text.chars().count() > 256 {
                    return None;
                }
                text.chars()
                    .map(|character| characters.get(&character).cloned())
                    .collect::<Option<Vec<_>>>()
                    .map(Self::Keys)
            }
            _ => None,
        }
    }
}

fn parse_key(key: &str, characters: &HashMap<char, Tap>) -> Option<Tap> {
    if !(1..=32).contains(&key.len()) {
        return None;
    }
    let mut parts = key.rsplit('+');
    let name = parts.next()?;
    let code = match name {
        "Escape" => 1,
        "Backspace" => 14,
        "Tab" => 15,
        "Enter" => 28,
        "Space" => 57,
        "Home" => 102,
        "Up" => 103,
        "PageUp" => 104,
        "Left" => 105,
        "Right" => 106,
        "End" => 107,
        "Down" => 108,
        "PageDown" => 109,
        "Delete" => 111,
        key if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() => 0,
        _ => return None,
    };
    let mut mods = Vec::new();
    for part in parts {
        let modifier = match part {
            "Ctrl" => 29,
            "Shift" => SHIFT,
            "Alt" => 56,
            "Super" => 125,
            _ => return None,
        };
        if mods.contains(&modifier) || (code == 1 && modifier == 125) {
            return None;
        }
        mods.push(modifier);
    }
    if code == 0 {
        let tap = characters.get(&char::from(name.as_bytes()[0].to_ascii_lowercase()))?;
        for &modifier in &tap.modifiers {
            if !mods.contains(&modifier) {
                mods.push(modifier);
            }
        }
        return Some(Tap {
            key: tap.key,
            modifiers: mods,
        });
    }
    Some(Tap {
        key: code,
        modifiers: mods,
    })
}

fn keymap(device: NonNull<Device>) -> Result<HashMap<char, Tap>> {
    // SAFETY: the resumed device is live; its keymap keeps the borrowed fd open.
    let (file, size) = unsafe {
        let map =
            NonNull::new(ei_device_keyboard_get_keymap(device.as_ptr())).ok_or_else(unavailable)?;
        let fd = ei_keymap_get_fd(map.as_ptr());
        let size = ei_keymap_get_size(map.as_ptr());
        if fd < 0 || !(1..=1024 * 1024).contains(&size) {
            return Err(unavailable());
        }
        (
            rustix::io::dup(BorrowedFd::borrow_raw(fd)).map_err(|_| unavailable())?,
            size,
        )
    };
    let mut input = vec![0; size];
    std::fs::File::from(file)
        .read_exact_at(&mut input, 0)
        .map_err(|_| unavailable())?;
    let input = String::from_utf8(input).map_err(|_| unavailable())?;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let map = xkb::Keymap::new_from_string(
        &context,
        input.trim_end_matches('\0').to_owned(),
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(unavailable)?;
    let mut state = xkb::State::new(&map);
    let mut chars = HashMap::new();
    for shifted in [false, true] {
        if shifted {
            state.update_key(xkb::Keycode::new(SHIFT + 8), xkb::KeyDirection::Down);
        }
        for raw in map.min_keycode().raw().max(9)..=map.max_keycode().raw().min(255) {
            let text = state.key_get_utf8(xkb::Keycode::new(raw));
            let mut text = text.chars();
            if let (Some(character), None) = (text.next(), text.next())
                && !character.is_control()
            {
                chars.entry(character).or_insert_with(|| Tap {
                    key: raw - 8,
                    modifiers: if shifted { vec![SHIFT] } else { vec![] },
                });
            }
        }
    }
    Ok(chars)
}

impl Eis {
    pub(super) fn new(fd: OwnedFd) -> Result<Self> {
        // SAFETY: libei owns the EIS descriptor once setup is called. The poll fd is duplicated
        // so Tokio can track it without taking ownership of libei's internal descriptor.
        let (context, poll) = unsafe {
            let context = Sender(NonNull::new(ei_new_sender(null_mut())).ok_or_else(unavailable)?);
            ei_configure_name(context.0.as_ptr(), c"enzimOS Assistant".as_ptr());
            if ei_setup_backend_fd(context.0.as_ptr(), fd.into_raw_fd()) != 0 {
                return Err(unavailable());
            }
            let poll = ei_get_fd(context.0.as_ptr());
            if poll < 0 {
                return Err(unavailable());
            }
            (
                context,
                rustix::io::dup(BorrowedFd::borrow_raw(poll)).map_err(|_| unavailable())?,
            )
        };
        // SAFETY: the owned duplicate stays open and its descriptor is stable until unregister.
        let fd = unsafe { AsyncFd::register_with_interest(poll, tokio::io::Interest::READABLE) }
            .map_err(|_| unavailable())?;
        Ok(Self {
            context,
            fd,
            pointer: None,
            keyboard: None,
            characters: HashMap::new(),
            sequence: 0,
        })
    }

    pub(super) fn ready(&self) -> bool {
        self.pointer.is_some() && self.keyboard.is_some()
    }

    pub(super) async fn dispatch(&mut self) -> Result<()> {
        self.fd
            .readable()
            .await
            .map_err(|_| unavailable())?
            .clear_ready();
        let zero = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // libei exports an epoll fd. Drain its level-triggered readiness before
        // relying on Tokio's next edge.
        for _ in 0..32 {
            self.drain()?;
            let mut wait = [rustix::event::PollFd::new(
                self.fd.get_ref(),
                rustix::event::PollFlags::IN,
            )];
            if rustix::event::poll(&mut wait, Some(&zero)).map_err(|_| unavailable())? == 0 {
                return Ok(());
            }
        }
        Err(unavailable())
    }

    fn drain(&mut self) -> Result<()> {
        let context = self.context.0.as_ptr();
        // SAFETY: the context owns all returned event references until they are unref'd.
        unsafe {
            ei_dispatch(context);
            while let Some(event) = NonNull::new(ei_get_event(context)) {
                let result = self.event(event);
                ei_event_unref(event.as_ptr());
                result?;
            }
        }
        Ok(())
    }

    fn event(&mut self, event: NonNull<Event>) -> Result<()> {
        // SAFETY: the event remains live for this handler, and retains its seat/device.
        unsafe {
            match ei_event_get_type(event.as_ptr()) {
                DISCONNECT => Err(unavailable()),
                SEAT_ADDED => {
                    let seat = ei_event_get_seat(event.as_ptr());
                    ei_seat_bind_capabilities(seat, ABSOLUTE, BUTTON, SCROLL, KEYBOARD, 0 as c_int);
                    Ok(())
                }
                kind @ (DEVICE_RESUMED | DEVICE_REMOVED | DEVICE_PAUSED) => {
                    let device = NonNull::new(ei_event_get_device(event.as_ptr()))
                        .ok_or_else(unavailable)?;
                    if kind != DEVICE_RESUMED {
                        if [&self.pointer, &self.keyboard]
                            .into_iter()
                            .flatten()
                            .any(|active| active.0 == device)
                        {
                            return Err(unavailable());
                        }
                    } else if ei_device_has_capability(device.as_ptr(), KEYBOARD) {
                        self.characters = keymap(device)?;
                        self.sequence += 1;
                        self.keyboard = Some(Emulation::new(device, self.sequence));
                    } else if [ABSOLUTE, BUTTON, SCROLL]
                        .into_iter()
                        .all(|cap| ei_device_has_capability(device.as_ptr(), cap))
                    {
                        self.sequence += 1;
                        self.pointer = Some(Emulation::new(device, self.sequence));
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    pub(super) fn perform(
        &mut self,
        tool: &str,
        arguments: Value,
        allowed: impl Fn() -> bool,
    ) -> Result<bool> {
        self.drain()?;
        let Some(action) = Action::validate(tool, arguments, &self.characters) else {
            return Ok(false);
        };
        let pointer = self.pointer.as_ref().ok_or_else(unavailable)?.0.as_ptr();
        let keyboard = self.keyboard.as_ref().ok_or_else(unavailable)?.0.as_ptr();
        let context = self.context.0.as_ptr();
        let frame = |device| {
            // SAFETY: the device and context are retained by self.
            unsafe { ei_device_frame(device, ei_now(context)) };
        };
        if !allowed() {
            return Err(unavailable());
        }
        // SAFETY: both devices are resumed, retain their capabilities, and input has been validated.
        unsafe {
            match action {
                Action::Move(ref point) | Action::Click(ref point) => {
                    let region = NonNull::new(ei_device_get_region(pointer, 0))
                        .ok_or_else(unavailable)?
                        .as_ptr();
                    let x = f64::from(ei_region_get_x(region))
                        + point.x * f64::from(ei_region_get_width(region));
                    let y = f64::from(ei_region_get_y(region))
                        + point.y * f64::from(ei_region_get_height(region));
                    ei_device_pointer_motion_absolute(pointer, x, y);
                    frame(pointer);
                    if matches!(action, Action::Click(_)) && allowed() {
                        ei_device_button_button(pointer, LEFT_BUTTON, true);
                        frame(pointer);
                        ei_device_button_button(pointer, LEFT_BUTTON, false);
                        frame(pointer);
                    }
                }
                Action::Scroll(scroll) => {
                    ei_device_scroll_discrete(pointer, scroll.x * 120, scroll.y * 120);
                    frame(pointer);
                }
                Action::Keys(keys) => {
                    let key = |code, pressed| {
                        ei_device_keyboard_key(keyboard, code, pressed);
                        frame(keyboard);
                    };
                    for tap in keys {
                        if !allowed() {
                            return Err(unavailable());
                        }
                        for &modifier in &tap.modifiers {
                            key(modifier, true);
                        }
                        key(tap.key, true);
                        key(tap.key, false);
                        for &modifier in tap.modifiers.iter().rev() {
                            key(modifier, false);
                        }
                    }
                }
            }
            ei_dispatch(context);
        }
        Ok(true)
    }
}

impl Drop for Eis {
    fn drop(&mut self) {
        self.pointer = None;
        self.keyboard = None;
        // SAFETY: the context is still live until field destruction.
        unsafe { ei_dispatch(self.context.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_is_bounded_and_key_transitions_are_paired() {
        let chars = HashMap::from([(
            'a',
            Tap {
                key: 30,
                modifiers: vec![],
            },
        )]);
        assert!(matches!(
            Action::validate("computer_click", json!({"x":0.999,"y":0}), &chars),
            Some(Action::Click(_))
        ));
        for point in [
            json!({"x":1,"y":0}),
            json!({"x":0,"y":-0.1}),
            json!({"x":"0","y":0}),
            json!({"x":0,"y":0,"button":"right"}),
        ] {
            assert!(Action::validate("computer_click", point, &chars).is_none());
        }
        assert!(Action::validate("computer_scroll", json!({"x":0,"y":13}), &chars).is_none());
        assert!(
            Action::validate("computer_type", json!({"text":"a".repeat(256)}), &chars).is_some()
        );
        for text in [
            "a".repeat(257),
            "a
a"
            .into(),
            "ab".into(),
        ] {
            assert!(Action::validate("computer_type", json!({"text":text}), &chars).is_none());
        }
        assert_eq!(
            parse_key("Ctrl+Shift+a", &chars),
            Some(Tap {
                key: 30,
                modifiers: vec![SHIFT, 29]
            })
        );
        assert!(parse_key("Shift+Shift+a", &chars).is_none());
        assert!(parse_key("Super+Escape", &chars).is_none());
    }
}
