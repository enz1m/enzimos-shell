use enzui_shell::i18n::tr;
use enzui_shell::tr;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    backend::ObjectId,
    event_created_child,
    protocol::{wl_output::Transform, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}
impl Mode {
    pub fn resolution(&self) -> String {
        format!("{} × {}", self.width, self.height)
    }
    pub fn refresh(&self) -> String {
        if self.refresh_mhz <= 0 {
            return "—".into();
        }
        tr!("shell-hz", "arg0" => enzui_shell::i18n::number(format!("{:.0}", f64::from(self.refresh_mhz) / 1000.0)))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub enabled: bool,
    pub modes: Vec<Mode>,
    pub current: Option<usize>,
    pub scale: f64,
    pub position: (i32, i32),
    pub physical_mm: (i32, i32),
    pub rotation: Rotation,
    pub serial: String,
}

/// How the picture is turned, clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    #[default]
    Normal,
    Right,
    UpsideDown,
    Left,
}
impl Rotation {
    pub const ALL: [Self; 4] = [Self::Normal, Self::Right, Self::UpsideDown, Self::Left];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => tr("shell-landscape"),
            Self::Right => tr("shell-portrait-turned-right"),
            Self::UpsideDown => tr("shell-landscape-upside-down"),
            Self::Left => tr("shell-portrait-turned-left"),
        }
    }
    fn sideways(self) -> bool {
        matches!(self, Self::Right | Self::Left)
    }
    fn transform(self) -> Transform {
        match self {
            Self::Normal => Transform::Normal,
            Self::Right => Transform::_90,
            Self::UpsideDown => Transform::_180,
            Self::Left => Transform::_270,
        }
    }
    fn from_transform(transform: Transform) -> Self {
        match transform {
            Transform::_90 | Transform::Flipped90 => Self::Right,
            Transform::_180 | Transform::Flipped180 => Self::UpsideDown,
            Transform::_270 | Transform::Flipped270 => Self::Left,
            _ => Self::Normal,
        }
    }
}

impl Output {
    /// Identifies the same monitor across ports and sessions when it
    /// reports a make, model and serial; otherwise by its port.
    pub fn key(&self) -> String {
        if self.make.trim().is_empty() && self.model.trim().is_empty() {
            format!("port:{}", self.name)
        } else {
            format!(
                "{}|{}|{}",
                self.make.trim(),
                self.model.trim(),
                self.serial.trim()
            )
        }
    }
    pub fn title(&self) -> String {
        let product = format!("{} {}", self.make.trim(), self.model.trim())
            .trim()
            .to_owned();
        if product.is_empty() || product.eq_ignore_ascii_case("unknown unknown") {
            self.name.clone()
        } else {
            product
        }
    }
    pub fn current_mode(&self) -> Option<Mode> {
        self.current.and_then(|i| self.modes.get(i).copied())
    }
    /// The size the output covers in the desktop layout.
    pub fn logical_size(&self) -> (f64, f64) {
        logical(
            self.current_mode().unwrap_or_default(),
            self.scale,
            self.rotation,
        )
    }
    pub fn diagonal_inches(&self) -> Option<f64> {
        let (w, h) = self.physical_mm;
        (w > 0 && h > 0).then(|| (f64::from(w).hypot(f64::from(h))) / 25.4)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Displays {
    /// `false` when the compositor does not offer wlr-output-management.
    pub supported: bool,
    pub outputs: Vec<Output>,
}

fn logical(mode: Mode, scale: f64, rotation: Rotation) -> (f64, f64) {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    // The compositor rounds fractional logical sizes to whole pixels.
    let (w, h) = (
        (f64::from(mode.width) / scale).round(),
        (f64::from(mode.height) / scale).round(),
    );
    if rotation.sideways() { (h, w) } else { (w, h) }
}

/// The requested state of one output. A layout lists every output.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub output: String,
    pub enabled: bool,
    pub mode: Mode,
    pub scale: f64,
    pub rotation: Rotation,
    pub position: (i32, i32),
}
impl Change {
    pub fn of(output: &Output) -> Option<Self> {
        Some(Self {
            output: output.name.clone(),
            enabled: output.enabled,
            mode: output
                .current_mode()
                .or_else(|| output.modes.iter().find(|m| m.preferred).copied())
                .or_else(|| output.modes.first().copied())?,
            scale: if output.scale > 0.0 {
                output.scale
            } else {
                1.0
            },
            rotation: output.rotation,
            position: output.position,
        })
    }
    pub fn logical_size(&self) -> (f64, f64) {
        logical(self.mode, self.scale, self.rotation)
    }
}

/// Where a display sits next to the main (first enabled) display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    Above,
    Below,
}
impl Side {
    pub const ALL: [Self; 4] = [Self::Right, Self::Left, Self::Above, Self::Below];
    pub fn label(self) -> &'static str {
        match self {
            Self::Right => tr("shell-right-of"),
            Self::Left => tr("shell-left-of"),
            Self::Above => tr("shell-above"),
            Self::Below => tr("shell-below"),
        }
    }
}

