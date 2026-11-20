use enzui_shell::i18n::tr;
use enzui_shell::tr;
use iced::widget::image::Handle;
use raster::{ImageDecoder, ImageFormat, ImageReader, Limits};
use sha2::{Digest, Sha256};
use std::{io::Cursor, sync::Arc};

pub const MAX_TRANSFER: usize = 8 * 1024 * 1024;
pub const MAX_TEXT: usize = 1024 * 1024;
pub const SELF_MIME: &str = "application/x-enzui-clipboard-restore";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mime {
    Text,
    Png,
}
impl Mime {
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Self::Text => &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"],
            Self::Png => &["image/png"],
        }
    }
    pub fn limit(self) -> usize {
        match self {
            Self::Text => MAX_TEXT,
            Self::Png => MAX_TRANSFER,
        }
    }
}
#[derive(Clone)]
pub struct Content {
    pub mime: Mime,
    pub bytes: Arc<[u8]>,
    pub preview: String,
    pub image: Option<Handle>,
    pub image_bytes: usize,
    pub digest: [u8; 32],
}
impl Content {
    pub fn decode(mime: Mime, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > mime.limit() {
            return Err(tr("shell-clipboard-content-exceeds-its-limit").into());
        }
        let (preview, image, image_bytes) = match mime {
            Mime::Text => {
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| tr("shell-clipboard-text-is-not-utf-8"))?;
                let preview = text.chars().filter_map(|c| {
                    if c.is_whitespace() { Some(' ') }
                    else if c.is_control() || matches!(c as u32,0x200b..=0x200f|0x202a..=0x202e|0x2060..=0x206f|0xfeff) { None }
                    else { Some(c) }
                }).take(240).collect();
                (preview, None, 0)
            }
            Mime::Png => {
                let mut reader = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Png);
                let mut limits = Limits::default();
                limits.max_image_width = Some(4096);
                limits.max_image_height = Some(4096);
                limits.max_alloc = Some(32 * 1024 * 1024);
                reader.limits(limits);
                let decoder = reader
                    .into_decoder()
                    .map_err(|_| tr("shell-invalid-clipboard-image"))?;
                let (w, h) = decoder.dimensions();
                if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 4 * 1024 * 1024 {
                    return Err(tr("shell-clipboard-image-dimensions-exceed-their-limit").into());
                }
                let decoded = raster::DynamicImage::from_decoder(decoder)
                    .map_err(|_| tr("shell-invalid-clipboard-image"))?;
                let thumb = decoded.thumbnail(256, 256).into_rgba8();
                let (width, height) = thumb.dimensions();
                let pixels = thumb.into_raw();
                let length = pixels.len();
                (
                    tr!("shell-image", "arg0" => format!("{:}", w), "arg1" => format!("{:}", h)),
                    Some(Handle::from_rgba(width, height, pixels)),
                    length,
                )
            }
        };
        // Fluent isolates numeric arguments; plugin text admits no bidi controls.
        let preview = preview
            .chars()
            .filter(|c| !matches!(*c as u32, 0x2066..=0x2069))
            .collect();
        let mut digest = Sha256::new();
        digest.update([match mime {
            Mime::Text => 0,
            Mime::Png => 1,
        }]);
        digest.update(&bytes);
        Ok(Self {
            digest: digest.finalize().into(),
            mime,
            bytes: bytes.into(),
            preview,
            image,
            image_bytes,
        })
    }
    pub fn size(&self) -> usize {
        self.bytes.len() + self.image_bytes
    }
}
pub fn choose(mimes: &[String]) -> Option<(Mime, String)> {
    if mimes.iter().any(|m| {
        let lower = m.to_ascii_lowercase();
        m == SELF_MIME
            || lower.contains("password")
            || lower.contains("sensitive")
            || lower.contains("secret")
    }) {
        return None;
    }
    for mime in [Mime::Png, Mime::Text] {
        for name in mime.names() {
            if mimes.iter().any(|m| m == name) {
                return Some((mime, (*name).into()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_behaviour() {
        // Negotiation prefers png and refuses sensitive or self offers.
        {
            let list = |s: &[&str]| s.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
            assert_eq!(
                choose(&list(&["text/plain", "image/png"])).unwrap().0,
                Mime::Png
            );
            assert_eq!(
                choose(&list(&["text/plain", "text/plain;charset=utf-8"]))
                    .unwrap()
                    .1,
                "text/plain;charset=utf-8"
            );
            for excluded in [
                SELF_MIME,
                "x-kde-passwordManagerHint",
                "application/x-sensitive",
            ] {
                assert!(choose(&list(&["text/plain", excluded])).is_none());
            }
            assert!(choose(&list(&["text/uri-list", "image/svg+xml"])).is_none());
        }
        // Previews are bounded but restoration preserves exact text bytes.
        {
            let bytes = "fixture\n\t\u{202e}text".as_bytes().to_vec();
            let content = Content::decode(Mime::Text, bytes.clone()).unwrap();
            assert_eq!(content.preview, "fixture  text");
            assert_eq!(&*content.bytes, bytes);
            assert!(Content::decode(Mime::Text, vec![0xff]).is_err());
            assert!(Content::decode(Mime::Text, vec![b'a'; MAX_TEXT + 1]).is_err());
            assert_eq!(
                Content::decode(Mime::Text, vec![b'a'; 1000])
                    .unwrap()
                    .preview
                    .len(),
                240
            );
        }
        // Image decode is bounded and uses raw thumbnail handles.
        {
            let image = raster::RgbaImage::from_pixel(600, 100, raster::Rgba([30, 90, 160, 255]));
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, ImageFormat::Png).unwrap();
            let content = Content::decode(Mime::Png, bytes.into_inner()).unwrap();
            assert_eq!(content.preview, "Image 600 × 100");
            assert!(content.image.is_some());
            assert!(content.image_bytes <= 256 * 256 * 4);
            assert!(Content::decode(Mime::Png, b"not png".to_vec()).is_err());
            assert!(Content::decode(Mime::Png, vec![0; MAX_TRANSFER + 1]).is_err());
        }
    }
}
