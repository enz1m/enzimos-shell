//! Scheduled compositor night-light transform without geolocation.
//! Releases the transform while off and remains off when another client owns it.

use enzui_shell::i18n::tr;
use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::time::Duration;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_output, wl_registry},
};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all,
    clippy::pedantic,
    reason = "Wayland scanner generates protocol bindings with upstream naming and signatures."
)]
mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;
    pub(super) mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-ctm-control-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/hyprland-ctm-control-v1.xml");
}
use protocol::hyprland_ctm_control_manager_v1::{self, HyprlandCtmControlManagerV1};

/// The neutral colour temperature: no change.
pub const NEUTRAL: u16 = 6500;
pub const WARMEST: u16 = 2500;
pub const COOLEST: u16 = 6000;

/// Stored under `night_light`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    /// Kelvin while active.
    pub temperature: u16,
    /// `false`: always on while enabled.
    pub scheduled: bool,
    /// Minutes after midnight, local time.
    pub from: u16,
    pub to: u16,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 4000,
            scheduled: true,
            from: 20 * 60,
            to: 7 * 60,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Enabled(bool),
    Temperature(u16),
    Scheduled(bool),
    From(u16),
    To(u16),
}

impl Preferences {
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, String> {
        let result: Self = match value {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| tr("shell-invalid-night-light-preferences"))?,
            None => Self::default(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        let ok = (WARMEST..=COOLEST).contains(&self.temperature)
            && self.from < 24 * 60
            && self.to < 24 * 60;
        if ok {
            Ok(())
        } else {
            Err(tr("shell-night-light-preferences-exceed-their-bounds").into())
        }
    }
    pub fn apply(&mut self, change: Change) -> Result<(), String> {
        let mut next = *self;
        match change {
            Change::Enabled(on) => next.enabled = on,
            Change::Temperature(kelvin) => next.temperature = kelvin,
            Change::Scheduled(on) => next.scheduled = on,
            Change::From(minutes) => next.from = minutes,
            Change::To(minutes) => next.to = minutes,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn update_json(value: &mut serde_json::Value, change: Change) -> Result<(), String> {
        let mut next = Self::from_json(value.get("night_light"))?;
        next.apply(change)?;
        value["night_light"] = serde_json::to_value(next)
            .map_err(|_| tr("shell-cannot-encode-night-light-preferences"))?;
        Ok(())
    }
    /// Whether night light is on at `minutes` after local midnight.
    #[must_use]
    pub fn active_at(&self, minutes: u16) -> bool {
        if !self.enabled {
            return false;
        }
        if !self.scheduled || self.from == self.to {
            return true;
        }
        if self.from < self.to {
            (self.from..self.to).contains(&minutes)
        } else {
            minutes >= self.from || minutes < self.to
        }
    }
    /// The colour temperature at `minutes` after local midnight.
    #[must_use]
    pub fn target_at(&self, minutes: u16) -> u16 {
        if self.active_at(minutes) {
            self.temperature
        } else {
            NEUTRAL
        }
    }
}

/// Minutes after local midnight now.
#[must_use]
pub fn local_minutes() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

fn channels(kelvin: f64) -> [f64; 3] {
    // Tanner Helland's fit of the black-body colour.
    let t = kelvin / 100.0;
    let red = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };
    let green = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    [red, green, blue].map(|c| c.clamp(0.0, 255.0))
}

/// The diagonal of the colour transform for `kelvin`, scaled so that
/// [`NEUTRAL`] is the identity.
#[must_use]
pub fn matrix(kelvin: f64) -> [f64; 3] {
    let neutral = channels(f64::from(NEUTRAL));
    let warm = channels(kelvin.clamp(1000.0, f64::from(NEUTRAL)));
    [0, 1, 2].map(|i| (warm[i] / neutral[i]).clamp(0.0, 1.0))
}

#[derive(Default)]
struct State {
    outputs: Vec<wl_output::WlOutput>,
    // Global name and version.
    manager: Option<(u32, u32)>,
    blocked: bool,
    changed: bool,
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
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    state
                        .outputs
                        .push(registry.bind(name, version.min(4), qh, ()));
                    state.changed = true;
                }
                "hyprland_ctm_control_manager_v1" => state.manager = Some((name, version)),
                _ => {}
            },
            wl_registry::Event::GlobalRemove { .. } => state.changed = true,
            _ => {}
        }
    }
}
impl Dispatch<HyprlandCtmControlManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &HyprlandCtmControlManagerV1,
        event: hyprland_ctm_control_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            hyprland_ctm_control_manager_v1::Event::Blocked => state.blocked = true,
        }
    }
}
wayland_client::delegate_noop!(State: ignore wl_output::WlOutput);