/// The current layout of every output that can be configured.
pub fn layout(displays: &Displays) -> Vec<Change> {
    displays.outputs.iter().filter_map(Change::of).collect()
}

/// Which side of the main display each other enabled display is on.
pub fn sides(layout: &[Change]) -> Vec<(String, Side)> {
    let Some(main) = layout.iter().find(|c| c.enabled) else {
        return Vec::new();
    };
    let (mw, mh) = main.logical_size();
    let centre = |c: &Change| {
        let (w, h) = c.logical_size();
        (
            f64::from(c.position.0) + w / 2.0,
            f64::from(c.position.1) + h / 2.0,
        )
    };
    let (mx, my) = centre(main);
    layout
        .iter()
        .filter(|c| c.enabled && c.output != main.output)
        .map(|c| {
            let (x, y) = centre(c);
            let (dx, dy) = ((x - mx) / mw.max(1.0), (y - my) / mh.max(1.0));
            let side = if dx.abs() >= dy.abs() {
                if dx >= 0.0 { Side::Right } else { Side::Left }
            } else if dy >= 0.0 {
                Side::Below
            } else {
                Side::Above
            };
            (c.output.clone(), side)
        })
        .collect()
}

/// Places displays edge to edge around the main one, so a size, scale or
/// rotation change never leaves gaps or overlaps.
pub fn pack(layout: &mut [Change], sides: &[(String, Side)]) {
    let Some(main) = layout.iter().position(|c| c.enabled) else {
        return;
    };
    let (mw, mh) = layout[main].logical_size();
    layout[main].position = (0, 0);
    let mut offsets = [0.0f64; 4];
    for change in layout.iter_mut().filter(|c| c.enabled) {
        let Some(&(_, side)) = sides.iter().find(|(name, _)| *name == change.output) else {
            continue;
        };
        let (w, h) = change.logical_size();
        let slot = Side::ALL.iter().position(|s| *s == side).unwrap_or(0);
        let offset = offsets[slot];
        change.position = match side {
            Side::Right => ((mw + offset).round() as i32, 0),
            Side::Left => ((-offset - w).round() as i32, 0),
            Side::Below => (0, (mh + offset).round() as i32),
            Side::Above => (0, (-offset - h).round() as i32),
        };
        offsets[slot] += if matches!(side, Side::Right | Side::Left) {
            w
        } else {
            h
        };
    }
    let min_x = layout
        .iter()
        .filter(|c| c.enabled)
        .map(|c| c.position.0)
        .min()
        .unwrap_or(0);
    let min_y = layout
        .iter()
        .filter(|c| c.enabled)
        .map(|c| c.position.1)
        .min()
        .unwrap_or(0);
    for change in layout.iter_mut().filter(|c| c.enabled) {
        change.position = (change.position.0 - min_x, change.position.1 - min_y);
    }
}

