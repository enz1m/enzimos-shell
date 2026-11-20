use crate::ui::Message;
use enzui_core::semantics::{Role, Semantics};
use enzui_shell::i18n;
use enzui_shell::session_preferences::{ClockStyle, LockClock, LockLayout};
use enzui_widgets::{
    adaptive_glass::{AdaptiveGlass, GlassSilhouette, enz_adaptive_glass},
    semantics_container::SemanticsContainer,
};
use iced::{
    Alignment, Color, Element, Size,
    widget::{column, image, text},
};
use std::sync::Arc;

pub(crate) fn view(
    prefs: &LockClock,
    clock_24h: bool,
    width: f32,
    wake: f32,
    alpha: f32,
    stacked: bool,
    light: bool,
) -> Element<'static, Message> {
    let now = chrono::Local::now();
    let time = now
        .format(match (clock_24h, stacked) {
            (true, true) => "%H\n%M",
            (false, true) => "%I\n%M",
            (true, false) => "%H:%M",
            (false, false) => "%I:%M",
        })
        .to_string();
    let ink = |opacity| {
        if light {
            Color::from_rgba(0.025, 0.035, 0.05, opacity * alpha)
        } else {
            Color::from_rgba(0.97, 0.95, 0.98, opacity * alpha)
        }
    };
    let style = prefs.material().style();
    let font = enzui_shell::lock_fonts::font(
        prefs.font_family(),
        prefs.weight.saturating_sub(100).max(400),
    );
    let mut face = column![]
        .align_x(Alignment::Center)
        .spacing(6.0 - 2.0 * wake)
        .width(width);
    if prefs.show_date {
        let size = 24.0 - 8.0 * wake;
        let short = width < size * 12.5;
        face = face.push(
            text(i18n::clock_date(now.date_naive(), short))
                .size(size)
                .line_height(1.1)
                .font(font)
                .color(ink(0.94)),
        );
    }
    if prefs.show_time {
        let graphic = graphic_font(&time, prefs.font_family(), prefs.weight);
        let fill = if light {
            Color::from_rgb8(8, 12, 20)
        } else {
            Color::from_rgb8(246, 244, 250)
        };
        let clock: Element<'static, Message> = if let Some(preset) = glass_preset(style) {
            enz_adaptive_glass(std::sync::Arc::clone(&graphic.silhouette), preset)
                .width(width)
                .alpha(alpha)
                .fallback_color(fill)
                .into()
        } else {
            image(graphic.simple_image(light))
                .width(width)
                .height(width / graphic.silhouette.aspect_ratio())
                .content_fit(iced::ContentFit::Fill)
                .opacity(alpha)
                .into()
        };
        let spoken = i18n::time(&now, clock_24h);
        face = face.push(SemanticsContainer::leaf(
            clock,
            Semantics::new(Role::Label).label(spoken),
        ));
        if !clock_24h {
            face = face.push(
                text(i18n::current().text(if chrono::Timelike::hour(&now) < 12 {
                    "enzui-time-am"
                } else {
                    "enzui-time-pm"
                }))
                .size(12)
                .font(font)
                .color(ink(0.7)),
            );
        }
    }
    face.into()
}

pub(crate) fn geometry(
    output: Size,
    prefs: &LockClock,
    wake: f32,
    keyboard: f32,
) -> (f32, f32, f32) {
    let size = (prefs.size.factor() / 0.18).sqrt();
    let wide = output.width >= 680.0;
    let stacked = prefs.stacked(output.width);
    let hero = if stacked {
        (output.height * 0.31 * size).min(output.width * 0.38)
    } else {
        (output.width * 0.80 * size).min(output.height * 1.40)
    };
    let hero = if prefs.layout == LockLayout::Corner && wide {
        hero.min(output.width * 0.50)
    } else {
        hero
    };
    let aspect = enzui_shell::lock_fonts::aspect(prefs.font_family(), prefs.weight, stacked);
    let hero = hero
        .max(100.0)
        .min(output.width - 40.0)
        .min(output.height * 0.48 * aspect);
    let small = if stacked {
        (hero * 0.80).min(output.height * 0.25)
    } else {
        (hero * 0.40).clamp(130.0, 340.0).min(output.width - 48.0)
    };
    let small = small.min(output.height * 0.32 * aspect);
    let width = hero + (small - hero) * wake;
    let center = match prefs.layout {
        LockLayout::Split if wide => output.width * 0.74,
        LockLayout::Corner if wide => output.width * (0.32 - 0.04 * wake),
        _ => output.width * 0.5,
    };
    let top = match prefs.layout {
        LockLayout::Split if wide => output.height * (0.13 + 0.04 * wake),
        LockLayout::Stacked => output.height * (0.10 - 0.08 * wake),
        LockLayout::Corner if wide => output.height * (0.18 - 0.08 * wake),
        _ => output.height * (0.265 - 0.205 * wake),
    };
    let left = center - width * 0.5;
    let compact = if wide {
        (small * 0.62).clamp(100.0, 190.0)
    } else {
        110.0
    };
    let compact = compact.min(output.height * 0.28 * aspect);
    let final_width = width + (compact - width) * keyboard;
    let final_center = if wide {
        24.0 + compact * 0.5
    } else {
        output.width * 0.5
    };
    (
        left + (final_center - compact * 0.5 - left) * keyboard,
        top + (18.0 - top) * keyboard,
        final_width,
    )
}

