use super::{PATH, owned, require_frontend, text};
use crate::notifications::{
    payload::{Draft, Image},
    server,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::Cursor,
    os::unix::fs::FileExt,
    path::PathBuf,
};
use zbus::{
    Connection, fdo,
    message::Header,
    object_server::SignalEmitter,
    zvariant::{self, OwnedValue, Structure, Value},
};

pub(crate) const BRIDGE: &str = "dev.enzui.Shell.PortalNotification1";
const INTERFACE: &str = "org.freedesktop.impl.portal.Notification";
type Options = HashMap<String, OwnedValue>;

pub(super) struct Notification;

#[zbus::interface(name = "org.freedesktop.impl.portal.Notification", spawn = false)]
impl Notification {
    async fn add_notification(
        &self,
        app_id: &str,
        id: &str,
        notification: Options,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let frontend = require_frontend(connection, &header).await?;
        let notification = tokio::task::spawn_blocking(move || prepare_icon(notification))
            .await
            .map_err(|error| fdo::Error::Failed(error.to_string()))?;
        forward(connection, "Add", &(frontend, app_id, id, notification)).await
    }

    async fn remove_notification(
        &self,
        app_id: &str,
        id: &str,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        let frontend = require_frontend(connection, &header).await?;
        forward(connection, "Remove", &(frontend, app_id, id)).await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The bridge carries a destination and the portal signal's arguments."
    )]
    async fn dispatch_action(
        &self,
        frontend: &str,
        app_id: &str,
        id: &str,
        action: &str,
        parameters: Vec<OwnedValue>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        super::require_owner(connection, &header, server::SERVICE).await?;
        let bus = fdo::DBusProxy::new(connection).await?;
        let owner = bus
            .get_name_owner(super::FRONTEND.try_into().map_err(zbus::Error::from)?)
            .await?;
        if owner.as_str() != frontend {
            return Err(fdo::Error::AccessDenied(
                "The portal frontend has changed".into(),
            ));
        }
        connection
            .emit_signal(
                Some(frontend),
                PATH,
                INTERFACE,
                "ActionInvoked",
                &(app_id, id, action, parameters),
            )
            .await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        app_id: &str,
        id: &str,
        action: &str,
        parameter: Vec<OwnedValue>,
    ) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    #[zbus(property)]
    fn supported_options(&self) -> Options {
        Options::new()
    }
}

async fn forward(
    connection: &Connection,
    method: &str,
    body: &(impl serde::Serialize + zvariant::DynamicType),
) -> fdo::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        zbus::Proxy::new(connection, server::SERVICE, server::PATH, BRIDGE)
            .await?
            .call(method, body)
            .await
            .map_err(Into::into)
    })
    .await
    .map_err(|_| fdo::Error::Timeout("Notification server did not respond".into()))?
}

pub(crate) async fn dispatch(
    connection: &Connection,
    backend: &str,
    portal: &Delivery,
    key: &str,
    token: Option<&str>,
) -> zbus::Result<()> {
    let Some(action) = portal.actions.get(key) else {
        return Ok(());
    };
    let args = action
        .target
        .as_ref()
        .map(OwnedValue::try_clone)
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();
    let mut platform = Options::new();
    if let Some(token) = token.filter(|token| !token.is_empty() && token.len() <= 4096) {
        platform.insert("activation-token".into(), owned(Value::from(token)));
    }
    if let Some(name) = action.name.strip_prefix("app.")
        && super::valid_app_id(&portal.app_id)
    {
        let path = format!("/{}", portal.app_id.replace('.', "/").replace('-', "_"));
        connection
            .call_method(
                Some(portal.app_id.as_str()),
                path.as_str(),
                Some("org.freedesktop.Application"),
                "ActivateAction",
                &(name, args, platform),
            )
            .await?;
        return Ok(());
    }
    let mut args = args;
    args.push(owned(Value::from(platform)));
    zbus::Proxy::new(connection, backend, PATH, INTERFACE)
        .await?
        .call_noreply(
            "DispatchAction",
            &(
                &portal.frontend,
                &portal.app_id,
                &portal.id,
                &action.name,
                args,
            ),
        )
        .await?;
    Ok(())
}

