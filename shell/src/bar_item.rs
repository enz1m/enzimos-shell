use crate::settings::Metrics;
use enzui_widgets::clickable::EnzClickable;

/// Shared shape for every interactive bar item. In Islands the item fills its
/// pill, so its hover and pressed states are the pill itself.
pub fn trigger<'a, Message: Clone + 'a>(
    clickable: EnzClickable<'a, Message>,
    metrics: Metrics,
) -> EnzClickable<'a, Message> {
    clickable
        .height(u32::from(metrics.bar_item_height()))
        .min_hit(28.0)
        .padding([0.0, f32::from(metrics.bar_padding(10))])
        .radius(metrics.bar_item_radius(7.0))
        .border_width(0.0)
}
