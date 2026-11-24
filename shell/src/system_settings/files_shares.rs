//! SMB shares saved in Files (`org.enzimos.Mount1`), offered as backup
//! destinations, and their passwords from the user's keyring.

use enzui_core::form::SecretText;
use serde_json::Value;
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as Variant};

const MOUNT: &str = "org.enzimos.Mount1";
const MOUNT_PATH: &str = "/org/enzimos/Mount1";
// The schema enzimos-mount stores location passwords under.
const PASSWORD_SCHEMA: &str = "org.enzimos.Mount.Password";

/// A saved SMB location from Files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilesShare {
    pub id: String,
    pub label: String,
    pub server: String,
    pub share: String,
    pub folder: String,
    pub user: String,
    pub domain: String,
    pub auto_connect: bool,
    pub mount_at_boot: bool,
}

impl FilesShare {
    /// Files connects it at login or at startup.
    pub fn connects_automatically(&self) -> bool {
        self.auto_connect || self.mount_at_boot
    }
    /// The same share as a backup setup, compared without letter case.
    pub fn matches(&self, server: &str, share: &str) -> bool {
        self.server.eq_ignore_ascii_case(server.trim())
            && self.share.eq_ignore_ascii_case(share.trim())
    }
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `smb://[domain;][user@]server[:port]/share[/folder]`.
pub fn parse_smb(uri: &str) -> Option<(String, String, String, String, String)> {
    let rest = uri.strip_prefix("smb://")?;
    let (authority, path) = rest.split_once('/')?;
    let (account, host) = authority.rsplit_once('@').unwrap_or(("", authority));
    let (domain, user) = account.split_once(';').unwrap_or(("", account));
    let user = user.split_once(':').map_or(user, |(user, _)| user);
    let server = host.split_once(':').map_or(host, |(host, _)| host);
    let mut parts = path.trim_matches('/').splitn(2, '/');
    let share = decode(parts.next()?)?;
    let folder = decode(parts.next().unwrap_or(""))?;
    (!server.is_empty() && !share.is_empty()).then(|| {
        Some((
            decode(server)?,
            share,
            folder.trim_matches('/').to_owned(),
            decode(user)?,
            decode(domain)?,
        ))
    })?
}

/// The saved SMB shares in a `List` snapshot.
pub fn from_snapshot(snapshot: &Value) -> Vec<FilesShare> {
    let Some(items) = snapshot["items"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item["kind"] == "network" && item["saved"] == true)
        .filter_map(|item| {
            let (server, share, folder, user, domain) = parse_smb(item["uri"].as_str()?)?;
            Some(FilesShare {
                id: item["id"].as_str()?.to_owned(),
                label: item["label"].as_str().unwrap_or_default().to_owned(),
                server,
                share,
                folder,
                user,
                domain,
                auto_connect: item["auto_connect"] == true,
                mount_at_boot: item["mount_at_boot"] == true,
            })
        })
        .collect()
}

async fn mount_proxy() -> Option<zbus::Proxy<'static>> {
    let connection = zbus::Connection::session().await.ok()?;
    zbus::Proxy::new(&connection, MOUNT, MOUNT_PATH, MOUNT)
        .await
        .ok()
}

/// The SMB shares saved in Files; empty when Files' mount service is absent.
pub async fn list() -> Vec<FilesShare> {
    let Some(proxy) = mount_proxy().await else {
        return Vec::new();
    };
    let snapshot: Option<String> =
        tokio::time::timeout(std::time::Duration::from_secs(10), proxy.call("List", &()))
            .await
            .ok()
            .and_then(Result::ok);
    snapshot
        .and_then(|text| serde_json::from_str(&text).ok())
        .map(|value| from_snapshot(&value))
        .unwrap_or_default()
}

/// Lets Files connect the share at every login.
pub async fn connect_at_login(id: String) -> Result<(), String> {
    let proxy = mount_proxy()
        .await
        .ok_or_else(|| crate::i18n::tr("shell-files-share-unavailable").to_owned())?;
    let options = HashMap::from([("auto_connect", Variant::from(true))]);
    proxy
        .call::<_, _, ()>("UpdateLocation", &(id.as_str(), options))
        .await
        .map_err(|_| crate::i18n::tr("shell-files-share-unavailable").to_owned())
}

/// The password Files remembers for this share, when the keyring is unlocked.
pub async fn password(share: FilesShare) -> Option<SecretText> {
    tokio::time::timeout(std::time::Duration::from_secs(10), lookup(&share))
        .await
        .ok()
        .flatten()
}

async fn lookup(share: &FilesShare) -> Option<SecretText> {
    let connection = zbus::Connection::session().await.ok()?;
    let service = zbus::Proxy::new(
        &connection,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .await
    .ok()?;
    let (_, session): (OwnedValue, OwnedObjectPath) = service
        .call("OpenSession", &("plain", Variant::from("")))
        .await
        .ok()?;
    let by_location = HashMap::from([
        ("xdg:schema", PASSWORD_SCHEMA),
        ("location", share.id.as_str()),
    ]);
    let by_account = HashMap::from([
        ("xdg:schema", PASSWORD_SCHEMA),
        ("protocol", "smb"),
        ("server", share.server.as_str()),
        ("user", share.user.as_str()),
    ]);
    let mut found = None;
    for attributes in [
        Some(by_location),
        (!share.user.is_empty()).then_some(by_account),
    ]
    .into_iter()
    .flatten()
    {
        let (unlocked, _locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            service.call("SearchItems", &(attributes,)).await.ok()?;
        if let Some(item) = unlocked.into_iter().next() {
            found = Some(item);
            break;
        }
    }
    let item = found?;
    let secrets: HashMap<OwnedObjectPath, (OwnedObjectPath, Vec<u8>, Vec<u8>, String)> = service
        .call("GetSecrets", &(vec![item.clone()], &session))
        .await
        .ok()?;
    let _ = zbus::Proxy::new(
        &connection,
        "org.freedesktop.secrets",
        session.as_str(),
        "org.freedesktop.Secret.Session",
    )
    .await
    .ok()?
    .call::<_, _, ()>("Close", &())
    .await;
    let (_, _, value, _) = secrets.get(&item)?;
    String::from_utf8(value.clone()).ok().map(SecretText::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn smb_addresses_and_snapshot() {
        assert_eq!(
            parse_smb("smb://userm@192.168.2.86/Downloads"),
            Some((
                "192.168.2.86".into(),
                "Downloads".into(),
                String::new(),
                "userm".into(),
                String::new()
            ))
        );
        assert_eq!(
            parse_smb("smb://WORK;alice@nas.local:445/Team%20Files/backups/"),
            Some((
                "nas.local".into(),
                "Team Files".into(),
                "backups".into(),
                "alice".into(),
                "WORK".into()
            ))
        );
        assert_eq!(
            parse_smb("smb://nas.local"),
            None,
            "a server without a share"
        );
        assert_eq!(parse_smb("sftp://nas.local/x"), None);
        let shares = from_snapshot(&json!({"items": [
            {"id": "net-1", "kind": "network", "saved": true, "uri": "smb://u@nas/a",
             "label": "a on nas", "auto_connect": true, "mount_at_boot": false},
            {"id": "net-2", "kind": "network", "saved": false, "uri": "smb://u@nas/b"},
            {"id": "net-3", "kind": "network", "saved": true, "uri": "sftp://u@nas/c"},
            {"id": "block:sdb1", "kind": "usb", "saved": false}
        ]}));
        assert_eq!(shares.len(), 1);
        assert!(shares[0].connects_automatically());
        assert!(shares[0].matches("NAS", "A"));
    }
}
