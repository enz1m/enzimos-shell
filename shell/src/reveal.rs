use iced::{
    Color, Rectangle, Size, Vector,
    advanced::{
        Renderer as _, renderer,
        svg::{self, Renderer as _},
    },
    border,
};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Attachment {
    LeftFrame,
    Bar,
    BothFrames,
    Island {
        center: f32,
        seed_width: f32,
    },
    #[default]
    RightFrame,
}
/// The room a popover's window keeps beside its panel, enough for the
/// largest shadow level, so placement never depends on the chosen level.
pub const SHADOW: f32 = 28.0;
/// The largest panel shadow a floating island panel takes: the large one
/// overran [`SHADOW`] and was cut off square round the panel.
pub const ISLAND_PANEL_SHADOW_MAX: u8 = 3;

/// Returns attachment-aware shadow margins, excluding edges joined to the bar or frame.
#[must_use]
pub fn margins(bottom: bool, attachment: Attachment) -> iced::Padding {
    let far = SHADOW + 8.0;
    let (near, left, right) = match attachment {
        Attachment::Island { .. } => (10.0, SHADOW, SHADOW),
        Attachment::Bar => (0.0, SHADOW, SHADOW),
        Attachment::LeftFrame => (0.0, 0.0, SHADOW),
        Attachment::RightFrame => (0.0, SHADOW, 0.0),
        Attachment::BothFrames => (0.0, 0.0, 0.0),
    };
    iced::Padding {
        top: if bottom { far } else { near },
        bottom: if bottom { near } else { far },
        left,
        right,
    }
}

