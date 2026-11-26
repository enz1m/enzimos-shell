use enzui_shell::{i18n::tr, surface_scale::Scale};
use std::time::Duration;

/// Milliseconds the preview takes to travel from the window to the work area.
pub const GROW: Duration = Duration::from_millis(140);
/// Milliseconds the preview takes to fade out once the gesture is abandoned.
pub const FADE: Duration = Duration::from_millis(110);
/// Body strength of the maximise wash: translucent so the dragged window
/// stays visible underneath while the target edge reads.
pub const BODY_WASH: f32 = 0.22;
pub const MAX_EDGE: u32 = 16384;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_EDGE
            && self.height <= MAX_EDGE
            && self.x > -(MAX_EDGE as i32)
            && self.y > -(MAX_EDGE as i32)
            && self.x < MAX_EDGE as i32
            && self.y < MAX_EDGE as i32
    }
    fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            x: mix(self.x as f32, other.x as f32).round() as i32,
            y: mix(self.y as f32, other.y as f32).round() as i32,
            width: mix(self.width as f32, other.width as f32).round().max(1.0) as u32,
            height: mix(self.height as f32, other.height as f32)
                .round()
                .max(1.0) as u32,
        }
    }
}

// Decelerating curve: fast at the start so the preview reacts to the gesture,
// settling slowly so it does not overshoot the work area.
fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn fraction(elapsed: Duration, total: Duration) -> f32 {
    if total.is_zero() {
        return 1.0;
    }
    (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Growing,
    Fading,
}

/// The preview's visible state. `from` is the window the gesture started on and
/// `to` is the work area it would maximise into.
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    from: Rect,
    to: Rect,
    phase: Phase,
    elapsed: Duration,
    // Opacity the fade started from, so abandoning a partly grown preview does
    // not jump to fully opaque before disappearing.
    fade_base: f32,
}
impl Animation {
    pub fn new(from: Rect, to: Rect) -> Self {
        Self {
            from,
            to,
            phase: Phase::Growing,
            elapsed: Duration::ZERO,
            fade_base: 1.0,
        }
    }
    pub fn advance(&mut self, delta: Duration) {
        self.elapsed = self.elapsed.saturating_add(delta);
    }
    /// Begins the fade-out. The preview keeps the geometry it had reached.
    pub fn release(&mut self) {
        if self.phase == Phase::Fading {
            return;
        }
        self.fade_base = self.opacity();
        self.from = self.rect();
        self.phase = Phase::Fading;
        self.elapsed = Duration::ZERO;
    }
    pub fn fading(&self) -> bool {
        self.phase == Phase::Fading
    }
    pub fn finished(&self) -> bool {
        self.phase == Phase::Fading && self.elapsed >= FADE
    }
    pub fn rect(&self) -> Rect {
        match self.phase {
            Phase::Growing => self
                .from
                .lerp(self.to, ease_out(fraction(self.elapsed, GROW))),
            Phase::Fading => self.from,
        }
    }
    pub fn opacity(&self) -> f32 {
        match self.phase {
            Phase::Growing => ease_out(fraction(self.elapsed, GROW)),
            Phase::Fading => self.fade_base * (1.0 - fraction(self.elapsed, FADE)),
        }
    }
}