/// The running night light; dropping it returns the screen to normal.
pub struct NightLight {
    targets: mpsc::Sender<u16>,
}

impl NightLight {
    /// Starts the colour transform worker; `errors` receives why it stopped.
    pub fn start(errors: impl Fn(String) + Send + 'static) -> Self {
        let (targets, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("night-light".into())
            .spawn(move || {
                if let Err(error) = run(&receiver) {
                    errors(error);
                }
            })
            .ok();
        Self { targets }
    }
    /// Fades to `kelvin` ([`NEUTRAL`] releases the screen).
    pub fn set(&self, kelvin: u16) {
        let _ = self.targets.send(kelvin);
    }
}

/// Whether some client already holds the colour transform: binds the
/// manager once, as a second client would, and lets go. For tests.
pub fn held() -> Result<bool, String> {
    let connection =
        Connection::connect_to_env().map_err(|_| tr("shell-no-display-server-connection"))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    let (name, version) = state
        .manager
        .ok_or_else(|| tr("shell-the-compositor-has-no-colour-transform-control"))?;
    let manager: HyprlandCtmControlManagerV1 = registry.bind(name, version.min(2), &qh, ());
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    manager.destroy();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    Ok(state.blocked)
}

fn run(targets: &mpsc::Receiver<u16>) -> Result<(), String> {
    let connection =
        Connection::connect_to_env().map_err(|_| tr("shell-no-display-server-connection"))?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    let mut bound: Option<HyprlandCtmControlManagerV1> = None;
    let (mut current, mut target) = (f64::from(NEUTRAL), NEUTRAL);
    loop {
        let fading = (current - f64::from(target)).abs() > 0.5;
        match targets.recv_timeout(if fading {
            Duration::from_millis(40)
        } else {
            Duration::from_secs(2)
        }) {
            Ok(kelvin) => target = kelvin.clamp(1000, NEUTRAL),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        queue
            .roundtrip(&mut state)
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        let before = current;
        // About a second and a half from neutral to 4000 K.
        let step = 70.0;
        current = if current < f64::from(target) {
            (current + step).min(f64::from(target))
        } else {
            (current - step).max(f64::from(target))
        };
        if current >= f64::from(NEUTRAL) - 0.5 {
            // Neutral: let go of the transform, which resets it.
            if let Some(manager) = bound.take() {
                manager.destroy();
                connection
                    .flush()
                    .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
            }
            continue;
        }
        if bound.is_none() {
            let (name, version) = state
                .manager
                .ok_or_else(|| tr("shell-the-compositor-has-no-colour-transform-control"))?;
            // Version 2 says when another client holds the transform.
            bound = Some(registry.bind(name, version.min(2), &qh, ()));
            state.changed = true;
            queue
                .roundtrip(&mut state)
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        }
        if state.blocked {
            if let Some(manager) = bound.take() {
                manager.destroy();
            }
            return Err(tr("shell-another-app-controls-the-screen-colours").into());
        }
        if (current - before).abs() < f64::EPSILON && !std::mem::take(&mut state.changed) {
            continue;
        }
        if let Some(manager) = &bound {
            let [r, g, b] = matrix(current);
            for output in &state.outputs {
                manager.set_ctm_for_output(output, r, 0.0, 0.0, 0.0, g, 0.0, 0.0, 0.0, b);
            }
            manager.commit();
            connection
                .flush()
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        }
    }
}