/// The name of a shadow level offered in Settings, from none (0) to large
/// (4), in the desktop's language.
pub fn shadow_level(level: u8) -> &'static str {
    crate::i18n::tr(match level {
        0 => "shell-off",
        1 => "shell-minimal",
        2 => "shell-small",
        3 => "shell-medium",
        _ => "shell-large",
    })
}
/// Applies framework reduced-motion settling to glass optics.
/// The person's adjustments to every glass material: its tint, blur and
/// how far its rim bends the backdrop (`chrome_style`). `None` keeps the
/// material's own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlassTweaks {
    /// Tint weight, `0..=1`.
    pub tint: Option<f32>,
    /// Blur sigma, in logical pixels.
    pub blur: Option<f32>,
    /// Rim refraction, `0..=1` of a full lens.
    pub bending: Option<f32>,
    /// Rim light, `0..=1`.
    pub highlight: Option<f32>,
    /// Reduced motion: no bending, whatever was asked.
    pub still: bool,
}
impl GlassTweaks {
    pub fn apply(self, mut tokens: enzui_theme::MaterialTokens) -> enzui_theme::MaterialTokens {
        if tokens.is_opaque() {
            return tokens;
        }
        if let Some(tint) = self.tint {
            tokens.tint_opacity = tint.clamp(0.0, 0.95);
        }
        if let Some(blur) = self.blur {
            tokens.blur_sigma = blur.clamp(0.0, 24.0);
        }
        if let Some(bending) = self.bending {
            let bending = bending.clamp(0.0, 1.0);
            tokens.refraction = bending;
            // A flat material has no rim to bend through: give it the
            // regular glass's.
            if bending > 0.0 {
                if tokens.bevel_width <= 0.0 {
                    tokens.bevel_width = 24.0;
                }
                if tokens.optical_depth <= 0.0 {
                    tokens.optical_depth = 40.0;
                }
                tokens.index_of_refraction = tokens.index_of_refraction.max(1.3);
            }
        }
        if let Some(highlight) = self.highlight {
            tokens.highlight = highlight.clamp(0.0, 1.0);
        }
        if self.still {
            tokens.refraction = 0.0;
            tokens.dispersion = 0.0;
        }
        tokens
    }
}
static GLASS_TWEAKS: std::sync::RwLock<GlassTweaks> = std::sync::RwLock::new(GlassTweaks {
    tint: None,
    blur: None,
    bending: None,
    highlight: None,
    still: false,
});
/// Sets the adjustments every glass region takes from now on (applied as its
/// tokens are sent, after the shell's own frost); returns whether they
/// changed.
/// The adjustments set last.
pub fn glass_tweaks() -> GlassTweaks {
    *GLASS_TWEAKS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub fn set_glass_tweaks(tweaks: GlassTweaks) -> bool {
    let mut current = GLASS_TWEAKS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let changed = *current != tweaks;
    *current = tweaks;
    changed
}

pub fn glass_tokens(
    style: enzui_theme::MaterialStyle,
    reduced_motion: bool,
) -> enzui_theme::MaterialTokens {
    let tokens = style.tokens();
    if reduced_motion {
        enzui_theme::MaterialTokens {
            refraction: 0.0,
            dispersion: 0.0,
            ..tokens
        }
    } else {
        tokens
    }
}
/// A shadow level as (blur, offset away from the bar, opacity) for a glass
/// body over `color`.
pub fn glass_shadow(level: u8, color: Color, bottom: bool) -> Option<(f32, f32, f32)> {
    let (blur, dark, light, offset) = PanelShadow {
        room: true,
        level,
        bar: false,
    }
    .look()?;
    let lit = 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b > 0.5;
    Some((
        blur,
        if bottom { -offset } else { offset },
        if lit { light } else { dark },
    ))
}
/// Stored panel-material indices; append new styles to preserve saved selections.
pub const PANEL_MATERIALS: [(&str, Option<enzui_theme::MaterialStyle>); 7] = [
    ("Solid", None),
    // An even frosted tint without refraction, darkening or
    // colour fringes, so panels read cleanly over light and dark wallpaper.
    ("Glass", Some(enzui_theme::MaterialStyle::FlatFrosted)),
    ("Clear", Some(enzui_theme::MaterialStyle::Clear)),
    ("Frosted", Some(enzui_theme::MaterialStyle::Frosted)),
    ("Regular", Some(enzui_theme::MaterialStyle::Regular)),
    ("Lens", Some(enzui_theme::MaterialStyle::Lens)),
    ("Subtle", Some(enzui_theme::MaterialStyle::Subtle)),
];

/// Selects chrome material: the bar's pills take the panels' material, so they
/// keep a visible body on any wallpaper (clear glass vanished over dark ones).
pub fn chrome_material(panel_glass: u8) -> Option<enzui_theme::MaterialStyle> {
    PANEL_MATERIALS[usize::from(panel_glass).min(PANEL_MATERIALS.len() - 1)].1
}

/// A popover's shadow: whether its window keeps room for one, the level
/// (0 none … 4 large), and whether the bar casts one too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelShadow {
    pub room: bool,
    pub level: u8,
    pub bar: bool,
}
impl PanelShadow {
    pub const NONE: Self = Self {
        room: false,
        level: 0,
        bar: false,
    };
    // Blur, opacity on a dark and on a light panel, and offset away from the bar.
    fn look(self) -> Option<(f32, f32, f32, f32)> {
        Some(match self.level {
            // Broad and restrained: dark panels' shadows read as a dense
            // black halo at the old opacities.
            1 => (6.0, 0.26, 0.10, 1.0),
            2 => (12.0, 0.36, 0.16, 3.0),
            3 => (18.0, 0.46, 0.22, 5.0),
            4 => (26.0, 0.58, 0.32, 7.0),
            _ => return None,
        })
    }
}

