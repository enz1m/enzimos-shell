//! Theme-choice previews rendered from the same palette and shape tokens as the desktop.

use enzui_shell::theme_palette::{Scheme, shown};
use enzui_shell::theme_presets::Choices;
use enzui_theme::EnzThemeMode;
use std::fmt::Write as _;

const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 240.0;

fn css(color: iced::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8
    )
}

fn bar(x: f32, y: f32, w: f32, h: f32, color: iced::Color) -> String {
    format!(
        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{}"/>"#,
        css(color),
        r = h / 2.0
    )
}

fn rect(x: f32, y: f32, w: f32, h: f32, r: f32, fill: iced::Color) -> String {
    format!(
        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{}"/>"#,
        css(fill)
    )
}

fn outlined(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    fill: iced::Color,
    line: iced::Color,
) -> String {
    format!(
        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{}" stroke="{}" stroke-width="1"/>"#,
        css(fill),
        css(line)
    )
}

// One app window painted from a generated palette: title bar, a sidebar with
// a selected page, and a settings card with a switch, a slider, chips and the
// two button kinds, so every role the style changes is on screen.
fn window(
    p: &enzui_shell::theme_palette::Generated,
    choices: Choices,
    [x, y, w, h]: [f32; 4],
) -> String {
    let radius = choices.corners.rungs();
    let (control_height, row_height, _) = choices.density.sizing();
    let control_height = control_height * 0.72;
    let pitch = row_height * 0.62;
    let (card_radius, control_radius) = (radius[3], radius[1]);
    let window_radius = (card_radius + 2.0).min(16.0);
    let mut s = String::new();
    // Soft drop shadow, then the window body clipped to its own corners.
    let _ = write!(
        s,
        r#"<rect x="{x}" y="{}" width="{w}" height="{h}" rx="{window_radius}" fill="{}" opacity="0.18" filter="url(#soft)"/>"#,
        y + 6.0,
        css(p.get("shadow"))
    );
    s.push_str(&outlined(
        x,
        y,
        w,
        h,
        window_radius,
        p.get("background"),
        p.get("border"),
    ));
    let sidebar = w * 0.3;
    let _ = write!(
        s,
        r#"<path d="M{x0} {y0} h{sw} v{sh} h-{sw2} a{r} {r} 0 0 1 -{r} -{r} v-{sh2} a{r} {r} 0 0 1 {r} -{r} z" fill="{}"/>"#,
        css(p.get("surface_alt")),
        x0 = x + window_radius,
        y0 = y + 0.5,
        sw = sidebar - window_radius,
        sh = h - 1.0,
        sw2 = sidebar - window_radius,
        sh2 = h - 1.0 - 2.0 * window_radius,
        r = window_radius,
    );
    // Window controls and the app title.
    for i in 0u8..3 {
        let _ = write!(
            s,
            r#"<circle cx="{}" cy="{}" r="3.5" fill="{}"/>"#,
            x + w - 14.0 - f32::from(i) * 13.0,
            y + 14.0,
            css(p.get("text_subtle"))
        );
    }
    s.push_str(&bar(x + 12.0, y + 11.0, sidebar * 0.42, 6.0, p.get("text")));
    // Sidebar pages: one selected.
    for i in 0u8..5 {
        let row_y = y + 30.0 + f32::from(i) * pitch;
        if row_y + pitch > y + h - 6.0 {
            break;
        }
        let selected = i == 1;
        if selected {
            s.push_str(&rect(
                x + 6.0,
                row_y,
                sidebar - 12.0,
                pitch - 3.0,
                control_radius,
                p.get("accent_soft"),
            ));
        }
        let ink = p.get(if selected { "accent" } else { "text_muted" });
        s.push_str(&rect(
            x + 13.0,
            row_y + (pitch - 3.0) / 2.0 - 4.0,
            8.0,
            8.0,
            2.5,
            ink,
        ));
        s.push_str(&bar(
            x + 27.0,
            row_y + (pitch - 3.0) / 2.0 - 2.5,
            sidebar
                * if selected {
                    0.46
                } else {
                    0.38 + f32::from(i % 2) * 0.1
                },
            5.0,
            ink,
        ));
    }
    // Page heading.
    let cx = x + sidebar + 14.0;
    let cw = w - sidebar - 28.0;
    s.push_str(&bar(cx, y + 30.0, cw * 0.38, 8.0, p.get("text")));
    s.push_str(&bar(cx, y + 44.0, cw * 0.62, 4.0, p.get("text_muted")));
    // Settings card.
    let card_y = y + 56.0;
    let card_h = h - 56.0 - 14.0;
    s.push_str(&outlined(
        cx,
        card_y,
        cw,
        card_h,
        card_radius,
        p.get("surface"),
        p.get("border"),
    ));
    let row = |i: f32| card_y + 8.0 + i * pitch;
    // Switch row.
    s.push_str(&bar(
        cx + 12.0,
        row(0.0) + pitch / 2.0 - 2.5,
        cw * 0.34,
        5.0,
        p.get("text"),
    ));
    let (sw, sh) = (control_height * 1.5, control_height * 0.82);
    let sx = cx + cw - 12.0 - sw;
    s.push_str(&bar(
        sx,
        row(0.0) + pitch / 2.0 - sh / 2.0,
        sw,
        sh,
        p.get("accent"),
    ));
    let _ = write!(
        s,
        r##"<circle cx="{}" cy="{}" r="{}" fill="#ffffff"/>"##,
        sx + sw - sh / 2.0,
        row(0.0) + pitch / 2.0,
        sh / 2.0 - 2.0
    );
    // Slider row.
    let slider_y = row(1.0) + pitch / 2.0;
    s.push_str(&bar(
        cx + 12.0,
        slider_y - 2.5,
        cw * 0.22,
        5.0,
        p.get("text"),
    ));
    let track_x = cx + 12.0 + cw * 0.3;
    let track_w = cw - 24.0 - cw * 0.3;
    s.push_str(&bar(
        track_x,
        slider_y - 2.0,
        track_w,
        4.0,
        p.get("control_border"),
    ));
    s.push_str(&bar(
        track_x,
        slider_y - 2.0,
        track_w * 0.6,
        4.0,
        p.get("accent"),
    ));
    let _ = write!(
        s,
        r#"<circle cx="{}" cy="{slider_y}" r="6" fill="{}" stroke="{}" stroke-width="1"/>"#,
        track_x + track_w * 0.6,
        css(p.get("surface_elevated")),
        css(p.get("control_border_strong"))
    );
    // Chips: the selected one carries the accent.
    let chip_y = row(2.0) + 3.0;
    let chip_h = (pitch - 8.0).max(10.0);
    let mut chip_x = cx + 12.0;
    for (i, chip_w) in [46.0_f32, 38.0, 52.0].into_iter().enumerate() {
        let selected = i == 0;
        s.push_str(&outlined(
            chip_x,
            chip_y,
            chip_w,
            chip_h,
            chip_h / 2.0,
            p.get(if selected { "accent_soft" } else { "surface" }),
            p.get(if selected { "accent" } else { "control_border" }),
        ));
        s.push_str(&bar(
            chip_x + 10.0,
            chip_y + chip_h / 2.0 - 2.0,
            chip_w - 20.0,
            4.0,
            p.get(if selected { "accent" } else { "text_muted" }),
        ));
        chip_x += chip_w + 6.0;
    }
    // Buttons, bottom right of the card.
    let button_h = control_height;
    let button_y = card_y + card_h - 10.0 - button_h;
    let primary_w = cw * 0.24;
    let primary_x = cx + cw - 12.0 - primary_w;
    s.push_str(&rect(
        primary_x,
        button_y,
        primary_w,
        button_h,
        control_radius,
        p.get("accent"),
    ));
    s.push_str(&bar(
        primary_x + primary_w * 0.25,
        button_y + button_h / 2.0 - 2.5,
        primary_w * 0.5,
        5.0,
        ink_on(p.get("accent")),
    ));
    let secondary_x = primary_x - 8.0 - primary_w;
    s.push_str(&outlined(
        secondary_x,
        button_y,
        primary_w,
        button_h,
        control_radius,
        p.get("surface_elevated"),
        p.get("control_border"),
    ));
    s.push_str(&bar(
        secondary_x + primary_w * 0.25,
        button_y + button_h / 2.0 - 2.5,
        primary_w * 0.5,
        5.0,
        p.get("text"),
    ));
    s
}