/// A display layout remembered in the shell settings (`displays`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub key: String,
    pub enabled: bool,
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub scale: f64,
    #[serde(default)]
    pub rotation: Rotation,
    pub x: i32,
    pub y: i32,
}
// Loaded entries pass `valid`, which rules out a NaN scale.
impl Eq for Saved {}
impl Saved {
    pub fn valid(&self) -> bool {
        !self.key.is_empty()
            && self.key.len() <= 320
            && (1..=16384).contains(&self.width)
            && (1..=16384).contains(&self.height)
            && (0..=1_000_000).contains(&self.refresh_mhz)
            && (0.5..=4.0).contains(&self.scale)
            && self.x.abs() <= 65536
            && self.y.abs() <= 65536
    }
}

const REMEMBERED: usize = 32;

/// Reads the remembered layouts; an absent key means none.
pub fn saved_from_json(value: Option<&serde_json::Value>) -> Result<Vec<Saved>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let saved: Vec<Saved> =
        serde_json::from_value(value.clone()).map_err(|_| tr("shell-invalid-display-layouts"))?;
    if saved.len() > REMEMBERED || saved.iter().any(|s| !s.valid()) {
        return Err(tr("shell-invalid-display-layouts").into());
    }
    Ok(saved)
}

/// Remembers `update`, replacing earlier entries for the same monitors;
/// monitors not connected now keep theirs, oldest dropped first.
pub fn remember(existing: &mut Vec<Saved>, update: &[Saved]) -> Result<(), String> {
    if update.len() > REMEMBERED || update.iter().any(|s| !s.valid()) {
        return Err(tr("shell-invalid-display-layout").into());
    }
    existing.retain(|old| !update.iter().any(|new| new.key == old.key));
    existing.extend(update.iter().cloned());
    let excess = existing.len().saturating_sub(REMEMBERED);
    existing.drain(..excess);
    Ok(())
}

/// What to remember for `layout`, keyed by each monitor's identity.
pub fn saved(displays: &Displays, layout: &[Change]) -> Vec<Saved> {
    layout
        .iter()
        .filter_map(|c| {
            let output = displays.outputs.iter().find(|o| o.name == c.output)?;
            Some(Saved {
                key: output.key(),
                enabled: c.enabled,
                width: c.mode.width,
                height: c.mode.height,
                refresh_mhz: c.mode.refresh_mhz,
                scale: c.scale,
                rotation: c.rotation,
                x: c.position.0,
                y: c.position.1,
            })
        })
        .collect()
}

/// Restores connected-output layouts without disabling every display; unknown outputs retain
/// their state.
pub fn restored(displays: &Displays, saved: &[Saved]) -> Option<Vec<Change>> {
    let current = layout(displays);
    let mut next = current.clone();
    for change in &mut next {
        let Some(output) = displays.outputs.iter().find(|o| o.name == change.output) else {
            continue;
        };
        let Some(entry) = saved.iter().find(|s| s.key == output.key() && s.valid()) else {
            continue;
        };
        let exact = output.modes.iter().find(|m| {
            (m.width, m.height, m.refresh_mhz) == (entry.width, entry.height, entry.refresh_mhz)
        });
        let close = output
            .modes
            .iter()
            .filter(|m| (m.width, m.height) == (entry.width, entry.height))
            .min_by_key(|m| (m.refresh_mhz - entry.refresh_mhz).abs());
        let Some(mode) = exact.or(close) else {
            continue;
        };
        *change = Change {
            output: change.output.clone(),
            enabled: entry.enabled,
            mode: *mode,
            scale: entry.scale,
            rotation: entry.rotation,
            position: (entry.x, entry.y),
        };
    }
    if !next.iter().any(|c| c.enabled) || next == current {
        return None;
    }
    // A display the layout does not know could overlap the others: keep
    // the remembered arrangement and put it on the right.
    let known = |c: &Change| {
        displays
            .outputs
            .iter()
            .any(|o| o.name == c.output && saved.iter().any(|s| s.key == o.key()))
    };
    if next.iter().any(|c| c.enabled && !known(c)) {
        let unknown: Vec<String> = next
            .iter()
            .filter(|c| !known(c))
            .map(|c| c.output.clone())
            .collect();
        let sides: Vec<(String, Side)> = sides(&next)
            .into_iter()
            .map(|(name, side)| {
                let side = if unknown.contains(&name) {
                    Side::Right
                } else {
                    side
                };
                (name, side)
            })
            .collect();
        pack(&mut next, &sides);
    }
    Some(next)
}