pub(crate) struct Delivery {
    pub(crate) frontend: String,
    pub(crate) app_id: String,
    pub(crate) id: String,
    actions: BTreeMap<String, Action>,
}

struct Action {
    name: String,
    target: Option<OwnedValue>,
}

fn valid_notification_app(app_id: &str) -> bool {
    app_id.len() <= 255
        && app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(crate) fn parse(
    frontend: String,
    app_id: String,
    id: String,
    mut options: Options,
) -> Result<(Draft, Delivery, Option<PathBuf>), String> {
    if id.len() > 256 || !valid_notification_app(&app_id) || options.len() > 32 {
        return Err("Invalid portal notification identity or options".into());
    }

    let title = text(&options, "title").unwrap_or_default();
    let body = text(&options, "body")
        .or_else(|| text(&options, "markup-body").map(plain_markup))
        .unwrap_or_default();
    let urgency = match text(&options, "priority").as_deref() {
        Some("low") => 0u8,
        Some("high" | "urgent") => 2,
        _ => 1,
    };
    let mut hints = Options::from([("urgency".into(), urgency.into())]);
    if options
        .remove("display-hint")
        .and_then(|value| Vec::<String>::try_from(value).ok())
        .is_some_and(|values| values.iter().any(|value| value == "transient"))
    {
        hints.insert("transient".into(), true.into());
    }
    let mut keys = Vec::new();
    let mut actions = BTreeMap::new();
    if let Some(name) = text(&options, "default-action") {
        keys.extend(["default".into(), String::new()]);
        actions.insert(
            "default".into(),
            action(name, options.remove("default-action-target"))?,
        );
    }
    if let Some(buttons) = options.remove("buttons") {
        let buttons =
            Vec::<Options>::try_from(buttons).map_err(|_| "Invalid portal notification buttons")?;
        for (index, mut button) in buttons.into_iter().take(7).enumerate() {
            let (Some(label), Some(name)) = (text(&button, "label"), text(&button, "action"))
            else {
                continue;
            };
            let key = format!("button-{index}");
            actions.insert(key.clone(), action(name, button.remove("target"))?);
            keys.extend([key, label]);
        }
    }
    let mut draft = Draft::parse((
        String::new(),
        0,
        String::new(),
        title,
        body,
        keys,
        hints,
        -1,
    ))?;
    let themed = options.remove("icon").and_then(|value| {
        let (kind, value): (String, OwnedValue) =
            Structure::try_from(value).ok()?.try_into().ok()?;
        match kind.as_str() {
            "themed" => Vec::<String>::try_from(value)
                .ok()?
                .iter()
                .filter(|name| !name.contains('/'))
                .find_map(|name| crate::icon_theme::lookup(name, 48)),
            "rgba" => {
                let (width, height, bytes): (u32, u32, Vec<u8>) =
                    Structure::try_from(value).ok()?.try_into().ok()?;
                draft.image = Image::from_rgba(width, height, bytes).ok();
                None
            }
            _ => None,
        }
    });
    Ok((
        draft,
        Delivery {
            frontend,
            app_id,
            id,
            actions,
        },
        themed,
    ))
}

fn action(name: String, target: Option<OwnedValue>) -> Result<Action, String> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("Invalid portal notification action".into());
    }
    if let Some(target) = &target {
        let serialized = zvariant::to_bytes(
            zvariant::serialized::Context::new_dbus(zvariant::LE, 0),
            target,
        )
        .map_err(|_| "Invalid portal notification action target")?;
        if serialized.len() > 4096 || !serialized.fds().is_empty() {
            return Err("Portal notification action target exceeds its limits".into());
        }
    }
    Ok(Action { name, target })
}

