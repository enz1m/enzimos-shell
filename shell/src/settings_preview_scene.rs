use super::css;
use enzui_shell::{
    notifications::preferences::Position,
    settings::{BarEdge, Settings, SidebarEdge},
    shell_style::ShellStyle,
};
use enzui_theme::palette::Palette;
use std::fmt::Write as _;

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 360.0;
type Rect = [f32; 4];

struct Scene<'a> {
    palette: &'a Palette,
    body: String,
}
impl Scene<'_> {
    fn rect(
        &mut self,
        bounds: Rect,
        radius: f32,
        color: iced::Color,
        border: Option<(iced::Color, f32)>,
    ) {
        let [x, y, width, height] = bounds;
        let (stroke, thickness) = border.map_or_else(|| ("none".into(), 0.0), |(c, w)| (css(c), w));
        let _ = write!(
            self.body,
            r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="{}" stroke="{stroke}" stroke-width="{thickness}"/>"#,
            css(color)
        );
    }
    fn shadow(&mut self, [x, y, width, height]: Rect, radius: f32, strength: f32) {
        let _ = write!(
            self.body,
            r#"<rect x="{x}" y="{}" width="{width}" height="{height}" rx="{radius}" fill="{}" opacity="{strength}" filter="url(#soft)"/>"#,
            y + 4.0,
            css(self.palette.shadow)
        );
    }
    fn line(&mut self, x: f32, y: f32, width: f32, height: f32, color: iced::Color) {
        self.rect([x, y, width.max(0.0), height], height / 2.0, color, None);
    }
    // An app window: title bar with controls, a sidebar with a selected
    // page, a heading and a few lines of content.
    fn window(&mut self, bounds: Rect, settings: &Settings, active: bool) {
        let p = self.palette;
        let [x, y, width, height] = bounds;
        let radius = f32::from(settings.compositor.rounding());
        self.shadow(bounds, radius, if active { 0.28 } else { 0.14 });
        // The focused window's border in the tone chosen for it, as the
        // compositor draws it (`enzui_window_border_colors`).
        use enzui_shell::desktop_preferences::BorderColor;
        let focused = match BorderColor::from_value(settings.compositor.border_color.into()) {
            BorderColor::Automatic | BorderColor::Accent => p.accent,
            BorderColor::Neutral => enzui_shell::theme_palette::rgb(0x92969d),
            BorderColor::White => iced::Color::WHITE,
            BorderColor::Custom => {
                enzui_shell::theme_palette::rgb(settings.compositor.border_custom)
            }
        };
        self.rect(
            bounds,
            radius,
            p.background,
            Some((
                if active { focused } else { p.control_border },
                f32::from(settings.compositor.border_size).max(0.5),
            )),
        );
        let inset = (radius * 0.3).max(4.0);
        let sidebar = (width * 0.28).min(84.0);
        if width > 90.0 && height > 60.0 {
            self.rect(
                [x + inset * 0.5, y + inset * 0.5, sidebar, height - inset],
                (radius - inset * 0.5).max(2.0),
                p.surface_alt,
                None,
            );
            for row in 0..4u8 {
                let row_y = y + 26.0 + f32::from(row) * 13.0;
                if row_y + 10.0 > y + height - 6.0 {
                    break;
                }
                if row == 1 {
                    self.rect(
                        [x + 7.0, row_y - 3.0, sidebar - 10.0, 11.0],
                        3.0,
                        p.accent_soft,
                        None,
                    );
                }
                self.line(
                    x + 12.0,
                    row_y,
                    sidebar * if row == 1 { 0.55 } else { 0.42 },
                    4.0,
                    if row == 1 { p.accent } else { p.text_muted },
                );
            }
        }
        let content = if width > 90.0 {
            x + sidebar + 10.0
        } else {
            x + 10.0
        };
        let content_width = x + width - content - 10.0;
        for i in 0..3u8 {
            let _ = write!(
                self.body,
                r#"<circle cx="{}" cy="{}" r="2.6" fill="{}"/>"#,
                x + width - 10.0 - f32::from(i) * 9.0,
                y + 10.0,
                css(p.text_subtle)
            );
        }
        self.line(content, y + 22.0, content_width * 0.42, 6.0, p.text);
        for (index, fraction) in [0.9, 0.74, 0.82, 0.6].into_iter().enumerate() {
            let line_y = y + 36.0 + index as f32 * 10.0;
            if line_y + 4.0 > y + height - 8.0 {
                break;
            }
            self.line(
                content,
                line_y,
                content_width * fraction,
                4.0,
                p.surface_hover,
            );
        }
        if height > 110.0 {
            self.rect(
                [
                    content,
                    y + height - 22.0,
                    (content_width * 0.26).min(44.0),
                    12.0,
                ],
                4.0,
                p.accent,
                None,
            );
        }
    }
    fn finish(self) -> String {
        let p = self.palette;
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360" viewBox="0 0 640 360"><defs><linearGradient id="desktop" x2="1" y2="1"><stop stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient><radialGradient id="glow"><stop stop-color="{}" stop-opacity="0.35"/><stop offset="1" stop-color="{}" stop-opacity="0"/></radialGradient><filter id="soft" x="-20%" y="-20%" width="140%" height="150%"><feGaussianBlur stdDeviation="5"/></filter><clipPath id="screen"><rect width="640" height="360" rx="16"/></clipPath></defs><g clip-path="url(#screen)"><rect width="640" height="360" fill="url(#desktop)"/><circle cx="120" cy="300" r="240" fill="url(#glow)"/><circle cx="560" cy="40" r="200" fill="url(#glow)"/>{}</g></svg>"#,
            css(p.accent_soft),
            css(p.surface_alt),
            css(p.accent),
            css(p.accent),
            self.body
        )
    }
}