pub const SCALES: [f64; 9] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// Whether `scale` leaves a usable desktop on `mode`. The compositor accepts any listed scale
/// and rounds the logical size, so fractional results such as 1804.8 pixels are fine; the
/// limit matches the automatic scale policy in `config/desktop-scale.lua`.
#[must_use]
pub fn supports_scale(mode: Mode, scale: f64) -> bool {
    let (w, h) = (
        (f64::from(mode.width) / scale).round(),
        (f64::from(mode.height) / scale).round(),
    );
    (scale <= 1.0 && w.max(h) <= 16384.0) || (w.max(h) >= 1024.0 && w.min(h) >= 640.0)
}

#[derive(Default)]
struct State {
    manager: Option<ZwlrOutputManagerV1>,
    serial: Option<u32>,
    heads: Vec<(ZwlrOutputHeadV1, Output, Vec<ZwlrOutputModeV1>)>,
    modes: HashMap<ObjectId, (usize, usize)>,
    outcome: Option<Result<(), String>>,
}
impl State {
    fn head(&mut self, head: &ZwlrOutputHeadV1) -> Option<usize> {
        self.heads.iter().position(|(h, ..)| h == head)
    }
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
            && interface == ZwlrOutputManagerV1::interface().name
        {
            state.manager = Some(registry.bind(name, version.min(4), qh, ()));
        }
    }
}
impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.heads.push((
                    head,
                    Output {
                        scale: 1.0,
                        ..Output::default()
                    },
                    Vec::new(),
                ));
            }
            zwlr_output_manager_v1::Event::Done { serial } => state.serial = Some(serial),
            _ => {}
        }
    }
    event_created_child!(State, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}
impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        head: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.head(head) else {
            return;
        };
        let text = |s: String| s.chars().take(96).collect::<String>();
        match event {
            zwlr_output_head_v1::Event::Name { name } => state.heads[index].1.name = text(name),
            zwlr_output_head_v1::Event::Description { description } => {
                state.heads[index].1.description = text(description);
            }
            zwlr_output_head_v1::Event::Make { make } => state.heads[index].1.make = text(make),
            zwlr_output_head_v1::Event::Model { model } => state.heads[index].1.model = text(model),
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                state.heads[index].1.physical_mm = (width, height);
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => {
                state.heads[index].1.enabled = enabled != 0;
            }
            zwlr_output_head_v1::Event::Position { x, y } => state.heads[index].1.position = (x, y),
            zwlr_output_head_v1::Event::Scale { scale } => state.heads[index].1.scale = scale,
            zwlr_output_head_v1::Event::Transform {
                transform: WEnum::Value(transform),
            } => {
                state.heads[index].1.rotation = Rotation::from_transform(transform);
            }
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                state.heads[index].1.serial = text(serial_number);
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                let modes = &mut state.heads[index];
                if modes.2.len() < 128 {
                    state.modes.insert(mode.id(), (index, modes.2.len()));
                    modes.1.modes.push(Mode::default());
                    modes.2.push(mode);
                }
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                if let Some(&(_, m)) = state.modes.get(&mode.id()) {
                    state.heads[index].1.current = Some(m);
                }
            }
            _ => {}
        }
    }
    event_created_child!(State, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}
impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        mode: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(&(h, m)) = state.modes.get(&mode.id()) else {
            return;
        };
        let entry = &mut state.heads[h].1.modes[m];
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                (entry.width, entry.height) = (width, height);
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => entry.refresh_mhz = refresh,
            zwlr_output_mode_v1::Event::Preferred => entry.preferred = true,
            _ => {}
        }
    }
}
impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.outcome = match event {
            zwlr_output_configuration_v1::Event::Succeeded => Some(Ok(())),
            zwlr_output_configuration_v1::Event::Failed => {
                Some(Err(tr("shell-the-display-rejected-that-setting").into()))
            }
            zwlr_output_configuration_v1::Event::Cancelled => {
                Some(Err(tr("shell-displays-changed-try-again").into()))
            }
            _ => state.outcome.take(),
        }
    }
}
impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn connect() -> Result<(Connection, wayland_client::EventQueue<State>, State), String> {
    let connection = Connection::connect_to_env()
        .map_err(|_| tr("shell-no-display-server-connection").to_owned())?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue
        .roundtrip(&mut state)
        .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    if state.manager.is_some() {
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.serial.is_none() && Instant::now() < deadline {
            queue
                .roundtrip(&mut state)
                .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
        }
        // Modes arrive after their heads; one more roundtrip settles them.
        queue
            .roundtrip(&mut state)
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    }
    settle(&mut state);
    Ok((connection, queue, state))
}

fn settle(state: &mut State) {
    // Nested and virtual outputs may advertise a single custom mode without
    // naming it current.
    for (_, output, _) in &mut state.heads {
        if output.current.is_none() && output.enabled && output.modes.len() == 1 {
            output.current = Some(0);
        }
    }
}

fn snapshot(state: &State) -> Displays {
    Displays {
        supported: state.manager.is_some(),
        outputs: state
            .heads
            .iter()
            .map(|(_, output, _)| output.clone())
            // Remote-session virtual outputs are managed by their session.
            .filter(|output| !output.name.starts_with("ENZREMOTE-"))
            .collect(),
    }
}

/// Reads the outputs of the compositor this process is connected to.
pub fn read() -> Displays {
    match connect() {
        Ok((_, _, state)) => snapshot(&state),
        Err(_) => Displays::default(),
    }
}

/// Whether applying `layout` changes the scale of a display that is on now
/// and stays on: the desktop's chrome is laid out again at the new size.
#[must_use]
pub fn rescales(displays: &Displays, layout: &[Change]) -> bool {
    layout.iter().filter(|c| c.enabled).any(|change| {
        displays
            .outputs
            .iter()
            .any(|o| o.name == change.output && o.enabled && (o.scale - change.scale).abs() > 1e-6)
    })
}

/// Applies a whole layout. Outputs it does not name keep their state.
pub fn apply(layout: &[Change]) -> Result<(), String> {
    if layout.iter().any(|c| !(0.5..=4.0).contains(&c.scale)) {
        return Err(tr("shell-scale-is-outside-its-allowed-range").into());
    }
    if !layout.iter().any(|c| c.enabled) {
        return Err(tr("shell-at-least-one-display-must-stay-on").into());
    }
    let (_connection, mut queue, mut state) = connect()?;
    let (Some(manager), Some(serial)) = (state.manager.clone(), state.serial) else {
        return Err(tr("shell-this-compositor-does-not-support-changing-displays").into());
    };
    if layout
        .iter()
        .any(|c| !state.heads.iter().any(|(_, o, _)| o.name == c.output))
    {
        return Err(tr("shell-that-display-is-no-longer-connected").into());
    }
    let qh = queue.handle();
    let configuration = manager.create_configuration(serial, &qh, ());
    for (head, output, modes) in &state.heads {
        let wanted = layout
            .iter()
            .find(|c| c.output == output.name)
            .cloned()
            .or_else(|| Change::of(output).filter(|_| output.enabled));
        let Some(change) = wanted.filter(|c| c.enabled) else {
            configuration.disable_head(head);
            continue;
        };
        let index = output.modes.iter().position(|m| {
            (m.width, m.height, m.refresh_mhz)
                == (
                    change.mode.width,
                    change.mode.height,
                    change.mode.refresh_mhz,
                )
        });
        let Some(index) = index else {
            configuration.destroy();
            return Err(tr("shell-that-resolution-is-no-longer-offered").into());
        };
        let config = configuration.enable_head(head, &qh, ());
        config.set_mode(&modes[index]);
        config.set_scale(change.scale);
        config.set_transform(change.rotation.transform());
        config.set_position(change.position.0, change.position.1);
    }
    configuration.apply();
    let deadline = Instant::now() + Duration::from_secs(3);
    while state.outcome.is_none() && Instant::now() < deadline {
        queue
            .roundtrip(&mut state)
            .map_err(|_| tr("shell-the-display-server-stopped-responding"))?;
    }
    configuration.destroy();
    state
        .outcome
        .unwrap_or_else(|| Err(tr("shell-the-display-did-not-confirm-the-change").into()))
}