/// Premultiplied ARGB pixels for the preview: a filled, rounded rectangle with
/// a brighter border, matching the shell's accent.
fn pixels(
    (width, height): (u32, u32),
    (w, h): (u32, u32),
    radius: f32,
    accent: [u8; 3],
    opacity: f32,
    body_alpha: f32,
) -> Result<Vec<u8>, String> {
    let length = u64::from(w) * u64::from(h) * 4;
    if length > 64 * 1024 * 1024 {
        return Err(tr("shell-preview-exceeds-its-memory-bound").into());
    }
    let opacity = opacity.clamp(0.0, 1.0);
    let radius = radius.clamp(0.0, (width.min(height) as f32) / 2.0);
    let border = 2.0_f32;
    let mut pixels = vec![0; length as usize];
    let (fw, fh) = (width as f32, height as f32);
    let (step_x, step_y) = (fw / w as f32, fh / h as f32);
    let step = step_x.max(step_y);
    // Rounded-rectangle signed distance: negative inside, positive outside.
    let distance = |x: f32, y: f32| {
        let dx = (radius - x).max(x - (fw - radius)).max(0.0);
        let dy = (radius - y).max(y - (fh - radius)).max(0.0);
        if dx > 0.0 && dy > 0.0 {
            (dx * dx + dy * dy).sqrt() - radius
        } else {
            -x.min(fw - x).min(y).min(fh - y)
        }
    };
    // Cache the two interior colours; only edge pixels need distance-field blending.
    let encode = |coverage: f32, border_mix: f32| -> [u8; 4] {
        let alpha = (body_alpha + (0.85 - body_alpha) * border_mix) * coverage * opacity;
        let lift = |c: u8| {
            let c = f32::from(c);
            (c + (255.0 - c) * 0.25 * border_mix).min(255.0)
        };
        let a = (alpha * 255.0).round().clamp(0.0, 255.0) as u32;
        let channel = |c: f32| ((c * alpha).round().clamp(0.0, 255.0) as u32) & 0xff;
        (a << 24
            | channel(lift(accent[0])) << 16
            | channel(lift(accent[1])) << 8
            | channel(lift(accent[2])))
        .to_ne_bytes()
    };
    let body_pixel = encode(1.0, 0.0);
    let border_pixel = encode(1.0, 1.0);
    // Rows that contain no corner curve are spans: border, body, border.
    let straight_rows = (radius.ceil() / step_y).ceil() as u32;
    let inset_x = (border / step_x).ceil() as u32;
    let inset_y = (border / step_y).ceil() as u32;
    for row in 0..h {
        if row < straight_rows || row >= h.saturating_sub(straight_rows) {
            continue;
        }
        let start = (row * w * 4) as usize;
        let full = row < inset_y || row >= h.saturating_sub(inset_y);
        let line = &mut pixels[start..start + (w * 4) as usize];
        if full {
            for pixel in line.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&border_pixel);
            }
            continue;
        }
        for (column, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let column = column as u32;
            if column < inset_x || column >= w.saturating_sub(inset_x) {
                pixel.copy_from_slice(&border_pixel);
            } else {
                pixel.copy_from_slice(&body_pixel);
            }
        }
    }
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let row = index as u32 / w;
        if row >= straight_rows && row < h.saturating_sub(straight_rows) {
            continue;
        }
        let px = ((index as u32 % w) as f32 + 0.5) * step_x;
        let py = (row as f32 + 0.5) * step_y;
        let centre = distance(px, py);
        // Supersample only near edges to keep window-sized previews within the frame budget.
        let ambiguous = centre.abs() <= step || (centre <= 0.0 && (-centre - border).abs() <= step);
        let (coverage, border_mix) = if ambiguous {
            let mut inside = 0u32;
            let mut on_border = 0u32;
            let samples = 4;
            for sy in 0..samples {
                for sx in 0..samples {
                    let ox = px + ((sx as f32 + 0.5) / samples as f32 - 0.5) * step_x;
                    let oy = py + ((sy as f32 + 0.5) / samples as f32 - 0.5) * step_y;
                    let d = distance(ox, oy);
                    if d <= 0.0 {
                        inside += 1;
                        if -d <= border {
                            on_border += 1;
                        }
                    }
                }
            }
            let total = (samples * samples) as f32;
            (inside as f32 / total, on_border as f32 / total)
        } else if centre <= 0.0 {
            (1.0, if -centre <= border { 1.0 } else { 0.0 })
        } else {
            (0.0, 0.0)
        };
        if coverage <= 0.0 {
            continue;
        }
        // The border is close to solid so the edge stays legible over any
        // wallpaper while the maximise wash remains translucent.
        let alpha = (body_alpha + (0.85 - body_alpha) * border_mix) * coverage * opacity;
        let lift = |c: u8| {
            let c = f32::from(c);
            (c + (255.0 - c) * 0.25 * border_mix).min(255.0)
        };
        let a = (alpha * 255.0).round().clamp(0.0, 255.0) as u32;
        let channel = |c: f32| ((c * alpha).round().clamp(0.0, 255.0) as u32) & 0xff;
        pixel.copy_from_slice(
            &(a << 24
                | channel(lift(accent[0])) << 16
                | channel(lift(accent[1])) << 8
                | channel(lift(accent[2])))
            .to_ne_bytes(),
        );
    }
    Ok(pixels)
}