fn chrome(settings: &Settings, style: ShellStyle) -> (Vec<(Rect, f32)>, Rect, Rect) {
    let top = settings.bar == BarEdge::Top;
    let left = settings.sidebar == SidebarEdge::Left;
    match style {
        ShellStyle::Frame => {
            let f = f32::from(settings.metrics.frame);
            let bar = f32::from(settings.metrics.bar);
            let rail = f32::from(settings.metrics.rail);
            let y = if top { f + bar } else { f };
            let h = HEIGHT - 2.0 * f - bar;
            let panels = vec![(
                [
                    f,
                    if top { f } else { HEIGHT - f - bar },
                    WIDTH - 2.0 * f,
                    bar,
                ],
                0.0,
            )];
            (
                panels,
                [if left { f } else { WIDTH - f - rail }, y, rail, h],
                [
                    f + if left && settings.sidebar_enabled {
                        rail
                    } else {
                        0.0
                    },
                    y,
                    WIDTH - 2.0 * f - if settings.sidebar_enabled { rail } else { 0.0 },
                    h,
                ],
            )
        }
        ShellStyle::Islands => {
            let s = settings.islands;
            let margin = f32::from(s.side_gap);
            let gap = f32::from(s.gap);
            let bar = f32::from(s.height);
            let by = if top {
                f32::from(s.edge_gap)
            } else {
                HEIGHT - f32::from(s.edge_gap) - bar
            };
            let panels = if s.joined_bar {
                vec![([margin, by, WIDTH - 2.0 * margin, bar], bar / 2.0)]
            } else {
                let available = WIDTH - 2.0 * margin - 2.0 * gap;
                let mut x = margin;
                [0.34, 0.30, 0.36]
                    .into_iter()
                    .map(|part| {
                        let width = available * part;
                        let result = ([x, by, width, bar], f32::from(s.radius));
                        x += width + gap;
                        result
                    })
                    .collect()
            };
            let usable = HEIGHT - bar - f32::from(s.edge_gap) - 2.0 * margin;
            let rail_height = ((f32::from(settings.metrics.icon) + 12.0) * 5.0
                + 2.0 * f32::from(s.padding))
            .min(usable);
            let y = if top { by + bar + margin } else { margin };
            let rail = f32::from(s.rail);
            (
                panels,
                [
                    if left { margin } else { WIDTH - margin - rail },
                    y + (usable - rail_height) / 2.0,
                    rail,
                    rail_height,
                ],
                [
                    margin
                        + if left && settings.sidebar_enabled {
                            rail + gap
                        } else {
                            0.0
                        },
                    y,
                    WIDTH
                        - 2.0 * margin
                        - if settings.sidebar_enabled {
                            rail + gap
                        } else {
                            0.0
                        },
                    usable,
                ],
            )
        }
    }
}