fn prepare_icon(mut options: Options) -> Options {
    if let Some(icon) = options.remove("icon")
        && let Ok(tuple) = Structure::try_from(&*icon)
        && let Ok((kind, data)) = <(String, OwnedValue)>::try_from(tuple)
    {
        match kind.as_str() {
            "themed" => {
                options.insert("icon".into(), icon);
            }
            "bytes" | "file-descriptor" => {
                let bytes = if kind == "bytes" {
                    Vec::<u8>::try_from(data).ok()
                } else {
                    zvariant::Fd::try_from(data)
                        .ok()
                        .and_then(|fd| std::os::fd::OwnedFd::try_from(fd).ok())
                        .and_then(|fd| read_icon(&File::from(fd)))
                };
                if let Some(image) = bytes.and_then(decode_icon) {
                    let (bytes, width, height) = image.pixels();
                    let rgba = owned(Value::from((width, height, bytes.to_vec())));
                    options.insert("icon".into(), owned(Value::from(("rgba", rgba))));
                }
            }
            _ => {}
        }
    }
    // The notification socket does not accept descriptors.
    let mut budget = 1024 * 1024;
    options.retain(|key, value| {
        matches!(
            key.as_str(),
            "title"
                | "body"
                | "markup-body"
                | "icon"
                | "priority"
                | "default-action"
                | "default-action-target"
                | "buttons"
                | "display-hint"
        ) && zvariant::to_bytes(
            zvariant::serialized::Context::new_dbus(zvariant::LE, 0),
            value,
        )
        .is_ok_and(|bytes| {
            if bytes.len() > budget || !bytes.fds().is_empty() {
                return false;
            }
            budget -= bytes.len();
            true
        })
    });
    options
}

fn read_icon(file: &File) -> Option<Vec<u8>> {
    let metadata = file.metadata().ok()?;
    let len = usize::try_from(metadata.len()).ok()?;
    if !metadata.is_file() || len > 2 * 1024 * 1024 {
        return None;
    }
    let mut bytes = vec![0; len];
    file.read_exact_at(&mut bytes, 0).ok()?;
    Some(bytes)
}

fn decode_icon(bytes: Vec<u8>) -> Option<Image> {
    if bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let mut reader = raster::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = raster::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(256, 256).into_rgba8();
    let (width, height) = image.dimensions();
    Image::from_rgba(width, height, image.into_raw()).ok()
}

fn plain_markup(markup: String) -> String {
    let mut plain = String::new();
    let mut remaining = markup.as_str();
    while let Some((before, after)) = remaining.split_once('<') {
        plain.push_str(before);
        let Some((_, next)) = after.split_once('>') else {
            plain.push('<');
            remaining = after;
            break;
        };
        remaining = next;
    }
    plain.push_str(remaining);
    plain
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_button_actions_keep_their_own_target() {
        let notification = Options::from([
            ("title".into(), owned(Value::from("Portal notification"))),
            ("markup-body".into(), owned(Value::from("<b>A &amp; B</b>"))),
            ("default-action".into(), owned(Value::from("default"))),
            ("default-action-target".into(), 42u32.into()),
            (
                "buttons".into(),
                owned(Value::from(vec![Options::from([
                    ("label".into(), owned(Value::from("Open"))),
                    ("action".into(), owned(Value::from("default"))),
                    ("target".into(), 18u32.into()),
                ])])),
            ),
        ]);
        let (draft, route, _) = parse(
            ":1.2".into(),
            "org.example.App".into(),
            "notice".into(),
            notification,
        )
        .unwrap();
        assert_eq!(draft.body, "A & B");
        assert!(draft.permits_action("default"));
        assert!(draft.permits_action("button-0"));
        assert_eq!(
            route.actions["default"]
                .target
                .as_ref()
                .and_then(|v| u32::try_from(v).ok()),
            Some(42)
        );
        assert_eq!(
            route.actions["button-0"]
                .target
                .as_ref()
                .and_then(|v| u32::try_from(v).ok()),
            Some(18)
        );
    }
}
