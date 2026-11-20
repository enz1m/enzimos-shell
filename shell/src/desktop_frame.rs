use crate::settings::{Metric, Metrics, SidebarEdge};
use enzui_shell::i18n::tr;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, sync::watch};

pub const THICKNESS: u32 = 3;
pub const RADIUS: u32 = 18;
pub const PANEL_HEIGHT: u32 = 36;
pub const HORIZONTAL_BAND: u32 = 116;
pub const SIDE_BAND: u32 = 64;

/// The fade curve `curve` (see [`Appearance::fade_curve`]) at `t`, 0 to 1:
/// iced's own curves, which the bar's fade takes.
pub fn fade_eased(curve: u8, t: f32) -> f32 {
    use std::f32::consts::PI;
    let t = t.clamp(0.0, 1.0);
    match curve {
        1 => t,
        2 => 1.0 - (t * PI / 2.0).cos(),
        3 => -((PI * t).cos() - 1.0) / 2.0,
        _ => (t * PI / 2.0).sin(),
    }
}
/// How much later than the shell's decision the frame and the rail start
/// their compositor fades, so they fade as one with the bar: going out the
/// bar's first faded frame came with no lag worth one (measured).
pub const FADE_LAG: std::time::Duration = std::time::Duration::ZERO;
/// The same coming in: the bar's window was just reshaped for the new
/// style, and its first faded frame comes a redraw later still.
pub const FADE_IN_LAG: std::time::Duration = std::time::Duration::from_millis(16);
/// The lag for a fade toward `to` (1 shown, 0 gone).
pub fn fade_lag(to: f32) -> std::time::Duration {
    if to > 0.5 { FADE_IN_LAG } else { FADE_LAG }
}
/// Shape coordinates per logical pixel.
pub const SHAPE_UNITS: f32 = 8.0;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    pub light: bool,
    pub surface: Option<[u8; 3]>,
    pub bottom: bool,
    pub sidebar: Option<SidebarEdge>,
    /// The side the rail paints, shown or hidden: a hidden rail still paints
    /// a frame-wide strip of the edge, so the frame leaves that side's band
    /// and corners to it (painted by both, the edge read doubled and faded
    /// out late).
    pub rail: Option<SidebarEdge>,
    pub expanded: bool,
    pub metrics: Metrics,
    /// In islands mode, the bar's floating shapes that cast a shadow:
    /// x, distance from the bar's screen edge, width and height, in
    /// [`SHAPE_UNITS`] of a logical pixel (the islands sit at fractional
    /// positions; whole pixels left a sliver of unshadowed wallpaper beside
    /// them).
    pub shapes: [[u16; 4]; 8],
    pub shape_count: u8,
    /// The optional soft shadow of the bar, islands and frame: 0 (none) to 4.
    pub bar_shadow: u8,
    /// How see-through the frame is, in percent: 0 (opaque) to 50.
    pub transparency: u8,
    /// The whole frame, shadows included, is faded out while the shell
    /// changes style: 0 (shown) or 100 (gone). The helper fades between them
    /// over `fade_ms`, at the display's pace.
    pub hidden: u8,
    /// How long the helper takes to fade, in milliseconds: the bar's fade.
    pub fade_ms: u16,
    /// The fade's curve, the bar's: 0 ease-out, 1 linear, 2 ease-in,
    /// 3 ease-in-out.
    pub fade_curve: u8,
}
impl Appearance {
    pub fn command(self) -> String {
        let floating = self.metrics.islands.map_or(String::new(), |style| {
            let sizes = crate::shell_style::IslandOption::ALL
                .into_iter()
                .map(|key| style.get(key).to_string())
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                " islands {sizes}{}",
                if style.joined_bar { " joined" } else { "" }
            )
        });
        let shapes = if self.shape_count == 0 {
            String::new()
        } else {
            let numbers: Vec<String> = self.shapes[..usize::from(self.shape_count)]
                .iter()
                .flatten()
                .map(u16::to_string)
                .collect();
            format!(" shapes {}", numbers.join(" "))
        };
        let shadow = if self.bar_shadow > 0 {
            format!(" shadow {}", self.bar_shadow.min(4))
        } else {
            String::new()
        };
        let clear = if self.transparency > 0 {
            format!(" clear {}", self.transparency.min(50))
        } else {
            String::new()
        };
        let hide = if self.hidden > 0 {
            format!(" hide {}", self.hidden.min(100))
        } else {
            String::new()
        };
        let fade = if self.fade_ms > 0 {
            format!(" fade {}", self.fade_ms.min(1000))
        } else {
            String::new()
        };
        let rail = match self.rail {
            Some(SidebarEdge::Left) => " rail left",
            Some(SidebarEdge::Right) => " rail right",
            None => "",
        };
        let curve = if self.fade_curve > 0 {
            format!(" curve {}", self.fade_curve.min(3))
        } else {
            String::new()
        };
        format!(
            "{} {} {} {} {} {} {} {} {} {} {}{floating}{clear}{hide}{fade}{curve}{rail}{shadow}{shapes}\n",
            if self.light { "light" } else { "dark" },
            if self.bottom { "bottom" } else { "top" },
            match self.sidebar {
                None => "none",
                Some(SidebarEdge::Left) => "left",
                Some(SidebarEdge::Right) => "right",
            },
            if self.expanded { "expanded" } else { "normal" },
            self.metrics.frame,
            self.metrics.radius,
            self.metrics.bar,
            self.metrics.rail,
            self.metrics.text,
            self.metrics.icon,
            self.surface.map_or_else(
                || "default".into(),
                |[r, g, b]| format!("{r:02x}{g:02x}{b:02x}")
            )
        )
    }
    pub fn parse(command: &[u8]) -> Result<Self, std::io::Error> {
        let bad = || std::io::Error::from(std::io::ErrorKind::InvalidData);
        let text = std::str::from_utf8(command).map_err(|_| bad())?;
        let (text, shape_text) = text.split_once(" shapes ").unwrap_or((text, ""));
        // A ` key N` token, N within `max`, taken out of the command.
        let take = |text: &str, key: &str, max: u8| -> Result<(String, u8), std::io::Error> {
            match text.split_once(key) {
                Some((head, tail)) => {
                    let (value, rest) = tail.split_once(' ').unwrap_or((tail, ""));
                    let value = value
                        .trim()
                        .parse::<u8>()
                        .ok()
                        .filter(|value| (1..=max).contains(value))
                        .ok_or_else(bad)?;
                    Ok((format!("{head} {rest}"), value))
                }
                None => Ok((text.to_owned(), 0)),
            }
        };
        let (text, transparency) = take(text, " clear ", 50)?;
        let (text, hidden) = take(&text, " hide ", 100)?;
        let (text, fade_curve) = take(&text, " curve ", 3)?;
        let (text, rail) = match text.split_once(" rail ") {
            Some((head, tail)) => {
                let (side, rest) = tail.split_once(' ').unwrap_or((tail, ""));
                let side = match side.trim() {
                    "left" => SidebarEdge::Left,
                    "right" => SidebarEdge::Right,
                    _ => return Err(bad()),
                };
                (format!("{head} {rest}"), Some(side))
            }
            None => (text, None),
        };
        let (text, fade_ms) = match text.split_once(" fade ") {
            Some((head, tail)) => {
                let (value, rest) = tail.split_once(' ').unwrap_or((tail, ""));
                let value = value
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|value| (1..=1000).contains(value))
                    .ok_or_else(bad)?;
                (format!("{head} {rest}"), value)
            }
            None => (text, 0),
        };
        let text = text.trim_end();
        let (text, bar_shadow) = match text.rsplit_once(" shadow ") {
            Some((text, level)) => (
                text,
                level
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|level| (1..=4).contains(level))
                    .ok_or_else(bad)?,
            ),
            None => (text, 0),
        };
        let numbers: Vec<u16> = shape_text
            .split_ascii_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| bad())?;
        if !numbers.len().is_multiple_of(4) || numbers.len() > 32 {
            return Err(bad());
        }
        let mut shapes = [[0; 4]; 8];
        for (shape, values) in shapes.iter_mut().zip(numbers.as_chunks::<4>().0.iter()) {
            shape.copy_from_slice(values);
        }
        let shape_count = (numbers.len() / 4) as u8;
        let words: Vec<_> = text.split_ascii_whitespace().collect();
        if !matches!(words.len(), 2 | 4 | 10 | 11 | 19 | 20) {
            return Err(bad());
        }
        let mut metrics = Metrics::default();
        if words.len() >= 10 {
            for (metric, value) in Metric::ALL.into_iter().zip(&words[4..10]) {
                metrics
                    .set(metric, value.parse().map_err(|_| bad())?)
                    .map_err(|_| bad())?;
            }
        }
        if words.len() >= 19 {
            if words[11] != "islands" || (words.len() == 20 && words[19] != "joined") {
                return Err(bad());
            }
            let mut style = crate::shell_style::Islands::default();
            for (key, value) in crate::shell_style::IslandOption::ALL
                .into_iter()
                .zip(&words[12..19])
            {
                style
                    .set(key, value.parse().map_err(|_| bad())?)
                    .map_err(|_| bad())?;
            }
            if metrics.frame != 0
                || metrics.bar != style.height
                || metrics.radius != style.radius
                || metrics.rail != style.rail
            {
                return Err(bad());
            }
            style.joined_bar = words.len() == 20;
            metrics.islands = Some(style);
        }
        if shape_count > 0 && metrics.islands.is_none() {
            return Err(bad());
        }
        Ok(Self {
            shapes,
            shape_count,
            bar_shadow,
            transparency,
            hidden,
            fade_ms,
            fade_curve,
            rail,
            light: match words[0] {
                "light" => true,
                "dark" => false,
                _ => return Err(bad()),
            },
            bottom: match words[1] {
                "bottom" => true,
                "top" => false,
                _ => return Err(bad()),
            },
            sidebar: match words.get(2).copied().unwrap_or("none") {
                "none" => None,
                "left" => Some(SidebarEdge::Left),
                "right" => Some(SidebarEdge::Right),
                _ => return Err(bad()),
            },
            expanded: match words.get(3).copied().unwrap_or("normal") {
                "normal" => false,
                "expanded" => true,
                _ => return Err(bad()),
            },
            metrics,
            surface: match words.get(10).copied().unwrap_or("default") {
                "default" => None,
                rgb if rgb.len() == 6 && rgb.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    let mut color = [0; 3];
                    for (index, channel) in color.iter_mut().enumerate() {
                        *channel = u8::from_str_radix(
                            rgb.get(index * 2..index * 2 + 2).ok_or_else(bad)?,
                            16,
                        )
                        .map_err(|_| bad())?;
                    }
                    Some(color)
                }
                _ => return Err(bad()),
            },
        })
    }
    pub fn edges(self) -> (u32, u32) {
        if let Some(style) = self.metrics.islands {
            return (style.side_gap.into(), style.side_gap.into());
        }
        // Windows stop at the wider of the frame and the rail (see
        // `enzui_sidebar_reservations`).
        let rail = self.metrics.rail.max(self.metrics.frame);
        (
            u32::from(if self.sidebar == Some(SidebarEdge::Left) {
                rail
            } else {
                self.metrics.frame
            }),
            u32::from(if self.sidebar == Some(SidebarEdge::Right) {
                rail
            } else {
                self.metrics.frame
            }),
        )
    }
}
// The bar shadow's strength and reach for its level.
fn shadow_look(a: Appearance) -> (f32, f32) {
    bar_shadow_look(a.bar_shadow, a.light)
}