fn windows(area: Rect, settings: &Settings, tiled: bool) -> [Rect; 2] {
    let [x, y, w, h] = area;
    if tiled {
        let outer = f32::from(settings.compositor.gaps_outer) * 0.5;
        let inner = f32::from(settings.compositor.gaps_inner) * 0.5;
        let width = (w - 2.0 * outer - 2.0 * inner) / 2.0;
        [
            [x + outer, y + outer, width, h - 2.0 * outer],
            [
                x + outer + width + 2.0 * inner,
                y + outer,
                width,
                h - 2.0 * outer,
            ],
        ]
    } else {
        [
            [x + w * 0.08, y + h * 0.10, w * 0.65, h * 0.62],
            [x + w * 0.30, y + h * 0.32, w * 0.63, h * 0.60],
        ]
    }
}

fn toasts(settings: &Settings, style: ShellStyle) -> Vec<Rect> {
    let p = &settings.notifications;
    let bottom = matches!(p.position, Position::BottomLeft | Position::BottomRight)
        || p.position == Position::BarEnd && settings.bar == BarEdge::Bottom;
    let left = matches!(p.position, Position::TopLeft | Position::BottomLeft);
    let near_bar = bottom == (settings.bar == BarEdge::Bottom);
    let edge = 12.0
        + if near_bar {
            match style {
                ShellStyle::Frame => {
                    f32::from(settings.metrics.bar) + f32::from(settings.metrics.frame)
                }
                ShellStyle::Islands => {
                    f32::from(settings.islands.height) + f32::from(settings.islands.edge_gap)
                }
            }
        } else {
            0.0
        };
    let width = f32::from(p.width) * WIDTH / 1440.0;
    let count = p.maximum_toasts.clamp(1, 5);
    let height = 44.0_f32.min((HEIGHT - edge - 12.0) / f32::from(count) - 6.0);
    (0..count)
        .map(|i| {
            let step = f32::from(i) * (height + 6.0);
            [
                if left { 12.0 } else { WIDTH - 12.0 - width },
                if bottom {
                    HEIGHT - edge - height - step
                } else {
                    edge + step
                },
                width,
                height,
            ]
        })
        .collect()
}

