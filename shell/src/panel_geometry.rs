mod operation;
pub(crate) use operation::Projected;

pub(crate) fn inverse_event(event: &iced::Event, transform: iced::Transformation) -> iced::Event {
    use iced::{Event, mouse, touch};
    let inverse = transform.inverse();
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Event::Mouse(mouse::Event::CursorMoved {
                position: *position * inverse,
            })
        }
        Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels { x, y },
        }) => Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels {
                x: *x / transform.scale_factor(),
                y: *y / transform.scale_factor(),
            },
        }),
        Event::Touch(touch::Event::FingerPressed { id, position }) => {
            Event::Touch(touch::Event::FingerPressed {
                id: *id,
                position: *position * inverse,
            })
        }
        Event::Touch(touch::Event::FingerMoved { id, position }) => {
            Event::Touch(touch::Event::FingerMoved {
                id: *id,
                position: *position * inverse,
            })
        }
        Event::Touch(touch::Event::FingerLifted { id, position }) => {
            Event::Touch(touch::Event::FingerLifted {
                id: *id,
                position: *position * inverse,
            })
        }
        Event::Touch(touch::Event::FingerLost { id, position }) => {
            Event::Touch(touch::Event::FingerLost {
                id: *id,
                position: *position * inverse,
            })
        }
        _ => event.clone(),
    }
}
use crate::reveal::{Attachment, Frame, Reveal, Shape};
use enzui_theme::ExpressiveMotion;
use iced::{Rectangle, Size, Vector};

pub(crate) fn frame(
    reveal: Reveal,
    extent: Size,
    bounded: f32,
    spatial: f32,
    motion: ExpressiveMotion,
) -> Frame {
    let Shape::Controls {
        bottom, attachment, ..
    } = reveal.shape
    else {
        return reveal.frame(extent, bounded);
    };
    let margins = reveal.margins();
    let size = Size::new(
        (extent.width - margins.left - margins.right).max(0.0),
        (extent.height - margins.top - margins.bottom).max(0.0),
    );
    let p = bounded.clamp(0.0, 1.0);
    let s = spatial.clamp(0.0, 1.0);
    let bounce = (spatial - bounded).clamp(-0.3, 0.3);
    // Each style starts close to the panel's resting size: a panel that
    // grew from a small tile read as a toy, and a long overshoot as a wobble.
    let (scale, travel) = match motion {
        ExpressiveMotion::FastSpatial => ((0.9 + 0.1 * spatial).clamp(0.9, 1.02), bounce * 6.0),
        ExpressiveMotion::DefaultSpatial => (1.0, (1.0 - p) * 14.0 + bounce * 6.0),
        ExpressiveMotion::SlowSpatial => (0.72 + 0.28 * p, bounce * 6.0),
        ExpressiveMotion::FastEffects
        | ExpressiveMotion::DefaultEffects
        | ExpressiveMotion::SlowEffects => return reveal.frame(extent, p),
    };
    if let Attachment::Island { center, seed_width } = attachment {
        let seed = seed_width.clamp(0.0, size.width);
        let center = center.clamp(seed / 2.0, size.width - seed / 2.0);
        let w = size.width * scale;
        let h = size.height * scale;
        let x = margins.left + center * (1.0 - scale);
        let shift = if bottom { -travel } else { travel };
        let y = margins.top + (if bottom { size.height - h } else { 0.0 }) + shift;
        let clip = Rectangle {
            x,
            y,
            width: w,
            height: h,
        };
        let radius = (reveal.radius * scale).max(0.0).min(w / 2.0).min(h / 2.0);
        Frame {
            extended: p > f32::EPSILON && w > 0.0 && h > 0.0,
            translation: if motion == ExpressiveMotion::DefaultSpatial {
                Vector::new(0.0, shift)
            } else {
                Vector::ZERO
            },
            clip,
            fills: vec![(clip, radius.into())],
            corners: vec![],
        }
    } else {
        let depth = match motion {
            ExpressiveMotion::FastSpatial => s,
            ExpressiveMotion::DefaultSpatial => p,
            ExpressiveMotion::SlowSpatial => p * p,
            _ => p,
        };
        let mut frame = reveal.frame(extent, depth);
        // Joined surfaces stay attached to their bar; only the free edge grows.
        frame.translation = Vector::ZERO;
        frame.extended &= p > f32::EPSILON;
        frame
    }
}