/// The bar shadow at `level` (1–4) as (opacity next to the edge, reach in
/// logical pixels), falling off quadratically away from the bar.
pub fn bar_shadow_look(level: u8, light: bool) -> (f32, f32) {
    let (dark, lit, reach) = match level {
        1 => (0.16, 0.07, 4.0),
        2 => (0.24, 0.10, 6.0),
        3 => (0.32, 0.14, 10.0),
        _ => (0.42, 0.18, 15.0),
    };
    (if light { lit } else { dark }, reach)
}

/// The islands' shadow at `level` as (opacity next to an island, reach in
/// logical pixels): lighter and wider than the frame's, as a floating
/// shape's. There is no large level here: the frame's large shadow round a
/// small island read as a dark smudge, so islands keep to medium.
pub const ISLAND_SHADOW_MAX: u8 = 3;
pub fn island_shadow_look(level: u8, light: bool) -> (f32, f32) {
    let (dark, lit, reach) = match level.min(ISLAND_SHADOW_MAX) {
        1 => (0.10, 0.05, 6.0),
        2 => (0.15, 0.07, 9.0),
        _ => (0.20, 0.10, 13.0),
    };
    (if light { lit } else { dark }, reach)
}

/// Returns the compositor shadow (blur, offset, opacity), or `None` for shadow level zero.
pub fn bar_glass_shadow(level: u8, light: bool, bottom: bool) -> Option<(f32, f32, f32)> {
    if level == 0 {
        return None;
    }
    // Only islands carry glass. The compositor's shadow starts at half its
    // opacity at the edge and is gone by its blur: the frame's island shadow,
    // full at the edge and gone by its reach, matches it so.
    let (opacity, reach) = island_shadow_look(level, light);
    Some((
        reach,
        if bottom { -1.0 } else { 1.0 },
        (2.0 * opacity).min(1.0),
    ))
}