impl Attachment {
    pub fn insets(self, radius: f32) -> (f32, f32) {
        match self {
            Self::LeftFrame => (0.0, radius),
            Self::Bar => (radius, radius),
            Self::BothFrames | Self::Island { .. } => (0.0, 0.0),
            Self::RightFrame => (radius, 0.0),
        }
    }
    pub fn join(self, radius: f32) -> f32 {
        match self {
            Self::Island { .. } => 0.0,
            _ => radius,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Overview {
        left: f32,
        right: f32,
        bottom: bool,
    },
    Controls {
        bottom: bool,
        attachment: Attachment,
        /// The panel's shadow and whether its window keeps room for one.
        shadow: PanelShadow,
    },
    Rail {
        right: bool,
        frame: f32,
    },
    FloatingRail {
        right: bool,
    },
}
#[derive(Clone, Copy, Debug)]
pub struct Reveal {
    pub shape: Shape,
    pub radius: f32,
    pub color: Color,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub extended: bool,
    pub translation: Vector,
    pub clip: Rectangle,
    pub fills: Vec<(Rectangle, border::Radius)>,
    pub corners: Vec<(Rectangle, bool, bool)>,
}
fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle {
        x,
        y,
        width: width.max(0.0),
        height: height.max(0.0),
    }
}
impl Reveal {
    /// The one shape every bar popup uses; `attachment` decides whether it
    /// joins the frame or floats as an island below its trigger.
    pub fn popup(
        bottom: bool,
        attachment: Attachment,
        radius: f32,
        color: Color,
        shadow: PanelShadow,
    ) -> Self {
        Self {
            shape: Shape::Controls {
                bottom,
                attachment,
                shadow,
            },
            radius,
            color,
        }
    }
    /// The panel's shadow as (blur, downward offset, opacity), for a body the
    /// compositor paints in glass and shadows itself.
    pub fn glass_shadow(self) -> Option<(f32, f32, f32)> {
        let Shape::Controls { bottom, shadow, .. } = self.shape else {
            return None;
        };
        glass_shadow(shadow.level, self.color, bottom)
    }
    pub fn is_island(self) -> bool {
        matches!(
            self.shape,
            Shape::Controls {
                attachment: Attachment::Island { .. },
                ..
            }
        )
    }
    pub fn join(self) -> f32 {
        match self.shape {
            Shape::Controls { attachment, .. } => attachment.join(self.radius),
            _ => self.radius,
        }
    }
    /// The room kept around the panel for its shadow.
    #[must_use]
    pub fn margins(self) -> iced::Padding {
        match self.shape {
            Shape::Controls {
                bottom,
                attachment,
                shadow,
            } if shadow.room => margins(bottom, attachment),
            _ => iced::Padding::ZERO,
        }
    }
    /// The shapes for a window of `size`: a shadowed popover's panel sits
    /// inside its [`margins`](Self::margins).
    pub fn frame(self, size: Size, progress: f32) -> Frame {
        let m = self.margins();
        if m == iced::Padding::ZERO {
            return self.panel_frame(size, progress);
        }
        let inner = Size::new(
            (size.width - m.left - m.right).max(0.0),
            (size.height - m.top - m.bottom).max(0.0),
        );
        let mut frame = self.panel_frame(inner, progress);
        let offset = Vector::new(m.left, m.top);
        frame.clip += offset;
        for (rect, _) in &mut frame.fills {
            *rect += offset;
        }
        for (rect, _, _) in &mut frame.corners {
            *rect += offset;
        }
        frame
    }
    /// The shapes for a panel of `size`, without shadow room.
    pub fn panel_frame(self, size: Size, progress: f32) -> Frame {
        let p = progress.clamp(0.0, 1.0);
        let r = self
            .radius
            .max(0.0)
            .min(size.width / 2.0)
            .min(size.height / 2.0);
        let mut f = Frame {
            extended: false,
            translation: Vector::ZERO,
            clip: rect(0.0, 0.0, 0.0, 0.0),
            fills: Vec::new(),
            corners: Vec::new(),
        };
        match self.shape {
            Shape::Overview {
                left,
                right,
                bottom,
            } => {
                let h = size.height - r;
                let d = (h * p).round();
                f.extended = d > 0.0;
                let y = if bottom { size.height - d } else { 0.0 };
                let edge = if bottom { y - r } else { d };
                f.translation = Vector::new(0.0, if bottom { h - d } else { d - h });
                f.clip = rect(0.0, y, size.width, d);
                f.fills.push((f.clip, border::Radius::default()));
                f.fills
                    .push((rect(0.0, edge, left, r), border::Radius::default()));
                f.fills.push((
                    rect(size.width - right, edge, right, r),
                    border::Radius::default(),
                ));
                f.corners.push((rect(left, edge, r, r), false, bottom));
                f.corners
                    .push((rect(size.width - right - r, edge, r, r), true, bottom));
            }
            Shape::Controls {
                bottom, attachment, ..
            } => {
                if let Attachment::Island { center, seed_width } = attachment {
                    let seed = seed_width.clamp(0.0, size.width);
                    let center = center.clamp(seed / 2.0, size.width - seed / 2.0);
                    let depth = (size.height * p).round();
                    let width = seed + (size.width - seed) * p;
                    let x = (center - seed / 2.0) * (1.0 - p);
                    let y = if bottom { size.height - depth } else { 0.0 };
                    f.extended = depth > 0.0;
                    f.clip = rect(x, y, width, depth);
                    f.fills
                        .push((f.clip, r.min(depth / 2.0).min(width / 2.0).into()));
                    return f;
                }
                let h = size.height - r;
                let d = (h * p).round();
                f.extended = d > 0.0;
                let q = r.min(d / 2.0);
                let y = if bottom { size.height - d } else { 0.0 };
                let edge = if bottom { y - r } else { d };
                let (left, right) = attachment.insets(r);
                f.translation = Vector::new(0.0, if bottom { h - d } else { d - h });
                f.clip = rect(left, y, size.width - left - right, d);
                let l = if matches!(attachment, Attachment::LeftFrame | Attachment::BothFrames) {
                    0.0
                } else {
                    q
                };
                let r_corner =
                    if matches!(attachment, Attachment::RightFrame | Attachment::BothFrames) {
                        0.0
                    } else {
                        q
                    };
                f.fills.push((
                    f.clip,
                    border::Radius {
                        top_left: if bottom { l } else { 0.0 },
                        bottom_left: if bottom { 0.0 } else { l },
                        top_right: if bottom { r_corner } else { 0.0 },
                        bottom_right: if bottom { 0.0 } else { r_corner },
                    },
                ));
                let join = if bottom { size.height - q } else { 0.0 };
                if left > 0.0 {
                    f.corners.push((rect(left - q, join, q, q), true, bottom));
                }
                if right > 0.0 {
                    f.corners
                        .push((rect(size.width - right, join, q, q), false, bottom));
                }
                match attachment {
                    Attachment::LeftFrame => f.corners.push((rect(0.0, edge, r, r), false, bottom)),
                    Attachment::RightFrame => {
                        f.corners
                            .push((rect(size.width - r, edge, r, r), true, bottom));
                    }
                    Attachment::BothFrames => {
                        f.corners.push((rect(0.0, edge, r, r), false, bottom));
                        f.corners
                            .push((rect(size.width - r, edge, r, r), true, bottom));
                    }
                    Attachment::Bar | Attachment::Island { .. } => {}
                }
            }
            Shape::FloatingRail { right } => {
                let width = (size.width * p).round();
                let x = if right { size.width - width } else { 0.0 };
                f.extended = width > 0.0;
                f.translation = Vector::new(
                    if right {
                        size.width - width
                    } else {
                        width - size.width
                    },
                    0.0,
                );
                f.clip = rect(x, 0.0, width, size.height);
                f.fills.push((f.clip, r.min(width / 2.0).into()));
            }
            Shape::Rail { right, frame } => {
                let width = size.width - r;
                let d = ((width - frame).max(0.0) * p).round() + frame;
                f.extended = d > frame;
                let x = if right { size.width - d } else { 0.0 };
                let edge = if right { x - r } else { d };
                f.translation = Vector::new(if right { width - d } else { d - width }, 0.0);
                f.clip = rect(x, 0.0, d, size.height);
                f.fills.push((f.clip, border::Radius::default()));
                f.corners.push((rect(edge, 0.0, r, r), right, false));
                f.corners
                    .push((rect(edge, size.height - r, r, r), right, true));
            }
        }
        f
    }
    pub fn draw(
        self,
        frame: &Frame,
        origin: iced::Point,
        renderer: &mut iced::Renderer,
        viewport: Rectangle,
    ) {
        // A shadowed popover's panel casts a soft shadow away from the bar.
        let mut shadow = iced::Shadow::default();
        if let Shape::Controls {
            bottom,
            attachment,
            shadow: panel,
        } = self.shape
            && let Some((blur, dark, light, offset)) = panel.look()
            && let Some(&(bounds, radius)) = frame.fills.first()
        {
            let lit = 0.2126 * self.color.r + 0.7152 * self.color.g + 0.0722 * self.color.b > 0.5;
            let alpha = if lit { light } else { dark };
            let look = |alpha: f32| iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, alpha),
                offset: Vector::new(0.0, if bottom { -offset } else { offset }),
                blur_radius: blur,
            };
            let joined = !matches!(attachment, Attachment::Island { .. });
            if !joined || panel.bar {
                // Floating, or joined to a bar that casts its own: the whole
                // panel casts, meeting the bar's shadow at the join.
                shadow = look(alpha);
            } else {
                // Joined-bar shadows fade away from the join to keep both surfaces visually
                // connected.
                let reach = (blur * 4.0).min(bounds.height / 2.0);
                const LAYERS: usize = 8;
                for layer in 0..LAYERS {
                    let hold = reach * layer as f32 / LAYERS as f32;
                    let caster = Rectangle {
                        y: if bottom { bounds.y } else { bounds.y + hold },
                        height: bounds.height - hold,
                        ..bounds
                    };
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: caster + Vector::new(origin.x, origin.y),
                            border: iced::Border {
                                radius,
                                ..Default::default()
                            },
                            shadow: look(alpha / LAYERS as f32),
                            ..Default::default()
                        },
                        Color::TRANSPARENT,
                    );
                }
            }
        }
        for (index, &(bounds, radius)) in frame.fills.iter().enumerate() {
            if bounds.width > 0.0 && bounds.height > 0.0 {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: bounds + Vector::new(origin.x, origin.y),
                        border: iced::Border {
                            radius,
                            ..Default::default()
                        },
                        shadow: if index == 0 {
                            shadow
                        } else {
                            iced::Shadow::default()
                        },
                        ..Default::default()
                    },
                    self.color,
                );
            }
        }
        for &(bounds, right, bottom) in &frame.corners {
            if bounds.width > 0.0 && bounds.height > 0.0 {
                renderer.draw_svg(
                    svg::Svg::new(corner(right, bottom)).color(self.color),
                    bounds + Vector::new(origin.x, origin.y),
                    viewport,
                );
            }
        }
    }
}
fn corner(right: bool, bottom: bool) -> svg::Handle {
    static HANDLES: OnceLock<[svg::Handle; 4]> = OnceLock::new();
    HANDLES.get_or_init(||std::array::from_fn(|i|{
        let x=i&1!=0;let y=i&2!=0;
        svg::Handle::from_memory(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\" viewBox=\"0 0 1 1\"><path transform=\"translate({} {}) scale({} {})\" fill=\"white\" d=\"M0 0 H1 A1 1 0 0 0 0 1 Z\"/></svg>",u8::from(x),u8::from(y),if x{-1}else{1},if y{-1}else{1}).into_bytes())
    }))[usize::from(right)+2*usize::from(bottom)].clone()
}