/// Restores layouts on connection changes, using the newest received list including at startup.
pub fn restore_on_connect(layouts: std::sync::mpsc::Receiver<Vec<Saved>>, restore_initial: bool) {
    let Ok(mut saved) = layouts.recv() else {
        return;
    };
    let mut connected: Option<Vec<String>> = None;
    // A connection kept open hears every output change as output
    // management's `done`; without one, the outputs are read every 2 s.
    let mut watch = connect().ok();
    loop {
        loop {
            match layouts.try_recv() {
                Ok(next) => saved = next,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }
        let displays = read();
        if !displays.supported {
            return;
        }
        let mut keys: Vec<String> = displays.outputs.iter().map(Output::key).collect();
        keys.sort();
        if connected.as_ref() != Some(&keys) {
            if (restore_initial || connected.is_some())
                && let Some(layout) = restored(&displays, &saved)
            {
                let _ = apply(&layout);
            }
            connected = Some(keys);
        }
        let changed = watch.as_mut().is_some_and(|(_, queue, state)| {
            let before = state.serial;
            while state.serial == before {
                if queue.blocking_dispatch(state).is_err() {
                    return false;
                }
            }
            true
        });
        if !changed {
            watch = None;
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_behaviour() {
        // Modes and outputs describe themselves.
        {
            let mode = Mode {
                width: 2560,
                height: 1440,
                refresh_mhz: 59_951,
                preferred: true,
            };
            assert_eq!(
                (
                    mode.resolution().as_str(),
                    mode.refresh()
                        .replace(['\u{2068}', '\u{2069}'], "")
                        .as_str()
                ),
                ("2560 × 1440", "60 Hz")
            );
            assert_eq!(
                Mode {
                    refresh_mhz: 143_856,
                    ..mode
                }
                .refresh()
                .replace(['\u{2068}', '\u{2069}'], ""),
                "144 Hz"
            );
            let output = Output {
                name: "DP-1".into(),
                make: "Dell".into(),
                model: "U2723QE".into(),
                modes: vec![mode],
                current: Some(0),
                scale: 1.25,
                physical_mm: (597, 336),
                ..Output::default()
            };
            assert_eq!(output.title(), "Dell U2723QE");
            assert_eq!(output.logical_size(), (2048.0, 1152.0));
            assert!((output.diagonal_inches().unwrap() - 27.0).abs() < 0.1);
            assert_eq!(
                Output {
                    name: "WAYLAND-1".into(),
                    ..Output::default()
                }
                .title(),
                "WAYLAND-1"
            );
            assert_eq!(Mode::default().refresh(), "—");
        }
        // Layouts pack edge to edge and follow rotation.
        {
            let displays = two();
            let mut layout = layout(&displays);
            let right = sides(&layout);
            assert_eq!(right, vec![("DP-1".to_owned(), Side::Right)]);
            layout[1].rotation = Rotation::Right;
            pack(&mut layout, &right);
            assert_eq!(layout[1].logical_size(), (1080.0, 1920.0));
            assert_eq!(
                (layout[0].position, layout[1].position),
                ((0, 0), (1000, 0))
            );
            pack(&mut layout, &[("DP-1".to_owned(), Side::Left)]);
            assert_eq!(
                (layout[0].position, layout[1].position),
                ((1080, 0), (0, 0))
            );
            pack(&mut layout, &[("DP-1".to_owned(), Side::Above)]);
            assert_eq!(
                (layout[0].position, layout[1].position),
                ((0, 1920), (0, 0))
            );
            assert_eq!(sides(&layout), vec![("DP-1".to_owned(), Side::Above)]);
        }
        // Remembered layouts restore by monitor and never turn everything off.
        {
            let displays = two();
            let mut wanted = layout(&displays);
            wanted[1].mode = displays.outputs[1].modes[1];
            wanted[1].rotation = Rotation::Left;
            let saved = super::saved(&displays, &wanted);
            assert_eq!(saved[1].key, "Dell|U|7");
            assert_eq!(restored(&displays, &saved), Some(wanted.clone()));
            let mut applied = displays.clone();
            applied.outputs[1].current = Some(1);
            applied.outputs[1].rotation = Rotation::Left;
            assert_eq!(restored(&applied, &saved), None);
            let off: Vec<Saved> = saved
                .iter()
                .cloned()
                .map(|s| Saved {
                    enabled: false,
                    ..s
                })
                .collect();
            assert_eq!(restored(&displays, &off), None);
            let mut moved = displays;
            moved.outputs[1].name = "DP-3".into();
            assert_eq!(
                restored(&moved, &saved).unwrap()[1].rotation,
                Rotation::Left
            );
            assert_eq!(
                Output {
                    name: "HDMI-A-1".into(),
                    ..Output::default()
                }
                .key(),
                "port:HDMI-A-1"
            );

            let mut remembered = saved_from_json(None).unwrap();
            remember(&mut remembered, &saved).unwrap();
            remember(&mut remembered, &saved[1..]).unwrap();
            assert_eq!(remembered.len(), 2);
            assert!(
                remember(
                    &mut remembered,
                    &[Saved {
                        scale: 9.0,
                        ..saved[0].clone()
                    }]
                )
                .is_err()
            );
            let json = serde_json::to_value(&remembered).unwrap();
            assert_eq!(saved_from_json(Some(&json)).unwrap(), remembered);
        }
        // Only a new scale on a display that stays on hides the chrome first.
        {
            let displays = two();
            let mut changed = layout(&displays);
            assert!(!rescales(&displays, &changed));
            changed[1].position = (0, 0);
            assert!(!rescales(&displays, &changed));
            changed[0].scale = 1.25;
            assert!(rescales(&displays, &changed));
            changed[0].enabled = false;
            assert!(!rescales(&displays, &changed));
        }
    }

    fn two() -> Displays {
        let mode = |w, h| Mode {
            width: w,
            height: h,
            refresh_mhz: 60_000,
            preferred: true,
        };
        Displays {
            supported: true,
            outputs: vec![
                Output {
                    name: "eDP-1".into(),
                    make: "BOE".into(),
                    model: "X".into(),
                    serial: "1".into(),
                    enabled: true,
                    modes: vec![mode(2000, 1000)],
                    current: Some(0),
                    scale: 2.0,
                    ..Output::default()
                },
                Output {
                    name: "DP-1".into(),
                    make: "Dell".into(),
                    model: "U".into(),
                    serial: "7".into(),
                    enabled: true,
                    modes: vec![mode(1920, 1080), mode(1280, 720)],
                    current: Some(0),
                    scale: 1.0,
                    position: (1000, 0),
                    ..Output::default()
                },
            ],
        }
    }
}
