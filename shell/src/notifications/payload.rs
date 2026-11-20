use enzui_shell::i18n::tr;
use iced::widget::image::Handle;
use std::collections::{BTreeSet, HashMap};
use zbus::zvariant::{OwnedValue, Structure};

pub type Arguments = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, OwnedValue>,
    i32,
);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Action {
    pub key: String,
    pub label: String,
}
#[derive(Clone)]
pub struct Reply {
    pub placeholder: String,
    pub submit: String,
}
#[derive(Clone)]
pub struct Image(Handle);
impl Image {
    pub fn handle(&self) -> Handle {
        self.0.clone()
    }
    pub fn pixels(&self) -> (&[u8], u32, u32) {
        match &self.0 {
            Handle::Rgba {
                pixels,
                width,
                height,
                ..
            } => (pixels.as_ref(), *width, *height),
            _ => unreachable!(),
        }
    }
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > 256
            || height > 256
            || u64::from(width) * u64::from(height) * 4 != pixels.len() as u64
        {
            return Err(tr("shell-invalid-notification-thumbnail").into());
        }
        Ok(Self(Handle::from_rgba(width, height, pixels)))
    }
    fn from_hint(value: OwnedValue) -> Result<Self, String> {
        let structure =
            Structure::try_from(value).map_err(|_| tr("shell-invalid-notification-image"))?;
        let (width, height, stride, alpha, bits, channels, data): (
            i32,
            i32,
            i32,
            bool,
            i32,
            i32,
            Vec<u8>,
        ) = structure
            .try_into()
            .map_err(|_| tr("shell-invalid-notification-image"))?;
        if !(1..=512).contains(&width)
            || !(1..=512).contains(&height)
            || bits != 8
            || !matches!(channels, 3 | 4)
            || (alpha && channels != 4)
            || stride < width * channels
            || stride > 4096
            || data.len() > 2 * 1024 * 1024
        {
            return Err(tr("shell-notification-image-exceeds-supported-geometry").into());
        }
        let required = (height as usize - 1) * stride as usize + width as usize * channels as usize;
        if data.len() < required {
            return Err(tr("shell-notification-image-is-truncated").into());
        }
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row in 0..height as usize {
            for column in 0..width as usize {
                let index = row * stride as usize + column * channels as usize;
                pixels.extend_from_slice(&data[index..index + 3]);
                pixels.push(if alpha { data[index + 3] } else { 255 });
            }
        }
        let image = raster::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| tr("shell-invalid-notification-pixels"))?;
        let thumbnail = raster::DynamicImage::ImageRgba8(image)
            .thumbnail((width as u32).min(256), (height as u32).min(256))
            .into_rgba8();
        let (width, height) = thumbnail.dimensions();
        Self::from_rgba(width, height, thumbnail.into_raw())
    }
}
#[derive(Clone)]
pub struct Draft {
    pub app_name: String,
    /// The `desktop-entry` hint: a desktop file ID without `.desktop`.
    pub desktop_entry: Option<String>,
    pub replaces: u32,
    pub summary: String,
    pub body: String,
    pub actions: Vec<Action>,
    pub reply: Option<Reply>,
    pub image: Option<Image>,
    pub urgency: Urgency,
    pub resident: bool,
    pub transient: bool,
    pub timeout_ms: i32,
    /// The `value` hint: progress or a level, 0 to 100.
    pub value: Option<u8>,
}
pub fn text(value: &str, max: usize, multiline: bool) -> Result<String, String> {
    if value.len() > max * 4 || value.chars().count() > max {
        return Err(tr("shell-notification-text-exceeds-its-limit").into());
    }
    Ok(value
        .chars()
        .filter(|c| {
            (!c.is_control() || (multiline && matches!(c, '\n' | '\t')))
                && !matches!(*c as u32,0x200b|0x200e..=0x200f|0x202a..=0x202e|0x2060..=0x206f|0xfeff)
        })
        .collect())
}
fn hint_text(
    hints: &HashMap<String, OwnedValue>,
    key: &str,
    default: &str,
    max: usize,
) -> Result<String, String> {
    text(
        hints
            .get(key)
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or(default),
        max,
        false,
    )
}
impl Draft {
    pub fn parse(arguments: Arguments) -> Result<Self, String> {
        let (app_name, replaces, icon, summary, body, raw_actions, mut hints, timeout_ms) =
            arguments;
        if icon.len() > 1024
            || hints.len() > 32
            || hints.keys().any(|key| key.len() > 128)
            || raw_actions.len() > 16
            || !raw_actions.len().is_multiple_of(2)
            || timeout_ms < -1
        {
            return Err(tr("shell-invalid-or-excessive-notification-fields").into());
        }
        let mut keys = BTreeSet::new();
        let mut actions = Vec::new();
        for pair in raw_actions.as_chunks::<2>().0 {
            let key = &pair[0];
            if key.is_empty()
                || key.len() > 96
                || key.chars().any(char::is_control)
                || !keys.insert(key.clone())
            {
                return Err(tr("shell-invalid-notification-action-identity").into());
            }
            let label = text(&pair[1], 80, false)?;
            actions.push(Action {
                key: key.clone(),
                label: if label.trim().is_empty() {
                    match key.as_str() {
                        "default" => tr("shell-open"),
                        "inline-reply" => tr("shell-reply"),
                        _ => tr("shell-action"),
                    }
                    .into()
                } else {
                    label
                },
            });
        }
        let reply = if keys.contains("inline-reply") {
            let submit = hint_text(&hints, "x-kde-reply-submit-button-text", "Send", 40)?;
            Some(Reply {
                placeholder: hint_text(&hints, "x-kde-reply-placeholder-text", "Reply…", 120)?,
                submit: if submit.trim().is_empty() {
                    tr("shell-send").into()
                } else {
                    submit
                },
            })
        } else {
            None
        };
        let boolean = |key| {
            hints
                .get(key)
                .and_then(|value| bool::try_from(value).ok())
                .unwrap_or(false)
        };
        let resident = boolean("resident");
        let value = hints.get("value").and_then(|value| {
            i32::try_from(value)
                .ok()
                .or_else(|| {
                    u32::try_from(value)
                        .ok()
                        .and_then(|n| i32::try_from(n).ok())
                })
                .or_else(|| u8::try_from(value).ok().map(i32::from))
                .map(|n| n.clamp(0, 100) as u8)
        });
        let transient = boolean("transient");
        let urgency = match hints
            .get("urgency")
            .and_then(|value| u8::try_from(value).ok())
        {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        let desktop_entry = hints
            .get("desktop-entry")
            .and_then(|value| <&str>::try_from(value).ok())
            .map(|id| id.strip_suffix(".desktop").unwrap_or(id))
            .filter(|id| {
                (1..=255).contains(&id.len())
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            })
            .map(str::to_owned);
        let image = hints
            .remove("image-data")
            .or_else(|| hints.remove("image_data"))
            .map(Image::from_hint)
            .transpose()?;
        Ok(Self {
            app_name: text(&app_name, 80, false)?,
            desktop_entry,
            replaces,
            summary: text(&summary, 256, false)?,
            body: text(&body, 4096, true)?,
            actions,
            reply,
            image,
            urgency,
            resident,
            transient,
            timeout_ms,
            value,
        })
    }
    pub fn permits_action(&self, key: &str) -> bool {
        key != "inline-reply" && self.actions.iter().any(|action| action.key == key)
    }
    pub fn retained_bytes(&self) -> usize {
        self.app_name.len()
            + self.summary.len()
            + self.body.len()
            + self
                .actions
                .iter()
                .map(|action| action.key.len() + action.label.len())
                .sum::<usize>()
            + self
                .reply
                .as_ref()
                .map_or(0, |reply| reply.placeholder.len() + reply.submit.len())
            + self
                .image
                .as_ref()
                .map_or(0, |image| image.pixels().0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;
    fn args() -> Arguments {
        (
            "Fixture".into(),
            0,
            "/private/icon-not-read".into(),
            "Summary".into(),
            "Body".into(),
            vec![],
            HashMap::new(),
            -1,
        )
    }
    #[test]
    fn payload_behaviour() {
        // Malformed or duplicate actions and excessive payloads are rejected.
        {
            let mut input = args();
            input.5 = vec!["one".into()];
            assert!(Draft::parse(input).is_err());
            let mut input = args();
            input.5 = vec!["one".into(), "First".into(), "one".into(), "Second".into()];
            assert!(Draft::parse(input).is_err());
            let mut input = args();
            input.4 = "x".repeat(4097);
            assert!(Draft::parse(input).is_err());
            let mut input = args();
            input.7 = -2;
            assert!(Draft::parse(input).is_err());
            assert!(Draft::parse(args()).is_ok());
        }
        // The `value` hint is a progress or level bar, kept within 0 to 100.
        //
        // Sabotage: drop the `clamp` in `Draft::parse`.
        {
            let value = |hint: Value<'static>| {
                let mut input = args();
                input.6 =
                    HashMap::from([("value".to_owned(), OwnedValue::try_from(hint).unwrap())]);
                Draft::parse(input).unwrap().value
            };
            assert_eq!(value(Value::from(42_i32)), Some(42));
            assert_eq!(value(Value::from(150_i32)), Some(100));
            assert_eq!(value(Value::from(-5_i32)), Some(0));
            assert_eq!(value(Value::from(7_u32)), Some(7));
            assert_eq!(Draft::parse(args()).unwrap().value, None);
        }
        // Image stride channels and dimensions are bounded.
        {
            let image = |w: i32, h: i32, stride: i32, channels: i32, bytes: Vec<u8>| {
                OwnedValue::try_from(Value::from((
                    w,
                    h,
                    stride,
                    channels == 4,
                    8i32,
                    channels,
                    bytes,
                )))
                .unwrap()
            };
            let good = Image::from_hint(image(2, 1, 8, 3, vec![1, 2, 3, 4, 5, 6, 0, 0])).unwrap();
            assert_eq!(good.pixels(), (&[1, 2, 3, 255, 4, 5, 6, 255][..], 2, 1));
            let opaque = Image::from_hint(
                OwnedValue::try_from(Value::from((
                    1i32,
                    1i32,
                    4i32,
                    false,
                    8i32,
                    4i32,
                    vec![1u8, 2, 3, 0],
                )))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(opaque.pixels().0, &[1, 2, 3, 255]);
            assert!(Image::from_hint(image(2, 1, 4, 3, vec![0; 8])).is_err());
            assert!(Image::from_hint(image(513, 1, 2052, 4, vec![0; 2052])).is_err());
            assert!(Image::from_hint(image(2, 2, 8, 4, vec![0; 8])).is_err());
        }
    }
}
