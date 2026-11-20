//! Shared notification card for toasts, history and previews.
//! Callers reserve badge_overhang outside the card's left edge.

use super::preferences::Style;
use enzui_theme::Palette;
use enzui_widgets::{
    card::enz_card,
    text::{TextSize, enz_text},
    text_measure::{MeasureTypography, enz_text_measure_with},
};
use iced::{
    Alignment, Color, Element, Length,
    widget::{column, container, row},
};

const ACTION: f32 = 26.0;
/// The reply row a toast shows in place of its buttons while replying.
pub const REPLY_ROW: f32 = 34.0;
/// Room around each card for its shadow; the toast window is this much larger
/// than its cards on every side.
pub const SHADOW_MARGIN: f32 = 12.0;

// One style's spacing.
#[derive(Clone, Copy)]
struct Density {
    padding: f32,
    glyph_gap: f32,
    header: f32,
    text_gap: f32,
    action_gap: f32,
    image: f32,
    elevation: u8,
    badge: f32,
}
/// A progress bar's thickness and its gap below the text.
const PROGRESS: f32 = 4.0;
const PROGRESS_GAP: f32 = 6.0;

fn density(style: Style) -> Density {
    match style {
        Style::Compact => Density {
            padding: 8.0,
            glyph_gap: 8.0,
            header: 18.0,
            text_gap: 0.0,
            action_gap: 6.0,
            image: 36.0,
            elevation: 1,
            badge: 0.0,
        },
        Style::Standard => Density {
            padding: 12.0,
            glyph_gap: 10.0,
            header: 22.0,
            text_gap: 2.0,
            action_gap: 8.0,
            image: 48.0,
            elevation: 2,
            badge: 0.0,
        },
        Style::Spacious => Density {
            padding: 16.0,
            glyph_gap: 14.0,
            header: 24.0,
            text_gap: 4.0,
            action_gap: 12.0,
            image: 64.0,
            elevation: 3,
            badge: 0.0,
        },
    }
}
/// Space between stacked cards.
pub const STACK_GAP: f32 = 8.0;
/// The row under two or more toasts that holds the dismiss-all button.
pub const DISMISS_ALL_ROW: f32 = 44.0;
/// How far a card's icon badge reaches past its left edge.
pub fn badge_overhang(style: Style) -> f32 {
    density(style).badge / 2.0
}

/// A card's shadow level in this style, as panel shadow levels count.
pub fn elevation(style: Style) -> u8 {
    density(style).elevation
}

/// What one card shows. Buttons are built by the caller, which owns their
/// messages, ids and enabled state.
pub struct Card<'a, Message> {
    pub style: Style,
    /// A background colour instead of the raised surface (the bar's).
    pub surface: Option<Color>,
    pub text: f32,
    pub radius: f32,
    pub icon: &'a str,
    /// The sending app's own icon file, drawn in the badge instead of `icon`.
    pub app_icon: Option<&'a std::path::Path>,
    pub source: &'a str,
    pub time: Option<String>,
    pub summary: &'a str,
    pub body: &'a str,
    /// Wrapped body lines shown; 0 shows all of it.
    pub body_lines: usize,
    pub critical: bool,
    pub unread: bool,
    pub image: Option<Element<'a, Message>>,
    pub dismiss: Option<Element<'a, Message>>,
    pub actions: Vec<Element<'a, Message>>,
    pub height: Option<f32>,
    /// The compositor paints the card's body in glass; it draws none itself.
    pub glass: bool,
    /// A bar under the text: progress, or a level such as a microphone's.
    pub progress: Option<u8>,
}

/// The inputs that decide a card's height, for sizing the toast window before
/// the cards are laid out.
pub struct Extent<'a> {
    pub style: Style,
    pub width: f32,
    pub text: f32,
    pub summary: &'a str,
    pub body: &'a str,
    pub body_lines: usize,
    pub image: bool,
    pub actions: bool,
    /// The reply row replaces the buttons.
    pub reply: bool,
    /// The card has a progress bar.
    pub progress: bool,
}

fn summary_size(text: f32) -> f32 {
    text + 1.0
}

fn meta_size(text: f32) -> f32 {
    (text - 1.0).max(10.0)
}

// Cards carry no icon: text starts at the card's own padding.
fn left_padding(d: Density) -> f32 {
    d.padding
}

// `width` includes the badge's overhang.
fn text_width(d: Density, width: f32, image: bool) -> f32 {
    let image = if image { d.image + d.glyph_gap } else { 0.0 };
    (width - left_padding(d) - d.padding - image).max(1.0)
}

fn lines(
    text: &str,
    width: f32,
    size: f32,
    weight: Option<iced::font::Weight>,
    limit: usize,
) -> f32 {
    let mut typography = MeasureTypography::new(TextSize::Body).pin_size_px(size);
    if let Some(weight) = weight {
        typography = typography.weight(weight);
    }
    let measured = enz_text_measure_with(text, width, typography);
    let count = if limit == 0 {
        measured.line_count
    } else {
        measured.line_count.min(limit)
    };
    count as f32 * measured.line_height
}