/// Draws into a fixed-size surface; per-frame layer-surface resizes would lag compositor
/// configure replies.
#[allow(
    clippy::too_many_arguments,
    reason = "The view composes independent state and actions supplied by its caller."
)]
pub fn frame_pixels(
    surface_width: u32,
    surface_height: u32,
    scale: Scale,
    rect: Rect,
    origin: (i32, i32),
    radius: f32,
    accent: [u8; 3],
    opacity: f32,
    body_alpha: f32,
) -> Result<Vec<u8>, String> {
    if !scale.supported()
        || surface_width == 0
        || surface_height == 0
        || surface_width > MAX_EDGE
        || surface_height > MAX_EDGE
    {
        return Err(tr("shell-unsupported-preview-surface").into());
    }
    let (w, h) = scale.physical((surface_width, surface_height));
    let length = u64::from(w) * u64::from(h) * 4;
    if length > 256 * 1024 * 1024 {
        return Err(tr("shell-preview-surface-exceeds-its-memory-bound").into());
    }
    let mut surface = vec![0u8; length as usize];
    if opacity <= 0.0 || !rect.valid() {
        return Ok(surface);
    }
    // Snap both edges in viewport coordinates so rounding the origin cannot shift the far edge.
    let pixel_edge = |edge: i64, physical: u32, logical: u32| {
        (edge as f64 * f64::from(physical) / f64::from(logical)).round() as i64
    };
    let x = i64::from(rect.x) - i64::from(origin.0);
    let y = i64::from(rect.y) - i64::from(origin.1);
    let left = pixel_edge(x, w, surface_width);
    let top = pixel_edge(y, h, surface_height);
    let tile_w = pixel_edge(x + i64::from(rect.width), w, surface_width) - left;
    let tile_h = pixel_edge(y + i64::from(rect.height), h, surface_height) - top;
    if tile_w == 0 || tile_h == 0 {
        return Ok(surface);
    }
    let tile = pixels(
        (rect.width, rect.height),
        (tile_w as u32, tile_h as u32),
        radius,
        accent,
        opacity,
        body_alpha,
    )?;
    // Rows are contiguous in both buffers, so each visible row is copied as one
    // slice rather than a pixel at a time.
    for row in 0..tile_h {
        let dy = row + top;
        if dy < 0 || dy >= i64::from(h) {
            continue;
        }
        let first = (-left).clamp(0, tile_w);
        let last = (i64::from(w) - left).clamp(0, tile_w);
        if first >= last {
            continue;
        }
        let span = ((last - first) * 4) as usize;
        let source = ((row * tile_w + first) * 4) as usize;
        let destination = ((dy * i64::from(w) + first + left) * 4) as usize;
        surface[destination..destination + span].copy_from_slice(&tile[source..source + span]);
    }
    Ok(surface)
}

/// `show x y w h ax ay aw ah rrggbb radius` begins a maximize preview;
/// `hide` fades it out. Restore drags never send a preview command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Show {
        from: Rect,
        to: Rect,
        accent: [u8; 3],
        radius: u16,
    },
    Hide,
}
impl Command {
    pub fn encode(self) -> String {
        match self {
            Self::Hide => "hide\n".into(),
            Self::Show {
                from,
                to,
                accent,
                radius,
            } => format!(
                "show {} {} {} {} {} {} {} {} {:02x}{:02x}{:02x} {}\n",
                from.x,
                from.y,
                from.width,
                from.height,
                to.x,
                to.y,
                to.width,
                to.height,
                accent[0],
                accent[1],
                accent[2],
                radius
            ),
        }
    }
    pub fn parse(line: &[u8]) -> Result<Self, String> {
        let text =
            std::str::from_utf8(line).map_err(|_| tr("shell-preview-command-is-not-text"))?;
        let words: Vec<_> = text.split_ascii_whitespace().collect();
        match words.first().copied() {
            Some("hide") if words.len() == 1 => Ok(Self::Hide),
            Some("show") if words.len() == 11 => Ok(Self::Show {
                from: parse_rect(&words, 1)?,
                to: parse_rect(&words, 5)?,
                accent: parse_accent(words[9])?,
                radius: parse_radius(words[10])?,
            }),
            _ => Err(tr("shell-preview-command-is-malformed").into()),
        }
    }
}