// The opacity of the islands' shadow at `x` and `near` (the distance from
// the bar's screen edge): soft around each shape, cast away from the edge,
// never over a shape itself.
fn island_shadow(x: f32, near: f32, a: Appearance, scale: f32) -> u32 {
    let (strength, reach) = island_shadow_look(a.bar_shadow, a.light);
    let radius = (f32::from(a.metrics.bar) / 2.0).min(f32::from(a.metrics.radius));
    let mut shade: f32 = 0.0;
    // How much of this pixel lies outside every shape: the shadow fades
    // across an island's antialiased edge instead of stopping a pixel short
    // of it (a light ring showed round each island).
    let mut outside: f32 = 1.0;
    for [sx, sy, sw, sh] in &a.shapes[..usize::from(a.shape_count)] {
        let (x0, y0) = (f32::from(*sx) / SHAPE_UNITS, f32::from(*sy) / SHAPE_UNITS);
        let (hx, hy) = (
            f32::from(*sw) / SHAPE_UNITS / 2.0,
            f32::from(*sh) / SHAPE_UNITS / 2.0,
        );
        let r = radius.min(hx).min(hy);
        let distance = |near: f32| {
            let (qx, qy) = (
                (x - x0 - hx).abs() - hx + r,
                (near - y0 - hy).abs() - hy + r,
            );
            qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
        };
        outside = outside.min((distance(near) * scale + 0.5).clamp(0.0, 1.0));
        if outside <= 0.0 {
            return 0;
        }
        // Cast away from the edge.
        let d = distance(near - 1.0).max(0.0);
        if d < reach {
            let fall = 1.0 - d / reach;
            shade = shade.max(strength * fall * fall);
        }
    }
    (shade * outside * 255.0) as u32
}
pub fn color(light: bool) -> [u8; 3] {
    if light { [248, 248, 249] } else { [32, 33, 36] }
}
pub fn valid_output(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 180
        && !name.starts_with("ENZREMOTE-")
        && !name.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug)]