pub(super) fn card(choices: Choices, mode: EnzThemeMode, other_mode: EnzThemeMode) -> String {
    let (current_bounds, other_bounds) = if mode == other_mode {
        ([24.0, 34.0, 330.0, 196.0], [258.0, 14.0, 318.0, 196.0])
    } else {
        let (margin, gap, height) = (8.0, 12.0, 196.0);
        let width = (WIDTH - 2.0 * margin - gap) / 2.0;
        let y = (HEIGHT - height) / 2.0;
        (
            [margin, y, width, height],
            [margin + width + gap, y, width, height],
        )
    };
    let other = window(&shown(choices.scheme, other_mode), choices, other_bounds);
    let current = window(&shown(choices.scheme, mode), choices, current_bounds);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}" role="img"><defs><filter id="soft" x="-20%" y="-20%" width="140%" height="150%"><feGaussianBlur stdDeviation="7"/></filter></defs>{other}{current}</svg>"#
    )
}

pub(super) const TILE_WIDTH: f32 = 132.0;
pub(super) const TILE_HEIGHT: f32 = 84.0;

// A small picture of one style in the current colour, for the style choice.
pub(super) fn tile(scheme: Scheme, mode: EnzThemeMode, radius: f32) -> String {
    let p = shown(scheme, mode);
    let (w, h) = (TILE_WIDTH, TILE_HEIGHT);
    let mut body = format!(
        r#"<rect width="{w}" height="{h}" fill="{}"/>"#,
        css(p.get("background"))
    );
    let _ = write!(
        body,
        r#"<rect x="10" y="10" width="{cw}" height="{ch}" rx="{r}" fill="{}" stroke="{}" stroke-width="1"/>"#,
        css(p.get("surface")),
        css(p.get("border")),
        cw = w - 20.0,
        ch = h - 20.0,
        r = (radius * 0.6).min(10.0)
    );
    let _ = write!(
        body,
        r#"<rect x="16" y="17" width="{sw}" height="14" rx="{r}" fill="{}"/>"#,
        css(p.get("surface_selected")),
        sw = w - 32.0,
        r = (radius * 0.4).min(7.0)
    );
    body.push_str(&bar(22.0, 21.5, 40.0, 5.0, p.get("text")));
    body.push_str(&bar(22.0, 38.0, 58.0, 4.0, p.get("text_muted")));
    body.push_str(&bar(22.0, 47.0, 44.0, 4.0, p.get("text_subtle")));
    body.push_str(&bar(w - 44.0, 36.0, 22.0, 12.0, p.get("accent")));
    let _ = write!(
        body,
        r##"<circle cx="{cx}" cy="42" r="4" fill="#ffffff"/>"##,
        cx = w - 28.0
    );
    let _ = write!(
        body,
        r#"<rect x="22" y="58" width="44" height="12" rx="{r}" fill="{}"/>"#,
        css(p.get("accent")),
        r = (radius * 0.4).min(6.0)
    );
    let _ = write!(
        body,
        r#"<rect x="72" y="58" width="38" height="12" rx="{r}" fill="{}" stroke="{}" stroke-width="1"/>"#,
        css(p.get("button_primary_bg")),
        css(p.get("control_border")),
        r = (radius * 0.4).min(6.0)
    );
    // The picture's own corners follow the corner style, like the windows it shows.
    let corner = radius.clamp(4.0, 14.0);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img"><defs><clipPath id="tile"><rect width="{w}" height="{h}" rx="{corner}"/></clipPath></defs><g clip-path="url(#tile)">{body}</g></svg>"#
    )
}

/// The label colour a filled accent button uses, as the apps pick it: white
/// or near-black, whichever reads better (white on amber or light green was
/// below 2:1 in the preview while the apps used dark ink).
fn ink_on(fill: iced::Color) -> iced::Color {
    let linear = |c: f32| {
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * linear(fill.r) + 0.7152 * linear(fill.g) + 0.0722 * linear(fill.b);
    let white = 1.05 / (l + 0.05);
    let dark = (l + 0.05) / (0.012 + 0.05);
    if white >= dark {
        iced::Color::WHITE
    } else {
        iced::Color::from_rgb8(0x1D, 0x1D, 0x1F)
    }
}