/// The exact height a card needs at `width`.
pub fn height(extent: &Extent<'_>) -> f32 {
    let d = density(extent.style);
    let width = text_width(d, extent.width, extent.image);
    let mut content = d.header
        + d.text_gap
        + lines(
            extent.summary,
            width,
            summary_size(extent.text),
            Some(iced::font::Weight::Semibold),
            2,
        );
    if !extent.body.trim().is_empty() {
        content += d.text_gap + lines(extent.body, width, extent.text, None, extent.body_lines);
    }
    if extent.progress {
        content += PROGRESS_GAP + PROGRESS;
    }
    if extent.image {
        content = content.max(d.header + d.image);
    }
    if extent.reply {
        content += d.action_gap + REPLY_ROW;
    } else if extent.actions {
        content += d.action_gap + ACTION;
    }
    (content + 2.0 * d.padding).ceil()
}

pub fn view<'a, Message: Clone + 'a>(
    card: Card<'a, Message>,
    palette: &Palette,
) -> Element<'a, Message> {
    let d = density(card.style);
    let backdrop = card.surface.unwrap_or(palette.surface_elevated);
    let danger = palette.danger;
    let summary = |card: &Card<'a, Message>| {
        let title = enz_text(card.summary.to_owned()).announce(false);
        let title = if card.critical {
            title.color(danger)
        } else {
            title
        };
        title
            .size_px(summary_size(card.text))
            .weight_rung(if card.unread {
                enzui_theme::WeightRung::Bold
            } else {
                enzui_theme::WeightRung::Semibold
            })
            .wrap(true)
            .lines(2)
            .width(Length::Fill)
            .into_element()
    };
    // Without a sender line (a group header names it) the title takes the
    // first line, beside the time and the dismiss button.
    let titled = card.source.is_empty();
    let mut title = Some(summary(&card));
    let meta: Element<'a, Message> = if titled {
        title.take().expect("the title")
    } else {
        row![
            enz_text(card.source.to_owned())
                .announce(false)
                .size_px(meta_size(card.text))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .muted_on(backdrop)
                .lines(1)
                .into_element(),
        ]
        .spacing(6)
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .into()
    };
    // When it arrived sits at the right, beside the dismiss button.
    let mut header = row![meta].spacing(6).align_y(Alignment::Center);
    if !titled {
        header = header.height(d.header);
    }
    if let Some(time) = card.time {
        header = header.push(
            enz_text(time)
                .announce(false)
                .size_px(meta_size(card.text))
                .subtle_on(backdrop)
                .lines(1)
                .into_element(),
        );
    }
    if let Some(dismiss) = card.dismiss {
        header = header.push(dismiss);
    }
    let mut text = column![header].spacing(d.text_gap).width(Length::Fill);
    if let Some(title) = title {
        text = text.push(title);
    }
    if !card.body.trim().is_empty() {
        let mut body = enz_text(card.body.to_owned())
            .announce(false)
            .size_px(card.text)
            .muted_on(backdrop)
            .wrap(true)
            .width(Length::Fill);
        if card.body_lines > 0 {
            body = body.lines(card.body_lines as u16);
        }
        text = text.push(body.into_element());
    }
    if let Some(value) = card.progress {
        let accent = palette.accent;
        let track = Color {
            a: 0.25,
            ..palette.text_muted
        };
        let part = |portion: u8, fill: Color| -> Element<'a, Message> {
            container(iced::widget::space())
                .width(Length::FillPortion(u16::from(portion)))
                .height(PROGRESS)
                .style(move |_| container::Style {
                    background: Some(fill.into()),
                    border: iced::Border {
                        radius: (PROGRESS / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into()
        };
        let value = value.min(100);
        let mut bar = iced::widget::Row::new()
            .width(Length::Fill)
            .height(PROGRESS);
        if value > 0 {
            bar = bar.push(part(value, accent));
        }
        if value < 100 {
            bar = bar.push(part(100 - value, track));
        }
        text = text.push(
            container(bar)
                .padding(iced::Padding::default().top(PROGRESS_GAP))
                .width(Length::Fill),
        );
    }
    let mut content = row![text].spacing(d.glyph_gap).align_y(Alignment::Start);
    if let Some(image) = card.image {
        content = content.push(
            container(image)
                .width(d.image)
                .height(d.image)
                .clip(true)
                .style(|_| container::Style {
                    border: iced::Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .padding(iced::Padding::default().top(d.header)),
        );
    }
    let mut stack = column![content];
    if !card.actions.is_empty() {
        // A fixed-height card spends its spare room (at least the gap) here.
        stack = stack.push(iced::widget::space().height(if card.height.is_some() {
            Length::Fill
        } else {
            Length::Fixed(d.action_gap)
        }));
        stack = stack.push(
            row(card.actions)
                .spacing(6)
                .width(Length::Fill)
                .align_y(Alignment::Center),
        );
    }
    let padding = iced::Padding {
        top: d.padding,
        right: d.padding,
        bottom: d.padding,
        left: left_padding(d),
    };
    let mut surface = enz_card(stack).padding(padding).radius(card.radius);
    surface = if card.glass {
        surface
            .surface_color(Color::TRANSPARENT)
            .elevation_override(0, None)
            .edge(0.0)
    } else {
        surface.elevation_override(d.elevation, None).edge(0.18)
    };
    if let (Some(colour), false) = (card.surface, card.glass) {
        surface = surface.surface_color(colour);
    }
    if let Some(height) = card.height {
        surface = surface.height_length(Length::Fixed(height));
    }
    // No icon badge: the sender is named in text (or by the group header),
    // and urgency shows in the title's colour.
    container(surface.width_length(Length::Fill).into_element())
        .width(Length::Fill)
        .into()
}