pub enum Piece {
    Top,
    Bottom,
    Left,
    Right,
}
impl Piece {
    pub fn pixels_for_bar(
        self,
        width: u32,
        height: u32,
        scale: i32,
        appearance: Appearance,
    ) -> Result<Vec<u8>, String> {
        if !(1..=4).contains(&scale) || width == 0 || height == 0 || width > 16384 || height > 16384
        {
            return Err(tr("shell-unsupported-frame-geometry").into());
        }
        let (w, h) = (width * scale as u32, height * scale as u32);
        let length = u64::from(w) * u64::from(h) * 4;
        if length > 16 * 1024 * 1024 {
            return Err(tr("shell-frame-strip-exceeds-its-memory-bound").into());
        }
        let mut pixels = vec![0; length as usize];
        if appearance.metrics.islands.is_some() {
            // Only the floating islands' shadows, beside the bar's edge.
            let bar_strip = matches!(
                (self, appearance.bottom),
                (Self::Top, false) | (Self::Bottom, true)
            );
            if bar_strip && appearance.bar_shadow > 0 && appearance.shape_count > 0 {
                for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let x = (index as u32 % w) as f32 / scale as f32 + 0.5 / scale as f32;
                    let y = (index as u32 / w) as f32 / scale as f32 + 0.5 / scale as f32;
                    let near = if matches!(self, Self::Top) {
                        y
                    } else {
                        height as f32 - y
                    };
                    pixel.copy_from_slice(
                        &(island_shadow(x, near, appearance, scale as f32) << 24).to_ne_bytes(),
                    );
                }
            }
            return Ok(pixels);
        }
        let rgb = appearance
            .surface
            .unwrap_or_else(|| color(appearance.light));
        for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = (index as u32 % w) as f32 / scale as f32;
            let y = (index as u32 / w) as f32 / scale as f32;
            let step = 1.0 / scale as f32;
            let a = if self.corner_pixel(x, y, width as f32, height as f32, appearance) {
                let mut count = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        count += u32::from(self.covered(
                            x + (sx as f32 + 0.5) * step / 4.0,
                            y + (sy as f32 + 0.5) * step / 4.0,
                            width as f32,
                            height as f32,
                            appearance,
                        ));
                    }
                }
                (count * 255 + 8) / 16
            } else {
                if self.covered(
                    x + step * 0.5,
                    y + step * 0.5,
                    width as f32,
                    height as f32,
                    appearance,
                ) {
                    255
                } else {
                    0
                }
            };
            // The frame's own see-through (its shadow keeps its strength).
            let a = a * u32::from(100 - appearance.transparency.min(50)) / 100;
            let channel = |c: u8| (u32::from(c) * a + 127) / 255;
            // Under the frame's inner edge the windows get a soft shadow, so
            // the frame reads above them.
            let shade = if a < 255 && appearance.bar_shadow > 0 {
                self.shadow(
                    x + step * 0.5,
                    y + step * 0.5,
                    width as f32,
                    height as f32,
                    appearance,
                )
            } else {
                0
            };
            let alpha = a + shade * (255 - a) / 255;
            pixel.copy_from_slice(
                &(alpha << 24 | channel(rgb[0]) << 16 | channel(rgb[1]) << 8 | channel(rgb[2]))
                    .to_ne_bytes(),
            );
        }
        Ok(pixels)
    }
    // The shadow's opacity (0–255) at a point of this strip outside the
    // frame: a soft falloff from the inner rounded edge into the window
    // area, slightly longer below the bar.
    fn shadow(self, x: f32, y: f32, width: f32, height: f32, a: Appearance) -> u32 {
        let (strength, reach) = shadow_look(a);
        let t = f32::from(a.metrics.frame);
        let r = f32::from(a.metrics.radius);
        let (l, right) = a.edges();
        let (l, right) = (l as f32, right as f32);
        let top = if a.bottom {
            t
        } else {
            f32::from(a.metrics.bar)
        };
        let bottom = if a.bottom {
            f32::from(a.metrics.bar)
        } else {
            t
        };
        const FAR: f32 = 1.0e5;
        // The window area as a rounded box in this strip's coordinates.
        let (x0, x1, y0, y1) = match self {
            Self::Top => (l, width - right, top, FAR),
            Self::Bottom => (l, width - right, -FAR, height - bottom),
            Self::Left | Self::Right => {
                // The corners belong to the top and bottom strips.
                let band = HORIZONTAL_BAND as f32;
                if y < band || y > height - band {
                    return 0;
                }
                if matches!(self, Self::Left) {
                    (l, FAR, top, height - bottom)
                } else {
                    (-FAR, width - right, top, height - bottom)
                }
            }
        };
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        let outside = |y: f32| {
            let (qx, qy) = ((x - cx).abs() - hx + r, (y - cy).abs() - hy + r);
            qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
        };
        // Only the window area is shaded (not what the shell's bar covers).
        if outside(y) > 0.0 {
            return 0;
        }
        // Cast slightly away from the bar; points the offset pushes past the
        // edge sit right under it.
        let depth = (-outside(if matches!(self, Self::Bottom) {
            y + 1.0
        } else {
            y - 1.0
        }))
        .max(0.0);
        if depth >= reach {
            return 0;
        }
        let fall = 1.0 - depth / reach;
        (strength * fall * fall * 255.0) as u32
    }
    fn corner_pixel(self, x: f32, y: f32, width: f32, height: f32, a: Appearance) -> bool {
        if matches!(self, Self::Left | Self::Right) {
            return false;
        }
        let near = if matches!(self, Self::Top) {
            y
        } else {
            height - y - 1.0
        };
        let bar = matches!((self, a.bottom), (Self::Top, false) | (Self::Bottom, true));
        if bar && a.expanded {
            return false;
        }
        let base = f32::from(if bar { a.metrics.bar } else { a.metrics.frame });
        let r = f32::from(a.metrics.radius);
        let (l, right) = a.edges();
        let l = l as f32;
        let right = width - right as f32;
        near >= base - 1.0
            && near <= base + r
            && ((x >= l - 1.0 && x <= l + r) || (x >= right - r - 1.0 && x <= right))
    }
    fn covered(self, x: f32, y: f32, width: f32, height: f32, a: Appearance) -> bool {
        let t = f32::from(a.metrics.frame);
        let r = f32::from(a.metrics.radius);
        let (l, right) = a.edges();
        let l = l as f32;
        let right = right as f32;
        if matches!(self, Self::Left | Self::Right) {
            let top = if a.bottom {
                t
            } else {
                f32::from(a.metrics.bar)
            };
            let bottom = if a.bottom {
                f32::from(a.metrics.bar)
            } else {
                t
            };
            // The rail paints its own band, in the frame's colour, with its
            // icons above: painted here too, the frame covered their edge.
            let railed = match self {
                Self::Left => a.rail == Some(SidebarEdge::Left),
                _ => a.rail == Some(SidebarEdge::Right),
            };
            if railed {
                return false;
            }
            let edge = if matches!(self, Self::Left) { l } else { right };
            let x = if matches!(self, Self::Left) {
                x
            } else {
                width - x
            };
            // The top and bottom strips own the rows they span: painted by
            // both, a see-through frame showed doubled corners.
            let band = HORIZONTAL_BAND as f32;
            return y >= top.max(band)
                && y < height - bottom.max(band)
                && x >= edge - t
                && x < edge;
        }
        let near = if matches!(self, Self::Top) {
            y
        } else {
            height - y
        };
        let bar = matches!((self, a.bottom), (Self::Top, false) | (Self::Bottom, true));
        let base = if bar { f32::from(a.metrics.bar) } else { t };
        if near < base {
            return !bar;
        }
        // Beside a rail the rail paints the side's band below the bar.
        let left_rail = a.rail == Some(SidebarEdge::Left);
        let right_rail = a.rail == Some(SidebarEdge::Right);
        if (x >= l - t && x < l && !left_rail)
            || (x >= width - right && x < width - right + t && !right_rail)
        {
            return true;
        }
        if near >= base + r || (bar && a.expanded) {
            return false;
        }
        // The rail rounds its own corners into the desktop.
        let cx = if x >= l && x < l + r && !left_rail {
            l + r
        } else if x >= width - right - r && x < width - right && !right_rail {
            width - right - r
        } else {
            return false;
        };
        (x - cx).powi(2) + (near - base - r).powi(2) > r * r
    }
}

