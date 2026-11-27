use super::Options;
use crate::desktop_portal::text;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

mod documents;
use zbus::zvariant::ObjectPath;

pub(super) struct Email;

fn strings(options: &Options, key: &str) -> Option<Vec<String>> {
    match options.get(key) {
        Some(value) => Vec::<String>::try_from(value.try_clone().ok()?).ok(),
        None => Some(Vec::new()),
    }
}

fn encode(out: &mut String, text: &str) {
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~@".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
}

fn attachment_paths(options: &Options) -> Option<Vec<PathBuf>> {
    strings(options, "attachments")?
        .into_iter()
        .map(|attachment| {
            // The frontend passes host paths for files exported from attachment_fds.
            if Path::new(&attachment).is_absolute() {
                return Some(PathBuf::from(attachment));
            }
            enzui_core::portal::uri::file_uri_to_path(&attachment)
                .ok()
                .filter(|path| path.is_absolute())
        })
        .collect()
}

fn mailto(options: &Options, attachments: &[PathBuf]) -> Option<String> {
    let mut to = strings(options, "addresses")?;
    if let Some(address) = text(options, "address")
        && !to.contains(&address)
    {
        to.push(address);
    }
    let cc = strings(options, "cc")?;
    let bcc = strings(options, "bcc")?;
    let subject = text(options, "subject").unwrap_or_default();
    if to
        .iter()
        .chain(&cc)
        .chain(&bcc)
        .any(|address| address.chars().any(char::is_control))
        || subject.chars().any(char::is_control)
    {
        return None;
    }
    let body = text(options, "body").unwrap_or_default();
    let mut uri = String::from("mailto:");
    for (i, address) in to.iter().enumerate() {
        if i > 0 {
            uri.push(',');
        }
        encode(&mut uri, address);
    }
    let attachments: Vec<_> = attachments
        .iter()
        .map(|path| crate::desktop_portal::uri(path))
        .collect();
    let cc = cc.join(",");
    let bcc = bcc.join(",");
    let mut separator = '?';
    for (key, value) in [
        ("cc", cc.as_str()),
        ("bcc", bcc.as_str()),
        ("subject", subject.as_str()),
        ("body", body.as_str()),
    ]
    .into_iter()
    .chain(attachments.iter().map(|uri| ("attachment", uri.as_str())))
    .filter(|(_, value)| !value.is_empty())
    {
        uri.push(separator);
        separator = '&';
        uri.push_str(key);
        uri.push('=');
        encode(&mut uri, value);
    }
    (uri.len() <= 64 * 1024).then_some(uri)
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Email")]
impl Email {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }

    async fn compose_email(
        &self,
        _handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        options: Options,
    ) -> (u32, Options) {
        let response = compose_mail(options).await;
        (if response.is_some() { 0 } else { 2 }, Options::new())
    }
}

async fn default_mail_handler() -> Option<(PathBuf, Option<String>)> {
    let output = tokio::process::Command::new("gio")
        .args(["mime", "x-scheme-handler/mailto"])
        .env("LC_ALL", "C")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    let (_, id) = output.lines().next()?.split_once(": ")?;
    crate::default_apps::desktop_file(id.trim())
}

async fn compose_mail(options: Options) -> Option<()> {
    let paths = attachment_paths(&options)?;
    let mut uri = mailto(&options, &paths)?;
    let mut command = tokio::process::Command::new("gio");
    if paths.is_empty() {
        command.arg("open");
    } else {
        let (desktop, flatpak) = default_mail_handler().await?;
        // Pass the exported paths to the same handler that received the grants.
        if let Some(app_id) = flatpak {
            let exports = documents::export(paths, &app_id).await?;
            uri = mailto(&options, &exports)?;
        }
        command.arg("launch").arg(desktop);
    }
    command
        .arg(uri)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(token) = text(&options, "activation_token") {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    command.status().await.ok()?.success().then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_portal::owned;
    use zbus::zvariant::Value;

    #[test]
    fn mailto_preserves_parameters_and_prevents_header_injection() {
        let mut options = Options::from([
            (
                "addresses".into(),
                owned(Value::from(vec!["a+b@example.org", "b@example.org"])),
            ),
            ("cc".into(), owned(Value::from(vec!["cc@example.org"]))),
            ("bcc".into(), owned(Value::from(vec!["bcc@example.org"]))),
            ("subject".into(), owned(Value::from("A & B"))),
            ("body".into(), owned(Value::from("Line 1\nLine 2"))),
            (
                "attachments".into(),
                owned(Value::from(vec!["file:///tmp/a%20b", "file:///tmp/c"])),
            ),
        ]);
        assert_eq!(
            mailto(
                &options,
                &attachment_paths(&options).expect("attachment URIs")
            )
            .as_deref(),
            Some(
                "mailto:a%2Bb@example.org,b@example.org?cc=cc@example.org&bcc=bcc@example.org&subject=A%20%26%20B&body=Line%201%0ALine%202&attachment=file%3A%2F%2F%2Ftmp%2Fa%2520b&attachment=file%3A%2F%2F%2Ftmp%2Fc"
            )
        );
        options.insert(
            "subject".into(),
            owned(Value::from("Hi\r\nCc: evil@example.org")),
        );
        assert!(mailto(&options, &[]).is_none());
        assert_eq!(mailto(&Options::new(), &[]).as_deref(), Some("mailto:"));
    }
}
