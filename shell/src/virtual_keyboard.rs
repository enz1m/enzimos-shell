//! Types Unicode text through a generated virtual-keyboard keymap.

use enzui_shell::i18n::tr;
use std::fmt::Write as _;
use std::io::Write;
use std::os::fd::AsFd;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(State: ZwpVirtualKeyboardV1);

const MAX_CHARS: usize = 64;

/// Whether the compositor offers a virtual keyboard; asked once.
#[must_use]
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Ok(connection) = Connection::connect_to_env() else {
            return false;
        };
        let mut queue = connection.new_event_queue();
        connection.display().get_registry(&queue.handle(), ());
        let mut state = State::default();
        queue.roundtrip(&mut state).is_ok() && state.seat.is_some() && state.manager.is_some()
    })
}

/// A key that is not text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Backspace,
    Enter,
    Tab,
    Left,
    Right,
    Up,
    Down,
}
impl Named {
    fn keysym(self) -> &'static str {
        match self {
            Self::Backspace => "BackSpace",
            Self::Enter => "Return",
            Self::Tab => "Tab",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Up => "Up",
            Self::Down => "Down",
        }
    }
}

fn distinct(keys: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for key in keys {
        if !seen.contains(key) {
            seen.push(key.clone());
        }
    }
    seen
}

/// An XKB keymap giving keycode 9 + i to the i-th keysym.
#[must_use]
pub fn keymap(keysyms: &[String]) -> String {
    let mut codes = String::new();
    let mut symbols = String::new();
    for (index, keysym) in keysyms.iter().enumerate() {
        let _ = writeln!(codes, "<K{index}> = {};", index + 9);
        let _ = writeln!(symbols, "key <K{index}> {{[{keysym}]}};");
    }
    format!(
        "xkb_keymap {{\nxkb_keycodes \"enzui\" {{\nminimum = 8;\nmaximum = {};\n{codes}}};\nxkb_types \"enzui\" {{ include \"complete\" }};\nxkb_compatibility \"enzui\" {{ include \"complete\" }};\nxkb_symbols \"enzui\" {{\n{symbols}}};\n}};\n",
        keysyms.len() + 9
    )
}

fn char_keysym(c: char) -> String {
    format!("U{:04X}", u32::from(c))
}

/// Types `text` into the focused window. Blocking; run it off the UI thread.
pub fn type_text(text: &str) -> Result<(), String> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err(tr("shell-nothing-to-type").into());
    }
    send(&text.chars().map(char_keysym).collect::<Vec<_>>())
}

/// Presses and releases `key` in the focused window. Blocking.
pub fn press(key: Named) -> Result<(), String> {
    send(&[key.keysym().to_owned()])
}

fn send(keys: &[String]) -> Result<(), String> {
    let keysyms = distinct(keys);
    if keysyms.is_empty() || keysyms.len() > MAX_CHARS {
        return Err(tr("shell-nothing-to-type").into());
    }
    let connection =
        Connection::connect_to_env().map_err(|_| tr("shell-no-display-server-connection"))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    let (Some(seat), Some(manager)) = (state.seat.as_ref(), state.manager.as_ref()) else {
        return Err(tr("shell-the-compositor-does-not-offer-a-virtual-keyboard").into());
    };
    let keyboard = manager.create_virtual_keyboard(seat, &qh, ());
    let map = keymap(&keysyms);
    let fd = rustix::fs::memfd_create("enzui-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|_| tr("shell-could-not-prepare-the-keymap"))?;
    let mut file = std::fs::File::from(fd);
    file.write_all(map.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|_| tr("shell-could-not-prepare-the-keymap"))?;
    // WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1
    keyboard.keymap(1, file.as_fd(), (map.len() + 1) as u32);
    keyboard.modifiers(0, 0, 0, 0);
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    let start = std::time::Instant::now();
    for keysym in keys {
        let key = keysyms
            .iter()
            .position(|known| known == keysym)
            .unwrap_or_default() as u32
            + 1;
        let time = start.elapsed().as_millis() as u32;
        keyboard.key(time, key, 1);
        keyboard.key(time + 1, key, 0);
        queue
            .roundtrip(&mut state)
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    }
    keyboard.destroy();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    Ok(())
}