pub(super) fn desktop(
    settings: &Settings,
    palette: &Palette,
    style: ShellStyle,
    tiled: bool,
    notifications: bool,
) -> String {
    let mut scene = Scene {
        palette,
        body: String::new(),
    };
    let (panels, rail, area) = chrome(settings, style);
    for (index, bounds) in windows(area, settings, tiled).into_iter().enumerate() {
        scene.window(bounds, settings, index == 1);
    }
    if style == ShellStyle::Frame {
        let f = f32::from(settings.metrics.frame);
        let _ = write!(
            scene.body,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="none" stroke="{}" stroke-width="{f}"/>"#,
            f / 2.0,
            f / 2.0,
            WIDTH - f,
            HEIGHT - f,
            settings.metrics.radius,
            css(palette.surface)
        );
    }
    let count = panels.len();
    for (index, (bounds, radius)) in panels.into_iter().enumerate() {
        scene.rect(bounds, radius, palette.surface, None);
        let [x, y, w, h] = bounds;
        let inset = if style == ShellStyle::Islands {
            f32::from(settings.islands.padding).max(6.0)
        } else {
            8.0
        }
        .min(w * 0.25);
        let mid = y + h / 2.0;
        let glyph = (h * 0.42).clamp(6.0, 12.0);
        // One bar shows all three regions; separate islands get one each.
        let regions: &[u8] = if count == 1 {
            &[0, 1, 2]
        } else {
            std::slice::from_ref(&[0u8, 1, 2][index.min(2)])
        };
        for region in regions {
            match region {
                0 => {
                    let pill = [x + inset * 0.5, mid - glyph * 0.8, glyph * 5.4, glyph * 1.6];
                    scene.rect(pill, glyph * 0.8, palette.surface_hover, None);
                    for k in 0..4u8 {
                        let (dx, dy) = (f32::from(k % 2), f32::from(k / 2));
                        scene.rect(
                            [
                                pill[0] + glyph * 0.45 + dx * glyph * 0.42,
                                mid - glyph * 0.4 + dy * glyph * 0.42,
                                glyph * 0.34,
                                glyph * 0.34,
                            ],
                            1.0,
                            palette.text,
                            None,
                        );
                    }
                    scene.line(
                        pill[0] + glyph * 1.6,
                        mid - 1.8,
                        glyph * 3.2,
                        3.6,
                        palette.text_muted,
                    );
                }
                1 => {
                    let width = (glyph * 6.0).min(w * 0.5);
                    scene.line(x + (w - width) / 2.0, mid - 2.0, width, 4.0, palette.text);
                }
                _ => {
                    for k in 0..3u8 {
                        let _ = write!(
                            scene.body,
                            r#"<circle cx="{}" cy="{mid}" r="{}" fill="{}"/>"#,
                            x + w - inset - glyph * 0.5 - f32::from(k) * glyph * 1.4,
                            glyph * 0.38,
                            css(palette.text_muted)
                        );
                    }
                }
            }
        }
    }
    if settings.sidebar_enabled {
        scene.rect(
            rail,
            if style == ShellStyle::Islands {
                f32::from(settings.islands.radius)
            } else {
                0.0
            },
            palette.surface,
            None,
        );
        let [x, y, w, h] = rail;
        let icon = f32::from(settings.metrics.icon).min(w - 8.0).min(h / 7.0);
        // Miniapp icons in calm, distinct colours; the open one is marked.
        let colours = [
            palette.accent,
            iced::Color::from_rgb8(0x2f, 0xa8, 0x5a),
            iced::Color::from_rgb8(0xf2, 0x9d, 0x2b),
            iced::Color::from_rgb8(0x1a, 0xa7, 0x9e),
            iced::Color::from_rgb8(0x6b, 0x7a, 0x8c),
        ];
        for (i, colour) in colours.into_iter().enumerate() {
            let iy = y + h / 2.0 + (i as f32 - 2.0) * (icon + 10.0) - icon / 2.0;
            scene.rect(
                [x + (w - icon) / 2.0, iy, icon, icon],
                icon * 0.28,
                colour,
                None,
            );
            if i == 0 {
                let edge = if settings.sidebar == SidebarEdge::Left {
                    x + 1.5
                } else {
                    x + w - 4.5
                };
                scene.rect(
                    [edge, iy + icon * 0.2, 3.0, icon * 0.6],
                    1.5,
                    palette.accent,
                    None,
                );
            }
        }
    }
    if notifications {
        for bounds in toasts(settings, style) {
            scene.shadow(bounds, 8.0, 0.22);
            scene.rect(
                bounds,
                8.0,
                palette.surface_elevated,
                Some((palette.control_border, 1.0)),
            );
            let [x, y, width, height] = bounds;
            let icon = (height * 0.36).clamp(8.0, 14.0);
            scene.rect(
                [x + 8.0, y + 8.0, icon, icon],
                icon * 0.3,
                palette.accent,
                None,
            );
            scene.line(
                x + 14.0 + icon,
                y + 8.0 + icon / 2.0 - 2.0,
                width * 0.45,
                4.0,
                palette.text,
            );
            if height > 26.0 {
                scene.line(
                    x + 14.0 + icon,
                    y + 8.0 + icon + 5.0,
                    width * 0.62,
                    3.5,
                    palette.text_muted,
                );
            }
        }
    }
    scene.finish()
}
