use super::{Dialogs, Options, PortalDialog, PortalDialogAnswer, PortalDialogKind};
use crate::desktop_portal::{owned, text, uri};
use std::path::PathBuf;
use zbus::zvariant::{ObjectPath, Value};

pub(super) struct Account(Dialogs);

impl Account {
    pub(super) fn new(dialogs: Dialogs) -> Self {
        Self(dialogs)
    }
}

struct Profile {
    user: String,
    name: String,
    avatar: Option<PathBuf>,
}

impl Profile {
    async fn load() -> Option<Self> {
        if let Ok(Some((user, name, icon))) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crate::user_accounts::current_profile(),
        )
        .await
        {
            return Some(Self {
                user,
                name,
                avatar: icon
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute() && path.is_file()),
            });
        }
        let uid = rustix::process::getuid().as_raw().to_string();
        let output = tokio::process::Command::new("getent")
            .args(["passwd", &uid])
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let entry = String::from_utf8(output.stdout).ok()?;
        let mut fields = entry.lines().next()?.split(':');
        let user = fields.next()?.to_owned();
        let name = fields
            .nth(3)?
            .split(',')
            .next()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&user);
        let name = name.chars().filter(|c| !c.is_control()).take(160).collect();
        let avatar = PathBuf::from(fields.next()?).join(".face");
        let avatar = (avatar.is_absolute() && avatar.is_file()).then_some(avatar);
        Some(Self { user, name, avatar })
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Account")]
impl Account {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn get_user_information(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
        window: &str,
        options: Options,
    ) -> (u32, Options) {
        let mut profile = None;
        let prepare = async {
            let data = Profile::load().await.ok_or(2u32)?;
            let dialog = PortalDialog::new(
                app_id,
                window,
                &options,
                PortalDialogKind::ShareAccount {
                    user: data.user.clone(),
                    name: data.name.clone(),
                    reason: text(&options, "reason")
                        .map(|reason| reason.chars().take(1000).collect()),
                    avatar: data.avatar.clone(),
                },
            );
            profile = Some(data);
            Ok(dialog)
        };
        match self.0.ask(server, handle, header, None, prepare).await {
            Ok(PortalDialogAnswer::ShareAccount) => {}
            Err(response) => return (response, Options::new()),
            _ => return (1, Options::new()),
        }
        let Some(profile) = profile else {
            return (2, Options::new());
        };
        let mut results = Options::from([
            ("id".into(), owned(Value::from(profile.user))),
            ("name".into(), owned(Value::from(profile.name))),
        ]);
        if let Some(avatar) = profile.avatar {
            results.insert("image".into(), owned(Value::from(uri(&avatar))));
        }
        (0, results)
    }
}
