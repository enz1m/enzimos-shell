use super::{Dialogs, Options, PortalDialog, PortalDialogAnswer, PortalDialogKind};
use crate::desktop_portal::{owned, text};
use iced::widget::image::Handle;
use std::io::Cursor;
use zbus::zvariant::{ObjectPath, OwnedValue, Structure, Value};

const APPLICATION: u32 = 1;
const WEB_APP: u32 = 2;
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct DynamicLauncher(Dialogs);

impl DynamicLauncher {
    pub(super) fn new(dialogs: Dialogs) -> Self {
        Self(dialogs)
    }
}

fn icon_bytes(icon: &OwnedValue) -> Option<Vec<u8>> {
    let structure = <&Structure<'_>>::try_from(icon).ok()?;
    let [Value::Str(kind), Value::Value(bytes)] = structure.fields() else {
        return None;
    };
    let Value::Array(bytes) = bytes.as_ref() else {
        return None;
    };
    if kind.as_str() != "bytes" || bytes.len() > MAX_ICON_BYTES {
        return None;
    }
    bytes
        .inner()
        .iter()
        .map(|value| match value {
            Value::U8(byte) => Some(*byte),
            _ => None,
        })
        .collect()
}

fn preview(bytes: Vec<u8>) -> Option<Handle> {
    match raster::guess_format(&bytes) {
        Ok(format @ (raster::ImageFormat::Png | raster::ImageFormat::Jpeg)) => {
            let mut reader = raster::ImageReader::with_format(Cursor::new(bytes), format);
            let mut limits = raster::Limits::default();
            limits.max_image_width = Some(512);
            limits.max_image_height = Some(512);
            limits.max_alloc = Some(16 * 1024 * 1024);
            reader.limits(limits);
            let pixels = reader.decode().ok()?.into_rgba8();
            let (width, height) = pixels.dimensions();
            (width == height).then(|| Handle::from_rgba(width, height, pixels.into_raw()))
        }
        Err(_) => svg_preview(&bytes),
        _ => None,
    }
}

fn svg_preview(bytes: &[u8]) -> Option<Handle> {
    use resvg::{tiny_skia, usvg};
    let options = usvg::Options {
        // Launcher icons may only render their own data.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(std::str::from_utf8(bytes).ok()?, &options).ok()?;
    let size = tree.size();
    if size.width() != size.height() || size.width() > 4096.0 {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(128, 128)?;
    let scale = 128.0 / size.width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect::<Vec<_>>();
    Some(Handle::from_rgba(128, 128, pixels))
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control)
}

#[zbus::interface(name = "org.freedesktop.impl.portal.DynamicLauncher")]
impl DynamicLauncher {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    #[zbus(property, name = "SupportedLauncherTypes")]
    fn supported_launcher_types(&self) -> u32 {
        APPLICATION | WEB_APP
    }

    #[allow(clippy::too_many_arguments, reason = "Defined by the portal protocol.")]
    async fn prepare_install(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        name: &str,
        icon_v: OwnedValue,
        options: Options,
    ) -> (u32, Options) {
        if !valid_name(name) {
            return (2, Options::new());
        }
        let target = match options
            .get("launcher_type")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(APPLICATION)
        {
            APPLICATION => None,
            WEB_APP => match text(&options, "target") {
                Some(target) if target.len() <= 4096 => Some(target),
                _ => return (2, Options::new()),
            },
            _ => return (2, Options::new()),
        };
        let editable = options
            .get("editable_name")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(true);
        let prepare = async {
            let bytes = icon_bytes(&icon_v).ok_or(2u32)?;
            let preview = tokio::task::spawn_blocking(move || preview(bytes))
                .await
                .ok()
                .flatten()
                .ok_or(2u32)?;
            Ok(PortalDialog::new(
                app_id,
                parent_window,
                &options,
                PortalDialogKind::Install {
                    name: name.to_owned(),
                    editable,
                    target,
                    preview,
                },
            ))
        };
        match self.0.ask(server, handle, header, None, prepare).await {
            Ok(PortalDialogAnswer::Install(chosen)) if valid_name(&chosen) => (
                0,
                Options::from([
                    (
                        "name".into(),
                        owned(Value::from(if editable { chosen } else { name.to_owned() })),
                    ),
                    // The frontend expects a variant inside the results dictionary variant.
                    ("icon".into(), owned(Value::Value(Box::new(icon_v.into())))),
                ]),
            ),
            Err(response) => (response, Options::new()),
            _ => (1, Options::new()),
        }
    }

    fn request_install_token(&self, _app_id: &str, _options: Options) -> u32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raster::ImageEncoder as _;

    #[test]
    fn launcher_icons_enforce_the_frontend_format_and_dimension_limits() {
        let preview = |icon: &OwnedValue| preview(icon_bytes(icon).expect("serialized GBytesIcon"));
        let icon = |bytes: Vec<u8>| owned(Value::from(("bytes", Value::from(bytes))));
        let png = |width, height| {
            let mut bytes = Vec::new();
            raster::codecs::png::PngEncoder::new(&mut bytes)
                .write_image(
                    &vec![0; (width * height) as usize],
                    width,
                    height,
                    raster::ExtendedColorType::L8,
                )
                .expect("encode PNG fixture");
            icon(bytes)
        };
        let svg = |edge| {
            icon(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{edge}" height="{edge}"><circle cx="16" cy="16" r="10"/></svg>"#).into_bytes())
        };
        assert!(preview(&png(512, 512)).is_some());
        assert!(preview(&png(513, 513)).is_none());
        assert!(preview(&png(16, 32)).is_none());
        assert!(preview(&svg(64)).is_some());
        assert!(preview(&svg(4097)).is_none());
        assert!(preview(&icon(b"truncated icon".to_vec())).is_none());
    }
}
