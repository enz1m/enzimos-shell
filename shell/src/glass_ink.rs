//! Adaptive glass ink using framework luminance thresholds, dwell timing and crossfades.

use enzui_theme::{GlassInk, MaterialStyle};
use iced::{Color, Rectangle};
use std::time::{Duration, Instant};

const TO_DARK_INK: f32 = 0.56;
const INK_MIDPOINT: f32 = 0.46;
const TO_LIGHT_INK: f32 = 0.38;
const SWITCH_DWELL: Duration = Duration::from_millis(900);
const SETTLE: Duration = Duration::from_millis(600);
const CROSSFADE: f32 = 0.3;
const SAME_SURFACE_JUMP: f32 = 48.0;

/// The adaptive ink of one glass surface, kept across frames.
#[derive(Clone, Debug)]
pub struct AdaptiveInk {
    light_ink: f32,
    target: f32,
    measured: bool,
    settling_since: Option<Instant>,
    crossing_since: Option<Instant>,
    last_frame: Option<Instant>,
    last_seen: Option<f32>,
    bounds: Option<Rectangle>,
}

impl Default for AdaptiveInk {
    fn default() -> Self {
        Self {
            light_ink: 1.0,
            target: 1.0,
            measured: false,
            settling_since: None,
            crossing_since: None,
            last_frame: None,
            last_seen: None,
            bounds: None,
        }
    }
}

impl AdaptiveInk {
    /// Starts afresh when `bounds` is not the surface this was measured for:
    /// a different size, or a jump further than a sliding panel moves.
    pub fn track(&mut self, bounds: Rectangle) {
        let same = self.bounds.is_none_or(|old| {
            (old.width - bounds.width).abs() < 1.0
                && (old.height - bounds.height).abs() < 1.0
                && (old.x - bounds.x).abs() <= SAME_SURFACE_JUMP
                && (old.y - bounds.y).abs() <= SAME_SURFACE_JUMP
        });
        if !same {
            *self = Self::default();
        }
        self.bounds = Some(bounds);
    }

    /// The ink to draw with, `1.0` light to `0.0` dark; `None` before the
    /// first measurement.
    pub fn ink(&self) -> Option<f32> {
        self.measured.then_some(self.light_ink)
    }

    /// Advances adaptive ink; returns whether settling, dwell or crossfade needs another frame.
    pub fn advance(&mut self, now: Instant, luminance: Option<f32>) -> bool {
        let changed = luminance.is_some() && luminance != self.last_seen;
        self.last_seen = luminance;
        if let Some(luminance) = luminance {
            let midpoint = if luminance > INK_MIDPOINT { 0.0 } else { 1.0 };
            if !self.measured {
                // The first reading decides at once: no fade from a guess.
                self.measured = true;
                self.target = midpoint;
                self.light_ink = midpoint;
                self.settling_since = Some(now);
            }
            if let Some(since) = self.settling_since {
                if changed {
                    self.settling_since = Some(now);
                } else {
                    let held = now.duration_since(since);
                    if held >= SETTLE / 4 {
                        self.target = midpoint;
                    }
                    if held >= SETTLE {
                        self.settling_since = None;
                    }
                }
            }
            let wanted = if self.settling_since.is_some() {
                self.target
            } else if self.target >= 0.5 {
                if luminance > TO_DARK_INK { 0.0 } else { 1.0 }
            } else if luminance < TO_LIGHT_INK {
                1.0
            } else {
                0.0
            };
            if wanted == self.target {
                self.crossing_since = None;
            } else {
                let since = *self.crossing_since.get_or_insert(now);
                if now.duration_since(since) >= SWITCH_DWELL {
                    self.target = wanted;
                    self.crossing_since = None;
                }
            }
        }
        let step = self.last_frame.map_or(0.0, |last| {
            now.duration_since(last).as_secs_f32() / CROSSFADE
        });
        self.last_frame = Some(now);
        if self.light_ink < self.target {
            self.light_ink = (self.light_ink + step).min(self.target);
        } else if self.light_ink > self.target {
            self.light_ink = (self.light_ink - step).max(self.target);
        }
        self.crossing_since.is_some()
            || self.settling_since.is_some()
            || (self.light_ink - self.target).abs() > f32::EPSILON
    }
}

/// Returns the style's light-to-dark ink mix; unmeasured adaptive styles return `None`.
pub fn ink(style: MaterialStyle, adaptive: Option<&AdaptiveInk>) -> Option<f32> {
    match style.ink() {
        GlassInk::Adaptive => adaptive.and_then(AdaptiveInk::ink),
        GlassInk::Light => Some(1.0),
        GlassInk::Theme => None,
    }
}

/// Returns content theme and glass tint matching the selected foreground ink.
pub fn look(
    theme: &iced::Theme,
    style: MaterialStyle,
    light_ink: Option<f32>,
) -> (iced::Theme, Color) {
    let palette = enzui_widgets::theme_lookup::palette(theme);
    let own = style.surface_color(&palette);
    match light_ink {
        None => (enzui_widgets::theme_lookup::on_material(theme, style), own),
        Some(t) => {
            let (dark, light) = if palette.is_dark() {
                (own, enzui_theme::Palette::light().surface_elevated)
            } else {
                (enzui_theme::Palette::dark().surface_elevated, own)
            };
            let tint = Color {
                r: light.r + (dark.r - light.r) * t,
                g: light.g + (dark.g - light.g) * t,
                b: light.b + (dark.b - light.b) * t,
                a: 1.0,
            };
            (
                enzui_widgets::theme_lookup::on_material_ink(theme, style, t),
                tint,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_ink_behaviour() {
        // The first reading decides and a switch waits for the dwell then fades.
        {
            let mut ink = AdaptiveInk::default();
            let start = Instant::now();
            assert_eq!(ink.ink(), None);
            ink.advance(start, Some(0.8));
            assert_eq!(ink.ink(), Some(0.0), "light backdrop, dark ink at once");
            ink.advance(start + SETTLE, Some(0.8));
            ink.advance(start + SETTLE + Duration::from_millis(100), Some(0.2));
            assert_eq!(ink.ink(), Some(0.0));
            // Pending switches and crossfades receive 16 ms ticks.
            let switched = start + SETTLE + Duration::from_millis(100) + SWITCH_DWELL;
            let mut at = start + SETTLE + Duration::from_millis(100);
            while at < switched + Duration::from_millis(150) {
                at += Duration::from_millis(16);
                ink.advance(at, Some(0.2));
            }
            let midway = ink.ink().unwrap();
            assert!(midway > 0.3 && midway < 0.7, "crossfading: {midway}");
            while ink.advance(at, Some(0.2)) {
                at += Duration::from_millis(16);
            }
            assert_eq!(ink.ink(), Some(1.0));
        }
        // A backdrop inside the band keeps the ink and a new surface starts afresh.
        {
            let mut ink = AdaptiveInk::default();
            let start = Instant::now();
            ink.advance(start, Some(0.3));
            ink.advance(start + SETTLE, Some(0.3));
            ink.advance(start + SETTLE * 3, Some(0.5));
            ink.advance(start + SETTLE * 6, Some(0.5));
            assert_eq!(ink.ink(), Some(1.0), "0.5 is inside the band");
            ink.track(Rectangle {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            });
            ink.track(Rectangle {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 40.0,
            });
            assert_eq!(ink.ink(), None);
        }
    }
}