pub async fn run(
    executable: &Path,
    output: &str,
    mut appearance: watch::Receiver<Appearance>,
) -> Result<(), String> {
    if !executable.is_absolute() || !valid_output(output) {
        return Err(tr("shell-invalid-desktop-frame-target").into());
    }
    let initial = *appearance.borrow_and_update();
    let mut child = Command::new(executable)
        .args([output, initial.command().trim()])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| tr("shell-could-not-start-desktop-frame"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| tr("shell-missing-desktop-frame-input"))?;
    loop {
        tokio::select! {
            result=child.wait()=>{result.map_err(|_|tr("shell-could-not-wait-for-desktop-frame"))?;return Err(tr("shell-desktop-frame-stopped").into());}
            changed=appearance.changed()=>{
                if changed.is_err(){drop(input);if tokio::time::timeout(Duration::from_secs(1),child.wait()).await.is_err(){let _=child.kill().await;}return Ok(());}
                let next=*appearance.borrow_and_update();
                tokio::time::timeout(Duration::from_millis(500),input.write_all(next.command().as_bytes())).await.map_err(|_|tr("shell-desktop-frame-update-timed-out"))?.map_err(|_|tr("shell-desktop-frame-input-closed"))?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn alpha(p: &[u8], w: usize, x: usize, y: usize) -> u32 {
        u32::from_ne_bytes(p[(y * w + x) * 4..(y * w + x + 1) * 4].try_into().unwrap()) >> 24
    }

    #[tokio::test]
    async fn desktop_frame_behaviour() {
        // Desktop frame behaviour.
        {
            // Every edge and size roundtrips and mirrors in place.
            {
                for frame in [0, 3, 12] {
                    for radius in [0, 18, 32] {
                        for sidebar in [None, Some(SidebarEdge::Left), Some(SidebarEdge::Right)] {
                            let a = Appearance {
                                sidebar,
                                metrics: Metrics {
                                    frame,
                                    radius,
                                    bar: 48,
                                    rail: 48,
                                    ..Default::default()
                                },
                                ..Default::default()
                            };
                            for bottom in [false, true] {
                                let a = Appearance { bottom, ..a };
                                assert_eq!(
                                    Appearance::parse(a.command().trim().as_bytes()).unwrap(),
                                    a
                                );
                                // The fade travels with the appearance, on the bar's
                                // curve, and so does the side a hidden rail paints.
                                let fading = Appearance {
                                    rail: sidebar,
                                    hidden: 100,
                                    fade_ms: 120,
                                    fade_curve: 3,
                                    transparency: 20,
                                    ..a
                                };
                                assert_eq!(
                                    Appearance::parse(fading.command().trim().as_bytes()).unwrap(),
                                    fading
                                );
                                // Each pixel has one painter, or a see-through frame
                                // doubles: the sides leave the top and bottom strips'
                                // rows to them.
                                for y in (0..HORIZONTAL_BAND).step_by(5) {
                                    for x in 0..SIDE_BAND {
                                        for side in [Piece::Left, Piece::Right] {
                                            for far in [false, true] {
                                                let y = if far { 900 - 1 - y } else { y } as f32;
                                                assert!(!side.covered(
                                                    x as f32,
                                                    y,
                                                    SIDE_BAND as f32,
                                                    900.0,
                                                    a
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            let top = Piece::Top
                                .pixels_for_bar(240, HORIZONTAL_BAND, 1, a)
                                .unwrap();
                            let bottom = Piece::Bottom
                                .pixels_for_bar(
                                    240,
                                    HORIZONTAL_BAND,
                                    1,
                                    Appearance { bottom: true, ..a },
                                )
                                .unwrap();
                            for y in 0..HORIZONTAL_BAND as usize {
                                for x in 0..240 {
                                    assert_eq!(
                                        alpha(&top, 240, x, y),
                                        alpha(&bottom, 240, x, HORIZONTAL_BAND as usize - 1 - y)
                                    );
                                }
                            }
                        }
                    }
                }
                assert!(Appearance::parse(b"dark top left wrong").is_err());
                assert!(Appearance::parse(b"dark top left normal 99 18 36 36 13 16").is_err());
                assert_eq!(
                    Appearance::parse(b"dark top").unwrap(),
                    Appearance::default()
                );
            }
            // Rendering and targets are bounded.
            {
                assert!(
                    Piece::Top
                        .pixels_for_bar(16384, 16384, 4, Appearance::default())
                        .is_err()
                );
                assert!(
                    Piece::Top
                        .pixels_for_bar(1, 1, 0, Appearance::default())
                        .is_err()
                );
                for curve in 0..=3 {
                    assert_eq!(fade_eased(curve, 0.0), 0.0);
                    assert!((fade_eased(curve, 1.0) - 1.0).abs() < 1e-6);
                }
                // Ease-out, the theme's default, is iced's: ahead of linear early on.
                assert!((fade_eased(0, 0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
                assert!(!valid_output("ENZREMOTE-APP"));
                assert!(!valid_output(""));
                assert!(valid_output("WAYLAND-1"));
            }
        }

        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("frame");
        let log = temp.path().join("input");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nexec cat >> '{}'\n",
                log.display(),
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, rx) = watch::channel(Appearance::default());
        let task = tokio::spawn(async move { run(&exe, "WAYLAND-1", rx).await });
        for _ in 0..100 {
            if log.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let next = Appearance {
            light: true,
            bottom: true,
            ..Default::default()
        };
        tx.send_replace(next);
        for _ in 0..100 {
            if std::fs::read_to_string(&log)
                .unwrap()
                .contains(&next.command())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(tx);
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(log).unwrap(),
            format!(
                "WAYLAND-1 {}{}",
                Appearance::default().command(),
                next.command()
            )
        );
    }
}