/// The standard opening of a floating panel: it settles from just under its
/// size at its trigger, with no overshoot. The wipe it replaces showed the
/// panel's content cut off while its corner grew.
pub(crate) fn settle(reveal: Reveal, extent: Size, progress: f32) -> Frame {
    let Shape::Controls {
        bottom,
        attachment: Attachment::Island { center, seed_width },
        ..
    } = reveal.shape
    else {
        return reveal.frame(extent, progress);
    };
    let margins = reveal.margins();
    let size = Size::new(
        (extent.width - margins.left - margins.right).max(0.0),
        (extent.height - margins.top - margins.bottom).max(0.0),
    );
    let p = progress.clamp(0.0, 1.0);
    let scale = 0.94 + 0.06 * p;
    let seed = seed_width.clamp(0.0, size.width);
    let center = center.clamp(seed / 2.0, size.width - seed / 2.0);
    let (w, h) = (size.width * scale, size.height * scale);
    let clip = Rectangle {
        x: margins.left + center * (1.0 - scale),
        y: margins.top + if bottom { size.height - h } else { 0.0 },
        width: w,
        height: h,
    };
    let radius = (reveal.radius * scale).max(0.0).min(w / 2.0).min(h / 2.0);
    Frame {
        extended: p > f32::EPSILON && w > 0.0 && h > 0.0,
        translation: Vector::ZERO,
        clip,
        fills: vec![(clip, radius.into())],
        corners: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reveal::PanelShadow;
    fn reveal(bottom: bool) -> Reveal {
        Reveal::popup(
            bottom,
            Attachment::Island {
                center: 290.0,
                seed_width: 48.0,
            },
            18.0,
            iced::Color::BLACK,
            PanelShadow {
                room: true,
                level: 2,
                bar: false,
            },
        )
    }
    #[test]
    fn panel_geometry_behaviour() {
        // Composed surface projection matches every painted edge and inverse pointer.
        {
            let size = Size::new(440.0, 360.0);
            for bottom in [false, true] {
                for motion in [
                    ExpressiveMotion::FastSpatial,
                    ExpressiveMotion::DefaultSpatial,
                    ExpressiveMotion::SlowSpatial,
                ] {
                    let reveal = reveal(bottom);
                    let rest = reveal.frame(size, 1.0);
                    for step in 1..=100 {
                        let p = step as f32 / 100.0;
                        let current = frame(reveal, size, p, p + 0.2 * (1.0 - p), motion);
                        let scale = current.clip.width / rest.clip.width;
                        let transform =
                            iced::Transformation::translate(current.clip.x, current.clip.y)
                                * iced::Transformation::scale(scale)
                                * iced::Transformation::translate(-rest.clip.x, -rest.clip.y);
                        let projected = rest.clip * transform;
                        assert!(
                            (projected.x - current.clip.x).abs() < 0.001
                                && (projected.y - current.clip.y).abs() < 0.001
                        );
                        assert!((projected.height - current.clip.height).abs() < 0.001);
                        let center = rest.clip.center();
                        let painted = center * transform;
                        let inverse = painted * transform.inverse();
                        assert!(
                            (inverse.x - center.x).abs() < 0.001
                                && (inverse.y - center.y).abs() < 0.001
                        );
                    }
                }
            }
        }
        // Joined surface retains its bar anchor and does not slide children through the edge.
        {
            for bottom in [false, true] {
                let reveal = Reveal::popup(
                    bottom,
                    Attachment::RightFrame,
                    18.0,
                    iced::Color::BLACK,
                    PanelShadow::NONE,
                );
                let frame = frame(
                    reveal,
                    Size::new(360.0, 280.0),
                    0.4,
                    0.7,
                    ExpressiveMotion::FastSpatial,
                );
                assert_eq!(frame.translation, Vector::ZERO);
                if bottom {
                    assert_eq!(frame.clip.y + frame.clip.height, 280.0);
                } else {
                    assert_eq!(frame.clip.y, 0.0);
                }
            }
        }
    }
}