fn glass_preset(style: ClockStyle) -> Option<AdaptiveGlass> {
    match style {
        ClockStyle::Glass => Some(AdaptiveGlass::Clear),
        ClockStyle::FrostedGlass => Some(AdaptiveGlass::Frosted),
        _ => None,
    }
}

#[cfg(feature = "greeter-preview")]
pub(crate) fn probe(style: ClockStyle, width: f32) -> Element<'static, Message> {
    static PILL: std::sync::OnceLock<Arc<GlassSilhouette>> = std::sync::OnceLock::new();
    let silhouette = PILL.get_or_init(|| Arc::new(GlassSilhouette::from_svg(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 360 128'><rect x='1' y='1' width='358' height='126' rx='63' fill='white'/></svg>",
        360.0, 128.0,
    ).expect("comparison pill")));
    SemanticsContainer::leaf(
        enz_adaptive_glass(
            std::sync::Arc::clone(silhouette),
            glass_preset(style).unwrap_or(AdaptiveGlass::Clear),
        )
        .width(width)
        .fallback_color(Color::from_rgb8(235, 237, 245)),
        Semantics::new(Role::Image).label(enzui_shell::i18n::tr(
            "shell-comparison-pill-using-the-clock-lens",
        )),
    )
    .into()
}

struct Graphic {
    silhouette: Arc<GlassSilhouette>,
    document: String,
    simple: std::sync::OnceLock<[image::Handle; 2]>,
}

impl Graphic {
    fn simple_image(&self, light: bool) -> image::Handle {
        self.simple.get_or_init(|| {
            let aspect = self.silhouette.aspect_ratio();
            let width = 1536.0_f32
                .min(1536.0 * aspect)
                .min((480_000.0 * aspect).sqrt())
                .floor()
                .max(1.0) as u32;
            let height = (width as f32 / aspect).ceil() as u32;
            let tree =
                resvg::usvg::Tree::from_str(&self.document, &resvg::usvg::Options::default())
                    .expect("clock vector");
            let mut pixels = resvg::tiny_skia::Pixmap::new(width, height).expect("clock texture");
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(
                    width as f32 / tree.size().width(),
                    height as f32 / tree.size().height(),
                ),
                &mut pixels.as_mut(),
            );
            [[246, 244, 250], [8, 12, 20]].map(|[r, g, b]| {
                image::Handle::from_rgba(
                    width,
                    height,
                    pixels
                        .pixels()
                        .iter()
                        .flat_map(|p| [r, g, b, p.alpha()])
                        .collect::<Vec<_>>(),
                )
            })
        })[usize::from(light)]
        .clone()
    }
}

fn graphic_font(time: &str, family: &str, weight: u16) -> Arc<Graphic> {
    static CACHE: std::sync::Mutex<Option<(String, Arc<Graphic>)>> = std::sync::Mutex::new(None);
    let key = format!("{family}/{weight}/{time}");
    let mut cache = CACHE.lock().expect("clock cache poisoned");
    if let Some((old_key, graphic)) = &*cache
        && old_key == &key
    {
        return std::sync::Arc::clone(graphic);
    }
    let (paths, width, height) = enzui_shell::lock_fonts::outlines(family, weight, time)
        .or_else(|| enzui_shell::lock_fonts::outlines("Fredoka", 600, time))
        .expect("bundled numeral outlines");
    let document = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {width} {height}'><g fill='white'>{paths}</g></svg>"
    );
    let graphic = Arc::new(Graphic {
        silhouette: Arc::new(
            GlassSilhouette::from_svg(&document, width, height).expect("clock silhouette"),
        ),
        document,
        simple: std::sync::OnceLock::new(),
    });
    *cache = Some((key, std::sync::Arc::clone(&graphic)));
    graphic
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeral_changes_do_not_resize_the_clock() {
        for time in ["20:48", "11:11", "00:00"] {
            assert_eq!(
                graphic_font(time, "Fredoka", 600).silhouette.aspect_ratio(),
                graphic_font("20:48", "Fredoka", 600)
                    .silhouette
                    .aspect_ratio()
            );
        }
    }
}