fn parse_number(words: &[&str], index: usize) -> Result<i64, String> {
    words[index]
        .parse::<i64>()
        .map_err(|_| tr("shell-preview-command-has-an-invalid-number").to_owned())
}

fn parse_edge(words: &[&str], index: usize) -> Result<u32, String> {
    let value = parse_number(words, index)?;
    u32::try_from(value)
        .ok()
        .filter(|v| *v > 0 && *v <= MAX_EDGE)
        .ok_or_else(|| tr("shell-preview-command-has-an-invalid-size").to_owned())
}

fn parse_position(words: &[&str], index: usize) -> Result<i32, String> {
    let value = parse_number(words, index)?;
    i32::try_from(value)
        .ok()
        .filter(|v| v.abs() <= MAX_EDGE as i32)
        .ok_or_else(|| tr("shell-preview-command-has-an-invalid-position").to_owned())
}

fn parse_rect(words: &[&str], index: usize) -> Result<Rect, String> {
    Ok(Rect {
        x: parse_position(words, index)?,
        y: parse_position(words, index + 1)?,
        width: parse_edge(words, index + 2)?,
        height: parse_edge(words, index + 3)?,
    })
}

fn parse_accent(word: &str) -> Result<[u8; 3], String> {
    if word.len() != 6 || !word.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(tr("shell-preview-command-has-an-invalid-colour").into());
    }
    let mut accent = [0; 3];
    for (index, channel) in accent.iter_mut().enumerate() {
        *channel = u8::from_str_radix(
            word.get(index * 2..index * 2 + 2)
                .ok_or_else(|| tr("shell-preview-command-has-an-invalid-colour"))?,
            16,
        )
        .map_err(|_| tr("shell-preview-command-has-an-invalid-colour"))?;
    }
    Ok(accent)
}

fn parse_radius(word: &str) -> Result<u16, String> {
    let radius = word
        .parse::<u16>()
        .map_err(|_| tr("shell-preview-command-has-an-invalid-radius"))?;
    if radius > 512 {
        return Err(tr("shell-preview-command-has-an-invalid-radius").into());
    }
    Ok(radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn fractional_preview_snaps_both_edges_of_a_clipped_rectangle() {
        let pixels = frame_pixels(
            10,
            9,
            Scale::Fractional(180),
            rect(1, -1, 1, 10),
            (0, 0),
            0.0,
            [255, 255, 255],
            1.0,
            BODY_WASH,
        )
        .unwrap();
        assert_eq!(pixels.len(), 15 * 14 * 4);
        for row in pixels.as_chunks::<{ 15 * 4 }>().0 {
            let painted = row
                .as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .filter_map(|(column, pixel)| (*pixel != [0; 4]).then_some(column))
                .collect::<Vec<_>>();
            assert_eq!(painted, [2]);
        }
    }

    #[test]
    fn show_and_hide_commands_round_trip_and_malformed_input_is_refused() {
        let command = Command::Show {
            from: rect(300, -20, 400, 300),
            to: rect(0, 50, 1440, 850),
            accent: [0x4a, 0x90, 0xd9],
            radius: 12,
        };
        let encoded = command.encode();
        assert!(encoded.ends_with('\n'));
        assert_eq!(
            Command::parse(encoded.trim_end().as_bytes()).unwrap(),
            command
        );
        assert_eq!(Command::parse(b"hide").unwrap(), Command::Hide);
        for bad in [
            &b"show"[..],
            b"show 1 2 3 4 5 6 7 8 zzzzzz 12",
            b"show 1 2 0 4 5 6 7 8 4a90d9 12",
            b"show 1 2 3 4 5 6 7 8 4a90d9 99999",
            b"hide now",
            b"maximise",
            b"morph 1 2 3 4 4a90d9 12 255",
            b"",
        ] {
            assert!(
                Command::parse(bad).is_err(),
                "accepted malformed command {:?}",
                std::str::from_utf8(bad)
            );
        }
    }
}
