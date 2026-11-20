use iced::widget::svg;
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Overview { width: u32, left: u32, right: u32 },
    Controls { width: u32, height: u32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    shape: Shape,
    bottom: bool,
    light: bool,
    radius: u32,
}

pub fn overview(
    width: f32,
    left: u32,
    right: u32,
    bottom: bool,
    light: bool,
    radius: u16,
) -> svg::Handle {
    cached(Key {
        shape: Shape::Overview {
            width: width.round().clamp(80.0, 16384.0) as u32,
            left,
            right,
        },
        bottom,
        light,
        radius: u32::from(radius.min(32)),
    })
}
pub fn controls(width: f32, height: f32, bottom: bool, light: bool, radius: u16) -> svg::Handle {
    cached(Key {
        shape: Shape::Controls {
            width: width.round().clamp(80.0, 16384.0) as u32,
            height: height.round().clamp(80.0, 16384.0) as u32,
        },
        bottom,
        light,
        radius: u32::from(radius.min(32)),
    })
}
fn cached(key: Key) -> svg::Handle {
    static CACHE: OnceLock<Mutex<VecDeque<(Key, svg::Handle)>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, handle)) = cache.iter().find(|(k, _)| *k == key) {
        return handle.clone();
    }
    let handle = svg::Handle::from_memory(document(key).into_bytes());
    if cache.len() == 16 {
        cache.pop_front();
    }
    cache.push_back((key, handle.clone()));
    handle
}
fn document(key: Key) -> String {
    let r = key.radius;
    let (width, height, path) = match key.shape {
        Shape::Overview { width, left, right } => {
            let l = left.min(width / 3);
            let right = right.min(width / 3);
            let x = width - right;
            (
                width,
                r,
                format!(
                    "M0 0 H{} A{r} {r} 0 0 0 {l} {r} H0 Z M{} 0 H{width} V{r} H{x} A{r} {r} 0 0 0 {} 0 Z",
                    l + r,
                    x - r,
                    x - r
                ),
            )
        }
        Shape::Controls { width, height } => {
            let w = width + r;
            let h = height + r;
            (
                w,
                h,
                format!(
                    "M0 0 H{w} V{h} A{r} {r} 0 0 0 {} {height} H{} A{r} {r} 0 0 1 {r} {} V{r} A{r} {r} 0 0 0 0 0 Z",
                    w - r,
                    2 * r,
                    height - r
                ),
            )
        }
    };
    let [red, green, blue] = crate::desktop_frame::color(key.light);
    let transform = if key.bottom {
        format!("translate(0 {height}) scale(1 -1)")
    } else {
        String::new()
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><path transform=\"{transform}\" fill=\"#{red:02x}{green:02x}{blue:02x}\" d=\"{path}\"/></svg>"
    )
}
